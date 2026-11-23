//! The conversion report card (the Arrange refactor's 4.3, layout spec
//! 4n): after Scenes → song, List → song or Song → scenes plays
//! differently from what it was made from, a card over the Song card (the
//! Song view or the song list) says how, one row a difference, by track
//! and bar, with UNDO on it.
//!
//! The Song card dims behind it. Its title line has the conversion and,
//! at its right, what it made and how many differences there are; under
//! it one line in plain words, then the differences under the column
//! names Track, Bar and What's different. Five rows show; more scroll
//! with a drag or the wheel, with "1 to 5 of 9" under them. The rows don't
//! answer a tap. UNDO (154 px) and Done (150 px, the accent) sit at its
//! bottom right. A touch outside the card is the app's: it closes the
//! card and does nothing else (`gui.rs`).
//!
//! The Song view and the song list each hold one while it is open
//! ([`crate::song::SongLanes::report`], [`crate::list::SongListView::report`]):
//! they draw it over themselves and hand it their touches.

use std::sync::Arc;

use mpfree_ui::{Align, Color, EventKind, FontId, PaintCx, Rect, Role, Target, Touch};

use crate::view::Report;

/// `Toggle(k)` from the card: its Done and its UNDO. Far from the Song
/// view's and the song list's own.
pub const REPORT_DONE: usize = 0x7E00;
pub const REPORT_UNDO: usize = 0x7E01;
/// Rows that show at once.
pub const SHOWN: usize = 5;
/// The card's width, and its height at most.
pub const CARD_W: i32 = 786;
pub const CARD_MAX_H: i32 = 498;
/// A row's height and the gap under it.
const ROW_H: i32 = 48;
const ROW_GAP: i32 = 6;
/// From the card's top: the title line, the lead line, the column names
/// and the first row.
const TITLE_Y: i32 = 18;
const LEAD_Y: i32 = 50;
const COLS_Y: i32 = 92;
const ROWS_Y: i32 = 116;
/// The "1 to 5 of 9" line under the rows, when they scroll.
const MORE_H: i32 = 28;
/// The buttons: their height, from the card's bottom, and their widths.
const BUTTON_H: i32 = 48;
const BUTTONS_UP: i32 = 66;
const UNDO_W: i32 = 154;
const DONE_W: i32 = 150;
/// The columns, from the card's left edge.
const TRACK_X: i32 = 32;
const BAR_X: i32 = 168;
const WHAT_X: i32 = 248;
/// How far a finger moves on the rows before it scrolls.
const DRAG_PX: i32 = 12;
/// How dark the Song card gets behind the card.
const DIM: u8 = 150;

/// The card as the Song view or the song list shows it: see the module
/// notes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ReportCard {
    pub report: Arc<Report>,
    /// Each row's track name and colour, in the report's order: None for
    /// one that is the whole song's ("Song", in the text colour).
    pub names: Vec<(String, Option<Color>)>,
    /// The first row shown.
    pub top: usize,
    /// A finger on the rows: which, where it went down, the first row
    /// then, and whether it has dragged.
    grab: Option<(u64, i32, usize, bool)>,
}

/// Where the card's parts are.
pub struct CardParts {
    pub card: Rect,
    /// The rows' area (the rows shown, top to bottom).
    pub rows: Rect,
    pub undo: Rect,
    pub done: Rect,
}

impl ReportCard {
    /// The card for `report`, with each track's name and colour (by track,
    /// as the Song view's lanes have them).
    pub fn new(report: Arc<Report>, tracks: &[(String, Color)]) -> ReportCard {
        let names = report
            .rows
            .iter()
            .map(|r| match r.track {
                Some(t) => match tracks.get(t) {
                    Some((name, colour)) => (name.clone(), Some(*colour)),
                    None => (format!("Track {}", t + 1), None),
                },
                None => ("Song".to_string(), None),
            })
            .collect();
        ReportCard {
            report,
            names,
            top: 0,
            grab: None,
        }
    }

    /// The same report as `old` (by its contents): the scroll stays.
    pub fn keep_scroll(&mut self, old: &ReportCard) {
        if self.report == old.report {
            self.top = old.top;
            self.grab = old.grab;
        }
    }

    fn shown(&self) -> usize {
        self.report.rows.len().min(SHOWN)
    }

