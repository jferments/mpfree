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
//! - Repeat ([`param::REPEAT`], R7 step C2) starts on the next line of its
//!   own length on the transport's beat grid (at once while the transport
//!   is stopped) and ends on the frame it is set Off; a new length while
//!   it repeats rolls the slice from the next line of the new length.

use mpfree_node::{
    Ctx, Curve, Held, HeldStep, Kind, Node, NodeEvent, NodeInfo, ParamInfo, Timed, Unit,
};

use crate::filter::{FilterKind, GrainFilter, CUTOFF_MAX_HZ, CUTOFF_MIN_HZ};
use crate::fx::GrainFx;
use crate::voice::{Budget, Params, SprayShape};
use crate::window::Window;

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
    /// How random the gaps between grains are, per cent (added after the
    /// meter, so the older ids keep their numbers).
    pub const TIMING: u16 = 11;
    /// The grains' shape, per cent: square (0) to Hann (60) to a swell
    /// (80) and a pluck (100), as `Window::Shape`.
    pub const SHAPE: u16 = 12;
    /// How each grain's length varies, per cent (100: half to twice).
    pub const SIZE_SPRAY: u16 = 13;
    /// How each grain's level varies, per cent (100: down to 24 dB less).
    pub const LEVEL_SPRAY: u16 = 14;
    /// Spray peaked at the position (on) or even across its range (off).
    pub const PEAKED: u16 = 15;
    /// How much of the grains goes back into the memory, per cent: trails
    /// that build, and at 100 an endless cloud ([`crate::GrainFx::set_feedback`]).
    pub const FEEDBACK: u16 = 16;
    /// A synced stutter of the last stretch of the input: Off, or how long
    /// each repeat is, a half note to a 32nd ([`super::REPEAT_BEATS`],
    /// [`crate::GrainFx::start_repeat`]).
    pub const REPEAT: u16 = 17;
    /// Each grain's filter: Off, Low pass, Band pass or High pass
    /// ([`super::FILTER_NAMES`], [`crate::GrainFilter`]).
    pub const FILTER: u16 = 18;
    /// The filter's cutoff, Hz.
    pub const CUTOFF: u16 = 19;
    /// The filter's resonance, per cent (Q 0.71 to 12).
    pub const RESONANCE: u16 = 20;
    /// How far each grain's cutoff lands from the cutoff, per cent (100:
    /// three octaves either way).
    pub const FILTER_SPRAY: u16 = 21;
}

/// Parameters, counting the meter's id.
const COUNT: usize = 22;

/// The filter's choices, as the page shows them.
pub const FILTER_NAMES: &[&str] = &["Off", "Low pass", "Band pass", "High pass"];

/// Repeat's choices, as the page shows them.
pub const REPEAT_NAMES: &[&str] = &["Off", "1/2", "1/4", "1/8", "1/16", "1/32"];
/// Each choice's length in beats (Off: none).
pub const REPEAT_BEATS: [f64; 6] = [0.0, 2.0, 1.0, 0.5, 0.25, 0.125];

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

