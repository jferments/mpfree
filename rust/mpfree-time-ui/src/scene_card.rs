//! The scene card (the Arrange refactor's 5.2, layout spec 4l): holding
//! a row's number on the Clips grid (or a step's number on the song list)
//! opens a card for that row's scene over the grid or the list, which dim
//! behind it, as the conversion report card does.
//!
//! - **The title line:** "SCENE · ROW 3 OF 16" and, at its right, how long
//!   the scene plays and whether it plays now; under it ‹ the scene's name
//!   › (a plain row is "Row 3"): ‹ › go to the scene before or after, and a
//!   tap on the name renames it.
//! - **The settings**, in two columns of rows, each its name and its value
//!   between ‹ and ›: Follow, After, Chance, Or and Tempo; Mutes, Program,
//!   Bank and Sent to. A tap on ‹ or › steps it (`Step { index, delta }`,
//!   the index a [`crate::view::SCENE_ROWS`] one); a tap on the name picks
//!   the row for the wheel and - / +. A tap on Tempo's value sets the tempo
//!   now, or takes the scene's tempo off ("as it is"); a tap on Mutes'
//!   value sets the mutes as they are now.
//! - **Empty cells:** a box a track, eight at a time (‹ › for more): the
//!   clip the row has for the track, or for an empty cell Stop (the track
//!   stops as the scene starts) or Keep (what it plays runs on). A tap on
//!   an empty cell's box turns it over.
//! - **Along its foot:** Capture (the clips playing now into a new row),
//!   Copy row, Insert row and Delete row, and Done.
//!
//! The grid and the song list each hold one while it is open
//! ([`crate::widgets::ClipGrid::scene_card`],
//! [`crate::list::SongListView::scene_card`]): they draw it over
//! themselves and hand it their touches. A touch on them outside the card
//! closes it.

use mpfree_ui::widgets::OptionRow;
use mpfree_ui::{Align, Color, EventKind, FontId, PaintCx, Rect, Role, Target, Touch};

use crate::view::SCENE_ROWS;

/// `Toggle(k)` from the card. Far from the grid's and the list's own, and
/// from the report card's.
pub const SCENE_DONE: usize = 0x7D00;
pub const SCENE_BEFORE: usize = 0x7D01;
pub const SCENE_AFTER: usize = 0x7D02;
pub const SCENE_RENAME: usize = 0x7D03;
pub const SCENE_CAPTURE: usize = 0x7D04;
pub const SCENE_COPY_ROW: usize = 0x7D05;
pub const SCENE_INSERT_ROW: usize = 0x7D06;
pub const SCENE_DELETE_ROW: usize = 0x7D07;
/// A tap on Tempo's value: the tempo now, or back to "as it is".
pub const SCENE_TEMPO_NOW: usize = 0x7D08;
/// A tap on Mutes' value: the tracks muted now, as the scene's mutes.
pub const SCENE_MUTES_NOW: usize = 0x7D09;
/// `Toggle(SCENE_PICK + index)`: a row's name, picked for the wheel.
pub const SCENE_PICK: usize = 0x7D40;
/// `Toggle(SCENE_KEEP + track)`: an empty cell's box, Stop or Keep.
pub const SCENE_KEEP: usize = 0x7C00;
/// The settings' rows on the left and on the right, as
/// [`crate::view::SCENE_ROWS`] indexes.
pub const LEFT_ROWS: [usize; 5] = [1, 2, 3, 4, 9];
pub const RIGHT_ROWS: [usize; 4] = [5, 6, 7, 8];
/// Empty cells' boxes shown at once.
pub const CELLS_SHOWN: usize = 8;
/// The card's width and height at most.
pub const CARD_W: i32 = 786;
pub const CARD_H: i32 = 494;
/// From the card's top: the title line, the name, the rows, the empty
/// cells' line and their boxes.
const EYEBROW_Y: i32 = 14;
const NAME_Y: i32 = 36;
const NAME_H: i32 = 42;
const ROWS_Y: i32 = 90;
const ROW_H: i32 = 44;
const ROW_GAP: i32 = 6;
const CELLS_Y: i32 = 344;
const CELLS_BOX_Y: i32 = 366;
const CELL_H: i32 = 50;
const CELL_GAP: i32 = 8;
/// A row's name, and an arrow of its value box.
const ROW_NAME_W: i32 = 96;
const ARROW_W: i32 = 40;
/// The buttons along the foot: from the card's bottom, their height, and
/// the widths.
const FOOT_UP: i32 = 60;
const BUTTON_H: i32 = 44;
const BUTTON_W: i32 = 112;
const DONE_W: i32 = 120;
const BUTTON_GAP: i32 = 10;
/// The names' ‹ and › and the cells' paging arrows.
const NAME_ARROW_W: i32 = 48;
const PAGE_W: i32 = 30;
/// How dark the grid gets behind the card.
const DIM: u8 = 150;
/// The foot's buttons, left to right.
pub const FOOT: [(usize, &str); 4] = [
    (SCENE_CAPTURE, "Capture"),
    (SCENE_COPY_ROW, "Copy row"),
    (SCENE_INSERT_ROW, "Insert row"),
    (SCENE_DELETE_ROW, "Delete row"),
];

