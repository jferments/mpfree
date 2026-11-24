//! The set check as a preflight (live shows B2): for one song of a set, a
//! line for each area checked, each with its mark and what it found, never
//! just a tick. The library checks what it can see on the stick: the
//! song's files and parts, the keyboards and controls its Needs list
//! (through the caller's `connected`), and how its last Soundcheck stands.
//! The app adds the areas only it knows (plugins, outputs, memory with the
//! neighbouring songs, room on the stick, streams).
//!
//! The quick check of B1 ([`Library::check_song_with`]) is made from the
//! same facts, so the two never disagree: a song NOT READY in the Set
//! editor has a red line here.

use crate::manifest::Manifest;
use crate::project::{is_take_file, PARTS};
use crate::sets::{Found, Set, SetSong, SongCheck, Soundcheck};
use crate::{Library, Loc, Snapshot};
use std::fmt;
use std::sync::Arc;

/// What a check view line is about, in the order the lines are listed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Area {
    /// Every sound, take and stem there and unchanged.
    Files,
    /// Every part reads.
    Parts,
    /// Each plugin and preset installed (the app's).
    Plugins,
    /// The keyboards and controls its Needs lists, connected.
    Keyboards,
    /// Its outputs and click routing exist on this MPC (the app's).
    Outputs,
    /// Its peak beside its neighbours, against what's free (the app's).
    Memory,
    /// Room for takes and recovery copies (the app's).
    Stick,
    /// Its streams fit the stream cache (the app's).
    Streams,
    /// What Soundcheck measured.
    Soundcheck,
}

impl Area {
    /// Its word at the line's left.
    pub fn word(self) -> &'static str {
        match self {
            Area::Files => "Files",
            Area::Parts => "Parts",
            Area::Plugins => "Plugins",
            Area::Keyboards => "Keyboards",
            Area::Outputs => "Outputs",
            Area::Memory => "Memory",
            Area::Stick => "Stick",
            Area::Streams => "Streams",
            Area::Soundcheck => "Soundcheck",
        }
    }
}

/// A line's mark: green, a blue note, amber limits, red not ready. A note
/// never changes the song's chip.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Mark {
    Fine,
    Note,
    Limits,
    NotReady,
}

/// One line of a song's preflight.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckLine {
    pub area: Area,
    pub mark: Mark,
    pub words: String,
}

impl CheckLine {
    pub fn new(area: Area, mark: Mark, words: impl Into<String>) -> CheckLine {
        CheckLine {
            area,
            mark,
            words: words.into(),
        }
    }
}

/// A song's chip from its lines: NOT READY if any line is, else LIMITS if
/// any line is, else READY.
pub fn chip(lines: &[CheckLine]) -> &'static str {
    match lines.iter().map(|l| l.mark).max() {
        Some(Mark::NotReady) => "NOT READY",
        Some(Mark::Limits) => "LIMITS",
        _ => "READY",
    }
}

/// Lines sorted into the check view's order (by area, the order found kept
/// within one).
pub fn in_order(mut lines: Vec<CheckLine>) -> Vec<CheckLine> {
    lines.sort_by_key(|l| l.area);
    lines
}

/// The most files a Files line names one by one; the rest are counted.
const NAMED: usize = 3;

/// What the checks found of one song, before it is put in words.
#[derive(Clone, Debug)]
pub(crate) struct Facts {
    pub found: Found,
    /// Its Needs: the name, whether it is needed (else it only limits),
    /// whether it is connected.
    pub keyboards: Vec<(String, bool, bool)>,
    /// What stopped the check, if anything did.
    pub stop: Option<Stop>,
    /// Its tempo, in tenths of a BPM (0: it didn't read).
    pub tempo_x10: u32,
    /// No list of its files yet.
    pub no_manifest: bool,
    /// Files listed but not there.
    pub missing: Vec<String>,
    /// Sounds, takes and loops whose bytes changed.
    pub damaged: Vec<String>,
    /// Parts changed since the last save (they still open).
    pub parts_changed: usize,
    /// From its list of files: sounds and the like, their bytes, parts.
    pub files: usize,
    pub bytes: u64,
    pub parts: usize,
    /// The newest change to a listed file, in seconds since 1970 (0 if
    /// unknown).
    pub newest: u64,
    /// Every file was read in full (Check every file).
    pub deep: bool,
    /// The song as it read, for the app's lines and the estimate.
    pub snap: Option<Snapshot>,
}

/// What kept a check from going on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Stop {
    /// No project has its id, or nothing is in its folder.
    NotThere,
    /// The project doesn't read.
    Unreadable(String),
    /// Its files couldn't be compared with its list.
    Unchecked(String),
}

impl Facts {
    /// The quick check's answer, as B1's Set editor shows it.
    pub fn song_check(&self, song: &SetSong) -> SongCheck {
        let mut c = SongCheck {
            tempo_x10: self.tempo_x10,
            ..SongCheck::default()
        };
        for (name, needed, connected) in &self.keyboards {
            if !connected {
                let why = format!("{name} isn\u{2019}t connected");
                if *needed {
                    c.not_ready.push(why);
                } else {
                    c.limits.push(why);
                }
            }
        }
        match &self.stop {
            Some(Stop::NotThere) => c.not_ready.insert(
                0,
                format!("the project {} isn\u{2019}t in Projects", song.name),
            ),
            Some(Stop::Unreadable(e)) => c
                .not_ready
                .insert(0, format!("it doesn\u{2019}t read: {e}")),
            Some(Stop::Unchecked(e)) => c
                .not_ready
                .insert(0, format!("its files couldn\u{2019}t be checked: {e}")),
            None if self.no_manifest => c.unchecked = true,
            None => {
                let mut gaps: Vec<String> = self
                    .missing
                    .iter()
                    .map(|p| format!("the file {p} is missing"))
                    .chain(self.damaged.iter().map(|p| format!("the file {p} changed")))
                    .collect();
                // A part that changed reads differently from the song that
                // was checked; it still opens, so it only limits.
                if self.parts_changed > 0 {
                    c.limits.push(parts_changed(self.parts_changed));
                }
                gaps.append(&mut c.not_ready);
                c.not_ready = gaps;
            }
        }
        c
    }

