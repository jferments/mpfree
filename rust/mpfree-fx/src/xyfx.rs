//! XY FX: one effect at a time, played from two controls at once, as the
//! MPC's XYFX insert plays an effect from its touch pad. X and Y are
//! ordinary parameters (Q-Links 1 and 2 on the first page, automation and
//! step locks like any other), and each effect maps them onto one of the
//! existing effects' parameters: the Filter is the DJ filter, Echo the
//! Delay, Wash the Plate, and so on, so what XY FX plays is exactly what
//! those effects play.
//!
//! **Active** opens the effect with an attack and closes it with a
//! release, the way the MPC's pad does when a finger lands and lifts.
//! It is on by default, so the insert plays from its tiles and Q-Links
//! straight away; a pad holds it on while touched. While it is off an
//! insert-style effect (filter, crush, flanger, phaser, ring mod, gate)
//! fades to the dry sound and then stops running; a send-style effect
//! (the echoes and the wash) stops being fed and rings out, so lifting
//! is a throw.
//!
//! Changing the effect crossfades from the old one to the new over
//! [`FADE_MS`]. Each engine starts from silence when it is picked, so an
//! old echo never comes back. Every engine has no latency, so XY FX has
//! none whatever is picked.
//!
//! Memory: all nine engines are made and prepared up front (nothing may
//! allocate on the audio thread when the effect changes); the two delays
//! and the plate make this about 3 MB at 48 kHz. Cost: the picked engine
//! plus about 20 instructions a frame for the envelope and mix.

use mpfree_node::{Ctx, Curve, Kind, Node, NodeEvent, NodeInfo, ParamInfo, Smooth, Timed, Unit};

use crate::ids::{crush, delay, dj_filter as dj, flanger, phaser, plate, ringmod, tape, tremolo};
use crate::kit::{param, Glide};
use crate::svf::Segs;
use crate::{defaults, registry, Preset};

/// [`XyFx`] parameter ids. X and Y come first so Q-Links 1 and 2 play
/// them on the first page.
pub mod ids {
    pub const X: u16 = 0;
    pub const Y: u16 = 1;
    pub const EFFECT: u16 = 2;
    pub const ACTIVE: u16 = 3;
    pub const MIX: u16 = 4;
    pub const ATTACK: u16 = 5;
    pub const RELEASE: u16 = 6;
}
use ids as xy;

/// How long a change of effect crossfades.
pub const FADE_MS: f32 = 30.0;

/// How a pad position (0 to 1) becomes an engine parameter's value.
#[derive(Clone, Copy, Debug)]
enum Map {
    /// Evenly from the first value to the second.
    Lin(f32, f32),
    /// Evenly in ratio (frequencies and times).
    Log(f32, f32),
    /// One of these values, the pad split into that many columns (a
    /// beat division, which the pad draws as columns).
    Steps(&'static [f32]),
}

impl Map {
    fn at(self, pos: f32) -> f32 {
        let pos = if pos.is_nan() {
            0.0
        } else {
            pos.clamp(0.0, 1.0)
        };
        match self {
            Map::Lin(a, b) => a + (b - a) * pos,
            Map::Log(a, b) => a * (b / a).powf(pos),
            Map::Steps(v) => v[step(pos, v.len())],
        }
    }
}

/// The column of `n` that `pos` falls in.
fn step(pos: f32, n: usize) -> usize {
    ((pos * n as f32) as usize).min(n.saturating_sub(1))
}

/// One direction of the pad in one effect: what it is called on the pad,
/// the engine parameter it plays, and how.
#[derive(Clone, Copy, Debug)]
pub struct PadAxis {
    /// The pad's label for this direction ("Feedback").
    pub name: &'static str,
    /// The unit of the engine value, for showing it ("1.2 kHz").
    pub unit: Unit,
    /// For a stepped direction, each column's name, left to right (or
    /// bottom to top); empty for a smooth one.
    pub steps: &'static [&'static str],
    param: u16,
    map: Map,
}

