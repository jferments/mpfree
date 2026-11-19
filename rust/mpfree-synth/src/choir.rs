//! Choir: sung vowels, many singers on each note, built from
//! `docs/dev/research/organ-wind-choir.md` and the choir's own research
//! notes in `docs/dev/design/synths.md` (Choir).
//!
//! - **Singers.** Each note has up to eight singers, each a band-limited
//!   sawtooth ([`poly_blep4`]) at the note's pitch moved by the singer's
//!   own detune, vibrato and drift: a fast flutter and a slow wander, each
//!   singer their own, as the small uneven pitch changes that make a
//!   section sound like many people (Ternström). Each singer comes in at
//!   their own moment in the first few milliseconds of a note.
//! - **The voice.** The singers of a note are summed (harmonics falling
//!   6 dB an octave, as a voice's do at the lips), then a first-order
//!   tilt turning an octave over the note: a soft voice is darker, a loud
//!   one brighter (Brightness, velocity, pressure), its loudness kept.
//!   Breath noise joins it, louder in the open part of each cycle (gated
//!   before it is shaped, so the shaping's strong lows never reach the
//!   formants) and shaped to fall the same way.
//! - **The vowel.** Five resonators per note, each at a formant's
//!   frequency, bandwidth and level from the vowel table (the Csound
//!   manual's, [`TABLE`]): band-passes in parallel, the first against the
//!   other four, each lifted by its frequency so its peak has the table's
//!   level on the falling source while its skirts fall 12 dB an octave
//!   (on a flat source a parallel bank's skirts leave a fizz above the
//!   formants; this way an "oo" stays dark). That sign pattern was picked
//!   by measuring all sixteen over the table's 25 vowels: it gives the
//!   shallowest false notches and a valley between the first two formants
//!   close to an all-pole tract's. Vowel moves smoothly through "oo",
//!   "oh", "ah", "eh" and "ee" (the knob glides over 20 ms); every 5 % of
//!   Vowel, at every three semitones and for every Section, the voice and
//!   its breath are levelled by their loudness through the bank
//!   ([`VowelTable`], worked out once a process), so no vowel, blend or
//!   key is louder than the next.
//!   When a note rises past the vowel's first formant, the formant follows
//!   it, as sopranos tune it (Joliveau, Smith and Wolfe, 2004), and the
//!   second follows a little.
//! - **The Ensemble** (`mpfree_fx::Ensemble`) on the output, as on Strings.
//!
//! **Laid out for speed.** The values the per-sample loops touch are kept
//! in rows of four voices ([`LANES`]): each formant's filter state and
//! coefficients, each singer's phase, the tilt and the gains. The
//! formants run as one function, [`formants4`]: `mpfree-simd`'s NEON
//! kernel on the MPC, its plain Rust twin elsewhere, the same samples to
//! the bit (`simd.md`). Notes, swells, vibrato, the vowel and the gains move at a
//! control step of [`CTRL`] frames aligned to the absolute frame, the
//! gains ramping across the step, so the output does not depend on how
//! the host cuts blocks. Rows with no voice are skipped, and a tenth of a
//! second after the last voice the ensemble stops too. Nothing allocates
//! in `process`.

use mpfree_fx::ids::ensemble as ens;
use mpfree_fx::Ensemble;
use mpfree_node::pedal::Pedals;
use mpfree_node::pressure::{Pressure, PressureCoefs, PressureShape};
use mpfree_node::{
    Ctl, Ctx, Cull, Kind, Node, NodeEvent, NodeInfo, ParamInfo, Smooth, Timed, Tuning,
};

use crate::choir_params::*;
pub use crate::choir_params::{Formant, TABLE, VOWELS};
use crate::dsp::{poly_blep4, Rng};
use crate::expr::{expr_coef, NoteExprs, VoiceExpr};
use crate::params::Preset;
use crate::voice::{velocity_gain, Adsr, Glide, NoteStack, Phase, Priority, Voices};

/// Frames per control step.
pub const CTRL: u64 = 16;
const STEP: usize = CTRL as usize;
/// Voices in a row: the width of a NEON register of `f32`s.
pub const LANES: usize = 4;
/// The most voices that sound at once, and the slots: the voices plus
/// four spare for stolen voices to fade out in.
pub const MAX_VOICES: usize = 16;
const SLOTS: usize = MAX_VOICES + LANES;
const ROWS: usize = SLOTS / LANES;
/// Formants per voice.
pub const FORMANTS: usize = 5;
/// The most singers on a note.
pub const MAX_SINGERS: usize = 8;
/// Each formant's sign in the sum: the first against the rest.
pub const SIGNS: [f32; FORMANTS] = [1.0, -1.0, -1.0, -1.0, -1.0];
/// Where the tilt turns: an octave over the note (the voice's own tilt
/// with effort starts just above its fundamental), kept within these
/// (Hz); and how far Brightness tilts it each way (dB).
const TILT_OVER: f32 = 2.0;
const TILT_LOW: f32 = 100.0;
const TILT_HIGH: f32 = 2_500.0;
const SHELF_DB: f32 = 18.0;
/// The softest note at Vel > tone 100 against the hardest, dB of tilt.
const VEL_TONE_DB: f32 = 40.0;
/// Pressure: the swell's depth and how far the vowel moves at 100 %.
const PRESS_SWELL_DB: f32 = 12.0;
const PRESS_VOWEL_SPAN: f32 = 50.0;
/// The soft pedal: level and tilt.
const SOFT_GAIN: f32 = 0.5012;
const SOFT_DB: f32 = -15.0;
/// Vibrato the mod wheel adds at full, cents.
const WHEEL_CENTS: f32 = 40.0;
/// The singers' vibrato: each one's rate and depth differ by up to these
/// shares, so no two move together.
const VIB_RATE_SPREAD: f32 = 0.07;
const VIB_DEPTH_SPREAD: f32 = 0.25;
/// How long the vibrato takes to come in after its delay, ms.
const VIB_FADE_MS: f32 = 400.0;
/// Drift at 100 %: the flutter's and the wander's spread, cents, and how
/// fast each moves (the corner of its smoothing, Hz). Flutter is the
/// small fast part, under 20 cents and faster than about 5 a second
/// (Ternström and Friberg); the wander is slow.
const FLUTTER_CENTS: f32 = 8.0;
const FLUTTER_HZ: f32 = 6.0;
const WANDER_CENTS: f32 = 8.0;
const WANDER_HZ: f32 = 0.35;
/// How far each singer's place in the spread moves from note to note.
const DETUNE_JITTER: f32 = 0.3;
/// The singers come in over the first part of a note: at most this long
/// apart, and never more than half the attack.
const SCATTER_MS: f32 = 30.0;
/// A singer's level glides over this long when it comes in or out.
const SINGER_MS: f32 = 8.0;
/// A stolen voice fades out over this long.
const FADE_MS: f32 = 3.0;
/// The open part of each cycle, where breath is louder: the cycle's last
/// share from here.
const OPEN_FROM: f32 = 0.4;
const CLOSED_BREATH: f32 = 0.5;
/// Breath at 100 % against the voice: the noise's level for a flat
/// spectrum as strong as the singers' at the reference pitch.
const BREATH_FULL: f32 = 0.42;
/// The pitch whose singers sound at their own level; a note's singers are
/// scaled by the square root of this over its pitch, so the voice is about
/// as loud low as high.
const KEY_REF_HZ: f32 = 261.63;
/// The formant tuning: the first formant becomes the larger of its own
/// and the note's frequency, smoothly (a power mean of this order); the
/// second moves by this power of the first's ratio (our reading of
/// Joliveau et al.: proportionally less).
const TUNE_ORDER: f32 = 8.0;
const F2_FOLLOW: f32 = 1.0 / 3.0;
/// The narrowest a formant gets, as a share of the note's frequency.
const BW_FOLLOW: f32 = 0.5;
/// Every voice into the output: one note at velocity 100 peaks near
/// -16 dBFS, so an eight-note chord stays under full scale.
const OUT_GAIN: f32 = 2.7;
/// The sample rate the levels are set at.
const REF_RATE: f32 = 48_000.0;
/// The breath's shaping: a leaky integrator turning at this, Hz.
const NOISE_HZ: f32 = 20.0;
/// The ensemble's own depths at Depth 100 %, cents.
const SLOW_CENTS: f32 = 12.0;
const FAST_CENTS: f32 = 5.0;
/// How long the ensemble runs on after the last voice, ms.
const QUIET_MS: f32 = 100.0;

