//! Perform FX: effects played live from pads. Hold a pad and its effect
//! starts on the next grid line; let go and the sound snaps back, while an
//! echo keeps ringing out. Sixteen slots, one per hardware pad on the
//! master; their pages are "Slot 1" to "Slot 16" (not "Pad", which on a
//! pad's own channel names the kit pad the chain belongs to).
//!
//! - **Time effects** take over the sound: stutter (loop what comes next),
//!   beat repeat (loop what just played), reverse, half speed, tape stop,
//!   vinyl brake and pitch drop. One plays at a time: the pad held last
//!   wins, and letting it go goes back to the one held before it (which
//!   starts again from that moment) or to the live sound.
//! - **Tone effects** stack on top, in pad order: trance gate, low-pass and
//!   high-pass sweeps, crush and a noise riser.
//! - **Echo throw** sends what you hear into a tempo-timed echo while the
//!   pad is held; the repeats ring out after it is let go.
//! - **Wash throw** sends it into a long, dark reverb while the pad is
//!   held; the tail rings out after. The reverb is a small one (four combs
//!   and two all-passes a side), awake only while it is fed or ringing.
//! - **Mix** ([`ids::MIX`]) sets how much of all this is heard: 0 is the
//!   input exactly, 100 % (the default) the effects alone.
//!
//! A pad is a note: `NoteOn` with note 0 to 15 holds slot 0 to 15,
//! `NoteOff` lets it go, and `AllOff` lets every slot go. Latching is the
//! caller's choice (it simply does not send the `NoteOff`).
//!
//! Each pad also has a **Hold** parameter ([`ids::hold`]): turning it on
//! holds the pad as a finger would, turning it off lets it go. A pad sounds
//! while a finger or its Hold holds it. That makes the pads parameters
//! like any other, so a step can lock a stutter on (a parameter lock) and
//! a recorded move can play a pad back.
//!
//! Everything is counted in frames from the moment an effect starts, and
//! the lengths are fixed (from the tempo) at that moment, so the output is
//! the same, sample for sample, however a stream is cut into blocks. Entry
//! waits for the next grid line only while the transport plays; when it is
//! stopped, effects start at once.

use mpfree_node::{Ctx, Curve, Kind, Node, NodeEvent, NodeInfo, ParamInfo, Smooth, Timed, Unit};

use crate::svf::{Svf, SvfMode};
use crate::{defaults, flush, Delay, DjFilter, Rng};

/// How many pads (slots) the bank has.
pub const SLOTS: usize = 16;

/// [`Perform`] parameter ids. Slot `k` has four, from [`ids::slot`]`(k)`:
/// effect, length, amount, and its read-only state.
pub mod ids {
    pub const QUANTISE: u16 = 0;
    pub const FADE: u16 = 1;
    /// Offsets inside a slot's four ids.
    pub const EFFECT: u16 = 0;
    pub const LENGTH: u16 = 1;
    pub const AMOUNT: u16 = 2;
    /// Read only: 0 off, 1 waiting for the grid, 2 on, 3 fading out.
    pub const STATE: u16 = 3;
    /// The first of slot `k`'s ids (`slot(k) + AMOUNT` is its amount).
    pub const fn slot(k: usize) -> u16 {
        2 + 4 * k as u16
    }
    /// After the pads: the DJ filter (-100 low-pass to 100 high-pass, off
    /// in the middle), the echo (0 to 100 %: how much goes into the echo,
    /// and its feedback with it) and the echo's time. The XY pad and the
    /// DJ knob play these.
    pub const DJ_FILTER: u16 = slot(super::SLOTS);
    pub const ECHO: u16 = DJ_FILTER + 1;
    pub const ECHO_TIME: u16 = DJ_FILTER + 2;
    /// Pad `k`'s Hold (a toggle): on holds the pad as a finger would, off
    /// lets it go (unless a finger still holds it). For parameter locks on
    /// steps and recorded moves; ids 69 to 84.
    pub const fn hold(k: usize) -> u16 {
        ECHO_TIME + 1 + k as u16
    }
    /// After the Holds: the overall mix, 0 to 100 % (0 is the input
    /// exactly, the echo and wash tails included); id 85.
    pub const MIX: u16 = hold(super::SLOTS);
}

/// A slot's state, as its read-only STATE parameter reports it.
pub mod state {
    pub const OFF: f32 = 0.0;
    pub const WAITING: f32 = 1.0;
    pub const ON: f32 = 2.0;
    pub const FADING: f32 = 3.0;
}

/// The effect names, in the order of a slot's EFFECT choice.
pub const EFFECTS: &[&str] = &[
    "Off",
    "Stutter",
    "Beat repeat",
    "Reverse",
    "Half speed",
    "Tape stop",
    "Vinyl brake",
    "Pitch drop",
    "Trance gate",
    "LP sweep",
    "HP sweep",
    "Crush",
    "Echo throw",
    "Noise riser",
    "Wash throw",
];

/// What a slot's Amount means for each effect, for the UI and the docs.
pub const AMOUNT_MEANS: &[&str] = &[
    "",
    "mix",
    "mix",
    "mix",
    "mix",
    "mix",
    "mix",
    "depth (100 % = two octaves down)",
    "depth",
    "how far the cutoff falls (100 % = ten octaves)",
    "how far the cutoff rises (100 % = ten octaves)",
    "bits and sample rate",
    "feedback",
    "level",
    "decay (100 % = about 5 seconds)",
];

const QUANTISE: &[&str] = &["Off", "1/32", "1/16", "1/8", "1/4", "1/2", "1 bar"];
const QUANTISE_BEATS: [f64; 7] = [0.0, 0.125, 0.25, 0.5, 1.0, 2.0, 4.0];

/// Note values for a slot's LENGTH choice.
pub const LENGTHS: &[&str] = &[
    "1/64", "1/32", "1/16T", "1/16", "1/8T", "1/16D", "1/8", "1/4T", "1/8D", "1/4", "1/2", "1 bar",
    "2 bars", "4 bars",
];
/// Beats per LENGTH choice.
pub const LENGTH_BEATS: [f64; 14] = [
    0.0625,
    0.125,
    1.0 / 6.0,
    0.25,
    1.0 / 3.0,
    0.375,
    0.5,
    2.0 / 3.0,
    0.75,
    1.0,
    2.0,
    4.0,
    8.0,
    16.0,
];

/// The longest loop (stutter, beat repeat, reverse, half speed), seconds:
/// 1 bar at 60 BPM, 2 bars at 120. Longer lengths are cut to this.
pub const MAX_LOOP_SECONDS: f32 = 4.0;
/// The longest fade, ms (the FADE parameter's top).
const MAX_FADE_MS: f32 = 30.0;
/// Coefficients (filters, crush, pitch) move every this many frames,
/// counted from the effect's start.
const STEP: u64 = 32;
/// The pitch drop's grain window, ms.
const GRAIN_MS: f32 = 30.0;

