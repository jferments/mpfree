//! What the pad player's controls do: the buttons, the wheel, the screen's
//! tabs and menus, the button guide, Note Repeat and Latch.
//!
//! Pure logic, no I/O: the audio thread hands it the controller's events and
//! the screen's actions, and carries out the `Effect`s it returns (lights,
//! pages, kit changes). Pads go through the arpeggiator thread's engine
//! (`mpfree-arp`), which plays them directly or repeats them (Note Repeat and
//! Latch) on their exact frames. Pads are fed as slots 0..15 (pad 1 = 0), as
//! that engine expects.
//!
//! Buttons: SHIFT 49, FULL LEVEL 39, STOP 81 and PLAY 82 are measured
//! (docs/hardware/controller-protocol.md). TAP TEMPO is taken to be 53, the
//! light that blinks at the tempo, until the guide confirms it. MENU, NOTE
//! REPEAT, - and + are unknown, so the guide asks for them by name. That
//! Half Level is SHIFT + FULL LEVEL and Latch is SHIFT + NOTE REPEAT is read
//! from the panel's print, not measured.

use mpfree_arp::{Engine, Events, LatchMode, Mode, Rate};
use mpfree_controller::mpc_one::button;

/// Button lights take the button's note as the CC number on the Private
/// cable. Akai's app sends 1 and 3 (dim and bright, inferred); so do we.
pub const LIGHT_OFF: u8 = 1;
pub const LIGHT_ON: u8 = 3;

/// The pages, in the order of the header's tabs, then the ones without a tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Play,
    Debug,
    Console,
    Prefs,
    /// The current page's options (MENU).
    Options,
    /// The button guide.
    Guide,
}

pub const TABS: [&str; 4] = ["Play", "Debug", "Console", "Prefs"];

impl View {
    pub fn from_tab(i: usize) -> View {
        [View::Play, View::Debug, View::Console, View::Prefs].get(i).copied().unwrap_or(View::Play)
    }

    pub fn tab(self) -> Option<usize> {
        match self {
            View::Play => Some(0),
            View::Debug => Some(1),
            View::Console => Some(2),
            View::Prefs => Some(3),
            View::Options | View::Guide => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Curve {
    Soft,
    Linear,
    Hard,
}

impl Curve {
    const ALL: [Curve; 3] = [Curve::Soft, Curve::Linear, Curve::Hard];

    fn label(self) -> &'static str {
        match self {
            Curve::Soft => "Soft",
            Curve::Linear => "Linear",
            Curve::Hard => "Hard",
        }
    }

    /// The velocity a hit plays at. Soft makes loud hits easier, Hard makes
    /// them harder, so more of the pad's travel stays below 127.
    pub fn apply(self, v: u8) -> u8 {
        let x = v.min(127) as f32 / 127.0;
        let y = match self {
            Curve::Soft => x.powf(0.6),
            Curve::Linear => x,
            Curve::Hard => x.powf(1.7),
        };
        ((y * 127.0).round() as u8).clamp(1, 127)
    }
}

/// FULL LEVEL (every hit 127) and Half Level (every hit 64).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PadLevel {
    Off,
    Full,
    Half,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuideStep {
    Menu,
    NoteRepeat,
    TapTempo,
    Minus,
    Plus,
}

impl GuideStep {
    const ALL: [GuideStep; 5] =
        [GuideStep::Menu, GuideStep::NoteRepeat, GuideStep::TapTempo, GuideStep::Minus, GuideStep::Plus];

    pub fn label(self) -> &'static str {
        match self {
            GuideStep::Menu => "MENU",
            GuideStep::NoteRepeat => "NOTE REPEAT",
            GuideStep::TapTempo => "TAP TEMPO",
            GuideStep::Minus => "- (next to the wheel)",
            GuideStep::Plus => "+ (next to the wheel)",
        }
    }

    fn short(self) -> &'static str {
        match self {
            GuideStep::Minus => "-",
            GuideStep::Plus => "+",
            other => other.label(),
        }
    }
}

/// Buttons whose numbers came from the guide (or the TAP TEMPO guess).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ButtonMap {
    pub menu: Option<u8>,
    pub note_repeat: Option<u8>,
    pub tap_tempo: Option<u8>,
    pub minus: Option<u8>,
    pub plus: Option<u8>,
}

impl ButtonMap {
    pub fn guessed() -> ButtonMap {
        ButtonMap { tap_tempo: Some(button::TAP_TEMPO), ..ButtonMap::default() }
    }

    fn get(&self, s: GuideStep) -> Option<u8> {
        match s {
            GuideStep::Menu => self.menu,
            GuideStep::NoteRepeat => self.note_repeat,
            GuideStep::TapTempo => self.tap_tempo,
            GuideStep::Minus => self.minus,
            GuideStep::Plus => self.plus,
        }
    }

    fn set(&mut self, s: GuideStep, note: u8) {
        let slot = match s {
            GuideStep::Menu => &mut self.menu,
            GuideStep::NoteRepeat => &mut self.note_repeat,
            GuideStep::TapTempo => &mut self.tap_tempo,
            GuideStep::Minus => &mut self.minus,
            GuideStep::Plus => &mut self.plus,
        };
        *slot = Some(note);
    }

