//! FM: a six-operator FM synth in the DX7's mould.
//!
//! - **Operators.** Six sine operators, each with a frequency (a ratio of
//!   the note, or a fixed frequency), an output level, a four-rate,
//!   four-level envelope, velocity sensitivity, keyboard level and rate
//!   scaling, and amplitude modulation from the LFO.
//! - **Algorithms.** The DX7's 32 ways of wiring them ([`ALGORITHMS`]), with
//!   feedback (0 to 7) on one operator or, for algorithms 4 and 6, around a
//!   loop of two or three.
//! - **Global.** A pitch envelope, an LFO (six waves, speed, delay, pitch and
//!   amplitude depth, key sync), transpose, poly or mono, bend.
//! - **Pressure.** Pad pressure goes through the shared shaper
//!   ([`mpfree_node::pressure`]) into four slots
//!   ([`crate::pressure::FM_TARGETS`]): brightness (every modulator's level
//!   together), a level swell, vibrato, the LFO's rate, tremolo, pitch and
//!   feedback.
//!
//! The envelopes work in the DX7's way: in decibels, falling at a constant
//! rate, with the attack curving over as it nears its level, and rates and
//! levels from 0 to 99 that mean what they mean on a DX7. The numbers come
//! from published descriptions of the DX7 (the operator level table, the
//! rate formula, the keyboard scaling curves); where the original is not
//! public (the LFO speed curve, the pitch envelope's tables, detune), the
//! values are close approximations. It is DX7-style, not a bit-exact clone.
//!
//! Timing: operators run every frame; envelopes, pitch and the LFO run once
//! every [`CTRL`] frames aligned to the absolute frame, with operator gains
//! ramped across each step, so the output does not depend on the block size.

use mpfree_node::{
    Ctx, Kind, Node, NodeEvent, NodeInfo, ParamInfo, Pressure, PressureChord, PressureCoefs,
    PressureMode, PressureShape, Smooth, Timed,
};

use crate::dsp::{note_hz, Rng};
use crate::fm_params::*;
use crate::pressure::{swell_gain, Depths, Slot};

/// Frames per control step.
pub const CTRL: u64 = 16;
/// The most notes sounding at once (the Voices parameter's top).
pub const MAX_POLY: usize = 16;
/// Voice slots: the notes plus room for stolen voices to fade out.
pub const POOL: usize = MAX_POLY + 4;
const KILL_MS: f32 = 3.0;
/// Output of one carrier at full level, about -12 dBFS. A voice with more
/// carriers is scaled by 1/sqrt(carriers), so its loudness stays about the
/// same when the algorithm changes.
const VOICE_GAIN: f32 = 0.25;
/// The DX7's envelope rates are defined at this sample rate.
const EG_RATE: f32 = 44_100.0;

static INFO: NodeInfo = NodeInfo {
    id: "mpfree.fm",
    name: "FM",
    kind: Kind::Instrument,
};

// ---------------------------------------------------------------- algorithms

/// One algorithm: which operator modulates which (`(from, to)`, operators
/// numbered 1 to 6), and the feedback path (`(from, to)`; the same operator
/// for a self loop). Operators that modulate nothing are carriers.
#[derive(Clone, Copy, Debug)]
pub struct Algorithm {
    pub edges: &'static [(u8, u8)],
    pub feedback: (u8, u8),
}

const fn alg(edges: &'static [(u8, u8)], fb_from: u8, fb_to: u8) -> Algorithm {
    Algorithm {
        edges,
        feedback: (fb_from, fb_to),
    }
}

const A1: &[(u8, u8)] = &[(2, 1), (4, 3), (5, 4), (6, 5)];
const A3: &[(u8, u8)] = &[(2, 1), (3, 2), (5, 4), (6, 5)];
const A5: &[(u8, u8)] = &[(2, 1), (4, 3), (6, 5)];
const A7: &[(u8, u8)] = &[(2, 1), (4, 3), (5, 3), (6, 5)];
const A10: &[(u8, u8)] = &[(2, 1), (3, 2), (5, 4), (6, 4)];
const A12: &[(u8, u8)] = &[(2, 1), (4, 3), (5, 3), (6, 3)];
const A14: &[(u8, u8)] = &[(2, 1), (4, 3), (5, 4), (6, 4)];
const A16: &[(u8, u8)] = &[(2, 1), (3, 1), (4, 3), (5, 1), (6, 5)];
const A26: &[(u8, u8)] = &[(3, 2), (5, 4), (6, 4)];

/// The DX7's 32 algorithms, in order.
pub const ALGORITHMS: [Algorithm; 32] = [
    alg(A1, 6, 6),
    alg(A1, 2, 2),
    alg(A3, 6, 6),
    alg(A3, 4, 6),
    alg(A5, 6, 6),
    alg(A5, 5, 6),
    alg(A7, 6, 6),
    alg(A7, 4, 4),
    alg(A7, 2, 2),
    alg(A10, 3, 3),
    alg(A10, 6, 6),
    alg(A12, 2, 2),
    alg(A12, 6, 6),
    alg(A14, 6, 6),
    alg(A14, 2, 2),
    alg(A16, 6, 6),
    alg(A16, 2, 2),
    alg(&[(2, 1), (3, 1), (4, 1), (5, 4), (6, 5)], 3, 3),
    alg(&[(2, 1), (3, 2), (6, 4), (6, 5)], 6, 6),
    alg(&[(3, 1), (3, 2), (5, 4), (6, 4)], 3, 3),
    alg(&[(3, 1), (3, 2), (6, 4), (6, 5)], 3, 3),
    alg(&[(2, 1), (6, 3), (6, 4), (6, 5)], 6, 6),
    alg(&[(3, 2), (6, 4), (6, 5)], 6, 6),
    alg(&[(6, 3), (6, 4), (6, 5)], 6, 6),
    alg(&[(6, 4), (6, 5)], 6, 6),
    alg(A26, 6, 6),
    alg(A26, 3, 3),
    alg(&[(2, 1), (4, 3), (5, 4)], 5, 5),
    alg(&[(4, 3), (6, 5)], 6, 6),
    alg(&[(4, 3), (5, 4)], 5, 5),
    alg(&[(6, 5)], 6, 6),
    alg(&[], 6, 6),
];