    /// The library's lines of the preflight: Files, Parts, Keyboards and
    /// Soundcheck (`soundcheck`, the song's last).
    pub fn lines(&self, song: &SetSong, soundcheck: Option<&Soundcheck>) -> Vec<CheckLine> {
        let mut out = Vec::new();
        self.files_lines(song, &mut out);
        self.parts_lines(&mut out);
        self.keyboard_lines(&mut out);
        self.soundcheck_lines(soundcheck, &mut out);
        out
    }

    fn files_lines(&self, song: &SetSong, out: &mut Vec<CheckLine>) {
        let line = |mark, w: String| CheckLine::new(Area::Files, mark, w);
        match &self.stop {
            Some(Stop::NotThere) => {
                out.push(line(
                    Mark::NotReady,
                    format!(
                        "the project {} isn\u{2019}t in Projects: deleted, or in the Trash",
                        song.name
                    ),
                ));
                return;
            }
            // It didn't read, so its files weren't looked at.
            Some(Stop::Unreadable(_)) => return,
            Some(Stop::Unchecked(e)) => {
                out.push(line(
                    Mark::NotReady,
                    format!("its files couldn\u{2019}t be checked: {e}"),
                ));
                return;
            }
            None => {}
        }
        if self.no_manifest {
            out.push(line(
                Mark::Note,
                "not listed yet, so not checked: saving the song lists its files".into(),
            ));
            return;
        }
        let gaps: Vec<String> = self
            .missing
            .iter()
            .map(|p| format!("{p} is missing"))
            .chain(
                self.damaged
                    .iter()
                    .map(|p| format!("{p} changed since its last save")),
            )
            .collect();
        if gaps.is_empty() {
            let how = if self.deep {
                "each read in full and unchanged"
            } else {
                "all there and unchanged"
            };
            out.push(line(
                Mark::Fine,
                format!(
                    "{}, {}, {how}",
                    count(self.files, "file"),
                    size_word(self.bytes)
                ),
            ));
            return;
        }
        for g in gaps.iter().take(NAMED) {
            out.push(line(Mark::NotReady, g.clone()));
        }
        if gaps.len() > NAMED {
            out.push(line(
                Mark::NotReady,
                count(gaps.len() - NAMED, "more file").replacen("", "and ", 1),
            ));
        }
    }

    fn parts_lines(&self, out: &mut Vec<CheckLine>) {
        let line = |mark, w: String| CheckLine::new(Area::Parts, mark, w);
        match &self.stop {
            Some(Stop::NotThere) => {}
            Some(Stop::Unreadable(e)) => {
                out.push(line(Mark::NotReady, format!("it doesn\u{2019}t read: {e}")))
            }
            _ if self.parts_changed > 0 => {
                out.push(line(Mark::Limits, parts_changed(self.parts_changed)))
            }
            _ if self.parts > 0 => out.push(line(
                Mark::Fine,
                format!("{} read", count(self.parts, "part")),
            )),
            _ => out.push(line(Mark::Fine, "it reads".into())),
        }
    }

    fn keyboard_lines(&self, out: &mut Vec<CheckLine>) {
        let line = |mark, w: String| CheckLine::new(Area::Keyboards, mark, w);
        if self.keyboards.is_empty() {
            out.push(line(Mark::Fine, "it needs none".into()));
            return;
        }
        for (name, needed, connected) in &self.keyboards {
            if !connected {
                out.push(if *needed {
                    line(Mark::NotReady, format!("{name} isn\u{2019}t connected"))
                } else {
                    line(
                        Mark::Limits,
                        format!("{name} isn\u{2019}t connected: it plays without it"),
                    )
                });
            }
        }
        let on: Vec<&str> = self
            .keyboards
            .iter()
            .filter(|k| k.2)
            .map(|k| k.0.as_str())
            .collect();
        if !on.is_empty() {
            out.push(line(Mark::Fine, format!("{} connected", and_list(&on))));
        }
    }

    fn soundcheck_lines(&self, sc: Option<&Soundcheck>, out: &mut Vec<CheckLine>) {
        let line = |mark, w: String| CheckLine::new(Area::Soundcheck, mark, w);
        let Some(sc) = sc else {
            out.push(line(
                Mark::Note,
                "not soundchecked yet: its numbers are estimates".into(),
            ));
            return;
        };
        if sc.glitched() {
            let mut what = Vec::new();
            if sc.misses > 0 {
                what.push(match sc.misses {
                    1 => "a stream ran dry once".to_string(),
                    n => format!("a stream ran dry {n} times"),
                });
            }
            if sc.xruns > 0 {
                what.push(count(sc.xruns as usize, "xrun"));
            }
            let mut w = what.join(" and ");
            if !sc.part.is_empty() {
                w.push_str(&format!(" in {}", sc.part));
            }
            out.push(line(
                Mark::Limits,
                format!("{w}: it plays, with that as its limit"),
            ));
        }
        out.push(line(Mark::Fine, sc.line()));
        if self.newest > sc.at {
            out.push(line(
                Mark::Note,
                "measured before its last change: soundcheck it again for its numbers now".into(),
            ));
        }
    }
}

