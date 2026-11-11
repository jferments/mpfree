//! Every colour, size and spacing in one place.

use crate::color::Color;

/// What a piece of text or a status means; the theme picks its colour.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tone {
    #[default]
    Normal,
    Dim,
    Accent,
    Good,
    Warn,
    Bad,
}

#[derive(Clone, Debug)]
pub struct Theme {
    /// Behind everything.
    pub bg: Color,
    /// Panels and cards.
    pub surface: Color,
    /// Selected rows, active tabs, pressed controls.
    pub raised: Color,
    pub outline: Color,
    pub text: Color,
    pub text_dim: Color,
    pub text_faint: Color,
    pub accent: Color,
    pub good: Color,
    pub warn: Color,
    pub bad: Color,
    /// Corner radius of panels; controls use a little less.
    pub radius: i32,
    /// Space between panels and around the page.
    pub gap: i32,
    /// Space inside a panel, from its edge to its content.
    pub pad: i32,
    pub size_brand: f32,
    pub size_title: f32,
    pub size_body: f32,
    pub size_small: f32,
    pub size_mono: f32,
}

impl Theme {
    /// The default: dark, for a studio, with one blue accent and status colours.
    pub fn dark() -> Theme {
        Theme {
            bg: Color::rgb(0x0B0D12),
            surface: Color::rgb(0x151922),
            raised: Color::rgb(0x222838),
            outline: Color::rgb(0x2B3245),
            text: Color::rgb(0xE9ECF2),
            text_dim: Color::rgb(0x98A1B3),
            text_faint: Color::rgb(0x5F687B),
            accent: Color::rgb(0x6EA8FE),
            good: Color::rgb(0x3DDC97),
            warn: Color::rgb(0xFFC145),
            bad: Color::rgb(0xFF5C5C),
            radius: 14,
            gap: 12,
            pad: 14,
            size_brand: 28.0,
            size_title: 19.0,
            size_body: 17.0,
            size_small: 14.0,
            size_mono: 14.0,
        }
    }

    pub fn tone(&self, t: Tone) -> Color {
        match t {
            Tone::Normal => self.text,
            Tone::Dim => self.text_dim,
            Tone::Accent => self.accent,
            Tone::Good => self.good,
            Tone::Warn => self.warn,
            Tone::Bad => self.bad,
        }
    }
}

impl Default for Theme {
    fn default() -> Theme {
        Theme::dark()
    }
}
