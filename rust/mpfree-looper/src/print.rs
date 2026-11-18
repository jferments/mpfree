//! Prints: a loop at another tempo with its pitch kept.
//!
//! A [`PrintJob`] stretches a loop's [`Bounce`] (from
//! [`crate::Looper::bounce_print`]) to the loop's length at another tempo,
//! with `mpfree-stretch` (the sampler's phase vocoder, which keeps hits
//! sharp), and gives the [`Print`] for [`crate::Looper::print`]. It is
//! heavy work for a worker thread: [`PrintJob::run`] does it in steps.
//!
//! A loop goes round, so its start follows its end. The job stretches the
//! loop with two stretch windows of it wrapped on before and after, and
//! keeps the middle. A held note rarely fits a whole number of its cycles
//! into the new length, so the print's start is crossfaded over
//! [`SEAM`] frames from what the stretch made after its end (which carries
//! on from the end) into its own start: the print joins up at its loop
//! point with no click.
//!
//! A loop at another speed ([`crate::Edit::Speed`]) plays its audio
//! faster or slower, like tape, by whole octaves ([`Bounce::rate`]). Its
//! print is the audio resampled by those octaves first ([`octave`], a
//! windowed sinc taken round the loop, so it joins up too), which makes it
//! the pitch the speed gives, then stretched only if the tempo also
//! differs. Played as it is, the print sounds as the loop does at that
//! speed, and an overdub can fit to it.

use std::sync::Arc;

use mpfree_stretch::{auto_window, Job, Settings, StretchError, TimeMap};
use mpfree_transport::time::{Tick, PPQ};

use crate::{Bounce, Print};

/// The crossfade at a print's loop point, in frames (at most half the
/// print).
pub const SEAM: usize = 1024;

/// Frames a loop `ticks` long lasts at `bpm`, at `rate`.
pub fn frames_at(ticks: Tick, bpm: f64, rate: u32) -> usize {
    (ticks as f64 * 60.0 * f64::from(rate) / (bpm * PPQ as f64)).round() as usize
}

/// One print being made.
pub struct PrintJob {
    /// The stretch, or None when the resample alone made it (`ready`).
    job: Option<Job>,
    ready: Option<Vec<f32>>,
    /// Output frames before the loop (the wrapped-on window, stretched).
    before: usize,
    frames: usize,
    ticks: Tick,
    origin: Tick,
    version: u64,
}

impl PrintJob {
    /// A job making `bounce` (stereo, at `rate`) `frames` long, at its
    /// speed ([`Bounce::rate`]): resampled by its octaves, then
    /// stretched. The stretch must stay within 10x either way.
    pub fn new(bounce: &Bounce, rate: u32, frames: usize) -> Result<PrintJob, StretchError> {
        let len = bounce.frames.min(bounce.audio.len() / 2);
        if len == 0 || frames == 0 {
            return Err(StretchError::Empty);
        }
        // At another speed: an octave at a time, slower longer.
        let mut sped: Option<Vec<f32>> = None;
        for _ in 0..bounce.rate.unsigned_abs().min(2) {
            let from = sped.as_deref().unwrap_or(&bounce.audio[..2 * len]);
            sped = Some(octave(from, from.len() / 2, bounce.rate < 0));
        }
        let audio = sped.as_deref().unwrap_or(&bounce.audio[..2 * len]);
        let len = audio.len() / 2;
        if len == 0 {
            return Err(StretchError::Empty);
        }
        // The resample made it the length it plays at (within a frame):
        // nothing to stretch.
        if sped.is_some() && frames.abs_diff(len) <= 1 {
            let mut out = Vec::with_capacity(2 * frames);
            for i in 0..frames {
                let q = i % len;
                out.push(audio[2 * q]);
                out.push(audio[2 * q + 1]);
            }
            return Ok(PrintJob {
                job: None,
                ready: Some(out),
                before: 0,
                frames,
                ticks: bounce.ticks,
                origin: bounce.origin,
                version: bounce.version,
            });
        }
        // Two windows of the loop on either side, taken round it.
        let wrap = 2 * auto_window(rate);
        let total = len + 2 * wrap;
        let mut data = Vec::with_capacity(2 * total);
        for i in 0..total {
            let q = (i + len * (wrap / len + 1) - wrap) % len;
            data.push(audio[2 * q]);
            data.push(audio[2 * q + 1]);
        }
        let r = frames as f64 / len as f64;
        let before = (wrap as f64 * r).round().max(1.0);
        let (w, l, f) = (wrap as f64, len as f64, frames as f64);
        let map = TimeMap::new(vec![
            (0.0, 0.0),
            (w, before),
            (w + l, before + f),
            (2.0 * w + l, 2.0 * before + f),
        ])?;
        let settings = Settings::default();
        let job = Job::new(Arc::from(data), 2, rate, map, settings)?;
        Ok(PrintJob {
            job: Some(job),
            ready: None,
            before: before as usize,
            frames,
            ticks: bounce.ticks,
            origin: bounce.origin,
            version: bounce.version,
        })
    }

    /// Does about `frames` output frames' worth of work; true once done.
    pub fn run(&mut self, frames: usize) -> bool {
        self.job.as_mut().is_none_or(|j| j.run(frames).finished)
    }

