//! First start on a stick the MPFree USB creator made: the MPC's own drum
//! kits and samples, with no effort from the musician.
//!
//! 1. **The backup.** The pad player is not root and never touches the
//!    eMMC. It asks the stick's root service for the backup by making
//!    `/run/mpfree-app/setup-backup.request` (in RAM); systemd's
//!    `mpfree-setup-backup.path` starts `mpfree-setup-backup.service`
//!    (`/etc/mpfree-test/setup-backup.sh`), which copies the eMMC through a
//!    short read-only window into partition 4's `backup/` and reports in
//!    `/run/mpfree-setup/status` (`key=value` lines), read here for the
//!    progress bar.
//! 2. **Kits and samples.** From the backup on the stick, never from the
//!    eMMC: the content partition inside the backup's copy of the user
//!    area, read with the read-only ext4 reader (`mpfree-factory`). The kits
//!    go to `library/Kits/Akai factory kits.mpfkits`, loaded at every start
//!    before MPFree's own; the samples to `library/Samples/Akai factory/`,
//!    a folder per kit ([`akai_layout`]), where BROWSE finds them. Both are
//!    written only through `mpfree-library`.
//! 3. **Done** is kept in the store (`settings/setup.conf`), step by step
//!    ([`STEPS`]), saved as each step ends (and the try's number as it
//!    starts), so each step runs once even when a later one is cut off. A failed attempt is tried again
//!    at the next start, at most [`MAX_TRIES`] times; the pad player works
//!    either way. A later version can add a step (the MPC's instruments,
//!    say): a stick set up before runs only that step, from the backup it
//!    already has.
//!
//! **The bar** ([`Estimate`]) covers the whole first start. Each stage's
//! share is the time it is expected to take: the bytes it moves (the copy,
//! the read-back, the samples) or the kits it makes, at the rate measured so
//! far, or a first guess before that. So the bar moves as the bytes move, at
//! a steady pace from one stage to the next, never backwards, and the card
//! says how long is left once a rate is known.
//!
//! It runs only where the stick has the backup service (the creator's
//! sticks), only with `--data`, and never in `play`.

use mpfree_diskimage::{Offset, ReadAt};
use mpfree_factory::import::{Content, DEFAULT_KITS};
use mpfree_factory::xpm::Kind;
use mpfree_library::{clean_name, unique_folder_name, unique_name, Library, Loc, Place};
use mpfree_store::Settings;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Read};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

/// The store's owner name: `settings/setup.conf`.
pub const OWNER: &str = "setup";
/// The Akai kits, in the library's Kits place.
pub const KITS_FILE: &str = "Akai factory kits.mpfkits";
/// Akai's samples, in the library's Samples place: a folder per drum kit
/// right inside it ([`akai_layout`]).
pub const SAMPLES_FOLDER: &str = "Akai factory";
/// In Akai factory: a folder per keygroup (and other) program.
pub const INSTRUMENTS: &str = "Instruments";
/// In Akai factory: the sounds in no program, by Akai's folder.
pub const OTHER_SOUNDS: &str = "Other sounds";
/// Akai's top folder for the musician's own projects and sounds on the MPC,
/// left out of Akai factory.
const DOCUMENTS: &str = "MPC Documents";
/// The stick's backup service; without it there is no first-start setup.
pub const SERVICE: &str = "/etc/mpfree-test/setup-backup.sh";
/// What the service reports.
pub const STATUS: &str = "/run/mpfree-setup/status";
/// The request's name, in the program's own folder in /run.
pub const REQUEST: &str = "setup-backup.request";
/// Starts that try the setup before it gives up until the stick is made again.
pub const MAX_TRIES: i64 = 3;
/// The setup's steps, in order, as the store records them (`done`).
/// "kit folders" took the place of "samples" (Akai's samples in Akai's own
/// folders, round 4, 16 Nov), so a stick set up before sorts them into kit
/// folders at its next start, moving what it has rather than copying again.
pub const STEPS: &[&str] = &["backup", "kits", "kit folders"];
/// The largest sample copied (the library's own limit is higher).
const MAX_SAMPLE: u64 = 512 << 20;
/// Samples that fail one after another before the samples step stops.
const GIVE_UP_AFTER: usize = 20;

/// Where things are; tests point them at a temporary folder.
pub struct Paths {
    pub data: PathBuf,
    pub service: PathBuf,
    pub status: PathBuf,
    pub request: PathBuf,
}

impl Paths {
    pub fn on_the_stick(data: &Path, report_dir: &Path) -> Paths {
        Paths {
            data: data.to_path_buf(),
            service: PathBuf::from(SERVICE),
            status: PathBuf::from(STATUS),
            request: report_dir.join(REQUEST),
        }
    }
}

/// How long to wait: for the service to answer at all, and for the whole
/// backup (the service's own limit is an hour).
pub struct Timing {
    pub poll: Duration,
    pub answer: Duration,
    pub backup: Duration,
}

impl Timing {
    pub fn on_the_mpc() -> Timing {
        Timing {
            poll: Duration::from_millis(500),
            answer: Duration::from_secs(60),
            backup: Duration::from_secs(70 * 60),
        }
    }
}

/// The steps the store records as done.
pub fn done_steps(saved: &Settings) -> Vec<String> {
    saved
        .get_str("done")
        .unwrap_or("")
        .split(',')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// Whether the setup still has to run: the stick has the service, a step
/// is not done, and the setup has not given up.
pub fn needed(paths: &Paths, saved: &Settings) -> bool {
    let done = done_steps(saved);
    paths.service.is_file()
        && STEPS.iter().any(|s| !done.iter().any(|d| d == s))
        && saved.get_i64("tries").unwrap_or(0) < MAX_TRIES
}

/// The service's report.
#[derive(Debug, PartialEq)]
pub struct Status {
    pub state: String,
    pub step: String,
    pub done: u64,
    pub total: u64,
    pub message: String,
    pub folder: Option<PathBuf>,
}

pub fn parse_status(text: &str) -> Option<Status> {
    let mut s = Status {
        state: String::new(),
        step: String::new(),
        done: 0,
        total: 0,
        message: String::new(),
        folder: None,
    };
    for line in text.lines() {
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        match k {
            "state" => s.state = v.to_string(),
            "step" => s.step = v.to_string(),
            "done" => s.done = v.parse().unwrap_or(0),
            "total" => s.total = v.parse().unwrap_or(0),
            "message" => s.message = v.to_string(),
            "folder" if !v.is_empty() => s.folder = Some(PathBuf::from(v)),
            _ => {}
        }
    }
    (!s.state.is_empty()).then_some(s)
}

/// One picture of the wizard: a heading, a line under it and the bar (0 to 1).
pub struct Show {
    pub message: String,
    pub detail: String,
    pub value: f32,
}

fn show(f: &mut dyn FnMut(Show), message: &str, detail: String, value: f32) {
    f(Show {
        message: message.to_string(),
        detail,
        value,
    });
}

/// The first start's stages, in order, for the bar.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Stage {
    /// The eMMC copied to the stick (bytes).
    Copy,
    /// The copy read back from the stick and checked (bytes).
    Verify,
    /// The Akai kits made (kits).
    Kits,
    /// The Akai samples copied (bytes).
    Samples,
}

const STAGES: [Stage; 4] = [Stage::Copy, Stage::Verify, Stage::Kits, Stage::Samples];

/// Rates measured over less time than this are not trusted yet.
const MEASURE: f64 = 10.0;

#[derive(Clone, Copy, Debug)]
struct Part {
    /// The work (bytes or kits) and the first guess at its rate, per second.
    work: f64,
    guess: f64,
    done: f64,
    started: Option<f64>,
    took: Option<f64>,
}

