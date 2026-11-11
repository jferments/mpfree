//! Moves: parameter moves recorded into clips (automation), for Perform FX
//! pads, the XY pad, the DJ knob and knob turns.
//!
//! A **gesture** is one move of one parameter: a finger on the XY pad
//! until it is back at rest, a knob from touch to let go, a pad's Hold
//! from press to release. A clip keeps its gestures in **lanes**, one per
//! parameter ([`Lane`]): points on the clip's ticks, each a knob position
//! (0 to 1, through the parameter's [`Map`]) and how it leads on
//! ([`Kind`]): a **ramp** glides to the next point, a **step** holds, and
//! an **end** lets the parameter go. Between a gesture's end and the next
//! gesture the lane is free: the parameter is wherever its knob is (the
//! player sends an unlock, like a parameter lock that ends).
//!
//! # Playing
//!
//! [`Moves::play`] takes a range of clip ticks of one pass and calls back
//! with each value due: at every point, and inside a ramp on every
//! [`GRID`]th tick of the clip. The ticks never depend on where a period
//! starts or ends, so the output is the same at any period size. A lane
//! still moving at the end of the pass carries on from the start of the
//! next (a gesture crossing the loop's end is stored as two, see
//! [`Moves::put`]). [`Moves::chase`] gives every lane's state at a tick,
//! for a clip launched or located into the middle. Neither allocates.
//!
//! # Addresses
//!
//! A lane names its parameter with a [`Target`]: on the clip's own track
//! it is track-relative ([`mpfree_param::RelAddr`]), so a clip copied to
//! another track moves that track; anything not on a track (the master's
//! Perform FX, a submix's insert, a macro) is fixed.
//!
//! # Recording and capturing
//!
//! [`MoveLog`] runs on the audio thread and keeps every move made while
//! the transport rolls, recording or not, in a fixed ring: nothing
//! allocates. [`MoveLog::phrase`] gives the moves since the last bar of
//! stillness as [`Gesture`]s (allocating, off the audio thread), and
//! [`Moves::put`] writes a gesture into a clip's lanes, replacing what was
//! there over the ticks it covers. That is how the moves just played are
//! captured into a clip, the way CAPTURE keeps the notes.
//! [`Moves::put_saving`] hands over what a write replaces, so an undo can
//! put it back exactly with [`Moves::restore`].
//!
//! # Editing
//!
//! The step pages move ticks around: [`Moves::copy_range`] copies a
//! stretch over another, [`Moves::cut`] takes one out and closes the gap,
//! [`Moves::insert`] opens a gap, and [`Moves::truncate`] cuts a clip
//! short. Each keeps what every lane does on the ticks it does not touch.
//! [`Moves::check`] says whether a clip's moves are sound.
#![forbid(unsafe_code)]

mod edit;
mod log;

pub use log::{Gesture, Input, LogEntry, MoveLog, GAP_BARS, MAX_ACTIVE, MAX_INPUT, MAX_LOG};

use mpfree_node::{Curve, ParamInfo};
use mpfree_param::{ParamAddr, RelAddr};

/// Inside a ramp, a value is sent on every tick of the clip that is a
/// multiple of this: 120 a beat at 960 PPQ, about every 4 ms at 120 BPM
/// (the effects smooth their parameters between).
pub const GRID: u32 = 8;
/// The most lanes a clip's moves have.
pub const MAX_LANES: usize = 64;
/// The most points a lane holds.
pub const MAX_POINTS: usize = 16_384;
/// Points dropped by thinning stay within this much knob travel of the
/// line through the points kept.
pub const THIN: f32 = 0.002;
/// Thinning looks at most this many points between two it keeps.
const THIN_WINDOW: usize = 64;

/// The parameter a lane moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Target {
    /// On the clip's own track.
    Track(RelAddr),
    /// Somewhere that is not on a track: the master, a submix, a return,
    /// a macro, the morph.
    Fixed(ParamAddr),
}

impl Target {
    /// How a clip on `track` names `addr`: relative when it is on that
    /// track, fixed otherwise.
    pub fn for_track(addr: ParamAddr, track: u8) -> Target {
        match addr.rel() {
            Some(rel) if addr.owner.track() == Some(track) => Target::Track(rel),
            _ => Target::Fixed(addr),
        }
    }

