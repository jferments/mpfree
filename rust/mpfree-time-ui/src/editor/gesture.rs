//! Two fingers on the roll move the view (piano roll plan 1A.1): a pinch
//! zooms round the point between them, and moving them together scrolls.
//! Which way a pinch zooms (time, pitch or both) is picked by its first
//! movement and kept until the fingers lift, so a pinch across never
//! drifts in pitch. Nothing here touches the clip: a pinch sends nothing
//! to the time engine.

use super::zoom::RollView;
use mpfree_transport::time::Tick;
use mpfree_ui::Rect;

/// The spread has to change this much (in pixels) before a pinch picks
/// its way.
pub const AXIS_SLOP: i32 = 16;
/// Under this spread (in pixels) a pinch doesn't zoom that way: fingers
/// one above the other don't zoom time by their few pixels across.
const MIN_SPREAD: i32 = 24;

/// Which way a pinch zooms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    Time,
    Pitch,
    Both,
}

/// A finger of a pinch: its id, where it was when the pinch began, and
/// where it is now.
pub type Finger = (u64, (i32, i32), (i32, i32));

/// Two fingers on the roll: where each went down and is now, the view
/// when the second came down, and the way it zooms once picked.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pinch {
    pub fingers: [Finger; 2],
    pub from: RollView,
    pub axis: Option<Axis>,
}

impl Pinch {
    /// Fingers `a` and `b` (id and place) on a roll showing `from`.
    pub fn new(a: (u64, i32, i32), b: (u64, i32, i32), from: RollView) -> Pinch {
        Pinch {
            fingers: [(a.0, (a.1, a.2), (a.1, a.2)), (b.0, (b.1, b.2), (b.1, b.2))],
            from,
            axis: None,
        }
    }

    /// It is finger `f`'s.
    pub fn has(&self, f: u64) -> bool {
        self.fingers.iter().any(|g| g.0 == f)
    }

    /// The other finger than `f`, where it is now.
    pub fn other(&self, f: u64) -> Option<(u64, i32, i32)> {
        self.fingers
            .iter()
            .find(|g| g.0 != f)
            .map(|g| (g.0, g.2 .0, g.2 .1))
    }

    /// Finger `f` moved to (`x`, `y`); the way is picked once the spread
    /// has changed by [`AXIS_SLOP`].
    pub fn moved(&mut self, f: u64, x: i32, y: i32) {
        if let Some(g) = self.fingers.iter_mut().find(|g| g.0 == f) {
            g.2 = (x, y);
        }
        if self.axis.is_none() {
            let (sx0, sy0) = self.spread(false);
            let (sx, sy) = self.spread(true);
            let (dx, dy) = ((sx - sx0).abs(), (sy - sy0).abs());
            if dx.max(dy) >= AXIS_SLOP {
                self.axis = Some(if dx > 2 * dy {
                    Axis::Time
                } else if dy > 2 * dx {
                    Axis::Pitch
                } else {
                    Axis::Both
                });
            }
        }
    }

    /// How far apart the fingers are across and up, when the pinch began
    /// or now.
    fn spread(&self, now: bool) -> (i32, i32) {
        let at = |g: &Finger| if now { g.2 } else { g.1 };
        let (a, b) = (at(&self.fingers[0]), at(&self.fingers[1]));
        ((a.0 - b.0).abs(), (a.1 - b.1).abs())
    }

    /// The point between the fingers, when the pinch began or now.
    fn mid(&self, now: bool) -> (i32, i32) {
        let at = |g: &Finger| if now { g.2 } else { g.1 };
        let (a, b) = (at(&self.fingers[0]), at(&self.fingers[1]));
        ((a.0 + b.0) / 2, (a.1 + b.1) / 2)
    }

