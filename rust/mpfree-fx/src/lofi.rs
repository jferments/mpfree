//! Lo-fi: worn tape and dusty vinyl, with one Amount knob over all of it,
//! so it can be a whisper of age or a broken machine.
//!
//! - Tape: wow (slow pitch drift) from a delay swept along a smooth random
//!   path at about Rate, so it wanders like a worn transport instead of
//!   beating like a vibrato, and can drift apart left and right (Stereo);
//!   flutter (fast waver) from two sines at unrelated rates; saturation,
//!   soft and a little asymmetric, run at twice the rate with its curve
//!   antialiased; and dropouts, short dips in level where the oxide is worn.
//! - Vinyl: crackle (clicks and the odd pop, random in size, length and
//!   place), hiss and mains hum. With Follow at 100 they ride the music:
//!   they rise with the input and fade over about a second once it stops,
//!   so a stopped track is silent and a bypassed slot rests. At 0 they run
//!   on as a steady bed.
//! - Tone: a low cut, a high cut and width (age narrows the image).
//!
//! Amount scales everything: wow, flutter, saturation, dropouts, crackle,
//! hiss and hum go to nothing at 0 %, the cuts open to 20 Hz and 20 kHz and
//! the width to full, and at 0 % exactly the output is the input, late by
//! the latency. Wow and flutter grow along a curve: the bottom of each
//! knob is as before, the top reaches a broken machine (2.2 % of wow,
//! 0.55 % of flutter). The tape delay costs a fixed 10 ms of latency,
//! reported for compensation (the deepest wow needs that much tape); the
//! dry path is delayed to match, so Mix never combs.

use mpfree_node::{Ctx, Curve, Kind, Node, NodeEvent, NodeInfo, ParamInfo, Smooth, Timed, Unit};

use crate::adaa::{Adaa, Flat2x};
use crate::character::soft_big;
use crate::oversample::IirPair;
use crate::svf::{Segs, Svf, SvfMode};
use crate::{db_to_gain, defaults, flush, lerp, soft, FixedDelay, Preset, Rng};

/// [`LoFi`] parameter ids.
pub mod ids {
    pub const AMOUNT: u16 = 0;
    pub const WOW: u16 = 1;
    pub const FLUTTER: u16 = 2;
    pub const SATURATION: u16 = 3;
    pub const CRACKLE: u16 = 4;
    pub const NOISE: u16 = 5;
    pub const LOW_CUT: u16 = 6;
    pub const HIGH_CUT: u16 = 7;
    pub const WIDTH: u16 = 8;
    pub const MIX: u16 = 9;
    /// Added in FX quality round 2 (the Lo-fi PR).
    pub const RATE: u16 = 10;
    pub const STEREO: u16 = 11;
    pub const DROPOUTS: u16 = 12;
    pub const HUM: u16 = 13;
    pub const MAINS: u16 = 14;
    pub const FOLLOW: u16 = 15;
}

static INFO: NodeInfo = NodeInfo {
    id: "mpfree.fx.lofi",
    name: "Lo-fi",
    kind: Kind::Effect,
};

const fn pct(id: u16, name: &'static str, group: &'static str, default: f32) -> ParamInfo {
    ParamInfo {
        id,
        name,
        group,
        min: 0.0,
        max: 100.0,
        default,
        unit: Unit::Percent,
        curve: Curve::Linear,
        readonly: false,
    }
}

/// The mains frequencies hum can sit on.
const MAINS_NAMES: &[&str] = &["50 Hz", "60 Hz"];

static PARAMS: [ParamInfo; 16] = [
    pct(0, "Amount", "Lo-fi", 60.0),
    pct(1, "Wow", "Tape", 40.0),
    pct(2, "Flutter", "Tape", 30.0),
    pct(3, "Saturation", "Tape", 40.0),
    pct(4, "Crackle", "Noise", 30.0),
    pct(5, "Noise", "Noise", 20.0),
    ParamInfo {
        id: 6,
        name: "Low cut",
        group: "Tone",
        min: 20.0,
        max: 500.0,
        default: 80.0,
        unit: Unit::Hz,
        curve: Curve::Log,
        readonly: false,
    },
    ParamInfo {
        id: 7,
        name: "High cut",
        group: "Tone",
        min: 2_000.0,
        max: 20_000.0,
        default: 8_000.0,
        unit: Unit::Hz,
        curve: Curve::Log,
        readonly: false,
    },
    pct(8, "Width", "Tone", 70.0),
    pct(9, "Mix", "Output", 100.0),
    ParamInfo {
        id: 10,
        name: "Rate",
        group: "Tape",
        min: 0.2,
        max: 4.0,
        default: 0.7,
        unit: Unit::Hz,
        curve: Curve::Log,
        readonly: false,
    },
    pct(11, "Stereo", "Tape", 0.0),
    pct(12, "Dropouts", "Tape", 0.0),
    pct(13, "Hum", "Noise", 0.0),
    ParamInfo {
        id: 14,
        name: "Mains",
        group: "Noise",
        min: 0.0,
        max: 1.0,
        default: 1.0,
        unit: Unit::None,
        curve: Curve::Choice(MAINS_NAMES),
        readonly: false,
    },
    pct(15, "Follow", "Noise", 100.0),
];

