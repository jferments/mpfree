//! The control side: work the audio thread hands to other threads so that
//! nothing in its period sleeps, waits on a file or builds an effect.
//!
//! The outside review of 12 Nov found three things on the audio thread
//! that can take longer than a period on their own, outside every engine:
//!
//! - **The controller's lights.** Each message was written to the MIDI port
//!   by the audio thread, which slept 1 ms at a time, for up to half a
//!   second, while the kernel's buffer was full (a kit change sends a light
//!   for every pad). [`Lights`] hands each message to the `mpfree-lights`
//!   thread through a bounded queue; that thread does the writing and the
//!   waiting. A full queue drops the message and counts it.
//! - **Log lines.** Every line was written to the report file by the audio
//!   thread (a system call each, sometimes two). [`Report`] hands each line
//!   to the `mpfree-report` thread, which writes them. A full queue drops
//!   the line and counts it; the count goes in the report.
//! - **Effects added in the chain editor.** Building an effect allocates
//!   everything it will use (the Cloud reverb about 9 MB, every page of it
//!   faulted in and locked under `mlockall`) and was done on the audio
//!   thread. [`Builder`] builds it on the `mpfree-build` thread and hands it
//!   back ready; the effects an edit takes out go there to be dropped too.
//!
//! These threads start from the audio thread, which runs at SCHED_FIFO with
//! SCHED_RESET_ON_FORK (`sys::realtime`), so they start at the normal
//! policy, and each moves itself to the background cores first
//! ([`crate::cores::background`]): with the sound on cores of its own they
//! run on cores 0 and 1, never beside the audio thread or its lanes.
//!
//! Every queue is made before the sound starts, and passing a message
//! never allocates except a long controller message (over [`SHORT`] bytes,
//! which the pad player does not send) and the log line's own text.

