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

use mpfree_node::{Ctx, Cull, Kind, Node, NodeEvent, NodeInfo, ParamInfo, Smooth, Timed, Tuning};

use crate::dsp::{drive_n, drive_norm, Filter, FilterCoefs, FilterMode, Rng};
use crate::expr::{expr_coef, NoteExprs, VoiceExpr};
use crate::keys_params::*;
use crate::params::Preset;
use crate::voice::velocity_gain;

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
/// The clav string's delay line, samples: enough for C1 with a 6-semitone
/// release drop at 48 kHz, or 3 at 96 kHz (lower, the note stops falling).
pub const STRING_LEN: usize = 4096;
const STRING_MASK: usize = STRING_LEN - 1;
/// Where the clav's two pickups sit along the string, as a share of its
/// length from the end the tangent strikes near: each makes a comb whose
/// notches fall on every `1 / share`-th partial. The neck pickup's fifth
/// is as measured on A-sharp 2 (Gabrielli et al. 2013); the bridge's
/// fourteenth is our reading.
pub const PICKUP_AT: [f32; 2] = [0.2, 1.0 / 14.0];
/// The clav pickup's curve, off centre: a hard hit adds even harmonics.
const CLAV_NL: f32 = 0.2;
/// A clav hit's first swing across the pickup's curve per unit of Bark's
/// drive and strike: measured (see [`clav_swing`]).
const CLAV_SWING: f32 = 3.07;
/// The clav's displacement integrator leaks this fast, Hz, so it never
/// drifts.
const CLAV_LEAK_HZ: f32 = 2.0;
/// How long the top of a clav note's band rings against its fundamental:
/// a string loses its high partials far sooner.
const CLAV_TOP_DECAY: f32 = 0.06;
/// Where the clav's mute starts to bite, Hz.
const MUTE_HZ: f32 = 1_000.0;
/// How much loudness the full mute takes from the low and middle notes.
const MUTE_LOSS_DB: f32 = 16.0;
/// How long the release drop takes, ms: the yarn meets the string at once.
const DROP_MS: f32 = 3.0;
/// The clav's level against the tine's, so the models sit together.
const CLAV_GAIN: f32 = 0.39;
/// The clav preamp's headroom on the bus: it passes the sound unchanged
/// up to the knee (8 dB under full scale at Level 0 dB) and rounds what
/// goes past it toward the ceiling (2 dB under).
const CLAV_KNEE: f32 = 0.4 / VOICE_GAIN;
const CLAV_CEIL: f32 = 0.8 / VOICE_GAIN;
/// The Reed model's index (`REED` here is its modes).
const REED_MODEL: f32 = crate::keys_params::REED;

/// The clav tone switch's high-pass and low-pass corners, Hz, for Soft,
/// Medium, Treble and Brilliant (our reading of the switches' networks:
/// Soft keeps the body and takes the top, Brilliant the reverse).
pub const CLAV_TONE_HZ: [(f32, f32); 4] = [
    (30.0, 2_000.0),
    (50.0, 5_000.0),
    (150.0, 9_000.0),
    (400.0, 14_000.0),
];

/// A pickup's comb at partial `k` for a pickup `at` (share of the
/// string) along it: `|1 - e^(-2 pi i k at)|`, 0 on every `1 / at`-th
/// partial.
pub fn comb_gain(at: f32, k: f32) -> f32 {
    2.0 * (std::f32::consts::PI * k * at).sin().abs()
}

/// The most partials [`clav_voicing`] counts.
pub const CLAV_PARTIALS: usize = 128;
/// A clav note's partials reach about this high, Hz (the tangent's
/// width): [`clav_voicing`] evens the switches over those under it.
const CLAV_TOP_HZ: f32 = 6_000.0;
/// The phase rotator's corners on the clav bus, Hz: first-order all-pass
/// stages in pairs.
const ROTATOR_HZ: [f32; 6] = [100.0, 100.0, 300.0, 300.0, 900.0, 900.0];

/// The two combs together with weights `w`, at partial `k`.
fn combs(w: [f32; 2], k: f32) -> f32 {
    // Each comb `1 - e^(-i theta)`: its real and imaginary parts.
    let (mut re, mut im) = (0.0f32, 0.0f32);
    for (at, w) in PICKUP_AT.iter().zip(w) {
        let th = std::f32::consts::TAU * k * at;
        re += w * (1.0 - th.cos());
        im += w * th.sin();
    }
    (re * re + im * im).sqrt()
}

