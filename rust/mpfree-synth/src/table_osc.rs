//! Reading the wavetables ([`crate::wavetable::table`]) as an oscillator:
//! where a voice reads during one control step ([`Run`]: two neighbouring
//! frames of one level and the crossfade between them) and how it bends
//! its phase first ([`Warp`]). Poly's Table wave reads a control step at a
//! time ([`render`], [`render_fm_pair`]); the Wavetable synth reads a frame
//! at a time ([`Run::read`]).
//!
//! A level's length is a power of two, so an unbent phase (32 bits, as
//! Poly's other waves keep it, [`crate::osc::phase`]) indexes it with a
//! shift and no float-to-integer conversion. A bent one is worked out as a
//! fraction of the cycle, and its index never passes the level's last
//! sample, so the read stays inside its frame whatever the warp gives.
//!
//! Sync restarts the read at each wrap of the running phase, and each
//! restart gets a band-limited correction ([`Restart`], [`Edges`]; DSP
//! quality step 1.1). Poly's sync read keeps its 32-bit phase: the read's
//! fraction of the cycle as a 32-bit phase, indexed as an unbent one is.

use std::sync::OnceLock;

use crate::wavetable::table::Table;

/// The fraction of a cycle in `x` (0 or more, under 2^24): `x.fract()`
/// to the bit, without the library call `fract` makes on some targets
/// (a sync read's phase is under 8 cycles).
#[inline(always)]
pub(crate) fn cycle(x: f32) -> f32 {
    x - x as i32 as f32
}

/// Frames per control step: both engines step every 16 frames.
const STEP: f32 = 16.0;
/// One cycle of a 32-bit phase, as a float.
const ONE: f32 = 4_294_967_296.0;
/// The highest phase a warp reads from.
const UNDER_ONE: f32 = 0.999_999;
/// How much faster than its ratio a sync read picks its table level
/// ([`Warp::speed`]).
const SYNC_MARGIN: f32 = 1.5;
/// How far a sync ratio may move before a restart's jumps are worked out
/// again ([`Restart`]).
const RATIO_HOLD: f32 = 1e-5;

/// Half the width of a sync restart's correction (samples each side of
/// the restart), and its tables' points a sample.
const EDGE_HALF: usize = 12;
const EDGE_RES: usize = 128;
/// The correction's kernel cutoff (cycles a sample): a windowed sinc a
/// little under half the rate, so most of its transition band lies below
/// the Nyquist frequency and little above it folds back.
const EDGE_CUTOFF: f64 = 0.43;
/// Points a sample the tables are worked out on before they are thinned
/// to [`EDGE_RES`].
const EDGE_FINE: usize = 1024;

/// A sync restart's residuals ([`Restart`]): for a jump in the read's
/// level and in its first three derivatives, the band-limited jump less
/// the naive one, `[level, slope, curve, cubic]` at [`EDGE_RES`] points a
/// sample, before the restart (`-EDGE_HALF` to 0) and after it (0 to
/// `EDGE_HALF`). The two halves meet at 0, where the level's residual
/// jumps by 1, so neither is read across it.
pub(crate) struct Edges {
    before: Box<[[f32; 4]]>,
    after: Box<[[f32; 4]]>,
}

impl Edges {
    /// The band-limited jumps are running integrals of the kernel, a
    /// Blackman-Harris windowed sinc: `B0` its integral (the band-limited
    /// step), `B1` to `B3` the integrals of each. A jump of `x^m / m!` at
    /// 0 band-limits to `Bm`, so its residual is `Bm(x) - x^m / m!` after
    /// the restart and `Bm(x)` before. The kernel's second moment is taken
    /// out first (a touch of its own second derivative), so the curve's
    /// and the cubic's residuals end at 0 rather than in a constant and a
    /// ramp past the window.
    fn build() -> Edges {
        use std::f64::consts::PI;
        let half = EDGE_HALF as f64;
        let n = 2 * EDGE_HALF * EDGE_FINE + 1;
        let du = 1.0 / EDGE_FINE as f64;
        let x = |i: usize| i as f64 * du - half;
        let mut k: Vec<f64> = (0..n)
            .map(|i| {
                let u = x(i);
                let sinc = if i == EDGE_HALF * EDGE_FINE {
                    2.0 * EDGE_CUTOFF
                } else {
                    (2.0 * PI * EDGE_CUTOFF * u).sin() / (PI * u)
                };
                let w = PI * (u + half) / half;
                let win = 0.35875 - 0.48829 * w.cos() + 0.14128 * (2.0 * w).cos()
                    - 0.01168 * (3.0 * w).cos();
                sinc * win
            })
            .collect();
        let area: f64 = k.iter().sum::<f64>() * du;
        k.iter_mut().for_each(|v| *v /= area);
        let m2: f64 = (0..n).map(|i| x(i) * x(i) * k[i]).sum::<f64>() * du;
        let curve: Vec<f64> = (0..n)
            .map(|i| {
                if i == 0 || i == n - 1 {
                    0.0
                } else {
                    (k[i + 1] - 2.0 * k[i] + k[i - 1]) / (du * du)
                }
            })
            .collect();
        for i in 0..n {
            k[i] -= 0.5 * m2 * curve[i];
        }
        // Running integrals (trapezoid): B0 from the kernel, each next
        // from the last.
        let integral = |f: &[f64]| {
            let mut out = vec![0.0f64; n];
            for i in 1..n {
                out[i] = out[i - 1] + 0.5 * (f[i - 1] + f[i]) * du;
            }
            out
        };
        let mut b = [integral(&k), vec![], vec![], vec![]];
        let total = b[0][n - 1];
        b[0].iter_mut().for_each(|v| *v /= total);
        for m in 1..4 {
            b[m] = integral(&b[m - 1]);
        }
        let fact = [1.0, 1.0, 2.0, 6.0];
        let side = |after: bool| -> Box<[[f32; 4]]> {
            let from = if after { EDGE_HALF * EDGE_FINE } else { 0 };
            let thin = EDGE_FINE / EDGE_RES;
            (0..=EDGE_HALF * EDGE_RES)
                .map(|j| {
                    let i = from + j * thin;
                    let u = x(i).max(0.0);
                    let mut r = [0.0f32; 4];
                    for m in 0..4 {
                        let naive = if after {
                            u.powi(m as i32) / fact[m]
                        } else {
                            0.0
                        };
                        r[m] = (b[m][i] - naive) as f32;
                    }
                    r
                })
                .collect()
        };
        Edges {
            before: side(false),
            after: side(true),
        }
    }

