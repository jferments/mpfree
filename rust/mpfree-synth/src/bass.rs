//! Bass: a one-voice bass engine for synth basses, 808s and reeses, built
//! from `docs/dev/research/bass.md` on the shared voice core.
//!
//! - **Bodies.** **Synth**: an oscillator (saw, pulse, triangle or sine)
//!   with a sub, a fifth and noise into the ladder. **808**: a sine (or a
//!   triangle) with no filter, shaped by its pitch drop, the drive and
//!   Tone. **Reese**: two copies of the oscillator, one tuned up and one
//!   down by **Detune**, high-passed so their beating never empties the
//!   low end, over the sub (a clean sine by default).
//! - **The oscillators are locked.** One whole-number phase counter turns
//!   once every four cycles of the note; the oscillator, the sub (a
//!   square one or two octaves down, a narrow pulse two down, or a sine
//!   one down or at the note) and the fifth (exactly 3:2) all read it, so
//!   they stay in phase, as a divider's sub does. Steps are band-limited
//!   with polyBLEP; a sub's steps fall on the oscillator's wraps and get
//!   the same correction. **Start**: each new note starts every oscillator
//!   at the cycle's start (0°: the sine and the triangle rising from
//!   zero), half way (180°), or where it was (Free).
//! - **Pitch.** **Punch** shifts the note up to an octave for its first
//!   6 ms (the 808's attack shift); **Drop** starts the note this many
//!   semitones high and lands on it exactly at **Drop time** (a cubic
//!   fall: fast, then easing in), so the note is in tune from then on.
//!   Glide by time or by rate ([`Glide`]), on legato notes or always; note
//!   priority last, low or high ([`NoteStack`]); a legato note restarts the
//!   envelopes only with **Retrigger**.
//! - **The ladder** (Synth and Reese): four stages, each `dv/dt = w
//!   tanh(x - v)`, the output fed back (k up to 4.1, so it whistles at
//!   Resonance 100), solved with zero-delay feedback each sample, each
//!   stage's `tanh(d)/d` taken from the previous sample (as Analog's), half
//!   the passband's loss to resonance made up. **Filter drive** sets how
//!   hard the oscillators push it, with the output scaled back, so it adds
//!   grit, not volume. The cutoff follows its envelope (four octaves at
//!   **Env amount** 100), the key and pressure.
//! - **Amplifier**: [`Adsr`] (times that mean what they say) and the
//!   velocity curve ([`velocity_gain`]). Then **Drive**, an asymmetric
//!   smooth curve scaled to keep the level, so it adds the second and
//!   third harmonics that make a bass heard on small speakers; **Tone**, a
//!   two-pole low-pass after it; **High-pass**, two poles, which also takes
//!   the drive's DC.
//! - **Playing.** Pad pressure moves the cutoff (up to three octaves), the
//!   drive or a vibrato, as **Press to** picks, by **Pressure**; the mod
//!   wheel adds vibrato (up to 50 cents at 5.5 Hz). Bend moves the note.
//!   The sustain pedal is the host's ([`mpfree_node::pedal`]).
//!
//! Below 64 kHz the oscillators, ladder, drive and Tone run at twice the
//! rate and come down through a polyphase IIR half-band (Squelch's). Pitch,
//! the envelopes, glide and the cutoff run at a control step of [`CTRL`]
//! frames aligned to the absolute frame, with the pitch, cutoff, level and
//! drive moving linearly across the step, so the output does not depend
//! on how the host cuts blocks. A tenth of a second after the release ends
//! nothing runs until the next note. Nothing allocates.

use std::f32::consts::PI;

use mpfree_node::{Ctl, Ctx, Kind, Node, NodeEvent, NodeInfo, ParamInfo, Timed, Tuning};

use crate::bass_params::*;
use crate::body808::{drive, pitch_semis, triangle_turns, PUNCH_S, PUNCH_SEMIS};
pub use crate::body808::{drive_gains, drive_shape, sin_turns};
use crate::dsp::Rng;
use crate::params::Preset;
use crate::voice::{velocity_gain, Adsr, Glide, NoteStack, Priority};

/// Frames per control step.
pub const CTRL: u64 = 16;

static INFO: NodeInfo = NodeInfo {
    id: "mpfree.bass",
    name: "Bass",
    kind: Kind::Instrument,
};

