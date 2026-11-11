//! The boot menu shown under the galaxy: MPFree OS or the factory OS.
//!
//! Rules (Jay, 2026-11-10): MPFree OS is the default and starts by itself
//! after a short countdown; the factory OS starts only when someone picks
//! it. Picking means turning the data wheel to it and pressing PLAY or any
//! pad. MPFree is offered only when its USB stick is present and verified
//! (the caller decides that); without it the menu says so and the factory OS
//! starts. Turning the wheel stops the countdown; if nobody touches anything
//! for [`IDLE_SECONDS`] after that, the menu goes back to the default.
//!
//! This module is pure logic and drawing: no I/O, no clock of its own.

use mpfree_screen_log::font;

/// After the last input, how long the menu waits before booting the default.
pub const IDLE_SECONDS: f32 = 30.0;
/// Without an MPFree stick, how long the message shows before the factory OS starts.
pub const NO_STICK_SECONDS: f32 = 3.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    MpFree,
    Factory,
}

impl Choice {
    pub fn label(self) -> &'static str {
        match self {
            Choice::MpFree => "MPFree OS",
            Choice::Factory => "Factory OS",
        }
    }
}

#[derive(Clone, Debug)]
pub struct BootMenu {
    mpfree_available: bool,
    countdown: f32,
    selected: Choice,
    /// When the countdown to the default started (start, or the last input + idle).
    since: f32,
    touched: bool,
    decided: Option<Choice>,
}

impl BootMenu {
    /// `countdown`: seconds before the default starts on its own. `now`: the
    /// caller's clock, in seconds.
    pub fn new(mpfree_available: bool, countdown: f32, now: f32) -> BootMenu {
        BootMenu {
            mpfree_available,
            countdown: countdown.max(1.0),
            selected: if mpfree_available {
                Choice::MpFree
            } else {
                Choice::Factory
            },
            since: now,
            touched: false,
            decided: None,
        }
    }

    pub fn selected(&self) -> Choice {
        self.selected
    }

    pub fn decided(&self) -> Option<Choice> {
        self.decided
    }

    /// The data wheel turned: clockwise (positive) moves right, to Factory OS.
    pub fn wheel(&mut self, delta: i32, now: f32) {
        if self.decided.is_some() || !self.mpfree_available || delta == 0 {
            return;
        }
        self.selected = if delta > 0 {
            Choice::Factory
        } else {
            Choice::MpFree
        };
        self.touched = true;
        self.since = now;
    }

    /// PLAY or a pad: start whatever is highlighted.
    pub fn confirm(&mut self) {
        if self.decided.is_none() {
            self.decided = Some(self.selected);
        }
    }

    /// Advances the clock; returns the choice once one is made.
    pub fn tick(&mut self, now: f32) -> Option<Choice> {
        if self.decided.is_none() {
            if !self.mpfree_available {
                if now - self.since >= NO_STICK_SECONDS {
                    self.decided = Some(Choice::Factory);
                }
            } else if !self.touched {
                if now - self.since >= self.countdown {
                    self.decided = Some(Choice::MpFree);
                }
            } else if now - self.since >= IDLE_SECONDS {
                // Nobody finished choosing: back to the default.
                self.selected = Choice::MpFree;
                self.touched = false;
                self.since = now;
            }
        }
        self.decided
    }

    /// The menu as text: the options line (with the highlighted one marked)
    /// and one or two hint lines.
    pub fn lines(&self, now: f32) -> MenuText {
        let left = |c: f32| (c - (now - self.since)).ceil().max(0.0) as u32;
        if !self.mpfree_available {
            return MenuText {
                options: vec![(Choice::Factory.label(), true)],
                hints: vec![
                    "No MPFree USB stick found".into(),
                    format!("Starting factory OS in {}", left(NO_STICK_SECONDS)),
                ],
            };
        }
        let options = vec![
            (Choice::MpFree.label(), self.selected == Choice::MpFree),
            (Choice::Factory.label(), self.selected == Choice::Factory),
        ];
        let hints = match self.decided {
            Some(c) => vec![format!("Starting {}", c.label())],
            None if !self.touched => vec![
                format!("MPFree OS starts in {}", left(self.countdown)),
                "Turn the wheel to choose".into(),
            ],
            None => vec![
                "Press PLAY or a pad".into(),
                format!("to start {}", self.selected.label()),
            ],
        };
        MenuText { options, hints }
    }
}

