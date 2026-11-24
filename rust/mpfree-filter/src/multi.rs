//! The multimode state-variable filter: low-pass, high-pass, band-pass and
//! notch at 12 or 24 dB an octave, Butterworth-shaped with its resonance
//! on the last stage, the resonant stages' band-pass states bending as an
//! analog filter's resonance loop saturates (the Filter effect's Multi
//! model, FX quality round 2). Moved here from `mpfree-fx` bit for bit.
//! The pads run the same stages ([`Multi`], with a per-frame glide), but
//! linear, with a guard that damps a loud sound's ring instead of bending
//! it: a bend inside the loop pushes a loud, lopsided wave (a sawtooth)
//! off centre, and the pads hear their sounds at full level.
//!
//! The pads' slopes ([`slope`], [`plan`]; the filter gap plan's F4): a
//! low-pass or high-pass at 6 to 48 dB an octave (1, 2, 3, 4, 6 or 8
//! poles), Butterworth at Resonance 0 so the cutoff is the -3 dB point,
//! with the resonance on the last (highest-Q) stage, so the gain at the
//! cutoff is the 2-pole's at every slope and the passband stays flat; the
//! odd slopes start with a one-pole stage, and the 6 dB takes its
//! resonance as a boost at the cutoff. A band-pass at 12 to 48 dB (2 to 8
//! poles: its skirts 6 to 24 dB an octave a side) is broad stages and one
//! resonant one, so its centre and its ring are the 2-pole's at any slope;
//! a notch at 12 or 24 dB; and a Peak, a boost at the cutoff that
//! Resonance raises.

use crate::db_to_gain;
use crate::onepole::OnePole;
use crate::svf::{corner, res_to_q, Svf, SvfCoefs, SvfMode};

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

/// How high the Filter effect's cutoff goes: 45 % of the rate, as
/// [`SvfCoefs::bilinear`] keeps it.
pub const EFFECT_TOP: f32 = 0.45;

/// Which of a [`SlopeStages`]' stages run: the one-pole, how many plain
/// stages, and whether the last two are both in ([`Stages::cascaded`]).
pub type Layout = (bool, u8, bool);

/// The Filter effect's stages at any slope (the filter gap plan's F4b):
/// an odd slope's one-pole, a steep slope's plain stages, then the last
/// one or two ([`Stages`], the ones that take the resonance), set from the
/// pads' [`plan`], so a slope is the same filter on a pad and in the
/// effect (the effect holds a loud ring back by its Soft bend or Hard clip
/// where the pads have their guard). The 12 and 24 dB filters are the
/// stages [`set_stages`] sets, bit for bit, the 24 dB band-pass's two
/// resonant ones too (its centre rings up twice the 12 dB's in dB, as it
/// always has; the pads' 24 dB band-pass rings as their 12 dB); the bell
/// and shelves are one stage, as there. Stereo, a
/// channel at a time, so a frame or a block runs as [`Stages`] does.
#[derive(Clone, Copy, Debug)]
pub struct SlopeStages {
    pub st: Stages,
    one: OnePole,
    one_on: bool,
    pre: [Svf; 2],
    pre_n: u8,
    /// What the bend's knee and room and the clip's ceiling are scaled by
    /// for the last stages: 1, or for a 6 dB slope's resonance (a boost at
    /// the cutoff) 1 over √2, as the pads' guard holds a bell
    /// ([`GUARD_KNEE_BELL`]): a bell adds its band-pass state to the sound
    /// times up to √2, where a low-pass rings at its output as that state
    /// does.
    hold: f32,
}

impl Default for SlopeStages {
    fn default() -> SlopeStages {
        SlopeStages {
            st: Stages {
                a: Svf::default(),
                b: Svf::default(),
                cascaded: false,
                bend_a: false,
                bend_b: false,
            },
            one: OnePole::default(),
            one_on: false,
            pre: [Svf::default(); 2],
            pre_n: 0,
            hold: 1.0,
        }
    }
}

impl SlopeStages {
    /// Sets `mode` at `poles` ([`slope`]: 1, 2, 3, 4, 6 or 8 for a
    /// low-pass or high-pass), cutoff `hz`, Resonance `res` (0 to 1) and,
    /// for the bell and shelves, `gain` dB, at `rate`. The stages that
    /// run keep their states; one that comes in carries whatever it last
    /// held until [`SlopeStages::clear_new`] clears it.
    #[allow(clippy::too_many_arguments)]
    pub fn set(&mut self, mode: SvfMode, poles: u8, hz: f32, res: f32, gain: f32, rate: u32) {
        let poles = slope(mode, poles);
        // The 12 and 24 dB filters as they always were (and the bell and
        // shelves): [`set_stages`] itself, so a moving cutoff costs what it
        // did. The 24 dB band-pass keeps its two resonant stages, the
        // effect's extra-peaky one (Jay's pick, 24 Nov), where the pads'
        // 24 dB band-pass rings as its 12 dB does.
        let old = match mode {
            SvfMode::Bell | SvfMode::LowShelf | SvfMode::HighShelf => Some(false),
            _ => match poles {
                2 => Some(false),
                4 => Some(true),
                _ => None,
            },
        };
        if let Some(cascaded) = old {
            set_stages(
                &mut self.st.a,
                &mut self.st.b,
                mode,
                cascaded,
                hz,
                res,
                gain,
                rate,
            );
            self.st.cascaded = cascaded;
            (self.st.bend_a, self.st.bend_b) = bends(mode, cascaded);
            self.one_on = false;
            self.pre_n = 0;
            self.hold = 1.0;
            return;
        }
        let p = plan(mode, poles, res);
        let t = corner(hz, rate, EFFECT_TOP);
        self.one_on = p.one.is_some();
        if let Some(hp) = p.one {
            self.one.set(OnePole::gain_for(t), hp);
        }
        let s = p.stages();
        let (pre, last) = s.split_at(s.len().saturating_sub(2));
        for (svf, stage) in self.pre.iter_mut().zip(pre) {
            svf.set_coefs(stage.coefs(t));
        }
        self.pre_n = pre.len() as u8;
        self.st.a.set_coefs(last[0].coefs(t));
        self.st.bend_a = last[0].resonant;
        self.hold = if last[0].mode == SvfMode::Bell {
            std::f32::consts::FRAC_1_SQRT_2
        } else {
            1.0
        };
        self.st.cascaded = last.len() == 2;
        if let Some(b) = last.get(1) {
            self.st.b.set_coefs(b.coefs(t));
            self.st.bend_b = b.resonant;
        } else {
            self.st.bend_b = false;
        }
    }

    /// Whether only the last one or two stages run (no one-pole or plain
    /// stages before them), so a host can run [`SlopeStages::st`] alone, at
    /// the knee or ceiling times [`SlopeStages::hold`].
    #[inline]
    pub fn last_only(&self) -> bool {
        !self.one_on && self.pre_n == 0
    }

    /// What the bend's knee and room and the clip's ceiling are scaled by
    /// for the last stages (1, or 1 over √2 for a 6 dB slope's boost).
    #[inline]
    pub fn hold(&self) -> f32 {
        self.hold
    }

    /// Which stages run now.
    #[inline]
    pub fn layout(&self) -> Layout {
        (self.one_on, self.pre_n, self.st.cascaded)
    }

    /// Starts each stage that runs now but didn't in `was` (a slope or
    /// type change): a one-pole or plain stage that comes in at the front
    /// takes up the low part of the sound the filter was hearing (the
    /// first stage's low-pass state, [`Svf::start_from`]), so the stages
    /// after it hear no dip; a second resonant stage starts from silence.
    pub fn clear_new(&mut self, was: Layout) {
        let low = if was.0 {
            self.one.low_state()
        } else if was.1 > 0 {
            self.pre[0].low_state()
        } else {
            self.st.a.low_state()
        };
        if self.one_on && !was.0 {
            self.one.start_from(low);
        }
        for s in self
            .pre
            .iter_mut()
            .take(usize::from(self.pre_n))
            .skip(usize::from(was.1))
        {
            s.start_from(low);
        }
        if self.st.cascaded && !was.2 {
            self.st.b.reset();
        }
    }

    /// The one-pole and plain stages for one sample of channel `ch`.
    #[inline(always)]
    fn front(&mut self, ch: usize, mut x: f32) -> f32 {
        if self.one_on {
            x = self.one.run(ch, x);
        }
        for s in &mut self.pre[..usize::from(self.pre_n)] {
            x = s.run(ch, x);
        }
        x
    }

