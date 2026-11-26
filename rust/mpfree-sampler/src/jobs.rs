//! One job service for the sampler's work away from the screen and the
//! sound (sampler plan, phase 2 step 7).
//!
//! The sampler used to start a thread of its own for each kind of heavy
//! work: a sound's view, its hits, its octave copies, Warp's stretched
//! copies, Process, loads. Each had its own queue, none knew about the
//! others, and none checked memory against what the rest were holding.
//! [`Jobs`] is one service in their place, moved onto it a few kinds at a
//! time:
//!
//! - **Two lanes**, so different work doesn't wait behind each other:
//!   [`Lane::Sound`] (working out views, pitch, hits, octave copies,
//!   Process) and [`Lane::File`] (reading sounds and kits), so
//!   a long analysis never holds up a load. Each lane has its own workers,
//!   started the first time it gets a job, at the priority the app gives
//!   its lane ([`Config::on_start`]).
//! - **One order.** A job for the sound being edited or shown goes first
//!   ([`Order::First`]); the rest wait their turn. Each job has a [`Key`]:
//!   what it works out, for which sound. A newer job with the key of one
//!   still waiting takes its place, and one already running is told to
//!   stop ([`Running::cancelled`]), so a quick run of changes works out
//!   only the last.
//! - **One memory account.** A job says what it will hold
//!   ([`Job::needs`]) and starts only with a promise of that much
//!   ([`Config::promise`]), held while it runs; otherwise it is refused,
//!   counted, and told why, and never starts. In the app the promise is the
//!   shared memory check's (`memory::reserve`): jobs, loads and work that
//!   holds memory back for later are promised under one lock, so two of
//!   them never count on the same free memory.
//! - **One way to cancel**: by key ([`Jobs::cancel`]) or by what a client
//!   keeps ([`Jobs::retain`]).
//! - **One a process.** [`Jobs::shared`] is the process's service, made on
//!   first use with the config the program gave [`configure`] at its start
//!   (the default one, for tests and offline renders, if it gave none). A
//!   test can still make a service of its own with [`Jobs::new`].
//! - **Warp's copies go on the stretch scheduler** ([`Jobs::stretch`],
//!   `mpfree_stretch::sched`, the audio tracks rethink 5b.3): one worker,
//!   a step at a time, for every stretch MPFree makes ahead, within one
//!   budget for derived audio. The process's service has the process's
//!   scheduler; a service made for a test has its own, started as its
//!   sound lane's workers are and promising memory as it does. Keys,
//!   holding, counts and memory reserved take in its jobs.
//! - **Nothing falls back to the screen.** If a worker can't start, its
//!   jobs wait, the failure is counted ([`Counts::spawn_failed`]) and the
//!   next job tries again. Only a service made for a test runs jobs in
//!   place ([`Jobs::in_place`]); the app never does.
//!
//! A job's result goes back however its client likes (usually a channel
//! the closure holds). Each result should carry what it was worked out for
//! (the sound, the settings, the edit), so a client can throw away one
//! that is out of date.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};

use mpfree_stretch::sched::{self, Priority, Sched};

/// Which workers run a job.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lane {
    /// Working out from sounds in memory: views, pitch, hits, octave
    /// copies, stretching, Process.
    Sound,
    /// Reading and writing files: loading sounds, opening kits.
    File,
}

impl Lane {
    fn index(self) -> usize {
        match self {
            Lane::Sound => 0,
            Lane::File => 1,
        }
    }

    fn thread_name(self, n: usize) -> String {
        match self {
            Lane::Sound => format!("mpfree-sound-{n}"),
            Lane::File => format!("mpfree-file-{n}"),
        }
    }
}

/// What a job works out, and for which sound: two jobs with one key are
/// the same work, and the newer replaces the older.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Key {
    /// The kind of work, such as "view" or "pitch".
    pub kind: &'static str,
    /// The sound (its id) or other thing it is for.
    pub id: u128,
}

impl Key {
    pub fn new(kind: &'static str, id: u128) -> Key {
        Key { kind, id }
    }

    /// The same key on the stretch scheduler ([`Jobs::stretch`]).
    pub fn stretch(self) -> sched::Key {
        sched::Key::new(self.kind, self.id)
    }
}

/// Where a new job waits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Order {
    /// Ahead of everything waiting: the sound being edited or shown.
    First,
    /// Behind everything waiting.
    Last,
    /// Right behind the job with this key if it is waiting, else first.
    After(Key),
}

/// What a running job can ask the service.
pub struct Running {
    cancel: Arc<AtomicBool>,
}

