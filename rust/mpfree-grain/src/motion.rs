//! Movement: one LFO and one envelope a voice ([`crate::Params::lfo`],
//! [`crate::Params::env`]), each moving one setting of the grains as they
//! are made (sampler roadmap R7, step B3).
//!
//! Both are worked out at the start of each 64-frame part of the voice's
//! own clock (the grains read their settings only when they are born),
//! from the voice's clock alone, so the output doesn't depend on how the
//! audio is cut into periods. Each starts again when the voice starts.

use crate::rng::Rng;

/// What the LFO or the envelope moves, and how far at full depth (a depth
/// of -1 moves it as far the other way).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MotionTarget {
    #[default]
    Off,
    /// Where grains read: by up to the whole span, wrapping.
    Position,
    /// Grain length (synced or not): up to 8 times longer or shorter.
    Size,
    /// Grains a second: up to 16 times more or fewer (a Tone's note: up
    /// to four octaves). A density synced to the tempo keeps its grid.
    Density,
    /// Grain pitch: up to 12 semitones.
    Pitch,
    /// Position spray: up to the whole span more or less.
    Spray,
    /// Each new grain's filter cutoff: up to four octaves (step C4).
    Cutoff,
}

/// The LFO's wave.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LfoShape {
    #[default]
    Sine,
    Triangle,
    /// Rises from -1 to 1.
    SawUp,
    /// Falls from 1 to -1.
    SawDown,
    Square,
    /// A new random value each cycle, held (sample and hold).
    Hold,
    /// A new random value each cycle, glided to (smooth random).
    Drift,
}

/// A voice's LFO.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GrainLfo {
    pub shape: LfoShape,
    /// Cycles a second when it runs free: 0.01 to 40.
    pub rate_hz: f32,
    /// Synced: one cycle every this many beats at the voice's tempo
    /// ([`crate::GrainVoice::set_tempo`], 120 BPM if none is set), in
    /// place of the rate.
    pub beats: Option<f32>,
    pub target: MotionTarget,
    /// -1 to 1.
    pub depth: f32,
}

impl Default for GrainLfo {
    fn default() -> Self {
        GrainLfo {
            shape: LfoShape::Sine,
            rate_hz: 1.0,
            beats: None,
            target: MotionTarget::Off,
            depth: 0.5,
        }
    }
}

/// A voice's movement envelope: from 0 up to full over the attack, then
/// down to the sustain level over the decay, held while the voice makes
/// grains.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GrainEnv {
    /// 0 to 30 s.
    pub attack_ms: f32,
    /// 0 to 30 s.
    pub decay_ms: f32,
    /// 0 to 1.
    pub sustain: f32,
    pub target: MotionTarget,
    /// -1 to 1.
    pub depth: f32,
}

impl Default for GrainEnv {
    fn default() -> Self {
        GrainEnv {
            attack_ms: 1000.0,
            decay_ms: 1000.0,
            sustain: 1.0,
            target: MotionTarget::Off,
            depth: 0.5,
        }
    }
}

/// The LFO's state: where in its cycle it is, and its random values.
#[derive(Clone, Copy, Debug)]
pub(crate) struct LfoState {
    phase: f64,
    held: [f32; 2],
    rng: Rng,
}

impl LfoState {
    pub(crate) fn new(rng: Rng) -> LfoState {
        let mut s = LfoState {
            phase: 0.0,
            held: [0.0; 2],
            rng,
        };
        s.restart();
        s
    }

    /// Back to the start of a cycle, with new random values (the sequence
    /// goes on from note to note).
    pub(crate) fn restart(&mut self) {
        self.phase = 0.0;
        self.held = [self.rng.bipolar(), self.rng.bipolar()];
    }

    /// The value now, -1 to 1.
    pub(crate) fn value(&self, shape: LfoShape) -> f32 {
        use std::f32::consts::{PI, TAU};
        let ph = self.phase as f32;
        match shape {
            LfoShape::Sine => (ph * TAU).sin(),
            LfoShape::Triangle => 1.0 - 4.0 * ((ph + 0.25).fract() - 0.5).abs(),
            LfoShape::SawUp => 2.0 * ph - 1.0,
            LfoShape::SawDown => 1.0 - 2.0 * ph,
            LfoShape::Square => {
                if ph < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            LfoShape::Hold => self.held[1],
            LfoShape::Drift => {
                let t = 0.5 - 0.5 * (ph * PI).cos();
                self.held[0] + (self.held[1] - self.held[0]) * t
            }
        }
    }

    /// Moves on by `cycles` (a fraction of a cycle, or more), drawing a
    /// new random value at each new cycle.
    pub(crate) fn advance(&mut self, cycles: f64) {
        let next = self.phase + cycles.max(0.0);
        // Past two new cycles, only the last two draws matter.
        let turns = next.floor().min(2.0) as u32;
        self.phase = next - next.floor();
        for _ in 0..turns {
            self.held = [self.held[1], self.rng.bipolar()];
        }
    }
}

/// The envelope's level `frames` after the voice started, at `rate`.
/// Linear attack, then an exponential decay that has gone 99.99 % of the
/// way to the sustain level at the decay time.
pub(crate) fn env_level(e: &GrainEnv, frames: u64, rate: u32) -> f32 {
    let ms = frames as f64 * 1000.0 / f64::from(rate.max(1));
    let a = f64::from(e.attack_ms.clamp(0.0, 30_000.0));
    if ms < a {
        return (ms / a) as f32;
    }
    let s = e.sustain.clamp(0.0, 1.0);
    let d = f64::from(e.decay_ms.clamp(0.0, 30_000.0));
    if d <= 0.0 {
        return s;
    }
    // e^(-9.21) is 1e-4.
    let k = (-(ms - a) / d * 9.21).exp() as f32;
    s + (1.0 - s) * k
}

/// `p`'s settings moved along `target` by `d` (-1 to 1 for full depth;
/// sums of several may go further and are clamped where they land).
pub(crate) fn moved(p: &mut crate::Params, target: MotionTarget, d: f32) {
    if d == 0.0 || !d.is_finite() {
        return;
    }
    match target {
        MotionTarget::Off => {}
        MotionTarget::Position => p.position = (p.position + d).rem_euclid(1.0),
        MotionTarget::Size => {
            let k = (3.0 * d).exp2();
            p.size_ms *= k;
            if let Some(b) = p.sync.as_mut().and_then(|s| s.size_beats.as_mut()) {
                *b *= k;
            }
        }
        MotionTarget::Density => p.density *= (4.0 * d).exp2(),
        MotionTarget::Pitch => p.pitch += 12.0 * d,
        MotionTarget::Spray => p.spray = (p.spray + d).clamp(0.0, 1.0),
        MotionTarget::Cutoff => p.filter.cutoff_hz *= (4.0 * d).exp2(),
    }
}
