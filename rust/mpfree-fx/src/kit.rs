//! Small parts the newer effects share: a parameter builder, a tempo-aware
//! LFO with several shapes, a fractional delay line, a polynomial sine, a
//! Linkwitz-Riley crossover and a peak envelope follower.
//!
//! Everything here is plain arithmetic (no library calls per sample), so a
//! render is the same on every machine and cheap on the MPC's ARM.

use mpfree_node::{Ctx, Curve, ParamInfo, Unit};

use crate::flush;

/// A parameter description, shorter to write than the struct.
#[allow(clippy::too_many_arguments)]
pub(crate) const fn param(
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

/// A read-only meter (the node writes it; presets and automation leave
/// it alone).
pub(crate) const fn meter(
    id: u16,
    name: &'static str,
    group: &'static str,
    min: f32,
    max: f32,
    unit: Unit,
) -> ParamInfo {
    ParamInfo {
        id,
        name,
        group,
        min,
        max,
        default: 0.0,
        unit,
        curve: Curve::Linear,
        readonly: true,
    }
}

/// A sine of `phase` cycles (0 to 1): a parabola with a cubic correction
/// (error under 0.1 %).
#[inline]
pub(crate) fn sine(phase: f32) -> f32 {
    let p = wrap(phase);
    let x = if p < 0.5 { p } else { p - 1.0 };
    let y = 8.0 * x - 16.0 * x * x.abs();
    0.225 * (y * y.abs() - y) + y
}

/// A cosine of `phase` cycles.
#[inline]
pub(crate) fn cosine(phase: f32) -> f32 {
    sine(phase + 0.25)
}

/// `p` wrapped into 0 to 1 (any sign), with no library call.
#[inline]
pub(crate) fn wrap(p: f32) -> f32 {
    let f = p - (p as i32) as f32;
    if f < 0.0 {
        f + 1.0
    } else {
        f
    }
}

/// The LFO's tempo choices.
pub(crate) const SYNCS: &[&str] = &[
    "Free", "1/32", "1/16T", "1/16", "1/8T", "1/8", "1/8D", "1/4T", "1/4", "1/4D", "1/2", "1 bar",
    "2 bars", "4 bars",
];
/// Beats per LFO cycle for each sync choice (0 is free).
pub(crate) const SYNC_BEATS: [f64; 14] = [
    0.0,
    0.125,
    1.0 / 6.0,
    0.25,
    1.0 / 3.0,
    0.5,
    0.75,
    2.0 / 3.0,
    1.0,
    1.5,
    2.0,
    4.0,
    8.0,
    16.0,
];

/// The LFO's shapes.
pub(crate) const SHAPES: &[&str] = &[
    "Sine",
    "Triangle",
    "Square",
    "Ramp up",
    "Ramp down",
    "Random",
];

/// A tempo-aware LFO: free in Hz or synced to the beat, restarting in time
/// with the bar when the transport starts. Its square is softened (a few
/// milliseconds per edge at musical rates) and its random steps glide, so
/// no shape clicks.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Lfo {
    phase: f64,
    inc: f64,
    bpm: f64,
    playing: bool,
    held: f32,
    prev: f32,
    rng: u32,
}

impl Default for Lfo {
    fn default() -> Lfo {
        Lfo {
            phase: 0.0,
            inc: 0.0,
            bpm: 120.0,
            playing: false,
            held: 0.0,
            prev: 0.0,
            rng: 0x1234_5678,
        }
    }
}

impl Lfo {
    /// At the start of a block: follow the tempo and, when synced, put the
    /// phase where the bar says as the transport starts.
    pub(crate) fn begin(&mut self, ctx: &Ctx, sync: usize) {
        if ctx.bpm > 0.0 {
            self.bpm = ctx.bpm;
        }
        let sync = sync.min(SYNC_BEATS.len() - 1);
        if sync > 0 && ctx.playing && !self.playing {
            self.phase = (ctx.beats / SYNC_BEATS[sync]).rem_euclid(1.0);
        }
        self.playing = ctx.playing;
    }

    /// Sets the speed: `hz` when free, else the sync choice at the tempo.
    pub(crate) fn set(&mut self, hz: f32, sync: usize, rate: u32) {
        let sync = sync.min(SYNC_BEATS.len() - 1);
        let cps = if sync > 0 {
            self.bpm / 60.0 / SYNC_BEATS[sync]
        } else {
            f64::from(hz)
        };
        self.inc = cps / f64::from(rate.max(1));
    }

