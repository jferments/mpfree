//! Loudness (ITU-R BS.1770-4 integrated, in LUFS) and true peak (dBTP),
//! so every comparison can be level-matched and every limiter checked.

use crate::wav::Stereo;

/// A direct-form-I biquad, run in f64.
#[derive(Clone, Copy, Debug)]
struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
}

impl Biquad {
    fn run(&self, x: &[f64]) -> Vec<f64> {
        let (mut x1, mut x2, mut y1, mut y2) = (0.0, 0.0, 0.0, 0.0);
        x.iter()
            .map(|&v| {
                let y = self.b[0] * v + self.b[1] * x1 + self.b[2] * x2
                    - self.a[0] * y1
                    - self.a[1] * y2;
                x2 = x1;
                x1 = v;
                y2 = y1;
                y1 = y;
                y
            })
            .collect()
    }
}

/// The K-weighting pair for any rate (the libebur128 derivation, which
/// gives the standard's 48 kHz coefficients exactly).
fn k_weighting(rate: u32) -> [Biquad; 2] {
    let fs = rate as f64;
    let (f0, g, q) = (1681.974450955533, 3.999843853973347, 0.7071752369554196);
    let k = (std::f64::consts::PI * f0 / fs).tan();
    let vh = 10f64.powf(g / 20.0);
    let vb = vh.powf(0.4996667741545416);
    let a0 = 1.0 + k / q + k * k;
    let shelf = Biquad {
        b: [
            (vh + vb * k / q + k * k) / a0,
            2.0 * (k * k - vh) / a0,
            (vh - vb * k / q + k * k) / a0,
        ],
        a: [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0],
    };
    let (f0, q) = (38.13547087602444, 0.5003270373238773);
    let k = (std::f64::consts::PI * f0 / fs).tan();
    let a0 = 1.0 + k / q + k * k;
    let hp = Biquad {
        b: [1.0, -2.0, 1.0],
        a: [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0],
    };
    [shelf, hp]
}

/// Integrated loudness in LUFS (gated), or `-inf` for silence.
pub fn integrated(s: &Stereo) -> f64 {
    let [shelf, hp] = k_weighting(s.rate);
    let l = hp.run(&shelf.run(&s.left));
    let r = hp.run(&shelf.run(&s.right));
    let block = (0.4 * s.rate as f64).round() as usize;
    let step = block / 4;
    let mut powers = Vec::new();
    if l.len() < block {
        let z = (l.iter().map(|v| v * v).sum::<f64>() + r.iter().map(|v| v * v).sum::<f64>())
            / l.len().max(1) as f64;
        powers.push(z);
    } else {
        let mut at = 0;
        while at + block <= l.len() {
            let z = (l[at..at + block].iter().map(|v| v * v).sum::<f64>()
                + r[at..at + block].iter().map(|v| v * v).sum::<f64>())
                / block as f64;
            powers.push(z);
            at += step;
        }
    }
    let lufs = |z: f64| -0.691 + 10.0 * z.max(1e-30).log10();
    let abs: Vec<f64> = powers.into_iter().filter(|z| lufs(*z) > -70.0).collect();
    if abs.is_empty() {
        return f64::NEG_INFINITY;
    }
    let mean = abs.iter().sum::<f64>() / abs.len() as f64;
    let gate = lufs(mean) - 10.0;
    let rel: Vec<f64> = abs.into_iter().filter(|z| lufs(*z) > gate).collect();
    lufs(rel.iter().sum::<f64>() / rel.len() as f64)
}

/// True peak in dBTP: the largest value of the signal upsampled 4 times
/// (a 48-tap-per-phase windowed sinc), over both channels.
pub fn true_peak(s: &Stereo) -> f64 {
    const PHASES: usize = 4;
    const TAPS: usize = 48;
    let half = TAPS as isize / 2;
    // h[p][t]: the sinc at fractional offset p / 4, Blackman windowed.
    let mut h = [[0.0f64; TAPS]; PHASES];
    for (p, row) in h.iter_mut().enumerate() {
        for (t, slot) in row.iter_mut().enumerate() {
            let x = (t as isize - half) as f64 + p as f64 / PHASES as f64;
            let sinc = if x == 0.0 {
                1.0
            } else {
                (std::f64::consts::PI * x).sin() / (std::f64::consts::PI * x)
            };
            let w = {
                let u = (x + half as f64) / TAPS as f64;
                let tau = std::f64::consts::TAU;
                if (0.0..=1.0).contains(&u) {
                    0.42 - 0.5 * (tau * u).cos() + 0.08 * (2.0 * tau * u).cos()
                } else {
                    0.0
                }
            };
            *slot = sinc * w;
        }
    }
    let mut peak = 0.0f64;
    for ch in [&s.left, &s.right] {
        let n = ch.len() as isize;
        for i in 0..n {
            peak = peak.max(ch[i as usize].abs());
            for row in h.iter().skip(1) {
                let mut acc = 0.0;
                for (t, c) in row.iter().enumerate() {
                    let j = i - (t as isize - half);
                    if (0..n).contains(&j) {
                        acc += c * ch[j as usize];
                    }
                }
                peak = peak.max(acc.abs());
            }
        }
    }
    20.0 * peak.max(1e-15).log10()
}

/// Sample peak in dBFS over both channels.
pub fn sample_peak(s: &Stereo) -> f64 {
    let p = s
        .left
        .iter()
        .chain(&s.right)
        .fold(0.0f64, |m, v| m.max(v.abs()));
    20.0 * p.max(1e-15).log10()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signals::sine;

    #[test]
    fn a_full_scale_997_hz_sine_on_both_sides_is_0_lufs() {
        let x = sine(997.0, 1.0, 48_000 * 5, 48_000);
        let l = integrated(&Stereo::mono(x, 48_000));
        assert!(l.abs() < 0.05, "{l}");
    }

    #[test]
    fn one_side_is_3_db_lower() {
        let x = sine(997.0, 1.0, 44_100 * 5, 44_100);
        let s = Stereo::new(x.clone(), vec![0.0; x.len()], 44_100);
        assert!((integrated(&s) + 3.01).abs() < 0.05);
    }

    #[test]
    fn a_quarter_rate_sine_at_45_degrees_peaks_3_db_above_its_samples() {
        let x: Vec<f64> = (0..4000)
            .map(|i| (std::f64::consts::FRAC_PI_2 * i as f64 + std::f64::consts::FRAC_PI_4).sin())
            .collect();
        let s = Stereo::mono(x, 48_000);
        assert!((sample_peak(&s) + 3.01).abs() < 0.01);
        assert!(true_peak(&s).abs() < 0.1, "{}", true_peak(&s));
    }
}
