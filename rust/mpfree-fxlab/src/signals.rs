//! Test signals: sines on exact FFT bins, exponential sweeps and their
//! inverse filters, noise, impulses, bursts and level steps. All are
//! deterministic (seeded), so a measurement repeats bit for bit.

use std::f64::consts::{PI, TAU};

/// A small seeded generator (xorshift64*), the same on every machine.
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed.max(1))
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform in [0, 1).
    pub fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Uniform in [-1, 1).
    pub fn bipolar(&mut self) -> f64 {
        2.0 * self.unit() - 1.0
    }

    /// Standard normal (Box-Muller).
    pub fn gauss(&mut self) -> f64 {
        let u = self.unit().max(1e-300);
        let v = self.unit();
        (-2.0 * u.ln()).sqrt() * (TAU * v).cos()
    }
}

/// dBFS (sine peak) to linear amplitude.
pub fn db_to_amp(db: f64) -> f64 {
    10f64.powf(db / 20.0)
}

/// Linear amplitude to dB, floored at -300 dB.
pub fn amp_to_db(a: f64) -> f64 {
    20.0 * a.abs().max(1e-15).log10()
}

/// The frequency nearest `hz` that puts a whole number of cycles in
/// `n` frames, so an FFT of `n` frames sees it on one bin with no
/// leakage. Returns (the bin, its frequency).
pub fn coherent(hz: f64, n: usize, rate: u32) -> (usize, f64) {
    let bin = ((hz * n as f64 / rate as f64).round() as usize).clamp(1, n / 2 - 1);
    // An odd bin keeps the harmonics and their aliases off each other's bins
    // as long as possible (an even bin's 2nd harmonic folds onto lower ones).
    let bin = if bin.is_multiple_of(2) { bin + 1 } else { bin };
    (bin, bin as f64 * rate as f64 / n as f64)
}

/// `frames` of a sine at `hz` and peak `amp`, starting at phase 0.
pub fn sine(hz: f64, amp: f64, frames: usize, rate: u32) -> Vec<f64> {
    let w = TAU * hz / rate as f64;
    (0..frames).map(|i| amp * (w * i as f64).sin()).collect()
}

/// An exponential (Farina) sine sweep and its inverse filter.
///
/// Convolving a system's response to `sweep` with `inverse` gives its
/// linear impulse response at index `sweep.len() - 1` (plus the system's
/// latency) and each harmonic's response `L ln(k)` frames before it,
/// where `L = rate * secs / ln(f2/f1)`.
#[derive(Clone, Debug)]
pub struct Sweep {
    pub sweep: Vec<f64>,
    pub inverse: Vec<f64>,
    pub f1: f64,
    pub f2: f64,
    pub rate: u32,
    /// Frames per natural-log unit of frequency (`L` above).
    pub l: f64,
}

impl Sweep {
    /// A sweep from `f1` to `f2` Hz over `secs` at peak `amp`, with 10 ms
    /// raised-cosine fades at both ends.
    pub fn new(f1: f64, f2: f64, secs: f64, amp: f64, rate: u32) -> Sweep {
        let n = (secs * rate as f64).round() as usize;
        let r = (f2 / f1).ln();
        let l = n as f64 / r;
        let fade = ((0.01 * rate as f64) as usize).min(n / 4).max(1);
        let sweep: Vec<f64> = (0..n)
            .map(|i| {
                let t = i as f64;
                let phase = TAU * f1 * l / rate as f64 * ((t / l).exp() - 1.0);
                let env = if i < fade {
                    0.5 - 0.5 * (PI * i as f64 / fade as f64).cos()
                } else if i >= n - fade {
                    0.5 - 0.5 * (PI * (n - 1 - i) as f64 / fade as f64).cos()
                } else {
                    1.0
                };
                amp * env * phase.sin()
            })
            .collect();
        // Time-reversed, with a -6 dB/octave tilt so the sweep's pink
        // energy becomes white; normalised so a straight wire gives a unit
        // peak.
        let mut inverse: Vec<f64> = (0..n)
            .map(|i| sweep[n - 1 - i] * (-(i as f64) / l).exp())
            .collect();
        let peak = crate::fft::convolve(&sweep, &inverse)
            .iter()
            .fold(0.0f64, |m, x| m.max(x.abs()));
        let k = 1.0 / peak;
        inverse.iter_mut().for_each(|x| *x *= k);
        Sweep {
            sweep,
            inverse,
            f1,
            f2,
            rate,
            l,
        }
    }

