//! The Ensemble: the string machine's chorus, which is what makes a
//! divided-down organ tone sound like a string section.
//!
//! Three delay lines (or four), each swept by two LFOs at once, a slow
//! one (the chorus, about 0.66 Hz) and a fast one (the vibrato, about
//! 6 Hz), with each line's LFOs a third of a cycle (or a quarter) apart
//! from the next line's, so the copies all wobble differently (Florian
//! Anwander's notes on the Roland string choruses; the Solina's three
//! lines; `docs/dev/research/string-machine.md`). The lines are panned
//! across the stereo field by Width and summed, and the wet path is
//! rolled off like the bucket-brigade lines it copies.
//!
//! **Depth is in cents.** Each LFO's depth is how far it bends the pitch
//! each way, so a label means what it says at any rate (wave 1 target
//! S1): a sweep of `A` seconds at `f` Hz bends the pitch by a ratio of
//! `1 ± 2 pi f A`, and that is set so the bends up and down average the
//! cents asked for. The lines' shared delay sits [`MIN_MS`] above the
//! deepest the sweeps reach, and glides when a depth or rate moves it.
//!
//! Everything that can move glides: depths, Width, Tone, Mix, and a change
//! from three lines to four, which fades the fourth line in while the
//! others move to their new places.

use mpfree_node::{Ctx, Curve, Kind, Node, NodeEvent, NodeInfo, ParamInfo, Smooth, Timed, Unit};

use crate::kit::{self, Blend};
use crate::modfx::{FlatLine, TAPS};
use crate::svf::{Segs, Svf, SvfMode};
use crate::{defaults, Preset};

/// [`Ensemble`] parameter ids.
pub mod ids {
    /// The slow LFO (the chorus), Hz.
    pub const SLOW_RATE: u16 = 0;
    /// How far the slow LFO bends the pitch each way, cents.
    pub const SLOW_DEPTH: u16 = 1;
    /// The fast LFO (the vibrato), Hz.
    pub const FAST_RATE: u16 = 2;
    /// How far the fast LFO bends the pitch each way, cents.
    pub const FAST_DEPTH: u16 = 3;
    /// Three lines or four.
    pub const LINES: u16 = 4;
    pub const WIDTH: u16 = 5;
    /// The wet path's roll-off, Hz (3 dB down there).
    pub const TONE: u16 = 6;
    pub const MIX: u16 = 7;
    /// How Mix blends ([`crate::kit::MIX_CURVES`]).
    pub const MIX_CURVE: u16 = 8;
}

/// The Lines control's choices.
const LINE_CHOICES: &[&str] = &["3", "4"];

static INFO: NodeInfo = NodeInfo {
    id: "mpfree.fx.ensemble",
    name: "Ensemble",
    kind: Kind::Effect,
};

static PARAMS: [ParamInfo; 9] = [
    ParamInfo {
        id: ids::SLOW_RATE,
        name: "Slow rate",
        group: "Slow",
        min: 0.1,
        max: 3.0,
        default: 0.66,
        unit: Unit::Hz,
        curve: Curve::Log,
        readonly: false,
    },
    ParamInfo {
        id: ids::SLOW_DEPTH,
        name: "Slow depth",
        group: "Slow",
        min: 0.0,
        max: 30.0,
        default: 12.0,
        unit: Unit::Cents,
        curve: Curve::Linear,
        readonly: false,
    },
    ParamInfo {
        id: ids::FAST_RATE,
        name: "Fast rate",
        group: "Fast",
        min: 2.0,
        max: 12.0,
        default: 6.25,
        unit: Unit::Hz,
        curve: Curve::Log,
        readonly: false,
    },
    ParamInfo {
        id: ids::FAST_DEPTH,
        name: "Fast depth",
        group: "Fast",
        min: 0.0,
        max: 15.0,
        default: 5.0,
        unit: Unit::Cents,
        curve: Curve::Linear,
        readonly: false,
    },
    ParamInfo {
        id: ids::LINES,
        name: "Lines",
        group: "Ensemble",
        min: 0.0,
        max: 1.0,
        default: 0.0,
        unit: Unit::None,
        curve: Curve::Choice(LINE_CHOICES),
        readonly: false,
    },
    ParamInfo {
        id: ids::WIDTH,
        name: "Width",
        group: "Ensemble",
        min: 0.0,
        max: 100.0,
        default: 85.0,
        unit: Unit::Percent,
        curve: Curve::Linear,
        readonly: false,
    },
    ParamInfo {
        id: ids::TONE,
        name: "Tone",
        group: "Output",
        min: 2_000.0,
        max: 20_000.0,
        default: 7_500.0,
        unit: Unit::Hz,
        curve: Curve::Log,
        readonly: false,
    },
    ParamInfo {
        id: ids::MIX,
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
        id: ids::MIX_CURVE,
        name: "Mix curve",
        group: "Output",
        min: 0.0,
        max: 2.0,
        default: 0.0,
        unit: Unit::None,
        curve: Curve::Choice(kit::MIX_CURVES),
        readonly: false,
    },
];

