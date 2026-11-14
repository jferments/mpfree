//! Small parts the newer effects share: a parameter builder, a tempo-aware
//! LFO with several shapes, a fractional delay line, a polynomial sine, a
//! Linkwitz-Riley crossover and a peak envelope follower.
//!
//! Everything here is plain arithmetic (no library calls per sample), so a
//! render is the same on every machine and cheap on the MPC's ARM.

use mpfree_node::{Ctx, Curve, ParamInfo, Unit};

use crate::flush;
use crate::svf::{Svf, SvfMode};

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

/// A mono delay line read at a fractional point (cubic interpolation).
#[derive(Clone, Debug, Default)]
pub(crate) struct Line {
    buf: Vec<f32>,
    mask: usize,
    w: usize,
}

impl Line {
    /// Room for `frames` of delay (control thread).
    pub(crate) fn new(frames: usize) -> Line {
        let n = (frames + 4).next_power_of_two();
        Line {
            buf: vec![0.0; n],
            mask: n - 1,
            w: 0,
        }
    }
    /// The longest delay a read can ask for.
    pub(crate) fn max(&self) -> f32 {
        self.buf.len().saturating_sub(4) as f32
    }
    #[inline]
    pub(crate) fn push(&mut self, x: f32) {
        if self.buf.is_empty() {
            return;
        }
        self.buf[self.w] = x;
        self.w = (self.w + 1) & self.mask;
    }
    /// The signal `d` frames before the last write (at least 1),
    /// interpolated.
    #[inline]
    pub(crate) fn read(&self, d: f32) -> f32 {
        if self.buf.len() < 8 {
            return 0.0;
        }
        let d = d.clamp(1.0, self.max());
        let i = d as usize;
        let t = d - i as f32;
        let last = self.w.wrapping_sub(1);
        let at = |k: usize| self.buf[last.wrapping_sub(k) & self.mask];
        let (x0, x1, x2, x3) = (at(i - 1), at(i), at(i + 1), at(i + 2));
        let c1 = 0.5 * (x2 - x0);
        let c2 = x0 - 2.5 * x1 + 2.0 * x2 - 0.5 * x3;
        let c3 = 0.5 * (x3 - x0) + 1.5 * (x1 - x2);
        ((c3 * t + c2) * t + c1) * t + x1
    }
    /// The frame `d - 1` frames before the newest (`tap(1)` is the newest).
    #[inline]
    pub(crate) fn tap(&self, d: usize) -> f32 {
        if self.buf.is_empty() {
            return 0.0;
        }
        let d = d.clamp(1, self.mask);
        self.buf[self.w.wrapping_sub(d) & self.mask]
    }
    pub(crate) fn clear(&mut self) {
        self.buf.fill(0.0);
        self.w = 0;
    }
}

/// A fourth-order Linkwitz-Riley split of one channel into a low and a
/// high band that sum back to an all-pass (flat in level).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Crossover {
    lp: [Svf; 2],
    hp: [Svf; 2],
}

impl Crossover {
    pub(crate) fn set(&mut self, hz: f32, rate: u32) {
        let q = std::f32::consts::FRAC_1_SQRT_2;
        for s in &mut self.lp {
            s.set(SvfMode::LowPass, hz, q, 0.0, rate);
        }
        for s in &mut self.hp {
            s.set(SvfMode::HighPass, hz, q, 0.0, rate);
        }
    }
    /// `(low, high)` for one sample of channel `ch`.
    #[inline]
    pub(crate) fn split(&mut self, ch: usize, x: f32) -> (f32, f32) {
        let lo = self.lp[0].run(ch, x);
        let lo = self.lp[1].run(ch, lo);
        let hi = self.hp[0].run(ch, x);
        let hi = self.hp[1].run(ch, hi);
        (lo, hi)
    }
    pub(crate) fn reset(&mut self) {
        for s in self.lp.iter_mut().chain(&mut self.hp) {
            s.reset();
        }
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
}

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
