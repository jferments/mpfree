//! The recording writer, the second recording step
//! (docs/dev/design/recording-files.md): takes' writes, checkpoint rounds
//! and finishes, on a thread of their own and scheduled around playback.
//!
//! - **Waiting for playback.** A round waits while playback's cushion (the
//!   time until a playing cursor needs a unit not yet cached, which the
//!   coordinator shares through a [`Gate`]) is no longer than the stick's
//!   worst commit time, but never more than [`WriterConfig::max_wait_s`]
//!   past its time, and not at all once its take's capture memory is half
//!   full (recording moves up).
//! - **Recording pressure** goes the other way through the same gate: the
//!   fullest take's capture memory, so the coordinator stops background
//!   reads past three quarters.
//! - **Spreading.** Several takes' rounds are spread across the interval,
//!   or run together ([`WriterConfig::spread`]; D4 measures which a real
//!   stick likes better).
//! - **Falling behind.** When a round starts an interval late or takes
//!   longer than the interval, the interval doubles, up to
//!   [`WriterConfig::max_interval_s`]. After 16 rounds in a row on time,
//!   each taking under a quarter of the interval, it halves back towards
//!   the one set. Both are counted for Dev.
//! - **The capture reserve** of each take still recording is its data rate
//!   x (the worst commit time measured + one interval), for
//!   [`crate::coord::Coordinator::set_reserve`].
//! - **"Unsaved: N s"** shows on a lane once its take has more than twice
//!   the interval not yet committed.
//!
//! [`Writer`] is the schedule and the takes, driven a pass at a time by
//! [`Writer::tick`] (the tests drive it on a made-up clock); [`Recorder`]
//! runs one on its own thread, with capture memory counted on the caller's
//! side so a stick that stops answering can't fill memory unseen.

use std::collections::{HashMap, VecDeque};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::fsio::Fs;
use crate::player::UNIT;
use crate::rec::{Meta, RecFormat, RecId, Take, TakeState, TakeStats, Why};

/// Seconds since some start.
pub trait Clock: Send + Sync {
    fn now(&self) -> f64;
}

/// The real clock.
#[derive(Debug)]
pub struct RealClock(Instant);

impl RealClock {
    pub fn new() -> Self {
        RealClock(Instant::now())
    }
}

impl Default for RealClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for RealClock {
    fn now(&self) -> f64 {
        self.0.elapsed().as_secs_f64()
    }
}

/// Playback's side of the schedule: the coordinator's cushion, shared
/// with the writer's thread ([`crate::coord::Coordinator::gate`]).
#[derive(Debug)]
pub struct Gate {
    cushion: AtomicU64,
    capture: AtomicU64,
}

impl Gate {
    /// Nothing playing yet: no limit.
    pub fn new() -> Self {
        Gate {
            cushion: AtomicU64::new(f64::INFINITY.to_bits()),
            capture: AtomicU64::new(0f64.to_bits()),
        }
    }
    pub fn set_cushion(&self, seconds: f64) {
        self.cushion.store(seconds.to_bits(), Ordering::Relaxed);
    }
    /// Seconds until a playing cursor needs a unit not yet cached.
    pub fn cushion(&self) -> f64 {
        f64::from_bits(self.cushion.load(Ordering::Relaxed))
    }
    pub fn set_capture_fill(&self, fill: f64) {
        self.capture.store(fill.to_bits(), Ordering::Relaxed);
    }
    /// How full the fullest recording take's capture memory is, 0 to 1.
    pub fn capture_fill(&self) -> f64 {
        f64::from_bits(self.capture.load(Ordering::Relaxed))
    }
}

impl Default for Gate {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug)]
pub struct WriterConfig {
    /// Seconds between a take's checkpoint rounds (2 s; D2 and D4 try 1,
    /// 2, 5 and 10).
    pub interval_s: f64,
    /// The interval lengthens up to this when rounds fall behind.
    pub max_interval_s: f64,
    /// Spread several takes' rounds across the interval (true), or run
    /// them together.
    pub spread: bool,
    /// The worst commit time assumed until a round is measured.
    pub commit_s: f64,
    /// A round waits for playback at most this long past its time.
    pub max_wait_s: f64,
    /// Capture memory per take, in bytes.
    pub limit: usize,
}

