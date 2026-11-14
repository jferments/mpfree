//! Projects: one folder each in the library's Projects place, complete in
//! itself and readable on any computer.
//!
//! ```text
//! Projects/Night drive/
//!   project.txt            "mpfree-project 1", then key = value lines
//!   parts/<owner>.txt      one text file per engine (transport, sampler, synth, mixer ...)
//!   samples/<hash>.<ext>   every sound the project uses, copied in, named by content
//!   versions/<id>/         earlier saves: their project.txt and parts/
//! ```
//!
//! - **The browser owns the folder; each engine owns its part.** An engine
//!   implements [`Part`] (its text out, its text in). The browser never
//!   reads a part. A part no engine claims (a newer build's, a removed
//!   engine's) is kept when the project is saved again
//!   ([`Snapshot::keep_unknown`]).
//! - **Whole or nothing.** A save writes the new parts to a hidden
//!   folder, syncs them, moves the old `parts/` into `versions/`, renames
//!   the new one into place and then replaces `project.txt` (written to a
//!   `.part` file and renamed). If power goes in between, opening the
//!   project finishes or undoes the save ([`Library::open_project`]): it
//!   never opens half of one.
//! - **Samples are copied in** and named by a 128-bit hash of their bytes
//!   and their length, so the same sound is stored once and a project
//!   never misses a sample because a drive is gone. Two different sounds
//!   with the same name are told apart by comparing their bytes.
//! - **Earlier saves are kept** ([`Library::versions`]), the newest
//!   `keep` of them, so going back is one tap.
//! - **Sounds an engine made** (a recording in RAM, a resample) come with
//!   the save as [`NewSample`]s, named by the engine for their content
//!   ([`Library::save_project_with`]). Each one the project's `samples/`
//!   doesn't have yet is written whole before any part is, so a part never
//!   names a sound that isn't there; one it has already is left as it is.

use crate::{name::valid_name, Library, Loc, Place, PROJECT_FILE};
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The first line of `project.txt`.
pub const FORMAT: &str = "mpfree-project 1";
/// The largest part file.
pub const MAX_PART: usize = 4 << 20;
/// The most parts a project holds.
pub const MAX_PARTS: usize = 64;
/// Earlier saves kept by default.
pub const KEEP_VERSIONS: usize = 10;
/// The project autosave writes to, at the top of Projects (Jay's pick, 14
/// Nov: one Autosave project). `Autosave 2` ... if a project of the
/// musician's own has the name ([`Library::autosave_project`]).
pub const AUTOSAVE: &str = "Autosave";
/// The project.txt key that marks autosave's project: `autosave = yes`.
pub const AUTOSAVE_KEY: &str = "autosave";

const PARTS: &str = "parts";
const NEW_PARTS: &str = ".parts-new";
const SAMPLES: &str = "samples";
const VERSIONS: &str = "versions";

/// An engine's piece of a project.
pub trait Part {
    /// Its part file's name: `[a-z0-9._-]`, as the store's owners.
    fn owner(&self) -> &str;
    /// Its state as text.
    fn save(&self) -> String;
    /// Its state from text it saved before (or a newer build saved). An
    /// error leaves the engine as it was; the project still opens.
    fn load(&mut self, text: &str) -> Result<(), String>;
}

/// A part file's owner: 1 to 64 of `[a-z0-9._-]`, not starting with `.`.
pub fn valid_owner(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && !s.starts_with('.')
        && s.bytes()
            .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'.' | b'_' | b'-'))
}

/// How a sound an engine holds is written out (the sampler's WAV writer).
pub trait SampleData: Send + Sync {
    fn write_to(&self, out: &mut dyn Write) -> io::Result<()>;
}

/// Any `Fn(&mut dyn Write)` writes a sound: the app wraps the sampler's
/// WAV writer in one (`move |w: &mut dyn Write| wav::write(&sound, w)`).
impl<F> SampleData for F
where
    F: Fn(&mut dyn Write) -> io::Result<()> + Send + Sync,
{
    fn write_to(&self, out: &mut dyn Write) -> io::Result<()> {
        self(out)
    }
}

/// A sound to keep in a project's `samples/` with a save.
#[derive(Clone)]
pub struct NewSample {
    /// Its file name there, given by the engine and the same for the same
    /// sound (the sampler names it by the sound's id), so a file already
    /// there is this sound and is not written again.
    pub file: String,
    pub data: Arc<dyn SampleData>,
}

impl NewSample {
    pub fn new(file: impl Into<String>, data: Arc<dyn SampleData>) -> NewSample {
        NewSample {
            file: file.into(),
            data,
        }
    }

    /// A sound already in a file (an audio track's take in its
    /// [`Library::work_dir`]), copied in whole with the save as any other
    /// (the project keeps its own copy). A link is not followed.
    pub fn file(file: impl Into<String>, path: PathBuf) -> NewSample {
        NewSample::new(
            file,
            Arc::new(move |w: &mut dyn Write| {
                let mut f = OpenOptions::new()
                    .read(true)
                    .custom_flags(0x20000) // O_NOFOLLOW
                    .open(&path)?;
                io::copy(&mut f, w).map(|_| ())
            }),
        )
    }
}

impl std::fmt::Debug for NewSample {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "NewSample({:?})", self.file)
    }
}

impl PartialEq for NewSample {
    fn eq(&self, o: &NewSample) -> bool {
        self.file == o.file && std::ptr::addr_eq(Arc::as_ptr(&self.data), Arc::as_ptr(&o.data))
    }
}

/// A file being written that may grow no bigger than [`crate::MAX_FILE`].
struct Capped<'a> {
    f: &'a mut File,
    n: u64,
}

