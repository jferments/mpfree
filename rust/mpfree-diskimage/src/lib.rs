//! Disk images, with no outside tools: what MPFree's USB creator and the
//! pad player's first-start wizard need to read and make the filesystems of
//! a stick or an eMMC backup on Linux, macOS and Windows alike.
//!
//! - [`gpt`]: read a GUID partition table, and write one (protective MBR,
//!   both headers, CRC32s).
//! - [`ext4`]: a read-only ext2/3/4 reader: extents and block maps,
//!   64-bit and meta_bg group descriptors, inline data, hash-indexed
//!   folders (read as plain ones), fast and slow links, extended
//!   attributes. It never writes and never replays a journal.
//! - [`ext2w`]: a writer for a fresh filesystem from a tree of files, in the
//!   plain ext2 rev 1 layout (no journal, no extents, no checksums), which
//!   Linux's ext4 driver mounts. MPFree's stick uses it for its read-only
//!   root filesystem, its kits partition and its small log partition.
//!
//! Every reader takes a [`ReadAt`], so the same code reads an image file,
//! a partition inside one (an [`Offset`]) or a whole device.

#![forbid(unsafe_code)]

pub mod ext2w;
pub mod ext4;
pub mod gpt;

use std::fs::File;
use std::io;

/// Positional reads: the whole buffer, or an error.
pub trait ReadAt {
    fn read_at(&self, buf: &mut [u8], offset: u64) -> io::Result<()>;
}

/// Positional writes: the whole buffer, or an error.
pub trait WriteAt {
    fn write_at(&self, buf: &[u8], offset: u64) -> io::Result<()>;
}

impl ReadAt for File {
    #[cfg(unix)]
    fn read_at(&self, buf: &mut [u8], offset: u64) -> io::Result<()> {
        std::os::unix::fs::FileExt::read_exact_at(self, buf, offset)
    }

    #[cfg(windows)]
    fn read_at(&self, mut buf: &mut [u8], mut offset: u64) -> io::Result<()> {
        use std::os::windows::fs::FileExt;
        while !buf.is_empty() {
            let n = self.seek_read(buf, offset)?;
            if n == 0 {
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "end of file"));
            }
            buf = &mut buf[n..];
            offset += n as u64;
        }
        Ok(())
    }
}

impl WriteAt for File {
    #[cfg(unix)]
    fn write_at(&self, buf: &[u8], offset: u64) -> io::Result<()> {
        std::os::unix::fs::FileExt::write_all_at(self, buf, offset)
    }

    #[cfg(windows)]
    fn write_at(&self, mut buf: &[u8], mut offset: u64) -> io::Result<()> {
        use std::os::windows::fs::FileExt;
        while !buf.is_empty() {
            let n = self.seek_write(buf, offset)?;
            if n == 0 {
                return Err(io::Error::new(io::ErrorKind::WriteZero, "wrote nothing"));
            }
            buf = &buf[n..];
            offset += n as u64;
        }
        Ok(())
    }
}

impl ReadAt for [u8] {
    fn read_at(&self, buf: &mut [u8], offset: u64) -> io::Result<()> {
        let start = usize::try_from(offset).map_err(|_| eof())?;
        let end = start.checked_add(buf.len()).ok_or_else(eof)?;
        let src = self.get(start..end).ok_or_else(eof)?;
        buf.copy_from_slice(src);
        Ok(())
    }
}

impl ReadAt for Vec<u8> {
    fn read_at(&self, buf: &mut [u8], offset: u64) -> io::Result<()> {
        self.as_slice().read_at(buf, offset)
    }
}

impl<T: ReadAt + ?Sized> ReadAt for &T {
    fn read_at(&self, buf: &mut [u8], offset: u64) -> io::Result<()> {
        (**self).read_at(buf, offset)
    }
}

impl<T: WriteAt + ?Sized> WriteAt for &T {
    fn write_at(&self, buf: &[u8], offset: u64) -> io::Result<()> {
        (**self).write_at(buf, offset)
    }
}

/// A window into another reader: a partition inside a disk image.
pub struct Offset<R> {
    pub inner: R,
    pub start: u64,
    /// The window's length; reads past it fail.
    pub len: u64,
}

