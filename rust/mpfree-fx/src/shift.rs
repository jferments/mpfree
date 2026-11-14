//! The [`FreqShifter`]: moves every frequency in the sound up or down by
//! the same number of hertz (a single-sideband shift), so harmonics stop
//! being harmonic. Small shifts (under a few hertz) sound like a slow
//! phaser that never repeats; bigger ones turn drums metallic and voices
//! robotic; with Feedback the shifted sound goes round again, climbing or
//! falling in a spiral.
//!
//! It covers the MPC's Frequency Shifter and AIR Freq Shift (whose modes
//! it shares: Up, Down, Up and down, and Stereo, which shifts the left
//! side up and the right side down).
//!
//! How: a pair of all-pass chains whose outputs stay 90 degrees apart
//! (Olli Niemitalo's eighth-order design: within 0.7 degrees from 20 Hz
//! to 22 kHz at 48 kHz, so the unwanted side is about 44 dB down) makes
//! the sound's analytic signal; multiplying it by a turning phasor and
//! keeping the real part shifts it. No delay line, so no latency, and
//! the cost is sixteen multiplies and the phasor a frame per side.

use mpfree_node::{Ctx, Kind, NodeInfo, ParamInfo, Smooth};

use crate::kit::{self, node_impl, param, Choice, Glide, Hz, Linear, Log, Percent, NONE};
use crate::{defaults, fast_exp2, fast_tanh, flush, lerp, Preset};

/// [`FreqShifter`] parameter ids.
pub mod ids {
    pub const SHIFT: u16 = 0;
    pub const MODE: u16 = 1;
    pub const FEEDBACK: u16 = 2;
    pub const MIX: u16 = 3;
}

/// The modes, as the MPC names them.
pub const MODES: &[&str] = &["Up", "Down", "Up and down", "Stereo"];

static INFO: NodeInfo = NodeInfo {
    id: "mpfree.fx.freqshift",
    name: "Freq shifter",
    kind: Kind::Effect,
};
static PARAMS: [ParamInfo; 4] = [
    param(0, "Shift", "Shift", 0.01, 5000.0, 100.0, Hz, Log),
    param(1, "Mode", "Shift", 0.0, 3.0, 0.0, NONE, Choice(MODES)),
    param(2, "Feedback", "Shift", 0.0, 95.0, 0.0, Percent, Linear),
    param(3, "Mix", "Shift", 0.0, 100.0, 100.0, Percent, Linear),
];

/// Frequency shifter factory presets.
pub static PRESETS: &[Preset] = &[
    Preset {
        name: "Barber pole",
        values: &[(ids::SHIFT, 0.3), (ids::FEEDBACK, 60.0)],
    },
    Preset {
        name: "Metal drums",
        values: &[(ids::SHIFT, 180.0)],
    },
    Preset {
        name: "Falling spiral",
        values: &[(ids::SHIFT, 12.0), (ids::MODE, 1.0), (ids::FEEDBACK, 80.0)],
    },
    Preset {
        name: "Wide shimmer",
        values: &[(ids::SHIFT, 2.0), (ids::MODE, 3.0)],
    },
    Preset {
        name: "Robot",
        values: &[(ids::SHIFT, 60.0), (ids::MODE, 2.0)],
    },
];

/// The all-pass coefficients (the squares of Niemitalo's) of the chain
/// that lags by 90 degrees (it ends with a one-frame delay) and of the
/// one that leads.
const LAG: [f32; 4] = [0.479_400_87, 0.876_218_5, 0.976_597_6, 0.997_499_3];
const LEAD: [f32; 4] = [0.161_758_5, 0.733_028_9, 0.945_349_7, 0.990_599_2];

/// One chain of four second-order all-passes, `y = c (x + y[-2]) - x[-2]`.
#[derive(Clone, Copy, Debug, Default)]
struct Chain {
    x1: [f32; 4],
    x2: [f32; 4],
    y1: [f32; 4],
    y2: [f32; 4],
}

