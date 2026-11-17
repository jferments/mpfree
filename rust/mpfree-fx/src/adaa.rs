//! First-order antiderivative antialiasing (ADAA; Parker, Zavalishin and
//! Le Bihan, DAFx 2016) for curves run inside oversampling (FX quality,
//! round 2, G1 and G2).
//!
//! A curve `f` sampled at single points makes harmonics to every order,
//! and those above the oversampled Nyquist fold back. ADAA plays instead
//! the curve's average over the straight line from one input sample to
//! the next, `(F(v) - F(v1)) / (v - v1)` with `F` the antiderivative of
//! `f`: the same curve heard through a one-step box filter in continuous
//! time, whose harmonics fall away far faster. Where the two samples are
//! nearly equal the quotient loses its precision, so it takes `f` at their
//! midpoint instead (the same value to within the curve's bend times the
//! step squared). `F` is worked out in 64-bit floats: it grows like `|v|`,
//! and the difference of two nearby values of it needs the extra digits.
//!
//! In the curve's straight part the average is the two-point mean
//! `(1 + z^-1) / 2`: half a step late and dull toward the top (-2.4 dB at
//! 20 kHz at 2x of 44.1 kHz). [`Flat2x`] and [`Flat4x`], short symmetric
//! FIRs after it, restore the top and make the pair a whole number of base
//! frames late, so a fixed latency and a dry path in phase still hold.

use std::f64::consts::LN_2;

/// Below this step the quotient takes the midpoint instead. At it, the
/// rounding of `F` (about 1e-16 of a value up to a few hundred) stays
/// under 1e-8, and the midpoint's error (the step squared times the bend,
/// or the step times a corner's jump) under 1e-5 of the curve's range.
const EPS: f64 = 1e-5;

/// Inputs are held to this range, so a stray huge sample can't make `F`
/// infinite and poison the state.
const LIMIT: f32 = 1.0e6;

/// One stream's ADAA state: the last input and `F` of it.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Adaa {
    v: f32,
    big: f64,
}

impl Adaa {
    /// The average of `f` (whose antiderivative is `big_f`) from the last
    /// input to `v`.
    #[inline(always)]
    pub(crate) fn run(
        &mut self,
        v: f32,
        f: impl Fn(f32) -> f32,
        big_f: impl Fn(f64) -> f64,
    ) -> f32 {
        let v = v.clamp(-LIMIT, LIMIT);
        let big = big_f(f64::from(v));
        let d = f64::from(v) - f64::from(self.v);
        let y = if d.abs() > EPS {
            ((big - self.big) / d) as f32
        } else {
            f(0.5 * (v + self.v))
        };
        self.v = v;
        self.big = big;
        y
    }

    /// The last input, for seeding another curve from it.
    pub(crate) fn last(&self) -> f32 {
        self.v
    }

    /// Starts from `v` as the last input, for a curve coming in (a mode
    /// change) so its first average is over a real step.
    pub(crate) fn seed(&mut self, v: f32, big_f: impl Fn(f64) -> f64) {
        let v = v.clamp(-LIMIT, LIMIT);
        self.v = v;
        self.big = big_f(f64::from(v));
    }

    pub(crate) fn reset(&mut self) {
        *self = Adaa::default();
    }
}

/// `ln cosh a` for `a >= 0`, the antiderivative of `tanh`, as `a` plus a
/// bounded part (so it keeps its precision at any size).
#[inline]
pub(crate) fn log_cosh(a: f64) -> f64 {
    if a > 18.0 {
        a - LN_2
    } else {
        a + ln(1.0 + exp(-2.0 * a)) - LN_2
    }
}

