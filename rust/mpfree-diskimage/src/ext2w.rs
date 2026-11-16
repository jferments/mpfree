//! A writer for fresh ext2-style filesystems, built from a tree in memory.
//!
//! The layout is plain ext2 revision 1: 4 KiB blocks, 256-byte inodes,
//! block maps (no extents), linear folders, no journal and no checksums,
//! with the features filetype, sparse_super and large_file (and ext_attr
//! when any entry has extended attributes). Linux's ext4 driver mounts it,
//! read-only or read-write, and `e2fsck -fn` finds nothing to fix (the
//! tests check both with e2fsprogs where it is installed).
//!
//! Files are written whole (no holes). Their bytes come from memory or
//! from a closure that writes them while the image is written, so a tree
//! read from another image never has to sit in memory. Every block the
//! filesystem uses is written, metadata included, so the target need not
//! be zeroed first; free blocks are left as they were.
//!
//! Inode numbers follow the tree: the root is 2, lost+found (made here,
//! always, as mke2fs makes it) is 11, and the rest count up from 12 in
//! depth-first name order, so the same tree always gives the same image
//! (with the same options).

use crate::ext4::has_super;
use crate::{bad, put16, put32, WriteAt};
use std::collections::{BTreeMap, HashMap};
use std::io::{self, Write};

/// The block size.
pub const BLOCK: u64 = 4096;
const INODE_SIZE: usize = 256;
const EXTRA_ISIZE: u16 = 32;
const FIRST_INO: u32 = 11;
const LOST_FOUND: u32 = 11;
const LOST_FOUND_BLOCKS: u64 = 4;
const BLOCKS_PER_GROUP: u64 = BLOCK * 8;
const MAX_IPG: u64 = BLOCK * 8;
const INODES_PER_BLOCK: u64 = BLOCK / INODE_SIZE as u64;
const PER_BLOCK: u64 = BLOCK / 4;
const DIRECT: u64 = 12;
/// The largest file: what twelve direct blocks, one indirect and one
/// double indirect block map (just over 4 GiB).
pub const MAX_FILE: u64 = (DIRECT + PER_BLOCK + PER_BLOCK * PER_BLOCK) * BLOCK;
/// The most links a folder can have without dir_nlink.
const LINK_MAX: u64 = 65000;
/// The smallest last group: its metadata and this many data blocks.
const MIN_LAST_GROUP_DATA: u64 = 32;
/// One inode for every 16 KiB when the size is given (as mke2fs does).
const BYTES_PER_INODE: u64 = 16384;

const COMPAT_EXT_ATTR: u32 = 0x8;
const INCOMPAT_FILETYPE: u32 = 0x2;
const RO_COMPAT_SPARSE_SUPER: u32 = 0x1;
const RO_COMPAT_LARGE_FILE: u32 = 0x2;

/// A node's index in a [`Tree`].
pub type NodeId = usize;

/// The tree's root folder.
pub const ROOT: NodeId = 0;

/// Owner, permissions, times and extended attributes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Meta {
    /// Permission bits, with setuid, setgid and sticky (0o7777 at most).
    pub perm: u16,
    pub uid: u32,
    pub gid: u32,
    pub atime: u32,
    pub ctime: u32,
    pub mtime: u32,
    /// Extended attributes by full name ("security.capability"), values
    /// as stored on disk (POSIX ACLs in ext4's own format, as
    /// [`crate::ext4::Ext4::xattrs`] reads them).
    pub xattrs: Vec<(String, Vec<u8>)>,
}

impl Meta {
    /// `perm`, owned by `uid`:`gid`, with all three times `time`.
    pub fn new(perm: u16, uid: u32, gid: u32, time: u32) -> Meta {
        Meta {
            perm,
            uid,
            gid,
            atime: time,
            ctime: time,
            mtime: time,
            xattrs: Vec::new(),
        }
    }
}

/// A closure that writes a file's bytes.
pub type Source<'a> = Box<dyn Fn(&mut dyn Write) -> io::Result<()> + 'a>;

/// A regular file's bytes.
pub enum Data<'a> {
    Bytes(Vec<u8>),
    /// `len` bytes, written by `write` when the image is written. It must
    /// write exactly `len` bytes, or the image write fails.
    Lazy {
        len: u64,
        write: Source<'a>,
    },
}

impl Data<'_> {
    pub fn len(&self) -> u64 {
        match self {
            Data::Bytes(b) => b.len() as u64,
            Data::Lazy { len, .. } => *len,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// What a node is.
pub enum Body<'a> {
    /// A folder: names to nodes.
    Dir(BTreeMap<Vec<u8>, NodeId>),
    File(Data<'a>),
    /// A symbolic link to this target.
    Symlink(Vec<u8>),
    /// A character device (major, minor).
    CharDev(u32, u32),
    /// A block device (major, minor).
    BlockDev(u32, u32),
    Fifo,
    Socket,
}

impl Body<'_> {
    fn file_type(&self) -> u8 {
        match self {
            Body::File(_) => 1,
            Body::Dir(_) => 2,
            Body::CharDev(..) => 3,
            Body::BlockDev(..) => 4,
            Body::Fifo => 5,
            Body::Socket => 6,
            Body::Symlink(_) => 7,
        }
    }

    fn mode_type(&self) -> u16 {
        use crate::ext4::{S_IFBLK, S_IFCHR, S_IFDIR, S_IFIFO, S_IFLNK, S_IFREG, S_IFSOCK};
        match self {
            Body::File(_) => S_IFREG,
            Body::Dir(_) => S_IFDIR,
            Body::CharDev(..) => S_IFCHR,
            Body::BlockDev(..) => S_IFBLK,
            Body::Fifo => S_IFIFO,
            Body::Socket => S_IFSOCK,
            Body::Symlink(_) => S_IFLNK,
        }
    }
}

/// One file, folder, link or device.
pub struct Node<'a> {
    pub body: Body<'a>,
    pub meta: Meta,
    /// A folder's parent (the root's is itself); None for anything else.
    parent: Option<NodeId>,
}

/// A tree of files to write. Folders have one parent each; files, links
/// and devices can have several names (hard links).
pub struct Tree<'a> {
    nodes: Vec<Node<'a>>,
}

fn check_name(name: &[u8]) -> io::Result<()> {
    if name.is_empty()
        || name.len() > 255
        || name == b"."
        || name == b".."
        || name.contains(&b'/')
        || name.contains(&0)
    {
        return Err(bad(format!(
            "{:?} is not a usable file name",
            String::from_utf8_lossy(name)
        )));
    }
    Ok(())
}