impl PadAxis {
    /// The engine's value at pad position `pos` (0 to 1).
    pub fn value(&self, pos: f32) -> f32 {
        self.map.at(pos)
    }
    /// The column's name at `pos`, for a stepped direction.
    pub fn step_name(&self, pos: f32) -> Option<&'static str> {
        (!self.steps.is_empty()).then(|| self.steps[step(pos.clamp(0.0, 1.0), self.steps.len())])
    }
}

/// Whether an engine replaces the sound or adds to it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Style {
    /// The engine processes the sound; Active and Mix fade it in over the
    /// dry sound.
    Insert,
    /// The engine (at 100 % wet) is fed the sound while Active, and Mix
    /// sets the level it is added at.
    Send,
}

/// One of the effects, in [`EFFECTS`]' order (named there).
struct Mode {
    engine: &'static str,
    style: Style,
    /// Engine settings that hold while this effect is picked.
    fixed: &'static [(u16, f32)],
    x: PadAxis,
    y: PadAxis,
}

const fn smooth(name: &'static str, unit: Unit, param: u16, map: Map) -> PadAxis {
    PadAxis {
        name,
        unit,
        steps: &[],
        param,
        map,
    }
}

const fn stepped(
    name: &'static str,
    param: u16,
    values: &'static [f32],
    steps: &'static [&'static str],
) -> PadAxis {
    PadAxis {
        name,
        unit: Unit::None,
        steps,
        param,
        map: Map::Steps(values),
    }
}

/// The Delay's sync choices the Echo's columns use (1/32 to 1/2).
const ECHO_SYNCS: [f32; 8] = [1.0, 3.0, 4.0, 6.0, 7.0, 9.0, 10.0, 11.0];
const ECHO_NAMES: [&str; 8] = ["1/32", "1/16", "1/16D", "1/8", "1/8D", "1/4", "1/4D", "1/2"];
/// The Tremolo's sync choices the Gate's columns use (1/32 to 1/4).
const GATE_SYNCS: [f32; 6] = [1.0, 2.0, 3.0, 4.0, 5.0, 8.0];
const GATE_NAMES: [&str; 6] = ["1/32", "1/16T", "1/16", "1/8T", "1/8", "1/4"];