static PARAMS: [ParamInfo; COUNT] = [
    p(
        param::MIX,
        "Mix",
        "Mix",
        0.0,
        100.0,
        50.0,
        Unit::Percent,
        Curve::Linear,
    ),
    p(
        param::FREEZE,
        "Freeze",
        "Mix",
        0.0,
        1.0,
        0.0,
        Unit::None,
        Curve::Toggle,
    ),
    p(
        param::POSITION,
        "Position",
        "Grains",
        0.0,
        100.0,
        85.0,
        Unit::Percent,
        Curve::Linear,
    ),
    p(
        param::SPRAY,
        "Spray",
        "Grains",
        0.0,
        100.0,
        10.0,
        Unit::Percent,
        Curve::Linear,
    ),
    p(
        param::PEAKED,
        "Peaked spray",
        "Grains",
        0.0,
        1.0,
        0.0,
        Unit::None,
        Curve::Toggle,
    ),
    p(
        param::TIMING,
        "Timing",
        "Grains",
        0.0,
        100.0,
        30.0,
        Unit::Percent,
        Curve::Linear,
    ),
    p(
        param::SHAPE,
        "Shape",
        "Grains",
        0.0,
        100.0,
        60.0,
        Unit::Percent,
        Curve::Linear,
    ),
    p(
        param::SIZE,
        "Size",
        "Grains",
        2.0,
        2000.0,
        120.0,
        Unit::Ms,
        Curve::Log,
    ),
    p(
        param::SIZE_SPRAY,
        "Size spray",
        "Grains",
        0.0,
        100.0,
        0.0,
        Unit::Percent,
        Curve::Linear,
    ),
    p(
        param::LEVEL_SPRAY,
        "Level spray",
        "Grains",
        0.0,
        100.0,
        0.0,
        Unit::Percent,
        Curve::Linear,
    ),
    p(
        param::DENSITY,
        "Density",
        "Grains",
        0.5,
        200.0,
        20.0,
        Unit::Hz,
        Curve::Log,
    ),
    p(
        param::PITCH,
        "Pitch",
        "Pitch",
        -24.0,
        24.0,
        0.0,
        Unit::Semitones,
        Curve::Linear,
    ),
    p(
        param::PITCH_SPRAY,
        "Pitch spray",
        "Pitch",
        0.0,
        24.0,
        0.0,
        Unit::Semitones,
        Curve::Linear,
    ),
    p(
        param::REVERSE,
        "Reverse",
        "Pitch",
        0.0,
        100.0,
        0.0,
        Unit::Percent,
        Curve::Linear,
    ),
    p(
        param::SPREAD,
        "Spread",
        "Space",
        0.0,
        100.0,
        60.0,
        Unit::Percent,
        Curve::Linear,
    ),
    p(
        param::FEEDBACK,
        "Feedback",
        "Mix",
        0.0,
        100.0,
        0.0,
        Unit::Percent,
        Curve::Linear,
    ),
    p(
        param::REPEAT,
        "Repeat",
        "Mix",
        0.0,
        5.0,
        0.0,
        Unit::None,
        Curve::Choice(REPEAT_NAMES),
    ),
    p(
        param::FILTER,
        "Filter",
        "Filter",
        0.0,
        3.0,
        0.0,
        Unit::None,
        Curve::Choice(FILTER_NAMES),
    ),
    p(
        param::CUTOFF,
        "Cutoff",
        "Filter",
        CUTOFF_MIN_HZ,
        CUTOFF_MAX_HZ,
        2000.0,
        Unit::Hz,
        Curve::Log,
    ),
    p(
        param::RESONANCE,
        "Resonance",
        "Filter",
        0.0,
        100.0,
        0.0,
        Unit::Percent,
        Curve::Linear,
    ),
    p(
        param::FILTER_SPRAY,
        "Filter spray",
        "Filter",
        0.0,
        100.0,
        0.0,
        Unit::Percent,
        Curve::Linear,
    ),
    ParamInfo {
        id: param::GRAINS,
        name: "Grains",
        group: "Meter",
        min: 0.0,
        max: 64.0,
        default: 0.0,
        unit: Unit::None,
        curve: Curve::Stepped,
        readonly: true,
    },
];

/// The grain settings the effect plays with these parameter values, by id.
fn params_of(v: &[f32; COUNT]) -> Params {
    Params {
        position: v[usize::from(param::POSITION)] / 100.0,
        spray: v[usize::from(param::SPRAY)] / 100.0,
        size_ms: v[usize::from(param::SIZE)],
        density: v[usize::from(param::DENSITY)],
        pitch: v[usize::from(param::PITCH)],
        pitch_spray: v[usize::from(param::PITCH_SPRAY)],
        reverse: v[usize::from(param::REVERSE)] / 100.0,
        spread: v[usize::from(param::SPREAD)] / 100.0,
        timing: v[usize::from(param::TIMING)] / 100.0,
        size_spray: v[usize::from(param::SIZE_SPRAY)] / 100.0,
        level_spray: v[usize::from(param::LEVEL_SPRAY)] / 100.0,
        spray_shape: if v[usize::from(param::PEAKED)] >= 0.5 {
            SprayShape::Peaked
        } else {
            SprayShape::Even
        },
        // 60 % is Hann, kept as such so the default plays as it did.
        window: match v[usize::from(param::SHAPE)] {
            60.0 => Window::Hann,
            shape => Window::Shape {
                shape: shape / 100.0,
            },
        },
        filter: GrainFilter {
            kind: match v[usize::from(param::FILTER)].round() as i32 {
                1 => FilterKind::Low,
                2 => FilterKind::Band,
                3 => FilterKind::High,
                _ => FilterKind::Off,
            },
            cutoff_hz: v[usize::from(param::CUTOFF)],
            resonance: v[usize::from(param::RESONANCE)] / 100.0,
            spray: v[usize::from(param::FILTER_SPRAY)] / 100.0,
        },
        ..Params::default()
    }
}

