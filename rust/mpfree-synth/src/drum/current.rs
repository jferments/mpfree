//! The Current voices: each drum type with its own knobs
//! ([`crate::drum_params::knobs`]), velocity and accent to tone, a little
//! variation from hit to hit, and decays that match their labels. A kit
//! plays these when its Voices setting is Current (every factory kit).
//!
//! The parts a hit is made of, each run every frame:
//!
//! - **The body**: up to two sines (or the 808's), with a pitch that moves
//!   every [`CTRL`] frames from the hit's start: an exponential sweep, or
//!   the 808's punch and sigh ([`crate::body808`]); a shaper, FM, and a
//!   one-pole low-pass (the 808's Tone).
//! - **The metal**: the 808's six square oscillators (205.3, 304.4, 369.6
//!   and 522.7 Hz, and the two trimmed to 540 and 800 Hz, each 47.98 %
//!   wide; Werner, Abel and Smith, ICMC-SMC 2014), band-limited (polyBLEP),
//!   and noise, through up to three bands, each a band-pass (the 808's
//!   near 3.44 and 7.1 kHz), an optional high-pass and its own envelope.
//! - **The modes** ([`crate::modal`]): toms, congas, woodblock and bell, a
//!   bank of decaying resonators struck by a half-sine stick, bent down
//!   after a hard hit as a drum head is.
//! - **The clap**: its bursts, at a gap a little uneven, over a room tail.
//! - **The click**: a short high-passed noise burst (a beater, a stick).
//! - **The drive**, and the fade when a hit is choked, replaced or let go
//!   (a smooth step, so it never clicks).
//!
//! Everything a voice does follows its own frames since the hit, so the
//! output doesn't depend on how the host cuts blocks.

use super::classic::{t60, Out, Svf, METAL, SILENT};
use crate::body808::{self, drive_lift, pitch_semis, sin_turns};
use crate::drum_params::{field, model::*, MODELS};
use crate::dsp::{drive_n, poly_blep4, Rng};
use crate::modal::{ModalBank, Pulse};

/// Frames per control step: the body's pitch and the modes' bend move
/// this often, from the hit's start.
pub const CTRL: u32 = 16;
/// The 808 metal's squares: 47.98 % wide (the paper's duty cycle).
const METAL_PW: f32 = 0.4798;
/// The 808 cymbal's band-pass centres, Hz.
pub const BP_LOW: f32 = 3440.0;
pub const BP_HIGH: f32 = 7100.0;
/// A hit at full velocity with full accent is this much louder, dB.
const ACCENT_DB: f32 = 4.0;
/// Membrane modes (a struck circular head): the zeros of the Bessel
/// functions, as (m, zero), in rising order. Ratios are zero / 2.405.
const MEMBRANE: [(u32, f32); 8] = [
    (0, 2.4048),
    (1, 3.8317),
    (2, 5.1356),
    (0, 5.5201),
    (3, 6.3802),
    (1, 7.0156),
    (4, 7.5883),
    (2, 8.4172),
];
/// A free bar's modes (a woodblock or clave): 1 : 2.756 : 5.404 : 8.933.
const BAR: [f32; 4] = [1.0, 2.756, 5.404, 8.933];
/// A hand bell's modes (agogo-like): ratio, level, decay ratio.
const BELL_MODES: [(f32, f32, f32); 6] = [
    (1.0, 1.0, 1.0),
    (2.07, 0.6, 0.7),
    (2.76, 0.5, 0.55),
    (4.17, 0.35, 0.45),
    (5.38, 0.25, 0.35),
    (6.9, 0.15, 0.3),
];

/// What a voice is doing.
pub(super) use super::classic::State;

/// The body's pitch curve.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Pitch {
    /// Fixed.
    #[default]
    Flat,
    /// An exponential sweep from `semis` down, to 1 % in `frames`.
    Exp { semis: f32, frames: f32 },
    /// The 808: a punch of `punch` semitones gone in 6 ms, and a sigh of
    /// `sigh` semitones landing at `sigh_frames`.
    Drop808 {
        punch: f32,
        punch_frames: f32,
        sigh: f32,
        sigh_frames: f32,
    },
}

impl Pitch {
    /// Semitones above the settled pitch `t` frames into the hit.
    fn at(&self, t: f32) -> f32 {
        match *self {
            Pitch::Flat => 0.0,
            Pitch::Exp { semis, frames } => semis * (-4.605_17 * t / frames.max(1.0)).exp(),
            Pitch::Drop808 {
                punch,
                punch_frames,
                sigh,
                sigh_frames,
            } => pitch_semis(t, sigh, sigh_frames, punch, punch_frames),
        }
    }
}