/// The two pickups' weights for **Pickup** `pickup`: one, the other, both
/// alike, or one less the other.
pub fn pickup_mix(pickup: f32) -> [f32; 2] {
    match pickup.round() as i32 {
        0 => [1.0, 0.0],
        1 => [0.0, 1.0],
        2 => [0.5, 0.5],
        _ => [0.5, -0.5],
    }
}

/// The level the pickups give partial `k` (1 the fundamental) of a clav
/// note for **Pickup** `pickup`, before the pickup's curve and
/// [`clav_voicing`]: for the Synth tab's picture and the tests.
pub fn pickup_response(pickup: f32, k: f32) -> f32 {
    combs(pickup_mix(pickup), k)
}

/// The clav tone switch's gain at `hz` for **Clav tone** `tone`: its
/// high-pass and low-pass, two-pole Butterworth each.
pub fn clav_tone_gain(tone: f32, hz: f32) -> f32 {
    let (hp, lp) = CLAV_TONE_HZ[(tone.round().max(0.0) as usize).min(3)];
    let h = (hz / hp).powi(4);
    let l = (hz / lp).powi(4);
    (h / ((1.0 + h) * (1.0 + l))).sqrt()
}

/// How loud the ear finds a partial at `hz` against one at 1 kHz, in
/// power: the loudness meter's weighting (BS.1770's high shelf, 4 dB up
/// over 2 kHz), near enough.
fn ear_weight(hz: f32) -> f32 {
    let f2 = hz * hz;
    1.0 + 1.5 * f2 / (f2 + 1_700.0 * 1_700.0)
}

/// The gain that keeps a clav note at `hz` about as loud whatever the
/// Pickup and Clav tone switches: one over their gain across the partials
/// the note has (those under [`CLAV_TOP_HZ`]), weighted as the ear hears
/// them. So a switch changes the colour, not the level. At most 12 dB.
pub fn clav_voicing(pickup: f32, tone: f32, hz: f32) -> f32 {
    let w = pickup_mix(pickup);
    voicing_from(|k| combs(w, k as f32).powi(2), tone, hz)
}

/// [`clav_voicing`] from the pickups' power at each partial (1 the
/// fundamental).
#[inline]
fn voicing_from(comb2: impl Fn(usize) -> f32, tone: f32, hz: f32) -> f32 {
    let n = ((CLAV_TOP_HZ / hz.max(1.0)) as usize).clamp(1, CLAV_PARTIALS);
    let (mut heard, mut flat) = (0.0f32, 0.0f32);
    for k in 1..=n {
        let f = k as f32 * hz;
        let e = ear_weight(f);
        heard += e * comb2(k) * clav_tone_gain(tone, f).powi(2);
        flat += e;
    }
    (heard / flat).sqrt().max(0.25).recip()
}

/// The clav tangent's contact in seconds, from Hardness (0 to 100) and the
/// velocity (0 to 1): shorter than a hammer's (a metal tangent on a
/// string), longer for a soft hit; never more than a third of the note's
/// period.
pub fn tangent_secs(hardness: f32, vel: f32, hz: f32) -> f32 {
    let soft = 1.0 - (hardness / 100.0).clamp(0.0, 1.0);
    let full = 0.000_08 + 0.000_8 * soft * soft;
    (full * vel.max(0.08).powf(-0.47))
        .min(MAX_CONTACT)
        .min(0.33 / hz.max(1.0))
}

/// The clav tangent's contact as struck: [`tangent_secs`], longer by up
/// to four times under **Mute** `mute` (0 to 100; the mute's felt is on
/// the string as the tangent lands), never more than a third of the
/// note's period.
pub fn clav_contact(hardness: f32, mute: f32, vel: f32, hz: f32) -> f32 {
    (tangent_secs(hardness, vel, hz) * (1.0 + 3.0 * mute / 100.0)).min(0.33 / hz.max(1.0))
}

