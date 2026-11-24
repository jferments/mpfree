//! Make a phrase (composing tools, step 3 of the phrases): the sheet that
//! makes a phrase of your own from a clip's notes or a MIDI file's, as the
//! owner's boards E and F draw it (`docs/dev/design/arpeggiator.md`, "Make
//! a phrase"). It opens in the clip card's place on Arrange (the card's
//! Phrase row) and in the Selected panel's place in the browser (a MIDI
//! file's Make a phrase).
//!
//! The sheet has a title and ×, where the notes came from, a picture of
//! the phrase made, what the notes were heard as, two rows (Kind and Home,
//! each a ‹ value › stepper: both read from the notes, either changed
//! before Keep), a few lines, the status (how to hear it, or why nothing
//! was made), what Keep does, and Back and Keep.
//!
//! Like the arp's other widgets it shows only what it is given
//! ([`SheetView`], which the app works out) and reports touches: a
//! stepper's ‹ or › gives `Step { index: row, delta }`, × and the buttons
//! `Toggle(SHEET_*)`.

use crate::maker::MakerRow;
use crate::phrase_tab::Sketch;
use mpfree_arp::Phrase;
use mpfree_ui::text::FontId;
use mpfree_ui::{Align, EventKind, PaintCx, Rect, Target, Touch, Widget};

/// The sheet's widget name, on both pages.
pub const MAKE_SHEET: &str = "make-phrase";

/// × in the corner: the same as Back.
pub const SHEET_CLOSE: usize = 0;
pub const SHEET_BACK: usize = 1;
pub const SHEET_KEEP: usize = 2;

/// What the sheet shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SheetView {
    /// "From Clip 1 · Track 1 · 2 bars", "From Bassline.mid · 2 bars".
    pub from: String,
    /// The phrase made, as Keep would keep it; None while none was made.
    pub phrase: Option<Phrase>,
    /// "Heard in C minor · on 1/16s".
    pub heard: String,
    /// Kind and Home.
    pub rows: Vec<MakerRow>,
    /// The row the wheel is on, which - and + change.
    pub row: usize,
    /// Lines under the rows.
    pub notes: Vec<String>,
    /// "Hold a pad: hear it from that note", or why nothing was made.
    pub status: String,
    /// The status says why nothing was made: in the warning colour.
    pub status_warn: bool,
    /// The name Keep gives it ("Clip 1").
    pub name: String,
    /// "Keep puts it on Yours as Clip 1."
    pub keep_line: String,
    pub can_keep: bool,
    /// The hint line while the sheet is open.
    pub hint: String,
}

/// A touch on the sheet, or the wheel or - / + while it is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SheetAction {
    /// Row `row`'s ‹ (-1) or › (+1): Kind or Home changes, and the phrase
    /// is made again.
    Step { row: usize, delta: i32 },
    /// The wheel: the row it is on.
    Wheel(i32),
    /// - or +: the wheel's row changes.
    Change(i32),
    /// Back, or ×: the sheet closes and nothing is kept.
    Back,
    /// The phrase joins the Phrase tab's Yours.
    Keep,
}

impl SheetAction {
    /// What a touch on the sheet's widget asks for.
    pub fn of(kind: &EventKind) -> Option<SheetAction> {
        match *kind {
            EventKind::Step { index, delta } => Some(SheetAction::Step { row: index, delta }),
            EventKind::Toggle(SHEET_CLOSE | SHEET_BACK) => Some(SheetAction::Back),
            EventKind::Toggle(SHEET_KEEP) => Some(SheetAction::Keep),
            _ => None,
        }
    }
}

/// The sheet ([`MAKE_SHEET`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MakeSheet {
    pub view: SheetView,
}

const PAD: i32 = 14;
const STEP_W: i32 = 140;
const STEP_H: i32 = 40;
const ROW_GAP: i32 = 10;
const LINE_H: i32 = 20;
const BUTTON_H: i32 = 46;
const PICTURE_H: i32 = 100;

