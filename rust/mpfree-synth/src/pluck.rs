//! Pluck: plucked strings and struck bars, two physical models behind one
//! set of pages.
//!
//! - **String** is a Karplus-Strong string (the Jaffe-Smith extension): a
//!   delay line one period long with a one-pole damping filter in the loop,
//!   tuned for the filter's phase delay so notes play in tune to a few
//!   cents. A pluck fills the line with a burst of noise shaped by the pick
//!   (its brightness) and the pick position (a comb that hollows the
//!   tone). Decay sets the ring at middle C; higher notes ring shorter.
//! - **Mallet** is a modal bank ([`crate::modal`], shared with the drums):
//!   up to eight two-pole resonators per voice at a material's mode ratios
//!   (marimba, vibes, bell, glass, kalimba, steel drum), struck by a
//!   half-sine mallet pulse. A harder mallet is a
//!   shorter pulse, so it reaches higher modes. Strike position, a stretch
//!   of the ratios and a high-mode damping shape the tone.
//!
//! Notes: a key held rings with Decay; letting go damps it over Release.
//! Playing a note that is still ringing re-plucks (or re-strikes) the same
//! voice, adding to what sounds, as a real string or bar does. Past the
//! Voices limit the oldest note fades out over 3 ms.
//!
//! Pitch (transpose, fine, bend) runs at a control rate of one step every
//! [`CTRL`] frames aligned to the absolute frame, so the output does not
//! depend on how the host cuts blocks. The delay lines are allocated in
//! `prepare`; nothing in `process` allocates.

use mpfree_node::{Ctx, Cull, Kind, Node, NodeEvent, NodeInfo, ParamInfo, Smooth, Timed, Tuning};

use crate::dsp::{note_hz, Rng};
use crate::expr::{expr_coef, NoteExprs, VoiceExpr};
use crate::modal::{
    mode_share, mode_t60, pulse_len, pulse_spectrum, stretched, strike_place, ModalBank, Modes,
    Pulse,
};
use crate::params::Preset;
use crate::pluck_params::*;
use crate::voice::velocity_gain;

/// Frames per control step.
pub const CTRL: u64 = 16;
/// The most notes sounding at once (the Voices parameter's top).
pub const MAX_POLY: usize = 16;
/// Voice slots: the notes plus room for stolen voices to fade out.
pub const POOL: usize = MAX_POLY + 4;
const KILL_MS: f32 = 3.0;
/// The lowest string pitch; lower notes sound at this pitch.
const MIN_HZ: f32 = 20.0;
/// Modes per mallet voice.
const MODES: usize = 8;
/// A voice ends when its level follower falls below this (-100 dB).
const SILENT: f32 = 1e-5;

static INFO: NodeInfo = NodeInfo {
    id: "mpfree.pluck",
    name: "Pluck",
    kind: Kind::Instrument,
};

/// The materials' modes, in the Material setting's order.
const MATERIAL_MODES: [Modes; 9] = [
    crate::modal::MARIMBA,
    crate::modal::VIBES,
    crate::modal::BELL,
    crate::modal::GLASS,
    crate::modal::KALIMBA,
    crate::modal::STEEL_DRUM,
    crate::modal::GLOCKENSPIEL,
    crate::modal::XYLOPHONE,
    crate::modal::CHIMES,
];

/// A material's modes, from the Material value.
fn material(value: f32) -> Modes {
    MATERIAL_MODES[(value as usize).min(MATERIAL_MODES.len() - 1)]
}

/// Decay (T60, seconds) at `hz` for a Decay of `decay_ms`: that at middle
/// C, halving every two octaves up.
fn decay_for(decay_ms: f32, hz: f32) -> f32 {
    decay_ms * 0.001 * (261.63 / hz).sqrt()
}

/// The string's loop filter coefficient for a note at `hz`. Brightness
/// (0 to 100) sets its cutoff, from 12 harmonics (dark) to 112. Never
/// lower, or the filter alone would kill the fundamental of a high note
/// within a few dozen periods.
fn loop_coef(brightness: f32, hz: f32, rate: f32) -> f32 {
    let b = brightness / 100.0;
    let fc = (hz * (12.0 + 100.0 * b * b)).min(0.45 * rate);
    (-std::f32::consts::TAU * fc / rate).exp()
}

/// The pick: a one-pole low-pass on the burst of white noise. Pick (0 to
/// 100) sets its cutoff in harmonics of the note, from 2 (a soft thumb) to
/// 181 (a hard plectrum), so a pick sounds alike on every note; a softer
/// note (`soft_oct`, octaves) lowers it. The pole's coefficient.
fn pick_coef(pick: f32, soft_oct: f32, hz: f32, rate: f32) -> f32 {
    let harmonics = (1.0 + 6.5 * (pick / 100.0).clamp(0.0, 1.0) - soft_oct).exp2();
    let fc = (hz * harmonics).min(0.45 * rate);
    (-std::f32::consts::TAU * fc / rate).exp()
}