    /// One sample of channel `ch`, the resonant stages bending past
    /// `knee` ([`Stages::run`]).
    #[inline]
    pub fn run(&mut self, ch: usize, x: f32, knee: f32, room: f32) -> f32 {
        let x = self.front(ch, x);
        let h = self.hold;
        self.st.run(ch, x, knee * h, room * h)
    }

    /// One sample of channel `ch`, the resonant stages clipped at `lim`
    /// ([`Stages::run_clip`]).
    #[inline]
    pub fn run_clip(&mut self, ch: usize, x: f32, lim: f32) -> f32 {
        let x = self.front(ch, x);
        self.st.run_clip(ch, x, lim * self.hold)
    }

    /// [`SlopeStages::run`] over a block of channel `ch`, in place, a stage
    /// at a time: the same numbers as frame by frame.
    pub fn run_block(&mut self, ch: usize, x: &mut [f32], knee: f32, room: f32) {
        if self.one_on {
            for v in x.iter_mut() {
                *v = self.one.run(ch, *v);
            }
        }
        for s in &mut self.pre[..usize::from(self.pre_n)] {
            s.run_slice(ch, x);
        }
        let h = self.hold;
        self.st.run_block(ch, x, knee * h, room * h);
    }

    /// Carries on from `from`, the same filter at another rate
    /// ([`Stages::take_states`]); a stage `from` doesn't run starts from
    /// silence.
    pub fn take_states(&mut self, from: &SlopeStages) {
        self.st.take_states(&from.st);
        if from.one_on {
            self.one.copy_states(&from.one);
        } else {
            self.one.reset();
        }
        for (i, s) in self.pre.iter_mut().enumerate() {
            if i < usize::from(from.pre_n) {
                s.copy_states(&from.pre[i]);
            } else {
                s.reset();
            }
        }
    }

    /// Every stage from silence.
    pub fn reset(&mut self) {
        self.one.reset();
        for s in &mut self.pre {
            s.reset();
        }
        self.st.a.reset();
        self.st.b.reset();
    }

    /// The steady-state response at `hz` as a complex gain `(re, im)`: the
    /// stages that run, exactly.
    pub fn response(&self, hz: f32, rate: u32) -> (f64, f64) {
        let mul = |a: (f64, f64), b: (f64, f64)| (a.0 * b.0 - a.1 * b.1, a.0 * b.1 + a.1 * b.0);
        let mut h = self.st.a.response(hz, rate);
        if self.st.cascaded {
            h = mul(h, self.st.b.response(hz, rate));
        }
        for s in &self.pre[..usize::from(self.pre_n)] {
            h = mul(h, s.response(hz, rate));
        }
        if self.one_on {
            h = mul(h, self.one.response(hz, rate));
        }
        h
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

/// The slope `mode` plays for `poles` (6 dB an octave each): a low-pass
/// or high-pass 1, 2, 3, 4, 6 or 8; a band-pass 2, 4, 6 or 8 (its skirts
/// 6 dB an octave a side for every two); a notch 2 or 4; anything else 2.
/// A count between goes to the one below it, and 0 (no setting) is 2.
pub fn slope(mode: SvfMode, poles: u8) -> u8 {
    let p = if poles == 0 { 2 } else { poles };
    match mode {
        SvfMode::LowPass | SvfMode::HighPass => match p {
            1..=4 => p,
            5 => 4,
            6 | 7 => 6,
            _ => 8,
        },
        SvfMode::BandPass => match p {
            1..=3 => 2,
            4 | 5 => 4,
            6 | 7 => 6,
            _ => 8,
        },
        SvfMode::Notch if p >= 4 => 4,
        _ => 2,
    }
}

/// A Butterworth filter's 2-pole stages' Qs, lowest first (`1 / (2 cos
/// a)` for its poles' angles `a` from the real axis); an odd order adds a
/// real pole, the one-pole stage. The 4-pole's are rounded as the 24 dB
/// filter has always had them.
const BUTTERWORTH_3: [f32; 1] = [1.0];
const BUTTERWORTH_4: [f32; 2] = [0.541, 1.307];
const BUTTERWORTH_6: [f32; 3] = [0.517_638_1, std::f32::consts::FRAC_1_SQRT_2, 1.931_851_6];
const BUTTERWORTH_8: [f32; 4] = [0.509_795_6, 0.601_344_9, 0.899_976_2, 2.562_915_5];

/// One state-variable stage of a slope ([`plan`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stage {
    pub mode: SvfMode,
    /// The Q its design is given ([`SvfCoefs::bilinear_at`]).
    pub q: f32,
    /// A bell's gain at its centre (dB).
    pub gain_db: f32,
    /// The scale on its output: a band-pass's broad stages are 1 at the
    /// centre.
    pub scale: f32,
    /// It takes the resonance, so a loud sound rings it up: the resonance
    /// guard watches it ([`Multi`]).
    pub resonant: bool,
}

impl Stage {
    fn new(mode: SvfMode, q: f32) -> Stage {
        Stage {
            mode,
            q,
            gain_db: 0.0,
            scale: 1.0,
            resonant: false,
        }
    }

    fn resonant(mode: SvfMode, q: f32) -> Stage {
        Stage {
            resonant: true,
            ..Stage::new(mode, q)
        }
    }

    /// A boost at the cutoff for a resonance of Q `q` (the 6 dB slope's,
    /// and the Peak): `q / 0.707` there, 0 dB at Resonance 0 up to 35 dB,
    /// its poles' Q `q`, so it rings as a 2-pole filter at that Resonance
    /// does.
    fn boost(q: f32) -> Stage {
        let gain = q / 0.707;
        Stage {
            mode: SvfMode::Bell,
            q: q / gain.sqrt(),
            gain_db: 20.0 * gain.log10(),
            scale: 1.0,
            resonant: true,
        }
    }

    /// Its design at the prewarped corner `t` ([`corner`]).
    #[inline]
    pub fn coefs(&self, t: f32) -> SvfCoefs {
        let c = SvfCoefs::bilinear_at(self.mode, t, self.q, self.gain_db);
        if self.scale == 1.0 {
            return c;
        }
        SvfCoefs {
            m0: c.m0 * self.scale,
            m1: c.m1 * self.scale,
            m2: c.m2 * self.scale,
            ..c
        }
    }

    /// The Q of its poles, what a ring at the cutoff builds up to in its
    /// band-pass state: a bell's design Q times the square root of its
    /// gain, any other stage's Q.
    #[inline]
    pub fn pole_q(&self) -> f32 {
        if self.mode == SvfMode::Bell {
            self.q * db_to_gain(self.gain_db * 0.5)
        } else {
            self.q
        }
    }
}

/// A slope's stages at a Resonance ([`plan`]): a one-pole first for an odd
/// slope, then up to four state-variable stages, the resonant one last.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plan {
    /// The one-pole stage (6 and 18 dB): `Some(true)` for a high-pass's.
    pub one: Option<bool>,
    pub stages: [Stage; 4],
    pub n: usize,
    /// The resonant stage's pole Q at Resonance 0 (Butterworth's, or a
    /// boost's 0.707): the most damping the guard gives takes it there.
    pub q0: f32,
}

impl Plan {
    /// The state-variable stages, in the order they run.
    pub fn stages(&self) -> &[Stage] {
        &self.stages[..self.n]
    }

    fn push(&mut self, s: Stage) {
        self.stages[self.n] = s;
        self.n += 1;
    }
}

/// The stages of `mode` at `poles` ([`slope`]) and Resonance `res` (0 to
/// 1, [`res_to_q`]'s Q 0.707 to 40). See the module notes; the 12 and 24
/// dB low-pass, high-pass and notch are the Filter effect's
/// ([`set_stages`]).
pub fn plan(mode: SvfMode, poles: u8, res: f32) -> Plan {
    plan_q(mode, poles, res_to_q(res))
}