impl Running {
    /// Whether the job was replaced by a newer one with its key, or
    /// cancelled, since it started: a job that can stop halfway should,
    /// and whatever it sends is out of date.
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

type Run = Box<dyn FnOnce(&Running) + Send>;
type Refused = Box<dyn FnOnce(String) + Send>;

/// One piece of work for the service.
pub struct Job {
    lane: Lane,
    key: Key,
    order: Order,
    need: u64,
    run: Run,
    refused: Option<Refused>,
}

impl Job {
    /// A job on `lane` with `key`, waiting last, holding no memory to
    /// speak of.
    pub fn new(lane: Lane, key: Key, run: impl FnOnce(&Running) + Send + 'static) -> Job {
        Job {
            lane,
            key,
            order: Order::Last,
            need: 0,
            run: Box::new(run),
            refused: None,
        }
    }

    /// The job waits at `order`.
    pub fn order(mut self, order: Order) -> Job {
        self.order = order;
        self
    }

    /// The job holds about `bytes` while it runs (and what it makes, after),
    /// so it starts only when that leaves the floor free; when it can't,
    /// `refused` is told why instead, and the job never runs.
    pub fn needs(mut self, bytes: u64, refused: impl FnOnce(String) + Send + 'static) -> Job {
        self.need = bytes;
        self.refused = Some(Box::new(refused));
        self
    }
}

/// How the service runs.
#[derive(Clone, Copy)]
pub struct Config {
    /// Workers on [`Lane::Sound`].
    pub sound_workers: usize,
    /// Workers on [`Lane::File`].
    pub file_workers: usize,
    /// Run first on each worker's own thread, with its lane: the app puts
    /// the sound lane at idle priority (its work is long and nobody waits
    /// on it frame by frame, so the screen goes first) and the file lane at
    /// the normal one, both off the sound's cores.
    pub on_start: fn(Lane),
    /// Promises a job's `need` bytes while it runs, if they fit beside the
    /// floor and every other promise standing; Err says why, in a
    /// musician's words. The promise ends when its [`Hold`] drops, as the
    /// job ends. The app's is the shared memory check, `memory::reserve`.
    pub promise: fn(u64) -> Result<Hold, String>,
}

/// A promise of memory a running job holds ([`Config::promise`]): whatever
/// the promise's maker returns, kept until the job ends. The stretch
/// scheduler's own, so one promise serves both ([`Jobs::stretch`]).
pub use mpfree_stretch::sched::Hold;

impl Default for Config {
    /// One worker a lane, nothing done at their start, and memory never
    /// short: for tests.
    fn default() -> Config {
        Config {
            sound_workers: 1,
            file_workers: 1,
            on_start: |_| {},
            promise: |_| Ok(Hold::none()),
        }
    }
}

/// What the service has done since it started.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    /// Jobs that ran to their end.
    pub done: u64,
    /// Jobs replaced by a newer one with their key, waiting or running.
    pub replaced: u64,
    /// Jobs cancelled ([`Jobs::cancel`], [`Jobs::retain`]), waiting or
    /// running.
    pub cancelled: u64,
    /// Jobs refused for want of memory.
    pub refused: u64,
    /// Workers that couldn't start. Their jobs wait for the next try.
    pub spawn_failed: u64,
}

struct Busy {
    key: Key,
    cancel: Arc<AtomicBool>,
}

#[derive(Default)]
struct State {
    waiting: VecDeque<Job>,
    running: Vec<Busy>,
    /// Workers alive on each lane.
    live: [usize; 2],
    /// Memory the running jobs said they hold (promised while they run).
    reserved: u64,
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
        // A job never runs while the lock is held, so nothing can poison it
        // halfway through a change: carry on with what is there.
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// The handles' common part: when the last handle goes, the workers stop
/// after the job they are on.
struct Owner {
    shared: Arc<Shared>,
    in_place: bool,
    /// Where the Warps' stretched copies are made ([`Jobs::stretch`]).
    stretch: Sched,
}

impl Drop for Owner {
    fn drop(&mut self) {
        let mut s = self.shared.lock();
        s.stop = true;
        s.waiting.clear();
        for b in &s.running {
            b.cancel.store(true, Ordering::Relaxed);
        }
        drop(s);
        self.shared.cv.notify_all();
    }
}

/// The job service. Cloning gives another handle to the same service.
#[derive(Clone)]
pub struct Jobs(Arc<Owner>);

/// The process's service, once made ([`Jobs::shared`]).
static SHARED: OnceLock<Jobs> = OnceLock::new();
/// Its config until then ([`configure`]).
static CONFIG: Mutex<Option<Config>> = Mutex::new(None);

/// Gives the process's service ([`Jobs::shared`]) its config, before its
/// first use: the program does this at its start. False, and the config
/// unchanged, if the service is made already.
pub fn configure(config: Config) -> bool {
    if SHARED.get().is_some() {
        return false;
    }
    *CONFIG.lock().unwrap_or_else(|e| e.into_inner()) = Some(config);
    SHARED.get().is_none()
}

impl Jobs {
    /// The process's one service, made the first time it is asked for,
    /// with the config given to [`configure`] (or the default).
    pub fn shared() -> Jobs {
        SHARED
            .get_or_init(|| {
                let config = CONFIG
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .take()
                    .unwrap_or_default();
                // The process's one stretch scheduler, which the program
                // configures as it does this service's sound lane.
                Jobs::make(config, false, Sched::shared())
            })
            .clone()
    }

