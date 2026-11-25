//! Prefs › Appearance › Customise (layout spec section 5, the redesign
//! canvas's Customise board, Jay's go 24 Nov 2026 17:35Z): a copy of the
//! theme in use, changed with friendly knobs and exact colours, drawn on
//! the whole screen while Customise is open, and saved as a theme file in
//! the themes folder. [`crate::theme_edit`] does the colour work; the
//! floor every theme must reach ([`mpfree_ui::Theme::unusable`]) holds
//! here too, so no change can make Prefs itself unreadable.

use super::*;
use crate::prefs_page::{Control, Look, RowView};
use crate::theme_edit::{self, Hsl, Knobs};
use mpfree_ui::roles::Mode as ThemeMode;
use mpfree_ui::{Backdrop, Color, Corners, Role, Theme};

/// A row of Customise.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemeItem {
    /// The copy's name and what it is; Rename.
    Name,
    /// Undo changes: back to the copy as it was made.
    Undo,
    /// Save theme: a file in the themes folder, in use from then on.
    Save,
    /// The Colours card's four knobs: Accent, Second accent, Page
    /// lightness and Warmth, or while a colour is being set, its Hue,
    /// Colour and Light, and Done.
    Dial(u8),
    /// Every colour: tap one to set it exactly.
    Colours,
    Mode,
    /// Panels stand out.
    Panels,
    Corners,
    /// Corner size.
    Radius,
    Outlines,
    Glow,
    TextGlow,
    /// Tint pads.
    Tint,
    /// Pattern size.
    Pattern,
    Backdrop,
    /// Backdrop colours: Fade's foot and the pattern's lines.
    BackdropColours,
    /// The eight track colours.
    Tracks,
    /// Set: the theme's own track colours or a palette.
    TrackSet,
}

use ThemeItem as T;

/// Customise's rows, in the wheel's order: the banner, then the left
/// column's cards, then the right's.
pub(super) const CUSTOMISE_ROWS: &[Item] = &[
    Item::Theme(T::Name),
    Item::Theme(T::Undo),
    Item::Theme(T::Save),
    Item::Theme(T::Dial(0)),
    Item::Theme(T::Dial(1)),
    Item::Theme(T::Dial(2)),
    Item::Theme(T::Dial(3)),
    Item::Theme(T::Colours),
    Item::Theme(T::Mode),
    Item::Theme(T::Panels),
    Item::Theme(T::Corners),
    Item::Theme(T::Radius),
    Item::Theme(T::Outlines),
    Item::Theme(T::Glow),
    Item::Theme(T::TextGlow),
    Item::Theme(T::Tint),
    Item::Theme(T::Pattern),
    Item::Theme(T::Backdrop),
    Item::Theme(T::BackdropColours),
    Item::Theme(T::Tracks),
    Item::Theme(T::TrackSet),
];

/// The most letters a theme's name may have.
pub const THEME_NAME_MAX: usize = 32;

const CORNERS: [(Corners, &str); 3] = [
    (Corners::Round, "Round"),
    (Corners::Chamfer, "Cut"),
    (Corners::Square, "Square"),
];
const BACKDROPS: [(Backdrop, &str); 5] = [
    (Backdrop::Solid, "Solid"),
    (Backdrop::Gradient, "Fade"),
    (Backdrop::Grid, "Grid"),
    (Backdrop::Dots, "Dots"),
    (Backdrop::Scanlines, "Lines"),
];
/// The smallest and largest pattern step the knob turns through.
const PATTERN_MIN: i32 = 4;
const PATTERN_MAX: i32 = 120;

/// The copy Customise changes.
pub(super) struct Custom {
    /// What `settings.theme` held when it was copied, and its name.
    from: String,
    from_label: String,
    /// The built-in theme its file starts from.
    base: String,
    /// The copy as it was made (Undo changes goes back to it), and now.
    start: Theme,
    pub(super) theme: Theme,
    start_knobs: Knobs,
    knobs: Knobs,
    /// The file Save theme wrote, and the theme as it was then.
    file: Option<String>,
    saved: Option<Theme>,
    /// The colour being set exactly: its key and its hue, colour and light.
    pick: Option<(String, Hsl)>,
    /// The track colours' set: the theme's own (0), a palette (1 on), or
    /// changed by hand (None).
    track_set: Option<usize>,
    /// The last change would have made the words unreadable, so it
    /// wasn't made: the banner says so until the next change.
    stopped: bool,
}

impl Custom {
    fn new(from: &str, from_label: &str, base: &str, mut theme: Theme, name: String) -> Custom {
        theme.name = name;
        let knobs = Knobs::read(&theme);
        Custom {
            from: from.to_string(),
            from_label: from_label.to_string(),
            base: base.to_string(),
            start: theme.clone(),
            theme,
            start_knobs: knobs,
            knobs,
            file: None,
            saved: None,
            pick: None,
            track_set: Some(0),
            stopped: false,
        }
    }

    /// How many of its theme file's lines differ from the copy as made.
    fn changes(&self) -> usize {
        let a = self.start.to_text();
        let b = self.theme.to_text();
        a.lines().zip(b.lines()).filter(|(x, y)| x != y).count()
    }

    /// Has changes Save theme hasn't written.
    fn unsaved(&self) -> bool {
        match &self.saved {
            Some(s) => *s != self.theme,
            None => self.changes() > 0,
        }
    }