/// The filter envelope's sweep at Env amount 100, octaves.
pub const ENV_OCT: f32 = 4.0;
/// Pressure's sweep of the cutoff at Pressure 100, octaves.
const PRESS_OCT: f32 = 3.0;
/// Vibrato at full (mod wheel or pressure), cents, and its rate.
const VIB_CENTS: f32 = 50.0;
const VIB_HZ: f32 = 5.5;
/// Key tracking's centre: the cutoff is as set at C3.
const TRACK_NOTE: f32 = 48.0;
/// Vel > tone at 100: the softest note's cutoff and Tone this many
/// octaves down.
const VEL_TONE_OCT: f32 = 2.0;
/// Keyboard level scaling under C3, dB an octave, against what the ear
/// (and loudness meters) take from low notes, so a line's notes are about
/// as loud from C0 to C3 (B6): the synth bodies, and the 808's sine.
const FOLLOW_BELOW: f32 = 48.0;
const FOLLOW_DB: f32 = 2.0;
const FOLLOW_808_DB: f32 = 3.5;
/// Each body's trim, dB, so their presets sit at about the same loudness
/// (the 808's sine puts all its level where loudness counts it).
const TRIM_DB: [f32; 3] = [0.0, -5.0, 1.5];
/// The ladder's feedback at Resonance 100: just past 4, so it whistles.
const K_MAX: f32 = 4.1;
/// Filter drive: the oscillators into the ladder at 0 and at 100.
const IN_MIN: f32 = 0.35;
const IN_MAX: f32 = 4.0;
/// The reese's pair: each one's level, and the high-pass under them.
const PAIR_LEVEL: f32 = 0.7;
/// The pair's high-pass: twice the note, so their beating fundamental
/// goes and the sub carries it (within these ends, Hz).
const PAIR_HP_NOTES: f32 = 2.0;
const PAIR_HP_LO: f32 = 20.0;
const PAIR_HP_HI: f32 = 400.0;
/// The noise's level at Noise 100.
const NOISE_LEVEL: f32 = 0.5;
/// The level into the drive (a full oscillator peaks near [`DRIVE_REF`]
/// there), and the output after it: one note of Init at velocity 100
/// peaks near -12 dBFS.
const PRE_GAIN: f32 = 0.25;
const OUT_GAIN: f32 = 0.68;
/// How long after the release ends the engine stops running.
const QUIET_MS: f32 = 100.0;

/// The half-band's all-pass coefficients (as Squelch's).
const HB_EVEN: [f32; 3] = [0.060_297_39, 0.412_590_7, 0.772_715_6];
const HB_ODD: [f32; 3] = [0.215_971_44, 0.604_358_6, 0.923_886_1];

/// `2^-32`: a whole-number phase to turns.
const TURN: f32 = 1.0 / 4_294_967_296.0;

/// The sub's value in each of the four cycles of the oscillator its
/// counter spans: a square an octave down, two down, and a pulse a
/// quarter wide two down (no DC: +1.5 then -0.5 three times).
const SUB_TABLES: [[f32; 4]; 3] = [
    [1.0, -1.0, 1.0, -1.0],
    [1.0, 1.0, -1.0, -1.0],
    [1.5, -0.5, -0.5, -0.5],
];

/// The step correction (polyBLEP) for a step at the phase's wrap: `t` the
/// phase in turns, `dt` its step, `rdt` the step's reciprocal. A step of
/// height `h` is corrected by adding `h / 2` times this.
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

/// One sample of `wave` (an index into [`WAVES`]) at phase `t`.
#[inline(always)]
fn wave_at(wave: u8, t: f32, dt: f32, rdt: f32, pw: f32) -> f32 {
    match wave {
        0 => 2.0 * t - 1.0 - blep(t, dt, rdt),
        1 => {
            // A pulse as the difference of two saws, pw apart: no DC.
            let u = if t < pw { t - pw + 1.0 } else { t - pw };
            (2.0 * u - blep(u, dt, rdt)) - (2.0 * t - blep(t, dt, rdt))
        }
        // Rising from zero at t = 0, peak at a quarter.
        2 => triangle_turns(t),
        _ => sin_turns(t),
    }
}

/// A one-pole stage's input share `G / (1 + G)` for `G = g tanh(d) / d`
/// (as Analog's ladder).
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

/// The ladder's small-signal gain at `hz` for **Cutoff** `cutoff` and
/// **Resonance** `resonance` (0 to 100), with its make-up: four equal
/// one-pole stages with the output fed back. For the Synth tab's picture
/// and the tests.
pub fn ladder_response(cutoff: f32, resonance: f32, hz: f32) -> f32 {
    let k = K_MAX * (resonance / 100.0).clamp(0.0, 1.0);
    let w = hz.max(0.0) / cutoff.max(1.0);
    // One stage: 1 / (1 + jw); four: 1 / (1 + jw)^4.
    let (mut re, mut im) = (1.0f32, 0.0f32);
    for _ in 0..4 {
        let (r, i) = (re - im * w, im + re * w);
        re = r;
        im = i;
    }
    // H = 1 / (D + k), D = (1 + jw)^4.
    let (dr, di) = (re + k, im);
    (1.0 + k).sqrt() / (dr * dr + di * di).sqrt()
}

/// A 2-pole state-variable filter (Simper's trapezoidal form): low- or
/// high-pass, Butterworth.
#[derive(Clone, Copy, Debug, Default)]
struct Svf {
    a1: f32,
    a2: f32,
    a3: f32,
    ic1: f32,
    ic2: f32,
}

impl Svf {
    fn set(&mut self, hz: f32, fs: f32) {
        let g = (PI * hz.clamp(1.0, 0.45 * fs) / fs).tan();
        let k = std::f32::consts::SQRT_2;
        self.a1 = 1.0 / (1.0 + g * (g + k));
        self.a2 = g * self.a1;
        self.a3 = g * self.a2;
    }

