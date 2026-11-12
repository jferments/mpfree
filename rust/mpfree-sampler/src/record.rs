//! Recording, workstation half of step S4.
//!
//! The audio thread does one thing: it appends every period of input (and,
//! with a second buffer, of the master output) to a [`rolling`] buffer that
//! always holds the last couple of minutes. It never allocates, locks or
//! waits. Everything else happens on a worker, reading the same buffer by
//! absolute frame number:
//!
//! - **Retrospective** ("sample what just happened"): [`RollingReader::copy`]
//!   any span that is still in the buffer, such as the last 10 seconds or the
//!   last 4 bars, cut exactly on the bar lines the transport gives.
//! - **Takes** ([`Take`]): manual, in time (starting on an exact frame such
//!   as the next bar and lasting an exact length) or threshold, where the
//!   pre-roll comes from the buffer so the attack is never lost. A take copies
//!   itself out of the buffer as it grows, so it is not limited to the
//!   buffer's length, only by how often the worker polls it.
//!
//! The buffer is shared memory of atomics (safe Rust, no locks). A reader
//! checks after each copy that the writer had not yet wrapped round over what
//! it copied, so a slow reader gets an error, never a mix of old and new audio.
//!
//! The line input reaches the sampler as an [`Input`] (step CAP): the audio
//! thread holds its [`Feed`] and, each period, marks the transport's bar
//! lines in the buffer's frames ([`BarWriter`]) and appends the input. The
//! screen thread holds the [`Input`] and makes [`Recording`]s from it: they
//! start now, on a threshold or on the next bar line, and end on a press,
//! on silence, after a length or on a bar line. [`Input::last_bars`] cuts
//! the last bars exactly on the bar lines.
//!
//! A recording that ends on a length or a bar line can let what is still
//! sounding ring out after it ([`Tail::Ring`], a one-shot with its reverb)
//! or be made a loop without a seam ([`Tail::Loop`]): printing N bars to a
//! pad.

