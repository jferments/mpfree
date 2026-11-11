//! What the pad player's controls do: the buttons, the wheel, the Q-Links,
//! the screen's tabs and menus, the button guide, Note Repeat, Latch and the
//! arp (the quick band, the Arp panel and the pattern workspace; the rules
//! are in the `arp` submodule).
//!
//! Pure logic, no I/O: the audio thread hands it the controller's events and
//! the screen's actions, and carries out the `Effect`s it returns (lights,
//! pages, kit changes). Pads go through the arpeggiator thread's engine
//! (`mpfree-arp`), which plays them directly, repeats them (Note Repeat and
//! Latch) or arpeggiates them on their exact frames. Pads are fed as slots
//! 0..15 (pad 1 = 0), as that engine expects.
//!
//! Buttons: SHIFT 49, FULL LEVEL 39, STOP 81 and PLAY 82 are measured
//! (docs/hardware/controller-protocol.md); MENU 123, NOTE REPEAT 11, TAP
//! TEMPO 53, - 55 and + 54 were pressed in the round 5 button guide, so the
//! guide no longer runs at power-on (Prefs still has it). Latch as SHIFT +
//! NOTE REPEAT worked in round 5; Half Level as SHIFT + FULL LEVEL is read
//! from the panel's print and not yet tried.

mod arp;

pub use arp::{built_in_patterns, mode_label, ArpItem, ArpView, WorkspaceView};

use mpfree_arp::{Engine, Events, LatchMode, Mode, Pattern, Rate, RecordMode, Recorder};
use mpfree_controller::mpc_one::button;

/// Button lights take the button's note as the CC number on the Private
/// cable. Akai's app sends 1 and 3 (dim and bright, inferred); so do we.
pub const LIGHT_OFF: u8 = 1;
pub const LIGHT_ON: u8 = 3;
/// Two NOTE REPEAT presses this close together (in seconds) latch, as
/// SHIFT + NOTE REPEAT does.
pub const DOUBLE_PRESS_SECONDS: f64 = 0.4;

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
    /// The Arp panel beside the pads (not a tab: it belongs to Play).
    Arp,
    /// The pattern workspace (full screen; Back returns to the panel).
    Pattern,
}

pub const TABS: [&str; 4] = ["Play", "Debug", "Console", "Prefs"];

impl View {
    pub fn from_tab(i: usize) -> View {
        [View::Play, View::Debug, View::Console, View::Prefs].get(i).copied().unwrap_or(View::Play)
    }

