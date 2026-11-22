//! A project's manifest: every file it holds (its parts and its sounds,
//! takes and loops in `samples/`), each with its size, when it last changed
//! and a hash of its bytes, kept in the project's folder as
//! `manifest.txt`. A set list's check (live shows stage B) reads it to say
//! quickly whether a song is whole and whether it changed since the last
//! soundcheck; a renamed song is found by its lasting id
//! ([`Library::find_project`]).
//!
//! ```text
//! mpfree-manifest 1
//! <32 hex digits> <bytes> <changed, ns since 1970> <path in the project>
//! ```
//!
//! - **A cache, never the truth.** A file's hash is read again only when
//!   its size or its time changed ([`Library::update_manifest`],
//!   [`Library::check_project`]), so a manifest left old by a cut-off write
//!   or by a hand edit costs a read, never a wrong answer. The deep check
//!   reads every file.
//! - **Off the save.** The browser asks for it after a save has landed, as
//!   a job of its own on the library's worker, so a save takes no longer.
//!   Only new or changed files are read: the parts every time (a few KB),
//!   a sound once.
//! - Autosave's project has none. `project.txt` and `versions/` aren't
//!   listed: the first changes at every save and the second is history.

use crate::project::{real_dir, write_synced, Hasher, PARTS, SAMPLES};
use crate::{Library, Loc, Place, PROJECT_FILE};
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::Path;
use std::time::UNIX_EPOCH;

/// The first line of `manifest.txt`.
pub const MANIFEST_FORMAT: &str = "mpfree-manifest 1";
/// The manifest's file in a project's folder.
pub const MANIFEST_FILE: &str = "manifest.txt";
/// The most lines a manifest is read for: a project holds a few hundred
/// files at most, so anything past this is not one of ours.
const MAX_LINES: usize = 100_000;

/// One file a project holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileEntry {
    /// Its place in the project: `parts/clips.txt`, `samples/<name>`.
    pub path: String,
    pub bytes: u64,
    /// When it last changed, in nanoseconds since 1970 (0 if unknown).
    pub changed: u128,
    /// 32 hex digits.
    pub hash: String,
}

/// A project's files, in order of path.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Manifest {
    pub files: Vec<FileEntry>,
}

impl Manifest {
    pub fn to_text(&self) -> String {
        let mut s = format!("{MANIFEST_FORMAT}\n");
        for f in &self.files {
            s.push_str(&format!(
                "{} {} {} {}\n",
                f.hash, f.bytes, f.changed, f.path
            ));
        }
        s
    }

    /// A manifest's text; lines that can't be read are skipped (they only
    /// cost a read later).
    pub fn parse(text: &str) -> io::Result<Manifest> {
        let mut lines = text.lines();
        if lines.next().map(str::trim) != Some(MANIFEST_FORMAT) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("not an {MANIFEST_FORMAT} file"),
            ));
        }
        let mut files: Vec<FileEntry> = lines
            .take(MAX_LINES)
            .filter_map(|l| {
                let mut it = l.splitn(4, ' ');
                let hash = it.next()?;
                let bytes = it.next()?.parse().ok()?;
                let changed = it.next()?.parse().ok()?;
                let path = it.next()?;
                (crate::project::valid_id(hash) && listed(path)).then(|| FileEntry {
                    path: path.to_string(),
                    bytes,
                    changed,
                    hash: hash.to_string(),
                })
            })
            .collect();
        files.sort_by(|a, b| a.path.cmp(&b.path));
        files.dedup_by(|a, b| a.path == b.path);
        Ok(Manifest { files })
    }

    pub fn get(&self, path: &str) -> Option<&FileEntry> {
        self.files
            .binary_search_by(|f| f.path.as_str().cmp(path))
            .ok()
            .map(|i| &self.files[i])
    }
}

/// Whether `path` is one a manifest lists: a part or a sample, by a plain
/// name (never `..`, a hidden file or a deeper folder).
fn listed(path: &str) -> bool {
    match path.split_once('/') {
        Some((dir, name)) => {
            (dir == PARTS || dir == SAMPLES) && crate::valid_name(name) && !name.contains('/')
        }
        None => false,
    }
}

/// What [`Library::update_manifest`] did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ManifestDone {
    /// Files listed.
    pub files: usize,
    /// Files read for their hash (new or changed since the last manifest).
    pub read: usize,
    /// Files that changed while they were read, left out this time (the
    /// next update reads them).
    pub moving: usize,
}

