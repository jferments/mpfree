//! The electric piano's parameters; its factory presets are in
//! `presets/keys.presets` ([`crate::factory::read`]).

use mpfree_node::{Curve, ParamInfo, Unit};

macro_rules! ids {
    ($($name:ident = $v:expr),* $(,)?) => { $(pub const $name: u16 = $v;)* };
}

ids! {
    MODEL = 0, DECAY = 1, RELEASE = 2,
    PICKUP = 3, CLAV_TONE = 4, MUTE = 5, DROP = 6,
    HARDNESS = 10, BELL = 11, VOICING = 12, BARK = 13, TONE = 14, VEL_TONE = 15,
    VELOCITY = 16,
    TREMOLO = 20, TREM_RATE = 21, TREM_MODE = 22, DRIVE = 23, KEY_NOISE = 24,
    TRANSPOSE = 30, FINE = 31, POLYPHONY = 32, BEND_RANGE = 33,
    LEVEL = 40, PAN = 41,
    METER_VOICES = 50,
}

/// The highest id plus one: values live in an array indexed by id.
pub const ID_SPACE: usize = 51;

/// Tine: a struck steel tine beside a magnetic pickup, round and bell-like,
/// barking when hit hard. Reed: a struck steel reed in an electrostatic
/// pickup, reedy and quick to buzz. Clav: a string struck and held by a
/// tangent, over two magnetic pickups, bright and percussive, dropping in
/// pitch as the yarn damps it.
pub const MODELS: &[&str] = &["Tine", "Reed", "Clav"];
pub const TINE: f32 = 0.0;
pub const REED: f32 = 1.0;
pub const CLAV: f32 = 2.0;
/// The clav's pickups: the one nearer the middle of the string (round),
/// the one by the bridge (thin and bright), both, or both with one turned
/// round (thin and nasal: what they share cancels).
pub const PICKUPS: &[&str] = &["Neck", "Bridge", "Both", "Out of phase"];
pub const NECK: f32 = 0.0;
pub const BRIDGE: f32 = 1.0;
pub const BOTH: f32 = 2.0;
pub const OUT_OF_PHASE: f32 = 3.0;
/// The clav's tone switches, darkest to brightest.
pub const CLAV_TONES: &[&str] = &["Soft", "Medium", "Treble", "Brilliant"];
pub const SOFT: f32 = 0.0;
pub const MEDIUM: f32 = 1.0;
pub const TREBLE: f32 = 2.0;
pub const BRILLIANT: f32 = 3.0;
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
    // shorter, lower ones longer. 12 s by default: tine pianos measured
    // 11 s and 19 s (the reed and clav presets set their own).
    p(
        DECAY,
        "Decay",
        "Model",
        500.0,
        20000.0,
        12000.0,
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
    // How much quieter the softest note is than the hardest (a square
    // law, as the voice core's).
    p(
        VELOCITY,
        "Velocity",
        "Tone",
        0.0,
        48.0,
        32.0,
        Unit::Db,
        Linear,
    ),
    choice(PICKUP, "Pickup", "Clav", PICKUPS, BOTH),
    choice(CLAV_TONE, "Clav tone", "Clav", CLAV_TONES, MEDIUM),
    // A hand or the mute bar on the strings: darker, and shorter (at 100,
    // a held note rings for 15 % of Decay).
    pct(MUTE, "Mute", "Clav", 0.0),
    // How far a clav note falls as its key comes up and the yarn damps
    // the string.
    p(
        DROP,
        "Release drop",
        "Clav",
        0.0,
        6.0,
        3.0,
        Unit::Semitones,
        Linear,
    ),
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
