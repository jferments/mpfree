//! Akai's update file: the "AZ01" container, its root filesystem record and
//! the check that it is one MPFree supports.
//!
//! The layout (little-endian; every string is a u32 length, the bytes, a
//! NUL, then zeros to a multiple of 4), as `offline-update-image.py` reads
//! it:
//!
//! ```text
//! "AZ01", u32 version, u32 header size,
//! string build, u32 n + n strings (compatible boards), u32 n + n u32 (USB ids), string description
//! from the header size on, per record:
//!   "PART", u32 record header size, u64 data size, string name, string compression,
//!   u32 n + n x (string algorithm, u32 size, value padded to 4), then the data, padded to 4
//! ```
//!
//! Only the file the musician already has is read; nothing of Akai's is
//! downloaded, kept or sent anywhere.

use sha1::{Digest, Sha1};
use sha2::Sha256;
use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom, Write};
use std::path::Path;

const MAGIC: &[u8; 4] = b"AZ01";
const MAX_COUNT: u32 = 256;
const MAX_STRING: u32 = 4096;
const MAX_HEADER: u32 = 1 << 16;
/// The largest root filesystem unpacked (the MPC One's rootfs partition is 500 MiB).
pub const MAX_ROOTFS: u64 = 1 << 30;

/// An update MPFree supports: Akai's build, the kernel release in its root
/// filesystem and the root filesystem's unpacked size. Deny by default.
pub struct Supported {
    pub firmware: &'static str,
    pub build: &'static str,
    pub release: &'static str,
    pub rootfs_bytes: u64,
}

pub const SUPPORTED: &[Supported] = &[Supported {
    firmware: "MPC 3.10.1 (MPC One, Gen 1)",
    build: "SNAPSHOT-20261107130239",
    release: "6.18.44-az01-2026-08-10-rt6",
    rootfs_bytes: 453_051_392,
}];

/// The board name of the MPC One (Gen 1) in the update's list and its
/// device tree.
pub const MPC_ONE: &str = "acva";

fn bad(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

/// One PART record.
#[derive(Clone, Debug)]
pub struct Part {
    pub name: String,
    pub compression: String,
    /// (algorithm, value)
    pub hashes: Vec<(String, Vec<u8>)>,
    /// Where its data starts in the file, and how long it is.
    pub offset: u64,
    pub length: u64,
}

/// An AZ01 update file.
#[derive(Debug)]
pub struct Update {
    pub version: u32,
    pub build: String,
    pub compatible: Vec<String>,
    pub description: String,
    pub parts: Vec<Part>,
    pub size: u64,
}

struct Fields<'a> {
    b: &'a [u8],
    at: usize,
}

impl Fields<'_> {
    fn u32(&mut self) -> io::Result<u32> {
        let s = self
            .b
            .get(self.at..self.at + 4)
            .ok_or_else(|| bad("an AZ01 field runs past its block"))?;
        self.at += 4;
        Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }

    fn count(&mut self, limit: u32) -> io::Result<u32> {
        let n = self.u32()?;
        if n > limit {
            return Err(bad(format!("an AZ01 count of {n} is implausible")));
        }
        Ok(n)
    }

    fn string(&mut self) -> io::Result<String> {
        let len = self.count(MAX_STRING)? as usize;
        let stored = (len + 1 + 3) & !3;
        let s = self
            .b
            .get(self.at..self.at + stored)
            .ok_or_else(|| bad("an AZ01 string runs past its block"))?;
        if s[len] != 0 {
            return Err(bad("an AZ01 string is not NUL-terminated"));
        }
        self.at += stored;
        Ok(String::from_utf8_lossy(&s[..len]).into_owned())
    }

    fn bytes(&mut self, n: usize) -> io::Result<Vec<u8>> {
        let s = self
            .b
            .get(self.at..self.at + n)
            .ok_or_else(|| bad("an AZ01 value runs past its block"))?;
        self.at += (n + 3) & !3;
        Ok(s.to_vec())
    }
}

