//! Squelch: a mono acid bass, built from the circuit analyses collected in
//! `docs/dev/research/acid-bass.md`.
//!
//! - **Oscillator.** A band-limited sawtooth (polyBLEP at the reset), or
//!   the square a limited-gain shaper makes from it: a soft threshold a
//!   little off centre, so the middle transition is an S-curve, the reset
//!   edge is sharp and the two halves differ (**Wave**).
//! - **Filter.** A four-pole diode ladder: four capacitor voltages, the
//!   input pair's current into the first (whose capacitor is half the
//!   others'), a diode pair between neighbours and one from the last to a
//!   fixed point, every pair's current through `tanh` of its voltage. Its
//!   linear part is Stinchcombe's transfer function exactly. It is solved
//!   with zero-delay feedback each sample, each stage's `tanh(d)/d` taken
//!   from the previous frame ([`Ladder`]). **Resonance** feeds the output
//!   back (k up to just past 17, where the bare ladder oscillates) through
//!   a high-pass, so low cutoffs ring far less; half the passband's loss
//!   is made up.
//! - **Envelopes.** The filter's envelope jumps up at each new note and
//!   falls exponentially (**Decay**, 200 ms on accented notes); **Env mod**
//!   sets its sweep and lowers the base as it widens. The volume envelope
//!   rises in 3 ms and falls over 3.5 s while held (**Amp decay**); letting
//!   go closes it in 8 ms.
//! - **Accent** (velocity from **Accent at**): the envelope into the
//!   amplifier through a 1.55 ms lag, and into the cutoff both directly and
//!   through a diode-charged 147 ms capacitor, mixed by Resonance as the
//!   original's dual pot does, so accents close together build up.
//! - **Slide.** A note played while another is held glides there over
//!   **Slide** (an exponential glide, three time constants) without new
//!   envelopes; letting go of it glides back to the key still held.
//! - **Drive** (an asymmetric smooth curve, after the filter), a 47 Hz
//!   high-pass (the original's weak low end) and a DC blocker.
//!
//! Below 64 kHz the oscillator, filter and drive run at twice the rate and
//! come down through a polyphase IIR half-band (flat to 0.4 of the rate,
//! -80 dB from 0.6). Pitch, envelopes, slide and cutoff run at a control
//! step of [`CTRL`] frames aligned to the absolute frame, with the cutoff,
//! pitch and level moving linearly across the step, so the output does not
//! depend on how the host cuts blocks. Pressure and per-note expression
//! are ignored. Nothing allocates, not even `prepare`.

use std::f32::consts::{PI, SQRT_2};

use mpfree_node::{Ctx, Kind, Node, NodeEvent, NodeInfo, ParamInfo, Smooth, Timed, Tuning};

use crate::params::Preset;
use crate::squelch_params::*;

/// Frames per control step.
pub const CTRL: u64 = 16;
/// Keys remembered while held, for gliding back when the top one is let go.
pub const HELD: usize = 16;

static INFO: NodeInfo = NodeInfo {
    id: "mpfree.squelch",
    name: "Squelch",
    kind: Kind::Instrument,
};

