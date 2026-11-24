//! The filter gap plan's step F6: which real-time ladder to build, chosen
//! by measurement against the exact reference
//! ([`crate::filter_reference`]). Each candidate is a four-stage
//! transistor ladder with its curve in every stage (Huovilainen's model,
//! the circuit's behaviour), solved one of three ways, at one, two or four
//! times the rate; the pad's Ladder of today (its curve on the feedback
//! alone, solved as if straight and bent after) is measured beside them.
//!
//! The ways to solve a sample (the plan's section 3):
//!
//! - **Linearised** (way 3): each curve replaced by the straight line
//!   through 0 and its value at last sample's voltage (`tanh(v) / v`),
//!   then the loop solved exactly, once: four divisions and one more for
//!   the feedback.
//! - **Newton** (way 4, a fixed few steps): the linearised solve, then
//!   each curve replaced by its tangent at that solution and solved again,
//!   once or twice.
//!
//! Measures, all in the ladder's own units (its curve bends from about 1):
//! harmonics against the reference's at a tone under the cutoff and at it,
//! the pitch and level it sings at, its aliases near the top, staying
//! finite under wild moves, and its cost a sample.
//!
//! Run: `cargo test --release -p mpfree-fxlab ladder_study -- --ignored
//! --nocapture`.

use std::f64::consts::PI;

use crate::fft::power_spectrum;
use crate::filter_reference::{self, Curve, Ladder as RefLadder};
use crate::signals::{coherent, sine};

/// The pad's Ladder half-band coefficients (`mpfree_sampler::dsp`'s
/// `HB_EVEN`, `HB_ODD`): an IIR polyphase half-band, three all-pass
/// sections a branch.
const HB_EVEN: [f32; 3] = [0.060_297_39, 0.412_590_7, 0.772_715_6];
const HB_ODD: [f32; 3] = [0.215_971_44, 0.604_358_6, 0.923_886_1];

#[inline(always)]
fn ap(a: f32, s: &mut [f32; 2], x: f32) -> f32 {
    let y = a * (x - s[1]) + s[0];
    *s = [x, y];
    y
}

/// One 2x stage: up (one sample in, two out) and down (two in, one out).
#[derive(Clone, Copy, Default)]
struct Half {
    up: [[f32; 2]; 6],
    down: [[f32; 2]; 6],
}

impl Half {
    #[inline(always)]
    fn up(&mut self, x: f32) -> (f32, f32) {
        let [u0, u1, u2, u3, u4, u5] = &mut self.up;
        let e = ap(HB_EVEN[2], u2, ap(HB_EVEN[1], u1, ap(HB_EVEN[0], u0, x)));
        let o = ap(HB_ODD[2], u5, ap(HB_ODD[1], u4, ap(HB_ODD[0], u3, x)));
        (e, o)
    }

    #[inline(always)]
    fn down(&mut self, a: f32, b: f32) -> f32 {
        let [d0, d1, d2, d3, d4, d5] = &mut self.down;
        let e = ap(HB_EVEN[2], d2, ap(HB_EVEN[1], d1, ap(HB_EVEN[0], d0, b)));
        let o = ap(HB_ODD[2], d5, ap(HB_ODD[1], d4, ap(HB_ODD[0], d3, a)));
        0.5 * (e + o)
    }
}

/// How a candidate takes each curve.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Tanh {
    /// The library's, in f32.
    Exact,
    /// The [3/2] Padé form, reaching 1 at ±3 (Squelch's and the pad's).
    Pade,
    /// `mpfree_fx::fast_tanh` (Eigen's rational fit, within 2e-7).
    Fast,
}

impl Tanh {
    #[inline(always)]
    fn at(self, v: f32) -> f32 {
        match self {
            Tanh::Exact => v.tanh(),
            Tanh::Fast => mpfree_fx::fast_tanh(v),
            Tanh::Pade => {
                let v = v.clamp(-3.0, 3.0);
                let v2 = v * v;
                v * (27.0 + v2) / (27.0 + 9.0 * v2)
            }
        }
    }

    /// `tanh(v) / v` (1 at 0).
    #[inline(always)]
    fn ratio(self, v: f32) -> f32 {
        match self {
            Tanh::Exact => {
                if v.abs() < 1e-4 {
                    1.0 - v * v / 3.0
                } else {
                    v.tanh() / v
                }
            }
            Tanh::Fast => {
                if v.abs() < 1e-4 {
                    1.0 - v * v / 3.0
                } else {
                    mpfree_fx::fast_tanh(v) / v
                }
            }
            Tanh::Pade => {
                let v2 = v * v;
                if v2 < 9.0 {
                    (27.0 + v2) / (27.0 + 9.0 * v2)
                } else {
                    1.0 / v.abs()
                }
            }
        }
    }

