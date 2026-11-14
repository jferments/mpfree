//! Render partitions: a **lane** runs part of each audio period on another
//! core, and the audio thread joins it before the mix.
//!
//! The MPC One has four cores. The audio thread renders on one; a lane is a
//! second thread, pinned to its own core, that renders another part of the
//! same period at the same time (the other half of the sampler's voices,
//! the synth, a mixer stage, a return). See the "Concurrency audit" tab of
//! the performance plan and `docs/dev/design/concurrency.md`.
//!
//! Each period, the audio thread:
//!
//! 1. [`Lane::post`]s block `n`: the lane's [`Job`] now belongs to the
//!    worker, which runs [`Job::run`]`(n)`;
//! 2. renders its own part;
//! 3. [`Lane::join`]s with a deadline: the job is back, with its output, or
//!    the deadline passed and the call says [`Late`]. Late is the same
//!    contract the plugin sandbox and the offload server keep: the audio
//!    thread never waits past its deadline, uses a fallback for that part
//!    (silence, or the last block), and the job comes back by itself; until
//!    it does, [`Lane::post`] answers [`Busy`].
//!
//! The job itself moves between the two threads, so the job owns its
//! buffers and nothing is shared while it runs: no lock, no copy, no
//! allocation per period. One atomic word says who holds the job. The
//! waiting side spins first and then sleeps on that word (a futex on Linux),
//! and the other side wakes it only if it is asleep, so a busy period costs
//! no system call.
//!
//! [`Lane::new`] never fails: if the thread cannot start, or
//! [`Config::threaded`] is off, the lane runs each job on the calling thread
//! inside [`Lane::post`], with the same calls and the same results. So the
//! audio loop is written once, and a setting compares one core against two.
//!
//! Pinning ([`Config::core`]) and SCHED_FIFO ([`Config::fifo`]) are asked
//! for by the worker itself when it starts; what happened is in
//! [`Lane::setup`]. They change only this process's own thread, and end
//! with it. Neither failing stops the lane. By default the worker then runs
//! where and how it can; with [`Config::strict`] it stops before touching
//! the job and the lane runs jobs on the calling thread instead. That is
//! the safe choice for the audio thread: a worker that could not be pinned
//! keeps the core set of the thread that made it, which on the MPC is the
//! audio thread's own core 3, so it would fight the audio thread for that
//! core while the audio thread waits for it.

#![deny(unsafe_code)]

pub mod os;

use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Work a lane runs once per period. It owns everything it reads and
/// writes; the audio thread reads its output after [`Lane::join`].
pub trait Job: Send + 'static {
    /// Renders block `block` (the period's number).
    fn run(&mut self, block: u64);
}

impl<F: FnMut(u64) + Send + 'static> Job for F {
    fn run(&mut self, block: u64) {
        self(block)
    }
}

/// How a lane runs.
#[derive(Clone, Debug)]
pub struct Config {
    /// The worker thread's name (Linux shows 15 bytes of it).
    pub name: String,
    /// Run jobs on a worker thread. Off: on the calling thread, in `post`.
    pub threaded: bool,
    /// Pin the worker to this core (on the MPC: 2, beside the audio thread
    /// on 3).
    pub core: Option<usize>,
    /// SCHED_FIFO priority for the worker (on the MPC: 69, one below the
    /// audio thread's 70). None keeps what the worker inherits: a Linux
    /// thread starts with the policy of the thread that made it, unless
    /// that thread asked for SCHED_RESET_ON_FORK, as mpfree-app's audio
    /// thread does (then the worker starts at the normal policy). Its cores
    /// are always inherited.
    pub fifo: Option<i32>,
    /// How long the worker spins for the next block after finishing one,
    /// before it sleeps. Zero: it sleeps at once (kind to the core, costs a
    /// wake-up each period). Longer than a period: it never sleeps while
    /// blocks keep coming (the lowest handoff, a busy core).
    pub worker_spin: Duration,
    /// How long [`Lane::join`] spins before it sleeps until the deadline.
    /// The audio thread has nothing else to do while it waits, so the
    /// default spins all the way.
    pub join_spin: Duration,
    /// Run on the worker only with the core and priority asked for: if
    /// [`Config::core`] or [`Config::fifo`] cannot be had, the worker stops
    /// before it touches the job and the lane runs jobs on the calling
    /// thread ([`Setup::threaded`] says why). Off: the worker runs anyway.
    pub strict: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            name: "mpfree-lane".into(),
            threaded: true,
            core: None,
            fifo: None,
            worker_spin: Duration::ZERO,
            join_spin: Duration::MAX,
            strict: false,
        }
    }
}