use std::sync::atomic::{fence, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;

use crate::model::{ModelError, Source};

#[derive(Debug, Clone, PartialEq)]
pub enum RecordError {
    /// The span starts before the oldest frame still in the buffer, or the
    /// writer wrapped round over it during the copy.
    Overwritten,
    /// The span ends after the newest frame written.
    NotYetWritten,
    BadSpan,
    /// Fewer bar lines than asked for have passed (or the transport has
    /// not rolled since the input started).
    NoBarLines,
    Model(ModelError),
}

impl std::fmt::Display for RecordError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for RecordError {}

struct Ring {
    channels: usize,
    frames: u64,
    data: Box<[AtomicU32]>,
    /// Frames the writer has started to write (claimed), then finished.
    claimed: AtomicU64,
    written: AtomicU64,
    /// The first frame of what the buffer holds now: frames before it came
    /// from somewhere else ([`RollingWriter::restart`]) and are not read.
    start: AtomicU64,
    /// How many times the writer has restarted.
    restarts: AtomicU64,
}

/// A rolling buffer of `frames` frames of `channels` (1 or 2) interleaved
/// channels. Allocates once, here; the writer goes to the audio thread.
pub fn rolling(channels: u8, frames: usize) -> (RollingWriter, RollingReader) {
    assert!(
        (1..=2).contains(&channels) && frames > 0,
        "1 or 2 channels and a non-empty buffer"
    );
    let ch = usize::from(channels);
    let ring = Arc::new(Ring {
        channels: ch,
        frames: frames as u64,
        data: (0..frames * ch).map(|_| AtomicU32::new(0)).collect(),
        claimed: AtomicU64::new(0),
        written: AtomicU64::new(0),
        start: AtomicU64::new(0),
        restarts: AtomicU64::new(0),
    });
    (
        RollingWriter {
            ring: ring.clone(),
            pos: 0,
        },
        RollingReader { ring },
    )
}

/// The audio thread's end of a rolling buffer.
pub struct RollingWriter {
    ring: Arc<Ring>,
    pos: u64,
}

impl RollingWriter {
    /// Appends interleaved frames (a trailing partial frame is ignored).
    /// Never allocates, locks or waits.
    pub fn push(&mut self, interleaved: &[f32]) {
        let ch = self.ring.channels;
        let n = (interleaved.len() / ch) as u64;
        self.begin(n);
        for (i, frame) in interleaved.chunks_exact(ch).enumerate() {
            let base = ((self.pos + i as u64) % self.ring.frames) as usize * ch;
            for (c, x) in frame.iter().enumerate() {
                self.ring.data[base + c].store(x.to_bits(), Ordering::Relaxed);
            }
        }
        self.end(n);
    }

    /// Appends a period in the engine's form (separate left and right). A
    /// mono buffer stores their average.
    pub fn push_planar(&mut self, left: &[f32], right: &[f32]) {
        let n = left.len().min(right.len()) as u64;
        let ch = self.ring.channels;
        self.begin(n);
        for i in 0..n as usize {
            let base = ((self.pos + i as u64) % self.ring.frames) as usize * ch;
            if ch == 2 {
                self.ring.data[base].store(left[i].to_bits(), Ordering::Relaxed);
                self.ring.data[base + 1].store(right[i].to_bits(), Ordering::Relaxed);
            } else {
                self.ring.data[base]
                    .store((0.5 * (left[i] + right[i])).to_bits(), Ordering::Relaxed);
            }
        }
        self.end(n);
    }

    fn begin(&self, n: u64) {
        // Readers check `claimed` after copying: anything they copied that we
        // are about to overwrite is then seen as overwritten.
        self.ring.claimed.store(self.pos + n, Ordering::Relaxed);
        fence(Ordering::Release);
    }

    fn end(&mut self, n: u64) {
        self.pos += n;
        self.ring.written.store(self.pos, Ordering::Release);
    }

    /// Frames written since the start (the absolute frame number of the next one).
    pub fn written(&self) -> u64 {
        self.pos
    }

    /// What comes next is from another source: readers stop at it, as if
    /// everything before it had been overwritten. Never allocates, locks
    /// or waits.
    pub fn restart(&mut self) {
        self.ring.start.store(self.pos, Ordering::Release);
        self.ring.restarts.fetch_add(1, Ordering::Release);
    }
}

/// A reader of a rolling buffer, for any thread but the audio thread.
#[derive(Clone)]
pub struct RollingReader {
    ring: Arc<Ring>,
}

impl RollingReader {
    pub fn channels(&self) -> usize {
        self.ring.channels
    }
    pub fn capacity(&self) -> u64 {
        self.ring.frames
    }
    /// The absolute frame number the writer will write next.
    pub fn written(&self) -> u64 {
        self.ring.written.load(Ordering::Acquire)
    }
    /// How many times the writer has restarted ([`RollingWriter::restart`]):
    /// a reader that asked for a new source sees it has come when this
    /// moves.
    pub fn restarts(&self) -> u64 {
        self.ring.restarts.load(Ordering::Acquire)
    }
    /// The oldest frame still in the buffer (and from the source it has
    /// now).
    pub fn oldest(&self) -> u64 {
        let start = self.ring.start.load(Ordering::Acquire);
        self.written().saturating_sub(self.ring.frames).max(start)
    }

    /// Appends frames `start..end` (absolute) to `out`, interleaved. On an
    /// error `out` is left as it was.
    pub fn copy(&self, start: u64, end: u64, out: &mut Vec<f32>) -> Result<(), RecordError> {
        if start > end {
            return Err(RecordError::BadSpan);
        }
        let written = self.written();
        if end > written {
            return Err(RecordError::NotYetWritten);
        }
        if start + self.ring.frames < written || start < self.ring.start.load(Ordering::Acquire) {
            return Err(RecordError::Overwritten);
        }
        let ch = self.ring.channels;
        let keep = out.len();
        out.reserve(((end - start) as usize) * ch);
        for f in start..end {
            let base = (f % self.ring.frames) as usize * ch;
            for c in 0..ch {
                out.push(f32::from_bits(
                    self.ring.data[base + c].load(Ordering::Relaxed),
                ));
            }
        }
        // Did the writer start to overwrite any of it while we copied?
        fence(Ordering::Acquire);
        if start + self.ring.frames < self.ring.claimed.load(Ordering::Relaxed)
            || start < self.ring.start.load(Ordering::Relaxed)
        {
            out.truncate(keep);
            return Err(RecordError::Overwritten);
        }
        Ok(())
    }

    /// The last `frames` frames (or as many as the buffer holds).
    pub fn skip_back(&self, frames: u64) -> Result<Vec<f32>, RecordError> {
        let end = self.written();
        let start = end.saturating_sub(frames).max(self.oldest());
        let mut out = Vec::new();
        self.copy(start, end, &mut out)?;
        Ok(out)
    }
}

/// How a take starts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Arm {
    /// At an exact frame: now (manual record) or the next bar line (in time).
    At(u64),
    /// When any channel goes above `level` (linear peak), starting `pre_roll`
    /// frames earlier, after the frame it was armed at.
    Threshold {
        level: f32,
        pre_roll: u64,
        armed_at: u64,
    },
}

/// How a take stops, besides [`Take::stop_at`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Stop {
    /// Only when told.
    Manual,
    /// After exactly this many frames (an in-time take's bars).
    Length(u64),
    /// After `frames` frames below `level`; the take ends after the last loud frame.
    Silence { level: f32, frames: u64 },
}