impl Default for WriterConfig {
    fn default() -> Self {
        WriterConfig {
            interval_s: 2.0,
            max_interval_s: 10.0,
            spread: true,
            commit_s: 1.0,
            max_wait_s: 10.0,
            limit: 32 << 20,
        }
    }
}

/// A take's number, chosen by the caller.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TakeNo(pub u32);

/// One take as its lane shows it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TakeView {
    pub state: TakeState,
    pub captured: u64,
    pub committed: u64,
    /// Seconds captured but not yet committed.
    pub unsaved_s: f64,
    /// Show "Unsaved: N s": more than twice the interval is uncommitted.
    pub show_unsaved: bool,
    /// Bytes in capture memory.
    pub memory: usize,
    /// Capture memory is at least 90% full.
    pub nearly_full: bool,
    pub stats: TakeStats,
}

/// The writer's counts, for Dev.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WriterStats {
    /// Rounds run: checkpoints and finishing passes.
    pub rounds: u64,
    /// Rounds that started an interval late or took longer than one.
    pub late: u64,
    /// Times the interval doubled, and halved back.
    pub lengthened: u32,
    pub shortened: u32,
    /// Rounds that waited for playback, and those that went anyway once
    /// they had waited the longest allowed.
    pub waited: u64,
    pub forced: u64,
}

/// How often a round waiting for playback looks again.
const WAIT_POLL: f64 = 0.02;
/// Past this much of its capture memory a take's rounds stop waiting for
/// playback.
pub const MOVE_UP: f64 = 0.5;
/// Rounds measured for the worst commit time.
const MEASURED: usize = 64;
/// Quick rounds in a row before a lengthened interval halves back.
const CALM_ROUNDS: u32 = 16;

struct Entry {
    no: TakeNo,
    take: Take,
    /// When its next round is due.
    due: f64,
    /// This round's wait for playback is counted.
    waiting: bool,
}

/// The takes and their schedule. Every file operation happens inside its
/// methods, so it belongs to one thread (see [`Recorder`]).
pub struct Writer {
    fs: Box<dyn Fs + Send>,
    clock: Arc<dyn Clock>,
    gate: Arc<Gate>,
    cfg: WriterConfig,
    interval: f64,
    takes: Vec<Entry>,
    /// The last rounds' times (those that touched the stick).
    took: VecDeque<f64>,
    on_time: u32,
    stats: WriterStats,
}

impl Writer {
    pub fn new(
        fs: Box<dyn Fs + Send>,
        clock: Arc<dyn Clock>,
        gate: Arc<Gate>,
        cfg: WriterConfig,
    ) -> Self {
        Writer {
            fs,
            clock,
            gate,
            cfg,
            interval: cfg.interval_s,
            takes: Vec::new(),
            took: VecDeque::with_capacity(MEASURED),
            on_time: 0,
            stats: WriterStats::default(),
        }
    }

    /// The interval now (lengthened when rounds fell behind).
    pub fn interval(&self) -> f64 {
        self.interval
    }

    pub fn stats(&self) -> WriterStats {
        self.stats
    }

    /// The worst commit time: the longest of the last 64 rounds that
    /// touched the stick, or the one assumed until there is one.
    pub fn worst_commit(&self) -> f64 {
        if self.took.is_empty() {
            self.cfg.commit_s
        } else {
            self.took.iter().copied().fold(0.0, f64::max)
        }
    }

    fn find(&self, no: TakeNo) -> Option<usize> {
        self.takes.iter().position(|e| e.no == no)
    }

