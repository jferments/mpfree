//! Analog: a vintage analog poly, built from the research collected in
//! `docs/dev/research/vintage-poly.md`.
//!
//! - **Voice cards.** Sixteen cards, each with fixed differences of its
//!   own: both oscillators' tuning, the cutoff, the envelopes' times, the
//!   pulse width and the level, scaled by **Age**. At Age 100 every
//!   oscillator also drifts by a few cents, a random walk smoothed over
//!   seconds. Notes go to the cards in turn, as on the old instruments, so
//!   a repeated chord lands on different cards.
//! - **Oscillators.** Two per card, each a saw, a pulse, both, or a
//!   triangle, band-limited with polyBLEP steps placed a sample late, so a
//!   step anywhere in a sample (a wrap, a pulse edge, a sync reset) gets
//!   both halves of its correction. Oscillator 1 can be synced to 2; a
//!   square sub an octave under oscillator 1; white noise. The oscillators
//!   run on while a card is silent, so each note starts at a different
//!   phase.
//! - **Poly-mod.** Each card's filter envelope (**PMod env**) and its
//!   oscillator 2 (**PMod osc 2**, at audio rate) go to oscillator 1's
//!   pitch, its pulse width and the cutoff, as switched.
//! - **Filter.** **LP 24**: four OTA stages, each `dv/dt = w tanh(x - v)`,
//!   with the output fed back (k up to 4.1, so it whistles at 100), solved
//!   with zero-delay feedback each sample, each stage's `tanh(d)/d` taken
//!   from the previous frame; half the passband's loss to resonance is
//!   made up. **LP 12**, **BP 12**, **HP 12**: a state-variable filter
//!   whose first integrator's input saturates the same way.
//! - **Envelopes.** The CEM3310's curve: the attack charges toward 1.3
//!   times the peak and stops at it, decay and release fall exponentially;
//!   1 ms to 20 s.
//! - **Chorus.** A bucket-brigade delay: its clock swept by a triangle, so
//!   the delay sweeps as the clock's reciprocal; filtered before and after,
//!   with a little hiss that follows the signal, left and right swept in
//!   antiphase and mixed 1.62 dB under the dry sound (**Slow**, **Fast**,
//!   **Vibe**).
//!
//! Everything per sample runs at the host's rate (no oversampling): the
//! oscillators are band-limited and the filter's nonlinearity is mild.
//! Pitch, the envelopes, the LFO, drift and the cutoff run at a control
//! step of [`CTRL`] frames aligned to the absolute frame, with pitch,
//! pulse width, cutoff and level moving linearly across the step, so the
//! output does not depend on how the host cuts blocks. Pressure and
//! per-note expression are ignored. Nothing allocates after `new`.

use std::f32::consts::PI;

use mpfree_node::{Ctx, Kind, Node, NodeEvent, NodeInfo, ParamInfo, Smooth, Timed, Tuning};

use crate::analog_params::*;
use crate::dsp::{lfo_value, LfoShape, Rng};
use crate::params::Preset;

/// Frames per control step.
pub const CTRL: u64 = 16;
/// Voice cards.
pub const CARDS: usize = 16;
/// Keys remembered while held, for Unison's return to a held key.
const HELD: usize = 16;

static INFO: NodeInfo = NodeInfo {
    id: "mpfree.analog",
    name: "Analog",
    kind: Kind::Instrument,
};

/// The filter envelope's reach at Env amount 100, in octaves.
pub const ENV_OCT: f32 = 5.0;
/// The LFO's reach on the cutoff at 100, in octaves.
const LFO_CUT_OCT: f32 = 2.0;
/// How far a pulse width swings at 100 (LFO or poly-mod).
const PW_SWING: f32 = 0.45;
/// Poly-mod's reach at 100: the envelope on oscillator 1's pitch (in
/// semitones) and on the cutoff (octaves); oscillator 2 on oscillator 1's
/// pitch and on the cutoff (octaves each way).
const PM_ENV_SEMIS: f32 = 48.0;
const PM_ENV_CUT_OCT: f32 = 4.0;
const PM_OSC2_OCT: f32 = 2.0;
const PM_OSC2_CUT_OCT: f32 = 3.0;
/// Unison's spread at Detune 100, in cents each side.
const UNI_CENTS: f32 = 25.0;
/// The four-pole's feedback at Resonance 100 (it oscillates past 4).
const K_MAX: f32 = 4.1;
/// The two-pole's damping (1/Q) at Resonance 0 and 100, and how much it
/// grows with the square of the band-pass's level, so a loud resonance
/// squashes instead of running away (our reading of an OTA filter's
/// limits).
const SVF_DAMP_MAX: f32 = 2.0;
const SVF_DAMP_MIN: f32 = 0.06;
const SVF_SQUASH: f32 = 0.1;
/// The attack's asymptote against its peak (CEM3310: 6.5 V and 5 V).
const ATTACK_TARGET: f32 = 1.3;
/// Age's spreads at 100, each the standard deviation of a card's fixed
/// offset (our reading of "+/- 5-10 cents per voice").
const AGE_TUNE_CENTS: f32 = 4.0;
const AGE_CUT_OCT: f32 = 0.084;
const AGE_TIME: f32 = 0.08;
const AGE_PW: f32 = 0.025;
const AGE_LEVEL_DB: f32 = 0.5;
/// Drift at Age 100: its rms in cents and how slowly it wanders.
const DRIFT_CENTS: f32 = 2.0;
const DRIFT_TAU_S: f32 = 5.0;
/// The oscillators into the filter at Drive 0, and how far Drive 100
/// raises that (octaves of gain); the filter's output is scaled back.
const IN_GAIN: f32 = 0.5;
const DRIVE_OCT: f32 = 3.0;
/// The voices' output: one note at the defaults peaks near -12 dBFS.
const OUT_GAIN: f32 = 0.2;
/// Saw and pulse together: each at this weight.
const BOTH: f32 = 0.6;
const DC_HZ: f32 = 10.0;
/// The chorus: the wet signal against the dry (-1.62 dB), the whole
/// trimmed so switching it on is not much louder, the filters around the
/// delay, the hiss against the signal and how fast it follows.
const CH_WET: f32 = 0.83;
const CH_TRIM: f32 = 0.71;
const CH_PRE_HZ: f32 = 9000.0;
const CH_POST_HZ: f32 = 7500.0;
const CH_HISS: f32 = 0.0015;
const CH_FOLLOW_S: f32 = 0.05;
/// The chorus's delay line (a power of two, longer than 5.6 ms at 384
/// kHz is not needed: delays are capped to fit).
const CH_BUF: usize = 4096;
/// The modes: LFO rate (Hz) and the delay's ends (ms).
const CH_MODES: [(f32, f32, f32); 3] = [(0.51, 1.4, 5.6), (0.83, 1.4, 5.6), (9.75, 3.2, 3.8)];