impl Write for Capped<'_> {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        self.n += b.len() as u64;
        if self.n > crate::MAX_FILE {
            return Err(bad(io::ErrorKind::InvalidInput, "sample too big"));
        }
        self.f.write(b)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.f.flush()
    }
}

/// What `project.txt` says.
#[derive(Clone, Debug, PartialEq)]
pub struct Meta {
    pub name: String,
    /// Beats per minute.
    pub tempo: f64,
    /// Seconds since 1970 (UTC).
    pub created: u64,
    pub saved: u64,
    /// The build that saved it.
    pub app: String,
    /// Any other `key = value` lines, kept as they were.
    pub extra: Vec<(String, String)>,
}

impl Default for Meta {
    fn default() -> Meta {
        Meta {
            name: String::new(),
            tempo: 120.0,
            created: 0,
            saved: 0,
            app: String::new(),
            extra: Vec::new(),
        }
    }
}

impl Meta {
    /// Whether this is autosave's project: its project.txt says
    /// `autosave = yes`.
    pub fn is_autosave(&self) -> bool {
        self.extra
            .iter()
            .any(|(k, v)| k == AUTOSAVE_KEY && v == "yes")
    }

    /// Marks this as autosave's project ([`Meta::is_autosave`]).
    pub fn mark_autosave(&mut self) {
        if !self.is_autosave() {
            self.extra
                .push((AUTOSAVE_KEY.to_string(), "yes".to_string()));
        }
    }

    pub fn to_text(&self) -> String {
        let mut s = format!(
            "{FORMAT}\nname = {}\ntempo = {}\ncreated = {}\nsaved = {}\napp = {}\n",
            one_line(&self.name),
            self.tempo,
            self.created,
            self.saved,
            one_line(&self.app)
        );
        for (k, v) in &self.extra {
            s.push_str(&format!("{} = {}\n", one_line(k), one_line(v)));
        }
        s
    }

    pub fn parse(text: &str) -> io::Result<Meta> {
        let mut lines = text.lines();
        if lines.next().map(str::trim) != Some(FORMAT) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("not an {FORMAT} file"),
            ));
        }
        let mut m = Meta::default();
        for line in lines.map(str::trim) {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            let (k, v) = (k.trim(), v.trim());
            match k {
                "name" => m.name = v.to_string(),
                "tempo" => {
                    if let Some(t) = v.parse::<f64>().ok().filter(|t| (1.0..=999.0).contains(t)) {
                        m.tempo = t;
                    }
                }
                "created" => m.created = v.parse().unwrap_or(0),
                "saved" => m.saved = v.parse().unwrap_or(0),
                "app" => m.app = v.to_string(),
                _ => m.extra.push((k.to_string(), v.to_string())),
            }
        }
        Ok(m)
    }
}

fn one_line(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .trim()
        .to_string()
}

/// A whole project as text: its meta and its parts by owner.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub meta: Meta,
    pub parts: Vec<(String, String)>,
}

impl Snapshot {
    /// The parts of these engines, as they are now.
    pub fn from_parts(meta: Meta, parts: &[&dyn Part]) -> Snapshot {
        Snapshot {
            meta,
            parts: parts
                .iter()
                .map(|p| (p.owner().to_string(), p.save()))
                .collect(),
        }
    }

    pub fn part(&self, owner: &str) -> Option<&str> {
        self.parts
            .iter()
            .find(|(o, _)| o == owner)
            .map(|(_, t)| t.as_str())
    }

    /// Adds the parts of `old` that this snapshot has none of, so a part
    /// no engine claims survives a save.
    pub fn keep_unknown(&mut self, old: &Snapshot) {
        for (o, t) in &old.parts {
            if self.part(o).is_none() {
                self.parts.push((o.clone(), t.clone()));
            }
        }
    }

    /// Gives each engine its part; returns the owners whose load failed,
    /// with why. Engines with no part keep their state.
    pub fn load_into(&self, parts: &mut [&mut dyn Part]) -> Vec<(String, String)> {
        let mut failed = Vec::new();
        for p in parts.iter_mut() {
            if let Some(t) = self.part(p.owner()) {
                if let Err(e) = p.load(t) {
                    failed.push((p.owner().to_string(), e));
                }
            }
        }
        failed
    }
}

/// An earlier save.
#[derive(Clone, Debug, PartialEq)]
pub struct Version {
    pub id: String,
    pub saved: u64,
}

fn bad(kind: io::ErrorKind, s: impl Into<String>) -> io::Error {
    io::Error::new(kind, s.into())
}

fn sync_dir(dir: &Path) -> io::Result<()> {
    File::open(dir)?.sync_all()
}

fn write_synced(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut f = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o644)
        .custom_flags(0x20000) // O_NOFOLLOW
        .open(path)?;
    f.write_all(bytes)?;
    f.sync_all()
}

fn real_dir(p: &Path) -> bool {
    fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_dir())
}

/// Two 64-bit FNV-1a hashes with different seeds, read in chunks.
#[derive(Clone, Copy)]
struct Hasher {
    a: u64,
    b: u64,
    len: u64,
}

impl Hasher {
    fn new() -> Hasher {
        Hasher {
            a: 0xcbf2_9ce4_8422_2325,
            b: 0x6c62_272e_07bb_0142,
            len: 0,
        }
    }

    fn update(&mut self, bytes: &[u8]) {
        for &x in bytes {
            self.a = (self.a ^ u64::from(x)).wrapping_mul(0x0000_0100_0000_01b3);
            self.b = (self.b ^ u64::from(x))
                .wrapping_mul(0x0000_0100_0000_01b3)
                .rotate_left(5);
        }
        self.len += bytes.len() as u64;
    }