    /// The colour being set's word (Page, Track 3...).
    fn pick_word(&self) -> Option<String> {
        let (key, _) = self.pick.as_ref()?;
        if let Some(n) = key.strip_prefix("track.") {
            return Some(format!("Track {n}"));
        }
        theme_edit::EVERY_COLOUR
            .iter()
            .chain(&theme_edit::BACKDROP_COLOURS)
            .find(|(k, _)| k == key)
            .map(|(_, w)| match *w {
                "Lines" if key == "backdrop_line" => "Pattern".to_string(),
                w => w.to_string(),
            })
    }

    /// The banner's line: what the copy is and how far it has come.
    fn line(&self) -> String {
        if self.stopped {
            return "Stopped there: the words would be too hard to read".to_string();
        }
        if let (Some(file), false) = (&self.file, self.unsaved()) {
            return format!("Saved as {file}, now the theme in use");
        }
        let from = &self.from_label;
        match self.changes() {
            0 => format!("A copy of {from} · changes show on every page at once"),
            1 => format!("A copy of {from} · 1 change, not saved yet"),
            n => format!("A copy of {from} · {n} changes, not saved yet"),
        }
    }
}

impl App {
    /// The theme Customise copies: what `settings.theme` holds, built in
    /// or a theme file, with its name and the built-in theme it starts
    /// from. A copy of a built-in theme is "My theme"; of a file, its name
    /// and 2 (Customise never writes over a file it didn't make).
    fn copy_theme_in_use(&self) -> Custom {
        let key = self.settings.theme.clone();
        let label = self
            .theme_choices()
            .find(|c| c.0 == key)
            .map_or(key.clone(), |c| c.1.to_string());
        let (base, look) = match Theme::builtin(&key) {
            Some(t) => (key.to_ascii_lowercase(), t),
            None => (
                "default".to_string(),
                self.theme_looks
                    .iter()
                    .find(|(f, _)| *f == key)
                    .map_or_else(Theme::dark, |(_, t)| t.clone()),
            ),
        };
        let stem = if Theme::builtin(&key).is_some() {
            "My theme".to_string()
        } else {
            label.clone()
        };
        let taken = |n: &str| self.theme_choices().any(|c| c.1.eq_ignore_ascii_case(n));
        let mut name = stem.clone();
        let mut k = 2;
        while taken(&name) {
            name = format!("{stem} {k}");
            k += 1;
        }
        Custom::new(&key, &label, &base, look, name)
    }

    /// Opens the copy for Customise (the tab opened, Customise › or New
    /// theme): the copy there already, if it has unsaved changes or (not
    /// `fresh`) is the theme in use's; else a new copy of the theme in
    /// use. It shows on the whole screen.
    pub(super) fn custom_open(&mut self, fresh: bool) -> Vec<Effect> {
        let in_use = self.settings.theme.clone();
        let keep = self.custom.as_ref().is_some_and(|c| {
            c.unsaved() || (!fresh && (c.from == in_use || c.file.as_deref() == Some(&in_use)))
        });
        if !keep {
            self.custom = Some(self.copy_theme_in_use());
        }
        self.custom_live = true;
        let theme = self.custom.as_ref().map(|c| c.theme.clone());
        vec![Effect::LiveTheme(theme.map(Box::new))]
    }

    /// Leaving Customise (another tab, or Prefs closed): the theme in use
    /// comes back. The copy waits, changes and all, for Customise again.
    pub(super) fn custom_leave(&mut self) -> Vec<Effect> {
        if !std::mem::take(&mut self.custom_live) {
            return Vec::new();
        }
        if let Some(c) = &mut self.custom {
            c.pick = None;
        }
        vec![Effect::LiveTheme(None)]
    }

    /// The New theme tile (`fresh`) or Customise › on the Themes tab: a
    /// theme on trial is kept, and Customise opens on a copy of it.
    pub fn customise_from_themes(&mut self, fresh: bool) -> Vec<Effect> {
        if self.view != View::Prefs {
            return Vec::new();
        }
        let mut fx = self.end_theme_trial(true);
        fx.retain(|e| !matches!(e, Effect::Menu(_)));
        if fresh {
            // A fresh copy even when the copy there is the theme in use's.
            let unsaved = self.custom.as_ref().is_some_and(|c| c.unsaved());
            if !unsaved {
                self.custom = None;
            }
        }
        fx.extend(self.pick_prefs_tab(PrefsTab::Customise.index()));
        fx
    }

    /// Rename's name typed (the text keyboard's Enter).
    pub fn name_theme(&mut self, name: &str) -> Vec<Effect> {
        let mut name: String = name.trim().chars().take(THEME_NAME_MAX).collect();
        if name.is_empty() || self.custom.is_none() {
            return Vec::new();
        }
        // A built-in theme's name would be shown as the file's name.
        if THEMES.iter().any(|b| b.1.eq_ignore_ascii_case(&name)) {
            name.push_str(" 2");
        }
        self.custom_change(|c| c.theme.name = name)
    }

    /// Changes the copy with `f`, unless that would leave it unreadable;
    /// the screen and Customise follow.
    fn custom_change(&mut self, f: impl FnOnce(&mut Custom)) -> Vec<Effect> {
        let Some(c) = self.custom.as_mut() else {
            return Vec::new();
        };
        let before = (c.theme.clone(), c.knobs, c.pick.clone(), c.track_set);
        f(c);
        c.stopped = c.theme.unusable().is_some();
        if c.stopped {
            (c.theme, c.knobs, c.pick, c.track_set) = before.clone();
        }
        let mut fx = Vec::new();
        if c.theme != before.0 && self.custom_live {
            fx.push(Effect::LiveTheme(Some(Box::new(c.theme.clone()))));
        }
        fx.push(self.menu_view());
        fx
    }

