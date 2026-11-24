//! **What the pads play** (sampler step 15e; the canvas "Pads and kits:
//! step 15e", board 3; Jay's "Build as drawn, with your picks" of 23 Nov
//! 2026 16:11Z): the panel a tap on the header's "Pads: Big Kit" chip
//! opens over the Synth tab's page, where the list of synths opens. It
//! shows the whole pad map at once: the base every bank plays, each bank's
//! own entry, the layers, the last change UNDO takes back, and what is
//! missing.
//!
//! - **The base, every bank**: what it plays ("Follow track" with the
//!   track's instrument, or a kit or a synth), **Follow track** while it
//!   isn't, and **Pick**, which opens the list of synths.
//! - **The banks**, A to H: "The base" faint for a bank with nothing of
//!   its own; its own kit or synth ("Big Kit", "From its bank C · you are
//!   on B"), with **Follow base**; a missing one in the warning colour
//!   ("Missing: kit 9", "Plays the base meanwhile") with **Replace** and
//!   **Remove**. The bank the pads are on is outlined. A tap on a row
//!   picks its sound (the list, for that bank).
//! - **The layers**, "LAYERS · 2 OF 4": each with where it plays (**Every
//!   bank**, or **Bank D**; a tap moves it between every bank and the bank
//!   the pads are on) and **Remove**; **+ Add a layer** while there is
//!   room.
//! - **UNDO**: the last change made here, which UNDO takes back; and a
//!   note on what is missing.
//!
//! Events: `Tab(DONE)`; `Tab(ADD_LAYER)`; `Cell { row, col }` for a row's
//! button `col` (or [`ROW`] for the row itself), the rows numbered
//! [`BASE`], then [`BANK`]` + b`, then [`LAYER`]` + l`.

use mpfree_ui::text::FontId;
use mpfree_ui::{Align, Color, EventKind, PaintCx, Rect, Target, Touch, Widget};

/// The widget's name in the app's page.
pub const PANEL: &str = "pad-map";
/// `Tab(DONE)`: the panel closes.
pub const DONE: usize = 0;
/// `Tab(ADD_LAYER)`: + Add a layer.
pub const ADD_LAYER: usize = 1;
/// The rows of `Cell { row, .. }`: the base, bank A to H, then the layers.
pub const BASE: usize = 0;
pub const BANK: usize = 1;
pub const LAYER: usize = 9;
/// `Cell { col: ROW, .. }`: the row itself, not one of its buttons.
pub const ROW: usize = 99;

/// The heading's height, a section label's, a row's and the gap.
const HEAD_H: i32 = 64;
const LABEL_H: i32 = 24;
const ROW_H: i32 = 50;
const GAP: i32 = 6;

/// How a row reads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Look {
    /// Something of its own.
    #[default]
    Own,
    /// Nothing of its own ("The base"): faint.
    Faint,
    /// Its instrument is gone: the warning colour.
    Missing,
}

/// A row: the base, a bank or a layer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MapRow {
    /// A bank's letter; None for the base and a layer (a swatch instead).
    pub letter: Option<char>,
    /// "Big Kit", "Follow track", "The base", "Missing: kit 9".
    pub what: String,
    /// "From its bank C · you are on B"; empty for none.
    pub sub: String,
    /// Its kit's colour (the base and layers), if a kit.
    pub colour: Option<Color>,
    pub look: Look,
    /// Its buttons, left to right ("Follow base"; "Replace", "Remove").
    pub buttons: Vec<String>,
}

/// The panel as the app fills it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PadMapPanel {
    pub base: MapRow,
    /// Bank A to H.
    pub banks: Vec<MapRow>,
    /// The bank the pads are on, outlined.
    pub current: usize,
    pub layers: Vec<MapRow>,
    /// The most layers, for "LAYERS · 2 OF 4" (the first counts).
    pub most: usize,
    /// "+ Add a layer" shows.
    pub can_add: bool,
    /// The last change here, for the UNDO box ("bank B got Big Kit, from
    /// its bank C"); None: nothing to take back.
    pub last: Option<String>,
    /// What is missing, for its note: how many, and what it says.
    pub missing: Option<(usize, String)>,
}

impl PadMapPanel {
    fn head(r: Rect) -> Rect {
        Rect::new(r.x + 16, r.y, r.w - 32, HEAD_H)
    }

    /// Done, at the heading's right.
    pub fn done(&self, r: Rect) -> Rect {
        let h = Self::head(r);
        Rect::new(h.right() - 120, h.y + 10, 120, h.h - 20)
    }

