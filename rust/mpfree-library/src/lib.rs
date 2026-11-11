//! The user's library: projects, samples, kits, patterns and recordings on
//! the test stick, and the Trash.
//!
//! It lives in `library/` inside the data folder the app is given with
//! `--data` (on the MPC, the stick's MPFREE-DATA partition at
//! `/run/mpfree-data`; see the storage spec's "Proposed" section). The
//! store's `settings/` and `files/` sit beside it, untouched.
//!
//! ```text
//! library/
//!   Projects/  Samples/  Kits/  Patterns/  Recordings/   the places
//!   .trash/<id>/<name>  .trash/<id>/origin.txt           deleted items
//! ```
//!
//! - **Nothing outside `library/` is ever written,** and nothing is reached
//!   through a symbolic link: a [`Loc`] is a place plus checked names
//!   ([`valid_name`]), never a typed path, and every folder on the way is
//!   checked to be a real folder before anything is read or written.
//! - **Whole or nothing:** a copied file is written to `NAME.part`, synced
//!   and renamed; a copied folder is built under a hidden name and renamed.
//!   A copy that is cut off leaves no half file under a real name.
//! - **Delete goes to the Trash** ([`Library::delete`]), from which an item
//!   can be put back ([`Library::restore`]) until the Trash is emptied.
//! - **Off the UI and audio threads:** [`Worker`] runs every operation on
//!   one thread named `mpfree-library` and hands back what happened.

#![forbid(unsafe_code)]

pub mod audio;
pub mod name;
pub mod project;
pub mod worker;

pub use audio::{read_info, AudioFormat, AudioInfo};
pub use name::{clean_name, natural_cmp, unique_folder_name, unique_name, valid_name};
pub use project::{Meta, Part, Snapshot, Version};
pub use worker::{Done, Job, Outcome, Worker};

use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// The folder inside the data folder.
pub const LIBRARY: &str = "library";
/// The Trash's folder inside the library.
pub const TRASH: &str = ".trash";
/// The largest file the library copies.
pub const MAX_FILE: u64 = 2 << 30;
/// The most entries a folder listing returns.
pub const MAX_LIST: usize = 5000;
/// A project folder holds this file (written by `mpfree-project`).
pub const PROJECT_FILE: &str = "project.txt";

/// The library's top-level places.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Place {
    Projects,
    Samples,
    Kits,
    Patterns,
    Recordings,
    Trash,
}

impl Place {
    /// The places a person keeps things in, in the order the browser lists
    /// them (the Trash is apart).
    pub const ALL: [Place; 5] = [
        Place::Projects,
        Place::Samples,
        Place::Kits,
        Place::Patterns,
        Place::Recordings,
    ];

    /// Its folder's name in `library/`.
    pub fn folder(self) -> &'static str {
        match self {
            Place::Projects => "Projects",
            Place::Samples => "Samples",
            Place::Kits => "Kits",
            Place::Patterns => "Patterns",
            Place::Recordings => "Recordings",
            Place::Trash => TRASH,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Place::Trash => "Trash",
            p => p.folder(),
        }
    }

    pub fn from_label(s: &str) -> Option<Place> {
        Place::ALL
            .into_iter()
            .chain([Place::Trash])
            .find(|p| p.label() == s)
    }
}

/// A file or folder in the library: a place, then checked names.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Loc {
    pub place: Place,
    names: Vec<String>,
}

impl Loc {
    /// The top of a place.
    pub fn top(place: Place) -> Loc {
        Loc {
            place,
            names: Vec::new(),
        }
    }

    /// `name` inside this folder.
    pub fn join(&self, name: &str) -> io::Result<Loc> {
        if !valid_name(name) {
            return Err(bad_name(name));
        }
        let mut names = self.names.clone();
        names.push(name.to_string());
        Ok(Loc {
            place: self.place,
            names,
        })
    }

    /// From a place and names, each checked.
    pub fn new(place: Place, names: &[&str]) -> io::Result<Loc> {
        names.iter().try_fold(Loc::top(place), |l, n| l.join(n))
    }

    /// The folder this is in; None at the top of a place.
    pub fn parent(&self) -> Option<Loc> {
        let mut names = self.names.clone();
        names.pop()?;
        Some(Loc {
            place: self.place,
            names,
        })
    }

    /// Its own name; the place's label at the top.
    pub fn name(&self) -> &str {
        self.names
            .last()
            .map(String::as_str)
            .unwrap_or(self.place.label())
    }