use std::io::{self, ErrorKind, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use mpfree_arp::scale::pads::{Pad, PADS};
use mpfree_fx::{Frozen, Insert, Print, Printed, StripId};
use mpfree_node::Tuning;
use mpfree_track::Instrument;

use crate::plugins::Plugins;

/// Controller messages this long or shorter travel without an allocation.
pub const SHORT: usize = 32;

/// Room in the lights' queue: a kit change sends 16 pad colours and the
/// button lights, so this holds several at once.
const LIGHTS_QUEUE: usize = 512;

/// Room in the report's queue, in lines.
const REPORT_QUEUE: usize = 2048;

/// Room in the builder's queues.
const BUILD_QUEUE: usize = 64;

/// How long a write waits for room in the MIDI port's buffer before it
/// gives up on that message.
const MIDI_WAIT: Duration = Duration::from_millis(500);

/// The lights' thread writing nothing this long while messages wait is a
/// stall ([`Lights::watch`]): the session ping waits behind them, and the
/// controller drops the session after about 10 s without one.
pub const STALL: Duration = Duration::from_secs(2);

/// Writes one whole message to a non-blocking MIDI output, waiting up to
/// `wait` for room in the kernel's buffer (1 ms at a time). Never call it on
/// the audio thread: it sleeps.
pub fn send_all(w: &mut impl Write, msg: &[u8], wait: Duration) -> io::Result<()> {
    let mut done = 0;
    let give_up = Instant::now() + wait;
    while done < msg.len() {
        match w.write(&msg[done..]) {
            Ok(0) => {
                return Err(io::Error::new(
                    ErrorKind::WriteZero,
                    "the MIDI port took nothing",
                ))
            }
            Ok(n) => done += n,
            Err(e) if e.kind() == ErrorKind::WouldBlock && Instant::now() < give_up => {
                thread::sleep(Duration::from_millis(1))
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// One controller message in the lights' queue.
enum Msg {
    Short {
        private: bool,
        len: u8,
        bytes: [u8; SHORT],
    },
    Long {
        private: bool,
        bytes: Vec<u8>,
    },
}

impl Msg {
    fn new(private: bool, msg: &[u8]) -> Msg {
        if msg.len() <= SHORT {
            let mut bytes = [0u8; SHORT];
            bytes[..msg.len()].copy_from_slice(msg);
            Msg::Short {
                private,
                len: msg.len() as u8,
                bytes,
            }
        } else {
            Msg::Long {
                private,
                bytes: msg.to_vec(),
            }
        }
    }

    fn parts(&self) -> (bool, &[u8]) {
        match self {
            Msg::Short {
                private,
                len,
                bytes,
            } => (*private, &bytes[..*len as usize]),
            Msg::Long { private, bytes } => (*private, bytes),
        }
    }
}

/// Counts from the lights' thread.
#[derive(Debug, Default)]
struct LightCounts {
    written: AtomicU64,
    failed: AtomicU64,
}

/// What the lights did, for the end of the session.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LightsSummary {
    /// Messages the thread wrote.
    pub written: u64,
    /// Messages the port refused (or had no room for within half a second).
    pub failed: u64,
    /// Messages dropped because the queue was full.
    pub dropped: u64,
    /// Times the thread wrote nothing for [`STALL`] with messages waiting.
    pub stalls: u64,
    /// The thread was still writing when the session ended.
    pub unfinished: bool,
}

/// What [`Lights::watch`] saw.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LightsWatch {
    /// Nothing written for [`STALL`] while `waiting` messages wait.
    Stalled { waiting: u64 },
    /// Writing again, `after` the stall was seen.
    Resumed { after: Duration },
}

/// The controller's outputs, written by the `mpfree-lights` thread.
pub struct Lights {
    tx: Option<SyncSender<Msg>>,
    errors: Receiver<String>,
    counts: Arc<LightCounts>,
    /// Messages put in the queue.
    queued: u64,
    dropped: u64,
    /// The watchdog ([`Lights::watch`]): the thread's count when it last
    /// moved (or the queue was empty), and the stall being seen.
    done: u64,
    moved_at: Instant,
    stalled_at: Option<Instant>,
    stalls: u64,
    thread: Option<JoinHandle<()>>,
}

impl Lights {
    /// Starts the thread on the Public output and, if open, the Private
    /// one. Allocates (call it before the loop).
    pub fn start<W: Write + Send + 'static>(public: W, private: Option<W>) -> io::Result<Lights> {
        Lights::start_with(public, private, MIDI_WAIT)
    }

    fn start_with<W: Write + Send + 'static>(
        mut public: W,
        mut private: Option<W>,
        wait: Duration,
    ) -> io::Result<Lights> {
        let (tx, rx) = mpsc::sync_channel::<Msg>(LIGHTS_QUEUE);
        let (err_tx, errors) = mpsc::sync_channel::<String>(8);
        let counts = Arc::new(LightCounts::default());
        let c = Arc::clone(&counts);
        let thread = thread::Builder::new()
            .name("mpfree-lights".into())
            .spawn(move || {
                let _ = crate::cores::background();
                for m in rx {
                    let (to_private, bytes) = m.parts();
                    let port = if to_private {
                        private.as_mut()
                    } else {
                        Some(&mut public)
                    };
                    let r = match port {
                        Some(w) => send_all(w, bytes, wait),
                        None => Err(io::Error::other("the Private output is not open")),
                    };
                    match r {
                        Ok(()) => {
                            c.written.fetch_add(1, Ordering::Relaxed);
                        }
                        Err(e) => {
                            c.failed.fetch_add(1, Ordering::Relaxed);
                            let name = if to_private { "Private" } else { "Public" };
                            let _ = err_tx.try_send(format!(
                                "controller: sending {} on {name}: {e}",
                                crate::hex(bytes)
                            ));
                        }
                    }
                }
            })?;
        Ok(Lights {
            tx: Some(tx),
            errors,
            counts,
            queued: 0,
            dropped: 0,
            done: 0,
            moved_at: Instant::now(),
            stalled_at: None,
            stalls: 0,
            thread: Some(thread),
        })
    }

    /// Queues `msg` for the Public output, or the Private one. False if the
    /// queue was full: the message is dropped (and counted). Never waits;
    /// allocates only for a message over [`SHORT`] bytes.
    pub fn send(&mut self, private: bool, msg: &[u8]) -> bool {
        let Some(tx) = self.tx.as_ref() else {
            return false;
        };
        match tx.try_send(Msg::new(private, msg)) {
            Ok(()) => {
                self.queued += 1;
                true
            }
            Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {
                self.dropped += 1;
                false
            }
        }
    }

    /// A message the port refused, for the log (at most eight wait); None
    /// most of the time, without allocating.
    pub fn error(&mut self) -> Option<String> {
        self.errors.try_recv().ok()
    }

    /// Messages dropped so far because the queue was full.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// Messages the port refused so far.
    pub fn failed(&self) -> u64 {
        self.counts.failed.load(Ordering::Relaxed)
    }

    /// Stalls seen so far ([`Lights::watch`]).
    pub fn stalls(&self) -> u64 {
        self.stalls
    }

    /// The watchdog, once a period: says when the thread has written
    /// nothing for [`STALL`] while messages wait (once per stall), and when
    /// it writes again. A stall is a fallback made visible: the lights and
    /// the session ping are late, not lost. Never waits; no allocation.
    pub fn watch(&mut self, now: Instant) -> Option<LightsWatch> {
        let done = self.counts.written.load(Ordering::Relaxed)
            + self.counts.failed.load(Ordering::Relaxed);
        let waiting = self.queued.saturating_sub(done);
        if done != self.done || waiting == 0 {
            self.done = done;
            self.moved_at = now;
            return self.stalled_at.take().map(|at| LightsWatch::Resumed {
                after: now.saturating_duration_since(at),
            });
        }
        if self.stalled_at.is_none() && now.saturating_duration_since(self.moved_at) >= STALL {
            self.stalled_at = Some(now);
            self.stalls += 1;
            return Some(LightsWatch::Stalled { waiting });
        }
        None
    }

    /// Lets the thread write what is queued (up to `wait`) and stops it.
    pub fn finish(mut self, wait: Duration) -> LightsSummary {
        self.tx = None;
        let unfinished = !join_within(self.thread.take(), wait);
        LightsSummary {
            written: self.counts.written.load(Ordering::Relaxed),
            failed: self.counts.failed.load(Ordering::Relaxed),
            dropped: self.dropped,
            stalls: self.stalls,
            unfinished,
        }
    }
}

/// Joins `thread` if it ends within `wait`; true if it did (or there was
/// none). A thread that does not end is left to finish on its own.
fn join_within(thread: Option<JoinHandle<()>>, wait: Duration) -> bool {
    let Some(t) = thread else {
        return true;
    };
    let give_up = Instant::now() + wait;
    while !t.is_finished() {
        if Instant::now() >= give_up {
            return false;
        }
        thread::sleep(Duration::from_millis(2));
    }
    t.join().is_ok()
}

/// The report file, written by the `mpfree-report` thread.
pub struct Report<W: Write + Send + 'static> {
    tx: Option<SyncSender<String>>,
    dropped: u64,
    thread: Option<JoinHandle<W>>,
}