impl<'a> Tree<'a> {
    /// An empty tree whose root folder has `root`'s owner and permissions.
    pub fn new(root: Meta) -> Tree<'a> {
        Tree {
            nodes: vec![Node {
                body: Body::Dir(BTreeMap::new()),
                meta: root,
                parent: Some(ROOT),
            }],
        }
    }

    pub fn node(&self, id: NodeId) -> &Node<'a> {
        &self.nodes[id]
    }

    /// A node's owner, permissions, times and attributes, to change.
    pub fn meta_mut(&mut self, id: NodeId) -> &mut Meta {
        &mut self.nodes[id].meta
    }

    /// Replaces a file's bytes.
    pub fn set_data(&mut self, id: NodeId, data: Data<'a>) -> io::Result<()> {
        match &mut self.nodes[id].body {
            Body::File(d) => {
                *d = data;
                Ok(())
            }
            _ => Err(bad("not a regular file")),
        }
    }

    fn dir(&self, id: NodeId) -> io::Result<&BTreeMap<Vec<u8>, NodeId>> {
        match &self.nodes[id].body {
            Body::Dir(m) => Ok(m),
            _ => Err(bad("not a folder")),
        }
    }

    fn dir_mut(&mut self, id: NodeId) -> io::Result<&mut BTreeMap<Vec<u8>, NodeId>> {
        match &mut self.nodes[id].body {
            Body::Dir(m) => Ok(m),
            _ => Err(bad("not a folder")),
        }
    }

    /// The entry `name` in folder `dir`.
    pub fn child(&self, dir: NodeId, name: &[u8]) -> Option<NodeId> {
        self.dir(dir).ok()?.get(name).copied()
    }

    /// A folder's entries, in name order.
    pub fn children(&self, dir: NodeId) -> Vec<(Vec<u8>, NodeId)> {
        match self.dir(dir) {
            Ok(m) => m.iter().map(|(k, v)| (k.clone(), *v)).collect(),
            Err(_) => Vec::new(),
        }
    }

    /// The node at an absolute path ("/etc/passwd"), without following links.
    pub fn lookup(&self, path: &str) -> Option<NodeId> {
        let mut cur = ROOT;
        for part in path.split('/').filter(|p| !p.is_empty()) {
            cur = self.child(cur, part.as_bytes())?;
        }
        Some(cur)
    }

    /// Adds a new node as `name` in folder `dir`. The name must be free.
    pub fn add(
        &mut self,
        dir: NodeId,
        name: &[u8],
        body: Body<'a>,
        meta: Meta,
    ) -> io::Result<NodeId> {
        check_name(name)?;
        if meta.perm & !0o7777 != 0 {
            return Err(bad(format!("permissions {:o} have type bits", meta.perm)));
        }
        if self.dir(dir)?.contains_key(name) {
            return Err(bad(format!(
                "{} already exists",
                String::from_utf8_lossy(name)
            )));
        }
        let id = self.nodes.len();
        let parent = matches!(body, Body::Dir(_)).then_some(dir);
        self.nodes.push(Node { body, meta, parent });
        self.dir_mut(dir)?.insert(name.to_vec(), id);
        Ok(id)
    }

    /// Adds another name for a file, link or device (a hard link).
    pub fn link(&mut self, dir: NodeId, name: &[u8], target: NodeId) -> io::Result<()> {
        check_name(name)?;
        if matches!(self.nodes[target].body, Body::Dir(_)) {
            return Err(bad("folders cannot be hard-linked"));
        }
        if self.dir(dir)?.contains_key(name) {
            return Err(bad(format!(
                "{} already exists",
                String::from_utf8_lossy(name)
            )));
        }
        self.dir_mut(dir)?.insert(name.to_vec(), target);
        Ok(())
    }

    /// Removes the entry `name` from folder `dir`; a folder's whole subtree goes with it.
    pub fn unlink(&mut self, dir: NodeId, name: &[u8]) -> Option<NodeId> {
        self.dir_mut(dir).ok()?.remove(name)
    }

    fn split(path: &str) -> io::Result<(&str, &str)> {
        let path = path.trim_end_matches('/');
        let (parent, name) = path
            .rsplit_once('/')
            .ok_or_else(|| bad(format!("{path} is not an absolute path")))?;
        if name.is_empty() {
            return Err(bad(format!("{path} names no entry")));
        }
        Ok((parent, name))
    }

    /// Puts a node at an absolute path whose folder exists, replacing what
    /// was there unless that is a folder.
    pub fn put(&mut self, path: &str, body: Body<'a>, meta: Meta) -> io::Result<NodeId> {
        let (parent, name) = Self::split(path)?;
        let dir = self
            .lookup(parent)
            .ok_or_else(|| bad(format!("{parent} does not exist")))?;
        if let Some(old) = self.child(dir, name.as_bytes()) {
            if matches!(self.nodes[old].body, Body::Dir(_)) {
                return Err(bad(format!("{path} is a folder; refusing to replace it")));
            }
            self.unlink(dir, name.as_bytes());
        }
        self.add(dir, name.as_bytes(), body, meta)
    }

    /// Removes the entry at an absolute path.
    pub fn remove(&mut self, path: &str) -> Option<NodeId> {
        let (parent, name) = Self::split(path).ok()?;
        let dir = self.lookup(parent)?;
        self.unlink(dir, name.as_bytes())
    }

    /// The folder at an absolute path, made (with `meta`) where missing.
    pub fn mkdir_all(&mut self, path: &str, meta: Meta) -> io::Result<NodeId> {
        let mut cur = ROOT;
        for part in path.split('/').filter(|p| !p.is_empty()) {
            cur = match self.child(cur, part.as_bytes()) {
                Some(id) if matches!(self.nodes[id].body, Body::Dir(_)) => id,
                Some(_) => return Err(bad(format!("{part} in {path} is not a folder"))),
                None => self.add(
                    cur,
                    part.as_bytes(),
                    Body::Dir(BTreeMap::new()),
                    meta.clone(),
                )?,
            };
        }
        Ok(cur)
    }
}

/// How big to make the filesystem.
#[derive(Clone, Copy, Debug)]
pub enum Size {
    /// Just big enough for the tree, plus this much room.
    Fit { spare_bytes: u64, spare_inodes: u32 },
    /// This many bytes (rounded down to whole blocks; a last block group
    /// too small to hold anything is left out).
    Bytes(u64),
}

/// How to make the filesystem.
#[derive(Clone, Debug)]
pub struct Options {
    /// The volume label, at most 16 bytes.
    pub label: String,
    /// The filesystem UUID (all zeros: a random one).
    pub uuid: [u8; 16],
    /// The creation time written into the superblock.
    pub time: u32,
    pub size: Size,
}

/// What [`plan`] and [`write`] make.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Summary {
    /// The filesystem's length (blocks × 4096).
    pub bytes: u64,
    pub blocks: u64,
    pub free_blocks: u64,
    pub inodes: u32,
    pub free_inodes: u32,
    pub uuid: [u8; 16],
}

/// One inode to write.
struct Ino {
    node: Option<NodeId>,
    links: u16,
    /// Data blocks (file bytes, folder blocks, a slow link's target).
    data: Vec<u64>,
    /// Indirect blocks with the block numbers each holds.
    maps: Vec<(u64, Vec<u32>)>,
    i_block: [u8; 60],
    size: u64,
    ea: Option<usize>,
    is_dir: bool,
}

struct Geo {
    blocks: u64,
    groups: u64,
    ipg: u64,
    gdt_blocks: u64,
}

impl Geo {
    fn itb(&self) -> u64 {
        self.ipg / INODES_PER_BLOCK
    }

    fn start(&self, g: u64) -> u64 {
        g * BLOCKS_PER_GROUP
    }

    fn len(&self, g: u64) -> u64 {
        (self.blocks - self.start(g)).min(BLOCKS_PER_GROUP)
    }

    fn sb_blocks(&self, g: u64) -> u64 {
        if has_super(g, true) {
            1 + self.gdt_blocks
        } else {
            0
        }
    }

    fn overhead(&self, g: u64) -> u64 {
        self.sb_blocks(g) + 2 + self.itb()
    }

    fn total_overhead(&self) -> u64 {
        (0..self.groups).map(|g| self.overhead(g)).sum()
    }

    fn inodes(&self) -> u64 {
        self.groups * self.ipg
    }
}

fn gdt_blocks(groups: u64) -> u64 {
    (groups * 32).div_ceil(BLOCK)
}

