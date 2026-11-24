//! The transistor ladder (the filter gap plan's step F6): four one-pole
//! low-passes in a loop, the last fed back, with a curve in every stage,
//! as a real ladder has (Huovilainen's model, DAFx 2004). It is the
//! circuit's behaviour where a ladder differs from a plain 24 dB
//! low-pass: a loud sound bends at the input pair and in every stage, a
//! driven ladder loses resonance, and a ladder that sings settles at the
//! level its stages allow and falls a little flat as it does (20 cents at
//! the pad's full Resonance, feedback 4.2), which [`tuned`] puts back.
//!
//! **The model**, in the ladder's own units (its curve bends from about
//! 1; full scale in the pad): `y1' = ωc (c(x - k y4) - c(y1))`, `yi' = ωc
//! (c(y(i-1)) - c(yi))`, the output `y4`. The curve `c` is the [3/2] Padé
//! form of `tanh`, `v (27 + v²) / (27 + 9 v²)`, flat at ±1 from ±3 (where
//! its slope has come down to 0), the pads' soft clip.
//!
//! **The solve** (the plan's section 3, its way 3, measured in
//! `mpfree_fxlab::ladder_study`): each sample, every curve is replaced by
//! its tangent where the trapezoidal states put the voltages (a
//! trapezoidal state runs half a step ahead of its output, so it predicts
//! them), and the loop is then solved exactly as a linear one, with one
//! division for all its reciprocals. At twice the rate this is within
//! 0.05 % of the exact model's harmonics (the reference solves Newton's
//! method to convergence at 16 times the rate) and sings within 2 cents
//! of where it sings; a Newton step more costs 75 % more and gains a
//! hundredth of that. The secant through 0 instead of the tangent
//! aliases less but sings up to 100 cents off at 5 kHz; at the session's
//! own rate the tangent aliases at -20 dB near the top. So the pads run
//! it at twice the rate ([`crate`]'s hosts own the half-bands).
//!
//! **Stability:** loud noise while the cutoff jumps anywhere from 20 Hz to
//! 20 kHz and the feedback from 0 to 6 every 1 to 64 samples stays finite
//! and under 1.5 (the lab's M4): every curve is bounded, so the loop is.
//!
//! No allocation and no branches.

/// Where the ladder starts to sing: a feedback past 4 (four equal stages
/// turn half a cycle at the cutoff, losing 12 dB there).
pub const SINGS_FROM: f32 = 4.0;

/// The cutoff a ladder with feedback `k` is set to so that it sings at
/// the cutoff it is asked for, as a ratio: past [`SINGS_FROM`], where it
/// settles at a level that bends its stages and falls flat (2.2 cents at
/// 4.02, 19.6 at 4.2, 69 at 5, measured through [`LadderCore`] at 250 Hz,
/// 1 kHz and 4 kHz, the same within 0.3 cents), `2^(A d / (1 + B d) /
/// 1200)` with `d = k - 4`; 1 below. Within 0.5 cents of every measured
/// point.
#[inline]
pub fn tuned(k: f32) -> f32 {
    const A: f32 = 108.5;
    const B: f32 = 0.572;
    let d = (k - SINGS_FROM).max(0.0);
    (A * d / (1.0 + B * d) * (1.0 / 1200.0)).exp2()
}

/// One stage's prewarped gain `tan(π fc / rate)` (the cutoff held under
/// 45 % of `rate`).
#[inline]
pub fn stage_gain(cutoff_hz: f32, rate: f32) -> f32 {
    (std::f32::consts::PI * cutoff_hz.clamp(1.0, 0.45 * rate) / rate).tan()
}

/// The curve's denominator at `v` held to ±3, `27 + 9 v²` (where `v` is
/// past ±3 the clamped `v` gives the curve's flat `±1` and slope 0, with
/// no branch).
#[inline(always)]
fn den(v: f32) -> f32 {
    let v = v.clamp(-3.0, 3.0);
    27.0 + 9.0 * v * v
}

/// The tangent to the curve at `v`, `c(w) ≈ a w + b`, given `r = 1 /
/// den(v)`: with `h` the clamped `v`, `c = h (27 + h²) r` and `c' = 9 ((9
/// - h²) r)²`, which is 0 at ±3, so past them `a` is 0 and `b` is `±1`.
#[inline(always)]
fn tangent(v: f32, r: f32) -> (f32, f32) {
    let h = v.clamp(-3.0, 3.0);
    let h2 = h * h;
    let c = h * (27.0 + h2) * r;
    let q = (9.0 - h2) * r;
    let a = 9.0 * q * q;
    (a, c - a * v)
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
    [inv * d[1], inv * d[0], r2, r3, r4]
}