impl<W: Write + Send + 'static> Report<W> {
    /// Starts the thread on `w` (allocates; call it before the loop).
    pub fn start(mut w: W) -> io::Result<Report<W>> {
        let (tx, rx) = mpsc::sync_channel::<String>(REPORT_QUEUE);
        let thread = thread::Builder::new()
            .name("mpfree-report".into())
            .spawn(move || {
                let _ = crate::cores::background();
                // Each line as it comes, then whatever else is waiting, then
                // one flush: a burst of lines costs one write.
                let mut buf = io::BufWriter::with_capacity(16 * 1024, &mut w);
                while let Ok(line) = rx.recv() {
                    let _ = writeln!(buf, "{line}");
                    while let Ok(more) = rx.try_recv() {
                        let _ = writeln!(buf, "{more}");
                    }
                    let _ = buf.flush();
                }
                let _ = buf.flush();
                drop(buf);
                w
            })?;
        Ok(Report {
            tx: Some(tx),
            dropped: 0,
            thread: Some(thread),
        })
    }

    /// Queues a line; false (counted) if the queue was full. Never waits.
    pub fn line(&mut self, line: String) -> bool {
        match self.tx.as_ref().map(|tx| tx.try_send(line)) {
            Some(Ok(())) => true,
            _ => {
                self.dropped += 1;
                false
            }
        }
    }

    /// Lines dropped so far because the queue was full.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// Writes what is queued and gives the writer back (None if the thread
    /// did not end within `wait`).
    pub fn finish(mut self, wait: Duration) -> Option<W> {
        self.tx = None;
        let t = self.thread.take()?;
        let give_up = Instant::now() + wait;
        while !t.is_finished() {
            if Instant::now() >= give_up {
                return None;
            }
            thread::sleep(Duration::from_millis(2));
        }
        t.join().ok()
    }
}

