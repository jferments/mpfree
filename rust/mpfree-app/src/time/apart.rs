//! The control half on a thread of its own (Arrange 2.6b; the app's
//! `--time-thread`; docs/dev/design/time-threads.md, "Apart").
//!
//! Together (the default), [`super::TimeHost`] runs both halves on the
//! audio thread, so an action that compiles a big song's view takes its
//! time out of a period. Apart, the control half ([`TimeControl`]) runs on
//! `mpfree-time`, off the sound's cores at the normal priority, and the
//! facade keeps only the audio half and a copy of what the app reads:
//!
//! - **Asks.** The screen's and the buttons' actions and TRACK MUTE go to
//!   this thread as messages ([`ToThread::Run`]) and return at once. What
//!   they say comes back as [`Told::Said`], with how the caller wants it
//!   said ([`Say`]), so the words are the same as together.
//! - **Lends.** Every other call borrows the control half back for the
//!   call ([`ToThread::Lend`]): this thread hands it over and waits, the
//!   call runs on the caller's thread exactly as together, and it comes
//!   back. Same answers in the same order; each lend is counted and
//!   timed on the status line ([`Counts`]), so a device run shows which
//!   calls still cost the audio thread.
//! - **The published copy** ([`Shown`]). What the app reads (the screens'
//!   state, the steps, the mutes, the selection, the project for saving)
//!   is a copy this thread publishes: the cheap part ([`Light`]) whenever
//!   it differs, the rest ([`Heavy`]) after an ask, a lend or a change, and
//!   at least once a second. The facade takes the newest once a period and
//!   sends the [`Heavy`] it replaces back here to be freed, so the audio
//!   thread frees none of it. Apart, a read straight after an ask sees the
//!   copy from before it, a few ms behind.
//!
//! Every queue is a bounded `sync_channel`, made once with all its room.
//! The facade waits only in a lend; this thread waits only while lent, or
//! asleep between polls ([`POLL`]), woken by each ask.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TryRecvError, TrySendError};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use mpfree_seq::loops::{LoopNow, LoopPlan};
use mpfree_seq::{Project, LOOPS, MAX_TRACKS};
use mpfree_time_ui::view::{MuteWait, TimeState};

use super::control::{MuteLight, TimeControl};
use super::StepsData;
use crate::mixing::PADS;

/// Asks the facade's queue holds before one waits in the facade
/// ([`Counts::full`]).
const ASKS: usize = 256;
/// Answers and notes on the way back.
const TOLD: usize = 256;
/// Copies on the way to the facade (it takes them once a period).
const SHOWN: usize = 8;
/// Copies on their way back to be freed.
const FREED: usize = 16;
/// How long the thread sleeps between polls when nothing asks: the audio
/// half's news (a cue that started, notes recorded) waits at most this.
pub const POLL: Duration = Duration::from_millis(1);
/// The heavy copy goes at least this often, for the status line's counts.
const HEAVY_EVERY: Duration = Duration::from_secs(1);
/// Lines and answers the facade keeps for the app between two reads.
const KEPT: usize = 64;

/// How the app says what an ask answered (main.rs `time_said`), chosen by
/// the caller so the words are the same together and apart.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Say {
    /// Into the log: `ok` and the line (an empty line says nothing), or
    /// `err` and why.
    Note { ok: &'static str, err: &'static str },
    /// On the screen in place of the hint, either way.
    Tell,
    /// UNDO or REDO, said on the screen.
    Undo,
    /// Into the log, and on the screen that PLAY waits for this device's
    /// Start.
    Waiting(String),
    /// The line into the log, or `err` and why on the screen (a step
    /// pad's tap).
    NoteOrTell { err: &'static str },
    /// A conversion (Scenes → song, List → song, Song → scenes; layout
    /// spec 4n): the line into the log, and on the screen too when it is
    /// exact (it ends in [`super::EXACT`]) or couldn't be made. One with
    /// differences opens the report card instead.
    Convert,
}

