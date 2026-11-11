//! Cloud: a lush algorithmic reverb, a Rust port of the Cloud Seed reverb
//! core (CloudSeedCore by Ghost Note Audio, MIT licence; the notice is in
//! `LICENSE-cloudseed.txt`). MPFree is not made or endorsed by Ghost Note
//! Audio.
//!
//! The signal path is Cloud Seed's: a little of each input side into the
//! other, low and high cut, a pre-delay, a multi-tap early pattern, a chain
//! of modulated all-pass diffusers, then up to 12 modulated feedback delay
//! lines, each with its own all-pass diffuser, shelving EQ and damping
//! inside its loop. Every length, gain and modulation rate comes from a
//! seeded random series, a different one for each side, so the left and
//! right tails never match.
//!
//! What changed in the port:
//! - Feedback is per frame, so the output is the same, sample for sample,
//!   for any block size (Cloud Seed feeds each line back one buffer late).
//! - Memory comes from `prepare`; nothing allocates on the audio thread,
//!   and a reset forgets every buffer in O(1) instead of clearing
//!   megabytes (a read further back than the writes since the reset is
//!   silence).
//! - Modulator start phases come from the seeds (Cloud Seed uses `rand()`)
//!   and the LFO is a polynomial sine, so a render is the same on every
//!   machine.
//! - The feedback gain counts each line's diffuser in the loop length, so
//!   Decay is the time to fall 60 dB however much diffusion there is.
//! - Cloud Seed's 45 parameters become 14. A Character picks the hidden
//!   structure (line count, diffuser stages, the early pattern, the EQ in
//!   the loop and the seeds); the rest are the controls people reach for.
//!   Size, decay, pre-delay and the rest glide, so moving them bends the
//!   tail instead of clicking; a Character change dips the wet signal for
//!   30 ms while the structure swaps.
//! - Width is mid/side on the wet signal; Freeze mutes the input and holds
//!   the tail (feedback 1, the EQ in the loop bypassed).

use std::f32::consts::{LN_10, PI};

use mpfree_node::{Ctx, Curve, Kind, Node, NodeEvent, NodeInfo, ParamInfo, Smooth, Timed, Unit};

use crate::oversample::{Down2, Up2};
use crate::svf::Segs;
use crate::{defaults, flush, lerp, Preset};

/// [`Cloud`] parameter ids.
pub mod ids {
    pub const CHARACTER: u16 = 0;
    pub const SIZE: u16 = 1;
    pub const DECAY: u16 = 2;
    pub const PRE_DELAY: u16 = 3;
    pub const DIFFUSION: u16 = 4;
    pub const MODULATION: u16 = 5;
    pub const MOD_RATE: u16 = 6;
    pub const DAMPING: u16 = 7;
    pub const LOW_CUT: u16 = 8;
    pub const HIGH_CUT: u16 = 9;
    pub const WIDTH: u16 = 10;
    pub const EARLY: u16 = 11;
    pub const MIX: u16 = 12;
    pub const FREEZE: u16 = 13;
    /// 0 Full; 1 Light: half the lines, diffuser stages and early taps,
    /// for about half the cost (a third for the Cloud character); 2 Eco
    /// (the default): Light's structure run at half the sample rate, so the
    /// reverb rolls off above about a fifth of the rate (9 kHz at 44.1
    /// kHz) for about a quarter of Full's cost.
    pub const QUALITY: u16 = 14;
}

/// The qualities, as [`ids::QUALITY`] values.
const FULL: usize = 0;
const ECO: usize = 2;

const CHARACTERS: &[&str] = &["Plate", "Room", "Hall", "Cloud", "Echoes"];
const QUALITIES: &[&str] = &["Full", "Light", "Eco"];

static INFO: NodeInfo = NodeInfo {
    id: "mpfree.fx.cloud",
    name: "Cloud reverb",
    kind: Kind::Effect,
};
static PARAMS: [ParamInfo; 15] = [
    ParamInfo {
        id: 0,
        name: "Character",
        group: "Reverb",
        min: 0.0,
        max: 4.0,
        default: 2.0,
        unit: Unit::None,
        curve: Curve::Choice(CHARACTERS),
        readonly: false,
    },
    ParamInfo {
        id: 1,
        name: "Size",
        group: "Reverb",
        min: 20.0,
        max: 1000.0,
        default: 120.0,
        unit: Unit::Ms,
        curve: Curve::Log,
        readonly: false,
    },
    ParamInfo {
        id: 2,
        name: "Decay",
        group: "Reverb",
        min: 50.0,
        max: 60_000.0,
        default: 2500.0,
        unit: Unit::Ms,
        curve: Curve::Log,
        readonly: false,
    },
    ParamInfo {
        id: 3,
        name: "Pre-delay",
        group: "Reverb",
        min: 0.0,
        max: 500.0,
        default: 10.0,
        unit: Unit::Ms,
        curve: Curve::Linear,
        readonly: false,
    },
    ParamInfo {
        id: 4,
        name: "Diffusion",
        group: "Texture",
        min: 0.0,
        max: 100.0,
        default: 70.0,
        unit: Unit::Percent,
        curve: Curve::Linear,
        readonly: false,
    },
    ParamInfo {
        id: 5,
        name: "Modulation",
        group: "Texture",
        min: 0.0,
        max: 100.0,
        default: 30.0,
        unit: Unit::Percent,
        curve: Curve::Linear,
        readonly: false,
    },
    ParamInfo {
        id: 6,
        name: "Mod rate",
        group: "Texture",
        min: 0.05,
        max: 5.0,
        default: 0.3,
        unit: Unit::Hz,
        curve: Curve::Log,
        readonly: false,
    },
    ParamInfo {
        id: 7,
        name: "Damping",
        group: "Tone",
        min: 400.0,
        max: 20_000.0,
        default: 9000.0,
        unit: Unit::Hz,
        curve: Curve::Log,
        readonly: false,
    },
    ParamInfo {
        id: 8,
        name: "Low cut",
        group: "Tone",
        min: 20.0,
        max: 1000.0,
        default: 80.0,
        unit: Unit::Hz,
        curve: Curve::Log,
        readonly: false,
    },
    ParamInfo {
        id: 9,
        name: "High cut",
        group: "Tone",
        min: 400.0,
        max: 20_000.0,
        default: 14_000.0,
        unit: Unit::Hz,
        curve: Curve::Log,
        readonly: false,
    },
    ParamInfo {
        id: 10,
        name: "Width",
        group: "Output",
        min: 0.0,
        max: 100.0,
        default: 100.0,
        unit: Unit::Percent,
        curve: Curve::Linear,
        readonly: false,
    },
    ParamInfo {
        id: 11,
        name: "Early",
        group: "Output",
        min: 0.0,
        max: 100.0,
        default: 40.0,
        unit: Unit::Percent,
        curve: Curve::Linear,
        readonly: false,
    },
    ParamInfo {
        id: 12,
        name: "Mix",
        group: "Output",
        min: 0.0,
        max: 100.0,
        default: 30.0,
        unit: Unit::Percent,
        curve: Curve::Linear,
        readonly: false,
    },
    ParamInfo {
        id: 13,
        name: "Freeze",
        group: "Output",
        min: 0.0,
        max: 1.0,
        default: 0.0,
        unit: Unit::None,
        curve: Curve::Toggle,
        readonly: false,
    },
    ParamInfo {
        id: 14,
        name: "Quality",
        group: "Reverb",
        min: 0.0,
        max: 2.0,
        default: 2.0,
        unit: Unit::None,
        curve: Curve::Choice(QUALITIES),
        readonly: false,
    },
];

/// Factory presets. Mix is left alone: it depends on whether the reverb
/// sits on an insert or a send.
pub static PRESETS: &[Preset] = {
    use ids::*;
    &[
        Preset {
            name: "Dark plate",
            values: &[
                (CHARACTER, 0.0),
                (SIZE, 96.0),
                (DECAY, 4800.0),
                (PRE_DELAY, 0.0),
                (DIFFUSION, 97.0),
                (MODULATION, 27.0),
                (MOD_RATE, 0.1),
                (DAMPING, 20_000.0),
                (LOW_CUT, 340.0),
                (HIGH_CUT, 20_000.0),
                (EARLY, 0.0),
            ],
        },
        Preset {
            name: "Small room",
            values: &[
                (CHARACTER, 1.0),
                (SIZE, 40.0),
                (DECAY, 700.0),
                (PRE_DELAY, 4.0),
                (DIFFUSION, 75.0),
                (MODULATION, 15.0),
                (MOD_RATE, 0.5),
                (DAMPING, 7000.0),
                (LOW_CUT, 120.0),
                (HIGH_CUT, 12_000.0),
                (EARLY, 60.0),
            ],
        },
        Preset {
            name: "Big hall",
            values: &[
                (CHARACTER, 2.0),
                (SIZE, 180.0),
                (DECAY, 3500.0),
                (PRE_DELAY, 25.0),
                (DIFFUSION, 80.0),
                (MODULATION, 35.0),
                (MOD_RATE, 0.3),
                (DAMPING, 6500.0),
                (LOW_CUT, 90.0),
                (HIGH_CUT, 11_000.0),
                (EARLY, 45.0),
            ],
        },
        Preset {
            name: "Endless cloud",
            values: &[
                (CHARACTER, 3.0),
                (SIZE, 300.0),
                (DECAY, 20_000.0),
                (PRE_DELAY, 40.0),
                (DIFFUSION, 90.0),
                (MODULATION, 60.0),
                (MOD_RATE, 0.25),
                (DAMPING, 8000.0),
                (LOW_CUT, 150.0),
                (HIGH_CUT, 10_000.0),
                (EARLY, 30.0),
            ],
        },
        Preset {
            name: "Echo chamber",
            values: &[
                (CHARACTER, 4.0),
                (SIZE, 260.0),
                (DECAY, 3000.0),
                (PRE_DELAY, 0.0),
                (DIFFUSION, 50.0),
                (MODULATION, 25.0),
                (MOD_RATE, 0.4),
                (DAMPING, 5000.0),
                (LOW_CUT, 200.0),
                (HIGH_CUT, 8000.0),
                (EARLY, 70.0),
            ],
        },
    ]
};

