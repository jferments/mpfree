//! The [`Granulator`]: cuts the incoming sound into short grains and plays
//! them back as a cloud: pitched, scattered in time, reversed, spread
//! across the stereo field, or frozen so a moment hangs in the air (the
//! MPC's Granulator idea).
//!
//! The sound goes into a four-second stereo buffer. A new grain starts
//! Density times a second (with a little random spacing, so the cloud
//! doesn't buzz at the grain rate); each one reads the buffer from Delay
//! back, moved by up to Jitter of its own size, at Pitch (plus a random
//! Detune), forwards or (by the Reverse chance) backwards, under a Hann
//! window, panned at random within Width. Feedback writes the cloud back
//! into the buffer for trails that build; Freeze stops writing, so the
//! grains keep playing what's there.
//!
//! Up to [`MAX_GRAINS`] play at once (a new grain waits for a free one),
//! and the level is scaled by how many overlap, so a dense cloud is no
//! louder than a sparse one. Grains read with cubic interpolation, and
//! every read point is kept inside the buffer for the grain's whole
//! life, whatever the pitch and direction, so a grain never reads across
//! the write point. The random numbers are seeded on reset, so a render
//! is the same every time.

use mpfree_node::{Ctx, Kind, NodeInfo, ParamInfo, Smooth, Unit};

use crate::kit::{
    self, node_impl, param, Choice, Glide, Line, Linear, Log, Ms, Percent, Toggle, NONE,
};
use crate::{defaults, fast_exp2, fast_tanh, lerp, Preset};

/// [`Granulator`] parameter ids.
pub mod ids {
    pub const SIZE: u16 = 0;
    pub const DENSITY: u16 = 1;
    pub const PITCH: u16 = 2;
    pub const DETUNE: u16 = 3;
    pub const DELAY: u16 = 4;
    pub const SYNC: u16 = 5;
    pub const JITTER: u16 = 6;
    pub const REVERSE: u16 = 7;
    pub const FEEDBACK: u16 = 8;
    pub const FREEZE: u16 = 9;
    pub const WIDTH: u16 = 10;
    pub const MIX: u16 = 11;
}

/// Grains that can play at once.
pub const MAX_GRAINS: usize = 12;
/// The buffer's length in seconds.
const BUFFER_SECONDS: f32 = 4.0;
/// The longest Delay, in ms (a synced delay is held to it too).
const MAX_DELAY_MS: f32 = 2000.0;
/// Frames kept clear of the buffer's ends for the cubic's neighbours.
const EDGE: f32 = 4.0;

static INFO: NodeInfo = NodeInfo {
    id: "mpfree.fx.granulator",
    name: "Granulator",
    kind: Kind::Effect,
};
static PARAMS: [ParamInfo; 12] = [
    param(0, "Size", "Grains", 10.0, 500.0, 80.0, Ms, Log),
    param(1, "Density", "Grains", 1.0, 80.0, 16.0, Unit::Hz, Log),
    param(
        2,
        "Pitch",
        "Pitch",
        -24.0,
        24.0,
        0.0,
        Unit::Semitones,
        Linear,
    ),
    param(3, "Detune", "Pitch", 0.0, 100.0, 0.0, Unit::Cents, Linear),
    param(4, "Delay", "Time", 0.0, MAX_DELAY_MS, 100.0, Ms, Linear),
    param(5, "Sync", "Time", 0.0, 13.0, 0.0, NONE, Choice(kit::SYNCS)),
    param(6, "Jitter", "Time", 0.0, 100.0, 20.0, Percent, Linear),
    param(7, "Reverse", "Grains", 0.0, 100.0, 0.0, Percent, Linear),
    param(8, "Feedback", "Time", 0.0, 90.0, 0.0, Percent, Linear),
    param(9, "Freeze", "Time", 0.0, 1.0, 0.0, NONE, Toggle),
    param(10, "Width", "Grains", 0.0, 100.0, 50.0, Percent, Linear),
    param(11, "Mix", "Grains", 0.0, 100.0, 50.0, Percent, Linear),
];

