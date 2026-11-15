//! Reverb and delay tails: decay time per octave band (Schroeder
//! integration), early decay, echo density (Abel and Huang), metallic
//! resonances in the tail, and how decorrelated the two sides are.

use crate::fft::{pow2_at_least, power_spectrum};
use crate::wav::Stereo;

/// Octave band centres measured.
pub const OCTAVES: &[f64] = &[125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0];

/// An RBJ band-pass (0 dB peak), run twice for a steeper octave band.
fn band(x: &[f64], hz: f64, rate: u32) -> Vec<f64> {
    let w = std::f64::consts::TAU * hz / rate as f64;
    let q = std::f64::consts::SQRT_2;
    let alpha = w.sin() / (2.0 * q);
    let a0 = 1.0 + alpha;
    let b = [alpha / a0, 0.0, -alpha / a0];
    let a = [-2.0 * w.cos() / a0, (1.0 - alpha) / a0];
    let run = |x: &[f64]| -> Vec<f64> {
        let (mut x1, mut x2, mut y1, mut y2) = (0.0, 0.0, 0.0, 0.0);
        x.iter()
            .map(|&v| {
                let y = b[0] * v + b[1] * x1 + b[2] * x2 - a[0] * y1 - a[1] * y2;
                x2 = x1;
                x1 = v;
                y2 = y1;
                y1 = y;
                y
            })
            .collect()
    };
    run(&run(x))
}

/// The energy decay curve in dB (0 at the start), by backward
/// integration of `ir` squared.
pub fn edc_db(ir: &[f64]) -> Vec<f64> {
    let mut acc = 0.0;
    let mut e: Vec<f64> = ir
        .iter()
        .rev()
        .map(|v| {
            acc += v * v;
            acc
        })
        .collect();
    e.reverse();
    let total = e.first().copied().unwrap_or(0.0).max(1e-300);
    e.iter()
        .map(|v| 10.0 * (v / total).max(1e-30).log10())
        .collect()
}

/// Seconds for the decay to fall 60 dB, from a straight-line fit to the
/// EDC between `from_db` and `to_db` (both negative). None if the EDC
/// never gets that low.
pub fn decay_time(edc: &[f64], from_db: f64, to_db: f64, rate: u32) -> Option<f64> {
    let start = edc.iter().position(|v| *v <= from_db)?;
    let end = edc.iter().position(|v| *v <= to_db)?;
    if end <= start + 2 {
        return None;
    }
    let n = (end - start) as f64;
    let (mut sx, mut sy, mut sxx, mut sxy) = (0.0, 0.0, 0.0, 0.0);
    for (i, y) in edc[start..end].iter().enumerate() {
        let x = i as f64 / rate as f64;
        sx += x;
        sy += y;
        sxx += x * x;
        sxy += x * y;
    }
    let slope = (n * sxy - sx * sy) / (n * sxx - sx * sx);
    (slope < 0.0).then(|| -60.0 / slope)
}

/// Decay measurements of one band.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BandDecay {
    pub hz: f64,
    /// Early decay time (0 to -10 dB, scaled to 60 dB), seconds.
    pub edt: Option<f64>,
    /// T20 (-5 to -25 dB), seconds.
    pub t20: Option<f64>,
    /// T30 (-5 to -35 dB), seconds.
    pub t30: Option<f64>,
}

/// Cuts an impulse response where its 10 ms envelope reaches the noise
/// floor (estimated from its last tenth) plus 10 dB. A clean rendered
/// tail, whose floor is far down, is kept whole.
pub fn truncate_at_noise(ir: &[f64], rate: u32) -> &[f64] {
    let n = ir.len();
    if n < 10 {
        return ir;
    }
    let tail = &ir[n - n / 10..];
    let noise = tail.iter().map(|v| v * v).sum::<f64>() / tail.len() as f64;
    let peak = ir.iter().fold(0.0f64, |m, v| m.max(v * v));
    if noise <= peak * 1e-14 {
        return ir;
    }
    let win = (rate as usize / 100).max(1);
    let limit = noise * 10.0;
    let mut i = 0;
    let mut last_above = 0;
    while i + win <= n {
        let e = ir[i..i + win].iter().map(|v| v * v).sum::<f64>() / win as f64;
        if e > limit {
            last_above = i + win;
        }
        i += win;
    }
    &ir[..last_above.max(win).min(n)]
}

