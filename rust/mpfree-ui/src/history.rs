//! A page's undo history as a list (Jay after live test 3: hold UNDO to
//! see the page's history and jump back several steps at once).
//!
//! The rows are "Start" (before the first step kept), then each step,
//! oldest first, then the steps Redo would bring back, faint. The row
//! that is now is filled and tagged "now". A tap on a row goes back (or
//! forward) to just after that step: the [`HistoryPanel`] gives
//! [`EventKind::Select`] with the row, and the app works out how many
//! steps that is from [`HistoryPanel::now`]. A drag scrolls the list.
//!
//! The wheel outlines a row ([`HistoryPanel::wheel`]) and its push goes
//! there, so the list works without touch. Close gives
//! [`EventKind::Tab`] with [`TAB_CLOSE`]. Rows and Close are at least
//! 64 px tall.

use crate::geom::Rect;
use crate::text::{Align, FontId};
use crate::ui::{EventKind, PaintCx, Touch, Widget};

/// Close's `Tab` index.
pub const TAB_CLOSE: usize = 0;

/// The card's width at most.
const CARD_W: i32 = 900;
/// A row's height and the gap under it.
const ROW_H: i32 = 64;
const GAP: i32 = 8;
/// The title band and the Close band.
const HEAD_H: i32 = 64;
const FOOT_H: i32 = 64;
/// A finger moving this far is a drag, not a tap.
const DRAG: i32 = 12;

/// The list. See the module page.
#[derive(Default)]
pub struct HistoryPanel {
    /// "Undo history · Loop 1".
    pub title: String,
    pub rows: Vec<String>,
    /// The row that is now.
    pub now: usize,
    /// The row the wheel outlines.
    pub picked: usize,
    /// The first row shown, once a drag has scrolled; until then the list
    /// keeps the outlined row in view.
    top: Option<usize>,
    /// A finger on the rows: where it went down, the first row shown then,
    /// and whether it has moved enough to be a drag.
    press: Option<(u64, i32, usize, bool)>,
}

impl HistoryPanel {
    /// Show `rows` with `now`. When now moves (or the list is new), the
    /// outline goes to it and the list follows it again.
    pub fn set(&mut self, title: &str, rows: Vec<String>, now: usize) {
        let now = now.min(rows.len().saturating_sub(1));
        if now != self.now || self.rows.is_empty() || title != self.title {
            self.picked = now;
            self.top = None;
        }
        self.title = title.to_string();
        self.rows = rows;
        self.now = now;
        self.picked = self.picked.min(self.rows.len().saturating_sub(1));
    }

    /// Empty again (the list closed).
    pub fn clear(&mut self) {
        *self = HistoryPanel::default();
    }

    /// The wheel (or – / +): the outline moves `d` rows, and the list
    /// scrolls to keep it in view.
    pub fn wheel(&mut self, d: i32) {
        let last = self.rows.len().saturating_sub(1) as i32;
        self.picked = (self.picked as i32 + d).clamp(0, last.max(0)) as usize;
        self.top = None;
    }

    /// The card in `r`: centred, as wide as fits up to [`CARD_W`].
    fn card(r: Rect) -> Rect {
        let w = r.w.min(CARD_W);
        Rect::new(r.x + (r.w - w) / 2, r.y, w, r.h)
    }

    /// Where the rows go.
    fn body(r: Rect) -> Rect {
        let c = HistoryPanel::card(r).inset(16);
        Rect::new(c.x, c.y + HEAD_H, c.w, c.h - HEAD_H - FOOT_H - GAP)
    }

    /// Close's button.
    pub fn close_rect(r: Rect) -> Rect {
        let c = HistoryPanel::card(r).inset(16);
        Rect::new(c.right() - 220, c.bottom() - FOOT_H, 220, FOOT_H)
    }

    /// How many rows fit in `body`.
    fn fit(body: Rect) -> usize {
        ((body.h + GAP) / (ROW_H + GAP)).max(1) as usize
    }

