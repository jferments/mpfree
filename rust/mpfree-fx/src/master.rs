//! Master safety: the last stage of every output, so nothing reaches the
//! converter clipped unless the user asks for it.
//!
//! In order: trim, DC block, then one of three modes.
//!
//! - **Safe limiter** (default): a 1.5 ms look-ahead true-peak limiter
//!   (the [`crate::limit`] core the Limiter uses, with true peaks always
//!   caught: decision 11.1 of FX quality round 4). The gain for each frame
//!   is the smallest any frame in the look-ahead window and the hold after
//!   it needs, as a 4x true-peak meter reads the peaks, released in
//!   decibels and then averaged over the window, so it is always low
//!   enough by the time the peak arrives and never moves faster than the
//!   window. Neither the samples nor the peaks between them go above the
//!   ceiling.
//! - **Soft clip**: a smooth saturator from 6 dB under the ceiling up to
//!   the ceiling (deliberate colour), run at twice the rate with
//!   antiderivative antialiasing so it doesn't fold back, then the same
//!   limiter, which catches the little the band-limited curve still
//!   overshoots.
//! - **Raw**: no limiting; what is over 0 dBFS clips in the converter. A
//!   deliberate choice for people who want that sound.
//!
//! Every mode counts overs (frames that came in above the ceiling) and
//! frames that leave above 0 dBFS, as meters. Nothing fails, stops or
//! mutes because of level. The delay is the same in every mode (the
//! limiter's look-ahead and the soft clip's, which the other modes match
//! with a short pad), so switching modes never moves the sound in time;
//! a switch crossfades over 10 ms.

use mpfree_node::{Ctx, Curve, Kind, Node, NodeEvent, NodeInfo, ParamInfo, Smooth, Timed, Unit};

use crate::adaa::{log_cosh, Adaa, Flat2x};
use crate::limit::PeakLimit;
use crate::oversample::IirPair;
use crate::svf::Segs;
use crate::{db_to_gain, defaults, gain_to_db, DcBlock, FixedDelay};

/// [`MasterSafety`] parameter ids.
pub mod ids {
    pub const MODE: u16 = 0;
    pub const TRIM: u16 = 1;
    pub const CEILING: u16 = 2;
    pub const RELEASE: u16 = 3;
    /// Meter: limiting now, dB (0 or below).
    pub const REDUCTION: u16 = 4;
    /// Meter: frames that came in above the ceiling.
    pub const OVERS: u16 = 5;
    /// Meter: frames that left above 0 dBFS (only Raw mode can make these).
    pub const CLIPPED: u16 = 6;
}

const MODES: &[&str] = &["Safe limiter", "Soft clip", "Raw"];
const LOOKAHEAD_MS: f32 = 1.5;
/// A mode switch's crossfade.
const FADE_MS: f32 = 10.0;
/// The soft clip's half-bands: Drive's Eco design (five all-pass
/// sections, flat to 20 kHz and 72 dB down from 25 kHz at 2x of 44.1 kHz).
const CLIP_SECTIONS: usize = 5;
const CLIP_TBW: f64 = 0.06;
/// How often the half-bands flush their states, frames.
const SETTLE: u64 = 32;
/// The soft clip's delay in base frames: the half-band pair's at low
/// frequencies (2.02) and ADAA with its [`Flat2x`] (2). The other modes
/// are padded by the same.
const CLIP_DELAY: usize = 2 + Flat2x::STEPS / 2;