/// The envelope's sweep at Env mod 100, in octaves, and the share of it
/// the base drops by.
const ENV_OCT: f32 = 4.0;
const ENV_BASE_SHARE: f32 = 0.25;
/// The accent's sweep at Accent 100, in octaves.
const ACCENT_OCT: f32 = 2.0;
/// The accent's extra level into the amplifier at Accent 100.
const ACCENT_VCA: f32 = 1.5;
/// The accent's lag into the amplifier (47k and 0.033 µF).
const ACCENT_VCA_S: f32 = 0.001_55;
/// The accent sweep's capacitor: charging through the diode and 147k,
/// and emptying (our reading), and its weight against the direct pulse
/// (our reading: a single accent's swell about 0.7 of the pulse).
const SWEEP_CHARGE_S: f32 = 0.147;
const SWEEP_EMPTY_S: f32 = 0.2;
const SWEEP_WEIGHT: f32 = 2.0;
/// Feedback at Resonance 100: past the ladder's 17, so that with a note
/// through it, whose level compresses the input pair, it still rings at
/// the edge of oscillation, locked to the harmonic nearest the cutoff
/// (our reading).
const K_MAX: f32 = 18.5;
/// The high-pass in the resonance loop (our reading).
const LOOP_HP_HZ: f32 = 120.0;
/// The output's high-pass: -5 dB at 32 Hz.
const OUT_HP_HZ: f32 = 47.0;
const DC_HZ: f32 = 10.0;
/// The volume envelope's rise and the fall when a key is let go.
const ATTACK_S: f32 = 0.003;
const GATE_END_S: f32 = 0.008;
/// The oscillator into the ladder (the diodes' scale), and the ladder's
/// output into the amplifier: one note at the defaults peaks near -12 dBFS.
const IN_GAIN: f32 = 0.25;
const OUT_GAIN: f32 = 2.15;
/// The square's shaper: its gain on the saw, its threshold and its level.
const SQ_GAIN: f32 = 6.0;
const SQ_THRESHOLD: f32 = -0.1;
const SQ_LEVEL: f32 = 0.7;
/// The drive curve's rest point: off centre, so it adds even harmonics.
const DRIVE_BIAS: f32 = 0.25;

/// The half-band's all-pass coefficients (6 sections, transition 0.2 to
/// 0.3 of the higher rate: Valenzuela and Constantinides' elliptic
/// design, as in mpfree-fx's oversampler).
const HB_EVEN: [f32; 3] = [0.060_297_39, 0.412_590_7, 0.772_715_6];
const HB_ODD: [f32; 3] = [0.215_971_44, 0.604_358_6, 0.923_886_1];

/// `tanh` by its [3/2] Padé form, exact at 0 and reaching 1 at ±3.
#[inline(always)]
fn tanh_pade(x: f32) -> f32 {
    let x = x.clamp(-3.0, 3.0);
    let x2 = x * x;
    x * (27.0 + x2) / (27.0 + 9.0 * x2)
}

/// `tanh(x) / x`, the factor a diode pair scales its voltage by: the Padé
/// form inside ±3 and `1 / |x|` past it, one division either way.
#[inline(always)]
fn tanh_ratio(x: f32) -> f32 {
    let x2 = x * x;
    if x2 < 9.0 {
        (27.0 + x2) / (27.0 + 9.0 * x2)
    } else {
        1.0 / x.abs()
    }
}

/// The drive's curve: smooth everywhere, so a hard push makes harmonics
/// that fall away quickly instead of grit.
#[inline(always)]
fn drive_curve(u: f32) -> f32 {
    u / (1.0 + u * u).sqrt()
}

/// The sawtooth's correction at its reset (polyBLEP): `t` the phase, `dt`
/// its step, `rdt` the step's reciprocal.
#[inline(always)]
fn blep(t: f32, dt: f32, rdt: f32) -> f32 {
    if t < dt {
        let x = t * rdt;
        x + x - x * x - 1.0
    } else if t > 1.0 - dt {
        let x = (t - 1.0) * rdt;
        x * x + x + x + 1.0
    } else {
        0.0
    }
}

/// The diode ladder (see the module docs). `s` are the trapezoidal
/// integrators' states, `v` the capacitor voltages last sample, `loop_s`
/// the feedback high-pass's state and `u` the input pair's last input; the
/// rest is the solve [`Ladder::set`] works out for the samples after it.
#[derive(Clone, Copy, Debug, Default)]
pub struct Ladder {
    s: [f32; 4],
    v: [f32; 4],
    loop_s: f32,
    u: f32,
    g1: f32,
    g2: f32,
    g3: f32,
    r1: f32,
    r2: f32,
    r3: f32,
    r4: f32,
    al2: f32,
    al3: f32,
    al4: f32,
    c: [f32; 4],
    kh: f32,
    h: f32,
    ru: f32,
}

