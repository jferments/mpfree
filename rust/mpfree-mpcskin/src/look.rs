//! The colours of MPFree's own pop-ups over a plugin skin: the big knob
//! editor and the list of choices. They follow the theme (owner, 22 Nov
//! 2026); the plugin's own art never changes colour.
//!
//! A theme can't make them unusable. [`Look::from_theme`] checks every
//! colour pair a pop-up draws: text must reach 4.5:1 on what is under it,
//! the knob's arc and pointer 3:1. A pair that falls short is moved just
//! far enough towards the theme's text or inverse text, and each move is a
//! [`Rescue`]: counted, logged by the app and shown on its Debug page. The
//! pop-up's panel is always solid, and its edge always stands out from the
//! dimmed skin ([`Look::edge`]). Marks MPFree draws on the art itself (the
//! Q-Link bank's frame) measure the skin under them ([`Look::mark`]).

use mpfree_ui::roles::{over, reach};
use mpfree_ui::theme::contrast;
use mpfree_ui::{Color, Role, Theme};

/// What text must reach on what is under it (WCAG's AA for text).
pub const TEXT_MIN: f32 = 4.5;
/// What lines and the knob must reach (WCAG's floor for controls).
pub const LINE_MIN: f32 = 3.0;
/// How see-through a mark on the art is when one ink reads.
pub const MARK_ALPHA: u8 = 170;

/// A knob's colours: its track, the lit arc, the cap and the pointer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KnobInk {
    pub track: Color,
    pub lit: Color,
    pub cap: Color,
    pub pointer: Color,
}

impl KnobInk {
    /// Akai's, for the generic knob types a skin places on its art.
    pub const AKAI: KnobInk = KnobInk {
        track: Color::rgb(0x3A3A3A),
        lit: Color::rgb(0xD31145),
        cap: Color::rgb(0x2A2A2A),
        pointer: Color::rgb(0xFFFFFF),
    };
}

/// The pop-ups' colours in one theme. Every colour is solid.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    pub panel: Color,
    /// The panel's outline, as the theme has it ([`Look::edge`] is what is
    /// drawn).
    pub line: Color,
    pub text: Color,
    pub knob: KnobInk,
    /// The chosen row of a list, and its text.
    pub row: Color,
    pub row_text: Color,
    pub radius: i32,
    pub row_radius: i32,
    /// The theme's text and inverse text: the inks a line can be moved
    /// towards, and a mark on the art drawn in.
    pub inks: [Color; 2],
}

/// A colour a theme gave a pop-up that didn't read, and the one drawn.
#[derive(Clone, Debug, PartialEq)]
pub struct Rescue {
    pub what: &'static str,
    pub was: Color,
    pub now: Color,
}

/// Black and white: the last resort, for a theme whose own text and
/// inverse text are too alike to reach the floor on something.
const LAST_RESORT: [Color; 2] = [Color::rgb(0x000000), Color::rgb(0xFFFFFF)];

/// Of `inks`, the one that reads best on `on`.
fn best(inks: [Color; 2], on: Color) -> Color {
    if contrast(inks[1], on) > contrast(inks[0], on) {
        inks[1]
    } else {
        inks[0]
    }
}

/// `c`, moved just far enough to reach `min` on `on`: towards the theme
/// ink that reads there, or if even that falls short, black or white.
fn fix(c: Color, inks: [Color; 2], on: Color, min: f32) -> Color {
    let moved = reach(c, best(inks, on), on, min);
    if contrast(moved, on) >= min {
        moved
    } else {
        reach(c, best(LAST_RESORT, on), on, min)
    }
}

