//! The contrast check: how readable a drawn screen's text is.
//!
//! [`text_runs`] plays a frame's display list ([`crate::record`]) back and,
//! for every glyph as it is drawn, measures its ink against the pixels it
//! lands on (the average under its solid part, before the glyph). Glyphs
//! of one ink on one line, close together, make a run: a label or a value.
//! A run's contrast is its ink against everything under all its glyphs.
//!
//! The app's test (`mpfree-app/tests/theme_contrast.rs`) runs it over every
//! sample screen in every built-in theme. It knows nothing about widgets,
//! so it catches text made unreadable by anything: a fixed colour, a theme
//! colour used on the wrong fill, a see-through panel.

use crate::color::Color;
use crate::geom::Rect;
use crate::record::{replay, Frame, Op, Store};
use crate::roles::over;
use crate::theme::contrast;
use crate::SoftCanvas;

/// Masks taller than this are strokes or pictures, not text.
pub const TEXT_MAX_H: usize = 40;

/// Under this a run is unreadable (WCAG's floor for large text and for
/// lines and controls).
pub const FLOOR: f32 = 3.0;

/// One run of text as drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Run {
    /// Where its first glyph was drawn, and the tallest glyph's height.
    pub x: i32,
    pub y: i32,
    pub h: i32,
    pub glyphs: usize,
    pub ink: Color,
    /// The average colour under its glyphs.
    pub under: Color,
    /// Contrast of the ink (blended as drawn) on `under`.
    pub ratio: f32,
}

