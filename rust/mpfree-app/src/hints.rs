//! Prefs › Appearance › Screen and lights › Hints: how much the hint line
//! says (Jay's second touch review, 13.3; the touch design set, tab 3).
//!
//! - **Persistent** (the default, for a new user): the line says what the
//!   page's controls do, always. Play keeps its line by taking the room out
//!   of its bottom row.
//! - **Contextual**: the page's help shows on a page's first
//!   [`FIRST_VISITS`] visits, until the first touch on the screen. Which
//!   pages you know is kept ([`Seen`]), so a page you have visited enough
//!   stays quiet after a restart too.
//! - **On demand**: the page's help shows while SHIFT is held for
//!   [`SHIFT_HELP_AFTER`] seconds (a quick SHIFT combo shows SHIFT's own
//!   keys, as always).
//! - **Off**: no ordinary help.
//!
//! In every mode the line still says what a held button does, what the app
//! tells you (as "Saved"), and the power card's, the letters pad's and
//! learning's keys. Away from Persistent, Play has no line: what it would
//! say shows over the bottom row, and what the app tells shows over the
//! tabs (`gui.rs`, `play_widgets::News`).

use crate::app::View;
use mpfree_ui::touch::Class;
use mpfree_ui::{Align, EventKind, FontId, PaintCx, Rect, Target, Tone, Touch, Widget};

/// How much the hint line says.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Hints {
    #[default]
    Persistent,
    Contextual,
    OnDemand,
    Off,
}

/// How many visits a page shows its help at Contextual.
pub const FIRST_VISITS: u8 = 3;

/// SHIFT held alone this long, in seconds, shows the page's help at On
/// demand and Contextual. Shorter, the line says SHIFT's own keys, so a
/// SHIFT combo reads as before.
pub const SHIFT_HELP_AFTER: f32 = 1.0;

impl Hints {
    pub const ALL: [Hints; 4] = [
        Hints::Persistent,
        Hints::Contextual,
        Hints::OnDemand,
        Hints::Off,
    ];

    pub fn name(self) -> &'static str {
        ["Persistent", "Contextual", "On demand", "Off"][self as usize]
    }

    /// The line under its button in Prefs.
    pub fn line(self) -> &'static str {
        ["always", "on new pages", "hold SHIFT", "what you hold"][self as usize]
    }

    pub fn key(self) -> &'static str {
        ["persistent", "contextual", "on-demand", "off"][self as usize]
    }

    pub fn parse(v: &str) -> Option<Hints> {
        Hints::ALL.into_iter().find(|h| h.key() == v.trim())
    }

    /// Play keeps its hint line only at Persistent: elsewhere its bottom
    /// row takes the room (the touch design set, tab 3).
    pub fn play_line(self) -> bool {
        self == Hints::Persistent
    }

    /// Whether the page's ordinary help shows: `fresh` is a page visit
    /// Contextual still helps on (no touch yet), `long_shift` SHIFT held
    /// for [`SHIFT_HELP_AFTER`].
    pub fn shows_help(self, fresh: bool, long_shift: bool) -> bool {
        match self {
            Hints::Persistent => true,
            Hints::Contextual => fresh || long_shift,
            Hints::OnDemand => long_shift,
            Hints::Off => false,
        }
    }
}

/// A page's name in [`Seen`]: the tab's (or view's) own name.
pub fn page_key(v: View) -> String {
    format!("{v:?}").to_lowercase()
}

/// How many times each page has been visited, up to [`FIRST_VISITS`]:
/// Contextual helps on a page until it is known. Kept in Prefs as
/// `hints-seen = play 3, looper 1`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Seen(Vec<(String, u8)>);

impl Seen {
    /// The visits counted for `page` (at most [`FIRST_VISITS`]).
    pub fn visits(&self, page: &str) -> u8 {
        self.0
            .iter()
            .find(|(p, _)| p == page)
            .map_or(0, |&(_, n)| n)
    }

    /// One more visit to `page`; false once it is known (nothing changes,
    /// nothing to keep).
    pub fn visit(&mut self, page: &str) -> bool {
        match self.0.iter_mut().find(|(p, _)| p == page) {
            Some((_, n)) if *n >= FIRST_VISITS => false,
            Some((_, n)) => {
                *n += 1;
                true
            }
            None => {
                self.0.push((page.to_string(), 1));
                true
            }
        }
    }

    /// Each page's count, the larger of the two.
    pub fn merge(&mut self, other: &Seen) {
        for (p, n) in &other.0 {
            match self.0.iter_mut().find(|(q, _)| q == p) {
                Some((_, m)) => *m = (*m).max(*n),
                None => self.0.push((p.clone(), *n)),
            }
        }
    }

    /// As kept: `play 3, looper 1`.
    pub fn text(&self) -> String {
        self.0
            .iter()
            .map(|(p, n)| format!("{p} {n}"))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// From [`Seen::text`]; a part that doesn't read is left out.
    pub fn parse(v: &str) -> Seen {
        Seen(
            v.split(',')
                .filter_map(|part| {
                    let (p, n) = part.trim().rsplit_once(' ')?;
                    let n: u8 = n.trim().parse().ok()?;
                    let p = p.trim();
                    (!p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric()))
                        .then(|| (p.to_string(), n.min(FIRST_VISITS)))
                })
                .collect(),
        )
    }
}

// ------------------------------------------------- Play without the line

