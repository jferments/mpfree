//! Drums' effects: each drum's own Punch, Distortion, EQ and Compressor,
//! its sends to the kit's Delay and Reverb, and those two.
//!
//! "The processing is part of the drum" (research, drums.md section 1): as
//! on the MPC's DrumSynth, every drum carries the same four effects, each
//! off until it is turned up (Jay, 19 Nov 2026: "Four on every drum"), and
//! the kit has two send effects. The kit's master drive or compressor is
//! the sound's own effects (`fx.N` lines in the kit's preset, played by
//! `mpfree_instruments::Sound`), so nothing here duplicates them.
//!
//! **Ids** (agreed with the voices' owner): drum `pad`'s effects are
//! `1024 + pad * 64 + field` ([`fx_id`], [`field`]), the kit's sends
//! `2048` up ([`kit`]). Their values live here, not in the voices' array.
//!
//! **How a drum is processed.** A drum with any effect or send up plays
//! its hits into its own mono bus instead of straight to the output
//! ([`Drums`](crate::Drums) asks [`KitFx::bus`]). The bus runs through, in
//! order:
//!
//! - **Punch** (−100 to +100 %) and **Sustain**: the Transient shaper's
//!   level-independent detector (`mpfree_fx::TransientShaper`, Speed 15 ms)
//!   on one channel: more or less of each hit's front, and of what rings
//!   after it.
//! - **Distortion** (0 to 48 dB), **Dist tone** and **Dist mix**: a soft
//!   clipper (`u - u³/3`, flat from `|u| = 1`) with first-order
//!   antiderivative antialiasing worked in `f64` (polynomial, no library
//!   call), level-matched so a hit peaking at −6 dBFS still peaks there.
//!   Tone below 0 closes a low-pass after it (20 kHz to 1.5 kHz); above 0 a
//!   high shelf before it (up to +12 dB at 2 kHz) makes the highs bite.
//! - **Low**, **Mid** at **Mid freq**, **High**: a 100 Hz shelf, a bell
//!   (Q 1) and an 8 kHz shelf, ±15 dB, designed as the EQ's matched filters
//!   (FX quality E3), gliding over 10 ms when turned.
//! - **Threshold**, **Ratio**, **Comp attack**, **Comp release**: the
//!   Compressor's Clean style on one channel (peak, 6 dB knee, its smooth
//!   decoupled detector and release calibration). Off at Ratio 1:1. It has
//!   no makeup of its own: an automatic one lifted a fast hit's first
//!   frames, which pass before the attack, by the whole makeup (a snare
//!   at −11.5 dBFS came out at +2.4 dBFS). Output brings a compressed drum
//!   back up.
//! - **Output** (−24 to +24 dB), then the pan the hit started with, then
//!   the **Delay send** and **Reverb send** (after the pan).
//!
//! Every stage is checked frame by frame and skipped while it would leave
//! the sound as it is, so an effect at zero costs nothing and a drum with
//! nothing up plays straight to the output, bit for bit as without this
//! module. A drum's bus sleeps once it has been silent for its stages'
//! rest and its output is quiet, and is reset then; the kit's Delay and
//! Reverb (`mpfree_fx::Delay` with ping-pong, `mpfree_fx::Room` with its
//! gate) sleep the same way. Every decision falls on a frame counted from
//! the sound, never on a block's edge, so the output is the same for any
//! block size. Nothing allocates after [`KitFx::prepare`].

use mpfree_fx::parts::{
    fast_db_to_gain, fast_level_db, flush, reduction, release_tau, Follow, FLOOR, GAP, LIMIT, SENS,
};
use mpfree_fx::{db_to_gain, Delay, Room, SvfCoefs, SvfMode};
use mpfree_node::{Ctx, Curve, Node, ParamInfo, Smooth, Unit};

use crate::drum_params::{self, PADS};

/// The first effect id.
pub const BASE: u16 = 1024;
/// Id stride between drums.
pub const STRIDE: u16 = 64;
/// The first kit send id.
pub const KIT: u16 = 2048;
/// One past the last id this module owns.
pub const END: u16 = 2080;

/// The id of effect `field` on drum `pad` (0 to 15).
pub const fn fx_id(pad: usize, field: u16) -> u16 {
    BASE + pad as u16 * STRIDE + field
}

/// Whether `id` is one of the effects' or the kit sends'.
pub const fn owns(id: u16) -> bool {
    id >= BASE && id < END
}

/// A drum's effects: add to its base ([`fx_id`]).
pub mod field {
    pub const PUNCH: u16 = 0;
    pub const SUSTAIN: u16 = 1;
    pub const DISTORTION: u16 = 2;
    pub const DIST_TONE: u16 = 3;
    pub const DIST_MIX: u16 = 4;
    pub const LOW: u16 = 5;
    pub const MID: u16 = 6;
    pub const MID_FREQ: u16 = 7;
    pub const HIGH: u16 = 8;
    pub const THRESHOLD: u16 = 9;
    pub const RATIO: u16 = 10;
    pub const COMP_ATTACK: u16 = 11;
    pub const COMP_RELEASE: u16 = 12;
    pub const OUTPUT: u16 = 13;
    pub const DELAY_SEND: u16 = 14;
    pub const REVERB_SEND: u16 = 15;
    /// Fields per drum.
    pub const COUNT: usize = 16;
}

/// The kit's send effects.
pub mod kit {
    pub const DELAY_TIME: u16 = 2048;
    pub const DELAY_FEEDBACK: u16 = 2049;
    pub const DELAY_TONE: u16 = 2050;
    pub const DELAY_LEVEL: u16 = 2051;
    pub const REVERB_SIZE: u16 = 2052;
    pub const REVERB_DECAY: u16 = 2053;
    pub const REVERB_TONE: u16 = 2054;
    pub const REVERB_GATE: u16 = 2055;
    pub const REVERB_LEVEL: u16 = 2056;
    /// Kit send settings.
    pub const COUNT: usize = 9;
}

/// The Delay time's choices: the delay's synced times (its Free left out).
pub const DELAY_TIMES: &[&str] = &[
    "1/32", "1/16T", "1/16", "1/16D", "1/8T", "1/8", "1/8D", "1/4T", "1/4", "1/4D", "1/2", "1 bar",
];

const GROUPS: [&str; PADS] = [
    "Pad 1 effects",
    "Pad 2 effects",
    "Pad 3 effects",
    "Pad 4 effects",
    "Pad 5 effects",
    "Pad 6 effects",
    "Pad 7 effects",
    "Pad 8 effects",
    "Pad 9 effects",
    "Pad 10 effects",
    "Pad 11 effects",
    "Pad 12 effects",
    "Pad 13 effects",
    "Pad 14 effects",
    "Pad 15 effects",
    "Pad 16 effects",
];

/// The group of the kit's sends.
pub const SENDS: &str = "Sends";

#[allow(clippy::too_many_arguments)]
const fn p(
    id: u16,
    name: &'static str,
    group: &'static str,
    min: f32,
    max: f32,
    default: f32,
    unit: Unit,
    curve: Curve,
) -> ParamInfo {
    ParamInfo {
        id,
        name,
        group,
        min,
        max,
        default,
        unit,
        curve,
        readonly: false,
    }
}

