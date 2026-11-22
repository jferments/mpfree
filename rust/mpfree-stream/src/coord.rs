//! The coordinator: knows the sources and their files, plans reads from
//! where the sound thread says each source plays, and fills the sound
//! thread's cache. Runs on its own thread (or is stepped by a test);
//! nothing here is on the sound thread.
//!
//! - **Planning.** Each cursor playing a source wants the units from where
//!   it plays to a window ahead ([`Plan::window_s`] of the file at its
//!   speed, round its loop). Units not cached or asked for are asked for
//!   earliest deadline first, up to [`Plan::in_flight`] at once. A cursor
//!   not heard of for two of the sound thread's rounds has stopped.
//! - **Room.** A unit goes in a free slot, else in the slot wanted longest
//!   ago and not wanted in the last round. With none, the read waits.
//! - **Generations.** Every request carries its [`Tag`]; a reply for a
//!   source closed or replaced meanwhile is dropped (counted). The sound
//!   thread checks again on its side.
//! - **Faults.** A read that fails or comes back short is counted and the
//!   unit asked for again when next wanted; nothing short is ever sent to
//!   be played.

use std::collections::{HashMap, VecDeque};
use std::fs::File;
use std::io;
use std::os::unix::fs::FileExt;
use std::path::Path;
use std::sync::mpsc::{Receiver, SyncSender, TrySendError};
use std::sync::Arc;

use crate::format::{decode, layout, Layout};
use crate::msg::{FromPlayer, Tag, ToPlayer, Where, MAX_SOURCES};
use crate::player::{unit_frames, Config, UNIT};

/// How far ahead to read.
#[derive(Clone, Copy, Debug)]
pub struct Plan {
    /// Seconds of each source wanted ahead of where it plays.
    pub window_s: f64,
    /// Reads asked for and not answered, at most.
    pub in_flight: usize,
}

impl Default for Plan {
    fn default() -> Plan {
        Plan {
            window_s: 8.0,
            in_flight: 8,
        }
    }
}

/// A read of part of a file, answered with the bytes.
pub struct ReadReq {
    pub tag: Tag,
    pub unit: u32,
    pub file: Arc<File>,
    pub at: u64,
    pub len: usize,
    pub buf: Vec<u8>,
}

pub struct ReadReply {
    pub tag: Tag,
    pub unit: u32,
    pub buf: Vec<u8>,
    /// Bytes read, or the error.
    pub got: io::Result<usize>,
}

/// What does the reading: the I/O thread of a device, or a test's stand-in.
pub trait Device: Send {
    fn submit(&mut self, r: ReadReq);
    fn poll(&mut self) -> Option<ReadReply>;
}

