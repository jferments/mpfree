//! The theme's effects, applied underneath the widgets.
//!
//! [`Styled`] wraps the real [`Canvas`] while widgets paint. Widgets keep
//! drawing what they mean (a rounded panel, an outline, text, a background
//! fill) and the wrapper draws it in the theme's style:
//!
//! - a fill in exactly the theme's `bg` becomes the theme's backdrop
//!   (gradient, grid, dots or scanlines), laid out in screen coordinates so
//!   a partial redraw lines up with the rest of the screen;
//! - a rounded box takes the theme's corner shape (round, cut or square;
//!   pills and circles stay round);
//! - outlines get a glow halo when the theme's `glow` is on;
//! - text gets a soft glow when `text_glow` is on.
//!
//! So every crate that draws through [`crate::PaintCx`] is themed with no
//! changes of its own. All of it is cheap enough for the RK3288: the
//! backdrop is a few spans per row, a halo is three extra outlines, and the
//! text glow blurs only each glyph's own small mask.

use crate::canvas::{Canvas, Image};
use crate::color::Color;
use crate::geom::Rect;
use crate::shape::Shape;
use crate::theme::{Backdrop, Corners, Theme};

pub struct Styled<'a> {
    pub inner: &'a mut dyn Canvas,
    pub theme: &'a Theme,
}

impl<'a> Styled<'a> {
    pub fn new(inner: &'a mut dyn Canvas, theme: &'a Theme) -> Styled<'a> {
        Styled { inner, theme }
    }
}

/// The theme's backdrop over `r` (screen `height` pixels tall), drawn with
/// fills only, so it works on any canvas.
pub fn paint_backdrop(c: &mut dyn Canvas, t: &Theme, r: Rect, height: i32) {
    let r = r.intersect(c.clip());
    if r.is_empty() {
        return;
    }
    if t.backdrop == Backdrop::Solid {
        return c.fill_rect(r, t.bg);
    }
    let step = t.backdrop_step.max(2);
    let h = height.max(1) as f32;
    // Row by row for the gradient, in bands of rows with the same colour.
    let mut y = r.y;
    while y < r.bottom() {
        let col = t.bg.mix(t.backdrop_to, y as f32 / h);
        let mut y2 = y + 1;
        while y2 < r.bottom() && t.bg.mix(t.backdrop_to, y2 as f32 / h) == col {
            y2 += 1;
        }
        c.fill_rect(Rect::new(r.x, y, r.w, y2 - y), col);
        y = y2;
    }
    let line = t.backdrop_line;
    match t.backdrop {
        Backdrop::Solid | Backdrop::Gradient => {}
        Backdrop::Scanlines => {
            let first = r.y + (step - r.y.rem_euclid(step)) % step;
            for y in (first..r.bottom()).step_by(step as usize) {
                c.fill_rect(Rect::new(r.x, y, r.w, 1), line.with_alpha(line.a.min(90)));
            }
        }
        Backdrop::Grid => {
            let first = r.y + (step - r.y.rem_euclid(step)) % step;
            for y in (first..r.bottom()).step_by(step as usize) {
                c.fill_rect(Rect::new(r.x, y, r.w, 1), line);
            }
            let first = r.x + (step - r.x.rem_euclid(step)) % step;
            for x in (first..r.right()).step_by(step as usize) {
                c.fill_rect(Rect::new(x, r.y, 1, r.h), line);
            }
        }
        Backdrop::Dots => {
            let fy = r.y + (step - r.y.rem_euclid(step)) % step;
            let fx = r.x + (step - r.x.rem_euclid(step)) % step;
            for y in (fy..r.bottom()).step_by(step as usize) {
                for x in (fx..r.right()).step_by(step as usize) {
                    c.fill_rect(Rect::new(x, y, 1, 1), line);
                }
            }
        }
    }
}

/// The row span (x0, x1) of a box with 45-degree corners `k` deep, at row y.
fn chamfer_span(r: Rect, k: i32, y: i32) -> Option<(i32, i32)> {
    if y < r.y || y >= r.bottom() || r.is_empty() {
        return None;
    }
    let from_edge = (y - r.y).min(r.bottom() - 1 - y);
    let inset = (k - from_edge).max(0);
    (r.w > 2 * inset).then_some((r.x + inset, r.right() - inset))
}

fn fill_chamfer(c: &mut dyn Canvas, r: Rect, k: i32, col: Color) {
    let rows = r.intersect(c.clip());
    let mut y = rows.y;
    while y < rows.bottom() {
        // The straight middle in one fill.
        if y >= r.y + k && y < r.bottom() - k {
            let end = (r.bottom() - k).min(rows.bottom());
            c.fill_rect(Rect::new(r.x, y, r.w, end - y), col);
            y = end;
            continue;
        }
        if let Some((x0, x1)) = chamfer_span(r, k, y) {
            c.fill_rect(Rect::new(x0, y, x1 - x0, 1), col);
        }
        y += 1;
    }
}