    /// One sample; `HIGH` picks the high-pass.
    #[inline(always)]
    fn run<const HIGH: bool>(&mut self, x: f32) -> f32 {
        let v3 = x - self.ic2;
        let v1 = self.a1 * self.ic1 + self.a2 * v3;
        let v2 = self.ic2 + self.a2 * self.ic1 + self.a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        if HIGH {
            x - std::f32::consts::SQRT_2 * v1 - v2
        } else {
            v2
        }
    }

    fn clear(&mut self) {
        self.ic1 = 0.0;
        self.ic2 = 0.0;
    }
}

/// One first-order all-pass section of the half-band.
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

/// What the inner loop reads, set once a control step.
#[derive(Clone, Copy, Debug, Default)]
struct Frame {
    body: u8,
    wave: u8,
    pw: f32,
    sub: f32,
    /// 0 to 2: a divided square or pulse; 3: a sine an octave down; 4: a
    /// sine at the note.
    sub_mode: u8,
    fifth: f32,
    noise: f32,
    /// The ladder's input gain, its output scale and its feedback.
    in_gain: f32,
    out_scale: f32,
    k: f32,
    hard: bool,
}

/// The one-voice bass. See the module docs.
pub struct Bass {
    rate: u32,
    /// Oversampling: 2 below 64 kHz, else 1.
    os: usize,
    values: [f32; ID_SPACE],
    tuning: Tuning,

    // Notes.
    stack: NoteStack,
    /// The note sounding (its key may be up during the release).
    note: u8,
    velocity: u8,
    /// Whether a note has sounded since the last reset (Glide on Always
    /// glides from it).
    played: bool,
    /// The note's pitch, gliding, before the pitch envelope, bend and
    /// vibrato; and the glide.
    pitch: f32,
    glide: Glide,
    /// Control steps since the note's start, for Punch and Drop.
    since: u32,
    bend: f32,
    wheel: f32,
    pressure: [u8; 128],
    press: f32,
    vib_phase: f32,

    // Envelopes.
    amp: Adsr,
    fenv: Adsr,
    vel_gain: f32,

    // Oscillators: the locked counter (four cycles of the note a turn)
    // and its step per inner sample now and its change per sample; the
    // reese's pair.
    p4: u32,
    inc: f32,
    dinc: f32,
    pa: u32,
    pb: u32,
    inc_a: f32,
    dinc_a: f32,
    inc_b: f32,
    dinc_b: f32,
    rng: Rng,

    // The ladder: integrator states, last differences, cutoff now and its
    // change per sample.
    s: [f32; 4],
    d: [f32; 4],
    g: f32,
    dg: f32,
    pair_hp: Svf,

    // Amplifier, drive and Tone.
    vca: f32,
    dvca: f32,
    /// The level and key scaling after the drive, and its change.
    out: f32,
    dout: f32,
    drive_k: f32,
    ddrive_k: f32,
    drive_n: f32,
    ddrive_n: f32,
    tone: Svf,
    tone_hz: f32,
    hb_even: [Ap; 3],
    hb_odd: [Ap; 3],
    hp: Svf,

    frame: Frame,
    /// Control steps of silence after the release, and whether nothing
    /// runs until the next note.
    quiet: u32,
    idle: bool,
}

impl Bass {
    pub fn new(rate: u32) -> Bass {
        let mut values = [0.0; ID_SPACE];
        for p in PARAMS {
            values[p.id as usize] = p.default;
        }
        let mut b = Bass {
            rate: rate.max(1),
            os: 2,
            values,
            tuning: Tuning::standard(),
            stack: NoteStack::new(),
            note: 48,
            velocity: 100,
            played: false,
            pitch: 48.0,
            glide: Glide::default(),
            since: 0,
            bend: 0.0,
            wheel: 0.0,
            pressure: [0; 128],
            press: 0.0,
            vib_phase: 0.0,
            amp: Adsr::default(),
            fenv: Adsr::default(),
            vel_gain: 1.0,
            p4: 0,
            inc: 0.0,
            dinc: 0.0,
            pa: 0,
            pb: 0,
            inc_a: 0.0,
            dinc_a: 0.0,
            inc_b: 0.0,
            dinc_b: 0.0,
            rng: Rng::new(0xBA55),
            s: [0.0; 4],
            d: [0.0; 4],
            g: 0.1,
            dg: 0.0,
            pair_hp: Svf::default(),
            vca: 0.0,
            dvca: 0.0,
            out: 1.0,
            dout: 0.0,
            drive_k: 0.0,
            ddrive_k: 0.0,
            drive_n: 1.0,
            ddrive_n: 0.0,
            tone: Svf::default(),
            tone_hz: 0.0,
            hb_even: [Ap::default(); 3],
            hb_odd: [Ap::default(); 3],
            hp: Svf::default(),
            frame: Frame::default(),
            quiet: 0,
            idle: true,
        };
        b.prepare(rate, 0);
        b
    }

