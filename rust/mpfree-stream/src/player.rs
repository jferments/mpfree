//! The sound thread's side: the cache it owns and reads from, and the
//! messages it takes and posts (see [`crate::msg`]).
//!
//! Every period: [`Player::begin`] takes at most [`Config::messages`]
//! messages and copies at most [`Config::copy`] samples of fills into the
//! cache (a unit can take several periods); [`Player::read`] copies a
//! source's frames out of the cache, silence where a unit isn't there
//! (counted as missed); [`Player::playing`] notes where a cursor plays,
//! posted to the coordinator once a planning round. None of these
//! allocates, frees, locks or blocks. [`crate::cursor::Cursor`] plays a
//! source through them, round a loop and at a speed.

use std::collections::VecDeque;
use std::sync::mpsc::{Receiver, SyncSender, TrySendError};
use std::sync::Arc;

use crate::msg::{FromPlayer, Place, Tag, ToPlayer, Where, MAX_CURSORS, MAX_SOURCES};

/// Samples (floats) in one cache unit: 64 KiB of float.
pub const UNIT: usize = 16_384;

/// Frames in one unit of a source with `channels` channels.
pub fn unit_frames(channels: u16) -> u64 {
    (UNIT / usize::from(channels.max(1))) as u64
}

/// Frames a voice fades over at the edge of a gap ([`Gaps::smooth`]):
/// 64, about 1.3 ms at 48 kHz.
pub const DECLICK: usize = 64;

/// Where a read ([`Player::read_gaps`]) found its frames missing: runs of
/// frames, each `(first, count)` in the frames read, in order. A read
/// covers a unit or two, so there are seldom more than two; past
/// [`Gaps::MAX`], the last run grows to take in the rest.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Gaps {
    runs: [(usize, usize); Gaps::MAX],
    n: usize,
}

impl Gaps {
    pub const MAX: usize = 4;

    fn add(&mut self, first: usize, count: usize) {
        if count == 0 {
            return;
        }
        if let Some(last) = self.runs[..self.n].last_mut() {
            if last.0 + last.1 == first || self.n == Gaps::MAX {
                last.1 = first + count - last.0;
                return;
            }
        }
        self.runs[self.n] = (first, count);
        self.n += 1;
    }

    /// The runs of frames missed, in order.
    pub fn runs(&self) -> &[(usize, usize)] {
        &self.runs[..self.n]
    }

    /// Whether the last of `frames` frames read was missed.
    pub fn ends_missed(&self, frames: usize) -> bool {
        self.runs().last().is_some_and(|&(a, c)| a + c >= frames)
    }

    /// Fade `out` (interleaved, `channels`; the frames read) so sound
    /// going missing and coming back never clicks: out over [`DECLICK`]
    /// frames before each gap and in after it. `lead` is what played
    /// just before these frames: if it had gone quiet, they fade in; if
    /// it was sounding and they start missing, the sound dies away from
    /// its last frame over the gap's first frames instead of stopping
    /// dead. Missed frames are silence already. Frames that play whole,
    /// with no gap near, are left as they are, bit for bit; so is all of
    /// a read with no gap and a clear lead.
    pub fn smooth(&self, out: &mut [f32], channels: usize, lead: Lead) {
        let ch = channels.clamp(1, 2);
        let frames = out.len() / ch;
        let ramp = |k: usize| (k + 1) as f32 / (DECLICK + 1) as f32;
        let mut start = 0;
        let mut quiet = lead == Lead::Quiet;
        if let (Lead::Sound(v), Some(&(0, c))) = (lead, self.runs().first()) {
            for j in 0..c.min(DECLICK).min(frames) {
                let g = 1.0 - ramp(j);
                for (o, x) in out[j * ch..(j + 1) * ch].iter_mut().zip(v) {
                    *o = x * g;
                }
            }
        }
        let runs = self.runs().iter().copied().chain([(frames, 0)]);
        for (a, c) in runs {
            let a = a.min(frames);
            // Frames start..a play: in after a gap, out before the next.
            for j in start..a {
                let mut g = 1.0f32;
                if quiet && j - start < DECLICK {
                    g *= ramp(j - start);
                }
                if a < frames && a - j <= DECLICK {
                    g *= ramp(a - j - 1);
                }
                if g < 1.0 {
                    for v in &mut out[j * ch..(j + 1) * ch] {
                        *v *= g;
                    }
                }
            }
            start = (a + c).min(frames);
            quiet = true;
        }
    }