/// How many octaves a velocity (0 to 1) lowers the pick's cutoff, from Vel >
/// tone (0 to 100): up to 4 at the softest note.
fn soft_octaves(vel_tone: f32, vel: f32) -> f32 {
    4.0 * (vel_tone / 100.0).clamp(0.0, 1.0) * (1.0 - vel).clamp(0.0, 1.0)
}

/// How much longer a velocity (0 to 1) makes the mallet's pulse than a
/// full-velocity one, from Vel > tone (0 to 100): up to 8 times at the
/// softest note (a softer hit stays on the bar longer, so it is darker).
fn soft_pulse(vel_tone: f32, vel: f32) -> f32 {
    (3.0 * (vel_tone / 100.0).clamp(0.0, 1.0) * (1.0 - vel).clamp(0.0, 1.0)).exp2()
}

/// Harmonics of a pluck's burst set to their average level: at most
/// [`FIX_HARMONICS`], and about [`FIX_WORK`] frames of work in all.
const FIX_HARMONICS: usize = 8;
const FIX_WORK: usize = 16384;

/// The rms of white noise of variance 1/3 through the pick's one-pole
/// low-pass (coefficient `c`, so its autocorrelation at lag j is `c^j`)
/// and then the comb with its tap `m` frames back (linear between frames).
fn comb_rms(c: f32, m: f32) -> f32 {
    let var = (1.0 / 3.0) * (1.0 - c) / (1.0 + c);
    let k = m.floor();
    let f = m - k;
    let rho = |j: f32| if j <= 0.0 { 1.0 } else { c.powf(j) };
    let e = 1.0 + (1.0 - f) * (1.0 - f) + f * f - 2.0 * (1.0 - f) * rho(k) - 2.0 * f * rho(k + 1.0)
        + 2.0 * f * (1.0 - f) * rho(1.0);
    (var * e.max(0.0)).sqrt()
}

/// The pick position's comb, `1 - H` with `H` the tap `m` frames back
/// (linear between frames, as `render` reads it), at `w` radians a frame:
/// its magnitude.
fn comb_mag(m: f32, w: f32) -> f32 {
    let k = m.floor();
    let f = m - k;
    // H = e^{-jwk} ((1 - f) + f e^{-jw})
    let (re0, im0) = ((1.0 - f) + f * w.cos(), -f * w.sin());
    let (c, s) = ((w * k).cos(), (-w * k).sin());
    let (re, im) = (re0 * c - im0 * s, re0 * s + im0 * c);
    ((1.0 - re) * (1.0 - re) + im * im).sqrt()
}

/// How long the mallet's pulse lasts on a full-velocity note `note`, in
/// seconds, with the values `value` gives by parameter id (for drawing it).
pub fn mallet_pulse_secs(value: impl Fn(u16) -> f32, note: u8, rate: u32) -> f32 {
    let pitch = f32::from(note) + value(TRANSPOSE) + value(FINE) / 100.0;
    pulse_len(value(HARDNESS), 1.0, note_hz(pitch), rate as f32) / rate as f32
}

/// One partial of a note, for drawing it (`mpfree-synth-ui`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Partial {
    /// Its frequency over the note's.
    pub ratio: f32,
    /// Its level at the start of a full-velocity note, the loudest 1.
    pub level: f32,
    /// How long it rings while held, to -60 dB, in seconds.
    pub t60: f32,
}

/// How many harmonics [`partials`] gives for a string.
pub const HARMONICS: usize = 16;

