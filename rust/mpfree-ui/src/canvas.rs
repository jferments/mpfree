//! The drawing interface every widget uses, and a software implementation.

use crate::color::{blend, Color};
use crate::geom::Rect;

/// Everything a widget may draw. Coordinates are logical pixels; drawing is
/// cut to the current clip rectangle. A GPU backend implements the same calls.
pub trait Canvas {
    fn size(&self) -> (i32, i32);
    fn clip(&self) -> Rect;
    /// Restrict drawing to `r` (cut to the canvas).
    fn set_clip(&mut self, r: Rect);
    fn fill_rect(&mut self, r: Rect, c: Color);
    /// A filled rectangle with round, anti-aliased corners.
    fn fill_round_rect(&mut self, r: Rect, radius: i32, c: Color);
    /// The outline of a rounded rectangle, `width` pixels thick, inside `r`.
    fn stroke_round_rect(&mut self, r: Rect, radius: i32, width: i32, c: Color);
    /// Blend `c` through a coverage mask (one byte per pixel, `w` x `h`, rows
    /// top to bottom) whose top left corner lands at (x, y). Text uses this.
    fn blend_mask(&mut self, x: i32, y: i32, w: usize, h: usize, mask: &[u8], c: Color);
}

/// A canvas in memory, one XRGB8888 word per pixel, rows top to bottom.
pub struct SoftCanvas {
    w: i32,
    h: i32,
    pub px: Vec<u32>,
    clip: Rect,
}

impl SoftCanvas {
    pub fn new(w: i32, h: i32) -> SoftCanvas {
        assert!(w > 0 && h > 0 && w <= 8192 && h <= 8192, "canvas size {w}x{h}");
        SoftCanvas { w, h, px: vec![0; (w * h) as usize], clip: Rect::new(0, 0, w, h) }
    }

    pub fn width(&self) -> i32 {
        self.w
    }

    pub fn height(&self) -> i32 {
        self.h
    }

    pub fn pixel(&self, x: i32, y: i32) -> Color {
        Color::from_xrgb(self.px[(y * self.w + x) as usize])
    }

    /// The whole canvas as RGB bytes (for PNG files).
    pub fn to_rgb(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.px.len() * 3);
        for &p in &self.px {
            out.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, p as u8]);
        }
        out
    }

    fn span(&mut self, y: i32, x0: i32, x1: i32, c: Color) {
        let x0 = x0.max(self.clip.x);
        let x1 = x1.min(self.clip.right());
        if x1 <= x0 || y < self.clip.y || y >= self.clip.bottom() {
            return;
        }
        let row = (y * self.w) as usize;
        let s = &mut self.px[row + x0 as usize..row + x1 as usize];
        if c.a == 255 {
            s.fill(c.xrgb());
        } else {
            for p in s {
                *p = blend(*p, c, 255);
            }
        }
    }

    fn put(&mut self, x: i32, y: i32, c: Color, cov: f32) {
        if cov <= 0.0 || !self.clip.contains(x, y) {
            return;
        }
        let i = (y * self.w + x) as usize;
        self.px[i] = blend(self.px[i], c, (cov.min(1.0) * 255.0 + 0.5) as u32);
    }
}

/// How much of pixel (px, py) lies inside the rounded rectangle `r`.
fn coverage(r: Rect, radius: f32, px: i32, py: i32) -> f32 {
    if r.is_empty() {
        return 0.0;
    }
    let (fx, fy) = (px as f32 + 0.5, py as f32 + 0.5);
    if fx < r.x as f32 || fy < r.y as f32 || fx > r.right() as f32 || fy > r.bottom() as f32 {
        return 0.0;
    }
    if radius < 0.5 {
        return 1.0;
    }
    let cx = fx.clamp(r.x as f32 + radius, r.right() as f32 - radius);
    let cy = fy.clamp(r.y as f32 + radius, r.bottom() as f32 - radius);
    let d = ((fx - cx).powi(2) + (fy - cy).powi(2)).sqrt();
    (radius - d + 0.5).clamp(0.0, 1.0)
}

fn radius_for(r: Rect, radius: i32) -> i32 {
    radius.clamp(0, r.w.min(r.h) / 2)
}

impl Canvas for SoftCanvas {
    fn size(&self) -> (i32, i32) {
        (self.w, self.h)
    }

    fn clip(&self) -> Rect {
        self.clip
    }

    fn set_clip(&mut self, r: Rect) {
        self.clip = r.intersect(Rect::new(0, 0, self.w, self.h));
    }