    fn scrolls(&self) -> bool {
        self.report.rows.len() > SHOWN
    }

    fn last_top(&self) -> usize {
        self.report.rows.len().saturating_sub(SHOWN)
    }

    /// Where the card goes in the Song card's rectangle `r`: as tall as its
    /// rows need, in the middle.
    pub fn parts(&self, r: Rect) -> CardParts {
        let shown = self.shown() as i32;
        let more = if self.scrolls() { MORE_H } else { 0 };
        let h = (ROWS_Y + shown * (ROW_H + ROW_GAP) - ROW_GAP + more + 24 + BUTTONS_UP)
            .min(CARD_MAX_H)
            .min(r.h);
        let w = CARD_W.min(r.w - 32).max(0);
        let card = Rect::new(r.x + (r.w - w) / 2, r.y + (r.h - h).max(0) / 2, w, h);
        let rows = Rect::new(
            card.x + 16,
            card.y + ROWS_Y,
            card.w - 32,
            shown * (ROW_H + ROW_GAP) - ROW_GAP,
        );
        let by = card.bottom() - BUTTONS_UP;
        let done = Rect::new(card.right() - 24 - DONE_W, by, DONE_W, BUTTON_H);
        let undo = Rect::new(done.x - 12 - UNDO_W, by, UNDO_W, BUTTON_H);
        CardParts {
            card,
            rows,
            undo,
            done,
        }
    }

    /// The card's rectangle in the Song card's rectangle `r`.
    pub fn card_rect(&self, r: Rect) -> Rect {
        self.parts(r).card
    }

    /// The wheel turned `delta` detents: the rows scroll (down for a turn
    /// to the right). False when they didn't move.
    pub fn scroll(&mut self, delta: i32) -> bool {
        let top = (self.top as i64 + i64::from(delta)).clamp(0, self.last_top() as i64) as usize;
        let moved = top != self.top;
        self.top = top;
        moved
    }

    /// The right of the title line: "rows 9 to 11 · 4 differences".
    pub fn made_said(&self) -> String {
        let n = self.report.rows.len();
        let s = if n == 1 { "" } else { "s" };
        format!("{} · {n} difference{s}", self.report.made)
    }

    /// "1 to 5 of 9 · drag or turn the wheel for the rest", when the rows
    /// scroll.
    pub fn more_said(&self) -> Option<String> {
        self.scrolls().then(|| {
            let top = self.top.min(self.last_top());
            format!(
                "{} to {} of {} · drag or turn the wheel for the rest",
                top + 1,
                top + SHOWN,
                self.report.rows.len()
            )
        })
    }

