//! Compressor: a feed-forward, soft-knee compressor with the gain computed
//! and smoothed in decibels (Giannoulis, Massberg and Reiss, "Digital
//! Dynamic Range Compressor Design", JAES 2012: the smooth, decoupled peak
//! detector), stereo-linked, with parallel mix and a sidechain input for
//! ducking.

use mpfree_node::{Ctx, Curve, Kind, Node, NodeEvent, NodeInfo, ParamInfo, Smooth, Timed, Unit};

use crate::svf::Segs;
use crate::{db_to_gain, defaults, gain_to_db};

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
}

static INFO: NodeInfo = NodeInfo { id: "mpfree.fx.compressor", name: "Compressor", kind: Kind::Effect };
static PARAMS: [ParamInfo; 9] = [
    ParamInfo { id: 0, name: "Threshold", group: "Compressor", min: -60.0, max: 0.0, default: -18.0, unit: Unit::Db, curve: Curve::Linear, readonly: false },
    ParamInfo { id: 1, name: "Ratio", group: "Compressor", min: 1.0, max: 20.0, default: 4.0, unit: Unit::None, curve: Curve::Log, readonly: false },
    ParamInfo { id: 2, name: "Attack", group: "Compressor", min: 0.1, max: 100.0, default: 10.0, unit: Unit::Ms, curve: Curve::Log, readonly: false },
    ParamInfo { id: 3, name: "Release", group: "Compressor", min: 10.0, max: 2000.0, default: 150.0, unit: Unit::Ms, curve: Curve::Log, readonly: false },
    ParamInfo { id: 4, name: "Knee", group: "Compressor", min: 0.0, max: 24.0, default: 6.0, unit: Unit::Db, curve: Curve::Linear, readonly: false },
    ParamInfo { id: 5, name: "Makeup", group: "Output", min: 0.0, max: 24.0, default: 0.0, unit: Unit::Db, curve: Curve::Linear, readonly: false },
    ParamInfo { id: 6, name: "Mix", group: "Output", min: 0.0, max: 100.0, default: 100.0, unit: Unit::Percent, curve: Curve::Linear, readonly: false },
    ParamInfo { id: 7, name: "Sidechain", group: "Output", min: 0.0, max: 1.0, default: 0.0, unit: Unit::None, curve: Curve::Toggle, readonly: false },
    ParamInfo { id: 8, name: "Reduction", group: "Meters", min: -60.0, max: 0.0, default: 0.0, unit: Unit::Db, curve: Curve::Linear, readonly: true },
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
    v: [f32; 9],
    rate: u32,
    thresh: Smooth,
    makeup: Smooth,
    mix: Smooth,
    /// Smoothed gain reduction, positive dB.
    env: f32,
    att: f32,
    rel: f32,
}

impl Default for Compressor {
    fn default() -> Self { Self::new() }
}

impl Compressor {
    pub fn new() -> Compressor {
        let v = defaults(&PARAMS);
        let mut c = Compressor { v, rate: 48_000, thresh: Smooth::new(v[0]), makeup: Smooth::new(v[5]), mix: Smooth::new(v[6] / 100.0), env: 0.0, att: 0.0, rel: 0.0 };
        c.prepare(48_000, mpfree_node::MAX_BLOCK);
        c
    }

    fn times(&mut self) {
        let fs = self.rate as f32;
        self.att = (-1.0 / (self.v[2] * 0.001 * fs)).exp();
        self.rel = (-1.0 / (self.v[3] * 0.001 * fs)).exp();
    }

    fn apply(&mut self, id: u16, value: f32) {
        let Some(p) = PARAMS.get(id as usize) else { return };
        if p.readonly { return; }
        let v = p.clamp(value);
        self.v[id as usize] = v;
        match id {
            ids::THRESHOLD => self.thresh.target = v,
            ids::MAKEUP => self.makeup.target = v,
            ids::MIX => self.mix.target = v / 100.0,
            ids::ATTACK | ids::RELEASE => self.times(),
            _ => {}
        }
    }

    fn run(&mut self, l: &mut [f32], r: &mut [f32], key: Option<(&[f32], &[f32])>) {
        let ratio = self.v[1];
        let knee = self.v[4];
        for i in 0..l.len() {
            let t = self.thresh.next();
            let makeup = self.makeup.next();
            let mix = self.mix.next();
            let lvl = match key {
                Some((kl, kr)) => kl[i].abs().max(kr[i].abs()),
                None => l[i].abs().max(r[i].abs()),
            };
            let x = gain_to_db(lvl).max(-120.0);
            let want = x - curve(x, t, ratio, knee); // reduction wanted, >= 0
            let k = if want > self.env { self.att } else { self.rel };
            self.env = crate::flush(k * self.env + (1.0 - k) * want);
            let g = db_to_gain(makeup - self.env);
            let wet = 1.0 + mix * (g - 1.0);
            l[i] *= wet;
            r[i] *= wet;
        }
        self.v[ids::REDUCTION as usize] = (-self.env).clamp(-60.0, 0.0);
    }
}

impl Node for Compressor {
    fn info(&self) -> &'static NodeInfo { &INFO }
    fn params(&self) -> &'static [ParamInfo] { &PARAMS }
    fn set_param(&mut self, id: u16, value: f32) { self.apply(id, value); }
    fn param(&self, id: u16) -> f32 { self.v.get(id as usize).copied().unwrap_or(f32::NAN) }
    fn prepare(&mut self, rate: u32, _max_block: usize) {
        self.rate = rate.max(1);
        for s in [&mut self.thresh, &mut self.makeup, &mut self.mix] { s.set_time(self.rate, 20.0); }
        self.times();
    }
    fn reset(&mut self) {
        for s in [&mut self.thresh, &mut self.makeup, &mut self.mix] { s.snap(s.target); }
        self.env = 0.0;
        self.v[ids::REDUCTION as usize] = 0.0;
    }
    fn process(&mut self, ctx: &Ctx, events: &[Timed], left: &mut [f32], right: &mut [f32]) {
        let len = left.len().min(right.len());
        let side = if self.v[7] >= 0.5 { ctx.sidechain.filter(|(a, b)| a.len() >= len && b.len() >= len) } else { None };
        let mut segs = Segs::new(events);
        while let Some((from, to)) = segs.next(len, |ev| if let NodeEvent::Param { id, value } = *ev { self.apply(id, value) }) {
            let key = side.map(|(a, b)| (&a[from..to], &b[from..to]));
            self.run(&mut left[from..to], &mut right[from..to], key);
        }
    }
    fn tail_frames(&self) -> u32 { 0 }
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
}