/// An algorithm worked out for the audio loop (operators 0 to 5).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Route {
    /// For each operator, a bit per operator it modulates.
    pub targets: [u8; 6],
    /// A bit per carrier.
    pub carriers: u8,
    pub fb_from: usize,
    pub fb_to: usize,
}

impl Route {
    /// Algorithm `n`, 1 to 32 (clamped).
    pub fn new(n: usize) -> Route {
        let a = &ALGORITHMS[n.clamp(1, 32) - 1];
        let mut targets = [0u8; 6];
        for &(from, to) in a.edges {
            targets[usize::from(from) - 1] |= 1 << (to - 1);
        }
        let carriers = (0..6)
            .filter(|&j| targets[j] == 0)
            .fold(0u8, |m, j| m | 1 << j);
        Route {
            targets,
            carriers,
            fb_from: usize::from(a.feedback.0) - 1,
            fb_to: usize::from(a.feedback.1) - 1,
        }
    }
}

// ---------------------------------------------------------------- DX7 tables

/// Output level 0 to 99 on the DX7's internal 0 to 127 scale.
pub fn scale_level(level: u8) -> i32 {
    const LOW: [u8; 20] = [
        0, 5, 9, 13, 17, 20, 23, 25, 27, 29, 31, 33, 35, 37, 39, 41, 42, 43, 45, 46,
    ];
    let l = level.min(99);
    if l >= 20 {
        i32::from(l) + 28
    } else {
        i32::from(LOW[usize::from(l)])
    }
}

/// Keyboard level scaling: how much a curve adds (or takes away) `group`
/// thirds of an octave from the break point, at depth 0 to 99.
fn scale_curve(group: i32, depth: i32, curve: u8) -> i32 {
    const EXP: [i32; 33] = [
        0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 11, 14, 16, 19, 23, 27, 33, 39, 47, 56, 66, 80, 94, 110, 126,
        142, 158, 174, 190, 206, 222, 238, 250,
    ];
    let s = if curve == 0 || curve == 3 {
        (group * depth * 329) >> 12
    } else {
        (EXP[group.clamp(0, 32) as usize] * depth * 329) >> 15
    };
    if curve < 2 {
        -s
    } else {
        s
    }
}

/// Keyboard level scaling for a note, on the 0 to 127 scale.
pub fn scale_keyboard(note: u8, brk: u8, ld: u8, rd: u8, lc: u8, rc: u8) -> i32 {
    // Break point 0 is A-1 (note 21); 39 is C3 (note 60).
    let offset = i32::from(note) - i32::from(brk) - 21;
    if offset >= 0 {
        scale_curve((offset + 1) / 3, i32::from(rd), rc)
    } else {
        scale_curve((1 - offset) / 3, i32::from(ld), lc)
    }
}

/// Velocity's effect on an operator's level (in 1/256 octave), for
/// sensitivity 0 to 7. Full velocity lifts a sensitive operator a little.
pub fn scale_velocity(velocity: u8, sens: u8) -> i32 {
    const CURVE: [u8; 64] = [
        0, 70, 86, 97, 106, 114, 121, 126, 132, 138, 142, 148, 152, 156, 160, 163, 166, 170, 173,
        174, 178, 181, 184, 186, 189, 190, 194, 196, 198, 200, 202, 205, 206, 209, 211, 214, 216,
        218, 220, 222, 224, 225, 227, 229, 230, 232, 233, 235, 237, 238, 240, 241, 242, 243, 244,
        246, 246, 248, 249, 250, 251, 252, 253, 254,
    ];
    let v = i32::from(CURVE[usize::from(velocity.min(127) >> 1)]) - 239;
    ((i32::from(sens.min(7)) * v + 7) >> 3) << 4
}

/// Keyboard rate scaling: how much faster the envelope runs for a note.
pub fn scale_rate(note: u8, sens: u8) -> i32 {
    let x = (i32::from(note) / 3 - 7).clamp(0, 31);
    (i32::from(sens.min(7)) * x) >> 3
}

/// An operator's frequency in Hz for a note pitch (fractional MIDI note).
pub fn op_freq(pitch: f32, fixed: bool, coarse: u8, fine: u8, detune: i8) -> f32 {
    if fixed {
        return 10f32.powf(f32::from(coarse & 3) + f32::from(fine.min(99)) / 100.0);
    }
    let ratio = if coarse == 0 {
        0.5
    } else {
        f32::from(coarse.min(31))
    };
    let ratio = ratio * (1.0 + f32::from(fine.min(99)) / 100.0);
    // Detune: about a cent a step.
    note_hz(pitch + f32::from(detune.clamp(-7, 7)) / 100.0) * ratio
}

/// LFO speed 0 to 99 in Hz (about 0.06 to 49 Hz).
pub fn lfo_hz(speed: u8) -> f32 {
    const PTS: [(f32, f32); 15] = [
        (0.0, 0.062),
        (5.0, 0.27),
        (10.0, 0.58),
        (15.0, 1.0),
        (20.0, 1.6),
        (25.0, 2.6),
        (30.0, 3.9),
        (35.0, 5.2),
        (40.0, 6.5),
        (50.0, 9.6),
        (60.0, 13.8),
        (70.0, 19.6),
        (80.0, 27.5),
        (90.0, 37.5),
        (99.0, 49.3),
    ];
    interp(&PTS, f32::from(speed.min(99)))
}

