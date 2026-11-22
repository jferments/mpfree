//! The coordinator: knows the sources and their files, plans reads from
//! where the sound thread says each cursor plays, and fills the sound
//! thread's cache. Runs on its own thread (or is stepped by a test);
//! nothing here is on the sound thread.
//!
//! - **Planning.** Each cursor playing a file source wants the units from
//!   where it plays to a window ahead (the window's seconds at the output
//!   rate, times its speed, round its loop once, up to its end). Units not
//!   cached or asked for are asked for earliest deadline first, up to
//!   [`Plan::in_flight`] reads at once. A cursor not heard of for two of
//!   the sound thread's rounds has stopped.
//! - **Joined reads.** A read takes the next units of the same file with
//!   it, up to [`Plan::join_bytes`], while the time it would take (from
//!   the device's measured speed) stays within half the time left before
//!   the next other deadline.
//! - **Cancelling.** A read asked for but no longer wanted (its cursor
//!   moved on, its source closed or replaced) is cancelled; if the device
//!   hasn't started it, it is skipped.
//! - **Holds.** A part of a source can be held in the cache whether or
//!   not anything plays it: a start, a loop point, a cued launch, fetched
//!   by a deadline or in the background. A group of holds succeeds whole
//!   or not at all, inside the budget. Holds have a [`HoldTier`]: when
//!   room runs short (what plays, at the least safe window, and capture
//!   memory come first), the lowest tier is given back first, newest
//!   first, and said so ([`Coordinator::given_back`]).
//! - **Jobs.** Whole files read for something other than playing (a kit's
//!   sounds, a preset, the next song's files) go through the same device,
//!   in steps of at most [`Plan::join_bytes`], one step at a time and only
//!   once every playback read wanted is asked for, best [`JobTier`]
//!   first.
//! - **Room and admission.** The budget is the sound thread's cache. What
//!   plays (at the wanted window), what is held and what recordings
//!   reserve must fit; if it doesn't, the window planned with shrinks and
//!   [`Admission`] says plainly that the workload isn't covered. A unit
//!   goes in a free slot, else in the slot wanted longest ago and not in
//!   the last round; with none, the read waits (counted).
//! - **Generations.** Every request carries its [`Tag`]; a reply for a
//!   source closed or replaced meanwhile is dropped (counted). The sound
//!   thread checks again on its side.
//! - **Faults.** A read that fails or comes back short is counted, and the
//!   units it didn't bring whole are asked for again when next wanted;
//!   nothing short is ever sent to be played.
//! - **Memory sources** are handed to the sound thread whole and never
//!   read from disk; they take no room in the cache.

use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::File;
use std::io;
use std::os::unix::fs::FileExt;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use mpfree_flac::Decoder;

use crate::flac::Index;
use crate::format::{decode, layout, Layout};
use crate::ledger::{Ledger, Pool};
use crate::msg::{FromPlayer, Tag, ToPlayer, Where, MAX_SOURCES};
use crate::player::{unit_frames, Config, UNIT};
use crate::resample::Rate;
use crate::storage::{Card, Located};
use crate::writer::Gate;

/// Past this much of a recording's capture memory, background reads stop.
pub const STOP_BACKGROUND: f64 = 0.75;

/// How far ahead to read, and how reads are made.
#[derive(Clone, Copy, Debug)]
pub struct Plan {
    /// The output's sample rate: a cursor's speed is file frames per
    /// output frame, so a window in seconds needs it.
    pub rate: u32,
    /// Seconds of each cursor wanted ahead of where it plays.
    pub window_s: f64,
    /// The least window a workload may play with and still be safe.
    pub min_window_s: f64,
    /// Reads asked for and not answered, at most.
    pub in_flight: usize,
    /// The most bytes one joined read takes.
    pub join_bytes: usize,
    /// The longest the device has been seen to stall on one read, in
    /// seconds (the stick test measures it). A whole-file read step starts
    /// only while every playing cursor has at least this long, or the
    /// longest stall seen lately, before it needs a unit not yet in hand.
    pub stall_s: f64,
    /// The stall comes from measuring this stick (the stick test, D1), not
    /// from a default. Until it does, admission calls a workload untested
    /// ([`crate::policy::Cover::Untested`]).
    pub measured: bool,
}

impl Default for Plan {
    fn default() -> Plan {
        Plan {
            rate: 44_100,
            window_s: 8.0,
            min_window_s: 4.0,
            in_flight: 8,
            join_bytes: 512 * 1024,
            stall_s: 0.5,
            measured: false,
        }
    }
}

impl Plan {
    /// The plan's window rule for a device whose worst shortfall measured
    /// `worst_s` seconds: twice that and one more, never under 4 s.
    pub fn for_shortfall(worst_s: f64) -> Plan {
        let measured = worst_s.is_finite() && worst_s > 0.0;
        let w = if measured { 2.0 * worst_s + 1.0 } else { 0.0 };
        Plan {
            window_s: w.max(4.0),
            measured,
            ..Plan::default()
        }
    }
}

/// A read of part of a file, answered with the bytes.
pub struct ReadReq {
    pub request: u32,
    pub file: Arc<File>,
    pub at: u64,
    pub len: usize,
    pub buf: Vec<u8>,
    /// Set once the read is no longer wanted: skip it if not started.
    pub cancel: Arc<AtomicBool>,
}

pub struct ReadReply {
    pub request: u32,
    pub buf: Vec<u8>,
    /// Bytes read, or the error.
    pub got: io::Result<usize>,
    /// How long the read took.
    pub took: Duration,
    /// Skipped: cancelled before it started.
    pub cancelled: bool,
}

/// What does the reading: the I/O thread of a device, or a test's stand-in.
pub trait Device: Send {
    fn submit(&mut self, r: ReadReq);
    fn poll(&mut self) -> Option<ReadReply>;
}

/// Read `len` bytes at `at`, or as many as the file has.
pub fn read_at(file: &File, at: u64, buf: &mut [u8]) -> io::Result<usize> {
    let mut got = 0;
    while got < buf.len() {
        match file.read_at(&mut buf[got..], at + got as u64) {
            Ok(0) => break,
            Ok(k) => got += k,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(got)
}

/// Do one read (or skip it, if cancelled).
pub fn serve(mut r: ReadReq) -> ReadReply {
    if r.cancel.load(Ordering::Relaxed) {
        return ReadReply {
            request: r.request,
            buf: r.buf,
            got: Ok(0),
            took: Duration::ZERO,
            cancelled: true,
        };
    }
    let start = Instant::now();
    r.buf.resize(r.len, 0);
    let got = read_at(&r.file, r.at, &mut r.buf);
    ReadReply {
        request: r.request,
        buf: r.buf,
        got,
        took: start.elapsed(),
        cancelled: false,
    }
}

/// Reads at once, in the caller's thread (tests, and tools).
#[derive(Default)]
pub struct Inline {
    done: VecDeque<ReadReply>,
}

impl Device for Inline {
    fn submit(&mut self, r: ReadReq) {
        self.done.push_back(serve(r));
    }
    fn poll(&mut self) -> Option<ReadReply> {
        self.done.pop_front()
    }
}

/// An I/O thread for one device: it only reads.
pub struct IoThread {
    tx: std::sync::mpsc::Sender<ReadReq>,
    rx: Receiver<ReadReply>,
}

impl IoThread {
    pub fn start(name: &str) -> io::Result<IoThread> {
        let (tx, jobs) = std::sync::mpsc::channel::<ReadReq>();
        let (done, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name(name.to_string())
            .spawn(move || {
                for r in jobs {
                    if done.send(serve(r)).is_err() {
                        break;
                    }
                }
            })?;
        Ok(IoThread { tx, rx })
    }
}

impl Device for IoThread {
    fn submit(&mut self, r: ReadReq) {
        let _ = self.tx.send(r);
    }
    fn poll(&mut self) -> Option<ReadReply> {
        self.rx.try_recv().ok()
    }
}

/// Counters for the screen and the tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Reads asked for, and units that went along in a joined read.
    pub reads: u64,
    pub joined: u64,
    pub read_errors: u32,
    /// Units a read brought back short.
    pub short_reads: u32,
    /// Replies for a source closed or replaced meanwhile.
    pub stale: u32,
    /// Reads skipped because they were no longer wanted.
    pub cancelled: u32,
    /// Fills the sound thread refused.
    pub refused: u32,
    /// Times a read waited because no slot could be given up.
    pub no_room: u32,
    /// Units read again while already in the cache, and not filled again.
    pub duplicates: u32,
    /// Reads and units dropped because the card they were for went.
    pub card_gone: u32,
    /// FLAC units that couldn't be decoded: the index was stale, or the
    /// file is damaged there. Their source stops ([`Coordinator::unplayable`]).
    pub flac_failed: u32,
}

/// Whether what plays, what is held and what is reserved fits the cache.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Admission {
    /// Cache units in all.
    pub budget: usize,
    /// Units reserved for recordings.
    pub reserved: usize,
    /// Units held.
    pub held: usize,
    /// Units the playing cursors want at the wanted window.
    pub playing: usize,
    /// The window planned with: the wanted one, or less if that doesn't
    /// fit.
    pub window_s: f64,
    /// Everything fits at the wanted window.
    pub covered: bool,
    /// At least the least safe window fits.
    pub safe: bool,
}

/// Names a hold, to release it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct HoldId(u32);

/// What a hold is for, most important first: when room runs short the
/// last is given back first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HoldTier {
    /// "Keep in memory": a sound the musician pinned.
    Pinned,
    /// A cued launch's starts, until it has started.
    Cued,
    /// Starts and loop points of the song playing.
    Head,
    /// The next song's starts, inside its own budget.
    Next,
}