static INFO: NodeInfo = NodeInfo {
    id: "mpfree.fx.master_safety",
    name: "Master safety",
    kind: Kind::Effect,
};
static PARAMS: [ParamInfo; 7] = [
    ParamInfo {
        id: 0,
        name: "Mode",
        group: "Master safety",
        min: 0.0,
        max: 2.0,
        default: 0.0,
        unit: Unit::None,
        curve: Curve::Choice(MODES),
        readonly: false,
    },
    ParamInfo {
        id: 1,
        name: "Trim",
        group: "Master safety",
        min: -24.0,
        max: 12.0,
        default: 0.0,
        unit: Unit::Db,
        curve: Curve::Linear,
        readonly: false,
    },
    ParamInfo {
        id: 2,
        name: "Ceiling",
        group: "Master safety",
        min: -6.0,
        max: 0.0,
        default: -0.3,
        unit: Unit::Db,
        curve: Curve::Linear,
        readonly: false,
    },
    ParamInfo {
        id: 3,
        name: "Release",
        group: "Master safety",
        min: 10.0,
        max: 1000.0,
        default: 80.0,
        unit: Unit::Ms,
        curve: Curve::Log,
        readonly: false,
    },
    ParamInfo {
        id: 4,
        name: "Limiting",
        group: "Meters",
        min: -60.0,
        max: 0.0,
        default: 0.0,
        unit: Unit::Db,
        curve: Curve::Linear,
        readonly: true,
    },
    ParamInfo {
        id: 5,
        name: "Overs",
        group: "Meters",
        min: 0.0,
        max: 1.0e12,
        default: 0.0,
        unit: Unit::None,
        curve: Curve::Linear,
        readonly: true,
    },
    ParamInfo {
        id: 6,
        name: "Clipped",
        group: "Meters",
        min: 0.0,
        max: 1.0e12,
        default: 0.0,
        unit: Unit::None,
        curve: Curve::Linear,
        readonly: true,
    },
];

/// Counters since the last [`MasterSafety::clear_stats`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MasterStats {
    /// Frames that came in above the ceiling.
    pub overs: u64,
    /// Frames that left above 0 dBFS.
    pub clipped: u64,
    /// The most limiting applied, dB (0 or below).
    pub max_reduction_db: f32,
    /// The loudest frame out, dBFS.
    pub peak_out_db: f32,
}

impl Default for MasterStats {
    fn default() -> Self {
        MasterStats {
            overs: 0,
            clipped: 0,
            max_reduction_db: 0.0,
            peak_out_db: -180.0,
        }
    }
}

/// The soft clip's curve: straight to `knee`, then `tanh` bending in to
/// `knee + span`, the ceiling.
#[inline]
fn soft(x: f32, knee: f32, span: f32) -> f32 {
    let a = x.abs();
    if a <= knee {
        x
    } else {
        x.signum() * (knee + span * crate::fast_tanh((a - knee) / span))
    }
}

/// The antiderivative of [`soft`], zero at 0, for ADAA.
#[inline]
fn soft_big(x: f64, knee: f64, span: f64) -> f64 {
    let a = x.abs();
    if a <= knee {
        0.5 * x * x
    } else {
        knee * (a - 0.5 * knee) + span * span * log_cosh((a - knee) / span)
    }
}

/// The soft clip for one channel, at twice the rate.
#[derive(Clone, Debug)]
struct SoftClip {
    iir: IirPair<CLIP_SECTIONS>,
    adaa: Adaa,
    flat: Flat2x,
}

impl SoftClip {
    fn new() -> SoftClip {
        SoftClip {
            iir: IirPair::new(CLIP_TBW),
            adaa: Adaa::default(),
            flat: Flat2x::default(),
        }
    }

    #[inline]
    fn run(&mut self, x: f32, ceil: f32) -> f32 {
        let knee = 0.5 * ceil;
        let span = ceil - knee;
        let (k, s) = (f64::from(knee), f64::from(span));
        let [a, b] = self.iir.up(x);
        let mut one = |v: f32| {
            let y = self
                .adaa
                .run(v, |u| soft(u, knee, span), |u| soft_big(u, k, s));
            self.flat.run(y)
        };
        let (ya, yb) = (one(a), one(b));
        self.iir.down([ya, yb])
    }

    fn reset(&mut self) {
        self.iir.reset();
        self.adaa.reset();
        self.flat.reset();
    }
}

/// The Master safety stage. See the module docs.
pub struct MasterSafety {
    v: [f32; 7],
    rate: u32,
    trim: Smooth,
    ceiling: Smooth,
    dc: [DcBlock; 2],
    core: PeakLimit,
    clip: [SoftClip; 2],
    /// The Safe limiter's and Raw's match for the soft clip's delay.
    pad: [FixedDelay; 2],
    /// The soft clip's share of what the limiter sees, and the limiter's
    /// share of the output: each 0 or 1, gliding between them for
    /// [`FADE_MS`] after a mode switch.
    clip_w: f32,
    limit_w: f32,
    fade_step: f32,
    n: u64,
    stats: MasterStats,
    gain_now: f32,
}

