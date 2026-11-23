//! The multimode state-variable filter: low-pass, high-pass, band-pass and
//! notch at 12 or 24 dB an octave, Butterworth-shaped with its resonance
//! on the last stage, the resonant stages' band-pass states bending as an
//! analog filter's resonance loop saturates (the Filter effect's Multi
//! model, FX quality round 2). Moved here from `mpfree-fx` bit for bit so
//! the pads run the same filter ([`Multi`], with a per-frame glide).

use crate::svf::{res_to_q, Svf, SvfCoefs, SvfMode};

/// Where the resonance starts to bend, and how much more it can give, in
/// the band-pass state's units (full scale 1), at Resonance 25 % and up.
/// Below 25 % both rise as 1 / (Resonance / 25 %), so a gentle filter never
/// bends a mix-level sound, and at 0 nothing bends at all.
pub const BEND_KNEE: f32 = 0.35;
pub const BEND_ROOM: f32 = 0.3;
pub const BEND_FULL_RES: f32 = 0.25;

/// Sets a multimode filter's one or two stages (the Filter effect's Multi
/// model and the pads' state-variable types).
#[allow(clippy::too_many_arguments)]
pub fn set_stages(
    a: &mut Svf,
    b: &mut Svf,
    mode: SvfMode,
    cascaded: bool,
    hz: f32,
    res: f32,
    gain: f32,
    rate: u32,
) {
    let q = res_to_q(res);
    if cascaded {
        // 24 dB: a Butterworth-shaped first stage, resonance on the second.
        let q1 = if matches!(mode, SvfMode::LowPass | SvfMode::HighPass) {
            0.541
        } else {
            q
        };
        let q2 = if matches!(mode, SvfMode::LowPass | SvfMode::HighPass) {
            1.307 * (q / 0.707)
        } else {
            q
        };
        a.set(mode, hz, q1, gain, rate);
        b.set(mode, hz, q2, gain, rate);
    } else {
        a.set(mode, hz, q, gain, rate);
    }
}

/// One filter's stages as set: the two `Svf`s, whether the second runs,
/// and which of them bend (the resonant ones).
#[derive(Clone, Copy, Debug)]
pub struct Stages {
    pub a: Svf,
    pub b: Svf,
    pub cascaded: bool,
    pub bend_a: bool,
    pub bend_b: bool,
}

impl Stages {
    /// One sample of channel `ch`; with an infinite `knee` (Resonance 0)
    /// every stage is the plain linear one.
    #[inline]
    pub fn run(&mut self, ch: usize, x: f32, knee: f32, room: f32) -> f32 {
        let bends = knee.is_finite();
        let mut x = if self.bend_a && bends {
            self.a.run_bounded(ch, x, knee, room)
        } else {
            self.a.run(ch, x)
        };
        if self.cascaded {
            x = if self.bend_b && bends {
                self.b.run_bounded(ch, x, knee, room)
            } else {
                self.b.run(ch, x)
            };
        }
        x
    }

    /// One sample of channel `ch` with the resonant stages clipped at `lim`
    /// (Hard); with an infinite `lim` (Resonance 0) every stage is linear.
    #[inline]
    pub fn run_clip(&mut self, ch: usize, x: f32, lim: f32) -> f32 {
        let clips = lim.is_finite();
        let mut x = if self.bend_a && clips {
            self.a.run_clipped(ch, x, lim)
        } else {
            self.a.run(ch, x)
        };
        if self.cascaded {
            x = if self.bend_b && clips {
                self.b.run_clipped(ch, x, lim)
            } else {
                self.b.run(ch, x)
            };
        }
        x
    }

    /// Carries on from `from`, the same filter at another rate: a
    /// state-variable filter's states are its band-pass and low-pass, so
    /// they mean nearly the same at any rate. A second stage that `from`
    /// doesn't run starts from silence.
    pub fn take_states(&mut self, from: &Stages) {
        self.a.copy_states(&from.a);
        if from.cascaded {
            self.b.copy_states(&from.b);
        } else {
            self.b.reset();
        }
    }

    /// [`Stages::run`] over a block of channel `ch`, in place, a stage at a
    /// time (each stage hears only the one before it, so the numbers are
    /// the same as frame by frame).
    pub fn run_block(&mut self, ch: usize, x: &mut [f32], knee: f32, room: f32) {
        let bends = knee.is_finite();
        if self.bend_a && bends {
            self.a.run_bounded_slice(ch, x, knee, room);
        } else {
            self.a.run_slice(ch, x);
        }
        if self.cascaded {
            if self.bend_b && bends {
                self.b.run_bounded_slice(ch, x, knee, room);
            } else {
                self.b.run_slice(ch, x);
            }
        }
    }
}