/// Factory presets: the string machines' chorus, and gentler ones for
/// pads and keys from other engines.
pub static PRESETS: &[Preset] = &[
    Preset {
        name: "String machine",
        values: &[],
    },
    Preset {
        name: "Four lines",
        values: &[
            (ids::LINES, 1.0),
            (ids::SLOW_RATE, 0.5),
            (ids::SLOW_DEPTH, 10.0),
            (ids::FAST_RATE, 5.5),
            (ids::FAST_DEPTH, 4.0),
        ],
    },
    Preset {
        name: "Slow shimmer",
        values: &[
            (ids::SLOW_RATE, 0.3),
            (ids::SLOW_DEPTH, 18.0),
            (ids::FAST_DEPTH, 2.0),
            (ids::TONE, 9_000.0),
        ],
    },
    Preset {
        name: "Vibrato strings",
        values: &[
            (ids::SLOW_DEPTH, 6.0),
            (ids::FAST_RATE, 7.0),
            (ids::FAST_DEPTH, 10.0),
        ],
    },
    Preset {
        name: "Pad widener",
        values: &[
            (ids::SLOW_RATE, 0.4),
            (ids::SLOW_DEPTH, 8.0),
            (ids::FAST_RATE, 5.0),
            (ids::FAST_DEPTH, 2.0),
            (ids::TONE, 12_000.0),
        ],
    },
    Preset {
        name: "Mono ensemble",
        values: &[(ids::WIDTH, 0.0)],
    },
];

/// The shortest any line's delay reaches, ms: the shared delay sits this
/// far above the deepest the sweeps go.
pub const MIN_MS: f32 = 5.0;
/// Room in the lines, ms: the shared delay and both sweeps at their
/// deepest (30 cents at 0.1 Hz and 15 cents at 2 Hz reach about 62 ms).
const MAX_MS: f32 = 70.0;
/// The wet path's level trim (+1 dB): Tone's roll-off and the lines' read
/// take about that much loudness off pink noise, so the effect goes in at
/// the dry's loudness (FX method, M1).
const WET_TRIM: f32 = 1.122;
/// How long the depths, Width and Tone take to follow their controls.
const GLIDE_MS: f32 = 30.0;
/// How long the shared delay takes to move: slow, so the move itself
/// bends the pitch by under a cent.
const BASE_MS: f32 = 500.0;
/// How long a change of Lines takes.
const LINES_MS: f32 = 500.0;

/// The sweep, in seconds each way, that bends the pitch `cents` each way
/// on average at `hz`: the bend's ratio is `1 ± x` with `x = 2 pi hz A`,
/// and `600 log2((1 + x) / (1 - x))` is `cents` when `x = tanh(cents ln 2
/// / 1200)`.
pub fn sweep_secs(cents: f32, hz: f32) -> f32 {
    let x = (cents.max(0.0) * std::f32::consts::LN_2 / 1200.0).tanh();
    x / (std::f32::consts::TAU * hz.max(1e-3))
}

/// A glide for the shared delay, kept in f64: it moves a few frames over
/// half a second while sitting hundreds of frames up, so each frame's step
/// is smaller than an f32 can add to it (and [`Smooth`] would snap
/// straight to its target, a pitch blip).
#[derive(Clone, Copy, Debug, Default)]
struct SlowGlide {
    value: f64,
    target: f64,
    coef: f64,
}