    /// The two columns under the heading.
    fn columns(r: Rect) -> (Rect, Rect) {
        let top = r.y + HEAD_H;
        let w = (r.w - 32 - 16) / 2;
        let h = r.bottom() - top - 16;
        (
            Rect::new(r.x + 16, top, w, h),
            Rect::new(r.x + 16 + w + 16, top, w, h),
        )
    }

    /// The base's row, under its label.
    pub fn base_row(&self, r: Rect) -> Rect {
        let (left, _) = Self::columns(r);
        Rect::new(left.x, left.y + LABEL_H, left.w, ROW_H)
    }

    /// Bank `b`'s row, under the base's and the banks' label.
    pub fn bank_row(&self, r: Rect, b: usize) -> Rect {
        let (left, _) = Self::columns(r);
        let top = left.y + LABEL_H + ROW_H + GAP + LABEL_H;
        let n = self.banks.len().max(1) as i32;
        let h = ROW_H.min((left.bottom() - top - GAP * (n - 1)) / n);
        Rect::new(left.x, top + b as i32 * (h + GAP), left.w, h)
    }

    /// Layer `l`'s row, under the layers' label.
    pub fn layer_row(&self, r: Rect, l: usize) -> Rect {
        let (_, right) = Self::columns(r);
        Rect::new(
            right.x,
            right.y + LABEL_H + l as i32 * (ROW_H + GAP),
            right.w,
            ROW_H,
        )
    }

    /// + Add a layer, after the layers, while there is room.
    pub fn add_row(&self, r: Rect) -> Option<Rect> {
        self.can_add.then(|| {
            let after = self.layer_row(r, self.layers.len());
            Rect::new(after.x, after.y, after.w, ROW_H - 8)
        })
    }

    /// The UNDO box and the missing note, under the layers.
    fn notes_top(&self, r: Rect) -> i32 {
        let after = self.layer_row(r, self.layers.len());
        after.y + if self.can_add { ROW_H - 8 + GAP } else { 0 } + 10
    }

    /// Row `row`'s buttons, right to left from its right edge, each sized
    /// to its words.
    pub fn buttons(&self, row: Rect, labels: &[String]) -> Vec<Rect> {
        let mut x = row.right() - 8;
        let mut out: Vec<Rect> = labels
            .iter()
            .rev()
            .map(|l| {
                let w = (l.chars().count() as i32 * 8 + 24).clamp(64, 120);
                x -= w;
                let b = Rect::new(x, row.y + 8, w, row.h - 16);
                x -= 6;
                b
            })
            .collect();
        out.reverse();
        out
    }

    /// Each row with its event row number and its rect.
    fn rows(&self, r: Rect) -> Vec<(usize, &MapRow, Rect)> {
        let mut out = vec![(BASE, &self.base, self.base_row(r))];
        for (b, row) in self.banks.iter().enumerate() {
            out.push((BANK + b, row, self.bank_row(r, b)));
        }
        for (l, row) in self.layers.iter().enumerate() {
            out.push((LAYER + l, row, self.layer_row(r, l)));
        }
        out
    }

    fn paint_row(&self, cx: &mut PaintCx, rect: Rect, row: &MapRow, current: bool) {
        let t = cx.theme.clone();
        cx.canvas
            .fill_round_rect(rect, 10, if current { t.raised } else { t.bg });
        if current {
            cx.canvas.stroke_round_rect(rect, 10, 2, t.accent);
        } else {
            cx.canvas.stroke_round_rect(rect, 10, 1, t.outline);
        }
        let inner = rect.inset_xy(10, 6);
        let mark = 26;
        let at = Rect::new(inner.x, inner.y + (inner.h - mark) / 2, mark, mark);
        match (row.letter, row.colour) {
            (Some(l), _) => {
                cx.canvas
                    .fill_round_rect(at, 6, if current { t.accent } else { t.raised });
                cx.text(
                    FontId::Bold,
                    t.size_small,
                    at,
                    Align::Center,
                    if current { t.on_accent } else { t.text },
                    &l.to_string(),
                );
            }
            (None, Some(c)) => cx
                .canvas
                .fill_round_rect(at.inset_xy(5, 5), 4, t.content(c)),
            (None, None) => cx
                .canvas
                .fill_round_rect(at.inset_xy(5, 5), 4, t.text_faint),
        }
        let buttons = self.buttons(rect, &row.buttons);
        let words_x = at.right() + 10;
        let words_r = buttons.first().map_or(inner.right(), |b| b.x - 8);
        let words = Rect::new(words_x, inner.y, (words_r - words_x).max(0), inner.h);
        let ink = match row.look {
            Look::Own => t.text,
            Look::Faint => t.text_dim,
            Look::Missing => t.warn,
        };
        if row.sub.is_empty() {
            cx.text(
                FontId::Bold,
                t.size_body - 1.0,
                words,
                Align::Left,
                ink,
                &row.what,
            );
        } else {
            let (top, bottom) = words.take_top(words.h / 2);
            cx.text(
                FontId::Bold,
                t.size_body - 1.0,
                top,
                Align::Left,
                ink,
                &row.what,
            );
            cx.text(
                FontId::Sans,
                t.size_small - 1.0,
                bottom,
                Align::Left,
                t.text_dim,
                &row.sub,
            );
        }
        for (b, label) in buttons.iter().zip(&row.buttons) {
            cx.canvas.fill_round_rect(*b, 8, t.raised);
            cx.canvas.stroke_round_rect(*b, 8, 1, t.outline);
            cx.text(FontId::Sans, t.size_small, *b, Align::Center, t.text, label);
        }
    }
}