impl Part {
    fn rate(&self, now: f64) -> Option<f64> {
        let t = now - self.started?;
        (t >= MEASURE && self.done > 0.0).then(|| self.done / t)
    }

    /// Seconds this stage takes (finished), or is expected to.
    fn seconds(&self, now: f64) -> f64 {
        if let Some(t) = self.took {
            return t;
        }
        self.work / self.rate(now).unwrap_or(self.guess).max(1e-9)
    }

    fn end(&mut self, now: f64) {
        if self.took.is_none() {
            self.took = Some(self.started.map_or(0.0, |t| (now - t).max(0.0)));
            self.done = self.work;
        }
    }

    fn left(&self, now: f64) -> f64 {
        if self.took.is_some() {
            return 0.0;
        }
        (self.work - self.done).max(0.0) / self.rate(now).unwrap_or(self.guess).max(1e-9)
    }

    fn fraction(&self) -> f64 {
        match self.took {
            Some(_) => 1.0,
            None if self.work > 0.0 => (self.done / self.work).clamp(0.0, 1.0),
            None => 0.0,
        }
    }
}

/// The whole first start as one bar (see the module notes). Times are
/// seconds from any fixed start, so tests can run it on their own clock.
#[derive(Clone, Debug)]
pub struct Estimate {
    parts: [Part; 4],
    shown: f32,
}

impl Estimate {
    /// For a backup of `bytes` (the eMMC's user area and boot areas). The
    /// first guesses: the copy at 10 MB/s (a USB 2 stick writing), the
    /// read-back at 25 MB/s, the kits at two a second, and 1.5 GB of samples
    /// at 6 MB/s (read from the backup and written to the same stick); each is
    /// replaced by the measured rate after 10 s, and the samples' size by the
    /// real one once the backup is open.
    pub fn new(bytes: u64) -> Estimate {
        let part = |work: f64, guess: f64| Part {
            work,
            guess,
            done: 0.0,
            started: None,
            took: None,
        };
        let b = bytes as f64;
        // Guesses until a rate is measured, from the MPC One with a USB 3
        // stick on its USB 2 port (about 7 MB/s written, 25 MB/s read): the
        // copy writes only the eMMC's used half and reads the rest; the
        // read-back reads only what was written; Akai's samples are about
        // 3 GB, read and written on the same stick.
        Estimate {
            parts: [
                part(b, 12e6),
                part(b, 35e6),
                part(24.0, 2.0),
                part(3.0e9, 5e6),
            ],
            shown: 0.0,
        }
    }

    fn part(&mut self, s: Stage) -> &mut Part {
        let i = STAGES.iter().position(|x| *x == s).unwrap_or(0);
        &mut self.parts[i]
    }

    /// The stage's real amount of work, once known.
    pub fn work(&mut self, s: Stage, work: u64) {
        self.part(s).work = work as f64;
    }

    /// `done` of the stage's work at `now`; the first report starts it.
    pub fn at(&mut self, s: Stage, done: u64, now: f64) {
        let p = self.part(s);
        p.started.get_or_insert(now);
        p.done = done as f64;
    }

    /// The stage is over (or was done at an earlier start: it then counts
    /// for nothing).
    pub fn finish(&mut self, s: Stage, now: f64) {
        self.part(s).end(now);
    }

    /// The bar, 0 to 1; it never goes back, and reaches 1 only when every
    /// stage is over.
    pub fn value(&mut self, now: f64) -> f32 {
        let total: f64 = self.parts.iter().map(|p| p.seconds(now)).sum();
        let done: f64 = self
            .parts
            .iter()
            .map(|p| p.seconds(now) * p.fraction())
            .sum();
        let all = self.parts.iter().all(|p| p.took.is_some());
        let v = if total > 0.0 {
            (done / total) as f32
        } else {
            0.0
        };
        let v = if all { 1.0 } else { v.min(0.99) };
        self.shown = self.shown.max(v);
        self.shown
    }

    /// Seconds left of the whole first start, once a stage has a measured
    /// rate (a first guess alone is not worth saying).
    pub fn left(&self, now: f64) -> Option<f64> {
        self.parts
            .iter()
            .any(|p| p.took.is_some_and(|t| t > 0.0) || p.rate(now).is_some())
            .then(|| self.parts.iter().map(|p| p.left(now)).sum())
    }

    /// "   ·   about 12 min left", or nothing before a rate is known.
    pub fn says_left(&self, now: f64) -> String {
        match self.left(now) {
            Some(secs) => format!("   ·   {}", time_left(secs)),
            None => String::new(),
        }
    }
}

/// "about 25 s left" or "about 3 min left": rounded up, to 5 s or a minute.
pub fn time_left(secs: f64) -> String {
    if secs < 55.0 {
        format!("about {} s left", ((secs / 5.0).ceil() as u64 * 5).max(5))
    } else {
        format!("about {} min left", (secs / 60.0).ceil() as u64)
    }
}

/// What the setup did: the settings to keep (the steps done, tries, the
/// backup's folder, how many kits and samples) and the kits file, if one
/// was made.
pub struct Outcome {
    pub saved: Settings,
    pub kits: Option<PathBuf>,
    pub failed: Option<String>,
}

const BACKING_UP: &str = "Backing up your MPC, so you can always go back";
const CHECKING: &str = "Checking the backup on the stick";
const EXTRACTING: &str = "Extracting Akai drum kits and samples";

fn gb(bytes: u64) -> String {
    format!("{:.1}", bytes as f64 / 1e9)
}

/// "120 of 480 MB" or "0.4 of 1.6 GB".
fn of(done: u64, total: u64) -> String {
    if total >= 1_000_000_000 {
        format!("{} of {} GB", gb(done), gb(total))
    } else {
        format!("{:.0} of {:.0} MB", done as f64 / 1e6, total as f64 / 1e6)
    }
}

/// The backup's size before the service says it: the MPC One's eMMC.
const EMMC_BYTES: u64 = (15_273_600 + 2 * 8192) * 512;

/// The bar and the clock it runs on.
struct Bar {
    est: Estimate,
    clock: Instant,
}

impl Bar {
    fn now(&self) -> f64 {
        self.clock.elapsed().as_secs_f64()
    }

    fn value(&mut self) -> f32 {
        let now = self.now();
        self.est.value(now)
    }

    fn left(&self) -> String {
        self.est.says_left(self.now())
    }

    fn at(&mut self, s: Stage, done: u64) {
        let now = self.now();
        self.est.at(s, done, now);
    }

    fn finish(&mut self, s: Stage) {
        let now = self.now();
        self.est.finish(s, now);
    }
}

/// The backup's copy of the eMMC's user area: a FILE in the backup folder on
/// partition 4 (made by the stick's root service), only ever read here.
fn image_in(folder: &Path) -> PathBuf {
    folder.join("mmcblk0.img")
}

/// The backup an earlier start made: the one the store names, if it is
/// still on partition 4, or else the newest finished one there.
fn earlier_backup(paths: &Paths, saved: &Settings) -> Option<PathBuf> {
    let named = done_steps(saved)
        .iter()
        .any(|d| d == "backup")
        .then(|| saved.get_str("backup").map(PathBuf::from))
        .flatten()
        .filter(|f| f.parent() == Some(&paths.data.join("backup")) && image_in(f).is_file());
    named.or_else(|| finished_backup(paths))
}

/// The newest finished backup in partition 4's `backup/`, for a start whose
/// record of it was never saved (the third creator stick's, 16 Nov, whose
/// store refused the folder). The root service gives a backup folder its
/// name only once it has read every image back and checked it against the
/// eMMC's hash, and writes SHA256SUMS and MANIFEST.txt first; a cut-off one
/// keeps its `.part` name.
fn finished_backup(paths: &Paths) -> Option<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(paths.data.join("backup"))
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            n.starts_with("mpc-one-emmc-") && !n.ends_with(".part")
        })
        .map(|e| e.path())
        .filter(|f| {
            image_in(f).is_file()
                && f.join("SHA256SUMS").is_file()
                && f.join("MANIFEST.txt").is_file()
        })
        .collect();
    found.sort();
    found.pop()
}