/// How fast the Vowel knob's value is followed (a one-pole's time
/// constant, ms), so turning it glides the formants and never steps them.
const VOWEL_MS: f32 = 20.0;
const INV_24: f32 = 1.0 / 16_777_216.0;
const TURN: f32 = 4_294_967_296.0;

static INFO: NodeInfo = NodeInfo {
    id: "mpfree.choir",
    name: "Choir",
    kind: Kind::Instrument,
};

/// The built-in Ensemble's settings for these values: ids and values of
/// `mpfree_fx::ids::ensemble`.
pub fn ensemble_settings(v: impl Fn(u16) -> f32) -> [(u16, f32); 7] {
    let depth = v(DEPTH) / 100.0;
    [
        (ens::MIX, v(ENSEMBLE)),
        (ens::LINES, v(LINES)),
        (ens::SLOW_RATE, v(SLOW_RATE)),
        (ens::FAST_RATE, v(FAST_RATE)),
        (ens::SLOW_DEPTH, (SLOW_CENTS * depth).min(30.0)),
        (ens::FAST_DEPTH, (FAST_CENTS * depth).min(15.0)),
        (ens::WIDTH, v(WIDTH)),
    ]
}

// ------------------------------------------------------------ the vowel

/// The two table sections a note blends and the share of the second, for
/// Section value `section` and a note sounding at `pitch` (MIDI note).
pub fn section_blend(section: f32, pitch: f32) -> (usize, usize, f32) {
    let s = section.round();
    if s < MIXED {
        let i = (s.max(0.0) as usize).min(4);
        return (i, i, 0.0);
    }
    let m = &MIXED_MIDDLES;
    if pitch <= m[0].1 {
        return (m[0].0 as usize, m[0].0 as usize, 0.0);
    }
    for w in m.windows(2) {
        let (a, b) = (w[0], w[1]);
        if pitch < b.1 {
            return (a.0 as usize, b.0 as usize, (pitch - a.1) / (b.1 - a.1));
        }
    }
    let top = m[m.len() - 1].0 as usize;
    (top, top, 0.0)
}

/// The vowel table in the forms the blend wants: each formant's
/// frequency and bandwidth as logs, its level in dB; and, for each
/// Section (Mixed too) at each of the trim vowels and pitches, the level
/// trim (dB) that brings the loudness of the formants as sung there
/// (blended and tuned) to that of the Alto's "Aah" at middle C.
#[derive(Clone, Copy, Debug)]
pub struct VowelTable {
    log: [[[(f32, f32, f32); FORMANTS]; 5]; 5],
    trim: [[[f32; TRIM_POINTS]; TRIM_VOWELS]; 6],
    /// The share of each one's loudness above the tilt's turn, so the
    /// tilt can keep the loudness where it was.
    high: [[[f32; TRIM_POINTS]; TRIM_VOWELS]; 6],
    /// The trim (dB) that brings breath through the formants as sung there
    /// to its loudness through the Alto's "Aah" at middle C.
    breath: [[[f32; TRIM_POINTS]; TRIM_VOWELS]; 6],
}

/// What a voice sings for a Section, Vowel and note: its five formants,
/// the trims (dB) for its singers and for its breath, and the share of
/// its singers' loudness above the tilt's turn.
#[derive(Clone, Copy, Debug)]
pub struct Sung {
    pub formants: [Formant; FORMANTS],
    pub trim_db: f32,
    pub breath_db: f32,
    pub high: f32,
}

/// Where the tilt turns for a note of `hz`.
pub fn tilt_hz(hz: f32) -> f32 {
    (hz * TILT_OVER).clamp(TILT_LOW, TILT_HIGH)
}

/// The pitches the level trims are worked out at: every `TRIM_STEP`
/// semitones from `TRIM_LOW` (C1) to C7, and the trims' limit (dB).
const TRIM_LOW: f32 = 24.0;
const TRIM_STEP: f32 = 3.0;
const TRIM_POINTS: usize = 25;
const TRIM_MAX_DB: f32 = 12.0;
/// The vowels the trims are worked out at: every 5 % of Vowel, as a blend
/// of two table vowels is not as loud as either.
const TRIM_VOWELS: usize = 21;

/// A formant's gain at `hz` (complex, as `(re, im)`): a band-pass at `fc`
/// with bandwidth `bw` and a peak of 1, as the analog prototype the
/// filters are made from.
fn band_pass(fc: f32, bw: f32, hz: f32) -> (f32, f32) {
    let k = bw / fc;
    let w = hz / fc;
    // k jw / (1 - w^2 + k jw)
    let (re_d, im_d) = (1.0 - w * w, k * w);
    let (re_n, im_n) = (0.0, k * w);
    let d = re_d * re_d + im_d * im_d;
    (
        (re_n * re_d + im_n * im_d) / d,
        (im_n * re_d - re_n * im_d) / d,
    )
}

/// Each formant's lift: the source's harmonics fall 6 dB an octave (a
/// sawtooth's, `2 / (pi n)`), so a formant at `fc` is lifted to put its
/// peak at the table's level, as a flat source of the same level as the
/// old pulses at 48 kHz would (`4 f0 / fs`).
#[inline]
fn lift(fc: f32) -> f32 {
    std::f32::consts::TAU * fc / REF_RATE
}

/// The voice's gain at `hz` for five formants (Hz, dB, bandwidth Hz):
/// the bank, each formant lifted by its frequency, on the source's
/// falling harmonics, so each formant peaks at its table level.
pub fn bank_gain(formants: &[Formant; FORMANTS], hz: f32) -> f32 {
    let (mut re, mut im) = (0.0f32, 0.0f32);
    for (k, &(fc, db, bw)) in formants.iter().enumerate() {
        let a = SIGNS[k] * db_gain(db) * fc / hz;
        let (r, i) = band_pass(fc, bw, hz);
        re += a * r;
        im += a * i;
    }
    (re * re + im * im).sqrt()
}

/// The loudness-weighted power of a sung note of `hz` through the bank:
/// its harmonics (all of one level, as the source makes them) up to
/// 10 kHz, each weighted as loudness meters weigh it (4 dB more above
/// about 1.5 kHz, less below about 40 Hz), times the note's frequency so
/// that a low note, whose harmonics sample the bank closely, gives the
/// bank's power for a flat spectrum. And the share of it above the
/// tilt's turn: through the turn's one-pole high-pass, whose power and
/// its low-pass's add up to the whole at every frequency.
fn sung_power(formants: &[Formant; FORMANTS], hz: f32) -> (f32, f32) {
    let turn2 = tilt_hz(hz).powi(2);
    let mut p = 0.0;
    let mut high = 0.0;
    let mut k = 1.0;
    while hz * k <= 10_000.0 {
        let f = hz * k;
        let g = bank_gain(formants, f);
        let f2 = f * f;
        let x = g * g * loudness_weight(f);
        p += x;
        high += x * f2 / (f2 + turn2);
        k += 1.0;
    }
    (p * hz, high / p.max(1e-30))
}

/// How loudness meters weigh a frequency's power: 4 dB more above about
/// 1.5 kHz, less below about 40 Hz.
fn loudness_weight(f: f32) -> f32 {
    let f2 = f * f;
    let shelf = 1.0 + (10f32.powf(0.4) - 1.0) * f2 / (f2 + 1_500.0 * 1_500.0);
    let low = f2 * f2 / (f2 * f2 + 40f32.powi(4));
    shelf * low
}

/// The loudness-weighted power of breath through the bank: a spectrum
/// falling as the singers' harmonics do, every frequency from 20 Hz to
/// 10 kHz (sixteen steps an octave), where the harmonics sample it only
/// at the note's multiples.
fn breath_power(formants: &[Formant; FORMANTS]) -> f32 {
    const STEP: f32 = 1.044_273_8; // 2^(1/16)
    let mut p = 0.0;
    let mut f = 20.0f32;
    while f <= 10_000.0 {
        let g = bank_gain(formants, f);
        p += g * g * loudness_weight(f) * f * (STEP - 1.0);
        f *= STEP;
    }
    p
}

impl VowelTable {
    /// The table with its formants only, the trims all 0: for drawing,
    /// which wants the vowel's shape and not its level. Cheap.
    pub fn shapes() -> VowelTable {
        let mut log = [[[(0.0, 0.0, 0.0); FORMANTS]; 5]; 5];
        for s in 0..5 {
            for v in 0..5 {
                for k in 0..FORMANTS {
                    let (hz, db, bw) = TABLE[s][v][k];
                    log[s][v][k] = (hz.ln(), db, bw.ln());
                }
            }
        }
        VowelTable {
            log,
            trim: [[[0.0; TRIM_POINTS]; TRIM_VOWELS]; 6],
            high: [[[0.0; TRIM_POINTS]; TRIM_VOWELS]; 6],
            breath: [[[0.0; TRIM_POINTS]; TRIM_VOWELS]; 6],
        }
    }

