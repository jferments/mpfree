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
//!
//! The time engine: PLAY plays and STOP stops the transport (a second STOP
//! also silences everything, as before); REC, OVERDUB, UNDO (SHIFT + UNDO
//! redoes) and PLAY START are found with the control mapper (or the button
//! guide in Prefs). They become `Effect::Time`, which the audio loop hands to
//! its `TimeHost`.
//!
//! Debug mode (Prefs) adds the Debug tab: the MIDI log, the Performance page
//! and the control mapper (the `mapper` submodule). The PAD BANK buttons,
//! once found, pick banks A-H (`Effect::Bank`).

mod arp;
pub mod mapper;

pub use arp::{built_in_patterns, mode_label, ArpView, WorkspaceView};
pub use mapper::{Code, MapView, Mark};

use mpfree_arp::{Engine, Events, LatchMode, Mode, Pattern, Rate, Recorder};
use mpfree_controller::mpc_one::button;
use crate::looping::{LoopAction, LoopSource};
use mpfree_looper::Command as LoopCommand;
use mpfree_time_ui::TimeAction;
use mpfree_transport::Command;

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
    /// The time engine's launch grid, record settings and takes.
    Clips,
    /// The synth's page (mpfree-synth-ui): Poly's controls.
    Synth,
    /// The audio looper: the loop's waveform, its buttons and the line
    /// input's meter.
    Looper,
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
    /// Debug mode: process, thread and audio numbers (on the Debug tab).
    Perf,
    /// Debug mode: the control mapper (on the Debug tab).
    Map,
}

/// The header's tabs: the Debug tab only in debug mode.
pub fn tabs(debug: bool) -> Vec<View> {
    let mut t = vec![View::Play, View::Clips, View::Synth, View::Looper];
    if debug {
        t.push(View::Debug);
    }
    t.extend([View::Console, View::Prefs]);
    t
}

/// The bank a PAD BANK button picks (0 = A ... 7 = H): SHIFT picks E-H,
/// and pressing the button of the bank playing again swaps A and E (an
/// assumption from other MPCs, to try on the MPC One).
pub fn bank_target(current: usize, button: u8, shift: bool) -> usize {
    let b = button as usize & 3;
    if shift || current == b {
        b + 4
    } else {
        b
    }
}

/// A kit's bank letter (kits 1-8 are banks A-H, then again from kit 9).
pub fn bank_letter(kit: usize) -> char {
    (b'A' + (kit % 8) as u8) as char
}

/// The Debug tab's pages, in the order of its strip.
pub const DEBUG_PAGES: [View; 3] = [View::Debug, View::Perf, View::Map];

impl View {
    /// The tab a page belongs to (the Arp panel to Play, the debug pages to
    /// Debug), if any.
    pub fn tab_view(self) -> Option<View> {
        match self {
            View::Arp | View::Pattern => Some(View::Play),
            View::Perf | View::Map => Some(View::Debug),
            View::Options | View::Guide => None,
            v => Some(v),
        }
    }

    /// The tab's place in the header, which depends on debug mode.
    pub fn tab_index(self, debug: bool) -> Option<usize> {
        let v = self.tab_view()?;
        tabs(debug).iter().position(|&t| t == v)
    }

    pub fn name(self) -> &'static str {
        match self {
            View::Play => "Play",
            View::Clips => "Clips",
            View::Synth => "Synth",
            View::Looper => "Looper",
            View::Debug => "Debug",
            View::Console => "Console",
            View::Prefs => "Prefs",
            View::Options => "Options",
            View::Guide => "Guide",
            View::Arp => "Arp",
            View::Pattern => "Pattern",
            View::Perf => "Performance",
            View::Map => "Map controls",
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
    /// PAD BANK A/E to D/H, found with the control mapper.
    pub bank: [Option<u8>; 4],
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
        if let Some(k) = self.bank_of(note) {
            return Some(["PAD BANK A/E", "PAD BANK B/F", "PAD BANK C/G", "PAD BANK D/H"][k as usize]);
        }
        GuideStep::ALL.iter().find(|&&s| self.get(s) == Some(note)).map(|s| s.short())
    }

    /// Which PAD BANK button a note is (0 = A/E), once found.
    pub fn bank_of(&self, note: u8) -> Option<u8> {
        self.bank.iter().position(|&b| b == Some(note)).map(|k| k as u8)
    }

    /// The number a control with a job has now (the mapper starts from it).
    pub fn of(&self, job: mapper::Job) -> Option<u8> {
        use mapper::Job;
        match job {
            Job::None => None,
            Job::Fixed(n) => Some(n),
            Job::Menu => self.menu,
            Job::NoteRepeat => self.note_repeat,
            Job::TapTempo => self.tap_tempo,
            Job::Minus => self.minus,
            Job::Plus => self.plus,
            Job::Rec => self.rec,
            Job::Overdub => self.overdub,
            Job::Undo => self.undo,
            Job::PlayStart => self.play_start,
            Job::Bank(k) => self.bank[k as usize & 3],
        }
    }

