//! A read-only ext2/3/4 reader.
//!
//! It reads what MPFree needs from Akai's root filesystem (in an update
//! file) and from the factory content partition (in an eMMC backup), and
//! what the creator copies off a stick: folders, files, links and their
//! owners, modes, times and extended attributes. It understands extents
//! and block maps, 32- and 64-bit group descriptors, meta_bg, inline data
//! and hash-indexed folders (read as plain folders: every leaf block is an
//! ordinary folder block). It never writes, never replays a journal and
//! checks no checksums; it bounds every count it reads, so a damaged image
//! gives an error, not a hang.

use crate::{bad, le16, le32, ReadAt};
use std::io::{self, Write};

pub const ROOT: u32 = 2;

const MAGIC: u16 = 0xEF53;
const INCOMPAT_FILETYPE: u32 = 0x2;
const INCOMPAT_META_BG: u32 = 0x10;
const INCOMPAT_64BIT: u32 = 0x80;
const INCOMPAT_INLINE_DATA: u32 = 0x8000;
const INCOMPAT_ENCRYPT: u32 = 0x10000;
const INCOMPAT_COMPRESSION: u32 = 0x1;
const INCOMPAT_JOURNAL_DEV: u32 = 0x8;
const INCOMPAT_DIRDATA: u32 = 0x1000;
/// Incompatible features this reader handles; any other one stops it.
const INCOMPAT_KNOWN: u32 =
    0x2 | 0x4 | 0x10 | 0x40 | 0x80 | 0x100 | 0x200 | 0x400 | 0x2000 | 0x4000 | 0x8000 | 0x20000;
const RO_COMPAT_HUGE_FILE: u32 = 0x8;

const FLAG_HUGE_FILE: u32 = 0x40000;
const FLAG_EXTENTS: u32 = 0x80000;
const FLAG_EA_INODE: u32 = 0x200000;
const FLAG_INLINE_DATA: u32 = 0x1000_0000;

pub const S_IFMT: u16 = 0o170000;
pub const S_IFSOCK: u16 = 0o140000;
pub const S_IFLNK: u16 = 0o120000;
pub const S_IFREG: u16 = 0o100000;
pub const S_IFBLK: u16 = 0o060000;
pub const S_IFDIR: u16 = 0o040000;
pub const S_IFCHR: u16 = 0o020000;
pub const S_IFIFO: u16 = 0o010000;

/// The largest file this reader hands back whole ([`Ext4::read_file`]).
pub const MAX_WHOLE_FILE: u64 = 1 << 30;
const MAX_EXTENT_DEPTH: u16 = 5;
const MAX_DIR_BYTES: u64 = 256 << 20;

/// What kind of thing an inode is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    File,
    Dir,
    Symlink,
    CharDev,
    BlockDev,
    Fifo,
    Socket,
}

impl Kind {
    pub fn of_mode(mode: u16) -> Option<Kind> {
        Some(match mode & S_IFMT {
            S_IFREG => Kind::File,
            S_IFDIR => Kind::Dir,
            S_IFLNK => Kind::Symlink,
            S_IFCHR => Kind::CharDev,
            S_IFBLK => Kind::BlockDev,
            S_IFIFO => Kind::Fifo,
            S_IFSOCK => Kind::Socket,
            _ => return None,
        })
    }
}

/// One inode, decoded.
#[derive(Clone, Debug)]
pub struct Inode {
    pub ino: u32,
    /// Type and permission bits, as stored.
    pub mode: u16,
    pub uid: u32,
    pub gid: u32,
    pub size: u64,
    pub atime: u32,
    pub ctime: u32,
    pub mtime: u32,
    pub links: u16,
    pub flags: u32,
    /// i_blocks, in 512-byte units (already scaled for huge files).
    pub sectors: u64,
    /// The 60 bytes of i_block.
    pub block: [u8; 60],
    /// The extended attribute block, or 0.
    pub file_acl: u64,
    /// The bytes past the first 128 (extra fields, in-inode attributes).
    pub extra: Vec<u8>,
}