fn ipg_for(inodes: u64, groups: u64) -> u64 {
    inodes.div_ceil(groups).div_ceil(INODES_PER_BLOCK).max(1) * INODES_PER_BLOCK
}

fn geometry(data_blocks: u64, inodes_needed: u64, size: Size) -> io::Result<Geo> {
    let too_big = || bad("the filesystem would be too big for 32-bit block numbers");
    match size {
        Size::Fit {
            spare_bytes,
            spare_inodes,
        } => {
            let spare = spare_bytes.div_ceil(BLOCK);
            let inodes = inodes_needed + spare_inodes as u64;
            let mut groups = inodes.div_ceil(MAX_IPG).max(1);
            loop {
                let ipg = ipg_for(inodes, groups);
                let mut geo = Geo {
                    blocks: 0,
                    groups,
                    ipg,
                    gdt_blocks: gdt_blocks(groups),
                };
                let need = geo.total_overhead() + data_blocks + spare;
                let need_groups = need.div_ceil(BLOCKS_PER_GROUP);
                if need_groups > groups {
                    groups = need_groups;
                    continue;
                }
                let last_min =
                    geo.start(groups - 1) + geo.overhead(groups - 1) + MIN_LAST_GROUP_DATA;
                geo.blocks = need.max(last_min);
                if geo.blocks > groups * BLOCKS_PER_GROUP {
                    groups += 1;
                    continue;
                }
                if geo.blocks > u32::MAX as u64 {
                    return Err(too_big());
                }
                return Ok(geo);
            }
        }
        Size::Bytes(bytes) => {
            let mut blocks = (bytes / BLOCK).min(u32::MAX as u64);
            let mut groups = blocks.div_ceil(BLOCKS_PER_GROUP).max(1);
            let inodes_for =
                |blocks: u64| (blocks * BLOCK / BYTES_PER_INODE).max(inodes_needed + 16);
            let mut geo;
            loop {
                let ipg = ipg_for(inodes_for(blocks), groups).min(MAX_IPG);
                geo = Geo {
                    blocks,
                    groups,
                    ipg,
                    gdt_blocks: gdt_blocks(groups),
                };
                let last = groups - 1;
                if groups > 1 && blocks - geo.start(last) < geo.overhead(last) + MIN_LAST_GROUP_DATA
                {
                    blocks = geo.start(last);
                    groups -= 1;
                    continue;
                }
                break;
            }
            if geo.inodes() < inodes_needed {
                return Err(bad(format!(
                    "{bytes} bytes cannot hold {inodes_needed} inodes"
                )));
            }
            if geo.blocks < geo.overhead(0) + MIN_LAST_GROUP_DATA
                || geo.total_overhead() + data_blocks > geo.blocks
            {
                return Err(bad(format!(
                    "the files need {} blocks of 4 KiB; {bytes} bytes hold {}",
                    geo.total_overhead() + data_blocks,
                    geo.blocks
                )));
            }
            Ok(geo)
        }
    }
}

/// Blocks a file of `n` data blocks needs for its block map.
fn map_blocks(n: u64) -> u64 {
    let mut m = 0;
    if n > DIRECT {
        m += 1;
    }
    if n > DIRECT + PER_BLOCK {
        m += 1 + (n - DIRECT - PER_BLOCK).div_ceil(PER_BLOCK);
    }
    m
}

fn entry_len(name_len: usize) -> usize {
    (8 + name_len + 3) & !3
}

/// Folder entries laid into blocks: (name, inode, file type) per block.
fn dir_layout(
    entries: &[(Vec<u8>, u32, u8)],
    me: u32,
    parent: u32,
) -> Vec<Vec<(Vec<u8>, u32, u8)>> {
    let mut blocks = vec![vec![
        (b".".to_vec(), me, 2u8),
        (b"..".to_vec(), parent, 2u8),
    ]];
    let mut used = 24usize;
    for e in entries {
        let len = entry_len(e.0.len());
        if used + len > BLOCK as usize {
            blocks.push(Vec::new());
            used = 0;
        }
        blocks.last_mut().unwrap().push(e.clone());
        used += len;
    }
    blocks
}

fn render_dir_block(entries: &[(Vec<u8>, u32, u8)]) -> Vec<u8> {
    let mut b = vec![0u8; BLOCK as usize];
    if entries.is_empty() {
        put16(&mut b, 4, BLOCK as u16);
        return b;
    }
    let mut at = 0usize;
    for (i, (name, ino, ft)) in entries.iter().enumerate() {
        let len = if i + 1 == entries.len() {
            BLOCK as usize - at
        } else {
            entry_len(name.len())
        };
        put32(&mut b, at, *ino);
        put16(&mut b, at + 4, len as u16);
        b[at + 6] = name.len() as u8;
        b[at + 7] = *ft;
        b[at + 8..at + 8 + name.len()].copy_from_slice(name);
        at += len;
    }
    b
}

fn split_xattr_name(name: &str) -> io::Result<(u8, &str)> {
    match name {
        "system.posix_acl_access" => return Ok((2, "")),
        "system.posix_acl_default" => return Ok((3, "")),
        "system.richacl" => return Ok((8, "")),
        _ => {}
    }
    for (prefix, index) in [
        ("user.", 1u8),
        ("trusted.", 4),
        ("security.", 6),
        ("system.", 7),
    ] {
        if let Some(rest) = name.strip_prefix(prefix) {
            if rest.is_empty() || rest.len() > 255 {
                break;
            }
            return Ok((index, rest));
        }
    }
    Err(bad(format!(
        "extended attribute {name:?} has no known prefix"
    )))
}

fn xattr_entry_hash(name: &[u8], value: &[u8]) -> u32 {
    let mut hash = 0u32;
    for &c in name {
        hash = (hash << 5) ^ (hash >> 27) ^ (c as i8 as i32 as u32);
    }
    let mut padded = value.to_vec();
    padded.resize(value.len().div_ceil(4) * 4, 0);
    for w in padded.chunks(4) {
        hash = (hash << 16) ^ (hash >> 16) ^ u32::from_le_bytes([w[0], w[1], w[2], w[3]]);
    }
    hash
}

/// An extended attribute block (refcount left at 0).
fn ea_block(xattrs: &[(String, Vec<u8>)]) -> io::Result<Vec<u8>> {
    let mut items = Vec::new();
    for (name, value) in xattrs {
        let (index, rest) = split_xattr_name(name)?;
        items.push((index, rest.as_bytes(), value.as_slice()));
    }
    items.sort_by(|a, b| (a.0, a.1.len(), a.1).cmp(&(b.0, b.1.len(), b.1)));
    for w in items.windows(2) {
        if (w[0].0, w[0].1) == (w[1].0, w[1].1) {
            return Err(bad("an extended attribute is named twice"));
        }
    }
    let mut b = vec![0u8; BLOCK as usize];
    put32(&mut b, 0, 0xEA02_0000);
    put32(&mut b, 8, 1);
    let mut at = 32usize;
    let mut value_end = BLOCK as usize;
    let mut block_hash = 0u32;
    for (index, name, value) in &items {
        let size = value.len().div_ceil(4) * 4;
        let entry = (16 + name.len() + 3) & !3;
        if value_end < size || at + entry + 4 > value_end - size {
            return Err(bad("the extended attributes do not fit in one block"));
        }
        value_end -= size;
        b[value_end..value_end + value.len()].copy_from_slice(value);
        let hash = xattr_entry_hash(name, value);
        b[at] = name.len() as u8;
        b[at + 1] = *index;
        put16(
            &mut b,
            at + 2,
            if value.is_empty() {
                0
            } else {
                value_end as u16
            },
        );
        put32(&mut b, at + 4, 0);
        put32(&mut b, at + 8, value.len() as u32);
        put32(&mut b, at + 12, hash);
        b[at + 16..at + 16 + name.len()].copy_from_slice(name);
        at += entry;
        block_hash = (block_hash << 16) ^ (block_hash >> 16) ^ hash;
    }
    put32(&mut b, 16, block_hash);
    Ok(b)
}