    /// A control with a job found (or cleared) with the mapper.
    pub fn learn(&mut self, job: mapper::Job, code: Option<u8>) {
        use mapper::Job;
        let slot = match job {
            Job::None | Job::Fixed(_) => return,
            Job::Menu => &mut self.menu,
            Job::NoteRepeat => &mut self.note_repeat,
            Job::TapTempo => &mut self.tap_tempo,
            Job::Minus => &mut self.minus,
            Job::Plus => &mut self.plus,
            Job::Rec => &mut self.rec,
            Job::Overdub => &mut self.overdub,
            Job::Undo => &mut self.undo,
            Job::PlayStart => &mut self.play_start,
            Job::Bank(k) => &mut self.bank[k as usize & 3],
        };
        *slot = code;
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
    /// Debug mode: the Debug tab (MIDI log, Performance, Map controls) and
    /// the `perf:` lines in the log.
    pub debug_mode: bool,
    /// The line input (the MPC's capture device) is open. Always off at
    /// start; nothing saves it.
    pub line_input: bool,
    /// The line input is heard through the outputs.
    pub loop_monitor: bool,
    /// What the looper records: the pads (the default) or the line input.
    pub loop_source: LoopSource,
    /// Bars in the looper's next first pass (None: until Record again).
    pub loop_bars: Option<u32>,
    /// Input latency the looper makes up for, in frames; estimated from
    /// the audio buffer until set by hand.
    pub loop_latency: u32,
    pub loop_latency_set: bool,
    pub loop_level_db: f32,
}

impl Settings {
    /// The latency the looper makes up for: none for the pads, which are
    /// in time with the transport.
    pub fn loop_latency_now(&self) -> u32 {
        match self.loop_source {
            LoopSource::Pads => 0,
            LoopSource::Line => self.loop_latency,
        }
    }
}

impl Default for Settings {
    fn default() -> Settings {
        Settings { curve: Curve::Linear, screen_velocity: 100, rate: Rate::Sixteenth, bpm: 120.0,
                   latch_mode: LatchMode::Replace, periods: 3, show_pressure: false, midi_paused: false,
                   console_paused: false, debug_mode: false, line_input: false, loop_monitor: false,
                   loop_source: LoopSource::Pads, loop_bars: Some(2), loop_latency: crate::looping::estimated_latency(3), loop_latency_set: false,
                   loop_level_db: 0.0 }
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
    DebugMode,
    MapControls,
    Rate,
    Tempo,
    PadLevel,
    Latch,
    ShowPressure,
    MidiLog,
    ClearLogs,
    ConsoleFollow,
    ClearConsole,
    LineInput,
    LoopSource,
    LoopMonitor,
    LoopBars,
    LoopLatency,
    LoopLevel,
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
    /// SHIFT + wheel on Play: move the synth's pads by octaves (the kit, as
    /// before, when a drum kit is playing).
    Octave(i32),
    /// The wheel on the Synth page: turn the selected knob.
    SynthWheel(i32),
    /// SHIFT + wheel, - or + on the Synth page: the next or previous page.
    SynthPage(i32),
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
    /// The data wheel on the Clips page: step the selected record setting,
    /// or (with SHIFT) scroll the grid.
    ClipsWheel { delta: i32, scroll: bool },
    /// STOP in the timed `play` step: end it.
    Quit,
    /// Open the Touch lab page (it starts at its first step).
    TouchLab,
    /// For the time engine (transport, clips, takes).
    Time(TimeAction),
    /// What the Map controls page shows.
    Map(MapView),
    /// A PAD BANK button pressed: 0 = A/E to 3 = D/H, and whether SHIFT
    /// was held (E-H). The audio loop knows the bank playing, so it picks.
    Bank { button: u8, shift: bool },
    /// For the looper and the line input.
    Looper(LoopAction),
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
    /// The patterns to pick from (built in; Save changes them in memory).
    patterns: Vec<(String, Pattern)>,
    pattern_index: usize,
    /// The pattern the engine plays (edited, mutated or recorded from the
    /// list's).
    pattern: Pattern,
    mutations: u32,
    /// Recording a pattern on the pads (Step mode) in the workspace.
    recorder: Option<Recorder>,
    /// Q-Link turns not yet used, per knob (rate and octaves need several).
    qlink_turns: [i32; 4],
    /// What the arp screens were last sent: the step and the notes, and
    /// whether anything else changed since.
    sent_step: u64,
    sent_notes: mpfree_arp::NoteSet,
    arp_dirty: bool,
    /// The transport is rolling (the audio loop says each period).
    rolling: bool,
    /// The control mapper (Debug mode).
    mapper: mapper::Mapper,
    /// Buttons pressed while the mapper listened: their release is kept
    /// from the rest of the app too.
    swallowed: [bool; 128],
    /// The Looper tab's Mute.
    loop_muted: bool,
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
                            tap_light: false, arp_mode: Mode::Repeat, arp: mpfree_arp::Settings::default(), patterns,
                            pattern_index: 0, pattern, mutations: 0, recorder: None, qlink_turns: [0; 4],
                            sent_step: 0, sent_notes, arp_dirty: true, rolling: false,
                            mapper: mapper::Mapper::default(), swallowed: [false; 128], loop_muted: false };
        app.engine.set_pattern(pattern);
        app.update_engine();
        app
    }

    pub fn view(&self) -> View {
        self.view
    }

