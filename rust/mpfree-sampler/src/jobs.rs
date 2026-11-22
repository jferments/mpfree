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
//!   stretching, Process) and [`Lane::File`] (reading sounds and kits), so
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
//!   ([`Job::needs`]). It starts only when that leaves the floor free on
//!   top of what the running jobs hold and what is claimed
//!   ([`Config::room`]); otherwise it is refused, counted, and told why,
//!   and never starts. A [`Claim`] holds memory back for work that will
//!   use it later (preparing a song, say), so nothing else takes it
//!   meanwhile; it is given back as the work uses it, or when it is
//!   dropped. Checks made outside the service subtract what it holds back
//!   ([`Jobs::held_back`]), so a load and a job never count the same
//!   memory twice.
//! - **One way to cancel**: by key ([`Jobs::cancel`]) or by what a client
//!   keeps ([`Jobs::retain`]).
//! - **One a process.** [`Jobs::shared`] is the process's service, made on
//!   first use with the config the program gave [`configure`] at its start
//!   (the default one, for tests and offline renders, if it gave none). A
//!   test can still make a service of its own with [`Jobs::new`].
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
    /// The memory free now, None where it can't be read.
    pub free: fn() -> Option<u64>,
    /// Whether taking `need` more of the memory free (None: unknown) leaves
    /// the floor; Err says why, in a musician's words. The app's is
    /// `memory::room_in`.
    pub room: fn(Option<u64>, u64) -> Result<(), String>,
}