static INFO: NodeInfo = NodeInfo {
    id: "mpfree.fx.perform",
    name: "Perform FX",
    kind: Kind::Effect,
};

const fn global(
    id: u16,
    name: &'static str,
    max: f32,
    default: f32,
    unit: Unit,
    curve: Curve,
) -> ParamInfo {
    ParamInfo {
        id,
        name,
        group: "Perform",
        min: if matches!(curve, Curve::Log) {
            1.0
        } else {
            0.0
        },
        max,
        default,
        unit,
        curve,
        readonly: false,
    }
}

macro_rules! bank {
    ($( ($k:expr, $group:literal, $fx:expr, $len:expr, $amount:expr) ),* $(,)?) => {
        [
            global(ids::QUANTISE, "Punch in on", 6.0, 2.0, Unit::None, Curve::Choice(QUANTISE)),
            global(ids::FADE, "Fade", MAX_FADE_MS, 4.0, Unit::Ms, Curve::Log),
            $(
                ParamInfo {
                    id: ids::slot($k) + ids::EFFECT,
                    name: "Effect",
                    group: $group,
                    min: 0.0,
                    max: (EFFECTS.len() - 1) as f32,
                    default: $fx as u8 as f32,
                    unit: Unit::None,
                    curve: Curve::Choice(EFFECTS),
                    readonly: false,
                },
                ParamInfo {
                    id: ids::slot($k) + ids::LENGTH,
                    name: "Length",
                    group: $group,
                    min: 0.0,
                    max: (LENGTHS.len() - 1) as f32,
                    default: $len as f32,
                    unit: Unit::None,
                    curve: Curve::Choice(LENGTHS),
                    readonly: false,
                },
                ParamInfo {
                    id: ids::slot($k) + ids::AMOUNT,
                    name: "Amount",
                    group: $group,
                    min: 0.0,
                    max: 100.0,
                    default: $amount,
                    unit: Unit::Percent,
                    curve: Curve::Linear,
                    readonly: false,
                },
                ParamInfo {
                    id: ids::slot($k) + ids::STATE,
                    name: "State",
                    group: $group,
                    min: 0.0,
                    max: 3.0,
                    default: 0.0,
                    unit: Unit::None,
                    curve: Curve::Stepped,
                    readonly: true,
                },
            )*
            ParamInfo {
                id: ids::DJ_FILTER,
                name: "DJ filter",
                group: "Play",
                min: -100.0,
                max: 100.0,
                default: 0.0,
                unit: Unit::Percent,
                curve: Curve::Linear,
                readonly: false,
            },
            ParamInfo {
                id: ids::ECHO,
                name: "Echo",
                group: "Play",
                min: 0.0,
                max: 100.0,
                default: 0.0,
                unit: Unit::Percent,
                curve: Curve::Linear,
                readonly: false,
            },
            ParamInfo {
                id: ids::ECHO_TIME,
                name: "Echo time",
                group: "Play",
                min: 0.0,
                max: (LENGTHS.len() - 1) as f32,
                default: L_8D as f32,
                unit: Unit::None,
                curve: Curve::Choice(LENGTHS),
                readonly: false,
            },
            $(
                ParamInfo {
                    id: ids::hold($k),
                    name: "Hold",
                    group: $group,
                    min: 0.0,
                    max: 1.0,
                    default: 0.0,
                    unit: Unit::None,
                    curve: Curve::Toggle,
                    readonly: false,
                },
            )*
            ParamInfo {
                id: ids::MIX,
                name: "Mix",
                group: "Play",
                min: 0.0,
                max: 100.0,
                default: 100.0,
                unit: Unit::Percent,
                curve: Curve::Linear,
                readonly: false,
            },
        ]
    };
}

/// How many parameters: the two globals, four per pad, then the DJ filter,
/// the echo and its time, then a Hold per pad, then the mix.
const COUNT: usize = 2 + 4 * SLOTS + 3 + SLOTS + 1;
/// The ids from here on are not a pad's.
const PADS_END: u16 = ids::slot(SLOTS);
/// The first Hold id.
const HOLDS: u16 = ids::hold(0);

// Length choices by index, for the default bank below.
const L_32: u8 = 1;
const L_16: u8 = 3;
const L_8: u8 = 6;
const L_8D: u8 = 8;
const L_4: u8 = 9;
const L_2: u8 = 10;
const L_BAR: u8 = 11;
const L_4BARS: u8 = 13;