    /// Steps one frame and returns the phase (0 to 1).
    #[inline]
    pub(crate) fn step(&mut self) -> f32 {
        self.phase += self.inc;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
            // A new random target each cycle.
            let mut x = self.rng;
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            self.rng = x;
            self.prev = self.held;
            self.held = (x >> 8) as f32 / (1u32 << 23) as f32 - 1.0;
        }
        self.phase as f32
    }

    /// The shape's value (-1 to 1) at `phase` plus `offset` cycles.
    #[inline]
    pub(crate) fn shape(&self, shape: usize, phase: f32, offset: f32) -> f32 {
        let p = wrap(phase + offset);
        match shape {
            1 => 1.0 - 4.0 * (p - 0.5).abs(),
            2 => (sine(p) * 6.0).clamp(-1.0, 1.0),
            3 => 2.0 * p - 1.0,
            4 => 1.0 - 2.0 * p,
            5 => {
                // A glide from the last random value to this cycle's
                // (a raised-cosine step over the cycle).
                let t = 0.5 - 0.5 * cosine(p * 0.5);
                self.prev + (self.held - self.prev) * t
            }
            _ => sine(p),
        }
    }

    pub(crate) fn reset(&mut self) {
        self.phase = 0.0;
        self.playing = false;
        self.held = 0.0;
        self.prev = 0.0;
        self.rng = 0x1234_5678;
    }
}

/// The Mix curves every delay and reverb offers (its "Mix curve" control,
/// FX quality plan 8.1.1). Equal (equal power, the default) keeps the
/// level as Mix turns: the wet signal is a different moment of the music,
/// so the powers add. Dry first keeps the dry at full to half way and only
/// adds the wet, then fades the dry. Linear is a plain crossfade, which
/// dips 3 dB half way. The names are short so the page can show them in a
/// small chip; the hint line says the rest.
pub(crate) const MIX_CURVES: &[&str] = &["Equal", "Dry first", "Linear"];

/// What the hint line says about each of [`MIX_CURVES`].
pub(crate) const MIX_CURVE_HINTS: [&str; 3] = [
    "Equal (equal power): the level stays the same as you turn Mix",
    "Dry first: up to half way Mix only adds the effect, then the dry fades",
    "Linear: a plain crossfade, quieter in the middle",
];

/// The dry and wet gains for Mix `m` (0 to 1) on curve `curve` (an index
/// into [`MIX_CURVES`]). Mix 0 is exactly (1, 0) and Mix 1 exactly (0, 1)
/// on every curve, so 0 % is the input bit for bit and 100 % the effect
/// alone.
#[inline]
pub(crate) fn mix_gains(m: f32, curve: usize) -> (f32, f32) {
    let m = at_most(at_least(m, 0.0), 1.0);
    match curve {
        1 => (at_most(2.0 - 2.0 * m, 1.0), at_most(2.0 * m, 1.0)),
        2 => (1.0 - m, m),
        // A quarter cycle: cos and sin of m pi/2.
        _ => (cosine(0.25 * m), sine(0.25 * m)),
    }
}

/// A delay's or reverb's dry and wet gains from its Mix and Mix curve.
/// The gains themselves glide, so a turn of Mix and a change of curve are
/// both smooth, and once they arrive a still knob costs two compares.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Blend {
    mix: f32,
    curve: usize,
    dry: mpfree_node::Smooth,
    wet: mpfree_node::Smooth,
}

impl Blend {
    /// Settled at Mix `mix` (0 to 1) on `curve`.
    pub(crate) fn new(mix: f32, curve: usize) -> Blend {
        let curve = curve.min(MIX_CURVES.len() - 1);
        let (d, w) = mix_gains(mix, curve);
        Blend {
            mix,
            curve,
            dry: mpfree_node::Smooth::new(d),
            wet: mpfree_node::Smooth::new(w),
        }
    }
    /// The Mix, 0 to 1.
    pub(crate) fn set_mix(&mut self, mix: f32) {
        self.mix = mix;
        self.aim();
    }
    /// The curve, as the control's value (an index into [`MIX_CURVES`]).
    pub(crate) fn set_curve(&mut self, curve: f32) {
        self.curve = (at_least(curve, 0.0) as usize).min(MIX_CURVES.len() - 1);
        self.aim();
    }
    fn aim(&mut self) {
        let (d, w) = mix_gains(self.mix, self.curve);
        self.dry.target = d;
        self.wet.target = w;
    }
    pub(crate) fn set_time(&mut self, rate: u32, ms: f32) {
        self.dry.set_time(rate, ms);
        self.wet.set_time(rate, ms);
    }
    /// Jumps to where the controls are (on reset).
    pub(crate) fn snap(&mut self) {
        self.dry.snap(self.dry.target);
        self.wet.snap(self.wet.target);
    }
    /// The next frame's (dry, wet) gains.
    #[inline]
    pub(crate) fn next(&mut self) -> (f32, f32) {
        (self.dry.glide(), self.wet.glide())
    }
    /// Whether it puts out the effect alone, and will (Mix 100 %, settled).
    pub(crate) fn all_wet(&self) -> bool {
        self.dry.value == 0.0 && self.dry.target == 0.0 && self.wet.value == 1.0
    }
}

