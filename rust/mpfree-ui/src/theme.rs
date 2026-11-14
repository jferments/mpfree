//! Every colour, size, spacing and effect in one place.
//!
//! A [`Theme`] is a set of design tokens. Widgets ask for what a colour
//! *means* (`surface`, `accent`, `text_dim`), never for a hex value, so a
//! new theme changes the whole UI without touching a widget. The effects
//! (corner shape, glow, backdrop pattern, which face the text uses) are
//! applied underneath the widgets by [`crate::style::Styled`], so widgets do
//! not need to know about them either.
//!
//! Built in: `default` (the original dark studio look), `console` (green on
//! black, glowing mono text), `synthwave` (80s neon on a purple grid) and
//! `spaceship` (black, white and purple, cut corners). Users write their own
//! as a small text file that starts from any of them; see [`Theme::parse`].

use crate::color::Color;
use crate::text::FontId;

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

/// The shape of panels, pads and buttons (anything drawn as a rounded box).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Corners {
    /// Round, with the theme's radius.
    Round,
    /// 45-degree cut corners, as long as the radius. Pills and circles
    /// (a radius of half the box or more) stay round.
    Chamfer,
    /// Square.
    Square,
}

/// What is drawn wherever the background shows (around and between panels,
/// and through translucent panels).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backdrop {
    /// `bg` everywhere.
    Solid,
    /// From `bg` at the top of the screen to `backdrop_to` at the bottom.
    Gradient,
    /// The gradient with lines of `backdrop_line` every `backdrop_step` pixels.
    Grid,
    /// The gradient with a dot of `backdrop_line` every `backdrop_step` pixels.
    Dots,
    /// The gradient with every `backdrop_step`-th row in `backdrop_line`,
    /// like a CRT.
    Scanlines,
}

/// Which startup screen goes with the theme.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Startup {
    /// The spiral galaxy (mpfree-splash).
    Galaxy,
    /// A swirl of green characters that settles into the name.
    Console,
    /// A striped sun over rolling neon hills, the name slashed across it.
    /// The default theme's startup screen (Jay, 2026-11-11, for round 8).
    Synthwave,
    /// A purple galaxy seen from a ship's bridge while its controls boot.
    Spaceship,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    /// Shown in the theme picker.
    pub name: String,
    /// Behind everything.
    pub bg: Color,
    /// Panels and cards. May be translucent to let the backdrop through.
    pub surface: Color,
    /// Selected rows, active tabs, pressed controls.
    pub raised: Color,
    pub outline: Color,
    pub text: Color,
    pub text_dim: Color,
    pub text_faint: Color,
    pub accent: Color,
    /// A second accent for highlights that must differ from `accent`.
    pub accent2: Color,
    /// Text drawn on an `accent` fill.
    pub on_accent: Color,
    pub good: Color,
    pub warn: Color,
    pub bad: Color,
    /// Colours for tracks, synth layers, series in a graph, in order.
    pub track: [Color; 8],
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
    pub corners: Corners,
    /// Glow around outlines, 0 (none) to 255.
    pub glow: u8,
    /// Glow around text, 0 (none) to 255.
    pub text_glow: u8,
    pub backdrop: Backdrop,
    pub backdrop_to: Color,
    pub backdrop_line: Color,
    pub backdrop_step: i32,
    /// The face drawn wherever a widget asks for the regular face.
    pub face_body: FontId,
    /// The face drawn wherever a widget asks for the bold face.
    pub face_title: FontId,
    pub startup: Startup,
    /// How far colours that come from content (kit and pad colours) are
    /// pulled towards `accent` on screen, 0 (as they are) to 255. The pads'
    /// own lights are not changed.
    pub tint: u8,
}

/// The built-in themes, by the name a theme file's `base` uses.
pub const BUILTIN: [&str; 4] = ["default", "console", "synthwave", "spaceship"];