    /// Picks colour `key` to set exactly, or (picked already) puts it
    /// down.
    fn custom_pick(&mut self, key: &str) -> Vec<Effect> {
        let key = key.to_string();
        self.custom_change(|c| {
            if c.pick.as_ref().is_some_and(|(k, _)| *k == key) {
                c.pick = None;
            } else {
                let hsl = theme_edit::to_hsl(theme_edit::colour(&c.theme, &key));
                c.pick = Some((key, hsl));
            }
        })
    }

    /// The colour being set at `hsl`.
    fn custom_set_pick(c: &mut Custom, hsl: Hsl) {
        let Some((key, _)) = c.pick.clone() else {
            return;
        };
        let a = theme_edit::colour(&c.theme, &key).a;
        theme_edit::set_colour(&mut c.theme, &key, theme_edit::from_hsl(hsl, a));
        c.pick = Some((key.clone(), hsl));
        if key.starts_with("track.") {
            c.track_set = None;
        }
    }

    /// The keys of the colours row `ti` offers to set.
    fn custom_keys(ti: ThemeItem) -> Vec<String> {
        match ti {
            T::Colours => theme_edit::EVERY_COLOUR
                .iter()
                .map(|(k, _)| k.to_string())
                .collect(),
            T::BackdropColours => theme_edit::BACKDROP_COLOURS
                .iter()
                .map(|(k, _)| k.to_string())
                .collect(),
            T::Tracks => (1..=8).map(|n| format!("track.{n}")).collect(),
            _ => Vec::new(),
        }
    }

    /// Row `ti` stepped by `d` (the wheel, - / +, or a tap on a button).
    pub(super) fn custom_step(&mut self, ti: ThemeItem, d: i32) -> Vec<Effect> {
        let d = d.signum();
        let picking = self.custom.as_ref().is_some_and(|c| c.pick.is_some());
        match ti {
            T::Name => {
                if d <= 0 {
                    return Vec::new();
                }
                let name = self
                    .custom
                    .as_ref()
                    .map(|c| c.theme.name.clone())
                    .unwrap_or_default();
                vec![Effect::NameTheme(name)]
            }
            T::Undo => {
                if d <= 0 {
                    return Vec::new();
                }
                self.custom_change(|c| {
                    c.theme = c.start.clone();
                    c.knobs = c.start_knobs;
                    c.pick = None;
                    c.track_set = Some(0);
                })
            }
            T::Save => {
                if d <= 0 {
                    return Vec::new();
                }
                self.custom_save()
            }
            T::Dial(3) if picking => {
                if d <= 0 {
                    return Vec::new();
                }
                self.custom_change(|c| c.pick = None)
            }
            T::Dial(k) if picking => self.custom_change(|c| {
                let Some((_, mut hsl)) = c.pick.clone() else {
                    return;
                };
                match k {
                    0 => hsl.h = (hsl.h + 5.0 * d as f32).rem_euclid(360.0),
                    1 => hsl.s = (hsl.s + 0.02 * d as f32).clamp(0.0, 1.0),
                    _ => hsl.l = (hsl.l + 0.01 * d as f32).clamp(0.0, 1.0),
                }
                App::custom_set_pick(c, hsl);
            }),
            T::Dial(k) => self.custom_change(|c| {
                let t = &mut c.theme;
                let kn = &mut c.knobs;
                let f = d as f32;
                match k {
                    0 => theme_edit::set_accent(t, kn, kn.accent + 5.0 * f),
                    1 => theme_edit::set_accent2(t, kn, kn.accent2 + 5.0 * f),
                    2 => theme_edit::set_lightness(t, kn, kn.lightness + 0.01 * f),
                    _ => theme_edit::set_warmth(t, kn, kn.warmth + 0.05 * f),
                }
            }),
            T::Colours | T::BackdropColours | T::Tracks => {
                let keys = App::custom_keys(ti);
                let at = self
                    .custom
                    .as_ref()
                    .and_then(|c| c.pick.as_ref())
                    .and_then(|(k, _)| keys.iter().position(|x| x == k));
                let next = match at {
                    Some(i) => (i as i32 + d).clamp(0, keys.len() as i32 - 1) as usize,
                    None if d > 0 => 0,
                    None => keys.len() - 1,
                };
                if Some(next) == at {
                    return Vec::new();
                }
                self.custom_pick(&keys[next])
            }
            T::Mode => {
                let at = self.custom_mode_at();
                let next = (at as i32 + d).clamp(0, 2) as usize;
                self.custom_choose(ti, next)
            }
            T::Panels => self.custom_change(|c| {
                let p = c.knobs.panels + 0.05 * d as f32;
                theme_edit::set_panels(&mut c.theme, &mut c.knobs, p);
            }),
            T::Corners => {
                let at = self.custom_corners_at();
                let next = (at as i32 + d).clamp(0, 2) as usize;
                self.custom_choose(ti, next)
            }
            T::Radius => self.custom_change(|c| c.theme.radius = (c.theme.radius + d).clamp(0, 40)),
            T::Outlines => self.custom_change(|c| {
                let p = c.knobs.outlines + 0.05 * d as f32;
                theme_edit::set_outlines(&mut c.theme, &mut c.knobs, p);
            }),
            T::Glow => self.custom_change(|c| c.theme.glow = step_byte(c.theme.glow, d * 16)),
            T::TextGlow => {
                self.custom_change(|c| c.theme.text_glow = step_byte(c.theme.text_glow, d * 16))
            }
            T::Tint => self.custom_change(|c| c.theme.tint = step_byte(c.theme.tint, d * 13)),
            T::Pattern => self.custom_change(|c| {
                c.theme.backdrop_step =
                    (c.theme.backdrop_step + 2 * d).clamp(PATTERN_MIN, PATTERN_MAX)
            }),
            T::Backdrop => {
                let at = self.custom_backdrop_at();
                let next = (at as i32 + d).clamp(0, BACKDROPS.len() as i32 - 1) as usize;
                self.custom_choose(ti, next)
            }
            T::TrackSet => {
                let at = self.custom.as_ref().and_then(|c| c.track_set);
                let last = theme_edit::PALETTES.len() as i32;
                let next = match at {
                    Some(k) => (k as i32 + d).clamp(0, last) as usize,
                    None if d > 0 => 1,
                    None => 0,
                };
                if Some(next) == at {
                    return Vec::new();
                }
                self.custom_change(|c| {
                    match next {
                        0 => restore_tracks(&mut c.theme, &c.start),
                        k => {
                            if let Some(cs) = theme_edit::palette(&c.theme, k - 1) {
                                theme_edit::set_tracks(&mut c.theme, &cs);
                            }
                        }
                    }
                    c.track_set = Some(next);
                    c.pick = c.pick.take().filter(|(k, _)| !k.starts_with("track."));
                })
            }
        }
    }

