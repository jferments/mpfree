//! The theme's colour roles, in one table.
//!
//! A theme sets a few core colours (`bg`, `surface`, `text`, `accent` and
//! the rest of [`Theme`]'s colour fields). Everything else a screen paints
//! is a *role*: a named colour with a default rule written in those core
//! colours, such as `text_inverse = {bg}` or `qlink.1 = {track.1}`. A theme
//! file can set any role to a colour or to a rule of its own; a role it
//! leaves alone follows its rule, so it turns the right way on a light or a
//! dark theme with no work from the theme writer.
//!
//! [`ROLES`] is the only list: the theme file's keys, [`Theme::to_text`],
//! the readability check and the manual's role list all come from it, so
//! nothing can drift. Widgets read a role with [`Theme::role`] or one of
//! its helpers ([`Theme::qlink`], [`Theme::series`], [`Theme::record`],
//! [`Theme::ink_on`]).
//!
//! The table grows with the screens that need it: a role is added in the
//! same change as the first drawing code that uses it, never ahead of it.
//!
//! Content colours (a kit's, a pad's, a track's own colour) are not roles:
//! [`Theme::swatch`] turns any of them into a fill, a quiet tint, a line, a
//! text colour and an ink that are readable on this theme.
//!
//! Rules are small colour expressions ([`Expr`]), the same ones a theme
//! file may use:
//!
//! - `#RRGGBB` or `#RRGGBBAA`;
//! - `{name}` or just `name`: another colour (`{accent}`, `{track.3}`,
//!   `{qlink.1}`);
//! - `mix(a, b, N%)`: N % of the way from `a` to `b`;
//! - `alpha(a, N%)`: `a` at N % opacity;
//! - `light_dark(a, b)`: `a` on a light theme, `b` on a dark one (as CSS's
//!   `light-dark()`); the theme's `mode` says which it is.

use crate::color::Color;
use crate::theme::{contrast, Theme};

/// Where a colour sits in the theme writer's guide.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    /// A core colour: set these and a theme is complete.
    Core,
    /// Follows its rule unless a theme sets it.
    Role,
}

/// One row of the role table.
#[derive(Clone, Copy, Debug)]
pub struct RoleDef {
    /// The theme file key, such as `qlink.1`.
    pub key: &'static str,
    /// The heading it is listed under.
    pub group: &'static str,
    pub tier: Tier,
    /// The default rule, an [`Expr`] in theme file syntax.
    pub rule: &'static str,
    /// What it colours, in one line, for the manual.
    pub doc: &'static str,
}