    /// Whether two handles are to the same service.
    pub fn same(&self, other: &Jobs) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    /// A service whose workers start as their lane's first job comes,
    /// with a stretch scheduler of its own promising memory as it does.
    pub fn new(config: Config) -> Jobs {
        Jobs::make(config, false, Jobs::own_stretch(&config))
    }

    /// A service for tests that runs each job on the caller as it comes,
    /// with the same memory check. The app never uses it. Its stretch
    /// scheduler is its own, with a worker as [`Jobs::new`]'s: Warp's
    /// copies are made on that worker, not on the caller.
    pub fn in_place(config: Config) -> Jobs {
        Jobs::make(config, true, Jobs::own_stretch(&config))
    }

    /// A stretch scheduler for a service of its own: its worker starts as
    /// this service's sound lane's do, and it promises memory as this
    /// service does.
    fn own_stretch(config: &Config) -> Sched {
        let start = config.on_start;
        Sched::new(sched::Config {
            on_start: Arc::new(move || start(Lane::Sound)),
            promise: config.promise,
            ..sched::Config::default()
        })
    }

    fn make(config: Config, in_place: bool, stretch: Sched) -> Jobs {
        Jobs(Arc::new(Owner {
            shared: Arc::new(Shared {
                state: Mutex::new(State::default()),
                cv: Condvar::new(),
                config,
            }),
            in_place,
            stretch,
        }))
    }

    /// The stretch scheduler the Warps' copies are made on
    /// (`mpfree_stretch::sched`, the audio tracks rethink 5b.3): one worker
    /// for every stretch made ahead, the tracks' tempo copies too, within
    /// one budget for derived audio. The process's service has the
    /// process's scheduler; a test's service has its own. [`Jobs::hold`],
    /// [`Jobs::waiting`], [`Jobs::busy`], [`Jobs::counts`] and
    /// [`Jobs::reserved`] count its jobs with this service's.
    pub fn stretch(&self) -> &Sched {
        &self.0.stretch
    }

    fn inner(&self) -> &Arc<Shared> {
        &self.0.shared
    }

    /// Puts `job` in the queue at its order. A job with its key that is
    /// still waiting is dropped for it, and one running is told to stop.
    pub fn submit(&self, job: Job) {
        if self.0.in_place {
            let shared = self.inner();
            let cancel = {
                let mut s = shared.lock();
                replace(&mut s, job.key);
                take(&mut s, job.key)
            };
            run_one(shared, job, cancel);
            return;
        }
        let lane = job.lane;
        {
            let mut s = self.inner().lock();
            if s.stop {
                return;
            }
            replace(&mut s, job.key);
            let at = match job.order {
                Order::First => 0,
                Order::Last => s.waiting.len(),
                Order::After(k) => s
                    .waiting
                    .iter()
                    .position(|w| w.key == k)
                    .map_or(0, |i| i + 1),
            };
            s.waiting.insert(at, job);
        }
        self.start_workers(lane);
        self.inner().cv.notify_all();
    }

    /// Moves the job with `key` to the front if it is waiting. True if it
    /// is waiting or running, so the caller needn't submit it again. A
    /// stretch on the stretch scheduler with that key is raised there to
    /// [`Priority::Protect`] (what something playing needs next).
    pub fn promote(&self, key: Key) -> bool {
        if self
            .0
            .stretch
            .reorder(key.stretch(), Priority::Protect, None)
        {
            return true;
        }
        let mut s = self.inner().lock();
        if s.running.iter().any(|b| b.key == key) {
            return true;
        }
        match s.waiting.iter().position(|w| w.key == key) {
            Some(i) => {
                if i > 0 {
                    if let Some(w) = s.waiting.remove(i) {
                        s.waiting.push_front(w);
                    }
                }
                true
            }
            None => false,
        }
    }