/// Runs the steps not done yet. `saved` is what the store holds now.
/// `save` keeps the settings on the stick as they change: the try's number
/// before anything runs, and each step as it ends. (The third creator stick
/// kept them only at the end: its pad player went before then, and the next
/// start backed the MPC up again.) The caller saves the outcome too.
pub fn run(
    paths: &Paths,
    saved: &Settings,
    timing: &Timing,
    show_f: &mut dyn FnMut(Show),
    say: &mut dyn FnMut(String),
    save: &mut dyn FnMut(&Settings),
) -> Outcome {
    let mut out = saved.clone();
    let tries = saved.get_i64("tries").unwrap_or(0) + 1;
    out.set_i64("tries", tries);
    say(format!("first start: setup, try {tries} of {MAX_TRIES}"));
    // A step this version no longer has ("samples") is forgotten.
    let mut done = done_steps(saved);
    done.retain(|d| STEPS.contains(&d.as_str()));
    if saved.get_str("done").is_some() {
        out.set_str("done", &done.join(","));
    }
    save(&out);
    let mark = |out: &mut Settings, done: &mut Vec<String>, step: &str| {
        if !done.iter().any(|d| d == step) {
            done.push(step.to_string());
        }
        out.set_str("done", &done.join(","));
    };
    let fail = |mut out: Settings, why: String, say: &mut dyn FnMut(String)| {
        say(format!("first start: {why}"));
        out.set_str("state", "failed");
        out.set_str("why", &why);
        Outcome {
            saved: out,
            kits: None,
            failed: Some(why),
        }
    };
    let mut bar = Bar {
        est: Estimate::new(EMMC_BYTES),
        clock: Instant::now(),
    };
    let folder = match earlier_backup(paths, saved) {
        Some(f) => {
            say(format!(
                "first start: using the backup made before: {}",
                f.display()
            ));
            bar.finish(Stage::Copy);
            bar.finish(Stage::Verify);
            f
        }
        None => match backup(paths, timing, &mut bar, show_f, say) {
            Ok(f) => f,
            Err(why) => return fail(out, why, say),
        },
    };
    out.set_str("backup", &folder.display().to_string());
    mark(&mut out, &mut done, "backup");
    save(&out);
    let v = bar.value();
    show(show_f, EXTRACTING, "Reading the backup".into(), v);
    // The backup's copy of the eMMC's user area: a file on partition 4, read only.
    let image = image_in(&folder);
    let content = match File::open(&image)
        .map_err(|e| format!("{}: {e}", image.display()))
        .and_then(|f| {
            Content::from_disk(f).map_err(|e| format!("the backup's content partition: {e}"))
        }) {
        Ok(c) => c,
        Err(why) => return fail(out, why, say),
    };
    let lib = match Library::at(&paths.data) {
        Ok(l) => l,
        Err(e) => return fail(out, format!("the library: {e}"), say),
    };
    let mut failed = Vec::new();
    // Each step is tried even when one before it failed; a later start
    // tries only what failed.
    let kits = match extract_kits(&content, &lib, &mut bar, show_f, say) {
        Ok((kits, made)) => {
            out.set_i64("akai-kits", made as i64);
            mark(&mut out, &mut done, "kits");
            save(&out);
            kits
        }
        Err(why) => {
            failed.push(why);
            None
        }
    };
    match extract_samples(&content, &lib, &mut bar, show_f, say) {
        Ok(samples) => {
            out.set_i64("akai-samples", samples as i64);
            mark(&mut out, &mut done, "kit folders");
            save(&out);
        }
        Err(why) => failed.push(why),
    }
    if !failed.is_empty() {
        let mut o = fail(out, failed.join("; "), say);
        o.kits = kits;
        return o;
    }
    out.set_str("state", "done");
    out.remove("why");
    // A step added later gets its own tries.
    out.set_i64("tries", 0);
    show(show_f, "Setup done", "Starting the pad player".into(), 1.0);
    say(format!(
        "first start: done: {} Akai kits, {} samples; the backup is in {}",
        out.get_i64("akai-kits").unwrap_or(0),
        out.get_i64("akai-samples").unwrap_or(0),
        folder.display()
    ));
    Outcome {
        saved: out,
        kits,
        failed: None,
    }
}

/// Asks for the backup and follows it; the backup's folder on partition 4.
fn backup(
    paths: &Paths,
    timing: &Timing,
    bar: &mut Bar,
    show_f: &mut dyn FnMut(Show),
    say: &mut dyn FnMut(String),
) -> Result<PathBuf, String> {
    show(show_f, BACKING_UP, "Asking for the backup".into(), 0.0);
    // The request: an empty file in the program's own folder in /run (RAM).
    match OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o644)
        .open(&paths.request)
    {
        Ok(_) => {}
        Err(e) if e.kind() == ErrorKind::AlreadyExists => {}
        Err(e) => {
            return Err(format!(
                "could not ask for the backup ({}: {e})",
                paths.request.display()
            ))
        }
    }
    say(format!(
        "first start: asked for the backup ({})",
        paths.request.display()
    ));
    let start = Instant::now();
    let mut last = String::new();
    loop {
        let text = std::fs::read_to_string(&paths.status).unwrap_or_default();
        let status = parse_status(&text);
        match &status {
            None if start.elapsed() >= timing.answer => {
                return Err("the backup service did not answer".into());
            }
            None => {}
            Some(s) => {
                if s.state == "done" {
                    let folder = s
                        .folder
                        .clone()
                        .ok_or("the backup service named no folder")?;
                    // Only a folder in partition 4's backup/ is read.
                    if folder.parent() != Some(&paths.data.join("backup")) {
                        return Err(format!(
                            "the backup is not on partition 4 ({})",
                            folder.display()
                        ));
                    }
                    say(format!("first start: backup done: {}", folder.display()));
                    bar.finish(Stage::Copy);
                    bar.finish(Stage::Verify);
                    return Ok(folder);
                }
                if s.state == "failed" {
                    return Err(format!("the backup did not work: {}", s.message));
                }
                if s.total > 0 {
                    bar.est.work(Stage::Copy, s.total);
                    bar.est.work(Stage::Verify, s.total);
                }
                let (heading, detail) = match s.step.as_str() {
                    "copying" if s.total > 0 => {
                        bar.at(Stage::Copy, s.done);
                        (
                            BACKING_UP,
                            format!(
                                "{} copied{}   ·   keep the stick in and the MPC on",
                                of(s.done, s.total),
                                bar.left()
                            ),
                        )
                    }
                    "closing" => {
                        bar.finish(Stage::Copy);
                        (
                            BACKING_UP,
                            "Copied. Switching the MPC's internal storage off again".to_string(),
                        )
                    }
                    "verifying" if s.total > 0 => {
                        bar.finish(Stage::Copy);
                        bar.at(Stage::Verify, s.done);
                        (
                            CHECKING,
                            format!(
                                "{} read back{}   ·   keep the stick in and the MPC on",
                                of(s.done, s.total),
                                bar.left()
                            ),
                        )
                    }
                    _ => (BACKING_UP, s.message.clone()),
                };
                let value = bar.value();
                show(show_f, heading, detail, value);
                if s.step != last {
                    say(format!("first start: backup: {} ({})", s.step, s.message));
                    last = s.step.clone();
                }
            }
        }
        if start.elapsed() >= timing.backup {
            return Err("the backup took too long".into());
        }
        thread::sleep(timing.poll);
    }
}

