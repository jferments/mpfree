//! One scheduler for the stretches MPFree makes ahead (the audio tracks
//! rethink, phase 5b).
//!
//! Stretched copies are cheap to keep and slow to make, so their clients
//! make the ones they will need before they are needed: the audio tracks'
//! tempo copies, and in time the sampler's Warps and the looper's tempo
//! copies. [`Sched`] runs them all on one worker, so they never compete
//! for the cores, and in an order that can change while they run:
//!
//! - **Steps.** A job is [`Work`] done a step at a time: about
//!   [`Config::step`] of work (10 ms), never more than
//!   [`Config::max_frames`] frames. After every step the most urgent job
//!   waiting goes next, so a long stretch never holds up a short one that
//!   is needed now by more than one step.
//! - **Four priorities** ([`Priority`]), then the nearest deadline, then
//!   the first asked.
//! - **Keys.** Each job has a [`Key`]: what it makes, for whom. A newer
//!   job with the key of one still waiting, or part done, takes its place,
//!   and one in its step stops at the step's end; either way the old one
//!   is told ([`Work::cancelled`]) and never finishes. [`Sched::cancel`]
//!   and [`Sched::retain`] stop jobs the same way.
//! - **Memory.** A job says what it will hold ([`Job::needs`]) and starts
//!   only with a promise of that much ([`Config::promise`]), held until it
//!   ends; otherwise it is refused, counted, and told why
//!   ([`Work::refused`]), and never starts.
//! - **Where it runs.** The worker starts with the first job, and runs
//!   [`Config::on_start`] first on its own thread: the app puts it where
//!   the sampler's sound lane runs, off the sound's cores at idle priority.
//!   If it can't start, the jobs wait, the failure is counted
//!   ([`Counts::spawn_failed`]) and the next job tries again.
//! - **One a process.** [`Sched::shared`] is the process's scheduler, made
//!   on first use with the config the program gave [`configure`] at its
//!   start. A test makes one of its own with [`Sched::new`], or one with
//!   no worker that it steps itself ([`Sched::manual`]).
//!
//! A job's result goes back however its client likes (usually a channel
//! its [`Work`] holds), and should carry what it was made for, so a client
//! can throw away one that is out of date.

use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

/// How urgent a job is. The worker always takes the most urgent first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Priority {
    /// Something is waiting for it now: a launch waiting for its copy.
    /// Give it a deadline ([`Job::deadline`]): the nearest goes first.
    Urgent,
    /// What something playing needs next: the next tempo point, a ramp,
    /// the copy after a tempo change nobody planned.
    Protect,
    /// What a musician has set up to come: the next scene's row, the
    /// cells cued, the next song in a set.
    Prepared,
    /// Everything else that may be wanted: every other copy at the tempo
    /// now.
    Speculative,
}

/// What a job makes, and for whom: two jobs with one key are the same
/// work, and the newer replaces the older.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Key {
    /// The client and the kind of work, such as "tracks copy".
    pub kind: &'static str,
    /// What it is for, in the client's terms (usually a hash of all that
    /// made it).
    pub id: u128,
}

impl Key {
    pub fn new(kind: &'static str, id: u128) -> Key {
        Key { kind, id }
    }
}

/// Work done a step at a time on the scheduler's worker. Exactly one of
/// [`Work::finish`], [`Work::cancelled`] and [`Work::refused`] is called,
/// once, at its end.
pub trait Work: Send {
    /// Does about `frames` frames of work (the frames of audio a stretch
    /// makes); true once it has finished.
    fn step(&mut self, frames: usize) -> bool;
    /// How far it has got, 0 to 1.
    fn progress(&self) -> f32 {
        0.0
    }
    /// It has finished: hand its result to its client.
    fn finish(self: Box<Self>);
    /// It stopped before finishing: replaced by a newer job with its key,
    /// cancelled, or the scheduler went.
    fn cancelled(self: Box<Self>) {}
    /// It never started: there was no memory for it, as `why` says in a
    /// musician's words.
    fn refused(self: Box<Self>, _why: String) {}
}

/// One job for the scheduler.
pub struct Job {
    key: Key,
    priority: Priority,
    deadline: Option<Instant>,
    need: u64,
    work: Box<dyn Work>,
}