/// "a part changed since its last save", "3 parts ...".
fn parts_changed(n: usize) -> String {
    if n == 1 {
        "a part changed since its last save".to_string()
    } else {
        format!("{n} parts changed since its last save")
    }
}

/// "1 file", "12 files".
fn count(n: usize, what: &str) -> String {
    if n == 1 {
        format!("1 {what}")
    } else {
        format!("{n} {what}s")
    }
}

/// "KeyLab 61", "KeyLab 61 and Nord", "A, B and C".
fn and_list(names: &[&str]) -> String {
    match names {
        [] => String::new(),
        [one] => one.to_string(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// Bytes as the check says them, 1 000 000 to a megabyte as the memory
/// screens do: "1.8 GB", "540 MB", "2.4 MB", "12 kB".
pub fn size_word(bytes: u64) -> String {
    let b = bytes as f64;
    if b >= 1e9 {
        format!("{:.1} GB", b / 1e9)
    } else if b >= 1e7 {
        format!("{:.0} MB", b / 1e6)
    } else if b >= 1e6 {
        format!("{:.1} MB", b / 1e6)
    } else {
        format!("{} kB", bytes.div_ceil(1000))
    }
}

/// A list of files' counts: sounds and the like, their bytes, parts, and
/// the newest change in seconds since 1970.
fn counts(m: &Manifest) -> (usize, u64, usize, u64) {
    let parts = format!("{PARTS}/");
    let (mut files, mut bytes, mut n_parts, mut newest) = (0, 0, 0, 0u64);
    for f in &m.files {
        if f.path.starts_with(&parts) {
            n_parts += 1;
        } else {
            files += 1;
            bytes += f.bytes;
        }
        newest = newest.max((f.changed / 1_000_000_000) as u64);
    }
    (files, bytes, n_parts, newest)
}

impl Library {
    /// What the checks find of `song`: where it is, whether it reads,
    /// its files against its list (every one read in full when `deep`),
    /// and its Needs (`connected` says, by their names in the rig).
    pub(crate) fn song_facts(
        &self,
        song: &SetSong,
        connected: &dyn Fn(&str) -> bool,
        deep: bool,
    ) -> Facts {
        let found = self.find_song(song);
        let mut f = Facts {
            found: found.clone(),
            keyboards: song
                .needs
                .iter()
                .map(|n| (n.name.clone(), n.needed, connected(&n.name)))
                .collect(),
            stop: None,
            tempo_x10: 0,
            no_manifest: false,
            missing: Vec::new(),
            damaged: Vec::new(),
            parts_changed: 0,
            files: 0,
            bytes: 0,
            parts: 0,
            newest: 0,
            deep,
            snap: None,
        };
        let Some(loc) = found.loc() else {
            f.stop = Some(Stop::NotThere);
            return f;
        };
        match self.open_project(loc) {
            Ok(snap) => {
                f.tempo_x10 = (snap.meta.tempo.clamp(0.0, 999.9) * 10.0).round() as u32;
                f.snap = Some(snap);
            }
            Err(e) => {
                f.stop = Some(Stop::Unreadable(e.to_string()));
                return f;
            }
        }
        self.compare_files(loc, deep, &mut f);
        f
    }

    fn compare_files(&self, loc: &Loc, deep: bool, f: &mut Facts) {
        match self.check_project(loc, deep) {
            Ok(check) if check.no_manifest => f.no_manifest = true,
            Ok(check) => {
                let (missing, damaged) = check.gaps();
                f.missing = missing.iter().map(|s| s.to_string()).collect();
                f.damaged = damaged.iter().map(|s| s.to_string()).collect();
                f.parts_changed = check
                    .changed
                    .iter()
                    .filter(|p| p.starts_with(&format!("{PARTS}/")))
                    .count();
                if let Ok(Some(m)) = self.manifest(loc) {
                    (f.files, f.bytes, f.parts, f.newest) = counts(&m);
                }
            }
            Err(e) => f.stop = Some(Stop::Unchecked(e.to_string())),
        }
    }

    /// The set check as a preflight (live shows B2): the quick check's
    /// answer, as [`Library::check_song_with`] gives it, and the library's
    /// lines of the check view for `song` (Files, Parts, Keyboards,
    /// Soundcheck), in the check view's order. The app adds its own areas'
    /// lines and orders them with [`in_order`].
    pub fn preflight_song(
        &self,
        song: &SetSong,
        connected: &dyn Fn(&str) -> bool,
        deep: bool,
    ) -> (Found, SongCheck, Vec<CheckLine>) {
        let f = self.song_facts(song, connected, deep);
        let check = f.song_check(song);
        let lines = f.lines(song, song.soundcheck.as_ref());
        (f.found, check, lines)
    }
}

/// What the app adds to a song's check: its lines of the areas only it
/// can see (Plugins, Outputs), and how many plugin processes the song
/// starts, for the open time's estimate.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AppSong {
    pub lines: Vec<CheckLine>,
    pub plugins: usize,
}

/// The app's half of the set check (live shows B2), run on the library's
/// worker: what it sees in a song's parts that the library doesn't read
/// (the browser never reads a part).
pub trait AppChecks: Send + Sync {
    /// The app's lines of the song read as `snap`.
    fn song(&self, snap: &Snapshot) -> AppSong;
}

/// The app's half, as a job carries it.
#[derive(Clone)]
pub struct AppCheck(pub Arc<dyn AppChecks>);

impl fmt::Debug for AppCheck {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AppCheck")
    }
}

impl PartialEq for AppCheck {
    fn eq(&self, other: &AppCheck) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// What the app knows of this MPC when a check starts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Machine {
    /// The memory free for songs: what is free now and what the open song
    /// holds, less what MPFree always keeps free. None if unknown.
    pub free_for_songs: Option<u64>,
    /// The stick: whether it can be written, and its free bytes. None if
    /// it couldn't be looked at.
    pub stick: Option<(bool, u64)>,
}

/// A song's numbers before Soundcheck measures them: about what it keeps
/// in memory, about how long a cold open takes, and its takes that stream.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Estimate {
    pub bytes: u64,
    pub open_ms: u32,
    pub takes: usize,
}