impl Default for MasterSafety {
    fn default() -> Self {
        Self::new()
    }
}

impl MasterSafety {
    /// What a steady peak at `x` (linear, 1 is 0 dBFS) leaves as, after
    /// the trim, in the mode set: the limiter holds it at the ceiling, Soft
    /// clip bends it in from 6 dB under the ceiling, Raw passes it (the
    /// converter clips it above 0 dBFS). For drawing.
    pub fn curve(&self, x: f32) -> f32 {
        let x = x * db_to_gain(self.v[ids::TRIM as usize]);
        let ceil = db_to_gain(self.v[ids::CEILING as usize]);
        let knee = ceil * 0.5;
        match self.v[ids::MODE as usize] as usize {
            0 => x.clamp(-ceil, ceil),
            1 if x.abs() > knee => {
                x.signum()
                    * (knee + (ceil - knee) * crate::fast_tanh((x.abs() - knee) / (ceil - knee)))
            }
            1 => x,
            _ => x.clamp(-1.0, 1.0),
        }
    }

    pub fn new() -> MasterSafety {
        let v = defaults(&PARAMS);
        let mut m = MasterSafety {
            v,
            rate: 0,
            trim: Smooth::new(v[1]),
            ceiling: Smooth::new(db_to_gain(v[2])),
            dc: [DcBlock::new(), DcBlock::new()],
            core: PeakLimit::new(),
            clip: [SoftClip::new(), SoftClip::new()],
            pad: [FixedDelay::new(CLIP_DELAY), FixedDelay::new(CLIP_DELAY)],
            clip_w: 0.0,
            limit_w: 1.0,
            fade_step: 1.0,
            n: 0,
            stats: MasterStats::default(),
            gain_now: 1.0,
        };
        m.core.set_true_peak(true);
        m.prepare(48_000, mpfree_node::MAX_BLOCK);
        m
    }

    pub fn stats(&self) -> MasterStats {
        self.stats
    }