/// Where the sheet's parts are.
#[derive(Clone, Debug, PartialEq)]
pub struct SheetGeo {
    pub close: Rect,
    pub from: Rect,
    pub picture: Rect,
    pub heard: Rect,
    /// Each row's stepper.
    pub steppers: Vec<Rect>,
    /// Where the lines under the rows start.
    pub notes_y: i32,
    pub status: Rect,
    pub keep_line: Rect,
    /// Back, Keep.
    pub buttons: [Rect; 2],
}

impl MakeSheet {
    pub fn new() -> MakeSheet {
        MakeSheet::default()
    }

    pub fn geo(&self, r: Rect) -> SheetGeo {
        let x = r.x + PAD;
        let w = (r.w - 2 * PAD).max(1);
        let picture = Rect::new(x, r.y + 72, w, PICTURE_H);
        let heard = Rect::new(x, picture.bottom() + 8, w, 22);
        let step_w = STEP_W.min(w);
        let mut y = heard.bottom() + 14;
        let steppers = self
            .view
            .rows
            .iter()
            .map(|_| {
                let s = Rect::new(x + w - step_w, y, step_w, STEP_H);
                y += STEP_H + ROW_GAP;
                s
            })
            .collect();
        let half = (w - 10) / 2;
        let b = r.bottom() - PAD - BUTTON_H;
        SheetGeo {
            close: Rect::new(r.right() - PAD - 40, r.y + 8, 40, 34),
            from: Rect::new(x, r.y + 42, w, 22),
            picture,
            heard,
            steppers,
            notes_y: y + 2,
            status: Rect::new(x, b - 104, w, 22),
            keep_line: Rect::new(x, b - 78, w, 22),
            buttons: [
                Rect::new(x, b, half, BUTTON_H),
                Rect::new(x + half + 10, b, w - half - 10, BUTTON_H),
            ],
        }
    }