const fn pad_param(pad: usize, f: u16) -> ParamInfo {
    use field::*;
    let g = GROUPS[pad];
    let id = fx_id(pad, f);
    match f {
        PUNCH => p(
            id,
            "Punch",
            g,
            -100.0,
            100.0,
            0.0,
            Unit::Percent,
            Curve::Linear,
        ),
        SUSTAIN => p(
            id,
            "Sustain",
            g,
            -100.0,
            100.0,
            0.0,
            Unit::Percent,
            Curve::Linear,
        ),
        DISTORTION => p(id, "Distortion", g, 0.0, 48.0, 0.0, Unit::Db, Curve::Linear),
        DIST_TONE => p(
            id,
            "Dist tone",
            g,
            -100.0,
            100.0,
            0.0,
            Unit::Percent,
            Curve::Linear,
        ),
        DIST_MIX => p(
            id,
            "Dist mix",
            g,
            0.0,
            100.0,
            100.0,
            Unit::Percent,
            Curve::Linear,
        ),
        LOW => p(id, "Low", g, -15.0, 15.0, 0.0, Unit::Db, Curve::Linear),
        MID => p(id, "Mid", g, -15.0, 15.0, 0.0, Unit::Db, Curve::Linear),
        MID_FREQ => p(
            id,
            "Mid freq",
            g,
            100.0,
            10_000.0,
            1000.0,
            Unit::Hz,
            Curve::Log,
        ),
        HIGH => p(id, "High", g, -15.0, 15.0, 0.0, Unit::Db, Curve::Linear),
        THRESHOLD => p(
            id,
            "Threshold",
            g,
            -48.0,
            0.0,
            -18.0,
            Unit::Db,
            Curve::Linear,
        ),
        RATIO => p(id, "Ratio", g, 1.0, 20.0, 1.0, Unit::None, Curve::Log),
        COMP_ATTACK => p(id, "Comp attack", g, 0.1, 100.0, 10.0, Unit::Ms, Curve::Log),
        COMP_RELEASE => p(
            id,
            "Comp release",
            g,
            10.0,
            1000.0,
            120.0,
            Unit::Ms,
            Curve::Log,
        ),
        OUTPUT => p(id, "Output", g, -24.0, 24.0, 0.0, Unit::Db, Curve::Linear),
        DELAY_SEND => p(
            id,
            "Delay send",
            g,
            0.0,
            100.0,
            0.0,
            Unit::Percent,
            Curve::Linear,
        ),
        _ => p(
            id,
            "Reverb send",
            g,
            0.0,
            100.0,
            0.0,
            Unit::Percent,
            Curve::Linear,
        ),
    }
}

const KIT_PARAMS: [ParamInfo; kit::COUNT] = [
    p(
        kit::DELAY_TIME,
        "Delay time",
        SENDS,
        0.0,
        (DELAY_TIMES.len() - 1) as f32,
        5.0,
        Unit::None,
        Curve::Choice(DELAY_TIMES),
    ),
    p(
        kit::DELAY_FEEDBACK,
        "Delay feedback",
        SENDS,
        0.0,
        90.0,
        30.0,
        Unit::Percent,
        Curve::Linear,
    ),
    p(
        kit::DELAY_TONE,
        "Delay tone",
        SENDS,
        500.0,
        20_000.0,
        6000.0,
        Unit::Hz,
        Curve::Log,
    ),
    p(
        kit::DELAY_LEVEL,
        "Delay level",
        SENDS,
        0.0,
        100.0,
        100.0,
        Unit::Percent,
        Curve::Linear,
    ),
    p(
        kit::REVERB_SIZE,
        "Reverb size",
        SENDS,
        0.0,
        100.0,
        45.0,
        Unit::Percent,
        Curve::Linear,
    ),
    p(
        kit::REVERB_DECAY,
        "Reverb decay",
        SENDS,
        100.0,
        5000.0,
        800.0,
        Unit::Ms,
        Curve::Log,
    ),
    p(
        kit::REVERB_TONE,
        "Reverb tone",
        SENDS,
        0.0,
        100.0,
        50.0,
        Unit::Percent,
        Curve::Linear,
    ),
    p(
        kit::REVERB_GATE,
        "Reverb gate",
        SENDS,
        0.0,
        600.0,
        0.0,
        Unit::Ms,
        Curve::Linear,
    ),
    p(
        kit::REVERB_LEVEL,
        "Reverb level",
        SENDS,
        0.0,
        100.0,
        100.0,
        Unit::Percent,
        Curve::Linear,
    ),
];

/// How many settings this module has.
pub const N_FX: usize = PADS * field::COUNT + kit::COUNT;

const fn build() -> [ParamInfo; N_FX] {
    let mut out = [KIT_PARAMS[0]; N_FX];
    let mut i = 0;
    while i < PADS * field::COUNT {
        out[i] = pad_param(i / field::COUNT, (i % field::COUNT) as u16);
        i += 1;
    }
    let mut k = 0;
    while k < kit::COUNT {
        out[PADS * field::COUNT + k] = KIT_PARAMS[k];
        k += 1;
    }
    out
}

// A constant (not a static) so `ALL` can be built from it at compile time.
#[allow(clippy::large_const_arrays)]
const TABLE: [ParamInfo; N_FX] = build();

/// The effects' settings: Pad 1 effects to Pad 16 effects, then Sends.
pub static PARAMS: [ParamInfo; N_FX] = TABLE;

const fn concat() -> [ParamInfo; drum_params::N_PARAMS + N_FX] {
    let mut out = [TABLE[0]; drum_params::N_PARAMS + N_FX];
    let mut i = 0;
    while i < drum_params::N_PARAMS {
        out[i] = drum_params::TABLE[i];
        i += 1;
    }
    let mut k = 0;
    while k < N_FX {
        out[drum_params::N_PARAMS + k] = TABLE[k];
        k += 1;
    }
    out
}

/// Every setting of [`Drums`](crate::Drums): the voices'
/// ([`drum_params::PARAMS`]), then the effects' ([`PARAMS`]).
pub static ALL: [ParamInfo; drum_params::N_PARAMS + N_FX] = concat();

/// The index into [`PARAMS`] of one of this module's ids.
const fn index(id: u16) -> Option<usize> {
    if id >= BASE && id < BASE + PADS as u16 * STRIDE {
        let pad = ((id - BASE) / STRIDE) as usize;
        let f = ((id - BASE) % STRIDE) as usize;
        if f < field::COUNT {
            return Some(pad * field::COUNT + f);
        }
        None
    } else if id >= KIT && ((id - KIT) as usize) < kit::COUNT {
        Some(PADS * field::COUNT + (id - KIT) as usize)
    } else {
        None
    }
}

/// Frames the voices and effects work through at a time: the buses are
/// this long.
pub const CHUNK: usize = 128;
/// A bus sleeps once its output has been under this (−100 dB)...
const QUIET_OUT: f32 = 1e-5;
/// ... for this many frames in a row, as well as its input silent for its
/// stages' rest.
const QUIET_RUN: u32 = 256;
/// The voice reads a hit's settings at its start; these move over 20 ms.
const SMOOTH_MS: f32 = 20.0;
/// Pan follows a new hit over 5 ms.
const PAN_MS: f32 = 5.0;
/// An EQ band glides to a new curve over 10 ms.
const GLIDE_MS: f32 = 10.0;

/// The low shelf's and high shelf's corners, and the bell's width.
const LOW_HZ: f32 = 100.0;
const HIGH_HZ: f32 = 8000.0;
const SHELF_Q: f32 = 0.707;
const MID_Q: f32 = 1.0;

/// Distortion: where the curve flattens, as an input level at 0 dB
/// (+3.5 dBFS), and the peak level it keeps (−6 dBFS).
const DIST_HEADROOM: f32 = 1.5;
const DIST_REF: f32 = 0.5;
/// Dist tone's corners: the bright side's shelf before the curve, the dark
/// side's low-pass after it at its darkest.
const BITE_HZ: f32 = 2000.0;
const BITE_DB: f32 = 12.0;
const DARK_HZ: f32 = 1500.0;
const OPEN_HZ: f32 = 20_000.0;

/// The compressor's knee, as the Compressor's default.
const KNEE: f32 = 6.0;
/// Punch's and the compressor's detectors step once every this many frames
/// (a group counted from the drum's sound, never from a block's edge) and
/// ramp their gain over the group: a twelfth of the work of stepping every
/// frame, for a gain at most 4 frames (0.08 ms) behind.
const DETECT: u32 = 4;
/// The Transient shaper's Speed, ms (its default).
const SPEED_MS: f32 = 15.0;

