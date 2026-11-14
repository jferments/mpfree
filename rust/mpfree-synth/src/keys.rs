//! Keys: an electric piano, two physical models behind one set of pages,
//! built from the measurements in `docs/dev/research/electric-piano.md`:
//! the vibrating part moves almost as a sine, and the pickup makes the tone.
//!
//! - **Tine**: a hammer strikes a steel tine. Each voice is a small modal
//!   bank (two-pole resonators): the fundamental, which rings for seconds,
//!   and the bending modes at 7.11 and 20.25 times it, which give the bell
//!   of the attack. The tine moves in a magnetic pickup's bell-shaped field
//!   ([`flux`]): centred (**Voicing** 0) the octave above the note
//!   dominates, at the curve's inflection (100) it is nearly pure, and in
//!   between a hard hit swings it across the curve into the growl players
//!   call bark (**Bark**; low notes swing wider).
//! - **Reed**: a hammer strikes a steel reed in an electrostatic pickup
//!   ([`capacitance`], `1 / (1 - y)`): its decay in two stages, a beating
//!   pair on its second mode, an RC high-pass and an asymmetric preamp
//!   with the amplitude tremolo inside it.
//!
//! The pickup is a Hammerstein model: the curve of the displacement, then
//! its change (Faraday's law, or a capacitor's current). The hammer is a
//! half-sine contact of unit area whose length follows a nonlinear contact
//! law ([`contact_secs`]). The voices sum to a mono bus, as in the
//! instrument, then key noise, a tilt **Tone**, the **Drive** (or the
//! reed's preamp) and the **Tremolo** (pan or amplitude) make the stereo
//! output.
//!
//! Notes: a key held rings with Decay (higher notes shorter); letting go
//! damps it over Release. Playing a note that is still ringing strikes the
//! same voice again, adding to what sounds. Past the Voices limit the
//! oldest note (released ones first) fades out over 3 ms. Pressure is
//! ignored.
//!
//! Pitch (transpose, fine, bend), the tremolo and retuning run at a control
//! rate of one step every [`CTRL`] frames aligned to the absolute frame, so
//! the output does not depend on how the host cuts blocks. Nothing
//! allocates, not even `prepare`.

use mpfree_node::{Ctx, Kind, Node, NodeEvent, NodeInfo, ParamInfo, Smooth, Timed, Tuning};

use crate::dsp::{drive_n, drive_norm, Rng};
use crate::expr::{expr_coef, NoteExprs, VoiceExpr};
use crate::keys_params::*;
use crate::params::Preset;

/// Frames per control step.
pub const CTRL: u64 = 16;
/// The most notes sounding at once (the Voices parameter's top).
pub const MAX_POLY: usize = 16;
/// Voice slots: the notes plus room for stolen voices to fade out.
pub const POOL: usize = MAX_POLY + 4;
/// Resonators per voice (a tine uses three, a reed five).
pub const MODES: usize = 5;
const KILL_MS: f32 = 3.0;
/// A voice whose resonators fall under this (after 256 frames) ends.
const SILENT: f32 = 2e-5;
/// The mono bus's scale into the output.
const VOICE_GAIN: f32 = 0.17;
/// The level the drive and the reed's preamp treat as full: a hard chord
/// on the bus, where one full-velocity note is near 1.
const DRIVE_REF: f32 = 2.0;
/// The longest hammer contact, in seconds.
const MAX_CONTACT: f32 = 0.008;

static INFO: NodeInfo = NodeInfo {
    id: "mpfree.keys",
    name: "Keys",
    kind: Kind::Instrument,
};

/// A resonator: its ratio to the note, its level (scaled by Bell when
/// `bell`), and its decay as a share of the note's.
#[derive(Clone, Copy, Debug)]
pub struct Mode {
    pub ratio: f32,
    pub level: f32,
    pub decay: f32,
    pub bell: bool,
}

const fn m(ratio: f32, level: f32, decay: f32, bell: bool) -> Mode {
    Mode {
        ratio,
        level,
        decay,
        bell,
    }
}