impl Job {
    /// `work` with `key` at `priority`, with no deadline, holding no memory
    /// to speak of.
    pub fn new(key: Key, priority: Priority, work: impl Work + 'static) -> Job {
        Job {
            key,
            priority,
            deadline: None,
            need: 0,
            work: Box::new(work),
        }
    }

    /// It is wanted by `at`: among jobs of its priority, the nearest
    /// deadline goes first (one without goes after those with one).
    pub fn deadline(mut self, at: Instant) -> Job {
        self.deadline = Some(at);
        self
    }

    /// It holds about `bytes` while it runs and what it makes after, so it
    /// starts only with a promise of that much ([`Config::promise`]).
    pub fn needs(mut self, bytes: u64) -> Job {
        self.need = bytes;
        self
    }

    pub fn key(&self) -> Key {
        self.key
    }
}

/// A promise of memory a started job holds ([`Config::promise`]): whatever
/// the promise's maker returns, kept until the job ends.
pub struct Hold(Option<Box<dyn Send>>);

impl Hold {
    /// No promise: memory isn't counted.
    pub fn none() -> Hold {
        Hold(None)
    }

    /// Holds `promise` (a memory ticket) until dropped.
    pub fn new(promise: impl Send + 'static) -> Hold {
        Hold(Some(Box::new(promise)))
    }
}

impl std::fmt::Debug for Hold {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.0.is_some() {
            "Hold"
        } else {
            "Hold(none)"
        })
    }
}

/// How the scheduler runs.
#[derive(Clone, Copy)]
pub struct Config {
    /// Run first on the worker's own thread. The app puts it where the
    /// sampler's sound lane runs: off the sound's cores, at idle priority.
    pub on_start: fn(),
    /// Promises a job's `need` bytes until it ends, if they fit beside the
    /// floor and every other promise standing; Err says why, in a
    /// musician's words. The promise ends when its [`Hold`] drops.
    pub promise: fn(u64) -> Result<Hold, String>,
    /// About how long a step lasts. The frames a step asks for are set
    /// from how long the last ones took.
    pub step: Duration,
    /// The most frames a step asks for, however quick the steps are.
    pub max_frames: usize,
}

/// The frames the first step asks for, before any has been timed.
const FIRST_FRAMES: usize = 2048;
/// The fewest frames a step asks for, however slow the steps are.
pub const MIN_FRAMES: usize = 256;
/// The most frames a step asks for by default: a job stops within this
/// many frames of being told to (the sampler's Warp guarantee).
pub const MAX_FRAMES: usize = 8192;

impl Default for Config {
    /// Nothing done at the worker's start, memory never short, steps of
    /// about 10 ms and at most [`MAX_FRAMES`] frames.
    fn default() -> Config {
        Config {
            on_start: || {},
            promise: |_| Ok(Hold::none()),
            step: Duration::from_millis(10),
            max_frames: MAX_FRAMES,
        }
    }
}

/// What the scheduler has done since it started.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    /// Jobs that ran to their end.
    pub done: u64,
    /// Jobs replaced by a newer one with their key.
    pub replaced: u64,
    /// Jobs cancelled ([`Sched::cancel`], [`Sched::retain`]), or still
    /// there when the scheduler went.
    pub cancelled: u64,
    /// Jobs refused for want of memory.
    pub refused: u64,
    /// Times the worker couldn't start. Its jobs wait for the next try.
    pub spawn_failed: u64,
    /// Steps run, of every job.
    pub steps: u64,
}

/// A job waiting its turn, or part done.
struct Entry {
    job: Job,
    /// Its place in the order it was asked in.
    seq: u64,
    /// Its promise, once it has started.
    hold: Option<Hold>,
    progress: f32,
}

impl Entry {
    fn rank(&self) -> (Priority, bool, Option<Instant>, u64) {
        let j = &self.job;
        (j.priority, j.deadline.is_none(), j.deadline, self.seq)
    }
}

/// The job in its step, out of the list while it runs.
struct Busy {
    key: Key,
    priority: Priority,
    /// Told to stop: `Some(true)` replaced, `Some(false)` cancelled.
    stop: Option<bool>,
}