    /// The parameter when the clip plays on `track`.
    pub fn on(self, track: u8) -> ParamAddr {
        match self {
            Target::Track(rel) => rel.on(track),
            Target::Fixed(a) => a,
        }
    }
}

/// How a parameter's knob travel (0 to 1) maps to its values. A copy of
/// what matters in its `ParamInfo`, so a lane can play without the node.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Map {
    pub min: f32,
    pub max: f32,
    /// Even steps in pitch (a cutoff): `min * (max / min)^u`.
    pub log: bool,
    pub steps: Steps,
}

/// Whether a parameter only takes some values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Steps {
    /// Any value in its range.
    None,
    /// Whole numbers (a choice, a stepped knob).
    Whole,
    /// Off (0) or on (1).
    Toggle,
}

impl Map {
    pub fn of(info: &ParamInfo) -> Map {
        Map {
            min: info.min,
            max: info.max,
            log: matches!(info.curve, Curve::Log) && info.min > 0.0,
            steps: match info.curve {
                Curve::Toggle => Steps::Toggle,
                Curve::Stepped | Curve::Choice(_) => Steps::Whole,
                _ => Steps::None,
            },
        }
    }

    pub fn linear(min: f32, max: f32) -> Map {
        Map {
            min,
            max,
            log: false,
            steps: Steps::None,
        }
    }

    /// Off or on (a Perform pad's Hold).
    pub fn toggle() -> Map {
        Map {
            min: 0.0,
            max: 1.0,
            log: false,
            steps: Steps::Toggle,
        }
    }

    /// A switch: its lanes hold each value (no ramps).
    pub fn stepped(&self) -> bool {
        self.steps != Steps::None
    }

    /// The value at knob position `u`.
    pub fn value(&self, u: f32) -> f32 {
        let u = if u.is_finite() {
            u.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let v = if self.log {
            self.min * (self.max / self.min).powf(u)
        } else {
            self.min + (self.max - self.min) * u
        };
        match self.steps {
            Steps::None => v,
            Steps::Whole => v.round(),
            Steps::Toggle => {
                if v >= 0.5 {
                    1.0
                } else {
                    0.0
                }
            }
        }
    }

    /// The knob position of value `v`.
    pub fn unit(&self, v: f32) -> f32 {
        if self.max.partial_cmp(&self.min) != Some(std::cmp::Ordering::Greater) || !v.is_finite() {
            return 0.0;
        }
        let v = v.clamp(self.min, self.max);
        let u = if self.log {
            (v / self.min).ln() / (self.max / self.min).ln()
        } else {
            (v - self.min) / (self.max - self.min)
        };
        u.clamp(0.0, 1.0)
    }
}

/// How a point leads to the next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Glides to the next point (holds when the next is an end or there
    /// is none).
    Ramp,
    /// Holds until the next point.
    Step,
    /// The gesture ends: the parameter goes back to its own value.
    End,
}

/// A point of a lane.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    /// The clip tick, within one pass.
    pub tick: u32,
    /// Knob position, 0 to 1 (unused by an end).
    pub u: f32,
    pub kind: Kind,
}

impl Point {
    pub fn ramp(tick: u32, u: f32) -> Point {
        Point {
            tick,
            u,
            kind: Kind::Ramp,
        }
    }
    pub fn step(tick: u32, u: f32) -> Point {
        Point {
            tick,
            u,
            kind: Kind::Step,
        }
    }
    pub fn end(tick: u32) -> Point {
        Point {
            tick,
            u: 0.0,
            kind: Kind::End,
        }
    }
}

/// A parameter's moves in a clip: points in tick order, at most one per
/// tick.
#[derive(Clone, Debug, PartialEq)]
pub struct Lane {
    pub target: Target,
    pub map: Map,
    pub points: Vec<Point>,
}

impl Lane {
    pub fn new(target: Target, map: Map) -> Lane {
        Lane {
            target,
            map,
            points: Vec::new(),
        }
    }