/// The bend's knee and room for Resonance `res` (0 to 1): [`BEND_KNEE`] and
/// [`BEND_ROOM`] from Resonance [`BEND_FULL_RES`] up, rising as one over
/// the Resonance below it, and nothing bending at 0 (an infinite knee).
#[inline]
pub fn bend_for(res: f32) -> (f32, f32) {
    let r = (res / BEND_FULL_RES).min(1.0);
    if r > 0.0 {
        (BEND_KNEE / r, BEND_ROOM / r)
    } else {
        (f32::INFINITY, 1.0)
    }
}

/// Whether a mode's stages bend, as the Filter effect sets them: a
/// low-pass's or high-pass's resonant stage only (the second at 24 dB; the
/// first is a flat Butterworth one), and both of a band-pass's. The notch,
/// bell and shelves don't ring up at their output, so they stay linear.
#[inline]
pub fn bends(mode: SvfMode, cascaded: bool) -> (bool, bool) {
    let resonant = matches!(
        mode,
        SvfMode::LowPass | SvfMode::HighPass | SvfMode::BandPass
    );
    (
        resonant && (!cascaded || mode == SvfMode::BandPass),
        resonant && cascaded,
    )
}

/// The two stages' Q at Resonance `res`, as [`set_stages`] gives them: at
/// 24 dB a low-pass or high-pass is a Butterworth pair (0.541 and 1.307)
/// with the resonance on the second; anything else is `res_to_q(res)`.
#[inline]
pub fn stage_qs(mode: SvfMode, cascaded: bool, res: f32) -> (f32, f32) {
    let q = res_to_q(res);
    if cascaded && matches!(mode, SvfMode::LowPass | SvfMode::HighPass) {
        (0.541, 1.307 * (q / 0.707))
    } else {
        (q, q)
    }
}

/// A [`Multi`]'s curve at `hz` (dB) set to `cutoff_hz` and `res` at
/// `rate`: the stages' exact response (for quiet sounds; a loud one bends
/// the resonance down), for drawing, so the curve shows what is heard up
/// to Nyquist, where the bilinear transform pulls a low-pass down and
/// cramps a high cutoff's peak.
pub fn response_db(
    mode: SvfMode,
    cascaded: bool,
    cutoff_hz: f32,
    res: f32,
    hz: f32,
    rate: u32,
) -> f32 {
    let [c1, c2] = stage_coefs(mode, cascaded, cutoff_hz, res, rate, MULTI_TOP);
    let power = |c: SvfCoefs| {
        let mut s = Svf::default();
        s.set_coefs(c);
        let (re, im) = s.response(hz, rate);
        re * re + im * im
    };
    let mut p = power(c1);
    if cascaded {
        p *= power(c2);
    }
    (10.0 * p.max(1e-24).log10()) as f32
}

/// The designs [`set_stages`] gives the two stages, without setting them,
/// the cutoff kept under `top` of the rate ([`SvfCoefs::bilinear_under`];
/// `set_stages` keeps it under 0.45).
#[inline]
pub fn stage_coefs(
    mode: SvfMode,
    cascaded: bool,
    hz: f32,
    res: f32,
    rate: u32,
    top: f32,
) -> [SvfCoefs; 2] {
    let (q1, q2) = stage_qs(mode, cascaded, res);
    [
        SvfCoefs::bilinear_under(mode, hz, q1, 0.0, rate, top),
        SvfCoefs::bilinear_under(mode, hz, q2, 0.0, rate, top),
    ]
}

/// How high a [`Multi`]'s cutoff goes: 49 % of the rate (21.6 kHz at 44.1
/// kHz), where the Filter effect's stops at 45 %. A pad's filter opened
/// all the way (as a live cutoff brings one in from a pad with none) is
/// then 3 dB down at 21.6 kHz and passes nearly all a sound has, as the
/// pad's filter did before it moved onto these stages.
pub const MULTI_TOP: f32 = 0.49;

