//! Prefs' tabs (layout spec, section 5): the sidebar's five tabs, which
//! settings each shows in which card, and how each is drawn and changed
//! ([`crate::prefs_page`] draws them). The redesign canvas, round 2, Jay's
//! go 24 Nov 2026 17:35Z.

use super::customise::{ThemeItem, CUSTOMISE_ROWS};
use super::*;
use crate::prefs_page::{
    CardView, Control, Gallery, Hue, Picture, PrefsView, RowView, SideTab, Split, ThemeTile,
};

/// The sidebar's tabs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PrefsTab {
    #[default]
    Playing,
    Sound,
    /// Appearance: the themes gallery.
    Themes,
    /// Appearance: Customise, a copy of the theme in use changed.
    Customise,
    /// Appearance: Screen and lights, the pads' lights and the hint line.
    Lights,
    Projects,
    System,
}

/// One card of a tab: its title, the dim words after it, its colour, its
/// column, whether it takes the column's spare room, and its settings.
struct CardPlan {
    title: &'static str,
    note: &'static str,
    hue: Hue,
    column: u8,
    fill: bool,
    rows: &'static [Item],
    /// System info's list after the first row.
    info: bool,
    /// The two memories' total at the foot.
    split: bool,
}

const fn card(
    title: &'static str,
    note: &'static str,
    hue: Hue,
    column: u8,
    rows: &'static [Item],
) -> CardPlan {
    CardPlan {
        title,
        note,
        hue,
        column,
        fill: false,
        rows,
        info: false,
        split: false,
    }
}

const PLAYING: &[CardPlan] = &[
    CardPlan {
        fill: true,
        ..card(
            "Pads",
            "how hard you hit",
            Hue::Cyan,
            0,
            &[
                Item::Curve,
                Item::CurveShape,
                Item::CurveSoftest,
                Item::CurveLoudest,
                Item::PadThreshold,
            ],
        )
    },
    card(
        "Latch and pedal",
        "what keeps playing",
        Hue::Coral,
        1,
        &[Item::LatchMode, Item::PedalLatch],
    ),
    card(
        "Touch screen",
        "pads tapped on the screen",
        Hue::Pink,
        1,
        &[Item::ScreenVelocity],
    ),
    card(
        "Controls",
        "the Q-Links and the data wheel",
        Hue::Lime,
        1,
        &[
            Item::QLinkSpeed,
            Item::WheelSpeed,
            Item::WheelSpeedUp,
            Item::FineShift,
        ],
    ),
];

const SOUND: &[CardPlan] = &[
    card(
        "Audio",
        "a change restarts the sound",
        Hue::Cyan,
        0,
        &[Item::Buffer, Item::SoundCores],
    ),
    CardPlan {
        split: true,
        ..card(
            "Memory",
            "set aside when MPFree starts",
            Hue::Grey,
            0,
            &[Item::LoopMemory, Item::AudioMemory],
        )
    },
    card(
        "Line input",
        "the INPUTS on the back",
        Hue::Coral,
        1,
        &[Item::LineInput, Item::LoopLatency, Item::InputMeter],
    ),
    card(
        "MIDI and CV out",
        "ports and sync: Mix › I/O",
        Hue::Lime,
        1,
        &[Item::SendsMidi],
    ),
    card(
        "Listening",
        "",
        Hue::Pink,
        1,
        &[Item::AuditionLevel, Item::MetronomeSound],
    ),
];

const THEMES: &[CardPlan] = &[card("Themes", "", Hue::Grey, 0, &[Item::Appearance])];

const LIGHTS: &[CardPlan] = &[
    card(
        "Pad lights",
        "the 16 pads on the MPC",
        Hue::Pink,
        0,
        &[Item::PadBrightness, Item::HeldPad, Item::EmptyPads],
    ),
    card("Screen", "on every page", Hue::Cyan, 0, &[Item::HintLine]),
    CardPlan {
        fill: true,
        ..card(
            "Pads show",
            "as they'd light",
            Hue::Lime,
            1,
            &[Item::PadsShow],
        )
    },
];

const fn th(t: ThemeItem) -> Item {
    Item::Theme(t)
}

const CUSTOMISE: &[CardPlan] = &[
    card(
        "",
        "",
        Hue::Grey,
        2,
        &[
            th(ThemeItem::Name),
            th(ThemeItem::Undo),
            th(ThemeItem::Save),
        ],
    ),
    card(
        "Colours",
        "the greys move together",
        Hue::Cyan,
        0,
        &[
            th(ThemeItem::Dial(0)),
            th(ThemeItem::Dial(1)),
            th(ThemeItem::Dial(2)),
            th(ThemeItem::Dial(3)),
            th(ThemeItem::Colours),
            th(ThemeItem::Mode),
            th(ThemeItem::Panels),
        ],
    ),
    card(
        "Shape",
        "sizes and places never change",
        Hue::Coral,
        0,
        &[
            th(ThemeItem::Corners),
            th(ThemeItem::Radius),
            th(ThemeItem::Outlines),
        ],
    ),
    card(
        "Effects",
        "drawn under every page",
        Hue::Pink,
        1,
        &[
            th(ThemeItem::Glow),
            th(ThemeItem::TextGlow),
            th(ThemeItem::Tint),
            th(ThemeItem::Pattern),
            th(ThemeItem::Backdrop),
            th(ThemeItem::BackdropColours),
        ],
    ),
    card(
        "Track colours",
        "also the Q-Links, graphs and EQ bands",
        Hue::Lime,
        1,
        &[th(ThemeItem::Tracks), th(ThemeItem::TrackSet)],
    ),
];

const PROJECTS: &[CardPlan] = &[
    CardPlan {
        fill: true,
        ..card(
            "Export",
            "the Export panel starts from these",
            Hue::Coral,
            0,
            &[
                Item::Export,
                Item::ExportFile,
                Item::ExportRate,
                Item::StemsBy,
            ],
        )
    },
    card(
        "At start",
        "once the sound is going",
        Hue::Cyan,
        1,
        &[Item::AtStart],
    ),
    card(
        "Sets",
        "how Next song goes",
        Hue::Pink,
        1,
        &[Item::QuickAdvance],
    ),
];

const SYSTEM: &[CardPlan] = &[
    CardPlan {
        fill: true,
        info: true,
        ..card(
            "This MPC",
            "",
            Hue::Grey,
            0,
            &[Item::SystemStatus, Item::SystemInfo],
        )
    },
    card(
        "Backup and Akai kits",
        "at the next start, stick in",
        Hue::Cyan,
        1,
        &[Item::SetupBackup, Item::SetupAkaiKits],
    ),
    card(
        "Buttons and touch",
        "checks",
        Hue::Lime,
        1,
        &[Item::FindButtons, Item::CalibrateTouch, Item::TouchLab],
    ),
    card(
        "Developers",
        "",
        Hue::Coral,
        0,
        &[Item::DebugMode, Item::MapControls],
    ),
];

