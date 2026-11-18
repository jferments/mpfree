//! Ducker: turns the sound down on a trigger and brings it back on a
//! shaped curve, the "pump" of a kick ducking a bass or pad.
//!
//! Two sources trigger it:
//!
//! - **Sidechain**: the key signal (another track, usually the kick) going
//!   over the threshold, through the key's own high-pass and low-pass so a
//!   drum bus can duck on its kick alone. While the key stays over, the
//!   duck holds; when it falls back under, the hold runs and then the
//!   release.
//! - **Tempo**: every 1/16, 1/8, 1/4, 1/2 or bar of the song, or a dotted
//!   or triplet eighth or quarter, with no key needed. The duck is already
//!   down when each beat lands: the attack runs over the Attack before it,
//!   since the song position is known ahead (FX quality round 4, decision
//!   11.3). Nudge moves it up to 20 ms either way, for a kick that sits
//!   early or late. While the transport plays it follows the song
//!   position; when stopped it keeps time on its own from the tempo.
//!
//! The duck itself is an envelope from 0 (untouched) to 1 (down by Depth):
//! a straight attack, a hold, then a release whose Shape bends it from a
//! straight line (0) to a slow start and fast finish (100), the classic
//! pumping curve. On the key, a trigger during the release attacks again
//! from where the envelope is, so nothing jumps. In time, the envelope is
//! the cycle as drawn ([`Ducker::curve`]): down from the beat through the
//! hold, the release, then the next beat's attack taking over wherever it
//! meets the release (so a release longer than the step never finishes,
//! as it plays). After a jump of the song position or a turn of a knob it
//! catches up on that cycle at its own attack's speed, in 5 ms at most.

use mpfree_node::{Ctx, Curve, Kind, Node, NodeEvent, NodeInfo, ParamInfo, Smooth, Timed, Unit};

use crate::svf::{Segs, Svf, SvfMode};
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
    /// The key's high-pass, Hz (20, the lowest, is off).
    pub const KEY_HP: u16 = 10;
    /// The key's low-pass, Hz (20 kHz, the highest, is off).
    pub const KEY_LP: u16 = 11;
    /// Tempo: moves the duck later (or earlier, under 0), ms.
    pub const NUDGE: u16 = 12;
}

/// The sources, as [`ids::SOURCE`] values.
pub const SOURCES: &[&str] = &["Sidechain", "Tempo"];
/// The tempo mode's lengths, as [`ids::RATE`] values. The triplet and
/// dotted ones were appended (FX quality round 4), so saved projects keep
/// the length they had; the names are the LFOs'.
pub const RATES: &[&str] = &[
    "1/16", "1/8", "1/4", "1/2", "1 bar", "1/8T", "1/8D", "1/4T", "1/4D",
];
/// Each rate's length in beats.
pub const RATE_BEATS: [f64; 9] = [0.25, 0.5, 1.0, 2.0, 4.0, 1.0 / 3.0, 0.75, 2.0 / 3.0, 1.5];

/// How long the tempo duck takes at most to catch up on its cycle after
/// the song position jumps or a knob turns, ms (for the whole depth; its
/// own attack when that is faster).
const CATCH_MS: f32 = 5.0;
/// How long the timing controls (Attack, Hold, Release, Shape) glide on
/// the tempo duck, ms, so turning one moves the cycle smoothly.
const GLIDE_MS: f32 = 20.0;

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

static PARAMS: [ParamInfo; 13] = [
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
        8.0,
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
    p(
        10,
        "Key HP",
        "Trigger",
        20.0,
        2000.0,
        20.0,
        Unit::Hz,
        Curve::Log,
    ),
    p(
        11,
        "Key LP",
        "Trigger",
        50.0,
        20_000.0,
        20_000.0,
        Unit::Hz,
        Curve::Log,
    ),
    p(
        12,
        "Nudge",
        "Trigger",
        -20.0,
        20.0,
        0.0,
        Unit::Ms,
        Curve::Linear,
    ),
];

/// Where the key's envelope is.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Stage {
    Idle,
    Attack,
    /// Frames of hold left.
    Hold(u32),
    Release,
}

/// The release's bend for a Shape (0 to 1): `1 - t^k` with k from 1
/// (straight) to 4, as the whole power under k and the blend toward the
/// next, so the curve needs no powf on the audio thread.
#[derive(Clone, Copy, Debug)]
struct Bend {
    lo: u32,
    frac: f32,
}

