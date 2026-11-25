//! The one small file layer every recording operation goes through, so a
//! test can stand a model stick in for the real one (`testkit::ModelStick`):
//! create, write, read, cut to length, sync, rename, remove and sync a
//! folder. Paths are whole; a real file is opened once and kept open until
//! it is renamed or removed. The SD card is read through a read-only one
//! ([`StdFs::read_only`]): it is mounted read-only, and nothing here
//! writes to it.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// The file operations recording uses.
pub trait Fs {
    /// Make an empty file; with `new_only`, fail if one has that name,
    /// otherwise empty the one there.
    fn create(&mut self, path: &Path, new_only: bool) -> io::Result<()>;
    /// Write all of `bytes` at `at`. An error or a short write is an
    /// error; how much of it landed is unknown.
    fn write_at(&mut self, path: &Path, at: u64, bytes: &[u8]) -> io::Result<()>;
    /// Read up to `buf.len()` bytes at `at`; fewer only at the end.
    fn read_at(&mut self, path: &Path, at: u64, buf: &mut [u8]) -> io::Result<usize>;
    /// As [`Fs::read_at`], but from the device past the system's cache
    /// where it allows, to check what a published file really holds.
    fn read_fresh(&mut self, path: &Path, at: u64, buf: &mut [u8]) -> io::Result<usize> {
        self.read_at(path, at, buf)
    }
    fn len(&mut self, path: &Path) -> io::Result<u64>;
    fn set_len(&mut self, path: &Path, len: u64) -> io::Result<()>;
    /// `fdatasync`: the file's data and length are on the stick.
    fn sync(&mut self, path: &Path) -> io::Result<()>;
    /// The folder's names are on the stick.
    fn sync_dir(&mut self, dir: &Path) -> io::Result<()>;
    fn rename(&mut self, from: &Path, to: &Path) -> io::Result<()>;
    fn remove(&mut self, path: &Path) -> io::Result<()>;
    /// The names of the files in `dir`.
    fn list(&mut self, dir: &Path) -> io::Result<Vec<String>>;
    fn exists(&mut self, path: &Path) -> bool;
    /// Let go of the file at `path` if it is held open: its drive went
    /// (the SD card taken out), so nothing more is done with it. Touches
    /// nothing on any drive.
    fn let_go(&mut self, _path: &Path) {}
}

/// The real file system.
#[derive(Default)]
pub struct StdFs {
    open: HashMap<PathBuf, File>,
    read_only: bool,
    fresh_fallbacks: u64,
}

/// Fresh reads are made in blocks of this size, at offsets and into
/// memory aligned to it (what `O_DIRECT` asks; Linux only).
#[cfg(target_os = "linux")]
const ALIGN: usize = 4096;

impl StdFs {
    pub fn new() -> Self {
        Self::default()
    }

    /// Files opened for reading only, and every change refused: for the
    /// SD card, mounted read-only.
    pub fn read_only() -> Self {
        StdFs {
            read_only: true,
            ..Self::default()
        }
    }

    /// Fresh reads that had to go through the system's cache instead (the
    /// file system doesn't read past it, or the read wasn't aligned): a
    /// check made that way proves less, and the app shows the count.
    pub fn fresh_fallbacks(&self) -> u64 {
        self.fresh_fallbacks
    }

    fn file(&mut self, path: &Path) -> io::Result<&mut File> {
        if !self.open.contains_key(path) {
            // Spelled out, so the MPC write check (tools/mpc-one) sees the
            // one open for writing.
            if self.read_only {
                let f = File::open(path)?;
                self.open.insert(path.to_path_buf(), f);
            } else {
                let f = OpenOptions::new().read(true).write(true).open(path)?;
                self.open.insert(path.to_path_buf(), f);
            }
        }
        Ok(self.open.get_mut(path).expect("just opened"))
    }