    /// The four residuals `tau` samples from a restart (under
    /// [`EDGE_HALF`] either way).
    #[inline(always)]
    fn at(&self, tau: f32) -> [f32; 4] {
        let (tab, x) = if tau < 0.0 {
            (&self.before, (tau + EDGE_HALF as f32) * EDGE_RES as f32)
        } else {
            (&self.after, tau * EDGE_RES as f32)
        };
        let i = (x.max(0.0) as usize).min(tab.len() - 2);
        let fr = x - i as f32;
        let (a, b) = (tab[i], tab[i + 1]);
        [
            a[0] + (b[0] - a[0]) * fr,
            a[1] + (b[1] - a[1]) * fr,
            a[2] + (b[2] - a[2]) * fr,
            a[3] + (b[3] - a[3]) * fr,
        ]
    }
}

/// The restart residuals, built on the first call (allocates; Poly and
/// the Wavetable synth call it when made, off the audio thread).
pub(crate) fn edges() -> &'static Edges {
    static E: OnceLock<Edges> = OnceLock::new();
    E.get_or_init(Edges::build)
}

/// A sync restart over one control step (DSP quality 1.1). Where the
/// running phase wraps, the read jumps from the ratio's fraction of a
/// cycle back to 0: a jump in its level and in each derivative. This
/// holds those jumps (level and first three derivatives, a cycle at a
/// time), read from the playing level, on the step's [`Run`] crossfaded
/// as it is where the step starts and where it ends, at the ratios there,
/// and [`Restart::at`] adds their band-limited residuals ([`Edges`]) to every
/// sample within [`EDGE_HALF`] of a restart, at the restart's fraction of
/// a sample. The restart comes at a known phase, so the samples before it
/// are corrected as well as those after, with the jumps it will have.
/// What the step before worked out is used again where it still holds.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Restart {
    /// The read's level and derivatives at phase 0, on each frame.
    zero: [[f32; 4]; 2],
    /// The jumps on each frame at ratio `from`, while it holds and the
    /// crossfade moves (`paired`).
    pair: [[f32; 4]; 2],
    /// The jumps where the step ends (the next starts there, if its ratio
    /// and crossfade do).
    last: [f32; 4],
    /// The jumps where the step starts (its first ratio, its run's first
    /// crossfade), and how much they change by where it ends.
    start: [f32; 4],
    change: [f32; 4],
    /// The first ratio and the last, and 1 over the change (0 if none).
    from: f32,
    to: f32,
    per: f32,
    /// The run's crossfade where the step starts and where it ends, and 1
    /// over the change (0 if none, or if the ratio moves: either tells how
    /// far through the step a restart falls).
    t0: f32,
    t1: f32,
    tper: f32,
    /// The frames: the level's data and their offsets in it.
    key: (usize, usize, usize),
    /// Whether there is anything to correct, and whether `pair` holds.
    on: bool,
    paired: bool,
}

impl Restart {
    /// Moves this restart on to a control step with `warp` on `run`, a run
    /// on `level` of `tab`, using what the step before worked out where it
    /// still holds, unless `fresh` (a note starts, or the table changed).
    /// Off unless the warp is sync and its ratio ends mid-cycle somewhere
    /// in the step. Works in place: a voice keeps one per oscillator.
    #[inline]
    pub(crate) fn update(&mut self, warp: Warp, run: &Run, tab: &Table, level: usize, fresh: bool) {
        match warp {
            Warp::Sync { ratio, glide } if glide != 0.0 || cycle(ratio) != 0.0 => {
                self.refresh(warp, run, tab.levels()[level].data(), fresh)
            }
            _ => self.on = false,
        }
    }