/// The partials of a full-velocity note `note` held at `rate`, with the
/// values `value` gives by parameter id, worked out with the same formulas
/// the voices use:
///
/// - String: the first [`HARMONICS`] harmonics, each at its average level
///   over many plucks (each pluck's noise burst gives the harmonics random
///   levels around it), shaped by the pick's low-pass and the pick
///   position's comb, and each with the decay the loop filter gives it
///   (higher harmonics die sooner, more so when darker).
/// - Mallet: the material's modes, weighed by where they are struck and by
///   the mallet pulse's spectrum (a soft mallet barely reaches the high
///   modes), with their own decays.
///
/// A partial at or over the engine's top frequency has level 0. Writes into
/// `out` and returns how many it wrote.
pub fn partials(value: impl Fn(u16) -> f32, note: u8, rate: u32, out: &mut [Partial]) -> usize {
    use std::f32::consts::TAU;
    let rate = rate as f32;
    let pitch = f32::from(note) + value(TRANSPOSE) + value(FINE) / 100.0;
    let mut n = 0;
    if value(MODEL) < 0.5 {
        let hz = note_hz(pitch).max(MIN_HZ).min(rate * 0.25);
        let a = loop_coef(value(BRIGHTNESS), hz, rate);
        let mag = |w: f32| (1.0 - a) / (1.0 - 2.0 * a * w.cos() + a * a).sqrt();
        let per_period = 10f32.powf(-3.0 / (decay_for(value(DECAY), hz) * hz));
        let g = (per_period / mag(TAU * hz / rate)).min(0.99999);
        let c = pick_coef(value(PICK), 0.0, hz, rate);
        // The comb's tap, a share of the string's period back.
        let m = value(POSITION) / 100.0 * rate / hz;
        for (k, p) in out.iter_mut().take(HARMONICS).enumerate() {
            let h = (k + 1) as f32;
            let w = TAU * hz * h / rate;
            let (level, t60) = if hz * h >= 0.45 * rate {
                (0.0, 0.0)
            } else {
                let pick = (1.0 - c) / (1.0 - 2.0 * c * w.cos() + c * c).sqrt();
                // The burst is white noise through the pick; the comb on
                // the string's output takes out the harmonics with a node
                // at the pick (exactly, up to the tap's interpolation).
                let loss = (g * mag(w)).min(0.99999);
                (pick * comb_mag(m, w), -3.0 / (hz * loss.log10()))
            };
            *p = Partial {
                ratio: h,
                level,
                t60,
            };
            n += 1;
        }
    } else {
        let modes = material(value(MATERIAL));
        let stretch = value(STRETCH) / 100.0;
        let damp = value(HIGH_DAMP) / 100.0;
        let pos = value(STRIKE) / 100.0;
        let f0 = note_hz(pitch);
        let base = decay_for(value(DECAY), f0);
        let share = mode_share(modes);
        // The half-sine pulse's spectrum, 1 at 0 Hz.
        let t = pulse_len(value(HARDNESS), 1.0, f0, rate) / rate;
        let pulse = |f: f32| pulse_spectrum(f, t);
        for (k, (&(ratio, level, dr), p)) in modes.iter().zip(out.iter_mut()).enumerate() {
            let r = stretched(ratio, stretch);
            let hz = f0 * r;
            let live = hz < 0.45 * rate && hz > 10.0;
            *p = Partial {
                ratio: r,
                level: if live {
                    level * strike_place(k, pos) * share * pulse(hz)
                } else {
                    0.0
                },
                t60: if live { mode_t60(base, dr, damp) } else { 0.0 },
            };
            n += 1;
        }
    }
    let top = out[..n].iter().fold(0f32, |m, p| m.max(p.level));
    if top > 0.0 {
        for p in &mut out[..n] {
            p.level /= top;
        }
    }
    n
}

/// How loud a K-weighted power at `hz` counts (ITU-R BS.1770's weighting,
/// as analog curves): its high-pass at 38 Hz and its +4 dB shelf above
/// about 1.7 kHz.
fn k_weight2(hz: f32) -> f32 {
    let (f0, q) = (38.135f32, 0.5003f32);
    let f2 = hz * hz;
    let hp = f2 * f2 / ((f0 * f0 - f2).powi(2) + (hz * f0 / q).powi(2));
    let fc2 = 1_682.0f32 * 1_682.0;
    let shelf = (fc2 + 2.512 * f2) / (fc2 + f2);
    hp * shelf
}

/// Blocks of the loudness measure (BS.1770, as the lab's): 400 ms long,
/// every 100 ms, over the first 1.5 s of a held note.
const BLOCKS: usize = 12;

/// Adds a partial whose K-weighted power starts at `power` and falls 60 dB
/// in `t60` seconds to each loudness block's mean power.
fn add_partial(blocks: &mut [f32; BLOCKS], power: f32, t60: f32) {
    // Amplitude e^(-t / tau), so power e^(-2 t / tau).
    let tau = (t60 / std::f32::consts::LN_10 / 3.0).max(1e-6);
    let step = (-0.2 / tau).exp();
    let mut e = power * tau * 0.5 * (1.0 - (-0.8 / tau).exp()) / 0.4;
    for b in blocks.iter_mut() {
        *b += e;
        e *= step;
    }
}

/// The loudness of a note from its blocks, gated as BS.1770 gates: blocks
/// 40 dB under the loudest stand in for its absolute gate (a note in the
/// lab is about 40 dB over it), then those 10 dB under the rest's mean
/// are left out. So a short note is heard over its first block, not
/// averaged with the silence after it.
fn gated(blocks: &[f32; BLOCKS]) -> f32 {
    let top = blocks.iter().copied().fold(0.0f32, f32::max);
    let mean_over = |gate: f32| {
        let (sum, n) = blocks
            .iter()
            .filter(|&&b| b > gate)
            .fold((0.0f32, 0u32), |(s, n), &b| (s + b, n + 1));
        if n > 0 {
            sum / n as f32
        } else {
            0.0
        }
    };
    let floor = top * 1e-4;
    mean_over((mean_over(floor) * 0.1).max(floor))
}

