//! Zero-delay-feedback (trapezoidal) state-variable filters, after Andrew
//! Simper's "Linear Trapezoidal Integrated SVF" (Cytomic, 2013): stable at
//! any cutoff below Nyquist, with its cutoff moving however fast, and with
//! its resonance moving as long as the damping never reaches zero
//! (McClellan, DAFx 2026), so a sweep never blows up. One stage is 12 dB an
//! octave; its band-pass state can
//! be bounded ([`Svf::run_bounded`], [`Svf::run_clipped`]) as an analog
//! filter's resonance loop saturates. Moved here from `mpfree-fx` (the
//! Filter effect's and the EQ's stage) so the pads run the same one.

use std::f32::consts::PI;

use crate::{db_to_gain, flush};

/// How wide [`hard`]'s rounded edge is either side of its ceiling, as a
/// part of it.
pub const HARD_ROUND: f32 = 0.25;

/// What a [`Svf`] outputs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SvfMode {
    LowPass,
    HighPass,
    BandPass,
    Notch,
    /// A bell (peaking EQ) with the gain given to [`Svf::set`].
    Bell,
    LowShelf,
    HighShelf,
}

/// A design for [`Svf`]: the analog prototype it is the bilinear transform
/// of, `H(s) = m0 + (m2 + m1 s) / (s^2 + k s + 1)` with `s` scaled so
/// the prewarped corner `g = tan(pi hz / rate)` is 1. Any stable digital
/// biquad is one of these, so a design made another way (`matched`) runs
/// in the same filter, sweeps as safely, and [`Svf::response`] stays exact.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SvfCoefs {
    pub g: f32,
    pub k: f32,
    pub m0: f32,
    pub m1: f32,
    pub m2: f32,
}

impl SvfCoefs {
    /// The bilinear transform of the analog filter (Simper's design): exact
    /// at low frequencies, but the bell and shelves cramp toward Nyquist (a
    /// 12 kHz, Q 0.71 bell at 44.1 kHz is 6.6 dB short at 20 kHz). The
    /// cutoff is kept between 5 Hz and 45 % of the sample rate.
    #[inline]
    pub fn bilinear(mode: SvfMode, hz: f32, q: f32, gain_db: f32, rate: u32) -> SvfCoefs {
        SvfCoefs::bilinear_under(mode, hz, q, gain_db, rate, 0.45)
    }

    /// [`SvfCoefs::bilinear`] with the cutoff kept under `top` of the
    /// sample rate (under a half) instead of 45 %. Any cutoff under
    /// Nyquist is a stable filter; 45 % keeps the bell and shelves out of
    /// the cramped top, while a low-pass wide open wants to reach higher.
    #[inline]
    pub fn bilinear_under(
        mode: SvfMode,
        hz: f32,
        q: f32,
        gain_db: f32,
        rate: u32,
        top: f32,
    ) -> SvfCoefs {
        SvfCoefs::bilinear_at(mode, corner(hz, rate, top), q, gain_db)
    }

    /// [`SvfCoefs::bilinear`] at a prewarped corner `t` worked out already
    /// ([`corner`]), so the stages of one filter share it.
    #[inline]
    pub fn bilinear_at(mode: SvfMode, t: f32, q: f32, gain_db: f32) -> SvfCoefs {
        let q = q.max(0.05);
        // The square root of the linear gain, for the modes that have one.
        let a = || db_to_gain(gain_db * 0.5);
        let (g, k, m0, m1, m2) = match mode {
            SvfMode::LowPass => (t, 1.0 / q, 0.0, 0.0, 1.0),
            SvfMode::HighPass => (t, 1.0 / q, 1.0, -1.0 / q, -1.0),
            SvfMode::BandPass => (t, 1.0 / q, 0.0, 1.0, 0.0),
            SvfMode::Notch => (t, 1.0 / q, 1.0, -1.0 / q, 0.0),
            SvfMode::Bell => {
                let a = a();
                let k = 1.0 / (q * a);
                (t, k, 1.0, k * (a * a - 1.0), 0.0)
            }
            SvfMode::LowShelf => {
                let (a, k) = (a(), 1.0 / q);
                (t / a.sqrt(), k, 1.0, k * (a - 1.0), a * a - 1.0)
            }
            SvfMode::HighShelf => {
                let (a, k) = (a(), 1.0 / q);
                (t * a.sqrt(), k, a * a, k * (1.0 - a) * a, 1.0 - a * a)
            }
        };
        SvfCoefs { g, k, m0, m1, m2 }
    }

