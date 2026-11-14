//! The Wavetable synth's parameters and factory presets.

use mpfree_node::{pressure as np, Curve, ParamInfo, Unit};

use super::factory::NAMES as TABLES;
use crate::params::{Preset, FILTER_MODES, LFO_SHAPES, LFO_SYNCS, VOICE_MODES};
use crate::pressure::WT_TARGETS;

macro_rules! ids {
    ($($name:ident = $v:expr),* $(,)?) => { $(pub const $name: u16 = $v;)* };
}

// Oscillator A's ids; oscillator B's are the same plus `OSC_B`.
ids! {
    TABLE = 0, POSITION = 1, WARP_MODE = 2, WARP = 3, OCT = 4, SEMI = 5, FINE = 6, OSC_LEVEL = 7,
}
/// Offset from oscillator A's ids to oscillator B's.
pub const OSC_B: u16 = 10;
/// Oscillator `o`'s (0 = A, 1 = B) id for one of the ids above.
pub const fn osc_id(o: usize, id: u16) -> u16 {
    id + o as u16 * OSC_B
}

ids! {
    SUB_LEVEL = 20, NOISE_LEVEL = 21,
    UNISON = 25, UNISON_DETUNE = 26, UNISON_SPREAD = 27,
    FILTER_MODE = 30, CUTOFF = 31, RESONANCE = 32, DRIVE = 33, MENV_CUTOFF = 34, KEY_TRACK = 35, VEL_CUTOFF = 36, DIST = 37,
    MENV_A = 40, MENV_D = 41, MENV_S = 42, MENV_R = 43, MENV_POS_A = 44, MENV_POS_B = 45, VEL_POS = 46, MENV_WARP = 47,
    AMP_A = 50, AMP_D = 51, AMP_S = 52, AMP_R = 53, VEL_AMP = 54,
    LFO_SHAPE = 60, LFO_RATE = 61, LFO_SYNC = 62, LFO_PITCH = 63, LFO_CUTOFF = 64, LFO_POS_A = 65, LFO_POS_B = 66, LFO_RETRIG = 67, LFO_AMP = 68,
    VOICE_MODE = 70, POLYPHONY = 71, GLIDE = 72, BEND_RANGE = 73, PRESS_VIB_RATE = 75,
    LEVEL = 80, PAN = 81,
    METER_VOICES = 90,
    PRESS_BASE = 100,
}

pub const fn press(field: u16) -> u16 {
    PRESS_BASE + field
}
pub const PRESS_MODE: u16 = press(np::field::MODE);
pub const PRESS_THRESHOLD: u16 = press(np::field::THRESHOLD);
pub const PRESS_FULL_AT: u16 = press(np::field::FULL_AT);
pub const fn press_target(n: usize) -> u16 {
    press(np::field::target(n))
}
pub const fn press_depth(n: usize) -> u16 {
    press(np::field::depth(n))
}

/// The Wavetable synth's pressure targets, by index into [`WT_TARGETS`].
pub mod target {
    pub const OFF: u8 = 0;
    pub const POSITION: u8 = 1;
    pub const WARP: u8 = 2;
    pub const CUTOFF: u8 = 3;
    pub const RESONANCE: u8 = 4;
    pub const LEVEL: u8 = 5;
    pub const VIBRATO: u8 = 6;
    pub const LFO_RATE: u8 = 7;
    pub const PITCH: u8 = 8;
}

pub const ID_SPACE: usize = PRESS_BASE as usize + np::FIELDS;

/// The Table choice: the factory tables, then the user table loaded into
/// that oscillator (from a WAV or a pad's sample).
pub const TABLE_NAMES: &[&str] = &[
    "Basic",
    "Harmonics",
    "Pulse",
    "Sync",
    "Vowels",
    "Organ",
    "Bits",
    "Resonance",
    "FM",
    "Drift",
    "User",
];
/// The Table choice's value for the user table.
pub const USER_TABLE: f32 = (TABLE_NAMES.len() - 1) as f32;