static MODES: [Mode; 9] = [
    // Filter
    Mode {
        engine: "mpfree.fx.dj_filter",
        style: Style::Insert,
        fixed: &[],
        x: smooth(
            "Low-pass · high-pass",
            Unit::Percent,
            dj::POSITION,
            Map::Lin(-100.0, 100.0),
        ),
        y: smooth(
            "Resonance",
            Unit::Percent,
            dj::RESONANCE,
            Map::Lin(0.0, 90.0),
        ),
    },
    // Echo
    Mode {
        engine: "mpfree.fx.delay",
        style: Style::Send,
        fixed: &[
            (delay::MIX, 100.0),
            (delay::PING_PONG, 1.0),
            (delay::TONE, 6000.0),
            (delay::SATURATION, 25.0),
            (delay::FREEZE, 0.0),
        ],
        x: stepped("Time", delay::SYNC, &ECHO_SYNCS, &ECHO_NAMES),
        y: smooth(
            "Feedback",
            Unit::Percent,
            delay::FEEDBACK,
            Map::Lin(0.0, 95.0),
        ),
    },
    // Dub echo
    Mode {
        engine: "mpfree.fx.tape_delay",
        style: Style::Send,
        fixed: &[
            (tape::SYNC, 0.0),
            (tape::MIX, 100.0),
            (tape::HEADS, 0.0),
            (tape::WOW, 30.0),
            (tape::SATURATION, 55.0),
            (tape::TONE, 3500.0),
        ],
        x: smooth("Time", Unit::Ms, tape::TIME, Map::Log(60.0, 900.0)),
        y: smooth(
            "Feedback",
            Unit::Percent,
            tape::FEEDBACK,
            Map::Lin(20.0, 105.0),
        ),
    },
    // Wash
    Mode {
        engine: "mpfree.fx.plate",
        style: Style::Send,
        fixed: &[
            (plate::MIX, 100.0),
            (plate::PREDELAY, 25.0),
            (plate::MOD, 55.0),
            (plate::LOW_CUT, 150.0),
        ],
        x: smooth("Decay", Unit::Ms, plate::DECAY, Map::Log(700.0, 12_000.0)),
        y: smooth(
            "Brightness",
            Unit::Hz,
            plate::TONE,
            Map::Log(1500.0, 16_000.0),
        ),
    },
    // Crush
    Mode {
        engine: "mpfree.fx.crush",
        style: Style::Insert,
        fixed: &[(crush::MIX, 100.0), (crush::DITHER, 0.0)],
        x: smooth(
            "Sample rate",
            Unit::Hz,
            crush::RATE,
            Map::Log(22_050.0, 700.0),
        ),
        y: smooth("Bits", Unit::None, crush::BITS, Map::Lin(16.0, 2.0)),
    },
    // Flanger
    Mode {
        engine: "mpfree.fx.flanger",
        style: Style::Insert,
        fixed: &[
            (flanger::DEPTH, 0.0),
            (flanger::SYNC, 0.0),
            (flanger::RATE, 0.02),
            (flanger::WIDTH, 100.0),
            (flanger::MIX, 50.0),
        ],
        x: smooth("Sweep", Unit::Ms, flanger::DELAY, Map::Log(0.2, 8.0)),
        y: smooth(
            "Feedback",
            Unit::Percent,
            flanger::FEEDBACK,
            Map::Lin(0.0, 92.0),
        ),
    },
    // Phaser
    Mode {
        engine: "mpfree.fx.phaser",
        style: Style::Insert,
        fixed: &[
            (phaser::DEPTH, 0.0),
            (phaser::SYNC, 0.0),
            (phaser::STAGES, 3.0),
            (phaser::WIDTH, 100.0),
            (phaser::MIX, 50.0),
        ],
        x: smooth("Sweep", Unit::Hz, phaser::CENTER, Map::Log(150.0, 6000.0)),
        y: smooth(
            "Feedback",
            Unit::Percent,
            phaser::FEEDBACK,
            Map::Lin(0.0, 90.0),
        ),
    },
    // Ring mod
    Mode {
        engine: "mpfree.fx.ringmod",
        style: Style::Insert,
        fixed: &[(ringmod::WAVE, 0.0), (ringmod::SWEEP, 0.0)],
        x: smooth("Carrier", Unit::Hz, ringmod::FREQ, Map::Log(30.0, 3000.0)),
        y: smooth("Amount", Unit::Percent, ringmod::MIX, Map::Lin(0.0, 100.0)),
    },
    // Gate
    Mode {
        engine: "mpfree.fx.tremolo",
        style: Style::Insert,
        fixed: &[
            (tremolo::SHAPE, 2.0),
            (tremolo::MODE, 0.0),
            (tremolo::WIDTH, 0.0),
        ],
        x: stepped("Rate", tremolo::SYNC, &GATE_SYNCS, &GATE_NAMES),
        y: smooth("Depth", Unit::Percent, tremolo::DEPTH, Map::Lin(0.0, 100.0)),
    },
];

/// The effects' names, in the Effect parameter's order.
pub const EFFECTS: &[&str] = &[
    "Filter", "Echo", "Dub echo", "Wash", "Crush", "Flanger", "Phaser", "Ring mod", "Gate",
];

/// The node id of the effect that plays effect `effect` (an index into
/// [`EFFECTS`]), for the MPC name table and the docs.
pub fn engine(effect: usize) -> &'static str {
    MODES[effect.min(MODES.len() - 1)].engine
}