/// The soft clipper: `u - u³/3` up to `|u| = 1`, then flat at ±2/3.
#[inline]
fn clip(u: f32) -> f32 {
    if u > 1.0 {
        2.0 / 3.0
    } else if u < -1.0 {
        -2.0 / 3.0
    } else {
        u - u * u * u * (1.0 / 3.0)
    }
}

/// The clipper's antiderivative, in `f64`: `u²/2 - u⁴/12`, then
/// `2|u|/3 - 1/4` (the two meet at 5/12).
#[inline]
fn clip_big(u: f64) -> f64 {
    let a = u.abs();
    if a > 1.0 {
        a * (2.0 / 3.0) - 0.25
    } else {
        let u2 = u * u;
        u2 * 0.5 - u2 * u2 * (1.0 / 12.0)
    }
}

/// The clipper averaged over the step from `v` to `u` (first-order
/// antiderivative antialiasing): `(F(u) - F(v)) / (u - v)`. Inside the
/// curve that is a polynomial, `(u+v)/2 - (u+v)(u²+v²)/12`, with no
/// division; past the knee on one side it is flat at ±2/3; only a step
/// across the knee divides, in `f64`, or takes the curve at the step's
/// middle when the step is too short to divide by.
#[inline(always)]
fn clip_step(u: f32, v: f32) -> f32 {
    if u.abs() <= 1.0 && v.abs() <= 1.0 {
        let s = u + v;
        s * 0.5 - s * (u * u + v * v) * (1.0 / 12.0)
    } else if u > 1.0 && v > 1.0 {
        2.0 / 3.0
    } else if u < -1.0 && v < -1.0 {
        -2.0 / 3.0
    } else {
        let d = f64::from(u) - f64::from(v);
        if d.abs() > 1e-5 {
            ((clip_big(f64::from(u)) - clip_big(f64::from(v))) / d) as f32
        } else {
            clip(0.5 * (u + v))
        }
    }
}

/// One state-variable filter stage on one channel: `mpfree_fx::Svf`'s
/// numbers (the same trapezoidal form and denormal flush), with half its
/// state, so a drum's EQ bands can run as one cascade.
#[derive(Clone, Copy, Debug)]
struct Mono {
    a1: f32,
    a2: f32,
    a3: f32,
    m0: f32,
    m1: f32,
    m2: f32,
    ic1: f32,
    ic2: f32,
}

impl Mono {
    fn new(c: SvfCoefs) -> Mono {
        let mut f = Mono {
            a1: 0.0,
            a2: 0.0,
            a3: 0.0,
            m0: 1.0,
            m1: 0.0,
            m2: 0.0,
            ic1: 0.0,
            ic2: 0.0,
        };
        f.set(c);
        f
    }

    #[inline]
    fn set(&mut self, c: SvfCoefs) {
        self.a1 = 1.0 / (1.0 + c.g * (c.g + c.k));
        self.a2 = c.g * self.a1;
        self.a3 = c.g * self.a2;
        self.m0 = c.m0;
        self.m1 = c.m1;
        self.m2 = c.m2;
    }

    fn reset(&mut self) {
        self.ic1 = 0.0;
        self.ic2 = 0.0;
    }

    #[inline(always)]
    fn run(&mut self, v0: f32) -> f32 {
        let (ic1, ic2) = (self.ic1, self.ic2);
        let v3 = v0 - ic2;
        let v1 = self.a1 * ic1 + self.a2 * v3;
        let v2 = ic2 + self.a2 * ic1 + self.a3 * v3;
        self.ic1 = flush(2.0 * v1 - ic1);
        self.ic2 = flush(2.0 * v2 - ic2);
        self.m0 * v0 + self.m1 * v1 + self.m2 * v2
    }
}

/// `buf` through the filters `f` in turn, in place, all in one pass with
/// their state in registers: the same numbers as one filter after another,
/// but the filters' chains overlap.
#[inline(always)]
fn cascade<const N: usize>(f: [&mut Mono; N], buf: &mut [f32]) {
    let mut m: [Mono; N] = std::array::from_fn(|j| *f[j]);
    for x in buf.iter_mut() {
        let mut v = *x;
        for fj in m.iter_mut() {
            v = fj.run(v);
        }
        *x = v;
    }
    for (dst, src) in f.into_iter().zip(m) {
        *dst = src;
    }
}

/// One drum's four effects, their output level and its pan and sends.
#[derive(Clone, Debug)]
struct Strip {
    rate: u32,
    // Punch: the Transient shaper on one channel.
    /// Frames into the detectors' group of [`DETECT`].
    phase: u32,
    punch_on: bool,
    punch: Smooth,
    sustain: Smooth,
    fast: Follow,
    slow: Follow,
    hang: Follow,
    drop: Follow,
    peak: f32,
    held: u32,
    hold: u32,
    fall: f32,
    /// The group's peak so far, and the gain ramping from and to.
    p_max: f32,
    p_from: f32,
    p_to: f32,
    // Distortion.
    dist_on: bool,
    /// The drive in dB and the wet share (Dist mix times the stage's own
    /// fade in and out).
    drive: Smooth,
    wet: Smooth,
    /// The clipper's last input (its antialiasing steps from it).
    adaa_u: f32,
    bite: Option<Mono>,
    dark: Option<Mono>,
    // EQ.
    bands: [Band; 3],
    // Compressor.
    comp_on: bool,
    thresh: Smooth,
    ratio: f32,
    att: f32,
    rel: f32,
    c_hold: f32,
    c_env: f32,
    c_max: f32,
    c_from: f32,
    c_to: f32,
    // Output, pan, sends.
    out: Smooth,
    pan_l: Smooth,
    pan_r: Smooth,
    send_d: Smooth,
    send_r: Smooth,
    // Sleep.
    asleep: bool,
    quiet_in: u32,
    quiet_out: u32,
}

/// One EQ band: its filter, the curve it glides to, and whether it runs.
#[derive(Clone, Copy, Debug)]
struct Band {
    on: bool,
    svf: Mono,
    now: SvfCoefs,
    to: SvfCoefs,
    left: u32,
    /// The gain `to` is for: 0 means the band ends off once there.
    gain: f32,
}

impl Band {
    fn new() -> Band {
        let flat = SvfCoefs {
            g: 0.1,
            k: 1.0,
            m0: 1.0,
            m1: 0.0,
            m2: 0.0,
        };
        Band {
            on: false,
            svf: Mono::new(flat),
            now: flat,
            to: flat,
            left: 0,
            gain: 0.0,
        }
    }

    /// Glides to the curve `c` for `gain`, over `frames`. A band that was
    /// off starts from the flat curve at the new corner, its memory
    /// cleared, so it fades in.
    fn aim(&mut self, c: SvfCoefs, flat: SvfCoefs, gain: f32, frames: u32) {
        if !self.on {
            if gain == 0.0 {
                self.gain = 0.0;
                return;
            }
            self.on = true;
            self.svf.reset();
            self.now = flat;
            self.svf.set(flat);
        }
        self.to = c;
        self.gain = gain;
        self.left = frames.max(1);
    }

    #[inline]
    fn run(&mut self, x: f32) -> f32 {
        if self.left > 0 {
            self.now = self.now.toward(self.to, 1.0 / self.left as f32);
            self.left -= 1;
            if self.left == 0 {
                self.now = self.to;
            }
            self.svf.set(self.now);
        }
        let y = self.svf.run(x);
        if self.left == 0 && self.gain == 0.0 {
            // Glided to flat: off from the next frame, memory cleared.
            self.on = false;
            self.svf.reset();
        }
        y
    }

    /// Lands on its curve at once (a sleeping bus), off if flat.
    fn settle(&mut self) {
        self.svf.reset();
        if self.left > 0 {
            self.now = self.to;
            self.left = 0;
            self.svf.set(self.now);
        }
        if self.gain == 0.0 {
            self.on = false;
        }
    }
}