/// What comes back from the thread, besides the copy.
pub enum Told {
    /// An ask's answer, and how to say it.
    Said(Say, Result<String, String>),
    /// A line for the log: what live to song did when the transport
    /// stopped, a take turning Rec song on or off.
    Note(String),
    /// Lines for the log, one each: what the looper's loops did on the
    /// grid ([`TimeControl::loops_are`]).
    Notes(Vec<String>),
}

/// An ask: run on the control half, perhaps with something to tell.
pub type Ask = Box<dyn FnOnce(&mut TimeControl) -> Option<Told> + Send>;

enum ToThread {
    Run(Ask),
    /// Hand the control half to the facade and wait for it.
    Lend,
}

/// The cheap part of the copy: everything the app reads every period.
/// Published whenever it differs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Light {
    pub armed: Option<usize>,
    pub selected: Option<usize>,
    pub also: u128,
    pub loop_cell: Option<(usize, usize)>,
    pub editing: bool,
    pub pads_write: bool,
    pub song_on: bool,
    pub perform: bool,
    pub song_tempo_plays: bool,
    pub tempo_here: Option<f64>,
    pub track_count: usize,
    pub redos: u64,
    pub can_redo: bool,
    pub silenced: u128,
    pub mute_bits: (u128, u128),
    pub loop_plan: LoopPlan,
    pub pad_waiting: [bool; PADS],
    pub muted: [Option<bool>; MAX_TRACKS],
    pub mute_light: [Option<MuteLight>; MAX_TRACKS],
    pub launched: u128,
    pub touch_lost: u64,
    pub dropped: u64,
}

impl Light {
    pub fn of(c: &TimeControl) -> Light {
        Light {
            armed: c.armed(),
            selected: c.selected(),
            also: c.also(),
            loop_cell: c.loop_cell(),
            editing: c.editing(),
            pads_write: c.pads_write(),
            song_on: c.song_on(),
            perform: c.perform(),
            song_tempo_plays: c.song_tempo_plays(),
            tempo_here: c.tempo_here(),
            track_count: c.track_count(),
            redos: c.redos(),
            can_redo: c.can_redo(),
            silenced: c.silenced(),
            mute_bits: c.mute_bits(),
            loop_plan: c.loop_plan(),
            pad_waiting: std::array::from_fn(|p| c.selected().is_some_and(|s| c.pad_waiting(s, p))),
            muted: std::array::from_fn(|t| c.muted(t)),
            mute_light: std::array::from_fn(|t| c.mute_light(t)),
            launched: (0..MAX_TRACKS.min(128))
                .filter(|&t| c.launched(t))
                .fold(0u128, |b, t| b | 1 << t),
            touch_lost: c.touch_lost(),
            dropped: c.dropped(),
        }
    }
}

/// The rest of the copy, which allocates: published after an ask, a lend
/// or a change, and at least once a second.
pub struct Heavy {
    pub project: Project,
    pub steps: StepsData,
    pub state: TimeState,
    pub mute_wait: Option<MuteWait>,
    /// The time engine's part of the status line.
    pub describe: String,
}

impl Heavy {
    pub fn of(c: &TimeControl) -> Heavy {
        Heavy {
            project: c.project.clone(),
            steps: c.steps(),
            state: c.state(),
            mute_wait: c.mute_wait().cloned(),
            describe: c.describe(),
        }
    }
}

/// One publication: the light part always, the heavy part when it is new,
/// and whether the clips or the steps changed since the last one.
struct Shown {
    light: Light,
    heavy: Option<Box<Heavy>>,
    changed: bool,
    steps_changed: bool,
}

/// What the facade's side counted, for the status line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    /// Asks sent to the thread.
    pub asks: u64,
    /// Calls that borrowed the control half back, and the longest wait
    /// for it (the thread finishing what it was doing).
    pub lends: u64,
    pub slowest_lend: Duration,
    /// Asks that found the queue full and waited in the facade.
    pub full: u64,
    /// Copies freed on the audio thread because the queue back was full
    /// (should stay 0).
    pub freed_here: u64,
}