/// Factory presets (Mix left as it is, Amount at its default but in Broken
/// machine).
pub static PRESETS: &[Preset] = &[
    Preset {
        name: "Worn tape",
        values: &[
            (ids::WOW, 60.0),
            (ids::FLUTTER, 40.0),
            (ids::SATURATION, 60.0),
            (ids::CRACKLE, 0.0),
            (ids::NOISE, 25.0),
            (ids::LOW_CUT, 50.0),
            (ids::HIGH_CUT, 11_000.0),
            (ids::WIDTH, 85.0),
            (ids::DROPOUTS, 15.0),
        ],
    },
    Preset {
        name: "Dusty vinyl",
        values: &[
            (ids::WOW, 15.0),
            (ids::FLUTTER, 5.0),
            (ids::SATURATION, 20.0),
            (ids::CRACKLE, 75.0),
            (ids::NOISE, 30.0),
            (ids::LOW_CUT, 70.0),
            (ids::HIGH_CUT, 7_500.0),
            (ids::WIDTH, 60.0),
        ],
    },
    Preset {
        name: "Cassette",
        values: &[
            (ids::WOW, 45.0),
            (ids::FLUTTER, 65.0),
            (ids::SATURATION, 50.0),
            (ids::CRACKLE, 0.0),
            (ids::NOISE, 45.0),
            (ids::LOW_CUT, 70.0),
            (ids::HIGH_CUT, 6_000.0),
            (ids::WIDTH, 55.0),
            (ids::STEREO, 15.0),
        ],
    },
    Preset {
        name: "Old radio",
        values: &[
            (ids::WOW, 0.0),
            (ids::FLUTTER, 10.0),
            (ids::SATURATION, 85.0),
            (ids::CRACKLE, 35.0),
            (ids::NOISE, 40.0),
            (ids::LOW_CUT, 420.0),
            (ids::HIGH_CUT, 3_200.0),
            (ids::WIDTH, 0.0),
            (ids::HUM, 30.0),
        ],
    },
    Preset {
        name: "Broken machine",
        values: &[
            // The only preset that sets Amount: its wow needs the top of
            // the curve.
            (ids::AMOUNT, 100.0),
            (ids::WOW, 85.0),
            (ids::RATE, 0.9),
            (ids::FLUTTER, 70.0),
            (ids::SATURATION, 50.0),
            (ids::CRACKLE, 0.0),
            (ids::NOISE, 35.0),
            (ids::LOW_CUT, 60.0),
            (ids::HIGH_CUT, 8_500.0),
            (ids::WIDTH, 85.0),
            (ids::STEREO, 35.0),
            (ids::DROPOUTS, 35.0),
        ],
    },
    Preset {
        name: "VHS",
        values: &[
            (ids::WOW, 55.0),
            (ids::RATE, 1.6),
            (ids::FLUTTER, 55.0),
            (ids::SATURATION, 40.0),
            (ids::CRACKLE, 0.0),
            (ids::NOISE, 55.0),
            (ids::LOW_CUT, 60.0),
            (ids::HIGH_CUT, 6_500.0),
            (ids::WIDTH, 80.0),
            (ids::STEREO, 35.0),
            (ids::DROPOUTS, 15.0),
            (ids::HUM, 15.0),
        ],
    },
    Preset {
        name: "Vinyl bed",
        values: &[
            (ids::WOW, 10.0),
            (ids::FLUTTER, 0.0),
            (ids::SATURATION, 15.0),
            (ids::CRACKLE, 60.0),
            (ids::NOISE, 30.0),
            (ids::LOW_CUT, 40.0),
            (ids::HIGH_CUT, 12_000.0),
            (ids::WIDTH, 80.0),
            (ids::FOLLOW, 0.0),
        ],
    },
];