/// The default bank follows the Perform FX mockup and the Play page's
/// throws: stutters and repeats, stops, reverse, pitch, a gate, sweeps,
/// crush, a riser, an echo throw and a wash throw.
static PARAMS: [ParamInfo; COUNT] = bank![
    (0, "Slot 1", Fx::Stutter, L_16, 100.0),
    (1, "Slot 2", Fx::Repeat, L_8, 100.0),
    (2, "Slot 3", Fx::TapeStop, L_4, 100.0),
    (3, "Slot 4", Fx::Brake, L_2, 100.0),
    (4, "Slot 5", Fx::Reverse, L_4, 100.0),
    (5, "Slot 6", Fx::HalfSpeed, L_4, 100.0),
    (6, "Slot 7", Fx::PitchDrop, L_2, 50.0),
    (7, "Slot 8", Fx::Gate, L_16, 80.0),
    (8, "Slot 9", Fx::HpSweep, L_BAR, 70.0),
    (9, "Slot 10", Fx::Echo, L_8D, 55.0),
    (10, "Slot 11", Fx::LpSweep, L_BAR, 70.0),
    (11, "Slot 12", Fx::Crush, L_16, 80.0),
    (12, "Slot 13", Fx::Riser, L_4BARS, 60.0),
    (13, "Slot 14", Fx::Stutter, L_32, 100.0),
    (14, "Slot 15", Fx::Repeat, L_4, 100.0),
    (15, "Slot 16", Fx::Wash, L_BAR, 70.0),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fx {
    Off,
    Stutter,
    Repeat,
    Reverse,
    HalfSpeed,
    TapeStop,
    Brake,
    PitchDrop,
    Gate,
    LpSweep,
    HpSweep,
    Crush,
    Echo,
    Riser,
    Wash,
}

impl Fx {
    fn from(v: f32) -> Fx {
        use Fx::*;
        const ALL: [Fx; 15] = [
            Off, Stutter, Repeat, Reverse, HalfSpeed, TapeStop, Brake, PitchDrop, Gate, LpSweep,
            HpSweep, Crush, Echo, Riser, Wash,
        ];
        ALL[(v.max(0.0) as usize).min(ALL.len() - 1)]
    }
    fn is_time(self) -> bool {
        use Fx::*;
        matches!(
            self,
            Stutter | Repeat | Reverse | HalfSpeed | TapeStop | Brake | PitchDrop
        )
    }
    fn is_loop(self) -> bool {
        matches!(self, Fx::Stutter | Fx::Repeat | Fx::Reverse)
    }
    fn is_tone(self) -> bool {
        use Fx::*;
        matches!(self, Gate | LpSweep | HpSweep | Crush | Riser)
    }
}

/// The input's recent past, a power of two long, indexed by absolute
/// frame.
struct Ring {
    ch: [Vec<f32>; 2],
    mask: usize,
}

impl Ring {
    fn new(frames: usize) -> Ring {
        let n = frames.next_power_of_two().max(8);
        Ring {
            ch: [vec![0.0; n], vec![0.0; n]],
            mask: n - 1,
        }
    }
    #[inline]
    fn put(&mut self, f: i64, x: [f32; 2]) {
        let i = f as usize & self.mask;
        self.ch[0][i] = x[0];
        self.ch[1][i] = x[1];
    }
    #[inline]
    fn at(&self, f: i64) -> [f32; 2] {
        let i = f as usize & self.mask;
        [self.ch[0][i], self.ch[1][i]]
    }
    /// The input `age` frames before frame `f` (cubic Hermite; points in
    /// the future are held at `f`).
    #[inline]
    fn cubic(&self, f: i64, age: f64) -> [f32; 2] {
        let k = age as i64; // age >= 0, so this is floor
        let t = (age - k as f64) as f32;
        let p = |d: i64| f - d.max(0);
        let (a, b, c, d) = (
            self.at(p(k - 1)),
            self.at(p(k)),
            self.at(p(k + 1)),
            self.at(p(k + 2)),
        );
        let mut out = [0.0; 2];
        for ch in 0..2 {
            let (x0, x1, x2, x3) = (a[ch], b[ch], c[ch], d[ch]);
            let c1 = 0.5 * (x2 - x0);
            let c2 = x0 - 2.5 * x1 + 2.0 * x2 - 0.5 * x3;
            let c3 = 0.5 * (x3 - x0) + 1.5 * (x1 - x2);
            out[ch] = ((c3 * t + c2) * t + c1) * t + x1;
        }
        out
    }
    fn clear(&mut self) {
        for c in &mut self.ch {
            c.fill(0.0);
        }
    }
}

/// The time effect playing (or fading out).
#[derive(Clone, Copy, Debug)]
struct Play {
    slot: usize,
    fx: Fx,
    /// Its length in frames (loop, stop or glide time).
    len: u64,
    /// Seam fade, frames (at most a quarter of `len`).
    fade: u64,
    /// Loop effects: which loop buffer, the absolute frame of its index 0,
    /// how many of its frames were already in the past at the start, and
    /// its size (pre-roll, the loop, post-roll).
    buf: usize,
    src: i64,
    hist: u64,
    span: u64,
    /// Pitch drop: grain phase (frames) and speed.
    phase: f64,
    ratio: f64,
    /// Frames since the start, the frame inside the pass and the pass
    /// (counted, not divided: 64-bit division is a library call on the
    /// MPC's 32-bit ARM).
    t: u64,
    ph: u64,
    n: u64,
}

/// One pad.
#[derive(Clone, Copy, Debug)]
struct Slot {
    /// Waiting for the grid: starts at frame `at`.
    waiting: bool,
    at: i64,
    on: bool,
    seq: u32,
    fx: Fx,
    len: u64,
    amount: Smooth,
    /// Tone effects: the fade in and out (0 to 1).
    env: f32,
    svf: Svf,
    /// Crush: held values, levels and hold length; riser: level.
    held: [f32; 2],
    levels: f32,
    hold: u64,
    level: f32,
    /// Frames since the start, inside the gate's period and inside the
    /// crush's hold.
    t: u64,
    ph: u64,
    hold_ph: u64,
}

impl Slot {
    fn new(amount: f32) -> Slot {
        Slot {
            waiting: false,
            at: 0,
            on: false,
            seq: 0,
            fx: Fx::Off,
            len: 1,
            amount: Smooth::new(amount),
            env: 0.0,
            svf: Svf::default(),
            held: [0.0; 2],
            levels: 1.0,
            hold: 1,
            level: 0.0,
            t: 0,
            ph: 0,
            hold_ph: 0,
        }
    }
}

/// The wash throw's reverb: four damped feedback combs and two all-passes
/// a side (Schroeder and Moorer's reverb, with Freeverb's line lengths, the
/// right side's a little longer), fed with the mono sum. It costs a few
/// dozen operations a frame while awake and nothing asleep.
struct Wash {
    combs: [[Vec<f32>; 4]; 2],
    comb_at: [[usize; 4]; 2],
    store: [[f32; 4]; 2],
    aps: [[Vec<f32>; 2]; 2],
    ap_at: [[usize; 2]; 2],
    /// The combs' feedback (the decay).
    fb: f32,
}

/// Freeverb's comb and all-pass lengths at 44.1 kHz, and the right side's
/// extra length.
const WASH_COMBS: [usize; 4] = [1116, 1188, 1277, 1356];
const WASH_APS: [usize; 2] = [556, 441];
const WASH_SPREAD: usize = 23;
/// The combs' damping (a dark tail), the feed's and the return's gains.
const WASH_DAMP: f32 = 0.35;
const WASH_IN: f32 = 0.5;
const WASH_OUT: f32 = 0.8;
/// The wash sleeps once nothing goes in and what comes out has stayed
/// below this (-100 dB) for its longest comb.
const WASH_QUIET: f32 = 1e-5;

impl Wash {
    fn new() -> Wash {
        Wash {
            combs: Default::default(),
            comb_at: [[0; 4]; 2],
            store: [[0.0; 4]; 2],
            aps: Default::default(),
            ap_at: [[0; 2]; 2],
            fb: 0.0,
        }
    }

    fn prepare(&mut self, rate: u32) {
        let scale = |n: usize| ((n as f64 * rate as f64 / 44_100.0).round() as usize).max(1);
        for ch in 0..2 {
            let extra = ch * WASH_SPREAD;
            for (c, n) in WASH_COMBS.iter().enumerate() {
                self.combs[ch][c] = vec![0.0; scale(n + extra)];
            }
            for (a, n) in WASH_APS.iter().enumerate() {
                self.aps[ch][a] = vec![0.0; scale(n + extra)];
            }
        }
        self.clear();
    }

    fn clear(&mut self) {
        for ch in 0..2 {
            for l in &mut self.combs[ch] {
                l.fill(0.0);
            }
            for l in &mut self.aps[ch] {
                l.fill(0.0);
            }
        }
        self.comb_at = [[0; 4]; 2];
        self.ap_at = [[0; 2]; 2];
        self.store = [[0.0; 4]; 2];
    }

    /// Feedback for a slot's amount (0 to 1): 0.8 to 0.97.
    fn set_decay(&mut self, amount: f32) {
        self.fb = 0.8 + 0.17 * amount.clamp(0.0, 1.0);
    }

    /// The longest comb, frames.
    fn longest(&self) -> usize {
        self.combs[1][3].len()
    }

    /// Frames until a full tail has fallen below the quiet level.
    fn tail_frames(&self) -> u32 {
        let passes = (WASH_QUIET.ln() / self.fb.max(0.01).ln()).ceil().max(1.0);
        let ap: usize = self.aps[1].iter().map(Vec::len).sum();
        ((passes as f64 * self.longest() as f64) as usize + ap).min(u32::MAX as usize) as u32
    }

    /// One frame: `x` in, the two sides out.
    #[inline]
    fn frame(&mut self, x: f32) -> [f32; 2] {
        let mut out = [0.0; 2];
        for (ch, o) in out.iter_mut().enumerate() {
            let mut sum = 0.0;
            for c in 0..4 {
                let line = &mut self.combs[ch][c];
                let at = self.comb_at[ch][c];
                let y = line[at];
                let st = &mut self.store[ch][c];
                *st = flush(y + WASH_DAMP * (*st - y));
                line[at] = x + *st * self.fb;
                self.comb_at[ch][c] = if at + 1 == line.len() { 0 } else { at + 1 };
                sum += y;
            }
            let mut v = 0.25 * sum;
            for a in 0..2 {
                let line = &mut self.aps[ch][a];
                let at = self.ap_at[ch][a];
                let b = line[at];
                line[at] = flush(v + 0.5 * b);
                v = b - v;
                self.ap_at[ch][a] = if at + 1 == line.len() { 0 } else { at + 1 };
            }
            *o = v;
        }
        out
    }

    fn bytes(&self) -> usize {
        let combs: usize = self.combs.iter().flatten().map(Vec::len).sum();
        let aps: usize = self.aps.iter().flatten().map(Vec::len).sum();
        4 * (combs + aps)
    }
}

/// Sixteen pads of performance effects: stutter, beat repeat, reverse,
/// half speed, tape stop, vinyl brake, pitch drop, trance gate, filter
/// sweeps, crush, echo and wash throws and a noise riser, with quantised
/// entry and click-free release, and an overall mix.
pub struct Perform {
    v: [f32; COUNT],
    rate: u32,
    slots: [Slot; SLOTS],
    seq: u32,
    /// The earliest waiting slot's start frame (`i64::MAX` when none).
    next_at: i64,
    ring: Ring,
    loops: [[Vec<f32>; 2]; 2],
    loop_max: u64,
    cur: Option<Play>,
    prev: Option<Play>,
    xf_left: u64,
    xf_len: u64,
    /// Something was engaged or let go: pick the time effect again.
    dirty: bool,
    /// The slots whose tone effect is on or fading out.
    tone_mask: u16,
    /// Pads held by a note (a finger) and by their Hold parameter; a pad
    /// sounds while either holds it.
    by_note: u16,
    by_param: u16,
    /// Hold changes from `set_param` (which has no frame), applied at the
    /// start of the next block: which pads, and on or off.
    pend: u16,
    pend_on: u16,
    rng: Rng,
    /// The echo: the delay, its send level, which slots hold it, its
    /// current time and feedback, and whether it is awake.
    echo: Delay,
    echo_env: f32,
    echo_on: u16,
    echo_last: usize,
    echo_ms: f32,
    echo_fb: f32,
    echo_awake: bool,
    echo_quiet: u64,
    send: [Vec<f32>; 2],
    /// The continuous echo send (the ECHO parameter), gliding.
    echo_amt: Smooth,
    /// The DJ filter at the end of the path, before the echo's send.
    dj: DjFilter,
    /// The wash: its reverb, its send level, which slots hold it, the
    /// newest, whether it is awake and frames it has been quiet.
    wash: Wash,
    wash_env: f32,
    wash_on: u16,
    wash_last: usize,
    wash_awake: bool,
    wash_quiet: usize,
    /// The overall mix, gliding, and each frame's value this block (the
    /// echo, added later, is scaled by it too).
    mix: Smooth,
    mixv: Vec<f32>,
}

impl Default for Perform {
    fn default() -> Self {
        Self::new()
    }
}

impl Perform {
    /// Allocates for 48 kHz; [`Node::prepare`] reallocates for other rates.
    pub fn new() -> Perform {
        let v = defaults(&PARAMS);
        let mut slots = [Slot::new(0.0); SLOTS];
        for (k, s) in slots.iter_mut().enumerate() {
            s.amount = Smooth::new(v[(ids::slot(k) + ids::AMOUNT) as usize] / 100.0);
        }
        let mut echo = Delay::new();
        echo.set_param(crate::ids::delay::SYNC, 0.0);
        echo.set_param(crate::ids::delay::MIX, 100.0);
        echo.set_param(crate::ids::delay::PING_PONG, 1.0);
        let mut p = Perform {
            v,
            rate: 0,
            slots,
            seq: 0,
            next_at: i64::MAX,
            ring: Ring::new(8),
            loops: [[Vec::new(), Vec::new()], [Vec::new(), Vec::new()]],
            loop_max: 1,
            cur: None,
            prev: None,
            xf_left: 0,
            xf_len: 1,
            dirty: false,
            tone_mask: 0,
            by_note: 0,
            by_param: 0,
            pend: 0,
            pend_on: 0,
            rng: Rng::new(0x9E21),
            echo,
            echo_env: 0.0,
            echo_on: 0,
            echo_last: 0,
            echo_ms: f32::NAN,
            echo_fb: f32::NAN,
            echo_awake: false,
            echo_quiet: u64::MAX,
            send: [
                vec![0.0; mpfree_node::MAX_BLOCK],
                vec![0.0; mpfree_node::MAX_BLOCK],
            ],
            echo_amt: Smooth::new(0.0),
            dj: DjFilter::new(),
            wash: Wash::new(),
            wash_env: 0.0,
            wash_on: 0,
            wash_last: 0,
            wash_awake: false,
            wash_quiet: 0,
            mix: Smooth::new(1.0),
            mixv: vec![1.0; mpfree_node::MAX_BLOCK],
        };
        p.prepare(48_000, mpfree_node::MAX_BLOCK);
        p
    }

    /// Bytes this node holds for audio (the ring, two loop buffers, the
    /// echo's and the wash's lines), for the memory budget.
    pub fn memory_bytes(&self) -> usize {
        let ring = 2 * self.ring.ch[0].len();
        let loops = 4 * self.loops[0][0].len();
        let send = 2 * self.send[0].len() + self.mixv.len();
        let echo = 2 * (MAX_LOOP_SECONDS * self.rate as f32) as usize;
        4 * (ring + loops + send + echo) + self.wash.bytes()
    }

    fn fade_frames(&self) -> u64 {
        ((self.v[ids::FADE as usize] * 0.001 * self.rate as f32) as u64).max(1)
    }

    fn apply(&mut self, id: u16, value: f32) {
        let Some(p) = PARAMS.get(id as usize) else {
            return;
        };
        if p.readonly {
            return;
        }
        let v = p.clamp(value);
        self.v[id as usize] = v;
        match id {
            ids::DJ_FILTER => self.dj.set_param(crate::ids::dj_filter::POSITION, v),
            ids::ECHO => self.echo_amt.target = v / 100.0,
            ids::MIX => self.mix.target = v / 100.0,
            2.. if id < PADS_END && (id - 2) % 4 == ids::AMOUNT => {
                self.slots[(id as usize - 2) / 4].amount.target = v / 100.0;
            }
            _ => {}
        }
    }

    /// Holds a pad: works out the frame it starts on (the next grid line
    /// while the transport plays).
    fn press(&mut self, ctx: &Ctx, k: usize, offset: usize, f: i64) {
        let q = QUANTISE_BEATS[(self.v[ids::QUANTISE as usize] as usize).min(6)];
        let at = if q <= 0.0 || !ctx.playing || ctx.bpm <= 0.0 {
            f
        } else {
            let per_frame = ctx.bpm / 60.0 / self.rate as f64;
            let b = ctx.beats + offset as f64 * per_frame;
            let grid = ((b - 0.5 * per_frame) / q).ceil() * q;
            f + ((grid - b) / per_frame).round().max(0.0) as i64
        };
        let s = &mut self.slots[k];
        if s.on {
            // A press on a held pad starts it again.
            self.release(k);
        }
        let s = &mut self.slots[k];
        s.waiting = true;
        s.at = at;
        self.next_at = self.next_at.min(at);
    }

    fn release(&mut self, k: usize) {
        let s = &mut self.slots[k];
        s.waiting = false;
        if s.on {
            s.on = false;
            if s.fx == Fx::Echo {
                self.echo_on &= !(1 << k);
            }
            if s.fx == Fx::Wash {
                self.wash_on &= !(1 << k);
            }
            self.dirty = true;
        }
    }

    fn event(&mut self, ctx: &Ctx, ev: &NodeEvent, offset: usize, f: i64) {
        match *ev {
            NodeEvent::Param { id, value } if (HOLDS..HOLDS + SLOTS as u16).contains(&id) => {
                self.hold(ctx, (id - HOLDS) as usize, value >= 0.5, offset, f);
            }
            NodeEvent::Param { id, value } => self.apply(id, value),
            NodeEvent::NoteOn { note, velocity } if (note as usize) < SLOTS => {
                let k = note as usize;
                if velocity == 0 {
                    self.note_off(k);
                } else {
                    self.by_note |= 1 << k;
                    self.press(ctx, k, offset, f);
                }
            }
            NodeEvent::NoteOff { note } if (note as usize) < SLOTS => self.note_off(note as usize),
            NodeEvent::AllOff => self.all_off(),
            _ => {}
        }
    }

    fn note_off(&mut self, k: usize) {
        self.by_note &= !(1 << k);
        if self.by_param & (1 << k) == 0 {
            self.release(k);
        }
    }

    /// Lets every pad go, fingers and Holds alike.
    fn all_off(&mut self) {
        for k in 0..SLOTS {
            self.release(k);
            self.v[ids::hold(k) as usize] = 0.0;
        }
        self.by_note = 0;
        self.by_param = 0;
        self.pend = 0;
    }

    /// Pad `k`'s Hold turned on or off: it presses the pad unless a finger
    /// already holds it, and lets it go unless a finger still does.
    fn hold(&mut self, ctx: &Ctx, k: usize, on: bool, offset: usize, f: i64) {
        let bit = 1 << k;
        self.v[ids::hold(k) as usize] = if on { 1.0 } else { 0.0 };
        if on == (self.by_param & bit != 0) {
            return;
        }
        if on {
            self.by_param |= bit;
            if self.by_note & bit == 0 {
                self.press(ctx, k, offset, f);
            }
        } else {
            self.by_param &= !bit;
            if self.by_note & bit == 0 {
                self.release(k);
            }
        }
    }

    /// Starts the slots whose grid line has come.
    fn engage_due(&mut self, ctx: &Ctx, f: i64, piece: &mut usize, i: usize, out: [&mut [f32]; 2]) {
        let bpm = if ctx.bpm > 0.0 { ctx.bpm } else { 120.0 };
        let mut next = i64::MAX;
        let mut echo_started = None;
        for k in 0..SLOTS {
            let s = &mut self.slots[k];
            if !s.waiting {
                continue;
            }
            if s.at > f {
                next = next.min(s.at);
                continue;
            }
            let fx_v = self.v[(ids::slot(k) + ids::EFFECT) as usize];
            let len_v = self.v[(ids::slot(k) + ids::LENGTH) as usize] as usize;
            let beats = LENGTH_BEATS[len_v.min(LENGTH_BEATS.len() - 1)];
            self.seq = self.seq.wrapping_add(1);
            s.waiting = false;
            s.on = true;
            s.seq = self.seq;
            s.fx = Fx::from(fx_v);
            s.len = ((beats * 60.0 / bpm * self.rate as f64).round() as u64).max(4);
            s.amount.snap(s.amount.target);
            s.t = 0;
            s.ph = 0;
            s.hold_ph = 0;
            if s.fx.is_tone() {
                if s.env <= 0.0 {
                    s.svf.reset();
                    s.held = [0.0; 2];
                }
                self.tone_mask |= 1 << k;
            } else {
                // A tone effect still fading on this pad stops here.
                self.tone_mask &= !(1 << k);
                s.env = 0.0;
            }
            if s.fx == Fx::Echo {
                self.echo_on |= 1 << k;
                self.echo_last = k;
                echo_started = Some(());
            }
            if s.fx == Fx::Wash {
                self.wash_on |= 1 << k;
                self.wash_last = k;
                self.wash_awake = true;
                self.wash_quiet = 0;
                self.wash
                    .set_decay(self.v[(ids::slot(k) + ids::AMOUNT) as usize] / 100.0);
            }
            self.dirty = true;
        }
        self.next_at = next;
        if echo_started.is_some() {
            // The delay takes the new time from this frame on.
            let [l, r] = out;
            self.wake_echo(ctx, piece, i, l, r);
        }
    }

    /// Wakes the echo at frame `i` and gives it its time and feedback from
    /// there on.
    fn wake_echo(&mut self, ctx: &Ctx, piece: &mut usize, i: usize, l: &mut [f32], r: &mut [f32]) {
        self.flush_echo(ctx, *piece, i, l, r);
        *piece = i;
        self.echo_awake = true;
        self.echo_quiet = 0;
        self.sync_echo(ctx);
    }

    /// Gives the delay its time and feedback: the newest held throw's, or
    /// else the ECHO parameter's while it is up (else they stay, so a
    /// ringing tail keeps its time).
    fn sync_echo(&mut self, ctx: &Ctx) {
        let (len, fb) = if self.echo_on != 0 {
            let k = self.echo_last;
            (
                self.slots[k].len as f64,
                self.v[(ids::slot(k) + ids::AMOUNT) as usize] * 0.95,
            )
        } else if self.echo_amt.target > 0.0 {
            let bpm = if ctx.bpm > 0.0 { ctx.bpm } else { 120.0 };
            let choice = (self.v[ids::ECHO_TIME as usize] as usize).min(LENGTH_BEATS.len() - 1);
            let len = LENGTH_BEATS[choice] * 60.0 / bpm * self.rate as f64;
            (len.round(), 25.0 + 60.0 * self.echo_amt.target)
        } else {
            return;
        };
        let ms = (len * 1000.0 / self.rate as f64) as f32;
        if ms != self.echo_ms {
            self.echo_ms = ms;
            self.echo.set_param(crate::ids::delay::TIME, ms);
        }
        if fb != self.echo_fb {
            self.echo_fb = fb;
            self.echo.set_param(crate::ids::delay::FEEDBACK, fb);
        }
    }

    /// Runs the echo over frames `from..to` of the block (if it is awake)
    /// and adds it to the output.
    fn flush_echo(&mut self, ctx: &Ctx, from: usize, to: usize, l: &mut [f32], r: &mut [f32]) {
        if !self.echo_awake || to <= from {
            return;
        }
        let [sl, sr] = &mut self.send;
        self.echo
            .process(ctx, &[], &mut sl[from..to], &mut sr[from..to]);
        for k in from..to {
            let m = self.mixv[k];
            if m == 1.0 {
                l[k] += sl[k];
                r[k] += sr[k];
            } else {
                l[k] += m * sl[k];
                r[k] += m * sr[k];
            }
        }
    }

    /// Picks the time effect: the held pad pressed last.
    fn pick(&mut self, f: i64) {
        self.dirty = false;
        let mut best: Option<usize> = None;
        for (k, s) in self.slots.iter().enumerate() {
            if s.on && s.fx.is_time() && best.is_none_or(|b| s.seq > self.slots[b].seq) {
                best = Some(k);
            }
        }
        if best == self.cur.map(|p| p.slot) {
            return;
        }
        let fade = self.fade_frames();
        let prev = self.cur;
        let buf = match prev {
            Some(p) if p.fx.is_loop() => 1 - p.buf,
            _ => 0,
        };
        self.cur = best.map(|k| {
            let s = &self.slots[k];
            let cap = match s.fx {
                Fx::TapeStop | Fx::Brake => 2 * self.loop_max,
                Fx::PitchDrop => u64::MAX,
                _ => self.loop_max,
            };
            let len = s.len.min(cap).max(4);
            let seam = fade.min(len / 4).max(1);
            let back = if s.fx == Fx::Stutter { 0 } else { len as i64 };
            Play {
                slot: k,
                fx: s.fx,
                len,
                fade: seam,
                buf,
                src: f - back - seam as i64,
                hist: back as u64 + seam,
                span: len + 2 * seam,
                phase: 0.0,
                ratio: 1.0,
                t: 0,
                ph: 0,
                n: 0,
            }
        });
        self.prev = prev;
        self.xf_len = fade;
        self.xf_left = fade;
    }

    fn tail(&self) -> u32 {
        let mut t = self.fade_frames() as u32;
        if self.echo_awake {
            t = t.max(self.echo.tail_frames());
        }
        if self.wash_awake {
            t = t.max(self.wash.tail_frames());
        }
        t
    }
}

/// One frame of a time effect.
#[inline(always)]
fn voice(
    p: &mut Play,
    f: i64,
    x: [f32; 2],
    ring: &Ring,
    loops: &mut [[Vec<f32>; 2]; 2],
    amount: f32,
    rate: u32,
) -> [f32; 2] {
    let (t, ph, n) = (p.t, p.ph, p.n);
    let (len, seam) = (p.len, p.fade);
    p.t += 1;
    p.ph += 1;
    if p.ph == len {
        p.ph = 0;
        p.n += 1;
    }
    let y = match p.fx {
        Fx::Stutter | Fx::Repeat | Fx::Reverse => {
            // Copy one frame of the past and the frame arriving now.
            let b = &mut loops[p.buf];
            if t < p.hist {
                let v = ring.at(p.src + t as i64);
                b[0][t as usize] = v[0];
                b[1][t as usize] = v[1];
            }
            let live = p.hist + t;
            if live < p.span {
                b[0][live as usize] = x[0];
                b[1][live as usize] = x[1];
            }
            let b = &loops[p.buf];
            // Past frames not copied yet are still in the ring.
            let read = |j: u64| -> [f32; 2] {
                if j < p.hist && j > t {
                    ring.at(p.src + j as i64)
                } else {
                    [b[0][j as usize], b[1][j as usize]]
                }
            };
            let rev = p.fx == Fx::Reverse;
            let base = read(if rev { seam + len - 1 - ph } else { seam + ph });
            if ph + seam >= len {
                // Fade toward what comes before the loop's start (after
                // its end, played backward), so the wrap is seamless.
                let k = ph + seam - len;
                let other = read(if rev { seam + len + (seam - 1 - k) } else { k });
                let w = (k + 1) as f32 / (seam + 1) as f32;
                [
                    base[0] + w * (other[0] - base[0]),
                    base[1] + w * (other[1] - base[1]),
                ]
            } else {
                base
            }
        }
        Fx::HalfSpeed => {
            let y = ring.cubic(f, ph as f64 * 0.5);
            if n > 0 && ph < seam {
                let old = ring.cubic(f, (len + ph) as f64 * 0.5);
                let w = (ph + 1) as f32 / (seam + 1) as f32;
                [old[0] + w * (y[0] - old[0]), old[1] + w * (y[1] - old[1])]
            } else {
                y
            }
        }
        Fx::TapeStop | Fx::Brake => {
            if t >= len {
                [0.0; 2]
            } else {
                let (t64, d) = (t as f64, len as f64);
                let u = t64 / d;
                let age = if p.fx == Fx::TapeStop {
                    0.5 * t64 * u
                } else {
                    let r = 1.0 - u;
                    t64 - d / 3.0 * (1.0 - r * r * r)
                };
                // The last tenth fades out, so the stop never clicks.
                let g = ((d - t64) / (0.1 * d)).min(1.0) as f32;
                let y = ring.cubic(f, age);
                [g * y[0], g * y[1]]
            }
        }
        Fx::PitchDrop => {
            let w = (GRAIN_MS * 0.001 * rate as f32) as f64;
            if t.is_multiple_of(STEP) {
                let u = (t as f64 / len as f64).min(1.0);
                p.ratio = (-24.0 * amount as f64 * u / 12.0).exp2();
            }
            let a1 = p.phase;
            let a2 = if a1 + 0.5 * w >= w {
                a1 - 0.5 * w
            } else {
                a1 + 0.5 * w
            };
            let g1 = (1.0 - (2.0 * a1 / w - 1.0).abs()) as f32;
            let (y1, y2) = (ring.cubic(f, a1), ring.cubic(f, a2));
            p.phase += 1.0 - p.ratio;
            if p.phase >= w {
                p.phase -= w;
            }
            [y2[0] + g1 * (y1[0] - y2[0]), y2[1] + g1 * (y1[1] - y2[1])]
        }
        _ => x,
    };
    if p.fx == Fx::PitchDrop {
        y
    } else {
        [x[0] + amount * (y[0] - x[0]), x[1] + amount * (y[1] - x[1])]
    }
}

/// One frame of a tone effect, in place.
#[inline]
fn tone(s: &mut Slot, y: &mut [f32; 2], rng: &mut Rng, rate: u32, ramp: u64) {
    let t = s.t;
    let ph = s.ph;
    s.t += 1;
    s.ph += 1;
    if s.ph >= s.len {
        s.ph = 0;
    }
    let amount = s.amount.next();
    let env = s.env;
    match s.fx {
        Fx::Gate => {
            let len = s.len;
            let half = len / 2;
            let r = ramp.min(len / 8).max(1);
            let g = if ph < r {
                ph as f32 / r as f32
            } else if ph < half {
                1.0
            } else if ph < half + r {
                1.0 - (ph - half) as f32 / r as f32
            } else {
                0.0
            };
            let gain = 1.0 - amount * env * (1.0 - g);
            y[0] *= gain;
            y[1] *= gain;
        }
        Fx::LpSweep | Fx::HpSweep => {
            if t.is_multiple_of(STEP) {
                let u = (t as f32 / s.len as f32).min(1.0);
                let oct = 10.0 * amount * u;
                let (mode, hz) = if s.fx == Fx::LpSweep {
                    (SvfMode::LowPass, 20_000.0 * (-oct).exp2())
                } else {
                    (SvfMode::HighPass, 20.0 * oct.exp2())
                };
                s.svf.set(mode, hz, 0.9, 0.0, rate);
            }
            for (c, v) in y.iter_mut().enumerate() {
                let w = s.svf.run(c, *v);
                *v += env * (w - *v);
            }
        }
        Fx::Crush => {
            if t.is_multiple_of(STEP) {
                s.levels = (15.0 - 12.0 * amount).exp2();
                s.hold = ((3.0 * amount).exp2().round() as u64).max(1);
            }
            if s.hold_ph == 0 {
                for (h, v) in s.held.iter_mut().zip(y.iter()) {
                    *h = (*v * s.levels).round() / s.levels;
                }
            }
            s.hold_ph += 1;
            if s.hold_ph >= s.hold {
                s.hold_ph = 0;
            }
            for (v, h) in y.iter_mut().zip(s.held) {
                *v += env * (h - *v);
            }
        }
        Fx::Riser => {
            if t.is_multiple_of(STEP) {
                let u = (t as f32 / s.len as f32).min(1.0);
                s.svf
                    .set(SvfMode::BandPass, 300.0 * (5.0 * u).exp2(), 2.0, 0.0, rate);
                s.level = 0.5 * amount * u * u;
            }
            for (c, v) in y.iter_mut().enumerate() {
                let n = s.svf.run(c, rng.bipolar());
                *v += env * s.level * n;
            }
        }
        _ => {}
    }
}

impl Node for Perform {
    fn info(&self) -> &'static NodeInfo {
        &INFO
    }
    fn params(&self) -> &'static [ParamInfo] {
        &PARAMS
    }
    fn set_param(&mut self, id: u16, value: f32) {
        if (HOLDS..HOLDS + SLOTS as u16).contains(&id) {
            let bit = 1 << (id - HOLDS);
            self.pend |= bit;
            if value >= 0.5 {
                self.pend_on |= bit;
            } else {
                self.pend_on &= !bit;
            }
            self.v[id as usize] = if value >= 0.5 { 1.0 } else { 0.0 };
            return;
        }
        self.apply(id, value);
    }
    fn param(&self, id: u16) -> f32 {
        if (2..PADS_END).contains(&id) && (id - 2) % 4 == ids::STATE {
            let k = (id as usize - 2) / 4;
            return match self.slots.get(k) {
                None => f32::NAN,
                Some(s) if s.waiting => state::WAITING,
                Some(s) if s.on => state::ON,
                Some(s) if s.fx.is_tone() && s.env > 0.0 => state::FADING,
                Some(_) => state::OFF,
            };
        }
        self.v.get(id as usize).copied().unwrap_or(f32::NAN)
    }
    fn prepare(&mut self, rate: u32, _max_block: usize) {
        let rate = rate.max(1);
        if rate != self.rate {
            self.rate = rate;
            let fmax = (MAX_FADE_MS * 0.001 * rate as f32) as usize + 1;
            let loop_max = (MAX_LOOP_SECONDS * rate as f32) as usize;
            // Reverse reads up to two loops back; the copy reads one loop
            // and a fade back.
            self.ring = Ring::new(2 * loop_max + 4 * fmax + 64);
            self.loop_max = loop_max as u64;
            let span = loop_max + 2 * fmax + 8;
            self.loops = [
                [vec![0.0; span], vec![0.0; span]],
                [vec![0.0; span], vec![0.0; span]],
            ];
            self.echo.prepare(rate, mpfree_node::MAX_BLOCK);
            self.wash.prepare(rate);
        }
        self.dj.prepare(rate, mpfree_node::MAX_BLOCK);
        self.dj.set_param(
            crate::ids::dj_filter::POSITION,
            self.v[ids::DJ_FILTER as usize],
        );
        self.echo_amt.set_time(rate, 20.0);
        self.mix.set_time(rate, 20.0);
        for s in &mut self.slots {
            s.amount.set_time(rate, 20.0);
        }
        self.reset();
    }
    fn reset(&mut self) {
        let fresh = [Slot::new(0.0); SLOTS];
        for (k, s) in self.slots.iter_mut().enumerate() {
            let amount = s.amount;
            *s = fresh[k];
            s.amount = amount;
            s.amount.snap(amount.target);
        }
        self.seq = 0;
        self.next_at = i64::MAX;
        self.ring.clear();
        for b in &mut self.loops {
            for c in b {
                c.fill(0.0);
            }
        }
        self.cur = None;
        self.prev = None;
        self.xf_left = 0;
        self.dirty = false;
        self.tone_mask = 0;
        self.by_note = 0;
        self.by_param = 0;
        self.pend = 0;
        for k in 0..SLOTS {
            self.v[ids::hold(k) as usize] = 0.0;
        }
        self.rng = Rng::new(0x9E21);
        self.echo.reset();
        self.echo_env = 0.0;
        self.echo_on = 0;
        self.echo_awake = false;
        self.echo_quiet = u64::MAX;
        self.echo_amt.snap(self.echo_amt.target);
        self.dj.reset();
        self.wash.clear();
        self.wash_env = 0.0;
        self.wash_on = 0;
        self.wash_awake = false;
        self.wash_quiet = 0;
        self.mix.snap(self.mix.target);
    }
    fn process(&mut self, ctx: &Ctx, events: &[Timed], left: &mut [f32], right: &mut [f32]) {
        let len = left.len().min(right.len()).min(self.send[0].len());
        let base = ctx.frame as i64;
        let fade = self.fade_frames();
        let step = 1.0 / fade as f32;
        let ramp = (0.002 * self.rate as f32) as u64;
        let end = base + len as i64;
        if self.pend != 0 {
            let (pend, on) = (self.pend, self.pend_on);
            self.pend = 0;
            for k in 0..SLOTS {
                if pend & (1 << k) != 0 {
                    self.hold(ctx, k, on & (1 << k) != 0, 0, base);
                }
            }
        }
        if events.is_empty()
            && self.cur.is_none()
            && self.xf_left == 0
            && self.tone_mask == 0
            && !self.echo_awake
            && !self.dirty
            && self.next_at >= end
            && self.dj.resting()
            && self.echo_amt.target == 0.0
            && !self.wash_awake
            && self.mix.settled()
        {
            // Nothing playing: the input only goes into the ring (the same
            // output, frame for frame, as the full path below).
            for i in 0..len {
                let x = [left[i], right[i]];
                self.ring.put(base + i as i64, x);
                left[i] = flush(x[0]);
                right[i] = flush(x[1]);
            }
            self.echo_quiet = self.echo_quiet.saturating_add(len as u64);
            return;
        }
        let mut e = 0;
        let mut piece = 0;
        for i in 0..len {
            let f = base + i as i64;
            if (f as u64).is_multiple_of(STEP) {
                self.flush_echo(ctx, piece, i, left, right);
                piece = i;
                if self.echo_awake {
                    self.sync_echo(ctx);
                }
                if self.echo_awake && self.echo_quiet > self.echo.tail_frames() as u64 {
                    self.echo_awake = false;
                }
            }
            while e < events.len() && events[e].offset as usize <= i {
                let ev = events[e].ev;
                self.event(ctx, &ev, i, f);
                e += 1;
            }
            if f >= self.next_at {
                self.engage_due(ctx, f, &mut piece, i, [&mut left[..], &mut right[..]]);
            }
            if !self.echo_awake && self.echo_amt.target > 0.0 {
                self.wake_echo(ctx, &mut piece, i, left, right);
            }
            if self.dirty {
                self.pick(f);
            }

            let x = [left[i], right[i]];
            self.ring.put(f, x);

            // The time effect, crossfaded from the one before.
            let Perform {
                ring,
                loops,
                cur,
                prev,
                slots,
                rate,
                ..
            } = self;
            let mut y = match cur {
                Some(p) => {
                    let a = slots[p.slot].amount.next();
                    voice(p, f, x, ring, loops, a, *rate)
                }
                None => x,
            };
            if self.xf_left > 0 {
                let old = match prev {
                    Some(p) => {
                        let a = slots[p.slot].amount.next();
                        voice(p, f, x, ring, loops, a, *rate)
                    }
                    None => x,
                };
                let w = (self.xf_len - self.xf_left + 1) as f32 / (self.xf_len + 1) as f32;
                y = [old[0] + w * (y[0] - old[0]), old[1] + w * (y[1] - old[1])];
                self.xf_left -= 1;
                if self.xf_left == 0 {
                    self.prev = None;
                }
            }

            // Tone effects, in pad order.
            let mut m = self.tone_mask;
            while m != 0 {
                let k = m.trailing_zeros() as usize;
                m &= m - 1;
                let s = &mut self.slots[k];
                s.env = if s.on {
                    (s.env + step).min(1.0)
                } else {
                    (s.env - step).max(0.0)
                };
                tone(s, &mut y, &mut self.rng, self.rate, ramp);
                if !s.on && s.env <= 0.0 {
                    self.tone_mask &= !(1 << k);
                }
            }

            // The DJ filter (resting in the middle, it passes the frame as
            // it is; its resonance never moves here, so skipping the call
            // changes nothing).
            if !self.dj.resting() {
                let (a, b) = self.dj.frame(y[0], y[1]);
                y = [a, b];
            }

            // The echo's send follows the held throws and the ECHO knob.
            if self.echo_on != 0 {
                self.echo_env = (self.echo_env + step).min(1.0);
            } else if self.echo_env > 0.0 {
                self.echo_env = (self.echo_env - step).max(0.0);
            }
            let amt = self.echo_amt.next();
            let send = self.echo_env.max(amt);
            if send > 0.0 || self.echo_on != 0 {
                self.echo_quiet = 0;
            } else {
                self.echo_quiet = self.echo_quiet.saturating_add(1);
            }
            if self.echo_awake {
                self.send[0][i] = y[0] * send;
                self.send[1][i] = y[1] * send;
            }

            // The wash: fed while a throw holds it, ringing on after.
            if self.wash_awake {
                if self.wash_on != 0 {
                    self.wash_env = (self.wash_env + step).min(1.0);
                } else if self.wash_env > 0.0 {
                    self.wash_env = (self.wash_env - step).max(0.0);
                }
                let feed = self.wash_env * WASH_IN * 0.5 * (y[0] + y[1]);
                let w = self.wash.frame(feed);
                if self.wash_env > 0.0 || w[0].abs() > WASH_QUIET || w[1].abs() > WASH_QUIET {
                    self.wash_quiet = 0;
                } else {
                    self.wash_quiet += 1;
                    if self.wash_quiet > self.wash.longest() {
                        self.wash_awake = false;
                    }
                }
                y[0] += WASH_OUT * w[0];
                y[1] += WASH_OUT * w[1];
            }

            // The mix: 0 is the input exactly (the echo, added after, is
            // scaled the same way in `flush_echo`).
            let m = self.mix.next();
            self.mixv[i] = m;
            if m == 1.0 {
                left[i] = flush(y[0]);
                right[i] = flush(y[1]);
            } else {
                let d = [flush(x[0]), flush(x[1])];
                left[i] = d[0] + m * (flush(y[0]) - d[0]);
                right[i] = d[1] + m * (flush(y[1]) - d[1]);
            }
        }
        self.flush_echo(ctx, piece, len, left, right);
        // Events at or past the end land on the next block's first frame.
        let f = base + len as i64;
        for t in &events[e..] {
            self.event(ctx, &t.ev, len, f);
        }
    }
    fn tail_frames(&self) -> u32 {
        self.tail()
    }
}
