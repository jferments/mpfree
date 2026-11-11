//! Edits that move a clip's ticks around, for the step pages: copy a
//! stretch over another, cut one out and close the gap, open a gap, cut
//! the clip short. Each keeps what every lane does on the ticks it does
//! not touch (a ramp running into an edit keeps its line, and what comes
//! after picks up as it was), and a check for a clip's moves.
//!
//! These allocate: control thread only.

use crate::{lerp, Kind, Lane, Moves, MovesError, Point, MAX_LANES, MAX_POINTS};

/// Whether a lane has a point on `tick`.
fn has(points: &[Point], tick: u32) -> bool {
    points.binary_search_by(|p| p.tick.cmp(&tick)).is_ok()
}

/// The point that makes a lane do from `tick` on what it did there: the
/// point on `tick` if there is one, else the line it was on, the value it
/// held, or an end where it was free. `None` for a lane with no points.
fn state_point(points: &[Point], tick: u32) -> Option<Point> {
    let i = points.partition_point(|q| q.tick < tick);
    if let Some(&q) = points.get(i).filter(|q| q.tick == tick) {
        return Some(q);
    }
    let &a = (if i == 0 {
        points.last()
    } else {
        points.get(i - 1)
    })?;
    Some(match a.kind {
        Kind::End => Point::end(tick),
        Kind::Step => Point::step(tick, a.u),
        Kind::Ramp => match points.get(i) {
            Some(&b) if i > 0 && b.kind != Kind::End => {
                Point::ramp(tick, lerp(a, b, f64::from(tick)))
            }
            _ => Point::step(tick, a.u),
        },
    })
}

/// What a gap opened at `tick` holds: the lane's value just before it
/// carried on to `tick` (a ramp's value there), or an end where the lane
/// was free. A point on `tick` belongs after the gap.
fn gap_point(points: &[Point], tick: u32) -> Option<Point> {
    let i = points.partition_point(|q| q.tick < tick);
    let &a = (if i == 0 {
        points.last()
    } else {
        points.get(i - 1)
    })?;
    Some(match a.kind {
        Kind::End => Point::end(tick),
        Kind::Step => Point::step(tick, a.u),
        Kind::Ramp => match points.get(i) {
            Some(&b) if i > 0 && b.kind != Kind::End => {
                Point::step(tick, lerp(a, b, f64::from(tick)))
            }
            _ => Point::step(tick, a.u),
        },
    })
}

/// A point on `tick` so that a ramp the lane is in there keeps its line
/// (or its hold, where it leads into an end) up to `tick` whatever comes
/// after; `None` where no ramp leads through `tick` or a point is on it.
fn lead_point(points: &[Point], tick: u32) -> Option<Point> {
    if has(points, tick) {
        return None;
    }
    let i = points.partition_point(|q| q.tick < tick);
    let a = if i == 0 {
        points.last()
    } else {
        points.get(i - 1)
    }?;
    if a.kind != Kind::Ramp {
        return None;
    }
    state_point(points, tick)
}

/// Puts `p` in its place unless a point is on its tick already.
fn insert_point(points: &mut Vec<Point>, p: Option<Point>) {
    let Some(p) = p else {
        return;
    };
    if let Err(i) = points.binary_search_by(|q| q.tick.cmp(&p.tick)) {
        points.insert(i, p);
    }
}

/// Drops ends that change nothing: an end after an end, counting round
/// the loop (the first point follows the last). A lane of ends goes empty.
fn tidy(points: &mut Vec<Point>) {
    let mut prev_end = points.last().is_some_and(|p| p.kind == Kind::End);
    points.retain(|p| {
        let end = p.kind == Kind::End;
        let keep = !(end && prev_end);
        prev_end = end;
        keep
    });
}

fn shifted(p: &Point, to: u32) -> Point {
    Point { tick: to, ..*p }
}

impl Moves {
    /// Whether the moves are sound for a clip `len` ticks long: at most
    /// [`MAX_LANES`] lanes, one per target, maps with finite ranges (a
    /// log map above 0), at most [`MAX_POINTS`] points a lane, ticks
    /// below `len` and rising (one point a tick), knob positions 0 to 1.
    pub fn check(&self, len: u32) -> Result<(), MovesError> {
        if self.lanes.len() > MAX_LANES {
            return Err(MovesError::Bad);
        }
        for (i, lane) in self.lanes.iter().enumerate() {
            let m = lane.map;
            let bad = self.lanes[..i].iter().any(|l| l.target == lane.target)
                || !m.min.is_finite()
                || !m.max.is_finite()
                || m.min > m.max
                || (m.log && m.min <= 0.0)
                || lane.points.len() > MAX_POINTS
                || lane
                    .points
                    .iter()
                    .any(|p| p.tick >= len || !(0.0..=1.0).contains(&p.u))
                || lane.points.windows(2).any(|w| w[1].tick <= w[0].tick);
            if bad {
                return Err(MovesError::Bad);
            }
        }
        Ok(())
    }

