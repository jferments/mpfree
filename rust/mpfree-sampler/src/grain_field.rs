//! A layer's grain settings as parameters (sampler refactor step 17): the
//! ones with a range, each with a number it keeps for good, so MIDI Learn,
//! and later locks, macros and the mod matrix, can name one
//! ([`GrainField`]). A pad's layer `l`'s setting `g` is the pad's parameter
//! `256 + l × 64 + g` (`mpfree_param::pad::layer`); the pad's own settings
//! ([`crate::engine::PadField`]) keep the numbers below 256.
//!
//! The choices (the window, the spray's shape, the filter's kind, the
//! quality, the LFO's and the envelope's targets and shapes, the scale,
//! tempo sync, the mode) stay settings of the Grain tab only.

use mpfree_grain::Params as GrainParams;
use mpfree_node::{Curve, ParamInfo, Unit};

/// A grain setting a parameter moves. The numbers are saved (in learned
/// controls, and later in locks), so they never change: a new setting
/// takes the next number.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum GrainField {
    /// Where grains read, 0 to 100 % of the region.
    Position = 0,
    /// The position's spray, 0 to 100 %.
    Spray = 1,
    /// Grain length, 2 ms to 2 s.
    Size = 2,
    /// Grains a second, 1 to 200.
    Density = 3,
    /// The grains' pitch, ±36 semitones.
    Pitch = 4,
    /// Random pitch per grain, 0 to 12 semitones.
    PitchSpray = 5,
    /// How fast the reading position moves: -2 to 2 times the sound's own
    /// speed (0 frozen).
    Scan = 6,
    /// The chance a grain plays backwards, 0 to 100 %.
    Reverse = 7,
    /// Random pan per grain, 0 to 100 %.
    Spread = 8,
    /// The cloud's level, -24 to +12 dB.
    Level = 9,
    /// How the gaps between grains vary, 0 (regular) to 100 % (random).
    Timing = 10,
    /// How each grain's length varies, 0 to 100 %.
    SizeSpray = 11,
    /// How each grain's level varies, 0 to 100 %.
    LevelSpray = 12,
    /// The grains' filter's cutoff, 20 Hz to 20 kHz (where it is on).
    Cutoff = 13,
    /// Its resonance, 0 to 100 %.
    Resonance = 14,
    /// How far each grain's cutoff lands from it, 0 to 100 %.
    FilterSpray = 15,
    /// The LFO's free rate, 0.05 to 40 Hz (not while it is synced).
    LfoRate = 16,
    /// The LFO's depth, -100 to 100 %.
    LfoDepth = 17,
    /// The movement envelope's depth, -100 to 100 %.
    EnvDepth = 18,
    /// Its attack and decay, 1 ms to 30 s.
    EnvAttack = 19,
    EnvDecay = 20,
    /// Its sustain, 0 to 100 %.
    EnvSustain = 21,
}

impl GrainField {
    pub const COUNT: usize = 22;
    pub const ALL: [GrainField; GrainField::COUNT] = [
        GrainField::Position,
        GrainField::Spray,
        GrainField::Size,
        GrainField::Density,
        GrainField::Pitch,
        GrainField::PitchSpray,
        GrainField::Scan,
        GrainField::Reverse,
        GrainField::Spread,
        GrainField::Level,
        GrainField::Timing,
        GrainField::SizeSpray,
        GrainField::LevelSpray,
        GrainField::Cutoff,
        GrainField::Resonance,
        GrainField::FilterSpray,
        GrainField::LfoRate,
        GrainField::LfoDepth,
        GrainField::EnvDepth,
        GrainField::EnvAttack,
        GrainField::EnvDecay,
        GrainField::EnvSustain,
    ];

    /// The setting numbered `n`, if there is one.
    pub fn of(n: u8) -> Option<GrainField> {
        GrainField::ALL.get(usize::from(n)).copied()
    }