    /// Frames before the linear response where harmonic `k` (2, 3, ...)
    /// lands.
    pub fn harmonic_offset(&self, k: u32) -> f64 {
        self.l * (k as f64).ln()
    }
}

/// White noise, uniform, at peak `amp`.
pub fn white(frames: usize, amp: f64, seed: u64) -> Vec<f64> {
    let mut r = Rng::new(seed);
    (0..frames).map(|_| amp * r.bipolar()).collect()
}

/// Pink noise (Paul Kellet's refined filter on white noise), scaled so its
/// RMS is `rms`.
pub fn pink(frames: usize, rms: f64, seed: u64) -> Vec<f64> {
    let mut r = Rng::new(seed);
    let mut b = [0.0f64; 7];
    let mut out: Vec<f64> = (0..frames)
        .map(|_| {
            let w = r.bipolar();
            b[0] = 0.99886 * b[0] + w * 0.0555179;
            b[1] = 0.99332 * b[1] + w * 0.0750759;
            b[2] = 0.96900 * b[2] + w * 0.1538520;
            b[3] = 0.86650 * b[3] + w * 0.3104856;
            b[4] = 0.55000 * b[4] + w * 0.5329522;
            b[5] = -0.7616 * b[5] - w * 0.0168980;
            let y = b.iter().sum::<f64>() + w * 0.5362;
            b[6] = w * 0.115926;
            y
        })
        .collect();
    let now = rms_of(&out);
    if now > 0.0 {
        let k = rms / now;
        out.iter_mut().for_each(|x| *x *= k);
    }
    out
}

/// A unit impulse at frame 0 followed by silence.
pub fn impulse(frames: usize, amp: f64) -> Vec<f64> {
    let mut v = vec![0.0; frames.max(1)];
    v[0] = amp;
    v
}

/// A sine whose level steps between `low_db` and `high_db` (dBFS peak)
/// every `step` frames, starting low: the burst a compressor's attack and
/// release are timed on. Steps land on the sine's zero crossings' frame
/// grid; the change itself is instant.
pub fn level_steps(
    hz: f64,
    low_db: f64,
    high_db: f64,
    step: usize,
    steps: usize,
    rate: u32,
) -> Vec<f64> {
    let s = sine(hz, 1.0, step * steps, rate);
    s.iter()
        .enumerate()
        .map(|(i, x)| {
            let high = (i / step) % 2 == 1;
            x * db_to_amp(if high { high_db } else { low_db })
        })
        .collect()
}

/// RMS of a signal (0 for an empty one).
pub fn rms_of(x: &[f64]) -> f64 {
    if x.is_empty() {
        return 0.0;
    }
    (x.iter().map(|v| v * v).sum::<f64>() / x.len() as f64).sqrt()
}

/// Peak absolute value.
pub fn peak_of(x: &[f64]) -> f64 {
    x.iter().fold(0.0f64, |m, v| m.max(v.abs()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coherent_frequencies_sit_on_odd_bins() {
        let (bin, hz) = coherent(1000.0, 65536, 44_100);
        assert_eq!(bin % 2, 1);
        assert!((hz - 1000.0).abs() < 1.0);
    }

    #[test]
    fn a_sweep_through_a_wire_deconvolves_to_a_unit_impulse() {
        let s = Sweep::new(20.0, 20_000.0, 0.5, 0.5, 48_000);
        let ir = crate::fft::convolve(&s.sweep, &s.inverse);
        let at = s.sweep.len() - 1;
        assert!((ir[at] - 1.0).abs() < 1e-9);
        // Everything away from the peak is far below it.
        let away = ir
            .iter()
            .enumerate()
            .filter(|(i, _)| (*i as isize - at as isize).abs() > 200)
            .fold(0.0f64, |m, (_, x)| m.max(x.abs()));
        assert!(away < 0.01, "sidelobes {away}");
    }

    #[test]
    fn pink_noise_has_the_rms_asked_for() {
        let p = pink(48_000, 0.1, 7);
        assert!((rms_of(&p) - 0.1).abs() < 1e-12);
    }

    #[test]
    fn the_generator_repeats() {
        let a: Vec<u64> = (0..4)
            .map({
                let mut r = Rng::new(3);
                move |_| r.next_u64()
            })
            .collect();
        let b: Vec<u64> = (0..4)
            .map({
                let mut r = Rng::new(3);
                move |_| r.next_u64()
            })
            .collect();
        assert_eq!(a, b);
    }
}