    fn name(&self) -> String {
        format!("{:016x}{:016x}-{}", self.a, self.b, self.len)
    }
}

fn same_bytes(a: &Path, b: &Path) -> io::Result<bool> {
    let (fa, fb) = (File::open(a)?, File::open(b)?);
    if fa.metadata()?.len() != fb.metadata()?.len() {
        return Ok(false);
    }
    let (mut ra, mut rb) = (BufReader::new(fa), BufReader::new(fb));
    let (mut ba, mut bb) = (vec![0u8; 64 * 1024], vec![0u8; 64 * 1024]);
    loop {
        let n = ra.read(&mut ba)?;
        if n == 0 {
            return Ok(true);
        }
        rb.read_exact(&mut bb[..n])?;
        if ba[..n] != bb[..n] {
            return Ok(false);
        }
    }
}

impl Library {
    fn project_dir(&self, project: &Loc) -> io::Result<PathBuf> {
        if project.place != Place::Projects || project.names().len() != 1 {
            return Err(bad(
                io::ErrorKind::InvalidInput,
                "a project is a folder at the top of Projects",
            ));
        }
        self.path(project)
    }

    /// A name for a new project that needs no typing:
    /// `Project 2026-11-11 21.04`, or ` 2` ... if taken.
    pub fn new_project_name(&self, now_utc: u64) -> String {
        let (y, mo, d, h, mi) = civil(now_utc);
        let base = format!("Project {y:04}-{mo:02}-{d:02} {h:02}.{mi:02}");
        crate::unique_folder_name(&base, |n| {
            Loc::top(Place::Projects)
                .join(n)
                .is_ok_and(|l| self.exists(&l))
        })
    }

    /// The project autosave writes to: `Projects/Autosave` when it is not
    /// there yet or is autosave's own (`autosave = yes`), else `Autosave 2`,
    /// `Autosave 3` ...: autosave never writes over a project of the
    /// musician's.
    pub fn autosave_project(&self) -> io::Result<Loc> {
        for n in 1..=99 {
            let name = match n {
                1 => AUTOSAVE.to_string(),
                n => format!("{AUTOSAVE} {n}"),
            };
            let loc = Loc::top(Place::Projects).join(&name)?;
            let dir = self.project_dir(&loc)?;
            if fs::symlink_metadata(&dir).is_err() {
                return Ok(loc);
            }
            let ours = fs::read_to_string(dir.join(PROJECT_FILE))
                .ok()
                .and_then(|t| Meta::parse(&t).ok())
                .is_some_and(|m| m.is_autosave());
            if ours && real_dir(&dir) {
                return Ok(loc);
            }
        }
        Err(bad(
            io::ErrorKind::AlreadyExists,
            "every name for the autosave is taken",
        ))
    }

    /// Autosave's project and its project.txt, if there is one
    /// ([`Library::autosave_project`]).
    pub fn autosave_meta(&self) -> Option<(Loc, Meta)> {
        let loc = self.autosave_project().ok()?;
        let meta = self.project_meta(&loc).ok()?;
        meta.is_autosave().then_some((loc, meta))
    }

    /// Sets autosave's project aside as a project of its own, named for
    /// when it was written (`Unsaved 2026-11-14 06.50`, or ` 2` ... if
    /// taken), so that autosave can't write over it. It keeps its mark:
    /// it opens as unsaved work, as autosave's own does.
    pub fn keep_autosave(&self, autosave: &Loc, saved: u64) -> io::Result<Loc> {
        let (y, mo, d, h, mi) = civil(saved);
        let base = format!("Unsaved {y:04}-{mo:02}-{d:02} {h:02}.{mi:02}");
        let name = crate::unique_folder_name(&base, |n| {
            Loc::top(Place::Projects)
                .join(n)
                .is_ok_and(|l| self.exists(&l))
        });
        self.rename(autosave, &name)
    }

    /// A project's project.txt (not its parts).
    pub fn project_meta(&self, project: &Loc) -> io::Result<Meta> {
        Meta::parse(&fs::read_to_string(
            self.project_dir(project)?.join(PROJECT_FILE),
        )?)
    }

    /// Saves a project: makes its folder the first time, then saves
    /// whole or not at all, keeping the newest `keep` earlier saves.
    pub fn save_project(&self, project: &Loc, snap: &Snapshot, keep: usize) -> io::Result<()> {
        self.save_project_with(project, snap, keep, &[])
    }