    /// Choice `k` of row `ti` tapped (a button, a look or a swatch).
    pub(super) fn custom_choose(&mut self, ti: ThemeItem, k: usize) -> Vec<Effect> {
        match ti {
            T::Mode => self.custom_change(|c| {
                match k {
                    0 => theme_edit::turn(&mut c.theme, false),
                    1 => theme_edit::turn(&mut c.theme, true),
                    _ => {
                        let _ = c.theme.set("mode", "auto");
                    }
                }
                c.knobs = Knobs::read(&c.theme);
            }),
            T::Corners => match CORNERS.get(k) {
                Some(&(corners, _)) => self.custom_change(|c| c.theme.corners = corners),
                None => Vec::new(),
            },
            T::Backdrop => match BACKDROPS.get(k) {
                Some(&(b, _)) => self.custom_change(|c| c.theme.backdrop = b),
                None => Vec::new(),
            },
            T::Colours | T::BackdropColours | T::Tracks => match App::custom_keys(ti).get(k) {
                Some(key) => self.custom_pick(&key.clone()),
                None => Vec::new(),
            },
            T::Dial(3) if self.custom.as_ref().is_some_and(|c| c.pick.is_some()) => {
                self.custom_change(|c| c.pick = None)
            }
            _ => self.custom_step(ti, 1),
        }
    }

    /// Row `ti`'s knob or slider moved to `p` (0 to 1).
    pub(super) fn custom_value(&mut self, ti: ThemeItem, p: f32) -> Vec<Effect> {
        let picking = self.custom.as_ref().is_some_and(|c| c.pick.is_some());
        match ti {
            T::Dial(k) if picking && k < 3 => self.custom_change(|c| {
                let Some((_, mut hsl)) = c.pick.clone() else {
                    return;
                };
                match k {
                    0 => hsl.h = (p * 360.0).min(359.9),
                    1 => hsl.s = p,
                    _ => hsl.l = p,
                }
                App::custom_set_pick(c, hsl);
            }),
            T::Dial(k) if !picking => self.custom_change(|c| {
                let (t, kn) = (&mut c.theme, &mut c.knobs);
                match k {
                    0 => theme_edit::set_accent(t, kn, (p * 360.0).min(359.9)),
                    1 => theme_edit::set_accent2(t, kn, (p * 360.0).min(359.9)),
                    2 => theme_edit::set_lightness(t, kn, p),
                    _ => theme_edit::set_warmth(t, kn, p * 2.0 - 1.0),
                }
            }),
            T::Panels => {
                self.custom_change(|c| theme_edit::set_panels(&mut c.theme, &mut c.knobs, p))
            }
            T::Radius => self.custom_change(|c| c.theme.radius = (p * 40.0).round() as i32),
            T::Outlines => {
                self.custom_change(|c| theme_edit::set_outlines(&mut c.theme, &mut c.knobs, p))
            }
            T::Glow => self.custom_change(|c| c.theme.glow = (p * 255.0).round() as u8),
            T::TextGlow => self.custom_change(|c| c.theme.text_glow = (p * 255.0).round() as u8),
            T::Tint => self.custom_change(|c| c.theme.tint = (p * 255.0).round() as u8),
            T::Pattern => self.custom_change(|c| {
                c.theme.backdrop_step =
                    PATTERN_MIN + (p * (PATTERN_MAX - PATTERN_MIN) as f32).round() as i32
            }),
            _ => Vec::new(),
        }
    }

    /// Save theme: the copy as a theme file in the themes folder (its
    /// own file again after the first save), in use from now on.
    fn custom_save(&mut self) -> Vec<Effect> {
        let Some(c) = self.custom.as_ref() else {
            return Vec::new();
        };
        if !c.unsaved() {
            // Faded: nothing new to save.
            return Vec::new();
        }
        let own = c.file.clone();
        let file = own.clone().unwrap_or_else(|| {
            theme_edit::file_name(&c.theme.name, |f| {
                self.theme_files.iter().any(|(x, _)| x == f)
            })
        });
        let text = theme_edit::file_text(&c.theme, &c.base);
        let theme = c.theme.clone();
        if let Some(c) = self.custom.as_mut() {
            c.file = Some(file.clone());
            c.saved = Some(theme.clone());
            c.stopped = false;
        }
        match self.theme_files.iter_mut().find(|(f, _)| *f == file) {
            Some(entry) => entry.1 = theme.name.clone(),
            None => self.theme_files.push((file.clone(), theme.name.clone())),
        }
        match self.theme_looks.iter_mut().find(|(f, _)| *f == file) {
            Some(entry) => entry.1 = theme.clone(),
            None => self.theme_looks.push((file.clone(), theme.clone())),
        }
        self.theme_problems.retain(|(f, _)| *f != file);
        self.settings.theme = file.clone();
        self.save_dirty = true;
        vec![
            Effect::SaveTheme {
                file: file.clone(),
                text,
                theme: Box::new(theme),
            },
            Effect::Settings(self.settings.clone()),
            Effect::Note(format!("theme: saved {file}, in use")),
            self.menu_view(),
        ]
    }

