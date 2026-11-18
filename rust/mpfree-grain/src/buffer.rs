//! What grains read from: a region of a sample, or a live circular buffer.
//!
//! Both are addressed by absolute frame number, so a grain keeps reading the
//! same audio while a live buffer moves on underneath it.

use crate::hermite;

/// How a source reads one grain, chosen when the grain is born
/// ([`GrainSource::aim`]) from how fast it reads. The default is the
/// Hermite read ([`GrainSource::read`]). A source with a band-limited read
/// (the sampler's sounds, for grains pitched up) keeps here which copy of
/// its sound the grain reads and how its filter reaches: the numbers are
/// the source's own.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Aim {
    /// Read with [`GrainSource::read_aimed`], not [`GrainSource::read`].
    pub clean: bool,
    /// The source's own: for the sampler, the octave copy (0: the sound).
    pub octave: u8,
    /// The source's own: for the sampler, its filter's step and reach.
    pub step: u32,
    pub taps: u32,
}

/// Audio a grain can read. Frames are absolute; [`GrainSource::span`] says
/// which ones are readable now. Reads outside the span return 0.
pub trait GrainSource {
    /// Sample rate of the audio, so grains play at the right pitch at any
    /// output rate.
    fn rate(&self) -> u32;
    /// 1 or 2.
    fn channels(&self) -> usize;
    /// The readable frames, `start..end`.
    fn span(&self) -> (i64, i64);
    /// One sample. `ch` beyond the last channel reads the last channel.
    fn sample(&self, ch: usize, frame: i64) -> f32;

    /// Left and right at `frame + t` (`t` in 0..1), Hermite-interpolated
    /// from frames `frame - 1 ..= frame + 2`. Mono reads the same value
    /// twice. Sources override this with a faster path; it must give the
    /// same result as this default.
    #[inline]
    fn read(&self, frame: i64, t: f32) -> [f32; 2] {
        read_slow(self, frame, t)
    }

    /// A grain's next `out.len()` frames read with [`GrainSource::read`]:
    /// frame `k` at `pos` plus `inc` added `k` times (one at a time, as the
    /// voice moves a grain), split into its whole frame and the fraction.
    /// One call a grain a part, so a source can check its range and find
    /// its frames once for the run. The default reads frame by frame; a
    /// source's own must give the same values.
    #[inline]
    fn read_run(&self, pos: f64, inc: f64, out: &mut [[f32; 2]]) {
        read_run_each(self, pos, inc, out);
    }

    /// How to read a grain that moves `speed` source frames an output
    /// frame (its size, whichever way it goes). The default reads every
    /// grain with [`GrainSource::read`].
    #[inline]
    fn aim(&self, speed: f64) -> Aim {
        let _ = speed;
        Aim::default()
    }

    /// Left and right at `pos` (a frame and its fraction, inside the span)
    /// for a grain whose [`GrainSource::aim`] was clean. The default is
    /// [`GrainSource::read`].
    #[inline]
    fn read_aimed(&self, pos: f64, aim: Aim) -> [f32; 2] {
        let _ = aim;
        let f = pos.floor();
        self.read(f as i64, (pos - f) as f32)
    }

    /// A clean grain's next `out.len()` frames: frame `k` is
    /// [`GrainSource::read_aimed`] at `pos` plus `inc` added `k` times
    /// (added one at a time, as the voice moves a grain, so the positions
    /// are the same to the bit). One call a grain a part, so a source can
    /// work out once what every frame of the run shares. The default reads
    /// frame by frame; a source's own must give the same values.
    #[inline]
    fn read_aimed_run(&self, pos: f64, inc: f64, aim: Aim, out: &mut [[f32; 2]]) {
        let mut pos = pos;
        for o in out {
            *o = self.read_aimed(pos, aim);
            pos += inc;
        }
    }

    /// The first hit (where a drum or a note starts) after frame `from`
    /// and at or before `to`, for a stretch that keeps its hits sharp:
    /// grains then stop reading at a hit rather than play it softly and
    /// early or late, and the scan reaching one starts a grain there (see
    /// [`crate::GrainVoice::render`], "Hits"). The default has none.
    #[inline]
    fn next_hit(&self, from: f64, to: f64) -> Option<f64> {
        let _ = (from, to);
        None
    }
}

/// The first of `hits` (frames, in order) after `from` and at or before
/// `to`: [`GrainSource::next_hit`] for a source that keeps a list.
#[inline]
pub fn first_hit(hits: &[u64], from: f64, to: f64) -> Option<f64> {
    if hits.is_empty() || to <= from {
        return None;
    }
    let i = hits.partition_point(|&h| h as f64 <= from);
    hits.get(i).map(|&h| h as f64).filter(|&h| h <= to)
}