/// Ringing out after a take's cut ([`Take::ring_out`]): it goes on until it
/// has been at or under `level` (linear peak) for `quiet` frames, and
/// ends after the last frame over it; at most `max` frames past the cut.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RingOut {
    pub level: f32,
    pub quiet: u64,
    pub max: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TakeState {
    /// Armed, not started (threshold not crossed, or the start frame not reached).
    Waiting,
    Recording {
        start: u64,
        frames: u64,
    },
    Done {
        start: u64,
        end: u64,
    },
}

/// A recording in progress. Lives on a worker, which calls [`Take::poll`]
/// regularly (every 10-50 ms is plenty; the buffer holds minutes).
#[derive(Clone, Debug)]
pub struct Take {
    arm: Arm,
    stop: Stop,
    /// Hard limit on the take's length, so a forgotten take can't fill RAM.
    max_frames: u64,
    channels: usize,
    start: Option<u64>,
    /// Where its own length ends (its length, its bar line or a press);
    /// with a ring it goes on past it.
    cut: Option<u64>,
    end: Option<u64>,
    ring: Option<RingOut>,
    /// The ringing stops after one more of the take's own length too.
    once_round: bool,
    /// Absolute frame up to which the take (or the threshold scan) has read.
    read_to: u64,
    last_loud: Option<u64>,
    /// The ring's scan: read up to here, and its last frame over the level.
    rung_to: u64,
    ring_loud: Option<u64>,
    data: Vec<f32>,
    scratch: Vec<f32>,
}

impl Take {
    pub fn new(reader: &RollingReader, arm: Arm, stop: Stop, max_frames: u64) -> Take {
        let read_to = match arm {
            Arm::At(f) => f,
            Arm::Threshold { armed_at, .. } => armed_at,
        };
        Take {
            arm,
            stop,
            max_frames: max_frames.max(1),
            channels: reader.channels(),
            start: None,
            cut: None,
            end: None,
            ring: None,
            once_round: false,
            read_to,
            last_loud: None,
            rung_to: 0,
            ring_loud: None,
            data: Vec::new(),
            scratch: Vec::new(),
        }
    }

    /// The take rings out after its cut (its length, or [`Take::cut_at`])
    /// as `ring` says, instead of ending there. A [`Stop::Silence`] take
    /// already ends after its sound, and [`Take::stop_at`] ends any take
    /// at once.
    pub fn ring_out(mut self, ring: RingOut) -> Take {
        if !matches!(self.stop, Stop::Silence { .. }) {
            self.ring = Some(ring);
        }
        self
    }

    /// [`Take::ring_out`], ending after one more of the take's own length
    /// (its start to its cut) at most, however long that is.
    pub fn ring_out_once_round(mut self, level: f32, quiet: u64) -> Take {
        self.once_round = true;
        self.ring_out(RingOut {
            level,
            quiet,
            max: u64::MAX,
        })
    }

    /// No ringing after all: a take past its cut ends on it (the next
    /// poll drops what came after).
    pub fn ring_off(&mut self) {
        self.ring = None;
        if self.end.is_none() {
            self.end = self.cut;
        }
    }

    /// Stops the take at `frame` (for example the frame of the button
    /// press), ringing out or not, and before a length it had not reached.
    pub fn stop_at(&mut self, frame: u64) {
        let start = *self.start.get_or_insert(self.start_by(frame));
        let end = frame.max(start);
        if self.end.is_none_or(|e| end < e) {
            self.end = Some(end);
            if self.cut.is_none_or(|c| end < c) {
                self.cut = Some(end);
            }
        }
    }

    /// Where a take not yet polled since its start began, as of `frame`:
    /// its start frame once that has passed, else `frame` (nothing).
    fn start_by(&self, frame: u64) -> u64 {
        match (self.start, self.arm) {
            (Some(s), _) => s,
            (None, Arm::At(f)) if f <= frame => f,
            _ => frame,
        }
    }

    /// The take's own length ends at `frame` (a bar line): it ends there,
    /// or rings out after it. Only the first cut counts.
    pub fn cut_at(&mut self, frame: u64) {
        if self.end.is_some() || self.cut.is_some() {
            return;
        }
        if self.ring.is_none() {
            self.stop_at(frame);
            return;
        }
        let start = *self.start.get_or_insert(self.start_by(frame));
        self.cut = Some(frame.max(start));
    }

    /// Where its own length ended, once known: the frames after it (up to
    /// the end) are the ringing.
    pub fn cut(&self) -> Option<u64> {
        self.cut
    }