impl Default for Config {
    /// One worker a lane, nothing done at their start, and memory never
    /// short: for tests.
    fn default() -> Config {
        Config {
            sound_workers: 1,
            file_workers: 1,
            on_start: |_| {},
            free: || None,
            room: |_, _| Ok(()),
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
    /// Memory the running jobs said they hold.
    reserved: u64,
    /// Memory held back by claims ([`Claim`]).
    claimed: u64,
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
                Jobs::new(config)
            })
            .clone()
    }

    /// Whether two handles are to the same service.
    pub fn same(&self, other: &Jobs) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    /// A service whose workers start as their lane's first job comes.
    pub fn new(config: Config) -> Jobs {
        Jobs::make(config, false)
    }

    /// A service for tests that runs each job on the caller as it comes,
    /// with the same memory check. The app never uses it.
    pub fn in_place(config: Config) -> Jobs {
        Jobs::make(config, true)
    }

    fn make(config: Config, in_place: bool) -> Jobs {
        Jobs(Arc::new(Owner {
            shared: Arc::new(Shared {
                state: Mutex::new(State::default()),
                cv: Condvar::new(),
                config,
            }),
            in_place,
        }))
    }

    fn inner(&self) -> &Arc<Shared> {
        &self.0.shared
    }

    /// Puts `job` in the queue at its order. A job with its key that is
    /// still waiting is dropped for it, and one running is told to stop.
    pub fn submit(&self, job: Job) {
        if self.0.in_place {
            let shared = self.inner();
            {
                let mut s = shared.lock();
                replace(&mut s, job.key);
            }
            run_one(shared, job);
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
    /// is waiting or running, so the caller needn't submit it again.
    pub fn promote(&self, key: Key) -> bool {
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

    /// Whether a job with `key` is waiting or running.
    pub fn has(&self, key: Key) -> bool {
        let s = self.inner().lock();
        s.running.iter().any(|b| b.key == key) || s.waiting.iter().any(|w| w.key == key)
    }

    /// Drops the job with `key` if it waits, and tells it to stop if it runs.
    pub fn cancel(&self, key: Key) {
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
        self.inner().lock().held = held;
        self.inner().cv.notify_all();
    }

    /// Jobs waiting, on both lanes.
    pub fn waiting(&self) -> usize {
        self.inner().lock().waiting.len()
    }

    /// Whether any job is waiting or running.
    pub fn busy(&self) -> bool {
        let s = self.inner().lock();
        !s.waiting.is_empty() || !s.running.is_empty()
    }

    /// What the service has done so far.
    pub fn counts(&self) -> Counts {
        self.inner().lock().counts
    }

    /// Memory the running jobs said they hold.
    pub fn reserved(&self) -> u64 {
        self.inner().lock().reserved
    }

    /// Memory held back by claims ([`Jobs::claim`]).
    pub fn claimed(&self) -> u64 {
        self.inner().lock().claimed
    }

    /// What the service holds back from the memory free: what the running
    /// jobs said they hold and what is claimed. A memory check made outside
    /// the service subtracts it from the memory free, so it doesn't count
    /// the same memory twice.
    pub fn held_back(&self) -> u64 {
        let s = self.inner().lock();
        s.reserved + s.claimed
    }

    /// Holds `bytes` back for work that will use them later, if taking them
    /// from the memory free, less what the running jobs hold and what is
    /// claimed already, leaves the floor ([`Config::room`]); Err says why,
    /// in a musician's words. Until it is given back ([`Claim::give_back`],
    /// or dropped), every job and claim counts it. `what` names it for the
    /// Debug page ("preparing the next song").
    pub fn claim(&self, bytes: u64, what: &'static str) -> Result<Claim, String> {
        let shared = self.inner();
        let free = if bytes > 0 {
            (shared.config.free)()
        } else {
            None
        };
        let mut s = shared.lock();
        if bytes > 0 {
            let left = free.map(|f| f.saturating_sub(s.reserved + s.claimed));
            (shared.config.room)(left, bytes)?;
        }
        s.claimed += bytes;
        Ok(Claim {
            shared: shared.clone(),
            bytes,
            what,
        })
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

/// Memory held back for work that will use it later ([`Jobs::claim`]):
/// every job and claim counts it until it is given back, a part at a time
/// as the work uses it ([`Claim::give_back`]) or the rest when it is
/// dropped, so a claim forgotten can't hold memory for ever.
pub struct Claim {
    shared: Arc<Shared>,
    bytes: u64,
    what: &'static str,
}

impl Claim {
    /// What it still holds back.
    pub fn bytes(&self) -> u64 {
        self.bytes
    }

    /// What it is for.
    pub fn what(&self) -> &'static str {
        self.what
    }

    /// Names it again ("preparing" to "playing", say); what it holds stays.
    pub fn rename(&mut self, what: &'static str) {
        self.what = what;
    }

    /// Gives back up to `bytes` of it, as the work uses them (memory in use
    /// is counted in the memory free already).
    pub fn give_back(&mut self, bytes: u64) {
        let n = bytes.min(self.bytes);
        self.bytes -= n;
        self.shared.lock().claimed -= n;
        self.shared.cv.notify_all();
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        let n = self.bytes;
        self.give_back(n);
    }
}

impl std::fmt::Debug for Claim {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Claim")
            .field("bytes", &self.bytes)
            .field("what", &self.what)
            .finish()
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

/// A worker on `lane`: takes the first job of its lane, checks its memory,
/// runs it; until the service stops.
fn work(shared: &Shared, lane: Lane) {
    loop {
        let job = {
            let mut s = shared.lock();
            loop {
                if s.stop {
                    s.live[lane.index()] -= 1;
                    return;
                }
                if !s.held {
                    if let Some(i) = s.waiting.iter().position(|w| w.lane == lane) {
                        if let Some(j) = s.waiting.remove(i) {
                            break j;
                        }
                    }
                }
                s = shared.cv.wait(s).unwrap_or_else(|e| e.into_inner());
            }
        };
        run_one(shared, job);
        shared.cv.notify_all();
    }
}

/// Runs `job` here, if its memory allows: the worker's step, or the caller's
/// for a service made [in place](Jobs::in_place).
fn run_one(shared: &Shared, job: Job) {
    let Job {
        key,
        need,
        run,
        refused,
        ..
    } = job;
    // Read before the lock: a file read on the device.
    let free = if need > 0 {
        (shared.config.free)()
    } else {
        None
    };
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut s = shared.lock();
        if need > 0 {
            let left = free.map(|f| f.saturating_sub(s.reserved + s.claimed));
            if let Err(why) = (shared.config.room)(left, need) {
                s.counts.refused += 1;
                drop(s);
                if let Some(r) = refused {
                    r(why);
                }
                return;
            }
        }
        s.reserved += need;
        s.running.push(Busy {
            key,
            cancel: cancel.clone(),
        });
    }
    run(&Running {
        cancel: cancel.clone(),
    });
    let mut s = shared.lock();
    s.reserved -= need;
    if let Some(i) = s
        .running
        .iter()
        .position(|b| Arc::ptr_eq(&b.cancel, &cancel))
    {
        s.running.swap_remove(i);
    }
    s.counts.done += 1;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

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

    #[test]
    fn a_claim_holds_memory_back_from_jobs_and_claims_until_given_back() {
        // 1000 bytes free, 200 kept spare.
        let config = Config {
            free: || Some(1000),
            room: |free, need| match free {
                Some(f) if f.saturating_sub(need) >= 200 => Ok(()),
                _ => Err(format!("not enough memory: needs {need}, {free:?} free")),
            },
            ..Config::default()
        };
        let jobs = Jobs::in_place(config);
        let mut song = jobs.claim(500, "preparing the next song").unwrap();
        assert_eq!((jobs.claimed(), jobs.held_back()), (500, 500));
        assert_eq!(
            (song.bytes(), song.what()),
            (500, "preparing the next song")
        );
        // 400 more would leave 100: refused, for a job and a claim alike.
        let why = jobs.claim(400, "another").unwrap_err();
        assert!(why.contains("Some(500)"), "the claim counted: {why}");
        let (tx, rx) = mpsc::channel();
        jobs.submit(
            Job::new(Lane::Sound, Key::new("view", 1), |_| panic!("refused")).needs(
                400,
                move |w| {
                    let _ = tx.send(w);
                },
            ),
        );
        assert!(rx.try_recv().unwrap().contains("Some(500)"));
        // Given back as the work uses it: 300 of it in use now.
        song.give_back(300);
        assert_eq!((song.bytes(), jobs.claimed()), (200, 200));
        song.rename("playing");
        assert_eq!(song.what(), "playing");
        let (tx, rx) = mpsc::channel();
        jobs.submit(
            Job::new(Lane::Sound, Key::new("view", 1), move |_| {
                let _ = tx.send(());
            })
            .needs(400, |w| panic!("{w}")),
        );
        rx.try_recv().unwrap();
        // Giving back more than it holds gives back what it holds.
        song.give_back(1_000);
        assert_eq!(jobs.claimed(), 0);
        // Dropped, the rest comes back.
        let other = jobs.claim(600, "another").unwrap();
        assert_eq!(jobs.held_back(), 600);
        drop(other);
        assert_eq!(jobs.held_back(), 0);
        // Nothing to hold back never fails, with memory unknown or short.
        assert!(jobs.claim(0, "nothing").is_ok());
    }

    #[test]
    fn a_job_starts_only_when_its_memory_and_the_running_jobs_leave_the_floor() {
        // 1000 bytes free, 200 kept spare.
        let config = Config {
            free: || Some(1000),
            room: |free, need| match free {
                Some(f) if f.saturating_sub(need) >= 200 => Ok(()),
                _ => Err(format!("not enough memory: needs {need}, {free:?} free")),
            },
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
        assert!(
            w.contains("Some(400)"),
            "the running job's 600 counted: {w}"
        );
        go_tx.send(()).unwrap();
        settle(&jobs);
        assert!(ran.try_recv().is_err(), "a refused job never runs");
        assert_eq!(jobs.reserved(), 0);
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
            free: || Some(100),
            room: |free, need| match free {
                Some(f) if f >= need => Ok(()),
                _ => Err("short".into()),
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