/// One hit of a Current voice. A model leaves the parts it doesn't use
/// off.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Voice {
    pub(super) state: State,
    pub(super) pad: u8,
    pub(super) choke: u8,
    pub(super) age: u64,
    /// Let go on its pad's note-off (Plays: While held).
    pub(super) gated: bool,
    /// The hit's gain before pan (velocity, level, accent, variation), and
    /// each side's after it.
    pub(super) gain: f32,
    pub(super) gain_l: f32,
    pub(super) gain_r: f32,
    /// The fade out: how far along (0 to 1) and its step a frame.
    fade: f32,
    fade_step: f32,
    frames: u32,
    model: u8,

    // The body.
    body_on: bool,
    pitch: Pitch,
    /// The settled pitch, Hz, with the variation's cents in it.
    f0: f32,
    inv_rate: f32,
    ph_a: f32,
    inc: f32,
    dinc: f32,
    ph_b: f32,
    ratio_b: f32,
    lvl_a: f32,
    lvl_b: f32,
    body: f32,
    body_mul: f32,
    body_b: f32,
    body_b_mul: f32,
    shape: f32,
    fm_ph: f32,
    fm_ratio: f32,
    fm_index: f32,
    fm_env: f32,
    fm_mul: f32,
    /// A one-pole low-pass on the body (the 808's Tone): coefficient, state.
    lp_a: f32,
    lp: f32,
    /// The 808's trigger pulse leaking through: frames left, level.
    trig: u32,
    trig_lvl: f32,

    // The metal and the noise, through the bands.
    n_metal: usize,
    ph_m: [f32; 6],
    dt_m: [f32; 6],
    metal_lvl: f32,
    /// The squares' gentle low-pass ([`METAL_TOP`]): its step and state.
    metal_a: f32,
    metal_y: f32,
    rng: Rng,
    noise_lvl: f32,
    n_bands: usize,
    bp: [Svf; 3],
    bp_out: [Out; 3],
    hp: [Svf; 3],
    hp_out: [Out; 3],
    env: [f32; 3],
    env_mul: [f32; 3],
    band_lvl: [f32; 3],
    /// The cymbal's top band reads the middle band's band-pass.
    top_from_mid: bool,

    // The clap: bursts left, frames to the next, the gap, the burst's
    // envelope, the room's tail.
    clap: bool,
    bursts_left: u32,
    next_burst: u32,
    gap: u32,
    burst: f32,
    burst_mul: f32,
    tail: f32,
    tail_mul: f32,
    tail_lvl: f32,

    // The click.
    click: f32,
    click_mul: f32,
    click_lvl: f32,
    click_hp_a: f32,
    click_x: f32,
    click_y: f32,

    // The modes.
    modal: bool,
    bank: ModalBank<8>,
    stick: Pulse,
    ratios: [f32; 8],
    n_modes: usize,
    bend: f32,
    bend_frames: f32,
    /// The longest mode's envelope and its step, for the silence check.
    modal_env: f32,
    modal_mul: f32,
    modal_lvl: f32,

    // The shaker: its swell, and the beads.
    shaker: bool,
    swell: f32,
    swell_step: f32,
    grain: f32,
    bead: f32,
    bead_mul: f32,
    bead_odds: f32,

    // The drive: 0 off, 1 the soft tanh, 2 the 808's; how hard the hit
    // drives it (its bite: a harder hit is driven harder).
    drive: u8,
    /// For the soft drive, the model's level too: every model meets
    /// it at about the same level.
    drive_in: f32,
    drive_gain: f32,
    drive_sq: f32,
    drive_amt: f32,
}

/// A smooth step from 1 to 0 as `x` goes 0 to 1: flat at both ends, so a
/// fade has no corner to click.
#[inline]
fn fade_gain(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    1.0 - x * x * (3.0 - 2.0 * x)
}

/// `10^(db / 20)`.
#[inline]
fn db(x: f32) -> f32 {
    10f32.powf(x / 20.0)
}

/// `J_m(x)`, the Bessel function of the first kind, by its series (`x`
/// under 10 here): how much a membrane mode moves at a radius.
pub fn bessel_j(m: u32, x: f32) -> f32 {
    let h = 0.5 * x;
    let mut term = 1.0f32;
    for k in 1..=m {
        term *= h / k as f32;
    }
    let mut sum = term;
    let h2 = h * h;
    for k in 1..24 {
        term *= -h2 / (k as f32 * (k + m) as f32);
        sum += term;
        if term.abs() < 1e-7 {
            break;
        }
    }
    sum
}

/// What the engine passes a build: the pad's settings and the hit's.
pub(super) struct Hit<'a> {
    pub get: &'a dyn Fn(u16) -> f32,
    pub rate: f32,
    pub pad: usize,
    pub velocity: u8,
    pub seed: u64,
    pub clock: u64,
    /// Accent, variation and Plays count (a Current kit).
    pub current: bool,
    /// The variation's signs for this hit (each ±1, alternating per pad).
    pub signs: (f32, f32),
    /// Whether the hit varies at all: off for the drawn hit.
    pub vary: bool,
}

/// The six metal oscillators' frequencies, Hz, at Spread `spread` (0 to
/// 100; 50 is the 808's) and the tuning `ratio` (1 at Tune 0): spread
/// around their middle.
pub fn metal_hz(spread: f32, ratio: f32) -> [f32; 6] {
    let k = (spread / 50.0).clamp(0.0, 2.0);
    let mid = (METAL.iter().map(|f| f.ln()).sum::<f32>() / 6.0).exp();
    METAL.map(|hz| mid * (hz / mid).powf(k) * ratio)
}

/// The cymbal's two band-pass centres, Hz, at Tone `tone` (0 to 1; the
/// 808's at a half), the tuning `ratio` and the hit's brightness `bright`
/// (1 at full velocity with no accent).
pub fn cymbal_bands(tone: f32, ratio: f32, bright: f32) -> [f32; 2] {
    let k = ratio.sqrt() * bright * ((tone - 0.5) * 1.6).exp2();
    [BP_LOW * k, BP_HIGH * k]
}

/// The metal bank alone, `frames` long at `rate`: the six band-limited
/// squares at Spread `spread` and Tune 0, summed, as a hat's voice makes
/// them (phases from `seed`), for measuring them (D2, D9).
pub fn metal_bank(spread: f32, rate: f32, frames: usize, seed: u64) -> Vec<f32> {
    let mut rng = Rng::new(seed);
    let hz = metal_hz(spread, 1.0);
    let dt = hz.map(|f| f / rate);
    let mut ph = [0.0f32; 6];
    for p in ph.iter_mut() {
        *p = rng.unit();
    }
    let a = metal_top_a(rate);
    let mut y = 0.0;
    (0..frames)
        .map(|_| {
            let mut m = 0.0;
            for k in 0..6 {
                m += metal_square(&mut ph[k], dt[k]);
            }
            y += a * (m - y);
            y
        })
        .collect()
}