/// The grain settings a pad plays to sound as the effect did with these
/// parameter values (id, value; any left out keep their defaults): what
/// [`Held::keep_values`] kept when the effect's memory was captured.
pub fn voice_params(values: &[(u16, f32)]) -> Params {
    let mut v = [0.0; COUNT];
    for p in &PARAMS {
        v[usize::from(p.id)] = p.default;
    }
    for &(id, x) in values {
        if let Some(p) = PARAMS.iter().find(|p| p.id == id && !p.readonly) {
            v[usize::from(id)] = p.clamp(x);
        }
    }
    params_of(&v)
}

static INFO: NodeInfo = NodeInfo {
    id: "mpfree.grains",
    name: "Grains",
    kind: Kind::Effect,
};

/// The granular effect as a node. Made with [`GrainNode::new`]; silent
/// (passes its input through) until [`Node::prepare`] gives it memory.
pub struct GrainNode {
    fx: Option<GrainFx>,
    budget: Budget,
    /// Every parameter's value, by id (the meter's slot is unused).
    values: [f32; COUNT],
    rate: u32,
    seed: u64,
    /// A repeat waiting for its line on the grid: its length in beats.
    armed: Option<f64>,
}

impl GrainNode {
    /// Seconds of input the grains can reach back into.
    pub const MEMORY_S: f32 = 8.0;
    /// Grains this node may play at once.
    pub const BUDGET: usize = 64;

    /// A node with the default settings. `seed` sets its random sequence,
    /// so the same input renders the same.
    pub fn new(seed: u64) -> GrainNode {
        let mut values = [0.0; COUNT];
        for p in &PARAMS {
            values[usize::from(p.id)] = p.default;
        }
        GrainNode {
            fx: None,
            budget: Budget::new(Self::BUDGET),
            values,
            rate: 0,
            seed,
            armed: None,
        }
    }

    /// The effect inside, for drawing its memory, grains and playhead.
    pub fn fx(&self) -> Option<&GrainFx> {
        self.fx.as_ref()
    }

    fn grain_params(&self) -> Params {
        params_of(&self.values)
    }

    /// Hands every setting to the effect.
    fn apply_all(&mut self) {
        let p = self.grain_params();
        let (mix, frozen, feedback) = (
            self.values[usize::from(param::MIX)] / 100.0,
            self.values[usize::from(param::FREEZE)] >= 0.5,
            self.values[usize::from(param::FEEDBACK)] / 100.0,
        );
        if let Some(fx) = &mut self.fx {
            fx.set_params(p);
            fx.set_mix(mix);
            fx.set_frozen(frozen);
            fx.set_feedback(feedback);
        }
        self.arm();
    }

    /// The repeat's length in beats, if Repeat is on.
    fn repeat_beats(&self) -> Option<f64> {
        let i = self.values[usize::from(param::REPEAT)] as usize;
        REPEAT_BEATS.get(i).copied().filter(|&b| b > 0.0)
    }

    /// Repeat set: wait for its line on the grid; Off: stop now.
    fn arm(&mut self) {
        self.armed = self.repeat_beats();
        if self.armed.is_none() {
            if let Some(fx) = &mut self.fx {
                fx.stop_repeat();
            }
        }
    }

