//! Wind: blown instruments as digital waveguides, played by the breath,
//! built from `docs/dev/research/wind.md` on the shared voice core.
//!
//! - **The bore** is a delay line as long as the note's period (half of it
//!   for the clarinet) with a loss at the open end: a one-pole low-pass
//!   whose corner follows the note (**Tone**: the same number of harmonics
//!   on every key), inverting as an open end does. The sound is what the
//!   bell lets out: the wave reaching it less what it sends back, so a
//!   high-pass, as Smith's waveguide models take it. Delays are read with
//!   cubic Lagrange interpolation, and each bore's length takes off the
//!   low-pass's phase delay at the note, so it plays in tune.
//! - **Clarinet**: a cylinder closed by a reed (Smith's reed table: the
//!   reed reflects the wave by `1 - m (hc - h)` of the pressure across it,
//!   fully once it closes), its two corners rounded so the reed curls shut
//!   rather than slapping and feeds no harmonics past the output's
//!   Nyquist. A closed cylinder has the odd harmonics only.
//! - **Sax**: a cone. A cone's input impedance is that of a pipe open at
//!   both ends driven a little way in, the missing tip of the cone being
//!   the short side (our derivation; Scavone's thesis models the cone with
//!   the same equivalence), so the reed sits between two delays in
//!   parallel, one for the tip and one for the bore to the bell. Every
//!   harmonic sounds. The tip is a third of the period: shorter tips let
//!   the bore jump to its octave (measured in the prototype).
//! - **Flute**: a pipe open at both ends blown across an edge: the air jet
//!   takes half a period to cross the mouth hole, the field in the pipe
//!   deflects it, and a smooth curve (`tanh`) says how much of it goes in.
//!   **Edge** aims the jet off centre, which brings in the even harmonics.
//! - **Breath** is the instrument. Each voice's breath is its envelope
//!   (Attack, Decay, Sustain, Release on [`Adsr`]) times its source: the
//!   note's velocity, or a breath controller, the mod wheel or a foot pedal
//!   on MIDI in (**Breath from**), all scaled by **Breath**; pad pressure
//!   adds to it through its slots (slot 1 on Breath by default). The breath
//!   sets the mouth pressure (inside the window where the reed or jet
//!   speaks) and the level: **Velocity** is how much quieter the softest
//!   breath is than the hardest, on the voice core's square law, with each
//!   bore's own level taken out first, so a harder breath is louder and
//!   brighter by design. Each octave below middle C plays 0.4 dB louder
//!   (above, softer), so the keys sound even to the ear. A note from
//!   silence starts with its bore primed, so it speaks within 30 ms.
//! - **Noise** is the breath's turbulence (in the mouth pressure, and for
//!   the flute in the jet and the air heard outside); **Growl** a rough
//!   flutter in it. **Tongue**: a tongued note starts with a puff of breath
//!   and a burst of noise. **Vibrato** (Depth, Rate, Delay, on the voice
//!   core's [`Lfo`]) moves the breath and the pitch, in each model's own
//!   mix; the mod wheel adds to its depth.
//! - **Playing.** One voice by default (**Voices** up to 8). With one
//!   voice, a key over a held key slurs to it (the breath carries on) or is
//!   tongued again (**Legato**), gliding by **Glide** (time or rate, the
//!   voice core's [`Glide`]) and never faster than 10 ms; **Priority**
//!   last, low or high ([`NoteStack`]). With more, each note takes a voice
//!   ([`Voices`]), stolen voices fading over 3 ms, and each plays at
//!   `1/sqrt(voices)` so a full chord stays under full scale.
//!
//! Below 64 kHz the bores run at twice the rate and come down through
//! Squelch's polyphase half-band. The breath, pitch, delays and gains run at
//! a control step of [`CTRL`] frames aligned to the absolute frame, the
//! delays, mouth pressure and gain moving linearly across the step, so the
//! output does not depend on how the host cuts blocks. A tenth of a second
//! after a voice falls silent it stops running; with none sounding the
//! output is exact silence. The delay lines are allocated in `prepare`;
//! nothing allocates while sounding.

use std::f32::consts::{PI, TAU};

use mpfree_node::pressure::{Pressure, PressureChord, PressureCoefs, PressureMode, PressureShape};
use mpfree_node::{Ctl, Ctx, Kind, Node, NodeEvent, NodeInfo, ParamInfo, Timed, Tuning};

use crate::dsp::{LfoShape, Rng};
use crate::params::Preset;
use crate::voice::{Adsr, Channel, Glide, Lfo, NoteStack, Phase, Priority, Voices};
use crate::wind_params::*;

/// Frames per control step.
pub const CTRL: u64 = 16;

static INFO: NodeInfo = NodeInfo {
    id: "mpfree.wind",
    name: "Wind",
    kind: Kind::Instrument,
};

/// Voices the player may ask for.
pub const MAX_VOICES: usize = 8;
/// Slots: the voices and room for stolen ones to fade out in.
const SLOTS: usize = MAX_VOICES + 4;
/// The lowest pitch a bore holds; lower notes play at it.
pub const LOWEST_HZ: f32 = 25.0;
/// A stolen voice's fade.
const FADE_MS: f32 = 3.0;
/// How long a voice stays silent before it stops running.
const QUIET_MS: f32 = 100.0;
/// Under this (about -86 dBFS) a voice counts as silent.
const QUIET_LEVEL: f32 = 5e-5;
/// A slur never moves faster than this.
const MIN_MOVE_MS: f32 = 10.0;
/// A tongued start's puff dies away with this time constant.
const TONGUE_MS: f32 = 18.0;
/// How much the puff adds to the breath at Tongue 100 (half again).
const TONGUE_BREATH: f32 = 0.5;
/// A bore's priming at the start of a note from silence, as a share of
/// the mouth pressure, and the more Tongue 100 adds; each bore scales it
/// by its own `prime` (ours: at 0.25 the clarinet starts at its full
/// swing, at 0.1 it takes about 15 ms at C4).
const PRIME: f32 = 0.1;
const PRIME_TONGUE: f32 = 0.15;
/// The growl's rate.
const GROWL_HZ: f32 = 31.0;
/// How deep the growl moves the mouth pressure, at 100.
const GROWL_DEPTH: f32 = 0.45;

// Half-band coefficients (Squelch's), for the 2x path.
const HB_EVEN: [f32; 3] = [0.060_297_39, 0.412_590_7, 0.772_715_6];
const HB_ODD: [f32; 3] = [0.215_971_44, 0.604_358_6, 0.923_886_1];

/// One bore's constants. "Ours" where measured in the prototype, not
/// taken from a source.
#[derive(Clone, Copy, Debug)]
struct Bore {
    /// The loss of a round trip at low frequencies (ours).
    loss: f32,
    /// Tone 0 and 100: the low-pass's corner, in harmonics of the note.
    harm_lo: f32,
    harm_hi: f32,
    /// The fewest harmonics the corner sits at, whatever Tone and the
    /// breath ask: lower, the bore loses too much to speak (ours).
    harm_floor: f32,
    /// The mouth pressure for a breath of 0+ and of 1, in units of the
    /// reed's closing point (reeds) or of the jet's full speed (flute).
    press_lo: f32,
    press_hi: f32,
    /// Vibrato's share in the breath (at Vibrato 100) and in the pitch, in
    /// semitones.
    vib_breath: f32,
    vib_semis: f32,
    /// Noise in the mouth pressure (reeds) or the jet (flute), and heard
    /// outside, at Noise 100.
    noise_in: f32,
    noise_out: f32,
    /// How much brighter a full breath is than none, in octaves of the
    /// low-pass's corner (ours: the simple reed table barely brightens as
    /// it is blown harder, while a real reed does).
    breath_tone: f32,
    /// How hard a note from silence is primed, times the share above: the
    /// reeds' lines start there; the flute's jet is deflected by it on its
    /// first pass (ours: speaking at C4 at Attack 15, the clarinet in 15
    /// ms at 1, the sax in 19 ms at 2 against 34 at 1, the flute in 30 ms
    /// at 16 against 46 unprimed).
    prime: f32,
}