macro_rules! roles {
    ($( $variant:ident $key:literal $group:literal $tier:ident $rule:literal $doc:literal; )*) => {
        /// A colour role (see the module docs and [`ROLES`]).
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum Role {
            $( #[doc = $doc] $variant, )*
        }

        /// Every role, in [`Role`] order.
        pub const ROLES: &[RoleDef] = &[
            $( RoleDef { key: $key, group: $group, tier: Tier::$tier, rule: $rule, doc: $doc }, )*
        ];

        impl Role {
            /// Every role, in table order.
            pub const ALL: &'static [Role] = &[ $( Role::$variant, )* ];
        }
    };
}

roles! {
    Record "record" "Status" Core "{bad}"
        "Recording: the record button, armed inputs and the level while recording.";
    Qlink1 "qlink.1" "Controls" Core "{track.1}"
        "Q-Link knob 1: its ring, its Q1 label and anything it moves.";
    Qlink2 "qlink.2" "Controls" Core "{track.2}"
        "Q-Link knob 2.";
    Qlink3 "qlink.3" "Controls" Core "{track.3}"
        "Q-Link knob 3.";
    Qlink4 "qlink.4" "Controls" Core "{track.4}"
        "Q-Link knob 4.";
    TextInverse "text_inverse" "Text" Role "{bg}"
        "Text on a fill too bright (or too dark) for `text`; ink_on() picks between the two.";
    Highlight "highlight" "Light and shade" Role "{text}"
        "What a pad or button flashes towards when it is hit.";
    Shadow "shadow" "Light and shade" Role "light_dark({text}, {bg})"
        "Drop shadows under lifted things (drawn see-through).";
    Series1 "series.1" "Palettes" Role "{track.1}"
        "Graph lines and EQ bands in order: the first.";
    Series2 "series.2" "Palettes" Role "{track.2}"
        "The second graph line or EQ band.";
    Series3 "series.3" "Palettes" Role "{track.3}"
        "The third.";
    Series4 "series.4" "Palettes" Role "{track.4}"
        "The fourth.";
    Series5 "series.5" "Palettes" Role "{track.5}"
        "The fifth.";
    Series6 "series.6" "Palettes" Role "{track.6}"
        "The sixth.";
    Series7 "series.7" "Palettes" Role "{track.7}"
        "The seventh.";
    Series8 "series.8" "Palettes" Role "{track.8}"
        "The eighth.";
}

/// How many roles there are.
pub const ROLE_COUNT: usize = ROLES.len();

impl Role {
    /// Its row in [`ROLES`].
    pub fn def(self) -> &'static RoleDef {
        &ROLES[self as usize]
    }

    /// The role a theme file key names.
    pub fn from_key(key: &str) -> Option<Role> {
        ROLES
            .iter()
            .position(|d| d.key == key)
            .map(|i| Role::ALL[i])
    }

    /// Its default rule, parsed. The table's rules are checked by a test,
    /// so this never falls back to `text`.
    pub fn rule(self) -> Expr {
        Expr::parse(self.def().rule).unwrap_or(Expr::Key(ColorKey::Text))
    }

    const QLINKS: [Role; 4] = [Role::Qlink1, Role::Qlink2, Role::Qlink3, Role::Qlink4];
    const SERIES: [Role; 8] = [
        Role::Series1,
        Role::Series2,
        Role::Series3,
        Role::Series4,
        Role::Series5,
        Role::Series6,
        Role::Series7,
        Role::Series8,
    ];
}

/// Whether a theme is light (dark text on light panels) or dark. It only
/// steers `light_dark()` rules.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    /// Worked out from `bg` and `text`: light when `bg` is the brighter.
    #[default]
    Auto,
    Light,
    Dark,
}

/// Any colour a theme has, by its theme file name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ColorKey {
    Bg,
    Surface,
    Raised,
    Outline,
    Text,
    TextDim,
    TextFaint,
    Accent,
    Accent2,
    OnAccent,
    Good,
    Warn,
    Bad,
    /// `track.1` to `track.8` (0 to 7 here).
    Track(u8),
    BackdropTo,
    BackdropLine,
    Role(Role),
}

/// The core colours' theme file names, in [`ColorKey`] order (track and
/// the role keys aside).
const CORE: [(&str, ColorKey); 13] = [
    ("bg", ColorKey::Bg),
    ("surface", ColorKey::Surface),
    ("raised", ColorKey::Raised),
    ("outline", ColorKey::Outline),
    ("text", ColorKey::Text),
    ("text_dim", ColorKey::TextDim),
    ("text_faint", ColorKey::TextFaint),
    ("accent", ColorKey::Accent),
    ("accent2", ColorKey::Accent2),
    ("on_accent", ColorKey::OnAccent),
    ("good", ColorKey::Good),
    ("warn", ColorKey::Warn),
    ("bad", ColorKey::Bad),
];

impl ColorKey {
    /// The colour a theme file name means: a core colour, `track.N`
    /// (1 to 8), `backdrop_to`, `backdrop_line` or a role.
    pub fn from_name(name: &str) -> Option<ColorKey> {
        if let Some(&(_, k)) = CORE.iter().find(|(n, _)| *n == name) {
            return Some(k);
        }
        match name {
            "backdrop_to" => return Some(ColorKey::BackdropTo),
            "backdrop_line" => return Some(ColorKey::BackdropLine),
            _ => {}
        }
        if let Some(n) = name.strip_prefix("track.") {
            return match n.parse::<u8>() {
                Ok(i @ 1..=8) => Some(ColorKey::Track(i - 1)),
                _ => None,
            };
        }
        Role::from_key(name).map(ColorKey::Role)
    }