impl Ladder {
    /// Works out the solve for the next samples: each diode pair's slope
    /// (`tanh(d) / d`) from the voltages now, with `g` the integrators'
    /// prewarped gain, `k` the feedback and `h` the loop high-pass's
    /// `1 / (1 + g)`. The divisions all happen here.
    #[inline(always)]
    pub fn set(&mut self, g: f32, k: f32, h: f32) {
        let [v1, v2, v3, v4] = self.v;
        let g0 = g * tanh_ratio(self.u);
        let g1 = g * tanh_ratio(v1 - v2);
        let g2 = g * tanh_ratio(v2 - v3);
        let g3 = g * tanh_ratio(v3 - v4);
        let g4 = g * tanh_ratio(v4);
        // Eliminate from the top: v4 = al4 v3 + be4, and so on down, then
        // v1 = c1 u + d1, so v4 = c4 u + d4.
        let r4 = 1.0 / (1.0 + g3 + g4);
        let al4 = g3 * r4;
        let r3 = 1.0 / (1.0 + g2 + g3 * (1.0 - al4));
        let al3 = g2 * r3;
        let r2 = 1.0 / (1.0 + g1 + g2 * (1.0 - al3));
        let al2 = g1 * r2;
        let r1 = 1.0 / (1.0 + 2.0 * g1 * (1.0 - al2));
        let c1 = 2.0 * g0 * r1;
        let c2 = al2 * c1;
        let c3 = al3 * c2;
        let c4 = al4 * c3;
        let kh = k * h;
        *self = Ladder {
            g1,
            g2,
            g3,
            r1,
            r2,
            r3,
            r4,
            al2,
            al3,
            al4,
            c: [c1, c2, c3, c4],
            kh,
            h,
            ru: 1.0 / (1.0 + kh * c4),
            ..*self
        };
    }

    /// One sample through the solve [`Ladder::set`] worked out: `x` in,
    /// the last capacitor's voltage out.
    #[inline(always)]
    pub fn step(&mut self, x: f32) -> f32 {
        let [s1, s2, s3, s4] = self.s;
        let be4 = s4 * self.r4;
        let be3 = (s3 + self.g3 * be4) * self.r3;
        let be2 = (s2 + self.g2 * be3) * self.r2;
        let d1 = (s1 + 2.0 * self.g1 * be2) * self.r1;
        let d2 = self.al2 * d1 + be2;
        let d3 = self.al3 * d2 + be3;
        let d4 = self.al4 * d3 + be4;
        // The feedback through its high-pass: hp = h (v4 - loop_s).
        let u = (x - self.kh * (d4 - self.loop_s)) * self.ru;
        let [c1, c2, c3, c4] = self.c;
        let n1 = c1 * u + d1;
        let n2 = c2 * u + d2;
        let n3 = c3 * u + d3;
        let n4 = c4 * u + d4;
        let hp = self.h * (n4 - self.loop_s);
        self.loop_s = 2.0 * (n4 - hp) - self.loop_s;
        self.s = [2.0 * n1 - s1, 2.0 * n2 - s2, 2.0 * n3 - s3, 2.0 * n4 - s4];
        self.v = [n1, n2, n3, n4];
        self.u = u;
        n4
    }

    /// [`Ladder::set`] then [`Ladder::step`]: a fresh solve every sample.
    pub fn run(&mut self, x: f32, g: f32, k: f32, h: f32) -> f32 {
        self.set(g, k, h);
        self.step(x)
    }

    /// Every state under `eps` set to exactly zero.
    fn flush(&mut self, eps: f32) {
        for x in self
            .s
            .iter_mut()
            .chain(self.v.iter_mut())
            .chain([&mut self.loop_s, &mut self.u])
        {
            if x.abs() < eps {
                *x = 0.0;
            }
        }
    }
}