/// How long a held clav note at `hz` rings, seconds to fall 60 dB:
/// **Decay**'s law ([`decay_for`]), shortened by **Mute** `mute` (0 to
/// 100) to 15 % at full.
pub fn clav_decay(decay_ms: f32, mute: f32, hz: f32) -> f32 {
    (decay_for(decay_ms, hz) * (1.0 - 0.85 * mute / 100.0)).max(0.02)
}

/// How long partial `k` (1 the fundamental) of a held clav note at `hz`
/// rings at `rate`, seconds to fall 60 dB, for **Decay** `decay_ms` and
/// **Mute** `mute` (0 to 100): the string loop's own losses, as the engine
/// sets them.
pub fn clav_ring(decay_ms: f32, mute: f32, hz: f32, k: f32, rate: f32) -> f32 {
    let w0 = std::f32::consts::TAU * hz / rate;
    let trip = 1.0 / hz;
    let w_mute = std::f32::consts::TAU * MUTE_HZ / rate;
    let t60 = clav_decay(decay_ms, mute, hz);
    let (gd, a) = clav_loop(w0, trip, t60, mute / 100.0, w_mute);
    let w = (k * w0).min(std::f32::consts::PI);
    let (re, im) = (1.0 - a * w.cos(), a * w.sin());
    let g = (gd * (1.0 - a) / (re * re + im * im).sqrt()).clamp(1e-9, 0.999_999);
    -3.0 * trip / g.log10()
}

/// The clav pickup's curve, from where the string sits over it to what it
/// hears: off centre, so a hard hit adds even harmonics.
pub fn clav_curve(u: f32) -> f32 {
    u - CLAV_NL * u * u
}

/// How far a clav hit's first swing reaches across [`clav_curve`], for
/// **Bark** `bark` (0 to 100), **Pickup** `pickup` and the velocity `vel`
/// (0 to 1): the same over either pickup or both, half Out of phase (what
/// the two share cancels). Measured on middle C, within 2 %; the top
/// octaves swing less (their tangent is narrower than its push).
pub fn clav_swing(bark: f32, pickup: f32, vel: f32) -> f32 {
    let oop = if pickup.round() >= OUT_OF_PHASE {
        0.5
    } else {
        1.0
    };
    CLAV_SWING * (0.5 + (bark / 100.0).powi(2)) * strike_amp(vel) * oop
}

/// How strong a clav note's fundamental is against middle C's at `hz`:
/// lower notes carry more harmonics under the tangent's width, so their
/// fundamental is weaker (as `sqrt(hz)`) for about the same loudness.
pub fn clav_register(hz: f32) -> f32 {
    (hz / 261.63).sqrt().clamp(0.25, 4.0)
}

/// The clav's preamp: unchanged under [`CLAV_KNEE`], then rounded toward
/// [`CLAV_CEIL`]. The string's sharpest peaks, when a hard chord lines
/// them up, are all it touches.
#[inline]
pub fn clav_preamp(x: f32) -> f32 {
    let a = x.abs();
    if a <= CLAV_KNEE {
        x
    } else {
        let room = CLAV_CEIL - CLAV_KNEE;
        (CLAV_KNEE + room * ((a - CLAV_KNEE) / room).tanh()).copysign(x)
    }
}

/// The four Lagrange weights reading a delay line at fractional delay
/// `1 + f` from its first tap (`f` 0 to 1).
#[inline]
fn lagrange(f: f32) -> [f32; 4] {
    let (a, b, c) = (f + 1.0, f - 1.0, f - 2.0);
    [
        -f * b * c / 6.0,
        a * b * c / 2.0,
        -a * f * c / 2.0,
        a * f * b / 6.0,
    ]
}