    /// The slope at `v`.
    #[inline(always)]
    fn slope(self, v: f32) -> f32 {
        match self {
            Tanh::Exact => {
                let t = v.tanh();
                1.0 - t * t
            }
            Tanh::Fast => {
                let t = mpfree_fx::fast_tanh(v);
                1.0 - t * t
            }
            Tanh::Pade => {
                if v.abs() >= 3.0 {
                    0.0
                } else {
                    let v2 = v * v;
                    let d = 27.0 + 9.0 * v2;
                    // d/dv [v (27 + v²) / (27 + 9 v²)].
                    ((27.0 + 3.0 * v2) * d - (27.0 * v + v2 * v) * 18.0 * v) / (d * d)
                }
            }
        }
    }
}

/// How a candidate solves a sample.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Solve {
    /// The secant through 0 at last sample's voltages, solved once.
    Linearised,
    /// That, then this many tangent (Newton) steps.
    Newton(u8),
    /// Tangents at last sample's voltages, then this many more Newton
    /// steps (0: one solve, as cheap as the secant).
    Tangent(u8),
}

/// A candidate ladder for one channel.
#[derive(Clone, Copy)]
pub struct Candidate {
    pub solve: Solve,
    pub tanh: Tanh,
    /// 1, 2 or 4 times the rate.
    pub os: usize,
    /// The integrators' states and last sample's voltages (the curves'
    /// inputs: `u` into the first stage, then each stage's output).
    s: [f32; 4],
    v: [f32; 5],
    g: f32,
    k: f32,
    half: [Half; 2],
}

impl Candidate {
    pub fn new(solve: Solve, tanh: Tanh, os: usize) -> Candidate {
        Candidate {
            solve,
            tanh,
            os,
            s: [0.0; 4],
            v: [0.0; 5],
            g: 0.0,
            k: 0.0,
            half: [Half::default(); 2],
        }
    }

    pub fn name(&self) -> String {
        let solve = match self.solve {
            Solve::Linearised => "linearised".to_string(),
            Solve::Newton(n) => format!("newton {n}"),
            Solve::Tangent(n) => format!("tangent+{n}"),
        };
        let tanh = match self.tanh {
            Tanh::Exact => "tanh",
            Tanh::Pade => "pade",
            Tanh::Fast => "fast",
        };
        format!("{solve}, {tanh}, {}x", self.os)
    }

    pub fn set(&mut self, cutoff_hz: f32, k: f32, rate: f32) {
        let r = rate * self.os as f32;
        self.g = (std::f32::consts::PI * cutoff_hz.min(0.45 * r) / r).tan();
        self.k = k;
    }

    /// One sample at the candidate's own rate.
    #[inline(always)]
    fn step(&mut self, x: f32) -> f32 {
        let t = self.tanh;
        // Affine stand-ins `a v + b` for the five curves.
        let mut a = [0.0f32; 5];
        let mut b = [0.0f32; 5];
        if let Solve::Tangent(_) = self.solve {
            for i in 0..5 {
                let sl = t.slope(self.v[i]);
                a[i] = sl;
                b[i] = t.at(self.v[i]) - sl * self.v[i];
            }
        } else {
            for i in 0..5 {
                a[i] = t.ratio(self.v[i]);
            }
        }
        let mut v = self.solve_with(x, &a, &b);
        if let Solve::Newton(n) | Solve::Tangent(n) = self.solve {
            for _ in 0..n {
                for i in 0..5 {
                    let sl = t.slope(v[i]);
                    a[i] = sl;
                    b[i] = t.at(v[i]) - sl * v[i];
                }
                v = self.solve_with(x, &a, &b);
            }
        }
        // The integrators move on: s = 2 y - s.
        for i in 0..4 {
            self.s[i] = 2.0 * v[i + 1] - self.s[i];
        }
        self.v = v;
        v[4]
    }

    /// The loop solved with each curve `c_i(v) = a_i v + b_i`: the
    /// voltages `[u, y1, y2, y3, y4]`.
    #[inline(always)]
    fn solve_with(&self, x: f32, a: &[f32; 5], b: &[f32; 5]) -> [f32; 5] {
        let (g, k) = (self.g, self.k);
        // Stage i: y_i (1 + g a_i) = s_i + g (a_(i-1) v_(i-1) + b_(i-1) - b_i),
        // so y_i = al_i v_(i-1) + be_i; then y4 = A u + B, u = x - k y4.
        let mut al = [0.0f32; 4];
        let mut be = [0.0f32; 4];
        for i in 0..4 {
            let r = 1.0 / (1.0 + g * a[i + 1]);
            al[i] = g * a[i] * r;
            be[i] = (self.s[i] + g * (b[i] - b[i + 1])) * r;
        }
        let big_a = al[0] * al[1] * al[2] * al[3];
        let big_b = ((be[0] * al[1] + be[1]) * al[2] + be[2]) * al[3] + be[3];
        let y4 = (big_a * x + big_b) / (1.0 + k * big_a);
        let u = x - k * y4;
        let y1 = al[0] * u + be[0];
        let y2 = al[1] * y1 + be[1];
        let y3 = al[2] * y2 + be[2];
        [u, y1, y2, y3, y4]
    }