    /// Arm a take (its files are made and synced now: S1 to S3).
    pub fn arm(
        &mut self,
        no: TakeNo,
        dir: &Path,
        name: &str,
        id: RecId,
        fmt: RecFormat,
        meta: Meta,
    ) -> io::Result<()> {
        if self.find(no).is_some() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "that take number is in use",
            ));
        }
        let take = Take::arm(&mut *self.fs, dir, name, id, fmt, meta, self.cfg.limit)?;
        let due = self.first_due(self.clock.now());
        self.takes.push(Entry {
            no,
            take,
            due,
            waiting: false,
        });
        Ok(())
    }

    /// When a new take's first round falls: an interval from now; with
    /// other takes, in the middle of the widest gap between their rounds
    /// (spread) or with the soonest (together).
    fn first_due(&self, now: f64) -> f64 {
        let i = self.interval;
        let mut phases: Vec<f64> = self
            .takes
            .iter()
            .filter(|e| e.take.state() != TakeState::Saved)
            .map(|e| (e.due - now).clamp(0.0, i))
            .collect();
        if phases.is_empty() {
            return now + i;
        }
        if !self.cfg.spread {
            return now + phases.iter().copied().fold(i, f64::min);
        }
        phases.sort_by(f64::total_cmp);
        let last = phases[phases.len() - 1];
        // (gap, where it starts), the one round the end first.
        let mut widest = (phases[0] + i - last, last);
        for w in phases.windows(2) {
            if w[1] - w[0] > widest.0 {
                widest = (w[1] - w[0], w[0]);
            }
        }
        let at = widest.1 + widest.0 / 2.0;
        now + if at >= i { at - i } else { at }
    }

    /// Add captured frames (interleaved) to a take's capture memory;
    /// returns how many it took.
    pub fn push(&mut self, no: TakeNo, frames: &[f32]) -> usize {
        match self.find(no) {
            Some(i) => self.takes[i].take.push(frames),
            None => 0,
        }
    }

    /// Capture memory counted by the caller is full: the take stops
    /// taking frames (and is finished like any other).
    pub fn refuse_more(&mut self, no: TakeNo) {
        if let Some(i) = self.find(no) {
            self.takes[i].take.refuse_more();
        }
    }

    /// The musician stopped: the take is finished at the next pass that
    /// playback can spare the stick.
    pub fn stop(&mut self, no: TakeNo) {
        let now = self.clock.now();
        if let Some(i) = self.find(no) {
            let e = &mut self.takes[i];
            e.take.stop();
            e.due = e.due.min(now);
        }
    }

    /// Give up an armed take that never recorded; its files go.
    pub fn disarm(&mut self, no: TakeNo) -> bool {
        match self.find(no) {
            Some(i) if self.takes[i].take.captured() == 0 => {
                let e = self.takes.remove(i);
                e.take.disarm(&mut *self.fs)
            }
            _ => false,
        }
    }

    /// Let go of a saved take.
    pub fn forget(&mut self, no: TakeNo) -> bool {
        match self.find(no) {
            Some(i) if self.takes[i].take.state() == TakeState::Saved => {
                self.takes.remove(i);
                true
            }
            _ => false,
        }
    }

    pub fn takes(&self) -> Vec<TakeNo> {
        self.takes.iter().map(|e| e.no).collect()
    }

    /// Where a take's finished file is (once Saved).
    pub fn final_path(&self, no: TakeNo) -> Option<PathBuf> {
        self.find(no).map(|i| self.takes[i].take.final_path())
    }

    pub fn format(&self, no: TakeNo) -> Option<RecFormat> {
        self.find(no).map(|i| self.takes[i].take.format())
    }

    /// Bytes in a take's capture memory.
    pub fn memory(&self, no: TakeNo) -> usize {
        self.find(no).map_or(0, |i| self.takes[i].take.memory())
    }

    pub fn view(&self, no: TakeNo) -> Option<TakeView> {
        let t = &self.takes[self.find(no)?].take;
        let fmt = t.format();
        let unsaved_s = t.unsaved() as f64 / f64::from(fmt.rate);
        Some(TakeView {
            state: t.state(),
            captured: t.captured(),
            committed: t.committed(),
            unsaved_s,
            show_unsaved: unsaved_s > 2.0 * self.interval && t.state() != TakeState::Saved,
            memory: t.memory(),
            nearly_full: t.nearly_full(),
            stats: t.stats(),
        })
    }

    /// The capture reserve, in bytes: each take still recording, its data
    /// rate x (the worst commit time + one interval).
    pub fn reserve_bytes(&self) -> usize {
        let per_s: f64 = self
            .takes
            .iter()
            .filter(|e| !e.take.stopping() && e.take.state() != TakeState::Saved)
            .map(|e| {
                let f = e.take.format();
                f64::from(f.rate) * f.block_align() as f64
            })
            .sum();
        (per_s * (self.worst_commit() + self.interval)).ceil() as usize
    }

    /// [`Writer::reserve_bytes`] in cache units, for
    /// [`crate::coord::Coordinator::set_reserve`].
    pub fn reserve_units(&self) -> usize {
        self.reserve_bytes().div_ceil(UNIT * 4)
    }

    /// One pass: every take's filled units go out, then the most overdue
    /// round runs if playback can spare the stick for it. Gives the
    /// seconds until it wants the next pass.
    pub fn tick(&mut self) -> f64 {
        let now = self.clock.now();
        for e in &mut self.takes {
            if e.take.state() == TakeState::Saved {
                continue;
            }
            // Stopped by itself (memory full): finished like a stop.
            if matches!(e.take.state(), TakeState::Stopped(Why::Memory)) && !e.take.stopping() {
                e.take.stop();
                e.due = e.due.min(now);
            }
            e.take.write_ready(&mut *self.fs);
        }
        self.share_fill();
        let Some(i) = self.next_round() else {
            return 1.0;
        };
        let due = self.takes[i].due;
        if due > now {
            return due - now;
        }
        if self.gate.cushion() <= self.worst_commit() && self.takes[i].take.fill() < MOVE_UP {
            let waited = now - due;
            if waited < self.cfg.max_wait_s {
                if !self.takes[i].waiting {
                    self.takes[i].waiting = true;
                    self.stats.waited += 1;
                }
                return WAIT_POLL.min(self.cfg.max_wait_s - waited);
            }
            self.stats.forced += 1;
        }
        self.round(i, due);
        self.share_fill();
        match self.next_round() {
            Some(j) => (self.takes[j].due - self.clock.now()).max(0.0),
            None => 1.0,
        }
    }

    /// Recording pressure for the coordinator: the fullest take's capture
    /// memory.
    fn share_fill(&self) {
        let fill = self
            .takes
            .iter()
            .filter(|e| e.take.state() != TakeState::Saved)
            .map(|e| e.take.fill())
            .fold(0.0, f64::max);
        self.gate.set_capture_fill(fill);
    }

    /// The take whose round is due first.
    fn next_round(&self) -> Option<usize> {
        self.takes
            .iter()
            .enumerate()
            .filter(|(_, e)| e.take.state() != TakeState::Saved)
            .min_by(|a, b| a.1.due.total_cmp(&b.1.due))
            .map(|(i, _)| i)
    }

    fn round(&mut self, i: usize, due: f64) {
        let e = &mut self.takes[i];
        e.waiting = false;
        let before = (e.take.stats(), e.take.committed(), e.take.state());
        let t0 = self.clock.now();
        if e.take.stopping() {
            e.take.finish(&mut *self.fs);
        } else {
            e.take.checkpoint(&mut *self.fs);
        }
        let t1 = self.clock.now();
        let took = t1 - t0;
        let touched = before != (e.take.stats(), e.take.committed(), e.take.state());
        self.stats.rounds += 1;
        if touched {
            if self.took.len() == MEASURED {
                self.took.pop_front();
            }
            self.took.push_back(took);
        }
        if t0 - due > self.interval || took > self.interval {
            self.stats.late += 1;
            self.on_time = 0;
            if self.interval < self.cfg.max_interval_s {
                self.interval = (2.0 * self.interval).min(self.cfg.max_interval_s);
                self.stats.lengthened += 1;
            }
        } else if touched && took < self.interval / 4.0 {
            self.on_time += 1;
            if self.on_time >= CALM_ROUNDS && self.interval > self.cfg.interval_s {
                self.interval = (self.interval / 2.0).max(self.cfg.interval_s);
                self.stats.shortened += 1;
                self.on_time = 0;
            }
        }
        self.takes[i].due = (t0 + self.interval).max(t1);
    }
}

