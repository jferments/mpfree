//! The pluck synth's parameters; its factory presets are in
//! `presets/pluck.presets` ([`crate::factory::read`]).

use mpfree_node::{Curve, ParamInfo, Unit};

macro_rules! ids {
    ($($name:ident = $v:expr),* $(,)?) => { $(pub const $name: u16 = $v;)* };
}

ids! {
    MODEL = 0,
    DECAY = 1, BRIGHTNESS = 2, PICK = 3, POSITION = 4,
    MATERIAL = 10, HARDNESS = 12, STRIKE = 13, STRETCH = 14, HIGH_DAMP = 15,
    VEL_TONE = 20, TRANSPOSE = 21, FINE = 22, POLYPHONY = 23, BEND_RANGE = 24, RELEASE = 25,
    VELOCITY = 26,
    LEVEL = 30, PAN = 31, SPREAD = 32,
    METER_VOICES = 40,
}

/// The highest id plus one: values live in an array indexed by id.
pub const ID_SPACE: usize = 41;

pub const MODELS: &[&str] = &["String", "Mallet"];
pub const MATERIALS: &[&str] = &[
    "Marimba",
    "Vibes",
    "Bell",
    "Glass",
    "Kalimba",
    "Steel drum",
    "Glockenspiel",
    "Xylophone",
    "Chimes",
];

#[allow(clippy::too_many_arguments)]
const fn p(
    id: u16,
    name: &'static str,
    group: &'static str,
    min: f32,
    max: f32,
    default: f32,
    unit: Unit,
    curve: Curve,
) -> ParamInfo {
    ParamInfo {
        id,
        name,
        group,
        min,
        max,
        default,
        unit,
        curve,
        readonly: false,
    }
}

const fn choice(
    id: u16,
    name: &'static str,
    group: &'static str,
    names: &'static [&'static str],
    default: f32,
) -> ParamInfo {
    p(
        id,
        name,
        group,
        0.0,
        (names.len() - 1) as f32,
        default,
        Unit::None,
        Curve::Choice(names),
    )
}

use Curve::{Linear, Log, Stepped};

pub static PARAMS: &[ParamInfo] = &[
    choice(MODEL, "Model", "Model", MODELS, 0.0),
    // Decay is the time to fall 60 dB at middle C; higher notes ring
    // shorter, lower ones longer.
    p(
        DECAY,
        "Decay",
        "Model",
        50.0,
        30000.0,
        4000.0,
        Unit::Ms,
        Log,
    ),
    p(
        RELEASE,
        "Release",
        "Model",
        10.0,
        5000.0,
        1500.0,
        Unit::Ms,
        Log,
    ),
    p(
        BRIGHTNESS,
        "Brightness",
        "String",
        0.0,
        100.0,
        60.0,
        Unit::Percent,
        Linear,
    ),
    p(
        PICK,
        "Pick",
        "String",
        0.0,
        100.0,
        70.0,
        Unit::Percent,
        Linear,
    ),
    p(
        POSITION,
        "Position",
        "String",
        2.0,
        50.0,
        15.0,
        Unit::Percent,
        Linear,
    ),
    choice(MATERIAL, "Material", "Mallet", MATERIALS, 0.0),
    p(
        HARDNESS,
        "Hardness",
        "Mallet",
        0.0,
        100.0,
        50.0,
        Unit::Percent,
        Linear,
    ),
    p(
        STRIKE,
        "Strike",
        "Mallet",
        0.0,
        50.0,
        22.0,
        Unit::Percent,
        Linear,
    ),
    p(
        STRETCH,
        "Stretch",
        "Mallet",
        -50.0,
        50.0,
        0.0,
        Unit::Percent,
        Linear,
    ),
    p(
        HIGH_DAMP,
        "High damp",
        "Mallet",
        0.0,
        100.0,
        50.0,
        Unit::Percent,
        Linear,
    ),
    p(
        VEL_TONE,
        "Vel > tone",
        "Voice",
        0.0,
        100.0,
        50.0,
        Unit::Percent,
        Linear,
    ),
    // How much quieter the softest note is than the hardest (a square
    // law, as the voice core's).
    p(
        VELOCITY,
        "Velocity",
        "Voice",
        0.0,
        48.0,
        30.0,
        Unit::Db,
        Linear,
    ),
    p(
        TRANSPOSE,
        "Transpose",
        "Voice",
        -24.0,
        24.0,
        0.0,
        Unit::Semitones,
        Stepped,
    ),
    p(
        FINE,
        "Fine",
        "Voice",
        -100.0,
        100.0,
        0.0,
        Unit::Cents,
        Linear,
    ),
    p(
        POLYPHONY,
        "Voices",
        "Voice",
        1.0,
        16.0,
        8.0,
        Unit::None,
        Stepped,
    ),
    p(
        BEND_RANGE,
        "Bend range",
        "Voice",
        0.0,
        12.0,
        2.0,
        Unit::Semitones,
        Stepped,
    ),
    // -2 dB: 8-note chords of Init at full velocity peak under 0 dBFS,
    // whichever noise each pluck draws (synth-quality L2; DSP quality
    // step 1.5).
    p(LEVEL, "Level", "Output", -60.0, 6.0, -2.0, Unit::Db, Linear),
    p(
        PAN,
        "Pan",
        "Output",
        -100.0,
        100.0,
        0.0,
        Unit::Percent,
        Linear,
    ),
    p(
        SPREAD,
        "Spread",
        "Output",
        0.0,
        100.0,
        30.0,
        Unit::Percent,
        Linear,
    ),
    ParamInfo {
        id: METER_VOICES,
        name: "Voices on",
        group: "Output",
        min: 0.0,
        max: 16.0,
        default: 0.0,
        unit: Unit::None,
        curve: Stepped,
        readonly: true,
    },
];