    fn fill_rect(&mut self, r: Rect, c: Color) {
        let r = r.intersect(self.clip);
        for y in r.y..r.bottom() {
            self.span(y, r.x, r.right(), c);
        }
    }

    fn fill_round_rect(&mut self, r: Rect, radius: i32, c: Color) {
        let rad = radius_for(r, radius);
        if rad == 0 {
            return self.fill_rect(r, c);
        }
        let rows = r.intersect(self.clip);
        for y in rows.y..rows.bottom() {
            if y >= r.y + rad && y < r.bottom() - rad {
                self.span(y, r.x, r.right(), c);
                continue;
            }
            for x in r.x..r.x + rad {
                self.put(x, y, c, coverage(r, rad as f32, x, y));
            }
            self.span(y, r.x + rad, r.right() - rad, c);
            for x in r.right() - rad..r.right() {
                self.put(x, y, c, coverage(r, rad as f32, x, y));
            }
        }
    }

    fn stroke_round_rect(&mut self, r: Rect, radius: i32, width: i32, c: Color) {
        let rad = radius_for(r, radius);
        let width = width.max(1);
        let inner = r.inset(width);
        let irad = (rad - width).max(0);
        // Rows clear of both the corners and the top and bottom borders: just
        // the left and right borders, solid.
        let m = rad.max(width);
        let rows = r.intersect(self.clip);
        for y in rows.y..rows.bottom() {
            if y >= r.y + m && y < r.bottom() - m {
                self.span(y, r.x, inner.x, c);
                self.span(y, inner.right(), r.right(), c);
                continue;
            }
            for x in r.x..r.right() {
                let outer = coverage(r, rad as f32, x, y);
                if outer <= 0.0 {
                    continue;
                }
                let cut = if inner.is_empty() { 0.0 } else { coverage(inner, irad as f32, x, y) };
                self.put(x, y, c, outer - cut);
            }
        }
    }

    fn blend_mask(&mut self, x: i32, y: i32, w: usize, h: usize, mask: &[u8], c: Color) {
        let area = Rect::new(x, y, w as i32, h as i32).intersect(self.clip);
        for py in area.y..area.bottom() {
            let mrow = (py - y) as usize * w;
            let row = (py * self.w) as usize;
            for px in area.x..area.right() {
                let m = mask[mrow + (px - x) as usize];
                if m != 0 {
                    let i = row + px as usize;
                    self.px[i] = blend(self.px[i], c, m as u32);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: Color = Color::rgb(0xFFFFFF);

    #[test]
    fn fills_respect_the_clip() {
        let mut c = SoftCanvas::new(20, 10);
        c.set_clip(Rect::new(5, 0, 5, 5));
        c.fill_rect(Rect::new(0, 0, 20, 10), W);
        assert_eq!(c.px.iter().filter(|&&p| p != 0).count(), 25);
        assert_eq!(c.pixel(5, 0), W);
        assert_eq!(c.pixel(10, 0), Color::rgb(0));
    }

    #[test]
    fn round_corners_are_soft_and_the_middle_is_solid() {
        let mut c = SoftCanvas::new(40, 40);
        c.fill_round_rect(Rect::new(0, 0, 40, 40), 10, W);
        assert_eq!(c.pixel(0, 0), Color::rgb(0)); // outside the corner arc
        assert_eq!(c.pixel(20, 20), W);
        assert_eq!(c.pixel(20, 0), W); // top edge between the corners
        let edge = c.pixel(1, 4).r; // on the arc: partly covered
        assert!(edge > 0 && edge < 255, "{edge}");
    }

    #[test]
    fn strokes_leave_the_inside_alone() {
        let mut c = SoftCanvas::new(40, 40);
        c.stroke_round_rect(Rect::new(0, 0, 40, 40), 8, 2, W);
        assert_eq!(c.pixel(20, 20), Color::rgb(0));
        assert_eq!(c.pixel(20, 0), W);
        assert_eq!(c.pixel(0, 20), W);
        assert_eq!(c.pixel(39, 20), W);
        assert_eq!(c.pixel(20, 2), Color::rgb(0));
    }

    #[test]
    fn masks_blend() {
        let mut c = SoftCanvas::new(4, 4);
        c.blend_mask(-1, 1, 3, 1, &[255, 255, 0], W);
        assert_eq!(c.pixel(0, 1), W);
        assert_eq!(c.pixel(1, 1), Color::rgb(0));
    }
}
