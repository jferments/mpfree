//! Freeze and print: a strip recorded as it sounds, then played in place
//! of its instrument and inserts, or handed out as a sample.
//!
//! - [`Print`] records a mixer capture point ([`crate::Mixer::set_capture`])
//!   from a song beat for a number of beats, sample for sample, lined up
//!   for the capture point's latency. It records into memory made off the
//!   audio thread ([`Print::new`]); arming, recording and finishing never
//!   allocate. With [`PrintJob::steady`] it waits until the transport has
//!   rolled a whole loop before the take starts, so the tails of the time
//!   round before (a reverb, an echo) are already in its first frames and
//!   the take loops without a seam. With [`Tail::UntilSilent`] it goes on
//!   past the beats until the strip falls silent (a one-shot sample with
//!   its reverb).
//! - [`Printed`] is the take. [`Printed::wav`] makes a WAV file of it for a
//!   pad or the library; [`Printed::frozen`] makes it a [`Frozen`] loop.
//! - [`Frozen`] is audio a strip plays in place of its inserts
//!   ([`crate::Mixer::freeze`]), in step with the song: its first frame is
//!   on the song beat [`Frozen::origin`], and it repeats every
//!   [`Frozen::beats`]. At another tempo it follows the beat (it plays
//!   faster or slower, like a tape).
//!
//! The take is real time: freezing a four-bar loop at 120 BPM takes eight
//! seconds, sixteen when the loop has to play once first. It costs one
//! copy per frame while it records.

use mpfree_node::Ctx;

/// Below this (about -90 dBFS) the tail of a take counts as silent.
pub const SILENT: f32 = 3.2e-5;
/// A tail ends after this long below [`SILENT`].
pub const QUIET_MS: f32 = 100.0;

/// What a take does after its beats.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Tail {
    /// It stops at the end of the beats (a loop).
    Cut,
    /// It goes on until the strip is silent for [`QUIET_MS`], at most
    /// `max_ms` (a one-shot with its reverb). The transport may stop.
    UntilSilent { max_ms: f32 },
}

/// What to record.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PrintJob {
    /// The song beat the take starts on. A beat already gone moves on by
    /// whole `beats`, to the next time round of the loop.
    pub start: f64,
    /// How long, in beats (a clip's length).
    pub beats: f64,
    /// Wait until the transport has rolled for a whole `beats` before the
    /// take starts (one more time round when it has not), so the take
    /// loops without a seam. For freezing.
    pub steady: bool,
    pub tail: Tail,
}

impl PrintJob {
    /// A loop of `beats` from song beat `start`, steady, cut at its end:
    /// what freezing a clip needs.
    pub fn freeze(start: f64, beats: f64) -> PrintJob {
        PrintJob {
            start,
            beats,
            steady: true,
            tail: Tail::Cut,
        }
    }
}

/// Where a [`Print`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrintState {
    /// Nothing asked, or the take was handed out.
    Idle,
    /// Armed, waiting for the transport or the start beat.
    Waiting,
    Recording,
    /// Past the beats, waiting for the strip to fall silent.
    Tail,
    /// The take is ready ([`Print::take`]).
    Done,
    Failed(PrintError),
}

/// Why a take failed. What was recorded is dropped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrintError {
    /// The transport stopped before the beats were in.
    Stopped,
    /// The transport jumped (a locate or a song loop) while recording.
    Jumped,
    /// The tempo changed while recording.
    TempoChanged,
    /// The take would not fit in the memory the print was made with.
    TooLong,
    /// The capture point gave nothing (it was freed or moved).
    NoSignal,
    /// Arming needs an idle print with its memory, and beats over zero.
    Refused,
}

/// Records one take from a capture point. See the module docs.
///
/// Call [`Print::process`] every block, after the mixer, with the capture
/// point's block and latency, whether or not a take is armed (it keeps
/// track of how long the transport has rolled, for
/// [`PrintJob::steady`]).
pub struct Print {
    buf: [Vec<f32>; 2],
    n: usize,
    state: PrintState,
    job: PrintJob,
    rate: u32,
    /// The tempo the take is recorded at.
    bpm: f64,
    /// The capture point's frame (on the `Ctx::frame` clock) of the take's
    /// first frame, once the transport rolls.
    first: Option<u64>,
    /// The beat the take starts on, once worked out.
    start: f64,
    loop_frames: usize,
    tail_max: usize,
    quiet: usize,
    quiet_frames: usize,
    /// The beat the transport has rolled from without a break.
    rolling_since: Option<f64>,
    /// The frame and beat the next block starts on while rolling.
    expect: Option<(u64, f64)>,
}

