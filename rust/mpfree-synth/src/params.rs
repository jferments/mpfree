//! The Poly synth's parameters; its factory presets are in
//! `presets/poly.presets` ([`crate::factory::read`]).

use mpfree_node::{pressure as np, Curve, ParamInfo, Unit};

use crate::pressure::POLY_TARGETS;
use crate::voice::Source;
use crate::wavetable::params::{TABLE_NAMES, WARP_MODES};

macro_rules! ids {
    ($($name:ident = $v:expr),* $(,)?) => { $(pub const $name: u16 = $v;)* };
}

ids! {
    OSC1_WAVE = 0, OSC1_OCT = 1, OSC1_PW = 2, OSC1_LEVEL = 3,
    OSC1_TABLE = 4, OSC1_POS = 5, OSC1_WARP_MODE = 6, OSC1_WARP = 7, OSC1_SEMI = 8, OSC1_FINE = 9,
    OSC2_WAVE = 10, OSC2_OCT = 11, OSC2_SEMI = 12, OSC2_FINE = 13, OSC2_PW = 14, OSC2_LEVEL = 15,
    SUB_LEVEL = 16, NOISE_LEVEL = 17, SUB_WAVE = 18,
    UNISON = 20, UNISON_DETUNE = 21, UNISON_SPREAD = 22,
    SUPER_DETUNE = 24, SUPER_MIX = 25,
    OSC2_TABLE = 26, OSC2_POS = 27, OSC2_WARP_MODE = 28, OSC2_WARP = 29,
    FILTER_MODE = 30, CUTOFF = 31, RESONANCE = 32, DRIVE = 33, FENV_AMOUNT = 34, KEY_TRACK = 35, VEL_CUTOFF = 36, DIST = 37,
    FENV_A = 40, FENV_D = 41, FENV_S = 42, FENV_R = 43,
    FENV_POS1 = 44, FENV_POS2 = 45, VEL_POS = 46, FENV_WARP = 47,
    AMP_A = 50, AMP_D = 51, AMP_S = 52, AMP_R = 53, VEL_AMP = 54,
    LFO_SHAPE = 60, LFO_RATE = 61, LFO_SYNC = 62, LFO_PITCH = 63, LFO_CUTOFF = 64, LFO_PW = 65, LFO_AMP = 66, LFO_RETRIG = 67,
    LFO_POS1 = 68, LFO_POS2 = 69,
    VOICE_MODE = 70, POLYPHONY = 71, GLIDE = 72, BEND_RANGE = 73, PRIORITY = 74, PRESS_VIB_RATE = 75, AGE = 76,
    GLIDE_MODE = 77,
    LEVEL = 80, PAN = 81,
    METER_VOICES = 90,
    PRESS_BASE = 100,
    LFO2_SHAPE = 120, LFO2_RATE = 121, LFO2_SYNC = 122, LFO2_FADE = 123, LFO2_RETRIG = 124,
    LFO2_PITCH = 125, LFO2_CUTOFF = 126, LFO2_PW = 127, LFO2_AMP = 128,
    ROUTE_BASE = 130,
}

/// Play routes: each takes a source ([`Source`]: the mod wheel, breath, a
/// foot controller, velocity, the key, the note's pressure or tone, or a
/// random value per note) to one of [`POLY_TARGETS`] with a depth of -100
/// to 100 %, reaching as far as a pressure slot's
/// ([`crate::pressure::full_scale`]). With the four pressure slots, a
/// sound has eight routes.
pub const ROUTES: usize = 4;
/// What each route's three settings are, as offsets in its run of ids.
pub mod route {
    pub const SOURCE: u16 = 0;
    pub const TARGET: u16 = 1;
    pub const DEPTH: u16 = 2;
}
/// Route `n`'s (0 to 3) setting `field` ([`route`]).
pub const fn route_id(n: usize, field: u16) -> u16 {
    ROUTE_BASE + 3 * n as u16 + field
}
/// Which route and setting `id` is, if it is one.
pub fn route_of(id: u16) -> Option<(usize, u16)> {
    let off = id.checked_sub(ROUTE_BASE)?;
    (off < 3 * ROUTES as u16).then(|| (usize::from(off / 3), off % 3))
}

