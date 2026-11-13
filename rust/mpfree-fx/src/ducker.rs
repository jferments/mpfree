//! Ducker: turns the sound down on a trigger and brings it back on a
//! shaped curve, the "pump" of a kick ducking a bass or pad.
//!
//! Two sources trigger it:
//!
//! - **Sidechain**: the key signal (another track, usually the kick) going
//!   over the threshold. While the key stays over, the duck holds; when it
//!   falls back under, the hold runs and then the release.
//! - **Tempo**: every 1/16, 1/8, 1/4, 1/2 or bar of the song, with no key
//!   needed. While the transport plays it follows the song position; when
//!   stopped it keeps time on its own from the tempo.
//!
//! The duck itself is an envelope from 0 (untouched) to 1 (down by Depth):
//! a straight attack, a hold, then a release whose Shape bends it from a
//! straight line (0) to a slow start and fast finish (100), the classic
//! pumping curve. A trigger during the release attacks again from where the
//! envelope is, so nothing jumps.

use mpfree_node::{Ctx, Curve, Kind, Node, NodeEvent, NodeInfo, ParamInfo, Smooth, Timed, Unit};

use crate::svf::Segs;
use crate::{defaults, fast_db_to_gain, fast_level_db};

/// [`Ducker`] parameter ids.
pub mod ids {
    pub const SOURCE: u16 = 0;
    pub const DEPTH: u16 = 1;
    pub const THRESHOLD: u16 = 2;
    pub const RATE: u16 = 3;
    pub const ATTACK: u16 = 4;
    pub const HOLD: u16 = 5;
    pub const RELEASE: u16 = 6;
    pub const SHAPE: u16 = 7;
    pub const MIX: u16 = 8;
    /// Meter: the gain now, in dB (0 or below).
    pub const GAIN: u16 = 9;
}

/// The sources, as [`ids::SOURCE`] values.
pub const SOURCES: &[&str] = &["Sidechain", "Tempo"];
/// The tempo mode's lengths, as [`ids::RATE`] values.
pub const RATES: &[&str] = &["1/16", "1/8", "1/4", "1/2", "1 bar"];
/// Each rate's length in beats.
pub const RATE_BEATS: [f64; 5] = [0.25, 0.5, 1.0, 2.0, 4.0];

static INFO: NodeInfo = NodeInfo {
    id: "mpfree.fx.ducker",
    name: "Ducker",
    kind: Kind::Effect,
};

#[allow(clippy::too_many_arguments)]
const fn p(
    id: u16,
    name: &'static str,
    group: &'static str,
    min: f32,
    max: f32,
    default: f32,
    unit: Unit,
    curve: Curve,
) -> ParamInfo {
    ParamInfo {
        id,
        name,
        group,
        min,
        max,
        default,
        unit,
        curve,
        readonly: false,
    }
}

static PARAMS: [ParamInfo; 10] = [
    p(
        0,
        "Source",
        "Trigger",
        0.0,
        1.0,
        0.0,
        Unit::None,
        Curve::Choice(SOURCES),
    ),
    p(1, "Depth", "Duck", 0.0, 48.0, 18.0, Unit::Db, Curve::Linear),
    p(
        2,
        "Threshold",
        "Trigger",
        -60.0,
        0.0,
        -24.0,
        Unit::Db,
        Curve::Linear,
    ),
    p(
        3,
        "Rate",
        "Trigger",
        0.0,
        4.0,
        2.0,
        Unit::None,
        Curve::Choice(RATES),
    ),
    p(4, "Attack", "Duck", 0.1, 50.0, 2.0, Unit::Ms, Curve::Log),
    p(5, "Hold", "Duck", 0.0, 500.0, 20.0, Unit::Ms, Curve::Linear),
    p(
        6,
        "Release",
        "Duck",
        10.0,
        2000.0,
        180.0,
        Unit::Ms,
        Curve::Log,
    ),
    p(
        7,
        "Shape",
        "Duck",
        0.0,
        100.0,
        50.0,
        Unit::Percent,
        Curve::Linear,
    ),
    p(
        8,
        "Mix",
        "Output",
        0.0,
        100.0,
        100.0,
        Unit::Percent,
        Curve::Linear,
    ),
    ParamInfo {
        id: 9,
        name: "Gain",
        group: "Meters",
        min: -48.0,
        max: 0.0,
        default: 0.0,
        unit: Unit::Db,
        curve: Curve::Linear,
        readonly: true,
    },
];