    pub fn presets() -> &'static [Preset] {
        crate::factory::presets!("bass.presets", &INFO, PARAMS)
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

    /// One while anything can be heard, else none.
    pub fn active_voices(&self) -> usize {
        usize::from(!self.idle)
    }

    /// The keys held now.
    pub fn keys_held(&self) -> usize {
        self.stack.len()
    }

    /// The note sounding now, if any is heard.
    pub fn sounding_note(&self) -> Option<u8> {
        (!self.idle).then_some(self.note)
    }

    /// The pitch the oscillator plays now, in Hz (for tests).
    pub fn osc_hz(&self) -> f32 {
        let fs = (self.rate as usize * self.os) as f32;
        4.0 * self.inc * TURN * fs
    }

    /// The ladder's cutoff now, in Hz (for tests and pictures).
    pub fn cutoff_hz(&self) -> f32 {
        let fs = (self.rate as usize * self.os) as f32;
        self.g.atan() * fs / PI
    }

    fn v(&self, id: u16) -> f32 {
        self.values[id as usize]
    }

    fn fs(&self) -> f32 {
        (self.rate as usize * self.os) as f32
    }

    fn steps_per_sec(&self) -> f32 {
        self.rate as f32 / CTRL as f32
    }

    fn update_times(&mut self) {
        let sps = self.steps_per_sec();
        self.amp.set(
            self.v(ATTACK),
            self.v(DECAY),
            self.v(SUSTAIN) / 100.0,
            self.v(RELEASE),
            sps,
        );
        let fd = self.v(FILTER_DECAY);
        self.fenv.set(0.0, fd, 0.0, fd, sps);
        let ms = self.v(GLIDE);
        self.glide = if self.v(GLIDE_MODE) >= 0.5 {
            Glide::rate(if ms > 0.0 { 12_000.0 / ms } else { 0.0 }, sps)
        } else {
            Glide::time(ms, sps)
        };
    }

    fn update_filters(&mut self) {
        let fs = self.fs();
        self.tone_hz = self.v(TONE);
        self.tone.set(self.tone_hz, fs);
        self.pair_hp.set(PAIR_HP_LO, fs);
        self.hp.set(self.v(HIGH_PASS), self.rate as f32);
    }

    /// The pitch the oscillator plays `steps` control steps into the note,
    /// before bend and vibrato: the glide, then Punch and Drop.
    fn envelope_semis(&self, steps: u32) -> f32 {
        // In ms, so a drop of whole milliseconds lands on its step exactly.
        let t = steps as f32 * (CTRL * 1000) as f32 / self.rate as f32;
        pitch_semis(
            t,
            self.v(DROP),
            self.v(DROP_TIME),
            self.v(PUNCH) * 0.01 * PUNCH_SEMIS,
            PUNCH_S * 1000.0,
        )
    }

    /// The target pitch for the note sounding, before glide.
    fn note_target(&self) -> f32 {
        f32::from(self.note) + self.v(TRANSPOSE) + self.v(OCTAVE) * 12.0 + self.v(FINE) * 0.01
    }

    /// The counter's step per inner sample for a pitch (fractional note).
    fn step_for(&self, pitch: f32) -> f32 {
        let hz = self.tuning.hz(pitch).max(0.0);
        // At most a tenth of a turn a sample: the oscillator under 0.4 of
        // a cycle, so its steps' corrections never overlap.
        (hz * 0.25 / self.fs() * 4_294_967_296.0).min(0.1 * 4_294_967_296.0)
    }

    /// The pitch now, all of it: glide, envelope, bend and vibrato.
    fn pitch_now(&self, steps: u32) -> f32 {
        let vib_depth = {
            let press = if self.v(PRESS_TO) == PRESS_VIBRATO {
                self.press * self.v(PRESSURE) * 0.01
            } else {
                0.0
            };
            self.wheel.max(press) * VIB_CENTS * 0.01
        };
        self.pitch
            + self.envelope_semis(steps)
            + self.bend * self.v(BEND_RANGE)
            + vib_depth * sin_turns(self.vib_phase)
    }

    /// Sets the oscillators' steps for the pitch now, at once.
    fn snap_pitch(&mut self) {
        let p = self.pitch_now(self.since);
        self.inc = self.step_for(p);
        self.dinc = 0.0;
        let (a, b) = self.pair_steps(self.inc);
        self.inc_a = a;
        self.inc_b = b;
        self.dinc_a = 0.0;
        self.dinc_b = 0.0;
    }

    /// The reese pair's steps (each a whole cycle a turn) for the
    /// counter's step.
    fn pair_steps(&self, inc4: f32) -> (f32, f32) {
        let r = (self.v(DETUNE) / 1_200.0).exp2();
        let main = (4.0 * inc4).min(0.45 * 4_294_967_296.0);
        ((main * r).min(0.45 * 4_294_967_296.0), main / r)
    }