    /// The table with its level trims and tilt shares worked out (some
    /// milliseconds; [`VowelTable::shared`] does it once a process).
    pub fn new() -> VowelTable {
        let mut t = VowelTable::shapes();
        let sung = |t: &VowelTable, section: f32, vowel: f32, pitch: f32| {
            let hz = 440.0 * ((pitch - 69.0) / 12.0).exp2();
            let f = t.sung(section, vowel, pitch, hz).formants;
            let (p, h) = sung_power(&f, hz);
            (p, h, breath_power(&f))
        };
        let (reference, _, breath_ref) = sung(&t, ALTO, 50.0, 60.0);
        let mut trim = [[[0.0; TRIM_POINTS]; TRIM_VOWELS]; 6];
        let mut high = [[[0.0; TRIM_POINTS]; TRIM_VOWELS]; 6];
        let mut breath = [[[0.0; TRIM_POINTS]; TRIM_VOWELS]; 6];
        for s in 0..6 {
            for v in 0..TRIM_VOWELS {
                let vowel = 100.0 * v as f32 / (TRIM_VOWELS - 1) as f32;
                for i in 0..TRIM_POINTS {
                    let (p, h, b) = sung(&t, s as f32, vowel, TRIM_LOW + TRIM_STEP * i as f32);
                    let db = 10.0 * (reference / p).log10();
                    trim[s][v][i] = db.clamp(-TRIM_MAX_DB, TRIM_MAX_DB);
                    high[s][v][i] = h;
                    let db = 10.0 * (breath_ref / b).log10();
                    breath[s][v][i] = db.clamp(-TRIM_MAX_DB, TRIM_MAX_DB);
                }
            }
        }
        t.trim = trim;
        t.high = high;
        t.breath = breath;
        t
    }

    /// The table with its trims, worked out the first time it is asked
    /// for in a process and shared by every choir after.
    pub fn shared() -> &'static VowelTable {
        static SHARED: std::sync::OnceLock<VowelTable> = std::sync::OnceLock::new();
        SHARED.get_or_init(VowelTable::new)
    }

    /// The level trim (dB) for `section` (Section's value) and `vowel` (0
    /// to 100) sung at `pitch` (MIDI note).
    pub fn trim(&self, section: f32, vowel: f32, pitch: f32) -> f32 {
        at_vowel_pitch(&self.trim, section, vowel, pitch)
    }

    /// The share of the loudness above the tilt's turn for `section`
    /// and `vowel` sung at `pitch` (MIDI note).
    pub fn high_share(&self, section: f32, vowel: f32, pitch: f32) -> f32 {
        at_vowel_pitch(&self.high, section, vowel, pitch)
    }

    /// What a voice sings for `section` (Section's value), `vowel` (0 to
    /// 100) and a note sounding at `pitch` (MIDI note) of `hz`: the
    /// table's vowels blended, then the first two formants tuned to the
    /// note; and its trims.
    pub fn sung(&self, section: f32, vowel: f32, pitch: f32, hz: f32) -> Sung {
        let (sa, sb, ts) = section_blend(section, pitch);
        let x = (vowel / 25.0).clamp(0.0, 4.0);
        let vi = (x.floor() as usize).min(3);
        let tv = x - vi as f32;
        let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
        let mut out = [(0.0, 0.0, 0.0); FORMANTS];
        for (k, o) in out.iter_mut().enumerate() {
            let at = |s: usize| {
                let (a, b) = (self.log[s][vi][k], self.log[s][vi + 1][k]);
                (lerp(a.0, b.0, tv), lerp(a.1, b.1, tv), lerp(a.2, b.2, tv))
            };
            let (a, b) = (at(sa), at(sb));
            *o = (
                lerp(a.0, b.0, ts).exp(),
                lerp(a.1, b.1, ts),
                lerp(a.2, b.2, ts).exp(),
            );
        }
        tune(&mut out, hz);
        Sung {
            formants: out,
            trim_db: self.trim(section, vowel, pitch),
            breath_db: at_vowel_pitch(&self.breath, section, vowel, pitch),
            high: self.high_share(section, vowel, pitch),
        }
    }
}

/// A value from a table over the trim vowels and pitches, for `section`
/// (Section's value) at `vowel` (0 to 100) and `pitch` (MIDI note).
fn at_vowel_pitch(
    t: &[[[f32; TRIM_POINTS]; TRIM_VOWELS]; 6],
    section: f32,
    vowel: f32,
    pitch: f32,
) -> f32 {
    let rows = &t[(section.round().max(0.0) as usize).min(5)];
    let y = (vowel / 100.0 * (TRIM_VOWELS - 1) as f32).clamp(0.0, (TRIM_VOWELS - 1) as f32);
    let j = (y as usize).min(TRIM_VOWELS - 2);
    let fy = y - j as f32;
    let x = ((pitch - TRIM_LOW) / TRIM_STEP).clamp(0.0, (TRIM_POINTS - 1) as f32);
    let i = (x as usize).min(TRIM_POINTS - 2);
    let fx = x - i as f32;
    let at = |r: &[f32; TRIM_POINTS]| r[i] + (r[i + 1] - r[i]) * fx;
    let (a, b) = (at(&rows[j]), at(&rows[j + 1]));
    a + (b - a) * fy
}

impl Default for VowelTable {
    fn default() -> Self {
        Self::new()
    }
}

/// The formant tuning for a note of `hz`. Each formant is at least half
/// the note wide, its level lowered to keep its power (so a high note's
/// few harmonics fall on a smooth curve, not on or off a sharp peak);
/// the first formant is at least the note (a power mean, so it bends
/// smoothly as the note nears it) and the second rises by a third power
/// of the first's ratio, staying below the third.
pub fn tune(f: &mut [Formant; FORMANTS], hz: f32) {
    let floor = hz.max(0.0) * BW_FOLLOW;
    for x in f.iter_mut() {
        if x.2 < floor {
            x.1 += 10.0 * (x.2 / floor).log10();
            x.2 = floor;
        }
    }
    let f1 = f[0].0;
    let tuned = (f1.powf(TUNE_ORDER) + hz.max(0.0).powf(TUNE_ORDER)).powf(1.0 / TUNE_ORDER);
    if tuned > f1 {
        f[0].0 = tuned;
        let f2 = (f[1].0 * (tuned / f1).powf(F2_FOLLOW)).max(tuned * 1.15);
        f[1].0 = f2.min(f[2].0 * 0.9).max(f[1].0);
    }
}

// ------------------------------------------------------------ drawing

/// The pitch the Synth tab draws a vowel at: middle C.
pub const DRAW_PITCH: f32 = 60.0;

/// The vowel as set, for drawing: the voice's gain at a frequency (Hz) for
/// a note at middle C, its loudest point at 1.
pub fn vowel_response(v: impl Fn(u16) -> f32) -> impl Fn(f32) -> f32 {
    let hz = 440.0 * ((DRAW_PITCH - 69.0) / 12.0).exp2();
    let f = VowelTable::shapes()
        .sung(v(SECTION), v(VOWEL), DRAW_PITCH, hz)
        .formants;
    let top = (0..400)
        .map(|i| bank_gain(&f, 80.0 * 100f32.powf(i as f32 / 399.0)))
        .fold(1e-9f32, f32::max);
    move |x| bank_gain(&f, x) / top
}

/// The Swell card: a held note's level (0 to 1) at `points` even times
/// over `secs`, let go at `held`.
pub fn swell_curve(v: impl Fn(u16) -> f32, held: f32, secs: f32, points: usize) -> Vec<f32> {
    const SPS: f32 = 4_000.0;
    let mut env = Adsr::default();
    env.set(v(ATTACK), v(DECAY), v(SUSTAIN) / 100.0, v(RELEASE), SPS);
    env.gate_on();
    let off = (held * SPS) as usize;
    let mut step = 0usize;
    let mut level = 0.0;
    (0..points)
        .map(|i| {
            let at = (i as f32 / (points.max(2) - 1) as f32 * secs * SPS) as usize;
            while step < at {
                if step == off {
                    env.gate_off();
                }
                level = env.next();
                step += 1;
            }
            level
        })
        .collect()
}

