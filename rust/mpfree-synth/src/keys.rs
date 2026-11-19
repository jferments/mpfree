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
/// The phase rotator's corners on the tine bus, Hz.
const TINE_ROTATOR_HZ: [f32; 6] = [100.0, 100.0, 300.0, 300.0, 900.0, 900.0];

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

/// A tine voice across `out`: its three modes (the fundamental under the
/// hammer, the bending modes ringing on their own) and the magnet. True
/// once it is free.
#[inline(always)]
fn tine_run(v: &mut Voice, out: &mut [f32]) -> bool {
    for o in out.iter_mut() {
        let x = strike(v);
        let y0 = v.b0[0] * x + v.c1[0] * v.y1[0] - v.c2[0] * v.y2[0];
        let y1 = v.c1[1] * v.y1[1] - v.c2[1] * v.y2[1];
        let y2 = v.c1[2] * v.y1[2] - v.c2[2] * v.y2[2];
        v.y2[0] = v.y1[0];
        v.y2[1] = v.y1[1];
        v.y2[2] = v.y1[2];
        v.y1[0] = y0;
        v.y1[1] = y1;
        v.y1[2] = y2;
        let d = y0 + y1 + y2;
        // The flux's change since the last frame, without taking two
        // nearly equal values apart (a soft note barely moves the curve):
        // with p where it was,
        // 1/(1+u^2) - 1/(1+p^2) = -(u-p)(u+p) / ((1+u^2)(1+p^2)).
        let du = v.swing * (d - v.last);
        let u = v.rest + v.swing * d;
        let den = 1.0 + u * u;
        let y = -du * (2.0 * u - du) / (den * v.last_den) * v.norm;
        v.last = d;
        v.last_den = den;
        if voice_out(v, y, d * v.fol, o) {
            return true;
        }
    }
    false
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
/// cantilever's second and third bending modes (about 7.35 and 19 times
/// it at middle C, sharp of a free bar's 6.27 and 17.55 because of the
/// tuning spring's mass), heard mostly in the attack. The engine sets the
/// bending modes' ratios, levels and decays for each note from the
/// measurements ([`tine_ratio`], [`tine_bell_db`], [`tine_mode_t60`]); the
/// values here are middle C's at Bell 40, for the Synth tab's pictures.
pub const TINE: [Mode; MODES] = [
    m(1.0, 1.0, 1.0, false),
    m(7.35, 0.022, 0.08, true),
    m(19.0, 0.003, 0.02, true),
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

/// How far a full-velocity hit moves a reed at `hz`, against middle C's:
/// low notes swing wider, so they bark sooner.
pub fn register_swing(hz: f32) -> f32 {
    (261.63 / hz.max(1.0)).sqrt().clamp(0.5, 1.6)
}

/// Where the tine's swing starts to fall with pitch, Hz (see
/// [`tine_register`]).
const TINE_KNEE_HZ: f32 = 280.0;
/// The swing a full-velocity hit on middle C takes across the tine's
/// pickup, in the field's widths, at Bark 0 and Bark 100 (an exponential
/// between): from 0.4 of the field (nearly clean, but for a little growl
/// in the bass) to eight fields (a snarl). The tine piano measured for
/// this pass swung about 1.5 fields on middle C at full velocity: Bark 45,
/// the default, which with Voicing near 55 to 85 matches its harmonics.
const TINE_SWING: (f32, f32) = (0.4, 8.0);
/// The range of the tine's pickup drive from the softest velocity to the
/// hardest at Vel > tone 100, dB (40 dB at the default 60: the measured
/// tine piano's softest hits swung about an eighth as far as its hardest).
const TINE_TONE_DB: f32 = 66.0;

/// How far a full-velocity hit swings a tine across its pickup at `hz`,
/// against middle C's. Fitted to a tine piano's recordings (single notes
/// F1 to C7, five velocities: the swing that gives each note's harmonics
/// through [`flux`]): about 1.8 times middle C's in the bass, falling as
/// the square of the frequency above about 280 Hz, so the top two octaves
/// stay nearly pure while the bass growls.
pub fn tine_register(hz: f32) -> f32 {
    let k = |f: f32| 1.0 / (1.0 + (f / TINE_KNEE_HZ) * (f / TINE_KNEE_HZ));
    k(hz.max(1.0)) / k(261.63)
}

/// The swing a full-velocity hit on middle C takes across the tine's
/// pickup, in the field's widths, for **Bark** `bark` (0 to 100): see
/// [`TINE_SWING`].
pub fn tine_bark(bark: f32) -> f32 {
    let b = (bark / 100.0).clamp(0.0, 1.0);
    TINE_SWING.0 * (TINE_SWING.1 / TINE_SWING.0).powf(b)
}

/// How hard a hit at `velocity` (1 to 127) drives a tine across its
/// pickup against a full-velocity hit, for **Vel > tone** `vel_tone` (0 to
/// 100): the voice core's square law over up to [`TINE_TONE_DB`]. At 0
/// every hit drives it as hard as a full one.
pub fn tine_drive(velocity: u8, vel_tone: f32) -> f32 {
    velocity_gain(velocity, TINE_TONE_DB * (vel_tone / 100.0).clamp(0.0, 1.0)).max(1e-3)
}

/// The tine pickup's gain for a swing of `a` field widths around the rest
/// point `u0` on a note at `hz`: how strong the flux's change over a cycle
/// of the note is against a plain sine's, plain (RMS) and as the loudness
/// meter hears it (BS.1770's weighting, against the note's own
/// fundamental). Small swings give the curve's slope at the rest point; a
/// wide swing spends most of its cycle where the field is flat and puts
/// its power into harmonics, which the ear weighs more in the bass and
/// over 2 kHz. The voice divides by it ([`Keys`]' `tune_tine`), so each
/// note plays at its velocity's level however hard it drives the pickup
/// and Bark changes the colour, not the level. As a hit dies down the gain
/// comes back up, so a hard note's sustain is held up: the pickup
/// compresses it, as on the instrument.
pub fn tine_pickup_gain(u0: f32, a: f32, hz: f32) -> (f32, f32) {
    const N: usize = 96;
    let mut s = [0.0f32; N];
    let (mut re, mut im, mut all) = (0.0f32, 0.0f32, 0.0f32);
    // Round the cycle by turning a unit vector (half a step in).
    let (ds, dc) = (std::f32::consts::TAU / N as f32).sin_cos();
    let (mut sin, mut cos) = (std::f32::consts::PI / N as f32).sin_cos();
    for si in s.iter_mut() {
        let d = flux_slope(u0 + a * sin) * cos;
        *si = d;
        re += d * cos;
        im += d * sin;
        all += d * d;
        (sin, cos) = (sin * dc + cos * ds, cos * dc - sin * ds);
    }
    // Power (per unit of a plain sine's) in the fundamental, in the rest,
    // and the rest's mean squared harmonic number, from the waveform's
    // slope across the cycle.
    let n = N as f32;
    let p1 = 2.0 * (re * re + im * im) / (n * n);
    let p = all / n;
    let mut slope = 0.0f32;
    for i in 0..N {
        let ds = s[(i + 1) % N] - s[(i + N - 1) % N];
        slope += ds * ds;
    }
    let k2 = slope / n * (n / (2.0 * std::f32::consts::TAU)).powi(2);
    let ph = (p - p1).max(0.0);
    let kh = if ph > 1e-12 {
        ((k2 - p1) / ph).max(4.0).sqrt()
    } else {
        2.0
    };
    let hz = hz.max(1.0);
    let heard = p1 + ph * loudness_weight(kh * hz) / loudness_weight(hz);
    ((2.0 * p).sqrt().max(1e-4), (2.0 * heard).sqrt().max(1e-4))
}

/// How much a tine note's resonator rings shorter (under 1) or longer
/// than Decay says, so middle C is heard ringing as labelled. The pickup
/// compresses a wide swing (its gain comes back up as the note dies down,
/// so the sound falls slower than the tine), or near the field's axis
/// expands it; over the note's first 40 dB (from 10 dB down, where a ring
/// time is read) its level falls `1 + s` times as fast as the tine's, `s`
/// the slope of the pickup's plain gain against the swing, on log scales.
/// For the swing `a` of a hit around the rest point `u0` at `hz`, between
/// 0.5 and 2.
pub fn tine_decay_comp(u0: f32, a: f32, hz: f32) -> f32 {
    let (hi, _) = tine_pickup_gain(u0, a * 0.316, hz);
    let (lo, _) = tine_pickup_gain(u0, a * 0.01, hz);
    let s = (hi / lo).ln() / (0.316f32 / 0.01).ln();
    (1.0 + s).clamp(0.5, 2.0)
}

/// How loud the ear (the loudness meter) finds a partial at `hz` against
/// one at 1 kHz, in power: BS.1770's high shelf ([`ear_weight`]) and its
/// low cut (two poles at 38 Hz), near enough.
fn loudness_weight(hz: f32) -> f32 {
    let f2 = hz * hz;
    let hp = f2 / (f2 + 38.0 * 38.0);
    ear_weight(hz) * hp * hp
}

/// The tine's bending modes as measured on a tine piano's recordings, at
/// C1, C2 ... C7 (a straight line between): the second mode's ratio to
/// the note (7.1 in the bass, 7.3 to 7.4 through the middle, falling to
/// about 6.3 at the top, where the tuning spring sits nearer the tip) and
/// the third's (20.6 in the bass down to about 17.5 at C5).
const TINE_RATIOS: [[f32; 7]; 2] = [
    [7.15, 7.15, 7.25, 7.35, 7.38, 6.6, 6.3],
    [20.6, 20.6, 20.3, 19.0, 17.5, 17.5, 17.5],
];

/// The tine's third bending mode against its bell, as heard: measured 15
/// to 20 dB under it.
const MODE3_SHARE: f32 = 0.14;

/// The tine's third bending mode against its bell for **Hardness**
/// `hardness` (0 to 100): [`MODE3_SHARE`] at 50, up to 10 dB more with a
/// hard hammer and 10 dB less with a soft one. A hammer's tip filters what
/// it strikes the more the higher it is, so its hardness shows most in the
/// tine's highest mode: the tick of the attack.
pub fn tine_tick(hardness: f32) -> f32 {
    MODE3_SHARE * 10f32.powf((hardness.clamp(0.0, 100.0) - 50.0) / 100.0)
}

/// A tine's bending mode `k` (1 or 2) at `hz`: its ratio to the note, from
/// [`TINE_RATIOS`].
pub fn tine_ratio(k: usize, hz: f32) -> f32 {
    let c = &TINE_RATIOS[(k.max(1) - 1).min(1)];
    let x = ((hz.max(1.0) / 32.703).log2()).clamp(0.0, 6.0);
    let i = (x as usize).min(5);
    c[i] + (c[i + 1] - c[i]) * (x - i as f32)
}

/// The tine's bell (its second bending mode) as heard against the note on
/// a full-velocity hit, dB, for **Bell** `bell` and **Hardness**
/// `hardness` (0 to 100) at `hz`; a softer hit rings it less, as the
/// fourth root of its drive. The measured tine piano's bell sat about 33
/// dB under the note on its softest hits from F1 to F4 and about 26 dB
/// under on hard ones (Bell 40 here: -30 and -36 dB), and fell away over
/// C6 (-46 dB at G6); a harder hammer rings it more (up to 6 dB either
/// way). Bell 100 rings it as loud as the note.
pub fn tine_bell_db(bell: f32, hardness: f32, hz: f32) -> f32 {
    let top = 20.0 * (hz.max(1.0) / 1046.5).log2().max(0.0);
    -50.0 + 0.5 * bell + 0.12 * (hardness - 50.0) - top
}

/// How long the tine's bending modes ring at `hz`, seconds to fall 60 dB:
/// the bell (k 1) about 0.65 s at D4 (measured 0.55 to 2 s through the
/// middle), shorter as `hz^-0.9` (0.15 s at D6), at most 1.5 s; the third
/// mode a quarter of that.
pub fn tine_mode_t60(k: usize, hz: f32) -> f32 {
    let t = (0.65 * (293.66 / hz.max(1.0)).powf(0.9)).min(1.5);
    if k >= 2 {
        0.25 * t
    } else {
        t
    }
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
/// key's own small difference `key` (-1 to 1). For a tine, a displacement
/// of 1 is a full-velocity hit on middle C ([`tine_bark`]).
pub fn pickup_shape(reed: bool, voicing: f32, bark: f32, key: f32) -> (f32, f32) {
    let v = (voicing / 100.0 + 0.04 * key).clamp(0.0, 1.0);
    let b = bark / 100.0;
    if reed {
        (0.4 * (1.0 - v), 0.2 + 0.6 * b)
    } else {
        // From the axis (0) to the flux curve's inflection, 1/sqrt(3),
        // where it is straightest.
        (0.6 * v, tine_bark(bark))
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
    [3.3, -1.8, -2.6, -0.75, 1.2, 2.8, 2.35],
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
    /// A tine: how hard its last hit drives the pickup ([`tine_drive`]),
    /// the bell's share from the drive ([`tine_bell_db`]),
    /// the pickup's gain at that swing ([`tine_pickup_gain`]) and the
    /// swings it was found for (the hit's and velocity 100's), the
    /// curve's denominator `1 + u^2` a frame ago (`last` holds the
    /// displacement a frame ago), each bending mode's kick at a hit and
    /// the frames until it lands (0 once it has).
    drive: f32,
    bellv: f32,
    pg: f32,
    pg_at: f32,
    pg_ref: f32,
    /// A tine's decay against Decay's, so it is heard as labelled
    /// ([`tine_decay_comp`]).
    comp: f32,
    last_den: f32,
    kick: [f32; MODES],
    kick_in: u32,
    /// A tine's level follower's scale: its displacement to its level.
    fol: f32,
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
    /// Whether this voice is a tine (set when it is struck, like `clav`).
    tine: bool,
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
    /// The tine bus's phase rotator, the same way, and whether it runs.
    trot_a: [f32; 6],
    trot_x: [f32; 6],
    trot_y: [f32; 6],
    tine_bus: bool,
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
            trot_a: [0.0; 6],
            trot_x: [0.0; 6],
            trot_y: [0.0; 6],
            tine_bus: false,
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

    /// Whether any tine still sounds.
    fn tine_alive(&self) -> bool {
        self.voices.iter().any(|v| v.tine && v.state != State::Free)
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
        if v.tine {
            self.tune_tine(v, pitch);
            return;
        }
        // A reed.
        let rate = self.rate as f32;
        let held = v.state == State::On;
        let bell = self.v(BELL) / 100.0;
        let f0 = self.tuning.hz(pitch).clamp(10.0, rate * 0.45);
        let base = decay_for(self.v(DECAY), f0);
        let release = self.v(RELEASE) * 0.001;
        for (k, md) in REED.iter().enumerate() {
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
            // ratio: levels are set as heard, so the reed's motion in each
            // mode is the level over the ratio.
            let mut lvl = md.level * v.jit_level[k] / md.ratio;
            if md.bell {
                lvl *= 0.3 + 0.7 * bell;
            }
            v.c1[k] = 2.0 * r * w.cos();
            v.c2[k] = r * r;
            v.b0[k] = w.sin() * lvl;
        }
        // The pickup: its rest point and swing, and the scale that brings
        // its output back to the reed's level. It hears the curve's change
        // (a capacitor's current), so its output grows with the note's
        // frequency: that is divided out, and what stays is the harmonics'
        // tilt up.
        let (rest, g) = pickup_shape(true, self.v(VOICING), self.v(BARK), key_spread(v.note));
        let swing = g * register_swing(f0);
        let w0 = std::f32::consts::TAU * f0 / rate;
        let slope = capacitance_slope(rest);
        if v.rest != rest || v.swing != swing {
            let x = v.y1.iter().sum::<f32>();
            v.last = capacitance(rest + swing * x);
        }
        v.rest = rest;
        v.swing = swing;
        v.norm = 1.0 / (swing * w0 * slope);
        v.tuned = pitch;
    }

    /// Sets a tine voice's resonators and pickup for its pitch and gate:
    /// the fundamental (Decay's law, heard: see [`tine_decay_comp`]),
    /// driven by the hammer; the two bending modes at the measured ratios,
    /// levels and decays for the note ([`tine_ratio`], [`tine_bell_db`],
    /// [`tine_mode_t60`]), which the hit kicks at once ([`Voice::kick`]):
    /// the measured bell is there whatever the hammer, which a smooth push
    /// of a millisecond or more would filter away over 1 kHz. The pickup's
    /// swing is the hit's drive across Bark's swing and the register's
    /// ([`tine_register`]), its output divided by its gain at that swing
    /// ([`tine_pickup_gain`]).
    fn tune_tine(&self, v: &mut Voice, pitch: f32) {
        let rate = self.rate as f32;
        let held = v.state == State::On;
        let f0 = self.tuning.hz(pitch).clamp(10.0, rate * 0.45);
        // The pickup first: the fundamental's decay depends on it.
        let (rest, g) = pickup_shape(false, self.v(VOICING), self.v(BARK), key_spread(v.note));
        let swing = g * tine_register(f0);
        let a = swing * v.drive;
        // Plain across the velocities (Velocity's square law is a level's)
        // and as heard across the keys, at velocity 100 (each key as loud
        // as the next). Found again only when the swing moves by 2 %
        // (a bend moves the register's): the level moves by under 1 %.
        let a_ref = swing * tine_drive(100, self.v(VEL_TONE));
        let moved = |x: f32, at: f32| (x - at).abs() > 0.02 * x;
        if v.rest != rest || moved(a, v.pg_at) || moved(a_ref, v.pg_ref) {
            let (plain, _) = tine_pickup_gain(rest, a, f0);
            let (plain_ref, heard_ref) = tine_pickup_gain(rest, a_ref, f0);
            v.pg = plain * heard_ref / plain_ref;
            // Middle C at this hit rings as labelled (the preset's own
            // rest point, without the key's offset); every key gets the
            // same, so the keys keep Decay's law.
            let rest0 = pickup_shape(false, self.v(VOICING), self.v(BARK), 0.0).0;
            v.comp = tine_decay_comp(rest0, g * v.drive, 261.63);
            v.pg_at = a;
            v.pg_ref = a_ref;
        }
        if v.rest != rest || v.swing != swing {
            // Where the curve was a frame ago, for the new rest and swing.
            let u = rest + swing * v.last;
            v.last_den = 1.0 + u * u;
        }
        v.rest = rest;
        v.swing = swing;
        let w0 = std::f32::consts::TAU * f0 / rate;
        v.norm = 1.0 / (a * w0 * v.pg);

        let base = decay_for(self.v(DECAY), f0);
        let release = self.v(RELEASE) * 0.001;
        let bell = if self.v(BELL) > 0.0 {
            10f32.powf(tine_bell_db(self.v(BELL), self.v(HARDNESS), f0) / 20.0) * v.bellv
        } else {
            0.0
        };
        // Each key's tuning spring sits a little differently: its bending
        // modes are its own, by up to 1.5 %.
        let spread = 1.0 + 0.015 * key_spread(v.note.wrapping_add(53));
        for k in 0..MODES {
            let (ratio, heard, t60) = match k {
                0 => (1.0, 1.0, base * v.comp),
                1 | 2 => (
                    tine_ratio(k, f0) * spread,
                    if k == 1 {
                        bell
                    } else {
                        bell * tine_tick(self.v(HARDNESS))
                    },
                    tine_mode_t60(k, f0).min(0.5 * base),
                ),
                _ => (0.0, 0.0, 0.0),
            };
            let hz = f0 * ratio * v.jit_ratio[k];
            if hz >= 0.45 * rate || hz <= 10.0 || heard == 0.0 {
                v.b0[k] = 0.0;
                v.c1[k] = 0.0;
                v.c2[k] = 0.0;
                v.kick[k] = 0.0;
                continue;
            }
            let w = std::f32::consts::TAU * hz / rate;
            let mut t = t60.max(0.004);
            if !held {
                t = t.min(release);
            }
            let r = 10f32.powf(-3.0 / (t * rate));
            // The pickup hears velocity, which favours a mode by its
            // ratio: levels are set as heard, so the tine's motion in each
            // mode is the level over the ratio.
            let lvl = heard * v.jit_level[k] / ratio;
            v.c1[k] = 2.0 * r * w.cos();
            v.c2[k] = r * r;
            if k == 0 {
                v.b0[k] = w.sin() * lvl;
                v.kick[k] = 0.0;
            } else {
                v.b0[k] = 0.0;
                v.kick[k] = w.sin() * lvl * v.drive;
            }
        }
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
        v.tine = !clav && !self.reed();
        self.tine_bus |= v.tine;
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
        // Velocity: a softer hit is a longer contact (Vel > tone sets how
        // much), so it reaches fewer upper modes; on a tine it also drives
        // the pickup less, so it barks less.
        let vt = self.v(VEL_TONE) / 100.0;
        let tone_vel = 1.0 - vt * (1.0 - vel);
        let tine = v.tine;
        if tine {
            v.drive = tine_drive(velocity, self.v(VEL_TONE));
            // A hard hit rings the bell more against the note: about 6 dB
            // from the softest hits to the hardest, as measured.
            v.bellv = v.drive.powf(0.25);
        }
        self.tune(&mut v, pitch);
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
        } else if tine {
            // Unit area times the drive: the tine swings as far as the
            // hit drives it, whatever the hammer.
            v.drive * std::f32::consts::FRAC_PI_2 / len
        } else {
            // Unit area: the fundamental rings at the strike's level
            // whatever the hammer.
            amp * std::f32::consts::FRAC_PI_2 / len
        };
        // The level follows Velocity's range; the strike's own level only
        // sets how hard the pickup is driven.
        if tine {
            v.gain = velocity_gain(velocity, self.v(VELOCITY)) * key_level(self.v(MODEL), pitch);
            // The hit kicks the bending modes somewhere in its contact (at
            // least a millisecond's span): a chord's bells then start out
            // of step, as no hand lands its fingers together, rather than
            // all in phase on one frame.
            let span = len.max(0.001 * rate);
            v.kick_in = 1 + (self.rng.unit() * span) as u32;
            // The level follower reads the displacement as the note's
            // level, and as it dies down the pickup's gain comes back up
            // to its slope at the rest point.
            v.fol = v.gain / v.drive * (flux_slope(v.rest).abs() / v.pg).max(1.0);
        } else {
            v.gain =
                velocity_gain(velocity, self.v(VELOCITY)) / amp * key_level(self.v(MODEL), pitch);
        }
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
            // A mode that has died away (the bending modes go long before
            // the fundamental) would ring on through the denormals while
            // the note sounds; under -400 dB it is exact 0.
            for m in 0..MODES {
                if v.y1[m].abs() < 1e-20 && v.y2[m].abs() < 1e-20 {
                    v.y1[m] = 0.0;
                    v.y2[m] = 0.0;
                }
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
        // The tines' rotator runs from a tine's strike to the first
        // control step with none left (decided here, at fixed frames, so
        // the output is the same for any block size).
        self.tine_bus = self.tine_alive();
        if !self.tine_bus {
            self.trot_x = [0.0; 6];
            self.trot_y = [0.0; 6];
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
        // A driven tine's output coupling (below).
        let tine_coupled = !reed && !self.clav() && self.drive_amt > 0.0;
        let amp_trem = self.v(TREM_MODE) >= 0.5;
        let clav_on = self.clav_alive();
        let len = to - from;
        // Each voice across the stretch in turn (at most a control block),
        // so its state stays in registers: the tines and reeds into one
        // bus, the clav strings into theirs, summed in the voices' order.
        let mut buses = [0.0f32; CTRL as usize];
        let mut clavs = [0.0f32; CTRL as usize];
        let mut tines = [0.0f32; CTRL as usize];
        let tine_on = self.tine_bus;
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
            } else if !v.tine {
                // A reed: five modes and the capacitor.
                for o in buses[..len].iter_mut() {
                    let x = strike(v);
                    let mut d = 0.0;
                    for k in 0..MODES {
                        let y = v.b0[k] * x + v.c1[k] * v.y1[k] - v.c2[k] * v.y2[k];
                        v.y2[k] = v.y1[k];
                        v.y1[k] = y;
                        d += y;
                    }
                    let u = v.rest + v.swing * d;
                    let c = capacitance(u);
                    let y = (c - v.last) * v.norm;
                    v.last = c;
                    if voice_out(v, y, d, o) {
                        break;
                    }
                }
            } else {
                // A tine: the stretch split where the bending modes' kick
                // lands, if it does.
                let out = &mut tines[..len];
                let at = (v.kick_in as usize).wrapping_sub(1);
                if at < len {
                    let (pre, post) = out.split_at_mut(at);
                    if !tine_run(v, pre) {
                        v.y1[1] += v.kick[1];
                        v.y1[2] += v.kick[2];
                        v.kick_in = 0;
                        tine_run(v, post);
                    }
                } else {
                    v.kick_in = v.kick_in.saturating_sub(len as u32);
                    tine_run(v, out);
                }
            }
        }
        for (j, i) in (from..to).enumerate() {
            let mut bus = buses[j];
            let clav_bus = clavs[j];
            if tine_on {
                // The tines' phase rotator: a hard hit's pickup pulses
                // spread in time, the spectrum untouched, so a loud chord
                // peaks lower for the same loudness.
                let mut c = tines[j];
                for k in 0..6 {
                    let y = self.trot_a[k] * (c - self.trot_y[k]) + self.trot_x[k];
                    self.trot_x[k] = c;
                    self.trot_y[k] = y;
                    c = y;
                }
                bus += c;
            }
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
            } else if tine_coupled {
                // The same coupling after the tine's Drive: a hard hit's
                // lopsided pulses leave an offset once driven.
                let hp = y - self.dc_x + self.dc_r * self.dc_y;
                self.dc_x = y;
                self.dc_y = hp;
                y = hp;
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
            MODEL
                | DECAY
                | RELEASE
                | HARDNESS
                | BELL
                | VOICING
                | BARK
                | PICKUP
                | CLAV_TONE
                | MUTE
                | DROP
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
        let allpass = |hz: f32| {
            let t = (std::f32::consts::PI * hz / r).tan();
            (t - 1.0) / (t + 1.0)
        };
        self.rot_a = ROTATOR_HZ.map(allpass);
        self.trot_a = TINE_ROTATOR_HZ.map(allpass);
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
        self.trot_x = [0.0; 6];
        self.trot_y = [0.0; 6];
        self.tine_bus = false;
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A chord held on long tines: the bending modes die away long before
    /// the fundamental, and none of them rings on through the denormals
    /// (which would make each sample many times slower on a processor
    /// without flush-to-zero).
    #[test]
    fn a_held_notes_died_away_modes_never_ring_through_the_denormals() {
        let rate = 44_100;
        let mut k = Keys::new(rate);
        let p = Keys::presets()
            .iter()
            .find(|p| p.name == "Long Tines")
            .expect("Long Tines");
        k.load_preset(p);
        k.prepare(rate, 128);
        let notes: Vec<Timed> = [36u8, 48, 55, 60, 64, 67, 72, 76, 84, 96]
            .iter()
            .map(|&note| {
                Timed::new(
                    0,
                    NodeEvent::NoteOn {
                        note,
                        velocity: 110,
                    },
                )
            })
            .collect();
        let (mut l, mut r) = ([0.0f32; 128], [0.0f32; 128]);
        let mut zeroed = 0;
        for b in 0..(14 * rate as u64 / 128) {
            let ev = if b == 0 { &notes[..] } else { &[] };
            k.process(&Ctx::new(rate, b * 128), ev, &mut l, &mut r);
            for v in k.voices.iter().filter(|v| v.state != State::Free) {
                for m in 0..MODES {
                    assert!(
                        !v.y1[m].is_subnormal() && !v.y2[m].is_subnormal(),
                        "block {b}"
                    );
                }
            }
            zeroed = k
                .voices
                .iter()
                .filter(|v| v.state != State::Free)
                .flat_map(|v| v.y1)
                .filter(|&y| y == 0.0)
                .count();
        }
        assert!(peak(&l) > 1e-4, "the chord still sounds");
        assert!(zeroed > 0, "some modes have died away to exact 0");
    }

    fn peak(x: &[f32]) -> f32 {
        x.iter().fold(0.0, |a, &v| a.max(v.abs()))
    }
}
