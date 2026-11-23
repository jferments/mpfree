//! Making the stick: Akai's root filesystem from the update, with MPFree's
//! changes, on a GPT stick laid out as the soft-reboot hook expects.
//!
//! | Partition | Name | What |
//! |---|---|---|
//! | 1 | `mpfree-test-root` | a fresh ext2 holding every file of Akai's root filesystem plus MPFree's changes ([`crate::recipe`]); the hook mounts it read-only |
//! | 2 | `mpfree-test-log` | an empty ext2 (32 MiB) the stick's log service writes |
//! | 3 | `mpfree-test-kits` | an ext2 holding `kits.bin`: MPFree's own drum kits, rendered from the Drums synth |
//! | 5 | `mpfree-setup` | FAT32 `MPFREE`: `autorun.sh` for Akai's script runner, the hook's installer and remover, `mode.txt`, `README.txt` |
//! | 4 | `mpfree-test-data` | from 1 GiB to the end: the pad player's settings, files, library and the eMMC backup |
//!
//! Partition 4's filesystem is not written here: a new stick gets a mark at
//! its start (`MPFREE-FORMAT-ME <token>`) and the stick's own system makes
//! the ext4 at its first start (`setup-format.sh`). A stick that already
//! has an MPFree data partition at 1 GiB keeps it, untouched.

use crate::dev::{self, DevTools};
use crate::recipe::{self, Content, Recipe};
use crate::update::{self, Supported, Unpacked, Update};
use crate::{bad, Msg};
use mpfree_diskimage::ext2w::{self, Body, Data, Meta, NodeId, Options, Size, Tree};
use mpfree_diskimage::ext4::{self, Ext4, Inode, Kind};
use mpfree_diskimage::gpt::{self, Part, Table};
use mpfree_diskimage::{Offset, ReadAt, WriteAt};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

pub const SECTOR: u64 = 512;
/// Partitions start on 1 MiB.
pub const ALIGN: u64 = 2048;
pub const ROOT_NAME: &str = "mpfree-test-root";
/// Partition 1's name while the stick is set to remove MPFree: the MPC's
/// startup hook looks for a partition named [`ROOT_NAME`], so at power-on
/// it passes over this stick and Akai's system starts, where the setup
/// script can take the hook off.
pub const ROOT_OFF_NAME: &str = "mpfree-test-root-off";
pub const LOG_NAME: &str = "mpfree-test-log";
pub const KITS_NAME: &str = "mpfree-test-kits";
pub const SETUP_NAME: &str = "mpfree-setup";
/// The setup partition's volume label: Akai's system mounts it at /media/MPFREE.
pub const SETUP_LABEL: &str = "MPFREE";
pub const SETUP_SIZE: u64 = 128 << 20;
pub const DATA_LABEL: &str = "MPFREE-DATA";
/// The smallest stick: partition 4 must hold the eMMC backup (7.3 GiB) with room to spare.
pub const MIN_STICK: u64 = 15_000_000_000;
/// Partition 4's size in an image file (a stick gets the rest of the stick).
pub const IMAGE_DATA: u64 = 64 << 20;
pub const MARK: &str = "MPFREE-FORMAT-ME ";
/// The MPC One's board file in Akai's /boot.
pub const BOARD_DTB: &str = "/boot/rk3288-az01-acva.dtb";
pub const TOKEN_FILE: &str = "/etc/mpfree-test/format-token";
pub const ABOUT_FILE: &str = "/etc/mpfree-test/made-by.txt";
/// `--new-backup`: asks the stick's first start to back the MPC up again
/// even when partition 4 has a backup. A random token in hex, which the pad
/// player's setup honours once (mpfree-app's `setup::NEW_BACKUP`, the same
/// path); readable by the pad player.
pub const NEW_BACKUP_FILE: &str = "/etc/mpfree-test/new-backup";
/// `--no-backup`, `--no-akai-kits`: what the first start leaves out, a word
/// a line (`backup`, `akai-kits`), read once by the pad player's setup
/// (mpfree-app's `setup::LEAVE_OUT`, the same path); readable by the pad
/// player. Without a backup, Akai's kits and samples are read straight from
/// the MPC through the read-only window.
pub const LEAVE_OUT_FILE: &str = "/etc/mpfree-test/leave-out";
/// `--akai-kits`: asks the first start of an updated stick for Akai's kits
/// and samples, when they were left out before. A random token in hex,
/// honoured once (mpfree-app's `setup::AKAI_KITS`, the same path).
pub const AKAI_KITS_FILE: &str = "/etc/mpfree-test/akai-kits";
/// What a write says when the stick's partition 4 (its backups, settings and
/// files) is kept; the window reads it back to tell an update from a new stick.
pub const KEPT_NOTE: &str = "partition 4: this stick's saved settings, files and backups are kept";

/// MPFree's own programs for the stick, built for the MPC (ARM).
pub struct Programs {
    pub pad_player: Vec<u8>,
    pub screen_log: Vec<u8>,
    /// The startup screen, `mpfree-startup`: it plays once per power-on,
    /// and the pad player keeps its last frame until it draws.
    pub startup: Vec<u8>,
    /// The Plugins place's download helper, [`FETCH_FILE`], if the folder
    /// has it: linked dynamically, since the static pad player can't open
    /// libcurl itself (`mpfree_mpcstore::fetch`). Without it the stick
    /// installs plugins from zips only.
    pub fetch: Option<Vec<u8>>,
    /// The process each plugin runs in, [`PLUGHOST_FILE`], if the folder has
    /// it: linked dynamically like the download helper, since a plugin is a
    /// library built against the MPC's own C library, which the static pad
    /// player can't load safely (device round 2, 20 Nov: every start's
    /// plugin crashed in it). The app runs it from beside itself
    /// (mpfree-app's `plugins::program`). Without it plugins run in the pad
    /// player itself.
    pub plughost: Option<Vec<u8>>,
}

/// The programs' file names in a programs folder: the pad player, the log
/// display and the startup screen.
pub const PROGRAM_FILES: [&str; 3] = ["mpfree", "mpfree-screen-log", "mpfree-startup"];
/// The download helper's file name in a programs folder (optional).
pub const FETCH_FILE: &str = "mpfree-fetch";
/// The plugin process's file name in a programs folder (optional).
pub const PLUGHOST_FILE: &str = "mpfree-plughost";
/// The newest glibc the MPC One has (3.10.1 reports 2.39).
pub const MPC_GLIBC: (u32, u32) = (2, 39);

/// Why `b` can't be the download helper: it must be an ARM program linked
/// dynamically for the MPC's loader, asking for no glibc newer than
/// [`MPC_GLIBC`] (cargo zigbuild for glibc 2.34 makes one).
pub fn fetch_problem(b: &[u8]) -> Option<String> {
    dynamic_problem(b, "it opens libcurl")
}

/// Why `b` can't be the plugin process: as [`fetch_problem`].
pub fn plughost_problem(b: &[u8]) -> Option<String> {
    dynamic_problem(b, "it loads plugins built against the MPC's C library")
}

/// Why `b` isn't an ARM program linked dynamically for the MPC's loader and
/// glibc; `why` says why it must not be static.
fn dynamic_problem(b: &[u8], why: &str) -> Option<String> {
    if !arm_elf(b) {
        return Some("not a program for the MPC (32-bit ARM)".into());
    }
    let u32_at = |at: usize| {
        b.get(at..at + 4)
            .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]) as usize)
    };
    let u16_at = |at: usize| {
        b.get(at..at + 2)
            .map(|s| u16::from_le_bytes([s[0], s[1]]) as usize)
    };
    let (Some(phoff), Some(size), Some(n)) = (u32_at(28), u16_at(42), u16_at(44)) else {
        return Some("cut short".into());
    };
    let interp = (0..n).find_map(|i| {
        let h = phoff + i * size;
        (u32_at(h)? == 3).then_some(())?;
        let (off, len) = (u32_at(h + 4)?, u32_at(h + 16)?);
        b.get(off..off + len)
    });
    if interp.map(|i| i.strip_suffix(b"\0").unwrap_or(i)) != Some(b"/lib/ld-linux-armhf.so.3") {
        return Some(format!(
            "not linked dynamically for the MPC's loader ({why}, so it must not be static)"
        ));
    }
    let mut newest = (0, 0);
    for (i, w) in b.windows(8).enumerate() {
        if w != b"GLIBC_2." {
            continue;
        }
        let digits: String = b[i + 8..]
            .iter()
            .take_while(|c| c.is_ascii_digit())
            .map(|&c| c as char)
            .collect();
        if let Ok(minor) = digits.parse::<u32>() {
            newest = newest.max((2, minor));
        }
    }
    (newest > MPC_GLIBC).then(|| {
        format!(
            "built against glibc {}.{}, newer than the MPC's {}.{}",
            newest.0, newest.1, MPC_GLIBC.0, MPC_GLIBC.1
        )
    })
}

/// Is this a 32-bit little-endian ARM ELF executable?
pub fn arm_elf(b: &[u8]) -> bool {
    b.len() > 52
        && &b[..4] == b"\x7fELF"
        && b[4] == 1
        && b[5] == 1
        && u16::from_le_bytes([b[18], b[19]]) == 40
}

