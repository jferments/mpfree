//! GUID partition tables: read one, and write one with its protective MBR,
//! primary and backup headers and their CRC32s. Sectors are 512 bytes.

use crate::{bad, crc32, le32, le64, put16, put32, put64, ReadAt, WriteAt};
use std::io;

pub const SECTOR: u64 = 512;
/// Partition entries in a table we write (and the most we read).
pub const ENTRIES: usize = 128;
pub const ENTRY_SIZE: usize = 128;
/// Sectors the entry array takes: 128 entries of 128 bytes.
pub const ENTRY_SECTORS: u64 = (ENTRIES * ENTRY_SIZE) as u64 / SECTOR;
/// "Linux filesystem data".
pub const LINUX_FS: &str = "0FC63DAF-8483-4772-8E79-3D69D8477DE4";
/// "Microsoft basic data": FAT and exFAT partitions every system mounts.
pub const BASIC_DATA: &str = "EBD0A0A2-B9E5-4433-87C0-68B6B72699C7";

pub type Guid = [u8; 16];

/// One partition: its entry number is its index in [`Table::parts`] plus one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Part {
    pub type_guid: Guid,
    pub guid: Guid,
    /// First sector.
    pub first: u64,
    /// Last sector, inclusive.
    pub last: u64,
    pub attrs: u64,
    pub name: String,
}

impl Part {
    pub fn sectors(&self) -> u64 {
        self.last + 1 - self.first
    }

    pub fn start_bytes(&self) -> u64 {
        self.first * SECTOR
    }

    pub fn len_bytes(&self) -> u64 {
        self.sectors() * SECTOR
    }
}

/// A partition table. Entry `i` of `parts` is partition `i + 1`; an unused
/// entry in the middle is `None`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Table {
    pub disk_guid: Guid,
    pub parts: Vec<Option<Part>>,
}

impl Table {
    /// Partition `n` (from 1).
    pub fn part(&self, n: usize) -> Option<&Part> {
        self.parts.get(n.checked_sub(1)?)?.as_ref()
    }

    /// The partition with this name, and its number. None if there are none
    /// or more than one.
    pub fn by_name(&self, name: &str) -> Option<(usize, &Part)> {
        let mut found = self
            .parts
            .iter()
            .enumerate()
            .filter_map(|(i, p)| p.as_ref().map(|p| (i + 1, p)))
            .filter(|(_, p)| p.name == name);
        let first = found.next()?;
        if found.next().is_some() {
            return None;
        }
        Some(first)
    }
}

/// Parses "0FC63DAF-8483-4772-8E79-3D69D8477DE4" into GPT's mixed-endian bytes.
pub fn guid(text: &str) -> io::Result<Guid> {
    let hex: String = text.chars().filter(|c| *c != '-').collect();
    if hex.len() != 32 || text.len() != 36 {
        return Err(bad(format!("not a GUID: {text}")));
    }
    let mut raw = [0u8; 16];
    for (i, b) in raw.iter_mut().enumerate() {
        *b = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16)
            .map_err(|_| bad(format!("not a GUID: {text}")))?;
    }
    let mut g = raw;
    g[0..4].reverse();
    g[4..6].reverse();
    g[6..8].reverse();
    Ok(g)
}