impl SlowGlide {
    fn set_time(&mut self, rate: u32, ms: f32) {
        let n = (f64::from(ms) * 0.001 * f64::from(rate)).max(1.0);
        self.coef = 1.0 - (-1.0 / n).exp();
    }
    fn snap(&mut self) {
        self.value = self.target;
    }
    fn settled(&self) -> bool {
        self.value == self.target
    }
    #[inline]
    fn next(&mut self) {
        let gap = self.target - self.value;
        self.value = if gap.abs() < 1e-6 {
            self.target
        } else {
            self.value + gap * self.coef
        };
    }
}

/// A string ensemble chorus: three or four swept delay lines, each with a
/// slow and a fast LFO, panned across the stereo field. See the module
/// notes.
pub struct Ensemble {
    v: [f32; 9],
    rate: u32,
    lines: [FlatLine; 2],
    /// The LFOs' phases in cycles (f64, so a long run never drifts), and
    /// their steps a frame.
    slow: f64,
    slow_inc: f64,
    fast: f64,
    fast_inc: f64,
    /// Each sweep either side of the shared delay, frames.
    slow_sweep: Smooth,
    fast_sweep: Smooth,
    /// The shared delay, frames.
    base: SlowGlide,
    /// 3 or 4 lines, gliding between them.
    count: Smooth,
    width: Smooth,
    log_tone: Smooth,
    tone: Svf,
    blend: Blend,
    /// While Lines and Width hold still: each line's gain on each side
    /// (the pan and the level correction together) and its LFO phase.
    gl: [f32; 4],
    gr: [f32; 4],
    offset: [f32; 4],
}

impl Default for Ensemble {
    fn default() -> Self {
        Self::new()
    }
}

impl Ensemble {
    pub fn new() -> Ensemble {
        let v = defaults(&PARAMS);
        let mut e = Ensemble {
            v,
            rate: 0,
            lines: [FlatLine::default(), FlatLine::default()],
            slow: 0.0,
            slow_inc: 0.0,
            fast: 0.0,
            fast_inc: 0.0,
            slow_sweep: Smooth::new(0.0),
            fast_sweep: Smooth::new(0.0),
            base: SlowGlide::default(),
            count: Smooth::new(3.0),
            width: Smooth::new(0.85),
            log_tone: Smooth::new(v[ids::TONE as usize].log2()),
            tone: Svf::default(),
            blend: Blend::new(1.0, 0),
            gl: [0.0; 4],
            gr: [0.0; 4],
            offset: [0.0; 4],
        };
        e.prepare(48_000, mpfree_node::MAX_BLOCK);
        e.reset();
        e
    }

    fn count_target(&self) -> f32 {
        if self.v[ids::LINES as usize] >= 0.5 {
            4.0
        } else {
            3.0
        }
    }

    /// Line `line`'s delay in ms at slow and fast LFO phases `slow` and
    /// `fast` (cycles), for the settings set; `None` for a line not in
    /// use. For drawing and the tests.
    pub fn delay_ms(&self, line: usize, slow: f32, fast: f32) -> Option<f32> {
        let n = self.count_target() as usize;
        if line >= n {
            return None;
        }
        let (s, f, base) = self.sweeps_ms();
        let off = line as f32 / n as f32;
        Some(base + s * kit::sine(slow + off) + f * kit::sine(fast + off))
    }

    /// The slow and fast sweeps either side of the shared delay, and the
    /// shared delay, ms, for the settings set.
    pub fn sweeps_ms(&self) -> (f32, f32, f32) {
        let s = 1000.0
            * sweep_secs(
                self.v[ids::SLOW_DEPTH as usize],
                self.v[ids::SLOW_RATE as usize],
            );
        let f = 1000.0
            * sweep_secs(
                self.v[ids::FAST_DEPTH as usize],
                self.v[ids::FAST_RATE as usize],
            );
        (s, f, MIN_MS + s + f)
    }

    /// Aims the sweeps and the shared delay, and sets the LFOs' steps.
    fn aim(&mut self) {
        let fs = self.rate as f32;
        let (s, f, base) = self.sweeps_ms();
        self.slow_sweep.target = s * 0.001 * fs;
        self.fast_sweep.target = f * 0.001 * fs;
        self.base.target = f64::from(base * 0.001 * fs);
        let r = f64::from(self.rate.max(1));
        self.slow_inc = f64::from(self.v[ids::SLOW_RATE as usize]) / r;
        self.fast_inc = f64::from(self.v[ids::FAST_RATE as usize]) / r;
    }

