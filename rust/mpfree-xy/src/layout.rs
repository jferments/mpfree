//! What the pad plays: layouts, checked and compiled for the sound loop.

use crate::input::unit;
use mpfree_mod::MapCurve;
use mpfree_param::ParamAddr;

/// The most settings on one axis.
pub const TARGETS: usize = 8;

/// The longest glide back, in ms (Setup's Glide back runs from 0 to 2 s).
pub const GLIDE_MAX_MS: f32 = 2000.0;

/// How a setting follows the pad (the XY controller plan, phase 2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum How {
    /// The pad decides where it is: `from` and `to` are values in the
    /// setting's own units, and the pad's value replaces what moves it
    /// underneath (a recorded move, a lock, a macro) while it holds it.
    #[default]
    Set,
    /// The pad pushes it from wherever it is: `from` and `to` are pushes
    /// in knob travel, -1 to 1 (the whole travel down or up), added on
    /// top, so an LFO, a lock or a recorded move underneath keeps going.
    /// The push always glides away when the pad lets go, even with Stay.
    Add,
}

/// One setting an axis moves: its value at the axis's two ends, and the
/// curve between.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Target {
    pub addr: ParamAddr,
    /// The value at position 0 (left or bottom) and at position 1. `to`
    /// below `from` turns it the other way. A push in knob travel for an
    /// Add setting ([`How::Add`]).
    pub from: f32,
    pub to: f32,
    pub curve: MapCurve,
    pub how: How,
}

impl Target {
    /// A setting moved in a straight line from `from` to `to`.
    pub fn linear(addr: ParamAddr, from: f32, to: f32) -> Target {
        Target {
            addr,
            from,
            to,
            curve: MapCurve::Linear,
            how: How::Set,
        }
    }

    /// A setting pushed in a straight line from `from` to `to` (knob
    /// travel, -1 to 1) on top of wherever it is.
    pub fn add(addr: ParamAddr, from: f32, to: f32) -> Target {
        Target {
            how: How::Add,
            ..Target::linear(addr, from, to)
        }
    }

    /// The setting's value at position `pos` (0 to 1).
    pub fn value(&self, pos: f32) -> f32 {
        self.from + (self.to - self.from) * self.curve.apply(unit(pos))
    }

    /// The position (0 to 1) where the setting has `value`: the inverse of
    /// [`Target::value`] (`None` when `from` and `to` are the same).
    pub fn position(&self, value: f32) -> Option<f32> {
        let span = self.to - self.from;
        if span == 0.0 || !value.is_finite() {
            return None;
        }
        let c = unit((value - self.from) / span);
        Some(match self.curve {
            MapCurve::Linear => c,
            MapCurve::Exp => c.sqrt(),
            MapCurve::Log => 1.0 - (1.0 - c).sqrt(),
            // The S-curve has no simple inverse: halve the interval (it
            // rises the whole way).
            MapCurve::S => {
                let (mut lo, mut hi) = (0.0f32, 1.0f32);
                for _ in 0..24 {
                    let mid = (lo + hi) / 2.0;
                    if MapCurve::S.apply(mid) < c {
                        lo = mid;
                    } else {
                        hi = mid;
                    }
                }
                (lo + hi) / 2.0
            }
        })
    }
}

/// One way across the pad: its name and the settings it moves.
#[derive(Clone, Debug, PartialEq)]
pub struct Axis {
    pub name: String,
    pub targets: Vec<Target>,
}

impl Axis {
    pub fn new(name: &str, targets: &[Target]) -> Axis {
        Axis {
            name: name.to_string(),
            targets: targets.to_vec(),
        }
    }
}

/// What the pad does when the finger lifts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Lift {
    /// Go back. A ready-made layout's pad glides back to where it rests,
    /// about `snap_ms` to cover 63 % of the way (it lands within a
    /// hundredth in five times that). On a layout of yours each setting
    /// glides back to its own value as it is by then, over `snap_ms`
    /// (0 to [`GLIDE_MAX_MS`]; 0 lets go at once).
    GoBack { snap_ms: f32 },
    /// Stay: it stays where the finger left it. On a layout of yours the
    /// Set settings keep the pad's values as their own (one UNDO step,
    /// never while recording); Add settings glide their push away.
    Stay,
}