/// `dry * x + wet * y`: one frame of a [`Blend`].
#[inline]
pub(crate) fn blend(x: f32, y: f32, (dry, wet): (f32, f32)) -> f32 {
    dry * x + wet * y
}

/// [`Smooth::next`] with a quick way out once the value has arrived: the
/// same values bit for bit, at about a third of the cost on ARMv7 while
/// the control holds still (most of the time).
pub(crate) trait Glide {
    fn glide(&mut self) -> f32;
}

impl Glide for mpfree_node::Smooth {
    #[inline]
    fn glide(&mut self) -> f32 {
        if self.value == self.target {
            self.value
        } else {
            self.next()
        }
    }
}

/// `x`, or `lo` if `x` is lower (or not a number). Cheaper than
/// `f32::max` on ARMv7, which has no single instruction for it.
#[inline]
pub(crate) fn at_least(x: f32, lo: f32) -> f32 {
    if x > lo {
        x
    } else {
        lo
    }
}

/// `x`, or `hi` if `x` is higher.
#[inline]
pub(crate) fn at_most(x: f32, hi: f32) -> f32 {
    if x < hi {
        x
    } else {
        hi
    }
}

/// A mono delay line read at a fractional point (cubic interpolation).
///
/// The buffer is a power of two long (at least 8) and every index is
/// masked by its length less one, so the compiler drops the bounds checks.
/// A read takes its four neighbours as one slice when they don't wrap
/// (all but three points in the ring), which on the MPC's ARM is about
/// half the address work of four masked indices.
#[derive(Clone, Debug, Default)]
pub(crate) struct Line {
    buf: Vec<f32>,
    w: usize,
    /// The longest delay a read can ask for, as a float.
    top: f32,
}

impl Line {
    /// Room for `frames` of delay (control thread).
    pub(crate) fn new(frames: usize) -> Line {
        let n = (frames + 4).next_power_of_two().max(8);
        Line {
            buf: vec![0.0; n],
            w: 0,
            top: (n - 4) as f32,
        }
    }
    /// The longest delay a read can ask for.
    pub(crate) fn max(&self) -> f32 {
        self.top
    }
    #[inline]
    pub(crate) fn push(&mut self, x: f32) {
        let n = self.buf.len();
        if n == 0 {
            return;
        }
        self.buf[self.w & (n - 1)] = x;
        self.w = (self.w + 1) & (n - 1);
    }
    /// The four frames from `i + 2` back to `i - 1` before the newest,
    /// oldest first (the line must be built).
    #[inline]
    fn four(&self, i: usize) -> [f32; 4] {
        let buf = &self.buf[..];
        let m = buf.len() - 1;
        let j = self.w.wrapping_sub(i).wrapping_sub(3) & m;
        if j + 3 <= m {
            let s = &buf[j..j + 4];
            [s[0], s[1], s[2], s[3]]
        } else {
            [buf[j], buf[(j + 1) & m], buf[(j + 2) & m], buf[(j + 3) & m]]
        }
    }
    /// The signal `d` frames before the last write (at least 1),
    /// interpolated.
    #[inline]
    pub(crate) fn read(&self, d: f32) -> f32 {
        if self.buf.len() < 8 {
            return 0.0;
        }
        let d = at_most(at_least(d, 1.0), self.top);
        let i = d as usize;
        let t = d - i as f32;
        let [x3, x2, x1, x0] = self.four(i);
        let c1 = 0.5 * (x2 - x0);
        let c2 = x0 - 2.5 * x1 + 2.0 * x2 - 0.5 * x3;
        let c3 = 0.5 * (x3 - x0) + 1.5 * (x1 - x2);
        ((c3 * t + c2) * t + c1) * t + x1
    }
    /// The frame `d - 1` frames before the newest (`tap(1)` is the newest).
    #[inline]
    pub(crate) fn tap(&self, d: usize) -> f32 {
        let buf = &self.buf[..];
        let n = buf.len();
        if n == 0 {
            return 0.0;
        }
        let d = d.clamp(1, n - 1);
        buf[self.w.wrapping_sub(d) & (n - 1)]
    }
    pub(crate) fn clear(&mut self) {
        self.buf.fill(0.0);
        self.w = 0;
    }
}

/// The transition band of an [`Os2`]'s half-bands (of the doubled rate).
/// Three first-order sections make each: the pair is an all-pass, so the
/// band stays flat whatever the design, and the stop band only has to
/// hold a loop saturator's harmonics, which fall away fast.
const OS2_TBW: f64 = 0.1;

