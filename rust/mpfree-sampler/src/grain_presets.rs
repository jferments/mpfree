//! Grain presets: starting points for a granular pad across styles
//! (sampler roadmap R7, step B6).
//!
//! A preset sets how a region plays as grains: its grain mode, every grain
//! setting, the key tracking, the glide and the movement. It keeps what
//! belongs to the sound and the pad: the region's sound, its points, its
//! position (where in the sound the grains read) and the cloud's level.
//! [`apply`] puts one on a region; [`matching`] says which one a region
//! plays as it is, so a page can show the name until a control moves.
//! Each preset plays at about its sound's level, as every cloud does (the
//! level law); the lab measures it (`mpfree_fxlab::grain`, row G14).

use crate::model::{
    GrainEnv, GrainKeys, GrainLfo, GrainMode, GrainParams, GrainWindow, LfoShape, MotionTarget,
    PlayMode, Region, TempoSync,
};

/// A named starting point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GrainPreset {
    pub name: &'static str,
    /// What it sounds like, in a few words, for a hint line.
    pub about: &'static str,
    pub mode: GrainMode,
    /// The grain settings; the position and the level are the region's.
    pub grain: GrainParams,
    pub keys: GrainKeys,
    pub glide_ms: f32,
}

/// Frames per beat a preset's tempo sync is saved with (120 BPM at
/// 44.1 kHz, as the Grain page saves it); the engine plays it at the
/// project tempo.
const FRAMES_PER_BEAT: f64 = 22_050.0;

/// Pitch quantise to fifths and octaves (bits 0 and 7 of the scale mask).
const FIFTHS: u16 = 0b0000_1000_0001;

/// MIDI note `n`'s frequency: a Tone preset's note.
fn note_hz(n: f32) -> f32 {
    440.0 * ((n - 69.0) / 12.0).exp2()
}

const fn shape(s: f32) -> GrainWindow {
    GrainWindow::Shape { shape: s }
}

const fn lfo(shape: LfoShape, rate_hz: f32, target: MotionTarget, depth: f32) -> GrainLfo {
    GrainLfo {
        shape,
        rate_hz,
        beats: None,
        target,
        depth,
    }
}

const NO_ENV: GrainEnv = GrainEnv {
    attack_ms: 1000.0,
    decay_ms: 1000.0,
    sustain: 1.0,
    target: MotionTarget::Off,
    depth: 0.5,
};

/// Every setting at its default, movement off: what each preset starts
/// from.
fn plain() -> GrainParams {
    GrainParams {
        lfo: GrainLfo::default(),
        env: NO_ENV,
        ..GrainParams::default()
    }
}

/// The presets, in the order a page steps through them.
pub fn presets() -> [GrainPreset; 8] {
    let none = GrainKeys::default();
    [
        GrainPreset {
            name: "Glass pad",
            about: "an octave up, long soft grains that open out over two seconds",
            mode: GrainMode::Cloud,
            grain: GrainParams {
                pitch: 12.0,
                size_ms: 300.0,
                density: 25.0,
                spray: 0.08,
                spread: 0.9,
                timing: 0.5,
                window: shape(0.6),
                lfo: lfo(LfoShape::Sine, 0.1, MotionTarget::Position, 0.03),
                env: GrainEnv {
                    attack_ms: 2000.0,
                    decay_ms: 0.0,
                    sustain: 1.0,
                    target: MotionTarget::Spray,
                    depth: 0.15,
                },
                ..plain()
            },
            keys: none,
            glide_ms: 0.0,
        },
        GrainPreset {
            name: "Frozen choir",
            about: "one spot held still, detuned a little and drifting",
            mode: GrainMode::Cloud,
            grain: GrainParams {
                size_ms: 400.0,
                density: 30.0,
                spray: 0.05,
                pitch_spray: 0.2,
                size_spray: 0.3,
                spread: 0.8,
                timing: 0.6,
                window: shape(0.6),
                lfo: lfo(LfoShape::Drift, 0.25, MotionTarget::Pitch, 0.02),
                ..plain()
            },
            keys: none,
            glide_ms: 0.0,
        },
        GrainPreset {
            name: "Tape stretch",
            about:
                "half speed at its own pitch, with a tape's wow; keys move speed and pitch together",
            mode: GrainMode::Stretch,
            grain: GrainParams {
                scan: 0.5,
                size_ms: 120.0,
                density: 35.0,
                timing: 0.0,
                window: shape(0.6),
                lfo: lfo(LfoShape::Sine, 0.6, MotionTarget::Pitch, 0.015),
                ..plain()
            },
            keys: GrainKeys {
                size: false,
                scan: true,
            },
            glide_ms: 0.0,
        },
        GrainPreset {
            name: "Stutter 1/16",
            about: "short repeats every sixteenth, jumping to a nearby spot each beat",
            mode: GrainMode::Repeat,
            grain: GrainParams {
                timing: 0.0,
                window: shape(0.1),
                sync: Some(TempoSync {
                    frames_per_beat: FRAMES_PER_BEAT,
                    grains_per_beat: Some(4.0),
                    size_beats: Some(0.125),
                }),
                lfo: GrainLfo {
                    beats: Some(1.0),
                    ..lfo(LfoShape::Hold, 1.0, MotionTarget::Position, 0.08)
                },
                ..plain()
            },
            keys: none,
            glide_ms: 0.0,
        },
        GrainPreset {
            name: "Shimmer fifths",
            about: "grains an octave up scattered over fifths and octaves, wide",
            mode: GrainMode::Cloud,
            grain: GrainParams {
                pitch: 12.0,
                pitch_spray: 12.0,
                scale: FIFTHS,
                size_ms: 200.0,
                density: 40.0,
                spray: 0.1,
                spread: 1.0,
                timing: 0.5,
                window: shape(0.6),
                lfo: lfo(LfoShape::Triangle, 0.15, MotionTarget::Spray, 0.1),
                ..plain()
            },
            keys: none,
            glide_ms: 0.0,
        },
        GrainPreset {
            name: "Grit cloud",
            about: "short hard-edged grains, rough in size, level and direction",
            mode: GrainMode::Cloud,
            grain: GrainParams {
                size_ms: 20.0,
                density: 100.0,
                spray: 0.25,
                size_spray: 0.6,
                level_spray: 0.4,
                pitch_spray: 2.0,
                reverse: 0.4,
                spread: 0.6,
                timing: 1.0,
                window: shape(0.0),
                lfo: lfo(LfoShape::Hold, 6.0, MotionTarget::Size, 0.25),
                ..plain()
            },
            keys: none,
            glide_ms: 0.0,
        },
        GrainPreset {
            name: "Swarm",
            about: "a dense buzz of small grains all over, wandering",
            mode: GrainMode::Cloud,
            grain: GrainParams {
                size_ms: 50.0,
                density: 160.0,
                spray: 0.6,
                pitch_spray: 0.6,
                size_spray: 0.4,
                spread: 1.0,
                timing: 1.0,
                window: shape(0.6),
                lfo: lfo(LfoShape::Drift, 0.3, MotionTarget::Position, 0.1),
                ..plain()
            },
            keys: none,
            glide_ms: 0.0,
        },
        GrainPreset {
            name: "Sub drone",
            about: "the sound as a low A (55 Hz) oscillator, its colour slowly moving; keys glide",
            mode: GrainMode::Tone,
            grain: GrainParams {
                density: note_hz(33.0),
                size_ms: 8.0 * 1000.0 / note_hz(33.0),
                timing: 0.0,
                window: shape(0.6),
                lfo: lfo(LfoShape::Sine, 0.07, MotionTarget::Position, 0.02),
                ..plain()
            },
            keys: none,
            glide_ms: 150.0,
        },
    ]
}

