//! A region of a sound, as grains read it.
//!
//! [`GrainClip`] is what [`mpfree_grain::Clip`] is, for a sound held as
//! floats or as 16 bits, read the same way (a 16-bit sound's grains are its
//! floats' grains, bit for bit), plus the sampler's clean pitching: unless
//! the pad is Vintage, a grain more than [`CLEAN_ABOVE`] faster than the
//! sound's own rate reads band-limited, as a sample voice pitched up does
//! ([`bandlimit::read2`], past an octave from the sound's octave copies), so
//! what it would fold back above the output's Nyquist frequency is filtered
//! out instead. Grains at or under two semitones up keep the Hermite read
//! and its cost.

use mpfree_grain::{hermite, Aim, GrainSource};

use crate::bandlimit::{self, Kernel, Octaves, Reach};
use crate::dsp::Sample;
use crate::engine::CLEAN_ABOVE;

/// One output frame's step in the sampler's 32.32 fixed point.
const ONE: f64 = 4_294_967_296.0;

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
        let at = (frame - 1) as usize * self.channels;
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

    /// Clean (unless Vintage) past [`CLEAN_ABOVE`]: the octave copy that
    /// brings the speed under [`bandlimit::MAX_STRETCH`], and the filter
    /// stretched for what is left, as a sample voice aims ([`Reach`]).
    fn aim(&self, speed: f64) -> Aim {
        if self.kernel.is_none() || speed * ONE <= CLEAN_ABOVE as f64 {
            return Aim::default();
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
    fn read_aimed(&self, pos: f64, aim: Aim) -> [f32; 2] {
        let Some(kernel) = self.kernel else {
            let f = pos.floor();
            return self.read(f as i64, (pos - f) as f32);
        };
        if self.end <= self.start {
            return [0.0; 2];
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
}
