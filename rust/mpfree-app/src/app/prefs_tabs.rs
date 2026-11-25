//! Prefs' tabs (layout spec, section 5): the sidebar's five tabs, which
//! settings each shows in which card, and how each is drawn and changed
//! ([`crate::prefs_page`] draws them). The redesign canvas, round 2, Jay's
//! go 24 Nov 2026 17:35Z.

use super::*;
use crate::prefs_page::{
    CardView, Control, Gallery, Hue, Picture, PrefsView, RowView, Split, ThemeTile,
};

/// The sidebar's tabs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PrefsTab {
    #[default]
    Playing,
    Sound,
    /// Appearance: the themes gallery.
    Themes,
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
        ..card("Pads", "how hard you hit", Hue::Cyan, 0, &[Item::Curve])
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
        "its pads and its checks",
        Hue::Pink,
        1,
        &[Item::ScreenVelocity, Item::CalibrateTouch, Item::TouchLab],
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
    card(
        "Line input",
        "the INPUTS on the back",
        Hue::Coral,
        0,
        &[Item::LineInput, Item::LoopLatency],
    ),
    CardPlan {
        split: true,
        ..card(
            "Memory",
            "set aside when MPFree starts",
            Hue::Lime,
            1,
            &[Item::LoopMemory, Item::AudioMemory],
        )
    },
    card("MIDI and CV out", "", Hue::Pink, 1, &[Item::SendsMidi]),
];

const THEMES: &[CardPlan] = &[card("Themes", "", Hue::Grey, 0, &[Item::Appearance])];

const PROJECTS: &[CardPlan] = &[
    CardPlan {
        fill: true,
        ..card(
            "Export",
            "the Export panel starts from these",
            Hue::Coral,
            0,
            &[Item::Export, Item::ExportFile, Item::ExportRate],
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
        "Buttons",
        "",
        Hue::Lime,
        1,
        &[Item::FindButtons, Item::MapControls],
    ),
    card("Developers", "", Hue::Coral, 1, &[Item::DebugMode]),
];

/// Each tab's settings in the wheel's order: the left column's cards top
/// to bottom, then the right's (a test keeps these in step with the
/// cards).
const PLAYING_ROWS: &[Item] = &[
    Item::Curve,
    Item::LatchMode,
    Item::PedalLatch,
    Item::ScreenVelocity,
    Item::CalibrateTouch,
    Item::TouchLab,
];
const SOUND_ROWS: &[Item] = &[
    Item::Buffer,
    Item::SoundCores,
    Item::LineInput,
    Item::LoopLatency,
    Item::LoopMemory,
    Item::AudioMemory,
    Item::SendsMidi,
];
const THEMES_ROWS: &[Item] = &[Item::Appearance];
const PROJECTS_ROWS: &[Item] = &[
    Item::Export,
    Item::ExportFile,
    Item::ExportRate,
    Item::AtStart,
    Item::QuickAdvance,
];
const SYSTEM_ROWS: &[Item] = &[
    Item::SystemStatus,
    Item::SystemInfo,
    Item::SetupBackup,
    Item::SetupAkaiKits,
    Item::FindButtons,
    Item::MapControls,
    Item::DebugMode,
];

/// The most frames the latency knob turns to (46 ms at 44.1 kHz); - and +
/// go on to 4096.
const LATENCY_KNOB_FRAMES: f32 = 2048.0;

impl PrefsTab {
    pub const ALL: [PrefsTab; 5] = [
        PrefsTab::Playing,
        PrefsTab::Sound,
        PrefsTab::Themes,
        PrefsTab::Projects,
        PrefsTab::System,
    ];

    pub fn name(self) -> &'static str {
        match self {
            PrefsTab::Playing => "Playing",
            PrefsTab::Sound => "Sound",
            PrefsTab::Themes => "Appearance",
            PrefsTab::Projects => "Projects",
            PrefsTab::System => "System",
        }
    }

    /// The line under its name in the sidebar.
    pub fn line(self) -> &'static str {
        match self {
            PrefsTab::Playing => "Pads, latch, touch",
            PrefsTab::Sound => "Buffer, memory, input",
            PrefsTab::Themes => "Themes",
            PrefsTab::Projects => "Start, export, sets",
            PrefsTab::System => "Info, setup, checks",
        }
    }

    fn cards(self) -> &'static [CardPlan] {
        match self {
            PrefsTab::Playing => PLAYING,
            PrefsTab::Sound => SOUND,
            PrefsTab::Themes => THEMES,
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
            PrefsTab::Projects => PROJECTS_ROWS,
            PrefsTab::System => SYSTEM_ROWS,
        }
    }

    /// The tab that shows `item`, if Prefs shows it.
    pub fn of(item: Item) -> Option<PrefsTab> {
        PrefsTab::ALL.into_iter().find(|t| t.rows().contains(&item))
    }

    fn index(self) -> usize {
        PrefsTab::ALL.iter().position(|&t| t == self).unwrap_or(0)
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
        }
        self.prefs_tab = tab;
        self.selected = 0;
        self.menu_editing = false;
        fx.push(self.menu_view());
        fx
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
        if item == Item::Appearance {
            return self.pick_theme(k);
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
                    ("Soft", "loud comes easily"),
                    ("Linear", "the pad's own"),
                    ("Hard", "more dynamics"),
                ]),
                Curve::ALL.iter().position(|&c| c == s.curve).unwrap_or(1),
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
                )),
                ..choice("How hard a pad must be hit for a loud note")
            },
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
            Item::AtStart => tiles(),
            Item::QuickAdvance => tiles(),
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
            tiles,
            at,
            trial,
            about,
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
                    note: c.note.to_string(),
                    hue: c.hue,
                    column: c.column,
                    fill: c.fill,
                    rows: c.rows.iter().map(row_of).collect(),
                    info: c.info,
                    split: c.split.then(|| self.memory_split()),
                })
                .collect()
        };
        PrefsView {
            tabs: PrefsTab::ALL
                .iter()
                .map(|t| (t.name().to_string(), t.line().to_string()))
                .collect(),
            tab: tab.index(),
            cards,
            rows: rows.iter().map(|&i| self.prefs_row(i)).collect(),
            selected: self.selected.min(rows.len() - 1),
            editing: self.menu_editing,
            gallery: (tab == PrefsTab::Themes).then(|| self.prefs_gallery()),
        }
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
            for col in 0..2 {
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
}