    /// Draw it over the Song card in `r` (already drawn), which dims.
    pub fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.canvas.fill_rect(r, t.role(Role::Shadow).with_alpha(DIM));
        let p = self.parts(r);
        let c = p.card;
        cx.canvas.fill_round_rect(c, t.radius, t.surface);
        cx.canvas.stroke_round_rect(c, t.radius, 2, t.outline);
        let line = |y: i32, h: i32| Rect::new(c.x + 24, c.y + y, c.w - 48, h);
        cx.text(
            FontId::Bold,
            t.size_small,
            line(TITLE_Y, 20),
            Align::Left,
            t.text_dim,
            &self.report.title,
        );
        cx.text(
            FontId::Sans,
            t.size_small,
            line(TITLE_Y, 20),
            Align::Right,
            t.text_dim,
            &self.made_said(),
        );
        cx.text(
            FontId::Bold,
            t.size_title,
            line(LEAD_Y, 26),
            Align::Left,
            t.text,
            &self.report.lead,
        );
        for (x, w, word) in [
            (TRACK_X - 8, BAR_X - TRACK_X, "Track"),
            (BAR_X, WHAT_X - BAR_X, "Bar"),
            (WHAT_X, c.w - WHAT_X - 24, "What's different"),
        ] {
            cx.text(
                FontId::Sans,
                t.size_small - 1.0,
                Rect::new(c.x + x, c.y + COLS_Y, w, 18),
                Align::Left,
                t.text_dim,
                word,
            );
        }
        let top = self.top.min(self.last_top());
        for (k, (row, (name, colour))) in self
            .report
            .rows
            .iter()
            .zip(&self.names)
            .enumerate()
            .skip(top)
            .take(SHOWN)
        {
            let y = p.rows.y + (k - top) as i32 * (ROW_H + ROW_GAP);
            let box_ = Rect::new(p.rows.x, y, p.rows.w, ROW_H);
            cx.canvas.fill_round_rect(box_, t.round(8), t.raised);
            // A track's name in its colour, moved just far enough to read
            // on the row (a light theme's pale rows): as drawn where it does.
            let ink = colour.map_or(t.text, |c| t.readable(c, t.raised));
            cx.canvas.fill_round_rect(
                Rect::new(box_.x, box_.y, 5, box_.h),
                t.round(3),
                colour.unwrap_or(t.text_dim),
            );
            let cell = |x: i32, w: i32| Rect::new(c.x + x, y, w, ROW_H);
            cx.text(
                FontId::Bold,
                t.size_body - 1.0,
                cell(TRACK_X, BAR_X - TRACK_X - 8),
                Align::Left,
                ink,
                name,
            );
            cx.text(
                FontId::Mono,
                t.size_mono + 1.0,
                cell(BAR_X, WHAT_X - BAR_X - 8),
                Align::Left,
                t.text,
                &format!("bar {}", row.bar),
            );
            cx.text(
                FontId::Sans,
                t.size_small + 1.0,
                cell(WHAT_X, c.w - WHAT_X - 24),
                Align::Left,
                t.text,
                &row.what,
            );
        }
        if let Some(more) = self.more_said() {
            cx.text(
                FontId::Sans,
                t.size_small - 1.0,
                Rect::new(c.x + 24, p.rows.bottom() + 6, c.w - 48, 18),
                Align::Left,
                t.text_dim,
                &more,
            );
        }
        cx.text(
            FontId::Sans,
            t.size_small,
            Rect::new(c.x + 24, p.undo.y, p.undo.x - c.x - 36, BUTTON_H),
            Align::Left,
            t.text_dim,
            "UNDO takes the whole conversion back",
        );
        cx.canvas.fill_round_rect(p.undo, t.round(10), t.raised);
        cx.text(
            FontId::Bold,
            t.size_body - 1.0,
            p.undo,
            Align::Center,
            t.text,
            "UNDO",
        );
        cx.canvas.fill_round_rect(p.done, t.round(10), t.accent);
        cx.text(
            FontId::Bold,
            t.size_body - 1.0,
            p.done,
            Align::Center,
            t.on_accent,
            "Done",
        );
    }

    /// Its controls in the Song card's rectangle `r`: Done, UNDO and, when
    /// they scroll, the rows.
    pub fn targets(&self, r: Rect) -> Vec<Target> {
        let p = self.parts(r);
        let mut out = vec![
            Target::tap("report-undo", 0, "UNDO", p.undo),
            Target::tap("report-done", 0, "Done", p.done),
        ];
        if self.scrolls() {
            out.push(Target::drag("report-rows", 0, "The differences", p.rows));
        }
        out
    }

    /// A finger on the card (the app keeps those outside it): Done and
    /// UNDO on their tap, the rows scroll with a drag.
    pub fn touch(
        &mut self,
        finger: u64,
        phase: Touch,
        x: i32,
        y: i32,
        r: Rect,
    ) -> Option<EventKind> {
        let p = self.parts(r);
        match phase {
            Touch::Down => {
                if p.done.contains(x, y) {
                    return Some(EventKind::Toggle(REPORT_DONE));
                }
                if p.undo.contains(x, y) {
                    return Some(EventKind::Toggle(REPORT_UNDO));
                }
                if self.scrolls() && p.rows.contains(x, y) {
                    self.grab = Some((finger, y, self.top, false));
                }
                None
            }
            Touch::Move => {
                let last = self.last_top() as i32;
                let g = self.grab.as_mut().filter(|g| g.0 == finger)?;
                if !g.3 && (y - g.1).abs() < DRAG_PX {
                    return None;
                }
                g.3 = true;
                let moved = (g.1 - y) / (ROW_H + ROW_GAP);
                self.top = (g.2 as i32 + moved).clamp(0, last) as usize;
                None
            }
            Touch::Up | Touch::Cancel => {
                self.grab = self.grab.filter(|g| g.0 != finger);
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::ReportRow;

    fn card(n: usize) -> ReportCard {
        let rows = (0..n)
            .map(|k| ReportRow {
                track: (k % 2 == 1).then_some(k % 3),
                bar: 1 + 4 * k as i64,
                what: format!("difference {}", k + 1),
            })
            .collect();
        let report = Arc::new(Report {
            title: "SONG → SCENES".into(),
            made: "rows 9 to 11".into(),
            lead: "Launch row 9 and the rows play the song, except:".into(),
            rows,
        });
        let tracks = vec![
            ("Drums".to_string(), Color::rgb(0xe8b56a)),
            ("Bass".to_string(), Color::rgb(0x8db9fb)),
        ];
        ReportCard::new(report, &tracks)
    }

    /// The Song view's rectangle on the 1280 × 800 screen.
    const SONG: Rect = Rect {
        x: 0,
        y: 104,
        w: 902,
        h: 606,
    };

    #[test]
    fn the_card_is_as_tall_as_its_rows_and_names_each_track() {
        let one = card(1);
        let p = one.parts(SONG);
        assert_eq!((p.card.w, p.card.h), (CARD_W, 254));
        assert_eq!(one.names[0], ("Song".to_string(), None));
        assert_eq!(one.made_said(), "rows 9 to 11 · 1 difference");
        assert_eq!(one.more_said(), None);
        let four = card(4);
        assert_eq!(four.names[1].0, "Bass");
        assert_eq!(four.names[3].0, "Drums");
        // A track the lanes don't have (yet): its number.
        let lost = ReportCard::new(four.report.clone(), &[]);
        assert_eq!(lost.names[1], ("Track 2".to_string(), None));
        // Five and more: the card is as tall as it gets, and says where
        // the rows are.
        let nine = card(9);
        let p = nine.parts(SONG);
        assert_eq!(p.card.h, CARD_MAX_H);
        assert_eq!(p.card.y, SONG.y + (SONG.h - CARD_MAX_H) / 2);
        assert_eq!(
            nine.more_said().unwrap(),
            "1 to 5 of 9 · drag or turn the wheel for the rest"
        );
        // The buttons at its bottom right, UNDO then Done.
        assert_eq!(p.done.right(), p.card.right() - 24);
        assert_eq!((p.undo.w, p.done.w), (UNDO_W, DONE_W));
        assert!(p.undo.right() < p.done.x);
    }

    #[test]
    fn done_and_undo_answer_their_tap_and_the_rows_scroll() {
        let mut nine = card(9);
        let p = nine.parts(SONG);
        let (dx, dy) = (p.done.x + p.done.w / 2, p.done.y + p.done.h / 2);
        assert_eq!(
            nine.touch(1, Touch::Down, dx, dy, SONG),
            Some(EventKind::Toggle(REPORT_DONE))
        );
        let (ux, uy) = (p.undo.x + p.undo.w / 2, p.undo.y + p.undo.h / 2);
        assert_eq!(
            nine.touch(2, Touch::Down, ux, uy, SONG),
            Some(EventKind::Toggle(REPORT_UNDO))
        );
        // A tap on a row does nothing; a drag up shows the later rows.
        let (rx, ry) = (p.rows.x + 40, p.rows.bottom() - 10);
        assert_eq!(nine.touch(3, Touch::Down, rx, ry, SONG), None);
        assert_eq!(nine.touch(3, Touch::Up, rx, ry, SONG), None);
        assert_eq!(nine.top, 0);
        nine.touch(3, Touch::Down, rx, ry, SONG);
        nine.touch(3, Touch::Move, rx, ry - 3 * (ROW_H + ROW_GAP), SONG);
        nine.touch(3, Touch::Up, rx, ry - 3 * (ROW_H + ROW_GAP), SONG);
        assert_eq!(nine.top, 3);
        assert!(nine.more_said().unwrap().starts_with("4 to 8 of 9"));
        // The wheel: to the last five, and no further.
        assert!(nine.scroll(5));
        assert_eq!(nine.top, 4);
        assert!(!nine.scroll(1));
        assert!(nine.scroll(-9));
        assert_eq!(nine.top, 0);
        // The targets: UNDO, Done and the rows (only when they scroll).
        let ids: Vec<&str> = nine.targets(SONG).iter().map(|t| t.id).collect();
        assert_eq!(ids, ["report-undo", "report-done", "report-rows"]);
        assert_eq!(card(3).targets(SONG).len(), 2);
    }
}
