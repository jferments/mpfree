//! The storage manager (disk streaming's P4, steps 1 to 3): where sounds
//! are read from, and what happens when the SD card goes.
//!
//! The stick is on at every start. The card is used only through a
//! handle the hardware project's storage safety layer hands over once
//! every one of its checks passes: it keeps Akai's disk service away,
//! makes the card read-only, checks the controller and partitions, and
//! mounts it. Nothing here can bind a controller or mount anything, so
//! nothing this layer does can switch the card (or the internal drive) on:
//! it only accepts a [`CardHandle`]. Removing the card revokes the handle.
//! The internal drive is never streamed from.
//!
//! Every insertion gets a new number, so a swapped card is never taken
//! for the first. A path is used only inside its library: no `..`, no
//! absolute paths, no links ([`Storage::locate`]).
//!
//! When the card is pulled, new reads from it stop at once, and reads
//! already running are checked against the insertion they were made for,
//! so nothing from an old or a different card is used (the coordinator's
//! part: [`crate::coord::Coordinator::open_located`]). Audio already in
//! memory keeps playing until it runs out and is never refilled from
//! another card; the coordinator reports which sounds lost their source.
//! Whether a stem group stops at once, finishes its bar or fades is the
//! clip launcher's and live shows' rule.

use std::fmt;
use std::fs::File;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use crate::policy::Device;

/// What the hardware project's storage safety layer hands over when a
/// card has passed its checks. Only that layer makes one.
pub trait CardHandle: Send + Sync {
    /// The card's library, mounted read-only by the safety layer.
    fn root(&self) -> &Path;
    /// Whether it still holds: removing the card revokes it, at once and
    /// for good.
    fn valid(&self) -> bool;
}

/// A card as this layer holds it: its handle and its insertion number.
#[derive(Clone)]
pub struct Card {
    pub handle: Arc<dyn CardHandle>,
    pub insertion: u32,
}

impl Card {
    /// Still the card in the slot, its handle still holding.
    pub fn valid(&self) -> bool {
        self.handle.valid()
    }
}

impl fmt::Debug for Card {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Card")
            .field("root", &self.handle.root())
            .field("insertion", &self.insertion)
            .field("valid", &self.valid())
            .finish()
    }
}

/// A file found inside a library.
#[derive(Clone, Debug)]
pub struct Located {
    pub path: PathBuf,
    pub device: Device,
    /// The card it is on, for a file on the card.
    pub card: Option<Card>,
}

impl Located {
    /// Whether it can still be read: always on the stick; on the card
    /// only while the insertion it was found on holds.
    pub fn valid(&self) -> bool {
        self.card.as_ref().is_none_or(Card::valid)
    }
}

/// Why a file can't be used.
#[derive(Debug, PartialEq, Eq)]
pub enum Refusal {
    /// No card in the slot, or its handle was revoked.
    NoCard,
    /// The path would leave the library (`..`, an absolute path, a drive).
    Outside(PathBuf),
    /// A link on the way: links aren't followed out of a library.
    Link(PathBuf),
    /// Not a file there.
    NotFound(PathBuf),
    /// Sounds play from the stick or the card only.
    NotStreamed,
    Io(String),
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refusal::NoCard => write!(f, "the SD card isn't there"),
            Refusal::Outside(p) => write!(f, "{}: outside the library", p.display()),
            Refusal::Link(p) => write!(f, "{}: a link, not followed", p.display()),
            Refusal::NotFound(p) => write!(f, "{}: not found", p.display()),
            Refusal::NotStreamed => write!(f, "sounds play from the stick or the SD card only"),
            Refusal::Io(e) => write!(f, "{e}"),
        }
    }
}

/// Where sounds are read from.
pub struct Storage {
    stick: PathBuf,
    card: Option<Card>,
    insertions: u32,
}

impl Storage {
    /// The stick's library at `stick`; no card yet.
    pub fn new(stick: &Path) -> Storage {
        Storage {
            stick: stick.to_path_buf(),
            card: None,
            insertions: 0,
        }
    }

    /// A card the safety layer has qualified: a new insertion, numbered
    /// apart from every earlier one.
    pub fn card_inserted(&mut self, handle: Arc<dyn CardHandle>) -> u32 {
        self.insertions += 1;
        self.card = Some(Card {
            handle,
            insertion: self.insertions,
        });
        self.insertions
    }

    /// The card went (the safety layer revoked its handle too).
    pub fn card_removed(&mut self) {
        self.card = None;
    }

    /// The card in the slot, if its handle still holds.
    pub fn card(&self) -> Option<&Card> {
        self.card.as_ref().filter(|c| c.valid())
    }

    /// `rel` inside `device`'s library: a relative path of plain names,
    /// no link anywhere on the way, ending at a file.
    pub fn locate(&self, device: Device, rel: &Path) -> Result<Located, Refusal> {
        let (root, card) = match device {
            Device::Stick => (self.stick.clone(), None),
            Device::Card => {
                let c = self.card().ok_or(Refusal::NoCard)?;
                (c.handle.root().to_path_buf(), Some(c.clone()))
            }
            Device::Memory => return Err(Refusal::NotStreamed),
        };
        if rel.as_os_str().is_empty()
            || !rel
                .components()
                .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
        {
            return Err(Refusal::Outside(rel.to_path_buf()));
        }
        let mut at = root;
        for c in rel.components() {
            if let Component::Normal(name) = c {
                at.push(name);
                let m = std::fs::symlink_metadata(&at).map_err(|e| match e.kind() {
                    io::ErrorKind::NotFound => Refusal::NotFound(rel.to_path_buf()),
                    _ => Refusal::Io(e.to_string()),
                })?;
                if m.file_type().is_symlink() {
                    return Err(Refusal::Link(rel.to_path_buf()));
                }
            }
        }
        if !at.is_file() {
            return Err(Refusal::NotFound(rel.to_path_buf()));
        }
        Ok(Located {
            path: at,
            device,
            card,
        })
    }

    /// Open a located file read-only, if it can still be read.
    pub fn open(&self, loc: &Located) -> Result<File, Refusal> {
        if !loc.valid() {
            return Err(Refusal::NoCard);
        }
        File::open(&loc.path).map_err(|e| Refusal::Io(format!("{}: {e}", loc.path.display())))
    }
}
