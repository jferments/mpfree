//! Transient shaper: more or less punch at the start of each hit and more
//! or less body after it, without a threshold to set.
//!
//! The stereo-linked peak is held for 25 ms (so a steady tone above 40 Hz
//! reads as a steady level, without ripple), then two pairs of envelope
//! followers run on it, in dB:
//!
//! - **Attack**: a fast follower minus a slow-rising one. The gap opens at
//!   each hit's front and closes as the slow one catches up; Speed sets how
//!   slow it is, so how long the front lasts.
//! - **Sustain**: a slow-falling follower minus a fast-falling one. The gap
//!   opens as a hit decays and is what rings after the front.
//!
//! Each gap times its amount (-100 to +100 %) becomes the gain, limited to
//! ±15 dB. Level doesn't matter, only the shape of the sound, so the same
//! setting works on a quiet and a loud kick.

use mpfree_node::{Ctx, Curve, Kind, Node, NodeEvent, NodeInfo, ParamInfo, Smooth, Timed, Unit};

use crate::svf::Segs;
use crate::{defaults, fast_db_to_gain, fast_level_db};

/// [`TransientShaper`] parameter ids.
pub mod ids {
    pub const ATTACK: u16 = 0;
    pub const SUSTAIN: u16 = 1;
    pub const SPEED: u16 = 2;
    pub const OUTPUT: u16 = 3;
    pub const MIX: u16 = 4;
    /// Meter: the gain now, in dB.
    pub const GAIN: u16 = 5;
}

static INFO: NodeInfo = NodeInfo {
    id: "mpfree.fx.transient",
    name: "Transient shaper",
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
    readonly: bool,
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
        readonly,
    }
}

static PARAMS: [ParamInfo; 6] = [
    p(
        0,
        "Attack",
        "Shape",
        -100.0,
        100.0,
        50.0,
        Unit::Percent,
        Curve::Linear,
        false,
    ),
    p(
        1,
        "Sustain",
        "Shape",
        -100.0,
        100.0,
        0.0,
        Unit::Percent,
        Curve::Linear,
        false,
    ),
    p(
        2,
        "Speed",
        "Shape",
        5.0,
        50.0,
        15.0,
        Unit::Ms,
        Curve::Log,
        false,
    ),
    p(
        3,
        "Output",
        "Output",
        -12.0,
        12.0,
        0.0,
        Unit::Db,
        Curve::Linear,
        false,
    ),
    p(
        4,
        "Mix",
        "Output",
        0.0,
        100.0,
        100.0,
        Unit::Percent,
        Curve::Linear,
        false,
    ),
    p(
        5,
        "Gain",
        "Meters",
        -15.0,
        15.0,
        0.0,
        Unit::Db,
        Curve::Linear,
        true,
    ),
];

/// The most the shaper turns up or down, in dB.
const LIMIT: f32 = 15.0;
/// How many dB of gain a 1 dB envelope gap makes at 100 %.
const SENS: f32 = 0.6;
/// The detector's floor, in dBFS: quieter than this counts as this, so a
/// hit out of silence opens a gap of its level above -60 dB, not above
/// the noise.
const FLOOR: f32 = -60.0;

/// A one-pole follower in dB with separate rise and fall coefficients.
#[derive(Clone, Copy, Debug)]
struct Follow {
    up: f32,
    down: f32,
    y: f32,
}

impl Follow {
    fn new() -> Follow {
        Follow {
            up: 0.0,
            down: 0.0,
            y: FLOOR,
        }
    }
    fn set(&mut self, rate: f32, up_ms: f32, down_ms: f32) {
        let k = |ms: f32| (-1.0 / (ms * 0.001 * rate).max(1.0)).exp();
        self.up = k(up_ms);
        self.down = k(down_ms);
    }
    #[inline]
    fn next(&mut self, x: f32) -> f32 {
        let k = if x > self.y { self.up } else { self.down };
        self.y = x + k * (self.y - x);
        self.y
    }
}

