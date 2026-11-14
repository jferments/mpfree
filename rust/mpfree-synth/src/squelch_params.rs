//! The mono acid bass's parameters and factory presets.

use mpfree_node::{Curve, ParamInfo, Unit};

use crate::params::Preset;

macro_rules! ids {
    ($($name:ident = $v:expr),* $(,)?) => { $(pub const $name: u16 = $v;)* };
}

ids! {
    WAVE = 0,
    CUTOFF = 10, RESONANCE = 11, ENV_MOD = 12, DECAY = 13, ACCENT = 14,
    SLIDE = 20, ACCENT_AT = 21, ACCENT_DECAY = 22, AMP_DECAY = 23, KEY_TRACK = 24,
    DRIVE = 30,
    TRANSPOSE = 40, FINE = 41, BEND_RANGE = 42,
    LEVEL = 50, PAN = 51,
}

/// The highest id plus one: values live in an array indexed by id.
pub const ID_SPACE: usize = 52;

/// The oscillator: a sawtooth, or the square a shaper makes from it.
pub const WAVES: &[&str] = &["Saw", "Square"];

pub const SAW: f32 = 0.0;
pub const SQUARE: f32 = 1.0;

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
    p(
        WAVE,
        "Wave",
        "Synth",
        0.0,
        1.0,
        SAW,
        Unit::None,
        Curve::Choice(WAVES),
    ),
    // Where the filter rests between notes (its resonant frequency with
    // no envelope).
    p(
        CUTOFF,
        "Cutoff",
        "Synth",
        30.0,
        5000.0,
        400.0,
        Unit::Hz,
        Log,
    ),
    // The filter's feedback: at 100 it sits at the edge of oscillation and
    // the harmonic nearest a mid or high cutoff rings out. Also how smooth
    // the accent's sweep is.
    pct(RESONANCE, "Resonance", "Synth", 60.0),
    // How far the envelope opens the filter (up to 4 octaves); more sweep
    // also starts a little lower.
    pct(ENV_MOD, "Env mod", "Synth", 50.0),
    // The time the filter's envelope takes to fall to a tenth.
    p(DECAY, "Decay", "Synth", 200.0, 2000.0, 500.0, Unit::Ms, Log),
    // How much an accented note jumps out: louder, and a sweep of the
    // filter that builds up over accents close together.
    pct(ACCENT, "Accent", "Synth", 60.0),
    // The glide to a note played while another is held.
    p(SLIDE, "Slide", "Play", 10.0, 500.0, 60.0, Unit::Ms, Log),
    // The velocity from which a note is accented.
    p(
        ACCENT_AT,
        "Accent at",
        "Play",
        1.0,
        127.0,
        100.0,
        Unit::None,
        Stepped,
    ),
    // The filter envelope's time on accented notes.
    p(
        ACCENT_DECAY,
        "Acc decay",
        "Play",
        30.0,
        2000.0,
        200.0,
        Unit::Ms,
        Log,
    ),
    // The volume envelope's fall to -60 dB while a note is held.
    p(
        AMP_DECAY,
        "Amp decay",
        "Play",
        100.0,
        8000.0,
        3500.0,
        Unit::Ms,
        Log,
    ),
    // How far the filter follows the note (100: an octave per octave).
    pct(KEY_TRACK, "Key track", "Play", 0.0),
    // Distortion after the filter, as from a pedal.
    pct(DRIVE, "Drive", "Play", 0.0),
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
];

pub static PRESETS: &[Preset] = &[
    Preset {
        name: "Init",
        values: &[],
    },
    // The classic squelch: high resonance, a fair sweep, short decay.
    Preset {
        name: "Acid Line",
        values: &[
            (CUTOFF, 320.0),
            (RESONANCE, 85.0),
            (ENV_MOD, 65.0),
            (DECAY, 350.0),
            (ACCENT, 80.0),
        ],
    },
    // Low and round: little resonance, a soft envelope, for bass lines
    // under a track.
    Preset {
        name: "Deep Bass",
        values: &[
            (CUTOFF, 180.0),
            (RESONANCE, 25.0),
            (ENV_MOD, 30.0),
            (DECAY, 700.0),
            (ACCENT, 40.0),
            (LEVEL, 2.0),
        ],
    },
    // The square through a nearly closed, very resonant filter.
    Preset {
        name: "Square Acid",
        values: &[
            (WAVE, SQUARE),
            (CUTOFF, 250.0),
            (RESONANCE, 90.0),
            (ENV_MOD, 75.0),
            (DECAY, 300.0),
            (ACCENT, 85.0),
        ],
    },
    // Distorted, with the filter wide open on accents.
    Preset {
        name: "Distorted",
        values: &[
            (CUTOFF, 400.0),
            (RESONANCE, 75.0),
            (ENV_MOD, 70.0),
            (DECAY, 450.0),
            (ACCENT, 90.0),
            (DRIVE, 70.0),
        ],
    },
    // Long decay and slow slides: lines that melt into each other.
    Preset {
        name: "Rubber",
        values: &[
            (CUTOFF, 220.0),
            (RESONANCE, 70.0),
            (ENV_MOD, 55.0),
            (DECAY, 1500.0),
            (SLIDE, 140.0),
            (ACCENT, 50.0),
        ],
    },
    // Short notes, short envelope, all click and squelch.
    Preset {
        name: "Plucky",
        values: &[
            (CUTOFF, 300.0),
            (RESONANCE, 65.0),
            (ENV_MOD, 80.0),
            (DECAY, 200.0),
            (AMP_DECAY, 400.0),
            (ACCENT, 70.0),
        ],
    },
    // The filter follows the keys: a lead an octave up.
    Preset {
        name: "Acid Lead",
        values: &[
            (CUTOFF, 600.0),
            (RESONANCE, 80.0),
            (ENV_MOD, 60.0),
            (DECAY, 400.0),
            (KEY_TRACK, 70.0),
            (TRANSPOSE, 12.0),
            (DRIVE, 30.0),
        ],
    },
    // Resonance right up: the harmonic at the cutoff screams.
    Preset {
        name: "Screamer",
        values: &[
            (CUTOFF, 700.0),
            (RESONANCE, 100.0),
            (ENV_MOD, 50.0),
            (DECAY, 600.0),
            (ACCENT, 100.0),
            (DRIVE, 40.0),
        ],
    },
];
