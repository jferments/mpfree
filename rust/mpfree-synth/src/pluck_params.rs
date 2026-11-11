//! The pluck synth's parameters and factory presets.

use mpfree_node::{Curve, ParamInfo, Unit};

use crate::params::Preset;

macro_rules! ids {
    ($($name:ident = $v:expr),* $(,)?) => { $(pub const $name: u16 = $v;)* };
}

ids! {
    MODEL = 0,
    DECAY = 1, BRIGHTNESS = 2, PICK = 3, POSITION = 4,
    MATERIAL = 10, HARDNESS = 12, STRIKE = 13, STRETCH = 14, HIGH_DAMP = 15,
    VEL_TONE = 20, TRANSPOSE = 21, FINE = 22, POLYPHONY = 23, BEND_RANGE = 24, RELEASE = 25,
    LEVEL = 30, PAN = 31, SPREAD = 32,
    METER_VOICES = 40,
}

/// The highest id plus one: values live in an array indexed by id.
pub const ID_SPACE: usize = 41;

pub const MODELS: &[&str] = &["String", "Mallet"];
pub const MATERIALS: &[&str] = &["Marimba", "Vibes", "Bell", "Glass", "Kalimba", "Steel drum"];

#[allow(clippy::too_many_arguments)]
const fn p(id: u16, name: &'static str, group: &'static str, min: f32, max: f32, default: f32, unit: Unit, curve: Curve) -> ParamInfo {
    ParamInfo { id, name, group, min, max, default, unit, curve, readonly: false }
}

const fn choice(id: u16, name: &'static str, group: &'static str, names: &'static [&'static str], default: f32) -> ParamInfo {
    p(id, name, group, 0.0, (names.len() - 1) as f32, default, Unit::None, Curve::Choice(names))
}

use Curve::{Linear, Log, Stepped};

pub static PARAMS: &[ParamInfo] = &[
    choice(MODEL, "Model", "Model", MODELS, 0.0),
    // Decay is the time to fall 60 dB at middle C; higher notes ring
    // shorter, lower ones longer.
    p(DECAY, "Decay", "Model", 50.0, 30000.0, 4000.0, Unit::Ms, Log),
    p(RELEASE, "Release", "Model", 10.0, 5000.0, 1500.0, Unit::Ms, Log),
    p(BRIGHTNESS, "Brightness", "String", 0.0, 100.0, 60.0, Unit::Percent, Linear),
    p(PICK, "Pick", "String", 0.0, 100.0, 70.0, Unit::Percent, Linear),
    p(POSITION, "Position", "String", 2.0, 50.0, 15.0, Unit::Percent, Linear),
    choice(MATERIAL, "Material", "Mallet", MATERIALS, 0.0),
    p(HARDNESS, "Hardness", "Mallet", 0.0, 100.0, 50.0, Unit::Percent, Linear),
    p(STRIKE, "Strike", "Mallet", 0.0, 50.0, 22.0, Unit::Percent, Linear),
    p(STRETCH, "Stretch", "Mallet", -50.0, 50.0, 0.0, Unit::Percent, Linear),
    p(HIGH_DAMP, "High damp", "Mallet", 0.0, 100.0, 50.0, Unit::Percent, Linear),
    p(VEL_TONE, "Vel > tone", "Voice", 0.0, 100.0, 50.0, Unit::Percent, Linear),
    p(TRANSPOSE, "Transpose", "Voice", -24.0, 24.0, 0.0, Unit::Semitones, Stepped),
    p(FINE, "Fine", "Voice", -100.0, 100.0, 0.0, Unit::Cents, Linear),
    p(POLYPHONY, "Voices", "Voice", 1.0, 16.0, 8.0, Unit::None, Stepped),
    p(BEND_RANGE, "Bend range", "Voice", 0.0, 12.0, 2.0, Unit::Semitones, Stepped),
    p(LEVEL, "Level", "Output", -60.0, 6.0, 0.0, Unit::Db, Linear),
    p(PAN, "Pan", "Output", -100.0, 100.0, 0.0, Unit::Percent, Linear),
    p(SPREAD, "Spread", "Output", 0.0, 100.0, 30.0, Unit::Percent, Linear),
    ParamInfo { id: METER_VOICES, name: "Voices on", group: "Output", min: 0.0, max: 16.0, default: 0.0, unit: Unit::None, curve: Stepped, readonly: true },
];

const MALLET: f32 = 1.0;

pub static PRESETS: &[Preset] = &[
    Preset { name: "Init", values: &[] },
    Preset { name: "Nylon", values: &[(DECAY, 3000.0), (BRIGHTNESS, 35.0), (PICK, 45.0), (POSITION, 20.0), (RELEASE, 250.0)] },
    Preset { name: "Steel String", values: &[(DECAY, 6000.0), (BRIGHTNESS, 80.0), (PICK, 85.0), (POSITION, 12.0), (RELEASE, 400.0)] },
    Preset { name: "Harp", values: &[(DECAY, 5000.0), (BRIGHTNESS, 55.0), (PICK, 50.0), (POSITION, 30.0), (RELEASE, 5000.0), (SPREAD, 60.0)] },
    Preset { name: "Koto", values: &[(DECAY, 2500.0), (BRIGHTNESS, 75.0), (PICK, 95.0), (POSITION, 5.0), (RELEASE, 600.0)] },
    Preset { name: "Muted Bass", values: &[(TRANSPOSE, -12.0), (DECAY, 1200.0), (BRIGHTNESS, 20.0), (PICK, 40.0), (POSITION, 25.0), (RELEASE, 60.0), (SPREAD, 0.0), (LEVEL, 2.0)] },
    Preset { name: "Marimba", values: &[(MODEL, MALLET), (MATERIAL, 0.0), (DECAY, 1200.0), (HARDNESS, 45.0), (RELEASE, 5000.0)] },
    Preset { name: "Vibes", values: &[(MODEL, MALLET), (MATERIAL, 1.0), (DECAY, 6000.0), (HARDNESS, 55.0), (HIGH_DAMP, 30.0), (RELEASE, 400.0)] },
    Preset { name: "Bell", values: &[(MODEL, MALLET), (MATERIAL, 2.0), (DECAY, 9000.0), (HARDNESS, 80.0), (HIGH_DAMP, 40.0), (RELEASE, 5000.0), (LEVEL, -2.0)] },
    Preset { name: "Glass", values: &[(MODEL, MALLET), (MATERIAL, 3.0), (DECAY, 4000.0), (HARDNESS, 70.0), (STRIKE, 35.0), (RELEASE, 3000.0)] },
    Preset { name: "Kalimba", values: &[(MODEL, MALLET), (MATERIAL, 4.0), (DECAY, 1800.0), (HARDNESS, 60.0), (RELEASE, 2000.0)] },
    Preset { name: "Steel Drum", values: &[(MODEL, MALLET), (MATERIAL, 5.0), (DECAY, 2000.0), (HARDNESS, 65.0), (STRETCH, 2.0), (RELEASE, 1500.0)] },
];
