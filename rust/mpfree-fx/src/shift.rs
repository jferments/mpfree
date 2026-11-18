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
//! makes the sound's analytic signal; multiplying it by a turning phasor
//! and keeping the real part shifts it. The pair is Valenzuela and
//! Constantinides' elliptic design (the IIR half-band's, turned a quarter
//! of the rate), twelve coefficients worked out for the rate: within 0.06
//! degrees from 35 Hz to as far below Nyquist, so the unwanted side is
//! 65 dB down or more at 44.1 and 48 kHz (FX quality Z1; Niemitalo's
//! eight-coefficient pair it replaces was 44). The phasor is a precise
//! sine of a double-precision phase, so it adds no spurs (Z2). A Mode
//! change crossfades. Feedback goes round through a soft clip and a
//! 20 Hz to 18 kHz band, since at 0 Hz and Nyquist the pair is not 90
//! degrees apart and a shift folds the sound there back onto itself
//! (Z6); with no Feedback the loop is skipped. No delay line, so no
//! latency; 24 multiplies a frame per side, and the phasor.

use mpfree_node::{Ctx, Kind, NodeInfo, ParamInfo, Smooth};

use crate::kit::{self, node_impl, param, Choice, Glide, Hz, Linear, Log, Percent, NONE};
use crate::oversample::iir_halfband;
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

/// Second-order all-passes in each chain of the 90-degree pair.
const SECTIONS: usize = 6;
/// The pair's lowest frequency, Hz: from here up to as far below Nyquist
/// the two chains are 90 degrees apart.
const LOW_HZ: f64 = 35.0;

/// One chain of second-order all-passes, `y = c (x + y[-2]) - x[-2]`.
#[derive(Clone, Copy, Debug, Default)]
struct Chain {
    x1: [f32; SECTIONS],
    x2: [f32; SECTIONS],
    y1: [f32; SECTIONS],
    y2: [f32; SECTIONS],
}

