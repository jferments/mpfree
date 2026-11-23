//! A slow, exact reference for analog-style filters (the filter gap plan's
//! step F1, for its analog-style models in F5; `docs/dev/design/
//! fx-filter.md`, "The pad filter"). A model is written as the circuit's
//! differential equations, `dy/dt = f(y, x)`, and solved by the
//! trapezoidal rule with Newton's method run to convergence at every step,
//! at 16 or more times the rate: no unit delay in a loop, no curve taken
//! after a linear solve, no curve run at the base rate. The input is
//! brought up and the output down by the long linear-phase filter of the
//! DSP quality plan's Y8 references ([`crate::reference`], 120 dB down
//! from 24.1 kHz), so what comes out is what the circuit would give,
//! band-limited to what the base rate carries.
//!
//! What a real-time model does in its budget (one solve a sample, its
//! curves at twice the rate) is measured against this. The reference is
//! trusted only where it has converged: [`converged_db`] renders a case at
//! 16 and at 32 times and compares the two. The trapezoidal rule bends
//! frequencies by `tan(πf/fs) / (πf/fs)`, which at 16 times 44.1 kHz is
//! 1.1 cents at 10 kHz and 0.01 cents at 1 kHz.
//!
//! Models: the linear state-variable filter (to check the solver against
//! its exact response) and the four-stage transistor ladder, its curve in
//! every stage (Huovilainen's model, DAFx 2004: the circuit's behaviour)
//! or on the feedback alone (the pad's Ladder today). They differ most
//! where a ladder sings: on its own after a click at 1 kHz, the ladder
//! with a curve in every stage sings 20 cents flat at `k` 4.2, 70 at 5 and
//! 100 at 6, as the circuit falls flat the louder it sings, while the one
//! with its curve on the feedback stays within 2 cents. The diode ladder,
//! the Sallen-Key with its diode clipper and the state-variable filter
//! with saturating integrators come with the F5 steps that build them.

use std::f64::consts::PI;

use crate::fft::convolve;
use crate::reference::decimation_taps;

/// A filter written as differential equations in time (seconds).
pub trait Model {
    /// How many states it has.
    fn states(&self) -> usize;
    /// `dy/dt` at state `y` and input `x`.
    fn slope(&self, y: &[f64], x: f64, dy: &mut [f64]);
    /// What it puts out at state `y` and input `x`.
    fn output(&self, y: &[f64], x: f64) -> f64;
}

/// A saturating curve, slope 1 through 0.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Curve {
    /// None: the linear filter.
    Linear,
    /// `tanh`: a transistor pair's.
    Tanh,
    /// The pad Ladder's `x (27 + x²) / (27 + 9x²)` up to ±3, then flat at
    /// ±1 (`mpfree_sampler::dsp::soft_clip`).
    PadSoftClip,
}

impl Curve {
    pub fn at(self, v: f64) -> f64 {
        match self {
            Curve::Linear => v,
            Curve::Tanh => v.tanh(),
            Curve::PadSoftClip => {
                let v = v.clamp(-3.0, 3.0);
                let v2 = v * v;
                v * (27.0 + v2) / (27.0 + 9.0 * v2)
            }
        }
    }
}

/// What a state-variable filter puts out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SvfOut {
    LowPass,
    BandPass,
    HighPass,
}

/// The linear state-variable filter: `hp = x - lp - bp / Q`, `bp' = ωc
/// hp`, `lp' = ωc bp`.
#[derive(Clone, Copy, Debug)]
pub struct Svf {
    pub cutoff_hz: f64,
    pub q: f64,
    pub out: SvfOut,
}

impl Model for Svf {
    fn states(&self) -> usize {
        2
    }

    fn slope(&self, y: &[f64], x: f64, dy: &mut [f64]) {
        let w = 2.0 * PI * self.cutoff_hz;
        let hp = x - y[1] - y[0] / self.q;
        dy[0] = w * hp;
        dy[1] = w * y[0];
    }

    fn output(&self, y: &[f64], x: f64) -> f64 {
        match self.out {
            SvfOut::LowPass => y[1],
            SvfOut::BandPass => y[0],
            SvfOut::HighPass => x - y[1] - y[0] / self.q,
        }
    }
}

