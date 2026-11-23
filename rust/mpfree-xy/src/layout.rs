//! What the pad plays: layouts, checked and compiled for the sound loop.

use crate::input::unit;
use mpfree_mod::MapCurve;
use mpfree_param::ParamAddr;

/// The most settings on one axis.
pub const TARGETS: usize = 8;

/// One setting an axis moves: its value at the axis's two ends, and the
/// curve between.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Target {
    pub addr: ParamAddr,
    /// The value at position 0 (left or bottom) and at position 1. `to`
    /// below `from` turns it the other way.
    pub from: f32,
    pub to: f32,
    pub curve: MapCurve,
}

impl Target {
    /// A setting moved in a straight line from `from` to `to`.
    pub fn linear(addr: ParamAddr, from: f32, to: f32) -> Target {
        Target {
            addr,
            from,
            to,
            curve: MapCurve::Linear,
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
    /// It glides back to where it rests, about `snap_ms` to cover 63 % of
    /// the way (it lands within a hundredth in five times that).
    GoBack { snap_ms: f32 },
    /// It stays where the finger left it.
    Stay,
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
    /// A glide time that isn't a positive number.
    Glide,
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
            LayoutError::Glide => write!(f, "the glide back needs a time above 0"),
        }
    }
}

impl Layout {
    /// Checked and resolved into the fixed list the sound loop plays.
    pub fn compile(&self) -> Result<Compiled, LayoutError> {
        let c = Compiled::new(&self.across.targets, &self.up.targets, self.gate)?;
        let on = |v: f32| (0.0..=1.0).contains(&v);
        if !on(self.rest.0) || !on(self.rest.1) {
            return Err(LayoutError::Rest);
        }
        if let Lift::GoBack { snap_ms } = self.lift {
            if snap_ms.is_nan() || snap_ms <= 0.0 {
                return Err(LayoutError::Glide);
            }
        }
        Ok(c)
    }
}

/// A layout as the sound loop plays it: fixed arrays, `Copy`, nothing to
/// look up.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Compiled {
    across: [Option<Target>; TARGETS],
    up: [Option<Target>; TARGETS],
    gate: Option<ParamAddr>,
}

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
                *o = Some(*t);
            }
        }
        Ok(c)
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
