//! Publishing a file the safe way (disk streaming's P3, step 4): prepared
//! copies, staged copies and the card's checked copies (P4,
//! [`crate::cardcopy`]).
//!
//! A published file appears whole or not at all, and is never written
//! over: it is written to a unique `NAME.<job>.part` (made new, never over
//! a file already there), synced, read back from the device past the
//! system's cache where it allows ([`Fs::read_fresh`]) and checked against
//! the hash taken as it was written, renamed into place,
//! and its folder synced. Files published here are named by their content
//! or by what made them, so a name already there with the same content is
//! the same file: it is kept and the new one dropped. A name already there
//! with other content is refused, never replaced. Everything goes through
//! [`crate::fsio::Fs`], so the tests cut a model stick after every step.

use std::io;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::fsio::Fs;

/// Bytes read at a time when reading back or hashing.
const CHUNK: usize = 256 * 1024;

/// Where a file's bytes go, in order.
pub type Sink<'a> = dyn FnMut(&[u8]) -> io::Result<()> + 'a;
/// What writes a file: handed a [`Sink`], it calls it with the file's
/// bytes in order.
pub type Writer<'a> = dyn FnMut(&mut Sink) -> io::Result<()> + 'a;

/// A file published, or found already there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Published {
    pub path: PathBuf,
    /// SHA-256 of its bytes.
    pub hash: [u8; 32],
    pub size: u64,
    /// It was already there, with the same content.
    pub existed: bool,
}

/// Lower-case hex.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// SHA-256 of a whole file, and its length, read through `fs`.
pub fn hash_file(fs: &mut dyn Fs, path: &Path) -> io::Result<([u8; 32], u64)> {
    hash_with(fs, path, false)
}

/// As [`hash_file`], read from the device past the system's cache where
/// it allows ([`Fs::read_fresh`]).
pub fn hash_file_fresh(fs: &mut dyn Fs, path: &Path) -> io::Result<([u8; 32], u64)> {
    hash_with(fs, path, true)
}

fn hash_with(fs: &mut dyn Fs, path: &Path, fresh: bool) -> io::Result<([u8; 32], u64)> {
    let mut h = Sha256::new();
    let mut buf = vec![0u8; CHUNK];
    let mut at = 0u64;
    loop {
        let n = if fresh {
            fs.read_fresh(path, at, &mut buf)?
        } else {
            fs.read_at(path, at, &mut buf)?
        };
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
        at += n as u64;
        if n < buf.len() {
            break;
        }
    }
    Ok((h.finalize().into(), at))
}

/// A name with no folder in it, not hidden.
fn plain(name: &str) -> io::Result<()> {
    if name.is_empty() || name.contains('/') || name.contains('\\') || name.starts_with('.') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{name:?} isn't a plain file name"),
        ));
    }
    Ok(())
}

/// Publish `dir/name` from what `write` produces: it is handed a sink and
/// calls it with the file's bytes in order. `job` makes the part file's
/// name unique. On any failure the part file is removed (as far as the
/// stick allows) and nothing is published.
pub fn publish(
    fs: &mut dyn Fs,
    dir: &Path,
    name: &str,
    job: u32,
    write: &mut Writer,
) -> io::Result<Published> {
    plain(name)?;
    publish_as(fs, dir, name, job, write, &|_| name.to_string())
}

/// As [`publish`], with the name chosen from the content's hash once it
/// is written (`name_of`), so a copy named by its content takes one pass
/// over the original. The part file is `dir/<part>.<job>.part`.
pub fn publish_as(
    fs: &mut dyn Fs,
    dir: &Path,
    part: &str,
    job: u32,
    write: &mut Writer,
    name_of: &dyn Fn(&[u8; 32]) -> String,
) -> io::Result<Published> {
    plain(part)?;
    let part = dir.join(format!("{part}.{job}.part"));
    let result = (|| {
        fs.create(&part, true)?;
        let mut h = Sha256::new();
        let mut at = 0u64;
        write(&mut |bytes: &[u8]| {
            fs.write_at(&part, at, bytes)?;
            h.update(bytes);
            at += bytes.len() as u64;
            Ok(())
        })?;
        let hash: [u8; 32] = h.finalize().into();
        fs.sync(&part)?;
        let name = name_of(&hash);
        plain(&name)?;
        let path = dir.join(&name);
        let (back, len) = hash_file_fresh(fs, &part)?;
        if back != hash || len != at {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{name}: read back different from what was written"),
            ));
        }
        if fs.exists(&path) {
            let (there, there_len) = hash_file(fs, &path)?;
            if there == hash && there_len == at {
                fs.remove(&part)?;
                return Ok(Published {
                    path,
                    hash,
                    size: at,
                    existed: true,
                });
            }
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("{name} is already there with other content: not replaced"),
            ));
        }
        fs.rename(&part, &path)?;
        fs.sync_dir(dir)?;
        Ok(Published {
            path,
            hash,
            size: at,
            existed: false,
        })
    })();
    if result.is_err() && fs.exists(&part) {
        let _ = fs.remove(&part);
    }
    result
}

/// Remove the part files a cut left in `dir` (`NAME.<job>.part`), and
/// sync the folder: at start, before any job writes there. A part file is
/// never finished later, so a copy a cut stopped starts again from the
/// beginning. Gives how many went.
pub fn sweep_parts(fs: &mut dyn Fs, dir: &Path) -> io::Result<usize> {
    let mut gone = 0;
    for name in fs.list(dir)? {
        let is_part = name
            .strip_suffix(".part")
            .and_then(|n| n.rsplit_once('.'))
            .is_some_and(|(stem, job)| {
                !stem.is_empty() && !job.is_empty() && job.bytes().all(|b| b.is_ascii_digit())
            });
        if is_part {
            fs.remove(&dir.join(&name))?;
            gone += 1;
        }
    }
    if gone > 0 {
        fs.sync_dir(dir)?;
    }
    Ok(gone)
}

/// Write `dir/name` whole, replacing what is there: for the few files
/// MPFree keeps up to date itself (a set's snapshot), never for audio. The
/// same steps as [`publish`], but the rename goes over the old file, so a
/// reader sees the old one or the new one, never half of either.
pub fn replace(fs: &mut dyn Fs, dir: &Path, name: &str, job: u32, bytes: &[u8]) -> io::Result<()> {
    let path = dir.join(name);
    let part = dir.join(format!("{name}.{job}.part"));
    let result = (|| {
        fs.create(&part, true)?;
        fs.write_at(&part, 0, bytes)?;
        fs.sync(&part)?;
        let (back, len) = hash_file_fresh(fs, &part)?;
        let want: [u8; 32] = Sha256::digest(bytes).into();
        if back != want || len != bytes.len() as u64 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{name}: read back different from what was written"),
            ));
        }
        fs.rename(&part, &path)?;
        fs.sync_dir(dir)
    })();
    if result.is_err() && fs.exists(&part) {
        let _ = fs.remove(&part);
    }
    result
}
