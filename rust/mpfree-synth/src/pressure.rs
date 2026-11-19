//! What pad pressure drives in MPFree's synths, and how far a slot's depth
//! reaches. The shaper itself (threshold, curve, hold-off, smoothing,
//! release) is shared with the sampler in [`mpfree_node::pressure`].
//!
//! Each engine has four slots, each a target and a depth of -100 to 100 %.
//! A depth's reach depends on the target ([`full_scale`]): 100 % on Cutoff
//! is 4 octaves, on Level a 24 dB swell. Level is a swell, never a boost:
//! with a positive depth the voice is lowered by the depth at zero pressure
//! and plays at the preset's level at full pressure; with a negative depth
//! pressing ducks it. So pressure on level never pushes a patch past the
//! level it was designed at. Deliberate grit goes through Drive.
//!
//! Poly's play routes ([`crate::params::route`]) reach the same targets
//! with the same depths, from the mod wheel, breath, a foot controller,
//! velocity, the key, the note's tone or a random value.

use mpfree_node::Unit;

/// Poly's targets, by slot or route target index.
pub const POLY_TARGETS: &[&str] = &[
    "Off",
    "Cutoff",
    "Resonance",
    "Level",
    "Vibrato",
    "LFO rate",
    "Pitch",
    "Drive",
    "Width",
    "Sub",
    "Noise",
    "Distortion",
    "LFO 1 amount",
    "LFO 2 amount",
    "Pan",
];
/// FM's targets, by slot target index.
pub const FM_TARGETS: &[&str] = &[
    "Off",
    "Brightness",
    "Level",
    "Vibrato",
    "LFO rate",
    "Tremolo",
    "Pitch",
    "Feedback",
];

/// The Wavetable synth's targets, by slot target index.
pub const WT_TARGETS: &[&str] = &[
    "Off",
    "Position",
    "Warp",
    "Cutoff",
    "Resonance",
    "Level",
    "Vibrato",
    "LFO rate",
    "Pitch",
];

/// The wind engine's targets, by slot target index.
pub const WIND_TARGETS: &[&str] = &[
    "Off", "Breath", "Vibrato", "Tone", "Growl", "Noise", "Pitch",
];

/// What a depth of 100 % means for a target, in that target's unit.
///
/// - Cutoff 4 octaves, Resonance 100 %, Width (pulse width) 45 %, Sub and
///   Noise 100 % of their level.
/// - Level: a 24 dB swell. Drive and Distortion: 24 dB more.
/// - Vibrato 1 semitone deep. Pitch 12 semitones. LFO rate 2 octaves (4 times
///   as fast); the LFO is shared, so it follows the highest pressure.
/// - LFO 1 amount and LFO 2 amount (Poly): how much of what the LFO's
///   own sends are set to gets through, as a swell: with a positive depth
///   none at rest and all of it at full; with a negative depth pressing
///   takes it away ([`lfo_gate`]). Pan: from the middle to one side.
/// - Position (Wavetable): the whole table. Warp (Wavetable): the whole
///   warp amount.
/// - Brightness (FM): 24 output-level steps (18 dB) on every modulator.
///   Tremolo (FM) 100 % amplitude dip. Feedback (FM) 7 steps.
/// - Breath (Wind): the whole breath, added to what the note blows. Tone
///   (Wind): 2 octaves brighter. Growl (Wind): its whole amount.
pub fn full_scale(target: &str) -> Option<(f32, Unit)> {
    Some(match target {
        "Cutoff" => (4.0, Unit::Octaves),
        "Resonance" => (100.0, Unit::Percent),
        "Level" => (24.0, Unit::Db),
        "Vibrato" => (1.0, Unit::Semitones),
        "LFO rate" => (2.0, Unit::Octaves),
        "Pitch" => (12.0, Unit::Semitones),
        "Drive" | "Distortion" => (24.0, Unit::Db),
        "Width" => (45.0, Unit::Percent),
        "Sub" | "Noise" | "Tremolo" | "Position" | "Warp" | "LFO 1 amount" | "LFO 2 amount"
        | "Pan" | "Breath" | "Growl" => (100.0, Unit::Percent),
        "Tone" => (2.0, Unit::Octaves),
        "Brightness" => (24.0, Unit::None),
        "Feedback" => (7.0, Unit::None),
        _ => return None,
    })
}