/// An effect asked for in the chain editor, built on the builder thread.
pub struct Built {
    pub strip: StripId,
    pub id: &'static str,
    /// None: no effect has that id.
    pub insert: Option<Insert>,
    /// The insert (its slot id) it takes the place of, for a switch of
    /// mode across engines; None: it goes at the end of the chain.
    pub replace: Option<u16>,
}

/// A track's own instrument asked for on the Synth tab, made on the
/// builder thread (`instruments.rs`): the track, its pad map and tuning
/// as they were asked for, and the instrument (None: it couldn't be made;
/// for a plugin the plugins' lines say why).
pub struct MadeInstrument {
    pub track: usize,
    pub id: &'static str,
    pub made: Option<(Instrument, crate::instruments::Held)>,
    pub pads: [Pad; PADS],
    /// Plain data, so dropping this on the audio thread frees nothing.
    pub tuning: Tuning,
}

/// What a track's instrument is made from.
pub struct InstrumentJob {
    pub track: usize,
    pub id: &'static str,
    /// A synth engine's sound (none for a plugin).
    pub values: Vec<f32>,
    pub pads: [Pad; PADS],
    pub tuning: Tuning,
}

/// A plugin synth's own presets, listed on the builder thread, or a
/// preset loaded into a track's plugin (`Rack::presets`,
/// `Rack::load_preset`).
pub enum PresetNews {
    /// Plugin `id`'s presets, or why they couldn't be listed.
    Listed {
        id: &'static str,
        list: Result<Vec<mpfree_plughost::Preset>, String>,
    },
    /// Preset `index` (named `name`) went into track `track`'s plugin
    /// `instance`, or the plugin's reason it didn't.
    Loaded {
        track: usize,
        instance: u64,
        index: usize,
        name: String,
        result: Result<(), String>,
    },
}

/// A preset to load into a track's plugin.
pub struct PresetJob {
    pub track: usize,
    pub instance: u64,
    pub index: usize,
    pub preset: mpfree_plughost::Preset,
}

impl PresetJob {
    /// Loads it (on the builder thread, or here in the tests).
    pub fn run(self, plugins: &Plugins) -> PresetNews {
        PresetNews::Loaded {
            track: self.track,
            instance: self.instance,
            index: self.index,
            result: plugins.load_preset(self.instance, &self.preset),
            name: self.preset.name,
        }
    }
}