    /// What `out` (the frames read, smoothed) leaves for the read after
    /// it: quiet if its last frame was missed, else its last frame.
    pub fn lead_after(&self, out: &[f32], channels: usize) -> Lead {
        let ch = channels.clamp(1, 2);
        let frames = out.len() / ch;
        if frames == 0 {
            return Lead::Clear;
        }
        if self.ends_missed(frames) {
            return Lead::Quiet;
        }
        let last = &out[(frames - 1) * ch..frames * ch];
        Lead::Sound([last[0], last[ch - 1]])
    }
}

/// What a voice played just before a read, for [`Gaps::smooth`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Lead {
    /// Nothing to fade from: the voice starts here, or played elsewhere.
    Clear,
    /// Its frames had gone missing: what comes back fades in.
    Quiet,
    /// It was sounding, its last frame these samples (left and right; a
    /// mono voice has the same in both): a gap now dies away from them.
    Sound([f32; 2]),
}

#[derive(Clone, Copy, Debug)]
pub struct Config {
    /// Cache slots (each [`UNIT`] samples).
    pub slots: usize,
    /// Samples copied into the cache a period, at most.
    pub copy: usize,
    /// Messages taken a period, at most.
    pub messages: usize,
    /// Frames between planning rounds (where each source plays is posted
    /// once a round).
    pub plan_frames: u32,
    /// Buffers in circulation (the coordinator's pool): the most the sound
    /// thread may have to hold while the way back is full.
    pub buffers: usize,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            slots: 2048, // 128 MiB
            copy: 8192,
            messages: 64,
            plan_frames: 1024,
            buffers: 64,
        }
    }
}

/// The fewest cache slots a budget makes: 1 MiB.
pub const MIN_SLOTS: usize = 16;

impl Config {
    /// The cache for the musician's budget of `bytes` (64, 128 or 256 MB):
    /// as many whole units as fit, at least [`MIN_SLOTS`], the rest as by
    /// default. The cache is made whole at start, so this is what it takes.
    pub fn for_budget(bytes: u64) -> Config {
        let unit = (UNIT * std::mem::size_of::<f32>()) as u64;
        Config {
            slots: usize::try_from(bytes / unit)
                .unwrap_or(usize::MAX)
                .max(MIN_SLOTS),
            ..Config::default()
        }
    }

    /// The bytes the cache takes.
    pub fn cache_bytes(&self) -> u64 {
        (self.slots as u64).saturating_mul((UNIT * std::mem::size_of::<f32>()) as u64)
    }
}

/// Counters for the screen and the tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Frames read whose unit was not in the cache (they played silence).
    pub missed: u64,
    /// Fills dropped for an older generation, or short.
    pub stale: u32,
    pub short: u32,
    /// Where messages that did not fit on the way out.
    pub where_dropped: u32,
    /// Units copied in.
    pub filled: u64,
    /// Messages that could neither go back nor be held: only if more
    /// buffers circulate than [`Config::buffers`] says (never freed here).
    pub lost: u32,
    /// Notes from cursors at or past [`MAX_CURSORS`]: not planned for, so
    /// they play only what is already in the cache.
    pub over_limit: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Free,
    Filling,
    Ready,
}

#[derive(Clone, Copy, Debug)]
struct Slot {
    state: State,
    source: u16,
    unit: u32,
}

#[derive(Clone, Copy, Debug)]
struct Source {
    gen: u32,
    open: bool,
    channels: u16,
    frames: u64,
}

/// A fill copying in over periods.
struct Copying {
    tag: Tag,
    slot: usize,
    buf: Vec<f32>,
    len: usize,
    done: usize,
}

const EMPTY: u64 = u64::MAX;

/// The most messages that can wait to go back at once: every buffer, a
/// refusal for each, every memory source, and a place for every cursor.
fn back(cfg: &Config) -> usize {
    2 * cfg.buffers + MAX_SOURCES + MAX_CURSORS
}

fn key(source: u16, unit: u32) -> u64 {
    (u64::from(source) << 32) | u64::from(unit)
}

fn hash(k: u64) -> usize {
    (k.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 20) as usize
}