/// The four-stage transistor ladder: each stage a one-pole at the cutoff,
/// the last fed back `k` times (it sings from `k = 4`). With its curve in
/// every stage (`stages`): `y1' = ωc (c(x - k y4) - c(y1))`, `yi' = ωc
/// (c(y(i-1)) - c(yi))`; with it on the feedback alone: `y1' = ωc (x -
/// c(k y4) - y1)`, the rest linear. The output is `makeup × y4`.
#[derive(Clone, Copy, Debug)]
pub struct Ladder {
    pub cutoff_hz: f64,
    pub k: f64,
    pub curve: Curve,
    pub stages: bool,
    pub makeup: f64,
}

impl Model for Ladder {
    fn states(&self) -> usize {
        4
    }

    fn slope(&self, y: &[f64], x: f64, dy: &mut [f64]) {
        let w = 2.0 * PI * self.cutoff_hz;
        let c = |v: f64| self.curve.at(v);
        if self.stages {
            let cy: [f64; 4] = [c(y[0]), c(y[1]), c(y[2]), c(y[3])];
            dy[0] = w * (c(x - self.k * y[3]) - cy[0]);
            for i in 1..4 {
                dy[i] = w * (cy[i - 1] - cy[i]);
            }
        } else {
            dy[0] = w * (x - c(self.k * y[3]) - y[0]);
            for i in 1..4 {
                dy[i] = w * (y[i - 1] - y[i]);
            }
        }
    }

    fn output(&self, y: &[f64], _x: f64) -> f64 {
        self.makeup * y[3]
    }
}

/// How closely Newton's method solves each step: the largest change of a
/// state in the last iteration.
const TOLERANCE: f64 = 1e-13;
const MAX_ITERATIONS: usize = 40;

/// The most states a model may have.
const MAX_STATES: usize = 8;

/// One trapezoidal step of `h` seconds from `y` (input `x0` at its start,
/// `x1` at its end), solved by Newton's method with a numerical Jacobian;
/// a step that doesn't converge is split in two (up to 16 parts).
fn step(m: &dyn Model, y: &mut [f64], x0: f64, x1: f64, h: f64, depth: u32) {
    let n = y.len();
    assert!(
        n <= MAX_STATES,
        "a reference model has at most {MAX_STATES} states"
    );
    let mut f0 = [0.0; MAX_STATES];
    m.slope(y, x0, &mut f0[..n]);
    // Explicit Euler for the first guess.
    let mut z = [0.0; MAX_STATES];
    for i in 0..n {
        z[i] = y[i] + h * f0[i];
    }
    let (mut f1, mut fe) = ([0.0; MAX_STATES], [0.0; MAX_STATES]);
    let mut jac = [0.0; MAX_STATES * MAX_STATES];
    let mut g = [0.0; MAX_STATES];
    for _ in 0..MAX_ITERATIONS {
        m.slope(&z[..n], x1, &mut f1[..n]);
        for i in 0..n {
            g[i] = z[i] - y[i] - 0.5 * h * (f0[i] + f1[i]);
        }
        // J = I - h/2 df/dz, by forward differences.
        for j in 0..n {
            let e = 1e-7 * (1.0 + z[j].abs());
            let keep = z[j];
            z[j] += e;
            m.slope(&z[..n], x1, &mut fe[..n]);
            z[j] = keep;
            for i in 0..n {
                let d = (fe[i] - f1[i]) / e;
                jac[i * n + j] = if i == j { 1.0 } else { 0.0 } - 0.5 * h * d;
            }
        }
        if !solve(&mut jac[..n * n], &mut g[..n], n) {
            break;
        }
        let mut worst = 0.0f64;
        let mut size = 0.0f64;
        for i in 0..n {
            z[i] -= g[i];
            worst = worst.max(g[i].abs());
            size = size.max(z[i].abs());
        }
        if !z[..n].iter().all(|v| v.is_finite()) {
            break;
        }
        if worst < TOLERANCE * (1.0 + size) {
            y.copy_from_slice(&z[..n]);
            return;
        }
    }
    assert!(depth < 4, "the reference's Newton steps did not converge");
    let xm = 0.5 * (x0 + x1);
    step(m, y, x0, xm, 0.5 * h, depth + 1);
    step(m, y, xm, x1, 0.5 * h, depth + 1);
}