const BORES: [Bore; 3] = [
    // Clarinet.
    Bore {
        loss: 0.97,
        harm_lo: 2.5,
        harm_hi: 14.0,
        harm_floor: 3.0,
        press_lo: 1.35,
        press_hi: 2.0,
        vib_breath: 0.10,
        vib_semis: 0.10,
        noise_in: 0.20,
        noise_out: 0.0,
        breath_tone: 2.0,
        prime: 1.0,
    },
    // Sax.
    Bore {
        loss: 0.97,
        harm_lo: 2.5,
        harm_hi: 8.0,
        harm_floor: 2.5,
        press_lo: 1.35,
        press_hi: 2.0,
        vib_breath: 0.08,
        vib_semis: 0.35,
        noise_in: 0.20,
        noise_out: 0.0,
        breath_tone: 2.0,
        prime: 2.0,
    },
    // Flute.
    Bore {
        loss: 0.97,
        harm_lo: 2.2,
        harm_hi: 6.1,
        harm_floor: 2.2,
        press_lo: 0.25,
        press_hi: 1.0,
        vib_breath: 0.25,
        vib_semis: 0.12,
        noise_in: 0.12,
        noise_out: 0.03,
        breath_tone: 0.5,
        prime: 16.0,
    },
];

/// The sax's tip, as a share of the period.
const SAX_TIP: f32 = 0.32;
/// The flute: the jet's crossing as a share of the period, the field's
/// pull on the jet, the reflection at the mouth hole and the jet's gain.
const JET_SHARE: f32 = 0.5;
const JET_PULL: f32 = 0.9;
const MOUTH_REFLECT: f32 = 0.6;
const JET_GAIN: f32 = 1.2;
/// Edge 100's aim, in the jet curve's units.
const EDGE_MAX: f32 = 1.2;
/// The breath the bore's own tone is set for: the corner moves by
/// `breath_tone` octaves a unit of breath from here (velocity 100 at
/// Breath 85).
const BREATH_TONE_AT: f32 = 0.67;
/// A harder reed plays louder (the mouth pressure scales with its closing
/// point): its level is taken out as `(hc / hc at Reed 45)` to this power
/// (measured, the same for clarinet and sax).
const REED_LEVEL: f32 = 1.37;
/// The level a bore is set to at a full breath, before the level law and
/// Level: dBFS RMS of the mid.
pub const TARGET_DB: f32 = -10.0;
/// How much louder each octave down from middle C plays (dB; ours, from
/// the keys' loudness, K-weighted, C2 to C6).
const KEY_TILT_DB: f32 = 0.4;
/// The highest the loss filter's corner goes (Hz).
const CORNER_MAX_HZ: f32 = 14_000.0;
/// The reed's closing point at Reed 0 and 100 (Smith's `hc`).
const REED_SOFT: f32 = 0.3;
const REED_HARD: f32 = 0.65;

/// Rows of the tables below: the clarinet, the sax, and the flute at Edge
/// 0, 50 and 100 (the flute's edge moves its level and pitch).
pub const ROWS: usize = 5;
/// Each row holds Tone 0, 50 and 100.
pub const TONES: usize = 3;
/// A table: by row, by Tone, at breaths 0, 1/8, ... 1.
pub type Table = [[[f32; 9]; TONES]; ROWS];

/// Each bore's level, dBFS RMS of the mid, at its breaths and tones with
/// its other settings at their defaults and the level law flat, taken out
/// before the level law so the law alone says how loud a breath is (Tone
/// then changes the colour, not the level). Measured with
/// `examples/wind_calibrate.rs`, which prints the next table to paste.
#[rustfmt::skip]
pub const NORM_DB: Table = [
    [
        [-31.77, -30.05, -28.99, -28.34, -27.96, -27.73, -27.64, -27.44, -27.46],
        [-31.77, -30.05, -28.86, -28.64, -28.85, -29.25, -29.75, -30.31, -30.92],
        [-31.96, -31.84, -32.01, -32.35, -32.78, -33.29, -33.86, -34.46, -35.10],
    ],
    [
        [-25.32, -24.78, -24.46, -24.24, -24.09, -23.97, -23.86, -24.04, -24.46],
        [-25.32, -24.78, -24.46, -24.13, -24.13, -24.30, -24.59, -25.03, -25.61],
        [-24.80, -24.63, -24.74, -25.05, -25.51, -26.09, -26.75, -27.47, -28.29],
    ],
    [
        [-21.57, -12.21, -9.37, -7.67, -6.45, -5.51, -4.83, -4.34, -3.95],
        [-17.15, -12.54, -10.41, -9.09, -8.16, -7.47, -6.93, -6.51, -6.16],
        [-18.27, -14.62, -12.66, -11.40, -10.50, -9.83, -9.31, -8.89, -8.56],
    ],
    [
        [-21.80, -12.41, -9.42, -7.57, -6.20, -5.10, -4.24, -3.56, -2.99],
        [-17.39, -12.60, -10.29, -8.79, -7.68, -6.80, -6.10, -5.52, -5.03],
        [-18.44, -14.63, -12.50, -11.06, -9.99, -9.14, -8.45, -7.88, -7.41],
    ],
    [
        [-22.89, -13.46, -10.35, -8.32, -6.71, -5.35, -4.18, -3.15, -2.25],
        [-18.46, -13.53, -11.02, -9.22, -7.77, -6.56, -5.53, -4.64, -3.85],
        [-19.48, -15.52, -13.17, -11.42, -10.01, -8.84, -7.83, -6.96, -6.20],
    ],
];
/// Each bore's pitch, cents sharp, at the same breaths and tones: the
/// nonlinear bore plays a little sharp or flat as it gets louder, the same
/// in cents on every key (the losses follow the key). Measured the same
/// way.
#[rustfmt::skip]
pub const SHARP_CENTS: Table = [
    [
        [0.16, 0.30, 0.43, 0.55, 0.62, 0.61, 0.54, 0.60, 0.69],
        [0.16, 0.30, 0.63, 1.13, 1.39, 1.44, 1.35, 1.18, 0.98],
        [0.92, 1.21, 1.31, 1.26, 1.13, 0.96, 0.79, 0.63, 0.50],
    ],
    [
        [2.42, 2.12, 1.75, 1.38, 1.10, 0.96, 0.74, 0.57, 0.50],
        [2.42, 2.12, 1.75, 1.26, 0.97, 0.80, 0.67, 0.56, 0.46],
        [2.17, 1.76, 1.39, 1.08, 0.84, 0.66, 0.53, 0.42, 0.34],
    ],
    [
        [-0.02, 0.47, 1.19, 1.92, 2.58, 3.17, 3.77, 4.32, 4.78],
        [0.17, 1.07, 1.94, 2.63, 3.17, 3.59, 3.89, 4.12, 4.28],
        [0.58, 1.35, 1.88, 2.26, 2.52, 2.70, 2.83, 2.91, 2.96],
    ],
    [
        [-0.05, -0.17, -0.11, -0.08, -0.13, -0.25, -0.31, -0.30, -0.29],
        [-0.01, 0.43, 0.83, 1.06, 1.15, 1.16, 1.13, 1.08, 1.02],
        [0.44, 0.95, 1.20, 1.29, 1.29, 1.24, 1.16, 1.08, 1.00],
    ],
    [
        [-0.10, -0.91, -1.57, -2.30, -3.15, -4.06, -4.82, -5.42, -5.91],
        [-0.24, -0.30, -0.44, -0.79, -1.23, -1.67, -2.07, -2.42, -2.71],
        [0.28, 0.50, 0.41, 0.15, -0.17, -0.48, -0.77, -1.03, -1.24],
    ],
];