const MAX_LINES: usize = 12;
const MAX_LATE_STAGES: usize = 6;
const MAX_EARLY_STAGES: usize = 10;
const MAX_TAPS: usize = 48;
/// Modulators and read points are refreshed every this many frames (Cloud
/// Seed: 8). The lines glide between updates; the diffusers, whose
/// modulation is shallower, step.
const UPDATE: u32 = 32;
/// The Size the character tables' times are given at.
const REF_SIZE_MS: f32 = 120.0;
/// Longest line delay: 1.5 x the largest Size, plus modulation.
const MAX_LINE_S: f32 = 1.5 * 1.0 + 0.003;
/// Longest all-pass stage, before modulation.
const MAX_STAGE_S: f32 = 0.1;
const MAX_PRE_S: f32 = 0.5;
const MAX_TAP_S: f32 = 1.0;
/// Modulation depth at 100 %, ms: the lines, the line diffusers, the early
/// diffuser.
const LINE_DEPTH_MS: f32 = 2.5;
const LATE_DEPTH_MS: f32 = 1.25;
const EARLY_DEPTH_MS: f32 = 1.0;
/// The longest a single diffuser stage may ring (60 dB), as a share of the
/// decay time: the early stages run in a long series outside the loops.
const EARLY_RING: f32 = 0.15;
const LATE_RING: f32 = 0.4;
/// All-pass feedback at 100 % diffusion.
const LATE_FB: f32 = 0.88;
const EARLY_FB: f32 = 0.82;

/// The hidden structure a Character stands for.
#[derive(Clone, Copy)]
struct Character {
    lines: usize,
    /// The late output is taken after each line's diffuser and EQ (dense),
    /// or before them (distinct echoes).
    post: bool,
    late_stages: usize,
    /// The longest line-diffuser stage at the reference Size, ms.
    late_ms: f32,
    early_stages: usize,
    early_ms: f32,
    taps: usize,
    /// The span of the early taps at the reference Size, ms.
    tap_ms: f32,
    /// How strongly later taps fade (0: not at all).
    tap_decay: f32,
    /// How much of each input side reaches the other side's reverb.
    input_mix: f32,
    /// Shelves inside every line's loop: (Hz, dB); 0 dB is off.
    low_shelf: (f32, f32),
    high_shelf: (f32, f32),
    /// Seeds for the taps, the early diffuser, the lines and the line
    /// diffusers, as in Cloud Seed (0 to 999).
    seeds: [u64; 4],
    /// Output level, so the characters sound about as loud as each other
    /// (at the default settings, steady white noise in comes out about
    /// 4 dB down).
    trim_db: f32,
}

impl Character {
    /// The Light quality's structure: half the lines (at least 4), half the
    /// stages in each line's diffuser (at least 1), half the early
    /// diffuser's stages and half the early taps. The seeds, lengths, EQ
    /// and decay stay, so it keeps the character's size, colour, level and
    /// decay time and loses some density. The lines and the line diffusers
    /// are most of the cost, so it runs in about half the time (a third
    /// for the Cloud character, with the most stages).
    fn light(self) -> Character {
        Character {
            lines: self.lines.div_ceil(2).max(4).min(self.lines),
            late_stages: self.late_stages.div_ceil(2),
            early_stages: self.early_stages / 2,
            taps: self.taps / 2,
            ..self
        }
    }

    /// Character `c` at quality `q` (Light and Eco share a structure).
    fn get(c: usize, q: usize) -> Character {
        let ch = CHARS[c.min(CHARS.len() - 1)];
        if q == FULL {
            ch
        } else {
            ch.light()
        }
    }
}

static CHARS: [Character; 5] = [
    // Plate: Cloud Seed's "Dark plate" structure (12 lines, the output
    // after a 4-stage diffuser, a high shelf darkening every pass), with a
    // short early smear added.
    Character {
        lines: 12,
        post: true,
        late_stages: 4,
        late_ms: 36.0,
        early_stages: 4,
        early_ms: 8.0,
        taps: 0,
        tap_ms: 0.0,
        tap_decay: 0.0,
        input_mix: 0.235,
        low_shelf: (0.0, 0.0),
        high_shelf: (4520.0, -4.6),
        seeds: [333, 184, 218, 365],
        trim_db: 0.3,
    },
    // Room: a dense, quick pattern of early reflections into 8 short lines.
    Character {
        lines: 8,
        post: true,
        late_stages: 3,
        late_ms: 14.0,
        early_stages: 4,
        early_ms: 10.0,
        taps: 24,
        tap_ms: 40.0,
        tap_decay: 1.0,
        input_mix: 0.3,
        low_shelf: (200.0, -2.0),
        high_shelf: (6000.0, -3.0),
        seeds: [511, 96, 742, 258],
        trim_db: -4.2,
    },
    // Hall: wider early reflections and a smooth, slowly built tail.
    Character {
        lines: 10,
        post: true,
        late_stages: 4,
        late_ms: 30.0,
        early_stages: 6,
        early_ms: 24.0,
        taps: 40,
        tap_ms: 90.0,
        tap_decay: 0.8,
        input_mix: 0.25,
        low_shelf: (0.0, 0.0),
        high_shelf: (5000.0, -3.0),
        seeds: [127, 873, 405, 619],
        trim_db: -2.4,
    },
    // Cloud: heavy diffusion everywhere, the endless ambient wash.
    Character {
        lines: 12,
        post: true,
        late_stages: 6,
        late_ms: 55.0,
        early_stages: 10,
        early_ms: 60.0,
        taps: 0,
        tap_ms: 0.0,
        tap_decay: 0.0,
        input_mix: 0.5,
        low_shelf: (0.0, 0.0),
        high_shelf: (6500.0, -2.5),
        seeds: [942, 333, 777, 155],
        trim_db: 0.6,
    },
    // Echoes: few lines heard before their diffusers, and long taps, so
    // repeats stay distinct and blur as they fade.
    Character {
        lines: 4,
        post: false,
        late_stages: 2,
        late_ms: 20.0,
        early_stages: 0,
        early_ms: 0.0,
        taps: 12,
        tap_ms: 600.0,
        tap_decay: 0.5,
        input_mix: 0.15,
        low_shelf: (250.0, -3.0),
        high_shelf: (3500.0, -5.0),
        seeds: [64, 0, 909, 488],
        trim_db: -5.5,
    },
];

/// Cloud Seed's random series: a 32-bit linear congruential generator
/// (a = 22695477, c = 1) from `seed`, blended with the series from `!seed`
/// by `cross`. Values are in [0, 1].
fn series(seed: u64, cross: f32, out: &mut [f32]) {
    let (mut a, mut b) = (seed, !seed);
    let unit = |x: u64| (x as f64 / u32::MAX as f64) as f32;
    for v in out.iter_mut() {
        a = a.wrapping_mul(22_695_477).wrapping_add(1) & 0xFFFF_FFFF;
        b = b.wrapping_mul(22_695_477).wrapping_add(1) & 0xFFFF_FFFF;
        *v = unit(a) * (1.0 - cross) + unit(b) * cross;
    }
}

/// One cycle of a sine for `phase` in [0, 1): a parabola with one
/// correction (error under 0.1 %), plain arithmetic, so it is the same on
/// every machine.
#[inline]
fn lfo(phase: f32) -> f32 {
    let x = if phase < 0.5 { phase } else { phase - 1.0 };
    let y = 8.0 * x - 16.0 * x * x.abs();
    0.225 * (y * y.abs() - y) + y
}

/// Cloud Seed's one-pole coefficient for a corner at `hz`.
fn one_pole(hz: f32, rate: u32) -> f32 {
    let fs = rate as f32;
    let x = 2.0 * PI * hz.min(0.499 * fs) / fs;
    let nn = 2.0 - x.cos();
    nn - (nn * nn - 1.0).max(0.0).sqrt()
}

/// [`one_pole`] for the network at `rate`, below the `host` rate (Eco).
/// A corner above a quarter of `rate` would land near or past the
/// network's Nyquist and dull every pass round the loop, so instead the
/// coefficient matches the host-rate filter's loss at a quarter of `rate`:
/// the band both rates share loses about the same per pass. At or below a
/// quarter of `rate`, and at the host rate, it is [`one_pole`] itself.
fn one_pole_as(hz: f32, host: u32, rate: u32) -> f32 {
    let r = 0.25 * rate as f32;
    if rate == host || hz <= r {
        return one_pole(hz, rate);
    }
    let a = one_pole(hz, host);
    let w = 2.0 * PI * r / host as f32;
    let g2 = (1.0 - a) * (1.0 - a) / (1.0 + a * a - 2.0 * a * w.cos());
    if g2 >= 1.0 - 1e-6 {
        return 0.0;
    }
    // At a quarter of `rate` the cosine is 0, so the corner equation of
    // [`one_pole`] becomes nn = 1 / (1 - g2).
    let nn = 1.0 / (1.0 - g2);
    nn - (nn * nn - 1.0).max(0.0).sqrt()
}

/// Hermite interpolation between `x1` (t = 0) and `x2` (t = 1), with
/// `x0` the newer neighbour and `x3` the older one.
#[inline]
fn hermite(x0: f32, x1: f32, x2: f32, x3: f32, t: f32) -> f32 {
    let c1 = 0.5 * (x2 - x0);
    let c2 = x0 - 2.5 * x1 + 2.0 * x2 - 0.5 * x3;
    let c3 = 0.5 * (x3 - x0) + 1.5 * (x1 - x2);
    ((c3 * t + c2) * t + c1) * t + x1
}