    /// What a note is: a measured button or one the guide found.
    pub fn name(&self, note: u8) -> Option<&'static str> {
        match note {
            button::SHIFT => return Some("SHIFT"),
            button::FULL_LEVEL => return Some("FULL LEVEL"),
            button::STOP => return Some("STOP"),
            button::PLAY => return Some("PLAY"),
            _ => {}
        }
        GuideStep::ALL.iter().find(|&&s| self.get(s) == Some(note)).map(|s| s.short())
    }

    /// One line for the log: every button the guide asked for.
    pub fn describe(&self) -> String {
        GuideStep::ALL.iter().map(|&s| match self.get(s) {
            Some(n) => format!("{} = button {n}", s.short()),
            None => format!("{} = not found", s.short()),
        }).collect::<Vec<_>>().join(", ")
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub curve: Curve,
    /// Velocity of pads played on the screen (touch has no pressure).
    pub screen_velocity: u8,
    pub rate: Rate,
    pub bpm: f64,
    /// How Latch keeps pads: Replace (a fresh press starts over) or Toggle.
    pub latch_mode: LatchMode,
    /// Audio buffer in periods of 128 frames: 3 as Akai's app, or 2.
    pub periods: u32,
    pub show_pressure: bool,
    pub midi_paused: bool,
    pub console_paused: bool,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings { curve: Curve::Linear, screen_velocity: 100, rate: Rate::Sixteenth, bpm: 120.0,
                   latch_mode: LatchMode::Replace, periods: 3, show_pressure: false, midi_paused: false,
                   console_paused: false }
    }
}

/// One row of a menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    Curve,
    ScreenVelocity,
    LatchMode,
    Buffer,
    FindButtons,
    TouchLab,
    Rate,
    Tempo,
    PadLevel,
    Latch,
    ShowPressure,
    MidiLog,
    ClearLogs,
    ConsoleFollow,
    ClearConsole,
}

/// The menu on screen: its rows (name, value), the selected row and help.
#[derive(Clone, Debug, PartialEq)]
pub struct MenuView {
    pub title: String,
    pub rows: Vec<(String, String)>,
    pub selected: usize,
    pub help_heading: String,
    pub help: Vec<String>,
}

/// What the guide card shows.
#[derive(Clone, Debug, PartialEq)]
pub struct GuideView {
    pub heading: String,
    pub lines: Vec<String>,
    pub found: Vec<(String, String)>,
}

/// The modes shown on the screen.
#[derive(Clone, Debug, PartialEq)]
pub struct Status {
    pub shift: bool,
    pub pad_level: PadLevel,
    pub repeat: bool,
    pub latched: bool,
    pub rate: &'static str,
    pub bpm: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    Show(View),
    Menu(MenuView),
    Guide(GuideView),
    Status(Status),
    Settings(Settings),
    /// A button light: the button's note and LIGHT_ON or LIGHT_OFF.
    Light(u8, u8),
    /// Stop every sound now.
    StopSounds,
    /// Move the kit by this many places.
    Kit(i32),
    /// Start the audio again with this many periods.
    Audio(u32),
    ClearLogs,
    ClearConsole,
    /// A line for the log.
    Note(String),
    /// STOP in the timed `play` step: end it.
    Quit,
    /// Open the Touch lab page (it starts at its first step).
    TouchLab,
}

pub struct App {
    pub settings: Settings,
    pub map: ButtonMap,
    pub engine: Engine,
    /// The `play` step: STOP ends it. The app: STOP stops the sounds.
    timed: bool,
    shift: bool,
    repeat_held: bool,
    latched: bool,
    pad_level: PadLevel,
    view: View,
    /// The page to go back to from Console, Options or the guide.
    back: View,
    selected: usize,
    guide: usize,
    guide_note: String,
    taps: Vec<f64>,
    /// When the tempo's beats started (seconds), for the TAP TEMPO light.
    beat_zero: f64,
    tap_light: bool,
}

impl App {
    pub fn new(sample_rate: u32, timed: bool) -> App {
        let settings = Settings::default();
        let engine = Engine::new(sample_rate, settings.bpm);
        let mut app = App { settings, map: ButtonMap::guessed(), engine, timed, shift: false, repeat_held: false,
                            latched: false, pad_level: PadLevel::Off, view: View::Play, back: View::Play,
                            selected: 0, guide: 0, guide_note: String::new(), taps: Vec::new(), beat_zero: 0.0,
                            tap_light: false };
        app.update_engine();
        app
    }

    #[cfg(test)]
    pub fn view(&self) -> View {
        self.view
    }

    /// The first page once everything is ready, and the lights to start with.
    pub fn start(&mut self) -> Vec<Effect> {
        let mut fx = vec![self.status(), Effect::Settings(self.settings.clone()),
                          Effect::Light(button::FULL_LEVEL, LIGHT_OFF)];
        if self.map.menu.is_none() || self.map.note_repeat.is_none() {
            self.guide = 0;
            self.guide_note.clear();
            self.back = View::Play;
            self.view = View::Guide;
            fx.push(self.guide_view());
        }
        fx.push(Effect::Show(self.view));
        fx
    }