/// What a track's cell in the scene's row is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CardCell {
    /// The row has a clip for the track: it plays as the scene starts.
    Clip(String),
    /// An empty cell that stops the track.
    Stop,
    /// An empty cell the track plays on through (Keep playing).
    Keep,
}

/// A track's box under Empty cells.
#[derive(Clone, Debug, PartialEq)]
pub struct CardTrack {
    pub track: usize,
    pub name: String,
    pub color: Color,
    pub cell: CardCell,
}

/// The card as the grid or the list shows it: see the module notes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SceneCard {
    /// The scene's row.
    pub row: usize,
    /// The rows the grid has (the card's ‹ › go 1 to this).
    pub rows_count: usize,
    /// "Drop", or "Row 3" for a plain row.
    pub name: String,
    /// The right of the title line: "4 bars · playing".
    pub about: String,
    /// The settings, one a [`SCENE_ROWS`] index.
    pub rows: Vec<OptionRow>,
    /// The row the wheel and - / + turn, a [`SCENE_ROWS`] index.
    pub picked: usize,
    /// Every track but the audio tracks, in order.
    pub tracks: Vec<CardTrack>,
    /// The first track shown under Empty cells.
    pub first: usize,
    /// Capture, Copy row, Insert row and Delete row: whether each can.
    pub can: [bool; 4],
}

/// Where the card's parts are.
pub struct SceneParts {
    pub card: Rect,
    pub before: Rect,
    pub name: Rect,
    pub after: Rect,
    /// Each settings row: its [`SCENE_ROWS`] index and its rectangle.
    pub rows: Vec<(usize, Rect)>,
    /// The empty cells' boxes shown: the index in [`SceneCard::tracks`]
    /// and the rectangle.
    pub cells: Vec<(usize, Rect)>,
    /// The cells' paging arrows, when there are more than [`CELLS_SHOWN`].
    pub page_back: Option<Rect>,
    pub page_on: Option<Rect>,
    /// The foot's buttons ([`FOOT`]) and Done.
    pub foot: [Rect; 4],
    pub done: Rect,
}

impl SceneCard {
    /// The same card as `old` (its row and tracks): the cells' page stays.
    pub fn keep_page(&mut self, old: &SceneCard) {
        if self.row == old.row && self.tracks.len() == old.tracks.len() {
            self.first = old.first;
        }
    }

    fn pages(&self) -> bool {
        self.tracks.len() > CELLS_SHOWN
    }

