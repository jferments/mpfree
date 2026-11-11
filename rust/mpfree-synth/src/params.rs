//! The Poly synth's parameters and factory presets.

use mpfree_node::{Curve, ParamInfo, Unit};

macro_rules! ids {
    ($($name:ident = $v:expr),* $(,)?) => { $(pub const $name: u16 = $v;)* };
}

ids! {
    OSC1_WAVE = 0, OSC1_OCT = 1, OSC1_PW = 2, OSC1_LEVEL = 3,
    OSC2_WAVE = 10, OSC2_OCT = 11, OSC2_SEMI = 12, OSC2_FINE = 13, OSC2_PW = 14, OSC2_LEVEL = 15,
    SUB_LEVEL = 16, NOISE_LEVEL = 17,
    UNISON = 20, UNISON_DETUNE = 21, UNISON_SPREAD = 22,
    FILTER_MODE = 30, CUTOFF = 31, RESONANCE = 32, DRIVE = 33, FENV_AMOUNT = 34, KEY_TRACK = 35, VEL_CUTOFF = 36,
    FENV_A = 40, FENV_D = 41, FENV_S = 42, FENV_R = 43,
    AMP_A = 50, AMP_D = 51, AMP_S = 52, AMP_R = 53, VEL_AMP = 54,
    LFO_SHAPE = 60, LFO_RATE = 61, LFO_SYNC = 62, LFO_PITCH = 63, LFO_CUTOFF = 64, LFO_PW = 65, LFO_AMP = 66, LFO_RETRIG = 67,
    VOICE_MODE = 70, POLYPHONY = 71, GLIDE = 72, BEND_RANGE = 73, PRESSURE_CUTOFF = 74,
    LEVEL = 80, PAN = 81,
    METER_VOICES = 90,
}

/// The highest id plus one: values live in an array indexed by id.
pub const ID_SPACE: usize = 96;

pub const WAVES: &[&str] = &["Saw", "Pulse", "Triangle", "Sine"];
pub const FILTER_MODES: &[&str] = &["LP 24", "LP 12", "BP 12", "HP 12"];
pub const LFO_SHAPES: &[&str] = &["Sine", "Triangle", "Saw", "Square", "S&H"];
/// Tempo sync: off, or a cycle length in beats.
pub const LFO_SYNCS: &[&str] = &["Off", "4 bars", "2 bars", "1 bar", "1/2", "1/4", "1/8", "1/16", "1/4 T", "1/8 T", "1/16 T"];
pub const LFO_SYNC_BEATS: [f64; 11] = [0.0, 16.0, 8.0, 4.0, 2.0, 1.0, 0.5, 0.25, 2.0 / 3.0, 1.0 / 3.0, 1.0 / 6.0];
pub const VOICE_MODES: &[&str] = &["Poly", "Mono", "Legato"];

#[allow(clippy::too_many_arguments)]
const fn p(id: u16, name: &'static str, group: &'static str, min: f32, max: f32, default: f32, unit: Unit, curve: Curve) -> ParamInfo {
    ParamInfo { id, name, group, min, max, default, unit, curve, readonly: false }
}

const fn choice(id: u16, name: &'static str, group: &'static str, names: &'static [&'static str], default: f32) -> ParamInfo {
    p(id, name, group, 0.0, (names.len() - 1) as f32, default, Unit::None, Curve::Choice(names))
}

use Curve::{Linear, Log, Stepped, Toggle};