    /// The knob position at `tick` (a point on `tick` counts), or `None`
    /// where the lane is free. Before its first point a lane is where the
    /// pass before left it.
    pub fn state_at(&self, tick: f64) -> Option<f32> {
        let p = &self.points;
        let i = p.partition_point(|q| f64::from(q.tick) <= tick);
        let &a = (if i == 0 { p.last() } else { p.get(i - 1) })?;
        match a.kind {
            Kind::End => None,
            Kind::Step => Some(a.u),
            Kind::Ramp => match p.get(i) {
                Some(&b) if i > 0 && b.kind != Kind::End => Some(lerp(a, b, tick)),
                _ => Some(a.u),
            },
        }
    }

    /// The value (through the map) at `tick`, or `None` where free.
    pub fn value_at(&self, tick: f64) -> Option<f32> {
        self.state_at(tick).map(|u| self.map.value(u))
    }
}

/// The knob position between `a` (a ramp) and `b` at `t`.
fn lerp(a: Point, b: Point, t: f64) -> f32 {
    let span = f64::from(b.tick) - f64::from(a.tick);
    if span <= 0.0 {
        return b.u;
    }
    let f = ((t - f64::from(a.tick)) / span).clamp(0.0, 1.0) as f32;
    a.u + (b.u - a.u) * f
}

/// What a lane says at a tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Out {
    /// Set the parameter to this value (a lock that moves).
    Value(f32),
    /// Let the parameter go (unlock).
    End,
}

/// Why a write was refused. Nothing changes when it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MovesError {
    /// No room: [`MAX_LANES`] lanes or [`MAX_POINTS`] in the lane.
    Full,
    /// The clip has no length, the points are not in tick order, or (for
    /// [`Moves::check`]) the moves are not sound.
    Bad,
}

/// A clip's moves: its lanes, in the order they were made.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Moves {
    lanes: Vec<Lane>,
}

/// One lane's events in a range of ticks, in order.
#[derive(Clone, Copy)]
struct Walk<'a> {
    lane: &'a Lane,
    /// The next point at or after `t`.
    i: usize,
    /// The next tick to look at.
    t: f64,
}

impl<'a> Walk<'a> {
    fn new(lane: &'a Lane, from: f64) -> Walk<'a> {
        let i = lane.points.partition_point(|p| f64::from(p.tick) < from);
        Walk { lane, i, t: from }
    }

    /// The next event at or after `t` and before `to`.
    fn peek(&self, to: f64) -> Option<(f64, Out)> {
        let p = &self.lane.points;
        let next = p.get(self.i);
        let pt = next.map_or(f64::INFINITY, |q| f64::from(q.tick));
        // Inside a ramp (from the point before to a point with a value),
        // the grid ticks before the next point.
        if self.i > 0 && p[self.i - 1].kind == Kind::Ramp {
            if let Some(&b) = next.filter(|b| b.kind != Kind::End) {
                let g = (self.t / f64::from(GRID)).ceil() * f64::from(GRID);
                if g < pt && g < to {
                    let u = lerp(p[self.i - 1], b, g);
                    return Some((g, Out::Value(self.lane.map.value(u))));
                }
            }
        }
        let q = next?;
        if pt >= to {
            return None;
        }
        Some((
            pt,
            match q.kind {
                Kind::End => Out::End,
                _ => Out::Value(self.lane.map.value(q.u)),
            },
        ))
    }

    /// Moves past an event at `tick` (a point there is passed too).
    fn pass(&mut self, tick: f64) {
        if self.lane.points.get(self.i).map(|q| f64::from(q.tick)) == Some(tick) {
            self.i += 1;
        }
        self.t = tick.floor() + 1.0;
    }
}

impl Moves {
    pub fn new() -> Moves {
        Moves::default()
    }

    pub fn lanes(&self) -> &[Lane] {
        &self.lanes
    }

    pub fn is_empty(&self) -> bool {
        self.lanes.iter().all(|l| l.points.is_empty())
    }

    /// Points in every lane.
    pub fn len(&self) -> usize {
        self.lanes.iter().map(|l| l.points.len()).sum()
    }

    pub fn lane(&self, target: Target) -> Option<&Lane> {
        self.lanes.iter().find(|l| l.target == target)
    }

    pub fn lane_mut(&mut self, target: Target) -> Option<&mut Lane> {
        self.lanes.iter_mut().find(|l| l.target == target)
    }

