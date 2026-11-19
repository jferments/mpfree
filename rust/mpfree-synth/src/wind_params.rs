//! The wind engine's parameters; its factory presets are in
//! `presets/wind.presets` ([`crate::factory::read`]).

use mpfree_node::{pressure as np, Curve, ParamInfo, Unit};

use crate::pressure::WIND_TARGETS;

macro_rules! ids {
    ($($name:ident = $v:expr),* $(,)?) => { $(pub const $name: u16 = $v;)* };
}

ids! {
    MODEL = 0, BREATH = 1, REED = 2, EDGE = 3, TONE = 4, NOISE = 5, GROWL = 6,
    VIB_DEPTH = 10, VIB_RATE = 11, VIB_DELAY = 12,
    BREATH_FROM = 20, VELOCITY = 21, TONGUE = 22,
    ATTACK = 23, DECAY = 24, SUSTAIN = 25, RELEASE = 26,
    METER_BREATH = 27,
    VOICES = 30, LEGATO = 31, GLIDE = 32, GLIDE_MODE = 33, PRIORITY = 34,
    TRANSPOSE = 35, FINE = 36, BEND_RANGE = 37,
    LEVEL = 40, SPREAD = 41,
    PRESS_BASE = 48,
}

/// Pressure settings and slots: [`mpfree_node::pressure::field`] offsets
/// from [`PRESS_BASE`].
pub const fn press(field: u16) -> u16 {
    PRESS_BASE + field
}
/// Slot `n` (0 to 3): its target ([`WIND_TARGETS`]) and depth.
pub const fn press_target(n: usize) -> u16 {
    press(np::field::target(n))
}
pub const fn press_depth(n: usize) -> u16 {
    press(np::field::depth(n))
}

/// Wind's pressure targets, by index into [`WIND_TARGETS`].
pub mod target {
    pub const OFF: u8 = 0;
    pub const BREATH: u8 = 1;
    pub const VIBRATO: u8 = 2;
    pub const TONE: u8 = 3;
    pub const GROWL: u8 = 4;
    pub const NOISE: u8 = 5;
    pub const PITCH: u8 = 6;
}

/// The highest id plus one: values live in an array indexed by id.
pub const ID_SPACE: usize = PRESS_BASE as usize + np::FIELDS;

/// The bores: a cylinder closed by a reed (the clarinet's odd harmonics),
/// a cone with a reed (every harmonic, as a saxophone's or an oboe's),
/// and a pipe open at both ends blown across an edge (a flute's).
pub const MODELS: &[&str] = &["Clarinet", "Sax", "Flute"];
pub const CLARINET: f32 = 0.0;
pub const SAX: f32 = 1.0;
pub const FLUTE: f32 = 2.0;

/// Where the breath comes from: the pads (velocity, then pressure through
/// its slots), or a controller on MIDI in.
pub const BREATH_FROMS: &[&str] = &["Velocity", "Breath ctrl", "Mod wheel", "Foot pedal"];
pub const FROM_VELOCITY: f32 = 0.0;
pub const FROM_BREATH: f32 = 1.0;
pub const FROM_WHEEL: f32 = 2.0;
pub const FROM_FOOT: f32 = 3.0;

/// A note played over a held key: the breath carries on and only the
/// fingers move (Slur), or it is tongued again (Tongue).
pub const LEGATOS: &[&str] = &["Slur", "Tongue"];
pub const SLUR: f32 = 0.0;
pub const TONGUED: f32 = 1.0;

pub const GLIDE_MODES: &[&str] = &["Time", "Rate"];
pub const PRIORITIES: &[&str] = &["Last", "Low", "High"];

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

use Curve::{Linear, Stepped};