/// The library folder `name` in `parent`, made if missing.
fn folder_in(lib: &Library, parent: &Loc, name: &str) -> std::io::Result<Loc> {
    let loc = parent.join(name)?;
    if lib.list(&loc).is_ok() {
        return Ok(loc);
    }
    lib.new_folder(parent, name)
}

/// How many kits a kits file holds, from its header.
fn kit_count(bytes: &[u8]) -> usize {
    bytes
        .get(8..12)
        .map_or(0, |n| u32::from_le_bytes([n[0], n[1], n[2], n[3]]) as usize)
}

/// The Akai kits from the backup: (the kits file, its kits).
fn extract_kits(
    content: &Content<Offset<File>>,
    lib: &Library,
    bar: &mut Bar,
    show_f: &mut dyn FnMut(Show),
    say: &mut dyn FnMut(String),
) -> Result<(Option<PathBuf>, usize), String> {
    let kits_loc = Loc::new(Place::Kits, &[KITS_FILE]).map_err(|e| e.to_string())?;
    let mut made = 0;
    let kits = match lib.file_path(&kits_loc) {
        Ok(p) => {
            // Counted from the file kept, so a later start (a step added
            // since) says how many kits there are, not 0.
            let mut head = [0u8; 12];
            made = File::open(&p)
                .and_then(|mut f| f.read_exact(&mut head))
                .map_or(0, |()| kit_count(&head));
            say(format!(
                "first start: {KITS_FILE} is there already ({made} kits); kept"
            ));
            Some(p)
        }
        Err(_) => {
            let bytes = content
                .kits(
                    DEFAULT_KITS,
                    &mut |line| say(format!("first start: kits: {line}")),
                    &mut |done, total| {
                        bar.est.work(Stage::Kits, total as u64);
                        bar.at(Stage::Kits, done as u64);
                        let value = bar.value();
                        show(
                            show_f,
                            EXTRACTING,
                            format!(
                                "Making drum kit {} of {total}{}",
                                (done + 1).min(total),
                                bar.left()
                            ),
                            value,
                        )
                    },
                )
                .map_err(|e| format!("the kits: {e}"))?;
            match bytes {
                Some(b) => {
                    made = kit_count(&b);
                    let loc = lib
                        .write_new(&Loc::top(Place::Kits), KITS_FILE, &b)
                        .map_err(|e| format!("saving the kits: {e}"))?;
                    Some(lib.file_path(&loc).map_err(|e| e.to_string())?)
                }
                None => {
                    say("first start: no drum programs in the backup; no Akai kits".into());
                    None
                }
            }
        }
    };
    bar.finish(Stage::Kits);
    Ok((kits, made))
}

/// Where one of Akai's sounds goes: `folder` (names under Akai factory),
/// as `name`, from `source` (its path in the content partition).
#[derive(Clone, Debug, PartialEq)]
pub struct Placement {
    pub folder: Vec<String>,
    pub name: String,
    pub source: String,
    pub size: u64,
}

/// Is this path in MPC Documents?
fn in_documents(path: &str) -> bool {
    path.trim_start_matches('/')
        .split('/')
        .next()
        .is_some_and(|f| f.eq_ignore_ascii_case(DOCUMENTS))
}

/// A name in a folder that doesn't take any of `taken` (lower case), and
/// is then taken. Compared without case, for a FAT card.
fn take(name: &str, folder: bool, taken: &mut HashSet<String>) -> String {
    let free = |n: &str| taken.contains(&n.to_lowercase());
    let n = if folder {
        unique_folder_name(name, free)
    } else {
        unique_name(name, free)
    };
    taken.insert(n.to_lowercase());
    n
}

/// Where Akai's sounds go in the library, under Samples/Akai factory:
///
/// - each drum program (a kit) in a folder of its own right inside it,
///   named as the program, with every sound its pads play;
/// - each keygroup (or other) program in `Instruments/<name>`;
/// - the sounds in no program in `Other sounds/<Akai's folder>` (a demo
///   project's `_[ProjectData]` dropped from the name).
///
/// MPC Documents is left out, and so are macOS's `._` files. A program with
/// the same name and sounds as one before it is left out; other names
/// taken get ` 2`. A sound in two kits is in both. The same content always
/// gives the same layout, so a later step can find each pad's file.
pub fn akai_layout<F: ReadAt>(content: &Content<F>, say: &mut dyn FnMut(String)) -> Vec<Placement> {
    let sizes: HashMap<&str, u64> = content.files().collect();
    let mut out = Vec::new();
    let mut in_a_program: HashSet<String> = HashSet::new();
    let mut top: HashSet<String> = [INSTRUMENTS, OTHER_SOUNDS]
        .iter()
        .map(|n| n.to_lowercase())
        .collect();
    let mut instruments: HashSet<String> = HashSet::new();
    let mut seen: Vec<(String, Vec<String>)> = Vec::new();
    let (mut kits, mut insts, mut same) = (0, 0, 0);
    let skip =
        |p: &str| in_documents(p) || p.rsplit('/').next().is_some_and(|b| b.starts_with("._"));
    for p in content.programs(&|p| !in_documents(p)) {
        let sounds: Vec<String> = p.samples.into_iter().filter(|s| !skip(s)).collect();
        if sounds.is_empty() {
            continue;
        }
        let base = clean_name(&p.name, "Program");
        if seen
            .iter()
            .any(|(n, s)| n.eq_ignore_ascii_case(&base) && *s == sounds)
        {
            same += 1;
            continue;
        }
        seen.push((base.clone(), sounds.clone()));
        let folder = if p.kind == Kind::Drum {
            kits += 1;
            vec![take(&base, true, &mut top)]
        } else {
            insts += 1;
            vec![INSTRUMENTS.to_string(), take(&base, true, &mut instruments)]
        };
        let mut names = HashSet::new();
        for src in sounds {
            let base = src.rsplit('/').next().unwrap_or(&src);
            out.push(Placement {
                folder: folder.clone(),
                name: take(&clean_name(base, "sample"), false, &mut names),
                size: sizes.get(src.as_str()).copied().unwrap_or(0),
                source: src.clone(),
            });
            in_a_program.insert(src);
        }
    }
    // The rest, by Akai's folder: (its name, the file names taken in it).
    let mut others: BTreeMap<String, (String, HashSet<String>)> = BTreeMap::new();
    let mut loose = 0;
    for (path, size) in content.samples() {
        if skip(&path) || in_a_program.contains(&path) {
            continue;
        }
        let (dir, base) = path.rsplit_once('/').unwrap_or(("", &path));
        let akai = dir.rsplit('/').next().unwrap_or("");
        let akai = akai.strip_suffix("_[ProjectData]").unwrap_or(akai);
        let mut folder = vec![OTHER_SOUNDS.to_string()];
        let names = if akai.is_empty() {
            &mut others.entry(String::new()).or_default().1
        } else {
            let clean = clean_name(akai, "Sounds");
            let (name, names) = others
                .entry(clean.to_lowercase())
                .or_insert((clean, HashSet::new()));
            folder.push(name.clone());
            names
        };
        out.push(Placement {
            folder,
            name: take(&clean_name(base, "sample"), false, names),
            source: path.clone(),
            size,
        });
        loose += 1;
    }
    let left_out = content
        .samples()
        .iter()
        .filter(|(p, _)| in_documents(p))
        .count();
    say(format!(
        "{kits} kits, {insts} instruments, {loose} sounds in no program; \
         {same} programs the same as one before; {left_out} sounds in {DOCUMENTS} left out"
    ));
    out
}

/// A whole file at `loc`: there, and `size` bytes long.
fn whole(lib: &Library, loc: &Loc, size: u64) -> bool {
    lib.file_path(loc)
        .is_ok_and(|p| std::fs::metadata(p).is_ok_and(|m| m.len() == size))
}