/// Where a touch puts the pad (Setup's "A touch").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Start {
    /// It jumps to the finger.
    #[default]
    Jump,
    /// It carries on from where the pad is: the finger moves it from
    /// there, by as much as the finger moves.
    FromHere,
}

impl Start {
    /// The pad's place for a finger at `now` that landed at `landed`, the
    /// pad having been at `here` when it landed.
    pub fn place(self, here: (f32, f32), landed: (f32, f32), now: (f32, f32)) -> (f32, f32) {
        match self {
            Start::Jump => (unit(now.0), unit(now.1)),
            Start::FromHere => (
                unit(here.0 + now.0 - landed.0),
                unit(here.1 + now.1 - landed.1),
            ),
        }
    }
}

/// What the pad plays.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub name: String,
    pub across: Axis,
    pub up: Axis,
    /// Where the pad rests, 0 to 1 each way.
    pub rest: (f32, f32),
    pub lift: Lift,
    /// A setting that is 1 while a gesture holds the pad and 0 when it
    /// lets go (an XY FX's Active: the effect opens when the finger lands
    /// and lets go when it lifts).
    pub gate: Option<ParamAddr>,
    /// Hold starts on: a lift leaves the pad in charge of its settings
    /// until Hold is off.
    pub hold: bool,
    /// Where a touch puts the pad.
    pub start: Start,
    /// Follow the selected track: its synth settings mean the selected
    /// track's synth, fixed when a finger lands.
    pub follow: bool,
}

impl Layout {
    /// An empty layout of yours called `name`: Go back over 120 ms, Hold
    /// off, a touch jumps, not following the selected track.
    pub fn new(name: &str) -> Layout {
        Layout {
            name: name.to_string(),
            across: Axis::new("Across", &[]),
            up: Axis::new("Up", &[]),
            rest: (0.5, 0.5),
            lift: Lift::GoBack { snap_ms: 120.0 },
            gate: None,
            hold: false,
            start: Start::Jump,
            follow: false,
        }
    }
}

/// Why a layout can't be played.
#[derive(Clone, Debug, PartialEq)]
pub enum LayoutError {
    /// An axis has more than [`TARGETS`] settings.
    TooMany { axis: &'static str, count: usize },
    /// A value that isn't a number.
    NotANumber(ParamAddr),
    /// Where it rests is off the pad.
    Rest,
    /// A glide time that isn't from 0 to [`GLIDE_MAX_MS`].
    Glide,
    /// A setting on the layout twice: a setting goes on one direction,
    /// once.
    Twice(ParamAddr),
    /// An Add setting's push outside -1 to 1 (the whole travel down or
    /// up).
    Push(ParamAddr),
}

impl std::fmt::Display for LayoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LayoutError::TooMany { axis, count } => write!(
                f,
                "{axis} moves {count} settings; it can move {TARGETS} at most"
            ),
            LayoutError::NotANumber(a) => write!(f, "{a} has a value that isn't a number"),
            LayoutError::Rest => write!(f, "where the pad rests is off the pad"),
            LayoutError::Glide => write!(f, "the glide back runs from 0 to 2 s"),
            LayoutError::Twice(a) => write!(
                f,
                "{a} is on the layout twice; a setting goes on one direction, once"
            ),
            LayoutError::Push(a) => write!(
                f,
                "{a} pushes past its whole travel; an Add setting pushes -100 to 100 %"
            ),
        }
    }
}

impl Layout {
    /// Checked and resolved into the fixed list the sound loop plays.
    pub fn compile(&self) -> Result<Compiled, LayoutError> {
        let mut c = Compiled::new(&self.across.targets, &self.up.targets, self.gate)?;
        let on = |v: f32| (0.0..=1.0).contains(&v);
        if !on(self.rest.0) || !on(self.rest.1) {
            return Err(LayoutError::Rest);
        }
        if let Lift::GoBack { snap_ms } = self.lift {
            if !(0.0..=GLIDE_MAX_MS).contains(&snap_ms) {
                return Err(LayoutError::Glide);
            }
        }
        c.rest = self.rest;
        c.lift = self.lift;
        c.hold = self.hold;
        c.start = self.start;
        c.follow = self.follow;
        Ok(c)
    }