/// The most the voicing across the keys lifts or lowers a note, dB.
const EVEN_DB: f32 = 9.0;

/// How loud a full-velocity note at `hz` is, as the loudness measure hears
/// a held note (K-weighted, in gated 400 ms blocks over its first 1.5 s:
/// [`gated`]), with the values `value` gives by parameter id, in
/// the units of a note whose power starts at 1 (a string's burst is set
/// to the same power on every note; a mallet's modes from the pulse). The
/// engine plays each note at the level that makes this the same as at
/// middle C's, so the keys sound even (row L11 of the lab) whatever the
/// knobs: higher notes ring shorter, and the weighting hears little of
/// the lowest fundamentals.
pub fn note_energy(value: impl Fn(u16) -> f32, hz: f32, rate: f32) -> f32 {
    use std::f32::consts::TAU;
    if value(MODEL) < 0.5 {
        let hz = hz.max(MIN_HZ).min(rate * 0.25);
        let a = loop_coef(value(BRIGHTNESS), hz, rate);
        let mag = |w: f32| (1.0 - a) / (1.0 - 2.0 * a * w.cos() + a * a).sqrt();
        let per_period = 10f32.powf(-3.0 / (decay_for(value(DECAY), hz) * hz));
        let g = (per_period / mag(TAU * hz / rate)).min(0.99999);
        let c = pick_coef(value(PICK), 0.0, hz, rate);
        let m = value(POSITION) / 100.0 * rate / hz;
        let top = 0.5 * rate / hz;
        // The comb repeats every 1 / position harmonics.
        let comb_period = 100.0 / value(POSITION).max(1.0);
        let mut power = 0.0f32;
        let mut blocks = [0.0f32; BLOCKS];
        // The first 12 harmonics one by one; above, one in each quarter
        // octave stands for the harmonics around it, and where those span
        // the comb's notches they average out (to 2).
        let mut h = 1.0f32;
        while h < top {
            let step = if h < 12.5 {
                1.0
            } else {
                (h * (std::f32::consts::SQRT_2.sqrt() - 1.0)).round()
            };
            let w = TAU * hz * h / rate;
            let comb2 = if step * 2.0 < comb_period {
                comb_mag(m, w).powi(2)
            } else {
                2.0
            };
            let pick2 = (1.0 - c) * (1.0 - c) / (1.0 - 2.0 * c * w.cos() + c * c);
            let p = pick2 * comb2 * step;
            let loss = (g * mag(w)).min(0.99999);
            let t60 = -3.0 / (hz * loss.log10());
            power += p;
            add_partial(&mut blocks, p * k_weight2(hz * h), t60);
            h += step;
        }
        if power > 0.0 {
            gated(&blocks) / power
        } else {
            0.0
        }
    } else {
        let modes = material(value(MATERIAL));
        let (stretch, damp, pos) = (
            value(STRETCH) / 100.0,
            value(HIGH_DAMP) / 100.0,
            value(STRIKE) / 100.0,
        );
        let base = decay_for(value(DECAY), hz);
        let share = mode_share(modes);
        let t = pulse_len(value(HARDNESS), 1.0, hz, rate) / rate;
        let mut blocks = [0.0f32; BLOCKS];
        for (k, &(ratio, level, dr)) in modes.iter().take(MODES).enumerate() {
            let f = hz * stretched(ratio, stretch);
            if f >= 0.45 * rate || f <= 10.0 {
                continue;
            }
            let amp = level * strike_place(k, pos) * share * pulse_spectrum(f, t);
            add_partial(
                &mut blocks,
                amp * amp * k_weight2(f),
                mode_t60(base, dr, damp),
            );
        }
        gated(&blocks)
    }
}

/// The level that evens a note at `hz` out with middle C, from their
/// [`note_energy`]s, within [`EVEN_DB`].
fn even_gain(energy: f32, at_c4: f32) -> f32 {
    if energy <= 0.0 || at_c4 <= 0.0 {
        return 1.0;
    }
    let lim = 10f32.powf(EVEN_DB / 20.0);
    (at_c4 / energy).sqrt().clamp(1.0 / lim, lim)
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
    vel: f32,
    age: u64,
    /// Pitch the voice is tuned to now (fractional MIDI note), or NaN to
    /// force a retune.
    tuned: f32,
    /// Its note's own slide (tone isn't used yet).
    ex: VoiceExpr,
    gain_l: f32,
    gain_r: f32,
    /// +1 or -1: alternate voices strike the other way, so the attacks of
    /// a chord's bars don't all peak together (one note alone sounds the
    /// same either way).
    sign: f32,
    kill: f32,
    kill_step: f32,
    /// Level follower, to end the voice once it is silent.
    follow: f32,
    frames: u32,
    // String.
    w: usize,
    delay: f32,
    a: f32,
    lp: f32,
    g: f32,
    /// The fractional part of the delay: a first-order allpass (Thiran)
    /// with coefficient `ap_c`, and its state.
    ap_c: f32,
    ap_x1: f32,
    ap_y1: f32,
    /// The pick position (a share of the string, from the pluck) and the
    /// comb's tap: whole frames back and the fraction past them.
    pos: f32,
    comb_k: usize,
    comb_f: f32,
    /// Frames until the comb's tap reaches the pluck in a string that was
    /// silent before it (the tap reads silence until then).
    quiet: u32,
    dc_x: f32,
    dc_y: f32,
    // Mallet.
    bank: ModalBank<MODES>,
    pulse: Pulse,
}