/// How the phase is bent before the table is read.
/// - Bend: phase distortion, the cycle squeezed to the front (Casio CZ).
/// - Sync: the cycle repeats up to 8 times per period (hard sync).
/// - Mirror: plays the cycle forward then back (its first half doubled).
/// - FM: the other oscillator moves this one's phase.
pub const WARP_MODES: &[&str] = &["Off", "Bend", "Sync", "Mirror", "FM"];

const _: () = assert!(TABLE_NAMES.len() == TABLES.len() + 1);

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

use Curve::{Linear, Log, Stepped, Toggle};

const BASE: &[ParamInfo] = &[
    choice(TABLE, "Table", "Osc A", TABLE_NAMES, 0.0),
    p(
        POSITION,
        "Position",
        "Osc A",
        0.0,
        100.0,
        0.0,
        Unit::Percent,
        Linear,
    ),
    choice(WARP_MODE, "Warp", "Osc A", WARP_MODES, 0.0),
    p(
        WARP,
        "Warp amt",
        "Osc A",
        0.0,
        100.0,
        0.0,
        Unit::Percent,
        Linear,
    ),
    p(OCT, "Octave", "Osc A", -3.0, 3.0, 0.0, Unit::None, Stepped),
    p(
        SEMI,
        "Semi",
        "Osc A",
        -12.0,
        12.0,
        0.0,
        Unit::Semitones,
        Stepped,
    ),
    p(
        FINE,
        "Fine",
        "Osc A",
        -100.0,
        100.0,
        0.0,
        Unit::Cents,
        Linear,
    ),
    p(
        OSC_LEVEL,
        "Level",
        "Osc A",
        0.0,
        100.0,
        100.0,
        Unit::Percent,
        Linear,
    ),
    choice(OSC_B + TABLE, "Table", "Osc B", TABLE_NAMES, 1.0),
    p(
        OSC_B + POSITION,
        "Position",
        "Osc B",
        0.0,
        100.0,
        0.0,
        Unit::Percent,
        Linear,
    ),
    choice(OSC_B + WARP_MODE, "Warp", "Osc B", WARP_MODES, 0.0),
    p(
        OSC_B + WARP,
        "Warp amt",
        "Osc B",
        0.0,
        100.0,
        0.0,
        Unit::Percent,
        Linear,
    ),
    p(
        OSC_B + OCT,
        "Octave",
        "Osc B",
        -3.0,
        3.0,
        0.0,
        Unit::None,
        Stepped,
    ),
    p(
        OSC_B + SEMI,
        "Semi",
        "Osc B",
        -12.0,
        12.0,
        0.0,
        Unit::Semitones,
        Stepped,
    ),
    p(
        OSC_B + FINE,
        "Fine",
        "Osc B",
        -100.0,
        100.0,
        7.0,
        Unit::Cents,
        Linear,
    ),
    p(
        OSC_B + OSC_LEVEL,
        "Level",
        "Osc B",
        0.0,
        100.0,
        0.0,
        Unit::Percent,
        Linear,
    ),
    p(
        SUB_LEVEL,
        "Sub",
        "Mix",
        0.0,
        100.0,
        0.0,
        Unit::Percent,
        Linear,
    ),
    p(
        NOISE_LEVEL,
        "Noise",
        "Mix",
        0.0,
        100.0,
        0.0,
        Unit::Percent,
        Linear,
    ),
    p(
        UNISON,
        "Unison",
        "Unison",
        1.0,
        8.0,
        1.0,
        Unit::None,
        Stepped,
    ),
    p(
        UNISON_DETUNE,
        "Detune",
        "Unison",
        0.0,
        100.0,
        15.0,
        Unit::Cents,
        Linear,
    ),
    p(
        UNISON_SPREAD,
        "Spread",
        "Unison",
        0.0,
        100.0,
        60.0,
        Unit::Percent,
        Linear,
    ),
    choice(FILTER_MODE, "Type", "Filter", FILTER_MODES, 0.0),
    p(
        CUTOFF,
        "Cutoff",
        "Filter",
        20.0,
        20_000.0,
        8_000.0,
        Unit::Hz,
        Log,
    ),
    p(
        RESONANCE,
        "Resonance",
        "Filter",
        0.0,
        100.0,
        10.0,
        Unit::Percent,
        Linear,
    ),
    p(DRIVE, "Drive", "Filter", 0.0, 24.0, 0.0, Unit::Db, Linear),
    p(
        DIST,
        "Distortion",
        "Filter",
        0.0,
        24.0,
        0.0,
        Unit::Db,
        Linear,
    ),
    p(
        MENV_CUTOFF,
        "Env amount",
        "Filter",
        -5.0,
        5.0,
        0.0,
        Unit::Octaves,
        Linear,
    ),
    p(
        KEY_TRACK,
        "Key track",
        "Filter",
        0.0,
        100.0,
        50.0,
        Unit::Percent,
        Linear,
    ),
    p(
        VEL_CUTOFF,
        "Velocity",
        "Filter",
        0.0,
        4.0,
        0.5,
        Unit::Octaves,
        Linear,
    ),
    p(
        MENV_A,
        "Attack",
        "Mod env",
        0.5,
        10_000.0,
        2.0,
        Unit::Ms,
        Log,
    ),
    p(
        MENV_D,
        "Decay",
        "Mod env",
        1.0,
        10_000.0,
        600.0,
        Unit::Ms,
        Log,
    ),
    p(
        MENV_S,
        "Sustain",
        "Mod env",
        0.0,
        100.0,
        0.0,
        Unit::Percent,
        Linear,
    ),
    p(
        MENV_R,
        "Release",
        "Mod env",
        1.0,
        10_000.0,
        400.0,
        Unit::Ms,
        Log,
    ),
    p(
        MENV_POS_A,
        "To pos A",
        "Mod env",
        -100.0,
        100.0,
        0.0,
        Unit::Percent,
        Linear,
    ),
    p(
        MENV_POS_B,
        "To pos B",
        "Mod env",
        -100.0,
        100.0,
        0.0,
        Unit::Percent,
        Linear,
    ),
    p(
        MENV_WARP,
        "To warp",
        "Mod env",
        -100.0,
        100.0,
        0.0,
        Unit::Percent,
        Linear,
    ),
    p(
        VEL_POS,
        "Vel to pos",
        "Mod env",
        -100.0,
        100.0,
        0.0,
        Unit::Percent,
        Linear,
    ),
    p(
        AMP_A,
        "Attack",
        "Amp env",
        0.5,
        10_000.0,
        2.0,
        Unit::Ms,
        Log,
    ),
    p(
        AMP_D,
        "Decay",
        "Amp env",
        1.0,
        10_000.0,
        300.0,
        Unit::Ms,
        Log,
    ),
    p(
        AMP_S,
        "Sustain",
        "Amp env",
        0.0,
        100.0,
        80.0,
        Unit::Percent,
        Linear,
    ),
    p(
        AMP_R,
        "Release",
        "Amp env",
        1.0,
        10_000.0,
        250.0,
        Unit::Ms,
        Log,
    ),
    p(
        VEL_AMP,
        "Velocity",
        "Amp env",
        0.0,
        100.0,
        60.0,
        Unit::Percent,
        Linear,
    ),
    choice(LFO_SHAPE, "Shape", "LFO", LFO_SHAPES, 0.0),
    p(LFO_RATE, "Rate", "LFO", 0.05, 30.0, 2.0, Unit::Hz, Log),
    choice(LFO_SYNC, "Sync", "LFO", LFO_SYNCS, 0.0),
    p(
        LFO_PITCH,
        "To pitch",
        "LFO",
        0.0,
        12.0,
        0.0,
        Unit::Semitones,
        Linear,
    ),
    p(
        LFO_CUTOFF,
        "To cutoff",
        "LFO",
        0.0,
        5.0,
        0.0,
        Unit::Octaves,
        Linear,
    ),
    p(
        LFO_POS_A,
        "To pos A",
        "LFO",
        0.0,
        100.0,
        0.0,
        Unit::Percent,
        Linear,
    ),
    p(
        LFO_POS_B,
        "To pos B",
        "LFO",
        0.0,
        100.0,
        0.0,
        Unit::Percent,
        Linear,
    ),
    p(
        LFO_AMP,
        "To level",
        "LFO",
        0.0,
        100.0,
        0.0,
        Unit::Percent,
        Linear,
    ),
    p(
        LFO_RETRIG,
        "Retrigger",
        "LFO",
        0.0,
        1.0,
        0.0,
        Unit::None,
        Toggle,
    ),
    choice(VOICE_MODE, "Mode", "Voice", VOICE_MODES, 0.0),
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
    p(GLIDE, "Glide", "Voice", 0.0, 2_000.0, 0.0, Unit::Ms, Linear),
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
    p(
        PRESS_VIB_RATE,
        "Vib rate",
        "Voice",
        1.0,
        12.0,
        5.5,
        Unit::Hz,
        Linear,
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
        max: 32.0,
        default: 0.0,
        unit: Unit::None,
        curve: Stepped,
        readonly: true,
    },
];

