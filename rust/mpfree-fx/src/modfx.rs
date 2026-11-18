//! Modulation effects: [`Chorus`], [`Flanger`] and [`Phaser`].
//!
//! They share an LFO that runs free (in Hz) or synced to the tempo (1/16
//! to 8 bars a cycle, and dotted and triplet eighths and quarters), a sine
//! or a triangle with rounded corners. A synced LFO starts on the bar when the transport
//! starts and follows the song while it plays: after a jump (a loop wrap,
//! a locate) it glides back onto the bar over 50 ms, so a swept delay
//! never steps. Width offsets the right side's LFO phase, so 100 % is a
//! wide stereo sweep and 0 % keeps both sides moving together. Everything
//! that can move glides, so turning a knob under a loop never clicks or
//! zips; a change of voice or stage count crossfades.
//!
//! The LFO is a polynomial sine (or a triangle of parabolas and lines) and
//! the read tables are built at compile
//! time, so a render is the same on every machine (the phaser's tangent
//! aside). The chorus and the flanger read their delays through an 8-tap
//! windowed sinc that sounds the same at any fraction of a frame, so a
//! sweep never flutters the highs and the flanger's high notches stay
//! deep round its feedback loop.

use std::f32::consts::PI;

use mpfree_node::{Ctx, Curve, Kind, Node, NodeEvent, NodeInfo, ParamInfo, Smooth, Timed, Unit};

use crate::kit::{self, Blend, Glide};
use crate::svf::Segs;
use crate::{defaults, flush, lerp};

/// The LFO's sync choices. The triplet and dotted ones were appended
/// (FX quality PR M4), so saved projects keep the choice they had.
const SYNCS: &[&str] = &[
    "Free", "1/16", "1/8", "1/4", "1/2", "1 bar", "2 bars", "4 bars", "8 bars", "1/8T", "1/8D",
    "1/4T", "1/4D",
];
/// Beats per LFO cycle for each sync choice (0 is free).
const SYNC_BEATS: [f64; 13] = [
    0.0,
    0.25,
    0.5,
    1.0,
    2.0,
    4.0,
    8.0,
    16.0,
    32.0,
    1.0 / 3.0,
    0.75,
    2.0 / 3.0,
    1.5,
];

/// The LFO's shapes (appended to all three effects by FX quality PR M4;
/// Sine, the default, is the shape saved projects had).
const SHAPES: &[&str] = &["Sine", "Triangle"];
/// How far either side of each of the triangle's corners is rounded, in
/// cycles: the sweep turns over a tenth of a cycle, so a chorus's bend
/// swings through instead of flipping.
const TRI_ROUND: f32 = 0.05;
/// How long a change of shape takes, ms: a crossfade of the two shapes,
/// so the sweep never steps.
const SHAPE_MS: f32 = 50.0;

/// A sine of `phase` cycles (0 to 1): a parabola with a cubic correction
/// (error under 0.1 %), plain arithmetic, so it is the same everywhere.
#[inline]
fn sine(phase: f32) -> f32 {
    let x = if phase < 0.5 { phase } else { phase - 1.0 };
    let y = 8.0 * x - 16.0 * x * x.abs();
    0.225 * (y * y.abs() - y) + y
}

/// `phase + off` (never negative), wrapped into 0 to 1. Truncation is floor
/// here, with no library call.
#[inline]
fn wrap(p: f32) -> f32 {
    p - (p as u32) as f32
}

/// A triangle of `phase` cycles (0 to 1) in step with [`sine`] (0 at 0,
/// 1 at a quarter, -1 at three quarters), each corner rounded by a
/// parabola [`TRI_ROUND`] either side, so its slope never steps; the
/// straight parts slope 4 / (1 - 2 TRI_ROUND) a cycle, under a sine's 2 pi.
#[inline]
fn triangle(phase: f32) -> f32 {
    const PEAK: f32 = 1.0 - 2.0 * TRI_ROUND;
    let x = if phase < 0.5 { phase } else { phase - 1.0 };
    let d = (x.abs() - 0.25).abs();
    let y = if d < TRI_ROUND {
        PEAK - 2.0 / TRI_ROUND * d * d
    } else {
        1.0 - 4.0 * d
    };
    (y / PEAK).copysign(x)
}

/// The LFO at `phase` cycles (0 to 1), `tri` of the way from the sine to
/// the triangle: 0 or 1, but while a change of shape crossfades.
#[inline]
fn wave(phase: f32, tri: f32) -> f32 {
    if tri == 0.0 {
        sine(phase)
    } else if tri == 1.0 {
        triangle(phase)
    } else {
        lerp(sine(phase), triangle(phase), tri)
    }
}

/// Cubic Hermite (Catmull-Rom) weights at `t` (0 to 1) of the way from
/// `x1` to `x2`, for `x3`, `x2`, `x1` and `x0` in that order (oldest
/// first, as the flat read's taps are).
#[inline]
fn hermite_weights(t: f32) -> [f32; 4] {
    let (t2, t3) = (t * t, t * t * t);
    [
        0.5 * (t3 - t2),
        0.5 * t + 2.0 * t2 - 1.5 * t3,
        1.0 - 2.5 * t2 + 1.5 * t3,
        -0.5 * t + t2 - 0.5 * t3,
    ]
}

/// How long a synced LFO takes at least to catch up with the song after a
/// jump (a loop wrap, a locate): long enough that a delay never steps,
/// short enough that the sweep is back on the bar within the beat. An
/// effect can ask for a slower catch-up (the chorus does, so a deep sweep
/// doesn't chirp).
const RELOCK_MS: f64 = 50.0;

/// The modulation LFO.
#[derive(Clone, Copy, Debug)]
struct Lfo {
    /// Cycles, 0 to 1 (f64, so a long free run never drifts or rounds).
    phase: f64,
    inc: f64,
    bpm: f64,
    playing: bool,
    /// A catch-up in progress (synced, after a jump): this much more
    /// phase each frame for `fix_left` more frames.
    fix_step: f64,
    fix_left: u32,
    /// The shape, 0 (sine) to 1 (triangle), gliding over [`SHAPE_MS`],
    /// and its value this frame.
    tri: Smooth,
    tri_now: f32,
}

impl Default for Lfo {
    fn default() -> Lfo {
        Lfo {
            phase: 0.0,
            inc: 0.0,
            bpm: 120.0,
            playing: false,
            fix_step: 0.0,
            fix_left: 0,
            tri: Smooth::new(0.0),
            tri_now: 0.0,
        }
    }
}

impl Lfo {
    /// At the start of a block: follow the tempo and, when synced, the
    /// song position. The phase lands on the bar as the transport starts;
    /// while it plays, a jump in the song (a loop wrap, a locate) is
    /// caught up over [`RELOCK_MS`], or slower if that would take more
    /// than `max_fix` cycles of extra phase a frame, so the swept delay
    /// glides instead of stepping and the sweep stays on the bar.
    fn begin(&mut self, ctx: &Ctx, sync: usize, max_fix: f64) {
        if ctx.bpm > 0.0 {
            self.bpm = ctx.bpm;
        }
        if sync > 0 && ctx.playing {
            let want = (ctx.beats / SYNC_BEATS[sync]).rem_euclid(1.0);
            if !self.playing {
                self.phase = want;
                self.fix_left = 0;
            } else {
                // How far the song is from the phase, the short way round.
                let gap = (want - self.phase + 0.5).rem_euclid(1.0) - 0.5;
                let pending = self.fix_step * f64::from(self.fix_left);
                if (gap - pending).abs() > 1e-6 {
                    // A new jump (or one the catch-up in progress doesn't
                    // cover): catch all of it up over the whole time.
                    let least = RELOCK_MS * 0.001 * f64::from(ctx.rate);
                    let frames = least.max(gap.abs() / max_fix).min(f64::from(ctx.rate));
                    self.fix_left = (frames.ceil() as u32).max(1);
                    self.fix_step = gap / f64::from(self.fix_left);
                } else if self.fix_left == 0 {
                    // Rounding between the tempo and the frame count.
                    self.phase = want;
                }
            }
        }
        self.playing = ctx.playing;
    }
    fn set(&mut self, hz: f32, sync: usize, rate: u32) {
        let cps = if sync > 0 {
            self.bpm / 60.0 / SYNC_BEATS[sync]
        } else {
            f64::from(hz)
        };
        self.inc = cps / f64::from(rate.max(1));
    }
    /// Aims the shape: the rounded triangle, or the sine.
    fn set_shape(&mut self, triangle: bool) {
        self.tri.target = if triangle { 1.0 } else { 0.0 };
    }
    fn prepare(&mut self, rate: u32) {
        self.tri.set_time(rate, SHAPE_MS);
    }
    /// The LFO's values at several phases (cycles, any from 0 up), in
    /// this frame's shape. The shape is picked once for all of them, so
    /// the sines are worked out side by side (the chorus's eight sweeps
    /// cost less this way than its two voices' four did one at a time).
    #[inline]
    fn waves<const N: usize>(&self, at: [f32; N]) -> [f32; N] {
        let tri = self.tri_now;
        if tri == 0.0 {
            at.map(|p| sine(wrap(p)))
        } else if tri == 1.0 {
            at.map(|p| triangle(wrap(p)))
        } else {
            at.map(|p| lerp(sine(wrap(p)), triangle(wrap(p)), tri))
        }
    }
    #[inline]
    fn next(&mut self) -> f32 {
        if self.tri.value != self.tri.target {
            self.tri_now = self.tri.next();
        }
        self.phase += self.inc;
        if self.fix_left > 0 {
            self.phase += self.fix_step;
            self.fix_left -= 1;
        }
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        } else if self.phase < 0.0 {
            self.phase += 1.0;
        }
        self.phase as f32
    }
    fn reset(&mut self) {
        self.phase = 0.0;
        self.playing = false;
        self.fix_left = 0;
        self.tri.snap(self.tri.target);
        self.tri_now = self.tri.value;
    }
}

/// Taps in [`FlatLine::read`]: the read point sits between the 4th and
/// 5th.
const TAPS: usize = 8;
/// Fractions of a frame [`READ_TABLE`] holds; reads between them blend
/// the two nearest rows.
const PHASES: usize = 64;
/// The windowed sinc's cutoff (in cycles a frame, Nyquist is 0.5) and its
/// Kaiser window's beta, chosen so every fraction sounds the same: within
/// 0.16 dB of each other to 16 kHz at 44.1 kHz, so a sweep never flutters
/// the highs. The read tilts the top a little, the same at any fraction
/// (0.3 dB at 14 kHz, about 1 dB at 16; the cubic lost 3.4 dB at 15 kHz
/// at half a frame). A higher cutoff droops less but varies more.
const SINC_CUTOFF: f64 = 0.47;
const KAISER_BETA: f64 = 4.0;