/// The 808 metal's squares' width (the paper's duty cycle), for measuring.
pub const METAL_WIDTH: f32 = METAL_PW;

/// Where the metal's gentle low-pass starts, Hz: well above the bands it
/// feeds (the cymbal's highest is near 7 kHz), it takes 4 to 5 dB off the
/// folded harmonics left near the top of hearing (D9).
const METAL_TOP: f32 = 10_000.0;

/// The one-pole step for [`METAL_TOP`] at `rate`.
fn metal_top_a(rate: f32) -> f32 {
    1.0 - (-std::f32::consts::TAU * METAL_TOP.min(0.4 * rate) / rate).exp()
}

/// One sample of a metal square (width [`METAL_PW`]) at phase `t`, then
/// advances it by `dt`. Each step is smoothed over four samples (a
/// four-point BLEP), not the usual two: six squares up to 8 kHz each fold
/// many harmonics back into the hearing range, and the wider smoothing
/// keeps them about 15 dB lower for little more work (D9).
#[inline(always)]
fn metal_square(t: &mut f32, dt: f32) -> f32 {
    let p = *t;
    let q = p - METAL_PW;
    let q = if q < 0.0 { q + 1.0 } else { q };
    let y = if p < METAL_PW { 1.0 } else { -1.0 } + poly_blep4(p, dt) - poly_blep4(q, dt);
    let n = p + dt;
    *t = if n >= 1.0 { n - 1.0 } else { n };
    // Less the DC an uneven square carries, as the synth's pulse does.
    y - (2.0 * METAL_PW - 1.0)
}

/// A stick's or a hand's pulse, in frames: from `long` seconds at no
/// hardness to 0.1 ms at full (`hard` 0 to 1), and never over half the
/// period of `hz`, so a high drum still sounds.
fn stick_len(hard: f32, long: f32, hz: f32, rate: f32) -> f32 {
    let soft = 1.0 - hard.clamp(0.0, 1.0);
    (rate * (0.0001 + long * soft * soft))
        .min(0.5 * rate / hz.max(1.0))
        .max(2.0)
}

/// The share of a part's decay that keeps the hit's fall to -60 dB on
/// its label, when the part starts `head_db` under the hit's peak and the
/// drive lifts quiet sound `lift` times more than loud.
fn decay_share(head_db: f32, lift: f32) -> f32 {
    let fall = 60.0 - head_db + 20.0 * lift.max(1.0).log10();
    60.0 / fall.max(20.0)
}