    /// Starts the oscillators where Start says, for a new note.
    fn start_phases(&mut self) {
        let start = self.v(START);
        if start == FREE {
            return;
        }
        // 180°: the oscillator half way, the counter an eighth of a turn.
        let (p4, pair) = if start == DEG180 {
            (1u32 << 29, 1u32 << 31)
        } else {
            (0, 0)
        };
        self.p4 = p4;
        self.pa = pair;
        self.pb = pair;
    }

    /// A key goes down.
    fn note_on(&mut self, note: u8, velocity: u8) {
        if velocity == 0 {
            self.note_off(note);
            return;
        }
        let held_before = !self.stack.is_empty();
        self.stack.push(note);
        self.press = f32::from(self.pressure[usize::from(note & 0x7F)]) / 127.0;
        let pick = self
            .stack
            .pick(Priority::from_index(self.v(PRIORITY)))
            .unwrap_or(note);
        if held_before {
            if pick != self.note {
                self.move_to(pick, velocity);
            }
            return;
        }
        // A fresh note: the envelopes start, and it glides only on Always.
        let glides = self.played && self.v(GLIDE_ON) == ALWAYS;
        self.note = pick;
        self.velocity = velocity;
        self.vel_gain = velocity_gain(velocity, self.v(VELOCITY));
        if !glides {
            self.pitch = self.note_target();
        }
        self.played = true;
        if self.idle {
            self.idle = false;
            self.amp.clear();
            self.fenv.clear();
        }
        self.quiet = 0;
        self.since = 0;
        // Start places the oscillators only for a note out of silence: a
        // jump in a note still ringing would click.
        if self.amp.level < 0.01 {
            self.start_phases();
        }
        self.amp.gate_on();
        self.fenv.gate_on();
        self.snap_pitch();
    }

    /// A legato move to another held key: it glides there, and starts the
    /// envelopes again only with Retrigger.
    fn move_to(&mut self, note: u8, velocity: u8) {
        self.note = note;
        self.press = f32::from(self.pressure[usize::from(note & 0x7F)]) / 127.0;
        if self.v(RETRIGGER) >= 0.5 {
            self.velocity = velocity;
            self.vel_gain = velocity_gain(velocity, self.v(VELOCITY));
            self.since = 0;
            self.amp.gate_on();
            self.fenv.gate_on();
        }
    }

    /// A key comes up.
    fn note_off(&mut self, note: u8) {
        if !self.stack.remove(note) {
            return;
        }
        match self.stack.pick(Priority::from_index(self.v(PRIORITY))) {
            Some(next) if next != self.note => self.move_to(next, self.velocity),
            Some(_) => {}
            None => {
                self.amp.gate_off();
                self.fenv.gate_off();
            }
        }
    }

    fn all_off(&mut self) {
        self.stack.clear();
        self.amp.gate_off();
        self.fenv.gate_off();
    }

    fn apply(&mut self, ev: &NodeEvent) {
        match *ev {
            NodeEvent::NoteOn { note, velocity } => self.note_on(note & 0x7F, velocity),
            NodeEvent::NoteOff { note } => self.note_off(note & 0x7F),
            NodeEvent::Pressure { note, value } => {
                let n = usize::from(note & 0x7F);
                self.pressure[n] = value.min(127);
                if note & 0x7F == self.note {
                    self.press = f32::from(self.pressure[n]) / 127.0;
                }
            }
            NodeEvent::Bend { value } => {
                if value.is_finite() {
                    self.bend = value.clamp(-1.0, 1.0);
                }
            }
            NodeEvent::Control { ctl, value } => {
                if ctl == Ctl::ModWheel {
                    if let Some(v) = Ctl::clamp(value) {
                        self.wheel = v;
                    }
                }
            }
            NodeEvent::AllOff => self.all_off(),
            NodeEvent::Param { id, value } => self.set_param(id, value),
            NodeEvent::Expression { .. } => {}
        }
    }

