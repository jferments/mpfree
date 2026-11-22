//! Snapshots of a set (disk streaming's P3, step 4): the real versions a
//! show plays, named by their content.
//!
//! Preparing a set writes a manifest naming each file by its SHA-256. Two
//! promises cover a file without copying it: MPFree never rewrites a
//! published file in place, or deletes one a set refers to, and preparing
//! checks each file still matches its hash. Recordings, prepared copies
//! and checked imports are published files, so they are referred to as
//! they are. Any other file, which MPFree itself might edit or replace,
//! gets a staged copy in managed storage, named by its content. A file at
//! another rate gets its prepared copy instead ([`crate::prepare`]).
//!
//! Another program could still change a file on the stick, so opening a
//! set checks every file's size and change time ([`Manifest::quick`]), and
//! a full content check can run before a show ([`Manifest::full`]), which
//! remembers when it last passed. Before writing anything, preparing
//! checks there is room for the copies beside what is kept for recording
//! and saves.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::format::Layout;
use crate::fsio::Fs;
use crate::prepare::{self, Recipe, Target};
use crate::publish::{hash_file, hex, publish, Sink};

/// How a snapshot holds a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A published file, referred to where it is.
    Published,
    /// A copy in managed storage, named by its content.
    Staged,
    /// A prepared copy at the engine's rate.
    Prepared,
}

impl Kind {
    fn word(self) -> &'static str {
        match self {
            Kind::Published => "published",
            Kind::Staged => "staged",
            Kind::Prepared => "prepared",
        }
    }

    fn from_word(w: &str) -> Option<Kind> {
        match w {
            "published" => Some(Kind::Published),
            "staged" => Some(Kind::Staged),
            "prepared" => Some(Kind::Prepared),
            _ => None,
        }
    }
}

/// One file of a snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// What the set calls it.
    pub name: String,
    /// The file that plays.
    pub path: PathBuf,
    /// SHA-256 of that file.
    pub hash: [u8; 32],
    pub size: u64,
    /// Its change time when the snapshot was made: seconds and
    /// nanoseconds since 1970.
    pub changed: Option<(u64, u32)>,
    pub kind: Kind,
    /// A prepared copy's original, by its hash.
    pub from: Option<[u8; 32]>,
}

/// A set's snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    pub set: String,
    /// When it was made, seconds since 1970.
    pub made: u64,
    /// When a full content check last passed.
    pub checked: Option<u64>,
    pub entries: Vec<Entry>,
}

/// What a check found wrong with one file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Problem {
    Missing(String),
    /// Its size, change time or content isn't what the snapshot says.
    Changed(String),
    Unreadable(String, String),
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Problem::Missing(n) => write!(f, "{n}: missing"),
            Problem::Changed(n) => write!(f, "{n}: changed since the set was prepared"),
            Problem::Unreadable(n, e) => write!(f, "{n}: can't be read ({e})"),
        }
    }
}

/// A file's size and change time (seconds and nanoseconds since 1970).
pub type Meta = (u64, Option<(u64, u32)>);

/// A file's size and change time from the file system.
pub fn meta(path: &Path) -> io::Result<Meta> {
    let m = std::fs::metadata(path)?;
    let changed = m
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| (d.as_secs(), d.subsec_nanos()));
    Ok((m.len(), changed))
}