/// One first-order all-pass section of the half-band, `y = a (x - y1) + x1`.
#[derive(Clone, Copy, Debug, Default)]
struct Ap {
    x1: f32,
    y1: f32,
}

impl Ap {
    #[inline(always)]
    fn run(&mut self, a: f32, x: f32) -> f32 {
        let y = a * (x - self.y1) + self.x1;
        self.x1 = x;
        self.y1 = y;
        y
    }
}

/// The mono acid bass. See the module docs.
pub struct Squelch {
    rate: u32,
    /// Oversampling: 2 below 64 kHz, else 1.
    os: usize,
    values: [f32; ID_SPACE],
    /// Keys held, oldest first.
    held: [u8; HELD],
    n_held: usize,
    /// The note sounding, whether its gate is open, and whether this gate
    /// is accented.
    note: u8,
    gate: bool,
    accent: bool,
    /// Pitch now (fractional note, gliding) and whether it glides to the
    /// note or jumps.
    pitch: f32,
    gliding: bool,
    bend: Smooth,
    bend_value: f32,
    /// The oscillator: phase (0 to 1), its step per sample now, the
    /// step's change per sample across this control step, and the step's
    /// reciprocal for the polyBLEP.
    phase: f32,
    inc: f32,
    inc_to: f32,
    /// The pitch `inc_to` was worked out for (NaN: work it out again).
    inc_pitch: f32,
    dinc: f32,
    rinc: f32,
    wave: Smooth,
    /// Envelopes: the filter's (0 to 1), the volume's, whether the volume
    /// is rising, and the gate's own fall after a key is let go.
    meg: f32,
    veg: f32,
    rising: bool,
    gate_level: f32,
    /// The accent: its lag into the amplifier, and the sweep's capacitor.
    acc_lag: f32,
    sweep: f32,
    /// The ladder's integrator gain now and its change per frame, its
    /// feedback, and the loop high-pass's `1 / (1 + g)`.
    g: f32,
    g_to: f32,
    dg: f32,
    k: f32,
    loop_h: f32,
    ladder: Ladder,
    /// The amplifier's gain now and its change per sample.
    vca: f32,
    vca_to: f32,
    dvca: f32,
    /// The output high-pass: state and `1 / (1 + g)`.
    ohp_s: f32,
    ohp_h: f32,
    /// Drive: input gain and output scale (0 gain: off).
    drive_k: f32,
    drive_n: f32,
    /// The half-band's sections.
    hb_even: [Ap; 3],
    hb_odd: [Ap; 3],
    /// DC blocker after the drive.
    dc_x: f32,
    dc_y: f32,
    dc_a: f32,
    /// Level and pan: smoothed at the control rate, then a straight line
    /// per frame across each step (gain now, its target, its step).
    out_l: Smooth,
    out_r: Smooth,
    gl: f32,
    gr: f32,
    dgl: f32,
    dgr: f32,
    /// Per control step: the filter envelope's fall (normal and
    /// accented), the volume's fall, the slide's share of the gap, the
    /// accent's lag, and the sweep capacitor's charging share and fall.
    meg_fall: f32,
    meg_fall_acc: f32,
    veg_fall: f32,
    slide_a: f32,
    lag_a: f32,
    charge_a: f32,
    empty_m: f32,
    /// Nothing to hear until the next note: render adds nothing.
    idle: bool,
    tuning: Tuning,
}

