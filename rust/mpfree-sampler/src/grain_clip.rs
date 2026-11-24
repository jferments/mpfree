//! A region of a sound, as grains read it.
//!
//! [`GrainClip`] is what [`mpfree_grain::Clip`] is, for a sound held as
//! floats or as 16 bits, read the same way (a 16-bit sound's grains are its
//! floats' grains, bit for bit), plus the sampler's clean pitching: unless
//! the pad is Vintage, a grain reads as a sample voice at its speed does.
//! At and under [`CLEAN_ABOVE`] (two semitones up) that is the 8-point read
//! ([`bandlimit::near_kernel`]); faster, the band-limited read
//! ([`bandlimit::read2`], past an octave from the sound's octave copies),
//! so what it would fold back above the output's Nyquist frequency is
//! filtered out instead. A Vintage pad's grains read with Hermite.

use mpfree_grain::buffer::{first_hit, read_run_each, run_frames, small_run};
use mpfree_grain::{hermite, Aim, GrainSource};

use crate::bandlimit::{
    self, near_kernel, near_sum, near_sum2, Kernel, Octaves, Reach, NEAR_HALF, NEAR_TAPS,
};
use crate::dsp::Sample;
use crate::engine::CLEAN_ABOVE;

/// One output frame's step in the sampler's 32.32 fixed point.
const ONE: f64 = 4_294_967_296.0;

/// The 8-point read's aim: no taps marks it (a band-limited read always
/// has some).
const NEAR: Aim = Aim {
    clean: true,
    octave: 0,
    step: 0,
    taps: 0,
};

/// A region of an interleaved sound (1 or 2 channels) for grains.
#[derive(Clone, Copy)]
pub struct GrainClip<'a, S> {
    data: &'a [S],
    channels: usize,
    rate: u32,
    start: i64,
    end: i64,
    /// The band-limited read's kernel: none reads every grain with Hermite
    /// (a Vintage pad).
    kernel: Option<&'static Kernel>,
    octaves: Option<&'a Octaves>,
    /// Its hits, kept sharp ([`GrainSource::next_hit`]).
    hits: &'a [u64],
}

impl<'a, S: Sample> GrainClip<'a, S> {
    /// The whole of `data`, read with Hermite only (as Vintage). None
    /// unless `channels` is 1 or 2, `data` holds whole frames and `rate`
    /// isn't 0.
    pub fn new(data: &'a [S], channels: usize, rate: u32) -> Option<Self> {
        if !(1..=2).contains(&channels) || !data.len().is_multiple_of(channels) || rate == 0 {
            return None;
        }
        let end = (data.len() / channels) as i64;
        Some(GrainClip {
            data,
            channels,
            rate,
            start: 0,
            end,
            kernel: None,
            octaves: None,
            hits: &[],
        })
    }

    /// Only frames `start..end` (clamped to the sound).
    pub fn with_range(mut self, start: u64, end: u64) -> Self {
        let frames = (self.data.len() / self.channels) as i64;
        self.start = (start.min(i64::MAX as u64) as i64).min(frames);
        self.end = (end.min(i64::MAX as u64) as i64).clamp(self.start, frames);
        self
    }

    /// Grains pitched more than two semitones up read band-limited, from
    /// `octaves` (the sound's copies, [`bandlimit::Octaves`]) past an
    /// octave when they are given; without them past an octave the read
    /// stretches its filter as far as it goes and folds a little, as a
    /// sample voice does.
    pub fn clean(mut self, octaves: Option<&'a Octaves>) -> Self {
        self.kernel = Some(bandlimit::kernel());
        self.octaves = octaves;
        self
    }

    /// The sound's hits (frames, in order), for a Stretch that keeps them
    /// sharp ([`crate::hits`]).
    pub fn with_hits(mut self, hits: &'a [u64]) -> Self {
        self.hits = hits;
        self
    }

    /// The Hermite read between frames `first + 1` and `first + 2`, at
    /// `t`, from frames `first ..= first + 3`, all inside the sound.
    #[inline(always)]
    fn read_inside(&self, first: usize, t: f32) -> [f32; 2] {
        let at = first * self.channels;
        if self.channels == 2 {
            let w: &[S; 8] = self.data[at..at + 8]
                .try_into()
                .expect("four stereo frames");
            let w = w.map(S::f);
            [
                hermite(w[0], w[2], w[4], w[6], t),
                hermite(w[1], w[3], w[5], w[7], t),
            ]
        } else {
            let w: &[S; 4] = self.data[at..at + 4].try_into().expect("four mono frames");
            let w = w.map(S::f);
            let l = hermite(w[0], w[1], w[2], w[3], t);
            [l, l]
        }
    }
}