    #[inline(never)]
    fn refresh(&mut self, warp: Warp, run: &Run, d: &[f32], fresh: bool) {
        let Warp::Sync { ratio, glide } = warp else {
            return;
        };
        let key = (d.as_ptr() as usize, run.off0, run.off1);
        let same = !fresh && self.on && self.key == key;
        // Whether the ratio moves this step, and whether the jumps worked
        // out at the ratio they were (the step before's) still hold: a
        // ratio within RATIO_HOLD of it changes them by far less than the
        // correction's own error (a smoothed knob creeps that little for
        // a long time).
        let to = ratio + glide * STEP;
        let moving = (to - ratio).abs() > RATIO_HOLD;
        let held = same && self.per == 0.0 && !moving && (ratio - self.from).abs() <= RATIO_HOLD;
        // Nothing moved: the same frames crossfaded the same.
        if held && run.t_inc == 0.0 && self.tper == 0.0 && self.t0 == run.t {
            return;
        }
        let len = 1usize << (32 - run.shift);
        let frames = [
            &d[run.off0..run.off0 + len + 1],
            &d[run.off1..run.off1 + len + 1],
        ];
        if !same {
            self.zero = derivs(frames, len, 0.0);
        }
        if !held {
            self.from = ratio;
            self.paired = false;
        }
        let zero = self.zero;
        // The jumps at ratio `r` on the frames crossfaded by `t`.
        let jumps = |r: f32, t: f32| -> [f32; 4] {
            let e = derivs_at(frames, len, cycle(r), t);
            std::array::from_fn(|m| zero[0][m] + (zero[1][m] - zero[0][m]) * t - e[m])
        };
        // Where the step starts and ends. Where it starts is where the
        // step before ended, if that was at this ratio and crossfade. In
        // between, a straight line: exact at both ends (the crossfade and
        // the ratio both move a frame at a time), and through the ratio
        // when it moves, else through the crossfade.
        let (t0, t1) = (run.t, run.t + run.t_inc * STEP);
        let follows =
            same && (self.to - ratio).abs() <= RATIO_HOLD && (self.t1 - t0).abs() <= RATIO_HOLD;
        let (a, b);
        if moving || run.t_inc == 0.0 {
            a = if follows { self.last } else { jumps(ratio, t0) };
            b = if moving { jumps(to, t1) } else { a };
            self.tper = 0.0;
        } else {
            // A held ratio while the crossfade moves: each frame's jumps,
            // worked out once while it holds.
            if !self.paired {
                let e = derivs(frames, len, cycle(ratio));
                self.pair = std::array::from_fn(|i| std::array::from_fn(|m| zero[i][m] - e[i][m]));
                self.paired = true;
            }
            let j = self.pair;
            let at =
                |t: f32| -> [f32; 4] { std::array::from_fn(|m| j[0][m] + (j[1][m] - j[0][m]) * t) };
            (a, b) = (at(t0), at(t1));
            self.tper = 1.0 / (run.t_inc * STEP);
        }
        if moving {
            self.to = to;
            self.per = 1.0 / (to - ratio);
        } else {
            self.to = self.from;
            self.per = 0.0;
        }
        self.start = a;
        self.change = std::array::from_fn(|m| b[m] - a[m]);
        self.last = b;
        (self.t0, self.t1) = (t0, t1);
        self.key = key;
        self.on = true;
    }

    /// What the restarts near running phase `p` (0 to 1, stepping `dt` a
    /// sample) add to the read, the frames crossfaded by `t` (moving
    /// `t_inc` a sample) and the warp's ratio at `ratio` (moving `glide`).
    #[inline(always)]
    pub(crate) fn at(&self, p: f32, dt: f32, t: f32, t_inc: f32, ratio: f32, glide: f32) -> f32 {
        // Within the span of a restart either side (always, when a master
        // cycle is shorter than the correction), and on.
        if (p - 0.5).abs() <= 0.5 - EDGE_HALF as f32 * dt || !self.on {
            return 0.0;
        }
        self.near(p, dt, t, t_inc, ratio, glide)
    }

    /// [`Restart::at`] for a sample already within its span check.
    #[inline(always)]
    pub(crate) fn near(&self, p: f32, dt: f32, t: f32, t_inc: f32, ratio: f32, glide: f32) -> f32 {
        if dt <= 0.0 {
            return 0.0;
        }
        let span = EDGE_HALF as f32 * dt;
        let inv = 1.0 / dt;
        if span < 0.5 {
            // One restart in reach: the last (`p` under the span) or the
            // next.
            let tau = if p < 0.5 { p * inv } else { (p - 1.0) * inv };
            return self.one(tau, dt, t, t_inc, ratio, glide);
        }
        // A master cycle shorter than the correction: every restart at a
        // whole phase `k` within the span.
        let mut sum = 0.0;
        let mut k = (p - span).ceil();
        while k <= p + span {
            sum += self.one((p - k) * inv, dt, t, t_inc, ratio, glide);
            k += 1.0;
        }
        sum
    }

    /// One restart's correction `tau` samples after it (before, if
    /// negative): nothing at or past [`EDGE_HALF`] (or at NaN, which the
    /// negated comparison also catches).
    #[inline(always)]
    #[allow(clippy::neg_cmp_op_on_partial_ord)]
    fn one(&self, tau: f32, dt: f32, t: f32, t_inc: f32, ratio: f32, glide: f32) -> f32 {
        if !(tau.abs() < EDGE_HALF as f32) {
            return 0.0;
        }
        // How far through the step the restart falls (by its ratio, or
        // else its crossfade), and each derivative's jump a sample there:
        // the read moves `ratio * dt` cycles a sample.
        let rk = ratio - glide * tau;
        let u = (rk - self.from) * self.per + (t - t_inc * tau - self.t0) * self.tper;
        let s = rk * dt;
        let scale = [1.0, s, s * s, s * s * s];
        let r = edges().at(tau);
        let mut sum = 0.0;
        for m in 0..4 {
            sum += (self.start[m] + self.change[m] * u) * scale[m] * r[m];
        }
        sum
    }
}

/// Where phase `w` (0 to 1) falls on a frame of `len` samples a cycle (a
/// power of two): the sample before it (round the cycle) and how far on.
#[inline(always)]
fn place(w: f32, len: usize) -> (usize, f32) {
    let x = w * len as f32;
    let i = x as i32 as usize;
    (i & (len - 1), x - i as f32)
}

/// The six samples of frame `f` round sample `i`, two before to three
/// after: in one piece unless they wrap round the cycle (each frame holds
/// `len + 1` samples, the first again at the end).
#[inline(always)]
fn six(f: &[f32], i: usize, len: usize) -> [f32; 6] {
    match f.get(i.wrapping_sub(2)..i + 4) {
        Some(&[a, b, c, d, e, g]) => [a, b, c, d, e, g],
        _ => std::array::from_fn(|k| f[(i + len - 2 + k) & (len - 1)]),
    }
}