/// The tape delay's centre: the latency, and room for the widest wow.
const LATENCY_MS: f32 = 10.0;
/// How far the wow may take the tape from its centre, in seconds: the
/// latency less the flutter's swing and the read's margin.
const WOW_BUDGET_S: f32 = 0.009;
/// Peak pitch deviation of wow and flutter: `dev(k) = BASE k (1 + LIFT k^3)`
/// for the knob's share `k` (Amount times Wow or Flutter). The bottom is
/// the first version's straight line, the top a broken machine.
const WOW_BASE: f32 = 0.004;
const WOW_LIFT: f32 = 4.5;
const FLUTTER_BASE: f32 = 0.0015;
const FLUTTER_LIFT: f32 = 2.67;
/// The deepest wow (`k` = 1).
const WOW_TOP: f32 = WOW_BASE * (1.0 + WOW_LIFT);
/// The wow path's range in seconds of tape per unit of deviation: the
/// deepest wow fills the budget.
const PATH_MAX_S: f32 = WOW_BUDGET_S / WOW_TOP;
/// Flutter's rates in Hz: two sines, at rates that never line up.
const FLUTTER_HZ: [f32; 2] = [7.1, 11.3];
/// Full saturation drives 18 dB into the shaper.
const SAT_DB: f32 = 18.0;
/// Below this saturation the curve fades into the straight line.
const SAT_FADE: f32 = 0.05;
/// What the oversampling and ADAA delay the sound by, in base frames.
const OS_DELAY: u32 = 2 + (Flat2x::STEPS / 2) as u32;
/// The half-bands: Drive's Eco design (flat to 0.19 of the doubled rate,
/// 71 dB down from 0.31).
const OS_SECTIONS: usize = 5;
const OS_TBW: f64 = 0.06;
/// Clicks per second at full crackle.
const CLICKS_PER_S: f32 = 40.0;
/// Hiss at full noise, before the music envelope: about -45 dBFS RMS.
const HISS: f32 = 0.01;
/// Hum at full: about -40 dBFS RMS, a mains fundamental and its first
/// harmonics as a transformer's buzz has them.
const HUM_AMP: f32 = 0.0124;
const HUM_HARMONICS: [f32; 5] = [1.0, 0.45, 0.3, 0.12, 0.08];
/// Dropouts at full: up to this many a second on average, this deep.
const DROPS_PER_S: f32 = 4.0;
const DROP_DB: [f32; 2] = [4.0, 24.0];
/// A dropout's way down and back up, seconds (a raised cosine).
const DROP_RAMP_S: f32 = 0.008;
/// The music envelope: attack and release time constants in seconds, and
/// the input level from which crackle and hiss play in full.
const ENV_ATTACK_S: f32 = 0.01;
const ENV_RELEASE_S: f32 = 0.35;
const ENV_FULL: f32 = 0.125;
/// Amount's first 2 % fades the whole effect in from the plain input, so
/// Amount 0 is the input to the bit.
const ENGAGE: f32 = 0.02;
/// Makeup at Amount 100, dB: what the cuts and saturation take from a
/// mix-level sound at the defaults, given back.
const MAKEUP_DB: f32 = 2.5;

/// A sine of `phase` cycles (polynomial, the same everywhere).
#[inline]
fn sine(phase: f32) -> f32 {
    let x = if phase < 0.5 { phase } else { phase - 1.0 };
    let y = 8.0 * x - 16.0 * x * x.abs();
    0.225 * (y * y.abs() - y) + y
}

#[inline]
fn hermite(x0: f32, x1: f32, x2: f32, x3: f32, t: f32) -> f32 {
    let c1 = 0.5 * (x2 - x0);
    let c2 = x0 - 2.5 * x1 + 2.0 * x2 - 0.5 * x3;
    let c3 = 0.5 * (x3 - x0) + 1.5 * (x1 - x2);
    ((c3 * t + c2) * t + c1) * t + x1
}

/// Peak pitch deviation for a knob's share `k`.
#[inline]
fn deviation(base: f32, lift: f32, k: f32) -> f32 {
    base * k * (1.0 + lift * k * k * k)
}

/// One channel's tape delay.
#[derive(Clone, Debug, Default)]
struct Tape {
    buf: Vec<f32>,
    mask: usize,
    w: usize,
}

impl Tape {
    fn new(frames: usize) -> Tape {
        let n = (frames + 8).next_power_of_two();
        Tape {
            buf: vec![0.0; n],
            mask: n - 1,
            w: 0,
        }
    }
    /// Writes `x`, then reads `d` frames back (0 is `x` itself; `d` at
    /// least 1).
    #[inline]
    fn run(&mut self, x: f32, d: f32) -> f32 {
        self.buf[self.w] = x;
        let last = self.w;
        self.w = (self.w + 1) & self.mask;
        let d = d.clamp(1.0, (self.buf.len() - 4) as f32);
        let i = d as usize;
        let t = d - i as f32;
        let at = |k: usize| self.buf[last.wrapping_sub(k) & self.mask];
        hermite(at(i - 1), at(i), at(i + 1), at(i + 2), t)
    }
    fn clear(&mut self) {
        self.buf.fill(0.0);
        self.w = 0;
    }
}

/// The wow's path: seconds of tape per unit of pitch deviation, wandering
/// from one random point to the next along half a cosine. Each leg takes
/// 0.15 to 0.85 of a period of the Rate, mostly back across the centre
/// (one in five, while there is room, on away from it), at a slope (the
/// pitch deviation it makes, against the whole wow) of 0.35 to 1, and
/// stays within `reach` of the centre: the wobble keeps about its rate and
/// size but never repeats.
#[derive(Clone, Copy, Debug)]
struct Wander {
    rng: Rng,
    from: f32,
    to: f32,
    /// How far along the leg, 0 to 1, and the step a frame.
    pos: f32,
    step: f32,
    reach: f32,
}