impl Estimate {
    /// "about 5 s · about 460 MB".
    pub fn line(&self) -> String {
        format!(
            "about {} s \u{b7} about {}",
            (self.open_ms + 500) / 1000,
            size_word(self.bytes)
        )
    }
}

/// The estimate's model, until the device round (A4) measures real songs:
/// a song's own start, the stick read at this many bytes a second, and
/// each plugin process's start (Surge XT's preset list alone took about
/// 5 s under qemu; most start in well under one).
const OPEN_BASE_MS: u64 = 500;
const READ_BYTES_PER_S: u64 = 20_000_000;
const PLUGIN_MS: u64 = 700;
/// What an open song holds beyond its sounds (its engines, effects and
/// clips), about.
const SONG_BASE: u64 = 24_000_000;

/// A stick with less free is nearly full (as the audio tracks' Stick
/// nearly full warning says), and with less than [`STICK_NONE`] it has no
/// room for takes or recovery copies.
pub const STICK_LOW: u64 = 1_000_000_000;
pub const STICK_NONE: u64 = 100_000_000;

/// One song's check view: the quick check's answer, every line in the
/// check view's order, and its estimate (None if it didn't read).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SongPreflight {
    pub found: Found,
    pub check: SongCheck,
    pub lines: Vec<CheckLine>,
    pub estimate: Option<Estimate>,
}

impl SongPreflight {
    /// Its chip in the check view, from its lines.
    pub fn chip(&self) -> &'static str {
        chip(&self.lines)
    }
}

/// A song's sounds kept in memory once open: each by its hash, with what
/// it keeps (so two songs sharing a sound count it once).
type Kept = Vec<(String, u64)>;

/// What a sound keeps in memory once open, from its header: 16 bits a
/// sample, or 32 for deeper ones, as the sampler keeps them. A header
/// that can't be read counts its file twice over.
fn kept_bytes(path: &std::path::Path, bytes: u64) -> u64 {
    let info = std::fs::File::open(path).and_then(|mut f| crate::read_info(&mut f));
    match info {
        Ok(i) if i.frames > 0 => {
            let each = if i.bits <= 16 && !i.float { 2 } else { 4 };
            i.frames * u64::from(i.channels.max(1)) * each
        }
        _ => bytes.saturating_mul(2),
    }
}

impl Library {
    /// A song's estimate and its sounds kept, from its list of files: the
    /// sounds in `samples/` are read whole when it opens, its audio
    /// tracks' takes stream.
    fn estimate(&self, loc: &Loc, plugins: usize) -> Option<(Estimate, Kept)> {
        let m = self.manifest(loc).ok().flatten()?;
        let dir = self.project_dir(loc).ok()?;
        let mut kept = Vec::new();
        let (mut read, mut takes) = (0u64, 0usize);
        for f in &m.files {
            let Some(name) = f.path.strip_prefix("samples/") else {
                continue;
            };
            if is_take_file(name) {
                takes += 1;
                continue;
            }
            read += f.bytes;
            kept.push((f.hash.clone(), kept_bytes(&dir.join(&f.path), f.bytes)));
        }
        let bytes = SONG_BASE + kept.iter().map(|k| k.1).sum::<u64>();
        let open_ms = OPEN_BASE_MS + read * 1000 / READ_BYTES_PER_S + plugins as u64 * PLUGIN_MS;
        Some((
            Estimate {
                bytes,
                open_ms: open_ms.min(u64::from(u32::MAX)) as u32,
                takes,
            },
            kept,
        ))
    }

    /// The set check as a preflight (live shows B2) for every song of
    /// `set`: each song's quick check and its lines (the library's, the
    /// app's through `app`, the stick's and its memory beside its
    /// neighbours from `machine`), in the check view's order.
    pub fn preflight_set(
        &self,
        set: &Set,
        connected: &dyn Fn(&str) -> bool,
        deep: bool,
        machine: Machine,
        app: Option<&dyn AppChecks>,
    ) -> Vec<SongPreflight> {
        let mut songs = Vec::new();
        let mut kepts = Vec::new();
        for song in &set.songs {
            let f = self.song_facts(song, connected, deep);
            let check = f.song_check(song);
            let mut lines = f.lines(song, song.soundcheck.as_ref());
            let mut estimate = None;
            let mut kept = Vec::new();
            if let (Some(snap), Some(loc)) = (&f.snap, f.found.loc()) {
                let a = app.map(|a| a.song(snap)).unwrap_or_default();
                lines.extend(a.lines);
                if let Some((e, k)) = self.estimate(loc, a.plugins) {
                    lines.push(streams_line(&e));
                    estimate = Some(e);
                    kept = k;
                }
                lines.push(stick_line(machine.stick));
            }
            songs.push(SongPreflight {
                found: f.found,
                check,
                lines,
                estimate,
            });
            kepts.push(kept);
        }
        for i in 0..songs.len() {
            let peak = |j: usize| -> Option<u64> {
                let measured = set.songs[j].soundcheck.as_ref().map(|s| s.peak_kb * 1024);
                measured.or(songs[j].estimate.map(|e| e.bytes))
            };
            let Some(own) = peak(i) else {
                continue;
            };
            let next = (i + 1 < songs.len())
                .then(|| peak(i + 1).map(|p| (i + 1, p)))
                .flatten();
            let shared = next.map_or(0, |(j, _)| shared_bytes(&kepts[i], &kepts[j]));
            let mut lines = memory_lines(own, next.map(|(j, p)| (j, p, shared)), machine);
            songs[i].lines.append(&mut lines);
        }
        for s in &mut songs {
            s.lines = in_order(std::mem::take(&mut s.lines));
            fold_in(&mut s.check, &s.lines);
        }
        songs
    }
}

