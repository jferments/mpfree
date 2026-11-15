//! A double-precision radix-2 FFT and FFT convolution.
//!
//! Measurements go 100 dB and more below full scale (aliases, noise
//! floors, deconvolved impulse responses), where single precision runs
//! out, so fxlab keeps its own f64 transform rather than reusing the
//! analysis crate's f32 one. Nothing here is fast or real-time; it runs
//! on the desktop.

use std::f64::consts::TAU;

/// An in-place complex FFT of one power-of-two size.
#[derive(Clone, Debug)]
pub struct Fft {
    n: usize,
    cos: Vec<f64>,
    sin: Vec<f64>,
    rev: Vec<usize>,
}

impl Fft {
    /// `n` must be a power of two, at least 2.
    pub fn new(n: usize) -> Fft {
        assert!(
            n >= 2 && n.is_power_of_two(),
            "FFT size {n} is not a power of two"
        );
        let bits = n.trailing_zeros();
        let cos = (0..n / 2)
            .map(|k| (TAU * k as f64 / n as f64).cos())
            .collect();
        let sin = (0..n / 2)
            .map(|k| -(TAU * k as f64 / n as f64).sin())
            .collect();
        let rev = (0..n)
            .map(|i| (i as u64).reverse_bits().wrapping_shr(64 - bits) as usize)
            .collect();
        Fft { n, cos, sin, rev }
    }

    pub fn len(&self) -> usize {
        self.n
    }

    pub fn is_empty(&self) -> bool {
        false
    }

    /// Forward transform (no scaling).
    pub fn forward(&self, re: &mut [f64], im: &mut [f64]) {
        self.run(re, im, false);
    }

    /// Inverse transform, scaled by 1/n so `inverse(forward(x)) == x`.
    pub fn inverse(&self, re: &mut [f64], im: &mut [f64]) {
        self.run(re, im, true);
        let k = 1.0 / self.n as f64;
        re.iter_mut().for_each(|x| *x *= k);
        im.iter_mut().for_each(|x| *x *= k);
    }

    fn run(&self, re: &mut [f64], im: &mut [f64], inverse: bool) {
        let n = self.n;
        assert!(
            re.len() == n && im.len() == n,
            "FFT buffers must be {n} long"
        );
        for i in 0..n {
            let j = self.rev[i];
            if j > i {
                re.swap(i, j);
                im.swap(i, j);
            }
        }
        let sign = if inverse { -1.0 } else { 1.0 };
        let mut size = 2;
        while size <= n {
            let half = size / 2;
            let step = n / size;
            for start in (0..n).step_by(size) {
                for k in 0..half {
                    let (wr, wi) = (self.cos[k * step], sign * self.sin[k * step]);
                    let (a, b) = (start + k, start + k + half);
                    let tr = re[b] * wr - im[b] * wi;
                    let ti = re[b] * wi + im[b] * wr;
                    re[b] = re[a] - tr;
                    im[b] = im[a] - ti;
                    re[a] += tr;
                    im[a] += ti;
                }
            }
            size *= 2;
        }
    }
}

/// The smallest power of two at least `n` (and at least 2).
pub fn pow2_at_least(n: usize) -> usize {
    n.max(2).next_power_of_two()
}

/// Linear convolution of `a` and `b` (length `a.len() + b.len() - 1`).
pub fn convolve(a: &[f64], b: &[f64]) -> Vec<f64> {
    if a.is_empty() || b.is_empty() {
        return Vec::new();
    }
    let out = a.len() + b.len() - 1;
    let n = pow2_at_least(out);
    let fft = Fft::new(n);
    let (mut ar, mut ai) = (vec![0.0; n], vec![0.0; n]);
    let (mut br, mut bi) = (vec![0.0; n], vec![0.0; n]);
    ar[..a.len()].copy_from_slice(a);
    br[..b.len()].copy_from_slice(b);
    fft.forward(&mut ar, &mut ai);
    fft.forward(&mut br, &mut bi);
    for k in 0..n {
        let (r, i) = (ar[k] * br[k] - ai[k] * bi[k], ar[k] * bi[k] + ai[k] * br[k]);
        ar[k] = r;
        ai[k] = i;
    }
    fft.inverse(&mut ar, &mut ai);
    ar.truncate(out);
    ar
}