/// The clav string loop's losses for a note at `w0` (radians a sample),
/// `trip` seconds round the loop, ringing for `t60` seconds with **Mute**
/// `mute` (0 to 1): a one-pole low-pass, its gain at DC and its pole.
/// Its gain at the note gives the decay; at the top of the band the top
/// dies in [`CLAV_TOP_DECAY`] of the time. The mute's felt damps what is
/// over about [`MUTE_HZ`] (`w_mute` radians a sample; for a note above it,
/// its octave): that dies in 0.02^Mute of the note's time.
fn clav_loop(w0: f32, trip: f32, t60: f32, mute: f32, w_mute: f32) -> (f32, f32) {
    let g_for = |t: f32| 10f32.powf(-3.0 * trip / t);
    let g0 = g_for(t60);
    let gn = g_for(t60 * CLAV_TOP_DECAY);
    let c = w0.cos();
    let shape = |a: f32| {
        let (re, im) = (1.0 - a * c, a * w0.sin());
        (1.0 - a) / (re * re + im * im).sqrt()
    };
    // The most it may roll off: what is under the note (DC most) must
    // still die within four times the note's decay, so its gain at DC is
    // at most g0^(1/4) and at the note g0^(3/4) of that.
    let s2 = g0.powf(1.5);
    let (qa, qb) = (1.0 - s2, 1.0 - s2 * c);
    let a_max = qa / (qb + (qb * qb - qa * qa).max(0.0).sqrt());
    // The mute's roll-off: with u = 2(1 - cos w), the one-pole's power is
    // q / (q + u) for q = (1 - a)^2 / a, so the ratio r between the eighth
    // partial (or the top of the band) and the note fixes q.
    let a_mute = if mute > 0.0 {
        let r = 10f32.powf(-3.0 * trip / t60 * (0.02f32.powf(-mute) - 1.0));
        let wm = w_mute.max(2.0 * w0).min(2.5);
        let (u0, u8) = (2.0 * (1.0 - c), 2.0 * (1.0 - wm.cos()));
        let r2 = r * r;
        let q = ((r2 * u8 - u0) / (1.0 - r2).max(1e-9)).max(1e-9);
        let b = 2.0 + q;
        (b - (b * b - 4.0).max(0.0).sqrt()) * 0.5
    } else {
        0.0
    };
    // Its gain at DC, so that at the note it is g0: a few steps.
    let pick = |gd: f32| (((gd - gn) / (gd + gn)).max(a_mute)).clamp(0.0, a_max);
    let mut gd = g0;
    for _ in 0..3 {
        gd = (g0 / shape(pick(gd))).min(0.999_95);
    }
    (gd, pick(gd))
}

/// The make-up gain for what the mute takes from a clav note at `hz`,
/// ahead of the preamp, so a muted clav plays about as loud as an open
/// one (its attack, still bright, is what the preamp rounds). The felt
/// bites over [`MUTE_HZ`]: it dulls the low and middle notes' many
/// partials (about `MUTE_LOSS_DB Mute^0.85` of their loudness, measured)
/// but leaves a top note's few, so the make-up fades from an octave under
/// it to none an octave over.
pub fn mute_makeup(mute: f32, hz: f32) -> f32 {
    if mute <= 0.0 {
        return 1.0;
    }
    let lost = MUTE_LOSS_DB * mute.powf(0.85);
    let share = ((hz / (0.5 * MUTE_HZ)).log2() * 0.5).clamp(0.0, 1.0);
    10f32.powf(lost * (1.0 - share) / 20.0)
}

/// One frame of a clav voice: the tangent's push `x` into the string's
/// loop, the two pickups' combs, the displacement they see and the
/// pickup's curve. Returns the pickup's output and the displacement (for
/// the level follower).
/// The tangent's or hammer's push on a voice this sample, while it lasts:
/// a half sine.
#[inline(always)]
fn strike(v: &mut Voice) -> f32 {
    if v.pulse_at < v.pulse_len {
        let t = (v.pulse_at as f32 + 0.5) / v.pulse_len as f32;
        v.pulse_at += 1;
        v.pulse_amp * (std::f32::consts::PI * t).sin()
    } else {
        0.0
    }
}

/// A voice's sample `y` (its motion `d`) onto its bus `out`, at its gain
/// and through a steal's fade; follows its level and frees it once
/// silent. True when it is free, so the rest of the stretch is skipped.
#[inline(always)]
fn voice_out(v: &mut Voice, y: f32, d: f32, out: &mut f32) -> bool {
    let mut y = y * v.gain;
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
    *out += y;
    v.state == State::Free
}