/// `2^x`, a cubic on the fraction (0.2 cents at worst), for modulation.
#[inline(always)]
fn exp2_fast(x: f32) -> f32 {
    let x = x.clamp(-24.0, 24.0);
    let t = x as i32;
    let i = if (t as f32) > x { t - 1 } else { t };
    let f = x - i as f32;
    let p = 1.0 + f * (0.695_502 + f * (0.226_270 + f * 0.078_228));
    p * f32::from_bits(((i + 127) as u32) << 23)
}

/// A one-pole stage's input share `G / (1 + G)` for `G = g tanh(d) / d`:
/// the Padé form inside ±3, `1/|d|` past it, one division either way. The
/// state's share is one minus it.
#[inline(always)]
fn stage_share(g: f32, d: f32) -> f32 {
    let d2 = d * d;
    if d2 < 9.0 {
        let n = g * (27.0 + d2);
        n / (27.0 + 9.0 * d2 + n)
    } else {
        g / (d.abs() + g)
    }
}

/// A wave as weights of its parts.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Shape {
    saw: f32,
    pulse: f32,
    tri: f32,
}

impl Shape {
    fn of(wave: f32) -> Shape {
        match wave as i32 {
            1 => Shape {
                pulse: 1.0,
                ..Shape::default()
            },
            2 => Shape {
                saw: BOTH,
                pulse: BOTH,
                tri: 0.0,
            },
            3 => Shape {
                tri: 1.0,
                ..Shape::default()
            },
            _ => Shape {
                saw: 1.0,
                ..Shape::default()
            },
        }
    }

    const SQUARE: Shape = Shape {
        saw: 0.0,
        pulse: 1.0,
        tri: 0.0,
    };

    /// The wave at phase `p` with no band limiting.
    #[inline(always)]
    fn naive(&self, p: f32, pw: f32) -> f32 {
        let mut y = self.saw * (2.0 * p - 1.0);
        if self.pulse != 0.0 {
            y += if p < pw { self.pulse } else { -self.pulse };
        }
        if self.tri != 0.0 {
            y += self.tri * (1.0 - 4.0 * (p - 0.5).abs());
        }
        y
    }
}

/// A step of height `h` a distance `d` (0 to 1 samples) before the current
/// sample: half its polyBLEP correction goes on the sample before, which
/// has not been heard yet, half on this one.
#[inline(always)]
fn emit(h: f32, d: f32, pre: &mut f32, post: &mut f32) {
    let d = d.clamp(0.0, 1.0);
    let e = 1.0 - d;
    *pre += 0.5 * h * d * d;
    *post -= 0.5 * h * e * e;
}

/// Moves phase `p` on by `inc * span`, emitting the wave's steps (the
/// saw's drop and the pulse's rise at the wrap, the pulse's fall at its
/// width) at their distance before the sample, `base` plus the time from
/// the step to the end of the span. Sets `wrap` to the wrap's distance.
#[allow(clippy::too_many_arguments)]
#[inline(always)]
fn advance(
    p: f32,
    inc: f32,
    span: f32,
    base: f32,
    pw: f32,
    sh: Shape,
    pre: &mut f32,
    post: &mut f32,
    wrap: &mut f32,
) -> f32 {
    let end = p + inc * span;
    if sh.pulse != 0.0 && p < pw && end >= pw {
        emit(-2.0 * sh.pulse, base + (end - pw) / inc, pre, post);
    }
    if end >= 1.0 {
        let e = end - 1.0;
        let d = base + e / inc;
        *wrap = d;
        emit(2.0 * (sh.pulse - sh.saw), d, pre, post);
        if sh.pulse != 0.0 && e >= pw {
            emit(-2.0 * sh.pulse, base + (e - pw) / inc, pre, post);
        }
        return e;
    }
    end
}

/// One oscillator: its phase, and the output held back a sample for the
/// next step's correction.
#[derive(Clone, Copy, Debug, Default)]
struct Osc {
    p: f32,
    held: f32,
}

impl Osc {
    #[inline(always)]
    fn step(&mut self, inc: f32, pw: f32, sh: Shape, wrap: &mut f32) -> f32 {
        let mut post = 0.0;
        self.p = advance(
            self.p,
            inc,
            1.0,
            0.0,
            pw,
            sh,
            &mut self.held,
            &mut post,
            wrap,
        );
        let out = self.held;
        self.held = sh.naive(self.p, pw) + post;
        out
    }

    /// A step that starts its cycle again at the master's wrap, `sync`
    /// samples before the sample (negative: no wrap).
    #[inline(always)]
    fn step_sync(&mut self, inc: f32, pw: f32, sh: Shape, sync: f32) -> f32 {
        if sync < 0.0 {
            let mut w = -1.0;
            return self.step(inc, pw, sh, &mut w);
        }
        let mut post = 0.0;
        let mut w = -1.0;
        let before = advance(
            self.p,
            inc,
            1.0 - sync,
            sync,
            pw,
            sh,
            &mut self.held,
            &mut post,
            &mut w,
        );
        let h = sh.naive(0.0, pw) - sh.naive(before, pw);
        emit(h, sync, &mut self.held, &mut post);
        self.p = advance(
            0.0,
            inc,
            sync,
            0.0,
            pw,
            sh,
            &mut self.held,
            &mut post,
            &mut w,
        );
        let out = self.held;
        self.held = sh.naive(self.p, pw) + post;
        out
    }
}

const IDLE: u8 = 0;
const ATTACK: u8 = 1;
const DECAY: u8 = 2;
const RELEASE: u8 = 3;

/// An envelope's per-step coefficients.
#[derive(Clone, Copy, Debug, Default)]
struct EnvCoefs {
    attack: f32,
    decay: f32,
    sustain: f32,
    release: f32,
}

#[derive(Clone, Copy, Debug, Default)]
struct Env {
    level: f32,
    stage: u8,
}

impl Env {
    /// The attack starts from where the level is, so a retrigger never
    /// clicks.
    fn gate_on(&mut self) {
        self.stage = ATTACK;
    }
    fn gate_off(&mut self) {
        if self.stage != IDLE {
            self.stage = RELEASE;
        }
    }
    #[inline]
    fn step(&mut self, c: &EnvCoefs) -> f32 {
        match self.stage {
            ATTACK => {
                self.level += (ATTACK_TARGET - self.level) * c.attack;
                if self.level >= 1.0 {
                    self.level = 1.0;
                    self.stage = DECAY;
                }
            }
            DECAY => self.level += (c.sustain - self.level) * c.decay,
            RELEASE => {
                self.level -= self.level * c.release;
                if self.level < 1e-4 {
                    self.level = 0.0;
                    self.stage = IDLE;
                }
            }
            _ => {}
        }
        self.level
    }
}

const FREE: u8 = 0;
const HELD_KEY: u8 = 1;
const RELEASED: u8 = 2;