impl Strip {
    fn new(rate: u32) -> Strip {
        let mut s = Strip {
            rate,
            phase: 0,
            punch_on: false,
            punch: Smooth::new(0.0),
            sustain: Smooth::new(0.0),
            fast: Follow::new(),
            slow: Follow::new(),
            hang: Follow::new(),
            drop: Follow::new(),
            peak: 0.0,
            held: 0,
            hold: 0,
            fall: 0.0,
            p_max: 0.0,
            p_from: 1.0,
            p_to: 1.0,
            dist_on: false,
            drive: Smooth::new(0.0),
            wet: Smooth::new(0.0),
            adaa_u: 0.0,
            bite: None,
            dark: None,
            bands: [Band::new(); 3],
            comp_on: false,
            thresh: Smooth::new(-18.0),
            ratio: 1.0,
            att: 0.0,
            rel: 0.0,
            c_hold: 0.0,
            c_env: 0.0,
            c_max: 0.0,
            c_from: 1.0,
            c_to: 1.0,
            out: Smooth::new(1.0),
            pan_l: Smooth::new(std::f32::consts::FRAC_1_SQRT_2),
            pan_r: Smooth::new(std::f32::consts::FRAC_1_SQRT_2),
            send_d: Smooth::new(0.0),
            send_r: Smooth::new(0.0),
            asleep: true,
            quiet_in: 0,
            quiet_out: 0,
        };
        s.prepare(rate);
        s
    }

    fn prepare(&mut self, rate: u32) {
        self.rate = rate.max(1);
        for sm in [
            &mut self.drive,
            &mut self.wet,
            &mut self.out,
            &mut self.send_d,
            &mut self.send_r,
        ] {
            sm.set_time(self.rate, SMOOTH_MS);
        }
        // The detectors and what they read step once a group.
        let det = (self.rate / DETECT).max(1);
        let fd = det as f32;
        for sm in [&mut self.punch, &mut self.sustain, &mut self.thresh] {
            sm.set_time(det, SMOOTH_MS);
        }
        self.pan_l.set_time(self.rate, PAN_MS);
        self.pan_r.set_time(self.rate, PAN_MS);
        self.fast.set(fd, 0.3, SPEED_MS * 4.0);
        self.slow.set(fd, SPEED_MS, SPEED_MS * 4.0);
        self.hang.set(fd, 0.3, SPEED_MS * 20.0);
        self.drop.set(fd, 0.3, SPEED_MS * 2.0);
        self.hold = (0.025 * fd) as u32;
        self.fall = (-1.0 / (0.005 * fd)).exp();
    }

    /// Aims the distortion at `db` of drive and `mix` (%): its wet share
    /// glides there (to nothing at 0 dB or a 0 % mix, after which the
    /// stage is off); a stage coming on starts at its drive, fading in.
    fn dist_aim(&mut self, db: f32, mix: f32) {
        let wet = if db > 0.0 { mix * 0.01 } else { 0.0 };
        self.drive.target = db;
        self.wet.target = wet;
        if wet > 0.0 && !self.dist_on {
            self.dist_on = true;
            self.drive.snap(db);
            self.adaa_u = 0.0;
        }
    }

    /// Sets the compressor's times from Comp attack and Comp release (ms).
    fn comp_times(&mut self, attack_ms: f32, release_ms: f32) {
        let fs = (self.rate / DETECT).max(1) as f32;
        let (a, r) = (attack_ms * 0.001, release_ms * 0.001);
        self.att = (-1.0 / (a * fs)).exp();
        self.rel = (-1.0 / (release_tau(a, false, r) * fs)).exp();
    }

    /// Frames of silence after which every stage that is on is as good as
    /// reset: the Transient shaper's and the Compressor's own rests, and
    /// 50 ms for the filters.
    fn rest_frames(&self, release_ms: f32) -> u32 {
        let mut ms: f32 = 50.0;
        if self.punch_on {
            ms = ms.max(25.0 + 50.0 + 10.0 * 20.0 * SPEED_MS);
        }
        if self.comp_on {
            ms = ms.max(10.0 * release_ms);
        }
        (ms * 0.001 * self.rate as f32) as u32
    }

    /// Whether a stage or a send would change the sound.
    fn live(&self) -> bool {
        self.punch_on
            || self.dist_on
            || self.bands.iter().any(|b| b.on)
            || self.comp_on
            || !(self.out.settled() && self.out.target == 1.0)
            || self.sends_on()
    }

    fn sends_on(&self) -> bool {
        !(self.send_d.settled() && self.send_d.target == 0.0)
            || !(self.send_r.settled() && self.send_r.target == 0.0)
    }

    /// Clears what the stages hold and lands every glide (a sleeping bus,
    /// or a reset): as good as the state a long silence leaves.
    fn rest(&mut self) {
        self.phase = 0;
        self.punch_rest();
        self.comp_rest();
        for sm in [
            &mut self.punch,
            &mut self.sustain,
            &mut self.drive,
            &mut self.wet,
            &mut self.thresh,
            &mut self.out,
            &mut self.pan_l,
            &mut self.pan_r,
            &mut self.send_d,
            &mut self.send_r,
        ] {
            sm.snap(sm.target);
        }
        self.punch_on = self.punch.target != 0.0 || self.sustain.target != 0.0;
        self.dist_on = self.wet.target > 0.0;
        self.adaa_u = 0.0;
        if let Some(f) = self.bite.as_mut() {
            f.reset();
        }
        if let Some(f) = self.dark.as_mut() {
            f.reset();
        }
        for b in &mut self.bands {
            b.settle();
        }
        self.comp_on = self.ratio > 1.0;
        self.quiet_in = 0;
        self.quiet_out = 0;
    }

    /// Punch's detector back on the floor, its gain at 1.
    fn punch_rest(&mut self) {
        for f in [
            &mut self.fast,
            &mut self.slow,
            &mut self.hang,
            &mut self.drop,
        ] {
            f.y = FLOOR;
        }
        self.peak = 0.0;
        self.held = 0;
        self.p_max = 0.0;
        self.p_from = 1.0;
        self.p_to = 1.0;
    }

    /// The compressor let go, its gain at 1.
    fn comp_rest(&mut self) {
        self.c_hold = 0.0;
        self.c_env = 0.0;
        self.c_max = 0.0;
        self.c_from = 1.0;
        self.c_to = 1.0;
    }

    /// The chunk `buf` through the four stages and the output level, in
    /// place, a stage at a time. A stage that turns off part way stops at
    /// that frame; the rest of the chunk passes it untouched.
    fn process(&mut self, buf: &mut [f32]) {
        let phase0 = self.phase;
        if self.punch_on {
            self.run_punch(buf, phase0);
        }
        if self.dist_on {
            self.run_dist(buf);
        }
        self.run_eq(buf);
        if self.comp_on {
            self.run_comp(buf, phase0);
        }
        if self.out.settled() {
            if self.out.value != 1.0 {
                let g = self.out.value;
                for x in buf.iter_mut() {
                    *x *= g;
                }
            }
        } else {
            for x in buf.iter_mut() {
                *x *= self.out.next();
            }
        }
        self.phase = ((phase0 as usize + buf.len()) % DETECT as usize) as u32;
    }

    /// Punch's detector, once a group: `a` is the peak of the frames since
    /// the last step, this one included. Sets the gain the group ramps to.
    #[inline(always)]
    fn punch_step(&mut self, a: f32) {
        let attack = glide(&mut self.punch);
        let sustain = glide(&mut self.sustain);
        if a >= self.peak {
            self.peak = a;
            self.held = self.hold;
        } else if self.held > 0 {
            self.held -= 1;
        } else {
            self.peak = flush(max(a, self.peak * self.fall));
        }
        let lv = max(fast_level_db(self.peak), FLOOR);
        let fast = self.fast.next(lv);
        self.slow.y = max(self.slow.next(lv), fast - GAP);
        let front = max(fast - self.slow.y, 0.0);
        let hang = self.hang.next(lv);
        self.drop.y = max(self.drop.next(lv), hang - GAP);
        let body = max(hang - self.drop.y, 0.0);
        let shaped = clamp((attack * front + sustain * body) * SENS, LIMIT);
        self.p_from = self.p_to;
        self.p_to = fast_db_to_gain(shaped);
    }

