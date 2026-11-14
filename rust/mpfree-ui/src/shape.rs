//! Anti-aliased vector shapes (lines, circles, arcs, the area under a curve)
//! for curves, knobs and graphs. A [`Shape`] keeps its parts, not pixels:
//! [`Canvas::blend_shape`] draws it, which in software rasterises it into a
//! coverage mask and blends that (the pixels the synth's masks always had),
//! and on the GPU sends the parts, so the GPU draws them itself instead of
//! being handed a picture of them.

use crate::canvas::Canvas;
use crate::color::Color;
use crate::geom::Rect;
use std::f32::consts::TAU;

/// One piece of a [`Shape`]. Points are in screen pixels (fractions
/// allowed); a pixel's coverage is measured at its centre.
#[derive(Clone, Copy, Debug)]
pub enum Part {
    /// A line `width` pixels thick from `a` to `b` with round ends.
    Segment {
        a: (f32, f32),
        b: (f32, f32),
        width: f32,
    },
    /// A filled circle.
    Disc { c: (f32, f32), radius: f32 },
    /// An arc of a circle, `width` thick with round ends, from angle `a0` to
    /// `a1` (radians, `a0 < a1`, clockwise on screen, 0 pointing right).
    Arc {
        c: (f32, f32),
        radius: f32,
        a0: f32,
        a1: f32,
        width: f32,
    },
    /// The pixels between the line from `a` to `b` (`a.0 < b.0`) and the
    /// line `y = base`, at coverage `alpha` (0 to 1).
    Under {
        a: (f32, f32),
        b: (f32, f32),
        base: f32,
        alpha: f32,
    },
}

impl Part {
    /// The part's numbers, in a fixed order (for hashing, comparing and
    /// the wire format): the tag, then the fields as written above.
    pub fn numbers(&self) -> (u8, Vec<f32>) {
        match *self {
            Part::Segment { a, b, width } => (0, vec![a.0, a.1, b.0, b.1, width]),
            Part::Disc { c, radius } => (1, vec![c.0, c.1, radius]),
            Part::Arc {
                c,
                radius,
                a0,
                a1,
                width,
            } => (2, vec![c.0, c.1, radius, a0, a1, width]),
            Part::Under { a, b, base, alpha } => (3, vec![a.0, a.1, b.0, b.1, base, alpha]),
        }
    }

    /// The part back from [`Part::numbers`]; None for an unknown tag or the
    /// wrong count.
    pub fn from_numbers(tag: u8, v: &[f32]) -> Option<Part> {
        Some(match (tag, v.len()) {
            (0, 5) => Part::Segment {
                a: (v[0], v[1]),
                b: (v[2], v[3]),
                width: v[4],
            },
            (1, 3) => Part::Disc {
                c: (v[0], v[1]),
                radius: v[2],
            },
            (2, 6) => Part::Arc {
                c: (v[0], v[1]),
                radius: v[2],
                a0: v[3],
                a1: v[4],
                width: v[5],
            },
            (3, 6) => Part::Under {
                a: (v[0], v[1]),
                b: (v[2], v[3]),
                base: v[4],
                alpha: v[5],
            },
            _ => return None,
        })
    }

    /// How many numbers a part with this tag has; None for an unknown tag.
    pub fn count_for(tag: u8) -> Option<usize> {
        match tag {
            0 => Some(5),
            1 => Some(3),
            2 | 3 => Some(6),
            _ => None,
        }
    }
}

/// Coverage for each pixel of a rectangle of the screen, from parts that
/// add to it by taking the larger coverage, so overlapping strokes of one
/// colour don't darken where they cross. Drawn in one colour.
#[derive(Clone, Debug)]
pub struct Shape {
    pub r: Rect,
    pub parts: Vec<Part>,
}