fn rdev_block(major: u32, minor: u32) -> io::Result<[u8; 60]> {
    let mut b = [0u8; 60];
    if major < 256 && minor < 256 {
        put32(&mut b, 0, (major << 8) | minor);
    } else if major < 4096 && minor < (1 << 20) {
        put32(
            &mut b,
            4,
            (minor & 0xFF) | (major << 8) | ((minor & !0xFF) << 12),
        );
    } else {
        return Err(bad(format!("device {major}:{minor} does not fit")));
    }
    Ok(b)
}

/// A folder's entries, block by block: (name, inode, file type).
type DirBlocks = Vec<Vec<(Vec<u8>, u32, u8)>>;

/// Everything about the image except the bytes of its files.
struct Layout {
    geo: Geo,
    inos: Vec<Ino>,                // index = inode number - 1
    eas: Vec<(Vec<u8>, u64, u32)>, // block bytes, block number, refcount
    dirs: HashMap<u32, DirBlocks>,
    used_data: Vec<u64>, // data blocks used in each group
    uuid: [u8; 16],
}

fn prepare(tree: &Tree, opts: &Options) -> io::Result<Layout> {
    if opts.label.len() > 16 {
        return Err(bad(format!(
            "label {:?} is longer than 16 bytes",
            opts.label
        )));
    }
    if tree.child(ROOT, b"lost+found").is_some() {
        return Err(bad(
            "the tree has its own lost+found; leave it out, it is made here",
        ));
    }
    // Inode numbers, depth first in name order.
    let mut ino_of: Vec<Option<u32>> = vec![None; tree.nodes.len()];
    let mut order: Vec<NodeId> = Vec::new();
    ino_of[ROOT] = Some(2);
    let mut next = FIRST_INO + 1;
    let mut refs: Vec<u64> = vec![0; tree.nodes.len()];
    let mut stack = vec![ROOT];
    while let Some(dir) = stack.pop() {
        let children = tree.dir(dir)?;
        let mut subdirs = Vec::new();
        for &id in children.values() {
            refs[id] += 1;
            if ino_of[id].is_none() {
                ino_of[id] = Some(next);
                next = next.checked_add(1).ok_or_else(|| bad("too many files"))?;
                order.push(id);
            }
            if matches!(tree.nodes[id].body, Body::Dir(_)) {
                subdirs.push(id);
            }
        }
        stack.extend(subdirs.into_iter().rev());
    }
    let max_ino = next - 1;

    let mut inos: Vec<Ino> = (0..max_ino)
        .map(|_| Ino {
            node: None,
            links: 0,
            data: Vec::new(),
            maps: Vec::new(),
            i_block: [0; 60],
            size: 0,
            ea: None,
            is_dir: false,
        })
        .collect();
    let mut dirs = HashMap::new();
    let mut ea_index: HashMap<Vec<u8>, usize> = HashMap::new();
    let mut eas: Vec<(Vec<u8>, u64, u32)> = Vec::new();
    let mut data_blocks = LOST_FOUND_BLOCKS;
    // (inode, data blocks) in allocation order.
    let mut wants: Vec<(u32, u64)> = Vec::new();

    let mut nodes = vec![ROOT];
    nodes.extend(order.iter().copied());
    // lost+found sits between the root and the rest in allocation order.
    let lf = &mut inos[LOST_FOUND as usize - 1];
    lf.links = 2;
    lf.is_dir = true;
    lf.size = LOST_FOUND_BLOCKS * BLOCK;
    dirs.insert(LOST_FOUND, {
        let mut v = dir_layout(&[], LOST_FOUND, 2);
        v.resize(LOST_FOUND_BLOCKS as usize, Vec::new());
        v
    });
    for (pos, &id) in nodes.iter().enumerate() {
        let ino = ino_of[id].unwrap();
        let node = &tree.nodes[id];
        let mut n_data = 0u64;
        let rec = &mut inos[ino as usize - 1];
        rec.node = Some(id);
        match &node.body {
            Body::Dir(children) => {
                let mut entries: Vec<(Vec<u8>, u32, u8)> = children
                    .iter()
                    .map(|(name, &c)| {
                        (
                            name.clone(),
                            ino_of[c].unwrap(),
                            tree.nodes[c].body.file_type(),
                        )
                    })
                    .collect();
                let mut subdirs = children
                    .values()
                    .filter(|&&c| matches!(tree.nodes[c].body, Body::Dir(_)))
                    .count() as u64;
                if id == ROOT {
                    entries.push((b"lost+found".to_vec(), LOST_FOUND, 2));
                    entries.sort();
                    subdirs += 1;
                }
                if 2 + subdirs > LINK_MAX {
                    return Err(bad("a folder has too many subfolders"));
                }
                let parent = ino_of[node.parent.unwrap()].unwrap();
                let layout = dir_layout(&entries, ino, parent);
                n_data = layout.len() as u64;
                rec.links = (2 + subdirs) as u16;
                rec.is_dir = true;
                rec.size = n_data * BLOCK;
                dirs.insert(ino, layout);
            }
            Body::File(d) => {
                if d.len() > MAX_FILE {
                    return Err(bad(format!(
                        "a file of {} bytes is over the {MAX_FILE}-byte limit",
                        d.len()
                    )));
                }
                n_data = d.len().div_ceil(BLOCK);
                rec.size = d.len();
            }
            Body::Symlink(target) => {
                if target.is_empty() || target.len() >= BLOCK as usize {
                    return Err(bad("a link target must be 1 to 4095 bytes"));
                }
                rec.size = target.len() as u64;
                if target.len() < 60 {
                    rec.i_block[..target.len()].copy_from_slice(target);
                } else {
                    n_data = 1;
                }
            }
            Body::CharDev(ma, mi) | Body::BlockDev(ma, mi) => rec.i_block = rdev_block(*ma, *mi)?,
            Body::Fifo | Body::Socket => {}
        }
        if !matches!(node.body, Body::Dir(_)) {
            if refs[id] > LINK_MAX {
                return Err(bad("a file has too many hard links"));
            }
            rec.links = refs[id] as u16;
        }
        if !node.meta.xattrs.is_empty() {
            let block = ea_block(&node.meta.xattrs)?;
            let idx = *ea_index.entry(block.clone()).or_insert_with(|| {
                eas.push((block, 0, 0));
                eas.len() - 1
            });
            eas[idx].2 += 1;
            rec.ea = Some(idx);
        }
        data_blocks += n_data + map_blocks(n_data);
        wants.push((ino, n_data));
        if pos == 0 {
            wants.push((LOST_FOUND, LOST_FOUND_BLOCKS));
        }
    }
    data_blocks += eas.len() as u64;

    let geo = geometry(data_blocks, max_ino as u64, opts.size)?;

    // Allocate data blocks in order, group by group past each group's metadata.
    let mut used_data = vec![0u64; geo.groups as usize];
    let mut g = 0u64;
    let mut cursor = geo.start(0) + geo.overhead(0);
    let mut alloc = || -> io::Result<u64> {
        while cursor >= geo.start(g) + geo.len(g) {
            g += 1;
            if g >= geo.groups {
                return Err(bad("ran out of blocks (a sizing bug)"));
            }
            cursor = geo.start(g) + geo.overhead(g);
        }
        used_data[g as usize] += 1;
        cursor += 1;
        Ok(cursor - 1)
    };
    for (ino, n) in wants {
        let rec = &mut inos[ino as usize - 1];
        let mut data = Vec::with_capacity(n as usize);
        let mut maps: Vec<(u64, Vec<u32>)> = Vec::new();
        let mut ind: Option<usize> = None; // the indirect block being filled
        let mut dind: Option<usize> = None;
        for i in 0..n {
            if i == DIRECT {
                maps.push((alloc()?, Vec::new()));
                ind = Some(maps.len() - 1);
            } else if i == DIRECT + PER_BLOCK {
                maps.push((alloc()?, Vec::new()));
                dind = Some(maps.len() - 1);
            }
            if i >= DIRECT + PER_BLOCK && (i - DIRECT - PER_BLOCK).is_multiple_of(PER_BLOCK) {
                let b = alloc()?;
                maps.push((b, Vec::new()));
                maps[dind.unwrap()].1.push(b as u32);
                ind = Some(maps.len() - 1);
            }
            let b = alloc()?;
            data.push(b);
            if i < DIRECT {
                put32(&mut rec.i_block, i as usize * 4, b as u32);
            } else {
                maps[ind.unwrap()].1.push(b as u32);
            }
        }
        if let Some(i) = maps.first() {
            put32(&mut rec.i_block, 48, i.0 as u32);
        }
        if let Some(d) = dind {
            put32(&mut rec.i_block, 52, maps[d].0 as u32);
        }
        rec.data = data;
        rec.maps = maps;
    }
    for ea in eas.iter_mut() {
        ea.1 = alloc()?;
        put32(&mut ea.0, 4, ea.2);
    }
    let uuid = if opts.uuid == [0; 16] {
        crate::gpt::random16()
    } else {
        opts.uuid
    };
    Ok(Layout {
        geo,
        inos,
        eas,
        dirs,
        used_data,
        uuid,
    })
}

