//! Crush: bit-depth and sample-rate reduction. The aliasing and
//! quantisation noise are the point, so nothing here is band-limited.

use mpfree_node::{Ctx, Curve, Kind, Node, NodeEvent, NodeInfo, ParamInfo, Smooth, Timed, Unit};

use crate::svf::Segs;
use crate::{defaults, Rng};

/// [`Crush`] parameter ids.
pub mod ids {
    pub const BITS: u16 = 0;
    pub const RATE: u16 = 1;
    pub const DITHER: u16 = 2;
    pub const MIX: u16 = 3;
}

static INFO: NodeInfo = NodeInfo {
    id: "mpfree.fx.crush",
    name: "Crush",
    kind: Kind::Effect,
};
static PARAMS: [ParamInfo; 4] = [
    ParamInfo {
        id: 0,
        name: "Bits",
        group: "Crush",
        min: 1.0,
        max: 16.0,
        default: 8.0,
        unit: Unit::None,
        curve: Curve::Linear,
        readonly: false,
    },
    ParamInfo {
        id: 1,
        name: "Rate",
        group: "Crush",
        min: 200.0,
        max: 48_000.0,
        default: 11_025.0,
        unit: Unit::Hz,
        curve: Curve::Log,
        readonly: false,
    },
    ParamInfo {
        id: 2,
        name: "Dither",
        group: "Crush",
        min: 0.0,
        max: 1.0,
        default: 0.0,
        unit: Unit::None,
        curve: Curve::Toggle,
        readonly: false,
    },
    ParamInfo {
        id: 3,
        name: "Mix",
        group: "Crush",
        min: 0.0,
        max: 100.0,
        default: 100.0,
        unit: Unit::Percent,
        curve: Curve::Linear,
        readonly: false,
    },
];

/// Bit-depth and sample-rate reduction with optional TPDF dither.
pub struct Crush {
    v: [f32; 4],
    rate: u32,
    bits: Smooth,
    hz: Smooth,
    mix: Smooth,
    phase: f32,
    held: [f32; 2],
    rng: Rng,
}

impl Default for Crush {
    fn default() -> Self {
        Self::new()
    }
}

impl Crush {
    pub fn new() -> Crush {
        let v = defaults(&PARAMS);
        let mut c = Crush {
            v,
            rate: 48_000,
            bits: Smooth::new(v[0]),
            hz: Smooth::new(v[1]),
            mix: Smooth::new(v[3] / 100.0),
            phase: 1.0,
            held: [0.0; 2],
            rng: Rng::new(0x5EED),
        };
        c.prepare(48_000, mpfree_node::MAX_BLOCK);
        c
    }

    fn apply(&mut self, id: u16, value: f32) {
        let Some(p) = PARAMS.get(id as usize) else {
            return;
        };
        let v = p.clamp(value);
        self.v[id as usize] = v;
        match id {
            ids::BITS => self.bits.target = v,
            ids::RATE => self.hz.target = v,
            ids::MIX => self.mix.target = v / 100.0,
            _ => {}
        }
    }

    fn run(&mut self, l: &mut [f32], r: &mut [f32]) {
        let dither = self.v[2] >= 0.5;
        let per_frame = 1.0 / self.rate as f32;
        // The levels for the bit depth, worked out again only while it
        // glides (exp2 per sample is a library call on the MPC's ARM).
        let (mut bits_was, mut levels) = (f32::NAN, 1.0);
        for i in 0..l.len() {
            let bits = self.bits.next();
            if bits != bits_was {
                bits_was = bits;
                levels = (bits - 1.0).exp2();
            }
            let step = (self.hz.next() * per_frame).min(1.0);
            let mix = self.mix.next();
            self.phase += step;
            let x = [l[i], r[i]];
            if self.phase >= 1.0 {
                self.phase -= 1.0;
                for (held, &v) in self.held.iter_mut().zip(&x) {
                    let d = if dither {
                        (self.rng.bipolar() + self.rng.bipolar()) * 0.5 / levels
                    } else {
                        0.0
                    };
                    *held = ((v + d) * levels).round() / levels;
                }
            }
            l[i] = crate::lerp(x[0], self.held[0], mix);
            r[i] = crate::lerp(x[1], self.held[1], mix);
        }
    }
}

impl Node for Crush {
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
        self.bits.set_time(self.rate, 20.0);
        self.hz.set_time(self.rate, 20.0);
        self.mix.set_time(self.rate, 20.0);
    }
    fn reset(&mut self) {
        self.bits.snap(self.bits.target);
        self.hz.snap(self.hz.target);
        self.mix.snap(self.mix.target);
        self.phase = 1.0;
        self.held = [0.0; 2];
        self.rng = Rng::new(0x5EED);
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