impl Inode {
    pub fn kind(&self) -> Option<Kind> {
        Kind::of_mode(self.mode)
    }

    pub fn is_dir(&self) -> bool {
        self.mode & S_IFMT == S_IFDIR
    }

    /// Permission bits, with setuid, setgid and sticky.
    pub fn perm(&self) -> u16 {
        self.mode & 0o7777
    }

    /// The device number of a character or block device (major, minor).
    pub fn rdev(&self) -> (u32, u32) {
        let a = le32(&self.block, 0);
        if a != 0 {
            ((a >> 8) & 0xFF, a & 0xFF)
        } else {
            let b = le32(&self.block, 4);
            ((b & 0xFFF00) >> 8, (b & 0xFF) | ((b >> 12) & 0xFFF00))
        }
    }
}

/// A folder entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: Vec<u8>,
    pub ino: u32,
}

/// An open filesystem.
pub struct Ext4<R> {
    r: R,
    pub block_size: u64,
    pub inode_size: u64,
    pub inodes_per_group: u32,
    pub inodes_count: u32,
    pub blocks_count: u64,
    pub groups: u32,
    pub feature_compat: u32,
    pub feature_incompat: u32,
    pub feature_ro_compat: u32,
    pub label: String,
    pub uuid: [u8; 16],
    /// Where each group's inode table starts (block number).
    inode_tables: Vec<u64>,
}

impl<R: ReadAt> Ext4<R> {
    /// Opens the filesystem `r` holds from its byte 0.
    pub fn open(r: R) -> io::Result<Ext4<R>> {
        let mut sb = [0u8; 1024];
        r.read_at(&mut sb, 1024)?;
        if le16(&sb, 0x38) != MAGIC {
            return Err(bad("not an ext2/3/4 filesystem (no magic number)"));
        }
        let log = le32(&sb, 0x18);
        if log > 6 {
            return Err(bad(format!("block size 1024 << {log} is implausible")));
        }
        let block_size = 1024u64 << log;
        let rev = le32(&sb, 0x4C);
        let inode_size = if rev == 0 {
            128
        } else {
            le16(&sb, 0x58) as u64
        };
        if inode_size < 128 || inode_size > block_size || !inode_size.is_power_of_two() {
            return Err(bad(format!("inode size {inode_size} is implausible")));
        }
        let incompat = if rev == 0 { 0 } else { le32(&sb, 0x60) };
        let compat = if rev == 0 { 0 } else { le32(&sb, 0x5C) };
        let ro_compat = if rev == 0 { 0 } else { le32(&sb, 0x64) };
        if incompat
            & (INCOMPAT_COMPRESSION | INCOMPAT_JOURNAL_DEV | INCOMPAT_DIRDATA | INCOMPAT_ENCRYPT)
            != 0
            || incompat & !INCOMPAT_KNOWN != 0
        {
            return Err(bad(format!(
                "unsupported ext4 features (incompat {incompat:#x})"
            )));
        }
        let is64 = incompat & INCOMPAT_64BIT != 0;
        let blocks_count = le32(&sb, 0x04) as u64
            | if is64 {
                (le32(&sb, 0x150) as u64) << 32
            } else {
                0
            };
        let first_data_block = le32(&sb, 0x14) as u64;
        let blocks_per_group = le32(&sb, 0x20) as u64;
        let inodes_per_group = le32(&sb, 0x28);
        let inodes_count = le32(&sb, 0x00);
        if blocks_per_group == 0 || inodes_per_group == 0 || blocks_count <= first_data_block {
            return Err(bad("the superblock's group sizes are implausible"));
        }
        let groups64 = (blocks_count - first_data_block).div_ceil(blocks_per_group);
        if groups64 == 0 || groups64 > 1 << 24 {
            return Err(bad(format!("{groups64} block groups is implausible")));
        }
        let groups = groups64 as u32;
        if (groups as u64) * (inodes_per_group as u64) < inodes_count as u64 {
            return Err(bad("the inode count does not fit the groups"));
        }
        let desc_size = if is64 {
            (le16(&sb, 0xFE) as u64).max(32)
        } else {
            32
        };
        if desc_size > block_size || desc_size % 4 != 0 {
            return Err(bad(format!(
                "group descriptor size {desc_size} is implausible"
            )));
        }
        let per_block = block_size / desc_size;
        let meta_bg = incompat & INCOMPAT_META_BG != 0;
        let first_meta_bg = if meta_bg {
            le32(&sb, 0x104) as u64
        } else {
            u64::MAX
        };
        let sparse = ro_compat & 0x1 != 0;
        let mut inode_tables = Vec::with_capacity(groups as usize);
        let mut buf = vec![0u8; block_size as usize];
        let mut loaded = u64::MAX;
        for g in 0..groups as u64 {
            let desc_block = g / per_block;
            let at = if desc_block < first_meta_bg {
                first_data_block + 1 + desc_block
            } else {
                // meta_bg: the descriptors of each run of groups sit in its first group
                // (after that group's superblock backup, if it has one).
                let lead = desc_block * per_block;
                let sb_here = has_super(lead, sparse) as u64;
                first_data_block + lead * blocks_per_group + sb_here
            };
            if at != loaded {
                r.read_at(&mut buf, at * block_size)?;
                loaded = at;
            }
            let d = &buf[((g % per_block) * desc_size) as usize..][..desc_size as usize];
            let mut table = le32(d, 8) as u64;
            if is64 && desc_size >= 64 {
                table |= (le32(d, 0x28) as u64) << 32;
            }
            if table == 0 || table >= blocks_count {
                return Err(bad(format!(
                    "group {g}'s inode table is outside the filesystem"
                )));
            }
            inode_tables.push(table);
        }
        let label = String::from_utf8_lossy(&sb[0x78..0x88])
            .trim_end_matches('\0')
            .to_string();
        let mut uuid = [0u8; 16];
        uuid.copy_from_slice(&sb[0x68..0x78]);
        Ok(Ext4 {
            r,
            block_size,
            inode_size,
            inodes_per_group,
            inodes_count,
            blocks_count,
            groups,
            feature_compat: compat,
            feature_incompat: incompat,
            feature_ro_compat: ro_compat,
            label,
            uuid,
            inode_tables,
        })
    }

