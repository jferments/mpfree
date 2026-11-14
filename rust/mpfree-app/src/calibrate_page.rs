//! The Calibrate touch page: M16's four-corner check on screen.
//!
//! The owner taps a target in each corner in turn (top-left, top-right,
//! bottom-right, bottom-left, as they face the MPC). The page turns each lift
//! position back into the raw reading through the built-in mapping
//! (`AxisMap::unmap`), so the calibrator sees what the panel really reported,
//! and then says whether the panel matches the built-in orientation.
//!
//! This version reports and logs only. Nothing is saved and the mapping in use
//! is not changed. It opens from Prefs > Touch screen ("Calibrate touch") and closes with its
//! Exit button or any page change from the app (MENU). Its buttons sit in the
//! middle of the screen, where corner taps never land, and the middle stays
//! the middle under every orientation.

use crate::touchlab_page::TouchPage;
use mpfree_touch::corners::{
    orientation_line, Calibration, CalibrationError, Corner, CornerCalibrator, TapOutcome,
};
use mpfree_touch::profile::MPC_ONE;
use mpfree_ui::{Align, Color, FontId, PaintCx, Rect, Tone, Touch, Widget};

/// How far each target's centre sits from the screen's edges.
const TARGET_INSET: i32 = 70;
const TARGET_RADIUS: i32 = 36;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Button {
    Again,
    Exit,
}

