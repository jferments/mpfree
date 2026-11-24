//! Poly's filter (DSP quality step 2.2c): one state-variable section for
//! LP 12, BP 12 and HP 12, two in a row for LP 24, each section with an
//! output mix `y = m0 x + m1 v1 + m2 v2` (`v1` its band output, `v2` its
//! low-pass output), so the one structure takes either of two designs:
//!
//! - up to [`MATCH_FROM`] of the rate, the bilinear (trapezoidal) design of
//!   the drawn analog curve with its cutoff prewarped: the filter Poly had
//!   before. (LP 24 was a ladder with the same response: the bilinear
//!   transform of a product is the product of the transforms, so the
//!   ladder's two factors as sections give the same curve.)
//! - above it, sections matched to the drawn curve (Vicanek, "Matched
//!   Second Order Digital Filters", 2016): impulse-invariant poles, and a
//!   numerator whose magnitude meets the curve's at DC and at the cutoff
//!   (low-pass) or at the cutoff (band-pass, high-pass).
//!
//! The bilinear design squeezes the whole curve into the band under
//! Nyquist, so near the top it drifts from what the screen draws: below
//! the cutoff up to 7.8 dB (LP 24 at 15 kHz), above it 14 to 43 dB in the
//! top octave. Matched, every type stays within about 1 dB of the drawn
//! curve up to 15 kHz, and the resonant peak sits where it is drawn.
//!
//! LP 24's curve is `(1 + 0.6 k) / ((1 + s)⁴ + k)`; with `r = k^¼` its
//! denominator splits into `s² + (2 ∓ √2 r) s + (1 ∓ √2 r + r²)`. The
//! first factor carries the resonance and takes the matched low-pass; the
//! second has no peak and is matched at DC and at Nyquist.

use std::f32::consts::PI;
use std::f64::consts::{FRAC_PI_2, LN_2, LOG2_E, PI as PI64, SQRT_2};

use crate::dsp::FilterMode;

/// Above this fraction of the rate (1,985 Hz at 44.1 kHz) the sections are
/// matched. Up to it the bilinear design is within 0.14 dB of the drawn
/// curve below the cutoff at any resonance, so the switch moves the
/// response by no more than that, and it costs less to work out.
pub const MATCH_FROM: f32 = 0.045;

/// One section: the state-variable filter's `a1`, `a2`, `a3`, then the
/// mix's `m0`, `m1`, `m2`.
type Sec = [f32; 6];

/// Coefficients for one setting of Poly's filter, worked out at the control
/// rate ([`PolyDesign::coefs`]) and shared by a voice's two channels. The
/// 12 dB types use the first section only.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PolyCoefs {
    s: [Sec; 2],
    /// Both sections are the bilinear design, whose mixes are the plain
    /// outputs (LP 24: `m2 v2` of the first, `v2` of the second; LP 12
    /// `v2`, BP 12 `damp v1`, HP 12 `x - damp v1 - v2`), which cost less
    /// than the full mix.
    plain: bool,
}

/// How far (octaves) the cutoff may move from the last matched design
/// before a control step works the coefficients out again: a 240th of an
/// octave, 5 cents. A matched design costs about three times the bilinear
/// one, and a filter envelope's decay moves the cutoff by a cent or two a
/// step, so most steps of a decaying envelope keep the last design (each
/// move still glides across its step). Under [`MATCH_FROM`] every move is
/// designed, as before.
pub const HOLD_OCT: f32 = 1.0 / 240.0;

/// Works out [`PolyCoefs`] a control step at a time for one voice
/// ([`PolyDesign::step`]), keeping LP 24's shape for the last resonance,
/// which moves far less often than the cutoff.
#[derive(Clone, Copy, Debug)]
pub struct PolyDesign {
    /// The resonance's bits that `shape` is for.
    res: u32,
    shape: Shape,
    /// The last design's resonance, rate and type, and its cutoff
    /// (octaves above 1 Hz); whether it was matched.
    key: [u32; 3],
    oct: f32,
    matched: bool,
}

impl Default for PolyDesign {
    fn default() -> PolyDesign {
        PolyDesign {
            // NaN patterns no input has.
            res: u32::MAX,
            shape: Shape::default(),
            key: [u32::MAX; 3],
            oct: f32::NAN,
            matched: false,
        }
    }
}