pub struct Player {
    cfg: Config,
    rx: Receiver<ToPlayer>,
    tx: SyncSender<FromPlayer>,
    project: u32,
    sources: Vec<Source>,
    /// Memory sources' frames, by source.
    resident: Vec<Option<Arc<[f32]>>>,
    data: Vec<f32>,
    slots: Vec<Slot>,
    keys: Vec<u64>,
    vals: Vec<u32>,
    mask: usize,
    copying: Option<Copying>,
    /// Buffers and refusals waiting for room on the way back.
    held: VecDeque<FromPlayer>,
    /// Where each cursor played this round, by cursor id, and the ids
    /// noted.
    wants: Vec<Option<Where>>,
    noted: Vec<u16>,
    since_plan: u32,
    round: u32,
    stats: Stats,
}

impl Player {
    /// The player (for the sound thread) and its channels' far ends (for
    /// the coordinator). Allocates everything it will ever use.
    pub fn new(cfg: Config) -> (Player, Receiver<FromPlayer>, SyncSender<ToPlayer>) {
        let (to_tx, to_rx) = std::sync::mpsc::sync_channel(cfg.messages.max(1) * 4);
        let (from_tx, from_rx) = std::sync::mpsc::sync_channel(back(&cfg) + cfg.messages.max(1));
        let size = (cfg.slots * 2).next_power_of_two().max(16);
        let p = Player {
            cfg,
            rx: to_rx,
            tx: from_tx,
            project: 0,
            sources: vec![
                Source {
                    gen: 0,
                    open: false,
                    channels: 1,
                    frames: 0,
                };
                MAX_SOURCES
            ],
            resident: vec![None; MAX_SOURCES],
            data: vec![0.0; cfg.slots * UNIT],
            slots: vec![
                Slot {
                    state: State::Free,
                    source: 0,
                    unit: 0,
                };
                cfg.slots
            ],
            keys: vec![EMPTY; size],
            vals: vec![0; size],
            mask: size - 1,
            copying: None,
            held: VecDeque::with_capacity(back(&cfg) + cfg.buffers),
            wants: vec![None; MAX_CURSORS],
            noted: Vec::with_capacity(MAX_CURSORS),
            since_plan: 0,
            round: 0,
            stats: Stats::default(),
        };
        (p, from_rx, to_tx)
    }

    pub fn stats(&self) -> Stats {
        self.stats
    }

    pub fn config(&self) -> Config {
        self.cfg
    }

    /// The source's generation and whether it is open.
    pub fn source(&self, source: u16) -> Option<(u32, bool)> {
        self.sources
            .get(usize::from(source))
            .map(|s| (s.gen, s.open))
    }

    /// An open source's channels and length in frames.
    pub fn shape(&self, source: u16) -> Option<(u16, u64)> {
        self.sources
            .get(usize::from(source))
            .filter(|s| s.open)
            .map(|s| (s.channels, s.frames))
    }

    /// Whether frames `from..from + frames` of `source` would all play
    /// now (a launch's check at its boundary): open, and every unit Ready
    /// (a memory source always is). Frames past the end don't count.
    pub fn covers(&self, source: u16, from: u64, frames: u64) -> bool {
        let Some(src) = self.sources.get(usize::from(source)).copied() else {
            return false;
        };
        if !src.open {
            return false;
        }
        if self.resident[usize::from(source)].is_some() {
            return true;
        }
        let end = (from + frames).min(src.frames);
        if from >= end {
            return true;
        }
        let uf = unit_frames(src.channels);
        ((from / uf) as u32..end.div_ceil(uf) as u32).all(|u| {
            self.find(source, u)
                .is_some_and(|s| self.slots[s].state == State::Ready)
        })
    }

    /// Units ready to play.
    pub fn ready(&self) -> usize {
        self.slots
            .iter()
            .filter(|s| s.state == State::Ready)
            .count()
    }

    // ---- the table ------------------------------------------------------

    fn find(&self, source: u16, unit: u32) -> Option<usize> {
        let k = key(source, unit);
        let mut i = hash(k) & self.mask;
        loop {
            if self.keys[i] == k {
                return Some(self.vals[i] as usize);
            }
            if self.keys[i] == EMPTY {
                return None;
            }
            i = (i + 1) & self.mask;
        }
    }

    fn insert(&mut self, k: u64, v: usize) {
        let mut i = hash(k) & self.mask;
        while self.keys[i] != EMPTY && self.keys[i] != k {
            i = (i + 1) & self.mask;
        }
        self.keys[i] = k;
        self.vals[i] = v as u32;
    }