    fn read(&self, buf: &mut [u8], offset: u64) -> io::Result<()> {
        self.r.read_at(buf, offset)
    }

    fn block(&self, n: u64) -> io::Result<Vec<u8>> {
        if n >= self.blocks_count {
            return Err(bad(format!("block {n} is outside the filesystem")));
        }
        let mut b = vec![0u8; self.block_size as usize];
        self.read(&mut b, n * self.block_size)?;
        Ok(b)
    }

    /// Inode number `ino`.
    pub fn inode(&self, ino: u32) -> io::Result<Inode> {
        if ino == 0 || ino > self.inodes_count {
            return Err(bad(format!("inode {ino} is outside the filesystem")));
        }
        let g = (ino - 1) / self.inodes_per_group;
        let i = (ino - 1) % self.inodes_per_group;
        let at = self.inode_tables[g as usize] * self.block_size + i as u64 * self.inode_size;
        let mut raw = vec![0u8; self.inode_size as usize];
        self.read(&mut raw, at)?;
        let mode = le16(&raw, 0);
        let uid = le16(&raw, 2) as u32 | (le16(&raw, 0x78) as u32) << 16;
        let gid = le16(&raw, 0x18) as u32 | (le16(&raw, 0x7A) as u32) << 16;
        let size = le32(&raw, 4) as u64 | (le32(&raw, 0x6C) as u64) << 32;
        let flags = le32(&raw, 0x20);
        let mut sectors = le32(&raw, 0x1C) as u64 | (le16(&raw, 0x74) as u64) << 32;
        if self.feature_ro_compat & RO_COMPAT_HUGE_FILE != 0 && flags & FLAG_HUGE_FILE != 0 {
            sectors *= self.block_size / 512;
        }
        let mut block = [0u8; 60];
        block.copy_from_slice(&raw[0x28..0x64]);
        let file_acl = le32(&raw, 0x68) as u64 | (le16(&raw, 0x76) as u64) << 32;
        Ok(Inode {
            ino,
            mode,
            uid,
            gid,
            size,
            atime: le32(&raw, 0x08),
            ctime: le32(&raw, 0x0C),
            mtime: le32(&raw, 0x10),
            links: le16(&raw, 0x1A),
            flags,
            sectors,
            block,
            file_acl,
            extra: raw[128..].to_vec(),
        })
    }

