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
const SYNC_BEATS: [f64; 14] = [
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

/// A fixed read point on a [`Line`] with its cubic weights (and a gain)
/// worked out, for taps that move only while a control glides.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Frac {
    i: usize,
    w: [f32; 4],
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
    /// A read point `d` frames back (as in [`Line::read`]) scaled by
    /// `gain`, worked out once for a point that holds still.
    pub(crate) fn frac(&self, d: f32, gain: f32) -> Frac {
        let d = at_most(at_least(d, 1.0), self.top);
        let i = d as usize;
        let t = d - i as f32;
        let (t2, t3) = (t * t, t * t * t);
        // Oldest first, as `four` returns them.
        Frac {
            i,
            w: [
                gain * (-0.5 * t2 + 0.5 * t3),
                gain * (0.5 * t + 2.0 * t2 - 1.5 * t3),
                gain * (1.0 - 2.5 * t2 + 1.5 * t3),
                gain * (-0.5 * t + t2 - 0.5 * t3),
            ],
        }
    }
    /// The line at a point from [`Line::frac`]: the same cubic as
    /// [`Line::read`] without working out the weights.
    #[inline]
    pub(crate) fn read_at(&self, f: &Frac) -> f32 {
        if self.buf.len() < 8 {
            return 0.0;
        }
        let s = self.four(f.i);
        f.w[0] * s[0] + f.w[1] * s[1] + f.w[2] * s[2] + f.w[3] * s[3]
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
/// `tail()` and `lat()`.
macro_rules! node_impl {
    ($t:ty, $info:expr, $params:expr) => {
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
    fn a_fixed_point_reads_as_read_does() {
        let mut l = Line::new(64);
        for i in 0..40 {
            l.push(kit_noise(i));
        }
        for d in [1.0, 1.3, 2.5, 7.75, 20.01, 59.9, 500.0, -3.0] {
            let want = l.read(d) * 0.7;
            let got = l.read_at(&l.frac(d, 0.7));
            assert!((want - got).abs() < 1e-5, "{d}: {want} {got}");
        }
    }

    fn kit_noise(i: u32) -> f32 {
        let s = i.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (s >> 9) as f32 / (1u32 << 23) as f32 - 1.0
    }
}