/// [`plan`] at a Q of `q` (Resonance 0's is 0.707).
fn plan_q(mode: SvfMode, poles: u8, q: f32) -> Plan {
    let mut p = Plan {
        one: None,
        stages: [Stage::new(mode, 0.707); 4],
        n: 0,
        q0: 0.707,
    };
    let poles = slope(mode, poles);
    match mode {
        SvfMode::LowPass | SvfMode::HighPass => {
            // The last stage's Q times the 2-pole's over Butterworth's:
            // at the cutoff the gain is the 2-pole's, `q`.
            let lift = q / 0.707;
            let hp = mode == SvfMode::HighPass;
            match poles {
                1 => {
                    p.one = Some(hp);
                    p.push(Stage::boost(q));
                }
                2 => p.push(Stage::resonant(mode, q)),
                3 => {
                    p.one = Some(hp);
                    p.q0 = BUTTERWORTH_3[0];
                    p.push(Stage::resonant(mode, BUTTERWORTH_3[0] * lift));
                }
                _ => {
                    let b: &[f32] = match poles {
                        4 => &BUTTERWORTH_4,
                        6 => &BUTTERWORTH_6,
                        _ => &BUTTERWORTH_8,
                    };
                    let (last, rest) = b.split_last().unwrap_or((&0.707, &[]));
                    for &bq in rest {
                        p.push(Stage::new(mode, bq));
                    }
                    p.q0 = *last;
                    p.push(Stage::resonant(mode, last * lift));
                }
            }
        }
        SvfMode::BandPass => {
            // Broad stages, 1 at the centre, steepen the skirts; the last
            // takes the resonance, so the centre is the 2-pole's.
            let broad = Stage {
                scale: 1.0 / 0.707,
                ..Stage::new(mode, 0.707)
            };
            for _ in 1..poles / 2 {
                p.push(broad);
            }
            p.push(Stage::resonant(mode, q));
        }
        SvfMode::Notch => {
            p.push(Stage::new(mode, q));
            if poles >= 4 {
                p.push(Stage::new(mode, q));
            }
        }
        SvfMode::Bell => p.push(Stage::boost(q)),
        SvfMode::LowShelf | SvfMode::HighShelf => p.push(Stage::new(mode, q)),
    }
    p
}

/// A [`Multi`]'s curve at `hz` (dB) set to `poles`, `cutoff_hz` and `res`
/// at `rate`: the stages' exact response (for any sound short of one loud
/// enough for the guard to ease the resonance), for drawing, so the curve
/// shows what is heard up to Nyquist, where the bilinear transform pulls a
/// low-pass down and cramps a high cutoff's peak.
pub fn response_db(mode: SvfMode, poles: u8, cutoff_hz: f32, res: f32, hz: f32, rate: u32) -> f32 {
    let plan = plan(mode, poles, res);
    let t = corner(cutoff_hz, rate, MULTI_TOP);
    let power = |h: (f64, f64)| h.0 * h.0 + h.1 * h.1;
    let mut p = 1.0f64;
    if let Some(hp) = plan.one {
        let mut o = OnePole::default();
        o.set(OnePole::gain_for(t), hp);
        p *= power(o.response(hz, rate));
    }
    for st in plan.stages() {
        let mut s = Svf::default();
        s.set_coefs(st.coefs(t));
        p *= power(s.response(hz, rate));
    }
    (10.0 * p.max(1e-24).log10()) as f32
}

/// How high a [`Multi`]'s cutoff goes: 49 % of the rate (21.6 kHz at 44.1
/// kHz), where the Filter effect's stops at 45 %. A pad's filter opened
/// all the way (as a live cutoff brings one in from a pad with none) is
/// then 3 dB down at 21.6 kHz and passes nearly all a sound has, as the
/// pad's filter did before it moved onto these stages.
pub const MULTI_TOP: f32 = 0.49;

/// The resonance guard (the pads' [`Multi`]): the ring a resonant stage's
/// band-pass state may reach before the guard damps it (full scale 1, so
/// 8 dB under it; the pad's level and velocity come after the filter). A
/// sound whose ring stays under it hears the filter as set: at full
/// Resonance (Q 40) that is any sound under about -40 dBFS at the cutoff.
pub const GUARD_KNEE: f32 = 0.4;

/// The guard's knee for a boost (the Peak, and a 1-pole's resonance): a
/// bell adds its band-pass state to the sound times `m1`, which is up to
/// √2 (`1/0.707 - 1/q` for a pole Q `q`), where a low-pass, high-pass or
/// band-pass rings at its output as its band-pass state does. So a bell's
/// state is held √2 lower, and what its ring adds at the output meets the
/// same knee.
pub const GUARD_KNEE_BELL: f32 = GUARD_KNEE * std::f32::consts::FRAC_1_SQRT_2;

/// How hard the guard holds a ring past [`GUARD_KNEE`]: the stage's
/// damping goes as the ring over the knee to this power, so a ring settles
/// where `ring^(p + 1) = drive * Q * knee^p`: every 10 dB more drive rings
/// 2 dB louder, and a full-scale sine on the cutoff at full Resonance
/// rings at about full scale (32 dB over it, linear).
pub const GUARD_POWER: i32 = 4;

/// The window the guard holds a ring's peak over: [`GUARD_BLOCKS`] blocks
/// of [`GUARD_BLOCK`] frames (46 to 58 ms at 44.1 kHz, longer than a cycle
/// of any note over 22 Hz), so that the damping holds still across a held
/// note's cycles. Damping that moves inside a cycle pushes a lopsided wave
/// off centre, as the Filter effect's bend does.
pub const GUARD_BLOCK: u32 = 512;
pub const GUARD_BLOCKS: usize = 4;

/// How fast the guard lets go once a loud peak has left its window: 6 dB
/// in this time.
pub const GUARD_FALL_S: f32 = 0.05;

/// How far over a ring's window peak the held peak may sit and stay: a
/// ring within it of the held peak holds the damping still, so that the
/// guard settles instead of hunting (damping a little, then letting go a
/// little, as the ring follows each change a moment later).
pub const GUARD_SLACK: f32 = 1.03;

// A rise takes the fourth root of the ring's overshoot (two square roots).
const _: () = assert!(GUARD_POWER == 4);

/// The guard's damping for a held peak of `held` on a stage whose knee is
/// `1 / over`: 1 up to the knee, then the peak over it to
/// [`GUARD_POWER`].
#[inline]
fn damping_for(held: f32, over: f32) -> f32 {
    let r = held * over;
    if r > 1.0 {
        r.powi(GUARD_POWER)
    } else {
        1.0
    }
}

/// One voice's multimode filter (the pads'): a slope's stages ([`plan`])
/// whose cutoff and resonance glide to each new setting over the frames
/// until the next one, so a filter envelope or LFO set at a control rate
/// moves the filter smoothly every frame. Stereo, both channels a frame at
/// a time. The last one or two stages are [`Stages`] (the 12 and 24 dB
/// filter's); a steeper slope's plain stages and an odd one's one-pole run
/// before them.
///
/// Its stages are linear, so it adds no harmonics and no DC offset; a
/// guard holds a loud sound's ring back instead of bending it. The
/// resonant stage (the last, and a notch has none) keeps the peak of its
/// band-pass state over a window of about 50 ms, and once that peak is
/// past [`GUARD_KNEE`] (a bell's [`GUARD_KNEE_BELL`]) the stage takes
/// more damping (its `k` times the peak over the knee to [`GUARD_POWER`],
/// at most Resonance 0's damping).
/// A peak higher than the one held is damped in the frame it comes, so a
/// loud note's first ring is held as it builds; the damping holds still
/// while the window holds the peak, and lets go 6 dB every
/// [`GUARD_FALL_S`] once it has gone. The Filter effect's bend inside the
/// loop holds the peak too, but on a loud lopsided wave it pushes the
/// output off centre (a full-scale sawtooth through a 2 kHz low-pass at
/// Resonance 30 %: -0.12, about -18 dB), which a pad, hearing its sound at
/// full level, can't have.
#[derive(Clone, Copy, Debug)]
pub struct Multi {
    pub st: Stages,
    /// A steeper slope's plain stages before the last two (36 and 48 dB,
    /// and a band-pass's broad ones), `pre_n` of them, and an odd slope's
    /// one-pole before those; `extra` if either runs (out of line,
    /// [`Multi::process_extra`]).
    extra: bool,
    pre: [Svf; 2],
    pre_n: u8,
    one: OnePole,
    one_on: bool,
    /// The setting's designs the glide runs from and to (undamped), and
    /// its frames: `left` of `span` still to go (0: landed, nothing to do
    /// per frame). The last two stages', the plain stages' and the
    /// one-pole's.
    from: [SvfCoefs; 2],
    to: [SvfCoefs; 2],
    pre_from: [SvfCoefs; 2],
    pre_to: [SvfCoefs; 2],
    one_from: f32,
    one_to: f32,
    left: u32,
    span: u32,
    /// The slope set ([`slope`]): a new one lands at once.
    poles: u8,
    /// Each of the last two stages is a high-pass, whose `m1` moves with
    /// its damping.
    hp: [bool; 2],
    /// Each stage's damping, its `k` times this (1: as set), and the most
    /// it may take (the stage's Butterworth damping over its own, so a held
    /// ring never dulls the filter past Resonance 0's).
    damp: [f32; 2],
    most: [f32; 2],
    /// Each guarded stage's held peak (at least the knee; infinite for a
    /// stage the guard leaves alone), its peak since the window's block
    /// began, and the window's blocks.
    held: [f32; 2],
    /// Each guarded stage's knee ([`GUARD_KNEE`], or [`GUARD_KNEE_BELL`]
    /// for a bell) and its reciprocal.
    knee: [f32; 2],
    over: [f32; 2],
    cur: [f32; 2],
    win: [[f32; GUARD_BLOCKS]; 2],
    /// Frames into the window's block, and the block written next.
    frames: u32,
    wi: usize,
    /// Both stages are guarded.
    both: bool,
    /// The held peak's fall over `fall_frames` frames at `fall_rate`.
    fall: f32,
    fall_frames: u32,
    fall_rate: u32,
    rate: u32,
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
            extra: false,
            pre: [Svf::default(); 2],
            pre_n: 0,
            one: OnePole::default(),
            one_on: false,
            from: [c; 2],
            to: [c; 2],
            pre_from: [c; 2],
            pre_to: [c; 2],
            one_from: 0.0,
            one_to: 0.0,
            left: 0,
            span: 0,
            poles: 2,
            hp: [false; 2],
            damp: [1.0; 2],
            most: [1.0; 2],
            held: [f32::INFINITY; 2],
            knee: [GUARD_KNEE; 2],
            over: [1.0 / GUARD_KNEE; 2],
            cur: [0.0; 2],
            win: [[0.0; GUARD_BLOCKS]; 2],
            frames: 0,
            wi: 0,
            both: false,
            fall: 1.0,
            fall_frames: 0,
            fall_rate: 0,
            rate: 48_000,
            set: false,
        }
    }
}