/// What happened when the lane started.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Setup {
    /// Ok: jobs run on the worker. Err: they run on the calling thread, and why.
    pub threaded: Result<(), String>,
    /// The pin asked for, and whether it took.
    pub pinned: Option<Result<usize, String>>,
    /// The SCHED_FIFO priority asked for, and whether it took.
    pub fifo: Option<Result<i32, String>>,
}

/// The deadline passed before the worker finished the block. The job stays
/// with the worker until it does; use a fallback for this part.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Late;

/// The worker still has the job from a late block, so this block was not
/// posted. Use a fallback for this part.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Busy;

/// Counts since the lane started.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Blocks handed to the job.
    pub posted: u64,
    /// Joins that came back in time.
    pub joined: u64,
    /// Joins that passed their deadline.
    pub late: u64,
    /// Posts refused because the job had not come back yet.
    pub busy: u64,
    /// Allocator calls the job made on the worker (an allocation and its
    /// free are two). Counted only where the counting allocator is
    /// installed, as in mpfree-bench and the tests; else 0.
    pub worker_allocs: u64,
}

/// The job is with the audio thread (or the lane is new).
const IDLE: u32 = 0;
/// Posted: the job is the worker's until it says DONE.
const POSTED: u32 = 1;
/// Done: the job is back with the audio thread.
const DONE: u32 = 2;
/// The lane is closing; the worker exits. Stored only while the job is
/// not posted, so it never overwrites POSTED.
const STOP: u32 = 3;

/// How often a spinning waiter reads the clock (a clock read can be a
/// system call; a spin is a few nanoseconds): when `n & CLOCK_MASK == 0`,
/// every 32 spins.
const CLOCK_MASK: u32 = 31;

struct Shared<J> {
    /// IDLE, POSTED, DONE or STOP; also the futex word both sides sleep on.
    state: AtomicU32,
    block: AtomicU64,
    worker_asleep: AtomicBool,
    joiner_asleep: AtomicBool,
    worker_allocs: AtomicU64,
    worker_spin: Duration,
    /// Touched only by the side that holds it (see `state`).
    job: UnsafeCell<J>,
}

// SAFETY: the job moves between threads (J: Send) and is touched by one
// thread at a time: the worker between seeing POSTED (Acquire) and storing
// DONE (Release); the lane's owner only while the state is IDLE or DONE,
// seen with Acquire. Only the owner stores POSTED and STOP.
#[allow(unsafe_code)]
unsafe impl<J: Send> Sync for Shared<J> {}

enum Runs<J> {
    Worker {
        shared: Arc<Shared<J>>,
        thread: Option<JoinHandle<()>>,
    },
    Here(J),
}

/// One render partition. See the crate docs.
pub struct Lane<J: Job> {
    runs: Runs<J>,
    /// Posted and not joined yet (in time or after a late join).
    out: bool,
    config: Config,
    setup: Setup,
    stats: Stats,
}

