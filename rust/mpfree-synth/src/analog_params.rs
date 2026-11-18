//! The vintage analog poly's parameters; its factory presets are in
//! `presets/analog.presets` ([`crate::factory::read`]).

use mpfree_node::{Curve, ParamInfo, Unit};

macro_rules! ids {
    ($($name:ident = $v:expr),* $(,)?) => { $(pub const $name: u16 = $v;)* };
}

ids! {
    OSC1_WAVE = 0, OSC1_OCT = 1, OSC1_PW = 2,
    OSC2_WAVE = 3, OSC2_OCT = 4, OSC2_SEMI = 5, OSC2_FINE = 6, OSC2_PW = 7,
    SYNC = 8,
    OSC1_LEVEL = 10, OSC2_LEVEL = 11, SUB_LEVEL = 12, NOISE_LEVEL = 13,
    FILTER_TYPE = 20, CUTOFF = 21, RESONANCE = 22, ENV_AMT = 23, KEY_TRACK = 24,
    VEL_FILTER = 25, DRIVE = 26,
    F_ATTACK = 30, F_DECAY = 31, F_SUSTAIN = 32, F_RELEASE = 33,
    A_ATTACK = 34, A_DECAY = 35, A_SUSTAIN = 36, A_RELEASE = 37, VEL_AMP = 38,
    LFO_RATE = 40, LFO_WAVE = 41, LFO_DELAY = 42, LFO_PITCH = 43, LFO_PW = 44,
    LFO_CUTOFF = 45,
    PM_ENV = 46, PM_OSC2 = 47, PM_PITCH = 48, PM_PW = 49, PM_CUTOFF = 50,
    VOICE_MODE = 60, POLYPHONY = 61, UNI_DETUNE = 62, GLIDE = 63, AGE = 64, CHORUS = 65,
    TRANSPOSE = 66, FINE = 67, BEND_RANGE = 68,
    LEVEL = 70, PAN = 71,
}

/// The highest id plus one: values live in an array indexed by id.
pub const ID_SPACE: usize = 72;

/// An oscillator's wave: a sawtooth, a pulse (its width set apart), both
/// sounding together, or a triangle.
pub const WAVES: &[&str] = &["Saw", "Pulse", "Saw+Pulse", "Triangle"];
pub const SAW: f32 = 0.0;
pub const PULSE: f32 = 1.0;
pub const SAW_PULSE: f32 = 2.0;
pub const TRIANGLE: f32 = 3.0;

/// The four-pole OTA low-pass, or the two-pole state-variable filter's
/// low-, band- or high-pass.
pub const FILTERS: &[&str] = &["LP 24", "LP 12", "BP 12", "HP 12"];
pub const LP24: f32 = 0.0;
pub const LP12: f32 = 1.0;
pub const BP12: f32 = 2.0;
pub const HP12: f32 = 3.0;

/// The LFO's shapes, in the order `dsp::LfoShape::from_index` reads.
pub const LFO_WAVES: &[&str] = &["Sine", "Triangle", "Saw", "Square", "S&H"];

/// Poly: a card per note, in turn. Unison: every card on the one note,
/// detuned.
pub const VOICE_MODES: &[&str] = &["Poly", "Unison"];
pub const POLY: f32 = 0.0;
pub const UNISON: f32 = 1.0;

/// The chorus: off, a slow deep sweep, a faster one, or a quick shallow
/// vibrato.
pub const CHORUSES: &[&str] = &["Off", "Slow", "Fast", "Vibe"];
pub const CHORUS_OFF: f32 = 0.0;
pub const CHORUS_SLOW: f32 = 1.0;
pub const CHORUS_FAST: f32 = 2.0;
pub const CHORUS_VIBE: f32 = 3.0;

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

const fn toggle(id: u16, name: &'static str, group: &'static str, default: f32) -> ParamInfo {
    p(
        id,
        name,
        group,
        0.0,
        1.0,
        default,
        Unit::None,
        Curve::Toggle,
    )
}

/// An envelope time: 1 ms to 20 s.
const fn time(id: u16, name: &'static str, default: f32) -> ParamInfo {
    p(
        id,
        name,
        "Envelopes",
        1.0,
        20000.0,
        default,
        Unit::Ms,
        Curve::Log,
    )
}

use Curve::{Linear, Log, Stepped};