/// The runs of text in a frame that holds everything it draws (the first
/// frame of a page's display list file), in drawing order. A glowing
/// theme's text glow (a blurred copy of the glyph in its ink, drawn just
/// before it) is not text: its glyph is measured against what was under
/// the glow.
pub fn text_runs(frame: &Frame) -> Vec<Run> {
    let mut store = Store::new(usize::MAX / 2);
    store.apply(frame);
    let mut c = SoftCanvas::new(frame.w, frame.h);
    let screen = Rect::new(0, 0, frame.w, frame.h);
    let mut clip = screen;
    let mut runs: Vec<Run> = Vec::new();
    // The open run: its glyph-pixel sums, and where the next glyph may be.
    let mut sums = [0f64; 4];
    let mut next_x = i32::MIN;
    // A glyph already measured (before its glow was drawn).
    let mut measured = false;
    for (i, op) in frame.ops.iter().enumerate() {
        match *op {
            Op::Clip(r) => clip = r.intersect(screen),
            Op::Mask { x, y, id, c: ink } => {
                let glyph = |id: u32| store.mask(id).filter(|m| m.h <= TEXT_MAX_H);
                // A glow is followed by its glyph, 2 px in and 4 px smaller.
                let glow_of = match (glyph(id), frame.ops.get(i + 1)) {
                    (
                        Some(g),
                        Some(&Op::Mask {
                            x: gx,
                            y: gy,
                            id: gid,
                            c: gc,
                        }),
                    ) => glyph(gid)
                        .filter(|m| {
                            gc == ink
                                && gx == x + 2
                                && gy == y + 2
                                && m.w + 4 == g.w
                                && m.h + 4 == g.h
                        })
                        .map(|m| (gx, gy, m)),
                    _ => None,
                };
                let target = match glow_of {
                    Some(t) => Some(t),
                    None if measured => {
                        measured = false;
                        None
                    }
                    None => glyph(id).map(|m| (x, y, m)),
                };
                if let Some((x, y, m)) = target {
                    measured = glow_of.is_some();
                    let mut s = [0f64; 4];
                    for yy in 0..m.h {
                        for xx in 0..m.w {
                            if m.data[yy * m.w + xx] < 128 {
                                continue;
                            }
                            let (px, py) = (x + xx as i32, y + yy as i32);
                            if !clip.contains(px, py) {
                                continue;
                            }
                            let p = c.pixel(px, py);
                            s[0] += p.r as f64;
                            s[1] += p.g as f64;
                            s[2] += p.b as f64;
                            s[3] += 1.0;
                        }
                    }
                    if s[3] > 0.0 {
                        let joins = runs.last().is_some_and(|r| {
                            r.ink == ink && (r.y - y).abs() < 12 && x >= r.x && x <= next_x
                        });
                        if joins {
                            for i in 0..4 {
                                sums[i] += s[i];
                            }
                        } else {
                            sums = s;
                            runs.push(Run {
                                x,
                                y,
                                h: 0,
                                glyphs: 0,
                                ink,
                                under: ink,
                                ratio: 0.0,
                            });
                        }
                        let r = runs.last_mut().unwrap();
                        r.glyphs += 1;
                        r.h = r.h.max(m.h as i32);
                        r.under = Color {
                            r: (sums[0] / sums[3]) as u8,
                            g: (sums[1] / sums[3]) as u8,
                            b: (sums[2] / sums[3]) as u8,
                            a: 255,
                        };
                        r.ratio = contrast(over(ink, r.under), r.under);
                        next_x = x + m.w as i32 + 40;
                    }
                }
            }
            _ => {}
        }
        replay(std::slice::from_ref(op), &store, &mut c);
    }
    runs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::Recorder;
    use crate::text::{Align, FontId, Fonts};
    use crate::Canvas;
    use crate::Theme;

    fn frame(draw: impl Fn(&mut Recorder, &mut Fonts)) -> Frame {
        let mut fonts = Fonts::embedded();
        let mut rec = Recorder::new(400, 200);
        rec.begin_frame();
        draw(&mut rec, &mut fonts);
        let mut bytes = Vec::new();
        rec.encode_frame(&mut bytes);
        Frame::decode_all(&bytes).unwrap().remove(0)
    }

    fn label(rec: &mut Recorder, fonts: &mut Fonts, y: i32, ink: Color, s: &str) {
        fonts.draw_in(
            rec,
            FontId::Sans,
            17.0,
            Rect::new(10, y, 380, 30),
            Align::Left,
            ink,
            s,
        );
    }

    #[test]
    fn readable_and_unreadable_labels_are_told_apart() {
        let t = Theme::dark();
        let f = frame(|rec, fonts| {
            rec.fill_rect(Rect::new(0, 0, 400, 200), t.surface);
            label(rec, fonts, 10, t.text, "Readable label");
            label(rec, fonts, 60, t.raised, "Nearly invisible");
            rec.fill_rect(Rect::new(0, 120, 400, 40), Color::rgb(0xFFC145));
            label(rec, fonts, 125, t.bg, "Dark on amber");
        });
        let runs = text_runs(&f);
        assert_eq!(runs.len(), 3, "{runs:?}");
        assert!(runs[0].ratio > 10.0 && runs[0].glyphs > 10, "{:?}", runs[0]);
        assert!(runs[1].ratio < FLOOR, "{:?}", runs[1]);
        assert_eq!(runs[1].under, t.surface);
        assert!(runs[2].ratio > 7.0, "{:?}", runs[2]);
    }

    #[test]
    fn glowing_text_is_measured_against_what_is_under_its_glow() {
        let mut t = Theme::console();
        t.text_glow = 200;
        let f = frame(|rec, fonts| {
            let mut c = crate::Styled::new(rec, &t);
            c.fill_rect(Rect::new(0, 0, 400, 200), Color::rgb(0x020803));
            fonts.draw_in(
                &mut c,
                FontId::Sans,
                17.0,
                Rect::new(10, 10, 380, 30),
                Align::Left,
                t.text,
                "Glowing label",
            );
        });
        let runs = text_runs(&f);
        assert_eq!(runs.len(), 1, "{runs:?}");
        assert!(runs[0].ratio > 10.0, "{:?}", runs[0]);
        // The page under it (console's scanlines darken a row here and
        // there), not the glow.
        let u = runs[0].under;
        assert!(u.r <= 4 && u.g <= 10 && u.b <= 5, "{u:?}");
    }

    #[test]
    fn clipped_away_text_is_not_measured() {
        let f = frame(|rec, fonts| {
            rec.fill_rect(Rect::new(0, 0, 400, 200), Color::rgb(0x101010));
            rec.set_clip(Rect::new(0, 0, 400, 40));
            label(rec, fonts, 100, Color::rgb(0x121212), "Hidden");
        });
        assert!(text_runs(&f).is_empty());
    }
}