    /// Its theme file name.
    pub fn name(self) -> String {
        match self {
            ColorKey::Track(i) => format!("track.{}", i + 1),
            ColorKey::BackdropTo => "backdrop_to".into(),
            ColorKey::BackdropLine => "backdrop_line".into(),
            ColorKey::Role(r) => r.def().key.into(),
            k => CORE
                .iter()
                .find(|(_, c)| *c == k)
                .map(|(n, _)| n.to_string())
                .unwrap_or_default(),
        }
    }

    /// Every colour key a theme has: the core colours, the tracks, the
    /// backdrop's two and the roles.
    pub fn all() -> Vec<ColorKey> {
        let mut v: Vec<ColorKey> = CORE.iter().map(|&(_, k)| k).collect();
        v.extend((0..8).map(ColorKey::Track));
        v.push(ColorKey::BackdropTo);
        v.push(ColorKey::BackdropLine);
        v.extend(Role::ALL.iter().map(|&r| ColorKey::Role(r)));
        v
    }
}

/// A colour expression: a role's rule, or a theme file's value.
#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Hex(Color),
    Key(ColorKey),
    /// From the first to the second, by this many percent.
    Mix(Box<Expr>, Box<Expr>, f32),
    /// At this opacity, in percent.
    Alpha(Box<Expr>, f32),
    /// The first on a light theme, the second on a dark one.
    LightDark(Box<Expr>, Box<Expr>),
}

impl Expr {
    /// Reads an expression in theme file syntax (see the module docs).
    pub fn parse(text: &str) -> Result<Expr, String> {
        let mut p = Parser {
            s: text.as_bytes(),
            at: 0,
            text,
        };
        let e = p.expr()?;
        p.ws();
        if p.at != p.s.len() {
            return Err(format!(
                "unexpected {:?} after the colour in {text:?}",
                &text[p.at..]
            ));
        }
        Ok(e)
    }

    /// The keys it reads.
    pub fn keys(&self, out: &mut Vec<ColorKey>) {
        match self {
            Expr::Hex(_) => {}
            Expr::Key(k) => out.push(*k),
            Expr::Mix(a, b, _) | Expr::LightDark(a, b) => {
                a.keys(out);
                b.keys(out);
            }
            Expr::Alpha(a, _) => a.keys(out),
        }
    }

    /// Its value, with `get` giving each key's colour and `light` saying
    /// whether the theme is light.
    pub fn eval(&self, light: bool, get: &mut dyn FnMut(ColorKey) -> Color) -> Color {
        match self {
            Expr::Hex(c) => *c,
            Expr::Key(k) => get(*k),
            Expr::Mix(a, b, pct) => {
                let a = a.eval(light, get);
                let b = b.eval(light, get);
                a.mix(b, pct / 100.0)
            }
            Expr::Alpha(a, pct) => {
                let c = a.eval(light, get);
                c.with_alpha(((c.a as f32) * pct / 100.0).round().clamp(0.0, 255.0) as u8)
            }
            Expr::LightDark(a, b) => {
                if light {
                    a.eval(light, get)
                } else {
                    b.eval(light, get)
                }
            }
        }
    }
}

impl std::fmt::Display for Expr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Expr::Hex(c) => f.write_str(&hex(*c)),
            Expr::Key(k) => write!(f, "{{{}}}", k.name()),
            Expr::Mix(a, b, p) => write!(f, "mix({a}, {b}, {p}%)"),
            Expr::Alpha(a, p) => write!(f, "alpha({a}, {p}%)"),
            Expr::LightDark(a, b) => write!(f, "light_dark({a}, {b})"),
        }
    }
}

/// `#RRGGBB`, or `#RRGGBBAA` when see-through.
pub fn hex(c: Color) -> String {
    if c.a == 255 {
        format!("#{:06X}", c.xrgb())
    } else {
        format!("#{:06X}{:02X}", c.xrgb(), c.a)
    }
}