fn read_at(f: &mut File, offset: u64, len: usize) -> io::Result<Vec<u8>> {
    let mut b = vec![0u8; len];
    f.seek(SeekFrom::Start(offset))?;
    f.read_exact(&mut b).map_err(|_| {
        bad(format!(
            "the file ends inside the AZ01 header at offset {offset}"
        ))
    })?;
    Ok(b)
}

impl Update {
    pub fn open(path: &Path) -> io::Result<Update> {
        let mut f = File::open(path)?;
        let size = f.metadata()?.len();
        if size < 12 {
            return Err(bad("this is not an Akai update file (too short)"));
        }
        let head = read_at(&mut f, 0, 12)?;
        if &head[..4] != MAGIC {
            return Err(bad("this is not an Akai update file of the AZ01 kind"));
        }
        let version = u32::from_le_bytes([head[4], head[5], head[6], head[7]]);
        let header_size = u32::from_le_bytes([head[8], head[9], head[10], head[11]]);
        if !(12..=MAX_HEADER).contains(&header_size) || header_size as u64 > size {
            return Err(bad(format!(
                "AZ01 header size {header_size} is implausible"
            )));
        }
        let header = read_at(&mut f, 0, header_size as usize)?;
        let mut h = Fields { b: &header, at: 12 };
        let build = h.string()?;
        let mut compatible = Vec::new();
        for _ in 0..h.count(MAX_COUNT)? {
            compatible.push(h.string()?);
        }
        let ids = h.count(MAX_COUNT)?;
        h.at += 4 * ids as usize;
        let description = h.string()?;
        if h.at > header.len() {
            return Err(bad("AZ01 header fields run past the header size"));
        }
        let mut parts = Vec::new();
        let mut pos = header_size as u64;
        while size - pos >= 8 {
            let rec = read_at(&mut f, pos, 8)?;
            if &rec[..4] != b"PART" {
                break;
            }
            let header_len = u32::from_le_bytes([rec[4], rec[5], rec[6], rec[7]]);
            if !(16..=MAX_HEADER).contains(&header_len) || pos + header_len as u64 > size {
                return Err(bad(format!(
                    "the PART record at offset {pos} has an implausible header size"
                )));
            }
            let r = read_at(&mut f, pos, header_len as usize)?;
            let length = u64::from_le_bytes(r[8..16].try_into().unwrap());
            let mut p = Fields { b: &r, at: 16 };
            let name = p.string()?;
            let compression = p.string()?;
            let mut hashes = Vec::new();
            for _ in 0..p.count(MAX_COUNT)? {
                let algo = p.string()?;
                let n = p.count(MAX_STRING)? as usize;
                hashes.push((algo, p.bytes(n)?));
            }
            if p.at != r.len() {
                return Err(bad(format!(
                    "the PART record at offset {pos}: its fields don't fill its header"
                )));
            }
            let offset = pos + header_len as u64;
            if offset + length > size {
                return Err(bad(format!(
                    "the file ends inside the {name} record (is the download complete?)"
                )));
            }
            parts.push(Part {
                name,
                compression,
                hashes,
                offset,
                length,
            });
            pos = offset + ((length + 3) & !3);
            if pos > size {
                break;
            }
        }
        Ok(Update {
            version,
            build,
            compatible,
            description,
            parts,
            size,
        })
    }

    /// The single root filesystem record.
    pub fn rootfs(&self) -> io::Result<&Part> {
        let mut found = self
            .parts
            .iter()
            .filter(|p| p.name.to_ascii_lowercase().contains("rootfs"));
        let part = found
            .next()
            .ok_or_else(|| bad("the update has no root filesystem record"))?;
        if found.next().is_some() {
            return Err(bad("the update has more than one root filesystem record"));
        }
        Ok(part)
    }

    /// Whether it names the MPC One (Gen 1) among its boards.
    pub fn for_mpc_one(&self) -> bool {
        self.compatible
            .iter()
            .any(|c| c.to_ascii_lowercase().contains(MPC_ONE))
    }