    /// Whether button `k` (Back, Keep) answers.
    fn live(&self, k: usize) -> bool {
        k == 0 || self.view.can_keep
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

impl Widget for MakeSheet {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        let v = &self.view;
        let g = self.geo(r);
        cx.panel(r);
        let x = r.x + PAD;
        cx.text(
            FontId::Bold,
            t.size_small,
            Rect::new(x, r.y + 8, g.close.x - x - 8, 34),
            Align::Left,
            t.text,
            "MAKE A PHRASE",
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
        cx.text(
            FontId::Sans,
            t.size_small - 1.0,
            g.from,
            Align::Left,
            t.text_dim,
            &v.from,
        );
        match &v.phrase {
            Some(p) => Sketch::of(p).paint_large(cx, g.picture, t.accent, None),
            None => cx.canvas.fill_round_rect(g.picture, t.round(10), t.bg),
        }
        cx.text(
            FontId::Sans,
            t.size_small - 1.0,
            g.heard,
            Align::Left,
            t.text_dim,
            &v.heard,
        );
        for (k, (row, s)) in v.rows.iter().zip(&g.steppers).enumerate() {
            let label = Rect::new(x + 2, s.y, s.x - x - 6, s.h);
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
            for (side, mark) in [(Align::Left, "‹"), (Align::Right, "›")] {
                cx.text(FontId::Bold, t.size_body, ends, side, t.text_dim, mark);
            }
            let ink = if row.accent {
                t.readable(t.accent, t.raised)
            } else {
                t.text
            };
            cx.text(
                FontId::Bold,
                t.size_small + 1.0,
                s.inset_xy(24, 0),
                Align::Center,
                ink,
                &row.value,
            );
        }
        let mut y = g.notes_y;
        for line in &v.notes {
            if y + LINE_H > g.status.y - 4 {
                break;
            }
            cx.text(
                FontId::Sans,
                t.size_small - 1.0,
                Rect::new(x + 2, y, r.w - 2 * PAD - 2, LINE_H),
                Align::Left,
                t.text_dim,
                line,
            );
            y += LINE_H;
        }
        let ink = if v.status_warn {
            t.readable(t.warn, t.surface)
        } else {
            t.readable(t.good, t.surface)
        };
        cx.text(
            FontId::Bold,
            t.size_small - 1.0,
            g.status,
            Align::Left,
            ink,
            &v.status,
        );
        cx.text(
            FontId::Sans,
            t.size_small - 1.0,
            g.keep_line,
            Align::Left,
            t.text_dim,
            &v.keep_line,
        );
        for (k, (b, label)) in g.buttons.iter().zip(["Back", "Keep"]).enumerate() {
            button(cx, *b, label, k == 1, self.live(k));
        }
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        let g = self.geo(r);
        let mut out = vec![Target::tap("sheet-close", SHEET_CLOSE, "Close", g.close)];
        for (k, (row, s)) in self.view.rows.iter().zip(&g.steppers).enumerate() {
            let (less, more) = s.take_left(s.w / 2);
            out.push(Target::tap(
                "sheet-row-less",
                k,
                format!("{} ‹", row.name),
                less,
            ));
            out.push(Target::tap(
                "sheet-row-more",
                k,
                format!("{} ›", row.name),
                more,
            ));
        }
        out.push(Target::tap("sheet-back", SHEET_BACK, "Back", g.buttons[0]));
        out.push(
            Target::tap("sheet-keep", SHEET_KEEP, "Keep", g.buttons[1]).disabled_if(!self.live(1)),
        );
        out
    }

    fn touch(&mut self, _f: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if phase != Touch::Down {
            return None;
        }
        let g = self.geo(r);
        if g.close.contains(x, y) {
            return Some(EventKind::Toggle(SHEET_CLOSE));
        }
        if let Some(k) = g.steppers.iter().position(|s| s.contains(x, y)) {
            let s = g.steppers[k];
            let delta = if x < s.x + s.w / 2 { -1 } else { 1 };
            return Some(EventKind::Step { index: k, delta });
        }
        let index = [SHEET_BACK, SHEET_KEEP];
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

    fn sheet(can_keep: bool) -> MakeSheet {
        MakeSheet {
            view: SheetView {
                from: "From Clip 1 · Track 1 · 2 bars".into(),
                phrase: Some(Phrase::default()),
                heard: "Heard in C minor · on 1/16s".into(),
                rows: [("Kind", "Bass"), ("Home", "C2")]
                    .iter()
                    .map(|(n, v)| MakerRow {
                        name: n.to_string(),
                        value: v.to_string(),
                        ..MakerRow::default()
                    })
                    .collect(),
                can_keep,
                ..SheetView::default()
            },
        }
    }

    fn at(r: Rect) -> (i32, i32) {
        (r.x + r.w / 2, r.y + r.h / 2)
    }

    /// The column beside the grid: its steppers' halves step their rows,
    /// × and Back close, Keep keeps, and Keep with nothing made answers
    /// nothing.
    #[test]
    fn the_steppers_and_buttons_answer_where_they_are() {
        let r = Rect::new(878, 140, 250, 608);
        let mut s = sheet(true);
        let g = s.geo(r);
        assert_eq!(g.steppers.len(), 2);
        assert!(g.steppers[1].bottom() < g.status.y, "rows above the status");
        assert!(g.keep_line.bottom() <= g.buttons[0].y);
        assert!(g.buttons[1].right() <= r.right() - PAD);
        let tap = |s: &mut MakeSheet, (x, y): (i32, i32)| s.touch(0, Touch::Down, x, y, r);
        let home = g.steppers[1];
        assert_eq!(
            tap(&mut s, (home.x + 6, home.y + 10)),
            Some(EventKind::Step {
                index: 1,
                delta: -1
            })
        );
        assert_eq!(
            tap(&mut s, (home.right() - 6, home.y + 10)),
            Some(EventKind::Step { index: 1, delta: 1 })
        );
        assert_eq!(
            tap(&mut s, at(g.close)),
            Some(EventKind::Toggle(SHEET_CLOSE))
        );
        assert_eq!(
            tap(&mut s, at(g.buttons[0])),
            Some(EventKind::Toggle(SHEET_BACK))
        );
        assert_eq!(
            tap(&mut s, at(g.buttons[1])),
            Some(EventKind::Toggle(SHEET_KEEP))
        );
        let mut none = sheet(false);
        assert_eq!(tap(&mut none, at(g.buttons[1])), None);
        let keep = none
            .targets(r)
            .into_iter()
            .find(|t| t.id == "sheet-keep")
            .unwrap();
        assert!(!keep.enabled);
    }
}