pub static PARAMS: &[ParamInfo] = &[
    choice(OSC1_WAVE, "Wave", "Osc 1", WAVES, 0.0),
    p(OSC1_OCT, "Octave", "Osc 1", -3.0, 3.0, 0.0, Unit::None, Stepped),
    p(OSC1_PW, "Width", "Osc 1", 5.0, 95.0, 50.0, Unit::Percent, Linear),
    p(OSC1_LEVEL, "Level", "Osc 1", 0.0, 100.0, 100.0, Unit::Percent, Linear),
    choice(OSC2_WAVE, "Wave", "Osc 2", WAVES, 0.0),
    p(OSC2_OCT, "Octave", "Osc 2", -3.0, 3.0, 0.0, Unit::None, Stepped),
    p(OSC2_SEMI, "Semi", "Osc 2", -12.0, 12.0, 0.0, Unit::Semitones, Stepped),
    p(OSC2_FINE, "Fine", "Osc 2", -100.0, 100.0, 7.0, Unit::Cents, Linear),
    p(OSC2_PW, "Width", "Osc 2", 5.0, 95.0, 50.0, Unit::Percent, Linear),
    p(OSC2_LEVEL, "Level", "Osc 2", 0.0, 100.0, 60.0, Unit::Percent, Linear),
    p(SUB_LEVEL, "Sub", "Mix", 0.0, 100.0, 0.0, Unit::Percent, Linear),
    p(NOISE_LEVEL, "Noise", "Mix", 0.0, 100.0, 0.0, Unit::Percent, Linear),
    p(UNISON, "Unison", "Unison", 1.0, 4.0, 1.0, Unit::None, Stepped),
    p(UNISON_DETUNE, "Detune", "Unison", 0.0, 100.0, 15.0, Unit::Cents, Linear),
    p(UNISON_SPREAD, "Spread", "Unison", 0.0, 100.0, 60.0, Unit::Percent, Linear),
    choice(FILTER_MODE, "Type", "Filter", FILTER_MODES, 0.0),
    p(CUTOFF, "Cutoff", "Filter", 20.0, 20_000.0, 2_500.0, Unit::Hz, Log),
    p(RESONANCE, "Resonance", "Filter", 0.0, 100.0, 15.0, Unit::Percent, Linear),
    p(DRIVE, "Drive", "Filter", 0.0, 24.0, 0.0, Unit::Db, Linear),
    p(FENV_AMOUNT, "Env amount", "Filter", -5.0, 5.0, 1.5, Unit::Octaves, Linear),
    p(KEY_TRACK, "Key track", "Filter", 0.0, 100.0, 50.0, Unit::Percent, Linear),
    p(VEL_CUTOFF, "Velocity", "Filter", 0.0, 4.0, 1.0, Unit::Octaves, Linear),
    p(FENV_A, "Attack", "Filter env", 0.5, 10_000.0, 2.0, Unit::Ms, Log),
    p(FENV_D, "Decay", "Filter env", 1.0, 10_000.0, 400.0, Unit::Ms, Log),
    p(FENV_S, "Sustain", "Filter env", 0.0, 100.0, 30.0, Unit::Percent, Linear),
    p(FENV_R, "Release", "Filter env", 1.0, 10_000.0, 300.0, Unit::Ms, Log),
    p(AMP_A, "Attack", "Amp env", 0.5, 10_000.0, 2.0, Unit::Ms, Log),
    p(AMP_D, "Decay", "Amp env", 1.0, 10_000.0, 300.0, Unit::Ms, Log),
    p(AMP_S, "Sustain", "Amp env", 0.0, 100.0, 80.0, Unit::Percent, Linear),
    p(AMP_R, "Release", "Amp env", 1.0, 10_000.0, 250.0, Unit::Ms, Log),
    p(VEL_AMP, "Velocity", "Amp env", 0.0, 100.0, 60.0, Unit::Percent, Linear),
    choice(LFO_SHAPE, "Shape", "LFO", LFO_SHAPES, 0.0),
    p(LFO_RATE, "Rate", "LFO", 0.05, 30.0, 5.0, Unit::Hz, Log),
    choice(LFO_SYNC, "Sync", "LFO", LFO_SYNCS, 0.0),
    p(LFO_PITCH, "To pitch", "LFO", 0.0, 12.0, 0.0, Unit::Semitones, Linear),
    p(LFO_CUTOFF, "To cutoff", "LFO", 0.0, 5.0, 0.0, Unit::Octaves, Linear),
    p(LFO_PW, "To width", "LFO", 0.0, 45.0, 0.0, Unit::Percent, Linear),
    p(LFO_AMP, "To level", "LFO", 0.0, 100.0, 0.0, Unit::Percent, Linear),
    p(LFO_RETRIG, "Retrigger", "LFO", 0.0, 1.0, 0.0, Unit::None, Toggle),
    choice(VOICE_MODE, "Mode", "Voice", VOICE_MODES, 0.0),
    p(POLYPHONY, "Voices", "Voice", 1.0, 16.0, 8.0, Unit::None, Stepped),
    p(GLIDE, "Glide", "Voice", 0.0, 2_000.0, 0.0, Unit::Ms, Linear),
    p(BEND_RANGE, "Bend range", "Voice", 0.0, 24.0, 2.0, Unit::Semitones, Stepped),
    p(PRESSURE_CUTOFF, "Pressure", "Voice", 0.0, 4.0, 1.0, Unit::Octaves, Linear),
    p(LEVEL, "Level", "Output", -60.0, 6.0, 0.0, Unit::Db, Linear),
    p(PAN, "Pan", "Output", -100.0, 100.0, 0.0, Unit::Percent, Linear),
    ParamInfo { id: METER_VOICES, name: "Voices on", group: "Output", min: 0.0, max: 32.0, default: 0.0, unit: Unit::None, curve: Stepped, readonly: true },
];

/// A factory preset: a name and the values that differ from the defaults.
#[derive(Clone, Copy, Debug)]
pub struct Preset {
    pub name: &'static str,
    pub values: &'static [(u16, f32)],
}

