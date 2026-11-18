//! What the XY pad plays: a tap on its chip opens this list in place of
//! Play's view (as the REC chip's target list does), and a tap on a choice
//! picks it (Jay, 18 Nov 2026 07:34Z: "I think we should just pick from a
//! list using the chip"). The choices are the Perform FX's DJ filter and
//! echo, as the pad always played, then the XY FX's nine effects, each
//! saying what across and up do. Each is a tile a fingertip covers.

use std::cell::RefCell;

use mpfree_ui::{Align, EventKind, FontId, PaintCx, Rect, Target, Touch, Widget};

/// The list's widget name and its page.
pub const XY_LIST: &str = "xy-list";
pub const XY_PAGE: &str = "Play · xy";

/// The list: a tap on choice `i` gives `Tab(i)` ([`crate::play_page::XY_CHOICES`]
/// order); on Close `Select(0)`.
#[derive(Default)]
pub struct XyList {
    /// The choices' names and what across and up do in each.
    pub choices: Vec<(String, String)>,
    /// The choice playing now.
    pub picked: usize,
    hits: RefCell<Vec<Rect>>,
}

const COLS: usize = 2;
const GAP: i32 = 10;
const HEAD_H: i32 = 48;
const CLOSE_W: i32 = 112;
const CLOSE_H: i32 = 44;

impl XyList {
    /// The choices with what each plays, the one now picked.
    pub fn new(picked: usize) -> XyList {
        let choices = crate::play_page::XY_CHOICES
            .iter()
            .enumerate()
            .map(|(i, n)| (n.to_string(), crate::play_page::xy_choice_detail(i)))
            .collect();
        XyList {
            choices,
            picked,
            hits: RefCell::new(Vec::new()),
        }
    }

    fn close(r: Rect) -> Rect {
        let inner = r.inset_xy(12, 12);
        Rect::new(inner.right() - CLOSE_W, inner.y, CLOSE_W, CLOSE_H)
    }

    /// Choice `i`'s tile: two across, in rows filling the room under the
    /// head.
    fn tile(&self, r: Rect, i: usize) -> Rect {
        let inner = r.inset_xy(12, 12);
        let body = Rect::new(inner.x, inner.y + HEAD_H + 8, inner.w, inner.h - HEAD_H - 8);
        let rows = self.choices.len().div_ceil(COLS).max(1) as i32;
        let w = (body.w - GAP * (COLS as i32 - 1)) / COLS as i32;
        let h = (body.h - GAP * (rows - 1)) / rows;
        let (col, row) = ((i % COLS) as i32, (i / COLS) as i32);
        Rect::new(body.x + col * (w + GAP), body.y + row * (h + GAP), w, h)
    }
}

impl Widget for XyList {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.canvas.fill_rect(r, t.bg);
        cx.panel(r);
        let inner = r.inset_xy(12, 12);
        let close = XyList::close(r);
        let title = "The XY pad plays";
        let tw = cx.measure(FontId::Bold, 18.0, title);
        cx.text(
            FontId::Bold,
            18.0,
            Rect::new(inner.x + 4, inner.y, tw + 4, CLOSE_H),
            Align::Left,
            t.text,
            title,
        );
        cx.text(
            FontId::Sans,
            13.0,
            Rect::new(
                inner.x + tw + 24,
                inner.y,
                (close.x - inner.x - tw - 30).max(0),
                CLOSE_H,
            ),
            Align::Left,
            t.text_dim,
            "Tap one: the pad plays it on the master, under one finger.",
        );
        cx.canvas.fill_round_rect(close, 10, t.raised);
        cx.text(FontId::Bold, 15.0, close, Align::Center, t.text, "Close");
        let mut hits = self.hits.borrow_mut();
        hits.clear();
        for (i, (name, detail)) in self.choices.iter().enumerate() {
            let tr = self.tile(r, i);
            hits.push(tr);
            let picked = i == self.picked;
            let (fill, ink, dim) = if picked {
                (t.accent, t.bg, t.bg)
            } else {
                (t.raised, t.text, t.text_dim)
            };
            cx.canvas.fill_round_rect(tr, 10, fill);
            let text = tr.inset_xy(16, 8);
            let (top, bottom) = text.take_top(text.h / 2);
            cx.text(FontId::Bold, 16.0, top, Align::Left, ink, name);
            cx.text(FontId::Sans, 13.0, bottom, Align::Left, dim, detail);
        }
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        let mut out: Vec<Target> = self
            .choices
            .iter()
            .enumerate()
            .map(|(i, (name, _))| {
                Target::tap("choice", i, name.clone(), self.tile(r, i))
                    .disabled_if(i == self.picked)
            })
            .collect();
        out.push(Target::tap("close", 0, "Close", XyList::close(r)));
        out
    }

    fn touch(&mut self, _finger: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if phase != Touch::Down {
            return None;
        }
        if XyList::close(r).contains(x, y) {
            return Some(EventKind::Select(0));
        }
        let i = (0..self.choices.len()).find(|&i| self.tile(r, i).contains(x, y))?;
        Some(EventKind::Tab(i))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_ui::{Fonts, SoftCanvas, Theme, Ui};

    /// Ten choices, two across, each a fingertip or more; a tap picks one,
    /// Close closes; the names are the XY FX's own after Filter + echo.
    #[test]
    fn the_xy_list_offers_ten_choices_a_fingertip_each() {
        assert_eq!(
            &crate::play_page::XY_CHOICES[1..],
            mpfree_fx::xyfx::EFFECTS,
            "the XY FX's effects, in order"
        );
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        ui.add(XY_LIST, XyList::new(0));
        ui.add_full_page("p", "col(xy-list:*)").unwrap();
        let mut c = SoftCanvas::new(1280, 800);
        ui.paint(&mut c);
        let r = ui.rect_of(XY_LIST).unwrap();
        let l = ui.get::<XyList>(XY_LIST).unwrap();
        assert_eq!(l.choices.len(), 10);
        assert_eq!(l.choices[4].0, "Wash");
        assert!(l.choices[4].1.starts_with("across: "), "{}", l.choices[4].1);
        for i in 0..10 {
            let t = l.tile(r, i);
            assert!(t.h >= 60 && t.w >= 200, "choice {i}: {t:?}");
        }
        let wash = l.tile(r, 4);
        let ev = |ui: &mut Ui, x, y| ui.touch(1, Touch::Down, x, y).map(|e| e.kind);
        assert_eq!(
            ev(&mut ui, wash.x + 10, wash.y + 10),
            Some(EventKind::Tab(4))
        );
        ui.touch(1, Touch::Up, wash.x + 10, wash.y + 10);
        let close = XyList::close(r);
        assert_eq!(
            ev(&mut ui, close.x + 10, close.y + 10),
            Some(EventKind::Select(0))
        );
    }
}