/// Where the envelope is.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Stage {
    Idle,
    Attack,
    /// Frames of hold left.
    Hold(u32),
    Release,
}

/// The release curve at `t` (0 at the start of the release, 1 at its end):
/// how far down the duck still is, from 1 to 0. Shape 0 is a straight
/// line; shape 1 stays down longer and comes back fast (`1 - t^k`).
#[inline]
fn release_curve(t: f32, shape: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    // k from 1 (straight) to 4: an integer-free power by repeated product
    // blended between whole powers, so no powf on the audio thread.
    let k = 1.0 + 3.0 * shape;
    let lo = k.floor();
    let frac = k - lo;
    let mut a = t;
    for _ in 1..lo as u32 {
        a *= t;
    }
    let b = a * t;
    1.0 - (a + (b - a) * frac)
}

/// A ducker: a volume envelope triggered by a sidechain key or the tempo.
pub struct Ducker {
    v: [f32; 10],
    rate: u32,
    depth: Smooth,
    mix: Smooth,
    stage: Stage,
    /// The duck now, 0 to 1.
    env: f32,
    /// Release progress, 0 to 1, and the duck it started from.
    rel_t: f32,
    rel_from: f32,
    /// The key's peak follower (linear).
    key: f32,
    key_fall: f32,
    /// Whether the key is over the threshold now.
    over: bool,
    /// The tempo cell last seen (None after a reset).
    cell: Option<i64>,
}

impl Default for Ducker {
    fn default() -> Self {
        Self::new()
    }
}

impl Ducker {
    pub fn new() -> Ducker {
        let v = defaults(&PARAMS);
        let mut d = Ducker {
            v,
            rate: 48_000,
            depth: Smooth::new(v[1]),
            mix: Smooth::new(v[8] / 100.0),
            stage: Stage::Idle,
            env: 0.0,
            rel_t: 0.0,
            rel_from: 0.0,
            key: 0.0,
            key_fall: 0.0,
            over: false,
            cell: None,
        };
        d.prepare(48_000, mpfree_node::MAX_BLOCK);
        d
    }

