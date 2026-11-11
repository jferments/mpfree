//! Where each moment of the input lands in the output.

use crate::StretchError;

/// The steepest and shallowest a stretch may be anywhere (output frames
/// per input frame).
pub const MIN_SLOPE: f64 = 0.1;
pub const MAX_SLOPE: f64 = 10.0;

/// A piecewise-linear map from input frames to output frames: warp
/// markers, or one straight line for a plain stretch. It starts at (0, 0),
/// both columns rise strictly, and it ends at the input's last frame.
#[derive(Clone, Debug, PartialEq)]
pub struct TimeMap {
    points: Vec<(f64, f64)>,
}

impl TimeMap {
    /// The whole input stretched evenly: `ratio` 2 doubles its length.
    pub fn ratio(input_frames: u64, ratio: f64) -> Result<TimeMap, StretchError> {
        if input_frames == 0 {
            return Err(StretchError::Empty);
        }
        let n = input_frames as f64;
        TimeMap::new(vec![(0.0, 0.0), (n, n * ratio)])
    }

    /// The input lasting exactly `output_frames`.
    pub fn to_length(input_frames: u64, output_frames: u64) -> Result<TimeMap, StretchError> {
        if input_frames == 0 || output_frames == 0 {
            return Err(StretchError::Empty);
        }
        TimeMap::new(vec![
            (0.0, 0.0),
            (input_frames as f64, output_frames as f64),
        ])
    }

    /// Warp markers as (input frame, output frame) pairs, from (0, 0).
    pub fn new(points: Vec<(f64, f64)>) -> Result<TimeMap, StretchError> {
        if points.len() < 2 {
            return Err(StretchError::Map("needs two points or more"));
        }
        if points[0] != (0.0, 0.0) {
            return Err(StretchError::Map("must start at (0, 0)"));
        }
        for w in points.windows(2) {
            let (a, b) = (w[0], w[1]);
            if !(b.0.is_finite() && b.1.is_finite()) {
                return Err(StretchError::Map("not a number"));
            }
            if b.0 <= a.0 || b.1 <= a.1 {
                return Err(StretchError::Map("points must rise"));
            }
            let slope = (b.1 - a.1) / (b.0 - a.0);
            if !(MIN_SLOPE..=MAX_SLOPE).contains(&slope) {
                return Err(StretchError::Map("a stretch past 10x either way"));
            }
        }
        Ok(TimeMap { points })
    }

    pub fn points(&self) -> &[(f64, f64)] {
        &self.points
    }

    /// The input frames the map covers.
    pub fn input_frames(&self) -> f64 {
        self.points[self.points.len() - 1].0
    }

    /// How long the output is.
    pub fn output_frames(&self) -> u64 {
        self.points[self.points.len() - 1].1.round() as u64
    }

    /// Where input frame `x` lands. Past either end the end segments carry
    /// on.
    pub fn output_at(&self, x: f64) -> f64 {
        let i = segment(&self.points, x, |p| p.0);
        let (a, b) = (self.points[i], self.points[i + 1]);
        a.1 + (x - a.0) * (b.1 - a.1) / (b.0 - a.0)
    }

    /// The input frame that lands at output frame `y`.
    pub fn input_at(&self, y: f64) -> f64 {
        let i = segment(&self.points, y, |p| p.1);
        let (a, b) = (self.points[i], self.points[i + 1]);
        a.0 + (y - a.1) * (b.0 - a.0) / (b.1 - a.1)
    }

    /// The same map with every output time times `k` (for a pitch shift,
    /// which stretches by `k` and then plays `k` times faster).
    pub(crate) fn scaled(&self, k: f64) -> TimeMap {
        TimeMap {
            points: self.points.iter().map(|&(x, y)| (x, y * k)).collect(),
        }
    }