impl Multi {
    /// Heads for a new setting, there in `frames` frames (0, the first
    /// setting or a new slope: at once). `poles` is the slope ([`slope`]),
    /// `res` 0 to 1 ([`res_to_q`]: Q 0.707 to 40). A guard holding a ring
    /// back goes on holding it, within what the new setting may take.
    #[inline]
    pub fn set(&mut self, mode: SvfMode, poles: u8, hz: f32, res: f32, rate: u32, frames: u32) {
        let poles = slope(mode, poles);
        let jump = !self.set || frames == 0 || poles != self.poles;
        let plan = plan(mode, poles, res);
        let t = corner(hz, rate, MULTI_TOP);
        let n = plan.n;
        let cascaded = n >= 2;
        // The last two stages are `a` and `b` (one alone is `a`).
        let (ia, ib) = if cascaded { (n - 2, n - 1) } else { (0, 0) };
        let ca = plan.stages[ia].coefs(t);
        let to = [
            ca,
            if cascaded {
                plan.stages[ib].coefs(t)
            } else {
                ca
            },
        ];
        let pre_n = n.saturating_sub(2);
        let guarded = [
            plan.stages[ia].resonant,
            cascaded && plan.stages[ib].resonant,
        ];
        for (i, si) in [ia, ib].into_iter().enumerate() {
            if guarded[i] {
                let knee = if plan.stages[si].mode == SvfMode::Bell {
                    GUARD_KNEE_BELL
                } else {
                    GUARD_KNEE
                };
                if !self.held[i].is_finite() {
                    self.held[i] = knee;
                    self.cur[i] = 0.0;
                    self.win[i] = [0.0; GUARD_BLOCKS];
                } else if knee != self.knee[i] {
                    // A type change to or from a boost: the held peak
                    // moves with the knee, so the damping holds.
                    self.held[i] *= knee / self.knee[i];
                }
                self.knee[i] = knee;
                self.over[i] = 1.0 / knee;
                // Resonance 0's damping, the most the guard may give (the
                // resonant stage is the last).
                self.most[i] = (1.0 / (plan.q0 * to[i].k)).max(1.0);
                self.damp[i] = damping_for(self.held[i], self.over[i]).min(self.most[i]);
            } else {
                self.held[i] = f32::INFINITY;
                self.damp[i] = 1.0;
                self.most[i] = 1.0;
            }
        }
        self.both = guarded[0] && guarded[1];
        self.hp = [
            plan.stages[ia].mode == SvfMode::HighPass,
            plan.stages[ib].mode == SvfMode::HighPass,
        ];
        if self.set && poles != self.poles {
            // A stage the new slope brings in starts from silence.
            if !self.st.cascaded {
                self.st.b.reset();
            }
            for s in &mut self.pre[self.pre_n as usize..] {
                s.reset();
            }
            if !self.one_on {
                self.one.reset();
            }
        }
        self.rate = rate;
        self.st.cascaded = cascaded;
        self.poles = poles;
        self.pre_n = pre_n as u8;
        self.one_on = plan.one.is_some();
        self.extra = self.one_on || pre_n > 0;
        if self.extra {
            self.set_extra(&plan, t, jump);
        }
        self.aim(to, frames, jump);
        self.set = true;
    }

    /// The stages before the last two heading for `plan`'s at the corner
    /// `t`: a steeper slope's plain stages and an odd slope's one-pole, at
    /// once on a `jump`, else from where their glide is. Before
    /// [`Multi::aim`], which starts the glide.
    #[inline(never)]
    fn set_extra(&mut self, plan: &Plan, t: f32, jump: bool) {
        let mut pre_to = self.pre_to;
        for (i, c) in pre_to.iter_mut().enumerate().take(self.pre_n as usize) {
            *c = plan.stages[i].coefs(t);
        }
        let one_to = OnePole::gain_for(t);
        if let Some(hp) = plan.one {
            self.one.set(self.one.gain(), hp);
        }
        (self.pre_from, self.one_from) = if jump {
            (pre_to, one_to)
        } else {
            self.extra_now()
        };
        (self.pre_to, self.one_to) = (pre_to, one_to);
    }

    /// Stage `i`'s design `c` with the guard's damping: its `k` (and a
    /// high-pass's mix of it) times the stage's damping.
    #[inline(always)]
    fn damped(&self, c: SvfCoefs, i: usize) -> SvfCoefs {
        let f = self.damp[i];
        if f == 1.0 {
            return c;
        }
        SvfCoefs {
            k: c.k * f,
            m1: if self.hp[i] { c.m1 * f } else { c.m1 },
            ..c
        }
    }

    /// Glides the last two stages to `to` over `frames` frames (the stages
    /// before them with them, [`Multi::set_extra`]), or lands there at
    /// once.
    #[inline]
    fn aim(&mut self, to: [SvfCoefs; 2], frames: u32, jump: bool) {
        if jump || frames == 0 {
            self.from = to;
            self.left = 0;
            self.span = 0;
            self.to = to;
            self.land();
        } else {
            // From where the glide is now.
            self.from = self.now();
            self.left = frames;
            self.span = frames;
            self.to = to;
        }
    }

    /// The stages set to the setting itself, damped.
    #[inline]
    fn land(&mut self) {
        let (a, b) = (self.damped(self.to[0], 0), self.damped(self.to[1], 1));
        self.st.a.set_coefs(a);
        self.st.b.set_coefs(b);
        if self.extra {
            for i in 0..self.pre_n as usize {
                self.pre[i].set_coefs(self.pre_to[i]);
            }
            self.one.set_gain(self.one_to);
        }
    }

    /// The guard's window, for the host to run every `frames` frames (the
    /// pads' control step), after the frames and before any new setting:
    /// each guarded stage's peak since the block began goes into the
    /// window when a block ends, the held peak falls to the window's (6 dB
    /// in [`GUARD_FALL_S`], never under the knee), and the damping eases
    /// with it. A peak over the held one is damped in its own frame
    /// ([`Multi::process`]). Quiet, it costs a few compares.
    #[inline]
    pub fn guard(&mut self, frames: u32) {
        if !self.set || !(self.held[0].is_finite() || self.held[1].is_finite()) {
            return;
        }
        if self.fall_frames != frames || self.fall_rate != self.rate {
            let s = frames as f32 / (GUARD_FALL_S * self.rate as f32);
            self.fall = 0.5f32.powf(s);
            (self.fall_frames, self.fall_rate) = (frames, self.rate);
        }
        self.frames += frames;
        let shift = self.frames >= GUARD_BLOCK;
        let mut eased = false;
        for i in 0..2 {
            if !self.held[i].is_finite() {
                continue;
            }
            if shift {
                self.win[i][self.wi] = self.cur[i];
                self.cur[i] = 0.0;
            }
            let window = self.win[i].iter().fold(self.cur[i], |m, &v| m.max(v));
            let floor = (window * GUARD_SLACK).max(self.knee[i]);
            if floor < self.held[i] {
                let held = floor.max(self.held[i] * self.fall);
                self.held[i] = held;
                let f = damping_for(held, self.over[i]).min(self.most[i]);
                if f != self.damp[i] {
                    self.damp[i] = f;
                    eased = true;
                }
            }
        }
        if shift {
            self.frames -= GUARD_BLOCK;
            self.wi = (self.wi + 1) % GUARD_BLOCKS;
        }
        if eased && self.left == 0 {
            self.land();
        }
    }