    fn remove(&mut self, k: u64) {
        let mut i = hash(k) & self.mask;
        loop {
            if self.keys[i] == k {
                break;
            }
            if self.keys[i] == EMPTY {
                return;
            }
            i = (i + 1) & self.mask;
        }
        let mut j = i;
        loop {
            j = (j + 1) & self.mask;
            if self.keys[j] == EMPTY {
                break;
            }
            let h = hash(self.keys[j]) & self.mask;
            let stays = if i <= j {
                i < h && h <= j
            } else {
                i < h || h <= j
            };
            if !stays {
                self.keys[i] = self.keys[j];
                self.vals[i] = self.vals[j];
                i = j;
            }
        }
        self.keys[i] = EMPTY;
    }

    fn free(&mut self, s: usize) {
        let sl = self.slots[s];
        if sl.state != State::Free {
            self.remove(key(sl.source, sl.unit));
            self.slots[s].state = State::Free;
        }
    }

    // ---- the period -----------------------------------------------------

    fn send(&mut self, m: FromPlayer) {
        if !self.held.is_empty() {
            self.hold(m);
            return;
        }
        match self.tx.try_send(m) {
            Ok(()) => {}
            Err(TrySendError::Full(m)) => self.hold(m),
            // The coordinator is gone: keep the buffer (never freed here).
            Err(TrySendError::Disconnected(m)) => self.hold(m),
        }
    }

    fn hold(&mut self, m: FromPlayer) {
        if self.held.len() < self.held.capacity() {
            self.held.push_back(m);
        } else if let FromPlayer::Where(_) = m {
            self.stats.where_dropped += 1;
        } else {
            // More buffers than the pool holds: a broken promise. Never
            // freed on this thread, so left (counted).
            self.stats.lost += 1;
            std::mem::forget(m);
        }
    }

    /// What waits to go back, oldest first, while there is room.
    fn flush(&mut self) {
        while let Some(m) = self.held.pop_front() {
            if let Err(TrySendError::Full(m) | TrySendError::Disconnected(m)) = self.tx.try_send(m)
            {
                self.held.push_front(m);
                break;
            }
        }
    }

    fn refuse(&mut self, tag: Tag, unit: u32, slot: u32, short: bool, buf: Vec<f32>) {
        if short {
            self.stats.short += 1;
        } else {
            self.stats.stale += 1;
        }
        self.send(FromPlayer::Refused {
            tag,
            unit,
            slot,
            short,
        });
        self.send(FromPlayer::Back(buf));
    }

    fn current(&self, tag: Tag) -> bool {
        tag.project == self.project
            && self
                .sources
                .get(usize::from(tag.source))
                .is_some_and(|s| s.open && s.gen == tag.gen)
    }