    /// [`Library::save_project`], first writing each of `samples` the
    /// project's `samples/` doesn't have yet (whole: a `.part` file,
    /// synced, renamed). If one can't be written, nothing of the save is:
    /// the last save stands.
    pub fn save_project_with(
        &self,
        project: &Loc,
        snap: &Snapshot,
        keep: usize,
        samples: &[NewSample],
    ) -> io::Result<()> {
        let dir = self.project_dir(project)?;
        for s in samples {
            if !valid_name(&s.file) {
                return Err(bad(
                    io::ErrorKind::InvalidInput,
                    format!("bad sample name {:?}", s.file),
                ));
            }
        }
        if snap.parts.len() > MAX_PARTS {
            return Err(bad(io::ErrorKind::InvalidInput, "too many parts"));
        }
        for (o, t) in &snap.parts {
            if !valid_owner(o) || t.len() > MAX_PART {
                return Err(bad(
                    io::ErrorKind::InvalidInput,
                    format!("part {o:?} has a bad name or is too big"),
                ));
            }
        }
        match fs::symlink_metadata(&dir) {
            Ok(m) if m.file_type().is_dir() => {}
            Ok(_) => return Err(bad(io::ErrorKind::AlreadyExists, "not a project folder")),
            Err(_) => {
                fs::create_dir(&dir)?;
                sync_dir(dir.parent().expect("in Projects"))?;
            }
        }
        self.recover(&dir)?;
        for sub in [SAMPLES, VERSIONS] {
            if !real_dir(&dir.join(sub)) {
                fs::create_dir(dir.join(sub))?;
            }
        }
        // 0. The sounds the parts name that aren't there yet.
        put_samples(&dir.join(SAMPLES), samples)?;
        // 1. The new parts, whole, in a hidden folder.
        let new = dir.join(NEW_PARTS);
        if real_dir(&new) {
            fs::remove_dir_all(&new)?;
        }
        fs::create_dir(&new)?;
        for (o, t) in &snap.parts {
            write_synced(&new.join(format!("{o}.txt")), t.as_bytes())?;
        }
        sync_dir(&new)?;
        // 2. The old save into versions/, with its project.txt.
        let parts = dir.join(PARTS);
        if real_dir(&parts) {
            let id = format!("{}-{}", snap.meta.saved, std::process::id());
            let id = crate::unique_folder_name(&id, |n| dir.join(VERSIONS).join(n).exists());
            let v = dir.join(VERSIONS).join(&id);
            fs::create_dir(&v)?;
            if let Ok(old) = fs::read(dir.join(PROJECT_FILE)) {
                write_synced(&v.join(PROJECT_FILE), &old)?;
            }
            sync_dir(&v)?;
            fs::rename(&parts, v.join(PARTS))?;
            sync_dir(&dir.join(VERSIONS))?;
        }
        // 3. The new parts into place, then project.txt.
        fs::rename(&new, &parts)?;
        sync_dir(&dir)?;
        let part = dir.join(format!("{PROJECT_FILE}.part"));
        write_synced(&part, snap.meta.to_text().as_bytes())?;
        fs::rename(&part, dir.join(PROJECT_FILE))?;
        sync_dir(&dir)?;
        self.prune_versions(&dir, keep);
        Ok(())
    }

    /// Finishes a save that was cut off: new parts written but not moved
    /// in yet go in; new parts half written go away.
    fn recover(&self, dir: &Path) -> io::Result<()> {
        let (parts, new) = (dir.join(PARTS), dir.join(NEW_PARTS));
        if real_dir(&new) {
            if real_dir(&parts) {
                // Cut off before step 2 finished: the old save stands.
                fs::remove_dir_all(&new)?;
            } else {
                // Cut off between steps 2 and 3: the new parts were whole.
                fs::rename(&new, &parts)?;
            }
            sync_dir(dir)?;
        }
        let _ = fs::remove_file(dir.join(format!("{PROJECT_FILE}.part")));
        Ok(())
    }

    fn prune_versions(&self, dir: &Path, keep: usize) {
        let mut v = self.read_versions(dir);
        if v.len() > keep {
            for old in v.drain(keep..) {
                let _ = fs::remove_dir_all(dir.join(VERSIONS).join(&old.id));
            }
            let _ = sync_dir(&dir.join(VERSIONS));
        }
    }

    fn read_versions(&self, dir: &Path) -> Vec<Version> {
        let mut out: Vec<Version> = fs::read_dir(dir.join(VERSIONS))
            .map(|rd| {
                rd.filter_map(Result::ok)
                    .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                    .filter_map(|e| {
                        let id = e.file_name().into_string().ok()?;
                        let saved = fs::read_to_string(e.path().join(PROJECT_FILE))
                            .ok()
                            .and_then(|t| Meta::parse(&t).ok())
                            .map(|m| m.saved)
                            .unwrap_or(0);
                        Some(Version { id, saved })
                    })
                    .collect()
            })
            .unwrap_or_default();
        out.sort_by(|a, b| {
            b.saved
                .cmp(&a.saved)
                .then_with(|| crate::natural_cmp(&b.id, &a.id))
        });
        out
    }

    /// Earlier saves of a project, newest first.
    pub fn versions(&self, project: &Loc) -> io::Result<Vec<Version>> {
        Ok(self.read_versions(&self.project_dir(project)?))
    }

    /// Opens a project (finishing a save that was cut off first).
    pub fn open_project(&self, project: &Loc) -> io::Result<Snapshot> {
        let dir = self.project_dir(project)?;
        if !real_dir(&dir) {
            return Err(bad(
                io::ErrorKind::NotFound,
                format!("no project {}", project.name()),
            ));
        }
        self.recover(&dir)?;
        read_snapshot(&dir)
    }

    /// An earlier save, to open in its place (saving it then makes it the
    /// newest save; nothing is lost).
    pub fn open_version(&self, project: &Loc, id: &str) -> io::Result<Snapshot> {
        let dir = self.project_dir(project)?;
        if !valid_name(id) {
            return Err(bad(io::ErrorKind::InvalidInput, "no such version"));
        }
        read_snapshot(&dir.join(VERSIONS).join(id))
    }

    /// Copies a sound from the library into a project's samples; returns
    /// its file name there (the name parts refer to it by).
    pub fn add_sample(&self, project: &Loc, sample: &Loc) -> io::Result<String> {
        let src = self.file_path(sample)?;
        self.add_sample_file(project, &src, sample.name())
    }