    /// The supported update with this build, if any.
    pub fn supported(&self) -> Option<&'static Supported> {
        SUPPORTED.iter().find(|s| s.build == self.build)
    }
}

/// What the unpacking found.
#[derive(Debug)]
pub struct Unpacked {
    pub bytes: u64,
    /// The record's hashes that were checked, and over what ("stored" or "unpacked").
    pub checked: Vec<String>,
    /// SHA-256 of the unpacked root filesystem.
    pub sha256: [u8; 32],
}

struct Hashing<W: Write> {
    w: W,
    n: u64,
    sha1: Sha1,
    sha256: Sha256,
    max: u64,
    progress: Box<dyn FnMut(u64)>,
}

impl<W: Write> Write for Hashing<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.n + buf.len() as u64 > self.max {
            return Err(bad(
                "the unpacked root filesystem is larger than any MPC One root filesystem",
            ));
        }
        let n = self.w.write(buf)?;
        self.sha1.update(&buf[..n]);
        self.sha256.update(&buf[..n]);
        self.n += n as u64;
        (self.progress)(self.n);
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.w.flush()
    }
}

/// Unpacks the root filesystem record into `out`, checking the record's
/// SHA-1 and SHA-256 hashes over the stored bytes or the unpacked data (a
/// hash that matches neither is an error). `progress(unpacked bytes)`.
pub fn unpack_rootfs(
    path: &Path,
    part: &Part,
    out: impl Write,
    progress: impl FnMut(u64) + 'static,
) -> io::Result<Unpacked> {
    let mut f = File::open(path)?;
    // Hashes over the stored bytes first.
    let (mut s1, mut s256) = (Sha1::new(), Sha256::new());
    f.seek(SeekFrom::Start(part.offset))?;
    let mut left = part.length;
    let mut buf = vec![0u8; 1 << 20];
    while left > 0 {
        let n = (left as usize).min(buf.len());
        f.read_exact(&mut buf[..n])?;
        s1.update(&buf[..n]);
        s256.update(&buf[..n]);
        left -= n as u64;
    }
    let (stored1, stored256) = (s1.finalize().to_vec(), s256.finalize().to_vec());
    f.seek(SeekFrom::Start(part.offset))?;
    let mut input = BufReader::with_capacity(1 << 20, f.take(part.length));
    let mut w = Hashing {
        w: out,
        n: 0,
        sha1: Sha1::new(),
        sha256: Sha256::new(),
        max: MAX_ROOTFS,
        progress: Box::new(progress),
    };
    match part.compression.as_str() {
        "xz" => lzma_rs::xz_decompress(&mut input, &mut w).map_err(|e| {
            bad(format!(
                "the root filesystem record does not unpack (xz): {e}"
            ))
        })?,
        "" | "none" => {
            io::copy(&mut input, &mut w)?;
        }
        other => {
            return Err(bad(format!(
            "the root filesystem record is compressed with {other}, which this app does not know"
        )))
        }
    }
    w.flush()?;
    let (got1, got256) = (w.sha1.finalize().to_vec(), w.sha256.finalize());
    let mut checked = Vec::new();
    for (algo, value) in &part.hashes {
        let (stored, unpacked) = match algo.as_str() {
            "sha1" => (&stored1, &got1),
            "sha256" => (&stored256, &got256.to_vec()),
            _ => continue,
        };
        if value == stored {
            checked.push(format!("{algo} (stored bytes)"));
        } else if value == unpacked {
            checked.push(format!("{algo} (unpacked)"));
        } else {
            return Err(bad(format!(
                "the root filesystem's {algo} does not match the update file's: the file is damaged or incomplete"
            )));
        }
    }
    if checked.is_empty() {
        return Err(bad(
            "the update file has no SHA-1 or SHA-256 for its root filesystem to check",
        ));
    }
    Ok(Unpacked {
        bytes: w.n,
        checked,
        sha256: got256.into(),
    })
}