    pub fn state(&self) -> TakeState {
        match (self.start, self.end) {
            (Some(start), Some(end)) if self.read_to >= end => TakeState::Done { start, end },
            (Some(start), _) => TakeState::Recording {
                start,
                frames: self.data.len() as u64 / self.channels as u64,
            },
            _ => TakeState::Waiting,
        }
    }

    /// Copies whatever has been recorded since the last poll.
    pub fn poll(&mut self, r: &RollingReader) -> Result<TakeState, RecordError> {
        let written = r.written();
        if self.start.is_none() {
            match self.arm {
                Arm::At(f) => {
                    if written > f {
                        self.start = Some(f);
                    }
                }
                Arm::Threshold {
                    level, pre_roll, ..
                } => {
                    let from = self.read_to.max(r.oldest());
                    if from >= written {
                        return Ok(TakeState::Waiting);
                    }
                    self.scratch.clear();
                    r.copy(from, written, &mut self.scratch)?;
                    if let Some(i) = self
                        .scratch
                        .chunks_exact(self.channels)
                        .position(|f| f.iter().any(|x| x.abs() > level))
                    {
                        let onset = from + i as u64;
                        // The pre-roll may reach back before arming, but not
                        // before what the buffer still holds.
                        self.start = Some(onset.saturating_sub(pre_roll).max(r.oldest()));
                    } else {
                        self.read_to = written;
                    }
                }
            }
            if let Some(s) = self.start {
                self.read_to = s;
            }
        }
        let Some(start) = self.start else {
            return Ok(TakeState::Waiting);
        };
        if let Stop::Length(n) = self.stop {
            self.cut_at(start + n);
        }
        let limit = start + self.max_frames;
        let until = written.min(self.end.unwrap_or(u64::MAX)).min(limit);
        if until > self.read_to {
            let from_len = self.data.len();
            r.copy(self.read_to, until, &mut self.data)?;
            if let Stop::Silence { level, frames } = self.stop {
                for (i, f) in self.data[from_len..]
                    .chunks_exact(self.channels)
                    .enumerate()
                {
                    if f.iter().any(|x| x.abs() > level) {
                        self.last_loud = Some(self.read_to + i as u64);
                    }
                }
                let quiet_from = self.last_loud.map_or(start, |l| l + 1);
                if self.end.is_none() && until >= quiet_from + frames {
                    self.end = Some(quiet_from);
                }
            }
            self.read_to = until;
        }
        self.scan_ring(start);
        if self.end.is_none() && self.read_to >= limit {
            self.end = Some(limit);
        }
        // A silence stop ends earlier than what was copied: drop the rest.
        if let Some(end) = self.end {
            let keep = ((end.saturating_sub(start)) as usize) * self.channels;
            self.data.truncate(keep);
        }
        Ok(self.state())
    }

    /// After the cut, scans what came in for the ringing's end.
    fn scan_ring(&mut self, start: u64) {
        let (Some(ring), Some(cut), None) = (self.ring, self.cut, self.end) else {
            return;
        };
        let from = self.rung_to.max(cut);
        if self.read_to > from {
            let ch = self.channels;
            let span = (from - start) as usize * ch..(self.read_to - start) as usize * ch;
            for (i, f) in self.data[span].chunks_exact(ch).enumerate() {
                if f.iter().any(|x| x.abs() > ring.level) {
                    self.ring_loud = Some(from + i as u64);
                }
            }
            self.rung_to = self.read_to;
        }
        let most = if self.once_round {
            ring.max.min(cut - start)
        } else {
            ring.max
        };
        let quiet_from = self.ring_loud.map_or(cut, |l| l + 1);
        if self.read_to >= quiet_from + ring.quiet {
            self.end = Some(quiet_from);
        } else if self.read_to >= cut.saturating_add(most) {
            self.end = Some(quiet_from.min(cut + most));
        }
    }

    /// The finished take as a source. None until it is done.
    pub fn into_source(self, name: &str, rate: u32) -> Option<Result<Source, RecordError>> {
        match self.state() {
            TakeState::Done { .. } => Some(
                Source::new(name, rate, self.channels as u8, self.data).map_err(RecordError::Model),
            ),
            _ => None,
        }
    }

    /// The audio copied so far, interleaved.
    pub fn data(&self) -> &[f32] {
        &self.data
    }
}

// ---------------------------------------------------------------- bar lines

/// Bar lines a [`BarReader`] can look back over: more than a minute of bars
/// at any tempo.
pub const BAR_LINES: usize = 64;

struct Bars {
    at: [AtomicU64; BAR_LINES],
    /// Lines the writer has started to write (claimed), then finished, as
    /// [`Ring`]'s frames.
    claimed: AtomicU64,
    count: AtomicU64,
    /// A bar's length in frames at the tempo now (f64 bits); 0 while the
    /// transport is stopped.
    bar: AtomicU64,
}