impl Theme {
    /// The default: dark, for a studio, with one blue accent and status colours.
    pub fn dark() -> Theme {
        Theme {
            name: "Default".into(),
            bg: Color::rgb(0x0B0D12),
            surface: Color::rgb(0x151922),
            raised: Color::rgb(0x222838),
            outline: Color::rgb(0x2B3245),
            text: Color::rgb(0xE9ECF2),
            text_dim: Color::rgb(0x98A1B3),
            text_faint: Color::rgb(0x5F687B),
            accent: Color::rgb(0x6EA8FE),
            accent2: Color::rgb(0xD46CFF),
            on_accent: Color::rgb(0x0B0D12),
            good: Color::rgb(0x3DDC97),
            warn: Color::rgb(0xFFC145),
            bad: Color::rgb(0xFF5C5C),
            track: [
                0x6EA8FE, 0x3DDC97, 0xFFC145, 0xD46CFF, 0xFF7A59, 0x4DD8E8, 0xF26BA8, 0xA3E05B,
            ]
            .map(Color::rgb),
            radius: 14,
            gap: 12,
            pad: 14,
            size_brand: 28.0,
            size_title: 19.0,
            size_body: 17.0,
            size_small: 14.0,
            size_mono: 14.0,
            corners: Corners::Round,
            glow: 0,
            text_glow: 0,
            backdrop: Backdrop::Solid,
            // Unused by a solid backdrop; the loading page's sky fades to it.
            backdrop_to: Color::rgb(0x1B2340),
            backdrop_line: Color::rgb(0x151922),
            backdrop_step: 40,
            face_body: FontId::Sans,
            face_title: FontId::Bold,
            startup: Startup::Synthwave,
            tint: 0,
        }
    }

    /// Green on black, glowing like a phosphor screen.
    pub fn console() -> Theme {
        Theme {
            name: "Console".into(),
            bg: Color::rgb(0x020803),
            surface: Color::rgb(0x061409).with_alpha(235),
            raised: Color::rgb(0x0C2A14),
            outline: Color::rgb(0x1A5C2C),
            text: Color::rgb(0x9CFFB0),
            text_dim: Color::rgb(0x3FC066),
            text_faint: Color::rgb(0x217A3C),
            accent: Color::rgb(0x39FF6A),
            accent2: Color::rgb(0xB8FF3C),
            on_accent: Color::rgb(0x021006),
            good: Color::rgb(0x39FF6A),
            warn: Color::rgb(0xE8FF3C),
            bad: Color::rgb(0xFF4D4D),
            track: [
                0x39FF6A, 0xB8FF3C, 0x3CFFD8, 0x8CFF9E, 0xE8FF3C, 0x2BD9A0, 0x6CFF3C, 0xC6FFD2,
            ]
            .map(Color::rgb),
            radius: 4,
            gap: 10,
            pad: 12,
            size_brand: 26.0,
            size_title: 17.0,
            size_body: 16.0,
            size_small: 13.0,
            size_mono: 14.0,
            corners: Corners::Chamfer,
            glow: 150,
            text_glow: 150,
            backdrop: Backdrop::Scanlines,
            backdrop_to: Color::rgb(0x031208),
            backdrop_line: Color::rgb(0x000000),
            backdrop_step: 3,
            face_body: FontId::Sans,
            face_title: FontId::Bold,
            startup: Startup::Console,
            tint: 170,
        }
    }

    /// 80s neon: hot pink and cyan on deep purple, over a perspective-free
    /// grid, everything glowing.
    pub fn synthwave() -> Theme {
        Theme {
            name: "Synthwave".into(),
            bg: Color::rgb(0x12052A),
            surface: Color::rgb(0x1E0B3D).with_alpha(225),
            raised: Color::rgb(0x3A1466),
            outline: Color::rgb(0xFF2E97).with_alpha(170),
            text: Color::rgb(0xFCE7FF),
            text_dim: Color::rgb(0xC39BE0),
            text_faint: Color::rgb(0x7E5AA6),
            accent: Color::rgb(0xFF2E97),
            accent2: Color::rgb(0x2DE2E6),
            on_accent: Color::rgb(0x12052A),
            good: Color::rgb(0x2DE2E6),
            warn: Color::rgb(0xFFB443),
            bad: Color::rgb(0xFF3864),
            track: [
                0xFF2E97, 0x2DE2E6, 0xFFB443, 0xB967FF, 0xF9F871, 0xFF6C11, 0x01CDFE, 0xFF71CE,
            ]
            .map(Color::rgb),
            radius: 10,
            gap: 12,
            pad: 14,
            size_brand: 28.0,
            size_title: 19.0,
            size_body: 17.0,
            size_small: 14.0,
            size_mono: 14.0,
            corners: Corners::Round,
            glow: 190,
            text_glow: 0,
            backdrop: Backdrop::Grid,
            backdrop_to: Color::rgb(0x2B0B4A),
            backdrop_line: Color::rgb(0xFF2E97).with_alpha(46),
            backdrop_step: 40,
            face_body: FontId::Sans,
            face_title: FontId::Bold,
            startup: Startup::Synthwave,
            tint: 70,
        }
    }

