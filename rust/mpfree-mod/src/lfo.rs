//! LFOs: the mod matrix's moving sources.
//!
//! An LFO runs once per tick, never per sample. Its phase moves by the
//! same amount every tick, and ticks sit on fixed absolute frames, so an
//! LFO gives the same values however the host cuts its blocks. A synced
//! LFO takes its phase from the transport's beat position instead, so it
//! stays locked to the bar while the music plays and through a locate.

use mpfree_node::{Curve, ParamInfo, Unit};

/// The shape of an LFO's wave.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Sine,
    Triangle,
    /// Rises from -1 to 1.
    SawUp,
    /// Falls from 1 to -1.
    SawDown,
    Square,
    /// A new random value at the start of each cycle, held (sample and hold).
    Hold,
    /// A new random value each cycle, glided to (smooth random).
    Drift,
}

pub const SHAPES: &[&str] = &["Sine", "Triangle", "Saw up", "Saw down", "Square", "S&H", "Drift"];

impl Shape {
    pub fn from_index(i: usize) -> Shape {
        [Shape::Sine, Shape::Triangle, Shape::SawUp, Shape::SawDown, Shape::Square, Shape::Hold, Shape::Drift][i.min(6)]
    }
    pub fn index(self) -> usize { self as usize }
}

/// Synced lengths in beats, for the Sync choice. "Off" runs free at Rate.
pub const SYNC_BEATS: [f64; 13] = [0.0, 0.125, 0.25, 1.0 / 3.0, 0.5, 2.0 / 3.0, 1.0, 1.5, 2.0, 4.0, 8.0, 16.0, 32.0];
pub const SYNC_NAMES: &[&str] = &["Off", "1/32", "1/16", "1/8 T", "1/8", "1/4 T", "1/4", "1/4 D", "1/2", "1 bar", "2 bars", "4 bars", "8 bars"];

/// LFO parameter ids (the `param` of an `Owner::Mod` address).
pub mod ids {
    pub const SHAPE: u16 = 0;
    pub const RATE: u16 = 1;
    pub const SYNC: u16 = 2;
    pub const PHASE: u16 = 3;
    /// Restart the cycle on each trigger (a note from the host).
    pub const RETRIGGER: u16 = 4;
}

pub static LFO_PARAMS: [ParamInfo; 5] = [
    ParamInfo { id: ids::SHAPE, name: "Shape", group: "LFO", min: 0.0, max: 6.0, default: 0.0, unit: Unit::None, curve: Curve::Choice(SHAPES), readonly: false },
    ParamInfo { id: ids::RATE, name: "Rate", group: "LFO", min: 0.01, max: 40.0, default: 1.0, unit: Unit::Hz, curve: Curve::Log, readonly: false },
    ParamInfo { id: ids::SYNC, name: "Sync", group: "LFO", min: 0.0, max: 12.0, default: 0.0, unit: Unit::None, curve: Curve::Choice(SYNC_NAMES), readonly: false },
    ParamInfo { id: ids::PHASE, name: "Phase", group: "LFO", min: 0.0, max: 100.0, default: 0.0, unit: Unit::Percent, curve: Curve::Linear, readonly: false },
    ParamInfo { id: ids::RETRIGGER, name: "Retrigger", group: "LFO", min: 0.0, max: 1.0, default: 0.0, unit: Unit::None, curve: Curve::Toggle, readonly: false },
];

/// A tiny deterministic random series (one per LFO), so renders repeat.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Rng(u32);

impl Rng {
    pub(crate) fn new(seed: u32) -> Rng { Rng(seed.wrapping_mul(0x9E37_79B9) | 1) }
    /// -1 to 1.
    pub(crate) fn next(&mut self) -> f32 {
        // xorshift32
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        (x >> 8) as f32 / (1u32 << 23) as f32 * 2.0 - 1.0
    }
}

/// One LFO. Output -1 to 1.
#[derive(Clone, Debug)]
pub struct Lfo {
    shape: Shape,
    rate: f32,
    sync: usize,
    phase_offset: f32,
    retrigger: bool,
    /// Free-running phase, 0 to 1.
    phase: f64,
    /// Whole cycles a free LFO has run.
    free_cycle: i64,
    /// The cycle number the random shapes last drew for.
    cycle: i64,
    rng: Rng,
    seed: u32,
    held: [f32; 2],
    out: f32,
}

impl Lfo {
    pub fn new(seed: u32) -> Lfo {
        let mut l = Lfo { shape: Shape::Sine, rate: 1.0, sync: 0, phase_offset: 0.0, retrigger: false, phase: 0.0, free_cycle: 0, cycle: i64::MIN, rng: Rng::new(seed), seed, held: [0.0; 2], out: 0.0 };
        l.reset();
        l
    }

    /// Back to the start of the cycle with the random series restarted.
    pub fn reset(&mut self) {
        self.phase = 0.0;
        self.free_cycle = 0;
        self.cycle = i64::MIN;
        self.rng = Rng::new(self.seed);
        self.held = [self.rng.next(), self.rng.next()];
        self.out = 0.0;
    }

    pub fn set_param(&mut self, id: u16, v: f32) {
        let Some(p) = LFO_PARAMS.iter().find(|p| p.id == id) else { return };
        let v = p.clamp(v);
        match id {
            ids::SHAPE => self.shape = Shape::from_index(v as usize),
            ids::RATE => self.rate = v,
            ids::SYNC => self.sync = v as usize,
            ids::PHASE => self.phase_offset = v / 100.0,
            ids::RETRIGGER => self.retrigger = v >= 0.5,
            _ => {}
        }
    }

