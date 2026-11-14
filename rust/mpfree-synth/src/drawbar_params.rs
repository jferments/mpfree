//! The tonewheel organ's parameters and factory presets.

use mpfree_node::{Curve, ParamInfo, Unit};

use crate::params::Preset;

macro_rules! ids {
    ($($name:ident = $v:expr),* $(,)?) => { $(pub const $name: u16 = $v;)* };
}

ids! {
    DB_16 = 0, DB_5_13 = 1, DB_8 = 2, DB_4 = 3, DB_2_23 = 4, DB_2 = 5, DB_1_35 = 6, DB_1_13 = 7, DB_1 = 8,
    PERC = 10, PERC_HARM = 11, PERC_DECAY = 12, PERC_VOL = 13,
    VIBRATO = 20, SCANNER = 21,
    CLICK = 30, LEAKAGE = 31, DRIVE = 32, TONE = 33, WHEELS = 34,
    ROTARY = 40, SPEED = 41, SPREAD = 42,
    TRANSPOSE = 50, FINE = 51,
    LEVEL = 60, PAN = 61,
    METER_KEYS = 70,
}

/// The highest id plus one: values live in an array indexed by id.
pub const ID_SPACE: usize = 71;

/// The drawbars in order, as on the organ: their ids, names and
/// semitones from the note.
pub const DRAWBARS: [(u16, &str, i32); 9] = [
    (DB_16, "16'", -12),
    (DB_5_13, "5 1/3'", 7),
    (DB_8, "8'", 0),
    (DB_4, "4'", 12),
    (DB_2_23, "2 2/3'", 19),
    (DB_2, "2'", 24),
    (DB_1_35, "1 3/5'", 28),
    (DB_1_13, "1 1/3'", 31),
    (DB_1, "1'", 36),
];

pub const ON_OFF: &[&str] = &["Off", "On"];
/// The percussion's harmonic: the octave (4') or the twelfth (2 2/3').
pub const PERC_HARMS: &[&str] = &["Second", "Third"];
pub const PERC_DECAYS: &[&str] = &["Fast", "Slow"];
pub const PERC_VOLS: &[&str] = &["Normal", "Soft"];
/// The scanner: off, three vibrato depths, the same three as chorus.
pub const VIBRATOS: &[&str] = &["Off", "V1", "V2", "V3", "C1", "C2", "C3"];
/// Wheels tuned by the organ's gear ratios (up to 0.7 cents off equal
/// temperament below the top octave, 1.9 cents sharp in it), carried onto
/// the project's tuning, or exactly to the project's tuning.
pub const WHEEL_TUNINGS: &[&str] = &["Gears", "Exact"];
/// The rotary speaker's motors: stopped (the rotors coast to a stop),
/// slow (chorale) or fast (tremolo).
pub const SPEEDS: &[&str] = &["Stop", "Slow", "Fast"];

pub const SECOND: f32 = 0.0;
pub const THIRD: f32 = 1.0;
pub const FAST: f32 = 0.0;
pub const SLOW: f32 = 1.0;
pub const SOFT: f32 = 1.0;
pub const STOP: f32 = 0.0;
pub const SPEED_SLOW: f32 = 1.0;
pub const SPEED_FAST: f32 = 2.0;
pub const V1: f32 = 1.0;
pub const V2: f32 = 2.0;
pub const V3: f32 = 3.0;
pub const C1: f32 = 4.0;
pub const C2: f32 = 5.0;
pub const C3: f32 = 6.0;

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

const fn bar(id: u16, name: &'static str, default: f32) -> ParamInfo {
    p(
        id,
        name,
        "Drawbars",
        0.0,
        8.0,
        default,
        Unit::None,
        Curve::Stepped,
    )
}

use Curve::{Linear, Stepped};