impl Voice {
    /// Builds a hit. See the module docs and [`crate::drum_params::knobs`]
    /// for what each model does with its knobs.
    pub(super) fn build(h: &Hit) -> Voice {
        let g = |f: u16| (h.get)(f);
        let rate = h.rate;
        let m = crate::drum_params::model_of(g(field::MODEL));
        let vel = f32::from(h.velocity.min(127)) / 127.0;
        let mut rng = Rng::new(h.seed);

        // Velocity and accent: the level, and the bite (how bright and
        // hard the hit is, 1 at full velocity with no accent).
        let vt = g(field::VEL_TONE) / 100.0;
        let vl = g(field::VEL_LEVEL) / 100.0;
        let accent = if h.current {
            let x = ((vel - 0.72) / 0.24).clamp(0.0, 1.0);
            g(field::ACCENT) / 100.0 * x * x * (3.0 - 2.0 * x)
        } else {
            0.0
        };
        // Variation: a level and a pitch away from the last hit's, the
        // other way, by 0.1 to 0.5 dB and 0.5 to 2.5 cents at 50 %, and a
        // little decay and bite.
        let var = if h.current && h.vary {
            g(field::VARIATION) / 50.0
        } else {
            0.0
        };
        let var_db = h.signs.0 * (0.1 + 0.4 * rng.unit()) * var;
        let var_cents = h.signs.1 * (0.5 + 2.0 * rng.unit()) * var;
        let var_decay = 1.0 + 0.03 * var * rng.bipolar();
        let var_bite = 1.0 + 0.04 * var * rng.bipolar();

        let bite = ((1.0 - 1.6 * vt * (1.0 - vel)) * var_bite + 0.35 * accent).max(0.1);
        // Filters move an octave and a half from no bite to full.
        let bright = ((bite - 1.0) * 1.5).exp2();
        let level = g(field::LEVEL);
        let amp = (1.0 - vl + vl * vel * vel)
            * if level <= -60.0 { 0.0 } else { db(level) }
            * db(ACCENT_DB * accent + var_db);
        let ratio = (g(field::TUNE) / 12.0 + var_cents / 1200.0).exp2();
        let decay = (g(field::DECAY) * 0.001 * var_decay).max(0.005);
        let tone = g(field::TONE) / 100.0;
        let snap = g(field::SNAP) / 100.0;
        let nz = g(field::NOISE) / 100.0;
        let drive_amt = g(field::DRIVE) / 100.0;

        let mut v = Voice {
            state: State::On,
            drive_in: bite.min(1.5),
            pad: h.pad as u8,
            choke: g(field::CHOKE) as u8,
            age: h.clock,
            gated: h.current && g(field::PLAYS) >= 0.5,
            model: m,
            inv_rate: 1.0 / rate,
            rng,
            body: 1.0,
            body_b: 1.0,
            click: 1.0,
            click_mul: t60(0.003, rate),
            env: [1.0; 3],
            lp_a: 0.0,
            modal_env: 1.0,
            metal_a: metal_top_a(rate),
            ..Voice::default()
        };
        // The click: a beater or stick, high-passed noise, brighter and
        // louder the harder the hit.
        let click_hp = |v: &mut Voice, hz: f32| {
            v.click_hp_a = (-std::f32::consts::TAU * hz.min(0.45 * rate) / rate).exp();
        };
        click_hp(&mut v, 1800.0 * bright);
        v.click_lvl = snap * 0.5 * bite * bite;

        // The drive: a tanh blended in by Drive, scaled so a hit peaking
        // at 0.5 after its model's level (about where every model peaks)
        // still peaks there; it lifts a hit's tail more than its start
        // (taken out of the decays below).
        let mut lift = 1.0;
        if drive_amt > 0.0 && m != KICK_808 {
            const PEAK: f32 = 0.5;
            v.drive = 1;
            v.drive_gain = 1.0 + drive_amt * 9.0;
            v.drive_sq = PEAK / (v.drive_gain * PEAK).tanh();
            v.drive_amt = drive_amt;
            lift = (1.0 + (v.drive_gain * v.drive_sq - 1.0) * drive_amt).max(1.0);
        }

        let norm;
        match m {
            KICK => {
                // A tuned sine with an exponential sweep (deeper the
                // harder the hit), shaped by Tone; the click on top.
                v.body_on = true;
                v.f0 = 50.0 * ratio;
                v.lvl_a = 1.0;
                v.shape = tone * 4.0 * bite * bite;
                v.pitch = Pitch::Exp {
                    semis: g(field::SWEEP) * (0.4 + 0.6 * bite),
                    frames: g(field::SWEEP_TIME) * 0.001 * rate,
                };
                v.body_mul = t60(decay * decay_share(0.0, lift), rate);
                v.click_lvl = snap * 0.6 * bite * bite * bite;
                norm = 0.415;
            }
            KICK_808 => {
                // The 808: a sine at 48 Hz (Tune 0) struck by the trigger:
                // the punch shifts it up for 6 ms, the sigh falls from
                // 58 Hz to 48 Hz over 300 ms; Tone a one-pole low-pass;
                // the 808 body's soft drive.
                v.body_on = true;
                v.f0 = 48.0 * ratio;
                v.lvl_a = 1.0;
                v.pitch = Pitch::Drop808 {
                    punch: snap * 24.0 * bite * bite.sqrt(),
                    punch_frames: body808::PUNCH_S * rate,
                    sigh: g(field::SWEEP),
                    sigh_frames: (g(field::SWEEP_TIME) * 0.001 * rate).max(1.0),
                };
                let fc = 120.0 * (tone * 6.0).exp2() * bright;
                v.lp_a = (-std::f32::consts::TAU * fc.min(0.45 * rate) / rate).exp();
                v.trig = (0.001 * rate) as u32;
                v.trig_lvl = 0.5 * snap * bite * bite * bite;
                v.click_lvl = 0.0;
                if drive_amt > 0.0 {
                    let (k, n) = body808::drive_gains(drive_amt * 100.0, false);
                    v.drive = 2;
                    v.drive_gain = k;
                    v.drive_sq = n;
                    v.drive_amt = drive_amt;
                    lift = drive_lift(k, n, false, body808::DRIVE_REF);
                }
                v.body_mul = t60(decay * decay_share(0.0, lift), rate);
                norm = 0.32;
            }
            SNARE => {
                // The shell: two modes (1 : 1.59, a head's first two),
                // ringing by Ring, dropping a little after a hard hit; the
                // wires: noise through a band-pass (Tone) and a high-pass,
                // as loud as Snappy.
                v.body_on = true;
                v.f0 = 185.0 * ratio;
                let shell = 1.0 - nz * 0.4;
                v.lvl_a = 0.65 * shell;
                v.ratio_b = 1.59;
                v.lvl_b = 0.4 * shell;
                let ring = g(field::RING) / 100.0;
                v.body_mul = t60(decay * (0.2 + 0.6 * ring), rate);
                v.body_b_mul = t60(decay * (0.12 + 0.4 * ring), rate);
                v.pitch = Pitch::Exp {
                    semis: 2.5 * bite,
                    frames: 0.03 * rate,
                };
                v.noise_lvl = nz * (0.7 + 0.3 * bite);
                v.n_bands = 1;
                v.bp[0].set((1500.0 + tone * 6000.0) * bright, 0.6, rate);
                v.bp_out[0] = Out::Bp;
                v.hp[0].set(700.0, 0.7, rate);
                v.hp_out[0] = Out::Hp;
                v.band_lvl[0] = 1.0;
                // The wires start a little under the shell and wires
                // together.
                v.env_mul[0] = t60(decay * decay_share(5.0, lift), rate);
                norm = 0.75;
            }
            CLAP => {
                // Bursts of noise through a band-pass, a little uneven,
                // then the room's tail; Sharp shortens each burst.
                v.clap = true;
                v.noise_lvl = 1.0;
                v.bursts_left = g(field::BURSTS).round().clamp(1.0, 8.0) as u32;
                v.gap = (g(field::BURST_GAP) * 0.001 * rate).max(1.0) as u32;
                v.burst_mul = t60(0.004 + (1.0 - snap) * 0.012, rate);
                v.tail_lvl = g(field::ROOM) / 100.0;
                // The tail starts under the bursts' peak: it falls the rest
                // of the way in the label.
                // (Three quarters of it: the bursts are short, so the
                // loudest few milliseconds are under their peak.)
                let head = -15.0 * (v.tail_lvl / (1.0 + v.tail_lvl)).max(1e-3).log10();
                v.tail = 1.0;
                v.tail_mul = t60(decay * decay_share(head, lift), rate);
                v.n_bands = 1;
                v.bp[0].set((800.0 + tone * 2400.0) * bright, 1.2, rate);
                v.bp_out[0] = Out::Bp;
                v.hp[0].set(400.0, 0.7, rate);
                v.hp_out[0] = Out::Hp;
                v.band_lvl[0] = 1.0;
                v.click_lvl = 0.0;
                norm = 1.75;
            }
            CLOSED_HAT | OPEN_HAT | CYMBAL => {
                // The 808's six squares, spread by Spread around their
                // middle (50 is the circuit's), against noise.
                v.n_metal = 6;
                for (k, f) in metal_hz(g(field::SPREAD), ratio).into_iter().enumerate() {
                    v.dt_m[k] = f / rate;
                    v.ph_m[k] = v.rng.unit();
                }
                v.metal_lvl = (1.0 - nz) / 3.0;
                v.noise_lvl = nz * 0.6;
                if m == CYMBAL {
                    // Three bands, as the 808's cymbal: the low band-pass
                    // short, the high one's two outputs longer, the top
                    // through a high-pass the longest. Tone tilts them.
                    v.n_bands = 3;
                    let [f_lo, f_hi] = cymbal_bands(tone, ratio, bright);
                    v.bp[0].set(f_lo, 1.0, rate);
                    v.bp_out[0] = Out::Bp;
                    v.bp[1].set(f_hi, 1.0, rate);
                    v.bp_out[1] = Out::Bp;
                    v.top_from_mid = true;
                    v.hp[2].set(f_hi * 1.25, 0.7, rate);
                    v.hp_out[2] = Out::Hp;
                    v.band_lvl = [0.7 * (1.2 - tone), 0.8, 1.1 * (0.4 + tone)];
                    // The top band starts under the three together.
                    let top = decay * decay_share(4.0, lift);
                    v.env_mul = [t60(top * 0.3, rate), t60(top * 0.6, rate), t60(top, rate)];
                    norm = 2.5;
                } else {
                    // A hat: the high band-pass and a high-pass after it.
                    v.n_bands = 1;
                    let f = cymbal_bands(tone, ratio, bright)[1];
                    v.bp[0].set(f, 0.9, rate);
                    v.bp_out[0] = Out::Bp;
                    v.hp[0].set(f * 0.85, 0.7, rate);
                    v.hp_out[0] = Out::Hp;
                    v.band_lvl[0] = 1.0;
                    v.env_mul[0] = t60(decay * decay_share(0.0, lift), rate);
                    norm = if m == CLOSED_HAT { 7.0 } else { 4.4 };
                }
                v.click_lvl = snap * 0.25 * bite * bite;
                click_hp(&mut v, 5000.0 * bright);
            }
            COWBELL => {
                // Two of the squares (540 and 800 Hz) through a band-pass,
                // a sharp attack over a longer ring.
                v.n_metal = 2;
                v.dt_m[0] = 540.0 * ratio / rate;
                v.dt_m[1] = 800.0 * ratio / rate;
                v.metal_lvl = 0.5 * (1.0 - nz * 0.5);
                v.noise_lvl = nz * 0.3;
                v.n_bands = 2;
                let f = 2640.0 * ratio * (0.7 + 0.6 * tone) * bright.sqrt();
                for b in 0..2 {
                    v.bp[b].set(f, 1.5, rate);
                    v.bp_out[b] = Out::Bp;
                }
                v.band_lvl = [0.6, 0.4, 0.0];
                let head = -20.0 * (0.4f32 / 1.0).log10();
                v.env_mul = [
                    t60(0.03, rate),
                    t60(decay * decay_share(head, lift), rate),
                    1.0,
                ];
                norm = 2.75;
            }
            TOM | CONGA => {
                // A head's modes, struck at Strike (centre to edge) by a
                // stick as hard as Tone and the hit; a hard hit bends the
                // head down by Bend over Bend time.
                let conga = m == CONGA;
                let strike = 0.85 * g(field::STRIKE) / 100.0;
                let f0 = if conga { 200.0 } else { 120.0 } * ratio;
                v.modal = true;
                v.n_modes = if conga { 6 } else { 8 };
                let base = decay * decay_share(0.0, lift);
                let mut total = 0.0;
                for (k, &(mm, zero)) in MEMBRANE.iter().take(v.n_modes).enumerate() {
                    let r = zero / MEMBRANE[0].1;
                    // A conga's open shell pulls its modes nearer harmonic.
                    let r = if conga { 1.0 + (r - 1.0) * 0.92 } else { r };
                    v.ratios[k] = r;
                    let place = bessel_j(mm, zero * strike).abs();
                    // A harder hit drives the higher modes harder.
                    let w = place * if mm == 0 && k > 0 { 0.6 } else { 1.0 } / r.powf(0.7)
                        * bite.min(1.0).powf((r - 1.0) * 2.0);
                    let t = (base * r.powf(if conga { -0.9 } else { -1.3 })).max(0.01);
                    v.bank.set(k, f0 * r, t, w, rate);
                    total += w;
                }
                v.bank.set_len(v.n_modes);
                v.modal_mul = t60(base, rate);
                let hard = (0.25 + 0.75 * tone) * bite;
                let len = stick_len(hard, if conga { 0.003 } else { 0.004 }, f0, rate);
                v.stick.start(len as u32, std::f32::consts::FRAC_PI_2 / len / total.max(0.3));
                v.bend = g(field::SWEEP) * (0.3 + 0.7 * vel * vel);
                v.bend_frames = g(field::SWEEP_TIME) * 0.001 * rate;
                v.f0 = f0;
                if conga {
                    // The slap: the stick shorter and a noise burst.
                    v.click_lvl = snap * 0.7 * bite * bite;
                    click_hp(&mut v, 2500.0 * bright);
                    v.click_mul = t60(0.012, rate);
                }
                if !conga {
                    v.click_lvl = snap * 0.9 * bite * bite * bite;
                    v.click_mul = t60(0.006, rate);
                }
                v.modal_lvl = 1.0;
                norm = if conga { 0.75 } else { 0.73 };
            }
            WOODBLOCK | BELL => {
                // A bar's modes (woodblock) or a hand bell's, struck at
                // Strike by a stick as hard as Tone.
                let bell = m == BELL;
                let pos = g(field::STRIKE) / 100.0;
                let f0 = if bell { 600.0 } else { 800.0 } * ratio;
                v.modal = true;
                let base = decay * decay_share(0.0, lift);
                let mut total = 0.0;
                if bell {
                    v.n_modes = BELL_MODES.len();
                    for (k, &(r, lv, dr)) in BELL_MODES.iter().enumerate() {
                        v.ratios[k] = r;
                        let w = lv
                            * crate::modal::strike_place(k, pos)
                            * bite.min(1.0).powf((r - 1.0) * 0.5);
                        v.bank.set(k, f0 * r, base * dr, w / r.sqrt(), rate);
                        total += w;
                    }
                } else {
                    v.n_modes = BAR.len();
                    for (k, &r) in BAR.iter().enumerate() {
                        v.ratios[k] = r;
                        let w = crate::modal::strike_place(k, pos) / r.sqrt()
                            * bite.min(1.0).powf(r - 1.0);
                        v.bank.set(k, f0 * r, base / r.powf(0.7), w, rate);
                        total += w;
                    }
                }
                v.bank.set_len(v.n_modes);
                v.modal_mul = t60(base, rate);
                let hard = (0.3 + 0.7 * tone) * bite;
                let len = stick_len(hard, 0.0012, f0, rate);
                v.stick.start(len as u32, std::f32::consts::FRAC_PI_2 / len / total.max(0.3));
                v.f0 = f0;
                v.click_lvl = snap * 0.4 * bite * bite;
                click_hp(&mut v, 3000.0 * bright);
                v.modal_lvl = 1.0;
                norm = if bell { 0.7 } else { 0.85 };
            }
            RIM => {
                // Two sines (the rim's ring) and a band-passed crack.
                v.body_on = true;
                v.f0 = 500.0 * ratio;
                v.lvl_a = 0.6;
                v.ratio_b = 3.3;
                v.lvl_b = 0.4 * bite.min(1.0);
                v.body_mul = t60(decay * decay_share(0.0, lift), rate);
                v.body_b_mul = t60(decay * 0.7, rate);
                v.noise_lvl = nz * 0.8 * bite * bite;
                v.n_bands = 1;
                v.bp[0].set((2000.0 + tone * 3000.0) * bright, 2.0, rate);
                v.bp_out[0] = Out::Bp;
                v.band_lvl[0] = 1.0;
                v.env_mul[0] = t60(decay, rate);
                v.click_lvl = snap * 0.8 * bite * bite;
                v.click_mul = t60(0.002, rate);
                click_hp(&mut v, 2500.0 * bright);
                norm = 0.82;
            }
            ZAP => {
                v.body_on = true;
                v.f0 = 90.0 * ratio;
                v.lvl_a = 1.0;
                v.shape = tone * 3.0 * bite * bite;
                v.pitch = Pitch::Exp {
                    semis: g(field::SWEEP) * (0.6 + 0.4 * bite),
                    frames: g(field::SWEEP_TIME) * 0.001 * rate,
                };
                v.body_mul = t60(decay * decay_share(0.0, lift), rate);
                v.noise_lvl = nz * 0.3;
                v.n_bands = 1;
                v.bp[0].set(3000.0 * bright, 0.7, rate);
                v.bp_out[0] = Out::Hp;
                v.band_lvl[0] = 1.0;
                v.env_mul[0] = t60(decay * 0.5, rate);
                v.click_lvl = 0.0;
                norm = 0.35;
            }
            FM_PERC => {
                // A sine modulated at Ratio; Tone and the hit set the
                // index, which falls with the sweep time.
                v.body_on = true;
                v.f0 = 220.0 * ratio;
                v.lvl_a = 1.0;
                v.fm_ratio = g(field::RATIO);
                v.fm_index = (0.5 + tone * 8.0) * bite;
                v.fm_env = 1.0;
                v.fm_mul = (-4.6 / (g(field::SWEEP_TIME) * 0.001 * rate).max(1.0)).exp();
                v.pitch = Pitch::Exp {
                    semis: g(field::SWEEP),
                    frames: g(field::SWEEP_TIME) * 0.001 * rate,
                };
                v.body_mul = t60(decay * decay_share(0.0, lift), rate);
                v.noise_lvl = nz * 0.3;
                v.n_bands = 1;
                v.bp[0].set(5000.0, 0.7, rate);
                v.bp_out[0] = Out::Hp;
                v.band_lvl[0] = 1.0;
                v.env_mul[0] = t60(decay * 0.5, rate);
                v.click_lvl = 0.0;
                norm = 0.35;
            }
            _ => {
                // The shaker: noise through a band-pass, swelling over
                // Attack, falling over Decay, beads by Grain.
                v.shaker = true;
                v.noise_lvl = 1.0;
                v.n_bands = 1;
                let f = 5500.0 * ratio * (tone - 0.5).exp2() * bright;
                v.bp[0].set(f, 1.1, rate);
                v.bp_out[0] = Out::Bp;
                v.hp[0].set(2000.0, 0.7, rate);
                v.hp_out[0] = Out::Hp;
                v.band_lvl[0] = 1.0;
                let attack = g(field::ATTACK) * 0.001 / bite.max(0.3);
                v.swell_step = 1.0 / (attack * rate).max(1.0);
                v.env_mul[0] = t60(decay * decay_share(0.0, lift), rate);
                v.grain = g(field::GRAIN) / 100.0;
                v.bead_mul = t60(0.0015, rate);
                v.bead_odds = 1800.0 / rate;
                v.click_lvl = 0.0;
                norm = 1.05;
            }
        }
        if v.body_on {
            v.inc = v.f0 * (v.pitch.at(0.0) / 12.0).exp2() * v.inv_rate;
            v.dinc = 0.0;
            v.set_step(rate);
        }
        if v.modal && v.bend > 0.0 {
            v.retune(0.0, rate);
        }
        if v.drive == 1 {
            v.drive_in *= norm;
        }
        v.gain = amp * norm;
        v.pan(g(field::PAN) / 100.0);
        v
    }