/// Pitch envelope level 0 to 99 in octaves (50 is none; 0 and 99 are about
/// four octaves down and up, with fine steps near the middle).
pub fn peg_octaves(level: u8) -> f32 {
    let x = f32::from(level.min(99)) - 50.0;
    let units = if x.abs() <= 35.0 {
        x
    } else if x > 0.0 {
        35.0 + (x - 35.0).powi(2) * (92.0 / 196.0)
    } else {
        -(35.0 + (-x - 35.0).powi(2) * (93.0 / 225.0))
    };
    units / 32.0
}

/// Pitch envelope rate 0 to 99 in octaves per second.
pub fn peg_rate(rate: u8) -> f32 {
    const PTS: [(f32, f32); 6] = [
        (0.0, 1.0),
        (25.0, 14.0),
        (50.0, 39.0),
        (75.0, 88.0),
        (90.0, 165.0),
        (99.0, 254.0),
    ];
    interp(&PTS, f32::from(rate.min(99))) * 1.502 / 32.0
}

/// Semitones of vibrato at full pitch depth, for pitch sensitivity 0 to 7.
const PMS_SEMIS: [f32; 8] = [0.0, 0.32, 0.6, 1.0, 1.65, 2.8, 4.8, 12.0];
/// Tremolo depth at full amplitude depth, in 1/256 octave, for AMS 0 to 3.
const AMS_UNITS: [f32; 4] = [0.0, 128.0, 384.0, 1536.0];

fn interp(pts: &[(f32, f32)], x: f32) -> f32 {
    for w in pts.windows(2) {
        let ((x0, y0), (x1, y1)) = (w[0], w[1]);
        if x <= x1 {
            return y0 + (y1 - y0) * (x - x0) / (x1 - x0);
        }
    }
    pts[pts.len() - 1].1
}

// ---------------------------------------------------------------- envelopes

/// Full level (output 99, envelope level 99) on the envelope's scale of
/// 1/256 octave.
pub const FULL: f32 = 3840.0;
/// An attack starts here at once, about 50 dB under full.
const JUMP: f32 = 1716.0;
/// The floor.
const FLOOR: f32 = 16.0;

/// A DX7-style envelope: four rates and four levels. Stage 3 (R3 to L3)
/// then holds until the key lifts; stage 4 (R4 to L4) is the release.
#[derive(Clone, Copy, Debug, Default)]
pub struct DxEnv {
    /// Level in 1/256 octave; [`FULL`] is full scale.
    pub level: f32,
    target: f32,
    inc: f32,
    rising: bool,
    /// 0 to 3, or 4 when done.
    pub stage: u8,
    down: bool,
}

impl DxEnv {
    /// Works out the current stage's target and speed. `outlevel` is the
    /// operator's level (output level, keyboard and velocity scaling) in
    /// 1/256 octave; `rs` is the keyboard rate scaling.
    fn aim(&mut self, rates: &[u8; 4], levels: &[u8; 4], outlevel: f32, rs: i32) {
        let ix = usize::from(self.stage);
        if ix >= 4 {
            return;
        }
        let l = ((scale_level(levels[ix]) >> 1) << 6) as f32;
        self.target = (l + outlevel - 4256.0).max(FLOOR);
        self.rising = self.target > self.level;
        let q = (((i32::from(rates[ix].min(99)) * 41) >> 6) + rs).clamp(0, 63);
        // Per sample at 44.1 kHz, in 1/256 octave.
        self.inc = ((4 + (q & 3)) << (2 + (q >> 2))) as f32 / 65_536.0;
    }

    pub fn key_on(&mut self) {
        self.down = true;
        self.stage = 0;
    }

    pub fn key_off(&mut self) {
        self.down = false;
        self.stage = 3;
    }

    /// Runs `n` frames' worth (at 44.1 kHz) and returns the level.
    fn run(&mut self, n: f32, rates: &[u8; 4], levels: &[u8; 4], outlevel: f32, rs: i32) -> f32 {
        self.aim(rates, levels, outlevel, rs);
        let moving = self.stage < 3 || (self.stage < 4 && !self.down);
        if !moving {
            return self.level;
        }
        if self.rising {
            if self.level < JUMP {
                self.level = JUMP;
            }
            // The attack slows as it nears the top.
            let slope = (17.0 - self.level / 256.0).floor().max(1.0);
            self.level += slope * self.inc * n;
            if self.level >= self.target {
                self.level = self.target;
                self.stage += 1;
            }
        } else {
            self.level -= self.inc * n;
            if self.level <= self.target {
                self.level = self.target;
                self.stage += 1;
            }
        }
        self.level
    }

    /// Heading down (or holding), not climbing.
    pub fn falling(&self) -> bool {
        !self.rising
    }

    /// The gain at a level, 1.0 at full.
    pub fn gain(level: f32) -> f32 {
        if level <= FLOOR {
            0.0
        } else {
            ((level - FULL) / 256.0).exp2()
        }
    }

    /// Frames (at 44.1 kHz) the current stage takes from here, once
    /// [`DxEnv::aim`] has run.
    fn stage_frames(&self) -> f32 {
        if self.inc <= 0.0 {
            return 0.0;
        }
        if !self.rising {
            return (self.level - self.target).max(0.0) / self.inc;
        }
        // The attack runs at a slope that steps down every 256 units.
        let mut lv = self.level.max(JUMP);
        let mut frames = 0.0;
        while lv < self.target {
            let slope = (17.0 - lv / 256.0).floor().max(1.0);
            let top = ((lv / 256.0).floor() + 1.0) * 256.0;
            let top = top.min(self.target);
            frames += (top - lv) / (slope * self.inc);
            lv = top;
        }
        frames
    }
}