struct Parser<'a> {
    s: &'a [u8],
    at: usize,
    text: &'a str,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.at < self.s.len() && self.s[self.at].is_ascii_whitespace() {
            self.at += 1;
        }
    }

    fn eat(&mut self, c: u8) -> Result<(), String> {
        self.ws();
        if self.s.get(self.at) == Some(&c) {
            self.at += 1;
            Ok(())
        } else {
            Err(format!("expected '{}' in {:?}", c as char, self.text))
        }
    }

    fn word(&mut self) -> &str {
        let start = self.at;
        while self.at < self.s.len()
            && (self.s[self.at].is_ascii_alphanumeric() || b"_.".contains(&self.s[self.at]))
        {
            self.at += 1;
        }
        &self.text[start..self.at]
    }

    fn percent(&mut self) -> Result<f32, String> {
        self.ws();
        let start = self.at;
        while self.at < self.s.len()
            && (self.s[self.at].is_ascii_digit() || self.s[self.at] == b'.')
        {
            self.at += 1;
        }
        let n = &self.text[start..self.at];
        self.eat(b'%')
            .map_err(|_| format!("expected a percentage like 40% in {:?}", self.text))?;
        match n.parse::<f32>() {
            Ok(p) if (0.0..=100.0).contains(&p) => Ok(p),
            _ => Err(format!(
                "expected a percentage from 0% to 100% in {:?}",
                self.text
            )),
        }
    }

    fn key(&mut self, name: &str) -> Result<Expr, String> {
        ColorKey::from_name(name)
            .map(Expr::Key)
            .ok_or_else(|| format!("no colour called {name:?}"))
    }

    fn expr(&mut self) -> Result<Expr, String> {
        self.ws();
        match self.s.get(self.at) {
            Some(b'#') => {
                let start = self.at;
                self.at += 1;
                while self.at < self.s.len() && self.s[self.at].is_ascii_hexdigit() {
                    self.at += 1;
                }
                parse_hex(&self.text[start..self.at]).map(Expr::Hex)
            }
            Some(b'{') => {
                self.at += 1;
                self.ws();
                let name = self.word().to_string();
                self.eat(b'}')?;
                self.key(&name)
            }
            Some(c) if c.is_ascii_alphabetic() => {
                let name = self.word().to_string();
                self.ws();
                if self.s.get(self.at) != Some(&b'(') {
                    return self.key(&name).map_err(|_| {
                        format!(
                            "expected a colour (#RRGGBB, {{name}}, mix(), alpha() or light_dark()), got {:?}",
                            self.text.trim()
                        )
                    });
                }
                self.at += 1;
                let e = match name.as_str() {
                    "mix" => {
                        let a = self.expr()?;
                        self.eat(b',')?;
                        let b = self.expr()?;
                        self.eat(b',')?;
                        let p = self.percent()?;
                        Expr::Mix(Box::new(a), Box::new(b), p)
                    }
                    "alpha" => {
                        let a = self.expr()?;
                        self.eat(b',')?;
                        let p = self.percent()?;
                        Expr::Alpha(Box::new(a), p)
                    }
                    "light_dark" => {
                        let a = self.expr()?;
                        self.eat(b',')?;
                        let b = self.expr()?;
                        Expr::LightDark(Box::new(a), Box::new(b))
                    }
                    _ => {
                        return Err(format!(
                            "no colour function {name:?} (try mix, alpha or light_dark)"
                        ))
                    }
                };
                self.eat(b')')?;
                Ok(e)
            }
            _ => Err(format!(
                "expected a colour (#RRGGBB, {{name}}, mix(), alpha() or light_dark()), got {:?}",
                self.text.trim()
            )),
        }
    }
}

/// `#RRGGBB` or `#RRGGBBAA`.
pub fn parse_hex(v: &str) -> Result<Color, String> {
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

/// A content colour (a kit's, a pad's, a track's own) in the five forms a
/// screen draws it in, all readable on the theme it came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Swatch {
    /// The colour as the theme shows it ([`Theme::content`]), for fills.
    pub fill: Color,
    /// A quiet version on panels, for large areas behind other things.
    pub tint: Color,
    /// For lines and outlines: at least 3:1 on panels.
    pub line: Color,
    /// For text in the colour: at least 4.5:1 on panels.
    pub text: Color,
    /// For text on `fill`: the theme's text or inverse text, whichever
    /// reads better.
    pub ink: Color,
}