/// One first-order all-pass section, `y = a (x - y1) + x1`, its state
/// `[x1, y1]`.
#[inline(always)]
fn ap1(a: f32, s: &mut [f32; 2], x: f32) -> f32 {
    let y = flush(a * (x - s[1]) + s[0]);
    *s = [x, y];
    y
}

/// A shaper in a feedback loop, run at twice the rate (FX quality, D5):
/// the harmonics it makes above the base Nyquist are filtered out instead
/// of folding back as aliases, which a loop would play again on every
/// repeat. Polyphase IIR half-bands of three sections each way (the
/// design of [`crate::oversample::IirUp2`], unrolled). With a straight
/// line inside, up then down is one all-pass, so the loop keeps its whole
/// band (no hidden loss); it is [`Os2::lag`] late at low frequencies
/// (about 1.3 frames) and later toward the top, which a repeat gathers
/// once a pass unless the loop reads that much sooner.
#[derive(Clone, Debug)]
pub(crate) struct Os2 {
    /// The sections' coefficients: 0 and 2 make the even branch, 1 the odd.
    c: [f32; 3],
    up: [[f32; 2]; 3],
    down: [[f32; 2]; 3],
    lag: f32,
}

impl Os2 {
    pub(crate) fn new() -> Os2 {
        let k = crate::oversample::iir_halfband(3, OS2_TBW);
        let c = [k[0], k[1], k[2]];
        // Up then down is the product of every section at the base rate,
        // each `(a + z^-1) / (1 + a z^-1)`, `(1 - a) / (1 + a)` frames late
        // at low frequencies.
        let lag = c.iter().map(|a| (1.0 - a) / (1.0 + a)).sum();
        Os2 {
            c,
            up: [[0.0; 2]; 3],
            down: [[0.0; 2]; 3],
            lag,
        }
    }
    /// How late the stage is at low frequencies, frames (about 1.3).
    pub(crate) fn lag(&self) -> f32 {
        self.lag
    }
    /// `shape` applied to `x` at twice the rate.
    #[inline]
    pub(crate) fn run(&mut self, x: f32, shape: impl Fn(f32) -> f32) -> f32 {
        let c = self.c;
        // Up: both branches see the input; the even one makes the first
        // of the two samples, the odd one the second.
        let e = ap1(c[0], &mut self.up[0], x);
        let e = ap1(c[2], &mut self.up[2], e);
        let o = ap1(c[1], &mut self.up[1], x);
        let (a, b) = (shape(e), shape(o));
        // Down: the second sample through the even branch, the first
        // through the odd one, averaged.
        let e = ap1(c[0], &mut self.down[0], b);
        let e = ap1(c[2], &mut self.down[2], e);
        let o = ap1(c[1], &mut self.down[1], a);
        0.5 * (e + o)
    }
    pub(crate) fn reset(&mut self) {
        self.up = [[0.0; 2]; 3];
        self.down = [[0.0; 2]; 3];
    }
}

/// How long a [`FlatRead`] takes to swap between its two reads, ms.
const SWAP_MS: f32 = 3.0;

/// A read of a [`Line`] that keeps the whole band while its delay holds
/// still and follows it smoothly while it moves (FX quality, D2). Still,
/// the fraction of a frame is a first-order all-pass (Thiran): flat in
/// level at every frequency and exact in time at low ones, where the
/// cubic loses up to 0.7 dB at 10 kHz and 3.5 dB at 15 kHz half way
/// between frames, which a feedback loop compounds on every repeat (and
/// so made the repeats' brightness depend on the tempo). Moving, it is
/// the cubic, which follows a glide where an all-pass would ring. Each
/// time the delay starts or stops moving it crossfades between the two
/// over [`SWAP_MS`]; the all-pass's state is the read's own last output,
/// so it picks up where the cubic left off.
#[derive(Clone, Copy, Debug)]
pub(crate) struct FlatRead {
    /// The last output: the all-pass's state.
    last: f32,
    /// How far over to the all-pass: 0 the cubic, 1 the all-pass.
    k: f32,
    step: f32,
    /// The still delay, its whole frames and its coefficient, worked out
    /// once.
    d: f32,
    i: usize,
    eta: f32,
}

impl Default for FlatRead {
    fn default() -> FlatRead {
        FlatRead {
            last: 0.0,
            k: 0.0,
            step: 1.0,
            d: f32::NAN,
            i: 0,
            eta: 0.0,
        }
    }
}