/// An operator envelope's four stages for drawing, as the engine runs them
/// at full output level with no keyboard rate scaling: each stage's points as
/// (seconds since the stage began, level), level 0 to 1 on the envelope's
/// own scale (1.0 is full, 0.5 about 45 dB down). The note starts from L4,
/// holds once stage 3 reaches L3, and stage 4 is the release.
pub fn env_stages(rates: &[u8; 4], levels: &[u8; 4]) -> [Vec<(f32, f32)>; 4] {
    // Output 99 and no scaling: the stage targets are the levels alone.
    let out = (scale_level(99) << 5) as f32;
    let mut e = DxEnv::default();
    let rest = ((scale_level(levels[3]) >> 1) << 6) as f32 + out - 4256.0;
    e.level = rest.max(FLOOR);
    e.key_on();
    let norm = |l: f32| {
        if l <= FLOOR {
            0.0
        } else {
            (l / FULL).clamp(0.0, 1.0)
        }
    };
    let mut stages: [Vec<(f32, f32)>; 4] = Default::default();
    for (k, pts) in stages.iter_mut().enumerate() {
        if k == 3 {
            e.key_off();
        }
        e.aim(rates, levels, out, 0);
        let frames = e.stage_frames();
        // About 40 points a stage, so the attack's bends show.
        let chunk = (frames / 40.0).max(1.0);
        pts.push((0.0, norm(e.level)));
        let mut t = 0.0;
        for _ in 0..200 {
            if usize::from(e.stage) != k {
                break;
            }
            e.run(chunk, rates, levels, out, 0);
            t += chunk;
            pts.push((t / EG_RATE, norm(e.level)));
        }
        // Equal levels take no time: one point is enough.
        if frames == 0.0 {
            pts.truncate(1);
            e.stage = k as u8 + 1;
        }
    }
    stages
}

/// The pitch envelope, in octaves. It starts at L4, moves to L1, L2, L3,
/// holds, and returns to L4 when the key lifts.
#[derive(Clone, Copy, Debug, Default)]
struct PitchEnv {
    level: f32,
    stage: u8,
    down: bool,
}

impl PitchEnv {
    fn key_on(&mut self, levels: &[u8; 4]) {
        self.level = peg_octaves(levels[3]);
        self.stage = 0;
        self.down = true;
    }

    fn key_off(&mut self) {
        self.down = false;
        self.stage = 3;
    }

    fn run(&mut self, secs: f32, rates: &[u8; 4], levels: &[u8; 4]) -> f32 {
        let moving = self.stage < 3 || (self.stage < 4 && !self.down);
        if !moving {
            return self.level;
        }
        let ix = usize::from(self.stage);
        let target = peg_octaves(levels[ix]);
        let step = peg_rate(rates[ix]) * secs;
        if (target - self.level).abs() <= step {
            self.level = target;
            self.stage += 1;
        } else {
            self.level += step.copysign(target - self.level);
        }
        self.level
    }
}

// ---------------------------------------------------------------- voices

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Free,
    On,
    Release,
    Kill,
}

#[derive(Clone, Copy, Debug, Default)]
struct OpState {
    phase: u32,
    dphase: u32,
    env: DxEnv,
    /// Gain now and its change per frame, ramped across a control step.
    g: f32,
    gstep: f32,
}

#[derive(Clone, Copy, Debug)]
struct Voice {
    state: State,
    note: u8,
    velocity: u8,
    age: u64,
    ops: [OpState; 6],
    /// The feedback path's last two outputs.
    fb: [f32; 2],
    peg: PitchEnv,
    press: Pressure,
    /// The pressure this voice uses, 0 to 1: its own, the chord's or none.
    pv: f32,
    /// Feedback gain (pressure can move it).
    fb_scale: f32,
    kill_gain: f32,
    kill_step: f32,
}

impl Voice {
    fn new() -> Voice {
        Voice {
            state: State::Free,
            note: 60,
            velocity: 0,
            age: 0,
            ops: [OpState::default(); 6],
            fb: [0.0; 2],
            peg: PitchEnv::default(),
            press: Pressure::new(),
            pv: 0.0,
            fb_scale: 0.0,
            kill_gain: 1.0,
            kill_step: 0.0,
        }
    }

    fn sounding(&self) -> bool {
        matches!(self.state, State::On | State::Release)
    }
}

/// One operator's settings, read from the parameters.
#[derive(Clone, Copy, Debug, Default)]
struct OpParams {
    rates: [u8; 4],
    levels: [u8; 4],
    out: u8,
    fixed: bool,
    coarse: u8,
    fine: u8,
    detune: i8,
    vel: u8,
    ams: u8,
    rs: u8,
    brk: u8,
    ld: u8,
    rd: u8,
    lc: u8,
    rc: u8,
    on: bool,
}

/// Settings every voice shares, read when a parameter changes.
#[derive(Clone, Copy, Debug, Default)]
struct Patch {
    ops: [OpParams; 6],
    route: Route,
    fb: u8,
    osc_sync: bool,
    transpose: f32,
    peg_rates: [u8; 4],
    peg_levels: [u8; 4],
    lfo_wave: u8,
    lfo_speed: u8,
    lfo_delay: u8,
    pmd: u8,
    amd: u8,
    lfo_sync: bool,
    pms: u8,
    poly: usize,
    mono: bool,
    bend_range: f32,
}