/// What X and Y play in effect `effect`, for the pad's labels.
pub fn axes(effect: usize) -> (&'static PadAxis, &'static PadAxis) {
    let m = &MODES[effect.min(MODES.len() - 1)];
    (&m.x, &m.y)
}

static INFO: NodeInfo = NodeInfo {
    id: "mpfree.fx.xy",
    name: "XY FX",
    kind: Kind::Effect,
};

static PARAMS: [ParamInfo; 7] = [
    param(
        0,
        "X",
        "Pad",
        0.0,
        100.0,
        50.0,
        Unit::Percent,
        Curve::Linear,
    ),
    param(
        1,
        "Y",
        "Pad",
        0.0,
        100.0,
        25.0,
        Unit::Percent,
        Curve::Linear,
    ),
    param(
        2,
        "Effect",
        "Pad",
        0.0,
        8.0,
        0.0,
        Unit::None,
        Curve::Choice(EFFECTS),
    ),
    param(3, "Active", "Pad", 0.0, 1.0, 1.0, Unit::None, Curve::Toggle),
    param(
        4,
        "Mix",
        "Pad",
        0.0,
        100.0,
        100.0,
        Unit::Percent,
        Curve::Linear,
    ),
    param(5, "Attack", "Touch", 1.0, 2000.0, 5.0, Unit::Ms, Curve::Log),
    param(
        6,
        "Release",
        "Touch",
        5.0,
        4000.0,
        150.0,
        Unit::Ms,
        Curve::Log,
    ),
];

/// The X position at the middle of column `k` of `n`.
const fn column(k: usize, n: usize) -> f32 {
    (k as f32 + 0.5) / n as f32 * 100.0
}

/// XY FX factory presets: an effect and a place on the pad.
pub static PRESETS: &[Preset] = &[
    Preset {
        name: "Filter sweep",
        values: &[(xy::EFFECT, 0.0), (xy::X, 50.0), (xy::Y, 45.0)],
    },
    Preset {
        name: "Echo throw 1/8D",
        values: &[(xy::EFFECT, 1.0), (xy::X, column(4, 8)), (xy::Y, 45.0)],
    },
    Preset {
        name: "Dub siren",
        values: &[(xy::EFFECT, 2.0), (xy::X, 35.0), (xy::Y, 88.0)],
    },
    Preset {
        name: "Washout",
        values: &[(xy::EFFECT, 3.0), (xy::X, 75.0), (xy::Y, 40.0)],
    },
    Preset {
        name: "Bit crunch",
        values: &[(xy::EFFECT, 4.0), (xy::X, 70.0), (xy::Y, 60.0)],
    },
    Preset {
        name: "Jet flange",
        values: &[(xy::EFFECT, 5.0), (xy::X, 30.0), (xy::Y, 80.0)],
    },
    Preset {
        name: "Phase sweep",
        values: &[(xy::EFFECT, 6.0), (xy::X, 40.0), (xy::Y, 60.0)],
    },
    Preset {
        name: "Robot",
        values: &[(xy::EFFECT, 7.0), (xy::X, 25.0), (xy::Y, 100.0)],
    },
    Preset {
        name: "Trance gate 1/16",
        values: &[(xy::EFFECT, 8.0), (xy::X, column(2, 6)), (xy::Y, 100.0)],
    },
];

/// One effect at a time from two controls, with an attack and release
/// on Active. See the module notes.
pub struct XyFx {
    v: [f32; 7],
    rate: u32,
    /// One engine per effect, in [`EFFECTS`]' order.
    engines: Vec<Box<dyn Node>>,
    /// The effect playing, the one asked for (it takes over at the start
    /// of the next stretch of frames), and the one fading out.
    cur: usize,
    want: usize,
    prev: Option<usize>,
    /// Frames of crossfade left, of `fade_len`.
    fade: u32,
    fade_len: u32,
    /// The Active envelope, 0 to 1, and its steps a frame.
    env: f32,
    up: f32,
    down: f32,
    mix: Smooth,
    /// Per frame of the stretch being processed: the dry input, the
    /// envelope, the mix, and the outgoing effect's output.
    dry: [Vec<f32>; 2],
    old: [Vec<f32>; 2],
    gain: Vec<f32>,
    level: Vec<f32>,
}