/// Names a whole-file read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct JobId(u32);

/// How soon a whole-file read is wanted, most first; every playback read
/// comes before any of them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum JobTier {
    /// Preparing the next song (kit sounds, presets, files).
    NextSong,
    /// Anything that can wait.
    Background,
}

/// Why a group of holds was refused (none of it is held).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The source isn't an open file source.
    NotOpen(u16),
    /// The hold is no longer held (given back, released, or its source
    /// closed).
    Gone(HoldId),
    /// The group needs `need` more units with `room` left.
    OverBudget { need: usize, room: usize },
}

enum Store {
    File {
        file: Arc<File>,
        layout: Layout,
    },
    /// A FLAC file: its frames found through its index and decoded on the
    /// coordinator's thread, never on the sound thread.
    Flac {
        file: Arc<File>,
        index: Arc<Index>,
    },
    /// Lent by the engine that holds it: its bytes, for the ledger.
    Memory {
        bytes: u64,
    },
}

struct Src {
    tag: Tag,
    store: Store,
    /// A file played at the engine's rate: its units are engine frames,
    /// made from the file's frames around them.
    rate: Option<Rate>,
    /// The card it is on: read only while that insertion holds.
    card: Option<Card>,
    /// Its card went and that was reported ([`Coordinator::lost`]).
    reported: bool,
    /// Why it can't be read any further, once it can't (a stale FLAC
    /// index, a damaged FLAC file).
    unplayable: Option<String>,
    /// Where each cursor playing it last played.
    cursors: Vec<Where>,
}

/// The frames a file source plays in: the file's own, or the engine's
/// for one played at the engine's rate. Units, holds, readiness and
/// cursors all count in these, never in the other.
#[derive(Clone, Copy)]
struct Shape {
    channels: u16,
    frames: u64,
}

impl Src {
    /// A WAV, RF64 or AIFF source's layout.
    fn layout(&self) -> Option<&Layout> {
        match &self.store {
            Store::File { layout, .. } => Some(layout),
            Store::Flac { .. } | Store::Memory { .. } => None,
        }
    }

    /// The file it reads, for a file source.
    fn file(&self) -> Option<&Arc<File>> {
        match &self.store {
            Store::File { file, .. } | Store::Flac { file, .. } => Some(file),
            Store::Memory { .. } => None,
        }
    }

    /// The file's own channels and frames, for a file source.
    fn native(&self) -> Option<(u16, u64)> {
        match &self.store {
            Store::File { layout, .. } => Some((layout.channels, layout.frames)),
            Store::Flac { index, .. } => Some((u16::from(index.info.channels), index.frames)),
            Store::Memory { .. } => None,
        }
    }

    /// The bytes (`start..end`) holding the file's frames `a..b`.
    fn bytes_of(&self, a: u64, b: u64) -> Option<(u64, u64)> {
        match &self.store {
            Store::File { layout, .. } => Some((layout.byte_of(a), layout.byte_of(b))),
            Store::Flac { index, .. } => {
                let (start, end, _) = index.span(a, b);
                Some((start, end))
            }
            Store::Memory { .. } => None,
        }
    }

    /// Its card went: nothing more is read for it.
    fn lost(&self) -> bool {
        self.card.as_ref().is_some_and(|c| !c.valid())
    }

    /// Nothing more is read for it: its card went, or it can't be read.
    fn stopped(&self) -> bool {
        self.lost() || self.unplayable.is_some()
    }

    fn shape(&self) -> Option<Shape> {
        let (channels, frames) = self.native()?;
        Some(Shape {
            channels,
            frames: self.rate.map_or(frames, |r| r.frames()),
        })
    }

    /// The file frames (`start..end`) unit `unit` is made from.
    fn span(&self, unit: u32) -> Option<(u64, u64)> {
        let sh = self.shape()?;
        let uf = unit_frames(sh.channels);
        let first = u64::from(unit) * uf;
        let n = sh.frames.saturating_sub(first).min(uf);
        if n == 0 {
            return None;
        }
        Some(match self.rate {
            Some(r) => r.span(first, n),
            None => (first, first + n),
        })
    }
}

struct Hold {
    id: u32,
    /// Holds made (or promoted) together are given back together.
    group: u32,
    tag: Tag,
    from: u64,
    to: u64,
    by: Option<Instant>,
    tier: HoldTier,
}

struct Job {
    id: u32,
    tier: JobTier,
    /// The card it reads from, if any.
    card: Option<Card>,
    file: Arc<File>,
    len: u64,
    data: Vec<u8>,
    busy: bool,
}

/// FLAC decoding's speed before any is measured, frames a second: about
/// ten times real time at 44.1 kHz, a cautious guess for the MPC.
const FLAC_FPS: f64 = 441_000.0;

/// Why a read from the card failed when the card went.
const CARD_GONE: &str = "the SD card was removed";

/// What a read in flight is for.
enum Kind {
    Units {
        tag: Tag,
        first: u32,
        units: u32,
    },
    /// Units read for a length that has since changed ([`Coordinator::grow`]):
    /// whatever comes back is dropped.
    Void,
    Job(u32),
}

/// A read in flight.
struct Flight {
    kind: Kind,
    cancel: Arc<AtomicBool>,
    bytes: usize,
    /// Seconds decoding what it brings will take (FLAC), counted in its
    /// deadline with the read.
    decode_s: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Held {
    Free,
    Unit { source: u16, unit: u32 },
}

#[derive(Clone, Copy, Debug)]
struct MSlot {
    held: Held,
    /// The round that last wanted it.
    wanted: u64,
}

/// What a round wants: each unit's earliest deadline in seconds.
type Wants = HashMap<(u16, u32), (f64, Tag)>;

pub struct Coordinator {
    plan: Plan,
    rx: Receiver<FromPlayer>,
    tx: SyncSender<ToPlayer>,
    dev: Box<dyn Device>,
    project: u32,
    next_gen: u32,
    sources: Vec<Option<Src>>,
    mirror: Vec<MSlot>,
    map: HashMap<(u16, u32), usize>,
    /// Units in flight: the generation that asked and the request.
    asked: HashMap<(u16, u32), (u32, u32)>,
    flights: HashMap<u32, Flight>,
    holds: Vec<Hold>,
    next_hold: u32,
    given_back: VecDeque<HoldId>,
    jobs: Vec<Job>,
    next_job: u32,
    finished: HashMap<u32, Result<Vec<u8>, String>>,
    reserve: usize,
    pool: Vec<Vec<f32>>,
    /// Buffers made at start (in the pool or out with a fill).
    buffers: usize,
    bytes: Vec<Vec<u8>>,
    /// A resampled unit's file frames, decoded (it grows to the largest
    /// unit read).
    scratch: Vec<f32>,
    /// Units read whole, waiting for a buffer or a slot.
    ready: VecDeque<(Tag, u32, Vec<u8>)>,
    /// Messages waiting for room on the way to the sound thread.
    out: VecDeque<ToPlayer>,
    round: u64,
    /// The newest of the sound thread's rounds heard of.
    heard: u32,
    request: u32,
    /// The device's measured speed, bytes a second.
    speed: f64,
    /// How long the last reads took, newest last (at most 64).
    took: VecDeque<f64>,
    /// Seconds until a playing cursor needs a unit not in hand, as of the
    /// last round.
    cushion_s: f64,
    /// The cushion, shared with the recording writer's thread.
    gate: Arc<Gate>,
    admission: Admission,
    /// Sources whose card went, not yet asked for.
    lost: VecDeque<Tag>,
    /// Sources that can't be read any further, and why, not yet asked for.
    unplayable: VecDeque<(Tag, String)>,
    /// FLAC decoding's measured speed, frames a second.
    flac_fps: f64,
    stats: Stats,
}

impl Coordinator {
    /// The coordinator for a player made with `cfg`, given the channels'
    /// far ends from [`crate::player::Player::new`].
    pub fn new(
        cfg: Config,
        plan: Plan,
        rx: Receiver<FromPlayer>,
        tx: SyncSender<ToPlayer>,
        dev: Box<dyn Device>,
    ) -> Coordinator {
        let mut sources = Vec::with_capacity(MAX_SOURCES);
        sources.resize_with(MAX_SOURCES, || None);
        Coordinator {
            plan,
            rx,
            tx,
            dev,
            project: 0,
            next_gen: 1,
            sources,
            mirror: vec![
                MSlot {
                    held: Held::Free,
                    wanted: 0,
                };
                cfg.slots
            ],
            map: HashMap::new(),
            asked: HashMap::new(),
            flights: HashMap::new(),
            holds: Vec::new(),
            next_hold: 1,
            given_back: VecDeque::new(),
            jobs: Vec::new(),
            next_job: 1,
            finished: HashMap::new(),
            reserve: 0,
            pool: (0..cfg.buffers).map(|_| vec![0.0; UNIT]).collect(),
            buffers: cfg.buffers,
            bytes: Vec::new(),
            scratch: Vec::new(),
            ready: VecDeque::new(),
            out: VecDeque::new(),
            round: 1,
            heard: 0,
            request: 0,
            // A slow stick, until measured.
            speed: 2.0e6,
            took: VecDeque::with_capacity(64),
            cushion_s: f64::INFINITY,
            gate: Arc::new(Gate::new()),
            admission: Admission::default(),
            lost: VecDeque::new(),
            unplayable: VecDeque::new(),
            flac_fps: FLAC_FPS,
            stats: Stats::default(),
        }
    }