impl Squelch {
    pub fn new(rate: u32) -> Squelch {
        let mut values = [0.0; ID_SPACE];
        for p in PARAMS {
            values[p.id as usize] = p.default;
        }
        let mut s = Squelch {
            rate,
            os: 2,
            values,
            held: [0; HELD],
            n_held: 0,
            note: 48,
            gate: false,
            accent: false,
            pitch: 48.0,
            gliding: false,
            bend: Smooth::new(0.0),
            bend_value: 0.0,
            phase: 0.0,
            inc: 0.0,
            inc_to: 0.0,
            inc_pitch: f32::NAN,
            dinc: 0.0,
            rinc: 1.0,
            wave: Smooth::new(0.0),
            meg: 0.0,
            veg: 0.0,
            rising: false,
            gate_level: 0.0,
            acc_lag: 0.0,
            sweep: 0.0,
            g: 0.0,
            g_to: 0.0,
            dg: 0.0,
            k: 0.0,
            loop_h: 1.0,
            ladder: Ladder::default(),
            vca: 0.0,
            vca_to: 0.0,
            dvca: 0.0,
            ohp_s: 0.0,
            ohp_h: 1.0,
            drive_k: 0.0,
            drive_n: 1.0,
            hb_even: [Ap::default(); 3],
            hb_odd: [Ap::default(); 3],
            dc_x: 0.0,
            dc_y: 0.0,
            dc_a: 0.0,
            out_l: Smooth::new(0.0),
            out_r: Smooth::new(0.0),
            gl: 0.0,
            gr: 0.0,
            dgl: 0.0,
            dgr: 0.0,
            meg_fall: 1.0,
            meg_fall_acc: 1.0,
            veg_fall: 1.0,
            slide_a: 1.0,
            lag_a: 1.0,
            charge_a: 1.0,
            empty_m: 1.0,
            idle: true,
            tuning: Tuning::standard(),
        };
        s.prepare(rate, mpfree_node::MAX_BLOCK);
        s
    }

    /// Loads a factory preset: defaults, then the preset's values.
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

    /// One while anything can be heard, else none.
    pub fn active_voices(&self) -> usize {
        usize::from(!self.idle)
    }

    /// The keys held now.
    pub fn keys_held(&self) -> usize {
        self.n_held
    }

    /// The filter's resonant frequency now, in Hz (for tests and pictures).
    pub fn cutoff_hz(&self) -> f32 {
        let fs = (self.rate as usize * self.os) as f32;
        self.g.atan() * fs / PI * SQRT_2
    }

    /// The pitch now, as a fractional note.
    pub fn pitch(&self) -> f32 {
        self.pitch
    }

    #[inline]
    fn v(&self, id: u16) -> f32 {
        self.values[id as usize]
    }