/// How much of the colour a [`Swatch::tint`] has over the panel.
pub const TINT: f32 = 0.18;

/// What a widget made before it knew the theme keeps for a colour: a
/// theme colour by name, a content colour (shown through
/// [`Theme::content`]), or, for artwork only, a colour as it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Paint {
    Key(ColorKey),
    Content(Color),
    Literal(Color),
}

impl Paint {
    /// The colour on `t`.
    pub fn on(self, t: &Theme) -> Color {
        match self {
            Paint::Key(k) => t.color(k),
            Paint::Content(c) => t.content(c),
            Paint::Literal(c) => c,
        }
    }
}

impl From<Role> for Paint {
    fn from(r: Role) -> Paint {
        Paint::Key(ColorKey::Role(r))
    }
}

impl From<ColorKey> for Paint {
    fn from(k: ColorKey) -> Paint {
        Paint::Key(k)
    }
}

/// `c` drawn over `under` (opaque).
pub fn over(c: Color, under: Color) -> Color {
    if c.a == 255 {
        c
    } else {
        under
            .with_alpha(255)
            .mix(c.with_alpha(255), c.a as f32 / 255.0)
    }
}

/// `c`, moved towards `toward` just far enough to reach `target` contrast
/// on `on` (or all the way, if even that falls short). A colour that
/// already reaches it comes back unchanged.
pub fn reach(c: Color, toward: Color, on: Color, target: f32) -> Color {
    let c = over(c, on);
    if contrast(c, on) >= target {
        return c;
    }
    let toward = over(toward, on);
    if contrast(toward, on) < target {
        return toward;
    }
    // The smallest step towards `toward` that reaches the target, to 1/64.
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    for _ in 0..6 {
        let mid = (lo + hi) / 2.0;
        if contrast(c.mix(toward, mid), on) >= target {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    c.mix(toward, hi)
}

impl Theme {
    /// Any of the theme's colours by its key.
    pub fn color(&self, k: ColorKey) -> Color {
        match k {
            ColorKey::Bg => self.bg,
            ColorKey::Surface => self.surface,
            ColorKey::Raised => self.raised,
            ColorKey::Outline => self.outline,
            ColorKey::Text => self.text,
            ColorKey::TextDim => self.text_dim,
            ColorKey::TextFaint => self.text_faint,
            ColorKey::Accent => self.accent,
            ColorKey::Accent2 => self.accent2,
            ColorKey::OnAccent => self.on_accent,
            ColorKey::Good => self.good,
            ColorKey::Warn => self.warn,
            ColorKey::Bad => self.bad,
            ColorKey::Track(i) => self.track[i as usize % 8],
            ColorKey::BackdropTo => self.backdrop_to,
            ColorKey::BackdropLine => self.backdrop_line,
            ColorKey::Role(r) => self.role(r),
        }
    }

    /// A role's colour.
    pub fn role(&self, r: Role) -> Color {
        self.roles[r as usize]
    }

    /// Q-Link knob `i`'s colour (0 for knob 1), wrapping round.
    pub fn qlink(&self, i: usize) -> Color {
        self.role(Role::QLINKS[i % 4])
    }

    /// Graph line or EQ band `i`'s colour (0 first), wrapping round.
    pub fn series(&self, i: usize) -> Color {
        self.role(Role::SERIES[i % 8])
    }

    /// The recording colour.
    pub fn record(&self) -> Color {
        self.role(Role::Record)
    }

    /// Whether the theme is light: its `mode`, or, left on auto, whether
    /// `bg` is brighter than `text`.
    pub fn is_light(&self) -> bool {
        match self.mode {
            Mode::Light => true,
            Mode::Dark => false,
            Mode::Auto => luminance(self.bg) > luminance(self.text),
        }
    }

    /// `surface` as it shows over `bg` (themes may make panels see-through).
    pub fn panel(&self) -> Color {
        over(self.surface, self.bg)
    }

    /// The ink for text on `fill`: `text` or `text_inverse`, whichever has
    /// more contrast on it (`text` on a tie). Use it for any label on a
    /// coloured fill, in place of `bg` or `on_accent` (which is only for
    /// the accent).
    pub fn ink_on(&self, fill: Color) -> Color {
        let fill = over(fill, self.panel());
        let (a, b) = (self.text, self.role(Role::TextInverse));
        if contrast(over(b, fill), fill) > contrast(over(a, fill), fill) {
            b
        } else {
            a
        }
    }

    /// `ink` for text on `fill`, moved just far enough towards the theme's
    /// text or inverse text (whichever reads on `fill`) to reach 4.5:1.
    /// Text that already reads comes back as it is, so a theme whose
    /// colours read draws exactly what it chose. Use it for text in a
    /// colour of its own (a knob's number, a marker's label) on a fill the
    /// theme doesn't choose.
    pub fn readable(&self, ink: Color, fill: Color) -> Color {
        let fill = over(fill, self.panel());
        reach(ink, self.ink_on(fill), fill, 4.5)
    }

    /// A content colour in its five forms ([`Swatch`]) for this theme.
    pub fn swatch(&self, c: Color) -> Swatch {
        let fill = self.content(c);
        let panel = self.panel();
        Swatch {
            fill,
            tint: panel.mix(over(fill, panel), TINT),
            line: reach(fill, self.text, panel, 3.0),
            text: reach(fill, self.text, panel, 4.5),
            ink: self.ink_on(fill),
        }
    }

    /// A corner radius drawn for the default theme's radius (14), scaled
    /// to this theme's: one number, `radius`, sets every corner. The
    /// default theme gets `r` back unchanged.
    pub fn round(&self, r: i32) -> i32 {
        if self.radius == DESIGN_RADIUS {
            r
        } else {
            (r * self.radius + DESIGN_RADIUS / 2) / DESIGN_RADIUS
        }
    }
}

/// The radius screens are drawn for (the default theme's).
pub const DESIGN_RADIUS: i32 = 14;

/// Relative luminance (WCAG), 0 to 1.
pub fn luminance(c: Color) -> f32 {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::BUILTIN;

    #[test]
    fn every_rule_parses_and_names_only_real_colours() {
        for d in ROLES {
            let e = Expr::parse(d.rule).unwrap_or_else(|e| panic!("{}: {e}", d.key));
            assert_eq!(Expr::parse(&e.to_string()).unwrap(), e, "{}", d.key);
            assert!(!d.doc.is_empty() && d.doc.ends_with('.'), "{}", d.key);
        }
        let keys: std::collections::HashSet<&str> = ROLES.iter().map(|d| d.key).collect();
        assert_eq!(keys.len(), ROLES.len(), "a key is listed twice");
        for k in ColorKey::all() {
            assert_eq!(ColorKey::from_name(&k.name()), Some(k));
        }
    }

    #[test]
    fn rules_never_go_round_in_a_circle() {
        // Every built-in theme resolves (no role falls back to a guess).
        for name in BUILTIN {
            let (_, problems) = Theme::load(&format!("base = {name}"));
            assert!(problems.is_empty(), "{name}: {problems:?}");
        }
    }

    #[test]
    fn expressions_read_and_evaluate() {
        let t = Theme::dark();
        let ev = |s: &str| Expr::parse(s).unwrap().eval(false, &mut |k| t.color(k));
        assert_eq!(ev("#112233"), Color::rgb(0x112233));
        assert_eq!(ev("{accent}"), t.accent);
        assert_eq!(ev("accent"), t.accent);
        assert_eq!(ev("{track.3}"), t.track[2]);
        assert_eq!(ev("mix(bg, surface, 50%)"), t.bg.mix(t.surface, 0.5));
        assert_eq!(ev("alpha({text}, 50%)"), t.text.with_alpha(128));
        assert_eq!(ev("light_dark(#FFFFFF, #000000)"), Color::rgb(0));
        assert_eq!(
            ev(" mix( {accent} , alpha(#FFFFFF, 20%) , 12.5% ) "),
            t.accent.mix(Color::rgb(0xFFFFFF).with_alpha(51), 0.125)
        );
        for (bad, says) in [
            ("blue", "expected a colour"),
            ("{blue}", "no colour called \"blue\""),
            ("mix(bg, text)", "expected ','"),
            ("mix(bg, text, 150%)", "0% to 100%"),
            ("tint(bg, 5%)", "no colour function"),
            ("#12345", "expected #RRGGBB or #RRGGBBAA"),
            ("#112233 #445566", "unexpected"),
            ("{track.9}", "no colour called"),
        ] {
            let e = Expr::parse(bad).unwrap_err();
            assert!(e.contains(says), "{bad}: {e}");
        }
    }

    #[test]
    fn ink_on_picks_the_readable_text() {
        let t = Theme::dark();
        // Light text on a dark fill, the dark inverse on a bright one.
        assert_eq!(t.ink_on(t.surface), t.text);
        assert_eq!(t.ink_on(t.accent), t.bg);
        assert_eq!(t.ink_on(Color::rgb(0xFFC145)), t.bg);
        assert_eq!(t.ink_on(Color::rgb(0x223344)), t.text);
    }

    #[test]
    fn a_swatch_keeps_a_readable_colour_and_fixes_an_unreadable_one() {
        let t = Theme::dark();
        // A pastel on the dark panel already reads: every form is itself.
        let blue = Color::rgb(0x6EA8FE);
        let s = t.swatch(blue);
        assert_eq!((s.fill, s.line, s.text), (blue, blue, blue));
        assert_eq!(s.ink, t.bg);
        // A dark navy doesn't: its text and line forms move towards the
        // theme's text until they read.
        let navy = Color::rgb(0x1A2850);
        let s = t.swatch(navy);
        assert_eq!(s.fill, navy);
        assert!(contrast(s.text, t.panel()) >= 4.5);
        assert!(contrast(s.line, t.panel()) >= 3.0);
        assert!(contrast(s.line, t.panel()) < contrast(s.text, t.panel()));
        assert_eq!(s.ink, t.text);
        assert_eq!(s.tint, t.panel().mix(navy, TINT));
    }

    #[test]
    fn readable_leaves_text_that_reads_and_moves_text_that_doesnt() {
        let t = Theme::dark();
        let blue = Color::rgb(0x6EA8FE);
        assert_eq!(t.readable(blue, t.surface), blue);
        // Blue on amber is under 2:1; it moves towards the dark ink that
        // reads on amber, just far enough.
        let amber = Color::rgb(0xFFC145);
        let fixed = t.readable(blue, amber);
        assert!(contrast(fixed, amber) >= 4.5, "{fixed:?}");
        assert!(contrast(fixed, amber) < contrast(t.bg, amber));
        // On a light theme the same blue on its panel darkens.
        let light = Theme::parse("bg = #E4E7EB\nsurface = #F4F5F7\ntext = #161A21").unwrap();
        let fixed = light.readable(blue, light.surface);
        assert!(contrast(fixed, light.surface) >= 4.5, "{fixed:?}");
    }

    #[test]
    fn round_scales_corners_by_the_themes_radius() {
        let mut t = Theme::dark();
        assert_eq!(t.round(10), 10);
        assert_eq!(t.round(0), 0);
        t.radius = 4;
        assert_eq!(t.round(14), 4);
        assert_eq!(t.round(10), 3);
        t.radius = 0;
        assert_eq!(t.round(10), 0);
        t.radius = 28;
        assert_eq!(t.round(10), 20);
    }

    #[test]
    fn paint_resolves_on_the_theme_it_is_drawn_with() {
        let (a, b) = (Theme::dark(), Theme::console());
        let p = Paint::from(Role::Qlink1);
        assert_eq!(p.on(&a), a.qlink(0));
        let p = Paint::Key(ColorKey::Accent);
        assert_eq!((p.on(&a), p.on(&b)), (a.accent, b.accent));
        let kit = Color::rgb(0xFF6B6B);
        assert_eq!(Paint::Content(kit).on(&b), b.content(kit));
        assert_eq!(Paint::Literal(kit).on(&b), kit);
    }

    #[test]
    fn mode_follows_the_colours_unless_set() {
        let mut t = Theme::dark();
        assert!(!t.is_light());
        std::mem::swap(&mut t.bg, &mut t.text);
        assert!(t.is_light());
        t.mode = Mode::Dark;
        assert!(!t.is_light());
    }
}
