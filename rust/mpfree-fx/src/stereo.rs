//! Two stereo tools: [`StereoWidth`] and [`SampleDelay`].
//!
//! - **Stereo width** (the MPC's AIR Stereo Width) makes the stereo image
//!   narrower or wider, as a whole and in three bands, so the low end can
//!   go mono while the top opens out. It works on the side signal (the
//!   difference between left and right): Width scales it, and a
//!   Linkwitz-Riley split gives the low, mid and high bands their own
//!   width. The mid signal goes through the all-passes the split sums to,
//!   so with every width at 100 % the sound comes out as it went in
//!   (only its phase turned, the same on both sides). Delay widens a mono
//!   sound: it adds a delayed copy of the mid signal to the side, which
//!   cancels in a mono sum, so unlike a plain one-sided (Haas) delay it
//!   leaves no comb in mono. Pan and Output trim the result.
//! - **Sample delay** (the MPC's Sample Delay) delays the left and the
//!   right side by whole samples, up to 4800 (100 ms at 48 kHz), and can
//!   flip either side's polarity: for lining up two microphones, fixing
//!   a sample whose sides are out of time, or a hard Haas spread. A
//!   change of delay crossfades from the old point to the new over
//!   10 ms, so moving it doesn't click.

use mpfree_node::{Ctx, Curve, Kind, NodeInfo, ParamInfo, Smooth};

use crate::kit::{
    node_impl, param, Crossover, CrossoverPhase, Db, Glide, Hz, Line, Linear, Log, Ms, Percent,
    Toggle, NONE,
};
use crate::{defaults, fast_db_to_gain, Preset};

// ------------------------------------------------------------ stereo width

/// [`StereoWidth`] parameter ids.
pub mod width_ids {
    pub const WIDTH: u16 = 0;
    pub const LOW: u16 = 1;
    pub const MID: u16 = 2;
    pub const HIGH: u16 = 3;
    pub const LOW_SPLIT: u16 = 4;
    pub const HIGH_SPLIT: u16 = 5;
    pub const DELAY: u16 = 6;
    pub const PAN: u16 = 7;
    pub const OUTPUT: u16 = 8;
}
use width_ids as w;

static WIDTH_INFO: NodeInfo = NodeInfo {
    id: "mpfree.fx.width",
    name: "Stereo width",
    kind: Kind::Effect,
};
static WIDTH_PARAMS: [ParamInfo; 9] = [
    param(0, "Width", "Width", 0.0, 200.0, 100.0, Percent, Linear),
    param(1, "Low width", "Bands", 0.0, 200.0, 100.0, Percent, Linear),
    param(2, "Mid width", "Bands", 0.0, 200.0, 100.0, Percent, Linear),
    param(3, "High width", "Bands", 0.0, 200.0, 100.0, Percent, Linear),
    param(4, "Low split", "Bands", 40.0, 800.0, 200.0, Hz, Log),
    param(5, "High split", "Bands", 1000.0, 10000.0, 4000.0, Hz, Log),
    param(6, "Delay", "Width", 0.0, 8.0, 0.0, Ms, Linear),
    param(7, "Pan", "Output", -100.0, 100.0, 0.0, Percent, Linear),
    param(8, "Output", "Output", -24.0, 12.0, 0.0, Db, Linear),
];

/// Stereo width factory presets.
pub static WIDTH_PRESETS: &[Preset] = &[
    Preset {
        name: "Mono bass",
        values: &[(w::LOW, 0.0), (w::LOW_SPLIT, 150.0)],
    },
    Preset {
        name: "Wide top",
        values: &[(w::LOW, 0.0), (w::MID, 110.0), (w::HIGH, 160.0)],
    },
    Preset {
        name: "Mono",
        values: &[(w::WIDTH, 0.0)],
    },
    Preset {
        name: "Widen mono",
        values: &[(w::DELAY, 4.0), (w::LOW, 0.0)],
    },
    Preset {
        name: "Extra wide",
        values: &[(w::WIDTH, 170.0), (w::LOW, 40.0), (w::OUTPUT, -2.0)],
    },
];