/// LP 24's two factors at one resonance.
#[derive(Clone, Copy, Debug, Default)]
struct Shape {
    /// Each factor's natural frequency in cutoffs, its damping `z` and
    /// `sqrt(1 - z²)`, and its Q.
    wn: [f64; 2],
    z: [f64; 2],
    zc: [f64; 2],
    q: [f64; 2],
    /// The second factor's `c0` and `c1`, for its gain at Nyquist.
    c0: f64,
    c1: f64,
    /// The curve's gain at DC, `(1 + 0.6 k) / (1 + k)`.
    gain: f64,
    /// For the bilinear design: each factor's frequency and damping `2 z`,
    /// and the gain.
    wn32: [f32; 2],
    k32: [f32; 2],
    gain32: f32,
}

impl Shape {
    fn lp24(res: f32) -> Shape {
        let k = 3.95 * f64::from(res);
        let r = k.sqrt().sqrt();
        let s2 = SQRT_2 * r;
        let c1 = [2.0 - s2, 2.0 + s2];
        let c0 = [1.0 - s2 + r * r, 1.0 + s2 + r * r];
        let wn = [c0[0].sqrt(), c0[1].sqrt()];
        let z = [0.5 * c1[0] / wn[0], 0.5 * c1[1] / wn[1]];
        let zc = z.map(|z| (1.0 - z * z).max(0.0).sqrt());
        let gain = (1.0 + 0.6 * k) / (1.0 + k);
        Shape {
            wn,
            z,
            zc,
            q: [wn[0] / c1[0], wn[1] / c1[1]],
            c0: c0[1],
            c1: c1[1],
            gain,
            wn32: wn.map(|w| w as f32),
            k32: z.map(|z| (2.0 * z) as f32),
            gain32: gain as f32,
        }
    }
}

impl PolyDesign {
    /// A control step's coefficients for `mode` with the cutoff at `oct`
    /// (octaves above 1 Hz) and resonance `res`, or `None` to keep the
    /// last ones: nothing has moved, or the last design was matched and
    /// the cutoff has moved less than [`HOLD_OCT`] from it.
    pub fn step(&mut self, mode: FilterMode, oct: f32, res: f32, rate: f32) -> Option<PolyCoefs> {
        let key = [res.to_bits(), rate.to_bits(), mode as u32];
        if crate::dsp::same_key(&key, &self.key)
            && (oct.to_bits() == self.oct.to_bits()
                || (self.matched && (oct - self.oct).abs() < HOLD_OCT))
        {
            return None;
        }
        self.key = key;
        self.oct = oct;
        let c = self.coefs(mode, crate::dsp::exp2(oct), res, rate);
        self.matched = !c.plain;
        Some(c)
    }

    /// The coefficients for `mode` at `fc` Hz and resonance `res` (0 to 1,
    /// 1 at the edge of self-oscillation), as
    /// [`crate::dsp::FilterCoefs::new`] takes them.
    pub fn coefs(&mut self, mode: FilterMode, fc: f32, res: f32, rate: f32) -> PolyCoefs {
        let fc = fc.clamp(10.0, 0.45 * rate);
        let res = res.clamp(0.0, 1.0);
        let matched = fc > MATCH_FROM * rate;
        if mode == FilterMode::Lp24 {
            if self.res != res.to_bits() {
                self.res = res.to_bits();
                self.shape = Shape::lp24(res);
            }
            let sh = &self.shape;
            if !matched {
                let g = (PI * fc / rate).tan();
                return PolyCoefs {
                    s: [
                        bilinear(g * sh.wn32[0], sh.k32[0], [0.0, 0.0, sh.gain32]),
                        bilinear(g * sh.wn32[1], sh.k32[1], [0.0, 0.0, 1.0]),
                    ],
                    plain: true,
                };
            }
            let w = 2.0 * PI64 * f64::from(fc) / f64::from(rate);
            let first = matched_lp(w * sh.wn[0], sh.z[0], sh.zc[0], sh.q[0], sh.gain);
            // The second factor's gain at Nyquist, `pi / w` cutoffs up.
            let sn = PI64 / w;
            let hn = sh.c0 / ((sh.c0 - sn * sn).powi(2) + (sh.c1 * sn).powi(2)).sqrt();
            let second = matched_dc_nyquist(w * sh.wn[1], sh.z[1], sh.zc[1], hn);
            return PolyCoefs {
                s: [first, second],
                plain: false,
            };
        }
        let damp = 2.0 - 1.95 * res;
        if !matched {
            let g = (PI * fc / rate).tan();
            let mix = match mode {
                FilterMode::Lp12 => [0.0, 0.0, 1.0],
                FilterMode::Bp12 => [0.0, damp, 0.0],
                _ => [1.0, -damp, -1.0],
            };
            return PolyCoefs {
                s: [bilinear(g, damp, mix), [0.0; 6]],
                plain: true,
            };
        }
        let w0 = 2.0 * PI64 * f64::from(fc) / f64::from(rate);
        let z = 0.5 * f64::from(damp);
        let zc = (1.0 - z * z).max(0.0).sqrt();
        let q = 1.0 / f64::from(damp);
        let sec = match mode {
            FilterMode::Lp12 => matched_lp(w0, z, zc, q, 1.0),
            FilterMode::Bp12 => matched_bp(w0, z, zc),
            _ => matched_hp(w0, z, zc, q),
        };
        PolyCoefs {
            s: [sec, [0.0; 6]],
            plain: false,
        }
    }
}