    /// How far it has got, 0 to 1.
    pub fn progress(&self) -> f32 {
        self.job.as_ref().map_or(1.0, |j| j.progress().done)
    }

    /// The print, once [`PrintJob::run`] is done.
    pub fn finish(self) -> Option<Print> {
        if let Some(audio) = self.ready {
            return Some(Print {
                audio,
                frames: self.frames,
                ticks: self.ticks,
                origin: self.origin,
                version: self.version,
            });
        }
        let mut audio = self.job?.into_output()?;
        let (from, n) = (2 * self.before, 2 * self.frames);
        let seam = SEAM.min(self.frames / 2).min(self.before);
        if audio.len() < from + n + 2 * seam {
            return None;
        }
        for i in 0..seam {
            let w = (i as f32 + 0.5) / seam as f32;
            for c in 0..2 {
                let after = audio[from + n + 2 * i + c];
                let start = &mut audio[from + 2 * i + c];
                *start = after * (1.0 - w) + *start * w;
            }
        }
        audio.drain(..from);
        audio.truncate(n);
        audio.shrink_to_fit();
        Some(Print {
            audio,
            frames: self.frames,
            ticks: self.ticks,
            origin: self.origin,
            version: self.version,
        })
    }
}

/// A whole print at once: `bounce` made `frames` long (see [`PrintJob`]).
pub fn print(bounce: &Bounce, rate: u32, frames: usize) -> Result<Print, StretchError> {
    let mut job = PrintJob::new(bounce, rate, frames)?;
    while !job.run(1 << 16) {}
    job.finish().ok_or(StretchError::Empty)
}

/// Taps either side of a speed resample's filter, in input frames.
const OCTAVE_TAPS: usize = 32;
/// The top of the band an octave resample keeps, as a share of the lower
/// rate's Nyquist (20 kHz of 22.05 kHz at 44.1 kHz).
const OCTAVE_PASS: f64 = 0.9;
/// The Kaiser window's shape (about 80 dB down in the stop band).
const OCTAVE_BETA: f64 = 8.0;

fn bessel_i0(x: f64) -> f64 {
    let (mut sum, mut term) = (1.0, 1.0);
    let q = x * x / 4.0;
    for k in 1..100 {
        term *= q / (k * k) as f64;
        sum += term;
        if term < 1e-15 * sum {
            break;
        }
    }
    sum
}

/// One octave of speed on a loop's audio (stereo, `len` frames, going
/// round): `up` true makes it twice as long (half speed, an octave down
/// when it plays in the time the loop had), false half as long (double
/// speed, an octave up), through a windowed-sinc low-pass that keeps the
/// band below the lower rate's Nyquist and takes what would fold back.
/// The loop's end runs into its start, so the result joins up too.
pub fn octave(audio: &[f32], len: usize, up: bool) -> Vec<f32> {
    let len = len.min(audio.len() / 2);
    if len == 0 {
        return Vec::new();
    }
    // Input frames a frame out moves on, and the cutoff in cycles an
    // input frame.
    let (step2, fc) = if up {
        (1usize, 0.5 * OCTAVE_PASS)
    } else {
        (4usize, 0.25 * OCTAVE_PASS)
    };
    // Each phase's taps: a frame out at input time m * step2 / 2 sits
    // at whole frame i0 plus 0 or a half.
    let n = 2 * OCTAVE_TAPS;
    let phases: Vec<Vec<f32>> = (0..if up { 2 } else { 1 })
        .map(|p| {
            let frac = 0.5 * p as f64;
            let mut h: Vec<f64> = (0..n)
                .map(|j| {
                    // Tap j reads input frame i0 + j - (TAPS - 1).
                    let t = j as f64 - (OCTAVE_TAPS as f64 - 1.0) - frac;
                    let x = 2.0 * fc * t;
                    let sinc = if x.abs() < 1e-12 {
                        1.0
                    } else {
                        (std::f64::consts::PI * x).sin() / (std::f64::consts::PI * x)
                    };
                    let w = t / OCTAVE_TAPS as f64;
                    let win = if w.abs() >= 1.0 {
                        0.0
                    } else {
                        bessel_i0(OCTAVE_BETA * (1.0 - w * w).sqrt()) / bessel_i0(OCTAVE_BETA)
                    };
                    2.0 * fc * sinc * win
                })
                .collect();
            // A steady level stays where it was.
            let sum: f64 = h.iter().sum();
            h.iter_mut().for_each(|v| *v /= sum);
            h.into_iter().map(|v| v as f32).collect()
        })
        .collect();
    let out_len = if up { 2 * len } else { len.div_ceil(2) };
    let mut out = Vec::with_capacity(2 * out_len);
    for m in 0..out_len {
        let t2 = m * step2; // input time, in half frames
        let (i0, p) = (t2 / 2, t2 % 2);
        let h = &phases[p];
        let base = i0 + len * (OCTAVE_TAPS / len + 1) - (OCTAVE_TAPS - 1);
        let (mut l, mut r) = (0.0f32, 0.0f32);
        for (j, &w) in h.iter().enumerate() {
            let q = (base + j) % len;
            l += audio[2 * q] * w;
            r += audio[2 * q + 1] * w;
        }
        out.push(l);
        out.push(r);
    }
    out
}