    /// Puts `t` on the direction `up` (across when false), over the same
    /// setting where it is: on the other direction it moves here (true
    /// when it moved). False with no change when the direction is full.
    pub fn put(&mut self, up: bool, t: Target) -> Result<bool, LayoutError> {
        let (to, other) = if up {
            (&mut self.up, &mut self.across)
        } else {
            (&mut self.across, &mut self.up)
        };
        if let Some(i) = to.targets.iter().position(|o| o.addr == t.addr) {
            to.targets[i] = t;
            return Ok(false);
        }
        if to.targets.len() >= TARGETS {
            return Err(LayoutError::TooMany {
                axis: if up { "Up" } else { "Across" },
                count: to.targets.len() + 1,
            });
        }
        let moved = other.targets.iter().position(|o| o.addr == t.addr);
        if let Some(i) = moved {
            other.targets.remove(i);
        }
        to.targets.push(t);
        Ok(moved.is_some())
    }

    /// Which direction `addr` is on: Some(false) across, Some(true) up.
    pub fn way_of(&self, addr: ParamAddr) -> Option<bool> {
        if self.across.targets.iter().any(|t| t.addr == addr) {
            Some(false)
        } else if self.up.targets.iter().any(|t| t.addr == addr) {
            Some(true)
        } else {
            None
        }
    }

    /// Takes `addr` off the layout: true when it was on it.
    pub fn take_off(&mut self, addr: ParamAddr) -> bool {
        let before = self.across.targets.len() + self.up.targets.len();
        self.across.targets.retain(|t| t.addr != addr);
        self.up.targets.retain(|t| t.addr != addr);
        before != self.across.targets.len() + self.up.targets.len()
    }
}

/// A layout as the sound loop plays it: fixed arrays, `Copy`, nothing to
/// look up.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Compiled {
    across: [Option<Target>; TARGETS],
    up: [Option<Target>; TARGETS],
    gate: Option<ParamAddr>,
    rest: (f32, f32),
    lift: Lift,
    hold: bool,
    start: Start,
    follow: bool,
}

/// A setting's place in a [`Compiled`] layout: 0 to 7 across, 8 to 15 up
/// (for state the sound loop keeps per setting).
pub const SLOTS: usize = 2 * TARGETS;

impl Compiled {
    /// The settings `across` and `up` and the `gate`, checked, with nothing
    /// allocated (for a layout made on the sound loop).
    pub fn new(
        across: &[Target],
        up: &[Target],
        gate: Option<ParamAddr>,
    ) -> Result<Compiled, LayoutError> {
        let mut c = Compiled {
            across: [None; TARGETS],
            up: [None; TARGETS],
            gate,
            rest: (0.5, 0.5),
            lift: Lift::Stay,
            hold: false,
            start: Start::Jump,
            follow: false,
        };
        for (ts, name, out) in [(across, "Across", &mut c.across), (up, "Up", &mut c.up)] {
            if ts.len() > TARGETS {
                return Err(LayoutError::TooMany {
                    axis: name,
                    count: ts.len(),
                });
            }
            for (o, t) in out.iter_mut().zip(ts) {
                if !t.from.is_finite() || !t.to.is_finite() {
                    return Err(LayoutError::NotANumber(t.addr));
                }
                let push = -1.0..=1.0;
                if t.how == How::Add && !(push.contains(&t.from) && push.contains(&t.to)) {
                    return Err(LayoutError::Push(t.addr));
                }
                *o = Some(*t);
            }
        }
        let all: [Option<Target>; SLOTS] = std::array::from_fn(|i| {
            if i < TARGETS {
                c.across[i]
            } else {
                c.up[i - TARGETS]
            }
        });
        for (i, t) in all.iter().enumerate() {
            let Some(t) = t else { continue };
            if all[..i].iter().flatten().any(|o| o.addr == t.addr) || c.gate == Some(t.addr) {
                return Err(LayoutError::Twice(t.addr));
            }
        }
        Ok(c)
    }