impl Print {
    /// A print that can hold `max_seconds` at `rate`. Allocates: make it
    /// off the audio thread.
    pub fn new(rate: u32, max_seconds: f32) -> Print {
        let rate = rate.max(1);
        let cap = (max_seconds.max(0.0) * rate as f32) as usize;
        Print {
            buf: [vec![0.0; cap], vec![0.0; cap]],
            n: 0,
            state: PrintState::Idle,
            job: PrintJob::freeze(0.0, 4.0),
            rate,
            bpm: 120.0,
            first: None,
            start: 0.0,
            loop_frames: 0,
            tail_max: 0,
            quiet: 0,
            quiet_frames: (QUIET_MS * 0.001 * rate as f32) as usize,
            rolling_since: None,
            expect: None,
        }
    }

    /// Frames the print can hold.
    pub fn capacity(&self) -> usize {
        self.buf[0].len()
    }

    pub fn state(&self) -> PrintState {
        self.state
    }

    /// Arms a take. Real-time safe. Refused unless the print is idle (or
    /// done, failed or cancelled) and still has its memory.
    pub fn arm(&mut self, job: PrintJob) -> Result<(), PrintError> {
        let ready = !matches!(
            self.state,
            PrintState::Waiting | PrintState::Recording | PrintState::Tail
        );
        if !ready
            || self.capacity() == 0
            || job.beats.is_nan()
            || job.beats <= 0.0
            || !job.start.is_finite()
            || !job.beats.is_finite()
        {
            return Err(PrintError::Refused);
        }
        self.job = job;
        self.state = PrintState::Waiting;
        self.n = 0;
        self.first = None;
        self.quiet = 0;
        Ok(())
    }

    /// Drops the take armed or under way; the memory stays for another.
    pub fn cancel(&mut self) {
        if matches!(
            self.state,
            PrintState::Waiting | PrintState::Recording | PrintState::Tail
        ) {
            self.state = PrintState::Idle;
        }
        self.n = 0;
        self.first = None;
    }

    /// How far the take is, 0 to 1 (1 in its tail and when done).
    pub fn progress(&self) -> f32 {
        match self.state {
            PrintState::Recording if self.loop_frames > 0 => {
                (self.n as f32 / self.loop_frames as f32).min(1.0)
            }
            PrintState::Tail | PrintState::Done => 1.0,
            _ => 0.0,
        }
    }

    /// The beat the take starts (or started) on, once the transport rolls.
    pub fn start_beat(&self) -> Option<f64> {
        self.first.map(|_| self.start)
    }

    /// Takes in the capture point's block. `input` is `Mixer::capture`'s
    /// answer and `latency` `Mixer::capture_latency`'s. Real-time safe.
    pub fn process(&mut self, ctx: &Ctx, input: Option<(&[f32], &[f32])>, latency: u32) {
        let len = input.map_or(0, |(l, r)| l.len().min(r.len()));
        let per_frame = ctx.bpm / 60.0 / ctx.rate.max(1) as f64;
        let continuous = ctx.playing
            && self.expect.is_some_and(|(f, b)| {
                f == ctx.frame && (b - ctx.beats).abs() <= 1e-6 * b.abs().max(1.0)
            });
        if ctx.playing {
            if !continuous {
                self.rolling_since = Some(ctx.beats);
            }
            self.expect = Some((ctx.frame + len as u64, ctx.beats + len as f64 * per_frame));
        } else {
            self.rolling_since = None;
            self.expect = None;
        }
        match self.state {
            PrintState::Waiting => {
                if !ctx.playing || ctx.bpm <= 0.0 {
                    self.first = None;
                    return;
                }
                if self.first.is_some() && !continuous {
                    // A jump before the take began: work the start out again.
                    self.first = None;
                }
                if self.first.is_none() && !self.plan(ctx, latency) {
                    return;
                }
                if ctx.bpm != self.bpm {
                    self.first = None;
                    return;
                }
                self.copy(ctx, input, len);
            }
            PrintState::Recording => {
                if !ctx.playing {
                    return self.fail(PrintError::Stopped);
                }
                if !continuous {
                    return self.fail(PrintError::Jumped);
                }
                if ctx.bpm != self.bpm {
                    return self.fail(PrintError::TempoChanged);
                }
                if input.is_none() {
                    return self.fail(PrintError::NoSignal);
                }
                self.copy(ctx, input, len);
            }
            PrintState::Tail => {
                if input.is_none() {
                    self.state = PrintState::Done;
                    return;
                }
                self.copy(ctx, input, len);
            }
            _ => {}
        }
    }