/// EDT, T20 and T30 per octave band of the mid of an impulse response.
pub fn decay(ir: &Stereo) -> Vec<BandDecay> {
    let mid = ir.mid();
    OCTAVES
        .iter()
        .filter(|f| **f < 0.4 * ir.rate as f64)
        .map(|&hz| {
            let b = band(&mid, hz, ir.rate);
            let b = truncate_at_noise(&b, ir.rate);
            let edc = edc_db(b);
            BandDecay {
                hz,
                edt: decay_time(&edc, -0.0001, -10.0, ir.rate),
                t20: decay_time(&edc, -5.0, -25.0, ir.rate),
                t30: decay_time(&edc, -5.0, -35.0, ir.rate),
            }
        })
        .collect()
}

/// The normalised echo density profile (Abel and Huang 2006) over 20 ms
/// windows: about 1 for a fully diffuse (Gaussian) tail, near 0 for a few
/// distinct echoes. One value every `step` frames.
pub fn echo_density(x: &[f64], rate: u32, step: usize) -> Vec<f64> {
    let half = (0.01 * rate as f64) as usize;
    let len = 2 * half + 1;
    let w: Vec<f64> = (0..len)
        .map(|i| 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / (len - 1) as f64).cos())
        .collect();
    let wsum: f64 = w.iter().sum();
    let erfc = 0.317_310_507_862_914_1; // erfc(1/sqrt 2)
    let mut out = Vec::new();
    let mut c = half;
    while c + half < x.len() {
        let seg = &x[c - half..=c + half];
        let var: f64 = seg.iter().zip(&w).map(|(v, k)| k * v * v).sum::<f64>() / wsum;
        let sd = var.sqrt();
        let count: f64 = seg
            .iter()
            .zip(&w)
            .filter(|(v, _)| v.abs() > sd)
            .map(|(_, k)| k)
            .sum::<f64>()
            / wsum;
        out.push(if sd > 0.0 { count / erfc } else { 0.0 });
        c += step;
    }
    out
}

/// Seconds after the start of `ir` (its first frame above -60 dB of its
/// peak) when the echo density first reaches 0.9, or None.
pub fn time_to_diffuse(ir: &Stereo) -> Option<f64> {
    let mid = ir.mid();
    let peak = mid.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    let start = mid.iter().position(|v| v.abs() > peak * 1e-3)?;
    let step = (ir.rate as usize / 1000).max(1);
    let d = echo_density(&mid[start..], ir.rate, step);
    let i = d.iter().position(|v| *v >= 0.9)?;
    Some((i * step + (0.01 * ir.rate as f64) as usize) as f64 / ir.rate as f64)
}

/// How far the tail's loudest narrow peak stands over its third-octave
/// average, in dB: a metallic, ringing tail scores high. The spectrum is
/// averaged over 4096-frame pieces of the tail (from where the decay has
/// fallen 5 dB to where it has fallen 45 dB), each scaled to the same
/// level, from 200 Hz to 10 kHz.
pub fn tail_excess_db(ir: &Stereo) -> Option<f64> {
    let mid = ir.mid();
    let edc = edc_db(&mid);
    let a = edc.iter().position(|v| *v <= -5.0)?;
    let b = edc.iter().position(|v| *v <= -45.0).unwrap_or(mid.len());
    const SEG: usize = 4096;
    if b < a + 2 * SEG {
        return None;
    }
    let win: Vec<f64> = crate::fft::hann_f64(SEG);
    let mut avg = vec![0.0; SEG / 2 + 1];
    let mut count = 0;
    let mut at = a;
    while at + SEG <= b {
        let piece: Vec<f64> = mid[at..at + SEG]
            .iter()
            .zip(&win)
            .map(|(v, w)| v * w)
            .collect();
        let p = power_spectrum(&piece);
        let total: f64 = p.iter().sum::<f64>().max(1e-300);
        for (s, v) in avg.iter_mut().zip(&p) {
            *s += v / total;
        }
        count += 1;
        at += SEG / 2;
    }
    if count < 4 {
        return None;
    }
    let rate = ir.rate as f64;
    let bin = |hz: f64| (hz * SEG as f64 / rate).round() as usize;
    let (lo, hi) = (bin(200.0), bin(10_000.0f64.min(0.45 * rate)));
    let db: Vec<f64> = avg.iter().map(|v| 10.0 * v.max(1e-300).log10()).collect();
    let mut worst = f64::NEG_INFINITY;
    for k in lo..=hi {
        // Third-octave neighbourhood average in power.
        let f = k as f64;
        let (a, b) = (
            (f / 2f64.powf(1.0 / 6.0)) as usize,
            ((f * 2f64.powf(1.0 / 6.0)) as usize).min(avg.len() - 1),
        );
        let mean = avg[a..=b].iter().sum::<f64>() / (b - a + 1) as f64;
        worst = worst.max(db[k] - 10.0 * mean.max(1e-300).log10());
    }
    Some(worst)
}