impl Programs {
    /// [`PROGRAM_FILES`] from a folder.
    pub fn from_dir(dir: &Path) -> io::Result<Programs> {
        let read = |name: &str| -> io::Result<Vec<u8>> {
            let p = dir.join(name);
            let b = std::fs::read(&p)
                .map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", p.display())))?;
            if !arm_elf(&b) {
                return Err(bad(format!(
                    "{} is not a program for the MPC (32-bit ARM)",
                    p.display()
                )));
            }
            Ok(b)
        };
        // A dynamic helper, only when the folder has it.
        let optional = |name: &str, problem: fn(&[u8]) -> Option<String>| {
            let p = dir.join(name);
            match std::fs::read(&p) {
                Ok(b) => match problem(&b) {
                    None => Ok(Some(b)),
                    Some(why) => Err(bad(format!(
                        "{} is {why}; build it with cargo zigbuild for armv7-unknown-linux-gnueabihf.2.34",
                        p.display()
                    ))),
                },
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(io::Error::new(e.kind(), format!("{}: {e}", p.display()))),
            }
        };
        let [pad_player, screen_log, startup] = PROGRAM_FILES;
        let fetch = optional(FETCH_FILE, fetch_problem)?;
        let plughost = optional(PLUGHOST_FILE, plughost_problem)?;
        Ok(Programs {
            pad_player: read(pad_player)?,
            screen_log: read(screen_log)?,
            startup: read(startup)?,
            fetch,
            plughost,
        })
    }
}

/// Developer SSH: the user's public key(s) and the stick's own host key.
pub struct Ssh {
    pub authorized_keys: String,
    pub host_private: String,
    pub host_public: String,
}

/// Akai's update, checked, with its root filesystem unpacked to a file.
pub struct Source {
    pub path: PathBuf,
    pub update: Update,
    pub supported: Option<&'static Supported>,
    pub release: String,
    pub rotation: u32,
    pub rootfs: PathBuf,
    pub unpacked: Unpacked,
    pub has_mke2fs: bool,
}

impl Drop for Source {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.rootfs);
    }
}

fn find(fs: &Ext4<File>, paths: &[&str]) -> io::Result<Option<Inode>> {
    for p in paths {
        if let Some(i) = fs.lookup(p)? {
            return Ok(Some(i));
        }
    }
    Ok(None)
}

/// Checks Akai's update and unpacks its root filesystem into `work`.
/// Without `any_update`, only an update in [`update::SUPPORTED`] is taken.
pub fn open_update(
    path: &Path,
    work: &Path,
    any_update: bool,
    say: &mut dyn FnMut(Msg),
) -> io::Result<Source> {
    say(Msg::Note(format!("update file: {}", path.display())));
    let u = Update::open(path)?;
    say(Msg::Note(format!(
        "Akai build {}, boards {}",
        u.build,
        u.compatible.join(" ")
    )));
    if !u.for_mpc_one() {
        return Err(bad("this update is not for the MPC One; MPFree has only been tested on the MPC One (Gen 1)"));
    }
    let supported = u.supported();
    match (supported, any_update) {
        (Some(s), _) => say(Msg::Note(format!("supported: {}", s.firmware))),
        (None, true) => say(Msg::Note(
            "NOT a supported update; going on because you asked (developer option)".into(),
        )),
        (None, false) => {
            return Err(bad(format!(
                "this update (Akai build {}) is not one MPFree supports yet; MPFree needs {}",
                u.build,
                update::SUPPORTED
                    .iter()
                    .map(|s| s.firmware)
                    .collect::<Vec<_>>()
                    .join(" or ")
            )))
        }
    }
    let part = u.rootfs()?.clone();
    std::fs::create_dir_all(work)?;
    let rootfs = work.join(format!("akai-rootfs-{}.img", std::process::id()));
    let out = File::create(&rootfs)?;
    let total = supported.map_or(part.length * 3, |s| s.rootfs_bytes);
    say(Msg::Progress(
        0.0,
        "Unpacking Akai's system from the update".into(),
    ));
    let mut last = 0u64;
    let mut progress_say: Vec<(f32, String)> = Vec::new();
    let unpacked = {
        let cell = std::rc::Rc::new(std::cell::RefCell::new(Vec::<u64>::new()));
        let c2 = cell.clone();
        let r = update::unpack_rootfs(
            path,
            &part,
            io::BufWriter::with_capacity(1 << 20, out),
            move |n| {
                if n >> 24 != last >> 24 {
                    c2.borrow_mut().push(n);
                }
                last = n;
            },
        );
        for n in cell.borrow().iter() {
            progress_say.push((
                (*n as f32 / total as f32).min(1.0),
                "Unpacking Akai's system from the update".into(),
            ));
        }
        match r {
            Ok(x) => x,
            Err(e) => {
                let _ = std::fs::remove_file(&rootfs);
                return Err(e);
            }
        }
    };
    if let Some((f, t)) = progress_say.pop() {
        say(Msg::Progress(f, t));
    }
    say(Msg::Note(format!(
        "root filesystem: {} bytes, checked: {}",
        unpacked.bytes,
        unpacked.checked.join(", ")
    )));
    let mut src = Source {
        path: path.to_path_buf(),
        update: u,
        supported,
        release: String::new(),
        rotation: 0,
        rootfs,
        unpacked,
        has_mke2fs: false,
    };
    if let Some(s) = supported {
        if src.unpacked.bytes != s.rootfs_bytes {
            return Err(bad(format!(
                "the root filesystem is {} bytes, not the {} bytes of {}",
                src.unpacked.bytes, s.rootfs_bytes, s.firmware
            )));
        }
    }
    let fs = Ext4::open(File::open(&src.rootfs)?)?;
    let boot = fs
        .lookup("/boot")?
        .ok_or_else(|| bad("Akai's root filesystem has no /boot"))?;
    let kernels: Vec<String> = fs
        .read_dir(&boot)?
        .into_iter()
        .filter_map(|e| String::from_utf8(e.name).ok())
        .filter_map(|n| n.strip_prefix("zImage-").map(str::to_string))
        .collect();
    let [release] = kernels.as_slice() else {
        return Err(bad(format!(
            "expected one kernel in /boot, found {}",
            kernels.len()
        )));
    };
    if let Some(s) = supported {
        if *release != s.release {
            return Err(bad(format!(
                "the kernel is {release}, not {}'s {}",
                s.firmware, s.release
            )));
        }
    }
    src.release = release.clone();
    say(Msg::Note(format!("kernel release: {release}")));
    let dtb_ino = fs
        .lookup(BOARD_DTB)?
        .ok_or_else(|| bad(format!("Akai's root filesystem has no {BOARD_DTB}")))?;
    let dtb = fs.read_file(&dtb_ino)?;
    let compatible = crate::dtb::prop(&dtb, "/", "compatible")?
        .map(|v| crate::dtb::strings(&v))
        .unwrap_or_default();
    if !compatible.iter().any(|c| c == "inmusic,acva") {
        return Err(bad(format!(
            "{BOARD_DTB} is not the MPC One's board file ({})",
            compatible.join(" ")
        )));
    }
    src.rotation = crate::dtb::panel_rotation(&dtb)?;
    say(Msg::Note(format!(
        "panel rotation: {} degrees",
        src.rotation
    )));
    src.has_mke2fs = find(
        &fs,
        &["/usr/sbin/mke2fs", "/sbin/mke2fs", "/usr/bin/mke2fs"],
    )?
    .is_some();
    if !src.has_mke2fs {
        return Err(bad("Akai's system has no mke2fs, which the stick needs to make its data partition at its first start"));
    }
    Ok(src)
}

/// Every file, folder, link and device of an ext2/3/4 filesystem as a tree
/// for the writer, with file bytes read from it when the tree is written.
pub fn copy_tree<'a, R: ReadAt>(fs: &'a Ext4<R>) -> io::Result<Tree<'a>> {
    let root = fs.inode(ext4::ROOT)?;
    let mut tree = Tree::new(meta_of(fs, &root)?);
    let mut seen = HashMap::new();
    copy_dir(fs, &root, &mut tree, ext2w::ROOT, &mut seen, 0)?;
    Ok(tree)
}

fn meta_of<R: ReadAt>(fs: &Ext4<R>, ino: &Inode) -> io::Result<Meta> {
    Ok(Meta {
        perm: ino.perm(),
        uid: ino.uid,
        gid: ino.gid,
        atime: ino.atime,
        ctime: ino.ctime,
        mtime: ino.mtime,
        xattrs: fs.xattrs(ino)?,
    })
}

fn copy_dir<'a, R: ReadAt>(
    fs: &'a Ext4<R>,
    dir: &Inode,
    tree: &mut Tree<'a>,
    node: NodeId,
    seen: &mut HashMap<u32, NodeId>,
    depth: usize,
) -> io::Result<()> {
    if depth > 64 {
        return Err(bad("folders nest more than 64 deep"));
    }
    for e in fs.read_dir(dir)? {
        if depth == 0 && e.name == b"lost+found" {
            continue; // the writer makes its own
        }
        if let Some(&id) = seen.get(&e.ino) {
            tree.link(node, &e.name, id)?;
            continue;
        }
        let ino = fs.inode(e.ino)?;
        let meta = meta_of(fs, &ino)?;
        let kind = ino
            .kind()
            .ok_or_else(|| bad(format!("inode {} has an unknown type", e.ino)))?;
        let body = match kind {
            Kind::Dir => Body::Dir(BTreeMap::new()),
            Kind::File => {
                let i = ino.clone();
                Body::File(Data::Lazy {
                    len: ino.size,
                    write: Box::new(move |w: &mut dyn Write| fs.copy_file(&i, w).map(|_| ())),
                })
            }
            Kind::Symlink => Body::Symlink(fs.read_link(&ino)?),
            Kind::CharDev => Body::CharDev(ino.rdev().0, ino.rdev().1),
            Kind::BlockDev => Body::BlockDev(ino.rdev().0, ino.rdev().1),
            Kind::Fifo => Body::Fifo,
            Kind::Socket => Body::Socket,
        };
        let id = tree.add(node, &e.name, body, meta)?;
        if kind == Kind::Dir {
            copy_dir(fs, &ino, tree, id, seen, depth + 1)?;
        } else if ino.links > 1 {
            seen.insert(e.ino, id);
        }
    }
    Ok(())
}