/// The pluck synth. See the module docs.
pub struct Pluck {
    rate: u32,
    values: [f32; ID_SPACE],
    voices: [Voice; POOL],
    /// One delay line per voice, `line` frames each.
    lines: Vec<f32>,
    line: usize,
    scratch: Vec<f32>,
    clock: u64,
    rng: Rng,
    bend: Smooth,
    bend_value: f32,
    out_l: Smooth,
    out_r: Smooth,
    next_pan: u32,
    /// How notes become frequencies.
    tuning: Tuning,
    /// Each note's slide and tone for its next note-on, and the voices'
    /// smoothing coefficient.
    exprs: NoteExprs,
    expr_c: f32,
    /// Middle C's [`note_energy`] for the knobs now, and each note's
    /// level to even it out with middle C ([`even_gain`]), or NaN when a
    /// knob or the tuning moved since (worked out again at the next note).
    c4_energy: f32,
    evens: [f32; 128],
}

impl Pluck {
    pub fn new(rate: u32) -> Pluck {
        let mut values = [0.0; ID_SPACE];
        for p in PARAMS {
            values[p.id as usize] = p.default;
        }
        let mut s = Pluck {
            rate,
            values,
            voices: [Voice::default(); POOL],
            lines: Vec::new(),
            line: 0,
            scratch: Vec::new(),
            clock: 0,
            rng: Rng::new(0x9_1C4),
            bend: Smooth::new(0.0),
            bend_value: 0.0,
            out_l: Smooth::new(0.0),
            out_r: Smooth::new(0.0),
            next_pan: 0,
            tuning: Tuning::standard(),
            exprs: NoteExprs::new(),
            expr_c: 1.0,
            c4_energy: f32::NAN,
            evens: [f32::NAN; 128],
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
        crate::factory::presets!("pluck.presets", &INFO, PARAMS)
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

    fn mallet(&self) -> bool {
        self.v(MODEL) >= 0.5
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
    }

    /// The pitch a voice plays now, in fractional MIDI notes.
    fn pitch(&self, note: u8) -> f32 {
        f32::from(note)
            + self.v(TRANSPOSE)
            + self.v(FINE) / 100.0
            + self.bend.value * self.v(BEND_RANGE)
    }

    /// The pitch a voice plays now: its note's, plus its own slide.
    fn voice_pitch(&self, note: u8, ex: &VoiceExpr) -> f32 {
        let p = self.pitch(note);
        if ex.slide != 0.0 {
            p + ex.slide
        } else {
            p
        }
    }

    /// Decay (T60, seconds) at `hz`: the Decay value at middle C, halving
    /// every two octaves up.
    fn decay_at(&self, hz: f32, held: bool) -> f32 {
        let d = decay_for(self.v(DECAY), hz);
        if held {
            d
        } else {
            d.min(self.v(RELEASE) * 0.001)
        }
    }

    /// The level that makes `note` as loud as middle C ([`note_energy`]),
    /// at its pitch without bend or slide; kept until a knob or the
    /// tuning moves.
    fn even(&mut self, note: u8) -> f32 {
        let i = usize::from(note.min(127));
        if self.evens[i].is_nan() {
            let rate = self.rate as f32;
            if self.c4_energy.is_nan() {
                self.c4_energy = note_energy(|id| self.v(id), 261.63, rate);
            }
            let pitch = f32::from(note) + self.v(TRANSPOSE) + self.v(FINE) / 100.0;
            let hz = self.tuning.hz(pitch);
            self.evens[i] = even_gain(note_energy(|id| self.v(id), hz, rate), self.c4_energy);
        }
        self.evens[i]
    }

    /// Forgets the evening levels (a knob or the tuning moved).
    fn forget_evens(&mut self) {
        self.c4_energy = f32::NAN;
        self.evens = [f32::NAN; 128];
    }

    /// Sets a voice's filters for its pitch and gate.
    fn tune(&self, v: &mut Voice, pitch: f32) {
        let rate = self.rate as f32;
        let held = v.state == State::On;
        if !self.mallet() {
            let hz = self.tuning.hz(pitch).max(MIN_HZ).min(rate * 0.25);
            let w = std::f32::consts::TAU * hz / rate;
            let a = loop_coef(self.v(BRIGHTNESS), hz, rate);
            let (s, c) = w.sin_cos();
            let phase_delay = (a * s).atan2(1.0 - a * c) / w;
            let mag = (1.0 - a) / (1.0 - 2.0 * a * c + a * a).sqrt();
            v.a = a;
            v.delay = (rate / hz - phase_delay).clamp(1.5, (self.line - 2) as f32);
            let d = v.delay - (v.delay - 0.5).floor().max(1.0);
            v.ap_c = (1.0 - d) / (1.0 + d);
            let per_period = 10f32.powf(-3.0 / (self.decay_at(hz, held) * hz));
            v.g = (per_period / mag).min(0.99999);
            // The comb's tap: the pick position's share of the note's
            // period, so its notches land on the harmonics the string plays.
            let m = (v.pos * rate / hz).clamp(0.0, (self.line / 2) as f32);
            v.comb_k = m as usize;
            v.comb_f = m - v.comb_k as f32;
        } else {
            let modes = material(self.v(MATERIAL));
            let stretch = self.v(STRETCH) / 100.0;
            let damp = self.v(HIGH_DAMP) / 100.0;
            let pos = self.v(STRIKE) / 100.0;
            let f0 = self.tuning.hz(pitch);
            let base = self.decay_at(f0, held);
            v.bank.set_len(modes.len().min(MODES));
            let share = mode_share(modes);
            for (k, &(ratio, level, dr)) in modes.iter().take(MODES).enumerate() {
                let hz = f0 * stretched(ratio, stretch);
                let t60 = mode_t60(base, dr, damp);
                v.bank.set_with(k, hz, t60, rate, |s| {
                    s * level * strike_place(k, pos) * share
                });
            }
        }
        v.tuned = pitch;
    }

    fn note_on(&mut self, note: u8, velocity: u8) {
        if velocity == 0 {
            self.note_off(note);
            return;
        }
        let vel = f32::from(velocity.min(127)) / 127.0;
        self.clock += 1;
        // A ringing note is re-plucked in its own voice.
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
                    // Steal the oldest, released notes first.
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
                let mut v = Voice {
                    tuned: f32::NAN,
                    w: 0,
                    ..Voice::default()
                };
                // Spread alternates voices left and right of the pan.
                let side = if self.next_pan.is_multiple_of(2) {
                    -1.0
                } else {
                    1.0
                };
                self.next_pan = self.next_pan.wrapping_add(1);
                let pan = (side * self.v(SPREAD) / 100.0
                    * (0.4 + 0.6 * ((self.next_pan / 2) % 3) as f32 / 2.0))
                    .clamp(-1.0, 1.0);
                v.gain_l = (1.0 - pan).sqrt() * std::f32::consts::FRAC_1_SQRT_2;
                v.gain_r = (1.0 + pan).sqrt() * std::f32::consts::FRAC_1_SQRT_2;
                v.sign = if side < 0.0 { 1.0 } else { -1.0 };
                let base = free * self.line;
                self.lines[base..base + self.line].fill(0.0);
                self.voices[free] = v;
                free
            }
        };
        let mut v = self.voices[slot];
        v.state = State::On;
        v.note = note;
        v.vel = vel;
        v.age = self.clock;
        v.frames = 0;
        v.ex.start(self.exprs.on(note));
        v.pos = self.v(POSITION) / 100.0;
        let pitch = self.voice_pitch(note, &v.ex);
        self.tune(&mut v, pitch);
        // Velocity: the level on Velocity's square law, and the tone by
        // Vel > tone.
        let rate = self.rate as f32;
        let amp = velocity_gain(velocity, self.v(VELOCITY)) * self.even(note);
        let vt = self.v(VEL_TONE);
        if self.mallet() {
            let hz = self.tuning.hz(pitch);
            // A soft hit's pulse stops at half the note's period, where it
            // still drives the fundamental at 0.8.
            let hard = pulse_len(self.v(HARDNESS), 1.0, hz, rate);
            let len = (hard * soft_pulse(vt, vel)).min(0.5 * rate / hz).max(hard);
            // Unit area, so the low modes sound the same at any hardness.
            v.pulse.start(
                len as u32,
                v.sign * amp * std::f32::consts::FRAC_PI_2 / len * 0.42,
            );
        } else {
            let hz = self.tuning.hz(pitch).max(MIN_HZ).min(rate * 0.25);
            let c = pick_coef(self.v(PICK), soft_octaves(vt, vel), hz, rate);
            // A string that was silent had nothing to subtract a share of
            // a period ago, so the note starts from silence, not on a step.
            if same.is_none() {
                v.quiet = v.comb_k as u32 + 1;
            }
            self.pluck(slot, &v, hz, amp, c);
        }
        self.voices[slot] = v;
    }