    /// Runs frames `from..to` of the block, starting an armed repeat on its
    /// line: the first frame at or after the next multiple of its length
    /// in beats (at once while the transport is stopped).
    fn run(&mut self, ctx: &Ctx, from: usize, to: usize, left: &mut [f32], right: &mut [f32]) {
        let Some(fx) = &mut self.fx else { return };
        let fpb = if ctx.bpm.is_finite() && ctx.bpm > 0.0 {
            60.0 * f64::from(self.rate) / ctx.bpm
        } else {
            0.5 * f64::from(self.rate)
        };
        fx.set_tempo(fpb);
        let mut at = from;
        if let Some(beats) = self.armed {
            let wait = if ctx.playing {
                // Beats at frame `at`, and the frames to the next line
                // (a line within a millionth of a beat counts as here).
                let b = ctx.beats + at as f64 / fpb;
                let line = (b / beats - 1e-6).ceil() * beats;
                ((line - b) * fpb - 1e-6).ceil().max(0.0) as usize
            } else {
                0
            };
            if at + wait < to {
                let k = at + wait;
                fx.process(
                    &mut self.budget,
                    ctx.frame + at as u64,
                    &mut left[at..k],
                    &mut right[at..k],
                );
                at = k;
                fx.start_repeat(beats);
                self.armed = None;
            }
        }
        if at < to {
            fx.process(
                &mut self.budget,
                ctx.frame + at as u64,
                &mut left[at..to],
                &mut right[at..to],
            );
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
        let Some(info) = PARAMS.iter().find(|p| p.id == id) else {
            return;
        };
        if info.readonly {
            return;
        }
        let v = info.clamp(value);
        let was = self.values[usize::from(id)];
        self.values[usize::from(id)] = v;
        if id == param::REPEAT {
            if v != was {
                self.arm();
            }
            return;
        }
        let p = self.grain_params();
        let Some(fx) = &mut self.fx else { return };
        match id {
            param::MIX => fx.set_mix(v / 100.0),
            param::FREEZE => fx.set_frozen(v >= 0.5),
            param::FEEDBACK => fx.set_feedback(v / 100.0),
            _ => fx.set_params(p),
        }
    }

    fn param(&self, id: u16) -> f32 {
        match id {
            param::GRAINS => self.fx.as_ref().map_or(0.0, |fx| fx.live() as f32),
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

    /// A repeat ends; with Repeat still on, the next starts on its line.
    fn reset(&mut self) {
        if let Some(fx) = &mut self.fx {
            fx.reset(&mut self.budget);
        }
        self.armed = self.repeat_beats();
    }

    fn process(&mut self, ctx: &Ctx, events: &[Timed], left: &mut [f32], right: &mut [f32]) {
        let n = left.len().min(right.len());
        let mut at = 0;
        for e in events {
            let o = (e.offset as usize).min(n);
            if o > at {
                self.run(ctx, at, o, left, right);
                at = o;
            }
            if let NodeEvent::Param { id, value } = e.ev {
                self.set_param(id, value);
            }
        }
        if at < n {
            self.run(ctx, at, n, left, right);
        }
    }

    /// Repeating (or about to), frozen, or with feedback of 99 % or more, it
    /// plays on for as long as it is called; otherwise its grains can reach
    /// back the whole memory, plus one grain, once for each time round the
    /// feedback loop it takes to fall 60 dB. Fully dry, nothing.
    fn tail_frames(&self) -> u32 {
        let Some(fx) = &self.fx else { return 0 };
        let feedback = self.values[usize::from(param::FEEDBACK)] / 100.0;
        if fx.repeating() || self.armed.is_some() {
            // A repeat plays at any Mix.
            u32::MAX
        } else if self.values[usize::from(param::MIX)] <= 0.0 {
            0
        } else if fx.frozen() || feedback >= 0.99 {
            u32::MAX
        } else {
            let once = Self::MEMORY_S + self.values[usize::from(param::SIZE)] / 1000.0;
            let rounds = if feedback > 0.0 {
                1.0 + 3.0 / -feedback.log10()
            } else {
                1.0
            };
            (once * rounds * self.rate as f32)
                .min(u32::MAX as f32 - 1.0)
                .ceil() as u32
        }
    }

    /// The memory the cloud plays from ([`GrainFx::copy_held`]), for a
    /// capture onto a pad; [`voice_params`] turns the values the host
    /// keeps with it into the pad's grain settings.
    fn copy_held(&mut self, into: &mut Held, max_frames: usize) -> HeldStep {
        self.fx
            .as_ref()
            .map_or(HeldStep::Nothing, |fx| fx.copy_held(into, max_frames))
    }
}