    /// At a group's end: whether Punch and Sustain have both landed at
    /// zero, in which case Punch is off from the next frame, its followers
    /// back on the floor.
    #[inline(always)]
    fn punch_done(&mut self) -> bool {
        // The targets first: while Punch is turned up the test ends there.
        let done = self.punch.target == 0.0
            && self.sustain.target == 0.0
            && self.punch.value == 0.0
            && self.sustain.value == 0.0
            && self.p_to == 1.0
            && self.p_from == 1.0;
        if done {
            self.punch_on = false;
            self.punch_rest();
        }
        done
    }

    /// One frame of Punch at `phase` in its group.
    #[inline(always)]
    fn punch_frame(&mut self, x: &mut f32, phase: u32) {
        self.p_max = max(self.p_max, x.abs());
        if phase == 0 {
            let a = self.p_max;
            self.p_max = 0.0;
            self.punch_step(a);
        }
        *x *= ramp(self.p_from, self.p_to, phase);
    }

    /// Punch: the detector steps on each group's first frame and the gain
    /// ramps over the group. Whole groups run four frames at a time; a
    /// group the chunk's edge cuts runs frame by frame, with the same
    /// numbers.
    fn run_punch(&mut self, buf: &mut [f32], phase0: u32) {
        let head = (((DETECT - phase0) % DETECT) as usize).min(buf.len());
        let (h, rest) = buf.split_at_mut(head);
        for (j, x) in h.iter_mut().enumerate() {
            let phase = phase0 + j as u32;
            self.punch_frame(x, phase);
            if phase == DETECT - 1 && self.punch_done() {
                return;
            }
        }
        let mut groups = rest.chunks_exact_mut(DETECT as usize);
        for g in &mut groups {
            let a = max(self.p_max, g[0].abs());
            self.p_max = max(max(g[1].abs(), g[2].abs()), g[3].abs());
            self.punch_step(a);
            apply_ramp(g, self.p_from, self.p_to);
            if self.punch_done() {
                return;
            }
        }
        for (j, x) in groups.into_remainder().iter_mut().enumerate() {
            self.punch_frame(x, j as u32);
        }
    }

    /// Distortion: the clipper's average over each step (first-order
    /// antiderivative antialiasing), level-matched, between its tone
    /// filters, blended by its wet share. Once the drive and the mix have
    /// landed their gains are worked out once a chunk, with the same
    /// numbers.
    fn run_dist(&mut self, buf: &mut [f32]) {
        if self.drive.settled() && self.wet.settled() {
            if self.wet.value == 0.0 {
                self.dist_off();
                return;
            }
            let (k, lm) = dist_gains(self.drive.value);
            let wet = self.wet.value;
            let mut v = self.adaa_u;
            for x in buf.iter_mut() {
                let dry = *x;
                let mut u = dry;
                if let Some(f) = self.bite.as_mut() {
                    u = f.run(u);
                }
                let u = k * u;
                let mut y = clip_step(u, v) * lm;
                v = u;
                if let Some(f) = self.dark.as_mut() {
                    y = f.run(y);
                }
                *x = dry + wet * (y - dry);
            }
            self.adaa_u = v;
            return;
        }
        for x in buf.iter_mut() {
            let (k, lm) = dist_gains(self.drive.next());
            let wet = self.wet.next();
            let dry = *x;
            let mut u = dry;
            if let Some(f) = self.bite.as_mut() {
                u = f.run(u);
            }
            let u = k * u;
            let mut y = clip_step(u, self.adaa_u) * lm;
            self.adaa_u = u;
            if let Some(f) = self.dark.as_mut() {
                y = f.run(y);
            }
            *x = dry + wet * (y - dry);
            if wet == 0.0 && self.wet.target == 0.0 {
                // Faded out: off from the next frame, its memory cleared.
                self.dist_off();
                return;
            }
        }
    }

    fn dist_off(&mut self) {
        self.dist_on = false;
        self.adaa_u = 0.0;
        if let Some(f) = self.bite.as_mut() {
            f.reset();
        }
        if let Some(f) = self.dark.as_mut() {
            f.reset();
        }
    }

    /// The EQ: the bands that are on, in one pass once none glides; while
    /// one glides, a band at a time, frame by frame for the gliding one.
    fn run_eq(&mut self, buf: &mut [f32]) {
        let [b0, b1, b2] = &mut self.bands;
        if (b0.on && b0.left > 0) || (b1.on && b1.left > 0) || (b2.on && b2.left > 0) {
            for b in [b0, b1, b2] {
                if !b.on {
                    continue;
                }
                if b.left == 0 {
                    cascade([&mut b.svf], buf);
                    continue;
                }
                for x in buf.iter_mut() {
                    *x = b.run(*x);
                    if !b.on {
                        break;
                    }
                }
            }
            return;
        }
        match (b0.on, b1.on, b2.on) {
            (false, false, false) => {}
            (true, false, false) => cascade([&mut b0.svf], buf),
            (false, true, false) => cascade([&mut b1.svf], buf),
            (false, false, true) => cascade([&mut b2.svf], buf),
            (true, true, false) => cascade([&mut b0.svf, &mut b1.svf], buf),
            (true, false, true) => cascade([&mut b0.svf, &mut b2.svf], buf),
            (false, true, true) => cascade([&mut b1.svf, &mut b2.svf], buf),
            (true, true, true) => cascade([&mut b0.svf, &mut b1.svf, &mut b2.svf], buf),
        }
    }

    /// The compressor's detector, once a group, on the group's peak `a`.
    #[inline(always)]
    fn comp_step(&mut self, a: f32) {
        let t = glide(&mut self.thresh);
        let want = reduction(fast_level_db(a), t, self.ratio, KNEE);
        self.c_hold = flush(max(want, self.rel * self.c_hold + (1.0 - self.rel) * want));
        self.c_env = flush(self.att * self.c_env + (1.0 - self.att) * self.c_hold);
        self.c_from = self.c_to;
        self.c_to = if self.c_env < 1e-4 {
            1.0
        } else {
            fast_db_to_gain(-self.c_env)
        };
    }

    /// At a group's end: whether the compressor is at 1:1 and has let go,
    /// in which case it is off from the next frame.
    #[inline(always)]
    fn comp_done(&mut self) -> bool {
        let done =
            self.ratio <= 1.0 && self.c_env < 1e-4 && self.c_hold < 1e-4 && self.c_from == 1.0;
        if done {
            self.comp_on = false;
            self.comp_rest();
        }
        done
    }

    #[inline(always)]
    fn comp_frame(&mut self, x: &mut f32, phase: u32) {
        self.c_max = max(self.c_max, x.abs());
        if phase == 0 {
            let a = self.c_max;
            self.c_max = 0.0;
            self.comp_step(a);
        }
        *x *= ramp(self.c_from, self.c_to, phase);
    }

    /// The compressor: the detector on each group's peak, the gain ramping
    /// over the group, whole groups four frames at a time (as Punch).
    fn run_comp(&mut self, buf: &mut [f32], phase0: u32) {
        let head = (((DETECT - phase0) % DETECT) as usize).min(buf.len());
        let (h, rest) = buf.split_at_mut(head);
        for (j, x) in h.iter_mut().enumerate() {
            let phase = phase0 + j as u32;
            self.comp_frame(x, phase);
            if phase == DETECT - 1 && self.comp_done() {
                return;
            }
        }
        let mut groups = rest.chunks_exact_mut(DETECT as usize);
        for g in &mut groups {
            let a = max(self.c_max, g[0].abs());
            self.c_max = max(max(g[1].abs(), g[2].abs()), g[3].abs());
            self.comp_step(a);
            apply_ramp(g, self.c_from, self.c_to);
            if self.comp_done() {
                return;
            }
        }
        for (j, x) in groups.into_remainder().iter_mut().enumerate() {
            self.comp_frame(x, j as u32);
        }
    }
}