// ---- on its own thread ------------------------------------------------------

/// A take as the [`Recorder`] last saw it.
#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    /// Its files are being made.
    Arming,
    /// It couldn't be armed (the track doesn't arm).
    Failed(String),
    Take(TakeView),
}

enum Cmd {
    Arm {
        no: TakeNo,
        dir: PathBuf,
        name: String,
        id: RecId,
        fmt: RecFormat,
        meta: Meta,
        counts: Arc<Counts>,
    },
    /// Frames, and the bytes the caller counted for them.
    Push(TakeNo, Vec<f32>, usize),
    Full(TakeNo),
    Stop(TakeNo),
    Disarm(TakeNo),
    Forget(TakeNo),
}

/// Capture memory for one take, counted across the two threads: bytes
/// sent but not yet taken, and bytes in the take's capture memory. Their
/// sum never reads lower than the truth.
#[derive(Debug, Default)]
struct Counts {
    queued: AtomicUsize,
    memory: AtomicUsize,
}

#[derive(Debug, Default)]
struct Shown {
    status: HashMap<TakeNo, Status>,
    paths: HashMap<TakeNo, PathBuf>,
    reserve_units: usize,
    interval: f64,
    worst_commit: f64,
    stats: WriterStats,
}

struct Sent {
    channels: usize,
    align: usize,
    full: bool,
    counts: Arc<Counts>,
}