/// The text form of GPT's mixed-endian bytes, upper case.
pub fn guid_text(g: &Guid) -> String {
    let mut raw = *g;
    raw[0..4].reverse();
    raw[4..6].reverse();
    raw[6..8].reverse();
    let h: Vec<String> = raw.iter().map(|b| format!("{b:02X}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        h[0..4].concat(),
        h[4..6].concat(),
        h[6..8].concat(),
        h[8..10].concat(),
        h[10..16].concat()
    )
}

/// 16 random bytes as a version-4 GUID (from the system's random hash keys
/// and the time; good enough for partition and filesystem ids).
pub fn random_guid() -> Guid {
    let mut g = random16();
    g[7] = (g[7] & 0x0F) | 0x40; // version 4, in the mixed-endian third field
    g[8] = (g[8] & 0x3F) | 0x80;
    g
}

/// 16 random bytes.
pub fn random16() -> [u8; 16] {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNT: AtomicU64 = AtomicU64::new(0);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let n = COUNT.fetch_add(1, Ordering::Relaxed);
    let mut out = [0u8; 16];
    for (i, half) in out.chunks_mut(8).enumerate() {
        let mut h = RandomState::new().build_hasher();
        h.write_u128(now);
        h.write_u64(n);
        h.write_usize(i);
        half.copy_from_slice(&h.finish().to_le_bytes());
    }
    out
}

/// Reads the primary table (its header and entries checked by CRC).
pub fn read<R: ReadAt + ?Sized>(r: &R) -> io::Result<Table> {
    let mut hdr = [0u8; 512];
    r.read_at(&mut hdr, SECTOR)?;
    if &hdr[0..8] != b"EFI PART" {
        return Err(bad("no GPT header at sector 1"));
    }
    let size = le32(&hdr, 12) as usize;
    if !(92..=512).contains(&size) {
        return Err(bad(format!("GPT header size {size} is implausible")));
    }
    let mut check = hdr[..size].to_vec();
    check[16..20].fill(0);
    if crc32(&check) != le32(&hdr, 16) {
        return Err(bad("the GPT header's CRC does not match"));
    }
    let entries_lba = le64(&hdr, 72);
    let count = le32(&hdr, 80) as usize;
    let esize = le32(&hdr, 84) as usize;
    if count == 0 || count > 1024 || !(128..=1024).contains(&esize) || !esize.is_multiple_of(8) {
        return Err(bad(format!(
            "GPT entries ({count} of {esize} bytes) are implausible"
        )));
    }
    let mut ents = vec![0u8; count * esize];
    r.read_at(&mut ents, entries_lba * SECTOR)?;
    if crc32(&ents) != le32(&hdr, 88) {
        return Err(bad("the GPT entries' CRC does not match"));
    }
    let mut disk_guid = [0u8; 16];
    disk_guid.copy_from_slice(&hdr[56..72]);
    let mut parts: Vec<Option<Part>> = Vec::new();
    for i in 0..count {
        let e = &ents[i * esize..i * esize + esize];
        if e[0..16].iter().all(|b| *b == 0) {
            parts.push(None);
            continue;
        }
        let mut type_guid = [0u8; 16];
        type_guid.copy_from_slice(&e[0..16]);
        let mut guid = [0u8; 16];
        guid.copy_from_slice(&e[16..32]);
        let first = le64(e, 32);
        let last = le64(e, 40);
        if last < first {
            return Err(bad(format!("GPT entry {} ends before it starts", i + 1)));
        }
        let units: Vec<u16> = e[56..128]
            .chunks(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .take_while(|u| *u != 0)
            .collect();
        parts.push(Some(Part {
            type_guid,
            guid,
            first,
            last,
            attrs: le64(e, 48),
            name: String::from_utf16_lossy(&units),
        }));
    }
    while parts.last() == Some(&None) {
        parts.pop();
    }
    Ok(Table { disk_guid, parts })
}

/// The first and last sectors a partition may use on a disk of `sectors`.
pub fn usable(sectors: u64) -> (u64, u64) {
    (2 + ENTRY_SECTORS, sectors - 2 - ENTRY_SECTORS)
}

/// Writes the protective MBR, the primary header and entries, and the
/// backup entries and header at the end of a disk of `sectors` sectors.
pub fn write<W: WriteAt + ?Sized>(w: &W, sectors: u64, table: &Table) -> io::Result<()> {
    let (first_usable, last_usable) = usable(sectors);
    if table.parts.len() > ENTRIES {
        return Err(bad("too many partitions"));
    }
    let mut ents = vec![0u8; ENTRIES * ENTRY_SIZE];
    for (i, p) in table.parts.iter().enumerate() {
        let Some(p) = p else { continue };
        if p.first < first_usable || p.last > last_usable || p.last < p.first {
            return Err(bad(format!(
                "partition {} ({}..{}) is outside {first_usable}..{last_usable}",
                i + 1,
                p.first,
                p.last
            )));
        }
        let units: Vec<u16> = p.name.encode_utf16().collect();
        if units.len() > 36 {
            return Err(bad(format!("partition name {:?} is too long", p.name)));
        }
        let e = &mut ents[i * ENTRY_SIZE..(i + 1) * ENTRY_SIZE];
        e[0..16].copy_from_slice(&p.type_guid);
        e[16..32].copy_from_slice(&p.guid);
        put64(e, 32, p.first);
        put64(e, 40, p.last);
        put64(e, 48, p.attrs);
        for (j, u) in units.iter().enumerate() {
            put16(e, 56 + 2 * j, *u);
        }
    }
    for (i, a) in table.parts.iter().enumerate() {
        for b in table.parts.iter().skip(i + 1) {
            if let (Some(a), Some(b)) = (a, b) {
                if a.first <= b.last && b.first <= a.last {
                    return Err(bad(format!("partitions {} and {} overlap", a.name, b.name)));
                }
            }
        }
    }
    let ents_crc = crc32(&ents);
    let header = |current: u64, backup: u64, entries_at: u64| {
        let mut h = [0u8; 512];
        h[0..8].copy_from_slice(b"EFI PART");
        put32(&mut h, 8, 0x0001_0000);
        put32(&mut h, 12, 92);
        put64(&mut h, 24, current);
        put64(&mut h, 32, backup);
        put64(&mut h, 40, first_usable);
        put64(&mut h, 48, last_usable);
        h[56..72].copy_from_slice(&table.disk_guid);
        put64(&mut h, 72, entries_at);
        put32(&mut h, 80, ENTRIES as u32);
        put32(&mut h, 84, ENTRY_SIZE as u32);
        put32(&mut h, 88, ents_crc);
        let c = crc32(&h[..92]);
        put32(&mut h, 16, c);
        h
    };
    let mut mbr = [0u8; 512];
    let e = &mut mbr[446..462];
    e[1..4].copy_from_slice(&[0x00, 0x02, 0x00]);
    e[4] = 0xEE;
    e[5..8].copy_from_slice(&[0xFF, 0xFF, 0xFF]);
    put32(e, 8, 1);
    put32(e, 12, (sectors - 1).min(0xFFFF_FFFF) as u32);
    mbr[510] = 0x55;
    mbr[511] = 0xAA;
    let last = sectors - 1;
    let backup_entries = last - ENTRY_SECTORS;
    w.write_at(&mbr, 0)?;
    w.write_at(&header(1, last, 2), SECTOR)?;
    w.write_at(&ents, 2 * SECTOR)?;
    w.write_at(&ents, backup_entries * SECTOR)?;
    w.write_at(&header(last, 1, backup_entries), last * SECTOR)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    struct Mem(RefCell<Vec<u8>>);
    impl WriteAt for Mem {
        fn write_at(&self, buf: &[u8], offset: u64) -> io::Result<()> {
            let mut v = self.0.borrow_mut();
            let o = offset as usize;
            v[o..o + buf.len()].copy_from_slice(buf);
            Ok(())
        }
    }

    #[test]
    fn guid_round_trip() {
        let g = guid(LINUX_FS).unwrap();
        assert_eq!(&g[0..4], &[0xAF, 0x3D, 0xC6, 0x0F]);
        assert_eq!(guid_text(&g), LINUX_FS);
        let r = random_guid();
        assert_eq!(guid(&guid_text(&r)).unwrap(), r);
        assert_ne!(random_guid(), random_guid());
    }

    #[test]
    fn write_then_read() {
        let sectors = 8192u64;
        let mem = Mem(RefCell::new(vec![0u8; (sectors * SECTOR) as usize]));
        let t = Table {
            disk_guid: random_guid(),
            parts: vec![
                Some(Part {
                    type_guid: guid(LINUX_FS).unwrap(),
                    guid: random_guid(),
                    first: 2048,
                    last: 4095,
                    attrs: 0,
                    name: "mpfree-test-root".into(),
                }),
                None,
                Some(Part {
                    type_guid: guid(BASIC_DATA).unwrap(),
                    guid: random_guid(),
                    first: 4096,
                    last: 8000,
                    attrs: 0,
                    name: "MPFREE-SETUP".into(),
                }),
            ],
        };
        write(&mem, sectors, &t).unwrap();
        let data = mem.0.into_inner();
        let back = read(data.as_slice()).unwrap();
        assert_eq!(back, t);
        assert_eq!(back.by_name("MPFREE-SETUP").unwrap().0, 3);
        assert!(back.part(2).is_none());
        // The backup header is where the primary says.
        assert_eq!(&data[((sectors - 1) * SECTOR) as usize..][..8], b"EFI PART");
        assert_eq!(&data[510..512], &[0x55, 0xAA]);
    }

    #[test]
    fn overlapping_partitions_are_refused() {
        let mem = Mem(RefCell::new(vec![0u8; 8192 * 512]));
        let p = |first, last| {
            Some(Part {
                type_guid: guid(LINUX_FS).unwrap(),
                guid: random_guid(),
                first,
                last,
                attrs: 0,
                name: "x".into(),
            })
        };
        let t = Table {
            disk_guid: random_guid(),
            parts: vec![p(2048, 4095), p(4000, 5000)],
        };
        assert!(write(&mem, 8192, &t).is_err());
    }
}