    /// Where the glide is in the glide's frames: `t` of the way (0 to 1).
    #[inline(always)]
    fn glide_t(&self) -> f32 {
        (self.span - self.left) as f32 / self.span as f32
    }

    /// The designs the glide is at (undamped).
    fn now(&self) -> [SvfCoefs; 2] {
        if self.left == 0 {
            return self.to;
        }
        let t = self.glide_t();
        [
            self.from[0].toward(self.to[0], t),
            self.from[1].toward(self.to[1], t),
        ]
    }

    /// The plain stages' and the one-pole's designs the glide is at.
    fn extra_now(&self) -> ([SvfCoefs; 2], f32) {
        if self.left == 0 {
            return (self.pre_to, self.one_to);
        }
        let t = self.glide_t();
        (
            [
                self.pre_from[0].toward(self.pre_to[0], t),
                self.pre_from[1].toward(self.pre_to[1], t),
            ],
            self.one_from + t * (self.one_to - self.one_from),
        )
    }

    /// One frame, both channels: a step of the glide, then the stages
    /// ([`Svf::step_stereo`]; the host flushes every few frames with
    /// [`Multi::flush_below`]), each guarded stage's band-pass state
    /// watched. Inlined into the voice's loop; a slope with stages before
    /// the last two runs out of line ([`Multi::process_extra`]).
    #[inline(always)]
    pub fn process(&mut self, l: f32, r: f32) -> (f32, f32) {
        if self.extra {
            return self.process_extra(l, r);
        }
        if self.left > 0 {
            self.glide();
        }
        self.run_last(l, r)
    }

    /// [`Multi::process`] for a slope with stages before the last two (6,
    /// 18, 36 and 48 dB, and a band-pass's broad ones): its glide step,
    /// its one-pole, its plain stages, then the last two. Out of line and
    /// marked cold, so the 12 and 24 dB filters' code in a voice's loop
    /// stays lean: with these stages inlined too, 16 held pads with a
    /// moving 12 dB cutoff cost 6 % more than before them, the loop's
    /// values spilling to memory; out of line, 4.5 % (the call's spills
    /// leaking into the frames that don't take it); cold, 2.5 % (the one
    /// test a frame and its join).
    #[cold]
    #[inline(never)]
    fn process_extra(&mut self, l: f32, r: f32) -> (f32, f32) {
        if self.left > 0 {
            self.glide();
            self.glide_extra();
        }
        let (mut l, mut r) = if self.one_on {
            self.one.step_stereo(l, r)
        } else {
            (l, r)
        };
        for s in &mut self.pre[..self.pre_n as usize] {
            (l, r) = s.step_stereo(l, r);
        }
        self.run_last(l, r)
    }

    /// The last one or two stages, each guarded stage's band-pass state
    /// watched.
    #[inline(always)]
    fn run_last(&mut self, l: f32, r: f32) -> (f32, f32) {
        if self.st.cascaded {
            let (l, r) = if self.both {
                let (l, r, p) = self.st.a.step_stereo_peak(l, r);
                self.watch(0, p);
                (l, r)
            } else {
                self.st.a.step_stereo(l, r)
            };
            let (l, r, p) = self.st.b.step_stereo_peak(l, r);
            self.watch(1, p);
            (l, r)
        } else {
            let (l, r, p) = self.st.a.step_stereo_peak(l, r);
            self.watch(0, p);
            (l, r)
        }
    }

    /// Stage `i`'s band-pass peak this frame: kept for the window, and
    /// damped at once if it is over the held one.
    #[inline(always)]
    fn watch(&mut self, i: usize, p: f32) {
        self.cur[i] = self.cur[i].max(p);
        if p > self.held[i] {
            self.rise(i, p);
        }
    }

    /// A ring over the held peak: the stage damped for it from the next
    /// frame, by the ring's overshoot (`p` over the held peak; the held
    /// peak rises by its fourth root). Damping by the power of the whole
    /// overshoot would damp a ring again for the peak it reached before
    /// the last damping took hold, and the guard would hunt.
    #[inline(never)]
    fn rise(&mut self, i: usize, p: f32) {
        self.held[i] *= (p / self.held[i]).sqrt().sqrt();
        let f = damping_for(self.held[i], self.over[i]).min(self.most[i]);
        if f != self.damp[i] {
            self.damp[i] = f;
            if self.left == 0 {
                let c = self.damped(self.to[i], i);
                if i == 0 {
                    self.st.a.set_coefs(c);
                } else {
                    self.st.b.set_coefs(c);
                }
            }
        }
    }

    /// One step of the glide of the last two stages.
    #[inline(always)]
    fn glide(&mut self) {
        self.left -= 1;
        // The last step lands on the setting itself, not a rounding short
        // of it.
        let last = self.left == 0;
        let t = self.glide_t();
        let at = |from: SvfCoefs, to: SvfCoefs| if last { to } else { from.toward(to, t) };
        let a = self.damped(at(self.from[0], self.to[0]), 0);
        self.st.a.set_coefs(a);
        if self.st.cascaded {
            let b = self.damped(at(self.from[1], self.to[1]), 1);
            self.st.b.set_coefs(b);
        }
    }

    /// The same step for the stages before them, after [`Multi::glide`]
    /// has taken it.
    #[inline(always)]
    fn glide_extra(&mut self) {
        let last = self.left == 0;
        let t = self.glide_t();
        for i in 0..self.pre_n as usize {
            let c = if last {
                self.pre_to[i]
            } else {
                self.pre_from[i].toward(self.pre_to[i], t)
            };
            self.pre[i].set_coefs(c);
        }
        if self.one_on {
            let g = if last {
                self.one_to
            } else {
                self.one_from + t * (self.one_to - self.one_from)
            };
            self.one.set_gain(g);
        }
    }

    /// The guard's damping on the more damped of the stages: 1 when it
    /// holds nothing back.
    pub fn damping(&self) -> f32 {
        self.damp[0].max(self.damp[1])
    }

    /// Every stage to silence and the guard let go; the setting stays.
    pub fn reset(&mut self) {
        self.st.a.reset();
        self.st.b.reset();
        for s in &mut self.pre {
            s.reset();
        }
        self.one.reset();
        for i in 0..2 {
            if self.held[i].is_finite() {
                self.held[i] = self.knee[i];
            }
            self.cur[i] = 0.0;
            self.win[i] = [0.0; GUARD_BLOCKS];
        }
        (self.frames, self.wi) = (0, 0);
        if self.damping() > 1.0 {
            self.damp = [1.0; 2];
            if self.left == 0 {
                self.land();
            }
        }
    }

    /// Zeros a channel of a stage once both its states are under `quiet`
    /// ([`Svf::flush_below`]); for a host to run every few frames.
    #[inline]
    pub fn flush_below(&mut self, quiet: f32) {
        if self.extra {
            self.one.flush_below(quiet);
            for s in &mut self.pre[..self.pre_n as usize] {
                s.flush_below(quiet);
            }
        }
        self.st.a.flush_below(quiet);
        if self.st.cascaded {
            self.st.b.flush_below(quiet);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A noise-like test signal: deterministic, full of every frequency.
    fn noise(n: usize) -> Vec<f32> {
        let mut x = 0x1234_5678u32;
        (0..n)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                (x as f32 / u32::MAX as f32) * 2.0 - 1.0
            })
            .collect()
    }

    #[test]
    fn the_effects_12_and_24_db_are_its_old_stages_bit_for_bit() {
        // Every mode at 12 dB, and the low-pass, high-pass, band-pass and
        // notch at 24: the same coefficients, the same bends, the same
        // numbers out.
        let x = noise(4096);
        for (mode, poles) in [
            (SvfMode::LowPass, 2),
            (SvfMode::HighPass, 2),
            (SvfMode::BandPass, 2),
            (SvfMode::Notch, 2),
            (SvfMode::Bell, 2),
            (SvfMode::LowShelf, 2),
            (SvfMode::HighShelf, 2),
            (SvfMode::LowPass, 4),
            (SvfMode::HighPass, 4),
            (SvfMode::BandPass, 4),
            (SvfMode::Notch, 4),
            (SvfMode::Bell, 4),
        ] {
            for res in [0.0, 0.37, 1.0] {
                let cascaded = poles == 4 && !matches!(mode, SvfMode::Bell);
                let mut old = SlopeStages::default().st;
                set_stages(
                    &mut old.a, &mut old.b, mode, cascaded, 1234.0, res, 5.0, 48_000,
                );
                old.cascaded = cascaded;
                (old.bend_a, old.bend_b) = bends(mode, cascaded);
                let mut new = SlopeStages::default();
                new.set(mode, poles, 1234.0, res, 5.0, 48_000);
                assert_eq!(new.layout(), (false, 0, cascaded), "{mode:?} {poles}");
                assert_eq!(
                    (new.st.bend_a, new.st.bend_b),
                    (old.bend_a, old.bend_b),
                    "{mode:?} {poles}"
                );
                let (knee, room) = bend_for(res);
                for &v in &x {
                    let a = old.run(0, v * 3.0, knee, room);
                    let b = new.run(0, v * 3.0, knee, room);
                    assert_eq!(a.to_bits(), b.to_bits(), "{mode:?} {poles} {res}");
                }
            }
        }
    }