/// The larger of two levels, as one compare (the detectors' peaks are
/// never NaN: a bus is finite).
#[inline(always)]
fn max(a: f32, b: f32) -> f32 {
    if a > b {
        a
    } else {
        b
    }
}

/// `x` within ±`lim`, as two compares.
#[inline(always)]
fn clamp(x: f32, lim: f32) -> f32 {
    if x > lim {
        lim
    } else if x < -lim {
        -lim
    } else {
        x
    }
}

/// A smoother's next value, without the glide's work once it has landed
/// (the same number [`Smooth::next`] gives then).
#[inline(always)]
fn glide(s: &mut Smooth) -> f32 {
    if s.settled() {
        s.value
    } else {
        s.next()
    }
}

/// A detector's gain at `phase` in its group, ramping from `from` to `to`.
#[inline(always)]
fn ramp(from: f32, to: f32, phase: u32) -> f32 {
    from + (to - from) * ((phase + 1) as f32 * (1.0 / DETECT as f32))
}

/// A whole group times its ramp: [`ramp`] at phases 0 to 3.
const _: () = assert!(DETECT == 4);
#[inline(always)]
fn apply_ramp(g: &mut [f32], from: f32, to: f32) {
    let d = to - from;
    g[0] *= from + d * 0.25;
    g[1] *= from + d * 0.5;
    g[2] *= from + d * 0.75;
    g[3] *= from + d * 1.0;
}

/// Distortion's input gain and level match for a drive in dB: a peak at
/// `DIST_REF` comes out at `DIST_REF`.
#[inline(always)]
fn dist_gains(db: f32) -> (f32, f32) {
    let g = fast_db_to_gain(db);
    (
        g * (1.0 / DIST_HEADROOM),
        DIST_REF / clip(g * (DIST_REF / DIST_HEADROOM)),
    )
}

/// A send effect's sleep: frames of silent input and of quiet output.
#[derive(Clone, Copy, Debug, Default)]
struct Rest {
    awake: bool,
    quiet_in: u32,
    quiet_out: u32,
}

/// Counters for tests and the Dev page.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FxStats {
    /// Drums' buses that fell asleep.
    pub sleeps: u64,
    /// Times the delay or reverb fell asleep.
    pub send_sleeps: u64,
}

/// The kit's effects: every drum's strip and bus, the send buses, and the
/// Delay and Reverb. See the module docs.
pub struct KitFx {
    rate: u32,
    values: [f32; N_FX],
    strips: Vec<Strip>,
    /// Each drum's mono bus for the chunk being worked.
    bus: Box<[[f32; CHUNK]; PADS]>,
    /// Whether a hit wrote into the drum's bus in this chunk.
    live: [bool; PADS],
    /// Whether the drum's hits go through its bus.
    routed: [bool; PADS],
    /// The delay's and reverb's send buses (left, right).
    sends: Box<[[f32; CHUNK]; 4]>,
    delay: Delay,
    reverb: Room,
    delay_rest: Rest,
    reverb_rest: Rest,
    delay_level: Smooth,
    reverb_level: Smooth,
    /// The block's context, for the delay's tempo.
    bpm: f64,
    beats: f64,
    frame: u64,
    playing: bool,
    stats: FxStats,
}

impl KitFx {
    pub fn new(rate: u32) -> KitFx {
        let mut values = [0.0; N_FX];
        for (v, p) in values.iter_mut().zip(PARAMS.iter()) {
            *v = p.default;
        }
        let mut k = KitFx {
            rate,
            values,
            strips: (0..PADS).map(|_| Strip::new(rate)).collect(),
            bus: Box::new([[0.0; CHUNK]; PADS]),
            live: [false; PADS],
            routed: [false; PADS],
            sends: Box::new([[0.0; CHUNK]; 4]),
            delay: Delay::new(),
            reverb: Room::new(),
            delay_rest: Rest::default(),
            reverb_rest: Rest::default(),
            delay_level: Smooth::new(1.0),
            reverb_level: Smooth::new(1.0),
            bpm: 120.0,
            beats: 0.0,
            frame: 0,
            playing: false,
            stats: FxStats::default(),
        };
        k.prepare(rate);
        k
    }

    /// Sets the rate and prepares the send effects (allocates their lines).
    pub fn prepare(&mut self, rate: u32) {
        self.rate = rate.max(1);
        self.delay.prepare(self.rate, CHUNK);
        self.reverb.prepare(self.rate, CHUNK);
        // Send effects: wet only, the delay bouncing left and right, the
        // reverb's lows cut so a kick's room stays clean.
        use mpfree_fx::ids::{delay as d, room as r};
        self.delay.set_param(d::MIX, 100.0);
        self.delay.set_param(d::PING_PONG, 1.0);
        self.reverb.set_param(r::MIX, 100.0);
        self.reverb.set_param(r::LOW_CUT, 150.0);
        self.delay_level.set_time(self.rate, SMOOTH_MS);
        self.reverb_level.set_time(self.rate, SMOOTH_MS);
        for s in &mut self.strips {
            s.prepare(self.rate);
        }
        for i in 0..N_FX {
            let id = PARAMS[i].id;
            self.apply(id, self.values[i]);
        }
        self.reset();
    }

    /// Ends every tail and clears every state; keeps the settings.
    pub fn reset(&mut self) {
        for s in &mut self.strips {
            s.rest();
            s.asleep = true;
        }
        for p in 0..PADS {
            self.routed[p] = self.strips[p].live();
        }
        self.live = [false; PADS];
        self.delay.reset();
        self.reverb.reset();
        self.delay_rest = Rest::default();
        self.reverb_rest = Rest::default();
        self.delay_level.snap(self.delay_level.target);
        self.reverb_level.snap(self.reverb_level.target);
    }

    /// Every setting back to its default (a kit loading).
    pub fn load_defaults(&mut self) {
        for p in PARAMS.iter() {
            self.set_param(p.id, p.default);
        }
    }

    /// Sets one of this module's settings, clamped; other ids are ignored.
    pub fn set_param(&mut self, id: u16, value: f32) {
        let Some(i) = index(id) else {
            return;
        };
        let v = PARAMS[i].clamp(value);
        self.values[i] = v;
        self.apply(id, v);
    }

    /// The value of one of this module's settings, NaN for another id.
    pub fn param(&self, id: u16) -> f32 {
        index(id).map_or(f32::NAN, |i| self.values[i])
    }

    fn v(&self, pad: usize, f: u16) -> f32 {
        self.values[pad * field::COUNT + f as usize]
    }

    fn kv(&self, id: u16) -> f32 {
        self.values[PADS * field::COUNT + (id - KIT) as usize]
    }

