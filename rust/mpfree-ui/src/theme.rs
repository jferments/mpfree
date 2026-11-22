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
use crate::roles::{hex, ColorKey, Expr, Mode, Role, ROLES, ROLE_COUNT};
use crate::text::FontId;
use std::collections::HashMap;

/// What a piece of text or a status means; the theme picks its colour.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tone {
    #[default]
    Normal,
    Dim,
    Accent,
    /// The theme's second accent (the Perform chip, sequencer S4).
    Accent2,
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
    /// Light or dark, for `light_dark()` rules ([`Mode::Auto`] works it
    /// out from `bg` and `text`).
    pub mode: Mode,
    /// Each role's colour, worked out from `sources` ([`Theme::role`]).
    pub(crate) roles: [Color; ROLE_COUNT],
    /// Each role's rule or colour: the table's rule unless the theme sets
    /// its own.
    sources: Vec<Expr>,
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
            mode: Mode::Auto,
            roles: [Color::rgb(0); ROLE_COUNT],
            sources: Vec::new(),
        }
        .with_roles(Theme::DEFAULT_ROLES)
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
            mode: Mode::Auto,
            roles: [Color::rgb(0); ROLE_COUNT],
            sources: Vec::new(),
        }
        .with_roles(Theme::FIXED_ROLES)
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
            mode: Mode::Auto,
            roles: [Color::rgb(0); ROLE_COUNT],
            sources: Vec::new(),
        }
        .with_roles(Theme::FIXED_ROLES)
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
            mode: Mode::Auto,
            roles: [Color::rgb(0); ROLE_COUNT],
            sources: Vec::new(),
        }
        .with_roles(Theme::FIXED_ROLES)
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
            Tone::Accent2 => self.accent2,
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

    /// The roles every built-in theme sets to the colours its screens
    /// drew before roles existed (phase 0 of the theme plan keeps every
    /// built-in theme pixel for pixel the same): the sampler's record red,
    /// the EQ's band colours, and the white flash and black shadow of the
    /// Play page's pads. The default theme's Q-Link colours are its first
    /// four track colours, which its rule gives anyway.
    const DEFAULT_ROLES: &'static [(Role, u32)] = &[
        (Role::Record, 0xFF4D5E),
        (Role::Highlight, 0xFFFFFF),
        (Role::Shadow, 0x000000),
        (Role::Series1, 0x6EA8FE),
        (Role::Series2, 0x3DDC97),
        (Role::Series3, 0xFFC145),
        (Role::Series4, 0xD46CFF),
        (Role::Series5, 0xFF8A65),
        (Role::Series6, 0x4FC3C9),
        (Role::Series7, 0xF06292),
        (Role::Series8, 0xAED581),
    ];

    /// The other built-in themes: the same, and the default theme's Q-Link
    /// colours, which every theme drew until now.
    const FIXED_ROLES: &'static [(Role, u32)] = &[
        (Role::Record, 0xFF4D5E),
        (Role::Highlight, 0xFFFFFF),
        (Role::Shadow, 0x000000),
        (Role::Series1, 0x6EA8FE),
        (Role::Series2, 0x3DDC97),
        (Role::Series3, 0xFFC145),
        (Role::Series4, 0xD46CFF),
        (Role::Series5, 0xFF8A65),
        (Role::Series6, 0x4FC3C9),
        (Role::Series7, 0xF06292),
        (Role::Series8, 0xAED581),
        (Role::Qlink1, 0x6EA8FE),
        (Role::Qlink2, 0x3DDC97),
        (Role::Qlink3, 0xFFC145),
        (Role::Qlink4, 0xD46CFF),
    ];

    /// The theme with every role on its table rule except `fixed`, worked
    /// out.
    fn with_roles(mut self, fixed: &[(Role, u32)]) -> Theme {
        self.sources = Role::ALL.iter().map(|r| r.rule()).collect();
        for &(r, c) in fixed {
            self.sources[r as usize] = Expr::Hex(Color::rgb(c));
        }
        self.resolve_roles();
        self
    }

    /// A role's rule or colour as this theme has it.
    pub fn role_source(&self, r: Role) -> &Expr {
        &self.sources[r as usize]
    }

    /// Sets a role to a colour or rule and works every role out again.
    /// A rule that would go round in a circle is refused.
    pub fn set_role(&mut self, r: Role, e: Expr) -> Result<(), String> {
        let old = std::mem::replace(&mut self.sources[r as usize], e);
        let problems = self.resolve_roles();
        if let Some((_, p)) = problems.into_iter().next() {
            self.sources[r as usize] = old;
            self.resolve_roles();
            return Err(p);
        }
        Ok(())
    }

    /// Works out every role from its source. A role in a circle of rules
    /// takes its table rule instead (the table's rules only read core
    /// colours, so that always works); each one that did is returned with
    /// why.
    fn resolve_roles(&mut self) -> Vec<(Role, String)> {
        let mut sources: HashMap<ColorKey, Expr> = HashMap::new();
        for &r in Role::ALL {
            sources.insert(ColorKey::Role(r), self.sources[r as usize].clone());
        }
        let light = self.is_light();
        let (got, cycles) = resolve(self, &sources, light);
        let mut problems = Vec::new();
        for &r in Role::ALL {
            let k = ColorKey::Role(r);
            self.roles[r as usize] = match got.get(&k) {
                Some(&c) => c,
                None => {
                    let rule = r.rule();
                    problems.push((
                        r,
                        format!(
                            "{}: {}; used its usual rule {rule}",
                            r.def().key,
                            cycles.get(&k).cloned().unwrap_or_default()
                        ),
                    ));
                    rule.eval(light, &mut |k| match k {
                        ColorKey::Role(_) => self.text,
                        k => self.color(k),
                    })
                }
            };
        }
        problems
    }

    /// A theme from a theme file: `key = value` lines, `#` comments. The
    /// first setting may be `base = NAME` (a built-in theme, `default` if
    /// left out); every other key changes one colour, role or effect of
    /// it, so a file only lists what it changes:
    ///
    /// ```text
    /// name = Midnight
    /// base = synthwave
    /// accent = #FF6C11
    /// accent2 = mix({accent}, #FFFFFF, 30%)
    /// corners = chamfer
    /// backdrop = dots
    /// ```
    ///
    /// Colours are `#RRGGBB` or `#RRGGBBAA`, or follow and mix others
    /// (see [`crate::roles`]); `track` takes up to eight, and `track.N`
    /// sets one. This is the strict reading for the app's themes folder:
    /// any mistake or unknown key is an error that names the line (see
    /// [`Theme::load`] for the forgiving one).
    pub fn parse(text: &str) -> Result<Theme, String> {
        let (theme, problems) = Theme::load(text);
        match problems
            .into_iter()
            .find(|p| p.kind != ProblemKind::HardToRead)
        {
            Some(p) => Err(match p.line {
                Some(n) => format!("theme line {n}: {}", p.text),
                None => p.text,
            }),
            None => Ok(theme),
        }
    }

    /// A theme from a theme file, never failing: a line with a mistake
    /// leaves what it would have changed as the base theme has it, an
    /// unknown key (from a newer MPFree, say) is skipped, and each is
    /// listed by line with what was used instead, followed by the theme's
    /// readability warnings ([`Theme::check`]).
    pub fn load(text: &str) -> (Theme, Vec<Problem>) {
        let mut problems = Vec::new();
        let mistake = |line: usize, text: String, used: Option<String>| Problem {
            line: Some(line),
            kind: ProblemKind::Mistake,
            text,
            used,
        };
        // Every setting with its line number, comments gone.
        let mut lines = Vec::new();
        for (n, raw) in text.lines().enumerate() {
            let line = strip_comment(raw).trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            match line.split_once('=') {
                Some((k, v)) => lines.push((n + 1, k.trim(), v.trim())),
                None => problems.push(mistake(
                    n + 1,
                    format!("expected key = value, got {line:?}"),
                    Some("skipped the line".into()),
                )),
            }
        }
        let mut theme = Theme::dark();
        let mut base_name = "default".to_string();
        if let Some(&(n, "base", v)) = lines.first() {
            match Theme::builtin(v) {
                Some(t) => {
                    theme = t;
                    base_name = v.trim().to_ascii_lowercase();
                }
                None => problems.push(mistake(
                    n,
                    format!("no built-in theme {v:?} (try {})", BUILTIN.join(", ")),
                    Some("started from default".into()),
                )),
            }
        }
        // Colours wait until every line is read (one may follow another
        // set further down); everything else is set as it comes.
        let mut colours: HashMap<ColorKey, (usize, Expr)> = HashMap::new();
        for (i, &(n, k, v)) in lines.iter().enumerate() {
            if k == "base" {
                if i > 0 {
                    problems.push(mistake(
                        n,
                        "base must come first".into(),
                        Some(format!("kept {base_name} as the base")),
                    ));
                }
                continue;
            }
            if k == "track" {
                match split_top(v)
                    .iter()
                    .map(|c| Expr::parse(c))
                    .collect::<Result<Vec<_>, _>>()
                {
                    Ok(cs) if !cs.is_empty() && cs.len() <= 8 => {
                        for i in 0..8 {
                            colours.insert(ColorKey::Track(i as u8), (n, cs[i % cs.len()].clone()));
                        }
                    }
                    Ok(_) => problems.push(mistake(
                        n,
                        "track: give 1 to 8 colours".into(),
                        Some(format!("kept {base_name}'s track colours")),
                    )),
                    Err(e) => problems.push(mistake(
                        n,
                        format!("track: {e}"),
                        Some(format!("kept {base_name}'s track colours")),
                    )),
                }
                continue;
            }
            if let Some(key) = ColorKey::from_name(k) {
                match Expr::parse(v) {
                    Ok(e) => {
                        colours.insert(key, (n, e));
                    }
                    Err(e) => problems.push(mistake(
                        n,
                        format!("{k}: {e}"),
                        Some(format!("kept {base_name}'s {}", hex(theme.color(key)))),
                    )),
                }
                continue;
            }
            let before = theme.clone();
            match theme.set_plain(k, v) {
                Ok(()) => {}
                Err(SetError::Unknown) => problems.push(Problem {
                    line: Some(n),
                    kind: ProblemKind::Unknown,
                    text: format!("unknown key {k:?}"),
                    used: Some("skipped the line".into()),
                }),
                Err(SetError::Bad(e)) => {
                    theme = before;
                    problems.push(mistake(n, e, Some(format!("kept {base_name}'s value"))));
                }
            }
        }
        // Each colour from the file, or the base theme's.
        let mut sources: HashMap<ColorKey, Expr> = HashMap::new();
        for k in ColorKey::all() {
            let e = match (colours.get(&k), k) {
                (Some((_, e)), _) => e.clone(),
                (None, ColorKey::Role(r)) => theme.sources[r as usize].clone(),
                (None, k) => Expr::Hex(theme.color(k)),
            };
            sources.insert(k, e);
        }
        // Worked out once on the base's light or dark, then again if the
        // file's own bg and text turned it round (mode auto).
        let mut light = theme.is_light();
        let (mut got, mut cycles) = resolve(&theme, &sources, light);
        let mut probe = theme.clone();
        apply(&mut probe, &got);
        if probe.is_light() != light {
            light = probe.is_light();
            (got, cycles) = resolve(&theme, &sources, light);
        }
        let mut by_line: Vec<(usize, String)> = Vec::new();
        for (k, why) in &cycles {
            let n = colours.get(k).map_or(0, |(n, _)| *n);
            by_line.push((n, format!("{}: {why}", k.name())));
        }
        by_line.sort();
        for (n, text) in by_line {
            problems.push(mistake(n, text, Some(format!("kept {base_name}'s colour"))));
        }
        apply(&mut theme, &got);
        for &r in Role::ALL {
            let k = ColorKey::Role(r);
            if let (Some((_, e)), true) = (colours.get(&k), got.contains_key(&k)) {
                theme.sources[r as usize] = e.clone();
            }
        }
        theme.resolve_roles();
        problems.sort_by_key(|p| p.line);
        for w in theme.check() {
            problems.push(Problem {
                line: None,
                kind: ProblemKind::HardToRead,
                text: w,
                used: None,
            });
        }
        (theme, problems)
    }

    /// Change one setting by its theme-file key: a colour or role (any
    /// colour expression, read against the theme as it is) or an effect.
    pub fn set(&mut self, key: &str, v: &str) -> Result<(), String> {
        if key == "track" {
            let cs = split_top(v)
                .iter()
                .map(|c| Expr::parse(c).map(|e| self.eval(&e)))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("track: {e}"))?;
            if cs.is_empty() || cs.len() > 8 {
                return Err("track: give 1 to 8 colours".into());
            }
            for i in 0..8 {
                self.track[i] = cs[i % cs.len()];
            }
            self.resolve_roles();
            return Ok(());
        }
        if let Some(k) = ColorKey::from_name(key) {
            let e = Expr::parse(v).map_err(|e| format!("{key}: {e}"))?;
            return match k {
                ColorKey::Role(r) => self.set_role(r, e).map_err(|e| format!("{key}: {e}")),
                k => {
                    let c = self.eval(&e);
                    set_core(self, k, c);
                    self.resolve_roles();
                    Ok(())
                }
            };
        }
        match self.set_plain(key, v) {
            Ok(()) => Ok(()),
            Err(SetError::Unknown) => Err(format!("unknown key {key:?}")),
            Err(SetError::Bad(e)) => Err(e),
        }
    }

    /// `e`'s colour on this theme as it is.
    pub fn eval(&self, e: &Expr) -> Color {
        e.eval(self.is_light(), &mut |k| self.color(k))
    }

    /// Change one setting that isn't a colour.
    fn set_plain(&mut self, key: &str, v: &str) -> Result<(), SetError> {
        let bad = SetError::Bad;
        let num = |v: &str| {
            v.parse::<i32>()
                .map_err(|_| bad(format!("{key}: expected a whole number, got {v:?}")))
        };
        let size = |v: &str| match v.parse::<f32>() {
            Ok(x) if (6.0..=200.0).contains(&x) => Ok(x),
            _ => Err(bad(format!(
                "{key}: expected a size from 6 to 200, got {v:?}"
            ))),
        };
        let byte = |v: &str| {
            v.parse::<u8>()
                .map_err(|_| bad(format!("{key}: expected 0 to 255, got {v:?}")))
        };
        let face = |v: &str| match v {
            "sans" => Ok(FontId::Sans),
            "bold" => Ok(FontId::Bold),
            "mono" => Ok(FontId::Mono),
            _ => Err(bad(format!(
                "{key}: expected sans, bold or mono, got {v:?}"
            ))),
        };
        match key {
            "name" => self.name = v.to_string(),
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
                        return Err(bad(format!(
                            "corners: expected round, chamfer or square, got {v:?}"
                        )))
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
                        return Err(bad(format!(
                        "backdrop: expected solid, gradient, grid, dots or scanlines, got {v:?}"
                    )))
                    }
                }
            }
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
                        return Err(bad(format!(
                            "startup: expected galaxy, console, synthwave or spaceship, got {v:?}"
                        )))
                    }
                }
            }
            "mode" => {
                self.mode = match v {
                    "auto" => Mode::Auto,
                    "light" => Mode::Light,
                    "dark" => Mode::Dark,
                    _ => {
                        return Err(bad(format!(
                            "mode: expected auto, light or dark, got {v:?}"
                        )))
                    }
                };
                self.resolve_roles();
            }
            _ => return Err(SetError::Unknown),
        }
        Ok(())
    }

    /// The whole theme as a theme file, every key written out: a starting
    /// point for a user's own theme. [`Theme::parse`] reads it back exactly.
    pub fn to_text(&self) -> String {
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
            kv(k, hex(v));
        }
        kv(
            "track",
            self.track
                .iter()
                .map(|&t| hex(t))
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
        kv("backdrop_to", hex(self.backdrop_to));
        kv("backdrop_line", hex(self.backdrop_line));
        kv("backdrop_step", self.backdrop_step.to_string());
        kv("face_body", face(self.face_body).into());
        kv("face_title", face(self.face_title).into());
        kv("startup", format!("{:?}", self.startup).to_lowercase());
        kv("mode", format!("{:?}", self.mode).to_lowercase());
        for (d, e) in ROLES.iter().zip(&self.sources) {
            kv(d.key, e.to_string());
        }
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
        // Roles drawn as lines and labels on panels: 3:1, as accents.
        for &r in Role::ALL {
            if !matches!(
                r,
                Role::Record
                    | Role::Qlink1
                    | Role::Qlink2
                    | Role::Qlink3
                    | Role::Qlink4
                    | Role::Series1
                    | Role::Series2
                    | Role::Series3
                    | Role::Series4
                    | Role::Series5
                    | Role::Series6
                    | Role::Series7
                    | Role::Series8
            ) {
                continue;
            }
            let c = contrast(on(self.role(r), surface), surface);
            if c < 3.0 {
                out.push(format!(
                    "{} on surface: contrast {c:.1}:1, below 3:1",
                    r.def().key
                ));
            }
        }
        out
    }
}