    /// The velocity a hit plays at, after Full or Half Level and the curve.
    pub fn velocity(&self, v: u8) -> u8 {
        match self.pad_level {
            PadLevel::Full => 127,
            PadLevel::Half => 64,
            PadLevel::Off => self.settings.curve.apply(v),
        }
    }

    // ------------------------------------------------------------ pads

    pub fn pad_down(&mut self, slot: usize, velocity: u8) {
        self.engine.press(slot as u8, velocity.max(1));
    }

    pub fn pad_up(&mut self, slot: usize) {
        self.engine.release(slot as u8);
    }

    pub fn pressure(&mut self, slot: usize, value: u8) {
        self.engine.pressure(slot as u8, value);
    }

    /// One audio period: the notes that start (or stop) in it.
    pub fn process(&mut self, frames: u32, out: &mut Events) {
        self.engine.process(frames, out);
    }

    /// Pads the engine keeps playing (held, or latched).
    pub fn active(&self, slot: usize) -> bool {
        self.engine.active().contains(slot as u8)
    }

    pub fn repeating(&self) -> bool {
        self.repeat_held || self.latched
    }

    fn update_engine(&mut self) {
        let mut s = self.engine.settings();
        s.mode = if self.repeating() { Mode::Repeat } else { Mode::Direct };
        s.rate = self.settings.rate;
        self.engine.set_settings(s);
        let latch = if self.latched { self.settings.latch_mode } else { LatchMode::Off };
        if self.engine.latch() != latch {
            self.engine.set_latch(latch);
        }
        self.engine.set_tempo(self.settings.bpm);
    }

    fn status(&self) -> Effect {
        Effect::Status(Status { shift: self.shift, pad_level: self.pad_level, repeat: self.repeating(),
                                latched: self.latched, rate: self.settings.rate.label(), bpm: self.settings.bpm })
    }

    fn repeat_light(&self) -> Option<Effect> {
        self.map.note_repeat.map(|n| Effect::Light(n, if self.repeating() { LIGHT_ON } else { LIGHT_OFF }))
    }

    // ------------------------------------------------------------ buttons

    /// A button pressed or released. `t` is seconds since the start (for
    /// TAP TEMPO).
    pub fn button(&mut self, note: u8, pressed: bool, t: f64) -> Vec<Effect> {
        let mut fx = Vec::new();
        if note == button::SHIFT {
            self.shift = pressed;
            fx.push(self.status());
            if self.view == View::Guide && pressed {
                fx.extend(self.guide_press(note));
            }
            return fx;
        }
        if !pressed {
            if Some(note) == self.map.note_repeat && self.repeat_held {
                self.repeat_held = false;
                self.update_engine();
                fx.push(self.status());
                fx.extend(self.repeat_light());
            }
            return fx;
        }
        if self.view == View::Guide {
            return self.guide_press(note);
        }
        let shift = self.shift;
        if note == button::FULL_LEVEL {
            let want = if shift { PadLevel::Half } else { PadLevel::Full };
            self.pad_level = if self.pad_level == want { PadLevel::Off } else { want };
            let on = self.pad_level != PadLevel::Off;
            fx.push(Effect::Light(button::FULL_LEVEL, if on { LIGHT_ON } else { LIGHT_OFF }));
            fx.push(self.status());
            fx.push(Effect::Note(match self.pad_level {
                PadLevel::Off => "pad level: as played".to_string(),
                PadLevel::Full => "pad level: Full Level (every hit 127)".to_string(),
                PadLevel::Half => "pad level: Half Level (every hit 64)".to_string(),
            }));
        } else if note == button::STOP {
            if shift {
                fx.extend(self.toggle(View::Console));
            } else if self.timed {
                fx.push(Effect::Quit);
            } else {
                fx.extend(self.stop());
            }
        } else if Some(note) == self.map.note_repeat {
            if shift {
                self.latched = !self.latched;
                fx.push(Effect::Note(if self.latched { "Latch on" } else { "Latch off" }.to_string()));
            } else if self.latched {
                self.latched = false;
                fx.push(Effect::Note("Latch off".to_string()));
            } else {
                self.repeat_held = true;
            }
            self.update_engine();
            fx.push(self.status());
            fx.extend(self.repeat_light());
        } else if Some(note) == self.map.menu {
            let target = if shift { View::Prefs } else { View::Options };
            fx.extend(self.toggle(target));
        } else if Some(note) == self.map.tap_tempo {
            fx.extend(self.tap(t));
        } else if Some(note) == self.map.minus || Some(note) == self.map.plus {
            let d = if Some(note) == self.map.plus { 1 } else { -1 };
            if self.in_menu() {
                fx.extend(self.step_item(self.selected, d));
            } else {
                fx.push(Effect::Kit(d));
            }
        }
        fx
    }

    /// STOP in the app: every sound stops, latched pads are let go.
    fn stop(&mut self) -> Vec<Effect> {
        self.latched = false;
        self.engine.panic();
        self.update_engine();
        let mut fx = vec![Effect::StopSounds, Effect::Note("STOP: all sounds stopped".to_string()), self.status()];
        fx.extend(self.repeat_light());
        fx
    }