    pub fn stats(&self) -> Stats {
        self.stats
    }

    pub fn plan(&self) -> Plan {
        self.plan
    }

    /// Units the mirror says are in the sound thread's cache (or on their
    /// way there).
    pub fn cached(&self) -> usize {
        self.map.len()
    }

    /// Reads in flight.
    pub fn in_flight(&self) -> usize {
        self.flights.len()
    }

    /// The device's measured speed, bytes a second.
    pub fn device_speed(&self) -> f64 {
        self.speed
    }

    /// Whether the workload fits, as of the last planning round.
    pub fn admission(&self) -> Admission {
        self.admission
    }

    /// The memory streaming holds, for the shared memory check and the
    /// screen ([`crate::ledger`]).
    pub fn ledger(&self) -> Ledger {
        const UNIT_BYTES: u64 = (UNIT * std::mem::size_of::<f32>()) as u64;
        let units = |n: usize| n as u64 * UNIT_BYTES;
        let allocated = units(self.mirror.len());
        let in_use = units(self.mirror.iter().filter(|m| m.held != Held::Free).count());
        // Every tier: Next is the last.
        let held = self.held_units(HoldTier::Next);
        let idle = self.bytes.iter().map(Vec::capacity).sum::<usize>();
        let raw = idle
            + self.ready.iter().map(|(.., b)| b.capacity()).sum::<usize>()
            + self.flights.values().map(|f| f.bytes).sum::<usize>();
        let floats = units(self.buffers) + (self.scratch.capacity() * 4) as u64;
        let jobs = self.jobs.iter().map(|j| j.data.capacity()).sum::<usize>()
            + self
                .finished
                .values()
                .map(|r| r.as_ref().map_or(0, Vec::capacity))
                .sum::<usize>();
        let lent = self
            .sources
            .iter()
            .flatten()
            .map(|s| match s.store {
                Store::Memory { bytes } => bytes,
                Store::File { .. } | Store::Flac { .. } => 0,
            })
            .sum();
        Ledger {
            cache: Pool {
                allocated,
                in_use,
                reserved: units(self.reserve + held.len()).min(allocated),
                reclaimable: 0,
            },
            buffers: Pool {
                allocated: floats + raw as u64,
                in_use: units(self.buffers.saturating_sub(self.pool.len())) + (raw - idle) as u64,
                reserved: 0,
                reclaimable: 0,
            },
            jobs: jobs as u64,
            lent,
        }
    }

    /// Seconds until a playing cursor needs a unit that isn't in the cache
    /// yet, as of the last round (the window when all it wants is there;
    /// infinite when nothing plays): the cushion playback has against a
    /// stall.
    pub fn cushion(&self) -> f64 {
        self.cushion_s
    }

    /// The cushion, shared: the recording writer's syncs wait while it is
    /// shorter than the stick's worst commit time.
    pub fn gate(&self) -> Arc<Gate> {
        Arc::clone(&self.gate)
    }

    /// The stall whole-file reads leave room for: the plan's, or the
    /// longest of the last 64 reads if longer.
    pub fn stall(&self) -> f64 {
        self.took.iter().copied().fold(self.plan.stall_s, f64::max)
    }

    /// Cache units kept for recordings (their capture memory). Holds that
    /// no longer fit are given back.
    pub fn set_reserve(&mut self, units: usize) {
        self.reserve = units;
        self.fit_holds();
    }

    fn send(&mut self, m: ToPlayer) {
        self.out.push_back(m);
        self.flush();
    }

    fn flush(&mut self) {
        while let Some(m) = self.out.pop_front() {
            match self.tx.try_send(m) {
                Ok(()) => {}
                Err(TrySendError::Full(m)) => {
                    self.out.push_front(m);
                    break;
                }
                Err(TrySendError::Disconnected(_)) => {
                    self.out.clear();
                    break;
                }
            }
        }
    }

    /// Forget what the coordinator knows of `source`'s audio: its cached
    /// units, reads, holds and units waiting.
    fn release(&mut self, source: u16) {
        let gone: Vec<usize> = self
            .map
            .iter()
            .filter(|((s, _), _)| *s == source)
            .map(|(_, &slot)| slot)
            .collect();
        for slot in gone {
            self.mirror[slot].held = Held::Free;
        }
        self.map.retain(|(s, _), _| *s != source);
        self.asked.retain(|(s, _), _| *s != source);
        for f in self.flights.values() {
            if matches!(f.kind, Kind::Units { tag, .. } if tag.source == source) {
                f.cancel.store(true, Ordering::Relaxed);
            }
        }
        self.holds.retain(|h| h.tag.source != source);
        self.ready.retain(|(t, ..)| t.source != source);
    }

    /// A new project: every source closed, the cache emptied.
    pub fn new_project(&mut self) -> u32 {
        self.project += 1;
        for s in &mut self.sources {
            *s = None;
        }
        for m in &mut self.mirror {
            m.held = Held::Free;
        }
        self.map.clear();
        self.asked.clear();
        for f in self.flights.values() {
            if matches!(f.kind, Kind::Units { .. }) {
                f.cancel.store(true, Ordering::Relaxed);
            }
        }
        self.holds.clear();
        self.ready.clear();
        let p = self.project;
        self.send(ToPlayer::Project(p));
        p
    }

    fn check_source(source: u16) -> Result<usize, String> {
        let s = usize::from(source);
        if s >= MAX_SOURCES {
            return Err(format!("source {source}: past the last"));
        }
        Ok(s)
    }

    fn next_tag(&mut self, source: u16) -> Tag {
        if self.sources[usize::from(source)].is_some() {
            self.release(source);
        }
        let tag = Tag {
            project: self.project,
            source,
            gen: self.next_gen,
        };
        self.next_gen += 1;
        tag
    }

    /// Make `source` the sound in the file at `path` (replacing what it
    /// was, under a new generation).
    pub fn open(&mut self, source: u16, path: &Path) -> Result<Tag, String> {
        self.open_as(source, path, false, None, None, None)
    }

    /// Make `source` the WAV or RF64 file at `path` that is still being
    /// written (a take's `.recording` file), playing only its first
    /// `frames` frames: what its writer has committed, never what the file
    /// happens to hold now (a write in progress, or room set aside). Never
    /// more than the file holds. It grows with [`Coordinator::grow`]. The
    /// file may be renamed while it is open (the open file follows it),
    /// but never shortened or rewritten below what it has committed.
    pub fn open_growing(&mut self, source: u16, path: &Path, frames: u64) -> Result<Tag, String> {
        self.open_as(source, path, false, None, None, Some(frames))
    }