    /// The bell and shelves matched to their analog curves instead of
    /// bilinear (FX quality E3); every other mode as [`SvfCoefs::bilinear`].
    /// The poles are the analog poles mapped by impulse invariance (`z =
    /// e^(sT)`), so a resonance sits where the analog one does and keeps its
    /// width. A bell's zeros then match its analog magnitude at DC, its gain
    /// at the centre and a flat top there (Vicanek, "Matched Second Order
    /// Digital Filters", 2016); a shelf's zeros are its analog zeros mapped
    /// the same way, scaled to its gain at DC. Worked in `f64`, which the
    /// mapping needs at low frequencies; a few `exp`, `cos` and `sqrt`, so
    /// a gliding EQ designs at a control rate and glides between designs
    /// ([`SvfCoefs::toward`]).
    pub fn matched(mode: SvfMode, hz: f32, q: f32, gain_db: f32, rate: u32) -> SvfCoefs {
        let fs = f64::from(rate);
        let hz = f64::from(hz).clamp(5.0, 0.45 * fs);
        let q = f64::from(q.max(0.05));
        let w0 = std::f64::consts::TAU * hz / fs;
        // The square root of the linear gain, as the bilinear design's `a`.
        let a = 10f64.powf(f64::from(gain_db) / 40.0);
        let (b, d) = match mode {
            SvfMode::Bell => matched_bell(w0, q, a),
            SvfMode::LowShelf | SvfMode::HighShelf => {
                matched_shelf(mode == SvfMode::LowShelf, w0, q, a)
            }
            _ => return SvfCoefs::bilinear(mode, hz as f32, q as f32, gain_db, rate),
        };
        from_biquad(b, d)
    }

    /// One step of a glide from `self` to `to`: `t` of the way (0 to 1).
    /// Between two designs every step is a stable filter (g and k stay
    /// positive), and its curve moves smoothly from one to the other.
    #[inline]
    pub fn toward(self, to: SvfCoefs, t: f32) -> SvfCoefs {
        let l = |x: f32, y: f32| x + t * (y - x);
        SvfCoefs {
            g: l(self.g, to.g),
            k: l(self.k, to.k),
            m0: l(self.m0, to.m0),
            m1: l(self.m1, to.m1),
            m2: l(self.m2, to.m2),
        }
    }
}

/// The prewarped corner `tan(pi hz / rate)` of a bilinear design, the
/// cutoff kept between 5 Hz and `top` of the rate (under a half).
#[inline]
pub fn corner(hz: f32, rate: u32, top: f32) -> f32 {
    let fs = rate as f32;
    let hz = hz.clamp(5.0, top * fs);
    (PI * hz / fs).tan()
}

/// The poles of `s^2 + 2 zeta wp s + wp^2` (`wp` in radians a sample)
/// mapped by impulse invariance: the denominator `1 + d1 z^-1 + d2 z^-2`.
/// A pole pair past Nyquist (`wp` near or above pi) is held at Nyquist's
/// angle, where the digital filter can put it.
fn impulse_poles(wp: f64, zeta: f64) -> (f64, f64) {
    let r = (-zeta * wp).exp();
    let d1 = if zeta < 1.0 {
        -2.0 * r
            * (wp * (1.0 - zeta * zeta).sqrt())
                .min(std::f64::consts::PI)
                .cos()
    } else {
        -2.0 * r * (wp * (zeta * zeta - 1.0).sqrt()).cosh()
    };
    (d1, r * r)
}