/// `r` playing as preset `i` (`r` itself if there is no such preset): a
/// granular region in the preset's mode with its settings, keeping the
/// sound, its points, the position, the level and the quality mode (a
/// Tape pad stays Tape as presets are tried on it).
pub fn apply(r: &Region, i: usize) -> Region {
    let Some(p) = presets().get(i).copied() else {
        return r.clone();
    };
    let mut out = r.clone();
    out.mode = PlayMode::Granular;
    out.grain_mode = p.mode;
    out.grain = GrainParams {
        position: r.grain.position,
        gain: r.grain.gain,
        quality: r.grain.quality,
        ..p.grain
    };
    out.grain_keys = p.keys;
    out.grain_glide_ms = p.glide_ms;
    out
}

/// The preset `r` plays as it is, if any: none once a setting a preset
/// sets has moved.
pub fn matching(r: &Region) -> Option<usize> {
    (0..presets().len()).find(|&i| apply(r, i) == *r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Region, Source};

    fn source() -> Source {
        Source::new("tone", 44_100, 1, vec![0.25; 44_100]).unwrap()
    }

    fn region() -> Region {
        let mut r = Region::whole(&source());
        r.grain.position = 0.3;
        r.grain.gain = 0.5;
        r
    }

    #[test]
    fn a_preset_keeps_the_sound_the_position_and_the_level() {
        let r = region();
        for i in 0..presets().len() {
            let p = apply(&r, i);
            assert_eq!(p.mode, PlayMode::Granular);
            assert_eq!(p.grain_mode, presets()[i].mode);
            assert_eq!((p.grain.position, p.grain.gain), (0.3, 0.5));
            assert_eq!((p.source, p.start, p.end), (r.source, r.start, r.end));
            assert_eq!(matching(&p), Some(i), "{}", presets()[i].name);
            assert!(p.check(&source()).is_ok(), "{}", presets()[i].name);
        }
        assert_eq!(apply(&r, 99), r);
        // The quality mode stays too, and the preset still matches.
        let mut tape = region();
        tape.grain.quality = crate::model::GrainQuality::Tape;
        let p = apply(&tape, 2);
        assert_eq!(p.grain.quality, crate::model::GrainQuality::Tape);
        assert_eq!(matching(&p), Some(2));
    }

    #[test]
    fn a_region_matches_no_preset_once_a_setting_moves() {
        let r = apply(&region(), 0);
        assert_eq!(matching(&r), Some(0));
        let mut moved = r.clone();
        moved.grain.size_ms += 1.0;
        assert_eq!(matching(&moved), None);
        // The position and the level aren't the preset's.
        let mut placed = r.clone();
        placed.grain.position = 0.9;
        placed.grain.gain = 2.0;
        assert_eq!(matching(&placed), Some(0));
        assert_eq!(matching(&region()), None);
    }

    #[test]
    fn the_names_are_distinct_and_every_mode_has_one() {
        let all = presets();
        for (i, a) in all.iter().enumerate() {
            assert!(all[i + 1..].iter().all(|b| b.name != a.name));
            assert!(a.name.len() <= 14, "{}", a.name);
        }
        for mode in [
            GrainMode::Cloud,
            GrainMode::Stretch,
            GrainMode::Repeat,
            GrainMode::Tone,
        ] {
            assert!(all.iter().any(|p| p.mode == mode), "{mode:?}");
        }
    }
}
