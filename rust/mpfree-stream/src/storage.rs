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
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};

use crate::policy::Device;
use crate::rec::RecId;

/// What the hardware project's storage safety layer hands over when a
/// card has passed its checks. Only that layer makes one.
pub trait CardHandle: Send + Sync {
    /// The card's library, mounted read-only by the safety layer.
    fn root(&self) -> &Path;
    /// Whether it still holds: removing the card revokes it, at once and
    /// for good.
    fn valid(&self) -> bool;
    /// Which card it is, once known ([`CardMark`]): None until then, or
    /// for a card nothing is recorded on.
    fn mark(&self) -> Option<CardMark> {
        None
    }
    /// How many of its files are open to play from now, as the reader
    /// last counted them: the card is held while any are (the SD card
    /// plan's B2c), so it isn't put away under a take playing from it.
    fn set_users(&self, _n: usize) {}
}

/// Which card: the random id in its `MPFree` folder's marker file
/// ([`MARKER`], [`make_marker`]) and its filesystem's volume serial (as
/// its window reads it). Either alone can be shared (a copy of the folder
/// carries the marker to another card, and a serial is only 32 bits), so a
/// card is taken for the same card only when both match. A take that
/// carried on elsewhere when its card went names the card its first part
/// stayed on with it ([`crate::rec::Hole`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CardMark {
    pub id: [u8; 16],
    pub serial: u32,
}

/// The marker file's name, in the card's `MPFree` folder.
pub const MARKER: &str = ".mpfree-card";
/// What the marker file holds before its id's 32 hex digits and a newline.
const MARKER_HEAD: &str = "MPFree card ";

/// The marker id in `folder` (the card's `MPFree`): None if it has no
/// marker file, or one that doesn't read as one (a link, too long, cut
/// short, other words).
pub fn read_marker(folder: &Path) -> io::Result<Option<[u8; 16]>> {
    let path = folder.join(MARKER);
    match std::fs::symlink_metadata(&path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
        Ok(m) if !m.is_file() || m.len() > 64 => return Ok(None),
        Ok(_) => {}
    }
    let text = match std::fs::read_to_string(&path) {
        Err(e) if e.kind() == io::ErrorKind::InvalidData => return Ok(None),
        r => r?,
    };
    Ok(text
        .strip_prefix(MARKER_HEAD)
        .and_then(|t| t.strip_suffix('\n'))
        .and_then(RecId::parse)
        .map(|id| id.0))
}

/// The marker id in `folder` (the card's `MPFree`), made if it has none
/// or one that doesn't read: a new random id written whole under another
/// name, synced, renamed into place and the folder synced, so a cut
/// leaves the old file or the new, never half of one. Made once for a
/// card, it stays: nothing changes or removes it.
pub fn make_marker(folder: &Path) -> io::Result<[u8; 16]> {
    if let Some(id) = read_marker(folder)? {
        return Ok(id);
    }
    let id = RecId::random();
    let new = folder.join(format!("{MARKER}.new"));
    {
        use std::io::Write;
        let mut f = File::create(&new)?;
        f.write_all(format!("{MARKER_HEAD}{}\n", id.hex()).as_bytes())?;
        f.sync_all()?;
    }
    std::fs::rename(&new, folder.join(MARKER))?;
    File::open(folder)?.sync_all()?;
    Ok(id.0)
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

/// One opening of the card, as the app's SD card slot hears it: made when
/// the card's window opens it (every check passed), and revoked when the
/// slot hears it closed (taken out, closed by the system, or put away).
/// An opening heard again while the card is still open is the same
/// lease, so it is never taken for a new insertion.
#[derive(Debug)]
pub struct Lease {
    root: PathBuf,
    valid: AtomicBool,
    mark: OnceLock<CardMark>,
    users: AtomicUsize,
}

impl Lease {
    /// The card opened with its files at `root` (its mount point).
    pub fn new(root: &Path) -> Arc<Lease> {
        Arc::new(Lease {
            root: root.to_path_buf(),
            valid: AtomicBool::new(true),
            mark: OnceLock::new(),
            users: AtomicUsize::new(0),
        })
    }

    /// How many files on this opening are open to play from, as the reader
    /// last said ([`CardHandle::set_users`]): while any are, the card is in
    /// use.
    pub fn users(&self) -> usize {
        self.users.load(Ordering::Acquire)
    }

    /// Which card this opening is, once its marker is read or made: said
    /// once (false, and nothing changes, if it was said before).
    pub fn set_mark(&self, mark: CardMark) -> bool {
        self.mark.set(mark).is_ok()
    }

    /// The card closed: nothing more is read on this lease, at once and
    /// for good.
    pub fn revoke(&self) {
        self.valid.store(false, Ordering::Release);
    }
}

impl CardHandle for Lease {
    fn root(&self) -> &Path {
        &self.root
    }
    fn valid(&self) -> bool {
        self.valid.load(Ordering::Acquire)
    }
    fn mark(&self) -> Option<CardMark> {
        self.mark.get().copied()
    }
    fn set_users(&self, n: usize) {
        self.users.store(n, Ordering::Release);
    }
}

/// Leases are told apart by which opening they are, not by where.
impl PartialEq for Lease {
    fn eq(&self, other: &Lease) -> bool {
        std::ptr::eq(self, other)
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