/// What [`Library::check_project`] found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Check {
    /// The project has no manifest yet: nothing could be compared.
    pub no_manifest: bool,
    /// Files the manifest lists that aren't there.
    pub missing: Vec<String>,
    /// Files whose bytes aren't the ones listed (a different size, or a
    /// different hash when read).
    pub changed: Vec<String>,
    /// Files there that the manifest doesn't list (added since).
    pub added: Vec<String>,
    /// Files read to check them.
    pub read: usize,
}

impl Check {
    /// Every file the manifest lists is there with its bytes.
    pub fn whole(&self) -> bool {
        !self.no_manifest && self.missing.is_empty() && self.changed.is_empty()
    }
}

/// A file's size and time, if it is a plain file.
fn stat(p: &Path) -> Option<(u64, u128)> {
    let m = fs::symlink_metadata(p).ok()?;
    if !m.file_type().is_file() {
        return None;
    }
    let t = m
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos());
    Some((m.len(), t))
}

/// The hash of a file's bytes.
fn hash_file(p: &Path) -> io::Result<String> {
    let mut f = File::open(p)?;
    let mut h = Hasher::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            return Ok(h.hex());
        }
        h.update(&buf[..n]);
    }
}

/// The paths of the files a project holds now, in order.
fn files_now(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for sub in [PARTS, SAMPLES] {
        if !real_dir(&dir.join(sub)) {
            continue;
        }
        if let Ok(rd) = fs::read_dir(dir.join(sub)) {
            for e in rd.filter_map(Result::ok) {
                let Ok(name) = e.file_name().into_string() else {
                    continue;
                };
                let path = format!("{sub}/{name}");
                if listed(&path) && e.file_type().is_ok_and(|t| t.is_file()) {
                    out.push(path);
                }
            }
        }
    }
    out.sort();
    out
}