pub static PARAMS: &[ParamInfo] = &[
    choice(OSC1_WAVE, "Osc 1 wave", "Osc", WAVES, SAW),
    p(
        OSC1_OCT,
        "Osc 1 oct",
        "Osc",
        -2.0,
        2.0,
        0.0,
        Unit::Octaves,
        Stepped,
    ),
    p(
        OSC1_PW,
        "Osc 1 PW",
        "Osc",
        5.0,
        95.0,
        50.0,
        Unit::Percent,
        Linear,
    ),
    choice(OSC2_WAVE, "Osc 2 wave", "Osc", WAVES, SAW),
    p(
        OSC2_OCT,
        "Osc 2 oct",
        "Osc",
        -2.0,
        2.0,
        0.0,
        Unit::Octaves,
        Stepped,
    ),
    p(
        OSC2_SEMI,
        "Osc 2 semi",
        "Osc",
        -12.0,
        12.0,
        0.0,
        Unit::Semitones,
        Stepped,
    ),
    p(
        OSC2_FINE,
        "Osc 2 fine",
        "Osc",
        -50.0,
        50.0,
        6.0,
        Unit::Cents,
        Linear,
    ),
    p(
        OSC2_PW,
        "Osc 2 PW",
        "Osc",
        5.0,
        95.0,
        50.0,
        Unit::Percent,
        Linear,
    ),
    // Oscillator 1 starts its cycle again whenever oscillator 2 does.
    toggle(SYNC, "Sync", "Osc", 0.0),
    pct(OSC1_LEVEL, "Osc 1", "Osc", 100.0),
    pct(OSC2_LEVEL, "Osc 2", "Osc", 0.0),
    // A square an octave under oscillator 1.
    pct(SUB_LEVEL, "Sub", "Osc", 0.0),
    pct(NOISE_LEVEL, "Noise", "Osc", 0.0),
    choice(FILTER_TYPE, "Filter", "Filter", FILTERS, LP24),
    p(
        CUTOFF,
        "Cutoff",
        "Filter",
        20.0,
        18000.0,
        8000.0,
        Unit::Hz,
        Log,
    ),
    // At 100 the four-pole whistles on its own; the two-pole only rings.
    pct(RESONANCE, "Resonance", "Filter", 0.0),
    // How far the filter envelope moves the cutoff: 100 is 5 octaves up,
    // -100 5 octaves down.
    p(
        ENV_AMT,
        "Env amount",
        "Filter",
        -100.0,
        100.0,
        0.0,
        Unit::Percent,
        Linear,
    ),
    // How far the cutoff follows the note (100: an octave per octave,
    // around middle C).
    pct(KEY_TRACK, "Key track", "Filter", 50.0),
    // How much softer notes open the filter less.
    pct(VEL_FILTER, "Vel > filter", "Filter", 0.0),
    // How hard the oscillators push the filter's stages.
    pct(DRIVE, "Drive", "Filter", 0.0),
    time(F_ATTACK, "F attack", 2.0),
    time(F_DECAY, "F decay", 600.0),
    pct(F_SUSTAIN, "F sustain", "Envelopes", 40.0),
    time(F_RELEASE, "F release", 400.0),
    time(A_ATTACK, "A attack", 2.0),
    time(A_DECAY, "A decay", 600.0),
    pct(A_SUSTAIN, "A sustain", "Envelopes", 100.0),
    time(A_RELEASE, "A release", 300.0),
    // How much softer notes play quieter.
    pct(VEL_AMP, "Vel > amp", "Envelopes", 50.0),
    p(LFO_RATE, "LFO rate", "Mod", 0.05, 30.0, 5.0, Unit::Hz, Log),
    choice(LFO_WAVE, "LFO wave", "Mod", LFO_WAVES, 1.0),
    // The LFO fades in over this long after a note played with no other
    // key held.
    p(
        LFO_DELAY,
        "LFO delay",
        "Mod",
        0.0,
        5000.0,
        0.0,
        Unit::Ms,
        Linear,
    ),
    p(
        LFO_PITCH,
        "Vibrato",
        "Mod",
        0.0,
        100.0,
        0.0,
        Unit::Cents,
        Linear,
    ),
    // The LFO on both pulse widths: 100 swings them 45 % each way.
    pct(LFO_PW, "LFO > PW", "Mod", 0.0),
    // The LFO on the cutoff: 100 is 2 octaves each way.
    pct(LFO_CUTOFF, "LFO > cutoff", "Mod", 0.0),
    // Poly-mod: each voice's filter envelope (bipolar) and oscillator 2, to
    // oscillator 1's pitch, its pulse width and the cutoff, as switched.
    p(
        PM_ENV,
        "PMod env",
        "Mod",
        -100.0,
        100.0,
        0.0,
        Unit::Percent,
        Linear,
    ),
    pct(PM_OSC2, "PMod osc 2", "Mod", 0.0),
    toggle(PM_PITCH, "PMod > pitch", "Mod", 1.0),
    toggle(PM_PW, "PMod > PW", "Mod", 0.0),
    toggle(PM_CUTOFF, "PMod > cutoff", "Mod", 0.0),
    choice(VOICE_MODE, "Mode", "Voice", VOICE_MODES, POLY),
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
    // The spread of the voices in Unison: 100 is 25 cents each side.
    pct(UNI_DETUNE, "Detune", "Voice", 30.0),
    // Each voice glides from its last note (three time constants).
    p(GLIDE, "Glide", "Voice", 0.0, 2000.0, 0.0, Unit::Ms, Linear),
    // How far the voices differ and drift: 0 is a perfect instrument,
    // 100 a tired one.
    pct(AGE, "Age", "Voice", 30.0),
    choice(CHORUS, "Chorus", "Voice", CHORUSES, CHORUS_OFF),
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
