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
//!    in Akai's folders, where BROWSE finds them. Both are written only
//!    through `mpfree-library`.
//! 3. **Done** is kept in the store (`settings/setup.conf`), step by step
//!    ([`STEPS`]), so each step runs once. A failed attempt is tried again
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

use mpfree_diskimage::Offset;
use mpfree_factory::import::{Content, DEFAULT_KITS};
use mpfree_library::{clean_name, Library, Loc, Place};
use mpfree_store::Settings;
use std::fs::{File, OpenOptions};
use std::io::ErrorKind;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

/// The store's owner name: `settings/setup.conf`.
pub const OWNER: &str = "setup";
/// The Akai kits, in the library's Kits place.
pub const KITS_FILE: &str = "Akai factory kits.mpfkits";
/// Akai's samples, in the library's Samples place.
pub const SAMPLES_FOLDER: &str = "Akai factory";
/// The stick's backup service; without it there is no first-start setup.
pub const SERVICE: &str = "/etc/mpfree-test/setup-backup.sh";
/// What the service reports.
pub const STATUS: &str = "/run/mpfree-setup/status";
/// The request's name, in the program's own folder in /run.
pub const REQUEST: &str = "setup-backup.request";
/// Starts that try the setup before it gives up until the stick is made again.
pub const MAX_TRIES: i64 = 3;
/// The setup's steps, in order, as the store records them (`done`).
pub const STEPS: &[&str] = &["backup", "kits", "samples"];
/// The largest sample copied (the library's own limit is higher).
const MAX_SAMPLE: u64 = 512 << 20;

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
        Estimate {
            parts: [
                part(b, 10e6),
                part(b, 25e6),
                part(24.0, 2.0),
                part(1.5e9, 6e6),
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

/// The backup an earlier start made, if the store names one that is still
/// on partition 4.
fn earlier_backup(paths: &Paths, saved: &Settings) -> Option<PathBuf> {
    if !done_steps(saved).iter().any(|d| d == "backup") {
        return None;
    }
    let folder = PathBuf::from(saved.get_str("backup")?);
    (folder.parent() == Some(&paths.data.join("backup")) && image_in(&folder).is_file())
        .then_some(folder)
}

/// Runs the steps not done yet. `saved` is what the store holds now.
pub fn run(
    paths: &Paths,
    saved: &Settings,
    timing: &Timing,
    show_f: &mut dyn FnMut(Show),
    say: &mut dyn FnMut(String),
) -> Outcome {
    let mut out = saved.clone();
    let tries = saved.get_i64("tries").unwrap_or(0) + 1;
    out.set_i64("tries", tries);
    say(format!("first start: setup, try {tries} of {MAX_TRIES}"));
    let mut done = done_steps(saved);
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
            mark(&mut out, &mut done, "samples");
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

/// The Akai kits from the backup: (the kits file, kits made).
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
            say(format!("first start: {KITS_FILE} is there already; kept"));
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
                    made = b
                        .get(8..12)
                        .map_or(0, |n| u32::from_le_bytes([n[0], n[1], n[2], n[3]]) as usize);
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

/// Akai's samples from the backup, in Akai's folders: how many are in the library.
fn extract_samples(
    content: &Content<Offset<File>>,
    lib: &Library,
    bar: &mut Bar,
    show_f: &mut dyn FnMut(Show),
    say: &mut dyn FnMut(String),
) -> Result<usize, String> {
    let samples = content.samples();
    let top = folder_in(lib, &Loc::top(Place::Samples), SAMPLES_FOLDER)
        .map_err(|e| format!("the samples folder: {e}"))?;
    let mut copied = 0;
    // The bar follows the bytes copied (the samples left out count as done).
    let total: u64 = samples.iter().map(|(_, size)| *size).sum();
    bar.est.work(Stage::Samples, total);
    let mut moved = 0u64;
    let mut last_show: Option<Instant> = None;
    for (path, size) in samples.iter() {
        if last_show.is_none_or(|t| t.elapsed() >= Duration::from_millis(250)) {
            bar.at(Stage::Samples, moved);
            let value = bar.value();
            show(
                show_f,
                EXTRACTING,
                format!("Copying samples: {}{}", of(moved, total), bar.left()),
                value,
            );
            last_show = Some(Instant::now());
        }
        moved += size;
        if *size > MAX_SAMPLE {
            say(format!("first start: {path} left out ({size} bytes)"));
            continue;
        }
        let mut parts: Vec<String> = path
            .split('/')
            .filter(|p| !p.is_empty())
            .map(|p| clean_name(p, "sample"))
            .collect();
        let Some(name) = parts.pop() else { continue };
        let mut at = top.clone();
        let mut ok = true;
        for p in &parts {
            match folder_in(lib, &at, p) {
                Ok(l) => at = l,
                Err(e) => {
                    say(format!("first start: {path}: folder {p}: {e}"));
                    ok = false;
                    break;
                }
            }
        }
        if !ok {
            continue;
        }
        // Kept from an earlier try that was cut off.
        if at.join(&name).is_ok_and(|l| lib.file_path(&l).is_ok()) {
            copied += 1;
            continue;
        }
        match content
            .read(path, MAX_SAMPLE)
            .and_then(|b| lib.write_new(&at, &name, &b))
        {
            Ok(_) => copied += 1,
            Err(e) => say(format!("first start: {path}: {e}")),
        }
    }
    bar.finish(Stage::Samples);
    Ok(copied)
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
        let (mut shown, mut said) = (Vec::new(), Vec::new());
        let o = run(p, saved, &quick(), &mut |s| shown.push(s), &mut |l| {
            said.push(l)
        });
        (o, shown, said)
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
        s.set_str("done", "backup,kits,samples");
        assert!(!needed(&p, &s));
        s.set_str("done", "backup,kits");
        assert!(needed(&p, &s), "a step not done yet");
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
        assert_eq!(o.saved.get_str("done"), Some("backup,kits,samples"));
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
        assert!(samples >= 4, "{samples}");
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
        assert_eq!(o.saved.get_str("done"), Some("backup,kits,samples"));
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
        // 1 GB to back up: guesses of 100 s copying, 40 s reading back, 12 s
        // of kits and 250 s of samples.
        let mut e = Estimate::new(1_000_000_000);
        assert_eq!(e.value(0.0), 0.0);
        assert_eq!(e.left(5.0), None, "no time left before a rate is measured");
        e.at(Stage::Copy, 0, 0.0);
        e.at(Stage::Copy, 100_000_000, 5.0);
        let v = e.value(5.0);
        assert!(
            (v - 10.0 / 402.0).abs() < 1e-3,
            "{v}: a tenth of the copy's share"
        );
        // Measured after 10 s: 20 MB/s, so the copy's share shrinks to 50 s.
        e.at(Stage::Copy, 200_000_000, 10.0);
        let v = e.value(10.0);
        assert!((v - 10.0 / 352.0).abs() < 1e-3, "{v}");
        let left = e.left(10.0).unwrap();
        assert!((left - (40.0 + 40.0 + 12.0 + 250.0)).abs() < 1e-6, "{left}");
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
    fn a_backup_made_before_counts_for_nothing() {
        let mut e = Estimate::new(1_000_000_000);
        e.finish(Stage::Copy, 0.0);
        e.finish(Stage::Verify, 0.0);
        assert_eq!(e.value(0.0), 0.0, "the kits and samples are the whole bar");
        assert_eq!(e.left(0.0), None);
        e.at(Stage::Kits, 0, 0.0);
        e.at(Stage::Kits, 12, 6.0);
        assert!(
            (e.value(6.0) - 6.0 / 262.0).abs() < 1e-4,
            "half the kits: 6 of 262 s"
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