/// Pressure settings and slots: [`mpfree_node::pressure::field`] offsets
/// from [`PRESS_BASE`]. Id 74 (the old fixed pressure-to-cutoff amount) is
/// retired: slot 1 on Cutoff does its job.
pub const fn press(field: u16) -> u16 {
    PRESS_BASE + field
}
pub const PRESS_MODE: u16 = press(np::field::MODE);
pub const PRESS_HOLDOFF: u16 = press(np::field::HOLDOFF);
pub const PRESS_THRESHOLD: u16 = press(np::field::THRESHOLD);
pub const PRESS_FULL_AT: u16 = press(np::field::FULL_AT);
pub const PRESS_CURVE: u16 = press(np::field::CURVE);
pub const PRESS_RISE: u16 = press(np::field::RISE);
pub const PRESS_FALL: u16 = press(np::field::FALL);
pub const PRESS_RELEASE: u16 = press(np::field::RELEASE);
/// Slot `n` (0 to 3): its target ([`POLY_TARGETS`]) and depth.
pub const fn press_target(n: usize) -> u16 {
    press(np::field::target(n))
}
pub const fn press_depth(n: usize) -> u16 {
    press(np::field::depth(n))
}

/// Poly's pressure targets, by index into [`POLY_TARGETS`].
pub mod target {
    pub const OFF: u8 = 0;
    pub const CUTOFF: u8 = 1;
    pub const RESONANCE: u8 = 2;
    pub const LEVEL: u8 = 3;
    pub const VIBRATO: u8 = 4;
    pub const LFO_RATE: u8 = 5;
    pub const PITCH: u8 = 6;
    pub const DRIVE: u8 = 7;
    pub const WIDTH: u8 = 8;
    pub const SUB: u8 = 9;
    pub const NOISE: u8 = 10;
    pub const DIST: u8 = 11;
    pub const LFO1_AMOUNT: u8 = 12;
    pub const LFO2_AMOUNT: u8 = 13;
    pub const PAN: u8 = 14;
    pub const POSITION: u8 = 15;
    pub const WARP: u8 = 16;
}

/// Each oscillator's Table settings ([`WAVE_TABLE`]): the table, the
/// position through it, the warp and its amount, by oscillator (0 or 1).
pub const TABLE_IDS: [[u16; 4]; 2] = [
    [OSC1_TABLE, OSC1_POS, OSC1_WARP_MODE, OSC1_WARP],
    [OSC2_TABLE, OSC2_POS, OSC2_WARP_MODE, OSC2_WARP],
];

/// The highest id plus one: values live in an array indexed by id.
pub const ID_SPACE: usize = ROUTE_BASE as usize + 3 * ROUTES;

pub const WAVES: &[&str] = &["Saw", "Pulse", "Triangle", "Sine", "Supersaw", "Table"];
/// The Wave menu's Table: the oscillator reads a wavetable (Wavetable's
/// tables, [`crate::wavetable::factory`], or one loaded into it).
pub const WAVE_TABLE: u8 = 5;