impl Widget for PadMapPanel {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.panel(r);
        let h = Self::head(r);
        let done = self.done(r);
        let words = Rect::new(h.x, h.y + 10, done.x - h.x - 12, h.h - 20);
        let (top, bottom) = words.take_top(words.h * 3 / 5);
        cx.text(
            FontId::Bold,
            t.size_title + 1.0,
            top,
            Align::Left,
            t.text,
            "What the pads play",
        );
        cx.text(
            FontId::Sans,
            t.size_small,
            bottom,
            Align::Left,
            t.text_dim,
            "Only a pick, Follow, UNDO or opening a project changes it",
        );
        cx.canvas.fill_round_rect(done, 10, t.accent);
        cx.text(
            FontId::Bold,
            t.size_body - 1.0,
            done,
            Align::Center,
            t.on_accent,
            "Done",
        );
        let (left, right) = Self::columns(r);
        let label = |cx: &mut PaintCx, x: i32, y: i32, w: i32, s: &str| {
            cx.text(
                FontId::Bold,
                t.size_small - 1.0,
                Rect::new(x, y, w, LABEL_H),
                Align::Left,
                t.text_dim,
                s,
            );
        };
        label(cx, left.x, left.y, left.w, "BASE · EVERY BANK");
        let banks_label = self.base_row(r).bottom() + GAP;
        label(cx, left.x, banks_label, left.w, "BANKS");
        label(
            cx,
            right.x,
            right.y,
            right.w,
            &format!("LAYERS · {} OF {}", self.layers.len(), self.most),
        );
        for (n, row, rect) in self.rows(r) {
            let current = (BANK..LAYER).contains(&n) && n - BANK == self.current;
            self.paint_row(cx, rect, row, current);
        }
        if let Some(add) = self.add_row(r) {
            cx.canvas.stroke_round_rect(add, 10, 1, t.outline);
            cx.text(
                FontId::Sans,
                t.size_body - 1.0,
                add,
                Align::Center,
                t.text,
                "+ Add a layer",
            );
        }
        let mut y = self.notes_top(r);
        label(cx, right.x, y, right.w, "UNDO");
        y += LABEL_H;
        let undo = Rect::new(right.x, y, right.w, 64);
        cx.canvas.fill_round_rect(undo, 10, t.raised);
        let (a, b) = undo.inset_xy(12, 8).take_top(24);
        cx.text(
            FontId::Sans,
            t.size_small,
            a,
            Align::Left,
            t.text,
            &self.last.as_ref().map_or_else(
                || "Nothing to take back here yet.".to_string(),
                |l| format!("Last change here: {l}."),
            ),
        );
        cx.text(
            FontId::Sans,
            t.size_small - 1.0,
            b,
            Align::Left,
            t.text_dim,
            "UNDO takes it back. Each pick, Follow and Remove is one step.",
        );
        if let Some((n, said)) = &self.missing {
            let note = Rect::new(right.x, undo.bottom() + 10, right.w, 72);
            cx.canvas.stroke_round_rect(note, 10, 1, t.warn);
            let (a, b) = note.inset_xy(12, 8).take_top(22);
            cx.text(
                FontId::Bold,
                t.size_small,
                a,
                Align::Left,
                t.warn,
                &format!("{n} missing"),
            );
            cx.text(
                FontId::Sans,
                t.size_small - 1.0,
                b,
                Align::Left,
                t.text_dim,
                said,
            );
        }
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        let mut out = vec![Target::tap("done", DONE, "Done", self.done(r))];
        for (n, row, rect) in self.rows(r) {
            out.push(Target::tap("row", n, row.what.clone(), rect));
            for (k, (b, label)) in self
                .buttons(rect, &row.buttons)
                .into_iter()
                .zip(&row.buttons)
                .enumerate()
            {
                let id = match n {
                    BASE => "base",
                    n if n < LAYER => "bank",
                    _ => "layer",
                };
                let which = match n {
                    BASE => String::new(),
                    n if n < LAYER => format!(" {}", row.letter.unwrap_or('?')),
                    n => format!(" {}", n - LAYER + 1),
                };
                out.push(Target::tap(id, n * 100 + k, format!("{label}{which}"), b));
            }
        }
        if let Some(add) = self.add_row(r) {
            out.push(Target::tap("add-layer", ADD_LAYER, "+ Add a layer", add));
        }
        out
    }

    fn touch(&mut self, _f: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if phase != Touch::Down {
            return None;
        }
        if self.done(r).contains(x, y) {
            return Some(EventKind::Tab(DONE));
        }
        if self.add_row(r).is_some_and(|a| a.contains(x, y)) {
            return Some(EventKind::Tab(ADD_LAYER));
        }
        for (n, row, rect) in self.rows(r) {
            if !rect.contains(x, y) {
                continue;
            }
            let col = self
                .buttons(rect, &row.buttons)
                .iter()
                .position(|b| b.contains(x, y))
                .unwrap_or(ROW);
            return Some(EventKind::Cell { row: n, col });
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn panel() -> PadMapPanel {
        let row = |letter: Option<char>, what: &str, buttons: &[&str]| MapRow {
            letter,
            what: what.to_string(),
            sub: String::new(),
            colour: None,
            look: Look::Own,
            buttons: buttons.iter().map(|b| b.to_string()).collect(),
        };
        PadMapPanel {
            base: row(None, "Follow track", &["Pick"]),
            banks: (0..8)
                .map(|b| {
                    let l = (b'A' + b) as char;
                    match b {
                        1 => row(Some(l), "Big Kit", &["Follow base"]),
                        4 => row(Some(l), "Missing: kit 9", &["Replace", "Remove"]),
                        _ => row(Some(l), "The base", &[]),
                    }
                })
                .collect(),
            current: 1,
            layers: vec![
                row(None, "Pluck", &["Every bank", "Remove"]),
                row(None, "Lo-Fi", &["Bank D", "Remove"]),
            ],
            most: 4,
            can_add: true,
            last: Some("bank B got Big Kit, from its bank C".into()),
            missing: Some((1, "This project names kit 9".into())),
        }
    }

    /// Board 3: every row and button fits the panel and takes its tap,
    /// as its row and button; Done and + Add a layer take theirs.
    #[test]
    fn every_row_and_button_fits_and_takes_its_tap() {
        let mut p = panel();
        let r = Rect::new(244, 84, 884, 666);
        let rows: Vec<(usize, Rect, Vec<Rect>)> = p
            .rows(r)
            .into_iter()
            .map(|(n, row, rect)| (n, rect, p.buttons(rect, &row.buttons)))
            .collect();
        assert_eq!(rows.len(), 1 + 8 + 2);
        for (n, rect, buttons) in rows {
            assert!(
                rect.bottom() <= r.bottom() && rect.right() <= r.right(),
                "row {n} {rect:?}"
            );
            for (k, b) in buttons.iter().enumerate() {
                assert!(b.x > rect.x + 120, "row {n} button {k} {b:?}");
                assert_eq!(
                    p.touch(0, Touch::Down, b.x + 3, b.y + 3, r),
                    Some(EventKind::Cell { row: n, col: k })
                );
            }
            assert_eq!(
                p.touch(0, Touch::Down, rect.x + 40, rect.y + 10, r),
                Some(EventKind::Cell { row: n, col: ROW })
            );
        }
        let d = p.done(r);
        assert_eq!(
            p.touch(0, Touch::Down, d.x + 3, d.y + 3, r),
            Some(EventKind::Tab(DONE))
        );
        let a = p.add_row(r).unwrap();
        assert_eq!(
            p.touch(0, Touch::Down, a.x + 3, a.y + 3, r),
            Some(EventKind::Tab(ADD_LAYER))
        );
        assert!(p.notes_top(r) + LABEL_H + 64 + 10 + 72 <= r.bottom());
        let names: Vec<String> = p.targets(r).into_iter().map(|t| t.label).collect();
        for n in [
            "Follow base B",
            "Remove E",
            "Remove 2",
            "Pick",
            "+ Add a layer",
        ] {
            assert!(names.contains(&n.to_string()), "{n}: {names:?}");
        }
    }
}
