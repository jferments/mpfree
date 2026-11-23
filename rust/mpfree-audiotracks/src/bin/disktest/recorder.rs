//! `mpfree-disktest --recorder`: MPFree's own recorder on the stick (D4 in
//! docs/dev/design/disk-streaming.md).
//!
//! D2 and D2b wrote one file and synced it every second. The app records
//! through `mpfree_stream::writer` instead: a file and a checkpoint file
//! for each take, written and synced in rounds every 2 s by default (the
//! recording file protocol, recording-files.md), the rounds spread across
//! the interval or run together, and the interval lengthening by itself
//! when rounds run late. This run puts that writer on the stick: 8 takes of
//! stereo float at 48 kHz (3.07 MB/s in all, D2's rate), fed in real time,
//! while 8 streams of 24-bit read the library with 128 KiB reads (the
//! engine's chunk), in ten steps of about 45 s:
//!
//! 1. to 4. a round every 1, 2, 5 and 10 s, spread across the interval;
//! 5. and 6. every 2 and 5 s, the takes' rounds together;
//! 7. and 8. every 2 and 5 s, spread, with the kernel's writeback limit
//!    raised to 32 MiB ([`super::vm`]); left out if it can't be raised;
//! 9. and 10. every 2 s, spread, the limit as found, with the streams'
//!    reads ahead of the recorder on the disk ([`super::prio`]): at best
//!    effort 0 with the recorder's thread at 7, then in the real-time
//!    class; each left out if the priority can't be set.
//!
//! The recorder doesn't wait for these streams (its gate says nothing
//! plays), so this measures the stick and the writer, not the schedule
//! around playback: the whole app's is D3. Every thread is ordinary
//! (`SCHED_OTHER`), as in D2b, and at the disk priority it started with
//! but in steps 9 and 10, which set it only on threads they start and end.
//!
//! **Files.** It writes only its takes, `recN_ID.wav.recording`,
//! `recN_ID.wav.checkpoint` and, once saved, `recN_ID.wav` (N 1 to 8, ID
//! 32 hex digits), in its own folder, made new for each step and deleted
//! after it: about 126 MiB a step, within the room the write run allows.
//! The library and backup are only read, and checked at the end.
//!
//! For each step: the writer's rounds (late ones, the interval lengthening
//! and shortening, the slowest round), how much each take had unsaved at
//! the most (how far the recording fell behind), its capture memory, frames
//! refused, how long the takes took to finish after the stop and whether
//! each file holds its audio whole, the most written data waiting in memory
//! (`Dirty` in `/proc/meminfo`), and the streams' read times, worst
//! shortfall and the plan's window. Then a summary table.

use super::prio::{self, IoPrio};
use super::read::{self, Config, Fail, Piece, KIB, RATE_24};
use super::vm::{self, Raised, Writeback};
use super::{stopped, watch, MIB, STOP};
use mpfree_stream::fsio::StdFs;
use mpfree_stream::rec::{Meta, RecFormat, RecId, TakeState, DATA_AT};
use mpfree_stream::writer::{self, Gate, RealClock, Recorder, Status, TakeNo, WriterConfig};
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub(super) const TAKES: usize = 8;
const TAKE_RATE: u32 = 48_000;
const FORMAT: RecFormat = RecFormat {
    float: true,
    channels: 2,
    rate: TAKE_RATE,
};
/// The feed's period: what one sound-thread period or a few hand over.
const FEED_S: f64 = 0.01;
/// The takes' status and `Dirty` are looked at every this many feeds.
const LOOK_EVERY: u64 = 10;
/// A stream set this many times its length behind is stopped.
const CAP: f64 = 3.0;
const MEMINFO: &str = "/proc/meminfo";

/// One step: a round every `every` s, spread or together, with the
/// writeback limit raised or as found, at a disk priority.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Step {
    pub(super) every: f64,
    pub(super) spread: bool,
    pub(super) raised: bool,
    pub(super) prio: Prio,
}

/// The disk priority a step's threads run at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Prio {
    /// As each thread started.
    AsFound,
    /// The streams' reads at best effort 0, the recorder's thread at 7.
    ReadsFirst,
    /// The streams' reads in the real-time class; the recorder as found.
    ReadsRealTime,
}

impl Prio {
    /// What the streams' threads are set to.
    pub(super) fn reads(self) -> Option<IoPrio> {
        match self {
            Prio::AsFound => None,
            Prio::ReadsFirst => Some(IoPrio::BestEffort(0)),
            Prio::ReadsRealTime => Some(IoPrio::RealTime(4)),
        }
    }