/// What the app tells, on Play without the line: over the header's tabs.
pub const NEWS: &str = "hint-news";
/// What the line would say otherwise, on Play without the line: over the
/// bottom row's foot.
pub const STRIP: &str = "hint-strip";
/// The message's size (the touch design set, tab 3, v3): wide enough to
/// cover the tabs, the chips at the header's right still showing.
pub const NEWS_W: i32 = 800;
pub const NEWS_H: i32 = 48;
/// The strip's height: two lines of the hint line's size.
pub const STRIP_H: i32 = 52;

/// A one-line message over the tabs while it shows. A tap on it closes it
/// (on the lift, as every tap: a finger that slides off leaves it) and
/// does nothing else: no touch reaches a tab under it (Jay's second
/// review, 13.4).
#[derive(Default)]
pub struct News {
    pub text: String,
    pub tone: Tone,
    /// The finger that went down on it.
    finger: Option<u64>,
}

impl News {
    /// Where it goes: over the left of the header (`header`, the tabs).
    pub fn place(header: Rect) -> Rect {
        let h = NEWS_H.min(header.h);
        Rect::new(
            header.x,
            header.y + (header.h - h) / 2,
            NEWS_W.min(header.w),
            h,
        )
    }
}

impl Widget for News {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.canvas.fill_rect(r, t.bg);
        let edge = match self.tone {
            Tone::Bad => t.bad,
            _ => t.warn,
        };
        cx.canvas.fill_round_rect(r, t.round(10), t.panel());
        cx.canvas
            .stroke_round_rect(r, t.round(10), 1, edge.with_alpha(160));
        let close = "tap to close";
        let close_w = cx.measure(FontId::Sans, 13.0, close);
        let (end, words) = r.inset_xy(18, 0).take_right(close_w + 8);
        let ink = t.readable(t.tone(self.tone), t.panel());
        cx.text(FontId::Bold, 16.0, words, Align::Left, ink, &self.text);
        cx.text(FontId::Sans, 13.0, end, Align::Right, t.text_dim, close);
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        vec![Target::tap("close", 0, "Close the message", r).class(Class::Secondary)]
    }

    fn touch(&mut self, finger: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        match phase {
            Touch::Down => {
                self.finger = Some(finger);
                None
            }
            Touch::Move => None,
            Touch::Up => (self.finger.take() == Some(finger) && r.contains(x, y))
                .then_some(EventKind::Tab(0)),
            Touch::Cancel => {
                self.finger = None;
                None
            }
        }
    }
}

/// What the hint line would say, over the foot of Play's bottom row while
/// it says anything: SHIFT's keys, what a held button does, the page's
/// help on a new page's visit (Contextual) or with SHIFT held a while. It
/// takes the touches that land on it and does nothing with them, so no
/// touch reaches a throw it covers.
#[derive(Default)]
pub struct Strip {
    pub text: String,
    pub tone: Tone,
}

impl Strip {
    /// Where it goes: the foot of `row`, the bottom row.
    pub fn place(row: Rect) -> Rect {
        let h = STRIP_H.min(row.h);
        Rect::new(row.x, row.bottom() - h, row.w, h)
    }
}

impl Widget for Strip {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.canvas.fill_rect(r, t.bg);
        cx.canvas.fill_round_rect(r, t.round(10), t.panel());
        cx.canvas
            .fill_rect(Rect::new(r.x + 10, r.y, r.w - 20, 2), t.accent2);
        let ink = match self.tone {
            Tone::Dim => t.text,
            tone => t.readable(t.tone(tone), t.panel()),
        };
        cx.text(
            FontId::Sans,
            t.size_small + 1.0,
            r.inset_xy(18, 0),
            Align::Left,
            ink,
            &self.text,
        );
    }

    fn touch(
        &mut self,
        _finger: u64,
        _phase: Touch,
        _x: i32,
        _y: i32,
        _r: Rect,
    ) -> Option<EventKind> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_four_modes_read_and_say_when_help_shows() {
        for h in Hints::ALL {
            assert_eq!(Hints::parse(h.key()), Some(h));
        }
        assert_eq!(Hints::parse("loud"), None);
        assert!(Hints::Persistent.shows_help(false, false));
        assert!(Hints::Contextual.shows_help(true, false));
        assert!(Hints::Contextual.shows_help(false, true));
        assert!(!Hints::Contextual.shows_help(false, false));
        assert!(Hints::OnDemand.shows_help(false, true));
        assert!(!Hints::OnDemand.shows_help(true, false), "no first visits");
        assert!(!Hints::Off.shows_help(true, true));
        assert!(Hints::Persistent.play_line());
        assert!(!Hints::OnDemand.play_line());
    }

    #[test]
    fn pages_are_known_after_three_visits_and_kept_as_text() {
        let mut s = Seen::default();
        let play = page_key(View::Play);
        assert_eq!(play, "play");
        assert!(s.visit(&play));
        assert!(s.visit(&play));
        assert!(s.visit("looper"));
        assert!(s.visit(&play));
        assert_eq!(s.visits(&play), FIRST_VISITS);
        assert!(!s.visit(&play), "known: nothing more to keep");
        assert_eq!(s.text(), "play 3, looper 1");
        assert_eq!(Seen::parse(&s.text()), s);
        assert_eq!(
            Seen::parse("play 9, , bad, mix x, a b 2, prefs 2"),
            Seen(vec![("play".into(), 3), ("prefs".into(), 2)])
        );
    }
}