/// How much delayed mid Delay adds to the side at full strength.
const SPREAD: f32 = 0.5;
/// Delay fades in over its first millisecond (a delay of nearly nothing
/// would only pan the sound).
const SPREAD_RAMP_MS: f32 = 1.0;
/// The longest Delay, in ms.
const MAX_DELAY_MS: f32 = 8.0;

/// Mid and side widths in three bands, a mono-safe spread and a trim.
pub struct StereoWidth {
    v: [f32; 9],
    rate: u32,
    /// Side splits: at the low split, then the rest at the high split.
    low_x: Crossover,
    high_x: Crossover,
    /// Keeps the side's low band in phase with the parts that went
    /// through the high split.
    low_phase: CrossoverPhase,
    /// The mid signal's all-passes, matching the side's.
    mid_low: CrossoverPhase,
    mid_high: CrossoverPhase,
    line: Line,
    width: Smooth,
    bands: [Smooth; 3],
    delay: Smooth,
    spread: Smooth,
    gains: [Smooth; 2],
}

impl Default for StereoWidth {
    fn default() -> Self {
        Self::new()
    }
}

impl StereoWidth {
    pub fn new() -> StereoWidth {
        let v = defaults(&WIDTH_PARAMS);
        let mut s = StereoWidth {
            v,
            rate: 0,
            low_x: Crossover::default(),
            high_x: Crossover::default(),
            low_phase: CrossoverPhase::default(),
            mid_low: CrossoverPhase::default(),
            mid_high: CrossoverPhase::default(),
            line: Line::default(),
            width: Smooth::new(1.0),
            bands: [Smooth::new(1.0); 3],
            delay: Smooth::new(0.0),
            spread: Smooth::new(0.0),
            gains: [Smooth::new(1.0); 2],
        };
        s.setup(48_000);
        s.clear();
        s
    }
    fn apply(&mut self, id: u16, v: f32) {
        match id {
            w::WIDTH => self.width.target = v / 100.0,
            w::LOW | w::MID | w::HIGH => {
                self.bands[(id - w::LOW) as usize].target = v / 100.0;
            }
            w::LOW_SPLIT | w::HIGH_SPLIT => self.tune(),
            w::DELAY => {
                self.delay.target = v * 0.001 * self.rate as f32;
                self.spread.target = SPREAD * (v / SPREAD_RAMP_MS).min(1.0);
            }
            w::PAN | w::OUTPUT => self.set_gains(),
            _ => {}
        }
    }
    fn tune(&mut self) {
        let (lo, hi) = (self.v[4], self.v[5]);
        self.low_x.set(lo, self.rate);
        self.mid_low.set(lo, self.rate);
        self.high_x.set(hi, self.rate);
        self.low_phase.set(hi, self.rate);
        self.mid_high.set(hi, self.rate);
    }
    fn set_gains(&mut self) {
        // A balance control: the far side fades, the near side stays.
        let pan = self.v[7] / 100.0;
        let out = fast_db_to_gain(self.v[8]);
        self.gains[0].target = out * (1.0 - pan).min(1.0);
        self.gains[1].target = out * (1.0 + pan).min(1.0);
    }
    fn begin(&mut self, _ctx: &Ctx) {}
    fn run(&mut self, _ctx: &Ctx, _from: usize, l: &mut [f32], r: &mut [f32]) {
        for i in 0..l.len() {
            let mid = 0.5 * (l[i] + r[i]);
            let side = 0.5 * (l[i] - r[i]);
            // The side in three bands.
            let (low, rest) = self.low_x.split(0, side);
            let low = self.low_phase.run(0, low);
            let (midb, high) = self.high_x.split(0, rest);
            let (lw, mw, hw) = (
                self.bands[0].glide(),
                self.bands[1].glide(),
                self.bands[2].glide(),
            );
            let side = lw * low + mw * midb + hw * high;
            // The mid through the same all-passes.
            let m = self.mid_high.run(0, self.mid_low.run(0, mid));
            // The spread: delayed mid into the side.
            self.line.push(m);
            let d = self.delay.glide();
            let spread = self.spread.glide();
            let wide = if spread > 0.0 {
                side + spread * self.line.read(d)
            } else {
                side
            };
            let s = self.width.glide() * wide;
            l[i] = self.gains[0].glide() * (m + s);
            r[i] = self.gains[1].glide() * (m - s);
        }
    }
    fn setup(&mut self, rate: u32) {
        if rate != self.rate {
            self.line = Line::new((MAX_DELAY_MS * 0.001 * rate as f32) as usize + 4);
        }
        self.rate = rate;
        for s in [&mut self.width, &mut self.delay, &mut self.spread]
            .into_iter()
            .chain(self.bands.iter_mut())
            .chain(self.gains.iter_mut())
        {
            s.set_time(rate, 20.0);
        }
        self.tune();
        self.apply(w::DELAY, self.v[6]);
        self.set_gains();
    }
    fn clear(&mut self) {
        self.low_x.reset();
        self.high_x.reset();
        self.low_phase.reset();
        self.mid_low.reset();
        self.mid_high.reset();
        self.line.clear();
        for s in [&mut self.width, &mut self.delay, &mut self.spread]
            .into_iter()
            .chain(self.bands.iter_mut())
            .chain(self.gains.iter_mut())
        {
            s.snap(s.target);
        }
    }
    fn tail(&self) -> u32 {
        (MAX_DELAY_MS * 0.001 * self.rate as f32) as u32 + self.rate / 100
    }
    fn lat(&self) -> u32 {
        0
    }
}