    pub fn names(&self) -> &[String] {
        &self.names
    }

    pub fn is_top(&self) -> bool {
        self.names.is_empty()
    }

    /// True if `other` is this or inside it.
    pub fn contains(&self, other: &Loc) -> bool {
        self.place == other.place && other.names.starts_with(&self.names)
    }

    /// `"Samples / Drums / Kick.wav"`.
    pub fn display(&self) -> String {
        std::iter::once(self.place.label())
            .chain(self.names.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" / ")
    }

    /// One line for the Trash's origin file: the place's label, then the
    /// names, separated by `/` (names never hold `/`).
    fn to_line(&self) -> String {
        std::iter::once(self.place.label())
            .chain(self.names.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join("/")
    }

    fn from_line(s: &str) -> Option<Loc> {
        let mut it = s.trim().split('/');
        let place = Place::from_label(it.next()?)?;
        let names: Vec<&str> = it.collect();
        Loc::new(place, &names).ok()
    }
}

/// What an entry is, from its name and, for a folder, whether it holds a
/// project file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Folder,
    Project,
    Audio(AudioFormat),
    Kit,
    Pattern,
    Other,
}

impl Kind {
    /// From a file's name (not a folder).
    pub fn of_file(name: &str) -> Kind {
        if let Some(f) = AudioFormat::from_name(name) {
            return Kind::Audio(f);
        }
        match name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()) {
            Some(e) if e == "kit" => Kind::Kit,
            Some(e) if e == "pattern" || e == "arp" => Kind::Pattern,
            _ => Kind::Other,
        }
    }

    pub fn is_folder(self) -> bool {
        matches!(self, Kind::Folder | Kind::Project)
    }
}

/// One row of a folder listing.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub name: String,
    pub kind: Kind,
    /// A file's size; 0 for a folder.
    pub bytes: u64,
    /// Seconds since 1970 (UTC), or 0 if unknown.
    pub modified: u64,
}

/// An item in the Trash.
#[derive(Clone, Debug, PartialEq)]
pub struct TrashItem {
    /// Its folder in `.trash/`.
    pub id: String,
    pub name: String,
    pub kind: Kind,
    /// Where it was.
    pub from: Option<Loc>,
    /// Seconds since 1970 (UTC).
    pub deleted: u64,
}

fn bad_name(name: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("{name:?} is not a name the library takes"),
    )
}