impl Default for XyFx {
    fn default() -> Self {
        Self::new()
    }
}

impl XyFx {
    pub fn new() -> XyFx {
        let v = defaults(&PARAMS);
        let engines = MODES
            .iter()
            .map(|m| registry::make(m.engine).expect("XY FX engines are in the registry"))
            .collect();
        let mut x = XyFx {
            v,
            rate: 48_000,
            engines,
            cur: 0,
            want: 0,
            prev: None,
            fade: 0,
            fade_len: 1,
            env: 0.0,
            up: 0.0,
            down: 0.0,
            mix: Smooth::new(v[xy::MIX as usize] / 100.0),
            dry: [Vec::new(), Vec::new()],
            old: [Vec::new(), Vec::new()],
            gain: Vec::new(),
            level: Vec::new(),
        };
        x.timing();
        x.clear();
        x
    }

    fn on(&self) -> bool {
        self.v[xy::ACTIVE as usize] >= 0.5
    }

    fn timing(&mut self) {
        let per_ms = self.rate as f32 / 1000.0;
        self.up = 1.0 / (self.v[xy::ATTACK as usize] * per_ms).max(1.0);
        self.down = 1.0 / (self.v[xy::RELEASE as usize] * per_ms).max(1.0);
        self.fade_len = ((FADE_MS * per_ms) as u32).max(1);
        // Fast enough to follow a hand, slow enough not to zip.
        self.mix.set_time(self.rate, 10.0);
    }

    /// Sets engine `k` up for its effect and starts it from silence.
    fn wake(&mut self, k: usize) {
        let m = &MODES[k];
        let e = &mut self.engines[k];
        for &(id, v) in m.fixed {
            e.set_param(id, v);
        }
        e.set_param(m.x.param, m.x.value(self.v[xy::X as usize] / 100.0));
        e.set_param(m.y.param, m.y.value(self.v[xy::Y as usize] / 100.0));
        e.reset();
    }

    fn apply(&mut self, id: u16, v: f32) {
        let m = &MODES[self.cur];
        match id {
            xy::X => self.engines[self.cur].set_param(m.x.param, m.x.value(v / 100.0)),
            xy::Y => self.engines[self.cur].set_param(m.y.param, m.y.value(v / 100.0)),
            xy::EFFECT => self.want = (v as usize).min(MODES.len() - 1),
            xy::MIX => self.mix.target = v / 100.0,
            xy::ATTACK | xy::RELEASE => self.timing(),
            _ => {}
        }
    }

    /// Back to a fresh start with the current settings: any change of
    /// effect asked for is done, and the envelope sits where Active puts
    /// it.
    fn clear(&mut self) {
        self.cur = self.want;
        self.prev = None;
        self.fade = 0;
        self.wake(self.cur);
        self.env = if self.on() { 1.0 } else { 0.0 };
        self.mix.snap(self.mix.target);
    }