    /// Where the card goes in the grid's (or the list's) rectangle `r`:
    /// in the middle.
    pub fn parts(&self, r: Rect) -> SceneParts {
        let w = CARD_W.min(r.w - 32).max(0);
        let h = CARD_H.min(r.h - 12).max(0);
        let c = Rect::new(r.x + (r.w - w) / 2, r.y + (r.h - h).max(0) / 2, w, h);
        let inner = Rect::new(c.x + 24, c.y, c.w - 48, c.h);
        let before = Rect::new(inner.x, c.y + NAME_Y, NAME_ARROW_W, NAME_H);
        let name = Rect::new(before.right() + 6, before.y, 300, NAME_H);
        let after = Rect::new(name.right() + 6, before.y, NAME_ARROW_W, NAME_H);
        let col_w = (inner.w - 24) / 2;
        let mut rows = Vec::new();
        for (col, list) in [(0, &LEFT_ROWS[..]), (1, &RIGHT_ROWS[..])] {
            let x = inner.x + col * (col_w + 24);
            for (k, &index) in list.iter().enumerate() {
                let y = c.y + ROWS_Y + k as i32 * (ROW_H + ROW_GAP);
                rows.push((index, Rect::new(x, y, col_w, ROW_H)));
            }
        }
        let (page_back, page_on, strip) = if self.pages() {
            let y = c.y + CELLS_BOX_Y;
            (
                Some(Rect::new(inner.x, y, PAGE_W, CELL_H)),
                Some(Rect::new(inner.right() - PAGE_W, y, PAGE_W, CELL_H)),
                Rect::new(inner.x + PAGE_W + 6, y, inner.w - 2 * (PAGE_W + 6), CELL_H),
            )
        } else {
            (
                None,
                None,
                Rect::new(inner.x, c.y + CELLS_BOX_Y, inner.w, CELL_H),
            )
        };
        let n = CELLS_SHOWN as i32;
        let cell_w = (strip.w - (n - 1) * CELL_GAP) / n;
        let first = self.first.min(self.tracks.len().saturating_sub(1));
        let cells = (first..self.tracks.len())
            .take(CELLS_SHOWN)
            .enumerate()
            .map(|(k, i)| {
                let x = strip.x + k as i32 * (cell_w + CELL_GAP);
                (i, Rect::new(x, strip.y, cell_w, CELL_H))
            })
            .collect();
        let by = c.bottom() - FOOT_UP;
        let mut foot = [Rect::new(0, 0, 0, 0); 4];
        for (k, b) in foot.iter_mut().enumerate() {
            let x = inner.x + k as i32 * (BUTTON_W + BUTTON_GAP);
            *b = Rect::new(x, by, BUTTON_W, BUTTON_H);
        }
        let done = Rect::new(inner.right() - DONE_W, by, DONE_W, BUTTON_H);
        SceneParts {
            card: c,
            before,
            name,
            after,
            rows,
            cells,
            page_back,
            page_on,
            foot,
            done,
        }
    }

    /// The card's rectangle in the grid's (or the list's) rectangle `r`.
    pub fn card_rect(&self, r: Rect) -> Rect {
        self.parts(r).card
    }

    /// A row's name and value box.
    fn row_parts(row: Rect) -> (Rect, Rect, Rect, Rect) {
        let (name, value) = row.take_left(ROW_NAME_W);
        let (left, mid) = value.take_left(ARROW_W);
        let (right, mid) = mid.take_right(ARROW_W);
        (name, left, mid, right)
    }

    fn value(&self, index: usize) -> &str {
        self.rows.get(index).map_or("", |r| r.value.as_str())
    }