    /// The source opened with [`Coordinator::open_growing`] now has
    /// `frames` frames (its writer's committed count; only ever up), under
    /// the same generation: units wholly below the old end stay cached, and
    /// the one cut short at the old end is dropped (in the cache, and a
    /// read of it in flight) and read again whole. Refused for less than
    /// it has, for more than the file holds, and for a FLAC, memory or
    /// engine-rate source.
    pub fn grow(&mut self, source: u16, frames: u64) -> Result<(), String> {
        let s = Self::check_source(source)?;
        let src = self.sources[s]
            .as_mut()
            .ok_or_else(|| format!("source {source} isn't open"))?;
        if src.rate.is_some() {
            return Err("a source played at the engine's rate can't grow".into());
        }
        let Store::File { file, layout } = &mut src.store else {
            return Err("only a WAV or RF64 file being written can grow".into());
        };
        let old = layout.frames;
        if frames < old {
            return Err(format!("{frames} frames: it has {old}, and only grows"));
        }
        if frames == old {
            return Ok(());
        }
        let len = file.metadata().map_err(|e| e.to_string())?.len();
        if layout.byte_of(frames) > len {
            return Err(format!("{frames} frames: the file holds fewer"));
        }
        layout.frames = frames;
        let (tag, uf) = (src.tag, unit_frames(layout.channels));
        if !old.is_multiple_of(uf) {
            // The unit cut short at the old end: dropped from the cache's
            // books, from what waits to be filled, and from reads in
            // flight, so it is asked for again whole.
            let short = (old / uf) as u32;
            if let Some(slot) = self.map.remove(&(source, short)) {
                self.mirror[slot].held = Held::Free;
            }
            for (t, u, b) in std::mem::take(&mut self.ready) {
                if t == tag && u == short {
                    self.bytes.push(b);
                } else {
                    self.ready.push_back((t, u, b));
                }
            }
            for (&request, f) in self.flights.iter_mut() {
                let Kind::Units {
                    tag: t,
                    first,
                    units,
                } = f.kind
                else {
                    continue;
                };
                if t == tag && (first..first + units).contains(&short) {
                    f.kind = Kind::Void;
                    f.cancel.store(true, Ordering::Relaxed);
                    for u in first..first + units {
                        if self.asked.get(&(source, u)) == Some(&(tag.gen, request)) {
                            self.asked.remove(&(source, u));
                        }
                    }
                }
            }
        }
        self.send(ToPlayer::Grow { tag, frames });
        Ok(())
    }

    /// Make `source` the FLAC file at `path`, found through `index`
    /// ([`crate::flac::Index`]: the file's own seek table, checked, or one
    /// built for it), at the engine's rate if `engine`. Its frames are
    /// decoded on the coordinator's thread into the same units as any
    /// other file. An index that doesn't fit the file (its size and change
    /// time) is refused: build it again. If a frame turns out not to be
    /// where the index says, or is damaged, the source stops being read
    /// and says so ([`Coordinator::unplayable`]); what is in the cache plays
    /// out, and nothing wrong is ever played.
    pub fn open_flac(
        &mut self,
        source: u16,
        path: &Path,
        index: Arc<Index>,
        engine: bool,
    ) -> Result<Tag, String> {
        self.open_as(source, path, engine, None, Some(index), None)
    }

    /// [`Coordinator::open_flac`] for a file the storage manager found: one
    /// on the card is read only while its insertion holds.
    pub fn open_located_flac(
        &mut self,
        source: u16,
        loc: &Located,
        index: Arc<Index>,
        engine: bool,
    ) -> Result<Tag, String> {
        if !loc.valid() {
            return Err(format!("{}: the SD card isn't there", loc.path.display()));
        }
        self.open_as(
            source,
            &loc.path,
            engine,
            loc.card.clone(),
            Some(index),
            None,
        )
    }

    /// Open `source` from a file the storage manager found
    /// ([`crate::storage::Storage::locate`]), at the engine's rate if
    /// `engine`. A file on the card is read only while its insertion
    /// holds: once the card goes, nothing more is read for it, reads
    /// already running are dropped, and what is in the cache plays out
    /// ([`Coordinator::lost`]).
    pub fn open_located(
        &mut self,
        source: u16,
        loc: &Located,
        engine: bool,
    ) -> Result<Tag, String> {
        if !loc.valid() {
            return Err(format!("{}: the SD card isn't there", loc.path.display()));
        }
        self.open_as(source, &loc.path, engine, loc.card.clone(), None, None)
    }

    /// Open `source` from a file played at the engine's rate
    /// ([`Plan::rate`]): a stem or a clip. A file at another rate is
    /// resampled ahead into engine-rate units ([`crate::resample`]), so
    /// its frames, holds, loop points and cursors count in engine frames
    /// ([`Coordinator::frames`]); one already at the engine's rate opens
    /// as [`Coordinator::open`] does.
    pub fn open_at_engine_rate(&mut self, source: u16, path: &Path) -> Result<Tag, String> {
        self.open_as(source, path, true, None, None, None)
    }

    fn open_as(
        &mut self,
        source: u16,
        path: &Path,
        engine: bool,
        card: Option<Card>,
        flac: Option<Arc<Index>>,
        growing: Option<u64>,
    ) -> Result<Tag, String> {
        let s = Self::check_source(source)?;
        let say = |e: &dyn std::fmt::Display| format!("{}: {e}", path.display());
        let mut file = File::open(path).map_err(|e| say(&e))?;
        let (store, file_rate, frames) = match flac {
            Some(index) => {
                let now = crate::snapshot::meta(path).map_err(|e| say(&e))?;
                if !index.fits(now) {
                    return Err(say(&"its FLAC index is out of date: build it again"));
                }
                let (rate, frames) = (index.info.rate, index.frames);
                let file = Arc::new(file);
                (Store::Flac { file, index }, rate, frames)
            }
            None => {
                let mut layout = layout(&mut file).map_err(|e| say(&e))?;
                if let Some(committed) = growing {
                    layout.frames = layout.frames.min(committed);
                }
                let file = Arc::new(file);
                (Store::File { file, layout }, layout.rate, layout.frames)
            }
        };
        let rate = if engine {
            Rate::new(file_rate, self.plan.rate, frames)
        } else {
            None
        };
        let tag = self.next_tag(source);
        let src = Src {
            tag,
            store,
            rate,
            card,
            reported: false,
            unplayable: None,
            cursors: Vec::new(),
        };
        let Some(sh) = src.shape() else {
            unreachable!("a file source has a shape");
        };
        self.sources[s] = Some(src);
        self.send(ToPlayer::Open {
            tag,
            channels: sh.channels,
            frames: sh.frames,
        });
        Ok(tag)
    }

    /// Make `source` a sound held whole in memory (interleaved, 1 or 2
    /// channels), played through the same cursors as a file.
    pub fn open_memory(
        &mut self,
        source: u16,
        channels: u16,
        data: Arc<[f32]>,
    ) -> Result<Tag, String> {
        let s = Self::check_source(source)?;
        if !(1..=2).contains(&channels) {
            return Err(format!("{channels} channels: only 1 or 2 play"));
        }
        if !data.len().is_multiple_of(usize::from(channels)) {
            return Err("a memory sound must hold whole frames".into());
        }
        let tag = self.next_tag(source);
        self.sources[s] = Some(Src {
            tag,
            store: Store::Memory {
                bytes: (data.len() * std::mem::size_of::<f32>()) as u64,
            },
            rate: None,
            card: None,
            reported: false,
            unplayable: None,
            cursors: Vec::new(),
        });
        self.send(ToPlayer::Resident {
            tag,
            channels,
            data,
        });
        Ok(tag)
    }

    /// Unload `source`.
    pub fn close(&mut self, source: u16) {
        let Some(src) = self
            .sources
            .get_mut(usize::from(source))
            .and_then(Option::take)
        else {
            return;
        };
        self.release(source);
        self.send(ToPlayer::Close { tag: src.tag });
    }

    /// A file source's layout: the file's own frames and rate.
    pub fn layout(&self, source: u16) -> Option<Layout> {
        self.sources
            .get(usize::from(source))?
            .as_ref()?
            .layout()
            .copied()
    }

    /// The frames a file source plays: engine frames for one opened with
    /// [`Coordinator::open_at_engine_rate`] at another rate, else the
    /// file's.
    pub fn frames(&self, source: u16) -> Option<u64> {
        Some(
            self.sources
                .get(usize::from(source))?
                .as_ref()?
                .shape()?
                .frames,
        )
    }

    fn current(&self, tag: Tag) -> bool {
        tag.project == self.project
            && self
                .sources
                .get(usize::from(tag.source))
                .and_then(Option::as_ref)
                .is_some_and(|s| s.tag == tag)
    }