/// Each tab's settings in the wheel's order: the left column's cards top
/// to bottom, then the right's (a test keeps these in step with the
/// cards).
const PLAYING_ROWS: &[Item] = &[
    Item::Curve,
    Item::CurveShape,
    Item::CurveSoftest,
    Item::CurveLoudest,
    Item::PadThreshold,
    Item::LatchMode,
    Item::PedalLatch,
    Item::ScreenVelocity,
    Item::QLinkSpeed,
    Item::WheelSpeed,
    Item::WheelSpeedUp,
    Item::FineShift,
];
const SOUND_ROWS: &[Item] = &[
    Item::Buffer,
    Item::SoundCores,
    Item::LoopMemory,
    Item::AudioMemory,
    Item::LineInput,
    Item::LoopLatency,
    Item::InputMeter,
    Item::SendsMidi,
    Item::AuditionLevel,
    Item::MetronomeSound,
];
const THEMES_ROWS: &[Item] = &[Item::Appearance];
const LIGHTS_ROWS: &[Item] = &[
    Item::PadBrightness,
    Item::HeldPad,
    Item::EmptyPads,
    Item::HintLine,
    Item::PadsShow,
];
const PROJECTS_ROWS: &[Item] = &[
    Item::Export,
    Item::ExportFile,
    Item::ExportRate,
    Item::StemsBy,
    Item::AtStart,
    Item::QuickAdvance,
];
const SYSTEM_ROWS: &[Item] = &[
    Item::SystemStatus,
    Item::SystemInfo,
    Item::DebugMode,
    Item::MapControls,
    Item::SetupBackup,
    Item::SetupAkaiKits,
    Item::FindButtons,
    Item::CalibrateTouch,
    Item::TouchLab,
];

/// The most frames the latency knob turns to (46 ms at 44.1 kHz); - and +
/// go on to 4096.
const LATENCY_KNOB_FRAMES: f32 = 2048.0;

impl PrefsTab {
    /// Every tab in SHIFT + the wheel's order, Appearance's views
    /// (Themes, Customise, Screen and lights) where Appearance sits.
    pub const ALL: [PrefsTab; 7] = [
        PrefsTab::Playing,
        PrefsTab::Sound,
        PrefsTab::Themes,
        PrefsTab::Customise,
        PrefsTab::Lights,
        PrefsTab::Projects,
        PrefsTab::System,
    ];

    /// The sidebar's tabs, each with its drawing's number.
    const SIDEBAR: [PrefsTab; 5] = [
        PrefsTab::Playing,
        PrefsTab::Sound,
        PrefsTab::Themes,
        PrefsTab::Projects,
        PrefsTab::System,
    ];

    /// Whether the sidebar has a Reset button on it (Playing, Sound,
    /// Projects: the tabs of plain settings).
    pub fn resets(self) -> bool {
        matches!(
            self,
            PrefsTab::Playing | PrefsTab::Sound | PrefsTab::Projects
        )
    }

    /// Its tab's name in the sidebar (Appearance for its views).
    pub fn name(self) -> &'static str {
        match self {
            PrefsTab::Playing => "Playing",
            PrefsTab::Sound => "Sound",
            PrefsTab::Themes | PrefsTab::Customise | PrefsTab::Lights => "Appearance",
            PrefsTab::Projects => "Projects",
            PrefsTab::System => "System",
        }
    }

    /// One of Appearance's views: its name under Appearance.
    pub fn view_name(self) -> Option<&'static str> {
        match self {
            PrefsTab::Themes => Some("Themes"),
            PrefsTab::Customise => Some("Customise"),
            PrefsTab::Lights => Some("Screen and lights"),
            _ => None,
        }
    }

    /// The line under its name in the sidebar.
    pub fn line(self) -> &'static str {
        match self {
            PrefsTab::Playing => "Pads, latch, controls",
            PrefsTab::Sound => "Buffer, memory, input",
            PrefsTab::Themes | PrefsTab::Customise | PrefsTab::Lights => "Themes and lights",
            PrefsTab::Projects => "Start, export, sets",
            PrefsTab::System => "Info, setup, checks",
        }
    }

    fn cards(self) -> &'static [CardPlan] {
        match self {
            PrefsTab::Playing => PLAYING,
            PrefsTab::Sound => SOUND,
            PrefsTab::Themes => THEMES,
            PrefsTab::Customise => CUSTOMISE,
            PrefsTab::Lights => LIGHTS,
            PrefsTab::Projects => PROJECTS,
            PrefsTab::System => SYSTEM,
        }
    }

    /// Its settings, in the wheel's order.
    pub fn rows(self) -> &'static [Item] {
        match self {
            PrefsTab::Playing => PLAYING_ROWS,
            PrefsTab::Sound => SOUND_ROWS,
            PrefsTab::Themes => THEMES_ROWS,
            PrefsTab::Customise => CUSTOMISE_ROWS,
            PrefsTab::Lights => LIGHTS_ROWS,
            PrefsTab::Projects => PROJECTS_ROWS,
            PrefsTab::System => SYSTEM_ROWS,
        }
    }

    /// The tab that shows `item`, if Prefs shows it.
    pub fn of(item: Item) -> Option<PrefsTab> {
        PrefsTab::ALL.into_iter().find(|t| t.rows().contains(&item))
    }

    /// Its number, as taps on the sidebar give it.
    pub fn index(self) -> usize {
        PrefsTab::ALL.iter().position(|&t| t == self).unwrap_or(0)
    }

    /// One of Appearance's views.
    fn appearance(self) -> bool {
        self.view_name().is_some()
    }
}

/// `(name, line)` pairs from string slices.
fn options(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect()
}

impl App {
    /// A tab of the sidebar picked (a tap, or SHIFT + the wheel). Leaving
    /// the themes while "Keep this theme?" asks puts the old theme back,
    /// as closing Prefs does.
    pub fn pick_prefs_tab(&mut self, i: usize) -> Vec<Effect> {
        let Some(&tab) = PrefsTab::ALL.get(i) else {
            return Vec::new();
        };
        if self.view != View::Prefs || tab == self.prefs_tab {
            return Vec::new();
        }
        let mut fx = Vec::new();
        if self.theme_trial.is_some() {
            fx = self.end_theme_trial(false);
            fx.retain(|e| !matches!(e, Effect::Menu(_)));
        }
        fx.extend(self.custom_leave());
        self.prefs_tab = tab;
        self.selected = 0;
        self.menu_editing = false;
        self.reset_armed = false;
        if tab == PrefsTab::Customise {
            fx.extend(self.custom_open(false));
        }
        fx.push(self.menu_view());
        fx
    }