    /// The physical block of every logical block `0..count` (0 for a hole
    /// or an unwritten extent), in order.
    fn map(&self, ino: &Inode, count: u64) -> io::Result<Vec<u64>> {
        if count > (MAX_WHOLE_FILE * 4) / self.block_size + 4 {
            return Err(bad(format!("inode {} is implausibly large", ino.ino)));
        }
        let mut out = vec![0u64; count as usize];
        if ino.flags & FLAG_EXTENTS != 0 {
            self.extents(&ino.block, MAX_EXTENT_DEPTH, &mut out)?;
        } else {
            let p = |i: usize| le32(&ino.block, i * 4) as u64;
            let per = self.block_size / 4;
            let mut l = 0u64;
            for i in 0..12 {
                if l >= count {
                    return Ok(out);
                }
                out[l as usize] = p(i);
                l += 1;
            }
            for (slot, depth) in [(12usize, 1u32), (13, 2), (14, 3)] {
                if l >= count {
                    break;
                }
                let span = per.pow(depth);
                self.indirect(p(slot), depth, &mut out, l)?;
                l += span;
            }
        }
        Ok(out)
    }

    fn indirect(&self, blk: u64, depth: u32, out: &mut [u64], first: u64) -> io::Result<()> {
        if blk == 0 || first >= out.len() as u64 {
            return Ok(());
        }
        let b = self.block(blk)?;
        let per = self.block_size / 4;
        let span = per.pow(depth - 1);
        for i in 0..per {
            let l = first + i * span;
            if l >= out.len() as u64 {
                break;
            }
            let child = le32(&b, (i * 4) as usize) as u64;
            if depth == 1 {
                out[l as usize] = child;
            } else {
                self.indirect(child, depth - 1, out, l)?;
            }
        }
        Ok(())
    }

    fn extents(&self, node: &[u8], depth_left: u16, out: &mut [u64]) -> io::Result<()> {
        if le16(node, 0) != 0xF30A {
            return Err(bad("bad extent header"));
        }
        let entries = le16(node, 2) as usize;
        let depth = le16(node, 6);
        if depth > depth_left || 12 + entries * 12 > node.len() {
            return Err(bad("bad extent tree"));
        }
        for e in 0..entries {
            let x = &node[12 + e * 12..24 + e * 12];
            if depth == 0 {
                let lblk = le32(x, 0) as u64;
                let raw_len = le16(x, 4) as u64;
                let (len, unwritten) = if raw_len > 32768 {
                    (raw_len - 32768, true)
                } else {
                    (raw_len, false)
                };
                let start = (le16(x, 6) as u64) << 32 | le32(x, 8) as u64;
                for k in 0..len {
                    let l = lblk + k;
                    if l >= out.len() as u64 {
                        break;
                    }
                    out[l as usize] = if unwritten { 0 } else { start + k };
                }
            } else {
                let leaf = (le16(x, 8) as u64) << 32 | le32(x, 4) as u64;
                let child = self.block(leaf)?;
                self.extents(&child, depth - 1, out)?;
            }
        }
        Ok(())
    }

    /// The bytes held inline (inline_data): i_block, then the system.data attribute.
    fn inline(&self, ino: &Inode) -> io::Result<Vec<u8>> {
        let mut data = ino.block.to_vec();
        if ino.size > 60 {
            for (name, value) in self.xattrs(ino)? {
                if name == "system.data" {
                    data.extend_from_slice(&value);
                }
            }
        }
        data.truncate(ino.size as usize);
        if (data.len() as u64) < ino.size {
            return Err(bad(format!(
                "inode {}: inline data is shorter than its size",
                ino.ino
            )));
        }
        Ok(data)
    }