/// The bilinear section at prewarped `g` with damping `damp` (`1 / Q`), as
/// [`crate::dsp::FilterCoefs::new`] works its state-variable filter out.
#[inline]
fn bilinear(g: f32, damp: f32, mix: [f32; 3]) -> Sec {
    let a1 = 1.0 / (1.0 + g * (g + damp));
    let a2 = g * a1;
    [a1, a2, g * a2, mix[0], mix[1], mix[2]]
}

/// The impulse-invariant poles `(a1, a2)` of `s² + 2 z w0 s + w0²`, `w0`
/// in radians a sample, `z` at most 1 (`zc = sqrt(1 - z²)`).
#[inline]
fn poles(w0: f64, z: f64, zc: f64) -> (f64, f64) {
    let e = exp_neg(-z * w0);
    (-2.0 * e * cos_0_pi(zc * w0), e * e)
}

/// `sin²(w0 / 2)` and `cos²(w0 / 2)`, Vicanek's `phi1` and `phi0`.
#[inline]
fn phis(w0: f64) -> (f64, f64) {
    let s = sin_half_pi(0.5 * w0);
    let p1 = s * s;
    (p1, 1.0 - p1)
}

/// Vicanek's low-pass, scaled by `gain`: right at DC and at the cutoff
/// (`Q` there).
fn matched_lp(w0: f64, z: f64, zc: f64, q: f64, gain: f64) -> Sec {
    let (a1, a2) = poles(w0, z, zc);
    let n0 = 1.0 + a1 + a2;
    let den = 1.0 - a1 + a2;
    let (p1, p0) = phis(w0);
    let r1 = (n0 * n0 * p0 + den * den * p1 - 16.0 * a2 * p0 * p1) * q * q;
    let bb1 = ((r1 - n0 * n0 * p0) / p1).max(0.0);
    let b0 = 0.5 * (n0 + bb1.sqrt());
    section([b0 * gain, (n0 - b0) * gain, 0.0], a1, a2)
}

/// Vicanek's band-pass: unity at the cutoff, its peak.
fn matched_bp(w0: f64, z: f64, zc: f64) -> Sec {
    let (a1, a2) = poles(w0, z, zc);
    let (aa0, aa1, aa2) = ((1.0 + a1 + a2).powi(2), (1.0 - a1 + a2).powi(2), -4.0 * a2);
    let (p1, p0) = phis(w0);
    let r1 = aa0 * p0 + aa1 * p1 + 4.0 * aa2 * p0 * p1;
    let r2 = -aa0 + aa1 + 4.0 * (p0 - p1) * aa2;
    let bb2 = (r1 - r2 * p1) / (4.0 * p1 * p1);
    let bb1 = r2 + 4.0 * (p1 - p0) * bb2;
    let b1 = -0.5 * bb1.max(0.0).sqrt();
    let b0 = 0.5 * ((bb2 + b1 * b1).max(0.0).sqrt() - b1);
    section([b0, b1, -b0 - b1], a1, a2)
}

/// Vicanek's high-pass: right at the cutoff (`Q` there).
fn matched_hp(w0: f64, z: f64, zc: f64, q: f64) -> Sec {
    let (a1, a2) = poles(w0, z, zc);
    let (aa0, aa1, aa2) = ((1.0 + a1 + a2).powi(2), (1.0 - a1 + a2).powi(2), -4.0 * a2);
    let (p1, p0) = phis(w0);
    let b0 = (aa0 * p0 + aa1 * p1 + 4.0 * aa2 * p0 * p1).max(0.0).sqrt() * q / (4.0 * p1);
    section([b0, -2.0 * b0, b0], a1, a2)
}

/// A low-pass with no peak, right at DC and at `hn` at Nyquist.
fn matched_dc_nyquist(w0: f64, z: f64, zc: f64, hn: f64) -> Sec {
    let (a1, a2) = poles(w0, z, zc);
    let n0 = 1.0 + a1 + a2;
    let nn = hn * (1.0 - a1 + a2);
    section([0.5 * (n0 + nn), 0.5 * (n0 - nn), 0.0], a1, a2)
}