    /// Sets the pan: equal power, as the first engine did.
    pub(super) fn pan(&mut self, pan: f32) {
        self.gain_l = self.gain * (1.0 - pan).max(0.0).sqrt() * std::f32::consts::FRAC_1_SQRT_2;
        self.gain_r = self.gain * (1.0 + pan).max(0.0).sqrt() * std::f32::consts::FRAC_1_SQRT_2;
    }

    /// The body's phase step over the next control step: from where it is
    /// to where the pitch curve will be at the step's end.
    fn set_step(&mut self, _rate: f32) {
        let t = (self.frames + CTRL) as f32;
        let to = self.f0 * (self.pitch.at(t) / 12.0).exp2() * self.inv_rate;
        self.dinc = (to - self.inc) / CTRL as f32;
    }

    /// The modes at the head's bend `t` frames into the hit.
    fn retune(&mut self, t: f32, rate: f32) {
        let semis = self.bend * (-4.605_17 * t / self.bend_frames.max(1.0)).exp();
        let k = (semis / 12.0).exp2();
        for i in 0..self.n_modes {
            self.bank.retune(i, self.f0 * self.ratios[i] * k, rate);
        }
    }

    /// Starts the fade out over `frames` frames (a choke, a replaced hit,
    /// a note let go, AllOff). A hit already fading keeps the quicker fade.
    pub(super) fn fade_out(&mut self, frames: f32) {
        let step = 1.0 / frames.max(1.0);
        if self.state == State::Kill {
            self.fade_step = self.fade_step.max(step);
        } else {
            self.state = State::Kill;
            self.fade = 0.0;
            self.fade_step = step;
        }
    }

