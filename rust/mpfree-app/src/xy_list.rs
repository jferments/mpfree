//! What the XY pad plays: a tap on its chip opens this list in place of
//! Play's view (as the REC chip's target list does), and a tap on a choice
//! picks it (Jay, 18 Nov 2026 07:34Z: "I think we should just pick from a
//! list using the chip"; board 1 of the XY mockups, approved 23 Nov
//! 10:34Z). Two groups of tiles, three across, each a fingertip and more:
//!
//! - **READY-MADE · 10**, on the master: the Perform FX's DJ filter and
//!   echo, as the pad always played, then the XY FX's nine effects, each
//!   saying what across and up do.
//! - **YOURS · IN THIS SONG · N**: your layouts (`xy_song.rs`), each
//!   saying what its directions are called, how many settings each moves
//!   and where they play, then **+ New layout** (dashed: an empty layout
//!   of yours, opened in Setup) and **From the browser** (a layout saved
//!   in another song, picked in the browser).
//!
//! The one playing is lit in the accent. Two faint lines under them; more
//! than fit scroll with a drag.

use std::cell::RefCell;

use mpfree_ui::{Align, EventKind, FontId, PaintCx, Rect, Target, Touch, Widget};

/// The list's widget name and its page.
pub const XY_LIST: &str = "xy-list";
pub const XY_PAGE: &str = "Play · xy";

/// The list: a tap on ready-made choice `i` gives `Tab(i)`
/// ([`crate::play_page::XY_CHOICES`] order), on layout `j` of yours
/// `Cell { row: 1, col: j }`, on + New layout `Cell { row: 2, col: 0 }`,
/// on From the browser `Cell { row: 2, col: 1 }`, on Close `Select(0)`.
#[derive(Default)]
pub struct XyList {
    /// The ready-made choices' names and what across and up do in each.
    pub choices: Vec<(String, String)>,
    /// The ready-made choice picked (lit while none of yours plays).
    pub picked: usize,
    /// Your layouts: each name and what it plays.
    pub yours: Vec<(String, String)>,
    /// The layout of yours playing.
    pub playing: Option<usize>,
    scroll: i32,
    drag: Option<(u64, i32, i32)>,
    hits: RefCell<Vec<Rect>>,
}

const COLS: usize = 3;
const GAP: i32 = 10;
const HEAD_H: i32 = 48;
const CLOSE_W: i32 = 112;
const CLOSE_H: i32 = 44;
const TILE_H: i32 = 52;
const GROUP_H: i32 = 26;
const FOOT_H: i32 = 48;
/// A drag this far scrolls instead of picking.
const SLOP: i32 = 12;