    fn custom_mode_at(&self) -> usize {
        match self.custom.as_ref().map(|c| c.theme.mode) {
            Some(ThemeMode::Light) => 1,
            Some(ThemeMode::Auto) => 2,
            _ => 0,
        }
    }

    fn custom_corners_at(&self) -> usize {
        let now = self.custom.as_ref().map(|c| c.theme.corners);
        CORNERS
            .iter()
            .position(|&(c, _)| Some(c) == now)
            .unwrap_or(0)
    }

    fn custom_backdrop_at(&self) -> usize {
        let now = self.custom.as_ref().map(|c| c.theme.backdrop);
        BACKDROPS
            .iter()
            .position(|&(b, _)| Some(b) == now)
            .unwrap_or(0)
    }

    /// A row that opens or does something once (Rename, Undo changes,
    /// Save theme, Done): the wheel's push does it.
    pub(super) fn custom_opens(&self, item: Item) -> bool {
        match item {
            Item::Theme(T::Name | T::Undo | T::Save) => true,
            Item::Theme(T::Dial(3)) => self.custom.as_ref().is_some_and(|c| c.pick.is_some()),
            _ => false,
        }
    }

    /// Row `ti`'s name and value, as the menu lists it.
    pub(super) fn custom_text(&self, ti: ThemeItem) -> (&'static str, String) {
        let Some(c) = &self.custom else {
            return ("Customise", String::new());
        };
        let pct = |v: f32| format!("{:.0} %", v * 100.0);
        let byte = |v: u8| {
            if v == 0 {
                "Off".to_string()
            } else {
                format!("{:.0} %", v as f32 * 100.0 / 255.0)
            }
        };
        let k = &c.knobs;
        match (ti, &c.pick) {
            (T::Name, _) => ("Theme", c.theme.name.clone()),
            (T::Undo, _) => ("Undo changes", String::new()),
            (T::Save, _) => ("Save theme", String::new()),
            (T::Dial(0), Some((_, h))) => ("Hue", format!("{:.0}°", h.h)),
            (T::Dial(1), Some((_, h))) => ("Colour", pct(h.s)),
            (T::Dial(2), Some((_, h))) => ("Light", pct(h.l)),
            (T::Dial(_), Some(_)) => ("Done", String::new()),
            (T::Dial(0), None) => ("Accent", format!("{:.0}°", k.accent)),
            (T::Dial(1), None) => ("Second accent", format!("{:.0}°", k.accent2)),
            (T::Dial(2), None) => ("Page lightness", pct(k.lightness)),
            (T::Dial(_), None) => ("Warmth", theme_edit::warmth_word(k.warmth).to_string()),
            (T::Colours, _) => (
                "Every colour · tap one to set it exactly",
                c.pick_word().unwrap_or_default(),
            ),
            (T::Mode, _) => (
                "Mode",
                ["Dark", "Light", "Auto"][self.custom_mode_at()].to_string(),
            ),
            (T::Panels, _) => ("Panels stand out", pct(k.panels)),
            (T::Corners, _) => ("Corners", CORNERS[self.custom_corners_at()].1.to_string()),
            (T::Radius, _) => ("Corner size", format!("{} px", c.theme.radius)),
            (T::Outlines, _) => ("Outlines", pct(k.outlines)),
            (T::Glow, _) => ("Glow", byte(c.theme.glow)),
            (T::TextGlow, _) => ("Text glow", byte(c.theme.text_glow)),
            (T::Tint, _) => (
                "Tint pads",
                format!("{:.0} %", c.theme.tint as f32 * 100.0 / 255.0),
            ),
            (T::Pattern, _) => ("Pattern size", format!("{} px", c.theme.backdrop_step)),
            (T::Backdrop, _) => (
                "Backdrop",
                BACKDROPS[self.custom_backdrop_at()].1.to_string(),
            ),
            (T::BackdropColours, _) => ("Backdrop colours · Fade and patterns", String::new()),
            (T::Tracks, _) => ("Track colours", String::new()),
            (T::TrackSet, _) => ("Set", track_set_word(c.track_set).to_string()),
        }
    }

