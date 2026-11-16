//! Analog reference curves for EQ bands, and how far a digital curve is
//! from them. The prototypes are the s-domain designs of R.
//! Bristow-Johnson's "Audio EQ cookbook" (the conventions MPFree's EQ
//! uses: Q is the half-gain bandwidth of a bell; a shelf's frequency is
//! where it reaches half its gain) and Butterworth cuts. A digital EQ
//! made by the bilinear transform matches them at low frequencies and
//! "cramps" near Nyquist; these functions measure by how much (the FX
//! quality plan's E3 and E4).

/// `n` frequencies from `lo` to `hi` Hz, evenly spaced on a log scale.
pub fn log_freqs(lo: f64, hi: f64, n: usize) -> Vec<f64> {
    let n = n.max(2);
    (0..n)
        .map(|i| lo * (hi / lo).powf(i as f64 / (n - 1) as f64))
        .collect()
}

fn db_of(nr: f64, ni: f64, dr: f64, di: f64) -> f64 {
    10.0 * ((nr * nr + ni * ni) / (dr * dr + di * di)).log10()
}

/// A bell's gain in dB at `f`: `H(s) = (s^2 + s A/Q + 1) / (s^2 + s/(A Q) + 1)`
/// with `A = 10^(gain/40)`, `s` normalised to `fc`.
pub fn bell_db(f: f64, fc: f64, gain_db: f64, q: f64) -> f64 {
    let a = 10f64.powf(gain_db / 40.0);
    let w = f / fc;
    let re = 1.0 - w * w;
    db_of(re, w * a / q, re, w / (a * q))
}

/// A low shelf's gain in dB at `f`:
/// `H(s) = A (s^2 + (sqrt A / Q) s + A) / (A s^2 + (sqrt A / Q) s + 1)`.
pub fn low_shelf_db(f: f64, fc: f64, gain_db: f64, q: f64) -> f64 {
    let a = 10f64.powf(gain_db / 40.0);
    let w = f / fc;
    let k = a.sqrt() / q * w;
    20.0 * a.log10() + db_of(a - w * w, k, 1.0 - a * w * w, k)
}

/// A high shelf's gain in dB at `f`:
/// `H(s) = A (A s^2 + (sqrt A / Q) s + 1) / (s^2 + (sqrt A / Q) s + A)`.
pub fn high_shelf_db(f: f64, fc: f64, gain_db: f64, q: f64) -> f64 {
    let a = 10f64.powf(gain_db / 40.0);
    let w = f / fc;
    let k = a.sqrt() / q * w;
    20.0 * a.log10() + db_of(1.0 - a * w * w, k, a - w * w, k)
}

/// A Butterworth cut of `order` (2 for 12 dB per octave, 4 for 24) at
/// `fc`: a high-pass (low cut) when `high_pass`, else a low-pass.
pub fn butterworth_db(f: f64, fc: f64, order: u32, high_pass: bool) -> f64 {
    let w = if high_pass { fc / f } else { f / fc };
    -10.0 * (1.0 + w.powi(2 * order as i32)).log10()
}

/// The largest difference of `digital` (dB at each of `freqs`) from
/// `analog`, with its sign, and where it is.
pub fn worst_diff(freqs: &[f64], digital: &[f64], analog: impl Fn(f64) -> f64) -> (f64, f64) {
    let mut worst = 0.0f64;
    let mut at = freqs.first().copied().unwrap_or(0.0);
    for (f, d) in freqs.iter().zip(digital) {
        let e = d - analog(*f);
        if e.abs() > worst.abs() {
            worst = e;
            at = *f;
        }
    }
    (worst, at)
}

/// The width in octaves of the region where `curve` (dB) is at least half
/// of `gain_db` (in dB), searched from `lo` to `hi` Hz: the bell's
/// half-gain bandwidth, the convention RBJ's Q follows. None if it never
/// gets there.
pub fn half_gain_width(curve: impl Fn(f64) -> f64, gain_db: f64, lo: f64, hi: f64) -> Option<f64> {
    let half = gain_db / 2.0;
    let inside = |v: f64| if gain_db >= 0.0 { v >= half } else { v <= half };
    let fs = log_freqs(lo, hi, 8000);
    let first = fs.iter().position(|f| inside(curve(*f)))?;
    let last = fs.iter().rposition(|f| inside(curve(*f)))?;
    Some((fs[last] / fs[first]).log2())
}

/// The half-gain bandwidth in octaves that RBJ's Q stands for:
/// `1/Q = 2 sinh(ln 2 / 2 * BW)`.
pub fn q_to_octaves(q: f64) -> f64 {
    2.0 / std::f64::consts::LN_2 * (1.0 / (2.0 * q)).asinh()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bells_peak_at_their_gain_and_fall_away() {
        assert!((bell_db(1000.0, 1000.0, 12.0, 1.0) - 12.0).abs() < 1e-9);
        assert!(bell_db(20.0, 1000.0, 12.0, 1.0).abs() < 0.05);
        // A boost and the same cut cancel.
        for f in log_freqs(20.0, 20_000.0, 50) {
            let s = bell_db(f, 2000.0, 9.0, 2.0) + bell_db(f, 2000.0, -9.0, 2.0);
            assert!(s.abs() < 1e-9);
        }
    }

    #[test]
    fn q_is_the_half_gain_bandwidth() {
        for q in [0.71, 1.41, 4.0] {
            let w =
                half_gain_width(|f| bell_db(f, 1000.0, 12.0, q), 12.0, 10.0, 100_000.0).unwrap();
            assert!((w - q_to_octaves(q)).abs() < 0.01, "Q {q}: {w}");
        }
        assert!((q_to_octaves(1.41) - 1.0).abs() < 0.01);
        assert!((q_to_octaves(0.71) - 1.89).abs() < 0.01);
    }

    #[test]
    fn shelves_reach_half_their_gain_at_their_frequency() {
        for g in [-12.0, 6.0] {
            assert!((low_shelf_db(500.0, 500.0, g, 0.71) - g / 2.0).abs() < 1e-9);
            assert!((high_shelf_db(5000.0, 5000.0, g, 0.71) - g / 2.0).abs() < 1e-9);
            assert!((low_shelf_db(10.0, 500.0, g, 0.71) - g).abs() < 0.05);
            assert!((high_shelf_db(100_000.0, 5000.0, g, 0.71) - g).abs() < 0.05);
        }
    }

    #[test]
    fn butterworth_is_3_db_down_at_the_corner() {
        for order in [1, 2, 4] {
            for hp in [false, true] {
                assert!((butterworth_db(1000.0, 1000.0, order, hp) + 3.0103).abs() < 1e-3);
            }
        }
        assert!((butterworth_db(4000.0, 1000.0, 2, false) + 10.0 * 257f64.log10()).abs() < 1e-9);
    }
}