    /// Adds a pluck's noise burst into voice `slot`'s delay line, a period
    /// long, ending where the line is read next: white noise through the
    /// pick's low-pass (coefficient `c`), at `amp` once through the comb.
    fn pluck(&mut self, slot: usize, v: &Voice, hz: f32, amp: f32, c: f32) {
        let period = (v.delay.round() as usize).clamp(2, self.line - 2);
        let mut lp = 0.0f32;
        let mut sum = 0.0f32;
        for i in 0..period {
            lp = (1.0 - c) * self.rng.bipolar() + c * lp;
            self.scratch[i] = lp;
            sum += lp;
        }
        let mean = sum / period as f32;
        for x in &mut self.scratch[..period] {
            *x -= mean;
        }
        // A burst of noise gives the low harmonics random levels, so one
        // pluck could come out 5 dB louder than the next. Set the first few
        // to their average level (their phases stay random, and the upper
        // harmonics stay noise): fewer for a long, low string, where more
        // harmonics share the sound.
        let pf = period as f32;
        let top = ((0.45 * self.rate as f32 / hz) as usize).max(1);
        let fix = (FIX_WORK / period).clamp(2, FIX_HARMONICS).min(top);
        for h in 1..=fix {
            // The string's own harmonic, which needn't be a whole number
            // of cycles in the burst (the loop's filters hold the rest of
            // the period).
            let w = std::f32::consts::TAU * h as f32 * hz / self.rate as f32;
            let (sw, cw) = w.sin_cos();
            let (mut re, mut im) = (0.0f32, 0.0f32);
            let (mut co, mut si) = (1.0f32, 0.0f32);
            for &x in &self.scratch[..period] {
                re += x * co;
                im -= x * si;
                (co, si) = (co * cw - si * sw, si * cw + co * sw);
            }
            let mag = (re * re + im * im).sqrt();
            // White noise of variance 1/3 through the pick, on average.
            let want = (pf / 3.0).sqrt() * (1.0 - c) / (1.0 - 2.0 * c * cw + c * c).sqrt();
            if mag > 1e-9 {
                let k = (want / mag - 1.0) * 2.0 / pf;
                let (ar, ai) = (k * re, k * im);
                let (mut co, mut si) = (1.0f32, 0.0f32);
                for x in &mut self.scratch[..period] {
                    *x += ar * co - ai * si;
                    (co, si) = (co * cw - si * sw, si * cw + co * sw);
                }
            }
        }
        // The pick position's comb runs on the string's output (`render`),
        // so its notches are exact. The level is set for what comes out of
        // it on average (the pick's noise through the comb), not for this
        // burst, so plucks don't differ in level by chance.
        let norm = amp * 0.32 / comb_rms(c, v.comb_k as f32 + v.comb_f).max(1e-6);
        let base = slot * self.line;
        let mask = self.line - 1;
        for i in 0..period {
            let at = (v.w + self.line - period + i) & mask;
            self.lines[base + at] += self.scratch[i] * norm * 0.5;
        }
    }