    /// The gain curve over one cycle, for drawing: `points` values from 0
    /// (no duck) to 1 (down by Depth) across `ms` milliseconds after a
    /// trigger.
    pub fn curve(&self, ms: f32, points: usize) -> Vec<f32> {
        let (att, hold, rel, shape) = (self.v[4], self.v[5], self.v[6], self.v[7] / 100.0);
        (0..points)
            .map(|i| {
                let t = ms * i as f32 / (points.max(2) - 1) as f32;
                if t < att {
                    t / att
                } else if t < att + hold {
                    1.0
                } else {
                    release_curve((t - att - hold) / rel, shape)
                }
            })
            .collect()
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
            ids::DEPTH => self.depth.target = v,
            ids::MIX => self.mix.target = v / 100.0,
            ids::SOURCE | ids::RATE => self.cell = None,
            _ => {}
        }
    }

    fn trigger(&mut self) {
        self.stage = Stage::Attack;
    }

    /// Runs `l` and `r` from frame `first` of the block.
    fn run(
        &mut self,
        ctx: &Ctx,
        first: usize,
        l: &mut [f32],
        r: &mut [f32],
        key: Option<(&[f32], &[f32])>,
    ) {
        let fs = self.rate as f32;
        let att_step = 1.0 / (self.v[4] * 0.001 * fs).max(1.0);
        let hold = (self.v[5] * 0.001 * fs) as u32;
        let rel_step = 1.0 / (self.v[6] * 0.001 * fs).max(1.0);
        let shape = self.v[7] / 100.0;
        let thresh = self.v[2];
        let tempo = self.v[0] >= 0.5;
        let beats = RATE_BEATS[(self.v[3] as usize).min(RATE_BEATS.len() - 1)];
        let per_frame = if ctx.bpm > 0.0 {
            ctx.bpm / 60.0 / self.rate as f64
        } else {
            0.0
        };
        for i in 0..l.len() {
            let depth = self.depth.next();
            let mix = self.mix.next();
            if tempo {
                if per_frame > 0.0 {
                    let at = (first + i) as f64;
                    // Song position while playing; free time from the
                    // absolute frame while stopped.
                    let b = if ctx.playing {
                        ctx.beats + at * per_frame
                    } else {
                        (ctx.frame as f64 + at) * per_frame
                    };
                    let cell = (b / beats).floor() as i64;
                    if self.cell != Some(cell) {
                        self.cell = Some(cell);
                        self.trigger();
                    }
                }
            } else if let Some((kl, kr)) = key {
                let x = kl[i].abs().max(kr[i].abs());
                self.key = if x > self.key {
                    x
                } else {
                    crate::flush(self.key * self.key_fall)
                };
                let db = fast_level_db(self.key);
                // 3 dB of hysteresis so a key sitting on the line doesn't
                // chatter.
                if !self.over && db > thresh {
                    self.over = true;
                    self.trigger();
                } else if self.over && db < thresh - 3.0 {
                    self.over = false;
                }
            } else {
                self.over = false;
            }
            match self.stage {
                Stage::Idle => {}
                Stage::Attack => {
                    self.env += att_step;
                    if self.env >= 1.0 {
                        self.env = 1.0;
                        self.stage = Stage::Hold(hold);
                    }
                }
                Stage::Hold(n) => {
                    if self.over {
                        self.stage = Stage::Hold(hold);
                    } else if n == 0 {
                        self.stage = Stage::Release;
                        self.rel_t = 0.0;
                        self.rel_from = self.env;
                    } else {
                        self.stage = Stage::Hold(n - 1);
                    }
                }
                Stage::Release => {
                    self.rel_t += rel_step;
                    if self.rel_t >= 1.0 {
                        self.env = 0.0;
                        self.stage = Stage::Idle;
                    } else {
                        self.env = self.rel_from * release_curve(self.rel_t, shape);
                    }
                }
            }
            let g = if self.env > 0.0 {
                fast_db_to_gain(-depth * self.env)
            } else {
                1.0
            };
            let wet = 1.0 + mix * (g - 1.0);
            l[i] *= wet;
            r[i] *= wet;
        }
        let g = -self.depth.target * self.env;
        self.v[ids::GAIN as usize] = g.clamp(-48.0, 0.0);
    }
}

impl Node for Ducker {
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
        self.depth.set_time(self.rate, 20.0);
        self.mix.set_time(self.rate, 20.0);
        // The key follower falls 60 dB in 50 ms.
        self.key_fall = (-6.9 / (0.05 * self.rate as f32)).exp();
    }
    fn reset(&mut self) {
        self.depth.snap(self.depth.target);
        self.mix.snap(self.mix.target);
        self.stage = Stage::Idle;
        self.env = 0.0;
        self.rel_t = 0.0;
        self.rel_from = 0.0;
        self.key = 0.0;
        self.over = false;
        self.cell = None;
        self.v[ids::GAIN as usize] = 0.0;
    }
    fn process(&mut self, ctx: &Ctx, events: &[Timed], left: &mut [f32], right: &mut [f32]) {
        let len = left.len().min(right.len());
        let side = ctx
            .sidechain
            .filter(|(a, b)| a.len() >= len && b.len() >= len);
        let mut segs = Segs::new(events);
        while let Some((from, to)) = segs.next(len, |ev| {
            if let NodeEvent::Param { id, value } = *ev {
                self.apply(id, value)
            }
        }) {
            let key = side.map(|(a, b)| (&a[from..to], &b[from..to]));
            self.run(ctx, from, &mut left[from..to], &mut right[from..to], key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_curve_ends_and_bends() {
        for s in [0.0, 0.3, 0.5, 1.0] {
            assert_eq!(release_curve(0.0, s), 1.0);
            assert_eq!(release_curve(1.0, s), 0.0);
        }
        assert!((release_curve(0.5, 0.0) - 0.5).abs() < 1e-6);
        // More shape stays down longer.
        assert!(release_curve(0.5, 1.0) > release_curve(0.5, 0.5));
        assert!(release_curve(0.5, 0.5) > release_curve(0.5, 0.0));
        // Continuous in shape across whole powers.
        let a = release_curve(0.4, 1.0 / 3.0 - 1e-4);
        let b = release_curve(0.4, 1.0 / 3.0 + 1e-4);
        assert!((a - b).abs() < 1e-3);
    }
}