/// The Singers card: each singer's pitch against the note (cents) at
/// `points` even times over `secs` seconds of a held note: their places
/// by Detune, their vibrato (each their own rate, depth and start) and,
/// for the drawing, their drift's slow wander as a smooth curve. The same
/// singers each time (a fixed seed), so the picture holds still.
pub fn singer_lines(v: impl Fn(u16) -> f32, secs: f32, points: usize) -> Vec<Vec<f32>> {
    let n = (v(SINGERS).round() as usize).clamp(1, MAX_SINGERS);
    let sd = ((n * n - 1) as f32 / 12.0).sqrt();
    let mut rng = Rng::new(0xC401);
    let delay = v(VIB_DELAY) * 0.001;
    let fade = VIB_FADE_MS * 0.001;
    (0..n)
        .map(|s| {
            let jitter = DETUNE_JITTER * rng.bipolar();
            let place = if n > 1 {
                (s as f32 - (n - 1) as f32 * 0.5) / sd + jitter
            } else {
                0.0
            };
            let phase = rng.unit();
            let rate = v(VIB_RATE) * (1.0 + VIB_RATE_SPREAD * rng.bipolar());
            let depth = v(VIBRATO) * (1.0 + VIB_DEPTH_SPREAD * rng.bipolar());
            let (w1, w2) = (rng.unit(), rng.unit());
            let drift = v(DRIFT) / 100.0 * WANDER_CENTS;
            (0..points)
                .map(|i| {
                    let t = i as f32 / (points.max(2) - 1) as f32 * secs;
                    let on = ((t - delay) / fade).clamp(0.0, 1.0);
                    let wander = 0.6 * sin_turns(WANDER_HZ * t + w1)
                        + 0.4 * sin_turns(WANDER_HZ * 1.7 * t + w2);
                    place * v(DETUNE) + depth * on * sin_turns(rate * t + phase) + drift * wander
                })
                .collect()
        })
        .collect()
}

/// The ensemble's lines, for drawing: each line's delay against the
/// lines' shared delay (-1 to 1 of the widest sweep) at `points` even
/// times across one slow cycle. Empty with the ensemble off.
pub fn ensemble_lines(v: impl Fn(u16) -> f32, points: usize) -> Vec<Vec<f32>> {
    if v(ENSEMBLE) <= 0.0 {
        return Vec::new();
    }
    let mut e = Ensemble::new();
    for (id, value) in ensemble_settings(&v) {
        e.set_param(id, value);
    }
    let (slow, fast, base) = e.sweeps_ms();
    let span = (slow + fast).max(1e-3);
    let ratio = v(FAST_RATE) / v(SLOW_RATE).max(1e-3);
    (0..4)
        .map_while(|k| {
            (0..points)
                .map(|i| {
                    let x = i as f32 / (points.max(2) - 1) as f32;
                    e.delay_ms(k, x, (ratio * x).fract())
                        .map(|d| (d - base) / span)
                })
                .collect::<Option<Vec<f32>>>()
        })
        .collect()
}

#[inline]
fn db_gain(db: f32) -> f32 {
    (db * (std::f32::consts::LN_10 / 20.0)).exp()
}

/// `2^x` for the small pitch moves of a singer (|x| under a tenth of an
/// octave): a quartic, within 1e-7 there (0.03 cents at a quarter).
#[inline]
fn exp2_small(x: f32) -> f32 {
    let y = x * std::f32::consts::LN_2;
    1.0 + y * (1.0 + y * (0.5 + y * (1.0 / 6.0 + y * (1.0 / 24.0))))
}

#[inline]
fn sin_turns(t: f32) -> f32 {
    (t * std::f32::consts::TAU).sin()
}

// ------------------------------------------------------------- rows

/// Four voices' formant filters, side by side as `[formant][lane]` (each
/// a zero-delay state-variable band-pass, Simper's, with its state and
/// signed gain), and the function that runs four voices' sources through
/// their five formants, frame by frame. Both live in `mpfree-simd`, the
/// function as a NEON kernel where NEON runs and as its plain Rust twin
/// elsewhere, the same samples to the bit (`docs/dev/design/simd.md`).
/// The engine's loop was the twin's model: every step a plain multiply
/// or add in a fixed order with no branch on a value.
pub use mpfree_simd::{formants4, Formants4};

// The kernel's shape is the engine's.
const _: () = assert!(FORMANTS == mpfree_simd::FORMANTS && LANES == 4);

/// Four voices' per-sample values, side by side: `[formant][lane]` and
/// `[singer][lane]`.
#[derive(Clone, Copy, Debug, Default)]
struct Row {
    /// The formants' filters.
    f: Formants4,
    // The singers.
    phase: [[u32; LANES]; MAX_SINGERS],
    inc: [[u32; LANES]; MAX_SINGERS],
    dt: [[f32; LANES]; MAX_SINGERS],
    sgain: [[f32; LANES]; MAX_SINGERS],
    dsgain: [[f32; LANES]; MAX_SINGERS],
    /// The breath's shaping (a leaky integrator's state).
    nlp: [f32; LANES],
    // The tilt: a one-pole low-pass and the gain above it.
    lp: [f32; LANES],
    /// The tilt's one-pole coefficient (its turn follows the note).
    shelf: [f32; LANES],
    tilt: [f32; LANES],
    dtilt: [f32; LANES],
    // Breath.
    rng: [u32; LANES],
    noise: [f32; LANES],
    dnoise: [f32; LANES],
    // The voice's level.
    gain: [f32; LANES],
    dgain: [f32; LANES],
}

impl Row {
    /// Lane `l` back to silence.
    fn clear_lane(&mut self, l: usize) {
        for k in 0..FORMANTS {
            self.f.s1[k][l] = 0.0;
            self.f.s2[k][l] = 0.0;
        }
        for s in 0..MAX_SINGERS {
            self.sgain[s][l] = 0.0;
            self.dsgain[s][l] = 0.0;
        }
        self.nlp[l] = 0.0;
        self.lp[l] = 0.0;
        self.noise[l] = 0.0;
        self.dnoise[l] = 0.0;
        self.gain[l] = 0.0;
        self.dgain[l] = 0.0;
    }

    /// Whether any lane can make sound this step.
    fn live(&self) -> bool {
        (0..LANES).any(|l| self.gain[l] != 0.0 || self.dgain[l] != 0.0)
    }
}

// ------------------------------------------------------------ voices

/// One singer's slow values.
#[derive(Clone, Copy, Debug, Default)]
struct Singer {
    /// Its place in the spread, in Detunes: evenly apart for the singers
    /// there are, plus its own jitter for this note.
    place: f32,
    jitter: f32,
    vib_phase: f32,
    vib_rate: f32,
    vib_depth: f32,
    flutter: f32,
    wander: f32,
    /// Control steps until it comes in.
    wait: u32,
    /// Its level now (control rate).
    gain: f32,
}

/// One voice's slow values.
#[derive(Clone, Copy, Debug)]
struct Voice {
    env: Adsr,
    /// Stolen or culled: its fade, 1 to 0, and the step down each control
    /// step.
    fade: f32,
    fade_step: f32,
    /// The key it plays, and its pitch (MIDI note, with glide).
    note: u8,
    pitch: f32,
    target: f32,
    vel_gain: f32,
    /// Velocity's tilt, dB.
    vel_tilt: f32,
    press: Pressure,
    /// Its own slide (semitones) and tone (0 to 1).
    ex: VoiceExpr,
    /// Control steps since it started.
    age: u32,
    singers: [Singer; MAX_SINGERS],
    rng: Rng,
    /// The formants' inputs last set (section, vowel, pitch, Hz) and the
    /// level trim they gave.
    key: [f32; 4],
    trim: f32,
    /// The breath's level against the trim: its own trim over the voice's.
    breath: f32,
    /// The share of its loudness above the tilt's turn.
    high: f32,
    /// Started from silence and not stepped yet: its tilt starts where it
    /// is set, not ramping from the lane's last.
    fresh: bool,
}

impl Default for Voice {
    fn default() -> Voice {
        Voice {
            env: Adsr::default(),
            fade: 1.0,
            fade_step: 0.0,
            note: 0,
            pitch: 60.0,
            target: 60.0,
            vel_gain: 1.0,
            vel_tilt: 0.0,
            press: Pressure::new(),
            ex: VoiceExpr::default(),
            age: 0,
            singers: [Singer::default(); MAX_SINGERS],
            rng: Rng::new(0),
            key: [f32::NAN; 4],
            trim: 0.0,
            breath: 1.0,
            high: 0.0,
            fresh: false,
        }
    }
}

/// A smoothing share per control step for a one-pole with corner `hz`,
/// and the input scale that keeps its output's spread at 1 for white
/// input of spread 1 (its noise gain).
fn smoother(hz: f32, steps_per_sec: f32) -> (f32, f32) {
    let a = (1.0 - (-std::f32::consts::TAU * hz / steps_per_sec).exp()).clamp(1e-6, 1.0);
    (a, ((2.0 - a) / a).sqrt())
}