/// Frame positions below this (2^31 less a margin for rounding) go through
/// an `i32`, which ARMv7 converts to and from a double in one instruction
/// each; an `i64` takes two library calls. Same result either way.
pub const SMALL_FRAMES: f64 = 2_147_000_000.0;

/// Whether a run of `n` frames from `pos`, `inc` apart, stays at or above
/// frame 0 and under [`SMALL_FRAMES`] (with a frame to spare at the end).
#[inline(always)]
pub fn small_run(pos: f64, inc: f64, n: usize) -> bool {
    let last = pos + inc * n as f64;
    pos.min(last) >= 0.0 && pos.max(last) < SMALL_FRAMES
}

/// [`GrainSource::read_run`] frame by frame through [`GrainSource::read`].
#[inline(always)]
pub fn read_run_each<S: GrainSource + ?Sized>(src: &S, pos: f64, inc: f64, out: &mut [[f32; 2]]) {
    let mut pos = pos;
    if small_run(pos, inc, out.len()) {
        for o in out {
            let f = pos as i32;
            *o = src.read(i64::from(f), (pos - f64::from(f)) as f32);
            pos += inc;
        }
    } else {
        for o in out {
            let f = pos as i64;
            *o = src.read(f, (pos - f as f64) as f32);
            pos += inc;
        }
    }
}

/// The whole frames the first and the last of a small run read from
/// (`pos` as the run moves it, truncated), lowest first.
#[inline(always)]
pub fn run_frames(pos: f64, inc: f64, n: usize) -> (i64, i64) {
    let mut last = pos;
    for _ in 1..n {
        last += inc;
    }
    let (a, b) = (pos as i32, last as i32);
    (i64::from(a.min(b)), i64::from(a.max(b)))
}

/// A small run's frames read from `data` (interleaved, `channels` 1 or 2)
/// where frame `f` starts at index `(f + shift) * channels` and every
/// frame the run's Hermite reads is known to be there.
#[inline(always)]
fn read_run_in(
    data: &[f32],
    channels: usize,
    shift: i64,
    pos: f64,
    inc: f64,
    out: &mut [[f32; 2]],
) {
    let mut pos = pos;
    let shift = shift as isize - 1;
    for o in out {
        let f = pos as i32;
        let t = (pos - f64::from(f)) as f32;
        let at = (f as isize + shift) as usize * channels;
        *o = read4(&data[at..at + 4 * channels], channels, t);
        pos += inc;
    }
}

/// Interpolates four interleaved frames that are known to be in range.
#[inline(always)]
fn read4(w: &[f32], channels: usize, t: f32) -> [f32; 2] {
    if channels == 2 {
        let w: &[f32; 8] = w.try_into().expect("four stereo frames");
        [
            hermite(w[0], w[2], w[4], w[6], t),
            hermite(w[1], w[3], w[5], w[7], t),
        ]
    } else {
        let w: &[f32; 4] = w.try_into().expect("four mono frames");
        let l = hermite(w[0], w[1], w[2], w[3], t);
        [l, l]
    }
}

/// A region of an interleaved sample: granular playback.
#[derive(Clone, Copy, Debug)]
pub struct Clip<'a> {
    data: &'a [f32],
    channels: usize,
    rate: u32,
    start: i64,
    end: i64,
    hits: &'a [u64],
}

impl<'a> Clip<'a> {
    /// The whole sample. `None` unless `channels` is 1 or 2, the data is
    /// whole frames and the rate is non-zero.
    pub fn new(data: &'a [f32], channels: usize, rate: u32) -> Option<Self> {
        if !(1..=2).contains(&channels) || !data.len().is_multiple_of(channels) || rate == 0 {
            return None;
        }
        let end = (data.len() / channels) as i64;
        Some(Clip {
            data,
            channels,
            rate,
            start: 0,
            end,
            hits: &[],
        })
    }

    /// Limits grains to frames `start..end` (a region), clamped to the data.
    pub fn with_range(mut self, start: u64, end: u64) -> Self {
        let frames = (self.data.len() / self.channels) as i64;
        self.start = (start.min(i64::MAX as u64) as i64).min(frames);
        self.end = (end.min(i64::MAX as u64) as i64).clamp(self.start, frames);
        self
    }

    /// The sample's hits (frames, in order), for a stretch that keeps them
    /// sharp ([`GrainSource::next_hit`]).
    pub fn with_hits(mut self, hits: &'a [u64]) -> Self {
        self.hits = hits;
        self
    }
}

