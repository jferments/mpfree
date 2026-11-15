//! Harmonic distortion and aliasing from one steady sine.
//!
//! The sine sits on an odd FFT bin (coherent sampling), so the fundamental,
//! every harmonic below Nyquist and every harmonic folded back from above
//! it land on single bins with no leakage, and no window is needed. Bins
//! are then sorted into four kinds: the fundamental, harmonics (what a
//! nonlinearity adds on purpose), aliases (harmonics above Nyquist folded
//! back: what oversampling is for) and everything else (noise,
//! modulation sidebands, intermodulation).

use crate::fft::{bin_db, power_spectrum};
use crate::signals::{coherent, db_to_amp, sine};
use crate::wav::Stereo;

/// FFT length of the analysis (about 1.5 s at 44.1 kHz).
pub const N: usize = 65_536;
/// Harmonics followed when finding aliases.
pub const MAX_HARMONIC: usize = 400;

/// The stimulus for [`analyse`]: a sine near `hz` at `level_db` dBFS peak,
/// `settle` frames of it before the `N` frames that are analysed.
pub fn stimulus(hz: f64, level_db: f64, settle: usize, rate: u32) -> (Stereo, usize, usize) {
    let (bin, f) = coherent(hz, N, rate);
    let x = sine(f, db_to_amp(level_db), settle + N, rate);
    (Stereo::mono(x, rate), bin, settle)
}

/// What one sine measured.
#[derive(Clone, Debug, PartialEq)]
pub struct Harmonics {
    /// The sine's frequency.
    pub hz: f64,
    /// Input level, dBFS peak.
    pub in_db: f64,
    /// Output fundamental, dBFS peak.
    pub out_db: f64,
    /// Total harmonic distortion below Nyquist, dB relative to the
    /// fundamental (power sum of harmonics 2 and up).
    pub thd_db: f64,
    /// Harmonics 2 to 9 relative to the fundamental (None above Nyquist).
    pub h: [Option<f64>; 8],
    /// The loudest alias line (at least 10 dB over the bins around it), dB
    /// relative to the fundamental.
    pub alias_worst_db: f64,
    /// All aliases together, dB relative to the fundamental.
    pub alias_total_db: f64,
    /// The loudest bin that is none of the above (above 20 Hz), dB
    /// relative to the fundamental.
    pub other_worst_db: f64,
    /// Everything that is none of the above, power sum, dB relative.
    pub other_total_db: f64,
}

/// Analyses the left channel of `out` from frame `start` (the stimulus's
/// settle plus the device's latency), for a sine on `bin`.
pub fn analyse(out: &Stereo, bin: usize, start: usize, in_db: f64) -> Harmonics {
    let seg: Vec<f64> = (start..start + N)
        .map(|i| out.left.get(i).copied().unwrap_or(0.0))
        .collect();
    let p = power_spectrum(&seg);
    let half = N / 2;
    let fund = p[bin].max(1e-300);
    let mut kind = vec![0u8; half + 1]; // 0 other, 1 fundamental, 2 harmonic, 3 alias
    kind[bin] = 1;
    kind[0] = 4; // DC: ignored
    let mut h = [None; 8];
    let mut harm = 0.0;
    let mut alias_bins = Vec::new();
    for k in 2..=MAX_HARMONIC {
        let f = k * bin;
        let folded = {
            let m = f % N;
            if m > half {
                N - m
            } else {
                m
            }
        };
        if f < half {
            if kind[f] == 0 {
                kind[f] = 2;
                harm += p[f];
            }
            if k <= 9 {
                h[k - 2] = Some(10.0 * (p[f] / fund).max(1e-30).log10());
            }
        } else if folded != 0 && kind[folded] == 0 {
            kind[folded] = 3;
            alias_bins.push(folded);
        }
    }
    // An alias counts only as a line: 10 dB over the median of the
    // unclassified bins around it. A modulated effect (chorus, tremolo)
    // smears energy across many bins, some of which fall where aliases
    // would; that smear is not aliasing and is reported as "other".
    let mut alias = 0.0;
    let mut alias_worst = 0.0f64;
    for &b in &alias_bins {
        let mut near: Vec<f64> = (b.saturating_sub(16)..=(b + 16).min(half))
            .filter(|&j| j != b && kind[j] == 0)
            .map(|j| p[j])
            .collect();
        let floor = if near.is_empty() {
            0.0
        } else {
            near.sort_by(f64::total_cmp);
            near[near.len() / 2]
        };
        if p[b] > 10.0 * floor {
            alias += p[b];
            alias_worst = alias_worst.max(p[b]);
        } else {
            kind[b] = 0;
        }
    }
    let low = (20.0 * N as f64 / out.rate as f64).ceil() as usize;
    let mut other = 0.0;
    let mut other_worst = 0.0f64;
    for k in low..=half {
        if kind[k] == 0 {
            other += p[k];
            other_worst = other_worst.max(p[k]);
        }
    }
    let rel = |x: f64| 10.0 * (x / fund).max(1e-30).log10();
    Harmonics {
        hz: bin as f64 * out.rate as f64 / N as f64,
        in_db,
        out_db: bin_db(fund),
        thd_db: rel(harm),
        h,
        alias_worst_db: rel(alias_worst),
        alias_total_db: rel(alias),
        other_worst_db: rel(other_worst),
        other_total_db: rel(other),
    }
}

/// Renders and analyses one sine through a device.
pub fn measure(dut: &mut dyn crate::dut::Dut, hz: f64, level_db: f64) -> Harmonics {
    let rate = dut.rate();
    let settle = rate as usize / 2;
    let (stim, bin, settle) = stimulus(hz, level_db, settle, rate);
    let lat = dut.latency();
    let out = dut.render(&stim, lat);
    analyse(&out, bin, settle + lat, level_db)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dut::{Shaper, Wire};

    #[test]
    fn a_wire_is_clean() {
        let m = measure(&mut Wire { rate: 44_100 }, 1000.0, -6.0);
        assert!((m.out_db + 6.0).abs() < 1e-6);
        assert!(m.thd_db < -250.0, "{m:?}");
        assert!(m.alias_worst_db < -250.0);
    }

    #[test]
    fn a_cubic_makes_a_third_harmonic_of_the_right_size() {
        // x - x^3/3 on a sine of amplitude a: the 3rd harmonic is a^3/12
        // against a fundamental of a - a^3/4.
        let a: f64 = 0.5;
        let mut d = Shaper {
            rate: 44_100,
            curve: |x: f64| x - x * x * x / 3.0,
        };
        let m = measure(&mut d, 1000.0, 20.0 * a.log10());
        let expect = 20.0 * ((a * a * a / 12.0) / (a - a * a * a / 4.0)).log10();
        assert!(
            (m.h[1].unwrap() - expect).abs() < 0.01,
            "{:?} vs {expect}",
            m.h[1]
        );
        assert!(m.h[0].unwrap() < -250.0, "no even harmonics");
    }

    #[test]
    fn a_hard_clip_at_high_frequency_aliases() {
        let mut d = Shaper {
            rate: 44_100,
            curve: |x: f64| x.clamp(-0.5, 0.5),
        };
        let m = measure(&mut d, 5000.0, 0.0);
        assert!(m.alias_worst_db > -40.0, "{m:?}");
    }
}