impl Wander {
    fn new(seed: u32, reach: f32) -> Wander {
        Wander {
            rng: Rng::new(seed),
            from: 0.0,
            to: 0.0,
            pos: 1.0,
            step: 0.0,
            reach,
        }
    }
    fn unit(&mut self) -> f32 {
        0.5 * (self.rng.bipolar() + 1.0)
    }
    /// The next frame's place on the path, for a rate of `hz`.
    #[inline]
    fn next(&mut self, hz: f32, fs: f32) -> f32 {
        self.pos += self.step;
        if self.pos >= 1.0 {
            self.leg(hz, fs);
        }
        let s = sine(0.25 * self.pos);
        self.from + (self.to - self.from) * (s * s)
    }
    fn leg(&mut self, hz: f32, fs: f32) {
        self.from = self.to;
        let secs = (0.3 + 1.4 * self.unit()) / (2.0 * hz);
        let slope = 0.35 + 0.65 * self.unit();
        // A leg of `secs` along half a cosine, `2 secs slope / pi` long,
        // is steepest at `slope` in the middle.
        let len = 2.0 * secs * slope / std::f32::consts::PI;
        // Back across the centre, or now and then on away from it while
        // there is room.
        let toward = if self.from > 0.0 { -1.0 } else { 1.0 };
        let room = self.from.abs() < 0.5 * self.reach;
        let dir = if room && self.rng.bipolar() < -0.6 {
            -toward
        } else {
            toward
        };
        self.to = (self.from + dir * len).clamp(-self.reach, self.reach);
        self.step = 1.0 / (secs * fs).max(1.0);
        self.pos = 0.0;
    }
}

/// Dropouts: now and then the level dips by the set depth for 20 to 150
/// ms, down and back up along a raised cosine.
#[derive(Clone, Copy, Debug)]
struct Drops {
    rng: Rng,
    /// Frames until the next dropout starts (while idle), or left in its
    /// current stage.
    left: u32,
    stage: u8,
    /// The dip's gain and the frames of its ramps.
    depth: f32,
    ramp: u32,
    gain: f32,
}

impl Drops {
    fn new(seed: u32) -> Drops {
        Drops {
            rng: Rng::new(seed),
            left: 0,
            stage: 0,
            depth: 1.0,
            ramp: 1,
            gain: 1.0,
        }
    }
    fn unit(&mut self) -> f32 {
        0.5 * (self.rng.bipolar() + 1.0)
    }
    /// The gap to the next dropout for wear `w`: random, on average
    /// `1 / rate` seconds.
    fn gap(&mut self, w: f32, fs: f32) -> u32 {
        let rate = DROPS_PER_S * (0.06 + 0.94 * w * w);
        let u = self.unit().max(1e-6);
        ((-u.ln() / rate).min(30.0) * fs) as u32 + 1
    }
    /// The gain this frame for wear `w` (0 to 1).
    #[inline]
    fn next(&mut self, w: f32, fs: f32) -> f32 {
        if w <= 0.0 && self.stage == 0 {
            self.left = 0;
            self.gain = 1.0;
            return 1.0;
        }
        if self.left == 0 {
            match self.stage {
                0 => {
                    // Idle: a first gap after wear comes in, then a dip.
                    if self.gain == 1.0 && self.depth == 1.0 {
                        self.depth = 0.0;
                        self.left = self.gap(w, fs);
                        return 1.0;
                    }
                    let db = DROP_DB[0] + (DROP_DB[1] - DROP_DB[0]) * w;
                    self.depth = db_to_gain(-db);
                    self.ramp = (DROP_RAMP_S * fs).max(1.0) as u32;
                    self.stage = 1;
                    self.left = self.ramp;
                }
                1 => {
                    self.stage = 2;
                    self.left = ((0.02 + 0.13 * self.unit()) * fs) as u32 + 1;
                }
                2 => {
                    self.stage = 3;
                    self.left = self.ramp;
                }
                _ => {
                    self.stage = 0;
                    self.gain = 1.0;
                    self.left = self.gap(w.max(1e-3), fs);
                    return 1.0;
                }
            }
        }
        self.left -= 1;
        let p = 1.0 - self.left as f32 / self.ramp as f32;
        let s = sine(0.25 * p);
        self.gain = match self.stage {
            1 => lerp(1.0, self.depth, s * s),
            2 => self.depth,
            3 => lerp(self.depth, 1.0, s * s),
            _ => 1.0,
        };
        self.gain
    }
    fn reset(&mut self, seed: u32) {
        *self = Drops::new(seed);
    }
}

/// Crackle for one channel: a burst of noise with a fast decay per click.
#[derive(Clone, Copy, Debug)]
struct Crackle {
    rng: Rng,
    level: f32,
    decay: f32,
    /// A one-pole low-pass on the burst: big pops are duller.
    lp: f32,
    lp_a: f32,
}

impl Crackle {
    fn new(seed: u32) -> Crackle {
        Crackle {
            rng: Rng::new(seed),
            level: 0.0,
            decay: 0.0,
            lp: 0.0,
            lp_a: 1.0,
        }
    }
    #[inline]
    fn run(&mut self, chance: f32, size: f32, rate: f32) -> f32 {
        // A new click: mostly small, now and then a pop (size cubed).
        let roll = 0.5 * (self.rng.bipolar() + 1.0);
        if roll < chance {
            let u = 0.5 * (self.rng.bipolar() + 1.0);
            let big = u * u * u;
            self.level = size * (0.05 + 0.6 * big);
            // 0.1 ms for a tick, up to 1 ms for a pop.
            self.decay = (-1.0 / ((0.0001 + 0.0009 * big) * rate)).exp();
            self.lp_a =
                1.0 - (-2.0 * std::f32::consts::PI * (9_000.0 - 7_000.0 * big) / rate).exp();
        }
        if self.level == 0.0 {
            return 0.0;
        }
        let n = self.rng.bipolar();
        let x = n * self.level;
        self.level = flush(self.level * self.decay);
        self.lp = flush(self.lp + self.lp_a * (x - self.lp));
        self.lp
    }
    fn reset(&mut self, seed: u32) {
        *self = Crackle::new(seed);
    }
}