    /// What the recorder's thread is set to.
    pub(super) fn writer(self) -> Option<IoPrio> {
        match self {
            Prio::ReadsFirst => Some(IoPrio::BestEffort(7)),
            Prio::AsFound | Prio::ReadsRealTime => None,
        }
    }

    fn short(self) -> &'static str {
        match self {
            Prio::AsFound => "f",
            Prio::ReadsFirst => "b",
            Prio::ReadsRealTime => "rt",
        }
    }
}

pub(super) const STEPS: [Step; 10] = [
    Step {
        every: 1.0,
        spread: true,
        raised: false,
        prio: Prio::AsFound,
    },
    Step {
        every: 2.0,
        spread: true,
        raised: false,
        prio: Prio::AsFound,
    },
    Step {
        every: 5.0,
        spread: true,
        raised: false,
        prio: Prio::AsFound,
    },
    Step {
        every: 10.0,
        spread: true,
        raised: false,
        prio: Prio::AsFound,
    },
    Step {
        every: 2.0,
        spread: false,
        raised: false,
        prio: Prio::AsFound,
    },
    Step {
        every: 5.0,
        spread: false,
        raised: false,
        prio: Prio::AsFound,
    },
    Step {
        every: 2.0,
        spread: true,
        raised: true,
        prio: Prio::AsFound,
    },
    Step {
        every: 5.0,
        spread: true,
        raised: true,
        prio: Prio::AsFound,
    },
    Step {
        every: 2.0,
        spread: true,
        raised: false,
        prio: Prio::ReadsFirst,
    },
    Step {
        every: 2.0,
        spread: true,
        raised: false,
        prio: Prio::ReadsRealTime,
    },
];

/// Is `name` one of the run's take files?
pub(super) fn is_take_file(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("rec") else {
        return false;
    };
    let Some((n, rest)) = rest.split_once('_') else {
        return false;
    };
    let n_ok = n.len() == 1 && (b'1'..=b'0' + TAKES as u8).contains(&n.as_bytes()[0]);
    let Some((id, ext)) = rest.split_once('.') else {
        return false;
    };
    n_ok && RecId::parse(id).is_some() && matches!(ext, "wav" | "wav.recording" | "wav.checkpoint")
}

/// The take files in `dir` (none if it is gone).
fn take_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let rd = match fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("{}: {e}", dir.display())),
    };
    let mut out = Vec::new();
    for e in rd.flatten() {
        let n = e.file_name().to_string_lossy().into_owned();
        let m = fs::symlink_metadata(e.path()).map_err(|e| e.to_string())?;
        if is_take_file(&n) && m.file_type().is_file() {
            out.push(e.path());
        }
    }
    out.sort();
    Ok(out)
}

/// Delete a step's take files, whatever happened in it.
fn remove_takes(dir: &Path) -> Result<(), String> {
    for p in take_files(dir)? {
        match fs::remove_file(&p) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("remove {}: {e}", p.display())),
        }
    }
    Ok(())
}

/// What the run needs, found before its folder is made.
pub(super) struct Ready {
    found: read::Found,
    before: read::Snapshot,
    dev: u64,
    set: Config,
    secs: f64,
    settle: f64,
    piece: u64,
    /// The intervals are this many times shorter (a quick run).
    scale: f64,
    vm: PathBuf,
    writeback: Option<Writeback>,
}

/// Find what to read and check there is enough, and read the writeback
/// settings; nothing is written yet. `vm` is None where they are left
/// alone (a desktop test without a stand-in).
pub(super) fn prepare(base: &Path, quick: bool, vm_dir: Option<PathBuf>) -> Result<Ready, Fail> {
    let (speed, streams, kib, secs, settle, piece, scale) = if quick {
        (16.0, 2, 64, 0.4, 0.1, MIB, 20.0)
    } else {
        (1.0, 8, 128, 40.0, 2.0, read::PIECE, 1.0)
    };
    let set = Config {
        streams,
        read: kib * KIB,
        rate: RATE_24 * speed,
        kind: "24-bit",
    };
    let (found, before) = read::find(base, piece)?;
    let dev = fs::symlink_metadata(base)
        .map_err(|e| Fail::Refused(format!("{}: {e}", base.display())))?
        .dev();
    if read::assign(&found.pieces, streams, wanted(&set, secs), set.read).is_none() {
        return Err(Fail::Refused(format!(
            "too little to read in {}/ and {}/: it needs at least {streams} files of 64 KiB or more",
            read::LIBRARY,
            read::BACKUP,
        )));
    }
    let writeback = match &vm_dir {
        Some(d) => Some(vm::read(d).map_err(Fail::Refused)?),
        None => None,
    };
    Ok(Ready {
        found,
        before,
        dev,
        set,
        secs,
        settle,
        piece,
        scale,
        vm: vm_dir.unwrap_or_default(),
        writeback,
    })
}

