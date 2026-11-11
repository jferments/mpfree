//! Drive: deliberate distortion done cleanly. The shaper runs at 2x (Eco)
//! or 4x (HQ) the sample rate so its new harmonics are filtered before they
//! can fold back as aliasing; a DC blocker removes the offset an
//! asymmetric curve makes; auto-gain keeps the level where it was, so more
//! drive means more grit, not just louder.

use std::f32::consts::{FRAC_PI_2, PI};

use mpfree_node::{Ctx, Curve, Kind, Node, NodeEvent, NodeInfo, ParamInfo, Smooth, Timed, Unit};

use crate::oversample::{Down2, Up2, TAPS_2X, TAPS_4X};
use crate::svf::{Segs, Svf, SvfMode};
use crate::{db_to_gain, defaults, gain_to_db, DcBlock, FixedDelay};

/// [`Drive`] parameter ids.
pub mod ids {
    pub const MODE: u16 = 0;
    pub const DRIVE: u16 = 1;
    pub const TONE: u16 = 2;
    pub const BIAS: u16 = 3;
    pub const MIX: u16 = 4;
    pub const OUTPUT: u16 = 5;
    pub const AUTO_GAIN: u16 = 6;
    pub const QUALITY: u16 = 7;
    /// Meter: output level against input level, in dB.
    pub const LEVEL_CHANGE: u16 = 8;
}

const MODES: &[&str] = &["Soft", "Hard", "Tube", "Foldback", "Fuzz", "Rectify"];
const QUALITY: &[&str] = &["Eco 2x", "HQ 4x"];

static INFO: NodeInfo = NodeInfo { id: "mpfree.fx.drive", name: "Drive", kind: Kind::Effect };
static PARAMS: [ParamInfo; 9] = [
    ParamInfo { id: 0, name: "Mode", group: "Drive", min: 0.0, max: 5.0, default: 0.0, unit: Unit::None, curve: Curve::Choice(MODES), readonly: false },
    ParamInfo { id: 1, name: "Drive", group: "Drive", min: 0.0, max: 36.0, default: 12.0, unit: Unit::Db, curve: Curve::Linear, readonly: false },
    ParamInfo { id: 2, name: "Tone", group: "Drive", min: 500.0, max: 20_000.0, default: 20_000.0, unit: Unit::Hz, curve: Curve::Log, readonly: false },
    ParamInfo { id: 3, name: "Bias", group: "Drive", min: -100.0, max: 100.0, default: 20.0, unit: Unit::Percent, curve: Curve::Linear, readonly: false },
    ParamInfo { id: 4, name: "Mix", group: "Output", min: 0.0, max: 100.0, default: 100.0, unit: Unit::Percent, curve: Curve::Linear, readonly: false },
    ParamInfo { id: 5, name: "Output", group: "Output", min: -24.0, max: 12.0, default: 0.0, unit: Unit::Db, curve: Curve::Linear, readonly: false },
    ParamInfo { id: 6, name: "Auto-gain", group: "Output", min: 0.0, max: 1.0, default: 1.0, unit: Unit::None, curve: Curve::Toggle, readonly: false },
    ParamInfo { id: 7, name: "Quality", group: "Output", min: 0.0, max: 1.0, default: 0.0, unit: Unit::None, curve: Curve::Choice(QUALITY), readonly: false },
    ParamInfo { id: 8, name: "Level change", group: "Meters", min: -48.0, max: 48.0, default: 0.0, unit: Unit::Db, curve: Curve::Linear, readonly: true },
];

/// The delay in base frames. The inner 2x-to-4x pair delays by
/// `(TAPS_4X - 1) / 2` samples at the 2x rate, half a base frame short of a
/// whole one, so the 4x path adds one 2x sample to make it whole; the 2x
/// path is padded to the same total, so switching quality never moves the
/// sound in time.
const LATENCY: usize = (TAPS_2X - 1) / 2 + (TAPS_4X - 1).div_ceil(4);
const PAD_2X: usize = LATENCY - (TAPS_2X - 1) / 2;
const _: () = assert!(2 * PAD_2X == (TAPS_4X - 1) / 2 + 1, "the 4x path pads exactly one 2x sample");
/// The level auto-gain matches at: -12 dBFS, the instruments' target peak.
const REF: f32 = 0.25;

#[inline]
fn shape(mode: usize, x: f32, b: f32) -> f32 {
    match mode {
        1 => x.clamp(-1.0, 1.0),
        2 => (x + b).tanh() - b.tanh(),
        3 => (FRAC_PI_2 * x).sin(),
        4 => {
            let s = |u: f32| u.signum() * (1.0 - (-2.0 * u.abs()).exp());
            s(x + b) - s(b)
        }
        5 => x.abs().tanh(),
        _ => x.tanh(),
    }
}