/// Granulator factory presets.
pub static PRESETS: &[Preset] = &[
    Preset {
        name: "Shimmer cloud",
        values: &[
            (ids::SIZE, 180.0),
            (ids::DENSITY, 24.0),
            (ids::PITCH, 12.0),
            (ids::DETUNE, 8.0),
            (ids::JITTER, 60.0),
            (ids::FEEDBACK, 45.0),
            (ids::WIDTH, 90.0),
        ],
    },
    Preset {
        name: "Stutter dust",
        values: &[
            (ids::SIZE, 25.0),
            (ids::DENSITY, 40.0),
            (ids::JITTER, 100.0),
            (ids::REVERSE, 30.0),
            (ids::WIDTH, 100.0),
        ],
    },
    Preset {
        name: "Octave down haze",
        values: &[
            (ids::SIZE, 250.0),
            (ids::DENSITY, 10.0),
            (ids::PITCH, -12.0),
            (ids::DETUNE, 15.0),
            (ids::JITTER, 40.0),
            (ids::MIX, 60.0),
        ],
    },
    Preset {
        name: "Reverse swell",
        values: &[
            (ids::SIZE, 400.0),
            (ids::DENSITY, 6.0),
            (ids::REVERSE, 100.0),
            (ids::DELAY, 500.0),
            (ids::FEEDBACK, 30.0),
        ],
    },
    Preset {
        name: "Freeze pad",
        values: &[
            (ids::SIZE, 300.0),
            (ids::DENSITY, 20.0),
            (ids::JITTER, 100.0),
            (ids::DETUNE, 10.0),
            (ids::FREEZE, 1.0),
            (ids::MIX, 100.0),
        ],
    },
];

/// One playing grain.
#[derive(Clone, Copy, Debug, Default)]
struct Grain {
    live: bool,
    /// The read point now, as frames back from the write point.
    d: f32,
    /// How the read point moves each frame (the buffer moves on by one
    /// while the grain reads at its speed, forwards or back).
    drift: f32,
    /// The window's phase, 0 to 1, and its step per frame.
    phase: f32,
    step: f32,
    /// Its left and right gains.
    gl: f32,
    gr: f32,
}

/// A granular cloud with pitch, jitter, reverse, feedback and freeze.
pub struct Granulator {
    v: [f32; 12],
    rate: u32,
    bpm: f64,
    lines: [Line; 2],
    grains: [Grain; MAX_GRAINS],
    /// Frames to the next grain.
    wait: f32,
    rng: u32,
    mix: Smooth,
    feedback: Smooth,
    /// The last frame's cloud, for the feedback.
    last: [f32; 2],
}

impl Default for Granulator {
    fn default() -> Self {
        Self::new()
    }
}