    #[test]
    fn the_effects_slopes_are_the_pads_and_run_as_drawn() {
        // At a cutoff well under 45 % of the rate the effect's stages are
        // the pads' plan (but the 24 dB band-pass, the effect's own); a
        // block runs as frame by frame does; and the linear stages are
        // heard as drawn.
        let rate = 48_000;
        let x = noise(8192);
        for mode in [
            SvfMode::LowPass,
            SvfMode::HighPass,
            SvfMode::BandPass,
            SvfMode::Notch,
        ] {
            for poles in [1, 2, 3, 4, 6, 8] {
                let mut st = SlopeStages::default();
                st.set(mode, poles, 900.0, 0.4, 0.0, rate);
                for hz in [100.0, 450.0, 900.0, 1800.0, 7000.0] {
                    if mode == SvfMode::BandPass && poles == 4 {
                        break;
                    }
                    let h = st.response(hz, rate);
                    let drawn = 10.0 * (h.0 * h.0 + h.1 * h.1).log10();
                    let pads = response_db(mode, poles, 900.0, 0.4, hz, rate);
                    assert!(
                        (drawn as f32 - pads).abs() < 1e-3,
                        "{mode:?} {poles} at {hz}: {drawn} against the pads' {pads}"
                    );
                }
                let mut frame = st;
                let mut block = st;
                let mut y = x.clone();
                block.run_block(1, &mut y, f32::INFINITY, 1.0);
                for (i, &v) in x.iter().enumerate() {
                    let f = frame.run(1, v, f32::INFINITY, 1.0);
                    assert_eq!(f.to_bits(), y[i].to_bits(), "{mode:?} {poles} {i}");
                }
            }
        }
    }

    #[test]
    fn a_stage_that_comes_in_takes_up_the_sound() {
        // A low tone through a 24 dB low-pass, then 48 dB: the two stages
        // that come in at the front start from what the first one heard,
        // so the output carries on near the tone, not from a dip.
        let rate = 48_000;
        let mut st = SlopeStages::default();
        st.set(SvfMode::LowPass, 4, 2000.0, 0.0, 0.0, rate);
        let tone = |i: usize| (std::f32::consts::TAU * 50.0 * i as f32 / rate as f32).cos();
        let mut last = 0.0;
        for i in 0..4800 {
            last = st.run(0, tone(i), f32::INFINITY, 1.0);
        }
        let was = st.layout();
        st.set(SvfMode::LowPass, 8, 2000.0, 0.0, 0.0, rate);
        let mut cold = st;
        st.clear_new(was);
        assert_eq!(st.layout(), (false, 2, true));
        // Against the 48 dB filter run from the start, over the next 5 ms;
        // and the same switch with the new stages at rest instead.
        let mut steady = SlopeStages::default();
        steady.set(SvfMode::LowPass, 8, 2000.0, 0.0, 0.0, rate);
        for i in 0..4800 {
            steady.run(0, tone(i), f32::INFINITY, 1.0);
        }
        for s in &mut cold.pre {
            s.reset();
        }
        let (mut warm_off, mut cold_off) = (0.0f32, 0.0f32);
        for i in 4800..5040 {
            let want = steady.run(0, tone(i), f32::INFINITY, 1.0);
            warm_off = warm_off.max((st.run(0, tone(i), f32::INFINITY, 1.0) - want).abs());
            cold_off = cold_off.max((cold.run(0, tone(i), f32::INFINITY, 1.0) - want).abs());
        }
        assert!(last.abs() > 0.1);
        assert!(
            warm_off < 0.1 * cold_off,
            "taken up {warm_off}, at rest {cold_off}"
        );
    }

    /// A plan's stages designed at `hz` (under 45 % of the rate, where the
    /// effect's stages stop).
    fn designed(mode: SvfMode, poles: u8, hz: f32, res: f32, rate: u32) -> Vec<Svf> {
        let t = corner(hz, rate, 0.45);
        plan(mode, poles, res)
            .stages()
            .iter()
            .map(|st| {
                let mut s = Svf::default();
                s.set_coefs(st.coefs(t));
                s
            })
            .collect()
    }

    #[test]
    fn the_12_and_24_db_stages_are_the_effects() {
        // The pads' 12 and 24 dB low-pass, high-pass and notch, and the
        // 12 dB band-pass, are the Filter effect's stages exactly (its 24
        // dB band-pass is two resonant stages; the pads' is F4's).
        for (mode, poles) in [
            (SvfMode::LowPass, 2),
            (SvfMode::LowPass, 4),
            (SvfMode::HighPass, 2),
            (SvfMode::HighPass, 4),
            (SvfMode::BandPass, 2),
            (SvfMode::Notch, 2),
            (SvfMode::Notch, 4),
        ] {
            let cascaded = poles == 4;
            for res in [0.0, 0.3, 1.0] {
                let (mut a, mut b) = (Svf::default(), Svf::default());
                set_stages(&mut a, &mut b, mode, cascaded, 1234.0, res, 0.0, 44_100);
                let ours = designed(mode, poles, 1234.0, res, 44_100);
                assert_eq!(ours.len(), if cascaded { 2 } else { 1 });
                for hz in [100.0, 1234.0, 9000.0] {
                    assert_eq!(a.response(hz, 44_100), ours[0].response(hz, 44_100));
                    if cascaded {
                        assert_eq!(b.response(hz, 44_100), ours[1].response(hz, 44_100));
                    }
                }
            }
        }
    }

    /// `plan`'s curve at `hz` (dB), worked out the long way: each stage's
    /// complex response multiplied, the one-pole's too, in f64.
    fn curve_db(mode: SvfMode, poles: u8, cutoff: f32, res: f32, hz: f32, rate: u32) -> f64 {
        let p = plan(mode, poles, res);
        let t = corner(cutoff, rate, MULTI_TOP);
        let mut h = (1.0f64, 0.0f64);
        let mul = |a: (f64, f64), b: (f64, f64)| (a.0 * b.0 - a.1 * b.1, a.0 * b.1 + a.1 * b.0);
        if let Some(hp) = p.one {
            let mut o = OnePole::default();
            o.set(OnePole::gain_for(t), hp);
            h = mul(h, o.response(hz, rate));
        }
        for st in p.stages() {
            let mut s = Svf::default();
            s.set_coefs(st.coefs(t));
            h = mul(h, s.response(hz, rate));
        }
        10.0 * (h.0 * h.0 + h.1 * h.1).log10()
    }

    #[test]
    fn every_slope_is_butterworth_at_resonance_0_and_falls_as_it_should() {
        // The -3 dB point at the cutoff, Butterworth's curve in the
        // passband, and 6 dB an octave a pole past the cutoff (over 4 to 8
        // times it, a little short of that for the 6 dB).
        let rate = 44_100;
        for mode in [SvfMode::LowPass, SvfMode::HighPass] {
            for poles in [1u8, 2, 3, 4, 6, 8] {
                // Low cutoffs and frequencies, where the bilinear
                // transform's warping is small.
                let (cutoff, pass, far, farther) = if mode == SvfMode::LowPass {
                    (100.0, 20.0, 400.0, 800.0)
                } else {
                    (400.0, 2_000.0, 100.0, 50.0)
                };
                let at = |hz: f32| curve_db(mode, poles, cutoff, 0.0, hz, rate);
                assert!(
                    (at(cutoff) + 3.0103).abs() < 0.01,
                    "{mode:?} {poles}: {}",
                    at(cutoff)
                );
                // Butterworth's own curve there: -10 log(1 + 5^-2n).
                let flat = -10.0 * (1.0 + 0.2f64.powi(2 * i32::from(poles))).log10();
                assert!(
                    (at(pass) - flat).abs() < 0.01,
                    "{mode:?} {poles}: {}",
                    at(pass)
                );
                let octave = at(far) - at(farther);
                let want = 6.02 * f64::from(poles);
                assert!(
                    (octave - want).abs() < 0.03 * want + 0.6,
                    "{mode:?} {poles}: {octave} for {want}"
                );
            }
        }
    }