/// Akai's samples from the backup, sorted into folders ([`akai_layout`]):
/// how many are in the library.
///
/// Thousands of files: each is written without its own sync and the whole
/// lot synced once at the end (a sync per file cost two flushes of the
/// stick each, and the copy crawled at about 1 MB/s on the MPC, 16 Nov).
/// Folders are made once each. A sound already in a kit folder is put in
/// the next kit that has it as a second name (a hard link: stored once).
/// A stick set up before keeps what it has: a sound in Akai's own folders
/// (the earlier layout) is moved, not copied again, and what is left of
/// those folders goes to the Trash. A file kept from an earlier try that
/// was cut off counts only if its size is right; a short one (the power
/// went before the sync) goes to the Trash and is copied again.
fn extract_samples<F: ReadAt>(
    content: &Content<F>,
    lib: &Library,
    bar: &mut Bar,
    show_f: &mut dyn FnMut(Show),
    say: &mut dyn FnMut(String),
) -> Result<usize, String> {
    let layout = akai_layout(content, &mut |l| {
        say(format!("first start: kit folders: {l}"))
    });
    let top = folder_in(lib, &Loc::top(Place::Samples), SAMPLES_FOLDER)
        .map_err(|e| format!("the samples folder: {e}"))?;
    // Where the earlier layout put a sound: Akai's own folders.
    let before = |source: &str| -> Option<Loc> {
        let mut at = top.clone();
        for p in source.split('/').filter(|p| !p.is_empty()) {
            at = at.join(&clean_name(p, "sample")).ok()?;
        }
        Some(at)
    };
    let dest_of = |pl: &Placement| -> Option<Loc> {
        let mut at = top.clone();
        for f in &pl.folder {
            at = at.join(f).ok()?;
        }
        at.join(&pl.name).ok()
    };
    // The bar follows the bytes to copy: each sound once, unless the
    // library has it already (moves and second names take no time).
    let mut have: HashSet<&str> = HashSet::new();
    for pl in &layout {
        if dest_of(pl).is_some_and(|d| whole(lib, &d, pl.size))
            || before(&pl.source).is_some_and(|b| whole(lib, &b, pl.size))
        {
            have.insert(&pl.source);
        }
    }
    let mut to_copy: HashMap<&str, u64> = HashMap::new();
    for pl in &layout {
        if !have.contains(pl.source.as_str()) && pl.size <= MAX_SAMPLE {
            to_copy.insert(&pl.source, pl.size);
        }
    }
    let total: u64 = to_copy.values().sum();
    bar.est.work(Stage::Samples, total);
    let mut copied_bytes = 0u64;
    let mut last_show: Option<Instant> = None;
    let mut folders: HashMap<Vec<String>, Loc> = HashMap::new();
    // Where each sound is whole in the new layout, once it is.
    let mut placed: HashMap<&str, Loc> = HashMap::new();
    let (mut kept, mut moved, mut linked, mut copied) = (0usize, 0usize, 0usize, 0usize);
    // Sounds that could not be put in place, one after another: past
    // GIVE_UP_AFTER the stick has most likely stopped taking writes, and the
    // step stops instead of trying thousands more.
    let mut in_a_row = 0usize;
    for (i, pl) in layout.iter().enumerate() {
        if last_show.is_none_or(|t| t.elapsed() >= Duration::from_millis(250)) {
            bar.at(Stage::Samples, copied_bytes);
            let value = bar.value();
            let what = if total == 0 {
                format!("Sorting samples into kits: {} of {}", i, layout.len())
            } else {
                format!("Copying samples: {}", of(copied_bytes, total))
            };
            show(show_f, EXTRACTING, format!("{what}{}", bar.left()), value);
            last_show = Some(Instant::now());
        }
        let path = &pl.source;
        if pl.size > MAX_SAMPLE {
            say(format!("first start: {path} left out ({} bytes)", pl.size));
            continue;
        }
        let at = match folders.get(&pl.folder) {
            Some(l) => l.clone(),
            None => {
                let mut at = top.clone();
                let mut made = Ok(());
                for f in &pl.folder {
                    match at.join(f) {
                        Ok(l) if lib.list(&l).is_ok() => at = l,
                        _ => match lib.new_folder_unsynced(&at, f) {
                            Ok(l) => at = l,
                            Err(e) => {
                                made = Err(format!("folder {f}: {e}"));
                                break;
                            }
                        },
                    }
                }
                if let Err(e) = made {
                    if let Some(why) = give_up(&mut in_a_row, path, e, say) {
                        return Err(why);
                    }
                    continue;
                }
                folders.insert(pl.folder.clone(), at.clone());
                at
            }
        };
        let Ok(dest) = at.join(&pl.name) else {
            continue;
        };
        if whole(lib, &dest, pl.size) {
            kept += 1;
            in_a_row = 0;
            placed.entry(path).or_insert(dest);
            continue;
        }
        if lib.file_path(&dest).is_ok() {
            // Short: from a try that was cut off.
            if let Err(e) = lib.delete(&dest) {
                say(format!(
                    "first start: {path}: a short copy from before stays: {e}"
                ));
            }
        }
        let copy = || {
            content
                .read(path, MAX_SAMPLE)
                .and_then(|b| lib.write_new_unsynced(&at, &pl.name, &b))
        };
        let r = if let Some(from) = placed.get(path.as_str()) {
            lib.link_file_unsynced(from, &at, &pl.name)
                .map(|_| &mut linked)
                .or_else(|_| copy().map(|_| &mut copied))
        } else if let Some(old) = before(path).filter(|b| *b != dest && whole(lib, b, pl.size)) {
            lib.move_file_unsynced(&old, &at, &pl.name)
                .map(|_| &mut moved)
                .or_else(|_| copy().map(|_| &mut copied))
        } else {
            copy().map(|_| &mut copied)
        };
        match r {
            Ok(n) => {
                *n += 1;
                in_a_row = 0;
                if to_copy.remove(path.as_str()).is_some() {
                    copied_bytes += pl.size;
                }
                placed.entry(path).or_insert(dest);
            }
            Err(e) => {
                if let Some(why) = give_up(&mut in_a_row, path, e.to_string(), say) {
                    return Err(why);
                }
            }
        }
    }
    // Everything written above reaches the stick before the step counts as done.
    crate::sys::sync_disks();
    say(format!(
        "first start: kit folders: {kept} sounds there already, {moved} moved from Akai's own \
         folders, {linked} shared with a kit before, {copied} copied from the backup"
    ));
    // What is left of the earlier layout (Akai's own folders, MPC Documents
    // with them) goes to the Trash, where Put back undoes it: a folder the
    // layout doesn't have, at any depth, and a file right in Akai factory.
    // A file the layout doesn't name inside one of its folders stays.
    let mut expected: HashMap<&[String], HashSet<String>> = HashMap::new();
    for pl in &layout {
        for d in 0..pl.folder.len() {
            expected
                .entry(&pl.folder[..d])
                .or_default()
                .insert(pl.folder[d].to_lowercase());
        }
        expected
            .entry(&pl.folder[..])
            .or_default()
            .insert(pl.name.to_lowercase());
    }
    let mut swept: Vec<(&[String], &HashSet<String>)> =
        expected.iter().map(|(f, w)| (*f, w)).collect();
    swept.sort_by_key(|(f, _)| *f);
    for (f, want) in swept {
        let Ok(at) = f.iter().try_fold(top.clone(), |a, n| a.join(n)) else {
            continue;
        };
        for e in lib.list(&at).unwrap_or_default() {
            if want.contains(&e.name.to_lowercase()) || (!f.is_empty() && !e.kind.is_folder()) {
                continue;
            }
            match at.join(&e.name).and_then(|l| lib.delete(&l)) {
                Ok(_) => say(format!(
                    "first start: kit folders: {} is from before; moved to the Trash",
                    at.join(&e.name).map_or(e.name.clone(), |l| l.display())
                )),
                Err(err) => say(format!("first start: kit folders: {}: {err}", e.name)),
            }
        }
    }
    bar.finish(Stage::Samples);
    Ok(kept + moved + linked + copied)
}