#[derive(Default)]
struct State {
    jobs: Vec<Entry>,
    busy: Option<Busy>,
    seq: u64,
    /// Frames the next step asks for.
    frames: usize,
    live: bool,
    stop: bool,
    held: bool,
    counts: Counts,
}

struct Shared {
    state: Mutex<State>,
    cv: Condvar,
    config: Config,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        // No work runs while the lock is held, so nothing can poison it
        // halfway through a change: carry on with what is there.
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// The handles' common part: when the last handle goes, the worker stops
/// after its step, and every job left is told it was cancelled.
struct Owner {
    shared: Arc<Shared>,
    manual: bool,
}

impl Drop for Owner {
    fn drop(&mut self) {
        let left = {
            let mut s = self.shared.lock();
            s.stop = true;
            let mut n = 0;
            if let Some(b) = s.busy.as_mut().filter(|b| b.stop.is_none()) {
                b.stop = Some(false);
                n += 1;
            }
            let left = std::mem::take(&mut s.jobs);
            s.counts.cancelled += n + left.len() as u64;
            left
        };
        self.shared.cv.notify_all();
        for e in left {
            e.job.work.cancelled();
        }
    }
}

/// The scheduler. Cloning gives another handle to the same one.
#[derive(Clone)]
pub struct Sched(Arc<Owner>);

/// The process's scheduler, once made ([`Sched::shared`]).
static SHARED: OnceLock<Sched> = OnceLock::new();
/// Its config until then ([`configure`]).
static CONFIG: Mutex<Option<Config>> = Mutex::new(None);

/// Gives the process's scheduler ([`Sched::shared`]) its config, before
/// its first use: the program does this at its start. False, and the
/// config unchanged, if it is made already.
pub fn configure(config: Config) -> bool {
    if SHARED.get().is_some() {
        return false;
    }
    *CONFIG.lock().unwrap_or_else(|e| e.into_inner()) = Some(config);
    SHARED.get().is_none()
}

impl Sched {
    /// The process's one scheduler, made the first time it is asked for,
    /// with the config given to [`configure`] (or the default).
    pub fn shared() -> Sched {
        SHARED
            .get_or_init(|| {
                let config = CONFIG
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .take()
                    .unwrap_or_default();
                Sched::new(config)
            })
            .clone()
    }

    /// A scheduler whose worker starts with its first job.
    pub fn new(config: Config) -> Sched {
        Sched::make(config, false)
    }

    /// A scheduler with no worker: its jobs run only when the caller steps
    /// them ([`Sched::step`], [`Sched::run_all`]). For tests.
    pub fn manual(config: Config) -> Sched {
        Sched::make(config, true)
    }

    fn make(config: Config, manual: bool) -> Sched {
        Sched(Arc::new(Owner {
            shared: Arc::new(Shared {
                state: Mutex::new(State {
                    frames: FIRST_FRAMES.clamp(MIN_FRAMES, config.max_frames.max(MIN_FRAMES)),
                    ..State::default()
                }),
                cv: Condvar::new(),
                config,
            }),
            manual,
        }))
    }

    fn inner(&self) -> &Arc<Shared> {
        &self.0.shared
    }

    /// Whether two handles are to the same scheduler.
    pub fn same(&self, other: &Sched) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    /// Puts `job` in the order. A job with its key that is waiting or part
    /// done is dropped for it, and one in its step stops at the step's
    /// end, each told it was cancelled.
    pub fn submit(&self, job: Job) {
        let gone = {
            let mut s = self.inner().lock();
            if s.stop {
                drop(s);
                job.work.cancelled();
                return;
            }
            let gone = take_where(&mut s, |k| k == job.key, true);
            let seq = s.seq;
            s.seq += 1;
            s.jobs.push(Entry {
                job,
                seq,
                hold: None,
                progress: 0.0,
            });
            gone
        };
        tell_cancelled(gone);
        if !self.0.manual {
            self.start_worker();
        }
        self.inner().cv.notify_all();
    }

    /// Drops the job with `key` if it waits, and stops it if it runs.
    pub fn cancel(&self, key: Key) {
        self.retain(|k| k != key);
    }