    /// One sample at the base rate.
    #[inline(always)]
    pub fn process(&mut self, x: f32) -> f32 {
        match self.os {
            1 => self.step(x),
            2 => {
                let (e, o) = self.half[0].up(x);
                let a = self.step(e);
                let b = self.step(o);
                self.half[0].down(a, b)
            }
            _ => {
                let (e, o) = self.half[0].up(x);
                let (e1, o1) = self.half[1].up(e);
                let (e2, o2) = self.half[1].up(o);
                let a1 = self.step(e1);
                let b1 = self.step(o1);
                let a2 = self.step(e2);
                let b2 = self.step(o2);
                let a = self.half[1].down(a1, b1);
                let b = self.half[1].down(a2, b2);
                self.half[0].down(a, b)
            }
        }
    }
}

/// Where a lean candidate straightens its curves.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum At {
    /// Last sample's voltages.
    Last,
    /// The integrators' states (a trapezoidal state runs half a step
    /// ahead of its output, so it predicts this sample's).
    Predict,
}

/// How a lean candidate straightens its curves.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Op {
    /// The line through 0 (`tanh(v) / v`), solved once.
    Secant(At),
    /// The tangent, solved once, then this many Newton steps.
    Tangent(At, u8),
}

/// The Padé curve's pieces at `v`: `(ratio, tanh, slope)` once `r` is
/// the reciprocal of [`den`]'s value.
#[inline(always)]
fn num(v: f32) -> f32 {
    let v2 = v * v;
    if v2 < 9.0 {
        27.0 + v2
    } else {
        1.0
    }
}

#[inline(always)]
fn den(v: f32) -> f32 {
    let v2 = v * v;
    if v2 < 9.0 {
        27.0 + 9.0 * v2
    } else {
        v.abs()
    }
}

/// The reciprocals of five values with one division.
#[inline(always)]
fn recip5(d: [f32; 5]) -> [f32; 5] {
    let p1 = d[0] * d[1];
    let p2 = p1 * d[2];
    let p3 = p2 * d[3];
    let p4 = p3 * d[4];
    let mut inv = 1.0 / p4;
    let r4 = inv * p3;
    inv *= d[4];
    let r3 = inv * p2;
    inv *= d[3];
    let r2 = inv * p1;
    inv *= d[2];
    let r1 = inv * d[0];
    let r0 = inv * d[1];
    [r0, r1, r2, r3, r4]
}

/// A lean candidate: the Padé curve in every stage, every reciprocal of a
/// sample found with one division (two with a Newton step).
#[derive(Clone, Copy)]
pub struct Lean {
    pub op: Op,
    pub os: usize,
    s: [f32; 4],
    v: [f32; 5],
    g: f32,
    k: f32,
    half: [Half; 2],
}

impl Lean {
    pub fn new(op: Op, os: usize) -> Lean {
        Lean {
            op,
            os,
            s: [0.0; 4],
            v: [0.0; 5],
            g: 0.0,
            k: 0.0,
            half: [Half::default(); 2],
        }
    }

    pub fn name(&self) -> String {
        let at = |a: At| match a {
            At::Last => "last",
            At::Predict => "pred",
        };
        match self.op {
            Op::Secant(a) => format!("lean secant {}, {}x", at(a), self.os),
            Op::Tangent(a, n) => format!("lean tangent {}+{n}, {}x", at(a), self.os),
        }
    }

    pub fn set(&mut self, cutoff_hz: f32, k: f32, rate: f32) {
        let r = rate * self.os as f32;
        self.g = (std::f32::consts::PI * cutoff_hz.min(0.45 * r) / r).tan();
        self.k = k;
    }

    /// The operating points: `[u, y1, y2, y3, y4]`.
    #[inline(always)]
    fn points(&self, at: At, x: f32) -> [f32; 5] {
        match at {
            At::Last => self.v,
            At::Predict => [
                x - self.k * self.s[3],
                self.s[0],
                self.s[1],
                self.s[2],
                self.s[3],
            ],
        }
    }