/// A bell `(s^2 + (A/q) s + 1) / (s^2 + s/(A q) + 1)` (its peak `A^2`),
/// as a biquad `(b0, b1, b2), (d1, d2)`: impulse-invariant poles, then the
/// zeros from the magnitude at DC (1), at the centre (`A^2`) and a zero
/// slope there (Vicanek 2016, the peaking EQ). The squared magnitudes are
/// written in `p = sin^2(w/2)`: `(B0 (1-p) + B1 p + 4 B2 p (1-p)) / (the
/// same with A0, A1, A2)`.
fn matched_bell(w0: f64, q: f64, a: f64) -> ((f64, f64, f64), (f64, f64)) {
    let g2 = a.powi(4); // the squared magnitude at the centre
    let (d1, d2) = impulse_poles(w0, 1.0 / (2.0 * q * a));
    let a0 = (1.0 + d1 + d2).powi(2);
    let a1 = (1.0 - d1 + d2).powi(2);
    let a2 = -4.0 * d2;
    let p1 = (0.5 * w0).sin().powi(2);
    let p0 = 1.0 - p1;
    let p2 = 4.0 * p0 * p1;
    let r1 = (a0 * p0 + a1 * p1 + a2 * p2) * g2;
    let r2 = (-a0 + a1 + 4.0 * (p0 - p1) * a2) * g2;
    let b0 = a0;
    let b2 = (r1 - r2 * p1 - b0) / (4.0 * p1 * p1);
    let b1 = (r2 + b0 + 4.0 * (p1 - p0) * b2).max(0.0);
    (zeros(b0, b1, b2), (d1, d2))
}

/// A shelf, RBJ's analog form (gain `A^2` below the corner for a low shelf,
/// above it for a high one, half that in dB at the corner), as a biquad:
/// poles and zeros each a pair at `w0` times or over `sqrt(A)` with damping
/// `1/(2q)`, mapped by impulse invariance, and scaled to the analog gain
/// at DC.
fn matched_shelf(low: bool, w0: f64, q: f64, a: f64) -> ((f64, f64, f64), (f64, f64)) {
    let zeta = 1.0 / (2.0 * q);
    let sa = a.sqrt();
    let (wp, wz, dc) = if low {
        (w0 / sa, w0 * sa, a * a)
    } else {
        (w0 * sa, w0 / sa, 1.0)
    };
    let (d1, d2) = impulse_poles(wp, zeta);
    let (n1, n2) = impulse_poles(wz, zeta);
    let scale = dc * (1.0 + d1 + d2) / (1.0 + n1 + n2);
    ((scale, scale * n1, scale * n2), (d1, d2))
}

/// The minimum-phase numerator `(b0, b1, b2)` whose squared magnitude is
/// `B0 (1-p) + B1 p + 4 B2 p (1-p)` (`B0 = (b0+b1+b2)^2`, `B1 =
/// (b0-b1+b2)^2`, `B2 = -4 b0 b2`).
fn zeros(bb0: f64, bb1: f64, bb2: f64) -> (f64, f64, f64) {
    let (s0, s1) = (bb0.sqrt(), bb1.sqrt());
    let w = 0.5 * (s0 + s1);
    let b0 = 0.5 * (w + (w * w + bb2).max(0.0).sqrt());
    let b1 = 0.5 * (s0 - s1);
    let b2 = -bb2 / (4.0 * b0);
    (b0, b1, b2)
}

/// A stable biquad `(b0 + b1 z^-1 + b2 z^-2) / (1 + d1 z^-1 + d2 z^-2)` as
/// the analog prototype the trapezoidal SVF runs: with `u = 1 - d1 + d2`,
/// `g^2 = (1 + d1 + d2) / u`, `k = 2 (1 - d2) / (u g)`; `m0` is the gain at
/// Nyquist, `m0 + m2` at DC, and `m1` the rest.
fn from_biquad((b0, b1, b2): (f64, f64, f64), (d1, d2): (f64, f64)) -> SvfCoefs {
    let u = 1.0 - d1 + d2;
    let g = ((1.0 + d1 + d2) / u).sqrt();
    let k = 2.0 * (1.0 - d2) / (u * g);
    let m0 = (b0 - b1 + b2) / u;
    let m2 = (b0 + b1 + b2) / (1.0 + d1 + d2) - m0;
    let a0 = 4.0 / u;
    let m1 = (a0 * (m0 * d2 - b2) + m2 * g * g) / g;
    SvfCoefs {
        g: g as f32,
        k: k as f32,
        m0: m0 as f32,
        m1: m1 as f32,
        m2: m2 as f32,
    }
}