/// `e^x` for `|x| <= 700`, within about 1e-15 of it, without the library
/// call: `x = n ln 2 + r` with `|r| <= ln 2 / 2`, then a degree-12 Taylor
/// polynomial in `r`, scaled by `2^n` through the exponent bits. Each
/// ADAA step takes an antiderivative once a sample at twice or four times
/// the rate; the C library's `exp` and `ln_1p` were a quarter of Drive's
/// cost. The quotient needs only that the error be tiny and smooth: an
/// error `e(v)` in the antiderivative moves the average by `e'`, and
/// rounding by at most 1e-16 over the step (at least `EPS`).
#[inline]
pub(crate) fn exp(x: f64) -> f64 {
    use std::f64::consts::LOG2_E;
    // Round to nearest by the 1.5 * 2^52 trick (`round` can be a call).
    const SHIFT: f64 = 6_755_399_441_055_744.0;
    // ln 2 split so that n LN2_HI is exact (fdlibm's constants).
    const LN2_HI: f64 = f64::from_bits(0x3fe6_2e42_fee0_0000);
    const LN2_LO: f64 = f64::from_bits(0x3dea_39ef_3579_3c76);
    let x = x.clamp(-700.0, 700.0);
    let n = (x * LOG2_E + SHIFT) - SHIFT;
    let r = (x - n * LN2_HI) - n * LN2_LO;
    let mut p = INV_FACT[12];
    for c in INV_FACT[..12].iter().rev() {
        p = p * r + c;
    }
    p * f64::from_bits(((n as i64 + 1023) as u64) << 52)
}

/// `1 / k!` for `k` from 0 to 12.
const INV_FACT: [f64; 13] = {
    let mut t = [1.0f64; 13];
    let mut k = 1;
    while k < 13 {
        t[k] = t[k - 1] / k as f64;
        k += 1;
    }
    t
};

/// `1 / (2 j + 1)` for `j` from 0 to 9.
const INV_ODD: [f64; 10] = {
    let mut t = [1.0f64; 10];
    let mut j = 0;
    while j < 10 {
        t[j] = 1.0 / (2 * j + 1) as f64;
        j += 1;
    }
    t
};

/// `ln y` for a normal `y > 0`, within about 1e-16 of it, without the
/// library call: `y = 2^k m` with `m` in `[sqrt 1/2, sqrt 2)` from the
/// bits, then `ln m = 2 atanh s`, `s = (m - 1) / (m + 1)` (under 0.172),
/// by its series to `s^19`.
#[inline]
pub(crate) fn ln(y: f64) -> f64 {
    use std::f64::consts::SQRT_2;
    let bits = y.to_bits();
    let mut k = ((bits >> 52) & 0x7ff) as i64 - 1023;
    let mut m = f64::from_bits((bits & 0x000f_ffff_ffff_ffff) | 0x3ff0_0000_0000_0000);
    if m > SQRT_2 {
        m *= 0.5;
        k += 1;
    }
    let s = (m - 1.0) / (m + 1.0);
    let s2 = s * s;
    let mut p = INV_ODD[9];
    for c in INV_ODD[..9].iter().rev() {
        p = p * s2 + c;
    }
    k as f64 * LN_2 + 2.0 * s * p
}

/// The compensation after ADAA at 2x, outer tap to centre (mirrored):
/// with the two-point mean before it, 4 steps (2 base frames) late and
/// flat within 0.01 dB to 20 kHz at 2x of 44.1 kHz.
const FLAT_2X: [f32; 4] = [-0.021_215_33, 0.121_291_87, -0.366_441_37, 0.766_364_8];
/// The same at 4x: 2 steps late (half a base frame), within 0.03 dB.
const FLAT_4X: [f32; 2] = [-0.139_004_94, 0.639_004_94];

/// [`FLAT_2X`] for one stream.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Flat2x {
    x: [f32; 8],
}

impl Flat2x {
    /// Steps of delay, ADAA's half step included.
    pub(crate) const STEPS: usize = 4;

    #[inline]
    pub(crate) fn run(&mut self, v: f32) -> f32 {
        let p = self.x;
        self.x = [v, p[0], p[1], p[2], p[3], p[4], p[5], p[6]];
        let x = &self.x;
        let c = FLAT_2X;
        c[0] * (x[0] + x[7]) + c[1] * (x[1] + x[6]) + c[2] * (x[2] + x[5]) + c[3] * (x[3] + x[4])
    }

    pub(crate) fn reset(&mut self) {
        self.x = [0.0; 8];
    }
}

/// [`FLAT_4X`] for one stream.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Flat4x {
    x: [f32; 4],
}