/// Gain that brings a sine at [`REF`] back to its own level after the
/// shaper (DC removed), from the RMS of one cycle.
fn auto_gain(mode: usize, drive: f32, b: f32) -> f32 {
    const N: usize = 64;
    let mut sum = 0.0f32;
    let mut sq = 0.0f32;
    for i in 0..N {
        let y = shape(mode, REF * drive * (2.0 * PI * i as f32 / N as f32).sin(), b);
        sum += y;
        sq += y * y;
    }
    let mean = sum / N as f32;
    let rms = (sq / N as f32 - mean * mean).max(0.0).sqrt();
    let want = REF / std::f32::consts::SQRT_2;
    if rms < 1e-6 { 1.0 } else { (want / rms).min(64.0) }
}

/// `db_to_gain(db)`, recomputed only when `db` changed.
#[inline]
fn cached(c: &mut (f32, f32), db: f32) -> f32 {
    if c.0 != db { *c = (db, db_to_gain(db)); }
    c.1
}

struct Channel {
    up1: Up2,
    down1: Down2,
    up2: Up2,
    down2: Down2,
    pad: FixedDelay,
    /// The 4x path's one-sample pad at the 2x rate.
    z: f32,
    dry: FixedDelay,
    dc: DcBlock,
}

impl Channel {
    fn new() -> Channel {
        Channel { up1: Up2::new(TAPS_2X), down1: Down2::new(TAPS_2X), up2: Up2::new(TAPS_4X), down2: Down2::new(TAPS_4X), pad: FixedDelay::new(PAD_2X), z: 0.0, dry: FixedDelay::new(LATENCY), dc: DcBlock::new() }
    }
    fn reset(&mut self) {
        self.up1.reset();
        self.down1.reset();
        self.up2.reset();
        self.down2.reset();
        self.pad.reset();
        self.z = 0.0;
        self.dry.reset();
        self.dc.reset();
    }
}

/// Oversampled drive with six curves, tone, bias, mix and auto-gain.
pub struct Drive {
    v: [f32; 9],
    rate: u32,
    drive_db: Smooth,
    bias: Smooth,
    log_tone: Smooth,
    mix: Smooth,
    out_db: Smooth,
    comp: Smooth,
    ch: [Channel; 2],
    tone: Svf,
    ms_in: f32,
    ms_out: f32,
    ms_coef: f32,
    /// The last drive and output levels in dB and their gains, so the
    /// gains are only recomputed while a level moves.
    gains: [(f32, f32); 2],
}

impl Default for Drive {
    fn default() -> Self { Self::new() }
}

impl Drive {
    /// Allocates the oversamplers (control thread).
    pub fn new() -> Drive {
        let v = defaults(&PARAMS);
        let mut d = Drive {
            v,
            rate: 48_000,
            drive_db: Smooth::new(v[1]),
            bias: Smooth::new(v[3] / 100.0),
            log_tone: Smooth::new(v[2].log2()),
            mix: Smooth::new(v[4] / 100.0),
            out_db: Smooth::new(v[5]),
            comp: Smooth::new(1.0),
            ch: [Channel::new(), Channel::new()],
            tone: Svf::default(),
            ms_in: 0.0,
            ms_out: 0.0,
            ms_coef: 0.0,
            gains: [(0.0, 1.0); 2],
        };
        d.comp.snap(d.comp_target());
        d.prepare(48_000, mpfree_node::MAX_BLOCK);
        d
    }

    fn comp_target(&self) -> f32 {
        if self.v[6] < 0.5 { 1.0 } else { auto_gain(self.v[0] as usize, db_to_gain(self.v[1]), self.v[3] / 100.0) }
    }

    fn apply(&mut self, id: u16, value: f32) {
        let Some(p) = PARAMS.get(id as usize) else { return };
        if p.readonly { return; }
        let v = p.clamp(value);
        let old = self.v[id as usize];
        self.v[id as usize] = v;
        match id {
            ids::DRIVE => self.drive_db.target = v,
            ids::TONE => self.log_tone.target = v.log2(),
            ids::BIAS => self.bias.target = v / 100.0,
            ids::MIX => self.mix.target = v / 100.0,
            ids::OUTPUT => self.out_db.target = v,
            ids::QUALITY if v != old => for c in &mut self.ch { c.up1.reset(); c.down1.reset(); c.up2.reset(); c.down2.reset(); c.pad.reset(); c.z = 0.0; },
            _ => {}
        }
        if matches!(id, ids::MODE | ids::DRIVE | ids::BIAS | ids::AUTO_GAIN) { self.comp.target = self.comp_target(); }
    }