impl Granulator {
    pub fn new() -> Granulator {
        let v = defaults(&PARAMS);
        let mut g = Granulator {
            v,
            rate: 0,
            bpm: 120.0,
            lines: [Line::default(), Line::default()],
            grains: [Grain::default(); MAX_GRAINS],
            wait: 0.0,
            rng: 1,
            mix: Smooth::new(v[11] / 100.0),
            feedback: Smooth::new(v[8] / 100.0),
            last: [0.0; 2],
        };
        g.setup(48_000);
        g.clear();
        g
    }
    fn apply(&mut self, id: u16, v: f32) {
        match id {
            ids::MIX => self.mix.target = v / 100.0,
            ids::FEEDBACK => self.feedback.target = v / 100.0,
            _ => {}
        }
    }
    fn begin(&mut self, ctx: &Ctx) {
        if ctx.bpm > 0.0 {
            self.bpm = ctx.bpm;
        }
    }
    /// A random number from 0 to 1.
    #[inline]
    fn rand(&mut self) -> f32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x >> 8) as f32 / (1u32 << 24) as f32
    }
    /// The delay grains start from, in frames.
    fn delay_frames(&self) -> f32 {
        let sync = (self.v[5] as usize).min(kit::SYNC_BEATS.len() - 1);
        let ms = if sync > 0 {
            (kit::SYNC_BEATS[sync] * 60_000.0 / self.bpm.max(1.0)) as f32
        } else {
            self.v[4]
        };
        ms.min(MAX_DELAY_MS) * 0.001 * self.rate as f32
    }
    /// Starts a grain in a free slot, if there is one.
    fn spawn(&mut self) {
        let Some(slot) = self.grains.iter().position(|g| !g.live) else {
            return;
        };
        let len = (self.v[0] * 0.001 * self.rate as f32).max(16.0);
        let cents = (self.rand() * 2.0 - 1.0) * self.v[3];
        let speed = fast_exp2((self.v[2] + 0.01 * cents) / 12.0);
        let back = self.rand() * 100.0 < self.v[7];
        let dir = if back { -1.0 } else { 1.0 };
        let writing = if self.v[9] >= 0.5 { 0.0 } else { 1.0 };
        let drift = writing - dir * speed;
        // Where it starts: Delay back, moved by up to Jitter of its size.
        let jitter = self.rand() * self.v[6] / 100.0 * len;
        let want = self.delay_frames() + jitter;
        // Keep the read point inside the buffer from the grain's start to
        // its end.
        let top = self.lines[0].max();
        let lo = EDGE + (-drift * len).max(0.0);
        let hi = (top - EDGE - (drift * len).max(0.0)).max(lo);
        let d = want.clamp(lo, hi);
        // A random place within Width: an equal-power pan.
        let pan = (self.rand() * 2.0 - 1.0) * self.v[10] / 100.0;
        let a = 0.125 * (pan + 1.0);
        let (gl, gr) = (kit::cosine(a), kit::sine(a));
        self.grains[slot] = Grain {
            live: true,
            d,
            drift,
            phase: 0.0,
            step: 1.0 / len,
            gl: gl * std::f32::consts::SQRT_2,
            gr: gr * std::f32::consts::SQRT_2,
        };
    }
    fn run(&mut self, _ctx: &Ctx, _from: usize, l: &mut [f32], r: &mut [f32]) {
        let rate = self.rate as f32;
        let interval = rate / self.v[1];
        // How many grains overlap on average, and the level for it: two
        // Hann windows half over each other sum to one.
        let overlap = (self.v[0] * 0.001 * self.v[1]).min(MAX_GRAINS as f32);
        let norm = 1.0 / (0.5 * overlap).max(1.0).sqrt();
        let frozen = self.v[9] >= 0.5;
        for i in 0..l.len() {
            self.wait -= 1.0;
            if self.wait <= 0.0 {
                self.spawn();
                // A little random spacing (up to a quarter either way).
                self.wait += interval * (0.75 + 0.5 * self.rand());
            }
            let (mut wl, mut wr) = (0.0f32, 0.0f32);
            for g in self.grains.iter_mut() {
                if !g.live {
                    continue;
                }
                let w = 0.5 - 0.5 * kit::cosine(g.phase);
                wl += w * g.gl * self.lines[0].read(g.d);
                wr += w * g.gr * self.lines[1].read(g.d);
                g.d += g.drift;
                g.phase += g.step;
                if g.phase >= 1.0 {
                    g.live = false;
                }
            }
            wl *= norm;
            wr *= norm;
            let fb = self.feedback.glide();
            if !frozen {
                self.lines[0].push(l[i] + fb * self.last[0]);
                self.lines[1].push(r[i] + fb * self.last[1]);
            }
            self.last = [fast_tanh(wl), fast_tanh(wr)];
            let mix = self.mix.glide();
            l[i] = lerp(l[i], wl, mix);
            r[i] = lerp(r[i], wr, mix);
        }
    }
    fn setup(&mut self, rate: u32) {
        if rate != self.rate {
            let n = (BUFFER_SECONDS * rate as f32) as usize;
            self.lines = [Line::new(n), Line::new(n)];
        }
        self.rate = rate;
        self.mix.set_time(rate, 20.0);
        self.feedback.set_time(rate, 20.0);
    }
    fn clear(&mut self) {
        for l in &mut self.lines {
            l.clear();
        }
        self.grains = [Grain::default(); MAX_GRAINS];
        self.wait = 0.0;
        self.rng = 0x9E37_79B9;
        self.last = [0.0; 2];
        self.mix.snap(self.mix.target);
        self.feedback.snap(self.feedback.target);
    }
    fn tail(&self) -> u32 {
        // The farthest a grain reads back plus its length, longer with
        // feedback.
        let reach = (MAX_DELAY_MS + 2.0 * self.v[0]) * 0.001 * self.rate as f32;
        if self.v[8] > 0.0 {
            reach as u32 + 4 * self.rate
        } else {
            reach as u32
        }
    }
    fn lat(&self) -> u32 {
        0
    }
}