/// Reads at once, in the caller's thread (tests, and tools).
#[derive(Default)]
pub struct Inline {
    done: VecDeque<ReadReply>,
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

impl Device for Inline {
    fn submit(&mut self, mut r: ReadReq) {
        r.buf.resize(r.len, 0);
        let got = read_at(&r.file, r.at, &mut r.buf);
        self.done.push_back(ReadReply {
            tag: r.tag,
            unit: r.unit,
            buf: r.buf,
            got,
        });
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
                for mut r in jobs {
                    r.buf.resize(r.len, 0);
                    let got = read_at(&r.file, r.at, &mut r.buf);
                    let reply = ReadReply {
                        tag: r.tag,
                        unit: r.unit,
                        buf: r.buf,
                        got,
                    };
                    if done.send(reply).is_err() {
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
    pub reads: u64,
    pub read_errors: u32,
    pub short_reads: u32,
    /// Replies for a source closed or replaced meanwhile.
    pub stale: u32,
    /// Fills the sound thread refused.
    pub refused: u32,
    /// Times a read waited because no slot could be given up.
    pub no_room: u32,
}

struct Src {
    tag: Tag,
    file: Arc<File>,
    layout: Layout,
    /// Where each cursor playing it last played.
    cursors: Vec<Where>,
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
    /// Reads in flight: the generation that asked.
    asked: HashMap<(u16, u32), u32>,
    pool: Vec<Vec<f32>>,
    bytes: Vec<Vec<u8>>,
    /// Decoded units waiting for a buffer or a slot.
    ready: VecDeque<(Tag, u32, Vec<u8>, usize)>,
    /// Messages waiting for room on the way to the sound thread.
    out: VecDeque<ToPlayer>,
    round: u64,
    /// The newest of the sound thread's rounds heard of.
    heard: u32,
    request: u32,
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
            pool: (0..cfg.buffers).map(|_| vec![0.0; UNIT]).collect(),
            bytes: Vec::new(),
            ready: VecDeque::new(),
            out: VecDeque::new(),
            round: 1,
            heard: 0,
            request: 0,
            stats: Stats::default(),
        }
    }

    pub fn stats(&self) -> Stats {
        self.stats
    }

    /// Units the mirror says are in the sound thread's cache (or on their
    /// way there).
    pub fn cached(&self) -> usize {
        self.map.len()
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
        self.ready.clear();
        let p = self.project;
        self.send(ToPlayer::Project(p));
        p
    }

    /// Make `source` the sound in the file at `path` (replacing what it
    /// was, under a new generation).
    pub fn open(&mut self, source: u16, path: &Path) -> Result<Tag, String> {
        let s = usize::from(source);
        if s >= MAX_SOURCES {
            return Err(format!("source {source}: past the last"));
        }
        let mut file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let layout = layout(&mut file).map_err(|e| format!("{}: {e}", path.display()))?;
        if self.sources[s].is_some() {
            self.release(source);
        }
        let tag = Tag {
            project: self.project,
            source,
            gen: self.next_gen,
        };
        self.next_gen += 1;
        self.sources[s] = Some(Src {
            tag,
            file: Arc::new(file),
            layout,
            cursors: Vec::new(),
        });
        self.send(ToPlayer::Open {
            tag,
            channels: layout.channels,
            frames: layout.frames,
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

    pub fn layout(&self, source: u16) -> Option<Layout> {
        self.sources
            .get(usize::from(source))?
            .as_ref()
            .map(|s| s.layout)
    }

    fn current(&self, tag: Tag) -> bool {
        tag.project == self.project
            && self
                .sources
                .get(usize::from(tag.source))
                .and_then(Option::as_ref)
                .is_some_and(|s| s.tag == tag)
    }

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
            // A reply for an older generation leaves the newer one's
            // request in flight.
            if self.asked.get(&(r.tag.source, r.unit)) == Some(&r.tag.gen) {
                self.asked.remove(&(r.tag.source, r.unit));
            }
            if !self.current(r.tag) {
                self.stats.stale += 1;
                self.bytes.push(r.buf);
                continue;
            }
            let want = self.unit_bytes(r.tag.source, r.unit);
            match r.got {
                Ok(n) if n == want => self.ready.push_back((r.tag, r.unit, r.buf, n)),
                Ok(_) => {
                    self.stats.short_reads += 1;
                    self.bytes.push(r.buf);
                }
                Err(_) => {
                    self.stats.read_errors += 1;
                    self.bytes.push(r.buf);
                }
            }
        }
        busy |= self.fill();
        busy |= self.ask();
        busy
    }

    fn unit_bytes(&self, source: u16, unit: u32) -> usize {
        let Some(src) = self.sources[usize::from(source)].as_ref() else {
            return 0;
        };
        let l = src.layout;
        let uf = unit_frames(l.channels);
        let first = u64::from(unit) * uf;
        (l.frames.saturating_sub(first).min(uf)) as usize * l.block_align()
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
            let (tag, unit, bytes, n) = self.ready.pop_front().expect("checked");
            let l = self.sources[usize::from(tag.source)]
                .as_ref()
                .expect("current")
                .layout;
            let mut buf = self.pool.pop().expect("checked");
            let len = decode(l.sample, l.endian, &bytes[..n], &mut buf);
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
            self.request += 1;
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

    /// Plan: mark what each source wants, ask for what is missing,
    /// earliest deadline first.
    fn ask(&mut self) -> bool {
        self.round += 1;
        let mut missing: Vec<(f64, Tag, u32)> = Vec::new();
        let heard = self.heard;
        for s in 0..MAX_SOURCES {
            let Some(src) = self.sources[s].as_mut() else {
                continue;
            };
            // Stopped cursors: not heard of in the last two rounds.
            src.cursors.retain(|c| heard.wrapping_sub(c.round) <= 2);
            let src = self.sources[s].as_ref().expect("checked");
            for w in &src.cursors {
                let l = src.layout;
                let uf = unit_frames(l.channels);
                let speed = if w.speed.is_finite() && w.speed > 0.0 {
                    w.speed
                } else {
                    1.0
                };
                let ahead = (self.plan.window_s * f64::from(l.rate) * speed) as u64;
                let mut f = w.frame;
                let mut left = ahead;
                let mut lead = 0.0f64;
                let mut lapped = false;
                // Walk the frames to be played, round the loop once at most
                // (a second lap wants nothing new).
                while left > 0 && f < l.frames {
                    let end = match w.looping {
                        Some((ls, le)) if f < le && le > ls => le.min(l.frames),
                        _ => l.frames,
                    };
                    let take = (end - f).min(left);
                    let mut u = f / uf;
                    while u * uf < f + take {
                        let unit = u as u32;
                        let deadline = lead + ((u * uf).saturating_sub(f)) as f64 / speed;
                        match self.map.get(&(s as u16, unit)) {
                            Some(&slot) => self.mirror[slot].wanted = self.round,
                            None if !self.asked.contains_key(&(s as u16, unit)) => {
                                missing.push((deadline, src.tag, unit));
                            }
                            None => {}
                        }
                        u += 1;
                    }
                    left -= take;
                    lead += take as f64 / speed;
                    f = match w.looping {
                        Some((ls, le)) if end == le.min(l.frames) && le > ls && !lapped => ls,
                        _ => break,
                    };
                    lapped = true;
                }
            }
        }
        missing.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut busy = false;
        for (_, tag, unit) in missing {
            if self.asked.len() >= self.plan.in_flight {
                break;
            }
            if self.asked.contains_key(&(tag.source, unit)) {
                continue;
            }
            let src = self.sources[usize::from(tag.source)]
                .as_ref()
                .expect("current");
            let l = src.layout;
            let first = u64::from(unit) * unit_frames(l.channels);
            let len = self.unit_bytes(tag.source, unit);
            let req = ReadReq {
                tag,
                unit,
                file: src.file.clone(),
                at: l.byte_of(first),
                len,
                buf: self.bytes.pop().unwrap_or_default(),
            };
            self.asked.insert((tag.source, unit), tag.gen);
            self.stats.reads += 1;
            self.dev.submit(req);
            busy = true;
        }
        busy
    }

    /// Step until nothing happens for `idle` in a row, sleeping `nap`
    /// between idle steps; for a coordinator thread.
    pub fn run_until(&mut self, mut stop: impl FnMut() -> bool, nap: std::time::Duration) {
        while !stop() {
            if !self.step() {
                std::thread::sleep(nap);
            }
        }
    }
}