/// One stereo state-variable filter stage (12 dB per octave).
#[derive(Clone, Copy, Debug)]
pub struct Svf {
    g: f32,
    a1: f32,
    a2: f32,
    a3: f32,
    m0: f32,
    m1: f32,
    m2: f32,
    ic1: [f32; 2],
    ic2: [f32; 2],
}

impl Default for Svf {
    fn default() -> Svf {
        let mut s = Svf {
            g: 0.0,
            a1: 0.0,
            a2: 0.0,
            a3: 0.0,
            m0: 0.0,
            m1: 0.0,
            m2: 1.0,
            ic1: [0.0; 2],
            ic2: [0.0; 2],
        };
        s.set(SvfMode::LowPass, 1000.0, 0.707, 0.0, 48_000);
        s
    }
}

impl Svf {
    /// Sets the response. `q` is the resonance (0.707 is flat for
    /// low-pass), `gain_db` is used by the bell and shelves. The cutoff is
    /// kept between 5 Hz and 45 % of the sample rate.
    #[inline]
    pub fn set(&mut self, mode: SvfMode, hz: f32, q: f32, gain_db: f32, rate: u32) {
        self.set_coefs(SvfCoefs::bilinear(mode, hz, q, gain_db, rate));
    }

    /// Takes a design from [`SvfCoefs`] as it is.
    #[inline]
    pub fn set_coefs(&mut self, c: SvfCoefs) {
        self.g = c.g;
        self.a1 = 1.0 / (1.0 + c.g * (c.g + c.k));
        self.a2 = c.g * self.a1;
        self.a3 = c.g * self.a2;
        self.m0 = c.m0;
        self.m1 = c.m1;
        self.m2 = c.m2;
    }

    /// Filters one sample of channel `ch` (0 or 1).
    #[inline]
    pub fn run(&mut self, ch: usize, v0: f32) -> f32 {
        let (ic1, ic2) = (self.ic1[ch], self.ic2[ch]);
        let v3 = v0 - ic2;
        let v1 = self.a1 * ic1 + self.a2 * v3;
        let v2 = ic2 + self.a2 * ic1 + self.a3 * v3;
        self.ic1[ch] = flush(2.0 * v1 - ic1);
        self.ic2[ch] = flush(2.0 * v2 - ic2);
        self.m0 * v0 + self.m1 * v1 + self.m2 * v2
    }

    /// [`Svf::run`] with the band-pass state bounded: past `knee` it bends
    /// smoothly toward `knee + room`, as an analog filter's resonance loop
    /// saturates. Below the knee it is [`Svf::run`] exactly. The band-pass
    /// state is what a resonance builds up (`Q` times the input at the
    /// cutoff, and the low-pass and high-pass outputs follow it), so
    /// bounding it bounds the peak while the passband is left alone.
    #[inline]
    pub fn run_bounded(&mut self, ch: usize, v0: f32, knee: f32, room: f32) -> f32 {
        let (ic1, ic2) = (self.ic1[ch], self.ic2[ch]);
        let v3 = v0 - ic2;
        let v1 = bend(self.a1 * ic1 + self.a2 * v3, knee, room);
        // The low-pass integrates the (bounded) band-pass: `v2 = ic2 + g v1`,
        // the same as `run`'s `ic2 + a2 ic1 + a3 v3` when nothing bends.
        let v2 = ic2 + self.g * v1;
        self.ic1[ch] = flush(2.0 * v1 - ic1);
        self.ic2[ch] = flush(2.0 * v2 - ic2);
        self.m0 * v0 + self.m1 * v1 + self.m2 * v2
    }