/// A tine: the fundamental, which rings for seconds, then the
/// cantilever's second and third bending modes at 7.11 and 20.25 times it
/// (the ratios measured on a tine, sharp of a free bar's 6.27 and 17.55
/// because of the tuning spring's mass), heard only in the attack.
pub const TINE: [Mode; MODES] = [
    m(1.0, 1.0, 1.0, false),
    m(7.11, 0.7, 0.03, true),
    m(20.25, 0.35, 0.01, true),
    m(0.0, 0.0, 0.0, false),
    m(0.0, 0.0, 0.0, false),
];
/// A reed: the fundamental in two parts, a long one and a quick one, so
/// its decay is not a single exponential (measured on reed pianos); the
/// second bending mode as a pair 0.1 % apart, so it beats; the third.
pub const REED: [Mode; MODES] = [
    m(1.0, 0.75, 1.0, false),
    m(1.0, 0.25, 0.12, false),
    m(6.27, 0.45, 0.15, true),
    m(6.27 * 1.001, 0.3, 0.2, true),
    m(17.55, 0.2, 0.04, true),
];

/// Decay (T60, seconds) at `hz` for a Decay of `decay_ms` at middle C:
/// falling as `hz^-0.75`, the slope of the Q factors measured on a tine
/// piano (T60 about 27 s at E-flat 2, 11 s at E-flat 4, 3 s at E-flat 6).
pub fn decay_for(decay_ms: f32, hz: f32) -> f32 {
    decay_ms * 0.001 * (261.63 / hz.max(1.0)).powf(0.75)
}

/// How far a full-velocity hit moves a tine or reed at `hz`, against
/// middle C's: low notes swing wider, so they bark sooner.
pub fn register_swing(hz: f32) -> f32 {
    (261.63 / hz.max(1.0)).sqrt().clamp(0.5, 1.6)
}

/// The magnetic pickup's flux against the tine's position `u` (in the
/// field's own width): a bell, highest with the tine on the pickup's axis.
#[inline]
pub fn flux(u: f32) -> f32 {
    1.0 / (1.0 + u * u)
}

/// [`flux`]'s slope.
#[inline]
fn flux_slope(u: f32) -> f32 {
    let d = 1.0 + u * u;
    -2.0 * u / (d * d)
}

/// The reed's limit: it never reaches the plate.
const REED_LIMIT: f32 = 0.9;

/// The electrostatic pickup's capacitance against the reed's position `y`
/// (as a share of the gap to the plate): `1 / (1 - y)`, the reed held off
/// the plate by a soft limit.
#[inline]
pub fn capacitance(y: f32) -> f32 {
    let s = y / (1.0 + y.abs() / REED_LIMIT);
    1.0 / (1.0 - s)
}

/// [`capacitance`]'s slope.
#[inline]
fn capacitance_slope(y: f32) -> f32 {
    let a = 1.0 + y.abs() / REED_LIMIT;
    let s = y / a;
    let ds = 1.0 / (a * a);
    ds / ((1.0 - s) * (1.0 - s))
}

/// Where a voice rests on its pickup's curve and how far a displacement
/// of 1 moves it there, for a model, Voicing and Bark (0 to 100) and a
/// key's own small difference `key` (-1 to 1).
pub fn pickup_shape(reed: bool, voicing: f32, bark: f32, key: f32) -> (f32, f32) {
    let v = (voicing / 100.0 + 0.04 * key).clamp(0.0, 1.0);
    let b = bark / 100.0;
    if reed {
        (0.4 * (1.0 - v), 0.2 + 0.6 * b)
    } else {
        // From the axis (0) to the flux curve's inflection, 1/sqrt(3),
        // where it is straightest.
        (0.6 * v, 0.2 + 0.9 * b * b)
    }
}

/// The hammer's contact time in seconds, from Hardness (0 to 100) and the
/// velocity (0 to 1). A nonlinear contact (Hunt and Crossley's law, force
/// growing as compression^2.8) is shorter the faster it lands, about as
/// `v^-0.47`; Hardness sets the time of a full-velocity hit, 4.5 ms (soft)
/// to 0.4 ms (hard). Never more than half the note's period.
pub fn contact_secs(hardness: f32, vel: f32, hz: f32) -> f32 {
    let soft = 1.0 - (hardness / 100.0).clamp(0.0, 1.0);
    let full = 0.0004 + 0.0041 * soft * soft;
    (full * vel.max(0.08).powf(-0.47))
        .min(MAX_CONTACT)
        .min(0.5 / hz.max(1.0))
}