    pub fn param(&self, id: u16) -> f32 {
        match id {
            ids::SHAPE => self.shape.index() as f32,
            ids::RATE => self.rate,
            ids::SYNC => self.sync as f32,
            ids::PHASE => self.phase_offset * 100.0,
            ids::RETRIGGER => self.retrigger as u8 as f32,
            _ => f32::NAN,
        }
    }

    pub fn is_synced(&self) -> bool { self.sync > 0 }

    /// A note from the host: restarts the cycle if Retrigger is on.
    pub fn trigger(&mut self) {
        if self.retrigger {
            self.phase = 0.0;
            self.free_cycle += 1;
        }
    }

    /// The last value, -1 to 1.
    pub fn value(&self) -> f32 { self.out }

    /// Moves on by one tick of `secs` seconds and returns the new value.
    /// `beats` is the transport position at the tick when it plays.
    pub(crate) fn tick(&mut self, secs: f64, beats: Option<f64>) -> f32 {
        let off = self.phase_offset as f64;
        let (ph, cycle) = match beats {
            Some(b) if self.sync > 0 => {
                let c = b / SYNC_BEATS[self.sync] + off;
                (c.rem_euclid(1.0), c.floor() as i64)
            }
            _ => {
                // A free LFO reads the phase before moving on, so the first
                // tick after a reset is the start of the wave.
                let c = self.phase + off;
                let at = (c.rem_euclid(1.0), self.free_cycle + c.floor() as i64);
                let next = self.phase + self.rate as f64 * secs;
                self.free_cycle += next.floor() as i64;
                self.phase = next.fract();
                at
            }
        };
        if matches!(self.shape, Shape::Hold | Shape::Drift) && cycle != self.cycle {
            self.held = [self.held[1], self.rng.next()];
            self.cycle = cycle;
        }
        self.out = wave(self.shape, ph as f32, self.held);
        self.out
    }
}

/// The wave at phase `ph` (0 to 1). `held` is the previous and the next
/// random value for the random shapes.
fn wave(shape: Shape, ph: f32, held: [f32; 2]) -> f32 {
    use std::f32::consts::TAU;
    match shape {
        Shape::Sine => (ph * TAU).sin(),
        Shape::Triangle => 1.0 - 4.0 * ((ph + 0.25).fract() - 0.5).abs(),
        Shape::SawUp => 2.0 * ph - 1.0,
        Shape::SawDown => 1.0 - 2.0 * ph,
        Shape::Square => if ph < 0.5 { 1.0 } else { -1.0 },
        Shape::Hold => held[1],
        Shape::Drift => {
            let t = 0.5 - 0.5 * (ph * std::f32::consts::PI).cos();
            held[0] + (held[1] - held[0]) * t
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes_cover_minus_one_to_one() {
        for s in 0..7 {
            let shape = Shape::from_index(s);
            let (mut lo, mut hi) = (f32::MAX, f32::MIN);
            for i in 0..1000 {
                let v = wave(shape, i as f32 / 1000.0, [-0.5, 0.5]);
                assert!((-1.0..=1.0).contains(&v), "{shape:?} {v}");
                lo = lo.min(v);
                hi = hi.max(v);
            }
            if !matches!(shape, Shape::Hold | Shape::Drift) {
                assert!(lo < -0.99 && hi > 0.99, "{shape:?} {lo} {hi}");
            }
        }
        // The triangle starts at 0 going up, like the sine.
        assert!(wave(Shape::Triangle, 0.0, [0.0; 2]).abs() < 1e-6);
        assert!((wave(Shape::Triangle, 0.25, [0.0; 2]) - 1.0).abs() < 1e-6);
        assert!((wave(Shape::Triangle, 0.75, [0.0; 2]) + 1.0).abs() < 1e-6);
    }

    #[test]
    fn free_lfo_runs_at_its_rate() {
        let mut l = Lfo::new(1);
        l.set_param(ids::RATE, 2.5);
        // 2.5 Hz ticked at 100 Hz: back to the start after 40 ticks.
        let first = l.tick(0.01, None);
        assert!(first.abs() < 1e-6);
        let rest: Vec<f32> = (0..39).map(|_| l.tick(0.01, None)).collect();
        assert!((rest[9] - 1.0).abs() < 1e-4, "a quarter cycle in is the top: {}", rest[9]);
        assert!(l.tick(0.01, None).abs() < 1e-3);
    }

    #[test]
    fn synced_lfo_follows_the_beat() {
        let mut l = Lfo::new(1);
        l.set_param(ids::SYNC, 6.0); // 1/4: one cycle per beat
        assert!((l.tick(0.0, Some(0.25)) - 1.0).abs() < 1e-6);
        assert!((l.tick(0.0, Some(7.25)) - 1.0).abs() < 1e-6, "same phase a few beats later");
        // Stopped transport: runs free.
        assert!(l.tick(0.01, None).is_finite());
    }

    #[test]
    fn random_shapes_repeat_with_the_seed_and_change_per_cycle() {
        let run = |seed| {
            let mut l = Lfo::new(seed);
            l.set_param(ids::SHAPE, Shape::Hold.index() as f32);
            l.set_param(ids::SYNC, 6.0);
            (0..16).map(|i| l.tick(0.0, Some(i as f64 * 0.5))).collect::<Vec<f32>>()
        };
        let a = run(7);
        assert_eq!(a, run(7));
        assert_ne!(a, run(8));
        // Held through each beat, new on the next.
        for i in 0..8 {
            assert_eq!(a[2 * i], a[2 * i + 1]);
        }
        assert!((0..7).any(|i| a[2 * i] != a[2 * i + 2]));
    }
}