/// One channel's saturation at twice the rate.
#[derive(Clone, Debug)]
struct Sat {
    iir: IirPair<OS_SECTIONS>,
    adaa: Adaa,
    flat: Flat2x,
    /// The last input at twice the rate, for the straight line's average.
    v1: f32,
}

impl Sat {
    fn new() -> Sat {
        Sat {
            iir: IirPair::new(OS_TBW),
            adaa: Adaa::default(),
            flat: Flat2x::default(),
            v1: 0.0,
        }
    }
    /// One base frame: `drive`, `bias` and `offset` set the curve, `blend`
    /// fades it in from the straight line (at 0 the straight line alone,
    /// and the curve's state follows the input). `seed` restarts the
    /// curve's ADAA from the last input, as the curve comes in.
    #[inline]
    fn run(&mut self, x: f32, drive: f32, undrive: f32, bias: f32, offset: f32, blend: f32) -> f32 {
        let up = self.iir.up(x);
        let mut out = [0.0f32; 2];
        for (o, &v) in out.iter_mut().zip(&up) {
            // In its straight part ADAA is the two-point mean; the curve's
            // average replaces it as the saturation comes in.
            let line = 0.5 * (v + self.v1);
            let y = if blend > 0.0 {
                let c = (self.adaa.run(drive * v + bias, soft, soft_big) - offset) * undrive;
                line + (c - line) * blend
            } else {
                line
            };
            self.v1 = v;
            *o = self.flat.run(y);
        }
        self.iir.down(out)
    }
    fn seed(&mut self, drive: f32, bias: f32) {
        self.adaa.seed(drive * self.v1 + bias, soft_big);
    }
    fn settle(&mut self) {
        self.iir.settle();
        self.v1 = flush(self.v1);
    }
    fn reset(&mut self) {
        self.iir.reset();
        self.adaa.reset();
        self.flat.reset();
        self.v1 = 0.0;
    }
}

/// Tape wear and vinyl dust with one Amount over all of it.
pub struct LoFi {
    v: [f32; 16],
    rate: u32,
    amount: Smooth,
    wow: Smooth,
    flutter: Smooth,
    sat: Smooth,
    crackle: Smooth,
    noise: Smooth,
    log_low: Smooth,
    log_high: Smooth,
    width: Smooth,
    mix: Smooth,
    wow_hz: Smooth,
    stereo: Smooth,
    drops: Smooth,
    hum: Smooth,
    follow: Smooth,
    /// Flutter's two sine phases (cycles), and their steps.
    phase: [f64; 2],
    inc: [f64; 2],
    /// The wow's paths: the left (and both, at Stereo 0) and the right's
    /// own.
    wander: [Wander; 2],
    tape: [Tape; 2],
    sat_os: [Sat; 2],
    /// Whether the curve ran last frame, to seed its ADAA as it comes in.
    sat_on: bool,
    dry: [FixedDelay; 2],
    centre: u32,
    latency: u32,
    low: Svf,
    high: Svf,
    clicks: [Crackle; 2],
    dropouts: Drops,
    hiss: Rng,
    hiss_lp: [f32; 2],
    hiss_a: f32,
    hum_phase: f64,
    hum_inc: f64,
    env: f32,
    env_up: f32,
    env_down: f32,
    /// Frames since the stream started, for settling the half-bands on a
    /// grid the block size doesn't move.
    grid: u32,
}

const SEEDS: [u32; 6] = [
    0x51ED_270B,
    0x2F6C_A3E1,
    0x9B3C_57D4,
    0x6A09_E667,
    0xBB67_AE85,
    0x3C6E_F372,
];

impl Default for LoFi {
    fn default() -> Self {
        Self::new()
    }
}

impl LoFi {
    /// The low and high cuts' response in dB at `hz`, makeup included, for
    /// the Amount and corners set (at Amount 0 they open right up and the
    /// makeup is 0 dB). For drawing.
    pub fn cuts_db(&self, hz: f32) -> f32 {
        let a = self.v[ids::AMOUNT as usize] / 100.0;
        let lo = self.v[ids::LOW_CUT as usize].log2();
        let hi = self.v[ids::HIGH_CUT as usize].log2();
        let (mut low, mut high) = (Svf::default(), Svf::default());
        let q = std::f32::consts::FRAC_1_SQRT_2;
        let low_hz = lerp(20f32.log2(), lo, a).exp2();
        let high_hz = lerp(20_000f32.log2(), hi, a).exp2();
        low.set(SvfMode::HighPass, low_hz, q, 0.0, self.rate);
        high.set(SvfMode::LowPass, high_hz, q, 0.0, self.rate);
        let h = crate::svf::cmul(low.response(hz, self.rate), high.response(hz, self.rate));
        // With the makeup, which the cuts' passband carries.
        crate::svf::power_db(h) + MAKEUP_DB * a
    }