impl FlatRead {
    pub(crate) fn set_rate(&mut self, rate: u32) {
        self.step = 1.0 / at_least(SWAP_MS * 0.001 * rate as f32, 1.0);
    }
    /// A fresh read, already on the all-pass when `still`.
    pub(crate) fn reset(&mut self, still: bool) {
        self.last = 0.0;
        self.k = if still { 1.0 } else { 0.0 };
        self.d = f32::NAN;
    }
    /// `line` `d` frames before its last write, as [`Line::read`]; `still`
    /// while `d` holds still.
    #[inline]
    pub(crate) fn read(&mut self, line: &Line, d: f32, still: bool) -> f32 {
        if still {
            if self.k < 1.0 {
                self.k = at_most(self.k + self.step, 1.0);
            }
        } else if self.k > 0.0 {
            self.k = at_least(self.k - self.step, 0.0);
        }
        let y = if self.k <= 0.0 {
            line.read(d)
        } else {
            let a = self.all_pass(line, d);
            if self.k >= 1.0 {
                a
            } else {
                let c = line.read(d);
                c + self.k * (a - c)
            }
        };
        self.last = y;
        y
    }
    /// The all-pass read: whole frames `i` and a fraction from 0.5 to 1.5
    /// toward `i + 1` (where its delay is nearly flat across the band),
    /// `y = at(i + 1) + eta * (at(i) - last)`.
    #[inline]
    fn all_pass(&mut self, line: &Line, d: f32) -> f32 {
        if d != self.d {
            self.d = d;
            let d = at_most(at_least(d, 1.0), line.max());
            let whole = d as usize;
            let t = d - whole as f32;
            let (i, frac) = if t >= 0.5 || whole < 2 {
                (whole, at_least(t, 0.5))
            } else {
                (whole - 1, 1.0 + t)
            };
            self.i = i;
            self.eta = (1.0 - frac) / (1.0 + frac);
        }
        // `tap(n)` is the frame n - 1 before the newest.
        flush(line.tap(self.i + 2) + self.eta * (line.tap(self.i + 1) - self.last))
    }
}

/// One 12 dB per octave Butterworth state-variable stage in Simper's
/// trapezoidal form, stereo, giving its low-pass and band-pass at once
/// (the high-pass is `x - SQRT_2 * band - low`, the all-pass
/// `x - 2 SQRT_2 * band`), so a crossover needs three stages, not four.
#[derive(Clone, Copy, Debug, Default)]
struct Stage {
    a1: f32,
    a2: f32,
    a3: f32,
    ic1: [f32; 2],
    ic2: [f32; 2],
}

/// The Butterworth stage's damping, 1 / Q.
const BUTTER_K: f32 = std::f32::consts::SQRT_2;

impl Stage {
    fn set(&mut self, hz: f32, rate: u32) {
        let g = (std::f32::consts::PI * hz.clamp(1.0, 0.45 * rate as f32) / rate as f32).tan();
        self.a1 = 1.0 / (1.0 + g * (g + BUTTER_K));
        self.a2 = g * self.a1;
        self.a3 = g * self.a2;
    }
    /// `(low, band)` for one sample of channel `ch`.
    #[inline]
    fn run(&mut self, ch: usize, v0: f32) -> (f32, f32) {
        let (ic1, ic2) = (self.ic1[ch], self.ic2[ch]);
        let v3 = v0 - ic2;
        let v1 = self.a1 * ic1 + self.a2 * v3;
        let v2 = ic2 + self.a2 * ic1 + self.a3 * v3;
        self.ic1[ch] = flush(2.0 * v1 - ic1);
        self.ic2[ch] = flush(2.0 * v2 - ic2);
        (v2, v1)
    }
    fn reset(&mut self) {
        self.ic1 = [0.0; 2];
        self.ic2 = [0.0; 2];
    }
}

/// A fourth-order Linkwitz-Riley split of one channel into a low and a
/// high band that sum back to an all-pass (flat in level): one stage
/// gives the first low-pass and high-pass together, two more square them.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Crossover {
    first: Stage,
    lo: Stage,
    hi: Stage,
}

impl Crossover {
    pub(crate) fn set(&mut self, hz: f32, rate: u32) {
        for s in [&mut self.first, &mut self.lo, &mut self.hi] {
            s.set(hz, rate);
        }
    }
    /// `(low, high)` for one sample of channel `ch`.
    #[inline]
    pub(crate) fn split(&mut self, ch: usize, x: f32) -> (f32, f32) {
        let (l1, b1) = self.first.run(ch, x);
        let h1 = x - BUTTER_K * b1 - l1;
        let (lo, _) = self.lo.run(ch, l1);
        let (l2, b2) = self.hi.run(ch, h1);
        (lo, h1 - BUTTER_K * b2 - l2)
    }
    pub(crate) fn reset(&mut self) {
        for s in [&mut self.first, &mut self.lo, &mut self.hi] {
            s.reset();
        }
    }
}