    fn apply(&mut self, id: u16, value: f32) {
        let Some(p) = PARAMS.get(id as usize) else {
            return;
        };
        let v = p.clamp(value);
        self.v[id as usize] = v;
        match id {
            ids::LINES => self.count.target = self.count_target(),
            ids::WIDTH => self.width.target = v / 100.0,
            ids::TONE => self.log_tone.target = v.log2(),
            ids::MIX => self.blend.set_mix(v / 100.0),
            ids::MIX_CURVE => self.blend.set_curve(v),
            _ => {}
        }
        self.aim();
    }

    /// Each line's place, gains and LFO phase for the line count and
    /// Width now. The lines sit evenly from left to right (three at left,
    /// centre and right; four at thirds), each panned at equal power, and
    /// each side is scaled so the lines' powers sum to one: the copies are
    /// far enough apart in time that above a few hundred Hz they add as
    /// powers. The fourth line fades in as the count goes from 3 to 4.
    fn spread(&mut self) {
        let n = self.count.value;
        let width = self.width.value;
        let (mut pl, mut pr) = (0.0f32, 0.0f32);
        for k in 0..4 {
            let kf = k as f32;
            let level = if k < 3 { 1.0 } else { n - 3.0 };
            let place = (-1.0 + 2.0 * kf / (n - 1.0)).min(1.0) * width;
            // Equal power: a quarter cycle from hard left to hard right.
            let a = (place + 1.0) * 0.125;
            self.gl[k] = level * kit::cosine(a);
            self.gr[k] = level * kit::sine(a);
            pl += self.gl[k] * self.gl[k];
            pr += self.gr[k] * self.gr[k];
            self.offset[k] = kf / n;
        }
        let (nl, nr) = (1.0 / pl.max(1e-9).sqrt(), 1.0 / pr.max(1e-9).sqrt());
        for k in 0..4 {
            self.gl[k] *= nl;
            self.gr[k] *= nr;
        }
    }

    fn set_tone(&mut self) {
        self.tone.set(
            SvfMode::LowPass,
            self.log_tone.value.exp2(),
            std::f32::consts::FRAC_1_SQRT_2,
            0.0,
            self.rate,
        );
    }

    /// Line `k`'s delay this frame, in frames.
    #[inline]
    fn line_delay(&self, k: usize) -> f32 {
        let (ss, fs) = (self.slow_sweep.value, self.fast_sweep.value);
        // The shortest read the flat read allows, in case the shared delay
        // is still on its way up.
        let base = (self.base.value as f32).max((TAPS / 2) as f32 + ss + fs);
        let off = self.offset[k];
        base + ss * kit::sine(self.slow as f32 + off) + fs * kit::sine(self.fast as f32 + off)
    }

    /// [`Node::process`] for a mono source with no events: `left` holds
    /// the input, and both sides come out. Each line is read once for both
    /// sides, so it costs well under the stereo path, and the output is the
    /// same to the bit as [`Node::process`] given the input on both sides.
    /// An instrument with the ensemble built in (Strings) uses it.
    pub fn process_mono(&mut self, left: &mut [f32], right: &mut [f32]) {
        let n = left.len().min(right.len());
        self.run_with::<true>(&mut left[..n], &mut right[..n]);
    }

    fn run(&mut self, l: &mut [f32], r: &mut [f32]) {
        self.run_with::<false>(l, r);
    }