/// A [`Writer`] on a thread of its own. Calls never wait for the stick:
/// arming, stopping and the rest happen on the writer's thread, and what
/// it last saw is read back with [`Recorder::status`].
pub struct Recorder {
    tx: Option<Sender<Cmd>>,
    shown: Arc<Mutex<Shown>>,
    sent: HashMap<TakeNo, Sent>,
    next: u32,
    limit: usize,
    thread: Option<JoinHandle<()>>,
}

impl Recorder {
    pub fn start(
        fs: Box<dyn Fs + Send>,
        clock: Arc<dyn Clock>,
        gate: Arc<Gate>,
        cfg: WriterConfig,
    ) -> io::Result<Recorder> {
        let (tx, rx) = mpsc::channel();
        let shown = Arc::new(Mutex::new(Shown {
            interval: cfg.interval_s,
            worst_commit: cfg.commit_s,
            ..Shown::default()
        }));
        let w = Writer::new(fs, clock, gate, cfg);
        let s = Arc::clone(&shown);
        let thread = std::thread::Builder::new()
            .name("mpfree-rec".into())
            .spawn(move || run(w, &rx, &s))?;
        Ok(Recorder {
            tx: Some(tx),
            shown,
            sent: HashMap::new(),
            next: 1,
            limit: cfg.limit,
            thread: Some(thread),
        })
    }

    fn send(&self, c: Cmd) {
        if let Some(tx) = &self.tx {
            // The thread only ends when this side does.
            let _ = tx.send(c);
        }
    }

    fn shown(&self) -> std::sync::MutexGuard<'_, Shown> {
        self.shown.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Arm a take; its status is [`Status::Arming`] until the writer has
    /// made its files.
    pub fn arm(&mut self, dir: &Path, name: &str, id: RecId, fmt: RecFormat, meta: Meta) -> TakeNo {
        let no = TakeNo(self.next);
        self.next += 1;
        let counts = Arc::new(Counts::default());
        self.sent.insert(
            no,
            Sent {
                channels: usize::from(fmt.channels.max(1)),
                align: fmt.block_align(),
                full: false,
                counts: Arc::clone(&counts),
            },
        );
        self.shown().status.insert(no, Status::Arming);
        self.send(Cmd::Arm {
            no,
            dir: dir.to_path_buf(),
            name: name.to_string(),
            id,
            fmt,
            meta,
            counts,
        });
        no
    }

    /// Hand captured frames (interleaved) to a take; returns how many
    /// were taken. Once its capture memory (sent but not yet written and
    /// committed) would pass the limit, the take stops taking frames:
    /// recording stops there, and the take is finished.
    pub fn push(&mut self, no: TakeNo, frames: &[f32]) -> usize {
        let Some(s) = self.sent.get_mut(&no) else {
            return 0;
        };
        if s.full {
            return 0;
        }
        let have =
            s.counts.queued.load(Ordering::Acquire) + s.counts.memory.load(Ordering::Acquire);
        let room = self.limit.saturating_sub(have) / s.align;
        let n = (frames.len() / s.channels).min(room);
        let full = n < frames.len() / s.channels;
        if n > 0 {
            let bytes = n * s.align;
            s.counts.queued.fetch_add(bytes, Ordering::AcqRel);
            let v = frames[..n * s.channels].to_vec();
            self.send(Cmd::Push(no, v, bytes));
        }
        if full {
            if let Some(s) = self.sent.get_mut(&no) {
                s.full = true;
            }
            self.send(Cmd::Full(no));
        }
        n
    }

    /// The musician stopped: the take is finished on the writer's thread.
    pub fn stop(&mut self, no: TakeNo) {
        if let Some(s) = self.sent.get_mut(&no) {
            s.full = true;
        }
        self.send(Cmd::Stop(no));
    }

    /// Give up an armed take that never recorded.
    pub fn disarm(&mut self, no: TakeNo) {
        self.sent.remove(&no);
        self.shown().status.remove(&no);
        self.send(Cmd::Disarm(no));
    }