/// The read's level and first three derivatives (a cycle at a time) from
/// [`six`] samples, `fr` of the way past the third: central differences
/// a table sample (1 / `len` of a cycle) apart.
#[inline(always)]
fn diffs(s: [f32; 6], fr: f32, len: usize) -> [f32; 4] {
    // Powers of two: exact.
    let n = len as f32;
    let (c1, c2, c3) = (0.5 * n, n * n, 0.5 * n * n * n);
    let [m2, m1, z, p1, p2]: [f32; 5] = std::array::from_fn(|k| s[k] + (s[k + 1] - s[k]) * fr);
    let d = p1 - m1;
    [
        z,
        d * c1,
        (p1 + m1 - 2.0 * z) * c2,
        (p2 - m2 - 2.0 * d) * c3,
    ]
}

/// [`diffs`] at phase `w` on each of `frames` (`len` samples a cycle).
fn derivs(frames: [&[f32]; 2], len: usize, w: f32) -> [[f32; 4]; 2] {
    let (i, fr) = place(w, len);
    frames.map(|f| diffs(six(f, i, len), fr, len))
}

/// [`diffs`] at phase `w` on `frames` crossfaded by `t`.
fn derivs_at(frames: [&[f32]; 2], len: usize, w: f32, t: f32) -> [f32; 4] {
    let (i, fr) = place(w, len);
    let (a, b) = (six(frames[0], i, len), six(frames[1], i, len));
    diffs(std::array::from_fn(|k| a[k] + (b[k] - a[k]) * t), fr, len)
}

/// How an oscillator bends its phase this control step.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) enum Warp {
    #[default]
    Off,
    /// Phase distortion: the knee at `k`, slopes `a` before it, `b` after.
    Bend {
        k: f32,
        a: f32,
        b: f32,
    },
    /// The read runs `ratio` cycles a running cycle, restarting with it;
    /// the ratio moves by `glide` a frame across a control step.
    Sync {
        ratio: f32,
        glide: f32,
    },
    Mirror {
        amt: f32,
    },
    Fm {
        index: f32,
    },
}

/// The Warp menu's FM entry ([`crate::wavetable::params::WARP_MODES`]).
pub(crate) const WARP_FM: u8 = 4;

impl Warp {
    pub(crate) fn new(mode: u8, amt: f32) -> Warp {
        let amt = amt.clamp(0.0, 1.0);
        match mode {
            1 => {
                let k = 0.5 - 0.48 * amt;
                Warp::Bend {
                    k,
                    a: 0.5 / k,
                    b: 0.5 / (1.0 - k),
                }
            }
            2 => Warp::Sync {
                ratio: 1.0 + 7.0 * amt,
                glide: 0.0,
            },
            3 => Warp::Mirror { amt },
            WARP_FM => Warp::Fm { index: amt * 0.5 },
            _ => Warp::Off,
        }
    }

    /// This control step's warp `self`, after `last` (the warp the step
    /// before left, as [`Warp::advance`] moved it): a sync ratio glides
    /// from where it stood to its new value across the step rather than
    /// jumping at the step, which would jump the read.
    pub(crate) fn glide_from(self, last: Warp) -> Warp {
        match (self, last) {
            (Warp::Sync { ratio: to, .. }, Warp::Sync { ratio: from, .. }) => Warp::Sync {
                ratio: from,
                glide: (to - from) / STEP,
            },
            _ => self,
        }
    }

    /// The warp `frames` frames on: a sync ratio's glide, a frame at a time
    /// as the reads add it, so the ratio comes out the same whatever the
    /// block size.
    #[inline]
    pub(crate) fn advance(&mut self, frames: usize) {
        if let Warp::Sync { ratio, glide } = self {
            for _ in 0..frames {
                *ratio += *glide;
            }
        }
    }

    /// How many times faster the table is read, for the mip level. Sync
    /// reads its fastest ratio this step, and a third faster again: the
    /// restart's correction ([`Restart`]) holds to 60 dB under the
    /// harmonics only while the read's top harmonic stays under about a
    /// third of the rate (DSP quality 1.1).
    pub(crate) fn speed(self) -> f32 {
        match self {
            Warp::Bend { a, .. } => a,
            Warp::Sync { ratio, glide } => ratio.max(ratio + glide * STEP) * SYNC_MARGIN,
            Warp::Mirror { amt } if amt > 0.0 => 2.0,
            Warp::Fm { index } => 1.0 + 8.0 * index,
            _ => 1.0,
        }
    }

    /// The phase read from, given the running phase and the other
    /// oscillator's sample (for FM): under 1, always. The bend's end and
    /// FM a hair under a whole cycle round up to 1 in floats, and a read
    /// there would pass its frame (and the table, on its last frame), so
    /// those two are held just under it, as the mirror always was.
    #[inline]
    pub(crate) fn apply(self, p: f32, other: f32) -> f32 {
        match self {
            Warp::Off => p,
            Warp::Bend { k, a, b } => {
                if p < k {
                    p * a
                } else {
                    (0.5 + (p - k) * b).min(UNDER_ONE)
                }
            }
            Warp::Sync { ratio, .. } => cycle(p * ratio),
            Warp::Mirror { amt } => {
                let m = 1.0 - (1.0 - 2.0 * p).abs();
                (p + (m - p) * amt).clamp(0.0, UNDER_ONE)
            }
            Warp::Fm { index } => {
                let x = p + index * other;
                (x - x.floor()).min(UNDER_ONE)
            }
        }
    }
}

/// Where an oscillator reads during one control step: two neighbouring
/// frames of its level and the crossfade between them, ramped per frame.
/// A fast sweep that crosses a frame inside a step extrapolates past the
/// pair for the rest of that step (16 frames), which is inaudible.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Run {
    pub(crate) off0: usize,
    pub(crate) off1: usize,
    pub(crate) t: f32,
    pub(crate) t_inc: f32,
    pub(crate) len: f32,
    /// How far a 32-bit phase shifts down to index the level.
    shift: u32,
}