#[inline]
fn clav_tick(v: &mut Voice, string: &mut [f32; STRING_LEN], x: f32, leak: f32) -> (f32, f32) {
    let w = v.sw;
    let at = |string: &[f32; STRING_LEN], d: usize| string[w.wrapping_sub(d) & STRING_MASK];
    // The loop: a Lagrange read, the losses' low-pass, the loop gain.
    let b = v.di - 1;
    let s = v.lw[0] * at(string, b)
        + v.lw[1] * at(string, b + 1)
        + v.lw[2] * at(string, b + 2)
        + v.lw[3] * at(string, b + 3);
    v.slp = s + v.sa * (v.slp - s);
    let fb = v.sg * v.slp + x;
    string[w] = fb;
    v.sw = (w + 1) & STRING_MASK;
    // Each pickup hears the wave less its reflection: a comb.
    let mut m = 0.0;
    for k in 0..2 {
        let (d, f) = (v.pi[k], v.pf[k]);
        let (a0, a1) = (at(string, d), at(string, d + 1));
        m += v.pmix[k] * (fb - (a0 + f * (a1 - a0)));
    }
    // The wave is the string's velocity: its integral is where the string
    // sits over the pickup, which the pickup's curve hears.
    v.sx = v.sx * leak + v.sint * m;
    let c = clav_curve(v.swing * v.sx);
    let y = (c - v.last) * v.norm;
    v.last = c;
    (y, v.sx)
}

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