    /// Each setting with its place ([`SLOTS`]): across (0 to 7), then up
    /// (8 to 15), in the layout's order.
    pub fn slots(&self) -> impl Iterator<Item = (usize, &Target)> {
        let across = self.across.iter().enumerate();
        let up = self.up.iter().enumerate().map(|(i, t)| (TARGETS + i, t));
        across
            .chain(up)
            .filter_map(|(i, t)| t.as_ref().map(|t| (i, t)))
    }

    /// The setting in place `slot`.
    pub fn slot(&self, slot: usize) -> Option<&Target> {
        match slot {
            s if s < TARGETS => self.across[s].as_ref(),
            s if s < SLOTS => self.up[s - TARGETS].as_ref(),
            _ => None,
        }
    }

    /// Each setting's place and value with the pad at `x`, `y` (a push in
    /// knob travel for an Add setting), across then up.
    pub fn each(&self, x: f32, y: f32, mut out: impl FnMut(usize, &Target, f32)) {
        for (i, t) in self.slots() {
            out(i, t, t.value(if i < TARGETS { x } else { y }));
        }
    }

    /// The same layout with each setting's address passed through `f` (a
    /// layout following the selected track, fixed when a finger lands).
    /// Allocates nothing.
    pub fn mapped(&self, f: impl Fn(ParamAddr) -> ParamAddr) -> Compiled {
        let mut c = *self;
        for t in c.across.iter_mut().chain(c.up.iter_mut()).flatten() {
            t.addr = f(t.addr);
        }
        c
    }

    /// The gate ([`Layout::gate`]).
    pub fn gate(&self) -> Option<ParamAddr> {
        self.gate
    }

    /// Where the pad rests.
    pub fn rest(&self) -> (f32, f32) {
        self.rest
    }

    /// What a lift does.
    pub fn lift(&self) -> Lift {
        self.lift
    }

    /// Whether Hold starts on.
    pub fn hold(&self) -> bool {
        self.hold
    }

    /// Where a touch puts the pad.
    pub fn start(&self) -> Start {
        self.start
    }

    /// Whether it follows the selected track.
    pub fn follows(&self) -> bool {
        self.follow
    }

    /// How many settings it plays by Add (each needs a host source).
    pub fn adds(&self) -> usize {
        self.slots().filter(|(_, t)| t.how == How::Add).count()
    }

    /// Each setting's value with the pad at `x`, `y`, held (`down`) or
    /// not: the settings across, then up, then the gate (1 or 0), in the
    /// layout's order.
    pub fn values(&self, x: f32, y: f32, down: bool, mut out: impl FnMut(ParamAddr, f32)) {
        for t in self.across.iter().flatten() {
            out(t.addr, t.value(x));
        }
        for t in self.up.iter().flatten() {
            out(t.addr, t.value(y));
        }
        if let Some(g) = self.gate {
            out(g, if down { 1.0 } else { 0.0 });
        }
    }

    /// Where the pad stands, read back from its settings' values now (the
    /// first setting of each way; `read` gives a setting's value, if it
    /// has one). `None` for a way with no setting or none to read.
    pub fn position(&self, read: impl Fn(ParamAddr) -> Option<f32>) -> (Option<f32>, Option<f32>) {
        let at = |ts: &[Option<Target>; TARGETS]| {
            let t = ts.iter().flatten().next()?;
            t.position(read(t.addr)?)
        };
        (at(&self.across), at(&self.up))
    }

    /// Whether `addr` is one of the settings it plays.
    pub fn plays(&self, addr: ParamAddr) -> bool {
        self.gate == Some(addr)
            || self
                .across
                .iter()
                .chain(&self.up)
                .flatten()
                .any(|t| t.addr == addr)
    }
}