    /// One frame of both channels, linear, for a host that flushes the
    /// states every few frames itself ([`Svf::flush_below`]) rather than on
    /// every sample, both channels sharing the coefficients: the outputs
    /// and the larger band-pass state, which is what a resonance rings up
    /// (`Q` times the input at the cutoff), for a guard on its level
    /// ([`crate::multi::Multi`]). A voice's path, so it is inlined into the
    /// voice's loop; it takes and gives values rather than an array, which
    /// the compiler packs into one integer register and back.
    #[inline(always)]
    pub fn step_stereo_peak(&mut self, l: f32, r: f32) -> (f32, f32, f32) {
        let (yl, bl) = self.step(0, l);
        let (yr, br) = self.step(1, r);
        (yl, yr, bl.abs().max(br.abs()))
    }

    /// [`Svf::step_stereo_peak`] without the band-pass level.
    #[inline(always)]
    pub fn step_stereo(&mut self, l: f32, r: f32) -> (f32, f32) {
        (self.step(0, l).0, self.step(1, r).0)
    }

    /// One sample of channel `ch`, linear, and its band-pass state.
    /// [`Svf::run`]'s filter with the low-pass worked out as
    /// [`Svf::run_bounded`] works it (`v2 = ic2 + g v1`, a multiply fewer:
    /// the same filter, to the rounding), and no flush.
    #[inline(always)]
    fn step(&mut self, ch: usize, x: f32) -> (f32, f32) {
        let (ic1, ic2) = (self.ic1[ch], self.ic2[ch]);
        let v3 = x - ic2;
        let v1 = self.a1 * ic1 + self.a2 * v3;
        let v2 = ic2 + self.g * v1;
        self.ic1[ch] = 2.0 * v1 - ic1;
        self.ic2[ch] = 2.0 * v2 - ic2;
        (self.m0 * x + self.m1 * v1 + self.m2 * v2, v1)
    }

    /// [`Svf::run_bounded`] with a hard ceiling instead of the bend: the
    /// band-pass state clipped at `lim` (its edge rounded), as a diode
    /// clipper in an analog filter's resonance loop.
    #[inline]
    pub fn run_clipped(&mut self, ch: usize, v0: f32, lim: f32) -> f32 {
        let (ic1, ic2) = (self.ic1[ch], self.ic2[ch]);
        let v3 = v0 - ic2;
        let v1 = hard(self.a1 * ic1 + self.a2 * v3, lim);
        let v2 = ic2 + self.g * v1;
        self.ic1[ch] = flush(2.0 * v1 - ic1);
        self.ic2[ch] = flush(2.0 * v2 - ic2);
        self.m0 * v0 + self.m1 * v1 + self.m2 * v2
    }

    /// [`Svf::run`] over a block of channel `ch`, in place, with the state
    /// held in registers: the same numbers as frame by frame.
    #[inline]
    pub fn run_slice(&mut self, ch: usize, x: &mut [f32]) {
        let (mut ic1, mut ic2) = (self.ic1[ch], self.ic2[ch]);
        let (a1, a2, a3) = (self.a1, self.a2, self.a3);
        let (m0, m1, m2) = (self.m0, self.m1, self.m2);
        for v0 in x {
            let v3 = *v0 - ic2;
            let v1 = a1 * ic1 + a2 * v3;
            let v2 = ic2 + a2 * ic1 + a3 * v3;
            ic1 = flush(2.0 * v1 - ic1);
            ic2 = flush(2.0 * v2 - ic2);
            *v0 = m0 * *v0 + m1 * v1 + m2 * v2;
        }
        self.ic1[ch] = ic1;
        self.ic2[ch] = ic2;
    }