/// Pressure: slot 1 scans the table (a third of it), slot 2 opens the
/// filter an octave.
const PRESS: [ParamInfo; np::FIELDS] = np::params(
    PRESS_BASE,
    "Pressure",
    WT_TARGETS,
    [
        (target::POSITION as f32, 33.0),
        (target::CUTOFF as f32, 25.0),
        (0.0, 0.0),
        (0.0, 0.0),
    ],
);

const N_PARAMS: usize = BASE.len() + np::FIELDS;

const fn all() -> [ParamInfo; N_PARAMS] {
    let mut out = [BASE[0]; N_PARAMS];
    let mut i = 0;
    while i < BASE.len() {
        out[i] = BASE[i];
        i += 1;
    }
    while i < N_PARAMS {
        out[i] = PRESS[i - BASE.len()];
        i += 1;
    }
    out
}

static ALL: [ParamInfo; N_PARAMS] = all();
pub static PARAMS: &[ParamInfo] = &ALL;

const B: u16 = OSC_B;

pub static PRESETS: &[Preset] = &[
    Preset {
        name: "Init",
        values: &[],
    },
    Preset {
        name: "Morph Pad",
        values: &[
            (TABLE, 4.0),
            (POSITION, 10.0),
            (B + TABLE, 9.0),
            (B + POSITION, 30.0),
            (B + OSC_LEVEL, 60.0),
            (B + FINE, 9.0),
            // Two oscillators times 3 stacks: 6 voices keep it inside the
            // synth budget on the MPC (an estimate until the device bench).
            (UNISON, 3.0),
            (UNISON_DETUNE, 14.0),
            (UNISON_SPREAD, 80.0),
            (POLYPHONY, 6.0),
            (CUTOFF, 3_500.0),
            (RESONANCE, 15.0),
            (AMP_A, 900.0),
            (AMP_D, 1_500.0),
            (AMP_S, 85.0),
            (AMP_R, 2_000.0),
            (LFO_SHAPE, 1.0),
            (LFO_RATE, 0.15),
            (LFO_POS_A, 60.0),
            (LFO_POS_B, 40.0),
            (LEVEL, -4.0),
            (press_target(0), target::POSITION as f32),
            (press_depth(0), 40.0),
            (press_target(1), target::LEVEL as f32),
            (press_depth(1), 25.0),
        ],
    },
    Preset {
        name: "Vowel Lead",
        values: &[
            (TABLE, 4.0),
            (B + TABLE, 0.0),
            (B + POSITION, 66.0),
            (B + OCT, -1.0),
            (B + OSC_LEVEL, 35.0),
            (CUTOFF, 6_000.0),
            (MENV_POS_A, 70.0),
            (MENV_D, 900.0),
            (MENV_S, 40.0),
            (VOICE_MODE, 2.0),
            (GLIDE, 70.0),
            (LFO_PITCH, 0.2),
            (LFO_RATE, 5.5),
            (BEND_RANGE, 12.0),
            (press_target(0), target::POSITION as f32),
            (press_depth(0), 60.0),
            (press_target(1), target::VIBRATO as f32),
            (press_depth(1), 40.0),
        ],
    },
    Preset {
        name: "Sync Sweep",
        values: &[
            (TABLE, 3.0),
            (B + TABLE, 0.0),
            (B + POSITION, 66.0),
            (B + OCT, -1.0),
            (B + OSC_LEVEL, 40.0),
            (MENV_POS_A, 100.0),
            (MENV_D, 450.0),
            (MENV_S, 10.0),
            (CUTOFF, 9_000.0),
            (DRIVE, 6.0),
            (AMP_D, 600.0),
            (AMP_S, 60.0),
            (AMP_R, 200.0),
            (VEL_POS, 30.0),
        ],
    },
    Preset {
        name: "Digital Bell",
        values: &[
            (TABLE, 8.0),
            (POSITION, 70.0),
            (MENV_POS_A, -70.0),
            (MENV_D, 800.0),
            (WARP_MODE, 2.0),
            (MENV_WARP, 25.0),
            (B + TABLE, 7.0),
            (B + POSITION, 40.0),
            (B + OCT, 1.0),
            (B + OSC_LEVEL, 30.0),
            (AMP_D, 1_800.0),
            (AMP_S, 0.0),
            (AMP_R, 1_200.0),
            (CUTOFF, 12_000.0),
            (VEL_POS, 30.0),
            (press_target(0), target::WARP as f32),
            (press_depth(0), 30.0),
            (press_target(1), target::OFF as f32),
        ],
    },
    Preset {
        name: "Growl Bass",
        values: &[
            (TABLE, 7.0),
            (OCT, -1.0),
            (WARP_MODE, 1.0),
            (WARP, 20.0),
            (B + TABLE, 0.0),
            (B + POSITION, 100.0),
            (B + OCT, -2.0),
            (B + OSC_LEVEL, 50.0),
            (LFO_SYNC, 7.0),
            (LFO_POS_A, 70.0),
            (LFO_SHAPE, 1.0),
            (CUTOFF, 1_400.0),
            (RESONANCE, 30.0),
            (DRIVE, 12.0),
            (AMP_S, 100.0),
            (AMP_R, 80.0),
            (VOICE_MODE, 2.0),
            (GLIDE, 30.0),
            (KEY_TRACK, 30.0),
            (press_target(0), target::WARP as f32),
            (press_depth(0), 50.0),
            (press_target(1), target::CUTOFF as f32),
            (press_depth(1), 25.0),
        ],
    },
    Preset {
        name: "Organ",
        values: &[
            (TABLE, 5.0),
            (POSITION, 45.0),
            (B + OSC_LEVEL, 0.0),
            (CUTOFF, 12_000.0),
            (AMP_A, 4.0),
            (AMP_S, 100.0),
            (AMP_R, 60.0),
            (VEL_AMP, 0.0),
            (LFO_PITCH, 0.08),
            (LFO_RATE, 6.5),
            (press_target(0), target::POSITION as f32),
            (press_depth(0), 55.0),
            (press_target(1), target::OFF as f32),
        ],
    },
    Preset {
        name: "Harmonic Pluck",
        values: &[
            (TABLE, 1.0),
            (POSITION, 100.0),
            (MENV_POS_A, -100.0),
            (MENV_D, 350.0),
            (MENV_S, 0.0),
            (B + OSC_LEVEL, 0.0),
            (AMP_D, 700.0),
            (AMP_S, 0.0),
            (AMP_R, 400.0),
            (VEL_POS, -40.0),
            (CUTOFF, 14_000.0),
            (press_target(0), target::OFF as f32),
            (press_target(1), target::OFF as f32),
        ],
    },
    Preset {
        name: "FM Keys",
        values: &[
            (TABLE, 0.0),
            (POSITION, 0.0),
            (WARP_MODE, 4.0),
            (WARP, 8.0),
            (B + TABLE, 0.0),
            (B + OCT, 1.0),
            (B + OSC_LEVEL, 0.0),
            (MENV_WARP, 45.0),
            (MENV_D, 900.0),
            (AMP_D, 1_400.0),
            (AMP_S, 30.0),
            (AMP_R, 500.0),
            (CUTOFF, 10_000.0),
            (press_target(0), target::WARP as f32),
            (press_depth(0), 35.0),
            (press_target(1), target::OFF as f32),
        ],
    },
];

/// Looks up a parameter by id.
pub fn info(id: u16) -> Option<&'static ParamInfo> {
    PARAMS.iter().find(|p| p.id == id)
}