    #[inline(always)]
    fn run_with<const MONO: bool>(&mut self, l: &mut [f32], r: &mut [f32]) {
        for i in 0..l.len() {
            if MONO {
                r[i] = l[i];
            }
            if !(self.slow_sweep.settled() && self.fast_sweep.settled()) {
                self.slow_sweep.next();
                self.fast_sweep.next();
            }
            if !self.base.settled() {
                self.base.next();
            }
            if !(self.count.settled() && self.width.settled()) {
                self.count.next();
                self.width.next();
                self.spread();
            }
            if !self.log_tone.settled() {
                self.log_tone.next();
                self.set_tone();
            }
            let mix = self.blend.next();
            self.lines[0].push(l[i]);
            self.lines[1].push(r[i]);
            let lines = if self.count.value > 3.0 { 4 } else { 3 };
            let (mut wl, mut wr) = (0.0f32, 0.0f32);
            for k in 0..lines {
                let d = self.line_delay(k);
                if MONO {
                    // Both lines hold the same: one read serves both sides.
                    if self.gl[k] != 0.0 || self.gr[k] != 0.0 {
                        let v = self.lines[0].read(d);
                        if self.gl[k] != 0.0 {
                            wl += self.gl[k] * v;
                        }
                        if self.gr[k] != 0.0 {
                            wr += self.gr[k] * v;
                        }
                    }
                    continue;
                }
                if self.gl[k] != 0.0 {
                    wl += self.gl[k] * self.lines[0].read(d);
                }
                if self.gr[k] != 0.0 {
                    wr += self.gr[k] * self.lines[1].read(d);
                }
            }
            let wl = self.tone.run(0, WET_TRIM * wl);
            let wr = self.tone.run(1, WET_TRIM * wr);
            l[i] = kit::blend(l[i], wl, mix);
            r[i] = kit::blend(r[i], wr, mix);
            self.slow += self.slow_inc;
            if self.slow >= 1.0 {
                self.slow -= 1.0;
            }
            self.fast += self.fast_inc;
            if self.fast >= 1.0 {
                self.fast -= 1.0;
            }
        }
    }
}