fn summary(l: &Layout) -> Summary {
    let geo = &l.geo;
    let used: u64 = geo.total_overhead() + l.used_data.iter().sum::<u64>();
    Summary {
        bytes: geo.blocks * BLOCK,
        blocks: geo.blocks,
        free_blocks: geo.blocks - used,
        inodes: geo.inodes() as u32,
        free_inodes: (geo.inodes() - l.inos.len() as u64) as u32,
        uuid: l.uuid,
    }
}

/// The size and counts [`write`] would give, without writing.
pub fn plan(tree: &Tree, opts: &Options) -> io::Result<Summary> {
    let mut opts = opts.clone();
    if opts.uuid == [0; 16] {
        opts.uuid = [1; 16];
    }
    let mut s = summary(&prepare(tree, &opts)?);
    s.uuid = [0; 16];
    Ok(s)
}

/// Writes the filesystem to `w` from its byte 0. The caller makes sure
/// `w` has room for [`Summary::bytes`] (from [`plan`]).
pub fn write<W: WriteAt + ?Sized>(tree: &Tree, opts: &Options, w: &W) -> io::Result<Summary> {
    let l = prepare(tree, opts)?;
    let geo = &l.geo;
    let sum = summary(&l);
    let has_ea = !l.eas.is_empty();

    // Group descriptors.
    let ipg = geo.ipg;
    let max_ino = l.inos.len() as u64;
    let mut gdt = vec![0u8; (geo.gdt_blocks * BLOCK) as usize];
    for g in 0..geo.groups {
        let base = geo.start(g) + geo.sb_blocks(g);
        let used_inodes = max_ino.saturating_sub(g * ipg).min(ipg);
        let dirs = (g * ipg..(g * ipg + used_inodes))
            .filter(|&i| l.inos[i as usize].is_dir)
            .count();
        let free_blocks = geo.len(g) - geo.overhead(g) - l.used_data[g as usize];
        let d = &mut gdt[(g * 32) as usize..][..32];
        put32(d, 0, base as u32);
        put32(d, 4, (base + 1) as u32);
        put32(d, 8, (base + 2) as u32);
        put16(d, 12, free_blocks as u16);
        put16(d, 14, (ipg - used_inodes) as u16);
        put16(d, 16, dirs as u16);
    }

    // Boot sector area.
    w.write_at(&[0u8; 1024], 0)?;
    for g in 0..geo.groups {
        let start = geo.start(g);
        if has_super(g, true) {
            let sb = superblock(&l, &sum, opts, g, has_ea);
            if g == 0 {
                w.write_at(&sb, 1024)?;
                w.write_at(&[0u8; 2048], 2048)?;
            } else {
                let mut block = vec![0u8; BLOCK as usize];
                block[..1024].copy_from_slice(&sb);
                w.write_at(&block, start * BLOCK)?;
            }
            w.write_at(&gdt, (start + 1) * BLOCK)?;
        }
        let base = start + geo.sb_blocks(g);
        // Block bitmap: metadata, then the used data blocks, then padding past the group's end.
        let mut bb = vec![0u8; BLOCK as usize];
        let used = geo.overhead(g) + l.used_data[g as usize];
        set_bits(&mut bb, 0, used);
        set_bits(&mut bb, geo.len(g), BLOCKS_PER_GROUP);
        w.write_at(&bb, base * BLOCK)?;
        // Inode bitmap.
        let mut ib = vec![0u8; BLOCK as usize];
        set_bits(&mut ib, 0, max_ino.saturating_sub(g * ipg).min(ipg));
        set_bits(&mut ib, ipg, BLOCK * 8);
        w.write_at(&ib, (base + 1) * BLOCK)?;
        // Inode table.
        let mut table = vec![0u8; (ipg as usize) * INODE_SIZE];
        for i in 0..ipg {
            let ino = g * ipg + i + 1;
            if ino > max_ino {
                break;
            }
            let rec = &l.inos[ino as usize - 1];
            if ino != LOST_FOUND as u64 && rec.node.is_none() {
                continue; // reserved, left zero
            }
            encode_inode(
                &mut table[(i as usize) * INODE_SIZE..][..INODE_SIZE],
                tree,
                &l,
                ino as u32,
                opts.time,
            );
        }
        w.write_at(&table, (base + 2) * BLOCK)?;
    }

    // Folders, links, block maps and attribute blocks.
    for (ino, blocks) in &l.dirs {
        let rec = &l.inos[*ino as usize - 1];
        for (entries, &b) in blocks.iter().zip(&rec.data) {
            w.write_at(&render_dir_block(entries), b * BLOCK)?;
        }
    }
    for rec in &l.inos {
        for (b, list) in &rec.maps {
            let mut block = vec![0u8; BLOCK as usize];
            for (i, n) in list.iter().enumerate() {
                put32(&mut block, i * 4, *n);
            }
            w.write_at(&block, b * BLOCK)?;
        }
        let Some(id) = rec.node else { continue };
        match &tree.nodes[id].body {
            Body::Symlink(t) if t.len() >= 60 => {
                let mut block = vec![0u8; BLOCK as usize];
                block[..t.len()].copy_from_slice(t);
                w.write_at(&block, rec.data[0] * BLOCK)?;
            }
            Body::File(data) => {
                let mut out = FileOut {
                    w,
                    blocks: &rec.data,
                    next: 0,
                    buf: Vec::new(),
                    written: 0,
                };
                match data {
                    Data::Bytes(b) => out.write_all(b)?,
                    Data::Lazy { write, .. } => write(&mut out)?,
                }
                let n = out.finish()?;
                if n != data.len() {
                    return Err(bad(format!(
                        "a file gave {n} bytes where {} were promised",
                        data.len()
                    )));
                }
            }
            _ => {}
        }
    }
    for (block, b, _) in &l.eas {
        w.write_at(block, b * BLOCK)?;
    }
    Ok(sum)
}