    /// Tangents at `p`: slopes `a` and offsets `b`.
    #[inline(always)]
    fn tangents(p: [f32; 5]) -> ([f32; 5], [f32; 5]) {
        let r = recip5([den(p[0]), den(p[1]), den(p[2]), den(p[3]), den(p[4])]);
        let mut a = [0.0; 5];
        let mut b = [0.0; 5];
        for i in 0..5 {
            let v = p[i];
            let v2 = v * v;
            if v2 < 9.0 {
                let t = v * (27.0 + v2) * r[i];
                let q = (9.0 - v2) * r[i];
                let sl = 9.0 * q * q;
                a[i] = sl;
                b[i] = t - sl * v;
            } else {
                a[i] = 0.0;
                b[i] = v.signum();
            }
        }
        (a, b)
    }

    #[inline(always)]
    fn step(&mut self, x: f32) -> f32 {
        let (mut a, mut b) = ([0.0f32; 5], [0.0f32; 5]);
        let mut newton = 0;
        match self.op {
            Op::Secant(at) => {
                let p = self.points(at, x);
                let r = recip5([den(p[0]), den(p[1]), den(p[2]), den(p[3]), den(p[4])]);
                for i in 0..5 {
                    a[i] = num(p[i]) * r[i];
                }
            }
            Op::Tangent(at, n) => {
                (a, b) = Self::tangents(self.points(at, x));
                newton = n;
            }
        }
        let mut v = self.solve(x, &a, &b);
        for _ in 0..newton {
            (a, b) = Self::tangents(v);
            v = self.solve(x, &a, &b);
        }
        for i in 0..4 {
            self.s[i] = 2.0 * v[i + 1] - self.s[i];
        }
        self.v = v;
        v[4]
    }

    /// The loop solved with each curve `a v + b`, one division.
    #[inline(always)]
    fn solve(&self, x: f32, a: &[f32; 5], b: &[f32; 5]) -> [f32; 5] {
        let (g, k) = (self.g, self.k);
        let c = [
            1.0 + g * a[1],
            1.0 + g * a[2],
            1.0 + g * a[3],
            1.0 + g * a[4],
        ];
        let ga = [g * a[0], g * a[1], g * a[2], g * a[3]];
        // y4 = (Q x + Bn) / (P + k Q), with P the product of the c's, Q
        // that of the ga's, Bn the stages' offsets carried through.
        let p01 = c[0] * c[1];
        let p012 = p01 * c[2];
        let big_p = p012 * c[3];
        let q = ga[0] * ga[1] * ga[2] * ga[3];
        let e = big_p + k * q;
        let inv = 1.0 / (big_p * e);
        let inv_p = e * inv;
        let inv_e = big_p * inv;
        // Each stage's reciprocal from 1 / P.
        let p23 = c[2] * c[3];
        let r0 = inv_p * c[1] * p23;
        let r1 = inv_p * c[0] * p23;
        let r2 = inv_p * p01 * c[3];
        let r3 = inv_p * p012;
        let n = [
            self.s[0] + g * (b[0] - b[1]),
            self.s[1] + g * (b[1] - b[2]),
            self.s[2] + g * (b[2] - b[3]),
            self.s[3] + g * (b[3] - b[4]),
        ];
        let be = [n[0] * r0, n[1] * r1, n[2] * r2, n[3] * r3];
        let al = [ga[0] * r0, ga[1] * r1, ga[2] * r2, ga[3] * r3];
        let bsum = ((be[0] * al[1] + be[1]) * al[2] + be[2]) * al[3] + be[3];
        // y4 = (A x + B) / (1 + k A) with A = Q / P, B = bsum.
        let y4 = (q * x + big_p * bsum) * inv_e;
        let u = x - k * y4;
        let y1 = al[0] * u + be[0];
        let y2 = al[1] * y1 + be[1];
        let y3 = al[2] * y2 + be[2];
        [u, y1, y2, y3, y4]
    }

    #[inline(always)]
    pub fn process(&mut self, x: f32) -> f32 {
        match self.os {
            1 => self.step(x),
            2 => {
                let (e, o) = self.half[0].up(x);
                let a = self.step(e);
                let b = self.step(o);
                self.half[0].down(a, b)
            }
            _ => {
                let (e, o) = self.half[0].up(x);
                let (e1, o1) = self.half[1].up(e);
                let (e2, o2) = self.half[1].up(o);
                let a1 = self.step(e1);
                let b1 = self.step(o1);
                let a2 = self.step(e2);
                let b2 = self.step(o2);
                let a = self.half[1].down(a1, b1);
                let b = self.half[1].down(a2, b2);
                self.half[0].down(a, b)
            }
        }
    }
}

