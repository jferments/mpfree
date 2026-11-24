//! New idea and More like this on the Phrase tab (composing tools P2): one
//! panel in the list's place while either is open, as the owner's boards B
//! and C draw it (`docs/dev/design/arpeggiator.md`, "New idea and More like
//! this").
//!
//! The panel has a title and ×, More like this's phrase it makes more from,
//! the rows (each a ‹ value › stepper), a few lines on what Keep and Back
//! do, the try's status, and four buttons: Try again, ‹ Try N, Back and
//! Keep. The try itself shows in the detail panel beside it
//! ([`crate::phrase_tab::plays`]), and a held pad plays it.
//!
//! Like the arp's other widgets it shows only what the panel put in it
//! ([`MakerView`], which the app works out) and reports touches: a
//! stepper's ‹ or › gives `Step { index: row, delta }`, × and the buttons
//! `Toggle(MAKER_*)`.

use crate::phrase_tab::Sketch;
use mpfree_arp::Phrase;
use mpfree_ui::text::FontId;
use mpfree_ui::{Align, EventKind, PaintCx, Rect, Target, Touch, Widget};

/// The panel's widget name: in the list's place while it is open.
pub const MAKER: &str = "arp-phrase-maker";

/// × in the corner: the same as Back.
pub const MAKER_CLOSE: usize = 0;
pub const MAKER_TRY: usize = 1;
/// ‹ Try N: the try before.
pub const MAKER_STEP: usize = 2;
pub const MAKER_BACK: usize = 3;
pub const MAKER_KEEP: usize = 4;

/// One row: its name, its value between ‹ and ›, and a line under it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MakerRow {
    pub name: String,
    pub value: String,
    /// The value in the accent colour (More like this's Change).
    pub accent: bool,
    /// A line under the stepper ("Its rhythm, with new notes"); empty for
    /// none.
    pub note: String,
}

/// What the panel shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MakerView {
    /// "New idea · Bass", or "More like this".
    pub title: String,
    /// More like this: the phrase it makes more from.
    pub from: Option<(String, Phrase)>,
    pub rows: Vec<MakerRow>,
    /// The row the wheel is on, which - and + change.
    pub row: usize,
    /// Lines under the rows.
    pub notes: Vec<String>,
    /// "Try 3 · hold a pad: it plays from that note", or why there is
    /// nothing new.
    pub status: String,
    /// The status says why nothing was made: in the warning colour.
    pub status_warn: bool,
    /// Try again answers: faint for a change that comes out one way.
    pub can_try: bool,
    /// "‹ Try 2", and whether it answers (faint with one try so far).
    pub step: (String, bool),
    pub can_keep: bool,
    /// The try shown, under the name Keep gives it: what the detail panel
    /// shows and a held pad plays. None while there is none.
    pub shown: Option<(String, Phrase)>,
    /// The detail panel's heading while a try shows: "Try 3 · Back keeps
    /// Night drive".
    pub head: String,
}

/// The panel ([`MAKER`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PhraseMaker {
    pub view: MakerView,
}

const PAD: i32 = 16;
const STEP_W: i32 = 232;
const STEP_H: i32 = 40;
const ROW_GAP: i32 = 12;
const NOTE_H: i32 = 24;
const LINE_H: i32 = 20;
const BUTTON_H: i32 = 46;
const SKETCH_W: i32 = 118;
const SKETCH_H: i32 = 30;

/// Where the panel's parts are.
#[derive(Clone, Debug, PartialEq)]
pub struct MakerGeo {
    pub close: Rect,
    /// The From line (More like this).
    pub from: Option<Rect>,
    /// Each row's stepper.
    pub steppers: Vec<Rect>,
    /// Where the lines under the rows start.
    pub notes_y: i32,
    pub status: Rect,
    /// Try again, ‹ Try N, Back, Keep.
    pub buttons: [Rect; 4],
}

impl PhraseMaker {
    pub fn geo(&self, r: Rect) -> MakerGeo {
        let v = &self.view;
        let x = r.x + PAD;
        let w = (r.w - 2 * PAD).max(1);
        let mut y = r.y + 58;
        let from = v.from.as_ref().map(|_| {
            let f = Rect::new(x, y, w, STEP_H);
            y += STEP_H + ROW_GAP;
            f
        });
        let step_w = STEP_W.min(w);
        let steppers = v
            .rows
            .iter()
            .map(|row| {
                let s = Rect::new(x + w - step_w, y, step_w, STEP_H);
                y += STEP_H + ROW_GAP;
                if !row.note.is_empty() {
                    y += NOTE_H - ROW_GAP / 2;
                }
                s
            })
            .collect();
        let half = (w - 10) / 2;
        let b2 = r.bottom() - PAD - BUTTON_H;
        let b1 = b2 - 10 - BUTTON_H;
        MakerGeo {
            close: Rect::new(r.right() - PAD - 40, r.y + 8, 40, 34),
            from,
            steppers,
            notes_y: y + 4,
            status: Rect::new(x, b1 - 30, w, 22),
            buttons: [
                Rect::new(x, b1, half, BUTTON_H),
                Rect::new(x + half + 10, b1, w - half - 10, BUTTON_H),
                Rect::new(x, b2, half, BUTTON_H),
                Rect::new(x + half + 10, b2, w - half - 10, BUTTON_H),
            ],
        }
    }