enum Job {
    Build {
        strip: StripId,
        id: &'static str,
        /// Parameter values set before it goes in (a factory preset).
        preset: &'static [(u16, f32)],
        rate: u32,
        block: usize,
        /// A switch of mode: what sets it up, and the insert it replaces.
        switch: Option<Box<(mpfree_fx::families::Switch, u16)>>,
    },
    Instrument(Box<InstrumentJob>, u32, usize),
    /// List a plugin's presets (the first time: its own process, seconds).
    Presets(&'static str),
    LoadPreset(Box<PresetJob>),
    Drop(Insert),
    DropInstrument(Instrument),
    /// A take's memory (takes.rs): a print of so many seconds at a rate.
    Print(u32, f32),
    DropFrozen(Frozen),
    DropPrint(Print),
    DropPrinted(Box<Printed>),
}

/// Builds effects for the chain editor, and drops the ones edits take out,
/// on the `mpfree-build` thread.
pub struct Builder {
    tx: SyncSender<Job>,
    built: Receiver<Built>,
    instruments: Receiver<MadeInstrument>,
    prints: Receiver<Print>,
    presets: Receiver<PresetNews>,
    /// Asked for and not back yet.
    pending: usize,
    _thread: JoinHandle<()>,
}

impl Builder {
    /// Starts the thread (allocates; call it before the loop). It also
    /// starts the plugins' processes when one is added, and watches them
    /// every [`mpfree_plughost::rack::WATCH_EVERY`] ([`Plugins::watch`]).
    pub fn start(plugins: Arc<Plugins>) -> io::Result<Builder> {
        let (tx, rx) = mpsc::sync_channel::<Job>(BUILD_QUEUE);
        let (built_tx, built) = mpsc::sync_channel::<Built>(BUILD_QUEUE);
        let (inst_tx, instruments) = mpsc::sync_channel::<MadeInstrument>(BUILD_QUEUE);
        let (print_tx, prints) = mpsc::sync_channel::<Print>(BUILD_QUEUE);
        let (preset_tx, presets) = mpsc::sync_channel::<PresetNews>(BUILD_QUEUE);
        let thread = thread::Builder::new()
            .name("mpfree-build".into())
            .spawn(move || {
                let _ = crate::cores::background();
                let mut watched = Instant::now();
                loop {
                    let job = match rx.recv_timeout(mpfree_plughost::rack::WATCH_EVERY) {
                        Ok(job) => Some(job),
                        Err(RecvTimeoutError::Timeout) => None,
                        Err(RecvTimeoutError::Disconnected) => return,
                    };
                    let now = Instant::now();
                    if now.duration_since(watched) >= mpfree_plughost::rack::WATCH_EVERY {
                        plugins.watch(now);
                        watched = now;
                    }
                    let Some(job) = job else { continue };
                    match job {
                        Job::Build {
                            strip,
                            id,
                            preset,
                            rate,
                            block,
                            switch,
                        } => {
                            let mut node = if Plugins::is_plugin(id) {
                                // Its own process; a refusal is said by
                                // the plugins' lines, and the chain
                                // editor hears "no maker".
                                plugins.make(id).ok()
                            } else {
                                mpfree_fx_ui::make(id)
                            };
                            if let Some(n) = node.as_mut() {
                                for &(p, v) in preset {
                                    n.set_param(p, v);
                                }
                                if let Some(sw) = &switch {
                                    sw.0.apply(n.as_mut());
                                }
                            }
                            let replace = switch.map(|sw| sw.1);
                            let insert = node.map(|node| Insert::new(node, rate, block));
                            // Room for every request (`pending` is checked
                            // against the queue), so this waits only if the
                            // audio thread stopped taking them.
                            let built = Built {
                                strip,
                                id,
                                insert,
                                replace,
                            };
                            if built_tx.send(built).is_err() {
                                return;
                            }
                        }
                        Job::Instrument(job, rate, block) => {
                            let made = make_instrument(*job, &plugins, rate, block);
                            if inst_tx.send(made).is_err() {
                                return;
                            }
                        }
                        Job::Presets(id) => {
                            let news = PresetNews::Listed {
                                id,
                                list: plugins.presets(id),
                            };
                            if preset_tx.send(news).is_err() {
                                return;
                            }
                        }
                        Job::LoadPreset(job) => {
                            if preset_tx.send(job.run(&plugins)).is_err() {
                                return;
                            }
                        }
                        Job::Drop(insert) => drop(insert),
                        Job::DropInstrument(inst) => drop(inst),
                        Job::Print(rate, seconds) => {
                            if print_tx.send(Print::new(rate, seconds)).is_err() {
                                return;
                            }
                        }
                        Job::DropFrozen(f) => drop(f),
                        Job::DropPrint(p) => drop(p),
                        Job::DropPrinted(p) => drop(p),
                    }
                }
            })?;
        Ok(Builder {
            tx,
            built,
            instruments,
            prints,
            presets,
            pending: 0,
            _thread: thread,
        })
    }

    /// [`Builder::build_with`] at the effect's defaults (the tests).
    #[cfg(test)]
    pub fn build(&mut self, strip: StripId, id: &'static str, rate: u32, block: usize) -> bool {
        self.build_with(strip, id, &[], rate, block)
    }