/// One voice card's state.
#[derive(Clone, Copy, Debug, Default)]
struct Voice {
    state: u8,
    note: u8,
    vel: f32,
    order: u64,
    /// The note it plays and glides toward, in semitones; a cent offset in
    /// Unison.
    target: f32,
    pitch: f32,
    gliding: bool,
    played: bool,
    uni: f32,
    /// The frequency of `hz_key` through the tuning.
    hz_key: f32,
    hz: f32,
    o1: Osc,
    o2: Osc,
    sub: Osc,
    noise: u32,
    inc1: f32,
    inc2: f32,
    dinc1: f32,
    dinc2: f32,
    pw1: f32,
    pw2: f32,
    dpw1: f32,
    dpw2: f32,
    /// Where this step's ramps arrive.
    inc1_to: f32,
    inc2_to: f32,
    pw1_to: f32,
    pw2_to: f32,
    g_to: f32,
    vca_to: f32,
    /// The OTA stages' states and their input differences last frame.
    s: [f32; 4],
    d: [f32; 4],
    /// The state-variable filter's states and its band-pass last frame.
    z1: f32,
    z2: f32,
    hp: f32,
    g: f32,
    dg: f32,
    fenv: Env,
    aenv: Env,
    vca: f32,
    dvca: f32,
}

impl Voice {
    fn silence(&mut self) {
        self.state = FREE;
        self.fenv = Env::default();
        self.aenv = Env::default();
        self.s = [0.0; 4];
        self.d = [0.0; 4];
        self.z1 = 0.0;
        self.z2 = 0.0;
        self.hp = 0.0;
        self.vca = 0.0;
        self.dvca = 0.0;
        self.vca_to = 0.0;
        self.o1.held = 0.0;
        self.o2.held = 0.0;
        self.sub.held = 0.0;
    }

    /// The four OTA stages for input `x`, cutoff `g` and feedback `k`.
    #[inline(always)]
    fn ota(&mut self, x: f32, g: f32, k: f32) -> f32 {
        let a1 = stage_share(g, self.d[0]);
        let a2 = stage_share(g, self.d[1]);
        let a3 = stage_share(g, self.d[2]);
        let a4 = stage_share(g, self.d[3]);
        let [s1, s2, s3, s4] = self.s;
        let b1 = (1.0 - a1) * s1;
        let b2 = (1.0 - a2) * s2;
        let b3 = (1.0 - a3) * s3;
        let b4 = (1.0 - a4) * s4;
        // The output as A u + B, so the feedback can be solved for.
        let a43 = a4 * a3;
        let aa = a43 * a2 * a1;
        let bb = a43 * (a2 * b1 + b2) + a4 * b3 + b4;
        let u = (x - k * bb) / (1.0 + k * aa);
        let v1 = a1 * u + b1;
        let v2 = a2 * v1 + b2;
        let v3 = a3 * v2 + b3;
        let v4 = a4 * v3 + b4;
        self.d = [u - v1, v1 - v2, v2 - v3, v3 - v4];
        self.s = [2.0 * v1 - s1, 2.0 * v2 - s2, 2.0 * v3 - s3, 2.0 * v4 - s4];
        v4
    }

    /// The state-variable filter: low-, band- (`mode` 1) or high-pass (2)
    /// for damping `r`, growing with the band-pass's level last frame.
    #[inline(always)]
    fn svf(&mut self, x: f32, g: f32, r: f32, mode: u8, bp_gain: f32) -> f32 {
        let r = r + SVF_SQUASH * self.hp * self.hp;
        let hp = (x - (r + g) * self.z1 - self.z2) / (1.0 + r * g + g * g);
        let bp = g * hp + self.z1;
        self.z1 = g * hp + bp;
        let lp = g * bp + self.z2;
        self.z2 = g * bp + lp;
        self.hp = bp;
        match mode {
            0 => lp,
            1 => bp * bp_gain,
            _ => hp,
        }
    }
}

/// What every voice reads in a control step.
#[derive(Clone, Copy, Debug, Default)]
struct Frame {
    sh1: Shape,
    sh2: Shape,
    l1: f32,
    l2: f32,
    lsub: f32,
    lnoise: f32,
    need2: bool,
    sync: bool,
    /// Oscillator 2 on oscillator 1's pitch (octaves), pulse width and
    /// the cutoff (octaves).
    fm: f32,
    pm_pw: f32,
    pm_cut: f32,
    /// 0 LP 24, 1 LP 12, 2 BP 12, 3 HP 12.
    filter: u8,
    k: f32,
    damp: f32,
    bp_gain: f32,
    in_gain: f32,
    g_max: f32,
}

/// A two-pole low-pass (Butterworth), for the chorus.
#[derive(Clone, Copy, Debug, Default)]
struct Lp2 {
    z1: f32,
    z2: f32,
    g: f32,
    h: f32,
}

impl Lp2 {
    fn set(&mut self, hz: f32, rate: f32) {
        self.g = (PI * hz.min(rate * 0.45) / rate).tan();
        self.h = 1.0 / (1.0 + SQRT2 * self.g + self.g * self.g);
    }
    #[inline(always)]
    fn run(&mut self, x: f32) -> f32 {
        let hp = (x - (SQRT2 + self.g) * self.z1 - self.z2) * self.h;
        let bp = self.g * hp + self.z1;
        self.z1 = self.g * hp + bp;
        let lp = self.g * bp + self.z2;
        self.z2 = self.g * bp + lp;
        lp
    }
    fn clear(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }
}

const SQRT2: f32 = std::f32::consts::SQRT_2;

/// The bucket-brigade chorus.
struct Chorus {
    buf: Vec<f32>,
    w: usize,
    phase: f32,
    pre: Lp2,
    post_l: Lp2,
    post_r: Lp2,
    follow: f32,
    follow_k: f32,
    rng: Rng,
    /// The mix (0 dry to 1 full), the rate, the delay's middle and its
    /// clock's swing, smoothed so a change of mode doesn't jump.
    wet: Smooth,
    rate: Smooth,
    mid: Smooth,
    swing: Smooth,
    /// The delays (samples) and their steps across a control step.
    dl: f32,
    dr: f32,
    ddl: f32,
    ddr: f32,
    max_delay: f32,
}

impl Chorus {
    fn new() -> Chorus {
        Chorus {
            buf: vec![0.0; CH_BUF],
            w: 0,
            phase: 0.0,
            pre: Lp2::default(),
            post_l: Lp2::default(),
            post_r: Lp2::default(),
            follow: 0.0,
            follow_k: 0.0,
            rng: Rng::new(0x5EED_C0DE),
            wet: Smooth::new(0.0),
            rate: Smooth::new(CH_MODES[0].0),
            mid: Smooth::new(0.0),
            swing: Smooth::new(0.0),
            dl: 0.0,
            dr: 0.0,
            ddl: 0.0,
            ddr: 0.0,
            max_delay: 0.0,
        }
    }

    /// The clock's middle delay and swing for a mode's delay range: the
    /// delay is `mid / (1 + swing * tri)`, the reciprocal of a clock swept
    /// by the triangle.
    fn targets(&mut self, mode: f32, rate: f32) {
        let m = (mode as usize).clamp(1, 3) - 1;
        let (hz, lo, hi) = CH_MODES[m];
        let swing = (hi - lo) / (hi + lo);
        let mid = lo * 0.001 * rate * (1.0 + swing);
        self.rate.target = hz;
        self.mid.target = mid;
        self.swing.target = swing;
        self.wet.target = if mode >= 1.0 { 1.0 } else { 0.0 };
    }