impl<S: Sample> GrainSource for GrainClip<'_, S> {
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
        self.data[frame as usize * self.channels + ch.min(self.channels - 1)].f()
    }

    #[inline(always)]
    fn read(&self, frame: i64, t: f32) -> [f32; 2] {
        if frame - 1 < self.start || frame + 2 >= self.end {
            // The trait's own read, as the float clip's edges use.
            let at = |ch| {
                hermite(
                    self.sample(ch, frame - 1),
                    self.sample(ch, frame),
                    self.sample(ch, frame + 1),
                    self.sample(ch, frame + 2),
                    t,
                )
            };
            let l = at(0);
            return [l, if self.channels == 2 { at(1) } else { l }];
        }
        self.read_inside((frame - 1) as usize, t)
    }

    /// [`GrainSource::read`] for a run of frames, the same values: when
    /// every frame the run reads is inside the region, it skips the
    /// edges' test frame by frame.
    #[inline]
    fn read_run(&self, pos: f64, inc: f64, out: &mut [[f32; 2]]) {
        if small_run(pos, inc, out.len()) {
            let (lo, hi) = run_frames(pos, inc, out.len());
            if lo > self.start && hi + 2 < self.end {
                let mut pos = pos;
                for o in out {
                    let f = pos as i32;
                    *o = self.read_inside(f as usize - 1, (pos - f64::from(f)) as f32);
                    pos += inc;
                }
                return;
            }
        }
        read_run_each(self, pos, inc, out);
    }

    /// Unless Vintage: the 8-point read at and under [`CLEAN_ABOVE`]
    /// ([`NEAR`]); past it the octave copy that brings the speed under
    /// [`bandlimit::MAX_STRETCH`], and the filter stretched for what is
    /// left, as a sample voice aims ([`Reach`]).
    fn aim(&self, speed: f64) -> Aim {
        if self.kernel.is_none() {
            return Aim::default();
        }
        if speed * ONE <= CLEAN_ABOVE as f64 {
            return NEAR;
        }
        let mut r = speed as f32;
        let mut octave = 0u8;
        if let Some(o) = self.octaves {
            while r >= bandlimit::MAX_STRETCH && o.level(usize::from(octave) + 1).is_some() {
                octave += 1;
                r *= 0.5;
            }
        }
        let (step, taps) = Reach::new(r).parts();
        Aim {
            clean: true,
            octave,
            step,
            taps,
        }
    }

    #[inline]
    fn next_hit(&self, from: f64, to: f64) -> Option<f64> {
        first_hit(self.hits, from, to)
    }

    #[inline]
    fn read_aimed(&self, pos: f64, aim: Aim) -> [f32; 2] {
        let Some(kernel) = self.kernel else {
            let f = pos.floor();
            return self.read(f as i64, (pos - f) as f32);
        };
        if self.end <= self.start {
            return [0.0; 2];
        }
        if aim == NEAR {
            let mut o = [[0.0f32; 2]; 1];
            near_run(
                self.data,
                self.channels == 2,
                self.start,
                self.end,
                pos,
                0.0,
                &mut o,
            );
            return o[0];
        }
        let k = u32::from(aim.octave);
        let at = (pos * ONE) as i64;
        let (lo, hi, at) = (self.start >> k, (self.end - 1) >> k, at >> k);
        let reach = Reach::from_parts(aim.step, aim.taps);
        let stereo = self.channels == 2;
        let (l, r) = match self.octaves.and_then(|o| o.level(k as usize)) {
            // The copies are floats, whatever the sound holds.
            Some(copy) if k > 0 => {
                bandlimit::read2(kernel, &copy[..], stereo, lo, hi, at, reach, (0, 1))
            }
            _ => bandlimit::read2(kernel, self.data, stereo, lo, hi, at, reach, (0, 1)),
        };
        [l, r]
    }

    /// [`GrainSource::read_aimed`] for a run of frames, the same values:
    /// the copy, its frames' range, the stereo test and the reach once for
    /// the run, and the fixed-point position without a 64-bit conversion.
    #[inline]
    fn read_aimed_run(&self, pos: f64, inc: f64, aim: Aim, out: &mut [[f32; 2]]) {
        let Some(kernel) = self.kernel else {
            let mut pos = pos;
            for o in out {
                *o = self.read_aimed(pos, aim);
                pos += inc;
            }
            return;
        };
        if self.end <= self.start {
            out.fill([0.0; 2]);
            return;
        }
        if aim == NEAR {
            near_run(
                self.data,
                self.channels == 2,
                self.start,
                self.end,
                pos,
                inc,
                out,
            );
            return;
        }
        let k = u32::from(aim.octave);
        let (lo, hi) = (self.start >> k, (self.end - 1) >> k);
        let reach = Reach::from_parts(aim.step, aim.taps);
        let stereo = self.channels == 2;
        match self.octaves.and_then(|o| o.level(k as usize)) {
            Some(copy) if k > 0 => {
                clean_run(kernel, &copy[..], stereo, lo, hi, k, reach, pos, inc, out)
            }
            _ => clean_run(kernel, self.data, stereo, lo, hi, k, reach, pos, inc, out),
        }
    }
}

