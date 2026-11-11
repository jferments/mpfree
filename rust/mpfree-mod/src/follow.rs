//! Envelope followers: a strip's loudness as a mod source (pumping,
//! ducking a filter, opening a reverb with the drums).
//!
//! A follower runs per sample on the audio the host hands the mod engine
//! (the louder side, rectified, through a one-pole with separate attack
//! and release), so it gives the same values however the blocks are cut.
//! Its output is the level in dB mapped from a floor to 0 dBFS onto 0 to
//! 1, so the source moves evenly with how loud it sounds.

use mpfree_node::{Curve, ParamInfo, Unit};

pub mod ids {
    pub const ATTACK: u16 = 0;
    pub const RELEASE: u16 = 1;
    /// The level that reads as 0; 0 dBFS reads as 1.
    pub const FLOOR: u16 = 2;
    /// Gain before the follower, for quiet sources.
    pub const GAIN: u16 = 3;
}

pub static FOLLOWER_PARAMS: [ParamInfo; 4] = [
    ParamInfo {
        id: ids::ATTACK,
        name: "Attack",
        group: "Follower",
        min: 0.1,
        max: 500.0,
        default: 5.0,
        unit: Unit::Ms,
        curve: Curve::Log,
        readonly: false,
    },
    ParamInfo {
        id: ids::RELEASE,
        name: "Release",
        group: "Follower",
        min: 5.0,
        max: 5000.0,
        default: 150.0,
        unit: Unit::Ms,
        curve: Curve::Log,
        readonly: false,
    },
    ParamInfo {
        id: ids::FLOOR,
        name: "Floor",
        group: "Follower",
        min: -80.0,
        max: -6.0,
        default: -48.0,
        unit: Unit::Db,
        curve: Curve::Linear,
        readonly: false,
    },
    ParamInfo {
        id: ids::GAIN,
        name: "Gain",
        group: "Follower",
        min: -24.0,
        max: 24.0,
        default: 0.0,
        unit: Unit::Db,
        curve: Curve::Linear,
        readonly: false,
    },
];

#[derive(Clone, Debug)]
pub struct Follower {
    attack_ms: f32,
    release_ms: f32,
    floor_db: f32,
    gain_db: f32,
    gain: f32,
    att: f32,
    rel: f32,
    env: f32,
    rate: u32,
}

impl Follower {
    pub fn new(rate: u32) -> Follower {
        let mut f = Follower {
            attack_ms: 5.0,
            release_ms: 150.0,
            floor_db: -48.0,
            gain_db: 0.0,
            gain: 1.0,
            att: 0.0,
            rel: 0.0,
            env: 0.0,
            rate: rate.max(1),
        };
        f.coefs();
        f
    }

    pub fn prepare(&mut self, rate: u32) {
        self.rate = rate.max(1);
        self.coefs();
    }

    fn coefs(&mut self) {
        let c = |ms: f32| 1.0 - (-1.0 / (ms * 0.001 * self.rate as f32).max(1.0)).exp();
        self.att = c(self.attack_ms);
        self.rel = c(self.release_ms);
        self.gain = 10f32.powf(self.gain_db / 20.0);
    }

    pub fn reset(&mut self) {
        self.env = 0.0;
    }

    pub fn set_param(&mut self, id: u16, v: f32) {
        let Some(p) = FOLLOWER_PARAMS.iter().find(|p| p.id == id) else {
            return;
        };
        let v = p.clamp(v);
        match id {
            ids::ATTACK => self.attack_ms = v,
            ids::RELEASE => self.release_ms = v,
            ids::FLOOR => self.floor_db = v,
            ids::GAIN => self.gain_db = v,
            _ => {}
        }
        self.coefs();
    }

    pub fn param(&self, id: u16) -> f32 {
        match id {
            ids::ATTACK => self.attack_ms,
            ids::RELEASE => self.release_ms,
            ids::FLOOR => self.floor_db,
            ids::GAIN => self.gain_db,
            _ => f32::NAN,
        }
    }

    #[inline]
    fn step(&mut self, x: f32) {
        // Non-finite input is ignored rather than poisoning the envelope.
        let x = if x.is_finite() { x } else { 0.0 };
        let c = if x > self.env { self.att } else { self.rel };
        self.env += (x - self.env) * c;
        // Flush denormals in long silences, per sample so the block size
        // never shows.
        if self.env < 1e-9 {
            self.env = 0.0;
        }
    }

    /// Follows `left` and `right` (the same length), sample by sample.
    pub fn run(&mut self, left: &[f32], right: &[f32]) {
        for (l, r) in left.iter().zip(right) {
            self.step(l.abs().max(r.abs()) * self.gain);
        }
    }

    /// Silence of `frames` (no audio given): the same as running zeros.
    pub fn decay(&mut self, frames: usize) {
        for _ in 0..frames {
            if self.env == 0.0 {
                break;
            }
            self.step(0.0);
        }
    }

    /// The level now, 0 (at or below the floor) to 1 (0 dBFS or more).
    pub fn value(&self) -> f32 {
        if self.env <= 0.0 {
            return 0.0;
        }
        let db = 20.0 * self.env.log10();
        ((db - self.floor_db) / -self.floor_db).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follows_level_and_falls_back() {
        let mut f = Follower::new(48_000);
        assert_eq!(f.value(), 0.0);
        let loud = vec![0.5f32; 4800];
        f.run(&loud, &loud);
        // -6 dB on a -48 dB floor reads 42/48.
        assert!((f.value() - 42.0 / 48.0).abs() < 0.01, "{}", f.value());
        let quiet = vec![0.0f32; 48_000];
        f.run(&quiet, &quiet);
        assert_eq!(f.value(), 0.0);
    }

    #[test]
    fn same_result_however_the_audio_is_cut() {
        let sig: Vec<f32> = (0..9000)
            .map(|i| ((i as f32) * 0.01).sin() * if i % 3000 < 1500 { 0.8 } else { 0.05 })
            .collect();
        let mut a = Follower::new(44_100);
        a.run(&sig, &sig);
        let mut b = Follower::new(44_100);
        for c in sig.chunks(37) {
            b.run(c, c);
        }
        assert_eq!(a.value(), b.value());
    }

    #[test]
    fn ignores_nan() {
        let mut f = Follower::new(48_000);
        f.run(&[f32::NAN, 0.5, f32::INFINITY], &[0.0; 3]);
        assert!(f.value().is_finite());
    }
}
