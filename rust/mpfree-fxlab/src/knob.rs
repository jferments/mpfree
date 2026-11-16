//! Does a control do what its label says? A reverb's Decay against the
//! decay measured from its impulse response, and its Pre-delay against
//! when the reverb starts (the FX quality plan's R1 and R12).

use crate::dut::Dut;
use crate::room;
use crate::wav::Stereo;

/// Seconds a tail takes to fall 60 dB at mid frequencies: the mean of the
/// 500 Hz and 1 kHz octave bands' T30 (T20 where the tail is too short or
/// too noisy for T30), the usual room-acoustics convention.
pub fn t60_mid(ir: &Stereo) -> Option<f64> {
    let bands = room::decay(ir);
    let t = |hz: f64| {
        bands
            .iter()
            .find(|b| b.hz == hz)
            .and_then(|b| b.t30.or(b.t20))
    };
    Some(0.5 * (t(500.0)? + t(1000.0)?))
}

/// One Decay setting and what it measured.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DecayPoint {
    /// The setting, seconds.
    pub set_s: f64,
    /// [`t60_mid`], seconds.
    pub measured_s: f64,
}

impl DecayPoint {
    /// How far the measured decay is from the setting, percent.
    pub fn error_pct(&self) -> f64 {
        (self.measured_s / self.set_s - 1.0) * 100.0
    }
}

/// Sets `decay_id` to each of `settings_ms` (milliseconds, the unit
/// MPFree's reverbs use) on a fresh device and measures [`t60_mid`] from
/// an impulse response 1.6 times the setting plus half a second long.
pub fn decay(
    make: &mut dyn FnMut() -> Box<dyn Dut>,
    decay_id: u16,
    settings_ms: &[f64],
) -> Vec<DecayPoint> {
    settings_ms
        .iter()
        .filter_map(|&ms| {
            let mut d = make();
            d.set(decay_id, ms);
            let rate = d.rate();
            let frames = ((ms / 1000.0 * 1.6 + 0.5) * rate as f64) as usize;
            let ir = room::impulse_response(d.as_mut(), frames, 0.5);
            Some(DecayPoint {
                set_s: ms / 1000.0,
                measured_s: t60_mid(&ir)?,
            })
        })
        .collect()
}

/// The first frame where `ir`'s louder side reaches `below_db` under its
/// peak (a negative number), or None for silence.
pub fn onset(ir: &Stereo, below_db: f64) -> Option<usize> {
    let e: Vec<f64> = ir
        .left
        .iter()
        .zip(&ir.right)
        .map(|(l, r)| l.abs().max(r.abs()))
        .collect();
    let peak = e.iter().fold(0.0f64, |m, v| m.max(*v));
    if peak <= 0.0 {
        return None;
    }
    let limit = peak * 10f64.powf(below_db / 20.0);
    e.iter().position(|v| *v >= limit)
}

/// How far the reverb's start moves from where Pre-delay says, frames:
/// for each of `settings_ms`, the onset (-30 dB from the peak) minus the
/// onset at the first setting, minus the difference of the settings.
pub fn predelay_errors(
    make: &mut dyn FnMut() -> Box<dyn Dut>,
    id: u16,
    settings_ms: &[f64],
) -> Vec<(f64, f64)> {
    let mut first: Option<(f64, usize)> = None;
    let mut out = Vec::new();
    for &ms in settings_ms {
        let mut d = make();
        d.set(id, ms);
        let rate = d.rate();
        let ir = room::impulse_response(
            d.as_mut(),
            (ms / 1000.0 * rate as f64) as usize + rate as usize / 2,
            0.5,
        );
        let Some(at) = onset(&ir, -30.0) else {
            continue;
        };
        match first {
            None => first = Some((ms, at)),
            Some((ms0, at0)) => {
                let want = (ms - ms0) / 1000.0 * rate as f64;
                out.push((ms, (at as f64 - at0 as f64) - want));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dut::Change;
    use crate::signals::Rng;
    use mpfree_fx::mpfree_node::ParamInfo;

    /// A "reverb" whose tail is noise falling 60 dB in Decay ms, starting
    /// Pre-delay ms after the input (id 0 decay, id 1 pre-delay).
    struct Noise {
        decay_ms: f64,
        pre_ms: f64,
    }

    impl Dut for Noise {
        fn name(&self) -> String {
            "noise".into()
        }
        fn rate(&self) -> u32 {
            44_100
        }
        fn latency(&self) -> usize {
            0
        }
        fn reset(&mut self) {}
        fn params(&self) -> Vec<ParamInfo> {
            Vec::new()
        }
        fn set(&mut self, id: u16, v: f64) {
            if id == 0 {
                self.decay_ms = v;
            } else {
                self.pre_ms = v;
            }
        }
        fn process(&mut self, x: &Stereo, _: &[Change]) -> Stereo {
            let n = x.len();
            let mut r = Rng::new(3);
            let k = (-6.907_755 / (self.decay_ms / 1000.0 * 44_100.0)).exp();
            let mut ir = vec![0.0; n];
            let start = (self.pre_ms / 1000.0 * 44_100.0) as usize + 100;
            let mut env = 1.0;
            for v in ir.iter_mut().skip(start) {
                *v = env * r.gauss();
                env *= k;
            }
            let y = crate::fft::convolve(&x.left, &ir);
            Stereo::mono(y[..n].to_vec(), 44_100)
        }
    }

    #[test]
    fn decay_and_predelay_of_a_known_tail() {
        let mut make = || -> Box<dyn Dut> {
            Box::new(Noise {
                decay_ms: 1000.0,
                pre_ms: 0.0,
            })
        };
        for p in decay(&mut make, 0, &[800.0, 2000.0]) {
            assert!(p.error_pct().abs() < 5.0, "{p:?}");
        }
        let e = predelay_errors(&mut make, 1, &[0.0, 20.0, 80.0]);
        assert_eq!(e.len(), 2);
        assert!(e.iter().all(|(_, f)| f.abs() <= 1.0), "{e:?}");
    }
}