impl<R: ReadAt> ReadAt for Offset<R> {
    fn read_at(&self, buf: &mut [u8], offset: u64) -> io::Result<()> {
        let end = offset.checked_add(buf.len() as u64).ok_or_else(eof)?;
        if end > self.len {
            return Err(eof());
        }
        self.inner.read_at(buf, self.start + offset)
    }
}

impl<R: WriteAt> WriteAt for Offset<R> {
    fn write_at(&self, buf: &[u8], offset: u64) -> io::Result<()> {
        let end = offset.checked_add(buf.len() as u64).ok_or_else(eof)?;
        if end > self.len {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "write past the end of the window",
            ));
        }
        self.inner.write_at(buf, self.start + offset)
    }
}

/// A disk in memory that grows as it is written: for tests and small images.
#[derive(Default)]
pub struct Mem {
    data: std::cell::RefCell<Vec<u8>>,
}

impl Mem {
    pub fn new() -> Mem {
        Mem::default()
    }

    /// A copy of the bytes.
    pub fn bytes(&self) -> Vec<u8> {
        self.data.borrow().clone()
    }

    pub fn into_inner(self) -> Vec<u8> {
        self.data.into_inner()
    }

    pub fn len(&self) -> u64 {
        self.data.borrow().len() as u64
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Grows (with zeros) or cuts to `len` bytes.
    pub fn set_len(&self, len: u64) {
        self.data.borrow_mut().resize(len as usize, 0);
    }
}

impl ReadAt for Mem {
    fn read_at(&self, buf: &mut [u8], offset: u64) -> io::Result<()> {
        self.data.borrow().as_slice().read_at(buf, offset)
    }
}

impl WriteAt for Mem {
    fn write_at(&self, buf: &[u8], offset: u64) -> io::Result<()> {
        let start = usize::try_from(offset).map_err(|_| eof())?;
        let end = start.checked_add(buf.len()).ok_or_else(eof)?;
        let mut d = self.data.borrow_mut();
        if d.len() < end {
            d.resize(end, 0);
        }
        d[start..end].copy_from_slice(buf);
        Ok(())
    }
}

fn eof() -> io::Error {
    io::Error::new(io::ErrorKind::UnexpectedEof, "read past the end")
}

/// An error with a plain message.
pub(crate) fn bad(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

/// CRC-32 (IEEE 802.3, reflected, as zlib and GPT use).
pub fn crc32(data: &[u8]) -> u32 {
    crc32_update(0, data)
}

/// Continue a CRC-32 over more bytes (start with 0).
pub fn crc32_update(crc: u32, data: &[u8]) -> u32 {
    let mut c = !crc;
    for &b in data {
        c = CRC_TABLE[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8);
    }
    !c
}

const CRC_TABLE: [u32; 256] = {
    let mut t = [0u32; 256];
    let mut n = 0;
    while n < 256 {
        let mut c = n as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 {
                (c >> 1) ^ 0xEDB8_8320
            } else {
                c >> 1
            };
            k += 1;
        }
        t[n] = c;
        n += 1;
    }
    t
};

pub(crate) fn le16(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

pub(crate) fn le32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

pub(crate) fn le64(b: &[u8], at: usize) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[at..at + 8]);
    u64::from_le_bytes(a)
}

pub(crate) fn put16(b: &mut [u8], at: usize, v: u16) {
    b[at..at + 2].copy_from_slice(&v.to_le_bytes());
}

pub(crate) fn put32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

pub(crate) fn put64(b: &mut [u8], at: usize, v: u64) {
    b[at..at + 8].copy_from_slice(&v.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_known_values() {
        assert_eq!(crc32(b""), 0);
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32_update(crc32(b"1234"), b"56789"), 0xCBF4_3926);
    }

    #[test]
    fn offset_reads_stay_inside() {
        let data: Vec<u8> = (0..100u8).collect();
        let w = Offset {
            inner: &data,
            start: 10,
            len: 20,
        };
        let mut b = [0u8; 5];
        w.read_at(&mut b, 15).unwrap();
        assert_eq!(b, [25, 26, 27, 28, 29]);
        assert!(w.read_at(&mut b, 16).is_err());
    }
}