    /// Draw it over the grid (or the list) in `r`, which dims.
    pub fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.canvas.fill_rect(r, t.role(Role::Shadow).with_alpha(DIM));
        let p = self.parts(r);
        let c = p.card;
        cx.canvas.fill_round_rect(c, t.radius, t.surface);
        cx.canvas.stroke_round_rect(c, t.radius, 2, t.outline);
        let line = Rect::new(c.x + 24, c.y + EYEBROW_Y, c.w - 48, 20);
        cx.text(
            FontId::Bold,
            t.size_small,
            line,
            Align::Left,
            t.text_dim,
            &format!("SCENE · ROW {} OF {}", self.row + 1, self.rows_count),
        );
        cx.text(
            FontId::Sans,
            t.size_small,
            line,
            Align::Right,
            t.text_dim,
            &self.about,
        );
        // ‹ the name ›: the name a box a tap renames.
        for (b, arrow, live) in [
            (p.before, "‹", self.row > 0),
            (p.after, "›", self.row + 1 < self.rows_count),
        ] {
            cx.canvas.fill_round_rect(b, t.round(10), t.raised);
            cx.text(
                FontId::Bold,
                t.size_title + 6.0,
                b,
                Align::Center,
                if live { t.text } else { t.outline },
                arrow,
            );
        }
        cx.canvas.fill_round_rect(p.name, t.round(10), t.raised);
        let (pencil, name_text) = p.name.inset_xy(14, 0).take_right(80);
        cx.text(
            FontId::Bold,
            t.size_title + 2.0,
            name_text,
            Align::Left,
            t.text,
            &self.name,
        );
        cx.text(
            FontId::Sans,
            t.size_small - 1.0,
            pencil,
            Align::Right,
            t.text_dim,
            "Rename",
        );
        // The settings.
        for &(index, row) in &p.rows {
            let (name, left, mid, right) = SceneCard::row_parts(row);
            let on = index == self.picked;
            cx.text(
                FontId::Sans,
                t.size_small + 1.0,
                name,
                Align::Left,
                if on { t.accent } else { t.text_dim },
                SCENE_ROWS[index],
            );
            let boxr = Rect::new(left.x, row.y, right.right() - left.x, row.h);
            cx.canvas.fill_round_rect(boxr, t.round(10), t.raised);
            if on {
                cx.canvas.stroke_round_rect(boxr, t.round(10), 2, t.accent);
            }
            let arrows = if on { t.accent } else { t.text_dim };
            for (b, a) in [(left, "‹"), (right, "›")] {
                cx.text(
                    FontId::Bold,
                    t.size_title + 2.0,
                    b,
                    Align::Center,
                    arrows,
                    a,
                );
            }
            let value = self.value(index);
            let dim = value == "—";
            cx.text(
                FontId::Bold,
                t.size_small + 1.0,
                mid,
                Align::Center,
                if dim { t.text_dim } else { t.text },
                value,
            );
        }
        // Empty cells: Stop or Keep for each track the row has no clip
        // for.
        cx.text(
            FontId::Bold,
            t.size_small - 1.0,
            Rect::new(c.x + 24, c.y + CELLS_Y, c.w - 48, 18),
            Align::Left,
            t.text_dim,
            "EMPTY CELLS · as the scene starts, a track with no clip in the row stops, or keeps playing",
        );
        for (b, arrow, live) in [
            (p.page_back, "‹", self.first > 0),
            (p.page_on, "›", self.first + CELLS_SHOWN < self.tracks.len()),
        ] {
            let Some(b) = b else { continue };
            cx.canvas.fill_round_rect(b, t.round(8), t.raised);
            cx.text(
                FontId::Bold,
                t.size_title,
                b,
                Align::Center,
                if live { t.text } else { t.outline },
                arrow,
            );
        }
        for &(i, b) in &p.cells {
            let tr = &self.tracks[i];
            let (fill, edge) = match tr.cell {
                CardCell::Clip(_) => (t.surface, Some(t.outline)),
                CardCell::Stop => (t.raised, None),
                CardCell::Keep => (t.raised.mix(t.good, 0.18), Some(t.good)),
            };
            cx.canvas.fill_round_rect(b, t.round(8), fill);
            if let Some(e) = edge {
                cx.canvas.stroke_round_rect(b, t.round(8), 2, e);
            }
            cx.canvas
                .fill_round_rect(Rect::new(b.x, b.y, 4, b.h), t.round(3), tr.color);
            let text = b.inset_xy(10, 4);
            let (top, bottom) = text.take_top(text.h / 2);
            cx.text(
                FontId::Bold,
                t.size_small - 1.0,
                top,
                Align::Left,
                t.readable(tr.color, fill),
                &tr.name,
            );
            let (word, ink, font) = match &tr.cell {
                CardCell::Clip(name) => (name.as_str(), t.text_dim, FontId::Sans),
                CardCell::Stop => ("Stop", t.text, FontId::Bold),
                CardCell::Keep => ("Keep", t.readable(t.good, fill), FontId::Bold),
            };
            cx.text(font, t.size_small - 1.0, bottom, Align::Left, ink, word);
        }
        // The foot.
        for (k, (_, word)) in FOOT.iter().enumerate() {
            let b = p.foot[k];
            let live = self.can[k];
            cx.canvas
                .fill_round_rect(b, t.round(10), if live { t.raised } else { t.surface });
            if !live {
                cx.canvas.stroke_round_rect(b, t.round(10), 1, t.outline);
            }
            let ink = match (live, k) {
                (false, _) => t.text_dim,
                (true, 3) => t.readable(t.bad, t.raised),
                (true, _) => t.text,
            };
            cx.text(FontId::Bold, t.size_small, b, Align::Center, ink, word);
        }
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