/// One channel of the ladder at its own rate: the four integrators'
/// trapezoidal states. (Both channels side by side in one pass, as pairs
/// of lanes, came out 18 % dearer: the compiler didn't put them in one
/// register.)
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LadderCore {
    s: [f32; 4],
}

impl LadderCore {
    /// One sample: `x` in, `y4` out, with each stage's prewarped gain `g`
    /// ([`stage_gain`]) and the feedback `k`.
    #[inline(always)]
    pub fn step(&mut self, x: f32, g: f32, k: f32) -> f32 {
        let s = self.s;
        // Where the voltages are about to be: the input pair's `u` and
        // each stage's output.
        let p = [x - k * s[3], s[0], s[1], s[2], s[3]];
        let r = recip5([den(p[0]), den(p[1]), den(p[2]), den(p[3]), den(p[4])]);
        let (a0, b0) = tangent(p[0], r[0]);
        let (a1, b1) = tangent(p[1], r[1]);
        let (a2, b2) = tangent(p[2], r[2]);
        let (a3, b3) = tangent(p[3], r[3]);
        let (a4, b4) = tangent(p[4], r[4]);
        // Stage i, with its curves straightened: yi (1 + g a_i) = s_i + g
        // (a_(i-1) v_(i-1) + b_(i-1) - b_i), so yi = al_i v_(i-1) + be_i,
        // and y4 = (Q x + P B) / (P + k Q) through the loop, where P and Q
        // are the products of the stages' `1 + g a_i` and `g a_(i-1)`.
        let c = [1.0 + g * a1, 1.0 + g * a2, 1.0 + g * a3, 1.0 + g * a4];
        let ga = [g * a0, g * a1, g * a2, g * a3];
        let p01 = c[0] * c[1];
        let p012 = p01 * c[2];
        let big_p = p012 * c[3];
        let q = ga[0] * ga[1] * ga[2] * ga[3];
        let e = big_p + k * q;
        let inv = 1.0 / (big_p * e);
        let inv_p = e * inv;
        let p23 = c[2] * c[3];
        let r0 = inv_p * c[1] * p23;
        let r1 = inv_p * c[0] * p23;
        let r2 = inv_p * p01 * c[3];
        let r3 = inv_p * p012;
        let be = [
            (s[0] + g * (b0 - b1)) * r0,
            (s[1] + g * (b1 - b2)) * r1,
            (s[2] + g * (b2 - b3)) * r2,
            (s[3] + g * (b3 - b4)) * r3,
        ];
        let al = [ga[0] * r0, ga[1] * r1, ga[2] * r2, ga[3] * r3];
        let b = ((be[0] * al[1] + be[1]) * al[2] + be[2]) * al[3] + be[3];
        let y4 = (q * x + big_p * b) * (big_p * inv);
        let u = x - k * y4;
        let y1 = al[0] * u + be[0];
        let y2 = al[1] * y1 + be[1];
        let y3 = al[2] * y2 + be[2];
        self.s = [
            2.0 * y1 - s[0],
            2.0 * y2 - s[1],
            2.0 * y3 - s[2],
            2.0 * y4 - s[3],
        ];
        y4
    }

    /// The last stage's state, about what it puts out now.
    #[inline]
    pub fn out(&self) -> f32 {
        self.s[3]
    }

    /// Zeros a state too small to hear.
    #[inline]
    pub fn flush(&mut self) {
        for x in &mut self.s {
            *x = crate::flush(*x);
        }
    }

    /// Back to rest.
    #[inline]
    pub fn reset(&mut self) {
        self.s = [0.0; 4];
    }
}