/// Cross-correlation of `a` against `b` at lags `-(b.len()-1)..a.len()`:
/// `out[lag + b.len() - 1] = sum_n a[n + lag] * b[n]`. The lag where it
/// peaks is how far `a` is behind `b`.
pub fn xcorr(a: &[f64], b: &[f64]) -> Vec<f64> {
    let rev: Vec<f64> = b.iter().rev().copied().collect();
    convolve(a, &rev)
}

/// Power spectrum `|X[k]|^2 / n^2` for `k` in `0..=n/2` of a real signal
/// whose length is the power of two `n` (no window: callers sample
/// coherently or window first). A full-scale sine on bin `k` reads 0.25
/// there (its two halves of 0.5 each), so [`bin_db`] adds 6 dB back.
pub fn power_spectrum(x: &[f64]) -> Vec<f64> {
    let n = x.len();
    let fft = Fft::new(n);
    let mut re = x.to_vec();
    let mut im = vec![0.0; n];
    fft.forward(&mut re, &mut im);
    let s = 1.0 / (n as f64 * n as f64);
    (0..=n / 2)
        .map(|k| (re[k] * re[k] + im[k] * im[k]) * s)
        .collect()
}

/// A Hann window of `n` frames (symmetric).
pub fn hann_f64(n: usize) -> Vec<f64> {
    if n < 2 {
        return vec![1.0; n];
    }
    (0..n)
        .map(|i| 0.5 - 0.5 * (TAU * i as f64 / (n - 1) as f64).cos())
        .collect()
}

/// A bin's power as the dBFS of the sine that would make it (0 dBFS for a
/// full-scale sine on that bin).
pub fn bin_db(power: f64) -> f64 {
    10.0 * (4.0 * power).max(1e-30).log10()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_is_exact_to_double_precision() {
        let n = 1024;
        let x: Vec<f64> = (0..n)
            .map(|i| ((i * 7919) % 101) as f64 / 50.0 - 1.0)
            .collect();
        let fft = Fft::new(n);
        let (mut re, mut im) = (x.clone(), vec![0.0; n]);
        fft.forward(&mut re, &mut im);
        fft.inverse(&mut re, &mut im);
        for (a, b) in re.iter().zip(&x) {
            assert!((a - b).abs() < 1e-12);
        }
    }

    #[test]
    fn convolution_matches_the_direct_sum() {
        let a = [1.0, 2.0, 3.0, -1.0];
        let b = [0.5, -0.25, 2.0];
        let c = convolve(&a, &b);
        let mut direct = vec![0.0; a.len() + b.len() - 1];
        for (i, x) in a.iter().enumerate() {
            for (j, y) in b.iter().enumerate() {
                direct[i + j] += x * y;
            }
        }
        for (x, y) in c.iter().zip(&direct) {
            assert!((x - y).abs() < 1e-12);
        }
    }

    #[test]
    fn xcorr_peaks_at_the_delay() {
        let b: Vec<f64> = (0..200).map(|i| ((i * 37) % 17) as f64 - 8.0).collect();
        let mut a = vec![0.0; 300];
        a[25..225].copy_from_slice(&b);
        let c = xcorr(&a, &b);
        let peak = c
            .iter()
            .enumerate()
            .max_by(|x, y| x.1.total_cmp(y.1))
            .map(|(i, _)| i as isize - (b.len() as isize - 1))
            .unwrap();
        assert_eq!(peak, 25);
    }

    #[test]
    fn a_full_scale_sine_on_a_bin_reads_0_dbfs() {
        let n = 4096;
        let x: Vec<f64> = (0..n)
            .map(|i| (TAU * 100.0 * i as f64 / n as f64).sin())
            .collect();
        let p = power_spectrum(&x);
        assert!((bin_db(p[100])).abs() < 1e-9);
        assert!(bin_db(p[101]) < -250.0);
    }
}