/// A table's value for `model` at breath `b`, Tone `tone` and the flute's
/// edge (both 0 to 1).
pub fn from_table(t: &Table, model: usize, edge: f32, tone: f32, b: f32) -> f32 {
    // Between two grid points of three, 0 to 1.
    let between = |x: f32| {
        let x = x.clamp(0.0, 1.0) * 2.0;
        let i = (x as usize).min(1);
        (i, x - i as f32)
    };
    let (ti, tf) = between(tone);
    let at = |row: &[[f32; 9]; TONES]| {
        let lo = table9(&row[ti], b);
        lo + (table9(&row[ti + 1], b) - lo) * tf
    };
    match model {
        0 | 1 => at(&t[model]),
        _ => {
            let (ei, ef) = between(edge);
            let lo = at(&t[2 + ei]);
            lo + (at(&t[3 + ei]) - lo) * ef
        }
    }
}

/// `t` (0 to 1) along a nine-point table.
fn table9(t: &[f32; 9], x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0) * 8.0;
    let i = (x as usize).min(7);
    let f = x - i as f32;
    t[i] + (t[i + 1] - t[i]) * f
}

/// The level for a breath `b` (0 to 1) on the voice core's square law
/// over `range_db` (as [`crate::voice::velocity_gain`] with the breath as
/// velocity / 127), falling to 0 at no breath.
pub fn breath_level(b: f32, range_db: f32) -> f32 {
    let b = b.clamp(0.0, 1.0);
    let v = (b * 127.0).max(1.0);
    let g = if range_db <= 0.0 {
        1.0
    } else {
        let r = 10f32.powf(range_db / 20.0);
        let k = 127.0 / (126.0 * r.sqrt()) - 1.0 / 126.0;
        let m = (1.0 - k) / 127.0;
        let a = m * v + k;
        a * a
    };
    g * (b * 127.0).min(1.0)
}

/// Smith's reed table: how much of the wave the reed sends back for the
/// pressure `h` across it, with its closing point `hc`. Its two corners,
/// where the reed shuts on the lay and where it opens wide, are rounded
/// over `REED_ROUND` either side (ours: a reed curls shut along the lay
/// rather than slapping, and a hard corner feeds harmonics past the
/// output's Nyquist that fold down as aliases: the clarinet at Tone 100,
/// 52 dB under the note at C7, against 83 rounded).
#[inline(always)]
pub fn reed_reflect(h: f32, hc: f32) -> f32 {
    let m = 1.0 / (hc + 1.0);
    let x = 1.0 - m * (hc - h);
    ramp(x) - ramp(x - 1.0)
}

/// How far either side of the reed table's corners they are rounded.
const REED_ROUND: f32 = 0.1;

/// `max(x, 0)` with its corner rounded by a parabola over `±REED_ROUND`.
#[inline(always)]
fn ramp(x: f32) -> f32 {
    if x >= REED_ROUND {
        x
    } else {
        let t = (x + REED_ROUND).max(0.0);
        t * t * (0.25 / REED_ROUND)
    }
}

/// `tanh` by its [3/2] Padé form, exact at 0 and reaching 1 at ±3.
#[inline(always)]
fn tanh_pade(x: f32) -> f32 {
    let x = x.clamp(-3.0, 3.0);
    let x2 = x * x;
    x * (27.0 + x2) / (27.0 + 9.0 * x2)
}

/// The one-pole low-pass's coefficient for a corner at `hz`.
#[inline]
fn lp_coef(hz: f32, fs: f32) -> f32 {
    (-TAU * hz / fs).exp()
}

/// The one-pole low-pass's phase delay, in samples, at `hz`.
#[inline]
fn lp_delay(a: f32, hz: f32, fs: f32) -> f32 {
    let w = TAU * hz / fs;
    (a * w.sin()).atan2(1.0 - a * w.cos()) / w
}

/// The flute's jet against the edge: aimed `aim` off centre (Edge), the
/// curve's value there, and the gain that makes its slope there 1, so
/// Edge moves only the tone (the even harmonics), not how hard the pipe
/// must be blown to speak.
#[derive(Clone, Copy, Debug)]
struct Jet {
    aim: f32,
    at_aim: f32,
    gain: f32,
}

impl Jet {
    fn new(aim: f32) -> Jet {
        let e = 1e-3;
        let slope = (tanh_pade(aim + e) - tanh_pade(aim - e)) / (2.0 * e);
        Jet {
            aim,
            at_aim: tanh_pade(aim),
            gain: 1.0 / slope.max(0.05),
        }
    }
}

/// Where a bore's loss filter sits, in harmonics of the note: Tone `t` (0
/// to 1) between the bore's darkest and brightest, moved by the breath
/// `b` and `tone_oct` octaves (pressure), never under the bore's floor.
/// (Its corner, in Hz, stays under `CORNER_MAX_HZ`: a high note on a bright
/// reed would otherwise feed harmonics past the output's Nyquist back into
/// the bore, and they fold down as aliases.)
fn corner_harmonics(bore: &Bore, t: f32, b: f32, tone_oct: f32) -> f32 {
    let bright = tone_oct + bore.breath_tone * (b - BREATH_TONE_AT);
    (bore.harm_lo * (bore.harm_hi / bore.harm_lo).powf(t) * bright.exp2()).max(bore.harm_floor)
}

// ------------------------------------------------------------ pictures
//
// The Synth tab draws the engine from its values with these, the engine's
// own formulas, so a picture follows a knob the moment it turns.

/// The note the pictures are drawn for: middle C, Hz.
pub const PICTURE_HZ: f32 = 261.63;
/// The inner rate the pictures are drawn at (48 kHz, oversampled twice).
const PICTURE_FS: f32 = 96_000.0;

/// The bore's loss filter's corner (Hz) for middle C at breath `b` (0 to
/// 1), with Tone as `get` has it: where the top of the sound rolls off.
pub fn tone_corner(get: impl Fn(u16) -> f32, b: f32) -> f32 {
    let bore = &BORES[(get(MODEL).round().max(0.0) as usize).min(2)];
    let t = (get(TONE) * 0.01).clamp(0.0, 1.0);
    (PICTURE_HZ * corner_harmonics(bore, t, b.clamp(0.0, 1.0), 0.0)).min(CORNER_MAX_HZ)
}

/// The loss filter's gain at `hz` with its corner at `corner` (Hz).
pub fn tone_gain(corner: f32, hz: f32) -> f32 {
    let a = lp_coef(corner, PICTURE_FS);
    let w = TAU * hz / PICTURE_FS;
    (1.0 - a) / (1.0 - 2.0 * a * w.cos() + a * a).max(1e-12).sqrt()
}

