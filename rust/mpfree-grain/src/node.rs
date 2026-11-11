//! The granular effect as an [`mpfree_node::Node`], so it can sit in any FX
//! chain: an insert, a send return or the master chain.
//!
//! [`GrainNode`] wraps a [`GrainFx`] with [`GrainNode::MEMORY_S`] seconds of
//! memory and its own grain [`Budget`] of [`GrainNode::BUDGET`] grains, so a
//! host can cost it per slot. Its parameters are in plain units (per cent,
//! ms, semitones); freeze is a toggle, so a sequencer can lock it on an exact
//! frame, and the live grain count is a meter.
//!
//! The rules the node interface asks for, as this node keeps them:
//!
//! - `prepare` allocates the memory (2.8 MB at 44.1 kHz); `process`,
//!   `set_param` and `reset` never allocate, free, lock or panic.
//! - The output is the same however the host cuts the frames into blocks,
//!   to within float rounding of the grains' read positions (about 1e-4,
//!   as for a lone grain voice); parameter events land on their exact frame.
//! - The mix glides over 10 ms; grain settings change from the next grain,
//!   and every grain is windowed, so nothing clicks.
//! - Out-of-range values are clamped.

use mpfree_node::{Ctx, Curve, Kind, Node, NodeEvent, NodeInfo, ParamInfo, Timed, Unit};

use crate::fx::GrainFx;
use crate::voice::{Budget, Params};

/// Parameter ids, stable for project files and automation.
pub mod param {
    pub const MIX: u16 = 0;
    pub const FREEZE: u16 = 1;
    pub const POSITION: u16 = 2;
    pub const SPRAY: u16 = 3;
    pub const SIZE: u16 = 4;
    pub const DENSITY: u16 = 5;
    pub const PITCH: u16 = 6;
    pub const PITCH_SPRAY: u16 = 7;
    pub const REVERSE: u16 = 8;
    pub const SPREAD: u16 = 9;
    /// A meter: grains playing now.
    pub const GRAINS: u16 = 10;
}

#[allow(clippy::too_many_arguments)]
const fn p(id: u16, name: &'static str, group: &'static str, min: f32, max: f32, default: f32, unit: Unit, curve: Curve) -> ParamInfo {
    ParamInfo { id, name, group, min, max, default, unit, curve, readonly: false }
}

static PARAMS: [ParamInfo; 11] = [
    p(param::MIX, "Mix", "Mix", 0.0, 100.0, 50.0, Unit::Percent, Curve::Linear),
    p(param::FREEZE, "Freeze", "Mix", 0.0, 1.0, 0.0, Unit::None, Curve::Toggle),
    p(param::POSITION, "Position", "Grains", 0.0, 100.0, 85.0, Unit::Percent, Curve::Linear),
    p(param::SPRAY, "Spray", "Grains", 0.0, 100.0, 10.0, Unit::Percent, Curve::Linear),
    p(param::SIZE, "Size", "Grains", 2.0, 2000.0, 120.0, Unit::Ms, Curve::Log),
    p(param::DENSITY, "Density", "Grains", 0.5, 200.0, 20.0, Unit::Hz, Curve::Log),
    p(param::PITCH, "Pitch", "Pitch", -24.0, 24.0, 0.0, Unit::Semitones, Curve::Linear),
    p(param::PITCH_SPRAY, "Pitch spray", "Pitch", 0.0, 24.0, 0.0, Unit::Semitones, Curve::Linear),
    p(param::REVERSE, "Reverse", "Pitch", 0.0, 100.0, 0.0, Unit::Percent, Curve::Linear),
    p(param::SPREAD, "Spread", "Space", 0.0, 100.0, 60.0, Unit::Percent, Curve::Linear),
    ParamInfo { id: param::GRAINS, name: "Grains", group: "Meter", min: 0.0, max: 64.0, default: 0.0, unit: Unit::None, curve: Curve::Stepped, readonly: true },
];

static INFO: NodeInfo = NodeInfo { id: "mpfree.grains", name: "Grains", kind: Kind::Effect };

/// The granular effect as a node. Made with [`GrainNode::new`]; silent
/// (passes its input through) until [`Node::prepare`] gives it memory.
pub struct GrainNode {
    fx: Option<GrainFx>,
    budget: Budget,
    /// Every parameter's value, by id (the meter's slot is unused).
    values: [f32; 11],
    rate: u32,
    seed: u64,
}

impl GrainNode {
    /// Seconds of input the grains can reach back into.
    pub const MEMORY_S: f32 = 8.0;
    /// Grains this node may play at once.
    pub const BUDGET: usize = 64;

