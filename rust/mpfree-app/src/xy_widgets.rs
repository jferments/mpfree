//! The XY pad's bar under the full-screen pad (Hold, Let go and Setup) and
//! the waiting banner (layout spec 4o and 6; the XY controller plan's
//! phase 2).
//!
//! - [`XyBar`]: a row of 44 px buttons under the pad. **Hold** (lit while
//!   on; on a ready-made layout it does nothing yet, and says why: Hold
//!   comes to them with the XY page), **Let go: Go back** or **Let go:
//!   Stay** (a tap switches it; on a ready-made layout a copy becomes
//!   yours), and **Setup**. A tap gives `Toggle(i)`, 0 to 2.
//! - [`XyBanner`]: after + across or + up, a 48 px banner in the hint
//!   line's place on every page (a float, `Ui::set_float`), outlined in
//!   the second accent: its chip ("XY · ACROSS"), what waits, what just
//!   happened (faint), then Cancel and Done. A tap gives `Tab(0)` (Cancel)
//!   or `Tab(1)` (Done).

use mpfree_ui::{Align, EventKind, FontId, PaintCx, Rect, Target, Touch, Widget};

/// The bar's widget name and its height in the full page's layout.
pub const XY_BAR: &str = "xy-bar";
pub const XY_BAR_H: i32 = 52;
/// The banner's widget name and its height.
pub const XY_BANNER: &str = "xy-banner";
pub const BANNER_H: i32 = 48;

/// The bar's buttons, their widths.
const BAR: [(&str, i32); 3] = [("Hold", 128), ("Let go", 220), ("Setup", 128)];
const BUTTON_H: i32 = 44;
const GAP: i32 = 10;

/// Hold, Let go and Setup under the full-screen pad.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct XyBar {
    /// Hold is on.
    pub hold: bool,
    /// A layout of yours plays (Hold works on it).
    pub yours: bool,
    /// Let go: Stay (else Go back).
    pub stay: bool,
}

impl XyBar {
    /// Button `i`'s place: from the left, under the pad.
    pub fn button(r: Rect, i: usize) -> Rect {
        let y = r.y + (r.h - BUTTON_H) / 2;
        let x = BAR[..i].iter().fold(r.x + 4, |x, (_, w)| x + w + GAP);
        Rect::new(x, y, BAR[i].1, BUTTON_H)
    }

    fn label(&self, i: usize) -> String {
        match i {
            1 if self.stay => "Let go: Stay".to_string(),
            1 => "Let go: Go back".to_string(),
            _ => BAR[i].0.to_string(),
        }
    }
}

impl Widget for XyBar {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.canvas.fill_rect(r, t.bg);
        for i in 0..BAR.len() {
            let b = XyBar::button(r, i);
            let lit = i == 0 && self.hold;
            let usable = i != 0 || self.yours;
            let fill = if lit { t.picked() } else { t.raised };
            cx.canvas.fill_round_rect(b, t.round(10), fill);
            let ink = if lit {
                t.on_picked()
            } else {
                t.button_ink(usable)
            };
            cx.text(FontId::Bold, 15.0, b, Align::Center, ink, &self.label(i));
        }
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        (0..BAR.len())
            .map(|i| Target::tap("xy-bar", i, self.label(i), XyBar::button(r, i)))
            .collect()
    }

    fn touch(&mut self, _finger: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if phase != Touch::Down {
            return None;
        }
        let i = (0..BAR.len()).find(|&i| XyBar::button(r, i).contains(x, y))?;
        Some(EventKind::Toggle(i))
    }
}

/// The waiting banner: settings SHIFT + a touch picks join a direction of
/// a layout until Done or Cancel.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct XyBanner {
    /// Up waits (else across).
    pub up: bool,
    /// "Bite on Squelch waits: hold SHIFT and touch settings on any page".
    pub line: String,
    /// What just happened, faint.
    pub just: String,
}

const BANNER_BUTTON: (i32, i32) = (104, 36);

impl XyBanner {
    /// Where the banner goes on a screen `w` × `h`: over the hint line, at
    /// the foot.
    pub fn place(w: i32, h: i32) -> Rect {
        Rect::new(6, h - BANNER_H - 2, w - 12, BANNER_H)
    }

    /// Cancel (0) and Done (1), at the right.
    pub fn button(r: Rect, i: usize) -> Rect {
        let (w, h) = BANNER_BUTTON;
        let right = r.right() - 8 - (1 - i as i32) * (w + 8);
        Rect::new(right - w, r.y + (r.h - h) / 2, w, h)
    }