    /// How the setting is shown and moved as a parameter: its name, its
    /// range in the units it shows, and its curve, as the Grain tab turns
    /// it. `id` is the setting's number (a pad's parameter id adds its
    /// layer, `mpfree_param::pad::layer`).
    pub fn info(self) -> ParamInfo {
        let info = |name, min, max, default, unit, curve| ParamInfo {
            id: self as u16,
            name,
            group: "Grain",
            min,
            max,
            default,
            unit,
            curve,
            readonly: false,
        };
        let pct = |name, default| info(name, 0.0, 100.0, default, Unit::Percent, Curve::Linear);
        let both = |name| info(name, -100.0, 100.0, 50.0, Unit::Percent, Curve::Linear);
        let ms = |name, default| info(name, 1.0, 30_000.0, default, Unit::Ms, Curve::Log);
        match self {
            GrainField::Position => pct("Grain position", 0.0),
            GrainField::Spray => pct("Grain spray", 0.0),
            GrainField::Size => info("Grain size", 2.0, 2000.0, 80.0, Unit::Ms, Curve::Log),
            GrainField::Density => info("Grain density", 1.0, 200.0, 25.0, Unit::Hz, Curve::Log),
            GrainField::Pitch => info(
                "Grain pitch",
                -36.0,
                36.0,
                0.0,
                Unit::Semitones,
                Curve::Linear,
            ),
            GrainField::PitchSpray => info(
                "Pitch spray",
                0.0,
                12.0,
                0.0,
                Unit::Semitones,
                Curve::Linear,
            ),
            GrainField::Scan => info("Scan", -2.0, 2.0, 0.0, Unit::None, Curve::Linear),
            GrainField::Reverse => pct("Reverse", 0.0),
            GrainField::Spread => pct("Spread", 0.0),
            GrainField::Level => info("Grain level", -24.0, 12.0, 0.0, Unit::Db, Curve::Linear),
            GrainField::Timing => pct("Timing", 30.0),
            GrainField::SizeSpray => pct("Size spray", 0.0),
            GrainField::LevelSpray => pct("Level spray", 0.0),
            GrainField::Cutoff => info(
                "Grain cutoff",
                mpfree_grain::CUTOFF_MIN_HZ,
                mpfree_grain::CUTOFF_MAX_HZ,
                2_000.0,
                Unit::Hz,
                Curve::Log,
            ),
            GrainField::Resonance => pct("Grain resonance", 0.0),
            GrainField::FilterSpray => pct("Filter spray", 0.0),
            GrainField::LfoRate => info("LFO rate", 0.05, 40.0, 1.0, Unit::Hz, Curve::Log),
            GrainField::LfoDepth => both("LFO depth"),
            GrainField::EnvDepth => both("Env depth"),
            GrainField::EnvAttack => ms("Env attack", 1000.0),
            GrainField::EnvDecay => ms("Env decay", 1000.0),
            GrainField::EnvSustain => pct("Env sustain", 100.0),
        }
    }

    /// The setting's value in `g`, in the units [`GrainField::info`]
    /// shows.
    pub fn get(self, g: &GrainParams) -> f32 {
        let pct = |v: f32| v * 100.0;
        match self {
            GrainField::Position => pct(g.position),
            GrainField::Spray => pct(g.spray),
            GrainField::Size => g.size_ms,
            GrainField::Density => g.density,
            GrainField::Pitch => g.pitch,
            GrainField::PitchSpray => g.pitch_spray,
            GrainField::Scan => g.scan,
            GrainField::Reverse => pct(g.reverse),
            GrainField::Spread => pct(g.spread),
            GrainField::Level => {
                if g.gain > 0.0 {
                    20.0 * g.gain.log10()
                } else {
                    f32::NEG_INFINITY
                }
            }
            GrainField::Timing => pct(g.timing),
            GrainField::SizeSpray => pct(g.size_spray),
            GrainField::LevelSpray => pct(g.level_spray),
            GrainField::Cutoff => g.filter.cutoff_hz,
            GrainField::Resonance => pct(g.filter.resonance),
            GrainField::FilterSpray => pct(g.filter.spray),
            GrainField::LfoRate => g.lfo.rate_hz,
            GrainField::LfoDepth => pct(g.lfo.depth),
            GrainField::EnvDepth => pct(g.env.depth),
            GrainField::EnvAttack => g.env.attack_ms,
            GrainField::EnvDecay => g.env.decay_ms,
            GrainField::EnvSustain => pct(g.env.sustain),
        }
    }