/// The pad's Ladder of today, through its own code: its curve on the
/// feedback, its makeup taken out so it compares raw.
struct Today {
    l: mpfree_sampler::dsp::Ladder,
    makeup: f32,
}

impl Today {
    fn new(cutoff_hz: f32, k: f32, rate: f32) -> Today {
        let res = k / mpfree_sampler::dsp::LADDER_K;
        let mut l = mpfree_sampler::dsp::Ladder::default();
        l.set(cutoff_hz, res, 0.0, rate, 0);
        Today {
            l,
            makeup: mpfree_sampler::dsp::ladder_makeup(k),
        }
    }

    fn process(&mut self, x: f32) -> f32 {
        self.l.process(x, 0.0).0 / self.makeup
    }
}

/// What a ladder under test is.
#[derive(Clone, Copy)]
pub enum Under {
    Today,
    /// Today's with Drive (dB): its curve first, at 2x.
    TodayDriven(f32),
    Cand(Candidate),
    Lean(Lean),
}

impl Under {
    pub fn name(&self) -> String {
        match self {
            Under::Today => "today (feedback curve)".into(),
            Under::TodayDriven(d) => format!("today driven {d} dB"),
            Under::Cand(c) => c.name(),
            Under::Lean(c) => c.name(),
        }
    }
}

/// `x` through `who` at `cutoff_hz` and feedback `k`, from rest.
pub fn run(who: Under, x: &[f64], cutoff_hz: f64, k: f64, rate: u32) -> Vec<f64> {
    match who {
        Under::Today => {
            let mut t = Today::new(cutoff_hz as f32, k as f32, rate as f32);
            x.iter().map(|&v| f64::from(t.process(v as f32))).collect()
        }
        Under::TodayDriven(d) => {
            let res = k as f32 / mpfree_sampler::dsp::LADDER_K;
            let mut l = mpfree_sampler::dsp::Ladder::default();
            l.set(cutoff_hz as f32, res, d, rate as f32, 0);
            x.iter()
                .map(|&v| f64::from(l.process(v as f32, 0.0).0))
                .collect()
        }
        Under::Cand(c) => {
            let mut c = Candidate::new(c.solve, c.tanh, c.os);
            c.set(cutoff_hz as f32, k as f32, rate as f32);
            x.iter().map(|&v| f64::from(c.process(v as f32))).collect()
        }
        Under::Lean(c) => {
            let mut c = Lean::new(c.op, c.os);
            c.set(cutoff_hz as f32, k as f32, rate as f32);
            x.iter().map(|&v| f64::from(c.process(v as f32))).collect()
        }
    }
}

/// The circuit (curve in every stage, `tanh`) at 16 times the rate.
pub fn circuit(x: &[f64], cutoff_hz: f64, k: f64, rate: u32) -> Vec<f64> {
    let m = RefLadder {
        cutoff_hz,
        k,
        curve: Curve::Tanh,
        stages: true,
        makeup: 1.0,
    };
    filter_reference::render(&m, x, rate, 16)
}

/// The circuit with the Padé curve in every stage (the lean
/// candidates' own model).
pub fn circuit_pade(x: &[f64], cutoff_hz: f64, k: f64, rate: u32) -> Vec<f64> {
    let m = RefLadder {
        cutoff_hz,
        k,
        curve: Curve::PadSoftClip,
        stages: true,
        makeup: 1.0,
    };
    filter_reference::render(&m, x, rate, 16)
}

/// Today's model (curve on the feedback), exactly, at 16 times.
pub fn today_exact(x: &[f64], cutoff_hz: f64, k: f64, rate: u32) -> Vec<f64> {
    let m = RefLadder {
        cutoff_hz,
        k,
        curve: Curve::PadSoftClip,
        stages: false,
        makeup: 1.0,
    };
    filter_reference::render(&m, x, rate, 16)
}

/// The first `count` harmonics' amplitudes of `y[from..from + n]`, a
/// coherent tone on `bin`.
fn harmonics(y: &[f64], from: usize, n: usize, bin: usize, count: usize) -> Vec<f64> {
    let ps = power_spectrum(&y[from..from + n]);
    (1..=count)
        .take_while(|h| h * bin < n / 2)
        .map(|h| (4.0 * ps[h * bin]).sqrt())
        .collect()
}

/// The harmonics' error against the reference's, per cent:
/// `|H - H_ref| / |H_ref|` over the first ten (Oyama's measure, on
/// amplitudes).
fn harmonic_error(h: &[f64], r: &[f64]) -> f64 {
    let (mut e, mut s) = (0.0, 0.0);
    for (a, b) in h.iter().zip(r) {
        e += (a - b).powi(2);
        s += b * b;
    }
    100.0 * (e / s.max(1e-300)).sqrt()
}

