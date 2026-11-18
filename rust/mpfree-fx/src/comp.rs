//! Compressor: a feed-forward, soft-knee compressor with the gain computed
//! and smoothed in decibels (Giannoulis, Massberg and Reiss, "Digital
//! Dynamic Range Compressor Design", JAES 2012: the smooth, decoupled peak
//! detector), with parallel mix and a sidechain input for ducking.
//!
//! The detector hears the strip (or the sidechain key) through an optional
//! high-pass, so a kick's low end doesn't pump the whole mix, and reads
//! either the peak or the RMS level. RMS is calibrated so a steady sine
//! reads the same in both: switching changes how hard short hits push it,
//! not where the threshold sits.
//!
//! The reduction the curve asks for goes through a peak hold that jumps up
//! at once and lets go at the release, then through the attack's one-pole:
//! a steady tone gets the whole reduction drawn, Attack is a time constant
//! (63 % of the way in that time) and the gain barely moves inside a bass
//! cycle. Release is matched to its label through the attack after it.
//! **Style** picks how the gain moves and colours ([`STYLES`]): Clean as
//! set; Opto, Bus and FET add a slow stage that charges during long
//! reduction and holds part of it back (a program-dependent release), Opto
//! eases its attack in and widens the knee, FET adds harmonics as it turns
//! down. Range caps the reduction; Link sets how much each side follows
//! the louder one; auto makeup adds back what the curve takes from a sound
//! at a mix level.

use mpfree_node::{Ctx, Curve, Kind, Node, NodeEvent, NodeInfo, ParamInfo, Smooth, Timed, Unit};

use crate::svf::{Segs, Svf, SvfMode};
use crate::{defaults, fast_db_to_gain, fast_level_db};

/// [`Compressor`] parameter ids.
pub mod ids {
    pub const THRESHOLD: u16 = 0;
    pub const RATIO: u16 = 1;
    pub const ATTACK: u16 = 2;
    pub const RELEASE: u16 = 3;
    pub const KNEE: u16 = 4;
    pub const MAKEUP: u16 = 5;
    pub const MIX: u16 = 6;
    pub const SIDECHAIN: u16 = 7;
    /// Meter: gain reduction now, in dB (0 or below).
    pub const REDUCTION: u16 = 8;
    /// The detector's high-pass, in Hz; off at its lowest, 20 Hz.
    pub const SC_FILTER: u16 = 9;
    /// What the detector reads: 0 Peak, 1 RMS.
    pub const DETECT: u16 = 10;
    /// Auto makeup: 0 off, 1 on: adds back what the curve takes from a
    /// sound at a mix level, on top of Makeup.
    pub const AUTO_MAKEUP: u16 = 11;
    /// Style: how the gain moves and colours ([`super::STYLES`]).
    pub const STYLE: u16 = 12;
    pub const CLEAN: f32 = 0.0;
    pub const OPTO: f32 = 1.0;
    pub const BUS: f32 = 2.0;
    pub const FET: f32 = 3.0;
    /// Range: the most reduction, dB; at its top, 60, no limit.
    pub const RANGE: u16 = 13;
    /// Stereo link, %: at 100 both sides get the louder side's gain; at 0
    /// each its own.
    pub const LINK: u16 = 14;
}

/// The SC filter's lowest setting, where it is off (not run at all).
const SC_FILTER_OFF: f32 = 20.0;
/// The RMS detector's window.
const RMS_MS: f32 = 10.0;
/// Where auto makeup reads the curve: a sound at a mix level (pink noise
/// at -18 dBFS RMS), as the peak and the RMS detectors read it.
const AUTO_REF_PEAK: f32 = -10.0;
const AUTO_REF_RMS: f32 = -14.5;
/// FET colour at full amount (20 dB of reduction): the second and third
/// harmonics of a steady tone, against it (-40 dB each); how fast the
/// level it is read against falls; the second's DC blocker's corner.
const COLOUR_H2: f32 = 0.01;
const COLOUR_H3: f32 = 0.01;
const COLOUR_FALL_MS: f32 = 50.0;
const COLOUR_DC_HZ: f32 = 10.0;

const STYLE_NAMES: &[&str] = &["Clean", "Opto", "Bus", "FET"];

/// What a style changes.
pub struct Style {
    /// The slow stage, a program-dependent release: the share of the
    /// reduction it holds back (none at 0), how long reduction must last to
    /// charge it, and its release as a multiple of Release, within limits,
    /// seconds.
    pub share: f32,
    pub charge_s: f32,
    pub slow_times: f32,
    pub slow_s: (f32, f32),
    /// The knee is at least this wide, dB.
    pub knee_min: f32,
    /// The attack is two poles, an eased-in onset, rather than one; its
    /// 63 % time stays Attack.
    pub two_pole: bool,
    /// Harmonics added as it turns down: their amount at 20 dB of
    /// reduction and more, scaled down with less.
    pub colour: f32,
}