impl Chain {
    #[inline]
    fn run(&mut self, c: &[f32; 4], x: f32) -> f32 {
        let mut x = x;
        for k in 0..4 {
            let y = flush(c[k] * (x + self.y2[k]) - self.x2[k]);
            self.x2[k] = self.x1[k];
            self.x1[k] = x;
            self.y2[k] = self.y1[k];
            self.y1[k] = y;
            x = y;
        }
        x
    }
}

/// The 90-degree pair for one channel.
#[derive(Clone, Copy, Debug, Default)]
struct Quad {
    lag: Chain,
    lead: Chain,
    /// The lagging chain's last output (its one-frame delay).
    held: f32,
}

impl Quad {
    /// `(in phase, quadrature)`: the quadrature is the in-phase part
    /// 90 degrees later.
    #[inline]
    fn run(&mut self, x: f32) -> (f32, f32) {
        let q = self.held;
        self.held = self.lag.run(&LAG, x);
        (self.lead.run(&LEAD, x), q)
    }
}

/// A single-sideband frequency shifter with feedback.
pub struct FreqShifter {
    v: [f32; 4],
    rate: u32,
    quad: [Quad; 2],
    /// The phasor's phase in cycles. Double precision so a shift of a
    /// hundredth of a hertz still turns smoothly.
    phase: f64,
    /// The shift as log2 Hz, so a sweep glides evenly in pitch.
    pitch: Smooth,
    feedback: Smooth,
    mix: Smooth,
    /// Each side's last output, for the feedback.
    last: [f32; 2],
}

impl Default for FreqShifter {
    fn default() -> Self {
        Self::new()
    }
}

impl FreqShifter {
    pub fn new() -> FreqShifter {
        let v = defaults(&PARAMS);
        let mut f = FreqShifter {
            v,
            rate: 48_000,
            quad: [Quad::default(); 2],
            phase: 0.0,
            pitch: Smooth::new(v[0].log2()),
            feedback: Smooth::new(v[2] / 100.0),
            mix: Smooth::new(v[3] / 100.0),
            last: [0.0; 2],
        };
        f.setup(48_000);
        f
    }
    fn apply(&mut self, id: u16, v: f32) {
        match id {
            ids::SHIFT => self.pitch.target = v.log2(),
            ids::FEEDBACK => self.feedback.target = v / 100.0,
            ids::MIX => self.mix.target = v / 100.0,
            _ => {}
        }
    }
    fn begin(&mut self, _ctx: &Ctx) {}
    fn run(&mut self, _ctx: &Ctx, _from: usize, l: &mut [f32], r: &mut [f32]) {
        let mode = self.v[1] as usize;
        let per = 1.0 / self.rate as f64;
        for i in 0..l.len() {
            let hz = fast_exp2(self.pitch.glide()) as f64;
            self.phase += hz * per;
            if self.phase >= 1.0 {
                self.phase -= 1.0;
            }
            let p = self.phase as f32;
            let (c, s) = (kit::cosine(p), kit::sine(p));
            let fb = self.feedback.glide();
            // Each pass round the loop lands on new frequencies, so the
            // passes add in power: this keeps the level where it was.
            let comp = (1.0 - fb * fb).sqrt();
            let mix = self.mix.glide();
            for (ch, x) in [&mut l[i], &mut r[i]].into_iter().enumerate() {
                let dry = *x;
                let (re, im) = self.quad[ch].run(dry + fb * self.last[ch]);
                let up = re * c - im * s;
                let down = re * c + im * s;
                let y = match mode {
                    1 => down,
                    2 => std::f32::consts::FRAC_1_SQRT_2 * (up + down),
                    3 if ch == 1 => down,
                    _ => up,
                };
                // Soft in the loop, so high feedback can't run away.
                self.last[ch] = fast_tanh(y);
                *x = lerp(dry, y * comp, mix);
            }
        }
    }
    fn setup(&mut self, rate: u32) {
        self.rate = rate;
        self.pitch.set_time(rate, 30.0);
        self.feedback.set_time(rate, 20.0);
        self.mix.set_time(rate, 20.0);
    }
    fn clear(&mut self) {
        self.quad = [Quad::default(); 2];
        self.phase = 0.0;
        self.last = [0.0; 2];
        self.pitch.snap(self.pitch.target);
        self.feedback.snap(self.feedback.target);
        self.mix.snap(self.mix.target);
    }
    fn tail(&self) -> u32 {
        // Feedback rings for a while; the all-passes for a few frames.
        if self.v[2] > 0.0 {
            self.rate
        } else {
            self.rate / 100
        }
    }
    fn lat(&self) -> u32 {
        0
    }
}

