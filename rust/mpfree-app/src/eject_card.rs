//! Eject card (the SD card plan's B2c, board 7b, approved 2026-11-25): before
//! the SD card is closed for taking out, a sheet says what still uses it
//! and what happens to each: a take recording on it carries on on the USB
//! stick, a take playing from it goes quiet until the card is back. Cancel
//! leaves the card in; the main button ejects it. With nothing using the
//! card, it is ejected at once, without the sheet (gui.rs).
//!
//! The widget only draws what it is given and says which button was
//! tapped; the screen shows it as a page over the one it was opened from,
//! as the power card is.

use mpfree_ui::{Align, Color, EventKind, FontId, PaintCx, Rect, Shape, Target, Touch, Widget};

/// The widget's name, and the page that shows it.
pub const CARD: &str = "eject-card";
pub const PAGE: &str = "Eject card";
pub const LAYOUT: &str = "col(header:60, eject-card:*, hint:26)";
pub const HINT: &str = "Dial: move   ·   push: choose   ·   BROWSE: cancel, the card stays in";

/// Something still using the card.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Use {
    /// A take recording on the card, on this track (its name).
    Recording { track: String },
    /// A take playing from its file on the card: its track's name and the
    /// file's.
    Playing { track: String, file: String },
}

/// A button on the sheet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    /// The card stays in.
    Cancel,
    /// Recording moves to the stick (if any) and the card is ejected.
    Eject,
}

const SHEET_W: i32 = 740;
const PAD: i32 = 28;
const ROW_H: i32 = 68;
const BUTTON_H: i32 = 52;

/// The sheet.
#[derive(Debug, Default)]
pub struct EjectSheet {
    pub uses: Vec<Use>,
    /// The button the wheel's outline is on.
    pub picked: usize,
}

impl EjectSheet {
    /// Shows `uses`, the outline on the main button.
    pub fn show(&mut self, uses: Vec<Use>) {
        self.uses = uses;
        self.picked = 1;
    }

    pub fn close(&mut self) {
        self.uses.clear();
    }

    /// A take records on the card: Eject carries it on on the stick.
    pub fn recording(&self) -> bool {
        self.uses.iter().any(|u| matches!(u, Use::Recording { .. }))
    }

    /// The buttons, left to right; the last is the main one.
    pub fn buttons(&self) -> [(Choice, &'static str); 2] {
        [
            (Choice::Cancel, "Cancel"),
            (
                Choice::Eject,
                if self.recording() {
                    "Move recording to the stick and eject"
                } else {
                    "Eject"
                },
            ),
        ]
    }

    /// The wheel or - / + moved the outline.
    pub fn wheel(&mut self, d: i32) {
        self.picked = (self.picked as i32 + d).clamp(0, 1) as usize;
    }

    /// The words under the title.
    fn lead(&self) -> &'static str {
        if self.recording() {
            "These still use the card. Eject moves recording to the USB stick first."
        } else {
            "These still use the card."
        }
    }

    fn sheet(&self, r: Rect) -> Rect {
        let w = r.w.min(SHEET_W);
        let rows = self.uses.len() as i32;
        let foot = if self.recording() { 30 } else { 0 };
        let h =
            (PAD + 40 + 30 + 18 + rows * (ROW_H + 8) + 10 + foot + 18 + BUTTON_H + PAD).min(r.h);
        Rect::new(r.x + (r.w - w) / 2, r.y + (r.h - h).max(0) / 3, w, h)
    }

    /// Each button's rectangle, at the sheet's foot on the right.
    pub fn button_rects(&self, r: Rect) -> [Rect; 2] {
        let inner = self.sheet(r).inset(PAD);
        let y = inner.bottom() - BUTTON_H;
        let main_w = 56 + 10 * self.buttons()[1].1.len() as i32;
        let main = Rect::new(inner.right() - main_w, y, main_w, BUTTON_H);
        let cancel = Rect::new(main.x - 12 - 140, y, 140, BUTTON_H);
        [cancel, main]
    }