impl GrainSource for Clip<'_> {
    fn rate(&self) -> u32 {
        self.rate
    }
    fn channels(&self) -> usize {
        self.channels
    }
    fn span(&self) -> (i64, i64) {
        (self.start, self.end)
    }
    #[inline]
    fn sample(&self, ch: usize, frame: i64) -> f32 {
        if frame < self.start || frame >= self.end {
            return 0.0;
        }
        self.data[frame as usize * self.channels + ch.min(self.channels - 1)]
    }
    #[inline(always)]
    fn read(&self, frame: i64, t: f32) -> [f32; 2] {
        if frame - 1 < self.start || frame + 2 >= self.end {
            return read_slow(self, frame, t);
        }
        let c = self.channels;
        let at = (frame - 1) as usize * c;
        read4(&self.data[at..at + 4 * c], c, t)
    }
    #[inline]
    fn read_run(&self, pos: f64, inc: f64, out: &mut [[f32; 2]]) {
        if small_run(pos, inc, out.len()) {
            let (lo, hi) = run_frames(pos, inc, out.len());
            if lo > self.start && hi + 2 < self.end {
                return read_run_in(self.data, self.channels, 0, pos, inc, out);
            }
        }
        read_run_each(self, pos, inc, out);
    }
    #[inline]
    fn next_hit(&self, from: f64, to: f64) -> Option<f64> {
        first_hit(self.hits, from, to)
    }
}

/// The trait's default read, for the edges.
fn read_slow<S: GrainSource + ?Sized>(src: &S, frame: i64, t: f32) -> [f32; 2] {
    let at = |ch| {
        hermite(
            src.sample(ch, frame - 1),
            src.sample(ch, frame),
            src.sample(ch, frame + 1),
            src.sample(ch, frame + 2),
            t,
        )
    };
    let l = at(0);
    [l, if src.channels() == 2 { at(1) } else { l }]
}

/// A circular buffer of the last few seconds of a live signal: the granular
/// effect. The audio thread writes each period into it, then renders grains
/// from it. Freeze stops writes, so the cloud keeps playing what was there.
///
/// While writing, the oldest `reserve` frames are kept out of the span, so a
/// grain that starts near the old end can't be overtaken by new writes
/// before it finishes. Grains are at most 2 s; the default reserve is a
/// quarter of the buffer, which covers that for a 8 s buffer.
#[derive(Clone, Debug)]
pub struct LiveBuffer {
    data: Vec<f32>,
    channels: usize,
    rate: u32,
    capacity: u64,
    written: u64,
    /// Where the next write goes: `written % capacity`, kept as it moves so
    /// that no read or write divides (a 64-bit `%` is a slow library call
    /// on ARMv7).
    head: usize,
    reserve: u64,
    frozen: bool,
    /// How many times it was cleared: a copy out of it ([`LiveBuffer::frames`])
    /// keeps this as its mark, so it knows if the memory changed under it.
    clears: u64,
}

impl LiveBuffer {
    /// Allocates the whole buffer now (call it off the audio thread).
    /// `None` unless `channels` is 1 or 2 and `frames` and `rate` are non-zero.
    pub fn new(channels: usize, rate: u32, frames: usize) -> Option<Self> {
        if !(1..=2).contains(&channels) || frames == 0 || rate == 0 {
            return None;
        }
        Some(LiveBuffer {
            data: vec![0.0; frames * channels],
            channels,
            rate,
            capacity: frames as u64,
            written: 0,
            head: 0,
            reserve: frames as u64 / 4,
            frozen: false,
            clears: 0,
        })
    }

    /// Sets how many of the oldest frames stay out of the span while writing.
    pub fn set_reserve(&mut self, frames: usize) {
        self.reserve = (frames as u64).min(self.capacity - 1);
    }

    pub fn capacity(&self) -> usize {
        self.capacity as usize
    }
    /// Frames written so far (the absolute frame of the next write).
    pub fn written(&self) -> u64 {
        self.written
    }
    pub fn frozen(&self) -> bool {
        self.frozen
    }
    /// Freeze on: writes are ignored and the span stops moving.
    pub fn set_frozen(&mut self, on: bool) {
        self.frozen = on;
    }

    /// Forgets everything written (the span becomes empty) without touching
    /// the memory, so it is cheap enough for the audio thread. Freeze stays
    /// as it was.
    pub fn clear(&mut self) {
        self.written = 0;
        self.head = 0;
        self.clears += 1;
    }