    fn fs(&self) -> f32 {
        (self.rate as usize * self.os) as f32
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

    /// Level and pan straight to where they are set.
    fn snap_out(&mut self) {
        self.out_l.snap(self.out_l.target);
        self.out_r.snap(self.out_r.target);
        self.gl = self.out_l.value;
        self.gr = self.out_r.value;
        self.dgl = 0.0;
        self.dgr = 0.0;
    }

    fn update_drive(&mut self) {
        let d = self.v(DRIVE) / 100.0;
        if d <= 0.0 {
            self.drive_k = 0.0;
            self.drive_n = 1.0;
            return;
        }
        self.drive_k = 1.0 + 100.0 * d * d;
        // A signal at 0.08 (a note's usual swing) comes out at 0.08
        // whatever the drive.
        let r = 0.08;
        self.drive_n = r / (drive_curve(self.drive_k * r + DRIVE_BIAS) - drive_curve(DRIVE_BIAS));
    }

    /// The control step's falls and shares for the times set now.
    fn update_times(&mut self) {
        let dt = CTRL as f32 / self.rate as f32;
        let fall = |secs: f32, db: f32| (-dt * db / 20.0 * std::f32::consts::LN_10 / secs).exp();
        let share = |tau: f32| 1.0 - (-dt / tau).exp();
        // The filter envelope falls to a tenth (20 dB) in its decay, the
        // volume 60 dB in Amp decay; a slide is three time constants.
        self.meg_fall = fall(self.v(DECAY) * 0.001, 20.0);
        self.meg_fall_acc = fall(self.v(ACCENT_DECAY) * 0.001, 20.0);
        self.veg_fall = fall(self.v(AMP_DECAY) * 0.001, 60.0);
        self.slide_a = share(self.v(SLIDE) * 0.001 / 3.0);
        self.lag_a = share(ACCENT_VCA_S);
        self.charge_a = share(SWEEP_CHARGE_S);
        self.empty_m = (-dt / SWEEP_EMPTY_S).exp();
    }

    fn update_wave(&mut self) {
        self.wave.target = self.v(WAVE);
        if self.idle {
            self.wave.snap(self.wave.target);
        }
    }

    /// The pitch a note asks for now (bend, transpose, fine).
    fn note_pitch(&self, note: u8) -> f32 {
        f32::from(note)
            + self.v(TRANSPOSE)
            + self.v(FINE) / 100.0
            + self.bend.value * self.v(BEND_RANGE)
    }

    /// The oscillator's step per sample for `pitch`.
    fn step_for(&self, pitch: f32) -> f32 {
        let fs = self.fs();
        let hz = self.tuning.hz(pitch);
        let hz = if hz.is_finite() { hz } else { 0.0 };
        hz.clamp(1.0, fs * 0.45) / fs
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
        match self.held[..self.n_held].iter().position(|&n| n == note) {
            Some(i) => {
                self.held.copy_within(i + 1..self.n_held, i);
                self.n_held -= 1;
                true
            }
            None => false,
        }
    }

    fn note_on(&mut self, note: u8, velocity: u8) {
        let legato = self.n_held > 0 && self.gate;
        if self.idle {
            self.bend.snap(self.bend_value);
        }
        self.push(note);
        self.note = note;
        self.idle = false;
        if legato {
            // A slide: glide there, the envelopes carry on.
            self.gliding = true;
            return;
        }
        self.gliding = false;
        self.pitch = self.note_pitch(note);
        self.inc = self.step_for(self.pitch);
        self.inc_to = self.inc;
        self.inc_pitch = self.pitch;
        self.dinc = 0.0;
        self.rinc = 1.0 / self.inc;
        self.gate = true;
        self.accent = f32::from(velocity) >= self.v(ACCENT_AT);
        self.meg = 1.0;
        // The volume rises from where it is heard now.
        self.veg *= self.gate_level;
        self.gate_level = 1.0;
        self.rising = true;
    }

    fn note_off(&mut self, note: u8) {
        if !self.remove(note) || note != self.note {
            return;
        }
        if self.n_held > 0 {
            // Back to the key still held, gliding.
            self.note = self.held[self.n_held - 1];
            self.gliding = true;
        } else {
            self.gate = false;
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
            NodeEvent::AllOff => {
                self.n_held = 0;
                self.gate = false;
            }
            NodeEvent::Param { id, value } => self.set_param(id, value),
            NodeEvent::Pressure { .. } | NodeEvent::Expression { .. } => {}
        }
    }

    /// One control step: pitch and slide, the envelopes and the accent,
    /// then the cutoff and level the next `CTRL` frames move toward.
    fn control(&mut self) {
        if self.idle {
            return;
        }
        let rate = self.rate as f32;
        let dt = CTRL as f32 / rate;
        let per = (CTRL as usize * self.os) as f32;
        // Last step's ramps have arrived: land exactly.
        self.inc = self.inc_to;
        self.g = self.g_to;
        self.vca = self.vca_to;
        self.bend.next();
        self.wave.next();
        self.gl = self.out_l.value;
        self.gr = self.out_r.value;
        self.dgl = (self.out_l.next() - self.gl) / CTRL as f32;
        self.dgr = (self.out_r.next() - self.gr) / CTRL as f32;

        // Pitch: a jump has already happened at the note; a slide glides.
        let target = self.note_pitch(self.note);
        if self.gliding {
            self.pitch += (target - self.pitch) * self.slide_a;
            if (target - self.pitch).abs() < 1e-4 {
                self.pitch = target;
                self.gliding = false;
            }
        } else {
            self.pitch = target;
        }
        if self.pitch != self.inc_pitch {
            self.inc_to = self.step_for(self.pitch);
            self.inc_pitch = self.pitch;
        }
        self.dinc = (self.inc_to - self.inc) / per;
        self.rinc = 1.0 / self.inc_to.max(self.inc).max(1e-6);

        // The envelopes.
        self.meg *= if self.accent {
            self.meg_fall_acc
        } else {
            self.meg_fall
        };
        if self.rising {
            self.veg += dt / ATTACK_S;
            if self.veg >= 1.0 {
                self.veg = 1.0;
                self.rising = false;
            }
        } else {
            self.veg *= self.veg_fall;
        }
        if !self.gate {
            self.gate_level = (self.gate_level - dt / GATE_END_S).max(0.0);
        }
        // The accent: the envelope while accented, lagged into the
        // amplifier and charging the sweep's capacitor through its diode.
        let acc = if self.accent { self.meg } else { 0.0 };
        self.acc_lag += (acc - self.acc_lag) * self.lag_a;
        if acc > self.sweep {
            self.sweep += (acc - self.sweep) * self.charge_a;
        } else {
            self.sweep *= self.empty_m;
        }

        // The cutoff, in octaves.
        let r = self.v(RESONANCE) / 100.0;
        let depth = self.v(ENV_MOD) / 100.0 * ENV_OCT;
        let a = self.v(ACCENT) / 100.0;
        let oct = self.v(CUTOFF).log2() - ENV_BASE_SHARE * depth
            + depth * self.meg
            + a * ACCENT_OCT * ((1.0 - r) * acc + r * SWEEP_WEIGHT * self.sweep)
            + self.v(KEY_TRACK) / 100.0 * (self.pitch - 48.0) / 12.0;
        let fs = self.fs();
        let fc = oct.exp2().clamp(20.0, (fs * 0.3).min(20_000.0));
        self.g_to = (PI * fc / SQRT_2 / fs).tan();
        self.dg = (self.g_to - self.g) / CTRL as f32;
        self.k = K_MAX * r;

        // The level: the volume envelope plus the accent, the gate's fall,
        // and half the resonance's passband loss made up.
        let comp = (1.0 + self.k).sqrt();
        self.vca_to =
            (self.veg + a * ACCENT_VCA * self.acc_lag) * self.gate_level * comp * OUT_GAIN;
        self.dvca = (self.vca_to - self.vca) / per;

        // Once the gate has closed and the tails have died, stop.
        if !self.gate && self.gate_level == 0.0 && self.vca == 0.0 {
            let eps = 1e-6;
            self.ladder.flush(1e-12);
            let tails = [
                self.ohp_s,
                self.dc_x,
                self.dc_y,
                self.hb_even[2].y1,
                self.hb_odd[2].y1,
            ];
            if tails.iter().all(|x| x.abs() < eps) {
                self.idle = true;
                self.ohp_s = 0.0;
                self.dc_x = 0.0;
                self.dc_y = 0.0;
                self.hb_even = [Ap::default(); 3];
                self.hb_odd = [Ap::default(); 3];
                self.meg = 0.0;
                self.acc_lag = 0.0;
                self.sweep = 0.0;
                self.veg = 0.0;
                self.dvca = 0.0;
                self.snap_out();
                self.wave.snap(self.wave.target);
            }
        }
    }

    /// One sample at the inner rate: oscillator, ladder, amplifier, the
    /// output high-pass and the drive.
    #[inline(always)]
    fn sample(&mut self) -> f32 {
        let t = self.phase;
        let dt = self.inc;
        let saw = 2.0 * t - 1.0 - blep(t, dt, self.rinc);
        self.phase += dt;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        self.inc += self.dinc;
        let w = self.wave.value;
        let osc = if w <= 0.0 {
            saw
        } else {
            let sq = SQ_LEVEL * tanh_pade(SQ_GAIN * (saw - SQ_THRESHOLD));
            saw + w * (sq - saw)
        };
        let y = self.ladder.step(osc * IN_GAIN);
        let y = y * self.vca;
        self.vca += self.dvca;
        // The output's high-pass (one pole, trapezoidal).
        let hp = (y - self.ohp_s) * self.ohp_h;
        self.ohp_s = 2.0 * (y - hp) - self.ohp_s;
        if self.drive_k > 0.0 {
            self.drive_n * (drive_curve(self.drive_k * hp + DRIVE_BIAS) - drive_curve(DRIVE_BIAS))
        } else {
            hp
        }
    }

    fn render(&mut self, from: usize, to: usize, left: &mut [f32], right: &mut [f32]) {
        if self.idle {
            return;
        }
        for i in from..to {
            // The diodes' slopes and the solve once a frame, for both
            // samples of it.
            self.ladder.set(self.g, self.k, self.loop_h);
            self.g += self.dg;
            let y = if self.os == 2 {
                let a = self.sample();
                let b = self.sample();
                // The half-band: the later sample through one branch, the
                // earlier through the other.
                let mut e = b;
                let mut o = a;
                for (s, c) in self.hb_even.iter_mut().zip(HB_EVEN) {
                    e = s.run(c, e);
                }
                for (s, c) in self.hb_odd.iter_mut().zip(HB_ODD) {
                    o = s.run(c, o);
                }
                0.5 * (e + o)
            } else {
                self.sample()
            };
            let d = y - self.dc_x + self.dc_a * self.dc_y;
            self.dc_x = y;
            self.dc_y = d;
            left[i] += d * self.gl;
            right[i] += d * self.gr;
            self.gl += self.dgl;
            self.gr += self.dgr;
        }
    }
}

impl Node for Squelch {
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
            DRIVE => self.update_drive(),
            WAVE => self.update_wave(),
            DECAY | ACCENT_DECAY | AMP_DECAY | SLIDE => self.update_times(),
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
        self.os = if self.rate < 64_000 { 2 } else { 1 };
        let r = self.rate as f32;
        let fs = self.fs();
        let cr = (self.rate / CTRL as u32).max(1);
        self.bend.set_time(cr, 4.0);
        self.wave.set_time(cr, 10.0);
        self.out_l.set_time(cr, 10.0);
        self.out_r.set_time(cr, 10.0);
        self.loop_h = 1.0 / (1.0 + (PI * LOOP_HP_HZ / fs).tan());
        self.ohp_h = 1.0 / (1.0 + (PI * OUT_HP_HZ / fs).tan());
        self.dc_a = (-std::f32::consts::TAU * DC_HZ / r).exp();
        self.update_times();
        self.reset();
    }

