//! The string machine's parameters and factory presets.

use mpfree_node::{Curve, ParamInfo, Unit};

use crate::params::Preset;

macro_rules! ids {
    ($($name:ident = $v:expr),* $(,)?) => { $(pub const $name: u16 = $v;)* };
}

ids! {
    VIOLIN = 0, VIOLA = 1, CELLO = 2, CONTRABASS = 3, HORN = 4, TRUMPET = 5, OCTAVE = 6,
    BRIGHTNESS = 10, BODY = 11, NOISE = 12,
    CRESCENDO = 20, SUSTAIN = 21, SWELL = 22, VELOCITY = 23,
    ENSEMBLE = 30, LINES = 31, SLOW_RATE = 32, FAST_RATE = 33, DEPTH = 34, WIDTH = 35,
    TRANSPOSE = 40, FINE = 41, BEND_RANGE = 42,
    LEVEL = 50,
    METER_KEYS = 70,
}

/// The highest id plus one: values live in an array indexed by id.
pub const ID_SPACE: usize = 71;

/// The registers in order: their ids and names.
pub const REGISTERS: [(u16, &str); 6] = [
    (VIOLIN, "Violin"),
    (VIOLA, "Viola"),
    (CELLO, "Cello"),
    (CONTRABASS, "Contrabass"),
    (HORN, "Horn"),
    (TRUMPET, "Trumpet"),
];

/// The swell: each key its own, or one for every key (the old machines'
/// way: a chord swells in together and the last one fades out).
pub const SWELLS: &[&str] = &["Per key", "Shared"];
pub const PER_KEY: f32 = 0.0;
pub const SHARED: f32 = 1.0;

/// The ensemble's lines.
pub const LINE_COUNTS: &[&str] = &["3", "4"];

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
    // Each register is a mix of the bank's octaves through its own fixed
    // tone filter; any mix of them sounds at once.
    pct(VIOLIN, "Violin", "Registers", 100.0),
    pct(VIOLA, "Viola", "Registers", 70.0),
    pct(CELLO, "Cello", "Registers", 0.0),
    pct(CONTRABASS, "Contrabass", "Registers", 0.0),
    pct(HORN, "Horn", "Registers", 0.0),
    pct(TRUMPET, "Trumpet", "Registers", 0.0),
    // Octaves up or down for the next notes played.
    p(
        OCTAVE,
        "Octave",
        "Registers",
        -2.0,
        2.0,
        0.0,
        Unit::Octaves,
        Stepped,
    ),
    // The registers' filters an octave down (0) or up (100) from where
    // they sit (50).
    pct(BRIGHTNESS, "Brightness", "Tone", 50.0),
    // The registers' body resonance: none (0), as voiced (50), doubled.
    pct(BODY, "Body", "Tone", 50.0),
    // Bow noise, following how loud the keys are.
    pct(NOISE, "Bow noise", "Tone", 10.0),
    // The swell in: silence to full level.
    p(
        CRESCENDO,
        "Crescendo",
        "Swell",
        2.0,
        8_000.0,
        400.0,
        Unit::Ms,
        Log,
    ),
    // The fade out: 60 dB down from full level.
    p(
        SUSTAIN,
        "Sustain",
        "Swell",
        20.0,
        12_000.0,
        1_500.0,
        Unit::Ms,
        Log,
    ),
    p(
        SWELL,
        "Swell",
        "Swell",
        0.0,
        1.0,
        PER_KEY,
        Unit::None,
        Curve::Choice(SWELLS),
    ),
    // How much quieter the softest note is than the hardest, and it is
    // darker too (0: velocity is ignored, as on the old machines).
    p(
        VELOCITY,
        "Velocity",
        "Swell",
        0.0,
        40.0,
        25.0,
        Unit::Db,
        Linear,
    ),
    // The built-in ensemble: dry (0) to all ensemble (100).
    pct(ENSEMBLE, "Ensemble", "Ensemble", 100.0),
    p(
        LINES,
        "Lines",
        "Ensemble",
        0.0,
        1.0,
        0.0,
        Unit::None,
        Curve::Choice(LINE_COUNTS),
    ),
    p(
        SLOW_RATE,
        "Slow rate",
        "Ensemble",
        0.1,
        3.0,
        0.66,
        Unit::Hz,
        Log,
    ),
    p(
        FAST_RATE,
        "Fast rate",
        "Ensemble",
        2.0,
        12.0,
        6.25,
        Unit::Hz,
        Log,
    ),
    // Both sweeps' depth against the ensemble's own (12 and 5 cents).
    p(
        DEPTH,
        "Depth",
        "Ensemble",
        0.0,
        200.0,
        100.0,
        Unit::Percent,
        Linear,
    ),
    pct(WIDTH, "Width", "Ensemble", 85.0),
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
        BEND_RANGE,
        "Bend range",
        "Voice",
        0.0,
        24.0,
        2.0,
        Unit::Semitones,
        Stepped,
    ),
    p(LEVEL, "Level", "Output", -36.0, 6.0, 0.0, Unit::Db, Linear),
    ParamInfo {
        id: METER_KEYS,
        name: "Keys held",
        group: "Output",
        min: 0.0,
        max: 128.0,
        default: 0.0,
        unit: Unit::None,
        curve: Stepped,
        readonly: true,
    },
];