    #[test]
    fn the_resonance_gives_the_same_peak_at_every_slope() {
        // At the cutoff a low-pass, high-pass or band-pass gives the
        // 2-pole's Q at any slope (-3 dB at Resonance 0 up to 32 dB), and
        // the Peak a boost of that over 0.707 (0 dB up to 35 dB).
        let rate = 44_100;
        for res in [0.0, 0.25, 0.5, 0.75, 1.0] {
            let want = 20.0 * f64::from(res_to_q(res)).log10();
            for (mode, slopes) in [
                (SvfMode::LowPass, &[1u8, 2, 3, 4, 6, 8][..]),
                (SvfMode::HighPass, &[1, 2, 3, 4, 6, 8]),
                (SvfMode::BandPass, &[2, 4, 6, 8]),
            ] {
                for &poles in slopes {
                    let got = curve_db(mode, poles, 1_000.0, res, 1_000.0, rate);
                    assert!(
                        (got - want).abs() < 0.05,
                        "{mode:?} {poles} res {res}: {got} for {want}"
                    );
                }
            }
            let peak = curve_db(SvfMode::Bell, 2, 1_000.0, res, 1_000.0, rate);
            assert!(
                (peak - (want + 3.0103)).abs() < 0.05,
                "peak res {res}: {peak}"
            );
            // And it leaves the rest alone.
            let far = curve_db(SvfMode::Bell, 2, 1_000.0, res, 20.0, rate);
            assert!(far.abs() < 0.05, "peak res {res} at 20 Hz: {far}");
        }
    }

    #[test]
    fn a_slope_count_goes_to_one_the_mode_has() {
        assert_eq!(slope(SvfMode::LowPass, 0), 2);
        assert_eq!(slope(SvfMode::LowPass, 5), 4);
        assert_eq!(slope(SvfMode::HighPass, 7), 6);
        assert_eq!(slope(SvfMode::HighPass, 200), 8);
        assert_eq!(slope(SvfMode::BandPass, 1), 2);
        assert_eq!(slope(SvfMode::BandPass, 3), 2);
        assert_eq!(slope(SvfMode::BandPass, 8), 8);
        assert_eq!(slope(SvfMode::Notch, 8), 4);
        assert_eq!(slope(SvfMode::Bell, 4), 2);
        // The stages make the slope: two poles a state-variable stage (a
        // boost adds none), one for the one-pole.
        for mode in [SvfMode::LowPass, SvfMode::HighPass, SvfMode::BandPass] {
            for poles in 0..=9 {
                let p = plan(mode, poles, 0.5);
                let made: usize = p
                    .stages()
                    .iter()
                    .map(|s| if s.mode == SvfMode::Bell { 0 } else { 2 })
                    .sum::<usize>()
                    + usize::from(p.one.is_some());
                assert_eq!(made, usize::from(slope(mode, poles)), "{mode:?} {poles}");
                assert_eq!(p.stages().iter().filter(|s| s.resonant).count(), 1);
                assert!(p.stages().last().is_some_and(|s| s.resonant));
            }
        }
        // The guard's floor: the resonant stage's pole Q at Resonance 0,
        // what that stage is in the Butterworth plan.
        for mode in [
            SvfMode::LowPass,
            SvfMode::HighPass,
            SvfMode::BandPass,
            SvfMode::Bell,
        ] {
            for poles in 0..=9 {
                let butterworth = plan(mode, poles, 0.0);
                let last = butterworth.stages()[butterworth.n - 1];
                for res in [0.0, 0.3, 1.0] {
                    let q0 = plan(mode, poles, res).q0;
                    assert!((q0 - last.pole_q()).abs() < 1e-6, "{mode:?} {poles}");
                }
            }
        }
    }

    /// `x` (mono, on both channels) through a [`Multi`] set once; the left
    /// channel out.
    fn heard(m: &mut Multi, x: &[f32]) -> Vec<f32> {
        x.iter()
            .enumerate()
            .map(|(n, &s)| {
                if n > 0 && n % 16 == 0 {
                    m.guard(16);
                    m.flush_below(1e-15);
                }
                m.process(s, s).0
            })
            .collect()
    }

    #[test]
    fn the_drawn_curve_is_what_plays() {
        // A quiet sine (the guard never acts) through every slope: its
        // level out is the drawn curve's at its pitch.
        let rate = 44_100;
        for (mode, slopes) in [
            (SvfMode::LowPass, &[1u8, 2, 3, 4, 6, 8][..]),
            (SvfMode::HighPass, &[1, 2, 3, 4, 6, 8]),
            (SvfMode::BandPass, &[2, 4, 6, 8]),
            (SvfMode::Notch, &[2, 4]),
            (SvfMode::Bell, &[2]),
        ] {
            for &poles in slopes {
                for hz in [700.0f32, 2_900.0, 9_000.0] {
                    let mut m = Multi::default();
                    m.set(mode, poles, 3_000.0, 0.6, rate, 0);
                    let a = 1e-3;
                    let x: Vec<f32> = (0..rate as usize / 2)
                        .map(|n| {
                            let w = std::f64::consts::TAU * f64::from(hz) * n as f64;
                            a * (w / f64::from(rate)).sin() as f32
                        })
                        .collect();
                    let y = heard(&mut m, &x);
                    let rms = |v: &[f32]| {
                        (v.iter().map(|&s| f64::from(s).powi(2)).sum::<f64>() / v.len() as f64)
                            .sqrt()
                    };
                    let tail = &y[y.len() / 2..];
                    let got = 20.0 * (rms(tail) / rms(&x[x.len() / 2..])).log10();
                    let drawn = f64::from(response_db(mode, poles, 3_000.0, 0.6, hz, rate));
                    assert!(
                        (got - drawn).abs() < 0.05,
                        "{mode:?} {poles} at {hz}: heard {got}, drawn {drawn}"
                    );
                    assert!((drawn - curve_db(mode, poles, 3_000.0, 0.6, hz, rate)).abs() < 1e-3);
                    assert_eq!(m.damping(), 1.0);
                }
            }
        }
    }

    #[test]
    fn a_multi_opens_wider_than_the_effect() {
        // Wide open, the voice's filter goes to 49 % of the rate; the
        // effect's stages stop at 45 %.
        let mut m = Multi::default();
        m.set(SvfMode::LowPass, 2, 22_000.0, 0.0, 44_100, 0);
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
        m.set(SvfMode::LowPass, 4, 500.0, 0.2, 44_100, 0);
        m.set(SvfMode::LowPass, 4, 4000.0, 0.2, 44_100, 16);
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
        let t = corner(4000.0, 44_100, MULTI_TOP);
        z.set_coefs(plan(SvfMode::LowPass, 4, 0.2).stages[0].coefs(t));
        assert_eq!(m.st.a.response(2000.0, 44_100), z.response(2000.0, 44_100));
    }

    #[test]
    fn a_steep_slope_glides_every_stage() {
        // An 8-pole (and a 6 dB) low-pass opening over 64 frames: each
        // frame lets more of a 3 kHz tone through, and it lands on the
        // setting, as one set there at once.
        for poles in [8u8, 1, 3, 6] {
            let mut m = Multi::default();
            m.set(SvfMode::LowPass, poles, 500.0, 0.3, 44_100, 0);
            m.set(SvfMode::LowPass, poles, 4000.0, 0.3, 44_100, 64);
            let mut glide = m;
            let mut at = Multi::default();
            at.set(SvfMode::LowPass, poles, 4000.0, 0.3, 44_100, 0);
            for _ in 0..64 {
                glide.process(0.0, 0.0);
            }
            // Landed: the same stages, so the same output from here on.
            let x: Vec<f32> = (0..256)
                .map(|n| ((n * 7919) % 97) as f32 / 48.0 - 1.0)
                .collect();
            for &s in &x {
                assert_eq!(glide.process(s, s), at.process(s, s), "{poles}");
            }
        }
    }

    const RATE: u32 = 44_100;
    const STEP: u32 = 16;

    /// `x` (mono, on both channels) through a [`Multi`] set once, the guard
    /// run every [`STEP`] frames as a voice runs it; the left channel out,
    /// and the guard's damping each frame.
    fn guarded(mode: SvfMode, poles: u8, hz: f32, res: f32, x: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let mut m = Multi::default();
        m.set(mode, poles, hz, res, RATE, 0);
        let mut y = Vec::with_capacity(x.len());
        let mut d = Vec::with_capacity(x.len());
        for (n, &s) in x.iter().enumerate() {
            if n > 0 && n % STEP as usize == 0 {
                m.guard(STEP);
                m.flush_below(1e-15);
            }
            y.push(m.process(s, s).0);
            d.push(m.damping());
        }
        (y, d)
    }

