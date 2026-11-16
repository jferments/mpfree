//! How loud an effect is as its Mix knob turns: the loudness change
//! (BS.1770) against the dry signal on steady pink noise and on drum-like
//! hits, at each of a list of Mix settings. A delay or reverb whose Mix
//! crossfades linearly dips in the middle, because its wet signal is a
//! different moment of the music and the powers add (the FX quality
//! plan's D6 and R7). [`Law`] gives the gains each Mix curve should make,
//! for checking an engine against its own curve.

use crate::dut::Dut;
use crate::loudness;
use crate::signals::{db_to_amp, pink};
use crate::wav::Stereo;

/// The Mix settings measured by default, percent.
pub const MIXES: [f64; 6] = [0.0, 10.0, 25.0, 50.0, 75.0, 100.0];

/// A Mix curve: the dry and wet gains for a Mix from 0 to 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Law {
    /// `(1 - m, m)`: right for a wet signal correlated with the dry.
    Linear,
    /// `(cos(m pi/2), sin(m pi/2))`: constant power for an uncorrelated
    /// wet signal.
    EqualPower,
    /// The dry at full to half way, then fading; the wet full from half
    /// way (JUCE calls it "balanced").
    DryFirst,
}

impl Law {
    pub fn gains(self, mix: f64) -> (f64, f64) {
        let m = mix.clamp(0.0, 1.0);
        match self {
            Law::Linear => (1.0 - m, m),
            Law::EqualPower => {
                let a = m * std::f64::consts::FRAC_PI_2;
                (a.cos(), a.sin())
            }
            Law::DryFirst => ((2.0 - 2.0 * m).min(1.0), (2.0 * m).min(1.0)),
        }
    }

    /// The loudness change this curve makes at `mix` (0 to 1) when the wet
    /// signal is uncorrelated with the dry and `wet_db` louder than it.
    pub fn expected_db(self, mix: f64, wet_db: f64) -> f64 {
        let (d, w) = self.gains(mix);
        10.0 * (d * d + w * w * db_to_amp(wet_db).powi(2))
            .max(1e-30)
            .log10()
    }
}

/// A drum-like test signal: a 40 ms burst of pink noise, falling 6 ms to
/// 1/e, every half second, peaking near -6 dBFS.
pub fn hits(secs: f64, rate: u32) -> Stereo {
    let n = (secs * rate as f64) as usize;
    let noise = pink(n, 0.3, 3);
    let every = rate as usize / 2;
    let len = (0.04 * rate as f64) as usize;
    let mut x = vec![0.0; n];
    let mut at = 0;
    while at + len < n {
        for i in 0..len {
            let env = (-(i as f64) / (0.006 * rate as f64)).exp();
            x[at + i] = noise[at + i] * env * 2.0;
        }
        at += every;
    }
    Stereo::mono(x, rate)
}

/// One Mix setting's loudness change against the dry, LU.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub mix: f64,
    pub pink_lu: f64,
    pub hits_lu: f64,
}

/// Measures a fresh device from `make` at each of `mixes` (percent,
/// written to parameter `mix_id`) on 6 s of pink noise at -18 dBFS and
/// 6 s of [`hits`]; the first and last second are left out so a reverb's
/// build-up and the cut-off don't count.
pub fn measure(make: &mut dyn FnMut() -> Box<dyn Dut>, mix_id: u16, mixes: &[f64]) -> Vec<Point> {
    let rate = make().rate();
    let p = Stereo::mono(pink(6 * rate as usize, db_to_amp(-18.0), 7), rate);
    let h = hits(6.0, rate);
    let a = rate as usize;
    let lu = |d: &mut dyn Dut, x: &Stereo| {
        let lat = d.latency();
        let out = d.render(x, lat);
        let body = out.slice(a + lat, x.len() - 2 * a);
        loudness::integrated(&body) - loudness::integrated(&x.slice(a, x.len() - 2 * a))
    };
    mixes
        .iter()
        .map(|&mix| {
            let mut d = make();
            d.set(mix_id, mix);
            let pink_lu = lu(d.as_mut(), &p);
            let hits_lu = lu(d.as_mut(), &h);
            Point {
                mix,
                pink_lu,
                hits_lu,
            }
        })
        .collect()
}

/// The deepest dip (the lowest change, LU) and the highest bump over
/// `points` on pink noise.
pub fn dip_and_bump(points: &[Point]) -> (f64, f64) {
    points.iter().fold((0.0f64, 0.0f64), |(lo, hi), p| {
        (lo.min(p.pink_lu), hi.max(p.pink_lu))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dut::Change;
    use mpfree_fx::mpfree_node::ParamInfo;

    /// A dry/wet mix of the input with an uncorrelated signal of the same
    /// loudness (the input reversed in time), by `law`.
    struct Mixer {
        law: Law,
        mix: f64,
    }

    impl Dut for Mixer {
        fn name(&self) -> String {
            "mixer".into()
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
        fn set(&mut self, _: u16, v: f64) {
            self.mix = v / 100.0;
        }
        fn process(&mut self, x: &Stereo, _: &[Change]) -> Stereo {
            let (d, w) = self.law.gains(self.mix);
            let rev = |v: &Vec<f64>| -> Vec<f64> {
                let r: Vec<f64> = v.iter().rev().copied().collect();
                v.iter().zip(&r).map(|(a, b)| d * a + w * b).collect()
            };
            Stereo::new(rev(&x.left), rev(&x.right), x.rate)
        }
    }

    #[test]
    fn each_law_measures_as_its_formula() {
        for law in [Law::Linear, Law::EqualPower, Law::DryFirst] {
            let mut make = || -> Box<dyn Dut> { Box::new(Mixer { law, mix: 0.0 }) };
            for p in measure(&mut make, 0, &MIXES) {
                let want = law.expected_db(p.mix / 100.0, 0.0);
                assert!(
                    (p.pink_lu - want).abs() < 0.3,
                    "{law:?} {p:?} against {want:.2}"
                );
            }
        }
        // Linear dips 3 dB at half; equal power stays put.
        assert!((Law::Linear.expected_db(0.5, 0.0) + 3.01).abs() < 0.01);
        assert!(Law::EqualPower.expected_db(0.37, 0.0).abs() < 1e-9);
        assert!((Law::DryFirst.expected_db(0.5, 0.0) - 3.01).abs() < 0.01);
    }
}
