//! The MPFree USB creator's core: an MPFree boot stick for the MPC One
//! (Gen 1), made from the owner's own Akai update file, on Linux, macOS and
//! Windows, with no outside tools.
//!
//! - [`update`]: reads Akai's update file (the AZ01 container) and unpacks
//!   its root filesystem, checking its hashes.
//! - [`recipe`]: the changes MPFree makes to that filesystem, recorded from
//!   `offline-kexec-stick.py`'s own `build_rootfs`, so the two never drift.
//! - [`stick`]: builds the stick's partitions (ext2 root, log and kits, the
//!   FAT32 setup partition), writes them, copies the eMMC backup back off a
//!   stick, and switches the setup partition between install and remove.
//! - [`disks`]: finds USB sticks and opens one for writing, on each system.
//! - [`ssh`]: developer SSH keys (off unless asked for).
//!
//! - [`cli`]: the command line over all of this: the `mpfree-stick`
//!   program, and what the creator's window (`mpfree-creator`) runs itself
//!   as, with the rights to write the stick.
//!
//! Nothing here ever reads or writes the MPC itself.

pub mod cli;
pub mod disks;
pub mod dtb;
pub mod recipe;
pub mod ssh;
pub mod stick;
pub mod update;

use std::io;

/// What a long step tells its caller: a line for the log, or how far along
/// it is (0 to 1) and what it is doing.
#[derive(Clone, Debug, PartialEq)]
pub enum Msg {
    Note(String),
    Progress(f32, String),
}

impl Msg {
    /// One line for the window to read back: `note TEXT` or `progress F TEXT`.
    pub fn line(&self) -> String {
        match self {
            Msg::Note(t) => format!("note {}", t.replace('\n', " ")),
            Msg::Progress(f, t) => {
                format!("progress {:.4} {}", f.clamp(0.0, 1.0), t.replace('\n', " "))
            }
        }
    }

    /// The other way: a line from [`Msg::line`] (and `error TEXT` /
    /// `done TEXT`, which the program writes at its end, as notes).
    pub fn parse(line: &str) -> Option<Msg> {
        let (kind, rest) = line.split_once(' ').unwrap_or((line, ""));
        match kind {
            "note" | "error" | "done" => Some(Msg::Note(rest.to_string())),
            "progress" => {
                let (f, t) = rest.split_once(' ').unwrap_or((rest, ""));
                Some(Msg::Progress(f.parse().ok()?, t.to_string()))
            }
            _ => None,
        }
    }
}

pub(crate) fn bad(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

/// The creator's name and version, for made-by.txt and the window.
pub const CREATOR: &str = concat!("MPFree USB creator ", env!("CARGO_PKG_VERSION"));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_go_through_a_line_and_back() {
        for m in [
            Msg::Note("hello there".into()),
            Msg::Progress(0.5, "Writing the stick".into()),
        ] {
            assert_eq!(Msg::parse(&m.line()), Some(m));
        }
        assert_eq!(
            Msg::parse("error it broke"),
            Some(Msg::Note("it broke".into()))
        );
        assert_eq!(Msg::parse("progress x y"), None);
        assert_eq!(Msg::parse("something else"), None);
    }
}