    /// Asks for effect `id` for `strip`, built for `rate` and blocks of
    /// `block` frames, with the values in `preset` set before it goes in
    /// (a factory preset from the picker; empty for its defaults). False
    /// if too many are under way. Never waits.
    pub fn build_with(
        &mut self,
        strip: StripId,
        id: &'static str,
        preset: &'static [(u16, f32)],
        rate: u32,
        block: usize,
    ) -> bool {
        self.send_build(Job::Build {
            strip,
            id,
            preset,
            rate,
            block,
            switch: None,
        })
    }

    /// Asks for engine `id` to take the place of insert `uid` on `strip`
    /// (a switch of mode, `mpfree_fx::families`), set up by `switch`. False
    /// if too many are under way. Never waits.
    pub fn build_switch(
        &mut self,
        strip: StripId,
        uid: u16,
        id: &'static str,
        switch: mpfree_fx::families::Switch,
        rate: u32,
        block: usize,
    ) -> bool {
        self.send_build(Job::Build {
            strip,
            id,
            preset: &[],
            rate,
            block,
            switch: Some(Box::new((switch, uid))),
        })
    }

    fn send_build(&mut self, job: Job) -> bool {
        if self.pending >= BUILD_QUEUE {
            return false;
        }
        if self.tx.try_send(job).is_err() {
            return false;
        }
        self.pending += 1;
        true
    }

    /// Effects asked for and not handed back yet.
    pub fn pending(&self) -> usize {
        self.pending
    }

    /// The next built effect, if one is ready. Never waits.
    pub fn take(&mut self) -> Option<Built> {
        let b = self.built.try_recv().ok()?;
        self.pending -= 1;
        Some(b)
    }

    /// Asks for a track's own instrument, made for `rate` and blocks of
    /// `block` frames and handed back by [`Builder::take_instrument`].
    /// False if the queue is full. Never waits.
    pub fn instrument(&mut self, job: Box<InstrumentJob>, rate: u32, block: usize) -> bool {
        self.tx.try_send(Job::Instrument(job, rate, block)).is_ok()
    }

    /// Asks for plugin `id`'s presets, handed back by
    /// [`Builder::take_preset_news`]. False if the queue is full. Never
    /// waits.
    pub fn presets(&mut self, id: &'static str) -> bool {
        self.tx.try_send(Job::Presets(id)).is_ok()
    }

    /// Asks for a preset to be loaded into a track's plugin. False if the
    /// queue is full. Never waits.
    pub fn load_preset(&mut self, job: Box<PresetJob>) -> bool {
        self.tx.try_send(Job::LoadPreset(job)).is_ok()
    }

    /// The next presets listed or preset loaded, if one is ready. Never
    /// waits.
    pub fn take_preset_news(&mut self) -> Option<PresetNews> {
        self.presets.try_recv().ok()
    }

    /// The next track instrument made, if one is ready. Never waits.
    pub fn take_instrument(&mut self) -> Option<MadeInstrument> {
        self.instruments.try_recv().ok()
    }

    /// Drops an instrument the track host retired, on the builder thread.
    /// False if the queue was full: then it was dropped here.
    pub fn drop_instrument_later(&mut self, inst: Instrument) -> bool {
        match self.tx.try_send(Job::DropInstrument(inst)) {
            Ok(()) => true,
            Err(TrySendError::Full(job) | TrySendError::Disconnected(job)) => {
                drop(job);
                false
            }
        }
    }

    /// Asks for a take's memory: a print of `seconds` at `rate`, handed
    /// back by [`Builder::take_print`]. False if the queue is full. Never
    /// waits.
    pub fn print(&mut self, rate: u32, seconds: f32) -> bool {
        self.tx.try_send(Job::Print(rate, seconds)).is_ok()
    }

    /// The next take's memory made, if one is ready. Never waits.
    pub fn take_print(&mut self) -> Option<Print> {
        self.prints.try_recv().ok()
    }

    /// Drops a take a strip is done with on the builder thread (here if
    /// the queue is full).
    pub fn drop_frozen_later(&mut self, f: Frozen) {
        if let Err(TrySendError::Full(job) | TrySendError::Disconnected(job)) =
            self.tx.try_send(Job::DropFrozen(f))
        {
            drop(job);
        }
    }

    /// Drops a print's memory on the builder thread (here if the queue is
    /// full).
    pub fn drop_print_later(&mut self, p: Print) {
        if let Err(TrySendError::Full(job) | TrySendError::Disconnected(job)) =
            self.tx.try_send(Job::DropPrint(p))
        {
            drop(job);
        }
    }