/// The facade's side: the queues, the thread, and the copy it reads.
pub struct Apart {
    to: SyncSender<ToThread>,
    told: Receiver<Told>,
    shown: Receiver<Shown>,
    freed: SyncSender<Box<Heavy>>,
    lent: Receiver<Box<TimeControl>>,
    back: SyncSender<Box<TimeControl>>,
    thread: Option<JoinHandle<Option<Box<TimeControl>>>>,
    stop: Arc<AtomicBool>,
    /// Asks a full queue handed back, sent first next time.
    waiting: VecDeque<ToThread>,
    pub light: Light,
    pub heavy: Box<Heavy>,
    changed: bool,
    steps_changed: bool,
    said: Vec<(Say, Result<String, String>)>,
    notes: VecDeque<String>,
    pub counts: Counts,
    /// What the facade last sent of the inputs it gives every period, so
    /// only a change goes: the app's tempo, whether a take records.
    pub tempo: Option<f64>,
    pub taking: Option<bool>,
}

impl Apart {
    /// The control half `c` onto a thread of its own, with the copy made
    /// here first so the app reads it at once. On failure (no thread), `c`
    /// comes back with why.
    pub fn start(c: Box<TimeControl>) -> Result<Apart, (Box<TimeControl>, String)> {
        let light = Light::of(&c);
        let heavy = Box::new(Heavy::of(&c));
        let (to, from) = sync_channel(ASKS);
        let (tell, told) = sync_channel(TOLD);
        let (show, shown) = sync_channel(SHOWN);
        let (freed, frees) = sync_channel(FREED);
        let (lend, lent) = sync_channel(1);
        let (back, back_rx) = sync_channel(1);
        let stop = Arc::new(AtomicBool::new(false));
        let quit = Arc::clone(&stop);
        // The control half crosses into the thread only once it exists.
        let (give, take) = sync_channel::<Box<TimeControl>>(1);
        let spawned = thread::Builder::new()
            .name("mpfree-time".into())
            .spawn(move || {
                let _ = crate::cores::background();
                let c = take.recv().ok()?;
                Thread {
                    from,
                    tell,
                    show,
                    frees,
                    lend,
                    back: back_rx,
                    stop: quit,
                    unsent: VecDeque::new(),
                    pending: None,
                }
                .run(c)
            });
        let thread = match spawned {
            Ok(t) => t,
            Err(e) => return Err((c, format!("the time engine's thread didn't start: {e}"))),
        };
        if let Err(e) = give.send(c) {
            return Err((e.0, "the time engine's thread stopped at once".into()));
        }
        Ok(Apart {
            to,
            told,
            shown,
            freed,
            lent,
            back,
            thread: Some(thread),
            stop,
            waiting: VecDeque::new(),
            light,
            heavy,
            changed: false,
            steps_changed: false,
            said: Vec::with_capacity(KEPT),
            notes: VecDeque::with_capacity(KEPT),
            counts: Counts::default(),
            tempo: None,
            taking: None,
        })
    }

    fn wake(&self) {
        if let Some(t) = &self.thread {
            t.thread().unpark();
        }
    }

    /// The asks a full queue handed back, in order, until it is full again.
    fn flush(&mut self) {
        while let Some(m) = self.waiting.pop_front() {
            match self.to.try_send(m) {
                Ok(()) => {}
                Err(TrySendError::Full(m)) => {
                    self.waiting.push_front(m);
                    return;
                }
                Err(TrySendError::Disconnected(_)) => {
                    self.waiting.clear();
                    return;
                }
            }
        }
    }

    /// Send `f` to run on the control half, without waiting.
    pub fn ask(&mut self, f: Ask) {
        self.counts.asks += 1;
        self.flush();
        if self.waiting.is_empty() {
            match self.to.try_send(ToThread::Run(f)) {
                Ok(()) | Err(TrySendError::Disconnected(_)) => {}
                Err(TrySendError::Full(m)) => {
                    self.counts.full += 1;
                    self.waiting.push_back(m);
                }
            }
        } else {
            self.counts.full += 1;
            self.waiting.push_back(ToThread::Run(f));
        }
        self.wake();
    }