/// [`hermite`] at a fixed `t` as four tap weights, for `x0` to `x3`: a
/// stage's read point holds still across a chunk, so its reads are a
/// 4-tap FIR worked out once per chunk (four multiplies a frame instead of
/// the polynomial).
#[inline]
fn hermite_taps(t: f32) -> [f32; 4] {
    let (t2, t3) = (t * t, t * t * t);
    [
        -0.5 * t + t2 - 0.5 * t3,
        1.0 - 2.5 * t2 + 1.5 * t3,
        0.5 * t + 2.0 * t2 - 1.5 * t3,
        -0.5 * t2 + 0.5 * t3,
    ]
}

/// A delay buffer that forgets in O(1): [`Buf::forget`] sets `filled` to 0,
/// and a read further back than the writes since then is silence, exactly
/// as if the buffer had been cleared.
struct Buf {
    data: Vec<f32>,
    w: usize,
    filled: usize,
}

impl Default for Buf {
    fn default() -> Buf {
        Buf::new(4)
    }
}

impl Buf {
    fn new(len: usize) -> Buf {
        Buf {
            data: vec![0.0; len.max(4)],
            w: 0,
            filled: 0,
        }
    }
    fn len(&self) -> usize {
        self.data.len()
    }
    #[inline]
    fn push(&mut self, x: f32) {
        self.data[self.w] = x;
        self.w = if self.w + 1 == self.data.len() {
            0
        } else {
            self.w + 1
        };
        self.filled = (self.filled + 1).min(self.data.len());
    }
    /// The sample written `d` pushes ago (1 is the last one).
    #[inline]
    fn at(&self, d: usize) -> f32 {
        if d == 0 || d > self.filled {
            return 0.0;
        }
        let i = if self.w >= d {
            self.w - d
        } else {
            self.w + self.data.len() - d
        };
        self.data[i]
    }
    /// Linear interpolation from `d` (at least 1) toward `d + 1` pushes ago.
    #[inline]
    fn lin(&self, d: usize, t: f32) -> f32 {
        if d >= self.filled {
            return lerp(self.at(d), self.at(d + 1), t);
        }
        let n = self.data.len();
        let i = if self.w > d {
            self.w - d - 1
        } else {
            self.w + n - d - 1
        };
        let j = if i + 1 == n { 0 } else { i + 1 };
        lerp(self.data[j], self.data[i], t)
    }
    /// Cubic (Hermite) interpolation from `d` (at least 2) toward `d + 1`
    /// pushes ago: exact at whole frames and nearly flat to a quarter of
    /// the sample rate, so a modulated loop does not dull itself (a linear
    /// read loses 0.7 dB at 6 kHz on every pass, this one 0.07 dB).
    #[inline]
    fn cubic(&self, d: usize, t: f32) -> f32 {
        if d + 2 > self.filled || d < 2 {
            return hermite(
                self.at(d - 1),
                self.at(d),
                self.at(d + 1),
                self.at(d + 2),
                t,
            );
        }
        let n = self.data.len();
        // The index of at(d + 2), the oldest point; the rest follow it.
        let i = if self.w > d + 1 {
            self.w - d - 2
        } else {
            self.w + n - d - 2
        };
        let at = |k: usize| self.data[if i + k >= n { i + k - n } else { i + k }];
        hermite(at(3), at(2), at(1), at(0), t)
    }
    /// The cubic read from `d` (at least 2) pushes ago with the weights of
    /// [`hermite_taps`]: a stage's read, the same for one frame or a run.
    #[inline]
    fn cubic_taps(&self, d: usize, h: &[f32; 4]) -> f32 {
        h[0] * self.at(d - 1) + h[1] * self.at(d) + h[2] * self.at(d + 1) + h[3] * self.at(d + 2)
    }
    /// `out[k] = cubic_taps(d - k, h)`, for a chunk read before any of its
    /// own writes. Needs `d > out.len()`.
    #[inline]
    fn cubic_run(&self, d: usize, h: &[f32; 4], out: &mut [f32]) {
        let n = out.len();
        let len = self.data.len();
        if d + 2 <= self.filled {
            let s = if self.w > d + 1 {
                self.w - d - 2
            } else {
                self.w + len - d - 2
            };
            if s + n + 2 < len {
                for (o, w) in out.iter_mut().zip(self.data[s..s + n + 3].windows(4)) {
                    *o = h[0] * w[3] + h[1] * w[2] + h[2] * w[1] + h[3] * w[0];
                }
                return;
            }
        }
        for (k, o) in out.iter_mut().enumerate() {
            *o = self.cubic_taps(d - k, h);
        }
    }
    /// A line's read across a chunk: `out[k] = cubic(d - k - 1, r - d)`
    /// with `r = from + glide * (at + k + 1)` and `d` its whole frames, as
    /// [`Line::run`] reads (the chunk's own writes come after). When the
    /// read point moves less than a frame per frame and the points needed
    /// sit in one unwrapped, written stretch, it indexes that stretch
    /// directly; otherwise it reads frame by frame. The same values either
    /// way.
    #[inline]
    fn cubic_glide(&self, from: f32, glide: f32, at: usize, out: &mut [f32]) {
        let n = out.len();
        let read = |k: usize| {
            let r = from + glide * (at + k + 1) as f32;
            // Truncation is floor here (r > 0) and, unlike floor, needs no
            // library call on targets without a rounding instruction.
            let d = r as usize;
            (d, r - d as f32)
        };
        // Moving well under a frame per frame (the frames' read points are
        // each rounded, by up to 0.008 of a frame at 1.5 s), `d - k` never
        // rises with k: the first frame reads furthest back, the last
        // nearest.
        if n > 0 && glide.abs() < 0.5 {
            let (d0, _) = read(0);
            let (d1, _) = read(n - 1);
            // cubic(e, t) reads at(e + 2) .. at(e - 1), and only from
            // written frames (e >= 2, e + 2 <= filled): frame k has
            // e = d - k - 1, so needs at(d - k + 1) .. at(d - k - 2).
            if d1 >= n + 2 && d0 < self.filled {
                let (far, near) = (d0 + 1, d1 - n - 1);
                if self.w >= far {
                    let base = self.w - far;
                    let seg = &self.data[base..self.w - near + 1];
                    for (k, o) in out.iter_mut().enumerate() {
                        let (d, t) = read(k);
                        // at(d - k + 1) is the oldest of the four points.
                        let i = far - (d - k + 1);
                        let p = &seg[i..i + 4];
                        *o = hermite(p[3], p[2], p[1], p[0], t);
                    }
                    return;
                }
            }
        }
        for (k, o) in out.iter_mut().enumerate() {
            let (d, t) = read(k);
            *o = self.cubic(d - k - 1, t);
        }
    }
    /// `out[k] += g * lin(d - k, t)`: reads moving forward in time, for a
    /// chunk read after its own writes. Needs `d >= out.len()`.
    #[inline]
    fn add_run(&self, d: usize, t: f32, g: f32, out: &mut [f32]) {
        let n = out.len();
        let len = self.data.len();
        if d < self.filled {
            let s = if self.w > d {
                self.w - d - 1
            } else {
                self.w + len - d - 1
            };
            if s + n < len {
                let seg = &self.data[s..s + n + 1];
                for (k, o) in out.iter_mut().enumerate() {
                    *o += g * lerp(seg[k + 1], seg[k], t);
                }
                return;
            }
        }
        for (k, o) in out.iter_mut().enumerate() {
            *o += g * self.lin(d - k, t);
        }
    }
    /// A first-order all-pass (Thiran) read from `d` toward `d + 1`:
    /// `y = at(d + 1) + eta * (at(d) - y')`, with `y'` the last output.
    #[inline]
    fn thiran(&self, d: usize, eta: f32, last: f32) -> f32 {
        flush(self.at(d + 1) + eta * (self.at(d) - last))
    }
    /// `out[k] = y[k]`, `y[k] = thiran(d - k, eta, y[k - 1])`, with `last`
    /// carrying `y` from chunk to chunk: the stages' read in Eco, for a
    /// chunk read before any of its own writes. Needs `d >= out.len()`.
    #[inline]
    fn thiran_run(&self, d: usize, eta: f32, last: &mut f32, out: &mut [f32]) {
        let n = out.len();
        let len = self.data.len();
        let mut y = *last;
        if d < self.filled {
            // at(d + 1 - k) is seg[k], at(d - k) is seg[k + 1].
            let s = if self.w > d {
                self.w - d - 1
            } else {
                self.w + len - d - 1
            };
            if s + n < len {
                let seg = &self.data[s..s + n + 1];
                for (k, o) in out.iter_mut().enumerate() {
                    y = flush(seg[k] + eta * (seg[k + 1] - y));
                    *o = y;
                }
                *last = y;
                return;
            }
        }
        for (k, o) in out.iter_mut().enumerate() {
            y = self.thiran(d - k, eta, y);
            *o = y;
        }
        *last = y;
    }
    /// [`Buf::add_run`] with a Thiran fraction: `out[k] += g * y[k]`, `y`
    /// as [`Buf::thiran_run`]: the taps' read in Eco, for a chunk read
    /// after its own writes.
    #[inline]
    fn add_run_thiran(&self, d: usize, eta: f32, g: f32, last: &mut f32, out: &mut [f32]) {
        let n = out.len();
        let len = self.data.len();
        let mut y = *last;
        if d < self.filled {
            let s = if self.w > d {
                self.w - d - 1
            } else {
                self.w + len - d - 1
            };
            if s + n < len {
                let seg = &self.data[s..s + n + 1];
                for (k, o) in out.iter_mut().enumerate() {
                    y = flush(seg[k] + eta * (seg[k + 1] - y));
                    *o += g * y;
                }
                *last = y;
                return;
            }
        }
        for (k, o) in out.iter_mut().enumerate() {
            y = self.thiran(d - k, eta, y);
            *o += g * y;
        }
        *last = y;
    }
    /// Writes a chunk (shorter than the buffer).
    #[inline]
    fn push_run(&mut self, x: &[f32]) {
        let len = self.data.len();
        let n = x.len();
        let first = (len - self.w).min(n);
        self.data[self.w..self.w + first].copy_from_slice(&x[..first]);
        self.data[..n - first].copy_from_slice(&x[first..]);
        self.w = if self.w + n >= len {
            self.w + n - len
        } else {
            self.w + n
        };
        self.filled = (self.filled + n).min(len);
    }
    fn forget(&mut self) {
        self.w = 0;
        self.filled = 0;
    }
}

