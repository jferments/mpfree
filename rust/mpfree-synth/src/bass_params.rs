//! The bass engine's parameters; its factory presets are in
//! `presets/bass.presets` ([`crate::factory::read`]).

use mpfree_node::{Curve, ParamInfo, Unit};

macro_rules! ids {
    ($($name:ident = $v:expr),* $(,)?) => { $(pub const $name: u16 = $v;)* };
}

ids! {
    BODY = 0, WAVE = 1, PW = 2, OCTAVE = 3, SUB = 4, SUB_MODE = 5, FIFTH = 6, NOISE = 7,
    DETUNE = 8, START = 9,
    PUNCH = 10, DROP = 11, DROP_TIME = 12,
    CUTOFF = 20, RESONANCE = 21, ENV_AMOUNT = 22, KEY_TRACK = 23, FILTER_DRIVE = 24,
    FILTER_DECAY = 25,
    ATTACK = 30, DECAY = 31, SUSTAIN = 32, RELEASE = 33, VELOCITY = 34, VEL_TONE = 35,
    GLIDE = 40, GLIDE_MODE = 41, GLIDE_ON = 42, PRIORITY = 43, RETRIGGER = 44,
    TRANSPOSE = 45, FINE = 46, BEND_RANGE = 47,
    DRIVE = 50, TONE = 51, HIGH_PASS = 52, LEVEL = 53, DRIVE_MODE = 54,
    PRESS_TO = 60, PRESSURE = 61,
}

/// The highest id plus one: values live in an array indexed by id.
pub const ID_SPACE: usize = 62;

/// The bodies: the synth bass (oscillators into the ladder), the 808 (a
/// sine or triangle with no filter, shaped by its pitch drop and the
/// drive), and the reese (two detuned oscillators over a clean sub).
pub const BODIES: &[&str] = &["Synth", "808", "Reese"];
pub const SYNTH: f32 = 0.0;
pub const B808: f32 = 1.0;
pub const REESE: f32 = 2.0;

pub const WAVES: &[&str] = &["Saw", "Pulse", "Triangle", "Sine"];
pub const SAW: f32 = 0.0;
pub const PULSE: f32 = 1.0;
pub const TRIANGLE: f32 = 2.0;
pub const SINE: f32 = 3.0;

/// The sub: a square or pulse divided from the oscillator (so it stays in
/// phase with it), or a sine.
pub const SUB_MODES: &[&str] = &[
    "Square -1 oct",
    "Square -2 oct",
    "Pulse -2 oct",
    "Sine -1 oct",
    "Sine",
];
pub const SUB_SQ1: f32 = 0.0;
pub const SUB_SQ2: f32 = 1.0;
pub const SUB_PULSE2: f32 = 2.0;
pub const SUB_SINE1: f32 = 3.0;
pub const SUB_SINE0: f32 = 4.0;

/// Where a note starts its oscillators: running on, or at the start of
/// the cycle (0°, rising from zero) or half way through (180°).
pub const STARTS: &[&str] = &["Free", "0°", "180°"];
pub const FREE: f32 = 0.0;
pub const DEG0: f32 = 1.0;
pub const DEG180: f32 = 2.0;

pub const GLIDE_MODES: &[&str] = &["Time", "Rate"];
/// When a note glides: only from a key still held, or always from the
/// last note.
pub const GLIDE_ONS: &[&str] = &["Legato", "Always"];
pub const LEGATO: f32 = 0.0;
pub const ALWAYS: f32 = 1.0;

pub const PRIORITIES: &[&str] = &["Last", "Low", "High"];
/// The drive: soft and off centre (the second and third harmonics), or
/// hard and even (a sine towards a square: the odd ones, high up).
pub const DRIVE_MODES: &[&str] = &["Soft", "Hard"];
pub const SOFT: f32 = 0.0;
pub const HARD: f32 = 1.0;
pub const OFF_ON: &[&str] = &["Off", "On"];

/// What pad pressure moves.
pub const PRESS_TOS: &[&str] = &["Off", "Cutoff", "Drive", "Vibrato"];
pub const PRESS_OFF: f32 = 0.0;
pub const PRESS_CUTOFF: f32 = 1.0;
pub const PRESS_DRIVE: f32 = 2.0;
pub const PRESS_VIBRATO: f32 = 3.0;

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

const fn ms(
    id: u16,
    name: &'static str,
    group: &'static str,
    min: f32,
    max: f32,
    default: f32,
) -> ParamInfo {
    p(id, name, group, min, max, default, Unit::Ms, Curve::Log)
}

use Curve::{Linear, Log, Stepped};