/// The all-pass a [`Crossover`] at the same frequency sums to (a
/// second-order Butterworth all-pass), for keeping the phase of a band
/// that skips that crossover in line with the bands that pass it.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct CrossoverPhase {
    stage: Stage,
}

impl CrossoverPhase {
    pub(crate) fn set(&mut self, hz: f32, rate: u32) {
        self.stage.set(hz, rate);
    }
    #[inline]
    pub(crate) fn run(&mut self, ch: usize, x: f32) -> f32 {
        let (_, band) = self.stage.run(ch, x);
        x - 2.0 * BUTTER_K * band
    }
    pub(crate) fn reset(&mut self) {
        self.stage.reset();
    }
}

/// A one-pole coefficient for a time constant of `ms` at `rate`.
pub(crate) fn coef(ms: f32, rate: u32) -> f32 {
    let n = (ms.max(0.01) * 0.001 * rate as f32).max(1.0);
    1.0 - (-1.0 / n).exp()
}

/// A peak follower with separate attack and release (one-pole each).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Follower {
    pub(crate) env: f32,
    att: f32,
    rel: f32,
}

impl Follower {
    pub(crate) fn set(&mut self, attack_ms: f32, release_ms: f32, rate: u32) {
        self.att = coef(attack_ms, rate);
        self.rel = coef(release_ms, rate);
    }
    #[inline]
    pub(crate) fn run(&mut self, x: f32) -> f32 {
        let x = x.abs();
        let k = if x > self.env { self.att } else { self.rel };
        self.env = flush(self.env + (x - self.env) * k);
        self.env
    }
    pub(crate) fn reset(&mut self) {
        self.env = 0.0;
    }
}

/// The units the builder needs most, by short name.
#[allow(unused_imports)]
pub(crate) use Curve::{Choice, Linear, Log, Toggle};
#[allow(unused_imports)]
pub(crate) use Unit::{Db, Hz, Ms, Percent};
pub(crate) const NONE: Unit = Unit::None;

