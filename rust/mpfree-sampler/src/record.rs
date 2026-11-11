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
