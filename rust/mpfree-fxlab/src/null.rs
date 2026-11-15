//! Null tests: how much of one signal is left after subtracting another,
//! lined up to a fraction of a frame and matched in gain.

use crate::fft::{pow2_at_least, xcorr, Fft};
use crate::wav::Stereo;

/// How two signals line up.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Alignment {
    /// Frames `b` must be delayed by to sit on `a` (may be fractional or
    /// negative).
    pub lag: f64,
    /// Gain applied to `b` (least squares).
    pub gain: f64,
}

/// Finds the lag (within `max_lag` frames) where `b` best matches `a`,
/// refined by a parabola through the correlation peak.
pub fn align(a: &[f64], b: &[f64], max_lag: usize) -> Alignment {
    let c = xcorr(a, b);
    let zero = b.len() as isize - 1;
    let lo = (zero - max_lag as isize).max(0) as usize;
    let hi = ((zero + max_lag as isize) as usize).min(c.len() - 1);
    let k = (lo..=hi)
        .max_by(|x, y| c[*x].abs().total_cmp(&c[*y].abs()))
        .unwrap_or(zero as usize);
    let whole = k as f64 - zero as f64;
    // Refine to a fraction of a frame by minimising the gain-matched
    // residual (golden section over half a frame either way), then keep
    // the whole-frame lag if that is at least as good.
    let fit = |lag: f64| -> (f64, f64) {
        let shifted = delay(b, lag, a.len());
        let num: f64 = a.iter().zip(&shifted).map(|(x, y)| x * y).sum();
        let den: f64 = shifted.iter().map(|y| y * y).sum();
        let gain = if den > 0.0 { num / den } else { 0.0 };
        let res: f64 = a
            .iter()
            .zip(&shifted)
            .map(|(x, y)| (x - gain * y).powi(2))
            .sum();
        (res, gain)
    };
    let phi = 0.5 * (5f64.sqrt() - 1.0);
    let (mut lo, mut hi) = (whole - 0.5, whole + 0.5);
    let mut c = hi - phi * (hi - lo);
    let mut d = lo + phi * (hi - lo);
    let (mut fc, mut fd) = (fit(c).0, fit(d).0);
    for _ in 0..40 {
        if fc < fd {
            hi = d;
            d = c;
            fd = fc;
            c = hi - phi * (hi - lo);
            fc = fit(c).0;
        } else {
            lo = c;
            c = d;
            fc = fd;
            d = lo + phi * (hi - lo);
            fd = fit(d).0;
        }
    }
    let frac_lag = 0.5 * (lo + hi);
    let (rf, gf) = fit(frac_lag);
    let (rw, gw) = fit(whole);
    if rw <= rf {
        Alignment {
            lag: whole,
            gain: gw,
        }
    } else {
        Alignment {
            lag: frac_lag,
            gain: gf,
        }
    }
}

/// `x` delayed by `lag` frames (fractional, by a phase shift in the
/// frequency domain), `len` frames long.
pub fn delay(x: &[f64], lag: f64, len: usize) -> Vec<f64> {
    let whole = lag.round();
    if (lag - whole).abs() < 1e-9 {
        let d = whole as isize;
        return (0..len as isize)
            .map(|i| {
                let j = i - d;
                if j >= 0 && (j as usize) < x.len() {
                    x[j as usize]
                } else {
                    0.0
                }
            })
            .collect();
    }
    let n = pow2_at_least(x.len().max(len) + lag.abs().ceil() as usize + 64);
    let fft = Fft::new(n);
    let mut re = vec![0.0; n];
    let mut im = vec![0.0; n];
    re[..x.len()].copy_from_slice(x);
    fft.forward(&mut re, &mut im);
    for k in 0..n {
        let f = if k <= n / 2 {
            k as f64
        } else {
            k as f64 - n as f64
        };
        let w = -std::f64::consts::TAU * f * lag / n as f64;
        let (c, s) = (w.cos(), w.sin());
        let (r, i) = (re[k] * c - im[k] * s, re[k] * s + im[k] * c);
        re[k] = r;
        im[k] = i;
    }
    fft.inverse(&mut re, &mut im);
    re.truncate(len);
    re.resize(len, 0.0);
    re
}

/// The result of a null test.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Null {
    pub alignment: Alignment,
    /// Residual energy against `a`'s, dB (very negative = they match).
    pub depth_db: f64,
}

/// Lines `b` up on `a` (both channels share the alignment found on the
/// mid signal) and measures what is left.
pub fn null(a: &Stereo, b: &Stereo, max_lag: usize) -> Null {
    let al = align(&a.mid(), &b.mid(), max_lag);
    let mut res = 0.0;
    let mut sig = 0.0;
    for (x, y) in [(&a.left, &b.left), (&a.right, &b.right)] {
        let s = delay(y, al.lag, x.len());
        for (p, q) in x.iter().zip(&s) {
            let d = p - al.gain * q;
            res += d * d;
            sig += p * p;
        }
    }
    Null {
        alignment: al,
        depth_db: 10.0 * (res.max(1e-300) / sig.max(1e-300)).log10(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signals::pink;

    #[test]
    fn a_delayed_scaled_copy_nulls_deeply() {
        let x = pink(20_000, 0.2, 1);
        let b = Stereo::mono(x.clone(), 48_000);
        let shifted: Vec<f64> = delay(&x, 37.0, x.len()).iter().map(|v| 0.5 * v).collect();
        let a = Stereo::mono(shifted, 48_000);
        let n = null(&a, &b, 100);
        assert!((n.alignment.lag - 37.0).abs() < 1e-6, "{n:?}");
        assert!((n.alignment.gain - 0.5).abs() < 1e-9);
        assert!(n.depth_db < -200.0, "{n:?}");
    }

    #[test]
    fn a_fractional_delay_is_found() {
        let x = pink(16_384, 0.2, 2);
        let a = Stereo::mono(delay(&x, 10.3, x.len()), 48_000);
        let b = Stereo::mono(x, 48_000);
        let n = null(&a, &b, 50);
        assert!((n.alignment.lag - 10.3).abs() < 0.05, "{n:?}");
    }
}