node_impl!(StereoWidth, WIDTH_INFO, WIDTH_PARAMS);

// ------------------------------------------------------------ sample delay

/// [`SampleDelay`] parameter ids.
pub mod sample_delay_ids {
    pub const LEFT: u16 = 0;
    pub const RIGHT: u16 = 1;
    pub const FLIP_LEFT: u16 = 2;
    pub const FLIP_RIGHT: u16 = 3;
}
use sample_delay_ids as sd;

/// The longest delay, in samples.
pub const MAX_SAMPLES: usize = 4800;

static DELAY_INFO: NodeInfo = NodeInfo {
    id: "mpfree.fx.sample_delay",
    name: "Sample delay",
    kind: Kind::Effect,
};
static DELAY_PARAMS: [ParamInfo; 4] = [
    param(
        0,
        "Left",
        "Samples",
        0.0,
        MAX_SAMPLES as f32,
        0.0,
        NONE,
        Curve::Stepped,
    ),
    param(
        1,
        "Right",
        "Samples",
        0.0,
        MAX_SAMPLES as f32,
        0.0,
        NONE,
        Curve::Stepped,
    ),
    param(2, "Flip left", "Polarity", 0.0, 1.0, 0.0, NONE, Toggle),
    param(3, "Flip right", "Polarity", 0.0, 1.0, 0.0, NONE, Toggle),
];

/// Sample delay factory presets.
pub static DELAY_PRESETS: &[Preset] = &[
    Preset {
        name: "Haas right",
        values: &[(sd::RIGHT, 720.0)],
    },
    Preset {
        name: "Haas left",
        values: &[(sd::LEFT, 720.0)],
    },
    Preset {
        name: "Flip one side",
        values: &[(sd::FLIP_RIGHT, 1.0)],
    },
];

/// Crossfade time when a delay changes, in ms.
const FADE_MS: f32 = 10.0;

/// One side's delay: the point it reads, and a fade to a new one.
#[derive(Clone, Copy, Debug, Default)]
struct Tap {
    now: usize,
    next: usize,
    /// How far through the fade to `next` (0 when not fading).
    t: f32,
}

/// Delays each side by whole samples and flips polarity.
pub struct SampleDelay {
    v: [f32; 4],
    rate: u32,
    lines: [Line; 2],
    taps: [Tap; 2],
    /// Fade step per frame.
    step: f32,
    /// Each side's polarity, gliding between 1 and -1.
    sign: [Smooth; 2],
}

impl Default for SampleDelay {
    fn default() -> Self {
        Self::new()
    }
}