    /// How the pads light, as Prefs › Screen and lights sets it; Track
    /// and Accent rest in the theme in use's colours (the armed track's).
    pub fn pad_look(&self) -> crate::pad_look::PadLook {
        use crate::pad_look::{rest_level, PadLook, PadsShow};
        let s = &self.settings;
        let tint = match s.pads_show {
            PadsShow::Kit | PadsShow::Heat => [0; 3],
            show => {
                let theme = self.theme_in_use();
                rest_level(if show == PadsShow::Accent {
                    theme.accent
                } else {
                    theme.track[self.track() % theme.track.len()]
                })
            }
        };
        PadLook {
            brightness: s.pad_brightness,
            held: s.held_pad,
            show: s.pads_show,
            empty: s.empty_pads,
            tint,
        }
    }

    /// The theme `settings.theme` names: built in, or a theme file's.
    fn theme_in_use(&self) -> mpfree_ui::Theme {
        let key = &self.settings.theme;
        mpfree_ui::Theme::builtin(key).unwrap_or_else(|| {
            self.theme_looks
                .iter()
                .find(|(f, _)| f == key)
                .map_or_else(mpfree_ui::Theme::dark, |(_, t)| t.clone())
        })
    }

    /// SHIFT + the wheel in Prefs: the tab after or before, stopping at
    /// the ends.
    pub(super) fn step_prefs_tab(&mut self, delta: i32) -> Vec<Effect> {
        let at = self.prefs_tab.index() as i32 + delta.signum();
        let last = PrefsTab::ALL.len() as i32 - 1;
        self.pick_prefs_tab(at.clamp(0, last) as usize)
    }

    /// Opens the tab that shows `item` (for tests, previews and the device
    /// checks' scripts) and gives its row there.
    pub fn prefs_go_to(&mut self, item: Item) -> Option<usize> {
        let tab = PrefsTab::of(item)?;
        if self.view == View::Prefs && tab != self.prefs_tab {
            self.pick_prefs_tab(tab.index());
        }
        self.row_of(item)
    }

    /// Choice `k` of row `i` picked on the screen (a button of a choice, a
    /// tile, a step of a slider, a theme). The settings move straight
    /// there, telling the rest of the app once.
    pub fn choose(&mut self, i: usize, k: usize) -> Vec<Effect> {
        if self.view != View::Prefs {
            return Vec::new();
        }
        let items = self.items();
        let i = i.min(items.len() - 1);
        let item = items[i];
        self.selected = i;
        self.menu_editing = false;
        self.reset_armed = false;
        if item == Item::Appearance {
            return self.pick_theme(k);
        }
        if let Item::Theme(ti) = item {
            return self.custom_choose(ti, k);
        }
        let Some((_, at)) = self.prefs_choices(item) else {
            return vec![self.menu_view()];
        };
        if k == at {
            return vec![self.menu_view()];
        }
        let d = if k > at { 1 } else { -1 };
        let mut fx = Vec::new();
        for _ in 0..k.abs_diff(at) {
            fx.extend(self.apply_item(item, d));
        }
        // Each step said the settings and the menu: once is enough.
        let last = |fx: &[Effect], f: fn(&Effect) -> bool| fx.iter().rposition(f);
        let settings = last(&fx, |e| matches!(e, Effect::Settings(_)));
        let menu = last(&fx, |e| matches!(e, Effect::Menu(_)));
        fx.into_iter()
            .enumerate()
            .filter(|(n, e)| match e {
                Effect::Settings(_) => Some(*n) == settings,
                Effect::Menu(_) => Some(*n) == menu,
                _ => true,
            })
            .map(|(_, e)| e)
            .collect()
    }

    /// Row `i`'s slider or knob moved to `permille` (0 to 1000).
    pub fn set_value(&mut self, i: usize, permille: i32) -> Vec<Effect> {
        if self.view != View::Prefs {
            return Vec::new();
        }
        let items = self.items();
        let i = i.min(items.len() - 1);
        self.selected = i;
        self.menu_editing = false;
        let p = permille.clamp(0, 1000) as f32 / 1000.0;
        self.reset_armed = false;
        if let Item::Theme(ti) = items[i] {
            return self.custom_value(ti, p);
        }
        let s = &mut self.settings;
        let mut fx = Vec::new();
        match items[i] {
            Item::ScreenVelocity => {
                let v = (10.0 + p * 117.0).round() as u8;
                if v == s.screen_velocity {
                    return Vec::new();
                }
                s.screen_velocity = v;
            }
            Item::PadBrightness => {
                use crate::pad_look::{BRIGHTNESS_MAX, BRIGHTNESS_MIN};
                let span = f32::from(BRIGHTNESS_MAX - BRIGHTNESS_MIN);
                let v = (f32::from(BRIGHTNESS_MIN) + p * span).round() as u8;
                if v == s.pad_brightness {
                    return Vec::new();
                }
                s.pad_brightness = v;
            }
            Item::LoopLatency => {
                let f = (p * LATENCY_KNOB_FRAMES / 16.0).round() as u32 * 16;
                if f == s.loop_latency && s.loop_latency_set {
                    return Vec::new();
                }
                s.loop_latency = f;
                s.loop_latency_set = true;
                fx.push(Effect::Looper(LoopAction::Command(
                    LoopCommand::SetLatency(s.loop_latency_now()),
                )));
            }
            Item::CurveShape | Item::CurveSoftest | Item::CurveLoudest => {
                let mut b = s.curve_bend;
                match items[i] {
                    Item::CurveShape => b.shape = (p * 200.0 - 100.0).round() as i8,
                    Item::CurveSoftest => b.softest = (1.0 + p * 126.0).round() as u8,
                    _ => b.loudest = (1.0 + p * 126.0).round() as u8,
                }
                if b == s.curve_bend && s.curve == Curve::Custom(b) {
                    return Vec::new();
                }
                s.curve_bend = b;
                s.curve = Curve::Custom(b);
            }
            Item::PadThreshold => {
                use crate::play_feel::{THRESHOLD_MAX, THRESHOLD_MIN};
                let span = f32::from(THRESHOLD_MAX - THRESHOLD_MIN);
                let v = (f32::from(THRESHOLD_MIN) + p * span).round() as u8;
                if v == s.pad_threshold {
                    return Vec::new();
                }
                s.pad_threshold = v;
            }
            Item::AuditionLevel => {
                use crate::preview::{LEVEL_MAX_DB, LEVEL_MIN_DB};
                let v = (LEVEL_MIN_DB + p * (LEVEL_MAX_DB - LEVEL_MIN_DB)).round() as i8;
                if v == s.audition_db {
                    return Vec::new();
                }
                s.audition_db = v;
            }
            Item::QLinkSpeed | Item::WheelSpeed | Item::WheelSpeedUp => {
                use crate::play_feel::{Speed, SpeedUp};
                let k = (p * 2.0).round() as usize;
                match items[i] {
                    Item::QLinkSpeed => s.qlink_speed = Speed::ALL[k],
                    Item::WheelSpeed => s.wheel_speed = Speed::ALL[k],
                    _ => s.wheel_speed_up = SpeedUp::ALL[k],
                }
            }
            _ => return Vec::new(),
        }
        self.save_dirty = true;
        fx.push(Effect::Settings(self.settings.clone()));
        fx.push(self.menu_view());
        fx
    }

