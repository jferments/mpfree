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
    Ctx, Cull, Kind, Node, NodeEvent, NodeInfo, ParamInfo, Pressure, PressureChord, PressureCoefs,
    PressureMode, PressureShape, Smooth, Timed, Tuning,
};

use crate::dsp::{same_key, Rng};
use crate::expr::{expr_coef, NoteExprs, VoiceExpr};
use crate::fm_params::*;
use crate::params::Preset;
use crate::pressure::{swell_gain, Depths, Slot};
use crate::voice::DcBlock;

/// Frames per control step.
pub const CTRL: u64 = 16;
/// The most notes sounding at once (the Voices parameter's top).
pub const MAX_POLY: usize = 16;
/// Voice slots: the notes plus room for stolen voices to fade out.
pub const POOL: usize = MAX_POLY + 4;
const KILL_MS: f32 = 3.0;
/// Output of one carrier at full level, about -12 dBFS (before the Level
/// setting, -3 dB by default). A voice with more carriers is brought back to
/// its loudest carrier's power ([`carrier_trim`]), so its loudness stays
/// about the same when the algorithm changes.
const VOICE_GAIN: f32 = 0.25;
/// The DC blocker's corner: the feedback and 1:1 pairs make DC.
const DC_HZ: f32 = 5.0;

/// The per-frame gain that fades the DC blocker's tail out in about 3 ms
/// once no voice sounds.
fn drain_coef(rate: u32) -> f32 {
    (-1.0 / (0.003 * rate.max(8_000) as f32)).exp()
}
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

/// An operator's frequency in Hz for a note pitch (fractional MIDI note),
/// in 12-TET at A4 = 440 Hz.
pub fn op_freq(pitch: f32, fixed: bool, coarse: u8, fine: u8, detune: i8) -> f32 {
    static STANDARD: Tuning = Tuning::standard();
    op_freq_in(&STANDARD, pitch, fixed, coarse, fine, detune)
}