impl SampleDelay {
    pub fn new() -> SampleDelay {
        let mut s = SampleDelay {
            v: defaults(&DELAY_PARAMS),
            rate: 48_000,
            lines: [Line::new(MAX_SAMPLES + 2), Line::new(MAX_SAMPLES + 2)],
            taps: [Tap::default(); 2],
            step: 0.0,
            sign: [Smooth::new(1.0); 2],
        };
        s.setup(48_000);
        s
    }
    fn apply(&mut self, id: u16, v: f32) {
        match id {
            sd::FLIP_LEFT | sd::FLIP_RIGHT => {
                self.sign[(id - sd::FLIP_LEFT) as usize].target = if v >= 0.5 { -1.0 } else { 1.0 };
            }
            _ => {}
        }
    }
    fn begin(&mut self, _ctx: &Ctx) {}
    fn run(&mut self, _ctx: &Ctx, _from: usize, l: &mut [f32], r: &mut [f32]) {
        let want = [self.v[0] as usize, self.v[1] as usize];
        for i in 0..l.len() {
            for (ch, x) in [&mut l[i], &mut r[i]].into_iter().enumerate() {
                let line = &mut self.lines[ch];
                line.push(*x);
                let tap = &mut self.taps[ch];
                if tap.t == 0.0 && tap.now != want[ch] {
                    tap.next = want[ch];
                    tap.t = self.step;
                }
                let y = if tap.t > 0.0 {
                    let a = line.tap(tap.now + 1);
                    let b = line.tap(tap.next + 1);
                    let y = a + (b - a) * tap.t;
                    tap.t += self.step;
                    if tap.t >= 1.0 {
                        tap.now = tap.next;
                        tap.t = 0.0;
                    }
                    y
                } else {
                    line.tap(tap.now + 1)
                };
                *x = self.sign[ch].glide() * y;
            }
        }
    }
    fn setup(&mut self, rate: u32) {
        self.rate = rate;
        self.step = 1.0 / (FADE_MS * 0.001 * rate as f32).max(1.0);
        for s in &mut self.sign {
            s.set_time(rate, 2.0);
        }
    }
    fn clear(&mut self) {
        for l in &mut self.lines {
            l.clear();
        }
        let want = [self.v[0] as usize, self.v[1] as usize];
        for (t, &d) in self.taps.iter_mut().zip(want.iter()) {
            *t = Tap {
                now: d,
                next: d,
                t: 0.0,
            };
        }
        for s in &mut self.sign {
            s.snap(s.target);
        }
    }
    fn tail(&self) -> u32 {
        self.v[0].max(self.v[1]) as u32
    }
    fn lat(&self) -> u32 {
        0
    }
}