impl Run {
    /// A run on `level` of `tab` from frame position `from` to `to`.
    pub(crate) fn new(tab: &Table, level: usize, from: f32, to: f32) -> Run {
        let lv = &tab.levels()[level];
        let len = lv.len as f32;
        let shift = 32 - lv.len.trailing_zeros();
        let last = tab.frames() - 1;
        if last == 0 {
            return Run {
                len,
                shift,
                ..Run::default()
            };
        }
        let (from, to) = (from.clamp(0.0, last as f32), to.clamp(0.0, last as f32));
        let f0 = (from.min(to) as usize).min(last - 1);
        let st = lv.stride();
        Run {
            off0: f0 * st,
            off1: (f0 + 1) * st,
            t: from - f0 as f32,
            t_inc: (to - from) / STEP,
            len,
            shift,
        }
    }

    /// One sample at phase `ph` (0 to 1): linear in phase, then between
    /// the two frames.
    #[inline]
    pub(crate) fn read(&self, d: &[f32], ph: f32) -> f32 {
        let x = ph * self.len;
        let i = x as usize;
        let fr = x - i as f32;
        let (a0, a1) = (d[self.off0 + i], d[self.off0 + i + 1]);
        let (b0, b1) = (d[self.off1 + i], d[self.off1 + i + 1]);
        let s0 = a0 + (a1 - a0) * fr;
        let s1 = b0 + (b1 - b0) * fr;
        s0 + (s1 - s0) * self.t
    }
}

/// The two frames' samples at index `i` and the next, between them by
/// `fr`, then between the frames by `t`.
#[inline(always)]
fn pair(f0: &[f32], f1: &[f32], i: usize, fr: f32, t: f32) -> f32 {
    let (a0, a1) = (f0[i], f0[i + 1]);
    let (b0, b1) = (f1[i], f1[i + 1]);
    let s0 = a0 + (a1 - a0) * fr;
    let s1 = b0 + (b1 - b0) * fr;
    s0 + (s1 - s0) * t
}

/// The read of a bent phase `w` (0 to 1): the index never passes the
/// level's last sample, so a warp that lands on 1 reads the cycle's end.
#[inline(always)]
fn at(f0: &[f32], f1: &[f32], len: f32, w: f32, t: f32) -> f32 {
    let x = w * len;
    let i = (x as usize).min(f0.len() - 2);
    pair(f0, f1, i, x - i as f32, t)
}

/// `buf[j] = 0 + y * level` (`ADD` false) or `buf[j] += y * level`.
#[inline(always)]
fn put<const ADD: bool>(x: &mut f32, y: f32, level: f32) {
    *x = if ADD { *x } else { 0.0 } + y * level;
}

/// One oscillator of one unison stack over `buf`, written (`ADD` false)
/// or added, times `level`: level data `d` through `run` from its
/// crossfade `t`, from phase `ph` by `dt` a frame (32-bit phases), each
/// phase bent by `warp` (a sync ratio gliding as it goes, with `restart`'s
/// correction; the caller [`Warp::advance`]s it after). `fm` is the other oscillator's samples (any
/// length when the warp isn't FM). Advances the phase and returns the
/// crossfade after the last frame, which is the same for every stack.
#[allow(clippy::too_many_arguments)]
#[inline(never)]
pub(crate) fn render<const ADD: bool>(
    run: &Run,
    restart: &Restart,
    t: f32,
    d: &[f32],
    warp: Warp,
    fm: &[f32],
    buf: &mut [f32],
    ph: &mut u32,
    dt: u32,
    level: f32,
) -> f32 {
    let n = run.len as usize + 1;
    let (f0, f1) = (&d[run.off0..run.off0 + n], &d[run.off1..run.off1 + n]);
    let (len, inc) = (run.len, run.t_inc);
    let (mut p, mut t) = (*ph, t);
    let unit = 1.0 / ONE;
    match warp {
        Warp::Off => {
            let mask = (1u32 << run.shift) - 1;
            let scale = 1.0 / (1u64 << run.shift) as f32;
            for x in buf.iter_mut() {
                let i = (p >> run.shift) as usize;
                let fr = (p & mask) as f32 * scale;
                put::<ADD>(x, pair(f0, f1, i, fr, t), level);
                p = p.wrapping_add(dt);
                t += inc;
            }
        }
        Warp::Fm { index } => {
            for (x, &o) in buf.iter_mut().zip(fm) {
                let w = p as f32 * unit + index * o;
                put::<ADD>(x, at(f0, f1, len, w - w.floor(), t), level);
                p = p.wrapping_add(dt);
                t += inc;
            }
        }
        Warp::Sync { mut ratio, glide } => {
            // The running phase from its top 24 bits, which a float holds
            // exactly: never a whole cycle, so a sample at a restart reads
            // and corrects as after it.
            let top = 1.0 / (1u32 << 24) as f32;
            let step = dt as f32 * unit;
            let mask = (1u32 << run.shift) - 1;
            let scale = 1.0 / (1u64 << run.shift) as f32;
            // The phases near enough a restart to correct, `p + reach`
            // under `wide`: a little wider than the correction's own reach
            // (which ends itself), never when it's off, and always when a
            // master cycle is shorter than two reaches.
            let reach = EDGE_HALF as u64 * u64::from(dt) + 4096;
            let (reach, wide) = match (restart.on, reach) {
                (false, _) => (0, 0),
                (true, r) if r >= 1 << 31 => (0, 1u64 << 32),
                (true, r) => (r as u32, 2 * r),
            };
            for x in buf.iter_mut() {
                let pf = (p >> 8) as f32 * top;
                // The read's phase as a 32-bit phase (the cycle's fraction
                // kept), indexed as an unbent one is.
                let q = (pf * ratio * ONE) as i64 as u32;
                let i = (q >> run.shift) as usize;
                let mut y = pair(f0, f1, i, (q & mask) as f32 * scale, t);
                if u64::from(p.wrapping_add(reach)) < wide {
                    y += restart.near(pf, step, t, inc, ratio, glide);
                }
                put::<ADD>(x, y, level);
                p = p.wrapping_add(dt);
                t += inc;
                ratio += glide;
            }
        }
        w => {
            for x in buf.iter_mut() {
                let q = w.apply(p as f32 * unit, 0.0);
                put::<ADD>(x, at(f0, f1, len, q, t), level);
                p = p.wrapping_add(dt);
                t += inc;
            }
        }
    }
    *ph = p;
    t
}