/// `sin(x)`, plain arithmetic (the table is built at compile time).
pub(crate) const fn const_sin(x: f64) -> f64 {
    let tau = 2.0 * std::f64::consts::PI;
    // Into -pi to pi.
    let mut x = x - tau * ((x / tau + 0.5) as i64 as f64);
    if x > std::f64::consts::PI {
        x -= tau;
    } else if x < -std::f64::consts::PI {
        x += tau;
    }
    let mut term = x;
    let mut sum = x;
    let mut k = 1;
    while k < 30 {
        term *= -x * x / ((2 * k) as f64 * (2 * k + 1) as f64);
        sum += term;
        k += 1;
    }
    sum
}

/// The zeroth-order modified Bessel function, for the Kaiser window.
pub(crate) const fn const_i0(x: f64) -> f64 {
    let mut term = 1.0;
    let mut sum = 1.0;
    let mut k = 1;
    while k < 25 {
        let h = x / (2.0 * k as f64);
        term *= h * h;
        sum += term;
        k += 1;
    }
    sum
}

/// `sqrt(x)` for 0 to 1, by Newton's method.
pub(crate) const fn const_sqrt(x: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    let mut y = 1.0;
    let mut k = 0;
    while k < 30 {
        y = 0.5 * (y + x / y);
        k += 1;
    }
    y
}

/// The read's taps at a fraction `t` (0 to 1) of a frame past a whole
/// delay: a Kaiser-windowed sinc, summing to 1 so the lows pass exactly.
const fn sinc_taps(t: f64) -> [f64; TAPS] {
    let half = (TAPS / 2) as f64;
    let mut h = [0.0; TAPS];
    let mut sum = 0.0;
    let mut j = 0;
    while j < TAPS {
        // Tap j is the sample (j - 3) frames further back than the whole
        // delay, `tau` frames from the read point.
        let tau = (j as f64 - (half - 1.0)) - t;
        let x = tau / half;
        // Less its value at the ends, so the end taps are zero and the
        // taps don't jump as the read crosses a whole frame.
        let w = const_i0(KAISER_BETA * const_sqrt(1.0 - x * x)) - 1.0;
        let arg = std::f64::consts::PI * 2.0 * SINC_CUTOFF * tau;
        let s = if arg == 0.0 {
            1.0
        } else {
            const_sin(arg) / arg
        };
        h[j] = 2.0 * SINC_CUTOFF * s * w;
        sum += h[j];
        j += 1;
    }
    let mut j = 0;
    while j < TAPS {
        h[j] /= sum;
        j += 1;
    }
    h
}

/// Each fraction's taps, oldest sample first, and the step to the next
/// fraction's.
struct ReadTable {
    taps: [[f32; TAPS]; PHASES],
    step: [[f32; TAPS]; PHASES],
}

const fn read_table() -> ReadTable {
    let mut rows = [[0.0f64; TAPS]; PHASES + 1];
    let mut r = 0;
    while r <= PHASES {
        rows[r] = sinc_taps(r as f64 / PHASES as f64);
        r += 1;
    }
    let mut t = ReadTable {
        taps: [[0.0; TAPS]; PHASES],
        step: [[0.0; TAPS]; PHASES],
    };
    let mut r = 0;
    while r < PHASES {
        let mut j = 0;
        while j < TAPS {
            // Tap j is the (j - 3)th sample past the whole delay, so the
            // oldest is the last.
            let k = TAPS - 1 - j;
            t.taps[r][k] = rows[r][j] as f32;
            t.step[r][k] = (rows[r + 1][j] - rows[r][j]) as f32;
            j += 1;
        }
        r += 1;
    }
    t
}

/// Built at compile time, so it is the same on every machine.
static READ_TABLE: ReadTable = read_table();

/// The flat read's eight weights for a point `d` frames before the last
/// write, and its whole part `i`: weight `k` is for the sample `i + 4 - k`
/// frames back (oldest first), as [`FlatLine::read_with`] takes them.
///
/// From 4 frames back it is the windowed sinc of [`FlatLine::read`]. In a
/// feedback loop the sinc's newest taps would be samples not yet written
/// nearer the write than that, so from 4 frames down to 3 the read fades
/// to a cubic (Catmull-Rom), which holds from there down to 1 frame. Only
/// a flanger's shortest sweeps (under about 0.1 ms) get there, where its
/// comb's first notch is above 5 kHz. `longest` is the line's room.
#[inline]
fn flat_weights(d: f32, longest: f32) -> (usize, [f32; TAPS]) {
    let d = d.clamp(1.0, longest);
    let i = d as usize;
    let t = d - i as f32;
    let mut w = [0.0f32; TAPS];
    if i >= 3 {
        let p = t * PHASES as f32;
        let r = (p as usize).min(PHASES - 1);
        let a = p - r as f32;
        let (taps, step) = (&READ_TABLE.taps[r], &READ_TABLE.step[r]);
        for k in 0..TAPS {
            w[k] = taps[k] + a * step[k];
        }
        if i >= 4 {
            return (i, w);
        }
        // 3 to 4 frames back: from the cubic (at 3) to the sinc (at 4).
        for v in &mut w {
            *v *= t;
        }
    }
    let c = hermite_weights(t);
    let share = if i >= 3 { 1.0 - t } else { 1.0 };
    for k in 0..4 {
        // The cubic's samples are i + 2 down to i - 1 frames back.
        w[k + 2] += share * c[k];
    }
    (i, w)
}

/// A delay line read through the windowed sinc: as bright at any fraction
/// of a frame as at a whole one, so a moving read doesn't flutter the
/// highs. Every sample is written twice, a line's length apart, so the
/// taps of any read sit side by side.
#[derive(Clone, Debug, Default)]
struct FlatLine {
    buf: Vec<f32>,
    /// The line's length, a power of two (`buf` holds two).
    n: usize,
    w: usize,
}

impl FlatLine {
    /// Room for `frames` of delay (control thread).
    fn new(frames: usize) -> FlatLine {
        let n = (frames + TAPS).next_power_of_two();
        FlatLine {
            buf: vec![0.0; 2 * n],
            n,
            w: 0,
        }
    }
    #[inline]
    fn push(&mut self, x: f32) {
        self.buf[self.w] = x;
        self.buf[self.w + self.n] = x;
        self.w = (self.w + 1) & (self.n - 1);
    }
    /// The signal `d` frames before the last write (0 is the last write),
    /// `d` at least half of [`TAPS`].
    #[inline]
    fn read(&self, d: f32) -> f32 {
        let d = d.clamp((TAPS / 2) as f32, (self.n - TAPS) as f32);
        let i = d as usize;
        let p = (d - i as f32) * PHASES as f32;
        let r = (p as usize).min(PHASES - 1);
        let a = p - r as f32;
        let (taps, step) = (&READ_TABLE.taps[r], &READ_TABLE.step[r]);
        // The oldest tap is i + 4 frames before the last write.
        let from = (self.w + self.n - 1 - i - TAPS / 2) & (self.n - 1);
        let x = &self.buf[from..from + TAPS];
        // Four lanes, then their sum: the same order everywhere, and one
        // vector add wide.
        let mut lane = [0.0f32; 4];
        for k in 0..4 {
            let lo = (taps[k] + a * step[k]) * x[k];
            let hi = (taps[k + 4] + a * step[k + 4]) * x[k + 4];
            lane[k] = lo + hi;
        }
        (lane[0] + lane[2]) + (lane[1] + lane[3])
    }
    /// The read with the weights [`flat_weights`] gives, for a point `i`
    /// whole frames (and a fraction) before the last write. Weights for
    /// samples not yet written are zero.
    #[inline]
    fn read_with(&self, i: usize, w: &[f32; TAPS]) -> f32 {
        let from = (self.w + self.n - 1 - i - TAPS / 2) & (self.n - 1);
        let x = &self.buf[from..from + TAPS];
        let mut lane = [0.0f32; 4];
        for k in 0..4 {
            lane[k] = w[k] * x[k] + w[k + 4] * x[k + 4];
        }
        (lane[0] + lane[2]) + (lane[1] + lane[3])
    }
    /// The furthest back a read can reach, in frames.
    fn longest(&self) -> f32 {
        (self.n - TAPS) as f32
    }
    fn clear(&mut self) {
        self.buf.fill(0.0);
        self.w = 0;
    }
}

/// The response of a read with `w` (from [`flat_weights`], whole part `i`)
/// that comes one frame after the last write, at `rad` radians a frame:
/// tap `k` is `i + 5 - k` frames before the frame read.
fn flat_response(i: usize, w: &[f32; TAPS], rad: f64) -> (f64, f64) {
    let (mut re, mut im) = (0.0f64, 0.0f64);
    for (k, &v) in w.iter().enumerate() {
        let a = rad * (i + 5 - k) as f64;
        re += f64::from(v) * a.cos();
        im -= f64::from(v) * a.sin();
    }
    (re, im)
}

fn smooth_all(list: &mut [&mut Smooth], rate: u32, ms: f32) {
    for s in list {
        s.set_time(rate, ms);
    }
}

// ---------------------------------------------------------------- chorus

/// [`Chorus`] parameter ids.
pub mod chorus {
    pub const RATE: u16 = 0;
    pub const SYNC: u16 = 1;
    pub const DEPTH: u16 = 2;
    pub const DELAY: u16 = 3;
    pub const VOICES: u16 = 4;
    pub const WIDTH: u16 = 5;
    pub const TONE: u16 = 6;
    pub const MIX: u16 = 7;
    /// How Mix blends: Equal (equal power), Dry first or Linear
    /// ([`crate::kit::MIX_CURVES`]), as on the delays and reverbs.
    pub const MIX_CURVE: u16 = 8;
    /// The LFO's shape, Sine or a rounded Triangle (FX quality PR M4).
    pub const SHAPE: u16 = 9;
}

static CHORUS_INFO: NodeInfo = NodeInfo {
    id: "mpfree.fx.chorus",
    name: "Chorus",
    kind: Kind::Effect,
};
static CHORUS_PARAMS: [ParamInfo; 10] = [
    ParamInfo {
        id: 0,
        name: "Rate",
        group: "LFO",
        min: 0.05,
        max: 10.0,
        default: 0.6,
        unit: Unit::Hz,
        curve: Curve::Log,
        readonly: false,
    },
    ParamInfo {
        id: 1,
        name: "Sync",
        group: "LFO",
        min: 0.0,
        max: 12.0,
        default: 0.0,
        unit: Unit::None,
        curve: Curve::Choice(SYNCS),
        readonly: false,
    },
    ParamInfo {
        id: 2,
        name: "Depth",
        group: "LFO",
        min: 0.0,
        max: 100.0,
        default: 40.0,
        unit: Unit::Percent,
        curve: Curve::Linear,
        readonly: false,
    },
    ParamInfo {
        id: 3,
        name: "Delay",
        group: "Chorus",
        min: 2.0,
        max: 40.0,
        default: 12.0,
        unit: Unit::Ms,
        curve: Curve::Log,
        readonly: false,
    },
    ParamInfo {
        id: 4,
        name: "Voices",
        group: "Chorus",
        min: 1.0,
        max: 4.0,
        default: 2.0,
        unit: Unit::None,
        curve: Curve::Stepped,
        readonly: false,
    },
    ParamInfo {
        id: 5,
        name: "Width",
        group: "Chorus",
        min: 0.0,
        max: 100.0,
        default: 100.0,
        unit: Unit::Percent,
        curve: Curve::Linear,
        readonly: false,
    },
    ParamInfo {
        id: 6,
        name: "Tone",
        group: "Output",
        min: 1_000.0,
        max: 20_000.0,
        default: 9_000.0,
        unit: Unit::Hz,
        curve: Curve::Log,
        readonly: false,
    },
    ParamInfo {
        id: 7,
        name: "Mix",
        group: "Output",
        min: 0.0,
        max: 100.0,
        default: 50.0,
        unit: Unit::Percent,
        curve: Curve::Linear,
        readonly: false,
    },
    ParamInfo {
        id: 8,
        name: "Mix curve",
        group: "Output",
        min: 0.0,
        max: 2.0,
        default: 0.0,
        unit: Unit::None,
        curve: Curve::Choice(kit::MIX_CURVES),
        readonly: false,
    },
    ParamInfo {
        id: 9,
        name: "Shape",
        group: "LFO",
        min: 0.0,
        max: 1.0,
        default: 0.0,
        unit: Unit::None,
        curve: Curve::Choice(SHAPES),
        readonly: false,
    },
];