/// Solves `a d = b` in place (an `n` by `n` matrix, row-major; `d` left
/// in `b`) by Gaussian elimination with partial pivoting; false if `a` is
/// singular.
fn solve(a: &mut [f64], b: &mut [f64], n: usize) -> bool {
    for col in 0..n {
        let p = (col..n)
            .max_by(|&i, &j| a[i * n + col].abs().total_cmp(&a[j * n + col].abs()))
            .unwrap_or(col);
        if a[p * n + col].abs() < 1e-300 {
            return false;
        }
        if p != col {
            for k in 0..n {
                a.swap(p * n + k, col * n + k);
            }
            b.swap(p, col);
        }
        for r in col + 1..n {
            let f = a[r * n + col] / a[col * n + col];
            for k in col..n {
                a[r * n + k] -= f * a[col * n + k];
            }
            b[r] -= f * b[col];
        }
    }
    for r in (0..n).rev() {
        let mut s = b[r];
        for k in r + 1..n {
            s -= a[r * n + k] * b[k];
        }
        b[r] = s / a[r * n + r];
    }
    true
}

/// `x` (at `rate`) brought up to `factor` times the rate: zeros between
/// its frames, then the decimation filter (at the high rate) times
/// `factor`, its delay removed.
fn upsample(x: &[f64], taps: &[f64], factor: usize) -> Vec<f64> {
    let mut z = vec![0.0; x.len() * factor];
    for (i, &v) in x.iter().enumerate() {
        z[i * factor] = v * factor as f64;
    }
    let y = convolve(&z, taps);
    let half = taps.len() / 2;
    y[half..half + z.len()].to_vec()
}

/// `x` (at `factor` times the rate) brought down by the decimation
/// filter, its delay removed: output frame `j` is input frame `j ×
/// factor` (as [`crate::reference::decimate`], by FFT).
fn downsample(x: &[f64], taps: &[f64], factor: usize) -> Vec<f64> {
    let y = convolve(x, taps);
    let half = taps.len() / 2;
    (0..x.len() / factor)
        .map(|j| y[j * factor + half])
        .collect()
}

/// `model` fed `x` (at `rate`, from rest), solved at `factor` times the
/// rate and brought back down: one output frame for each input frame.
pub fn render(model: &dyn Model, x: &[f64], rate: u32, factor: usize) -> Vec<f64> {
    let taps = decimation_taps(rate * factor as u32);
    let up = upsample(x, &taps, factor);
    let h = 1.0 / (f64::from(rate) * factor as f64);
    let mut y = vec![0.0; model.states()];
    let mut out = Vec::with_capacity(up.len());
    let mut last = 0.0;
    for &v in &up {
        step(model, &mut y, last, v, h, 0);
        out.push(model.output(&y, v));
        last = v;
    }
    downsample(&out, &taps, factor)
}

/// The frames at a render's end the band-limiting filters leave out of
/// true (they run out of input): half the 32x filter, rounded up.
pub const TAIL: usize = 128;

