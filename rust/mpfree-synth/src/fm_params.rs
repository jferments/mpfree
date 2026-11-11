//! The FM synth's parameters and factory presets.
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
pub const fn press(field: u16) -> u16 { PRESS_BASE + field }
pub const PRESS_MODE: u16 = press(np::field::MODE);
pub const PRESS_HOLDOFF: u16 = press(np::field::HOLDOFF);
pub const PRESS_THRESHOLD: u16 = press(np::field::THRESHOLD);
pub const PRESS_FULL_AT: u16 = press(np::field::FULL_AT);
pub const PRESS_CURVE: u16 = press(np::field::CURVE);
pub const PRESS_RISE: u16 = press(np::field::RISE);
pub const PRESS_FALL: u16 = press(np::field::FALL);
pub const PRESS_RELEASE: u16 = press(np::field::RELEASE);
/// Slot `n` (0 to 3): its target ([`FM_TARGETS`]) and depth.
pub const fn press_target(n: usize) -> u16 { press(np::field::target(n)) }
pub const fn press_depth(n: usize) -> u16 { press(np::field::depth(n)) }

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
    /// Fields per operator.
    pub const FIELDS: usize = 22;
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

const fn p(id: u16, name: &'static str, group: &'static str, min: f32, max: f32, default: f32, curve: Curve) -> ParamInfo {
    ParamInfo { id, name, group, min, max, default, unit: Unit::None, curve, readonly: false }
}

use Curve::{Choice, Stepped, Toggle};

const GLOBALS: [ParamInfo; 24] = [
    p(ALGORITHM, "Algorithm", "Algo", 1.0, 32.0, 1.0, Stepped),
    p(FEEDBACK, "Feedback", "Algo", 0.0, 7.0, 0.0, Stepped),
    p(OSC_SYNC, "Osc key sync", "Algo", 0.0, 1.0, 1.0, Toggle),
    ParamInfo { id: TRANSPOSE, name: "Transpose", group: "Algo", min: -24.0, max: 24.0, default: 0.0, unit: Unit::Semitones, curve: Stepped, readonly: false },
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
    p(VOICE_MODE, "Mode", "Voice", 0.0, 1.0, 0.0, Choice(VOICE_MODES)),
    ParamInfo { id: BEND_RANGE, name: "Bend range", group: "Voice", min: 0.0, max: 12.0, default: 2.0, unit: Unit::Semitones, curve: Stepped, readonly: false },
    ParamInfo { id: LEVEL, name: "Level", group: "Output", min: -60.0, max: 6.0, default: 0.0, unit: Unit::Db, curve: Curve::Linear, readonly: false },
    ParamInfo { id: METER_VOICES, name: "Voices on", group: "Output", min: 0.0, max: 32.0, default: 0.0, unit: Unit::None, curve: Stepped, readonly: true },
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
];

/// Each operator's sound (frequency, level, envelope) and its keyboard
/// scaling are separate groups, so each fits a page.
const OP_GROUPS: [&str; 6] = ["Op 1", "Op 2", "Op 3", "Op 4", "Op 5", "Op 6"];
const OP_KBD_GROUPS: [&str; 6] = ["Op 1 kbd", "Op 2 kbd", "Op 3 kbd", "Op 4 kbd", "Op 5 kbd", "Op 6 kbd"];

/// Whether an operator field is keyboard scaling.
const fn is_kbd(field: u16) -> bool {
    field >= op::RATE_SCALE && field <= op::R_CURVE
}

/// Pressure: the shaper's settings, then four slots. Slot 1 adds a little
/// vibrato by default (0.18 semitones), as the old fixed setting did.
const PRESS: [ParamInfo; np::FIELDS] = np::params(PRESS_BASE, "Pressure", FM_TARGETS, [(target::VIBRATO as f32, 18.0), (0.0, 0.0), (0.0, 0.0), (0.0, 0.0)]);

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
            q.group = if is_kbd(t.id) { OP_KBD_GROUPS[n] } else { OP_GROUPS[n] };
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

/// An FM preset: a name and the values that differ from the defaults.
#[derive(Clone, Debug, PartialEq)]
pub struct FmPreset {
    pub name: String,
    pub values: Vec<(u16, f32)>,
}

/// One operator's settings in a preset, in DX7 order.
#[derive(Clone, Copy, Debug)]
struct Op {
    rates: [u8; 4],
    levels: [u8; 4],
    out: u8,
    coarse: u8,
    fine: u8,
    detune: i8,
    vel: u8,
    rs: u8,
}

#[allow(clippy::too_many_arguments)]
const fn o(rates: [u8; 4], levels: [u8; 4], out: u8, coarse: u8, fine: u8, detune: i8, vel: u8, rs: u8) -> Op {
    Op { rates, levels, out, coarse, fine, detune, vel, rs }
}

/// An unused operator.
const OFF: Op = o([99, 99, 99, 99], [99, 99, 99, 0], 0, 1, 0, 0, 0, 0);