    /// Theme `k` of the gallery (built in, then the themes folder's)
    /// tried: the screen switches now, and "Keep this theme?" asks, as
    /// stepping Appearance does.
    fn pick_theme(&mut self, k: usize) -> Vec<Effect> {
        let at = self
            .theme_choices()
            .position(|t| t.0 == self.settings.theme);
        let Some(at) = at else {
            // A theme file from --theme: the first step goes to the first.
            return self.apply_item(Item::Appearance, 1);
        };
        if k == at || k >= self.theme_choices().count() {
            return vec![self.menu_view()];
        }
        let d = if k > at { 1 } else { -1 };
        let mut fx = Vec::new();
        for _ in 0..k.abs_diff(at) {
            fx = self.apply_item(Item::Appearance, d);
        }
        fx
    }

    /// A choice setting's choices (each a name and the line under it) and
    /// the one chosen; None for the rest.
    fn prefs_choices(&self, item: Item) -> Option<(Vec<(String, String)>, usize)> {
        let s = &self.settings;
        let at = |all: &[bool]| all.iter().position(|&b| b).unwrap_or(0);
        Some(match item {
            Item::Curve => (
                options(&[
                    ("Soft", "loud is easy"),
                    ("Linear", "the pad's own"),
                    ("Hard", "more dynamics"),
                    ("Custom", "your own"),
                ]),
                s.curve.index(),
            ),
            Item::LatchMode => (
                options(&[
                    ("Replace", "a new set"),
                    ("Toggle", "add or remove"),
                    ("Add", "keep adding"),
                ]),
                at(&[
                    !matches!(s.latch_mode, LatchMode::Toggle | LatchMode::Add),
                    s.latch_mode == LatchMode::Toggle,
                    s.latch_mode == LatchMode::Add,
                ]),
            ),
            Item::StemsBy => (
                options(&[("Track", ""), ("Channel", "")]),
                usize::from(s.stems_by == mpfree_browser_ui::StemsBy::Channel),
            ),
            Item::MetronomeSound => (
                crate::time::ClickSound::ALL
                    .iter()
                    .map(|c| (c.name().to_string(), String::new()))
                    .collect(),
                s.metronome_sound as usize,
            ),
            Item::Buffer => (
                vec![
                    ("2 periods".to_string(), ms(2)),
                    ("3 periods".to_string(), format!("{} · as Akai", ms(3))),
                ],
                (s.periods.clamp(2, 3) - 2) as usize,
            ),
            Item::SoundCores => {
                let now = self.sound_cores_now;
                let line = |c: SoundCores| {
                    if c == now && c != s.sound_cores {
                        "now"
                    } else if c == s.sound_cores && c != now {
                        "next start"
                    } else {
                        ""
                    }
                };
                (
                    SoundCores::ALL
                        .iter()
                        .map(|&c| (cores_label(c).to_string(), line(c).to_string()))
                        .collect(),
                    SoundCores::ALL
                        .iter()
                        .position(|&c| c == s.sound_cores)
                        .unwrap_or(0),
                )
            }
            Item::LoopMemory => (
                LoopMemory::ALL
                    .iter()
                    .map(|m| (m.minutes().to_string(), String::new()))
                    .collect(),
                LoopMemory::ALL
                    .iter()
                    .position(|&m| m == s.loop_memory)
                    .unwrap_or(0),
            ),
            Item::AudioMemory => (
                crate::audiotracks::AudioMemory::ALL
                    .iter()
                    .map(|m| (memory_label(m.seconds()), String::new()))
                    .collect(),
                crate::audiotracks::AudioMemory::ALL
                    .iter()
                    .position(|&m| m == s.audio_memory)
                    .unwrap_or(0),
            ),
            Item::Export => (
                options(&[
                    ("Fast, quiet", "faster than real time"),
                    ("Listen", "in real time, you hear it"),
                ]),
                usize::from(s.export_mode == mpfree_export::Mode::Listen),
            ),
            Item::ExportFile => (
                options(&[("24-bit", ""), ("16-bit", ""), ("32-bit float", "")]),
                mpfree_fx::Bits::ALL
                    .iter()
                    .position(|&b| b == s.export_bits)
                    .unwrap_or(0),
            ),
            Item::ExportRate => (
                options(&[("44.1 kHz", ""), ("48 kHz", "")]),
                EXPORT_RATES
                    .iter()
                    .position(|&r| r == s.export_rate)
                    .unwrap_or(0),
            ),
            Item::AtStart => (
                options(&[
                    ("New project", "An empty song"),
                    (
                        "Last project",
                        "The one you last saved or opened; nothing after a crash",
                    ),
                ]),
                usize::from(s.open_last),
            ),
            Item::HeldPad => (
                options(&[
                    ("Brighter", "its own colour"),
                    ("White", ""),
                    ("Same", "no change"),
                ]),
                crate::pad_look::HeldPad::ALL
                    .iter()
                    .position(|&h| h == s.held_pad)
                    .unwrap_or(0),
            ),
            Item::PadsShow => (
                crate::pad_look::PadsShow::ALL
                    .iter()
                    .map(|p| (p.name().to_string(), p.line().to_string()))
                    .collect(),
                crate::pad_look::PadsShow::ALL
                    .iter()
                    .position(|&p| p == s.pads_show)
                    .unwrap_or(0),
            ),
            Item::EmptyPads => (
                options(&[("Dark", ""), ("Dim", "")]),
                usize::from(s.empty_pads == crate::pad_look::EmptyPads::Dim),
            ),
            Item::QuickAdvance => (
                options(&[
                    (
                        "Arm, then go",
                        "Next song arms the next song, and a second press goes",
                    ),
                    ("One press", "With the transport stopped, one press goes"),
                ]),
                usize::from(s.quick_advance),
            ),
            _ => return None,
        })
    }