/// The transport's bar lines in a rolling buffer's frames: the audio
/// thread marks them as they pass, the screen thread reads them to start a
/// take on a bar line or to cut the last bars. Allocates once, here.
pub fn bar_lines() -> (BarWriter, BarReader) {
    let bars = Arc::new(Bars {
        at: std::array::from_fn(|_| AtomicU64::new(0)),
        claimed: AtomicU64::new(0),
        count: AtomicU64::new(0),
        bar: AtomicU64::new(0),
    });
    (
        BarWriter {
            bars: bars.clone(),
            count: 0,
        },
        BarReader { bars },
    )
}

/// The audio thread's end of [`bar_lines`].
pub struct BarWriter {
    bars: Arc<Bars>,
    count: u64,
}

impl BarWriter {
    /// A bar line at absolute frame `frame` of the rolling buffer the lines
    /// go with. Lines come in order. Never allocates, locks or waits.
    pub fn mark(&mut self, frame: u64) {
        let b = &self.bars;
        b.claimed.store(self.count + 1, Ordering::Relaxed);
        fence(Ordering::Release);
        b.at[(self.count % BAR_LINES as u64) as usize].store(frame, Ordering::Relaxed);
        self.count += 1;
        b.count.store(self.count, Ordering::Release);
    }

    /// A bar's length in frames at the tempo now; None while stopped.
    pub fn set_bar(&mut self, frames: Option<f64>) {
        let bits = frames
            .filter(|f| f.is_finite() && *f > 0.0)
            .map_or(0, f64::to_bits);
        self.bars.bar.store(bits, Ordering::Relaxed);
    }
}

/// A reader of [`bar_lines`], for any thread but the audio thread.
#[derive(Clone)]
pub struct BarReader {
    bars: Arc<Bars>,
}

impl BarReader {
    /// Bar lines marked since the start.
    pub fn count(&self) -> u64 {
        self.bars.count.load(Ordering::Acquire)
    }

    /// The `n`-th latest bar line (0 is the latest), if it is still kept.
    pub fn back(&self, n: usize) -> Option<u64> {
        let count = self.count();
        if n >= BAR_LINES || n as u64 >= count {
            return None;
        }
        let i = count - 1 - n as u64;
        let frame = self.bars.at[(i % BAR_LINES as u64) as usize].load(Ordering::Relaxed);
        // Did the writer start to write over it while we read?
        fence(Ordering::Acquire);
        let claimed = self.bars.claimed.load(Ordering::Relaxed);
        (claimed <= i + BAR_LINES as u64).then_some(frame)
    }

    /// The `k`-th bar line after frame `frame` (`k` = 1 is the first one
    /// strictly later), if it has passed and is still kept.
    pub fn nth_after(&self, frame: u64, k: usize) -> Option<u64> {
        if k == 0 {
            return None;
        }
        // Count back from the latest line to the first one after `frame`;
        // if every line kept is later, older ones may be too, and which is
        // the first is not known any more.
        let mut first = None;
        let mut found = false;
        for n in 0..BAR_LINES {
            match self.back(n) {
                Some(f) if f > frame => first = Some(n),
                Some(_) => {
                    found = true;
                    break;
                }
                None => {
                    found = n as u64 >= self.count();
                    break;
                }
            }
        }
        let first = first.filter(|_| found)?;
        first.checked_sub(k - 1).and_then(|n| self.back(n))
    }

    /// A bar's length in frames at the tempo now; None while stopped.
    pub fn bar_frames(&self) -> Option<f64> {
        let bits = self.bars.bar.load(Ordering::Relaxed);
        (bits != 0).then(|| f64::from_bits(bits))
    }
}

// ---------------------------------------------------------------- the input

/// The audio thread's end of an [`Input`]: the rolling buffer's writer and
/// the bar lines.
pub struct Feed {
    pub audio: RollingWriter,
    pub bars: BarWriter,
}

impl Feed {
    /// The input comes from another source from now on: Skip back and
    /// takes stop at this frame ([`RollingWriter::restart`]).
    pub fn restart(&mut self) {
        self.audio.restart();
    }
}

/// What the sampler records from (the line input, or a point in the mix
/// when resampling): the last `seconds` of stereo input and the bar lines
/// in it. The screen thread keeps this end.
#[derive(Clone)]
pub struct Input {
    pub audio: RollingReader,
    pub bars: BarReader,
    pub rate: u32,
}