    /// Keeps only the jobs whose key `keep` says yes to: the rest are
    /// dropped if waiting and stopped if running, each told so.
    pub fn retain(&self, keep: impl Fn(Key) -> bool) {
        let gone = {
            let mut s = self.inner().lock();
            take_where(&mut s, |k| !keep(k), false)
        };
        tell_cancelled(gone);
    }

    /// Gives the job with `key` a new priority and deadline, if it is
    /// there (waiting, part done or in its step). True if it is, so the
    /// caller needn't submit it again.
    pub fn reorder(&self, key: Key, priority: Priority, deadline: Option<Instant>) -> bool {
        let mut s = self.inner().lock();
        if let Some(e) = s.jobs.iter_mut().find(|e| e.job.key == key) {
            e.job.priority = priority;
            e.job.deadline = deadline;
            return true;
        }
        match s.busy.as_mut() {
            Some(b) if b.key == key && b.stop.is_none() => {
                b.priority = priority;
                true
            }
            _ => false,
        }
    }

    /// Whether a job with `key` is waiting, part done or in its step (and
    /// not told to stop).
    pub fn has(&self, key: Key) -> bool {
        let s = self.inner().lock();
        s.jobs.iter().any(|e| e.job.key == key)
            || s.busy
                .as_ref()
                .is_some_and(|b| b.key == key && b.stop.is_none())
    }

    /// How far the job with `key` has got, 0 to 1, if it is there.
    pub fn progress(&self, key: Key) -> Option<f32> {
        let s = self.inner().lock();
        s.jobs.iter().find(|e| e.job.key == key).map(|e| e.progress)
    }