    /// As [`Library::add_sample`], from a file the app made (a recording
    /// in RAM written out, a resample). `name` gives the extension.
    pub fn add_sample_file(&self, project: &Loc, src: &Path, name: &str) -> io::Result<String> {
        let dir = self.project_dir(project)?;
        if !real_dir(&dir) {
            return Err(bad(io::ErrorKind::NotFound, "save the project first"));
        }
        let samples = dir.join(SAMPLES);
        if !real_dir(&samples) {
            fs::create_dir(&samples)?;
        }
        let ext = name
            .rsplit_once('.')
            .map(|(_, e)| e.to_ascii_lowercase())
            .filter(|e| {
                !e.is_empty() && e.len() <= 5 && e.bytes().all(|b| b.is_ascii_alphanumeric())
            })
            .unwrap_or_else(|| "bin".into());
        let tmp = samples.join(format!(".incoming-{}.part", std::process::id()));
        let r = (|| {
            let mut i = OpenOptions::new()
                .read(true)
                .custom_flags(0x20000)
                .open(src)?;
            if i.metadata()?.len() > crate::MAX_FILE {
                return Err(bad(io::ErrorKind::InvalidInput, "sample too big"));
            }
            let mut o = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o644)
                .open(&tmp)?;
            let mut h = Hasher::new();
            let mut buf = vec![0u8; 256 * 1024];
            loop {
                let n = i.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                h.update(&buf[..n]);
                o.write_all(&buf[..n])?;
            }
            o.sync_all()?;
            let mut file = format!("{}.{ext}", h.name());
            let mut k = 2;
            loop {
                let dst = samples.join(&file);
                if !dst.exists() {
                    fs::rename(&tmp, &dst)?;
                    sync_dir(&samples)?;
                    return Ok(file);
                }
                if same_bytes(&tmp, &dst)? {
                    fs::remove_file(&tmp)?;
                    return Ok(file);
                }
                file = format!("{}~{k}.{ext}", h.name());
                k += 1;
            }
        })();
        if r.is_err() {
            let _ = fs::remove_file(&tmp);
        }
        r
    }

    /// Where a project's sample is, for the sampler to read.
    pub fn sample_path(&self, project: &Loc, file: &str) -> io::Result<PathBuf> {
        let dir = self.project_dir(project)?;
        if !valid_name(file) {
            return Err(bad(io::ErrorKind::InvalidInput, "bad sample name"));
        }
        let p = dir.join(SAMPLES).join(file);
        match fs::symlink_metadata(&p) {
            Ok(m) if m.is_file() => Ok(p),
            _ => Err(bad(
                io::ErrorKind::NotFound,
                format!("sample {file} is missing"),
            )),
        }
    }
}

/// Writes each of `samples` that `dir` doesn't hold yet.
fn put_samples(dir: &Path, samples: &[NewSample]) -> io::Result<()> {
    let tmp = dir.join(format!(".incoming-{}.part", std::process::id()));
    let mut wrote = false;
    for s in samples {
        let dst = dir.join(&s.file);
        match fs::symlink_metadata(&dst) {
            Ok(m) if m.is_file() => continue,
            Ok(_) => {
                return Err(bad(
                    io::ErrorKind::AlreadyExists,
                    format!("{} is not a file", s.file),
                ))
            }
            Err(_) => {}
        }
        let r = (|| {
            let mut f = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o644)
                .custom_flags(0x20000) // O_NOFOLLOW
                .open(&tmp)?;
            {
                let mut w = io::BufWriter::with_capacity(256 * 1024, Capped { f: &mut f, n: 0 });
                s.data.write_to(&mut w)?;
                w.flush()?;
            }
            f.sync_all()?;
            fs::rename(&tmp, &dst)
        })();
        if r.is_err() {
            let _ = fs::remove_file(&tmp);
            return r;
        }
        wrote = true;
    }
    if wrote {
        sync_dir(dir)?;
    }
    Ok(())
}

fn read_snapshot(dir: &Path) -> io::Result<Snapshot> {
    let meta = Meta::parse(&fs::read_to_string(dir.join(PROJECT_FILE))?)?;
    let mut parts = Vec::new();
    if let Ok(rd) = fs::read_dir(dir.join(PARTS)) {
        for e in rd.filter_map(Result::ok) {
            let Some(name) = e.file_name().into_string().ok() else {
                continue;
            };
            let Some(owner) = name.strip_suffix(".txt").filter(|o| valid_owner(o)) else {
                continue;
            };
            let m = fs::symlink_metadata(e.path())?;
            if !m.is_file() || m.len() > MAX_PART as u64 {
                continue;
            }
            let text = String::from_utf8_lossy(&fs::read(e.path())?).into_owned();
            parts.push((owner.to_string(), text));
        }
    }
    parts.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(Snapshot { meta, parts })
}