impl Bend {
    fn new(shape: f32) -> Bend {
        let k = 1.0 + 3.0 * shape.clamp(0.0, 1.0);
        let lo = k as u32;
        Bend {
            lo,
            frac: k - lo as f32,
        }
    }

    /// How far down the duck still is at `t` (0 at the start of the
    /// release, 1 at its end), from 1 to 0.
    #[inline]
    fn at(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        let mut a = t;
        for _ in 1..self.lo {
            a *= t;
        }
        let b = a * t;
        1.0 - (a + (b - a) * self.frac)
    }
}

/// The release curve at `t` for `shape`: shape 0 is a straight line;
/// shape 1 stays down longer and comes back fast (`1 - t^4`).
#[inline]
fn release_curve(t: f32, shape: f32) -> f32 {
    Bend::new(shape).at(t)
}

/// The tempo duck at `p` ms after the beat, in a step of `step` ms: down
/// from the beat through the hold, then the release, and the next beat's
/// attack (a straight line reaching 1 as the beat lands) taking over
/// wherever it is the further down. `inv_att` and `inv_rel` are 1/Attack
/// and 1/Release, in 1/ms.
#[inline]
fn tempo_duck(p: f32, step: f32, inv_att: f32, hold: f32, inv_rel: f32, bend: Bend) -> f32 {
    let t = (p - hold) * inv_rel;
    let down = if t <= 0.0 {
        1.0
    } else if t >= 1.0 {
        0.0
    } else {
        bend.at(t)
    };
    let attack = 1.0 - (step - p) * inv_att;
    down.max(attack).clamp(0.0, 1.0)
}

/// `x.floor() as i64`, without the library calls `floor` and the
/// 64-bit conversion are on the MPC's ARM while `x` fits an `i32` (the
/// tempo cell for decades of playing); the same value either way.
#[inline]
fn floor_i64(x: f64) -> i64 {
    if x.abs() < 2_000_000_000.0 {
        let i = x as i32;
        i64::from(if f64::from(i) > x { i - 1 } else { i })
    } else {
        x.floor() as i64
    }
}