    /// Zeroes the counters (control thread or audio thread; no allocation).
    pub fn clear_stats(&mut self) {
        self.stats = MasterStats::default();
        self.v[ids::OVERS as usize] = 0.0;
        self.v[ids::CLIPPED as usize] = 0.0;
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
            ids::MODE => {
                // The soft clip starts clean when it fades in from nothing.
                if v as usize == 1 && self.clip_w == 0.0 {
                    for c in &mut self.clip {
                        c.reset();
                    }
                }
            }
            ids::TRIM => self.trim.target = v,
            ids::CEILING => self.ceiling.target = db_to_gain(v),
            ids::RELEASE => self.core.set_release(v, false),
            _ => {}
        }
    }

    /// The soft clip's and the limiter's shares in `mode`.
    fn shares(mode: usize) -> (f32, f32) {
        match mode {
            0 => (0.0, 1.0),
            1 => (1.0, 1.0),
            _ => (0.0, 0.0),
        }
    }

    fn clear_state(&mut self) {
        for d in &mut self.pad {
            d.reset();
        }
        for d in &mut self.dc {
            d.reset();
        }
        for c in &mut self.clip {
            c.reset();
        }
        self.core.clear();
        (self.clip_w, self.limit_w) = Self::shares(self.v[ids::MODE as usize] as usize);
        self.n = 0;
        self.gain_now = 1.0;
    }

    fn run(&mut self, l: &mut [f32], r: &mut [f32]) {
        let n = l.len().min(r.len());
        let (l, r) = (&mut l[..n], &mut r[..n]);
        let (clip_to, limit_to) = Self::shares(self.v[ids::MODE as usize] as usize);
        let step = self.fade_step;
        let toward = |w: f32, to: f32| w + (to - w).clamp(-step, step);
        // In stages over the piece, each frame by frame: what the limiter
        // sees, the limiter, then the meters. The ceiling glides the same
        // in the first two (a copy of its smoother replays it).
        let mut ceiling = self.ceiling;
        // The trim as a gain, worked out again only while it glides.
        let (mut t_db, mut t) = (f32::NAN, 1.0);
        for i in 0..n {
            let d = self.trim.next();
            if d != t_db {
                t_db = d;
                t = db_to_gain(d);
            }
            let ceil = self.ceiling.next();
            let a = self.dc[0].run(l[i] * t);
            let b = self.dc[1].run(r[i] * t);
            if a.abs().max(b.abs()) > ceil {
                self.stats.overs += 1;
            }
            self.clip_w = toward(self.clip_w, clip_to);
            let plain = [self.pad[0].run(a), self.pad[1].run(b)];
            let pre = if self.clip_w > 0.0 {
                if self.n.is_multiple_of(SETTLE) {
                    for c in &mut self.clip {
                        c.iir.settle();
                    }
                }
                let w = self.clip_w;
                let sa = self.clip[0].run(a, ceil);
                let sb = self.clip[1].run(b, ceil);
                [
                    plain[0] + (sa - plain[0]) * w,
                    plain[1] + (sb - plain[1]) * w,
                ]
            } else {
                plain
            };
            self.n += 1;
            l[i] = pre[0];
            r[i] = pre[1];
        }
        // Most of the time nothing is near the ceiling: a quiet piece with
        // the limiter at rest needs no gain, only its delay.
        if ceiling.settled() && self.core.quiet_block(l, r, 1.0, ceiling.target) {
            // The output is the same in every share of the limiter; only
            // a mode's fade moves on.
            if self.limit_w != limit_to {
                for _ in 0..n {
                    self.limit_w = toward(self.limit_w, limit_to);
                }
            }
            self.gain_now = 1.0;
        } else {
            for i in 0..n {
                let ceil = ceiling.next();
                self.limit_w = toward(self.limit_w, limit_to);
                let ([da, db], g) = self.core.run([l[i], r[i]], ceil);
                let g = 1.0 + (g - 1.0) * self.limit_w;
                let (ya, yb) = if self.limit_w >= 1.0 {
                    // The gain already keeps every frame under the
                    // ceiling; the clamp only acts while the ceiling glides
                    // down.
                    ((da * g).clamp(-ceil, ceil), (db * g).clamp(-ceil, ceil))
                } else {
                    (da * g, db * g)
                };
                self.gain_now = g;
                l[i] = ya;
                r[i] = yb;
            }
        }
        // The loudest output, turned into dB once per piece.
        let mut loudest = 0.0f32;
        for (a, b) in l.iter().zip(r.iter()) {
            let out_peak = a.abs().max(b.abs());
            if out_peak > 1.0 {
                self.stats.clipped += 1;
            }
            loudest = loudest.max(out_peak);
        }
        let pd = gain_to_db(loudest);
        if pd > self.stats.peak_out_db {
            self.stats.peak_out_db = pd;
        }
        let red = gain_to_db(self.gain_now).min(0.0);
        if red < self.stats.max_reduction_db {
            self.stats.max_reduction_db = red;
        }
        self.v[ids::REDUCTION as usize] = red.max(-60.0);
        self.v[ids::OVERS as usize] = self.stats.overs as f32;
        self.v[ids::CLIPPED as usize] = self.stats.clipped as f32;
    }
}

impl Node for MasterSafety {
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
        self.rate = rate;
        self.trim.set_time(rate, 20.0);
        self.ceiling.set_time(rate, 20.0);
        for d in &mut self.dc {
            d.set(5.0, rate);
        }
        self.fade_step = 1.0 / (FADE_MS * 0.001 * rate as f32).max(1.0);
        self.core.set_release(self.v[ids::RELEASE as usize], false);
        self.core.setup(rate, LOOKAHEAD_MS);
        self.clear_state();
    }
    fn reset(&mut self) {
        self.trim.snap(self.trim.target);
        self.ceiling.snap(self.ceiling.target);
        self.clear_state();
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
        self.core.latency() + CLIP_DELAY as u32
    }
    fn tail_frames(&self) -> u32 {
        self.latency()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_latency_is_the_look_ahead_the_detector_and_the_soft_clip() {
        // 1.5 ms (72 frames) at 48 kHz, the true-peak detector's 15 and
        // the soft clip's 4.
        let m = MasterSafety::new();
        assert_eq!(m.latency(), 72 + 15 + 4);
        assert_eq!(m.pad[0].frames(), CLIP_DELAY);
    }
}
