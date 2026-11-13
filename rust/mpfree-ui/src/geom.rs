//! Integer rectangles in logical pixels: (0, 0) is the top left corner of the
//! screen as the player sees it.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { x, y, w, h }
    }

    pub const fn right(&self) -> i32 {
        self.x + self.w
    }

    pub const fn bottom(&self) -> i32 {
        self.y + self.h
    }

    pub const fn is_empty(&self) -> bool {
        self.w <= 0 || self.h <= 0
    }

    pub const fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && y >= self.y && x < self.right() && y < self.bottom()
    }

    /// The overlap of two rectangles; empty (zero size) if they do not overlap.
    pub fn intersect(&self, o: Rect) -> Rect {
        let x0 = self.x.max(o.x);
        let y0 = self.y.max(o.y);
        let x1 = self.right().min(o.right());
        let y1 = self.bottom().min(o.bottom());
        if x1 <= x0 || y1 <= y0 {
            Rect::new(x0, y0, 0, 0)
        } else {
            Rect::new(x0, y0, x1 - x0, y1 - y0)
        }
    }

    /// The smallest rectangle holding both (an empty one is ignored).
    pub fn union(&self, o: Rect) -> Rect {
        if self.is_empty() {
            return o;
        }
        if o.is_empty() {
            return *self;
        }
        let x0 = self.x.min(o.x);
        let y0 = self.y.min(o.y);
        Rect::new(
            x0,
            y0,
            self.right().max(o.right()) - x0,
            self.bottom().max(o.bottom()) - y0,
        )
    }

    /// Shrink by `d` on every side (never below zero size).
    pub fn inset(&self, d: i32) -> Rect {
        self.inset_xy(d, d)
    }

    pub fn inset_xy(&self, dx: i32, dy: i32) -> Rect {
        Rect::new(
            self.x + dx,
            self.y + dy,
            (self.w - 2 * dx).max(0),
            (self.h - 2 * dy).max(0),
        )
    }

    /// Split off the top `h` pixels: (top, rest).
    pub fn take_top(&self, h: i32) -> (Rect, Rect) {
        let h = h.clamp(0, self.h.max(0));
        (
            Rect::new(self.x, self.y, self.w, h),
            Rect::new(self.x, self.y + h, self.w, self.h - h),
        )
    }

    /// Split off the bottom `h` pixels: (bottom, rest).
    pub fn take_bottom(&self, h: i32) -> (Rect, Rect) {
        let h = h.clamp(0, self.h.max(0));
        (
            Rect::new(self.x, self.bottom() - h, self.w, h),
            Rect::new(self.x, self.y, self.w, self.h - h),
        )
    }

    /// Split off the left `w` pixels: (left, rest).
    pub fn take_left(&self, w: i32) -> (Rect, Rect) {
        let w = w.clamp(0, self.w.max(0));
        (
            Rect::new(self.x, self.y, w, self.h),
            Rect::new(self.x + w, self.y, self.w - w, self.h),
        )
    }

    /// Split off the right `w` pixels: (right, rest).
    pub fn take_right(&self, w: i32) -> (Rect, Rect) {
        let w = w.clamp(0, self.w.max(0));
        (
            Rect::new(self.right() - w, self.y, w, self.h),
            Rect::new(self.x, self.y, self.w - w, self.h),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlap_and_union() {
        let a = Rect::new(0, 0, 10, 10);
        let b = Rect::new(5, 5, 10, 10);
        assert_eq!(a.intersect(b), Rect::new(5, 5, 5, 5));
        assert!(a.intersect(Rect::new(20, 0, 5, 5)).is_empty());
        assert_eq!(a.union(b), Rect::new(0, 0, 15, 15));
        assert_eq!(Rect::default().union(b), b);
        assert!(a.contains(9, 9) && !a.contains(10, 9));
    }

    #[test]
    fn splits() {
        let r = Rect::new(10, 20, 100, 50);
        assert_eq!(
            r.take_top(10),
            (Rect::new(10, 20, 100, 10), Rect::new(10, 30, 100, 40))
        );
        assert_eq!(r.take_right(30).0, Rect::new(80, 20, 30, 50));
        assert_eq!(r.take_left(200).1.w, 0);
        assert_eq!(r.inset(30), Rect::new(40, 50, 40, 0));
    }
}