    fn tap(&mut self, t: f64) -> Vec<Effect> {
        if self.taps.last().is_some_and(|&last| t - last > 2.0) {
            self.taps.clear();
        }
        self.taps.push(t);
        if self.taps.len() > 5 {
            self.taps.remove(0);
        }
        self.beat_zero = t;
        if self.taps.len() < 2 {
            return Vec::new();
        }
        let span = self.taps[self.taps.len() - 1] - self.taps[0];
        let bpm = 60.0 * (self.taps.len() - 1) as f64 / span;
        if !(30.0..=300.0).contains(&bpm) {
            return Vec::new();
        }
        self.settings.bpm = (bpm * 10.0).round() / 10.0;
        self.update_engine();
        let mut fx = vec![self.status(), Effect::Note(format!("tempo {:.1} BPM (tapped)", self.settings.bpm))];
        if self.in_menu() {
            fx.push(self.menu_view());
        }
        fx
    }

    /// The TAP TEMPO light, on for the first fifth of each beat. Returns a
    /// light message only when it changes.
    pub fn tick(&mut self, t: f64) -> Option<Effect> {
        let note = self.map.tap_tempo?;
        let beats = (t - self.beat_zero) * self.settings.bpm / 60.0;
        let on = beats.rem_euclid(1.0) < 0.2;
        if on == self.tap_light {
            return None;
        }
        self.tap_light = on;
        Some(Effect::Light(note, if on { LIGHT_ON } else { LIGHT_OFF }))
    }

    /// The data wheel: values in a menu, kits elsewhere.
    pub fn wheel(&mut self, delta: i32) -> Vec<Effect> {
        match self.view {
            View::Guide => Vec::new(),
            // In a menu the wheel moves between rows and - / + change the
            // value, so menus work without touch; SHIFT + wheel also changes
            // the value, in case - and + are not known yet.
            _ if self.in_menu() && self.shift => self.step_item(self.selected, delta.signum()),
            _ if self.in_menu() => {
                let last = self.items().len() as i32 - 1;
                self.select((self.selected as i32 + delta).clamp(0, last) as usize)
            }
            _ => vec![Effect::Kit(delta)],
        }
    }

    // ------------------------------------------------------------ pages

    fn in_menu(&self) -> bool {
        matches!(self.view, View::Options | View::Prefs)
    }

    fn show(&mut self, v: View) -> Vec<Effect> {
        if matches!(self.view, View::Play | View::Debug | View::Console) {
            self.back = self.view;
        }
        self.view = v;
        let mut fx = Vec::new();
        if self.in_menu() {
            self.selected = 0;
            fx.push(self.menu_view());
        }
        fx.push(Effect::Show(v));
        fx
    }

    /// Open `v`, or go back if it is open already.
    fn toggle(&mut self, v: View) -> Vec<Effect> {
        if self.view == v {
            let back = if self.back == v { View::Play } else { self.back };
            self.view = back;
            vec![Effect::Show(back)]
        } else {
            self.show(v)
        }
    }

    /// A tab touched on the screen.
    pub fn tab(&mut self, i: usize) -> Vec<Effect> {
        if self.view == View::Guide {
            return Vec::new();
        }
        self.show(View::from_tab(i))
    }

    // ------------------------------------------------------------ menus