    /// Row `item` as Prefs draws it.
    fn prefs_row(&self, item: Item) -> RowView {
        if let Item::Theme(ti) = item {
            return self.custom_row(ti);
        }
        let s = &self.settings;
        let (name, _) = self.item_text(item);
        let row = |desc: &str, control: Control| RowView {
            name: name.to_string(),
            desc: desc.to_string(),
            control: Some(control),
            ..RowView::default()
        };
        let choice = |desc: &str| {
            let (options, at) = self.prefs_choices(item).unwrap_or_default();
            let now = match item {
                Item::SoundCores => SoundCores::ALL
                    .iter()
                    .position(|&c| c == self.sound_cores_now)
                    .filter(|&n| n != at),
                _ => None,
            };
            row(desc, Control::Choice { options, at, now })
        };
        let tiles = || {
            let (options, at) = self.prefs_choices(item).unwrap_or_default();
            RowView {
                bare: true,
                ..row("", Control::Tiles { options, at })
            }
        };
        let steps = |desc: &str, value: String| {
            let (options, at) = self.prefs_choices(item).unwrap_or_default();
            let mut labels: Vec<String> = options.into_iter().map(|o| o.0).collect();
            if let Some(last) = labels.last_mut() {
                last.push_str(" min");
            }
            RowView {
                value,
                ..row(desc, Control::Steps { labels, at })
            }
        };
        let switch = |desc: &str, on: bool| row(desc, Control::Switch(on));
        let button = |desc: &str, label: &str| row(desc, Control::Button(label.to_string()));
        let next = |differs: bool| if differs { " · next start" } else { "" };
        match item {
            Item::Curve => RowView {
                picture: Some(Picture::Curve(
                    (0..=24)
                        .map(|n| {
                            let v = (n as f32 * 127.0 / 24.0).round() as u8;
                            (v as f32 / 127.0, s.curve.apply(v) as f32 / 127.0)
                        })
                        .collect(),
                    self.last_hit().map(|v| (v, s.curve.apply(v))),
                )),
                ..choice("How hard a pad must be hit for a loud note")
            },
            Item::CurveShape | Item::CurveSoftest | Item::CurveLoudest => {
                let b = s.curve_bend;
                let pos = match item {
                    Item::CurveShape => (f32::from(b.shape) + 100.0) / 200.0,
                    Item::CurveSoftest => f32::from(b.softest - 1) / 126.0,
                    _ => f32::from(b.loudest - 1) / 126.0,
                };
                RowView {
                    share: 1,
                    value: self.item_text(item).1,
                    // The custom curve's: a turn chooses Custom.
                    faded: !matches!(s.curve, Curve::Custom(_)),
                    ..row("", Control::Dial { pos, dot: None })
                }
            }
            Item::PadThreshold => RowView {
                value: s.pad_threshold.to_string(),
                ..row(
                    "How light a touch still plays a note (1 plays every hit)",
                    Control::Slider {
                        pos: f32::from(s.pad_threshold - crate::play_feel::THRESHOLD_MIN)
                            / f32::from(
                                crate::play_feel::THRESHOLD_MAX - crate::play_feel::THRESHOLD_MIN,
                            ),
                    },
                )
            },
            Item::QLinkSpeed | Item::WheelSpeed | Item::WheelSpeedUp => {
                let k = match item {
                    Item::QLinkSpeed => s.qlink_speed as usize,
                    Item::WheelSpeed => s.wheel_speed as usize,
                    _ => s.wheel_speed_up as usize,
                };
                RowView {
                    share: 1,
                    value: self.item_text(item).1,
                    ..row(
                        "",
                        Control::Dial {
                            pos: k as f32 / 2.0,
                            dot: None,
                        },
                    )
                }
            }
            Item::InputMeter => RowView {
                value: if s.line_input {
                    String::new()
                } else {
                    "turn it on to see its level".to_string()
                },
                faded: !s.line_input,
                ..row("", Control::Meter)
            },
            Item::AuditionLevel => RowView {
                value: self.item_text(item).1,
                ..row("How loud BROWSE plays a sound you pick", {
                    use crate::preview::{LEVEL_MAX_DB, LEVEL_MIN_DB};
                    Control::Slider {
                        pos: (f32::from(s.audition_db) - LEVEL_MIN_DB)
                            / (LEVEL_MAX_DB - LEVEL_MIN_DB),
                    }
                })
            },
            Item::MetronomeSound => RowView {
                share: 1,
                ..choice("")
            },
            Item::FineShift => switch("SHIFT + a Q-Link or the wheel turns finer", s.fine_shift),
            Item::LatchMode => choice("With Latch on, what a fresh press does"),
            Item::PedalLatch => switch("The pedal holds the arp as Latch does", s.pedal_latches),
            Item::ScreenVelocity => RowView {
                value: s.screen_velocity.to_string(),
                ..row(
                    "The screen can't feel how hard you tap",
                    Control::Slider {
                        pos: (f32::from(s.screen_velocity) - 10.0) / 117.0,
                    },
                )
            },
            Item::CalibrateTouch => button("Tap a target in each corner", "Start"),
            Item::TouchLab => button("Timed tests: taps, drags, swipes", "Open"),
            Item::Buffer => choice("Sound queued ahead of the speakers"),
            Item::SoundCores => choice("Which of the MPC's cores make the sound"),
            Item::LineInput => switch("Off every time MPFree starts", s.line_input),
            Item::LoopLatency => RowView {
                value: format!(
                    "{} frames · {:.1} ms{}",
                    s.loop_latency,
                    s.loop_latency as f64 / 44.1,
                    if s.loop_latency_set {
                        ""
                    } else {
                        ", estimated"
                    }
                ),
                ..row(
                    "How late the input reaches loops and takes",
                    Control::Knob {
                        pos: (s.loop_latency as f32 / LATENCY_KNOB_FRAMES).min(1.0),
                    },
                )
            },
            Item::SendsMidi => switch("MIDI OUT, clock and the CV/Gate jacks", s.sends_midi),
            Item::LoopMemory => steps(
                "All loops and layers, with 60 s kept for Capture",
                format!(
                    "{} min · {} MB{}",
                    s.loop_memory.minutes(),
                    s.loop_memory.megabytes(self.sample_rate),
                    next(s.loop_memory != self.loop_memory_now)
                ),
            ),
            Item::AudioMemory => steps(
                "Every take on the audio tracks",
                format!(
                    "{} · {} MB{}",
                    s.audio_memory.time(),
                    s.audio_memory.megabytes(self.sample_rate),
                    next(s.audio_memory != self.audio_memory_now)
                ),
            ),
            Item::Export => choice("Listen plays it through the speakers as it goes"),
            Item::ExportFile => choice("16-bit adds a little dither, for CDs and older gear"),
            Item::ExportRate => choice("48 kHz is for video, converted as the files are written"),
            Item::StemsBy => choice("One file for each track, or for each mixer channel"),
            Item::AtStart => tiles(),
            Item::QuickAdvance => tiles(),
            Item::PadBrightness => RowView {
                value: format!("{} %", s.pad_brightness),
                ..row(
                    "Every pad light, at rest and held",
                    Control::Slider {
                        pos: f32::from(s.pad_brightness - crate::pad_look::BRIGHTNESS_MIN)
                            / f32::from(
                                crate::pad_look::BRIGHTNESS_MAX - crate::pad_look::BRIGHTNESS_MIN,
                            ),
                    },
                )
            },
            Item::HeldPad => choice("While you hold a pad down"),
            Item::EmptyPads => choice("Pads with no sound on them"),
            Item::HintLine => switch("The help line along the foot of every page", s.hint_line),
            Item::PadsShow => RowView {
                picture: Some(Picture::Pads(
                    crate::pad_look::example(crate::akai::kit_colour(0), &self.pad_look()).to_vec(),
                )),
                ..choice("The colour a pad with a sound rests in")
            },
            Item::SystemStatus => RowView {
                name: name.to_string(),
                // The screen fills in the status board's words.
                control: Some(Control::Status {
                    title: String::new(),
                    line: String::new(),
                    tone: mpfree_ui::Tone::Good,
                    button: "System status ›".to_string(),
                }),
                ..RowView::default()
            },
            Item::SystemInfo => RowView {
                name: "All the details".to_string(),
                ..button("Host name, sound cards and the rest", "Open")
            },
            Item::SetupBackup => {
                let asked = self.setup.as_ref().is_some_and(|st| st.backup.next_start);
                RowView {
                    faded: self.setup.is_none(),
                    ..button(
                        &self.setup_backup_text(),
                        if asked {
                            "Take it back"
                        } else {
                            "At next start"
                        },
                    )
                }
            }
            Item::SetupAkaiKits => {
                let st = self.setup.as_ref();
                let asked = st.is_some_and(|st| st.akai.next_start);
                let done = st.is_some_and(|st| st.akai.done);
                RowView {
                    faded: st.is_none() || done,
                    ..button(
                        &self.setup_akai_text(),
                        if asked {
                            "Take it back"
                        } else {
                            "Bring them in"
                        },
                    )
                }
            }
            Item::FindButtons => button("The button guide asks for nine buttons", "Start"),
            Item::MapControls => button("Every button's number; learn one again", "Open"),
            Item::DebugMode => switch("Adds the Dev tab: MIDI log, load, Console", s.debug_mode),
            _ => RowView {
                name: name.to_string(),
                ..RowView::default()
            },
        }
    }