    fn writable(&self, path: &Path) -> io::Result<()> {
        if self.read_only {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("{}: read-only here", path.display()),
            ));
        }
        Ok(())
    }

    /// A read past the page cache (`O_DIRECT`), or None where the file
    /// system doesn't allow one.
    #[cfg(target_os = "linux")]
    fn direct(path: &Path, at: u64, buf: &mut [u8]) -> io::Result<Option<usize>> {
        use std::os::unix::fs::{FileExt, OpenOptionsExt};
        if !at.is_multiple_of(ALIGN as u64) {
            return Ok(None);
        }
        let f = match OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECT)
            .open(path)
        {
            Ok(f) => f,
            Err(e) if e.raw_os_error() == Some(libc::EINVAL) => return Ok(None),
            Err(e) => return Err(e),
        };
        let len = buf.len().next_multiple_of(ALIGN);
        let mut raw = vec![0u8; len + ALIGN];
        let off = raw.as_ptr().align_offset(ALIGN);
        let mem = &mut raw[off..off + len];
        let mut got = 0;
        while got < len {
            match f.read_at(&mut mem[got..], at + got as u64) {
                Ok(0) => break,
                Ok(n) => {
                    got += n;
                    // Short only at the end of the file.
                    if !got.is_multiple_of(ALIGN) {
                        break;
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) if e.raw_os_error() == Some(libc::EINVAL) => return Ok(None),
                Err(e) => return Err(e),
            }
        }
        let n = got.min(buf.len());
        buf[..n].copy_from_slice(&mem[..n]);
        Ok(Some(n))
    }

    #[cfg(not(target_os = "linux"))]
    fn direct(_: &Path, _: u64, _: &mut [u8]) -> io::Result<Option<usize>> {
        Ok(None)
    }
}

impl Fs for StdFs {
    fn create(&mut self, path: &Path, new_only: bool) -> io::Result<()> {
        self.writable(path)?;
        self.open.remove(path);
        let mut o = OpenOptions::new();
        o.read(true).write(true);
        if new_only {
            o.create_new(true);
        } else {
            o.create(true).truncate(true);
        }
        let f = o.open(path)?;
        self.open.insert(path.to_path_buf(), f);
        Ok(())
    }

    fn write_at(&mut self, path: &Path, at: u64, bytes: &[u8]) -> io::Result<()> {
        self.writable(path)?;
        let f = self.file(path)?;
        f.seek(SeekFrom::Start(at))?;
        f.write_all(bytes)
    }

    fn read_at(&mut self, path: &Path, at: u64, buf: &mut [u8]) -> io::Result<usize> {
        let f = self.file(path)?;
        f.seek(SeekFrom::Start(at))?;
        let mut got = 0;
        while got < buf.len() {
            match f.read(&mut buf[got..]) {
                Ok(0) => break,
                Ok(n) => got += n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        Ok(got)
    }

    fn read_fresh(&mut self, path: &Path, at: u64, buf: &mut [u8]) -> io::Result<usize> {
        match Self::direct(path, at, buf)? {
            Some(n) => Ok(n),
            None => {
                self.fresh_fallbacks += 1;
                self.read_at(path, at, buf)
            }
        }
    }

    fn len(&mut self, path: &Path) -> io::Result<u64> {
        Ok(fs::metadata(path)?.len())
    }

    fn set_len(&mut self, path: &Path, len: u64) -> io::Result<()> {
        self.writable(path)?;
        self.file(path)?.set_len(len)
    }

    fn sync(&mut self, path: &Path) -> io::Result<()> {
        self.writable(path)?;
        self.file(path)?.sync_data()
    }

    fn sync_dir(&mut self, dir: &Path) -> io::Result<()> {
        self.writable(dir)?;
        File::open(dir)?.sync_all()
    }

    fn rename(&mut self, from: &Path, to: &Path) -> io::Result<()> {
        self.writable(from)?;
        self.open.remove(from);
        self.open.remove(to);
        fs::rename(from, to)
    }

    fn remove(&mut self, path: &Path) -> io::Result<()> {
        self.writable(path)?;
        self.open.remove(path);
        fs::remove_file(path)
    }

    fn list(&mut self, dir: &Path) -> io::Result<Vec<String>> {
        let mut v = Vec::new();
        for e in fs::read_dir(dir)? {
            let e = e?;
            if e.file_type()?.is_file() {
                if let Some(n) = e.file_name().to_str() {
                    v.push(n.to_string());
                }
            }
        }
        v.sort();
        Ok(v)
    }

    fn exists(&mut self, path: &Path) -> bool {
        path.is_file()
    }

    fn let_go(&mut self, path: &Path) {
        self.open.remove(path);
    }
}