    fn apply(&mut self, m: ToPlayer) {
        match m {
            ToPlayer::Project(p) => {
                self.project = p;
                for s in 0..MAX_SOURCES {
                    self.sources[s].open = false;
                    self.give_back(s);
                }
                if let Some(c) = self.copying.take() {
                    self.send(FromPlayer::Back(c.buf));
                }
                for s in 0..self.slots.len() {
                    self.free(s);
                }
            }
            ToPlayer::Resident {
                tag,
                channels,
                data,
            } => {
                let Some(old) = self.sources.get(usize::from(tag.source)).copied() else {
                    self.send(FromPlayer::Resident(data));
                    return;
                };
                let ch = channels.clamp(1, 2);
                if tag.project != self.project || tag.gen <= old.gen || channels != ch {
                    self.stats.stale += 1;
                    self.send(FromPlayer::Resident(data));
                    return;
                }
                self.drop_source(tag.source);
                self.sources[usize::from(tag.source)] = Source {
                    gen: tag.gen,
                    open: true,
                    channels: ch,
                    frames: (data.len() / usize::from(ch)) as u64,
                };
                self.resident[usize::from(tag.source)] = Some(data);
            }
            ToPlayer::Open {
                tag,
                channels,
                frames,
            } => {
                let Some(old) = self.sources.get(usize::from(tag.source)).copied() else {
                    return;
                };
                // A source's generations only go up (the coordinator's
                // count, never reused in a run).
                if tag.project != self.project || tag.gen <= old.gen {
                    self.stats.stale += 1;
                    return;
                }
                self.drop_source(tag.source);
                self.sources[usize::from(tag.source)] = Source {
                    gen: tag.gen,
                    open: true,
                    channels: channels.clamp(1, 2),
                    frames,
                };
            }
            ToPlayer::Close { tag } => {
                if self.current(tag) {
                    self.drop_source(tag.source);
                    self.sources[usize::from(tag.source)].open = false;
                } else {
                    self.stats.stale += 1;
                }
            }
            ToPlayer::Free { slot } => {
                if (slot as usize) < self.slots.len() {
                    self.free(slot as usize);
                }
            }
            ToPlayer::Grow { tag, frames } => {
                let s = usize::from(tag.source);
                if !self.current(tag) || self.resident[s].is_some() {
                    self.stats.stale += 1;
                    return;
                }
                let src = self.sources[s];
                if frames <= src.frames {
                    return;
                }
                // The unit cut short at the old end, if one was, goes.
                // Messages are taken only between copies, so a fill of it
                // sent before this has finished copying in, and goes too.
                let uf = unit_frames(src.channels);
                if !src.frames.is_multiple_of(uf) {
                    if let Some(slot) = self.find(tag.source, (src.frames / uf) as u32) {
                        self.free(slot);
                    }
                }
                self.sources[s].frames = frames;
            }
            ToPlayer::Fill {
                tag,
                unit,
                slot,
                request: _,
                buf,
                len,
            } => {
                let s = slot as usize;
                if !self.current(tag) || s >= self.slots.len() {
                    return self.refuse(tag, unit, slot, false, buf);
                }
                let src = self.sources[usize::from(tag.source)];
                let uf = unit_frames(src.channels);
                let want = (src.frames.saturating_sub(u64::from(unit) * uf)).min(uf) as usize
                    * usize::from(src.channels);
                if want == 0 || len < want || len > buf.len() {
                    return self.refuse(tag, unit, slot, true, buf);
                }
                // The slot is the coordinator's choice: what was there goes.
                self.free(s);
                if let Some(old) = self.find(tag.source, unit) {
                    self.free(old);
                }
                self.slots[s] = Slot {
                    state: State::Filling,
                    source: tag.source,
                    unit,
                };
                self.insert(key(tag.source, unit), s);
                self.copying = Some(Copying {
                    tag,
                    slot: s,
                    buf,
                    len: want,
                    done: 0,
                });
            }
        }
    }

    /// A memory source's frames go back to the coordinator.
    fn give_back(&mut self, s: usize) {
        if let Some(d) = self.resident[s].take() {
            self.send(FromPlayer::Resident(d));
        }
    }

    fn drop_source(&mut self, source: u16) {
        self.give_back(usize::from(source));
        if let Some(c) = &self.copying {
            if c.tag.source == source {
                let c = self.copying.take().expect("checked");
                self.send(FromPlayer::Back(c.buf));
            }
        }
        for s in 0..self.slots.len() {
            if self.slots[s].state != State::Free && self.slots[s].source == source {
                self.free(s);
            }
        }
    }

    /// Start of a period: send what waits to go back, take messages and
    /// copy fills in, within the period's limits.
    pub fn begin(&mut self) {
        self.flush();
        let mut budget = self.cfg.copy;
        let mut taken = 0;
        loop {
            if let Some(mut c) = self.copying.take() {
                let n = budget.min(c.len - c.done);
                let b = c.slot * UNIT + c.done;
                self.data[b..b + n].copy_from_slice(&c.buf[c.done..c.done + n]);
                c.done += n;
                budget -= n;
                if c.done == c.len {
                    self.slots[c.slot].state = State::Ready;
                    self.stats.filled += 1;
                    self.send(FromPlayer::Back(c.buf));
                } else {
                    self.copying = Some(c);
                    break;
                }
            }
            if budget == 0 || taken == self.cfg.messages {
                break;
            }
            match self.rx.try_recv() {
                Ok(m) => {
                    taken += 1;
                    self.apply(m);
                }
                Err(_) => break,
            }
        }
    }

    /// Copy frames `from..` of `source` into `out` (interleaved, the
    /// source's channels; as many frames as `out` holds). Frames past the
    /// source's end, or of a source not open, are silence and not counted;
    /// frames whose unit isn't ready are silence and counted as missed.
    /// Gives the frames missed.
    pub fn read(&mut self, source: u16, from: u64, out: &mut [f32]) -> u64 {
        let missed = self.peek(source, from, out);
        self.stats.missed += missed;
        missed
    }

