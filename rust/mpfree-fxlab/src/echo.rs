//! A delay's echoes, one by one, from its impulse response: when each
//! repeat lands, how loud it is and how its tone changes from repeat to
//! repeat. Built for the FX quality plan's delay targets (D1, D2, D8,
//! D10; `docs/dev/design/fx-quality.md`).

use crate::fft::Fft;
use crate::wav::Stereo;

/// The bands each repeat's level is read at.
pub const BANDS: [f64; 4] = [1000.0, 5000.0, 10_000.0, 15_000.0];

/// Frames read around each repeat (the repeats must be further apart).
const PIECE: usize = 512;

/// One repeat.
#[derive(Clone, Debug, PartialEq)]
pub struct Repeat {
    /// 1 for the first echo.
    pub n: usize,
    /// Where it lands, in frames from the impulse: its group delay from
    /// 1 to 4 kHz (the slope of its phase), so a fractional time reads as
    /// a fraction, an all-pass read reads at its true delay (its energy
    /// centroid sits up to half a frame late) and a low cut's long, quiet
    /// tail doesn't pull it.
    pub frame: f64,
    /// Its level at each of [`BANDS`] against the impulse, dB (both sides'
    /// power, so a repeat on one side reads 3 dB under one on both).
    pub gain_db: [f64; 4],
    /// Left against right energy, dB (positive: louder on the left).
    pub balance_db: f64,
}

impl Repeat {
    /// The level at band `b` relative to 1 kHz, dB: its tilt.
    pub fn tilt(&self, b: usize) -> f64 {
        self.gain_db[b] - self.gain_db[0]
    }
}

/// The first `count` repeats of `ir` (an impulse response of amplitude
/// `amp`, latency removed), expected near multiples of `period` frames.
/// A repeat is looked for within a quarter period (at most 2048 frames)
/// of where it should be; one that is missing or silent ends the list.
pub fn repeats(ir: &Stereo, amp: f64, period: f64, count: usize) -> Vec<Repeat> {
    let rate = ir.rate as f64;
    let n = ir.len();
    let fft = Fft::new(8192);
    let search = ((period / 4.0) as usize).clamp(16, 2048);
    let mut out = Vec::new();
    for k in 1..=count {
        let expect = k as f64 * period;
        let lo = (expect as usize).saturating_sub(search);
        let hi = ((expect as usize) + search).min(n);
        if lo + PIECE >= hi {
            break;
        }
        let energy = |i: usize| ir.left[i] * ir.left[i] + ir.right[i] * ir.right[i];
        let peak = (lo..hi).max_by(|a, b| energy(*a).total_cmp(&energy(*b)));
        let Some(peak) = peak else { break };
        if energy(peak) <= 1e-24 * amp * amp {
            break;
        }
        // The repeat's spectrum: PIECE frames centred on it, unwindowed
        // (the echo sits whole inside, the neighbours far outside).
        let start = peak.saturating_sub(PIECE / 2);
        let frame = start as f64 + group_delay(ir, start, &fft, rate);
        let spectrum = |x: &[f64]| -> Vec<f64> {
            let mut re = vec![0.0; fft.len()];
            let mut im = vec![0.0; fft.len()];
            for (j, v) in x.iter().skip(start).take(PIECE).enumerate() {
                re[j] = v / amp;
            }
            fft.forward(&mut re, &mut im);
            BANDS
                .iter()
                .map(|f| {
                    let k = (f * fft.len() as f64 / rate).round() as usize;
                    re[k] * re[k] + im[k] * im[k]
                })
                .collect()
        };
        let (pl, pr) = (spectrum(&ir.left), spectrum(&ir.right));
        let mut gain_db = [0.0; 4];
        for i in 0..4 {
            gain_db[i] = 10.0 * ((pl[i] + pr[i]) / 2.0).max(1e-30).log10();
        }
        let el: f64 = ir.left[start..(start + PIECE).min(n)]
            .iter()
            .map(|v| v * v)
            .sum();
        let er: f64 = ir.right[start..(start + PIECE).min(n)]
            .iter()
            .map(|v| v * v)
            .sum();
        out.push(Repeat {
            n: k,
            frame,
            gain_db,
            balance_db: 10.0 * (el.max(1e-30) / er.max(1e-30)).log10(),
        });
    }
    out
}

/// The group delay of both sides summed, `PIECE` frames from `start`,
/// in frames: a straight line through the unwrapped phase of every bin
/// from 1 to 4 kHz.
fn group_delay(ir: &Stereo, start: usize, fft: &Fft, rate: f64) -> f64 {
    let len = fft.len();
    let mut re = vec![0.0; len];
    let mut im = vec![0.0; len];
    for j in 0..PIECE.min(ir.len().saturating_sub(start)) {
        re[j] = ir.left[start + j] + ir.right[start + j];
    }
    fft.forward(&mut re, &mut im);
    let bin = |hz: f64| (hz * len as f64 / rate).round() as usize;
    let (mut sx, mut sy, mut sxx, mut sxy, mut n) = (0.0, 0.0, 0.0, 0.0, 0.0);
    let mut was: Option<f64> = None;
    for k in bin(1000.0)..=bin(4000.0) {
        let mut p = im[k].atan2(re[k]);
        if let Some(w) = was {
            while p - w > std::f64::consts::PI {
                p -= std::f64::consts::TAU;
            }
            while p - w < -std::f64::consts::PI {
                p += std::f64::consts::TAU;
            }
        }
        was = Some(p);
        let x = std::f64::consts::TAU * k as f64 / len as f64;
        sx += x;
        sy += p;
        sxx += x * x;
        sxy += x * p;
        n += 1.0;
    }
    -(n * sxy - sx * sy) / (n * sxx - sx * sx)
}