node_impl!(SampleDelay, DELAY_INFO, DELAY_PARAMS);

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_node::Node;

    fn run(fx: &mut dyn Node, l: &mut [f32], r: &mut [f32]) {
        let ctx = Ctx::new(48_000, 0);
        let n = l.len();
        for k in (0..n).step_by(64) {
            let e = (k + 64).min(n);
            fx.process(&ctx, &[], &mut l[k..e], &mut r[k..e]);
        }
    }

    fn noise(n: usize, seed: u32) -> Vec<f32> {
        let mut s = seed;
        (0..n)
            .map(|_| {
                s ^= s << 13;
                s ^= s >> 17;
                s ^= s << 5;
                (s as f32 / u32::MAX as f32) - 0.5
            })
            .collect()
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
    }

    #[test]
    fn width_zero_is_mono_and_full_width_keeps_the_sides() {
        let mut s = StereoWidth::new();
        s.prepare(48_000, 64);
        let (mut l, mut r) = (noise(24_000, 1), noise(24_000, 2));
        s.set_param(w::WIDTH, 0.0);
        s.reset();
        run(&mut s, &mut l, &mut r);
        let diff: Vec<f32> = l.iter().zip(&r).map(|(a, b)| a - b).collect();
        assert!(rms(&diff[4800..]) < 1e-4, "{}", rms(&diff[4800..]));

        // At 100 % everywhere, the side and the mid keep their levels
        // (only their phase turns, the same for both).
        let (mut l, mut r) = (noise(24_000, 1), noise(24_000, 2));
        let side_in: Vec<f32> = l.iter().zip(&r).map(|(a, b)| a - b).collect();
        s.set_param(w::WIDTH, 100.0);
        s.reset();
        run(&mut s, &mut l, &mut r);
        let side_out: Vec<f32> = l.iter().zip(&r).map(|(a, b)| a - b).collect();
        let ratio = rms(&side_out[4800..]) / rms(&side_in[4800..]);
        assert!((ratio - 1.0).abs() < 0.03, "{ratio}");
    }

    #[test]
    fn low_width_zero_makes_the_bass_mono() {
        let mut s = StereoWidth::new();
        s.prepare(48_000, 64);
        s.set_param(w::LOW, 0.0);
        s.reset();
        // A 60 Hz tone on the left only: its side part goes, so both
        // sides end up equal.
        let n = 48_000;
        let mut l: Vec<f32> = (0..n)
            .map(|i| 0.5 * (i as f32 * 60.0 / 48_000.0 * std::f32::consts::TAU).sin())
            .collect();
        let mut r = vec![0.0; n];
        run(&mut s, &mut l, &mut r);
        let diff: Vec<f32> = l.iter().zip(&r).map(|(a, b)| a - b).collect();
        assert!(rms(&diff[24_000..]) < 0.1 * rms(&l[24_000..]));
        // A 10 kHz tone keeps its side.
        let mut l: Vec<f32> = (0..n)
            .map(|i| 0.5 * (i as f32 * 10_000.0 / 48_000.0 * std::f32::consts::TAU).sin())
            .collect();
        let mut r = vec![0.0; n];
        s.reset();
        run(&mut s, &mut l, &mut r);
        assert!(rms(&r[24_000..]) < 0.05 * rms(&l[24_000..]));
    }

    #[test]
    fn delay_widens_mono_and_cancels_in_a_mono_sum() {
        let mut s = StereoWidth::new();
        s.prepare(48_000, 64);
        s.set_param(w::DELAY, 5.0);
        s.reset();
        let mono = noise(24_000, 3);
        let (mut l, mut r) = (mono.clone(), mono.clone());
        run(&mut s, &mut l, &mut r);
        let diff: Vec<f32> = l.iter().zip(&r).map(|(a, b)| a - b).collect();
        let sum: Vec<f32> = l.iter().zip(&r).map(|(a, b)| 0.5 * (a + b)).collect();
        assert!(rms(&diff[4800..]) > 0.5 * rms(&mono[4800..]));
        // The mono sum keeps its level (the mid's all-passes only turn
        // its phase).
        let ratio = rms(&sum[4800..]) / rms(&mono[4800..]);
        assert!((ratio - 1.0).abs() < 0.03, "{ratio}");
    }

    #[test]
    fn sample_delay_delays_and_flips() {
        let mut d = SampleDelay::new();
        d.prepare(48_000, 64);
        d.set_param(sd::LEFT, 10.0);
        d.set_param(sd::FLIP_RIGHT, 1.0);
        d.reset();
        let mut l = vec![0.0; 256];
        let mut r = vec![0.0; 256];
        l[0] = 1.0;
        r[0] = 1.0;
        run(&mut d, &mut l, &mut r);
        assert_eq!(l[10], 1.0);
        assert!(l.iter().enumerate().all(|(i, v)| i == 10 || *v == 0.0));
        assert_eq!(r[0], -1.0);
    }

    #[test]
    fn a_new_delay_fades_in_without_a_jump() {
        let mut d = SampleDelay::new();
        d.prepare(48_000, 64);
        d.reset();
        let n = 9600;
        let tone: Vec<f32> = (0..n)
            .map(|i| 0.5 * (i as f32 * 220.0 / 48_000.0 * std::f32::consts::TAU).sin())
            .collect();
        let (mut l, mut r) = (tone.clone(), tone.clone());
        let ctx = Ctx::new(48_000, 0);
        d.process(&ctx, &[], &mut l[..4800], &mut r[..4800]);
        d.set_param(sd::LEFT, 2400.0);
        d.process(&ctx, &[], &mut l[4800..], &mut r[4800..]);
        // No step bigger than the tone's own slope allows, plus a little.
        let slope = 0.5 * 220.0 / 48_000.0 * std::f32::consts::TAU;
        for i in 1..n {
            assert!((l[i] - l[i - 1]).abs() < 4.0 * slope, "{i}");
        }
    }
}