impl Node for Ensemble {
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
        let new_rate = rate != self.rate;
        if new_rate {
            let frames = (MAX_MS * 0.001 * rate as f32) as usize + TAPS;
            self.lines = [FlatLine::new(frames), FlatLine::new(frames)];
        }
        self.rate = rate;
        for s in [
            &mut self.slow_sweep,
            &mut self.fast_sweep,
            &mut self.width,
            &mut self.log_tone,
        ] {
            s.set_time(rate, GLIDE_MS);
        }
        self.base.set_time(rate, BASE_MS);
        self.count.set_time(rate, LINES_MS);
        self.blend.set_time(rate, GLIDE_MS);
        self.aim();
        if new_rate {
            // The sweeps and the shared delay are in frames.
            for s in [&mut self.slow_sweep, &mut self.fast_sweep] {
                s.snap(s.target);
            }
            self.base.snap();
        }
        self.spread();
        self.set_tone();
    }
    fn reset(&mut self) {
        for s in [
            &mut self.slow_sweep,
            &mut self.fast_sweep,
            &mut self.count,
            &mut self.width,
            &mut self.log_tone,
        ] {
            s.snap(s.target);
        }
        self.base.snap();
        self.blend.snap();
        for line in &mut self.lines {
            line.clear();
        }
        self.tone.reset();
        self.slow = 0.0;
        self.fast = 0.0;
        self.spread();
        self.set_tone();
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
        let (s, f, base) = self.sweeps_ms();
        // The longest delay, then the tone filter settling.
        ((base + s + f) * 0.001 * self.rate as f32) as u32 + self.rate / 100
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_mono_path_gives_the_stereo_path_to_the_bit() {
        let mut a = Ensemble::new();
        let mut b = Ensemble::new();
        for e in [&mut a, &mut b] {
            e.set_param(ids::LINES, 1.0);
            e.set_param(ids::WIDTH, 70.0);
        }
        let mut seed = 1u32;
        let ctx = Ctx::new(48_000, 0);
        for block in 0..40 {
            let x: Vec<f32> = (0..96)
                .map(|_| {
                    seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    (seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5
                })
                .collect();
            if block == 20 {
                // A move mid-run glides the same on both.
                a.set_param(ids::LINES, 0.0);
                b.set_param(ids::LINES, 0.0);
            }
            let (mut l1, mut r1) = (x.clone(), x.clone());
            a.process(&ctx, &[], &mut l1, &mut r1);
            let (mut l2, mut r2) = (x.clone(), vec![0.0; x.len()]);
            b.process_mono(&mut l2, &mut r2);
            assert_eq!(l1, l2, "block {block}");
            assert_eq!(r1, r2, "block {block}");
        }
    }

    use super::*;

    /// Line `k`'s delay, frame by frame, over `secs` of silence.
    fn trace(e: &mut Ensemble, k: usize, secs: f32) -> Vec<f32> {
        let n = (secs * e.rate as f32) as usize;
        let ctx = Ctx::new(e.rate, 0);
        let (mut l, mut r) = ([0.0f32], [0.0f32]);
        (0..n)
            .map(|_| {
                let d = e.line_delay(k);
                e.process(&ctx, &[], &mut l, &mut r);
                d
            })
            .collect()
    }

    /// The largest bends up and down, cents, of a delay trace.
    fn bends(d: &[f32]) -> (f32, f32) {
        let mut up = 0.0f32;
        let mut down = 0.0f32;
        for w in d.windows(2) {
            let c = 1200.0 * (1.0 - f64::from(w[1] - w[0])).log2() as f32;
            up = up.max(c);
            down = down.min(c);
        }
        (up, down)
    }

    #[test]
    fn each_wobble_bends_the_pitch_by_its_label() {
        // S1: depth in cents as labelled, each LFO alone.
        for (slow, fast) in [(12.0, 0.0), (0.0, 5.0), (30.0, 0.0), (0.0, 15.0)] {
            let mut e = Ensemble::new();
            e.prepare(48_000, 64);
            e.set_param(ids::SLOW_DEPTH, slow);
            e.set_param(ids::FAST_DEPTH, fast);
            e.reset();
            let (up, down) = bends(&trace(&mut e, 0, 2.0));
            let want = slow + fast;
            let mean = (up - down) / 2.0;
            assert!(
                (mean - want).abs() < 0.02 * want,
                "{slow}/{fast}: {up} {down}"
            );
        }
    }

    #[test]
    fn the_wobbles_run_at_their_rates() {
        // S1: rates within 5 % (here exact to the frame).
        let mut e = Ensemble::new();
        e.prepare(48_000, 64);
        e.set_param(ids::FAST_DEPTH, 0.0);
        e.reset();
        let d = trace(&mut e, 0, 4.0);
        let mid = d.iter().sum::<f32>() / d.len() as f32;
        let ups = d.windows(2).filter(|w| w[0] < mid && w[1] >= mid).count();
        // 0.66 Hz over 4 s: 2.64 cycles, so two or three upward crossings.
        assert!((2..=3).contains(&ups), "{ups}");
        let mut f = Ensemble::new();
        f.prepare(48_000, 64);
        f.set_param(ids::SLOW_DEPTH, 0.0);
        f.reset();
        let d = trace(&mut f, 0, 2.0);
        let mid = d.iter().sum::<f32>() / d.len() as f32;
        let ups = d.windows(2).filter(|w| w[0] < mid && w[1] >= mid).count() as f32;
        assert!((ups / 2.0 - 6.25).abs() <= 0.5, "{ups}");
    }

    #[test]
    fn a_sweep_bends_the_pitch_by_its_cents() {
        for (cents, hz) in [(12.0f32, 0.66f32), (5.0, 6.25), (30.0, 0.1)] {
            let x = std::f32::consts::TAU * hz * sweep_secs(cents, hz);
            let up = 1200.0 * (1.0 + x).log2();
            let down = -1200.0 * (1.0 - x).log2();
            assert!(((up + down) / 2.0 - cents).abs() < 1e-3, "{cents} at {hz}");
        }
        assert_eq!(sweep_secs(0.0, 1.0), 0.0);
    }

    #[test]
    fn the_lines_keep_their_power_at_every_width_and_count() {
        let mut e = Ensemble::new();
        for lines in [0.0, 1.0] {
            for width in [0.0, 50.0, 100.0] {
                e.set_param(ids::LINES, lines);
                e.set_param(ids::WIDTH, width);
                e.reset();
                let n = if lines > 0.5 { 4 } else { 3 };
                let pl: f32 = e.gl[..n].iter().map(|g| g * g).sum();
                let pr: f32 = e.gr[..n].iter().map(|g| g * g).sum();
                assert!((pl - 1.0).abs() < 1e-5 && (pr - 1.0).abs() < 1e-5);
                assert!(e.gl[n..].iter().all(|&g| g == 0.0));
            }
        }
    }

    #[test]
    fn the_lines_sit_a_third_of_a_cycle_apart() {
        let e = Ensemble::new();
        let (s, _, base) = e.sweeps_ms();
        // Line 1 at phase 0 is where line 0 is a third of a cycle on.
        let a = e.delay_ms(0, 1.0 / 3.0, 1.0 / 3.0).unwrap();
        let b = e.delay_ms(1, 0.0, 0.0).unwrap();
        assert!((a - b).abs() < 1e-5);
        assert!(e.delay_ms(3, 0.0, 0.0).is_none());
        // The deepest any line reaches stays MIN_MS above nothing.
        assert!(base - s >= MIN_MS);
    }
}