pub static PRESETS: &[Preset] = &[
    Preset { name: "Init", values: &[] },
    Preset {
        name: "Grit Bass",
        values: &[
            (OSC1_OCT, -1.0), (OSC2_WAVE, 1.0), (OSC2_OCT, -1.0), (OSC2_FINE, -9.0), (OSC2_LEVEL, 70.0), (SUB_LEVEL, 70.0),
            (CUTOFF, 260.0), (RESONANCE, 35.0), (DRIVE, 18.0), (FENV_AMOUNT, 3.2), (FENV_D, 220.0), (FENV_S, 10.0),
            (AMP_D, 400.0), (AMP_S, 70.0), (AMP_R, 90.0), (VOICE_MODE, 2.0), (GLIDE, 40.0), (KEY_TRACK, 30.0), (LEVEL, 6.0),
        ],
    },
    Preset {
        name: "Acid",
        values: &[
            (OSC2_LEVEL, 0.0), (OSC1_OCT, -1.0), (CUTOFF, 380.0), (RESONANCE, 82.0), (DRIVE, 12.0), (FENV_AMOUNT, 3.8),
            (FENV_D, 180.0), (FENV_S, 0.0), (AMP_S, 90.0), (AMP_R, 40.0), (VOICE_MODE, 2.0), (GLIDE, 70.0), (VEL_CUTOFF, 2.0), (LEVEL, 5.0),
        ],
    },
    Preset {
        name: "Supersaw",
        values: &[
            (UNISON, 4.0), (UNISON_DETUNE, 28.0), (UNISON_SPREAD, 90.0), (OSC2_OCT, 1.0), (OSC2_FINE, 4.0), (OSC2_LEVEL, 45.0),
            (CUTOFF, 6_000.0), (RESONANCE, 10.0), (FENV_AMOUNT, 0.8), (AMP_A, 5.0), (AMP_R, 500.0), (FILTER_MODE, 1.0), (LEVEL, -3.0),
        ],
    },
    Preset {
        name: "Dust Pad",
        values: &[
            (OSC1_WAVE, 1.0), (OSC1_PW, 35.0), (OSC2_FINE, 11.0), (OSC2_LEVEL, 80.0), (NOISE_LEVEL, 8.0), (UNISON, 2.0), (UNISON_DETUNE, 12.0),
            (CUTOFF, 900.0), (RESONANCE, 25.0), (FENV_AMOUNT, 1.2), (FENV_A, 900.0), (FENV_D, 2_000.0), (FENV_S, 50.0), (FENV_R, 1_500.0),
            (AMP_A, 700.0), (AMP_D, 1_500.0), (AMP_S, 85.0), (AMP_R, 1_800.0), (LFO_SHAPE, 1.0), (LFO_RATE, 0.3), (LFO_PW, 20.0), (LFO_CUTOFF, 0.6), (LEVEL, -6.0),
        ],
    },
    Preset {
        name: "Pluck",
        values: &[
            (OSC2_WAVE, 1.0), (OSC2_OCT, 1.0), (OSC2_LEVEL, 35.0), (CUTOFF, 400.0), (RESONANCE, 30.0), (FENV_AMOUNT, 4.0),
            (FENV_D, 160.0), (FENV_S, 0.0), (FENV_R, 160.0), (AMP_D, 450.0), (AMP_S, 0.0), (AMP_R, 300.0), (VEL_CUTOFF, 1.5),
        ],
    },
    Preset {
        name: "Brass",
        values: &[
            (OSC2_FINE, 5.0), (OSC2_LEVEL, 90.0), (CUTOFF, 700.0), (RESONANCE, 10.0), (FENV_AMOUNT, 2.5), (FENV_A, 60.0),
            (FENV_D, 500.0), (FENV_S, 50.0), (AMP_A, 30.0), (AMP_S, 90.0), (LFO_PITCH, 0.15), (LFO_RATE, 5.5),
        ],
    },
    Preset {
        name: "Sub Bass",
        values: &[
            (OSC1_WAVE, 3.0), (OSC1_OCT, -1.0), (OSC2_WAVE, 2.0), (OSC2_OCT, -1.0), (OSC2_FINE, 0.0), (OSC2_LEVEL, 40.0),
            (CUTOFF, 1_200.0), (RESONANCE, 0.0), (FENV_AMOUNT, 0.0), (AMP_S, 100.0), (AMP_R, 120.0), (VOICE_MODE, 1.0), (GLIDE, 25.0),
        ],
    },
    Preset {
        name: "Lead",
        values: &[
            (OSC1_WAVE, 1.0), (OSC1_PW, 40.0), (OSC2_FINE, 12.0), (OSC2_LEVEL, 80.0), (CUTOFF, 1_800.0), (RESONANCE, 40.0), (DRIVE, 9.0),
            (FENV_AMOUNT, 1.8), (FENV_D, 300.0), (FENV_S, 40.0), (VOICE_MODE, 2.0), (GLIDE, 90.0), (LFO_PITCH, 0.3), (LFO_RATE, 5.8), (BEND_RANGE, 12.0), (LEVEL, 3.0),
        ],
    },
];

/// Looks up a parameter by id.
pub fn info(id: u16) -> Option<&'static ParamInfo> { PARAMS.iter().find(|p| p.id == id) }