/// The largest normalised cross-correlation of left and right within
/// +-1 ms, from frame `from` on: 1 for the same signal both sides, near 0
/// for decorrelated ones.
pub fn iacc(s: &Stereo, from: usize) -> f64 {
    let l = &s.left[from.min(s.len())..];
    let r = &s.right[from.min(s.len())..];
    let el: f64 = l.iter().map(|v| v * v).sum();
    let er: f64 = r.iter().map(|v| v * v).sum();
    if el <= 0.0 || er <= 0.0 {
        return 0.0;
    }
    let max_lag = (s.rate as usize / 1000) as isize;
    let n = l.len() as isize;
    let mut best = 0.0f64;
    for lag in -max_lag..=max_lag {
        let mut acc = 0.0;
        for i in 0..n {
            let j = i + lag;
            if (0..n).contains(&j) {
                acc += l[i as usize] * r[j as usize];
            }
        }
        best = best.max(acc.abs());
    }
    best / (el * er).sqrt()
}

/// The impulse response of a device: a unit impulse then `frames` of
/// silence (enough for the tail), after the reported latency.
pub fn impulse_response(dut: &mut dyn crate::dut::Dut, frames: usize, amp: f64) -> Stereo {
    let rate = dut.rate();
    let lat = dut.latency();
    let stim = Stereo::mono(crate::signals::impulse(1, amp), rate);
    let out = dut.render(&stim, frames + lat);
    let k = 1.0 / amp;
    let take = |v: &[f64]| v[lat..].iter().map(|x| x * k).collect::<Vec<f64>>();
    Stereo::new(take(&out.left), take(&out.right), rate)
}

/// Frames to give a tail of `secs` seconds, rounded to a power of two for
/// the analyses that like one.
pub fn tail_frames(secs: f64, rate: u32) -> usize {
    pow2_at_least((secs * rate as f64) as usize)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signals::Rng;

    /// Gaussian noise under an exponential that falls 60 dB in `t60` s.
    fn synthetic(t60: f64, secs: f64, rate: u32, seed: u64) -> Vec<f64> {
        let mut r = Rng::new(seed);
        let k = (-6.907_755_278_982_137 / (t60 * rate as f64)).exp(); // 10^(-3) per t60
        let mut env = 1.0;
        (0..(secs * rate as f64) as usize)
            .map(|_| {
                let v = env * r.gauss();
                env *= k;
                v
            })
            .collect()
    }

    #[test]
    fn decay_times_of_a_synthetic_tail() {
        let x = synthetic(1.5, 4.0, 48_000, 9);
        let ir = Stereo::mono(x, 48_000);
        for b in decay(&ir).iter().filter(|b| b.hz >= 250.0) {
            let t30 = b.t30.unwrap();
            assert!((t30 - 1.5).abs() < 0.08, "{b:?}");
        }
    }

    #[test]
    fn gaussian_noise_is_diffuse_and_clicks_are_not() {
        let mut r = Rng::new(4);
        let noise: Vec<f64> = (0..48_000).map(|_| r.gauss()).collect();
        let d = echo_density(&noise, 48_000, 480);
        let mean = d.iter().sum::<f64>() / d.len() as f64;
        assert!((mean - 1.0).abs() < 0.05, "{mean}");
        let mut clicks = vec![0.0; 48_000];
        for i in (0..48_000).step_by(2400) {
            clicks[i] = 1.0;
        }
        let d = echo_density(&clicks, 48_000, 480);
        assert!(d.iter().all(|v| *v < 0.2));
    }

    #[test]
    fn a_ringing_mode_stands_out() {
        let rate = 48_000;
        let mut x = synthetic(1.0, 2.0, rate, 5);
        let clean = tail_excess_db(&Stereo::mono(x.clone(), rate)).unwrap();
        let k = (-6.9 / (3.0 * rate as f64)).exp();
        let mut env = 0.3;
        for (i, v) in x.iter_mut().enumerate() {
            *v += env * (std::f64::consts::TAU * 1234.0 * i as f64 / rate as f64).sin();
            env *= k;
        }
        let ringing = tail_excess_db(&Stereo::mono(x, rate)).unwrap();
        assert!(clean < 5.0, "clean {clean}");
        assert!(ringing > clean + 6.0, "ringing {ringing} clean {clean}");
    }

    #[test]
    fn iacc_of_same_and_independent_sides() {
        let a = synthetic(1.0, 1.0, 48_000, 1);
        let b = synthetic(1.0, 1.0, 48_000, 2);
        assert!((iacc(&Stereo::mono(a.clone(), 48_000), 0) - 1.0).abs() < 1e-9);
        assert!(iacc(&Stereo::new(a, b, 48_000), 0) < 0.15);
    }
}