    /// One frame.
    #[inline]
    pub(super) fn tick(&mut self, rate: f32) -> f32 {
        let mut out = 0.0;
        if self.frames.is_multiple_of(CTRL) && self.frames > 0 {
            if self.body_on {
                self.set_step(rate);
            }
            if self.modal && self.bend > 0.0 {
                self.retune(self.frames as f32, rate);
            }
        }
        if self.body_on {
            let inc = self.inc;
            self.inc += self.dinc;
            self.ph_a += inc;
            if self.ph_a >= 1.0 {
                self.ph_a -= 1.0;
            }
            let mut s = if self.fm_index > 0.0 {
                self.fm_ph += inc * self.fm_ratio;
                if self.fm_ph >= 1.0 {
                    self.fm_ph -= self.fm_ph.floor();
                }
                let ix = self.fm_index * (0.15 + 0.85 * self.fm_env);
                self.fm_env *= self.fm_mul;
                sin_turns(self.ph_a + ix * sin_turns(self.fm_ph) * 0.159_155)
            } else {
                sin_turns(self.ph_a)
            };
            if self.shape > 0.0 {
                s = s * (1.0 + self.shape) / (1.0 + self.shape * s.abs());
            }
            let mut b = self.lvl_a * s * self.body;
            self.body *= self.body_mul;
            if self.lvl_b > 0.0 {
                self.ph_b += inc * self.ratio_b;
                if self.ph_b >= 1.0 {
                    self.ph_b -= self.ph_b.floor();
                }
                b += self.lvl_b * sin_turns(self.ph_b) * self.body_b;
                self.body_b *= self.body_b_mul;
            }
            if self.lp_a > 0.0 {
                self.lp = b + self.lp_a * (self.lp - b);
                b = self.lp;
            }
            if self.trig > 0 {
                // The trigger pulse leaking past the filter: the click.
                self.trig -= 1;
                b += self.trig_lvl;
            }
            out += b;
        }
        if self.n_bands > 0 {
            let mut x = 0.0;
            if self.n_metal > 0 {
                let mut m = 0.0;
                for k in 0..self.n_metal {
                    m += metal_square(&mut self.ph_m[k], self.dt_m[k]);
                }
                self.metal_y += self.metal_a * (m - self.metal_y);
                x += self.metal_y * self.metal_lvl;
            }
            if self.noise_lvl > 0.0 {
                let mut n = self.rng.bipolar() * self.noise_lvl;
                if self.shaker {
                    // Beads: short spikes at random over a bed of noise.
                    if self.rng.unit() < self.bead_odds {
                        self.bead = 1.0;
                    }
                    let e = (1.0 - self.grain) + self.grain * 2.5 * self.bead;
                    self.bead *= self.bead_mul;
                    n *= e * (1.0 - fade_gain(self.swell));
                    if self.swell < 1.0 {
                        self.swell = (self.swell + self.swell_step).min(1.0);
                    }
                }
                x += n;
            }
            let mut mid = 0.0;
            for b in 0..self.n_bands {
                let y = if b == 2 && self.top_from_mid {
                    self.hp[2].run(self.hp_out[2], mid)
                } else {
                    let y = self.bp[b].run(self.bp_out[b], x);
                    if b == 1 {
                        mid = y;
                    }
                    self.hp[b].run(self.hp_out[b], y)
                };
                let mut env = self.env[b];
                self.env[b] *= self.env_mul[b];
                if self.clap && b == 0 {
                    if self.bursts_left > 0 && self.next_burst == 0 {
                        self.burst = 1.0;
                        self.bursts_left -= 1;
                        // Each gap a little uneven, as hands are.
                        let jitter = 1.0 + 0.15 * self.rng.bipolar();
                        self.next_burst = ((self.gap as f32 * jitter) as u32).max(1);
                    }
                    self.next_burst = self.next_burst.saturating_sub(1);
                    env = self.burst + self.tail_lvl * self.tail;
                    self.burst *= self.burst_mul;
                    self.tail *= self.tail_mul;
                }
                out += y * env * self.band_lvl[b];
            }
        }
        if self.modal {
            out += self.bank.tick(self.stick.next());
            self.modal_env *= self.modal_mul;
        }
        if self.click_lvl > 0.0 && self.click > SILENT {
            let n = self.rng.bipolar();
            // A one-pole high-pass.
            self.click_y = self.click_hp_a * (self.click_y + n - self.click_x);
            self.click_x = n;
            out += self.click_y * self.click * self.click_lvl;
            self.click *= self.click_mul;
        }
        match self.drive {
            0 => {}
            1 => {
                out = drive_n(out * self.drive_in, self.drive_gain, self.drive_sq, self.drive_amt)
                    / self.drive_in
            }
            _ => {
                let r = body808::DRIVE_REF * self.drive_in;
                out = body808::drive(out * r, self.drive_gain, self.drive_sq, false) / r
            }
        }
        if self.state == State::Kill {
            out *= fade_gain(self.fade);
            self.fade += self.fade_step;
            if self.fade >= 1.0 {
                self.state = State::Free;
            }
        }
        self.frames = self.frames.wrapping_add(1);
        if self.frames.is_multiple_of(64) && self.state == State::On && self.silent() {
            self.state = State::Free;
        }
        out
    }

