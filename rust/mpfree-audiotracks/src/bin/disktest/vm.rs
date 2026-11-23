//! The kernel's writeback limit, for `--recorder` (D4 in
//! docs/dev/design/disk-streaming.md): how much written data Linux may hold
//! in memory before a writer has to wait for the stick
//! (`/proc/sys/vm/dirty_bytes`).
//!
//! The run reads the four writeback settings first and changes only one,
//! `dirty_bytes`, and only when the kernel already counts its limit in
//! bytes (not as a share of memory) and that limit is under 32 MiB: for two
//! steps it is 32 MiB, then the value found is written back, and read back
//! to check. That happens at the end of those steps, after an error, on
//! Ctrl-C and when the ssh session closes (a [`Raised`] put back when it is
//! dropped). It lives in the running kernel only: a restart puts it back
//! too, whatever happened to the program.
//!
//! On the MPC's stick the limit found was 1 MiB (D0, 22 Nov), so a cap on
//! the stick's own share (`strict_limit` and `max_bytes` on its `bdi`, the
//! plan's first idea) could change nothing: the kernel's own limit is far
//! below it. D4 tries the other way: whether letting more wait in memory
//! helps a recording keep up, or makes reads wait longer.

use std::fs;
use std::path::{Path, PathBuf};

/// Where the settings are (a desktop test may name a stand-in).
pub(super) const VM: &str = "/proc/sys/vm";
/// The raised limit: what the first-start backup copy already sets on the
/// MPC while it copies (tools/mpc-one/stick/setup-backup.sh).
pub(super) const RAISED: u64 = 32 << 20;

/// The four writeback settings, as the kernel gives them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Writeback {
    pub(super) bytes: u64,
    pub(super) ratio: u64,
    pub(super) background_bytes: u64,
    pub(super) background_ratio: u64,
}

fn value(dir: &Path, name: &str) -> Result<u64, String> {
    let p = dir.join(name);
    let text = fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
    text.trim()
        .parse()
        .map_err(|_| format!("{}: {:?} is not a number", p.display(), text.trim()))
}

/// Read the settings (read-only).
pub(super) fn read(dir: &Path) -> Result<Writeback, String> {
    Ok(Writeback {
        bytes: value(dir, "dirty_bytes")?,
        ratio: value(dir, "dirty_ratio")?,
        background_bytes: value(dir, "dirty_background_bytes")?,
        background_ratio: value(dir, "dirty_background_ratio")?,
    })
}

fn size(b: u64) -> String {
    if b >= 1 << 20 {
        format!("{:.1} MiB", b as f64 / (1u64 << 20) as f64)
    } else {
        format!("{:.1} KiB", b as f64 / 1024.0)
    }
}

/// "at most 1.0 MiB waiting (dirty_bytes 1048576), written out from
/// 50.0 KiB (dirty_background_bytes 51200)".
pub(super) fn describe(w: &Writeback) -> String {
    let limit = if w.bytes > 0 {
        format!(
            "at most {} waiting (dirty_bytes {})",
            size(w.bytes),
            w.bytes
        )
    } else {
        format!(
            "at most {} % of memory waiting (dirty_ratio {})",
            w.ratio, w.ratio
        )
    };
    let from = if w.background_bytes > 0 {
        format!(
            "written out from {} (dirty_background_bytes {})",
            size(w.background_bytes),
            w.background_bytes
        )
    } else {
        format!(
            "written out from {} % of memory (dirty_background_ratio {})",
            w.background_ratio, w.background_ratio
        )
    };
    format!("{limit}, {from}")
}

/// Why the limit is left as it is, if it is.
pub(super) fn not_raised(w: &Writeback) -> Option<String> {
    if w.bytes == 0 {
        Some(format!(
            "the kernel's limit is a share of memory (dirty_ratio {} %), not a size, so it is left as it is",
            w.ratio
        ))
    } else if w.bytes >= RAISED {
        Some(format!(
            "the kernel's limit is already {}, so it is left as it is",
            size(w.bytes)
        ))
    } else {
        None
    }
}

/// `dirty_bytes` raised for a while; put back as found when dropped.
pub(super) struct Raised {
    dir: PathBuf,
    found: Writeback,
    back: bool,
}