/// LFO phase offsets per voice: any count of voices starts from the same
/// ones, so adding a voice never moves the others.
const VOICE_PHASE: [f32; 4] = [0.0, 0.5, 0.25, 0.75];
/// How far the right side's voices sit from the left's at full Width, in
/// cycles, by voice count: a quarter for one or two voices, an eighth for
/// three or four, so the right's voices fall between the left's and the
/// sides differ at every count.
const RIGHT_OFFSET: [f32; 4] = [0.25, 0.25, 0.125, 0.125];
/// How long the right side's voices take to move to their new places
/// when the voice count changes: slow enough that the pitch barely bends.
const SPREAD_MS: f32 = 250.0;
/// The sweep at Depth 100 %, ms either side of the Delay.
const DEPTH_MS: f32 = 5.0;
/// The sweep never reaches past this share of the Delay.
const DEPTH_OF_DELAY: f32 = 0.9;
/// The steepest the swept delay may change, frames a frame: its pitch
/// bend stays inside a semitone (a semitone down is 1 - 2^(-1/12), 0.0561,
/// with a little room for the LFO's shape).
const PITCH_CEILING: f32 = 0.055;
const MAX_CHORUS_MS: f32 = 46.0;

/// A lush stereo chorus: up to four voices reading a delay swept by one
/// LFO at spread phases, the right side's voices between the left's,
/// and a tone control to keep the wet signal warm. Depth is a sweep in
/// milliseconds (100 % is 5 ms either way), so the Delay moves where the
/// copies sit and not how much they wobble, and the bend never passes a
/// semitone, however fast the Rate.
pub struct Chorus {
    v: [f32; 10],
    rate: u32,
    lfo: Lfo,
    lines: [FlatLine; 2],
    /// The sweep either side of the Delay, frames ([`Chorus::swing_ms`]):
    /// one glide whatever moved it (Depth, Delay, Rate, the tempo), so the
    /// swept delay's path never kinks.
    sweep: Smooth,
    log_delay: Smooth,
    width: Smooth,
    log_tone: Smooth,
    /// [`RIGHT_OFFSET`] at the voice count set.
    spread: Smooth,
    blend: Blend,
    gains: [Smooth; 4],
    /// Cached while nothing moves: base delay and swing in frames, the
    /// tone coefficient.
    base: f32,
    swing: f32,
    tone_a: f32,
    tone: [f32; 2],
}

impl Default for Chorus {
    fn default() -> Self {
        Self::new()
    }
}

impl Chorus {
    /// Voice `voice`'s delay in ms at LFO phase `phase` (0 to 1 a cycle),
    /// for the settings set; `None` for a voice not in use. For drawing.
    pub fn delay_ms(&self, voice: usize, phase: f32) -> Option<f32> {
        if voice >= self.v[chorus::VOICES as usize] as usize || voice >= 4 {
            return None;
        }
        let base = self.v[chorus::DELAY as usize];
        let shape = self.lfo.tri.target;
        Some(base + self.swing_ms() * wave(wrap(phase + VOICE_PHASE[voice]), shape))
    }

    /// The sweep either side of the Delay, ms, for the settings set.
    fn swing_ms(&self) -> f32 {
        let fs = self.rate as f32;
        let depth = self.v[chorus::DEPTH as usize] / 100.0 * DEPTH_MS;
        let ceiling = self.ceiling_frames() / fs * 1000.0;
        depth
            .min(DEPTH_OF_DELAY * self.v[chorus::DELAY as usize])
            .min(ceiling)
    }

    /// The largest sweep, frames, whose bend stays inside a semitone at
    /// the LFO's rate: a sine sweep of s frames at f cycles a frame moves
    /// at most 2 pi f s frames a frame.
    fn ceiling_frames(&self) -> f32 {
        let per_frame = (2.0 * std::f64::consts::PI * self.lfo.inc) as f32;
        (PITCH_CEILING / per_frame.max(1e-9)).min(1e6)
    }

    /// The most extra LFO phase a frame a synced catch-up may add (cycles):
    /// as much as bends the sweep by the pitch ceiling again, so a jump is
    /// caught up within two semitones of bend however deep the sweep.
    fn catch_up(&self) -> f64 {
        let swing = f64::from(self.swing);
        if swing > 0.0 {
            f64::from(PITCH_CEILING) / (2.0 * std::f64::consts::PI * swing)
        } else {
            f64::INFINITY
        }
    }

    pub fn new() -> Chorus {
        let v = defaults(&CHORUS_PARAMS);
        let mut c = Chorus {
            v,
            rate: 0,
            lfo: Lfo::default(),
            lines: [FlatLine::default(), FlatLine::default()],
            sweep: Smooth::new(0.0),
            log_delay: Smooth::new(v[3].log2()),
            width: Smooth::new(v[5] / 100.0),
            log_tone: Smooth::new(v[6].log2()),
            spread: Smooth::new(RIGHT_OFFSET[0]),
            blend: Blend::new(v[7] / 100.0, v[8] as usize),
            gains: [Smooth::new(0.0); 4],
            base: 0.0,
            swing: 0.0,
            tone_a: 1.0,
            tone: [0.0; 2],
        };
        c.voices();
        c.prepare(48_000, mpfree_node::MAX_BLOCK);
        c.reset();
        c
    }

    fn voices(&mut self) {
        let n = self.v[4] as usize;
        let g = 1.0 / (n as f32).sqrt();
        for (k, s) in self.gains.iter_mut().enumerate() {
            s.target = if k < n { g } else { 0.0 };
        }
        self.spread.target = RIGHT_OFFSET[n.clamp(1, 4) - 1];
    }

    /// Follows the LFO's rate (a Rate change, a tempo change when synced)
    /// and aims the sweep.
    fn follow_rate(&mut self) {
        self.lfo.set(self.v[0], self.v[1] as usize, self.rate);
        self.sweep.target = self.swing_ms() * 0.001 * self.rate as f32;
    }

    fn apply(&mut self, id: u16, value: f32) {
        let Some(p) = CHORUS_PARAMS.get(id as usize) else {
            return;
        };
        let v = p.clamp(value);
        self.v[id as usize] = v;
        match id {
            chorus::DELAY => self.log_delay.target = v.log2(),
            chorus::VOICES => self.voices(),
            chorus::WIDTH => self.width.target = v / 100.0,
            chorus::TONE => self.log_tone.target = v.log2(),
            chorus::MIX => self.blend.set_mix(v / 100.0),
            chorus::MIX_CURVE => self.blend.set_curve(v),
            chorus::SHAPE => self.lfo.set_shape(v >= 0.5),
            _ => {}
        }
        self.follow_rate();
    }

    fn update(&mut self) {
        let fs = self.rate as f32;
        self.base = self.log_delay.value.exp2() * 0.001 * fs;
        // The read's shortest delay, in case a long Delay drops fast.
        self.swing = self.sweep.value.min(self.base - (TAPS / 2) as f32).max(0.0);
        self.tone_a = 1.0 - (-2.0 * PI * self.log_tone.value.exp2() / fs).exp();
    }

    fn run(&mut self, l: &mut [f32], r: &mut [f32]) {
        for i in 0..l.len() {
            if !(self.sweep.settled() && self.log_delay.settled() && self.log_tone.settled()) {
                self.sweep.next();
                self.log_delay.next();
                self.log_tone.next();
                self.update();
            }
            let ph = self.lfo.next();
            let off_r = self.spread.glide() * self.width.glide();
            let mix = self.blend.next();
            let mut g = [0.0f32; 4];
            for (g, s) in g.iter_mut().zip(&mut self.gains) {
                *g = s.glide();
            }
            // Every voice's sweep on both sides, side by side (left's
            // voices first).
            let v = VOICE_PHASE;
            let lfo = self.lfo.waves([
                ph + v[0],
                ph + v[1],
                ph + v[2],
                ph + v[3],
                ph + v[0] + off_r,
                ph + v[1] + off_r,
                ph + v[2] + off_r,
                ph + v[3] + off_r,
            ]);
            let x = [l[i], r[i]];
            let mut y = [0.0f32; 2];
            for ch in 0..2 {
                self.lines[ch].push(x[ch]);
                let mut wet = 0.0;
                for k in 0..4 {
                    if g[k] == 0.0 {
                        continue;
                    }
                    let d = self.base + self.swing * lfo[4 * ch + k];
                    wet += g[k] * self.lines[ch].read(d);
                }
                self.tone[ch] = flush(self.tone[ch] + self.tone_a * (wet - self.tone[ch]));
                y[ch] = kit::blend(x[ch], self.tone[ch], mix);
            }
            l[i] = y[0];
            r[i] = y[1];
        }
    }
}

impl Node for Chorus {
    fn info(&self) -> &'static NodeInfo {
        &CHORUS_INFO
    }
    fn params(&self) -> &'static [ParamInfo] {
        &CHORUS_PARAMS
    }
    fn set_param(&mut self, id: u16, value: f32) {
        self.apply(id, value);
    }
    fn param(&self, id: u16) -> f32 {
        self.v.get(id as usize).copied().unwrap_or(f32::NAN)
    }
    fn prepare(&mut self, rate: u32, _max_block: usize) {
        let rate = rate.max(1);
        let new_rate = rate != self.rate;
        if new_rate {
            let frames = (MAX_CHORUS_MS * 0.001 * rate as f32) as usize + 4;
            self.lines = [FlatLine::new(frames), FlatLine::new(frames)];
        }
        self.rate = rate;
        smooth_all(
            &mut [
                &mut self.sweep,
                &mut self.log_delay,
                &mut self.width,
                &mut self.log_tone,
            ],
            rate,
            30.0,
        );
        self.spread.set_time(rate, SPREAD_MS);
        self.lfo.prepare(rate);
        self.blend.set_time(rate, 30.0);
        for s in &mut self.gains {
            s.set_time(rate, 20.0);
        }
        self.follow_rate();
        if new_rate {
            // The sweep is in frames.
            self.sweep.snap(self.sweep.target);
        }
        self.update();
    }
    fn reset(&mut self) {
        for s in [
            &mut self.sweep,
            &mut self.log_delay,
            &mut self.width,
            &mut self.log_tone,
            &mut self.spread,
        ] {
            s.snap(s.target);
        }
        self.blend.snap();
        for s in &mut self.gains {
            s.snap(s.target);
        }
        for line in &mut self.lines {
            line.clear();
        }
        self.tone = [0.0; 2];
        self.lfo.reset();
        self.update();
    }
    fn process(&mut self, ctx: &Ctx, events: &[Timed], left: &mut [f32], right: &mut [f32]) {
        self.lfo.begin(ctx, self.v[1] as usize, self.catch_up());
        self.follow_rate();
        let len = left.len().min(right.len());
        let mut segs = Segs::new(events);
        while let Some((from, to)) = segs.next(len, |ev| {
            if let NodeEvent::Param { id, value } = *ev {
                self.apply(id, value)
            }
        }) {
            self.run(&mut left[from..to], &mut right[from..to]);
        }
    }
    fn tail_frames(&self) -> u32 {
        let ms = self.v[3] + self.swing_ms();
        // The delay, then the tone filter settling.
        (ms * 0.001 * self.rate as f32) as u32 + self.rate / 100
    }
}