    pub fn tab(self) -> Option<usize> {
        match self {
            View::Play | View::Arp | View::Pattern => Some(0),
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
    Rec,
    Overdub,
    Undo,
    PlayStart,
}

impl GuideStep {
    const ALL: [GuideStep; 9] =
        [GuideStep::Menu, GuideStep::NoteRepeat, GuideStep::TapTempo, GuideStep::Minus, GuideStep::Plus,
         GuideStep::Rec, GuideStep::Overdub, GuideStep::Undo, GuideStep::PlayStart];

    pub fn label(self) -> &'static str {
        match self {
            GuideStep::Menu => "MENU",
            GuideStep::NoteRepeat => "NOTE REPEAT",
            GuideStep::TapTempo => "TAP TEMPO",
            GuideStep::Minus => "- (next to the wheel)",
            GuideStep::Plus => "+ (next to the wheel)",
            GuideStep::Rec => "REC",
            GuideStep::Overdub => "OVERDUB",
            GuideStep::Undo => "UNDO",
            GuideStep::PlayStart => "PLAY START",
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

/// The buttons the guide asks for: measured in round 5, or learned again.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ButtonMap {
    pub menu: Option<u8>,
    pub note_repeat: Option<u8>,
    pub tap_tempo: Option<u8>,
    pub minus: Option<u8>,
    pub plus: Option<u8>,
    /// For the time engine (transport and recording); nothing uses them yet.
    pub rec: Option<u8>,
    pub overdub: Option<u8>,
    pub undo: Option<u8>,
    pub play_start: Option<u8>,
}

impl ButtonMap {
    /// The numbers the round 5 guide found on the reference MPC One.
    pub fn measured() -> ButtonMap {
        ButtonMap { menu: Some(button::MENU), note_repeat: Some(button::NOTE_REPEAT), tap_tempo: Some(button::TAP_TEMPO),
                    minus: Some(button::MINUS), plus: Some(button::PLUS), ..ButtonMap::default() }
    }

    fn get(&self, s: GuideStep) -> Option<u8> {
        match s {
            GuideStep::Menu => self.menu,
            GuideStep::NoteRepeat => self.note_repeat,
            GuideStep::TapTempo => self.tap_tempo,
            GuideStep::Minus => self.minus,
            GuideStep::Plus => self.plus,
            GuideStep::Rec => self.rec,
            GuideStep::Overdub => self.overdub,
            GuideStep::Undo => self.undo,
            GuideStep::PlayStart => self.play_start,
        }
    }

    fn set(&mut self, s: GuideStep, note: u8) {
        let slot = match s {
            GuideStep::Menu => &mut self.menu,
            GuideStep::NoteRepeat => &mut self.note_repeat,
            GuideStep::TapTempo => &mut self.tap_tempo,
            GuideStep::Minus => &mut self.minus,
            GuideStep::Plus => &mut self.plus,
            GuideStep::Rec => &mut self.rec,
            GuideStep::Overdub => &mut self.overdub,
            GuideStep::Undo => &mut self.undo,
            GuideStep::PlayStart => &mut self.play_start,
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
    /// The arp's own settings (the Options menu opened from the Arp panel).
    Arp(ArpItem),
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
    /// What the arp screens show: the panel, the quick band and the chip.
    Arp(ArpView),
    /// The arp's pattern list (names and patterns), when it changes.
    Patterns(Vec<(String, Pattern)>),
    /// The pattern workspace: open it on a pattern, or start or stop
    /// recording on the pads.
    Workspace(WorkspaceView),
    /// A step recorded on the pads: the pattern so far (the steps not yet
    /// written are rests) and the next step to write.
    Recorded(Pattern, usize),
    /// The data wheel in the workspace: turn the selected cell.
    WorkspaceWheel(i32),
    /// Write this library text to the patterns file (off the audio thread).
    SavePatterns(String),
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
    /// When NOTE REPEAT was last pressed to hold, for the double press
    /// that latches.
    repeat_pressed_at: Option<f64>,
    latched: bool,
    pad_level: PadLevel,
    view: View,
    /// The page to go back to from Console, Options or the guide.
    back: View,
    selected: usize,
    /// The buttons the guide asks for this time, and which one it is on.
    guide_steps: Vec<GuideStep>,
    guide: usize,
    guide_note: String,
    taps: Vec<f64>,
    /// When the tempo's beats started (seconds), for the TAP TEMPO light.
    beat_zero: f64,
    tap_light: bool,
    /// The mode the Arp panel picked. NOTE REPEAT and Latch turn on Repeat
    /// when it is Direct.
    arp_mode: Mode,
    /// The arp's other settings (order, octaves, gate, swing); the rate is
    /// `settings.rate` and the mode follows the buttons and pages.
    arp: mpfree_arp::Settings,
    /// The scale's root (0 = C), kept while the scale is Off.
    scale_root: u8,
    /// The transport's clock for the next period ([`App::set_clock`]).
    clock: Option<mpfree_arp::Clock>,
    /// While the transport plays: its swing (true) or the arp's own.
    swing_from_transport: bool,
    /// The patterns to pick from (built in; Save changes them in memory).
    patterns: Vec<(String, Pattern)>,
    pattern_index: usize,
    /// The pattern the engine plays (edited, mutated or recorded from the
    /// list's).
    pattern: Pattern,
    mutations: u32,
    /// Recording a pattern on the pads (Step mode) in the workspace.
    recorder: Option<Recorder>,
    /// How the next recording on the pads goes: Step time or Free time.
    record_mode: RecordMode,
    /// Save also writes the patterns file (`--patterns` with
    /// `--save-patterns`), through [`Effect::SavePatterns`].
    pub pattern_file: bool,
    /// Q-Link turns not yet used, per knob (rate and octaves need several).
    qlink_turns: [i32; 4],
    /// What the arp screens were last sent: the step and the notes, and
    /// whether anything else changed since.
    sent_step: u64,
    sent_notes: mpfree_arp::NoteSet,
    arp_dirty: bool,
}

impl App {
    pub fn new(sample_rate: u32, timed: bool) -> App {
        let settings = Settings::default();
        let engine = Engine::new(sample_rate, settings.bpm);
        let patterns = arp::built_in_patterns();
        let pattern = patterns.first().map(|(_, p)| *p).unwrap_or_default();
        let sent_notes = engine.active().clone();
        let mut app = App { settings, map: ButtonMap::measured(), engine, timed, shift: false, repeat_held: false,
                            repeat_pressed_at: None,
                            latched: false, pad_level: PadLevel::Off, view: View::Play, back: View::Play,
                            selected: 0, guide_steps: Vec::new(), guide: 0, guide_note: String::new(),
                            taps: Vec::new(), beat_zero: 0.0,
                            tap_light: false, arp_mode: Mode::Repeat, arp: mpfree_arp::Settings::default(), scale_root: 0, clock: None, swing_from_transport: true, patterns,
                            pattern_index: 0, pattern, mutations: 0, recorder: None,
                            record_mode: RecordMode::Step, pattern_file: false, qlink_turns: [0; 4],
                            sent_step: 0, sent_notes, arp_dirty: true };
        app.engine.set_pattern(pattern);
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
                          Effect::Light(button::FULL_LEVEL, LIGHT_OFF), Effect::Patterns(self.patterns.clone()),
                          self.arp_view()];
        fx.extend(self.repeat_light());
        // At power-on the guide asks only for buttons nobody has pressed yet.
        let unknown: Vec<GuideStep> = GuideStep::ALL.iter().copied().filter(|&s| self.map.get(s).is_none()).collect();
        if !unknown.is_empty() {
            self.back = View::Play;
            self.open_guide(unknown);
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

    /// A pad pressed: it plays, and writes a step while recording a pattern.
    pub fn pad_down(&mut self, slot: usize, velocity: u8) -> Vec<Effect> {
        if !self.free_time_recording() {
            // In Free time the pads only write steps; the pattern keeps
            // playing the latched notes.
            self.engine.press(slot as u8, velocity.max(1));
        }
        self.record_press(slot as u8, velocity)
    }

    pub fn pad_up(&mut self, slot: usize) -> Vec<Effect> {
        if !self.free_time_recording() {
            self.engine.release(slot as u8);
        }
        self.record_release(slot as u8)
    }

    pub fn pressure(&mut self, slot: usize, value: u8) {
        self.engine.pressure(slot as u8, value);
    }

    /// One audio period: the notes that start (or stop) in it. The arp
    /// follows the clock given by [`App::set_clock`] for this period, and
    /// free-runs at the app's tempo without one.
    pub fn process(&mut self, frames: u32, out: &mut Events) {
        self.sync_arp();
        self.engine.process(frames, out);
    }

    /// Pads the engine keeps playing (held, or latched).
    pub fn active(&self, slot: usize) -> bool {
        self.engine.active().contains(slot as u8)
    }

    /// NOTE REPEAT is held or Latch is on.
    pub fn repeating(&self) -> bool {
        self.repeat_held || self.latched
    }

    /// The engine makes notes of its own (Repeat, Arp or Pattern), so a hit
    /// may sound later than the press, and latched pads stay lit.
    pub fn generating(&self) -> bool {
        self.engine.settings().mode != Mode::Direct
    }

    fn update_engine(&mut self) {
        let mut s = self.arp;
        s.mode = self.engine_mode();
        s.rate = self.settings.rate;
        self.engine.set_settings(s);
        let latch = if self.latched { self.settings.latch_mode } else { LatchMode::Off };
        if self.engine.latch() != latch {
            self.engine.set_latch(latch);
        }
        self.engine.set_tempo(self.settings.bpm);
        self.arp_dirty = true;
    }

    #[cfg(test)]
    fn status_now(&self) -> Status {
        match self.status() {
            Effect::Status(s) => s,
            _ => unreachable!(),
        }
    }

    fn status(&self) -> Effect {
        Effect::Status(Status { shift: self.shift, pad_level: self.pad_level, repeat: self.repeating(),
                                latched: self.latched, rate: self.settings.rate.label(), bpm: self.settings.bpm })
    }

    /// The NOTE REPEAT light: on while the pads play through the arp.
    fn repeat_light(&self) -> Option<Effect> {
        self.map.note_repeat.map(|n| Effect::Light(n, if self.generating() { LIGHT_ON } else { LIGHT_OFF }))
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
                fx.extend(self.arp_changed());
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
            let double = self.repeat_pressed_at.is_some_and(|at| t - at <= DOUBLE_PRESS_SECONDS);
            self.repeat_pressed_at = None;
            if shift || self.latched || double {
                // SHIFT + NOTE REPEAT, or NOTE REPEAT pressed twice, toggles
                // Latch; NOTE REPEAT alone lets it go.
                fx.extend(self.toggle_latch());
            } else {
                self.repeat_held = true;
                self.repeat_pressed_at = Some(t);
                self.qlink_turns = [0; 4];
                fx.extend(self.arp_changed());
            }
        } else if Some(note) == self.map.menu {
            if self.repeat_held {
                // NOTE REPEAT + MENU opens and closes the Arp panel.
                fx.extend(self.toggle_panel());
            } else if shift {
                fx.extend(self.toggle(View::Prefs));
            } else if self.view != View::Pattern {
                fx.extend(self.toggle(View::Options));
            }
        } else if Some(note) == self.map.tap_tempo {
            fx.extend(self.tap(t));
        } else if Some(note) == self.map.minus || Some(note) == self.map.plus {
            let d = if Some(note) == self.map.plus { 1 } else { -1 };
            if self.in_menu() {
                fx.extend(self.step_item(self.selected, d));
            } else if self.repeat_held || self.view == View::Arp {
                fx.extend(self.arp_mode_step(d));
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
            View::Pattern => vec![Effect::WorkspaceWheel(delta)],
            // NOTE REPEAT held or the Arp panel: the rate, order or pattern.
            _ if self.repeat_held || self.view == View::Arp => self.arp_wheel(delta),
            _ => vec![Effect::Kit(delta)],
        }
    }

    // ------------------------------------------------------------ pages

    fn in_menu(&self) -> bool {
        matches!(self.view, View::Options | View::Prefs)
    }

    /// Show `v`. The page left is remembered for going back, unless it is
    /// a menu, the guide or the workspace (whose Back goes to the panel).
    fn show(&mut self, v: View) -> Vec<Effect> {
        if self.view != v && matches!(self.view, View::Play | View::Debug | View::Console | View::Arp) {
            self.back = self.view;
        }
        self.view = v;
        let mut fx = Vec::new();
        if self.in_menu() {
            self.selected = 0;
            fx.push(self.menu_view());
        }
        fx.push(Effect::Show(v));
        fx.extend(self.after_view_change());
        fx
    }

    /// Open `v`, or go back if it is open already.
    fn toggle(&mut self, v: View) -> Vec<Effect> {
        if self.view == v {
            let back = if self.back == v { View::Play } else { self.back };
            self.view = back;
            let mut fx = vec![Effect::Show(back)];
            fx.extend(self.after_view_change());
            fx
        } else {
            self.show(v)
        }
    }

    /// The Arp panel and the workspace play the pads through the arp, so
    /// the engine and the NOTE REPEAT light follow the page. Leaving the
    /// workspace stops recording.
    fn after_view_change(&mut self) -> Vec<Effect> {
        let mut fx = Vec::new();
        if self.view != View::Pattern && self.recorder.is_some() {
            fx.extend(self.set_recording(false));
        }
        let was = self.generating();
        self.update_engine();
        if self.generating() != was {
            fx.extend(self.repeat_light());
        }
        fx
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
            (_, View::Arp) => &arp::ARP_OPTIONS,
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
            Item::Arp(a) => self.arp_item_text(a),
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
            Item::LatchMode => vec!["With Latch on (NOTE REPEAT twice, or SHIFT +",
                                    "NOTE REPEAT), pads keep repeating after you",
                                    "let go.",
                                    "Replace: a fresh press starts a new set.",
                                    "Toggle: each press adds or removes that pad."],
            Item::Buffer => vec!["How much sound is queued ahead of the speakers.",
                                 "3 periods is what Akai's app uses. 2 periods",
                                 "plays about 2.9 ms sooner after a hit, with",
                                 "more risk of clicks (underruns). Changing it",
                                 "restarts the sound."],
            Item::FindButtons => vec!["Ask for MENU, NOTE REPEAT, TAP TEMPO, - and +,",
                                      "REC, OVERDUB, UNDO and PLAY START again,",
                                      "one at a time."],
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
            Item::Latch => vec!["NOTE REPEAT twice, or SHIFT + NOTE REPEAT.",
                                "Pads you play keep repeating until Latch goes",
                                "off (NOTE REPEAT once) or STOP."],
            Item::ShowPressure => vec!["Pad pressure (aftertouch) sends many messages",
                                       "while a pad is held. Hidden, they are counted",
                                       "but not listed."],
            Item::MidiLog => vec!["Pause the MIDI log to read it."],
            Item::ClearLogs => vec!["Empty the MIDI and event logs."],
            Item::ConsoleFollow => vec!["Pause the console to read it; it keeps",
                                        "collecting lines meanwhile."],
            Item::ClearConsole => vec!["Empty the console page."],
            Item::Arp(a) => Self::arp_item_help(a),
        }
    }

    pub fn menu_view(&self) -> Effect {
        let items = self.items();
        let selected = self.selected.min(items.len() - 1);
        let title = match (self.view, self.back) {
            (View::Prefs, _) => "Prefs".to_string(),
            (_, View::Arp) => "Arp options".to_string(),
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
                self.open_guide(GuideStep::ALL.to_vec());
                return vec![self.guide_view(), Effect::Show(View::Guide)];
            }
            Item::TouchLab => return vec![Effect::TouchLab],
            Item::Rate => {
                s.rate = cycle(&Rate::ALL, s.rate, d);
                fx.extend(self.arp_changed());
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
                fx.extend(self.arp_changed());
            }
            Item::ShowPressure => s.show_pressure = d > 0,
            Item::MidiLog => s.midi_paused = d > 0,
            Item::ClearLogs => fx.push(Effect::ClearLogs),
            Item::ConsoleFollow => s.console_paused = d > 0,
            Item::ClearConsole => fx.push(Effect::ClearConsole),
            Item::Arp(a) => fx.extend(self.step_arp_item(a, d)),
        }
        fx.push(Effect::Settings(self.settings.clone()));
        fx.push(self.menu_view());
        fx
    }

    // ------------------------------------------------------------ guide

    /// Start the guide on `steps`, one at a time.
    fn open_guide(&mut self, steps: Vec<GuideStep>) {
        self.guide_steps = steps;
        self.guide = 0;
        self.guide_note.clear();
        self.view = View::Guide;
    }

    fn guide_step(&self) -> Option<GuideStep> {
        self.guide_steps.get(self.guide).copied()
    }

    fn guide_view(&self) -> Effect {
        let now = self.guide_step();
        let found = GuideStep::ALL.iter().map(|&s| {
            let v = match self.map.get(s) {
                Some(n) => format!("button {n}"),
                None if now == Some(s) => "now".to_string(),
                None => "-".to_string(),
            };
            (s.short().to_string(), v)
        }).collect();
        let (heading, mut lines) = match now {
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
            return self.guide_button(if self.guide_step().is_some() { 0 } else { 1 });
        }
        let Some(step) = self.guide_step() else { return vec![self.guide_view()] };
        if let Some(name) = self.map.name(note).filter(|_| self.map.get(step) != Some(note)) {
            if self.map.get(step).is_none() || GuideStep::ALL.iter().all(|&s| s.short() != name) {
                self.guide_note = format!("That was {name} (button {note}).");
                return vec![self.guide_view()];
            }
        }
        // Asked again from Prefs: what was pressed replaces what was known.
        self.map.set(step, note);
        self.guide_note = format!("{} is button {note}.", step.short());
        let mut fx = vec![Effect::Note(format!("button guide: {} is button {note}", step.short()))];
        fx.extend(self.next_guide_step());
        fx
    }

    fn next_guide_step(&mut self) -> Vec<Effect> {
        self.guide += 1;
        if self.guide_step().is_some() {
            return vec![self.guide_view()];
        }
        self.finish_guide()
    }

    fn finish_guide(&mut self) -> Vec<Effect> {
        self.guide = self.guide_steps.len();
        self.view = self.back;
        let mut fx = vec![Effect::Note(format!("button guide done: {}", self.map.describe())), self.status()];
        fx.extend(self.repeat_light());
        fx.push(Effect::Show(self.view));
        fx.extend(self.after_view_change());
        fx
    }

    /// The guide card's buttons: 0 skips this button, 1 ends the guide.
    pub fn guide_button(&mut self, i: usize) -> Vec<Effect> {
        if self.view != View::Guide {
            return Vec::new();
        }
        if let (0, Some(step)) = (i, self.guide_step()) {
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
    use mpfree_arp::{EventKind, Order};
    use mpfree_arp_ui::view::{ArpAction, PatternButton};
    use mpfree_arp_ui::workspace::WsAction;

    const MENU: u8 = button::MENU;
    const REPEAT: u8 = button::NOTE_REPEAT;

    /// The pad player with every button known (the time engine's four get
    /// made-up numbers, so the guide stays closed).
    fn ready() -> App {
        let mut app = App::new(44_100, false);
        (app.map.rec, app.map.overdub, app.map.undo, app.map.play_start) = (Some(90), Some(91), Some(92), Some(93));
        let fx = app.start();
        assert!(fx.contains(&Effect::Show(View::Play)));
        app
    }

    fn held(app: &mut App, note: u8, f: impl FnOnce(&mut App) -> Vec<Effect>) -> Vec<Effect> {
        app.button(note, true, 0.0);
        let fx = f(app);
        app.button(note, false, 0.0);
        fx
    }

    fn notes(app: &mut App, frames: u32) -> Vec<u8> {
        ons(app, frames).into_iter().map(|(_, n, _)| n).collect()
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
    fn at_power_on_the_guide_asks_only_for_unknown_buttons() {
        let mut app = App::new(44_100, false);
        assert_eq!(app.map.menu, Some(123));
        assert_eq!((app.map.note_repeat, app.map.tap_tempo, app.map.minus, app.map.plus), (Some(11), Some(53), Some(55), Some(54)));
        let fx = app.start();
        assert!(fx.iter().any(|e| matches!(e, Effect::Guide(g) if g.heading == "Press REC")));
        let fx = press(&mut app, button::STOP);
        assert!(fx.contains(&Effect::Note("button guide: REC skipped".into())));
        assert_eq!(app.view(), View::Guide);
        // PLAY is named, not taken for PLAY START.
        press(&mut app, button::STOP);
        press(&mut app, button::STOP);
        let fx = press(&mut app, button::PLAY);
        assert!(matches!(&fx[0], Effect::Guide(g) if g.lines[0].contains("PLAY (button 82)")));
        press(&mut app, button::STOP);
        assert_eq!(app.view(), View::Play);
        assert_eq!(app.map.rec, None);
        assert_eq!(app.map.menu, Some(MENU));
    }

    #[test]
    fn the_guide_learns_buttons_by_name() {
        let mut app = ready();
        app.map = ButtonMap::default();
        app.tab(3);
        let fx = app.step_item(4, 1);
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
        press(&mut app, 54);
        assert_eq!(app.map.tap_tempo, Some(54));
        app.guide_button(0); // skip -
        let fx = app.guide_button(1); // done
        assert_eq!(app.view(), View::Play);
        assert!(fx.iter().any(|e| matches!(e, Effect::Note(n) if n.contains("MENU = button 123")
                                             && n.contains("- = not found") && n.contains("UNDO = not found"))));
        // The guide again from Prefs, every button.
        app.tab(3);
        let fx = app.step_item(4, 1);
        assert_eq!(app.view(), View::Guide);
        assert!(fx.contains(&Effect::Show(View::Guide)));
        assert_eq!(app.guide_steps.len(), 9);
    }

    #[test]
    fn note_repeat_turns_on_the_arp_mode_the_panel_picked() {
        let mut app = ready();
        assert_eq!(app.engine.settings().mode, Mode::Direct);
        // Hold NOTE REPEAT: Repeat, and - / + pick the mode, the wheel the order.
        app.button(REPEAT, true, 0.0);
        assert_eq!(app.engine.settings().mode, Mode::Repeat);
        press(&mut app, button::PLUS);
        assert_eq!(app.engine.settings().mode, Mode::Arp);
        app.wheel(1);
        assert_eq!(app.engine.settings().order, Order::ALL[1]);
        let fx = app.button(REPEAT, false, 0.0);
        assert!(fx.contains(&Effect::Light(REPEAT, LIGHT_OFF)));
        assert_eq!(app.engine.settings().mode, Mode::Direct);
        assert_eq!(app.wheel(1), vec![Effect::Kit(1)], "the wheel is the kit again");
        // Held again, a while later: the arp, not Repeat.
        app.pad_down(0, 100);
        app.pad_down(4, 100);
        let fx = app.button(REPEAT, true, 10.0);
        assert!(!fx.contains(&Effect::Note("Latch on".into())), "too slow for a double press");
        assert!(fx.contains(&Effect::Light(REPEAT, LIGHT_ON)));
        let _ = ons(&mut app, 128);
        let got = notes(&mut app, 4 * 5513);
        assert!(got.contains(&0) && got.contains(&4), "{got:?}");
        assert!(got.len() >= 3, "{got:?}");
    }

    #[test]
    fn note_repeat_pressed_twice_latches_like_shift_and_note_repeat() {
        let mut app = ready();
        // One press: held, and let go with the button.
        app.button(REPEAT, true, 1.0);
        app.button(REPEAT, false, 1.1);
        assert!(!app.repeating());
        // Pressed twice within DOUBLE_PRESS_SECONDS: latched, and it stays.
        app.button(REPEAT, true, 5.0);
        app.button(REPEAT, false, 5.1);
        let fx = app.button(REPEAT, true, 5.0 + DOUBLE_PRESS_SECONDS - 0.05);
        assert!(fx.contains(&Effect::Note("Latch on".into())), "{fx:?}");
        app.button(REPEAT, false, 5.4);
        assert!(app.repeating());
        app.pad_down(4, 90);
        app.pad_up(4);
        assert_eq!(ons(&mut app, 11_100).len(), 3, "keeps repeating after release");
        // One press lets go, and a press soon after is a plain hold again.
        let fx = app.button(REPEAT, true, 8.0);
        assert!(fx.contains(&Effect::Note("Latch off".into())));
        app.button(REPEAT, false, 8.1);
        assert!(!app.repeating());
        let fx = app.button(REPEAT, true, 8.2);
        assert!(!fx.contains(&Effect::Note("Latch on".into())));
        assert!(app.repeating());
        app.button(REPEAT, false, 8.3);
        assert!(!app.repeating());
        // Two presses too far apart: two holds, no latch.
        app.button(REPEAT, true, 20.0);
        app.button(REPEAT, false, 20.1);
        app.button(REPEAT, true, 20.0 + DOUBLE_PRESS_SECONDS + 0.1);
        app.button(REPEAT, false, 20.6);
        assert!(!app.repeating());
    }

    #[test]
    fn note_repeat_and_menu_open_the_arp_panel() {
        let mut app = ready();
        held(&mut app, REPEAT, |a| a.button(MENU, true, 0.0));
        app.button(MENU, false, 0.0);
        assert_eq!(app.view(), View::Arp);
        // The panel plays the pads through its mode, Repeat to begin with.
        assert_eq!(app.engine.settings().mode, Mode::Repeat);
        assert!(app.generating());
        // - / + walk its tabs, Direct included.
        let fx = press(&mut app, button::MINUS);
        assert_eq!(app.engine.settings().mode, Mode::Direct);
        assert!(fx.contains(&Effect::Light(REPEAT, LIGHT_OFF)));
        press(&mut app, button::PLUS);
        press(&mut app, button::PLUS);
        press(&mut app, button::PLUS);
        assert_eq!(app.engine.settings().mode, Mode::Pattern);
        // The wheel picks a pattern, the Q-Links rate, gate, swing and octaves.
        app.wheel(2);
        assert_eq!(app.pattern_index, 2);
        assert_eq!(*app.engine.pattern(), app.patterns[2].1);
        assert_eq!(app.qlink(1, 5, None), Some(vec![Effect::Status(app.status_now()), Effect::Light(REPEAT, LIGHT_ON)]));
        assert_eq!(app.engine.settings().gate, 60);
        assert_eq!(app.qlink(3, 3, None), Some(Vec::new()), "octaves need a longer turn");
        app.qlink(3, 1, None);
        assert_eq!(app.engine.settings().octaves, 2);
        app.qlink(0, 3, None);
        assert_eq!(app.settings.rate, Rate::SixteenthTriplet);
        app.qlink(2, 0, Some(127));
        assert_eq!(app.engine.settings().swing, 75, "absolute positions without Akai's session");
        // MENU: the options, and back to the panel.
        press(&mut app, MENU);
        assert_eq!(app.view(), View::Options);
        assert!(app.generating(), "the panel's options keep the arp on");
        press(&mut app, MENU);
        assert_eq!(app.view(), View::Arp);
        // The chip (or NOTE REPEAT + MENU) closes it: the pads play as pressed.
        app.toggle_panel();
        assert_eq!(app.view(), View::Play);
        assert!(!app.generating());
        assert_eq!(app.qlink(0, 1, None), None, "Q-Link 1 is the level again");
    }

    #[test]
    fn menu_on_the_arp_panel_sets_accent_scale_strum_and_restart() {
        use mpfree_arp::scale::{Scale, DORIAN, MINOR};
        let mut app = ready();
        app.toggle_panel();
        let fx = press(&mut app, MENU);
        let Some(Effect::Menu(m)) = fx.into_iter().find(|e| matches!(e, Effect::Menu(_))) else { panic!("no menu") };
        assert_eq!(m.title, "Arp options");
        assert_eq!(m.rows.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(),
                   ["Tempo", "Accent level", "Scale", "Scale root", "Strum", "New notes restart", "Swing from"]);
        assert_eq!(m.rows[6].1, "Transport");
        assert_eq!(m.rows[2].1, "Off", "no scale until one is picked: drum kits stay as they are");
        assert!(app.generating(), "the arp plays on under its options");
        let row = |app: &App, a: ArpItem| app.items().iter().position(|&i| i == Item::Arp(a)).unwrap();
        // Accent level steps by 8, down from 127.
        let fx = app.step_item(row(&app, ArpItem::Accent), -1);
        assert_eq!(app.engine.settings().accent, 119);
        assert!(fx.contains(&Effect::Note("arp accent level: 119".to_string())));
        // Scale: Off, then the named scales; the root moves it.
        app.step_item(row(&app, ArpItem::Scale), 1);
        app.step_item(row(&app, ArpItem::Scale), 1);
        assert_eq!(app.engine.settings().scale, Some(Scale::new(0, MINOR)));
        app.step_item(row(&app, ArpItem::Root), 1);
        let fx = app.step_item(row(&app, ArpItem::Root), 1);
        assert_eq!(app.engine.settings().scale, Some(Scale::new(2, MINOR)));
        assert!(fx.contains(&Effect::Note("arp scale: D Minor".to_string())));
        assert!(matches!(fx.last(), Some(Effect::Menu(m)) if m.rows[3].1 == "D" && m.rows[2].1 == "Minor"));
        app.step_item(row(&app, ArpItem::Scale), 1);
        app.step_item(row(&app, ArpItem::Scale), 1);
        assert_eq!(app.engine.settings().scale, Some(Scale::new(2, DORIAN)));
        // Off keeps the root for next time.
        for _ in 0..5 {
            app.step_item(row(&app, ArpItem::Scale), -1);
        }
        assert_eq!(app.engine.settings().scale, None);
        assert_eq!(app.step_item(row(&app, ArpItem::Scale), -1).iter().filter(|e| matches!(e, Effect::Note(_))).count(),
                   0, "nothing below Off");
        app.step_item(row(&app, ArpItem::Scale), 1);
        assert_eq!(app.engine.settings().scale, Some(Scale::new(2, mpfree_arp::scale::MAJOR)));
        // Strum in tens of percent; New notes restart on and off.
        app.step_item(row(&app, ArpItem::Strum), 1);
        assert_eq!(app.engine.settings().strum, 10);
        app.step_item(row(&app, ArpItem::Retrigger), 1);
        assert!(app.engine.settings().retrigger);
        // The panel's own settings leave them alone.
        app.arp_action(ArpAction::Settings(mpfree_arp::Settings { gate: 80, ..app.arp }));
        let s = app.engine.settings();
        assert_eq!((s.accent, s.strum, s.retrigger, s.gate), (119, 10, true, 80));
        // MENU goes back to the panel; the Play page's options are as before.
        press(&mut app, MENU);
        assert_eq!(app.view(), View::Arp);
        app.toggle_panel();
        press(&mut app, MENU);
        assert!(matches!(app.menu_view(), Effect::Menu(m) if m.title == "Play options" && m.rows.len() == 4));
    }

    #[test]
    fn the_arp_follows_the_transport_clock_and_its_swing_if_asked() {
        // NOTE REPEAT held on pad 1 at 1/16, 120 BPM: a 16th is 5512.5 frames.
        let gaps = |from_transport: bool, swing: Option<(u32, u8)>, synced: bool| {
            let mut app = ready();
            if !from_transport {
                app.toggle_panel();
                press(&mut app, MENU);
                let row = app.items().iter().position(|&i| i == Item::Arp(ArpItem::SwingFrom)).unwrap();
                let fx = app.step_item(row, -1);
                assert!(fx.contains(&Effect::Note("arp swing from: arp".to_string())));
                press(&mut app, MENU);
                app.toggle_panel();
            }
            app.button(REPEAT, true, 0.0);
            app.pad_down(0, 100);
            let mut ev = Events::new();
            let mut ons = Vec::new();
            for i in 0..400u64 {
                if synced {
                    let mut c = mpfree_arp::Clock::playing(0, i * 128, 120.0);
                    c.swing = swing;
                    app.set_clock(Some(c));
                }
                app.process(128, &mut ev);
                ons.extend(ev.as_slice().iter().filter(|e| matches!(e.kind, EventKind::On { .. }))
                    .map(|e| i * 128 + u64::from(e.offset)));
            }
            ons.windows(2).skip(1).take(4).map(|w| w[1] - w[0]).collect::<Vec<_>>()
        };
        let straight = gaps(true, None, true);
        assert!(straight.iter().all(|&g| g.abs_diff(5512) <= 1), "{straight:?}");
        // The transport swings 1/16s at 75 %: long, short, long, short.
        let swung = gaps(true, Some((240, 75)), true);
        assert!(swung.iter().all(|&g| g.abs_diff(8269) <= 1 || g.abs_diff(2756) <= 1), "{swung:?}");
        assert_ne!(swung[0], swung[1]);
        // Swing from: Arp ignores it; no clock at all free-runs.
        assert_eq!(gaps(false, Some((240, 75)), true), straight);
        let free = gaps(true, Some((240, 75)), false);
        assert!(free.iter().all(|&g| g.abs_diff(5512) <= 1), "{free:?}");
    }

    #[test]
    fn pads_arpeggiate_on_the_panel() {
        let mut app = ready();
        app.toggle_panel();
        app.arp_action(ArpAction::Mode(Mode::Arp));
        app.arp_action(ArpAction::Order(Order::Down));
        app.pad_down(0, 100);
        app.pad_down(2, 100);
        app.pad_up(0);
        app.pad_up(2);
        assert!(ons(&mut app, 20_000).is_empty(), "not latched: nothing after letting go");
        app.arp_action(ArpAction::Latch);
        app.pad_down(0, 100);
        app.pad_down(2, 100);
        app.pad_up(0);
        app.pad_up(2);
        let got = notes(&mut app, 4 * 5513 + 100);
        assert_eq!(&got[..4], &[2, 0, 2, 0], "Down over pads 1 and 3: {got:?}");
        // The screens hear of it once per change.
        assert!(matches!(app.arp_poll(), Some(Effect::Arp(v)) if v.latched && v.notes.len() == 2));
        assert_eq!(app.arp_poll(), None);
        let _ = ons(&mut app, 5513);
        assert!(app.arp_poll().is_some(), "a new step");
        press(&mut app, button::STOP);
        assert!(!app.latched);
    }

    #[test]
    fn the_workspace_records_on_the_pads() {
        let mut app = ready();
        app.toggle_panel();
        let fx = app.arp_action(ArpAction::Button(PatternButton::Record));
        assert_eq!(app.view(), View::Pattern);
        assert!(fx.iter().any(|e| matches!(e, Effect::Workspace(w) if w.open && !w.recording)));
        assert!(fx.iter().any(|e| matches!(e, Effect::Workspace(w) if !w.open && w.recording)));
        assert_eq!(app.engine.settings().mode, Mode::Direct, "pads sound as pressed while recording");
        let len = app.recorder.unwrap().length();
        assert_eq!(len, 8, "at least 8 steps, though Straight has 1");
        // Pad 1 writes "1", pad 16 a rest, pad 13 the next note.
        let fx = app.pad_down(0, 100);
        assert!(fx.iter().any(|e| matches!(e, Effect::Recorded(p, 1) if p.len() == len)));
        app.pad_up(0);
        assert_eq!(ons(&mut app, 128).len(), 1, "the pad still sounds");
        app.pad_down(15, 100);
        app.pad_up(15);
        let mut fx = Vec::new();
        for _ in 2..len {
            fx = app.pad_down(12, 120);
            app.pad_up(12);
        }
        assert!(fx.iter().any(|e| matches!(e, Effect::Workspace(w) if !w.recording)), "full: recording stops");
        assert_eq!(app.pattern.steps()[0], mpfree_arp::Step::play(mpfree_arp::Pick::Index(1)));
        assert_eq!(app.pattern.steps()[1], mpfree_arp::Step::Rest);
        assert!(matches!(app.pattern.steps()[2], mpfree_arp::Step::Play { accent: true, .. }), "a hard hit");
        assert_eq!(app.engine.settings().mode, Mode::Repeat, "the panel's mode again");
        // Save keeps it in the list (in memory); Back returns to the panel.
        let p = app.pattern;
        let fx = app.workspace_action(WsAction::Save(p));
        assert!(fx.iter().any(|e| matches!(e, Effect::Patterns(l) if l[0].1 == p)));
        app.workspace_action(WsAction::Back);
        assert_eq!(app.view(), View::Arp);
    }

    #[test]
    fn free_time_records_over_the_latched_notes_as_the_pattern_plays() {
        use mpfree_arp::{Pick, RecordMode, Step};
        let mut app = ready();
        app.toggle_panel();
        app.arp_action(ArpAction::Mode(Mode::Pattern));
        app.arp_action(ArpAction::Button(PatternButton::Edit));
        app.workspace_action(WsAction::Changed(Pattern::parse("1 . . .").unwrap()));
        // Free time with nothing latched: Step time, and it says why.
        let fx = app.workspace_action(WsAction::RecordMode(RecordMode::FreeTime));
        assert!(fx.iter().any(|e| matches!(e, Effect::Workspace(w) if w.free_time && !w.recording)));
        let fx = app.workspace_action(WsAction::Record(true));
        assert!(fx.iter().any(|e| matches!(e, Effect::Note(n) if n.starts_with("Free time plays the pattern"))));
        assert_eq!(app.recorder.unwrap().mode(), RecordMode::Step);
        app.workspace_action(WsAction::Record(false));
        app.workspace_action(WsAction::Changed(Pattern::parse("1 . . .").unwrap()));
        // Latch a chord (NOTE REPEAT twice), then record: the pattern plays it.
        app.button(REPEAT, true, 1.0);
        app.button(REPEAT, false, 1.1);
        app.button(REPEAT, true, 1.2);
        app.button(REPEAT, false, 1.3);
        app.pad_down(0, 100);
        app.pad_down(4, 100);
        app.pad_up(0);
        app.pad_up(4);
        let fx = app.workspace_action(WsAction::Record(true));
        assert!(fx.iter().any(|e| matches!(e, Effect::Workspace(w) if w.free_time && w.recording)));
        assert_eq!(app.recorder.unwrap().mode(), RecordMode::FreeTime);
        assert_eq!(app.engine.settings().mode, Mode::Pattern, "the pattern keeps playing");
        let _ = ons(&mut app, 3 * 5513 + 400);
        // Pad 2 (the 2nd held note), hit hard, lands on the step nearest now.
        let at = (app.engine.position().round() as usize) % 4;
        let fx = app.pad_down(1, 127);
        assert!(fx.iter().any(|e| matches!(e, Effect::Recorded(_, i) if *i == at)), "{fx:?}");
        assert!(matches!(app.pattern.steps()[at], Step::Play { pick: Pick::Index(2), accent: true, .. }));
        assert_eq!(*app.engine.pattern(), app.pattern, "the engine plays it at once");
        assert!(app.pad_up(1).is_empty(), "a short press ties nothing");
        let active: Vec<u8> = (0..16).filter(|&s| app.active(s)).map(|s| s as u8).collect();
        assert_eq!(active, [0, 4], "record pads don't change the latched chord");
        // Held across two more steps: ties after it.
        let at = (app.engine.position().round() as usize) % 4;
        app.pad_down(12, 90);
        let _ = ons(&mut app, 2 * 5513);
        let fx = app.pad_up(12);
        assert!(fx.iter().any(|e| matches!(e, Effect::Recorded(..))));
        assert_eq!(app.pattern.steps()[at], Step::play(Pick::Next));
        assert_eq!(app.pattern.steps()[(at + 1) % 4], Step::Tie);
        // Undo is for Step time; Clear rests every step.
        assert!(app.workspace_action(WsAction::Undo).is_empty());
        app.workspace_action(WsAction::Clear);
        assert_eq!(app.pattern.to_text(), ". . . .");
        // Back to Step time restarts the recording there.
        app.workspace_action(WsAction::RecordMode(RecordMode::Step));
        assert_eq!(app.recorder.unwrap().mode(), RecordMode::Step);
        assert_eq!(app.engine.settings().mode, Mode::Direct);
        for pad in [0, 1] {
            app.pad_down(pad, 100);
            app.pad_up(pad);
        }
        assert_eq!(app.recorder.unwrap().written(), 2);
        app.workspace_action(WsAction::Undo);
        assert_eq!(app.recorder.unwrap().written(), 1);
    }

    #[test]
    fn a_patterns_file_adds_and_replaces_and_save_writes_only_the_owners() {
        let mut app = ready();
        let fx = app.load_patterns("# mine\nSkip = 1 . 3 2\nstraight = x x .\n");
        assert!(fx.contains(&Effect::Note("arp patterns file: 1 added, 1 built-in ones replaced".into())), "{fx:?}");
        assert_eq!(app.patterns.last().unwrap().0, "Skip");
        assert_eq!(app.patterns[0], ("Straight".to_string(), Pattern::parse("x x .").unwrap()), "keeps its place");
        assert_eq!(*app.engine.pattern(), Pattern::parse("x x .").unwrap(), "the pattern in use is reloaded");
        let text = app.library_text();
        let lib = mpfree_arp::Library::parse(&text).unwrap();
        let names: Vec<&str> = lib.iter().map(|(n, _)| n).collect();
        assert_eq!(names, ["Straight", "Skip"], "only patterns that differ from the built-ins");
        // A bad file is not used at all.
        let before = app.patterns.clone();
        let fx = app.load_patterns("Fine = 1 2\nBad = 1 z\nNo equals sign\n");
        assert_eq!(app.patterns, before);
        assert!(fx.contains(&Effect::Note("arp patterns file not used (2 bad lines); the built-in patterns stay".into())));
        // Save logs a library line; it writes the file only with --save-patterns.
        app.toggle_panel();
        app.arp_action(ArpAction::Button(PatternButton::Edit));
        let p = Pattern::parse("1 2 . 3").unwrap();
        let fx = app.workspace_action(WsAction::Save(p));
        assert!(fx.contains(&Effect::Note("arp-pattern-saved: Straight = 1 2 . 3".into())));
        assert!(!fx.iter().any(|e| matches!(e, Effect::SavePatterns(_))));
        app.pattern_file = true;
        let fx = app.workspace_action(WsAction::Save(p));
        let saved = fx.iter().find_map(|e| match e { Effect::SavePatterns(t) => Some(t.clone()), _ => None }).unwrap();
        assert!(saved.contains("Straight = 1 2 . 3\n") && saved.contains("Skip     = 1 . 3 2\n"), "{saved}");
    }

    #[test]
    fn leaving_the_workspace_stops_recording_and_edits_reach_the_engine() {
        let mut app = ready();
        app.toggle_panel();
        app.arp_action(ArpAction::Button(PatternButton::Edit));
        let edited = Pattern::parse("1 . 2! _").unwrap();
        app.workspace_action(WsAction::Changed(edited));
        assert_eq!(*app.engine.pattern(), edited);
        assert!(matches!(app.arp_view(), Effect::Arp(v) if v.pattern_changed));
        app.workspace_action(WsAction::Record(true));
        assert!(app.recorder.is_some());
        let fx = app.tab(1);
        assert!(app.recorder.is_none());
        assert!(fx.iter().any(|e| matches!(e, Effect::Workspace(w) if !w.recording)));
        assert_eq!(app.workspace_action(WsAction::Back), Vec::new(), "only in the workspace");
        // Original goes back to the list's pattern.
        app.tab(0);
        app.toggle_panel();
        app.arp_action(ArpAction::Button(PatternButton::Original));
        assert!(matches!(app.arp_view(), Effect::Arp(v) if !v.pattern_changed));
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