    fn units_of(l: Shape, from: u64, to: u64) -> std::ops::Range<u32> {
        let uf = unit_frames(l.channels);
        let to = to.min(l.frames);
        if from >= to {
            return 0..0;
        }
        (from / uf) as u32..to.div_ceil(uf) as u32
    }

    // ---- holds and readiness ---------------------------------------------

    /// Hold frames `from..to` of each part's source in the cache, by `by`
    /// from now (or in the background), until released or given back: all
    /// of them, or none if any source isn't an open file or they don't fit
    /// beside what plays (at the least safe window), what is reserved and
    /// what is held at the same tier or above. Holds below it that no
    /// longer fit are given back. A memory source is whole in memory
    /// already: holding it is refused as not a file.
    pub fn hold_all(
        &mut self,
        parts: &[(u16, u64, u64)],
        by: Option<Duration>,
        tier: HoldTier,
    ) -> Result<Vec<HoldId>, Refusal> {
        let mut new: HashSet<(u16, u32)> = HashSet::new();
        let mut tags = Vec::with_capacity(parts.len());
        for &(source, from, to) in parts {
            let src = self
                .sources
                .get(usize::from(source))
                .and_then(Option::as_ref)
                .ok_or(Refusal::NotOpen(source))?;
            let l = src.shape().ok_or(Refusal::NotOpen(source))?;
            for u in Self::units_of(l, from, to) {
                new.insert((source, u));
            }
            tags.push(src.tag);
        }
        let mut have = self.held_units(tier);
        have.extend(self.wants(self.plan.min_window_s).into_keys());
        let before = have.len() + self.reserve;
        have.extend(new);
        let need = have.len() + self.reserve - before;
        let budget = self.mirror.len();
        if before + need > budget {
            return Err(Refusal::OverBudget {
                need,
                room: budget.saturating_sub(before),
            });
        }
        let by = by.map(|d| Instant::now() + d);
        let mut ids = Vec::with_capacity(parts.len());
        let group = self.next_hold;
        for (&(_, from, to), tag) in parts.iter().zip(tags) {
            let id = self.next_hold;
            self.next_hold += 1;
            self.holds.push(Hold {
                id,
                group,
                tag,
                from,
                to,
                by,
                tier,
            });
            ids.push(HoldId(id));
        }
        self.fit_holds();
        Ok(ids)
    }

    /// Give back holds, lowest tier and newest first, until what plays at
    /// the least safe window, what is reserved and what is held fit.
    fn fit_holds(&mut self) {
        let budget = self.mirror.len();
        let playing: HashSet<(u16, u32)> = self.wants(self.plan.min_window_s).into_keys().collect();
        loop {
            let mut have = self.held_units(HoldTier::Next);
            have.extend(playing.iter().copied());
            if have.len() + self.reserve <= budget || self.holds.is_empty() {
                return;
            }
            let worst = (0..self.holds.len())
                .max_by_key(|&i| (self.holds[i].tier, self.holds[i].id))
                .expect("not empty");
            let group = self.holds[worst].group;
            let mut i = 0;
            while i < self.holds.len() {
                if self.holds[i].group == group {
                    let h = self.holds.remove(i);
                    self.given_back.push_back(HoldId(h.id));
                } else {
                    i += 1;
                }
            }
        }
    }

    /// The next source whose card went since last asked, each once, by
    /// the tag it had: nothing more is read for it, and what is in the
    /// cache plays out. Closing or replacing it is its owner's choice.
    pub fn lost(&mut self) -> Option<Tag> {
        self.lost.pop_front()
    }

    /// The next source that can't be read any further since last asked,
    /// each once, with why (a FLAC file whose index is stale, or that is
    /// damaged): what is in the cache plays out. A stale index is built
    /// again and the source opened again; that is its owner's step.
    pub fn unplayable(&mut self) -> Option<(Tag, String)> {
        self.unplayable.pop_front()
    }

    /// FLAC decoding's measured speed, in frames a second (a cautious
    /// guess until a FLAC unit has been decoded).
    pub fn flac_speed(&self) -> f64 {
        self.flac_fps
    }

    /// The next hold given back for room since last asked (a "ready"
    /// answer that counted on it no longer stands).
    pub fn given_back(&mut self) -> Option<HoldId> {
        self.given_back.pop_front()
    }

    /// [`Coordinator::hold_all`] with one part.
    pub fn hold(
        &mut self,
        source: u16,
        from: u64,
        to: u64,
        by: Option<Duration>,
        tier: HoldTier,
    ) -> Result<HoldId, Refusal> {
        self.hold_all(&[(source, from, to)], by, tier).map(|v| v[0])
    }

    /// Move holds to another tier, all or none: they must all still be
    /// held, and fit beside what plays (at the least safe window), what is
    /// reserved and what is held at the new tier or above. On refusal
    /// nothing changes. Moved holds become a group of their own (given
    /// back together); holds below the new tier that no longer fit are
    /// given back. A song switch moves its next song's holds from Next to
    /// Cued once Go is accepted, then to Head once it plays.
    pub fn promote(&mut self, ids: &[HoldId], tier: HoldTier) -> Result<(), Refusal> {
        let mut units: HashSet<(u16, u32)> = HashSet::new();
        for id in ids {
            let h = self
                .holds
                .iter()
                .find(|h| h.id == id.0)
                .ok_or(Refusal::Gone(*id))?;
            let l = self.sources[usize::from(h.tag.source)]
                .as_ref()
                .and_then(Src::shape)
                .ok_or(Refusal::Gone(*id))?;
            for u in Self::units_of(l, h.from, h.to) {
                units.insert((h.tag.source, u));
            }
        }
        let mut have = self.held_units(tier);
        have.extend(self.wants(self.plan.min_window_s).into_keys());
        let before = have.len() + self.reserve;
        have.extend(units);
        let need = have.len() + self.reserve - before;
        let budget = self.mirror.len();
        if before + need > budget {
            return Err(Refusal::OverBudget {
                need,
                room: budget.saturating_sub(before),
            });
        }
        let group = self.next_hold;
        self.next_hold += 1;
        for h in self
            .holds
            .iter_mut()
            .filter(|h| ids.contains(&HoldId(h.id)))
        {
            h.tier = tier;
            h.group = group;
        }
        self.fit_holds();
        Ok(())
    }

    /// The tier a hold is held at, if it still is.
    pub fn hold_tier(&self, id: HoldId) -> Option<HoldTier> {
        self.holds.iter().find(|h| h.id == id.0).map(|h| h.tier)
    }

    /// Let a hold go (its units may be given up when room is needed).
    pub fn release_hold(&mut self, id: HoldId) {
        self.holds.retain(|h| h.id != id.0);
    }

    /// Whether a hold is still held (a close, a replace or a new project
    /// lets it go).
    pub fn is_held(&self, id: HoldId) -> bool {
        self.holds.iter().any(|h| h.id == id.0)
    }

    /// Units held at `tier` or above.
    fn held_units(&self, tier: HoldTier) -> HashSet<(u16, u32)> {
        let mut out = HashSet::new();
        for h in self.holds.iter().filter(|h| h.tier <= tier) {
            let Some(l) = self.sources[usize::from(h.tag.source)]
                .as_ref()
                .and_then(Src::shape)
            else {
                continue;
            };
            for u in Self::units_of(l, h.from, h.to) {
                out.insert((h.tag.source, u));
            }
        }
        out
    }

    /// Whether frames `from..to` of `source` are in the cache (or on their
    /// way there: the sound thread's own answer, at a boundary, is
    /// [`crate::player::Player::covers`]). A memory source always is.
    pub fn is_ready(&self, source: u16, from: u64, to: u64) -> bool {
        let Some(src) = self
            .sources
            .get(usize::from(source))
            .and_then(Option::as_ref)
        else {
            return false;
        };
        match src.shape() {
            None => true,
            Some(l) => Self::units_of(l, from, to).all(|u| self.map.contains_key(&(source, u))),
        }
    }