    /// The depth of a dropout at the Amount and Dropouts set, in dB (0
    /// with none), for drawing and measuring.
    pub fn dropout_db(&self) -> f32 {
        let w = self.v[ids::AMOUNT as usize] * self.v[ids::DROPOUTS as usize] / 10_000.0;
        if w > 0.0 {
            DROP_DB[0] + (DROP_DB[1] - DROP_DB[0]) * w
        } else {
            0.0
        }
    }

    /// The peak pitch deviation the wow and the flutter reach at the
    /// Amount, Wow and Flutter set, in percent (for the page).
    pub fn wow_flutter_pct(&self) -> (f32, f32) {
        let a = self.v[ids::AMOUNT as usize] / 100.0;
        let w = a * self.v[ids::WOW as usize] / 100.0;
        let f = a * self.v[ids::FLUTTER as usize] / 100.0;
        (
            100.0 * deviation(WOW_BASE, WOW_LIFT, w),
            100.0 * deviation(FLUTTER_BASE, FLUTTER_LIFT, f),
        )
    }

    pub fn new() -> LoFi {
        let v = defaults(&PARAMS);
        let mut f = LoFi {
            v,
            rate: 0,
            amount: Smooth::new(v[0] / 100.0),
            wow: Smooth::new(v[1] / 100.0),
            flutter: Smooth::new(v[2] / 100.0),
            sat: Smooth::new(v[3] / 100.0),
            crackle: Smooth::new(v[4] / 100.0),
            noise: Smooth::new(v[5] / 100.0),
            log_low: Smooth::new(v[6].log2()),
            log_high: Smooth::new(v[7].log2()),
            width: Smooth::new(v[8] / 100.0),
            mix: Smooth::new(v[9] / 100.0),
            wow_hz: Smooth::new(v[10]),
            stereo: Smooth::new(v[11] / 100.0),
            drops: Smooth::new(v[12] / 100.0),
            hum: Smooth::new(v[13] / 100.0),
            follow: Smooth::new(v[15] / 100.0),
            phase: [0.0, 0.61],
            inc: [0.0; 2],
            wander: [
                Wander::new(SEEDS[3], PATH_MAX_S),
                Wander::new(SEEDS[4], PATH_MAX_S),
            ],
            tape: [Tape::default(), Tape::default()],
            sat_os: [Sat::new(), Sat::new()],
            sat_on: false,
            dry: [FixedDelay::default(), FixedDelay::default()],
            centre: 0,
            latency: 0,
            low: Svf::default(),
            high: Svf::default(),
            clicks: [Crackle::new(SEEDS[0]), Crackle::new(SEEDS[1])],
            dropouts: Drops::new(SEEDS[5]),
            hiss: Rng::new(SEEDS[2]),
            hiss_lp: [0.0; 2],
            hiss_a: 1.0,
            hum_phase: 0.0,
            hum_inc: 0.0,
            env: 0.0,
            env_up: 1.0,
            env_down: 1.0,
            grid: 0,
        };
        f.prepare(48_000, mpfree_node::MAX_BLOCK);
        f.reset();
        f
    }

    fn apply(&mut self, id: u16, value: f32) {
        let Some(p) = PARAMS.get(id as usize) else {
            return;
        };
        let v = p.clamp(value);
        self.v[id as usize] = v;
        let target = match id {
            ids::LOW_CUT | ids::HIGH_CUT => v.log2(),
            ids::RATE => v,
            ids::MAINS => {
                self.hum_inc = Self::mains_hz(v) / f64::from(self.rate.max(1));
                return;
            }
            _ => v / 100.0,
        };
        self.smooth(id).target = target;
    }

    fn mains_hz(v: f32) -> f64 {
        if v >= 0.5 {
            60.0
        } else {
            50.0
        }
    }

    fn smooth(&mut self, id: u16) -> &mut Smooth {
        match id {
            ids::AMOUNT => &mut self.amount,
            ids::WOW => &mut self.wow,
            ids::FLUTTER => &mut self.flutter,
            ids::SATURATION => &mut self.sat,
            ids::CRACKLE => &mut self.crackle,
            ids::NOISE => &mut self.noise,
            ids::LOW_CUT => &mut self.log_low,
            ids::HIGH_CUT => &mut self.log_high,
            ids::WIDTH => &mut self.width,
            ids::RATE => &mut self.wow_hz,
            ids::STEREO => &mut self.stereo,
            ids::DROPOUTS => &mut self.drops,
            ids::HUM => &mut self.hum,
            ids::FOLLOW => &mut self.follow,
            _ => &mut self.mix,
        }
    }

    fn each_smooth(&mut self, f: &mut dyn FnMut(&mut Smooth)) {
        for s in [
            &mut self.amount,
            &mut self.wow,
            &mut self.flutter,
            &mut self.sat,
            &mut self.crackle,
            &mut self.noise,
            &mut self.log_low,
            &mut self.log_high,
            &mut self.width,
            &mut self.mix,
            &mut self.wow_hz,
            &mut self.stereo,
            &mut self.drops,
            &mut self.hum,
            &mut self.follow,
        ] {
            f(s);
        }
    }