    /// Each use's row.
    fn row_rects(&self, r: Rect) -> Vec<Rect> {
        let inner = self.sheet(r).inset(PAD);
        let top = inner.y + 40 + 30 + 18;
        (0..self.uses.len() as i32)
            .map(|i| Rect::new(inner.x, top + i * (ROW_H + 8), inner.w, ROW_H))
            .collect()
    }
}

/// The eject sign: a triangle over a bar.
fn eject_sign(cx: &mut PaintCx, r: Rect, c: Color) {
    let (x, y) = (r.x as f32, r.y as f32);
    let mut s = Shape::new(r);
    s.fill_under(
        &[
            (x + 2.0, y + 12.0),
            (x + 11.0, y + 2.0),
            (x + 20.0, y + 12.0),
        ],
        y + 12.0,
        1.0,
    );
    cx.canvas.blend_shape(&s, c);
    cx.canvas
        .fill_round_rect(Rect::new(r.x + 2, r.y + 16, 18, 4), 1, c);
}

impl Widget for EjectSheet {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.canvas.fill_rect(r, t.bg);
        if self.uses.is_empty() {
            return;
        }
        let sheet = self.sheet(r);
        cx.panel(sheet);
        cx.canvas.stroke_round_rect(sheet, t.radius, 1, t.outline);
        let inner = sheet.inset(PAD);
        let (head, rest) = inner.take_top(40);
        eject_sign(cx, Rect::new(head.x, head.y + 9, 22, 22), t.text);
        cx.text(
            FontId::Bold,
            t.size_title,
            Rect::new(head.x + 34, head.y, head.w - 34, head.h),
            Align::Left,
            t.text,
            "Eject the SD card",
        );
        cx.text(
            FontId::Sans,
            t.size_small,
            rest.take_top(30).0,
            Align::Left,
            t.text_dim,
            self.lead(),
        );
        let rows = self.row_rects(r);
        for (u, rr) in self.uses.iter().zip(&rows) {
            cx.canvas.fill_round_rect(*rr, t.round(10), t.raised);
            let body = rr.inset_xy(18, 0);
            let (icon, body) = body.take_left(36);
            let circle = Rect::new(icon.x, icon.y + (icon.h - 36) / 2, 36, 36);
            cx.canvas.fill_round_rect(circle, 18, t.bg);
            let (title, what, then) = match u {
                Use::Recording { track } => {
                    let dot = Rect::new(circle.x + 11, circle.y + 11, 14, 14);
                    cx.canvas.fill_round_rect(dot, 7, t.bad);
                    (
                        track.clone(),
                        "Recording a take on the card",
                        "Carries on on the USB stick",
                    )
                }
                Use::Playing { track, file } => {
                    let (x, y) = (circle.x as f32, circle.y as f32);
                    // A play sign: the halves above and below its middle.
                    let mut s = Shape::new(circle);
                    s.fill_under(&[(x + 13.0, y + 11.0), (x + 26.0, y + 18.0)], y + 18.0, 1.0);
                    s.fill_under(&[(x + 13.0, y + 25.0), (x + 26.0, y + 18.0)], y + 18.0, 1.0);
                    cx.canvas.blend_shape(&s, t.accent);
                    (
                        format!("{track} \u{b7} {file}"),
                        "Playing from the card",
                        "Goes quiet until the card is back",
                    )
                }
            };
            let (words, right) = body.inset_xy(16, 0).take_right(250);
            let (top, bottom) = words.take_top(words.h / 2);
            cx.text(FontId::Bold, t.size_small, top, Align::Left, t.text, &title);
            cx.text(
                FontId::Sans,
                t.size_small,
                bottom,
                Align::Left,
                t.text_dim,
                what,
            );
            cx.text(
                FontId::Sans,
                t.size_small,
                right,
                Align::Right,
                t.text,
                then,
            );
        }
        let mut y = rows.last().map_or(inner.y + 88, |r| r.bottom() + 10);
        if self.recording() {
            cx.text(
                FontId::Sans,
                t.size_small,
                Rect::new(inner.x, y, inner.w, 30),
                Align::Left,
                t.text_dim,
                "Put the same card back in later: MPFree joins each take\u{2019}s two parts on the USB stick.",
            );
            y += 30;
        }
        let _ = y;
        for (i, ((choice, label), br)) in
            self.buttons().iter().zip(self.button_rects(r)).enumerate()
        {
            let main = *choice == Choice::Eject;
            cx.canvas
                .fill_round_rect(br, t.round(8), if main { t.accent } else { t.raised });
            if i == self.picked {
                cx.canvas.stroke_round_rect(br.inset(-4), 12, 3, t.text);
            }
            let ink = if main { t.on_accent } else { t.text };
            cx.text(FontId::Bold, t.size_small, br, Align::Center, ink, label);
        }
    }

    fn touch(&mut self, _f: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if phase != Touch::Down || self.uses.is_empty() {
            return None;
        }
        let i = self.button_rects(r).iter().position(|b| b.contains(x, y))?;
        self.picked = i;
        Some(EventKind::Tab(i))
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        if self.uses.is_empty() {
            return Vec::new();
        }
        self.buttons()
            .iter()
            .zip(self.button_rects(r))
            .enumerate()
            .map(|(i, ((_, label), rect))| Target::tap("button", i, *label, rect))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const R: Rect = Rect {
        x: 0,
        y: 60,
        w: 1280,
        h: 714,
    };

    /// The button the outline is on (what the wheel's push takes).
    fn chosen(s: &EjectSheet) -> Choice {
        s.buttons()[s.picked].0
    }

    fn sheet() -> EjectSheet {
        let mut s = EjectSheet::default();
        s.show(vec![
            Use::Recording {
                track: "Vocals".into(),
            },
            Use::Playing {
                track: "Keys".into(),
                file: "Rhodes chord.wav".into(),
            },
        ]);
        s
    }

    #[test]
    fn the_outline_starts_on_eject_and_the_wheel_moves_it() {
        let mut s = sheet();
        assert_eq!(chosen(&s), Choice::Eject);
        assert_eq!(s.buttons()[1].1, "Move recording to the stick and eject");
        s.wheel(-3);
        assert_eq!(chosen(&s), Choice::Cancel);
        s.wheel(5);
        assert_eq!(chosen(&s), Choice::Eject);
        // Only takes playing from it: Eject, and no word of the stick.
        s.show(vec![Use::Playing {
            track: "Keys".into(),
            file: "Rhodes chord.wav".into(),
        }]);
        assert_eq!(s.buttons()[1].1, "Eject");
        assert_eq!(s.lead(), "These still use the card.");
    }

    #[test]
    fn a_tap_on_a_button_picks_it_and_everything_fits() {
        let mut s = sheet();
        let [cancel, main] = s.button_rects(R);
        let sheet = s.sheet(R);
        for b in [cancel, main] {
            assert!(b.h >= 48 && sheet.inset(PAD - 1).contains(b.x, b.y));
            assert!(b.right() <= sheet.right() - PAD && b.bottom() <= sheet.bottom() - PAD);
        }
        assert!(cancel.right() < main.x, "apart");
        let rows = s.row_rects(R);
        assert_eq!(rows.len(), 2);
        assert!(rows[1].bottom() < cancel.y, "the rows above the buttons");
        let mid = |b: Rect| (b.x + b.w / 2, b.y + b.h / 2);
        let (x, y) = mid(cancel);
        assert_eq!(s.touch(1, Touch::Down, x, y, R), Some(EventKind::Tab(0)));
        assert_eq!(chosen(&s), Choice::Cancel);
        assert_eq!(s.touch(1, Touch::Down, 5, 70, R), None);
        let labels: Vec<String> = s.targets(R).into_iter().map(|t| t.label).collect();
        assert_eq!(labels, ["Cancel", "Move recording to the stick and eject"]);
        s.close();
        assert!(s.targets(R).is_empty());
    }
}
