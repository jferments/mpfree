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
    /// The oldest frame still in the buffer.
    pub fn oldest(&self) -> u64 {
        self.written().saturating_sub(self.ring.frames)
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
        if start + self.ring.frames < written {
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
        if start + self.ring.frames < self.ring.claimed.load(Ordering::Relaxed) {
            out.truncate(keep);
            return Err(RecordError::Overwritten);
        }
        Ok(())
    }

    /// The last `frames` frames (or as many as the buffer holds).
    pub fn skip_back(&self, frames: u64) -> Result<Vec<f32>, RecordError> {
        let end = self.written();
        let start = end
            .saturating_sub(frames)
            .max(end.saturating_sub(self.ring.frames));
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
    end: Option<u64>,
    /// Absolute frame up to which the take (or the threshold scan) has read.
    read_to: u64,
    last_loud: Option<u64>,
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
            end: None,
            read_to,
            last_loud: None,
            data: Vec::new(),
            scratch: Vec::new(),
        }
    }

    /// Stops a manual take at `frame` (for example the frame of the button press).
    pub fn stop_at(&mut self, frame: u64) {
        if self.end.is_none() {
            let start = self.start.unwrap_or(frame);
            self.end = Some(frame.max(start));
            if self.start.is_none() {
                self.start = Some(start);
            }
        }
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
        if self.end.is_none() {
            match self.stop {
                Stop::Length(n) => self.end = Some(start + n),
                Stop::Manual | Stop::Silence { .. } => {}
            }
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

/// The line input as the sampler records it: the last `seconds` of stereo
/// input and the bar lines in it. The screen thread keeps this end.
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
    /// latest one to the latest one, stereo.
    pub fn last_bars(&self, n: usize) -> Result<Vec<f32>, RecordError> {
        let (Some(end), Some(start)) = (self.bars.back(0), self.bars.back(n)) else {
            return Err(RecordError::NoBarLines);
        };
        if n == 0 {
            return Err(RecordError::BadSpan);
        }
        let mut out = Vec::new();
        self.audio.copy(start, end, &mut out)?;
        Ok(out)
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
        let armed_at = input.now();
        let mut r = Recording {
            start,
            end,
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
        self.take = Some(Take::new(&input.audio, arm, stop, self.max_frames));
    }

    pub fn start(&self) -> Start {
        self.start
    }

    pub fn end(&self) -> End {
        self.end
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
                    t.stop_at(last);
                }
            }
        }
        if let Some(t) = self.take.as_mut() {
            t.poll(&input.audio)?;
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
        let take = self.take?;
        Some(pick.source(name, self.rate, take.data))
    }
}