fn set_bits(map: &mut [u8], from: u64, to: u64) {
    for bit in from..to.min(map.len() as u64 * 8) {
        map[(bit / 8) as usize] |= 1 << (bit % 8);
    }
}

fn superblock(l: &Layout, s: &Summary, opts: &Options, group: u64, has_ea: bool) -> Vec<u8> {
    let geo = &l.geo;
    let mut sb = vec![0u8; 1024];
    put32(&mut sb, 0x00, s.inodes);
    put32(&mut sb, 0x04, geo.blocks as u32);
    put32(&mut sb, 0x08, 0);
    put32(&mut sb, 0x0C, s.free_blocks as u32);
    put32(&mut sb, 0x10, s.free_inodes);
    put32(&mut sb, 0x14, 0);
    put32(&mut sb, 0x18, 2);
    put32(&mut sb, 0x1C, 2);
    put32(&mut sb, 0x20, BLOCKS_PER_GROUP as u32);
    put32(&mut sb, 0x24, BLOCKS_PER_GROUP as u32);
    put32(&mut sb, 0x28, geo.ipg as u32);
    put32(&mut sb, 0x2C, 0);
    put32(&mut sb, 0x30, opts.time);
    put16(&mut sb, 0x34, 0);
    put16(&mut sb, 0x36, 0xFFFF);
    put16(&mut sb, 0x38, 0xEF53);
    put16(&mut sb, 0x3A, 1); // clean
    put16(&mut sb, 0x3C, 1); // continue on errors
    put32(&mut sb, 0x40, opts.time);
    put32(&mut sb, 0x48, 0); // Linux
    put32(&mut sb, 0x4C, 1);
    put32(&mut sb, 0x54, FIRST_INO);
    put16(&mut sb, 0x58, INODE_SIZE as u16);
    put16(&mut sb, 0x5A, group as u16);
    put32(&mut sb, 0x5C, if has_ea { COMPAT_EXT_ATTR } else { 0 });
    put32(&mut sb, 0x60, INCOMPAT_FILETYPE);
    put32(&mut sb, 0x64, RO_COMPAT_SPARSE_SUPER | RO_COMPAT_LARGE_FILE);
    sb[0x68..0x78].copy_from_slice(&l.uuid);
    sb[0x78..0x78 + opts.label.len()].copy_from_slice(opts.label.as_bytes());
    // A hash seed for the day dir_index is turned on, from the UUID.
    let seed = crate::crc32(&l.uuid);
    for i in 0..4 {
        put32(
            &mut sb,
            0xEC + i * 4,
            seed.rotate_left(i as u32 * 8) ^ i as u32,
        );
    }
    sb[0xFC] = 1; // half_md4
    put32(&mut sb, 0x100, 0x000C); // user_xattr, acl
    put32(&mut sb, 0x108, opts.time);
    put16(&mut sb, 0x15C, EXTRA_ISIZE);
    put16(&mut sb, 0x15E, EXTRA_ISIZE);
    put32(&mut sb, 0x160, 0x1); // signed directory hash
    sb
}

fn encode_inode(b: &mut [u8], tree: &Tree, l: &Layout, ino: u32, time: u32) {
    let rec = &l.inos[ino as usize - 1];
    let (mode, meta) = match rec.node {
        Some(id) => {
            let n = &tree.nodes[id];
            (n.body.mode_type() | n.meta.perm, n.meta.clone())
        }
        None => (crate::ext4::S_IFDIR | 0o700, Meta::new(0o700, 0, 0, time)), // lost+found
    };
    let blocks = rec.data.len() as u64 + rec.maps.len() as u64 + rec.ea.is_some() as u64;
    let sectors = blocks * (BLOCK / 512);
    put16(b, 0x00, mode);
    put16(b, 0x02, meta.uid as u16);
    put32(b, 0x04, rec.size as u32);
    put32(b, 0x08, meta.atime);
    put32(b, 0x0C, meta.ctime);
    put32(b, 0x10, meta.mtime);
    put16(b, 0x18, meta.gid as u16);
    put16(b, 0x1A, rec.links);
    put32(b, 0x1C, sectors as u32);
    b[0x28..0x28 + 60].copy_from_slice(&rec.i_block);
    if let Some(ea) = rec.ea {
        put32(b, 0x68, l.eas[ea].1 as u32);
    }
    put32(b, 0x6C, (rec.size >> 32) as u32);
    put16(b, 0x74, (sectors >> 32) as u16);
    put16(b, 0x78, (meta.uid >> 16) as u16);
    put16(b, 0x7A, (meta.gid >> 16) as u16);
    put16(b, 0x80, EXTRA_ISIZE);
    put32(b, 0x90, meta.ctime); // creation time
}

/// Writes a file's bytes into its blocks, a megabyte at a time.
struct FileOut<'w, W: WriteAt + ?Sized> {
    w: &'w W,
    blocks: &'w [u64],
    /// The logical block `buf` starts at.
    next: usize,
    buf: Vec<u8>,
    written: u64,
}

const CHUNK: usize = 1 << 20;

impl<W: WriteAt + ?Sized> FileOut<'_, W> {
    /// Writes the whole blocks in `buf`, keeping a partial one.
    fn flush_blocks(&mut self, all: bool) -> io::Result<()> {
        if all && !self.buf.len().is_multiple_of(BLOCK as usize) {
            let len = self.buf.len().div_ceil(BLOCK as usize) * BLOCK as usize;
            self.buf.resize(len, 0);
        }
        let whole = self.buf.len() / BLOCK as usize;
        if self.next + whole > self.blocks.len() {
            return Err(bad("a file wrote more bytes than it promised"));
        }
        let mut i = 0;
        while i < whole {
            // A run of consecutive blocks goes in one write.
            let first = self.blocks[self.next + i];
            let mut run = 1;
            while i + run < whole && self.blocks[self.next + i + run] == first + run as u64 {
                run += 1;
            }
            let at = i * BLOCK as usize;
            self.w
                .write_at(&self.buf[at..at + run * BLOCK as usize], first * BLOCK)?;
            i += run;
        }
        self.buf.drain(..whole * BLOCK as usize);
        self.next += whole;
        Ok(())
    }

    fn finish(mut self) -> io::Result<u64> {
        self.flush_blocks(true)?;
        Ok(self.written)
    }
}