    fn note_off(&mut self, note: u8) {
        self.exprs.off(note);
        for i in 0..POOL {
            if self.voices[i].note == note && self.voices[i].state == State::On {
                let mut v = self.voices[i];
                v.state = State::Release;
                let pitch = self.voice_pitch(note, &v.ex);
                self.tune(&mut v, pitch);
                self.voices[i] = v;
            }
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

    /// The voice load shedding gives up first: the least audible one
    /// ringing ([`Cull::audibility`]), by its level follower. Voices
    /// already fading are left be.
    fn cull_pick(&self) -> Option<(usize, Cull)> {
        let mut best = None;
        for (i, v) in self.voices.iter().enumerate() {
            let releasing = match v.state {
                State::On => false,
                State::Release => true,
                State::Free | State::Kill => continue,
            };
            // A mallet still in its pulse has not rung up yet: count it
            // as loud as it will be.
            let level = if v.pulse.active() {
                v.pulse.amp.max(v.follow).max(1.0)
            } else {
                v.follow
            };
            best = Cull::first(best, Some((i, Cull { level, releasing })));
        }
        best
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
                // One note's own slide: its held voices glide to it.
                let Some(value) = self.exprs.set(note, kind, value) else {
                    return;
                };
                for v in &mut self.voices {
                    if v.state == State::On && v.note == note {
                        v.ex.aim(kind, value);
                    }
                }
            }
            // The pedals are held by the host for this engine (`Node::hears`).
            NodeEvent::Control { .. } => {}
        }
    }