    /// [`Player::read`], also saying where in `out` the frames missed lie
    /// ([`Gaps`]), so a voice can fade out before a gap and back in after
    /// it ([`Gaps::smooth`]) instead of clicking.
    pub fn read_gaps(&mut self, source: u16, from: u64, out: &mut [f32], gaps: &mut Gaps) -> u64 {
        *gaps = Gaps::default();
        let missed = self.fill(source, from, out, Some(gaps));
        self.stats.missed += missed;
        missed
    }

    /// [`Player::read`] without counting what's missed (for frames looked
    /// at but not played, like an interpolator's neighbours).
    pub fn peek(&self, source: u16, from: u64, out: &mut [f32]) -> u64 {
        self.fill(source, from, out, None)
    }

    fn fill(&self, source: u16, from: u64, out: &mut [f32], mut gaps: Option<&mut Gaps>) -> u64 {
        let Some(src) = self.sources.get(usize::from(source)).copied() else {
            out.fill(0.0);
            return 0;
        };
        let ch = usize::from(src.channels);
        let frames = (out.len() / ch) as u64;
        if !src.open {
            out.fill(0.0);
            return 0;
        }
        if let Some(d) = &self.resident[usize::from(source)] {
            let first = usize::try_from(from)
                .unwrap_or(usize::MAX)
                .min(d.len() / ch);
            let n = (d.len() / ch - first).min(frames as usize) * ch;
            out[..n].copy_from_slice(&d[first * ch..first * ch + n]);
            out[n..].fill(0.0);
            return 0;
        }
        let uf = unit_frames(src.channels);
        let mut missed = 0;
        let mut f = from;
        let mut o = 0;
        let end = from + frames;
        while f < end {
            let unit = (f / uf) as u32;
            let in_unit = f % uf;
            let n = (uf - in_unit).min(end - f);
            let samples = n as usize * ch;
            let dst = &mut out[o..o + samples];
            let have = src.frames.saturating_sub(f).min(n) as usize * ch;
            match self.find(source, unit) {
                Some(s) if self.slots[s].state == State::Ready => {
                    let b = s * UNIT + in_unit as usize * ch;
                    dst[..have].copy_from_slice(&self.data[b..b + have]);
                    dst[have..].fill(0.0);
                }
                _ => {
                    dst.fill(0.0);
                    missed += (have / ch) as u64;
                    if let Some(g) = gaps.as_deref_mut() {
                        g.add(o / ch, have / ch);
                    }
                }
            }
            f += n;
            o += samples;
        }
        missed
    }

    /// Note that cursor `cursor` plays `source` at `place` this period
    /// (the last note of a round is posted). A cursor id at or past
    /// [`MAX_CURSORS`] isn't planned for, and is counted
    /// ([`Stats::over_limit`]).
    pub fn playing(&mut self, source: u16, cursor: u16, place: Place) {
        let c = usize::from(cursor);
        if c >= MAX_CURSORS {
            self.stats.over_limit = self.stats.over_limit.saturating_add(1);
            return;
        }
        let Some(src) = self.sources.get(usize::from(source)).copied() else {
            return;
        };
        if !src.open {
            return;
        }
        let w = Where {
            tag: Tag {
                project: self.project,
                source,
                gen: src.gen,
            },
            cursor,
            round: self.round,
            place,
        };
        if self.wants[c].replace(w).is_none() {
            self.noted.push(cursor);
        }
    }

    /// End of a period of `frames`: once a round, post where each cursor
    /// played.
    pub fn end(&mut self, frames: u32) {
        self.since_plan += frames;
        if self.since_plan < self.cfg.plan_frames {
            return;
        }
        self.since_plan = 0;
        if self.noted.is_empty() && self.held.is_empty() {
            // Nothing played: the round alone, if there is room (a full
            // queue tries again next round).
            let _ = self.tx.try_send(FromPlayer::Round(self.round));
        }
        for i in 0..self.noted.len() {
            let c = usize::from(self.noted[i]);
            if let Some(mut w) = self.wants[c].take() {
                w.round = self.round;
                self.send(FromPlayer::Where(w));
            }
        }
        self.noted.clear();
        self.round = self.round.wrapping_add(1);
    }
}