/// UTC (year, month, day, hour, minute) from seconds since 1970.
pub fn civil(secs: u64) -> (i64, u32, u32, u32, u32) {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d, (rem / 3600) as u32, (rem / 60 % 60) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::temp;

    struct Engine {
        owner: &'static str,
        state: String,
    }

    impl Part for Engine {
        fn owner(&self) -> &str {
            self.owner
        }
        fn save(&self) -> String {
            self.state.clone()
        }
        fn load(&mut self, text: &str) -> Result<(), String> {
            if text.starts_with("bad") {
                return Err("bad state".into());
            }
            self.state = text.to_string();
            Ok(())
        }
    }

    fn meta(name: &str, saved: u64) -> Meta {
        Meta {
            name: name.into(),
            tempo: 92.5,
            created: 1,
            saved,
            app: "test".into(),
            extra: vec![("key".into(), "C minor".into())],
        }
    }

    #[test]
    fn meta_round_trips_and_keeps_lines_it_does_not_know() {
        let m = meta("Night drive", 5);
        assert_eq!(Meta::parse(&m.to_text()).unwrap(), m);
        assert!(Meta::parse("hello\nname = x").is_err());
        let m = Meta::parse("mpfree-project 1\ntempo = 5000\nname = a\nb").unwrap();
        assert_eq!((m.tempo, m.name.as_str()), (120.0, "a"));
    }

    #[test]
    fn save_and_open_give_back_every_part() {
        let d = temp();
        let lib = Library::at(&d).unwrap();
        let p = Loc::top(Place::Projects).join("Night drive").unwrap();
        let seq = Engine {
            owner: "transport",
            state: "bpm = 92.5\n".into(),
        };
        let smp = Engine {
            owner: "sampler",
            state: "pad 1 = kick.wav\n".into(),
        };
        let snap = Snapshot::from_parts(meta("Night drive", 10), &[&seq, &smp]);
        lib.save_project(&p, &snap, KEEP_VERSIONS).unwrap();
        // The browser lists it as a project.
        let l = lib.list(&Loc::top(Place::Projects)).unwrap();
        assert_eq!(l[0].kind, crate::Kind::Project);
        let back = lib.open_project(&p).unwrap();
        assert_eq!(back.meta, snap.meta);
        assert_eq!(back.part("transport"), Some("bpm = 92.5\n"));
        let mut seq2 = Engine {
            owner: "transport",
            state: String::new(),
        };
        let mut other = Engine {
            owner: "synth",
            state: "kept".into(),
        };
        assert!(back.load_into(&mut [&mut seq2, &mut other]).is_empty());
        assert_eq!(seq2.state, "bpm = 92.5\n");
        assert_eq!(other.state, "kept");
    }

    /// Autosave writes to `Projects/Autosave`, its own (marked) project,
    /// and never over a project of the musician's with that name.
    #[test]
    fn autosave_has_its_own_project_and_never_takes_the_musicians() {
        let d = temp();
        let lib = Library::at(&d).unwrap();
        let top = Loc::top(Place::Projects);
        assert_eq!(lib.autosave_project().unwrap(), top.join(AUTOSAVE).unwrap());
        assert_eq!(lib.autosave_meta(), None, "none yet");
        // The musician's own project called Autosave stays theirs.
        let theirs = Snapshot {
            meta: meta(AUTOSAVE, 3),
            parts: vec![("transport".into(), "mine".into())],
        };
        lib.save_project(&top.join(AUTOSAVE).unwrap(), &theirs, KEEP_VERSIONS)
            .unwrap();
        assert_eq!(
            lib.autosave_project().unwrap(),
            top.join("Autosave 2").unwrap()
        );
        assert_eq!(lib.autosave_meta(), None);
        // A marked one is autosave's, saved again in place.
        let mut m = meta("Autosave of Night drive", 7);
        assert!(!m.is_autosave());
        m.mark_autosave();
        m.mark_autosave();
        assert!(m.is_autosave());
        assert_eq!(m.extra.iter().filter(|(k, _)| k == AUTOSAVE_KEY).count(), 1);
        let ours = Snapshot {
            meta: m.clone(),
            parts: vec![("transport".into(), "unsaved".into())],
        };
        let at = lib.autosave_project().unwrap();
        lib.save_project(&at, &ours, 0).unwrap();
        lib.save_project(&at, &ours, 0).unwrap();
        assert_eq!(lib.autosave_project().unwrap(), at);
        assert_eq!(lib.autosave_meta(), Some((at.clone(), m)));
        assert!(
            lib.versions(&at).unwrap().is_empty(),
            "no earlier saves kept"
        );
        assert_eq!(
            lib.open_project(&top.join(AUTOSAVE).unwrap())
                .unwrap()
                .part("transport"),
            Some("mine"),
            "the musician's project is untouched"
        );
        // Set aside, it is a project of its own, still marked, and the
        // next autosave starts a new one in its place.
        let t = 1_794_637_200; // 2026-11-14 06:20 UTC
        let kept = lib.keep_autosave(&at, t).unwrap();
        assert_eq!(kept, top.join("Unsaved 2026-11-14 06.20").unwrap());
        assert!(lib.open_project(&kept).unwrap().meta.is_autosave());
        assert_eq!(lib.autosave_meta(), None);
        assert_eq!(lib.autosave_project().unwrap(), at);
        lib.save_project(&at, &ours, 0).unwrap();
        assert_eq!(
            lib.keep_autosave(&at, t).unwrap(),
            top.join("Unsaved 2026-11-14 06.20 2").unwrap(),
            "never over one set aside before"
        );
    }

    #[test]
    fn parts_no_engine_claims_survive_a_save() {
        let old = Snapshot {
            meta: meta("a", 1),
            parts: vec![
                ("future".into(), "x".into()),
                ("transport".into(), "old".into()),
            ],
        };
        let mut new = Snapshot {
            meta: meta("a", 2),
            parts: vec![("transport".into(), "new".into())],
        };
        new.keep_unknown(&old);
        assert_eq!(new.part("transport"), Some("new"));
        assert_eq!(new.part("future"), Some("x"));
    }

    #[test]
    fn each_save_keeps_the_last_ones_as_versions() {
        let d = temp();
        let lib = Library::at(&d).unwrap();
        let p = Loc::top(Place::Projects).join("Song").unwrap();
        for i in 1..=5u64 {
            let snap = Snapshot {
                meta: meta("Song", i),
                parts: vec![("transport".into(), format!("take {i}"))],
            };
            lib.save_project(&p, &snap, 3).unwrap();
        }
        let v = lib.versions(&p).unwrap();
        assert_eq!(v.iter().map(|v| v.saved).collect::<Vec<_>>(), [4, 3, 2]);
        assert_eq!(
            lib.open_project(&p).unwrap().part("transport"),
            Some("take 5")
        );
        let old = lib.open_version(&p, &v[2].id).unwrap();
        assert_eq!(old.part("transport"), Some("take 2"));
        assert!(lib.open_version(&p, "../../x").is_err());
    }

    #[test]
    fn a_save_cut_off_at_any_step_opens_whole() {
        let d = temp();
        let lib = Library::at(&d).unwrap();
        let p = Loc::top(Place::Projects).join("Song").unwrap();
        let snap = |s: &str, t: u64| Snapshot {
            meta: meta("Song", t),
            parts: vec![("transport".into(), s.into())],
        };
        lib.save_project(&p, &snap("one", 1), 10).unwrap();
        let dir = lib.root().join("Projects").join("Song");
        // Cut off while writing the new parts: the old save stands.
        fs::create_dir(dir.join(NEW_PARTS)).unwrap();
        fs::write(dir.join(NEW_PARTS).join("transport.txt"), "half").unwrap();
        assert_eq!(lib.open_project(&p).unwrap().part("transport"), Some("one"));
        assert!(!dir.join(NEW_PARTS).exists());
        // Cut off after the old parts went to versions/: the new ones go in.
        fs::create_dir(dir.join(NEW_PARTS)).unwrap();
        fs::write(dir.join(NEW_PARTS).join("transport.txt"), "two").unwrap();
        fs::rename(dir.join(PARTS), dir.join(VERSIONS).join("x")).unwrap();
        assert_eq!(lib.open_project(&p).unwrap().part("transport"), Some("two"));
        // A project.txt.part left over is ignored and cleared.
        fs::write(dir.join("project.txt.part"), "junk").unwrap();
        assert_eq!(lib.open_project(&p).unwrap().meta.name, "Song");
        assert!(!dir.join("project.txt.part").exists());
    }

    #[test]
    fn samples_are_stored_once_by_content() {
        let d = temp();
        let lib = Library::at(&d).unwrap();
        let s = Loc::top(Place::Samples);
        let a = lib.write_new(&s, "Kick.wav", b"same bytes").unwrap();
        let b = lib.write_new(&s, "Kick copy.WAV", b"same bytes").unwrap();
        let c = lib.write_new(&s, "Snare.wav", b"other bytes").unwrap();
        let p = Loc::top(Place::Projects).join("Song").unwrap();
        assert!(lib.add_sample(&p, &a).is_err(), "needs a saved project");
        lib.save_project(
            &p,
            &Snapshot {
                meta: meta("Song", 1),
                parts: vec![],
            },
            10,
        )
        .unwrap();
        let fa = lib.add_sample(&p, &a).unwrap();
        let fb = lib.add_sample(&p, &b).unwrap();
        let fc = lib.add_sample(&p, &c).unwrap();
        assert_eq!(fa, fb);
        assert_ne!(fa, fc);
        assert!(fa.ends_with("-10.wav"));
        assert_eq!(
            fs::read(lib.sample_path(&p, &fa).unwrap()).unwrap(),
            b"same bytes"
        );
        assert!(lib.sample_path(&p, "../../x").is_err());
        let n = fs::read_dir(lib.root().join("Projects/Song/samples"))
            .unwrap()
            .count();
        assert_eq!(n, 2);
    }

    #[test]
    fn bad_parts_and_places_are_refused() {
        let d = temp();
        let lib = Library::at(&d).unwrap();
        let p = Loc::top(Place::Projects).join("Song").unwrap();
        for owner in ["", "../x", "Synth", ".hidden"] {
            let s = Snapshot {
                meta: meta("Song", 1),
                parts: vec![(owner.into(), "x".into())],
            };
            assert!(lib.save_project(&p, &s, 10).is_err(), "{owner}");
        }
        let not_projects = Loc::top(Place::Samples).join("Song").unwrap();
        assert!(lib
            .save_project(&not_projects, &Snapshot::default(), 10)
            .is_err());
        let nested = p.join("Inner").unwrap();
        assert!(lib.save_project(&nested, &Snapshot::default(), 10).is_err());
    }

    #[test]
    fn new_names_need_no_typing() {
        let d = temp();
        let lib = Library::at(&d).unwrap();
        // 2026-11-11 21:04:00 UTC.
        let t = 1_794_431_040;
        assert_eq!(civil(t), (2026, 11, 11, 21, 4));
        assert_eq!(lib.new_project_name(t), "Project 2026-11-11 21.04");
        lib.new_folder(&Loc::top(Place::Projects), "Project 2026-11-11 21.04")
            .unwrap();
        assert_eq!(lib.new_project_name(t), "Project 2026-11-11 21.04 2");
    }

    /// A sound that writes `bytes`, counting how often it is written, or
    /// fails partway.
    struct Sound {
        bytes: Vec<u8>,
        fail: bool,
        writes: std::sync::atomic::AtomicUsize,
    }

    impl SampleData for Sound {
        fn write_to(&self, out: &mut dyn Write) -> io::Result<()> {
            self.writes
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            out.write_all(&self.bytes[..self.bytes.len() / 2])?;
            if self.fail {
                return Err(io::Error::other("the disk is full"));
            }
            out.write_all(&self.bytes[self.bytes.len() / 2..])
        }
    }

    fn sound(bytes: &[u8], fail: bool) -> Arc<Sound> {
        Arc::new(Sound {
            bytes: bytes.to_vec(),
            fail,
            writes: Default::default(),
        })
    }

    fn writes(s: &Sound) -> usize {
        s.writes.load(std::sync::atomic::Ordering::Relaxed)
    }

    #[test]
    fn sounds_an_engine_made_are_written_with_the_save_once() {
        let d = temp();
        let lib = Library::at(&d).unwrap();
        let p = Loc::top(Place::Projects).join("Song").unwrap();
        let take = sound(b"RIFF a take", false);
        let kick = sound(b"RIFF a kick", false);
        let snap = |t: u64| Snapshot {
            meta: meta("Song", t),
            parts: vec![("sampler".into(), "pad 1 = take.wav".into())],
        };
        let new = |f: &str, s: &Arc<Sound>| NewSample::new(f, s.clone() as Arc<dyn SampleData>);
        // The first save makes the folder, the sound and the part.
        lib.save_project_with(&p, &snap(1), 10, &[new("take.wav", &take)])
            .unwrap();
        let path = lib.sample_path(&p, "take.wav").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"RIFF a take");
        // Saved again with another sound: only the new one is written.
        lib.save_project_with(
            &p,
            &snap(2),
            10,
            &[new("take.wav", &take), new("kick.wav", &kick)],
        )
        .unwrap();
        assert_eq!((writes(&take), writes(&kick)), (1, 1));
        assert_eq!(
            fs::read(lib.sample_path(&p, "kick.wav").unwrap()).unwrap(),
            b"RIFF a kick"
        );
        // Nothing half written is left about.
        let names: Vec<String> = fs::read_dir(lib.root().join("Projects/Song/samples"))
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names.len(), 2, "{names:?}");
        // The worker's save job does the same.
        let hat = sound(b"RIFF a hat", false);
        crate::worker::run(
            &lib,
            crate::Job::SaveProject {
                project: p.clone(),
                snap: snap(3),
                keep: 10,
                samples: vec![new("hat.wav", &hat), new("take.wav", &take)],
            },
        )
        .unwrap();
        assert_eq!((writes(&hat), writes(&take)), (1, 1));
        assert!(lib.sample_path(&p, "hat.wav").is_ok());
        // A closure writes one too.
        let bytes = b"RIFF a closure".to_vec();
        let f = NewSample::new(
            "closure.wav",
            Arc::new(move |w: &mut dyn Write| w.write_all(&bytes)),
        );
        lib.save_project_with(&p, &snap(4), 10, &[f]).unwrap();
        assert_eq!(
            fs::read(lib.sample_path(&p, "closure.wav").unwrap()).unwrap(),
            b"RIFF a closure"
        );
    }

    #[test]
    fn takes_in_a_work_folder_are_copied_in_and_cleared() {
        let d = temp();
        let lib = Library::at(&d).unwrap();
        let work = lib.work_dir("audiotracks").unwrap();
        assert_eq!(work, lib.root().join(".work/audiotracks"));
        assert!(lib.work_dir("../x").is_err());
        assert!(lib.work_dir("Bad").is_err());
        fs::write(work.join("take-1.wav"), b"RIFF take one").unwrap();
        let p = Loc::top(Place::Projects).join("Song").unwrap();
        let snap = Snapshot {
            meta: meta("Song", 1),
            parts: vec![("audiotracks".into(), "take take-1.wav".into())],
        };
        let take = NewSample::file("take-1.wav", work.join("take-1.wav"));
        lib.save_project_with(&p, &snap, 10, &[take]).unwrap();
        let kept = lib.sample_path(&p, "take-1.wav").unwrap();
        assert_eq!(fs::read(&kept).unwrap(), b"RIFF take one");
        // The project has its own copy: clearing the work folder leaves it.
        fs::create_dir(work.join("folder")).unwrap();
        assert_eq!(lib.clear_work("audiotracks").unwrap(), 1);
        assert!(!work.join("take-1.wav").exists());
        assert!(work.join("folder").is_dir());
        assert_eq!(fs::read(&kept).unwrap(), b"RIFF take one");
        // A file that has gone fails the save, which leaves the last one.
        let gone = NewSample::file("take-2.wav", work.join("take-2.wav"));
        assert!(lib.save_project_with(&p, &snap, 10, &[gone]).is_err());
        assert!(lib.sample_path(&p, "take-2.wav").is_err());
        assert_eq!(lib.open_project(&p).unwrap().meta.saved, 1);
        // The work folder isn't a place: the browser never shows it.
        assert!(Place::ALL.iter().all(|pl| pl.folder() != ".work"));
    }

    #[test]
    fn a_sound_that_cannot_be_written_leaves_the_last_save_standing() {
        let d = temp();
        let lib = Library::at(&d).unwrap();
        let p = Loc::top(Place::Projects).join("Song").unwrap();
        let ok = Snapshot {
            meta: meta("Song", 1),
            parts: vec![("sampler".into(), "first".into())],
        };
        lib.save_project(&p, &ok, 10).unwrap();
        let broken = sound(b"RIFF half of it", true);
        let snap = Snapshot {
            meta: meta("Song", 2),
            parts: vec![("sampler".into(), "names broken.wav".into())],
        };
        let e = lib
            .save_project_with(&p, &snap, 10, &[NewSample::new("broken.wav", broken)])
            .unwrap_err();
        assert!(e.to_string().contains("the disk is full"), "{e}");
        assert_eq!(lib.open_project(&p).unwrap().part("sampler"), Some("first"));
        assert!(lib.sample_path(&p, "broken.wav").is_err());
        assert_eq!(
            fs::read_dir(lib.root().join("Projects/Song/samples"))
                .unwrap()
                .count(),
            0,
            "no part file left"
        );
        // A name that could leave the folder is refused before anything.
        for bad in ["../x.wav", ".hidden.wav", "a/b.wav", "x.wav.part", ""] {
            let s = NewSample::new(bad, sound(b"x", false) as Arc<dyn SampleData>);
            assert!(lib.save_project_with(&p, &snap, 10, &[s]).is_err(), "{bad}");
        }
        assert_eq!(lib.open_project(&p).unwrap().part("sampler"), Some("first"));
    }
}
