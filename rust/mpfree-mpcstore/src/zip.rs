//! Reads a zip archive held in memory: the central directory, then each
//! entry, stored or deflated, checked against its CRC. Enough for the
//! catalog's release zips; no encryption, no ZIP64, no split archives.

use mpfree_mpcskin::inflate::{crc32, inflate_raw};

const EOCD: u32 = 0x0605_4b50;
const CENTRAL: u32 = 0x0201_4b50;
const LOCAL: u32 = 0x0403_4b50;
/// At most this many entries (the catalog's largest zip has a few hundred).
pub const MAX_ENTRIES: usize = 20_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    File,
    Dir,
    /// A symbolic link. Never unpacked: it could point anywhere.
    Link,
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub kind: Kind,
    /// Unpacked size, from the central directory.
    pub size: u64,
    /// Unix permission bits when the zip was made on Unix (0 otherwise).
    pub mode: u32,
    method: u16,
    encrypted: bool,
    crc: u32,
    packed: u64,
    offset: u64,
}

impl Entry {
    /// Executable for its owner, as a plugin's `.so` usually is.
    pub fn executable(&self) -> bool {
        self.mode & 0o100 != 0
    }
}

pub struct Zip<'a> {
    data: &'a [u8],
    pub entries: Vec<Entry>,
}

fn u16_at(d: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_le_bytes(d.get(i..i + 2)?.try_into().ok()?))
}

fn u32_at(d: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes(d.get(i..i + 4)?.try_into().ok()?))
}