/// One voice's multimode filter (the pads'): [`Stages`] whose cutoff and
/// resonance glide to each new setting over the frames until the next
/// one, so a filter envelope or LFO set at a control rate moves the
/// filter smoothly every frame. Stereo, both channels a frame at a time.
#[derive(Clone, Copy, Debug)]
pub struct Multi {
    pub st: Stages,
    /// The designs the glide runs from and to, and its frames: `left` of
    /// `span` still to go (0: landed, nothing to do per frame).
    from: [SvfCoefs; 2],
    to: [SvfCoefs; 2],
    left: u32,
    span: u32,
    /// Where each stage's band-pass state bends (infinite: it doesn't),
    /// and how far past it it can go ([`bend_for`]).
    knee_a: f32,
    knee_b: f32,
    room: f32,
    /// Set at least once (the first setting lands at once).
    set: bool,
}

impl Default for Multi {
    fn default() -> Multi {
        let c = SvfCoefs::bilinear(SvfMode::LowPass, 1000.0, 0.707, 0.0, 48_000);
        Multi {
            st: Stages {
                a: Svf::default(),
                b: Svf::default(),
                cascaded: false,
                bend_a: false,
                bend_b: false,
            },
            from: [c; 2],
            to: [c; 2],
            left: 0,
            span: 0,
            knee_a: f32::INFINITY,
            knee_b: f32::INFINITY,
            room: 1.0,
            set: false,
        }
    }
}

impl Multi {
    /// Heads for a new setting, there in `frames` frames (0 or the first
    /// setting: at once). `res` is 0 to 1 ([`res_to_q`]: Q 0.707 to 40).
    #[inline]
    pub fn set(
        &mut self,
        mode: SvfMode,
        cascaded: bool,
        hz: f32,
        res: f32,
        rate: u32,
        frames: u32,
    ) {
        let to = stage_coefs(mode, cascaded, hz, res, rate, MULTI_TOP);
        let (bend_a, bend_b) = bends(mode, cascaded);
        let jump = !self.set || frames == 0 || cascaded != self.st.cascaded;
        if jump {
            self.from = to;
            self.left = 0;
            self.span = 0;
            self.st.a.set_coefs(to[0]);
            self.st.b.set_coefs(to[1]);
        } else {
            // From where the glide is now.
            self.from = self.now();
            self.left = frames;
            self.span = frames;
        }
        self.to = to;
        self.st.cascaded = cascaded;
        self.st.bend_a = bend_a;
        self.st.bend_b = bend_b;
        let (knee, room) = bend_for(res);
        let off = f32::INFINITY;
        (self.knee_a, self.knee_b) = (
            if bend_a { knee } else { off },
            if bend_b { knee } else { off },
        );
        self.room = room;
        self.set = true;
    }

    /// The designs the glide is at.
    fn now(&self) -> [SvfCoefs; 2] {
        if self.left == 0 {
            return self.to;
        }
        let t = (self.span - self.left) as f32 / self.span as f32;
        [
            self.from[0].toward(self.to[0], t),
            self.from[1].toward(self.to[1], t),
        ]
    }

    /// One frame, both channels: a step of the glide, then the stages
    /// ([`Svf::step_stereo`]; the host flushes every few frames with
    /// [`Multi::flush_below`]). Inlined into the voice's loop.
    #[inline(always)]
    pub fn process(&mut self, l: f32, r: f32) -> (f32, f32) {
        if self.left > 0 {
            self.glide();
        }
        let (mut l, mut r) = self.st.a.step_stereo(l, r, self.knee_a, self.room);
        if self.st.cascaded {
            (l, r) = self.st.b.step_stereo(l, r, self.knee_b, self.room);
        }
        (l, r)
    }

    /// One frame's step of the glide.
    #[inline(always)]
    fn glide(&mut self) {
        self.left -= 1;
        // The last step lands on the setting itself, not a rounding short
        // of it.
        let last = self.left == 0;
        let t = (self.span - self.left) as f32 / self.span as f32;
        let at = |from: SvfCoefs, to: SvfCoefs| if last { to } else { from.toward(to, t) };
        self.st.a.set_coefs(at(self.from[0], self.to[0]));
        if self.st.cascaded {
            self.st.b.set_coefs(at(self.from[1], self.to[1]));
        }
    }

    /// Both stages to silence; the setting stays.
    pub fn reset(&mut self) {
        self.st.a.reset();
        self.st.b.reset();
    }