    /// Takes a lane out.
    pub fn remove(&mut self, target: Target) -> Option<Lane> {
        let i = self.lanes.iter().position(|l| l.target == target)?;
        Some(self.lanes.remove(i))
    }

    /// Makes the lane for `target` (if new) with room for `points` more
    /// points, so [`Moves::put`] on it does not allocate while that room
    /// lasts. Allocates.
    pub fn reserve(&mut self, target: Target, map: Map, points: usize) -> Result<(), MovesError> {
        let k = self.lane_index(target, map)?;
        let lane = &mut self.lanes[k];
        lane.points
            .reserve(points.min(MAX_POINTS.saturating_sub(lane.points.len())));
        Ok(())
    }

    /// Whether [`Moves::put_saving`] of `points` points into the lane for
    /// `target` would go without allocating: the lane exists and has room
    /// for them and the up to three points a write adds. (A write never
    /// frees: it never removes a lane or shrinks one.)
    pub fn fits(&self, target: Target, points: usize) -> bool {
        self.lane(target)
            .is_some_and(|l| l.points.capacity() - l.points.len() >= points + 3)
    }

    fn lane_index(&mut self, target: Target, map: Map) -> Result<usize, MovesError> {
        if let Some(k) = self.lanes.iter().position(|l| l.target == target) {
            return Ok(k);
        }
        if self.lanes.len() >= MAX_LANES {
            return Err(MovesError::Full);
        }
        self.lanes.push(Lane::new(target, map));
        Ok(self.lanes.len() - 1)
    }

    /// Calls `emit(tick, target, out)` for every value due in clip ticks
    /// `from..to` of one pass, in tick order (lanes in their order on the
    /// same tick): each point, and inside a ramp every [`GRID`]th tick. The
    /// same ticks however the range is cut. Real-time safe.
    pub fn play(&self, from: f64, to: f64, mut emit: impl FnMut(f64, Target, Out)) {
        if to.partial_cmp(&from) != Some(std::cmp::Ordering::Greater) {
            return;
        }
        let n = self.lanes.len().min(MAX_LANES);
        let mut walks: [Option<Walk>; MAX_LANES] = [None; MAX_LANES];
        for (w, lane) in walks.iter_mut().zip(&self.lanes[..n]) {
            if !lane.points.is_empty() {
                *w = Some(Walk::new(lane, from));
            }
        }
        loop {
            let mut best: Option<(usize, f64, Out)> = None;
            for (k, w) in walks[..n].iter().enumerate() {
                if let Some((t, o)) = w.as_ref().and_then(|w| w.peek(to)) {
                    if best.is_none_or(|(_, bt, _)| t < bt) {
                        best = Some((k, t, o));
                    }
                }
            }
            let Some((k, t, o)) = best else {
                return;
            };
            if let Some(w) = walks[k].as_mut() {
                emit(t, w.lane.target, o);
                w.pass(t);
            }
        }
    }

    /// Every lane's state at clip tick `at`: its value, or [`Out::End`]
    /// where it is free. For a clip launched or located into the middle.
    /// Real-time safe.
    pub fn chase(&self, at: f64, mut emit: impl FnMut(Target, Out)) {
        for lane in &self.lanes {
            if lane.points.is_empty() {
                continue;
            }
            match lane.value_at(at) {
                Some(v) => emit(lane.target, Out::Value(v)),
                None => emit(lane.target, Out::End),
            }
        }
    }

    /// Writes a gesture into the lane for `target` (made with `map` if
    /// new) of a clip `len` ticks long, replacing what the lane had over
    /// the ticks the gesture covers, then thins what it wrote.
    ///
    /// `points` are clip ticks counted on from the pass the gesture began
    /// in, so they may run past `len`: a gesture crossing the loop's end
    /// is written as two (with a point on the last tick and one on tick 0
    /// where it crossed), and one longer than the clip keeps its last
    /// `len` ticks. A gesture that ends where an older one was still
    /// moving hands back to it (the older one's value from there on, not
    /// an end). On a switch (a stepped map) every point holds.
    ///
    /// It does not allocate when the lane exists and has room
    /// ([`Moves::reserve`]).
    pub fn put(
        &mut self,
        target: Target,
        map: Map,
        points: &[(f64, f32, Kind)],
        len: u32,
    ) -> Result<(), MovesError> {
        self.put_saving(target, map, points, len, |_, _, _| {})
    }