    /// About how loud the hit is now: its parts' envelopes times their
    /// levels and the hit's gain.
    pub(super) fn level(&self) -> f32 {
        let mut parts = self.body * self.lvl_a.max(self.lvl_b) + self.click * self.click_lvl;
        if self.n_bands > 0 {
            let src = self.metal_lvl + self.noise_lvl;
            for b in 0..self.n_bands {
                parts += self.env[b] * self.band_lvl[b] * src;
            }
        }
        if self.clap {
            parts += self.burst + self.tail * self.tail_lvl;
        }
        if self.modal {
            parts += self.modal_env * self.modal_lvl;
        }
        parts * self.gain * std::f32::consts::SQRT_2
    }

    fn silent(&self) -> bool {
        if self.stick.active() || (self.clap && self.bursts_left > 0) || self.trig > 0 {
            return false;
        }
        if self.shaker && self.swell < 1.0 {
            return false;
        }
        let mut parts = 0.0;
        if self.body_on {
            parts += self.body * self.lvl_a + self.body_b * self.lvl_b;
        }
        parts += self.click * self.click_lvl;
        if self.n_bands > 0 {
            let src = self.metal_lvl + self.noise_lvl;
            for b in 0..self.n_bands {
                parts += self.env[b] * self.band_lvl[b] * src;
            }
        }
        if self.clap {
            parts += self.burst + self.tail * self.tail_lvl;
        }
        if self.modal {
            parts += self.modal_env * self.modal_lvl;
        }
        parts < SILENT
    }