// --------------------------------------------------------------- flanger

/// [`Flanger`] parameter ids.
pub mod flanger {
    pub const RATE: u16 = 0;
    pub const SYNC: u16 = 1;
    pub const DEPTH: u16 = 2;
    pub const DELAY: u16 = 3;
    pub const FEEDBACK: u16 = 4;
    pub const WIDTH: u16 = 5;
    pub const MIX: u16 = 6;
    pub const THROUGH_ZERO: u16 = 7;
    /// The LFO's shape, Sine or a rounded Triangle (FX quality PR M4).
    pub const SHAPE: u16 = 8;
}

static FLANGER_INFO: NodeInfo = NodeInfo {
    id: "mpfree.fx.flanger",
    name: "Flanger",
    kind: Kind::Effect,
};
static FLANGER_PARAMS: [ParamInfo; 9] = [
    ParamInfo {
        id: 0,
        name: "Rate",
        group: "LFO",
        min: 0.02,
        max: 10.0,
        default: 0.2,
        unit: Unit::Hz,
        curve: Curve::Log,
        readonly: false,
    },
    ParamInfo {
        id: 1,
        name: "Sync",
        group: "LFO",
        min: 0.0,
        max: 12.0,
        default: 0.0,
        unit: Unit::None,
        curve: Curve::Choice(SYNCS),
        readonly: false,
    },
    ParamInfo {
        id: 2,
        name: "Depth",
        group: "LFO",
        min: 0.0,
        max: 100.0,
        default: 60.0,
        unit: Unit::Percent,
        curve: Curve::Linear,
        readonly: false,
    },
    ParamInfo {
        id: 3,
        name: "Delay",
        group: "Flanger",
        min: 0.1,
        max: 10.0,
        default: 1.5,
        unit: Unit::Ms,
        curve: Curve::Log,
        readonly: false,
    },
    ParamInfo {
        id: 4,
        name: "Feedback",
        group: "Flanger",
        min: -95.0,
        max: 95.0,
        default: 50.0,
        unit: Unit::Percent,
        curve: Curve::Linear,
        readonly: false,
    },
    ParamInfo {
        id: 5,
        name: "Width",
        group: "Flanger",
        min: 0.0,
        max: 100.0,
        default: 100.0,
        unit: Unit::Percent,
        curve: Curve::Linear,
        readonly: false,
    },
    ParamInfo {
        id: 6,
        name: "Mix",
        group: "Output",
        min: 0.0,
        max: 100.0,
        default: 50.0,
        unit: Unit::Percent,
        curve: Curve::Linear,
        readonly: false,
    },
    ParamInfo {
        id: 7,
        name: "Through zero",
        group: "Output",
        min: 0.0,
        max: 1.0,
        default: 0.0,
        unit: Unit::None,
        curve: Curve::Choice(THROUGH_ZERO),
        readonly: false,
    },
    ParamInfo {
        id: 8,
        name: "Shape",
        group: "LFO",
        min: 0.0,
        max: 1.0,
        default: 0.0,
        unit: Unit::None,
        curve: Curve::Choice(SHAPES),
        readonly: false,
    },
];

/// The sweep at full depth: two octaves of delay each way.
const FLANGE_OCTAVES: f32 = 2.0;
/// Through zero's choices.
const THROUGH_ZERO: &[&str] = &["Off", "On"];
/// With Through zero on, the dry waits this long (the latency reported)
/// and the copy sweeps either side of it, so the two cross.
const TZ_MS: f32 = 5.0;
/// The time constant of Through zero's crossover when it is switched (it
/// is done in about 140 ms).
const TZ_FADE_MS: f32 = 10.0;
/// With Through zero, the share of the power the dry and the copy have in
/// common on music, averaged over the sweep: the copy is within a few ms
/// of the dry, so the bass is shared. Pink noise shares 7 to 9 %, drum
/// hits 20 %; the trim takes off the middle.
const TZ_SHARED: f32 = 0.14;

/// The dry's wait with Through zero on, in whole frames.
fn tz_frames(rate: u32) -> f32 {
    (TZ_MS * 0.001 * rate as f32).round()
}
/// The longest the sweep reaches: past 20 ms a flanger is a doubler. At a
/// Delay over 5 ms the sweep is narrowed to fit, the same both ways.
const FLANGE_MAX_MS: f32 = 20.0;
/// `log2(FLANGE_MAX_MS)`.
const LOG2_FLANGE_MAX_MS: f32 = 4.321_928;

/// The sweep in octaves each way for a Depth of `depth` (0 to 1) at a
/// Delay of `log_delay` (log2 of its ms).
#[inline]
fn flange_octaves(depth: f32, log_delay: f32) -> f32 {
    (FLANGE_OCTAVES * depth).min(LOG2_FLANGE_MAX_MS - log_delay)
}

/// The flanger's level trim at Feedback `g`, Mix `m` and Through zero
/// `tz` (0 to 1 each): the dry and the comb summed carry `(1 - m)² + m² /
/// (1 - g²)` of a noise's power (the loop's own gain is `1 / (1 - g²)`,
/// and the comb's echoes don't line up with the dry), so this holds the
/// level at any Feedback and Mix, and the notches keep their depth. With
/// Through zero the copy is inverted and passes the dry, so it cancels
/// what they share, [`TZ_SHARED`] of the power on music; that comes off.
/// 1 at Mix 0.
#[inline]
fn flange_trim(g: f32, m: f32, tz: f32) -> f32 {
    let dry = 1.0 - m;
    1.0 / (dry * dry + m * m / (1.0 - g * g) - 2.0 * TZ_SHARED * tz * dry * m).sqrt()
}

/// A stereo flanger: a short delay swept in octaves (so the comb moves
/// evenly), with positive (metallic) or negative (hollow) feedback, read
/// through the flat read so the high notches stay deep round the loop,
/// and trimmed so it goes in at the dry's level at any Feedback.
///
/// **Through zero** (off by default) is the tape flange: the dry waits
/// [`TZ_MS`] and the copy, inverted, sweeps evenly either side of it
/// (Delay times Depth each way, at most the wait), so the two cross twice
/// a cycle and cancel there. The dry is read through the same flat read
/// at the wait, so with no Feedback the crossing cancels exactly.
pub struct Flanger {
    v: [f32; 9],
    rate: u32,
    lfo: Lfo,
    lines: [FlatLine; 2],
    /// The dry's lines for Through zero, written whether it is on or not.
    dry: [FlatLine; 2],
    /// Through zero, 0 off to 1 on, gliding over [`TZ_FADE_MS`].
    tz: Smooth,
    /// The dry's wait in frames ([`tz_frames`]) and its read's weights.
    tz_at: f32,
    tz_w: (usize, [f32; TAPS]),
    /// Frames written to the dry's lines since Through zero was last off
    /// (they aren't written while it is off).
    dry_fill: usize,
    depth: Smooth,
    log_delay: Smooth,
    feedback: Smooth,
    width: Smooth,
    mix: Smooth,
    /// The base delay in frames, cached while it holds still.
    base: f32,
    /// [`flange_octaves`]'s ceiling at the base delay, cached with it.
    max_oct: f32,
    /// [`flange_trim`] for the Feedback, Mix and Through zero it was
    /// worked out at.
    trim: f32,
    trim_at: (f32, f32, f32),
}

impl Default for Flanger {
    fn default() -> Self {
        Self::new()
    }
}

impl Flanger {
    /// Whether Through zero is on (the setting, not the glide).
    pub fn through_zero(&self) -> bool {
        self.v[flanger::THROUGH_ZERO as usize] >= 0.5
    }

    /// With Through zero on, how far the copy sweeps either side of the
    /// dry, in frames: Delay times Depth, at most the dry's wait less two
    /// frames.
    fn reach_frames(&self, delay_frames: f32, depth: f32) -> f32 {
        (delay_frames * depth).min(self.tz_at - 2.0)
    }

    /// The delay in frames at LFO phase `phase` (0 to 1 a cycle) for the
    /// settings set, worked out as the sound is.
    fn delay_frames(&self, phase: f32) -> f32 {
        if self.through_zero() {
            let delay = self.v[flanger::DELAY as usize] * 0.001 * self.rate as f32;
            let reach = self.reach_frames(delay, self.v[flanger::DEPTH as usize] / 100.0);
            return self.tz_at + reach * wave(wrap(phase), self.lfo.tri.target);
        }
        let log_delay = self.v[flanger::DELAY as usize].log2();
        let base = log_delay.exp2() * 0.001 * self.rate as f32;
        let oct = flange_octaves(self.v[flanger::DEPTH as usize] / 100.0, log_delay);
        base * (oct * wave(wrap(phase), self.lfo.tri.target)).exp2()
    }

    /// The copy's delay in ms at LFO phase `phase` (0 to 1 a cycle), for
    /// the settings set. For drawing.
    pub fn delay_ms(&self, phase: f32) -> f32 {
        if self.through_zero() {
            return self.delay_frames(phase) * 1000.0 / self.rate as f32;
        }
        let oct = flange_octaves(
            self.v[flanger::DEPTH as usize] / 100.0,
            self.v[flanger::DELAY as usize].log2(),
        );
        self.v[flanger::DELAY as usize] * (oct * wave(wrap(phase), self.lfo.tri.target)).exp2()
    }

    /// How far the copy is behind the dry in ms at LFO phase `phase` (0 to
    /// 1 a cycle), for the settings set: the copy's delay, less the dry's
    /// wait with Through zero on (then negative while the copy is ahead).
    /// For drawing.
    pub fn gap_ms(&self, phase: f32) -> f32 {
        self.delay_ms(phase) - self.wait_ms()
    }

    /// How long the dry waits in ms: [`TZ_MS`] (to the frame) with
    /// Through zero on, none with it off. For drawing.
    pub fn wait_ms(&self) -> f32 {
        if self.through_zero() {
            self.tz_at * 1000.0 / self.rate.max(1) as f32
        } else {
            0.0
        }
    }