    /// The view now: the tick and row that were between the fingers are
    /// between them still, zoomed by how far the fingers spread (the way
    /// it picked only).
    pub fn view(&self, roll: Rect, length: Tick, rows: u8) -> RollView {
        let v0 = self.from;
        let (m0, m) = (self.mid(false), self.mid(true));
        let (s0, s) = (self.spread(false), self.spread(true));
        let scale = |a: i32, b: i32| {
            if a >= MIN_SPREAD && b >= MIN_SPREAD {
                f64::from(b) / f64::from(a)
            } else {
                1.0
            }
        };
        let (time, pitch) = match self.axis {
            None => (false, false),
            Some(Axis::Time) => (true, false),
            Some(Axis::Pitch) => (false, true),
            Some(Axis::Both) => (true, true),
        };
        let kx = if time { scale(s0.0, s.0) } else { 1.0 };
        let ky = if pitch { scale(s0.1, s.1) } else { 1.0 };
        let tick = v0.tick_mid(roll, m0.0);
        let row = v0.row_mid(roll, m0.1);
        let v = v0.zoom_time(roll, length, rows, v0.ppt * kx, tick, m.0);
        let row_h = (f64::from(v0.row_h) * ky).round() as i32;
        v.zoom_rows(roll, length, rows, row_h, row, m.1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_transport::time::PPQ;

    const ROLL: Rect = Rect::new(92, 180, 1060, 440);
    const BAR: Tick = 4 * PPQ as Tick;

    fn from() -> RollView {
        RollView::open([(0, 60), (0, 64)], 4 * BAR, BAR, ROLL, 128)
    }

    /// Spread across: it zooms time only, however the fingers drift up or
    /// down after, and the tick between them stays between them.
    #[test]
    fn a_pinch_across_zooms_time_only() {
        let v0 = from();
        let mut p = Pinch::new((1, 400, 300), (2, 600, 300), v0);
        p.moved(1, 300, 302);
        assert_eq!(p.axis, Some(Axis::Time));
        p.moved(2, 700, 360);
        let v = p.view(ROLL, 4 * BAR, 128);
        assert!((v.ppt / v0.ppt - 2.0).abs() < 1e-9);
        assert_eq!(v.row_h, v0.row_h, "pitch kept");
        let t0 = v0.tick_mid(ROLL, 500);
        assert!((v.tick_mid(ROLL, 500) - t0).abs() <= 1.0 / v.ppt);
    }

    /// Spread up: pitch only; both ways at once: both.
    #[test]
    fn a_pinch_up_zooms_pitch_and_a_diagonal_both() {
        let v0 = from();
        let mut p = Pinch::new((1, 500, 250), (2, 500, 350), v0);
        p.moved(2, 503, 400);
        assert_eq!(p.axis, Some(Axis::Pitch));
        let v = p.view(ROLL, 4 * BAR, 128);
        assert_eq!(v.ppt, v0.ppt);
        assert!(v.row_h > v0.row_h);
        let mut p = Pinch::new((1, 400, 250), (2, 500, 350), v0);
        p.moved(2, 530, 380);
        assert_eq!(p.axis, Some(Axis::Both));
    }

    /// Two fingers moved together scroll: nothing zooms.
    #[test]
    fn two_fingers_together_scroll() {
        let v0 = RollView { ppt: 0.3, ..from() }.clamped(ROLL, 4 * BAR, 128);
        let mut p = Pinch::new((1, 400, 300), (2, 600, 300), v0);
        // As the screen reports them: a few pixels a finger at a time.
        for k in 1..=10 {
            p.moved(1, 400 - 10 * k, 300 + 4 * k);
            p.moved(2, 600 - 10 * k, 300 + 4 * k);
        }
        assert_eq!(p.axis, None);
        let v = p.view(ROLL, 4 * BAR, 128);
        assert_eq!((v.ppt, v.row_h), (v0.ppt, v0.row_h));
        // Dragged left by 100 px: 100 px of ticks on; down by 40: a row
        // higher shows.
        assert!((v.start - v0.start - (100.0 / v0.ppt) as Tick).abs() <= 1);
        assert_eq!(v.lift, (v0.lift + 40).min(128 * v0.row_h - ROLL.h));
    }
}