impl Library {
    /// The project's manifest as it was last written, if it has one.
    pub fn manifest(&self, project: &Loc) -> io::Result<Option<Manifest>> {
        let p = self.project_dir(project)?.join(MANIFEST_FILE);
        match fs::read_to_string(&p) {
            Ok(t) => Manifest::parse(&t).map(Some),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Brings the project's manifest up to date: lists every file it
    /// holds, reading only those that are new or changed since the last
    /// manifest, and writes it whole (a `.part` file, synced, renamed).
    /// Autosave's project gets none.
    pub fn update_manifest(&self, project: &Loc) -> io::Result<ManifestDone> {
        let dir = self.project_dir(project)?;
        if !real_dir(&dir) || !dir.join(PROJECT_FILE).is_file() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("no project {}", project.name()),
            ));
        }
        if self.project_meta(project).is_ok_and(|m| m.is_autosave()) {
            return Ok(ManifestDone::default());
        }
        let old = self.manifest(project).ok().flatten().unwrap_or_default();
        let mut done = ManifestDone::default();
        let mut files = Vec::new();
        for path in files_now(&dir) {
            let p = dir.join(&path);
            let Some((bytes, changed)) = stat(&p) else {
                continue;
            };
            let hash = match old.get(&path) {
                Some(f) if f.bytes == bytes && f.changed == changed && changed != 0 => {
                    f.hash.clone()
                }
                _ => {
                    let hash = hash_file(&p)?;
                    done.read += 1;
                    if stat(&p) != Some((bytes, changed)) {
                        done.moving += 1;
                        continue;
                    }
                    hash
                }
            };
            files.push(FileEntry {
                path,
                bytes,
                changed,
                hash,
            });
        }
        done.files = files.len();
        let m = Manifest { files };
        let part = dir.join(format!("{MANIFEST_FILE}.part"));
        write_synced(&part, m.to_text().as_bytes())?;
        fs::rename(&part, dir.join(MANIFEST_FILE))?;
        Ok(done)
    }

    /// Checks the project's files against its manifest. Quick (`deep`
    /// false): a file with the listed size and time counts as unchanged,
    /// and only one whose time changed is read. Deep: every file is read.
    pub fn check_project(&self, project: &Loc, deep: bool) -> io::Result<Check> {
        let dir = self.project_dir(project)?;
        let Some(m) = self.manifest(project)? else {
            return Ok(Check {
                no_manifest: true,
                ..Check::default()
            });
        };
        let mut c = Check::default();
        for f in &m.files {
            let p = dir.join(&f.path);
            match stat(&p) {
                None => c.missing.push(f.path.clone()),
                Some((bytes, _)) if bytes != f.bytes => c.changed.push(f.path.clone()),
                Some((_, changed)) if !deep && changed == f.changed && changed != 0 => {}
                Some(_) => {
                    c.read += 1;
                    if hash_file(&p)? != f.hash {
                        c.changed.push(f.path.clone());
                    }
                }
            }
        }
        c.added = files_now(&dir)
            .into_iter()
            .filter(|p| m.get(p).is_none())
            .collect();
        Ok(c)
    }

    /// Finds the project with lasting id `id` ([`crate::Meta::id`]): at
    /// `at` if it is there, else wherever it is in Projects now (renamed or
    /// moved), preferring one still in the folder it was saved in. None if
    /// no project has it (deleted, or in the Trash).
    pub fn find_project(&self, id: &str, at: Option<&Loc>) -> Option<Loc> {
        if !crate::project::valid_id(id) {
            return None;
        }
        if let Some(at) = at {
            if self.project_meta(at).is_ok_and(|m| m.id == id) {
                return Some(at.clone());
            }
        }
        let top = Loc::top(Place::Projects);
        let mut found: Vec<(bool, String)> = self
            .list(&top)
            .ok()?
            .into_iter()
            .filter_map(|e| {
                let loc = top.join(&e.name).ok()?;
                let m = self.project_meta(&loc).ok()?;
                (m.id == id).then(|| (m.folder != e.name, e.name))
            })
            .collect();
        found.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| crate::natural_cmp(&a.1, &b.1)));
        let (_, name) = found.into_iter().next()?;
        top.join(&name).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{Meta, Snapshot};
    use crate::tests::temp;

    fn song(saved: u64) -> Snapshot {
        Snapshot {
            meta: Meta {
                name: "Song".into(),
                saved,
                ..Meta::default()
            },
            parts: vec![
                ("transport".into(), format!("bpm = {saved}\n")),
                ("sampler".into(), "pad 1 = kick.wav\n".into()),
            ],
        }
    }

    fn project(lib: &Library, name: &str) -> Loc {
        let p = Loc::top(Place::Projects).join(name).unwrap();
        lib.save_project(&p, &song(1), 10).unwrap();
        let dir = lib.project_dir(&p).unwrap();
        fs::write(dir.join("samples/kick.wav"), vec![7u8; 5000]).unwrap();
        fs::write(dir.join("samples/take 1.wav"), vec![9u8; 70_000]).unwrap();
        p
    }

    #[test]
    fn a_manifest_reads_back_and_skips_lines_that_arent_ours() {
        let m = Manifest {
            files: vec![
                FileEntry {
                    path: "parts/clips.txt".into(),
                    bytes: 12,
                    changed: 1_794_430_000_123_456_789,
                    hash: "0123456789abcdef0123456789abcdef".into(),
                },
                FileEntry {
                    path: "samples/take 1.wav".into(),
                    bytes: 70_000,
                    changed: 5,
                    hash: "fedcba9876543210fedcba9876543210".into(),
                },
            ],
        };
        assert_eq!(Manifest::parse(&m.to_text()).unwrap(), m);
        let junk = format!(
            "{}0123456789abcdef0123456789abcdef 1 1 ../etc/passwd\nnot a line\n0123456789abcdef0123456789abcdef 1 1 versions/x\n0123456789abcdef0123456789abcdef x 1 parts/a.txt\n",
            m.to_text()
        );
        assert_eq!(Manifest::parse(&junk).unwrap(), m);
        assert!(Manifest::parse("hello\n").is_err());
    }

    /// The manifest lists the parts and sounds, reads a file again only
    /// when it changed, and a check notices a changed, missing or added
    /// file; the deep check reads every file.
    #[test]
    fn a_changed_file_is_noticed() {
        let d = temp();
        let lib = Library::at(&d).unwrap();
        let p = project(&lib, "Song");
        assert!(lib.check_project(&p, false).unwrap().no_manifest);
        let first = lib.update_manifest(&p).unwrap();
        assert_eq!(first.files, 4, "{first:?}");
        assert_eq!(first.read, 4);
        let m = lib.manifest(&p).unwrap().unwrap();
        let paths: Vec<&str> = m.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(
            paths,
            [
                "parts/sampler.txt",
                "parts/transport.txt",
                "samples/kick.wav",
                "samples/take 1.wav"
            ]
        );
        // Nothing changed: nothing read.
        assert_eq!(lib.update_manifest(&p).unwrap().read, 0);
        let c = lib.check_project(&p, false).unwrap();
        assert!(c.whole() && c.read == 0 && c.added.is_empty(), "{c:?}");
        // A save rewrites the parts (read again); the sounds stay.
        lib.save_project(&p, &song(2), 10).unwrap();
        assert_eq!(lib.update_manifest(&p).unwrap().read, 2);
        // The same size, different bytes: read, because its time changed.
        let dir = lib.project_dir(&p).unwrap();
        let take = dir.join("samples/take 1.wav");
        let before = fs::metadata(&take).unwrap().modified().unwrap();
        let mut bytes = vec![9u8; 70_000];
        bytes[123] = 1;
        fs::write(&take, &bytes).unwrap();
        // A file system with coarse times may give the same time back:
        // make sure it differs, as any real edit a second later would.
        if fs::metadata(&take).unwrap().modified().unwrap() == before {
            let f = File::options().write(true).open(&take).unwrap();
            f.set_modified(before + std::time::Duration::from_secs(2))
                .unwrap();
        }
        let c = lib.check_project(&p, false).unwrap();
        assert_eq!(
            (c.changed.clone(), c.read),
            (vec!["samples/take 1.wav".to_string()], 1)
        );
        assert!(!c.whole());
        // Missing, a different size, and added.
        fs::remove_file(dir.join("samples/kick.wav")).unwrap();
        fs::write(dir.join("samples/new.wav"), b"x").unwrap();
        let c = lib.check_project(&p, false).unwrap();
        assert_eq!(c.missing, ["samples/kick.wav"]);
        assert_eq!(c.added, ["samples/new.wav"]);
        // Brought up to date, it is whole again.
        lib.update_manifest(&p).unwrap();
        let c = lib.check_project(&p, false).unwrap();
        assert!(c.whole() && c.added.is_empty(), "{c:?}");
        // The deep check reads everything, and catches bytes changed with
        // the time put back as it was (which the quick one can't).
        let listed = lib.manifest(&p).unwrap().unwrap();
        let kept = listed.get("samples/take 1.wav").unwrap().clone();
        let mut bytes = fs::read(&take).unwrap();
        bytes[5] ^= 0xff;
        fs::write(&take, &bytes).unwrap();
        let f = File::options().write(true).open(&take).unwrap();
        let t = UNIX_EPOCH + std::time::Duration::from_nanos(kept.changed as u64);
        f.set_modified(t).unwrap();
        drop(f);
        if stat(&take) == Some((kept.bytes, kept.changed)) {
            assert!(lib.check_project(&p, false).unwrap().whole());
        }
        let deep = lib.check_project(&p, true).unwrap();
        assert_eq!(deep.read, listed.files.len());
        assert_eq!(deep.changed, ["samples/take 1.wav"]);
    }

    /// A renamed song is found by its id, wherever it went; one in the
    /// Trash isn't; autosave's project gets no manifest.
    #[test]
    fn a_renamed_song_is_still_found() {
        let d = temp();
        let lib = Library::at(&d).unwrap();
        let p = project(&lib, "Song");
        let id = lib.project_meta(&p).unwrap().id;
        assert_eq!(lib.find_project(&id, Some(&p)), Some(p.clone()));
        // Renamed in the browser, or by hand on a computer.
        let moved = lib.rename(&p, "Song (encore)").unwrap();
        assert_eq!(lib.find_project(&id, Some(&p)), Some(moved.clone()));
        let dir = lib.project_dir(&moved).unwrap();
        fs::rename(&dir, dir.with_file_name("Encore")).unwrap();
        let encore = Loc::top(Place::Projects).join("Encore").unwrap();
        assert_eq!(lib.find_project(&id, Some(&p)), Some(encore.clone()));
        // A copy by hand: the one in its own folder is the song.
        let back = lib.save_project(&encore, &song(3), 10).unwrap();
        assert_eq!(back.id, id);
        let edir = lib.project_dir(&encore).unwrap();
        fs::create_dir(edir.with_file_name("A copy")).unwrap();
        fs::copy(
            edir.join(PROJECT_FILE),
            edir.with_file_name("A copy").join(PROJECT_FILE),
        )
        .unwrap();
        assert_eq!(lib.find_project(&id, None), Some(encore.clone()));
        // In the Trash: gone from the set.
        lib.delete(&encore).unwrap();
        let copy = Loc::top(Place::Projects).join("A copy").unwrap();
        assert_eq!(lib.find_project(&id, Some(&encore)), Some(copy.clone()));
        lib.delete(&copy).unwrap();
        assert_eq!(lib.find_project(&id, Some(&encore)), None);
        assert_eq!(lib.find_project("nonsense", None), None);
        // Autosave's project: none.
        let mut auto = song(4);
        auto.meta.mark_autosave();
        let at = lib.autosave_project().unwrap();
        lib.save_project(&at, &auto, 0).unwrap();
        assert_eq!(lib.update_manifest(&at).unwrap(), ManifestDone::default());
        assert_eq!(lib.manifest(&at).unwrap(), None);
    }
}