/// What went wrong in a theme file ([`Theme::load`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    /// The file's line (from 1), or None for the theme as a whole.
    pub line: Option<usize>,
    pub kind: ProblemKind,
    /// What is wrong, naming the key.
    pub text: String,
    /// What was used instead, when something was.
    pub used: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProblemKind {
    /// A value that can't be used, a circle of rules or a misplaced line.
    Mistake,
    /// A key this MPFree doesn't know.
    Unknown,
    /// Text or lines too close to what they sit on ([`Theme::check`]).
    HardToRead,
}

impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(n) = self.line {
            write!(f, "line {n}: ")?;
        }
        f.write_str(&self.text)?;
        if let Some(u) = &self.used {
            write!(f, " ({u})")?;
        }
        Ok(())
    }
}

enum SetError {
    Unknown,
    Bad(String),
}

/// Splits on spaces outside brackets (a `track` line's colours).
fn split_top(v: &str) -> Vec<String> {
    let (mut out, mut cur, mut depth) = (Vec::new(), String::new(), 0i32);
    for ch in v.chars() {
        match ch {
            '(' | '{' => depth += 1,
            ')' | '}' => depth -= 1,
            _ => {}
        }
        if ch.is_whitespace() && depth <= 0 {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
        } else {
            cur.push(ch);
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Sets one core (non-role) colour.
fn set_core(t: &mut Theme, k: ColorKey, c: Color) {
    match k {
        ColorKey::Bg => t.bg = c,
        ColorKey::Surface => t.surface = c,
        ColorKey::Raised => t.raised = c,
        ColorKey::Outline => t.outline = c,
        ColorKey::Text => t.text = c,
        ColorKey::TextDim => t.text_dim = c,
        ColorKey::TextFaint => t.text_faint = c,
        ColorKey::Accent => t.accent = c,
        ColorKey::Accent2 => t.accent2 = c,
        ColorKey::OnAccent => t.on_accent = c,
        ColorKey::Good => t.good = c,
        ColorKey::Warn => t.warn = c,
        ColorKey::Bad => t.bad = c,
        ColorKey::Track(i) => t.track[i as usize % 8] = c,
        ColorKey::BackdropTo => t.backdrop_to = c,
        ColorKey::BackdropLine => t.backdrop_line = c,
        ColorKey::Role(r) => t.roles[r as usize] = c,
    }
}

/// Every resolved colour into `t` (core fields and role values).
fn apply(t: &mut Theme, got: &HashMap<ColorKey, Color>) {
    for (&k, &c) in got {
        set_core(t, k, c);
    }
}

/// Works out every key in `sources`, each read from its source, the keys
/// it names worked out first; a key missing from `sources` reads `t`'s
/// colour. Keys in a circle are left out of the result and listed with
/// the circle.
fn resolve(
    t: &Theme,
    sources: &HashMap<ColorKey, Expr>,
    light: bool,
) -> (HashMap<ColorKey, Color>, HashMap<ColorKey, String>) {
    #[derive(Clone, Copy, PartialEq)]
    enum State {
        Working,
        Done,
        Failed,
    }
    struct Cx<'a> {
        t: &'a Theme,
        sources: &'a HashMap<ColorKey, Expr>,
        light: bool,
        state: HashMap<ColorKey, State>,
        got: HashMap<ColorKey, Color>,
        failed: HashMap<ColorKey, String>,
        stack: Vec<ColorKey>,
    }
    fn visit(cx: &mut Cx, k: ColorKey) -> bool {
        match cx.state.get(&k) {
            Some(State::Done) => return true,
            Some(State::Failed) => return false,
            Some(State::Working) => {
                let from = cx.stack.iter().position(|&s| s == k).unwrap_or(0);
                let names: Vec<String> = cx.stack[from..]
                    .iter()
                    .chain(std::iter::once(&k))
                    .map(|k| k.name())
                    .collect();
                let why = format!("goes round in a circle ({})", names.join(" → "));
                for &s in &cx.stack[from..] {
                    cx.failed.entry(s).or_insert_with(|| why.clone());
                }
                return false;
            }
            None => {}
        }
        let Some(e) = cx.sources.get(&k) else {
            cx.got.insert(k, cx.t.color(k));
            cx.state.insert(k, State::Done);
            return true;
        };
        cx.state.insert(k, State::Working);
        cx.stack.push(k);
        let mut deps = Vec::new();
        e.keys(&mut deps);
        let mut ok = true;
        for d in deps {
            if !visit(cx, d) {
                ok = false;
                cx.failed
                    .entry(k)
                    .or_insert_with(|| format!("follows {}, which can't be worked out", d.name()));
            }
        }
        cx.stack.pop();
        if ok {
            let got = &cx.got;
            let c = e.eval(cx.light, &mut |d| got.get(&d).copied().unwrap_or(cx.t.text));
            cx.got.insert(k, c);
            cx.state.insert(k, State::Done);
        } else {
            cx.state.insert(k, State::Failed);
        }
        ok
    }
    let mut cx = Cx {
        t,
        sources,
        light,
        state: HashMap::new(),
        got: HashMap::new(),
        failed: HashMap::new(),
        stack: Vec::new(),
    };
    let mut keys: Vec<ColorKey> = sources.keys().copied().collect();
    keys.sort();
    for k in keys {
        visit(&mut cx, k);
    }
    for k in cx.failed.keys() {
        cx.got.remove(k);
    }
    (cx.got, cx.failed)
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
    crate::roles::parse_hex(v)
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
        assert_eq!(ex.outline, Color::rgb(0xFF6C11).with_alpha(0xAA));
        assert_eq!(ex.backdrop_line, Color::rgb(0xFF6C11).with_alpha(0x66));
        assert_eq!(ex.qlink(0), ex.accent);
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
            "theme line 1: accent: expected a colour (#RRGGBB, {name}, mix(), alpha() or light_dark()), got \"blue\""
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

    #[test]
    fn every_built_in_theme_keeps_the_colours_its_screens_drew() {
        // Phase 0 of the theme plan changes no pixel: each role is the
        // colour the screens it replaces drew before.
        for name in BUILTIN {
            let t = Theme::builtin(name).unwrap();
            assert_eq!(t.record(), Color::rgb(0xFF4D5E), "{name}");
            assert_eq!(
                (0..4).map(|i| t.qlink(i)).collect::<Vec<_>>(),
                [0x6EA8FE, 0x3DDC97, 0xFFC145, 0xD46CFF]
                    .map(Color::rgb)
                    .to_vec(),
                "{name}"
            );
            assert_eq!(t.role(Role::TextInverse), t.bg, "{name}");
            assert_eq!(t.role(Role::Highlight), Color::rgb(0xFFFFFF), "{name}");
            assert_eq!(t.role(Role::Shadow), Color::rgb(0), "{name}");
            let eq = [
                0x6EA8FE, 0x3DDC97, 0xFFC145, 0xD46CFF, 0xFF8A65, 0x4FC3C9, 0xF06292, 0xAED581,
            ];
            for (i, c) in eq.iter().enumerate() {
                assert_eq!(t.series(i), Color::rgb(*c), "{name} series {i}");
            }
        }
    }

    #[test]
    fn colours_can_follow_and_mix_others() {
        let t = Theme::parse(
            "base = default\n\
             accent = #FF6C11\n\
             accent2 = {accent}\n\
             outline = alpha({accent}, 40%)\n\
             raised = mix(surface, {text}, 10%)\n\
             track = {accent} mix({good}, #000000, 20%) #112233\n\
             track.8 = {bad}\n",
        )
        .unwrap();
        assert_eq!(t.accent2, Color::rgb(0xFF6C11));
        assert_eq!(t.outline, Color::rgb(0xFF6C11).with_alpha(102));
        assert_eq!(t.raised, t.surface.mix(t.text, 0.1));
        assert_eq!(t.track[0], t.accent);
        assert_eq!(t.track[1], t.good.mix(Color::rgb(0), 0.2));
        assert_eq!(t.track[2], Color::rgb(0x112233));
        assert_eq!(t.track[3], t.accent, "a short list repeats");
        assert_eq!(t.track[7], t.bad, "track.8 wins over the list");
        // A colour may follow one set further down the file.
        let t = Theme::parse("accent2 = {accent}\naccent = #00FF00\n").unwrap();
        assert_eq!(t.accent2, Color::rgb(0x00FF00));
    }

    #[test]
    fn roles_follow_their_rules_until_a_theme_sets_them() {
        // Default's Q-Links are its first four tracks, by rule: new track
        // colours move them too.
        let t = Theme::parse("track = #112233 #445566 #778899 #AABBCC").unwrap();
        assert_eq!(t.qlink(0), Color::rgb(0x112233));
        assert_eq!(t.qlink(3), Color::rgb(0xAABBCC));
        // Inverse text follows bg.
        let t = Theme::parse("bg = #202020").unwrap();
        assert_eq!(t.role(Role::TextInverse), Color::rgb(0x202020));
        // A role set in the file, to a colour or a rule.
        let t = Theme::parse("qlink.2 = #00FF00\nrecord = mix({bad}, {text}, 10%)").unwrap();
        assert_eq!(t.qlink(1), Color::rgb(0x00FF00));
        assert_eq!(t.record(), t.bad.mix(t.text, 0.1));
        // light_dark() follows the mode, worked out from bg and text.
        let light = "bg = #F0F0F0\nsurface = #FFFFFF\nraised = #E0E0E0\ntext = #101010\n\
                     text_dim = #404040\naccent = #2050C0\non_accent = #FFFFFF\n";
        let t = Theme::parse(light).unwrap();
        assert!(t.is_light());
        assert_eq!(
            t.role(Role::Shadow),
            Theme::dark().role(Role::Shadow),
            "kept from base"
        );
        let t = Theme::parse(&format!("{light}shadow = light_dark({{text}}, {{bg}})")).unwrap();
        assert_eq!(t.role(Role::Shadow), t.text);
        let t = Theme::parse(&format!(
            "{light}mode = dark\nshadow = light_dark({{text}}, {{bg}})"
        ))
        .unwrap();
        assert_eq!(t.role(Role::Shadow), t.bg);
    }

    #[test]
    fn a_circle_of_colours_is_a_mistake_that_names_its_lines() {
        let (t, problems) = Theme::load("accent = {accent2}\naccent2 = {accent}\ngood = #00FF00\n");
        let mistakes: Vec<String> = problems
            .iter()
            .filter(|p| p.kind == ProblemKind::Mistake)
            .map(|p| p.to_string())
            .collect();
        assert_eq!(mistakes.len(), 2, "{mistakes:?}");
        assert!(
            mistakes[0].starts_with("line 1: accent: goes round in a circle"),
            "{mistakes:?}"
        );
        assert!(
            mistakes[1].starts_with("line 2: accent2: goes round in a circle"),
            "{mistakes:?}"
        );
        // Both keep the base's colours; the rest of the file still counts.
        assert_eq!(
            (t.accent, t.accent2),
            (Theme::dark().accent, Theme::dark().accent2)
        );
        assert_eq!(t.good, Color::rgb(0x00FF00));
        // A role in a circle takes its usual rule.
        let (t, problems) = Theme::load("qlink.1 = {qlink.2}\nqlink.2 = {qlink.1}\n");
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert_eq!(t.qlink(0), t.track[0]);
        let mut t = Theme::dark();
        assert!(t
            .set("qlink.1", "{qlink.1}")
            .unwrap_err()
            .contains("circle"));
        assert_eq!(t, Theme::dark(), "a refused rule changes nothing");
    }

    #[test]
    fn load_keeps_going_past_mistakes_and_says_what_it_used() {
        let text = "base = synthwave\n\
                    name = Dusk\n\
                    accent = blue\n\
                    glow = lots\n\
                    sparkle = 3\n\
                    good = #00FF00\n\
                    base = console\n\
                    track = #112233 nope\n\
                    just some words\n";
        let (t, problems) = Theme::load(text);
        let lines: Vec<String> = problems.iter().map(|p| p.to_string()).collect();
        assert_eq!(t.name, "Dusk");
        assert_eq!(t.accent, Theme::synthwave().accent);
        assert_eq!(t.glow, Theme::synthwave().glow);
        assert_eq!(t.track, Theme::synthwave().track);
        assert_eq!(t.good, Color::rgb(0x00FF00));
        assert_eq!(
            lines,
            [
                "line 3: accent: expected a colour (#RRGGBB, {name}, mix(), alpha() or light_dark()), got \"blue\" (kept synthwave's #FF2E97)",
                "line 4: glow: expected 0 to 255, got \"lots\" (kept synthwave's value)",
                "line 5: unknown key \"sparkle\" (skipped the line)",
                "line 7: base must come first (kept synthwave as the base)",
                "line 8: track: expected a colour (#RRGGBB, {name}, mix(), alpha() or light_dark()), got \"nope\" (kept synthwave's track colours)",
                "line 9: expected key = value, got \"just some words\" (skipped the line)",
            ]
        );
        assert_eq!(problems[2].kind, ProblemKind::Unknown);
        // The strict reading stops at the first.
        assert!(Theme::parse(text)
            .unwrap_err()
            .starts_with("theme line 3: accent:"));
        // Hard-to-read colours are listed after, as warnings, and don't
        // stop the strict reading.
        let (_, problems) = Theme::load("text = #151922\nqlink.3 = #1A1F2A\n");
        assert!(problems
            .iter()
            .all(|p| p.kind == ProblemKind::HardToRead && p.line.is_none()));
        assert!(
            problems
                .iter()
                .any(|p| p.text.starts_with("text on surface")),
            "{problems:?}"
        );
        assert!(
            problems
                .iter()
                .any(|p| p.text.starts_with("qlink.3 on surface")),
            "{problems:?}"
        );
        assert!(Theme::parse("text = #151922\nqlink.3 = #1A1F2A\n").is_ok());
    }

    #[test]
    fn to_text_writes_roles_as_rules_and_reads_them_back() {
        let t = Theme::parse("accent = #FF6C11\nqlink.1 = mix({accent}, {text}, 25%)").unwrap();
        let text = t.to_text();
        assert!(
            text.contains("qlink.1       = mix({accent}, {text}, 25%)\n"),
            "{text}"
        );
        assert!(text.contains("qlink.2       = {track.2}\n"), "{text}");
        assert!(text.contains("mode          = auto\n"), "{text}");
        assert_eq!(Theme::parse(&text).unwrap(), t);
    }
}