impl Flat4x {
    /// Steps of delay, ADAA's half step included.
    pub(crate) const STEPS: usize = 2;

    #[inline]
    pub(crate) fn run(&mut self, v: f32) -> f32 {
        let p = self.x;
        self.x = [v, p[0], p[1], p[2]];
        let x = &self.x;
        FLAT_4X[0] * (x[0] + x[3]) + FLAT_4X[1] * (x[1] + x[2])
    }

    pub(crate) fn reset(&mut self) {
        self.x = [0.0; 4];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tanh_big(v: f64) -> f64 {
        log_cosh(v.abs())
    }

    #[test]
    fn a_held_input_gives_the_curve_and_a_ramp_its_average() {
        let mut a = Adaa::default();
        a.seed(0.7, tanh_big);
        // Held still: the midpoint, the curve itself.
        let y = a.run(0.7, f32::tanh, tanh_big);
        assert!((y - 0.7f32.tanh()).abs() < 1e-6, "{y}");
        // A step: the curve's mean over it, (ln cosh 2 - ln cosh 0.7) / 1.3.
        let y = a.run(2.0, f32::tanh, tanh_big);
        let want = ((2.0f64.cosh().ln() - 0.7f64.cosh().ln()) / 1.3) as f32;
        assert!((y - want).abs() < 1e-6, "{y} {want}");
        // Tiny steps far out on the curve keep their precision.
        a.seed(300.0, tanh_big);
        let y = a.run(300.0 + 1.0e-3, f32::tanh, tanh_big);
        assert!((y - 1.0).abs() < 1e-6, "{y}");
    }

    #[test]
    fn the_fast_exp_and_ln_match_the_library() {
        let mut x = -700.0f64;
        while x <= 700.0 {
            let (want, got) = (x.exp(), exp(x));
            assert!((got - want).abs() <= 2e-15 * want, "exp {x}: {got} {want}");
            x += 0.013_7;
        }
        let mut y = 1e-300f64;
        while y < 1e300 {
            let (want, got) = (y.ln(), ln(y));
            assert!(
                (got - want).abs() <= 1e-15 * want.abs().max(1.0),
                "ln {y}: {got} {want}"
            );
            y *= 1.000_9;
        }
        // Where it matters: ln cosh over the whole curve, to 2e-15.
        for k in 0..=200_000 {
            let a = k as f64 * 1e-4;
            let want = a.cosh().ln();
            assert!((log_cosh(a) - want).abs() < 2e-15 * want.max(1.0), "{a}");
        }
    }

    #[test]
    fn huge_and_odd_inputs_stay_finite() {
        let mut a = Adaa::default();
        for v in [f32::MAX, -f32::MAX, f32::INFINITY, 0.0, 1.0e-30, -5.0] {
            let y = a.run(v, f32::tanh, tanh_big);
            assert!(y.is_finite(), "{v}: {y}");
        }
    }

    #[test]
    fn the_flat_filters_undo_the_mean_and_are_whole_frames_late() {
        // The two-point mean then the FIR, on a sine well inside the band:
        // the same level, and the delay the STEPS say.
        for (os, steps, run) in [(2usize, Flat2x::STEPS, 0usize), (4, Flat4x::STEPS, 1)] {
            assert_eq!(steps % (os / 2), 0);
            let mut f2 = Flat2x::default();
            let mut f4 = Flat4x::default();
            let w = std::f32::consts::TAU * 15_000.0 / (44_100.0 * os as f32);
            let mut last = 0.0f32;
            let mut peak = 0.0f32;
            let mut err = 0.0f32;
            for n in 0..4000 {
                let x = (w * n as f32).sin();
                let m = 0.5 * (x + last);
                last = x;
                let y = if run == 0 { f2.run(m) } else { f4.run(m) };
                if n > 100 {
                    peak = peak.max(y.abs());
                    err = err.max((y - (w * (n - steps) as f32).sin()).abs());
                }
            }
            assert!((peak - 1.0).abs() < 0.01, "{os}x: {peak}");
            assert!(err < 0.01, "{os}x: {err}");
        }
    }
}