/// The biquad `b / [1, a1, a2]` as a state-variable section and its mix.
/// The section's `g` and damping `k` come from the poles (`g² = (1 + a1 +
/// a2) / (1 - a1 + a2)`, `g k = 2 (1 - a2) / (1 - a1 + a2)`, so its `a1`
/// is `(1 - a1 + a2) / 4`), and the mix from matching the numerator.
fn section(b: [f64; 3], a1: f64, a2: f64) -> Sec {
    let den = 1.0 - a1 + a2;
    let inv_den = 1.0 / den;
    let g2 = (1.0 + a1 + a2) * inv_den;
    let g = g2.sqrt();
    let gk = 2.0 * (1.0 - a2) * inv_den;
    let m0 = (b[0] - b[1] + b[2]) * inv_den;
    let m2 = (4.0 * b[1] * inv_den - 2.0 * m0 * (g2 - 1.0)) / (2.0 * g2);
    let m1 = (4.0 * (b[0] - b[2]) * inv_den - 2.0 * m0 * gk) / (2.0 * g);
    let sa1 = 0.25 * den;
    [
        sa1 as f32,
        (g * sa1) as f32,
        (g2 * sa1) as f32,
        m0 as f32,
        m1 as f32,
        m2 as f32,
    ]
}

/// `sin x` for `|x| <= pi / 2`, within 1e-9 (Taylor to `x^13`).
#[inline]
fn sin_half_pi(x: f64) -> f64 {
    let x2 = x * x;
    x * (1.0
        + x2 * (-1.0 / 6.0
            + x2 * (1.0 / 120.0
                + x2 * (-1.0 / 5040.0
                    + x2 * (1.0 / 362_880.0
                        + x2 * (-1.0 / 39_916_800.0 + x2 * (1.0 / 6_227_020_800.0)))))))
}

/// `cos x` for `0 <= x <= pi`.
#[inline]
fn cos_0_pi(x: f64) -> f64 {
    -sin_half_pi(x - FRAC_PI_2)
}

/// `e^x` for `-700 < x <= 0`, within 1e-9 (the nearest power of two taken
/// out, then Taylor to the 8th power). The power is found by converting to
/// an `i32`, which rounds toward zero, half a step down: one instruction on
/// ARMv7, where `round` and a conversion to `i64` are library calls.
#[inline]
fn exp_neg(x: f64) -> f64 {
    let n = (x * LOG2_E - 0.5) as i32;
    let f = x - f64::from(n) * LN_2;
    let p = 1.0
        + f * (1.0
            + f * (0.5
                + f * (1.0 / 6.0
                    + f * (1.0 / 24.0
                        + f * (1.0 / 120.0
                            + f * (1.0 / 720.0 + f * (1.0 / 5040.0 + f * (1.0 / 40_320.0))))))));
    p * f64::from_bits(((n + 1023) as u64) << 52)
}

/// The coefficients moving in a straight line across each control step,
/// as [`crate::dsp::FilterGlide`] moves Poly's old ones (every second
/// sample, two samples' worth at a time).
#[derive(Clone, Copy, Debug, Default)]
pub struct PolyGlide {
    /// The coefficients now.
    now: PolyCoefs,
    /// Whether the next sample is the second of a pair.
    half: bool,
    to: PolyCoefs,
    /// A sample's step while moving.
    d: [Sec; 2],
    moving: bool,
}

/// Where a buffer leaves a [`PolyGlide`], from [`PolyFilter::run_moving`]:
/// stored back with [`PolyGlide::set_pos`] once every channel has run.
#[derive(Clone, Copy, Debug)]
pub struct PolyPos {
    now: PolyCoefs,
    half: bool,
}

impl PolyGlide {
    /// Jumps to `c` and holds there (a fresh note).
    pub fn snap(&mut self, c: PolyCoefs) {
        self.now = c;
        self.half = false;
        self.to = c;
        self.moving = false;
    }

    /// Takes up where a buffer left it.
    pub fn set_pos(&mut self, p: PolyPos) {
        self.now = p.now;
        self.half = p.half;
    }

    /// The start of a control step of `frames`: lands exactly on the last
    /// step's coefficients, then heads for `c`, or holds with `None`. While
    /// it moves, the plain outputs are used only if both ends have them
    /// (between two plain ends the mixes stay plain).
    pub fn step(&mut self, c: Option<PolyCoefs>, frames: u32) {
        self.now = self.to;
        self.half = false;
        self.moving = false;
        if let Some(c) = c {
            let per = 1.0 / frames.max(1) as f32;
            for (d, (a, b)) in self.d.iter_mut().zip(self.now.s.iter().zip(&c.s)) {
                for i in 0..6 {
                    d[i] = (b[i] - a[i]) * per;
                }
            }
            self.now.plain &= c.plain;
            self.to = c;
            self.moving = true;
        }
    }