/// The frames of a clean run from `data` (the sound, or its octave copy
/// `k`): [`bandlimit::read2`] at each position, as `read_aimed` reads it.
#[inline(always)]
#[allow(clippy::too_many_arguments)]
fn clean_run<T: Sample>(
    kernel: &Kernel,
    data: &[T],
    stereo: bool,
    lo: i64,
    hi: i64,
    k: u32,
    reach: Reach,
    mut pos: f64,
    inc: f64,
    out: &mut [[f32; 2]],
) {
    // A small run's position in 32.32 fixed point without a 64-bit
    // conversion (a library call on ARMv7): the whole frames through an
    // `i32` and the fraction through a `u32`, one instruction each. It is
    // the same number: `pos * ONE` is exact, and so is `pos` less its
    // whole frames.
    if small_run(pos, inc, out.len()) {
        for o in out.iter_mut() {
            let f = pos as i32;
            let frac = ((pos - f64::from(f)) * ONE) as u32;
            let at = ((i64::from(f) << 32) | i64::from(frac)) >> k;
            let (l, r) = bandlimit::read2(kernel, data, stereo, lo, hi, at, reach, (0, 1));
            *o = [l, r];
            pos += inc;
        }
    } else {
        for o in out.iter_mut() {
            let at = ((pos * ONE) as i64) >> k;
            let (l, r) = bandlimit::read2(kernel, data, stereo, lo, hi, at, reach, (0, 1));
            *o = [l, r];
            pos += inc;
        }
    }
}

/// The 8-point read for a run of frames from `pos`, `inc` apart, as a
/// sample voice reads at the same speed; frames outside `start..end` read
/// as 0 (as the Hermite read's edges do).
#[inline(always)]
fn near_run<T: Sample>(
    data: &[T],
    stereo: bool,
    start: i64,
    end: i64,
    mut pos: f64,
    inc: f64,
    out: &mut [[f32; 2]],
) {
    let k = near_kernel();
    let ch = if stereo { 2 } else { 1 };
    // Every frame inside: no edge tests, and the position split through
    // an i32 (one instruction) instead of a floor.
    if small_run(pos, inc, out.len()) {
        let (lo, hi) = run_frames(pos, inc, out.len());
        if lo + 1 - NEAR_HALF as i64 >= start && hi + (NEAR_HALF as i64) < end {
            for o in out.iter_mut() {
                let f = pos as i32;
                let c = k.weights(((pos - f64::from(f)) * ONE) as u32);
                let at = (f as usize + 1 - NEAR_HALF) * ch;
                if stereo {
                    let w: &[T; 2 * NEAR_TAPS] = data[at..at + 2 * NEAR_TAPS]
                        .try_into()
                        .expect("8 stereo frames");
                    let (l, r) = near_sum2(&c, &w.map(T::f));
                    *o = [l, r];
                } else {
                    let w: &[T; NEAR_TAPS] = data[at..at + NEAR_TAPS].try_into().expect("8 frames");
                    let l = near_sum(&c, &w.map(T::f));
                    *o = [l, l];
                }
                pos += inc;
            }
            return;
        }
    }
    for o in out.iter_mut() {
        let f = pos.floor();
        let i = f as i64;
        let c = k.weights(((pos - f) * ONE) as u32);
        let first = i + 1 - NEAR_HALF as i64;
        if first >= start && first + NEAR_TAPS as i64 <= end {
            let at = first as usize * ch;
            if stereo {
                let mut x = [0.0f32; 2 * NEAR_TAPS];
                for (d, s) in x.iter_mut().zip(&data[at..at + 2 * NEAR_TAPS]) {
                    *d = s.f();
                }
                let (l, r) = near_sum2(&c, &x);
                *o = [l, r];
            } else {
                let mut x = [0.0f32; NEAR_TAPS];
                for (d, s) in x.iter_mut().zip(&data[at..at + NEAR_TAPS]) {
                    *d = s.f();
                }
                let l = near_sum(&c, &x);
                *o = [l, l];
            }
        } else {
            let get = |j: i64, c: usize| {
                if j < start || j >= end {
                    0.0
                } else {
                    data[j as usize * ch + c].f()
                }
            };
            let mut l = [0.0f32; NEAR_TAPS];
            let mut r = [0.0f32; NEAR_TAPS];
            for m in 0..NEAR_TAPS {
                l[m] = get(first + m as i64, 0);
                r[m] = get(first + m as i64, ch - 1);
            }
            *o = [near_sum(&c, &l), near_sum(&c, &r)];
        }
        pos += inc;
    }
}