/// The `Node` methods every newer effect writes the same way. The type
/// gives `v` (the values by id, meters included) and inherent methods:
/// `apply(id, value)` (clamped already), `begin(&Ctx)` once per block,
/// `run(&Ctx, from, l, r)` per piece between events (`from` is the
/// piece's first frame in the block, for reading the sidechain), `setup(rate)`, `clear()`,
/// `tail()` and `lat()`. A fourth argument names the inherent method that
/// gives [`mpfree_node::Node::rest_frames`], for an effect that has been
/// checked to sleep on silence (`chain.rs`); without it, never.
macro_rules! node_impl {
    ($t:ty, $info:expr, $params:expr $(, $rest:ident)?) => {
        impl mpfree_node::Node for $t {
            fn info(&self) -> &'static mpfree_node::NodeInfo {
                &$info
            }
            fn params(&self) -> &'static [mpfree_node::ParamInfo] {
                &$params
            }
            fn set_param(&mut self, id: u16, value: f32) {
                if let Some(p) = $params.get(id as usize) {
                    if !p.readonly {
                        let v = p.clamp(value);
                        self.v[id as usize] = v;
                        self.apply(id, v);
                    }
                }
            }
            fn param(&self, id: u16) -> f32 {
                self.v.get(id as usize).copied().unwrap_or(f32::NAN)
            }
            fn prepare(&mut self, rate: u32, _max_block: usize) {
                self.setup(rate.max(1));
            }
            fn reset(&mut self) {
                self.clear();
            }
            fn tail_frames(&self) -> u32 {
                self.tail()
            }
            fn latency(&self) -> u32 {
                self.lat()
            }
            $(
                fn rest_frames(&self) -> u32 {
                    self.$rest()
                }
            )?
            fn process(
                &mut self,
                ctx: &mpfree_node::Ctx,
                events: &[mpfree_node::Timed],
                left: &mut [f32],
                right: &mut [f32],
            ) {
                let len = left.len().min(right.len());
                self.begin(ctx);
                let mut segs = crate::svf::Segs::new(events);
                while let Some((from, to)) = segs.next(len, |ev| {
                    if let mpfree_node::NodeEvent::Param { id, value } = *ev {
                        mpfree_node::Node::set_param(self, id, value);
                    }
                }) {
                    self.run(ctx, from, &mut left[from..to], &mut right[from..to]);
                }
            }
        }
    };
}
pub(crate) use node_impl;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sine_is_close_and_wraps() {
        for i in -200..200 {
            let p = i as f32 / 100.0;
            let want = (p * std::f32::consts::TAU).sin();
            assert!((sine(p) - want).abs() < 0.002, "{p}");
        }
        assert!((cosine(0.0) - 1.0).abs() < 0.002);
    }

    #[test]
    fn every_shape_stays_in_range() {
        let mut l = Lfo::default();
        l.set(3.0, 0, 48_000);
        for _ in 0..100_000 {
            let p = l.step();
            for s in 0..SHAPES.len() {
                let v = l.shape(s, p, 0.3);
                assert!((-1.0..=1.0).contains(&v), "{s} {v}");
            }
        }
    }

    #[test]
    fn crossover_sums_flat() {
        let mut x = Crossover::default();
        x.set(1000.0, 48_000);
        // A sine well inside each band comes back at the same level.
        for hz in [100.0f32, 1000.0, 8000.0] {
            x.reset();
            let mut sum = 0.0f64;
            for n in 0..48_000 {
                let s = (n as f32 * hz / 48_000.0 * std::f32::consts::TAU).sin();
                let (l, h) = x.split(0, s);
                if n >= 24_000 {
                    sum += f64::from((l + h) * (l + h));
                }
            }
            // A unit sine's RMS is 1 / root 2.
            let level = (sum / 24_000.0).sqrt() * std::f64::consts::SQRT_2;
            assert!((level - 1.0).abs() < 0.005, "{hz}: {level}");
        }
    }

    #[test]
    fn crossover_phase_matches_the_crossover_sum() {
        let (mut x, mut p) = (Crossover::default(), CrossoverPhase::default());
        x.set(2500.0, 48_000);
        p.set(2500.0, 48_000);
        for n in 0..4800u32 {
            let s = kit_noise(n);
            let (l, h) = x.split(1, s);
            let a = p.run(1, s);
            assert!((l + h - a).abs() < 1e-4, "{n}: {} {a}", l + h);
        }
    }

    #[test]
    fn line_reads_whole_and_fractional() {
        let mut l = Line::new(16);
        for i in 0..10 {
            l.push(i as f32);
        }
        // tap(1) is the newest frame; read(d) is d frames before it.
        assert_eq!(l.tap(1), 9.0);
        assert_eq!(l.tap(3), 7.0);
        assert_eq!(l.read(3.0), 6.0);
        assert!((l.read(2.5) - 6.5).abs() < 1e-5);
    }

    #[test]
    fn every_mix_curve_ends_exactly_and_keeps_its_shape() {
        for c in 0..MIX_CURVES.len() {
            assert_eq!(mix_gains(0.0, c), (1.0, 0.0), "{}", MIX_CURVES[c]);
            assert_eq!(mix_gains(1.0, c), (0.0, 1.0), "{}", MIX_CURVES[c]);
            // Out of range is held at the ends.
            assert_eq!(mix_gains(-0.5, c), (1.0, 0.0));
            assert_eq!(mix_gains(1.5, c), (0.0, 1.0));
        }
        for i in 1..100 {
            let m = i as f32 / 100.0;
            // Equal: the powers add to 1 (within the polynomial sine).
            let (d, w) = mix_gains(m, 0);
            assert!((d * d + w * w - 1.0).abs() < 0.005, "Equal at {m}");
            // Dry first: the dry stays whole to half way, the wet from it.
            let (d, w) = mix_gains(m, 1);
            assert_eq!(d, if m <= 0.5 { 1.0 } else { 2.0 - 2.0 * m });
            assert_eq!(w, if m >= 0.5 { 1.0 } else { 2.0 * m });
            // Linear: the gains add to 1.
            let (d, w) = mix_gains(m, 2);
            assert!((d + w - 1.0).abs() < 1e-6, "Linear at {m}");
        }
    }

    #[test]
    fn a_blend_glides_to_its_gains_and_arrives_exactly() {
        let mut b = Blend::new(0.0, 0);
        b.set_time(48_000, 20.0);
        assert_eq!(b.next(), (1.0, 0.0));
        assert!(!b.all_wet());
        b.set_mix(1.0);
        let (d, w) = b.next();
        assert!(d > 0.99 && w < 0.01, "a glide, not a step: {d} {w}");
        for _ in 0..48_000 {
            b.next();
        }
        assert_eq!(b.next(), (0.0, 1.0));
        assert!(b.all_wet());
        // A change of curve glides too; a snap jumps there.
        b.set_mix(0.5);
        b.set_curve(1.0);
        assert_ne!(b.next(), (1.0, 1.0));
        b.snap();
        assert_eq!(b.next(), (1.0, 1.0));
        // A curve past the list is the last one.
        b.set_curve(9.0);
        b.snap();
        assert_eq!(b.next(), mix_gains(0.5, 2));
    }

    /// A sine's level and phase in `y` from frame `from` on: one DFT bin
    /// over a whole number of cycles.
    fn bin(y: &[f32], from: usize, cycles: usize, period: f64) -> (f64, f64) {
        let n = (cycles as f64 * period).round() as usize;
        let (mut re, mut im) = (0.0f64, 0.0f64);
        for (k, v) in y[from..from + n].iter().enumerate() {
            let a = std::f64::consts::TAU * (from + k) as f64 / period;
            re += f64::from(*v) * a.cos();
            im -= f64::from(*v) * a.sin();
        }
        ((re * re + im * im).sqrt() * 2.0 / n as f64, im.atan2(re))
    }

    /// A sine of `period` frames through `f`, which sees each input frame
    /// once: its gain in dB and its delay in frames.
    fn through(period: f64, mut f: impl FnMut(f32) -> f32) -> (f64, f64) {
        let total = 4096 + (40.0 * period) as usize;
        let x: Vec<f32> = (0..total)
            .map(|i| (std::f64::consts::TAU * i as f64 / period).sin() as f32 * 0.5)
            .collect();
        let y: Vec<f32> = x.iter().map(|&v| f(v)).collect();
        let (ax, px) = bin(&x, 4096, 20, period);
        let (ay, py) = bin(&y, 4096, 20, period);
        let mut dp = px - py;
        while dp < 0.0 {
            dp += std::f64::consts::TAU;
        }
        (
            20.0 * (ay / ax).log10(),
            dp / std::f64::consts::TAU * period,
        )
    }

    #[test]
    fn a_still_flat_read_keeps_every_frequency_and_its_time() {
        // Half a frame, a fifth and a whole number of frames.
        for d in [100.5f32, 100.2, 100.0, 37.9] {
            for period in [48.0f64, 4.8, 3.2, 2.4] {
                let mut line = Line::new(512);
                let mut r = FlatRead::default();
                r.set_rate(48_000);
                r.reset(true);
                let (db, late) = through(period, |v| {
                    line.push(v);
                    r.read(&line, d, true)
                });
                // Flat to 0.01 dB up to 20 kHz (the cubic loses 3.5 dB at
                // 15 kHz half way between frames).
                assert!(db.abs() < 0.01, "{d} frames, {period}-frame sine: {db} dB");
                // On time at low frequencies (the newest frame is 0 back,
                // so a read `d` back is `d` late).
                if period >= 48.0 {
                    let off = (late - d as f64).rem_euclid(period);
                    let off = off.min(period - off);
                    assert!(off < 0.02, "{d} frames: {late} late");
                }
            }
        }
    }

    #[test]
    fn a_flat_read_swaps_to_the_cubic_and_back_without_a_step() {
        let mut line = Line::new(4096);
        let mut r = FlatRead::default();
        r.set_rate(48_000);
        r.reset(true);
        let mut last = 0.0f32;
        let mut worst = 0.0f32;
        for i in 0..20_000 {
            let x = (i as f32 * 0.01).sin() * 0.5;
            line.push(x);
            // Still, then a glide from 1000 to 1500 frames, then still.
            let (d, still) = match i {
                0..=5_999 => (1000.0, true),
                6_000..=9_999 => (1000.0 + 500.0 * (i - 6_000) as f32 / 4_000.0, false),
                _ => (1500.0, true),
            };
            let y = r.read(&line, d, still);
            if i > 2_000 {
                worst = worst.max((y - last).abs());
            }
            last = y;
        }
        // A 0.01-radian sine of 0.5 moves at most 0.005 a frame; a glide
        // slows it a little. A step would show far above.
        assert!(worst < 0.0052, "{worst}");
    }

    #[test]
    fn os2_with_a_straight_line_is_the_iir_pair() {
        let mut o = Os2::new();
        let mut up = crate::oversample::IirUp2::new(3, OS2_TBW);
        let mut down = crate::oversample::IirDown2::new(3, OS2_TBW);
        for i in 0..4000 {
            let x = kit_noise(i);
            let a = o.run(x, |v| v);
            let b = down.run(up.run(x));
            assert!((a - b).abs() < 1e-6, "{i}: {a} {b}");
        }
    }

    #[test]
    fn os2_keeps_the_whole_band_and_lags_as_it_says() {
        for period in [96.0f64, 9.6, 4.8, 3.2, 2.6] {
            let mut o = Os2::new();
            let (db, late) = through(period, |v| o.run(v, |s| s));
            assert!(db.abs() < 0.01, "{period}-frame sine: {db} dB");
            if period >= 96.0 {
                assert!((late - o.lag() as f64).abs() < 0.02, "{late} {}", o.lag());
            }
        }
        assert!((Os2::new().lag() - 1.29).abs() < 0.01);
    }

    fn kit_noise(i: u32) -> f32 {
        let s = i.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (s >> 9) as f32 / (1u32 << 23) as f32 - 1.0
    }
}