/// What differs from stick to stick.
pub struct Values {
    /// Partition 1's: a developer route's command line names it.
    pub root_uuid: String,
    pub log_uuid: String,
    pub kits_uuid: String,
    pub data_uuid: String,
    pub release: String,
    pub rotation: u32,
    pub token: String,
    pub time: u32,
}

impl Values {
    pub fn new(release: &str, rotation: u32) -> Values {
        let uuid = || uuid_text(&gpt::random_guid());
        Values {
            root_uuid: uuid(),
            log_uuid: uuid(),
            kits_uuid: uuid(),
            data_uuid: uuid(),
            release: release.to_string(),
            rotation,
            token: update::hex(&gpt::random16()),
            time: now(),
        }
    }
}

pub fn now() -> u32 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as u32)
        .unwrap_or(0)
}

/// A partition GUID as Linux shows it in /dev/disk/by-partuuid (lower case).
pub fn uuid_text(g: &gpt::Guid) -> String {
    gpt::guid_text(g).to_ascii_lowercase()
}

fn kind_of(tree: &Tree, path: &str) -> Option<&'static str> {
    let id = tree.lookup(path)?;
    Some(match tree.node(id).body {
        Body::Dir(_) => "directory",
        Body::File(_) => "regular",
        Body::Symlink(_) => "symlink",
        _ => "other",
    })
}

fn file_text(
    tree: &Tree,
    path: &str,
    src: &dyn Fn(&str) -> io::Result<Vec<u8>>,
) -> io::Result<(String, u16)> {
    let id = tree
        .lookup(path)
        .ok_or_else(|| bad(format!("Akai's root filesystem has no {path}")))?;
    let n = tree.node(id);
    if !matches!(n.body, Body::File(_)) {
        return Err(bad(format!("{path} is not a regular file")));
    }
    let text = String::from_utf8(src(path)?).map_err(|_| bad(format!("{path} is not text")))?;
    Ok((text, n.meta.perm))
}

/// The mpfree user in /etc/passwd and /etc/group, as the builder's app_user_edits.
pub fn app_user_edits(r: &Recipe, passwd: &str, group: &str) -> io::Result<(String, String)> {
    let u = &r.app_user;
    let uid = u.uid.to_string();
    let mut out = Vec::new();
    for (path, text, fields) in [("/etc/passwd", passwd, 7), ("/etc/group", group, 4)] {
        if !text.ends_with('\n') || text.lines().any(|l| l.split(':').count() < fields) {
            return Err(bad(format!(
                "{path} is not lines of colon-separated fields"
            )));
        }
        let mut rows: Vec<Vec<String>> = text
            .lines()
            .map(|l| l.split(':').map(str::to_string).collect())
            .collect();
        if rows.iter().any(|r| r[0] == u.name || r[2] == uid) {
            return Err(bad(format!(
                "{path} already has the name {} or the id {}",
                u.name, u.uid
            )));
        }
        if path == "/etc/passwd" {
            rows.push(vec![
                u.name.clone(),
                "x".into(),
                uid.clone(),
                uid.clone(),
                u.gecos.clone(),
                u.home.clone(),
                u.shell.clone(),
            ]);
        } else {
            let names: Vec<&String> = rows.iter().map(|r| &r[0]).collect();
            let missing: Vec<&String> = u.groups.iter().filter(|g| !names.contains(g)).collect();
            if !missing.is_empty() {
                return Err(bad(format!("Akai's /etc/group has no {:?} group", missing)));
            }
            for row in rows.iter_mut() {
                if u.groups.contains(&row[0]) || u.optional_groups.contains(&row[0]) {
                    let mut members: Vec<String> = row[3]
                        .split(',')
                        .filter(|m| !m.is_empty())
                        .map(str::to_string)
                        .collect();
                    members.push(u.name.clone());
                    row[3] = members.join(",");
                }
            }
            rows.push(vec![u.name.clone(), "x".into(), uid.clone(), String::new()]);
        }
        out.push(rows.iter().map(|r| r.join(":") + "\n").collect::<String>());
    }
    Ok((out.remove(0), out.remove(0)))
}

/// Root's password field: "!" (locked) becomes "*", so OpenSSH takes key
/// logins; no password works either way. Only with developer SSH.
pub fn shadow_edit(shadow: &str) -> io::Result<Option<String>> {
    let mut lines: Vec<String> = shadow.split('\n').map(str::to_string).collect();
    let roots: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.starts_with("root:"))
        .map(|(i, _)| i)
        .collect();
    let [i] = roots.as_slice() else {
        return Err(bad("expected one root line in /etc/shadow"));
    };
    let mut f: Vec<String> = lines[*i].split(':').map(str::to_string).collect();
    if f.len() < 2 {
        return Err(bad("the root line in /etc/shadow is malformed"));
    }
    if !f[1].starts_with('!') {
        return Ok(None);
    }
    f[1] = "*".into();
    lines[*i] = f.join(":");
    Ok(Some(lines.join("\n")))
}

fn fill(text: &str, r: &Recipe, v: &Values, ssh: Option<&Ssh>, kernel_sha256: &str) -> String {
    let p = |k: &str| r.placeholders.get(k).cloned().unwrap_or_default();
    let mut t = text
        .replace(&p("root_uuid"), &v.root_uuid)
        .replace(&p("kernel_sha256"), kernel_sha256)
        .replace(&p("log_uuid"), &v.log_uuid)
        .replace(&p("kits_uuid"), &v.kits_uuid)
        .replace(&p("data_uuid"), &v.data_uuid)
        .replace(&p("release"), &v.release)
        .replace(&p("rotation"), &v.rotation.to_string());
    if let Some(s) = ssh {
        t = t.replace(&p("authorized_keys"), s.authorized_keys.trim_end());
    }
    t
}

