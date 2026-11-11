//! Anti-aliased strokes for curves and knobs. They are drawn into a coverage
//! [`Mask`] and blended with [`Canvas::blend_mask`], so they work on any
//! canvas without new calls in `mpfree-ui`.

use mpfree_ui::{Canvas, Color, Rect};
use std::f32::consts::TAU;

/// Coverage (0 to 255) for each pixel of a rectangle of the screen. Shapes
/// add to it by taking the larger coverage, so overlapping strokes of one
/// colour don't darken where they cross.
pub struct Mask {
    pub r: Rect,
    pub px: Vec<u8>,
}

impl Mask {
    pub fn new(r: Rect) -> Mask {
        let n = (r.w.max(0) * r.h.max(0)) as usize;
        Mask { r, px: vec![0; n] }
    }

    /// The coverage at screen pixel (x, y), 0 outside the mask.
    pub fn at(&self, x: i32, y: i32) -> u8 {
        if !self.r.contains(x, y) {
            return 0;
        }
        self.px[((y - self.r.y) * self.r.w + (x - self.r.x)) as usize]
    }

    fn cover(&mut self, x: i32, y: i32, c: f32) {
        if c <= 0.0 || !self.r.contains(x, y) {
            return;
        }
        let i = ((y - self.r.y) * self.r.w + (x - self.r.x)) as usize;
        let v = (c.min(1.0) * 255.0).round() as u8;
        if v > self.px[i] {
            self.px[i] = v;
        }
    }

    /// The pixels whose centres lie within `pad` of the box, cut to the mask.
    fn span(&self, x0: f32, y0: f32, x1: f32, y1: f32, pad: f32) -> (i32, i32, i32, i32) {
        let xa = ((x0.min(x1) - pad).floor() as i32).max(self.r.x);
        let xb = ((x0.max(x1) + pad).ceil() as i32).min(self.r.right() - 1);
        let ya = ((y0.min(y1) - pad).floor() as i32).max(self.r.y);
        let yb = ((y0.max(y1) + pad).ceil() as i32).min(self.r.bottom() - 1);
        (xa, xb, ya, yb)
    }