    /// One control step: smooth the bend, retune voices whose pitch moved.
    fn control(&mut self) {
        self.bend.next();
        for i in 0..POOL {
            // Look before copying: a voice is a few hundred bytes, and
            // most steps retune nothing.
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
    }

    fn render(&mut self, from: usize, to: usize, left: &mut [f32], right: &mut [f32]) {
        let mallet = self.mallet();
        let line = self.line;
        let mask = line - 1;
        for i in from..to {
            let gl = self.out_l.next();
            let gr = self.out_r.next();
            for (vi, v) in self.voices.iter_mut().enumerate() {
                if v.state == State::Free {
                    continue;
                }
                let mut y = if mallet {
                    let x = v.pulse.next();
                    v.bank.tick(x)
                } else {
                    let buf = &mut self.lines[vi * line..(vi + 1) * line];
                    // The whole frames of delay, then the allpass for the
                    // rest (0.5 to 1.5 frames, where it is most even).
                    let n = (v.delay - 0.5).floor().max(1.0) as usize;
                    let x = buf[(v.w + line - n) & mask];
                    let ap = v.ap_c * x + v.ap_x1 - v.ap_c * v.ap_y1;
                    v.ap_x1 = x;
                    v.ap_y1 = ap;
                    v.lp = (1.0 - v.a) * ap + v.a * v.lp;
                    let out = v.g * v.lp;
                    buf[v.w] = out;
                    // The pick position's comb: less the string as it was
                    // that share of a period ago.
                    let at = v.w + line - v.comb_k;
                    let tap = if v.quiet == 0 {
                        let a = buf[at & mask];
                        a + v.comb_f * (buf[(at - 1) & mask] - a)
                    } else {
                        // A fresh string: nothing there yet, then only the
                        // first frame of the pluck.
                        v.quiet -= 1;
                        if v.quiet == 0 {
                            buf[at & mask] * (1.0 - v.comb_f)
                        } else {
                            0.0
                        }
                    };
                    v.w = (v.w + 1) & mask;
                    out - tap
                };
                // A DC blocker (5 Hz) keeps an asymmetric pluck centred.
                let dc = y - v.dc_x + 0.9993 * v.dc_y;
                v.dc_x = y;
                v.dc_y = dc;
                y = dc;
                if v.state == State::Kill {
                    y *= v.kill;
                    v.kill -= v.kill_step;
                    if v.kill <= 0.0 {
                        v.state = State::Free;
                    }
                }
                let a = y.abs();
                v.follow = if a > v.follow { a } else { v.follow * 0.9995 };
                v.frames = v.frames.saturating_add(1);
                if v.follow < SILENT && v.frames > 256 && !v.pulse.active() {
                    v.state = State::Free;
                }
                left[i] += y * v.gain_l * gl;
                right[i] += y * v.gain_r * gr;
            }
        }
    }
}

impl Node for Pluck {
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
        if self.values[id as usize] != old
            && !matches!(
                id,
                LEVEL | PAN | SPREAD | POLYPHONY | BEND_RANGE | VEL_TONE | VELOCITY
            )
        {
            self.forget_evens();
        }
        if id == MODEL && self.values[id as usize] != old {
            // A voice can't change model while it rings: fade them out.
            for i in 0..POOL {
                self.kill(i);
            }
        }
        match id {
            LEVEL | PAN => self.update_out(),
            // Anything that shapes a ringing voice: retune at the next step.
            DECAY | RELEASE | BRIGHTNESS | MATERIAL | STRETCH | HIGH_DAMP | STRIKE => {
                for v in self.voices.iter_mut() {
                    v.tuned = f32::NAN;
                }
            }
            _ => {}
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
        self.forget_evens();
        self.line = ((rate as f32 / MIN_HZ) as usize + 4).next_power_of_two();
        self.lines = vec![0.0; self.line * POOL];
        self.scratch = vec![0.0; self.line];
        let cr = rate / CTRL as u32;
        self.bend.set_time(cr, 4.0);
        self.expr_c = expr_coef(cr as f32);
        self.out_l.set_time(rate, 10.0);
        self.out_r.set_time(rate, 10.0);
        self.reset();
    }

    fn reset(&mut self) {
        for v in self.voices.iter_mut() {
            *v = Voice::default();
        }
        self.lines.fill(0.0);
        self.update_out();
        self.out_l.snap(self.out_l.target);
        self.out_r.snap(self.out_r.target);
        self.bend.snap(self.bend_value);
        self.exprs.clear();
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
        self.forget_evens();
        // Every voice retunes on the next control step.
        for v in self.voices.iter_mut() {
            v.tuned = f32::NAN;
        }
    }

    fn sounding_voices(&self) -> usize {
        self.active_voices()
    }

    fn cull_candidate(&self) -> Option<Cull> {
        self.cull_pick().map(|(_, c)| c)
    }

    fn cull(&mut self) -> bool {
        let Some((i, _)) = self.cull_pick() else {
            return false;
        };
        self.kill(i);
        true
    }
}