/// A read point: whole frames back and the fraction toward one more.
#[derive(Clone, Copy, Default)]
struct Read {
    d: usize,
    t: f32,
}

impl Read {
    /// `frames` back, kept between `lo` and the buffer's end.
    fn new(frames: f32, lo: f32, len: usize) -> Read {
        let f = frames.clamp(lo, (len - 2) as f32);
        let d = f as usize;
        Read { d, t: f - d as f32 }
    }
}

impl Read {
    /// The read as a first-order all-pass (Thiran) fraction: whole frames
    /// and the coefficient. The fraction stays within 0.5 to 1.5 frames,
    /// where the all-pass's delay is close to flat across the band (a
    /// read under 1.5 frames back takes 0.5 at least).
    fn thiran(self) -> (usize, f32) {
        let (d, frac) = if self.t >= 0.5 || self.d < 2 {
            (self.d, self.t.max(0.5))
        } else {
            (self.d - 1, 1.0 + self.t)
        };
        (d, (1.0 - frac) / (1.0 + frac))
    }
}

/// A sine modulator, advanced every [`UPDATE`] frames.
#[derive(Clone, Copy, Default)]
struct Lfo {
    phase: f32,
    depth_k: f32,
    rate_k: f32,
}

impl Lfo {
    /// Advances by `step` cycles (times this modulator's rate factor) and
    /// returns the new value.
    #[inline]
    fn advance(&mut self, step: f32) -> f32 {
        self.phase += step * self.rate_k;
        if self.phase >= 1.0 {
            self.phase -= self.phase.floor();
        }
        lfo(self.phase)
    }
}

/// A modulated all-pass stage (Cloud Seed's ModulatedAllpass).
#[derive(Default)]
struct Allpass {
    buf: Buf,
    /// The stage's share of the diffuser delay (0.1 to 1).
    share: f32,
    phase0: f32,
    lfo: Lfo,
    /// The current delay in frames, before modulation.
    delay: f32,
    /// This chunk's read: `taps` from `d` back (as [`Buf::cubic_taps`]),
    /// or with `thiran`, [`Buf::thiran`] from `d` with `eta`.
    d: usize,
    taps: [f32; 4],
    thiran: bool,
    eta: f32,
    /// The Thiran read's last output.
    last: f32,
    /// The most feedback this stage takes: enough that its own ring dies
    /// in half the decay time, so a long stage cannot outlast Decay.
    cap: f32,
}

impl Allpass {
    /// Seeds the stage: `r` sets the delay share, `m` and `q` the depth and
    /// rate factors, `p` the start phase.
    fn seed(&mut self, r: f32, m: f32, q: f32, p: f32) {
        self.share = 0.1 * 10f32.powf(r);
        self.lfo.depth_k = 0.85 + 0.3 * m;
        self.lfo.rate_k = 0.85 + 0.3 * q;
        self.phase0 = 0.01 + 0.98 * p;
    }
    fn restart(&mut self) {
        self.buf.forget();
        self.lfo.phase = self.phase0;
        self.last = 0.0;
    }
    /// Moves the read point: `span` is the diffuser delay in frames, `depth`
    /// the modulation depth in frames, `step` the LFO step (0 holds it);
    /// with `decay` (frames), also refreshes the feedback cap; `u` gives
    /// the longest stage. With `u.thiran` (Eco) the fraction is read by a first-order all-pass
    /// (Thiran) instead of the cubic: a cubic read loses the top of the
    /// band on every pass round the stage, and at half rate that top is
    /// the 5 to 10 kHz the ear still hears; the all-pass read loses none.
    #[inline]
    fn update(
        &mut self,
        span: f32,
        depth: f32,
        step: f32,
        decay: Option<f32>,
        ring: f32,
        u: &Tick,
    ) {
        let (max, thiran) = (u.stage_max, u.thiran);
        // Whole frames, as in Cloud Seed: unmodulated, a stage is exactly
        // all-pass (a fractional read would dull every pass).
        self.delay = (span * self.share).round().clamp(MIN_STAGE, max);
        if let Some(decay) = decay {
            // 60 dB down after decay * ring: feedback^(decay * ring / delay) = 1e-3.
            self.cap = (-3.0 * LN_10 * self.delay / (decay * ring)).exp();
        }
        let amt = (depth * self.lfo.depth_k)
            .min(self.delay - MIN_STAGE)
            .max(0.0);
        let m = if step > 0.0 {
            self.lfo.advance(step)
        } else {
            lfo(self.lfo.phase)
        };
        let r = Read::new(self.delay + amt * m, MIN_STAGE, self.buf.len());
        self.thiran = thiran;
        if thiran {
            (self.d, self.eta) = r.thiran();
        } else {
            self.d = r.d;
            self.taps = hermite_taps(r.t);
        }
    }
    /// Runs the stage over a chunk in place.
    #[inline]
    fn run(&mut self, x: &mut [f32], fb: &[f32]) {
        let (d, h) = (self.d, self.taps);
        let n = x.len();
        if d > n {
            // Longer than the chunk: read it all, then write it all.
            let (mut y, mut v) = ([0.0f32; CHUNK], [0.0f32; CHUNK]);
            if self.thiran {
                self.buf
                    .thiran_run(d, self.eta, &mut self.last, &mut y[..n]);
            } else {
                self.buf.cubic_run(d, &h, &mut y[..n]);
            }
            let cap = self.cap;
            for (((x, &y), v), &g) in x.iter_mut().zip(&y[..n]).zip(&mut v[..n]).zip(fb) {
                let g = g.min(cap);
                *v = flush(*x + y * g);
                *x = y - *v * g;
            }
            self.buf.push_run(&v[..n]);
        } else {
            for (x, &g) in x.iter_mut().zip(fb) {
                let g = g.min(self.cap);
                let y = if self.thiran {
                    self.last = self.buf.thiran(d, self.eta, self.last);
                    self.last
                } else {
                    self.buf.cubic_taps(d, &h)
                };
                let v = flush(*x + y * g);
                self.buf.push(v);
                *x = y - v * g;
            }
        }
    }
}

/// One feedback delay line with its diffuser, shelves and damping (Cloud
/// Seed's DelayLine).
#[derive(Default)]
struct Line {
    buf: Buf,
    /// Delay as a multiple of Size (0.5 to 1.5).
    size_k: f32,
    phase0: f32,
    lfo: Lfo,
    delay: f32,
    /// The read point (frames back, modulated) at the last update and at
    /// this one; reads glide from one to the other across the chunk.
    from: f32,
    to: f32,
    /// Feedback gain for the current loop length and decay.
    gain: f32,
    /// The loop's output last frame.
    state: f32,
    stages: [Allpass; MAX_LATE_STAGES],
    /// First-order shelf states and the damping low-pass.
    low: f32,
    high: f32,
    damp: f32,
}

impl Line {
    fn restart(&mut self) {
        self.buf.forget();
        self.lfo.phase = self.phase0;
        self.state = 0.0;
        self.damp = 0.0;
        self.low = 0.0;
        self.high = 0.0;
        for s in &mut self.stages {
            s.restart();
        }
    }

    /// Runs the loop over a chunk of `x.len()` frames starting `at` frames
    /// into the update interval, adding its output to `out`. The loop is
    /// longer than the chunk, so every read comes from frames written
    /// before it: read the chunk, run it through the diffuser and EQ, then
    /// write it back with the input.
    #[inline]
    fn run(&mut self, x: &[f32], at: usize, c: &Character, f: &Frames, eq: &Eq, out: &mut [f32]) {
        let n = x.len();
        let mut y = [0.0f32; CHUNK];
        let glide = (self.to - self.from) / CHUNK as f32;
        // Frame k reads from its own write, which comes after k + 1 writes
        // of this chunk. The read point depends only on the frame's place
        // in the update interval, so any chunking rounds it the same way.
        self.buf.cubic_glide(self.from, glide, at, &mut y[..n]);
        let mut z = y;
        for st in &mut self.stages[..c.late_stages] {
            st.run(&mut z[..n], &f.late_fb[..n]);
        }
        let mut prev = self.state;
        let mut v = [0.0f32; CHUNK];
        for k in 0..n {
            let mut e = z[k];
            if eq.low_on {
                self.low = flush(self.low + eq.low_a * (e - self.low));
                e = eq.low_g * self.low + (e - self.low);
            }
            if eq.high_on {
                self.high = flush(self.high + eq.high_a * (e - self.high));
                e = self.high + eq.high_g * (e - self.high);
            }
            self.damp = flush((1.0 - f.damp[k]) * e + f.damp[k] * self.damp);
            let g = lerp(self.gain, 1.0, f.freeze[k]);
            v[k] = flush(x[k] + prev * g);
            // Frozen: the loop skips its EQ and damping, so it never fades.
            prev = flush(lerp(self.damp, z[k], f.freeze[k]));
            out[k] += if c.post { prev } else { y[k] };
        }
        self.buf.push_run(&v[..n]);
        self.state = prev;
    }
}