/// [`op_freq`] in a tuning. A fixed frequency stays fixed.
pub fn op_freq_in(
    tuning: &Tuning,
    pitch: f32,
    fixed: bool,
    coarse: u8,
    fine: u8,
    detune: i8,
) -> f32 {
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
    tuning.hz(pitch + f32::from(detune.clamp(-7, 7)) / 100.0) * ratio
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

// ---------------------------------------------------------------- level and feedback

/// How loud an operator can get, 1.0 at full: its output level, with its
/// keyboard scaling at middle C, and its envelope at its highest level,
/// before velocity. 0 for an operator switched off or at output 0.
fn op_weight(o: &OpParams) -> f32 {
    if !o.on || o.out == 0 {
        return 0.0;
    }
    let peak = o.levels.iter().copied().max().unwrap_or(0);
    let env = ((scale_level(peak) >> 1) << 6) as f32;
    let kbd = scale_keyboard(60, o.brk, o.ld, o.rd, o.lc, o.rc);
    let out = ((scale_level(o.out) + kbd).clamp(0, 127) << 5) as f32;
    DxEnv::gain((env + out - 4256.0).max(FLOOR))
}

/// The level match across algorithms: the gain that brings the carriers'
/// summed power back to the loudest carrier's. One carrier, or several at
/// the same level, play as before (1, or 1/sqrt(carriers)); carriers at
/// different levels no longer drop the voice by the ones that add little.
/// So the loudest carrier sets the voice's level and the others its
/// balance, whichever algorithm makes them carriers.
pub fn carrier_trim(weights: &[f32; 6], carriers: u8) -> f32 {
    let (mut sum, mut top) = (0.0f32, 0.0f32);
    for (j, &w) in weights.iter().enumerate() {
        if carriers & (1 << j) != 0 {
            sum += w * w;
            top = top.max(w);
        }
    }
    if sum > 0.0 {
        top / sum.sqrt()
    } else {
        1.0
    }
}

/// The feedback amount (feedback scale times the operator's gain) at
/// feedback 5 with the operator at full level: the strongest feedback
/// that is still a steady, saw-like tone. Above it (feedback 6 and 7
/// near full level) the loop turns rough and noisy, as on the DX7.
const FB_CLEAN: f32 = 0.125;
/// The operator frequency (a fraction of the rate) up to which feedback
/// is left as it is: about C5 at 48 kHz for a ratio-1 operator.
const FB_X0: f32 = 0.011;
/// The feedback amount where the loop turns rough (about feedback 6 with
/// the operator two thirds up). From here on it is noise and whine more
/// than harmonics, as on the DX7, and is left as it is at every note.
const FB_ROUGH: f32 = 0.165;
/// How steeply the feedback climbs from the clean limit back to
/// [`FB_ROUGH`] at high notes: settings just past the clean limit stay
/// near it, and only those close to rough climb. It eases in over the
/// first [`FB_KNEE_IN`] of the easing, so nothing jumps as notes rise past
/// C5.
const FB_KNEE: f32 = 7.0;
const FB_KNEE_IN: f32 = 0.1;
/// How much of [`FB_CLEAN`] stays clean, at half-octave steps of the
/// operator's frequency from [`FB_X0`]: the most feedback whose harmonics
/// past Nyquist fold back at least 67 dB under the loudest, as measured on
/// the loop itself (`docs/dev/design/synths.md`, FM). Past the last step
/// nothing stays clean.
const FB_KEEP: [f32; 12] = [
    1.0, 0.9304, 0.8584, 0.7536, 0.652, 0.5168, 0.36, 0.2176, 0.136, 0.0536, 0.0008, 0.0,
];

/// The share of [`FB_CLEAN`] that stays clean for a loop whose highest
/// operator runs at `x` of the rate.
pub fn fb_keep(x: f32) -> f32 {
    if x <= FB_X0 {
        return 1.0;
    }
    let t = 2.0 * (x / FB_X0).log2();
    let i = t as usize;
    if i + 1 >= FB_KEEP.len() {
        return 0.0;
    }
    let f = t - i as f32;
    FB_KEEP[i] + (FB_KEEP[i + 1] - FB_KEEP[i]) * f
}

/// `ln` of the largest modulation index (radians) whose `n`th sideband,
/// `n` = 1 to 24, stays 60 dB under the carrier: `J_n(I) < 0.001`, from the
/// series' first term, `(I/2)^n / n! = 0.001` (which overstates `J_n` a
/// little, so the limit errs on the safe side). Past 24 it grows by
/// [`IDX_SLOPE`] a sideband.
const IDX_LN: [f32; 24] = [
    -6.2146, -2.4142, -1.0122, -0.2393, 0.2691, 0.6384, 0.9242, 1.1553, 1.348, 1.5128, 1.6563,
    1.7831, 1.8966, 1.9991, 2.0926, 2.1784, 2.2577, 2.3314, 2.4001, 2.4645, 2.5252, 2.5824, 2.6366,
    2.688,
];
const IDX_SLOPE: f32 = 0.7375;

/// The most a modulator at `xm` of the rate can drive an operator at `xd`
/// (as a gain, 1 at full level: a full modulator swings the phase two
/// cycles, an index of 4π) before its sidebands fold back past Nyquist
/// louder than 60 dB under the carrier. The first sideband to fold is the
/// `(0.5 - xd) / xm`th; up high a bright modulator so eases off, as the
/// feedback does ([`fb_amount`]), and at the middle of the keys nothing
/// changes (a ratio-14 tine is left alone up to about C4).
pub fn index_cap(xm: f32, xd: f32) -> f32 {
    if xm <= 0.0 {
        return f32::INFINITY;
    }
    let n = (0.5 - xd).max(0.0) / xm;
    let index = if n <= 1.0 {
        IDX_LN[0].exp() * n
    } else if n >= 24.0 {
        IDX_LN[23].exp() + IDX_SLOPE * (n - 24.0)
    } else {
        let k = n as usize;
        let f = n - k as f32;
        (IDX_LN[k - 1] + (IDX_LN[k] - IDX_LN[k - 1]) * f).exp()
    };
    index * (1.0 / (4.0 * std::f32::consts::PI))
}

/// The feedback amount for a loop at `x` of the rate, from the amount
/// `p` the settings ask for: as asked up to C5, and at higher notes as
/// asked up to what stays clean there ([`fb_keep`]). Past that it holds
/// near the clean limit and climbs steeply back to [`FB_ROUGH`], so a
/// rough setting stays rough (and as the DX7 plays it) and a feedback
/// swept by pressure never jumps.
pub fn fb_amount(p: f32, x: f32) -> f32 {
    let keep = fb_keep(x);
    let c = FB_CLEAN * keep;
    if keep >= 1.0 || p <= c || p >= FB_ROUGH {
        p
    } else {
        let span = FB_ROUGH - c;
        let knee = 1.0 + (FB_KNEE - 1.0) * ((1.0 - keep) / FB_KNEE_IN).min(1.0);
        c + span * ((p - c) / span).powf(knee)
    }
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
    /// What `dphase` was worked out from: the pitch's bits, the frequency
    /// fields and the rate. A step with the same inputs keeps it.
    fkey: [u32; 3],
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
    /// Its note's own slide and tone.
    ex: VoiceExpr,
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
            ex: VoiceExpr::default(),
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
    sine: Box<[f32; SIN_N + 1]>,
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
    /// The output's DC blocker (FM is mono: one for both sides) and its
    /// last output. Once no voice sounds, what is left in it fades out
    /// (`dc_fade`, by `dc_drain` a frame, about 3 ms) and it lets go.
    dc: DcBlock,
    dc_last: f32,
    dc_fade: f32,
    dc_drain: f32,
    idle: bool,
    stack: [u8; MAX_POLY],
    stack_len: usize,
    clock: u64,
    meter_voices: f32,
    /// How notes become frequencies.
    tuning: Tuning,
    /// Each note's slide and tone for its next note-on, and the voices'
    /// smoothing coefficient.
    exprs: NoteExprs,
    expr_c: f32,
    /// Notes that did not fit anywhere (should stay 0).
    pub dropped: u64,
}

const SIN_BITS: u32 = 12;
const SIN_N: usize = 1 << SIN_BITS;
const FRAC_BITS: u32 = 32 - SIN_BITS;

/// Whether an operator sounds across a run of frames (see `render`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Live {
    Never,
    Always,
    Check,
}