/// The quick check's answer gains what only the full check finds (the
/// app's areas, memory, the stick, a glitch in Soundcheck), so the Set
/// editor's chip and the check view's never disagree.
fn fold_in(check: &mut SongCheck, lines: &[CheckLine]) {
    for l in lines {
        if matches!(l.area, Area::Files | Area::Parts | Area::Keyboards) {
            continue;
        }
        let words = match l.area {
            Area::Stick => format!("the stick: {}", l.words),
            _ => l.words.clone(),
        };
        match l.mark {
            Mark::NotReady => check.not_ready.push(words),
            Mark::Limits => check.limits.push(words),
            Mark::Fine | Mark::Note => {}
        }
    }
}

/// The bytes two songs' sounds have in common, counted once.
fn shared_bytes(a: &Kept, b: &Kept) -> u64 {
    a.iter()
        .filter(|(h, _)| b.iter().any(|(h2, _)| h2 == h))
        .map(|(_, k)| *k)
        .sum()
}

/// "1,450 MB": megabytes of 1 000 000 bytes, with a thousands comma,
/// rounded as [`size_word`] rounds a song's "about" figure, so the two agree.
fn mb(bytes: u64) -> String {
    let m = (bytes + 500_000) / 1_000_000;
    if m >= 1000 {
        format!("{},{:03} MB", m / 1000, m % 1000)
    } else {
        format!("{m} MB")
    }
}

/// The Memory lines of a song that takes `own` bytes at its peak, with
/// the song after it (its number from 0, its peak, the bytes they share)
/// if there is one. A song too big on its own is NOT READY; a pair too big
/// together is a note: that change is always cold (D1 prepares a song
/// ahead only when both fit).
fn memory_lines(own: u64, next: Option<(usize, u64, u64)>, m: Machine) -> Vec<CheckLine> {
    let line = |mark, w: String| CheckLine::new(Area::Memory, mark, w);
    let mut out = Vec::new();
    let Some(free) = m.free_for_songs else {
        out.push(line(
            Mark::Fine,
            format!("{} at its peak; what's free couldn't be read", mb(own)),
        ));
        return out;
    };
    if own > free {
        out.push(line(
            Mark::NotReady,
            format!(
                "{} at its peak, more than the {} free for songs",
                mb(own),
                mb(free)
            ),
        ));
        return out;
    }
    out.push(line(
        Mark::Fine,
        format!(
            "{} at its peak, of the {} free for songs",
            mb(own),
            mb(free)
        ),
    ));
    if let Some((j, p, shared)) = next {
        let both = (own + p).saturating_sub(shared);
        if both > free {
            out.push(line(
                Mark::Note,
                format!(
                    "with song {} beside it, {} of the {} free: that change stays cold",
                    j + 1,
                    mb(both).trim_end_matches(" MB"),
                    mb(free)
                ),
            ));
        }
    }
    out
}

/// The Stick line: room for takes and recovery copies.
fn stick_line(stick: Option<(bool, u64)>) -> CheckLine {
    let line = |mark, w: String| CheckLine::new(Area::Stick, mark, w);
    match stick {
        None => line(Mark::Note, "it couldn't be looked at just now".into()),
        Some((false, _)) => line(
            Mark::Limits,
            "it can't be written: no takes or recovery copies can be kept".into(),
        ),
        Some((true, free)) if free < STICK_NONE => line(
            Mark::NotReady,
            format!(
                "only {} free: no room for takes or recovery copies",
                size_word(free)
            ),
        ),
        Some((true, free)) if free < STICK_LOW => line(
            Mark::Limits,
            format!(
                "{} free: nearly full, takes stop when it fills",
                size_word(free)
            ),
        ),
        Some((true, free)) => line(
            Mark::Fine,
            format!("{} free for takes and recovery copies", size_word(free)),
        ),
    }
}