fn db(a: f64) -> f64 {
    20.0 * a.max(1e-15).log10()
}

/// The pitch (Hz, by rising zero crossings) of `x`.
fn pitch(x: &[f64], rate: u32) -> f64 {
    let (mut first, mut last, mut count) = (None, 0.0, 0usize);
    for i in 1..x.len() {
        if x[i - 1] < 0.0 && x[i] >= 0.0 {
            let t = (i - 1) as f64 + -x[i - 1] / (x[i] - x[i - 1]);
            if first.is_none() {
                first = Some(t);
            } else {
                count += 1;
            }
            last = t;
        }
    }
    count as f64 * f64::from(rate) / (last - first.unwrap_or(0.0)).max(1e-9)
}

fn cents(hz: f64, of: f64) -> f64 {
    1200.0 * (hz / of).log2()
}

/// The candidates the study compares.
pub fn candidates() -> Vec<Under> {
    let mut v = vec![Under::Today];
    for (op, os) in [
        (Op::Tangent(At::Predict, 0), 1),
        (Op::Secant(At::Last), 2),
        (Op::Secant(At::Predict), 2),
        (Op::Tangent(At::Last, 0), 2),
        (Op::Tangent(At::Predict, 0), 2),
        (Op::Tangent(At::Predict, 1), 2),
        (Op::Secant(At::Predict), 4),
        (Op::Tangent(At::Predict, 0), 4),
    ] {
        v.push(Under::Lean(Lean::new(op, os)));
    }
    v
}

/// M1: harmonics against the circuit's, per cent, by level and feedback.
pub fn m1(rate: u32) -> String {
    let n = 8192;
    let settle = 8192;
    let mut out = String::from(
        "M1 harmonic error against the circuit with the Padé curve (%, first 10 harmonics), cutoff 1 kHz\n",
    );
    let cases: Vec<(f64, f64, f64)> = vec![
        // (tone Hz, amplitude, k)
        (200.0, 0.25, 0.0),
        (200.0, 1.0, 0.0),
        (200.0, 4.0, 0.0),
        (200.0, 1.0, 2.0),
        (200.0, 4.0, 2.0),
        (200.0, 1.0, 3.6),
        (200.0, 4.0, 3.6),
        (1000.0, 1.0, 3.6),
        (1000.0, 4.0, 3.6),
    ];
    let who = candidates();
    out.push_str(&format!("{:<28}", "tone, amp, k"));
    for w in &who {
        out.push_str(&format!(" | {:>22}", w.name()));
    }
    out.push('\n');
    for (hz, amp, k) in cases {
        let (bin, f) = coherent(hz, n, rate);
        let x = sine(f, amp, settle + n + filter_reference::TAIL, rate);
        let r = circuit_pade(&x, 1000.0, k, rate);
        let hr = harmonics(&r, settle, n, bin, 10);
        out.push_str(&format!("{:<28}", format!("{hz} Hz, {amp}, k {k}")));
        for w in &who {
            let y = run(*w, &x, 1000.0, k, rate);
            let h = harmonics(&y, settle, n, bin, 10);
            out.push_str(&format!(" | {:>22.3}", harmonic_error(&h, &hr)));
        }
        out.push('\n');
    }
    out
}

/// M2: where it sings after a click, cents against the cutoff, and how
/// loud (dB of the ladder's unit).
pub fn m2(rate: u32) -> String {
    let len = rate as usize / 2;
    let mut out =
        String::from("M2 singing after a 0.1 click: cents against the cutoff / level dB\n");
    let who = candidates();
    out.push_str(&format!("{:<20} | {:>16}", "cutoff, k", "circuit"));
    for w in &who {
        out.push_str(&format!(" | {:>22}", w.name()));
    }
    out.push('\n');
    for fc in [110.0, 1000.0, 5000.0] {
        for k in [4.2, 5.0, 6.0] {
            let mut click = vec![0.0; len + filter_reference::TAIL];
            click[10] = 0.1;
            let sing = |y: &[f64]| {
                let tail = &y[len / 2..len];
                let rms = (tail.iter().map(|v| v * v).sum::<f64>() / tail.len() as f64).sqrt();
                format!(
                    "{:+.1} / {:.1}",
                    cents(pitch(tail, rate), fc),
                    db(rms * 2f64.sqrt())
                )
            };
            let r = circuit_pade(&click, fc, k, rate);
            out.push_str(&format!(
                "{:<20} | {:>16}",
                format!("{fc} Hz, k {k}"),
                sing(&r)
            ));
            for w in &who {
                let y = run(*w, &click, fc, k, rate);
                out.push_str(&format!(" | {:>22}", sing(&y)));
            }
            out.push('\n');
        }
    }
    out
}

