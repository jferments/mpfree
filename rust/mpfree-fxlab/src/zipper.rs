//! Zipper noise and clicks: each continuous parameter is swept across its
//! whole range in 50 ms under a steady sine, and the sharpest step in the
//! output (its largest second difference) is compared with the sharpest
//! step while the parameter rests at either end. A smooth control stays
//! near 0 dB; a zipper or a click shows as a spike well above.

use mpfree_fx::mpfree_node::{Curve, ParamInfo};

use crate::dut::{Change, Dut};
use crate::signals::{db_to_amp, sine};
use crate::wav::Stereo;

/// The result for one parameter.
#[derive(Clone, Debug, PartialEq)]
pub struct Zip {
    pub id: u16,
    pub name: String,
    /// The sweep's sharpest step against the resting ends', dB.
    pub step_db: f64,
}

/// Whether a parameter is swept (continuous and writable).
pub fn sweepable(p: &ParamInfo) -> bool {
    !p.readonly && matches!(p.curve, Curve::Linear | Curve::Log) && p.max > p.min
}

fn max_d2(x: &[f64]) -> f64 {
    x.windows(3)
        .fold(0.0f64, |m, w| m.max((w[2] - 2.0 * w[1] + w[0]).abs()))
}

/// Sweeps one parameter: rest at its minimum, sweep up over `sweep_ms`,
/// rest at its maximum, sweep down, rest; under a 997 Hz sine at -12 dBFS.
pub fn sweep_param(dut: &mut dyn Dut, p: &ParamInfo, sweep_ms: f64) -> Zip {
    let rate = dut.rate();
    let rest = (0.4 * rate as f64) as usize;
    let sweep = ((sweep_ms / 1000.0) * rate as f64).max(1.0) as usize;
    let total = 3 * rest + 2 * sweep;
    let x = sine(997.0, db_to_amp(-12.0), total, rate);
    let mut changes = vec![Change {
        frame: 0,
        id: p.id,
        value: p.min as f64,
    }];
    let steps = (sweep / 32).max(1);
    for k in 1..=steps {
        let t = k as f64 / steps as f64;
        changes.push(Change {
            frame: rest + k * sweep / steps - 1,
            id: p.id,
            value: p.from_unit(t as f32) as f64,
        });
    }
    for k in 1..=steps {
        let t = 1.0 - k as f64 / steps as f64;
        changes.push(Change {
            frame: 2 * rest + sweep + k * sweep / steps - 1,
            id: p.id,
            value: p.from_unit(t as f32) as f64,
        });
    }
    dut.reset();
    // Rest values first so the node starts settled at the minimum.
    dut.set(p.id, p.min as f64);
    let out = dut.process(&Stereo::mono(x, rate), &changes);
    let y = &out.left;
    let guard = rate as usize / 20; // 50 ms for the end rests to settle
    let ref_lo = max_d2(&y[rest / 2..rest]);
    let ref_hi = max_d2(&y[rest + sweep + guard..2 * rest + sweep]);
    let reference = ref_lo.max(ref_hi).max(1e-12);
    let moving = max_d2(&y[rest..rest + sweep + guard])
        .max(max_d2(&y[2 * rest + sweep..2 * rest + 2 * sweep + guard]));
    Zip {
        id: p.id,
        name: p.name.to_string(),
        step_db: 20.0 * (moving / reference).max(1e-12).log10(),
    }
}

/// Every sweepable parameter, from the device's defaults each time.
pub fn sweep_all(make: &mut dyn FnMut() -> Box<dyn Dut>, sweep_ms: f64) -> Vec<Zip> {
    let params = make().params();
    params
        .iter()
        .filter(|p| sweepable(p))
        .map(|p| sweep_param(make().as_mut(), p, sweep_ms))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dut::NodeDut;

    #[test]
    fn the_eq_output_glides_without_a_zipper() {
        let mut make = || -> Box<dyn Dut> {
            Box::new(NodeDut::from_registry("mpfree.fx.eq", 48_000).unwrap())
        };
        let z = sweep_all(&mut make, 50.0);
        assert!(!z.is_empty());
        for r in &z {
            assert!(r.step_db < 12.0, "{r:?}");
        }
    }
}