impl<'a> Zip<'a> {
    pub fn parse(data: &'a [u8]) -> Result<Zip<'a>, String> {
        if data.len() < 22 {
            return Err("too short to be a zip".into());
        }
        // The end record is the last 22 bytes plus a comment of up to 64 KB.
        let lowest = data.len().saturating_sub(22 + 0xffff);
        let mut at = None;
        let mut i = data.len() - 22;
        loop {
            if u32_at(data, i) == Some(EOCD) {
                let comment = u16_at(data, i + 20).unwrap_or(0) as usize;
                if i + 22 + comment == data.len() {
                    at = Some(i);
                    break;
                }
            }
            if i == lowest {
                break;
            }
            i -= 1;
        }
        let e = at.ok_or("not a zip (no end record)")?;
        let disk = u16_at(data, e + 4).unwrap_or(1);
        let cd_disk = u16_at(data, e + 6).unwrap_or(1);
        if disk != 0 || cd_disk != 0 {
            return Err("a split zip".into());
        }
        let count = u16_at(data, e + 10).unwrap_or(0) as usize;
        let cd_size = u32_at(data, e + 12).unwrap_or(0) as usize;
        let cd_off = u32_at(data, e + 16).unwrap_or(0) as usize;
        if count == 0xffff || cd_off == 0xffff_ffff || cd_size == 0xffff_ffff {
            return Err("a ZIP64 archive (over 4 GB or 65,535 entries)".into());
        }
        if count > MAX_ENTRIES {
            return Err(format!("{count} entries, more than {MAX_ENTRIES}"));
        }
        if cd_off.checked_add(cd_size).is_none_or(|end| end > e) {
            return Err("its directory is outside the file".into());
        }
        let mut entries = Vec::with_capacity(count);
        let mut p = cd_off;
        for n in 0..count {
            let bad = || format!("entry {} of the directory is cut short", n + 1);
            if u32_at(data, p) != Some(CENTRAL) {
                return Err(format!("entry {} of the directory is damaged", n + 1));
            }
            let made_by = u16_at(data, p + 4).ok_or_else(bad)?;
            let flags = u16_at(data, p + 8).ok_or_else(bad)?;
            let method = u16_at(data, p + 10).ok_or_else(bad)?;
            let crc = u32_at(data, p + 16).ok_or_else(bad)?;
            let packed = u32_at(data, p + 20).ok_or_else(bad)?;
            let size = u32_at(data, p + 24).ok_or_else(bad)?;
            let name_len = u16_at(data, p + 28).ok_or_else(bad)? as usize;
            let extra_len = u16_at(data, p + 30).ok_or_else(bad)? as usize;
            let comment_len = u16_at(data, p + 32).ok_or_else(bad)? as usize;
            let external = u32_at(data, p + 38).ok_or_else(bad)?;
            let offset = u32_at(data, p + 42).ok_or_else(bad)?;
            let raw = data.get(p + 46..p + 46 + name_len).ok_or_else(bad)?;
            if packed == 0xffff_ffff || size == 0xffff_ffff || offset == 0xffff_ffff {
                return Err("a ZIP64 archive (over 4 GB)".into());
            }
            // Bit 11 says UTF-8; older tools wrote CP437, which is the same
            // for the ASCII names nearly every zip has.
            let name = String::from_utf8_lossy(raw).into_owned();
            let unix = made_by >> 8 == 3;
            let mode = if unix { external >> 16 } else { 0 };
            let kind = if unix && mode & 0o170000 == 0o120000 {
                Kind::Link
            } else if name.ends_with('/') || (unix && mode & 0o170000 == 0o040000) {
                Kind::Dir
            } else {
                Kind::File
            };
            entries.push(Entry {
                name,
                kind,
                size: size as u64,
                mode: mode & 0o7777,
                method,
                encrypted: flags & 1 != 0,
                crc,
                packed: packed as u64,
                offset: offset as u64,
            });
            p += 46 + name_len + extra_len + comment_len;
        }
        Ok(Zip { data, entries })
    }

    /// The entry's contents, checked against its CRC. Fails when it is
    /// encrypted, packed some other way, damaged, or bigger than `limit`.
    pub fn read(&self, e: &Entry, limit: u64) -> Result<Vec<u8>, String> {
        let name = &e.name;
        if e.encrypted {
            return Err(format!("{name} is encrypted"));
        }
        if e.size > limit {
            return Err(format!("{name} is {} bytes, more than {limit}", e.size));
        }
        let o = e.offset as usize;
        if u32_at(self.data, o) != Some(LOCAL) {
            return Err(format!("{name}: its header is damaged"));
        }
        let n = u16_at(self.data, o + 26).unwrap_or(0) as usize;
        let x = u16_at(self.data, o + 28).unwrap_or(0) as usize;
        let start = o + 30 + n + x;
        let packed = self
            .data
            .get(start..start.saturating_add(e.packed as usize))
            .ok_or_else(|| format!("{name} is cut short"))?;
        let out = match e.method {
            0 => {
                if e.packed != e.size {
                    return Err(format!("{name}: stored sizes disagree"));
                }
                packed.to_vec()
            }
            8 => {
                let (out, _) = inflate_raw(packed, e.size as usize)
                    .map_err(|why| format!("{name} doesn't unpack: {why}"))?;
                if out.len() as u64 != e.size {
                    return Err(format!("{name} unpacks to the wrong size"));
                }
                out
            }
            m => return Err(format!("{name} is packed with method {m}, not deflate")),
        };
        if crc32(&out) != e.crc {
            return Err(format!("{name} is damaged (its CRC doesn't match)"));
        }
        Ok(out)
    }
}

/// The parts of an entry's name, if it is safe to unpack under a folder:
/// relative, no `.` or `..`, no backslash, no control characters, and no
/// character a FAT or exFAT stick refuses.
pub fn safe_parts(name: &str) -> Option<Vec<&str>> {
    let trimmed = name.strip_suffix('/').unwrap_or(name);
    if trimmed.is_empty() || trimmed.starts_with('/') {
        return None;
    }
    let mut parts = Vec::new();
    for p in trimmed.split('/') {
        if p.is_empty()
            || p == "."
            || p == ".."
            || p.len() > 255
            || p.chars().any(|c| {
                c.is_control() || matches!(c, '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
            })
            || p.ends_with(' ')
            || p.ends_with('.')
        {
            return None;
        }
        parts.push(p);
    }
    Some(parts)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Builds a zip of stored entries (method 0), for tests: (name, data,
    /// unix mode). A mode of 0o120777 makes a link.
    pub(crate) fn make(files: &[(&str, &[u8], u32)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut central = Vec::new();
        for (name, data, mode) in files {
            let off = out.len() as u32;
            let crc = crc32(data);
            let head = |sig: u32, v: &mut Vec<u8>| {
                v.extend_from_slice(&sig.to_le_bytes());
            };
            head(LOCAL, &mut out);
            out.extend_from_slice(&20u16.to_le_bytes());
            out.extend_from_slice(&0x0800u16.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&[0; 4]);
            out.extend_from_slice(&crc.to_le_bytes());
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(data);
            head(CENTRAL, &mut central);
            central.extend_from_slice(&(0x0300u16 | 20).to_le_bytes());
            central.extend_from_slice(&20u16.to_le_bytes());
            central.extend_from_slice(&0x0800u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&[0; 4]);
            central.extend_from_slice(&crc.to_le_bytes());
            central.extend_from_slice(&(data.len() as u32).to_le_bytes());
            central.extend_from_slice(&(data.len() as u32).to_le_bytes());
            central.extend_from_slice(&(name.len() as u16).to_le_bytes());
            central.extend_from_slice(&[0; 8]);
            central.extend_from_slice(&(mode << 16).to_le_bytes());
            central.extend_from_slice(&off.to_le_bytes());
            central.extend_from_slice(name.as_bytes());
        }
        let cd_off = out.len() as u32;
        out.extend_from_slice(&central);
        out.extend_from_slice(&EOCD.to_le_bytes());
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&(files.len() as u16).to_le_bytes());
        out.extend_from_slice(&(files.len() as u16).to_le_bytes());
        out.extend_from_slice(&(central.len() as u32).to_le_bytes());
        out.extend_from_slice(&cd_off.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out
    }

    #[test]
    fn reads_stored_entries_and_their_kinds() {
        let z = make(&[
            ("a/", b"", 0o040755),
            ("a/x.so", b"elf", 0o100755),
            ("a/link", b"/etc/passwd", 0o120777),
        ]);
        let zip = Zip::parse(&z).unwrap();
        let kinds: Vec<Kind> = zip.entries.iter().map(|e| e.kind).collect();
        assert_eq!(kinds, [Kind::Dir, Kind::File, Kind::Link]);
        assert!(zip.entries[1].executable());
        assert_eq!(zip.read(&zip.entries[1], 100).unwrap(), b"elf");
        assert!(zip.read(&zip.entries[1], 2).is_err(), "over the limit");
    }

    #[test]
    fn a_damaged_entry_is_refused() {
        let mut z = make(&[("f", b"hello", 0o100644)]);
        let at = z.windows(5).position(|w| w == b"hello").unwrap();
        z[at] = b'j';
        let zip = Zip::parse(&z).unwrap();
        assert!(zip.read(&zip.entries[0], 100).unwrap_err().contains("CRC"));
        assert!(Zip::parse(b"PK not really a zip file at all").is_err());
        assert!(Zip::parse(&z[..z.len() - 3]).is_err());
    }

    #[test]
    fn unsafe_names_are_refused() {
        assert_eq!(
            safe_parts("a/b c/[SYN] d.so"),
            Some(vec!["a", "b c", "[SYN] d.so"])
        );
        assert_eq!(safe_parts("dir/"), Some(vec!["dir"]));
        for bad in [
            "/etc/x", "a/../b", "..", "a//b", "a\\b", "c:x", "a/./b", "a\u{0}b", "",
        ] {
            assert_eq!(safe_parts(bad), None, "{bad:?}");
        }
    }
}