/// The choir. See the module docs.
pub struct Choir {
    values: [f32; ID_SPACE],
    rate: u32,
    now: u64,
    quiet: u32,
    tuning: Tuning,
    table: &'static VowelTable,
    slots: Voices<SLOTS>,
    v: [Voice; SLOTS],
    rows: [Row; ROWS],
    stack: NoteStack,
    pedals: Pedals,
    /// Each key's last pressure, and its expression waiting for its next
    /// note.
    pressure: [u8; 128],
    exprs: NoteExprs,
    expr_c: f32,
    press_coefs: PressureCoefs,
    /// The breath's leaky integrator's coefficient.
    noise_c: f32,
    glide: Glide,
    bend: f32,
    wheel: f32,
    soft: bool,
    expression: f32,
    level: Smooth,
    /// The Vowel knob, followed once a control step.
    vowel: Smooth,
    morph_phase: f32,
    /// Singer slots that may still have a level.
    singers_run: usize,
    seed: u64,
    ensemble: Ensemble,
}

impl Choir {
    pub fn new(rate: u32) -> Choir {
        let mut values = [0.0; ID_SPACE];
        for p in PARAMS {
            values[p.id as usize] = p.default;
        }
        let mut c = Choir {
            values,
            rate: 0,
            now: 0,
            quiet: u32::MAX,
            tuning: Tuning::standard(),
            table: VowelTable::shared(),
            slots: Voices::new(),
            v: [Voice::default(); SLOTS],
            rows: [Row::default(); ROWS],
            stack: NoteStack::default(),
            pedals: Pedals::new(),
            pressure: [0; 128],
            exprs: NoteExprs::new(),
            expr_c: 1.0,
            press_coefs: PressureCoefs::default(),
            noise_c: 1.0,
            glide: Glide::default(),
            bend: 0.0,
            wheel: 0.0,
            soft: false,
            expression: 1.0,
            level: Smooth::new(1.0),
            vowel: Smooth::new(50.0),
            morph_phase: 0.0,
            singers_run: 0,
            seed: 0xC401,
            ensemble: Ensemble::new(),
        };
        c.prepare(rate, mpfree_node::MAX_BLOCK);
        c
    }