/// [`flux`]'s curvature.
#[inline]
fn flux_bend(u: f32) -> f32 {
    let d = 1.0 + u * u;
    (6.0 * u * u - 2.0) / (d * d * d)
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

/// Each model's voicing across the keys, dB at C1, C2 ... C7 (a straight
/// line between): what a technician does key by key so the instrument
/// plays evenly. Fitted to the factory sounds' loudness (velocity 100,
/// held 1.5 s), so each note sounds about as loud as middle C's octave.
const KEY_CURVE: [[f32; 7]; 3] = [
    [3.85, 0.25, -0.45, 0.45, 2.65, 3.55, 3.35],
    [6.65, 2.85, -0.05, 0.05, 1.45, 2.05, -0.05],
    [-3.59, -2.71, -0.9, 0.9, 3.07, 3.76, 3.64],
];

/// The voicing's gain for `model` (0 Tine, 1 Reed, 2 Clav) at `pitch`
/// (a MIDI note, fractional).
pub fn key_level(model: f32, pitch: f32) -> f32 {
    let c = &KEY_CURVE[(model.round().max(0.0) as usize).min(2)];
    let x = ((pitch - 24.0) / 12.0).clamp(0.0, 6.0);
    let i = (x as usize).min(5);
    let db = c[i] + (c[i + 1] - c[i]) * (x - i as f32);
    10f32.powf(db / 20.0)
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
    /// The output gain: the velocity's level (over the strike's own) and
    /// the key's.
    gain: f32,
    /// The clav string: the loop's read delay now (samples), where it is
    /// going and how far it moves a control step (the release drop), its
    /// whole part and Lagrange weights, the loop's gain and low-pass, and
    /// where the next sample goes.
    sd: f32,
    sd_to: f32,
    sd_rate: f32,
    di: usize,
    lw: [f32; 4],
    sg: f32,
    sa: f32,
    slp: f32,
    sw: usize,
    /// The pickups: each one's comb delay (whole and fraction) and weight.
    pi: [usize; 2],
    pf: [f32; 2],
    pmix: [f32; 2],
    /// Whether this voice is a clav string (set when it is struck, so a
    /// voice fading out as the model changes keeps its own sound), the
    /// string's displacement at the pickups (the integral of the wave the
    /// loop carries) and its scale.
    clav: bool,
    sx: f32,
    sint: f32,
    /// [`clav_voicing`] for this note.
    voicing: f32,
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
    /// The clav's strings, one per voice slot, and its tone switch (a
    /// high-pass and a low-pass on the bus).
    strings: Box<[[f32; STRING_LEN]]>,
    clav_hp: Filter,
    clav_lp: Filter,
    clav_hc: FilterCoefs,
    clav_lc: FilterCoefs,
    clav_leak: f32,
    /// Each Pickup choice's power at partials 1 to [`CLAV_PARTIALS`], for
    /// [`clav_voicing`] at a strike.
    comb2: [[f32; CLAV_PARTIALS]; 4],
    /// The reed preamp's output coupling (a high-pass): its pole and its
    /// last input and output.
    dc_r: f32,
    dc_x: f32,
    dc_y: f32,
    /// The clav bus's phase rotator: each stage's coefficient and its last
    /// input and output.
    rot_a: [f32; 6],
    rot_x: [f32; 6],
    rot_y: [f32; 6],
}

/// The reed preamp's rest point on its curve: transistors are not
/// symmetric, so it adds even harmonics as it is pushed.
const PRE_REST: f32 = 0.35;
/// The reed preamp's output coupling's corner, Hz.
const REED_COUPLING_HZ: f32 = 20.0;
/// The reed model's make-up gain, so both models sit at the same level.
const REED_GAIN: f32 = 1.9;

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
            strings: vec![[0.0; STRING_LEN]; POOL].into_boxed_slice(),
            clav_hp: Filter::default(),
            clav_lp: Filter::default(),
            clav_hc: FilterCoefs::default(),
            clav_lc: FilterCoefs::default(),
            clav_leak: 1.0,
            comb2: [0.0, 1.0, 2.0, 3.0].map(|p| {
                let w = pickup_mix(p);
                std::array::from_fn(|k| combs(w, (k + 1) as f32).powi(2))
            }),
            rot_a: [0.0; 6],
            rot_x: [0.0; 6],
            rot_y: [0.0; 6],
            dc_r: 1.0,
            dc_x: 0.0,
            dc_y: 0.0,
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
        crate::factory::presets!("keys.presets", &INFO, PARAMS)
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
        self.v(MODEL).round() == REED_MODEL
    }

    fn clav(&self) -> bool {
        self.v(MODEL).round() == CLAV
    }

    /// Whether any clav string still sounds.
    fn clav_alive(&self) -> bool {
        self.voices.iter().any(|v| v.clav && v.state != State::Free)
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
        // The pickup network's high-pass: low for a tine or the clav,
        // higher for the reed's RC load.
        let hz = if self.reed() { 120.0 } else { 30.0 };
        self.hp_a = (-std::f32::consts::TAU * hz / r).exp();
        let (hp, lp) = CLAV_TONE_HZ[(self.v(CLAV_TONE).round() as usize).min(3)];
        // Butterworth: the shared filter's damping at Resonance 0.3.
        self.clav_hc = FilterCoefs::new(hp, 0.3, r);
        self.clav_lc = FilterCoefs::new(lp, 0.3, r);
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
        if v.clav {
            self.tune_clav(v, pitch);
            return;
        }
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
            // Near the axis the slope vanishes and the curve's bend makes
            // the octave instead: the pickup's gain is the two together.
            flux_slope(rest)
                .hypot(0.5 * flux_bend(rest) * swing)
                .max(0.05)
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

    /// Sets a voice's clav string for its pitch and gate: the loop's
    /// delay (less its low-pass's own delay at the note, so it is in tune),
    /// its loss for the decay (or, let go, the release, with the drop), and
    /// the pickups.
    fn tune_clav(&self, v: &mut Voice, pitch: f32) {
        let rate = self.rate as f32;
        let held = v.state == State::On;
        let lowest = (rate / (STRING_LEN - 8) as f32).max(10.0);
        let f0 = self.tuning.hz(pitch).clamp(lowest, rate * 0.2);
        let period = rate / f0;
        let w0 = std::f32::consts::TAU * f0 / rate;
        let mute = self.v(MUTE) / 100.0;
        let t60 = if held {
            clav_decay(self.v(DECAY), self.v(MUTE), f0)
        } else {
            (self.v(RELEASE) * 0.001).max(0.004)
        };
        let trip = period / rate;
        let w_mute = std::f32::consts::TAU * MUTE_HZ / rate;
        let (gd, a) = clav_loop(w0, trip, t60, mute, w_mute);
        let (re, im) = (1.0 - a * w0.cos(), a * w0.sin());
        let d_lp = im.atan2(re) / w0;
        v.sg = gd;
        v.sa = a;
        if v.tuned.is_nan() {
            // Struck, or a switch moved (not on every bend).
            let c = &self.comb2[(self.v(PICKUP).round().max(0.0) as usize).min(3)];
            v.voicing = voicing_from(|k| c[k - 1], self.v(CLAV_TONE), f0) * mute_makeup(mute, f0);
        }
        // Let go, the yarn damps the string and its speaking part grows:
        // the note falls by Release drop.
        let drop = if held { 0.0 } else { self.v(DROP) };
        let to = (period * (drop / 12.0).exp2() - d_lp).clamp(2.0, (STRING_LEN - 4) as f32);
        if v.sd.is_nan() || held {
            v.sd = to;
            v.sd_rate = 0.0;
        } else if v.sd_rate == 0.0 && v.sd != to {
            // The release starts: the drop takes DROP_MS.
            let steps = (DROP_MS * 0.001 * rate / CTRL as f32).max(1.0);
            v.sd_rate = (to - v.sd).abs() / steps;
        }
        v.sd_to = to;
        Self::set_read(v);
        for k in 0..2 {
            let d = (PICKUP_AT[k] * period).max(1.0);
            v.pi[k] = d as usize;
            v.pf[k] = d - d.floor();
        }
        v.pmix = pickup_mix(self.v(PICKUP));
        // The pickup's curve: a hard hit (Bark) pushes it further. It
        // hears the curve's change, so its output grows with the note:
        // divided out.
        // The string's displacement grows toward the bass (see `note_on`):
        // the swing takes that out, so Bark drives every note alike.
        let swing = (0.5 + (self.v(BARK) / 100.0).powi(2)) * clav_register(f0).recip();
        if v.swing != swing {
            let u = swing * v.sx;
            v.last = clav_curve(u);
        }
        v.rest = 0.0;
        v.swing = swing;
        v.sint = w0;
        v.norm = CLAV_GAIN * v.voicing / (swing * w0);
        v.tuned = pitch;
    }

    /// The loop's read point from its delay: the whole samples and the
    /// Lagrange weights.
    fn set_read(v: &mut Voice) {
        let d = v.sd.max(2.0);
        let di = d.floor();
        v.di = di as usize;
        v.lw = lagrange(d - di);
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
                    sd: f32::NAN,
                    ..Voice::default()
                };
                free
            }
        };
        let mut v = self.voices[slot];
        let clav = self.clav();
        if v.clav != clav || (clav && same.is_none()) {
            // A fresh string (nothing left from the slot's last note), or
            // the same note struck again after the model changed.
            v = Voice {
                tuned: f32::NAN,
                rest: f32::NAN,
                sd: f32::NAN,
                ..Voice::default()
            };
            self.strings[slot].fill(0.0);
        }
        v.clav = clav;
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
        let secs = if clav {
            clav_contact(self.v(HARDNESS), self.v(MUTE), tone_vel, hz)
        } else {
            contact_secs(self.v(HARDNESS), tone_vel, hz)
        };
        let len = (secs * rate).max(2.0);
        v.pulse_len = len as u32;
        v.pulse_at = 0;
        let amp = strike_amp(vel);
        v.pulse_amp = if clav {
            // The string carries the tangent's push round its loop once a
            // period: a half-sine of area `amp * period / 2` gives a
            // fundamental of `amp`, with every harmonic as strong up to
            // the push's own width. A low note has more of them, so its
            // fundamental is brought down by the register.
            let period = rate / hz.max(1.0);
            amp * clav_register(hz) * std::f32::consts::PI * period / (4.0 * len)
        } else {
            // Unit area: the fundamental rings at the strike's level
            // whatever the hammer.
            amp * std::f32::consts::FRAC_PI_2 / len
        };
        // The level follows Velocity's range; the strike's own level only
        // sets how hard the pickup is driven.
        v.gain = velocity_gain(velocity, self.v(VELOCITY)) / amp * key_level(self.v(MODEL), pitch);
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

    /// The voice load shedding gives up first: the least audible one
    /// ringing ([`Cull::audibility`]), by its level follower (a
    /// full-velocity strike rings at about 1). Voices already fading are
    /// left be.
    fn cull_pick(&self) -> Option<(usize, Cull)> {
        let mut best = None;
        for (i, v) in self.voices.iter().enumerate() {
            let releasing = match v.state {
                State::On => false,
                State::Release => true,
                State::Free | State::Kill => continue,
            };
            // A hammer still in its contact has not rung up yet: count it
            // as loud as it will be.
            let level = if v.pulse_at < v.pulse_len {
                v.follow.max(1.0)
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
            // The clav's release drop, on its way.
            let v = &mut self.voices[i];
            if v.clav && v.sd_rate > 0.0 && v.sd != v.sd_to {
                let step = v.sd_to - v.sd;
                v.sd = if step.abs() <= v.sd_rate {
                    v.sd_to
                } else {
                    v.sd + v.sd_rate.copysign(step)
                };
                Self::set_read(v);
            }
        }
        // The bus filters' tails: flushed once inaudible, so a silent
        // Keys puts out exact zeros (and never runs on denormals). The
        // clav's tone switch rests once its strings have stopped.
        if !self.clav_alive() {
            self.clav_hp.clear();
            self.clav_lp.clear();
            self.rot_x = [0.0; 6];
            self.rot_y = [0.0; 6];
        }
        for x in [
            &mut self.dc_x,
            &mut self.dc_y,
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
        let clav_on = self.clav_alive();
        let len = to - from;
        // Each voice across the stretch in turn (at most a control block),
        // so its state stays in registers: the tines and reeds into one
        // bus, the clav strings into theirs, summed in the voices' order.
        let mut buses = [0.0f32; CTRL as usize];
        let mut clavs = [0.0f32; CTRL as usize];
        let leak = self.clav_leak;
        for (v, string) in self.voices.iter_mut().zip(self.strings.iter_mut()) {
            if v.state == State::Free {
                continue;
            }
            if v.clav {
                for o in clavs[..len].iter_mut() {
                    let x = strike(v);
                    let (y, d) = clav_tick(v, string, x, leak);
                    if voice_out(v, y, d, o) {
                        break;
                    }
                }
            } else {
                for o in buses[..len].iter_mut() {
                    let x = strike(v);
                    let mut d = 0.0;
                    for k in 0..n_modes {
                        let y = v.b0[k] * x + v.c1[k] * v.y1[k] - v.c2[k] * v.y2[k];
                        v.y2[k] = v.y1[k];
                        v.y1[k] = y;
                        d += y;
                    }
                    let u = v.rest + v.swing * d;
                    let c = if reed { capacitance(u) } else { flux(u) };
                    let y = (c - v.last) * v.norm;
                    v.last = c;
                    if voice_out(v, y, d, o) {
                        break;
                    }
                }
            }
        }
        for (j, i) in (from..to).enumerate() {
            let mut bus = buses[j];
            let clav_bus = clavs[j];
            if clav_on {
                // The clav's tone switch, then a phase rotator (as
                // broadcast processors use): it spreads the string's sharp
                // pulses in time without changing the spectrum, so chords
                // peak about 4 dB lower for the same loudness.
                let h = self
                    .clav_hp
                    .process(FilterMode::Hp12, &self.clav_hc, clav_bus);
                let mut c = self.clav_lp.process(FilterMode::Lp12, &self.clav_lc, h);
                for k in 0..6 {
                    let y = self.rot_a[k] * (c - self.rot_y[k]) + self.rot_x[k];
                    self.rot_x[k] = c;
                    self.rot_y[k] = y;
                    c = y;
                }
                bus += clav_preamp(c);
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
                // The preamp's output coupling: its curve is lopsided, so
                // it leaves an offset (moving with the tremolo) that the
                // instrument's output capacitor keeps from the speaker.
                let hp = y - self.dc_x + self.dc_r * self.dc_y;
                self.dc_x = y;
                self.dc_y = hp;
                // The reed piano's preamp makes up the electrostatic
                // pickup's lower output.
                y = hp * REED_GAIN;
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
            MODEL | CLAV_TONE => self.update_model(),
            _ => {}
        }
        if matches!(
            id,
            MODEL | DECAY | RELEASE | BELL | VOICING | BARK | PICKUP | CLAV_TONE | MUTE | DROP
        ) {
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
        self.clav_leak = (-std::f32::consts::TAU * CLAV_LEAK_HZ / r).exp();
        self.dc_r = (-std::f32::consts::TAU * REED_COUPLING_HZ / r).exp();
        self.rot_a = ROTATOR_HZ.map(|hz| {
            let t = (std::f32::consts::PI * hz / r).tan();
            (t - 1.0) / (t + 1.0)
        });
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
        for s in self.strings.iter_mut() {
            s.fill(0.0);
        }
        self.clav_hp.clear();
        self.clav_lp.clear();
        self.rot_x = [0.0; 6];
        self.rot_y = [0.0; 6];
        self.dc_x = 0.0;
        self.dc_y = 0.0;
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