pub static PARAMS: &[ParamInfo] = &[
    bar(DB_16, "16'", 8.0),
    bar(DB_5_13, "5 1/3'", 8.0),
    bar(DB_8, "8'", 8.0),
    bar(DB_4, "4'", 0.0),
    bar(DB_2_23, "2 2/3'", 0.0),
    bar(DB_2, "2'", 0.0),
    bar(DB_1_35, "1 3/5'", 0.0),
    bar(DB_1_13, "1 1/3'", 0.0),
    bar(DB_1, "1'", 0.0),
    choice(PERC, "Percussion", "Percussion", ON_OFF, 0.0),
    choice(PERC_HARM, "Perc harm", "Percussion", PERC_HARMS, 1.0),
    choice(PERC_DECAY, "Perc decay", "Percussion", PERC_DECAYS, FAST),
    choice(PERC_VOL, "Perc volume", "Percussion", PERC_VOLS, 0.0),
    choice(VIBRATO, "Vibrato", "Vibrato", VIBRATOS, 0.0),
    // The scanner's rate: about 7 Hz on the organ, fixed by its motor.
    p(
        SCANNER,
        "Scanner",
        "Vibrato",
        4.0,
        10.0,
        6.9,
        Unit::Hz,
        Linear,
    ),
    // How unevenly the key contacts close and bounce: 0 is a smooth
    // start, 100 a hard spit.
    pct(CLICK, "Click", "Tone", 50.0),
    // Each wheel heard faintly in its compartment partner, four octaves
    // away: off at 0, -40 dB at 40, -26 dB at 100.
    pct(LEAKAGE, "Leakage", "Tone", 40.0),
    pct(DRIVE, "Drive", "Tone", 10.0),
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
    choice(WHEELS, "Wheels", "Tone", WHEEL_TUNINGS, 0.0),
    choice(ROTARY, "Rotary", "Rotary", ON_OFF, 1.0),
    choice(SPEED, "Speed", "Rotary", SPEEDS, 1.0),
    // How far apart the two microphones stand around the cabinet: 0 is
    // one point (mono), 100 opposite sides.
    pct(SPREAD, "Spread", "Rotary", 70.0),
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

/// Drawbar settings as the organ's players write them: nine digits,
/// 16' first ("888000000").
const fn bars(s: &[u8; 9]) -> [(u16, f32); 9] {
    let mut out = [(0u16, 0.0f32); 9];
    let mut i = 0;
    while i < 9 {
        out[i] = (DRAWBARS[i].0, (s[i] - b'0') as f32);
        i += 1;
    }
    out
}

const INIT: [(u16, f32); 9] = bars(b"888000000");
const GOSPEL: [(u16, f32); 9] = bars(b"888800000");
const JAZZ: [(u16, f32); 9] = bars(b"888000000");
const FULL: [(u16, f32); 9] = bars(b"888888888");
const BALLAD: [(u16, f32); 9] = bars(b"008800000");
const FLUTE: [(u16, f32); 9] = bars(b"008000000");
const ROCK: [(u16, f32); 9] = bars(b"888600000");
const CHURCH: [(u16, f32); 9] = bars(b"868868446");
const REED: [(u16, f32); 9] = bars(b"008080800");
const BRIGHT: [(u16, f32); 9] = bars(b"800000888");
const HOLLOW: [(u16, f32); 9] = bars(b"808000000");
const SOFT_BASS: [(u16, f32); 9] = bars(b"850000000");

macro_rules! preset {
    ($name:expr, $bars:expr, [$($extra:expr),* $(,)?]) => {
        Preset {
            name: $name,
            values: &[
                $bars[0], $bars[1], $bars[2], $bars[3], $bars[4], $bars[5], $bars[6], $bars[7], $bars[8],
                $($extra),*
            ],
        }
    };
}

pub static PRESETS: &[Preset] = &[
    preset!("Init", INIT, []),
    preset!(
        "Jazz Perc",
        JAZZ,
        [
            (PERC, 1.0),
            (PERC_HARM, THIRD),
            (PERC_DECAY, FAST),
            (VIBRATO, C3),
            (CLICK, 60.0),
            (LEVEL, 2.0),
        ]
    ),
    preset!(
        "Gospel",
        GOSPEL,
        [
            (VIBRATO, C3),
            (SPEED, SPEED_FAST),
            (DRIVE, 25.0),
            (CLICK, 55.0),
            (LEVEL, -1.0)
        ]
    ),
    preset!(
        "Full Organ",
        FULL,
        [(DRIVE, 20.0), (LEVEL, -6.0), (VIBRATO, C1)]
    ),
    preset!(
        "Rock Drive",
        ROCK,
        [
            (DRIVE, 70.0),
            (CLICK, 75.0),
            (VIBRATO, C3),
            (TONE, 15.0),
            (LEVEL, -8.0)
        ]
    ),
    preset!(
        "Ballad",
        BALLAD,
        [(CLICK, 25.0), (VIBRATO, V3), (TONE, -20.0), (LEVEL, 3.0)]
    ),
    preset!(
        "Flute",
        FLUTE,
        [(CLICK, 15.0), (VIBRATO, V1), (TONE, -30.0), (LEVEL, 5.0)]
    ),
    preset!(
        "Church",
        CHURCH,
        [(ROTARY, 0.0), (CLICK, 10.0), (VIBRATO, C1), (LEVEL, -5.0)]
    ),
    preset!("Reedy", REED, [(CLICK, 40.0), (VIBRATO, C2), (LEVEL, 2.0)]),
    preset!(
        "Bright Perc",
        BRIGHT,
        [
            (PERC, 1.0),
            (PERC_HARM, SECOND),
            (PERC_DECAY, FAST),
            (CLICK, 70.0),
            (LEVEL, 1.0),
        ]
    ),
    preset!(
        "Hollow",
        HOLLOW,
        [(VIBRATO, V2), (SPEED, STOP), (CLICK, 30.0), (LEVEL, 2.0)]
    ),
    preset!(
        "Soft Bass",
        SOFT_BASS,
        [
            (ROTARY, 0.0),
            (CLICK, 35.0),
            (TONE, -40.0),
            (TRANSPOSE, -12.0),
            (LEVEL, 3.0)
        ]
    ),
];