    pub fn presets() -> &'static [Preset] {
        crate::factory::presets!("choir.presets", &INFO, PARAMS)
    }

    /// Sets every parameter to its default, then the preset's values.
    pub fn load_preset(&mut self, preset: &Preset) {
        for p in PARAMS.iter().filter(|p| !p.readonly) {
            self.set_param(p.id, p.default);
        }
        for &(id, v) in preset.values {
            self.set_param(id, v);
        }
    }

    /// Notes sounding, fading out included.
    pub fn active_voices(&self) -> usize {
        self.slots.busy()
    }

    /// Voices stolen to stay within Voices.
    pub fn stolen(&self) -> u64 {
        self.slots.stolen
    }

    /// Each sounding voice's pressure, 0 to 1, for the Pressure page's
    /// live bars. Returns how many it wrote.
    pub fn pressures(&self, out: &mut [f32]) -> usize {
        let mut n = 0;
        for i in 0..SLOTS {
            if n == out.len() {
                break;
            }
            if self.slots.slots[i].sounding() {
                out[n] = self.v[i].press.value();
                n += 1;
            }
        }
        n
    }

    /// The vowel table this choir sings from.
    pub fn vowel_table(&self) -> &VowelTable {
        self.table
    }

    /// Voice `slot`'s singer `s`'s pitch against the note's, cents, as last
    /// set (None for a free slot): for the tests that check the singers
    /// move apart.
    pub fn singer_cents(&self, slot: usize, s: usize) -> Option<f32> {
        let st = self.slots.slots.get(slot)?;
        if st.phase == Phase::Free || s >= MAX_SINGERS {
            return None;
        }
        let row = &self.rows[slot / LANES];
        let inc = row.inc[s][slot % LANES];
        let hz = f64::from(inc) / f64::from(TURN) * f64::from(self.rate);
        let base = f64::from(self.note_hz(&self.v[slot]));
        Some((1200.0 * (hz / base).log2()) as f32)
    }

    /// Voice `slot`'s five formants as last set (Hz, dB, bandwidth): for
    /// the tests and the lab.
    pub fn voice_formants(&self, slot: usize) -> Option<[Formant; FORMANTS]> {
        let st = self.slots.slots.get(slot)?;
        if st.phase == Phase::Free {
            return None;
        }
        let k = self.v[slot].key;
        Some(self.table.sung(k[0], k[1], k[2], k[3]).formants)
    }

    /// Voice `slot`'s formant `k` filter's gain at `hz` on the source's
    /// falling harmonics, worked out from the coefficients it runs with
    /// (None for a free slot): for the tests and the lab, which find each
    /// formant's peak and width from it.
    pub fn formant_gain(&self, slot: usize, k: usize, hz: f32) -> Option<f32> {
        let st = self.slots.slots.get(slot)?;
        if st.phase == Phase::Free || k >= FORMANTS {
            return None;
        }
        let row = &self.rows[slot / LANES];
        let l = slot % LANES;
        let a1 = f64::from(row.f.a1[k][l]);
        let g = f64::from(row.f.a2[k][l]) / a1;
        let kk = (1.0 / a1 - 1.0) / g - g;
        // The band-pass as the filter runs it:
        // g (1 - z^-2) / ((1 + gk + g^2) + 2 (g^2 - 1) z^-1 + (1 - gk + g^2) z^-2)
        let w = std::f64::consts::TAU * f64::from(hz) / f64::from(self.rate);
        let (c1, s1, c2, s2) = (w.cos(), w.sin(), (2.0 * w).cos(), (2.0 * w).sin());
        let (num_re, num_im) = (g * (1.0 - c2), g * s2);
        let (d0, d1, d2) = (
            1.0 + g * kk + g * g,
            2.0 * (g * g - 1.0),
            1.0 - g * kk + g * g,
        );
        let den_re = d0 + d1 * c1 + d2 * c2;
        let den_im = -d1 * s1 - d2 * s2;
        let mag = (num_re.hypot(num_im) / den_re.hypot(den_im)) * f64::from(row.f.g[k][l]).abs();
        // On the source's falling harmonics.
        Some((mag / f64::from(lift(hz))) as f32)
    }

    /// The slots in use, for the tests.
    pub fn busy_slots(&self) -> impl Iterator<Item = usize> + '_ {
        (0..SLOTS).filter(|&i| self.slots.slots[i].phase != Phase::Free)
    }

    fn v(&self, id: u16) -> f32 {
        self.values[id as usize]
    }

    fn steps_per_sec(&self) -> f32 {
        self.rate as f32 / CTRL as f32
    }

    fn mono(&self) -> bool {
        self.v(MODE) >= 0.5
    }

    fn singers(&self) -> usize {
        (self.v(SINGERS).round() as usize).clamp(1, MAX_SINGERS)
    }

    fn update_env(&mut self) {
        let sps = self.steps_per_sec();
        let (a, d, s, r) = (
            self.v(ATTACK),
            self.v(DECAY),
            self.v(SUSTAIN) / 100.0,
            self.v(RELEASE),
        );
        for v in self.v.iter_mut() {
            v.env.set(a, d, s, r, sps);
        }
    }

    fn update_ensemble(&mut self) {
        for (id, value) in ensemble_settings(|id| self.v(id)) {
            self.ensemble.set_param(id, value);
        }
    }

    fn update_level(&mut self) {
        let soft = if self.soft { SOFT_GAIN } else { 1.0 };
        self.level.target = db_gain(self.v(LEVEL)) * soft * self.expression * OUT_GAIN;
    }

    fn update_glide(&mut self) {
        self.glide = Glide::time(self.v(GLIDE), self.steps_per_sec());
    }

    /// The pitch a key sounds at (MIDI note), before glide.
    fn key_pitch(&self, note: u8) -> f32 {
        f32::from(note) + self.v(TRANSPOSE) + 12.0 * self.v(OCTAVE) + self.v(FINE) * 0.01
    }

    /// The pitch voice `v` sounds at now, with bend and its slide.
    fn sounding(&self, v: &Voice) -> f32 {
        v.pitch + self.bend + v.ex.slide
    }

    fn note_hz(&self, v: &Voice) -> f32 {
        self.tuning.hz(self.sounding(v)).max(0.0)
    }

    /// Starts voice `i` on `note` from silence: singers, phases and filters
    /// fresh.
    fn start_voice(&mut self, i: usize, note: u8, velocity: u8) {
        let sps = self.steps_per_sec();
        let scatter = (SCATTER_MS.min(self.v(ATTACK) * 0.5) * 0.001 * sps).max(0.0);
        self.seed = self.seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut rng = Rng::new(self.seed);
        let row = &mut self.rows[i / LANES];
        let l = i % LANES;
        row.clear_lane(l);
        row.rng[l] = (rng.next_u64() as u32) | 1;
        for s in 0..MAX_SINGERS {
            row.phase[s][l] = rng.next_u64() as u32;
        }
        let pitch = self.key_pitch(note);
        let n = self.exprs.on(note);
        let v = &mut self.v[i];
        v.env.clear();
        v.env.gate_on();
        v.fade = 1.0;
        v.fade_step = 0.0;
        v.note = note;
        v.target = pitch;
        v.pitch = pitch;
        v.press.clear();
        v.press.start();
        v.age = 0;
        v.key = [f32::NAN; 4];
        v.fresh = true;
        v.ex.start(n);
        for (s, singer) in v.singers.iter_mut().enumerate() {
            *singer = Singer {
                place: 0.0,
                jitter: DETUNE_JITTER * rng.bipolar(),
                vib_phase: rng.unit(),
                vib_rate: 1.0 + VIB_RATE_SPREAD * rng.bipolar(),
                vib_depth: 1.0 + VIB_DEPTH_SPREAD * rng.bipolar(),
                flutter: rng.bipolar() * 1.7,
                wander: rng.bipolar() * 1.7,
                wait: if s == 0 {
                    0
                } else {
                    (rng.unit() * scatter) as u32
                },
                gain: 0.0,
            };
        }
        v.rng = rng;
        self.set_velocity(i, velocity);
        self.set_places(i);
    }

    /// The singers' places in the spread: evenly apart with a spread
    /// (standard deviation) of 1 for the singers there are, each moved by
    /// its jitter for this note. One singer sings the note.
    fn set_places(&mut self, i: usize) {
        let n = self.singers();
        let sd = ((n * n - 1) as f32 / 12.0).sqrt();
        for (s, singer) in self.v[i].singers.iter_mut().enumerate() {
            singer.place = if n > 1 {
                (s.min(n - 1) as f32 - (n - 1) as f32 * 0.5) / sd + singer.jitter
            } else {
                0.0
            };
        }
    }

    fn set_velocity(&mut self, i: usize, velocity: u8) {
        let range = self.v(VELOCITY);
        let tone = self.v(VEL_TONE) / 100.0;
        let soft = 1.0 - f32::from(velocity.min(127)) / 127.0;
        let v = &mut self.v[i];
        v.vel_gain = velocity_gain(velocity, range);
        v.vel_tilt = -VEL_TONE_DB * tone * soft;
    }

    /// Voice `i` fades out over [`FADE_MS`] (stolen or culled).
    fn fade_out(&mut self, i: usize) {
        let steps = (FADE_MS * 0.001 * self.steps_per_sec()).max(1.0);
        let v = &mut self.v[i];
        v.fade_step = 1.0 / steps;
    }

    fn note_on(&mut self, note: u8, velocity: u8) {
        let note = note & 0x7F;
        self.pedals.press(note);
        // Nothing sounding: the vowel turned since needs no glide.
        if self.slots.busy() == 0 {
            self.vowel.snap(self.v(VOWEL));
        }
        if self.mono() {
            self.mono_on(note, velocity);
            return;
        }
        // The same key still on: it rings on in its release.
        self.release_key(note);
        let limit = (self.v(VOICES).round() as usize).clamp(1, MAX_VOICES);
        let st = self.slots.start(note, limit);
        if let Some(s) = st.stolen {
            self.fade_out(s);
        }
        self.start_voice(st.voice, note, velocity);
    }

    /// Mono and Legato: one voice, in slot 0.
    fn mono_on(&mut self, note: u8, velocity: u8) {
        let held = !self.stack.is_empty();
        self.stack.push(note);
        let target = self.key_pitch(note);
        if self.slots.slots[0].phase == Phase::Free || self.slots.slots[0].phase == Phase::Fade {
            self.slots.slots[0].phase = Phase::On;
            self.slots.slots[0].note = note;
            let was = self.v[0].pitch;
            self.start_voice(0, note, velocity);
            if self.v(GLIDE) > 0.0 && held {
                self.v[0].pitch = was;
            }
            return;
        }
        self.slots.slots[0].phase = Phase::On;
        self.slots.slots[0].note = note;
        let legato = self.v(MODE) >= LEGATO - 0.5;
        let snap = self.v(GLIDE) <= 0.0;
        let ex = self.exprs.on(note);
        let v = &mut self.v[0];
        v.note = note;
        v.target = target;
        v.ex.retarget(ex);
        if snap {
            v.pitch = target;
        }
        if !(legato && held) {
            v.env.gate_on();
            v.press.start();
            v.age = 0;
            self.set_velocity(0, velocity);
        }
    }

    /// Every voice on `note` that is on goes to its release.
    fn release_key(&mut self, note: u8) {
        for i in 0..SLOTS {
            let s = self.slots.slots[i];
            if s.phase == Phase::On && s.note == note {
                self.slots.slots[i].phase = Phase::Release;
                self.v[i].env.gate_off();
                self.v[i].press.release();
            }
        }
    }

    /// The note ends: its key is up and no pedal holds it.
    fn note_off(&mut self, note: u8) {
        let note = note & 0x7F;
        self.exprs.off(note);
        if self.mono() {
            self.stack.remove(note);
            if self.slots.slots[0].phase != Phase::On || self.v[0].note != note {
                return;
            }
            if let Some(back) = self.stack.pick(Priority::Last) {
                // Back to a key still held, without starting again.
                let target = self.key_pitch(back);
                self.slots.slots[0].note = back;
                let snap = self.v(GLIDE) <= 0.0;
                let ex = self.exprs.on(back);
                let v = &mut self.v[0];
                v.note = back;
                v.target = target;
                v.ex.retarget(ex);
                if snap {
                    v.pitch = target;
                }
            } else {
                self.release_key(note);
            }
            return;
        }
        self.release_key(note);
    }

    fn release_all(&mut self) {
        self.stack.clear();
        for i in 0..SLOTS {
            if self.slots.slots[i].phase == Phase::On {
                self.slots.slots[i].phase = Phase::Release;
                self.v[i].env.gate_off();
                self.v[i].press.release();
            }
        }
        self.pedals.clear_notes();
        self.exprs.clear();
    }

    fn control_event(&mut self, ctl: Ctl, value: f32) {
        let Some(value) = Ctl::clamp(value) else {
            return;
        };
        match ctl {
            Ctl::Sustain | Ctl::Sostenuto => {
                let mut ends = [0u8; 128];
                let mut n = 0;
                self.pedals.set(ctl, value, |note| {
                    ends[n] = note;
                    n += 1;
                });
                for &note in &ends[..n] {
                    self.note_off(note);
                }
            }
            Ctl::Soft => {
                self.soft = Ctl::down(value);
                self.update_level();
            }
            Ctl::ModWheel => self.wheel = value,
            Ctl::Breath | Ctl::Foot => {
                self.expression = value * value;
                self.update_level();
            }
        }
    }

    fn apply(&mut self, ev: &NodeEvent) {
        match *ev {
            NodeEvent::NoteOn { note, velocity } if velocity > 0 => self.note_on(note, velocity),
            NodeEvent::NoteOn { note, .. } | NodeEvent::NoteOff { note } => {
                if self.pedals.lift(note & 0x7F) {
                    self.note_off(note);
                }
            }
            NodeEvent::AllOff => self.release_all(),
            NodeEvent::Param { id, value } => self.set_param(id, value),
            NodeEvent::Bend { value } => {
                if value.is_finite() {
                    self.bend = value.clamp(-1.0, 1.0) * self.v(BEND_RANGE);
                }
            }
            NodeEvent::Pressure { note, value } => {
                let n = note & 0x7F;
                self.pressure[usize::from(n)] = value.min(127);
                for i in 0..SLOTS {
                    let s = self.slots.slots[i];
                    if s.sounding() && s.note == n {
                        self.v[i].press.set(value);
                    }
                }
            }
            NodeEvent::Expression { note, kind, value } => {
                let Some(value) = self.exprs.set(note, kind, value) else {
                    return;
                };
                let n = note & 0x7F;
                for i in 0..SLOTS {
                    let s = self.slots.slots[i];
                    if s.phase == Phase::On && s.note == n {
                        self.v[i].ex.aim(kind, value);
                    }
                }
            }
            NodeEvent::Control { ctl, value } => self.control_event(ctl, value),
        }
    }

    /// One control step at absolute frame `frame`: every voice's swell,
    /// singers, vowel and gains for the step.
    fn control(&mut self) {
        let sps = self.steps_per_sec();
        let inv = 1.0 / CTRL as f32;
        let n_singers = self.singers();
        // The shared vowel swing.
        let rate = self.v(MORPH_RATE);
        self.morph_phase = (self.morph_phase + rate / sps).fract();
        let morph = self.v(MORPH) * 0.5 * sin_turns(self.morph_phase);
        self.vowel.target = self.v(VOWEL);
        let base_vowel = self.vowel.next();
        let start = self.v(START);
        let start_steps = (self.v(START_TIME) * 0.001 * sps).max(1.0);
        let section = self.v(SECTION);
        // The shaped noise is `c fs / (2 pi f)` of the white noise above
        // the integrator's corner, and a formant's lift is `2 pi f / 48k`:
        // so at every formant it is `c fs / 48k` of the white noise, whose
        // power per hertz goes as `1 / fs`. This brings it to the white
        // noise's level and its power per hertz to 48 kHz's.
        let breath = self.v(BREATH) / 100.0 * BREATH_FULL * (REF_RATE / self.rate as f32).sqrt()
            / self.noise_c;
        let tilt_base =
            (self.v(BRIGHTNESS) - 50.0) / 50.0 * SHELF_DB + if self.soft { SOFT_DB } else { 0.0 };
        let press_swell = self.v(PRESS_SWELL) / 100.0;
        let press_vowel = self.v(PRESS_VOWEL) / 100.0 * PRESS_VOWEL_SPAN;
        let vib_cents = self.v(VIBRATO);
        let vib_rate = self.v(VIB_RATE) / sps;
        let vib_delay = self.v(VIB_DELAY) * 0.001 * sps;
        let vib_fade = (VIB_FADE_MS * 0.001 * sps).max(1.0);
        let wheel = self.wheel * WHEEL_CENTS;
        let drift = self.v(DRIFT) / 100.0;
        let (fa, fs) = smoother(FLUTTER_HZ, sps);
        let (wa, ws) = smoother(WANDER_HZ, sps);
        let detune = self.v(DETUNE);
        let singer_a = (1.0 - (-1.0 / (SINGER_MS * 0.001 * sps)).exp()).min(1.0);
        let rate_f = self.rate as f32;
        let top = 0.45 * rate_f;
        let coefs = self.press_coefs;
        let expr_c = self.expr_c;
        let mut run = 0usize;
        for i in 0..SLOTS {
            let slot = self.slots.slots[i];
            let (r, l) = (i / LANES, i % LANES);
            if slot.phase == Phase::Free {
                continue;
            }
            // The swell, and the end of a voice.
            let level = self.v[i].env.next();
            if slot.phase == Phase::Fade || self.v[i].fade < 1.0 {
                let v = &mut self.v[i];
                v.fade = (v.fade - v.fade_step).max(0.0);
            }
            let ended = self.v[i].env.idle() || self.v[i].fade <= 0.0;
            if ended {
                self.slots.free(i);
                let row = &mut self.rows[r];
                row.dgain[l] = -row.gain[l] * inv;
                continue;
            }
            if self.mono() && i == 0 {
                let (mut p, t) = (self.v[0].pitch, self.v[0].target);
                self.glide.step(&mut p, t);
                self.v[0].pitch = p;
            }
            self.v[i].ex.step(expr_c);
            let hz = self.note_hz(&self.v[i]);
            let pitch = self.sounding(&self.v[i]);
            let p = self.v[i].press.step(&coefs);
            // The singers.
            let key_gain = (KEY_REF_HZ / hz.max(20.0)).sqrt().clamp(0.25, 4.0);
            let norm = key_gain / (n_singers as f32).sqrt();
            let v = &mut self.v[i];
            v.age = v.age.saturating_add(1);
            let vib_on = ((v.age as f32 - vib_delay) / vib_fade).clamp(0.0, 1.0);
            let row = &mut self.rows[r];
            for (s, singer) in v.singers.iter_mut().enumerate() {
                let wanted = if s < n_singers { norm } else { 0.0 };
                let to = if singer.wait > 0 {
                    singer.wait -= 1;
                    0.0
                } else {
                    wanted
                };
                let g = singer.gain + (to - singer.gain) * singer_a;
                let g = if (to - g).abs() < 1e-6 { to } else { g };
                row.sgain[s][l] = singer.gain;
                row.dsgain[s][l] = (g - singer.gain) * inv;
                singer.gain = g;
                if g == 0.0 && to == 0.0 && s >= n_singers {
                    continue;
                }
                run = run.max(s + 1);
                // Its pitch: its place in the spread, vibrato and drift.
                singer.flutter += (v.rng.bipolar() * 1.7320508 * fs - singer.flutter) * fa;
                singer.wander += (v.rng.bipolar() * 1.7320508 * ws - singer.wander) * wa;
                let vib =
                    (vib_cents * singer.vib_depth * vib_on + wheel) * sin_turns(singer.vib_phase);
                singer.vib_phase = (singer.vib_phase + vib_rate * singer.vib_rate).fract();
                let cents = singer.place * detune
                    + vib
                    + drift * (FLUTTER_CENTS * singer.flutter + WANDER_CENTS * singer.wander);
                let f = hz * exp2_small(cents / 1200.0);
                if f < top {
                    let d = f / rate_f;
                    row.inc[s][l] = (d * TURN) as u32;
                    row.dt[s][l] = d;
                } else {
                    row.inc[s][l] = 0;
                    row.dt[s][l] = 0.0;
                    row.sgain[s][l] = 0.0;
                    row.dsgain[s][l] = 0.0;
                }
            }
            // The vowel.
            let opening = 1.0 - (v.age as f32 / start_steps).min(1.0);
            let vowel = (base_vowel
                + morph
                + start * opening * opening
                + press_vowel * p
                + v.ex.tone * PRESS_VOWEL_SPAN)
                .clamp(0.0, 100.0);
            let key = [section, vowel, pitch, hz];
            let moved = (key[1] - v.key[1]).abs() > 0.02
                || key[0] != v.key[0]
                || (key[2] - v.key[2]).abs() > 0.01;
            if moved || v.key[0].is_nan() {
                let sung = self.table.sung(section, vowel, pitch, hz);
                let f = sung.formants;
                v.key = key;
                v.trim = db_gain(sung.trim_db);
                v.breath = db_gain(sung.breath_db - sung.trim_db);
                v.high = sung.high;
                row.shelf[l] = 1.0 - (-std::f32::consts::TAU * tilt_hz(hz) / rate_f).exp();
                for (k, &(fc, db, bw)) in f.iter().enumerate() {
                    let fc = fc.min(top);
                    let g = (std::f32::consts::PI * fc / rate_f).tan();
                    let kk = (bw / fc).min(2.0);
                    let a1 = 1.0 / (1.0 + g * (g + kk));
                    row.f.a1[k][l] = a1;
                    row.f.a2[k][l] = g * a1;
                    row.f.a3[k][l] = g * g * a1;
                    row.f.g[k][l] = SIGNS[k] * db_gain(db) * kk * lift(fc);
                }
            }
            // The tilt.
            let swell_db = -PRESS_SWELL_DB * press_swell * (1.0 - p);
            let tilt_db = tilt_base + v.vel_tilt + 0.5 * swell_db;
            let tilt_to = db_gain(tilt_db);
            if v.fresh {
                v.fresh = false;
                row.tilt[l] = tilt_to;
            }
            row.dtilt[l] = (tilt_to - row.tilt[l]) * inv;
            // Breath and the level, the tilt's change of loudness made up.
            // The voice's trim is for its singers' harmonics; the breath
            // has its own, so it is as loud at every vowel and key.
            row.dnoise[l] = (breath * v.breath - row.noise[l]) * inv;
            let comp = 1.0 / ((1.0 - v.high) + v.high * tilt_to * tilt_to).sqrt();
            let gain = level * v.vel_gain * v.trim * v.fade * db_gain(swell_db) * comp;
            self.slots.slots[i].level = level * v.vel_gain;
            row.dgain[l] = (gain - row.gain[l]) * inv;
        }
        self.singers_run = run;
    }

    /// Renders the first `n` frames of `mono` (one control step at most).
    fn render(&mut self, n: usize, mono: &mut [f32; STEP]) {
        let run = self.singers_run;
        let noise_c = self.noise_c;
        let mut out = [0.0f32; STEP];
        let mut src = [[0.0f32; LANES]; STEP];
        let mut ys = [[0.0f32; LANES]; STEP];
        for row in self.rows.iter_mut() {
            if !row.live() {
                continue;
            }
            for e in src.iter_mut().take(n) {
                // The singers, summed.
                let mut x = [0.0f32; LANES];
                for s in 0..run {
                    for l in 0..LANES {
                        let p = row.phase[s][l];
                        let t = (p >> 8) as f32 * INV_24;
                        row.phase[s][l] = p.wrapping_add(row.inc[s][l]);
                        let saw = t + t - 1.0 - poly_blep4(t, row.dt[s][l]);
                        x[l] += saw * row.sgain[s][l];
                        row.sgain[s][l] += row.dsgain[s][l];
                    }
                }
                for l in 0..LANES {
                    e[l] = x[l];
                    // Breath, falling as the singers' harmonics do, louder
                    // in the open part of the first singer's cycle.
                    if row.noise[l] != 0.0 || row.dnoise[l] != 0.0 {
                        let mut r = row.rng[l];
                        r ^= r << 13;
                        r ^= r >> 17;
                        r ^= r << 5;
                        row.rng[l] = r;
                        let w = (r as i32) as f32 * (1.0 / 2_147_483_648.0);
                        // Louder while the first singer's cycle is open,
                        // before the shaping: gated after it, the shaped
                        // noise's strong lows would be moved up to the
                        // note's harmonics.
                        let t = (row.phase[0][l] >> 8) as f32 * INV_24;
                        let open = if t >= OPEN_FROM { 1.0 } else { CLOSED_BREATH };
                        row.nlp[l] += noise_c * (w * open - row.nlp[l]);
                        e[l] += row.nlp[l] * row.noise[l];
                        row.noise[l] += row.dnoise[l];
                    }
                    // The tilt: the low part as it is, the part above
                    // the turn scaled.
                    row.lp[l] += row.shelf[l] * (e[l] - row.lp[l]);
                    e[l] = row.lp[l] + row.tilt[l] * (e[l] - row.lp[l]);
                    row.tilt[l] += row.dtilt[l];
                }
            }
            // The five formants, side by side across the lanes.
            formants4(&src[..n], &mut ys[..n], &mut row.f);
            for (o, y) in out.iter_mut().zip(&ys).take(n) {
                let mut sum = 0.0;
                for l in 0..LANES {
                    sum += y[l] * row.gain[l];
                    row.gain[l] += row.dgain[l];
                }
                *o += sum;
            }
        }
        for (o, x) in mono.iter_mut().zip(&out).take(n) {
            *o = x * self.level.next();
        }
    }

    /// At the end of a control step: the lanes of free slots (whose level
    /// has just ramped to 0) back to exact zeros, so a row with one voice
    /// left runs its other lanes on silence and a row with none stops.
    fn settle_free_lanes(&mut self) {
        for i in 0..SLOTS {
            if self.slots.slots[i].phase == Phase::Free {
                self.rows[i / LANES].clear_lane(i % LANES);
            }
        }
    }

    /// Adds frames `at..next` into `left` and `right`: a control step at a
    /// time, rendered, through the ensemble, then added.
    fn segment(&mut self, frame: u64, at: usize, next: usize, left: &mut [f32], right: &mut [f32]) {
        let mut i = at;
        while i < next {
            let abs = frame + i as u64;
            if abs.is_multiple_of(CTRL) {
                self.control();
            }
            let end = next.min(i + (CTRL - abs % CTRL) as usize);
            let n = end - i;
            let live = self.rows.iter().any(Row::live);
            if !live {
                if self.quiet as f32 >= QUIET_MS * 0.001 * self.rate as f32 {
                    i = end;
                    continue;
                }
                self.quiet = self.quiet.saturating_add(n as u32);
            } else {
                self.quiet = 0;
            }
            let mut l = [0.0f32; STEP];
            let mut r = [0.0f32; STEP];
            self.render(n, &mut l);
            if (frame + end as u64).is_multiple_of(CTRL) {
                self.settle_free_lanes();
            }
            self.ensemble.process_mono(&mut l[..n], &mut r[..n]);
            for (o, x) in left[i..end].iter_mut().zip(&l) {
                *o += x;
            }
            for (o, x) in right[i..end].iter_mut().zip(&r) {
                *o += x;
            }
            i = end;
        }
    }
}