const BASE: [ParamInfo; 28] = [
    choice(MODEL, "Model", "Instrument", MODELS, CLARINET),
    // How hard the player blows: every source of breath is scaled by it.
    pct(BREATH, "Breath", "Instrument", 85.0),
    // The reed: soft (speaks easily, buzzes early) to hard (needs more
    // breath, rounder). Reeds only.
    pct(REED, "Reed", "Instrument", 45.0),
    // Where the flute's air jet meets the edge: centred (odd harmonics,
    // hollow) to off centre (the second harmonic, brighter). Flute only.
    pct(EDGE, "Edge", "Instrument", 60.0),
    // How much of the top the bore keeps: dark to bright, the same
    // number of harmonics on every note.
    pct(TONE, "Tone", "Instrument", 50.0),
    // Breath noise: the air rushing past the reed or across the edge.
    pct(NOISE, "Noise", "Instrument", 15.0),
    // A rough flutter in the breath, as a player singing into the horn.
    pct(GROWL, "Growl", "Instrument", 0.0),
    // Vibrato, mostly in the breath for a flute, mostly in the pitch for a
    // sax; the mod wheel adds to it.
    pct(VIB_DEPTH, "Vibrato", "Vibrato", 20.0),
    p(VIB_RATE, "Rate", "Vibrato", 1.0, 9.0, 5.0, Unit::Hz, Linear),
    // How long into a note before the vibrato starts; it fades in over the
    // same time again.
    ms(VIB_DELAY, "Delay", "Vibrato", 10.0, 2_000.0, 350.0),
    choice(
        BREATH_FROM,
        "Breath from",
        "Breath",
        BREATH_FROMS,
        FROM_VELOCITY,
    ),
    // How much quieter the softest breath is than the hardest.
    p(
        VELOCITY,
        "Velocity",
        "Breath",
        0.0,
        40.0,
        24.0,
        Unit::Db,
        Linear,
    ),
    // A tongued note starts with a puff of breath and a consonant.
    pct(TONGUE, "Tongue", "Breath", 35.0),
    ms(ATTACK, "Attack", "Breath", 1.0, 4_000.0, 15.0),
    ms(DECAY, "Decay", "Breath", 5.0, 10_000.0, 400.0),
    pct(SUSTAIN, "Sustain", "Breath", 90.0),
    ms(RELEASE, "Release", "Breath", 5.0, 4_000.0, 90.0),
    ParamInfo {
        id: METER_BREATH,
        name: "Breath now",
        group: "Breath",
        min: 0.0,
        max: 100.0,
        default: 0.0,
        unit: Unit::Percent,
        curve: Linear,
        readonly: true,
    },
    p(VOICES, "Voices", "Play", 1.0, 8.0, 1.0, Unit::None, Stepped),
    choice(LEGATO, "Legato", "Play", LEGATOS, SLUR),
    // Time: every move takes it. Rate: an octave takes it. A note over a
    // held key always moves over at least 10 ms, as fingers do.
    p(GLIDE, "Glide", "Play", 0.0, 2_000.0, 0.0, Unit::Ms, Linear),
    choice(GLIDE_MODE, "Glide mode", "Play", GLIDE_MODES, 0.0),
    choice(PRIORITY, "Priority", "Play", PRIORITIES, 0.0),
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
    p(LEVEL, "Level", "Play", -36.0, 6.0, 0.0, Unit::Db, Linear),
    // With more than one voice: alternate voices left and right.
    pct(SPREAD, "Spread", "Play", 0.0),
];

/// Pressure: slot 1 blows harder (the note swells and opens under the
/// finger), as the architecture's table has it for winds.
const PRESS: [ParamInfo; np::FIELDS] = np::params(
    PRESS_BASE,
    "Pressure",
    WIND_TARGETS,
    [
        (target::BREATH as f32, 60.0),
        (0.0, 0.0),
        (0.0, 0.0),
        (0.0, 0.0),
    ],
);

const N_BASE: usize = BASE.len();
const N_PARAMS: usize = N_BASE + np::FIELDS;

const fn all() -> [ParamInfo; N_PARAMS] {
    let mut out = [BASE[0]; N_PARAMS];
    let mut i = 0;
    while i < N_BASE {
        out[i] = BASE[i];
        i += 1;
    }
    while i < N_PARAMS {
        out[i] = PRESS[i - N_BASE];
        i += 1;
    }
    out
}

static ALL: [ParamInfo; N_PARAMS] = all();
pub static PARAMS: &[ParamInfo] = &ALL;