    /// Run `f` here on the control half, borrowed back from its thread
    /// after the asks before it: the same answer as together. The light
    /// copy and the steps are read again before it goes back, so a read
    /// straight after sees what `f` did.
    pub fn lend<R>(&mut self, f: impl FnOnce(&mut TimeControl) -> R) -> R {
        let at = Instant::now();
        // Everything asked before goes first, in order.
        for m in self.waiting.drain(..) {
            let _ = self.to.send(m);
        }
        if self.to.send(ToThread::Lend).is_err() {
            panic!("the time engine's thread has gone");
        }
        self.wake();
        let mut c = match self.lent.recv() {
            Ok(c) => c,
            Err(_) => panic!("the time engine's thread has gone"),
        };
        let waited = at.elapsed();
        self.counts.lends += 1;
        self.counts.slowest_lend = self.counts.slowest_lend.max(waited);
        // Copies published before the lend come first, so none of them
        // can come after it and show the facade an older state. (The
        // thread publishes nothing while it is lent.)
        self.take_shown();
        let r = f(&mut c);
        self.light = Light::of(&c);
        if c.take_steps_changed() {
            self.heavy.steps = c.steps();
            self.steps_changed = true;
        }
        if self.back.send(c).is_err() {
            panic!("the time engine's thread has gone");
        }
        r
    }

    /// The newest copy, once a period: allocates and frees nothing (the
    /// heavy part it replaces goes back to be freed on the thread).
    pub fn take_shown(&mut self) {
        while let Ok(s) = self.shown.try_recv() {
            self.light = s.light;
            self.changed |= s.changed;
            self.steps_changed |= s.steps_changed;
            if let Some(h) = s.heavy {
                let old = std::mem::replace(&mut self.heavy, h);
                if let Err(TrySendError::Full(old) | TrySendError::Disconnected(old)) =
                    self.freed.try_send(old)
                {
                    self.counts.freed_here += 1;
                    drop(old);
                }
            }
        }
    }

    /// What came back since the last read, into the lists the app takes.
    fn drain_told(&mut self) {
        while self.said.len() < KEPT && self.notes.len() < KEPT {
            match self.told.try_recv() {
                Ok(Told::Said(say, r)) => self.said.push((say, r)),
                Ok(Told::Note(line)) => self.notes.push_back(line),
                Ok(Told::Notes(lines)) => self.notes.extend(lines),
                Err(_) => return,
            }
        }
    }

    /// The asks' answers since the last call, oldest first, with how to say
    /// each.
    pub fn take_said(&mut self) -> Vec<(Say, Result<String, String>)> {
        self.drain_told();
        std::mem::replace(&mut self.said, Vec::with_capacity(KEPT))
    }

    /// A line for the log, oldest first.
    pub fn take_note(&mut self) -> Option<String> {
        self.drain_told();
        self.notes.pop_front()
    }

    pub fn take_changed(&mut self) -> bool {
        std::mem::take(&mut self.changed)
    }

    pub fn take_steps_changed(&mut self) -> bool {
        std::mem::take(&mut self.steps_changed)
    }

    /// The facade's part of the status line: "editing on its own thread:
    /// 120 asks, 14 lends (slowest 0.80 ms)", and anything that waited.
    pub fn describe(&self) -> String {
        let c = &self.counts;
        let mut s = format!(
            "editing on its own thread: {} asks, {} lends (slowest {:.2} ms)",
            c.asks,
            c.lends,
            c.slowest_lend.as_secs_f64() * 1e3
        );
        if c.full > 0 {
            s += &format!(", {} asks waited for room", c.full);
        }
        if c.freed_here > 0 {
            s += &format!(", {} copies freed on the audio thread", c.freed_here);
        }
        s
    }

    /// The control half back from its thread (for a test, or to run both
    /// halves together again): the thread ends after the asks sent so far.
    pub fn stop(mut self) -> Option<Box<TimeControl>> {
        for m in self.waiting.drain(..) {
            let _ = self.to.send(m);
        }
        self.stop.store(true, Ordering::Release);
        self.wake();
        self.thread.take()?.join().ok().flatten()
    }