struct Def {
    name: &'static str,
    alg: u8,
    fb: u8,
    ops: [Op; 6],
    extra: &'static [(u16, f32)],
}

/// MPFree's own FM voices. They are written for MPFree, not copied from
/// any factory bank; DX7 banks a player owns load through [`crate::dx7`].
const DEFS: &[Def] = &[
    Def { name: "Init", alg: 1, fb: 0, ops: [o([99, 99, 99, 99], [99, 99, 99, 0], 99, 1, 0, 0, 0, 0), OFF, OFF, OFF, OFF, OFF], extra: &[] },
    // Three pairs: a soft body, a bell-like tine (ratio 14, decaying fast)
    // and a slightly detuned second body for width.
    Def { name: "Glass Keys", alg: 5, fb: 4, ops: [
        o([96, 25, 25, 67], [99, 75, 0, 0], 99, 1, 0, 2, 2, 3),
        o([95, 50, 35, 78], [99, 75, 0, 0], 58, 14, 0, 0, 7, 3),
        o([95, 20, 20, 50], [99, 95, 0, 0], 95, 1, 0, -2, 2, 3),
        o([95, 29, 20, 50], [99, 95, 0, 0], 72, 1, 0, 0, 6, 3),
        o([95, 20, 20, 50], [99, 95, 0, 0], 86, 1, 0, 3, 2, 3),
        o([95, 29, 20, 50], [99, 95, 0, 0], 70, 1, 0, -3, 6, 3),
    ], extra: &[(LEVEL, -4.0), (press_depth(0), 20.0)] },
    // Inharmonic modulators (3.5 and 1.41) over long decays.
    Def { name: "Bell Tower", alg: 5, fb: 0, ops: [
        o([99, 18, 15, 40], [99, 70, 0, 0], 99, 1, 0, 0, 2, 2),
        o([99, 20, 15, 40], [99, 60, 0, 0], 78, 3, 50, 1, 3, 2),
        o([99, 22, 15, 40], [99, 65, 0, 0], 90, 2, 0, -1, 2, 2),
        o([99, 25, 15, 40], [99, 55, 0, 0], 70, 1, 41, 0, 3, 2),
        o([99, 15, 10, 40], [99, 60, 0, 0], 82, 0, 0, 2, 2, 2),
        o([99, 30, 20, 40], [99, 50, 0, 0], 60, 7, 0, 0, 2, 2),
    ], extra: &[(LEVEL, -3.0), (press_depth(0), 20.0)] },
    // A punchy bass: the modulator's quick decay gives the pluck, op 3 is a
    // sine an octave down under it.
    Def { name: "Rubber Bass", alg: 3, fb: 6, ops: [
        o([99, 50, 30, 70], [99, 90, 80, 0], 99, 1, 0, 0, 2, 0),
        o([99, 70, 40, 70], [99, 60, 40, 0], 82, 1, 0, 0, 5, 0),
        o([99, 85, 50, 70], [99, 70, 30, 0], 70, 1, 0, 0, 3, 0),
        o([99, 40, 30, 70], [99, 90, 85, 0], 92, 0, 0, 0, 2, 0),
        OFF, OFF,
    // FM has no drive, so pressure brightens it instead (8 steps).
    ], extra: &[(VOICE_MODE, 1.0), (LEVEL, 2.0), (press_target(0), target::BRIGHTNESS as f32), (press_depth(0), 33.0)] },
    // Drawbar-style organ: six carriers at 0.5, 1, 2, 3, 4 and 8.
    Def { name: "Drawbars", alg: 32, fb: 3, ops: [
        o([99, 99, 99, 85], [99, 99, 99, 0], 92, 1, 0, 0, 0, 0),
        o([99, 99, 99, 85], [99, 99, 99, 0], 88, 0, 0, 1, 0, 0),
        o([99, 99, 99, 85], [99, 99, 99, 0], 84, 2, 0, -1, 0, 0),
        o([99, 99, 99, 85], [99, 99, 99, 0], 76, 3, 0, 0, 0, 0),
        o([99, 99, 99, 85], [99, 99, 99, 0], 74, 4, 0, 1, 0, 0),
        o([99, 99, 99, 85], [99, 99, 99, 0], 68, 8, 0, 0, 0, 0),
    // Pressure doubles the LFO's speed, like a rotary speaker speeding up.
    ], extra: &[(LFO_PMD, 6.0), (LFO_SPEED, 33.0), (LFO_WAVE, 4.0), (LEVEL, 2.0), (press_target(0), target::LFO_RATE as f32), (press_depth(0), 50.0)] },
    // Brass: the modulator swells a little behind the carrier, so the tone
    // brightens as the note starts; a touch of pitch envelope bite.
    Def { name: "Brass Section", alg: 22, fb: 7, ops: [
        o([72, 76, 99, 71], [99, 88, 96, 0], 98, 1, 0, 0, 1, 2),
        o([62, 51, 29, 71], [82, 95, 96, 0], 80, 1, 0, 0, 3, 2),
        o([72, 76, 99, 71], [99, 88, 96, 0], 96, 1, 0, 3, 1, 2),
        o([72, 76, 99, 71], [99, 88, 96, 0], 94, 1, 0, -3, 1, 2),
        o([72, 76, 99, 71], [99, 88, 96, 0], 90, 1, 0, 1, 1, 2),
        o([62, 51, 29, 71], [82, 95, 96, 0], 76, 1, 0, 0, 3, 2),
    ], extra: &[(PEG_R1, 84.0), (PEG_R2, 95.0), (PEG_L1, 47.0), (PEG_L2, 50.0), (PEG_L3, 50.0), (PEG_L4, 50.0),
                (LFO_PMD, 4.0), (LFO_DELAY, 40.0), (LEVEL, -8.0),
                // Pressure swells it 9 dB and brightens it 12 steps.
                (press_target(0), target::LEVEL as f32), (press_depth(0), 37.5), (press_target(1), target::BRIGHTNESS as f32), (press_depth(1), 50.0)] },
    // Mallets: a short, woody hit with a ratio-4 modulator.
    Def { name: "Log Drum", alg: 5, fb: 0, ops: [
        o([99, 45, 30, 50], [99, 0, 0, 0], 99, 1, 0, 0, 3, 4),
        o([99, 70, 40, 50], [99, 0, 0, 0], 74, 4, 0, 0, 5, 4),
        o([99, 40, 30, 50], [99, 0, 0, 0], 90, 1, 0, 2, 3, 4),
        o([99, 65, 40, 50], [99, 0, 0, 0], 66, 3, 0, 0, 5, 4),
        OFF, OFF,
    ], extra: &[(LEVEL, -3.0), (press_target(0), target::OFF as f32)] },
    // A slow, chorused pad: detuned carriers, gentle modulators and vibrato.
    Def { name: "Silk Pad", alg: 5, fb: 2, ops: [
        o([40, 30, 25, 35], [99, 95, 90, 0], 92, 1, 0, 4, 1, 0),
        o([35, 25, 20, 35], [99, 90, 80, 0], 60, 2, 0, 0, 2, 0),
        o([40, 30, 25, 35], [99, 95, 90, 0], 92, 1, 0, -4, 1, 0),
        o([35, 25, 20, 35], [99, 90, 80, 0], 58, 1, 0, 0, 2, 0),
        o([38, 30, 25, 35], [99, 95, 90, 0], 86, 0, 0, 2, 1, 0),
        o([30, 25, 20, 35], [99, 85, 75, 0], 55, 3, 0, 0, 2, 0),
    // Pressure swells it 6 dB and adds 0.3 semitones of vibrato.
    ], extra: &[(LFO_PMD, 5.0), (LFO_SPEED, 30.0), (LFO_WAVE, 4.0), (LFO_DELAY, 30.0), (LEVEL, 0.0),
                (press_target(0), target::LEVEL as f32), (press_depth(0), 25.0), (press_target(1), target::VIBRATO as f32), (press_depth(1), 30.0)] },
    // Metallic: a fixed-frequency-free stack with odd ratios and feedback.
    Def { name: "Steel Pluck", alg: 1, fb: 5, ops: [
        o([99, 55, 35, 60], [99, 0, 0, 0], 99, 1, 0, 0, 3, 3),
        o([99, 70, 40, 60], [99, 0, 0, 0], 76, 3, 0, 0, 6, 3),
        o([99, 50, 35, 60], [99, 0, 0, 0], 88, 2, 0, 1, 3, 3),
        o([99, 75, 40, 60], [99, 0, 0, 0], 70, 5, 0, 0, 6, 3),
        o([99, 80, 45, 60], [99, 0, 0, 0], 64, 7, 0, 0, 6, 3),
        o([99, 85, 50, 60], [99, 0, 0, 0], 58, 9, 0, 0, 6, 3),
    ], extra: &[(LEVEL, -3.0), (press_target(0), target::OFF as f32)] },
];

/// The factory presets as parameter lists.
pub fn presets() -> Vec<FmPreset> {
    DEFS.iter()
        .map(|d| {
            let mut values = vec![(ALGORITHM, f32::from(d.alg)), (FEEDBACK, f32::from(d.fb))];
            for (k, o) in d.ops.iter().enumerate() {
                let n = k + 1;
                for s in 0..4 {
                    values.push((op_id(n, op::R1 + s as u16), f32::from(o.rates[s])));
                    values.push((op_id(n, op::L1 + s as u16), f32::from(o.levels[s])));
                }
                values.push((op_id(n, op::LEVEL), f32::from(o.out)));
                values.push((op_id(n, op::COARSE), f32::from(o.coarse)));
                values.push((op_id(n, op::FINE), f32::from(o.fine)));
                values.push((op_id(n, op::DETUNE), f32::from(o.detune)));
                values.push((op_id(n, op::VEL_SENS), f32::from(o.vel)));
                values.push((op_id(n, op::RATE_SCALE), f32::from(o.rs)));
            }
            values.extend_from_slice(d.extra);
            FmPreset { name: d.name.to_string(), values }
        })
        .collect()
}