    /// Jobs waiting or part done, and the one in its step.
    pub fn len(&self) -> usize {
        let s = self.inner().lock();
        s.jobs.len() + usize::from(s.busy.is_some())
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// What the scheduler has done so far.
    pub fn counts(&self) -> Counts {
        self.inner().lock().counts
    }

    /// Holds the worker before its next step (`true`), or lets it go on:
    /// for tests that look at the order before anything runs.
    pub fn hold(&self, held: bool) {
        self.inner().lock().held = held;
        self.inner().cv.notify_all();
    }

    /// Runs one step of the most urgent job, on the caller: what a
    /// [`Sched::manual`] scheduler does instead of a worker. The key of the
    /// job stepped, or None if there was none.
    pub fn step(&self) -> Option<Key> {
        let shared = self.inner();
        let picked = {
            let mut s = shared.lock();
            if s.busy.is_some() {
                return None;
            }
            pick(shared, &mut s)
        };
        match picked {
            Picked::None => None,
            Picked::Refused(e, why) => {
                let key = e.job.key;
                e.job.work.refused(why);
                Some(key)
            }
            Picked::Run(e) => {
                let key = e.job.key;
                run_step(shared, e);
                Some(key)
            }
        }
    }

    /// Steps until no job is left (a [`Sched::manual`] scheduler's). The
    /// steps run.
    pub fn run_all(&self) -> usize {
        let mut n = 0;
        while self.step().is_some() {
            n += 1;
        }
        n
    }

    /// Starts the worker if it isn't running. One that can't start is
    /// counted, and the jobs wait for the next try.
    fn start_worker(&self) {
        let shared = self.inner();
        {
            let mut s = shared.lock();
            if s.stop || s.live {
                return;
            }
            s.live = true;
        }
        let worker = shared.clone();
        let spawned = std::thread::Builder::new()
            .name("mpfree-stretch".into())
            .spawn(move || {
                (worker.config.on_start)();
                work(&worker);
            });
        if spawned.is_err() {
            let mut s = shared.lock();
            s.live = false;
            s.counts.spawn_failed += 1;
        }
    }
}

/// Takes out every job whose key `gone` says yes to, counted as replaced
/// (`replaced`) or cancelled, and tells one in its step to stop. The ones
/// taken out, to be told outside the lock.
fn take_where(s: &mut State, gone: impl Fn(Key) -> bool, replaced: bool) -> Vec<Entry> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < s.jobs.len() {
        if gone(s.jobs[i].job.key) {
            out.push(s.jobs.swap_remove(i));
        } else {
            i += 1;
        }
    }
    let mut n = out.len() as u64;
    if let Some(b) = s.busy.as_mut() {
        if b.stop.is_none() && gone(b.key) {
            b.stop = Some(replaced);
            n += 1;
        }
    }
    if replaced {
        s.counts.replaced += n;
    } else {
        s.counts.cancelled += n;
    }
    out
}

fn tell_cancelled(gone: Vec<Entry>) {
    for e in gone {
        e.job.work.cancelled();
    }
}

enum Picked {
    None,
    Run(Entry),
    Refused(Entry, String),
}

/// The most urgent job, out of the list and marked in its step; a job
/// starting is promised its memory first, or refused.
fn pick(shared: &Shared, s: &mut State) -> Picked {
    let Some(i) = (0..s.jobs.len()).min_by_key(|&i| s.jobs[i].rank()) else {
        return Picked::None;
    };
    let mut e = s.jobs.swap_remove(i);
    if e.hold.is_none() {
        match (shared.config.promise)(e.job.need) {
            Ok(h) => e.hold = Some(h),
            Err(why) => {
                s.counts.refused += 1;
                return Picked::Refused(e, why);
            }
        }
    }
    s.busy = Some(Busy {
        key: e.job.key,
        priority: e.job.priority,
        stop: None,
    });
    Picked::Run(e)
}

/// One step of `e`, timed, then back in the list, finished or stopped.
fn run_step(shared: &Shared, mut e: Entry) {
    let frames = shared.lock().frames;
    let t = Instant::now();
    let finished = e.job.work.step(frames);
    let took = t.elapsed();
    e.progress = if finished { 1.0 } else { e.job.work.progress() };
    let finish = {
        let mut s = shared.lock();
        s.counts.steps += 1;
        s.frames = next_frames(frames, took, &shared.config);
        match s.busy.take() {
            Some(b) if b.stop.is_some() => false,
            _ if finished => {
                s.counts.done += 1;
                true
            }
            b => {
                if let Some(b) = b {
                    e.job.priority = b.priority;
                }
                s.jobs.push(e);
                return;
            }
        }
    };
    shared.cv.notify_all();
    if finish {
        e.job.work.finish();
    } else {
        e.job.work.cancelled();
    }
}

/// The frames the next step asks for: as many as the last step's rate
/// does in [`Config::step`], within [`MIN_FRAMES`] and
/// [`Config::max_frames`], moving at most twice either way a step.
fn next_frames(frames: usize, took: Duration, config: &Config) -> usize {
    let max = config.max_frames.max(MIN_FRAMES);
    let took = took.as_secs_f64().max(1e-6);
    let want = frames as f64 * config.step.as_secs_f64() / took;
    let want = want.clamp(frames as f64 / 2.0, frames as f64 * 2.0);
    (want.round() as usize).clamp(MIN_FRAMES, max)
}

/// The worker: a step of the most urgent job, again and again, until the
/// scheduler goes.
fn work(shared: &Shared) {
    loop {
        let picked = {
            let mut s = shared.lock();
            loop {
                if s.stop {
                    s.live = false;
                    return;
                }
                if !s.held && !s.jobs.is_empty() {
                    break pick(shared, &mut s);
                }
                s = shared.cv.wait(s).unwrap_or_else(|e| e.into_inner());
            }
        };
        match picked {
            Picked::None => {}
            Picked::Refused(e, why) => e.job.work.refused(why),
            Picked::Run(e) => run_step(shared, e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::{channel, Receiver, Sender};

    /// Work `total` frames long that says what became of it.
    struct Fake {
        name: &'static str,
        total: usize,
        done: usize,
        tx: Sender<(&'static str, &'static str)>,
    }

    impl Work for Fake {
        fn step(&mut self, frames: usize) -> bool {
            self.done = (self.done + frames).min(self.total);
            let _ = self.tx.send((self.name, "step"));
            self.done == self.total
        }
        fn progress(&self) -> f32 {
            self.done as f32 / self.total as f32
        }
        fn finish(self: Box<Self>) {
            let _ = self.tx.send((self.name, "finished"));
        }
        fn cancelled(self: Box<Self>) {
            let _ = self.tx.send((self.name, "cancelled"));
        }
        fn refused(self: Box<Self>, _why: String) {
            let _ = self.tx.send((self.name, "refused"));
        }
    }

    fn fake(name: &'static str, total: usize, tx: &Sender<(&'static str, &'static str)>) -> Fake {
        Fake {
            name,
            total,
            done: 0,
            tx: tx.clone(),
        }
    }

    fn key(id: u128) -> Key {
        Key::new("test", id)
    }

    fn drain(rx: &Receiver<(&'static str, &'static str)>) -> Vec<(&'static str, &'static str)> {
        rx.try_iter().collect()
    }

    fn manual() -> Sched {
        Sched::manual(Config {
            max_frames: 1000,
            ..Config::default()
        })
    }

    #[test]
    fn a_short_urgent_job_waits_one_step_behind_a_long_one() {
        let s = manual();
        let (tx, rx) = channel();
        // A 64-bar backing track: many steps of work.
        s.submit(Job::new(
            key(1),
            Priority::Speculative,
            fake("long", 1_000_000, &tx),
        ));
        s.step();
        assert_eq!(drain(&rx), [("long", "step")]);
        // A launch waits for a 2-bar loop's copy.
        s.submit(
            Job::new(key(2), Priority::Urgent, fake("short", 2500, &tx)).deadline(Instant::now()),
        );
        let mut seen = Vec::new();
        while !seen.contains(&("short", "finished")) {
            s.step();
            seen.extend(drain(&rx));
        }
        // Only its own steps came before it finished: the long job waited.
        assert!(seen.iter().all(|(n, _)| *n == "short"), "{seen:?}");
        assert_eq!(seen.len(), 4, "three steps and its end: {seen:?}");
        assert!(s.progress(key(1)).is_some_and(|p| p > 0.0 && p < 1.0));
        s.run_all();
        assert!(drain(&rx).ends_with(&[("long", "finished")]));
        assert_eq!(s.counts().done, 2);
    }

    #[test]
    fn the_order_is_priority_then_deadline_then_first_asked() {
        let s = manual();
        let (tx, rx) = channel();
        let now = Instant::now();
        s.submit(Job::new(
            key(1),
            Priority::Speculative,
            fake("spec", 10, &tx),
        ));
        s.submit(Job::new(
            key(2),
            Priority::Prepared,
            fake("prep a", 10, &tx),
        ));
        s.submit(Job::new(
            key(3),
            Priority::Prepared,
            fake("prep b", 10, &tx),
        ));
        s.submit(Job::new(
            key(4),
            Priority::Urgent,
            fake("urgent none", 10, &tx),
        ));
        s.submit(
            Job::new(key(5), Priority::Urgent, fake("urgent late", 10, &tx))
                .deadline(now + Duration::from_secs(2)),
        );
        s.submit(
            Job::new(key(6), Priority::Urgent, fake("urgent soon", 10, &tx))
                .deadline(now + Duration::from_secs(1)),
        );
        s.submit(Job::new(
            key(7),
            Priority::Protect,
            fake("protect", 10, &tx),
        ));
        s.run_all();
        let order: Vec<&str> = drain(&rx)
            .into_iter()
            .filter(|(_, what)| *what == "finished")
            .map(|(n, _)| n)
            .collect();
        assert_eq!(
            order,
            [
                "urgent soon",
                "urgent late",
                "urgent none",
                "protect",
                "prep a",
                "prep b",
                "spec"
            ]
        );
    }

    #[test]
    fn a_newer_job_with_a_key_takes_the_old_ones_place_and_the_old_never_finishes() {
        let s = manual();
        let (tx, rx) = channel();
        s.submit(Job::new(key(1), Priority::Prepared, fake("old", 5000, &tx)));
        s.step();
        s.submit(Job::new(key(1), Priority::Prepared, fake("new", 5000, &tx)));
        s.run_all();
        let seen = drain(&rx);
        assert!(seen.contains(&("old", "cancelled")));
        assert!(!seen.contains(&("old", "finished")));
        assert!(seen.ends_with(&[("new", "finished")]));
        let c = s.counts();
        assert_eq!((c.done, c.replaced, c.cancelled), (1, 1, 0));
    }

    #[test]
    fn a_job_told_to_stop_in_its_step_stops_at_the_steps_end() {
        // On a worker: the step in progress is held until the test lets it
        // go, and the job is cancelled meanwhile.
        struct Gated {
            go: Receiver<()>,
            tx: Sender<&'static str>,
        }
        impl Work for Gated {
            fn step(&mut self, _frames: usize) -> bool {
                let _ = self.tx.send("step");
                let _ = self.go.recv();
                false
            }
            fn finish(self: Box<Self>) {
                let _ = self.tx.send("finished");
            }
            fn cancelled(self: Box<Self>) {
                let _ = self.tx.send("cancelled");
            }
        }
        let s = Sched::new(Config::default());
        let (go_tx, go) = channel();
        let (tx, rx) = channel();
        s.submit(Job::new(key(9), Priority::Speculative, Gated { go, tx }));
        assert_eq!(rx.recv_timeout(Duration::from_secs(10)), Ok("step"));
        assert!(s.has(key(9)));
        s.cancel(key(9));
        assert!(!s.has(key(9)));
        go_tx.send(()).unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(10)), Ok("cancelled"));
        // It never steps again.
        let _ = go_tx.send(());
        assert!(rx.recv_timeout(Duration::from_millis(100)).is_err());
        assert_eq!(s.counts().cancelled, 1);
    }

    #[test]
    fn a_job_with_no_memory_for_it_is_refused_with_why_and_never_steps() {
        let s = Sched::manual(Config {
            promise: |need| {
                if need > 100 {
                    Err("not enough memory".into())
                } else {
                    Ok(Hold::none())
                }
            },
            ..Config::default()
        });
        struct Why(Sender<String>);
        impl Work for Why {
            fn step(&mut self, _: usize) -> bool {
                let _ = self.0.send("stepped".into());
                true
            }
            fn finish(self: Box<Self>) {}
            fn refused(self: Box<Self>, why: String) {
                let _ = self.0.send(why);
            }
        }
        let (tx, rx) = channel();
        s.submit(Job::new(key(1), Priority::Urgent, Why(tx.clone())).needs(101));
        s.submit(Job::new(key(2), Priority::Urgent, Why(tx)).needs(100));
        s.run_all();
        let said: Vec<String> = rx.try_iter().collect();
        assert_eq!(said, ["not enough memory", "stepped"]);
        let c = s.counts();
        assert_eq!((c.refused, c.done), (1, 1));
    }

    #[test]
    fn retain_stops_what_it_doesnt_keep_and_a_reorder_moves_a_job_up() {
        let s = manual();
        let (tx, rx) = channel();
        for i in 0..4 {
            s.submit(Job::new(
                key(i),
                Priority::Speculative,
                fake(["a", "b", "c", "d"][i as usize], 10, &tx),
            ));
        }
        s.retain(|k| k.id % 2 == 1);
        assert!(s.reorder(key(3), Priority::Urgent, None));
        assert!(!s.reorder(key(0), Priority::Urgent, None));
        s.run_all();
        let seen = drain(&rx);
        let finished: Vec<&str> = seen
            .iter()
            .filter(|(_, w)| *w == "finished")
            .map(|(n, _)| *n)
            .collect();
        assert_eq!(finished, ["d", "b"]);
        assert_eq!(seen.iter().filter(|(_, w)| *w == "cancelled").count(), 2);
        assert_eq!(s.counts().cancelled, 2);
    }

    #[test]
    fn steps_follow_the_time_they_take_within_their_bounds() {
        let c = Config::default();
        // Quick steps grow, at most twice a step, up to the most.
        assert_eq!(next_frames(2048, Duration::from_millis(1), &c), 4096);
        assert_eq!(next_frames(8192, Duration::from_millis(1), &c), MAX_FRAMES);
        // Slow ones shrink, at most by half, down to the fewest.
        assert_eq!(next_frames(2048, Duration::from_millis(40), &c), 1024);
        assert_eq!(next_frames(300, Duration::from_millis(40), &c), MIN_FRAMES);
        // One on time stays.
        assert_eq!(next_frames(2048, Duration::from_millis(10), &c), 2048);
    }

    #[test]
    fn jobs_left_when_the_scheduler_goes_are_told_they_were_cancelled() {
        let (tx, rx) = channel();
        {
            let s = manual();
            s.submit(Job::new(key(1), Priority::Prepared, fake("left", 10, &tx)));
        }
        assert_eq!(drain(&rx), [("left", "cancelled")]);
    }
}