    fn apply(&mut self, id: u16, v: f32) {
        use mpfree_fx::ids::{delay as d, room as r};
        if id >= KIT {
            match id {
                kit::DELAY_TIME => self.delay.set_param(d::SYNC, v.round() + 1.0),
                kit::DELAY_FEEDBACK => self.delay.set_param(d::FEEDBACK, v),
                kit::DELAY_TONE => self.delay.set_param(d::TONE, v),
                kit::DELAY_LEVEL => self.delay_level.target = v * 0.01,
                kit::REVERB_SIZE => self.reverb.set_param(r::SIZE, v),
                kit::REVERB_DECAY => self.reverb.set_param(r::DECAY, v),
                kit::REVERB_TONE => self.reverb.set_param(r::DAMPING, 100.0 - v),
                kit::REVERB_GATE => self.reverb.set_param(r::GATE, v),
                _ => self.reverb_level.target = v * 0.01,
            }
            return;
        }
        let pad = ((id - BASE) / STRIDE) as usize;
        let f = (id - BASE) % STRIDE;
        let rate = self.rate;
        let glide = (GLIDE_MS * 0.001 * rate as f32) as u32;
        let (attack, release) = (
            self.v(pad, field::COMP_ATTACK),
            self.v(pad, field::COMP_RELEASE),
        );
        let mid_hz = self.v(pad, field::MID_FREQ);
        let mid_db = self.v(pad, field::MID);
        let (dist_db, dist_mix) = (self.v(pad, field::DISTORTION), self.v(pad, field::DIST_MIX));
        let s = &mut self.strips[pad];
        match f {
            field::PUNCH | field::SUSTAIN => {
                let sm = if f == field::PUNCH {
                    &mut s.punch
                } else {
                    &mut s.sustain
                };
                sm.target = v * 0.01;
                if v != 0.0 && !s.punch_on {
                    s.punch_on = true;
                }
            }
            field::DISTORTION | field::DIST_MIX => s.dist_aim(dist_db, dist_mix),
            field::DIST_TONE => {
                let t = v * 0.01;
                // A side's filter keeps its memory while it is turned, and
                // starts clear when it comes on.
                s.bite = (t > 0.0).then(|| {
                    let c =
                        SvfCoefs::bilinear(SvfMode::HighShelf, BITE_HZ, SHELF_Q, BITE_DB * t, rate);
                    let mut f = s.bite.unwrap_or(Mono::new(c));
                    f.set(c);
                    f
                });
                s.dark = (t < 0.0).then(|| {
                    let hz = OPEN_HZ * (DARK_HZ / OPEN_HZ).powf(-t);
                    let c = SvfCoefs::bilinear(SvfMode::LowPass, hz, SHELF_Q, 0.0, rate);
                    let mut f = s.dark.unwrap_or(Mono::new(c));
                    f.set(c);
                    f
                });
            }
            field::LOW => {
                let c = SvfCoefs::matched(SvfMode::LowShelf, LOW_HZ, SHELF_Q, v, rate);
                let flat = SvfCoefs::matched(SvfMode::LowShelf, LOW_HZ, SHELF_Q, 0.0, rate);
                s.bands[0].aim(c, flat, v, glide);
            }
            field::MID | field::MID_FREQ => {
                let c = SvfCoefs::matched(SvfMode::Bell, mid_hz, MID_Q, mid_db, rate);
                let flat = SvfCoefs::matched(SvfMode::Bell, mid_hz, MID_Q, 0.0, rate);
                s.bands[1].aim(c, flat, mid_db, glide);
            }
            field::HIGH => {
                let c = SvfCoefs::matched(SvfMode::HighShelf, HIGH_HZ, SHELF_Q, v, rate);
                let flat = SvfCoefs::matched(SvfMode::HighShelf, HIGH_HZ, SHELF_Q, 0.0, rate);
                s.bands[2].aim(c, flat, v, glide);
            }
            field::THRESHOLD => s.thresh.target = v,
            field::RATIO => {
                s.ratio = v;
                if v > 1.0 {
                    s.comp_on = true;
                }
            }
            field::COMP_ATTACK | field::COMP_RELEASE => s.comp_times(attack, release),
            field::OUTPUT => s.out.target = db_to_gain(v),
            field::DELAY_SEND => s.send_d.target = v * 0.01,
            _ => s.send_r.target = v * 0.01,
        }
        if s.asleep {
            // Nothing sounding through it: the settings land at once (no
            // glide from where they were), and the drum goes through its
            // bus only if something is up.
            s.rest();
            self.routed[pad] = s.live();
        } else if s.live() {
            self.routed[pad] = true;
        }
    }

    /// Takes the block's tempo and position (for the delay's synced time).
    pub fn begin(&mut self, ctx: &Ctx) {
        self.bpm = ctx.bpm;
        self.beats = ctx.beats;
        self.frame = ctx.frame;
        self.playing = ctx.playing;
    }

    /// A hit started on `pad` with pan `pan` (−1 to 1): the drum's bus
    /// takes the pan, gliding to it if the drum is still sounding.
    pub fn on_hit(&mut self, pad: usize, pan: f32) {
        let pan = pan.clamp(-1.0, 1.0);
        let s = &mut self.strips[pad];
        let l = (1.0 - pan).max(0.0).sqrt() * std::f32::consts::FRAC_1_SQRT_2;
        let r = (1.0 + pan).max(0.0).sqrt() * std::f32::consts::FRAC_1_SQRT_2;
        s.pan_l.target = l;
        s.pan_r.target = r;
        if s.asleep {
            s.pan_l.snap(l);
            s.pan_r.snap(r);
        }
    }

    /// Whether anything here runs: a drum through its bus, or a send
    /// effect ringing. When not, the voices play straight to the output.
    pub fn engaged(&self) -> bool {
        self.routed.iter().any(|&r| r) || self.delay_rest.awake || self.reverb_rest.awake
    }

    /// Clears the buses for a chunk of `n` frames (at most [`CHUNK`]).
    pub fn begin_chunk(&mut self, n: usize) {
        for p in 0..PADS {
            if self.routed[p] {
                self.bus[p][..n].fill(0.0);
            }
        }
        self.live = [false; PADS];
    }

    /// Drum `pad`'s bus for the chunk, if its hits go through it: a hit
    /// adds its frames times its gain before pan.
    #[inline]
    pub fn bus(&mut self, pad: usize) -> Option<&mut [f32; CHUNK]> {
        if self.routed[pad] {
            self.live[pad] = true;
            Some(&mut self.bus[pad])
        } else {
            None
        }
    }