    /// Streams a regular file's contents into `w`.
    pub fn copy_file<W: Write + ?Sized>(&self, ino: &Inode, w: &mut W) -> io::Result<u64> {
        if ino.flags & FLAG_INLINE_DATA != 0 && self.feature_incompat & INCOMPAT_INLINE_DATA != 0 {
            let d = self.inline(ino)?;
            w.write_all(&d)?;
            return Ok(d.len() as u64);
        }
        let bs = self.block_size;
        let count = ino.size.div_ceil(bs);
        let map = self.map(ino, count)?;
        let mut left = ino.size;
        let zero = vec![0u8; bs as usize];
        let mut buf = vec![0u8; bs as usize];
        let mut i = 0usize;
        while i < map.len() {
            // Read runs of consecutive blocks at once.
            let mut run = 1usize;
            if map[i] != 0 {
                while i + run < map.len() && map[i + run] == map[i] + run as u64 && run < 256 {
                    run += 1;
                }
                let bytes = (run as u64 * bs).min(left) as usize;
                if buf.len() < run * bs as usize {
                    buf.resize(run * bs as usize, 0);
                }
                if map[i] + run as u64 > self.blocks_count {
                    return Err(bad(format!(
                        "inode {}: data outside the filesystem",
                        ino.ino
                    )));
                }
                self.read(&mut buf[..run * bs as usize], map[i] * bs)?;
                w.write_all(&buf[..bytes])?;
                left -= bytes as u64;
            } else {
                let bytes = bs.min(left) as usize;
                w.write_all(&zero[..bytes])?;
                left -= bytes as u64;
            }
            i += run;
        }
        Ok(ino.size)
    }

    /// A regular file's contents, whole (at most [`MAX_WHOLE_FILE`] bytes).
    pub fn read_file(&self, ino: &Inode) -> io::Result<Vec<u8>> {
        if ino.size > MAX_WHOLE_FILE {
            return Err(bad(format!(
                "inode {} is {} bytes, too big to read whole",
                ino.ino, ino.size
            )));
        }
        let mut v = Vec::with_capacity(ino.size as usize);
        self.copy_file(ino, &mut v)?;
        Ok(v)
    }

    /// A symbolic link's target.
    pub fn read_link(&self, ino: &Inode) -> io::Result<Vec<u8>> {
        if ino.size > 4096 {
            return Err(bad(format!(
                "link inode {} is {} bytes long",
                ino.ino, ino.size
            )));
        }
        if ino.flags & FLAG_INLINE_DATA != 0 {
            return self.inline(ino);
        }
        let ea_sectors = if ino.file_acl != 0 {
            self.block_size / 512
        } else {
            0
        };
        let fast = ino.flags & FLAG_EA_INODE == 0 && ino.sectors.saturating_sub(ea_sectors) == 0;
        if fast {
            if ino.size > 60 {
                return Err(bad(format!(
                    "fast link inode {} is longer than 60 bytes",
                    ino.ino
                )));
            }
            return Ok(ino.block[..ino.size as usize].to_vec());
        }
        self.read_file(ino)
    }

    /// A folder's entries, without "." and "..".
    pub fn read_dir(&self, ino: &Inode) -> io::Result<Vec<Entry>> {
        if !ino.is_dir() {
            return Err(bad(format!("inode {} is not a folder", ino.ino)));
        }
        if ino.size > MAX_DIR_BYTES {
            return Err(bad(format!(
                "folder inode {} is implausibly large",
                ino.ino
            )));
        }
        let filetype = self.feature_incompat & INCOMPAT_FILETYPE != 0;
        let mut out = Vec::new();
        if ino.flags & FLAG_INLINE_DATA != 0 && self.feature_incompat & INCOMPAT_INLINE_DATA != 0 {
            // An inline folder: 4 bytes of the parent's inode, then entries.
            let data = self.inline(ino)?;
            parse_entries(&data[4.min(data.len())..], filetype, &mut out)?;
            return Ok(out);
        }
        let mut data = Vec::with_capacity(ino.size as usize);
        self.copy_file(ino, &mut data)?;
        for block in data.chunks(self.block_size as usize) {
            parse_entries(block, filetype, &mut out)?;
        }
        Ok(out)
    }