/// The amount a slot's depth (%) gives at full pressure, in the target's
/// unit: Cutoff at 37.5 % is 1.5 octaves.
pub fn amount(target: &str, depth: f32) -> Option<(f32, Unit)> {
    full_scale(target).map(|(scale, unit)| (scale * depth / 100.0, unit))
}

/// The depth (%) that gives `amount` of a target: the inverse of [`amount`].
pub fn depth_for(target: &str, amount: f32) -> f32 {
    full_scale(target).map_or(0.0, |(scale, _)| amount / scale * 100.0)
}

/// A level slot's gain at shaped pressure `p` (0 to 1) for depth `d` (-1 to
/// 1): 1.0 at most, never a boost.
#[inline]
pub fn swell_gain(d: f32, p: f32) -> f32 {
    let dip = if d >= 0.0 { d * (1.0 - p) } else { -d * p };
    // 24 dB at full depth: 10^(-24/20 * dip).
    (-2.763_102 * dip).exp()
}

/// How much of an LFO's sends get through for depth `d` (-1 to 1) at `p`
/// (0 to 1): a swell, as [`swell_gain`] but straight. With a positive
/// depth `1 - d` at rest and all at full; with a negative one all at rest
/// and `1 + d` at full. 1 with no depth.
#[inline]
pub fn lfo_gate(d: f32, p: f32) -> f32 {
    let d = d.clamp(-1.0, 1.0);
    if d >= 0.0 {
        1.0 - d * (1.0 - p)
    } else {
        1.0 + d * p
    }
}

/// One slot, read from the parameters: its target index and depth, -1 to 1.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Slot {
    pub target: u8,
    pub depth: f32,
}

/// The four slots summed by target index, so an engine adds one number per
/// target (two slots on one target add up).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Depths {
    pub by_target: [f32; 16],
    /// Any slot sends pressure somewhere.
    pub any: bool,
}

impl Depths {
    pub fn from_slots(slots: &[Slot]) -> Depths {
        let mut d = Depths::default();
        for s in slots {
            let t = usize::from(s.target);
            if t > 0 && t < d.by_target.len() && s.depth != 0.0 {
                d.by_target[t] += s.depth;
                d.any = true;
            }
        }
        d
    }

    #[inline]
    pub fn get(&self, target: u8) -> f32 {
        self.by_target[usize::from(target)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_target_has_a_scale() {
        for t in POLY_TARGETS
            .iter()
            .chain(FM_TARGETS)
            .chain(WT_TARGETS)
            .filter(|t| **t != "Off")
        {
            assert!(full_scale(t).is_some(), "{t}");
        }
        assert!(POLY_TARGETS.len() <= 16 && FM_TARGETS.len() <= 16 && WT_TARGETS.len() <= 16);
        assert_eq!(amount("Cutoff", 37.5), Some((1.5, Unit::Octaves)));
        assert!((depth_for("Level", 9.0) - 37.5).abs() < 1e-4);
    }

    #[test]
    fn a_swell_never_boosts() {
        for d in [-1.0, -0.5, 0.0, 0.375, 1.0] {
            for k in 0..=10 {
                let g = swell_gain(d, k as f32 / 10.0);
                assert!(g <= 1.0 && g > 0.0, "{d} {k}: {g}");
            }
        }
        // 9 dB swell: -9 dB at rest, 0 dB pressed.
        let d = 9.0 / 24.0;
        assert!((20.0 * swell_gain(d, 0.0).log10() + 9.0).abs() < 0.01);
        assert_eq!(swell_gain(d, 1.0), 1.0);
    }

    #[test]
    fn an_lfo_gate_opens_with_a_positive_depth_and_closes_with_a_negative_one() {
        assert_eq!(lfo_gate(0.0, 0.3), 1.0);
        assert_eq!(lfo_gate(1.0, 0.0), 0.0);
        assert_eq!(lfo_gate(1.0, 1.0), 1.0);
        assert_eq!(lfo_gate(0.5, 0.0), 0.5);
        assert_eq!(lfo_gate(-1.0, 0.0), 1.0);
        assert_eq!(lfo_gate(-1.0, 1.0), 0.0);
        assert_eq!(lfo_gate(-0.25, 1.0), 0.75);
    }
}