/// How much darker each repeat gets at band `b` than the one before,
/// relative to 1 kHz, in dB per repeat (positive: darker), from a
/// straight line through every repeat's tilt. None with fewer than two.
pub fn loss_per_repeat(reps: &[Repeat], b: usize) -> Option<f64> {
    if reps.len() < 2 {
        return None;
    }
    let n = reps.len() as f64;
    let (mut sx, mut sy, mut sxx, mut sxy) = (0.0, 0.0, 0.0, 0.0);
    for r in reps {
        let (x, y) = (r.n as f64, r.tilt(b));
        sx += x;
        sy += y;
        sxx += x * x;
        sxy += x * y;
    }
    Some(-(n * sxy - sx * sy) / (n * sxx - sx * sx))
}

/// The largest distance of a repeat from `n * period`, frames.
pub fn worst_time_error(reps: &[Repeat], period: f64) -> f64 {
    reps.iter()
        .map(|r| (r.frame - r.n as f64 * period).abs())
        .fold(0.0, f64::max)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Echoes every `period` frames, each one through the 3-tap filter
    /// [1/4, 1/2, 1/4] once more than the last and `gain` quieter.
    fn synthetic(period: usize, count: usize, gain: f64) -> Stereo {
        let mut ir = vec![0.0; period * (count + 1) + 64];
        let mut pulse = vec![1.0];
        for k in 1..=count {
            let mut next = vec![0.0; pulse.len() + 2];
            for (i, v) in pulse.iter().enumerate() {
                next[i] += 0.25 * v;
                next[i + 1] += 0.5 * v;
                next[i + 2] += 0.25 * v;
            }
            pulse = next;
            let at = k * period - (pulse.len() - 1) / 2;
            for (i, v) in pulse.iter().enumerate() {
                ir[at + i] += v * gain.powi(k as i32);
            }
        }
        Stereo::mono(ir, 44_100)
    }

    #[test]
    fn a_known_loop_filter_is_read_back() {
        let ir = synthetic(4000, 6, 0.8);
        let reps = repeats(&ir, 1.0, 4000.0, 6);
        assert_eq!(reps.len(), 6);
        assert!(worst_time_error(&reps, 4000.0) < 1e-6, "{reps:?}");
        // |H(f)| = cos^2(pi f / fs), relative to 1 kHz.
        let h = |f: f64| 20.0 * (std::f64::consts::PI * f / 44_100.0).cos().powi(2).log10();
        for b in 1..4 {
            let want = h(BANDS[0]) - h(BANDS[b]);
            let got = loss_per_repeat(&reps, b).unwrap();
            // Read at the nearest FFT bin, a few Hz off the band.
            assert!((got - want).abs() < 0.01, "band {b}: {got} against {want}");
        }
        // Each repeat 0.8 times the last at 1 kHz, plus the filter.
        let step = reps[1].gain_db[0] - reps[0].gain_db[0];
        assert!((step - (20.0 * 0.8f64.log10() + h(1000.0))).abs() < 0.01);
    }

    #[test]
    fn an_all_pass_read_half_a_frame_late_reads_at_its_delay() {
        // Repeats every 3000.5 frames, each through a first-order all-pass
        // (Thiran) for its fraction, as the delays read: from the whole
        // frame `w` with a fraction d from 0.5 to 1.5 past it, eta =
        // (1 - d) / (1 + d). At half a frame the energy centroid sits half
        // a frame late; the group delay is on time.
        let period = 3000.5;
        let mut ir = vec![0.0; 4 * 3001 + 64];
        for k in 1..=3usize {
            let at = k as f64 * period;
            let mut w = at.floor();
            if at - w < 0.5 {
                w -= 1.0;
            }
            let d = at - w;
            let eta = (1.0 - d) / (1.0 + d);
            let g = 0.5f64.powi(k as i32);
            // The all-pass's impulse response: eta, then (1 - eta^2)
            // (-eta)^(n - 1).
            ir[w as usize] += eta * g;
            for n in 1..40 {
                ir[w as usize + n] += (1.0 - eta * eta) * (-eta).powi(n as i32 - 1) * g;
            }
        }
        let ir = Stereo::mono(ir, 44_100);
        let reps = repeats(&ir, 1.0, period, 3);
        assert_eq!(reps.len(), 3);
        // Within 0.02 frame (the all-pass is exact at low frequencies and
        // a hundredth of a frame later by 4 kHz).
        assert!(worst_time_error(&reps, period) < 0.02, "{reps:?}");
    }

    #[test]
    fn a_flat_loop_loses_nothing_and_a_late_repeat_shows() {
        let mut ir = synthetic(3000, 4, 0.5);
        // No filter: put plain impulses instead.
        ir.left.iter_mut().for_each(|v| *v = 0.0);
        for k in 1..=4 {
            ir.left[k * 3000 + usize::from(k == 3)] = 0.5f64.powi(k as i32);
        }
        ir.right = ir.left.clone();
        let reps = repeats(&ir, 1.0, 3000.0, 4);
        assert!(loss_per_repeat(&reps, 3).unwrap().abs() < 1e-9);
        assert!((worst_time_error(&reps, 3000.0) - 1.0).abs() < 1e-9);
        assert!(reps.iter().all(|r| r.balance_db.abs() < 1e-9));
    }
}