    /// The response in dB at `hz` with the sweep held at `phase`, for the
    /// settings set: the comb the flanger makes there, through the read
    /// it makes it with. For drawing.
    pub fn response_db(&self, hz: f32, phase: f32) -> f32 {
        let fs = self.rate as f64;
        let (i, w) = flat_weights(self.delay_frames(phase) - 1.0, self.lines[0].longest());
        let g = self.v[flanger::FEEDBACK as usize] / 100.0;
        let m = self.v[flanger::MIX as usize] / 100.0;
        let (fb, mix) = (f64::from(g), f64::from(m));
        let (zr, zi) = flat_response(i, &w, 2.0 * std::f64::consts::PI * f64::from(hz) / fs);
        // wet = H / (1 - fb H), H the read round the loop;
        // out = trim ((1 - mix) + mix wet).
        let (dr, di) = (1.0 - fb * zr, -fb * zi);
        let den = dr * dr + di * di;
        let (wr, wi) = ((zr * dr + zi * di) / den, (zi * dr - zr * di) / den);
        let tz = if self.through_zero() { 1.0 } else { 0.0 };
        let trim = f64::from(flange_trim(g, m, tz));
        if self.through_zero() {
            // out = trim ((1 - mix) dry - mix wet), the dry through the
            // same read at the wait.
            let (i, w) = self.tz_w;
            let (xr, xi) = flat_response(i, &w, 2.0 * std::f64::consts::PI * f64::from(hz) / fs);
            return crate::svf::power_db((
                trim * ((1.0 - mix) * xr - mix * wr),
                trim * ((1.0 - mix) * xi - mix * wi),
            ));
        }
        crate::svf::power_db((trim * (1.0 - mix + mix * wr), trim * mix * wi))
    }

    pub fn new() -> Flanger {
        let v = defaults(&FLANGER_PARAMS);
        let mut f = Flanger {
            v,
            rate: 0,
            lfo: Lfo::default(),
            lines: [FlatLine::default(), FlatLine::default()],
            dry: [FlatLine::default(), FlatLine::default()],
            tz: Smooth::new(if v[7] >= 0.5 { 1.0 } else { 0.0 }),
            tz_at: 0.0,
            tz_w: (0, [0.0; TAPS]),
            dry_fill: usize::MAX,
            depth: Smooth::new(v[2] / 100.0),
            log_delay: Smooth::new(v[3].log2()),
            feedback: Smooth::new(v[4] / 100.0),
            width: Smooth::new(v[5] / 100.0),
            mix: Smooth::new(v[6] / 100.0),
            base: 0.0,
            max_oct: 0.0,
            trim: 1.0,
            trim_at: (f32::NAN, f32::NAN, f32::NAN),
        };
        f.prepare(48_000, mpfree_node::MAX_BLOCK);
        f.reset();
        f
    }

    fn apply(&mut self, id: u16, value: f32) {
        let Some(p) = FLANGER_PARAMS.get(id as usize) else {
            return;
        };
        let v = p.clamp(value);
        self.v[id as usize] = v;
        match id {
            flanger::DEPTH => self.depth.target = v / 100.0,
            flanger::DELAY => self.log_delay.target = v.log2(),
            flanger::FEEDBACK => self.feedback.target = v / 100.0,
            flanger::WIDTH => self.width.target = v / 100.0,
            flanger::MIX => self.mix.target = v / 100.0,
            flanger::THROUGH_ZERO => self.tz.target = if v >= 0.5 { 1.0 } else { 0.0 },
            flanger::SHAPE => self.lfo.set_shape(v >= 0.5),
            _ => {}
        }
        self.lfo.set(self.v[0], self.v[1] as usize, self.rate);
    }

    /// The base delay and its sweep's ceiling from the glided Delay.
    fn set_base(&mut self, log_delay: f32) {
        self.base = log_delay.exp2() * 0.001 * self.rate as f32;
        self.max_oct = LOG2_FLANGE_MAX_MS - log_delay;
    }

    fn run(&mut self, l: &mut [f32], r: &mut [f32]) {
        if self.tz.value == 0.0 && self.tz.target == 0.0 {
            // Through zero off: the dry's lines go stale.
            self.dry_fill = 0;
            self.run_off(l, r);
        } else {
            self.run_tz(l, r);
        }
    }

    /// The flanger with Through zero off and settled.
    fn run_off(&mut self, l: &mut [f32], r: &mut [f32]) {
        let longest = self.lines[0].longest();
        for i in 0..l.len() {
            if !self.log_delay.settled() {
                let ld = self.log_delay.next();
                self.set_base(ld);
            }
            let depth = (FLANGE_OCTAVES * self.depth.next()).min(self.max_oct);
            let fb = self.feedback.next();
            let off_r = 0.25 * self.width.next();
            let mix = self.mix.next();
            if (fb, mix, 0.0) != self.trim_at {
                self.trim = flange_trim(fb, mix, 0.0);
                self.trim_at = (fb, mix, 0.0);
            }
            let ph = self.lfo.next();
            let lfo = self.lfo.waves([ph, ph + off_r]);
            let x = [l[i], r[i]];
            let mut y = [0.0f32; 2];
            for ch in 0..2 {
                let d = self.base * (depth * lfo[ch]).exp2();
                // Read before the write: the last write is one frame back.
                let (k, w) = flat_weights(d - 1.0, longest);
                let wet = self.lines[ch].read_with(k, &w);
                self.lines[ch].push(flush(x[ch] + fb * wet));
                y[ch] = self.trim * lerp(x[ch], wet, mix);
            }
            l[i] = y[0];
            r[i] = y[1];
        }
    }

    /// The flanger with Through zero on, or gliding either way. Turned
    /// on, the dry's lines fill for the wait before the glide starts.
    fn run_tz(&mut self, l: &mut [f32], r: &mut [f32]) {
        let longest = self.lines[0].longest();
        let (zi, zw) = self.tz_w;
        let fill = self.dry[0].n;
        for i in 0..l.len() {
            if !self.log_delay.settled() {
                let ld = self.log_delay.next();
                self.set_base(ld);
            }
            let raw = self.depth.next();
            let depth = (FLANGE_OCTAVES * raw).min(self.max_oct);
            let fb = self.feedback.next();
            let off_r = 0.25 * self.width.next();
            let mix = self.mix.next();
            let tz = if self.dry_fill < fill {
                self.dry_fill += 1;
                self.tz.value
            } else {
                self.tz.next()
            };
            if (fb, mix, tz) != self.trim_at {
                self.trim = flange_trim(fb, mix, tz);
                self.trim_at = (fb, mix, tz);
            }
            let reach = if tz > 0.0 {
                self.reach_frames(self.base, raw)
            } else {
                0.0
            };
            let ph = self.lfo.next();
            let lfo = self.lfo.waves([ph, ph + off_r]);
            let x = [l[i], r[i]];
            let mut y = [0.0f32; 2];
            for ch in 0..2 {
                let s = lfo[ch];
                // Read before the write: the last write is one frame back.
                let swept = if tz < 1.0 {
                    let d = self.base * (depth * s).exp2();
                    let (k, w) = flat_weights(d - 1.0, longest);
                    self.lines[ch].read_with(k, &w)
                } else {
                    0.0
                };
                let (dry, wet) = if tz > 0.0 {
                    let (k, w) = flat_weights(self.tz_at + reach * s - 1.0, longest);
                    let crossed = self.lines[ch].read_with(k, &w);
                    let waited = self.dry[ch].read_with(zi, &zw);
                    // Off to on: the dry to the waited dry, the copy to the
                    // crossing copy, inverted.
                    (
                        lerp(x[ch], waited, tz),
                        (lerp(swept, crossed, tz), lerp(swept, -crossed, tz)),
                    )
                } else {
                    (x[ch], (swept, swept))
                };
                self.dry[ch].push(x[ch]);
                self.lines[ch].push(flush(x[ch] + fb * wet.0));
                y[ch] = self.trim * lerp(dry, wet.1, mix);
            }
            l[i] = y[0];
            r[i] = y[1];
        }
    }
}

impl Node for Flanger {
    fn info(&self) -> &'static NodeInfo {
        &FLANGER_INFO
    }
    fn params(&self) -> &'static [ParamInfo] {
        &FLANGER_PARAMS
    }
    fn set_param(&mut self, id: u16, value: f32) {
        self.apply(id, value);
    }
    fn param(&self, id: u16) -> f32 {
        self.v.get(id as usize).copied().unwrap_or(f32::NAN)
    }
    fn prepare(&mut self, rate: u32, _max_block: usize) {
        let rate = rate.max(1);
        if rate != self.rate {
            // The longest the sweep reaches (Through zero's reaches twice
            // the dry's wait, well inside it).
            let frames = (FLANGE_MAX_MS * 0.001 * rate as f32) as usize + 8;
            self.lines = [FlatLine::new(frames), FlatLine::new(frames)];
            let wait = tz_frames(rate) as usize + 8;
            self.dry = [FlatLine::new(wait), FlatLine::new(wait)];
        }
        self.rate = rate;
        self.tz_at = tz_frames(rate);
        self.tz_w = flat_weights(self.tz_at - 1.0, self.dry[0].longest());
        smooth_all(
            &mut [
                &mut self.depth,
                &mut self.log_delay,
                &mut self.feedback,
                &mut self.width,
                &mut self.mix,
            ],
            rate,
            30.0,
        );
        self.tz.set_time(rate, TZ_FADE_MS);
        self.lfo.set(self.v[0], self.v[1] as usize, rate);
        self.lfo.prepare(rate);
        self.set_base(self.log_delay.value);
    }
    fn reset(&mut self) {
        for s in [
            &mut self.depth,
            &mut self.log_delay,
            &mut self.feedback,
            &mut self.width,
            &mut self.mix,
            &mut self.tz,
        ] {
            s.snap(s.target);
        }
        for line in self.lines.iter_mut().chain(self.dry.iter_mut()) {
            line.clear();
        }
        // Cleared, the dry's lines hold the silence before.
        self.dry_fill = usize::MAX;
        self.lfo.reset();
        self.set_base(self.log_delay.value);
    }
    fn process(&mut self, ctx: &Ctx, events: &[Timed], left: &mut [f32], right: &mut [f32]) {
        self.lfo.begin(ctx, self.v[1] as usize, f64::INFINITY);
        self.lfo.set(self.v[0], self.v[1] as usize, self.rate);
        let len = left.len().min(right.len());
        let mut segs = Segs::new(events);
        while let Some((from, to)) = segs.next(len, |ev| {
            if let NodeEvent::Param { id, value } = *ev {
                self.apply(id, value)
            }
        }) {
            self.run(&mut left[from..to], &mut right[from..to]);
        }
    }
    fn latency(&self) -> u32 {
        // Through zero holds the dry back; the copy sweeps either side.
        if self.through_zero() {
            self.tz_at as u32
        } else {
            0
        }
    }
    fn tail_frames(&self) -> u32 {
        // The longest loop, rung down 80 dB by the feedback.
        let longest = self.delay_ms(0.25) * 0.001 * self.rate as f32;
        let fb = (self.v[4] / 100.0).abs();
        let rings = if fb > 0.0 {
            (1e-4f32).ln() / fb.ln()
        } else {
            0.0
        };
        (longest * (1.0 + rings)) as u32 + 2
    }
}