    /// The Themes tab's gallery.
    fn prefs_gallery(&self) -> Gallery {
        let tiles: Vec<ThemeTile> = self
            .theme_choices()
            .map(|(key, label)| ThemeTile {
                key: key.to_string(),
                label: label.to_string(),
                problems: self
                    .theme_problems
                    .iter()
                    .find(|(f, _)| f == key)
                    .map_or(0, |(_, p)| p.len()),
            })
            .collect();
        let at = tiles
            .iter()
            .position(|t| t.key == self.settings.theme)
            .unwrap_or(0);
        let trial = self
            .theme_trial_help()
            .map(|(heading, lines, _)| (heading, lines.into_iter().next().unwrap_or_default()));
        let file = !mpfree_ui::theme::BUILTIN.contains(&self.settings.theme.as_str());
        let mut about = vec![if file {
            format!("Your theme file {}", self.settings.theme)
        } else {
            "Built in. A tap on another theme tries it for 15 seconds.".to_string()
        }];
        about.extend(
            self.theme_problem_lines()
                .into_iter()
                .filter(|l| !l.is_empty()),
        );
        Gallery {
            customise: trial.is_none(),
            tiles,
            at,
            trial,
            about,
            new_theme: true,
        }
    }

    /// Prefs as the screen draws it: the tabs, the tab's cards and rows,
    /// the row the wheel is on.
    pub(super) fn prefs_view(&self) -> PrefsView {
        let tab = self.prefs_tab;
        let rows = tab.rows();
        let row_of = |item: &Item| rows.iter().position(|r| r == item).unwrap_or(0);
        let cards = if tab == PrefsTab::Themes {
            Vec::new()
        } else {
            tab.cards()
                .iter()
                .map(|c| CardView {
                    title: c.title.to_string(),
                    note: if tab == PrefsTab::Customise {
                        self.custom_note(c.note)
                    } else {
                        c.note.to_string()
                    },
                    hue: c.hue,
                    column: c.column,
                    fill: c.fill,
                    rows: c.rows.iter().map(row_of).collect(),
                    info: c.info,
                    split: c.split.then(|| self.memory_split()),
                })
                .collect()
        };
        // The sidebar: the five tabs, Appearance's views under it while
        // one of them is open.
        let mut tabs = Vec::new();
        for (icon, t) in PrefsTab::SIDEBAR.into_iter().enumerate() {
            tabs.push(SideTab {
                name: t.name().to_string(),
                line: t.line().to_string(),
                sub: false,
                tab: t.index(),
                lit: t == tab || (t.appearance() && tab.appearance()),
                icon,
            });
            if t.appearance() && tab.appearance() {
                for v in [PrefsTab::Themes, PrefsTab::Customise, PrefsTab::Lights] {
                    tabs.push(SideTab {
                        name: v.view_name().unwrap_or_default().to_string(),
                        sub: true,
                        tab: v.index(),
                        lit: v == tab,
                        ..SideTab::default()
                    });
                }
            }
        }
        PrefsView {
            tabs,
            tab: tab.index(),
            cards,
            rows: rows.iter().map(|&i| self.prefs_row(i)).collect(),
            selected: self.selected.min(rows.len() - 1),
            editing: self.menu_editing,
            gallery: (tab == PrefsTab::Themes).then(|| self.prefs_gallery()),
            hint: (tab == PrefsTab::Customise)
                .then(|| self.custom_hint())
                .flatten(),
            reset: tab.resets().then(|| {
                if self.reset_armed {
                    "Tap again to reset".to_string()
                } else {
                    format!("Reset {}", tab.name())
                }
            }),
        }
    }

    /// Prefs' Reset: the first tap asks for a second, which puts the
    /// tab's settings back to how they came (as a new stick has them).
    pub fn prefs_reset(&mut self) -> Vec<Effect> {
        if self.view != View::Prefs || !self.prefs_tab.resets() {
            return Vec::new();
        }
        if !self.reset_armed {
            self.reset_armed = true;
            return vec![self.menu_view()];
        }
        self.reset_armed = false;
        let tab = self.prefs_tab;
        let mut fx = self.reset_tab(tab);
        fx.push(Effect::Note(format!(
            "prefs: {} set back to how they came",
            tab.name()
        )));
        self.save_dirty = true;
        fx.push(Effect::Settings(self.settings.clone()));
        fx.push(Effect::Tell(format!("{} reset", tab.name())));
        fx.push(self.menu_view());
        fx
    }