/// The phase offset for a modulation input in cycles: the low 32 bits of
/// `input * 2^32` truncated, as `(x as i64) as u32` gives. The i64
/// conversion is a library call on ARMv7, so a value below 2^31 in size
/// (in practice all of them) goes through i32, which keeps the same low
/// 32 bits; anything else (and NaN) still takes the i64 path.
#[inline]
fn phase_shift(input: f32) -> u32 {
    let x = input * 4_294_967_296.0;
    if x.abs() < 2_147_483_648.0 {
        x as i32 as u32
    } else {
        x as i64 as u32
    }
}

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
        let mut sine = Box::new([0.0; SIN_N + 1]);
        for (i, s) in sine.iter_mut().enumerate() {
            *s = (i as f64 / SIN_N as f64 * std::f64::consts::TAU).sin() as f32;
        }
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
            dc: DcBlock::new(rate, DC_HZ),
            dc_last: 0.0,
            dc_fade: 1.0,
            dc_drain: drain_coef(rate),
            idle: true,
            stack: [0; MAX_POLY],
            stack_len: 0,
            clock: 0,
            meter_voices: 0.0,
            tuning: Tuning::standard(),
            exprs: NoteExprs::new(),
            expr_c: 1.0,
            dropped: 0,
        };
        s.read_patch();
        s.prepare(rate, mpfree_node::MAX_BLOCK);
        s
    }

    /// Loads a factory preset: defaults, then its values. Sounding notes
    /// carry on.
    pub fn load_preset(&mut self, preset: &Preset) {
        self.load_values(preset.values);
    }

    /// Loads a DX7 voice ([`crate::dx7`]) the same way.
    pub fn load_voice(&mut self, voice: &crate::dx7::Dx7Voice) {
        self.load_values(&voice.values);
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

    /// The factory presets, read once from `presets/fm.presets`.
    pub fn presets() -> &'static [Preset] {
        crate::factory::presets!("fm.presets", &INFO, &PARAMS)
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
        self.update_slots();
    }

    /// Works out the pressure slots' targets and depths, for the shaper's
    /// mode as it stands (a slot's knob needs no new curve).
    fn update_slots(&mut self) {
        let slots: [Slot; 4] = std::array::from_fn(|n| Slot {
            target: self.v(press_target(n)) as u8,
            depth: self.v(press_depth(n)) / 100.0,
        });
        // Off: pressure does nothing, and a level swell plays at the preset's
        // full level (not at its resting dip).
        self.depths = if self.press_c.mode == PressureMode::Off {
            Depths::default()
        } else {
            Depths::from_slots(&slots)
        };
    }

    fn v(&self, id: u16) -> f32 {
        self.values[id as usize]
    }

    /// The whole patch and the pressure shaper from the values (a preset).
    fn read_patch(&mut self) {
        self.read_voice();
        self.update_pressure();
    }

    /// The operators, routing, level, LFO and voicing from the values: all
    /// but the pressure shaper, whose 128-step curve costs more than the
    /// rest together, so a knob that isn't a pressure setting skips it.
    fn read_voice(&mut self) {
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
        let trim = carrier_trim(&ops.map(|o| op_weight(&o)), route.carriers);
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
    }

    // ---- notes ----

    fn note_on(&mut self, note: u8, velocity: u8) {
        if velocity == 0 {
            self.note_off(note);
            return;
        }
        if self.dc_fade < 1.0 {
            // A note after silence: what was left in the blocker (fading,
            // far under hearing) goes, and the new note plays in full.
            self.dc.reset();
            self.dc_last = 0.0;
            self.dc_fade = 1.0;
        }
        self.idle = false;
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
                v.ex.retarget(self.exprs.on(note));
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
        let ex = self.exprs.on(note);
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
        v.ex.start(ex);
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
            self.fade_out(i);
        }
    }

    /// Fades voice `i` out over [`KILL_MS`], then frees it: a steal, or
    /// a voice given up under load.
    fn fade_out(&mut self, i: usize) {
        let n = (KILL_MS * 0.001 * self.rate as f32).max(1.0);
        let v = &mut self.voices[i];
        v.state = State::Kill;
        v.kill_gain = 1.0;
        v.kill_step = 1.0 / n;
    }

    /// The voice load shedding gives up first: the least audible one
    /// sounding ([`Cull::audibility`]). Voices already fading are left be.
    fn cull_pick(&self) -> Option<(usize, Cull)> {
        let mut best = None;
        for i in 0..POOL {
            let releasing = match self.voices[i].state {
                State::On => false,
                State::Release => true,
                State::Free | State::Kill => continue,
            };
            let level = self.voice_level(i);
            best = Cull::first(best, Some((i, Cull { level, releasing })));
        }
        best
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
        self.exprs.off(note);
        self.stack_remove(note);
        if self.patch.mono {
            let back = (self.stack_len > 0).then(|| self.stack[self.stack_len - 1]);
            let v = &mut self.voices[0];
            if v.state != State::On || v.note != note {
                return;
            }
            match back {
                Some(n) => {
                    v.ex.retarget(self.exprs.on(n));
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
        self.exprs.clear();
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
            NodeEvent::Expression { note, kind, value } => {
                // One note's own slide or tone: its held voices glide to it.
                let Some(value) = self.exprs.set(note, kind, value) else {
                    return;
                };
                for v in &mut self.voices {
                    if v.state == State::On && v.note == note {
                        v.ex.aim(kind, value);
                    }
                }
            }
            // The pedals are held by the host for this engine (`Node::hears`).
            NodeEvent::Control { .. } => {}
        }
    }

    // ---- control rate ----

    fn control(&mut self) {
        // Once every voice has ended, the blocker's tail fades out, and
        // under -120 dB it lets go, so a silent engine puts out exact zeros.
        self.idle = self.voices.iter().all(|v| v.state == State::Free);
        if self.idle && (self.dc_last * self.dc_fade).abs() < 1e-6 {
            self.dc.reset();
            self.dc_last = 0.0;
            // Nothing to fade: kept off the denormals.
            self.dc_fade = 0.0;
        }
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
        let tuning = &self.tuning;
        let v = &mut self.voices[i];
        let press = v.pv;
        if advance {
            v.ex.step(self.expr_c);
        }
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
        // Each operator's gain at the end of this step.
        let mut ends = [0.0f32; 6];
        let mut bent = pitch + bend;
        if v.ex.slide != 0.0 {
            bent += v.ex.slide;
        }
        for (j, o) in v.ops.iter_mut().enumerate() {
            let q = &p.ops[j];
            // The same inputs give the same increment, so a held note
            // skips the exp2 and the divides.
            let key = [
                if q.fixed { 0 } else { bent.to_bits() },
                0x8000_0000
                    | u32::from(q.fixed) << 24
                    | u32::from(q.coarse) << 16
                    | u32::from(q.fine) << 8
                    | u32::from(q.detune as u8),
                self.rate,
            ];
            if !same_key(&o.fkey, &key) {
                let hz = if q.fixed {
                    op_freq_in(tuning, 0.0, true, q.coarse, q.fine, q.detune)
                } else {
                    op_freq_in(tuning, bent, false, q.coarse, q.fine, q.detune)
                };
                o.dphase = ((hz / rate).clamp(0.0, 0.49) * 4_294_967_296.0) as u32;
                o.fkey = key;
            }
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
            ends[j] = target;
            // A carrier is done once it is near silent (-80 dB) and not climbing.
            if carrier && q.on && (target > 1e-4 || !o.env.falling()) {
                carriers_done = false;
            }
        }
        // Modulators ease off at high notes, where their sidebands would
        // fold back past Nyquist ([`index_cap`]): judged on each one's
        // highest target.
        for j in 0..6 {
            let t = p.route.targets[j];
            if t == 0 || ends[j] <= 0.0 {
                continue;
            }
            let xd = (0..6)
                .filter(|&k| t & (1 << k) != 0)
                .map(|k| v.ops[k].dphase)
                .max()
                .unwrap_or(0) as f32
                * (1.0 / 4_294_967_296.0);
            let cap = index_cap(v.ops[j].dphase as f32 * (1.0 / 4_294_967_296.0), xd);
            if ends[j] > cap {
                ends[j] = cap;
                let o = &mut v.ops[j];
                o.gstep = (cap - o.g) / CTRL as f32;
            }
        }
        // Feedback eases off at high notes, where its harmonics would fold
        // back past Nyquist ([`fb_amount`]): judged on the loop's highest
        // operator and the feedback operator's gain.
        if v.fb_scale > 0.0 {
            let r = p.route;
            let (lo, hi) = (r.fb_from.min(r.fb_to), r.fb_from.max(r.fb_to));
            let top = v.ops[lo..=hi].iter().map(|o| o.dphase).max().unwrap_or(0);
            let x = top as f32 * (1.0 / 4_294_967_296.0);
            let g = ends[r.fb_from];
            if g > 0.0 && x > FB_X0 {
                v.fb_scale = fb_amount(v.fb_scale * g, x) / g;
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
    fn sin(tab: &[f32; SIN_N + 1], ph: u32) -> f32 {
        let i = (ph >> FRAC_BITS) as usize;
        let f = (ph & ((1 << FRAC_BITS) - 1)) as f32 * (1.0 / (1u32 << FRAC_BITS) as f32);
        let a = tab[i];
        a + (tab[i + 1] - a) * f
    }

    /// Renders frames `from` to `to` (at most one control step) and adds
    /// them, through the DC blocker, to `left` and `right`.
    fn render(&mut self, from: usize, to: usize, left: &mut [f32], right: &mut [f32]) {
        let r = self.patch.route;
        let gain = VOICE_GAIN * self.out.value;
        let tab = &*self.sine;
        let n = to - from;
        debug_assert!(n <= CTRL as usize);
        let mut buf = [0.0f32; CTRL as usize];
        for v in &mut self.voices {
            if v.state == State::Free {
                continue;
            }
            // Which operators sound across this run. An operator's gain
            // step only changes at a control step, so one at zero with no
            // step stays silent, and one with a step (or above zero with
            // none) always sounds; anything else is checked every frame
            // as before.
            let mut live = [Live::Check; 6];
            for (l, o) in live.iter_mut().zip(&v.ops) {
                *l = if o.g == 0.0 && o.gstep == 0.0 {
                    Live::Never
                } else if o.gstep != 0.0 || o.g > 0.0 {
                    Live::Always
                } else {
                    Live::Check
                };
            }
            for x in &mut buf[..n] {
                let mut m = [0f32; 6];
                let mut out = 0.0;
                let fb_in = (v.fb[0] + v.fb[1]) * v.fb_scale;
                for j in (0..6).rev() {
                    let o = &mut v.ops[j];
                    let mut input = m[j];
                    if j == r.fb_to {
                        input += fb_in;
                    }
                    let sounds = match live[j] {
                        Live::Never => false,
                        Live::Always => true,
                        Live::Check => !(o.g == 0.0 && o.gstep == 0.0),
                    };
                    let y = if sounds {
                        Self::sin(tab, o.phase.wrapping_add(phase_shift(input))) * o.g
                    } else {
                        0.0
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
                *x += out * g;
                if v.state == State::Free {
                    break;
                }
            }
        }
        if self.idle {
            for (k, &x) in buf[..n].iter().enumerate() {
                self.dc_last = self.dc.tick(x);
                self.dc_fade *= self.dc_drain;
                let y = self.dc_last * self.dc_fade;
                left[from + k] += y;
                right[from + k] += y;
            }
        } else {
            for (k, &x) in buf[..n].iter().enumerate() {
                let y = self.dc.tick(x);
                left[from + k] += y;
                right[from + k] += y;
                self.dc_last = y;
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
        // Each knob reworks only what it feeds: a slot its target and
        // depth, a shaper setting the curve, anything else the voice.
        if id >= press_target(0) {
            self.update_slots();
        } else if id >= PRESS_BASE {
            self.update_pressure();
        } else {
            self.read_voice();
        }
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
        let old = self.rate;
        self.rate = rate.max(8_000);
        if self.rate != old {
            self.dc = DcBlock::new(self.rate, DC_HZ);
            self.dc_drain = drain_coef(self.rate);
        }
        let cr = self.rate / CTRL as u32;
        self.bend.set_time(cr, 4.0);
        self.expr_c = expr_coef(cr as f32);
        self.out.set_time(cr, 15.0);
        self.out.snap(self.out.target);
        self.update_pressure();
    }

    fn reset(&mut self) {
        for v in &mut self.voices {
            *v = Voice::new();
        }
        self.stack_len = 0;
        self.exprs.clear();
        self.lfo_phase = 0.0;
        self.lfo_time = 0.0;
        self.chord.clear();
        self.meter_voices = 0.0;
        self.bend.snap(0.0);
        self.dc.reset();
        self.dc_last = 0.0;
        self.dc_fade = 1.0;
        self.idle = true;
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

    fn set_tuning(&mut self, tuning: &Tuning) {
        if self.tuning == *tuning {
            return;
        }
        self.tuning.clone_from(tuning);
        // Every operator works its increment out again on the next step.
        for v in &mut self.voices {
            for o in &mut v.ops {
                o.fkey = [0; 3];
            }
        }
    }

    fn sounding_voices(&self) -> usize {
        self.active_voices()
    }

    fn cull_candidate(&self) -> Option<Cull> {
        self.cull_pick().map(|(_, c)| c)
    }

    fn cull(&mut self) -> bool {
        let Some((i, _)) = self.cull_pick() else {
            return false;
        };
        self.fade_out(i);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::{
        carrier_trim, fb_amount, fb_keep, index_cap, phase_shift, FB_CLEAN, FB_KNEE, FB_ROUGH,
        FB_X0,
    };

    #[test]
    fn a_modulator_eases_only_where_its_sidebands_would_fold() {
        let c4 = 261.63 / 48_000.0;
        // A 1:1 pair is left alone at full level up past C6.
        for semis in [0.0f32, 12.0, 24.0] {
            let x = c4 * 2f32.powf(semis / 12.0);
            assert!(index_cap(x, x) >= 1.0, "1:1 at {semis}");
        }
        // A ratio-14 tine at C4 keeps every level a preset gives it (Glass
        // Keys drives it at about 0.03), and is nearly gone by C7.
        assert!(index_cap(14.0 * c4, c4) > 0.15);
        assert!(index_cap(14.0 * 8.0 * c4, 8.0 * c4) < 1e-3);
        // Smooth and falling as the note rises, for any ratio.
        for ratio in [0.5f32, 1.0, 2.0, 3.5, 7.0, 14.0] {
            let mut x = 0.0005f32;
            let mut last = index_cap(ratio * x, x);
            while ratio * x < 0.49 {
                x *= 2f32.powf(1.0 / 1536.0);
                let cap = index_cap(ratio * x, x);
                assert!(cap <= last * 1.0001, "rises at {ratio}, {x}");
                assert!(
                    last - cap <= 0.02 * last + 1e-4,
                    "jumps at {ratio}, {x}: {last} to {cap}"
                );
                last = cap;
            }
        }
        assert_eq!(index_cap(0.0, 0.1), f32::INFINITY);
    }

    #[test]
    fn feedback_is_left_alone_up_to_c5_and_when_rough() {
        for k in 0..=100 {
            let p = 0.5 * k as f32 / 100.0;
            assert_eq!(fb_amount(p, FB_X0), p);
            assert_eq!(fb_amount(p, 0.001), p);
        }
        for x in [0.02f32, 0.05, 0.1, 0.2, 0.4] {
            for p in [FB_ROUGH, 0.25, 0.5] {
                assert_eq!(fb_amount(p, x), p, "{p} at {x}");
            }
        }
    }

    #[test]
    fn feedback_eases_smoothly_at_high_notes() {
        for x in [0.012f32, 0.02, 0.044, 0.1, 0.2, 0.4] {
            let c = FB_CLEAN * fb_keep(x);
            let mut last = 0.0;
            let step = 0.5 / 2000.0;
            for k in 0..=2000 {
                let p = step * k as f32;
                let q = fb_amount(p, x);
                // Never more than asked, never less as more is asked, and
                // no jumps: at most the knee's own slope.
                assert!(q <= p + 1e-7, "{q} > {p} at {x}");
                assert!(q >= last - 1e-7, "{q} < {last} at {p}, {x}");
                assert!(
                    q - last <= FB_KNEE * step + 1e-6,
                    "{last} to {q} at {p}, {x}"
                );
                if p <= c {
                    assert_eq!(q, p);
                }
                last = q;
            }
        }
        // Nor as the note rises, from below C5 to past the top.
        for k in 0..=50 {
            let p = FB_ROUGH * k as f32 / 50.0;
            let mut x = 0.005f32;
            let mut last = fb_amount(p, x);
            while x < 0.45 {
                x *= 2f32.powf(1.0 / 400.0);
                let q = fb_amount(p, x);
                assert!(
                    q <= last + 1e-7 && last - q < 2.0e-3,
                    "{last} to {q} at {p}, {x}"
                );
                last = q;
            }
        }
        // Feedback 5 at full level at C7 (48 kHz) holds near the clean limit.
        let x = 2093.0 / 48_000.0;
        let c = FB_CLEAN * fb_keep(x);
        assert!(c < 0.1 && fb_amount(FB_CLEAN, x) < c * 1.02);
        assert!(fb_keep(0.5) == 0.0 && fb_keep(FB_X0) == 1.0);
    }

    #[test]
    fn the_loudest_carrier_sets_the_level() {
        let w = |a: [f32; 6]| a;
        assert_eq!(carrier_trim(&w([0.5, 0.0, 0.0, 0.0, 0.0, 0.0]), 0b1), 1.0);
        let even = carrier_trim(&w([0.7; 6]), 0b11_1111);
        assert!((even - 1.0 / 6f32.sqrt()).abs() < 1e-6);
        // A quiet second carrier barely lowers the loud one.
        let t = carrier_trim(&w([1.0, 0.1, 0.0, 0.0, 0.0, 0.0]), 0b11);
        assert!((t - 1.0 / 1.01f32.sqrt()).abs() < 1e-6);
        // Only carriers count, and none at all leaves the voice as it is.
        assert_eq!(carrier_trim(&w([1.0, 1.0, 0.0, 0.0, 0.0, 0.0]), 0b01), 1.0);
        assert_eq!(carrier_trim(&w([0.0; 6]), 0b11), 1.0);
    }

    #[test]
    fn the_phase_shift_matches_the_i64_conversion_for_every_input() {
        let wide = |input: f32| (input * 4_294_967_296.0) as i64 as u32;
        let mut edge = vec![
            0.0,
            -0.0,
            f32::NAN,
            -f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::MIN,
            f32::MAX,
            f32::MIN_POSITIVE,
            f32::EPSILON,
        ];
        // Around the i32 edge (2^31 / 2^32 = 0.5) and the i64 edge.
        for c in [0.5f32, 1.0, 2.0, 2_147_483_648.0, 4_294_967_296.0] {
            let mut x = c;
            for _ in 0..4 {
                x = f32::from_bits(x.to_bits() - 1);
            }
            for _ in 0..9 {
                edge.extend([x, -x]);
                x = f32::from_bits(x.to_bits() + 1);
            }
        }
        for x in edge {
            assert_eq!(phase_shift(x), wide(x), "{x:e}");
        }
        // Every 997th bit pattern: every exponent and sign, NaNs included.
        for bits in (0..=u32::MAX).step_by(997) {
            let x = f32::from_bits(bits);
            assert_eq!(phase_shift(x), wide(x), "{bits:#x}");
        }
    }
}