// ---------------------------------------------------------------- phaser

/// [`Phaser`] parameter ids.
pub mod phaser {
    pub const RATE: u16 = 0;
    pub const SYNC: u16 = 1;
    pub const DEPTH: u16 = 2;
    pub const CENTER: u16 = 3;
    pub const STAGES: u16 = 4;
    pub const FEEDBACK: u16 = 5;
    pub const WIDTH: u16 = 6;
    pub const MIX: u16 = 7;
    /// How far apart the sides sweep at Width 100 %: a quarter cycle (0,
    /// as saved projects had it) or half (1, opposite sweeps).
    pub const STEREO: u16 = 8;
    /// The LFO's shape, Sine or a rounded Triangle (FX quality PR M4).
    pub const SHAPE: u16 = 9;
}

const STAGE_NAMES: &[&str] = &["2", "4", "6", "8", "12"];
const STAGE_COUNTS: [usize; 5] = [2, 4, 6, 8, 12];
const MAX_STAGES: usize = 12;
/// The sweep at full depth: two and a half octaves each way.
const PHASE_OCTAVES: f32 = 2.5;
const PHASER_STEREO: &[&str] = &["Quarter", "Opposite"];

/// The phaser's dry and wet together, on music, carry `2 m (1 - m) C` of
/// its power on top of what each carries alone: the wet is the dry
/// turned in phase, so below the first notch, where most of a mix's
/// loudness is, they add. 0.3 is that share measured on pink noise
/// through a K-weighting, over the sweep, at the defaults; with it the
/// defaults and every preset go in within 0.6 LU of the dry, and any
/// Centre and stage count within about 2 LU (FX quality PR M3).
const PHASER_CROSS: f32 = 0.3;

/// The corner of the high-pass in the phaser's feedback path, Hz. The
/// stages pass DC unturned, so without it the loop rings at DC: at +95 %
/// Feedback a sound's lowest few Hz came out 26 dB up (and its DC
/// offset with them), which an analogue phaser's coupled feedback never
/// does.
const PHASER_FB_HP_HZ: f32 = 10.0;

/// The phaser's level trim at Feedback `g` and Mix `m` (0 to 1): the
/// loop is all-pass, so its noise gain is `1 / (1 - g²)`, and the dry
/// and the wet together carry `(1 - m)² + m² / (1 - g²) + 2 m (1 - m)
/// C`. On dry and wet together, so the notches keep their depth. 1 at
/// Mix 0; at Mix 100 % it holds the level exactly at any Feedback.
#[inline]
fn phaser_trim(g: f32, m: f32) -> f32 {
    let dry = 1.0 - m;
    1.0 / (dry * dry + m * m / (1.0 - g * g) + 2.0 * m * dry * PHASER_CROSS).sqrt()
}

static PHASER_INFO: NodeInfo = NodeInfo {
    id: "mpfree.fx.phaser",
    name: "Phaser",
    kind: Kind::Effect,
};
static PHASER_PARAMS: [ParamInfo; 10] = [
    ParamInfo {
        id: 0,
        name: "Rate",
        group: "LFO",
        min: 0.02,
        max: 10.0,
        default: 0.3,
        unit: Unit::Hz,
        curve: Curve::Log,
        readonly: false,
    },
    ParamInfo {
        id: 1,
        name: "Sync",
        group: "LFO",
        min: 0.0,
        max: 12.0,
        default: 0.0,
        unit: Unit::None,
        curve: Curve::Choice(SYNCS),
        readonly: false,
    },
    ParamInfo {
        id: 2,
        name: "Depth",
        group: "LFO",
        min: 0.0,
        max: 100.0,
        default: 70.0,
        unit: Unit::Percent,
        curve: Curve::Linear,
        readonly: false,
    },
    ParamInfo {
        id: 3,
        name: "Center",
        group: "Phaser",
        min: 100.0,
        max: 8_000.0,
        default: 700.0,
        unit: Unit::Hz,
        curve: Curve::Log,
        readonly: false,
    },
    ParamInfo {
        id: 4,
        name: "Stages",
        group: "Phaser",
        min: 0.0,
        max: 4.0,
        default: 1.0,
        unit: Unit::None,
        curve: Curve::Choice(STAGE_NAMES),
        readonly: false,
    },
    ParamInfo {
        id: 5,
        name: "Feedback",
        group: "Phaser",
        min: -95.0,
        max: 95.0,
        default: 30.0,
        unit: Unit::Percent,
        curve: Curve::Linear,
        readonly: false,
    },
    ParamInfo {
        id: 6,
        name: "Width",
        group: "Phaser",
        min: 0.0,
        max: 100.0,
        default: 100.0,
        unit: Unit::Percent,
        curve: Curve::Linear,
        readonly: false,
    },
    ParamInfo {
        id: 7,
        name: "Mix",
        group: "Output",
        min: 0.0,
        max: 100.0,
        default: 50.0,
        unit: Unit::Percent,
        curve: Curve::Linear,
        readonly: false,
    },
    ParamInfo {
        id: 8,
        name: "Stereo",
        group: "Phaser",
        min: 0.0,
        max: 1.0,
        default: 0.0,
        unit: Unit::None,
        curve: Curve::Choice(PHASER_STEREO),
        readonly: false,
    },
    ParamInfo {
        id: 9,
        name: "Shape",
        group: "LFO",
        min: 0.0,
        max: 1.0,
        default: 0.0,
        unit: Unit::None,
        curve: Curve::Choice(SHAPES),
        readonly: false,
    },
];

/// A stereo phaser: 2 to 12 first-order all-pass stages (zero-delay
/// feedback, so a fast sweep stays clean) swept in octaves around a centre,
/// with feedback for resonance. Each pair of stages adds a notch. A level
/// trim ([`phaser_trim`]) holds it near the dry's level at any Feedback
/// and Mix.
pub struct Phaser {
    v: [f32; 10],
    rate: u32,
    lfo: Lfo,
    depth: Smooth,
    log_center: Smooth,
    feedback: Smooth,
    /// The right side's LFO offset in cycles: Width times a quarter, or
    /// a half on Opposite.
    width: Smooth,
    mix: Smooth,
    /// The dry's and the wet's gains, Mix times the level trim, and the
    /// (Feedback, Mix) they were worked out for.
    trim: [f32; 2],
    trim_at: (f32, f32),
    /// Crossfade weights for the taps after 2, 4, 6, 8 and 12 stages.
    taps: [Smooth; 5],
    state: [[f32; MAX_STAGES]; 2],
    last: [f32; 2],
    /// The feedback path's high-pass ([`PHASER_FB_HP_HZ`]): its pole and,
    /// for each side, its last input (its output is `last`).
    hp_r: f32,
    hp_x: [f32; 2],
    /// The stages' coefficient for each side, worked out every [`CTL`]
    /// frames (an exp2 and a tan each) and glided to between.
    gg: [f32; 2],
    gg_step: [f32; 2],
    /// Frames until the next coefficient; whether it lands at once (after
    /// a reset) instead of gliding.
    ctl: u32,
    land: bool,
}

/// Frames between the Phaser's coefficient updates: every 0.36 ms at 44.1
/// kHz, far quicker than its sweep, with a straight glide between, so the
/// sweep stays smooth for a sixteenth of the cost.
const CTL: u32 = 16;

impl Default for Phaser {
    fn default() -> Self {
        Self::new()
    }
}

impl Phaser {
    /// How far the right side's sweep runs behind the left's, in cycles,
    /// for the settings set. For drawing.
    pub fn right_offset(&self) -> f32 {
        self.reach() * self.v[phaser::WIDTH as usize] / 100.0
    }

    /// The right side's offset at Width 100 %.
    fn reach(&self) -> f32 {
        if self.v[phaser::STEREO as usize] >= 0.5 {
            0.5
        } else {
            0.25
        }
    }

    /// The all-pass corner in Hz at LFO phase `phase` (0 to 1 a cycle),
    /// for the settings set. For drawing.
    pub fn center_hz(&self, phase: f32) -> f32 {
        let depth = PHASE_OCTAVES * self.v[phaser::DEPTH as usize] / 100.0;
        let center = self.v[phaser::CENTER as usize].log2();
        (center + depth * wave(wrap(phase), self.lfo.tri.target))
            .exp2()
            .min(0.45 * self.rate as f32)
    }

    /// The response in dB at `hz` with the sweep held at `phase`, for the
    /// settings set: the notches the phaser makes there. For drawing.
    pub fn response_db(&self, hz: f32, phase: f32) -> f32 {
        let fs = self.rate as f64;
        let g = (std::f64::consts::PI * f64::from(self.center_hz(phase)) / fs).tan();
        // Each stage is the all-pass (a + z^-1) / (1 + a z^-1).
        let a = 2.0 * g / (1.0 + g) - 1.0;
        let n = STAGE_COUNTS[(self.v[phaser::STAGES as usize] as usize).min(4)];
        let fb = f64::from(self.v[phaser::FEEDBACK as usize]) / 100.0;
        let mix = f64::from(self.v[phaser::MIX as usize]) / 100.0;
        let w = 2.0 * std::f64::consts::PI * f64::from(hz) / fs;
        let z = (w.cos(), -w.sin()); // z^-1
        let cdiv = |a: (f64, f64), b: (f64, f64)| {
            let den = b.0 * b.0 + b.1 * b.1;
            ((a.0 * b.0 + a.1 * b.1) / den, (a.1 * b.0 - a.0 * b.1) / den)
        };
        let stage = cdiv((a + z.0, z.1), (1.0 + a * z.0, a * z.1));
        let mut chain = (1.0, 0.0);
        for _ in 0..n {
            chain = crate::svf::cmul(chain, stage);
        }
        // out = A^n x / (1 - fb z^-1 HP A^n): the feedback is one frame
        // late, through the high-pass (1 - z^-1) / (1 - r z^-1).
        let r = f64::from(self.hp_r);
        let hp = cdiv((1.0 - z.0, -z.1), (1.0 - r * z.0, -r * z.1));
        let loop_ = crate::svf::cmul(crate::svf::cmul(z, hp), chain);
        let wet = cdiv(chain, (1.0 - fb * loop_.0, -fb * loop_.1));
        let trim = f64::from(phaser_trim(fb as f32, mix as f32));
        crate::svf::power_db((trim * (1.0 - mix + mix * wet.0), trim * mix * wet.1))
    }

    pub fn new() -> Phaser {
        let v = defaults(&PHASER_PARAMS);
        let mut p = Phaser {
            v,
            rate: 48_000,
            lfo: Lfo::default(),
            depth: Smooth::new(v[2] / 100.0),
            log_center: Smooth::new(v[3].log2()),
            feedback: Smooth::new(v[5] / 100.0),
            width: Smooth::new(0.25 * v[6] / 100.0),
            mix: Smooth::new(v[7] / 100.0),
            trim: [0.0; 2],
            trim_at: (f32::NAN, f32::NAN),
            taps: [Smooth::new(0.0); 5],
            state: [[0.0; MAX_STAGES]; 2],
            last: [0.0; 2],
            hp_r: 1.0,
            hp_x: [0.0; 2],
            gg: [0.0; 2],
            gg_step: [0.0; 2],
            ctl: 0,
            land: true,
        };
        p.stages();
        p.prepare(48_000, mpfree_node::MAX_BLOCK);
        p.reset();
        p
    }