    /// Row `ti` as Customise draws it.
    pub(super) fn custom_row(&self, ti: ThemeItem) -> RowView {
        let Some(c) = &self.custom else {
            return RowView::default();
        };
        let (name, value) = self.custom_text(ti);
        let row = |control: Control| RowView {
            name: name.to_string(),
            value: value.clone(),
            control: Some(control),
            ..RowView::default()
        };
        let swatches = |list: &[(&str, &str)], numbered: bool| {
            let at = c
                .pick
                .as_ref()
                .and_then(|(k, _)| list.iter().position(|(x, _)| x == k));
            Control::Swatches {
                colours: list
                    .iter()
                    .map(|(k, w)| (w.to_string(), theme_edit::colour(&c.theme, k)))
                    .collect(),
                at,
                numbered,
            }
        };
        let dial = |pos: f32, dot: Option<Color>, share: u8| RowView {
            share,
            ..row(Control::Dial { pos, dot })
        };
        let k = &c.knobs;
        let patterned = matches!(
            c.theme.backdrop,
            Backdrop::Grid | Backdrop::Dots | Backdrop::Scanlines
        );
        let changes = c.changes();
        match (ti, &c.pick) {
            (T::Name, _) => RowView {
                desc: c.line(),
                value: String::new(),
                share: 8,
                name: c.theme.name.clone(),
                ..row(Control::Button("Rename".to_string()))
            },
            (T::Undo, _) => RowView {
                bare: true,
                share: 2,
                faded: changes == 0,
                ..row(Control::Button("Undo changes".to_string()))
            },
            (T::Save, _) => RowView {
                bare: true,
                share: 2,
                strong: true,
                faded: !c.unsaved(),
                ..row(Control::Button("Save theme".to_string()))
            },
            (T::Dial(0), Some((key, h))) => {
                dial(h.h / 360.0, Some(theme_edit::colour(&c.theme, key)), 1)
            }
            (T::Dial(1), Some((_, h))) => dial(h.s, None, 1),
            (T::Dial(2), Some((_, h))) => dial(h.l, None, 1),
            (T::Dial(_), Some(_)) => RowView {
                bare: true,
                share: 1,
                ..row(Control::Button("Done".to_string()))
            },
            (T::Dial(0), None) => dial(k.accent / 360.0, Some(c.theme.accent), 1),
            (T::Dial(1), None) => dial(k.accent2 / 360.0, Some(c.theme.accent2), 1),
            (T::Dial(2), None) => dial(k.lightness, None, 1),
            (T::Dial(_), None) => dial((k.warmth + 1.0) / 2.0, None, 1),
            (T::Colours, _) => RowView {
                value: String::new(),
                ..row(swatches(&theme_edit::EVERY_COLOUR, false))
            },
            (T::Mode, _) => RowView {
                share: 4,
                value: String::new(),
                ..row(Control::Choice {
                    options: ["Dark", "Light", "Auto"]
                        .iter()
                        .map(|o| (o.to_string(), String::new()))
                        .collect(),
                    at: self.custom_mode_at(),
                    now: None,
                })
            },
            (T::Panels, _) => RowView {
                share: 5,
                ..row(Control::Slider { pos: k.panels })
            },
            (T::Corners, _) => RowView {
                bare: true,
                share: 5,
                ..row(Control::Looks {
                    options: CORNERS
                        .iter()
                        .map(|&(c, w)| (w.to_string(), Look::Corners(c)))
                        .collect(),
                    at: self.custom_corners_at(),
                })
            },
            (T::Radius, _) => dial(c.theme.radius as f32 / 40.0, None, 2),
            (T::Outlines, _) => dial(k.outlines, None, 2),
            (T::Glow, _) => dial(c.theme.glow as f32 / 255.0, None, 1),
            (T::TextGlow, _) => dial(c.theme.text_glow as f32 / 255.0, None, 1),
            (T::Tint, _) => dial(c.theme.tint as f32 / 255.0, None, 1),
            (T::Pattern, _) => RowView {
                faded: !patterned,
                ..dial(
                    (c.theme.backdrop_step - PATTERN_MIN) as f32
                        / (PATTERN_MAX - PATTERN_MIN) as f32,
                    None,
                    1,
                )
            },
            (T::Backdrop, _) => RowView {
                value: String::new(),
                ..row(Control::Looks {
                    options: BACKDROPS
                        .iter()
                        .map(|&(b, w)| (w.to_string(), Look::Backdrop(b)))
                        .collect(),
                    at: self.custom_backdrop_at(),
                })
            },
            (T::BackdropColours, _) => RowView {
                faded: c.theme.backdrop == Backdrop::Solid,
                ..row(swatches(&theme_edit::BACKDROP_COLOURS, false))
            },
            (T::Tracks, _) => {
                let list: Vec<(String, String)> = (1..=8)
                    .map(|n| (format!("track.{n}"), n.to_string()))
                    .collect();
                let at = c
                    .pick
                    .as_ref()
                    .and_then(|(k, _)| list.iter().position(|(x, _)| x == k));
                RowView {
                    bare: true,
                    ..row(Control::Swatches {
                        colours: list
                            .iter()
                            .map(|(k, w)| (w.clone(), theme_edit::colour(&c.theme, k)))
                            .collect(),
                        at,
                        numbered: true,
                    })
                }
            }
            (T::TrackSet, _) => RowView {
                value: String::new(),
                ..row(Control::Stepper(value.clone()))
            },
        }
    }

    /// The Colours card's note: what it does, or the colour being set.
    pub(super) fn custom_note(&self, note: &str) -> String {
        match self.custom.as_ref().and_then(|c| c.pick_word()) {
            Some(w) if note == "the greys move together" => format!("setting {w}"),
            _ => note.to_string(),
        }
    }

    /// Customise's hint line: how well the copy reads, and the keys.
    pub(super) fn custom_hint(&self) -> Option<String> {
        let c = self.custom.as_ref()?;
        let (words, dim, accent) = theme_edit::readability(&c.theme);
        let keys = if self.menu_editing {
            "Wheel: change it   ·   push the wheel: set it"
        } else {
            "Wheel: pick a control   ·   push the wheel, then turn: change it"
        };
        Some(format!(
            "Readable: words {words:.1}:1, dim {dim:.1}:1, accent {accent:.1}:1   ·   {keys}   ·   - / +: change it   ·   MENU: close"
        ))
    }
}