    fn set_tone(&mut self) { self.tone.set(SvfMode::LowPass, self.log_tone.value.exp2(), 0.707, 0.0, self.rate); }

    fn run(&mut self, l: &mut [f32], r: &mut [f32]) {
        let mode = self.v[0] as usize;
        let hq = self.v[7] >= 0.5;
        for i in 0..l.len() {
            let g = cached(&mut self.gains[0], self.drive_db.next());
            let b = self.bias.next();
            if !self.log_tone.settled() {
                self.log_tone.next();
                self.set_tone();
            }
            let mix = self.mix.next();
            let out = cached(&mut self.gains[1], self.out_db.next());
            let comp = self.comp.next();
            let x = [l[i], r[i]];
            let mut y = [0.0f32; 2];
            for c in 0..2 {
                let chn = &mut self.ch[c];
                let pre = x[c] * g;
                let [a, bb] = chn.up1.run(pre);
                let wet = if hq {
                    let [a0, a1] = chn.up2.run(a);
                    let [b0, b1] = chn.up2.run(bb);
                    let sa = chn.down2.run([shape(mode, a0, b), shape(mode, a1, b)]);
                    let sb = chn.down2.run([shape(mode, b0, b), shape(mode, b1, b)]);
                    let w = chn.down1.run([chn.z, sa]);
                    chn.z = sb;
                    w
                } else {
                    let w = chn.down1.run([shape(mode, a, b), shape(mode, bb, b)]);
                    chn.pad.run(w)
                };
                let wet = chn.dc.run(wet) * comp;
                let wet = self.tone.run(c, wet);
                let dry = chn.dry.run(x[c]);
                y[c] = crate::lerp(dry, wet, mix) * out;
            }
            let k = self.ms_coef;
            self.ms_in += k * (0.5 * (x[0] * x[0] + x[1] * x[1]) - self.ms_in);
            self.ms_out += k * (0.5 * (y[0] * y[0] + y[1] * y[1]) - self.ms_out);
            self.ms_in = crate::flush(self.ms_in);
            self.ms_out = crate::flush(self.ms_out);
            l[i] = y[0];
            r[i] = y[1];
        }
        self.v[ids::LEVEL_CHANGE as usize] = if self.ms_in > 1e-10 { (gain_to_db((self.ms_out / self.ms_in).sqrt())).clamp(-48.0, 48.0) } else { 0.0 };
    }
}

impl Node for Drive {
    fn info(&self) -> &'static NodeInfo { &INFO }
    fn params(&self) -> &'static [ParamInfo] { &PARAMS }
    fn set_param(&mut self, id: u16, value: f32) { self.apply(id, value); }
    fn param(&self, id: u16) -> f32 { self.v.get(id as usize).copied().unwrap_or(f32::NAN) }
    fn prepare(&mut self, rate: u32, _max_block: usize) {
        self.rate = rate.max(1);
        for s in [&mut self.drive_db, &mut self.bias, &mut self.log_tone, &mut self.mix, &mut self.out_db, &mut self.comp] { s.set_time(self.rate, 20.0); }
        for c in &mut self.ch { c.dc.set(10.0, self.rate); }
        self.ms_coef = 1.0 - (-1.0 / (0.3 * self.rate as f32)).exp();
        self.set_tone();
    }
    fn reset(&mut self) {
        for s in [&mut self.drive_db, &mut self.bias, &mut self.log_tone, &mut self.mix, &mut self.out_db, &mut self.comp] { s.snap(s.target); }
        for c in &mut self.ch { c.reset(); }
        self.tone.reset();
        self.set_tone();
        self.ms_in = 0.0;
        self.ms_out = 0.0;
    }
    fn process(&mut self, _ctx: &Ctx, events: &[Timed], left: &mut [f32], right: &mut [f32]) {
        let len = left.len().min(right.len());
        let mut segs = Segs::new(events);
        while let Some((from, to)) = segs.next(len, |ev| if let NodeEvent::Param { id, value } = *ev { self.apply(id, value) }) {
            self.run(&mut left[from..to], &mut right[from..to]);
        }
    }
    fn latency(&self) -> u32 { LATENCY as u32 }
    fn tail_frames(&self) -> u32 { LATENCY as u32 + self.rate / 20 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_gain_matches_reference_level() {
        for mode in 0..6 {
            for drive_db in [0.0f32, 12.0, 30.0] {
                let g = auto_gain(mode, db_to_gain(drive_db), 0.2);
                assert!(g.is_finite() && g > 0.0 && g <= 64.0, "mode {mode} drive {drive_db}: {g}");
            }
        }
        // Clean tanh at 0 dB drive and -12 dBFS is nearly linear: gain near 1.
        assert!((auto_gain(0, 1.0, 0.0) - 1.0).abs() < 0.02);
    }
}