    /// Whether the transport is rolling, for STOP (the audio loop tells it
    /// each period).
    pub fn set_rolling(&mut self, rolling: bool) {
        self.rolling = rolling;
    }

    /// The first page once everything is ready, and the lights to start with.
    pub fn start(&mut self) -> Vec<Effect> {
        let mut fx = vec![self.status(), Effect::Settings(self.settings.clone()),
                          Effect::Light(button::FULL_LEVEL, LIGHT_OFF), Effect::Patterns(self.patterns.clone()),
                          self.arp_view()];
        fx.extend(self.repeat_light());
        let s = &self.settings;
        fx.push(Effect::Looper(LoopAction::Source(s.loop_source)));
        fx.push(Effect::Looper(LoopAction::Command(LoopCommand::SetBars(s.loop_bars))));
        fx.push(Effect::Looper(LoopAction::Command(LoopCommand::SetLatency(s.loop_latency_now()))));
        // Buttons not known yet are found with the control mapper (Debug
        // mode), at the owner's pace: nothing asks for them at power-on.
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
        self.engine.press(slot as u8, velocity.max(1));
        self.record_press(slot as u8, velocity)
    }

    pub fn pad_up(&mut self, slot: usize) {
        self.engine.release(slot as u8);
        if let Some(r) = self.recorder.as_mut() {
            r.release(slot as u8, 0.0);
        }
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
        // The mapper listening: every button is for it, release included.
        let n = note as usize & 0x7F;
        if !pressed && self.swallowed[n] {
            self.swallowed[n] = false;
            return fx;
        }
        if pressed && self.view == View::Map {
            let listening = self.mapper.listening;
            fx.extend(self.map_input(Code::Button(note)));
            if listening {
                self.swallowed[n] = true;
                return fx;
            }
        }
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
            } else if self.rolling {
                // The transport stops; sounds ring out and latched pads keep
                // going, so a second STOP is what silences everything.
                fx.push(Effect::Time(TimeAction::Transport(Command::Stop)));
                fx.push(Effect::Note("STOP: transport stopped".to_string()));
            } else {
                fx.extend(self.stop());
                // Stopped already: the transport goes back to bar 1.
                fx.push(Effect::Time(TimeAction::Transport(Command::Stop)));
            }
        } else if note == button::PLAY {
            fx.push(Effect::Time(TimeAction::Transport(Command::Play)));
        } else if Some(note) == self.map.play_start {
            fx.push(Effect::Time(TimeAction::Transport(Command::PlayStart)));
        } else if self.view == View::Looper && Some(note) == self.map.rec {
            fx.extend(self.looper_tap(crate::looper_page::B_RECORD));
        } else if self.view == View::Looper && Some(note) == self.map.overdub {
            fx.extend(self.looper_tap(crate::looper_page::B_OVERDUB));
        } else if self.view == View::Looper && Some(note) == self.map.undo {
            fx.extend(self.looper_tap(if shift { crate::looper_page::B_REDO } else { crate::looper_page::B_UNDO }));
        } else if Some(note) == self.map.rec {
            fx.push(Effect::Time(TimeAction::Transport(Command::Record)));
        } else if Some(note) == self.map.overdub {
            fx.push(Effect::Time(TimeAction::Transport(Command::ToggleOverdub)));
        } else if Some(note) == self.map.undo {
            fx.push(Effect::Time(if shift { TimeAction::Redo } else { TimeAction::Undo }));
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
        } else if let Some(k) = self.map.bank_of(note) {
            fx.push(Effect::Bank { button: k, shift });
        } else if Some(note) == self.map.minus || Some(note) == self.map.plus {
            let d = if Some(note) == self.map.plus { 1 } else { -1 };
            if self.in_menu() {
                fx.extend(self.step_item(self.selected, d));
            } else if self.view == View::Map && !shift {
                // + listens for the selected control, - skips it.
                fx.extend(if d > 0 { self.map_listen() } else { self.map_skip() });
            } else if matches!(self.view, View::Debug | View::Perf | View::Map) {
                fx.extend(self.debug_step(d));
            } else if self.repeat_held || self.view == View::Arp {
                fx.extend(self.arp_mode_step(d));
            } else if self.view == View::Synth {
                fx.push(Effect::SynthPage(d));
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

    /// The tempo, set from elsewhere (the Clips page): Note Repeat, the arp
    /// and the transport follow it.
    pub fn set_tempo(&mut self, bpm: f64) -> Vec<Effect> {
        self.settings.bpm = bpm.clamp(30.0, 300.0);
        self.update_engine();
        let mut fx = vec![self.status(), Effect::Note(format!("tempo {:.1} BPM", self.settings.bpm))];
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
            // On the Debug tab SHIFT + wheel picks the page; on Map controls
            // the wheel picks the control.
            View::Debug | View::Perf | View::Map if self.shift => self.debug_step(delta.signum()),
            View::Map => {
                self.mapper.move_by(delta);
                vec![Effect::Map(self.mapper.view())]
            }
            // NOTE REPEAT held or the Arp panel: the rate, order or pattern.
            _ if self.repeat_held || self.view == View::Arp => self.arp_wheel(delta),
            View::Clips => vec![Effect::ClipsWheel { delta, scroll: self.shift }],
            View::Synth if self.shift => vec![Effect::SynthPage(delta.signum())],
            View::Synth => vec![Effect::SynthWheel(delta)],
            View::Play if self.shift => vec![Effect::Octave(delta.signum())],
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
        if self.view != v
            && matches!(self.view, View::Play | View::Clips | View::Synth | View::Looper | View::Debug | View::Console
                                   | View::Arp | View::Perf | View::Map)
        {
            self.back = self.view;
        }
        let mut fx = Vec::new();
        if self.view == View::Map && v != View::Map {
            self.mapper.listening = false;
            if self.mapper.changed {
                self.mapper.changed = false;
                fx.push(Effect::Note(self.mapper.summary()));
            }
        }
        if v == View::Map && self.view != View::Map {
            self.refresh_mapper();
            fx.push(Effect::Map(self.mapper.view()));
        }
        self.view = v;
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

    /// A tab touched on the screen (its place in the header).
    pub fn tab(&mut self, i: usize) -> Vec<Effect> {
        match tabs(self.settings.debug_mode).get(i) {
            Some(&v) => self.open_tab(v),
            None => Vec::new(),
        }
    }

    /// Open a tab's page.
    pub fn open_tab(&mut self, v: View) -> Vec<Effect> {
        if self.view == View::Guide {
            return Vec::new();
        }
        self.show(v)
    }

    // ------------------------------------------------------------ debug mode

    /// A tap on the Debug tab's strip: MIDI, Performance or Map controls.
    pub fn debug_page(&mut self, i: usize) -> Vec<Effect> {
        match DEBUG_PAGES.get(i) {
            Some(&v) if self.settings.debug_mode => self.show(v),
            _ => Vec::new(),
        }
    }

    /// The next or previous page of the Debug tab.
    fn debug_step(&mut self, d: i32) -> Vec<Effect> {
        let at = DEBUG_PAGES.iter().position(|&v| v == self.view).unwrap_or(0) as i32;
        let to = (at + d).rem_euclid(DEBUG_PAGES.len() as i32) as usize;
        self.show(DEBUG_PAGES[to])
    }

    fn set_debug_mode(&mut self, on: bool) -> Vec<Effect> {
        if self.settings.debug_mode == on {
            return Vec::new();
        }
        self.settings.debug_mode = on;
        if !on && matches!(self.back, View::Debug | View::Perf | View::Map) {
            self.back = View::Play;
        }
        vec![Effect::Note(format!("debug mode: {}", if on { "on" } else { "off" }))]
    }

    /// The mapper's list starts from what the app knows (measured buttons,
    /// and those found since).
    fn refresh_mapper(&mut self) {
        for (i, c) in mapper::CONTROLS.iter().enumerate() {
            self.mapper.know(i, self.map.of(c.job));
        }
    }

    /// A control pressed or turned on the Map controls page.
    fn map_input(&mut self, code: Code) -> Vec<Effect> {
        let mut fx = Vec::new();
        if let Some((line, learned)) = self.mapper.input(code) {
            fx.push(Effect::Note(line));
            if let Some(l) = learned {
                fx.extend(self.learn(l));
            }
        }
        fx.push(Effect::Map(self.mapper.view()));
        fx
    }

    fn learn(&mut self, l: mapper::Learned) -> Vec<Effect> {
        self.map.learn(l.job, l.code);
        let mut fx = Vec::new();
        if l.job == mapper::Job::NoteRepeat {
            fx.extend(self.repeat_light());
        }
        if matches!(l.job, mapper::Job::Bank(_)) {
            // The audio loop lights the bank playing.
            fx.push(Effect::Bank { button: u8::MAX, shift: false });
        }
        fx
    }

    /// A CC the decoder does not know (not the wheel or a Q-Link), from the
    /// audio loop. Only the mapper uses it.
    pub fn unknown_cc(&mut self, cc: u8) -> Vec<Effect> {
        if self.view == View::Map {
            self.map_input(Code::Cc(cc))
        } else {
            Vec::new()
        }
    }

    /// A measured control that is not a button (a Q-Link turned or touched),
    /// on the Map controls page: named in the card.
    pub fn known_control(&mut self, code: Code) -> Vec<Effect> {
        if self.view == View::Map {
            self.map_input(code)
        } else {
            Vec::new()
        }
    }

    /// A control tapped in the list: listen for it.
    pub fn map_row(&mut self, i: usize) -> Vec<Effect> {
        if self.view != View::Map {
            return Vec::new();
        }
        self.mapper.select(i, true);
        vec![Effect::Map(self.mapper.view())]
    }

    fn map_listen(&mut self) -> Vec<Effect> {
        let i = self.mapper.selected;
        self.mapper.select(i, true);
        vec![Effect::Map(self.mapper.view())]
    }

    fn map_skip(&mut self) -> Vec<Effect> {
        let line = self.mapper.skip();
        vec![Effect::Note(line), Effect::Map(self.mapper.view())]
    }

    /// The card's buttons: 0 Listen or Stop listening, 1 Skip, 2 Clear.
    pub fn map_button(&mut self, i: usize) -> Vec<Effect> {
        if self.view != View::Map {
            return Vec::new();
        }
        match i {
            0 if self.mapper.listening => {
                self.mapper.stop_listening();
                vec![Effect::Map(self.mapper.view())]
            }
            0 => self.map_listen(),
            1 => self.map_skip(),
            2 => {
                let mut fx = Vec::new();
                if let Some((line, learned)) = self.mapper.clear() {
                    fx.push(Effect::Note(line));
                    fx.extend(self.learn(learned));
                }
                fx.push(Effect::Map(self.mapper.view()));
                fx
            }
            _ => Vec::new(),
        }
    }

    // ------------------------------------------------------------ looper

    /// A button on the Looper tab (or REC, OVERDUB, UNDO there).
    pub fn looper_tap(&mut self, i: usize) -> Vec<Effect> {
        use crate::looper_page::*;
        let c = |c: LoopCommand| Effect::Looper(LoopAction::Command(c));
        match i {
            B_RECORD => vec![c(LoopCommand::Record)],
            B_OVERDUB => vec![c(LoopCommand::Overdub)],
            B_UNDO => vec![c(LoopCommand::Undo)],
            B_REDO => vec![c(LoopCommand::Redo)],
            B_CLEAR => vec![c(LoopCommand::Clear)],
            B_MUTE => {
                self.loop_muted = !self.loop_muted;
                vec![c(LoopCommand::Mute(self.loop_muted))]
            }
            B_MONITOR if self.settings.line_input => {
                self.settings.loop_monitor = !self.settings.loop_monitor;
                let mut fx = vec![Effect::Looper(LoopAction::Monitor(self.settings.loop_monitor)),
                                  Effect::Settings(self.settings.clone())];
                if self.in_menu() {
                    fx.push(self.menu_view());
                }
                fx
            }
            _ => Vec::new(),
        }
    }

    /// The line input could not open (or closed itself): the row and the
    /// Looper tab say Off again, and the looper records the pads.
    pub fn line_input_off(&mut self) -> Vec<Effect> {
        self.settings.line_input = false;
        self.settings.loop_monitor = false;
        let mut fx = vec![Effect::Looper(LoopAction::Monitor(false))];
        fx.extend(self.set_loop_source(LoopSource::Pads));
        fx.push(Effect::Settings(self.settings.clone()));
        if self.in_menu() {
            fx.push(self.menu_view());
        }
        fx
    }

    /// Record from `source`, with the latency that goes with it.
    fn set_loop_source(&mut self, source: LoopSource) -> Vec<Effect> {
        if source == self.settings.loop_source {
            return Vec::new();
        }
        self.settings.loop_source = source;
        vec![Effect::Looper(LoopAction::Source(source)),
             Effect::Looper(LoopAction::Command(LoopCommand::SetLatency(self.settings.loop_latency_now())))]
    }

    // ------------------------------------------------------------ menus

    fn items(&self) -> &'static [Item] {
        match (self.view, self.back) {
            // Line input goes after the rows the runbook names by place; the
            // Touch lab stays last.
            (View::Prefs, _) => &[Item::Curve, Item::ScreenVelocity, Item::LatchMode, Item::Buffer,
                               Item::FindButtons, Item::DebugMode, Item::MapControls, Item::LineInput,
                               Item::TouchLab],
            (_, View::Looper) => &[Item::LoopSource, Item::LineInput, Item::LoopMonitor, Item::LoopBars, Item::LoopLatency,
                                   Item::LoopLevel],
            (_, View::Debug | View::Perf | View::Map) => &[Item::ShowPressure, Item::MidiLog, Item::ClearLogs],
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
            Item::DebugMode => ("Debug mode", if s.debug_mode { "On" } else { "Off" }.to_string()),
            Item::MapControls => ("Map controls", "Open".to_string()),
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
            Item::LineInput => ("Line input", if s.line_input { "On" } else { "Off" }.to_string()),
            Item::LoopSource => ("Record from", s.loop_source.name().to_string()),
            Item::LoopMonitor => ("Monitor the input", if s.loop_monitor { "On" } else { "Off" }.to_string()),
            Item::LoopBars => ("First pass length", crate::looping::bars_text(s.loop_bars)),
            Item::LoopLatency => ("Latency to make up for", format!("{} frames, {:.1} ms{}", s.loop_latency,
                                                                     s.loop_latency as f64 / 44.1,
                                                                     if s.loop_latency_set { "" } else { " (estimated)" })),
            Item::LoopLevel => ("Loop level", format!("{:+.0} dB", s.loop_level_db)),
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
            Item::DebugMode => vec!["Adds the Debug tab: the MIDI log, the",
                                    "Performance page (CPU per thread, memory,",
                                    "audio render time) and Map controls. The",
                                    "numbers also go to the log every 30 s.",
                                    "On the Debug tab, SHIFT + wheel picks the page."],
            Item::MapControls => vec!["Find the numbers of the MPC's buttons, one at",
                                      "a time, at your pace: pick a control, press it,",
                                      "and see which ones are left. Turns on debug",
                                      "mode. Wheel: pick, +: listen, -: skip."],
            Item::Rate => vec!["How often a held pad repeats with NOTE REPEAT",
                               "or Latch: from quarter notes to 32nd notes,",
                               "T for triplets."],
            Item::Tempo => vec!["The tempo the transport, the clips and",
                                "Note Repeat follow. TAP TEMPO sets it too:",
                                "tap it three or four times."],
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
            Item::LineInput => vec!["Opens the MPC's line input for the looper.",
                                    "Off at every start. Which jacks it reads is",
                                    "not checked yet (line in L and R, most",
                                    "likely). Turning it off closes the input."],
            Item::LoopSource => vec!["What the looper records: the pads and the",
                                     "synth (before the click), or the line input.",
                                     "Turning Line input on picks it; recording",
                                     "the pads opens nothing."],
            Item::LoopMonitor => vec!["Hear the line input through the outputs,",
                                      "about 9 ms late. Leave it off if you hear",
                                      "the instrument another way."],
            Item::LoopBars => vec!["How long the looper's first pass is: 1, 2,",
                                   "4 or 8 bars, or free: from the next bar line",
                                   "until the bar line after Record again."],
            Item::LoopLatency => vec!["How late your playing reaches the looper:",
                                      "the output buffer, the input buffer and the",
                                      "converters. Estimated from the audio buffer",
                                      "until you set it. Measure it with a cable",
                                      "from an output to the input."],
            Item::LoopLevel => vec!["The loop's playback level, -24 to +6 dB.",
                                    "0 dB plays it as it was recorded."],
        }
    }

    pub fn menu_view(&self) -> Effect {
        let items = self.items();
        let selected = self.selected.min(items.len() - 1);
        let title = match (self.view, self.back) {
            (View::Prefs, _) => "Prefs".to_string(),
            (_, b) => format!("{} options", b.tab_view().unwrap_or(View::Play).name()),
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
                    if !s.loop_latency_set {
                        s.loop_latency = crate::looping::estimated_latency(p);
                        fx.push(Effect::Looper(LoopAction::Command(LoopCommand::SetLatency(s.loop_latency_now()))));
                    }
                }
            }
            Item::FindButtons => {
                self.open_guide(GuideStep::ALL.to_vec());
                return vec![self.guide_view(), Effect::Show(View::Guide)];
            }
            Item::TouchLab => return vec![Effect::TouchLab],
            Item::DebugMode => fx.extend(self.set_debug_mode(d > 0)),
            Item::MapControls => {
                let mut fx = self.set_debug_mode(true);
                fx.push(Effect::Settings(self.settings.clone()));
                fx.extend(self.show(View::Map));
                return fx;
            }
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
            Item::LineInput => {
                let on = d > 0;
                if on != s.line_input {
                    s.line_input = on;
                    fx.push(Effect::Looper(LoopAction::LineInput(on)));
                    if !on && s.loop_monitor {
                        s.loop_monitor = false;
                        fx.push(Effect::Looper(LoopAction::Monitor(false)));
                    }
                    // The looper follows: the line input while it is on.
                    fx.extend(self.set_loop_source(if on { LoopSource::Line } else { LoopSource::Pads }));
                }
            }
            Item::LoopSource => {
                let source = cycle(&LoopSource::ALL, s.loop_source, d);
                fx.extend(self.set_loop_source(source));
            }
            Item::LoopMonitor => {
                let on = d > 0 && s.line_input;
                if on != s.loop_monitor {
                    s.loop_monitor = on;
                    fx.push(Effect::Looper(LoopAction::Monitor(on)));
                }
            }
            Item::LoopBars => {
                const BARS: [Option<u32>; 5] = [Some(1), Some(2), Some(4), Some(8), None];
                s.loop_bars = cycle(&BARS, s.loop_bars, d);
                fx.push(Effect::Looper(LoopAction::Command(LoopCommand::SetBars(s.loop_bars))));
            }
            Item::LoopLatency => {
                s.loop_latency = (s.loop_latency as i32 + 16 * d).clamp(0, 4096) as u32;
                s.loop_latency_set = true;
                fx.push(Effect::Looper(LoopAction::Command(LoopCommand::SetLatency(s.loop_latency_now()))));
            }
            Item::LoopLevel => {
                s.loop_level_db = (s.loop_level_db + d as f32).clamp(-24.0, 6.0);
                fx.push(Effect::Looper(LoopAction::Command(LoopCommand::SetLevel(10f32.powf(s.loop_level_db / 20.0)))));
            }
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
    fn the_synth_tab_takes_the_wheel_and_shift_wheel_moves_octaves_on_play() {
        let mut app = ready();
        // SHIFT + wheel on Play: octaves (main.rs moves the kit when drums play).
        app.button(button::SHIFT, true, 0.0);
        assert_eq!(app.wheel(3), vec![Effect::Octave(1)]);
        app.button(button::SHIFT, false, 0.0);
        assert_eq!(app.wheel(-2), vec![Effect::Kit(-2)]);
        assert_eq!(app.open_tab(View::Synth), vec![Effect::Show(View::Synth)]);
        assert_eq!(app.view(), View::Synth);
        assert_eq!(app.wheel(2), vec![Effect::SynthWheel(2)]);
        app.button(button::SHIFT, true, 0.0);
        assert_eq!(app.wheel(-4), vec![Effect::SynthPage(-1)]);
        app.button(button::SHIFT, false, 0.0);
        // - and + step pages once the guide knows them.
        app.map.minus = Some(40);
        app.map.plus = Some(41);
        assert_eq!(press(&mut app, 41), vec![Effect::SynthPage(1)]);
        assert_eq!(press(&mut app, 40), vec![Effect::SynthPage(-1)]);
        // MENU on the Synth page opens its options and goes back to it.
        let fx = press(&mut app, MENU);
        assert!(matches!(&fx[0], Effect::Menu(m) if m.title == "Synth options"));
        press(&mut app, MENU);
        assert_eq!(app.view(), View::Synth);
        assert_eq!(app.tab(0), vec![Effect::Show(View::Play)]);
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
        app.open_tab(View::Debug);
        press(&mut app, MENU);
        assert!(matches!(app.menu_view(), Effect::Menu(m) if m.title == "Debug options"));
        // The wheel picks kits outside menus.
        press(&mut app, MENU);
        assert_eq!(app.wheel(-2), vec![Effect::Kit(-2)]);
    }

    #[test]
    fn at_power_on_nothing_asks_and_the_mapper_finds_buttons_at_the_owners_pace() {
        let mut app = App::new(44_100, false);
        assert_eq!(app.map.menu, Some(123));
        assert_eq!((app.map.note_repeat, app.map.tap_tempo, app.map.minus, app.map.plus), (Some(11), Some(53), Some(55), Some(54)));
        let fx = app.start();
        assert!(!fx.iter().any(|e| matches!(e, Effect::Guide(_))));
        assert_eq!(app.view(), View::Play);
        // No Debug tab until debug mode: tab 4 (after Looper) is Console.
        assert_eq!(tabs(false)[4], View::Console);
        assert_eq!(app.tab(4), vec![Effect::Show(View::Console)]);
        app.open_tab(View::Prefs);
        let row = app.items().iter().position(|&i| i == Item::MapControls).unwrap();
        let fx = app.step_item(row, 1);
        assert!(app.settings.debug_mode);
        assert_eq!(app.view(), View::Map);
        assert!(fx.contains(&Effect::Note("debug mode: on".into())));
        assert!(fx.iter().any(|e| matches!(e, Effect::Map(m) if m.rows[2].value == "button 81" && m.left > 20)));
        // Tap REC: MPFree listens; STOP stops listening without silencing.
        app.map_row(0);
        let fx = press(&mut app, button::STOP);
        assert!(!fx.contains(&Effect::StopSounds), "{fx:?}");
        assert!(fx.iter().any(|e| matches!(e, Effect::Map(m) if !m.listening)));
        app.map_row(0);
        let fx = press(&mut app, 73);
        assert!(fx.contains(&Effect::Note("map: REC = button 73".into())));
        assert_eq!(app.map.rec, Some(73));
        // The wheel picks PAD BANK A/E, + listens, and it works at once.
        let at = mapper::CONTROLS.iter().position(|c| c.name == "PAD BANK A/E").unwrap() as i32;
        app.wheel(at - app.mapper.selected as i32);
        press(&mut app, button::PLUS);
        let fx = press(&mut app, 35);
        assert!(fx.contains(&Effect::Bank { button: u8::MAX, shift: false }), "{fx:?}");
        // - skips the next one; leaving the page writes the summary.
        let fx = press(&mut app, button::MINUS);
        assert!(fx.iter().any(|e| matches!(e, Effect::Note(n) if n.ends_with("skipped"))));
        let fx = app.open_tab(View::Play);
        assert!(fx.iter().any(|e| matches!(e, Effect::Note(n) if n.starts_with("map summary: ")
                                             && n.contains("REC button 73") && n.contains("PAD BANK A/E button 35"))));
        assert_eq!(press(&mut app, 73), vec![Effect::Time(TimeAction::Transport(Command::Record))]);
        assert_eq!(press(&mut app, 35), vec![Effect::Bank { button: 0, shift: false }]);
        app.button(button::SHIFT, true, 0.0);
        assert_eq!(press(&mut app, 35), vec![Effect::Bank { button: 0, shift: true }]);
        app.button(button::SHIFT, false, 0.0);
        // Debug mode adds the Debug tab, whose pages SHIFT + wheel steps through.
        assert_eq!(tabs(true)[4], View::Debug);
        app.tab(4);
        assert_eq!(app.view(), View::Debug);
        app.button(button::SHIFT, true, 0.0);
        app.wheel(1);
        assert_eq!(app.view(), View::Perf);
        app.wheel(1);
        assert_eq!(app.view(), View::Map);
        app.button(button::SHIFT, false, 0.0);
        assert_eq!(app.debug_page(1), vec![Effect::Show(View::Perf)]);
    }

    /// The looper records the pads until the line input is turned on, and
    /// goes back to them when it is turned off or fails to open.
    #[test]
    fn the_line_input_is_off_at_start_and_the_looper_follows_it() {
        let mut app = ready();
        let source = |s| Effect::Looper(LoopAction::Source(s));
        let latency = |l| Effect::Looper(LoopAction::Command(LoopCommand::SetLatency(l)));
        assert!(!app.settings.line_input && app.settings.loop_source == LoopSource::Pads);
        let fx = App::new(44_100, false).start();
        assert!(fx.contains(&source(LoopSource::Pads)) && fx.contains(&latency(0)), "{fx:?}");
        assert!(!fx.iter().any(|e| matches!(e, Effect::Looper(LoopAction::LineInput(_)))), "nothing opens at start");
        // The Looper tab's options: Record from, then Line input.
        app.open_tab(View::Looper);
        press(&mut app, MENU);
        assert_eq!(app.view(), View::Options);
        assert_eq!(app.items()[..2], [Item::LoopSource, Item::LineInput]);
        let fx = app.step_item(1, 1);
        assert!(fx.contains(&Effect::Looper(LoopAction::LineInput(true))));
        assert!(fx.contains(&source(LoopSource::Line)) && fx.contains(&latency(512)), "{fx:?}");
        // REC on the Looper tab is the looper's Record.
        press(&mut app, MENU);
        assert_eq!(app.view(), View::Looper);
        assert_eq!(press(&mut app, 90), vec![Effect::Looper(LoopAction::Command(LoopCommand::Record))]);
        // The input fails to open: Off again, and the pads.
        let fx = app.line_input_off();
        assert!(fx.contains(&source(LoopSource::Pads)) && fx.contains(&latency(0)), "{fx:?}");
        assert!(!app.settings.line_input);
        // Record from can pick the line input by hand; Pads needs no latency.
        press(&mut app, MENU);
        let fx = app.step_item(0, 1);
        assert!(fx.contains(&source(LoopSource::Line)) && fx.contains(&latency(512)), "{fx:?}");
        let fx = app.step_item(0, -1);
        assert!(fx.contains(&source(LoopSource::Pads)) && fx.contains(&latency(0)), "{fx:?}");
        assert!(app.step_item(0, -1).iter().all(|e| !matches!(e, Effect::Looper(_))), "already the pads");
    }

    #[test]
    fn pad_banks_pick_e_to_h_with_shift_or_a_second_press() {
        assert_eq!(bank_target(0, 1, false), 1);
        assert_eq!(bank_target(1, 1, false), 5, "B again: F");
        assert_eq!(bank_target(5, 1, false), 1, "F, then B: B");
        assert_eq!(bank_target(5, 0, false), 0);
        assert_eq!(bank_target(2, 3, true), 7, "SHIFT + D/H: H");
        assert_eq!((bank_letter(0), bank_letter(7), bank_letter(9)), ('A', 'H', 'B'));
    }

    #[test]
    fn the_guide_learns_buttons_by_name() {
        let mut app = ready();
        app.map = ButtonMap::default();
        app.open_tab(View::Prefs);
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
        app.open_tab(View::Prefs);
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

    #[test]
    fn the_transport_buttons_drive_the_time_engine() {
        let mut app = ready();
        let time = |a: TimeAction| Effect::Time(a);
        assert_eq!(press(&mut app, button::PLAY), vec![time(TimeAction::Transport(Command::Play))]);
        assert_eq!(press(&mut app, 93), vec![time(TimeAction::Transport(Command::PlayStart))]);
        assert_eq!(press(&mut app, 90), vec![time(TimeAction::Transport(Command::Record))]);
        assert_eq!(press(&mut app, 91), vec![time(TimeAction::Transport(Command::ToggleOverdub))]);
        assert_eq!(press(&mut app, 92), vec![time(TimeAction::Undo)]);
        assert_eq!(held(&mut app, button::SHIFT, |a| press(a, 92)), vec![time(TimeAction::Redo)]);
        // Rolling, STOP stops the transport and leaves the sounds alone.
        app.set_rolling(true);
        let fx = press(&mut app, button::STOP);
        assert_eq!(fx[0], time(TimeAction::Transport(Command::Stop)));
        assert!(!fx.contains(&Effect::StopSounds));
        // Stopped, STOP silences everything, as before, and goes back to bar 1.
        app.set_rolling(false);
        let fx = press(&mut app, button::STOP);
        assert!(fx.contains(&Effect::StopSounds) && fx.contains(&time(TimeAction::Transport(Command::Stop))));
    }

    #[test]
    fn the_clips_page_takes_the_wheel_and_the_tempo() {
        let mut app = ready();
        app.open_tab(View::Clips);
        assert_eq!(app.view(), View::Clips);
        assert_eq!(app.wheel(1), vec![Effect::ClipsWheel { delta: 1, scroll: false }]);
        assert_eq!(held(&mut app, button::SHIFT, |a| a.wheel(-1)), vec![Effect::ClipsWheel { delta: -1, scroll: true }]);
        // MENU there opens the options and goes back to Clips.
        press(&mut app, MENU);
        assert_eq!(app.view(), View::Options);
        press(&mut app, MENU);
        assert_eq!(app.view(), View::Clips);
        // The page's tempo is the app's: Note Repeat and the arp follow it.
        let fx = app.set_tempo(96.5);
        assert_eq!(app.settings.bpm, 96.5);
        assert!(fx.contains(&Effect::Note("tempo 96.5 BPM".into())));
        app.set_tempo(1000.0);
        assert_eq!(app.settings.bpm, 300.0);
        app.open_tab(View::Play);
        assert_eq!(app.wheel(1), vec![Effect::Kit(1)], "the wheel picks kits on Play");
    }
}