fn wanted(c: &Config, secs: f64) -> u64 {
    (c.rate * secs) as u64 + c.read as u64
}

/// The takes' bytes a second, all together.
fn rate_bytes() -> f64 {
    f64::from(TAKE_RATE) * FORMAT.block_align() as f64 * TAKES as f64
}

/// The bytes a step's takes write: the settle, the streams, a second over.
pub(super) fn step_bytes(secs: f64, settle: f64) -> u64 {
    ((secs + settle + 1.0) * rate_bytes()) as u64
}

/// The most `Dirty` in `/proc/meminfo`, in KiB.
fn dirty_kib() -> Option<u64> {
    let text = fs::read_to_string(MEMINFO).ok()?;
    parse_dirty(&text)
}

pub(super) fn parse_dirty(text: &str) -> Option<u64> {
    let line = text.lines().find(|l| l.starts_with("Dirty:"))?;
    line.split_whitespace().nth(1)?.parse().ok()
}

/// What one take did.
#[derive(Clone, Debug, Default)]
struct TakeResult {
    unsaved_max_s: f64,
    memory_max: usize,
    nearly_full: bool,
    refused: u64,
    pushed: u64,
    state: Option<TakeState>,
    captured: u64,
    committed: u64,
    write_errors: u32,
    sync_errors: u32,
    rewrites: u32,
    /// The saved file holds the header and every frame.
    whole: bool,
}

/// What one step measured.
#[derive(Default)]
struct Measured {
    takes: Vec<TakeResult>,
    rec_secs: f64,
    finish_s: f64,
    finished: bool,
    rounds: u64,
    late: u64,
    lengthened: u32,
    shortened: u32,
    worst_commit_s: f64,
    interval_end: f64,
    dirty_max_kib: Option<u64>,
    /// The feed's own lateness (how late its thread woke), ms.
    feed_late_ms: f64,
    /// The recording stopped at the most the step may write.
    room_stop: bool,
    streams: Option<read::Result1>,
}

fn take_view(
    rec: &Recorder,
    no: TakeNo,
) -> Result<Option<mpfree_stream::writer::TakeView>, String> {
    match rec.status(no) {
        Some(Status::Take(v)) => Ok(Some(v)),
        Some(Status::Arming) | None => Ok(None),
        Some(Status::Failed(why)) => Err(format!("a take could not be armed: {why}")),
    }
}

/// Look at each take and the dirty data; keep the worst.
fn look(rec: &Recorder, nos: &[TakeNo], m: &mut Measured) -> Result<(), String> {
    for (k, no) in nos.iter().enumerate() {
        if let Some(v) = take_view(rec, *no)? {
            let t = &mut m.takes[k];
            t.unsaved_max_s = t.unsaved_max_s.max(v.unsaved_s);
            t.memory_max = t.memory_max.max(v.memory);
            t.nearly_full |= v.nearly_full;
        }
    }
    if let Some(d) = dirty_kib() {
        m.dirty_max_kib = Some(m.dirty_max_kib.map_or(d, |x| x.max(d)));
    }
    Ok(())
}

/// The recording side of a step: arm the takes, feed them in real time
/// until `on` goes false (or `max_s` have passed: the room the step may
/// use), stop them, wait for them to be saved, and check their files.
/// Runs on its own thread; the streams run on the caller's. The writer's
/// thread is always ended before this returns, so a step's files can be
/// deleted after it.
fn record(
    dir: &Path,
    step: Step,
    scale: f64,
    max_s: f64,
    on: &AtomicBool,
) -> Result<Measured, String> {
    let every = step.every / scale;
    let cfg = WriterConfig {
        interval_s: every,
        max_interval_s: (10.0 / scale).max(every),
        spread: step.spread,
        ..WriterConfig::default()
    };
    let mut rec = Recorder::start(
        Box::new(StdFs::new()),
        Arc::new(RealClock::new()),
        Arc::new(Gate::new()),
        cfg,
    )
    .map_err(|e| format!("the recorder could not start: {e}"))?;
    if let Some(p) = step.prio.writer() {
        let set = prio::thread_named(writer::THREAD, Duration::from_secs(2))
            .ok_or_else(|| {
                format!(
                    "the recorder's thread ({}) was not found to set its disk priority",
                    writer::THREAD
                )
            })
            .and_then(|tid| prio::set(tid, p));
        if let Err(e) = set {
            rec.close();
            return Err(e);
        }
    }
    let r = takes(&mut rec, dir, cfg, max_s, on);
    rec.close();
    r
}