pub struct Fm {
    rate: u32,
    values: [f32; ID_SPACE],
    patch: Patch,
    voices: [Voice; POOL],
    sine: Vec<f32>,
    rng: Rng,
    lfo_phase: f32,
    lfo_held: f32,
    lfo_value: f32,
    /// The LFO's wave without the delay's fade-in (pressure vibrato and
    /// tremolo start at once).
    lfo_raw: f32,
    /// Seconds since the LFO delay last started.
    lfo_time: f32,
    /// The pressure shaper worked out for the control rate, and the slots.
    press_c: PressureCoefs,
    depths: Depths,
    chord: PressureChord,
    bend: Smooth,
    out: Smooth,
    stack: [u8; MAX_POLY],
    stack_len: usize,
    clock: u64,
    meter_voices: f32,
    /// Notes that did not fit anywhere (should stay 0).
    pub dropped: u64,
}

const SIN_BITS: u32 = 12;
const SIN_N: usize = 1 << SIN_BITS;
const FRAC_BITS: u32 = 32 - SIN_BITS;

impl Default for Fm {
    fn default() -> Self {
        Fm::new(48_000)
    }
}

impl Fm {
    pub fn new(rate: u32) -> Fm {
        let mut values = [0.0; ID_SPACE];
        for p in PARAMS.iter() {
            values[p.id as usize] = p.default;
        }
        let sine = (0..=SIN_N)
            .map(|i| (i as f64 / SIN_N as f64 * std::f64::consts::TAU).sin() as f32)
            .collect();
        let mut s = Fm {
            rate,
            values,
            patch: Patch::default(),
            voices: [Voice::new(); POOL],
            sine,
            rng: Rng::new(0x00F3_D7E5),
            lfo_phase: 0.0,
            lfo_held: 0.0,
            lfo_value: 0.0,
            lfo_raw: 0.0,
            lfo_time: 0.0,
            press_c: PressureCoefs::default(),
            depths: Depths::default(),
            chord: PressureChord::default(),
            bend: Smooth::new(0.0),
            out: Smooth::new(1.0),
            stack: [0; MAX_POLY],
            stack_len: 0,
            clock: 0,
            meter_voices: 0.0,
            dropped: 0,
        };
        s.read_patch();
        s.prepare(rate, mpfree_node::MAX_BLOCK);
        s
    }

    /// Loads a preset: defaults, then its values. Sounding notes carry on.
    pub fn load_preset(&mut self, preset: &FmPreset) {
        self.load_values(&preset.values);
    }

    /// Defaults, then `values` (a DX7 voice, a preset).
    pub fn load_values(&mut self, values: &[(u16, f32)]) {
        for p in PARAMS.iter() {
            if !p.readonly {
                self.values[p.id as usize] = p.default;
            }
        }
        for &(id, v) in values {
            if let Some(p) = info(id) {
                if !p.readonly {
                    self.values[id as usize] = p.clamp(v);
                }
            }
        }
        self.read_patch();
    }

    pub fn presets() -> Vec<FmPreset> {
        presets()
    }

    /// Voices sounding or fading.
    pub fn active_voices(&self) -> usize {
        self.voices
            .iter()
            .filter(|v| v.state != State::Free)
            .count()
    }

    /// The pressure each sounding voice uses now (0 to 1), in voice order,
    /// for the Synth tab's live bars. Returns how many it wrote.
    pub fn pressures(&self, out: &mut [f32]) -> usize {
        let mut n = 0;
        for v in self.voices.iter().filter(|v| v.sounding()) {
            if n == out.len() {
                break;
            }
            out[n] = v.pv;
            n += 1;
        }
        n
    }

    /// Works out the pressure shaper and slots from the values.
    fn update_pressure(&mut self) {
        let shape = PressureShape::from_params(PRESS_BASE, |id| self.values[id as usize]);
        self.press_c = shape.coefs(self.rate as f32 / CTRL as f32);
        let slots: [Slot; 4] = std::array::from_fn(|n| Slot {
            target: self.v(press_target(n)) as u8,
            depth: self.v(press_depth(n)) / 100.0,
        });
        // Off: pressure does nothing, and a level swell plays at the preset's
        // full level (not at its resting dip).
        self.depths = if shape.mode == PressureMode::Off {
            Depths::default()
        } else {
            Depths::from_slots(&slots)
        };
    }

    fn v(&self, id: u16) -> f32 {
        self.values[id as usize]
    }

    fn read_patch(&mut self) {
        let u = |id: u16| self.values[id as usize].max(0.0) as u8;
        let mut ops = [OpParams::default(); 6];
        for (k, o) in ops.iter_mut().enumerate() {
            let f = |field: u16| op_id(k + 1, field);
            *o = OpParams {
                rates: [u(f(op::R1)), u(f(op::R2)), u(f(op::R3)), u(f(op::R4))],
                levels: [u(f(op::L1)), u(f(op::L2)), u(f(op::L3)), u(f(op::L4))],
                out: u(f(op::LEVEL)),
                fixed: self.v(f(op::MODE)) >= 0.5,
                coarse: u(f(op::COARSE)),
                fine: u(f(op::FINE)),
                detune: self.v(f(op::DETUNE)) as i8,
                vel: u(f(op::VEL_SENS)),
                ams: u(f(op::AMS)),
                rs: u(f(op::RATE_SCALE)),
                brk: u(f(op::BREAK)),
                ld: u(f(op::L_DEPTH)),
                rd: u(f(op::R_DEPTH)),
                lc: u(f(op::L_CURVE)),
                rc: u(f(op::R_CURVE)),
                on: self.v(f(op::ON)) >= 0.5,
            };
        }
        let lvl = self.v(LEVEL);
        let route = Route::new(self.v(ALGORITHM) as usize);
        let trim = 1.0 / (route.carriers.count_ones().max(1) as f32).sqrt();
        self.out.target = if lvl <= -60.0 {
            0.0
        } else {
            10f32.powf(lvl / 20.0) * trim
        };
        if self.voices.iter().all(|v| v.state == State::Free) {
            // Nothing sounds, so nothing can click: take the new level at once.
            self.out.snap(self.out.target);
        }
        self.patch = Patch {
            ops,
            route,
            fb: u(FEEDBACK),
            osc_sync: self.v(OSC_SYNC) >= 0.5,
            transpose: self.v(TRANSPOSE),
            peg_rates: [u(PEG_R1), u(PEG_R2), u(PEG_R3), u(PEG_R4)],
            peg_levels: [u(PEG_L1), u(PEG_L2), u(PEG_L3), u(PEG_L4)],
            lfo_wave: u(LFO_WAVE),
            lfo_speed: u(LFO_SPEED),
            lfo_delay: u(LFO_DELAY),
            pmd: u(LFO_PMD),
            amd: u(LFO_AMD),
            lfo_sync: self.v(LFO_SYNC) >= 0.5,
            pms: u(LFO_PMS),
            poly: (self.v(POLYPHONY) as usize).clamp(1, MAX_POLY),
            mono: self.v(VOICE_MODE) >= 0.5,
            bend_range: self.v(BEND_RANGE),
        };
        self.update_pressure();
    }