fn unhex(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(s.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}

fn plain(s: &str) -> bool {
    !s.contains(['\t', '\n', '\r'])
}

const HEADER: &str = "mpfree-snapshot 1";

impl Manifest {
    /// The manifest as text: a header, then a line for the set, when it
    /// was made and checked, and one line per file, fields apart by tabs.
    pub fn to_text(&self) -> String {
        let mut t = format!("{HEADER}\nset\t{}\nmade\t{}\n", self.set, self.made);
        t += &match self.checked {
            Some(c) => format!("checked\t{c}\n"),
            None => "checked\t-\n".into(),
        };
        for e in &self.entries {
            let changed = e.changed.map_or("-".into(), |(s, n)| format!("{s}.{n:09}"));
            let from = e.from.map_or("-".into(), |h| hex(&h));
            t += &format!(
                "file\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                e.kind.word(),
                hex(&e.hash),
                e.size,
                changed,
                from,
                e.name,
                e.path.display()
            );
        }
        t
    }

    /// Read a manifest back; None if it isn't one.
    pub fn from_text(text: &str) -> Option<Manifest> {
        let mut lines = text.lines();
        if lines.next()? != HEADER {
            return None;
        }
        let mut m = Manifest {
            set: String::new(),
            made: 0,
            checked: None,
            entries: Vec::new(),
        };
        for line in lines {
            let f: Vec<&str> = line.split('\t').collect();
            match f.as_slice() {
                ["set", s] => m.set = (*s).to_string(),
                ["made", t] => m.made = t.parse().ok()?,
                ["checked", "-"] => m.checked = None,
                ["checked", t] => m.checked = Some(t.parse().ok()?),
                ["file", kind, hash, size, changed, from, name, path] => {
                    let changed = match *changed {
                        "-" => None,
                        c => {
                            let (s, n) = c.split_once('.')?;
                            Some((s.parse().ok()?, n.parse().ok()?))
                        }
                    };
                    let from = match *from {
                        "-" => None,
                        h => Some(unhex(h)?),
                    };
                    m.entries.push(Entry {
                        name: (*name).to_string(),
                        path: PathBuf::from(path),
                        hash: unhex(hash)?,
                        size: size.parse().ok()?,
                        changed,
                        kind: Kind::from_word(kind)?,
                        from,
                    });
                }
                [""] => {}
                _ => return None,
            }
        }
        Some(m)
    }

    /// The check when a set opens: every file there, at the size and
    /// change time it had. Quick: nothing is read.
    pub fn quick(&self, meta: &mut dyn FnMut(&Path) -> io::Result<Meta>) -> Vec<Problem> {
        let mut out = Vec::new();
        for e in &self.entries {
            match meta(&e.path) {
                Err(err) if err.kind() == io::ErrorKind::NotFound => {
                    out.push(Problem::Missing(e.name.clone()))
                }
                Err(err) => out.push(Problem::Unreadable(e.name.clone(), err.to_string())),
                Ok((size, changed)) if size != e.size || changed != e.changed => {
                    out.push(Problem::Changed(e.name.clone()))
                }
                Ok(_) => {}
            }
        }
        out
    }

    /// The full content check before a show: every file read and hashed.
    /// When nothing is wrong, `checked` becomes `now`.
    pub fn full(&mut self, fs: &mut dyn Fs, now: u64) -> Vec<Problem> {
        let mut out = Vec::new();
        for e in &self.entries {
            if !fs.exists(&e.path) {
                out.push(Problem::Missing(e.name.clone()));
                continue;
            }
            match hash_file(fs, &e.path) {
                Err(err) => out.push(Problem::Unreadable(e.name.clone(), err.to_string())),
                Ok((hash, size)) if hash != e.hash || size != e.size => {
                    out.push(Problem::Changed(e.name.clone()))
                }
                Ok(_) => {}
            }
        }
        if out.is_empty() {
            self.checked = Some(now);
        }
        out
    }
}

/// A file a set plays.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wanted {
    /// What the set calls it.
    pub name: String,
    pub path: PathBuf,
    /// A published file (a recording, a prepared copy, a checked import):
    /// MPFree never rewrites it, so it needs no staged copy.
    pub published: bool,
}

/// Why a set couldn't be prepared.
#[derive(Debug, PartialEq, Eq)]
pub enum Refused {
    /// Not room for the copies beside what is kept for recording and
    /// saves.
    Space {
        need: u64,
        free: u64,
        keep: u64,
    },
    /// A name or path a snapshot can't hold (a tab or a line break).
    Name(String),
    /// A file changed while it was being prepared.
    Changed(String),
    Io(String),
}

impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mb = |b: u64| b.div_ceil(1_000_000);
        match self {
            Refused::Space { need, free, keep } => write!(
                f,
                "not enough room on the stick: the copies need about {} MB, {} MB free, {} MB kept for recording and saves",
                mb(*need),
                mb(*free),
                mb(*keep)
            ),
            Refused::Name(n) => write!(f, "{n:?}: a name with a tab or a line break"),
            Refused::Changed(n) => write!(f, "{n}: changed while the set was being prepared"),
            Refused::Io(e) => write!(f, "{e}"),
        }
    }
}