    /// One control step: glide, envelopes, the pitch and cutoff ramps, the
    /// level and the drive.
    fn control(&mut self) {
        if self.idle {
            return;
        }
        let n = (CTRL as usize * self.os) as f32;
        let target = self.note_target();
        let mut pitch = self.pitch;
        self.glide.step(&mut pitch, target);
        self.pitch = pitch;
        self.since = self.since.saturating_add(1);
        self.vib_phase += VIB_HZ / self.steps_per_sec();
        self.vib_phase -= self.vib_phase.floor();
        let p = self.pitch_now(self.since);
        let inc_to = self.step_for(p);
        self.dinc = (inc_to - self.inc) / n;
        let (a, b) = self.pair_steps(inc_to);
        self.dinc_a = (a - self.inc_a) / n;
        self.dinc_b = (b - self.inc_b) / n;

        let amp = self.amp.next();
        let fenv = self.fenv.next();
        let fs = self.fs();
        let press = self.press * self.v(PRESSURE) * 0.01;
        let press_to = self.v(PRESS_TO);
        let oct = ENV_OCT * self.v(ENV_AMOUNT) * 0.01 * fenv
            + self.v(KEY_TRACK) * 0.01 * (p - TRACK_NOTE) / 12.0
            + if press_to == PRESS_CUTOFF {
                PRESS_OCT * press
            } else {
                0.0
            };
        let soft =
            self.v(VEL_TONE) * 0.01 * VEL_TONE_OCT * (1.0 - f32::from(self.velocity) / 127.0);
        let oct = oct - soft;
        let fc = (self.v(CUTOFF) * oct.exp2()).clamp(10.0, 0.45 * fs);
        let tone = (self.v(TONE) * (-soft).exp2()).clamp(20.0, 0.45 * fs);
        if tone != self.tone_hz {
            self.tone_hz = tone;
            self.tone.set(tone, fs);
        }
        let g_to = (PI * fc / fs).tan();
        self.dg = (g_to - self.g) / n;

        let follow = if self.v(BODY) == B808 {
            FOLLOW_808_DB
        } else {
            FOLLOW_DB
        };
        let below = ((FOLLOW_BELOW - p) / 12.0).clamp(0.0, 4.0);
        let vca_to = amp * self.vel_gain * PRE_GAIN;
        // A ramp to 0 leaves a rounding's worth behind, and the next ramp a
        // rounding of that: under -400 dB the amplifier lands on exact 0.
        if vca_to == 0.0 && self.vca.abs() < 1e-20 {
            self.vca = 0.0;
        }
        self.dvca = (vca_to - self.vca) / n;
        // The level and the key's scaling after the drive, so the drive
        // hears every note alike.
        let trim = TRIM_DB[(self.v(BODY) as usize).min(2)];
        let out_to = 10f32.powf((self.v(LEVEL) + trim + follow * below) / 20.0) * OUT_GAIN;
        self.dout = (out_to - self.out) / n;
        if self.v(BODY) == REESE {
            let f0 = 4.0 * inc_to * TURN * fs;
            self.pair_hp
                .set((PAIR_HP_NOTES * f0).clamp(PAIR_HP_LO, PAIR_HP_HI), fs);
        }

        let drive = (self.v(DRIVE)
            + if press_to == PRESS_DRIVE {
                100.0 * press
            } else {
                0.0
            })
        .clamp(0.0, 100.0);
        let hard = self.v(DRIVE_MODE) >= 0.5;
        let (k, norm) = drive_gains(drive, hard);
        self.ddrive_k = (k - self.drive_k) / n;
        self.ddrive_n = (norm - self.drive_n) / n;

        let res = self.v(RESONANCE) * 0.01;
        let in_gain = IN_MIN * (IN_MAX / IN_MIN).powf(self.v(FILTER_DRIVE) * 0.01);
        let k_fb = K_MAX * res;
        self.frame = Frame {
            body: self.v(BODY) as u8,
            wave: self.v(WAVE) as u8,
            pw: self.v(PW) * 0.01,
            sub: self.v(SUB) * 0.01,
            sub_mode: self.v(SUB_MODE) as u8,
            fifth: self.v(FIFTH) * 0.01,
            noise: self.v(NOISE) * 0.01 * NOISE_LEVEL,
            in_gain,
            // The ladder's output back to the oscillators' level, half the
            // resonance's loss made up.
            out_scale: (1.0 + k_fb).sqrt() / in_gain,
            k: k_fb,
            hard,
        };

        // A note held at a level of 0 feeds the stages after the amplifier
        // zeros, and their states would sink through the denormals; under
        // -400 dB each is exact 0.
        let tiny = |x: &mut f32| {
            if x.abs() < 1e-20 {
                *x = 0.0;
            }
        };
        for f in [&mut self.tone, &mut self.hp] {
            tiny(&mut f.ic1);
            tiny(&mut f.ic2);
        }
        for ap in self.hb_even.iter_mut().chain(self.hb_odd.iter_mut()) {
            tiny(&mut ap.x1);
            tiny(&mut ap.y1);
        }

        if self.amp.idle() {
            self.quiet += 1;
            if self.quiet as f32 >= QUIET_MS * 0.001 * self.steps_per_sec() {
                self.silence();
            }
        } else {
            self.quiet = 0;
        }
    }

    /// Stops everything until the next note: the output is exact silence.
    fn silence(&mut self) {
        self.idle = true;
        self.vca = 0.0;
        self.dvca = 0.0;
        self.dout = 0.0;
        self.dg = 0.0;
        self.ddrive_k = 0.0;
        self.ddrive_n = 0.0;
        self.dinc = 0.0;
        self.dinc_a = 0.0;
        self.dinc_b = 0.0;
        self.s = [0.0; 4];
        self.d = [0.0; 4];
        self.pair_hp.clear();
        self.tone.clear();
        self.hp.clear();
        self.hb_even = [Ap::default(); 3];
        self.hb_odd = [Ap::default(); 3];
        self.amp.clear();
        self.fenv.clear();
    }