    fn snap(&mut self) {
        self.wet.snap(self.wet.target);
        self.rate.snap(self.rate.target);
        self.mid.snap(self.mid.target);
        self.swing.snap(self.swing.target);
    }

    fn clear(&mut self) {
        self.buf.fill(0.0);
        self.pre.clear();
        self.post_l.clear();
        self.post_r.clear();
        self.follow = 0.0;
    }

    /// One control step: the LFO moves on and the delays head for where it
    /// will be at the step's end.
    fn control(&mut self, dt: f32) {
        self.wet.next();
        let hz = self.rate.next();
        let mid = self.mid.next();
        let swing = self.swing.next();
        self.phase += hz * dt;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        let tri = 1.0 - 4.0 * (self.phase - 0.5).abs();
        let to_l = (mid / (1.0 + swing * tri)).min(self.max_delay);
        let to_r = (mid / (1.0 - swing * tri)).min(self.max_delay);
        self.ddl = (to_l - self.dl) / CTRL as f32;
        self.ddr = (to_r - self.dr) / CTRL as f32;
    }

    #[inline(always)]
    fn read(&self, delay: f32) -> f32 {
        let d = delay.max(1.0);
        let n = d as usize;
        let f = d - n as f32;
        let mask = CH_BUF - 1;
        let a = self.buf[(self.w + CH_BUF - n) & mask];
        let b = self.buf[(self.w + CH_BUF - n - 1) & mask];
        a + (b - a) * f
    }

    /// One frame: the dry signal in, left and right out.
    #[inline(always)]
    fn run(&mut self, x: f32) -> (f32, f32) {
        let wet = self.wet.value;
        let xf = self.pre.run(x);
        self.w = (self.w + 1) & (CH_BUF - 1);
        self.buf[self.w] = xf;
        let l = self.read(self.dl);
        let r = self.read(self.dr);
        self.dl += self.ddl;
        self.dr += self.ddr;
        // The hiss follows the signal, as a compander makes it.
        let a = xf.abs();
        self.follow = if a > self.follow {
            a
        } else {
            self.follow * self.follow_k
        };
        let n = CH_HISS * self.follow;
        let hl = self.rng.bipolar() * n;
        let hr = self.rng.bipolar() * n;
        let wl = self.post_l.run(l + hl);
        let wr = self.post_r.run(r + hr);
        let dry = 1.0 - (1.0 - CH_TRIM) * wet;
        let gw = CH_WET * CH_TRIM * wet;
        (x * dry + wl * gw, x * dry + wr * gw)
    }
}

/// A unit normal draw (three uniforms summed, scaled).
fn normal(rng: &mut Rng) -> f32 {
    (rng.unit() + rng.unit() + rng.unit() - 1.5) * 2.0
}

/// One unit-normal draw per card, clamped to ±2.5.
fn draw_cards(rng: &mut Rng) -> [f32; CARDS] {
    let mut a = [0.0; CARDS];
    for v in &mut a {
        *v = normal(rng).clamp(-2.5, 2.5);
    }
    a
}

/// The cards' fixed offsets (unit normals: osc 1 and osc 2 tuning, cutoff,
/// times, pulse width, level), the same in every instance, and the
/// generator after them (for the drift's start).
fn spread_and_rng() -> ([[f32; CARDS]; 6], Rng) {
    let mut rng = Rng::new(0xA1A1_0C4D);
    let spread = std::array::from_fn(|_| draw_cards(&mut rng));
    (spread, rng)
}

/// Each card's fixed detune in cents, oscillator 1 and 2, at **Age** `age`
/// (0 to 100), drift left out. For the Synth tab's picture.
pub fn card_cents(age: f32) -> [(f32, f32); CARDS] {
    let ([t1, t2, ..], _) = spread_and_rng();
    let a = AGE_TUNE_CENTS * (age / 100.0).clamp(0.0, 1.0);
    std::array::from_fn(|c| (a * t1[c], a * t2[c]))
}

/// The envelope's attack at `t` (0 to 1 of the attack time): an RC charge
/// toward 1.3 times the peak, stopped at the peak.
pub fn attack_shape(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    ATTACK_TARGET * (1.0 - (1.0 - 1.0 / ATTACK_TARGET).powf(t))
}

/// The filter's small-signal gain at `hz` with the amplifier's make-up,
/// for **Filter** `mode` (LP 24, LP 12, BP 12, HP 12), **Cutoff** `cutoff`
/// (Hz) and **Resonance** `resonance` (0 to 100): four equal one-poles
/// with feedback, or the state-variable filter at its small-signal
/// damping. For the Synth tab's picture.
pub fn filter_gain(mode: f32, cutoff: f32, resonance: f32, hz: f32) -> f32 {
    let w = hz.max(0.0) / cutoff.max(1.0);
    let r = (resonance / 100.0).clamp(0.0, 1.0);
    match (mode.round() as i32).clamp(0, 3) {
        0 => {
            // 1 / ((1 + s)^4 + k), s = jw, with sqrt(1 + k) made up.
            let k = K_MAX * r;
            let re = 1.0 - 6.0 * w * w + w.powi(4) + k;
            let im = 4.0 * w - 4.0 * w.powi(3);
            (1.0 + k).sqrt() / (re * re + im * im).sqrt()
        }
        m => {
            let d = SVF_DAMP_MAX * (SVF_DAMP_MIN / SVF_DAMP_MAX).powf(r);
            let den = ((1.0 - w * w).powi(2) + (d * w).powi(2)).sqrt();
            let num = match m {
                1 => 1.0,
                2 => w * d.sqrt(),
                _ => w * w,
            };
            num / den
        }
    }
}

pub struct Analog {
    rate: u32,
    values: [f32; ID_SPACE],
    voices: [Voice; CARDS],
    /// The next card in the rotation, and a counter that orders notes.
    next_card: usize,
    order: u64,
    held: [u8; HELD],
    n_held: usize,
    /// Unison's note.
    uni_note: u8,
    /// Each card's fixed offsets, as unit normals: tuning of each
    /// oscillator, cutoff, envelope times, pulse width, level.
    off_tune1: [f32; CARDS],
    off_tune2: [f32; CARDS],
    off_cut: [f32; CARDS],
    off_time: [f32; CARDS],
    off_pw: [f32; CARDS],
    off_level: [f32; CARDS],
    /// Each oscillator's drift (unit rms) and its step's factors.
    drift1: [f32; CARDS],
    drift2: [f32; CARDS],
    drift_keep: f32,
    drift_new: f32,
    drift_rng: Rng,
    /// Each card's envelope coefficients (filter, amp) with its time
    /// offset.
    fco: [EnvCoefs; CARDS],
    aco: [EnvCoefs; CARDS],
    fr: Frame,
    lfo_phase: f32,
    lfo_held: f32,
    lfo_fade: f32,
    lfo_rng: Rng,
    bend: Smooth,
    bend_value: f32,
    out_l: Smooth,
    out_r: Smooth,
    gl: f32,
    gr: f32,
    dgl: f32,
    dgr: f32,
    dc_x: f32,
    dc_y: f32,
    dc_a: f32,
    chorus: Chorus,
    /// Frames since the last voice stopped, for the chorus's tail.
    quiet: u32,
    idle: bool,
    tuning: Tuning,
}