    /// Let go of a saved take.
    pub fn forget(&mut self, no: TakeNo) {
        self.sent.remove(&no);
        self.shown().status.remove(&no);
        self.send(Cmd::Forget(no));
    }

    /// What the writer last saw of a take.
    pub fn status(&self, no: TakeNo) -> Option<Status> {
        self.shown().status.get(&no).cloned()
    }

    /// Where a take's finished file is.
    pub fn final_path(&self, no: TakeNo) -> Option<PathBuf> {
        self.shown().paths.get(&no).cloned()
    }

    /// The capture reserve in cache units, for
    /// [`crate::coord::Coordinator::set_reserve`].
    pub fn reserve_units(&self) -> usize {
        self.shown().reserve_units
    }

    pub fn interval(&self) -> f64 {
        self.shown().interval
    }

    pub fn worst_commit(&self) -> f64 {
        self.shown().worst_commit
    }

    pub fn stats(&self) -> WriterStats {
        self.shown().stats
    }

    /// End the writer's thread once its current step is done, and wait
    /// for it. Takes not finished are left as recordings, finished at the
    /// next start by recovery.
    pub fn close(mut self) {
        self.tx = None;
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        // The thread ends by itself once it sees the channel closed.
        self.tx = None;
    }
}

fn run(mut w: Writer, rx: &Receiver<Cmd>, shown: &Mutex<Shown>) {
    let mut counts: HashMap<TakeNo, Arc<Counts>> = HashMap::new();
    let mut failed: HashMap<TakeNo, String> = HashMap::new();
    loop {
        let wait = w.tick();
        for (no, c) in &counts {
            c.memory.store(w.memory(*no), Ordering::Release);
        }
        publish(&w, &failed, shown);
        let first = match rx.recv_timeout(Duration::from_secs_f64(wait.clamp(0.001, 1.0))) {
            Ok(c) => c,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => return,
        };
        let mut next = Some(first);
        while let Some(c) = next {
            apply(&mut w, c, &mut counts, &mut failed);
            next = rx.try_recv().ok();
        }
    }
}

fn apply(
    w: &mut Writer,
    c: Cmd,
    counts: &mut HashMap<TakeNo, Arc<Counts>>,
    failed: &mut HashMap<TakeNo, String>,
) {
    match c {
        Cmd::Arm {
            no,
            dir,
            name,
            id,
            fmt,
            meta,
            counts: k,
        } => match w.arm(no, &dir, &name, id, fmt, meta) {
            Ok(()) => {
                counts.insert(no, k);
            }
            Err(e) => {
                failed.insert(no, e.to_string());
            }
        },
        Cmd::Push(no, v, bytes) => {
            w.push(no, &v);
            if let Some(k) = counts.get(&no) {
                // Memory first, then the queue: the sum never dips.
                k.memory.store(w.memory(no), Ordering::Release);
                k.queued.fetch_sub(bytes, Ordering::AcqRel);
            }
        }
        Cmd::Full(no) => w.refuse_more(no),
        Cmd::Stop(no) => w.stop(no),
        Cmd::Disarm(no) => {
            w.disarm(no);
            counts.remove(&no);
            failed.remove(&no);
        }
        Cmd::Forget(no) => {
            w.forget(no);
            counts.remove(&no);
            failed.remove(&no);
        }
    }
}

fn publish(w: &Writer, failed: &HashMap<TakeNo, String>, shown: &Mutex<Shown>) {
    let mut s = shown.lock().unwrap_or_else(PoisonError::into_inner);
    let takes = w.takes();
    s.status
        .retain(|no, st| *st == Status::Arming || takes.contains(no) || failed.contains_key(no));
    for no in takes {
        if let Some(v) = w.view(no) {
            s.status.insert(no, Status::Take(v));
        }
        if let Some(p) = w.final_path(no) {
            s.paths.insert(no, p);
        }
    }
    for (no, why) in failed {
        s.status.insert(*no, Status::Failed(why.clone()));
    }
    let Shown { status, paths, .. } = &mut *s;
    paths.retain(|no, _| status.contains_key(no));
    s.reserve_units = w.reserve_units();
    s.interval = w.interval();
    s.worst_commit = w.worst_commit();
    s.stats = w.stats();
}