    fn stages(&mut self) {
        let pick = self.v[4] as usize;
        for (k, t) in self.taps.iter_mut().enumerate() {
            t.target = if k == pick { 1.0 } else { 0.0 };
        }
    }

    fn apply(&mut self, id: u16, value: f32) {
        let Some(p) = PHASER_PARAMS.get(id as usize) else {
            return;
        };
        let v = p.clamp(value);
        self.v[id as usize] = v;
        match id {
            phaser::DEPTH => self.depth.target = v / 100.0,
            phaser::CENTER => self.log_center.target = v.log2(),
            phaser::STAGES => self.stages(),
            phaser::FEEDBACK => self.feedback.target = v / 100.0,
            phaser::WIDTH | phaser::STEREO => self.width.target = self.right_offset(),
            phaser::MIX => self.mix.target = v / 100.0,
            phaser::SHAPE => self.lfo.set_shape(v >= 0.5),
            _ => {}
        }
        self.lfo.set(self.v[0], self.v[1] as usize, self.rate);
    }

    fn run(&mut self, l: &mut [f32], r: &mut [f32]) {
        let fs = self.rate as f32;
        let top = 0.45 * fs;
        for i in 0..l.len() {
            let depth = PHASE_OCTAVES * self.depth.next();
            let center = self.log_center.next();
            let fb = self.feedback.next();
            let off_r = self.width.next();
            let mix = self.mix.next();
            if (fb, mix) != self.trim_at {
                let trim = phaser_trim(fb, mix);
                self.trim = [trim * (1.0 - mix), trim * mix];
                self.trim_at = (fb, mix);
            }
            let [dry, wet] = self.trim;
            let mut w = [0.0f32; 5];
            for (w, t) in w.iter_mut().zip(&mut self.taps) {
                *w = t.next();
            }
            // Stages past the last tap in use rest, cleared.
            let used = (0..5)
                .rev()
                .find(|&k| w[k] != 0.0)
                .map_or(0, |k| STAGE_COUNTS[k]);
            let ph = self.lfo.next();
            if self.ctl == 0 {
                // The coefficient now; the stages glide to it over the next
                // CTL frames (it lands at once after a reset). The counter
                // runs on across blocks, so any block size sounds the same.
                self.ctl = CTL;
                let lfo = self.lfo.waves([ph, ph + off_r]);
                for ch in 0..2 {
                    let hz = (center + depth * lfo[ch]).exp2().min(top);
                    let g = (PI * hz / fs).tan();
                    let to = g / (1.0 + g);
                    if self.land {
                        self.gg[ch] = to;
                    }
                    self.gg_step[ch] = (to - self.gg[ch]) / CTL as f32;
                }
                self.land = false;
            }
            self.ctl -= 1;
            let x = [l[i], r[i]];
            let mut y = [0.0f32; 2];
            for ch in 0..2 {
                let gg = self.gg[ch];
                self.gg[ch] += self.gg_step[ch];
                let mut u = x[ch] + fb * self.last[ch];
                let mut out = 0.0;
                let mut tap = 0;
                let st = &mut self.state[ch];
                for (k, s) in st.iter_mut().enumerate().take(used) {
                    // A trapezoidal one-pole low-pass; all-pass = 2 lp - in.
                    let v = (u - *s) * gg;
                    let lp = v + *s;
                    *s = flush(lp + v);
                    u = 2.0 * lp - u;
                    if k + 1 == STAGE_COUNTS[tap] {
                        out += w[tap] * u;
                        tap += 1;
                    }
                }
                for s in &mut st[used..] {
                    *s = 0.0;
                }
                // What goes round again, through the high-pass.
                let fed = out - self.hp_x[ch] + self.hp_r * self.last[ch];
                self.hp_x[ch] = out;
                self.last[ch] = flush(fed);
                y[ch] = dry * x[ch] + wet * out;
            }
            l[i] = y[0];
            r[i] = y[1];
        }
    }
}