    /// Processes one stretch with no events in it.
    fn run(&mut self, ctx: &Ctx, l: &mut [f32], r: &mut [f32]) {
        let n = l.len().min(r.len()).min(self.gain.len());
        let (l, r) = (&mut l[..n], &mut r[..n]);
        if self.want != self.cur {
            // A change cut short drops the effect that was fading out.
            self.prev = Some(self.cur);
            self.cur = self.want;
            self.wake(self.cur);
            self.fade = self.fade_len;
        }
        let on = self.on();
        // Opening from closed starts an insert from silence, wherever the
        // blocks fall, so the output never depends on how they were cut.
        // (A send keeps running and is never reset here.)
        if on && self.env == 0.0 {
            for k in [Some(self.cur), self.prev].into_iter().flatten() {
                if MODES[k].style == Style::Insert {
                    self.engines[k].reset();
                }
            }
        }
        // Closed and silent: an insert passes the dry sound untouched and
        // needn't run at all.
        let idle = !on && self.env == 0.0;
        let mut env = self.env;
        for i in 0..n {
            env = if on {
                (env + self.up).min(1.0)
            } else {
                (env - self.down).max(0.0)
            };
            self.gain[i] = env;
            self.level[i] = self.mix.glide();
        }
        self.env = env;
        self.dry[0][..n].copy_from_slice(l);
        self.dry[1][..n].copy_from_slice(r);

        let XyFx {
            engines,
            dry,
            old,
            gain,
            level,
            ..
        } = self;
        let (dry_l, dry_r) = (&dry[0][..n], &dry[1][..n]);
        let (gain, level) = (&gain[..n], &level[..n]);
        render(
            engines[self.cur].as_mut(),
            MODES[self.cur].style,
            idle,
            ctx,
            (dry_l, dry_r),
            (gain, level),
            l,
            r,
        );
        if let Some(p) = self.prev {
            let [old_l, old_r] = old;
            let (ol, or) = (&mut old_l[..n], &mut old_r[..n]);
            render(
                engines[p].as_mut(),
                MODES[p].style,
                idle,
                ctx,
                (dry_l, dry_r),
                (gain, level),
                ol,
                or,
            );
            let len = self.fade_len as f32;
            for i in 0..n {
                if self.fade == 0 {
                    break;
                }
                let a = 1.0 - self.fade as f32 / len;
                l[i] = ol[i] + a * (l[i] - ol[i]);
                r[i] = or[i] + a * (r[i] - or[i]);
                self.fade -= 1;
            }
            if self.fade == 0 {
                self.prev = None;
            }
        }
    }
}

/// One engine's output for a stretch, from the dry input, into `l`, `r`.
#[allow(clippy::too_many_arguments)]
fn render(
    engine: &mut dyn Node,
    style: Style,
    idle: bool,
    ctx: &Ctx,
    (dry_l, dry_r): (&[f32], &[f32]),
    (gain, level): (&[f32], &[f32]),
    l: &mut [f32],
    r: &mut [f32],
) {
    match style {
        Style::Insert => {
            l.copy_from_slice(dry_l);
            r.copy_from_slice(dry_r);
            if idle {
                return;
            }
            engine.process(ctx, &[], l, r);
            for i in 0..l.len() {
                let g = gain[i] * level[i];
                l[i] = dry_l[i] + g * (l[i] - dry_l[i]);
                r[i] = dry_r[i] + g * (r[i] - dry_r[i]);
            }
        }
        Style::Send => {
            for i in 0..l.len() {
                l[i] = dry_l[i] * gain[i];
                r[i] = dry_r[i] * gain[i];
            }
            engine.process(ctx, &[], l, r);
            for i in 0..l.len() {
                l[i] = dry_l[i] + level[i] * l[i];
                r[i] = dry_r[i] + level[i] * r[i];
            }
        }
    }
}