    /// Its controls in the grid's (or the list's) rectangle `r`.
    pub fn targets(&self, r: Rect) -> Vec<Target> {
        let p = self.parts(r);
        let mut out = vec![
            Target::tap("scene-before", 0, "Scene before", p.before).disabled_if(self.row == 0),
            Target::tap("scene-name", 0, "Rename", p.name),
            Target::tap("scene-after", 0, "Scene after", p.after)
                .disabled_if(self.row + 1 >= self.rows_count),
        ];
        for &(index, row) in &p.rows {
            let (name, left, mid, right) = SceneCard::row_parts(row);
            let label = SCENE_ROWS[index];
            out.push(Target::tap("scene-row", index, label, name));
            out.push(Target::tap("scene-less", index, format!("{label} ‹"), left));
            out.push(Target::tap(
                "scene-more",
                index,
                format!("{label} ›"),
                right,
            ));
            if index == crate::view::SCENE_TEMPO {
                out.push(Target::tap("scene-tempo-now", 0, "Tempo now", mid));
            }
            if index == crate::view::SCENE_MUTES {
                out.push(Target::tap("scene-mutes-now", 0, "Mutes now", mid));
            }
        }
        for &(i, b) in &p.cells {
            let tr = &self.tracks[i];
            out.push(
                Target::tap("scene-cell", tr.track, tr.name.clone(), b)
                    .disabled_if(matches!(tr.cell, CardCell::Clip(_))),
            );
        }
        if let Some(b) = p.page_back {
            out.push(
                Target::tap("scene-cells-back", 0, "Tracks before", b).disabled_if(self.first == 0),
            );
        }
        if let Some(b) = p.page_on {
            out.push(
                Target::tap("scene-cells-on", 0, "Tracks after", b)
                    .disabled_if(self.first + CELLS_SHOWN >= self.tracks.len()),
            );
        }
        for (k, (_, word)) in FOOT.iter().enumerate() {
            let id = if k == 3 { "scene-delete" } else { "scene-foot" };
            out.push(Target::tap(id, k, *word, p.foot[k]).disabled_if(!self.can[k]));
        }
        out.push(Target::tap("scene-done", 0, "Done", p.done));
        out
    }