/// Both oscillators of one stack on FM, each moving the other's phase,
/// into `buf` as `a * levels[0] + b * levels[1]`: the second reads with
/// the first's sample from the frame before (`last`, kept between calls),
/// the first with the second's from this frame, as on the Wavetable synth.
/// Returns each run's crossfade after the last frame.
#[allow(clippy::too_many_arguments)]
#[inline(never)]
pub(crate) fn render_fm_pair(
    runs: [&Run; 2],
    ts: [f32; 2],
    ds: [&[f32]; 2],
    index: [f32; 2],
    buf: &mut [f32],
    ph: [&mut u32; 2],
    dt: [u32; 2],
    levels: [f32; 2],
    last: &mut f32,
) -> [f32; 2] {
    fn frames<'a>(r: &Run, d: &'a [f32]) -> (&'a [f32], &'a [f32]) {
        let n = r.len as usize + 1;
        (&d[r.off0..r.off0 + n], &d[r.off1..r.off1 + n])
    }
    let (a0, a1) = frames(runs[0], ds[0]);
    let (b0, b1) = frames(runs[1], ds[1]);
    let (la, lb) = (runs[0].len, runs[1].len);
    let unit = 1.0 / ONE;
    let [pa, pb] = ph;
    let (mut p, mut q) = (*pa, *pb);
    let [mut ta, mut tb] = ts;
    let mut a = *last;
    for x in buf.iter_mut() {
        let wb = q as f32 * unit + index[1] * a;
        let b = at(b0, b1, lb, wb - wb.floor(), tb);
        let wa = p as f32 * unit + index[0] * b;
        a = at(a0, a1, la, wa - wa.floor(), ta);
        *x = 0.0 + a * levels[0] + b * levels[1];
        p = p.wrapping_add(dt[0]);
        q = q.wrapping_add(dt[1]);
        ta += runs[0].t_inc;
        tb += runs[1].t_inc;
    }
    *pa = p;
    *pb = q;
    *last = a;
    [ta, tb]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wavetable::factory;

    #[test]
    fn warps_stay_in_the_cycle() {
        for mode in 0..5u8 {
            for amt in [0.0, 0.3, 1.0] {
                let w = Warp::new(mode, amt);
                for k in 0..1000 {
                    let p = k as f32 / 1000.0;
                    let q = w.apply(p, (k as f32 * 0.37).sin());
                    assert!((0.0..1.0).contains(&q), "mode {mode} amt {amt} p {p}: {q}");
                }
            }
        }
        assert_eq!(Warp::new(1, 0.0).apply(0.3, 0.0), 0.3);
        // Where floats round up to a whole cycle: the bend's end, and FM
        // pulling a phase of 0 a hair below it.
        let end = 1.0 - f32::EPSILON / 2.0;
        for amt in [0.0, 0.5, 1.0] {
            assert!(Warp::new(1, amt).apply(end, 0.0) < 1.0, "bend {amt}");
        }
        assert!(Warp::new(4, 1.0).apply(0.0, -1e-9) < 1.0);
    }

    /// A step of the block read gives what the frame-at-a-time read does,
    /// on every level and at the table's last pair of frames, bent or not
    /// (sync, which adds its restart's correction, below).
    #[test]
    fn the_block_read_matches_the_frame_read() {
        let tab = &factory()[3];
        let last = (tab.frames() - 1) as f32;
        for level in [0, 5, 16] {
            let d = tab.levels()[level].data();
            for mode in [0u8, 1, 3] {
                let w = Warp::new(mode, 0.4);
                let run = Run::new(tab, level, last - 0.7, last);
                let mut buf = [0.0f32; 16];
                let (mut p, dt) = (0x1234_5678u32, 0x0321_0000u32);
                let none = Restart::default();
                let t = render::<false>(&run, &none, run.t, d, w, &[], &mut buf, &mut p, dt, 1.0);
                let mut r = run;
                let mut q = 0x1234_5678u32;
                for (j, &y) in buf.iter().enumerate() {
                    let ph = w.apply(q as f32 / ONE, 0.0);
                    let want = r.read(d, ph);
                    assert!(
                        (y - want).abs() < 1e-5,
                        "level {level} mode {mode} frame {j}"
                    );
                    q = q.wrapping_add(dt);
                    r.t += r.t_inc;
                }
                assert_eq!(p, q);
                assert!((t - r.t).abs() < 1e-6);
            }
        }
    }

    /// With sync, the block read (Poly's) gives the frame read plus the
    /// restart's correction (the Wavetable synth's), on both sides of a
    /// restart and across it, with the ratio held or gliding.
    #[test]
    fn the_sync_block_read_matches_the_frame_read() {
        let tab = &factory()[3];
        let last = (tab.frames() - 1) as f32;
        let top = 1.0 / (1u32 << 24) as f32;
        for level in [0, 5, 12] {
            let d = tab.levels()[level].data();
            for (amt, from) in [(0.2, 0.2), (0.55, 0.3), (0.9, 0.95)] {
                let w = Warp::new(2, amt).glide_from(Warp::new(2, from));
                for start in [0x1234_5678u32, 0xF800_0000, 0xFFFF_FF00] {
                    let run = Run::new(tab, level, last - 0.7, last);
                    let mut rs = Restart::default();
                    rs.update(w, &run, tab, level, true);
                    let mut buf = [0.0f32; 16];
                    let (mut p, dt) = (start, 0x0321_0000u32);
                    render::<false>(&run, &rs, run.t, d, w, &[], &mut buf, &mut p, dt, 1.0);
                    let (mut r, mut wf) = (run, w);
                    let mut q = start;
                    let mut corrected = 0;
                    for (j, &y) in buf.iter().enumerate() {
                        let ph = (q >> 8) as f32 * top;
                        let Warp::Sync { ratio, glide } = wf else {
                            unreachable!()
                        };
                        let fix = rs.at(ph, dt as f32 / ONE, r.t, r.t_inc, ratio, glide);
                        corrected += usize::from(fix != 0.0);
                        let want = r.read(d, wf.apply(ph, 0.0)) + fix;
                        assert!(
                            (y - want).abs() < 1e-5,
                            "level {level} amt {amt} start {start:#x} frame {j}"
                        );
                        q = q.wrapping_add(dt);
                        r.t += r.t_inc;
                        wf.advance(1);
                    }
                    assert!(corrected > 0, "level {level} amt {amt} start {start:#x}");
                }
            }
        }
    }

    /// The restart's residuals: the level's drops by the whole jump across
    /// the restart, the derivatives' meet there, and all four have died
    /// away at both ends of the window, so nothing is left past it.
    #[test]
    fn the_restart_residuals_meet_and_die_away() {
        let e = edges();
        let (b, a) = (e.before[e.before.len() - 1], e.after[0]);
        assert!((b[0] - a[0] - 1.0).abs() < 1e-4, "{b:?} {a:?}");
        for m in 1..4 {
            assert!((b[m] - a[m]).abs() < 1e-4, "{m}: {b:?} {a:?}");
        }
        let (first, end) = (e.before[0], e.after[e.after.len() - 1]);
        for m in 0..4 {
            assert!(
                first[m].abs() < 1e-4 && end[m].abs() < 1e-4,
                "{m}: {first:?} {end:?}"
            );
        }
    }

    /// Basic's last frame (a square) at `hz` (44.1 kHz) with the sync warp,
    /// a control step at a time, `frames` long. `step(k)` gives control
    /// step `k`'s table position (0 to 1, the run sweeping to it from the
    /// last step's) and sync amount. `fine` 1 is the block read as the
    /// engines play it, the ratio gliding across each step. Above 1 it is
    /// the same moves `fine` times faster on the same levels, read frame by
    /// frame with no correction: brought down, the reference. `fine` 0 is
    /// the read as it was before DSP quality 1.1: no correction, the ratio
    /// jumping at each step. `hold` keeps the level the one for that
    /// speed, rather than the one each step's warp picks.
    fn sync_square(
        hz: f64,
        frames: usize,
        fine: usize,
        step: impl Fn(usize) -> (f32, f32),
        hold: Option<f32>,
    ) -> Vec<f32> {
        let tab = &factory()[0];
        let last = (tab.frames() - 1) as f32;
        let dt = hz / 44_100.0;
        let mut out = Vec::with_capacity(frames * fine.max(1));
        let (mut p, mut pf) = (0u32, 0.0f64);
        let mut from = step(0).0 * last;
        let mut warp = Warp::new(2, step(0).1);
        let mut restart = Restart::default();
        for k in 0..frames / 16 {
            let (pos, amt) = step(k);
            let to = pos * last;
            let w = if fine == 0 {
                Warp::new(2, amt)
            } else {
                Warp::new(2, amt).glide_from(warp)
            };
            let level = tab.level_for(dt as f32 * hold.unwrap_or(w.speed()));
            let run = Run::new(tab, level, from, to);
            from = to;
            let d = tab.levels()[level].data();
            restart.update(w, &run, tab, level, false);
            if fine == 1 {
                let mut buf = [0.0f32; 16];
                let di = (dt * ONE as f64) as u32;
                render::<false>(&run, &restart, run.t, d, w, &[], &mut buf, &mut p, di, 1.0);
                out.extend_from_slice(&buf);
            } else {
                let n = fine.max(1);
                let (mut r, mut wf) = (run, w);
                r.t_inc /= n as f32;
                if let Warp::Sync { glide, .. } = &mut wf {
                    *glide /= n as f32;
                }
                for _ in 0..16 * n {
                    out.push(r.read(d, wf.apply(pf as f32, 0.0)));
                    pf = (pf + dt / n as f64).fract();
                    r.t += r.t_inc;
                    wf.advance(1);
                }
            }
            warp = w;
            warp.advance(16);
        }
        out
    }

    /// `x`'s power spectrum through a Blackman-Harris window, bin `k` at
    /// `k / x.len()` cycles a sample.
    fn spectrum(x: &[f32]) -> Vec<f64> {
        use std::f64::consts::TAU;
        let n = x.len();
        let mut re: Vec<f64> = x
            .iter()
            .enumerate()
            .map(|(i, &v)| {
                let w = TAU * i as f64 / n as f64;
                let win = 0.35875 - 0.48829 * w.cos() + 0.14128 * (2.0 * w).cos()
                    - 0.01168 * (3.0 * w).cos();
                f64::from(v) * win
            })
            .collect();
        let mut im = vec![0.0; n];
        crate::wavetable::fft::fft(&mut re, &mut im, false);
        (0..n / 2).map(|k| re[k] * re[k] + im[k] * im[k]).collect()
    }

    /// Restarts at every fraction of a sample (notes whose cycles aren't a
    /// whole number of samples) fold back at least 60 dB under the loudest
    /// harmonic, 20 Hz to 20 kHz, at short and long sync ratios.
    #[test]
    fn sync_restarts_between_samples_fold_back_little() {
        let n = 16_384;
        for hz in [523.25, 739.99, 1046.5] {
            for amt in [0.2, 0.5, 0.93] {
                let x = sync_square(hz, n + 1024, 1, |_| (1.0, amt), None);
                let ps = spectrum(&x[1024..]);
                let f0 = hz / 44_100.0;
                let (mut harm, mut other) = (0.0f64, 0.0f64);
                for (k, &p) in ps.iter().enumerate() {
                    let f = k as f64 / n as f64;
                    if !(20.0 / 44_100.0..=20_000.0 / 44_100.0).contains(&f) {
                        continue;
                    }
                    let h = (f / f0).round().max(1.0);
                    if ((f - h * f0) * n as f64).abs() <= 8.0 {
                        harm = harm.max(p);
                    } else {
                        other = other.max(p);
                    }
                }
                let db = 10.0 * (other / harm).log10();
                assert!(db < -60.0, "{hz} Hz, sync {amt}: {db:.1} dB");
            }
        }
    }

    /// A table position sweeping the whole table, a sync amount moved by a
    /// fast LFO, and both, a control step at a time, stay close to the
    /// same moves rendered 32 times faster and brought down. 20 Hz to 16
    /// kHz, the difference is at least 20 dB under the one the read had
    /// before DSP quality 1.1 and 45 dB under the signal; on one held
    /// level, 55 dB under it. (What the held level takes away is the jump
    /// where a moving ratio changes the table level, which 1.1 leaves.)
    #[test]
    fn a_moving_table_and_ratio_track_the_oversampled_read() {
        use std::f64::consts::{PI, TAU};
        let (n, fine) = (8_192usize, 32usize);
        // Down by `fine` through a Blackman-Harris windowed sinc, 64
        // samples each side at 44.1 kHz, cut at 0.45 of its rate.
        let half = 64 * fine;
        let kernel: Vec<f64> = (0..=2 * half)
            .map(|i| {
                let u = (i as f64 - half as f64) / fine as f64;
                let sinc = if i == half {
                    0.9
                } else {
                    (TAU * 0.45 * u).sin() / (PI * u)
                };
                let w = PI * i as f64 / half as f64;
                let win = 0.35875 - 0.48829 * w.cos() + 0.14128 * (2.0 * w).cos()
                    - 0.01168 * (3.0 * w).cos();
                sinc * win / fine as f64
            })
            .collect();
        let down = |hi: &[f32]| -> Vec<f32> {
            (0..n)
                .map(|j| {
                    let mut acc = 0.0;
                    for (i, &k) in kernel.iter().enumerate() {
                        let at = (j * fine + i) as isize - half as isize;
                        if at >= 0 && (at as usize) < hi.len() {
                            acc += k * f64::from(hi[at as usize]);
                        }
                    }
                    acc as f32
                })
                .collect()
        };
        // The difference from the reference, 20 Hz to 16 kHz, against the
        // reference there (dB), away from both ends.
        let band = |x: &[f32], reference: &[f32]| -> f64 {
            let m = 4_096;
            let from = (n - m) / 2;
            let diff: Vec<f32> = (from..from + m).map(|j| x[j] - reference[j]).collect();
            let (e, s) = (spectrum(&diff), spectrum(&reference[from..from + m]));
            let keep = |k: &usize| (20.0..=16_000.0).contains(&(*k as f64 / m as f64 * 44_100.0));
            let e: f64 = (0..m / 2).filter(keep).map(|k| e[k]).sum();
            let s: f64 = (0..m / 2).filter(keep).map(|k| s[k]).sum();
            10.0 * (e / s).log10()
        };
        for case in ["position", "ratio", "both"] {
            let step = move |k: usize| {
                let sec = k as f32 * 16.0 / 44_100.0;
                let pos = if case == "ratio" {
                    1.0
                } else {
                    (k as f32 * 16.0 / n as f32).min(1.0)
                };
                let lfo = (std::f32::consts::TAU * 9.0 * sec).sin();
                (
                    pos,
                    if case == "position" {
                        0.5
                    } else {
                        0.5 + 0.35 * lfo
                    },
                )
            };
            for hz in [523.25, 1046.5] {
                let hi = sync_square(hz, n, fine, step, None);
                let reference = down(&hi);
                let ours = band(&sync_square(hz, n, 1, step, None), &reference);
                let before = band(&sync_square(hz, n, 0, step, None), &reference);
                assert!(ours < -45.0, "{case}, {hz} Hz: {ours:.1} dB");
                assert!(
                    ours < before - 20.0,
                    "{case}, {hz} Hz: {ours:.1} against {before:.1} dB before"
                );
                // On the level for the move's fastest ratio (sync 85).
                let hold = Some(Warp::new(2, 0.85).speed());
                let hi = sync_square(hz, n, fine, step, hold);
                let reference = down(&hi);
                let held = band(&sync_square(hz, n, 1, step, hold), &reference);
                assert!(held < -55.0, "{case}, {hz} Hz, one level: {held:.1} dB");
            }
        }
    }

    /// FM that lands a hair under a whole cycle (which rounds to 1) still
    /// reads inside the last frame.
    #[test]
    fn fm_reads_inside_the_last_frame() {
        let tab = &factory()[0];
        let last = (tab.frames() - 1) as f32;
        let d = tab.levels()[0].data();
        let run = Run::new(tab, 0, last, last);
        let fm = [-1e-9f32; 16];
        let mut buf = [0.0f32; 16];
        let mut p = 0u32;
        render::<false>(
            &run,
            &Restart::default(),
            run.t,
            d,
            Warp::new(WARP_FM, 1.0),
            &fm,
            &mut buf,
            &mut p,
            0,
            1.0,
        );
        assert!(buf.iter().all(|x| x.is_finite()));
    }
}