/// `v` stepped by `by`, kept to 0 to 255.
fn step_byte(v: u8, by: i32) -> u8 {
    (v as i32 + by).clamp(0, 255) as u8
}

/// The track colours as `from` had them, with its Q-Links, graph lines
/// and EQ bands.
fn restore_tracks(t: &mut Theme, from: &Theme) {
    theme_edit::set_tracks(t, &from.track);
    for key in (1..=4)
        .map(|n| format!("qlink.{n}"))
        .chain((1..=8).map(|n| format!("series.{n}")))
    {
        if let Some(r) = Role::from_key(&key) {
            let _ = t.set_role(r, from.role_source(r).clone());
        }
    }
}

/// The track colours' set, in words.
fn track_set_word(set: Option<usize>) -> &'static str {
    match set {
        Some(0) => "The theme's own",
        Some(k) => theme_edit::PALETTES
            .get(k - 1)
            .copied()
            .unwrap_or("The theme's own"),
        None => "Your own",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(fx: &[Effect]) -> crate::prefs_page::PrefsView {
        match fx.iter().rev().find(|e| matches!(e, Effect::Menu(_))) {
            Some(Effect::Menu(m)) => *m.prefs.clone().expect("Prefs"),
            _ => panic!("no menu: {fx:?}"),
        }
    }

    fn live(fx: &[Effect]) -> Option<Option<String>> {
        fx.iter().rev().find_map(|e| match e {
            Effect::LiveTheme(t) => Some(t.as_ref().map(|t| t.name.clone())),
            _ => None,
        })
    }

    fn at(app: &mut App, t: ThemeItem) -> usize {
        app.prefs_go_to(Item::Theme(t)).expect("a Customise row")
    }

    fn open() -> App {
        let mut app = App::new(44_100, false);
        app.open_tab(View::Prefs);
        app
    }

    fn line(app: &App) -> String {
        app.custom.as_ref().unwrap().line()
    }

    /// The copy shows while Customise is open, and waits while it isn't.
    #[test]
    fn customise_shows_a_copy_of_the_theme_in_use_only_while_open() {
        let mut app = open();
        let fx = app.pick_prefs_tab(PrefsTab::Customise.index());
        assert_eq!(live(&fx), Some(Some("My theme".to_string())));
        let v = view(&fx);
        assert!(v
            .tabs
            .iter()
            .any(|t| t.sub && t.name == "Customise" && t.lit));
        assert!(v.hint.as_deref().unwrap().starts_with("Readable: words "));
        let accent = at(&mut app, T::Dial(0));
        app.step_item(accent, 1);
        let fx = app.pick_prefs_tab(PrefsTab::Playing.index());
        assert_eq!(live(&fx), Some(None), "the theme in use back");
        assert!(
            view(&fx).tabs.iter().all(|t| !t.sub),
            "views only under Appearance"
        );
        let fx = app.pick_prefs_tab(PrefsTab::Customise.index());
        assert_eq!(live(&fx), Some(Some("My theme".to_string())));
        assert!(line(&app).contains("1 change"), "{}", line(&app));
        // MENU closes Prefs: the theme in use again.
        let fx = app.button(button::MENU, true, 0.0);
        assert_eq!(live(&fx), Some(None), "{fx:?}");
        assert_eq!(app.settings.theme, "default");
    }

    /// A knob moves the copy; Undo changes puts it back.
    #[test]
    fn a_knob_changes_the_copy_and_undo_changes_puts_it_back() {
        let mut app = open();
        let accent = at(&mut app, T::Dial(0));
        let before = app.custom.as_ref().unwrap().theme.accent;
        let fx = app.step_item(accent, 1);
        let after = app.custom.as_ref().unwrap().theme.accent;
        assert_ne!(before, after);
        assert!((theme_edit::to_hsl(after).h - theme_edit::to_hsl(before).h - 5.0).abs() < 2.0);
        assert!(matches!(live(&fx), Some(Some(_))));
        let v = view(&fx);
        assert_eq!(v.rows[accent].value, "221°");
        assert!(line(&app).contains("1 change"));
        let undo = at(&mut app, T::Undo);
        assert!(!view(&[app.menu_view()]).rows[undo].faded);
        app.step_item(undo, 1);
        assert_eq!(app.custom.as_ref().unwrap().theme.accent, before);
        assert!(view(&[app.menu_view()]).rows[undo].faded);
        // The other knobs and controls each change something.
        for (t, d) in [
            (T::Dial(1), 1),
            (T::Dial(2), 1),
            (T::Dial(3), 1),
            (T::Panels, 1),
            (T::Radius, -1),
            (T::Outlines, 1),
            (T::Glow, 1),
            (T::TextGlow, 1),
            (T::Tint, 1),
            (T::Corners, 1),
            (T::Backdrop, 1),
            (T::Pattern, 1),
            (T::TrackSet, 1),
        ] {
            let i = at(&mut app, t);
            let was = app.custom.as_ref().unwrap().theme.clone();
            app.step_item(i, d);
            assert_ne!(app.custom.as_ref().unwrap().theme, was, "{t:?}");
        }
    }

    /// A colour tapped turns the Colours card's knobs into its hue, colour
    /// and light, until Done.
    #[test]
    fn a_colour_tapped_is_set_by_its_hue_colour_and_light() {
        let mut app = open();
        let colours = at(&mut app, T::Colours);
        let fx = app.choose(colours, 6);
        let v = view(&fx);
        let dial = at(&mut app, T::Dial(0));
        assert_eq!(v.rows[dial].name, "Hue");
        assert!(matches!(
            v.rows[colours].control,
            Some(Control::Swatches { at: Some(6), .. })
        ));
        assert!(v.cards.iter().any(|c| c.note == "setting Good"));
        let light = at(&mut app, T::Dial(2));
        let was = app.custom.as_ref().unwrap().theme.good;
        app.set_value(light, 300);
        let good = app.custom.as_ref().unwrap().theme.good;
        assert_ne!(good, was);
        assert!((theme_edit::to_hsl(good).l - 0.3).abs() < 0.01);
        let done = at(&mut app, T::Dial(3));
        assert!(app.custom_opens(Item::Theme(T::Dial(3))));
        app.select(done);
        app.wheel_push();
        assert!(app.custom.as_ref().unwrap().pick.is_none());
        // A track's colour: the set becomes your own.
        let tracks = at(&mut app, T::Tracks);
        app.choose(tracks, 2);
        let hue = at(&mut app, T::Dial(0));
        app.step_item(hue, 1);
        let set = at(&mut app, T::TrackSet);
        assert_eq!(
            view(&[app.menu_view()]).rows[set].control,
            Some(Control::Stepper("Your own".into()))
        );
        assert_eq!(
            app.custom.as_ref().unwrap().theme.qlink(2),
            app.custom.as_ref().unwrap().theme.track[2]
        );
    }

    /// No change may leave the words unreadable: it isn't made, and the
    /// banner says why.
    #[test]
    fn a_change_that_would_leave_the_words_unreadable_is_not_made() {
        let mut app = open();
        let colours = at(&mut app, T::Colours);
        app.choose(colours, 4);
        let was = app.custom.as_ref().unwrap().theme.clone();
        let light = at(&mut app, T::Dial(2));
        app.set_value(light, 30);
        assert_eq!(app.custom.as_ref().unwrap().theme, was);
        assert!(line(&app).starts_with("Stopped there"), "{}", line(&app));
        let colour = at(&mut app, T::Dial(1));
        app.step_item(colour, 1);
        assert!(!line(&app).starts_with("Stopped there"));
    }

    /// Mode turns the copy light, still readable.
    #[test]
    fn mode_light_turns_the_copy_light() {
        let mut app = open();
        let mode = at(&mut app, T::Mode);
        app.choose(mode, 1);
        let t = &app.custom.as_ref().unwrap().theme;
        assert!(t.is_light());
        assert!(t.unusable().is_none());
        assert_eq!(app.custom_text(T::Mode).1, "Light");
    }

    /// Save theme writes the copy to the themes folder with only what
    /// changed, and it is the theme in use from then on.
    #[test]
    fn save_theme_writes_a_theme_file_and_puts_it_in_use() {
        let mut app = open();
        let name = at(&mut app, T::Name);
        assert_eq!(
            app.step_item(name, 1),
            vec![Effect::NameTheme("My theme".into())]
        );
        app.name_theme("  Night  ");
        let save = at(&mut app, T::Save);
        let corners = at(&mut app, T::Corners);
        app.choose(corners, 2);
        let fx = app.step_item(save, 1);
        let (file, text, theme) = fx
            .iter()
            .find_map(|e| match e {
                Effect::SaveTheme { file, text, theme } => {
                    Some((file.clone(), text.clone(), theme.clone()))
                }
                _ => None,
            })
            .expect("saved");
        assert_eq!(file, "night.theme");
        assert_eq!(Theme::parse(&text).unwrap(), *theme);
        assert!(text.contains("base          = default"));
        assert!(text.contains("corners       = square"));
        assert_eq!(app.settings.theme, "night.theme");
        assert!(fx
            .iter()
            .any(|e| matches!(e, Effect::Settings(s) if s.theme == "night.theme")));
        assert!(app.offers_theme("night.theme"));
        assert!(line(&app).starts_with("Saved as night.theme"));
        assert!(view(&[app.menu_view()]).rows[save].faded);
        assert!(app.step_item(save, 1).is_empty(), "nothing new to save");
        // A change saves to the same file again.
        let radius = at(&mut app, T::Radius);
        app.step_item(radius, 1);
        let fx = app.step_item(save, 1);
        assert!(fx
            .iter()
            .any(|e| matches!(e, Effect::SaveTheme { file, .. } if file == "night.theme")));
        assert_eq!(
            app.theme_files
                .iter()
                .filter(|f| f.0 == "night.theme")
                .count(),
            1
        );
        // Leaving Customise: the saved theme is the one in use.
        let fx = app.pick_prefs_tab(0);
        assert_eq!(live(&fx), Some(None));
        assert_eq!(app.settings.theme, "night.theme");
    }

    /// The New theme tile and Customise › open Customise: a theme on trial
    /// is kept and copied.
    #[test]
    fn new_theme_copies_the_theme_on_trial() {
        let mut app = open();
        let at_row = app.prefs_go_to(Item::Appearance).unwrap();
        let console = app.theme_choices().position(|t| t.0 == "console").unwrap();
        app.choose(at_row, console);
        let fx = app.customise_from_themes(true);
        assert_eq!(app.settings.theme, "console");
        assert!(fx.contains(&Effect::Note("theme: kept Console".into())));
        let c = app.custom.as_ref().unwrap();
        assert_eq!(
            (c.base.as_str(), c.from_label.as_str()),
            ("console", "Console")
        );
        assert_eq!(c.theme.corners, Corners::Chamfer);
        assert_eq!(app.prefs_tab, PrefsTab::Customise);
        // The gallery offers both.
        app.pick_prefs_tab(PrefsTab::Themes.index());
        let g = view(&[app.menu_view()]).gallery.unwrap();
        assert!(g.new_theme && g.customise);
    }
}