/// The styles, in [`ids::STYLE`]'s order.
pub static STYLES: [Style; 4] = [
    // Clean: the curve and times as set, nothing added.
    Style {
        share: 0.0,
        charge_s: 1.0,
        slow_times: 1.0,
        slow_s: (0.1, 0.1),
        knee_min: 0.0,
        two_pole: false,
        colour: 0.0,
    },
    // Opto, an LA-2A's light cell: it lights up gently (an eased attack),
    // its ratio grows with level (a knee of at least 12 dB), and it
    // remembers: after long reduction the last half comes back over 0.5
    // to 5 s.
    Style {
        share: 0.5,
        charge_s: 1.0,
        slow_times: 10.0,
        slow_s: (0.5, 5.0),
        knee_min: 12.0,
        two_pole: true,
        colour: 0.0,
    },
    // Bus, an SSL bus compressor's Auto release: quick back after a hit,
    // slower under a steady mix (half over 0.3 to 2 s), so it holds a mix
    // together without pumping.
    Style {
        share: 0.5,
        charge_s: 0.5,
        slow_times: 3.0,
        slow_s: (0.3, 2.0),
        knee_min: 0.0,
        two_pole: false,
        colour: 0.0,
    },
    // FET, an 1176: second and third harmonics that grow as it turns down,
    // and a release that slows a little after long reduction.
    Style {
        share: 0.35,
        charge_s: 0.3,
        slow_times: 4.0,
        slow_s: (0.2, 2.0),
        knee_min: 0.0,
        two_pole: false,
        colour: 1.0,
    },
];

/// The 63 % time of two equal one-poles over one's time constant.
const TWO_POLE_63: f32 = 2.146;
/// Range at its top: no limit.
const RANGE_OFF: f32 = 60.0;

const DETECT: &[&str] = &["Peak", "RMS"];

static INFO: NodeInfo = NodeInfo {
    id: "mpfree.fx.compressor",
    name: "Compressor",
    kind: Kind::Effect,
};
static PARAMS: [ParamInfo; 15] = [
    ParamInfo {
        id: 0,
        name: "Threshold",
        group: "Compressor",
        min: -60.0,
        max: 0.0,
        default: -18.0,
        unit: Unit::Db,
        curve: Curve::Linear,
        readonly: false,
    },
    ParamInfo {
        id: 1,
        name: "Ratio",
        group: "Compressor",
        min: 1.0,
        max: 20.0,
        default: 4.0,
        unit: Unit::None,
        curve: Curve::Log,
        readonly: false,
    },
    ParamInfo {
        id: 2,
        name: "Attack",
        group: "Compressor",
        min: 0.1,
        max: 100.0,
        default: 10.0,
        unit: Unit::Ms,
        curve: Curve::Log,
        readonly: false,
    },
    ParamInfo {
        id: 3,
        name: "Release",
        group: "Compressor",
        min: 10.0,
        max: 2000.0,
        default: 150.0,
        unit: Unit::Ms,
        curve: Curve::Log,
        readonly: false,
    },
    ParamInfo {
        id: 4,
        name: "Knee",
        group: "Compressor",
        min: 0.0,
        max: 24.0,
        default: 6.0,
        unit: Unit::Db,
        curve: Curve::Linear,
        readonly: false,
    },
    ParamInfo {
        id: 5,
        name: "Makeup",
        group: "Output",
        min: 0.0,
        max: 24.0,
        default: 0.0,
        unit: Unit::Db,
        curve: Curve::Linear,
        readonly: false,
    },
    ParamInfo {
        id: 6,
        name: "Mix",
        group: "Output",
        min: 0.0,
        max: 100.0,
        default: 100.0,
        unit: Unit::Percent,
        curve: Curve::Linear,
        readonly: false,
    },
    ParamInfo {
        id: 7,
        name: "Sidechain",
        group: "Output",
        min: 0.0,
        max: 1.0,
        default: 0.0,
        unit: Unit::None,
        curve: Curve::Toggle,
        readonly: false,
    },
    ParamInfo {
        id: 8,
        name: "Reduction",
        group: "Meters",
        min: -60.0,
        max: 0.0,
        default: 0.0,
        unit: Unit::Db,
        curve: Curve::Linear,
        readonly: true,
    },
    ParamInfo {
        id: 9,
        name: "SC filter",
        group: "Detector",
        min: SC_FILTER_OFF,
        max: 500.0,
        default: SC_FILTER_OFF,
        unit: Unit::Hz,
        curve: Curve::Log,
        readonly: false,
    },
    ParamInfo {
        id: 10,
        name: "Detect",
        group: "Detector",
        min: 0.0,
        max: 1.0,
        default: 0.0,
        unit: Unit::None,
        curve: Curve::Choice(DETECT),
        readonly: false,
    },
    ParamInfo {
        id: 11,
        name: "Auto makeup",
        group: "Output",
        min: 0.0,
        max: 1.0,
        default: 0.0,
        unit: Unit::None,
        curve: Curve::Toggle,
        readonly: false,
    },
    ParamInfo {
        id: 12,
        name: "Style",
        group: "Compressor",
        min: 0.0,
        max: 3.0,
        default: 0.0,
        unit: Unit::None,
        curve: Curve::Choice(STYLE_NAMES),
        readonly: false,
    },
    ParamInfo {
        id: 13,
        name: "Range",
        group: "Compressor",
        min: 1.0,
        max: RANGE_OFF,
        default: RANGE_OFF,
        unit: Unit::Db,
        curve: Curve::Linear,
        readonly: false,
    },
    ParamInfo {
        id: 14,
        name: "Link",
        group: "Detector",
        min: 0.0,
        max: 100.0,
        default: 100.0,
        unit: Unit::Percent,
        curve: Curve::Linear,
        readonly: false,
    },
];

/// The reduction the static curve asks for at input level `x`, positive
/// dB: exactly 0 below the knee and at Ratio 1, so the compressor passes
/// the input untouched there.
#[inline]
fn reduction(x: f32, t: f32, ratio: f32, knee: f32) -> f32 {
    let over = x - t;
    let slope = 1.0 - 1.0 / ratio;
    if 2.0 * over < -knee {
        0.0
    } else if knee > 0.0 && 2.0 * over.abs() <= knee {
        let a = over + knee * 0.5;
        slope * a * a / (2.0 * knee)
    } else {
        slope * over
    }
}