impl Analog {
    pub fn new(rate: u32) -> Analog {
        let mut values = [0.0; ID_SPACE];
        for p in PARAMS {
            values[p.id as usize] = p.default;
        }
        let (spread, mut rng) = spread_and_rng();
        let [off_tune1, off_tune2, off_cut, off_time, off_pw, off_level] = spread;
        let drift1 = draw_cards(&mut rng);
        let drift2 = draw_cards(&mut rng);
        let mut s = Analog {
            rate: rate.max(1),
            values,
            voices: [Voice::default(); CARDS],
            next_card: 0,
            order: 0,
            held: [0; HELD],
            n_held: 0,
            uni_note: 60,
            off_tune1,
            off_tune2,
            off_cut,
            off_time,
            off_pw,
            off_level,
            drift1,
            drift2,
            drift_keep: 1.0,
            drift_new: 0.0,
            drift_rng: Rng::new(0xD21F_7001),
            fco: [EnvCoefs::default(); CARDS],
            aco: [EnvCoefs::default(); CARDS],
            fr: Frame::default(),
            lfo_phase: 0.0,
            lfo_held: 0.0,
            lfo_fade: 1.0,
            lfo_rng: Rng::new(0x1F0_5A3D),
            bend: Smooth::new(0.0),
            bend_value: 0.0,
            out_l: Smooth::new(1.0),
            out_r: Smooth::new(1.0),
            gl: 1.0,
            gr: 1.0,
            dgl: 0.0,
            dgr: 0.0,
            dc_x: 0.0,
            dc_y: 0.0,
            dc_a: 0.0,
            chorus: Chorus::new(),
            quiet: 0,
            idle: true,
            tuning: Tuning::default(),
        };
        // Each oscillator starts somewhere different in its cycle.
        let mut ph = Rng::new(0x000F_F5E7);
        for (i, v) in s.voices.iter_mut().enumerate() {
            v.o1.p = ph.unit();
            v.o2.p = ph.unit();
            v.sub.p = ph.unit();
            v.noise = 0x9E37_79B9u32.wrapping_mul(i as u32 + 1);
            v.pitch = 60.0;
            v.target = 60.0;
            v.hz_key = f32::NAN;
        }
        s.prepare(rate, 0);
        s
    }

    pub fn load_preset(&mut self, preset: &Preset) {
        for p in PARAMS {
            if !p.readonly {
                self.set_param(p.id, p.default);
            }
        }
        for &(id, v) in preset.values {
            self.set_param(id, v);
        }
    }