impl<J: Job> Lane<J> {
    /// Starts a lane for `job`. Never fails: without a worker thread it runs
    /// jobs on the calling thread ([`Lane::setup`] says why).
    pub fn new(job: J, config: Config) -> Self {
        if !config.threaded {
            return Self::here(job, config, Err("off in the lane's settings".into()));
        }
        let shared = Arc::new(Shared {
            state: AtomicU32::new(IDLE),
            block: AtomicU64::new(0),
            worker_asleep: AtomicBool::new(false),
            joiner_asleep: AtomicBool::new(false),
            worker_allocs: AtomicU64::new(0),
            worker_spin: config.worker_spin,
            job: UnsafeCell::new(job),
        });
        let (tx, rx) = mpsc::channel();
        let (core, fifo, strict) = (config.core, config.fifo, config.strict);
        let theirs = Arc::clone(&shared);
        let spawned = std::thread::Builder::new()
            .name(config.name.clone())
            .spawn(move || {
                let pinned = core.map(|c| os::pin(c).map(|()| c));
                let fifo = fifo.map(|p| os::fifo(p).map(|()| p));
                let missed =
                    strict && (matches!(pinned, Some(Err(_))) || matches!(fifo, Some(Err(_))));
                let _ = tx.send((pinned, fifo));
                // Strict and short of its core or priority: stop without
                // touching the job, so the lane can take it back.
                if !missed {
                    worker(&theirs);
                }
            });
        match spawned {
            Ok(thread) => {
                let (pinned, fifo) = rx
                    .recv()
                    .unwrap_or((None, Some(Err("the worker stopped while starting".into()))));
                if let Some(why) = strict.then(|| missing(&pinned, &fifo)).flatten() {
                    // The worker has returned (or is returning) without
                    // running the job; once it has, its Arc is gone.
                    let _ = thread.join();
                    let job = match Arc::try_unwrap(shared) {
                        Ok(s) => s.job.into_inner(),
                        Err(_) => unreachable!("the worker has ended"),
                    };
                    let mut lane = Self::here(job, config, Err(why));
                    (lane.setup.pinned, lane.setup.fifo) = (pinned, fifo);
                    return lane;
                }
                Self {
                    runs: Runs::Worker {
                        shared,
                        thread: Some(thread),
                    },
                    out: false,
                    config,
                    setup: Setup {
                        threaded: Ok(()),
                        pinned,
                        fifo,
                    },
                    stats: Stats::default(),
                }
            }
            Err(e) => {
                // The closure (and its Arc) is gone with the failed spawn, so
                // the job can be taken back.
                let job = match Arc::try_unwrap(shared) {
                    Ok(s) => s.job.into_inner(),
                    Err(_) => unreachable!("the thread never started"),
                };
                Self::here(job, config, Err(format!("no worker thread: {e}")))
            }
        }
    }

    /// A lane that runs every job on the calling thread.
    pub fn inline(job: J) -> Self {
        Self::here(
            job,
            Config {
                threaded: false,
                ..Config::default()
            },
            Err("an inline lane".into()),
        )
    }

    fn here(job: J, config: Config, why: Result<(), String>) -> Self {
        Self {
            runs: Runs::Here(job),
            out: false,
            config,
            setup: Setup {
                threaded: why,
                pinned: None,
                fifo: None,
            },
            stats: Stats::default(),
        }
    }