/// A stereo rolling buffer of `seconds` at `rate`, with its bar lines.
/// Allocates once, here (about 21 MB a minute).
pub fn input(seconds: u32, rate: u32) -> (Feed, Input) {
    let (audio_w, audio) = rolling(2, (seconds.max(1) as usize) * rate as usize);
    let (bars_w, bars) = bar_lines();
    (
        Feed {
            audio: audio_w,
            bars: bars_w,
        },
        Input { audio, bars, rate },
    )
}

impl Input {
    /// The frame the writer will write next: "now".
    pub fn now(&self) -> u64 {
        self.audio.written()
    }

    /// Frames there are to skip back over.
    pub fn held(&self) -> u64 {
        self.now() - self.audio.oldest()
    }

    /// The last `frames` frames (or as many as the buffer holds), stereo.
    pub fn last_frames(&self, frames: u64) -> Result<Vec<f32>, RecordError> {
        self.audio.skip_back(frames)
    }

    /// The last `n` whole bars, from the bar line `n` bars before the
    /// latest one that has come in to that one, stereo. (A bar line can be
    /// marked ahead of the audio: an input heard through effects with
    /// latency gets its lines that much later.)
    pub fn last_bars(&self, n: usize) -> Result<Vec<f32>, RecordError> {
        if n == 0 {
            return Err(RecordError::BadSpan);
        }
        let (start, end) = self.bars_back(n).ok_or(RecordError::NoBarLines)?;
        let mut out = Vec::new();
        self.audio.copy(start, end, &mut out)?;
        Ok(out)
    }

    /// The span of the last `n` whole bars that have come in: from the bar
    /// line `n` before the latest one at or before now, to that one.
    pub fn bars_back(&self, n: usize) -> Option<(u64, u64)> {
        let now = self.now();
        let latest = (0..BAR_LINES).find(|k| self.bars.back(*k).is_some_and(|f| f <= now))?;
        Some((self.bars.back(latest + n)?, self.bars.back(latest)?))
    }
}

/// Which of the input's channels a recording keeps.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Pick {
    /// Both, as a stereo sound.
    #[default]
    Stereo,
    /// The left jack only, as a mono sound.
    Left,
    /// The right jack only, as a mono sound.
    Right,
    /// Both mixed to one, as a mono sound.
    Mono,
}

impl Pick {
    pub const ALL: [Pick; 4] = [Pick::Stereo, Pick::Left, Pick::Right, Pick::Mono];

    pub fn label(self) -> &'static str {
        match self {
            Pick::Stereo => "Left + right",
            Pick::Left => "Left",
            Pick::Right => "Right",
            Pick::Mono => "Mono mix",
        }
    }

    pub fn channels(self) -> u8 {
        if self == Pick::Stereo {
            2
        } else {
            1
        }
    }

    /// Stereo interleaved `data` as this pick keeps it.
    pub fn apply(self, data: Vec<f32>) -> Vec<f32> {
        let frames = data.chunks_exact(2);
        match self {
            Pick::Stereo => data,
            Pick::Left => frames.map(|f| f[0]).collect(),
            Pick::Right => frames.map(|f| f[1]).collect(),
            Pick::Mono => frames.map(|f| 0.5 * (f[0] + f[1])).collect(),
        }
    }

    /// A source named `name` from stereo interleaved `data`.
    pub fn source(self, name: &str, rate: u32, data: Vec<f32>) -> Result<Source, RecordError> {
        Source::new(name, rate, self.channels(), self.apply(data)).map_err(RecordError::Model)
    }
}

// ---------------------------------------------------------------- recordings

/// How a [`Recording`] starts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Start {
    /// When it is asked for.
    Now,
    /// When either channel goes above `level` (linear peak), keeping
    /// `pre_roll` frames from before.
    Threshold { level: f32, pre_roll: u64 },
    /// On the next bar line (the transport must be rolling).
    NextBar,
}

/// How a [`Recording`] ends, besides [`Recording::stop`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum End {
    /// Only when stopped.
    Press,
    /// After `frames` frames below `level`; it ends after the last loud frame.
    Silence { level: f32, frames: u64 },
    /// After exactly this many frames.
    Frames(u64),
    /// On the `n`-th bar line after its start (a recording that starts on
    /// a bar line); one that starts otherwise lasts `n` bars at the tempo
    /// it starts at.
    Bars(u32),
}

/// What a [`Recording`] that ends on a length or a bar line ([`End::Frames`],
/// [`End::Bars`]) does after it. A press ([`Recording::stop`]) ends any
/// recording at once.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Tail {
    /// It ends there.
    #[default]
    Cut,
    /// It rings out ([`RingOut`]): a one-shot with its reverb or echo.
    Ring(RingOut),
    /// It is made a loop of exactly its length, without a seam ([`Looped`]
    /// says how). "Sounding" is over `level` (linear peak); "silent" is
    /// at or under it for `quiet` frames.
    Loop { level: f32, quiet: u64 },
}