impl Node for XyFx {
    fn info(&self) -> &'static NodeInfo {
        &INFO
    }
    fn params(&self) -> &'static [ParamInfo] {
        &PARAMS
    }
    fn set_param(&mut self, id: u16, value: f32) {
        if let Some(p) = PARAMS.get(id as usize) {
            let v = p.clamp(value);
            self.v[id as usize] = v;
            self.apply(id, v);
        }
    }
    fn param(&self, id: u16) -> f32 {
        self.v.get(id as usize).copied().unwrap_or(f32::NAN)
    }
    fn prepare(&mut self, rate: u32, max_block: usize) {
        self.rate = rate.max(1);
        for e in &mut self.engines {
            e.prepare(self.rate, max_block);
        }
        for b in self.dry.iter_mut().chain(self.old.iter_mut()) {
            *b = vec![0.0; max_block];
        }
        self.gain = vec![0.0; max_block];
        self.level = vec![0.0; max_block];
        self.timing();
        self.clear();
    }
    fn reset(&mut self) {
        self.clear();
    }
    fn tail_frames(&self) -> u32 {
        let tail = |k: usize| self.engines[k].tail_frames();
        let t = tail(self.cur).max(self.prev.map_or(0, tail));
        t.saturating_add(self.down.recip() as u32)
    }
    fn latency(&self) -> u32 {
        0
    }
    fn process(&mut self, ctx: &Ctx, events: &[Timed], left: &mut [f32], right: &mut [f32]) {
        let len = left.len().min(right.len());
        let per_beat = ctx.bpm / (60.0 * f64::from(ctx.rate.max(1)));
        let mut segs = Segs::new(events);
        while let Some((from, to)) = segs.next(len, |ev| {
            if let NodeEvent::Param { id, value } = *ev {
                self.set_param(id, value);
            }
        }) {
            let part = Ctx {
                rate: ctx.rate,
                frame: ctx.frame + from as u64,
                bpm: ctx.bpm,
                beats: ctx.beats + from as f64 * per_beat,
                playing: ctx.playing,
                sidechain: ctx
                    .sidechain
                    .and_then(|(a, b)| Some((a.get(from..to)?, b.get(from..to)?))),
            };
            // A stretch longer than the scratch (never, with the host's
            // promise) is done in pieces.
            let mut at = from;
            while at < to {
                let end = to.min(at + self.gain.len().max(1));
                if self.gain.is_empty() {
                    return;
                }
                self.run(&part, &mut left[at..end], &mut right[at..end]);
                at = end;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;

    fn made() -> XyFx {
        let mut x = XyFx::new();
        x.prepare(RATE, 256);
        x
    }

    fn noise(frames: usize) -> Vec<f32> {
        let mut s = 7u32;
        (0..frames)
            .map(|_| {
                s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (s >> 9) as f32 / (1u32 << 23) as f32 - 0.5
            })
            .collect()
    }

    fn play(x: &mut XyFx, input: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let (mut l, mut r) = (input.to_vec(), input.to_vec());
        for (i, (a, b)) in l.chunks_mut(128).zip(r.chunks_mut(128)).enumerate() {
            let mut ctx = Ctx::new(RATE, (i * 128) as u64);
            ctx.beats = (i * 128) as f64 / RATE as f64 * 2.0;
            ctx.playing = true;
            x.process(&ctx, &[], a, b);
        }
        (l, r)
    }

    fn diff(a: &[f32], b: &[f32]) -> f32 {
        let d: f32 = a.iter().zip(b).map(|(x, y)| (x - y) * (x - y)).sum();
        (d / a.len() as f32).sqrt()
    }

    #[test]
    fn the_table_matches_the_engines() {
        assert_eq!(EFFECTS.len(), MODES.len());
        for (k, m) in MODES.iter().enumerate() {
            let name = EFFECTS[k];
            let e = registry::make(m.engine).unwrap();
            assert_eq!(e.latency(), 0, "{name}: no latency");
            let ps = e.params();
            for &(id, v) in m.fixed {
                let p = ps.iter().find(|p| p.id == id).unwrap();
                assert!((p.min..=p.max).contains(&v), "{name} fixed {}", p.name);
            }
            for a in [&m.x, &m.y] {
                let p = ps.iter().find(|p| p.id == a.param).unwrap();
                for pos in [0.0, 0.3, 0.5, 1.0] {
                    let v = a.value(pos);
                    assert!((p.min..=p.max).contains(&v), "{name} {} at {pos}", a.name);
                }
                if let Map::Steps(v) = a.map {
                    assert_eq!(v.len(), a.steps.len());
                    if let Curve::Choice(names) = p.curve {
                        for (s, name) in v.iter().zip(a.steps) {
                            assert_eq!(names[*s as usize], *name, "{name} column");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn every_effect_moves_with_x_and_with_y() {
        let input = noise(24_000);
        for k in 0..MODES.len() {
            let at = |xv: f32, yv: f32| {
                let mut x = made();
                x.set_param(xy::EFFECT, k as f32);
                x.set_param(xy::X, xv);
                x.set_param(xy::Y, yv);
                x.reset();
                play(&mut x, &input).0
            };
            let base = at(20.0, 30.0);
            assert!(base.iter().all(|v| v.is_finite()));
            assert!(diff(&base, &at(85.0, 30.0)) > 1e-3, "{}: X", EFFECTS[k]);
            assert!(diff(&base, &at(20.0, 95.0)) > 1e-3, "{}: Y", EFFECTS[k]);
        }
    }

    #[test]
    fn closed_an_insert_is_the_dry_sound_and_a_send_rings_out() {
        let input = noise(24_000);
        // The filter, closed: exactly the input once the release ends.
        let mut x = made();
        x.set_param(xy::X, 10.0);
        x.set_param(xy::ACTIVE, 0.0);
        x.set_param(xy::RELEASE, 5.0);
        let (l, _) = play(&mut x, &input);
        let settle = 2 * 240;
        assert!(l[settle..]
            .iter()
            .zip(&input[settle..])
            .all(|(a, b)| a.to_bits() == b.to_bits()));
        // The echo: fed while open, then closed with silence coming in,
        // and the repeats still sound.
        let mut x = made();
        x.set_param(xy::EFFECT, 1.0);
        x.set_param(xy::Y, 60.0);
        x.reset();
        let mut burst = noise(4800);
        burst.resize(48_000, 0.0);
        let (mut l, mut r) = (burst.clone(), burst.clone());
        let mut ctx = Ctx::new(RATE, 0);
        ctx.playing = true;
        for (i, (a, b)) in l.chunks_mut(128).zip(r.chunks_mut(128)).enumerate() {
            if i == 40 {
                x.set_param(xy::ACTIVE, 0.0);
            }
            ctx.frame = (i * 128) as u64;
            ctx.beats = ctx.frame as f64 / RATE as f64 * 2.0;
            x.process(&ctx, &[], a, b);
        }
        let tail: f32 = l[12_000..36_000].iter().map(|v| v * v).sum();
        assert!(tail > 1.0, "the echoes ring on after closing ({tail})");
    }

    #[test]
    fn a_change_of_effect_crossfades_without_a_jump() {
        let input: Vec<f32> = (0..9600)
            .map(|i| 0.5 * (i as f32 * 220.0 / RATE as f32 * std::f32::consts::TAU).sin())
            .collect();
        let mut x = made();
        x.set_param(xy::X, 50.0);
        let (mut l, mut r) = (input.clone(), input.clone());
        let mut ctx = Ctx::new(RATE, 0);
        for (i, (a, b)) in l.chunks_mut(128).zip(r.chunks_mut(128)).enumerate() {
            if i == 30 {
                x.set_param(xy::EFFECT, 4.0);
            }
            ctx.frame = (i * 128) as u64;
            x.process(&ctx, &[], a, b);
        }
        // The neutral filter is the input; the crush at mid X is not, and
        // the switch never steps by more than the sine itself moves plus
        // the crush's own steps.
        let worst = l
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0, f32::max);
        assert!(worst < 0.25, "largest step {worst}");
        assert!(diff(&l[6000..], &input[6000..]) > 1e-3);
    }

    #[test]
    fn presets_name_real_columns() {
        let (x, _) = axes(1);
        assert_eq!(x.step_name(column(4, 8) / 100.0), Some("1/8D"));
        let (x, _) = axes(8);
        assert_eq!(x.step_name(column(2, 6) / 100.0), Some("1/16"));
        assert_eq!(axes(0).0.step_name(0.5), None);
        assert_eq!(engine(3), "mpfree.fx.plate");
    }
}