    /// Whether button `k` (Try again, ‹ Try N, Back, Keep) answers.
    fn live(&self, k: usize) -> bool {
        let v = &self.view;
        [v.can_try, v.step.1, true, v.can_keep][k]
    }
}

fn button(cx: &mut PaintCx, b: Rect, label: &str, lit: bool, live: bool) {
    let t = cx.theme.clone();
    let fill = if lit && live { t.accent } else { t.raised };
    cx.canvas.fill_round_rect(b, t.round(10), fill);
    let ink = match (lit, live) {
        (_, false) => t.text_faint,
        (true, true) => t.ink_on(t.accent),
        (false, true) => t.text,
    };
    cx.text(
        FontId::Bold,
        t.size_body - 1.0,
        b,
        Align::Center,
        ink,
        label,
    );
}

impl Widget for PhraseMaker {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        let v = &self.view;
        let g = self.geo(r);
        cx.panel(r);
        let x = r.x + PAD;
        cx.text(
            FontId::Bold,
            t.size_small,
            Rect::new(x, r.y + 10, g.close.x - x - 8, 30),
            Align::Left,
            t.text,
            &v.title.to_uppercase(),
        );
        cx.canvas.fill_round_rect(g.close, t.round(10), t.raised);
        cx.text(
            FontId::Sans,
            t.size_body + 2.0,
            g.close,
            Align::Center,
            t.text,
            "×",
        );
        cx.canvas
            .fill_rect(Rect::new(r.x + 12, r.y + 47, r.w - 24, 1), t.outline);
        if let (Some(f), Some((name, p))) = (g.from, &v.from) {
            cx.text(FontId::Sans, t.size_body, f, Align::Left, t.text, "From");
            let sketch = Rect::new(
                f.right() - SKETCH_W,
                f.y + (f.h - SKETCH_H) / 2,
                SKETCH_W,
                SKETCH_H,
            );
            let words = Rect::new(f.x + 90, f.y, (sketch.x - 12 - f.x - 90).max(1), f.h);
            cx.text(FontId::Bold, t.size_body, words, Align::Right, t.text, name);
            Sketch::of(p).paint_small(cx, sketch, t.text_dim);
        }
        for (k, (row, s)) in v.rows.iter().zip(&g.steppers).enumerate() {
            let label = Rect::new(x + 4, s.y, s.x - x - 8, s.h);
            if k == v.row {
                cx.canvas
                    .fill_round_rect(Rect::new(x - 6, s.y + 8, 3, s.h - 16), 2, t.accent);
            }
            cx.text(
                if k == v.row {
                    FontId::Bold
                } else {
                    FontId::Sans
                },
                t.size_body,
                label,
                Align::Left,
                t.text,
                &row.name,
            );
            cx.canvas.fill_round_rect(*s, t.round(10), t.raised);
            let ends = s.inset_xy(12, 0);
            cx.text(
                FontId::Bold,
                t.size_body,
                ends,
                Align::Left,
                t.text_dim,
                "‹",
            );
            cx.text(
                FontId::Bold,
                t.size_body,
                ends,
                Align::Right,
                t.text_dim,
                "›",
            );
            let ink = if row.accent {
                t.readable(t.accent, t.raised)
            } else {
                t.text
            };
            cx.text(
                FontId::Bold,
                t.size_small + 1.0,
                s.inset_xy(26, 0),
                Align::Center,
                ink,
                &row.value,
            );
            if !row.note.is_empty() {
                cx.text(
                    FontId::Sans,
                    t.size_small - 1.0,
                    Rect::new(s.x, s.bottom() + 2, s.w, NOTE_H - 4),
                    Align::Left,
                    t.text_dim,
                    &row.note,
                );
            }
        }
        let mut y = g.notes_y;
        for line in &v.notes {
            if y + LINE_H > g.status.y - 4 {
                break;
            }
            cx.text(
                FontId::Sans,
                t.size_small - 1.0,
                Rect::new(x + 4, y, r.w - 2 * PAD - 4, LINE_H),
                Align::Left,
                t.text_dim,
                line,
            );
            y += LINE_H + 2;
        }
        let ink = if v.status_warn {
            t.readable(t.warn, t.surface)
        } else {
            t.readable(t.good, t.surface)
        };
        cx.text(
            FontId::Bold,
            t.size_small,
            g.status,
            Align::Left,
            ink,
            &v.status,
        );
        let labels = ["Try again", v.step.0.as_str(), "Back", "Keep"];
        for (k, b) in g.buttons.iter().enumerate() {
            button(cx, *b, labels[k], k == 3, self.live(k));
        }
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        let g = self.geo(r);
        let v = &self.view;
        let mut out = vec![Target::tap("maker-close", MAKER_CLOSE, "Close", g.close)];
        for (k, (row, s)) in v.rows.iter().zip(&g.steppers).enumerate() {
            let (less, more) = s.take_left(s.w / 2);
            out.push(Target::tap(
                "maker-row-less",
                k,
                format!("{} ‹", row.name),
                less,
            ));
            out.push(Target::tap(
                "maker-row-more",
                k,
                format!("{} ›", row.name),
                more,
            ));
        }
        let ids = [
            ("maker-try-again", MAKER_TRY, "Try again".to_string()),
            ("maker-step", MAKER_STEP, v.step.0.clone()),
            ("maker-back", MAKER_BACK, "Back".to_string()),
            ("maker-keep", MAKER_KEEP, "Keep".to_string()),
        ];
        for (k, (id, index, label)) in ids.into_iter().enumerate() {
            out.push(Target::tap(id, index, label, g.buttons[k]).disabled_if(!self.live(k)));
        }
        out
    }

    fn touch(&mut self, _f: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if phase != Touch::Down {
            return None;
        }
        let g = self.geo(r);
        if g.close.contains(x, y) {
            return Some(EventKind::Toggle(MAKER_CLOSE));
        }
        if let Some(k) = g.steppers.iter().position(|s| s.contains(x, y)) {
            let s = g.steppers[k];
            let delta = if x < s.x + s.w / 2 { -1 } else { 1 };
            return Some(EventKind::Step { index: k, delta });
        }
        let index = [MAKER_TRY, MAKER_STEP, MAKER_BACK, MAKER_KEEP];
        g.buttons
            .iter()
            .position(|b| b.contains(x, y))
            .filter(|&k| self.live(k))
            .map(|k| EventKind::Toggle(index[k]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view() -> MakerView {
        MakerView {
            title: "New idea · Bass".into(),
            rows: ["Kind", "Style", "Bars", "Activity"]
                .iter()
                .map(|n| MakerRow {
                    name: n.to_string(),
                    value: "x".into(),
                    ..MakerRow::default()
                })
                .collect(),
            step: ("‹ Try 1".into(), true),
            can_try: true,
            can_keep: true,
            ..MakerView::default()
        }
    }

    fn at(r: Rect) -> (i32, i32) {
        (r.x + r.w / 2, r.y + r.h / 2)
    }

    #[test]
    fn the_panels_steppers_and_buttons_answer_where_they_are() {
        let r = Rect::new(190, 244, 450, 506);
        let mut m = PhraseMaker { view: view() };
        let g = m.geo(r);
        assert_eq!(g.steppers.len(), 4);
        assert!(g.buttons[3].bottom() <= r.bottom() - 10);
        assert!(g.steppers[3].bottom() < g.status.y);
        let s = g.steppers[2];
        assert_eq!(
            m.touch(1, Touch::Down, s.x + 4, s.y + 4, r),
            Some(EventKind::Step {
                index: 2,
                delta: -1
            })
        );
        assert_eq!(
            m.touch(1, Touch::Down, s.right() - 4, s.y + 4, r),
            Some(EventKind::Step { index: 2, delta: 1 })
        );
        for (k, want) in [MAKER_TRY, MAKER_STEP, MAKER_BACK, MAKER_KEEP]
            .into_iter()
            .enumerate()
        {
            let (x, y) = at(g.buttons[k]);
            assert_eq!(
                m.touch(1, Touch::Down, x, y, r),
                Some(EventKind::Toggle(want))
            );
        }
        let (x, y) = at(g.close);
        assert_eq!(
            m.touch(1, Touch::Down, x, y, r),
            Some(EventKind::Toggle(MAKER_CLOSE))
        );
        // Faint buttons answer nothing, and say so to the sweep.
        m.view.can_try = false;
        m.view.step.1 = false;
        m.view.can_keep = false;
        for k in [0, 1, 3] {
            let (x, y) = at(g.buttons[k]);
            assert_eq!(m.touch(1, Touch::Down, x, y, r), None);
        }
        let disabled: Vec<&str> = m
            .targets(r)
            .iter()
            .filter(|t| !t.enabled)
            .map(|t| t.id)
            .collect();
        assert_eq!(disabled, ["maker-try-again", "maker-step", "maker-keep"]);
    }

    #[test]
    fn more_like_this_has_its_from_line_over_the_rows() {
        let r = Rect::new(190, 244, 450, 506);
        let mut v = view();
        v.from = Some(("Night drive".into(), Phrase::default()));
        v.rows.truncate(2);
        v.rows[0].note = "Its rhythm, with new notes".into();
        let m = PhraseMaker { view: v };
        let g = m.geo(r);
        let from = g.from.expect("a From line");
        assert!(from.bottom() < g.steppers[0].y);
        // The note under the first row leaves room before the second.
        assert!(g.steppers[1].y >= g.steppers[0].bottom() + NOTE_H);
    }
}