    /// The sources of a launch whose part isn't ready (empty: the launch
    /// can go).
    pub fn not_ready(&self, parts: &[(u16, u64, u64)]) -> Vec<u16> {
        let mut out: Vec<u16> = parts
            .iter()
            .filter(|&&(s, from, to)| !self.is_ready(s, from, to))
            .map(|&(s, ..)| s)
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    // ---- the round -------------------------------------------------------

    /// One round of work: take the sound thread's messages and the
    /// device's replies, plan, ask for reads, send fills. True if
    /// anything happened.
    pub fn step(&mut self) -> bool {
        let mut busy = false;
        self.flush();
        while let Ok(m) = self.rx.try_recv() {
            busy = true;
            match m {
                FromPlayer::Back(b) => self.pool.push(b),
                // A memory sound back: freed here, off the sound thread.
                FromPlayer::Resident(d) => drop(d),
                FromPlayer::Round(r) => {
                    if r.wrapping_sub(self.heard) < u32::MAX / 2 {
                        self.heard = r;
                    }
                }
                FromPlayer::Where(w) => {
                    if w.round.wrapping_sub(self.heard) < u32::MAX / 2 {
                        self.heard = w.round;
                    }
                    if self.current(w.tag) {
                        let s = usize::from(w.tag.source);
                        if let Some(src) = self.sources[s].as_mut() {
                            match src.cursors.iter_mut().find(|c| c.cursor == w.cursor) {
                                Some(c) => *c = w,
                                None => src.cursors.push(w),
                            }
                        }
                    }
                }
                FromPlayer::Refused {
                    tag, unit, slot, ..
                } => {
                    self.stats.refused += 1;
                    let slot = slot as usize;
                    if self.current(tag) && self.map.get(&(tag.source, unit)) == Some(&slot) {
                        self.map.remove(&(tag.source, unit));
                        self.mirror[slot].held = Held::Free;
                    }
                }
            }
        }
        while let Some(r) = self.dev.poll() {
            busy = true;
            self.reply(r);
        }
        self.note_lost();
        busy |= self.fill();
        busy |= self.ask();
        busy
    }

    /// Report each source whose card went, once; its reads in flight are
    /// cancelled (a reply that comes anyway is dropped).
    fn note_lost(&mut self) {
        for src in self.sources.iter_mut().flatten() {
            if !src.reported && src.lost() {
                src.reported = true;
                self.lost.push_back(src.tag);
                for f in self.flights.values() {
                    if matches!(f.kind, Kind::Units { tag, .. } if tag == src.tag) {
                        f.cancel.store(true, Ordering::Relaxed);
                    }
                }
            }
        }
    }

    /// `tag`'s source can't be read any further: say so once, cancel its
    /// reads in flight and read nothing more for it.
    fn stop_source(&mut self, tag: Tag, why: String) {
        let Some(src) = self.sources[usize::from(tag.source)]
            .as_mut()
            .filter(|s| s.tag == tag && s.unplayable.is_none())
        else {
            return;
        };
        src.unplayable = Some(why.clone());
        self.unplayable.push_back((tag, why));
        for f in self.flights.values() {
            if matches!(f.kind, Kind::Units { tag: t, .. } if t == tag) {
                f.cancel.store(true, Ordering::Relaxed);
            }
        }
    }

    fn reply(&mut self, r: ReadReply) {
        let Some(fl) = self.flights.remove(&r.request) else {
            self.bytes.push(r.buf);
            return;
        };
        let (tag, first, units) = match fl.kind {
            Kind::Units { tag, first, units } => (tag, first, units),
            Kind::Job(id) => return self.job_reply(id, r),
            Kind::Void => {
                self.stats.stale += 1;
                self.bytes.push(r.buf);
                return;
            }
        };
        let s = tag.source;
        for u in first..first + units {
            if self.asked.get(&(s, u)) == Some(&(tag.gen, r.request)) {
                self.asked.remove(&(s, u));
            }
        }
        if r.cancelled {
            self.stats.cancelled += 1;
            self.bytes.push(r.buf);
            return;
        }
        if !self.current(tag) {
            self.stats.stale += 1;
            self.bytes.push(r.buf);
            return;
        }
        if let Some(src) = self.sources[usize::from(s)]
            .as_ref()
            .filter(|s| s.stopped())
        {
            // Read for a card that went, or a source that can't be read
            // any more: whatever came back isn't used.
            if src.lost() {
                self.stats.card_gone += 1;
            }
            self.bytes.push(r.buf);
            return;
        }
        let n = match r.got {
            Ok(n) => n,
            Err(_) => {
                self.stats.read_errors += 1;
                self.bytes.push(r.buf);
                return;
            }
        };
        let secs = r.took.as_secs_f64();
        if self.took.len() == 64 {
            self.took.pop_front();
        }
        self.took.push_back(secs);
        if n >= 16 * 1024 && secs > 0.0 {
            let sample = (n as f64 / secs).clamp(1.0e5, 1.0e10);
            self.speed = 0.8 * self.speed + 0.2 * sample;
        }
        if units == 1 {
            if n == fl.bytes {
                self.ready.push_back((tag, first, r.buf));
            } else {
                self.stats.short_reads += 1;
                self.bytes.push(r.buf);
            }
            return;
        }
        // A joined read: each unit it brought whole goes on.
        let src = self.sources[usize::from(s)].as_ref().expect("current");
        let Some((start, _)) = src.span(first).and_then(|(a, _)| src.bytes_of(a, a)) else {
            self.bytes.push(r.buf);
            return;
        };
        // Each unit's own bytes (a FLAC unit's run from the index point
        // before it, so neighbours can share some).
        let pieces: Vec<Option<(usize, usize)>> = (first..first + units)
            .map(|u| {
                let (a, b) = src.span(u)?;
                let (s, e) = src.bytes_of(a, b)?;
                Some(((s - start) as usize, (e - s) as usize))
            })
            .collect();
        for (u, piece) in (first..).zip(pieces) {
            match piece {
                Some((at, want)) if at + want <= n => {
                    let mut b = self.bytes.pop().unwrap_or_default();
                    b.clear();
                    b.extend_from_slice(&r.buf[at..at + want]);
                    self.ready.push_back((tag, u, b));
                }
                _ => self.stats.short_reads += 1,
            }
        }
        self.bytes.push(r.buf);
    }

    fn job_reply(&mut self, id: u32, r: ReadReply) {
        let Some(i) = self.jobs.iter().position(|j| j.id == id) else {
            // Cancelled meanwhile.
            self.bytes.push(r.buf);
            return;
        };
        let job = &mut self.jobs[i];
        job.busy = false;
        if job.card.as_ref().is_some_and(|c| !c.valid()) {
            let j = self.jobs.remove(i);
            self.stats.card_gone += 1;
            self.finished.insert(j.id, Err(CARD_GONE.into()));
            self.bytes.push(r.buf);
            return;
        }
        let done = match r.got {
            _ if r.cancelled => false,
            Err(e) => {
                let j = self.jobs.remove(i);
                self.finished.insert(j.id, Err(e.to_string()));
                self.bytes.push(r.buf);
                return;
            }
            Ok(n) => {
                job.data.extend_from_slice(&r.buf[..n]);
                // A short step: the file ended early (it shrank).
                n < r.buf.len() || job.data.len() as u64 >= job.len
            }
        };
        self.bytes.push(r.buf);
        if done {
            let j = self.jobs.remove(i);
            self.finished.insert(j.id, Ok(j.data));
        }
    }

    // ---- whole-file reads -----------------------------------------------

    /// Read the whole file at `path` through the device, behind every
    /// playback read; [`Coordinator::take_job`] gives it once done.
    pub fn read_file(&mut self, path: &Path, tier: JobTier) -> Result<JobId, String> {
        self.read_file_from(path, tier, None)
    }

    /// [`Coordinator::read_file`] for a file the storage manager found: one
    /// on the card fails as soon as the card goes.
    pub fn read_located(&mut self, loc: &Located, tier: JobTier) -> Result<JobId, String> {
        if !loc.valid() {
            return Err(format!("{}: the SD card isn't there", loc.path.display()));
        }
        self.read_file_from(&loc.path, tier, loc.card.clone())
    }

    fn read_file_from(
        &mut self,
        path: &Path,
        tier: JobTier,
        card: Option<Card>,
    ) -> Result<JobId, String> {
        let file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let len = file
            .metadata()
            .map_err(|e| format!("{}: {e}", path.display()))?
            .len();
        let id = self.next_job;
        self.next_job += 1;
        if len == 0 {
            self.finished.insert(id, Ok(Vec::new()));
        } else {
            self.jobs.push(Job {
                id,
                tier,
                card,
                file: Arc::new(file),
                len,
                data: Vec::new(),
                busy: false,
            });
        }
        Ok(JobId(id))
    }

    /// A finished whole-file read: the bytes, or why it failed. `None`
    /// while it is still reading (or if it was cancelled or taken).
    pub fn take_job(&mut self, id: JobId) -> Option<Result<Vec<u8>, String>> {
        self.finished.remove(&id.0)
    }

    /// Stop a whole-file read (a step in flight is let go when it lands).
    pub fn cancel_job(&mut self, id: JobId) {
        self.jobs.retain(|j| j.id != id.0);
        self.finished.remove(&id.0);
    }

    /// Whole-file reads not finished.
    pub fn jobs_waiting(&self) -> usize {
        self.jobs.len()
    }

    /// One step of the most wanted job, if none is in flight.
    fn job_step(&mut self) -> bool {
        if self.jobs.iter().any(|j| j.busy) {
            return false;
        }
        // Jobs on a card that went fail now, not at their next step.
        let gone: Vec<u32> = self
            .jobs
            .iter()
            .filter(|j| j.card.as_ref().is_some_and(|c| !c.valid()))
            .map(|j| j.id)
            .collect();
        for id in gone {
            self.jobs.retain(|j| j.id != id);
            self.finished.insert(id, Err(CARD_GONE.into()));
        }
        // Recording pressure: past three quarters of capture memory,
        // background work stops.
        let pressed = self.gate.capture_fill() >= STOP_BACKGROUND;
        let Some(i) = (0..self.jobs.len())
            .filter(|&i| !pressed || self.jobs[i].tier == JobTier::NextSong)
            .min_by_key(|&i| (self.jobs[i].tier, self.jobs[i].id))
        else {
            return false;
        };
        let step = self.plan.join_bytes.max(64 * 1024) as u64;
        let job = &mut self.jobs[i];
        let at = job.data.len() as u64;
        let len = (job.len - at).min(step) as usize;
        job.busy = true;
        let (id, file) = (job.id, job.file.clone());
        self.request = self.request.wrapping_add(1);
        let request = self.request;
        let cancel = Arc::new(AtomicBool::new(false));
        self.flights.insert(
            request,
            Flight {
                kind: Kind::Job(id),
                cancel: cancel.clone(),
                bytes: len,
                decode_s: 0.0,
            },
        );
        self.dev.submit(ReadReq {
            request,
            file,
            at,
            len,
            buf: self.bytes.pop().unwrap_or_default(),
            cancel,
        });
        true
    }

    /// The file bytes a read of units `first..first + units` of `source`
    /// takes: from where the first one's frames start to where the last
    /// one's end (a resampled source's units share the frames at their
    /// edges, read once). 0 if the last one is past the end.
    fn read_bytes(&self, source: u16, first: u32, units: u32) -> usize {
        let Some(src) = self.sources[usize::from(source)].as_ref() else {
            return 0;
        };
        let (Some(a), Some(b)) = (src.span(first), src.span(first + units.saturating_sub(1)))
        else {
            return 0;
        };
        src.bytes_of(a.0, b.1).map_or(0, |(s, e)| (e - s) as usize)
    }

    /// Seconds decoding units `first..first + units` of `source` takes: 0
    /// but for FLAC, whose frames decode from the index point before them.
    fn decode_time(&self, source: u16, first: u32, units: u32) -> f64 {
        let Some(src) = self.sources[usize::from(source)].as_ref() else {
            return 0.0;
        };
        let Store::Flac { index, .. } = &src.store else {
            return 0.0;
        };
        let (Some(a), Some(b)) = (src.span(first), src.span(first + units.saturating_sub(1)))
        else {
            return 0.0;
        };
        (b.1 - index.point(a.0).sample) as f64 / self.flac_fps.max(1.0)
    }

    /// A slot for a new unit: a free one, else the one wanted longest ago
    /// and not in the last round.
    fn slot(&mut self) -> Option<usize> {
        let mut best: Option<(usize, u64)> = None;
        for (i, m) in self.mirror.iter().enumerate() {
            match m.held {
                Held::Free => return Some(i),
                Held::Unit { .. } if m.wanted + 1 < self.round => {
                    if best.is_none_or(|b| m.wanted < b.1) {
                        best = Some((i, m.wanted));
                    }
                }
                Held::Unit { .. } => {}
            }
        }
        best.map(|b| b.0)
    }

    fn fill(&mut self) -> bool {
        let mut busy = false;
        while !self.ready.is_empty() && !self.pool.is_empty() {
            let Some(slot) = self.slot() else {
                self.stats.no_room += 1;
                break;
            };
            let (tag, unit, bytes) = self.ready.pop_front().expect("checked");
            if let Some(src) = self.sources[usize::from(tag.source)]
                .as_ref()
                .filter(|s| s.stopped())
            {
                if src.lost() {
                    self.stats.card_gone += 1;
                }
                self.bytes.push(bytes);
                continue;
            }
            if self.map.contains_key(&(tag.source, unit)) {
                // Already in the cache (a read that crossed one already
                // filled): filling it again would take the playing copy
                // away while the new one is copied in.
                self.stats.duplicates += 1;
                self.bytes.push(bytes);
                continue;
            }
            let src = self.sources[usize::from(tag.source)]
                .as_ref()
                .expect("a current file source");
            let (Some((a, b)), Some(sh)) = (src.span(unit), src.shape()) else {
                self.bytes.push(bytes);
                continue;
            };
            let ch = usize::from(sh.channels);
            let first = u64::from(unit) * unit_frames(sh.channels);
            let mut buf = self.pool.pop().expect("checked");
            // The file's frames a..b as floats: into the unit itself, or
            // first into scratch for a source made at the engine's rate.
            let out: &mut [f32] = if src.rate.is_some() {
                let samples = (b - a) as usize * ch;
                if self.scratch.len() < samples {
                    self.scratch.resize(samples, 0.0);
                }
                &mut self.scratch[..samples]
            } else {
                &mut buf[..]
            };
            let decoded = match &src.store {
                Store::File { layout: l, .. } => Ok(decode(l.sample, l.endian, &bytes, out)),
                Store::Flac { index, .. } => {
                    let started = Instant::now();
                    let (_, _, from) = index.span(a, b);
                    let mut dec = Decoder::new(index.info);
                    let r = index.decode(&mut dec, &bytes, from, a, b, out);
                    let secs = started.elapsed().as_secs_f64();
                    if r.is_ok() && secs > 1e-4 {
                        let fps = ((b - from) as f64 / secs).clamp(1.0e3, 1.0e9);
                        self.flac_fps = 0.8 * self.flac_fps + 0.2 * fps;
                    }
                    r.map_err(|e| e.to_string())
                }
                Store::Memory { .. } => Err("a memory source isn't read".to_string()),
            };
            let len = match (decoded, src.rate) {
                (Ok(got), Some(rate)) => {
                    // The unit's engine frames, made from the file's.
                    let len = (sh.frames - first).min(unit_frames(sh.channels)) as usize * ch;
                    rate.run(ch, &self.scratch[..got], a, first, &mut buf[..len]);
                    len
                }
                (Ok(got), None) => got,
                (Err(why), _) => {
                    // Not what the index says, or damaged: never played.
                    // The source stops; what is cached plays out.
                    self.stats.flac_failed += 1;
                    self.pool.push(buf);
                    self.bytes.push(bytes);
                    self.stop_source(tag, why);
                    continue;
                }
            };
            self.bytes.push(bytes);
            if let Held::Unit { source, unit } = self.mirror[slot].held {
                self.map.remove(&(source, unit));
            }
            // The sound thread drops a unit's older copy (see the player's
            // Fill), so the mirror does too.
            if let Some(old) = self.map.remove(&(tag.source, unit)) {
                self.mirror[old].held = Held::Free;
            }
            self.mirror[slot] = MSlot {
                held: Held::Unit {
                    source: tag.source,
                    unit,
                },
                wanted: self.round,
            };
            self.map.insert((tag.source, unit), slot);
            self.request = self.request.wrapping_add(1);
            let request = self.request;
            self.send(ToPlayer::Fill {
                tag,
                unit,
                slot: slot as u32,
                request,
                buf,
                len,
            });
            busy = true;
        }
        busy
    }

    /// The units every playing cursor wants at a window of `window_s`,
    /// each with its earliest deadline in seconds.
    fn wants(&self, window_s: f64) -> Wants {
        let mut out = Wants::new();
        let rate = f64::from(self.plan.rate.max(1));
        for (s, src) in self.sources.iter().enumerate() {
            let Some(src) = src.as_ref() else {
                continue;
            };
            let Some(l) = src.shape() else {
                continue;
            };
            let uf = unit_frames(l.channels);
            for w in &src.cursors {
                let p = w.place;
                let speed = if p.speed.is_finite() && p.speed > 0.0 {
                    p.speed
                } else {
                    1.0
                };
                let stop = p.end.unwrap_or(u64::MAX).min(l.frames);
                let ahead = (window_s * rate * speed) as u64;
                let mut f = p.frame;
                let mut left = ahead;
                // Output frames before frame `f` plays.
                let mut lead = 0.0f64;
                let mut lapped = false;
                // Walk the frames to be played, round the loop once at most
                // (a second lap wants nothing new).
                while left > 0 && f < stop {
                    let end = match p.looping {
                        Some((ls, le)) if f < le && le > ls => le.min(stop),
                        _ => stop,
                    };
                    let take = (end - f).min(left);
                    let mut u = f / uf;
                    while u * uf < f + take {
                        let deadline = (lead + ((u * uf).saturating_sub(f)) as f64 / speed) / rate;
                        let e = out
                            .entry((s as u16, u as u32))
                            .or_insert((deadline, src.tag));
                        if deadline < e.0 {
                            e.0 = deadline;
                        }
                        u += 1;
                    }
                    left -= take;
                    lead += take as f64 / speed;
                    f = match p.looping {
                        Some((ls, le)) if end == le.min(stop) && le > ls && !lapped => ls,
                        _ => break,
                    };
                    lapped = true;
                }
            }
        }
        out
    }

    /// How long a read of `bytes` should take, from the measured speed.
    fn read_time(&self, bytes: usize) -> f64 {
        bytes as f64 / self.speed.max(1.0)
    }

    /// Plan: work out what fits, mark what is wanted, cancel what isn't,
    /// and ask for what is missing, earliest deadline first.
    fn ask(&mut self) -> bool {
        self.round += 1;
        let heard = self.heard;
        for src in self.sources.iter_mut().flatten() {
            // Stopped cursors: not heard of in the last two rounds.
            src.cursors.retain(|c| heard.wrapping_sub(c.round) <= 2);
        }
        // Admission: what plays at the wanted window, beside what is held
        // and reserved; if that doesn't fit, the largest window that does.
        self.fit_holds();
        let held = self.held_units(HoldTier::Next);
        let budget = self.mirror.len();
        let reserve = self.reserve;
        let need = |wants: &Wants| -> usize {
            wants.keys().filter(|k| !held.contains(k)).count() + held.len() + reserve
        };
        let mut wants = self.wants(self.plan.window_s);
        let playing = wants.len();
        let mut window = self.plan.window_s;
        let covered = need(&wants) <= budget;
        if !covered {
            let (mut lo, mut hi) = (0.0, self.plan.window_s);
            for _ in 0..12 {
                let mid = 0.5 * (lo + hi);
                if need(&self.wants(mid)) <= budget {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            window = lo;
            wants = self.wants(window);
        }
        self.admission = Admission {
            budget,
            reserved: reserve,
            held: held.len(),
            playing,
            window_s: window,
            covered,
            safe: window >= self.plan.min_window_s,
        };
        // The cushion: how long until a playing cursor needs a unit not in
        // hand (cached, not just asked for); the window when all of it is.
        self.cushion_s = if wants.is_empty() {
            f64::INFINITY
        } else {
            wants
                .iter()
                .filter(|(k, _)| !self.map.contains_key(k))
                .map(|(_, &(d, _))| d)
                .fold(window, f64::min)
        };
        self.gate.set_cushion(self.cushion_s);
        // Holds: by their deadline, or after everything playing.
        let now = Instant::now();
        for h in &self.holds {
            let Some(l) = self.sources[usize::from(h.tag.source)]
                .as_ref()
                .and_then(Src::shape)
            else {
                continue;
            };
            let deadline = match h.by {
                Some(t) => t.saturating_duration_since(now).as_secs_f64(),
                None => window + 1.0,
            };
            for u in Self::units_of(l, h.from, h.to) {
                let e = wants.entry((h.tag.source, u)).or_insert((deadline, h.tag));
                if deadline < e.0 {
                    e.0 = deadline;
                }
            }
        }
        // Mark what is cached; list what is missing. A unit read and
        // waiting for a buffer isn't missing: asking again would read it
        // twice, and its second fill would replace a copy already playing.
        let waiting: HashSet<(u16, u32)> =
            self.ready.iter().map(|(t, u, _)| (t.source, *u)).collect();
        let mut missing: Vec<(f64, Tag, u32)> = Vec::new();
        for (&(s, u), &(deadline, tag)) in &wants {
            if self.sources[usize::from(s)]
                .as_ref()
                .is_some_and(Src::stopped)
            {
                // Its card went, or it can't be read: what is cached plays
                // out, nothing is read.
                if let Some(&slot) = self.map.get(&(s, u)) {
                    self.mirror[slot].wanted = self.round;
                }
                continue;
            }
            match self.map.get(&(s, u)) {
                Some(&slot) => self.mirror[slot].wanted = self.round,
                None if !self.asked.contains_key(&(s, u)) && !waiting.contains(&(s, u)) => {
                    missing.push((deadline, tag, u))
                }
                None => {}
            }
        }
        // Cancel reads none of whose units are wanted now (and take back a
        // cancel for any wanted again before the device got to it).
        for f in self.flights.values() {
            if let Kind::Units { tag, first, units } = f.kind {
                let wanted = self.current(tag)
                    && !self.sources[usize::from(tag.source)]
                        .as_ref()
                        .is_some_and(Src::stopped)
                    && (first..first + units).any(|u| wants.contains_key(&(tag.source, u)));
                f.cancel.store(!wanted, Ordering::Relaxed);
            }
        }
        missing.sort_by(|a, b| {
            a.0.total_cmp(&b.0)
                .then(a.1.source.cmp(&b.1.source))
                .then(a.2.cmp(&b.2))
        });
        let missing_set: HashSet<(u16, u32)> =
            missing.iter().map(|&(_, t, u)| (t.source, u)).collect();
        let mut queue_s: f64 = self
            .flights
            .values()
            .map(|f| self.read_time(f.bytes) + f.decode_s)
            .sum();
        let mut busy = false;
        let mut all_asked = true;
        for i in 0..missing.len() {
            if self.flights.len() >= self.plan.in_flight {
                all_asked = false;
                break;
            }
            let (_, tag, unit) = missing[i];
            let s = tag.source;
            if self.asked.contains_key(&(s, unit)) {
                continue;
            }
            let mut bytes = self.read_bytes(s, unit, 1);
            let mut decode_s = self.decode_time(s, unit, 1);
            // The next deadline that isn't one of this file's next units.
            let other = missing[i + 1..]
                .iter()
                .find(|&&(_, t, u)| t.source != s || u < unit || u > unit + 64)
                .map_or(f64::INFINITY, |m| m.0);
            let mut units = 1u32;
            loop {
                let next = unit + units;
                if !missing_set.contains(&(s, next)) || self.asked.contains_key(&(s, next)) {
                    break;
                }
                let joined = self.read_bytes(s, unit, units + 1);
                let decoded = self.decode_time(s, unit, units + 1);
                if joined == 0
                    || joined > self.plan.join_bytes
                    || queue_s + self.read_time(joined) + decoded > 0.5 * other
                {
                    break;
                }
                bytes = joined;
                decode_s = decoded;
                units += 1;
            }
            let src = self.sources[usize::from(s)].as_ref().expect("current");
            let Some(file) = src.file().cloned() else {
                continue;
            };
            let Some(at) = src
                .span(unit)
                .and_then(|(first, _)| src.bytes_of(first, first))
                .map(|(at, _)| at)
            else {
                continue;
            };
            self.request = self.request.wrapping_add(1);
            let request = self.request;
            let cancel = Arc::new(AtomicBool::new(false));
            let req = ReadReq {
                request,
                file,
                at,
                len: bytes,
                buf: self.bytes.pop().unwrap_or_default(),
                cancel: cancel.clone(),
            };
            for u in unit..unit + units {
                self.asked.insert((s, u), (tag.gen, request));
            }
            self.flights.insert(
                request,
                Flight {
                    kind: Kind::Units {
                        tag,
                        first: unit,
                        units,
                    },
                    cancel,
                    bytes,
                    decode_s,
                },
            );
            queue_s += self.read_time(bytes) + decode_s;
            self.stats.reads += 1;
            self.stats.joined += u64::from(units - 1);
            self.dev.submit(req);
            busy = true;
        }
        // Whole-file reads: only once every playback read wanted is asked
        // for, one step at a time, so a playback read never waits behind
        // more than one step, and only while playback's cushion is longer
        // than a stall: a step capped at 512 KiB still stalls as long as
        // the stick does.
        if all_asked && self.flights.len() < self.plan.in_flight && self.cushion_s > self.stall() {
            busy |= self.job_step();
        }
        busy
    }

    /// Step until `stop`, sleeping `nap` after a step with nothing to do;
    /// for a coordinator thread.
    pub fn run_until(&mut self, mut stop: impl FnMut() -> bool, nap: Duration) {
        while !stop() {
            if !self.step() {
                std::thread::sleep(nap);
            }
        }
    }
}