    /// Zeros a channel of a stage once both its states are under `quiet`
    /// ([`Svf::flush_below`]); for a host to run every few frames.
    #[inline]
    pub fn flush_below(&mut self, quiet: f32) {
        self.st.a.flush_below(quiet);
        if self.st.cascaded {
            self.st.b.flush_below(quiet);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_coefs_are_what_set_stages_sets() {
        for (mode, cascaded) in [
            (SvfMode::LowPass, false),
            (SvfMode::LowPass, true),
            (SvfMode::HighPass, true),
            (SvfMode::BandPass, true),
            (SvfMode::Notch, true),
        ] {
            for res in [0.0, 0.3, 1.0] {
                let (mut a, mut b) = (Svf::default(), Svf::default());
                set_stages(&mut a, &mut b, mode, cascaded, 1234.0, res, 0.0, 44_100);
                let c = stage_coefs(mode, cascaded, 1234.0, res, 44_100, 0.45);
                let (mut x, mut y) = (Svf::default(), Svf::default());
                x.set_coefs(c[0]);
                y.set_coefs(c[1]);
                for hz in [100.0, 1234.0, 9000.0] {
                    assert_eq!(a.response(hz, 44_100), x.response(hz, 44_100));
                    if cascaded {
                        assert_eq!(b.response(hz, 44_100), y.response(hz, 44_100));
                    }
                }
            }
        }
    }

    #[test]
    fn the_drawn_curve_is_the_set_filter() {
        for (mode, cascaded) in [
            (SvfMode::LowPass, false),
            (SvfMode::LowPass, true),
            (SvfMode::HighPass, true),
            (SvfMode::BandPass, false),
            (SvfMode::Notch, true),
        ] {
            let mut m = Multi::default();
            m.set(mode, cascaded, 3_000.0, 0.6, 44_100, 0);
            for hz in [50.0, 2_900.0, 18_000.0] {
                let p = |h: (f64, f64)| h.0 * h.0 + h.1 * h.1;
                let mut heard = p(m.st.a.response(hz, 44_100));
                if cascaded {
                    heard *= p(m.st.b.response(hz, 44_100));
                }
                let drawn = response_db(mode, cascaded, 3_000.0, 0.6, hz, 44_100);
                let heard = (10.0 * heard.max(1e-24).log10()) as f32;
                assert!(
                    (drawn - heard).abs() < 1e-3,
                    "{mode:?} {hz}: {drawn} {heard}"
                );
            }
        }
    }

    #[test]
    fn a_multi_opens_wider_than_the_effect() {
        // Wide open, the voice's filter goes to 49 % of the rate; the
        // effect's stages stop at 45 %.
        let mut m = Multi::default();
        m.set(SvfMode::LowPass, false, 22_000.0, 0.0, 44_100, 0);
        let p = |h: (f64, f64)| 10.0 * (h.0 * h.0 + h.1 * h.1).log10();
        let at_top = p(m.st.a.response(0.49 * 44_100.0, 44_100));
        assert!((at_top + 3.01).abs() < 0.05, "{at_top}");
        let (mut a, mut b) = (Svf::default(), Svf::default());
        set_stages(
            &mut a,
            &mut b,
            SvfMode::LowPass,
            false,
            22_000.0,
            0.0,
            0.0,
            44_100,
        );
        let effect = p(a.response(0.45 * 44_100.0, 44_100));
        assert!((effect + 3.01).abs() < 0.05, "{effect}");
    }

    #[test]
    fn a_glide_lands_on_the_setting_and_moves_smoothly() {
        let mut m = Multi::default();
        m.set(SvfMode::LowPass, true, 500.0, 0.2, 44_100, 0);
        m.set(SvfMode::LowPass, true, 4000.0, 0.2, 44_100, 16);
        let mut last = m.st.a.response(2000.0, 44_100);
        for _ in 0..16 {
            m.process(0.0, 0.0);
            let now = m.st.a.response(2000.0, 44_100);
            // Opening the filter: the gain at 2 kHz only rises.
            let p = |h: (f64, f64)| h.0 * h.0 + h.1 * h.1;
            assert!(p(now) >= p(last) - 1e-12);
            last = now;
        }
        let mut z = Svf::default();
        z.set_coefs(stage_coefs(SvfMode::LowPass, true, 4000.0, 0.2, 44_100, MULTI_TOP)[0]);
        assert_eq!(m.st.a.response(2000.0, 44_100), z.response(2000.0, 44_100));
    }
}