    /// Drops a print that wasn't saved on the builder thread (here if the
    /// queue is full).
    pub fn drop_printed_later(&mut self, p: Box<Printed>) {
        if let Err(TrySendError::Full(job) | TrySendError::Disconnected(job)) =
            self.tx.try_send(Job::DropPrinted(p))
        {
            drop(job);
        }
    }

    /// Drops `insert` on the builder thread. False if its queue was full
    /// (or the thread gone): then the insert was dropped here, on the
    /// calling thread, and the caller says so.
    pub fn drop_later(&mut self, insert: Insert) -> bool {
        match self.tx.try_send(Job::Drop(insert)) {
            Ok(()) => true,
            Err(TrySendError::Full(job) | TrySendError::Disconnected(job)) => {
                drop(job);
                false
            }
        }
    }
}

/// Makes a track's instrument (`instruments::make`): on the builder
/// thread, or here when there is none (the tests).
pub fn make_instrument(
    job: InstrumentJob,
    plugins: &Plugins,
    rate: u32,
    block: usize,
) -> MadeInstrument {
    MadeInstrument {
        track: job.track,
        id: job.id,
        made: crate::instruments::make(plugins, job.id, &job.values, rate, block),
        pads: job.pads,
        tuning: job.tuning,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use std::sync::Mutex;

    /// A port that keeps what it is given, refusing the first `busy` writes
    /// with WouldBlock (a full kernel buffer). While `held`, a write waits
    /// (a port the kernel holds up).
    #[derive(Clone, Default)]
    struct Port {
        got: Arc<Mutex<Vec<u8>>>,
        busy: Arc<Mutex<u32>>,
        held: Arc<AtomicBool>,
    }

    impl Write for Port {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            while self.held.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(1));
            }
            let mut busy = self.busy.lock().unwrap();
            if *busy > 0 {
                *busy -= 1;
                return Err(ErrorKind::WouldBlock.into());
            }
            self.got.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn lights_are_written_in_order_by_their_own_thread() {
        let public = Port::default();
        let private = Port::default();
        *public.busy.lock().unwrap() = 5;
        // The port is held up until the sends are done, then busy five
        // times. After two seconds it lets go on its own, so a send that
        // waited for it fails the test rather than hanging it.
        public.held.store(true, Ordering::Relaxed);
        let held = public.held.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_secs(2));
            held.store(false, Ordering::Relaxed);
        });
        let mut l = Lights::start(public.clone(), Some(private.clone())).unwrap();
        let t = Instant::now();
        for k in 0..20u8 {
            assert!(l.send(false, &[0xF0, k, 0xF7]));
        }
        assert!(l.send(true, &[0x90, 1, 2]));
        let long: Vec<u8> = (0..40).collect();
        assert!(l.send(false, &long));
        // The sends never waited for the port: a wait takes the two
        // seconds. (A second leaves room for a slow runner: CI runs these
        // as 32-bit ARM under qemu.)
        assert!(t.elapsed() < Duration::from_secs(1), "{:?}", t.elapsed());
        public.held.store(false, Ordering::Relaxed);
        let s = l.finish(Duration::from_secs(5));
        assert_eq!(
            s,
            LightsSummary {
                written: 22,
                failed: 0,
                dropped: 0,
                stalls: 0,
                unfinished: false
            }
        );
        let mut want: Vec<u8> = (0..20u8).flat_map(|k| [0xF0, k, 0xF7]).collect();
        want.extend_from_slice(&long);
        assert_eq!(*public.got.lock().unwrap(), want);
        assert_eq!(*private.got.lock().unwrap(), vec![0x90, 1, 2]);
    }

    #[test]
    fn a_port_that_stays_full_is_counted_and_said() {
        let public = Port::default();
        *public.busy.lock().unwrap() = u32::MAX;
        let mut l = Lights::start_with(public, None, Duration::from_millis(5)).unwrap();
        assert!(l.send(false, &[0xB0, 1, 2]));
        // No Private output: refused by the thread, and said.
        assert!(l.send(true, &[0xB0, 3, 4]));
        let give_up = Instant::now() + Duration::from_secs(5);
        let mut said = Vec::new();
        while said.len() < 2 && Instant::now() < give_up {
            if let Some(e) = l.error() {
                said.push(e);
            }
            thread::sleep(Duration::from_millis(1));
        }
        assert!(
            said[0].starts_with("controller: sending B0 01 02 on Public: "),
            "{said:?}"
        );
        assert!(
            said[1].contains("the Private output is not open"),
            "{said:?}"
        );
        assert_eq!(l.finish(Duration::from_secs(5)).failed, 2);
    }

    #[test]
    fn a_stalled_lights_thread_is_seen_once_and_so_is_its_return() {
        let public = Port::default();
        *public.busy.lock().unwrap() = u32::MAX;
        let busy = Arc::clone(&public.busy);
        // The thread waits up to 30 s on the first message.
        let mut l = Lights::start_with(public, None, Duration::from_secs(30)).unwrap();
        let t0 = Instant::now();
        assert_eq!(l.watch(t0), None, "nothing waiting");
        for k in 0..3 {
            assert!(l.send(false, &[0x90, k, 1]));
        }
        assert_eq!(l.watch(t0 + Duration::from_secs(1)), None);
        assert_eq!(
            l.watch(t0 + STALL),
            Some(LightsWatch::Stalled { waiting: 3 })
        );
        assert_eq!(l.watch(t0 + 2 * STALL), None, "said once");
        assert_eq!(l.stalls(), 1);
        // The port takes messages again.
        *busy.lock().unwrap() = 0;
        let give_up = Instant::now() + Duration::from_secs(5);
        let mut seen = None;
        while seen.is_none() && Instant::now() < give_up {
            seen = l.watch(t0 + 3 * STALL);
            thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(seen, Some(LightsWatch::Resumed { after: 2 * STALL }));
        let s = l.finish(Duration::from_secs(5));
        assert_eq!((s.written, s.stalls), (3, 1));
    }

    #[test]
    fn a_full_lights_queue_drops_and_counts() {
        let public = Port::default();
        *public.busy.lock().unwrap() = u32::MAX;
        // The thread holds the first message for up to a second.
        let mut l = Lights::start_with(public, None, Duration::from_secs(1)).unwrap();
        let mut refused = 0;
        for k in 0..(LIGHTS_QUEUE + 10) {
            if !l.send(false, &[0x90, k as u8, 1]) {
                refused += 1;
            }
        }
        assert!(refused >= 9, "{refused}");
        assert_eq!(l.dropped(), refused);
    }

    #[test]
    fn report_lines_are_written_by_their_own_thread_and_the_file_comes_back() {
        let mut r = Report::start(Vec::<u8>::new()).unwrap();
        for k in 0..100 {
            assert!(r.line(format!("line {k}")));
        }
        assert_eq!(r.dropped(), 0);
        let w = r.finish(Duration::from_secs(5)).expect("the writer back");
        let text = String::from_utf8(w).unwrap();
        assert_eq!(text.lines().count(), 100);
        assert!(text.starts_with("line 0\nline 1\n"));
        assert!(text.ends_with("line 99\n"));
    }

    #[test]
    fn effects_are_built_and_dropped_on_the_builder_thread() {
        let mut b = Builder::start(Arc::new(Plugins::none())).unwrap();
        assert!(b.build(StripId::Track(1), "mpfree.fx.drive", 44_100, 128));
        assert!(b.build(StripId::Track(2), "no.such.effect", 44_100, 128));
        assert_eq!(b.pending(), 2);
        let give_up = Instant::now() + Duration::from_secs(5);
        let mut got = Vec::new();
        while got.len() < 2 && Instant::now() < give_up {
            if let Some(x) = b.take() {
                got.push(x);
            }
            thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(b.pending(), 0);
        assert_eq!(got[0].strip, StripId::Track(1));
        assert_eq!(got[0].id, "mpfree.fx.drive");
        let insert = got[0].insert.take().expect("a drive");
        assert!(got[1].insert.is_none(), "no such effect");
        assert!(b.drop_later(insert), "dropped on the builder thread");
    }
}