fn takes(
    rec: &mut Recorder,
    dir: &Path,
    cfg: WriterConfig,
    max_s: f64,
    on: &AtomicBool,
) -> Result<Measured, String> {
    let nos: Vec<TakeNo> = (1..=TAKES)
        .map(|k| {
            rec.arm(
                dir,
                &format!("rec{k}"),
                RecId::random(),
                FORMAT,
                Meta::default(),
            )
        })
        .collect();
    let mut m = Measured {
        takes: vec![TakeResult::default(); TAKES],
        ..Measured::default()
    };
    // Wait for every take's files (made and synced as it arms).
    let t = Instant::now();
    loop {
        stopped()?;
        let mut armed = 0;
        for no in &nos {
            if take_view(rec, *no)?.is_some() {
                armed += 1;
            }
        }
        if armed == nos.len() {
            break;
        }
        if t.elapsed() > Duration::from_secs(30) {
            return Err("the takes' files were not made within 30 s".into());
        }
        std::thread::sleep(Duration::from_millis(5));
    }

    let frames = (f64::from(TAKE_RATE) * FEED_S).round() as usize;
    let mut buf = vec![0f32; frames * 2];
    let start = Instant::now();
    let mut n = 0u64;
    let mut late = 0f64;
    let mut err = None;
    while on.load(Ordering::Relaxed) {
        if let Err(e) = stopped() {
            err = Some(e);
            break;
        }
        let due = n as f64 * FEED_S;
        if due >= max_s {
            m.room_stop = true;
            break;
        }
        let now = start.elapsed().as_secs_f64();
        if now < due {
            std::thread::sleep(Duration::from_secs_f64(due - now));
        } else {
            late = late.max((now - due) * 1e3);
        }
        for (i, s) in buf.iter_mut().enumerate() {
            // A quiet saw, different on each channel: any sound will do.
            *s = (((n as usize * frames * 2 + i) % 997) as f32 / 997.0 - 0.5) * 0.1;
        }
        for (k, no) in nos.iter().enumerate() {
            let took = rec.push(*no, &buf) as u64;
            m.takes[k].pushed += took;
            m.takes[k].refused += frames as u64 - took;
        }
        n += 1;
        if n.is_multiple_of(LOOK_EVERY) {
            if let Err(e) = look(rec, &nos, &mut m) {
                err = Some(e);
                break;
            }
        }
    }
    m.rec_secs = start.elapsed().as_secs_f64();
    m.feed_late_ms = late;
    for no in &nos {
        rec.stop(*no);
    }
    // Saved: finished, synced and named. Wait for all of them, six times
    // the longest interval at most (a minute).
    let t = Instant::now();
    let limit = (6.0 * cfg.max_interval_s).max(10.0);
    loop {
        let _ = look(rec, &nos, &mut m);
        let views: Vec<_> = nos
            .iter()
            .map(|no| take_view(rec, *no).ok().flatten())
            .collect();
        let done = views.iter().all(|v| {
            v.is_some_and(|v| matches!(v.state, TakeState::Saved | TakeState::Stopped(_)))
        });
        if done || t.elapsed().as_secs_f64() > limit || STOP.load(Ordering::Relaxed) {
            m.finish_s = t.elapsed().as_secs_f64();
            m.finished = done;
            for (k, v) in views.into_iter().enumerate() {
                if let Some(v) = v {
                    let r = &mut m.takes[k];
                    r.state = Some(v.state);
                    r.captured = v.captured;
                    r.committed = v.committed;
                    r.write_errors = v.stats.write_errors;
                    r.sync_errors = v.stats.sync_errors;
                    r.rewrites = v.stats.rewrites;
                }
            }
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    for (k, no) in nos.iter().enumerate() {
        let r = &mut m.takes[k];
        if r.state == Some(TakeState::Saved) {
            if let Some(p) = rec.final_path(*no) {
                let want = DATA_AT + r.captured * FORMAT.block_align() as u64;
                r.whole = r.committed == r.captured
                    && r.captured == r.pushed
                    && fs::symlink_metadata(&p).is_ok_and(|md| md.len() == want);
            }
        }
    }
    let s = rec.stats();
    m.rounds = s.rounds;
    m.late = s.late;
    m.lengthened = s.lengthened;
    m.shortened = s.shortened;
    m.worst_commit_s = rec.worst_commit();
    m.interval_end = rec.interval();
    for no in &nos {
        rec.forget(*no);
    }
    if let Some(e) = err {
        return Err(e);
    }
    Ok(m)
}

/// Run one step: the recording on its own thread, the streams here once
/// it has settled.
fn run_step(env: &Env, step: Step) -> Result<Measured, String> {
    let on = AtomicBool::new(true);
    let (on, dir, scale, max_s) = (&on, env.dir, env.scale, env.max_s);
    std::thread::scope(|s| {
        let rec = std::thread::Builder::new()
            .name("disktest-rec".into())
            .spawn_scoped(s, move || {
                let r = record(dir, step, scale, max_s, on);
                if r.is_err() {
                    STOP.store(true, Ordering::Relaxed);
                }
                r
            })
            .map_err(|e| format!("the recording's thread could not start: {e}"))?;
        std::thread::sleep(Duration::from_secs_f64(env.settle));
        let streams = match &env.group {
            Some(g) => read::run_config_at(
                env.set,
                g,
                env.dev,
                env.secs,
                CAP * env.secs,
                step.prio.reads(),
            ),
            None => Err("too little to read".to_string()),
        };
        on.store(false, Ordering::Relaxed);
        let rec = rec
            .join()
            .unwrap_or_else(|_| Err("the recording's thread failed".into()));
        let mut m = rec?;
        m.streams = Some(streams?);
        Ok(m)
    })
}

struct Env<'a> {
    dir: &'a Path,
    dev: u64,
    set: Config,
    group: Option<Vec<Vec<&'a Piece>>>,
    secs: f64,
    settle: f64,
    scale: f64,
    /// The longest a step's recording may run: the room it may use.
    max_s: f64,
}

fn step_label(step: Step, scale: f64) -> String {
    format!(
        "A round every {} s, {}, writeback {}",
        trim(step.every / scale),
        if step.spread {
            "spread across the interval"
        } else {
            "the takes together"
        },
        if step.raised {
            "limit raised to 32 MiB"
        } else {
            "as found"
        }
    ) + match step.prio {
        Prio::AsFound => "",
        Prio::ReadsFirst => ", the reads ahead of the recorder on the disk",
        Prio::ReadsRealTime => ", the reads real-time on the disk",
    }
}

/// What a step sets, for its report.
fn prio_line(p: Prio) -> Option<String> {
    let reads = p.reads()?;
    Some(format!(
        "disk priority: the streams' reads {}, the recorder's thread {}; set on this step's threads only, which end with it (the kernel's writeback threads keep theirs)",
        reads.name(),
        p.writer().map_or_else(|| "as found".to_string(), IoPrio::name)
    ))
}

fn trim(x: f64) -> String {
    let s = format!("{x:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// One row of the summary.
enum Row {
    LeftOut(String),
    Ran {
        every: f64,
        spread: bool,
        raised: bool,
        prio: Prio,
        unsaved_max_s: f64,
        worst_commit_s: f64,
        late: u64,
        lengthened: u32,
        finish_s: f64,
        saved: usize,
        short_ms: f64,
        read_max_ms: f64,
        dirty_max_kib: Option<u64>,
    },
}

fn report_step(env: &Env, step: Step, m: &Measured) -> Row {
    let mib = |b: u64| b as f64 / MIB as f64;
    let bytes: u64 = m
        .takes
        .iter()
        .map(|t| t.pushed * FORMAT.block_align() as u64)
        .sum();
    let rate = rate_bytes();
    say!(
        "  recording: {TAKES} takes, stereo float at {} kHz ({:.2} MB/s in all), {:.1} s, {:.1} MiB",
        TAKE_RATE / 1000,
        rate / 1e6,
        m.rec_secs,
        mib(bytes)
    );
    if m.room_stop {
        say!(
            "  the recording stopped at {:.0} s, the most this step may write on the stick",
            env.max_s
        );
    }
    say!(
        "  rounds: {} ({} late); the interval lengthened {} and shortened {} times, {} s at the end; the slowest round {:.3} s",
        m.rounds,
        m.late,
        m.lengthened,
        m.shortened,
        trim(m.interval_end),
        m.worst_commit_s
    );
    let (worst_k, worst) = m
        .takes
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.unsaved_max_s.total_cmp(&b.1.unsaved_max_s))
        .map(|(k, t)| (k + 1, t.unsaved_max_s))
        .unwrap_or((0, 0.0));
    let memory = m.takes.iter().map(|t| t.memory_max).max().unwrap_or(0);
    let refused: u64 = m.takes.iter().map(|t| t.refused).sum();
    say!(
        "  unsaved at the most: {worst:.2} s (take {worst_k}); capture memory at the most {:.1} MiB a take{}; frames refused: {}",
        mib(memory as u64),
        if m.takes.iter().any(|t| t.nearly_full) {
            ", NEARLY FULL"
        } else {
            ""
        },
        if refused == 0 {
            "none".to_string()
        } else {
            format!("{refused} (capture memory full: the take stopped there)")
        }
    );
    let saved = m
        .takes
        .iter()
        .filter(|t| t.state == Some(TakeState::Saved))
        .count();
    let whole = m.takes.iter().filter(|t| t.whole).count();
    say!(
        "  after the stop: {:.2} s to finish; {saved} of {TAKES} takes saved, {whole} of them whole (every frame in the file){}",
        m.finish_s,
        if m.finished { "" } else { ", NOT ALL FINISHED" }
    );
    let errors: u32 = m.takes.iter().map(|t| t.write_errors + t.sync_errors).sum();
    let rewrites: u32 = m.takes.iter().map(|t| t.rewrites).sum();
    if errors > 0 || rewrites > 0 {
        say!("  errors: {errors} write or sync errors, {rewrites} pieces written again");
    }
    for (k, t) in m.takes.iter().enumerate() {
        if t.state != Some(TakeState::Saved) {
            say!("  take {}: ended {:?}", k + 1, t.state);
        }
    }
    match m.dirty_max_kib {
        Some(d) => say!("  written data waiting in memory at the most: {d} KiB (Dirty)"),
        None => say!("  written data waiting in memory: /proc/meminfo could not be read"),
    }
    say!("  the feed woke {:.1} ms late at the most", m.feed_late_ms);
    let (short_ms, read_max_ms) = match &m.streams {
        Some(r) => {
            let w = read::window_s(r.behind_ms);
            say!(
                "  {} streams, {} ({:.1} kB/s each), {} KiB reads",
                env.set.streams,
                env.set.kind,
                env.set.rate / 1e3,
                env.set.read / KIB
            );
            say!("  {}", r.reads.report_tails("read"));
            say!(
                "    worst shortfall {:.1} ms, longest behind {:.1} ms; the plan's window {w:.1} s a stream",
                r.behind_ms,
                r.longest_behind_ms
            );
            if r.cut {
                say!(
                    "    stopped at {:.0} s: the streams could not keep up, so the shortfall is at least this",
                    CAP * env.secs
                );
            }
            if let Some(b) = &r.beside {
                say!("    {}", b.line());
                say!("    {}", b.slow_line());
            }
            (r.behind_ms, r.reads.max())
        }
        None => (f64::NAN, f64::NAN),
    };
    Row::Ran {
        every: step.every / env.scale,
        spread: step.spread,
        raised: step.raised,
        prio: step.prio,
        unsaved_max_s: worst,
        worst_commit_s: m.worst_commit_s,
        late: m.late,
        lengthened: m.lengthened,
        finish_s: m.finish_s,
        saved: whole,
        short_ms,
        read_max_ms,
        dirty_max_kib: m.dirty_max_kib,
    }
}

fn summary(rows: &[Row]) {
    say!("\nSummary: for each step the round's interval (s), the takes' rounds spread (s) or together (t), the writeback limit as found (f) or raised (r), the disk priority as found (f), the reads ahead of the recorder (b) or real-time (rt); the most a take had unsaved and the slowest round (s), late rounds, times the interval lengthened, the finish after the stop (s), takes saved whole; the streams' worst shortfall and slowest read (ms); the most written data waiting in memory (KiB).");
    say!(
        "  {:>4}  {:>5}  {:>2}  {:>2}  {:>4}  {:>7}  {:>7}  {:>4}  {:>6}  {:>6}  {:>5}  {:>7}  {:>7}  {:>8}",
        "step",
        "every",
        "",
        "wb",
        "disk",
        "unsaved",
        "slowest",
        "late",
        "longer",
        "finish",
        "whole",
        "short",
        "max",
        "dirty"
    );
    for (k, row) in rows.iter().enumerate() {
        match row {
            Row::LeftOut(why) => say!("  {:>4}  left out: {why}", k + 1),
            Row::Ran {
                every,
                spread,
                raised,
                prio,
                unsaved_max_s,
                worst_commit_s,
                late,
                lengthened,
                finish_s,
                saved,
                short_ms,
                read_max_ms,
                dirty_max_kib,
            } => say!(
                "  {:>4}  {:>5}  {:>2}  {:>2}  {:>4}  {unsaved_max_s:>7.2}  {worst_commit_s:>7.3}  {late:>4}  {lengthened:>6}  {finish_s:>6.2}  {:>5}  {short_ms:>7.1}  {read_max_ms:>7.1}  {:>8}",
                k + 1,
                trim(*every),
                if *spread { "s" } else { "t" },
                if *raised { "r" } else { "f" },
                prio.short(),
                format!("{saved}/{TAKES}"),
                dirty_max_kib.map_or("?".to_string(), |d| d.to_string())
            ),
        }
    }
}

/// The run, in `dir` (made and emptied by the caller), with at most `room`
/// bytes of its own files at once.
pub(super) fn run(base: &Path, dir: &Path, room: u64, z: Ready) -> Result<(), String> {
    let mib = |b: u64| b as f64 / MIB as f64;
    let found = &z.found;
    say!(
        "recorder: MPFree's own recorder writes {TAKES} takes in {} while {} streams read the audio under {}/{} and the files under {}/{}",
        dir.display(),
        z.set.streams,
        base.display(),
        read::LIBRARY,
        base.display(),
        read::BACKUP
    );
    say!(
        "found: {} in {}/ ({:.1} MiB), {} in {}/ ({:.1} MiB); {} pieces to read",
        read::count(found.files[0], "audio file"),
        read::LIBRARY,
        mib(found.bytes[0]),
        read::count(found.files[1], "file"),
        read::BACKUP,
        mib(found.bytes[1]),
        found.pieces.len()
    );
    let per_step = step_bytes(z.secs, z.settle);
    if per_step > room {
        return Err(format!(
            "a step writes about {:.0} MiB, more than the {:.0} MiB of room",
            mib(per_step),
            mib(room)
        ));
    }
    say!(
        "files: about {:.0} MiB of takes a step, made new for each step and deleted after it",
        mib(per_step)
    );
    match &z.writeback {
        Some(w) => say!("writeback as found: {}", vm::describe(w)),
        None => say!("writeback: left alone (a desktop test without a stand-in)"),
    }
    say!(
        "disk priority as found: {}; the stick's scheduler: {}",
        prio::get(0).map_or_else(|e| e, IoPrio::name),
        match watch::STICK.get() {
            Some(stick) =>
                prio::scheduler(&stick.dir).unwrap_or_else(|e| format!("can't be read ({e})")),
            None => "not known (the stick isn't watched)".to_string(),
        }
    );
    let env = Env {
        dir,
        dev: z.dev,
        set: z.set,
        group: read::assign(
            &found.pieces,
            z.set.streams,
            wanted(&z.set, z.secs),
            z.set.read,
        ),
        secs: z.secs,
        settle: z.settle,
        scale: z.scale,
        max_s: 0.9 * room as f64 / rate_bytes(),
    };
    let mut raised: Option<Raised> = None;
    let mut rows = Vec::new();
    for (k, &step) in STEPS.iter().enumerate() {
        stopped()?;
        say!("\n{}. {}", k + 1, step_label(step, z.scale));
        if step.raised && raised.is_none() {
            let why = match &z.writeback {
                None => Some("the writeback settings are left alone here".to_string()),
                Some(w) => match Raised::raise(&z.vm, *w) {
                    Ok(r) => {
                        say!(
                            "  writeback limit raised to 32 MiB (dirty_bytes {}), until the last of these steps",
                            vm::RAISED
                        );
                        raised = Some(r);
                        None
                    }
                    Err(why) => Some(why),
                },
            };
            if let Some(why) = why {
                say!("  left out: {why}");
                rows.push(Row::LeftOut(why));
                continue;
            }
        }
        if !step.raised {
            if let Some(mut r) = raised.take() {
                r.put_back()?;
                say!(
                    "  writeback limit put back as found first: dirty_bytes {}",
                    z.writeback.map_or(0, |w| w.bytes)
                );
            }
        }
        if let Some(line) = prio_line(step.prio) {
            let wanted: Vec<IoPrio> = step
                .prio
                .reads()
                .into_iter()
                .chain(step.prio.writer())
                .collect();
            if let Err(why) = prio::can_set(&wanted) {
                say!("  left out: {why}");
                rows.push(Row::LeftOut(why));
                continue;
            }
            say!("  {line}");
        }
        let m = run_step(&env, step);
        let removed = remove_takes(dir);
        let m = m?;
        removed?;
        rows.push(report_step(&env, step, &m));
    }
    if let Some(mut r) = raised.take() {
        r.put_back()?;
        say!(
            "\nwriteback limit put back as found: dirty_bytes {}",
            z.writeback.map_or(0, |w| w.bytes)
        );
    }
    summary(&rows);

    let (_, after) = read::find(base, z.piece).map_err(|e| match e {
        Fail::Refused(e) | Fail::Failed(e) => e,
    })?;
    if after == z.before {
        say!(
            "\nthe files read in {}/ and {}/ are as they were",
            read::LIBRARY,
            read::BACKUP
        );
    } else {
        say!(
            "\nfiles in {}/ or {}/ changed during the run (something else saved there); this tool only read them",
            read::LIBRARY,
            read::BACKUP
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_its_own_take_files_are_its_own() {
        let id = "0123456789abcdef0123456789abcdef";
        for ext in ["wav", "wav.recording", "wav.checkpoint"] {
            assert!(is_take_file(&format!("rec1_{id}.{ext}")), "{ext}");
            assert!(is_take_file(&format!("rec8_{id}.{ext}")), "{ext}");
        }
        for bad in [
            format!("rec0_{id}.wav"),
            format!("rec9_{id}.wav"),
            format!("rec10_{id}.wav"),
            format!("rec1_{id}.wav.part"),
            format!("rec1_{id}.tail.unverified.wav"),
            format!("rec1_{}.wav", &id[..31]),
            format!("rec1_{id}x.wav"),
            format!("Vocal_{id}.wav"),
            "rec1_.wav".to_string(),
            "rec.bin".to_string(),
        ] {
            assert!(!is_take_file(&bad), "{bad}");
        }
    }

    #[test]
    fn the_steps_try_every_interval_both_ways_and_the_raised_limit() {
        let found = |s: &&Step| s.prio == Prio::AsFound;
        let every: Vec<f64> = STEPS
            .iter()
            .filter(found)
            .filter(|s| s.spread && !s.raised)
            .map(|s| s.every)
            .collect();
        assert_eq!(every, [1.0, 2.0, 5.0, 10.0]);
        assert!(STEPS.iter().any(|s| !s.spread && s.every == 2.0));
        // The raised steps are together, so the limit is raised once.
        let raised: Vec<usize> = (0..STEPS.len()).filter(|&k| STEPS[k].raised).collect();
        assert_eq!(raised, [6, 7]);
        // The priority steps change only the priority from the writer's
        // default (2 s, spread, step 2), with the limit as found.
        let base = STEPS[1];
        assert_eq!((base.every, base.spread, base.raised), (2.0, true, false));
        let tried: Vec<Prio> = STEPS
            .iter()
            .filter(|s| s.prio != Prio::AsFound)
            .inspect(|s| {
                assert_eq!(
                    Step {
                        prio: Prio::AsFound,
                        ..**s
                    },
                    base
                )
            })
            .map(|s| s.prio)
            .collect();
        assert_eq!(tried, [Prio::ReadsFirst, Prio::ReadsRealTime]);
        assert_eq!(
            (STEPS[8].prio, STEPS[9].prio),
            (Prio::ReadsFirst, Prio::ReadsRealTime)
        );
        // A step's takes fit the write run's room on the stick.
        assert!(step_bytes(40.0, 2.0) < 448 * MIB);
    }

    #[test]
    fn the_priority_steps_put_the_reads_ahead_and_never_raise_the_writer() {
        assert_eq!(Prio::AsFound.reads(), None);
        assert_eq!(Prio::AsFound.writer(), None);
        assert!(prio_line(Prio::AsFound).is_none());
        assert_eq!(Prio::ReadsFirst.reads(), Some(IoPrio::BestEffort(0)));
        assert_eq!(Prio::ReadsFirst.writer(), Some(IoPrio::BestEffort(7)));
        assert_eq!(Prio::ReadsRealTime.reads(), Some(IoPrio::RealTime(4)));
        assert_eq!(Prio::ReadsRealTime.writer(), None);
        assert_eq!(
            prio_line(Prio::ReadsFirst).unwrap(),
            "disk priority: the streams' reads best effort 0, the recorder's thread best effort 7; set on this step's threads only, which end with it (the kernel's writeback threads keep theirs)"
        );
        assert!(step_label(STEPS[9], 1.0).ends_with(", the reads real-time on the disk"));
    }

    #[test]
    fn dirty_is_read_from_meminfo() {
        let text = "MemTotal:        2067188 kB\nDirty:              1012 kB\nWriteback:             0 kB\n";
        assert_eq!(parse_dirty(text), Some(1012));
        assert_eq!(parse_dirty("MemTotal: 1 kB\n"), None);
    }
}