/// A level-independent transient shaper.
pub struct TransientShaper {
    v: [f32; 6],
    rate: u32,
    attack: Smooth,
    sustain: Smooth,
    output: Smooth,
    mix: Smooth,
    fast: Follow,
    slow: Follow,
    hang: Follow,
    drop: Follow,
    /// The held peak, its hold left in frames, the hold length and the
    /// fall after it.
    peak: f32,
    held: u32,
    hold: u32,
    fall: f32,
}

impl Default for TransientShaper {
    fn default() -> Self {
        Self::new()
    }
}

impl TransientShaper {
    pub fn new() -> TransientShaper {
        let v = defaults(&PARAMS);
        let mut t = TransientShaper {
            v,
            rate: 48_000,
            attack: Smooth::new(v[0] / 100.0),
            sustain: Smooth::new(v[1] / 100.0),
            output: Smooth::new(v[3]),
            mix: Smooth::new(v[4] / 100.0),
            fast: Follow::new(),
            slow: Follow::new(),
            hang: Follow::new(),
            drop: Follow::new(),
            peak: 0.0,
            held: 0,
            hold: 0,
            fall: 0.0,
        };
        t.prepare(48_000, mpfree_node::MAX_BLOCK);
        t
    }

    fn times(&mut self) {
        let fs = self.rate as f32;
        let speed = self.v[2];
        self.fast.set(fs, 0.3, speed * 4.0);
        self.slow.set(fs, speed, speed * 4.0);
        self.hang.set(fs, 0.3, speed * 20.0);
        self.drop.set(fs, 0.3, speed * 2.0);
        self.hold = (0.025 * fs) as u32;
        self.fall = (-1.0 / (0.005 * fs)).exp();
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
            ids::ATTACK => self.attack.target = v / 100.0,
            ids::SUSTAIN => self.sustain.target = v / 100.0,
            ids::SPEED => self.times(),
            ids::OUTPUT => self.output.target = v,
            ids::MIX => self.mix.target = v / 100.0,
            _ => {}
        }
    }

    fn run(&mut self, l: &mut [f32], r: &mut [f32]) {
        let mut shaped = 0.0;
        for i in 0..l.len() {
            let attack = self.attack.next();
            let sustain = self.sustain.next();
            let output = self.output.next();
            let mix = self.mix.next();
            let a = l[i].abs().max(r[i].abs());
            if a >= self.peak {
                self.peak = a;
                self.held = self.hold;
            } else if self.held > 0 {
                self.held -= 1;
            } else {
                self.peak = crate::flush(a.max(self.peak * self.fall));
            }
            let x = fast_level_db(self.peak).max(FLOOR);
            let front = (self.fast.next(x) - self.slow.next(x)).max(0.0);
            let body = (self.hang.next(x) - self.drop.next(x)).max(0.0);
            shaped = ((attack * front + sustain * body) * SENS).clamp(-LIMIT, LIMIT);
            let g = fast_db_to_gain(shaped + output);
            let wet = 1.0 + mix * (g - 1.0);
            l[i] *= wet;
            r[i] *= wet;
        }
        if !l.is_empty() {
            self.v[ids::GAIN as usize] = shaped;
        }
    }
}

impl Node for TransientShaper {
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
        for s in [
            &mut self.attack,
            &mut self.sustain,
            &mut self.output,
            &mut self.mix,
        ] {
            s.set_time(self.rate, 20.0);
        }
        self.times();
    }
    fn reset(&mut self) {
        for s in [
            &mut self.attack,
            &mut self.sustain,
            &mut self.output,
            &mut self.mix,
        ] {
            s.snap(s.target);
        }
        for f in [
            &mut self.fast,
            &mut self.slow,
            &mut self.hang,
            &mut self.drop,
        ] {
            f.y = FLOOR;
        }
        self.peak = 0.0;
        self.held = 0;
        self.v[ids::GAIN as usize] = 0.0;
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
}