impl Raised {
    /// Raise `dirty_bytes` to [`RAISED`] and check it took. Refused (and
    /// nothing written) where [`not_raised`] says so; if it doesn't read
    /// back as written, the found value is written back first.
    pub(super) fn raise(dir: &Path, found: Writeback) -> Result<Raised, String> {
        if let Some(why) = not_raised(&found) {
            return Err(why);
        }
        let p = dir.join("dirty_bytes");
        let mut r = Raised {
            dir: dir.to_path_buf(),
            found,
            back: false,
        };
        fs::write(&p, format!("{RAISED}\n")).map_err(|e| {
            // Nothing was changed if the write itself failed.
            r.back = true;
            format!("{}: {e}", p.display())
        })?;
        match value(dir, "dirty_bytes") {
            Ok(v) if v == RAISED => Ok(r),
            Ok(v) => {
                let back = r.put_back();
                Err(format!(
                    "dirty_bytes read {v} after writing {RAISED}{}",
                    back.err().map_or(String::new(), |e| format!("; {e}"))
                ))
            }
            Err(e) => {
                let back = r.put_back();
                Err(format!(
                    "{e}{}",
                    back.err().map_or(String::new(), |e| format!("; {e}"))
                ))
            }
        }
    }

    /// Write the found `dirty_bytes` back and check all four settings are
    /// as found.
    pub(super) fn put_back(&mut self) -> Result<(), String> {
        if self.back {
            return Ok(());
        }
        let p = self.dir.join("dirty_bytes");
        fs::write(&p, format!("{}\n", self.found.bytes))
            .map_err(|e| format!("{}: {e}; a restart puts it back", p.display()))?;
        let now = read(&self.dir)?;
        if now != self.found {
            return Err(format!(
                "the writeback settings read {} after putting them back, not {}; a restart puts them back",
                describe(&now),
                describe(&self.found)
            ));
        }
        self.back = true;
        Ok(())
    }
}

impl Drop for Raised {
    fn drop(&mut self) {
        if let Err(e) = self.put_back() {
            esay!("mpfree-disktest: could not put the writeback limit back: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stand_in(name: &str, bytes: u64, ratio: u64) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("mpfree-disktest-vm-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        for (n, v) in [
            ("dirty_bytes", bytes),
            ("dirty_ratio", ratio),
            ("dirty_background_bytes", 51_200),
            ("dirty_background_ratio", 0),
        ] {
            fs::write(d.join(n), format!("{v}\n")).unwrap();
        }
        d
    }

    #[test]
    fn the_limit_is_raised_and_put_back_as_found() {
        let d = stand_in("raise", 1 << 20, 0);
        let found = read(&d).unwrap();
        assert_eq!(
            describe(&found),
            "at most 1.0 MiB waiting (dirty_bytes 1048576), written out from 50.0 KiB (dirty_background_bytes 51200)"
        );
        {
            let _r = Raised::raise(&d, found).unwrap();
            assert_eq!(read(&d).unwrap().bytes, RAISED);
        }
        // Dropped: as found.
        assert_eq!(read(&d).unwrap(), found);
        let mut r = Raised::raise(&d, found).unwrap();
        r.put_back().unwrap();
        assert_eq!(read(&d).unwrap(), found);
        drop(r);
        assert_eq!(read(&d).unwrap(), found);
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_share_of_memory_or_a_big_limit_is_left_alone() {
        let d = stand_in("ratio", 0, 20);
        let found = read(&d).unwrap();
        let why = Raised::raise(&d, found).err().unwrap();
        assert!(why.contains("share of memory (dirty_ratio 20 %)"), "{why}");
        assert_eq!(read(&d).unwrap(), found);
        fs::remove_dir_all(&d).unwrap();
        let d = stand_in("big", 64 << 20, 0);
        let found = read(&d).unwrap();
        let why = Raised::raise(&d, found).err().unwrap();
        assert!(why.contains("already 64.0 MiB"), "{why}");
        assert_eq!(read(&d).unwrap(), found);
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn settings_that_dont_read_are_an_error() {
        let d = stand_in("bad", 1 << 20, 0);
        fs::write(d.join("dirty_ratio"), "lots\n").unwrap();
        assert!(read(&d).unwrap_err().contains("is not a number"));
        fs::remove_file(d.join("dirty_ratio")).unwrap();
        assert!(read(&d).is_err());
        fs::remove_dir_all(&d).unwrap();
    }
}