    /// Runs the chunk's buses through their strips into `left` and
    /// `right` (added, as the voices are) and the send buses, then the
    /// send effects. `at` is the chunk's offset in the block.
    pub fn end_chunk(&mut self, at: usize, left: &mut [f32], right: &mut [f32]) {
        let n = left.len().min(right.len()).min(CHUNK);
        let (dl, rest) = self.sends.split_at_mut(1);
        let (dr, rest) = rest.split_at_mut(1);
        let (rl, rr) = rest.split_at_mut(1);
        let (dl, dr, rl, rr) = (&mut dl[0], &mut dr[0], &mut rl[0], &mut rr[0]);
        let mut to_delay = false;
        let mut to_reverb = false;
        for p in 0..PADS {
            if !self.routed[p] {
                continue;
            }
            let s = &mut self.strips[p];
            if s.asleep {
                if !self.live[p] {
                    // Asleep with nothing coming in: silence. A drum with
                    // nothing up goes back to playing straight out.
                    if !s.live() {
                        self.routed[p] = false;
                    }
                    continue;
                }
                s.asleep = false;
            }
            let rest = s.rest_frames(self.values[p * field::COUNT + field::COMP_RELEASE as usize]);
            let buf = &mut self.bus[p][..n];
            // The input's last frame with sound, before the strip works in
            // place: the silent run at frame i after it is i - last.
            let last_in = buf.iter().rposition(|&x| x != 0.0);
            s.process(buf);
            // Sleep: from the first frame (after the chunk's last sound in)
            // where the input has been silent for the strip's rest and its
            // output quiet for QUIET_RUN frames. The input's silent run
            // only grows after its last sound, so if it is short of the
            // rest at the chunk's end no frame here sleeps, and only the
            // output's quiet run carries on.
            let q_end = match last_in {
                Some(l) => (n - 1 - l) as u32,
                None => s.quiet_in.saturating_add(n as u32),
            };
            let mut m = n;
            let mut slept = false;
            let quiet_out = if q_end < rest {
                match buf.iter().rposition(|y| y.abs() >= QUIET_OUT) {
                    Some(k) => (n - 1 - k) as u32,
                    None => s.quiet_out.saturating_add(n as u32),
                }
            } else {
                let mut quiet_out = s.quiet_out;
                let from = last_in.map_or(0, |l| l + 1);
                for (i, &y) in buf.iter().enumerate() {
                    quiet_out = if y.abs() < QUIET_OUT {
                        quiet_out.saturating_add(1)
                    } else {
                        0
                    };
                    if i < from {
                        continue;
                    }
                    let quiet_in = match last_in {
                        Some(l) => (i - l) as u32,
                        None => s.quiet_in.saturating_add(i as u32 + 1),
                    };
                    if quiet_in >= rest && quiet_out >= QUIET_RUN {
                        m = i + 1;
                        slept = true;
                        break;
                    }
                }
                quiet_out
            };
            if slept {
                // Silent past its rest: what it would put out after frame
                // m - 1 is dropped, and it is reset and asleep from there.
                buf[m..].fill(0.0);
                s.rest();
                s.asleep = true;
                self.stats.sleeps += 1;
            } else {
                s.quiet_out = quiet_out;
                s.quiet_in = q_end;
            }
            let buf = &buf[..m];
            // Pan, then the sends after it. What has settled stays put for
            // the chunk (settings change only between chunks).
            if s.pan_l.settled() && s.pan_r.settled() {
                let (pl, pr) = (s.pan_l.value, s.pan_r.value);
                for ((l, r), &y) in left.iter_mut().zip(right.iter_mut()).zip(buf) {
                    *l += y * pl;
                    *r += y * pr;
                }
            } else {
                for ((l, r), &y) in left.iter_mut().zip(right.iter_mut()).zip(buf) {
                    *l += y * s.pan_l.next();
                    *r += y * s.pan_r.next();
                }
            }
            if s.sends_on() {
                let still = s.send_d.settled() && s.send_r.settled();
                let (pl, pr) = (s.pan_l.value, s.pan_r.value);
                let (mut a, mut b) = (s.send_d.value, s.send_r.value);
                for (i, &y) in buf.iter().enumerate() {
                    if !still {
                        a = s.send_d.next();
                        b = s.send_r.next();
                    }
                    let (l, r) = (y * pl, y * pr);
                    dl[i] += l * a;
                    dr[i] += r * a;
                    rl[i] += l * b;
                    rr[i] += r * b;
                }
                to_delay |= a != 0.0 || s.send_d.target != 0.0;
                to_reverb |= b != 0.0 || s.send_r.target != 0.0;
            }
        }
        let ctx = Ctx {
            rate: self.rate,
            frame: self.frame + at as u64,
            bpm: self.bpm,
            beats: self.beats + at as f64 / f64::from(self.rate) * self.bpm / 60.0,
            playing: self.playing,
            sidechain: None,
        };
        let stats = &mut self.stats;
        send(
            &mut self.delay,
            &mut self.delay_rest,
            &mut self.delay_level,
            to_delay,
            &ctx,
            &mut dl[..n],
            &mut dr[..n],
            left,
            right,
            stats,
        );
        send(
            &mut self.reverb,
            &mut self.reverb_rest,
            &mut self.reverb_level,
            to_reverb,
            &ctx,
            &mut rl[..n],
            &mut rr[..n],
            left,
            right,
            stats,
        );
    }

    /// Frames the send effects ring on after the hits end (0 when asleep).
    pub fn tail_frames(&self) -> u32 {
        let d = if self.delay_rest.awake || self.sends_to(field::DELAY_SEND) {
            self.delay.tail_frames()
        } else {
            0
        };
        let r = if self.reverb_rest.awake || self.sends_to(field::REVERB_SEND) {
            self.reverb.tail_frames()
        } else {
            0
        };
        d.max(r)
    }

    fn sends_to(&self, f: u16) -> bool {
        (0..PADS).any(|p| self.v(p, f) > 0.0)
    }

    /// Drums whose hits go through their bus now.
    pub fn routed(&self) -> usize {
        self.routed.iter().filter(|&&r| r).count()
    }

    /// Drums whose bus is running (not asleep).
    pub fn running(&self) -> usize {
        (0..PADS)
            .filter(|&p| self.routed[p] && !self.strips[p].asleep)
            .count()
    }

    /// Whether the delay and the reverb are running.
    pub fn sends_running(&self) -> (bool, bool) {
        (self.delay_rest.awake, self.reverb_rest.awake)
    }

    pub fn stats(&self) -> FxStats {
        self.stats
    }

    /// The level of the kit's delay and reverb returns, 0 to 1.
    pub fn return_levels(&self) -> (f32, f32) {
        (
            self.kv(kit::DELAY_LEVEL) * 0.01,
            self.kv(kit::REVERB_LEVEL) * 0.01,
        )
    }
}

/// Runs one send effect on its bus (`l`, `r`: the chunk's send) and adds
/// its return to `out_l`, `out_r`. It wakes from the first frame with
/// sound, and sleeps from the frame its input has been silent for its
/// rest and its output quiet; both fall on frames counted from the sound,
/// so the block size never moves them.
#[allow(clippy::too_many_arguments)]
fn send(
    node: &mut dyn Node,
    rest: &mut Rest,
    level: &mut Smooth,
    fed: bool,
    ctx: &Ctx,
    l: &mut [f32],
    r: &mut [f32],
    out_l: &mut [f32],
    out_r: &mut [f32],
    stats: &mut FxStats,
) {
    let n = l.len();
    let from = if rest.awake {
        0
    } else {
        // Asleep: wakes at the first frame with sound in it.
        match fed
            .then(|| (0..n).find(|&i| l[i] != 0.0 || r[i] != 0.0))
            .flatten()
        {
            Some(i) => {
                rest.awake = true;
                rest.quiet_in = 0;
                rest.quiet_out = 0;
                level.snap(level.target);
                i
            }
            None => {
                l.fill(0.0);
                r.fill(0.0);
                return;
            }
        }
    };
    // The input's silent run at each frame (carried from the chunks
    // before), read before the node works in place.
    let mut quiet_in = rest.quiet_in;
    let mut silent_from = [0u32; CHUNK];
    for i in from..n {
        quiet_in = if l[i] == 0.0 && r[i] == 0.0 {
            quiet_in.saturating_add(1)
        } else {
            0
        };
        silent_from[i] = quiet_in;
    }
    let sub_ctx = Ctx {
        rate: ctx.rate,
        frame: ctx.frame + from as u64,
        bpm: ctx.bpm,
        beats: ctx.beats,
        playing: ctx.playing,
        sidechain: None,
    };
    node.process(&sub_ctx, &[], &mut l[from..n], &mut r[from..n]);
    // A room whose modulation or gate keeps time across silence never
    // rests by its own measure; here it sleeps after its tail all the
    // same, its wobble starting again on the next sound (nothing anyone
    // hears under -100 dB).
    let rest_frames = match node.rest_frames() {
        u32::MAX => node.tail_frames(),
        n => n,
    };
    let mut end = n;
    for i in from..n {
        let g = level.next();
        let (a, b) = (l[i], r[i]);
        out_l[i] += a * g;
        out_r[i] += b * g;
        rest.quiet_out = if a.abs() < QUIET_OUT && b.abs() < QUIET_OUT {
            rest.quiet_out.saturating_add(1)
        } else {
            0
        };
        if silent_from[i] >= rest_frames && rest.quiet_out >= QUIET_RUN {
            end = i + 1;
            break;
        }
    }
    if end < n || (silent_from[n - 1] >= rest_frames && rest.quiet_out >= QUIET_RUN) {
        // What it would put out after this frame is under −100 dB and its
        // input silent: reset, asleep from the next frame.
        node.reset();
        rest.awake = false;
        rest.quiet_in = 0;
        rest.quiet_out = 0;
        stats.send_sleeps += 1;
    } else {
        rest.quiet_in = silent_from[n - 1];
    }
    l.fill(0.0);
    r.fill(0.0);
}
