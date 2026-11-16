//! `mpfree copy-image FROM TO`: the copy step of a creator stick's
//! first-start backup ([USB creator design](../../../docs/dev/design/usb-creator.md)),
//! run by the stick's root service `setup-backup.sh` inside its read-only
//! window onto the eMMC.
//!
//! It copies FROM (an eMMC disk, opened read-only) into a new file TO on
//! the stick's partition 4, a MiB at a time, and leaves every MiB that is
//! all zeros as a hole instead of writing it. The file reads back exactly
//! as the disk (a hole reads as zeros), so its SHA-256 is the disk's, but
//! the stick only has to write the MiBs that hold something. On the MPC
//! One about half of the eMMC is unused space, and the stick's writes are
//! what the backup waits on (about 7 MB/s on the MPC's USB).
//!
//! - FROM is only ever opened with `File::open` (read-only). The service
//!   has also made every eMMC disk read-only in the kernel first.
//! - TO must not exist: it is created new (`create_new`), so nothing is
//!   overwritten and a link in its place is not followed.
//! - TO's size grows as the copy goes, holes included, so the service can
//!   show progress from it as it did with `dd`.
//! - At the end the file is synced, and one line on standard output says
//!   what was copied: `copied BYTES bytes: WRITTEN written, ZEROS left as
//!   holes`.
//!
//! It runs before anything else in `main`: no report file, no settings.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{FileExt, OpenOptionsExt};
use std::path::Path;

/// The command's name after `mpfree`.
pub const SUBCOMMAND: &str = "copy-image";

/// The block it reads, and the unit it leaves as a hole when all zeros.
const BLOCK: usize = 1 << 20;

/// What a copy did.
#[derive(Debug, PartialEq, Eq)]
pub struct Copied {
    pub bytes: u64,
    pub written: u64,
    pub zeros: u64,
}

fn all_zero(b: &[u8]) -> bool {
    let words = b.chunks_exact(8);
    let tail = words.remainder();
    words
        .into_iter()
        .all(|w| u64::from_ne_bytes(w.try_into().unwrap()) == 0)
        && tail.iter().all(|&x| x == 0)
}

/// Reads until `buf` is full or the end; returns how many bytes.
fn fill(r: &mut impl Read, buf: &mut [u8]) -> io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match r.read(&mut buf[n..]) {
            Ok(0) => break,
            Ok(k) => n += k,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(n)
}

/// Copies `from` (read-only) into the new file `to`, with holes for blocks
/// of zeros.
pub fn copy(from: &Path, to: &Path) -> io::Result<Copied> {
    let mut src = File::open(from)
        .map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", from.display())))?;
    let out = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o640)
        .open(to)
        .map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", to.display())))?;
    let mut buf = vec![0u8; BLOCK];
    let mut done = Copied {
        bytes: 0,
        written: 0,
        zeros: 0,
    };
    loop {
        let n = fill(&mut src, &mut buf)
            .map_err(|e| io::Error::new(e.kind(), format!("reading {}: {e}", from.display())))?;
        if n == 0 {
            break;
        }
        let block = &buf[..n];
        if all_zero(block) {
            done.zeros += n as u64;
            // A hole: only the size grows, so progress shows.
            out.set_len(done.bytes + n as u64)?;
        } else {
            out.write_all_at(block, done.bytes)?;
            done.written += n as u64;
        }
        done.bytes += n as u64;
    }
    out.set_len(done.bytes)?;
    out.sync_all()?;
    Ok(done)
}

/// `mpfree copy-image FROM TO`; returns the exit code.
pub fn main(args: impl IntoIterator<Item = std::ffi::OsString>) -> i32 {
    let args: Vec<_> = args.into_iter().collect();
    if args.len() != 2 {
        eprintln!("usage: mpfree {SUBCOMMAND} FROM TO   (TO must not exist)");
        return 2;
    }
    match copy(Path::new(&args[0]), Path::new(&args[1])) {
        Ok(c) => {
            let mut o = io::stdout().lock();
            let _ = writeln!(
                o,
                "copied {} bytes: {} written, {} left as holes",
                c.bytes, c.written, c.zeros
            );
            0
        }
        Err(e) => {
            eprintln!("copy-image: {e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::MetadataExt;

    fn dir(name: &str) -> std::path::PathBuf {
        let d =
            std::env::temp_dir().join(format!("mpfree-imagecopy-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn copies_exactly_and_leaves_zeros_as_holes() {
        let d = dir("holes");
        let from = d.join("disk");
        // 1 MiB of data, 3 MiB of zeros, 1 MiB of data, then a short tail of zeros.
        let mut disk = vec![0u8; 5 * BLOCK + 1000];
        for (i, b) in disk[..BLOCK].iter_mut().enumerate() {
            *b = (i % 251) as u8 + 1;
        }
        disk[4 * BLOCK + 7] = 9;
        std::fs::write(&from, &disk).unwrap();
        let to = d.join("disk.img");
        let c = copy(&from, &to).unwrap();
        assert_eq!(
            c,
            Copied {
                bytes: disk.len() as u64,
                written: 2 * BLOCK as u64,
                zeros: 3 * BLOCK as u64 + 1000,
            }
        );
        assert_eq!(std::fs::read(&to).unwrap(), disk, "reads back as the disk");
        let m = std::fs::metadata(&to).unwrap();
        assert_eq!(m.mode() & 0o777, 0o640);
        // Where the filesystem keeps holes (ext4, tmpfs), fewer blocks than the size.
        assert!(m.blocks() * 512 <= disk.len() as u64);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn never_overwrites_and_never_writes_the_source() {
        let d = dir("never");
        let from = d.join("disk");
        std::fs::write(&from, vec![7u8; 3000]).unwrap();
        let to = d.join("there");
        std::fs::write(&to, b"keep").unwrap();
        assert!(copy(&from, &to).is_err(), "TO exists");
        assert_eq!(std::fs::read(&to).unwrap(), b"keep");
        // A link where TO goes is not followed.
        let link = d.join("link");
        std::os::unix::fs::symlink(d.join("elsewhere"), &link).unwrap();
        assert!(copy(&from, &link).is_err());
        assert!(!d.join("elsewhere").exists());
        // The source can be read-only to us: it is only opened for reading.
        let mut p = std::fs::metadata(&from).unwrap().permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut p, 0o444);
        std::fs::set_permissions(&from, p).unwrap();
        let c = copy(&from, &d.join("ok.img")).unwrap();
        assert_eq!((c.bytes, c.written, c.zeros), (3000, 3000, 0));
        assert_eq!(std::fs::read(&from).unwrap(), vec![7u8; 3000]);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn an_empty_source_and_bad_arguments() {
        let d = dir("empty");
        std::fs::write(d.join("disk"), b"").unwrap();
        let c = copy(&d.join("disk"), &d.join("e.img")).unwrap();
        assert_eq!((c.bytes, c.written, c.zeros), (0, 0, 0));
        assert_eq!(std::fs::metadata(d.join("e.img")).unwrap().len(), 0);
        assert!(copy(&d.join("missing"), &d.join("m.img")).is_err());
        assert!(
            !d.join("m.img").exists(),
            "nothing made when the source is missing"
        );
        assert_eq!(main(Vec::<std::ffi::OsString>::new()), 2);
        assert_eq!(main(["a".into(), "b".into(), "c".into()]), 2);
        assert!(all_zero(&[0u8; 13]) && !all_zero(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 1]));
        let _ = std::fs::remove_dir_all(&d);
    }
}