/// The linear ladder's gain at `hz` (dB) for a quiet sound: `1 / ((1 +
/// s)^4 + k)` at `s = j hz / fc`, the analog prototype (the bilinear
/// response at twice the rate differs by 0.1 dB at 3 kHz under a 1 kHz
/// cutoff).
pub fn quiet_db(cutoff_hz: f32, k: f32, hz: f32) -> f32 {
    let x = hz / cutoff_hz.max(1.0);
    let (mut re, mut im) = (1.0f32, 0.0f32);
    for _ in 0..4 {
        (re, im) = (re - im * x, re * x + im);
    }
    -10.0 * ((re + k) * (re + k) + im * im).max(1e-30).log10()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sine's amplitude once settled, by projection over whole periods.
    fn amplitude(y: &[f32], hz: f32, rate: f32, from: usize) -> f32 {
        let n = (rate / hz * 20.0).round() as usize;
        let (mut c, mut s) = (0.0f64, 0.0f64);
        for (i, &v) in y[from..from + n].iter().enumerate() {
            let p = std::f64::consts::TAU * f64::from(hz) * (from + i) as f64 / f64::from(rate);
            c += f64::from(v) * p.cos();
            s += f64::from(v) * p.sin();
        }
        (2.0 * (c * c + s * s).sqrt() / n as f64) as f32
    }

    #[test]
    fn a_quiet_ladder_is_the_linear_ladder() {
        let rate = 88_200.0;
        for k in [0.0, 2.0, 3.5] {
            for hz in [200.0, 1_000.0, 3_000.0] {
                let mut l = LadderCore::default();
                let g = stage_gain(1_000.0, rate);
                let n = 20_000;
                let y: Vec<f32> = (0..n)
                    .map(|i| {
                        let x = 1e-4 * (std::f32::consts::TAU * hz * i as f32 / rate).sin();
                        l.step(x, g, k)
                    })
                    .collect();
                let got = 20.0 * (amplitude(&y, hz, rate, 10_000) / 1e-4).log10();
                // The bilinear map: the analog response at the warped ratio.
                let warped = (std::f32::consts::PI * hz / rate).tan() / g * 1_000.0;
                let want = quiet_db(1_000.0, k, warped);
                assert!(
                    (got - want).abs() < 0.02,
                    "k {k} {hz} Hz: {got} against {want}"
                );
            }
        }
    }

    #[test]
    fn a_loud_sound_bends_and_stays_bounded() {
        let rate = 88_200.0;
        let g = stage_gain(2_000.0, rate);
        let mut l = LadderCore::default();
        let mut peak = 0.0f32;
        for i in 0..40_000 {
            let x = 8.0 * (std::f32::consts::TAU * 150.0 * i as f32 / rate).sin();
            let y = l.step(x, g, 3.9);
            assert!(y.is_finite());
            peak = peak.max(y.abs());
        }
        // The curves hold it near the ladder's unit, whatever comes in.
        assert!(peak < 2.0, "{peak}");
    }

    #[test]
    fn it_sings_flat_past_4_and_tuned_puts_it_back() {
        let rate = 88_200.0;
        for (k, flat) in [(4.2f32, -19.5f32), (5.0, -69.0)] {
            for tune in [false, true] {
                let fc = 1_000.0 * if tune { tuned(k) } else { 1.0 };
                let g = stage_gain(fc, rate);
                let mut l = LadderCore::default();
                let n = (rate * 3.0) as usize;
                let mut y = Vec::with_capacity(n);
                for i in 0..n {
                    y.push(l.step(if i == 10 { 0.1 } else { 0.0 }, g, k));
                }
                // Rising zero crossings over the last second.
                let tail = &y[n - rate as usize..];
                let (mut first, mut last, mut count) = (None, 0.0f64, 0usize);
                for i in 1..tail.len() {
                    if tail[i - 1] < 0.0 && tail[i] >= 0.0 {
                        let t = (i - 1) as f64
                            - f64::from(tail[i - 1]) / f64::from(tail[i] - tail[i - 1]);
                        if first.is_none() {
                            first = Some(t);
                        } else {
                            count += 1;
                        }
                        last = t;
                    }
                }
                let hz = count as f64 * f64::from(rate) / (last - first.unwrap());
                let cents = 1200.0 * (hz / 1_000.0).log2();
                let want = if tune { 0.0 } else { f64::from(flat) };
                assert!((cents - want).abs() < 1.0, "k {k} tuned {tune}: {cents}");
            }
        }
    }

    #[test]
    fn tuned_is_1_until_it_sings() {
        assert_eq!(tuned(0.0), 1.0);
        assert_eq!(tuned(4.0), 1.0);
        assert!(tuned(4.2) > 1.0);
    }
}