/// Applies MPFree's changes to the tree of Akai's root filesystem.
/// `read(path)` reads one of Akai's files.
pub fn apply<'a>(
    tree: &mut Tree<'a>,
    read: &dyn Fn(&str) -> io::Result<Vec<u8>>,
    v: &Values,
    programs: &'a Programs,
    ssh: Option<&'a Ssh>,
    about: &str,
    choices: &Choices,
) -> io::Result<Vec<String>> {
    let (r, route) = match choices.route.recipe_name() {
        None => (Recipe::load(), None),
        Some(name) => {
            let (r, route) = Recipe::load()
                .with_route(name)
                .ok_or_else(|| bad(format!("the recipe has no route {name}")))?;
            (r, Some(route))
        }
    };
    let mut notes = Vec::new();
    for top in ["/bin", "/sbin", "/lib"] {
        let ok = match tree.lookup(top).map(|id| &tree.node(id).body) {
            Some(Body::Symlink(t)) => {
                t == format!("usr{top}").as_bytes() || t == format!("/usr{top}").as_bytes()
            }
            None => true,
            _ => false,
        };
        if !ok {
            return Err(bad(format!(
                "{top} is not a link into /usr; this app does not know this layout"
            )));
        }
    }
    let resolve = |p: &str| -> String {
        for top in ["/bin/", "/sbin/", "/lib/"] {
            if let Some(rest) = p.strip_prefix(top) {
                return format!("/usr{top}{rest}");
            }
        }
        p.to_string()
    };
    for (path, kind) in &r.required {
        let found = kind_of(tree, &resolve(path));
        if found.is_none() || (!kind.is_empty() && found != Some(kind.as_str())) {
            return Err(bad(format!(
                "Akai's root filesystem has no {} {path}",
                if kind.is_empty() { "file" } else { kind }
            )));
        }
    }
    for prog in &r.app_user.programs {
        if !["/usr/bin", "/usr/sbin"].iter().any(|d| {
            matches!(
                kind_of(tree, &format!("{d}/{prog}")),
                Some("regular" | "symlink")
            )
        }) {
            return Err(bad(format!(
                "Akai's root filesystem has no {prog} in /usr/bin or /usr/sbin"
            )));
        }
    }
    let missing: Vec<&str> = recipe::setup_programs()
        .into_iter()
        .filter(|prog| {
            !["/usr/bin", "/usr/sbin"].iter().any(|d| {
                matches!(
                    kind_of(tree, &format!("{d}/{prog}")),
                    Some("regular" | "symlink")
                )
            })
        })
        .collect();
    if !missing.is_empty() {
        return Err(bad(format!(
            "Akai's root filesystem has no {} in /usr/bin or /usr/sbin, which the stick's first-start setup needs",
            missing.join(", ")
        )));
    }
    if let Some(c) = route.as_ref().and_then(|x| x.chain_check.as_ref()) {
        notes.push(chain_check(tree, read, c)?);
    }
    // A developer route's hook files name Akai's kernel by its hash, and the
    // chain's kexec.dtb is built for this MPC.
    let kernel_sha256 = match &route {
        Some(_) => update::hex(&Sha256::digest(read(&format!(
            "/boot/zImage-{}",
            v.release
        ))?)),
        None => String::new(),
    };
    let kexec_dtb = match &choices.route {
        Route::Chain { live_dtb } => {
            let made = crate::dtb::kexec_tree(&read(BOARD_DTB)?, live_dtb, &r.kexec_dtb)?;
            notes.push(format!(
                "chain: kexec.dtb from the update's board file ({BOARD_DTB}) and this MPC's running tree, {} bytes; what changed, by name:",
                made.blob.len()
            ));
            notes.extend(made.notes.iter().map(|n| format!("  {n}")));
            Some(made.blob)
        }
        _ => None,
    };
    let skip = |p: &str| ssh.is_none() && r.only_with_ssh.iter().any(|s| s == p);
    let mut new_paths: Vec<&str> = r
        .files
        .keys()
        .chain(r.links.keys())
        .map(String::as_str)
        .collect();
    new_paths.extend(recipe::SETUP_FILES.iter().map(|f| f.0));
    new_paths.extend(recipe::SETUP_LINKS.iter().map(|l| l.0));
    new_paths.extend([TOKEN_FILE, ABOUT_FILE]);
    if choices.new_backup {
        new_paths.push(NEW_BACKUP_FILE);
    }
    if choices.akai_kits {
        new_paths.push(AKAI_KITS_FILE);
    }
    if !choices.left_out().is_empty() {
        new_paths.push(LEAVE_OUT_FILE);
    }
    if let Some(x) = &route {
        new_paths.extend(x.mkdirs.iter().map(String::as_str));
        new_paths.extend(x.kexec_sums.iter().map(|k| k.path.as_str()));
    }
    for p in &new_paths {
        if kind_of(tree, p).is_some() {
            return Err(bad(format!(
                "{p} already exists in Akai's root filesystem; refusing to guess what it is"
            )));
        }
    }
    let root_meta = |perm: u16| Meta::new(perm, 0, 0, v.time);
    for d in &r.mkdirs {
        tree.mkdir_all(d, root_meta(0o755))?;
    }
    for d in &r.made_if_missing {
        match kind_of(tree, d) {
            None => {
                tree.mkdir_all(d, root_meta(0o755))?;
            }
            Some("directory") => {}
            Some(k) => return Err(bad(format!("{d} is a {k}, not a folder"))),
        }
    }
    let mut written: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for (path, f) in &r.files {
        if skip(path) || *path == r.softreboot_sums {
            continue;
        }
        let bytes = match &f.content {
            Content::Text(t) => fill(t, &r, v, ssh, &kernel_sha256).into_bytes(),
            Content::Placeholder(p) if *p == r.placeholders["pad_player"] => {
                programs.pad_player.clone()
            }
            Content::Placeholder(p) if *p == r.placeholders["screen_log"] => {
                programs.screen_log.clone()
            }
            Content::Placeholder(p) if *p == r.placeholders["startup"] => programs.startup.clone(),
            // Only when the programs folder has the helper.
            Content::Placeholder(p) if *p == r.placeholders["fetch"] => match &programs.fetch {
                Some(b) => b.clone(),
                None => continue,
            },
            Content::Placeholder(p) if *p == r.placeholders["plughost"] => {
                match &programs.plughost {
                    Some(b) => b.clone(),
                    None => continue,
                }
            }
            Content::Placeholder(p) if *p == r.placeholders["kexec_dtb"] => kexec_dtb
                .clone()
                .ok_or_else(|| bad("kexec.dtb is only for the chain"))?,
            Content::Placeholder(p) if *p == r.placeholders["host_key"] => ssh
                .ok_or_else(|| bad("the host key is only for developer SSH"))?
                .host_private
                .clone()
                .into_bytes(),
            Content::Placeholder(p) => return Err(bad(format!("unknown placeholder {p}"))),
        };
        written.insert(path.clone(), bytes);
    }
    // The chain's own list, which the hook's list hashes in turn.
    let kexec_sums = route.as_ref().and_then(|x| x.kexec_sums.as_ref());
    if let Some(k) = kexec_sums {
        let mut text = String::new();
        for name in &k.names {
            let path = format!("{}/{name}", k.dir);
            let b = match (written.get(&path), r.links.get(&path)) {
                (Some(b), _) => b.clone(),
                (None, Some(target)) => read(&link_path(
                    &k.dir,
                    &fill(target, &r, v, ssh, &kernel_sha256),
                ))?,
                (None, None) => {
                    return Err(bad(format!("{path} is listed in {} but not made", k.path)))
                }
            };
            text.push_str(&format!("{}  {name}\n", update::hex(&Sha256::digest(&b))));
        }
        written.insert(k.path.clone(), text.into_bytes());
    }
    let sums: String = r
        .softreboot_listed
        .iter()
        .map(|p| {
            let b = written
                .get(p)
                .ok_or_else(|| bad(format!("{p} is listed for the hook but not written")))?;
            Ok(format!("{}  {p}\n", update::hex(&Sha256::digest(b))))
        })
        .collect::<io::Result<String>>()?;
    written.insert(r.softreboot_sums.clone(), sums.into_bytes());
    for (path, f) in &r.files {
        if let Some(b) = written.remove(path) {
            tree.put(path, Body::File(Data::Bytes(b)), root_meta(f.mode))?;
        }
    }
    if let Some(b) = kexec_sums.and_then(|k| written.remove(&k.path)) {
        let path = &kexec_sums.expect("taken from it").path;
        tree.put(path, Body::File(Data::Bytes(b)), root_meta(0o644))?;
    }
    notes.push(match &choices.route {
        Route::SoftReboot => "route: soft-reboot (the hook restarts the MPC into this stick's system)".into(),
        other => {
            let cmdline = r
                .files
                .get(&format!("{}/cmdline", parent_dir(&r.softreboot_sums)))
                .and_then(|f| match &f.content {
                    Content::Text(t) => Some(fill(t, &r, v, ssh, &kernel_sha256)),
                    Content::Placeholder(_) => None,
                })
                .unwrap_or_default();
            format!(
                "route: {} (developer option): the hook restarts into Akai's own kernel from the MPC (file-based kexec) with this stick as its root, and soft-reboots if that is refused; its command line: {}{}",
                other.flags(),
                cmdline.trim_end(),
                if matches!(other, Route::Chain { .. }) {
                    "; on that kernel the stick's system restarts once more, into its own kernel and kexec.dtb (once per power-on: a failed try starts MPFree on the first kernel)"
                } else {
                    ""
                }
            )
        }
    });
    let mut stubbed = Vec::new();
    for prog in &r.stub_programs {
        match kind_of(tree, prog) {
            None => {}
            Some("regular" | "symlink") => {
                tree.put(
                    prog,
                    Body::File(Data::Bytes(r.stub.clone().into_bytes())),
                    root_meta(0o755),
                )?;
                stubbed.push(prog.as_str());
            }
            Some(k) => return Err(bad(format!("{prog} is a {k}; refusing to replace it"))),
        }
    }
    notes.push(format!("stubbed: {}", stubbed.join(" ")));
    notes.push(match &programs.fetch {
        Some(b) => format!(
            "download helper: {FETCH_FILE} ({} bytes, sha256 {}): the Plugins place downloads through it",
            b.len(),
            update::hex(&Sha256::digest(b))
        ),
        None => format!(
            "download helper: none (no {FETCH_FILE} in the programs folder): the Plugins place installs from zips on the stick only"
        ),
    });
    notes.push(match &programs.plughost {
        Some(b) => format!(
            "plugin process: {PLUGHOST_FILE} ({} bytes, sha256 {}): each plugin runs in it",
            b.len(),
            update::hex(&Sha256::digest(b))
        ),
        None => format!(
            "plugin process: none (no {PLUGHOST_FILE} in the programs folder): plugins run in the pad player itself, which can't run most of them"
        ),
    });
    if ssh.is_some() {
        let (shadow, perm) = file_text(tree, "/etc/shadow", read)?;
        if let Some(edited) = shadow_edit(&shadow)? {
            tree.put(
                "/etc/shadow",
                Body::File(Data::Bytes(edited.into_bytes())),
                root_meta(perm),
            )?;
            notes.push("root account was marked locked (\"!\"); set to \"*\" (key login only, no password)".into());
        }
    }
    let (passwd, pperm) = file_text(tree, "/etc/passwd", read)?;
    let (group, gperm) = file_text(tree, "/etc/group", read)?;
    let (passwd, group) = app_user_edits(&r, &passwd, &group)?;
    tree.put(
        "/etc/passwd",
        Body::File(Data::Bytes(passwd.into_bytes())),
        root_meta(pperm),
    )?;
    tree.put(
        "/etc/group",
        Body::File(Data::Bytes(group.into_bytes())),
        root_meta(gperm),
    )?;
    for (path, mode, text) in recipe::SETUP_FILES {
        let text = text.replace("@DATA_UUID@", &v.data_uuid);
        if let Some((folder, _)) = path.rsplit_once('/') {
            tree.mkdir_all(folder, root_meta(0o755))?;
        }
        tree.put(
            path,
            Body::File(Data::Bytes(text.into_bytes())),
            root_meta(*mode),
        )?;
    }
    tree.put(
        TOKEN_FILE,
        Body::File(Data::Bytes(format!("{}\n", v.token).into_bytes())),
        root_meta(0o600),
    )?;
    tree.put(
        ABOUT_FILE,
        Body::File(Data::Bytes(about.as_bytes().to_vec())),
        root_meta(0o644),
    )?;
    if choices.new_backup {
        tree.put(
            NEW_BACKUP_FILE,
            Body::File(Data::Bytes(
                format!("{}\n", update::hex(&gpt::random16())).into_bytes(),
            )),
            root_meta(0o644),
        )?;
        notes.push(
            "first start: backs the MPC up again, keeping the backups partition 4 has (--new-backup)"
                .into(),
        );
    }
    if choices.akai_kits {
        tree.put(
            AKAI_KITS_FILE,
            Body::File(Data::Bytes(
                format!("{}\n", update::hex(&gpt::random16())).into_bytes(),
            )),
            root_meta(0o644),
        )?;
        notes.push(
            "first start: brings in Akai's kits and samples if they were left out (--akai-kits)"
                .into(),
        );
    }
    let left = choices.left_out();
    if !left.is_empty() {
        let text: String = left.iter().map(|w| format!("{w}\n")).collect();
        tree.put(
            LEAVE_OUT_FILE,
            Body::File(Data::Bytes(text.into_bytes())),
            root_meta(0o644),
        )?;
        notes.push(format!(
            "first start: leaves out {} (as chosen; MPFree's Prefs can bring {} in later)",
            choices.left_out_text(),
            if left.len() == 1 { "it" } else { "them" }
        ));
    }
    let mut links: Vec<(&str, &str)> = r
        .links
        .iter()
        .filter(|(p, _)| !skip(p))
        .map(|(p, t)| (p.as_str(), t.as_str()))
        .collect();
    links.extend(recipe::SETUP_LINKS.iter().copied());
    for (path, target) in links {
        tree.put(
            path,
            Body::Symlink(fill(target, &r, v, ssh, &kernel_sha256).into_bytes()),
            root_meta(0o777),
        )?;
    }
    Ok(notes)
}