pub struct MenuText {
    /// Each option's label and whether it is highlighted.
    pub options: Vec<(&'static str, bool)>,
    pub hints: Vec<String>,
}

/// Characters across the options line, gaps included.
fn options_width(t: &MenuText) -> usize {
    t.options.iter().map(|(l, _)| l.len() + 2).sum::<usize>()
        + 2 * t.options.len().saturating_sub(1)
}

/// Whether the 8x8-pixel menu fits a canvas this size (otherwise the caller
/// should show the menu as plain text cells).
pub fn fits_pixels(width: usize, height: usize) -> bool {
    width >= 30 * 8 && height >= 120
}

/// Draws the menu with the 8x8 font into the lower part of the scene's
/// pixels: a dark panel, the options (highlighted one boxed in cyan) and hints.
pub fn draw_pixels(out: &mut [[u8; 3]], width: usize, height: usize, t: &MenuText) {
    let rows = 1 + t.hints.len();
    let panel_h = rows * 12 + 10;
    let panel_w = (t
        .hints
        .iter()
        .map(|h| h.len())
        .max()
        .unwrap_or(0)
        .max(options_width(t))
        * 8
        + 24)
        .min(width);
    let top = (height * 3 / 4)
        .saturating_sub(panel_h / 2)
        .min(height.saturating_sub(panel_h));
    let left = (width - panel_w) / 2;
    for y in top..(top + panel_h).min(height) {
        for x in left..left + panel_w {
            let p = &mut out[y * width + x];
            for c in p.iter_mut() {
                *c = (*c as u32 * 30 / 100) as u8;
            }
        }
    }
    let mut text = |s: &str, x0: usize, y0: usize, fg: [u8; 3], bg: Option<[u8; 3]>| {
        for (i, ch) in s.bytes().enumerate() {
            let glyph = if (font::FIRST..=font::LAST).contains(&ch) {
                font::GLYPHS[(ch - font::FIRST) as usize]
            } else {
                [0; 8]
            };
            for (gy, bits) in glyph.iter().enumerate() {
                for gx in 0..8 {
                    let (x, y) = (x0 + i * 8 + gx, y0 + gy);
                    if x < width && y < height {
                        if bits & (1 << gx) != 0 {
                            out[y * width + x] = fg;
                        } else if let Some(b) = bg {
                            out[y * width + x] = b;
                        }
                    }
                }
            }
        }
    };
    // Options, centred, each as " label " with the highlighted one on cyan.
    let mut x = left + (panel_w - options_width(t) * 8) / 2;
    let y = top + 6;
    for (label, on) in &t.options {
        let padded = format!(" {label} ");
        if *on {
            text(&padded, x, y, [5, 10, 30], Some([90, 225, 255]));
        } else {
            text(&padded, x, y, [170, 180, 200], None);
        }
        x += (padded.len() + 2) * 8;
    }
    for (i, hint) in t.hints.iter().enumerate() {
        let hx = left + (panel_w - hint.len() * 8) / 2;
        text(hint, hx, top + 6 + 12 * (i + 1), [230, 230, 240], None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mpfree_starts_by_itself() {
        let mut m = BootMenu::new(true, 5.0, 0.0);
        assert_eq!(m.selected(), Choice::MpFree);
        assert_eq!(m.tick(4.9), None);
        assert_eq!(m.tick(5.0), Some(Choice::MpFree));
        // Once decided, nothing changes it.
        m.wheel(1, 5.1);
        m.confirm();
        assert_eq!(m.tick(6.0), Some(Choice::MpFree));
    }

    #[test]
    fn factory_only_when_picked_and_confirmed() {
        let mut m = BootMenu::new(true, 5.0, 0.0);
        m.wheel(1, 2.0);
        assert_eq!(m.selected(), Choice::Factory);
        // Turning stops the countdown: nothing starts at the old deadline or soon after.
        assert_eq!(m.tick(5.0), None);
        assert_eq!(m.tick(20.0), None);
        m.confirm();
        assert_eq!(m.tick(20.1), Some(Choice::Factory));
    }

    #[test]
    fn turning_back_and_confirming_starts_mpfree() {
        let mut m = BootMenu::new(true, 5.0, 0.0);
        m.wheel(1, 1.0);
        m.wheel(-1, 2.0);
        m.confirm();
        assert_eq!(m.tick(2.1), Some(Choice::MpFree));
    }

    #[test]
    fn an_abandoned_choice_goes_back_to_the_default() {
        let mut m = BootMenu::new(true, 5.0, 0.0);
        m.wheel(1, 1.0);
        assert_eq!(m.tick(1.0 + IDLE_SECONDS - 0.1), None);
        assert_eq!(
            m.tick(1.0 + IDLE_SECONDS),
            None,
            "back to the default, countdown restarts"
        );
        assert_eq!(m.selected(), Choice::MpFree);
        assert_eq!(m.tick(1.0 + IDLE_SECONDS + 5.0), Some(Choice::MpFree));
    }

    #[test]
    fn without_the_stick_it_is_the_factory_os() {
        let mut m = BootMenu::new(false, 5.0, 0.0);
        assert_eq!(m.selected(), Choice::Factory);
        m.wheel(-1, 0.5);
        assert_eq!(
            m.selected(),
            Choice::Factory,
            "the wheel can't pick MPFree without a stick"
        );
        assert_eq!(m.tick(NO_STICK_SECONDS - 0.1), None);
        assert_eq!(m.tick(NO_STICK_SECONDS), Some(Choice::Factory));
        assert!(m.lines(0.0).hints[0].contains("No MPFree"));
    }

    #[test]
    fn countdown_text() {
        let m = BootMenu::new(true, 5.0, 0.0);
        assert_eq!(m.lines(0.2).hints[0], "MPFree OS starts in 5");
        assert_eq!(m.lines(4.5).hints[0], "MPFree OS starts in 1");
        let t = m.lines(0.0);
        assert_eq!(t.options, vec![("MPFree OS", true), ("Factory OS", false)]);
    }

    #[test]
    fn the_pixel_menu_fits_the_mpc_canvas() {
        // 320x200: the MPC One's scene at 4x8-pixel cells.
        assert!(fits_pixels(320, 200));
        assert!(!fits_pixels(80, 48));
        let mut px = vec![[100u8, 100, 100]; 320 * 200];
        let mut m = BootMenu::new(true, 5.0, 0.0);
        m.wheel(1, 0.1);
        draw_pixels(&mut px, 320, 200, &m.lines(0.2));
        // The panel darkened something and the highlight colour appears.
        assert!(px.contains(&[30, 30, 30]));
        assert!(px.contains(&[90, 225, 255]));
    }
}
