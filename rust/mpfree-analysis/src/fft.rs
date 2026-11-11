//! A small radix-2 FFT for the analysers.
//!
//! The analysers only need a few fixed power-of-two sizes and run off the
//! audio thread, so this is a plain iterative complex FFT with a table of
//! twiddles, about forty lines with no dependencies. If analysis ever needs
//! to be faster, `realfft` (MIT) is the drop-in replacement named in the
//! architecture.

#[derive(Clone, Debug)]
pub struct Fft {
    n: usize,
    cos: Vec<f32>,
    sin: Vec<f32>,
    rev: Vec<u32>,
}

impl Fft {
    /// `n` must be a power of two, at least 2.
    pub fn new(n: usize) -> Fft {
        assert!(
            n >= 2 && n.is_power_of_two(),
            "FFT size must be a power of two"
        );
        let bits = n.trailing_zeros();
        let cos = (0..n / 2)
            .map(|k| (std::f64::consts::TAU * k as f64 / n as f64).cos() as f32)
            .collect();
        let sin = (0..n / 2)
            .map(|k| -(std::f64::consts::TAU * k as f64 / n as f64).sin() as f32)
            .collect();
        let rev = (0..n as u32)
            .map(|i| i.reverse_bits() >> (32 - bits))
            .collect();
        Fft { n, cos, sin, rev }
    }

    pub fn len(&self) -> usize {
        self.n
    }
    pub fn is_empty(&self) -> bool {
        false
    }

    /// In-place forward transform of `re` + i·`im`.
    pub fn forward(&self, re: &mut [f32], im: &mut [f32]) {
        let n = self.n;
        assert!(re.len() == n && im.len() == n);
        for i in 0..n {
            let j = self.rev[i] as usize;
            if j > i {
                re.swap(i, j);
                im.swap(i, j);
            }
        }
        let mut size = 2;
        while size <= n {
            let half = size / 2;
            let step = n / size;
            for start in (0..n).step_by(size) {
                for k in 0..half {
                    let (wr, wi) = (self.cos[k * step], self.sin[k * step]);
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

/// A Hann window of length `n` (periodic form, for analysis).
pub fn hann(n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| (0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / n as f64).cos()) as f32)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_a_direct_dft() {
        let n = 64;
        let f = Fft::new(n);
        let x: Vec<f32> = (0..n).map(|i| ((i * 37 % 11) as f32 - 5.0) / 5.0).collect();
        let mut re = x.clone();
        let mut im = vec![0.0; n];
        f.forward(&mut re, &mut im);
        for k in 0..n {
            let (mut dr, mut di) = (0.0f64, 0.0f64);
            for (t, v) in x.iter().enumerate() {
                let a = -std::f64::consts::TAU * (k * t) as f64 / n as f64;
                dr += f64::from(*v) * a.cos();
                di += f64::from(*v) * a.sin();
            }
            assert!(
                (f64::from(re[k]) - dr).abs() < 1e-3 && (f64::from(im[k]) - di).abs() < 1e-3,
                "bin {k}"
            );
        }
    }

    #[test]
    fn a_sine_lands_in_its_bin() {
        let n = 1024;
        let f = Fft::new(n);
        let mut re: Vec<f32> = (0..n)
            .map(|i| (std::f32::consts::TAU * 10.0 * i as f32 / n as f32).sin())
            .collect();
        let mut im = vec![0.0; n];
        f.forward(&mut re, &mut im);
        let mag: Vec<f32> = (0..n / 2).map(|k| re[k].hypot(im[k])).collect();
        let peak = mag
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .unwrap()
            .0;
        assert_eq!(peak, 10);
        assert!((mag[10] - n as f32 / 2.0).abs() < 0.5);
    }
}