fn parent_dir(path: &str) -> &str {
    path.rsplit_once('/').map_or("/", |(d, _)| d)
}

/// A link's target as a path from the root: `target` relative to `dir`.
fn link_path(dir: &str, target: &str) -> String {
    let mut parts: Vec<&str> = if target.starts_with('/') {
        Vec::new()
    } else {
        dir.split('/').filter(|p| !p.is_empty()).collect()
    };
    for part in target.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    format!("/{}", parts.join("/"))
}

/// The chain loads with Akai's own kexec: it must be there, and be the one
/// systemd-shutdown runs for kexec.target (the builder's chain_kexec_check).
fn chain_check(
    tree: &Tree,
    read: &dyn Fn(&str) -> io::Result<Vec<u8>>,
    c: &recipe::ChainCheck,
) -> io::Result<String> {
    if kind_of(tree, &c.kexec_program) != Some("regular") {
        return Err(bad(format!(
            "--chain: Akai's system has no regular {} (Akai's kexec, which the chain loads with)",
            c.kexec_program
        )));
    }
    if kind_of(tree, &c.shutdown_program) != Some("regular") {
        return Err(bad(format!(
            "--chain: Akai's system has no {}, so it is unknown how systemd starts a loaded kernel",
            c.shutdown_program
        )));
    }
    let named = kexec_names(&read(&c.shutdown_program)?);
    let other: Vec<&String> = named
        .iter()
        .filter(|p| !c.kexec_paths.contains(p))
        .collect();
    if !other.is_empty() {
        return Err(bad(format!(
            "--chain: {} names {} for kexec, which the creator does not keep; systemd would not start the loaded kernel",
            c.shutdown_program,
            other.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
        )));
    }
    Ok(format!(
        "--chain: {} runs {} for kexec.target; {} stays Akai's",
        c.shutdown_program,
        if named.is_empty() {
            "no kexec program (it starts the loaded kernel itself)".to_string()
        } else {
            named.join(" or ")
        },
        c.kexec_program
    ))
}

/// Every path to a program named kexec in `b`, sorted, once each: what the
/// builder's `/[A-Za-z0-9_./-]*/kexec(?![A-Za-z0-9_.-])` finds.
pub fn kexec_names(b: &[u8]) -> Vec<String> {
    let path_char = |c: u8| c.is_ascii_alphanumeric() || b"_./-".contains(&c);
    let name_char = |c: u8| c.is_ascii_alphanumeric() || b"_.-".contains(&c);
    let mut found = std::collections::BTreeSet::new();
    let mut i = 0;
    while i < b.len() {
        if !path_char(b[i]) {
            i += 1;
            continue;
        }
        let end = (i..b.len()).find(|&j| !path_char(b[j])).unwrap_or(b.len());
        // Within this run of path characters, matches go left to right: each
        // from a '/', to the last "/kexec" after it that no name character follows.
        let mut at = i;
        while let Some(start) = (at..end).find(|&j| b[j] == b'/') {
            let last = (start + 1..end.saturating_sub(5))
                .rev()
                .find(|&j| &b[j..j + 6] == b"/kexec" && (j + 6 == end || !name_char(b[j + 6])));
            match last {
                Some(j) => {
                    found.insert(String::from_utf8_lossy(&b[start..j + 6]).into_owned());
                    at = j + 6;
                }
                None => break,
            }
        }
        i = end;
    }
    found.into_iter().collect()
}

/// The kits partition's tree: `kits.bin`, read-only for everyone.
pub fn kits_tree(kits: &[u8], time: u32) -> io::Result<Tree<'static>> {
    let mut t = Tree::new(Meta::new(0o755, 0, 0, time));
    t.add(
        ext2w::ROOT,
        b"kits.bin",
        Body::File(Data::Bytes(kits.to_vec())),
        Meta::new(0o644, 0, 0, time),
    )?;
    Ok(t)
}

/// The setup partition (FAT32), whole, in memory, with the developer tools
/// under `mpfree-dev/` if there are any.
pub fn setup_partition(release: &str, mode: &str, dev: Option<&DevTools>) -> io::Result<Vec<u8>> {
    let mut img = vec![0u8; SETUP_SIZE as usize];
    let mut label = [b' '; 11];
    label[..SETUP_LABEL.len()].copy_from_slice(SETUP_LABEL.as_bytes());
    fatfs::format_volume(
        Cursor::new(&mut img[..]),
        fatfs::FormatVolumeOptions::new()
            .fat_type(fatfs::FatType::Fat32)
            .bytes_per_cluster(1024)
            .volume_label(label),
    )?;
    {
        let fs = fatfs::FileSystem::new(Cursor::new(&mut img[..]), fatfs::FsOptions::new())?;
        let root = fs.root_dir();
        for (name, text) in recipe::SETUP_DRIVE {
            let text = match *name {
                "autorun.sh" => text.replace("@RELEASE@", release),
                "mode.txt" => format!("{mode}\n"),
                _ => text.to_string(),
            };
            let mut f = root.create_file(name)?;
            f.truncate()?;
            f.write_all(text.as_bytes())?;
            f.flush()?;
        }
        if let Some(dev) = dev {
            root.create_dir(dev::DIR)?;
            for folder in &dev.folders {
                root.create_dir(&format!("{}/{folder}", dev::DIR))?;
            }
            let made = [
                ("SHA256SUMS".to_string(), dev.sums().into_bytes()),
                (dev::MARK.to_string(), dev::MARK_TEXT.as_bytes().to_vec()),
            ];
            for (path, data) in dev.files.iter().chain(made.iter()) {
                let mut f = root.create_file(&format!("{}/{path}", dev::DIR))?;
                f.truncate()?;
                f.write_all(data)?;
                f.flush()?;
            }
        }
    }
    Ok(img)
}

/// A file from a setup partition held in memory.
pub fn setup_read(img: &mut [u8], name: &str) -> io::Result<Option<Vec<u8>>> {
    let fs = fatfs::FileSystem::new(Cursor::new(img), fatfs::FsOptions::new())?;
    let mut f = match fs.root_dir().open_file(name) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let mut out = Vec::new();
    f.read_to_end(&mut out)?;
    Ok(Some(out))
}

/// Writes a file into a setup partition held in memory.
pub fn setup_write(img: &mut [u8], name: &str, data: &[u8]) -> io::Result<()> {
    let fs = fatfs::FileSystem::new(Cursor::new(img), fatfs::FsOptions::new())?;
    let mut f = fs.root_dir().create_file(name)?;
    f.truncate()?;
    f.write_all(data)?;
    f.flush()
}

fn align_up(sectors: u64) -> u64 {
    sectors.div_ceil(ALIGN) * ALIGN
}

/// Where the partitions go on a disk of `disk_sectors`.
pub struct Layout {
    pub table: Table,
    pub disk_sectors: u64,
}

impl Layout {
    pub fn part(&self, n: usize) -> &Part {
        self.table.part(n).expect("every partition is laid out")
    }
}

pub fn layout(
    root_bytes: u64,
    kits_bytes: u64,
    disk_sectors: u64,
    data_end: Option<u64>,
    v: &Values,
) -> io::Result<Layout> {
    let r = Recipe::load();
    let linux = gpt::guid(gpt::LINUX_FS)?;
    let data = gpt::guid(gpt::BASIC_DATA)?;
    let guid = |t: &str| gpt::guid(&t.to_ascii_uppercase());
    let root_start = ALIGN;
    let root_end = root_start + align_up(root_bytes.div_ceil(SECTOR));
    let log_end = root_end + r.log_size / SECTOR;
    let kits_end = log_end + align_up(kits_bytes.div_ceil(SECTOR));
    let setup_end = kits_end + SETUP_SIZE / SECTOR;
    let data_start = r.data_start_sectors;
    if setup_end > data_start {
        return Err(bad(format!(
            "partitions 1, 2, 3 and 5 need {} MiB, past where partition 4 starts ({} MiB)",
            (setup_end * SECTOR) >> 20,
            (data_start * SECTOR) >> 20
        )));
    }
    let last_usable = disk_sectors
        .checked_sub(34)
        .ok_or_else(|| bad("the disk is too small"))?;
    let data_last = data_end.unwrap_or(last_usable).min(last_usable);
    if data_last <= data_start + ALIGN {
        return Err(bad("the disk is too small for partition 4"));
    }
    let part = |type_guid, guid, first: u64, end: u64, name: &str| Part {
        type_guid,
        guid,
        first,
        last: end - 1,
        attrs: 0,
        name: name.to_string(),
    };
    let table = Table {
        disk_guid: gpt::random_guid(),
        parts: vec![
            Some(part(
                linux,
                guid(&v.root_uuid)?,
                root_start,
                root_end,
                ROOT_NAME,
            )),
            Some(part(linux, guid(&v.log_uuid)?, root_end, log_end, LOG_NAME)),
            Some(part(
                linux,
                guid(&v.kits_uuid)?,
                log_end,
                kits_end,
                KITS_NAME,
            )),
            Some(part(
                linux,
                guid(&v.data_uuid)?,
                data_start,
                data_last + 1,
                &r.data_name,
            )),
            Some(part(
                data,
                gpt::random_guid(),
                kits_end,
                setup_end,
                SETUP_NAME,
            )),
        ],
    };
    Ok(Layout {
        table,
        disk_sectors,
    })
}

