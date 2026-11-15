//! Frequency response from an exponential sweep (Farina's method).
//!
//! The device's response to the sweep, deconvolved, gives its impulse
//! response with the harmonics pushed to negative time; a window keeps
//! the linear part. Dividing its spectrum by a wire's (the same sweep,
//! the same window) cancels the sweep's own band edges, so the result is
//! the device's gain at each frequency, exact for a linear device.

use crate::dut::Dut;
use crate::fft::{convolve, pow2_at_least, Fft};
use crate::signals::{db_to_amp, Sweep};
use crate::wav::Stereo;

/// The standard third-octave centres, 25 Hz to 20 kHz.
pub const THIRD_OCTAVES: &[f64] = &[
    25.0, 31.5, 40.0, 50.0, 63.0, 80.0, 100.0, 125.0, 160.0, 200.0, 250.0, 315.0, 400.0, 500.0,
    630.0, 800.0, 1000.0, 1250.0, 1600.0, 2000.0, 2500.0, 3150.0, 4000.0, 5000.0, 6300.0, 8000.0,
    10_000.0, 12_500.0, 16_000.0, 20_000.0,
];

/// A sweep stimulus: 20 Hz to 0.47 of the rate over 3 seconds at
/// `level_db` dBFS peak.
pub fn sweep(level_db: f64, rate: u32) -> Sweep {
    Sweep::new(20.0, 0.47 * rate as f64, 3.0, db_to_amp(level_db), rate)
}

/// A measured response.
#[derive(Clone, Debug, PartialEq)]
pub struct Response {
    /// (Hz, gain dB, phase in radians after removing the latency) at the
    /// third-octave centres the sweep covers.
    pub points: Vec<(f64, f64, f64)>,
    /// Frames from input to the response's peak (the measured latency).
    pub delay: usize,
}

impl Response {
    /// The gain at the third-octave centre nearest `hz`.
    pub fn at(&self, hz: f64) -> Option<f64> {
        self.points
            .iter()
            .min_by(|a, b| (a.0 / hz).ln().abs().total_cmp(&(b.0 / hz).ln().abs()))
            .map(|p| p.1)
    }

    /// The largest gain error against `target(hz)` (dB) from `lo` to `hi`.
    pub fn worst_error(&self, lo: f64, hi: f64, target: impl Fn(f64) -> f64) -> f64 {
        self.points
            .iter()
            .filter(|p| p.0 >= lo && p.0 <= hi)
            .map(|p| (p.1 - target(p.0)).abs())
            .fold(0.0, f64::max)
    }
}

/// Deconvolves `out` (the left channel) against `s`, keeping `window`
/// frames of linear response from a few frames before its peak, and
/// returns its gain and phase at the third-octave centres.
pub fn analyse(s: &Sweep, out: &Stereo, window: usize) -> Response {
    let n = s.sweep.len();
    let dev = convolve(&out.left, &s.inverse);
    let wire = convolve(&s.sweep, &s.inverse);
    // The linear response starts at n - 1; find its peak within the window.
    let base = n - 1;
    let delay = (0..window.min(dev.len().saturating_sub(base)))
        .max_by(|a, b| dev[base + a].abs().total_cmp(&dev[base + b].abs()))
        .unwrap_or(0);
    // Keep a little before time zero (filters ring both ways after
    // deconvolution) but well clear of the 2nd harmonic, L ln 2 earlier.
    let pre = ((s.l * 2f64.ln()) * 0.5).min(512.0) as usize;
    let size = pow2_at_least(window + pre);
    let fft = Fft::new(size);
    let take = |ir: &[f64]| -> Vec<f64> {
        let mut v = vec![0.0; size];
        for (j, slot) in v.iter_mut().enumerate().take(window + pre) {
            let i = base + j;
            if i >= pre {
                *slot = ir.get(i - pre).copied().unwrap_or(0.0);
            }
        }
        // Half-Hann fade over the last tenth so the cut doesn't ring.
        let fade = (window / 10).max(1);
        for k in 0..fade {
            let at = window + pre - fade + k;
            v[at] *= 0.5 + 0.5 * (std::f64::consts::PI * k as f64 / fade as f64).cos();
        }
        v
    };
    let (mut dr, mut di) = (take(&dev), vec![0.0; size]);
    let (mut wr, mut wi) = (take(&wire), vec![0.0; size]);
    fft.forward(&mut dr, &mut di);
    fft.forward(&mut wr, &mut wi);
    let rate = s.rate as f64;
    let points = THIRD_OCTAVES
        .iter()
        .filter(|f| **f >= s.f1 * 1.25 && **f <= s.f2 / 1.12)
        .map(|&f| {
            let k = ((f * size as f64 / rate).round() as usize).min(size / 2);
            let (a, b, c, d) = (dr[k], di[k], wr[k], wi[k]);
            let den = c * c + d * d;
            let (hr, hi) = ((a * c + b * d) / den, (b * c - a * d) / den);
            let gain = 10.0 * (hr * hr + hi * hi).max(1e-30).log10();
            let w = std::f64::consts::TAU * k as f64 / size as f64;
            let phase = (hi.atan2(hr) + w * delay as f64 + std::f64::consts::PI)
                .rem_euclid(std::f64::consts::TAU)
                - std::f64::consts::PI;
            (f, gain, phase)
        })
        .collect();
    Response { points, delay }
}

/// Measures a device's response at `level_db` dBFS, keeping `window`
/// frames of its impulse response.
pub fn measure(dut: &mut dyn Dut, level_db: f64, window: usize) -> Response {
    let s = sweep(level_db, dut.rate());
    let stim = Stereo::mono(s.sweep.clone(), s.rate);
    let out = dut.render(&stim, window + dut.latency());
    analyse(&s, &out, window)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dut::Wire;

    #[test]
    fn a_wire_is_flat() {
        let r = measure(&mut Wire { rate: 48_000 }, -12.0, 4096);
        assert_eq!(r.delay, 0);
        assert!(r.worst_error(30.0, 20_000.0, |_| 0.0) < 0.01, "{r:?}");
    }

    #[test]
    fn a_one_pole_low_pass_matches_its_formula() {
        // y = a y + (1 - a) x, at 48 kHz.
        let a = 0.9f64;
        struct OnePole(f64, f64, f64);
        impl crate::dut::Dut for OnePole {
            fn name(&self) -> String {
                "one-pole".into()
            }
            fn rate(&self) -> u32 {
                48_000
            }
            fn latency(&self) -> usize {
                0
            }
            fn reset(&mut self) {
                self.1 = 0.0;
                self.2 = 0.0;
            }
            fn params(&self) -> Vec<mpfree_fx::mpfree_node::ParamInfo> {
                Vec::new()
            }
            fn set(&mut self, _: u16, _: f64) {}
            fn process(&mut self, input: &Stereo, _: &[crate::dut::Change]) -> Stereo {
                let a = self.0;
                let l: Vec<f64> = input
                    .left
                    .iter()
                    .map(|x| {
                        self.1 = a * self.1 + (1.0 - a) * x;
                        self.1
                    })
                    .collect();
                Stereo::mono(l, input.rate)
            }
        }
        let r = measure(&mut OnePole(a, 0.0, 0.0), -12.0, 8192);
        let expect = |hz: f64| {
            let w = std::f64::consts::TAU * hz / 48_000.0;
            let (re, im) = (1.0 - a * w.cos(), a * w.sin());
            20.0 * ((1.0 - a) / (re * re + im * im).sqrt()).log10()
        };
        assert!(r.worst_error(30.0, 20_000.0, expect) < 0.05, "{r:?}");
    }
}