/// Frames per chunk: the modulator update interval, so read points hold
/// still within a chunk.
const CHUNK: usize = UPDATE as usize;
/// The shortest all-pass stage and line, frames: longer than a chunk, so a
/// line can run a whole chunk at once.
const MIN_STAGE: f32 = 2.0;
const MIN_LINE: f32 = CHUNK as f32 + 2.0;

/// The first-order shelves inside every line's loop.
#[derive(Clone, Copy, Default)]
struct Eq {
    low_on: bool,
    high_on: bool,
    low_a: f32,
    low_g: f32,
    high_a: f32,
    high_g: f32,
}

impl Eq {
    fn new(c: &Character, rate: u32) -> Eq {
        let a = |hz: f32| 1.0 - (-2.0 * PI * hz / rate as f32).exp();
        Eq {
            low_on: c.low_shelf.1 != 0.0,
            high_on: c.high_shelf.1 != 0.0,
            low_a: a(c.low_shelf.0),
            low_g: crate::db_to_gain(c.low_shelf.1),
            high_a: a(c.high_shelf.0),
            high_g: crate::db_to_gain(c.high_shelf.1),
        }
    }
}

/// Per-frame values for one chunk, shared by both sides.
struct Frames {
    late_fb: [f32; CHUNK],
    early_fb: [f32; CHUNK],
    /// 0 to 1: how frozen the tail is.
    freeze: [f32; CHUNK],
    /// One-pole coefficients: damping, input low cut, input high cut.
    damp: [f32; CHUNK],
    low: [f32; CHUNK],
    high: [f32; CHUNK],
    /// The pre-delay, frames.
    pre: [f32; CHUNK],
}

/// One side of the reverb (Cloud Seed's ReverbChannel).
struct Side {
    /// Input low cut (a one-pole high-pass: its low-pass state) and high cut.
    low_cut: f32,
    high_cut: f32,
    pre: Buf,
    taps: Buf,
    tap_pos: [f32; MAX_TAPS],
    tap_gain: [f32; MAX_TAPS],
    tap_read: [Read; MAX_TAPS],
    /// Eco: the taps read with a Thiran fraction, as [`Allpass`]: their
    /// coefficients and last outputs.
    tap_thiran: bool,
    tap_eta: [f32; MAX_TAPS],
    tap_last: [f32; MAX_TAPS],
    early: [Allpass; MAX_EARLY_STAGES],
    lines: [Line; MAX_LINES],
}

impl Default for Side {
    fn default() -> Side {
        Side {
            low_cut: 0.0,
            high_cut: 0.0,
            pre: Buf::default(),
            taps: Buf::default(),
            tap_pos: [0.0; MAX_TAPS],
            tap_gain: [0.0; MAX_TAPS],
            tap_read: [Read::default(); MAX_TAPS],
            tap_thiran: false,
            tap_eta: [0.0; MAX_TAPS],
            tap_last: [0.0; MAX_TAPS],
            early: Default::default(),
            lines: Default::default(),
        }
    }
}

impl Side {
    fn allocate(&mut self, rate: u32) {
        let frames = |s: f32| (s * rate as f32).ceil() as usize + CHUNK + 4;
        let stage = frames(MAX_STAGE_S + 0.002);
        self.pre = Buf::new(frames(MAX_PRE_S));
        self.taps = Buf::new(frames(MAX_TAP_S));
        for s in &mut self.early {
            s.buf = Buf::new(stage);
        }
        for line in &mut self.lines {
            line.buf = Buf::new(frames(MAX_LINE_S));
            for s in &mut line.stages {
                s.buf = Buf::new(stage);
            }
        }
    }

    /// Sets this side's structure from a character's seeds; `cross` picks
    /// the side's random series, as in Cloud Seed.
    fn seed(&mut self, c: &Character, cross: f32) {
        let mut taps = [0.0f32; 3 * MAX_TAPS];
        series(c.seeds[0], cross, &mut taps);
        let n = c.taps.max(1) as f32;
        let total = 3.0 / (1.0 + n).sqrt() * (1.0 + 2.0 * c.tap_decay);
        for j in 0..MAX_TAPS {
            let sign = if taps[3 * j] < 0.5 { 1.0 } else { -1.0 };
            let gain = crate::db_to_gain(-20.0 + 20.0 * taps[3 * j + 1]);
            let pos = j as f32 + taps[3 * j + 2];
            let fade = (-pos / n * 3.3).exp() * c.tap_decay + (1.0 - c.tap_decay);
            self.tap_pos[j] = pos;
            self.tap_gain[j] = sign * gain * fade * total;
        }
        let mut s = [0.0f32; 48];
        series(c.seeds[1], cross, &mut s);
        for (i, st) in self.early.iter_mut().enumerate() {
            st.seed(s[i], s[12 + i], s[24 + i], s[36 + i]);
        }
        series(c.seeds[2], cross, &mut s);
        for (i, line) in self.lines.iter_mut().enumerate() {
            line.lfo.depth_k = 0.7 + 0.3 * s[i];
            line.lfo.rate_k = 0.7 + 0.3 * s[12 + i];
            line.size_k = 0.5 + s[24 + i];
            line.phase0 = 0.01 + 0.98 * s[36 + i];
        }
        let mut d = [0.0f32; 48];
        for (i, line) in self.lines.iter_mut().enumerate() {
            series(c.seeds[3] * (i as u64 + 1), cross, &mut d);
            for (k, st) in line.stages.iter_mut().enumerate() {
                st.seed(d[k], d[12 + k], d[24 + k], d[36 + k]);
            }
        }
    }

    /// After a change of structure from `old` to `new`: whatever starts
    /// running again (lines, diffuser stages, the early taps) starts from
    /// silence, not from what it held when it last ran.
    fn wake(&mut self, old: &Character, new: &Character) {
        if old.taps == 0 && new.taps > 0 {
            self.taps.forget();
        }
        for last in self.tap_last.iter_mut().take(new.taps).skip(old.taps) {
            *last = 0.0;
        }
        for st in self
            .early
            .iter_mut()
            .take(new.early_stages)
            .skip(old.early_stages)
        {
            st.restart();
        }
        for (i, line) in self.lines.iter_mut().enumerate().take(new.lines) {
            if i >= old.lines {
                line.restart();
            } else {
                for st in line
                    .stages
                    .iter_mut()
                    .take(new.late_stages)
                    .skip(old.late_stages)
                {
                    st.restart();
                }
            }
        }
    }

    fn restart(&mut self) {
        self.low_cut = 0.0;
        self.high_cut = 0.0;
        self.pre.forget();
        self.taps.forget();
        self.tap_last = [0.0; MAX_TAPS];
        for s in &mut self.early {
            s.restart();
        }
        for line in &mut self.lines {
            line.restart();
        }
    }

    /// Moves every read point and, when `gains`, recomputes the feedback
    /// gains. `step` advances the modulators (0 holds them).
    fn update(&mut self, c: &Character, u: &Tick, step: f32, gains: bool) {
        self.tap_thiran = u.thiran;
        for j in 0..c.taps {
            let r = Read::new(self.tap_pos[j] * u.tap_scale + 1.0, 1.0, self.taps.len());
            self.tap_read[j] = r;
            if u.thiran {
                // The taps are fixed, but a linear read of a fraction dulls
                // what half rate keeps of the top: read them as the stages.
                let (d, eta) = r.thiran();
                self.tap_read[j] = Read { d, t: 0.0 };
                self.tap_eta[j] = eta;
            }
        }
        let decay = gains.then_some(u.decay);
        for st in &mut self.early[..c.early_stages] {
            st.update(
                u.early_span,
                u.early_depth,
                step * u.early_rate,
                decay,
                EARLY_RING,
                u,
            );
        }
        for line in &mut self.lines[..c.lines] {
            let mut lp = 0.0;
            for st in &mut line.stages[..c.late_stages] {
                st.update(
                    u.late_span,
                    u.late_depth,
                    step * u.late_rate,
                    decay,
                    LATE_RING,
                    u,
                );
                lp += st.delay;
            }
            let amt = u.line_depth * line.lfo.depth_k;
            // Whole frames, as in Cloud Seed, so an unmodulated loop is
            // lossless (Freeze then holds for good).
            line.delay = (u.size * line.size_k).round().max((amt + MIN_LINE).ceil());
            let m = if step > 0.0 {
                line.lfo.advance(step)
            } else {
                lfo(line.lfo.phase)
            };
            let to = (line.delay + amt * m).clamp(MIN_LINE, (line.buf.len() - 2) as f32);
            // A held update (a reset or a new character) jumps; a moving
            // one glides from the last point.
            line.from = if step > 0.0 { line.to } else { to };
            line.to = to;
            if gains {
                // 60 dB down after `decay` frames, counting the diffuser in
                // the loop length.
                line.gain = (-3.0 * LN_10 * (line.delay + lp) / u.decay).exp();
            }
        }
    }