    /// The first row shown: where a drag left it, or with the outlined row
    /// and up to two after it in view.
    fn first(&self, body: Rect) -> usize {
        let fit = HistoryPanel::fit(body);
        let last = self.rows.len().saturating_sub(fit);
        self.top
            .unwrap_or_else(|| (self.picked + 3).saturating_sub(fit))
            .min(last)
    }

    /// The rows shown: (row, its rectangle).
    pub fn row_rects(&self, r: Rect) -> Vec<(usize, Rect)> {
        let body = HistoryPanel::body(r);
        let first = self.first(body);
        let fit = HistoryPanel::fit(body);
        (first..self.rows.len().min(first + fit))
            .enumerate()
            .map(|(i, k)| {
                let y = body.y + i as i32 * (ROW_H + GAP);
                (k, Rect::new(body.x, y, body.w, ROW_H))
            })
            .collect()
    }
}

impl Widget for HistoryPanel {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.canvas.fill_rect(r, t.bg);
        let card = HistoryPanel::card(r);
        cx.panel(card);
        let inner = card.inset(16);
        let head = Rect::new(inner.x, inner.y, inner.w, 32);
        cx.text(
            FontId::Bold,
            t.size_title,
            head,
            Align::Left,
            t.text,
            &self.title,
        );
        let shown = self.row_rects(r);
        let first = shown.first().map_or(0, |s| s.0);
        let last = shown.last().map_or(0, |s| s.0 + 1);
        if first > 0 || last < self.rows.len() {
            cx.text(
                FontId::Sans,
                t.size_body,
                head,
                Align::Right,
                t.text_dim,
                &format!("{} to {} of {}", first + 1, last, self.rows.len()),
            );
        }
        let sub = Rect::new(inner.x, inner.y + 30, inner.w, 26);
        let what = if self.rows.is_empty() {
            "Reading the history…"
        } else if self.rows.len() == 1 {
            "Nothing to undo here yet"
        } else {
            "Tap a step to go back (or forward) to just after it"
        };
        cx.text(
            FontId::Sans,
            t.size_body,
            sub,
            Align::Left,
            t.text_dim,
            what,
        );
        for &(k, sr) in &shown {
            let now = k == self.now;
            let after = k > self.now;
            cx.canvas.fill_round_rect(
                sr,
                10,
                if now {
                    t.raised
                } else {
                    t.bg.mix(t.surface, 0.6)
                },
            );
            if k == self.picked {
                cx.canvas.stroke_round_rect(sr, 10, 3, t.accent);
            }
            let ink = if after { t.text_faint } else { t.text };
            let label = Rect::new(sr.x + 18, sr.y, sr.w - 150, sr.h);
            cx.text(
                FontId::Bold,
                t.size_body,
                label,
                Align::Left,
                ink,
                &self.rows[k],
            );
            let tag = if now {
                "now"
            } else if after {
                "redo"
            } else {
                ""
            };
            cx.text(
                FontId::Sans,
                t.size_body,
                Rect::new(sr.right() - 130, sr.y, 112, sr.h),
                Align::Right,
                if now { t.text } else { t.text_faint },
                tag,
            );
        }
        let close = HistoryPanel::close_rect(r);
        cx.canvas.fill_round_rect(close, 12, t.raised);
        cx.text(
            FontId::Bold,
            t.size_title,
            close,
            Align::Center,
            t.button_ink(true),
            "Close",
        );
    }

    fn touch(&mut self, finger: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        let body = HistoryPanel::body(r);
        match phase {
            Touch::Down => {
                if HistoryPanel::close_rect(r).contains(x, y) {
                    return Some(EventKind::Tab(TAB_CLOSE));
                }
                if body.contains(x, y) {
                    self.press = Some((finger, y, self.first(body), false));
                }
                None
            }
            Touch::Move => {
                let (f, y0, top0, _) = self.press?;
                if f != finger || (y - y0).abs() < DRAG {
                    return None;
                }
                self.press = Some((f, y0, top0, true));
                let rows = (y0 - y) / (ROW_H + GAP);
                let last = self.rows.len().saturating_sub(HistoryPanel::fit(body));
                self.top = Some((top0 as i32 + rows).clamp(0, last as i32) as usize);
                None
            }
            Touch::Up => {
                let (f, _, _, dragged) = self.press.take()?;
                if f != finger || dragged {
                    return None;
                }
                let (k, _) = self
                    .row_rects(r)
                    .into_iter()
                    .find(|(_, sr)| sr.contains(x, y))?;
                self.picked = k;
                Some(EventKind::Select(k))
            }
            Touch::Cancel => {
                self.press = None;
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const R: Rect = Rect {
        x: 0,
        y: 0,
        w: 1256,
        h: 690,
    };

    fn panel(n: usize, now: usize) -> HistoryPanel {
        let mut p = HistoryPanel::default();
        let mut rows = vec!["Start".to_string()];
        rows.extend((1..n).map(|k| format!("Step {k}")));
        p.set("Undo history · Takes", rows, now);
        p
    }

    fn tap(p: &mut HistoryPanel, (x, y): (i32, i32)) -> Option<EventKind> {
        p.touch(1, Touch::Down, x, y, R);
        p.touch(1, Touch::Up, x, y, R)
    }

    fn mid(r: Rect) -> (i32, i32) {
        (r.x + r.w / 2, r.y + r.h / 2)
    }

    #[test]
    fn a_tap_on_a_row_picks_it_and_close_closes() {
        let mut p = panel(5, 3);
        let rows = p.row_rects(R);
        assert_eq!(rows.len(), 5, "five rows fit");
        assert_eq!(tap(&mut p, mid(rows[1].1)), Some(EventKind::Select(1)));
        assert_eq!(p.picked, 1);
        let c = HistoryPanel::close_rect(R);
        assert_eq!(
            p.touch(2, Touch::Down, c.x + 5, c.y + 5, R),
            Some(EventKind::Tab(TAB_CLOSE))
        );
    }

    #[test]
    fn the_list_follows_the_outline_and_a_drag_scrolls_without_picking() {
        let mut p = panel(40, 39);
        let shown = p.row_rects(R);
        assert_eq!(shown.last().unwrap().0, 39, "now in view at the end");
        // The wheel moves the outline up; the list follows it.
        p.wheel(-30);
        assert_eq!(p.picked, 9);
        assert!(p.row_rects(R).iter().any(|(k, _)| *k == 9));
        // A drag scrolls and picks nothing.
        let (x, y) = mid(p.row_rects(R)[2].1);
        p.touch(3, Touch::Down, x, y, R);
        p.touch(3, Touch::Move, x, y - 3 * (ROW_H + GAP), R);
        assert_eq!(p.touch(3, Touch::Up, x, y - 3 * (ROW_H + GAP), R), None);
        // Past the ends the outline stops.
        p.wheel(-100);
        assert_eq!(p.picked, 0);
        p.wheel(100);
        assert_eq!(p.picked, 39);
    }

    #[test]
    fn when_now_moves_the_outline_goes_back_to_it() {
        let mut p = panel(6, 5);
        p.wheel(-2);
        assert_eq!(p.picked, 3);
        // The same history again: the outline stays.
        let rows = p.rows.clone();
        p.set("Undo history · Takes", rows.clone(), 5);
        assert_eq!(p.picked, 3);
        // Now moved (two steps undone): the outline is on now.
        p.set("Undo history · Takes", rows, 3);
        assert_eq!((p.now, p.picked), (3, 3));
    }

    #[test]
    fn rows_and_close_are_big_enough_to_hit() {
        let p = panel(20, 0);
        for (k, r) in p.row_rects(R) {
            assert!(r.h >= 64 && r.w >= 64, "row {k}: {r:?}");
        }
        let c = HistoryPanel::close_rect(R);
        assert!(c.h >= 64 && c.w >= 64);
    }
}