    /// What happened when the lane started.
    pub fn setup(&self) -> &Setup {
        &self.setup
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    /// True if jobs run on a worker thread.
    pub fn threaded(&self) -> bool {
        matches!(self.runs, Runs::Worker { .. })
    }

    pub fn stats(&self) -> Stats {
        let mut s = self.stats;
        if let Runs::Worker { shared, .. } = &self.runs {
            s.worker_allocs = shared.worker_allocs.load(Ordering::Relaxed);
        }
        s
    }

    /// Hands block `block` to the job. [`Busy`] if the job has not come
    /// back from a late block yet (nothing is posted; use a fallback).
    pub fn post(&mut self, block: u64) -> Result<(), Busy> {
        match &mut self.runs {
            Runs::Here(job) => {
                job.run(block);
            }
            Runs::Worker { shared, .. } => {
                if self.out && shared.state.load(Ordering::Acquire) != DONE {
                    self.stats.busy += 1;
                    return Err(Busy);
                }
                shared.block.store(block, Ordering::Relaxed);
                // SeqCst with the worker's (asleep = true; read state): either
                // it sees POSTED and does not sleep, or we see it asleep.
                shared.state.store(POSTED, Ordering::SeqCst);
                if shared.worker_asleep.load(Ordering::SeqCst) {
                    os::wake(&shared.state);
                }
            }
        }
        self.out = true;
        self.stats.posted += 1;
        Ok(())
    }

    /// Waits for the posted block until `deadline` and hands back the job
    /// with its output. [`Late`] if the deadline passed first: the job stays
    /// with the worker; [`Lane::post`] refuses until it is back. With
    /// nothing posted, the job is simply handed back.
    pub fn join(&mut self, deadline: Instant) -> Result<&mut J, Late> {
        if self.out {
            if let Runs::Worker { shared, .. } = &self.runs {
                if !wait_done(shared, deadline, self.config.join_spin) {
                    self.stats.late += 1;
                    return Err(Late);
                }
            }
            self.out = false;
            self.stats.joined += 1;
        }
        Ok(self.job_here())
    }

    /// The job, if it is with this thread now (not posted, or back), without
    /// waiting. None while the worker has it.
    pub fn job(&mut self) -> Option<&mut J> {
        if self.out {
            if let Runs::Worker { shared, .. } = &self.runs {
                if shared.state.load(Ordering::Acquire) != DONE {
                    return None;
                }
            }
            self.out = false;
        }
        Some(self.job_here())
    }

    /// The job; the caller has checked that this thread holds it.
    fn job_here(&mut self) -> &mut J {
        match &mut self.runs {
            Runs::Here(job) => job,
            #[allow(unsafe_code)]
            // SAFETY: `out` is false, so the state is IDLE or DONE (seen with
            // Acquire): the worker does not touch the job until the next
            // POSTED, which only `post` (&mut self) stores. The borrow of
            // self keeps `post` from running while this reference lives.
            Runs::Worker { shared, .. } => unsafe { &mut *shared.job.get() },
        }
    }
}

impl<J: Job> Drop for Lane<J> {
    fn drop(&mut self) {
        let Runs::Worker { shared, thread } = &mut self.runs else {
            return;
        };
        // A posted block finishes first (up to a second); STOP must not
        // overwrite POSTED, or the worker would never see it.
        if self.out
            && !wait_done(
                shared,
                Instant::now() + Duration::from_secs(1),
                Duration::ZERO,
            )
        {
            // A job that never finishes: leave its thread parked, holding
            // its own Arc, rather than hang the caller.
            return;
        }
        shared.state.store(STOP, Ordering::SeqCst);
        os::wake(&shared.state);
        if let Some(t) = thread.take() {
            let _ = t.join();
        }
    }
}

/// What a strict lane could not have (its core, its priority), or None.
fn missing(
    pinned: &Option<Result<usize, String>>,
    fifo: &Option<Result<i32, String>>,
) -> Option<String> {
    let mut why = Vec::new();
    if let Some(Err(e)) = pinned {
        why.push(format!("not pinned: {e}"));
    }
    if let Some(Err(e)) = fifo {
        why.push(format!("no SCHED_FIFO: {e}"));
    }
    (!why.is_empty()).then(|| format!("strict lane, {}", why.join("; ")))
}

/// Spins, then sleeps, until the state is DONE or the deadline passes.
fn wait_done<J>(shared: &Shared<J>, deadline: Instant, spin: Duration) -> bool {
    let started = Instant::now();
    let spin_until = started.checked_add(spin).unwrap_or(deadline).min(deadline);
    let mut n = 0u32;
    loop {
        if shared.state.load(Ordering::Acquire) == DONE {
            return true;
        }
        n = n.wrapping_add(1);
        if n & CLOCK_MASK != 0 {
            std::hint::spin_loop();
            continue;
        }
        let now = Instant::now();
        if now >= deadline {
            return shared.state.load(Ordering::Acquire) == DONE;
        }
        if now < spin_until {
            std::hint::spin_loop();
            continue;
        }
        shared.joiner_asleep.store(true, Ordering::SeqCst);
        let s = shared.state.load(Ordering::SeqCst);
        if s == POSTED {
            os::wait(&shared.state, POSTED, Some(deadline - now));
        }
        shared.joiner_asleep.store(false, Ordering::SeqCst);
    }
}

/// The worker thread: wait for POSTED, run the job, say DONE.
fn worker<J: Job>(shared: &Shared<J>) {
    let mut finished = Instant::now();
    loop {
        // Wait for POSTED or STOP: spin for `worker_spin` after the last
        // block, then sleep.
        let mut n = 0u32;
        let mut spinning = !shared.worker_spin.is_zero();
        loop {
            match shared.state.load(Ordering::Acquire) {
                POSTED => break,
                STOP => return,
                _ => {}
            }
            if spinning {
                n = n.wrapping_add(1);
                if n & CLOCK_MASK == 0 && finished.elapsed() >= shared.worker_spin {
                    spinning = false;
                }
                std::hint::spin_loop();
                continue;
            }
            shared.worker_asleep.store(true, Ordering::SeqCst);
            let s = shared.state.load(Ordering::SeqCst);
            if s == IDLE || s == DONE {
                os::wait(&shared.state, s, None);
            }
            shared.worker_asleep.store(false, Ordering::SeqCst);
        }
        let block = shared.block.load(Ordering::Relaxed);
        let before = mpfree_prof::alloc::thread_count();
        #[allow(unsafe_code)]
        // SAFETY: POSTED was seen with Acquire: the job is this thread's
        // until DONE is stored below (see `Shared`).
        unsafe {
            (*shared.job.get()).run(block);
        }
        let made = mpfree_prof::alloc::thread_count().wrapping_sub(before);
        if made > 0 {
            shared.worker_allocs.fetch_add(made, Ordering::Relaxed);
        }
        finished = Instant::now();
        // SeqCst with the joiner's (asleep = true; read state), as in `post`.
        shared.state.store(DONE, Ordering::SeqCst);
        if shared.joiner_asleep.load(Ordering::SeqCst) {
            os::wake(&shared.state);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    const FRAMES: usize = 128;

    /// A job with its own output buffer: block b's samples, made the slow way.
    struct Tone {
        out: Vec<f32>,
    }

    impl Tone {
        fn new() -> Self {
            Self {
                out: vec![0.0; FRAMES],
            }
        }
        fn expected(block: u64, i: usize) -> f32 {
            ((block as f32 * 0.37) + i as f32 * 0.011).sin()
        }
    }

    impl Job for Tone {
        fn run(&mut self, block: u64) {
            for (i, s) in self.out.iter_mut().enumerate() {
                *s = Self::expected(block, i);
            }
        }
    }

    fn soon() -> Instant {
        Instant::now() + Duration::from_secs(5)
    }

    fn check(lane: &mut Lane<Tone>, blocks: u64) {
        for b in 0..blocks {
            lane.post(b).expect("posted");
            let job = lane.join(soon()).expect("in time");
            for (i, &s) in job.out.iter().enumerate() {
                assert_eq!(s, Tone::expected(b, i), "block {b} frame {i}");
            }
        }
        assert_eq!(lane.stats().posted, blocks);
        assert_eq!(lane.stats().joined, blocks);
        assert_eq!(lane.stats().late, 0);
    }

    #[test]
    fn a_lane_renders_what_the_job_renders_here() {
        let mut lane = Lane::new(Tone::new(), Config::default());
        assert!(lane.threaded(), "{:?}", lane.setup());
        check(&mut lane, 500);
        let mut here = Lane::inline(Tone::new());
        assert!(!here.threaded());
        check(&mut here, 500);
    }

    #[test]
    fn sleeping_and_spinning_sides_give_the_same_blocks() {
        for (worker_spin, join_spin) in [
            (Duration::ZERO, Duration::ZERO),
            (Duration::ZERO, Duration::MAX),
            (Duration::from_millis(50), Duration::ZERO),
            (Duration::from_millis(50), Duration::MAX),
        ] {
            let mut lane = Lane::new(
                Tone::new(),
                Config {
                    worker_spin,
                    join_spin,
                    ..Config::default()
                },
            );
            check(&mut lane, 2_000);
        }
    }

    #[test]
    fn many_tiny_blocks_lose_no_wake_up() {
        // Both sides sleep every time: the futex path, many times over.
        let count = Arc::new(AtomicUsize::new(0));
        let c = Arc::clone(&count);
        let mut lane = Lane::new(
            move |_b: u64| {
                c.fetch_add(1, Ordering::Relaxed);
            },
            Config {
                join_spin: Duration::ZERO,
                ..Config::default()
            },
        );
        for b in 0..20_000 {
            lane.post(b).expect("posted");
            lane.join(soon()).expect("in time");
        }
        assert_eq!(count.load(Ordering::Relaxed), 20_000);
    }

    /// Sleeps on the blocks it is told to, so a join can be late.
    struct Slow {
        slow_on: u64,
        ran: Vec<u64>,
    }

    impl Job for Slow {
        fn run(&mut self, block: u64) {
            if block == self.slow_on {
                std::thread::sleep(Duration::from_millis(60));
            }
            self.ran.push(block);
        }
    }

    #[test]
    fn a_late_block_is_reported_and_the_lane_refuses_until_the_job_is_back() {
        let mut lane = Lane::new(
            Slow {
                slow_on: 3,
                ran: Vec::with_capacity(16),
            },
            Config::default(),
        );
        for b in 0..3 {
            lane.post(b).unwrap();
            lane.join(soon()).unwrap();
        }
        lane.post(3).unwrap();
        assert_eq!(
            lane.join(Instant::now() + Duration::from_millis(2))
                .map(|_| ()),
            Err(Late)
        );
        assert!(lane.job().is_none(), "the worker still has the job");
        assert_eq!(lane.post(4), Err(Busy));
        // It comes back by itself.
        let job = lane.join(soon()).expect("back");
        assert_eq!(job.ran, vec![0, 1, 2, 3]);
        lane.post(5).unwrap();
        assert_eq!(lane.join(soon()).unwrap().ran, vec![0, 1, 2, 3, 5]);
        let s = lane.stats();
        assert_eq!((s.posted, s.joined, s.late, s.busy), (5, 5, 1, 1));
    }

    #[test]
    fn a_job_that_comes_back_late_can_be_posted_again_without_a_join() {
        let mut lane = Lane::new(
            Slow {
                slow_on: 0,
                ran: Vec::with_capacity(4),
            },
            Config::default(),
        );
        lane.post(0).unwrap();
        assert!(lane.join(Instant::now()).is_err());
        // Wait for it to come back by itself, without a join.
        let give_up = soon();
        while lane.job().is_none() {
            assert!(Instant::now() < give_up, "the job never came back");
            std::thread::sleep(Duration::from_millis(5));
        }
        lane.post(1).expect("the job is back, so this posts");
        assert_eq!(lane.join(soon()).unwrap().ran, vec![0, 1]);
    }

    #[test]
    fn join_without_a_post_hands_the_job_back_at_once() {
        let mut lane = Lane::new(Tone::new(), Config::default());
        assert!(lane.join(Instant::now()).is_ok());
        assert!(lane.job().is_some());
        assert_eq!(lane.stats().joined, 0);
    }

    #[test]
    fn dropping_a_lane_stops_its_thread_and_drops_the_job() {
        let alive = Arc::new(());
        let held = Arc::clone(&alive);
        let lane = Lane::new(
            move |_b: u64| {
                let _ = &held;
            },
            Config::default(),
        );
        assert_eq!(Arc::strong_count(&alive), 2);
        drop(lane);
        assert_eq!(Arc::strong_count(&alive), 1);
    }

    #[test]
    fn dropping_a_lane_mid_block_waits_for_the_block() {
        let alive = Arc::new(());
        let held = Arc::clone(&alive);
        let mut lane = Lane::new(
            move |_b: u64| {
                let _ = &held;
                std::thread::sleep(Duration::from_millis(30));
            },
            Config::default(),
        );
        lane.post(0).unwrap();
        drop(lane);
        assert_eq!(Arc::strong_count(&alive), 1);
    }

    #[test]
    fn an_inline_lane_runs_on_the_calling_thread() {
        let me = std::thread::current().id();
        let mut lane = Lane::new(
            move |_b: u64| assert_eq!(std::thread::current().id(), me),
            Config {
                threaded: false,
                ..Config::default()
            },
        );
        assert!(lane.setup().threaded.is_err());
        lane.post(0).unwrap();
        lane.join(Instant::now()).unwrap();
    }

    #[test]
    fn the_worker_has_its_own_thread_and_name() {
        let me = std::thread::current().id();
        let mut lane = Lane::new(
            move |_b: u64| {
                assert_ne!(std::thread::current().id(), me);
                assert_eq!(std::thread::current().name(), Some("mpfree-lane-t"));
            },
            Config {
                name: "mpfree-lane-t".into(),
                ..Config::default()
            },
        );
        lane.post(0).unwrap();
        lane.join(soon()).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_pinned_worker_runs_on_its_core() {
        let core = *os::allowed_cores().last().expect("some core");
        let seen = Arc::new(AtomicUsize::new(usize::MAX));
        let s = Arc::clone(&seen);
        let mut lane = Lane::new(
            move |_b: u64| s.store(os::current_core().unwrap_or(usize::MAX), Ordering::Relaxed),
            Config {
                core: Some(core),
                ..Config::default()
            },
        );
        assert_eq!(lane.setup().pinned, Some(Ok(core)));
        for b in 0..20 {
            lane.post(b).unwrap();
            lane.join(soon()).unwrap();
            assert_eq!(seen.load(Ordering::Relaxed), core);
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_helper_thread_goes_to_the_background_cores_at_normal_priority() {
        let core = *os::allowed_cores().first().expect("some core");
        os::set_background(&[core]);
        assert_eq!(os::background_cores(), vec![core]);
        let got = std::thread::spawn(|| os::background().map(|()| os::allowed_cores()))
            .join()
            .unwrap();
        os::set_background(&[]);
        assert_eq!(got, Ok(vec![core]));
        assert!(os::background_cores().is_empty());
        // None named: only the policy changes.
        let all = os::allowed_cores();
        let got = std::thread::spawn(|| os::background().map(|()| os::allowed_cores()))
            .join()
            .unwrap();
        assert_eq!(got, Ok(all));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_long_job_runs_at_idle_priority_once_the_program_allows_it() {
        // The calling thread's policy, from its stat line (field 41).
        fn policy() -> u64 {
            let stat = std::fs::read_to_string("/proc/thread-self/stat").unwrap();
            let fields = &stat[stat.rfind(')').unwrap() + 1..];
            fields
                .split_whitespace()
                .nth(41 - 3)
                .unwrap()
                .parse()
                .unwrap()
        }
        let job = || {
            std::thread::spawn(|| {
                os::idle_job();
                policy()
            })
            .join()
            .unwrap()
        };
        // Off, as in tests and until a program turns it on: normal.
        assert_eq!(job(), 0, "SCHED_OTHER");
        let before = os::idle_counts();
        os::set_idle_jobs(true);
        let got = job();
        os::set_idle_jobs(false);
        assert_eq!(got, 5, "SCHED_IDLE");
        let after = os::idle_counts();
        assert_eq!((after.asked, after.refused), (before.asked + 1, 0));
        // Only the thread that asked changed.
        assert_eq!(policy(), 0);
    }

    #[test]
    fn a_strict_lane_without_its_core_runs_its_jobs_here() {
        let me = std::thread::current().id();
        let ran = Arc::new(AtomicUsize::new(0));
        let r = Arc::clone(&ran);
        let mut lane = Lane::new(
            move |_b: u64| {
                assert_eq!(std::thread::current().id(), me, "on the calling thread");
                r.fetch_add(1, Ordering::Relaxed);
            },
            Config {
                core: Some(100_000),
                strict: true,
                ..Config::default()
            },
        );
        assert!(!lane.threaded());
        let why = lane.setup().threaded.clone().unwrap_err();
        assert!(why.starts_with("strict lane, not pinned: "), "{why}");
        assert!(matches!(lane.setup().pinned, Some(Err(_))));
        for b in 0..10 {
            lane.post(b).unwrap();
            lane.join(Instant::now()).unwrap();
        }
        assert_eq!(ran.load(Ordering::Relaxed), 10);
        // The same output as a worker would give.
        let mut tone = Lane::new(
            Tone::new(),
            Config {
                core: Some(100_000),
                strict: true,
                ..Config::default()
            },
        );
        check(&mut tone, 50);
    }

    #[test]
    fn a_strict_lane_with_nothing_to_ask_for_has_its_worker() {
        let mut lane = Lane::new(
            Tone::new(),
            Config {
                strict: true,
                ..Config::default()
            },
        );
        assert!(lane.threaded(), "{:?}", lane.setup());
        check(&mut lane, 50);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_strict_lane_that_gets_its_core_has_its_worker() {
        let core = *os::allowed_cores().last().expect("some core");
        let mut lane = Lane::new(
            Tone::new(),
            Config {
                core: Some(core),
                strict: true,
                ..Config::default()
            },
        );
        assert!(lane.threaded(), "{:?}", lane.setup());
        assert_eq!(lane.setup().pinned, Some(Ok(core)));
        check(&mut lane, 50);
    }

    #[test]
    fn a_core_or_priority_that_cannot_be_had_leaves_the_lane_working() {
        // Core 100000 is past any machine; FIFO usually needs privileges.
        let mut lane = Lane::new(
            Tone::new(),
            Config {
                core: Some(100_000),
                fifo: Some(1),
                ..Config::default()
            },
        );
        assert!(matches!(lane.setup().pinned, Some(Err(_))));
        assert!(lane.setup().fifo.is_some());
        check(&mut lane, 50);
    }
}