    /// [`Svf::run_bounded`] over a block of channel `ch`, in place.
    #[inline]
    pub fn run_bounded_slice(&mut self, ch: usize, x: &mut [f32], knee: f32, room: f32) {
        let (mut ic1, mut ic2) = (self.ic1[ch], self.ic2[ch]);
        let (a1, a2, g) = (self.a1, self.a2, self.g);
        let (m0, m1, m2) = (self.m0, self.m1, self.m2);
        for v0 in x {
            let v3 = *v0 - ic2;
            let v1 = bend(a1 * ic1 + a2 * v3, knee, room);
            let v2 = ic2 + g * v1;
            ic1 = flush(2.0 * v1 - ic1);
            ic2 = flush(2.0 * v2 - ic2);
            *v0 = m0 * *v0 + m1 * v1 + m2 * v2;
        }
        self.ic1[ch] = ic1;
        self.ic2[ch] = ic2;
    }

    #[inline]
    pub fn reset(&mut self) {
        self.ic1 = [0.0; 2];
        self.ic2 = [0.0; 2];
    }

    /// Zeros each channel whose two states are both under `quiet`, for a
    /// host to run every few frames. The flush in every sample works on
    /// each state alone, so at a low cutoff it can zero the band-pass
    /// state on every sample while the low-pass one is still above it
    /// (the band-pass is about `g` times the low-pass there): what is left
    /// then decays at about `2 g²` a sample instead of the filter's own
    /// rate, a tail far below hearing (under 2e-18 at 40 Hz, 1.4e-17 at
    /// 5 Hz) that takes seconds to minutes to reach zero. A `quiet` over
    /// those levels ends it at once.
    #[inline]
    pub fn flush_below(&mut self, quiet: f32) {
        for ch in 0..2 {
            if self.ic1[ch].abs() < quiet && self.ic2[ch].abs() < quiet {
                self.ic1[ch] = 0.0;
                self.ic2[ch] = 0.0;
            }
        }
    }

    /// Its low-pass state, each channel's: what the stage has heard, under
    /// its cutoff.
    #[inline]
    pub fn low_state(&self) -> [f32; 2] {
        self.ic2
    }

    /// Starts as if it had been hearing a sound whose low part is `low` for
    /// a while: its low-pass state there and its band-pass state at rest,
    /// so a stage that comes into a running filter (a steeper slope) takes
    /// up the sound without a dip or a jump.
    #[inline]
    pub fn start_from(&mut self, low: [f32; 2]) {
        self.ic1 = [0.0; 2];
        self.ic2 = low;
    }

    /// Carries on from `from`'s states (its band-pass and low-pass, which
    /// mean nearly the same at any rate or setting).
    #[inline]
    pub fn copy_states(&mut self, from: &Svf) {
        self.ic1 = from.ic1;
        self.ic2 = from.ic2;
    }

    /// The steady-state response at `hz` as a complex gain `(re, im)`, for
    /// drawing curves. Exact: the trapezoidal filter is the bilinear
    /// transform of its analog prototype, so at `hz` the prototype sees
    /// `s = j tan(pi hz / rate) / g`.
    pub fn response(&self, hz: f32, rate: u32) -> (f64, f64) {
        let (a1, a2) = (f64::from(self.a1), f64::from(self.a2));
        let g = a2 / a1;
        let k = (1.0 / a1 - 1.0) / g - g;
        let w = (std::f64::consts::PI * f64::from(hz) / f64::from(rate)).tan() / g;
        // H = m0 + (m2 + j m1 w) / (1 - w^2 + j k w)
        let (dr, di) = (1.0 - w * w, k * w);
        let (nr, ni) = (f64::from(self.m2), f64::from(self.m1) * w);
        let den = dr * dr + di * di;
        (
            f64::from(self.m0) + (nr * dr + ni * di) / den,
            (ni * dr - nr * di) / den,
        )
    }
}

/// `v` unchanged up to `knee` in size, then bending smoothly (no corner:
/// the slope is 1 at the knee and falls away) toward `knee + room`.
#[inline]
pub fn bend(v: f32, knee: f32, room: f32) -> f32 {
    let a = v.abs();
    if a <= knee {
        return v;
    }
    let u = (a - knee) / room;
    (knee + room * u / (1.0 + u)).copysign(v)
}