pub static PARAMS: &[ParamInfo] = &[
    choice(BODY, "Body", "Body", BODIES, SYNTH),
    // The oscillator (Synth and 808: the 808 takes a triangle, or else a
    // sine; Reese: both of its pair).
    choice(WAVE, "Wave", "Source", WAVES, SAW),
    p(
        PW,
        "Pulse width",
        "Source",
        5.0,
        50.0,
        50.0,
        Unit::Percent,
        Linear,
    ),
    p(
        OCTAVE,
        "Octave",
        "Source",
        -2.0,
        1.0,
        0.0,
        Unit::Octaves,
        Stepped,
    ),
    pct(SUB, "Sub", "Source", 50.0),
    choice(SUB_MODE, "Sub mode", "Source", SUB_MODES, SUB_SQ1),
    // A fifth above the oscillator, locked to it.
    pct(FIFTH, "Fifth", "Source", 0.0),
    pct(NOISE, "Noise", "Source", 0.0),
    // The reese's pair, each this far from the note, one up, one down.
    p(
        DETUNE,
        "Detune",
        "Source",
        0.0,
        100.0,
        27.0,
        Unit::Cents,
        Linear,
    ),
    choice(START, "Start", "Source", STARTS, DEG0),
    // The attack shift: up to an octave over the first 6 ms.
    pct(PUNCH, "Punch", "Pitch", 0.0),
    // The pitch drop: from this far above the note, landing on it at
    // Drop time.
    p(
        DROP,
        "Drop",
        "Pitch",
        0.0,
        24.0,
        0.0,
        Unit::Semitones,
        Linear,
    ),
    ms(DROP_TIME, "Drop time", "Pitch", 5.0, 1_000.0, 40.0),
    p(
        CUTOFF,
        "Cutoff",
        "Filter",
        20.0,
        12_000.0,
        800.0,
        Unit::Hz,
        Log,
    ),
    pct(RESONANCE, "Resonance", "Filter", 15.0),
    // The filter envelope's sweep: four octaves up (or down) at 100.
    p(
        ENV_AMOUNT,
        "Env amount",
        "Filter",
        -100.0,
        100.0,
        40.0,
        Unit::Percent,
        Linear,
    ),
    pct(KEY_TRACK, "Key track", "Filter", 0.0),
    // How hard the oscillators push the ladder: grit, not volume.
    pct(FILTER_DRIVE, "Filter drive", "Filter", 20.0),
    ms(FILTER_DECAY, "Filter decay", "Filter", 5.0, 5_000.0, 300.0),
    ms(ATTACK, "Attack", "Amp", 0.5, 2_000.0, 1.0),
    ms(DECAY, "Decay", "Amp", 5.0, 10_000.0, 600.0),
    pct(SUSTAIN, "Sustain", "Amp", 100.0),
    ms(RELEASE, "Release", "Amp", 5.0, 5_000.0, 80.0),
    // How much quieter the softest note is than the hardest.
    p(
        VELOCITY,
        "Velocity",
        "Amp",
        0.0,
        40.0,
        12.0,
        Unit::Db,
        Linear,
    ),
    // How much darker a soft note is: at 100 the softest is two octaves
    // darker (the cutoff and Tone).
    pct(VEL_TONE, "Vel > tone", "Amp", 40.0),
    // Time: every glide takes it. Rate: an octave takes it.
    p(GLIDE, "Glide", "Play", 0.0, 2_000.0, 60.0, Unit::Ms, Linear),
    choice(GLIDE_MODE, "Glide mode", "Play", GLIDE_MODES, 0.0),
    choice(GLIDE_ON, "Glide on", "Play", GLIDE_ONS, LEGATO),
    choice(PRIORITY, "Priority", "Play", PRIORITIES, 0.0),
    // A note played over a held key starts the envelopes again.
    choice(RETRIGGER, "Retrigger", "Play", OFF_ON, 0.0),
    p(
        TRANSPOSE,
        "Transpose",
        "Play",
        -24.0,
        24.0,
        0.0,
        Unit::Semitones,
        Stepped,
    ),
    p(
        FINE,
        "Fine",
        "Play",
        -100.0,
        100.0,
        0.0,
        Unit::Cents,
        Linear,
    ),
    p(
        BEND_RANGE,
        "Bend range",
        "Play",
        0.0,
        24.0,
        2.0,
        Unit::Semitones,
        Stepped,
    ),
    // Distortion after the filter, then a low-pass that keeps the low
    // harmonics and takes the fizz.
    pct(DRIVE, "Drive", "Output", 10.0),
    choice(DRIVE_MODE, "Drive mode", "Output", DRIVE_MODES, SOFT),
    p(
        TONE,
        "Tone",
        "Output",
        200.0,
        16_000.0,
        8_000.0,
        Unit::Hz,
        Log,
    ),
    p(
        HIGH_PASS,
        "High-pass",
        "Output",
        10.0,
        500.0,
        15.0,
        Unit::Hz,
        Log,
    ),
    p(LEVEL, "Level", "Output", -36.0, 6.0, 0.0, Unit::Db, Linear),
    choice(PRESS_TO, "Press to", "Pressure", PRESS_TOS, PRESS_OFF),
    pct(PRESSURE, "Pressure", "Pressure", 50.0),
];