/// The static curve: output level for input level `x`, all in dB.
#[cfg(test)]
fn curve(x: f32, t: f32, ratio: f32, knee: f32) -> f32 {
    x - reduction(x, t, ratio, knee)
}

/// The share of a step that a one-pole of time constant `a` followed by
/// the attack (one pole of `b`, or two if `two`) covers by time `t`.
fn covered(a: f32, b: f32, two: bool, t: f32) -> f32 {
    // Distinct poles only: nudge a coincidence apart (far under what
    // anyone hears).
    let a = if (a - b).abs() < 1e-3 * (a + b) {
        a * 1.002
    } else {
        a
    };
    let (p, q) = (1.0 / a, 1.0 / b);
    if two {
        let d = (q - p) * (q - p);
        1.0 - q * q / d * (-p * t).exp()
            - p * (p - 2.0 * q) / d * (-q * t).exp()
            - p * q / (p - q) * t * (-q * t).exp()
    } else {
        1.0 - (a * (-t / a).exp() - b * (-t / b).exp()) / (a - b)
    }
}

/// The release's own time constant, in seconds, for a labelled `release`
/// after an attack of time constant `attack` (two poles of it if `two`):
/// the hold's release runs through the attack smoothing after it, so on
/// its own it is a little quicker, and together they cover 63 % of the
/// way back in the labelled time. Where the attack alone already takes
/// longer than the release, the hold lets go at once and the attack sets
/// the pace.
pub(crate) fn release_tau(attack: f32, two: bool, release: f32) -> f32 {
    let goal = 1.0 - (-1.0f32).exp();
    let (mut lo, mut hi) = (1e-6 * release, release);
    if covered(lo, attack, two, release) < goal {
        return lo;
    }
    for _ in 0..40 {
        let mid = 0.5 * (lo + hi);
        if covered(mid, attack, two, release) > goal {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    0.5 * (lo + hi)
}

/// A soft-knee compressor, stereo-linked by default.
pub struct Compressor {
    v: [f32; 15],
    rate: u32,
    thresh: Smooth,
    makeup: Smooth,
    mix: Smooth,
    /// The detector per side (the first alone when fully linked), in
    /// positive dB of reduction: the peak hold (up at once, down at the
    /// release), the attack's smoothing after it (a second pole of it in
    /// Opto), and the style's slow stage.
    hold: [f32; 2],
    env: [f32; 2],
    env2: [f32; 2],
    slow: [f32; 2],
    /// The reduction applied now, the larger side's.
    now: f32,
    att: f32,
    rel: f32,
    slow_up: f32,
    slow_down: f32,
    /// FET colour: its amount (gliding in and out with the style); per
    /// side the level it reads the sound against and the second
    /// harmonic's DC blocker (last in, last out).
    colour: Smooth,
    peak: [f32; 2],
    peak_fall: f32,
    dc: [(f32, f32); 2],
    dc_r: f32,
    /// The detector's high-pass (both channels of the key).
    hpf: Svf,
    /// The RMS detector's mean square per side, and its one-pole
    /// coefficient.
    ms: [f32; 2],
    ms_k: f32,
}

impl Default for Compressor {
    fn default() -> Self {
        Self::new()
    }
}

/// A level in dB from a peak `p`: itself, or (RMS) through the mean
/// square `ms`, calibrated so a steady sine reads its peak.
#[inline(always)]
fn level<const RMS: bool>(ms: &mut f32, k: f32, p: f32) -> f32 {
    if RMS {
        *ms = crate::flush(*ms + k * (p * p - *ms));
        // 10 log10(2 ms): a sine's RMS level plus 3 dB, its peak.
        // Scaled by 1e4 so the -120 dB floor and the 1e6 ceiling of
        // `fast_level_db` land at -100 dB and +17 dB.
        0.5 * fast_level_db(*ms * 2e4) - 40.0
    } else {
        // dB without log10: on the MPC's ARM the library call was most of
        // the compressor's time.
        fast_level_db(p)
    }
}

impl Compressor {
    /// The reduction the curve asks for at a steady `in_db`, Range
    /// included, positive dB.
    fn reduction_at(&self, in_db: f32) -> f32 {
        let (t, ratio) = (self.v[0], self.v[1]);
        reduction(in_db, t, ratio, self.knee()).min(self.v[ids::RANGE as usize])
    }

    /// The makeup in dB: the knob's, and auto makeup's when it is on.
    pub fn makeup_db(&self) -> f32 {
        let auto = if self.v[ids::AUTO_MAKEUP as usize] >= 0.5 {
            let rms = self.v[ids::DETECT as usize] >= 0.5;
            self.reduction_at(if rms { AUTO_REF_RMS } else { AUTO_REF_PEAK })
        } else {
            0.0
        };
        self.v[ids::MAKEUP as usize] + auto
    }

    /// The static curve with makeup: the output level for a steady input
    /// at `in_db`, both dBFS. For drawing.
    pub fn curve_db(&self, in_db: f32) -> f32 {
        in_db - self.reduction_at(in_db) + self.makeup_db()
    }

    pub fn new() -> Compressor {
        let v = defaults(&PARAMS);
        let mut c = Compressor {
            v,
            rate: 48_000,
            thresh: Smooth::new(v[0]),
            makeup: Smooth::new(v[5]),
            mix: Smooth::new(v[6] / 100.0),
            hold: [0.0; 2],
            env: [0.0; 2],
            env2: [0.0; 2],
            slow: [0.0; 2],
            now: 0.0,
            att: 0.0,
            rel: 0.0,
            slow_up: 0.0,
            slow_down: 0.0,
            colour: Smooth::new(0.0),
            peak: [0.0; 2],
            peak_fall: 0.0,
            dc: [(0.0, 0.0); 2],
            dc_r: 0.0,
            hpf: Svf::default(),
            ms: [0.0; 2],
            ms_k: 0.0,
        };
        c.prepare(48_000, mpfree_node::MAX_BLOCK);
        c
    }

    /// The style chosen.
    pub fn style(&self) -> &'static Style {
        &STYLES[(self.v[ids::STYLE as usize].round() as usize).min(STYLES.len() - 1)]
    }

    /// The knee in use: the knob's, or the style's least.
    pub fn knee(&self) -> f32 {
        self.v[ids::KNEE as usize].max(self.style().knee_min)
    }

    /// The style's slow stage's release, seconds.
    fn slow_release(&self) -> f32 {
        let st = self.style();
        (st.slow_times * self.v[ids::RELEASE as usize] * 0.001).clamp(st.slow_s.0, st.slow_s.1)
    }

    fn times(&mut self) {
        let fs = self.rate as f32;
        let st = self.style();
        let (attack, release) = (self.v[2] * 0.001, self.v[3] * 0.001);
        // Two poles reach 63 % in 2.146 of their time constants.
        let tau = if st.two_pole {
            attack / TWO_POLE_63
        } else {
            attack
        };
        self.att = (-1.0 / (tau * fs)).exp();
        self.rel = (-1.0 / (release_tau(tau, st.two_pole, release) * fs)).exp();
        self.slow_up = (-1.0 / (st.charge_s * fs)).exp();
        self.slow_down = (-1.0 / (self.slow_release() * fs)).exp();
        self.ms_k = 1.0 - (-1.0 / (RMS_MS * 0.001 * fs)).exp();
        self.dc_r = (-std::f32::consts::TAU * COLOUR_DC_HZ / fs).exp();
        self.peak_fall = (-1.0 / (COLOUR_FALL_MS * 0.001 * fs)).exp();
    }

    fn sc_filter(&mut self) {
        let hz = self.v[ids::SC_FILTER as usize];
        self.hpf.set(
            SvfMode::HighPass,
            hz,
            std::f32::consts::FRAC_1_SQRT_2,
            0.0,
            self.rate,
        );
    }

    fn apply(&mut self, id: u16, value: f32) {
        let Some(p) = PARAMS.get(id as usize) else {
            return;
        };
        if p.readonly {
            return;
        }
        let v = p.clamp(value);
        let v = if matches!(p.curve, Curve::Choice(_)) {
            v.round()
        } else {
            v
        };
        let was = self.v[id as usize];
        self.v[id as usize] = v;
        match id {
            ids::THRESHOLD => self.thresh.target = v,
            ids::MIX => self.mix.target = v / 100.0,
            ids::ATTACK | ids::RELEASE => self.times(),
            ids::SC_FILTER => self.sc_filter(),
            ids::STYLE if was != v => {
                // The slow stage starts uncharged and the applied
                // reduction carries on from where it was, so nothing
                // jumps.
                if STYLES[was as usize].two_pole {
                    self.env = self.env2;
                }
                self.env2 = self.env;
                self.slow = [0.0; 2];
                self.colour.target = self.style().colour;
                self.times();
            }
            // Unlinking: the right side starts where the shared one is.
            ids::LINK if was >= 100.0 && v < 100.0 => {
                self.hold[1] = self.hold[0];
                self.env[1] = self.env[0];
                self.env2[1] = self.env2[0];
                self.slow[1] = self.slow[0];
                self.ms[1] = self.ms[0];
            }
            _ => {}
        }
        self.makeup.target = self.makeup_db();
    }

    fn run(&mut self, l: &mut [f32], r: &mut [f32], key: Option<(&[f32], &[f32])>) {
        // One loop per detector setting, so the default (peak, no filter,
        // linked) costs what the compressor did before it had the choices.
        let hp = self.v[ids::SC_FILTER as usize] > SC_FILTER_OFF;
        let rms = self.v[ids::DETECT as usize] >= 0.5;
        let split = self.v[ids::LINK as usize] < 100.0;
        match (hp, rms, split) {
            (false, false, false) => self.run_with::<false, false, false>(l, r, key),
            (true, false, false) => self.run_with::<true, false, false>(l, r, key),
            (false, true, false) => self.run_with::<false, true, false>(l, r, key),
            (true, true, false) => self.run_with::<true, true, false>(l, r, key),
            (false, false, true) => self.run_with::<false, false, true>(l, r, key),
            (true, false, true) => self.run_with::<true, false, true>(l, r, key),
            (false, true, true) => self.run_with::<false, true, true>(l, r, key),
            (true, true, true) => self.run_with::<true, true, true>(l, r, key),
        }
        self.v[ids::REDUCTION as usize] = (-self.now).clamp(-60.0, 0.0);
    }

    /// One side's detector: the reduction it applies for `want`.
    ///
    /// The smooth decoupled peak detector on the reduction (Giannoulis,
    /// Massberg and Reiss): the hold jumps to what the curve asks for and
    /// lets go at the release, toward it; the attack then smooths the
    /// hold (twice, if `two`). With a `share`, the style's slow stage
    /// follows that, charging over a long stretch of reduction, and holds
    /// that share of it back at its slow release.
    #[inline(always)]
    fn detect(&mut self, c: usize, want: f32, two: bool, share: f32) -> f32 {
        self.hold[c] = crate::flush(want.max(self.rel * self.hold[c] + (1.0 - self.rel) * want));
        self.env[c] = crate::flush(self.att * self.env[c] + (1.0 - self.att) * self.hold[c]);
        let g = if two {
            self.env2[c] = crate::flush(self.att * self.env2[c] + (1.0 - self.att) * self.env[c]);
            self.env2[c]
        } else {
            self.env[c]
        };
        if share > 0.0 {
            let k = if g > self.slow[c] {
                self.slow_up
            } else {
                self.slow_down
            };
            self.slow[c] = crate::flush(k * self.slow[c] + (1.0 - k) * g);
            g.max(share * self.slow[c])
        } else {
            g
        }
    }

    /// FET colour on one side's compressed sample `y` at `amount`: the
    /// sound read against its own recent peak `u` (so the harmonics are a
    /// share of it at any level), through the Chebyshev polynomials that
    /// turn a full-scale tone into its pure second (2u² - 1, its DC taken
    /// out) and third (4u³ - 3u) harmonics, and scaled back by the peak.
    #[inline(always)]
    fn colour(&mut self, c: usize, y: f32, amount: f32) -> f32 {
        let p = crate::flush(y.abs().max(self.peak_fall * self.peak[c]));
        self.peak[c] = p;
        let (h2, h3) = if p > 1e-6 {
            let u = y / p;
            (p * (2.0 * u * u - 1.0), p * u * (4.0 * u * u - 3.0))
        } else {
            (0.0, 0.0)
        };
        let (x1, y1) = self.dc[c];
        let out = crate::flush(h2 - x1 + self.dc_r * y1);
        self.dc[c] = (h2, out);
        amount * (COLOUR_H2 * out + COLOUR_H3 * h3)
    }

    /// The compressor with the detector's high-pass on (`HP`), reading
    /// RMS (`RMS`) or peak, and each side detected on its own (`SPLIT`).
    fn run_with<const HP: bool, const RMS: bool, const SPLIT: bool>(
        &mut self,
        l: &mut [f32],
        r: &mut [f32],
        key: Option<(&[f32], &[f32])>,
    ) {
        let ratio = self.v[1];
        let knee = self.knee();
        let range = self.v[ids::RANGE as usize];
        let st = self.style();
        let (two, share) = (st.two_pole, st.share);
        let colours = self.colour.target > 0.0 || self.colour.value > 0.0;
        let link = self.v[ids::LINK as usize] * 0.01;
        let k = self.ms_k;
        for i in 0..l.len() {
            let t = self.thresh.next();
            let makeup = self.makeup.next();
            let mix = self.mix.next();
            let (mut a, mut b) = match key {
                Some((kl, kr)) => (kl[i], kr[i]),
                None => (l[i], r[i]),
            };
            if HP {
                a = self.hpf.run(0, a);
                b = self.hpf.run(1, b);
            }
            let want = |x: f32| reduction(x, t, ratio, knee).min(range);
            let (ga, gb) = if SPLIT {
                let xa = level::<RMS>(&mut self.ms[0], k, a.abs());
                let xb = level::<RMS>(&mut self.ms[1], k, b.abs());
                let ga = self.detect(0, want(xa), two, share);
                let gb = self.detect(1, want(xb), two, share);
                let loud = ga.max(gb);
                self.now = loud;
                (ga + link * (loud - ga), gb + link * (loud - gb))
            } else {
                let x = level::<RMS>(&mut self.ms[0], k, a.abs().max(b.abs()));
                let g = self.detect(0, want(x), two, share);
                self.now = g;
                (g, g)
            };
            let wa = fast_db_to_gain(makeup - ga);
            let wb = if SPLIT {
                fast_db_to_gain(makeup - gb)
            } else {
                wa
            };
            let (x, y) = (l[i], r[i]);
            l[i] = x * (1.0 + mix * (wa - 1.0));
            r[i] = y * (1.0 + mix * (wb - 1.0));
            // Checked each frame once it is on, so where the glide reaches
            // nothing doesn't depend on the block size.
            if colours && (self.colour.target > 0.0 || self.colour.value > 0.0) {
                let c = self.colour.next();
                let ca = mix * c * (ga * 0.05).min(1.0);
                let cb = mix * c * (gb * 0.05).min(1.0);
                l[i] += self.colour(0, x * wa, ca);
                r[i] += self.colour(1, y * wb, cb);
                if c == 0.0 {
                    // Off: the next time it colours starts from nothing.
                    self.peak = [0.0; 2];
                    self.dc = [(0.0, 0.0); 2];
                }
            }
        }
    }
}

impl Node for Compressor {
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
        self.rate = rate.max(1);
        for s in [
            &mut self.thresh,
            &mut self.makeup,
            &mut self.mix,
            &mut self.colour,
        ] {
            s.set_time(self.rate, 20.0);
        }
        self.times();
        self.sc_filter();
    }
    fn reset(&mut self) {
        for s in [&mut self.thresh, &mut self.makeup, &mut self.mix] {
            s.snap(s.target);
        }
        self.hold = [0.0; 2];
        self.env = [0.0; 2];
        self.env2 = [0.0; 2];
        self.slow = [0.0; 2];
        self.now = 0.0;
        self.colour.snap(self.colour.target);
        self.peak = [0.0; 2];
        self.dc = [(0.0, 0.0); 2];
        self.ms = [0.0; 2];
        self.hpf.reset();
        self.v[ids::REDUCTION as usize] = 0.0;
    }
    fn process(&mut self, ctx: &Ctx, events: &[Timed], left: &mut [f32], right: &mut [f32]) {
        let len = left.len().min(right.len());
        let side = if self.v[7] >= 0.5 {
            ctx.sidechain
                .filter(|(a, b)| a.len() >= len && b.len() >= len)
        } else {
            None
        };
        let mut segs = Segs::new(events);
        while let Some((from, to)) = segs.next(len, |ev| {
            if let NodeEvent::Param { id, value } = *ev {
                self.apply(id, value)
            }
        }) {
            let key = side.map(|(a, b)| (&a[from..to], &b[from..to]));
            self.run(&mut left[from..to], &mut right[from..to], key);
        }
    }
    fn tail_frames(&self) -> u32 {
        0
    }
    fn rest_frames(&self) -> u32 {
        // Keyed, the envelope follows the key through silence.
        if self.v[ids::SIDECHAIN as usize] >= 0.5 {
            return u32::MAX;
        }
        // The release, ten time constants (80 dB of reduction to under
        // 0.01 dB), and the RMS detector falling below any threshold;
        // with a slow stage, its ten.
        let mut ms = 10.0 * self.v[ids::RELEASE as usize] + 25.0 * RMS_MS;
        if self.style().share > 0.0 {
            ms = ms.max(10_000.0 * self.slow_release());
        }
        (ms * 0.001 * self.rate as f32) as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_curve() {
        assert_eq!(curve(-30.0, -20.0, 4.0, 0.0), -30.0);
        assert!((curve(-4.0, -20.0, 4.0, 0.0) - -16.0).abs() < 1e-5);
        // Continuous through the knee.
        let a = curve(-23.0 - 1e-3, -20.0, 4.0, 6.0);
        let b = curve(-23.0 + 1e-3, -20.0, 4.0, 6.0);
        assert!((a - b).abs() < 0.01);
        let a = curve(-17.0 - 1e-3, -20.0, 4.0, 6.0);
        let b = curve(-17.0 + 1e-3, -20.0, 4.0, 6.0);
        assert!((a - b).abs() < 0.01);
    }

    /// The reduction (positive dB) a compressor at -30 dB threshold, 4:1,
    /// no knee, settles to on `signal`, with `params` set.
    fn settled(params: &[(u16, f32)], signal: impl Fn(usize) -> f32) -> f32 {
        let mut c = Compressor::new();
        c.prepare(48_000, 512);
        for &(id, v) in [(ids::THRESHOLD, -30.0), (ids::KNEE, 0.0)]
            .iter()
            .chain(params)
        {
            c.set_param(id, v);
        }
        c.reset();
        let mut worst = 0.0f32;
        for b in 0..200 {
            let mut l: Vec<f32> = (0..240).map(|i| signal(b * 240 + i)).collect();
            let mut r = l.clone();
            c.process(&Ctx::new(48_000, (b * 240) as u64), &[], &mut l, &mut r);
            if b >= 150 {
                worst = worst.max(-c.param(ids::REDUCTION));
            }
        }
        worst
    }

    fn sine(hz: f32, amp: f32) -> impl Fn(usize) -> f32 {
        move |n| amp * (std::f32::consts::TAU * hz * n as f32 / 48_000.0).sin()
    }

    #[test]
    fn rms_and_peak_agree_on_a_steady_sine() {
        // A -12 dBFS sine is 18 dB over: 13.5 dB of reduction at 4:1 on
        // its crest (the 10 ms attack averages a little of |x|'s ripple
        // away in Peak).
        for hz in [100.0, 1000.0, 5000.0] {
            let peak = settled(&[], sine(hz, 0.25));
            let rms = settled(&[(ids::DETECT, 1.0)], sine(hz, 0.25));
            assert!(peak > 12.0 && peak < 14.0, "{hz} Hz: peak {peak}");
            assert!((rms - 13.5).abs() < 0.5, "{hz} Hz: rms {rms}");
        }
    }

    #[test]
    fn rms_lets_short_hits_through() {
        // A 1 ms full-scale click every 50 ms, read with the fastest
        // attack: Peak sees the click, RMS its energy over 10 ms.
        let clicks = |n: usize| if n % 2400 < 48 { 0.9 } else { 0.0 };
        let fast = [(ids::ATTACK, 0.1), (ids::RELEASE, 2000.0)];
        let peak = settled(&fast, clicks);
        let rms = settled(&[fast[0], fast[1], (ids::DETECT, 1.0)], clicks);
        assert!(peak > rms + 4.0, "peak {peak} rms {rms}");
    }

    /// A compressor at 48 kHz with `params` set, settled.
    fn with(params: &[(u16, f32)]) -> Compressor {
        let mut c = Compressor::new();
        c.prepare(48_000, 512);
        for &(id, v) in params {
            c.set_param(id, v);
        }
        c.reset();
        c
    }

    /// `left` and `right` through `c` in blocks of 64.
    fn render(c: &mut Compressor, left: &[f32], right: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let (mut l, mut r) = (left.to_vec(), right.to_vec());
        for (k, (a, b)) in l.chunks_mut(64).zip(r.chunks_mut(64)).enumerate() {
            c.process(&Ctx::new(48_000, (k * 64) as u64), &[], a, b);
        }
        (l, r)
    }

    fn tone(hz: f32, db: f32, n: usize) -> Vec<f32> {
        let a = 10f32.powf(db / 20.0);
        (0..n).map(sine(hz, a)).collect()
    }

    fn rms_db(x: &[f32]) -> f32 {
        let ms = x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32;
        10.0 * ms.log10()
    }

    #[test]
    fn a_steady_tone_gets_the_reduction_drawn() {
        // K1: peak and RMS, three knees, levels through the knee and over.
        for knee in [0.0, 6.0, 12.0] {
            for detect in [0.0, 1.0] {
                let mut c = with(&[(ids::KNEE, knee), (ids::DETECT, detect)]);
                for db in [-24.0, -18.0, -12.0, -6.0, 0.0] {
                    c.reset();
                    let x = tone(1000.0, db, 24_000);
                    let (y, _) = render(&mut c, &x, &x);
                    let got = rms_db(&y[19_200..]) - rms_db(&x[19_200..]);
                    let drawn = c.curve_db(db) - db;
                    assert!(
                        (got - drawn).abs() < 0.25,
                        "knee {knee} detect {detect} at {db} dBFS: {got} against {drawn}"
                    );
                }
            }
        }
    }

    /// Milliseconds after frame `from` until the gain in `x` to `y` (read
    /// wherever the tone isn't near a zero crossing) covers `share` of the
    /// way to `to` dB.
    fn time_to(x: &[f32], y: &[f32], from: usize, start: f32, to: f32, share: f32) -> f32 {
        let goal = start + share * (to - start);
        let amp = x[from..from + 480]
            .iter()
            .fold(0.0f32, |m, v| m.max(v.abs()));
        let i = (from..x.len())
            .filter(|&i| x[i].abs() > 0.2 * amp)
            .find(|&i| {
                let g = 20.0 * (y[i] / x[i]).abs().log10();
                if to < start {
                    g <= goal
                } else {
                    g >= goal
                }
            })
            .expect("it gets there");
        (i - from) as f32 / 48.0
    }

    #[test]
    fn attack_and_release_take_their_labels() {
        // K2 and K3: a -40 to 0 dBFS step and back; 63 % of the gain change
        // in the labelled time.
        for (att, rel) in [(1.0, 50.0), (10.0, 150.0), (30.0, 500.0)] {
            let mut c = with(&[(ids::ATTACK, att), (ids::RELEASE, rel)]);
            let (quiet, loud) = (4_800, (att * 48.0 * 10.0) as usize + 4_800);
            let back = (rel * 48.0 * 6.0) as usize;
            let mut x = tone(1000.0, -40.0, quiet + loud + back);
            for v in &mut x[quiet..quiet + loud] {
                *v *= 100.0;
            }
            let (y, _) = render(&mut c, &x, &x);
            let full = c.curve_db(0.0);
            let a = time_to(&x, &y, quiet, 0.0, full, 0.63);
            let r = time_to(&x, &y, quiet + loud, full, 0.0, 0.63);
            assert!((a / att - 1.0).abs() < 0.15, "attack {att} ms took {a}");
            assert!((r / rel - 1.0).abs() < 0.10, "release {rel} ms took {r}");
        }
    }

    #[test]
    fn idle_it_passes_the_input_bit_for_bit() {
        // K8: Ratio 1 on a loud tone, and a tone under the knee.
        let loud = tone(997.0, -3.0, 9_600);
        let mut c = with(&[(ids::RATIO, 1.0)]);
        assert_eq!(render(&mut c, &loud, &loud).0, loud);
        let quiet = tone(997.0, -30.0, 9_600);
        let mut c = with(&[]);
        assert_eq!(render(&mut c, &quiet, &quiet).0, quiet);
    }

    #[test]
    fn range_caps_the_reduction() {
        let x = tone(1000.0, 0.0, 24_000);
        let mut c = with(&[(ids::RANGE, 6.0)]);
        render(&mut c, &x, &x);
        assert!((c.param(ids::REDUCTION) + 6.0).abs() < 0.05);
        let mut c = with(&[]);
        render(&mut c, &x, &x);
        assert!((c.param(ids::REDUCTION) + 13.5).abs() < 0.1);
    }

    #[test]
    fn the_styles_come_back_slower_after_long_reduction() {
        // The time to come 90 % of the way back after `loud` frames at
        // 0 dBFS, from the meter every 64 frames.
        let back90 = |style: f32, loud: usize| -> f32 {
            let mut c = with(&[(ids::STYLE, style)]);
            let mut x = tone(1000.0, -40.0, loud + 480_000);
            for v in &mut x[..loud] {
                *v *= 100.0;
            }
            let mut start = None;
            for (k, block) in x.chunks(64).enumerate() {
                let (mut l, mut r) = (block.to_vec(), block.to_vec());
                c.process(&Ctx::new(48_000, (k * 64) as u64), &[], &mut l, &mut r);
                let gr = -c.param(ids::REDUCTION);
                if k * 64 + 64 == loud {
                    start = Some(gr);
                } else if let Some(s) = start {
                    if gr <= 0.1 * s {
                        return (k * 64 + 64 - loud) as f32 / 48.0;
                    }
                }
            }
            f32::INFINITY
        };
        // Whole blocks: 50 ms and 3 s.
        let short = 2_432;
        let long = 144_000;
        // Clean: the same whatever came before (90 % in 2.3 releases).
        let (a, b) = (back90(ids::CLEAN, short), back90(ids::CLEAN, long));
        assert!((a - b).abs() < 0.05 * a, "clean {a} and {b} ms");
        assert!((a / 150.0 - 2.3).abs() < 0.2, "{a}");
        // The others: about as quick after a hit, much slower after 3 s.
        for style in [ids::OPTO, ids::BUS, ids::FET] {
            let (a, b) = (back90(style, short), back90(style, long));
            assert!(
                a < 1.3 * back90(ids::CLEAN, short),
                "{style} after a hit {a} ms"
            );
            assert!(b > 2.0 * a, "{style} after 3 s {b} ms, after a hit {a}");
        }
    }

    #[test]
    fn opto_eases_in_and_still_takes_its_attack() {
        let mut x = tone(1000.0, -40.0, 4_800 + 9_600);
        for v in &mut x[4_800..] {
            *v *= 100.0;
        }
        let full = with(&[(ids::STYLE, ids::OPTO)]).curve_db(0.0);
        let at = |style: f32, ms: f32| {
            let mut c = with(&[(ids::STYLE, style)]);
            let (y, _) = render(&mut c, &x, &x);
            time_to(&x, &y, 4_800, 0.0, full, ms)
        };
        // 63 % at the label either way; 10 % later in Opto (an S, not a
        // jump).
        assert!((at(ids::OPTO, 0.63) / 10.0 - 1.0).abs() < 0.15);
        assert!(at(ids::OPTO, 0.1) > 2.0 * at(ids::CLEAN, 0.1));
    }

    #[test]
    fn fet_colours_only_as_it_turns_down() {
        // A 100 Hz tone under the knee comes through untouched; 18 dB over
        // it gains harmonics.
        let quiet = tone(100.0, -30.0, 9_600);
        let mut c = with(&[(ids::STYLE, ids::FET)]);
        assert_eq!(render(&mut c, &quiet, &quiet).0, quiet);
        let loud = tone(100.0, 0.0, 48_000);
        let harmonic = |style: f32, k: f32| {
            let mut c = with(&[(ids::STYLE, style)]);
            let (y, _) = render(&mut c, &loud, &loud);
            let y = &y[24_000..];
            let w = std::f32::consts::TAU * 100.0 * k / 48_000.0;
            let (re, im) = y.iter().enumerate().fold((0.0, 0.0), |(re, im), (n, v)| {
                (re + v * (w * n as f32).cos(), im + v * (w * n as f32).sin())
            });
            2.0 * (re * re + im * im).sqrt() / y.len() as f32
        };
        let f = harmonic(ids::FET, 1.0);
        for k in [2.0, 3.0] {
            let fet = 20.0 * (harmonic(ids::FET, k) / f).log10();
            let clean = 20.0 * (harmonic(ids::CLEAN, k) / f).log10();
            assert!(fet > -60.0 && fet < -30.0, "H{k}: {fet} dB");
            assert!(clean < fet - 15.0, "H{k}: clean {clean} dB");
        }
    }

    #[test]
    fn link_sets_how_much_the_quiet_side_follows() {
        let loud = tone(1000.0, 0.0, 24_000);
        let quiet = tone(1000.0, -30.0, 24_000);
        let gain = |link: f32| {
            let mut c = with(&[(ids::LINK, link)]);
            let (l, r) = render(&mut c, &loud, &quiet);
            let g = |y: &[f32], x: &[f32]| rms_db(&y[19_200..]) - rms_db(&x[19_200..]);
            (g(&l, &loud), g(&r, &quiet))
        };
        let (l, r) = gain(100.0);
        assert!((l - r).abs() < 0.1, "linked {l} {r}");
        let (l, r) = gain(0.0);
        assert!(l < -13.0 && r.abs() < 0.01, "unlinked {l} {r}");
        let (l, r) = gain(50.0);
        assert!((r - 0.5 * l).abs() < 0.1, "half {l} {r}");
    }

    #[test]
    fn auto_makeup_adds_back_the_curve_at_a_mix_level() {
        // Peak reads a mix-level sound near -10 dBFS: 8 dB over at 4:1.
        let c = with(&[(ids::AUTO_MAKEUP, 1.0)]);
        assert!((c.makeup_db() - 6.0).abs() < 1e-4);
        // RMS reads it near -14.5: 3.5 dB over, in the 6 dB knee's top.
        let c = with(&[(ids::AUTO_MAKEUP, 1.0), (ids::DETECT, 1.0)]);
        assert!((c.makeup_db() - reduction(-14.5, -18.0, 4.0, 6.0)).abs() < 1e-4);
        // On top of Makeup; off, Makeup alone.
        let c = with(&[(ids::AUTO_MAKEUP, 1.0), (ids::MAKEUP, 3.0)]);
        assert!((c.makeup_db() - 9.0).abs() < 1e-4);
        assert_eq!(with(&[(ids::MAKEUP, 3.0)]).makeup_db(), 3.0);
    }

    #[test]
    fn the_sc_filter_keeps_lows_from_pushing_the_detector() {
        let rms = (ids::DETECT, 1.0);
        let low = |f: f32| settled(&[(ids::SC_FILTER, f), rms], sine(50.0, 0.25));
        let high = |f: f32| settled(&[(ids::SC_FILTER, f), rms], sine(2000.0, 0.25));
        // Off: the 50 Hz tone compresses like any other.
        assert!(
            (low(SC_FILTER_OFF) - 13.5).abs() < 0.8,
            "{}",
            low(SC_FILTER_OFF)
        );
        // At 300 Hz the detector hears 50 Hz about 31 dB down: well under.
        assert!(low(300.0) < 0.5, "{}", low(300.0));
        // A 2 kHz tone is untouched by the filter.
        assert!((high(300.0) - high(SC_FILTER_OFF)).abs() < 0.3);
    }
}