node_impl!(FreqShifter, INFO, PARAMS);

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_node::Node;

    /// The level (0 to 1 of a full-scale sine) at `hz` in `x`, by a
    /// single-bin DFT over whole cycles of 1 Hz.
    fn level_at(x: &[f32], hz: f32) -> f32 {
        let n = x.len();
        let (mut re, mut im) = (0.0f64, 0.0f64);
        for (i, v) in x.iter().enumerate() {
            let w = std::f64::consts::TAU * hz as f64 * i as f64 / 48_000.0;
            re += *v as f64 * w.cos();
            im += *v as f64 * w.sin();
        }
        (2.0 * (re * re + im * im).sqrt() / n as f64) as f32
    }

    /// Shifts a 1 kHz sine and returns both sides' last second.
    fn shifted(f: &mut FreqShifter) -> (Vec<f32>, Vec<f32>) {
        f.prepare(48_000, 128);
        f.reset();
        let n = 96_000;
        let mut l: Vec<f32> = (0..n)
            .map(|i| 0.5 * (i as f32 * 1000.0 / 48_000.0 * std::f32::consts::TAU).sin())
            .collect();
        let mut r = l.clone();
        let ctx = Ctx::new(48_000, 0);
        for k in (0..n).step_by(128) {
            f.process(&ctx, &[], &mut l[k..k + 128], &mut r[k..k + 128]);
        }
        (l[48_000..].to_vec(), r[48_000..].to_vec())
    }

    #[test]
    fn up_moves_the_tone_up_and_leaves_no_image() {
        let mut f = FreqShifter::new();
        f.set_param(ids::SHIFT, 100.0);
        let (l, _) = shifted(&mut f);
        let want = level_at(&l, 1100.0);
        let image = level_at(&l, 900.0);
        let left = level_at(&l, 1000.0);
        assert!((want - 0.5).abs() < 0.02, "{want}");
        assert!(image < 0.5 * 0.01, "image {image}");
        assert!(left < 0.5 * 0.01, "carrier {left}");
    }

    #[test]
    fn down_and_stereo_go_the_other_way() {
        let mut f = FreqShifter::new();
        f.set_param(ids::SHIFT, 100.0);
        f.set_param(ids::MODE, 1.0);
        let (l, _) = shifted(&mut f);
        assert!(level_at(&l, 900.0) > 0.48 && level_at(&l, 1100.0) < 0.005);
        f.set_param(ids::MODE, 3.0);
        let (l, r) = shifted(&mut f);
        assert!(level_at(&l, 1100.0) > 0.48 && level_at(&r, 900.0) > 0.48);
        f.set_param(ids::MODE, 2.0);
        let (l, _) = shifted(&mut f);
        let (a, b) = (level_at(&l, 1100.0), level_at(&l, 900.0));
        assert!((a - b).abs() < 0.01 && a > 0.3, "{a} {b}");
    }

    #[test]
    fn full_feedback_stays_bounded() {
        let mut f = FreqShifter::new();
        f.set_param(ids::SHIFT, 5.0);
        f.set_param(ids::FEEDBACK, 95.0);
        let (l, r) = shifted(&mut f);
        let peak = l.iter().chain(r.iter()).fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(peak.is_finite() && peak < 12.0, "{peak}");
    }
}