/// A tile: a ready-made choice, a layout of yours, + New layout or From
/// the browser.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Tile {
    Ready(usize),
    Yours(usize),
    New,
    Browser,
}

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
            ..XyList::default()
        }
    }

    fn close(r: Rect) -> Rect {
        let inner = r.inset_xy(12, 12);
        Rect::new(inner.right() - CLOSE_W, inner.y, CLOSE_W, CLOSE_H)
    }

    /// Where the tiles go: under the head, above the foot's lines.
    fn body(r: Rect) -> Rect {
        let inner = r.inset_xy(12, 12);
        let (_, rest) = inner.take_top(HEAD_H + 8);
        rest.take_bottom(FOOT_H).1
    }

    fn tiles(&self) -> Vec<Tile> {
        (0..self.choices.len())
            .map(Tile::Ready)
            .chain((0..self.yours.len()).map(Tile::Yours))
            .chain([Tile::New, Tile::Browser])
            .collect()
    }

    /// The top of a group's label, and its tiles' first row (unscrolled,
    /// from the body's top).
    fn group_y(&self, yours: bool) -> i32 {
        if !yours {
            return 0;
        }
        let rows = self.choices.len().div_ceil(COLS) as i32;
        GROUP_H + rows * (TILE_H + GAP) + 6
    }

    /// Tile `t`'s place on the screen (it may be outside the body).
    fn tile_rect(&self, r: Rect, t: Tile) -> Rect {
        let body = XyList::body(r);
        let w = (body.w - GAP * (COLS as i32 - 1)) / COLS as i32;
        let (top, k) = match t {
            Tile::Ready(i) => (self.group_y(false), i),
            Tile::Yours(j) => (self.group_y(true), j),
            Tile::New => (self.group_y(true), self.yours.len()),
            Tile::Browser => (self.group_y(true), self.yours.len() + 1),
        };
        let (col, row) = ((k % COLS) as i32, (k / COLS) as i32);
        Rect::new(
            body.x + col * (w + GAP),
            body.y + top + GROUP_H + row * (TILE_H + GAP) - self.scroll,
            w,
            TILE_H,
        )
    }

    /// Ready-made choice `i`'s tile (the tests').
    #[cfg(test)]
    fn tile(&self, r: Rect, i: usize) -> Rect {
        self.tile_rect(r, Tile::Ready(i))
    }

    fn content_h(&self) -> i32 {
        let rows = (self.yours.len() + 2).div_ceil(COLS) as i32;
        self.group_y(true) + GROUP_H + rows * (TILE_H + GAP)
    }

    fn shown(r: Rect, t: Rect) -> bool {
        let body = XyList::body(r);
        t.y >= body.y && t.bottom() <= body.bottom()
    }

    fn event(t: Tile) -> EventKind {
        match t {
            Tile::Ready(i) => EventKind::Tab(i),
            Tile::Yours(j) => EventKind::Cell { row: 1, col: j },
            Tile::New => EventKind::Cell { row: 2, col: 0 },
            Tile::Browser => EventKind::Cell { row: 2, col: 1 },
        }
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
            "Tap one: the pad plays it.   Yours change in Setup on the XY page.",
        );
        cx.canvas.fill_round_rect(close, t.round(10), t.raised);
        cx.text(FontId::Bold, 15.0, close, Align::Center, t.text, "Close");
        let body = XyList::body(r);
        let clip = cx.canvas.clip();
        cx.canvas.set_clip(clip.intersect(body));
        for (yours, label, note) in [
            (
                false,
                format!("READY-MADE · {}", self.choices.len()),
                "on the master",
            ),
            (
                true,
                format!("YOURS · IN THIS SONG · {}", self.yours.len()),
                "",
            ),
        ] {
            let y = body.y + self.group_y(yours) - self.scroll;
            let lr = Rect::new(body.x + 4, y, body.w, GROUP_H);
            let w = cx.text(FontId::Bold, 12.0, lr, Align::Left, t.text_dim, &label);
            let nr = Rect::new(lr.x + w + 10, y, body.w, GROUP_H);
            cx.text(FontId::Sans, 12.0, nr, Align::Left, t.text_dim, note);
        }
        let mut hits = self.hits.borrow_mut();
        hits.clear();
        for tile in self.tiles() {
            let tr = self.tile_rect(r, tile);
            hits.push(tr);
            if tr.bottom() < body.y || tr.y > body.bottom() {
                continue;
            }
            let (name, detail, picked) = match tile {
                Tile::Ready(i) => (
                    self.choices[i].0.as_str(),
                    self.choices[i].1.as_str(),
                    self.playing.is_none() && i == self.picked,
                ),
                Tile::Yours(j) => (
                    self.yours[j].0.as_str(),
                    self.yours[j].1.as_str(),
                    self.playing == Some(j),
                ),
                Tile::New => ("+ New layout", "empty: add settings in Setup", false),
                Tile::Browser => (
                    "From the browser",
                    "a layout you saved in another song",
                    false,
                ),
            };
            let (ink, dim) = if picked {
                cx.canvas.fill_round_rect(tr, t.round(10), t.picked());
                (t.on_picked(), t.on_picked())
            } else if tile == Tile::New {
                cx.canvas.stroke_round_rect(tr, t.round(10), 2, t.text_dim);
                (t.text, t.text_dim)
            } else {
                cx.canvas.fill_round_rect(tr, t.round(10), t.raised);
                (t.text, t.text_dim)
            };
            let text = tr.inset_xy(14, 6);
            let (top, bottom) = text.take_top(text.h / 2 + 2);
            cx.text(FontId::Bold, 15.0, top, Align::Left, ink, name);
            cx.text(FontId::Sans, 12.0, bottom, Align::Left, dim, detail);
        }
        cx.canvas.set_clip(clip);
        let foot = Rect::new(inner.x + 4, body.bottom() + 4, inner.w, FOOT_H - 8);
        let (l1, l2) = foot.take_top(foot.h / 2);
        cx.text(
            FontId::Sans,
            12.0,
            l1,
            Align::Left,
            t.text_dim,
            "Change a ready-made one in Setup and it becomes one of yours; the ready-made one stays as it was.",
        );
        cx.text(
            FontId::Sans,
            12.0,
            l2,
            Align::Left,
            t.text_dim,
            "Under one finger: a second finger on the pad does nothing until the first lifts.",
        );
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        let mut out: Vec<Target> = Vec::new();
        for tile in self.tiles() {
            let tr = self.tile_rect(r, tile);
            if !XyList::shown(r, tr) {
                continue;
            }
            out.push(match tile {
                Tile::Ready(i) => Target::tap("choice", i, self.choices[i].0.clone(), tr)
                    .disabled_if(self.playing.is_none() && i == self.picked),
                Tile::Yours(j) => Target::tap("yours", j, self.yours[j].0.clone(), tr)
                    .disabled_if(self.playing == Some(j)),
                Tile::New => Target::tap("new", 0, "+ New layout", tr),
                Tile::Browser => Target::tap("browser", 0, "From the browser", tr),
            });
        }
        out.push(Target::tap("close", 0, "Close", XyList::close(r)));
        out
    }

    fn touch(&mut self, finger: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        match phase {
            Touch::Down => {}
            Touch::Move => {
                let (f, y0, from) = self.drag?;
                if f == finger && (y - y0).abs() > SLOP {
                    let max = (self.content_h() - XyList::body(r).h).max(0);
                    self.scroll = (from - (y - y0)).clamp(0, max);
                    return Some(EventKind::Select(1));
                }
                return None;
            }
            Touch::Up | Touch::Cancel => {
                self.drag = None;
                return None;
            }
        }
        if XyList::close(r).contains(x, y) {
            return Some(EventKind::Select(0));
        }
        self.drag = Some((finger, y, self.scroll));
        let tile = self.tiles().into_iter().find(|&t| {
            let tr = self.tile_rect(r, t);
            XyList::shown(r, tr) && tr.contains(x, y)
        })?;
        Some(XyList::event(tile))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_ui::{Fonts, SoftCanvas, Theme, Ui};

    /// Ten ready-made choices three across, then yours and + New layout,
    /// each a fingertip or more; a tap picks one, Close closes; the names
    /// are the XY FX's own after Filter + echo.
    #[test]
    fn the_xy_list_offers_ten_choices_a_fingertip_each() {
        assert_eq!(
            &crate::play_page::XY_CHOICES[1..],
            mpfree_fx::xyfx::EFFECTS,
            "the XY FX's effects, in order"
        );
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        let mut list = XyList::new(0);
        list.yours = vec![
            (
                "Squelch".into(),
                "across: Bite, 2 settings · up: Wet, 2".into(),
            ),
            ("Wobble".into(), "follows the selected track".into()),
        ];
        ui.add(XY_LIST, list);
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
            assert!(t.h >= 52 && t.w >= 300, "choice {i}: {t:?}");
        }
        assert_eq!(l.tile(r, 1).y, l.tile(r, 0).y, "three across");
        assert_eq!(l.tile(r, 3).x, l.tile(r, 0).x);
        let wash = l.tile(r, 4);
        let squelch = l.tile_rect(r, Tile::Yours(0));
        let new = l.tile_rect(r, Tile::New);
        let browser = l.tile_rect(r, Tile::Browser);
        assert_eq!(
            (browser.y, browser.x),
            (new.y + TILE_H + GAP, squelch.x),
            "after + New layout (on the next row with two of yours)"
        );
        let ev = |ui: &mut Ui, x, y| {
            let e = ui.touch(1, Touch::Down, x, y).map(|e| e.kind);
            ui.touch(1, Touch::Up, x, y);
            e
        };
        assert_eq!(
            ev(&mut ui, wash.x + 10, wash.y + 10),
            Some(EventKind::Tab(4))
        );
        assert_eq!(
            ev(&mut ui, squelch.x + 10, squelch.y + 10),
            Some(EventKind::Cell { row: 1, col: 0 })
        );
        assert_eq!(
            ev(&mut ui, new.x + 10, new.y + 10),
            Some(EventKind::Cell { row: 2, col: 0 })
        );
        assert_eq!(
            ev(&mut ui, browser.x + 10, browser.y + 10),
            Some(EventKind::Cell { row: 2, col: 1 })
        );
        let close = XyList::close(r);
        assert_eq!(
            ev(&mut ui, close.x + 10, close.y + 10),
            Some(EventKind::Select(0))
        );
    }
}
