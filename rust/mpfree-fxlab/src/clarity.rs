//! The shape of a reverb's start: how much of its energy arrives early
//! (C50, C80), whether the early decay runs at the same rate as the late
//! one (EDT against T30), and whether a source on one side stays on that
//! side (the FX quality plan's R3 and R10). All from an impulse response
//! of the wet signal alone.

use crate::room;
use crate::wav::Stereo;

/// Energy in the first `ms` after the onset against the energy after it,
/// dB (ISO 3382's C50 and C80 when `ms` is 50 or 80). The onset is the
/// first frame within 20 dB of the peak.
pub fn clarity_db(ir: &Stereo, ms: f64) -> Option<f64> {
    let mid = ir.mid();
    let peak = mid.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    let start = mid.iter().position(|v| v.abs() >= peak * 0.1)?;
    let split = start + (ms / 1000.0 * ir.rate as f64) as usize;
    if split >= mid.len() {
        return None;
    }
    let e = |s: &[f64]| s.iter().map(|v| v * v).sum::<f64>();
    let (early, late) = (e(&mid[start..split]), e(&mid[split..]));
    (late > 0.0).then(|| 10.0 * (early.max(1e-300) / late).log10())
}

/// EDT over T30 at mid frequencies (the mean of the 500 Hz and 1 kHz
/// bands for each): 1 for a single straight decay, under 1 when the start
/// falls fast and a fainter tail hangs on, over 1 when the reverb builds.
pub fn edt_over_t30(ir: &Stereo) -> Option<f64> {
    let bands = room::decay(ir);
    let get = |hz: f64, f: fn(&room::BandDecay) -> Option<f64>| {
        bands.iter().find(|b| b.hz == hz).and_then(f)
    };
    let edt = 0.5 * (get(500.0, |b| b.edt)? + get(1000.0, |b| b.edt)?);
    let t30 = 0.5 * (get(500.0, |b| b.t30.or(b.t20))? + get(1000.0, |b| b.t30.or(b.t20))?);
    Some(edt / t30)
}

/// Left against right energy, dB, in the first `ms` after the onset: fed
/// from the left only, a reverb that keeps the source's side reads well
/// above 0.
pub fn early_side_db(ir: &Stereo, ms: f64) -> Option<f64> {
    let e: Vec<f64> = ir
        .left
        .iter()
        .zip(&ir.right)
        .map(|(l, r)| l.abs().max(r.abs()))
        .collect();
    let peak = e.iter().fold(0.0f64, |m, v| m.max(*v));
    let start = e.iter().position(|v| *v >= peak * 0.1)?;
    let end = (start + (ms / 1000.0 * ir.rate as f64) as usize).min(ir.len());
    let en = |s: &[f64]| s[start..end].iter().map(|v| v * v).sum::<f64>();
    let (l, r) = (en(&ir.left), en(&ir.right));
    Some(10.0 * (l.max(1e-300) / r.max(1e-300)).log10())
}

/// The impulse response of a device fed from the left side only.
pub fn left_impulse_response(dut: &mut dyn crate::dut::Dut, frames: usize, amp: f64) -> Stereo {
    let rate = dut.rate();
    let lat = dut.latency();
    let mut l = vec![0.0; 1];
    l[0] = amp;
    let stim = Stereo::new(l, vec![0.0], rate);
    let out = dut.render(&stim, frames + lat);
    let k = 1.0 / amp;
    let take = |v: &[f64]| v[lat..].iter().map(|x| x * k).collect::<Vec<f64>>();
    Stereo::new(take(&out.left), take(&out.right), rate)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signals::Rng;

    fn tail(t60: f64, rate: u32, seed: u64) -> Vec<f64> {
        let mut r = Rng::new(seed);
        let k = (-6.907_755 / (t60 * rate as f64)).exp();
        let mut env = 1.0;
        (0..(2.5 * t60 * rate as f64) as usize)
            .map(|_| {
                let v = env * r.gauss();
                env *= k;
                v
            })
            .collect()
    }

    #[test]
    fn a_single_slope_reads_one_and_its_clarity_follows_its_decay() {
        let ir = Stereo::new(tail(1.0, 44_100, 1), tail(1.0, 44_100, 2), 44_100);
        let r = edt_over_t30(&ir).unwrap();
        assert!((r - 1.0).abs() < 0.1, "{r}");
        // An exponential falling 60 dB in T has C80 = 10 log10(10^(6*0.08/T) - 1).
        let want = 10.0 * (10f64.powf(6.0 * 0.08) - 1.0).log10();
        let c80 = clarity_db(&ir, 80.0).unwrap();
        assert!((c80 - want).abs() < 0.5, "{c80} against {want}");
        assert!(early_side_db(&ir, 50.0).unwrap().abs() < 1.0);
    }

    #[test]
    fn a_left_source_reads_left() {
        let l = tail(0.5, 44_100, 3);
        let r: Vec<f64> = tail(0.5, 44_100, 4).iter().map(|v| v * 0.5).collect();
        let s = early_side_db(&Stereo::new(l, r, 44_100), 50.0).unwrap();
        assert!((s - 6.02).abs() < 1.0, "{s}");
    }
}