/// The tremolo's left and right gains for **Tremolo** `depth` (0 to 100)
/// at `phase` (0 to 1) of its cycle. Pan follows a rounded square, as the
/// tine pianos' stereo vibrato did (a square tremolo that went stereo);
/// Amp (`amp`) a sine.
pub fn tremolo_gains(depth: f32, amp: bool, phase: f32) -> (f32, f32) {
    let d = depth / 100.0;
    if d <= 0.0 {
        return (1.0, 1.0);
    }
    let s = (std::f32::consts::TAU * phase).sin();
    if amp {
        let g = 1.0 - d * 0.5 * (1.0 + s);
        (g, g)
    } else {
        let sq = s * 2.5 / (1.0 + 1.5 * s.abs());
        let p = d * sq;
        // Equal power, never louder on one side than the dry sound.
        let n = 1.0 / (1.0 + d).sqrt();
        ((1.0 - p).max(0.0).sqrt() * n, (1.0 + p).max(0.0).sqrt() * n)
    }
}

/// The level a velocity (0 to 1) strikes at.
#[inline]
pub fn strike_amp(vel: f32) -> f32 {
    0.06 + 0.94 * vel * vel.sqrt()
}

/// A key's own small difference (-1 to 1), the same every time it plays:
/// no two tines are voiced quite alike.
pub fn key_spread(note: u8) -> f32 {
    let mut h = u32::from(note).wrapping_mul(0x9E37_79B9);
    h ^= h >> 15;
    h = h.wrapping_mul(0x85EB_CA6B);
    h ^= h >> 13;
    (h & 0xFFFF) as f32 / 32767.5 - 1.0
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
enum State {
    #[default]
    Free,
    On,
    Release,
    Kill,
}

#[derive(Clone, Copy, Debug, Default)]
struct Voice {
    state: State,
    note: u8,
    age: u64,
    /// Pitch the voice is tuned to now (fractional MIDI note), or NaN to
    /// force a retune.
    tuned: f32,
    /// Its note's own slide.
    ex: VoiceExpr,
    kill: f32,
    kill_step: f32,
    /// Level follower on the resonators, to end the voice once silent.
    follow: f32,
    frames: u32,
    y1: [f32; MODES],
    y2: [f32; MODES],
    c1: [f32; MODES],
    c2: [f32; MODES],
    b0: [f32; MODES],
    /// This strike's small random detuning and level of each mode.
    jit_ratio: [f32; MODES],
    jit_level: [f32; MODES],
    /// Frames into the hammer contact, its length and its height.
    pulse_at: u32,
    pulse_len: u32,
    pulse_amp: f32,
    /// The pickup: rest point, swing per unit displacement, output scale,
    /// and the curve's last value (the pickup hears its change).
    rest: f32,
    swing: f32,
    norm: f32,
    last: f32,
}

/// The electric piano. See the module docs.
pub struct Keys {
    rate: u32,
    values: [f32; ID_SPACE],
    voices: [Voice; POOL],
    clock: u64,
    rng: Rng,
    bend: Smooth,
    bend_value: f32,
    out_l: Smooth,
    out_r: Smooth,
    /// Tone: the low and high shelves' gains, and the split's state.
    tone_lo: Smooth,
    tone_hi: Smooth,
    tone_a: f32,
    tone_lp: f32,
    /// The reed pickup's RC high-pass: coefficient and state.
    hp_a: f32,
    hp_x: f32,
    hp_y: f32,
    /// Drive: input gain, output scale and blend (tine); the reed's
    /// preamp curve's input gain.
    drive_k: f32,
    drive_n: f32,
    drive_amt: f32,
    pre_k: f32,
    /// The tremolo's phase (0 to 1), and its gains now and their step per
    /// frame toward the next control step's.
    trem_phase: f32,
    trem_l: f32,
    trem_r: f32,
    trem_dl: f32,
    trem_dr: f32,
    /// Key noise: its envelope, the envelope's fall per frame, and two
    /// one-pole low-passes.
    thump: f32,
    thump_fall: f32,
    thump_a: f32,
    thump_lp1: f32,
    thump_lp2: f32,
    /// How notes become frequencies.
    tuning: Tuning,
    exprs: NoteExprs,
    expr_c: f32,
}

/// The reed preamp's rest point on its curve: transistors are not
/// symmetric, so it adds even harmonics as it is pushed.
const PRE_REST: f32 = 0.35;
/// The reed model's make-up gain, so both models sit at the same level.
const REED_GAIN: f32 = 1.5;

/// The reed piano's preamp: [`pickup`]'s soft curve around a rest point,
/// scaled so small signals pass unchanged.
#[inline]
fn preamp(x: f32, k: f32) -> f32 {
    let c = |u: f32| u / (1.0 + u.abs());
    let k = k / DRIVE_REF;
    (c(k * x + PRE_REST) - c(PRE_REST)) * (1.0 + PRE_REST) * (1.0 + PRE_REST) / k
}

impl Keys {
    pub fn new(rate: u32) -> Keys {
        let mut values = [0.0; ID_SPACE];
        for p in PARAMS {
            values[p.id as usize] = p.default;
        }
        let mut s = Keys {
            rate,
            values,
            voices: [Voice::default(); POOL],
            clock: 0,
            rng: Rng::new(0x6E75),
            bend: Smooth::new(0.0),
            bend_value: 0.0,
            out_l: Smooth::new(0.0),
            out_r: Smooth::new(0.0),
            tone_lo: Smooth::new(1.0),
            tone_hi: Smooth::new(1.0),
            tone_a: 0.0,
            tone_lp: 0.0,
            hp_a: 0.0,
            hp_x: 0.0,
            hp_y: 0.0,
            drive_k: 1.0,
            drive_n: 1.0,
            drive_amt: 0.0,
            pre_k: 1.0,
            trem_phase: 0.0,
            trem_l: 1.0,
            trem_r: 1.0,
            trem_dl: 0.0,
            trem_dr: 0.0,
            thump: 0.0,
            thump_fall: 0.0,
            thump_a: 0.0,
            thump_lp1: 0.0,
            thump_lp2: 0.0,
            tuning: Tuning::standard(),
            exprs: NoteExprs::new(),
            expr_c: 1.0,
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

    /// Voices sounding or fading.
    pub fn active_voices(&self) -> usize {
        self.voices
            .iter()
            .filter(|v| v.state != State::Free)
            .count()
    }

    #[inline]
    fn v(&self, id: u16) -> f32 {
        self.values[id as usize]
    }

    fn reed(&self) -> bool {
        self.v(MODEL) >= 0.5
    }

    fn update_out(&mut self) {
        let db = self.v(LEVEL);
        let lvl = if db <= -60.0 {
            0.0
        } else {
            10f32.powf(db / 20.0) * VOICE_GAIN
        };
        let pan = self.v(PAN) / 100.0;
        self.out_l.target = lvl * (1.0 - pan).sqrt();
        self.out_r.target = lvl * (1.0 + pan).sqrt();
    }

    fn update_tone(&mut self) {
        let t = self.v(TONE) / 100.0;
        // Up to 9 dB more (or less) treble, against 3 dB the other way in
        // the bass, split at 900 Hz.
        self.tone_hi.target = 10f32.powf(0.45 * t);
        self.tone_lo.target = 10f32.powf(-0.15 * t);
    }

    fn update_drive(&mut self) {
        let d = self.v(DRIVE) / 100.0;
        let gain = 1.0 + 9.0 * d;
        self.drive_k = gain / DRIVE_REF;
        self.drive_n = drive_norm(gain) * DRIVE_REF;
        self.drive_amt = (d * 2.5).min(1.0);
        // The reed's preamp always colours a little; Drive pushes it.
        self.pre_k = 1.5 + 14.0 * d * d;
    }

    fn update_model(&mut self) {
        let r = self.rate as f32;
        // The pickup network's high-pass: low for a tine, higher for the
        // reed's RC load.
        let hz = if self.reed() { 120.0 } else { 30.0 };
        self.hp_a = (-std::f32::consts::TAU * hz / r).exp();
    }

    /// The pitch a voice plays now, in fractional MIDI notes.
    fn pitch(&self, note: u8) -> f32 {
        f32::from(note)
            + self.v(TRANSPOSE)
            + self.v(FINE) / 100.0
            + self.bend.value * self.v(BEND_RANGE)
    }

    fn voice_pitch(&self, note: u8, ex: &VoiceExpr) -> f32 {
        let p = self.pitch(note);
        if ex.slide != 0.0 {
            p + ex.slide
        } else {
            p
        }
    }

    /// Sets a voice's resonators and pickup for its pitch and gate.
    fn tune(&self, v: &mut Voice, pitch: f32) {
        let rate = self.rate as f32;
        let held = v.state == State::On;
        let reed = self.reed();
        let modes = if reed { &REED } else { &TINE };
        let bell = self.v(BELL) / 100.0;
        let f0 = self.tuning.hz(pitch).clamp(10.0, rate * 0.45);
        let base = decay_for(self.v(DECAY), f0);
        let release = self.v(RELEASE) * 0.001;
        for (k, md) in modes.iter().enumerate() {
            let hz = f0 * md.ratio * v.jit_ratio[k];
            if hz >= 0.45 * rate || hz <= 10.0 || md.level == 0.0 {
                v.b0[k] = 0.0;
                v.c1[k] = 0.0;
                v.c2[k] = 0.0;
                continue;
            }
            let w = std::f32::consts::TAU * hz / rate;
            let mut t60 = (base * md.decay).max(0.004);
            if !held {
                t60 = t60.min(release);
            }
            let r = 10f32.powf(-3.0 / (t60 * rate));
            // The pickup hears velocity, which favours a mode by its
            // ratio: levels are set as heard, so the tine's motion in each
            // mode is the level over the ratio.
            let mut lvl = md.level * v.jit_level[k] / md.ratio;
            if md.bell {
                lvl *= if reed { 0.3 + 0.7 * bell } else { bell };
            }
            v.c1[k] = 2.0 * r * w.cos();
            v.c2[k] = r * r;
            v.b0[k] = w.sin() * lvl;
        }
        // The pickup: its rest point and swing, and the scale that brings
        // its output back to the tine's level. It hears the curve's change
        // (Faraday's law; a capacitor's current), so its output grows with
        // the note's frequency: that is divided out, and what stays is the
        // harmonics' tilt up.
        let (rest, g) = pickup_shape(reed, self.v(VOICING), self.v(BARK), key_spread(v.note));
        let swing = g * register_swing(f0);
        let w0 = std::f32::consts::TAU * f0 / rate;
        let slope = if reed {
            capacitance_slope(rest)
        } else {
            flux_slope(rest).abs().max(0.3)
        };
        if v.rest != rest || v.swing != swing {
            let x = v.y1.iter().sum::<f32>();
            v.last = if reed {
                capacitance(rest + swing * x)
            } else {
                flux(rest + swing * x)
            };
        }
        v.rest = rest;
        v.swing = swing;
        v.norm = 1.0 / (swing * w0 * slope);
        v.tuned = pitch;
    }

    fn note_on(&mut self, note: u8, velocity: u8) {
        if velocity == 0 {
            self.note_off(note);
            return;
        }
        let vel = f32::from(velocity.min(127)) / 127.0;
        self.clock += 1;
        let same = self
            .voices
            .iter()
            .position(|v| v.note == note && matches!(v.state, State::On | State::Release));
        let slot = match same {
            Some(i) => i,
            None => {
                let sounding = self
                    .voices
                    .iter()
                    .filter(|v| matches!(v.state, State::On | State::Release))
                    .count();
                if sounding >= self.v(POLYPHONY) as usize {
                    let old = self
                        .voices
                        .iter()
                        .enumerate()
                        .filter(|(_, v)| matches!(v.state, State::On | State::Release))
                        .min_by_key(|(_, v)| (v.state == State::On, v.age))
                        .map(|(i, _)| i);
                    if let Some(i) = old {
                        self.kill(i);
                    }
                }
                let free = self
                    .voices
                    .iter()
                    .position(|v| v.state == State::Free)
                    .or_else(|| {
                        self.voices
                            .iter()
                            .enumerate()
                            .filter(|(_, v)| v.state == State::Kill)
                            .min_by_key(|(_, v)| v.age)
                            .map(|(i, _)| i)
                    })
                    .unwrap_or(0);
                self.voices[free] = Voice {
                    tuned: f32::NAN,
                    rest: f32::NAN,
                    ..Voice::default()
                };
                free
            }
        };
        let mut v = self.voices[slot];
        v.state = State::On;
        v.note = note;
        v.age = self.clock;
        v.frames = 0;
        // No two strikes are quite alike: the upper modes move by up to
        // 0.3 % and 1.3 dB each time.
        v.jit_ratio[0] = 1.0;
        v.jit_level[0] = 1.0;
        for k in 1..MODES {
            v.jit_ratio[k] = 1.0 + 0.003 * self.rng.bipolar();
            v.jit_level[k] = 1.0 + 0.15 * self.rng.bipolar();
        }
        v.ex.start(self.exprs.on(note));
        let pitch = self.voice_pitch(note, &v.ex);
        self.tune(&mut v, pitch);
        // Velocity: a softer hit is a longer contact (Vel > tone sets how
        // much), so it reaches fewer upper modes.
        let vt = self.v(VEL_TONE) / 100.0;
        let tone_vel = 1.0 - vt * (1.0 - vel);
        let hz = self.tuning.hz(pitch);
        let rate = self.rate as f32;
        let len = (contact_secs(self.v(HARDNESS), tone_vel, hz) * rate).max(2.0);
        v.pulse_len = len as u32;
        v.pulse_at = 0;
        // Unit area: the fundamental rings at the strike's level whatever
        // the hammer.
        v.pulse_amp = strike_amp(vel) * std::f32::consts::FRAC_PI_2 / len;
        self.voices[slot] = v;
        self.thump += self.v(KEY_NOISE) / 100.0 * (0.3 + 0.7 * vel) * 0.05;
    }

    fn note_off(&mut self, note: u8) {
        self.exprs.off(note);
        let mut damped = false;
        for i in 0..POOL {
            if self.voices[i].note == note && self.voices[i].state == State::On {
                let mut v = self.voices[i];
                v.state = State::Release;
                let pitch = self.voice_pitch(note, &v.ex);
                self.tune(&mut v, pitch);
                self.voices[i] = v;
                damped = true;
            }
        }
        if damped {
            // The damper's felt lands: a softer thump than the hammer's.
            self.thump += self.v(KEY_NOISE) / 100.0 * 0.02;
        }
    }

    fn kill(&mut self, i: usize) {
        let v = &mut self.voices[i];
        if matches!(v.state, State::On | State::Release) {
            v.state = State::Kill;
            v.kill = 1.0;
            v.kill_step = 1.0 / (KILL_MS * 0.001 * self.rate as f32).max(1.0);
        }
    }

    fn apply(&mut self, ev: &NodeEvent) {
        match *ev {
            NodeEvent::NoteOn { note, velocity } => self.note_on(note, velocity),
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
                for i in 0..POOL {
                    if self.voices[i].state == State::On {
                        let n = self.voices[i].note;
                        self.note_off(n);
                    }
                }
                self.exprs.clear();
            }
            NodeEvent::Param { id, value } => self.set_param(id, value),
            NodeEvent::Pressure { .. } => {}
            NodeEvent::Expression { note, kind, value } => {
                let Some(value) = self.exprs.set(note, kind, value) else {
                    return;
                };
                for v in &mut self.voices {
                    if v.state == State::On && v.note == note {
                        v.ex.aim(kind, value);
                    }
                }
            }
        }
    }

    /// The tremolo's left and right gains at its phase now. Pan follows a
    /// rounded square, as the tine pianos' stereo vibrato did (a square
    /// tremolo that went stereo); Amp a sine.
    fn trem_gains(&self) -> (f32, f32) {
        tremolo_gains(self.v(TREMOLO), self.v(TREM_MODE) >= 0.5, self.trem_phase)
    }

    /// One control step: smooth the bend, retune voices whose pitch moved,
    /// move the tremolo.
    fn control(&mut self) {
        self.bend.next();
        for i in 0..POOL {
            let (state, note, tuned, mut ex) = {
                let v = &self.voices[i];
                (v.state, v.note, v.tuned, v.ex)
            };
            if state == State::Free {
                continue;
            }
            if ex.active() {
                ex.step(self.expr_c);
                self.voices[i].ex = ex;
            }
            let p = self.voice_pitch(note, &ex);
            if tuned.is_nan() || (p - tuned).abs() > 1e-4 {
                let mut v = self.voices[i];
                self.tune(&mut v, p);
                self.voices[i] = v;
            }
        }
        // The bus filters' tails: flushed once inaudible, so a silent
        // Keys puts out exact zeros (and never runs on denormals).
        for x in [
            &mut self.hp_x,
            &mut self.hp_y,
            &mut self.tone_lp,
            &mut self.thump_lp1,
            &mut self.thump_lp2,
        ] {
            if x.abs() < 1e-12 {
                *x = 0.0;
            }
        }
        self.trem_phase += self.v(TREM_RATE) * CTRL as f32 / self.rate as f32;
        self.trem_phase -= self.trem_phase.floor();
        let (l, r) = self.trem_gains();
        let n = CTRL as f32;
        self.trem_dl = (l - self.trem_l) / n;
        self.trem_dr = (r - self.trem_r) / n;
    }

    fn render(&mut self, from: usize, to: usize, left: &mut [f32], right: &mut [f32]) {
        let reed = self.reed();
        let n_modes = if reed { 5 } else { 3 };
        let amp_trem = self.v(TREM_MODE) >= 0.5;
        for i in from..to {
            let mut bus = 0.0f32;
            for v in self.voices.iter_mut() {
                if v.state == State::Free {
                    continue;
                }
                let x = if v.pulse_at < v.pulse_len {
                    let t = (v.pulse_at as f32 + 0.5) / v.pulse_len as f32;
                    v.pulse_at += 1;
                    v.pulse_amp * (std::f32::consts::PI * t).sin()
                } else {
                    0.0
                };
                let mut d = 0.0;
                for k in 0..n_modes {
                    let y = v.b0[k] * x + v.c1[k] * v.y1[k] - v.c2[k] * v.y2[k];
                    v.y2[k] = v.y1[k];
                    v.y1[k] = y;
                    d += y;
                }
                let u = v.rest + v.swing * d;
                let c = if reed { capacitance(u) } else { flux(u) };
                let mut y = (c - v.last) * v.norm;
                v.last = c;
                if v.state == State::Kill {
                    y *= v.kill;
                    v.kill -= v.kill_step;
                    if v.kill <= 0.0 {
                        v.state = State::Free;
                    }
                }
                let a = d.abs();
                v.follow = if a > v.follow { a } else { v.follow * 0.9995 };
                v.frames = v.frames.saturating_add(1);
                if v.follow < SILENT && v.frames > 256 && v.pulse_at >= v.pulse_len {
                    v.state = State::Free;
                }
                bus += y;
            }
            if self.thump > 1e-6 {
                let n = self.rng.bipolar() * self.thump;
                self.thump *= self.thump_fall;
                self.thump_lp1 += self.thump_a * (n - self.thump_lp1);
                self.thump_lp2 += self.thump_a * (self.thump_lp1 - self.thump_lp2);
            } else {
                self.thump = 0.0;
                self.thump_lp1 *= 0.99;
                self.thump_lp2 *= 0.99;
            }
            bus += self.thump_lp2 * 4.0;
            // The pickup network's high-pass.
            let hp = self.hp_a * (self.hp_y + bus - self.hp_x);
            self.hp_x = bus;
            self.hp_y = hp;
            // Tone: a tilt around 900 Hz.
            self.tone_lp += self.tone_a * (hp - self.tone_lp);
            let lo = self.tone_lo.next();
            let hi = self.tone_hi.next();
            let mut y = self.tone_lp * lo + (hp - self.tone_lp) * hi;
            self.trem_l += self.trem_dl;
            self.trem_r += self.trem_dr;
            let (tl, tr) = if reed && amp_trem {
                // The reed piano's tremolo works inside its preamp, so it
                // moves the tone with the level.
                y = preamp(y * self.trem_l, self.pre_k);
                (1.0, 1.0)
            } else {
                y = if reed {
                    preamp(y, self.pre_k)
                } else {
                    drive_n(y, self.drive_k, self.drive_n, self.drive_amt)
                };
                (self.trem_l, self.trem_r)
            };
            if reed {
                // The reed piano's preamp makes up the electrostatic
                // pickup's lower output.
                y *= REED_GAIN;
            }
            left[i] += y * tl * self.out_l.next();
            right[i] += y * tr * self.out_r.next();
        }
    }
}

impl Node for Keys {
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
        let old = self.values[id as usize];
        self.values[id as usize] = p.clamp(value);
        if id == MODEL && self.values[id as usize] != old {
            // A voice can't change model while it rings: fade them out.
            for i in 0..POOL {
                self.kill(i);
            }
        }
        match id {
            LEVEL | PAN => self.update_out(),
            TONE => self.update_tone(),
            DRIVE => self.update_drive(),
            MODEL => self.update_model(),
            _ => {}
        }
        if matches!(id, MODEL | DECAY | RELEASE | BELL | VOICING | BARK) {
            for v in self.voices.iter_mut() {
                v.tuned = f32::NAN;
            }
        }
    }

    fn param(&self, id: u16) -> f32 {
        if id == METER_VOICES {
            return self
                .voices
                .iter()
                .filter(|v| matches!(v.state, State::On | State::Release))
                .count() as f32;
        }
        match PARAMS.iter().find(|p| p.id == id) {
            Some(_) => self.values[id as usize],
            None => f32::NAN,
        }
    }

    fn prepare(&mut self, rate: u32, _max_block: usize) {
        self.rate = rate;
        let r = rate as f32;
        let cr = rate / CTRL as u32;
        self.bend.set_time(cr, 4.0);
        self.expr_c = expr_coef(cr as f32);
        self.out_l.set_time(rate, 10.0);
        self.out_r.set_time(rate, 10.0);
        self.tone_lo.set_time(rate, 10.0);
        self.tone_hi.set_time(rate, 10.0);
        self.tone_a = 1.0 - (-std::f32::consts::TAU * 900.0 / r).exp();
        self.thump_a = 1.0 - (-std::f32::consts::TAU * 500.0 / r).exp();
        self.thump_fall = (-1.0 / (0.012 * r)).exp();
        self.reset();
    }

    fn reset(&mut self) {
        for v in self.voices.iter_mut() {
            *v = Voice::default();
        }
        self.update_out();
        self.update_tone();
        self.update_drive();
        self.update_model();
        self.out_l.snap(self.out_l.target);
        self.out_r.snap(self.out_r.target);
        self.tone_lo.snap(self.tone_lo.target);
        self.tone_hi.snap(self.tone_hi.target);
        self.tone_lp = 0.0;
        self.hp_x = 0.0;
        self.hp_y = 0.0;
        self.bend.snap(self.bend_value);
        self.exprs.clear();
        self.trem_phase = 0.0;
        let (l, r) = self.trem_gains();
        self.trem_l = l;
        self.trem_r = r;
        self.trem_dl = 0.0;
        self.trem_dr = 0.0;
        self.thump = 0.0;
        self.thump_lp1 = 0.0;
        self.thump_lp2 = 0.0;
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
        (self.v(RELEASE).max(KILL_MS) * 0.001 * self.rate as f32 * 1.5) as u32
    }

    fn set_tuning(&mut self, tuning: &Tuning) {
        if self.tuning == *tuning {
            return;
        }
        self.tuning.clone_from(tuning);
        for v in self.voices.iter_mut() {
            v.tuned = f32::NAN;
        }
    }
}