impl PartialEq for Shape {
    /// The same rectangle and parts, number for number (bit for bit, so a
    /// shape always equals itself).
    fn eq(&self, other: &Shape) -> bool {
        self.r == other.r
            && self.parts.len() == other.parts.len()
            && self.parts.iter().zip(&other.parts).all(|(a, b)| {
                let (ta, va) = a.numbers();
                let (tb, vb) = b.numbers();
                ta == tb && va.iter().zip(&vb).all(|(x, y)| x.to_bits() == y.to_bits())
            })
    }
}

impl Eq for Shape {}

impl Shape {
    pub fn new(r: Rect) -> Shape {
        Shape {
            r,
            parts: Vec::new(),
        }
    }

    /// A line `width` pixels thick from `a` to `b` with round ends.
    pub fn segment(&mut self, a: (f32, f32), b: (f32, f32), width: f32) {
        self.parts.push(Part::Segment { a, b, width });
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
        self.parts.push(Part::Disc {
            c: (cx, cy),
            radius,
        });
    }

    /// An arc of a circle, `width` thick with round ends, from angle `a0` to
    /// `a1` (radians, `a0 <= a1`, clockwise on screen, 0 pointing right).
    pub fn arc(&mut self, cx: f32, cy: f32, radius: f32, a0: f32, a1: f32, width: f32) {
        if a1 <= a0 {
            return;
        }
        self.parts.push(Part::Arc {
            c: (cx, cy),
            radius,
            a0,
            a1,
            width,
        });
    }

    /// Fills between the polyline (x increasing) and the line `y = base`,
    /// with coverage `alpha` (0 to 1), for a soft area under a curve.
    pub fn fill_under(&mut self, pts: &[(f32, f32)], base: f32, alpha: f32) {
        for w in pts.windows(2) {
            let (a, b) = (w[0], w[1]);
            if b.0 <= a.0 {
                continue;
            }
            self.parts.push(Part::Under { a, b, base, alpha });
        }
    }

    /// Blends the shape onto the canvas in colour `c`.
    pub fn draw(&self, canvas: &mut dyn Canvas, c: Color) {
        if self.r.is_empty() {
            return;
        }
        canvas.blend_shape(self, c);
    }

    /// The coverage of every pixel of `r` (0 to 255, rows top to bottom):
    /// what software blends.
    pub fn coverage(&self) -> Vec<u8> {
        let mut m = Raster::new(self.r);
        for p in &self.parts {
            match *p {
                Part::Segment { a, b, width } => m.segment(a, b, width),
                Part::Disc { c, radius } => m.disc(c.0, c.1, radius),
                Part::Arc {
                    c,
                    radius,
                    a0,
                    a1,
                    width,
                } => m.arc(c.0, c.1, radius, a0, a1, width),
                Part::Under { a, b, base, alpha } => m.under(a, b, base, alpha),
            }
        }
        m.px
    }

    /// The coverage at screen pixel (x, y), 0 outside the shape. It
    /// rasterises the whole shape: for tests.
    pub fn at(&self, x: i32, y: i32) -> u8 {
        if !self.r.contains(x, y) {
            return 0;
        }
        self.coverage()[((y - self.r.y) * self.r.w + (x - self.r.x)) as usize]
    }
}

/// The software rasteriser: coverage 0 to 255 per pixel of `r`.
struct Raster {
    r: Rect,
    px: Vec<u8>,
}