/// The sub oscillator's waves ([`SUB_WAVE`]).
pub const SUB_WAVES: &[&str] = &["Square", "Sine"];
pub const FILTER_MODES: &[&str] = &["LP 24", "LP 12", "BP 12", "HP 12"];
pub const LFO_SHAPES: &[&str] = &["Sine", "Triangle", "Saw", "Square", "S&H"];
/// Tempo sync: off, or a cycle length in beats.
pub const LFO_SYNCS: &[&str] = &[
    "Off", "4 bars", "2 bars", "1 bar", "1/2", "1/4", "1/8", "1/16", "1/4 T", "1/8 T", "1/16 T",
];
pub const LFO_SYNC_BEATS: [f64; 11] = [
    0.0,
    16.0,
    8.0,
    4.0,
    2.0,
    1.0,
    0.5,
    0.25,
    2.0 / 3.0,
    1.0 / 3.0,
    1.0 / 6.0,
];
pub const VOICE_MODES: &[&str] = &["Poly", "Mono", "Legato"];
/// Which held key Mono and Legato play: the last pressed, the lowest or
/// the highest ([`crate::voice::Priority`]).
pub const PRIORITIES: &[&str] = crate::voice::Priority::NAMES;
/// How a glide moves: every interval in the glide's time, or at a fixed
/// speed, Glide then being how long an octave takes
/// ([`crate::voice::GlideMode`]).
pub const GLIDE_MODES: &[&str] = crate::voice::GlideMode::NAMES;

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
    choice(OSC1_WAVE, "Wave", "Osc 1", WAVES, 0.0),
    p(
        OSC1_OCT,
        "Octave",
        "Osc 1",
        -3.0,
        3.0,
        0.0,
        Unit::None,
        Stepped,
    ),
    p(
        OSC1_SEMI,
        "Semi",
        "Osc 1",
        -12.0,
        12.0,
        0.0,
        Unit::Semitones,
        Stepped,
    ),
    p(
        OSC1_FINE,
        "Fine",
        "Osc 1",
        -100.0,
        100.0,
        0.0,
        Unit::Cents,
        Linear,
    ),
    p(
        OSC1_PW,
        "Width",
        "Osc 1",
        5.0,
        95.0,
        50.0,
        Unit::Percent,
        Linear,
    ),
    p(
        OSC1_LEVEL,
        "Level",
        "Osc 1",
        0.0,
        100.0,
        100.0,
        Unit::Percent,
        Linear,
    ),
    // Its Table wave: which table, where through it, and the warp.
    choice(OSC1_TABLE, "Table", "Osc 1", TABLE_NAMES, 0.0),
    p(
        OSC1_POS,
        "Position",
        "Osc 1",
        0.0,
        100.0,
        0.0,
        Unit::Percent,
        Linear,
    ),
    choice(OSC1_WARP_MODE, "Warp", "Osc 1", WARP_MODES, 0.0),
    p(
        OSC1_WARP,
        "Warp amt",
        "Osc 1",
        0.0,
        100.0,
        0.0,
        Unit::Percent,
        Linear,
    ),
    choice(OSC2_WAVE, "Wave", "Osc 2", WAVES, 0.0),
    p(
        OSC2_OCT,
        "Octave",
        "Osc 2",
        -3.0,
        3.0,
        0.0,
        Unit::None,
        Stepped,
    ),
    p(
        OSC2_SEMI,
        "Semi",
        "Osc 2",
        -12.0,
        12.0,
        0.0,
        Unit::Semitones,
        Stepped,
    ),
    p(
        OSC2_FINE,
        "Fine",
        "Osc 2",
        -100.0,
        100.0,
        7.0,
        Unit::Cents,
        Linear,
    ),
    p(
        OSC2_PW,
        "Width",
        "Osc 2",
        5.0,
        95.0,
        50.0,
        Unit::Percent,
        Linear,
    ),
    p(
        OSC2_LEVEL,
        "Level",
        "Osc 2",
        0.0,
        100.0,
        60.0,
        Unit::Percent,
        Linear,
    ),
    // Its Table wave: which table, where through it, and the warp.
    choice(OSC2_TABLE, "Table", "Osc 2", TABLE_NAMES, 0.0),
    p(
        OSC2_POS,
        "Position",
        "Osc 2",
        0.0,
        100.0,
        0.0,
        Unit::Percent,
        Linear,
    ),
    choice(OSC2_WARP_MODE, "Warp", "Osc 2", WARP_MODES, 0.0),
    p(
        OSC2_WARP,
        "Warp amt",
        "Osc 2",
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
    // The sub's wave: a square, or a sine (the Wavetable synth's sub).
    choice(SUB_WAVE, "Sub wave", "Mix", SUB_WAVES, 0.0),
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
    // The supersaw's seven saws (either oscillator on Supersaw): how far
    // apart they go, and how loud the six around the note are against it.
    p(
        SUPER_DETUNE,
        "Detune",
        "Supersaw",
        0.0,
        100.0,
        50.0,
        Unit::Percent,
        Linear,
    ),
    p(
        SUPER_MIX,
        "Mix",
        "Supersaw",
        0.0,
        100.0,
        50.0,
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
        2_500.0,
        Unit::Hz,
        Log,
    ),
    p(
        RESONANCE,
        "Resonance",
        "Filter",
        0.0,
        100.0,
        15.0,
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
        FENV_AMOUNT,
        "Env amount",
        "Filter",
        -5.0,
        5.0,
        1.5,
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
        1.0,
        Unit::Octaves,
        Linear,
    ),
    p(
        FENV_A,
        "Attack",
        "Filter env",
        0.1,
        10_000.0,
        0.64,
        Unit::Ms,
        Log,
    ),
    p(
        FENV_D,
        "Decay",
        "Filter env",
        1.0,
        10_000.0,
        400.0,
        Unit::Ms,
        Log,
    ),
    p(
        FENV_S,
        "Sustain",
        "Filter env",
        0.0,
        100.0,
        30.0,
        Unit::Percent,
        Linear,
    ),
    p(
        FENV_R,
        "Release",
        "Filter env",
        1.0,
        10_000.0,
        295.0,
        Unit::Ms,
        Log,
    ),
    // What the filter envelope and velocity do to a Table wave.
    p(
        FENV_POS1,
        "To position 1",
        "Filter env",
        -100.0,
        100.0,
        0.0,
        Unit::Percent,
        Linear,
    ),
    p(
        FENV_POS2,
        "To position 2",
        "Filter env",
        -100.0,
        100.0,
        0.0,
        Unit::Percent,
        Linear,
    ),
    p(
        FENV_WARP,
        "To warp",
        "Filter env",
        -100.0,
        100.0,
        0.0,
        Unit::Percent,
        Linear,
    ),
    p(
        VEL_POS,
        "Vel to position",
        "Filter env",
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
        0.1,
        10_000.0,
        0.64,
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
        246.0,
        Unit::Ms,
        Log,
    ),
    // How much quieter the softest note is than the hardest, on the
    // voice core's square-law curve.
    p(
        VEL_AMP,
        "Velocity",
        "Amp env",
        0.0,
        40.0,
        8.0,
        Unit::Db,
        Linear,
    ),
    choice(LFO_SHAPE, "Shape", "LFO", LFO_SHAPES, 0.0),
    p(LFO_RATE, "Rate", "LFO", 0.05, 30.0, 5.0, Unit::Hz, Log),
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
        LFO_PW,
        "To width",
        "LFO",
        0.0,
        45.0,
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
    // A sweep up a Table wave from its position.
    p(
        LFO_POS1,
        "To position 1",
        "LFO",
        0.0,
        100.0,
        0.0,
        Unit::Percent,
        Linear,
    ),
    p(
        LFO_POS2,
        "To position 2",
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
    // The second LFO is each voice's own: it starts with each note (or
    // somewhere of its own when Retrigger is off) and fades in.
    choice(LFO2_SHAPE, "Shape", "LFO 2", LFO_SHAPES, 0.0),
    p(LFO2_RATE, "Rate", "LFO 2", 0.05, 30.0, 5.0, Unit::Hz, Log),
    choice(LFO2_SYNC, "Sync", "LFO 2", LFO_SYNCS, 0.0),
    p(
        LFO2_FADE,
        "Fade in",
        "LFO 2",
        1.0,
        10_000.0,
        1.0,
        Unit::Ms,
        Log,
    ),
    p(
        LFO2_RETRIG,
        "Retrigger",
        "LFO 2",
        0.0,
        1.0,
        1.0,
        Unit::None,
        Toggle,
    ),
    p(
        LFO2_PITCH,
        "To pitch",
        "LFO 2",
        0.0,
        12.0,
        0.0,
        Unit::Semitones,
        Linear,
    ),
    p(
        LFO2_CUTOFF,
        "To cutoff",
        "LFO 2",
        0.0,
        5.0,
        0.0,
        Unit::Octaves,
        Linear,
    ),
    p(
        LFO2_PW,
        "To width",
        "LFO 2",
        0.0,
        45.0,
        0.0,
        Unit::Percent,
        Linear,
    ),
    p(
        LFO2_AMP,
        "To level",
        "LFO 2",
        0.0,
        100.0,
        0.0,
        Unit::Percent,
        Linear,
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
    choice(GLIDE_MODE, "Glide mode", "Voice", GLIDE_MODES, 0.0),
    choice(PRIORITY, "Priority", "Voice", PRIORITIES, 0.0),
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
    // Character (the voice core's): how far each voice is from the
    // others, as on an analog poly's voice cards. 0, the default, is none.
    p(AGE, "Age", "Voice", 0.0, 100.0, 0.0, Unit::Percent, Linear),
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

/// Pressure: the shaper's settings, then four slots. Slot 1 opens the filter
/// by an octave by default, as the old fixed setting did.
const PRESS: [ParamInfo; np::FIELDS] = np::params(
    PRESS_BASE,
    "Pressure",
    POLY_TARGETS,
    [
        (target::CUTOFF as f32, 25.0),
        (0.0, 0.0),
        (0.0, 0.0),
        (0.0, 0.0),
    ],
);

const ROUTE_FROM: [&str; ROUTES] = ["From 1", "From 2", "From 3", "From 4"];
const ROUTE_TO: [&str; ROUTES] = ["To 1", "To 2", "To 3", "To 4"];
const ROUTE_DEPTH: [&str; ROUTES] = ["Depth 1", "Depth 2", "Depth 3", "Depth 4"];

/// The routes: the first sends the mod wheel to vibrato (half a semitone at
/// the top), as most keyboards expect; the rest are off.
const ROUTE_PARAMS: [ParamInfo; 3 * ROUTES] = {
    let mut out = [BASE[0]; 3 * ROUTES];
    let mut n = 0;
    while n < ROUTES {
        let (from, to, depth) = if n == 0 {
            (Source::ModWheel as u8 as f32, target::VIBRATO as f32, 50.0)
        } else {
            (0.0, 0.0, 0.0)
        };
        out[3 * n] = choice(
            route_id(n, route::SOURCE),
            ROUTE_FROM[n],
            "Routes",
            Source::NAMES,
            from,
        );
        out[3 * n + 1] = choice(
            route_id(n, route::TARGET),
            ROUTE_TO[n],
            "Routes",
            POLY_TARGETS,
            to,
        );
        out[3 * n + 2] = p(
            route_id(n, route::DEPTH),
            ROUTE_DEPTH[n],
            "Routes",
            -100.0,
            100.0,
            depth,
            Unit::Percent,
            Linear,
        );
        n += 1;
    }
    out
};

const N_PARAMS: usize = BASE.len() + np::FIELDS + 3 * ROUTES;

const fn all() -> [ParamInfo; N_PARAMS] {
    let mut out = [BASE[0]; N_PARAMS];
    let mut i = 0;
    while i < BASE.len() {
        out[i] = BASE[i];
        i += 1;
    }
    while i < BASE.len() + np::FIELDS {
        out[i] = PRESS[i - BASE.len()];
        i += 1;
    }
    while i < N_PARAMS {
        out[i] = ROUTE_PARAMS[i - BASE.len() - np::FIELDS];
        i += 1;
    }
    out
}

static ALL: [ParamInfo; N_PARAMS] = all();
pub static PARAMS: &[ParamInfo] = &ALL;

/// A factory preset: a name and the values that differ from the defaults,
/// read from the engine's preset file ([`crate::factory::read`]).
#[derive(Clone, Copy, Debug)]
pub struct Preset {
    pub name: &'static str,
    pub values: &'static [(u16, f32)],
    /// The preset as it is in the file, with any lines for what hosts the
    /// engine (a sound's effects and macros, `mpfree-instruments`).
    pub text: &'static str,
}

/// Looks up a parameter by id.
pub fn info(id: u16) -> Option<&'static ParamInfo> {
    PARAMS.iter().find(|p| p.id == id)
}
