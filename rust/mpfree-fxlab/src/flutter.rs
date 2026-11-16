//! Wow and flutter: how much a tone's pitch wavers on its way through a
//! device, measured the way a tape deck is, with a 3.15 kHz tone (IEC 386,
//! AES6). The tone is isolated (bins outside a quarter of its frequency
//! either side are dropped, so a saturator's harmonics don't count), its
//! instantaneous frequency read from the analytic signal and smoothed
//! over 2.2 ms (about a 200 Hz band), and the deviation reported in
//! percent, unweighted: the standards' weighting filter, which favours
//! wavering near 4 Hz, is not applied (the FX quality plan's D9).

use crate::fft::{pow2_at_least, Fft};
use crate::signals::{db_to_amp, sine};
use crate::wav::Stereo;

/// The test tone, Hz.
pub const TONE_HZ: f64 = 3150.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Flutter {
    /// The tone's mean frequency, Hz.
    pub mean_hz: f64,
    /// RMS deviation from the mean, percent.
    pub rms_pct: f64,
    /// The largest deviation from the mean, percent.
    pub peak_pct: f64,
}

/// The instantaneous frequency of the tone near `near_hz` in `x`, Hz per
/// frame, with the first and last tenth (the transform's edges) left out.
pub fn instantaneous_hz(x: &[f64], rate: u32, near_hz: f64) -> Vec<f64> {
    let n = pow2_at_least(x.len());
    let fft = Fft::new(n);
    let mut re = vec![0.0; n];
    let mut im = vec![0.0; n];
    re[..x.len()].copy_from_slice(x);
    fft.forward(&mut re, &mut im);
    let bin = |hz: f64| (hz * n as f64 / rate as f64) as usize;
    let (lo, hi) = (bin(near_hz * 0.75), bin(near_hz * 1.25).min(n / 2));
    for k in 0..n {
        // Keep the tone's positive frequencies, doubled; drop the rest.
        let keep = k >= lo && k <= hi;
        let s = if keep { 2.0 } else { 0.0 };
        re[k] *= s;
        im[k] *= s;
    }
    fft.inverse(&mut re, &mut im);
    let len = x.len();
    let mut prev = im[0].atan2(re[0]);
    let mut f = Vec::with_capacity(len);
    let k = rate as f64 / std::f64::consts::TAU;
    for i in 1..len {
        let ph = im[i].atan2(re[i]);
        let mut d = ph - prev;
        while d > std::f64::consts::PI {
            d -= std::f64::consts::TAU;
        }
        while d < -std::f64::consts::PI {
            d += std::f64::consts::TAU;
        }
        f.push(d * k);
        prev = ph;
    }
    // Smooth over 2.2 ms.
    let w = ((0.0022 * rate as f64) as usize).max(1);
    let mut acc = 0.0;
    let mut smooth = Vec::with_capacity(f.len());
    for i in 0..f.len() {
        acc += f[i];
        if i >= w {
            acc -= f[i - w];
        }
        if i + 1 >= w {
            smooth.push(acc / w as f64);
        }
    }
    let cut = smooth.len() / 10;
    smooth[cut..smooth.len() - cut].to_vec()
}

/// Wow and flutter of the tone near `near_hz` in `x`.
pub fn analyse(x: &[f64], rate: u32, near_hz: f64) -> Option<Flutter> {
    let f = instantaneous_hz(x, rate, near_hz);
    if f.len() < 16 {
        return None;
    }
    let mean = f.iter().sum::<f64>() / f.len() as f64;
    if mean <= 0.0 {
        return None;
    }
    let var = f.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / f.len() as f64;
    let peak = f.iter().fold(0.0f64, |m, v| m.max((v - mean).abs()));
    Some(Flutter {
        mean_hz: mean,
        rms_pct: var.sqrt() / mean * 100.0,
        peak_pct: peak / mean * 100.0,
    })
}

/// Plays [`TONE_HZ`] at -12 dBFS through a device for `secs` seconds and
/// measures the output's middle from the first second on.
pub fn measure(dut: &mut dyn crate::dut::Dut, secs: f64) -> Option<Flutter> {
    let rate = dut.rate();
    let n = (secs * rate as f64) as usize;
    let out = dut.render(
        &Stereo::mono(sine(TONE_HZ, db_to_amp(-12.0), n, rate), rate),
        0,
    );
    let mid = out.mid();
    analyse(&mid[rate as usize..], rate, TONE_HZ)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tone whose frequency swings `dev` (a fraction) at `rate_hz`.
    fn fm(dev: f64, rate_hz: f64, secs: f64) -> Vec<f64> {
        let fs = 44_100.0;
        let mut ph = 0.0f64;
        (0..(secs * fs) as usize)
            .map(|i| {
                let t = i as f64 / fs;
                let f = TONE_HZ * (1.0 + dev * (std::f64::consts::TAU * rate_hz * t).sin());
                ph += std::f64::consts::TAU * f / fs;
                ph.sin()
            })
            .collect()
    }

    #[test]
    fn a_steady_tone_does_not_waver() {
        let f = analyse(&fm(0.0, 4.0, 3.0), 44_100, TONE_HZ).unwrap();
        assert!((f.mean_hz - TONE_HZ).abs() < 0.01, "{f:?}");
        // The isolating filter's edges leave a ripple far under 0.01 %.
        assert!(f.peak_pct < 1e-3, "{f:?}");
    }

    #[test]
    fn a_known_wow_reads_back() {
        for (dev, rate) in [(0.001, 4.0), (0.005, 0.8), (0.0005, 30.0)] {
            // 6.25 s leaves 5 s once the edges are cut: whole cycles of each.
            let f = analyse(&fm(dev, rate, 6.25), 44_100, TONE_HZ).unwrap();
            let pct = dev * 100.0;
            assert!(
                (f.peak_pct / pct - 1.0).abs() < 0.03,
                "{dev} at {rate} Hz: {f:?}"
            );
            let rms = pct / std::f64::consts::SQRT_2;
            assert!(
                (f.rms_pct / rms - 1.0).abs() < 0.05,
                "{dev} at {rate} Hz: {f:?}"
            );
        }
    }
}