    /// One chunk: `x` is this side's input; adds the early signal to
    /// `early` and the late signal to `late`.
    #[inline]
    #[allow(clippy::too_many_arguments)]
    fn run(
        &mut self,
        x: &mut [f32],
        at: usize,
        c: &Character,
        f: &Frames,
        eq: &Eq,
        early: &mut [f32],
        late: &mut [f32],
    ) {
        let n = x.len();
        for v in x.iter_mut().zip(&f.low[..n]).zip(&f.high[..n]) {
            let ((x, &lo), &hi) = v;
            // Low cut: a one-pole high-pass (input minus its low-pass).
            self.low_cut = flush((1.0 - lo) * *x + lo * self.low_cut);
            let v = *x - self.low_cut;
            self.high_cut = flush((1.0 - hi) * v + hi * self.high_cut);
            *x = self.high_cut;
        }
        // Writes before reads: frame k's read sits n - 1 - k frames further
        // back from the end of the chunk.
        self.pre.push_run(x);
        for (k, x) in x.iter_mut().enumerate() {
            // The fraction comes from the pre-delay alone, so any chunking
            // rounds it the same way.
            let r = Read::new(f.pre[k] + 1.0, 1.0, self.pre.len() - CHUNK);
            *x = self.pre.lin(r.d + n - 1 - k, r.t);
        }
        if c.taps > 0 {
            self.taps.push_run(x);
            let mut y = [0.0f32; CHUNK];
            if self.tap_thiran {
                for j in 0..c.taps {
                    self.taps.add_run_thiran(
                        self.tap_read[j].d + n - 1,
                        self.tap_eta[j],
                        self.tap_gain[j],
                        &mut self.tap_last[j],
                        &mut y[..n],
                    );
                }
            } else {
                for (r, &g) in self.tap_read[..c.taps].iter().zip(&self.tap_gain) {
                    self.taps.add_run(r.d + n - 1, r.t, g, &mut y[..n]);
                }
            }
            x.copy_from_slice(&y[..n]);
        }
        for st in &mut self.early[..c.early_stages] {
            st.run(x, &f.early_fb[..n]);
        }
        early[..n].copy_from_slice(x);
        let mut sum = [0.0f32; CHUNK];
        for line in &mut self.lines[..c.lines] {
            line.run(x, at, c, f, eq, &mut sum[..n]);
        }
        let norm = 1.0 / (c.lines as f32).sqrt();
        for (l, s) in late.iter_mut().zip(&sum[..n]) {
            *l = s * norm;
        }
    }
}

/// Taps of Eco's half-band resamplers: flat to about 0.19 of the node's
/// rate, so the reverb keeps up to about 8.5 kHz at 44.1 kHz and 9 kHz at
/// 48 kHz.
const ECO_TAPS: usize = 63;
/// How much later the wet signal comes out at Eco, node frames: both
/// half-bands, plus one pair waiting to be complete.
pub(crate) const ECO_DELAY: usize = ECO_TAPS - 1 + 2;
/// Silent network frames a change of rate waits for: the up-sampler's
/// history, so nothing it still holds is cut off.
const ECO_DRAIN: u32 = ECO_TAPS as u32 / 2 + 1;

/// Eco's resamplers: the input down to the network's half rate and the wet
/// signal back up, with a two-frame queue between the pairs the network
/// takes and the frames the node gives back.
struct EcoRate {
    down: [Down2; 2],
    up: [Up2; 2],
    /// The first frame of a pair still waiting for its second.
    first: (f32, f32),
    held: usize,
    /// The wet signal at the node's rate, oldest first, as a ring.
    queue: [[f32; QUEUE]; 2],
    head: usize,
    len: usize,
}

/// Room for one chunk's frames (two per network frame) plus the two the
/// queue starts with.
const QUEUE: usize = 2 * CHUNK + 4;

impl EcoRate {
    fn new() -> EcoRate {
        let mut e = EcoRate {
            down: [Down2::new(ECO_TAPS), Down2::new(ECO_TAPS)],
            up: [Up2::new(ECO_TAPS), Up2::new(ECO_TAPS)],
            first: (0.0, 0.0),
            held: 0,
            queue: [[0.0; QUEUE]; 2],
            head: 0,
            len: 0,
        };
        e.reset();
        e
    }
    fn reset(&mut self) {
        for d in &mut self.down {
            d.reset();
        }
        for u in &mut self.up {
            u.reset();
        }
        self.held = 0;
        self.queue = [[0.0; QUEUE]; 2];
        self.head = 0;
        // Two frames of silence first: a pair's wet signal comes out after
        // its second frame has gone in.
        self.len = 2;
    }
    /// One node frame in; every second one completes a pair and gives the
    /// network's next input frame.
    #[inline]
    fn down(&mut self, l: f32, r: f32) -> Option<(f32, f32)> {
        if self.held == 0 {
            self.first = (l, r);
            self.held = 1;
            return None;
        }
        self.held = 0;
        Some((
            self.down[0].run([self.first.0, l]),
            self.down[1].run([self.first.1, r]),
        ))
    }
    /// One network frame of the wet signal in, two node frames queued.
    #[inline]
    fn up(&mut self, l: f32, r: f32) {
        let (a, b) = (self.up[0].run(l), self.up[1].run(r));
        for (x, y) in a.into_iter().zip(b) {
            // One wrap at most (head and len are under QUEUE): no divide.
            let at = self.head + self.len;
            let at = if at >= QUEUE { at - QUEUE } else { at };
            self.queue[0][at] = x;
            self.queue[1][at] = y;
            self.len += 1;
        }
    }
    /// The next node frame of the wet signal.
    #[inline]
    fn pop(&mut self) -> (f32, f32) {
        let v = (self.queue[0][self.head], self.queue[1][self.head]);
        self.head = if self.head + 1 == QUEUE {
            0
        } else {
            self.head + 1
        };
        self.len -= 1;
        v
    }
}

/// The values a modulator update needs.
struct Tick {
    /// Size in frames.
    size: f32,
    early_span: f32,
    late_span: f32,
    stage_max: f32,
    line_depth: f32,
    late_depth: f32,
    early_depth: f32,
    /// LFO rate factors for the diffusers (the lines' is 1).
    early_rate: f32,
    late_rate: f32,
    /// Decay in frames.
    decay: f32,
    /// Early taps: frames per unit of tap position.
    tap_scale: f32,
    /// The stages read their fractions with an all-pass (Eco).
    thiran: bool,
}

/// The Cloud reverb: a port of Cloud Seed with five characters, freeze and
/// gliding controls.
pub struct Cloud {
    v: [f32; 15],
    /// The rate the reverb's network runs at: the node's own rate, or half
    /// of it at Eco.
    rate: u32,
    /// The node's rate.
    host: u32,
    /// The character and quality in use, and the ones asked for (they
    /// differ during the dip that hides a change), and the structure in use.
    ch: usize,
    want: usize,
    q: usize,
    want_q: usize,
    c: Character,
    dip: f32,
    dip_step: f32,
    /// Network frames run silent (the dip at 0) while a change of rate
    /// waits: it waits until Eco's resamplers hold only silence.
    quiet: u32,
    eco: Box<EcoRate>,
    size: Smooth,
    log_decay: Smooth,
    pre: Smooth,
    diff: Smooth,
    modu: Smooth,
    log_rate: Smooth,
    log_damp: Smooth,
    log_low: Smooth,
    log_high: Smooth,
    width: Smooth,
    early: Smooth,
    mix: Smooth,
    frozen: Smooth,
    damp_a: f32,
    low_a: f32,
    high_a: f32,
    /// Frames until the next modulator update.
    tick: u32,
    /// The size and decay the gains were last computed for.
    gain_for: (f32, f32),
    /// The character's output trim, linear.
    trim: f32,
    eq: Eq,
    sides: Box<[Side; 2]>,
}

impl Default for Cloud {
    fn default() -> Self {
        Self::new()
    }
}

impl Cloud {
    /// Allocates for 48 kHz (about 11 MB); [`Node::prepare`] reallocates for
    /// other rates.
    pub fn new() -> Cloud {
        let v = defaults(&PARAMS);
        let ch = v[0] as usize;
        let mut c = Cloud {
            v,
            rate: 0,
            host: 0,
            ch,
            want: ch,
            q: v[14] as usize,
            want_q: v[14] as usize,
            c: Character::get(ch, v[14] as usize),
            dip: 1.0,
            dip_step: 0.0,
            quiet: 0,
            eco: Box::new(EcoRate::new()),
            size: Smooth::new(v[1]),
            log_decay: Smooth::new(v[2].log2()),
            pre: Smooth::new(v[3]),
            diff: Smooth::new(v[4] / 100.0),
            modu: Smooth::new(v[5] / 100.0),
            log_rate: Smooth::new(v[6].log2()),
            log_damp: Smooth::new(v[7].log2()),
            log_low: Smooth::new(v[8].log2()),
            log_high: Smooth::new(v[9].log2()),
            width: Smooth::new(v[10] / 100.0),
            early: Smooth::new(v[11] / 100.0),
            mix: Smooth::new(v[12] / 100.0),
            frozen: Smooth::new(v[13]),
            damp_a: 0.0,
            low_a: 0.0,
            high_a: 0.0,
            tick: 0,
            gain_for: (0.0, 0.0),
            trim: 1.0,
            eq: Eq::default(),
            sides: Box::default(),
        };
        c.prepare(48_000, mpfree_node::MAX_BLOCK);
        c.reset();
        c
    }

    fn apply(&mut self, id: u16, value: f32) {
        let Some(p) = PARAMS.get(id as usize) else {
            return;
        };
        let v = p.clamp(value);
        self.v[id as usize] = v;
        match id {
            ids::CHARACTER => self.want = v as usize,
            ids::QUALITY => self.want_q = v as usize,
            ids::SIZE => self.size.target = v,
            ids::DECAY => self.log_decay.target = v.log2(),
            ids::PRE_DELAY => self.pre.target = v,
            ids::DIFFUSION => self.diff.target = v / 100.0,
            ids::MODULATION => self.modu.target = v / 100.0,
            ids::MOD_RATE => self.log_rate.target = v.log2(),
            ids::DAMPING => self.log_damp.target = v.log2(),
            ids::LOW_CUT => self.log_low.target = v.log2(),
            ids::HIGH_CUT => self.log_high.target = v.log2(),
            ids::WIDTH => self.width.target = v / 100.0,
            ids::EARLY => self.early.target = v / 100.0,
            ids::MIX => self.mix.target = v / 100.0,
            ids::FREEZE => self.frozen.target = v,
            _ => {}
        }
    }

    /// The network's rate at quality `q`.
    fn rate_for(&self, q: usize) -> u32 {
        if q == ECO {
            (self.host / 2).max(1)
        } else {
            self.host
        }
    }