    /// A sawtooth at full scale, `cycle` frames a cycle, centred so that
    /// its cycles average to 0; sharp edges down.
    fn sawtooth(n: usize, cycle: usize) -> Vec<f32> {
        (0..n)
            .map(|i| (2 * (i % cycle) + 1) as f32 / cycle as f32 - 1.0)
            .collect()
    }

    fn mean(x: &[f32]) -> f32 {
        (x.iter().map(|&v| f64::from(v)).sum::<f64>() / x.len() as f64) as f32
    }

    fn peak(x: &[f32]) -> f32 {
        x.iter().fold(0.0f32, |a, &b| a.max(b.abs()))
    }

    #[test]
    fn a_loud_sawtooth_stays_centred_and_its_ring_is_held() {
        // A lopsided wave at full scale through a resonant low-pass, a high
        // note (1.8 kHz, 2 kHz cutoff) and a low one (110 Hz, 120 Hz): the
        // guard damps it but holds still across the cycles, so nothing
        // pushes the output off centre (the Filter effect's bend gives
        // -0.12 on the high one), and the ring is held near full scale,
        // not at 4.7 (Q 40's, linear, on the high note). Every slope; the
        // 6 dB's guard, its boost rung hard by the sawtooth's first edge,
        // takes the longest to settle (about a second on the low note), so
        // the last half second is measured.
        for (cycle, hz) in [(25, 2_000.0), (401, 120.0)] {
            let x = sawtooth(2 * RATE as usize, cycle);
            // Whole cycles of the last half second.
            let tail = RATE as usize / 2 / cycle * cycle;
            let from = x.len() - tail;
            for poles in [2u8, 4, 1, 3, 6, 8] {
                for res in [0.3, 0.6, 1.0] {
                    let (y, d) = guarded(SvfMode::LowPass, poles, hz, res, &x);
                    let dc = mean(&y[from..]);
                    assert!(dc.abs() < 2e-4, "{cycle} {poles} res {res}: {dc}");
                    assert!(
                        peak(&y[from..]) < 1.6,
                        "{cycle} {poles} res {res}: {}",
                        peak(&y)
                    );
                    // Held still once it has the note.
                    let (lo, hi) = d[from..]
                        .iter()
                        .fold((f32::MAX, 0.0f32), |(a, b), &v| (a.min(v), b.max(v)));
                    assert_eq!(lo, hi, "{cycle} {poles} res {res}");
                }
            }
            let (_, d) = guarded(SvfMode::LowPass, 2, hz, 1.0, &x);
            assert!(d[x.len() - 1] > 4.0, "{cycle}: {}", d[x.len() - 1]);
            // The Peak passes the sawtooth whole and adds its ring, held
            // as a low-pass's (its band-pass state √2 lower, as the bell
            // adds it up to √2 times).
            for res in [0.3, 0.6, 1.0] {
                let (y, d) = guarded(SvfMode::Bell, 2, hz, res, &x);
                let dc = mean(&y[from..]);
                assert!(dc.abs() < 2e-4, "{cycle} peak res {res}: {dc}");
                assert!(
                    peak(&y[from..]) < 1.8,
                    "{cycle} peak res {res}: {}",
                    peak(&y)
                );
                let (lo, hi) = d[from..]
                    .iter()
                    .fold((f32::MAX, 0.0f32), |(a, b), &v| (a.min(v), b.max(v)));
                assert_eq!(lo, hi, "{cycle} peak res {res}");
            }
        }
    }

    #[test]
    fn a_quiet_sound_rings_up_in_full() {
        // -42 dBFS on the cutoff at full Resonance: under the knee, so the
        // guard never acts, and the ring is Q 40's (32 dB).
        let a = 0.008;
        let x: Vec<f32> = (0..RATE as usize)
            .map(|n| a * (std::f32::consts::TAU * 2_000.0 * n as f32 / RATE as f32).sin())
            .collect();
        for poles in [2u8, 4, 1, 3, 6, 8] {
            let (y, d) = guarded(SvfMode::LowPass, poles, 2_000.0, 1.0, &x);
            let up = 20.0 * (peak(&y[y.len() / 2..]) / a).log10();
            assert!((up - 32.0).abs() < 0.3, "{poles}: {up}");
            assert!(d.iter().all(|&v| v == 1.0), "{poles}");
        }
        // The Peak's knee is √2 lower: at -46 dBFS its boost is in full,
        // 35 dB.
        let a = 0.005;
        let x: Vec<f32> = x.iter().map(|v| v * (a / 0.008)).collect();
        let (y, d) = guarded(SvfMode::Bell, 2, 2_000.0, 1.0, &x);
        let up = 20.0 * (peak(&y[y.len() / 2..]) / a).log10();
        assert!((up - 35.05).abs() < 0.3, "peak: {up}");
        assert!(d.iter().all(|&v| v == 1.0), "peak");
    }

    #[test]
    fn the_guard_lets_go_as_the_sound_quietens() {
        // A loud half second, then a quiet one: the damping is back to
        // none within a few tenths of a second.
        let x: Vec<f32> = sawtooth(RATE as usize, 25)
            .iter()
            .enumerate()
            .map(|(n, &s)| if n < RATE as usize / 2 { s } else { 0.005 * s })
            .collect();
        let (_, d) = guarded(SvfMode::LowPass, 2, 2_000.0, 1.0, &x);
        assert!(d[RATE as usize / 2 - 1] > 4.0);
        assert_eq!(d[x.len() - 1], 1.0);
    }

    #[test]
    fn at_resonance_0_and_on_a_notch_the_guard_never_acts() {
        // Butterworth is as damped as the guard goes, and a notch rings up
        // inside, not at its output: both are the linear filter as set, at
        // any level.
        let x: Vec<f32> = sawtooth(RATE as usize / 4, 25)
            .iter()
            .map(|s| 4.0 * s)
            .collect();
        for poles in [2u8, 4, 1, 3, 6, 8] {
            for (mode, res) in [(SvfMode::LowPass, 0.0), (SvfMode::Notch, 1.0)] {
                let (_, d) = guarded(mode, poles, 2_000.0, res, &x);
                assert!(d.iter().all(|&v| v == 1.0), "{mode:?} {poles}");
            }
        }
    }

    #[test]
    fn a_loud_note_s_first_ring_is_held_as_it_builds() {
        // Full-scale noise from silence through resonant band-passes high
        // up, where a ring builds in a millisecond: damped in the frame it
        // rises, so even the first frames stay within 6 dB of full scale
        // (linear, the 24 dB band-pass at Resonance 0.9 rings past 20).
        let mut seed = 0x5eed_u32;
        let x: Vec<f32> = (0..RATE as usize / 4)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                (seed as f32 / u32::MAX as f32) * 2.0 - 1.0
            })
            .collect();
        for (poles, hz) in [
            (2u8, 10_000.0),
            (4, 10_000.0),
            (4, 3_000.0),
            (8, 10_000.0),
            (8, 3_000.0),
        ] {
            let (y, _) = guarded(SvfMode::BandPass, poles, hz, 0.9, &x);
            assert!(peak(&y) < 2.0, "{poles} {hz}: {}", peak(&y));
        }
    }

    #[test]
    fn the_guard_eases_without_a_step() {
        // A tone swelling from silence to full scale on a resonant
        // band-pass's centre: the guard takes over on the way, and the
        // output never moves from one frame to the next by more than a
        // sine at its own peak level would (2 pi f / rate times it).
        let n = RATE as usize / 2;
        let x: Vec<f32> = (0..n)
            .map(|i| {
                let a = i as f32 / n as f32;
                a * (std::f32::consts::TAU * 700.0 * i as f32 / RATE as f32).sin()
            })
            .collect();
        for poles in [2u8, 4, 6, 8] {
            let (y, d) = guarded(SvfMode::BandPass, poles, 700.0, 0.8, &x);
            assert!(d[n - 1] > 1.0, "{poles}");
            let top = peak(&y);
            let most = y
                .windows(2)
                .map(|w| (w[1] - w[0]).abs())
                .fold(0.0f32, f32::max);
            let sine = std::f32::consts::TAU * 700.0 / RATE as f32 * top;
            assert!(most < 1.05 * sine, "{poles}: {most} {sine}");
            assert!(top < 1.2, "{poles}: {top}");
        }
    }
}