    /// The model's name.
    pub(super) fn model(&self) -> &'static str {
        MODELS[usize::from(self.model) % MODELS.len()]
    }

    /// For the hit's picture: (body level, its share left a frame), the
    /// noise and metal's, the click's, the sweep (semitones, share left),
    /// the clap's burst starts (seconds) and their share, the drive.
    pub(super) fn shape(&self, rate: f32) -> super::HitShape {
        let src = self.metal_lvl * (self.n_metal as f32).sqrt() + self.noise_lvl;
        let (noise, noise_mul) = if self.n_bands > 0 {
            let b = (0..self.n_bands)
                .max_by(|&a, &b| self.env_mul[a].total_cmp(&self.env_mul[b]))
                .unwrap_or(0);
            (src * self.band_lvl[b], self.env_mul[b])
        } else {
            (0.0, 1.0)
        };
        let (body, body_mul) = if self.body_on {
            (self.lvl_a + self.lvl_b, self.body_mul)
        } else if self.modal {
            (1.0, self.modal_mul)
        } else {
            (0.0, 1.0)
        };
        let (sweep, sweep_mul) = match self.pitch {
            Pitch::Exp { semis, frames } => (semis, (-4.605_17 / frames.max(1.0)).exp()),
            // The sigh (the punch is over in 6 ms, too quick to draw).
            Pitch::Drop808 {
                sigh, sigh_frames, ..
            } => (sigh, (-4.605_17 / sigh_frames.max(1.0)).exp()),
            Pitch::Flat if self.modal => {
                (self.bend, (-4.605_17 / self.bend_frames.max(1.0)).exp())
            }
            Pitch::Flat => (0.0, 1.0),
        };
        let bursts = self.clap.then(|| {
            let mut starts = [0.0f32; 8];
            for (k, b) in starts.iter_mut().enumerate() {
                *b = k as f32 * self.gap as f32 / rate;
            }
            super::Bursts {
                starts,
                count: self.bursts_left.min(8) as usize,
                mul: self.burst_mul,
                tail: self.tail_lvl,
            }
        });
        let (noise, noise_mul) = if self.clap {
            (self.noise_lvl, self.tail_mul)
        } else {
            (noise, noise_mul)
        };
        super::HitShape {
            model: self.model(),
            hz: if self.body_on || self.modal { self.f0 } else { 0.0 },
            body: (body, body_mul),
            noise: (noise, noise_mul),
            click: (self.click_lvl, self.click_mul),
            sweep: (sweep, sweep_mul),
            bursts,
            drive: if self.drive > 0 { self.drive_amt.max(0.01) } else { 0.0 },
        }
    }
}