fn err(kind: io::ErrorKind, s: String) -> io::Error {
    io::Error::new(kind, s)
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn modified(m: &fs::Metadata) -> u64 {
    m.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Syncs a folder, so a rename in it is on the disk.
fn sync_dir(dir: &Path) -> io::Result<()> {
    File::open(dir)?.sync_all()
}

fn mkdir(p: &Path) -> io::Result<()> {
    DirBuilder::new().mode(0o755).create(p)
}

/// A real folder, not a link to one.
fn is_real_dir(p: &Path) -> bool {
    fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_dir())
}

/// The library in a data folder.
#[derive(Clone, Debug)]
pub struct Library {
    root: PathBuf,
}

impl Library {
    /// The library in `data` (the folder given with `--data`): makes
    /// `library/`, the places and the Trash if they are not there.
    /// Refuses a `library` that is a link or a file.
    pub fn at(data: &Path) -> io::Result<Library> {
        if !is_real_dir(data) {
            return Err(err(
                io::ErrorKind::NotFound,
                format!("{} is not a folder", data.display()),
            ));
        }
        let root = data.join(LIBRARY);
        Self::ensure_dir(&root)?;
        let lib = Library { root };
        for p in Place::ALL.into_iter().chain([Place::Trash]) {
            Self::ensure_dir(&lib.root.join(p.folder()))?;
        }
        Ok(lib)
    }

    fn ensure_dir(p: &Path) -> io::Result<()> {
        match fs::symlink_metadata(p) {
            Ok(m) if m.file_type().is_dir() => Ok(()),
            Ok(_) => Err(err(
                io::ErrorKind::AlreadyExists,
                format!("{} is there but is not a folder", p.display()),
            )),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                mkdir(p)?;
                if let Some(parent) = p.parent() {
                    sync_dir(parent)?;
                }
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    /// The `library/` folder.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The path of `loc`. Every folder on the way must be a real folder;
    /// the last part may be anything, or nothing yet.
    fn path(&self, loc: &Loc) -> io::Result<PathBuf> {
        let mut p = self.root.join(loc.place.folder());
        if !is_real_dir(&p) {
            return Err(err(
                io::ErrorKind::NotFound,
                format!("{} is missing", loc.place.label()),
            ));
        }
        let n = loc.names.len();
        for (i, name) in loc.names.iter().enumerate() {
            if !valid_name(name) {
                return Err(bad_name(name));
            }
            p.push(name);
            if i + 1 < n && !is_real_dir(&p) {
                return Err(err(
                    io::ErrorKind::NotFound,
                    format!("no folder {}", loc.display()),
                ));
            }
        }
        Ok(p)
    }

    /// The path of an existing file or folder that is not a link.
    fn existing(&self, loc: &Loc) -> io::Result<(PathBuf, fs::Metadata)> {
        let p = self.path(loc)?;
        let m = fs::symlink_metadata(&p)
            .map_err(|e| err(e.kind(), format!("{}: {e}", loc.display())))?;
        if m.file_type().is_symlink() {
            return Err(err(
                io::ErrorKind::InvalidInput,
                format!(
                    "{} is a link; the library does not follow links",
                    loc.display()
                ),
            ));
        }
        Ok((p, m))
    }

    fn exists(&self, loc: &Loc) -> bool {
        self.path(loc)
            .is_ok_and(|p| fs::symlink_metadata(p).is_ok())
    }

    /// What a folder holds: folders and projects first, then files, each
    /// in natural order. Hidden files, unfinished `.part` files, links and
    /// names the library would not take are left out. At most
    /// [`MAX_LIST`] entries.
    pub fn list(&self, folder: &Loc) -> io::Result<Vec<Entry>> {
        let (p, m) = self.existing(folder)?;
        if !m.is_dir() {
            return Err(err(
                io::ErrorKind::InvalidInput,
                format!("{} is not a folder", folder.display()),
            ));
        }
        let mut out = Vec::new();
        for e in fs::read_dir(&p)? {
            let e = e?;
            let Some(name) = e.file_name().to_str().map(str::to_string) else {
                continue;
            };
            if !valid_name(&name) {
                continue;
            }
            let Ok(m) = e.metadata() else { continue };
            let ft = m.file_type();
            let kind = if ft.is_dir() {
                if is_project(&e.path()) {
                    Kind::Project
                } else {
                    Kind::Folder
                }
            } else if ft.is_file() {
                Kind::of_file(&name)
            } else {
                continue;
            };
            out.push(Entry {
                bytes: if ft.is_file() { m.len() } else { 0 },
                modified: modified(&m),
                name,
                kind,
            });
            if out.len() >= MAX_LIST {
                break;
            }
        }
        out.sort_by(|a, b| {
            b.kind
                .is_folder()
                .cmp(&a.kind.is_folder())
                .then_with(|| natural_cmp(&a.name, &b.name))
        });
        Ok(out)
    }

    /// An audio file's header.
    pub fn audio_info(&self, loc: &Loc) -> io::Result<AudioInfo> {
        let (p, m) = self.existing(loc)?;
        if !m.is_file() {
            return Err(err(
                io::ErrorKind::InvalidInput,
                format!("{} is not a file", loc.display()),
            ));
        }
        read_info(&mut io::BufReader::new(File::open(p)?))
    }

    /// A whole file, if it is at most `max` bytes (a project file, a
    /// pattern; the sampler reads audio itself).
    pub fn read(&self, loc: &Loc, max: u64) -> io::Result<Vec<u8>> {
        let (p, m) = self.existing(loc)?;
        if !m.is_file() || m.len() > max {
            return Err(err(
                io::ErrorKind::InvalidInput,
                format!("{} is not a file of at most {max} bytes", loc.display()),
            ));
        }
        let mut v = Vec::with_capacity(m.len() as usize);
        File::open(p)?.take(max + 1).read_to_end(&mut v)?;
        Ok(v)
    }

    /// The real path of an existing file, to hand to a loader (the
    /// sampler) that only reads it.
    pub fn file_path(&self, loc: &Loc) -> io::Result<PathBuf> {
        let (p, m) = self.existing(loc)?;
        if m.is_file() {
            Ok(p)
        } else {
            Err(err(
                io::ErrorKind::InvalidInput,
                format!("{} is not a file", loc.display()),
            ))
        }
    }

    /// A new folder in `parent`, named `name` or `name 2` ... if taken.
    pub fn new_folder(&self, parent: &Loc, name: &str) -> io::Result<Loc> {
        self.check_writable(parent)?;
        let (pp, _) = self.existing(parent)?;
        let name = unique_folder_name(name, |n| parent.join(n).is_ok_and(|l| self.exists(&l)));
        let loc = parent.join(&name)?;
        mkdir(&pp.join(&name))?;
        sync_dir(&pp)?;
        Ok(loc)
    }

    /// A new file in `folder` from whole bytes (a recording, a saved kit),
    /// named `name` or `name 2` ... if taken. Written to a `.part` file,
    /// synced and renamed, so it is there whole or not at all.
    pub fn write_new(&self, folder: &Loc, name: &str, bytes: &[u8]) -> io::Result<Loc> {
        self.check_writable(folder)?;
        if bytes.len() as u64 > MAX_FILE {
            return Err(err(io::ErrorKind::InvalidInput, "file too big".into()));
        }
        let (fp, _) = self.existing(folder)?;
        let name = unique_name(name, |n| folder.join(n).is_ok_and(|l| self.exists(&l)));
        let loc = folder.join(&name)?;
        let part = fp.join(format!("{name}.part"));
        let r = (|| {
            let mut f = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o644)
                .custom_flags(0x20000) // O_NOFOLLOW
                .open(&part)?;
            f.write_all(bytes)?;
            f.sync_all()?;
            fs::rename(&part, fp.join(&name))?;
            sync_dir(&fp)
        })();
        if r.is_err() {
            let _ = fs::remove_file(&part);
        }
        r.map(|_| loc)
    }

    /// Renames a file or folder in place. Fails if the new name is taken.
    pub fn rename(&self, loc: &Loc, new_name: &str) -> io::Result<Loc> {
        self.check_writable(loc)?;
        if loc.is_top() {
            return Err(err(
                io::ErrorKind::InvalidInput,
                "a place cannot be renamed".into(),
            ));
        }
        let (p, _) = self.existing(loc)?;
        let parent = loc.parent().expect("not top");
        let to = parent.join(new_name)?;
        if to == *loc {
            return Ok(to);
        }
        // A change of case only is the same file on a FAT stick; allow it.
        if self.exists(&to) && !new_name.eq_ignore_ascii_case(loc.name()) {
            return Err(err(
                io::ErrorKind::AlreadyExists,
                format!("{} is already there", to.display()),
            ));
        }
        let tp = self.path(&to)?;
        fs::rename(&p, &tp)?;
        sync_dir(tp.parent().expect("has parent"))?;
        Ok(to)
    }

    /// Copies a file or folder into `folder`, keeping its name or adding
    /// ` 2` ... if it is taken.
    pub fn copy(&self, from: &Loc, folder: &Loc) -> io::Result<Loc> {
        self.check_writable(folder)?;
        if from.is_top() {
            return Err(err(
                io::ErrorKind::InvalidInput,
                "a place cannot be copied".into(),
            ));
        }
        if from.contains(folder) {
            return Err(err(
                io::ErrorKind::InvalidInput,
                "a folder cannot be copied into itself".into(),
            ));
        }
        let (src, m) = self.existing(from)?;
        let (dst_dir, dm) = self.existing(folder)?;
        if !dm.is_dir() {
            return Err(err(io::ErrorKind::InvalidInput, "not a folder".into()));
        }
        let taken = |n: &str| folder.join(n).is_ok_and(|l| self.exists(&l));
        let name = if m.is_dir() {
            unique_folder_name(from.name(), taken)
        } else {
            unique_name(from.name(), taken)
        };
        let loc = folder.join(&name)?;
        let dst = dst_dir.join(&name);
        if m.is_dir() {
            let tmp = dst_dir.join(format!(".copy-{}-{name}", unique_id()));
            let r = copy_tree(&src, &tmp, 0).and_then(|_| fs::rename(&tmp, &dst));
            if r.is_err() {
                let _ = fs::remove_dir_all(&tmp);
            }
            r?;
        } else {
            let part = dst_dir.join(format!("{name}.part"));
            let r = copy_file(&src, &part).and_then(|_| fs::rename(&part, &dst));
            if r.is_err() {
                let _ = fs::remove_file(&part);
            }
            r?;
        }
        sync_dir(&dst_dir)?;
        Ok(loc)
    }

    /// Moves a file or folder into `folder` (same name, or ` 2` ...).
    pub fn move_to(&self, from: &Loc, folder: &Loc) -> io::Result<Loc> {
        self.check_writable(from)?;
        self.check_writable(folder)?;
        if from.is_top() {
            return Err(err(
                io::ErrorKind::InvalidInput,
                "a place cannot be moved".into(),
            ));
        }
        if from.contains(folder) {
            return Err(err(
                io::ErrorKind::InvalidInput,
                "a folder cannot be moved into itself".into(),
            ));
        }
        if from.parent().as_ref() == Some(folder) {
            return Ok(from.clone());
        }
        let (src, _) = self.existing(from)?;
        let (dst_dir, dm) = self.existing(folder)?;
        if !dm.is_dir() {
            return Err(err(io::ErrorKind::InvalidInput, "not a folder".into()));
        }
        let name = unique_name(from.name(), |n| {
            folder.join(n).is_ok_and(|l| self.exists(&l))
        });
        let loc = folder.join(&name)?;
        fs::rename(&src, dst_dir.join(&name))?;
        sync_dir(&dst_dir)?;
        sync_dir(src.parent().expect("has parent"))?;
        Ok(loc)
    }

    /// Moves a file or folder to the Trash, with a note of where it was.
    pub fn delete(&self, loc: &Loc) -> io::Result<TrashItem> {
        self.check_writable(loc)?;
        if loc.is_top() {
            return Err(err(
                io::ErrorKind::InvalidInput,
                "a place cannot be deleted".into(),
            ));
        }
        let (src, m) = self.existing(loc)?;
        let kind = if m.is_dir() {
            if is_project(&src) {
                Kind::Project
            } else {
                Kind::Folder
            }
        } else {
            Kind::of_file(loc.name())
        };
        let trash = self.root.join(TRASH);
        let id = unique_id();
        let slot = trash.join(&id);
        mkdir(&slot)?;
        let r = (|| {
            let mut f = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o644)
                .open(slot.join("origin.txt"))?;
            f.write_all(format!("{}\n{}\n", loc.to_line(), now()).as_bytes())?;
            f.sync_all()?;
            sync_dir(&slot)?;
            fs::rename(&src, slot.join(loc.name()))?;
            sync_dir(src.parent().expect("has parent"))?;
            sync_dir(&slot)
        })();
        if r.is_err() && fs::symlink_metadata(slot.join(loc.name())).is_err() {
            let _ = fs::remove_dir_all(&slot);
        }
        r?;
        Ok(TrashItem {
            id,
            name: loc.name().to_string(),
            kind,
            from: Some(loc.clone()),
            deleted: now(),
        })
    }

    /// What is in the Trash, newest first.
    pub fn trash(&self) -> io::Result<Vec<TrashItem>> {
        let trash = self.root.join(TRASH);
        let mut out = Vec::new();
        for e in fs::read_dir(&trash)? {
            let e = e?;
            let Some(id) = e.file_name().to_str().map(str::to_string) else {
                continue;
            };
            if !e.file_type().is_ok_and(|t| t.is_dir()) || id.starts_with('.') {
                continue;
            }
            if let Some(item) = self.trash_item(&trash.join(&id), &id) {
                out.push(item);
            }
            if out.len() >= MAX_LIST {
                break;
            }
        }
        out.sort_by(|a, b| b.deleted.cmp(&a.deleted).then_with(|| b.id.cmp(&a.id)));
        Ok(out)
    }

    fn trash_item(&self, slot: &Path, id: &str) -> Option<TrashItem> {
        let origin = fs::read_to_string(slot.join("origin.txt")).unwrap_or_default();
        let mut lines = origin.lines();
        let from = lines.next().and_then(Loc::from_line);
        let deleted = lines
            .next()
            .and_then(|l| l.trim().parse().ok())
            .unwrap_or(0);
        let (name, kind) = fs::read_dir(slot).ok()?.find_map(|e| {
            let e = e.ok()?;
            let name = e.file_name().to_str()?.to_string();
            if name == "origin.txt" || !valid_name(&name) {
                return None;
            }
            let t = e.file_type().ok()?;
            let kind = if t.is_dir() {
                if is_project(&e.path()) {
                    Kind::Project
                } else {
                    Kind::Folder
                }
            } else if t.is_file() {
                Kind::of_file(&name)
            } else {
                return None;
            };
            Some((name, kind))
        })?;
        Some(TrashItem {
            id: id.to_string(),
            name,
            kind,
            from,
            deleted,
        })
    }

    /// Puts an item back where it was (or, if that folder is gone, at the
    /// top of its place; or Samples if its place is unknown), with ` 2` ...
    /// if its name has been taken since.
    pub fn restore(&self, id: &str) -> io::Result<Loc> {
        if !valid_trash_id(id) {
            return Err(err(
                io::ErrorKind::InvalidInput,
                "no such item in the Trash".into(),
            ));
        }
        let slot = self.root.join(TRASH).join(id);
        let item = self
            .trash_item(&slot, id)
            .ok_or_else(|| err(io::ErrorKind::NotFound, "no such item in the Trash".into()))?;
        let mut folder = item
            .from
            .as_ref()
            .and_then(Loc::parent)
            .unwrap_or(Loc::top(Place::Samples));
        if folder.place == Place::Trash || !self.existing(&folder).is_ok_and(|(_, m)| m.is_dir()) {
            folder = Loc::top(folder.place);
            if folder.place == Place::Trash {
                folder = Loc::top(Place::Samples);
            }
        }
        let (dst_dir, _) = self.existing(&folder)?;
        let name = unique_name(&item.name, |n| {
            folder.join(n).is_ok_and(|l| self.exists(&l))
        });
        let loc = folder.join(&name)?;
        fs::rename(slot.join(&item.name), dst_dir.join(&name))?;
        sync_dir(&dst_dir)?;
        let _ = fs::remove_file(slot.join("origin.txt"));
        let _ = fs::remove_dir(&slot);
        let _ = sync_dir(&self.root.join(TRASH));
        Ok(loc)
    }

    /// Deletes everything in the Trash for good; the bytes freed.
    pub fn empty_trash(&self) -> io::Result<u64> {
        let trash = self.root.join(TRASH);
        let mut freed = 0;
        for e in fs::read_dir(&trash)? {
            let e = e?;
            let p = e.path();
            freed += tree_bytes(&p, 0);
            if e.file_type()?.is_dir() {
                fs::remove_dir_all(&p)?;
            } else {
                fs::remove_file(&p)?;
            }
        }
        sync_dir(&trash)?;
        Ok(freed)
    }

    /// Bytes used by a place (the Trash included), counted on the disk.
    pub fn bytes_in(&self, place: Place) -> u64 {
        tree_bytes(&self.root.join(place.folder()), 0)
    }

    fn check_writable(&self, loc: &Loc) -> io::Result<()> {
        if loc.place == Place::Trash {
            return Err(err(
                io::ErrorKind::PermissionDenied,
                "the Trash only takes deleted items; restore an item to change it".into(),
            ));
        }
        Ok(())
    }
}

fn is_project(dir: &Path) -> bool {
    fs::symlink_metadata(dir.join(PROJECT_FILE)).is_ok_and(|m| m.is_file())
}

fn valid_trash_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 40 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// Unique within this run and across runs: seconds, then a counter.
fn unique_id() -> String {
    static N: AtomicU64 = AtomicU64::new(0);
    format!(
        "{}-{}-{}",
        now(),
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    )
}

/// How deep a copied folder may go.
const MAX_DEPTH: usize = 16;

fn copy_file(src: &Path, dst: &Path) -> io::Result<()> {
    let mut i = OpenOptions::new()
        .read(true)
        .custom_flags(0x20000) // O_NOFOLLOW
        .open(src)?;
    if i.metadata()?.len() > MAX_FILE {
        return Err(err(
            io::ErrorKind::InvalidInput,
            "file too big to copy".into(),
        ));
    }
    let mut o = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o644)
        .open(dst)?;
    let mut buf = vec![0u8; 256 * 1024];
    let mut total = 0u64;
    loop {
        let n = i.read(&mut buf)?;
        if n == 0 {
            break;
        }
        total += n as u64;
        if total > MAX_FILE {
            return Err(err(
                io::ErrorKind::InvalidInput,
                "file too big to copy".into(),
            ));
        }
        o.write_all(&buf[..n])?;
    }
    o.sync_all()
}