/// The update to use in a folder (such as Downloads): the supported MPC One
/// update in it (the newest, if there are copies). With `any_update`, the
/// newest MPC One update when none is supported. Only the folder itself is
/// looked in, and only each file's header is read.
pub fn find_in_dir(dir: &Path, any_update: bool) -> io::Result<std::path::PathBuf> {
    let mut found: Vec<(bool, std::time::SystemTime, std::path::PathBuf, String)> = Vec::new();
    for e in std::fs::read_dir(dir)? {
        let e = e?;
        let p = e.path();
        let Ok(md) = e.metadata() else { continue };
        if !md.is_file() || md.len() < 1024 {
            continue;
        }
        let mut head = [0u8; 4];
        if File::open(&p)
            .and_then(|mut f| f.read_exact(&mut head))
            .is_err()
            || &head != MAGIC
        {
            continue;
        }
        if let Ok(u) = Update::open(&p) {
            if u.for_mpc_one() {
                found.push((
                    u.supported().is_some(),
                    md.modified().unwrap_or(std::time::UNIX_EPOCH),
                    p,
                    u.build,
                ));
            }
        }
    }
    found.sort();
    match found.last() {
        Some((true, _, p, _)) => Ok(p.clone()),
        Some((false, _, p, _)) if any_update => Ok(p.clone()),
        Some(_) => Err(bad(format!(
            "{} holds MPC One updates ({}), but none MPFree supports; MPFree needs {}",
            dir.display(),
            found
                .iter()
                .map(|f| f.3.as_str())
                .collect::<Vec<_>>()
                .join(", "),
            SUPPORTED
                .iter()
                .map(|s| s.firmware)
                .collect::<Vec<_>>()
                .join(" or ")
        ))),
        None => Err(bad(format!(
            "{} holds no Akai update file for the MPC One",
            dir.display()
        ))),
    }
}

/// A whole file's SHA-256.
pub fn file_sha256(path: &Path) -> io::Result<[u8; 32]> {
    let mut f = File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().into())
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// A small AZ01 file for tests: one rootfs record (xz) holding `rootfs`.
pub mod testfile {
    use super::*;

    fn string(out: &mut Vec<u8>, s: &str) {
        out.extend_from_slice(&(s.len() as u32).to_le_bytes());
        out.extend_from_slice(s.as_bytes());
        out.push(0);
        while !out.len().is_multiple_of(4) {
            out.push(0);
        }
    }