    /// Whether this step moves the coefficients.
    pub fn moving(&self) -> bool {
        self.moving
    }
}

/// One channel of Poly's filter state: the two sections' integrators.
#[derive(Clone, Copy, Debug, Default)]
pub struct PolyFilter {
    s: [f32; 4],
}

/// One sample through a section's state-variable filter: `(v1, v2)`.
#[inline(always)]
fn svf(ic: &mut [f32; 2], x: f32, c: &Sec) -> (f32, f32) {
    let v3 = x - ic[1];
    let v1 = c[0] * ic[0] + c[1] * v3;
    let v2 = ic[1] + c[1] * ic[0] + c[2] * v3;
    ic[0] = 2.0 * v1 - ic[0];
    ic[1] = 2.0 * v2 - ic[1];
    (v1, v2)
}

/// A section's output: the full mix, or with `PLAIN` the plain output of
/// type `OUT` (0 low-pass, 1 band-pass, 2 high-pass), the same numbers
/// without the terms that are zero.
#[inline(always)]
fn mix<const OUT: u8, const PLAIN: bool>(x: f32, v1: f32, v2: f32, c: &Sec) -> f32 {
    if PLAIN {
        match OUT {
            0 => v2,
            1 => v1 * c[4],
            _ => x + c[4] * v1 - v2,
        }
    } else {
        c[3] * x + c[4] * v1 + c[5] * v2
    }
}

/// One sample through the filter of type `OUT` (3 for LP 24's two
/// sections, else one section of that type).
#[inline(always)]
fn tick<const OUT: u8, const PLAIN: bool>(s: &mut [[f32; 2]; 2], x: f32, c: &[Sec; 2]) -> f32 {
    if OUT == 3 {
        let (v1, v2) = svf(&mut s[0], x, &c[0]);
        let y = if PLAIN {
            c[0][5] * v2
        } else {
            mix::<0, false>(x, v1, v2, &c[0])
        };
        let (w1, w2) = svf(&mut s[1], y, &c[1]);
        mix::<0, PLAIN>(y, w1, w2, &c[1])
    } else {
        let (v1, v2) = svf(&mut s[0], x, &c[0]);
        mix::<OUT, PLAIN>(x, v1, v2, &c[0])
    }
}

/// Moves `c` by `d2`, two samples' worth: only the terms that can move
/// (with `PLAIN` the fixed parts of the mixes stay as they are, and their
/// step is exactly zero anyway).
#[inline(always)]
fn advance<const OUT: u8, const PLAIN: bool>(c: &mut [Sec; 2], d2: &[Sec; 2]) {
    let n = if OUT == 3 { 2 } else { 1 };
    for j in 0..n {
        for i in 0..3 {
            c[j][i] += d2[j][i];
        }
        if PLAIN {
            // The band-pass and high-pass mixes carry the damping, LP 24's
            // first section the gain; both move only with the resonance.
            if (OUT == 1 || OUT == 2) && j == 0 {
                c[j][4] += d2[j][4];
            }
            if OUT == 3 && j == 0 {
                c[j][5] += d2[j][5];
            }
        } else {
            for i in 3..6 {
                c[j][i] += d2[j][i];
            }
        }
    }
}

impl PolyFilter {
    pub fn clear(&mut self) {
        self.s = [0.0; 4];
    }

    /// True when both channels hold exactly the same numbers, so the same
    /// input gives the same output.
    #[inline]
    pub fn same_as(&self, other: &PolyFilter) -> bool {
        self.s
            .iter()
            .zip(&other.s)
            .all(|(a, b)| a.to_bits() == b.to_bits())
    }

    /// Filters `buf` in place with `c` held.
    #[inline]
    pub fn run(&mut self, mode: FilterMode, c: &PolyCoefs, buf: &mut [f32]) {
        match (mode, c.plain) {
            (FilterMode::Lp24, true) => self.held::<3, true>(c, buf),
            (FilterMode::Lp24, false) => self.held::<3, false>(c, buf),
            (FilterMode::Lp12, true) => self.held::<0, true>(c, buf),
            (FilterMode::Lp12, false) => self.held::<0, false>(c, buf),
            (FilterMode::Bp12, true) => self.held::<1, true>(c, buf),
            (FilterMode::Bp12, false) => self.held::<1, false>(c, buf),
            (FilterMode::Hp12, true) => self.held::<2, true>(c, buf),
            (FilterMode::Hp12, false) => self.held::<2, false>(c, buf),
        }
    }