fn stroke_chamfer(c: &mut dyn Canvas, r: Rect, k: i32, width: i32, col: Color) {
    let inner = r.inset(width);
    // The inner edge of a 45-degree cut sits about 0.41 of the width further in.
    let ik = (k - width * 41 / 100).max(0);
    let rows = r.intersect(c.clip());
    for y in rows.y..rows.bottom() {
        let Some((x0, x1)) = chamfer_span(r, k, y) else {
            continue;
        };
        match chamfer_span(inner, ik, y) {
            Some((i0, i1)) if !inner.is_empty() => {
                c.fill_rect(Rect::new(x0, y, (i0 - x0).max(1), 1), col);
                c.fill_rect(Rect::new(i1.min(x1 - 1), y, (x1 - i1).max(1), 1), col);
            }
            _ => c.fill_rect(Rect::new(x0, y, x1 - x0, 1), col),
        }
    }
}

impl Styled<'_> {
    fn shaped(&self, r: Rect, radius: i32) -> Corners {
        // Pills and circles keep their round ends whatever the theme.
        if radius * 2 >= r.w.min(r.h) - 1 {
            Corners::Round
        } else {
            self.theme.corners
        }
    }

    fn outline(&mut self, r: Rect, radius: i32, width: i32, c: Color) {
        match self.shaped(r, radius) {
            Corners::Round => self.inner.stroke_round_rect(r, radius, width, c),
            Corners::Square => self.inner.stroke_round_rect(r, 0, width, c),
            Corners::Chamfer => stroke_chamfer(
                self.inner,
                r,
                radius.clamp(0, (r.w.min(r.h) / 2).max(0)),
                width,
                c,
            ),
        }
    }
}