/// Where and how a set is prepared.
pub struct Prep<'a> {
    /// Reads the set's files.
    pub src: &'a mut dyn Fs,
    /// Writes managed storage.
    pub fs: &'a mut dyn Fs,
    /// Where staged and prepared copies go.
    pub managed: &'a Path,
    /// The engine's rate.
    pub engine: u32,
    /// The set asks for float copies.
    pub float: bool,
    /// Bytes free on the stick now.
    pub free: u64,
    /// Bytes kept for recording and saves.
    pub keep: u64,
    pub job: u32,
    /// A file's layout (opens it and reads its header).
    pub layout: &'a mut dyn FnMut(&Path) -> io::Result<Layout>,
    /// A file's size and change time.
    pub meta: &'a mut dyn FnMut(&Path) -> io::Result<Meta>,
}

fn extension(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .filter(|e| e.chars().all(|c| c.is_ascii_alphanumeric()))
        .map_or("bin".into(), str::to_ascii_lowercase)
}

/// Prepare a set made at `now`: hash its files, check there is room,
/// then stage what MPFree might edit and make the copies a file at
/// another rate needs. Nothing is written if there isn't room.
pub fn prepare(p: &mut Prep, set: &str, files: &[Wanted], now: u64) -> Result<Manifest, Refused> {
    let io = |e: io::Error| Refused::Io(e.to_string());
    for w in files {
        if !plain(&w.name) || !plain(&w.path.to_string_lossy()) || !plain(set) {
            return Err(Refused::Name(w.name.clone()));
        }
    }
    // What each file needs, and the room it takes.
    enum Plan {
        AsItIs,
        Stage(String),
        Copy(Recipe),
    }
    let mut plans = Vec::with_capacity(files.len());
    let mut need = 0u64;
    for w in files {
        let (hash, size) = hash_file(p.src, &w.path).map_err(io)?;
        let layout = (p.layout)(&w.path).map_err(io)?;
        let plan = match prepare::target(&layout, p.engine, p.float) {
            Target::Original if w.published => Plan::AsItIs,
            Target::Original => {
                let name = format!("{}.{}", hex(&hash), extension(&w.path));
                if !p.fs.exists(&p.managed.join(&name)) {
                    need += size;
                }
                Plan::Stage(name)
            }
            target => {
                let r = Recipe {
                    original: hash,
                    from: layout,
                    target,
                };
                let there = r.name().is_some_and(|n| p.fs.exists(&p.managed.join(n)));
                if !there {
                    need += prepare::copy_bytes(&r);
                }
                Plan::Copy(r)
            }
        };
        plans.push((hash, size, plan));
    }
    if need.saturating_add(p.keep) > p.free {
        return Err(Refused::Space {
            need,
            free: p.free,
            keep: p.keep,
        });
    }
    let mut entries = Vec::with_capacity(files.len());
    for (w, (hash, size, plan)) in files.iter().zip(plans) {
        let (path, hash, size, kind, from) = match plan {
            Plan::AsItIs => (w.path.clone(), hash, size, Kind::Published, None),
            Plan::Stage(name) => {
                let mut buf = vec![0u8; 256 * 1024];
                let src = &mut *p.src;
                let path = &w.path;
                let mut write = |sink: &mut Sink| {
                    let mut at = 0u64;
                    loop {
                        let n = src.read_at(path, at, &mut buf)?;
                        if n == 0 {
                            return Ok(());
                        }
                        sink(&buf[..n])?;
                        at += n as u64;
                    }
                };
                let made = publish(p.fs, p.managed, &name, p.job, &mut write).map_err(io)?;
                if made.hash != hash {
                    // What was copied isn't what was hashed: the file
                    // changed meanwhile. A fresh copy is dropped; one
                    // already there is the right one and stays.
                    if !made.existed {
                        let _ = p.fs.remove(&made.path);
                    }
                    return Err(Refused::Changed(w.name.clone()));
                }
                (made.path, hash, size, Kind::Staged, None)
            }
            Plan::Copy(r) => {
                let made = prepare::make(p.src, &w.path, p.fs, &r, p.managed, p.job).map_err(io)?;
                (
                    made.file.path,
                    made.file.hash,
                    made.file.size,
                    Kind::Prepared,
                    Some(hash),
                )
            }
        };
        let (_, changed) = (p.meta)(&path).map_err(io)?;
        entries.push(Entry {
            name: w.name.clone(),
            path,
            hash,
            size,
            changed,
            kind,
            from,
        });
    }
    Ok(Manifest {
        set: set.to_string(),
        made: now,
        checked: None,
        entries,
    })
}