    /// Filters `buf` in place with `glide`'s coefficients, moving them
    /// while it moves; gives back where the buffer leaves the glide.
    #[inline]
    pub fn run_moving(&mut self, mode: FilterMode, glide: &PolyGlide, buf: &mut [f32]) -> PolyPos {
        if !glide.moving {
            self.run(mode, &glide.now, buf);
            return PolyPos {
                now: glide.now,
                half: glide.half,
            };
        }
        let (c, d, h) = (glide.now, &glide.d, glide.half);
        match (mode, c.plain) {
            (FilterMode::Lp24, true) => self.moving::<3, true>(c, d, h, buf),
            (FilterMode::Lp24, false) => self.moving::<3, false>(c, d, h, buf),
            (FilterMode::Lp12, true) => self.moving::<0, true>(c, d, h, buf),
            (FilterMode::Lp12, false) => self.moving::<0, false>(c, d, h, buf),
            (FilterMode::Bp12, true) => self.moving::<1, true>(c, d, h, buf),
            (FilterMode::Bp12, false) => self.moving::<1, false>(c, d, h, buf),
            (FilterMode::Hp12, true) => self.moving::<2, true>(c, d, h, buf),
            (FilterMode::Hp12, false) => self.moving::<2, false>(c, d, h, buf),
        }
    }

    #[inline]
    fn held<const OUT: u8, const PLAIN: bool>(&mut self, c: &PolyCoefs, buf: &mut [f32]) {
        let mut s = [[self.s[0], self.s[1]], [self.s[2], self.s[3]]];
        for x in buf {
            *x = tick::<OUT, PLAIN>(&mut s, *x, &c.s);
        }
        self.s = [s[0][0], s[0][1], s[1][0], s[1][1]];
    }