    /// The inode at `path` ("/etc/passwd"), without following links, or None.
    pub fn lookup(&self, path: &str) -> io::Result<Option<Inode>> {
        let mut cur = self.inode(ROOT)?;
        for part in path.split('/').filter(|p| !p.is_empty()) {
            if !cur.is_dir() {
                return Ok(None);
            }
            let found = self
                .read_dir(&cur)?
                .into_iter()
                .find(|e| e.name == part.as_bytes());
            match found {
                Some(e) => cur = self.inode(e.ino)?,
                None => return Ok(None),
            }
        }
        Ok(Some(cur))
    }

    /// Every extended attribute of an inode, with its full name ("security.capability").
    pub fn xattrs(&self, ino: &Inode) -> io::Result<Vec<(String, Vec<u8>)>> {
        let mut out = Vec::new();
        // In the inode, after i_extra_isize.
        if self.inode_size > 128 && ino.extra.len() >= 4 {
            let extra_isize = le16(&ino.extra, 0) as usize;
            let start = extra_isize;
            if extra_isize.is_multiple_of(4)
                && start + 4 <= ino.extra.len()
                && le32(&ino.extra, start) == 0xEA02_0000
            {
                let area = &ino.extra[start + 4..];
                parse_xattrs(area, area, &mut out)?;
            }
        }
        if ino.file_acl != 0 {
            let b = self.block(ino.file_acl)?;
            if le32(&b, 0) != 0xEA02_0000 {
                return Err(bad(format!(
                    "inode {}: bad extended attribute block",
                    ino.ino
                )));
            }
            parse_xattrs(&b[32..], &b, &mut out)?;
        }
        Ok(out)
    }

    /// Walks the tree from the root, depth first, in folder order, calling
    /// `f(path, inode)` for every entry (the root itself first, as "/").
    /// Folders nest at most 64 deep.
    pub fn walk(&self, f: &mut dyn FnMut(&str, &Inode) -> io::Result<()>) -> io::Result<()> {
        let root = self.inode(ROOT)?;
        f("/", &root)?;
        self.walk_in("", &root, f, 0)
    }

    fn walk_in(
        &self,
        prefix: &str,
        dir: &Inode,
        f: &mut dyn FnMut(&str, &Inode) -> io::Result<()>,
        depth: usize,
    ) -> io::Result<()> {
        if depth > 64 {
            return Err(bad(format!("folders nest more than 64 deep at {prefix}")));
        }
        for e in self.read_dir(dir)? {
            let name = String::from_utf8_lossy(&e.name).into_owned();
            let path = format!("{prefix}/{name}");
            let ino = self.inode(e.ino)?;
            f(&path, &ino)?;
            if ino.is_dir() {
                self.walk_in(&path, &ino, f, depth + 1)?;
            }
        }
        Ok(())
    }
}

/// Is there a superblock backup in group `g`?
pub(crate) fn has_super(g: u64, sparse: bool) -> bool {
    if g <= 1 || !sparse {
        return true;
    }
    for base in [3u64, 5, 7] {
        let mut p = base;
        while p < g {
            p *= base;
        }
        if p == g {
            return true;
        }
    }
    false
}

fn parse_entries(block: &[u8], filetype: bool, out: &mut Vec<Entry>) -> io::Result<()> {
    let mut at = 0usize;
    while at + 8 <= block.len() {
        let ino = le32(block, at);
        let rec_len = le16(block, at + 4) as usize;
        let name_len = if filetype {
            block[at + 6] as usize
        } else {
            le16(block, at + 6) as usize
        };
        if rec_len < 8 || at + rec_len > block.len() || !rec_len.is_multiple_of(4) {
            return Err(bad("a damaged folder entry"));
        }
        if ino != 0 && name_len > 0 {
            if 8 + name_len > rec_len {
                return Err(bad("a folder entry's name runs past it"));
            }
            let name = block[at + 8..at + 8 + name_len].to_vec();
            if name != b"." && name != b".." {
                out.push(Entry { name, ino });
            }
        }
        at += rec_len;
    }
    Ok(())
}