    /// A node with the default settings. `seed` sets its random sequence,
    /// so the same input renders the same.
    pub fn new(seed: u64) -> GrainNode {
        let mut values = [0.0; 11];
        for p in &PARAMS {
            values[usize::from(p.id)] = p.default;
        }
        GrainNode { fx: None, budget: Budget::new(Self::BUDGET), values, rate: 0, seed }
    }

    /// The effect inside, for drawing its memory, grains and playhead.
    pub fn fx(&self) -> Option<&GrainFx> {
        self.fx.as_ref()
    }

    fn grain_params(&self) -> Params {
        let v = &self.values;
        Params {
            position: v[usize::from(param::POSITION)] / 100.0,
            spray: v[usize::from(param::SPRAY)] / 100.0,
            size_ms: v[usize::from(param::SIZE)],
            density: v[usize::from(param::DENSITY)],
            pitch: v[usize::from(param::PITCH)],
            pitch_spray: v[usize::from(param::PITCH_SPRAY)],
            reverse: v[usize::from(param::REVERSE)] / 100.0,
            spread: v[usize::from(param::SPREAD)] / 100.0,
            ..Params::default()
        }
    }

    /// Hands every setting to the effect.
    fn apply_all(&mut self) {
        let p = self.grain_params();
        let (mix, frozen) = (self.values[usize::from(param::MIX)] / 100.0, self.values[usize::from(param::FREEZE)] >= 0.5);
        if let Some(fx) = &mut self.fx {
            fx.set_params(p);
            fx.set_mix(mix);
            fx.set_frozen(frozen);
        }
    }
}

impl Node for GrainNode {
    fn info(&self) -> &'static NodeInfo {
        &INFO
    }

    fn params(&self) -> &'static [ParamInfo] {
        &PARAMS
    }

    fn set_param(&mut self, id: u16, value: f32) {
        let Some(info) = PARAMS.iter().find(|p| p.id == id) else { return };
        if info.readonly {
            return;
        }
        let v = info.clamp(value);
        self.values[usize::from(id)] = v;
        let p = self.grain_params();
        let Some(fx) = &mut self.fx else { return };
        match id {
            param::MIX => fx.set_mix(v / 100.0),
            param::FREEZE => fx.set_frozen(v >= 0.5),
            _ => fx.set_params(p),
        }
    }

    fn param(&self, id: u16) -> f32 {
        match id {
            param::GRAINS => self.fx.as_ref().map_or(0.0, |fx| fx.stats().active as f32),
            _ if usize::from(id) < self.values.len() => self.values[usize::from(id)],
            _ => f32::NAN,
        }
    }

    fn prepare(&mut self, rate: u32, _max_block: usize) {
        if rate == self.rate && self.fx.is_some() {
            return;
        }
        if let Some(old) = &mut self.fx {
            old.kill(&mut self.budget);
        }
        self.fx = GrainFx::new(rate, Self::MEMORY_S, self.seed);
        self.rate = if self.fx.is_some() { rate } else { 0 };
        self.apply_all();
    }

    fn reset(&mut self) {
        if let Some(fx) = &mut self.fx {
            fx.reset(&mut self.budget);
        }
    }

    fn process(&mut self, ctx: &Ctx, events: &[Timed], left: &mut [f32], right: &mut [f32]) {
        let n = left.len().min(right.len());
        let mut at = 0;
        for e in events {
            let o = (e.offset as usize).min(n);
            if o > at {
                if let Some(fx) = &mut self.fx {
                    fx.process(&mut self.budget, ctx.frame + at as u64, &mut left[at..o], &mut right[at..o]);
                }
                at = o;
            }
            if let NodeEvent::Param { id, value } = e.ev {
                self.set_param(id, value);
            }
        }
        if at < n {
            if let Some(fx) = &mut self.fx {
                fx.process(&mut self.budget, ctx.frame + at as u64, &mut left[at..n], &mut right[at..n]);
            }
        }
    }

    /// Frozen, it plays on for as long as it is called; otherwise its grains
    /// can reach back the whole memory, plus one grain. Fully dry, nothing.
    fn tail_frames(&self) -> u32 {
        let Some(fx) = &self.fx else { return 0 };
        if self.values[usize::from(param::MIX)] <= 0.0 {
            0
        } else if fx.frozen() {
            u32::MAX
        } else {
            let s = Self::MEMORY_S + self.values[usize::from(param::SIZE)] / 1000.0;
            (s * self.rate as f32).ceil() as u32
        }
    }
}