    pub fn presets() -> &'static [Preset] {
        PRESETS
    }

    /// Cards sounding, then one while the chorus rings out.
    pub fn active_voices(&self) -> usize {
        let n = self.voices.iter().filter(|v| v.state != FREE).count();
        if n == 0 && !self.idle {
            1
        } else {
            n
        }
    }

    /// The note a card plays, if it is sounding.
    pub fn card_note(&self, card: usize) -> Option<u8> {
        self.voices
            .get(card)
            .filter(|v| v.state != FREE)
            .map(|v| v.note)
    }

    /// A card's oscillator 1 frequency now, in Hz.
    pub fn card_hz(&self, card: usize) -> f32 {
        self.voices.get(card).map_or(0.0, |v| v.inc1 * self.fs())
    }

    /// A card's cutoff now, in Hz.
    pub fn card_cutoff(&self, card: usize) -> f32 {
        self.voices
            .get(card)
            .map_or(0.0, |v| v.g.atan() * self.fs() / PI)
    }

    #[inline]
    fn v(&self, id: u16) -> f32 {
        self.values[id as usize]
    }

    fn fs(&self) -> f32 {
        self.rate as f32
    }

    fn update_out(&mut self) {
        let db = self.v(LEVEL);
        let lvl = if db <= -60.0 {
            0.0
        } else {
            10f32.powf(db / 20.0)
        };
        let pan = self.v(PAN) / 100.0;
        self.out_l.target = lvl * (1.0 - pan).sqrt();
        self.out_r.target = lvl * (1.0 + pan).sqrt();
        if self.idle {
            self.snap_out();
        }
    }

    fn snap_out(&mut self) {
        self.out_l.snap(self.out_l.target);
        self.out_r.snap(self.out_r.target);
        self.gl = self.out_l.value;
        self.gr = self.out_r.value;
        self.dgl = 0.0;
        self.dgr = 0.0;
    }

    fn update_chorus(&mut self) {
        let r = self.fs();
        self.chorus.targets(self.v(CHORUS), r);
        if self.idle {
            self.chorus.snap();
        }
    }

    /// Every card's envelope coefficients, with its own time offset.
    fn update_times(&mut self) {
        let steps = self.fs() / CTRL as f32;
        let age = self.v(AGE) / 100.0;
        let attack = |ms: f32| {
            let n = (ms * 0.001 * steps).max(1.0);
            1.0 - (1.0 - 1.0 / ATTACK_TARGET).powf(1.0 / n)
        };
        let fall = |ms: f32| {
            let n = (ms * 0.001 * steps).max(1.0);
            1.0 - (-4.6 / n).exp()
        };
        for c in 0..CARDS {
            let k = (AGE_TIME * age * self.off_time[c]).exp();
            self.fco[c] = EnvCoefs {
                attack: attack(self.v(F_ATTACK) * k),
                decay: fall(self.v(F_DECAY) * k),
                sustain: self.v(F_SUSTAIN) / 100.0,
                release: fall(self.v(F_RELEASE) * k),
            };
            self.aco[c] = EnvCoefs {
                attack: attack(self.v(A_ATTACK) * k),
                decay: fall(self.v(A_DECAY) * k),
                sustain: self.v(A_SUSTAIN) / 100.0,
                release: fall(self.v(A_RELEASE) * k),
            };
        }
    }

    fn push(&mut self, note: u8) {
        self.remove(note);
        if self.n_held == HELD {
            self.held.copy_within(1.., 0);
            self.n_held -= 1;
        }
        self.held[self.n_held] = note;
        self.n_held += 1;
    }

    fn remove(&mut self, note: u8) -> bool {
        if let Some(i) = self.held[..self.n_held].iter().position(|&n| n == note) {
            self.held.copy_within(i + 1..self.n_held, i);
            self.n_held -= 1;
            true
        } else {
            false
        }
    }

    fn cards(&self) -> usize {
        (self.v(POLYPHONY) as usize).clamp(1, CARDS)
    }

    /// Starts `card` on `note`, gliding there if Glide is set and the card
    /// has played before.
    fn start(&mut self, card: usize, note: u8, vel: f32, uni: f32) {
        self.order += 1;
        let glide = self.v(GLIDE) > 0.0;
        let v = &mut self.voices[card];
        v.state = HELD_KEY;
        v.note = note;
        v.vel = vel;
        v.order = self.order;
        v.uni = uni;
        v.target = f32::from(note);
        if glide && v.played {
            v.gliding = true;
        } else {
            v.pitch = v.target;
            v.gliding = false;
        }
        v.played = true;
        v.fenv.gate_on();
        v.aenv.gate_on();
    }

    /// The card for a new note: the same note still sounding keeps its
    /// card; otherwise the next free card in turn, then the one let go
    /// longest ago, then the oldest held.
    fn pick(&mut self, note: u8) -> usize {
        let n = self.cards();
        if let Some(c) =
            (0..n).find(|&c| self.voices[c].state != FREE && self.voices[c].note == note)
        {
            return c;
        }
        let start = self.next_card % n;
        let order = (0..n).map(|i| (start + i) % n);
        if let Some(c) = order.clone().find(|&c| self.voices[c].state == FREE) {
            return c;
        }
        let oldest = |state: u8| {
            (0..n)
                .filter(|&c| self.voices[c].state == state)
                .min_by_key(|&c| self.voices[c].order)
        };
        oldest(RELEASED)
            .or_else(|| oldest(HELD_KEY))
            .unwrap_or(start)
    }

    fn note_on(&mut self, note: u8, velocity: u8) {
        if self.idle {
            self.bend.snap(self.bend_value);
            self.idle = false;
        }
        if self.n_held == 0 {
            // The LFO's delay starts again with a fresh phrase.
            self.lfo_fade = if self.v(LFO_DELAY) > 0.0 { 0.0 } else { 1.0 };
        }
        self.push(note);
        let vel = f32::from(velocity) / 127.0;
        if self.v(VOICE_MODE) >= UNISON {
            self.uni_note = note;
            let n = self.cards();
            for c in 0..n {
                let spread = if n > 1 {
                    (c as f32 / (n - 1) as f32) * 2.0 - 1.0
                } else {
                    0.0
                };
                self.start(c, note, vel, spread);
            }
        } else {
            let c = self.pick(note);
            self.start(c, note, vel, 0.0);
            self.next_card = c + 1;
        }
    }

    fn note_off(&mut self, note: u8) {
        self.remove(note);
        if self.v(VOICE_MODE) >= UNISON {
            if note != self.uni_note {
                return;
            }
            if self.n_held > 0 {
                // Back to the key still held, gliding if set.
                let back = self.held[self.n_held - 1];
                self.uni_note = back;
                let glide = self.v(GLIDE) > 0.0;
                for v in self.voices.iter_mut().filter(|v| v.state == HELD_KEY) {
                    v.note = back;
                    v.target = f32::from(back);
                    if glide {
                        v.gliding = true;
                    } else {
                        v.pitch = v.target;
                    }
                }
                return;
            }
        }
        let uni = self.v(VOICE_MODE) >= UNISON;
        for v in self.voices.iter_mut() {
            if v.state == HELD_KEY && (v.note == note || uni) {
                v.state = RELEASED;
                v.fenv.gate_off();
                v.aenv.gate_off();
            }
        }
    }

    fn all_off(&mut self) {
        self.n_held = 0;
        for v in self.voices.iter_mut() {
            if v.state == HELD_KEY {
                v.state = RELEASED;
                v.fenv.gate_off();
                v.aenv.gate_off();
            }
        }
    }

    fn apply(&mut self, ev: &NodeEvent) {
        match *ev {
            NodeEvent::NoteOn { note, velocity } => {
                if velocity == 0 {
                    self.note_off(note);
                } else {
                    self.note_on(note.min(127), velocity);
                }
            }
            NodeEvent::NoteOff { note } => self.note_off(note),
            NodeEvent::Bend { value } => {
                self.bend_value = if value.is_finite() {
                    value.clamp(-1.0, 1.0)
                } else {
                    0.0
                };
                self.bend.target = self.bend_value;
            }
            NodeEvent::AllOff => self.all_off(),
            NodeEvent::Param { id, value } => self.set_param(id, value),
            NodeEvent::Pressure { .. } | NodeEvent::Expression { .. } => {}
        }
    }

    /// What the voices read this step, from the values.
    fn frame(&self) -> Frame {
        // The mixer: sources at full add up, but a sum past one source is
        // brought down by its power, so two oscillators are 3 dB up, not 6.
        let (l1, l2) = (self.v(OSC1_LEVEL) / 100.0, self.v(OSC2_LEVEL) / 100.0);
        let (ls, ln) = (self.v(SUB_LEVEL) / 100.0, self.v(NOISE_LEVEL) / 100.0);
        let norm = 1.0 / (l1 * l1 + l2 * l2 + ls * ls + ln * ln).max(1.0).sqrt();
        let (l1, l2, ls, ln) = (l1 * norm, l2 * norm, ls * norm, ln * norm);
        let pm2 = self.v(PM_OSC2) / 100.0;
        let fm = if self.v(PM_PITCH) > 0.5 {
            pm2 * PM_OSC2_OCT
        } else {
            0.0
        };
        let pm_pw = if self.v(PM_PW) > 0.5 {
            pm2 * PW_SWING
        } else {
            0.0
        };
        let pm_cut = if self.v(PM_CUTOFF) > 0.5 {
            pm2 * PM_OSC2_CUT_OCT
        } else {
            0.0
        };
        let sync = self.v(SYNC) > 0.5;
        let r = self.v(RESONANCE) / 100.0;
        let filter = (self.v(FILTER_TYPE) as u8).min(3);
        let damp = SVF_DAMP_MAX * (SVF_DAMP_MIN / SVF_DAMP_MAX).powf(r);
        Frame {
            sh1: Shape::of(self.v(OSC1_WAVE)),
            sh2: Shape::of(self.v(OSC2_WAVE)),
            l1,
            l2,
            lsub: ls,
            lnoise: ln,
            need2: l2 > 0.0 || sync || fm > 0.0 || pm_pw > 0.0 || pm_cut > 0.0,
            sync,
            fm,
            pm_pw,
            pm_cut,
            filter,
            k: K_MAX * r,
            damp,
            // The band-pass's peak grows as 1/damping; half of that (in dB)
            // is kept.
            bp_gain: damp.sqrt(),
            in_gain: IN_GAIN * exp2_fast(DRIVE_OCT * self.v(DRIVE) / 100.0),
            g_max: (PI * 0.45).tan(),
        }
    }

    /// One control step: the LFO, drift, then each card's pitch,
    /// envelopes, cutoff and level for the next `CTRL` frames.
    fn control(&mut self) {
        if self.idle {
            return;
        }
        let fs = self.fs();
        let dt = CTRL as f32 / fs;
        let per = CTRL as f32;
        self.bend.next();
        self.gl = self.out_l.value;
        self.gr = self.out_r.value;
        self.dgl = (self.out_l.next() - self.gl) / per;
        self.dgr = (self.out_r.next() - self.gr) / per;
        self.chorus.control(dt);
        self.fr = self.frame();
        let fr = self.fr;

        // The LFO, shared, fading in after its delay.
        self.lfo_phase += self.v(LFO_RATE) * dt;
        if self.lfo_phase >= 1.0 {
            self.lfo_phase -= self.lfo_phase.floor();
            self.lfo_held = self.lfo_rng.bipolar();
        }
        let delay = self.v(LFO_DELAY) * 0.001;
        if self.lfo_fade < 1.0 {
            self.lfo_fade = if delay > 0.0 {
                (self.lfo_fade + dt / delay).min(1.0)
            } else {
                1.0
            };
        }
        let shape = LfoShape::from_index(self.v(LFO_WAVE));
        let lfo = lfo_value(shape, self.lfo_phase, self.lfo_held) * self.lfo_fade;

        // Drift: each oscillator a slow random walk (unit rms).
        let age = self.v(AGE) / 100.0;
        for c in 0..CARDS {
            self.drift1[c] =
                self.drift1[c] * self.drift_keep + self.drift_new * normal(&mut self.drift_rng);
            self.drift2[c] =
                self.drift2[c] * self.drift_keep + self.drift_new * normal(&mut self.drift_rng);
        }

        let transpose = self.v(TRANSPOSE);
        let bend = self.bend.value * self.v(BEND_RANGE);
        let oct1 = exp2_fast(self.v(OSC1_OCT));
        let semis2 = self.v(OSC2_OCT) * 12.0 + self.v(OSC2_SEMI);
        let fine = self.v(FINE) + self.v(LFO_PITCH) * lfo;
        let fine2 = self.v(OSC2_FINE);
        let uni = self.v(UNI_DETUNE) / 100.0 * UNI_CENTS;
        let glide_ms = self.v(GLIDE);
        let glide_a = if glide_ms > 0.0 {
            1.0 - (-dt * 3.0 / (glide_ms * 0.001)).exp()
        } else {
            1.0
        };
        let pm_env = self.v(PM_ENV) / 100.0;
        let pm_env_pitch = if self.v(PM_PITCH) > 0.5 {
            pm_env * PM_ENV_SEMIS
        } else {
            0.0
        };
        let pm_env_pw = if self.v(PM_PW) > 0.5 {
            pm_env * PW_SWING
        } else {
            0.0
        };
        let pm_env_cut = if self.v(PM_CUTOFF) > 0.5 {
            pm_env * PM_ENV_CUT_OCT
        } else {
            0.0
        };
        let lfo_pw = self.v(LFO_PW) / 100.0 * PW_SWING * lfo;
        let pw1 = self.v(OSC1_PW) / 100.0 + lfo_pw;
        let pw2 = self.v(OSC2_PW) / 100.0 + lfo_pw;
        let cut_oct = self.v(CUTOFF).log2() + self.v(LFO_CUTOFF) / 100.0 * LFO_CUT_OCT * lfo;
        let env_oct = self.v(ENV_AMT) / 100.0 * ENV_OCT;
        let kt = self.v(KEY_TRACK) / 100.0;
        let vf = self.v(VEL_FILTER) / 100.0;
        let va = self.v(VEL_AMP) / 100.0;
        let comp = if fr.filter == 0 {
            (1.0 + fr.k).sqrt()
        } else {
            1.0
        };
        let out = OUT_GAIN * comp / fr.in_gain;
        let fmax = (fs * 0.45).min(20_000.0);
        let mut sounding = 0;

        for c in 0..CARDS {
            let v = &mut self.voices[c];
            if v.state == FREE {
                continue;
            }
            // Last step's ramps have arrived: land exactly.
            let fresh = v.vca_to == 0.0;
            v.inc1 = v.inc1_to;
            v.inc2 = v.inc2_to;
            v.pw1 = v.pw1_to;
            v.pw2 = v.pw2_to;
            v.g = v.g_to;
            v.vca = v.vca_to;
            let fe = v.fenv.step(&self.fco[c]);
            let ae = v.aenv.step(&self.aco[c]);
            if v.state == RELEASED && v.aenv.stage == IDLE {
                v.silence();
                continue;
            }
            sounding += 1;
            if v.gliding {
                v.pitch += (v.target - v.pitch) * glide_a;
                if (v.target - v.pitch).abs() < 1e-3 {
                    v.pitch = v.target;
                    v.gliding = false;
                }
            }
            let key = v.pitch + transpose + bend;
            if key != v.hz_key {
                let hz = self.tuning.hz(key);
                v.hz = if hz.is_finite() { hz } else { 0.0 };
                v.hz_key = key;
            }
            let t1 = AGE_TUNE_CENTS * age * self.off_tune1[c] + DRIFT_CENTS * age * self.drift1[c];
            let t2 = AGE_TUNE_CENTS * age * self.off_tune2[c] + DRIFT_CENTS * age * self.drift2[c];
            let cents = fine + v.uni * uni;
            let semis1 = (cents + t1) / 100.0 + pm_env_pitch * fe;
            let to1 = (v.hz * oct1 * exp2_fast(semis1 / 12.0) / fs).clamp(0.0, 0.45);
            let to2 = (v.hz * exp2_fast((semis2 + (cents + t2 + fine2) / 100.0) / 12.0) / fs)
                .clamp(0.0, 0.45);
            let pwo = AGE_PW * age * self.off_pw[c];
            let p1 = (pw1 + pwo + pm_env_pw * fe).clamp(0.05, 0.95);
            let p2 = (pw2 - pwo).clamp(0.05, 0.95);
            if fresh {
                // A card starting from silence: no sweep from where it was.
                v.inc1 = to1;
                v.inc2 = to2;
                v.pw1 = p1;
                v.pw2 = p2;
            }
            v.inc1_to = to1;
            v.inc2_to = to2;
            v.pw1_to = p1;
            v.pw2_to = p2;
            v.dinc1 = (to1 - v.inc1) / per;
            v.dinc2 = (to2 - v.inc2) / per;
            v.dpw1 = (p1 - v.pw1) / per;
            v.dpw2 = (p2 - v.pw2) / per;
            let velf = 1.0 - vf + vf * v.vel;
            let oct = cut_oct
                + AGE_CUT_OCT * age * self.off_cut[c]
                + env_oct * fe * velf
                + kt * (key - 60.0) / 12.0
                + pm_env_cut * fe;
            let fc = exp2_fast(oct).clamp(16.0, fmax);
            let g = (PI * fc / fs).tan();
            if fresh {
                v.g = g;
            }
            v.g_to = g;
            v.dg = (g - v.g) / per;
            let level = exp2_fast(AGE_LEVEL_DB * age * self.off_level[c] / 6.02);
            let gain = ae * (1.0 - va + va * v.vel) * level * out;
            v.vca_to = gain;
            v.dvca = (gain - v.vca) / per;
        }

        // Once every card has stopped, let the chorus and the DC blocker
        // ring out, then stop.
        if sounding == 0 {
            self.quiet = self.quiet.saturating_add(CTRL as u32);
            let tail = (0.02 * fs) as u32;
            if self.quiet >= tail && self.dc_y.abs() < 1e-6 {
                self.idle = true;
                self.quiet = 0;
                self.dc_x = 0.0;
                self.dc_y = 0.0;
                self.chorus.clear();
                self.chorus.snap();
                self.snap_out();
            }
        } else {
            self.quiet = 0;
        }
    }

    fn render(&mut self, from: usize, to: usize, left: &mut [f32], right: &mut [f32]) {
        if self.idle {
            return;
        }
        let n = to - from;
        let mut buf = [0.0f32; CTRL as usize];
        let fr = self.fr;
        for v in self.voices.iter_mut() {
            if v.state == FREE {
                continue;
            }
            for y in buf.iter_mut().take(n) {
                let mut wrap = -1.0;
                let o2 = if fr.need2 {
                    v.o2.step(v.inc2, v.pw2, fr.sh2, &mut wrap)
                } else {
                    0.0
                };
                let inc1 = if fr.fm > 0.0 {
                    (v.inc1 * exp2_fast(fr.fm * o2)).min(0.45)
                } else {
                    v.inc1
                };
                let pw1 = if fr.pm_pw > 0.0 {
                    (v.pw1 + fr.pm_pw * o2).clamp(0.05, 0.95)
                } else {
                    v.pw1
                };
                let o1 = if fr.sync {
                    v.o1.step_sync(inc1, pw1, fr.sh1, wrap)
                } else {
                    v.o1.step(inc1, pw1, fr.sh1, &mut wrap)
                };
                let mut x = o1 * fr.l1 + o2 * fr.l2;
                if fr.lsub > 0.0 {
                    let mut w = -1.0;
                    x += fr.lsub * v.sub.step(v.inc1 * 0.5, 0.5, Shape::SQUARE, &mut w);
                }
                if fr.lnoise > 0.0 {
                    v.noise ^= v.noise << 13;
                    v.noise ^= v.noise >> 17;
                    v.noise ^= v.noise << 5;
                    x += fr.lnoise * ((v.noise as i32) as f32 * (1.0 / 2_147_483_648.0));
                }
                let x = x * fr.in_gain;
                let g = if fr.pm_cut > 0.0 {
                    (v.g * exp2_fast(fr.pm_cut * o2)).min(fr.g_max)
                } else {
                    v.g
                };
                let f = if fr.filter == 0 {
                    v.ota(x, g, fr.k)
                } else {
                    v.svf(x, g, fr.damp, fr.filter - 1, fr.bp_gain)
                };
                *y += f * v.vca;
                v.vca += v.dvca;
                v.g += v.dg;
                v.inc1 += v.dinc1;
                v.inc2 += v.dinc2;
                v.pw1 += v.dpw1;
                v.pw2 += v.dpw2;
            }
        }
        let chorus = self.chorus.wet.value > 0.0 || self.chorus.wet.target > 0.0;
        for (i, &y) in buf.iter().take(n).enumerate() {
            let d = y - self.dc_x + self.dc_a * self.dc_y;
            self.dc_x = y;
            self.dc_y = d;
            let (l, r) = if chorus { self.chorus.run(d) } else { (d, d) };
            left[from + i] += l * self.gl;
            right[from + i] += r * self.gr;
            self.gl += self.dgl;
            self.gr += self.dgr;
        }
    }
}