    /// Sets the network's rate for quality `q` and everything timed by it.
    /// The mix glides at the node's rate: at Eco it is applied outside the
    /// network.
    fn set_rate(&mut self, q: usize) {
        let rate = self.rate_for(q);
        self.rate = rate;
        for s in [
            &mut self.size,
            &mut self.pre,
            &mut self.log_damp,
            &mut self.log_low,
            &mut self.log_high,
        ] {
            s.set_time(rate, 60.0);
        }
        for s in [
            &mut self.log_decay,
            &mut self.diff,
            &mut self.modu,
            &mut self.log_rate,
        ] {
            s.set_time(rate, 100.0);
        }
        for s in [&mut self.width, &mut self.early] {
            s.set_time(rate, 20.0);
        }
        self.mix.set_time(self.host, 20.0);
        self.frozen.set_time(rate, 30.0);
        self.dip_step = 1.0 / (0.015 * rate as f32);
        self.filters();
    }

    /// Switches to character `c` at quality `q`: new seeds, taps, stages
    /// and shelves. A change of rate (to or from Eco) starts every line
    /// and the resamplers from silence.
    fn configure(&mut self, c: usize, q: usize) {
        self.ch = c.min(CHARS.len() - 1);
        let rerate = self.rate_for(q) != self.rate;
        self.q = q;
        if rerate {
            self.set_rate(q);
            for side in self.sides.iter_mut() {
                side.restart();
            }
            self.eco.reset();
        }
        self.quiet = 0;
        let old = self.c;
        self.c = Character::get(self.ch, q);
        let ch = &self.c;
        self.trim = crate::db_to_gain(ch.trim_db);
        self.eq = Eq::new(ch, self.rate);
        // Cloud Seed's crossSeed with full width: the left side runs the
        // `!seed` series, the right side the `seed` series.
        for (side, cross) in self.sides.iter_mut().zip([1.0, 0.0]) {
            side.seed(ch, cross);
            side.wake(&old, ch);
        }
        self.gain_for = (0.0, 0.0);
        self.update(false);
    }

    fn tick_values(&self) -> Tick {
        let fs = self.rate as f32;
        let ms = fs / 1000.0;
        let c = &self.c;
        let scale = self.size.value / REF_SIZE_MS;
        let m = self.modu.value;
        let tap_span = (c.tap_ms * scale).min(MAX_TAP_S * 1000.0);
        Tick {
            size: self.size.value * ms,
            early_span: c.early_ms * scale * ms,
            late_span: c.late_ms * scale * ms,
            stage_max: (MAX_STAGE_S * fs).max(MIN_STAGE),
            line_depth: LINE_DEPTH_MS * m * ms,
            late_depth: LATE_DEPTH_MS * m * ms,
            early_depth: EARLY_DEPTH_MS * m * ms,
            early_rate: 1.1,
            late_rate: 0.6,
            decay: self.log_decay.value.exp2() * ms,
            tap_scale: tap_span * ms / c.taps.max(1) as f32,
            thiran: self.rate != self.host,
        }
    }

    /// Refreshes every read point (and the gains when size or decay moved);
    /// `advance` steps the modulators.
    fn update(&mut self, advance: bool) {
        let u = self.tick_values();
        let step = if advance {
            self.log_rate.value.exp2() / self.rate as f32 * UPDATE as f32
        } else {
            0.0
        };
        let now = (self.size.value, self.log_decay.value);
        let gains = now != self.gain_for;
        self.gain_for = now;
        let c = &self.c;
        for side in self.sides.iter_mut() {
            side.update(c, &u, step, gains);
        }
    }

    fn filters(&mut self) {
        self.damp_a = one_pole_as(self.log_damp.value.exp2(), self.host, self.rate);
        self.low_a = one_pole_as(self.log_low.value.exp2(), self.host, self.rate);
        self.high_a = one_pole_as(self.log_high.value.exp2(), self.host, self.rate);
    }

    /// Runs up to one chunk (it ends at the next modulator update, at the
    /// end of `left`, or where a Character change takes effect) and returns
    /// how many frames it ran.
    /// Whether a change waiting on the dip moves the network to another
    /// rate (to or from Eco).
    fn rerate_pending(&self) -> bool {
        self.rate_for(self.want_q) != self.rate
    }

    /// Runs the network over up to one chunk (to the next modulator
    /// update), in place. With `wet_only` (Eco, at half rate) it writes
    /// the wet signal alone and leaves the mix to [`Cloud::run_eco`].
    fn run_chunk(&mut self, left: &mut [f32], right: &mut [f32], wet_only: bool) -> usize {
        let at = self.tick as usize;
        let room = (UPDATE - self.tick) as usize;
        let mut f = Frames {
            late_fb: [0.0; CHUNK],
            early_fb: [0.0; CHUNK],
            freeze: [0.0; CHUNK],
            damp: [0.0; CHUNK],
            low: [0.0; CHUNK],
            high: [0.0; CHUNK],
            pre: [0.0; CHUNK],
        };
        let (mut gain, mut early, mut width, mut mix) = (
            [0.0f32; CHUNK],
            [0.0f32; CHUNK],
            [0.0f32; CHUNK],
            [0.0f32; CHUNK],
        );
        let ms = self.rate as f32 / 1000.0;
        let mut n = 0;
        while n < room.min(left.len()) {
            if (self.want, self.want_q) != (self.ch, self.q) {
                let next = self.dip - self.dip_step;
                if next <= 0.0 && self.rerate_pending() {
                    // A change of rate waits, silent, for the next update
                    // after the resamplers have emptied ([`Cloud::run`]).
                    self.dip = 0.0;
                    self.quiet = self.quiet.saturating_add(1);
                } else if next <= 0.0 {
                    // The swap starts a chunk of its own.
                    if n > 0 {
                        break;
                    }
                    self.dip = 0.0;
                    self.configure(self.want, self.want_q);
                } else {
                    self.dip = next;
                    self.quiet = 0;
                }
            } else if self.dip < 1.0 {
                self.dip = (self.dip + self.dip_step).min(1.0);
                self.quiet = 0;
            }
            self.size.next();
            self.log_decay.next();
            self.modu.next();
            self.log_rate.next();
            let mut moved = false;
            for s in [&mut self.log_damp, &mut self.log_low, &mut self.log_high] {
                if !s.settled() {
                    s.next();
                    moved = true;
                }
            }
            if moved {
                self.filters();
            }
            if n == 0 && self.tick == 0 {
                self.update(true);
            }
            let diff = self.diff.next();
            f.late_fb[n] = LATE_FB * diff;
            f.early_fb[n] = EARLY_FB * diff;
            f.freeze[n] = self.frozen.next();
            f.damp[n] = self.damp_a;
            f.low[n] = self.low_a;
            f.high[n] = self.high_a;
            f.pre[n] = self.pre.next() * ms;
            gain[n] = self.trim * self.dip;
            early[n] = self.early.next();
            width[n] = self.width.next();
            if !wet_only {
                mix[n] = self.mix.next();
            }
            n += 1;
        }
        self.tick = (self.tick + n as u32) % UPDATE;

        let c = &self.c;
        let cm = 0.5 * c.input_mix;
        let (mut xl, mut xr) = ([0.0f32; CHUNK], [0.0f32; CHUNK]);
        for k in 0..n {
            let inp = 1.0 - f.freeze[k];
            xl[k] = (left[k] * (1.0 - cm) + right[k] * cm) * inp;
            xr[k] = (right[k] * (1.0 - cm) + left[k] * cm) * inp;
        }
        let (mut el, mut ll, mut er, mut lr) = (
            [0.0f32; CHUNK],
            [0.0f32; CHUNK],
            [0.0f32; CHUNK],
            [0.0f32; CHUNK],
        );
        let eq = self.eq;
        let [a, b] = &mut *self.sides;
        a.run(&mut xl[..n], at, c, &f, &eq, &mut el, &mut ll);
        b.run(&mut xr[..n], at, c, &f, &eq, &mut er, &mut lr);
        for k in 0..n {
            let (wl, wr) = (ll[k] + early[k] * el[k], lr[k] + early[k] * er[k]);
            let (m, s) = (0.5 * (wl + wr), 0.5 * (wl - wr) * width[k]);
            if wet_only {
                left[k] = (m + s) * gain[k];
                right[k] = (m - s) * gain[k];
            } else {
                left[k] = lerp(left[k], (m + s) * gain[k], mix[k]);
                right[k] = lerp(right[k], (m - s) * gain[k], mix[k]);
            }
        }
        n
    }

    /// Eco: up to one chunk of the network at half rate, between a 2x
    /// down-sampler and a 2x up-sampler, with the mix at the node's rate.
    /// Takes host frames up to the next modulator update (so a change of
    /// rate always falls on the same frame, however the stream is cut into
    /// blocks) and returns how many it took. The wet signal comes out
    /// [`ECO_DELAY`] frames later than at the other qualities.
    fn run_eco(&mut self, left: &mut [f32], right: &mut [f32]) -> usize {
        let room = (UPDATE - self.tick) as usize;
        let take = (2 * room - self.eco.held).min(left.len());
        let (mut hl, mut hr) = ([0.0f32; CHUNK], [0.0f32; CHUNK]);
        let mut m = 0;
        for i in 0..take {
            if let Some((l, r)) = self.eco.down(left[i], right[i]) {
                hl[m] = l;
                hr[m] = r;
                m += 1;
            }
        }
        let mut k = 0;
        while k < m {
            k += self.run_chunk(&mut hl[k..m], &mut hr[k..m], true);
        }
        for j in 0..m {
            self.eco.up(hl[j], hr[j]);
        }
        for i in 0..take {
            let (wl, wr) = self.eco.pop();
            let mix = self.mix.next();
            left[i] = lerp(left[i], wl, mix);
            right[i] = lerp(right[i], wr, mix);
        }
        take
    }