/// What a build asks for.
pub struct Build<'a> {
    pub source: &'a Source,
    pub programs: &'a Programs,
    pub kits: &'a [u8],
    pub ssh: Option<&'a Ssh>,
    /// Developer tools for Akai's system, on partition 5 (`--dev-tools`).
    pub dev: Option<&'a DevTools>,
    /// A line for made-by.txt and the report: the creator's version.
    pub creator: &'a str,
    pub choices: Choices,
}

/// What the person making the stick chose beyond the defaults.
#[derive(Clone, Debug, Default)]
pub struct Choices {
    /// `--new-backup`: the first start backs the MPC up again even when
    /// partition 4 has a backup ([`NEW_BACKUP_FILE`]).
    pub new_backup: bool,
    /// `--no-backup`: the first start makes no backup ([`LEAVE_OUT_FILE`]).
    pub no_backup: bool,
    /// `--no-akai-kits`: the first start brings in none of Akai's kits and
    /// samples ([`LEAVE_OUT_FILE`]).
    pub no_akai_kits: bool,
    /// `--akai-kits`: an updated stick's first start brings in Akai's kits
    /// and samples if they were left out before ([`AKAI_KITS_FILE`]).
    pub akai_kits: bool,
    /// How the MPC's hook starts the stick (developer options).
    pub route: Route,
}

impl Choices {
    /// The words of [`LEAVE_OUT_FILE`].
    pub fn left_out(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if self.no_backup {
            out.push("backup");
        }
        if self.no_akai_kits {
            out.push("akai-kits");
        }
        out
    }

    /// "the backup", "Akai's kits and samples" or both, for notes.
    pub fn left_out_text(&self) -> String {
        match (self.no_backup, self.no_akai_kits) {
            (true, true) => "the backup and Akai's kits and samples".into(),
            (true, false) => "the backup".into(),
            (false, true) => "Akai's kits and samples".into(),
            (false, false) => "nothing".into(),
        }
    }
}

/// How the MPC's startup hook starts the stick's system. The soft-reboot is
/// what MPFree supports; the others are developer options, the same routes
/// as `offline-kexec-stick.py --kexec-file [--loadpin-off [--chain]]`
/// (docs/dev/design/soft-reboot-boot.md).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Route {
    /// The hook soft-reboots the MPC into the stick's system.
    #[default]
    SoftReboot,
    /// `--kexec-file`: the hook restarts into Akai's own kernel from the
    /// MPC (file-based kexec) with the stick as its root, and soft-reboots
    /// if that is refused. With `--loadpin-off`, that kernel's command line
    /// also has loadpin.enforce=0.
    KexecFile { loadpin_off: bool },
    /// `--chain` (with both): on that kernel the stick's system restarts
    /// once more, into its own kernel and a device tree built from the
    /// update's board file and `live_dtb`, the MPC's running tree
    /// (collect-over-ssh.sh --kexec-preflight's live.dtb).
    Chain { live_dtb: Vec<u8> },
}

impl Route {
    /// Its name in the recipe's routes (None: the soft-reboot recipe itself).
    pub fn recipe_name(&self) -> Option<&'static str> {
        match self {
            Route::SoftReboot => None,
            Route::KexecFile { loadpin_off: false } => Some("kexec-file"),
            Route::KexecFile { loadpin_off: true } => Some("kexec-file-loadpin-off"),
            Route::Chain { .. } => Some("chain"),
        }
    }

    /// The command-line flags that ask for it.
    pub fn flags(&self) -> &'static str {
        match self {
            Route::SoftReboot => "",
            Route::KexecFile { loadpin_off: false } => "--kexec-file",
            Route::KexecFile { loadpin_off: true } => "--kexec-file --loadpin-off",
            Route::Chain { .. } => "--kexec-file --loadpin-off --chain",
        }
    }
}

/// What was made.
pub struct Made {
    pub layout: Layout,
    pub values: Values,
    pub notes: Vec<String>,
    /// SHA-256 of everything before partition 4 as written.
    pub head_sha256: [u8; 32],
}

/// Writes partitions 1, 2, 3 and 5 into `head` (the disk's first 1 GiB),
/// laid out for a disk of `disk_sectors` whose partition 4 ends at
/// `data_end` (None: the end of the disk). The partition table is not
/// written here.
pub fn write_head<W: WriteAt + ReadAt>(
    b: &Build,
    disk_sectors: u64,
    data_end: Option<u64>,
    head: &W,
    say: &mut dyn FnMut(Msg),
) -> io::Result<Made> {
    let src = b.source;
    let v = Values::new(&src.release, src.rotation);
    let fs = Ext4::open(File::open(&src.rootfs)?)?;
    say(Msg::Progress(0.0, "Reading Akai's system".into()));
    let mut tree = copy_tree(&fs)?;
    let read = |p: &str| -> io::Result<Vec<u8>> {
        let i = fs
            .lookup(p)?
            .ok_or_else(|| bad(format!("Akai's root filesystem has no {p}")))?;
        fs.read_file(&i)
    };
    let mut about = format!(
        "MPFree stick, made by {}\nfrom Akai build {} (kernel {})\nmade: {} (seconds since 1970)\ndeveloper SSH: {}\n",
        b.creator,
        src.update.build,
        src.release,
        v.time,
        if b.ssh.is_some() { "on (port 2222, key only)" } else { "off" }
    );
    if b.choices.new_backup {
        about.push_str("new backup at the first start: asked\n");
    }
    if b.choices.akai_kits {
        about.push_str("Akai's kits at the first start: asked\n");
    }
    if !b.choices.left_out().is_empty() {
        about.push_str(&format!(
            "left out of the first start: {}\n",
            b.choices.left_out_text()
        ));
    }
    if b.choices.route != Route::SoftReboot {
        about.push_str(&format!(
            "route: {} (developer option)\n",
            b.choices.route.flags()
        ));
    }
    let mut notes = apply(&mut tree, &read, &v, b.programs, b.ssh, &about, &b.choices)?;
    let root_opts = Options {
        label: fs.label.clone(),
        uuid: gpt::random16(),
        time: v.time,
        size: Size::Fit {
            spare_bytes: 16 << 20,
            spare_inodes: 4096,
        },
    };
    let root_plan = ext2w::plan(&tree, &root_opts)?;
    let kits = kits_tree(b.kits, v.time)?;
    let kits_opts = Options {
        label: "MPFREE-KITS".into(),
        uuid: gpt::random16(),
        time: v.time,
        size: Size::Fit {
            spare_bytes: 1 << 20,
            spare_inodes: 16,
        },
    };
    let kits_plan = ext2w::plan(&kits, &kits_opts)?;
    let lay = layout(root_plan.bytes, kits_plan.bytes, disk_sectors, data_end, &v)?;
    notes.push(format!(
        "partition 1: {} MiB, {} inodes; partition 3: {} KiB",
        root_plan.bytes >> 20,
        root_plan.inodes,
        kits_plan.bytes >> 10
    ));
    let window = |n: usize| {
        let p = lay.part(n);
        Offset {
            inner: head,
            start: p.start_bytes(),
            len: p.len_bytes(),
        }
    };
    say(Msg::Progress(
        0.1,
        "Writing MPFree's system (partition 1)".into(),
    ));
    ext2w::write(&tree, &root_opts, &window(1))?;
    drop(tree);
    say(Msg::Progress(
        0.8,
        "Writing the log and kits partitions".into(),
    ));
    let r = Recipe::load();
    let log = Tree::new(Meta::new(0o755, 0, 0, v.time));
    ext2w::write(
        &log,
        &Options {
            label: "MPFREE-LOG".into(),
            uuid: gpt::random16(),
            time: v.time,
            size: Size::Bytes(r.log_size),
        },
        &window(2),
    )?;
    ext2w::write(&kits, &kits_opts, &window(3))?;
    say(Msg::Progress(0.85, "Writing the setup partition".into()));
    match b.dev {
        Some(d) => notes.extend(d.report()),
        None => notes.push("developer tools: none (no --dev-tools)".into()),
    }
    let setup = setup_partition(&src.release, "install", b.dev)?;
    window(5).write_at(&setup, 0)?;
    // Hash what is in the head now, for the read-back.
    say(Msg::Progress(0.9, "Checking what was written".into()));
    let end = lay.part(5).last + 1;
    let head_sha256 = sha256_of(head, 0, end * SECTOR)?;
    Ok(Made {
        layout: lay,
        values: v,
        notes,
        head_sha256,
    })
}

/// SHA-256 of `len` bytes from `offset`.
pub fn sha256_of<R: ReadAt + ?Sized>(r: &R, offset: u64, len: u64) -> io::Result<[u8; 32]> {
    let mut h = Sha256::new();
    hash_into(&mut h, r, offset, len)?;
    Ok(h.finalize().into())
}

/// Adds `len` bytes from `offset` to `h`.
fn hash_into<R: ReadAt + ?Sized>(h: &mut Sha256, r: &R, offset: u64, len: u64) -> io::Result<()> {
    let mut buf = vec![0u8; 1 << 20];
    let mut at = 0;
    while at < len {
        let n = ((len - at) as usize).min(buf.len());
        r.read_at(&mut buf[..n], offset + at)?;
        h.update(&buf[..n]);
        at += n as u64;
    }
    Ok(())
}