/// The mouthpiece, `n` points across, as `get` sets it, and where a full
/// breath (at Breath) puts it, 0 to 1 across.
///
/// - A reed: the air it lets through (0 to 1 high) against the pressure
///   across it, from none to past where the hardest reed shuts. The air
///   rises with the pressure, then falls to nothing as the reed closes on
///   the lay; the breath sits on the falling side, where the reed feeds
///   the bore and it speaks.
/// - The flute: the jet's push on the pipe (-1 to 1) against how far the
///   pipe's wave deflects it; Edge sets the curve off centre (the breath
///   sits at no deflection, the middle).
pub fn mouthpiece_curve(get: impl Fn(u16) -> f32, n: usize) -> (Vec<f32>, f32) {
    let model = (get(MODEL).round().max(0.0) as usize).min(2);
    let n = n.max(2);
    let across = |i: usize| i as f32 / (n - 1) as f32;
    let fill = |y: Vec<f32>| {
        let top = y.iter().fold(1e-9f32, |m, v| m.max(v.abs()));
        y.into_iter().map(|v| v / top).collect::<Vec<f32>>()
    };
    if model == 2 {
        let jet = Jet::new(EDGE_MAX * (get(EDGE) * 0.01).clamp(0.0, 1.0));
        let y = (0..n)
            .map(|i| -(tanh_pade(6.0 * across(i) - 3.0 + jet.aim) - jet.at_aim) * jet.gain)
            .collect();
        (fill(y), 0.5)
    } else {
        let bore = &BORES[model];
        let hc = REED_SOFT + (REED_HARD - REED_SOFT) * (get(REED) * 0.01).clamp(0.0, 1.0);
        let b = (get(BREATH) * 0.01).clamp(0.0, 1.0);
        let pm = (bore.press_lo + (bore.press_hi - bore.press_lo) * b) * hc;
        let span = 1.25 * REED_HARD;
        let y = (0..n)
            .map(|i| {
                let h = span * across(i);
                h * (1.0 - reed_reflect(h, hc))
            })
            .collect();
        (fill(y), (0.5 * pm / span).clamp(0.0, 1.0))
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

/// One voice: its note, breath and bore.
#[derive(Clone, Copy, Debug)]
struct Voice {
    note: u8,
    velocity: u8,
    /// The pitch, gliding (notes), before bend, vibrato and pressure.
    pitch: f32,
    env: Adsr,
    press: Pressure,
    lfo: Lfo,
    /// The tongued start's puff now, 0 to 1.
    tongue: f32,
    /// The bore is to be primed at the next step (a note from silence).
    kick: bool,
    growl: f32,
    /// The breath now, 0 to 1 (for the meter and the level).
    breath: f32,

    // Ramped across a step: the delays (samples), mouth pressure, gain.
    d1: f32,
    dd1: f32,
    d2: f32,
    dd2: f32,
    pm: f32,
    dpm: f32,
    gain: f32,
    dgain: f32,
    /// The stolen voice's fade, 1 to 0, and its step.
    fade: f32,
    dfade: f32,

    // Set each step.
    lp_a: f32,
    hc: f32,
    noise_in: f32,
    noise_out: f32,
    pan_l: f32,
    pan_r: f32,

    // States.
    lp: f32,
    nz_lo: f32,
    nz_hi: f32,
    /// The voice's DC blocker (before its gain, so a sudden change in the
    /// breath leaves no thump behind).
    dc_x: f32,
    dc_y: f32,
    /// Where both lines write next.
    w: usize,
    /// The loudest sample since the last step, and silent steps.
    peak: f32,
    quiet: u32,
    rng: Rng,
}

impl Voice {
    fn new(seed: u64) -> Voice {
        Voice {
            note: 60,
            velocity: 100,
            pitch: 60.0,
            env: Adsr::default(),
            press: Pressure::new(),
            lfo: Lfo::new(seed),
            tongue: 0.0,
            kick: false,
            growl: 0.0,
            breath: 0.0,
            d1: 100.0,
            dd1: 0.0,
            d2: 50.0,
            dd2: 0.0,
            pm: 0.0,
            dpm: 0.0,
            gain: 0.0,
            dgain: 0.0,
            fade: 1.0,
            dfade: 0.0,
            lp_a: 0.5,
            hc: 0.45,
            noise_in: 0.0,
            noise_out: 0.0,
            pan_l: std::f32::consts::FRAC_1_SQRT_2,
            pan_r: std::f32::consts::FRAC_1_SQRT_2,
            lp: 0.0,
            nz_lo: 0.0,
            nz_hi: 0.0,
            dc_x: 0.0,
            dc_y: 0.0,
            w: 0,
            peak: 0.0,
            quiet: 0,
            rng: Rng::new(seed ^ 0x5EED),
        }
    }
}

/// The wind engine. See the module docs.
pub struct Wind {
    rate: u32,
    /// Oversampling: 2 below 64 kHz, else 1.
    os: usize,
    values: [f32; ID_SPACE],
    tuning: Tuning,

    slots: Voices<SLOTS>,
    v: [Voice; SLOTS],
    /// Two delay lines a slot, `len` each (a power of two).
    lines: Vec<f32>,
    len: usize,

    /// Held keys, for one voice.
    stack: NoteStack,
    /// Whether a note has sounded since the last reset.
    played: bool,
    bend: f32,
    chan: Channel,
    /// Which of breath, the mod wheel and the foot controller have moved
    /// since the last reset: until the one Breath from names has, notes
    /// play from their velocity, so a sound set to a controller is never
    /// silent without one.
    heard: [bool; 3],
    pressure: [u8; 128],
    press_c: PressureCoefs,
    chord: PressureChord,
    glide: Glide,
    /// The newest voice, for the meter.
    newest: usize,
    /// With one voice: the slot that holds the instrument.
    lead: usize,

    hb: [[Ap; 3]; 4],
    /// The voices' DC blockers' pole, at the inner rate.
    dc_r: f32,
    /// Whether nothing runs until the next note.
    idle: bool,
}

impl Wind {
    pub fn new(rate: u32) -> Wind {
        let mut values = [0.0; ID_SPACE];
        for p in PARAMS {
            values[p.id as usize] = p.default;
        }
        let mut v = [Voice::new(0); SLOTS];
        for (i, s) in v.iter_mut().enumerate() {
            *s = Voice::new(0x3D15 + i as u64 * 977);
        }
        let mut w = Wind {
            rate: rate.max(1),
            os: 2,
            values,
            tuning: Tuning::standard(),
            slots: Voices::new(),
            v,
            lines: Vec::new(),
            len: 0,
            stack: NoteStack::new(),
            played: false,
            bend: 0.0,
            chan: Channel::default(),
            heard: [false; 3],
            pressure: [0; 128],
            press_c: PressureCoefs::default(),
            chord: PressureChord::default(),
            glide: Glide::default(),
            newest: 0,
            lead: 0,
            hb: [[Ap::default(); 3]; 4],
            dc_r: 0.999,
            idle: true,
        };
        w.prepare(rate, 0);
        w
    }

    pub fn presets() -> &'static [Preset] {
        crate::factory::presets!("wind.presets", &INFO, PARAMS)
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

    /// Each sounding voice's pressure, 0 to 1, into `out` (as many as
    /// fit); how many it wrote.
    pub fn pressures(&self, out: &mut [f32]) -> usize {
        let mut n = 0;
        for (slot, v) in self.slots.slots.iter().zip(&self.v) {
            if n == out.len() {
                break;
            }
            if slot.sounding() {
                out[n] = v.press.value();
                n += 1;
            }
        }
        n
    }

    /// Voices sounding now.
    pub fn active_voices(&self) -> usize {
        self.slots.sounding()
    }

    /// The pitch voice `i` is set to play now, in Hz (for tests).
    pub fn voice_hz(&self, i: usize) -> f32 {
        let v = &self.v[i.min(SLOTS - 1)];
        self.tuning.hz(self.pitch_of(v, 0.0)).max(LOWEST_HZ)
    }

    fn val(&self, id: u16) -> f32 {
        self.values[id as usize]
    }

    fn fs(&self) -> f32 {
        (self.rate as usize * self.os) as f32
    }

    fn steps_per_sec(&self) -> f32 {
        self.rate as f32 / CTRL as f32
    }

    fn model(&self) -> usize {
        (self.val(MODEL).round() as usize).min(2)
    }

    fn voices_limit(&self) -> usize {
        (self.val(VOICES).round() as usize).clamp(1, MAX_VOICES)
    }

    fn mono(&self) -> bool {
        self.voices_limit() == 1
    }

    fn update_times(&mut self) {
        let sps = self.steps_per_sec();
        let (a, d, s, r) = (
            self.val(ATTACK),
            self.val(DECAY),
            self.val(SUSTAIN) / 100.0,
            self.val(RELEASE),
        );
        let rate = self.val(VIB_RATE);
        let delay = self.val(VIB_DELAY);
        for v in &mut self.v {
            v.env.set(a, d, s, r, sps);
            v.lfo.set_free(LfoShape::Sine, rate, sps);
            v.lfo.set_delay(delay, delay, sps);
        }
        let ms = self.val(GLIDE).max(MIN_MOVE_MS);
        self.glide = if self.val(GLIDE_MODE) >= 0.5 && self.val(GLIDE) > 0.0 {
            crate::voice::Glide::rate(12_000.0 / self.val(GLIDE), sps)
        } else {
            Glide::time(ms, sps)
        };
    }

    fn update_pressure(&mut self) {
        let shape = PressureShape::from_params(PRESS_BASE, |id| self.values[id as usize]);
        self.press_c = shape.coefs(self.steps_per_sec());
    }

    /// The target pitch of a note, before glide.
    fn note_target(&self, note: u8) -> f32 {
        f32::from(note) + self.val(TRANSPOSE) + self.val(FINE) * 0.01
    }

    /// A voice's pitch now with `extra` semitones (vibrato, pressure).
    fn pitch_of(&self, v: &Voice, extra: f32) -> f32 {
        v.pitch + self.bend * self.val(BEND_RANGE) + extra
    }

    /// Clears a slot's two lines (a voice starting from silence).
    fn clear_lines(&mut self, slot: usize) {
        let n = self.len;
        let at = slot * 2 * n;
        if let Some(s) = self.lines.get_mut(at..at + 2 * n) {
            s.fill(0.0);
        }
    }

    /// Starts slot `i` on `note` from silence.
    fn start_voice(&mut self, i: usize, note: u8, velocity: u8, glide_from: Option<f32>) {
        self.clear_lines(i);
        let target = self.note_target(note);
        let v = &mut self.v[i];
        v.note = note;
        v.velocity = velocity;
        v.pitch = glide_from.unwrap_or(target);
        v.env.clear();
        v.env.gate_on();
        v.press.clear();
        v.press.start();
        v.press.set(self.pressure[usize::from(note & 0x7F)]);
        v.lfo.key();
        v.tongue = 1.0;
        v.kick = true;
        v.lp = 0.0;
        v.nz_lo = 0.0;
        v.nz_hi = 0.0;
        v.dc_x = 0.0;
        v.dc_y = 0.0;
        v.pm = 0.0;
        v.dpm = 0.0;
        v.gain = 0.0;
        v.dgain = 0.0;
        v.fade = 1.0;
        v.dfade = 0.0;
        v.peak = 0.0;
        v.quiet = 0;
        v.breath = 0.0;
        self.newest = i;
        self.snap(i);
    }

    /// Sets slot `i`'s delays for its pitch now, at once.
    fn snap(&mut self, i: usize) {
        let (d1, d2, a) = self.delays(&self.v[i], 0.0, self.v[i].breath);
        let v = &mut self.v[i];
        v.d1 = d1;
        v.d2 = d2;
        v.dd1 = 0.0;
        v.dd2 = 0.0;
        v.lp_a = a;
    }

    /// The bore's delays (samples) and its low-pass coefficient for a
    /// voice with `extra` semitones at breath `b` and `tone_oct` octaves of
    /// extra brightness.
    fn delays(&self, v: &Voice, extra: f32, b: f32) -> (f32, f32, f32) {
        self.delays_toned(v, extra, b, 0.0)
    }

    fn delays_toned(&self, v: &Voice, extra: f32, b: f32, tone_oct: f32) -> (f32, f32, f32) {
        let fs = self.fs();
        let model = self.model();
        let bore = &BORES[model];
        let edge = self.val(EDGE) * 0.01;
        let t = self.val(TONE) * 0.01;
        let sharp = from_table(&SHARP_CENTS, model, edge, t, b);
        let pitch = self.pitch_of(v, extra) - sharp * 0.01;
        let max_period = (self.len.saturating_sub(8)) as f32;
        let hz = self
            .tuning
            .hz(pitch)
            .max(LOWEST_HZ)
            .max(fs / max_period.max(16.0));
        let corner = (hz * corner_harmonics(bore, t, b, tone_oct))
            .clamp(100.0, (0.45 * fs).min(CORNER_MAX_HZ));
        let a = lp_coef(corner, fs);
        let pd = lp_delay(a, hz, fs);
        let period = fs / hz;
        let (d1, d2) = match model {
            0 => (0.5 * period - pd, 0.0),
            1 => ((1.0 - SAX_TIP) * period - pd, SAX_TIP * period),
            _ => (period - pd, JET_SHARE * period),
        };
        (d1.max(2.0), d2.max(2.0), a)
    }

    /// A key goes down.
    fn note_on(&mut self, note: u8, velocity: u8) {
        if velocity == 0 {
            self.note_off(note);
            return;
        }
        self.idle = false;
        if self.mono() {
            self.mono_on(note, velocity);
        } else {
            let st = self.slots.start(note, self.voices_limit());
            if let Some(s) = st.stolen {
                self.steal(s);
            }
            self.start_voice(st.voice, note, velocity, None);
            self.set_pans();
        }
        self.played = true;
    }

    fn steal(&mut self, s: usize) {
        let n = (FADE_MS * 0.001 * self.fs()).max(1.0);
        let v = &mut self.v[s];
        v.dfade = -v.fade / n;
    }

    fn set_pans(&mut self) {
        // One voice sits in the middle; more alternate left and right.
        let spread = if self.mono() {
            0.0
        } else {
            self.val(SPREAD) * 0.01
        };
        for (i, v) in self.v.iter_mut().enumerate() {
            let side = if i % 2 == 0 { -1.0 } else { 1.0 };
            let pan = side * spread;
            let angle = (pan + 1.0) * 0.25 * PI;
            v.pan_l = angle.cos();
            v.pan_r = angle.sin();
        }
    }

    /// One voice: the lead slot holds the instrument; keys over held keys
    /// slur or tongue.
    fn mono_on(&mut self, note: u8, velocity: u8) {
        let held_before = !self.stack.is_empty();
        self.stack.push(note);
        let pick = self
            .stack
            .pick(Priority::from_index(self.val(PRIORITY)))
            .unwrap_or(note);
        let l = self.lead;
        let sounding = self.slots.slots[l].sounding();
        if held_before && sounding {
            if pick != self.v[l].note {
                self.move_to(pick, velocity);
            }
            return;
        }
        if sounding {
            // The instrument is still ringing (released): tongue the new
            // note on the same bore, from where its breath is.
            let v = &mut self.v[l];
            v.note = pick;
            v.velocity = velocity;
            v.env.gate_on();
            v.press.start();
            v.press.set(self.pressure[usize::from(pick & 0x7F)]);
            v.lfo.key();
            v.tongue = 1.0;
            self.slots.slots[l].phase = Phase::On;
            self.slots.slots[l].note = pick;
            return;
        }
        let from = self.v[l].pitch;
        let st = self.slots.start(pick, 1);
        if let Some(s) = st.stolen {
            self.steal(s);
        }
        self.lead = st.voice;
        let glide_from = (self.played && self.val(GLIDE) > 0.0).then_some(from);
        self.start_voice(st.voice, pick, velocity, glide_from);
        self.set_pans();
    }

    /// A slur or tongued move to another held key.
    fn move_to(&mut self, note: u8, velocity: u8) {
        let tongued = self.val(LEGATO) >= 0.5;
        let raw = self.pressure[usize::from(note & 0x7F)];
        let l = self.lead;
        let v = &mut self.v[l];
        v.note = note;
        v.press.set(raw);
        if tongued {
            v.velocity = velocity;
            v.env.gate_on();
            v.tongue = 1.0;
        }
        self.slots.slots[l].note = note;
    }

    /// A key comes up.
    fn note_off(&mut self, note: u8) {
        if self.mono() {
            if !self.stack.remove(note) {
                return;
            }
            let l = self.lead;
            match self.stack.pick(Priority::from_index(self.val(PRIORITY))) {
                Some(next) if next != self.v[l].note => {
                    let vel = self.v[l].velocity;
                    self.move_to(next, vel);
                }
                Some(_) => {}
                None => {
                    if self.slots.slots[l].phase == Phase::On {
                        self.slots.slots[l].phase = Phase::Release;
                        self.v[l].env.gate_off();
                        self.v[l].press.release();
                    }
                }
            }
            return;
        }
        for i in 0..SLOTS {
            let s = self.slots.slots[i];
            if s.phase == Phase::On && s.note == note {
                self.slots.slots[i].phase = Phase::Release;
                self.v[i].env.gate_off();
                self.v[i].press.release();
            }
        }
    }

    fn all_off(&mut self) {
        self.stack.clear();
        self.slots.release_all();
        for v in &mut self.v {
            v.env.gate_off();
            v.press.release();
        }
    }

    fn apply(&mut self, ev: &NodeEvent) {
        match *ev {
            NodeEvent::NoteOn { note, velocity } => self.note_on(note & 0x7F, velocity),
            NodeEvent::NoteOff { note } => self.note_off(note & 0x7F),
            NodeEvent::Pressure { note, value } => {
                let n = note & 0x7F;
                self.pressure[usize::from(n)] = value.min(127);
                for i in 0..SLOTS {
                    let s = self.slots.slots[i];
                    if s.phase != Phase::Free && s.note == n {
                        self.v[i].press.set(value.min(127));
                    }
                }
            }
            NodeEvent::Bend { value } => {
                if value.is_finite() {
                    self.bend = value.clamp(-1.0, 1.0);
                }
            }
            NodeEvent::Control { ctl, value } => {
                self.chan.set(ctl, value);
                if let Some(i) = [Ctl::Breath, Ctl::ModWheel, Ctl::Foot]
                    .iter()
                    .position(|&c| c == ctl)
                {
                    self.heard[i] |= value.is_finite();
                }
            }
            NodeEvent::AllOff => self.all_off(),
            NodeEvent::Param { id, value } => self.set_param(id, value),
            NodeEvent::Expression { .. } => {}
        }
    }

    /// The breath source for a voice: its velocity, or the controller once
    /// it has moved.
    fn source(&self, v: &Voice) -> f32 {
        let s = match self.val(BREATH_FROM).round() as usize {
            1 if self.heard[0] => self.chan.breath,
            2 if self.heard[1] => self.chan.mod_wheel,
            3 if self.heard[2] => self.chan.foot,
            _ => f32::from(v.velocity) / 127.0,
        };
        s * self.val(BREATH) * 0.01
    }

    /// One control step for every voice.
    fn control(&mut self) {
        if self.idle {
            return;
        }
        let sps = self.steps_per_sec();
        let n = (CTRL as usize * self.os) as f32;
        let model = self.model();
        let bore = BORES[model];
        let c = self.press_c;
        // Highest mode: one value for every voice.
        let chord = if c.mode == PressureMode::Highest {
            let it = (0..SLOTS).filter_map(|i| {
                let s = self.slots.slots[i];
                s.sounding()
                    .then(|| (s.phase == Phase::On, self.v[i].press.aim(&c)))
            });
            Some(self.chord.step(&c, it))
        } else {
            None
        };
        let wheel_vib = if self.val(BREATH_FROM).round() as i32 == 2 {
            0.0
        } else {
            self.chan.mod_wheel
        };
        let tongue_k = (-1.0 / (TONGUE_MS * 0.001 * sps)).exp();
        let growl_inc = GROWL_HZ / sps;
        let reed = REED_SOFT + (REED_HARD - REED_SOFT) * self.val(REED) * 0.01;
        let range = self.val(VELOCITY);
        let edge = self.val(EDGE) * 0.01;
        let tone = self.val(TONE) * 0.01;
        let reed_db = if model == 2 {
            0.0
        } else {
            let hc_def = REED_SOFT + (REED_HARD - REED_SOFT) * 0.45;
            -20.0 * REED_LEVEL * (reed / hc_def).log10()
        };
        // With more voices each plays quieter, so a full chord stays under
        // full scale: 1/sqrt(voices) keeps a chord of uncorrelated voices
        // about as loud as one.
        let per_voice = if self.mono() {
            1.0
        } else {
            (self.voices_limit() as f32).sqrt().recip()
        };
        let level = 10f32.powf((self.val(LEVEL) + TARGET_DB + reed_db) / 20.0) * per_voice;
        let mut any = false;
        for i in 0..SLOTS {
            let phase = self.slots.slots[i].phase;
            if phase == Phase::Free {
                continue;
            }
            any = true;
            let mut v = self.v[i];
            // Glide (one voice) toward the note.
            let target = self.note_target(v.note);
            if self.mono() {
                let mut p = v.pitch;
                self.glide.step(&mut p, target);
                v.pitch = p;
            } else {
                v.pitch = target;
            }
            let env = v.env.next();
            let own = v.press.step(&c);
            let press = match c.mode {
                PressureMode::Off => 0.0,
                PressureMode::Highest => chord.unwrap_or(own),
                PressureMode::PerNote => own,
            };
            // Pressure's slots.
            let mut to = [0.0f32; 7];
            for s in 0..4 {
                let t = self.val(press_target(s)).round() as usize;
                if t > 0 && t < to.len() {
                    to[t] += press * self.val(press_depth(s)) * 0.01;
                }
            }
            let lfo = v.lfo.next();
            let depth = (self.val(VIB_DEPTH) * 0.01 + wheel_vib).max(0.0);
            let vib_semis = depth * bore.vib_semis + to[target::VIBRATO as usize];
            let vib_breath = (depth * bore.vib_breath
                + to[target::VIBRATO as usize] * bore.vib_breath / bore.vib_semis.max(0.05))
            .min(0.5);
            v.tongue *= tongue_k;
            v.growl += growl_inc;
            v.growl -= v.growl.floor();

            // The breath.
            let src = self.source(&v);
            let b = (env * src + to[target::BREATH as usize]).clamp(0.0, 1.0);
            // The tongue's puff: an accent on the breath, so no breath
            // stays silence.
            let puff = 1.0 + TONGUE_BREATH * self.val(TONGUE) * 0.01 * v.tongue;
            let growl = (self.val(GROWL) * 0.01 + to[target::GROWL as usize]).clamp(0.0, 1.0);
            let wobble = 1.0
                + vib_breath * lfo
                + GROWL_DEPTH * growl * (TAU * v.growl).sin() * (0.75 + 0.25 * v.rng.bipolar());
            // What the bore is blown with: the breath, the puff, the
            // vibrato and growl in it. It sets the mouth pressure, the tone,
            // the pitch correction and the level alike.
            let bb = (b * puff * wobble).clamp(0.0, 1.0);
            v.breath = b;

            // Pitch and the bore.
            let extra = vib_semis * lfo + 12.0 * to[target::PITCH as usize];
            let tone_oct = 2.0 * to[target::TONE as usize];
            let (d1, d2, a) = self.delays_toned(&v, extra, bb, tone_oct);
            v.dd1 = (d1 - v.d1) / n;
            v.dd2 = (d2 - v.d2) / n;
            v.lp_a = a;
            v.hc = reed;

            // The mouth pressure: from just over where the bore starts to
            // speak (no breath) up. The bore keeps speaking as the breath
            // falls to nothing and the level law takes it down to silence,
            // so a fading breath never cuts off where the reed would stop.
            let pm = (bore.press_lo + (bore.press_hi - bore.press_lo) * bb)
                * if model == 2 { 1.0 } else { reed };
            v.dpm = (pm - v.pm) / n;
            // A bore from silence is primed, so the note speaks in a few
            // periods instead of building up from nothing (60 ms at C4 for
            // the reeds, four times that at C2): a reed bore's lines start
            // at a share of the mouth pressure (Tongue's crisp release, or
            // a little of it with Tongue at 0), a square wave at the note's
            // period that the reed then shapes.
            if v.kick {
                v.kick = false;
                let share = bore.prime * (PRIME + PRIME_TONGUE * self.val(TONGUE) * 0.01);
                let at = i * 2 * self.len;
                if model != 2 {
                    self.lines[at..at + 2 * self.len].fill(-share * pm);
                } else {
                    // The flute: the jet's first pass strikes the edge hard
                    // (a "tu"), the pipe locking in a few periods.
                    self.lines[at + self.len..at + 2 * self.len].fill(-share * pm);
                }
            }
            let noise = (self.val(NOISE) * 0.01 + to[target::NOISE as usize]).clamp(0.0, 1.5)
                + 2.0 * self.val(TONGUE) * 0.01 * v.tongue;
            v.noise_in = bore.noise_in * noise;
            v.noise_out = bore.noise_out * noise * bb;

            // The level: the law for the breath, the bore's own level out.
            // Each key a touch louder below middle C and softer above, so
            // the keys sound as loud as each other: a bore is as loud at
            // C2 as at C6 on a meter, but the ear (and a loudness meter's
            // weighting) hears the low notes quieter.
            let norm = 10f32.powf(
                (KEY_TILT_DB * (60.0 - v.pitch) / 12.0
                    - from_table(&NORM_DB, model, edge, tone, bb))
                    / 20.0,
            );
            let g = breath_level(bb, range) * norm * level;
            // A gain ramped to nothing lands a hair off it: settle it at
            // exactly 0 (under -140 dB, inaudible), so no breath is exact
            // silence.
            if g == 0.0 && v.gain.abs() < 1e-7 {
                v.gain = 0.0;
            }
            v.dgain = (g - v.gain) / n;

            // Silence: the envelope done and the bore quiet.
            if v.peak < QUIET_LEVEL && (v.env.idle() || phase == Phase::Fade) {
                v.quiet += 1;
            } else {
                v.quiet = 0;
            }
            v.peak = 0.0;
            for x in [&mut v.lp, &mut v.nz_lo, &mut v.nz_hi, &mut v.dc_y] {
                if x.abs() < 1e-20 {
                    *x = 0.0;
                }
            }
            let done =
                v.quiet as f32 >= QUIET_MS * 0.001 * sps || (phase == Phase::Fade && v.fade <= 0.0);
            self.v[i] = v;
            if done {
                self.free(i);
            }
        }
        // The half-band's tails flushed, as the voices' are.
        for ap in self.hb.iter_mut().flatten() {
            if ap.x1.abs() < 1e-20 {
                ap.x1 = 0.0;
            }
            if ap.y1.abs() < 1e-20 {
                ap.y1 = 0.0;
            }
        }
        if !any || self.slots.busy() == 0 {
            self.silence();
        }
    }

    /// Slot `i` is silent: its lines are cleared and it is free.
    fn free(&mut self, i: usize) {
        self.slots.free(i);
        self.clear_lines(i);
        let v = &mut self.v[i];
        v.env.clear();
        v.gain = 0.0;
        v.dgain = 0.0;
        v.pm = 0.0;
        v.dpm = 0.0;
        v.lp = 0.0;
        v.breath = 0.0;
        v.fade = 1.0;
        v.dfade = 0.0;
    }

    /// The voice the host may take back when it runs short of time: the
    /// voice core's victim, but never the one voice while its key is held
    /// (that would silence the instrument mid-note).
    fn cull_pick(&self) -> Option<usize> {
        let i = self.slots.victim()?;
        (!(self.mono() && i == self.lead && self.slots.slots[i].phase == Phase::On)).then_some(i)
    }

    /// Nothing sounds: everything stops until the next note.
    fn silence(&mut self) {
        self.idle = true;
        self.hb = [[Ap::default(); 3]; 4];
    }

    /// One sample of slot `i` at the inner rate.
    #[inline(always)]
    fn sample(&mut self, i: usize, model: usize, loss: f32, jet: Jet) -> f32 {
        let len = self.len;
        let mask = len - 1;
        let base = i * 2 * len;
        let dc_r = self.dc_r;
        let v = &mut self.v[i];
        let (bore_line, rest) = self.lines[base..base + 2 * len].split_at_mut(len);
        let jet_line = rest;
        let w = v.w;
        let x = lagrange(bore_line, w, mask, v.d1);
        v.d1 += v.dd1;
        v.lp = (1.0 - v.lp_a) * x + v.lp_a * v.lp;
        let pm = v.pm;
        v.pm += v.dpm;
        // Breath noise, band-passed (a high-pass at ~500 Hz, then the
        // low-pass at ~4 kHz, at 96 kHz).
        let white = v.rng.bipolar();
        v.nz_hi += 0.032 * (white - v.nz_hi);
        let hp = white - v.nz_hi;
        v.nz_lo += 0.23 * (hp - v.nz_lo);
        let nz = v.nz_lo;
        let y = match model {
            0 => {
                let pb = -loss * v.lp;
                let pms = pm * (1.0 + v.noise_in * nz);
                let h = 0.5 * pms - pb;
                let out = 0.5 * pms - reed_reflect(h, v.hc) * h;
                bore_line[w] = out;
                x + pb
            }
            1 => {
                let xa = lagrange(jet_line, w, mask, v.d2);
                let a_in = -xa;
                let b_in = -loss * v.lp;
                let pms = pm * (1.0 + v.noise_in * nz);
                let pplus = 0.5 * (a_in + b_in);
                let h = 0.5 * pms - pplus;
                let p = pplus + 0.5 * pms - reed_reflect(h, v.hc) * h;
                jet_line[w] = p - a_in;
                bore_line[w] = p - b_in;
                x + b_in
            }
            _ => {
                let xr = -loss * v.lp;
                let e = pm.max(0.0);
                // The jet carries the breath across the mouth hole: what
                // reaches the edge is what left the lips half a period ago,
                // deflected by the field then, less the breath now (so a
                // change in the breath kicks the pipe into speaking).
                let jd = lagrange(jet_line, w, mask, v.d2) - e;
                let drive = -(tanh_pade(jd + jet.aim) - jet.at_aim) * jet.gain;
                bore_line[w] = -MOUTH_REFLECT * xr + JET_GAIN * e.sqrt() * drive;
                jet_line[w] = e - JET_PULL * xr + v.noise_in * nz * e;
                x + xr + v.noise_out * nz
            }
        };
        v.d2 += v.dd2;
        v.w = (w + 1) & mask;
        let hp = y - v.dc_x + dc_r * v.dc_y;
        v.dc_x = y;
        v.dc_y = hp;
        let g = v.gain * v.fade;
        v.gain += v.dgain;
        if v.dfade != 0.0 {
            v.fade = (v.fade + v.dfade).max(0.0);
        }
        let out = hp * g;
        v.peak = v.peak.max(out.abs());
        out
    }

    fn render(&mut self, from: usize, to: usize, left: &mut [f32], right: &mut [f32]) {
        if self.idle {
            return;
        }
        let model = self.model();
        let loss = BORES[model].loss;
        let jet = Jet::new(EDGE_MAX * self.val(EDGE) * 0.01);
        let active: [bool; SLOTS] =
            std::array::from_fn(|i| self.slots.slots[i].phase != Phase::Free);
        for f in from..to {
            let mut l = [0.0f32; 2];
            let mut r = [0.0f32; 2];
            for k in 0..self.os {
                for i in 0..SLOTS {
                    if active[i] {
                        let y = self.sample(i, model, loss, jet);
                        l[k] += y * self.v[i].pan_l;
                        r[k] += y * self.v[i].pan_r;
                    }
                }
            }
            let (yl, yr) = if self.os == 2 {
                (self.half(0, l), self.half(2, r))
            } else {
                (l[0], r[0])
            };
            left[f] += yl;
            right[f] += yr;
        }
    }

    /// The half-band on a pair of inner samples, path `p` (0 left, 2 right).
    #[inline(always)]
    fn half(&mut self, p: usize, s: [f32; 2]) -> f32 {
        let mut e = s[1];
        let mut o = s[0];
        for (st, c) in self.hb[p].iter_mut().zip(HB_EVEN) {
            e = st.run(c, e);
        }
        for (st, c) in self.hb[p + 1].iter_mut().zip(HB_ODD) {
            o = st.run(c, o);
        }
        0.5 * (e + o)
    }
}

/// A delay line read `d` samples back (at least 2; the last sample
/// written is 1 back) with cubic Lagrange interpolation.
#[inline(always)]
fn lagrange(line: &[f32], w: usize, mask: usize, d: f32) -> f32 {
    let k = d - 1.0;
    let j = k as usize;
    let t = k - j as f32;
    let at = |back: usize| line[w.wrapping_sub(1 + back) & mask];
    let (xm1, x0, x1, x2) = (at(j - 1), at(j), at(j + 1), at(j + 2));
    let c_m1 = -t * (t - 1.0) * (t - 2.0) * (1.0 / 6.0);
    let c0 = (t + 1.0) * (t - 1.0) * (t - 2.0) * 0.5;
    let c1 = -(t + 1.0) * t * (t - 2.0) * 0.5;
    let c2 = (t + 1.0) * t * (t - 1.0) * (1.0 / 6.0);
    c_m1 * xm1 + c0 * x0 + c1 * x1 + c2 * x2
}

impl Node for Wind {
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
        let old = self.values[id as usize];
        self.values[id as usize] = p.clamp(value);
        match id {
            ATTACK | DECAY | SUSTAIN | RELEASE | GLIDE | GLIDE_MODE | VIB_RATE | VIB_DELAY => {
                self.update_times()
            }
            SPREAD => self.set_pans(),
            VOICES if old != self.values[id as usize] => {
                self.all_off();
                self.set_pans();
            }
            MODEL if old != self.values[id as usize] => {
                // A new bore: what sounds fades out at once, since the old
                // bore's waves mean nothing in the new one.
                self.all_off();
                for i in 0..SLOTS {
                    if self.slots.slots[i].phase != Phase::Free {
                        self.slots.slots[i].phase = Phase::Fade;
                        self.steal(i);
                    }
                }
            }
            _ if id >= PRESS_BASE => self.update_pressure(),
            _ => {}
        }
    }

    fn param(&self, id: u16) -> f32 {
        if id == METER_BREATH {
            let s = self.slots.slots[self.newest];
            return if s.phase == Phase::Free {
                0.0
            } else {
                self.v[self.newest].breath * 100.0
            };
        }
        match PARAMS.iter().find(|p| p.id == id) {
            Some(_) => self.values[id as usize],
            None => f32::NAN,
        }
    }

    fn prepare(&mut self, rate: u32, _max_block: usize) {
        self.rate = rate.max(1);
        self.os = if self.rate < 64_000 { 2 } else { 1 };
        let need = (self.fs() / LOWEST_HZ) as usize + 16;
        let len = need.next_power_of_two();
        if len != self.len {
            self.len = len;
            self.lines = vec![0.0; SLOTS * 2 * len];
        }
        self.dc_r = 1.0 - TAU * 5.0 / self.fs();
        self.update_times();
        self.update_pressure();
        self.set_pans();
        self.reset();
    }

    fn reset(&mut self) {
        self.stack.clear();
        self.slots.clear();
        self.played = false;
        self.bend = 0.0;
        self.chan = Channel::default();
        self.heard = [false; 3];
        self.pressure = [0; 128];
        self.chord.clear();
        self.lines.fill(0.0);
        for v in &mut self.v {
            v.env.clear();
            v.press.clear();
            v.gain = 0.0;
            v.dgain = 0.0;
            v.pm = 0.0;
            v.dpm = 0.0;
            v.lp = 0.0;
            v.nz_lo = 0.0;
            v.nz_hi = 0.0;
            v.dc_x = 0.0;
            v.dc_y = 0.0;
            v.breath = 0.0;
            v.tongue = 0.0;
            v.fade = 1.0;
            v.dfade = 0.0;
            v.w = 0;
        }
        self.silence();
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
        ((self.val(RELEASE) * 1.4 + QUIET_MS + 200.0) * 0.001 * self.rate as f32) as u32
    }

    fn set_tuning(&mut self, tuning: &Tuning) {
        if self.tuning == *tuning {
            return;
        }
        self.tuning.clone_from(tuning);
    }

    fn sounding_voices(&self) -> usize {
        self.slots.busy()
    }

    fn cull_candidate(&self) -> Option<mpfree_node::Cull> {
        let i = self.cull_pick()?;
        let s = self.slots.slots[i];
        let v = &self.v[i];
        Some(mpfree_node::Cull {
            level: v.gain * v.env.level,
            releasing: s.phase == Phase::Release,
        })
    }

    fn cull(&mut self) -> bool {
        let Some(i) = self.cull_pick() else {
            return false;
        };
        self.slots.slots[i].phase = Phase::Fade;
        self.slots.stolen += 1;
        self.steal(i);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reed_closes_and_opens() {
        assert_eq!(reed_reflect(1.0, 0.4), 1.0);
        assert_eq!(reed_reflect(-1.2, 0.4), 0.0);
        assert!(reed_reflect(0.0, 0.4) > 0.0 && reed_reflect(0.0, 0.4) < 1.0);
        // Rounded, smooth and rising through both corners.
        let mut last = 0.0;
        for i in 0..=400 {
            let r = reed_reflect(-1.4 + i as f32 * 0.01, 0.4);
            assert!(r >= last && r - last < 0.0101, "{i}: {last} to {r}");
            last = r;
        }
    }

    #[test]
    fn the_pictures_follow_the_knobs() {
        let at = |model: f32, reed: f32, edge: f32, tone: f32| {
            move |id: u16| match id {
                MODEL => model,
                REED => reed,
                EDGE => edge,
                TONE => tone,
                BREATH => 85.0,
                _ => 0.0,
            }
        };
        // A reed: the air rises, then shuts off; the breath sits past the
        // top, on the falling side.
        for reed in [0.0, 45.0, 100.0] {
            let (y, dot) = mouthpiece_curve(at(0.0, reed, 0.0, 50.0), 101);
            let top = y.iter().position(|&v| v >= 1.0).unwrap();
            assert!(y[0].abs() < 1e-6 && *y.last().unwrap() < 1e-3, "{reed}");
            assert!(top > 0 && top < 100);
            assert!(dot > top as f32 / 100.0 && dot < 1.0, "{reed}: {dot}");
        }
        // A harder reed shuts later.
        let shut = |reed: f32| {
            let (y, _) = mouthpiece_curve(at(1.0, reed, 0.0, 50.0), 101);
            y.iter().rposition(|&v| v > 1e-3).unwrap()
        };
        assert!(shut(100.0) > shut(0.0));
        // The flute's jet: odd about the middle with the edge centred, not
        // with it off centre.
        let (y, dot) = mouthpiece_curve(at(2.0, 0.0, 0.0, 50.0), 101);
        assert_eq!(dot, 0.5);
        assert!((y[20] + y[80]).abs() < 1e-3 && y[50].abs() < 1e-6);
        let (y, _) = mouthpiece_curve(at(2.0, 0.0, 100.0, 50.0), 101);
        assert!((y[20] + y[80]).abs() > 0.05);
        // Tone and the breath move the corner up; the filter falls past it.
        let c = |tone: f32, b: f32| tone_corner(at(0.0, 45.0, 0.0, tone), b);
        assert!(c(100.0, 0.67) > c(0.0, 0.67) && c(50.0, 1.0) > c(50.0, 0.3));
        let corner = c(50.0, 0.67);
        assert!((tone_gain(corner, 20.0) - 1.0).abs() < 0.01);
        assert!(tone_gain(corner, corner * 4.0) < 0.3);
    }

    #[test]
    fn the_breath_law_is_the_voice_cores() {
        for v in [1u8, 20, 64, 100, 127] {
            let b = f32::from(v) / 127.0;
            let want = crate::voice::velocity_gain(v, 24.0);
            assert!((breath_level(b, 24.0) - want).abs() < 1e-5, "{v}");
        }
        assert_eq!(breath_level(0.0, 24.0), 0.0);
    }

    #[test]
    fn lagrange_reads_whole_and_half_delays() {
        let mut line = vec![0.0f32; 64];
        for (i, x) in line.iter_mut().enumerate() {
            *x = i as f32;
        }
        // The last written is at 31: one back.
        let w = 32;
        assert!((lagrange(&line, w, 63, 1.0 + 3.0) - 28.0).abs() < 1e-4);
        assert!((lagrange(&line, w, 63, 4.5) - 27.5).abs() < 1e-4);
    }
}