node_impl!(Granulator, INFO, PARAMS);

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_node::Node;

    fn render(g: &mut Granulator, input: &[f32], block: usize) -> (Vec<f32>, Vec<f32>) {
        g.prepare(48_000, 512);
        g.reset();
        let mut l = input.to_vec();
        let mut r = input.to_vec();
        let ctx = Ctx::new(48_000, 0);
        let mut k = 0;
        while k < l.len() {
            let e = (k + block).min(l.len());
            g.process(&ctx, &[], &mut l[k..e], &mut r[k..e]);
            k = e;
        }
        (l, r)
    }

    fn tone(hz: f32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| 0.5 * (i as f32 * hz / 48_000.0 * std::f32::consts::TAU).sin())
            .collect()
    }

    /// The level at `hz` by a single-bin DFT.
    fn level_at(x: &[f32], hz: f32) -> f32 {
        let (mut re, mut im) = (0.0f64, 0.0f64);
        for (i, v) in x.iter().enumerate() {
            let w = std::f64::consts::TAU * hz as f64 * i as f64 / 48_000.0;
            re += *v as f64 * w.cos();
            im += *v as f64 * w.sin();
        }
        (2.0 * (re * re + im * im).sqrt() / x.len() as f64) as f32
    }

    #[test]
    fn an_octave_up_puts_the_tone_an_octave_up() {
        let mut g = Granulator::new();
        g.set_param(ids::PITCH, 12.0);
        g.set_param(ids::MIX, 100.0);
        g.set_param(ids::WIDTH, 0.0);
        let (l, _) = render(&mut g, &tone(400.0, 48_000), 128);
        let tail = &l[24_000..];
        assert!(level_at(tail, 800.0) > 4.0 * level_at(tail, 400.0));
    }

    #[test]
    fn the_cloud_stays_near_the_input_level() {
        for (size, density) in [(20.0, 4.0), (80.0, 16.0), (500.0, 80.0)] {
            let mut g = Granulator::new();
            g.set_param(ids::SIZE, size);
            g.set_param(ids::DENSITY, density);
            g.set_param(ids::JITTER, 100.0);
            g.set_param(ids::MIX, 100.0);
            let input = tone(300.0, 96_000);
            let (l, r) = render(&mut g, &input, 128);
            let rms = |x: &[f32]| (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt();
            let ratio = 0.5 * (rms(&l[48_000..]) + rms(&r[48_000..])) / rms(&input[48_000..]);
            assert!(ratio > 0.1 && ratio < 2.0, "{size} {density}: {ratio}");
            let peak = l.iter().chain(&r).fold(0.0f32, |m, v| m.max(v.abs()));
            assert!(peak < 1.5, "{peak}");
        }
    }

    #[test]
    fn freeze_keeps_playing_after_the_input_stops() {
        let mut g = Granulator::new();
        g.set_param(ids::MIX, 100.0);
        g.set_param(ids::DELAY, 50.0);
        let mut input = tone(300.0, 96_000);
        for v in &mut input[48_000..] {
            *v = 0.0;
        }
        // Freeze set during the tone (at 0.9 s).
        g.prepare(48_000, 512);
        g.reset();
        let mut l = input.clone();
        let mut r = input.clone();
        let ctx = Ctx::new(48_000, 0);
        for k in (0..96_000).step_by(480) {
            if k == 43_200 {
                g.set_param(ids::FREEZE, 1.0);
            }
            g.process(&ctx, &[], &mut l[k..k + 480], &mut r[k..k + 480]);
        }
        let late = &l[90_000..];
        assert!(level_at(late, 300.0) > 0.05, "{}", level_at(late, 300.0));
    }

    #[test]
    fn reverse_and_any_pitch_stay_finite_and_bounded() {
        for pitch in [-24.0, -7.0, 0.0, 12.0, 24.0] {
            for frozen in [0.0, 1.0] {
                let mut g = Granulator::new();
                g.set_param(ids::PITCH, pitch);
                g.set_param(ids::REVERSE, 100.0);
                g.set_param(ids::SIZE, 500.0);
                g.set_param(ids::JITTER, 100.0);
                g.set_param(ids::DELAY, MAX_DELAY_MS);
                g.set_param(ids::FEEDBACK, 90.0);
                g.set_param(ids::FREEZE, frozen);
                let (l, r) = render(&mut g, &tone(200.0, 96_000), 64);
                for v in l.iter().chain(&r) {
                    assert!(v.is_finite() && v.abs() < 4.0, "{pitch} {frozen}: {v}");
                }
            }
        }
    }

    #[test]
    fn the_same_render_whatever_the_block_size() {
        let mut a = Granulator::new();
        let mut b = Granulator::new();
        for g in [&mut a, &mut b] {
            g.set_param(ids::DETUNE, 30.0);
            g.set_param(ids::REVERSE, 50.0);
            g.set_param(ids::FEEDBACK, 40.0);
        }
        let input = tone(250.0, 20_000);
        let (la, _) = render(&mut a, &input, 37);
        let (lb, _) = render(&mut b, &input, 512);
        assert_eq!(la, lb);
    }
}