    /// `tab`'s settings back to the defaults, with what a change of each
    /// tells the rest of the app.
    fn reset_tab(&mut self, tab: PrefsTab) -> Vec<Effect> {
        let d = Settings::default();
        let mut fx = Vec::new();
        // A setting that tells the rest of the app steps there as a turn
        // would, so the same effects go out.
        let toward = |now: bool, want: bool| if want && !now { 1 } else { -1 };
        match tab {
            PrefsTab::Playing => {
                let s = &mut self.settings;
                s.curve = d.curve;
                s.curve_bend = d.curve_bend;
                s.pad_threshold = d.pad_threshold;
                s.screen_velocity = d.screen_velocity;
                s.qlink_speed = d.qlink_speed;
                s.wheel_speed = d.wheel_speed;
                s.wheel_speed_up = d.wheel_speed_up;
                s.fine_shift = d.fine_shift;
                if s.latch_mode != d.latch_mode {
                    s.latch_mode = d.latch_mode;
                    self.update_engine();
                }
                if self.settings.pedal_latches != d.pedal_latches {
                    let dir = toward(self.settings.pedal_latches, d.pedal_latches);
                    fx.extend(self.apply_item(Item::PedalLatch, dir));
                }
            }
            PrefsTab::Sound => {
                while self.settings.periods != d.periods {
                    let dir = if d.periods > self.settings.periods {
                        1
                    } else {
                        -1
                    };
                    let was = self.settings.periods;
                    fx.extend(self.apply_item(Item::Buffer, dir));
                    if self.settings.periods == was {
                        break;
                    }
                }
                if self.settings.sends_midi != d.sends_midi {
                    let dir = toward(self.settings.sends_midi, d.sends_midi);
                    fx.extend(self.apply_item(Item::SendsMidi, dir));
                }
                if self.settings.line_input {
                    fx.extend(self.apply_item(Item::LineInput, -1));
                }
                if self.settings.metronome_sound != d.metronome_sound {
                    self.settings.metronome_sound = d.metronome_sound;
                    fx.push(Effect::ClickSound(d.metronome_sound));
                }
                let s = &mut self.settings;
                s.audition_db = d.audition_db;
                s.sound_cores = d.sound_cores;
                s.loop_memory = d.loop_memory;
                s.audio_memory = d.audio_memory;
                s.loop_latency_set = false;
                s.loop_latency = crate::looping::estimated_latency(s.periods);
                fx.push(Effect::Looper(LoopAction::Command(
                    LoopCommand::SetLatency(s.loop_latency_now()),
                )));
            }
            PrefsTab::Projects => {
                let s = &mut self.settings;
                s.export_mode = d.export_mode;
                s.export_bits = d.export_bits;
                s.export_rate = d.export_rate;
                s.open_last = d.open_last;
                s.quick_advance = d.quick_advance;
                s.stems_by = d.stems_by;
            }
            _ => {}
        }
        fx
    }

    /// What the two memories set aside together.
    fn memory_split(&self) -> Split {
        let s = &self.settings;
        let loops = s.loop_memory.megabytes(self.sample_rate);
        let takes = s.audio_memory.megabytes(self.sample_rate);
        let total = (loops + takes).max(1) as f32;
        Split {
            title: "Set aside together".to_string(),
            total: format!("{} MB", loops + takes),
            parts: vec![
                (format!("Loops {loops} MB"), loops as f32 / total),
                (format!("Audio tracks {takes} MB"), takes as f32 / total),
            ],
        }
    }
}

/// The audio buffer's delay at `periods` of 128 frames.
fn ms(periods: u32) -> String {
    format!("{:.1} ms", periods as f64 * 128.0 / 44.1)
}

/// Sound cores' short names, for its buttons.
fn cores_label(c: SoundCores) -> &'static str {
    match c {
        SoundCores::Shared => "Shared",
        SoundCores::Core3 => "Core 3",
        SoundCores::Two => "Cores 3, 2",
        SoundCores::Three => "Cores 3, 2, 1",
    }
}