/// The Streams line: its audio tracks' takes, which play from the stick.
fn streams_line(e: &Estimate) -> CheckLine {
    CheckLine::new(
        Area::Streams,
        Mark::Fine,
        match e.takes {
            0 => "nothing streams: every sound is in memory".to_string(),
            n => format!("{} stream from the stick as they play", count(n, "take")),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{Meta, Snapshot};
    use crate::sets::Need;
    use crate::tests::temp;
    use crate::Place;
    use std::fs;

    fn song_snap(name: &str) -> Snapshot {
        Snapshot {
            meta: Meta {
                name: name.into(),
                saved: 1,
                ..Meta::default()
            },
            parts: vec![("clips".into(), "clip a\n".into())],
        }
    }

    fn words(lines: &[CheckLine], area: Area) -> Vec<(Mark, String)> {
        lines
            .iter()
            .filter(|l| l.area == area)
            .map(|l| (l.mark, l.words.clone()))
            .collect()
    }

    /// Each area says what it found, the chip follows the worst line, and
    /// the quick check made from the same facts agrees.
    #[test]
    fn a_songs_preflight_says_what_each_check_found() {
        let lib = Library::at(&temp()).unwrap();
        let night = Loc::top(Place::Projects).join("Night drive").unwrap();
        let saved = lib
            .save_project(&night, &song_snap("Night drive"), 0)
            .unwrap();
        let dir = lib.root().join("Projects/Night drive");
        fs::create_dir_all(dir.join("samples")).unwrap();
        fs::write(dir.join("samples/pad.wav"), vec![0u8; 2_400_000]).unwrap();
        fs::write(dir.join("samples/hat.wav"), vec![1u8; 100_000]).unwrap();
        lib.update_manifest(&night).unwrap();
        let mut song = SetSong::new(&saved.id, "Night drive");
        let yes = |_: &str| true;

        let (_, check, lines) = lib.preflight_song(&song, &yes, false);
        assert!(check.ready());
        assert_eq!(
            words(&lines, Area::Files),
            [(
                Mark::Fine,
                "2 files, 2.5 MB, all there and unchanged".to_string()
            )]
        );
        assert_eq!(
            words(&lines, Area::Parts),
            [(Mark::Fine, "1 part read".into())]
        );
        assert_eq!(
            words(&lines, Area::Keyboards),
            [(Mark::Fine, "it needs none".into())]
        );
        assert_eq!(
            words(&lines, Area::Soundcheck),
            [(
                Mark::Note,
                "not soundchecked yet: its numbers are estimates".into()
            )]
        );
        assert_eq!(chip(&lines), "READY", "a note leaves it ready");
        let (_, _, deep) = lib.preflight_song(&song, &yes, true);
        assert_eq!(
            words(&deep, Area::Files)[0].1,
            "2 files, 2.5 MB, each read in full and unchanged"
        );

        // Keyboards: a needed one off stops it, a nice one limits it, and
        // the ones connected are named.
        song.needs = ["Keystep 37", "FS-6", "Nord"]
            .iter()
            .zip([true, false, true])
            .map(|(n, needed)| Need {
                name: n.to_string(),
                needed,
            })
            .collect();
        let (_, check, lines) = lib.preflight_song(&song, &|n: &str| n == "Nord", false);
        assert_eq!(
            words(&lines, Area::Keyboards),
            [
                (
                    Mark::NotReady,
                    "Keystep 37 isn\u{2019}t connected".to_string()
                ),
                (
                    Mark::Limits,
                    "FS-6 isn\u{2019}t connected: it plays without it".into()
                ),
                (Mark::Fine, "Nord connected".into()),
            ]
        );
        assert_eq!(chip(&lines), "NOT READY");
        assert_eq!(check.word(), "NOT READY");
        let (_, _, lines) = lib.preflight_song(&song, &|n: &str| n != "FS-6", false);
        assert_eq!(chip(&lines), "LIMITS");
        assert_eq!(
            words(&lines, Area::Keyboards).last().unwrap().1,
            "Keystep 37 and Nord connected"
        );
        song.needs.clear();

        // Soundcheck: its numbers, a glitch as a limit, and a note once the
        // song changed after it.
        song.soundcheck = Some(Soundcheck {
            at: u64::MAX / 2,
            open_ms: 3_140,
            peak_kb: 527_344,
            cpu_x10: 712,
            misses: 2,
            xruns: 1,
            part: "Chorus 2".into(),
            ..Soundcheck::default()
        });
        let (_, _, lines) = lib.preflight_song(&song, &yes, false);
        assert_eq!(
            words(&lines, Area::Soundcheck),
            [
                (
                    Mark::Limits,
                    "a stream ran dry 2 times and 1 xrun in Chorus 2: it plays, with that as its limit"
                        .to_string()
                ),
                (
                    Mark::Fine,
                    "opens in 3.1 s \u{b7} 540 MB \u{b7} CPU 71 % in Chorus 2".into()
                ),
            ]
        );
        assert_eq!(chip(&lines), "LIMITS");
        let sc = song.soundcheck.as_mut().unwrap();
        (sc.misses, sc.xruns, sc.at) = (0, 0, 1);
        let (_, _, lines) = lib.preflight_song(&song, &yes, false);
        assert_eq!(
            words(&lines, Area::Soundcheck)[1],
            (
                Mark::Note,
                "measured before its last change: soundcheck it again for its numbers now"
                    .to_string()
            )
        );
        assert_eq!(chip(&lines), "READY");
        song.soundcheck = None;

        // Files gone or damaged are named, three at most, then counted.
        fs::remove_file(dir.join("samples/pad.wav")).unwrap();
        fs::write(dir.join("samples/hat.wav"), vec![2u8; 100_000]).unwrap();
        let (_, check, lines) = lib.preflight_song(&song, &yes, true);
        assert_eq!(
            words(&lines, Area::Files),
            [
                (Mark::NotReady, "samples/pad.wav is missing".to_string()),
                (
                    Mark::NotReady,
                    "samples/hat.wav changed since its last save".into()
                ),
            ]
        );
        assert_eq!(
            check.not_ready,
            [
                "the file samples/pad.wav is missing",
                "the file samples/hat.wav changed"
            ]
        );
        for k in 0..4 {
            fs::write(dir.join(format!("samples/s{k}.wav")), b"RIFF").unwrap();
        }
        lib.update_manifest(&night).unwrap();
        for k in 0..4 {
            fs::remove_file(dir.join(format!("samples/s{k}.wav"))).unwrap();
        }
        let (_, _, lines) = lib.preflight_song(&song, &yes, false);
        let files = words(&lines, Area::Files);
        assert_eq!(files.len(), 4, "{files:?}");
        assert_eq!(files[3], (Mark::NotReady, "and 1 more file".to_string()));
    }

    /// A song that isn't there, or doesn't read, says so once, where it
    /// belongs; one with no list of its files yet gets a note.
    #[test]
    fn a_song_gone_or_unread_or_unlisted_says_so_once() {
        let lib = Library::at(&temp()).unwrap();
        let yes = |_: &str| true;
        let gone = SetSong::new("", "Nowhere");
        let (found, check, lines) = lib.preflight_song(&gone, &yes, false);
        assert_eq!(found, Found::Gone);
        assert_eq!(
            check.not_ready,
            ["the project Nowhere isn\u{2019}t in Projects"]
        );
        assert_eq!(
            lines
                .iter()
                .filter(|l| l.mark == Mark::NotReady)
                .map(|l| (l.area, l.words.as_str()))
                .collect::<Vec<_>>(),
            [(
                Area::Files,
                "the project Nowhere isn\u{2019}t in Projects: deleted, or in the Trash"
            )]
        );
        assert!(words(&lines, Area::Parts).is_empty());

        let old = Loc::top(Place::Projects).join("Old song").unwrap();
        lib.save_project(&old, &song_snap("Old song"), 0).unwrap();
        let (_, check, lines) = lib.preflight_song(&SetSong::new("", "Old song"), &yes, false);
        assert!(check.unchecked);
        assert_eq!(
            words(&lines, Area::Files),
            [(
                Mark::Note,
                "not listed yet, so not checked: saving the song lists its files".to_string()
            )]
        );
        assert_eq!(
            words(&lines, Area::Parts),
            [(Mark::Fine, "it reads".into())]
        );
        assert_eq!(chip(&lines), "READY");

        // A project that doesn't read (a part the disk can't give back):
        // a Parts line, and its files aren't looked at.
        let unread = Facts {
            found: Found::At(old.clone()),
            keyboards: Vec::new(),
            stop: Some(Stop::Unreadable("input/output error".into())),
            tempo_x10: 0,
            no_manifest: false,
            missing: Vec::new(),
            damaged: Vec::new(),
            parts_changed: 0,
            files: 0,
            bytes: 0,
            parts: 0,
            newest: 0,
            deep: false,
            snap: None,
        };
        let song = SetSong::new("", "Old song");
        let lines = unread.lines(&song, None);
        assert!(words(&lines, Area::Files).is_empty(), "{lines:#?}");
        assert_eq!(
            words(&lines, Area::Parts),
            [(
                Mark::NotReady,
                "it doesn\u{2019}t read: input/output error".to_string()
            )]
        );
        assert_eq!(
            unread.song_check(&song).not_ready,
            ["it doesn\u{2019}t read: input/output error"]
        );
    }

    /// A song with `sounds` (name, frames of 16-bit stereo) and `takes`
    /// (take file names) in Projects, listed, as a set's song.
    fn listed_song(lib: &Library, name: &str, sounds: &[(&str, u32)], takes: &[&str]) -> SetSong {
        let loc = Loc::top(Place::Projects).join(name).unwrap();
        let saved = lib.save_project(&loc, &song_snap(name), 0).unwrap();
        let dir = lib.root().join("Projects").join(name).join("samples");
        fs::create_dir_all(&dir).unwrap();
        for (file, frames) in sounds {
            fs::write(
                dir.join(file),
                crate::audio::wav_bytes(44_100, 2, 16, *frames),
            )
            .unwrap();
        }
        for t in takes {
            fs::write(dir.join(t), crate::audio::wav_bytes(44_100, 2, 16, 10)).unwrap();
        }
        lib.update_manifest(&loc).unwrap();
        SetSong::new(&saved.id, name)
    }

    struct Plugs;
    impl AppChecks for Plugs {
        fn song(&self, snap: &Snapshot) -> AppSong {
            let bad = snap.meta.name == "Bad plug";
            AppSong {
                lines: vec![CheckLine::new(
                    Area::Plugins,
                    if bad { Mark::NotReady } else { Mark::Fine },
                    if bad {
                        "x.clap is missing"
                    } else {
                        "it uses none"
                    },
                )],
                plugins: usize::from(bad) * 2,
            }
        }
    }

    /// The full check: the app's lines, the estimate (sounds kept from
    /// their headers, takes stream), Streams, the stick and memory beside
    /// the next song, all folded into the quick check's answer.
    #[test]
    fn a_sets_preflight_adds_the_apps_lines_memory_and_the_stick() {
        let lib = Library::at(&temp()).unwrap();
        // 1,000,000 frames of 16-bit stereo: 4 MB kept.
        let a = listed_song(&lib, "Big one", &[("pad.wav", 1_000_000)], &[]);
        let take = format!("Vocal_{}.wav", "0123456789abcdef0123456789abcdef");
        assert!(crate::project::is_take_file(&take), "{take}");
        let b = listed_song(
            &lib,
            "Bad plug",
            &[("pad.wav", 1_000_000), ("hat.wav", 250_000)],
            &[&take],
        );
        let set = Set {
            media: None,
            songs: vec![a, b],
            extra: vec![],
        };
        let yes = |_: &str| true;
        let roomy = Machine {
            free_for_songs: Some(1_000_000_000),
            stick: Some((true, 20_000_000_000)),
        };
        let out = lib.preflight_set(&set, &yes, false, roomy, Some(&Plugs));
        assert_eq!(out.len(), 2);
        let e = out[0].estimate.unwrap();
        assert_eq!(e.bytes, SONG_BASE + 4_000_000);
        assert_eq!(e.takes, 0);
        assert_eq!(
            e.open_ms as u64,
            OPEN_BASE_MS + 4_000_044 * 1000 / READ_BYTES_PER_S
        );
        let e = out[1].estimate.unwrap();
        // The pad is shared by hash with song 1, the hat is its own; the
        // take streams and isn't read whole.
        assert_eq!(e.bytes, SONG_BASE + 5_000_000);
        assert_eq!(e.takes, 1);
        assert!(e.open_ms as u64 >= OPEN_BASE_MS + 2 * PLUGIN_MS);
        assert_eq!(
            words(&out[1].lines, Area::Streams),
            [(
                Mark::Fine,
                "1 take stream from the stick as they play".to_string()
            )]
        );
        assert_eq!(
            words(&out[0].lines, Area::Streams),
            [(
                Mark::Fine,
                "nothing streams: every sound is in memory".to_string()
            )]
        );
        assert_eq!(
            words(&out[0].lines, Area::Stick),
            [(
                Mark::Fine,
                "20.0 GB free for takes and recovery copies".to_string()
            )]
        );
        assert_eq!(
            words(&out[0].lines, Area::Memory),
            [(
                Mark::Fine,
                "28 MB at its peak, of the 1,000 MB free for songs".to_string()
            )]
        );
        // The lines come in the check view's order, the app's among them.
        let areas: Vec<Area> = out[1].lines.iter().map(|l| l.area).collect();
        let mut sorted = areas.clone();
        sorted.sort();
        assert_eq!(areas, sorted);
        assert_eq!(out[1].chip(), "NOT READY");
        assert_eq!(out[1].check.not_ready, ["x.clap is missing"]);
        assert_eq!(out[0].chip(), "READY");

        // Tight memory: the pair over what's free is a note (that change
        // stays cold), counting the shared pad once; a song over it on
        // its own isn't ready.
        let tight = Machine {
            free_for_songs: Some(40_000_000),
            ..roomy
        };
        let out = lib.preflight_set(&set, &yes, false, tight, None);
        assert_eq!(
            words(&out[0].lines, Area::Memory),
            [
                (
                    Mark::Fine,
                    "28 MB at its peak, of the 40 MB free for songs".to_string()
                ),
                (
                    Mark::Note,
                    "with song 2 beside it, 53 of the 40 MB free: that change stays cold".into()
                ),
            ]
        );
        assert_eq!(out[0].chip(), "READY", "a note leaves it ready");
        let small = Machine {
            free_for_songs: Some(20_000_000),
            ..roomy
        };
        let out = lib.preflight_set(&set, &yes, false, small, None);
        assert_eq!(
            words(&out[0].lines, Area::Memory),
            [(
                Mark::NotReady,
                "28 MB at its peak, more than the 20 MB free for songs".to_string()
            )]
        );
        assert!(out[0].check.not_ready[0].starts_with("28 MB at its peak"));
        let unknown = Machine {
            free_for_songs: None,
            stick: None,
        };
        let out = lib.preflight_set(&set, &yes, false, unknown, None);
        assert_eq!(
            words(&out[0].lines, Area::Memory)[0].1,
            "28 MB at its peak; what's free couldn't be read"
        );
        assert_eq!(
            words(&out[0].lines, Area::Stick),
            [(Mark::Note, "it couldn't be looked at just now".to_string())]
        );
        // A measured soundcheck wins over the estimate.
        let mut measured = set.clone();
        measured.songs[0].soundcheck = Some(Soundcheck {
            at: u64::MAX / 2,
            peak_kb: 10_000,
            ..Soundcheck::default()
        });
        let out = lib.preflight_set(&measured, &yes, false, roomy, None);
        assert_eq!(
            words(&out[0].lines, Area::Memory)[0].1,
            "10 MB at its peak, of the 1,000 MB free for songs"
        );
    }

    /// The stick's three levels, and one that can't be written.
    #[test]
    fn the_stick_line_says_room_nearly_full_none_or_read_only() {
        let w = |s| {
            let l = stick_line(s);
            (l.mark, l.words)
        };
        assert_eq!(w(Some((true, STICK_LOW))).0, Mark::Fine);
        assert_eq!(
            w(Some((true, 600_000_000))),
            (
                Mark::Limits,
                "600 MB free: nearly full, takes stop when it fills".to_string()
            )
        );
        assert_eq!(
            w(Some((true, 50_000_000))),
            (
                Mark::NotReady,
                "only 50 MB free: no room for takes or recovery copies".to_string()
            )
        );
        assert_eq!(
            w(Some((false, 9_000_000_000))),
            (
                Mark::Limits,
                "it can't be written: no takes or recovery copies can be kept".to_string()
            )
        );
        let mut check = SongCheck::default();
        fold_in(
            &mut check,
            &[
                stick_line(Some((true, 600_000_000))),
                CheckLine::new(Area::Files, Mark::NotReady, "already in"),
                CheckLine::new(Area::Memory, Mark::Note, "a note"),
            ],
        );
        assert_eq!(
            check.limits,
            ["the stick: 600 MB free: nearly full, takes stop when it fills"]
        );
        assert!(
            check.not_ready.is_empty(),
            "the library's own lines are in already"
        );
    }

    #[test]
    fn sizes_and_order_read_as_the_check_view_says_them() {
        assert_eq!(size_word(12_345), "13 kB");
        assert_eq!(size_word(2_400_000), "2.4 MB");
        assert_eq!(size_word(540_000_000), "540 MB");
        assert_eq!(size_word(1_830_000_000), "1.8 GB");
        let lines = in_order(vec![
            CheckLine::new(Area::Soundcheck, Mark::Note, "s"),
            CheckLine::new(Area::Memory, Mark::Note, "m"),
            CheckLine::new(Area::Files, Mark::Fine, "f1"),
            CheckLine::new(Area::Files, Mark::NotReady, "f2"),
        ]);
        let order: Vec<&str> = lines.iter().map(|l| l.words.as_str()).collect();
        assert_eq!(order, ["f1", "f2", "m", "s"]);
        assert_eq!(chip(&[]), "READY");
        assert_eq!(Area::Keyboards.word(), "Keyboards");
    }
}
