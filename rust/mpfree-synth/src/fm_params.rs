//! The FM synth's parameters. Its factory presets are a file,
//! `presets/fm.presets` ([`crate::factory`]).
//!
//! Ranges follow the DX7 (0 to 99 for rates and levels, 0 to 7 for
//! sensitivities), so a DX7 voice maps onto them one to one ([`crate::dx7`])
//! and players can use the numbers they know.

use mpfree_node::{pressure as np, Curve, ParamInfo, Unit};

use crate::pressure::FM_TARGETS;

macro_rules! ids {
    ($($name:ident = $v:expr),* $(,)?) => { $(pub const $name: u16 = $v;)* };
}

ids! {
    ALGORITHM = 0, FEEDBACK = 1, OSC_SYNC = 2, TRANSPOSE = 3,
    PEG_R1 = 4, PEG_R2 = 5, PEG_R3 = 6, PEG_R4 = 7, PEG_L1 = 8, PEG_L2 = 9, PEG_L3 = 10, PEG_L4 = 11,
    LFO_WAVE = 12, LFO_SPEED = 13, LFO_DELAY = 14, LFO_PMD = 15, LFO_AMD = 16, LFO_SYNC = 17, LFO_PMS = 18,
    POLYPHONY = 19, VOICE_MODE = 20, BEND_RANGE = 21, LEVEL = 23,
    METER_VOICES = 30,
}

/// The first pressure parameter, after the six operators' blocks. Id 22 (the
/// old fixed pressure-to-vibrato amount) is retired: slot 1 on Vibrato does
/// its job.
pub const PRESS_BASE: u16 = 32 * 7;

/// Pressure settings and slots: [`mpfree_node::pressure::field`] offsets
/// from [`PRESS_BASE`].
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
/// Slot `n` (0 to 3): its target ([`FM_TARGETS`]) and depth.
pub const fn press_target(n: usize) -> u16 {
    press(np::field::target(n))
}
pub const fn press_depth(n: usize) -> u16 {
    press(np::field::depth(n))
}

/// FM's pressure targets, by index into [`FM_TARGETS`].
pub mod target {
    pub const OFF: u8 = 0;
    pub const BRIGHTNESS: u8 = 1;
    pub const LEVEL: u8 = 2;
    pub const VIBRATO: u8 = 3;
    pub const LFO_RATE: u8 = 4;
    pub const TREMOLO: u8 = 5;
    pub const PITCH: u8 = 6;
    pub const FEEDBACK: u8 = 7;
}

/// Fields of one operator, added to [`op_id`]'s base.
pub mod op {
    pub const R1: u16 = 0;
    pub const R2: u16 = 1;
    pub const R3: u16 = 2;
    pub const R4: u16 = 3;
    pub const L1: u16 = 4;
    pub const L2: u16 = 5;
    pub const L3: u16 = 6;
    pub const L4: u16 = 7;
    pub const LEVEL: u16 = 8;
    pub const MODE: u16 = 9;
    pub const COARSE: u16 = 10;
    pub const FINE: u16 = 11;
    pub const DETUNE: u16 = 12;
    pub const VEL_SENS: u16 = 13;
    pub const AMS: u16 = 14;
    pub const RATE_SCALE: u16 = 15;
    pub const BREAK: u16 = 16;
    pub const L_DEPTH: u16 = 17;
    pub const R_DEPTH: u16 = 18;
    pub const L_CURVE: u16 = 19;
    pub const R_CURVE: u16 = 20;
    pub const ON: u16 = 21;
    pub const WAVE: u16 = 22;
    /// Fields per operator.
    pub const FIELDS: usize = 23;
}

/// The id of field `field` of operator `n` (1 to 6, as on the DX7).
pub const fn op_id(n: usize, field: u16) -> u16 {
    (32 * n) as u16 + field
}

/// The highest id plus one: values live in an array indexed by id.
pub const ID_SPACE: usize = PRESS_BASE as usize + np::FIELDS;

pub const VOICE_MODES: &[&str] = &["Poly", "Mono"];
pub const LFO_WAVES: &[&str] = &["Triangle", "Saw down", "Saw up", "Square", "Sine", "S&H"];
pub const OP_MODES: &[&str] = &["Ratio", "Fixed"];
pub const CURVES: &[&str] = &["-Lin", "-Exp", "+Exp", "+Lin"];
/// An operator's wave ([`crate::fm`]): the sine, as on the DX7, or one of
/// four more, each as loud as the sine.
pub const OP_WAVES: &[&str] = &["Sine", "Triangle", "Saw", "Square", "Half sine"];