/// A ducker: a volume envelope triggered by a sidechain key or the tempo.
pub struct Ducker {
    v: [f32; 13],
    rate: u32,
    depth: Smooth,
    mix: Smooth,
    /// The tempo duck's timing, gliding: 1/Attack and 1/Release (1/ms),
    /// Hold (ms) and Shape (0 to 1).
    inv_att: Smooth,
    hold: Smooth,
    inv_rel: Smooth,
    shape: Smooth,
    stage: Stage,
    /// The duck now, 0 to 1.
    env: f32,
    /// Release progress, 0 to 1, and the duck it started from.
    rel_t: f32,
    rel_from: f32,
    /// The key's filters and peak follower (linear).
    key_hp: Svf,
    key_lp: Svf,
    key: f32,
    key_fall: f32,
    /// Whether the key is over the threshold now.
    over: bool,
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
            inv_att: Smooth::new(1.0 / v[4]),
            hold: Smooth::new(v[5]),
            inv_rel: Smooth::new(1.0 / v[6]),
            shape: Smooth::new(v[7] / 100.0),
            stage: Stage::Idle,
            env: 0.0,
            rel_t: 0.0,
            rel_from: 0.0,
            key_hp: Svf::default(),
            key_lp: Svf::default(),
            key: 0.0,
            key_fall: 0.0,
            over: false,
        };
        d.prepare(48_000, mpfree_node::MAX_BLOCK);
        d
    }

    /// The gain curve, for drawing: `points` values from 0 (no duck) to 1
    /// (down by Depth) across `ms` milliseconds. On Sidechain, one duck
    /// after a trigger at 0. On Tempo, `ms` is the step's length and the
    /// drawing is one step as it plays, from the beat (Nudge left out):
    /// down through the hold, the release, and the next beat's attack
    /// wherever it meets the release.
    pub fn curve(&self, ms: f32, points: usize) -> Vec<f32> {
        let (att, hold, rel, shape) = (self.v[4], self.v[5], self.v[6], self.v[7] / 100.0);
        let tempo = self.v[ids::SOURCE as usize] >= 0.5;
        (0..points)
            .map(|i| {
                let t = ms * i as f32 / (points.max(2) - 1) as f32;
                if tempo {
                    tempo_duck(t, ms, 1.0 / att, hold, 1.0 / rel, Bend::new(shape))
                } else if t < att {
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
        let was_tempo = self.v[ids::SOURCE as usize] >= 0.5;
        self.v[id as usize] = v;
        match id {
            ids::DEPTH => self.depth.target = v,
            ids::MIX => self.mix.target = v / 100.0,
            ids::ATTACK => self.inv_att.target = 1.0 / v,
            ids::HOLD => self.hold.target = v,
            ids::RELEASE => self.inv_rel.target = 1.0 / v,
            ids::SHAPE => self.shape.target = v / 100.0,
            ids::KEY_HP | ids::KEY_LP => self.set_key_filters(),
            // From Tempo to Sidechain: let go of the duck from where it is.
            ids::SOURCE if was_tempo && v < 0.5 => {
                self.over = false;
                if self.env > 0.0 {
                    self.stage = Stage::Release;
                    self.rel_t = 0.0;
                    self.rel_from = self.env;
                } else {
                    self.stage = Stage::Idle;
                }
            }
            _ => {}
        }
    }

    fn set_key_filters(&mut self) {
        let rate = self.rate;
        let hp = self.v[ids::KEY_HP as usize];
        let lp = self.v[ids::KEY_LP as usize].min(0.45 * rate as f32);
        self.key_hp.set(SvfMode::HighPass, hp, 0.707, 0.0, rate);
        self.key_lp.set(SvfMode::LowPass, lp, 0.707, 0.0, rate);
    }

    /// The gain for duck `env` at `depth` dB, mixed.
    #[inline]
    fn gain(env: f32, depth: f32, mix: f32) -> f32 {
        let g = if env > 0.0 {
            fast_db_to_gain(-depth * env)
        } else {
            1.0
        };
        1.0 + mix * (g - 1.0)
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
        if self.v[ids::SOURCE as usize] >= 0.5 {
            self.run_tempo(ctx, first, l, r);
        } else {
            self.run_key(l, r, key);
        }
        let g = -self.depth.target * self.env;
        self.v[ids::GAIN as usize] = g.clamp(-48.0, 0.0);
    }

    /// The tempo duck: the cycle as drawn, at the song position (or the
    /// free-running time while stopped), caught up at the attack's speed.
    fn run_tempo(&mut self, ctx: &Ctx, first: usize, l: &mut [f32], r: &mut [f32]) {
        let ms_frame = 1000.0 / self.rate as f32;
        let beats = RATE_BEATS[(self.v[ids::RATE as usize] as usize).min(RATE_BEATS.len() - 1)];
        let bpm = ctx.bpm.max(0.0);
        // The step and where in it the first frame is, in ms; then a
        // frame's worth at a time.
        let (step, mut since) = if bpm > 0.0 {
            let ms_beat = 60_000.0 / bpm;
            let per_frame = bpm / 60.0 / self.rate as f64;
            let at = first as f64;
            // Song position while playing; free time from the absolute
            // frame while stopped.
            let b = if ctx.playing {
                ctx.beats + at * per_frame
            } else {
                (ctx.frame as f64 + at) * per_frame
            } - f64::from(self.v[ids::NUDGE as usize]) / ms_beat;
            let into = b - floor_i64(b / beats) as f64 * beats;
            (beats * ms_beat, into * ms_beat)
        } else {
            (0.0, 0.0)
        };
        let dp = f64::from(ms_frame);
        // As fast as the cycle itself moves at its steepest (the attack, or
        // the end of the release at its sharpest shape), and at least the
        // whole depth in CATCH_MS.
        let slew_of = |ia: f32, ir: f32| 1.01 * ms_frame * ia.max(4.0 * ir).max(1.0 / CATCH_MS);
        let gliding = !(self.inv_att.settled()
            && self.hold.settled()
            && self.inv_rel.settled()
            && self.shape.settled());
        let (mut inv_att, mut hold, mut inv_rel) =
            (self.inv_att.value, self.hold.value, self.inv_rel.value);
        let mut bend = Bend::new(self.shape.value);
        let mut slew = slew_of(inv_att, inv_rel);
        for i in 0..l.len() {
            let depth = self.depth.next();
            let mix = self.mix.next();
            if gliding {
                inv_att = self.inv_att.next();
                hold = self.hold.next();
                inv_rel = self.inv_rel.next();
                bend = Bend::new(self.shape.next());
                slew = slew_of(inv_att, inv_rel);
            }
            let target = if step > 0.0 {
                let p = since as f32;
                since += dp;
                if since >= step {
                    since -= step;
                }
                tempo_duck(p, step as f32, inv_att, hold, inv_rel, bend)
            } else {
                0.0
            };
            self.env += (target - self.env).clamp(-slew, slew);
            let w = Self::gain(self.env, depth, mix);
            l[i] *= w;
            r[i] *= w;
        }
    }

    /// The key's duck: triggered by the filtered key over the threshold.
    fn run_key(&mut self, l: &mut [f32], r: &mut [f32], key: Option<(&[f32], &[f32])>) {
        let fs = self.rate as f32;
        let att_step = 1.0 / (self.v[4] * 0.001 * fs).max(1.0);
        let hold = (self.v[5] * 0.001 * fs) as u32;
        let rel_step = 1.0 / (self.v[6] * 0.001 * fs).max(1.0);
        let bend = Bend::new(self.v[7] / 100.0);
        let thresh = self.v[2];
        let hp_on = self.v[ids::KEY_HP as usize] > 20.5;
        let lp_on = self.v[ids::KEY_LP as usize] < 19_900.0;
        // The tempo duck's glides aren't heard here.
        for s in [
            &mut self.inv_att,
            &mut self.hold,
            &mut self.inv_rel,
            &mut self.shape,
        ] {
            s.snap(s.target);
        }
        for i in 0..l.len() {
            let depth = self.depth.next();
            let mix = self.mix.next();
            if let Some((kl, kr)) = key {
                let mut k = [kl[i], kr[i]];
                for (ch, x) in k.iter_mut().enumerate() {
                    if hp_on {
                        *x = self.key_hp.run(ch, *x);
                    }
                    if lp_on {
                        *x = self.key_lp.run(ch, *x);
                    }
                }
                let x = k[0].abs().max(k[1].abs());
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
                    self.stage = Stage::Attack;
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
                        self.env = self.rel_from * bend.at(self.rel_t);
                    }
                }
            }
            let w = Self::gain(self.env, depth, mix);
            l[i] *= w;
            r[i] *= w;
        }
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
        for s in [
            &mut self.inv_att,
            &mut self.hold,
            &mut self.inv_rel,
            &mut self.shape,
        ] {
            s.set_time(self.rate, GLIDE_MS);
        }
        // The key follower falls 60 dB in 50 ms.
        self.key_fall = (-6.9 / (0.05 * self.rate as f32)).exp();
        self.set_key_filters();
    }
    fn reset(&mut self) {
        for s in [
            &mut self.depth,
            &mut self.mix,
            &mut self.inv_att,
            &mut self.hold,
            &mut self.inv_rel,
            &mut self.shape,
        ] {
            s.snap(s.target);
        }
        self.stage = Stage::Idle;
        self.env = 0.0;
        self.rel_t = 0.0;
        self.rel_from = 0.0;
        self.key_hp.reset();
        self.key_lp.reset();
        self.key = 0.0;
        self.over = false;
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
    fn floor_i64_is_floor() {
        for x in [
            0.0, 0.5, 0.999_999, 1.0, -0.0, -0.25, -1.0, -1.5, 7.0, 1e9, -1e9, 3e9, -3e9,
        ] {
            assert_eq!(floor_i64(x), x.floor() as i64, "{x}");
        }
    }

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

    #[test]
    fn the_tempo_cycle_is_down_on_the_beat_and_continuous() {
        // A 125 ms step, 2 ms attack, 20 ms hold, 180 ms release: the
        // release never finishes, and the attack meets it.
        let (step, ia, h, ir, s) = (125.0, 0.5, 20.0, 1.0 / 180.0, Bend::new(0.5));
        assert_eq!(tempo_duck(0.0, step, ia, h, ir, s), 1.0);
        assert_eq!(tempo_duck(19.0, step, ia, h, ir, s), 1.0);
        assert!(tempo_duck(100.0, step, ia, h, ir, s) < 1.0);
        assert!((tempo_duck(step, step, ia, h, ir, s) - 1.0).abs() < 1e-6);
        let mut last = tempo_duck(0.0, step, ia, h, ir, s);
        let mut p = 0.0;
        while p < step {
            p += 0.01;
            let v = tempo_duck(p, step, ia, h, ir, s);
            assert!((v - last).abs() <= 0.01 * 0.5 * 1.001, "a step at {p}");
            last = v;
        }
    }
}
