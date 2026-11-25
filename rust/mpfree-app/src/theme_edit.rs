//! Prefs › Appearance › Customise: friendly controls over a theme's
//! colours (layout spec section 5, the redesign canvas's Customise board,
//! Jay's go 24 Nov 2026 17:35Z). Each one reads its value from a theme and
//! writes a changed theme; the knobs move several colours together (the
//! accent's hue, how light the page is, how warm its greys are, how far
//! the panels stand out), and a picked colour can be set exactly by its
//! hue, colour and light. Nothing here draws or saves: the app keeps the
//! copy being changed ([`crate::app`]) and Save theme writes
//! [`file_text`] to the themes folder.

use mpfree_ui::roles::{hex, luminance, over, reach};
use mpfree_ui::theme::contrast;
use mpfree_ui::{Color, Theme};

/// A colour as hue (0 to 360), saturation and lightness (0 to 1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hsl {
    pub h: f32,
    pub s: f32,
    pub l: f32,
}

/// `c` as hue, saturation and lightness (its alpha left out).
pub fn to_hsl(c: Color) -> Hsl {
    let (r, g, b) = (c.r as f32 / 255.0, c.g as f32 / 255.0, c.b as f32 / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    let d = max - min;
    if d < 1e-6 {
        return Hsl { h: 0.0, s: 0.0, l };
    }
    let s = d / (1.0 - (2.0 * l - 1.0).abs()).max(1e-6);
    let h = if max == r {
        60.0 * ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    Hsl {
        h: h.rem_euclid(360.0),
        s: s.clamp(0.0, 1.0),
        l,
    }
}

/// The colour of `c`, with alpha `a`.
pub fn from_hsl(c: Hsl, a: u8) -> Color {
    let s = c.s.clamp(0.0, 1.0);
    let l = c.l.clamp(0.0, 1.0);
    let k = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let h = c.h.rem_euclid(360.0) / 60.0;
    let x = k * (1.0 - (h.rem_euclid(2.0) - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (k, x, 0.0),
        1 => (x, k, 0.0),
        2 => (0.0, k, x),
        3 => (0.0, x, k),
        4 => (x, 0.0, k),
        _ => (k, 0.0, x),
    };
    let m = l - k / 2.0;
    let v = |f: f32| ((f + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    Color {
        r: v(r),
        g: v(g),
        b: v(b),
        a,
    }
}

/// Sets one colour of `t` by its theme file key (its roles follow).
fn put(t: &mut Theme, key: &str, c: Color) {
    // A colour always parses, so this can't fail.
    let _ = t.set(key, &hex(c));
}

/// The page and panel greys the lightness and the panels move together.
const PAGE_GREYS: [&str; 4] = ["bg", "surface", "raised", "outline"];
/// Every grey the warmth tints: the page's, and the words'.
const ALL_GREYS: [&str; 7] = [
    "bg",
    "surface",
    "raised",
    "outline",
    "text",
    "text_dim",
    "text_faint",
];

/// How far apart the page and the panels are at "stand out" 100 %, in
/// lightness.
const PANELS_FULL: f32 = 0.1275;
/// How far the outlines go from the panel towards the words at 100 %.
const OUTLINES_FULL: f32 = 0.4;
/// The hue of a cool and of a warm grey, and the saturation at full
/// warmth either way.
const COOL_HUE: f32 = 222.0;
const WARM_HUE: f32 = 32.0;
const WARMTH_FULL: f32 = 0.4;

fn grey(t: &Theme, key: &str) -> Color {
    match key {
        "bg" => t.bg,
        "surface" => t.surface,
        "raised" => t.raised,
        "outline" => t.outline,
        "text" => t.text,
        "text_dim" => t.text_dim,
        "text_faint" => t.text_faint,
        "backdrop_to" => t.backdrop_to,
        "backdrop_line" => t.backdrop_line,
        "accent" => t.accent,
        "accent2" => t.accent2,
        "good" => t.good,
        "warn" => t.warn,
        "bad" => t.bad,
        _ => t.bg,
    }
}

/// The friendly knobs' values, read from a theme when Customise copies
/// it (and after Mode turns it round). The copy keeps them as they are
/// turned, so a step always moves the knob even where eight bits of
/// colour can't show it yet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Knobs {
    /// The accent's and second accent's hue, 0 to 360.
    pub accent: f32,
    pub accent2: f32,
    /// How light the page is, 0 to 1.
    pub lightness: f32,
    /// -1 (cool) to 1 (warm).
    pub warmth: f32,
    /// How far the panels stand out from the page, 0 to 1.
    pub panels: f32,
    /// How strong the outlines are, 0 to 1.
    pub outlines: f32,
    /// The accents' saturation and lightness, kept as the hue turns.
    accent_sl: (f32, f32),
    accent2_sl: (f32, f32),
    /// Buttons' distance from the page over the panels' (kept as the
    /// panels stand out more or less), signed.
    raised_ratio: f32,
}

impl Knobs {
    pub fn read(t: &Theme) -> Knobs {
        let a = to_hsl(t.accent);
        let a2 = to_hsl(t.accent2);
        let bg = to_hsl(t.bg).l;
        let ds = to_hsl(t.surface).l - bg;
        let dr = to_hsl(t.raised).l - bg;
        let raised_ratio = if ds.abs() > 0.004 { dr / ds } else { 2.3 };
        let panel = to_hsl(over(t.surface, t.bg)).l;
        let text = to_hsl(t.text).l;
        let outline = to_hsl(over(t.outline, over(t.surface, t.bg))).l;
        let span = text - panel;
        let outlines = if span.abs() > 0.01 {
            ((outline - panel) / span / OUTLINES_FULL).clamp(0.0, 1.0)
        } else {
            0.0
        };
        Knobs {
            accent: a.h,
            accent2: a2.h,
            lightness: bg,
            warmth: warmth_of(t),
            panels: (ds.abs() / PANELS_FULL).clamp(0.0, 1.0),
            outlines,
            accent_sl: (a.s, a.l),
            accent2_sl: (a2.s, a2.l),
            raised_ratio: raised_ratio.clamp(-6.0, 6.0),
        }
    }
}

/// How warm the page's greys are, -1 (cool, bluish) to 1 (warm).
fn warmth_of(t: &Theme) -> f32 {
    let mut sum = 0.0;
    for key in ["bg", "surface", "raised"] {
        let c = to_hsl(grey(t, key));
        let amount = (c.s / WARMTH_FULL).min(1.0);
        // Hues from green-blue round to purple read as cool.
        let cool = (150.0..=330.0).contains(&c.h);
        sum += if cool { -amount } else { amount };
    }
    (sum / 3.0).clamp(-1.0, 1.0)
}

/// The warmth knob's word.
pub fn warmth_word(w: f32) -> &'static str {
    match w {
        w if w <= -0.5 => "Cool",
        w if w < -0.15 => "A bit cool",
        w if w <= 0.15 => "Neutral",
        w if w < 0.5 => "A bit warm",
        _ => "Warm",
    }
}

/// Words on a fill of `fill`: the theme's darkest or lightest colour,
/// whichever reads better.
fn ink_for(t: &Theme, fill: Color) -> Color {
    let (dark, light) = if luminance(t.bg) < luminance(t.text) {
        (t.bg, t.text)
    } else {
        (t.text, t.bg)
    };
    if contrast(dark, fill) >= contrast(light, fill) {
        dark
    } else {
        light
    }
}

/// The accent at hue `h`, keeping its colour and light; its words follow.
pub fn set_accent(t: &mut Theme, k: &mut Knobs, h: f32) {
    k.accent = h.rem_euclid(360.0);
    let (s, l) = k.accent_sl;
    let c = from_hsl(Hsl { h: k.accent, s, l }, t.accent.a);
    put(t, "accent", c);
    if contrast(t.on_accent, c) < 4.5 {
        let ink = ink_for(t, c);
        put(t, "on_accent", ink);
    }
}

/// The second accent at hue `h`.
pub fn set_accent2(t: &mut Theme, k: &mut Knobs, h: f32) {
    k.accent2 = h.rem_euclid(360.0);
    let (s, l) = k.accent2_sl;
    let c = from_hsl(Hsl { h: k.accent2, s, l }, t.accent2.a);
    put(t, "accent2", c);
}

/// The page at lightness `l`: the panels, buttons and lines move with it.
pub fn set_lightness(t: &mut Theme, k: &mut Knobs, l: f32) {
    let l = l.clamp(0.0, 1.0);
    let d = l - k.lightness;
    k.lightness = l;
    for key in PAGE_GREYS {
        let c = grey(t, key);
        let mut h = to_hsl(c);
        h.l = (h.l + d).clamp(0.0, 1.0);
        put(t, key, from_hsl(h, c.a));
    }
}

/// The greys tinted cool (below 0) or warm (above).
pub fn set_warmth(t: &mut Theme, k: &mut Knobs, w: f32) {
    k.warmth = w.clamp(-1.0, 1.0);
    let hue = if k.warmth < 0.0 { COOL_HUE } else { WARM_HUE };
    for key in ALL_GREYS {
        let c = grey(t, key);
        let h = to_hsl(c);
        let s = k.warmth.abs() * WARMTH_FULL;
        put(t, key, from_hsl(Hsl { h: hue, s, l: h.l }, c.a));
    }
}

/// The panels `p` (0 to 1) away from the page, the buttons further in
/// the same proportion.
pub fn set_panels(t: &mut Theme, k: &mut Knobs, p: f32) {
    k.panels = p.clamp(0.0, 1.0);
    let bg = to_hsl(t.bg).l;
    // Panels go lighter than a dark page and lighter than a light one
    // (as Light's do), unless the theme had them the other way.
    let now = to_hsl(t.surface).l - bg;
    let way = if now.abs() > 0.004 {
        now.signum()
    } else if bg > 0.92 {
        -1.0
    } else {
        1.0
    };
    let ds = way * k.panels * PANELS_FULL;
    for (key, d) in [("surface", ds), ("raised", ds * k.raised_ratio)] {
        let c = grey(t, key);
        let mut h = to_hsl(c);
        h.l = (bg + d).clamp(0.0, 1.0);
        put(t, key, from_hsl(h, c.a));
    }
}

/// The outlines `p` (0 to 1) of the way from the panel to the words.
pub fn set_outlines(t: &mut Theme, k: &mut Knobs, p: f32) {
    k.outlines = p.clamp(0.0, 1.0);
    let panel = over(t.surface, t.bg);
    let c = panel
        .mix(t.text, k.outlines * OUTLINES_FULL)
        .with_alpha(t.outline.a);
    put(t, "outline", c);
}

/// The colours a dark theme turns round to make a light one, or a light
/// one a dark one, as Mode asks: the greys and words swap light for dark,
/// and the coloured ones move to where colours read on the new page.
/// Nothing changes when the theme already is that way round.
pub fn turn(t: &mut Theme, light: bool) {
    let is_light = luminance(t.bg) > luminance(t.text);
    if is_light != light {
        for key in ALL_GREYS.iter().chain(&["backdrop_to", "backdrop_line"]) {
            let c = grey(t, key);
            let mut h = to_hsl(c);
            h.l = 1.0 - h.l;
            put(t, key, from_hsl(h, c.a));
        }
        let colour = |c: Color| {
            let mut h = to_hsl(c);
            h.l = if light {
                0.55 - (h.l - 0.5) * (0.30 / 0.35)
            } else {
                0.5 + (0.55 - h.l) * (0.35 / 0.30)
            }
            .clamp(0.2, 0.9);
            from_hsl(h, c.a)
        };
        for key in ["accent", "accent2", "good", "warn", "bad"] {
            let c = colour(grey(t, key));
            put(t, key, c);
        }
        let tracks = t.track.map(colour);
        set_tracks(t, &tracks);
        let ink = ink_for(t, t.accent);
        put(t, "on_accent", ink);
        // The words read on the new page, its panels and its buttons.
        // Black on a light page, white on a dark one.
        let extreme = from_hsl(
            Hsl {
                h: 0.0,
                s: 0.0,
                l: if light { 0.0 } else { 1.0 },
            },
            255,
        );
        let panel = over(t.surface, t.bg);
        let fills = [t.bg, panel, over(t.raised, panel)];
        let mut text = t.text;
        for f in fills {
            text = reach(text, extreme, f, 7.0);
        }
        put(t, "text", text.with_alpha(t.text.a));
        for (key, target) in [("text_dim", 4.5), ("text_faint", 3.0)] {
            let mut c = grey(t, key);
            let a = c.a;
            for f in fills {
                c = reach(c, text, f, target);
            }
            put(t, key, c.with_alpha(a));
        }
    }
    let _ = t.set("mode", if light { "light" } else { "dark" });
}

/// The track colours as `cs`, with the Q-Links, graph lines and EQ bands
/// following them again (some built-in themes fixed those to colours of
/// their own).
pub fn set_tracks(t: &mut Theme, cs: &[Color; 8]) {
    let line: Vec<String> = cs.iter().map(|&c| hex(c)).collect();
    let _ = t.set("track", &line.join(" "));
    for n in 1..=4 {
        let _ = t.set(&format!("qlink.{n}"), &format!("{{track.{n}}}"));
    }
    for n in 1..=8 {
        let _ = t.set(&format!("series.{n}"), &format!("{{track.{n}}}"));
    }
}

/// The sets of track colours Set steps through, after the theme's own.
pub const PALETTES: [&str; 6] = ["Rainbow", "Pastel", "Neon", "Earth", "Ocean", "The accent"];

/// Palette `k`'s eight colours for `t` (darker on a light page).
pub fn palette(t: &Theme, k: usize) -> Option<[Color; 8]> {
    let light = luminance(t.bg) > luminance(t.text);
    let (hues, s, ld, ll): ([f32; 8], f32, f32, f32) = match k {
        0 => (
            [215.0, 150.0, 42.0, 280.0, 14.0, 188.0, 330.0, 88.0],
            0.85,
            0.66,
            0.42,
        ),
        1 => (
            [215.0, 150.0, 42.0, 280.0, 14.0, 188.0, 330.0, 88.0],
            0.7,
            0.8,
            0.55,
        ),
        2 => (
            [320.0, 180.0, 55.0, 275.0, 25.0, 200.0, 345.0, 100.0],
            1.0,
            0.58,
            0.45,
        ),
        3 => (
            [30.0, 95.0, 45.0, 15.0, 170.0, 60.0, 355.0, 130.0],
            0.45,
            0.58,
            0.4,
        ),
        4 => (
            [200.0, 185.0, 170.0, 215.0, 230.0, 160.0, 245.0, 190.0],
            0.65,
            0.62,
            0.4,
        ),
        5 => {
            let a = to_hsl(t.accent);
            let mut out = [t.accent; 8];
            for (n, c) in out.iter_mut().enumerate() {
                let step = n as f32 / 7.0;
                let l = if light {
                    0.30 + step * 0.25
                } else {
                    0.80 - step * 0.30
                };
                *c = from_hsl(
                    Hsl {
                        h: a.h,
                        s: a.s.max(0.5),
                        l,
                    },
                    255,
                );
            }
            return Some(out);
        }
        _ => return None,
    };
    let l = if light { ll } else { ld };
    Some(hues.map(|h| from_hsl(Hsl { h, s, l }, 255)))
}

/// The keys Every colour sets, each with its word under its swatch.
pub const EVERY_COLOUR: [(&str, &str); 9] = [
    ("bg", "Page"),
    ("surface", "Panels"),
    ("raised", "Buttons"),
    ("outline", "Lines"),
    ("text", "Words"),
    ("text_dim", "Dim"),
    ("good", "Good"),
    ("warn", "Warn"),
    ("bad", "Bad"),
];

/// The backdrop's two colours: Fade's foot, and the pattern's lines.
pub const BACKDROP_COLOURS: [(&str, &str); 2] =
    [("backdrop_to", "Fade"), ("backdrop_line", "Lines")];

/// A colour of `t` by its key: one of the above, or `track.N`.
pub fn colour(t: &Theme, key: &str) -> Color {
    match key
        .strip_prefix("track.")
        .and_then(|n| n.parse::<usize>().ok())
    {
        Some(n) => t.track((n + 7) % 8),
        None => grey(t, key),
    }
}

/// Sets a colour of `t` by its key, as [`colour`] reads it.
pub fn set_colour(t: &mut Theme, key: &str, c: Color) {
    match key
        .strip_prefix("track.")
        .and_then(|n| n.parse::<usize>().ok())
    {
        Some(n) if (1..=8).contains(&n) => {
            let mut cs = t.track;
            cs[n - 1] = c;
            set_tracks(t, &cs);
        }
        _ => put(t, key, c),
    }
    if key == "accent" && contrast(t.on_accent, c) < 4.5 {
        let ink = ink_for(t, c);
        put(t, "on_accent", ink);
    }
}

/// How well `t` reads: words, dim words and the accent on a panel.
pub fn readability(t: &Theme) -> (f32, f32, f32) {
    let panel = t.panel();
    (
        contrast(t.text, panel),
        contrast(t.text_dim, panel),
        contrast(t.accent, panel),
    )
}

/// `t` as a theme file: `base = BASE`, its name, then only the lines that
/// differ from the built-in theme `base`, so it reads like one written by
/// hand and [`Theme::parse`] gives `t` back.
pub fn file_text(t: &Theme, base: &str) -> String {
    let from = Theme::builtin(base).unwrap_or_default();
    let key = |line: &str| line.split('=').next().unwrap_or("").trim().to_string();
    let theirs: Vec<(String, String)> = from
        .to_text()
        .lines()
        .map(|l| (key(l), l.to_string()))
        .collect();
    let mut out = String::from(
        "# Made in MPFree's Prefs > Appearance > Customise: the built-in theme\n\
         # named by base, with the lines below changed.\n",
    );
    out.push_str(&format!("{:<13} = {}\n", "base", base));
    for line in t.to_text().lines() {
        let k = key(line);
        let same = theirs.iter().any(|(tk, tl)| *tk == k && tl == line);
        if k == "name" || !same {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// The file a theme named `name` is saved as: its name in small letters,
/// numbers and hyphens, then `.theme`, with -2, -3... after it when
/// `taken` says the file belongs to another theme.
pub fn file_name(name: &str, taken: impl Fn(&str) -> bool) -> String {
    let mut stem = String::new();
    for c in name.trim().chars().flat_map(|c| c.to_lowercase()) {
        if c.is_ascii_alphanumeric() {
            stem.push(c);
        } else if !stem.ends_with('-') && !stem.is_empty() {
            stem.push('-');
        }
    }
    let mut stem: String = stem.trim_end_matches('-').chars().take(40).collect();
    if stem.is_empty() {
        stem = "my-theme".to_string();
    }
    let mut file = format!("{stem}.theme");
    let mut n = 2;
    while taken(&file) {
        file = format!("{stem}-{n}.theme");
        n += 1;
    }
    file
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_ui::theme::BUILTIN;

    #[test]
    fn colours_go_to_hue_saturation_lightness_and_back() {
        for v in [
            0x000000, 0xFFFFFF, 0x6EA8FE, 0x0B0D12, 0xFFC145, 0x3DDC97, 0xD46CFF, 0x808080,
        ] {
            let c = Color::rgb(v);
            assert_eq!(from_hsl(to_hsl(c), 255), c, "{v:06X}");
        }
        let h = to_hsl(Color::rgb(0xFF0000));
        assert_eq!((h.h, h.s, h.l), (0.0, 1.0, 0.5));
    }

    #[test]
    fn the_default_themes_knobs_read_as_drawn() {
        let k = Knobs::read(&Theme::dark());
        assert_eq!((k.lightness * 100.0).round(), 6.0);
        assert_eq!((k.panels * 100.0).round(), 40.0);
        assert_eq!(warmth_word(k.warmth), "Cool");
        assert_eq!(k.accent.round(), 216.0);
        assert_eq!(k.accent2.round(), 282.0);
    }

    #[test]
    fn the_accent_turns_and_its_words_still_read() {
        let mut t = Theme::dark();
        let mut k = Knobs::read(&t);
        for h in (0..360).step_by(15) {
            set_accent(&mut t, &mut k, h as f32);
            assert!((to_hsl(t.accent).h - h as f32).abs() < 3.0, "{h}");
            assert!(contrast(t.on_accent, t.accent) >= 4.5, "{h}");
            assert_eq!(t.picked(), t.accent);
        }
    }

    #[test]
    fn the_greys_move_together() {
        let mut t = Theme::dark();
        let mut k = Knobs::read(&t);
        let before = to_hsl(t.surface).l - to_hsl(t.bg).l;
        set_lightness(&mut t, &mut k, 0.15);
        assert!((to_hsl(t.bg).l - 0.15).abs() < 0.01);
        let after = to_hsl(t.surface).l - to_hsl(t.bg).l;
        assert!((after - before).abs() < 0.01);
        set_panels(&mut t, &mut k, 0.8);
        let ds = to_hsl(t.surface).l - to_hsl(t.bg).l;
        assert!((ds - 0.8 * PANELS_FULL).abs() < 0.01, "{ds}");
        set_warmth(&mut t, &mut k, 1.0);
        let w = to_hsl(t.bg);
        assert!((w.h - WARM_HUE).abs() < 10.0, "{w:?}");
        assert_eq!(warmth_word(Knobs::read(&t).warmth), "Warm");
        set_warmth(&mut t, &mut k, 0.0);
        assert_eq!(to_hsl(t.bg).s, 0.0);
        set_outlines(&mut t, &mut k, 1.0);
        assert!((Knobs::read(&t).outlines - 1.0).abs() < 0.05);
    }

    #[test]
    fn mode_turns_a_dark_theme_light_and_back_and_both_read() {
        for name in BUILTIN {
            let mut t = Theme::builtin(name).unwrap();
            let light = t.is_light();
            turn(&mut t, !light);
            assert_eq!(t.is_light(), !light, "{name}");
            assert!(t.unusable().is_none(), "{name}: {:?}", t.unusable());
            turn(&mut t, light);
            assert_eq!(t.is_light(), light, "{name}");
            assert!(t.unusable().is_none(), "{name}");
        }
    }

    #[test]
    fn every_palette_has_eight_colours_and_the_qlinks_follow() {
        let mut t = Theme::console();
        for k in 0..PALETTES.len() {
            let cs = palette(&t, k).unwrap();
            set_tracks(&mut t, &cs);
            assert_eq!(t.track, cs);
            assert_eq!(t.qlink(0), cs[0]);
            assert_eq!(t.series(7), cs[7]);
        }
        assert!(palette(&t, PALETTES.len()).is_none());
    }

    #[test]
    fn a_saved_theme_reads_back_the_same_and_lists_only_what_changed() {
        for base in BUILTIN {
            let mut t = Theme::builtin(base).unwrap();
            t.name = "My theme".into();
            let mut k = Knobs::read(&t);
            let h = k.accent + 40.0;
            set_accent(&mut t, &mut k, h);
            set_colour(&mut t, "track.3", Color::rgb(0x123456));
            let _ = t.set("corners", "square");
            let text = file_text(&t, base);
            let back = Theme::parse(&text).unwrap();
            assert_eq!(back, t, "{base}:\n{text}");
            assert!(text.contains("corners "), "{base}");
            assert!(!text.contains("startup"), "{base}: {text}");
            assert!(text.lines().count() < 26, "{base}: {text}");
        }
    }

    #[test]
    fn a_theme_is_saved_under_its_name() {
        let none = |_: &str| false;
        assert_eq!(file_name("My theme", none), "my-theme.theme");
        assert_eq!(file_name("  Night & Day! ", none), "night-day.theme");
        assert_eq!(file_name("***", none), "my-theme.theme");
        let taken = |f: &str| f == "my-theme.theme" || f == "my-theme-2.theme";
        assert_eq!(file_name("My theme", taken), "my-theme-3.theme");
    }

    #[test]
    fn colours_by_key_include_the_tracks() {
        let mut t = Theme::dark();
        assert_eq!(colour(&t, "track.1"), t.track[0]);
        set_colour(&mut t, "track.8", Color::rgb(0x112233));
        assert_eq!(t.track[7], Color::rgb(0x112233));
        set_colour(&mut t, "warn", Color::rgb(0x445566));
        assert_eq!(colour(&t, "warn"), Color::rgb(0x445566));
    }
}