    /// A finger on the grid (or the list) while the card is open: its
    /// controls on the way down, the cells' paging here; a touch outside
    /// the card closes it.
    pub fn touch(&mut self, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if phase != Touch::Down {
            return None;
        }
        let p = self.parts(r);
        if !p.card.contains(x, y) {
            return Some(EventKind::Toggle(SCENE_DONE));
        }
        if p.done.contains(x, y) {
            return Some(EventKind::Toggle(SCENE_DONE));
        }
        if p.before.contains(x, y) {
            return Some(EventKind::Toggle(SCENE_BEFORE));
        }
        if p.after.contains(x, y) {
            return Some(EventKind::Toggle(SCENE_AFTER));
        }
        if p.name.contains(x, y) {
            return Some(EventKind::Toggle(SCENE_RENAME));
        }
        for &(index, row) in &p.rows {
            if !row.contains(x, y) {
                continue;
            }
            let (name, left, mid, right) = SceneCard::row_parts(row);
            if left.contains(x, y) {
                return Some(EventKind::Step { index, delta: -1 });
            }
            if right.contains(x, y) {
                return Some(EventKind::Step { index, delta: 1 });
            }
            if index == crate::view::SCENE_TEMPO && mid.contains(x, y) {
                return Some(EventKind::Toggle(SCENE_TEMPO_NOW));
            }
            if index == crate::view::SCENE_MUTES && mid.contains(x, y) {
                return Some(EventKind::Toggle(SCENE_MUTES_NOW));
            }
            if name.contains(x, y) || mid.contains(x, y) {
                return Some(EventKind::Toggle(SCENE_PICK + index));
            }
            return None;
        }
        if p.page_back.is_some_and(|b| b.contains(x, y)) {
            self.first = self.first.saturating_sub(CELLS_SHOWN);
            return None;
        }
        if p.page_on.is_some_and(|b| b.contains(x, y)) {
            if self.first + CELLS_SHOWN < self.tracks.len() {
                self.first += CELLS_SHOWN;
            }
            return None;
        }
        for &(i, b) in &p.cells {
            if b.contains(x, y) {
                let tr = &self.tracks[i];
                return match tr.cell {
                    CardCell::Clip(_) => None,
                    _ => Some(EventKind::Toggle(SCENE_KEEP + tr.track)),
                };
            }
        }
        for (k, (ev, _)) in FOOT.iter().enumerate() {
            if p.foot[k].contains(x, y) {
                return self.can[k].then_some(EventKind::Toggle(*ev));
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(tracks: usize) -> SceneCard {
        SceneCard {
            row: 2,
            rows_count: 16,
            name: "Drop".into(),
            about: "4 bars".into(),
            rows: SCENE_ROWS
                .iter()
                .map(|n| OptionRow {
                    name: n.to_string(),
                    value: "off".into(),
                })
                .collect(),
            picked: 1,
            tracks: (0..tracks)
                .map(|k| CardTrack {
                    track: k,
                    name: format!("Track {}", k + 1),
                    color: Color::rgb(0x8db9fb),
                    cell: match k % 3 {
                        0 => CardCell::Clip("Beat".into()),
                        1 => CardCell::Stop,
                        _ => CardCell::Keep,
                    },
                })
                .collect(),
            first: 0,
            can: [true; 4],
        }
    }

    const R: Rect = Rect {
        x: 12,
        y: 140,
        w: 852,
        h: 532,
    };

    #[test]
    fn the_cards_parts_fit_it_and_answer_their_taps() {
        let mut c = card(8);
        let p = c.parts(R);
        assert!(p.card.x >= R.x && p.card.right() <= R.right());
        assert!(p.card.y >= R.y && p.card.bottom() <= R.bottom());
        assert!(p.page_back.is_none(), "eight tracks show at once");
        assert_eq!(p.rows.len(), LEFT_ROWS.len() + RIGHT_ROWS.len());
        for &(_, row) in &p.rows {
            assert!(row.bottom() < p.cells[0].1.y, "{row:?}");
        }
        for &(_, b) in &p.cells {
            assert!(b.right() <= p.card.right() - 24 && b.bottom() < p.done.y);
        }
        let at = |b: Rect| (b.x + 3, b.y + 3);
        let tap = |c: &mut SceneCard, (x, y): (i32, i32)| c.touch(Touch::Down, x, y, R);
        assert_eq!(tap(&mut c, at(p.done)), Some(EventKind::Toggle(SCENE_DONE)));
        assert_eq!(
            tap(&mut c, (R.x + 2, R.y + 2)),
            Some(EventKind::Toggle(SCENE_DONE)),
            "outside the card"
        );
        assert_eq!(
            tap(&mut c, at(p.name)),
            Some(EventKind::Toggle(SCENE_RENAME))
        );
        let (index, row) = p.rows[0];
        let (_, left, _, right) = SceneCard::row_parts(row);
        assert_eq!(
            tap(&mut c, at(left)),
            Some(EventKind::Step { index, delta: -1 })
        );
        assert_eq!(
            tap(&mut c, at(right)),
            Some(EventKind::Step { index, delta: 1 })
        );
        assert_eq!(
            tap(&mut c, at(row)),
            Some(EventKind::Toggle(SCENE_PICK + index))
        );
        // A clip's box is the clip's; an empty cell's turns over.
        assert_eq!(tap(&mut c, at(p.cells[0].1)), None);
        assert_eq!(
            tap(&mut c, at(p.cells[1].1)),
            Some(EventKind::Toggle(SCENE_KEEP + 1))
        );
        for (k, (ev, _)) in FOOT.iter().enumerate() {
            assert_eq!(tap(&mut c, at(p.foot[k])), Some(EventKind::Toggle(*ev)));
        }
        c.can[3] = false;
        assert_eq!(tap(&mut c, at(p.foot[3])), None);
    }

    #[test]
    fn more_than_eight_tracks_page_under_empty_cells() {
        let mut c = card(12);
        let p = c.parts(R);
        let (back, on) = (p.page_back.unwrap(), p.page_on.unwrap());
        assert_eq!(p.cells.len(), CELLS_SHOWN);
        assert!(back.right() < p.cells[0].1.x && p.cells[7].1.right() < on.x);
        assert_eq!(c.touch(Touch::Down, on.x + 3, on.y + 3, R), None);
        assert_eq!(c.first, CELLS_SHOWN);
        let p = c.parts(R);
        assert_eq!(p.cells.len(), 4);
        assert_eq!(p.cells[0].0, 8);
        c.touch(Touch::Down, back.x + 3, back.y + 3, R);
        assert_eq!(c.first, 0);
        // A new card for the same row keeps the page.
        c.first = 8;
        let mut again = card(12);
        again.keep_page(&c);
        assert_eq!(again.first, 8);
    }
}