    /// `g` with the setting at `value` (in the units [`GrainField::info`]
    /// shows), kept within its range; a value that isn't a number leaves
    /// it as it is.
    pub fn set(self, g: &mut GrainParams, value: f32) {
        if !value.is_finite() {
            return;
        }
        let i = self.info();
        let v = value.clamp(i.min, i.max);
        let frac = v / 100.0;
        match self {
            GrainField::Position => g.position = frac,
            GrainField::Spray => g.spray = frac,
            GrainField::Size => g.size_ms = v,
            GrainField::Density => g.density = v,
            GrainField::Pitch => g.pitch = v,
            GrainField::PitchSpray => g.pitch_spray = v,
            GrainField::Scan => g.scan = v,
            GrainField::Reverse => g.reverse = frac,
            GrainField::Spread => g.spread = frac,
            GrainField::Level => g.gain = 10f32.powf(v / 20.0),
            GrainField::Timing => g.timing = frac,
            GrainField::SizeSpray => g.size_spray = frac,
            GrainField::LevelSpray => g.level_spray = frac,
            GrainField::Cutoff => g.filter.cutoff_hz = v,
            GrainField::Resonance => g.filter.resonance = frac,
            GrainField::FilterSpray => g.filter.spray = frac,
            GrainField::LfoRate => g.lfo.rate_hz = v,
            GrainField::LfoDepth => g.lfo.depth = frac,
            GrainField::EnvDepth => g.env.depth = frac,
            GrainField::EnvAttack => g.env.attack_ms = v,
            GrainField::EnvDecay => g.env.decay_ms = v,
            GrainField::EnvSustain => g.env.sustain = frac,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_grain_field_keeps_its_number_and_reads_back_what_it_sets() {
        assert_eq!(GrainField::ALL.len(), GrainField::COUNT);
        for (n, f) in GrainField::ALL.into_iter().enumerate() {
            assert_eq!(f as usize, n, "{f:?} keeps its number");
            assert_eq!(GrainField::of(n as u8), Some(f));
            let i = f.info();
            assert_eq!(i.id, n as u16);
            assert!(i.min < i.max, "{f:?}");
            assert!(i.min <= i.default && i.default <= i.max, "{f:?}'s default");
            let d = f.get(&GrainParams::default());
            assert!(
                (d - i.default).abs() < 1e-3,
                "{f:?}: the grains' default {d}"
            );
            if i.curve == Curve::Log {
                assert!(i.min > 0.0, "{f:?} on a log curve");
            }
            for v in [i.min, i.default, i.max, (i.min + i.max) / 2.0] {
                let mut g = GrainParams::default();
                f.set(&mut g, v);
                let back = f.get(&g);
                assert!(
                    (back - v).abs() <= v.abs() * 1e-4 + 1e-4,
                    "{f:?}: {v} came back {back}"
                );
            }
            let mut g = GrainParams::default();
            f.set(&mut g, i.max * 10.0 + 10.0);
            assert!(
                f.get(&g) <= i.max * 1.0001 + 1e-4,
                "{f:?} stays in its range"
            );
            let before = g;
            f.set(&mut g, f32::NAN);
            assert_eq!(g, before, "{f:?}: NaN changes nothing");
        }
        assert_eq!(GrainField::of(GrainField::COUNT as u8), None);
    }

    #[test]
    fn a_grain_field_moves_only_its_own_setting() {
        for f in GrainField::ALL {
            let i = f.info();
            let mut g = GrainParams::default();
            let v = if (f.get(&g) - i.max).abs() < 1e-3 {
                i.min
            } else {
                i.max
            };
            f.set(&mut g, v);
            for other in GrainField::ALL.into_iter().filter(|o| *o != f) {
                let d = GrainParams::default();
                assert_eq!(
                    other.get(&g).to_bits(),
                    other.get(&d).to_bits(),
                    "{f:?} moved {other:?}"
                );
            }
        }
    }
}