/// Copies a folder's real files and folders (links are skipped).
fn copy_tree(src: &Path, dst: &Path, depth: usize) -> io::Result<()> {
    if depth > MAX_DEPTH {
        return Err(err(
            io::ErrorKind::InvalidInput,
            "folders too deep to copy".into(),
        ));
    }
    mkdir(dst)?;
    for e in fs::read_dir(src)? {
        let e = e?;
        let Some(name) = e.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if name.ends_with(".part") || name.starts_with(".copy-") {
            continue;
        }
        let t = e.file_type()?;
        if t.is_dir() {
            copy_tree(&e.path(), &dst.join(&name), depth + 1)?;
        } else if t.is_file() {
            copy_file(&e.path(), &dst.join(&name))?;
        }
    }
    sync_dir(dst)
}

fn tree_bytes(p: &Path, depth: usize) -> u64 {
    let Ok(m) = fs::symlink_metadata(p) else {
        return 0;
    };
    if m.is_file() {
        return m.len();
    }
    if !m.is_dir() || depth > MAX_DEPTH {
        return 0;
    }
    fs::read_dir(p)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .map(|e| tree_bytes(&e.path(), depth + 1))
                .sum()
        })
        .unwrap_or(0)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    pub fn temp() -> PathBuf {
        static N: AtomicUsize = AtomicUsize::new(0);
        let d = std::env::temp_dir().join(format!(
            "mpfree-library-{}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn names(v: &[Entry]) -> Vec<&str> {
        v.iter().map(|e| e.name.as_str()).collect()
    }

    #[test]
    fn a_new_library_has_its_places_and_nothing_else() {
        let d = temp();
        let lib = Library::at(&d).unwrap();
        let mut top: Vec<String> = fs::read_dir(&d)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        top.sort();
        assert_eq!(top, ["library"]);
        for p in Place::ALL {
            assert!(lib.list(&Loc::top(p)).unwrap().is_empty());
        }
        // Opening it again changes nothing.
        Library::at(&d).unwrap();
    }

    #[test]
    fn lists_put_folders_first_in_natural_order_and_hide_the_rest() {
        let d = temp();
        let lib = Library::at(&d).unwrap();
        let s = Loc::top(Place::Samples);
        lib.write_new(&s, "Kick 10.wav", &audio::wav_bytes(44_100, 1, 16, 10))
            .unwrap();
        lib.write_new(&s, "Kick 2.wav", &audio::wav_bytes(44_100, 1, 16, 10))
            .unwrap();
        lib.write_new(&s, "notes.txt", b"hi").unwrap();
        lib.new_folder(&s, "Drums").unwrap();
        let p = lib.new_folder(&s, "Song").unwrap();
        lib.write_new(&p, PROJECT_FILE, b"mpfree-project 1\n")
            .unwrap();
        let raw = lib.root().join("Samples");
        fs::write(raw.join(".hidden"), b"x").unwrap();
        fs::write(raw.join("half.wav.part"), b"x").unwrap();
        std::os::unix::fs::symlink("/etc", raw.join("etc")).unwrap();
        let l = lib.list(&s).unwrap();
        assert_eq!(
            names(&l),
            ["Drums", "Song", "Kick 2.wav", "Kick 10.wav", "notes.txt"]
        );
        assert_eq!(l[1].kind, Kind::Project);
        assert_eq!(l[2].kind, Kind::Audio(AudioFormat::Wav));
        assert_eq!(l[2].bytes, 64);
        assert_eq!(l[4].kind, Kind::Other);
    }

    #[test]
    fn audio_info_comes_from_the_header() {
        let d = temp();
        let lib = Library::at(&d).unwrap();
        let s = Loc::top(Place::Samples);
        let k = lib
            .write_new(&s, "Kick.wav", &audio::wav_bytes(48_000, 2, 24, 4800))
            .unwrap();
        let i = lib.audio_info(&k).unwrap();
        assert_eq!(
            (i.rate, i.channels, i.bits, i.frames),
            (48_000, 2, 24, 4800)
        );
    }

    #[test]
    fn names_never_leave_the_library() {
        let d = temp();
        let lib = Library::at(&d).unwrap();
        let s = Loc::top(Place::Samples);
        for bad in ["..", "../x", "/etc", ".trash", "a/b"] {
            assert!(s.join(bad).is_err(), "{bad}");
            assert!(lib.new_folder(&s, bad).is_err(), "{bad}");
            assert!(lib.write_new(&s, bad, b"x").is_err(), "{bad}");
        }
        // A link inside the library is not followed, for reads or writes.
        let outside = temp();
        std::os::unix::fs::symlink(&outside, lib.root().join("Samples").join("Out")).unwrap();
        let out = s.join("Out").unwrap();
        assert!(lib.list(&out).is_err());
        assert!(lib.write_new(&out, "x.wav", b"x").is_err());
        assert!(lib.new_folder(&out, "x").is_err());
        assert!(lib.delete(&out.join("x").unwrap()).is_err());
        assert_eq!(fs::read_dir(&outside).unwrap().count(), 0);
        // A library folder that is a link is refused.
        let d2 = temp();
        std::os::unix::fs::symlink(&outside, d2.join(LIBRARY)).unwrap();
        assert!(Library::at(&d2).is_err());
        assert_eq!(fs::read_dir(&outside).unwrap().count(), 0);
    }

    #[test]
    fn rename_copy_and_move() {
        let d = temp();
        let lib = Library::at(&d).unwrap();
        let s = Loc::top(Place::Samples);
        let kick = lib.write_new(&s, "kick.wav", b"RIFF").unwrap();
        let drums = lib.new_folder(&s, "Drums").unwrap();
        let kick = lib.rename(&kick, "Kick.wav").unwrap();
        assert_eq!(kick.name(), "Kick.wav");
        let other = lib.write_new(&s, "Other.wav", b"x").unwrap();
        assert!(lib.rename(&other, "Kick.wav").is_err());
        let copy = lib.copy(&kick, &s).unwrap();
        assert_eq!(copy.name(), "Kick 2.wav");
        let moved = lib.move_to(&copy, &drums).unwrap();
        assert_eq!(moved.display(), "Samples / Drums / Kick 2.wav");
        assert_eq!(lib.read(&moved, 100).unwrap(), b"RIFF");
        // A folder copy brings what is inside it.
        let dcopy = lib.copy(&drums, &Loc::top(Place::Kits)).unwrap();
        assert_eq!(names(&lib.list(&dcopy).unwrap()), ["Kick 2.wav"]);
        // Not into itself.
        assert!(lib.copy(&drums, &drums).is_err());
        assert!(lib.move_to(&s, &drums).is_err());
        let inner = lib.new_folder(&drums, "Inner").unwrap();
        assert!(lib.move_to(&drums, &inner).is_err());
        // Two names a folder could take: a new folder gets " 2".
        assert_eq!(lib.new_folder(&s, "Drums").unwrap().name(), "Drums 2");
    }

    #[test]
    fn delete_goes_to_the_trash_and_comes_back() {
        let d = temp();
        let lib = Library::at(&d).unwrap();
        let s = Loc::top(Place::Samples);
        let drums = lib.new_folder(&s, "Drums").unwrap();
        let kick = lib.write_new(&drums, "Kick.wav", b"abc").unwrap();
        let item = lib.delete(&kick).unwrap();
        assert!(lib.list(&drums).unwrap().is_empty());
        let t = lib.trash().unwrap();
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].name, "Kick.wav");
        assert_eq!(t[0].from.as_ref(), Some(&kick));
        // The Trash is not a place to put things.
        assert!(lib.write_new(&Loc::top(Place::Trash), "x", b"x").is_err());
        // Back where it was; if the name is taken, with " 2".
        lib.write_new(&drums, "Kick.wav", b"new").unwrap();
        let back = lib.restore(&item.id).unwrap();
        assert_eq!(back.display(), "Samples / Drums / Kick 2.wav");
        assert_eq!(lib.read(&back, 10).unwrap(), b"abc");
        assert!(lib.trash().unwrap().is_empty());
        // A folder whose folder is gone comes back at the top of its place.
        let snare = lib.write_new(&drums, "Snare.wav", b"s").unwrap();
        let item = lib.delete(&snare).unwrap();
        lib.delete(&drums).unwrap();
        assert_eq!(
            lib.restore(&item.id).unwrap().display(),
            "Samples / Snare.wav"
        );
        // Emptying frees the bytes for good.
        assert!(lib.empty_trash().unwrap() >= 3 + 3);
        assert!(lib.trash().unwrap().is_empty());
        assert!(lib.restore("../../etc").is_err());
    }

    #[test]
    fn a_cut_off_write_leaves_no_file_under_a_real_name() {
        let d = temp();
        let lib = Library::at(&d).unwrap();
        let s = Loc::top(Place::Recordings);
        // A folder where the .part file would go makes the write fail.
        fs::create_dir(lib.root().join("Recordings").join("Take.wav.part")).unwrap();
        assert!(lib.write_new(&s, "Take.wav", b"x").is_err());
        assert!(lib.list(&s).unwrap().is_empty());
    }

    #[test]
    fn locs_round_trip_through_the_trash_line() {
        let l = Loc::new(Place::Samples, &["Drums", "Kick 1.wav"]).unwrap();
        assert_eq!(Loc::from_line(&l.to_line()), Some(l.clone()));
        assert_eq!(l.parent().unwrap().display(), "Samples / Drums");
        assert!(Loc::top(Place::Samples).contains(&l));
        assert!(!l.contains(&Loc::top(Place::Samples)));
        assert_eq!(Loc::from_line("Nowhere/x"), None);
        assert_eq!(Loc::from_line("Samples/../x"), None);
    }
}