    /// Works out the take's start beat and first frame on the first block
    /// the transport rolls. False when it fails.
    fn plan(&mut self, ctx: &Ctx, latency: u32) -> bool {
        let job = self.job;
        let mut start = job.start;
        if start < ctx.beats {
            start += ((ctx.beats - start) / job.beats).ceil() * job.beats;
        }
        if job.steady {
            let since = self.rolling_since.unwrap_or(ctx.beats);
            if since > start - job.beats + 1e-9 {
                start += job.beats;
            }
        }
        let fpb = 60.0 * self.rate as f64 / ctx.bpm;
        let loop_frames = (job.beats * fpb).round() as usize;
        let tail_max = match job.tail {
            Tail::Cut => 0,
            Tail::UntilSilent { max_ms } => (max_ms.max(0.0) * 0.001 * self.rate as f32) as usize,
        };
        if loop_frames == 0 || loop_frames > self.capacity() {
            self.fail(PrintError::TooLong);
            return false;
        }
        self.tail_max = tail_max.min(self.capacity() - loop_frames);
        self.loop_frames = loop_frames;
        self.bpm = ctx.bpm;
        self.start = start;
        let ahead = ((start - ctx.beats) * fpb).round().max(0.0) as u64;
        self.first = Some(ctx.frame + ahead + u64::from(latency));
        true
    }

    fn copy(&mut self, ctx: &Ctx, input: Option<(&[f32], &[f32])>, len: usize) {
        let Some(first) = self.first else {
            return;
        };
        let end = ctx.frame + len as u64;
        if end <= first {
            return;
        }
        let Some((l, r)) = input else {
            if self.state == PrintState::Waiting {
                self.fail(PrintError::NoSignal);
            }
            return;
        };
        if self.state == PrintState::Waiting {
            self.state = PrintState::Recording;
        }
        let mut i = first.saturating_sub(ctx.frame) as usize;
        while i < len {
            let want = match self.state {
                PrintState::Recording => self.loop_frames,
                PrintState::Tail => self.loop_frames + self.tail_max,
                _ => return,
            };
            let take = (want - self.n).min(len - i);
            self.buf[0][self.n..self.n + take].copy_from_slice(&l[i..i + take]);
            self.buf[1][self.n..self.n + take].copy_from_slice(&r[i..i + take]);
            if self.state == PrintState::Tail {
                for k in i..i + take {
                    if l[k].abs() < SILENT && r[k].abs() < SILENT {
                        self.quiet += 1;
                    } else {
                        self.quiet = 0;
                    }
                }
            }
            self.n += take;
            i += take;
            if self.state == PrintState::Tail && self.quiet >= self.quiet_frames {
                self.state = PrintState::Done;
                return;
            }
            if self.n == want {
                self.state = match self.state {
                    PrintState::Recording if self.tail_max > 0 => PrintState::Tail,
                    _ => PrintState::Done,
                };
                if self.state == PrintState::Done {
                    return;
                }
            }
        }
    }

    fn fail(&mut self, e: PrintError) {
        self.state = PrintState::Failed(e);
        self.n = 0;
        self.first = None;
    }

    /// Hands the finished take out (`None` until it is done). It moves the
    /// memory out without allocating or freeing; drop the take off the
    /// audio thread. The print is spent after this.
    pub fn take(&mut self) -> Option<Printed> {
        if self.state != PrintState::Done {
            return None;
        }
        self.state = PrintState::Idle;
        let [mut left, mut right] = std::mem::take(&mut self.buf);
        left.truncate(self.n);
        right.truncate(self.n);
        let p = Printed {
            left,
            right,
            rate: self.rate,
            bpm: self.bpm,
            start: self.start,
            beats: self.job.beats,
            loop_frames: self.loop_frames.min(self.n),
        };
        self.n = 0;
        self.first = None;
        Some(p)
    }
}

/// A finished take.
#[derive(Clone, Debug, PartialEq)]
pub struct Printed {
    pub left: Vec<f32>,
    pub right: Vec<f32>,
    pub rate: u32,
    /// The tempo it was recorded at.
    pub bpm: f64,
    /// The song beat its first frame is on.
    pub start: f64,
    /// Its beats (the loop), without the tail.
    pub beats: f64,
    /// Frames in the beats; the rest is the tail.
    pub loop_frames: usize,
}

impl Printed {
    pub fn frames(&self) -> usize {
        self.left.len()
    }