/// A step of Audio track memory: "0:30", "1", "2"...
fn memory_label(seconds: u32) -> String {
    if seconds.is_multiple_of(60) {
        (seconds / 60).to_string()
    } else {
        format!("{}:{:02}", seconds / 60, seconds % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_tabs_rows_are_its_cards_left_column_first() {
        for tab in PrefsTab::ALL {
            let mut order: Vec<Item> = Vec::new();
            for col in [2, 0, 1] {
                for c in tab.cards().iter().filter(|c| c.column == col) {
                    order.extend(c.rows);
                }
            }
            assert_eq!(order, tab.rows(), "{tab:?}");
        }
    }

    #[test]
    fn every_setting_prefs_had_has_a_tab() {
        for item in [
            Item::Curve,
            Item::LatchMode,
            Item::PedalLatch,
            Item::QuickAdvance,
            Item::Buffer,
            Item::FindButtons,
            Item::DebugMode,
            Item::SoundCores,
            Item::LoopMemory,
            Item::AudioMemory,
            Item::MapControls,
            Item::LineInput,
            Item::LoopLatency,
            Item::SendsMidi,
            Item::SystemInfo,
            Item::SystemStatus,
            Item::Appearance,
            Item::AtStart,
            Item::Export,
            Item::ExportFile,
            Item::ExportRate,
            Item::SetupBackup,
            Item::SetupAkaiKits,
            Item::ScreenVelocity,
            Item::CalibrateTouch,
            Item::TouchLab,
        ] {
            assert!(PrefsTab::of(item).is_some(), "{item:?}");
        }
    }

    /// The custom curve's knobs choose Custom, a hit on the MPC is marked
    /// on the curve, the Controls card sets how turns feel, and Reset
    /// asks once before it puts the tab back as it came.
    #[test]
    fn playing_sets_the_custom_curve_and_reset_takes_two_taps() {
        use crate::play_feel::{Bend, Speed, SpeedUp};
        let mut app = App::new(44_100, false);
        app.open_tab(View::Prefs);
        assert_eq!(app.prefs_tab, PrefsTab::Playing);
        let shape = app.prefs_go_to(Item::CurveShape).expect("a row");
        app.step_item(shape, 1);
        assert_eq!(
            app.settings.curve,
            Curve::Custom(Bend {
                shape: 10,
                ..Bend::default()
            })
        );
        let softest = app.prefs_go_to(Item::CurveSoftest).expect("a row");
        app.set_value(softest, 500);
        assert_eq!(app.settings.curve_bend.softest, 64);
        // Another curve, then Custom again: the knobs are kept.
        let curve = app.prefs_go_to(Item::Curve).expect("a row");
        app.choose(curve, 0);
        assert_eq!(app.settings.curve, Curve::Soft);
        app.choose(curve, 3);
        assert_eq!(app.settings.curve, Curve::Custom(app.settings.curve_bend));
        assert_eq!(app.settings.curve_bend.softest, 64);

        let fx = app.pad_hit(30);
        let view = |fx: &[Effect]| match fx.last() {
            Some(Effect::Menu(m)) => m.prefs.clone().expect("Prefs"),
            _ => panic!("no menu: {fx:?}"),
        };
        match &view(&fx).rows[curve].picture {
            Some(Picture::Curve(_, hit)) => {
                assert_eq!(*hit, Some((30, app.settings.curve.apply(30))))
            }
            p => panic!("{p:?}"),
        }

        let threshold = app.prefs_go_to(Item::PadThreshold).expect("a row");
        app.set_value(threshold, 1000);
        let wheel = app.prefs_go_to(Item::WheelSpeed).expect("a row");
        app.set_value(wheel, 1000);
        let up = app.prefs_go_to(Item::WheelSpeedUp).expect("a row");
        app.step_item(up, 1);
        let fine = app.prefs_go_to(Item::FineShift).expect("a row");
        app.step_item(fine, -1);
        let feel = app.feel();
        assert_eq!(
            (
                feel.threshold,
                feel.wheel,
                feel.speed_up,
                app.settings.fine_shift
            ),
            (40, Speed::Fast, SpeedUp::Some, false)
        );

        let fx = app.prefs_reset();
        assert_eq!(view(&fx).reset.as_deref(), Some("Tap again to reset"));
        assert_eq!(app.settings.pad_threshold, 40, "the first tap only asks");
        let fx = app.prefs_reset();
        assert_eq!(view(&fx).reset.as_deref(), Some("Reset Playing"));
        let d = Settings::default();
        let s = &app.settings;
        assert_eq!(
            (
                s.curve,
                s.curve_bend,
                s.pad_threshold,
                s.wheel_speed,
                s.fine_shift
            ),
            (
                d.curve,
                d.curve_bend,
                d.pad_threshold,
                d.wheel_speed,
                d.fine_shift
            )
        );
        // A change in between: Reset asks again.
        app.prefs_reset();
        app.step_item(fine, -1);
        app.prefs_reset();
        assert!(!app.settings.fine_shift);
        // Themes has no Reset.
        app.pick_prefs_tab(PrefsTab::Themes.index());
        assert!(app.prefs_reset().is_empty());
    }

    #[test]
    fn sound_sets_the_metronome_and_audition_and_projects_the_stems() {
        use crate::time::ClickSound;
        use mpfree_browser_ui::StemsBy;
        let mut app = App::new(44_100, false);
        app.open_tab(View::Prefs);
        let click = app.prefs_go_to(Item::MetronomeSound).expect("a row");
        assert_eq!(app.prefs_tab, PrefsTab::Sound);
        let fx = app.choose(click, 1);
        assert!(
            fx.iter()
                .any(|e| matches!(e, Effect::ClickSound(ClickSound::Wood))),
            "{fx:?}"
        );
        let level = app.prefs_go_to(Item::AuditionLevel).expect("a row");
        app.step_item(level, 1);
        assert_eq!(app.settings.audition_db, -5);
        app.set_value(level, 0);
        assert_eq!(app.settings.audition_db, -40);
        // The meter reads the line input; with it off it says so.
        let meter = app.prefs_go_to(Item::InputMeter).expect("a row");
        let fx = app.prefs_reset();
        let view = |fx: &[Effect]| match fx.last() {
            Some(Effect::Menu(m)) => m.prefs.clone().expect("Prefs"),
            _ => panic!("no menu: {fx:?}"),
        };
        assert!(view(&fx).rows[meter].faded);
        let fx = app.prefs_reset();
        assert!(fx
            .iter()
            .any(|e| matches!(e, Effect::ClickSound(ClickSound::Click))));
        assert_eq!(
            (app.settings.audition_db, app.settings.metronome_sound),
            (-6, ClickSound::Click)
        );

        let stems = app.prefs_go_to(Item::StemsBy).expect("a row");
        assert_eq!(app.prefs_tab, PrefsTab::Projects);
        app.choose(stems, 0);
        assert_eq!(app.settings.stems_by, StemsBy::Track);
        app.prefs_reset();
        app.prefs_reset();
        assert_eq!(app.settings.stems_by, StemsBy::Channel);
    }

    /// Screen and lights' rows change the settings, and the pads' look
    /// follows them: Track rests in the armed track's colour in the
    /// theme in use, Accent in its accent.
    #[test]
    fn screen_and_lights_rows_set_how_the_pads_light() {
        use crate::pad_look::{rest_level, EmptyPads, HeldPad, PadsShow};
        let mut app = App::new(44_100, false);
        app.open_tab(View::Prefs);
        let bright = app.prefs_go_to(Item::PadBrightness).expect("a row");
        assert_eq!(app.prefs_tab, PrefsTab::Lights);
        app.set_value(bright, 0);
        assert_eq!(app.settings.pad_brightness, 10);
        app.set_value(bright, 1000);
        assert_eq!(app.settings.pad_brightness, 100);
        app.step_item(bright, -1);
        assert_eq!(app.pad_look().brightness, 90);

        let held = app.prefs_go_to(Item::HeldPad).expect("a row");
        app.choose(held, 1);
        assert_eq!(app.pad_look().held, HeldPad::White);
        let empty = app.prefs_go_to(Item::EmptyPads).expect("a row");
        app.choose(empty, 1);
        assert_eq!(app.pad_look().empty, EmptyPads::Dim);
        let hint = app.prefs_go_to(Item::HintLine).expect("a row");
        app.step_item(hint, -1);
        assert!(!app.settings.hint_line);

        let show = app.prefs_go_to(Item::PadsShow).expect("a row");
        let theme = app.theme_in_use();
        app.choose(show, 1);
        let look = app.pad_look();
        assert_eq!(look.show, PadsShow::Track);
        let track = app.track() % theme.track.len();
        assert_eq!(look.tint, rest_level(theme.track[track]));
        app.choose(show, 2);
        assert_eq!(app.pad_look().tint, rest_level(theme.accent));
        app.choose(show, 3);
        assert_eq!(app.pad_look().show, PadsShow::Heat);
    }
}