/// Partition 4 of a disk already holding an MPFree data partition at 1 GiB
/// (by name, start and its ext4's label): where it ends. None otherwise.
pub fn existing_data<R: ReadAt + ?Sized>(disk: &R) -> Option<u64> {
    let r = Recipe::load();
    let table = gpt::read(disk).ok()?;
    let p = table.part(4)?;
    if p.name != r.data_name || p.first != r.data_start_sectors {
        return None;
    }
    let mut sb = [0u8; 1024];
    disk.read_at(&mut sb, p.start_bytes() + 1024).ok()?;
    let magic = u16::from_le_bytes([sb[56], sb[57]]);
    let label = String::from_utf8_lossy(&sb[120..136])
        .trim_end_matches('\0')
        .to_string();
    (magic == 0xEF53 && label == DATA_LABEL).then_some(p.last)
}

/// Whether the stick's partition 4, wherever the table puts it, starts with
/// an ext2/3/4 filesystem: saved data that a remake keeping partition 4
/// must not lose when [`existing_data`] can't keep it (another place or
/// label, from another version). With no table to read (another tool
/// cleared it), where MPFree puts partition 4 (1 GiB) is looked at instead,
/// so its data still counts.
pub fn holds_data<R: ReadAt + ?Sized>(disk: &R) -> bool {
    let start = match gpt::read(disk) {
        Ok(t) => match t.part(4) {
            Some(p) => p.start_bytes(),
            None => return false,
        },
        Err(_) => Recipe::load().data_start_sectors * SECTOR,
    };
    let mut magic = [0u8; 2];
    disk.read_at(&mut magic, start + 1024 + 56).is_ok() && u16::from_le_bytes(magic) == 0xEF53
}

/// Marks partition 4 for the stick's first start: zeros its first MiB and
/// writes MPFREE-FORMAT-ME and the token at its start.
pub fn mark_data<W: WriteAt + ?Sized>(disk: &W, lay: &Layout, token: &str) -> io::Result<()> {
    let mut b = vec![0u8; 1 << 20];
    let mark = format!("{MARK}{token}");
    b[..mark.len()].copy_from_slice(mark.as_bytes());
    disk.write_at(&b, lay.part(4).start_bytes())
}

/// Makes a stick image file (partition 4 small and marked). For tests and
/// for writing with another tool; a stick written by this app keeps its
/// partition 4.
pub fn build_image(b: &Build, out: &Path, say: &mut dyn FnMut(Msg)) -> io::Result<Made> {
    let r = Recipe::load();
    let sectors = r.data_start_sectors + IMAGE_DATA / SECTOR + ALIGN;
    let f = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(out)?;
    f.set_len(sectors * SECTOR)?;
    let made = write_head(b, sectors, None, &f, say)?;
    gpt::write(&f, sectors, &made.layout.table)?;
    mark_data(&f, &made.layout, &made.values.token)?;
    f.sync_all()?;
    say(Msg::Progress(1.0, "Done".into()));
    Ok(made)
}

/// A disk to write, opened read-write, and its size.
pub trait Disk: ReadAt + WriteAt {
    fn sync(&self) -> io::Result<()>;
}

impl Disk for File {
    fn sync(&self) -> io::Result<()> {
        self.sync_all()
    }
}

/// Writes the stick onto `disk` (`bytes` long): partitions 1, 2, 3 and 5
/// from a scratch file in `work`, read back and compared, then the
/// partition table, then partition 4: kept if it is an MPFree data
/// partition (and `fresh_data` is not asked), marked otherwise. Until the
/// whole table goes on, the stick's table shows partition 4 alone, so a
/// write cut off part way can be run again and still keeps partition 4.
pub fn write_disk<D: Disk + ?Sized>(
    b: &Build,
    disk: &D,
    bytes: u64,
    work: &Path,
    fresh_data: bool,
    say: &mut dyn FnMut(Msg),
) -> io::Result<Made> {
    if bytes < MIN_STICK {
        return Err(bad(format!(
            "the stick is {:.1} GB; MPFree needs 16 GB or more (32 GB recommended)",
            bytes as f64 / 1e9
        )));
    }
    let r = Recipe::load();
    let sectors = bytes / SECTOR;
    let keep = if fresh_data {
        None
    } else {
        existing_data(disk)
    };
    say(Msg::Note(match keep {
        Some(_) => KEPT_NOTE.into(),
        None => {
            "partition 4: new and empty (MPFree makes its filesystem at the first start)".into()
        }
    }));
    let head_len = r.data_start_sectors * SECTOR;
    std::fs::create_dir_all(work)?;
    let scratch_path = work.join(format!("mpfree-stick-head-{}.img", std::process::id()));
    let scratch = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&scratch_path)?;
    let result = (|| {
        scratch.set_len(head_len)?;
        let mut sub = |m: Msg| match m {
            Msg::Progress(f, t) => say(Msg::Progress(f * 0.45, t)),
            other => say(other),
        };
        let made = write_head(b, sectors, keep, &scratch, &mut sub)?;
        let end = (made.layout.part(5).last + 1) * SECTOR;
        // While partitions 1, 2, 3 and 5 are copied, the stick's table shows
        // partition 4 alone, where it already is and never written here:
        // no system sees a half-written partition, and a copy cut off part
        // way leaves partition 4 where the next run finds and keeps it.
        // (Copying over the table cleared it, and the next run took
        // partition 4 for new.) The whole table goes on after the read-back.
        let only_data = Table {
            disk_guid: made.layout.table.disk_guid,
            parts: vec![None, None, None, Some(made.layout.part(4).clone())],
        };
        // Nothing has touched the stick before this line.
        say(Msg::Progress(0.45, "Writing the stick".into()));
        gpt::write(disk, sectors, &only_data)?;
        disk.sync()?;
        // Then the copy, after the table's sectors, and the read-back.
        let table_end = gpt::usable(sectors).0 * SECTOR;
        let mut buf = vec![0u8; 4 << 20];
        let mut at = table_end;
        while at < end {
            let n = ((end - at) as usize).min(buf.len());
            scratch.read_at(&mut buf[..n], at)?;
            disk.write_at(&buf[..n], at)?;
            at += n as u64;
            say(Msg::Progress(
                0.45 + 0.4 * at as f32 / end as f32,
                "Writing the stick".into(),
            ));
        }
        disk.sync()?;
        say(Msg::Progress(0.86, "Reading the stick back".into()));
        // The head's table sectors (which the new table replaces) from the
        // scratch file, the rest as the stick reads back.
        let mut h = Sha256::new();
        hash_into(&mut h, &scratch, 0, table_end)?;
        hash_into(&mut h, disk, table_end, end - table_end)?;
        let got: [u8; 32] = h.finalize().into();
        if got != made.head_sha256 {
            return Err(bad(
                "the stick does not read back as written: try another stick or USB port",
            ));
        }
        gpt::write(disk, sectors, &made.layout.table)?;
        if keep.is_none() {
            mark_data(disk, &made.layout, &made.values.token)?;
        }
        disk.sync()?;
        let table = gpt::read(disk)?;
        if table
            .parts
            .iter()
            .flatten()
            .map(|p| (p.first, p.last, &p.name))
            .collect::<Vec<_>>()
            != made
                .layout
                .table
                .parts
                .iter()
                .flatten()
                .map(|p| (p.first, p.last, &p.name))
                .collect::<Vec<_>>()
        {
            return Err(bad(
                "the stick's partition table does not read back as written",
            ));
        }
        say(Msg::Progress(1.0, "Done".into()));
        Ok(made)
    })();
    drop(scratch);
    let _ = std::fs::remove_file(&scratch_path);
    result
}

/// The newest eMMC backup on partition 4 of a stick, copied into `to` and
/// checked against its SHA256SUMS. Returns the new folder.
pub fn copy_backup<R: ReadAt + ?Sized>(
    disk: &R,
    to: &Path,
    say: &mut dyn FnMut(Msg),
) -> io::Result<PathBuf> {
    let r = Recipe::load();
    let table = gpt::read(disk)?;
    let (_, p) = table
        .by_name(&r.data_name)
        .ok_or_else(|| bad("this is not an MPFree stick (no partition 4)"))?;
    let fs = Ext4::open(Offset {
        inner: disk,
        start: p.start_bytes(),
        len: p.len_bytes(),
    })?;
    let backup = fs.lookup("/backup")?.ok_or_else(|| {
        bad("there is no backup on this stick yet: start MPFree on the MPC with the stick in first")
    })?;
    let mut names: Vec<String> = fs
        .read_dir(&backup)?
        .into_iter()
        .filter_map(|e| String::from_utf8(e.name).ok())
        .filter(|n| n.starts_with("mpc-one-emmc-") && !n.ends_with(".part"))
        .collect();
    names.sort();
    let name = names
        .pop()
        .ok_or_else(|| bad("there is no finished backup on this stick yet"))?;
    let dir = fs
        .lookup(&format!("/backup/{name}"))?
        .ok_or_else(|| bad("the backup folder went away"))?;
    let dest = to.join(&name);
    if dest.exists() {
        return Err(bad(format!("{} exists already", dest.display())));
    }
    let part = to.join(format!("{name}.part"));
    std::fs::create_dir_all(&part)?;
    let entries = fs.read_dir(&dir)?;
    let total: u64 = entries
        .iter()
        .filter_map(|e| fs.inode(e.ino).ok())
        .map(|i| i.size)
        .sum();
    let mut done = 0u64;
    for e in &entries {
        let fname = String::from_utf8_lossy(&e.name).into_owned();
        if fname.contains('/') || fname.starts_with('.') {
            continue;
        }
        let ino = fs.inode(e.ino)?;
        if ino.kind() != Some(Kind::File) {
            continue;
        }
        let mut out = io::BufWriter::with_capacity(1 << 20, File::create(part.join(&fname))?);
        let mut prog = Progressing {
            w: &mut out,
            done: &mut done,
            total,
            say: &mut *say,
        };
        fs.copy_file(&ino, &mut prog)?;
        out.flush()?;
    }
    say(Msg::Progress(0.95, "Checking the copy".into()));
    let sums = std::fs::read_to_string(part.join("SHA256SUMS"))
        .map_err(|_| bad("the backup has no SHA256SUMS"))?;
    let mut n = 0;
    for line in sums.lines().filter(|l| !l.trim().is_empty()) {
        let (sum, file) = line
            .split_once("  ")
            .ok_or_else(|| bad("SHA256SUMS has a line it can't read"))?;
        let got = update::file_sha256(&part.join(file.trim()))?;
        if update::hex(&got) != sum.trim() {
            return Err(bad(format!("{file} does not match its SHA-256: the copy is not good (the stick may be failing)")));
        }
        n += 1;
    }
    if n == 0 {
        return Err(bad("SHA256SUMS lists nothing"));
    }
    std::fs::rename(&part, &dest)?;
    say(Msg::Note(format!(
        "copied and checked: {} ({n} images)",
        dest.display()
    )));
    say(Msg::Progress(1.0, "Done".into()));
    Ok(dest)
}