impl<W: WriteAt + ?Sized> Write for FileOut<'_, W> {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.buf.extend_from_slice(data);
        self.written += data.len() as u64;
        if self.buf.len() >= CHUNK {
            self.flush_blocks(false)?;
        }
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ext4::{Ext4, Kind};
    use crate::Mem;
    use std::process::Command;

    const T: u32 = 1_763_200_000;

    fn opts(size: Size) -> Options {
        Options {
            label: "TEST".into(),
            uuid: [7; 16],
            time: T,
            size,
        }
    }

    fn e2fsck() -> Option<String> {
        for p in ["/usr/sbin/e2fsck", "/sbin/e2fsck"] {
            if std::path::Path::new(p).exists() {
                return Some(p.to_string());
            }
        }
        if std::env::var("MPFREE_REQUIRE_OFFLINE_TOOLS").as_deref() == Ok("1") {
            panic!("e2fsck is missing and MPFREE_REQUIRE_OFFLINE_TOOLS=1");
        }
        None
    }

    /// Runs e2fsck -fn on an image; panics with its report on any problem.
    /// A scratch folder for one test, removed when dropped.
    pub(crate) struct Scratch(pub std::path::PathBuf);

    impl Scratch {
        pub(crate) fn new(tag: &str) -> Scratch {
            let dir =
                std::env::temp_dir().join(format!("mpfree-diskimage-{}-{tag}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            Scratch(dir)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).ok();
        }
    }

    /// Runs e2fsck -fn on an image file; panics with its report on any problem.
    fn fsck_file(path: &std::path::Path) {
        let Some(tool) = e2fsck() else {
            eprintln!("e2fsck not installed; skipping the e2fsck check");
            return;
        };
        let out = Command::new(tool).arg("-fn").arg(path).output().unwrap();
        assert!(
            out.status.success(),
            "e2fsck found problems:\n{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn fsck(bytes: &[u8]) {
        let dir = Scratch::new(&format!("{:x}", crate::crc32(bytes)));
        let path = dir.0.join("fs.img");
        std::fs::write(&path, bytes).unwrap();
        fsck_file(&path);
    }

    fn pattern(len: usize, seed: u8) -> Vec<u8> {
        (0..len)
            .map(|i| {
                (i as u32)
                    .wrapping_mul(2654435761)
                    .rotate_left(seed as u32 % 31) as u8
                    ^ seed
            })
            .collect()
    }

    fn sample_tree() -> Tree<'static> {
        let mut t = Tree::new(Meta::new(0o755, 0, 0, T));
        let etc = t
            .add(
                ROOT,
                b"etc",
                Body::Dir(BTreeMap::new()),
                Meta::new(0o755, 0, 0, T),
            )
            .unwrap();
        t.add(
            etc,
            b"passwd",
            Body::File(Data::Bytes(b"root:x:0:0::/:/bin/sh\n".to_vec())),
            Meta::new(0o644, 0, 0, T),
        )
        .unwrap();
        t.add(
            etc,
            b"empty",
            Body::File(Data::Bytes(Vec::new())),
            Meta::new(0o600, 1000, 1000, T),
        )
        .unwrap();
        let bin = t.mkdir_all("/usr/bin", Meta::new(0o755, 0, 0, T)).unwrap();
        // Direct, single indirect and double indirect sizes.
        for (name, len) in [
            ("twelve", 12 * 4096),
            ("thirteen", 12 * 4096 + 1),
            ("big", (12 + 1024 + 1500) * 4096 + 77),
        ] {
            let mut m = Meta::new(0o4755, 0, 0, T);
            if name == "big" {
                m.xattrs.push((
                    "security.capability".into(),
                    vec![
                        1, 0, 0, 2, 0x20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                    ],
                ));
            }
            t.add(
                bin,
                name.as_bytes(),
                Body::File(Data::Bytes(pattern(len, name.len() as u8))),
                m,
            )
            .unwrap();
        }
        let lazy_len = 3 * 4096 + 5;
        t.add(
            bin,
            b"lazy",
            Body::File(Data::Lazy {
                len: lazy_len as u64,
                write: Box::new(move |w: &mut dyn Write| w.write_all(&pattern(lazy_len, 9))),
            }),
            Meta::new(0o755, 70000, 70001, T),
        )
        .unwrap();
        let big = t.lookup("/usr/bin/big").unwrap();
        t.link(etc, b"big-again", big).unwrap();
        t.add(
            bin,
            b"sh",
            Body::Symlink(b"busybox".to_vec()),
            Meta::new(0o777, 0, 0, T),
        )
        .unwrap();
        let long = vec![b'x'; 300];
        t.add(
            bin,
            b"long-link",
            Body::Symlink(long),
            Meta::new(0o777, 0, 0, T),
        )
        .unwrap();
        let dev = t
            .add(
                ROOT,
                b"dev",
                Body::Dir(BTreeMap::new()),
                Meta::new(0o755, 0, 0, T),
            )
            .unwrap();
        t.add(dev, b"null", Body::CharDev(1, 3), Meta::new(0o666, 0, 0, T))
            .unwrap();
        t.add(
            dev,
            b"sdz",
            Body::BlockDev(259, 70000),
            Meta::new(0o660, 0, 6, T),
        )
        .unwrap();
        t.add(dev, b"fifo", Body::Fifo, Meta::new(0o644, 0, 0, T))
            .unwrap();
        t.add(dev, b"sock", Body::Socket, Meta::new(0o755, 0, 0, T))
            .unwrap();
        // Enough entries to need several folder blocks.
        let many = t
            .add(
                ROOT,
                b"many",
                Body::Dir(BTreeMap::new()),
                Meta::new(0o1777, 0, 0, T),
            )
            .unwrap();
        for i in 0..400 {
            let name = format!("entry-with-a-longish-name-{i:04}");
            let mut m = Meta::new(0o644, 0, 0, T);
            if i % 50 == 0 {
                m.xattrs
                    .push(("user.note".into(), format!("n{}", i % 100).into_bytes()));
            }
            t.add(
                many,
                name.as_bytes(),
                Body::File(Data::Bytes(vec![i as u8; i % 7])),
                m,
            )
            .unwrap();
        }
        t.mkdir_all("/many/sub/deeper", Meta::new(0o700, 5, 5, T))
            .unwrap();
        t
    }

    fn check_against(t: &Tree, fs: &Ext4<&Mem>) {
        let mut seen = 0usize;
        fs.walk(&mut |path, ino| {
            seen += 1;
            if path == "/lost+found" {
                assert!(ino.is_dir());
                assert_eq!(ino.perm(), 0o700);
                assert!(fs.read_dir(ino)?.is_empty());
                return Ok(());
            }
            let id = t
                .lookup(path)
                .unwrap_or_else(|| panic!("{path} is not in the tree"));
            let node = t.node(id);
            assert_eq!(ino.perm(), node.meta.perm, "{path}");
            assert_eq!((ino.uid, ino.gid), (node.meta.uid, node.meta.gid), "{path}");
            assert_eq!(ino.mtime, node.meta.mtime, "{path}");
            let mut want_x = node.meta.xattrs.clone();
            want_x.sort();
            let mut got_x = fs.xattrs(ino)?;
            got_x.sort();
            assert_eq!(got_x, want_x, "{path}");
            match &node.body {
                Body::Dir(m) => {
                    assert_eq!(ino.kind(), Some(Kind::Dir));
                    let mut names: Vec<Vec<u8>> =
                        fs.read_dir(ino)?.into_iter().map(|e| e.name).collect();
                    names.retain(|n| !(path == "/" && n == b"lost+found"));
                    names.sort();
                    assert_eq!(names, m.keys().cloned().collect::<Vec<_>>(), "{path}");
                }
                Body::File(d) => {
                    assert_eq!(ino.kind(), Some(Kind::File));
                    let got = fs.read_file(ino)?;
                    let want = match d {
                        Data::Bytes(b) => b.clone(),
                        Data::Lazy { write, .. } => {
                            let mut v = Vec::new();
                            write(&mut v)?;
                            v
                        }
                    };
                    assert!(got == want, "{path}: contents differ");
                }
                Body::Symlink(target) => assert_eq!(&fs.read_link(ino)?, target, "{path}"),
                Body::CharDev(ma, mi) => {
                    assert_eq!((ino.kind(), ino.rdev()), (Some(Kind::CharDev), (*ma, *mi)))
                }
                Body::BlockDev(ma, mi) => {
                    assert_eq!((ino.kind(), ino.rdev()), (Some(Kind::BlockDev), (*ma, *mi)))
                }
                Body::Fifo => assert_eq!(ino.kind(), Some(Kind::Fifo)),
                Body::Socket => assert_eq!(ino.kind(), Some(Kind::Socket)),
            }
            Ok(())
        })
        .unwrap();
        assert!(seen > 400);
    }

    #[test]
    fn fit_image_reads_back_and_checks_clean() {
        let t = sample_tree();
        let o = opts(Size::Fit {
            spare_bytes: 1 << 20,
            spare_inodes: 32,
        });
        let planned = plan(&t, &o).unwrap();
        let mem = Mem::new();
        let s = write(&t, &o, &mem).unwrap();
        assert_eq!(
            (s.bytes, s.blocks, s.free_blocks, s.inodes, s.free_inodes),
            (
                planned.bytes,
                planned.blocks,
                planned.free_blocks,
                planned.inodes,
                planned.free_inodes
            )
        );
        assert!(s.free_blocks >= 256 && s.free_inodes >= 32, "{s:?}");
        mem.set_len(s.bytes);
        let fs = Ext4::open(&mem).unwrap();
        assert_eq!(fs.label, "TEST");
        assert_eq!(fs.uuid, [7; 16]);
        assert!(fs.inode(11).unwrap().is_dir());
        check_against(&t, &fs);
        // The hard link is one inode with two links.
        let a = fs.lookup("/usr/bin/big").unwrap().unwrap();
        let b = fs.lookup("/etc/big-again").unwrap().unwrap();
        assert_eq!((a.ino, a.links), (b.ino, 2));
        fsck(&mem.bytes());
    }

    #[test]
    fn same_tree_same_image() {
        let a = Mem::new();
        let b = Mem::new();
        let o = opts(Size::Fit {
            spare_bytes: 0,
            spare_inodes: 0,
        });
        write(&sample_tree(), &o, &a).unwrap();
        write(&sample_tree(), &o, &b).unwrap();
        assert!(a.bytes() == b.bytes());
    }

    #[test]
    fn several_groups_and_a_given_size() {
        // 1200 MiB, in a sparse file: ten groups of 128 MiB, with superblock
        // backups in 0, 1, 3, 5, 7 and 9.
        let mut t = Tree::new(Meta::new(0o755, 0, 0, T));
        let len = 140 << 20; // spans groups
        t.add(
            ROOT,
            b"big",
            Body::File(Data::Lazy {
                len,
                write: Box::new(move |w: &mut dyn Write| {
                    let chunk = pattern(1 << 20, 3);
                    for _ in 0..140 {
                        w.write_all(&chunk)?;
                    }
                    Ok(())
                }),
            }),
            Meta::new(0o644, 0, 0, T),
        )
        .unwrap();
        let dir = Scratch::new("groups");
        let path = dir.0.join("fs.img");
        let file = std::fs::File::create(&path).unwrap();
        let s = write(&t, &opts(Size::Bytes(1200 << 20)), &file).unwrap();
        assert_eq!(s.bytes, 1200 << 20);
        file.set_len(s.bytes).unwrap();
        let file = std::fs::File::open(&path).unwrap();
        let fs = Ext4::open(&file).unwrap();
        assert_eq!(fs.groups, 10);
        let ino = fs.lookup("/big").unwrap().unwrap();
        let mut h = Crc(0);
        assert_eq!(fs.copy_file(&ino, &mut h).unwrap(), len);
        let mut want = Crc(0);
        let chunk = pattern(1 << 20, 3);
        for _ in 0..140 {
            want.write_all(&chunk).unwrap();
        }
        assert_eq!(h.0, want.0);
        fsck_file(&path);
    }

    #[test]
    fn a_tiny_last_group_is_left_out() {
        let t = Tree::new(Meta::new(0o755, 0, 0, T));
        // One group and 10 blocks: too few for a second group's metadata.
        let s = plan(&t, &opts(Size::Bytes((BLOCKS_PER_GROUP + 10) * BLOCK))).unwrap();
        assert_eq!(s.blocks, BLOCKS_PER_GROUP);
        let mem = Mem::new();
        let s = write(&t, &opts(Size::Bytes(32 << 20)), &mem).unwrap();
        mem.set_len(s.bytes);
        assert_eq!(s.inodes, 2048);
        fsck(&mem.bytes());
    }

    #[test]
    fn mistakes_are_refused() {
        let mut t = Tree::new(Meta::new(0o755, 0, 0, T));
        assert!(t
            .add(ROOT, b"a/b", Body::Fifo, Meta::new(0o644, 0, 0, T))
            .is_err());
        assert!(t
            .add(ROOT, b"..", Body::Fifo, Meta::new(0o644, 0, 0, T))
            .is_err());
        assert!(t
            .add(ROOT, b"x", Body::Fifo, Meta::new(0o100644, 0, 0, T))
            .is_err());
        let d = t
            .add(
                ROOT,
                b"d",
                Body::Dir(BTreeMap::new()),
                Meta::new(0o755, 0, 0, T),
            )
            .unwrap();
        assert!(t.link(ROOT, b"d2", d).is_err());
        assert!(t
            .add(ROOT, b"d", Body::Fifo, Meta::new(0o644, 0, 0, T))
            .is_err());
        assert!(t.put("/d", Body::Fifo, Meta::new(0o644, 0, 0, T)).is_err());
        t.put("/f", Body::Fifo, Meta::new(0o644, 0, 0, T)).unwrap();
        t.put("/f", Body::Socket, Meta::new(0o644, 0, 0, T))
            .unwrap();
        assert!(matches!(t.node(t.lookup("/f").unwrap()).body, Body::Socket));
        // A short promise fails the write.
        t.put(
            "/short",
            Body::File(Data::Lazy {
                len: 10,
                write: Box::new(|w: &mut dyn Write| w.write_all(b"abc")),
            }),
            Meta::new(0o644, 0, 0, T),
        )
        .unwrap();
        assert!(write(
            &t,
            &opts(Size::Fit {
                spare_bytes: 0,
                spare_inodes: 0
            }),
            &Mem::new()
        )
        .is_err());
        t.remove("/short").unwrap();
        // Too small.
        t.put(
            "/big",
            Body::File(Data::Bytes(vec![0; 2 << 20])),
            Meta::new(0o644, 0, 0, T),
        )
        .unwrap();
        assert!(plan(&t, &opts(Size::Bytes(1 << 20))).is_err());
        // lost+found is the writer's.
        t.add(
            ROOT,
            b"lost+found",
            Body::Dir(BTreeMap::new()),
            Meta::new(0o700, 0, 0, T),
        )
        .unwrap();
        assert!(plan(
            &t,
            &opts(Size::Fit {
                spare_bytes: 0,
                spare_inodes: 0
            })
        )
        .is_err());
    }

    /// A CRC-32 of everything written.
    struct Crc(u32);

    impl Write for Crc {
        fn write(&mut self, data: &[u8]) -> io::Result<usize> {
            self.0 = crate::crc32_update(self.0, data);
            Ok(data.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
}