impl Node for Phaser {
    fn info(&self) -> &'static NodeInfo {
        &PHASER_INFO
    }
    fn params(&self) -> &'static [ParamInfo] {
        &PHASER_PARAMS
    }
    fn set_param(&mut self, id: u16, value: f32) {
        self.apply(id, value);
    }
    fn param(&self, id: u16) -> f32 {
        self.v.get(id as usize).copied().unwrap_or(f32::NAN)
    }
    fn prepare(&mut self, rate: u32, _max_block: usize) {
        self.rate = rate.max(1);
        smooth_all(
            &mut [
                &mut self.depth,
                &mut self.log_center,
                &mut self.feedback,
                &mut self.width,
                &mut self.mix,
            ],
            self.rate,
            30.0,
        );
        for t in &mut self.taps {
            t.set_time(self.rate, 20.0);
        }
        self.hp_r = 1.0 - 2.0 * PI * PHASER_FB_HP_HZ / self.rate as f32;
        self.lfo.set(self.v[0], self.v[1] as usize, self.rate);
        self.lfo.prepare(self.rate);
    }
    fn reset(&mut self) {
        for s in [
            &mut self.depth,
            &mut self.log_center,
            &mut self.feedback,
            &mut self.width,
            &mut self.mix,
        ] {
            s.snap(s.target);
        }
        for t in &mut self.taps {
            t.snap(t.target);
        }
        self.state = [[0.0; MAX_STAGES]; 2];
        self.last = [0.0; 2];
        self.hp_x = [0.0; 2];
        self.gg_step = [0.0; 2];
        self.ctl = 0;
        self.land = true;
        self.lfo.reset();
    }
    fn process(&mut self, ctx: &Ctx, events: &[Timed], left: &mut [f32], right: &mut [f32]) {
        self.lfo.begin(ctx, self.v[1] as usize, f64::INFINITY);
        self.lfo.set(self.v[0], self.v[1] as usize, self.rate);
        let len = left.len().min(right.len());
        let mut segs = Segs::new(events);
        while let Some((from, to)) = segs.next(len, |ev| {
            if let NodeEvent::Param { id, value } = *ev {
                self.apply(id, value)
            }
        }) {
            self.run(&mut left[from..to], &mut right[from..to]);
        }
    }
    fn tail_frames(&self) -> u32 {
        // Each stage holds about 1 / (2 pi f) seconds at the lowest point
        // of the sweep; the feedback rings that loop down 80 dB.
        let low = (self.v[3] * (-PHASE_OCTAVES * self.v[2] / 100.0).exp2()).max(10.0);
        let stages = STAGE_COUNTS[self.v[4] as usize] as f32;
        let lp = stages / (2.0 * PI * low);
        let fb = (self.v[5] / 100.0).abs();
        let rings = if fb > 0.0 {
            (1e-4f32).ln() / fb.ln()
        } else {
            0.0
        };
        // The feedback's high-pass rings out too: round the loop its pole
        // moves to (r - g) / (1 - g), slowest at -95 % (31 ms at 48 kHz),
        // so 80 dB down takes 9.2 (1 - g) / (1 - r) frames.
        let g = self.v[5] / 100.0;
        let hp = 9.2 * (1.0 - g) / (1.0 - self.hp_r).max(1e-6);
        // At least 8 time constants of the slowest stage, plus the rings.
        let stages_frames = (8.0 / (2.0 * PI * low) + lp * rings) * self.rate as f32;
        (stages_frames + hp) as u32 + 2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lfo_sine_is_a_sine() {
        for i in 0..1000 {
            let p = i as f32 / 1000.0;
            assert!((sine(p) - (2.0 * PI * p).sin()).abs() < 0.002, "{p}");
        }
    }

    #[test]
    fn the_triangle_is_in_step_with_the_sine_and_its_corners_are_round() {
        let n = 10_000;
        let tri: Vec<f32> = (0..=n).map(|i| triangle(i as f32 / n as f32)).collect();
        // 0, 1, 0, -1 at the quarters, as the sine is, and the same sign.
        for (q, want) in [(0, 0.0f32), (1, 1.0), (2, 0.0), (3, -1.0)] {
            let got = tri[q * n / 4];
            assert!((got - want).abs() < 1e-5, "quarter {q}: {got}");
        }
        for (i, t) in tri.iter().enumerate().take(n) {
            let s = sine(i as f32 / n as f32);
            assert!(t * s >= 0.0 && t.abs() <= 1.0 + 1e-6, "{i}: {t} {s}");
        }
        // The slope: steady between the corners, never more than the
        // straight parts', and never stepping, so the bend doesn't flip.
        let slope: Vec<f32> = tri.windows(2).map(|w| (w[1] - w[0]) * n as f32).collect();
        let straight = 4.0 / (1.0 - 2.0 * TRI_ROUND);
        assert!((slope[n / 2] + straight).abs() < 1e-2, "{}", slope[n / 2]);
        assert!((slope[n / 8] - straight).abs() < 1e-2, "{}", slope[n / 8]);
        let steepest = slope.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(steepest < straight * 1.002, "{steepest}");
        let step = slope
            .windows(2)
            .fold(0.0f32, |m, w| m.max((w[1] - w[0]).abs()));
        // A corner turns the slope round over 2 TRI_ROUND cycles (with
        // a little f32 rounding; a sharp corner would step 1,000 times this).
        let turn = 2.0 * straight / (2.0 * TRI_ROUND) / n as f32;
        assert!(step < turn * 1.5, "{step} against {turn}");
        // Under the sine's steepest (2 pi), so a sweep limited for the
        // sine is limited for the triangle too.
        assert!(straight < 2.0 * PI);
        // A crossfade sits between the two.
        for i in 0..100 {
            let p = i as f32 / 100.0;
            let mid = wave(p, 0.5);
            assert!((mid - 0.5 * (sine(p) + triangle(p))).abs() < 1e-6);
        }
    }

    #[test]
    fn the_dotted_and_triplet_syncs_take_their_beats() {
        assert_eq!(SYNCS.len(), SYNC_BEATS.len());
        let at_120 = |sync: usize| {
            let mut lfo = Lfo::default();
            let ctx = Ctx {
                bpm: 120.0,
                beats: 1.0,
                playing: true,
                ..Ctx::new(48_000, 0)
            };
            lfo.begin(&ctx, sync, f64::INFINITY);
            lfo.set(1.0, sync, 48_000);
            (lfo.phase, lfo.inc * 48_000.0)
        };
        // A beat at 120 BPM is half a second.
        for (name, beats) in [
            ("1/8T", 1.0 / 3.0),
            ("1/8D", 0.75),
            ("1/4T", 2.0 / 3.0),
            ("1/4D", 1.5),
        ] {
            let k = SYNCS.iter().position(|s| *s == name).unwrap();
            assert!(
                k > 8,
                "{name} appended after the choices saved projects have"
            );
            let (phase, hz) = at_120(k);
            assert!((hz - 2.0 / beats).abs() < 1e-9, "{name}: {hz} Hz");
            // One beat into the song.
            assert!(
                (phase - (1.0 / beats).rem_euclid(1.0)).abs() < 1e-9,
                "{name}: {phase}"
            );
        }
        for params in [&CHORUS_PARAMS[..], &FLANGER_PARAMS[..], &PHASER_PARAMS[..]] {
            let sync = params.iter().find(|p| p.name == "Sync").unwrap();
            assert_eq!(sync.max as usize, SYNCS.len() - 1);
            let shape = params.iter().find(|p| p.name == "Shape").unwrap();
            assert_eq!((shape.max as usize, shape.default), (SHAPES.len() - 1, 0.0));
        }
    }

    #[test]
    fn the_phasers_stereo_puts_the_right_sweep_a_quarter_or_half_behind() {
        // Each side's corner, from its coefficient the frame after a
        // reset (it lands there at once), with the LFO a fifth of the way
        // round. Opposite at Width 100 % mirrors the sweep about the
        // centre; Quarter (and every saved project) keeps a quarter.
        let corner = |stereo: f32, width: f32| {
            let mut p = Phaser::new();
            p.set_param(phaser::DEPTH, 100.0);
            p.set_param(phaser::CENTER, 700.0);
            p.set_param(phaser::WIDTH, width);
            p.set_param(phaser::STEREO, stereo);
            p.reset();
            p.lfo.phase = 0.2 - p.lfo.inc;
            p.run(&mut [0.0], &mut [0.0]);
            p.gg.map(|gg| {
                let g = f64::from(gg / (1.0 - gg));
                (g.atan() * f64::from(p.rate) / std::f64::consts::PI) as f32
            })
        };
        let at = |ph: f32| 700.0 * (PHASE_OCTAVES * sine(wrap(ph))).exp2();
        let close = |a: f32, b: f32| (a / b - 1.0).abs() < 1e-3;
        for (stereo, width, off) in [
            (0.0, 100.0, 0.25),
            (1.0, 100.0, 0.5),
            (1.0, 50.0, 0.25),
            (1.0, 0.0, 0.0),
        ] {
            let [l, r] = corner(stereo, width);
            assert!(close(l, at(0.2)), "left {l} against {}", at(0.2));
            assert!(
                close(r, at(0.2 + off)),
                "Stereo {stereo}, Width {width}: right {r} against {}",
                at(0.2 + off)
            );
        }
        // Opposite at Width 100 %: the two corners sit an equal number of
        // octaves either side of the centre.
        let [l, r] = corner(1.0, 100.0);
        assert!(((l * r).sqrt() / 700.0 - 1.0).abs() < 1e-3, "{l} and {r}");
    }

    #[test]
    fn the_flat_read_near_the_write_is_whole_and_smooth() {
        // A ramp, near the write where the read fades to a cubic: whole
        // frames read exactly, fractions follow the ramp, and the cubic
        // and the sinc meet without a step.
        let mut m = FlatLine::new(64);
        for k in 0..100 {
            m.push(k as f32);
        }
        let read = |d: f32| {
            let (i, w) = flat_weights(d, m.longest());
            m.read_with(i, &w)
        };
        for d in [1.0f32, 2.0, 3.0, 4.0, 5.0, 9.0] {
            assert!((read(d) - (99.0 - d)).abs() < 1e-4, "{d}: {}", read(d));
        }
        for d in [1.25f32, 2.5, 3.25, 3.5, 3.75, 4.5] {
            assert!((read(d) - (99.0 - d)).abs() < 2e-2, "{d}: {}", read(d));
        }
        for d in [3.0f32, 4.0] {
            let (lo, hi) = (read(d - 1e-3), read(d + 1e-3));
            assert!((lo - hi).abs() < 1e-2, "{d}: {lo} against {hi}");
        }
        // Past 4 frames it is the sinc of FlatLine::read.
        for d in [4.0f32, 6.3, 17.5] {
            assert_eq!(read(d), m.read(d), "{d}");
        }
        // The weights always sum to 1.
        for k in 0..=80 {
            let (_, w) = flat_weights(1.0 + k as f32 * 0.05, 60.0);
            assert!((w.iter().sum::<f32>() - 1.0).abs() < 1e-5, "{k}");
        }
    }

    #[test]
    fn a_synced_lfo_lands_on_the_bar_when_the_transport_starts() {
        let mut lfo = Lfo::default();
        let ctx = Ctx {
            bpm: 120.0,
            beats: 6.0,
            playing: true,
            ..Ctx::new(48_000, 0)
        };
        // One cycle a bar, starting two beats into the bar: half a cycle.
        lfo.begin(&ctx, 5, f64::INFINITY);
        assert!((lfo.phase - 0.5).abs() < 1e-12);
        lfo.set(1.0, 5, 48_000);
        // A bar at 120 BPM is 2 s.
        assert!((lfo.inc * 96_000.0 - 1.0).abs() < 1e-9);
        // Already playing and on the song: nothing to catch up.
        lfo.begin(&ctx, 5, f64::INFINITY);
        assert_eq!((lfo.phase, lfo.fix_left), (0.5, 0));
    }

    #[test]
    fn a_synced_lfo_glides_back_onto_the_song_after_a_jump() {
        let mut lfo = Lfo::default();
        let at = |beats: f64| Ctx {
            bpm: 120.0,
            beats,
            playing: true,
            ..Ctx::new(48_000, 0)
        };
        lfo.begin(&at(0.0), 5, f64::INFINITY);
        lfo.set(1.0, 5, 48_000);
        // Play a block, then the song jumps back 1.5 beats (a loop wrap):
        // the phase doesn't step, it catches up over RELOCK_MS.
        for _ in 0..128 {
            lfo.next();
        }
        let before = lfo.phase;
        let song = 128.0 / 24_000.0 - 1.5;
        lfo.begin(&at(song), 5, f64::INFINITY);
        assert_eq!(lfo.phase, before, "no step");
        let frames = (RELOCK_MS * 48.0) as usize;
        let mut last = lfo.phase;
        for _ in 0..frames {
            let p = f64::from(lfo.next());
            let d = (p - last + 0.5).rem_euclid(1.0) - 0.5;
            // Never more than the jump spread over the catch-up a frame.
            assert!(d.abs() < lfo.inc + 0.375 / frames as f64 + 1e-6);
            last = p;
        }
        let want = ((song + frames as f64 / 24_000.0) / 4.0).rem_euclid(1.0);
        assert!(
            (lfo.phase - want).abs() < 1e-9,
            "{} against {want}",
            lfo.phase
        );
        assert_eq!(lfo.fix_left, 0);
    }

    #[test]
    fn a_catch_up_can_be_held_to_a_rate() {
        let mut lfo = Lfo::default();
        let at = |beats: f64| Ctx {
            bpm: 120.0,
            beats,
            playing: true,
            ..Ctx::new(48_000, 0)
        };
        lfo.begin(&at(0.0), 5, 1e-5);
        lfo.set(1.0, 5, 48_000);
        // Half a cycle to catch up at most 1e-5 cycles a frame: 50,000
        // frames, more than a second, so held to a second.
        lfo.begin(&at(2.0), 5, 1e-5);
        assert_eq!(lfo.fix_left, 48_000);
        // A quarter cycle: 25,000 frames.
        let mut lfo = Lfo::default();
        lfo.begin(&at(0.0), 5, 1e-5);
        lfo.begin(&at(1.0), 5, 1e-5);
        assert_eq!(lfo.fix_left, 25_000);
        assert!((lfo.fix_step - 1e-5).abs() < 1e-12);
    }

    /// The flat read's response at `hz` (rate 44.1 kHz) for a fraction
    /// `t` of a frame, from the table as `FlatLine::read` blends it.
    fn flat_db(t: f32, hz: f64) -> f64 {
        let p = t * PHASES as f32;
        let r = (p as usize).min(PHASES - 1);
        let a = p - r as f32;
        let (mut re, mut im) = (0.0f64, 0.0f64);
        for k in 0..TAPS {
            let h = f64::from(READ_TABLE.taps[r][k] + a * READ_TABLE.step[r][k]);
            let w = 2.0 * std::f64::consts::PI * hz / 44_100.0 * k as f64;
            re += h * w.cos();
            im -= h * w.sin();
        }
        10.0 * (re * re + im * im).log10()
    }

    #[test]
    fn the_flat_read_is_the_same_at_every_fraction() {
        // What matters for a moving read is that every fraction sounds
        // the same, so the highs don't flutter as the sweep crosses
        // them: within 0.2 dB of each other to 16 kHz. The read itself
        // tilts a little at the top (0.3 dB at 14 kHz, 1 dB at 16), the
        // same everywhere, and passes DC exactly.
        let fractions: Vec<f32> = (0..=200).map(|i| i as f32 / 200.0 * 0.999_9).collect();
        for &t in &fractions {
            assert!(flat_db(t, 0.0).abs() < 1e-5, "DC at {t}");
        }
        for k in 0..=160 {
            let hz = 100.0 * k as f64;
            let dbs: Vec<f64> = fractions.iter().map(|&t| flat_db(t, hz)).collect();
            let lo = dbs.iter().copied().fold(f64::MAX, f64::min);
            let hi = dbs.iter().copied().fold(f64::MIN, f64::max);
            assert!(hi - lo < 0.2, "{hz} Hz: {lo} to {hi} dB across fractions");
            assert!(hi < 0.1, "{hz} Hz: no gain ({hi} dB)");
            let floor = if hz <= 14_000.0 { -0.35 } else { -1.1 };
            assert!(lo > floor, "{hz} Hz: {lo} dB");
        }
    }

    #[test]
    fn the_flat_read_moves_smoothly_through_whole_frames() {
        let mut m = FlatLine::new(64);
        let mut s = 7u32;
        for _ in 0..100 {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            m.push((s >> 9) as f32 / (1u32 << 22) as f32 - 1.0);
        }
        // Either side of a whole frame, and either side of a table row,
        // the read barely moves: no step for a sweep to click on.
        for d in [10.0f32, 10.5, 11.0, 23.0, 10.0 + 1.0 / PHASES as f32] {
            let (lo, hi) = (m.read(d - 1e-3), m.read(d + 1e-3));
            assert!((lo - hi).abs() < 0.01, "{d}: {lo} against {hi}");
        }
        // An impulse comes out at its delay, and DC passes exactly.
        let mut m = FlatLine::new(64);
        for k in 0..40 {
            m.push(if k == 20 { 1.0 } else { 0.0 });
        }
        let peak = (8..30)
            .max_by(|&a, &b| m.read(a as f32).total_cmp(&m.read(b as f32)))
            .unwrap();
        assert_eq!(peak, 19);
        let mut m = FlatLine::new(64);
        for _ in 0..40 {
            m.push(0.5);
        }
        for d in [4.0f32, 9.25, 17.5, 30.999] {
            assert!((m.read(d) - 0.5).abs() < 1e-6, "{d}: {}", m.read(d));
        }
    }
}