    // ---- notes ----

    fn note_on(&mut self, note: u8, velocity: u8) {
        if velocity == 0 {
            self.note_off(note);
            return;
        }
        self.clock += 1;
        if self.stack_len == 0 {
            self.lfo_time = 0.0;
        }
        if self.patch.lfo_sync {
            self.lfo_phase = 0.0;
        }
        self.stack_remove(note);
        if self.stack_len == MAX_POLY {
            self.stack.copy_within(1.., 0);
            self.stack_len -= 1;
        }
        let held = self.stack_len > 0;
        self.stack[self.stack_len] = note;
        self.stack_len += 1;

        let slot = if self.patch.mono {
            let v = &mut self.voices[0];
            if held && v.state == State::On {
                // Legato: a new pitch, no new attack.
                v.note = note;
                v.press.start();
                self.voice_control(0, false);
                return;
            }
            0
        } else {
            match self
                .voices
                .iter()
                .position(|v| v.sounding() && v.note == note)
            {
                Some(i) => i,
                None => {
                    let sounding = self.voices.iter().filter(|v| v.sounding()).count();
                    if sounding >= self.patch.poly {
                        self.steal();
                    }
                    match self.voices.iter().position(|v| v.state == State::Free) {
                        Some(i) => i,
                        None => {
                            let Some(i) = (0..POOL).min_by(|&a, &b| {
                                self.voices[a]
                                    .kill_gain
                                    .total_cmp(&self.voices[b].kill_gain)
                            }) else {
                                self.dropped += 1;
                                return;
                            };
                            i
                        }
                    }
                }
            }
        };
        let sync = self.patch.osc_sync;
        let peg_levels = self.patch.peg_levels;
        let v = &mut self.voices[slot];
        if !v.sounding() {
            v.ops = [OpState::default(); 6];
            v.fb = [0.0; 2];
            v.press.clear();
            v.pv = 0.0;
            v.kill_gain = 1.0;
        } else if sync {
            for o in &mut v.ops {
                o.phase = 0;
            }
        }
        v.state = State::On;
        v.press.start();
        v.note = note;
        v.velocity = velocity.min(127);
        v.age = self.clock;
        for o in &mut v.ops {
            o.env.key_on();
        }
        v.peg.key_on(&peg_levels);
        self.voice_control(slot, false);
    }

    fn steal(&mut self) {
        let rel = (0..POOL)
            .filter(|&i| self.voices[i].state == State::Release)
            .min_by(|&a, &b| self.voice_level(a).total_cmp(&self.voice_level(b)));
        let victim = rel.or_else(|| {
            (0..POOL)
                .filter(|&i| self.voices[i].state == State::On)
                .min_by_key(|&i| self.voices[i].age)
        });
        if let Some(i) = victim {
            let n = (KILL_MS * 0.001 * self.rate as f32).max(1.0);
            let v = &mut self.voices[i];
            v.state = State::Kill;
            v.kill_gain = 1.0;
            v.kill_step = 1.0 / n;
        }
    }

    /// A voice's loudest carrier gain.
    fn voice_level(&self, i: usize) -> f32 {
        let c = self.patch.route.carriers;
        (0..6)
            .filter(|j| c & (1 << j) != 0)
            .map(|j| self.voices[i].ops[j].g)
            .fold(0.0, f32::max)
    }

    fn stack_remove(&mut self, note: u8) {
        if let Some(i) = self.stack[..self.stack_len].iter().position(|&n| n == note) {
            self.stack.copy_within(i + 1..self.stack_len, i);
            self.stack_len -= 1;
        }
    }

    fn release(v: &mut Voice) {
        v.state = State::Release;
        v.press.release();
        for o in &mut v.ops {
            o.env.key_off();
        }
        v.peg.key_off();
    }

    fn note_off(&mut self, note: u8) {
        self.stack_remove(note);
        if self.patch.mono {
            let back = (self.stack_len > 0).then(|| self.stack[self.stack_len - 1]);
            let v = &mut self.voices[0];
            if v.state != State::On || v.note != note {
                return;
            }
            match back {
                Some(n) => {
                    v.note = n;
                    self.voice_control(0, false);
                }
                None => Self::release(v),
            }
            return;
        }
        for v in &mut self.voices {
            if v.state == State::On && v.note == note {
                Self::release(v);
            }
        }
    }

    fn all_off(&mut self) {
        self.stack_len = 0;
        for v in &mut self.voices {
            if v.state == State::On {
                Self::release(v);
            }
        }
    }