    /// The loops the looper has, for [`TimeControl::loops_are`]: sent as an
    /// ask, its lines come back as notes.
    pub fn loops_are(&mut self, now: [LoopNow; LOOPS]) {
        self.ask(Box::new(move |c| {
            let lines = c.loops_are(&now);
            (!lines.is_empty()).then_some(Told::Notes(lines))
        }));
    }
}

impl Drop for Apart {
    /// The thread ends on its own; the audio thread never waits for it.
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.wake();
    }
}

/// The thread's side.
struct Thread {
    from: Receiver<ToThread>,
    tell: SyncSender<Told>,
    show: SyncSender<Shown>,
    frees: Receiver<Box<Heavy>>,
    lend: SyncSender<Box<TimeControl>>,
    back: Receiver<Box<TimeControl>>,
    stop: Arc<AtomicBool>,
    /// Answers that found the queue back full, sent first next time.
    unsent: VecDeque<Told>,
    /// A copy that found the queue full, joined into the next.
    pending: Option<Shown>,
}

impl Thread {
    /// Asks, then a poll, then the copy if anything is new; then sleep
    /// until woken or [`POLL`] goes by. Returns the control half when told
    /// to stop, or None if the facade went while it was lent.
    fn run(mut self, mut c: Box<TimeControl>) -> Option<Box<TimeControl>> {
        let mut last: Option<Light> = None;
        let mut heavy_at = Instant::now();
        // The first pass publishes everything.
        let mut asked = true;
        loop {
            while self.frees.try_recv().is_ok() {}
            loop {
                match self.from.try_recv() {
                    Ok(ToThread::Run(f)) => {
                        asked = true;
                        if let Some(t) = f(&mut c) {
                            self.unsent.push_back(t);
                        }
                    }
                    Ok(ToThread::Lend) => {
                        asked = true;
                        if let Err(e) = self.lend.send(c) {
                            return Some(e.0);
                        }
                        c = self.back.recv().ok()?;
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => return Some(c),
                }
            }
            c.poll();
            if let Some(line) = c.take_later_note() {
                self.unsent.push_back(Told::Note(line));
            }
            while let Some(t) = self.unsent.pop_front() {
                match self.tell.try_send(t) {
                    Ok(()) => {}
                    Err(TrySendError::Full(t)) => {
                        self.unsent.push_front(t);
                        break;
                    }
                    Err(TrySendError::Disconnected(_)) => return Some(c),
                }
            }
            let changed = c.take_changed();
            let steps_changed = c.take_steps_changed();
            let light = Light::of(&c);
            let heavy = asked || changed || steps_changed || heavy_at.elapsed() >= HEAVY_EVERY;
            if heavy || last != Some(light) {
                if heavy {
                    heavy_at = Instant::now();
                }
                self.publish(Shown {
                    light,
                    heavy: heavy.then(|| Box::new(Heavy::of(&c))),
                    changed,
                    steps_changed,
                });
                last = Some(light);
            } else if let Some(p) = self.pending.take() {
                self.publish(p);
            }
            asked = false;
            if self.stop.load(Ordering::Acquire) {
                // What was asked before the stop still runs.
                while let Ok(m) = self.from.try_recv() {
                    if let ToThread::Run(f) = m {
                        let _ = f(&mut c);
                    }
                }
                return Some(c);
            }
            thread::park_timeout(POLL);
        }
    }

    /// Send a copy; one that finds the queue full is joined into the next,
    /// so no change is lost.
    fn publish(&mut self, mut s: Shown) {
        if let Some(p) = self.pending.take() {
            s.changed |= p.changed;
            s.steps_changed |= p.steps_changed;
            if s.heavy.is_none() {
                s.heavy = p.heavy;
            }
        }
        if let Err(TrySendError::Full(s)) = self.show.try_send(s) {
            self.pending = Some(s);
        }
    }
}