impl Node for Analog {
    fn info(&self) -> &'static NodeInfo {
        &INFO
    }

    fn params(&self) -> &'static [ParamInfo] {
        PARAMS
    }

    fn set_param(&mut self, id: u16, value: f32) {
        let Some(p) = PARAMS.iter().find(|p| p.id == id) else {
            return;
        };
        if p.readonly {
            return;
        }
        self.values[id as usize] = p.clamp(value);
        match id {
            LEVEL | PAN => self.update_out(),
            CHORUS => self.update_chorus(),
            F_ATTACK | F_DECAY | F_SUSTAIN | F_RELEASE | A_ATTACK | A_DECAY | A_SUSTAIN
            | A_RELEASE | AGE => self.update_times(),
            VOICE_MODE => self.all_off(),
            _ => {}
        }
    }

    fn param(&self, id: u16) -> f32 {
        match PARAMS.iter().find(|p| p.id == id) {
            Some(_) => self.values[id as usize],
            None => f32::NAN,
        }
    }

    fn prepare(&mut self, rate: u32, _max_block: usize) {
        self.rate = rate.max(1);
        let r = self.fs();
        let cr = (self.rate / CTRL as u32).max(1);
        self.bend.set_time(cr, 4.0);
        self.out_l.set_time(cr, 10.0);
        self.out_r.set_time(cr, 10.0);
        self.chorus.wet.set_time(cr, 20.0);
        self.chorus.rate.set_time(cr, 50.0);
        self.chorus.mid.set_time(cr, 50.0);
        self.chorus.swing.set_time(cr, 50.0);
        self.chorus.pre.set(CH_PRE_HZ, r);
        self.chorus.post_l.set(CH_POST_HZ, r);
        self.chorus.post_r.set(CH_POST_HZ, r);
        self.chorus.follow_k = (-1.0 / (CH_FOLLOW_S * r)).exp();
        self.chorus.max_delay = (CH_BUF - 4) as f32;
        let dt = CTRL as f32 / r;
        self.drift_keep = (-dt / DRIFT_TAU_S).exp();
        self.drift_new = (1.0 - self.drift_keep * self.drift_keep).sqrt();
        self.dc_a = (-std::f32::consts::TAU * DC_HZ / r).exp();
        self.update_times();
        self.reset();
    }

    fn reset(&mut self) {
        self.n_held = 0;
        for v in self.voices.iter_mut() {
            v.silence();
            v.played = false;
            v.gliding = false;
            v.g = 0.0;
            v.g_to = 0.0;
            v.dg = 0.0;
            v.hz_key = f32::NAN;
        }
        self.next_card = 0;
        self.lfo_phase = 0.0;
        self.lfo_fade = 1.0;
        self.dc_x = 0.0;
        self.dc_y = 0.0;
        self.quiet = 0;
        self.chorus.clear();
        self.idle = true;
        self.bend.snap(self.bend_value);
        self.fr = self.frame();
        self.update_out();
        self.update_chorus();
        self.chorus.snap();
    }

    fn process(&mut self, ctx: &Ctx, events: &[Timed], left: &mut [f32], right: &mut [f32]) {
        if ctx.rate != self.rate {
            self.prepare(ctx.rate, left.len());
        }
        let len = left.len().min(right.len());
        let mut at = 0usize;
        let mut i = 0usize;
        while at < len {
            while i < events.len() && events[i].offset as usize <= at {
                let ev = events[i].ev;
                self.apply(&ev);
                i += 1;
            }
            let next = events.get(i).map_or(len, |e| (e.offset as usize).min(len));
            while at < next {
                let abs = ctx.frame + at as u64;
                if abs.is_multiple_of(CTRL) {
                    self.control();
                }
                let end = next.min(at + (CTRL - abs % CTRL) as usize);
                self.render(at, end, left, right);
                at = end;
            }
        }
        for e in &events[i..] {
            let ev = e.ev;
            self.apply(&ev);
        }
    }

    fn tail_frames(&self) -> u32 {
        (0.05 * self.rate as f32) as u32
    }

    fn set_tuning(&mut self, tuning: &Tuning) {
        if self.tuning == *tuning {
            return;
        }
        self.tuning.clone_from(tuning);
        for v in self.voices.iter_mut() {
            v.hz_key = f32::NAN;
        }
    }
}