    /// A dark ship's console: black, white and violet, cut corners, thin
    /// lines and a faint dot grid.
    pub fn spaceship() -> Theme {
        Theme {
            name: "Spaceship".into(),
            bg: Color::rgb(0x07060C),
            surface: Color::rgb(0x110E1C).with_alpha(240),
            raised: Color::rgb(0x2A1F4A),
            outline: Color::rgb(0x3D2F6B),
            text: Color::rgb(0xF2EEFF),
            text_dim: Color::rgb(0xA99CD6),
            text_faint: Color::rgb(0x5E5385),
            accent: Color::rgb(0xA78BFA),
            accent2: Color::rgb(0xFFFFFF),
            on_accent: Color::rgb(0x07060C),
            good: Color::rgb(0x8BE9C8),
            warn: Color::rgb(0xF5C97A),
            bad: Color::rgb(0xFF6B8B),
            track: [
                0xA78BFA, 0xFFFFFF, 0x7C5CFF, 0xD4C6FF, 0x5B8CFF, 0xE58BFA, 0x8BE9C8, 0xC0B5E8,
            ]
            .map(Color::rgb),
            radius: 10,
            gap: 12,
            pad: 14,
            size_brand: 28.0,
            size_title: 19.0,
            size_body: 17.0,
            size_small: 14.0,
            size_mono: 14.0,
            corners: Corners::Chamfer,
            glow: 90,
            text_glow: 0,
            backdrop: Backdrop::Dots,
            backdrop_to: Color::rgb(0x0E0A1C),
            backdrop_line: Color::rgb(0x2E2552),
            backdrop_step: 24,
            face_body: FontId::Sans,
            face_title: FontId::Bold,
            startup: Startup::Spaceship,
            tint: 110,
        }
    }