    fn apply(&mut self, ev: &NodeEvent) {
        match *ev {
            NodeEvent::NoteOn { note, velocity } => self.note_on(note, velocity),
            NodeEvent::NoteOff { note } => self.note_off(note),
            NodeEvent::Pressure { note, value } => {
                for v in &mut self.voices {
                    if v.sounding() && v.note == note {
                        v.press.set(value);
                    }
                }
            }
            NodeEvent::Bend { value } => {
                self.bend.target = if value.is_finite() {
                    value.clamp(-1.0, 1.0)
                } else {
                    0.0
                }
            }
            NodeEvent::AllOff => self.all_off(),
            NodeEvent::Param { id, value } => self.set_param(id, value),
        }
    }

    // ---- control rate ----

    fn control(&mut self) {
        let secs = CTRL as f32 / self.rate as f32;
        self.bend.next();
        self.out.next();
        // Pressure: the chord (Highest mode) aims before the voices step.
        let c = &self.press_c;
        let chord = if c.mode == PressureMode::Highest {
            self.chord.step(
                c,
                self.voices
                    .iter()
                    .filter(|v| v.sounding())
                    .map(|v| (v.state == State::On, v.press.aim(c))),
            )
        } else {
            0.0
        };
        let mut top = 0.0f32;
        for v in self.voices.iter_mut().filter(|v| v.state != State::Free) {
            let own = v.press.step(c);
            v.pv = match c.mode {
                PressureMode::PerNote => own,
                PressureMode::Highest => chord,
                PressureMode::Off => 0.0,
            };
            if v.sounding() {
                top = top.max(v.pv);
            }
        }
        // LFO: phase, wave, and the delay's fade-in. Pressure on LFO rate
        // follows the highest pressure (the LFO is shared).
        let prev = self.lfo_phase;
        let speed =
            lfo_hz(self.patch.lfo_speed) * (self.depths.get(target::LFO_RATE) * 2.0 * top).exp2();
        self.lfo_phase = (self.lfo_phase + speed * secs).fract();
        if self.lfo_phase < prev {
            self.lfo_held = self.rng.bipolar();
        }
        let t = self.lfo_phase;
        let wave = match self.patch.lfo_wave {
            0 => {
                if t < 0.5 {
                    4.0 * t - 1.0
                } else {
                    3.0 - 4.0 * t
                }
            }
            1 => 1.0 - 2.0 * t,
            2 => 2.0 * t - 1.0,
            3 => {
                if t < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            4 => (std::f32::consts::TAU * t).sin(),
            _ => self.lfo_held,
        };
        self.lfo_time += secs;
        let d = 5.0 * (f32::from(self.patch.lfo_delay) / 99.0).powf(2.5);
        let fade = if d <= 0.0 {
            1.0
        } else {
            ((self.lfo_time - 0.5 * d) / (0.5 * d)).clamp(0.0, 1.0)
        };
        self.lfo_raw = wave;
        self.lfo_value = wave * fade;
        for i in 0..POOL {
            if self.voices[i].state != State::Free {
                self.voice_control(i, true);
            }
        }
    }

    /// Pitch and operator gains for one voice. `advance` runs the envelopes
    /// one control step (false right after a note starts).
    fn voice_control(&mut self, i: usize, advance: bool) {
        let p = &self.patch;
        let rate = self.rate as f32;
        let secs = CTRL as f32 / rate;
        let n = CTRL as f32 * EG_RATE / rate;
        let lfo = self.lfo_value;
        let raw = self.lfo_raw;
        let bend = self.bend.value * p.bend_range;
        let d = self.depths;
        let v = &mut self.voices[i];
        let press = v.pv;
        let peg = if advance {
            v.peg.run(secs, &p.peg_rates, &p.peg_levels)
        } else {
            v.peg.level
        };
        let vib = lfo * f32::from(p.pmd) / 99.0 * PMS_SEMIS[usize::from(p.pms.min(7))]
            + raw * press * d.get(target::VIBRATO);
        let pitch = f32::from(v.note)
            + p.transpose
            + peg * 12.0
            + vib
            + press * d.get(target::PITCH) * 12.0;
        let am = (1.0 - lfo) * 0.5 * f32::from(p.amd) / 99.0;
        // Carriers: the swell and tremolo. Modulators: brightness, in level
        // units (32 to an output level step, 0.75 dB).
        let carrier_gain = swell_gain(d.get(target::LEVEL).clamp(-1.0, 1.0), press)
            * (1.0 - d.get(target::TREMOLO).abs().min(1.0) * press * (0.5 - 0.5 * raw));
        let bright = press * d.get(target::BRIGHTNESS) * 24.0 * 32.0;
        let fb = (f32::from(p.fb.min(7)) + press * d.get(target::FEEDBACK) * 7.0).clamp(0.0, 7.0);
        v.fb_scale = if fb <= 0.0 { 0.0 } else { (fb - 8.0).exp2() };
        let mut carriers_done = true;
        for (j, o) in v.ops.iter_mut().enumerate() {
            let q = &p.ops[j];
            let hz = if q.fixed {
                op_freq(0.0, true, q.coarse, q.fine, q.detune)
            } else {
                op_freq(pitch + bend, false, q.coarse, q.fine, q.detune)
            };
            o.dphase = ((hz / rate).clamp(0.0, 0.49) * 4_294_967_296.0) as u32;
            let kbd = scale_keyboard(v.note, q.brk, q.ld, q.rd, q.lc, q.rc);
            let outlevel = (((scale_level(q.out) + kbd).clamp(0, 127)) << 5)
                + scale_velocity(v.velocity, q.vel);
            let outlevel = outlevel as f32;
            let rs = scale_rate(v.note, q.rs);
            let level = if advance {
                o.env.run(n, &q.rates, &q.levels, outlevel, rs)
            } else {
                o.env.level
            };
            let level = level - AMS_UNITS[usize::from(q.ams.min(3))] * am;
            let carrier = p.route.carriers & (1 << j) != 0;
            let target = if !(q.on && q.out > 0) {
                0.0
            } else if carrier {
                DxEnv::gain(level) * carrier_gain
            } else if level > FLOOR {
                // Brightness never brings a silent modulator back.
                DxEnv::gain((level + bright).max(FLOOR))
            } else {
                0.0
            };
            o.gstep = (target - o.g) / CTRL as f32;
            // A carrier is done once it is near silent (-80 dB) and not climbing.
            if carrier && q.on && (target > 1e-4 || !o.env.falling()) {
                carriers_done = false;
            }
        }
        if v.state == State::Release && carriers_done {
            // Every carrier has finished its release (or is near silent):
            // ramp to zero this step, then free.
            for o in &mut v.ops {
                o.gstep = -o.g / CTRL as f32;
            }
            v.state = State::Kill;
            v.kill_gain = 1.0;
            v.kill_step = 1.0 / CTRL as f32;
        }
    }

    // ---- audio rate ----

    #[inline]
    fn sin(tab: &[f32], ph: u32) -> f32 {
        let i = (ph >> FRAC_BITS) as usize;
        let f = (ph & ((1 << FRAC_BITS) - 1)) as f32 * (1.0 / (1u32 << FRAC_BITS) as f32);
        let a = tab[i];
        a + (tab[i + 1] - a) * f
    }

    fn render(&mut self, from: usize, to: usize, left: &mut [f32], right: &mut [f32]) {
        let r = self.patch.route;
        let gain = VOICE_GAIN * self.out.value;
        let tab = &self.sine[..];
        for v in &mut self.voices {
            if v.state == State::Free {
                continue;
            }
            for k in from..to {
                let mut m = [0f32; 6];
                let mut out = 0.0;
                let fb_in = (v.fb[0] + v.fb[1]) * v.fb_scale;
                for j in (0..6).rev() {
                    let o = &mut v.ops[j];
                    let mut input = m[j];
                    if j == r.fb_to {
                        input += fb_in;
                    }
                    let y = if o.g == 0.0 && o.gstep == 0.0 {
                        0.0
                    } else {
                        let shift = (input * 4_294_967_296.0) as i64 as u32;
                        Self::sin(tab, o.phase.wrapping_add(shift)) * o.g
                    };
                    o.phase = o.phase.wrapping_add(o.dphase);
                    o.g = (o.g + o.gstep).max(0.0);
                    if j == r.fb_from {
                        v.fb[1] = v.fb[0];
                        v.fb[0] = y;
                    }
                    let t = r.targets[j];
                    if t == 0 {
                        out += y;
                    } else {
                        // A modulator at full level swings the phase two cycles.
                        for (b, mb) in m.iter_mut().enumerate() {
                            if t & (1 << b) != 0 {
                                *mb += 2.0 * y;
                            }
                        }
                    }
                }
                let mut g = gain;
                if v.state == State::Kill {
                    v.kill_gain -= v.kill_step;
                    if v.kill_gain <= 0.0 {
                        v.kill_gain = 0.0;
                        v.state = State::Free;
                    }
                    g *= v.kill_gain;
                }
                left[k] += out * g;
                right[k] += out * g;
                if v.state == State::Free {
                    break;
                }
            }
        }
    }
}

impl Node for Fm {
    fn info(&self) -> &'static NodeInfo {
        &INFO
    }

