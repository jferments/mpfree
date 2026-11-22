//! The one small file layer every recording operation goes through, so a
//! test can stand a model stick in for the real one (`testkit::ModelStick`):
//! create, write, read, cut to length, sync, rename, remove and sync a
//! folder. Paths are whole; a real file is opened once and kept open until
//! it is renamed or removed.

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
}

/// The real file system.
#[derive(Default)]
pub struct StdFs {
    open: HashMap<PathBuf, File>,
}

impl StdFs {
    pub fn new() -> Self {
        Self::default()
    }

    fn file(&mut self, path: &Path) -> io::Result<&mut File> {
        if !self.open.contains_key(path) {
            let f = OpenOptions::new().read(true).write(true).open(path)?;
            self.open.insert(path.to_path_buf(), f);
        }
        Ok(self.open.get_mut(path).expect("just opened"))
    }
}

impl Fs for StdFs {
    fn create(&mut self, path: &Path, new_only: bool) -> io::Result<()> {
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

    fn len(&mut self, path: &Path) -> io::Result<u64> {
        Ok(fs::metadata(path)?.len())
    }

    fn set_len(&mut self, path: &Path, len: u64) -> io::Result<()> {
        self.file(path)?.set_len(len)
    }

    fn sync(&mut self, path: &Path) -> io::Result<()> {
        self.file(path)?.sync_data()
    }

    fn sync_dir(&mut self, dir: &Path) -> io::Result<()> {
        File::open(dir)?.sync_all()
    }

    fn rename(&mut self, from: &Path, to: &Path) -> io::Result<()> {
        self.open.remove(from);
        self.open.remove(to);
        fs::rename(from, to)
    }

    fn remove(&mut self, path: &Path) -> io::Result<()> {
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
}
