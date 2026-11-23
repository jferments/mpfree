//! Checked copies (disk streaming's P4, step 4): a file from the SD card
//! copied onto the stick, which plays it, so a card pulled mid-song can't
//! stop a stem.
//!
//! Each copy is published whole or not at all ([`crate::publish`]): it is
//! written to a unique `NAME.<job>.part` (never over a file), hashed as
//! it goes (the hash of exactly what the card gave), synced, read back
//! fresh and checked against that hash, renamed into place and its folder
//! synced. A copy is named by its
//! content (`<hash>-<name>`), so a newer version of a file on the card is
//! a new copy beside the old one, and the old one stays while anything
//! still refers to it: clearing copies nothing refers to is the managed
//! files' cleanup, after a save. A project refers only to the path a
//! finished copy gives ([`Checked`]), so never to a file before its copy
//! is checked.
//!
//! A card pulled mid-copy stops it, and the part file goes. Nothing is
//! ever resumed: a part file a cut left is removed at start
//! ([`crate::publish::sweep_parts`]), and the next copy, from any card,
//! starts from the beginning. Copies go a block at a time, asking before
//! each block whether to go on ([`Pace`]): they wait while anything
//! records or playback's cushion is short ([`background`]), and run at
//! the lowest I/O priority (set with the copy's thread in P5's tuning).

use std::cell::Cell;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::fsio::Fs;
use crate::publish::{hex, publish_as, Sink};
use crate::storage::Located;
use crate::writer::Gate;

/// Bytes copied a block at a time.
const BLOCK: usize = 256 * 1024;
/// How long a copy waits before asking again.
const WAIT: Duration = Duration::from_millis(20);
/// Hex digits of the content hash in a copy's name.
const NAME_HEX: usize = 16;

/// Whether a copy goes on with its next block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pace {
    Go,
    /// Not now: ask again shortly.
    Wait,
    /// Stop the copy (the musician cancelled it, or the app is closing).
    Stop,
}

/// The pace for copies behind playback and recording: wait while a take
/// has captured audio not yet saved (on the MPC's stick a heavy copy beside
/// a recording put it 24 s behind in 45 s, D2b), or while playback's
/// cushion is under `cushion_s` seconds.
pub fn background(gate: &Gate, cushion_s: f64) -> Pace {
    if gate.recording() || gate.cushion() < cushion_s {
        Pace::Wait
    } else {
        Pace::Go
    }
}

/// A finished, checked copy: the only path a project may refer to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Checked {
    pub path: PathBuf,
    /// SHA-256 of its bytes: what the card gave, read back from the stick.
    pub hash: [u8; 32],
    pub size: u64,
    /// The same content was already copied: that copy is used.
    pub existed: bool,
}

/// Why a copy didn't happen. Nothing is left behind by any of them.
#[derive(Debug, PartialEq, Eq)]
pub enum Refused {
    /// The card it was found on went (before or during the copy).
    CardGone,
    /// Not enough room on the stick beside what is kept for recording
    /// and saves.
    Space {
        need: u64,
        free: u64,
        keep: u64,
    },
    /// The file on the card isn't what it was when the copy began.
    Changed(String),
    /// Stopped by its [`Pace`].
    Stopped,
    /// Not a file name a copy can have.
    Name(String),
    Io(String),
}

impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mb = |b: u64| format!("{} MB", b.div_ceil(1_000_000));
        match self {
            Refused::CardGone => write!(f, "the SD card was removed: copy stopped"),
            Refused::Space { need, free, keep } => write!(
                f,
                "not enough room on the stick: the copy needs about {}, {} free, {} kept for \
                 recording and saves",
                mb(*need),
                mb(*free),
                mb(*keep)
            ),
            Refused::Changed(n) => write!(f, "{n}: changed while it was copied"),
            Refused::Stopped => write!(f, "copy stopped"),
            Refused::Name(n) => write!(f, "{n}: not a name a copy can have"),
            Refused::Io(e) => write!(f, "{e}"),
        }
    }
}

/// A copy's settings.
pub struct Copy<'a> {
    /// Reads the card.
    pub src: &'a mut dyn Fs,
    /// Writes the stick.
    pub fs: &'a mut dyn Fs,
    /// The managed folder copies go in.
    pub dir: &'a Path,
    /// Makes the part file's name unique.
    pub job: u32,
    /// Bytes free on the stick now.
    pub free: u64,
    /// Bytes kept free for recording and saves.
    pub keep: u64,
    /// Asked before each block.
    pub pace: &'a mut dyn FnMut() -> Pace,
}

/// The name a copy of `name` with content `hash` has.
pub fn copy_name(hash: &[u8; 32], name: &str) -> String {
    format!("{}-{name}", &hex(hash)[..NAME_HEX])
}

/// Copy the file `loc` found (on the card, or anywhere the storage manager
/// finds files) into `c.dir`, checked, or find the same content already
/// copied there.
pub fn copy(c: &mut Copy, loc: &Located) -> Result<Checked, Refused> {
    if !loc.valid() {
        return Err(Refused::CardGone);
    }
    let name = loc
        .path
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|n| !n.starts_with('.'))
        .ok_or_else(|| Refused::Name(loc.path.display().to_string()))?
        .to_string();
    let io = |e: io::Error| Refused::Io(format!("{name}: {e}"));
    let size = c.src.len(&loc.path).map_err(io)?;
    if size.saturating_add(c.keep) > c.free {
        return Err(Refused::Space {
            need: size,
            free: c.free,
            keep: c.keep,
        });
    }
    // Why the writer stopped, when it wasn't the stick.
    let why: Cell<Option<Refused>> = Cell::new(None);
    let mut buf = vec![0u8; BLOCK];
    let (src, pace) = (&mut *c.src, &mut *c.pace);
    let stop = |r: Refused| {
        why.set(Some(r));
        io::Error::other("copy stopped")
    };
    let mut write = |sink: &mut Sink| -> io::Result<()> {
        let mut at = 0u64;
        loop {
            loop {
                if !loc.valid() {
                    return Err(stop(Refused::CardGone));
                }
                match pace() {
                    Pace::Go => break,
                    Pace::Wait => std::thread::sleep(WAIT),
                    Pace::Stop => return Err(stop(Refused::Stopped)),
                }
            }
            let n = src.read_at(&loc.path, at, &mut buf)?;
            // Read while the card was still the one it was found on.
            if !loc.valid() {
                return Err(stop(Refused::CardGone));
            }
            if n == 0 {
                break;
            }
            sink(&buf[..n])?;
            at += n as u64;
            if at > size {
                break;
            }
        }
        if at != size {
            return Err(stop(Refused::Changed(name.clone())));
        }
        Ok(())
    };
    let made = publish_as(c.fs, c.dir, &name, c.job, &mut write, &|h| {
        copy_name(h, &name)
    });
    let made = match made {
        Ok(m) => m,
        Err(e) => return Err(why.take().unwrap_or_else(|| io(e))),
    };
    Ok(Checked {
        path: made.path,
        hash: made.hash,
        size: made.size,
        existed: made.existed,
    })
}