/// How far the reference is from converged on `x`: the render at 16 times
/// the rate against the render at 32, the difference's RMS against the
/// 32x render's, dB, from frame `from` to [`TAIL`] frames from the end.
pub fn converged_db(model: &dyn Model, x: &[f64], rate: u32, from: usize) -> f64 {
    let a = render(model, x, rate, 16);
    let b = render(model, x, rate, 32);
    let (mut e, mut s) = (0.0f64, 0.0f64);
    for i in from..a.len().min(b.len()).saturating_sub(TAIL) {
        e += (a[i] - b[i]).powi(2);
        s += b[i] * b[i];
    }
    10.0 * (e / s.max(1e-300)).max(1e-30).log10()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signals::sine;

    const RATE: u32 = 44_100;

    /// The amplitude of `x[from..]` at `hz` against a unit sine's, by
    /// projection (the window holds whole periods).
    fn amplitude(x: &[f64], hz: f64, from: usize, periods: usize) -> f64 {
        let n = (periods as f64 * f64::from(RATE) / hz).round() as usize;
        let (mut c, mut s) = (0.0f64, 0.0f64);
        for (i, &v) in x[from..from + n].iter().enumerate() {
            let p = 2.0 * PI * hz * (from + i) as f64 / f64::from(RATE);
            c += v * p.cos();
            s += v * p.sin();
        }
        2.0 * (c * c + s * s).sqrt() / n as f64
    }

    /// The gain at `hz` (a multiple of 20 Hz, so 2,205 frames hold whole
    /// periods) once settled, clear of the render's last frames (where
    /// the band-limiting filters run out of input).
    fn gain_db(m: &dyn Model, hz: f64, amp: f64) -> f64 {
        let x = sine(hz, amp, 4_410 + 200, RATE);
        let y = render(m, &x, RATE, 16);
        20.0 * (amplitude(&y, hz, 2_205, (hz / 20.0) as usize) / amp).log10()
    }

    #[test]
    fn the_linear_state_variable_filter_is_its_exact_response() {
        for (q, out) in [
            (0.707, SvfOut::LowPass),
            (5.0, SvfOut::LowPass),
            (0.707, SvfOut::HighPass),
            (2.0, SvfOut::BandPass),
        ] {
            let m = Svf {
                cutoff_hz: 1_000.0,
                q,
                out,
            };
            for hz in [300.0f64, 1_000.0, 3_000.0] {
                let x = hz / 1_000.0;
                let den = ((1.0 - x * x).powi(2) + (x / q).powi(2)).sqrt();
                let num = match out {
                    SvfOut::LowPass => 1.0,
                    SvfOut::BandPass => x,
                    SvfOut::HighPass => x * x,
                };
                let exact = 20.0 * (num / den).log10();
                let got = gain_db(&m, hz, 0.25);
                assert!(
                    (got - exact).abs() < 0.01,
                    "Q {q} {out:?} {hz} Hz: {got} against {exact}"
                );
            }
        }
    }

    #[test]
    fn a_quiet_ladder_is_the_linear_ladder() {
        // m / ((1 + s)^4 + k) at s = j x.
        let exact = |k: f64, x: f64| {
            let (mut re, mut im) = (1.0f64, 0.0f64);
            for _ in 0..4 {
                (re, im) = (re - im * x, re * x + im);
            }
            -20.0 * ((re + k).powi(2) + im * im).sqrt().log10()
        };
        for stages in [true, false] {
            let m = Ladder {
                cutoff_hz: 1_000.0,
                k: 2.0,
                curve: Curve::Tanh,
                stages,
                makeup: 1.0,
            };
            for hz in [200.0f64, 1_000.0, 2_000.0] {
                let got = gain_db(&m, hz, 1e-4);
                let want = exact(2.0, hz / 1_000.0);
                assert!(
                    (got - want).abs() < 0.01,
                    "{stages} {hz} Hz: {got} against {want}"
                );
            }
        }
    }

    #[test]
    fn a_loud_ladder_bends_and_the_reference_converges() {
        let m = Ladder {
            cutoff_hz: 2_000.0,
            k: 3.0,
            curve: Curve::Tanh,
            stages: true,
            makeup: 1.0,
        };
        // Far from linear at 0 dBFS: the gain at the cutoff falls.
        let quiet = gain_db(&m, 2_000.0, 1e-3);
        let loud = gain_db(&m, 2_000.0, 1.0);
        assert!(quiet - loud > 3.0, "{quiet} {loud}");
        let x = sine(1_500.0, 1.0, 2_048, RATE);
        let c = converged_db(&m, &x, RATE, 512);
        assert!(c < -80.0, "{c}");
    }

    /// The pitch (Hz, by rising zero crossings) of `x`.
    fn pitch(x: &[f64]) -> f64 {
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
        count as f64 * f64::from(RATE) / (last - first.unwrap_or(0.0))
    }

    #[test]
    fn the_circuit_sings_flat_as_it_gets_louder_and_a_feedback_curve_does_not() {
        let mut click = vec![0.0; 11_025];
        click[10] = 0.1;
        let cents = |stages: bool| {
            let m = Ladder {
                cutoff_hz: 1_000.0,
                k: 5.0,
                curve: Curve::Tanh,
                stages,
                makeup: 1.0,
            };
            let y = render(&m, &click, RATE, 16);
            1200.0 * (pitch(&y[5_512..11_025 - TAIL]) / 1_000.0).log2()
        };
        let circuit = cents(true);
        let feedback = cents(false);
        assert!(circuit < -30.0, "{circuit}");
        assert!(feedback.abs() < 2.0, "{feedback}");
    }

    #[test]
    fn the_curves_have_slope_1_through_0_and_flatten() {
        for c in [Curve::Linear, Curve::Tanh, Curve::PadSoftClip] {
            assert!((c.at(1e-6) / 1e-6 - 1.0).abs() < 1e-6, "{c:?}");
        }
        assert!((Curve::PadSoftClip.at(3.0) - 1.0).abs() < 1e-12);
        assert_eq!(Curve::PadSoftClip.at(10.0), 1.0);
    }
}