/// Says why a sample was not copied; past [`GIVE_UP_AFTER`] failures in a
/// row, why the samples step stops.
fn give_up(
    in_a_row: &mut usize,
    path: &str,
    e: String,
    say: &mut dyn FnMut(String),
) -> Option<String> {
    say(format!("first start: {path}: {e}"));
    *in_a_row += 1;
    (*in_a_row >= GIVE_UP_AFTER).then(|| {
        format!(
            "{} samples in a row could not be copied (the last: {path}: {e}); \
             the stick may have stopped taking writes",
            *in_a_row
        )
    })
}

/// The kits to play: the Akai kits first, then MPFree's own, at most
/// `max` in all.
pub fn combine<T>(akai: Vec<T>, own: Vec<T>, max: usize) -> Vec<T> {
    akai.into_iter().chain(own).take(max).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Moves a file out of the way, as the stick's root service does with
    /// the request (the pad player's code, tests included, never deletes
    /// files: a static check in CI holds it to that).
    fn take_away(path: &Path) {
        std::fs::rename(path, path.with_extension("taken")).unwrap();
    }

    struct Tmp(PathBuf);
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn tmp(tag: &str) -> Tmp {
        let p = std::env::temp_dir().join(format!("mpfree-setup-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(p.join("data")).unwrap();
        std::fs::create_dir_all(p.join("run")).unwrap();
        Tmp(p)
    }

    fn paths(t: &Tmp) -> Paths {
        std::fs::write(t.0.join("setup-backup.sh"), "# service\n").unwrap();
        Paths {
            data: t.0.join("data"),
            service: t.0.join("setup-backup.sh"),
            status: t.0.join("run/status"),
            request: t.0.join("run/setup-backup.request"),
        }
    }

    fn quick() -> Timing {
        Timing {
            poll: Duration::from_millis(5),
            answer: Duration::from_millis(100),
            backup: Duration::from_secs(5),
        }
    }

    fn run_it(p: &Paths, saved: &Settings) -> (Outcome, Vec<Show>, Vec<String>) {
        let (o, shown, said, _) = run_saving(p, saved);
        (o, shown, said)
    }

    /// `run`, and every save it made along the way.
    fn run_saving(p: &Paths, saved: &Settings) -> (Outcome, Vec<Show>, Vec<String>, Vec<Settings>) {
        let (mut shown, mut said) = (Vec::new(), Vec::new());
        let mut saves = Vec::new();
        let o = run(
            p,
            saved,
            &quick(),
            &mut |s| shown.push(s),
            &mut |l| said.push(l),
            &mut |s| saves.push(s.clone()),
        );
        // The try's number is kept before anything runs, and every save
        // after it keeps what the one before it had.
        assert_eq!(
            saves.first().and_then(|s| s.get_i64("tries")),
            Some(saved.get_i64("tries").unwrap_or(0) + 1)
        );
        for w in saves.windows(2) {
            let (a, b) = (done_steps(&w[0]), done_steps(&w[1]));
            assert!(a.iter().all(|d| b.contains(d)), "{a:?} then {b:?}");
        }
        assert_eq!(
            saves.last().map(done_steps),
            Some(done_steps(&o.saved)),
            "the last save has every step done"
        );
        (o, shown, said, saves)
    }

    #[test]
    fn status_lines() {
        let s =
            parse_status("state=running\nstep=copying\ndone=10\ntotal=40\nmessage=Backing up\n")
                .unwrap();
        assert_eq!(
            (s.state.as_str(), s.step.as_str(), s.done, s.total),
            ("running", "copying", 10, 40)
        );
        assert_eq!(s.folder, None);
        assert!(parse_status("").is_none());
        let s = parse_status("state=done\nfolder=/x/backup/y\n").unwrap();
        assert_eq!(s.folder, Some(PathBuf::from("/x/backup/y")));
    }

    #[test]
    fn needed_only_with_the_service_and_until_done() {
        let t = tmp("needed");
        let p = paths(&t);
        let mut s = Settings::default();
        assert!(needed(&p, &s));
        s.set_i64("tries", MAX_TRIES);
        assert!(!needed(&p, &s), "gives up after {MAX_TRIES} tries");
        let mut s = Settings::default();
        s.set_str("done", "backup,kits,kit folders");
        assert!(!needed(&p, &s));
        s.set_str("done", "backup,kits");
        assert!(needed(&p, &s), "a step not done yet");
        s.set_str("done", "backup,kits,samples");
        assert!(needed(&p, &s), "set up before kit folders");
        take_away(&p.service);
        assert!(
            !needed(&p, &Settings::default()),
            "only on the creator's sticks"
        );
    }

    #[test]
    fn kits_and_samples_from_the_backup() {
        let t = tmp("done");
        let p = paths(&t);
        let folder = p.data.join("backup/mpc-one-emmc-20261115T120000Z");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(
            folder.join("mmcblk0.img"),
            mpfree_factory::import::testdisk::disk(),
        )
        .unwrap();
        std::fs::write(
            &p.status,
            format!("state=done\nstep=done\nfolder={}\n", folder.display()),
        )
        .unwrap();
        let (o, shown, said) = run_it(&p, &Settings::default());
        assert_eq!(o.failed, None, "{said:?}");
        assert!(p.request.exists(), "the backup was asked for");
        assert_eq!(o.saved.get_str("state"), Some("done"));
        assert_eq!(o.saved.get_str("done"), Some("backup,kits,kit folders"));
        assert_eq!(
            o.saved.get_i64("tries"),
            Some(0),
            "a later step gets its own tries"
        );
        assert!(!needed(&p, &o.saved));
        let kits = o.kits.expect("a kits file");
        assert_eq!(kits, p.data.join("library/Kits").join(KITS_FILE));
        let n = o.saved.get_i64("akai-kits").unwrap();
        assert!(n >= 1);
        let bytes = std::fs::read(&kits).unwrap();
        assert_eq!(crate::kit::parse_kits(&bytes).unwrap().len() as i64, n);
        let samples = o.saved.get_i64("akai-samples").unwrap();
        // The test disk's kits (16 + 16 + 12 + 4), keys (2), loop and stem.
        assert_eq!(samples, 52);
        let top = p.data.join("library/Samples").join(SAMPLES_FOLDER);
        let mut found = 0;
        let mut stack = vec![top];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).unwrap() {
                let e = e.unwrap();
                if e.file_type().unwrap().is_dir() {
                    stack.push(e.path());
                } else {
                    found += 1;
                }
            }
        }
        assert_eq!(found as i64, samples);
        assert!(shown.iter().any(|s| s.message == EXTRACTING));
        assert_eq!(shown.last().unwrap().message, "Setup done");
        // Run again (as if the store had not saved): nothing is copied twice.
        let (o, _, _) = run_it(&p, &Settings::default());
        assert_eq!(o.saved.get_i64("akai-samples"), Some(samples));
        assert!(!p
            .data
            .join("library/Kits/Akai factory kits 2.mpfkits")
            .exists());
        // A stick set up by an older version, before a step was added: only
        // that step runs, from the backup it has (none is asked for).
        let mut older = o.saved.clone();
        older.set_str("done", "backup,kits");
        assert!(needed(&p, &older));
        take_away(&p.request);
        take_away(&p.status);
        let (o, _, said) = run_it(&p, &older);
        assert_eq!(o.failed, None, "{said:?}");
        assert!(!p.request.exists(), "no new backup");
        assert!(said
            .iter()
            .any(|l| l.contains("using the backup made before")));
        assert_eq!(o.saved.get_str("done"), Some("backup,kits,kit folders"));
        // A sample cut short (the power went before the one sync at the end) is
        // copied again, and the short one goes to the Trash: twice, as Alpha
        // Kit's sounds are Alpha-Kit 2's too.
        let mut one = p.data.join("library/Samples").join(SAMPLES_FOLDER);
        while one.is_dir() {
            let mut names: Vec<_> = std::fs::read_dir(&one)
                .unwrap()
                .map(|e| e.unwrap().path())
                .collect();
            names.sort();
            one = names.remove(0);
        }
        let whole = std::fs::read(&one).unwrap();
        std::fs::write(&one, &whole[..whole.len() / 2]).unwrap();
        let (o, _, said) = run_it(&p, &older);
        assert_eq!(o.failed, None, "{said:?}");
        assert_eq!(o.saved.get_i64("akai-samples"), Some(samples));
        assert_eq!(std::fs::read(&one).unwrap(), whole);
        assert!(
            one.ends_with("Akai factory/Alpha Kit/Hit 01.wav"),
            "{one:?}"
        );
        let twin = one
            .parent()
            .unwrap()
            .with_file_name("Alpha-Kit 2")
            .join("Hit 01.wav");
        assert_eq!(std::fs::read(&twin).unwrap(), whole);
        let lib = Library::at(&p.data).unwrap();
        assert_eq!(lib.trash().unwrap().len(), 2);
    }

    /// The test disk's backup in a temporary stick's backup folder; the store's settings after the backup step.
    fn with_backup(p: &Paths) -> Settings {
        let folder = p.data.join("backup/mpc-one-emmc-20261115T120000Z");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(
            folder.join("mmcblk0.img"),
            mpfree_factory::import::testdisk::disk(),
        )
        .unwrap();
        let mut s = Settings::default();
        s.set_str("backup", &folder.display().to_string());
        s.set_str("done", "backup");
        s
    }

    #[test]
    fn each_kit_in_a_folder_of_its_own() {
        let c = Content::from_disk(mpfree_factory::import::testdisk::disk()).unwrap();
        let mut said = Vec::new();
        let layout = akai_layout(&c, &mut |l| said.push(l));
        let mut folders: Vec<String> = layout.iter().map(|p| p.folder.join("/")).collect();
        folders.dedup();
        assert_eq!(
            folders,
            [
                "Alpha Kit",
                "Alpha-Kit 2",
                "Beta Kit",
                "Gamma",
                "Instruments/Soft Keys",
                "Other sounds/Demo One",
                "Other sounds/Loops"
            ]
        );
        let alpha: Vec<&str> = layout
            .iter()
            .filter(|p| p.folder == ["Alpha Kit"])
            .map(|p| p.name.as_str())
            .collect();
        assert_eq!(alpha.len(), 16);
        assert_eq!(alpha[0], "Hit 01.wav");
        assert!(layout
            .iter()
            .all(|p| !p.source.starts_with("/MPC Documents/")));
        assert_eq!(
            said,
            ["4 kits, 1 instruments, 2 sounds in no program; 0 programs the same as one before; \
              1 sounds in MPC Documents left out"]
        );
        // The same disk, the same layout.
        assert_eq!(akai_layout(&c, &mut |_| {}), layout);
    }

    #[test]
    fn a_stick_set_up_before_moves_its_samples_into_kit_folders() {
        use std::os::unix::fs::MetadataExt;
        let t = tmp("before");
        let p = paths(&t);
        let mut older = with_backup(&p);
        older.set_str("done", "backup,kits,samples");
        // The earlier layout: Akai's own folders, MPC Documents too.
        let c = Content::from_disk(mpfree_factory::import::testdisk::disk()).unwrap();
        let top = p.data.join("library/Samples").join(SAMPLES_FOLDER);
        for (path, _) in c.samples() {
            let to = top.join(path.trim_start_matches('/'));
            std::fs::create_dir_all(to.parent().unwrap()).unwrap();
            let mut out = Vec::new();
            c.copy(&path, &mut out).unwrap();
            std::fs::write(&to, out).unwrap();
        }
        let ino = |p: &Path| std::fs::metadata(p).unwrap().ino();
        let hit = ino(&top.join("Drums/Alpha/Hit 01.wav"));
        let keys = ino(&top.join("Instruments/Keys/Keys C3.wav"));
        assert!(needed(&p, &older));
        let (o, _, said) = run_it(&p, &older);
        assert_eq!(o.failed, None, "{said:?}");
        assert!(!p.request.exists(), "no new backup");
        assert_eq!(o.saved.get_str("done"), Some("backup,kits,kit folders"));
        assert_eq!(o.saved.get_i64("akai-samples"), Some(52));
        let kits = o.saved.get_i64("akai-kits").unwrap();
        assert!(kits > 0);
        assert!(
            said.iter().any(|l| l.ends_with(
                "0 sounds there already, 36 moved from Akai's own folders, \
                 16 shared with a kit before, 0 copied from the backup"
            )),
            "{said:#?}"
        );
        // Moved, not copied: the same files, under the kits' folders; a
        // sound in two kits is one file.
        assert_eq!(ino(&top.join("Alpha Kit/Hit 01.wav")), hit);
        assert_eq!(ino(&top.join("Alpha-Kit 2/Hit 01.wav")), hit);
        assert_eq!(ino(&top.join("Instruments/Soft Keys/Keys C3.wav")), keys);
        // What is left of Akai's folders went to the Trash.
        let mut names: Vec<String> = std::fs::read_dir(&top)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        assert_eq!(
            names,
            [
                "Alpha Kit",
                "Alpha-Kit 2",
                "Beta Kit",
                "Gamma",
                "Instruments",
                "Other sounds"
            ]
        );
        let mut inst: Vec<String> = std::fs::read_dir(top.join("Instruments"))
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        inst.sort();
        assert_eq!(inst, ["Soft Keys"], "Akai's old Instruments/Keys went too");
        let lib = Library::at(&p.data).unwrap();
        let mut gone: Vec<String> = lib.trash().unwrap().into_iter().map(|t| t.name).collect();
        gone.sort();
        assert_eq!(
            gone,
            ["Drums", "Expansions", "Keys", "Loops", "MPC Documents"]
        );
        // Again: nothing moves, nothing is copied, and the kits file kept
        // still counts its kits (round 5's log said "done: 0 Akai kits").
        let (o, _, said) = run_it(&p, &older);
        assert_eq!(o.failed, None, "{said:?}");
        assert_eq!(o.saved.get_i64("akai-kits"), Some(kits));
        assert!(
            said.iter()
                .any(|l| l.contains(&format!("is there already ({kits} kits); kept"))
                    && l.starts_with("first start: ")),
            "{said:#?}"
        );
        assert!(
            said.iter()
                .any(|l| l.starts_with(&format!("first start: done: {kits} Akai kits, 52 samples"))),
            "{said:#?}"
        );
        assert!(said.iter().any(|l| l.ends_with(
            "52 sounds there already, 0 moved from Akai's own folders, \
             0 shared with a kit before, 0 copied from the backup"
        )));
        assert_eq!(lib.trash().unwrap().len(), 5);
    }

    #[test]
    fn a_backup_that_is_gone_is_made_again() {
        let t = tmp("gone");
        let p = paths(&t);
        let mut s = Settings::default();
        s.set_str("done", "backup");
        s.set_str(
            "backup",
            &p.data.join("backup/mpc-one-emmc-x").display().to_string(),
        );
        let (o, _, _) = run_it(&p, &s);
        assert!(p.request.exists(), "asked for a new backup");
        assert_eq!(
            o.failed.as_deref(),
            Some("the backup service did not answer")
        );
    }

    #[test]
    fn a_failed_or_missing_backup_is_tried_again_later() {
        let t = tmp("fail");
        let p = paths(&t);
        let (o, _, _) = run_it(&p, &Settings::default());
        assert_eq!(
            o.failed.as_deref(),
            Some("the backup service did not answer")
        );
        assert_eq!(o.saved.get_str("state"), Some("failed"));
        assert!(needed(&p, &o.saved), "tried again at the next start");
        std::fs::write(
            &p.status,
            "state=failed\nstep=checking\nmessage=partition 4 has 10 MiB free\n",
        )
        .unwrap();
        let (o, _, _) = run_it(&p, &o.saved);
        assert_eq!(
            o.failed.as_deref(),
            Some("the backup did not work: partition 4 has 10 MiB free")
        );
        assert_eq!(o.saved.get_i64("tries"), Some(2));
        // A folder outside partition 4's backup/ is never read.
        std::fs::write(&p.status, "state=done\nfolder=/etc\n").unwrap();
        let (o, _, _) = run_it(&p, &o.saved);
        assert_eq!(
            o.failed.as_deref(),
            Some("the backup is not on partition 4 (/etc)")
        );
        assert!(!needed(&p, &o.saved), "three tries, then it gives up");
    }

    #[test]
    fn one_bar_for_the_whole_first_start() {
        // 1 GB to back up: guesses of 83.3 s copying, 28.6 s reading back,
        // 12 s of kits and 600 s of samples.
        let (copy, verify): (f64, f64) = (1e9 / 12e6, 1e9 / 35e6);
        let rest = verify + 12.0 + 600.0;
        let mut e = Estimate::new(1_000_000_000);
        assert_eq!(e.value(0.0), 0.0);
        assert_eq!(e.left(5.0), None, "no time left before a rate is measured");
        e.at(Stage::Copy, 0, 0.0);
        e.at(Stage::Copy, 100_000_000, 5.0);
        let v = e.value(5.0);
        assert!(
            (v as f64 - copy / 10.0 / (copy + rest)).abs() < 1e-4,
            "{v}: a tenth of the copy's share"
        );
        // Measured after 10 s: 20 MB/s, so the copy's share shrinks to 50 s.
        e.at(Stage::Copy, 200_000_000, 10.0);
        let v = e.value(10.0);
        assert!((v as f64 - 10.0 / (50.0 + rest)).abs() < 1e-4, "{v}");
        let left = e.left(10.0).unwrap();
        assert!((left - (40.0 + rest)).abs() < 1e-3, "{left}");
        // A slower stretch never takes the bar back.
        e.at(Stage::Copy, 210_000_000, 30.0);
        assert!(e.value(30.0) >= v);
        e.finish(Stage::Copy, 50.0);
        e.at(Stage::Verify, 500_000_000, 60.0);
        let before = e.value(60.0);
        e.work(Stage::Samples, 6_000_000);
        assert!(
            e.value(60.0) > before,
            "fewer samples than guessed: further along"
        );
        e.finish(Stage::Verify, 70.0);
        e.finish(Stage::Kits, 80.0);
        assert!(
            e.value(80.0) <= 0.99,
            "short of the end until every stage is over"
        );
        e.finish(Stage::Samples, 81.0);
        assert_eq!(e.value(81.0), 1.0);
        assert_eq!(e.left(81.0), Some(0.0));
    }

    #[test]
    fn a_finished_backup_on_the_stick_is_used_when_the_store_names_none() {
        // The third creator stick: its backup was made and checked, but the
        // store never saved that, and a later start was cut off mid-copy.
        let t = tmp("found");
        let p = paths(&t);
        let folder = p.data.join("backup/mpc-one-emmc-20261116T014409Z");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(
            folder.join("mmcblk0.img"),
            mpfree_factory::import::testdisk::disk(),
        )
        .unwrap();
        let half = p.data.join("backup/mpc-one-emmc-20261116T024543Z.part");
        std::fs::create_dir_all(&half).unwrap();
        std::fs::write(half.join("mmcblk0.img"), b"cut off").unwrap();
        // Not finished without its checksums and manifest: a backup is asked for.
        assert_eq!(finished_backup(&p), None);
        for f in ["SHA256SUMS", "MANIFEST.txt"] {
            std::fs::write(folder.join(f), b"").unwrap();
        }
        std::fs::write(half.join("SHA256SUMS"), b"").unwrap();
        std::fs::write(half.join("MANIFEST.txt"), b"").unwrap();
        assert_eq!(finished_backup(&p), Some(folder.clone()));
        let (o, _, said, saves) = run_saving(&p, &Settings::default());
        assert_eq!(o.failed, None, "{said:?}");
        assert!(!p.request.exists(), "no new backup");
        assert!(said
            .iter()
            .any(|l| l.contains("using the backup made before")));
        assert_eq!(o.saved.get_str("backup"), Some(folder.to_str().unwrap()));
        assert_eq!(o.saved.get_str("done"), Some("backup,kits,kit folders"));
        // Saved as it went: the try, then the backup before the kits ran.
        assert_eq!(done_steps(&saves[0]), Vec::<String>::new());
        assert_eq!(done_steps(&saves[1]), vec!["backup".to_string()]);
    }

    #[test]
    fn the_samples_step_stops_after_many_failures_in_a_row() {
        let (mut n, mut said) = (0, Vec::new());
        for i in 1..GIVE_UP_AFTER {
            assert_eq!(
                give_up(&mut n, "/a.wav", "no".into(), &mut |l| said.push(l)),
                None,
                "{i}"
            );
        }
        let why = give_up(&mut n, "/b.wav", "Read-only file system".into(), &mut |l| {
            said.push(l)
        })
        .unwrap();
        assert!(
            why.starts_with("20 samples in a row could not be copied (the last: /b.wav: Read-only")
        );
        assert_eq!(said.len(), GIVE_UP_AFTER);
    }

    #[test]
    fn a_backup_made_before_counts_for_nothing() {
        let mut e = Estimate::new(1_000_000_000);
        e.finish(Stage::Copy, 0.0);
        e.finish(Stage::Verify, 0.0);
        assert_eq!(e.value(0.0), 0.0, "the kits and samples are the whole bar");
        assert_eq!(e.left(0.0), None);
        e.at(Stage::Kits, 0, 0.0);
        e.at(Stage::Kits, 12, 6.0);
        assert!(
            (e.value(6.0) - 6.0 / 612.0).abs() < 1e-4,
            "half the kits: 6 of 612 s"
        );
    }

    #[test]
    fn says_how_long_is_left() {
        assert_eq!(time_left(0.3), "about 5 s left");
        assert_eq!(time_left(41.0), "about 45 s left");
        assert_eq!(time_left(55.0), "about 1 min left");
        assert_eq!(time_left(13.2 * 60.0), "about 14 min left");
        assert_eq!(of(2_100_000_000, 7_828_000_000), "2.1 of 7.8 GB");
        assert_eq!(of(120_000_000, 480_000_000), "120 of 480 MB");
    }

    #[test]
    fn the_kits_use_the_mpc_ones_pad_notes() {
        assert_eq!(
            mpfree_factory::kitfile::PAD_NOTES,
            mpfree_controller::mpc_one::PAD_NOTES
        );
    }

    #[test]
    fn akai_kits_come_first() {
        assert_eq!(combine(vec![1, 2], vec![3, 4, 5], 4), vec![1, 2, 3, 4]);
        assert_eq!(combine(Vec::<u8>::new(), vec![3], 16), vec![3]);
    }
}