    /// Copies every lane's moves over clip ticks `from..to` onto the ticks
    /// from `at` on (cut off at `len`), replacing what was there. The
    /// copy starts as the lane was on `from` and ends as it was on `to -
    /// 1`; after it, the lane picks up as it was. Copying a page onto
    /// another, or onto ticks a longer clip has just gained.
    pub fn copy_range(&mut self, from: u32, to: u32, at: u32, len: u32) -> Result<(), MovesError> {
        if to > len || at >= len || from > to {
            return Err(MovesError::Bad);
        }
        let w = (to - from).min(len - at);
        if w == 0 {
            return Ok(());
        }
        self.edit(|old| {
            let i_at = old.partition_point(|p| p.tick < at);
            let j = old.partition_point(|p| p.tick < at + w);
            let mut out: Vec<Point> = old[..i_at].to_vec();
            if at > 0 {
                insert_point(&mut out, lead_point(old, at - 1));
                if at + w == len && !has(old, 0) {
                    // The copy ends the clip: keep what the start did
                    // before its first point (the end of the last pass).
                    insert_point(&mut out, state_point(old, 0));
                }
            }
            // The stretch, as the lane was over it.
            out.extend(state_point(old, from).map(|p| shifted(&p, at)));
            out.extend(
                old.iter()
                    .filter(|p| p.tick > from && p.tick < from + w)
                    .map(|p| shifted(p, p.tick - from + at)),
            );
            if w > 1 {
                let last = from + w - 1;
                out.extend(lead_point(old, last).map(|p| shifted(&p, at + w - 1)));
            }
            // After it, the lane as it was.
            if at + w < len && !has(old, at + w) {
                out.extend(state_point(old, at + w));
            }
            out.extend_from_slice(&old[j..]);
            out
        })
    }

    /// Takes clip ticks `from..to` out of every lane of a clip `len` ticks
    /// long and closes the gap: what came after moves back by `to -
    /// from`, starting as the lane was on `to`. Taking a page off.
    pub fn cut(&mut self, from: u32, to: u32, len: u32) -> Result<(), MovesError> {
        if to > len || from > to {
            return Err(MovesError::Bad);
        }
        let w = to - from;
        if w == 0 {
            return Ok(());
        }
        self.edit(|old| {
            let i = old.partition_point(|p| p.tick < from);
            let j = old.partition_point(|p| p.tick < to);
            let mut out: Vec<Point> = old[..i].to_vec();
            if from > 0 {
                insert_point(&mut out, lead_point(old, from - 1));
                if to == len && !has(old, 0) {
                    insert_point(&mut out, state_point(old, 0));
                }
            }
            if to < len {
                if !has(old, to) {
                    out.extend(state_point(old, to).map(|p| shifted(&p, from)));
                }
                out.extend(old[j..].iter().map(|p| shifted(p, p.tick - w)));
            }
            out
        })
    }

    /// Opens `n` ticks at clip tick `at` in every lane: what was from `at`
    /// on moves on by `n`, and the gap holds what came before it (its
    /// value carried on to `at`, or free where it was free). Inserting a
    /// page; copy into it with [`Moves::copy_range`].
    pub fn insert(&mut self, at: u32, n: u32) -> Result<(), MovesError> {
        if n == 0 {
            return Ok(());
        }
        let last = self
            .lanes
            .iter()
            .filter_map(|l| l.points.last())
            .map(|p| p.tick)
            .max();
        if last.is_some_and(|t| t.checked_add(n).is_none()) {
            return Err(MovesError::Bad);
        }
        self.edit(|old| {
            let i = old.partition_point(|p| p.tick < at);
            if i == old.len() {
                // Nothing after: the lane holds on through the gap.
                return old.to_vec();
            }
            let mut out: Vec<Point> = old[..i].to_vec();
            // The gap holds (a ramp running through `at` stops there and
            // picks up after it).
            out.extend(gap_point(old, at));
            if !has(old, at) {
                out.extend(state_point(old, at).map(|p| shifted(&p, at + n)));
            }
            out.extend(old[i..].iter().map(|p| shifted(p, p.tick + n)));
            out
        })
    }

    /// Cuts every lane off at `len` ticks, for a clip made shorter: the
    /// ticks before `len` play as they did, and so does the clip's start
    /// (which follows on from the new end).
    pub fn truncate(&mut self, len: u32) -> Result<(), MovesError> {
        self.edit(|old| {
            if old.last().is_none_or(|p| p.tick < len) {
                return old.to_vec();
            }
            if len == 0 {
                return Vec::new();
            }
            let i = old.partition_point(|p| p.tick < len);
            let mut out: Vec<Point> = old[..i].to_vec();
            if !has(old, 0) {
                insert_point(&mut out, state_point(old, 0));
            }
            insert_point(&mut out, lead_point(old, len - 1));
            out
        })
    }

    /// Rewrites each lane's points with `f` (given them as they were),
    /// drops ends that change nothing and lanes the edit left empty.
    /// Nothing changes when a lane would be too full.
    fn edit(&mut self, mut f: impl FnMut(&[Point]) -> Vec<Point>) -> Result<(), MovesError> {
        let mut lanes = Vec::with_capacity(self.lanes.len());
        for lane in &self.lanes {
            if lane.points.is_empty() {
                lanes.push(lane.clone());
                continue;
            }
            let mut points = f(&lane.points);
            tidy(&mut points);
            if points.len() > MAX_POINTS {
                return Err(MovesError::Full);
            }
            if !points.is_empty() {
                lanes.push(Lane {
                    target: lane.target,
                    map: lane.map,
                    points,
                });
            }
        }
        self.lanes = lanes;
        Ok(())
    }
}