    /// How many times it has been cleared.
    pub fn clears(&self) -> u64 {
        self.clears
    }

    /// Frames `from..to` (absolute frame numbers) as they sit in memory,
    /// interleaved: the part before the wrap and the part after it (often
    /// empty). `None` unless every frame is still held.
    pub fn frames(&self, from: u64, to: u64) -> Option<(&[f32], &[f32])> {
        let (oldest, end) = self.held();
        if to < from || (from as i64) < oldest || (to as i64) > end {
            return None;
        }
        if to == from {
            return Some((&[], &[]));
        }
        let c = self.channels;
        let a = self.slot(from as i64);
        let n = (to - from) as usize;
        let first = n.min(self.capacity as usize - a);
        Some((
            &self.data[a * c..(a + first) * c],
            &self.data[..(n - first) * c],
        ))
    }

    /// Appends interleaved frames (in this buffer's channel count). Ignored
    /// while frozen. A trailing partial frame is dropped.
    pub fn write(&mut self, interleaved: &[f32]) {
        if self.frozen {
            return;
        }
        let c = self.channels;
        for frame in interleaved.chunks_exact(c) {
            let at = self.head * c;
            self.data[at..at + c].copy_from_slice(frame);
            self.advance();
        }
    }

    /// Appends planar left and right (as the engine renders them). A mono
    /// buffer stores their average.
    pub fn write_planar(&mut self, left: &[f32], right: &[f32]) {
        if self.frozen {
            return;
        }
        for (&l, &r) in left.iter().zip(right) {
            let at = self.head * self.channels;
            if self.channels == 2 {
                self.data[at] = l;
                self.data[at + 1] = r;
            } else {
                self.data[at] = 0.5 * (l + r);
            }
            self.advance();
        }
    }

    #[inline(always)]
    fn advance(&mut self) {
        self.written += 1;
        self.head += 1;
        if self.head as u64 == self.capacity {
            self.head = 0;
        }
    }

    /// Where held frame `frame` is in `data`, in frames: `frame % capacity`,
    /// worked out back from the write head instead of by dividing. `frame`
    /// must be held (`written - capacity <= frame < written`).
    #[inline(always)]
    fn slot(&self, frame: i64) -> usize {
        let back = (self.written as i64 - frame) as usize;
        if back <= self.head {
            self.head - back
        } else {
            self.head + self.capacity as usize - back
        }
    }

    fn held(&self) -> (i64, i64) {
        let oldest = self.written.saturating_sub(self.capacity);
        (oldest as i64, self.written as i64)
    }
}

impl GrainSource for LiveBuffer {
    fn rate(&self) -> u32 {
        self.rate
    }
    fn channels(&self) -> usize {
        self.channels
    }
    fn span(&self) -> (i64, i64) {
        let (oldest, end) = self.held();
        let start = if self.frozen {
            oldest
        } else {
            // Frames that new writes will reach within `reserve` frames.
            ((self.written + self.reserve).saturating_sub(self.capacity) as i64).clamp(oldest, end)
        };
        (start, end)
    }
    #[inline]
    fn sample(&self, ch: usize, frame: i64) -> f32 {
        let (oldest, end) = self.held();
        if frame < oldest || frame >= end {
            return 0.0;
        }
        let at = self.slot(frame) * self.channels;
        self.data[at + ch.min(self.channels - 1)]
    }
    #[inline(always)]
    fn read(&self, frame: i64, t: f32) -> [f32; 2] {
        let (oldest, end) = self.held();
        if frame - 1 < oldest || frame + 2 >= end {
            return read_slow(self, frame, t);
        }
        let first = self.slot(frame - 1);
        if first + 4 > self.capacity as usize {
            return read_slow(self, frame, t); // the four frames wrap round
        }
        let c = self.channels;
        read4(&self.data[first * c..(first + 4) * c], c, t)
    }
    #[inline]
    fn read_run(&self, pos: f64, inc: f64, out: &mut [[f32; 2]]) {
        if small_run(pos, inc, out.len()) {
            let (lo, hi) = run_frames(pos, inc, out.len());
            let (oldest, end) = self.held();
            if lo > oldest && hi + 2 < end {
                // Frames lo - 1 ..= hi + 2 in one piece of `data`, unless
                // they wrap round its end.
                let first = self.slot(lo - 1);
                if first + (hi - lo) as usize + 4 <= self.capacity as usize {
                    let shift = first as i64 - (lo - 1);
                    return read_run_in(&self.data, self.channels, shift, pos, inc, out);
                }
            }
        }
        read_run_each(self, pos, inc, out);
    }
}