    fn chip(&self) -> &'static str {
        if self.up {
            "XY · UP"
        } else {
            "XY · ACROSS"
        }
    }
}

impl Widget for XyBanner {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.canvas.fill_rect(r, t.bg);
        let card = r.inset_xy(0, 1);
        cx.canvas.fill_round_rect(card, t.round(12), t.panel());
        cx.canvas.stroke_round_rect(card, t.round(12), 2, t.accent2);
        let chip_w = cx.measure(FontId::Bold, 13.0, self.chip()) + 28;
        let chip = Rect::new(card.x + 10, card.y + (card.h - 30) / 2, chip_w, 30);
        cx.canvas.fill_round_rect(chip, 15, t.accent2);
        let ink = t.ink_on(t.accent2);
        cx.text(FontId::Bold, 13.0, chip, Align::Center, ink, self.chip());
        let cancel = XyBanner::button(r, 0);
        let text = Rect::new(
            chip.right() + 14,
            card.y + 4,
            cancel.x - chip.right() - 22,
            card.h - 8,
        );
        let (top, bottom) = text.take_top(text.h / 2 + 2);
        cx.text(FontId::Bold, 15.0, top, Align::Left, t.text, &self.line);
        cx.text(
            FontId::Sans,
            12.0,
            bottom,
            Align::Left,
            t.text_dim,
            &self.just,
        );
        cx.canvas.fill_round_rect(cancel, t.round(10), t.raised);
        cx.text(FontId::Bold, 15.0, cancel, Align::Center, t.text, "Cancel");
        let done = XyBanner::button(r, 1);
        cx.canvas.fill_round_rect(done, t.round(10), t.accent2);
        cx.text(FontId::Bold, 15.0, done, Align::Center, ink, "Done");
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        vec![
            Target::tap("cancel", 0, "Cancel", XyBanner::button(r, 0)),
            Target::tap("done", 1, "Done", XyBanner::button(r, 1)),
        ]
    }

    fn touch(&mut self, _finger: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if phase != Touch::Down {
            return None;
        }
        let i = (0..2).find(|&i| XyBanner::button(r, i).contains(x, y))?;
        Some(EventKind::Tab(i))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_ui::{Fonts, SoftCanvas, Theme, Ui};

    /// The bar's three buttons and the banner's two are a fingertip each
    /// and say what they are.
    #[test]
    fn the_bar_and_the_banner_answer_a_tap() {
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        ui.add(
            XY_BAR,
            XyBar {
                hold: true,
                yours: true,
                stay: true,
            },
        );
        ui.add(XY_BANNER, XyBanner::default());
        let label = || mpfree_ui::widgets::Label::new("", mpfree_ui::Tone::Dim, Align::Left);
        ui.add("x", label());
        ui.add("hint", label());
        // As on Play: the bar over the hint line, the page's margin round
        // them.
        ui.add_page("p", &format!("col(x:*, {XY_BAR}:{XY_BAR_H}, hint:26)"))
            .unwrap();
        ui.set_float("p", XY_BANNER, Some(XyBanner::place(1280, 800)))
            .unwrap();
        let mut c = SoftCanvas::new(1280, 800);
        ui.paint(&mut c);
        let bar = ui.rect_of(XY_BAR).unwrap();
        let b = ui.get::<XyBar>(XY_BAR).unwrap();
        let labels: Vec<_> = b.targets(bar).into_iter().map(|t| t.label).collect();
        assert_eq!(labels, ["Hold", "Let go: Stay", "Setup"]);
        for i in 0..3 {
            assert!(XyBar::button(bar, i).h >= 44);
        }
        let setup = XyBar::button(bar, 2);
        let ev = ui
            .touch(1, Touch::Down, setup.x + 5, setup.y + 5)
            .map(|e| e.kind);
        assert_eq!(ev, Some(EventKind::Toggle(2)));
        ui.touch(1, Touch::Up, setup.x + 5, setup.y + 5);
        let banner = XyBanner::place(1280, 800);
        assert!(banner.bottom() <= 800 && banner.h == BANNER_H);
        // It covers the hint line and the margins round it, not the bar.
        let hint = ui.rect_of("hint").unwrap();
        assert!(banner.y <= hint.y && banner.bottom() >= hint.bottom());
        assert!(banner.y >= bar.bottom(), "{banner:?} {bar:?}");
        let done = XyBanner::button(banner, 1);
        let ev = ui
            .touch(2, Touch::Down, done.x + 5, done.y + 5)
            .map(|e| e.kind);
        assert_eq!(ev, Some(EventKind::Tab(1)), "the banner is on top");
    }
}