    /// The filter with its coefficients `c` moving by `d` a sample, two
    /// samples' worth every second sample; `half` when the first sample
    /// ends a pair.
    #[inline]
    fn moving<const OUT: u8, const PLAIN: bool>(
        &mut self,
        mut c: PolyCoefs,
        d: &[Sec; 2],
        half: bool,
        buf: &mut [f32],
    ) -> PolyPos {
        let mut s = [[self.s[0], self.s[1]], [self.s[2], self.s[3]]];
        let d2 = d.map(|d| d.map(|v| 2.0 * v));
        let first = usize::from(half && !buf.is_empty());
        if first == 1 {
            buf[0] = tick::<OUT, PLAIN>(&mut s, buf[0], &c.s);
            advance::<OUT, PLAIN>(&mut c.s, &d2);
        }
        let mut open = half && first == 0;
        let mut pairs = buf[first..].chunks_exact_mut(2);
        for pair in &mut pairs {
            pair[0] = tick::<OUT, PLAIN>(&mut s, pair[0], &c.s);
            pair[1] = tick::<OUT, PLAIN>(&mut s, pair[1], &c.s);
            advance::<OUT, PLAIN>(&mut c.s, &d2);
        }
        if let Some(x) = pairs.into_remainder().first_mut() {
            *x = tick::<OUT, PLAIN>(&mut s, *x, &c.s);
            open = true;
        }
        self.s = [s[0][0], s[0][1], s[1][0], s[1][1]];
        PolyPos { now: c, half: open }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::{Filter, FilterCoefs, FilterGlide, Rng};

    const RATE: f32 = 44_100.0;
    const MODES: [FilterMode; 4] = [
        FilterMode::Lp24,
        FilterMode::Lp12,
        FilterMode::Bp12,
        FilterMode::Hp12,
    ];

    /// A test signal with steps, both signs and loud parts.
    fn signal(n: usize) -> Vec<f32> {
        let mut rng = Rng::new(7);
        (0..n)
            .map(|i| rng.bipolar() * if i % 50 < 25 { 3.0 } else { 0.4 })
            .collect()
    }

    fn same(a: &[f32], b: &[f32]) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(p, q)| p.to_bits() == q.to_bits())
    }

    /// A section's response at `w` radians a sample, from its coefficients
    /// (`g = a2 / a1`, `1 + g k + g² = 1 / a1`).
    fn sec_response(c: &Sec, w: f64) -> (f64, f64) {
        let (a1, a2) = (f64::from(c[0]), f64::from(c[1]));
        let g = a2 / a1;
        let d0 = 1.0 / a1;
        let gk = d0 - 1.0 - g * g;
        // z^-1 and z^-2.
        let (c1, s1) = (w.cos(), -w.sin());
        let (c2, s2) = ((2.0 * w).cos(), -(2.0 * w).sin());
        let poly = |p: [f64; 3]| (p[0] + p[1] * c1 + p[2] * c2, p[1] * s1 + p[2] * s2);
        let den = poly([d0, 2.0 * g * g - 2.0, 1.0 - gk + g * g]);
        let lp = poly([g * g, 2.0 * g * g, g * g]);
        let bp = poly([g, 0.0, -g]);
        let (m0, m1, m2) = (f64::from(c[3]), f64::from(c[4]), f64::from(c[5]));
        let num = (
            m0 * den.0 + m1 * bp.0 + m2 * lp.0,
            m0 * den.1 + m1 * bp.1 + m2 * lp.1,
        );
        let dd = den.0 * den.0 + den.1 * den.1;
        (
            (num.0 * den.0 + num.1 * den.1) / dd,
            (num.1 * den.0 - num.0 * den.1) / dd,
        )
    }

    fn gain(mode: FilterMode, c: &PolyCoefs, hz: f64) -> f64 {
        let w = std::f64::consts::TAU * hz / f64::from(RATE);
        let (re, im) = sec_response(&c.s[0], w);
        let mut g = (re * re + im * im).sqrt();
        if mode == FilterMode::Lp24 {
            let (re, im) = sec_response(&c.s[1], w);
            g *= (re * re + im * im).sqrt();
        }
        g
    }

    #[test]
    fn a_section_filters_as_its_biquad() {
        // Poles of a resonant low-pass at 9 kHz and a numerator with all
        // three taps, through the section and through the biquad itself.
        let (a1, a2) = poles(1.3, 0.2, (1.0f64 - 0.04).sqrt());
        let b = [0.31, -0.12, 0.05];
        let c = section(b, a1, a2);
        let x = signal(256);
        let mut f = PolyFilter::default();
        let mut y = x.clone();
        f.held::<0, false>(
            &PolyCoefs {
                s: [c, [0.0; 6]],
                plain: false,
            },
            &mut y,
        );
        let (mut z1, mut z2) = (0.0f64, 0.0f64);
        for (&xi, &yi) in x.iter().zip(&y) {
            let xi = f64::from(xi);
            let out = b[0] * xi + z1;
            z1 = b[1] * xi - a1 * out + z2;
            z2 = b[2] * xi - a2 * out;
            assert!(
                (out - f64::from(yi)).abs() < 1e-4 * (1.0 + out.abs()),
                "{out} {yi}"
            );
        }
    }

    #[test]
    fn under_the_switch_the_twelve_db_types_are_the_old_filter_to_the_bit() {
        for mode in &MODES[1..] {
            let mut d = PolyDesign::default();
            for (fc, res) in [(300.0, 0.0), (900.0, 0.6), (1_900.0, 1.0)] {
                let new = d.coefs(*mode, fc, res, RATE);
                assert!(new.plain);
                let old = FilterCoefs::new(fc, res, RATE);
                let mut x1 = signal(200);
                let mut x2 = x1.clone();
                Filter::default().run(*mode, &old, &mut x1);
                PolyFilter::default().run(*mode, &new, &mut x2);
                assert!(same(&x1, &x2), "{mode:?} {fc} {res}");
            }
            // Moving between two settings under the switch, the cutoff and
            // the resonance both.
            let (a, b) = ((500.0, 0.3), (1_800.0, 0.8));
            let mut og = FilterGlide::default();
            og.snap(FilterCoefs::new(a.0, a.1, RATE));
            og.step(Some(FilterCoefs::new(b.0, b.1, RATE)), 16);
            let mut ng = PolyGlide::default();
            ng.snap(d.coefs(*mode, a.0, a.1, RATE));
            ng.step(Some(d.coefs(*mode, b.0, b.1, RATE)), 16);
            let mut x1 = signal(16);
            let mut x2 = x1.clone();
            Filter::default().run_moving(*mode, &og, &mut x1);
            PolyFilter::default().run_moving(*mode, &ng, &mut x2);
            assert!(same(&x1, &x2), "{mode:?} moving");
        }
    }

    #[test]
    fn under_the_switch_lp_24_is_the_old_ladder() {
        let mut d = PolyDesign::default();
        for (fc, res) in [(200.0, 0.0), (800.0, 0.5), (1_900.0, 0.9), (1_000.0, 1.0)] {
            let new = d.coefs(FilterMode::Lp24, fc, res, RATE);
            let old = FilterCoefs::new(fc, res, RATE);
            let mut x1 = signal(4_000);
            let mut x2 = x1.clone();
            Filter::default().run(FilterMode::Lp24, &old, &mut x1);
            PolyFilter::default().run(FilterMode::Lp24, &new, &mut x2);
            let peak = x1.iter().fold(0.0f32, |p, v| p.max(v.abs()));
            let err = x1
                .iter()
                .zip(&x2)
                .fold(0.0f32, |e, (p, q)| e.max((p - q).abs()));
            assert!(err < 2e-4 * peak, "{fc} {res}: {err} of {peak}");
        }
    }

    #[test]
    fn a_held_glide_filters_as_the_plain_run_and_a_moving_one_lands_exactly() {
        // Under the switch, over it, and across it.
        let ends = [
            ((800.0, 0.6), (1_600.0, 0.9)),
            ((6_000.0, 0.6), (14_000.0, 0.9)),
            ((800.0, 0.2), (9_000.0, 0.7)),
        ];
        for mode in MODES {
            for &(a, b) in &ends {
                let mut d = PolyDesign::default();
                let ca = d.coefs(mode, a.0, a.1, RATE);
                let cb = d.coefs(mode, b.0, b.1, RATE);
                // Held: the same samples, to the bit, as `run`.
                let mut g = PolyGlide::default();
                g.snap(ca);
                g.step(None, 16);
                assert!(!g.moving());
                let (mut f1, mut f2) = (PolyFilter::default(), PolyFilter::default());
                let mut x1 = signal(64);
                let mut x2 = x1.clone();
                f1.run(mode, &ca, &mut x1);
                g.set_pos(f2.run_moving(mode, &g, &mut x2));
                assert!(same(&x1, &x2));
                // Moving: a step cut in pieces gives the same samples as one
                // run, and the next step starts exactly on the target.
                let mut g1 = PolyGlide::default();
                g1.snap(ca);
                g1.step(Some(cb), 16);
                let mut g2 = g1;
                let (mut f1, mut f2) = (PolyFilter::default(), PolyFilter::default());
                let mut x1 = signal(16);
                let mut x2 = x1.clone();
                g1.set_pos(f1.run_moving(mode, &g1, &mut x1));
                for (from, to) in [(0, 5), (5, 5), (5, 6), (6, 9), (9, 16)] {
                    g2.set_pos(f2.run_moving(mode, &g2, &mut x2[from..to]));
                }
                assert!(same(&x1, &x2), "{mode:?} {a:?} {b:?}");
                assert!(f1.same_as(&f2));
                g1.step(None, 16);
                let mut x3 = signal(8);
                let mut x4 = x3.clone();
                let mut fb = f1;
                f1.run(mode, &cb, &mut x3);
                fb.run_moving(mode, &g1, &mut x4);
                assert!(same(&x3, &x4), "{mode:?} lands");
            }
        }
    }

    #[test]
    fn matched_sections_meet_the_drawn_curve_where_they_are_pinned() {
        let mut d = PolyDesign::default();
        for fc in [3_000.0, 8_000.0, 15_000.0, 19_000.0] {
            for res in [0.0, 0.5, 0.9, 1.0] {
                let k = 3.95 * f64::from(res);
                let c = d.coefs(FilterMode::Lp24, fc, res, RATE);
                assert!(!c.plain);
                let dc = (1.0 + 0.6 * k) / (1.0 + k);
                assert!((gain(FilterMode::Lp24, &c, 0.0) / dc - 1.0).abs() < 1e-4);
                let c = d.coefs(FilterMode::Lp12, fc, res, RATE);
                assert!((gain(FilterMode::Lp12, &c, 0.0) - 1.0).abs() < 1e-4);
                // The low-pass and high-pass at the cutoff (Q there) and the
                // band-pass at its peak, on the drawn curves.
                let q = 1.0 / (2.0 - 1.95 * f64::from(res));
                let got = gain(FilterMode::Lp12, &c, f64::from(fc));
                assert!((got / q - 1.0).abs() < 1e-3, "LP {fc} {res}: {got} {q}");
                let c = d.coefs(FilterMode::Hp12, fc, res, RATE);
                let got = gain(FilterMode::Hp12, &c, f64::from(fc));
                assert!((got / q - 1.0).abs() < 1e-3, "HP {fc} {res}: {got} {q}");
                let c = d.coefs(FilterMode::Bp12, fc, res, RATE);
                let got = gain(FilterMode::Bp12, &c, f64::from(fc));
                assert!((got - 1.0).abs() < 1e-3, "BP {fc} {res}: {got}");
            }
        }
    }

    #[test]
    fn the_series_are_within_their_range_and_close() {
        let mut x = 0.0;
        while x <= std::f64::consts::FRAC_PI_2 {
            assert!((sin_half_pi(x) - x.sin()).abs() < 1e-9);
            assert!((cos_0_pi(2.0 * x) - (2.0 * x).cos()).abs() < 1e-9);
            assert!((exp_neg(-4.0 * x) - (-4.0 * x).exp()).abs() < 1e-12 + 1e-9 * (-4.0 * x).exp());
            x += 1e-3;
        }
    }
}