impl Look {
    /// The pop-ups' colours for theme `t`, with the colours it gave that
    /// had to be moved to read.
    pub fn from_theme(t: &Theme) -> (Look, Vec<Rescue>) {
        let mut rescues = Vec::new();
        let mut guard = |what: &'static str, was: Color, now: Color| {
            if now != was {
                rescues.push(Rescue { what, was, now });
            }
            now
        };
        let inks = [over(t.text, t.bg), over(t.role(Role::TextInverse), t.bg)];
        // The panel is solid in every theme: a see-through one would show
        // the skin's art through the text.
        let panel = over(t.surface, t.bg);
        let text = over(t.text, panel);
        let text = guard("text on the pop-up", text, fix(text, inks, panel, TEXT_MIN));
        let track = over(t.raised, panel);
        let lit = over(t.accent, panel);
        let lit = guard("the knob's arc", lit, fix(lit, inks, panel, LINE_MIN));
        let cap = over(t.surface.mix(t.raised, 0.5), panel);
        let pointer = guard("the knob's pointer", text, fix(text, inks, cap, LINE_MIN));
        // The chosen row: the accent, its text whichever ink reads; if
        // neither reaches 4.5:1 on the accent, the row moves away from it.
        // A theme whose inks are too alike to read on anything gets black
        // or white text.
        let alike = contrast(inks[0], inks[1]) < TEXT_MIN;
        let row = over(t.accent, panel);
        let theirs = best(inks, row);
        let row_text = if contrast(theirs, row) < TEXT_MIN && alike {
            best(LAST_RESORT, row)
        } else {
            theirs
        };
        let row_text = guard("text on the chosen row", theirs, row_text);
        let away = best(if alike { LAST_RESORT } else { inks }, row_text);
        let row = guard(
            "the chosen row",
            row,
            fix(row, [away, away], row_text, TEXT_MIN),
        );
        let look = Look {
            panel,
            line: over(t.outline, panel),
            text,
            knob: KnobInk {
                track,
                lit,
                cap,
                pointer,
            },
            row,
            row_text,
            radius: t.round(18),
            row_radius: t.round(10),
            inks,
        };
        (look, rescues)
    }

    /// The panel's outline over `under` (the dimmed skin round it): the
    /// theme's outline, moved towards an ink that reads just far enough to
    /// stand out 3:1, unless the panel stands out on its own.
    pub fn edge(&self, under: Color) -> Color {
        if contrast(self.panel, under) >= LINE_MIN {
            self.line
        } else {
            fix(self.line, self.inks, under, LINE_MIN)
        }
    }

    /// A mark on the skin's art over `under` (the average of the pixels it
    /// covers): the theme ink that reads there, see-through, or `None` when
    /// neither reaches 3:1, for a two-tone line (an inner line in one ink
    /// on an outer one in the other).
    pub fn mark(&self, under: Color) -> Option<Color> {
        let ink = best(self.inks, under).with_alpha(MARK_ALPHA);
        (contrast(over(ink, under), under) >= LINE_MIN).then_some(ink)
    }
}

impl Default for Look {
    fn default() -> Look {
        Look::from_theme(&Theme::default()).0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every colour a theme gives the pop-ups is either as it gave it or
    /// rescued, and what is drawn reads.
    fn reads(t: &Theme) -> Vec<Rescue> {
        let (l, rescues) = Look::from_theme(t);
        assert_eq!(l.panel.a, 255);
        assert!(contrast(l.text, l.panel) >= TEXT_MIN, "{}", t.name);
        assert!(
            contrast(l.knob.lit, l.panel) >= LINE_MIN - 0.01,
            "{}",
            t.name
        );
        assert!(
            contrast(l.knob.pointer, l.knob.cap) >= LINE_MIN - 0.01,
            "{}",
            t.name
        );
        assert!(contrast(l.row_text, l.row) >= TEXT_MIN - 0.01, "{}", t.name);
        for under in [Color::rgb(0), Color::rgb(0x3F3F3F), Color::rgb(0x808080)] {
            let edge = l.edge(under);
            assert!(
                contrast(edge, under) >= LINE_MIN - 0.01 || contrast(l.panel, under) >= LINE_MIN,
                "{}: edge on {under:?}",
                t.name
            );
        }
        rescues
    }

    #[test]
    fn the_built_in_themes_need_no_rescue() {
        for name in mpfree_ui::theme::BUILTIN {
            let t = Theme::builtin(name).unwrap();
            assert_eq!(reads(&t), Vec::new(), "{name}");
        }
    }

    #[test]
    fn a_torture_theme_is_rescued_and_every_rescue_is_listed() {
        // Every colour alike, see-through panels. The app refuses such a
        // theme (`Theme::unusable`); the guards don't count on that.
        let (t, _) = Theme::load(
            "bg = #777777\nsurface = #7A7A7A80\nraised = #7C7C7C\noutline = #787878\n\
             text = #7B7B7B\ntext_dim = #7B7B7B\ntext_faint = #7B7B7B\naccent = #797979\n\
             on_accent = #7A7A7A\nglow = 255",
        );
        assert!(t.unusable().is_some());
        let rescues = reads(&t);
        let what: Vec<&str> = rescues.iter().map(|r| r.what).collect();
        assert!(what.contains(&"text on the pop-up"), "{what:?}");
        assert!(what.contains(&"the knob's arc"), "{what:?}");
        assert!(
            what.contains(&"the chosen row") || what.contains(&"text on the chosen row"),
            "{what:?}"
        );
    }

    #[test]
    fn a_mark_takes_the_ink_that_reads_or_none() {
        let l = Look::default();
        let on_black = l.mark(Color::rgb(0)).unwrap();
        assert_eq!(on_black.a, MARK_ALPHA);
        assert!(contrast(over(on_black, Color::rgb(0)), Color::rgb(0)) >= LINE_MIN);
        let on_white = l.mark(Color::rgb(0xFFFFFF)).unwrap();
        assert!(contrast(over(on_white, Color::rgb(0xFFFFFF)), Color::rgb(0xFFFFFF)) >= LINE_MIN);
        // Mid grey: neither ink, see-through, reaches 3:1.
        assert_eq!(l.mark(Color::rgb(0x6C6C6C)), None);
    }
}