impl Raster {
    fn new(r: Rect) -> Raster {
        let n = (r.w.max(0) * r.h.max(0)) as usize;
        Raster { r, px: vec![0; n] }
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

    fn segment(&mut self, a: (f32, f32), b: (f32, f32), width: f32) {
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

    fn disc(&mut self, cx: f32, cy: f32, radius: f32) {
        let (xa, xb, ya, yb) = self.span(cx, cy, cx, cy, radius + 1.0);
        for y in ya..=yb {
            for x in xa..=xb {
                let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
                self.cover(x, y, radius + 0.5 - d);
            }
        }
    }

    fn arc(&mut self, cx: f32, cy: f32, radius: f32, a0: f32, a1: f32, width: f32) {
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

    fn under(&mut self, a: (f32, f32), b: (f32, f32), base: f32, alpha: f32) {
        if b.0 <= a.0 {
            return;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_thick_line_is_solid_in_the_middle_and_soft_at_the_edge() {
        let mut m = Shape::new(Rect::new(0, 0, 40, 20));
        m.segment((5.0, 10.25), (35.0, 10.25), 4.0);
        assert_eq!(m.at(20, 9), 255);
        assert_eq!(m.at(20, 10), 255);
        assert_eq!(m.at(20, 15), 0);
        let edge = m.at(20, 12);
        assert!(edge > 0 && edge < 255, "edge {edge}");
    }

    #[test]
    fn arcs_cover_only_their_angles() {
        let mut m = Shape::new(Rect::new(0, 0, 60, 60));
        // The lower half (0 to pi, clockwise on screen from the right).
        m.arc(30.0, 30.0, 20.0, 0.0, std::f32::consts::PI, 3.0);
        assert!(m.at(30, 50) > 200, "bottom is on");
        assert_eq!(m.at(30, 9), 0, "top is off");
        assert_eq!(m.at(30, 30), 0, "centre is off");
    }

    #[test]
    fn shapes_outside_the_rectangle_are_cut() {
        let mut m = Shape::new(Rect::new(10, 10, 10, 10));
        m.segment((-50.0, -50.0), (100.0, 100.0), 3.0);
        m.disc(500.0, 500.0, 30.0);
        assert!(m.at(15, 15) > 0);
        assert_eq!(m.coverage().len(), 100);
    }

    #[test]
    fn the_area_under_a_curve_is_filled_to_the_base() {
        let mut m = Shape::new(Rect::new(0, 0, 20, 20));
        m.fill_under(&[(0.0, 5.0), (20.0, 5.0)], 15.0, 0.5);
        assert_eq!(m.at(10, 10), 128);
        assert_eq!(m.at(10, 2), 0);
        assert_eq!(m.at(10, 17), 0);
    }

    #[test]
    fn crossing_strokes_take_the_larger_coverage() {
        let mut m = Shape::new(Rect::new(0, 0, 20, 20));
        m.segment((0.0, 10.0), (20.0, 10.0), 2.0);
        m.segment((10.0, 0.0), (10.0, 20.0), 2.0);
        let one = {
            let mut s = Shape::new(Rect::new(0, 0, 20, 20));
            s.segment((0.0, 10.0), (20.0, 10.0), 2.0);
            s.at(10, 10)
        };
        assert_eq!(m.at(10, 10), one);
    }

    #[test]
    fn parts_go_to_numbers_and_back() {
        let parts = [
            Part::Segment {
                a: (1.5, 2.0),
                b: (3.0, -4.25),
                width: 2.0,
            },
            Part::Disc {
                c: (5.0, 6.0),
                radius: 3.5,
            },
            Part::Arc {
                c: (1.0, 1.0),
                radius: 9.0,
                a0: 0.5,
                a1: 2.0,
                width: 1.5,
            },
            Part::Under {
                a: (0.0, 1.0),
                b: (8.0, 2.0),
                base: 10.0,
                alpha: 0.25,
            },
        ];
        for p in parts {
            let (tag, v) = p.numbers();
            assert_eq!(Part::count_for(tag), Some(v.len()));
            let back = Part::from_numbers(tag, &v).unwrap();
            assert_eq!(back.numbers(), (tag, v));
        }
        assert!(Part::from_numbers(9, &[]).is_none());
        assert!(Part::from_numbers(1, &[1.0]).is_none());
        let mut a = Shape::new(Rect::new(0, 0, 4, 4));
        a.disc(1.0, 1.0, f32::NAN);
        assert_eq!(a, a.clone(), "a shape equals itself, even with NaN");
    }
}