    /// The four ladder stages for input `x`, cutoff `g` and feedback `k`.
    #[inline(always)]
    fn ladder(&mut self, x: f32, g: f32, k: f32) -> f32 {
        let a1 = stage_share(g, self.d[0]);
        let a2 = stage_share(g, self.d[1]);
        let a3 = stage_share(g, self.d[2]);
        let a4 = stage_share(g, self.d[3]);
        let [s1, s2, s3, s4] = self.s;
        let b1 = (1.0 - a1) * s1;
        let b2 = (1.0 - a2) * s2;
        let b3 = (1.0 - a3) * s3;
        let b4 = (1.0 - a4) * s4;
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

    /// One sample at the inner rate.
    #[inline(always)]
    fn sample(&mut self) -> f32 {
        let f = self.frame;
        let c4 = self.p4;
        self.p4 = c4.wrapping_add(self.inc as u32);
        let dt = 4.0 * self.inc * TURN;
        self.inc += self.dinc;
        let t = c4.wrapping_mul(4) as f32 * TURN;
        let rdt = 1.0 / dt.max(1e-9);

        // The sub, from the same counter.
        let mut mix = 0.0;
        if f.sub > 0.0 {
            let sub = match f.sub_mode {
                3 => sin_turns(c4.wrapping_mul(2) as f32 * TURN),
                4 => sin_turns(t),
                m => {
                    let tab = &SUB_TABLES[usize::from(m.min(2))];
                    let c = (c4 >> 30) as usize;
                    let mut v = tab[c];
                    let b = blep(t, dt, rdt);
                    if b != 0.0 {
                        let h = if t < 0.5 {
                            tab[c] - tab[(c + 3) & 3]
                        } else {
                            tab[(c + 1) & 3] - tab[c]
                        };
                        v += 0.5 * h * b;
                    }
                    v
                }
            };
            mix += f.sub * sub;
        }
        if f.fifth > 0.0 {
            let tf = c4.wrapping_mul(6) as f32 * TURN;
            let dtf = 1.5 * dt;
            mix += f.fifth * wave_at(f.wave, tf, dtf, 1.0 / dtf, f.pw);
        }
        if f.noise > 0.0 {
            mix += f.noise * self.rng.bipolar();
        }

        let y = match f.body {
            1 => {
                // 808: a sine or a triangle, no filter.
                let w = if f.wave == 2 { 2 } else { 3 };
                (mix + wave_at(w, t, dt, rdt, f.pw)) * self.vca
            }
            body => {
                let osc = if body == 2 {
                    let (ca, cb) = (self.pa, self.pb);
                    self.pa = ca.wrapping_add(self.inc_a as u32);
                    self.pb = cb.wrapping_add(self.inc_b as u32);
                    let (da, db) = (self.inc_a * TURN, self.inc_b * TURN);
                    self.inc_a += self.dinc_a;
                    self.inc_b += self.dinc_b;
                    let ta = ca as f32 * TURN;
                    let tb = cb as f32 * TURN;
                    let pair = PAIR_LEVEL
                        * (wave_at(f.wave, ta, da, 1.0 / da.max(1e-9), f.pw)
                            + wave_at(f.wave, tb, db, 1.0 / db.max(1e-9), f.pw));
                    self.pair_hp.run::<true>(pair)
                } else {
                    wave_at(f.wave, t, dt, rdt, f.pw)
                };
                let g = self.g;
                self.g += self.dg;
                let y = self.ladder((osc + mix) * f.in_gain, g, f.k) * f.out_scale;
                y * self.vca
            }
        };
        self.vca += self.dvca;

        // Drive, then Tone.
        let k = self.drive_k;
        let norm = self.drive_n;
        self.drive_k += self.ddrive_k;
        self.drive_n += self.ddrive_n;
        let y = drive(y, k, norm, f.hard);
        let out = self.out;
        self.out += self.dout;
        self.tone.run::<false>(y) * out
    }

    fn render(&mut self, from: usize, to: usize, left: &mut [f32], right: &mut [f32]) {
        if self.idle {
            return;
        }
        for i in from..to {
            let y = if self.os == 2 {
                let a = self.sample();
                let b = self.sample();
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
            let y = self.hp.run::<true>(y);
            left[i] += y;
            right[i] += y;
        }
    }
}

/// The pitch envelope over its first `secs` seconds, in semitones above
/// the note, `points` points: for the Synth tab's picture.
pub fn drop_curve(v: impl Fn(u16) -> f32, secs: f32, points: usize) -> Vec<f32> {
    let n = points.max(2);
    (0..n)
        .map(|i| {
            let t = secs * i as f32 / (n - 1) as f32;
            pitch_semis(
                t,
                v(DROP),
                v(DROP_TIME) * 0.001,
                v(PUNCH) * 0.01 * PUNCH_SEMIS,
                PUNCH_S,
            )
        })
        .collect()
}

/// One cycle of the sub's own period (four of the oscillator's, two for a
/// sine an octave down; one for a sine at the note), naive, of the
/// oscillator with its sub and fifth: for the Synth tab's picture.
pub fn wave_shape(v: impl Fn(u16) -> f32, points: usize) -> Vec<f32> {
    let n = points.max(2);
    let wave = v(WAVE) as u8;
    let body = v(BODY);
    let wave = if body == B808 {
        if wave == 2 {
            2
        } else {
            3
        }
    } else {
        wave
    };
    let pw = v(PW) * 0.01;
    let sub = v(SUB) * 0.01;
    let sub_mode = v(SUB_MODE) as u8;
    let fifth = v(FIFTH) * 0.01;
    // A tiny step: no corrections.
    let dt = 1e-6;
    (0..n)
        .map(|i| {
            let x = i as f32 / (n - 1) as f32 * 0.999_999;
            let t4 = x;
            let t = (4.0 * t4).fract();
            let mut y = if body == REESE {
                2.0 * PAIR_LEVEL * wave_at(wave, t, dt, 1.0 / dt, pw)
            } else {
                wave_at(wave, t, dt, 1.0 / dt, pw)
            };
            if sub > 0.0 {
                y += sub
                    * match sub_mode {
                        3 => sin_turns((2.0 * t4).fract()),
                        4 => sin_turns(t),
                        m => SUB_TABLES[usize::from(m.min(2))][(4.0 * t4) as usize & 3],
                    };
            }
            if fifth > 0.0 {
                y += fifth * wave_at(wave, (6.0 * t4).fract(), dt, 1.0 / dt, pw);
            }
            y
        })
        .collect()
}

impl Node for Bass {
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
        if p.readonly || !value.is_finite() {
            return;
        }
        self.values[id as usize] = p.clamp(value);
        match id {
            ATTACK | DECAY | SUSTAIN | RELEASE | FILTER_DECAY | GLIDE | GLIDE_MODE => {
                self.update_times()
            }
            TONE | HIGH_PASS => self.update_filters(),
            VELOCITY => self.vel_gain = velocity_gain(self.velocity, self.v(VELOCITY)),
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
        self.update_times();
        self.update_filters();
        self.reset();
    }

    fn reset(&mut self) {
        self.stack.clear();
        self.played = false;
        self.since = 0;
        self.bend = 0.0;
        self.wheel = 0.0;
        self.pressure = [0; 128];
        self.press = 0.0;
        self.vib_phase = 0.0;
        self.p4 = 0;
        self.pa = 0;
        self.pb = 0;
        self.pitch = self.note_target();
        self.g = (PI * self.v(CUTOFF).min(0.45 * self.fs()) / self.fs()).tan();
        self.dg = 0.0;
        let (k, n) = drive_gains(self.v(DRIVE), self.v(DRIVE_MODE) >= 0.5);
        self.drive_k = k;
        self.drive_n = n;
        self.ddrive_k = 0.0;
        self.ddrive_n = 0.0;
        self.rng = Rng::new(0xBA55);
        self.quiet = 0;
        self.silence();
        self.snap_pitch();
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
        // The longest release reaches silence at about 1.35 times its
        // label, then the quiet wait.
        ((self.v(RELEASE) * 1.4 + QUIET_MS) * 0.001 * self.rate as f32) as u32
    }

    fn set_tuning(&mut self, tuning: &Tuning) {
        if self.tuning == *tuning {
            return;
        }
        self.tuning.clone_from(tuning);
    }

    fn sounding_voices(&self) -> usize {
        self.active_voices()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::body808::{drive_curve, tanh_pade, DRIVE_BIAS, DRIVE_REF};

    const RATE: u32 = 48_000;

    #[test]
    fn subs_have_no_dc() {
        for tab in SUB_TABLES {
            assert!(tab.iter().sum::<f32>().abs() < 1e-6);
        }
    }

    #[test]
    fn the_drop_lands_on_the_note() {
        let mut b = Bass::new(RATE);
        b.set_param(DROP, 12.0);
        b.set_param(DROP_TIME, 40.0);
        b.set_param(PUNCH, 0.0);
        // 40 ms is 120 steps at 48 kHz.
        assert!((b.envelope_semis(0) - 12.0).abs() < 1e-4);
        assert_eq!(b.envelope_semis(120), 0.0);
        assert!(b.envelope_semis(60) > 0.0 && b.envelope_semis(60) < 6.0);
    }

    #[test]
    fn the_ladder_response_is_flat_below_cutoff() {
        let g = ladder_response(1_000.0, 0.0, 10.0);
        assert!((g - 1.0).abs() < 0.01, "{g}");
        // Four poles: -24 dB an octave well above.
        let a = ladder_response(1_000.0, 0.0, 8_000.0);
        let b = ladder_response(1_000.0, 0.0, 16_000.0);
        let slope = 20.0 * (b / a).log10();
        assert!((slope + 24.0).abs() < 1.0, "{slope}");
    }

    #[test]
    fn drive_keeps_the_reference_peak() {
        for d in [10.0, 50.0, 100.0] {
            let (k, n) = drive_gains(d, false);
            let y = n * (drive_curve(k * DRIVE_REF + DRIVE_BIAS) - drive_curve(DRIVE_BIAS));
            assert!((y - DRIVE_REF).abs() < 1e-5, "{d}: {y}");
            let (k, n) = drive_gains(d, true);
            let y = n * tanh_pade(k * DRIVE_REF);
            assert!((y - DRIVE_REF).abs() < 1e-5, "hard {d}: {y}");
        }
    }
}