/// The factory presets: string sections, brass, swells and the
/// ensemble's moods. None sets Level, so they sit at the same loudness.
pub static PRESETS: &[Preset] = &[
    Preset {
        name: "Init",
        values: &[],
    },
    Preset {
        name: "Classic Strings",
        values: &[
            (VIOLIN, 100.0),
            (VIOLA, 80.0),
            (CELLO, 35.0),
            (CRESCENDO, 300.0),
            (SUSTAIN, 1_800.0),
        ],
    },
    Preset {
        name: "Bright Strings",
        values: &[
            (VIOLIN, 100.0),
            (VIOLA, 40.0),
            (BRIGHTNESS, 75.0),
            (BODY, 60.0),
            (CRESCENDO, 150.0),
        ],
    },
    Preset {
        name: "Cello and Viola",
        values: &[
            (VIOLIN, 0.0),
            (VIOLA, 90.0),
            (CELLO, 100.0),
            (BRIGHTNESS, 40.0),
            (CRESCENDO, 350.0),
            (SUSTAIN, 2_000.0),
        ],
    },
    Preset {
        name: "Full Ensemble",
        values: &[
            (VIOLIN, 90.0),
            (VIOLA, 80.0),
            (CELLO, 70.0),
            (CONTRABASS, 40.0),
            (LINES, 1.0),
            (SUSTAIN, 2_200.0),
        ],
    },
    Preset {
        name: "Orchestra",
        values: &[
            (VIOLIN, 80.0),
            (VIOLA, 60.0),
            (CELLO, 60.0),
            (HORN, 60.0),
            (TRUMPET, 30.0),
            (CRESCENDO, 200.0),
            (SUSTAIN, 1_600.0),
            // Five registers at once: room for a full chord.
            (LEVEL, -2.0),
        ],
    },
    Preset {
        name: "Brass Ensemble",
        values: &[
            (VIOLIN, 0.0),
            (VIOLA, 0.0),
            (HORN, 100.0),
            (TRUMPET, 70.0),
            (ENSEMBLE, 60.0),
            (CRESCENDO, 60.0),
            (SUSTAIN, 600.0),
            (VELOCITY, 20.0),
        ],
    },
    Preset {
        name: "Slow Swell",
        values: &[
            (VIOLIN, 90.0),
            (VIOLA, 90.0),
            (CELLO, 40.0),
            (CRESCENDO, 2_500.0),
            (SUSTAIN, 4_000.0),
        ],
    },
    Preset {
        name: "Staccato Strings",
        values: &[
            (VIOLIN, 100.0),
            (VIOLA, 60.0),
            (CELLO, 30.0),
            (CRESCENDO, 12.0),
            (SUSTAIN, 220.0),
            (VELOCITY, 18.0),
            (NOISE, 25.0),
        ],
    },
    Preset {
        name: "Swirl Strings",
        values: &[
            (VIOLIN, 100.0),
            (VIOLA, 70.0),
            (LINES, 1.0),
            (SLOW_RATE, 0.18),
            (FAST_RATE, 5.9),
            (DEPTH, 140.0),
            (SUSTAIN, 2_500.0),
        ],
    },
    Preset {
        name: "Dark Strings",
        values: &[
            (VIOLIN, 30.0),
            (VIOLA, 90.0),
            (CELLO, 70.0),
            (BRIGHTNESS, 15.0),
            (BODY, 70.0),
            (SUSTAIN, 2_500.0),
        ],
    },
    Preset {
        name: "High Strings",
        values: &[
            (VIOLIN, 100.0),
            (VIOLA, 40.0),
            (OCTAVE, 1.0),
            (BRIGHTNESS, 55.0),
            (CRESCENDO, 450.0),
        ],
    },
    Preset {
        name: "Low Strings",
        values: &[
            (VIOLIN, 0.0),
            (VIOLA, 60.0),
            (CELLO, 100.0),
            (CONTRABASS, 60.0),
            (BRIGHTNESS, 60.0),
            (CRESCENDO, 250.0),
            (SUSTAIN, 1_200.0),
        ],
    },
    Preset {
        name: "Paraphonic 70s",
        values: &[
            (VIOLIN, 100.0),
            (VIOLA, 80.0),
            (SWELL, SHARED),
            (CRESCENDO, 500.0),
            (SUSTAIN, 2_000.0),
        ],
    },
    Preset {
        name: "Disco Strings",
        values: &[
            (VIOLIN, 100.0),
            (VIOLA, 50.0),
            (BRIGHTNESS, 65.0),
            (CRESCENDO, 40.0),
            (SUSTAIN, 900.0),
            (FAST_RATE, 7.0),
        ],
    },
    Preset {
        name: "Ballad Pad",
        values: &[
            (VIOLIN, 40.0),
            (VIOLA, 80.0),
            (CELLO, 60.0),
            (CRESCENDO, 900.0),
            (SUSTAIN, 3_500.0),
            (DEPTH, 80.0),
        ],
    },
    Preset {
        name: "Lo-fi Strings",
        values: &[
            (VIOLIN, 100.0),
            (VIOLA, 60.0),
            (BRIGHTNESS, 25.0),
            (NOISE, 45.0),
            (DEPTH, 160.0),
            (WIDTH, 60.0),
        ],
    },
    Preset {
        name: "Wide Strings",
        values: &[
            (VIOLIN, 100.0),
            (VIOLA, 80.0),
            (LINES, 1.0),
            (WIDTH, 100.0),
            (DEPTH, 110.0),
        ],
    },
    Preset {
        name: "Mono Ensemble",
        values: &[(VIOLIN, 100.0), (VIOLA, 80.0), (WIDTH, 0.0)],
    },
    Preset {
        name: "Dry Divider",
        values: &[(VIOLIN, 100.0), (VIOLA, 70.0), (ENSEMBLE, 0.0)],
    },
    Preset {
        name: "Bowed",
        values: &[
            (VIOLIN, 100.0),
            (VIOLA, 50.0),
            (NOISE, 60.0),
            (BODY, 80.0),
            (CRESCENDO, 180.0),
            (VELOCITY, 20.0),
        ],
    },
    Preset {
        name: "Horn Section",
        values: &[
            (VIOLIN, 0.0),
            (VIOLA, 0.0),
            (CELLO, 30.0),
            (HORN, 100.0),
            (CRESCENDO, 120.0),
            (SUSTAIN, 700.0),
        ],
    },
    Preset {
        name: "Brassy Lead",
        values: &[
            (VIOLIN, 0.0),
            (VIOLA, 0.0),
            (TRUMPET, 100.0),
            (ENSEMBLE, 40.0),
            (CRESCENDO, 30.0),
            (SUSTAIN, 400.0),
            (VELOCITY, 14.0),
        ],
    },
    Preset {
        name: "Vox Strings",
        values: &[
            (VIOLIN, 0.0),
            (VIOLA, 60.0),
            (HORN, 50.0),
            (BRIGHTNESS, 35.0),
            (BODY, 90.0),
            (CRESCENDO, 600.0),
            (SUSTAIN, 2_500.0),
        ],
    },
];