impl Canvas for Styled<'_> {
    fn size(&self) -> (i32, i32) {
        self.inner.size()
    }

    fn clip(&self) -> Rect {
        self.inner.clip()
    }

    fn set_clip(&mut self, r: Rect) {
        self.inner.set_clip(r)
    }

    fn fill_rect(&mut self, r: Rect, c: Color) {
        if c == self.theme.bg && self.theme.backdrop != Backdrop::Solid {
            let h = self.inner.size().1;
            paint_backdrop(self.inner, self.theme, r, h);
        } else {
            self.inner.fill_rect(r, c)
        }
    }

    fn fill_round_rect(&mut self, r: Rect, radius: i32, c: Color) {
        match self.shaped(r, radius) {
            Corners::Round => self.inner.fill_round_rect(r, radius, c),
            Corners::Square => self.inner.fill_rect(r, c),
            Corners::Chamfer => {
                fill_chamfer(self.inner, r, radius.clamp(0, (r.w.min(r.h) / 2).max(0)), c)
            }
        }
    }

    fn stroke_round_rect(&mut self, r: Rect, radius: i32, width: i32, c: Color) {
        let g = self.theme.glow as u32;
        if g > 0 && c.a > 0 {
            // A halo: three outlines outside the line, fading out.
            for (i, k) in [(1, 90u32), (2, 50), (3, 24)] {
                let a = (c.a as u32 * g * k / (255 * 100)) as u8;
                if a > 0 {
                    let rr = r.inset(-i);
                    self.outline(rr, radius + i, 1, c.with_alpha(a));
                }
            }
            if width == 1 {
                // A thin line under glow reads better a little brighter inside.
                self.outline(
                    r.inset(1),
                    (radius - 1).max(0),
                    1,
                    c.with_alpha((c.a as u32 * g / 600) as u8),
                );
            }
        }
        self.outline(r, radius, width, c)
    }

    fn blend_mask(&mut self, x: i32, y: i32, w: usize, h: usize, mask: &[u8], c: Color) {
        let g = self.theme.text_glow as u32;
        if g > 0 && h >= 8 {
            const R: usize = 2;
            let (gw, gh) = (w + 2 * R, h + 2 * R);
            let mut a = vec![0u16; gw * gh];
            for yy in 0..h {
                for xx in 0..w {
                    a[(yy + R) * gw + xx + R] = mask[yy * w + xx] as u16;
                }
            }
            // A box blur, across then down (radius R).
            let mut b = vec![0u16; gw * gh];
            for yy in 0..gh {
                for xx in 0..gw {
                    let lo = xx.saturating_sub(R);
                    let hi = (xx + R).min(gw - 1);
                    let s: u32 = a[yy * gw + lo..=yy * gw + hi]
                        .iter()
                        .map(|&v| v as u32)
                        .sum();
                    b[yy * gw + xx] = (s / (2 * R as u32 + 1)) as u16;
                }
            }
            let mut out = vec![0u8; gw * gh];
            for yy in 0..gh {
                for xx in 0..gw {
                    let lo = yy.saturating_sub(R);
                    let hi = (yy + R).min(gh - 1);
                    let s: u32 = (lo..=hi).map(|k| b[k * gw + xx] as u32).sum();
                    let v = s / (2 * R as u32 + 1) * g * 2 / 255;
                    out[yy * gw + xx] = v.min(255) as u8;
                }
            }
            self.inner
                .blend_mask(x - R as i32, y - R as i32, gw, gh, &out, c);
        }
        self.inner.blend_mask(x, y, w, h, mask, c)
    }

    fn draw_image(&mut self, x: i32, y: i32, img: &Image) {
        self.inner.draw_image(x, y, img)
    }

    fn blend_shape(&mut self, shape: &Shape, c: Color) {
        let r = shape.r;
        if r.is_empty() {
            return;
        }
        if self.theme.text_glow > 0 {
            // The glow is worked out from the pixels, so it goes the way
            // masks go (as these shapes did when they were masks).
            let cov = shape.coverage();
            self.blend_mask(r.x, r.y, r.w as usize, r.h as usize, &cov, c);
        } else {
            self.inner.blend_shape(shape, c)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::SoftCanvas;

    #[test]
    fn default_theme_draws_exactly_as_before() {
        let t = Theme::dark();
        let mut a = SoftCanvas::new(120, 80);
        let mut b = SoftCanvas::new(120, 80);
        let draw = |c: &mut dyn Canvas| {
            c.fill_rect(Rect::new(0, 0, 120, 80), t.bg);
            c.fill_round_rect(Rect::new(10, 10, 60, 40), 12, t.surface);
            c.stroke_round_rect(Rect::new(20, 20, 60, 40), 8, 2, t.accent);
            c.blend_mask(5, 5, 3, 1, &[255, 128, 0], t.text);
        };
        draw(&mut a);
        draw(&mut Styled::new(&mut b, &t));
        assert!(a.px == b.px);
    }

    #[test]
    fn chamfered_corners_are_cut() {
        let mut t = Theme::dark();
        t.corners = Corners::Chamfer;
        let mut c = SoftCanvas::new(60, 60);
        let red = Color::rgb(0xFF0000);
        Styled::new(&mut c, &t).fill_round_rect(Rect::new(10, 10, 40, 30), 8, red);
        assert_eq!(c.pixel(10, 10), Color::rgb(0)); // corner cut
        assert_eq!(c.pixel(18, 10), red); // top edge after the cut
        assert_eq!(c.pixel(10, 25), red); // left edge
        assert_eq!(c.pixel(49, 39), Color::rgb(0)); // far corner cut
                                                    // A pill stays round: its end is soft, not cut flat.
        let mut p = SoftCanvas::new(60, 20);
        Styled::new(&mut p, &t).fill_round_rect(Rect::new(0, 0, 60, 20), 10, red);
        let mut q = SoftCanvas::new(60, 20);
        q.fill_round_rect(Rect::new(0, 0, 60, 20), 10, red);
        assert!(p.px == q.px);
    }

    #[test]
    fn background_fills_become_the_backdrop_and_line_up() {
        let t = Theme::synthwave();
        let mut whole = SoftCanvas::new(200, 100);
        Styled::new(&mut whole, &t).fill_rect(Rect::new(0, 0, 200, 100), t.bg);
        let mut part = SoftCanvas::new(200, 100);
        Styled::new(&mut part, &t).fill_rect(Rect::new(37, 13, 90, 61), t.bg);
        for y in 13..74 {
            for x in 37..127 {
                assert_eq!(whole.pixel(x, y), part.pixel(x, y), "{x},{y}");
            }
        }
        // Grid lines are there.
        assert_ne!(whole.pixel(40, 10), whole.pixel(41, 10));
    }

    #[test]
    fn glow_draws_outside_the_line() {
        let t = Theme::synthwave();
        let mut c = SoftCanvas::new(60, 60);
        c.fill_rect(Rect::new(0, 0, 60, 60), Color::rgb(0));
        Styled::new(&mut c, &t).stroke_round_rect(Rect::new(10, 10, 40, 40), 6, 1, t.accent);
        assert_ne!(c.pixel(30, 8), Color::rgb(0));
        assert_eq!(c.pixel(30, 4), Color::rgb(0));
    }

    #[test]
    fn shapes_glow_as_masks_and_otherwise_stay_shapes() {
        use crate::record::{Op, Recorder};
        let mut shape = Shape::new(Rect::new(2, 2, 30, 20));
        shape.polyline(&[(3.0, 18.0), (16.0, 4.0), (30.0, 18.0)], 2.0);
        for theme in [Theme::dark(), Theme::console()] {
            // Through a shape and through its coverage as a mask: the same
            // pixels, glow included.
            let mut a = SoftCanvas::new(40, 30);
            shape.draw(&mut Styled::new(&mut a, &theme), theme.text);
            let mut b = SoftCanvas::new(40, 30);
            let cov = shape.coverage();
            Styled::new(&mut b, &theme).blend_mask(2, 2, 30, 20, &cov, theme.text);
            assert_eq!(a.px, b.px, "glow {}", theme.text_glow);
            // A recorded list keeps the parts unless the glow needs pixels.
            let mut rec = Recorder::new(40, 30);
            rec.begin_frame();
            shape.draw(&mut Styled::new(&mut rec, &theme), theme.text);
            let kept = rec.ops().iter().any(|op| matches!(op, Op::Shape { .. }));
            assert_eq!(kept, theme.text_glow == 0, "glow {}", theme.text_glow);
        }
    }
}