/// How a [`Tail::Loop`] recording was made a loop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Looped {
    /// Its source was sounding in the quiet time before it started: the
    /// music was going round, so its start already holds the ringing of
    /// the time before and it is cut on its length.
    GoingRound,
    /// It started from silence and was silent within one more length:
    /// the ringing after its length (`frames` of it) is added onto its
    /// start, where it sounds as the loop comes round.
    Folded { frames: u64 },
    /// It started from silence, but still sounded a whole length on: the
    /// music went on round, so the take is that second time round, which
    /// starts with the first's ringing.
    SecondTime,
}

/// Where a [`Recording`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Progress {
    /// Waiting for the threshold or the bar line.
    Waiting,
    /// Recording, with `frames` copied so far.
    Recording { frames: u64 },
    /// Finished: [`Recording::finish`] gives the sound.
    Done { frames: u64 },
    /// Stopped before anything was recorded.
    Nothing,
}

/// A recording from an [`Input`], made and polled on the screen thread (or
/// a worker) every few tens of milliseconds; the buffer holds minutes, so
/// a late poll loses nothing.
#[derive(Clone, Debug)]
pub struct Recording {
    start: Start,
    end: End,
    tail: Tail,
    /// Whether its source was sounding just before it started (for
    /// [`Tail::Loop`]), once it has.
    rang_in: Option<bool>,
    /// The frame it was asked for at.
    armed_at: u64,
    max_frames: u64,
    rate: u32,
    take: Option<Take>,
    /// The bar line it started on (a [`Start::NextBar`] recording).
    bar0: Option<u64>,
    cancelled: bool,
}

impl Recording {
    /// A recording asked for now, at most `max_frames` long.
    pub fn new(input: &Input, start: Start, end: End, max_frames: u64) -> Recording {
        Recording::with_tail(input, start, end, Tail::Cut, max_frames)
    }

    /// [`Recording::new`], doing `tail` after its length or bar line. Its
    /// ringing counts in `max_frames`.
    pub fn with_tail(
        input: &Input,
        start: Start,
        end: End,
        tail: Tail,
        max_frames: u64,
    ) -> Recording {
        let armed_at = input.now();
        let mut r = Recording {
            start,
            end,
            tail,
            rang_in: None,
            armed_at,
            max_frames: max_frames.max(1),
            rate: input.rate,
            take: None,
            bar0: None,
            cancelled: false,
        };
        match start {
            Start::Now => r.begin(input, Arm::At(armed_at)),
            Start::Threshold { level, pre_roll } => r.begin(
                input,
                Arm::Threshold {
                    level,
                    pre_roll,
                    armed_at,
                },
            ),
            Start::NextBar => {}
        }
        r
    }

    fn begin(&mut self, input: &Input, arm: Arm) {
        let stop = match self.end {
            End::Press => Stop::Manual,
            End::Silence { level, frames } => Stop::Silence { level, frames },
            End::Frames(n) => Stop::Length(n.max(1)),
            // On bar lines: stopped by `poll` when the last one passes.
            End::Bars(_) if self.start == Start::NextBar => Stop::Manual,
            End::Bars(n) => {
                let bar = input.bars.bar_frames().unwrap_or(0.0);
                if bar > 0.0 {
                    Stop::Length(((f64::from(n.max(1)) * bar).round() as u64).max(1))
                } else {
                    Stop::Manual
                }
            }
        };
        let take = Take::new(&input.audio, arm, stop, self.max_frames);
        self.take = Some(match (self.end, self.tail) {
            (End::Press | End::Silence { .. }, _) | (_, Tail::Cut) => take,
            (_, Tail::Ring(ring)) => take.ring_out(ring),
            (_, Tail::Loop { level, quiet }) => take.ring_out_once_round(level, quiet),
        });
    }

    pub fn start(&self) -> Start {
        self.start
    }

    pub fn end(&self) -> End {
        self.end
    }

    pub fn tail(&self) -> Tail {
        self.tail
    }

    /// The input's sample rate.
    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// Frames up to the end of its length or bars, once known; the rest
    /// of what it records is the ringing.
    pub fn cut_frames(&self) -> Option<u64> {
        let t = self.take.as_ref()?;
        Some(t.cut()?.saturating_sub(self.started_at()?))
    }

    /// How a [`Tail::Loop`] recording is made a loop, once it is done.
    pub fn looped(&self) -> Option<Looped> {
        let Tail::Loop { .. } = self.tail else {
            return None;
        };
        let Progress::Done { frames } = self.progress() else {
            return None;
        };
        let cut = self.cut_frames()?;
        Some(match self.rang_in {
            Some(true) => Looped::GoingRound,
            _ if cut > 0 && frames >= 2 * cut => Looped::SecondTime,
            _ => Looped::Folded {
                frames: frames.saturating_sub(cut),
            },
        })
    }