    fn params(&self) -> &'static [ParamInfo] {
        &PARAMS
    }

    fn set_param(&mut self, id: u16, value: f32) {
        let Some(p) = info(id) else { return };
        if p.readonly {
            return;
        }
        self.values[id as usize] = p.clamp(value);
        self.read_patch();
        if id == VOICE_MODE {
            self.all_off();
        }
    }

    fn param(&self, id: u16) -> f32 {
        match id {
            METER_VOICES => self.meter_voices,
            _ if info(id).is_some() => self.values[id as usize],
            _ => f32::NAN,
        }
    }

    fn prepare(&mut self, rate: u32, _max_block: usize) {
        self.rate = rate.max(8_000);
        let cr = self.rate / CTRL as u32;
        self.bend.set_time(cr, 4.0);
        self.out.set_time(cr, 15.0);
        self.out.snap(self.out.target);
        self.update_pressure();
    }

    fn reset(&mut self) {
        for v in &mut self.voices {
            *v = Voice::new();
        }
        self.stack_len = 0;
        self.lfo_phase = 0.0;
        self.lfo_time = 0.0;
        self.chord.clear();
        self.meter_voices = 0.0;
        self.bend.snap(0.0);
        let rate = self.rate;
        self.prepare(rate, mpfree_node::MAX_BLOCK);
    }

    fn process(&mut self, ctx: &Ctx, events: &[Timed], left: &mut [f32], right: &mut [f32]) {
        assert_eq!(left.len(), right.len());
        if ctx.rate != self.rate {
            self.prepare(ctx.rate, left.len());
        }
        let len = left.len();
        let mut at = 0usize;
        let mut i = 0usize;
        while at < len {
            while i < events.len() && events[i].offset as usize <= at {
                self.apply(&events[i].ev);
                i += 1;
            }
            let next = events.get(i).map_or(len, |e| (e.offset as usize).min(len));
            while at < next {
                let abs = ctx.frame + at as u64;
                if abs.is_multiple_of(CTRL) {
                    self.control();
                }
                let end = next.min(at + (CTRL - abs % CTRL) as usize);
                self.render(at, end, left, right);
                at = end;
            }
        }
        for e in &events[i..] {
            self.apply(&e.ev);
        }
        self.meter_voices = self.voices.iter().filter(|v| v.sounding()).count() as f32;
    }

    fn tail_frames(&self) -> u32 {
        // The slowest release can take a long time; a host waits for the
        // voices to free instead. This is a bound for a typical patch.
        (4.0 * self.rate as f32) as u32
    }
}