    /// [`Moves::put`], first calling `save(from, to, old)` for each range
    /// of clip ticks `from..to` the write may change (one, or two when the
    /// gesture crosses the loop's end), with the points the lane has
    /// there (none for a new lane). [`Moves::restore`] with them undoes
    /// the write exactly. Nothing is saved when the write is refused.
    pub fn put_saving(
        &mut self,
        target: Target,
        map: Map,
        points: &[(f64, f32, Kind)],
        len: u32,
        mut save: impl FnMut(u32, u32, &[Point]),
    ) -> Result<(), MovesError> {
        if len == 0
            || points.iter().any(|p| !p.0.is_finite())
            || points.windows(2).any(|w| w[1].0 < w[0].0)
        {
            return Err(MovesError::Bad);
        }
        let (Some(&first), Some(&last)) = (points.first(), points.last()) else {
            return Ok(());
        };
        // A crossing adds two points; thinning comes after, so ask for all.
        let have = self.lane(target).map_or(0, |l| l.points.len());
        if have + points.len() + 3 > MAX_POINTS {
            return Err(MovesError::Full);
        }
        let k = self.lane_index(target, map)?;
        let lane = &mut self.lanes[k];
        let stepped = lane.map.stepped();
        let lenf = f64::from(len);
        // Keep the last `len` ticks of a gesture longer than the clip.
        let keep_from = (last.0 - lenf + 1.0).max(first.0);
        let start = points.partition_point(|p| p.0 < keep_from);
        let lead = (start > 0 && start < points.len() && points[start].0 > keep_from).then(|| {
            let (a, b) = (points[start - 1], points[start]);
            (keep_from, value_between(a, b, keep_from, stepped), a.2)
        });
        let base = (keep_from / lenf).floor();
        let pass_of = |t: f64| ((t / lenf).floor() - base) as u32;
        let tick_in = |t: f64, pass: u32| {
            (t - (base + f64::from(pass)) * lenf)
                .round()
                .clamp(0.0, lenf - 1.0) as u32
        };
        // What the lane was doing where each piece ends, from the lane as
        // it was (before any piece goes in): the first piece ends on the
        // last tick when the gesture crosses, else where the gesture ends.
        let crosses = pass_of(last.0) > pass_of(keep_from);
        let end_tick = tick_in(last.0, pass_of(last.0));
        let back = lane_back(&lane.points, end_tick);
        // What the write may change: from the tick before its first point
        // (a lead into an old ramp) to the tick after its last (an end
        // moved on a tick), through the loop's end when it crosses.
        let lo = tick_in(keep_from, 0).saturating_sub(1);
        let hi = (end_tick + 2).min(len);
        let reach: &[(u32, u32)] = if !crosses {
            &[(lo, hi)]
        } else if hi > lo {
            &[(0, len)]
        } else {
            &[(lo, len), (0, hi)]
        };
        for &(a, b) in reach {
            let i = lane.points.partition_point(|p| p.tick < a);
            let j = lane.points.partition_point(|p| p.tick < b);
            save(a, b, &lane.points[i..j]);
        }
        // The new points go on the end of the lane, a piece per pass.
        let old_len = lane.points.len();
        let mut split = None;
        let mut prev: Option<(f64, f32, Kind)> = None;
        for p in lead.into_iter().chain(points[start..].iter().copied()) {
            let pass = pass_of(p.0);
            if let Some(q) = prev.filter(|_| pass > 0 && split.is_none()) {
                // Crossing the loop's end: the line to the last tick, then
                // the next pass from tick 0.
                let edge = (base + 1.0) * lenf;
                let kind = q.2;
                push_point(
                    lane,
                    len - 1,
                    value_between(q, p, edge - 1.0, stepped),
                    kind,
                    stepped,
                );
                split = Some(lane.points.len());
                push_point(lane, 0, value_between(q, p, edge, stepped), kind, stepped);
            }
            push_point(lane, tick_in(p.0, pass), p.1, p.2, stepped);
            prev = Some(p);
        }
        let end = lane.points.len();
        let mid = split.unwrap_or(end);
        // Place the first pass's piece, then the second's (it sits after
        // the first; its place moves by what the first one changed).
        let after = end - mid;
        place(
            lane,
            old_len,
            mid,
            if crosses { None } else { back },
            stepped,
            len,
        );
        if after > 0 {
            let a = lane.points.len() - after;
            place(lane, a, lane.points.len(), back, stepped, len);
        }
        Ok(())
    }

