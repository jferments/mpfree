//! The electric piano's parameters and factory presets.

use mpfree_node::{Curve, ParamInfo, Unit};

use crate::params::Preset;

macro_rules! ids {
    ($($name:ident = $v:expr),* $(,)?) => { $(pub const $name: u16 = $v;)* };
}

ids! {
    MODEL = 0, DECAY = 1, RELEASE = 2,
    HARDNESS = 10, BELL = 11, VOICING = 12, BARK = 13, TONE = 14, VEL_TONE = 15,
    TREMOLO = 20, TREM_RATE = 21, TREM_MODE = 22, DRIVE = 23, KEY_NOISE = 24,
    TRANSPOSE = 30, FINE = 31, POLYPHONY = 32, BEND_RANGE = 33,
    LEVEL = 40, PAN = 41,
    METER_VOICES = 50,
}

/// The highest id plus one: values live in an array indexed by id.
pub const ID_SPACE: usize = 51;

/// Tine: a struck steel tine beside a magnetic pickup, round and bell-like,
/// barking when hit hard. Reed: a struck steel reed in an electrostatic
/// pickup, reedy and quick to buzz.
pub const MODELS: &[&str] = &["Tine", "Reed"];
/// Pan: the tremolo moves the sound between the speakers. Amp: it moves its
/// level.
pub const TREM_MODES: &[&str] = &["Pan", "Amp"];

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

const fn pct(id: u16, name: &'static str, group: &'static str, default: f32) -> ParamInfo {
    p(
        id,
        name,
        group,
        0.0,
        100.0,
        default,
        Unit::Percent,
        Curve::Linear,
    )
}

use Curve::{Linear, Log, Stepped};

pub static PARAMS: &[ParamInfo] = &[
    choice(MODEL, "Model", "Model", MODELS, 0.0),
    // The time a held middle C takes to fall 60 dB; higher notes ring
    // shorter, lower ones longer.
    p(
        DECAY,
        "Decay",
        "Model",
        500.0,
        20000.0,
        8000.0,
        Unit::Ms,
        Log,
    ),
    // How fast the damper stops a note once its key is let go.
    p(
        RELEASE,
        "Release",
        "Model",
        20.0,
        3000.0,
        160.0,
        Unit::Ms,
        Log,
    ),
    pct(HARDNESS, "Hardness", "Tone", 50.0),
    pct(BELL, "Bell", "Tone", 40.0),
    // Where the tine or reed sits against its pickup, as a technician
    // voices it: 0 is centred (hollow, the octave above the note
    // strongest), 100 far off centre (pure and round). The bark lives in
    // between.
    pct(VOICING, "Voicing", "Tone", 55.0),
    // How hard a loud note drives the pickup: the growl of a hard hit.
    pct(BARK, "Bark", "Tone", 45.0),
    p(
        TONE,
        "Tone",
        "Tone",
        -100.0,
        100.0,
        0.0,
        Unit::Percent,
        Linear,
    ),
    pct(VEL_TONE, "Vel > tone", "Tone", 60.0),
    pct(TREMOLO, "Tremolo", "Amp", 0.0),
    p(TREM_RATE, "Trem rate", "Amp", 0.5, 12.0, 4.5, Unit::Hz, Log),
    choice(TREM_MODE, "Trem mode", "Amp", TREM_MODES, 0.0),
    pct(DRIVE, "Drive", "Amp", 0.0),
    pct(KEY_NOISE, "Key noise", "Amp", 20.0),
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
        12.0,
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
    p(LEVEL, "Level", "Output", -60.0, 6.0, 0.0, Unit::Db, Linear),
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

const REED: f32 = 1.0;
const AMP: f32 = 1.0;

pub static PRESETS: &[Preset] = &[
    Preset {
        name: "Init",
        values: &[],
    },
    Preset {
        name: "Soft Tines",
        values: &[
            (HARDNESS, 30.0),
            (BELL, 25.0),
            (VOICING, 65.0),
            (BARK, 25.0),
            (TONE, -25.0),
        ],
    },
    Preset {
        name: "Bright Tines",
        values: &[
            (HARDNESS, 70.0),
            (BELL, 60.0),
            (VOICING, 50.0),
            (TONE, 35.0),
            (LEVEL, -1.5),
        ],
    },
    Preset {
        name: "Barking Tines",
        values: &[
            (HARDNESS, 75.0),
            (BELL, 45.0),
            (VOICING, 60.0),
            (BARK, 90.0),
            (DRIVE, 25.0),
            (VEL_TONE, 80.0),
        ],
    },
    Preset {
        name: "Bell Keys",
        values: &[
            (HARDNESS, 85.0),
            (BELL, 95.0),
            (VOICING, 75.0),
            (BARK, 15.0),
            (DECAY, 10000.0),
            (TONE, 20.0),
            (LEVEL, -2.0),
        ],
    },
    Preset {
        name: "Hollow Tines",
        values: &[
            (VOICING, 15.0),
            (BARK, 40.0),
            (BELL, 30.0),
            (TONE, -10.0),
            (LEVEL, -2.0),
        ],
    },
    Preset {
        name: "Pan Tines",
        values: &[
            (BELL, 45.0),
            (TREMOLO, 70.0),
            (TREM_RATE, 4.0),
            (TONE, 10.0),
        ],
    },
    Preset {
        name: "Ballad",
        values: &[
            (HARDNESS, 25.0),
            (BELL, 20.0),
            (VOICING, 70.0),
            (BARK, 15.0),
            (TONE, -50.0),
            (DECAY, 12000.0),
            (RELEASE, 350.0),
            (KEY_NOISE, 10.0),
        ],
    },
    Preset {
        name: "Dusty Keys",
        values: &[
            (HARDNESS, 40.0),
            (BELL, 30.0),
            (VOICING, 40.0),
            (BARK, 55.0),
            (TONE, -70.0),
            (DRIVE, 45.0),
            (KEY_NOISE, 45.0),
            (TREMOLO, 25.0),
            (TREM_MODE, AMP),
            (TREM_RATE, 3.0),
        ],
    },
    Preset {
        name: "Reed",
        values: &[(MODEL, REED), (DECAY, 3500.0), (RELEASE, 90.0)],
    },
    Preset {
        name: "Reed Bite",
        values: &[
            (MODEL, REED),
            (DECAY, 3000.0),
            (RELEASE, 80.0),
            (HARDNESS, 75.0),
            (VOICING, 25.0),
            (BARK, 85.0),
            (DRIVE, 30.0),
            (TONE, 20.0),
            (LEVEL, 2.0),
        ],
    },
    Preset {
        name: "Trem Reed",
        values: &[
            (MODEL, REED),
            (DECAY, 3500.0),
            (RELEASE, 100.0),
            (TREMOLO, 55.0),
            (TREM_MODE, AMP),
            (TREM_RATE, 5.6),
            (TONE, -10.0),
        ],
    },
];