    pub fn tail_frames(&self) -> usize {
        self.frames() - self.loop_frames
    }

    /// The loudest sample, linear.
    pub fn peak(&self) -> f32 {
        self.left
            .iter()
            .chain(&self.right)
            .fold(0.0f32, |m, &v| m.max(v.abs()))
    }

    /// A stereo 32-bit float WAV file of the whole take, tail included.
    pub fn wav(&self) -> Vec<u8> {
        wav_f32(&self.left, &self.right, self.rate)
    }

    /// The take as a loop for [`crate::Mixer::freeze`]: its beats, with any
    /// tail added onto its start (where it would ring as the loop comes
    /// round).
    pub fn frozen(self) -> Frozen {
        let Printed {
            mut left,
            mut right,
            rate,
            bpm,
            start,
            beats,
            loop_frames,
        } = self;
        if loop_frames > 0 {
            for i in loop_frames..left.len() {
                let k = (i - loop_frames) % loop_frames;
                left[k] += left[i];
                right[k] += right[i];
            }
        }
        left.truncate(loop_frames);
        right.truncate(loop_frames);
        Frozen {
            left,
            right,
            origin: start,
            beats,
            bpm,
            rate,
        }
    }
}

/// A stereo 32-bit float WAV file (format 3, 16-byte fmt chunk).
pub fn wav_f32(left: &[f32], right: &[f32], rate: u32) -> Vec<u8> {
    let n = left.len().min(right.len());
    let data = (n * 8) as u32;
    let mut w = Vec::with_capacity(44 + n * 8);
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&3u16.to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&rate.to_le_bytes());
    w.extend_from_slice(&(rate * 8).to_le_bytes());
    w.extend_from_slice(&8u16.to_le_bytes());
    w.extend_from_slice(&32u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&data.to_le_bytes());
    for i in 0..n {
        w.extend_from_slice(&left[i].to_le_bytes());
        w.extend_from_slice(&right[i].to_le_bytes());
    }
    w
}

/// Audio a frozen strip plays in place of its inserts. See the module
/// docs.
#[derive(Clone, Debug, PartialEq)]
pub struct Frozen {
    pub left: Vec<f32>,
    pub right: Vec<f32>,
    /// The song beat its first frame plays on (a clip's start). The loop
    /// lines up with it: `origin - beats` and `origin + beats` are the same
    /// place.
    pub origin: f64,
    /// Beats in one time round.
    pub beats: f64,
    /// The tempo it was recorded at.
    pub bpm: f64,
    pub rate: u32,
}

impl Frozen {
    pub fn frames(&self) -> usize {
        self.left.len().min(self.right.len())
    }

    /// Bytes of audio it holds.
    pub fn memory_bytes(&self) -> usize {
        (self.left.capacity() + self.right.capacity()) * 4
    }

    /// Writes the block that starts on `ctx.beats` into `left` and `right`
    /// (replacing what is there). At the recorded tempo and rate on a
    /// whole frame it copies; otherwise it reads between frames.
    pub fn render(&self, ctx: &Ctx, left: &mut [f32], right: &mut [f32]) {
        let n = self.frames();
        let len = left.len().min(right.len());
        let bad = |x: f64| x.is_nan() || x <= 0.0;
        if n == 0 || bad(self.beats) || bad(self.bpm) || ctx.rate == 0 {
            left[..len].fill(0.0);
            right[..len].fill(0.0);
            return;
        }
        let fpb = 60.0 * self.rate as f64 / self.bpm;
        let pos = (ctx.beats - self.origin).rem_euclid(self.beats) * fpb;
        let step = ctx.bpm / self.bpm * self.rate as f64 / ctx.rate as f64;
        let whole = pos.round();
        if step == 1.0 && (pos - whole).abs() < 1e-4 {
            let mut k = (whole as usize) % n;
            let mut i = 0;
            while i < len {
                let take = (n - k).min(len - i);
                left[i..i + take].copy_from_slice(&self.left[k..k + take]);
                right[i..i + take].copy_from_slice(&self.right[k..k + take]);
                i += take;
                k = 0;
            }
            return;
        }
        let nf = n as f64;
        for i in 0..len {
            let p = (pos + i as f64 * step).rem_euclid(nf);
            let k = (p as usize).min(n - 1);
            let f = (p - k as f64) as f32;
            let k1 = if k + 1 == n { 0 } else { k + 1 };
            left[i] = self.left[k] + (self.left[k1] - self.left[k]) * f;
            right[i] = self.right[k] + (self.right[k1] - self.right[k]) * f;
        }
    }
}