    /// A line `width` pixels thick from `a` to `b` with round ends.
    pub fn segment(&mut self, a: (f32, f32), b: (f32, f32), width: f32) {
        let h = width * 0.5;
        let (xa, xb, ya, yb) = self.span(a.0, a.1, b.0, b.1, h + 1.0);
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let len2 = dx * dx + dy * dy;
        for y in ya..=yb {
            for x in xa..=xb {
                let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                let t = if len2 > 0.0 {
                    (((px - a.0) * dx + (py - a.1) * dy) / len2).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let (qx, qy) = (a.0 + t * dx - px, a.1 + t * dy - py);
                let d = (qx * qx + qy * qy).sqrt();
                self.cover(x, y, h + 0.5 - d);
            }
        }
    }

    /// Joined segments through `pts`.
    pub fn polyline(&mut self, pts: &[(f32, f32)], width: f32) {
        if pts.len() == 1 {
            self.segment(pts[0], pts[0], width);
        }
        for w in pts.windows(2) {
            self.segment(w[0], w[1], width);
        }
    }

    /// Every other piece of the polyline, `dash` pixels long, measured along it.
    pub fn dashed(&mut self, pts: &[(f32, f32)], width: f32, dash: f32) {
        let mut along = 0.0f32;
        for w in pts.windows(2) {
            let (a, b) = (w[0], w[1]);
            let len = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
            if len <= 0.0 {
                continue;
            }
            let mut t = 0.0;
            while t < len {
                let piece = (dash - along % dash).min(len - t);
                let on = (along / dash) as i64 % 2 == 0;
                if on {
                    let p0 = (a.0 + (b.0 - a.0) * t / len, a.1 + (b.1 - a.1) * t / len);
                    let p1 = (
                        a.0 + (b.0 - a.0) * (t + piece) / len,
                        a.1 + (b.1 - a.1) * (t + piece) / len,
                    );
                    self.segment(p0, p1, width);
                }
                t += piece;
                along += piece;
            }
        }
    }

    /// A filled circle.
    pub fn disc(&mut self, cx: f32, cy: f32, radius: f32) {
        let (xa, xb, ya, yb) = self.span(cx, cy, cx, cy, radius + 1.0);
        for y in ya..=yb {
            for x in xa..=xb {
                let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
                self.cover(x, y, radius + 0.5 - d);
            }
        }
    }

    /// An arc of a circle, `width` thick with round ends, from angle `a0` to
    /// `a1` (radians, `a0 <= a1`, clockwise on screen, 0 pointing right).
    pub fn arc(&mut self, cx: f32, cy: f32, radius: f32, a0: f32, a1: f32, width: f32) {
        if a1 <= a0 {
            return;
        }
        let h = width * 0.5;
        let ends = [
            (cx + radius * a0.cos(), cy + radius * a0.sin()),
            (cx + radius * a1.cos(), cy + radius * a1.sin()),
        ];
        let (xa, xb, ya, yb) = self.span(cx, cy, cx, cy, radius + h + 1.0);
        for y in ya..=yb {
            for x in xa..=xb {
                let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
                let ang = (dy.atan2(dx) - a0).rem_euclid(TAU) + a0;
                let d = if ang <= a1 {
                    ((dx * dx + dy * dy).sqrt() - radius).abs()
                } else {
                    ends.iter()
                        .map(|e| {
                            ((x as f32 + 0.5 - e.0).powi(2) + (y as f32 + 0.5 - e.1).powi(2)).sqrt()
                        })
                        .fold(f32::MAX, f32::min)
                };
                self.cover(x, y, h + 0.5 - d);
            }
        }
    }

    /// Fills between the polyline (x increasing) and the line `y = base`,
    /// with coverage `alpha` (0 to 1), for a soft area under a curve.
    pub fn fill_under(&mut self, pts: &[(f32, f32)], base: f32, alpha: f32) {
        for w in pts.windows(2) {
            let (a, b) = (w[0], w[1]);
            if b.0 <= a.0 {
                continue;
            }
            let x0 = (a.0.floor() as i32).max(self.r.x);
            let x1 = (b.0.ceil() as i32).min(self.r.right());
            for x in x0..x1 {
                let px = x as f32 + 0.5;
                if px < a.0 || px >= b.0 {
                    continue;
                }
                let yc = a.1 + (b.1 - a.1) * (px - a.0) / (b.0 - a.0);
                let (top, bot) = if yc < base { (yc, base) } else { (base, yc) };
                let ya = (top.round() as i32).max(self.r.y);
                let yb = (bot.round() as i32).min(self.r.bottom());
                for y in ya..yb {
                    self.cover(x, y, alpha);
                }
            }
        }
    }

    /// Blends the mask onto the canvas in colour `c`.
    pub fn draw(&self, canvas: &mut dyn Canvas, c: Color) {
        if self.r.is_empty() {
            return;
        }
        canvas.blend_mask(
            self.r.x,
            self.r.y,
            self.r.w as usize,
            self.r.h as usize,
            &self.px,
            c,
        );
    }
}

/// Strokes `pts` in colour `c` inside `r`.
pub fn stroke(canvas: &mut dyn Canvas, r: Rect, pts: &[(f32, f32)], width: f32, c: Color) {
    let mut m = Mask::new(r);
    m.polyline(pts, width);
    m.draw(canvas, c);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_thick_line_is_solid_in_the_middle_and_soft_at_the_edge() {
        let mut m = Mask::new(Rect::new(0, 0, 40, 20));
        m.segment((5.0, 10.25), (35.0, 10.25), 4.0);
        assert_eq!(m.at(20, 9), 255);
        assert_eq!(m.at(20, 10), 255);
        assert_eq!(m.at(20, 15), 0);
        let edge = m.at(20, 12);
        assert!(edge > 0 && edge < 255, "edge {edge}");
    }

    #[test]
    fn arcs_cover_only_their_angles() {
        let mut m = Mask::new(Rect::new(0, 0, 60, 60));
        // The lower half (0 to pi, clockwise on screen from the right).
        m.arc(30.0, 30.0, 20.0, 0.0, std::f32::consts::PI, 3.0);
        assert!(m.at(30, 50) > 200, "bottom is on");
        assert_eq!(m.at(30, 9), 0, "top is off");
        assert_eq!(m.at(30, 30), 0, "centre is off");
    }

    #[test]
    fn shapes_outside_the_mask_are_cut() {
        let mut m = Mask::new(Rect::new(10, 10, 10, 10));
        m.segment((-50.0, -50.0), (100.0, 100.0), 3.0);
        m.disc(500.0, 500.0, 30.0);
        assert!(m.at(15, 15) > 0);
        assert_eq!(m.px.len(), 100);
    }

    #[test]
    fn the_area_under_a_curve_is_filled_to_the_base() {
        let mut m = Mask::new(Rect::new(0, 0, 20, 20));
        m.fill_under(&[(0.0, 5.0), (20.0, 5.0)], 15.0, 0.5);
        assert_eq!(m.at(10, 10), 128);
        assert_eq!(m.at(10, 2), 0);
        assert_eq!(m.at(10, 17), 0);
    }
}