impl Button {
    fn label(self) -> &'static str {
        match self {
            Button::Again => "Start again",
            Button::Exit => "Exit",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Finger {
    Button(Button),
    Tap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Note {
    Ask,
    TooCentral,
    SameAs(Corner),
}

pub struct CalibrateView {
    cal: CornerCalibrator,
    /// Where each recorded tap landed on screen, for drawing.
    taps: Vec<(i32, i32)>,
    note: Note,
    result: Option<Result<Calibration, CalibrationError>>,
    fingers: Vec<(u64, Finger)>,
    lines: Vec<String>,
    exit: bool,
}

impl CalibrateView {
    pub fn new() -> CalibrateView {
        CalibrateView {
            cal: CornerCalibrator::new(MPC_ONE),
            taps: Vec::new(),
            note: Note::Ask,
            result: None,
            fingers: Vec::new(),
            lines: Vec::new(),
            exit: false,
        }
    }

    fn restart(&mut self) {
        self.cal.reset();
        self.taps.clear();
        self.note = Note::Ask;
        self.result = None;
    }

    /// The centre of a corner's target.
    fn target(r: Rect, c: Corner) -> (i32, i32) {
        let (left, top) = (r.x + TARGET_INSET, r.y + TARGET_INSET);
        let (right, bottom) = (r.right() - 1 - TARGET_INSET, r.bottom() - 1 - TARGET_INSET);
        match c {
            Corner::TopLeft => (left, top),
            Corner::TopRight => (right, top),
            Corner::BottomRight => (right, bottom),
            Corner::BottomLeft => (left, bottom),
        }
    }

    fn buttons(&self, r: Rect) -> Vec<(Button, Rect)> {
        let (cx, cy) = (r.x + r.w / 2, r.y + r.h / 2);
        vec![
            (Button::Again, Rect::new(cx - 190, cy + 40, 180, 64)),
            (Button::Exit, Rect::new(cx + 10, cy + 40, 180, 64)),
        ]
    }

    fn tap(&mut self, x: i32, y: i32, r: Rect) {
        let Ok(map) = MPC_ONE.axis_map(r.w as u32, r.h as u32) else {
            return;
        };
        let Some(corner) = self.cal.next_corner() else {
            return;
        };
        let (raw_x, raw_y) = map.unmap((x - r.x) as f32, (y - r.y) as f32);
        let outcome = self.cal.tap(raw_x, raw_y);
        let word = match outcome {
            TapOutcome::Next(_) | TapOutcome::Done => "ok".to_string(),
            TapOutcome::TooCentral => "too_central".to_string(),
            TapOutcome::SameAsEarlier(c) => format!("same_as_{}", c.label()),
            TapOutcome::Ignored => "ignored".to_string(),
        };
        self.lines.push(format!(
            "touch-calibrate-tap: corner={} x={x} y={y} raw_x={raw_x} raw_y={raw_y} outcome={word}",
            corner.label()
        ));
        match outcome {
            TapOutcome::Next(_) => {
                self.taps.push((x, y));
                self.note = Note::Ask;
            }
            TapOutcome::Done => {
                self.taps.push((x, y));
                self.note = Note::Ask;
                let result = self.cal.result();
                match &result {
                    Ok(c) => {
                        let word = if c.matches_profile {
                            "match"
                        } else {
                            "differs"
                        };
                        self.lines.push(format!("touch-calibrate: result={word}"));
                        self.lines.push(orientation_line(c.orientation));
                    }
                    Err(CalibrationError::NoFit(n)) => self
                        .lines
                        .push(format!("touch-calibrate: result=no_fit fits={n}")),
                    Err(CalibrationError::Incomplete) => {}
                }
                self.result = Some(result);
            }
            TapOutcome::TooCentral => self.note = Note::TooCentral,
            TapOutcome::SameAsEarlier(c) => self.note = Note::SameAs(c),
            TapOutcome::Ignored => {}
        }
    }

    fn message(&self) -> (String, Tone) {
        match (&self.result, self.cal.next_corner()) {
            (Some(Ok(c)), _) if c.matches_profile => (
                "Touch matches the built-in mapping. Nothing needs changing.".to_string(),
                Tone::Good,
            ),
            (Some(Ok(c)), _) => {
                let o = c.orientation;
                (format!("This panel is mounted differently (swap_xy={} flip_x={} flip_y={}). It is logged, not applied.",
                         o.swap_xy as u8, o.flip_x as u8, o.flip_y as u8), Tone::Warn)
            }
            (Some(Err(_)), _) => (
                "The taps fit no mapping: a corner was probably tapped out of order.".to_string(),
                Tone::Bad,
            ),
            (None, Some(next)) => match self.note {
                Note::Ask => (
                    format!("Tap the {} target, as you face the MPC.", next.label()),
                    Tone::Accent,
                ),
                Note::TooCentral => (
                    format!(
                        "That was too near the middle. Tap the {} target.",
                        next.label()
                    ),
                    Tone::Warn,
                ),
                Note::SameAs(c) => (
                    format!(
                        "That landed in the {} corner, which is done. Tap the {} target.",
                        c.label(),
                        next.label()
                    ),
                    Tone::Warn,
                ),
            },
            (None, None) => (String::new(), Tone::Normal),
        }
    }
}

impl Default for CalibrateView {
    fn default() -> Self {
        CalibrateView::new()
    }
}

fn circle(cx: &mut PaintCx, x: i32, y: i32, radius: i32, c: Color) {
    cx.canvas.fill_round_rect(
        Rect::new(x - radius, y - radius, 2 * radius, 2 * radius),
        radius,
        c,
    );
}

impl Widget for CalibrateView {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        let cy0 = r.y + r.h / 2;
        cx.text(
            FontId::Bold,
            t.size_title,
            Rect::new(r.x, cy0 - 120, r.w, 40),
            Align::Center,
            t.text,
            "Calibrate touch",
        );
        let (msg, tone) = self.message();
        let color = t.tone(tone);
        cx.text(
            FontId::Sans,
            t.size_body,
            Rect::new(r.x + 160, cy0 - 70, r.w - 320, 40),
            Align::Center,
            color,
            &msg,
        );
        let step = format!("{} of 4 corners", self.taps.len());
        cx.text(
            FontId::Sans,
            t.size_small,
            Rect::new(r.x, cy0 - 30, r.w, 30),
            Align::Center,
            t.text_dim,
            &step,
        );
        for (i, c) in Corner::ORDER.into_iter().enumerate() {
            let (x, y) = Self::target(r, c);
            if Some(c) == self.cal.next_corner() && self.result.is_none() {
                circle(cx, x, y, TARGET_RADIUS, t.accent);
                circle(cx, x, y, TARGET_RADIUS - 8, t.bg);
                circle(cx, x, y, 8, t.accent);
            } else if i >= self.taps.len() {
                circle(cx, x, y, 10, t.outline);
            }
        }
        for &(x, y) in &self.taps {
            circle(cx, x, y, 12, t.good);
        }
        for (b, br) in self.buttons(r) {
            cx.canvas.fill_round_rect(br, t.radius, t.raised);
            cx.canvas.stroke_round_rect(br, t.radius, 2, t.outline);
            cx.text(
                FontId::Bold,
                t.size_body,
                br,
                Align::Center,
                t.text,
                b.label(),
            );
        }
    }

    fn targets(&self, r: Rect) -> Vec<mpfree_ui::Target> {
        use mpfree_ui::Target;
        let mut out: Vec<Target> = self
            .buttons(r)
            .into_iter()
            .enumerate()
            .map(|(i, (b, br))| Target::tap("button", i, b.label(), br))
            .collect();
        // A tap anywhere else is the tap on the mark the page asks for.
        out.push(Target::tap(
            "mark",
            0,
            "the mark",
            Rect::new(r.x + 40, r.y + 40, 80, 80),
        ));
        out
    }

    fn touch(
        &mut self,
        finger: u64,
        phase: Touch,
        x: i32,
        y: i32,
        r: Rect,
    ) -> Option<mpfree_ui::EventKind> {
        if phase == Touch::Down {
            let what = match self
                .buttons(r)
                .into_iter()
                .find(|(_, br)| br.contains(x, y))
            {
                Some((b, _)) => Finger::Button(b),
                None => Finger::Tap,
            };
            self.fingers.retain(|(f, _)| *f != finger);
            self.fingers.push((finger, what));
        }
        if !matches!(phase, Touch::Up | Touch::Cancel) {
            return None;
        }
        let i = self.fingers.iter().position(|(f, _)| *f == finger)?;
        let (_, what) = self.fingers.remove(i);
        if phase == Touch::Cancel {
            return None;
        }
        match what {
            Finger::Button(b)
                if self
                    .buttons(r)
                    .iter()
                    .any(|(bb, br)| *bb == b && br.contains(x, y)) =>
            {
                match b {
                    Button::Again => self.restart(),
                    Button::Exit => self.leave(),
                }
            }
            Finger::Button(_) => {}
            Finger::Tap => self.tap(x, y, r),
        }
        None
    }
}

impl TouchPage for CalibrateView {
    fn open(&mut self) {
        self.restart();
        self.fingers.clear();
        self.exit = false;
        self.lines.push("touch-calibrate-open".to_string());
    }

    fn leave(&mut self) {
        if !self.exit {
            self.exit = true;
            self.lines.push("touch-calibrate-exit".to_string());
        }
    }

    fn wants_exit(&self) -> bool {
        self.exit
    }

    fn has_lines(&self) -> bool {
        !self.lines.is_empty()
    }

    fn take_lines(&mut self) -> Vec<String> {
        std::mem::take(&mut self.lines)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_touch::pointer::Orientation;

    const SCREEN: Rect = Rect::new(0, 0, 1280, 800);

    fn tap(v: &mut CalibrateView, f: u64, x: i32, y: i32) {
        v.touch(f, Touch::Down, x, y, SCREEN);
        v.touch(f, Touch::Up, x, y, SCREEN);
    }

    fn opened() -> CalibrateView {
        let mut v = CalibrateView::new();
        v.open();
        assert_eq!(v.take_lines(), vec!["touch-calibrate-open"]);
        v
    }

    #[test]
    fn four_targets_on_the_built_in_mapping_match() {
        let mut v = opened();
        for (i, c) in Corner::ORDER.into_iter().enumerate() {
            let (x, y) = CalibrateView::target(SCREEN, c);
            tap(&mut v, i as u64, x, y);
        }
        let lines = v.take_lines();
        assert_eq!(lines.len(), 6, "{lines:?}");
        assert!(
            lines[0].starts_with("touch-calibrate-tap: corner=top-left x=70 y=70 raw_x="),
            "{}",
            lines[0]
        );
        assert!(lines[0].ends_with(" outcome=ok"));
        assert_eq!(lines[4], "touch-calibrate: result=match");
        assert_eq!(lines[5], orientation_line(MPC_ONE.orientation));
        assert!(matches!(v.result, Some(Ok(c)) if c.matches_profile));
    }

    #[test]
    fn a_differently_mounted_panel_is_reported_not_applied() {
        // A panel mounted with no swap or flip: the owner taps the physical
        // corners, which the built-in mapping draws at other places.
        let other = Orientation {
            swap_xy: false,
            flip_x: false,
            flip_y: false,
        };
        let theirs = mpfree_touch::profile::TouchProfile {
            orientation: other,
            ..MPC_ONE
        }
        .axis_map(1280, 800)
        .unwrap();
        let ours = MPC_ONE.axis_map(1280, 800).unwrap();
        let mut v = opened();
        for (i, c) in Corner::ORDER.into_iter().enumerate() {
            let (x, y) = CalibrateView::target(SCREEN, c);
            let (rx, ry) = theirs.unmap(x as f32, y as f32);
            let (sx, sy) = ours.map(rx, ry);
            tap(&mut v, i as u64, sx.round() as i32, sy.round() as i32);
        }
        let lines = v.take_lines();
        assert_eq!(lines[4], "touch-calibrate: result=differs");
        assert_eq!(lines[5], "touch-orientation: swap_xy=0 flip_x=0 flip_y=0");
    }

    #[test]
    fn central_and_repeated_taps_are_asked_again_and_buttons_are_not_taps() {
        let mut v = opened();
        tap(&mut v, 1, 640, 200);
        assert_eq!(v.note, Note::TooCentral);
        let (x, y) = CalibrateView::target(SCREEN, Corner::TopLeft);
        tap(&mut v, 2, x, y);
        tap(&mut v, 3, x + 5, y + 5);
        assert_eq!(v.note, Note::SameAs(Corner::TopLeft));
        assert_eq!(v.taps.len(), 1);
        let lines = v.take_lines();
        assert!(
            lines[0].ends_with("outcome=too_central")
                && lines[2].ends_with("outcome=same_as_top-left"),
            "{lines:?}"
        );
        // Start again clears the taps and logs nothing.
        let (_, again) = v.buttons(SCREEN)[0];
        tap(&mut v, 4, again.x + 5, again.y + 5);
        assert!(v.taps.is_empty() && v.take_lines().is_empty());
        let (_, exit) = v.buttons(SCREEN)[1];
        tap(&mut v, 5, exit.x + 5, exit.y + 5);
        assert!(v.wants_exit());
        assert_eq!(v.take_lines(), vec!["touch-calibrate-exit"]);
    }

    #[test]
    fn out_of_order_corners_fit_nothing() {
        let mut v = opened();
        for (i, c) in [
            Corner::TopLeft,
            Corner::BottomRight,
            Corner::TopRight,
            Corner::BottomLeft,
        ]
        .into_iter()
        .enumerate()
        {
            let (x, y) = CalibrateView::target(SCREEN, c);
            tap(&mut v, i as u64, x, y);
        }
        let lines = v.take_lines();
        assert!(
            lines
                .last()
                .unwrap()
                .starts_with("touch-calibrate: result=no_fit"),
            "{lines:?}"
        );
        assert_eq!(v.message().1, Tone::Bad);
    }

    #[test]
    fn buttons_stay_in_the_middle_under_any_mounting() {
        let v = CalibrateView::new();
        // Buttons are inside the 15%-from-middle band, where no corner tap counts.
        for (_, br) in v.buttons(SCREEN) {
            assert!(
                br.x >= 1280 * 35 / 100 && br.right() <= 1280 * 65 / 100,
                "{br:?}"
            );
            assert!(
                br.y >= 800 * 35 / 100 && br.bottom() <= 800 * 65 / 100,
                "{br:?}"
            );
        }
        let mut canvas = mpfree_ui::SoftCanvas::new(1280, 800);
        let mut fonts = mpfree_ui::Fonts::embedded();
        let theme = mpfree_ui::Theme::dark();
        let mut cx = PaintCx {
            canvas: &mut canvas,
            theme: &theme,
            fonts: &mut fonts,
        };
        v.paint(&mut cx, SCREEN);
    }
}
