//! A small in-place radix-2 FFT in `f64`, for building tables off the audio
//! thread. Not used while playing.

use std::f64::consts::TAU;

/// Transforms `re`/`im` in place. `inverse` uses the positive exponent and
/// does not scale. The length must be a power of two.
pub fn fft(re: &mut [f64], im: &mut [f64], inverse: bool) {
    let n = re.len();
    assert_eq!(n, im.len());
    assert!(n.is_power_of_two());
    // Bit reversal.
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let sign = if inverse { 1.0 } else { -1.0 };
    let mut len = 2;
    while len <= n {
        let ang = sign * TAU / len as f64;
        let (wr, wi) = (ang.cos(), ang.sin());
        for start in (0..n).step_by(len) {
            let (mut cr, mut ci) = (1.0f64, 0.0f64);
            for k in 0..len / 2 {
                let a = start + k;
                let b = a + len / 2;
                let tr = re[b] * cr - im[b] * ci;
                let ti = re[b] * ci + im[b] * cr;
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
                let ncr = cr * wr - ci * wi;
                ci = cr * wi + ci * wr;
                cr = ncr;
            }
        }
        len <<= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_finds_a_harmonic() {
        let n = 64;
        let mut re: Vec<f64> = (0..n).map(|i| (TAU * 3.0 * i as f64 / n as f64).cos()).collect();
        let orig = re.clone();
        let mut im = vec![0.0; n];
        fft(&mut re, &mut im, false);
        assert!((re[3] - n as f64 / 2.0).abs() < 1e-9);
        assert!(re[5].abs() < 1e-9 && im[3].abs() < 1e-9);
        fft(&mut re, &mut im, true);
        for i in 0..n {
            assert!((re[i] / n as f64 - orig[i]).abs() < 1e-12);
        }
    }
}