    /// The cut filters for the current Amount and corners: at Amount 0
    /// they open to 20 Hz and 20 kHz.
    fn filters(&mut self) {
        let a = self.amount.value;
        let low = lerp(20f32.log2(), self.log_low.value, a).exp2();
        let high = lerp(20_000f32.log2(), self.log_high.value, a).exp2();
        self.low.set(
            SvfMode::HighPass,
            low,
            std::f32::consts::FRAC_1_SQRT_2,
            0.0,
            self.rate,
        );
        self.high.set(
            SvfMode::LowPass,
            high,
            std::f32::consts::FRAC_1_SQRT_2,
            0.0,
            self.rate,
        );
    }

    /// Whether any of the noises can play with nothing coming in.
    fn plays_alone(&self) -> bool {
        let v = &self.v;
        v[ids::AMOUNT as usize] > 0.0
            && v[ids::FOLLOW as usize] < 100.0
            && (v[ids::CRACKLE as usize] > 0.0
                || v[ids::NOISE as usize] > 0.0
                || v[ids::HUM as usize] > 0.0)
    }

    fn run(&mut self, l: &mut [f32], r: &mut [f32]) {
        let fs = self.rate as f32;
        let centre = self.centre as f32;
        let tau = 2.0 * std::f32::consts::PI;
        // Flutter's swing in frames for a deviation of 1.
        let flutter_frames = fs / (tau * 0.5 * (FLUTTER_HZ[0] + FLUTTER_HZ[1]));
        // The saturator's gain, its inverse and the bias's own offset,
        // worked out again only while the saturation glides (powf per
        // sample is a library call on the MPC's ARM).
        let (mut sat_was, mut drive, mut undrive, mut bias, mut offset) =
            (f32::NAN, 1.0, 1.0, 0.0, 0.0);
        let mut amount_was = f32::NAN;
        let mut makeup = 1.0;
        for i in 0..l.len() {
            if !(self.amount.settled() && self.log_low.settled() && self.log_high.settled()) {
                self.amount.next();
                self.log_low.next();
                self.log_high.next();
                self.filters();
            }
            let a = self.amount.value;
            if a != amount_was {
                amount_was = a;
                makeup = db_to_gain(MAKEUP_DB * a);
            }
            let wow = deviation(WOW_BASE, WOW_LIFT, a * self.wow.next());
            let flutter = deviation(FLUTTER_BASE, FLUTTER_LIFT, a * self.flutter.next());
            let sat = a * self.sat.next();
            let crackle = a * self.crackle.next();
            let noise = a * self.noise.next();
            let hum = a * self.hum.next();
            let wear = a * self.drops.next();
            let follow = self.follow.next();
            let width = 1.0 - a * (1.0 - self.width.next());
            let stereo = self.stereo.next();
            let hz = self.wow_hz.next();
            let mix = self.mix.next() * (a / ENGAGE).min(1.0);
            for (p, inc) in self.phase.iter_mut().zip(&self.inc) {
                *p += inc;
                if *p >= 1.0 {
                    *p -= 1.0;
                }
            }
            let fl = 0.7 * sine(self.phase[0] as f32) + 0.3 * sine(self.phase[1] as f32);
            let fl = flutter * flutter_frames * fl;
            let pl = self.wander[0].next(hz, fs);
            let pr = self.wander[1].next(hz, fs);
            let pr = pl + (pr - pl) * stereo;
            let d = [centre + wow * fs * pl + fl, centre + wow * fs * pr + fl];
            let x = [l[i], r[i]];
            // The music envelope that crackle, hiss and hum ride.
            let level = x[0].abs().max(x[1].abs());
            let k = if level > self.env {
                self.env_up
            } else {
                self.env_down
            };
            self.env = flush(self.env + k * (level - self.env));
            let ride = (self.env / ENV_FULL).min(1.0);
            let ride = 1.0 - follow + follow * ride;
            let chance = crackle * CLICKS_PER_S / fs;
            if sat != sat_was {
                sat_was = sat;
                drive = db_to_gain(SAT_DB * sat);
                undrive = 1.0 / drive;
                bias = 0.15 * sat;
                offset = soft(bias);
            }
            let blend = (sat / SAT_FADE).min(1.0);
            if blend > 0.0 && !self.sat_on {
                for s in &mut self.sat_os {
                    s.seed(drive, bias);
                }
            }
            self.sat_on = blend > 0.0;
            let gain = self.dropouts.next(wear, fs);
            let hum_s = if hum > 0.0 && ride > 0.0 {
                self.hum_phase += self.hum_inc;
                if self.hum_phase >= 1.0 {
                    self.hum_phase -= 1.0;
                }
                let p = self.hum_phase as f32;
                let mut h = 0.0;
                for (k, w) in HUM_HARMONICS.iter().enumerate() {
                    h += w * sine(((k + 1) as f32 * p).fract());
                }
                hum * HUM_AMP * h
            } else {
                0.0
            };
            let mut y = [0.0f32; 2];
            for ch in 0..2 {
                let s = self.tape[ch].run(x[ch], d[ch]);
                // Asymmetric, so it adds even harmonics; the bias's own
                // offset is taken back out, and the low cut removes the
                // rest of the DC it leaves on a changing signal.
                let mut s = self.sat_os[ch].run(s, drive, undrive, bias, offset, blend) * gain;
                s = self.low.run(ch, s);
                s = self.high.run(ch, s);
                if ride > 0.0 {
                    let h = self.hiss.bipolar();
                    self.hiss_lp[ch] =
                        flush(self.hiss_lp[ch] + self.hiss_a * (h - self.hiss_lp[ch]));
                    s += ride
                        * (noise * HISS * self.hiss_lp[ch]
                            + self.clicks[ch].run(chance, crackle, fs)
                            + hum_s);
                }
                y[ch] = s * makeup;
            }
            // Width: narrow the side signal.
            let mid = 0.5 * (y[0] + y[1]);
            let side = 0.5 * (y[0] - y[1]) * width;
            let dry = [self.dry[0].run(x[0]), self.dry[1].run(x[1])];
            l[i] = lerp(dry[0], mid + side, mix);
            r[i] = lerp(dry[1], mid - side, mix);
            self.grid = self.grid.wrapping_add(1);
            if self.grid.is_multiple_of(64) {
                for s in &mut self.sat_os {
                    s.settle();
                }
            }
        }
    }
}