    fn reset(&mut self) {
        self.n_held = 0;
        self.gate = false;
        self.accent = false;
        self.gliding = false;
        self.phase = 0.0;
        self.inc = self.step_for(self.pitch);
        self.inc_to = self.inc;
        self.inc_pitch = self.pitch;
        self.dinc = 0.0;
        self.rinc = 1.0 / self.inc;
        self.meg = 0.0;
        self.veg = 0.0;
        self.rising = false;
        self.gate_level = 0.0;
        self.acc_lag = 0.0;
        self.sweep = 0.0;
        self.ladder = Ladder::default();
        self.vca = 0.0;
        self.vca_to = 0.0;
        self.dvca = 0.0;
        self.g = (PI * 400.0 / SQRT_2 / self.fs()).tan();
        self.g_to = self.g;
        self.dg = 0.0;
        self.k = 0.0;
        self.ohp_s = 0.0;
        self.hb_even = [Ap::default(); 3];
        self.hb_odd = [Ap::default(); 3];
        self.dc_x = 0.0;
        self.dc_y = 0.0;
        self.idle = true;
        self.bend.snap(self.bend_value);
        self.update_out();
        self.update_drive();
        self.update_wave();
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
        (0.1 * self.rate as f32) as u32
    }

    fn set_tuning(&mut self, tuning: &Tuning) {
        if self.tuning == *tuning {
            return;
        }
        self.tuning.clone_from(tuning);
        self.inc_pitch = f32::NAN;
    }
}