    /// Puts back the points the lane for `target` had over clip ticks
    /// `from..to`, as [`Moves::put_saving`] saved them, in place of what it
    /// has there (making the lane with `map` if it has gone). A lane left
    /// with no points goes (freeing it). Otherwise it does not allocate
    /// while the lane has room.
    pub fn restore(
        &mut self,
        target: Target,
        map: Map,
        from: u32,
        to: u32,
        old: &[Point],
    ) -> Result<(), MovesError> {
        if old.iter().any(|p| p.tick < from || p.tick >= to)
            || old.windows(2).any(|w| w[1].tick <= w[0].tick)
        {
            return Err(MovesError::Bad);
        }
        let k = match self.lanes.iter().position(|l| l.target == target) {
            Some(k) => k,
            None if old.is_empty() => return Ok(()),
            None => self.lane_index(target, map)?,
        };
        let points = &mut self.lanes[k].points;
        let a = points.partition_point(|p| p.tick < from);
        let b = points.partition_point(|p| p.tick < to);
        if points.len() - (b - a) + old.len() > MAX_POINTS {
            return Err(MovesError::Full);
        }
        replace_range(points, a, b, old);
        if points.is_empty() {
            self.lanes.remove(k);
        }
        Ok(())
    }

    /// Takes every lane's points in `from..to` (clip ticks) out, leaving
    /// the lanes free there. Drops lanes left empty.
    pub fn clear(&mut self, from: u32, to: u32) {
        for lane in &mut self.lanes {
            lane.points.retain(|p| p.tick < from || p.tick >= to);
        }
        self.lanes.retain(|l| !l.points.is_empty());
    }
}

/// The knob position between two input points at `t` (held on a switch
/// or after a step).
fn value_between(a: (f64, f32, Kind), b: (f64, f32, Kind), t: f64, stepped: bool) -> f32 {
    if stepped || a.2 != Kind::Ramp || b.2 == Kind::End || b.0 <= a.0 {
        return a.1;
    }
    let f = ((t - a.0) / (b.0 - a.0)).clamp(0.0, 1.0) as f32;
    a.1 + (b.1 - a.1) * f
}

/// Puts a point on the end of the lane.
fn push_point(lane: &mut Lane, tick: u32, u: f32, kind: Kind, stepped: bool) {
    let kind = if stepped && kind == Kind::Ramp {
        Kind::Step
    } else {
        kind
    };
    let u = if u.is_finite() {
        u.clamp(0.0, 1.0)
    } else {
        0.0
    };
    lane.points.push(Point { tick, u, kind });
}

/// What a lane was doing at `tick` (its knob position and how it leads
/// on), or `None` where it was free.
fn lane_back(points: &[Point], tick: u32) -> Option<(f32, Kind)> {
    let i = points.partition_point(|q| q.tick <= tick);
    let q = *(if i == 0 {
        points.last()
    } else {
        points.get(i - 1)
    })?;
    match q.kind {
        Kind::End => None,
        Kind::Step => Some((q.u, Kind::Step)),
        Kind::Ramp => match points.get(i) {
            Some(&r) if i > 0 && r.kind != Kind::End => {
                Some((lerp(q, r, f64::from(tick)), Kind::Ramp))
            }
            _ => Some((q.u, Kind::Ramp)),
        },
    }
}

/// Moves the piece at `lane.points[a..b]` (in tick order, within one
/// pass, after the lane's sorted points `..a`; anything after `b` stays
/// after it) into place: the old points over the ticks it covers go, an
/// end hands back to `back` (what the old lane was doing there), and the
/// piece is thinned.
fn place(lane: &mut Lane, a: usize, b: usize, back: Option<(f32, Kind)>, stepped: bool, len: u32) {
    if b <= a {
        return;
    }
    // Points on the same tick: the later wins, except that an end right
    // after a value moves on a tick (so the value is heard).
    let mut w = a + 1;
    for r in a + 1..b {
        let mut p = lane.points[r];
        let q = lane.points[w - 1];
        if q.tick == p.tick && p.kind == Kind::End && q.kind != Kind::End && p.tick + 1 < len {
            p.tick += 1;
        }
        if q.tick >= p.tick {
            lane.points[w - 1] = Point {
                tick: q.tick.max(p.tick),
                ..p
            };
        } else {
            lane.points[w] = p;
            w += 1;
        }
    }
    remove_range(&mut lane.points, w, b);
    let mut b = w;
    // Punching into an old ramp: the old line goes on up to the tick
    // before the piece.
    let t0 = lane.points[a].tick;
    let lo = lane.points[..a].partition_point(|q| q.tick < t0);
    if lo > 0 && t0 > 0 {
        let q = lane.points[lo - 1];
        if let Some(&r) = lane.points[..a].get(lo) {
            if q.kind == Kind::Ramp && r.kind != Kind::End && q.tick + 1 < t0 {
                let u = lerp(q, r, f64::from(t0 - 1));
                lane.points.insert(a, Point::ramp(t0 - 1, u));
                b += 1;
            }
        }
    }
    let n = b - a;
    let (t0, t1) = (lane.points[a].tick, lane.points[b - 1].tick);
    if lane.points[b - 1].kind == Kind::End {
        if let Some((u, kind)) = back {
            lane.points[b - 1] = Point {
                tick: t1,
                u,
                kind: if stepped { Kind::Step } else { kind },
            };
        }
    }
    // The old points over t0..=t1 go: [..lo][lo..hi old][hi..a][piece]
    // becomes [..lo][piece][hi..a].
    let lo = lane.points[..a].partition_point(|q| q.tick < t0);
    let hi = lane.points[..a].partition_point(|q| q.tick <= t1);
    lane.points[lo..b].rotate_right(n);
    remove_range(&mut lane.points, lo + n, lo + n + (hi - lo));
    thin(&mut lane.points, lo, lo + n);
}

/// Puts `new` in place of `v[a..b]`, without allocating while `v` has
/// room.
fn replace_range(v: &mut Vec<Point>, a: usize, b: usize, new: &[Point]) {
    let n = new.len();
    if n <= b - a {
        v[a..a + n].copy_from_slice(new);
        remove_range(v, a + n, b);
    } else {
        let extra = n - (b - a);
        let end = v.len();
        v.resize(end + extra, Point::end(0));
        v.copy_within(b..end, b + extra);
        v[a..a + n].copy_from_slice(new);
    }
}

/// Removes `v[a..b]` without allocating.
fn remove_range(v: &mut Vec<Point>, a: usize, b: usize) {
    if b <= a {
        return;
    }
    let end = v.len();
    v.copy_within(b..end, a);
    v.truncate(end - (b - a));
}

/// Drops ramp points in `points[a..b]` that the line between the points
/// kept around them passes within [`THIN`] of, in place (no allocation).
/// Steps, ends and the first and last points stay.
pub fn thin_range(points: &mut Vec<Point>, a: usize, b: usize) {
    thin(points, a, b);
}

fn thin(points: &mut Vec<Point>, a: usize, b: usize) {
    let b = b.min(points.len());
    if b < a + 3 {
        return;
    }
    // Kept points are written at `w`; `anchor` is the last kept one's
    // index as it was (nothing at or after it has been written over).
    let mut w = a + 1;
    let mut anchor = a;
    let mut j = a + 1;
    while j < b {
        let pa = points[anchor];
        let pj = points[j];
        let drop = pa.kind == Kind::Ramp
            && pj.kind == Kind::Ramp
            && j + 1 < b
            && points[j + 1].kind != Kind::End
            && j - anchor < THIN_WINDOW
            && {
                let pn = points[j + 1];
                (anchor + 1..=j).all(|k| {
                    let q = points[k];
                    (lerp(pa, pn, f64::from(q.tick)) - q.u).abs() <= THIN
                })
            };
        if !drop {
            points[w] = pj;
            w += 1;
            anchor = j;
        }
        j += 1;
    }
    remove_range(points, w, b);
}