struct Progressing<'a, W: Write> {
    w: &'a mut W,
    done: &'a mut u64,
    total: u64,
    say: &'a mut dyn FnMut(Msg),
}

impl<W: Write> Write for Progressing<'_, W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.w.write(buf)?;
        let before = *self.done;
        *self.done += n as u64;
        if before >> 26 != *self.done >> 26 {
            (self.say)(Msg::Progress(
                0.95 * *self.done as f32 / self.total.max(1) as f32,
                format!(
                    "Copying the backup: {:.1} of {:.1} GB",
                    *self.done as f64 / 1e9,
                    self.total as f64 / 1e9
                ),
            ));
        }
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.w.flush()
    }
}

/// The setup partition of a stick, read whole.
pub fn read_setup<R: ReadAt + ?Sized>(disk: &R) -> io::Result<(Part, Vec<u8>)> {
    let table = gpt::read(disk)?;
    let (_, p) = table
        .by_name(SETUP_NAME)
        .ok_or_else(|| bad("this is not an MPFree stick (no setup partition)"))?;
    let mut img = vec![0u8; p.len_bytes() as usize];
    disk.read_at(&mut img, p.start_bytes())?;
    Ok((p.clone(), img))
}

/// Partition 1 of an MPFree stick, under either of its names, with its
/// number (from 1).
pub fn root_part(table: &Table) -> Option<(usize, &Part)> {
    table
        .by_name(ROOT_NAME)
        .or_else(|| table.by_name(ROOT_OFF_NAME))
}

/// Sets what the stick's setup script does on the MPC, on a disk of
/// `disk_bytes`: "install" or "remove" in mode.txt (writing back only the
/// parts that changed), and partition 1 named [`ROOT_NAME`] for install or
/// [`ROOT_OFF_NAME`] for remove, so that with the hook on the MPC the stick
/// can be in at power-on and Akai's system still starts. Only the partition
/// table's name field and mode.txt change; "install" undoes "remove".
pub fn set_mode<D: Disk + ?Sized>(disk: &D, disk_bytes: u64, mode: &str) -> io::Result<()> {
    let name = match mode {
        "install" => ROOT_NAME,
        "remove" => ROOT_OFF_NAME,
        _ => return Err(bad("the mode is install or remove")),
    };
    let mut table = gpt::read(disk)?;
    let n = root_part(&table)
        .ok_or_else(|| bad("this is not an MPFree stick (no partition 1 named mpfree-test-root)"))?
        .0;
    if let Some(Some(p)) = table.parts.get_mut(n - 1) {
        if p.name != name {
            p.name = name.to_string();
            gpt::write(disk, disk_bytes / SECTOR, &table)?;
        }
    }
    let (p, before) = read_setup(disk)?;
    let mut after = before.clone();
    setup_write(&mut after, "mode.txt", format!("{mode}\n").as_bytes())?;
    for (i, (a, b)) in after
        .chunks(64 << 10)
        .zip(before.chunks(64 << 10))
        .enumerate()
    {
        if a != b {
            disk.write_at(a, p.start_bytes() + (i as u64) * (64 << 10))?;
        }
    }
    disk.sync()
}

/// Opens a file as a disk (for image files and tests): read-write.
pub fn open_file_disk(path: &Path) -> io::Result<(File, u64)> {
    let mut f = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)?;
    let len = f.seek(SeekFrom::End(0))?;
    Ok((f, len))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kexec_paths_are_found_as_the_builder_finds_them() {
        // The answers of the builder's regular expression for the same bytes.
        for (b, want) in [
            (&b"\0/usr/sbin/kexec\0"[..], &["/usr/sbin/kexec"][..]),
            (
                b"/sbin/kexec /usr/sbin/kexec",
                &["/sbin/kexec", "/usr/sbin/kexec"],
            ),
            (b"/kexec", &[]),
            (b"//kexec", &["//kexec"]),
            (b"/usr/sbin/kexec-tools", &[]),
            (b"/usr/sbin/kexec/x", &["/usr/sbin/kexec"]),
            (b"a/b/kexec.x", &[]),
            (b"/a/kexec/b/kexec", &["/a/kexec/b/kexec"]),
            (b"no path here", &[]),
            (
                b"/usr/local/sbin/kexec;/bin/kexec",
                &["/bin/kexec", "/usr/local/sbin/kexec"],
            ),
        ] {
            assert_eq!(
                super::kexec_names(b),
                want,
                "{}",
                String::from_utf8_lossy(b)
            );
        }
    }

    #[test]
    fn app_user_lines_as_the_builder_makes_them() {
        let r = Recipe::load();
        let (p, g) = app_user_edits(
            &r,
            "root:x:0:0:root:/home/root:/bin/sh\n",
            "root:x:0:\naudio:x:29:pulse\nvideo:x:44:\ninput:x:101:\nsystemd-journal:x:190:\n",
        )
        .unwrap();
        assert_eq!(p, "root:x:0:0:root:/home/root:/bin/sh\nmpfree:x:960:960:MPFree pad player:/run/mpfree-app:/sbin/nologin\n");
        assert_eq!(
            g,
            "root:x:0:\naudio:x:29:pulse,mpfree\nvideo:x:44:mpfree\ninput:x:101:mpfree\nsystemd-journal:x:190:mpfree\nmpfree:x:960:\n"
        );
        assert!(app_user_edits(&r, "root:x:0:0:root:/home/root:/bin/sh\n", "root:x:0:\n").is_err());
        assert!(app_user_edits(
            &r,
            "mpfree:x:960:960::/:/bin/sh\n",
            "audio:x:1:\nvideo:x:2:\ninput:x:3:\n"
        )
        .is_err());
    }

    #[test]
    fn shadow_root_unlocked_for_keys_only() {
        assert_eq!(
            shadow_edit("root:!:1:0:99999:7:::\nd:*:1::::::")
                .unwrap()
                .unwrap(),
            "root:*:1:0:99999:7:::\nd:*:1::::::"
        );
        assert_eq!(shadow_edit("root:*:1:0:99999:7:::\n").unwrap(), None);
        assert!(shadow_edit("x:!:1\n").is_err());
    }

    #[test]
    fn the_setup_partition_holds_the_scripts() {
        let mut img = setup_partition("6.18.44-test", "install", None).unwrap();
        let autorun =
            String::from_utf8(setup_read(&mut img, "autorun.sh").unwrap().unwrap()).unwrap();
        assert!(autorun.contains("RELEASE='6.18.44-test'"));
        assert_eq!(
            setup_read(&mut img, "mode.txt").unwrap().unwrap(),
            b"install\n"
        );
        assert_eq!(
            setup_read(&mut img, "hook-install.sh").unwrap().unwrap(),
            recipe::SETUP_DRIVE[1].1.as_bytes(),
            "byte for byte"
        );
        setup_write(&mut img, "mode.txt", b"remove\n").unwrap();
        assert_eq!(
            setup_read(&mut img, "mode.txt").unwrap().unwrap(),
            b"remove\n"
        );
        assert_eq!(setup_read(&mut img, "missing.txt").unwrap(), None);
        // FAT32, labelled MPFREE.
        assert_eq!(&img[82..90], b"FAT32   ");
        assert_eq!(&img[71..82], b"MPFREE     ");
    }

    #[test]
    fn layout_keeps_partition_4_at_1_gib() {
        let v = Values::new("r", 0);
        let lay = layout(470 << 20, 10 << 20, 31_000_000_000 / 512, None, &v).unwrap();
        let p: Vec<(u64, u64, &str)> = lay
            .table
            .parts
            .iter()
            .flatten()
            .map(|p| (p.first, p.last, p.name.as_str()))
            .collect();
        assert_eq!(p[0].0, 2048);
        assert_eq!(p[0].2, ROOT_NAME);
        assert_eq!(p[1].0, p[0].1 + 1);
        assert_eq!(p[3].0, 2 * 1024 * 1024);
        assert_eq!(p[3].1, 31_000_000_000 / 512 - 34);
        assert_eq!(p[4].2, SETUP_NAME);
        assert!(p[4].1 < p[3].0);
        assert_eq!(uuid_text(&lay.part(4).guid), v.data_uuid);
        assert!(
            layout(900 << 20, 10 << 20, 31_000_000_000 / 512, None, &v).is_err(),
            "too big for the first GiB"
        );
    }
}