impl Chain {
    #[inline]
    fn run(&mut self, c: &[f32; SECTIONS], x: f32) -> f32 {
        let mut x = x;
        for k in 0..SECTIONS {
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
    fn run(&mut self, net: &Net, x: f32) -> (f32, f32) {
        let q = self.held;
        self.held = self.lag.run(&net.lag, x);
        (self.lead.run(&net.lead, x), q)
    }
}

/// The pair's coefficients: the chain that lags by 90 degrees (it ends
/// with a one-frame delay) takes the design's odd ones, the one that
/// leads its even ones.
#[derive(Clone, Copy, Debug, Default)]
struct Net {
    lag: [f32; SECTIONS],
    lead: [f32; SECTIONS],
}

impl Net {
    /// Works out the design for `rate` (control thread: it allocates).
    fn new(rate: u32) -> Net {
        let c = iir_halfband(2 * SECTIONS, LOW_HZ / f64::from(rate.max(1)));
        let mut n = Net::default();
        for k in 0..SECTIONS {
            n.lead[k] = c[2 * k];
            n.lag[k] = c[2 * k + 1];
        }
        n
    }
}

/// Each side's share of the up and the down shift in `mode`.
fn mode_shares(mode: usize) -> [[f32; 2]; 2] {
    use std::f32::consts::FRAC_1_SQRT_2 as H;
    match mode {
        1 => [[0.0, 1.0]; 2],
        2 => [[H, H]; 2],
        3 => [[1.0, 0.0], [0.0, 1.0]],
        _ => [[1.0, 0.0]; 2],
    }
}

/// A single-sideband frequency shifter with feedback.
pub struct FreqShifter {
    v: [f32; 4],
    rate: u32,
    net: Net,
    quad: [Quad; 2],
    /// Each side's share of the up and the down shift, heard, and the
    /// first of the two one-poles that glide it to the Mode's after a
    /// change (so a change mid-way carries on from where it is).
    shares: [[f32; 2]; 2],
    pull: [[f32; 2]; 2],
    fading: bool,
    share_k: f32,
    /// The phasor's phase in cycles. Double precision so a shift of a
    /// hundredth of a hertz still turns smoothly.
    phase: f64,
    /// The shift as log2 Hz, so a sweep glides evenly in pitch.
    pitch: Smooth,
    feedback: Smooth,
    mix: Smooth,
    /// Each side's last output, for the feedback, kept between 20 Hz and
    /// 18 kHz by a one-pole high-pass and a one-pole low-pass with a zero
    /// at Nyquist (each side's last inputs to them, and their
    /// coefficients). At 0 Hz and at Nyquist the two chains are in or out
    /// of phase, not 90 degrees apart, so a shift folds a sound there back
    /// onto itself up to 3 dB louder; with high Feedback it would build up
    /// and ring on (FX quality Z6).
    last: [f32; 2],
    hp_x1: [f32; 2],
    lp_x1: [f32; 2],
    hp_r: f32,
    lp_b: f32,
    lp_a: f32,
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
            net: Net::default(),
            quad: [Quad::default(); 2],
            shares: mode_shares(v[1] as usize),
            pull: mode_shares(v[1] as usize),
            fading: false,
            share_k: 1.0,
            phase: 0.0,
            pitch: Smooth::new(v[0].log2()),
            feedback: Smooth::new(v[2] / 100.0),
            mix: Smooth::new(v[3] / 100.0),
            last: [0.0; 2],
            hp_x1: [0.0; 2],
            lp_x1: [0.0; 2],
            hp_r: 0.0,
            lp_b: 1.0,
            lp_a: 0.0,
        };
        f.setup(48_000);
        f
    }
    fn apply(&mut self, id: u16, v: f32) {
        match id {
            ids::SHIFT => self.pitch.target = v.log2(),
            ids::FEEDBACK => self.feedback.target = v / 100.0,
            ids::MIX => self.mix.target = v / 100.0,
            ids::MODE => self.fading = true,
            _ => {}
        }
    }
    fn begin(&mut self, _ctx: &Ctx) {}
    /// A frame of a Mode change: the shares a step toward the Mode's,
    /// through two one-poles, until they are there.
    #[cold]
    #[inline(never)]
    fn fade_shares(&mut self) {
        let to = mode_shares(self.v[1] as usize);
        let c = self.share_k;
        let mut gap = 0.0f32;
        for ch in 0..2 {
            for k in 0..2 {
                let p = &mut self.pull[ch][k];
                *p += (to[ch][k] - *p) * c;
                let w = &mut self.shares[ch][k];
                *w += (*p - *w) * c;
                gap = gap.max((to[ch][k] - *w).abs());
            }
        }
        if gap < 1e-5 {
            self.shares = to;
            self.pull = to;
            self.fading = false;
        }
    }
    fn run(&mut self, _ctx: &Ctx, _from: usize, l: &mut [f32], r: &mut [f32]) {
        let per = 1.0 / self.rate as f64;
        // The Shift still: the phasor's step is worked out once.
        let still = self.pitch.settled();
        let still_dt = fast_exp2(self.pitch.value) as f64 * per;
        // No Feedback (the default): the loop is skipped. Whenever a
        // frame's Feedback is 0 its states are left at 0, so the samples
        // are the same either way.
        if self.feedback.settled() && self.feedback.value == 0.0 {
            for i in 0..l.len() {
                let (c, s) = self.turn(still, still_dt, per);
                let mix = self.mix.glide();
                for (ch, x) in [&mut l[i], &mut r[i]].into_iter().enumerate() {
                    let dry = *x;
                    let (re, im) = self.quad[ch].run(&self.net, dry);
                    let [a, b] = self.shares[ch];
                    let y = a * (re * c - im * s) + b * (re * c + im * s);
                    *x = lerp(dry, y, mix);
                }
            }
            return;
        }
        for i in 0..l.len() {
            let (c, s) = self.turn(still, still_dt, per);
            let fb = self.feedback.glide();
            // Each pass round the loop lands on new frequencies, so the
            // passes add in power: this keeps the level where it was.
            let comp = (1.0 - fb * fb).sqrt();
            let mix = self.mix.glide();
            for (ch, x) in [&mut l[i], &mut r[i]].into_iter().enumerate() {
                let dry = *x;
                let into = if fb != 0.0 {
                    dry + fb * self.last[ch]
                } else {
                    dry
                };
                let (re, im) = self.quad[ch].run(&self.net, into);
                let [a, b] = self.shares[ch];
                let y = a * (re * c - im * s) + b * (re * c + im * s);
                if fb != 0.0 {
                    // Soft in the loop, so high feedback can't run away.
                    let t = fast_tanh(y);
                    let h = flush(t - self.hp_x1[ch] + self.hp_r * self.lp_x1[ch]);
                    self.hp_x1[ch] = t;
                    self.last[ch] =
                        flush(self.lp_b * (h + self.lp_x1[ch]) + self.lp_a * self.last[ch]);
                    self.lp_x1[ch] = h;
                } else {
                    self.last[ch] = 0.0;
                    self.hp_x1[ch] = 0.0;
                    self.lp_x1[ch] = 0.0;
                }
                *x = lerp(dry, y * comp, mix);
            }
        }
    }
    /// A frame of the phasor: its phase a step on (the still step, or the
    /// Shift's glided), a frame of any Mode change, and its cosine and sine.
    #[inline(always)]
    fn turn(&mut self, still: bool, still_dt: f64, per: f64) -> (f32, f32) {
        self.phase += if still {
            still_dt
        } else {
            fast_exp2(self.pitch.glide()) as f64 * per
        };
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        if self.fading {
            self.fade_shares();
        }
        let p = self.phase as f32;
        (kit::cosine_fine(p), kit::sine_fine(p))
    }
    fn setup(&mut self, rate: u32) {
        self.rate = rate;
        self.net = Net::new(rate);
        // The feedback's band: a high-pass at 20 Hz, and a bilinear low-pass
        // at 18 kHz (or just under Nyquist at low rates).
        let fs = rate.max(1) as f32;
        self.hp_r = (-std::f32::consts::TAU * 20.0 / fs).exp();
        let k = (std::f32::consts::PI * (18_000.0f32).min(0.4 * fs) / fs).tan();
        self.lp_b = k / (1.0 + k);
        self.lp_a = (1.0 - k) / (1.0 + k);
        // Each one-pole 2.5 ms: the Mode change glides over about 20 ms.
        self.share_k = 1.0 - (-1.0 / (0.0025 * rate as f32).max(1.0)).exp();
        self.pitch.set_time(rate, 30.0);
        self.feedback.set_time(rate, 20.0);
        self.mix.set_time(rate, 20.0);
    }
    fn clear(&mut self) {
        self.quad = [Quad::default(); 2];
        self.shares = mode_shares(self.v[1] as usize);
        self.pull = self.shares;
        self.fading = false;
        self.phase = 0.0;
        self.last = [0.0; 2];
        self.hp_x1 = [0.0; 2];
        self.lp_x1 = [0.0; 2];
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

    /// The level at `hz` in `x` at `rate`, by a single-bin DFT (whole
    /// cycles when `x` is a second long and `hz` whole).
    fn level_at_rate(x: &[f32], hz: f64, rate: u32) -> f64 {
        let (mut re, mut im) = (0.0f64, 0.0f64);
        for (i, v) in x.iter().enumerate() {
            let w = std::f64::consts::TAU * hz * i as f64 / f64::from(rate);
            re += f64::from(*v) * w.cos();
            im += f64::from(*v) * w.sin();
        }
        2.0 * (re * re + im * im).sqrt() / x.len() as f64
    }

    /// Runs `input` through `f` (both sides the same) a block at a time
    /// and returns the left side.
    fn render(f: &mut FreqShifter, rate: u32, input: &[f32]) -> Vec<f32> {
        let mut l = input.to_vec();
        let mut r = input.to_vec();
        let ctx = Ctx::new(rate, 0);
        for k in (0..l.len()).step_by(128) {
            let e = (k + 128).min(l.len());
            f.process(&ctx, &[], &mut l[k..e], &mut r[k..e]);
        }
        l
    }

    #[test]
    fn the_image_is_60_db_down_from_40_hz_to_15_khz_at_either_rate() {
        for rate in [44_100, 48_000] {
            for hz in [40.0, 1000.0, 15_000.0] {
                let mut f = FreqShifter::new();
                f.set_param(ids::SHIFT, 100.0);
                f.prepare(rate, 128);
                f.reset();
                let n = 2 * rate as usize;
                let x: Vec<f32> = (0..n)
                    .map(|i| {
                        let w = std::f64::consts::TAU * hz * i as f64 / f64::from(rate);
                        (0.5 * w.sin()) as f32
                    })
                    .collect();
                let y = render(&mut f, rate, &x);
                let last = &y[rate as usize..];
                let want = level_at_rate(last, hz + 100.0, rate);
                let image = level_at_rate(last, (hz - 100.0).abs(), rate);
                assert!((want - 0.5).abs() < 0.01, "{rate} {hz}: {want}");
                assert!(image < 0.001 * want, "{rate} {hz}: image {image}");
            }
        }
    }

    #[test]
    fn a_mode_change_glides() {
        let rate = 48_000;
        let mut f = FreqShifter::new();
        f.prepare(rate, 128);
        f.reset();
        let ctx = Ctx::new(rate, 0);
        let (mut l, mut r) = ([0.1f32; 1], [0.1f32; 1]);
        f.set_param(ids::MODE, 1.0);
        f.process(&ctx, &[], &mut l, &mut r);
        // A frame on, still nearly all Up; 60 ms on, all Down.
        assert!(f.shares[0][0] > 0.99 && f.fading, "{:?}", f.shares);
        let (mut l, mut r) = (vec![0.1f32; 2880], vec![0.1f32; 2880]);
        f.process(&ctx, &[], &mut l, &mut r);
        assert_eq!(f.shares, mode_shares(1));
        assert!(!f.fading);
    }

    #[test]
    fn high_feedback_rings_out_at_any_shift() {
        // Before the loop was kept under 18 kHz, a 100 Hz shift at
        // 44.1 kHz rang on near Nyquist for ever.
        let rate = 44_100;
        for shift in [5.0, 100.0, 2000.0] {
            let mut f = FreqShifter::new();
            f.set_param(ids::SHIFT, shift);
            f.set_param(ids::FEEDBACK, 95.0);
            f.prepare(rate, 128);
            f.reset();
            let mut seed = 1u32;
            let n = 3 * rate as usize;
            let x: Vec<f32> = (0..n)
                .map(|i| {
                    seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    if i < rate as usize / 2 {
                        0.5 * ((seed >> 8) as f32 / 8_388_608.0 - 1.0)
                    } else {
                        0.0
                    }
                })
                .collect();
            let y = render(&mut f, rate, &x);
            let tail = y[n - rate as usize / 4..]
                .iter()
                .fold(0.0f32, |m, v| m.max(v.abs()));
            assert!(tail < 1e-4, "shift {shift}: tail {tail}");
        }
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