impl Node for Choir {
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
        let was_mono = self.mono();
        self.values[id as usize] = p.clamp(value);
        match id {
            LEVEL => self.update_level(),
            ATTACK | DECAY | SUSTAIN | RELEASE => self.update_env(),
            ENSEMBLE | LINES | SLOW_RATE | FAST_RATE | DEPTH | WIDTH => self.update_ensemble(),
            GLIDE => self.update_glide(),
            SINGERS => {
                for i in 0..SLOTS {
                    if self.slots.slots[i].phase != Phase::Free {
                        self.set_places(i);
                    }
                }
            }
            MODE if self.mono() != was_mono => self.release_all(),
            _ => {}
        }
    }

    fn param(&self, id: u16) -> f32 {
        if id == METER_NOTES {
            return self.active_voices() as f32;
        }
        match PARAMS.iter().find(|p| p.id == id) {
            Some(_) => self.values[id as usize],
            None => f32::NAN,
        }
    }

    fn prepare(&mut self, rate: u32, max_block: usize) {
        self.rate = rate;
        self.ensemble.prepare(rate, max_block.max(1));
        self.level.set_time(rate, 10.0);
        self.vowel.set_time((rate as u64 / CTRL) as u32, VOWEL_MS);
        self.press_coefs = PressureShape::default().coefs(self.steps_per_sec());
        self.noise_c = 1.0 - (-std::f32::consts::TAU * NOISE_HZ / rate as f32).exp();
        self.expr_c = expr_coef(self.steps_per_sec());
        self.reset();
    }

    fn reset(&mut self) {
        self.slots.clear();
        self.v = [Voice::default(); SLOTS];
        self.rows = [Row::default(); ROWS];
        self.stack.clear();
        self.pedals.reset();
        self.pressure = [0; 128];
        self.exprs.clear();
        self.bend = 0.0;
        self.wheel = 0.0;
        self.soft = false;
        self.expression = 1.0;
        self.morph_phase = 0.0;
        self.singers_run = 0;
        self.seed = 0xC401;
        self.now = 0;
        self.quiet = u32::MAX;
        self.update_env();
        self.update_glide();
        self.update_level();
        self.level.snap(self.level.target);
        self.vowel.snap(self.v(VOWEL));
        self.update_ensemble();
        self.ensemble.reset();
    }

    fn process(&mut self, ctx: &Ctx, events: &[Timed], left: &mut [f32], right: &mut [f32]) {
        if ctx.rate != self.rate {
            self.prepare(ctx.rate, left.len());
        }
        let len = left.len().min(right.len());
        let mut at = 0usize;
        let mut i = 0usize;
        while at < len {
            self.now = ctx.frame + at as u64;
            while i < events.len() && events[i].offset as usize <= at {
                let ev = events[i].ev;
                self.apply(&ev);
                i += 1;
            }
            let next = events.get(i).map_or(len, |e| (e.offset as usize).min(len));
            if next > at {
                self.segment(ctx.frame, at, next, left, right);
            }
            at = next;
        }
        self.now = ctx.frame + len as u64;
        for e in &events[i..] {
            let ev = e.ev;
            self.apply(&ev);
        }
    }

    fn tail_frames(&self) -> u32 {
        // The release reaches silence at about 1.35 times its time; then
        // the formants ring and the ensemble's lines.
        (self.rate as f32 * (self.v(RELEASE) * 0.0014 + 0.1)) as u32
    }

    fn set_tuning(&mut self, tuning: &Tuning) {
        if self.tuning == *tuning {
            return;
        }
        self.tuning.clone_from(tuning);
    }

    fn hears(&self, ctl: Ctl) -> bool {
        matches!(ctl, Ctl::Sustain | Ctl::Sostenuto)
    }

    fn sounding_voices(&self) -> usize {
        self.slots.busy()
    }

    fn cull_candidate(&self) -> Option<Cull> {
        let i = self.slots.victim()?;
        Some(Cull {
            level: self.slots.slots[i].level,
            releasing: self.slots.slots[i].phase == Phase::Release,
        })
    }

    fn cull(&mut self) -> bool {
        let Some(i) = self.slots.victim() else {
            return false;
        };
        self.slots.slots[i].phase = Phase::Fade;
        self.fade_out(i);
        true
    }
}