    fn items(&self) -> &'static [Item] {
        match (self.view, self.back) {
            (View::Prefs, _) => &[Item::Curve, Item::ScreenVelocity, Item::LatchMode, Item::Buffer, Item::FindButtons,
                               Item::TouchLab],
            (_, View::Debug) => &[Item::ShowPressure, Item::MidiLog, Item::ClearLogs],
            (_, View::Console) => &[Item::ConsoleFollow, Item::ClearConsole],
            _ => &[Item::Rate, Item::Tempo, Item::PadLevel, Item::Latch],
        }
    }

    fn item_text(&self, item: Item) -> (&'static str, String) {
        let s = &self.settings;
        match item {
            Item::Curve => ("Pad velocity curve", s.curve.label().to_string()),
            Item::ScreenVelocity => ("Screen pad velocity", s.screen_velocity.to_string()),
            Item::LatchMode => ("Latch keeps pads", match s.latch_mode {
                LatchMode::Toggle => "Toggle".to_string(),
                _ => "Replace".to_string(),
            }),
            Item::Buffer => ("Audio buffer", format!("{} periods, {:.1} ms", s.periods, s.periods as f64 * 128.0 / 44.1)),
            Item::FindButtons => ("Find the buttons again", "Start".to_string()),
            Item::TouchLab => ("Touch lab", "Open".to_string()),
            Item::Rate => ("Note repeat rate", s.rate.label().to_string()),
            Item::Tempo => ("Tempo", format!("{:.1} BPM", s.bpm)),
            Item::PadLevel => ("Pad level", match self.pad_level {
                PadLevel::Off => "As played".to_string(),
                PadLevel::Full => "Full (127)".to_string(),
                PadLevel::Half => "Half (64)".to_string(),
            }),
            Item::Latch => ("Latch", if self.latched { "On" } else { "Off" }.to_string()),
            Item::ShowPressure => ("Pad pressure in the MIDI log", if s.show_pressure { "Shown" } else { "Hidden" }.to_string()),
            Item::MidiLog => ("MIDI log", if s.midi_paused { "Paused" } else { "Running" }.to_string()),
            Item::ClearLogs => ("Clear the MIDI and event logs", "Clear".to_string()),
            Item::ConsoleFollow => ("Console", if s.console_paused { "Paused" } else { "Following" }.to_string()),
            Item::ClearConsole => ("Clear the console", "Clear".to_string()),
        }
    }

    fn item_help(&self, item: Item) -> Vec<&'static str> {
        match item {
            Item::Curve => vec!["How hard a pad must be hit for a loud note.",
                                "Soft: loud notes come easily. Hard: more of the",
                                "pad's travel stays below the top, for more",
                                "dynamics. Linear plays the pad's own value.",
                                "FULL LEVEL and Half Level override it."],
            Item::ScreenVelocity => vec!["The velocity of pads played on the screen.",
                                         "The touchscreen cannot sense how hard you tap."],
            Item::LatchMode => vec!["With Latch on (SHIFT + NOTE REPEAT), pads keep",
                                    "repeating after you let go.",
                                    "Replace: a fresh press starts a new set.",
                                    "Toggle: each press adds or removes that pad."],
            Item::Buffer => vec!["How much sound is queued ahead of the speakers.",
                                 "3 periods is what Akai's app uses. 2 periods",
                                 "plays about 2.9 ms sooner after a hit, with",
                                 "more risk of clicks (underruns). Changing it",
                                 "restarts the sound."],
            Item::FindButtons => vec!["Ask for MENU, NOTE REPEAT, TAP TEMPO, - and +",
                                      "again, one at a time."],
            Item::TouchLab => vec!["Timed touchscreen tests: taps, long presses,",
                                   "drags, swipes, flings and held fingers. Each",
                                   "step's result goes to the system log. Exit,",
                                   "MENU or a page button leaves it."],
            Item::Rate => vec!["How often a held pad repeats with NOTE REPEAT",
                               "or Latch: from quarter notes to 32nd notes,",
                               "T for triplets."],
            Item::Tempo => vec!["The tempo Note Repeat follows. TAP TEMPO",
                                "sets it too: tap it three or four times."],
            Item::PadLevel => vec!["FULL LEVEL plays every hit at 127,",
                                   "SHIFT + FULL LEVEL (Half Level) at 64."],
            Item::Latch => vec!["SHIFT + NOTE REPEAT. Pads you play keep",
                                "repeating until Latch goes off or STOP."],
            Item::ShowPressure => vec!["Pad pressure (aftertouch) sends many messages",
                                       "while a pad is held. Hidden, they are counted",
                                       "but not listed."],
            Item::MidiLog => vec!["Pause the MIDI log to read it."],
            Item::ClearLogs => vec!["Empty the MIDI and event logs."],
            Item::ConsoleFollow => vec!["Pause the console to read it; it keeps",
                                        "collecting lines meanwhile."],
            Item::ClearConsole => vec!["Empty the console page."],
        }
    }

    pub fn menu_view(&self) -> Effect {
        let items = self.items();
        let selected = self.selected.min(items.len() - 1);
        let title = match (self.view, self.back) {
            (View::Prefs, _) => "Prefs".to_string(),
            (_, b) => format!("{} options", TABS[b.tab().unwrap_or(0)]),
        };
        let (name, _) = self.item_text(items[selected]);
        Effect::Menu(MenuView {
            title,
            rows: items.iter().map(|&i| {
                let (n, v) = self.item_text(i);
                (n.to_string(), v)
            }).collect(),
            selected,
            help_heading: name.to_string(),
            help: self.item_help(items[selected]).into_iter().map(String::from).collect(),
        })
    }

    /// A menu row touched.
    pub fn select(&mut self, i: usize) -> Vec<Effect> {
        if !self.in_menu() {
            return Vec::new();
        }
        self.selected = i.min(self.items().len() - 1);
        vec![self.menu_view()]
    }

    /// Step the value of row `i` by `delta` (an arrow, the wheel, - or +).
    pub fn step_item(&mut self, i: usize, delta: i32) -> Vec<Effect> {
        if !self.in_menu() {
            return Vec::new();
        }
        let items = self.items();
        let i = i.min(items.len() - 1);
        self.selected = i;
        let d = delta.signum();
        let mut fx = Vec::new();
        fn cycle<T: Copy + PartialEq>(all: &[T], now: T, d: i32) -> T {
            let at = all.iter().position(|&x| x == now).unwrap_or(0) as i32;
            all[(at + d).clamp(0, all.len() as i32 - 1) as usize]
        }
        let s = &mut self.settings;
        match items[i] {
            Item::Curve => s.curve = cycle(&Curve::ALL, s.curve, d),
            Item::ScreenVelocity => s.screen_velocity = (s.screen_velocity as i32 + 10 * d).clamp(10, 127) as u8,
            Item::LatchMode => {
                s.latch_mode = if s.latch_mode == LatchMode::Toggle { LatchMode::Replace } else { LatchMode::Toggle };
                self.update_engine();
            }
            Item::Buffer => {
                let p = (s.periods as i32 + d).clamp(2, 3) as u32;
                if p != s.periods {
                    s.periods = p;
                    fx.push(Effect::Audio(p));
                }
            }
            Item::FindButtons => {
                self.guide = 0;
                self.guide_note.clear();
                self.view = View::Guide;
                return vec![self.guide_view(), Effect::Show(View::Guide)];
            }
            Item::TouchLab => return vec![Effect::TouchLab],
            Item::Rate => {
                s.rate = cycle(&Rate::ALL, s.rate, d);
                self.update_engine();
                fx.push(self.status());
            }
            Item::Tempo => {
                s.bpm = (s.bpm + d as f64).clamp(30.0, 300.0).round();
                self.update_engine();
                fx.push(self.status());
            }
            Item::PadLevel => {
                self.pad_level = cycle(&[PadLevel::Off, PadLevel::Full, PadLevel::Half], self.pad_level, d);
                let on = self.pad_level != PadLevel::Off;
                fx.push(Effect::Light(button::FULL_LEVEL, if on { LIGHT_ON } else { LIGHT_OFF }));
                fx.push(self.status());
            }
            Item::Latch => {
                self.latched = d > 0;
                self.update_engine();
                fx.push(self.status());
                fx.extend(self.repeat_light());
            }
            Item::ShowPressure => s.show_pressure = d > 0,
            Item::MidiLog => s.midi_paused = d > 0,
            Item::ClearLogs => fx.push(Effect::ClearLogs),
            Item::ConsoleFollow => s.console_paused = d > 0,
            Item::ClearConsole => fx.push(Effect::ClearConsole),
        }
        fx.push(Effect::Settings(self.settings.clone()));
        fx.push(self.menu_view());
        fx
    }

    // ------------------------------------------------------------ guide

    fn guide_view(&self) -> Effect {
        let found = GuideStep::ALL.iter().map(|&s| {
            let v = match self.map.get(s) {
                Some(n) => format!("button {n}"),
                None if GuideStep::ALL[self.guide.min(4)] == s && self.guide < GuideStep::ALL.len() => "now".to_string(),
                None => "-".to_string(),
            };
            (s.short().to_string(), v)
        }).collect();
        let (heading, mut lines) = match GuideStep::ALL.get(self.guide) {
            Some(step) => (format!("Press {}", step.label()), vec![
                "MPFree does not know this button's number yet.".to_string(),
                "Press it once; the number goes in the log for next time.".to_string(),
                "Not on your MPC, or not sure? Press STOP or tap Skip.".to_string(),
            ]),
            None => ("All done".to_string(), vec!["Tap Done to play.".to_string()]),
        };
        if !self.guide_note.is_empty() {
            lines.insert(0, self.guide_note.clone());
        }
        Effect::Guide(GuideView { heading, lines, found })
    }

    fn guide_press(&mut self, note: u8) -> Vec<Effect> {
        // STOP skips a button, so the guide can be finished without touch.
        if note == button::STOP {
            return self.guide_button(if self.guide < GuideStep::ALL.len() { 0 } else { 1 });
        }
        let Some(&step) = GuideStep::ALL.get(self.guide) else { return vec![self.guide_view()] };
        if let Some(name) = self.map.name(note).filter(|_| self.map.get(step) != Some(note)) {
            if self.map.get(step).is_none() || GuideStep::ALL.iter().all(|&s| s.short() != name) {
                self.guide_note = format!("That was {name} (button {note}).");
                return vec![self.guide_view()];
            }
        }
        // A guessed number is replaced by what was pressed.
        if step == GuideStep::TapTempo {
            self.map.tap_tempo = None;
        }
        self.map.set(step, note);
        self.guide_note = format!("{} is button {note}.", step.short());
        let mut fx = vec![Effect::Note(format!("button guide: {} is button {note}", step.short()))];
        fx.extend(self.next_guide_step());
        fx
    }

    fn next_guide_step(&mut self) -> Vec<Effect> {
        self.guide += 1;
        if self.guide < GuideStep::ALL.len() {
            return vec![self.guide_view()];
        }
        self.finish_guide()
    }

    fn finish_guide(&mut self) -> Vec<Effect> {
        self.guide = GuideStep::ALL.len();
        self.view = self.back;
        let mut fx = vec![Effect::Note(format!("button guide done: {}", self.map.describe())), self.status()];
        fx.extend(self.repeat_light());
        fx.push(Effect::Show(self.view));
        fx
    }

    /// The guide card's buttons: 0 skips this button, 1 ends the guide.
    pub fn guide_button(&mut self, i: usize) -> Vec<Effect> {
        if self.view != View::Guide {
            return Vec::new();
        }
        if i == 0 && self.guide < GuideStep::ALL.len() {
            let step = GuideStep::ALL[self.guide];
            self.guide_note = format!("Skipped {}.", step.short());
            let mut fx = vec![Effect::Note(format!("button guide: {} skipped", step.short()))];
            fx.extend(self.next_guide_step());
            fx
        } else {
            self.finish_guide()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_arp::EventKind;

    const MENU: u8 = 123;
    const REPEAT: u8 = 11;

    fn ready() -> App {
        let mut app = App::new(44_100, false);
        app.map.menu = Some(MENU);
        app.map.note_repeat = Some(REPEAT);
        let fx = app.start();
        assert!(fx.contains(&Effect::Show(View::Play)));
        app
    }

    fn press(app: &mut App, note: u8) -> Vec<Effect> {
        let fx = app.button(note, true, 0.0);
        app.button(note, false, 0.0);
        fx
    }

    fn ons(app: &mut App, frames: u32) -> Vec<(u32, u8, u8)> {
        let mut ev = Events::new();
        app.process(frames, &mut ev);
        ev.as_slice().iter().filter_map(|e| match e.kind {
            EventKind::On { note, velocity } => Some((e.offset, note, velocity)),
            EventKind::Off { .. } => None,
        }).collect()
    }

    #[test]
    fn curves_keep_the_ends_and_bend_the_middle() {
        for c in Curve::ALL {
            assert_eq!(c.apply(127), 127);
            assert_eq!(c.apply(0), 1);
        }
        assert_eq!(Curve::Linear.apply(64), 64);
        assert!(Curve::Soft.apply(64) > 80);
        assert!(Curve::Hard.apply(64) <= 40);
        assert!(Curve::Hard.apply(120) <= 115, "Hard keeps strong hits below the top");
    }

    #[test]
    fn full_and_half_level_toggle_with_their_light() {
        let mut app = ready();
        assert_eq!(app.velocity(40), 40);
        let fx = press(&mut app, button::FULL_LEVEL);
        assert!(fx.contains(&Effect::Light(button::FULL_LEVEL, LIGHT_ON)));
        assert_eq!(app.velocity(40), 127);
        app.button(button::SHIFT, true, 0.0);
        press(&mut app, button::FULL_LEVEL);
        app.button(button::SHIFT, false, 0.0);
        assert_eq!(app.velocity(127), 64, "SHIFT + FULL LEVEL is Half Level");
        let fx = app.button(button::SHIFT, true, 0.0);
        assert!(matches!(&fx[0], Effect::Status(s) if s.shift));
        let fx = press(&mut app, button::FULL_LEVEL);
        assert!(fx.contains(&Effect::Light(button::FULL_LEVEL, LIGHT_OFF)));
        assert_eq!(app.velocity(40), 40);
    }

    #[test]
    fn pads_play_at_once_and_repeat_while_note_repeat_is_held() {
        let mut app = ready();
        app.pad_down(0, 100);
        assert_eq!(ons(&mut app, 128), vec![(0, 0, 100)]);
        assert!(ons(&mut app, 44_100).is_empty(), "no repeat without NOTE REPEAT");
        let fx = app.button(REPEAT, true, 0.0);
        assert!(fx.contains(&Effect::Light(REPEAT, LIGHT_ON)));
        // 1/16 at 120 BPM = 5512.5 frames: hits at 0 and 5512.5 within 6000 frames.
        let got = ons(&mut app, 6000);
        assert_eq!(got.len(), 2, "{got:?}");
        assert_eq!(got[0].0, 0);
        assert!((5512..=5513).contains(&got[1].0), "{got:?}");
        app.pressure(0, 30);
        let got = ons(&mut app, 6000);
        assert_eq!(got[0].2, 30, "repeats follow pad pressure");
        let fx = app.button(REPEAT, false, 0.0);
        assert!(fx.contains(&Effect::Light(REPEAT, LIGHT_OFF)));
        assert!(ons(&mut app, 20_000).is_empty());
    }

    #[test]
    fn latch_keeps_pads_repeating_until_it_goes_off_or_stop() {
        let mut app = ready();
        app.button(button::SHIFT, true, 0.0);
        let fx = press(&mut app, REPEAT);
        app.button(button::SHIFT, false, 0.0);
        assert!(fx.contains(&Effect::Light(REPEAT, LIGHT_ON)) && app.repeating());
        app.pad_down(4, 90);
        app.pad_up(4);
        assert_eq!(ons(&mut app, 11_100).len(), 3, "keeps repeating after release");
        assert!(app.active(4));
        let fx = press(&mut app, button::STOP);
        assert!(fx.contains(&Effect::StopSounds) && !app.repeating());
        assert!(ons(&mut app, 20_000).is_empty());
        // NOTE REPEAT alone while latched turns Latch off.
        app.button(button::SHIFT, true, 0.0);
        press(&mut app, REPEAT);
        app.button(button::SHIFT, false, 0.0);
        app.pad_down(2, 90);
        app.pad_up(2);
        press(&mut app, REPEAT);
        assert!(!app.repeating());
        let _ = ons(&mut app, 128);
        assert!(ons(&mut app, 20_000).is_empty());
    }

    #[test]
    fn stop_ends_the_timed_step_and_shift_stop_toggles_the_console() {
        let mut timed = App::new(44_100, true);
        assert_eq!(press(&mut timed, button::STOP), vec![Effect::Quit]);
        let mut app = ready();
        app.button(button::SHIFT, true, 0.0);
        assert_eq!(press(&mut app, button::STOP), vec![Effect::Show(View::Console)]);
        assert_eq!(press(&mut app, button::STOP), vec![Effect::Show(View::Play)]);
    }

    #[test]
    fn menu_and_shift_menu_open_options_and_prefs() {
        let mut app = ready();
        let fx = press(&mut app, MENU);
        assert_eq!(app.view(), View::Options);
        assert!(matches!(&fx[0], Effect::Menu(m) if m.title == "Play options" && m.rows[0].0 == "Note repeat rate"));
        // The wheel moves between rows; SHIFT + wheel steps the value.
        let fx = app.wheel(1);
        assert!(matches!(&fx[0], Effect::Menu(m) if m.selected == 1));
        app.wheel(-5);
        app.button(button::SHIFT, true, 0.0);
        let fx = app.wheel(1);
        app.button(button::SHIFT, false, 0.0);
        assert_eq!(app.settings.rate, Rate::SixteenthTriplet);
        assert!(fx.iter().any(|e| matches!(e, Effect::Menu(m) if m.selected == 0 && m.rows[0].1 == "1/16T")));
        press(&mut app, MENU);
        assert_eq!(app.view(), View::Play);
        app.button(button::SHIFT, true, 0.0);
        press(&mut app, MENU);
        app.button(button::SHIFT, false, 0.0);
        assert_eq!(app.view(), View::Prefs);
        // Touch: select the curve and step it down to Soft.
        let fx = app.step_item(0, -1);
        assert_eq!(app.settings.curve, Curve::Soft);
        assert!(fx.contains(&Effect::Settings(app.settings.clone())));
        // The audio buffer restarts the sound only when it changes.
        assert_eq!(app.step_item(3, 1).iter().filter(|e| matches!(e, Effect::Audio(_))).count(), 0);
        assert!(app.step_item(3, -1).contains(&Effect::Audio(2)));
        // The Debug page's options.
        app.tab(1);
        press(&mut app, MENU);
        assert!(matches!(app.menu_view(), Effect::Menu(m) if m.title == "Debug options"));
        // The wheel picks kits outside menus.
        press(&mut app, MENU);
        assert_eq!(app.wheel(-2), vec![Effect::Kit(-2)]);
    }

    #[test]
    fn stop_skips_a_button_in_the_guide() {
        let mut app = App::new(44_100, false);
        app.start();
        let fx = press(&mut app, button::STOP);
        assert!(fx.contains(&Effect::Note("button guide: MENU skipped".into())));
        assert_eq!(app.view(), View::Guide);
        for _ in 0..4 {
            press(&mut app, button::STOP);
        }
        assert_eq!(app.view(), View::Play);
        assert_eq!(app.map.menu, None);
    }

    #[test]
    fn the_guide_learns_buttons_by_name() {
        let mut app = App::new(44_100, false);
        let fx = app.start();
        assert_eq!(app.view(), View::Guide);
        assert!(fx.iter().any(|e| matches!(e, Effect::Guide(g) if g.heading == "Press MENU")));
        // A measured button is named, not taken.
        let fx = press(&mut app, button::FULL_LEVEL);
        assert!(matches!(&fx[0], Effect::Guide(g) if g.lines[0].contains("FULL LEVEL")));
        assert_eq!(app.map.menu, None);
        let fx = press(&mut app, MENU);
        assert!(fx.contains(&Effect::Note("button guide: MENU is button 123".into())));
        assert_eq!(app.map.menu, Some(MENU));
        // MENU again while asked for NOTE REPEAT is named, not taken twice.
        press(&mut app, MENU);
        assert_eq!(app.map.note_repeat, None);
        press(&mut app, REPEAT);
        assert_eq!(app.map.note_repeat, Some(REPEAT));
        // TAP TEMPO replaces the guess.
        press(&mut app, 54);
        assert_eq!(app.map.tap_tempo, Some(54));
        app.guide_button(0); // skip -
        let fx = app.guide_button(1); // done
        assert_eq!(app.view(), View::Play);
        assert!(fx.iter().any(|e| matches!(e, Effect::Note(n) if n.contains("MENU = button 123")
                                             && n.contains("- = not found"))));
        // The guide again from Prefs.
        app.tab(3);
        let fx = app.step_item(4, 1);
        assert_eq!(app.view(), View::Guide);
        assert!(fx.contains(&Effect::Show(View::Guide)));
    }

    #[test]
    fn tap_tempo_sets_the_tempo_and_blinks() {
        let mut app = ready();
        for k in 0..4 {
            app.button(button::TAP_TEMPO, true, 10.0 + k as f64 * 0.5);
        }
        assert_eq!(app.settings.bpm, 120.0);
        app.button(button::TAP_TEMPO, true, 20.0);
        app.button(button::TAP_TEMPO, true, 20.6);
        assert_eq!(app.settings.bpm, 100.0, "a pause starts a new count");
        assert_eq!(app.tick(20.61), Some(Effect::Light(button::TAP_TEMPO, LIGHT_ON)));
        assert_eq!(app.tick(20.62), None);
        assert_eq!(app.tick(20.9), Some(Effect::Light(button::TAP_TEMPO, LIGHT_OFF)));
    }
}