/// Resonance 0 to 1 as a filter Q: 0 is flat (0.707), 1 is 40 (rings
/// long, close to self-oscillation, but never unstable).
#[inline]
pub fn res_to_q(res: f32) -> f32 {
    0.707 * (40.0f32 / 0.707).powf(res.clamp(0.0, 1.0))
}

/// `v` unchanged up to three quarters of `lim` in size, held at `lim` from
/// five quarters, and a parabola between (its slope from 1 to 0, no
/// corner): a hard clip with its edge rounded, as a diode's.
#[inline]
pub fn hard(v: f32, lim: f32) -> f32 {
    let a = v.abs();
    let lo = lim * (1.0 - HARD_ROUND);
    if a <= lo {
        return v;
    }
    let y = if a >= lim * (1.0 + HARD_ROUND) {
        lim
    } else {
        let e = a - lo;
        a - e * e * (0.25 / (HARD_ROUND * lim))
    };
    y.copysign(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stereo_step_is_the_linear_run() {
        // Without the flush (nothing here is tiny), both channels as
        // run_bounded with no knee runs each (the same form of v2), and the
        // level it gives is the larger band-pass state.
        let mut a = Svf::default();
        a.set(SvfMode::LowPass, 2_000.0, 12.0, 0.0, 44_100);
        let (mut b, mut c) = (a, a);
        for n in 0..2_000 {
            let x = [
                ((n * 37) % 17) as f32 / 8.0 - 1.0,
                ((n * 11) % 13) as f32 / 6.0 - 1.0,
            ];
            let (l, r, peak) = a.step_stereo_peak(x[0], x[1]);
            let z = [
                b.run_bounded(0, x[0], f32::INFINITY, 1.0),
                b.run_bounded(1, x[1], f32::INFINITY, 1.0),
            ];
            assert_eq!(l.to_bits(), z[0].to_bits(), "{n}");
            assert_eq!(r.to_bits(), z[1].to_bits(), "{n}");
            assert_eq!(c.step_stereo(x[0], x[1]), (l, r), "{n}");
            assert!(peak >= 0.0 && peak.is_finite(), "{n}");
        }
    }

    #[test]
    fn the_stereo_peak_is_the_band_pass_state() {
        // A sine on the cutoff: the band-pass state rings to Q times it.
        let mut a = Svf::default();
        a.set(SvfMode::LowPass, 1_000.0, 8.0, 0.0, 44_100);
        let mut most = 0.0f32;
        for n in 0..44_100 {
            let x = 0.01 * (std::f32::consts::TAU * 1_000.0 * n as f32 / 44_100.0).sin();
            let (_, _, peak) = a.step_stereo_peak(x, x);
            if n > 22_050 {
                most = most.max(peak);
            }
        }
        assert!((most / 0.01 - 8.0).abs() < 0.1, "{most}");
    }

    #[test]
    fn a_biquad_maps_onto_the_svf_exactly() {
        // RBJ's bell is the bilinear transform of the same analog bell the
        // SVF's own design is, so mapped across it must give that design.
        let (rate, hz, q, gain_db) = (48_000u32, 3000.0f64, 1.3f64, 7.0f64);
        let a = 10f64.powf(gain_db / 40.0);
        let w0 = std::f64::consts::TAU * hz / f64::from(rate);
        let alpha = w0.sin() / (2.0 * q);
        let a0 = 1.0 + alpha / a;
        let b = (
            (1.0 + alpha * a) / a0,
            -2.0 * w0.cos() / a0,
            (1.0 - alpha * a) / a0,
        );
        let d = (-2.0 * w0.cos() / a0, (1.0 - alpha / a) / a0);
        let mapped = from_biquad(b, d);
        let own = SvfCoefs::bilinear(SvfMode::Bell, hz as f32, q as f32, gain_db as f32, rate);
        for (x, y) in [
            (mapped.g, own.g),
            (mapped.k, own.k),
            (mapped.m0, own.m0),
            (mapped.m1, own.m1),
            (mapped.m2, own.m2),
        ] {
            assert!((x - y).abs() < 1e-4 * (1.0 + y.abs()), "{mapped:?} {own:?}");
        }
    }
}