fn prefix(index: u8) -> &'static str {
    match index {
        1 => "user.",
        2 => "system.posix_acl_access",
        3 => "system.posix_acl_default",
        4 => "trusted.",
        6 => "security.",
        7 => "system.",
        8 => "system.richacl",
        _ => "",
    }
}

/// Entries start at `entries`; values are at offsets from `base`.
fn parse_xattrs(entries: &[u8], base: &[u8], out: &mut Vec<(String, Vec<u8>)>) -> io::Result<()> {
    let mut at = 0usize;
    for _ in 0..1024 {
        if at + 4 > entries.len() || le32(entries, at) == 0 {
            return Ok(());
        }
        if at + 16 > entries.len() {
            return Err(bad("a damaged extended attribute entry"));
        }
        let name_len = entries[at] as usize;
        let index = entries[at + 1];
        let value_offs = le16(entries, at + 2) as usize;
        let value_inum = le32(entries, at + 4);
        let value_size = le32(entries, at + 8) as usize;
        if at + 16 + name_len > entries.len() {
            return Err(bad("an extended attribute name runs past its block"));
        }
        let name = format!(
            "{}{}",
            prefix(index),
            String::from_utf8_lossy(&entries[at + 16..at + 16 + name_len])
        );
        if value_inum != 0 {
            return Err(bad(
                "extended attributes in their own inodes are not supported",
            ));
        }
        let value = base
            .get(value_offs..value_offs + value_size)
            .ok_or_else(|| bad("an extended attribute value runs past its block"))?
            .to_vec();
        out.push((name, value));
        at += (16 + name_len + 3) & !3;
    }
    Err(bad("too many extended attributes"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    use std::path::{Path, PathBuf};
    use std::process::Command;

    fn mke2fs() -> Option<&'static str> {
        for p in ["/usr/sbin/mke2fs", "/sbin/mke2fs"] {
            if Path::new(p).exists() {
                return Some(p);
            }
        }
        if std::env::var("MPFREE_REQUIRE_OFFLINE_TOOLS").as_deref() == Ok("1") {
            panic!("mke2fs is missing and MPFREE_REQUIRE_OFFLINE_TOOLS=1");
        }
        None
    }

    struct Scratch(PathBuf);

    impl Drop for Scratch {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).ok();
        }
    }

    fn pattern(len: usize, seed: u32) -> Vec<u8> {
        (0..len)
            .map(|i| ((i as u32).wrapping_mul(2_654_435_761) >> (seed % 24)) as u8)
            .collect()
    }

    /// A folder of files covering what the reader handles.
    fn source(dir: &Path) {
        let w = |p: &str, data: &[u8], mode: u32| {
            let path = dir.join(p);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, data).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
        };
        w("small.txt", b"hello, inline world\n", 0o644);
        w("empty", b"", 0o600);
        w("bin/big", &pattern(5 << 20, 3), 0o4755);
        w("bin/twelve", &pattern(12 * 1024 + 1, 5), 0o755);
        // A file with a hole in the middle.
        {
            use std::io::{Seek, SeekFrom};
            let path = dir.join("sparse");
            let mut f = std::fs::File::create(&path).unwrap();
            f.write_all(b"start").unwrap();
            f.seek(SeekFrom::Start(3 << 20)).unwrap();
            f.write_all(b"end").unwrap();
        }
        for i in 0..2000 {
            w(
                &format!("many/file-number-{i:05}"),
                format!("{i}").as_bytes(),
                0o644,
            );
        }
        std::fs::create_dir_all(dir.join("empty-dir/inner")).unwrap();
        std::os::unix::fs::symlink("small.txt", dir.join("fast-link")).unwrap();
        std::os::unix::fs::symlink("x".repeat(100), dir.join("slow-link")).unwrap();
    }

    /// Checks the image against the folder it was made from.
    fn compare(src: &Path, fs: &Ext4<std::fs::File>) {
        let mut count = 0;
        fs.walk(&mut |path, ino| {
            if path == "/" || path.starts_with("/lost+found") {
                return Ok(());
            }
            count += 1;
            let local = src.join(&path[1..]);
            let meta = std::fs::symlink_metadata(&local)
                .unwrap_or_else(|_| panic!("{path} is not in the source"));
            assert_eq!(ino.mode as u32 & 0o7777, meta.mode() & 0o7777, "{path}");
            if meta.file_type().is_symlink() {
                assert_eq!(ino.kind(), Some(Kind::Symlink));
                let target = std::fs::read_link(&local).unwrap();
                assert_eq!(
                    fs.read_link(ino)?,
                    target.as_os_str().as_encoded_bytes(),
                    "{path}"
                );
            } else if meta.is_dir() {
                assert!(ino.is_dir(), "{path}");
                let mut names: Vec<_> = fs.read_dir(ino)?.into_iter().map(|e| e.name).collect();
                names.sort();
                let mut want: Vec<_> = std::fs::read_dir(&local)
                    .unwrap()
                    .map(|e| e.unwrap().file_name().as_encoded_bytes().to_vec())
                    .collect();
                want.sort();
                assert_eq!(names, want, "{path}");
            } else {
                assert_eq!(ino.kind(), Some(Kind::File));
                assert_eq!(ino.size, meta.len(), "{path}");
                assert!(
                    fs.read_file(ino)? == std::fs::read(&local).unwrap(),
                    "{path}: contents differ"
                );
            }
            Ok(())
        })
        .unwrap();
        assert!(count > 2000, "{count} entries");
        assert!(fs.lookup("/many/file-number-01999").unwrap().is_some());
        assert!(fs.lookup("/many/nope").unwrap().is_none());
        assert!(fs.lookup("/small.txt/x").unwrap().is_none());
    }

    #[test]
    fn reads_what_mke2fs_makes() {
        let Some(tool) = mke2fs() else {
            eprintln!("mke2fs not installed; skipping");
            return;
        };
        let scratch =
            Scratch(std::env::temp_dir().join(format!("mpfree-ext4-{}", std::process::id())));
        let src = scratch.0.join("src");
        std::fs::create_dir_all(&src).unwrap();
        source(&src);
        let cases: &[(&str, &[&str], u64)] = &[
            ("ext4", &["-t", "ext4"], 64),
            ("inline", &["-t", "ext4", "-O", "inline_data"], 64),
            ("ext3", &["-t", "ext3"], 64),
            ("ext2", &["-t", "ext2"], 64),
            (
                "meta_bg-1k",
                &["-t", "ext4", "-b", "1024", "-O", "meta_bg,^resize_inode"],
                200,
            ),
        ];
        for (name, args, mib) in cases {
            let img = scratch.0.join(format!("{name}.img"));
            std::fs::File::create(&img)
                .unwrap()
                .set_len(mib << 20)
                .unwrap();
            let out = Command::new(tool)
                .args(["-q", "-F", "-L", "READER-TEST", "-E", "root_owner=0:0"])
                .args(*args)
                .arg("-d")
                .arg(&src)
                .arg(&img)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "mke2fs {name}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            let fs = Ext4::open(std::fs::File::open(&img).unwrap()).unwrap();
            assert_eq!(fs.label, "READER-TEST", "{name}");
            if *name == "meta_bg-1k" {
                assert!(fs.groups > 16, "{name}: {} groups", fs.groups);
                assert_eq!(fs.block_size, 1024);
            }
            compare(&src, &fs);
        }
    }

    #[test]
    fn rejects_what_is_not_ext() {
        let zeros = vec![0u8; 8192];
        assert!(Ext4::open(&zeros).is_err());
        let mut sb = vec![0u8; 8192];
        sb[1024 + 0x38] = 0x53;
        sb[1024 + 0x39] = 0xEF;
        sb[1024 + 0x18] = 9; // absurd block size
        assert!(Ext4::open(&sb).is_err());
    }
}