    /// A built-in theme by name (see [`BUILTIN`]).
    pub fn builtin(name: &str) -> Option<Theme> {
        match name.trim().to_ascii_lowercase().as_str() {
            "default" | "dark" => Some(Theme::dark()),
            "console" => Some(Theme::console()),
            "synthwave" => Some(Theme::synthwave()),
            "spaceship" => Some(Theme::spaceship()),
            _ => None,
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

    /// A content colour (a kit's or a pad's) as this theme shows it.
    pub fn content(&self, c: Color) -> Color {
        if self.tint == 0 {
            return c;
        }
        // Keep the colour's brightness, take the accent's hue by `tint`.
        let l = |c: Color| (c.r as u32 * 3 + c.g as u32 * 6 + c.b as u32) / 10;
        let (lc, la) = (l(c).max(1), l(self.accent).max(1));
        let k = |v: u8| ((v as u32 * lc / la).min(255)) as u8;
        let toned = Color {
            r: k(self.accent.r),
            g: k(self.accent.g),
            b: k(self.accent.b),
            a: c.a,
        };
        c.mix(toned, self.tint as f32 / 255.0)
    }

    /// Colour `i` of [`Theme::track`], wrapping round.
    pub fn track(&self, i: usize) -> Color {
        self.track[i % self.track.len()]
    }

    /// A theme from a theme file: `key = value` lines, `#` comments. The
    /// first setting may be `base = NAME` (a built-in theme, `default` if
    /// left out); every other key changes one token of it, so a file only
    /// lists what it changes:
    ///
    /// ```text
    /// name = Midnight
    /// base = synthwave
    /// accent = #FF6C11
    /// corners = chamfer
    /// backdrop = dots
    /// ```
    ///
    /// Colours are `#RRGGBB` or `#RRGGBBAA`; `track` takes up to eight.
    /// Unknown keys and bad values are errors that name the line.
    pub fn parse(text: &str) -> Result<Theme, String> {
        let mut theme = Theme::dark();
        let mut first = true;
        for (n, raw) in text.lines().enumerate() {
            let line = strip_comment(raw).trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let at = |e: String| format!("theme line {}: {e}", n + 1);
            let (k, v) = line
                .split_once('=')
                .ok_or_else(|| at(format!("expected key = value, got {line:?}")))?;
            let (k, v) = (k.trim(), v.trim());
            if k == "base" {
                if !first {
                    return Err(at("base must come first".into()));
                }
                theme = Theme::builtin(v).ok_or_else(|| {
                    at(format!(
                        "no built-in theme {v:?} (try {})",
                        BUILTIN.join(", ")
                    ))
                })?;
            } else {
                theme.set(k, v).map_err(at)?;
            }
            first = false;
        }
        Ok(theme)
    }

    /// Change one token by its theme-file key.
    pub fn set(&mut self, key: &str, v: &str) -> Result<(), String> {
        let num = |v: &str| {
            v.parse::<i32>()
                .map_err(|_| format!("{key}: expected a whole number, got {v:?}"))
        };
        let size = |v: &str| match v.parse::<f32>() {
            Ok(x) if (6.0..=200.0).contains(&x) => Ok(x),
            _ => Err(format!("{key}: expected a size from 6 to 200, got {v:?}")),
        };
        let byte = |v: &str| {
            v.parse::<u8>()
                .map_err(|_| format!("{key}: expected 0 to 255, got {v:?}"))
        };
        let col = |v: &str| parse_color(v).map_err(|e| format!("{key}: {e}"));
        let face = |v: &str| match v {
            "sans" => Ok(FontId::Sans),
            "bold" => Ok(FontId::Bold),
            "mono" => Ok(FontId::Mono),
            _ => Err(format!("{key}: expected sans, bold or mono, got {v:?}")),
        };
        match key {
            "name" => self.name = v.to_string(),
            "bg" => self.bg = col(v)?,
            "surface" => self.surface = col(v)?,
            "raised" => self.raised = col(v)?,
            "outline" => self.outline = col(v)?,
            "text" => self.text = col(v)?,
            "text_dim" => self.text_dim = col(v)?,
            "text_faint" => self.text_faint = col(v)?,
            "accent" => self.accent = col(v)?,
            "accent2" => self.accent2 = col(v)?,
            "on_accent" => self.on_accent = col(v)?,
            "good" => self.good = col(v)?,
            "warn" => self.warn = col(v)?,
            "bad" => self.bad = col(v)?,
            "track" => {
                let cs = v
                    .split_whitespace()
                    .map(col)
                    .collect::<Result<Vec<_>, _>>()?;
                if cs.is_empty() || cs.len() > 8 {
                    return Err("track: give 1 to 8 colours".into());
                }
                for i in 0..8 {
                    self.track[i] = cs[i % cs.len()];
                }
            }
            "radius" => self.radius = num(v)?.clamp(0, 40),
            "gap" => self.gap = num(v)?.clamp(0, 40),
            "pad" => self.pad = num(v)?.clamp(0, 40),
            "size_brand" => self.size_brand = size(v)?,
            "size_title" => self.size_title = size(v)?,
            "size_body" => self.size_body = size(v)?,
            "size_small" => self.size_small = size(v)?,
            "size_mono" => self.size_mono = size(v)?,
            "corners" => {
                self.corners = match v {
                    "round" => Corners::Round,
                    "chamfer" => Corners::Chamfer,
                    "square" => Corners::Square,
                    _ => {
                        return Err(format!(
                            "corners: expected round, chamfer or square, got {v:?}"
                        ))
                    }
                }
            }
            "glow" => self.glow = byte(v)?,
            "tint" => self.tint = byte(v)?,
            "text_glow" => self.text_glow = byte(v)?,
            "backdrop" => {
                self.backdrop = match v {
                    "solid" => Backdrop::Solid,
                    "gradient" => Backdrop::Gradient,
                    "grid" => Backdrop::Grid,
                    "dots" => Backdrop::Dots,
                    "scanlines" => Backdrop::Scanlines,
                    _ => {
                        return Err(format!(
                        "backdrop: expected solid, gradient, grid, dots or scanlines, got {v:?}"
                    ))
                    }
                }
            }
            "backdrop_to" => self.backdrop_to = col(v)?,
            "backdrop_line" => self.backdrop_line = col(v)?,
            "backdrop_step" => self.backdrop_step = num(v)?.clamp(2, 400),
            "face_body" => self.face_body = face(v)?,
            "face_title" => self.face_title = face(v)?,
            "startup" => {
                self.startup = match v {
                    "galaxy" => Startup::Galaxy,
                    "console" => Startup::Console,
                    "synthwave" => Startup::Synthwave,
                    "spaceship" => Startup::Spaceship,
                    _ => {
                        return Err(format!(
                            "startup: expected galaxy, console, synthwave or spaceship, got {v:?}"
                        ))
                    }
                }
            }
            _ => return Err(format!("unknown key {key:?}")),
        }
        Ok(())
    }

    /// The whole theme as a theme file, every key written out: a starting
    /// point for a user's own theme. [`Theme::parse`] reads it back exactly.
    pub fn to_text(&self) -> String {
        let c = |c: Color| {
            if c.a == 255 {
                format!("#{:06X}", c.xrgb())
            } else {
                format!("#{:06X}{:02X}", c.xrgb(), c.a)
            }
        };
        let face = |f: FontId| match f {
            FontId::Sans => "sans",
            FontId::Bold => "bold",
            FontId::Mono => "mono",
        };
        let mut s = String::new();
        let mut kv = |k: &str, v: String| {
            s.push_str(&format!("{k:<13} = {v}\n"));
        };
        kv("name", self.name.clone());
        for (k, v) in [
            ("bg", self.bg),
            ("surface", self.surface),
            ("raised", self.raised),
            ("outline", self.outline),
            ("text", self.text),
            ("text_dim", self.text_dim),
            ("text_faint", self.text_faint),
            ("accent", self.accent),
            ("accent2", self.accent2),
            ("on_accent", self.on_accent),
            ("good", self.good),
            ("warn", self.warn),
            ("bad", self.bad),
        ] {
            kv(k, c(v));
        }
        kv(
            "track",
            self.track
                .iter()
                .map(|&t| c(t))
                .collect::<Vec<_>>()
                .join(" "),
        );
        for (k, v) in [
            ("radius", self.radius),
            ("gap", self.gap),
            ("pad", self.pad),
        ] {
            kv(k, v.to_string());
        }
        for (k, v) in [
            ("size_brand", self.size_brand),
            ("size_title", self.size_title),
            ("size_body", self.size_body),
            ("size_small", self.size_small),
            ("size_mono", self.size_mono),
        ] {
            kv(k, format!("{v}"));
        }
        kv("corners", format!("{:?}", self.corners).to_lowercase());
        kv("glow", self.glow.to_string());
        kv("tint", self.tint.to_string());
        kv("text_glow", self.text_glow.to_string());
        kv("backdrop", format!("{:?}", self.backdrop).to_lowercase());
        kv("backdrop_to", c(self.backdrop_to));
        kv("backdrop_line", c(self.backdrop_line));
        kv("backdrop_step", self.backdrop_step.to_string());
        kv("face_body", face(self.face_body).into());
        kv("face_title", face(self.face_title).into());
        kv("startup", format!("{:?}", self.startup).to_lowercase());
        s
    }

    /// The ink for a button's label on a `raised` fill: `text` when the
    /// button can be used, `text_dim` when it can't. Never `text_faint`,
    /// which is too close to `raised` to read ([`Theme::check`] holds
    /// both at 4.5:1 or better).
    pub fn button_ink(&self, usable: bool) -> Color {
        if usable {
            self.text
        } else {
            self.text_dim
        }
    }

    /// Readability problems: text that is too close to what it sits on
    /// (WCAG contrast below 4.5:1 for text, 3:1 for dim text and accents).
    /// A user theme can still be used; the picker shows these as warnings.
    pub fn check(&self) -> Vec<String> {
        let mut out = Vec::new();
        let on = |c: Color, under: Color| {
            if c.a == 255 {
                c
            } else {
                under.mix(c.with_alpha(255), c.a as f32 / 255.0)
            }
        };
        let surface = on(self.surface, self.bg);
        for (what, fg, min) in [
            ("text", self.text, 4.5),
            ("text_dim", self.text_dim, 3.0),
            ("accent", self.accent, 3.0),
        ] {
            for (bgname, bg) in [("bg", self.bg), ("surface", surface)] {
                let r = contrast(fg, bg);
                if r < min {
                    out.push(format!(
                        "{what} on {bgname}: contrast {r:.1}:1, below {min}:1"
                    ));
                }
            }
        }
        // Button labels sit on `raised`: both inks a label may use need
        // 4.5:1 there (Jay after live test 3: "light gray on a medium
        // gray button is no bueno"). `text_faint` is not for labels.
        for (what, fg) in [("text", self.text), ("text_dim", self.text_dim)] {
            let r = contrast(fg, self.raised);
            if r < 4.5 {
                out.push(format!(
                    "{what} on raised (button labels): contrast {r:.1}:1, below 4.5:1"
                ));
            }
        }
        let r = contrast(self.on_accent, self.accent);
        if r < 3.0 {
            out.push(format!("on_accent on accent: contrast {r:.1}:1, below 3:1"));
        }
        out
    }
}

impl Default for Theme {
    fn default() -> Theme {
        Theme::dark()
    }
}

/// A line without its comment: `#` at the start, or `#` followed by a space
/// or the end of the line (so colours like `#FF2E97` are kept).
fn strip_comment(line: &str) -> &str {
    let b = line.as_bytes();
    for (i, &c) in b.iter().enumerate() {
        if c == b'#'
            && (line[..i].trim().is_empty() || b.get(i + 1).is_none_or(|n| n.is_ascii_whitespace()))
        {
            return &line[..i];
        }
    }
    line
}

/// `#RRGGBB` or `#RRGGBBAA`.
pub fn parse_color(v: &str) -> Result<Color, String> {
    let h = v
        .strip_prefix('#')
        .ok_or_else(|| format!("expected #RRGGBB, got {v:?}"))?;
    let n = u32::from_str_radix(h, 16).map_err(|_| format!("expected #RRGGBB, got {v:?}"))?;
    match h.len() {
        6 => Ok(Color::rgb(n)),
        8 => Ok(Color::rgb(n >> 8).with_alpha(n as u8)),
        _ => Err(format!("expected #RRGGBB or #RRGGBBAA, got {v:?}")),
    }
}

/// WCAG contrast ratio between two opaque colours (1 to 21).
pub fn contrast(a: Color, b: Color) -> f32 {
    fn lum(c: Color) -> f32 {
        let ch = |v: u8| {
            let v = v as f32 / 255.0;
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * ch(c.r) + 0.7152 * ch(c.g) + 0.0722 * ch(c.b)
    }
    let (x, y) = (lum(a), lum(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_round_trip_through_text() {
        for name in BUILTIN {
            let t = Theme::builtin(name).unwrap();
            let back = Theme::parse(&t.to_text()).unwrap();
            assert_eq!(back, t, "{name}");
        }
    }

    #[test]
    fn the_theme_files_match_the_builtins() {
        let files = [
            ("default", include_str!("../themes/default.theme")),
            ("console", include_str!("../themes/console.theme")),
            ("synthwave", include_str!("../themes/synthwave.theme")),
            ("spaceship", include_str!("../themes/spaceship.theme")),
        ];
        for (name, text) in files {
            assert_eq!(
                Theme::parse(text).unwrap(),
                Theme::builtin(name).unwrap(),
                "themes/{name}.theme is out of date"
            );
        }
        let ex = Theme::parse(include_str!("../themes/example-sunrise.theme")).unwrap();
        assert_eq!(ex.name, "Sunrise");
        assert!(ex.check().is_empty(), "{:?}", ex.check());
    }

    #[test]
    fn builtins_are_readable() {
        for name in BUILTIN {
            let t = Theme::builtin(name).unwrap();
            assert!(t.check().is_empty(), "{name}: {:?}", t.check());
        }
    }

    #[test]
    fn a_file_lists_only_what_it_changes() {
        let t = Theme::parse("# mine\n#no space\nbase = synthwave\nname = Midnight\naccent = #FF6C11  # orange\ncorners = chamfer\n")
            .unwrap();
        assert_eq!(t.name, "Midnight");
        assert_eq!(t.accent, Color::rgb(0xFF6C11));
        assert_eq!(t.corners, Corners::Chamfer);
        assert_eq!(t.accent2, Theme::synthwave().accent2);
        assert_eq!(
            Theme::parse("name = Mine\nbase = console").unwrap_err(),
            "theme line 2: base must come first"
        );
    }

    #[test]
    fn mistakes_name_the_line() {
        assert_eq!(
            Theme::parse("accent = blue").unwrap_err(),
            "theme line 1: accent: expected #RRGGBB, got \"blue\""
        );
        assert_eq!(
            Theme::parse("\nfoo = 1").unwrap_err(),
            "theme line 2: unknown key \"foo\""
        );
        assert!(Theme::parse("base = vaporwave")
            .unwrap_err()
            .contains("no built-in theme"));
        assert_eq!(
            Theme::parse("surface = #11223380").unwrap().surface,
            Color::rgb(0x112233).with_alpha(0x80)
        );
    }

    #[test]
    fn low_contrast_is_reported() {
        let t = Theme::parse("text = #151922").unwrap();
        assert!(
            t.check().iter().any(|w| w.starts_with("text on surface")),
            "{:?}",
            t.check()
        );
    }
}