impl Node for LoFi {
    fn info(&self) -> &'static NodeInfo {
        &INFO
    }
    fn params(&self) -> &'static [ParamInfo] {
        &PARAMS
    }
    fn set_param(&mut self, id: u16, value: f32) {
        self.apply(id, value);
    }
    fn param(&self, id: u16) -> f32 {
        self.v.get(id as usize).copied().unwrap_or(f32::NAN)
    }
    fn prepare(&mut self, rate: u32, _max_block: usize) {
        let rate = rate.max(1);
        let fs = rate as f32;
        if rate != self.rate {
            self.centre = (LATENCY_MS * 0.001 * fs).round() as u32;
            self.latency = self.centre + OS_DELAY;
            let frames = 2 * self.centre as usize + 8;
            self.tape = [Tape::new(frames), Tape::new(frames)];
            self.dry = [
                FixedDelay::new(self.latency as usize),
                FixedDelay::new(self.latency as usize),
            ];
        }
        self.rate = rate;
        self.each_smooth(&mut |s| s.set_time(rate, 30.0));
        for (inc, hz) in self.inc.iter_mut().zip(FLUTTER_HZ.iter()) {
            *inc = f64::from(*hz) / f64::from(rate);
        }
        self.hum_inc = Self::mains_hz(self.v[ids::MAINS as usize]) / f64::from(rate);
        self.hiss_a = 1.0 - (-2.0 * std::f32::consts::PI * 6_000.0f32.min(0.45 * fs) / fs).exp();
        self.env_up = 1.0 - (-1.0 / (ENV_ATTACK_S * fs)).exp();
        self.env_down = 1.0 - (-1.0 / (ENV_RELEASE_S * fs)).exp();
        self.filters();
    }
    fn reset(&mut self) {
        self.each_smooth(&mut |s| s.snap(s.target));
        for t in &mut self.tape {
            t.clear();
        }
        for d in &mut self.dry {
            d.reset();
        }
        for s in &mut self.sat_os {
            s.reset();
        }
        self.sat_on = false;
        self.low.reset();
        self.high.reset();
        self.clicks[0].reset(SEEDS[0]);
        self.clicks[1].reset(SEEDS[1]);
        self.dropouts.reset(SEEDS[5]);
        self.wander = [
            Wander::new(SEEDS[3], PATH_MAX_S),
            Wander::new(SEEDS[4], PATH_MAX_S),
        ];
        self.hiss = Rng::new(SEEDS[2]);
        self.hiss_lp = [0.0; 2];
        self.hum_phase = 0.0;
        self.env = 0.0;
        self.phase = [0.0, 0.61];
        self.grid = 0;
        self.filters();
    }
    fn process(&mut self, _ctx: &Ctx, events: &[Timed], left: &mut [f32], right: &mut [f32]) {
        let len = left.len().min(right.len());
        let mut segs = Segs::new(events);
        while let Some((from, to)) = segs.next(len, |ev| {
            if let NodeEvent::Param { id, value } = *ev {
                self.apply(id, value)
            }
        }) {
            self.run(&mut left[from..to], &mut right[from..to]);
        }
    }
    fn latency(&self) -> u32 {
        self.latency
    }
    fn rest_frames(&self) -> u32 {
        if self.plays_alone() {
            // Noise that runs on without the music: never at rest.
            return u32::MAX;
        }
        // What it holds is its tail: once rung out it is as good as reset.
        self.tail_frames()
    }
    fn tail_frames(&self) -> u32 {
        if self.plays_alone() {
            return u32::MAX;
        }
        let fs = self.rate as f32;
        // The tape delay, the half-bands and the filters' settling; while
        // a noise is on, the music envelope's release down 80 dB (from
        // full).
        let mut secs = 0.05;
        if self.v[4] > 0.0 || self.v[5] > 0.0 || self.v[13] > 0.0 {
            secs += ENV_RELEASE_S * (1e4f32 / ENV_FULL).ln();
        }
        self.latency + (secs * fs) as u32
    }
}