const fn p(
    id: u16,
    name: &'static str,
    group: &'static str,
    min: f32,
    max: f32,
    default: f32,
    curve: Curve,
) -> ParamInfo {
    ParamInfo {
        id,
        name,
        group,
        min,
        max,
        default,
        unit: Unit::None,
        curve,
        readonly: false,
    }
}

use Curve::{Choice, Stepped, Toggle};

const GLOBALS: [ParamInfo; 24] = [
    p(ALGORITHM, "Algorithm", "Algo", 1.0, 32.0, 1.0, Stepped),
    p(FEEDBACK, "Feedback", "Algo", 0.0, 7.0, 0.0, Stepped),
    p(OSC_SYNC, "Osc key sync", "Algo", 0.0, 1.0, 1.0, Toggle),
    ParamInfo {
        id: TRANSPOSE,
        name: "Transpose",
        group: "Algo",
        min: -24.0,
        max: 24.0,
        default: 0.0,
        unit: Unit::Semitones,
        curve: Stepped,
        readonly: false,
    },
    p(PEG_R1, "Rate 1", "Pitch env", 0.0, 99.0, 99.0, Stepped),
    p(PEG_R2, "Rate 2", "Pitch env", 0.0, 99.0, 99.0, Stepped),
    p(PEG_R3, "Rate 3", "Pitch env", 0.0, 99.0, 99.0, Stepped),
    p(PEG_R4, "Rate 4", "Pitch env", 0.0, 99.0, 99.0, Stepped),
    p(PEG_L1, "Level 1", "Pitch env", 0.0, 99.0, 50.0, Stepped),
    p(PEG_L2, "Level 2", "Pitch env", 0.0, 99.0, 50.0, Stepped),
    p(PEG_L3, "Level 3", "Pitch env", 0.0, 99.0, 50.0, Stepped),
    p(PEG_L4, "Level 4", "Pitch env", 0.0, 99.0, 50.0, Stepped),
    p(LFO_WAVE, "Wave", "LFO", 0.0, 5.0, 0.0, Choice(LFO_WAVES)),
    p(LFO_SPEED, "Speed", "LFO", 0.0, 99.0, 35.0, Stepped),
    p(LFO_DELAY, "Delay", "LFO", 0.0, 99.0, 0.0, Stepped),
    p(LFO_PMD, "Pitch depth", "LFO", 0.0, 99.0, 0.0, Stepped),
    p(LFO_AMD, "Amp depth", "LFO", 0.0, 99.0, 0.0, Stepped),
    p(LFO_SYNC, "Key sync", "LFO", 0.0, 1.0, 1.0, Toggle),
    p(LFO_PMS, "Pitch sens", "LFO", 0.0, 7.0, 3.0, Stepped),
    p(POLYPHONY, "Voices", "Voice", 1.0, 16.0, 8.0, Stepped),
    p(
        VOICE_MODE,
        "Mode",
        "Voice",
        0.0,
        1.0,
        0.0,
        Choice(VOICE_MODES),
    ),
    ParamInfo {
        id: BEND_RANGE,
        name: "Bend range",
        group: "Voice",
        min: 0.0,
        max: 12.0,
        default: 2.0,
        unit: Unit::Semitones,
        curve: Stepped,
        readonly: false,
    },
    // -9 dB: one full-level sine (Init, and a DX7 voice's loudest carrier)
    // plays about as loud as the other synths' Init sounds, and an 8-note
    // chord of it at full velocity stays under 0 dBFS.
    ParamInfo {
        id: LEVEL,
        name: "Level",
        group: "Output",
        min: -60.0,
        max: 6.0,
        default: -9.0,
        unit: Unit::Db,
        curve: Curve::Linear,
        readonly: false,
    },
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

/// One operator's fields; `id` is the field, `group` is filled in per operator.
const OP_FIELDS: [ParamInfo; op::FIELDS] = [
    p(op::R1, "Rate 1", "", 0.0, 99.0, 99.0, Stepped),
    p(op::R2, "Rate 2", "", 0.0, 99.0, 99.0, Stepped),
    p(op::R3, "Rate 3", "", 0.0, 99.0, 99.0, Stepped),
    p(op::R4, "Rate 4", "", 0.0, 99.0, 99.0, Stepped),
    p(op::L1, "Level 1", "", 0.0, 99.0, 99.0, Stepped),
    p(op::L2, "Level 2", "", 0.0, 99.0, 99.0, Stepped),
    p(op::L3, "Level 3", "", 0.0, 99.0, 99.0, Stepped),
    p(op::L4, "Level 4", "", 0.0, 99.0, 0.0, Stepped),
    p(op::LEVEL, "Output", "", 0.0, 99.0, 0.0, Stepped),
    p(op::MODE, "Mode", "", 0.0, 1.0, 0.0, Choice(OP_MODES)),
    p(op::COARSE, "Coarse", "", 0.0, 31.0, 1.0, Stepped),
    p(op::FINE, "Fine", "", 0.0, 99.0, 0.0, Stepped),
    p(op::DETUNE, "Detune", "", -7.0, 7.0, 0.0, Stepped),
    p(op::VEL_SENS, "Velocity", "", 0.0, 7.0, 0.0, Stepped),
    p(op::AMS, "Amp mod", "", 0.0, 3.0, 0.0, Stepped),
    p(op::RATE_SCALE, "Rate", "", 0.0, 7.0, 0.0, Stepped),
    p(op::BREAK, "Break", "", 0.0, 99.0, 39.0, Stepped),
    p(op::L_DEPTH, "L depth", "", 0.0, 99.0, 0.0, Stepped),
    p(op::R_DEPTH, "R depth", "", 0.0, 99.0, 0.0, Stepped),
    p(op::L_CURVE, "L curve", "", 0.0, 3.0, 0.0, Choice(CURVES)),
    p(op::R_CURVE, "R curve", "", 0.0, 3.0, 0.0, Choice(CURVES)),
    p(op::ON, "On", "", 0.0, 1.0, 1.0, Toggle),
    p(op::WAVE, "Wave", "", 0.0, 4.0, 0.0, Choice(OP_WAVES)),
];

/// Each operator's sound (frequency, level, envelope) and its keyboard
/// scaling are separate groups, so each fits a page.
const OP_GROUPS: [&str; 6] = ["Op 1", "Op 2", "Op 3", "Op 4", "Op 5", "Op 6"];
const OP_KBD_GROUPS: [&str; 6] = [
    "Op 1 kbd", "Op 2 kbd", "Op 3 kbd", "Op 4 kbd", "Op 5 kbd", "Op 6 kbd",
];

/// Whether an operator field is keyboard scaling.
const fn is_kbd(field: u16) -> bool {
    field >= op::RATE_SCALE && field <= op::R_CURVE
}

/// Pressure: the shaper's settings, then four slots. Slot 1 adds a little
/// vibrato by default (0.18 semitones), as the old fixed setting did.
const PRESS: [ParamInfo; np::FIELDS] = np::params(
    PRESS_BASE,
    "Pressure",
    FM_TARGETS,
    [
        (target::VIBRATO as f32, 18.0),
        (0.0, 0.0),
        (0.0, 0.0),
        (0.0, 0.0),
    ],
);

pub const N_PARAMS: usize = GLOBALS.len() + 6 * op::FIELDS + np::FIELDS;

const fn build() -> [ParamInfo; N_PARAMS] {
    let mut out = [GLOBALS[0]; N_PARAMS];
    let mut i = 0;
    while i < GLOBALS.len() {
        out[i] = GLOBALS[i];
        i += 1;
    }
    let mut n = 0;
    while n < 6 {
        let mut f = 0;
        while f < op::FIELDS {
            let t = OP_FIELDS[f];
            let mut q = t;
            q.id = op_id(n + 1, t.id);
            q.group = if is_kbd(t.id) {
                OP_KBD_GROUPS[n]
            } else {
                OP_GROUPS[n]
            };
            // The init voice: only operator 1 sounds, at full level.
            if n == 0 && t.id == op::LEVEL {
                q.default = 99.0;
            }
            out[GLOBALS.len() + n * op::FIELDS + f] = q;
            f += 1;
        }
        n += 1;
    }
    let mut k = 0;
    while k < np::FIELDS {
        out[GLOBALS.len() + 6 * op::FIELDS + k] = PRESS[k];
        k += 1;
    }
    out
}

pub static PARAMS: [ParamInfo; N_PARAMS] = build();

pub fn info(id: u16) -> Option<&'static ParamInfo> {
    PARAMS.iter().find(|p| p.id == id)
}