    /// Whether a job with `key` is waiting or running, here or on the
    /// stretch scheduler.
    pub fn has(&self, key: Key) -> bool {
        if self.0.stretch.has(key.stretch()) {
            return true;
        }
        let s = self.inner().lock();
        s.running.iter().any(|b| b.key == key) || s.waiting.iter().any(|w| w.key == key)
    }

    /// Drops the job with `key` if it waits, and tells it to stop if it
    /// runs, here or on the stretch scheduler.
    pub fn cancel(&self, key: Key) {
        self.0.stretch.cancel(key.stretch());
        self.retain(|k| k != key);
    }

    /// Keeps only the jobs whose key `keep` says yes to: the rest are
    /// dropped if waiting and told to stop if running.
    pub fn retain(&self, keep: impl Fn(Key) -> bool) {
        let mut s = self.inner().lock();
        let before = s.waiting.len();
        s.waiting.retain(|w| keep(w.key));
        let mut gone = (before - s.waiting.len()) as u64;
        for b in &s.running {
            if !keep(b.key) && !b.cancel.swap(true, Ordering::Relaxed) {
                gone += 1;
            }
        }
        s.counts.cancelled += gone;
    }

    /// Holds every worker before its next job (`true`), or lets them go
    /// on: for tests that look at what comes before a job's result.
    pub fn hold(&self, held: bool) {
        self.0.stretch.hold(held);
        self.inner().lock().held = held;
        self.inner().cv.notify_all();
    }

    /// Jobs waiting, on both lanes and the stretch scheduler (not started
    /// there).
    pub fn waiting(&self) -> usize {
        self.inner().lock().waiting.len() + self.0.stretch.waiting()
    }

    /// Whether any job is waiting or running, the stretch scheduler's too.
    pub fn busy(&self) -> bool {
        // The scheduler first, so its lock is never taken under this one.
        if !self.0.stretch.is_empty() {
            return true;
        }
        let s = self.inner().lock();
        !s.waiting.is_empty() || !s.running.is_empty()
    }

    /// What the service has done so far, with the stretch scheduler's.
    pub fn counts(&self) -> Counts {
        let mut c = self.inner().lock().counts;
        let st = self.0.stretch.counts();
        c.done += st.done;
        c.replaced += st.replaced;
        c.cancelled += st.cancelled;
        c.refused += st.refused;
        c.spawn_failed += st.spawn_failed;
        c
    }

    /// Memory the running jobs said they hold, the stretch scheduler's
    /// too.
    pub fn reserved(&self) -> u64 {
        self.inner().lock().reserved + self.0.stretch.reserved()
    }