    fn run(&mut self, left: &mut [f32], right: &mut [f32]) {
        let mut at = 0;
        while at < left.len() {
            // A change to or from Eco happens at a modulator update, after
            // the dip, once the resamplers hold only silence.
            if self.tick == 0 && self.dip == 0.0 && self.quiet >= ECO_DRAIN && self.rerate_pending()
            {
                self.configure(self.want, self.want_q);
            }
            at += if self.q == ECO {
                self.run_eco(&mut left[at..], &mut right[at..])
            } else {
                self.run_chunk(&mut left[at..], &mut right[at..], false)
            };
        }
    }
}

impl Node for Cloud {
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
        if rate != self.host {
            // Sized for the node's rate: Eco uses half of each buffer.
            for side in self.sides.iter_mut() {
                side.allocate(rate);
            }
            self.host = rate;
            self.set_rate(self.q);
            self.configure(self.ch, self.q);
            for side in self.sides.iter_mut() {
                side.restart();
            }
            self.eco.reset();
        }
        self.set_rate(self.q);
    }
    fn reset(&mut self) {
        for s in [
            &mut self.size,
            &mut self.log_decay,
            &mut self.pre,
            &mut self.diff,
            &mut self.modu,
            &mut self.log_rate,
            &mut self.log_damp,
            &mut self.log_low,
            &mut self.log_high,
            &mut self.width,
            &mut self.early,
            &mut self.mix,
            &mut self.frozen,
        ] {
            s.snap(s.target);
        }
        self.dip = 1.0;
        self.tick = 0;
        self.configure(self.want, self.want_q);
        for side in self.sides.iter_mut() {
            side.restart();
        }
        self.eco.reset();
        self.update(false);
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
    fn tail_frames(&self) -> u32 {
        if self.v[13] >= 0.5 {
            return u32::MAX;
        }
        let c = &Character::get(self.want, self.want_q);
        let scale = self.v[1] / REF_SIZE_MS;
        // How long a diffuser rings: passes to fall 90 dB, times its
        // longest stage.
        let ring = |fb: f32, ms: f32, stages: usize| {
            if stages == 0 {
                return 0.0;
            }
            let fb = (fb * self.v[4] / 100.0).clamp(0.01, 0.99);
            (90.0 / (-20.0 * fb.log10())).min(200.0) * ms * scale / 1000.0
        };
        // 90 dB down (1.5 x the 60 dB decay), after the pre-delay, the
        // taps, the longest line and the diffusers have all spoken.
        let secs = 1.5 * self.v[2] / 1000.0
            + self.v[3] / 1000.0
            + (c.tap_ms * scale / 1000.0).min(MAX_TAP_S)
            + 1.5 * self.v[1] / 1000.0
            + ring(EARLY_FB, c.early_ms, c.early_stages)
            + ring(LATE_FB, c.late_ms, c.late_stages)
            + 0.1;
        let eco = if self.want_q == ECO {
            ECO_DELAY as f32 / self.host as f32
        } else {
            0.0
        };
        ((secs + eco) * self.host as f32).min(u32::MAX as f32 - 1.0) as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn series_matches_cloud_seed() {
        // Cloud Seed's LcgRandom from seed 1: x = 22695477 * 1 + 1.
        let mut s = [0.0f32; 2];
        series(1, 0.0, &mut s);
        assert_eq!(s[0], (22_695_478f64 / u32::MAX as f64) as f32);
        let x2 = (22_695_477u64 * 22_695_478 + 1) & 0xFFFF_FFFF;
        assert_eq!(s[1], (x2 as f64 / u32::MAX as f64) as f32);
        // Cross 1 is the `!seed` series.
        let mut b = [0.0f32; 1];
        series(1, 1.0, &mut b);
        let y = (!1u64).wrapping_mul(22_695_477).wrapping_add(1) & 0xFFFF_FFFF;
        assert_eq!(b[0], (y as f64 / u32::MAX as f64) as f32);
    }

    #[test]
    fn lfo_is_a_sine() {
        for i in 0..1000 {
            let p = i as f32 / 1000.0;
            assert!((lfo(p) - (2.0 * PI * p).sin()).abs() < 0.002, "{p}");
        }
    }

    #[test]
    fn a_forgotten_buffer_reads_silence() {
        let mut b = Buf::new(8);
        for x in 1..=20 {
            b.push(x as f32);
        }
        assert_eq!(b.at(1), 20.0);
        assert_eq!(b.at(8), 13.0);
        b.forget();
        b.push(5.0);
        assert_eq!(b.at(1), 5.0);
        assert_eq!(b.at(2), 0.0, "older than the reset");
        assert_eq!(b.lin(1, 0.5), 2.5);
    }

    #[test]
    fn bulk_reads_match_single_reads() {
        // The run paths take a fast slice when the span does not wrap and
        // fall back otherwise; both must give what one read at a time does,
        // across the wrap and before the buffer has filled.
        let mut b = Buf::new(40);
        for w in 0..120 {
            for (d, n) in [(9, 6), (20, 8), (35, 3)] {
                for t in [0.0, 0.3, 0.99] {
                    let mut run = [0.0f32; 8];
                    let h = hermite_taps(t);
                    b.cubic_run(d, &h, &mut run[..n]);
                    for (k, &v) in run[..n].iter().enumerate() {
                        assert_eq!(v, b.cubic_taps(d - k, &h), "cubic w {w} d {d} k {k} t {t}");
                        // The FIR form is the polynomial's, to rounding.
                        assert!(
                            (v - b.cubic(d - k, t)).abs() < 1e-6,
                            "taps w {w} d {d} k {k} t {t}"
                        );
                    }
                    let mut add = [1.0f32; 8];
                    b.add_run(d, t, 0.5, &mut add[..n]);
                    for (k, &v) in add[..n].iter().enumerate() {
                        assert_eq!(
                            v,
                            1.0 + 0.5 * b.lin(d - k, t),
                            "lin w {w} d {d} k {k} t {t}"
                        );
                    }
                    let (m, eta) = Read { d, t }.thiran();
                    let (mut add, mut last) = ([1.0f32; 8], 0.25);
                    b.add_run_thiran(m, eta, 0.5, &mut last, &mut add[..n]);
                    let (mut run, mut last_run) = ([0.0f32; 8], 0.25);
                    b.thiran_run(m, eta, &mut last_run, &mut run[..n]);
                    let mut y = 0.25;
                    for (k, (&v, &r)) in add[..n].iter().zip(&run[..n]).enumerate() {
                        y = b.thiran(m - k, eta, y);
                        assert_eq!(v, 1.0 + 0.5 * y, "thiran add w {w} d {d} k {k} t {t}");
                        assert_eq!(r, y, "thiran run w {w} d {d} k {k} t {t}");
                    }
                    assert_eq!((last, last_run), (y, y));
                }
            }
            b.push((w as f32 * 0.37).sin());
        }
    }

    #[test]
    fn a_thiran_read_has_the_delay_and_keeps_the_top() {
        // Read as the taps read (after the frame's own write): a ramp comes
        // out the read point's frames late, and a tone at 0.4 of the rate
        // (where a cubic read halfway between frames loses 7 dB) comes
        // out at full level.
        for pos in [1.0f32, 1.3, 2.5, 3.7, 6.49, 6.5, 6.51, 20.02] {
            let (m, eta) = Read::new(pos + 1.0, 1.0, 64).thiran();
            assert!((-0.21..=0.34).contains(&eta), "pos {pos}: eta {eta}");
            let (mut ramp, mut tone) = (Buf::new(64), Buf::new(64));
            let (mut lr, mut lt) = (0.0f32, 0.0f32);
            let mut power = 0.0f64;
            for i in 0..400 {
                ramp.push(i as f32);
                tone.push((0.8 * std::f32::consts::PI * i as f32).sin());
                let (mut yr, mut yt) = ([0.0f32], [0.0f32]);
                ramp.add_run_thiran(m, eta, 1.0, &mut lr, &mut yr);
                tone.add_run_thiran(m, eta, 1.0, &mut lt, &mut yt);
                if i >= 100 {
                    assert!(
                        (yr[0] - (i as f32 - pos)).abs() < 2e-3,
                        "pos {pos} frame {i}: {} for {}",
                        yr[0],
                        i as f32 - pos
                    );
                    power += (yt[0] * yt[0]) as f64;
                }
            }
            // 300 frames: a whole number of the tone's 5-frame periods.
            let level = (2.0 * power / 300.0).sqrt();
            assert!((level - 1.0).abs() < 1e-3, "pos {pos}: tone level {level}");
        }
    }

    #[test]
    fn a_gliding_line_read_matches_frame_by_frame_reads() {
        // Line::run's read, the fast stretch or the fallback, across the
        // wrap, before the buffer has filled and with the read point
        // gliding either way (or faster than the fast path takes).
        let mut b = Buf::new(64);
        for w in 0..200 {
            for from in [10.4f32, 23.0, 37.75, 61.9] {
                for glide in [0.0f32, 0.013, -0.02, 0.37, -0.45, 0.8, -1.3] {
                    for (at, n) in [(0usize, 8usize), (5, 3), (12, 1)] {
                        // A line never reads closer than its chunk plus two.
                        let close = (0..n).any(|k| {
                            let r = from + glide * (at + k + 1) as f32;
                            r < (k + 3) as f32
                        });
                        if close {
                            continue;
                        }
                        let mut got = [0.0f32; 8];
                        b.cubic_glide(from, glide, at, &mut got[..n]);
                        for (k, &v) in got[..n].iter().enumerate() {
                            let r = from + glide * (at + k + 1) as f32;
                            let d = r as usize;
                            let want = b.cubic(d - k - 1, r - d as f32);
                            assert_eq!(
                                v.to_bits(),
                                want.to_bits(),
                                "w {w} from {from} glide {glide} at {at} k {k}"
                            );
                        }
                    }
                }
            }
            b.push((w as f32 * 0.29).sin() + 0.1);
        }
    }
}