    /// The frame it was asked for at.
    pub fn armed_at(&self) -> u64 {
        self.armed_at
    }

    /// The frame its sound starts at, once it has started.
    pub fn started_at(&self) -> Option<u64> {
        match self.take.as_ref()?.state() {
            TakeState::Waiting => None,
            TakeState::Recording { start, .. } | TakeState::Done { start, .. } => Some(start),
        }
    }

    /// Where it is now, without copying anything.
    pub fn progress(&self) -> Progress {
        if self.cancelled {
            return Progress::Nothing;
        }
        match self.take.as_ref().map(Take::state) {
            None | Some(TakeState::Waiting) => Progress::Waiting,
            Some(TakeState::Recording { frames, .. }) => Progress::Recording { frames },
            Some(TakeState::Done { start, end }) if end > start => Progress::Done {
                frames: end - start,
            },
            Some(TakeState::Done { .. }) => Progress::Nothing,
        }
    }

    /// Copies whatever has been recorded since the last poll; starts on
    /// the bar line or ends on it.
    pub fn poll(&mut self, input: &Input) -> Result<Progress, RecordError> {
        if self.cancelled {
            return Ok(Progress::Nothing);
        }
        if self.take.is_none() {
            // Waiting for the next bar line.
            let Some(bar) = input.bars.nth_after(self.armed_at, 1) else {
                return Ok(Progress::Waiting);
            };
            self.bar0 = Some(bar);
            self.begin(input, Arm::At(bar));
        }
        if let (End::Bars(n), Some(bar0)) = (self.end, self.bar0) {
            if let Some(last) = input.bars.nth_after(bar0, n.max(1) as usize) {
                if let Some(t) = self.take.as_mut() {
                    t.cut_at(last);
                }
            }
        }
        if let Some(t) = self.take.as_mut() {
            t.poll(&input.audio)?;
        }
        if let (Tail::Loop { level, quiet }, None, Some(start)) =
            (self.tail, self.rang_in, self.started_at())
        {
            let going_round = sounding_before(input, start, level, quiet);
            self.rang_in = Some(going_round);
            if let (true, Some(t)) = (going_round, self.take.as_mut()) {
                // Cut on its length: nothing to wait for after it.
                t.ring_off();
                t.poll(&input.audio)?;
            }
        }
        Ok(self.progress())
    }

    /// Stops it now: a recording that has started ends here; one still
    /// waiting records nothing.
    pub fn stop(&mut self, input: &Input) {
        match self.progress() {
            Progress::Waiting => self.cancelled = true,
            Progress::Recording { .. } => {
                if let Some(t) = self.take.as_mut() {
                    t.stop_at(input.now());
                }
            }
            Progress::Done { .. } | Progress::Nothing => {}
        }
    }

    /// The audio copied so far, stereo interleaved.
    pub fn data(&self) -> &[f32] {
        self.take.as_ref().map_or(&[], Take::data)
    }

    /// The finished recording as a source named `name`, with the channels
    /// picked. None until it is done (or if it recorded nothing).
    pub fn finish(self, name: &str, pick: Pick) -> Option<Result<Source, RecordError>> {
        if !matches!(self.progress(), Progress::Done { .. }) {
            return None;
        }
        let looped = self.looped();
        let cut = self.cut_frames().unwrap_or(0) as usize;
        let take = self.take?;
        let n = cut * take.channels;
        let mut data = take.data;
        match looped {
            None | Some(Looped::GoingRound) => {}
            Some(Looped::SecondTime) => {
                data.drain(..n);
            }
            Some(Looped::Folded { .. }) => fold(&mut data, n),
        }
        if looped.is_some() {
            data.truncate(n);
        }
        Some(pick.source(name, self.rate, data))
    }
}

/// Whether `input` went over `level` in the `quiet` frames before frame
/// `start` (what the buffer still holds of them).
fn sounding_before(input: &Input, start: u64, level: f32, quiet: u64) -> bool {
    let from = start.saturating_sub(quiet).max(input.audio.oldest());
    let mut before = Vec::new();
    from < start
        && input.audio.copy(from, start, &mut before).is_ok()
        && before.iter().any(|x| x.abs() > level)
}

/// Adds everything in `data` after its first `n` samples onto them, round
/// and round (a tail longer than the loop goes round again).
fn fold(data: &mut [f32], n: usize) {
    if n == 0 {
        return;
    }
    for i in n..data.len() {
        let x = data[i];
        data[(i - n) % n] += x;
    }
}