#[cfg(test)]
mod gaps_tests {
    use super::{Gaps, Lead, DECLICK};

    fn gaps(runs: &[(usize, usize)]) -> Gaps {
        let mut g = Gaps::default();
        for &(a, c) in runs {
            g.add(a, c);
        }
        g
    }

    /// Stereo frames of 1.0 with `runs` missed (silence), as a read gives.
    fn read(frames: usize, runs: &[(usize, usize)]) -> Vec<f32> {
        let mut out = vec![1.0f32; 2 * frames];
        for &(a, c) in runs {
            out[2 * a..2 * (a + c)].fill(0.0);
        }
        out
    }

    #[test]
    fn neighbouring_runs_join_and_past_the_most_the_last_grows() {
        assert_eq!(gaps(&[(0, 10), (10, 5)]).runs(), [(0, 15)]);
        let g = gaps(&[(0, 1), (2, 1), (4, 1), (6, 1), (8, 1)]);
        assert_eq!(g.runs(), [(0, 1), (2, 1), (4, 1), (6, 3)]);
        assert!(g.ends_missed(9));
        assert!(!g.ends_missed(10));
        assert!(gaps(&[]).runs().is_empty());
    }

    #[test]
    fn sound_fades_out_before_a_gap_and_in_after_it() {
        let runs = [(300, 100)];
        let mut out = read(600, &runs);
        gaps(&runs).smooth(&mut out, 2, Lead::Clear);
        let at = |j: usize| out[2 * j];
        // Whole until the fade, then down to nearly nothing at the gap.
        assert_eq!(at(300 - DECLICK - 1), 1.0);
        assert!(at(299) > 0.0 && at(299) < 0.05);
        assert!((1..DECLICK).all(|k| at(300 - DECLICK + k) < at(300 - DECLICK + k - 1)));
        assert_eq!(at(300), 0.0);
        // Back from nearly nothing, and whole again after the fade.
        assert!(at(400) > 0.0 && at(400) < 0.05);
        assert!((401..400 + DECLICK).all(|j| at(j) > at(j - 1)));
        assert_eq!(at(400 + DECLICK), 1.0);
        // Both channels alike.
        assert!(out.chunks_exact(2).all(|f| f[0] == f[1]));
        assert_eq!(gaps(&runs).lead_after(&out, 2), Lead::Sound([1.0, 1.0]));
    }

    #[test]
    fn a_read_after_quiet_fades_in_and_one_starting_missing_dies_away() {
        // The read before ended missing: this one fades in from its first
        // frame.
        let mut out = read(300, &[]);
        gaps(&[]).smooth(&mut out, 2, Lead::Quiet);
        assert!(out[0] < 0.05);
        assert_eq!(out[2 * DECLICK], 1.0);
        // The read before ended sounding, and this one starts missing: the
        // sound dies away from its last frame instead of stopping dead.
        let runs = [(0, 200)];
        let mut out = read(300, &runs);
        let g = gaps(&runs);
        g.smooth(&mut out, 2, Lead::Sound([0.8, -0.4]));
        assert!((out[0] - 0.8).abs() < 0.05 && (out[1] + 0.4).abs() < 0.05);
        assert!((1..DECLICK).all(|j| out[2 * j] < out[2 * (j - 1)]));
        assert!(out[2 * DECLICK..2 * 200].iter().all(|&v| v == 0.0));
        // And what comes back in the same read fades in.
        assert!(out[2 * 200] < 0.05);
        // A read that ends missing leaves quiet behind it.
        let runs = [(250, 50)];
        let mut out = read(300, &runs);
        gaps(&runs).smooth(&mut out, 2, Lead::Clear);
        assert_eq!(gaps(&runs).lead_after(&out, 2), Lead::Quiet);
        // Mono: its one channel on both sides.
        let mono = vec![0.5f32; 10];
        assert_eq!(gaps(&[]).lead_after(&mono, 1), Lead::Sound([0.5, 0.5]));
    }

    #[test]
    fn with_no_gap_and_a_clear_or_sounding_lead_nothing_changes() {
        for lead in [Lead::Clear, Lead::Sound([1.0, 1.0])] {
            let mut out = read(300, &[]);
            gaps(&[]).smooth(&mut out, 2, lead);
            assert!(out.iter().all(|&v| v == 1.0));
        }
    }
}