/// M3: aliases near the top: the loudest bin that is no harmonic of the
/// tone, dB against the tone, Blackman-Harris windowed.
pub fn m3(rate: u32) -> String {
    let n = 16_384;
    let settle = 4096;
    let mut out = String::from("M3 worst alias against the tone (dB)\n");
    let who = candidates();
    out.push_str(&format!("{:<34}", "tone, amp, cutoff, k"));
    for w in &who {
        out.push_str(&format!(" | {:>22}", w.name()));
    }
    out.push('\n');
    let w = blackman_harris(n);
    for (hz, amp, fc, k) in [
        (2500.0, 2.0, 5000.0, 3.8),
        (5000.0, 2.0, 8000.0, 2.9),
        (7000.0, 2.0, 10000.0, 3.8),
        (5000.0, 4.0, 8000.0, 2.9),
        (2500.0, 4.0, 5000.0, 3.8),
        (7000.0, 0.5, 10000.0, 3.8),
        (7000.0, 1.0, 10000.0, 3.8),
        (10000.0, 0.5, 15000.0, 3.0),
        (10000.0, 2.0, 15000.0, 3.0),
    ] {
        let (bin, f) = coherent(hz, n, rate);
        let x = sine(f, amp, settle + n, rate);
        out.push_str(&format!(
            "{:<34}",
            format!("{hz} Hz, {amp}, {fc} Hz, k {k}")
        ));
        for who in &who {
            let y = run(*who, &x, fc, k, rate);
            let seg: Vec<f64> = y[settle..settle + n]
                .iter()
                .zip(&w)
                .map(|(a, b)| a * b)
                .collect();
            let ps = power_spectrum(&seg);
            let fund = ps[bin - 2..=bin + 2].iter().cloned().fold(0.0, f64::max);
            let hz_of = |i: usize| i as f64 * f64::from(rate) / n as f64;
            let near_harm = |i: usize| {
                let h = (i as f64 / bin as f64).round() as usize;
                h >= 1 && (i as i64 - (h * bin) as i64).abs() <= 4
            };
            let worst = (1..n / 2)
                .filter(|&i| !near_harm(i) && hz_of(i) > 20.0 && hz_of(i) < 20_000.0)
                .map(|i| ps[i])
                .fold(0.0, f64::max);
            out.push_str(&format!(
                " | {:>22.1}",
                10.0 * (worst / fund).max(1e-30).log10()
            ));
        }
        out.push('\n');
    }
    out
}

fn blackman_harris(n: usize) -> Vec<f64> {
    (0..n)
        .map(|i| {
            let x = 2.0 * PI * i as f64 / n as f64;
            0.35875 - 0.48829 * x.cos() + 0.14128 * (2.0 * x).cos() - 0.01168 * (3.0 * x).cos()
        })
        .collect()
}

/// M4: loud noise while the cutoff and feedback jump about: the loudest
/// output sample (finite and bounded is the test).
pub fn m4(rate: u32) -> String {
    let mut out = String::from(
        "M4 wild moves (noise at 8, cutoff 20 Hz-20 kHz and k 0-6 jumping): loudest sample\n",
    );
    for who in candidates() {
        let Under::Lean(c) = who else { continue };
        let mut c = Lean::new(c.op, c.os);
        let mut seed = 12345u64;
        let mut rnd = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 11) as f64 / (1u64 << 53) as f64
        };
        let mut peak = 0.0f32;
        let mut finite = true;
        let mut left = 0usize;
        for _ in 0..rate as usize * 4 {
            if left == 0 {
                let fc = 20.0 * 1000f64.powf(rnd());
                let k = 6.0 * rnd();
                c.set(fc as f32, k as f32, rate as f32);
                left = 1 + (rnd() * 64.0) as usize;
            }
            left -= 1;
            let x = (8.0 * (2.0 * rnd() - 1.0)) as f32;
            let y = c.process(x);
            finite &= y.is_finite();
            peak = peak.max(y.abs());
        }
        out.push_str(&format!(
            "{:<24} finite {finite}, peak {peak:.2}\n",
            c.name()
        ));
    }
    out
}