    /// Starts `lane`'s workers up to its number. One that can't start is
    /// counted, and its jobs wait for the next try.
    fn start_workers(&self, lane: Lane) {
        let shared = self.inner();
        let want = match lane {
            Lane::Sound => shared.config.sound_workers,
            Lane::File => shared.config.file_workers,
        }
        .max(1);
        loop {
            let n = {
                let mut s = shared.lock();
                if s.stop || s.live[lane.index()] >= want {
                    return;
                }
                s.live[lane.index()] += 1;
                s.live[lane.index()]
            };
            let worker = shared.clone();
            let spawned = std::thread::Builder::new()
                .name(lane.thread_name(n))
                .spawn(move || {
                    (worker.config.on_start)(lane);
                    work(&worker, lane);
                });
            if spawned.is_err() {
                let mut s = shared.lock();
                s.live[lane.index()] -= 1;
                s.counts.spawn_failed += 1;
                return;
            }
        }
    }
}

/// Drops a waiting job with `key` and tells a running one to stop, each
/// counted as replaced.
fn replace(s: &mut State, key: Key) {
    let before = s.waiting.len();
    s.waiting.retain(|w| w.key != key);
    let mut gone = (before - s.waiting.len()) as u64;
    for b in &s.running {
        if b.key == key && !b.cancel.swap(true, Ordering::Relaxed) {
            gone += 1;
        }
    }
    s.counts.replaced += gone;
}

/// Marks the job with `key` running as it leaves the queue, in the same
/// hold of the lock, so there is no moment when it is in neither and
/// [`Jobs::busy`] says nothing is (train 155's race: a test that waited
/// for the service to settle went on while a job was between the two).
/// It can be told to stop from here on. Its cancel flag, for [`run_one`].
fn take(s: &mut State, key: Key) -> Arc<AtomicBool> {
    let cancel = Arc::new(AtomicBool::new(false));
    s.running.push(Busy {
        key,
        cancel: cancel.clone(),
    });
    cancel
}

/// The job with `cancel` no longer running ([`take`]).
fn finish(s: &mut State, cancel: &Arc<AtomicBool>) {
    if let Some(i) = s
        .running
        .iter()
        .position(|b| Arc::ptr_eq(&b.cancel, cancel))
    {
        s.running.swap_remove(i);
    }
}

/// A worker on `lane`: takes the first job of its lane, checks its memory,
/// runs it; until the service stops.
fn work(shared: &Shared, lane: Lane) {
    loop {
        let (job, cancel) = {
            let mut s = shared.lock();
            loop {
                if s.stop {
                    s.live[lane.index()] -= 1;
                    return;
                }
                if !s.held {
                    if let Some(i) = s.waiting.iter().position(|w| w.lane == lane) {
                        if let Some(j) = s.waiting.remove(i) {
                            let cancel = take(&mut s, j.key);
                            break (j, cancel);
                        }
                    }
                }
                s = shared.cv.wait(s).unwrap_or_else(|e| e.into_inner());
            }
        };
        run_one(shared, job, cancel);
        shared.cv.notify_all();
    }
}

/// Runs `job` here, if its memory allows: the worker's step, or the caller's
/// for a service made [in place](Jobs::in_place). The job is running
/// already ([`take`], with `cancel`) and stays so until it has run, or
/// has been refused and told why.
fn run_one(shared: &Shared, job: Job, cancel: Arc<AtomicBool>) {
    let Job {
        need, run, refused, ..
    } = job;
    // Promised outside the service's lock: the promise reads the memory
    // free (a file read on the device) under its own.
    let promise = if need > 0 {
        (shared.config.promise)(need)
    } else {
        Ok(Hold::none())
    };
    let _held = match promise {
        Ok(held) => held,
        Err(why) => {
            shared.lock().counts.refused += 1;
            if let Some(r) = refused {
                r(why);
            }
            finish(&mut shared.lock(), &cancel);
            return;
        }
    };
    shared.lock().reserved += need;
    run(&Running {
        cancel: cancel.clone(),
    });
    let mut s = shared.lock();
    s.reserved -= need;
    finish(&mut s, &cancel);
    s.counts.done += 1;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    /// A memory account for a test, as the app's shared check keeps one:
    /// `free` bytes, `floor` kept spare, promises held until dropped.
    struct Account {
        free: u64,
        floor: u64,
        held: Mutex<u64>,
    }

    struct Promised(&'static Account, u64);

    impl Drop for Promised {
        fn drop(&mut self) {
            *self.0.held.lock().unwrap() -= self.1;
        }
    }

    impl Account {
        const fn new(free: u64, floor: u64) -> Account {
            Account {
                free,
                floor,
                held: Mutex::new(0),
            }
        }

        fn promise(&'static self, need: u64) -> Result<Hold, String> {
            let mut held = self.held.lock().unwrap();
            if self.free.saturating_sub(*held).saturating_sub(need) < self.floor {
                return Err(format!(
                    "not enough memory: needs {need}, {} free, {} promised",
                    self.free, *held
                ));
            }
            *held += need;
            Ok(Hold::new(Promised(self, need)))
        }

        fn promised(&self) -> u64 {
            *self.held.lock().unwrap()
        }
    }

    fn settle(jobs: &Jobs) {
        let end = Instant::now() + Duration::from_secs(10);
        while jobs.busy() {
            assert!(Instant::now() < end, "the jobs never finished");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// A job that sends `n` when it runs.
    fn say(lane: Lane, kind: &'static str, n: u64, tx: &mpsc::Sender<u64>) -> Job {
        let tx = tx.clone();
        Job::new(lane, Key::new(kind, u128::from(n)), move |_| {
            let _ = tx.send(n);
        })
    }

    #[test]
    fn jobs_run_in_their_order_and_the_first_goes_ahead() {
        let jobs = Jobs::new(Config::default());
        let (tx, rx) = mpsc::channel();
        jobs.hold(true);
        jobs.submit(say(Lane::Sound, "a", 1, &tx));
        jobs.submit(say(Lane::Sound, "a", 2, &tx));
        jobs.submit(say(Lane::Sound, "a", 3, &tx).order(Order::First));
        jobs.submit(say(Lane::Sound, "a", 4, &tx).order(Order::After(Key::new("a", 3))));
        assert_eq!(jobs.waiting(), 4);
        jobs.hold(false);
        settle(&jobs);
        assert_eq!(rx.try_iter().collect::<Vec<_>>(), vec![3, 4, 1, 2]);
        assert_eq!(jobs.counts().done, 4);
    }

    #[test]
    fn after_a_key_not_waiting_goes_first() {
        let jobs = Jobs::new(Config::default());
        let (tx, rx) = mpsc::channel();
        jobs.hold(true);
        jobs.submit(say(Lane::Sound, "a", 1, &tx));
        jobs.submit(say(Lane::Sound, "a", 2, &tx).order(Order::After(Key::new("b", 9))));
        jobs.hold(false);
        settle(&jobs);
        assert_eq!(rx.try_iter().collect::<Vec<_>>(), vec![2, 1]);
    }

    #[test]
    fn a_newer_job_with_a_waiting_ones_key_takes_its_place() {
        let jobs = Jobs::new(Config::default());
        let (tx, rx) = mpsc::channel();
        jobs.hold(true);
        let key = Key::new("stretch", 7);
        for n in [10, 11, 12] {
            let tx = tx.clone();
            jobs.submit(Job::new(Lane::Sound, key, move |_| {
                let _ = tx.send(n);
            }));
        }
        assert_eq!(jobs.waiting(), 1);
        jobs.hold(false);
        settle(&jobs);
        assert_eq!(rx.try_iter().collect::<Vec<_>>(), vec![12]);
        assert_eq!(jobs.counts().replaced, 2);
    }

    #[test]
    fn a_running_job_is_told_to_stop_when_a_newer_one_comes() {
        let jobs = Jobs::new(Config::default());
        let key = Key::new("stretch", 1);
        let (started_tx, started) = mpsc::channel();
        let (go_tx, go) = mpsc::channel::<()>();
        let (out_tx, out) = mpsc::channel();
        let o = out_tx.clone();
        jobs.submit(Job::new(Lane::Sound, key, move |r| {
            let _ = started_tx.send(());
            let _ = go.recv();
            let _ = o.send(("first", r.cancelled()));
        }));
        started.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(jobs.has(key) && jobs.promote(key));
        jobs.submit(Job::new(Lane::Sound, key, move |r| {
            let _ = out_tx.send(("second", r.cancelled()));
        }));
        go_tx.send(()).unwrap();
        settle(&jobs);
        assert_eq!(
            out.try_iter().collect::<Vec<_>>(),
            vec![("first", true), ("second", false)]
        );
        assert_eq!(jobs.counts().replaced, 1);
    }

    #[test]
    fn promote_moves_a_waiting_job_to_the_front_and_says_whether_there_is_one() {
        let jobs = Jobs::new(Config::default());
        let (tx, rx) = mpsc::channel();
        jobs.hold(true);
        for n in 1..=3 {
            jobs.submit(say(Lane::Sound, "view", n, &tx));
        }
        assert!(jobs.promote(Key::new("view", 3)));
        assert!(!jobs.promote(Key::new("view", 4)));
        assert!(!jobs.has(Key::new("view", 4)));
        jobs.hold(false);
        settle(&jobs);
        assert_eq!(rx.try_iter().collect::<Vec<_>>(), vec![3, 1, 2]);
    }

    /// The audio tracks rethink 5b.3: promote on the key of a stretch on
    /// the stretch scheduler (the Warps' copies) raises it there, to what
    /// something playing needs next.
    #[test]
    fn promote_raises_a_stretch_waiting_on_the_stretch_scheduler() {
        struct Say(u32, mpsc::Sender<u32>);
        impl sched::Work for Say {
            fn step(&mut self, _: usize) -> bool {
                true
            }
            fn finish(self: Box<Self>) {
                let _ = self.1.send(self.0);
            }
        }
        let jobs = Jobs::new(Config::default());
        let (tx, rx) = mpsc::channel();
        jobs.hold(true);
        let st = jobs.stretch();
        for (n, priority) in [(1, Priority::Prepared), (2, Priority::Speculative)] {
            let key = Key::new("stretch", u128::from(n)).stretch();
            st.submit(sched::Job::new(key, priority, Say(n, tx.clone())));
        }
        assert!(jobs.has(Key::new("stretch", 2)));
        assert_eq!(jobs.waiting(), 2);
        assert!(jobs.promote(Key::new("stretch", 2)));
        jobs.hold(false);
        let got: Vec<u32> = (0..2)
            .map(|_| rx.recv_timeout(Duration::from_secs(10)).unwrap())
            .collect();
        assert_eq!(got, vec![2, 1]);
        assert_eq!(jobs.counts().done, 2);
    }

    #[test]
    fn cancelled_and_unkept_jobs_never_run() {
        let jobs = Jobs::new(Config::default());
        let (tx, rx) = mpsc::channel();
        jobs.hold(true);
        for n in 1..=5 {
            jobs.submit(say(Lane::Sound, "view", n, &tx));
        }
        jobs.cancel(Key::new("view", 2));
        jobs.retain(|k| k.id != 4);
        jobs.hold(false);
        settle(&jobs);
        assert_eq!(rx.try_iter().collect::<Vec<_>>(), vec![1, 3, 5]);
        assert_eq!(jobs.counts().cancelled, 2);
    }

    #[test]
    fn a_long_sound_job_never_holds_up_a_file_job() {
        let jobs = Jobs::new(Config::default());
        let (go_tx, go) = mpsc::channel::<()>();
        let (tx, rx) = mpsc::channel();
        let t = tx.clone();
        jobs.submit(Job::new(Lane::Sound, Key::new("view", 1), move |_| {
            let _ = go.recv();
            let _ = t.send("view");
        }));
        jobs.submit(Job::new(Lane::File, Key::new("load", 2), move |_| {
            let _ = tx.send("load");
        }));
        assert_eq!(rx.recv_timeout(Duration::from_secs(10)), Ok("load"));
        go_tx.send(()).unwrap();
        settle(&jobs);
        assert_eq!(rx.try_iter().collect::<Vec<_>>(), vec!["view"]);
    }

    /// A job is running from the moment it leaves the queue (train 155's
    /// race): while its memory is promised, before it runs, the service is
    /// busy and has it, and a newer job with its key, or a cancel, tells
    /// it to stop. A refused one stays running until it has been told why.
    #[test]
    fn a_job_is_running_from_the_moment_it_leaves_the_queue() {
        static IN_PROMISE: AtomicBool = AtomicBool::new(false);
        static GATE: Mutex<bool> = Mutex::new(false);
        static OPEN: Condvar = Condvar::new();
        let config = Config {
            promise: |need| {
                IN_PROMISE.store(true, Ordering::SeqCst);
                let mut open = GATE.lock().unwrap();
                while !*open {
                    open = OPEN.wait(open).unwrap();
                }
                if need > 100 {
                    return Err("short".into());
                }
                Ok(Hold::none())
            },
            ..Config::default()
        };
        let jobs = Jobs::new(config);
        let wait_for_promise = || {
            let end = Instant::now() + Duration::from_secs(10);
            while !IN_PROMISE.load(Ordering::SeqCst) {
                assert!(Instant::now() < end, "the promise never came");
                std::thread::sleep(Duration::from_millis(1));
            }
        };
        let open = |to: bool| {
            *GATE.lock().unwrap() = to;
            OPEN.notify_all();
        };
        let (tx, rx) = mpsc::channel();
        let key = Key::new("view", 1);
        jobs.submit(
            Job::new(Lane::Sound, key, move |r| {
                let _ = tx.send(r.cancelled());
            })
            .needs(10, |w| panic!("{w}")),
        );
        wait_for_promise();
        assert_eq!(jobs.waiting(), 0, "it has left the queue");
        assert!(jobs.busy(), "and is still the service's");
        assert!(jobs.has(key));
        jobs.cancel(key);
        assert_eq!(jobs.counts().cancelled, 1, "told to stop before it runs");
        open(true);
        settle(&jobs);
        assert_eq!(rx.try_recv(), Ok(true), "it ran told to stop");

        // Refused: busy until it has been told why.
        open(false);
        IN_PROMISE.store(false, Ordering::SeqCst);
        let (said_tx, said) = mpsc::channel();
        let (go_tx, go) = mpsc::channel::<()>();
        jobs.submit(
            Job::new(Lane::Sound, Key::new("view", 2), |_| panic!("refused")).needs(
                500,
                move |w| {
                    let _ = go.recv();
                    let _ = said_tx.send(w);
                },
            ),
        );
        wait_for_promise();
        assert!(jobs.busy());
        open(true);
        std::thread::sleep(Duration::from_millis(20));
        assert!(jobs.busy(), "telling why is still its work");
        go_tx.send(()).unwrap();
        settle(&jobs);
        assert_eq!(said.try_recv().as_deref(), Ok("short"));
        assert_eq!(jobs.counts().refused, 1);
    }

    #[test]
    fn a_job_starts_only_when_its_memory_and_the_running_jobs_leave_the_floor() {
        // 1000 bytes free, 200 kept spare, as the app's shared check does.
        static MEMORY: Account = Account::new(1000, 200);
        let config = Config {
            promise: |need| MEMORY.promise(need),
            ..Config::default()
        };
        let jobs = Jobs::new(config);
        let (started_tx, started) = mpsc::channel();
        let (go_tx, go) = mpsc::channel::<()>();
        jobs.submit(
            Job::new(Lane::Sound, Key::new("big", 1), move |_| {
                let _ = started_tx.send(());
                let _ = go.recv();
            })
            .needs(600, |_| panic!("600 of 1000 leaves 400")),
        );
        started.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(jobs.reserved(), 600);
        // On the file lane, alongside: 300 more leaves 100.
        let (why_tx, why) = mpsc::channel();
        let (ran_tx, ran) = mpsc::channel();
        jobs.submit(
            Job::new(Lane::File, Key::new("load", 2), move |_| {
                let _ = ran_tx.send(());
            })
            .needs(300, move |w| {
                let _ = why_tx.send(w);
            }),
        );
        let w = why.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(w.starts_with("not enough memory"), "{w}");
        assert!(w.contains("600 promised"), "the running job's 600: {w}");
        // Another check of the same account (a load, say) sees it too.
        assert!(MEMORY.promise(300).is_err());
        go_tx.send(()).unwrap();
        settle(&jobs);
        assert!(ran.try_recv().is_err(), "a refused job never runs");
        assert_eq!((jobs.reserved(), MEMORY.promised()), (0, 0));
        assert_eq!(jobs.counts().refused, 1);
        // With the first job done, the same load fits.
        let (ran_tx, ran) = mpsc::channel();
        jobs.submit(
            Job::new(Lane::File, Key::new("load", 2), move |_| {
                let _ = ran_tx.send(());
            })
            .needs(300, |w| panic!("{w}")),
        );
        ran.recv_timeout(Duration::from_secs(10)).unwrap();
    }

    #[test]
    fn a_service_for_tests_runs_each_job_in_place_with_the_same_checks() {
        let config = Config {
            promise: |need| {
                if need <= 100 {
                    Ok(Hold::none())
                } else {
                    Err("short".into())
                }
            },
            ..Config::default()
        };
        let jobs = Jobs::in_place(config);
        let (tx, rx) = mpsc::channel();
        jobs.submit(say(Lane::Sound, "view", 1, &tx));
        assert_eq!(rx.try_recv(), Ok(1), "ran before submit returned");
        let refused = Arc::new(Mutex::new(None));
        let r = refused.clone();
        jobs.submit(
            say(Lane::Sound, "view", 2, &tx).needs(500, move |w| *r.lock().unwrap() = Some(w)),
        );
        assert!(rx.try_recv().is_err());
        assert_eq!(refused.lock().unwrap().as_deref(), Some("short"));
        assert_eq!(
            jobs.counts(),
            Counts {
                done: 1,
                refused: 1,
                ..Counts::default()
            }
        );
    }

    #[test]
    fn workers_start_with_their_lanes_first_job_and_stop_when_the_service_goes() {
        let jobs = Jobs::new(Config::default());
        assert_eq!(jobs.inner().lock().live, [0, 0]);
        let (tx, rx) = mpsc::channel();
        jobs.submit(say(Lane::Sound, "view", 1, &tx));
        rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(jobs.inner().lock().live, [1, 0]);
        let shared = jobs.inner().clone();
        drop(jobs);
        let end = Instant::now() + Duration::from_secs(10);
        while shared.lock().live != [0, 0] {
            assert!(Instant::now() < end, "the worker never stopped");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn the_process_has_one_service_and_its_config_is_set_before_it_is_made() {
        let a = Jobs::shared();
        assert!(a.same(&Jobs::shared()));
        assert!(!a.same(&Jobs::new(Config::default())));
        assert!(!configure(Config::default()), "made already: too late");
        assert!(a.same(&Jobs::shared()));
    }

    #[test]
    fn the_running_job_is_told_to_stop_when_the_service_goes() {
        let jobs = Jobs::new(Config::default());
        let (started_tx, started) = mpsc::channel();
        let (go_tx, go) = mpsc::channel::<()>();
        let (out_tx, out) = mpsc::channel();
        jobs.submit(Job::new(Lane::Sound, Key::new("view", 1), move |r| {
            let _ = started_tx.send(());
            let _ = go.recv();
            let _ = out_tx.send(r.cancelled());
        }));
        started.recv_timeout(Duration::from_secs(10)).unwrap();
        drop(jobs);
        go_tx.send(()).unwrap();
        assert_eq!(out.recv_timeout(Duration::from_secs(10)), Ok(true));
    }
}
