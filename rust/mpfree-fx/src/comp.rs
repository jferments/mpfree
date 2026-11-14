//! Compressor: a feed-forward, soft-knee compressor with the gain computed
//! and smoothed in decibels (Giannoulis, Massberg and Reiss, "Digital
//! Dynamic Range Compressor Design", JAES 2012: the smooth, decoupled peak
//! detector), stereo-linked, with parallel mix and a sidechain input for
//! ducking.
//!
//! The detector hears the strip (or the sidechain key) through an optional
//! high-pass, so a kick's low end doesn't pump the whole mix, and reads
//! either the peak or the RMS level. RMS is calibrated so a steady sine
//! reads the same in both: switching changes how hard short hits push it,
//! not where the threshold sits.

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
}

/// The SC filter's lowest setting, where it is off (not run at all).
const SC_FILTER_OFF: f32 = 20.0;
/// The RMS detector's window.
const RMS_MS: f32 = 10.0;

const DETECT: &[&str] = &["Peak", "RMS"];

static INFO: NodeInfo = NodeInfo {
    id: "mpfree.fx.compressor",
    name: "Compressor",
    kind: Kind::Effect,
};
static PARAMS: [ParamInfo; 11] = [
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
];

/// The static curve: output level for input level `x`, all in dB.
#[inline]
fn curve(x: f32, t: f32, ratio: f32, knee: f32) -> f32 {
    let over = x - t;
    if 2.0 * over < -knee {
        x
    } else if knee > 0.0 && 2.0 * over.abs() <= knee {
        let a = over + knee * 0.5;
        x + (1.0 / ratio - 1.0) * a * a / (2.0 * knee)
    } else {
        t + over / ratio
    }
}

/// A stereo-linked, soft-knee compressor.
pub struct Compressor {
    v: [f32; 11],
    rate: u32,
    thresh: Smooth,
    makeup: Smooth,
    mix: Smooth,
    /// Smoothed gain reduction, positive dB.
    env: f32,
    att: f32,
    rel: f32,
    /// The detector's high-pass (both channels of the key).
    hpf: Svf,
    /// The RMS detector's mean square, and its one-pole coefficient.
    ms: f32,
    ms_k: f32,
}

impl Default for Compressor {
    fn default() -> Self {
        Self::new()
    }
}

impl Compressor {
    /// The static curve with makeup: the output level for a steady input
    /// at `in_db`, both dBFS. For drawing.
    pub fn curve_db(&self, in_db: f32) -> f32 {
        let (t, ratio, knee) = (self.v[0], self.v[1], self.v[4]);
        curve(in_db, t, ratio, knee) + self.v[5]
    }

    pub fn new() -> Compressor {
        let v = defaults(&PARAMS);
        let mut c = Compressor {
            v,
            rate: 48_000,
            thresh: Smooth::new(v[0]),
            makeup: Smooth::new(v[5]),
            mix: Smooth::new(v[6] / 100.0),
            env: 0.0,
            att: 0.0,
            rel: 0.0,
            hpf: Svf::default(),
            ms: 0.0,
            ms_k: 0.0,
        };
        c.prepare(48_000, mpfree_node::MAX_BLOCK);
        c
    }

    fn times(&mut self) {
        let fs = self.rate as f32;
        self.att = (-1.0 / (self.v[2] * 0.001 * fs)).exp();
        self.rel = (-1.0 / (self.v[3] * 0.001 * fs)).exp();
        self.ms_k = 1.0 - (-1.0 / (RMS_MS * 0.001 * fs)).exp();
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
        self.v[id as usize] = v;
        match id {
            ids::THRESHOLD => self.thresh.target = v,
            ids::MAKEUP => self.makeup.target = v,
            ids::MIX => self.mix.target = v / 100.0,
            ids::ATTACK | ids::RELEASE => self.times(),
            ids::SC_FILTER => self.sc_filter(),
            _ => {}
        }
    }

    fn run(&mut self, l: &mut [f32], r: &mut [f32], key: Option<(&[f32], &[f32])>) {
        // One loop per detector setting, so the default (peak, no filter)
        // costs what the compressor did before it had the choice.
        let hp = self.v[ids::SC_FILTER as usize] > SC_FILTER_OFF;
        let rms = self.v[ids::DETECT as usize] >= 0.5;
        match (hp, rms) {
            (false, false) => self.run_with::<false, false>(l, r, key),
            (true, false) => self.run_with::<true, false>(l, r, key),
            (false, true) => self.run_with::<false, true>(l, r, key),
            (true, true) => self.run_with::<true, true>(l, r, key),
        }
        self.v[ids::REDUCTION as usize] = (-self.env).clamp(-60.0, 0.0);
    }

    /// The compressor with the detector's high-pass on (`HP`) and reading
    /// RMS (`RMS`) or peak.
    fn run_with<const HP: bool, const RMS: bool>(
        &mut self,
        l: &mut [f32],
        r: &mut [f32],
        key: Option<(&[f32], &[f32])>,
    ) {
        let ratio = self.v[1];
        let knee = self.v[4];
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
            // dB in and out without log10 and powf: on the MPC's ARM those
            // library calls were most of the compressor's time.
            let x = if RMS {
                self.ms = crate::flush(self.ms + self.ms_k * ((a * a).max(b * b) - self.ms));
                // 10 log10(2 ms): a sine's RMS level plus 3 dB, its peak.
                // Scaled by 1e4 so the -120 dB floor and the 1e6 ceiling of
                // `fast_level_db` land at -100 dB and +17 dB.
                0.5 * fast_level_db(self.ms * 2e4) - 40.0
            } else {
                fast_level_db(a.abs().max(b.abs()))
            };
            let want = x - curve(x, t, ratio, knee); // reduction wanted, >= 0
            let k = if want > self.env { self.att } else { self.rel };
            self.env = crate::flush(k * self.env + (1.0 - k) * want);
            let g = fast_db_to_gain(makeup - self.env);
            let wet = 1.0 + mix * (g - 1.0);
            l[i] *= wet;
            r[i] *= wet;
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
        for s in [&mut self.thresh, &mut self.makeup, &mut self.mix] {
            s.set_time(self.rate, 20.0);
        }
        self.times();
        self.sc_filter();
    }
    fn reset(&mut self) {
        for s in [&mut self.thresh, &mut self.makeup, &mut self.mix] {
            s.snap(s.target);
        }
        self.env = 0.0;
        self.ms = 0.0;
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
        // 0.01 dB), and the RMS detector falling below any threshold.
        let ms = 10.0 * self.v[ids::RELEASE as usize] + 25.0 * RMS_MS;
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