/// M5: time a sample (ns), release builds only meaningful.
pub fn m5(rate: u32) -> String {
    let mut out = String::from("M5 time per base-rate sample, one channel (ns)\n");
    let x = crate::signals::pink(rate as usize * 4, 0.3, 7);
    for who in std::iter::once(Under::TodayDriven(12.0)).chain(candidates()) {
        let t = std::time::Instant::now();
        let y = run(who, &x, 1200.0, 3.5, rate);
        let ns = t.elapsed().as_nanos() as f64 / x.len() as f64;
        std::hint::black_box(&y);
        out.push_str(&format!("{:<24} {ns:.1}\n", who.name()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore]
    fn ladder_study() {
        let rate = 44_100;
        println!("{}", m5(rate));
        println!("{}", m4(rate));
        println!("{}", m1(rate));
        println!("{}", m2(rate));
        println!("{}", m3(rate));
    }

    /// For counting instructions under callgrind: `LADDER_CAND` picks a
    /// candidate (an index into [`candidates`], or none for the baseline).
    #[test]
    #[ignore]
    fn ladder_cost() {
        let rate = 44_100;
        let x = crate::signals::pink(rate as usize * 4, 0.3, 7);
        let who = std::env::var("LADDER_CAND")
            .ok()
            .and_then(|v| v.parse::<usize>().ok());
        let mut all = vec![Under::TodayDriven(12.0)];
        all.extend(candidates());
        if let Some(i) = who {
            let y = run(all[i], &x, 1200.0, 3.5, rate);
            std::hint::black_box(&y);
        } else {
            let y: Vec<f64> = x.iter().map(|v| v * 0.5).collect();
            std::hint::black_box(&y);
        }
    }

    #[test]
    fn a_quiet_candidate_is_the_linear_ladder() {
        // At 1e-4 every candidate is the linear TPT ladder: within 0.05
        // dB of the circuit's quiet render at 200 Hz, 1 kHz and 2 kHz (0.2
        // at the session's own rate, where the bilinear map's warp is
        // 0.15 dB at 2 kHz).
        let rate = 44_100;
        let n = 8192;
        for hz in [200.0, 1000.0, 2000.0] {
            let (bin, f) = coherent(hz, n, rate);
            let x = sine(f, 1e-4, 8192 + n + filter_reference::TAIL, rate);
            let r = harmonics(&circuit(&x, 1000.0, 2.0, rate), 8192, n, bin, 1)[0];
            for who in candidates() {
                let y = harmonics(&run(who, &x, 1000.0, 2.0, rate), 8192, n, bin, 1)[0];
                let within = match who {
                    Under::Lean(c) if c.os == 1 => 0.2,
                    _ => 0.05,
                };
                assert!(
                    (db(y) - db(r)).abs() < within,
                    "{} {hz}: {} {}",
                    who.name(),
                    db(y),
                    db(r)
                );
            }
        }
    }
}

#[cfg(test)]
mod tuning {
    use super::*;

    /// The steady singing pitch (cents against the cutoff) and level by
    /// feedback, through the lean tangent candidate at 2x.
    #[test]
    #[ignore]
    fn singing_by_feedback() {
        let rate = 44_100u32;
        for fc in [250.0f64, 1000.0, 4000.0] {
            for k in [4.0f64, 4.02, 4.05, 4.1, 4.15, 4.2, 4.3, 4.5, 5.0] {
                let n = rate as usize * 6;
                let mut x = vec![0.0; n];
                x[10] = 0.1;
                let y = run(
                    Under::Lean(Lean::new(Op::Tangent(At::Predict, 0), 2)),
                    &x,
                    fc,
                    k,
                    rate,
                );
                let tail = &y[n - rate as usize..];
                let rms = (tail.iter().map(|v| v * v).sum::<f64>() / tail.len() as f64).sqrt();
                println!(
                    "fc {fc} k {k}: {:+.2} cents, {:.1} dB",
                    cents(pitch(tail, rate), fc),
                    db(rms * 2f64.sqrt())
                );
            }
        }
    }
}

#[cfg(test)]
mod dc {
    use super::*;

    #[test]
    #[ignore]
    fn saw_dc() {
        let rate = 44_100u32;
        let period = 24usize;
        let n = period * 2000 + filter_reference::TAIL;
        let x: Vec<f64> = (0..n)
            .map(|i| 2.0 * ((i % period) as f64 + 0.5) / period as f64 - 1.0)
            .collect();
        let mean = |y: &[f64]| {
            let w = &y[period * 1000..period * 1900];
            w.iter().sum::<f64>() / w.len() as f64
        };
        for k in [0.0, 2.1] {
            let lean = run(
                Under::Lean(Lean::new(Op::Tangent(At::Predict, 0), 2)),
                &x,
                2000.0,
                k,
                rate,
            );
            let cp = circuit_pade(&x, 2000.0, k, rate);
            let ct = circuit(&x, 2000.0, k, rate);
            println!(
                "k {k}: lean {:.1} dB, circuit pade {:.1} dB, circuit tanh {:.1} dB, input {:.1}",
                db(mean(&lean).abs()),
                db(mean(&cp).abs()),
                db(mean(&ct).abs()),
                db(mean(&x).abs())
            );
        }
    }
}