    pub fn make(build: &str, boards: &[&str], rootfs: &[u8], hash_over_unpacked: bool) -> Vec<u8> {
        let mut stored = Vec::new();
        lzma_rs::xz_compress(&mut &rootfs[..], &mut stored).unwrap();
        let mut h = Vec::new();
        string(&mut h, build);
        h.extend_from_slice(&(boards.len() as u32).to_le_bytes());
        for b in boards {
            string(&mut h, b);
        }
        h.extend_from_slice(&1u32.to_le_bytes());
        h.extend_from_slice(&0x1234u32.to_le_bytes());
        string(&mut h, "test update");
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&1u32.to_le_bytes());
        out.extend_from_slice(&((12 + h.len()) as u32).to_le_bytes());
        out.extend_from_slice(&h);
        let mut r = Vec::new();
        string(&mut r, "rootfs");
        string(&mut r, "xz");
        r.extend_from_slice(&1u32.to_le_bytes());
        string(&mut r, "sha1");
        let sum = if hash_over_unpacked {
            Sha1::digest(rootfs)
        } else {
            Sha1::digest(&stored)
        };
        r.extend_from_slice(&20u32.to_le_bytes());
        r.extend_from_slice(&sum);
        while r.len() % 4 != 0 {
            r.push(0);
        }
        out.extend_from_slice(b"PART");
        out.extend_from_slice(&((16 + r.len()) as u32).to_le_bytes());
        out.extend_from_slice(&(stored.len() as u64).to_le_bytes());
        out.extend_from_slice(&r);
        out.extend_from_slice(&stored);
        while out.len() % 4 != 0 {
            out.push(0);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpfile(tag: &str, data: &[u8]) -> std::path::PathBuf {
        let p =
            std::env::temp_dir().join(format!("mpfree-stick-update-{tag}-{}", std::process::id()));
        std::fs::write(&p, data).unwrap();
        p
    }

    #[test]
    fn finds_the_supported_update_in_a_folder() {
        let dir = std::env::temp_dir().join(format!("mpfree-stick-find-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(find_in_dir(&dir, true).is_err(), "an empty folder");
        std::fs::write(dir.join("notes.txt"), vec![b'x'; 4096]).unwrap();
        let rootfs = vec![0u8; 4096];
        std::fs::write(
            dir.join("old.img"),
            testfile::make("SNAPSHOT-OLD", &["ACVA"], &rootfs, false),
        )
        .unwrap();
        std::fs::write(
            dir.join("other-board.img"),
            testfile::make(SUPPORTED[0].build, &["ACVB"], &rootfs, false),
        )
        .unwrap();
        let e = find_in_dir(&dir, false).unwrap_err().to_string();
        assert!(
            e.contains("SNAPSHOT-OLD") && e.contains("none MPFree supports"),
            "{e}"
        );
        assert_eq!(
            find_in_dir(&dir, true).unwrap(),
            dir.join("old.img"),
            "a developer may take it"
        );
        std::fs::write(
            dir.join("MPC-3.10.1.img"),
            testfile::make(SUPPORTED[0].build, &["ACVA", "ACVB"], &rootfs, false),
        )
        .unwrap();
        assert_eq!(
            find_in_dir(&dir, false).unwrap(),
            dir.join("MPC-3.10.1.img")
        );
        assert_eq!(
            find_in_dir(&dir, true).unwrap(),
            dir.join("MPC-3.10.1.img"),
            "the supported one first"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn reads_and_unpacks_an_update() {
        let rootfs: Vec<u8> = (0..300_000u32).map(|i| (i * 7 % 251) as u8).collect();
        for over_unpacked in [false, true] {
            let p = tmpfile(
                &format!("ok{over_unpacked}"),
                &testfile::make("SNAPSHOT-X", &["ACVA", "ACVB"], &rootfs, over_unpacked),
            );
            let u = Update::open(&p).unwrap();
            assert_eq!(u.build, "SNAPSHOT-X");
            assert_eq!(u.compatible, ["ACVA", "ACVB"]);
            assert!(u.for_mpc_one());
            assert!(u.supported().is_none(), "deny by default");
            let part = u.rootfs().unwrap().clone();
            let mut out = Vec::new();
            let got = unpack_rootfs(&p, &part, &mut out, |_| {}).unwrap();
            assert_eq!(out, rootfs);
            assert_eq!(got.bytes, rootfs.len() as u64);
            assert_eq!(
                got.checked,
                [if over_unpacked {
                    "sha1 (unpacked)"
                } else {
                    "sha1 (stored bytes)"
                }]
            );
            std::fs::remove_file(&p).unwrap();
        }
    }

    #[test]
    fn damage_and_other_files_are_refused() {
        let rootfs = vec![7u8; 100_000];
        let mut data = testfile::make("B", &["ACVB"], &rootfs, false);
        let p = tmpfile("other", &data);
        let u = Update::open(&p).unwrap();
        assert!(!u.for_mpc_one());
        // A changed byte in the payload: the hash catches it.
        let at = data.len() - 20;
        data[at] ^= 0x55;
        std::fs::write(&p, &data).unwrap();
        let u = Update::open(&p).unwrap();
        let part = u.rootfs().unwrap().clone();
        assert!(unpack_rootfs(&p, &part, io::sink(), |_| {}).is_err());
        // Cut short.
        std::fs::write(&p, &data[..data.len() - 1000]).unwrap();
        assert!(Update::open(&p)
            .unwrap_err()
            .to_string()
            .contains("download complete"));
        std::fs::write(&p, b"PK\x03\x04 a zip file").unwrap();
        assert!(Update::open(&p).is_err());
        std::fs::remove_file(&p).unwrap();
    }
}