    /// The map with each hit in `onsets` (input frames) played at its own
    /// speed from `before` frames ahead of it to `after` frames past it,
    /// the rest of the stretch taken up between hits, so every window that
    /// holds the attack reads it as it was. A hit too close to a marker or
    /// the last hit for that is left as it was.
    pub(crate) fn locked(&self, onsets: &[u64], before: f64, after: f64) -> TimeMap {
        let mut points = self.points.clone();
        let end = self.input_frames();
        let ok = |s: f64| (MIN_SLOPE..=MAX_SLOPE).contains(&s);
        let mut onsets = onsets.to_vec();
        onsets.sort_unstable();
        for o in onsets {
            let u = o as f64;
            if u <= 0.0 || u >= end {
                continue;
            }
            // Where the hit lands stays where the markers put it.
            let v = self.output_at(u);
            let i = segment(&points, u, |p| p.0);
            let (prev, next) = (points[i], points[i + 1]);
            let b = before.min(0.4 * (u - prev.0)).min(0.4 * (v - prev.1));
            let a = after.min(0.4 * (next.0 - u)).min(0.4 * (next.1 - v));
            if b < 1.0 || a < 1.0 {
                continue;
            }
            let (pa, pb) = ((u - b, v - b), (u + a, v + a));
            let slope = |p: (f64, f64), q: (f64, f64)| (q.1 - p.1) / (q.0 - p.0);
            if !ok(slope(prev, pa)) || !ok(slope(pb, next)) {
                continue;
            }
            points.insert(i + 1, pa);
            points.insert(i + 2, pb);
        }
        TimeMap { points }
    }
}

/// The segment (index of its first point) holding `t` on the axis `key`
/// picks; the end segments for times past either end.
fn segment(points: &[(f64, f64)], t: f64, key: impl Fn(&(f64, f64)) -> f64) -> usize {
    let last = points.len() - 2;
    // The first point past t, less one.
    let i = points.partition_point(|p| key(p) <= t);
    i.saturating_sub(1).min(last)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ratio_maps_both_ways_and_carries_on_past_the_ends() {
        let m = TimeMap::ratio(1000, 2.0).unwrap();
        assert_eq!(m.output_frames(), 2000);
        assert_eq!(m.output_at(250.0), 500.0);
        assert_eq!(m.input_at(500.0), 250.0);
        assert_eq!(m.input_at(-100.0), -50.0);
        assert_eq!(m.input_at(2100.0), 1050.0);
    }

    #[test]
    fn markers_must_start_at_zero_and_rise_within_ten_times() {
        assert!(TimeMap::new(vec![(0.0, 0.0), (10.0, 20.0), (20.0, 25.0)]).is_ok());
        assert!(TimeMap::new(vec![(0.0, 1.0), (10.0, 20.0)]).is_err());
        assert!(TimeMap::new(vec![(0.0, 0.0), (10.0, 20.0), (10.0, 30.0)]).is_err());
        assert!(TimeMap::new(vec![(0.0, 0.0), (10.0, 200.0)]).is_err());
        assert!(TimeMap::ratio(0, 1.0).is_err());
    }

    #[test]
    fn a_held_hit_plays_at_its_own_speed_and_the_length_stays() {
        let m = TimeMap::ratio(10_000, 2.0).unwrap();
        let l = m.locked(&[4000], 300.0, 500.0);
        assert_eq!(l.output_frames(), 20_000);
        let v = l.output_at(4000.0);
        assert_eq!(v, 8000.0);
        assert_eq!(l.output_at(4500.0) - v, 500.0, "the hold is 1:1");
        assert_eq!(v - l.output_at(3700.0), 300.0, "from before the hit");
        assert_eq!(l.input_at(l.output_at(7000.0)), 7000.0);
        // Every hit lands where the plain map puts it, however many.
        let many = m.locked(&[1000, 2500, 4000, 5500, 8000], 300.0, 500.0);
        for u in [1000.0, 2500.0, 4000.0, 5500.0, 8000.0] {
            assert_eq!(many.output_at(u), m.output_at(u), "hit at {u}");
            assert_eq!(many.output_at(u + 400.0) - many.output_at(u), 400.0);
        }
        assert_eq!(many.output_frames(), 20_000);
        // A hit too near the end is left alone.
        let near = m.locked(&[9999], 300.0, 500.0);
        assert_eq!(near.points().len(), 2);
    }
}
