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
//! (docs/dev/hardware/controller-protocol.md); MENU 123, NOTE REPEAT 11, TAP
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
//! "Show developer tools" (Prefs) adds the Dev tab: the MIDI log, the
//! Performance page, the control mapper (the `mapper` submodule) and the
//! Console. Prefs itself is not a tab: SHIFT + MENU opens it. The PAD BANK buttons,
//! once found, pick banks A-H (`Effect::Bank`).

mod arp;
pub mod mapper;
pub mod saved;
mod synth_pads;

pub use arp::{built_in_patterns, mode_label, ArpItem, ArpView, LiveMap, WorkspaceView};
pub use mapper::{Code, MapView, Mark};
use mpfree_arp::scale::pads::Pad as PadNotes;
pub use synth_pads::SynthItem;

use crate::cores::SoundCores;
use crate::looping::{LoopAction, LoopSource};
use mpfree_arp::{Events, LatchMode, Mode, Pattern, Rate, RecordMode, Recorder};
use mpfree_controller::mpc_one::button;
use mpfree_looper::Command as LoopCommand;
use mpfree_play::TrackArps;
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
    /// Arrange: the time engine's launch grid, record settings and takes
    /// (the Clips tab until Jay renamed it, 12:55Z).
    Arrange,
    /// The synth's page (mpfree-synth-ui): the controls of Poly or FM.
    Synth,
    /// The audio looper: the loop's waveform, its buttons and the line
    /// input's meter.
    Looper,
    /// The sampler's pad editor (mpfree-sampler-ui): the Wave tab on the
    /// pad last hit in the playing kit.
    Sample,
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
    /// Debug mode: CPU by engine, each audio stage against its budget (on
    /// the Debug tab; `engines.rs`).
    Engines,
    /// Debug mode: every process on the MPC, CPU and memory (on the Debug
    /// tab; read from /proc only while the page shows).
    Procs,
    /// Debug mode: the control mapper (on the Debug tab).
    Map,
    /// Debug mode: test, measure and tune the touchscreen (on the Debug tab;
    /// `touch_dev.rs`).
    Touch,
    /// Prefs > System info: the program's version, the system and the
    /// hardware (on the Prefs tab).
    System,
    /// The mixer (mpfree-mixer-ui), over the page under the top bar: TRACK
    /// MIX or the bar's mixer icon (Jay, 09:17Z). Not a tab.
    Mixer,
    /// The FX chain editor (mpfree-chain-ui) in the mixer's place: a strip's
    /// insert row or its "+" opens it, and its Back goes back to the mixer
    /// (layout spec 4b, Jay 22:34Z). Not a tab.
    Chain,
    /// The macros and morph panel over the page under the top bar: a long
    /// press on a macro card in the Q-Link column (layout spec 4b). Not a
    /// tab; its Back goes back to the page it opened over.
    Macros,
}

/// The header's tabs, in the order of the approved layout spec
/// (docs/dev/design/layout-spec.md, approved by the owner 2026-11-11): the music
/// pages, and the Dev tab only with the developer tools on. Prefs is not a
/// tab (SHIFT + MENU).
pub fn tabs(dev: bool) -> Vec<View> {
    let mut t = vec![
        View::Play,
        View::Arrange,
        View::Synth,
        View::Looper,
        View::Sample,
    ];
    if dev {
        t.push(View::Debug);
    }
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
pub const DEBUG_PAGES: [View; 7] = [
    View::Debug,
    View::Perf,
    View::Engines,
    View::Procs,
    View::Map,
    View::Touch,
    View::Console,
];

impl View {
    /// The tab a page belongs to (the Arp panel to Play, the debug pages to
    /// Debug), if any.
    pub fn tab_view(self) -> Option<View> {
        match self {
            View::Arp | View::Pattern => Some(View::Play),
            View::Perf | View::Engines | View::Procs | View::Map | View::Touch | View::Console => {
                Some(View::Debug)
            }
            View::Options
            | View::Guide
            | View::Prefs
            | View::System
            | View::Mixer
            | View::Chain
            | View::Macros => None,
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
            View::Arrange => "Arrange",
            View::Synth => "Synth",
            View::Looper => "Looper",
            View::Sample => "Sample",
            View::Debug => "Dev",
            View::Console => "Console",
            View::Prefs => "Prefs",
            View::Options => "Options",
            View::Guide => "Guide",
            View::Arp => "Arp",
            View::Pattern => "Pattern",
            View::Perf => "Performance",
            View::Engines => "CPU by engine",
            View::Procs => "Processes",
            View::Map => "Map controls",
            View::Touch => "Touch",
            View::System => "System info",
            View::Mixer => "Mixer",
            View::Chain => "FX chain",
            View::Macros => "Macros",
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

/// FILL (control map decision 9, Jay 20:54Z): PLAY held while the
/// transport plays, or pressed twice to latch it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fill {
    Off,
    /// While PLAY is held.
    Held,
    /// Until PLAY is pressed again, or STOP.
    Latched,
}

/// TRACK MUTE (control map decision 5): the pads as mute keys over
/// whatever page shows, until TRACK MUTE is pressed again.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MuteKeys {
    /// The pads play.
    #[default]
    Off,
    /// TRACK MUTE: pads 1-8 mute and unmute tracks 1-8.
    Tracks,
    /// SHIFT + TRACK MUTE: each pad mutes and unmutes itself in the
    /// playing kit (the speakers on Play).
    Pads,
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
    const ALL: [GuideStep; 9] = [
        GuideStep::Menu,
        GuideStep::NoteRepeat,
        GuideStep::TapTempo,
        GuideStep::Minus,
        GuideStep::Plus,
        GuideStep::Rec,
        GuideStep::Overdub,
        GuideStep::Undo,
        GuideStep::PlayStart,
    ];

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
    /// TRACK MIX (SHIFT: PAD MIX), found with the control mapper.
    pub track_mix: Option<u8>,
    /// The mode buttons and the rest of the hardware control map (Jay,
    /// 20:40Z), found with the control mapper.
    pub erase: Option<u8>,
    pub main: Option<u8>,
    pub sixteen_level: Option<u8>,
    pub next_seq: Option<u8>,
    pub step_seq: Option<u8>,
    pub sampler: Option<u8>,
    pub sample_edit: Option<u8>,
    pub prog_edit: Option<u8>,
    pub q_link: Option<u8>,
    pub track_mute: Option<u8>,
}

impl ButtonMap {
    /// The numbers the round 5 guide found on the reference MPC One.
    pub fn measured() -> ButtonMap {
        ButtonMap {
            menu: Some(button::MENU),
            note_repeat: Some(button::NOTE_REPEAT),
            tap_tempo: Some(button::TAP_TEMPO),
            minus: Some(button::MINUS),
            plus: Some(button::PLUS),
            ..ButtonMap::default()
        }
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
            return Some(
                [
                    "PAD BANK A/E",
                    "PAD BANK B/F",
                    "PAD BANK C/G",
                    "PAD BANK D/H",
                ][k as usize],
            );
        }
        GuideStep::ALL
            .iter()
            .find(|&&s| self.get(s) == Some(note))
            .map(|s| s.short())
            .or_else(|| {
                mapper::CONTROLS
                    .iter()
                    .find(|c| {
                        !matches!(c.job, mapper::Job::None | mapper::Job::Fixed(_))
                            && self.of(c.job) == Some(note)
                    })
                    .map(|c| c.name)
            })
    }

    /// Which PAD BANK button a note is (0 = A/E), once found.
    pub fn bank_of(&self, note: u8) -> Option<u8> {
        self.bank
            .iter()
            .position(|&b| b == Some(note))
            .map(|k| k as u8)
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
            Job::TrackMix => self.track_mix,
            Job::Erase => self.erase,
            Job::Main => self.main,
            Job::SixteenLevel => self.sixteen_level,
            Job::NextSeq => self.next_seq,
            Job::StepSeq => self.step_seq,
            Job::Sampler => self.sampler,
            Job::SampleEdit => self.sample_edit,
            Job::ProgEdit => self.prog_edit,
            Job::QLink => self.q_link,
            Job::TrackMute => self.track_mute,
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
            Job::TrackMix => &mut self.track_mix,
            Job::Erase => &mut self.erase,
            Job::Main => &mut self.main,
            Job::SixteenLevel => &mut self.sixteen_level,
            Job::NextSeq => &mut self.next_seq,
            Job::StepSeq => &mut self.step_seq,
            Job::Sampler => &mut self.sampler,
            Job::SampleEdit => &mut self.sample_edit,
            Job::ProgEdit => &mut self.prog_edit,
            Job::QLink => &mut self.q_link,
            Job::TrackMute => &mut self.track_mute,
        };
        *slot = code;
    }

    /// One line for the log: every button the guide asked for.
    pub fn describe(&self) -> String {
        GuideStep::ALL
            .iter()
            .map(|&s| match self.get(s) {
                Some(n) => format!("{} = button {n}", s.short()),
                None => format!("{} = not found", s.short()),
            })
            .collect::<Vec<_>>()
            .join(", ")
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
    /// "Show developer tools": the Dev tab (MIDI log, Performance, Map
    /// controls, Console) and the `perf:` lines in the log.
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
    /// Prefs > Appearance: the theme in use, a built-in name (see
    /// [`THEMES`]); `--theme` can also set a theme file for one run.
    pub theme: String,
    /// Prefs "Sound cores": where the sound runs from the next start
    /// (cores.rs); `--sound-cores` can set it for one run.
    pub sound_cores: SoundCores,
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
        Settings {
            curve: Curve::Linear,
            screen_velocity: 100,
            rate: Rate::Sixteenth,
            bpm: 120.0,
            latch_mode: LatchMode::Replace,
            periods: 3,
            show_pressure: false,
            midi_paused: false,
            console_paused: false,
            debug_mode: false,
            line_input: false,
            loop_monitor: false,
            loop_source: LoopSource::Pads,
            loop_bars: Some(2),
            loop_latency: crate::looping::estimated_latency(3),
            loop_latency_set: false,
            loop_level_db: 0.0,
            theme: "default".to_string(),
            sound_cores: SoundCores::Shared,
        }
    }
}

/// The themes Prefs > Appearance offers: the built-in name and its row.
/// The same names, in the same order, as `mpfree_ui::theme::BUILTIN`.
pub const THEMES: [(&str, &str); 4] = [
    ("default", "Default"),
    ("console", "Console"),
    ("synthwave", "Synthwave"),
    ("spaceship", "Spaceship"),
];

/// The store owner whose files are the themes folder: theme files the
/// user puts in `files/themes/` on the data partition, named `NAME.theme`.
pub const THEMES_FOLDER: &str = "themes";

/// The store owner whose files are the tunings folder: Scala scales the
/// user puts in `files/tunings/` on the data partition (`NAME.scl`, with an
/// optional keyboard map `NAME.kbm`), offered on the Synth tab.
pub const TUNINGS_FOLDER: &str = "tunings";

/// Which part of Prefs is on screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PrefsSection {
    Main,
    Touch,
}

/// One row of a menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    Curve,
    ScreenVelocity,
    LatchMode,
    Buffer,
    FindButtons,
    /// Prefs: opens the touch screen section.
    TouchSection,
    /// Prefs: the theme in use; the arrows step through [`THEMES`].
    Appearance,
    /// The touch screen section: back to the other Prefs.
    PrefsBack,
    CalibrateTouch,
    TouchLab,
    DebugMode,
    /// Prefs: where the sound runs from the next start (cores.rs).
    SoundCores,
    MapControls,
    SystemInfo,
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
    /// The arp's own settings (the Options menu opened from the Arp panel).
    Arp(ArpItem),
    /// The synth's engine and pad layouts (the Synth tab's options).
    Synth(SynthItem),
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
    /// TRACK MUTE: what the pads are, the hint line says.
    pub mute_keys: MuteKeys,
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
    /// The Synth tab's options changed: the engine, the scale or the pad
    /// layouts, for the synth on the audio thread.
    SynthPads(crate::synth::PadSettings),
    /// The mixer opened on a view, or closed (`None`): the audio thread
    /// sends the panel its state only while it is open.
    Mixer(Option<mpfree_mixer_ui::MixView>),
    /// While the mixer is open: Q-Link `knob` (or the wheel, as Q-Link 1)
    /// turned by `delta`, for the panel's selected strip.
    MixerTurn {
        knob: usize,
        delta: i32,
    },
    /// While the chain editor or the macros panel is open: Q-Link `knob`
    /// (or the wheel, as Q-Link 1) turned by `delta`, for the knobs the
    /// Q-Link column shows there.
    ChainTurn {
        knob: usize,
        delta: i32,
    },
    /// Q-LINK: the Q-Link column's next bank.
    QLinkBank,
    /// The wheel on the Synth page: turn the selected knob.
    SynthWheel(i32),
    /// SHIFT + wheel, - or + on the Synth page: the next or previous page.
    SynthPage(i32),
    /// The wheel on the Sample page: zoom the waveform.
    SampleWheel(i32),
    /// SHIFT + wheel, - or + on the Sample page: the previous or next pad
    /// with a sound.
    SamplePad(i32),
    /// UNDO (or SHIFT + UNDO, redo) on the Sample page: the last edit there.
    SampleUndo {
        redo: bool,
    },
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
    /// The data wheel on the Arrange page: step the selected record setting,
    /// or (with SHIFT) scroll the grid.
    ClipsWheel {
        delta: i32,
        scroll: bool,
    },
    /// Keep this library text in the store as arp/patterns.txt (the store's
    /// own thread writes it).
    SavePatterns(String),
    /// STOP in the timed `play` step: end it.
    Quit,
    /// Open the Touch lab page (it starts at its first step).
    TouchLab,
    /// Open the Calibrate touch page (four corner taps).
    CalibrateTouch,
    /// The Dev tab's Touch page: the wheel picks a value (by detents)...
    TouchWheel(i32),
    /// ...and - / + step it (or run the action picked).
    TouchStep(i32),
    /// For the time engine (transport, clips, takes).
    Time(TimeAction),
    /// What the Map controls page shows.
    Map(MapView),
    /// A PAD BANK button pressed: 0 = A/E to 3 = D/H, and whether SHIFT
    /// was held (E-H). The audio loop knows the bank playing, so it picks.
    Bank {
        button: u8,
        shift: bool,
    },
    /// For the looper and the line input.
    Looper(LoopAction),
    /// MAIN on the Play page (None: the next view) and STEP SEQ (Beats).
    PlayView(Option<crate::play_page::PlayView>),
    /// SAMPLE EDIT: the Sample tab opens on this editor.
    SampleEditor(crate::sample_page::Editor),
    /// PROG EDIT: the sound the pads play: the Synth tab while the synth
    /// has them, otherwise Sample > Pads (the audio loop knows which).
    ProgEdit,
    /// SHIFT + PLAY START: the metronome on or off.
    Metronome,
    /// FILL on or off from the coming period: notes with the FILL and
    /// !FILL trig conditions follow it.
    Fill(bool),
}

pub struct App {
    pub settings: Settings,
    pub map: ButtonMap,
    /// One arp per track (owner decision 2). The pads play the armed
    /// track's ([`App::set_track`]); the arp panel, the NOTE REPEAT band and
    /// Latch show and change it. The time engine borrows these for each
    /// period (`TimeHost::period_arps`), so clips through an arp and a
    /// latched arp on another track keep playing.
    pub arps: TrackArps,
    /// The armed track: the one the pads and the arp screens play.
    track: usize,
    /// Each track's arp panel state while another track is armed
    /// ([`App::set_track`] swaps it in and out).
    track_ui: [arp::TrackUi; mpfree_seq::MAX_TRACKS],
    /// The `play` step: STOP ends it. The app: STOP stops the sounds.
    timed: bool,
    shift: bool,
    repeat_held: bool,
    /// ERASE is down: SHIFT and ERASE held, then TAP TEMPO, is the
    /// Console's chord.
    erase_held: bool,
    /// When NOTE REPEAT was last pressed to hold, for the double press
    /// that latches.
    repeat_pressed_at: Option<f64>,
    latched: bool,
    pad_level: PadLevel,
    fill: Fill,
    /// When PLAY was last pressed while the transport played, for the
    /// double press that latches FILL.
    play_pressed_at: Option<f64>,
    /// TRACK MUTE: the pads play, or mute tracks or the kit's pads.
    mute_keys: MuteKeys,
    view: View,
    /// The page to go back to from Console, Options or the guide.
    back: View,
    /// What the chain editor's Back goes back to (the mixer, or the page
    /// it opened on), and the macros panel's.
    chain_back: View,
    macros_back: View,
    selected: usize,
    /// The buttons the guide asks for this time, and which one it is on.
    guide_steps: Vec<GuideStep>,
    guide: usize,
    guide_note: String,
    taps: Vec<f64>,
    /// When the tempo's beats started (seconds), for the TAP TEMPO light.
    beat_zero: f64,
    tap_light: bool,
    /// The part of Prefs on screen: the main rows or one of its sections.
    prefs_section: PrefsSection,
    /// Theme files found in the themes folder at start (`files/themes/` in
    /// `--data`): the file name, which is what `settings.theme` holds, and
    /// the theme's own name for the Appearance row. After [`THEMES`].
    pub theme_files: Vec<(String, String)>,
    /// Tuning files found in the tunings folder at start (`files/tunings/`
    /// in `--data`), in the synth's order ([`crate::synth::Synth::tuning_files`]):
    /// each one's name and whether its keyboard map sets the root and
    /// pitch. The Synth tab's Tuning row offers them after the built-in
    /// tunings.
    pub tuning_files: Vec<(String, bool)>,
    /// The mode the Arp panel picked. NOTE REPEAT and Latch turn on Repeat
    /// when it is Direct.
    arp_mode: Mode,
    /// The arp's other settings (order, octaves, gate, swing); the rate is
    /// `settings.rate` and the mode follows the buttons and pages.
    arp: mpfree_arp::Settings,
    /// The scale's root (0 = C), kept while the scale is Off.
    scale_root: u8,
    /// The owner's own scales from the patterns file: name, notes going up
    /// and coming down. They follow the built-in ones in the Scale row.
    custom_scales: Vec<(String, u16, u16)>,
    /// The Scale row's choice: 0 is Off, then the built-in scales, then the
    /// owner's (kept as a place, since an owner's scale may have the same
    /// notes as a built-in one).
    scale_pick: usize,
    /// The synth pads' engine, scale and bank layouts (the Synth tab's
    /// options), as handed to the synth.
    synth_pads: crate::synth::PadSettings,
    /// The pads' Scale row's choice and root, as `scale_pick` and
    /// `scale_root` are for the arp.
    pad_scale_pick: usize,
    pad_scale_root: u8,
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
    /// Save also keeps the patterns in the store (arp/patterns.txt, when
    /// the store saves: `app --data DIR`), through [`Effect::SavePatterns`].
    pub pattern_file: bool,
    /// Q-Link turns not yet used, per knob (rate and octaves need several).
    qlink_turns: [i32; 4],
    /// What the arp screens were last sent: the step and the notes, and
    /// whether anything else changed since.
    sent_step: u64,
    sent_notes: mpfree_arp::NoteSet,
    arp_dirty: bool,
    /// The transport is rolling (the audio loop says each period).
    rolling: bool,
    /// The synth's pad layout while the synth is on the pads
    /// ([`App::set_pad_layout`]): the pads then feed the arp the layout's
    /// notes, and its events carry notes rather than pad slots.
    pad_layout: Option<[PadNotes; 16]>,
    /// What each held pad pressed into the arp (kept so a layout change
    /// while a pad is down still lets go of the right notes).
    pad_pressed: [PadNotes; 16],
    /// How many held pads press each note (two chord pads can share one).
    note_holds: [u8; 128],
    /// The control mapper (Debug mode).
    mapper: mapper::Mapper,
    /// Buttons pressed while the mapper listened: their release is kept
    /// from the rest of the app too.
    swallowed: [bool; 128],
    /// The Looper tab's Mute.
    loop_muted: bool,
    /// Where the sound runs in this start (cores.rs); Prefs' "Sound cores"
    /// says "(next start)" while the setting differs.
    sound_cores_now: SoundCores,
    /// Prefs, a button or a Synth tab option changed since the last look
    /// (`saved.rs`), and the text last handed to the store for "app",
    /// "map" and "synth".
    save_dirty: bool,
    saved: [String; 3],
}

/// Prefs' main rows. The touch screen settings sit in their own section,
/// opened by the last row; Line input, System info and Appearance (the
/// theme) go just before it.
const PREFS_ROWS: &[Item] = &[
    Item::Curve,
    Item::LatchMode,
    Item::Buffer,
    Item::FindButtons,
    Item::DebugMode,
    Item::SoundCores,
    Item::MapControls,
    Item::LineInput,
    Item::SystemInfo,
    Item::Appearance,
    Item::TouchSection,
];
/// Prefs' touch screen section.
const PREFS_TOUCH_ROWS: &[Item] = &[
    Item::PrefsBack,
    Item::ScreenVelocity,
    Item::CalibrateTouch,
    Item::TouchLab,
];

impl App {
    pub fn new(sample_rate: u32, timed: bool) -> App {
        let settings = Settings::default();
        let arps = TrackArps::new(sample_rate, settings.bpm);
        let patterns = arp::built_in_patterns();
        let pattern = patterns.first().map(|(_, p)| *p).unwrap_or_default();
        let sent_notes = arps.engine_at(0).active().clone();
        let mut app = App {
            settings,
            map: ButtonMap::measured(),
            arps,
            track: 0,
            track_ui: [arp::TrackUi::default(); mpfree_seq::MAX_TRACKS],
            timed,
            shift: false,
            repeat_held: false,
            erase_held: false,
            repeat_pressed_at: None,
            latched: false,
            pad_level: PadLevel::Off,
            fill: Fill::Off,
            play_pressed_at: None,
            mute_keys: MuteKeys::Off,
            view: View::Play,
            back: View::Play,
            chain_back: View::Mixer,
            macros_back: View::Play,
            selected: 0,
            guide_steps: Vec::new(),
            guide: 0,
            guide_note: String::new(),
            taps: Vec::new(),
            beat_zero: 0.0,
            tap_light: false,
            arp_mode: Mode::Repeat,
            arp: mpfree_arp::Settings::default(),
            scale_root: 0,
            custom_scales: Vec::new(),
            scale_pick: 0,
            synth_pads: Default::default(),
            pad_scale_pick: 0,
            pad_scale_root: 0,
            clock: None,
            swing_from_transport: true,
            patterns,
            pattern_index: 0,
            pattern,
            mutations: 0,
            recorder: None,
            record_mode: RecordMode::Step,
            pattern_file: false,
            qlink_turns: [0; 4],
            sent_step: 0,
            sent_notes,
            arp_dirty: true,
            rolling: false,
            pad_layout: None,
            pad_pressed: [PadNotes::default(); 16],
            note_holds: [0; 128],
            mapper: mapper::Mapper::default(),
            swallowed: [false; 128],
            prefs_section: PrefsSection::Main,
            theme_files: Vec::new(),
            tuning_files: Vec::new(),
            loop_muted: false,
            sound_cores_now: SoundCores::Shared,
            save_dirty: false,
            saved: Default::default(),
        };
        // Every track's arp starts on the first pattern, as the panel does.
        for t in 0..mpfree_seq::MAX_TRACKS {
            app.arps.engine_at_mut(t).set_pattern(pattern);
            app.track_ui[t].pattern = pattern;
        }
        app.update_engine();
        // The Synth tab's options start at their defaults: nothing to save
        // until one changes.
        app.saved[saved::SYNTH_SLOT] = app.synth_settings().to_text();
        app
    }

    pub fn view(&self) -> View {
        self.view
    }

    /// Whether the transport is rolling, for STOP and FILL (the audio loop
    /// tells it each period).
    pub fn set_rolling(&mut self, rolling: bool) {
        self.rolling = rolling;
    }

    /// FILL let go, if it was on.
    fn fill_off(&mut self) -> Option<Effect> {
        if self.fill == Fill::Off {
            return None;
        }
        self.fill = Fill::Off;
        Some(Effect::Fill(false))
    }

    /// The first page once everything is ready, and the lights to start with.
    pub fn start(&mut self) -> Vec<Effect> {
        let mut fx = vec![
            self.status(),
            Effect::Settings(self.settings.clone()),
            Effect::Light(button::FULL_LEVEL, LIGHT_OFF),
            Effect::Patterns(self.patterns.clone()),
            self.arp_view(),
        ];
        fx.extend(self.repeat_light());
        let s = &self.settings;
        fx.push(Effect::Looper(LoopAction::Source(s.loop_source)));
        fx.push(Effect::Looper(LoopAction::Command(LoopCommand::SetBars(
            s.loop_bars,
        ))));
        fx.push(Effect::Looper(LoopAction::Command(
            LoopCommand::SetLatency(s.loop_latency_now()),
        )));
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

    /// The armed track's arp engine: the one the pads play.
    #[cfg(test)]
    pub fn eng(&self) -> &mpfree_arp::Engine {
        self.arps.engine_at(self.track)
    }

    #[cfg(test)]
    pub fn eng_mut(&mut self) -> &mut mpfree_arp::Engine {
        self.arps.engine_at_mut(self.track)
    }

    /// A pad pressed: it plays, and writes a step while recording a pattern.
    pub fn pad_down(&mut self, slot: usize, velocity: u8) -> Vec<Effect> {
        if !self.free_time_recording() {
            // In Free time the pads only write steps; the pattern keeps
            // playing the latched notes.
            self.press_pad(slot, velocity.max(1));
        }
        self.record_press(slot as u8, velocity)
    }

    /// What a pad press gives the clip recorder "as pressed" (a track whose
    /// clips play through its arp records the pads before the arp): the
    /// pad, at the velocity it plays at. None while the pads only write
    /// pattern steps (Free time).
    pub fn pressed_event(
        &self,
        slot: usize,
        velocity: u8,
        down: bool,
    ) -> Option<mpfree_play::Event> {
        if self.free_time_recording() || slot >= 16 {
            return None;
        }
        let note = slot as u8;
        Some(if down {
            mpfree_play::Event::NoteOn {
                note,
                velocity: self.live_map().velocity(velocity.max(1)),
            }
        } else {
            mpfree_play::Event::NoteOff { note }
        })
    }

    pub fn pad_up(&mut self, slot: usize) -> Vec<Effect> {
        if !self.free_time_recording() {
            self.release_pad(slot);
        }
        self.record_release(slot as u8)
    }

    pub fn pressure(&mut self, slot: usize, value: u8) {
        if self.pad_layout.is_none() {
            self.arps
                .engine_at_mut(self.track)
                .pressure(slot as u8, value);
            return;
        }
        // Every note of a chord pad gets the pad's pressure.
        if let Some(p) = self.pad_pressed.get(slot).copied() {
            for &n in p.notes() {
                self.arps.engine_at_mut(self.track).pressure(n, value);
            }
        }
    }

    /// One audio period: the notes that start (or stop) in it. The arp
    /// follows the clock given by [`App::set_clock`] for this period, and
    /// free-runs at the app's tempo without one.
    pub fn process(&mut self, frames: u32, out: &mut Events) {
        self.sync_arp();
        self.arps.engine_at_mut(self.track).process(frames, out);
    }

    /// Pads the engine keeps playing (held, or latched). With a pad layout:
    /// a pad any of whose notes is held or latched.
    pub fn active(&self, slot: usize) -> bool {
        match &self.pad_layout {
            None => self
                .arps
                .engine_at(self.track)
                .active()
                .contains(slot as u8),
            Some(l) => l.get(slot).is_some_and(|p| {
                p.notes()
                    .iter()
                    .any(|&n| self.arps.engine_at(self.track).active().contains(n))
            }),
        }
    }

    /// TRACK MUTE: whether a pad press plays, or mutes a track or a pad
    /// (the audio loop applies it, and lights the pads).
    pub fn mute_keys(&self) -> MuteKeys {
        self.mute_keys
    }

    /// NOTE REPEAT is held or Latch is on.
    pub fn repeating(&self) -> bool {
        self.repeat_held || self.latched
    }

    /// The engine makes notes of its own (Repeat, Arp or Pattern), so a hit
    /// may sound later than the press, and latched pads stay lit.
    pub fn generating(&self) -> bool {
        self.arps.engine_at(self.track).settings().mode != Mode::Direct
    }

    fn update_engine(&mut self) {
        let mut s = self.arp;
        s.mode = self.engine_mode();
        s.rate = self.settings.rate;
        self.arps.engine_at_mut(self.track).set_settings(s);
        let latch = if self.latched {
            self.settings.latch_mode
        } else {
            LatchMode::Off
        };
        if self.arps.engine_at(self.track).latch() != latch {
            self.arps.engine_at_mut(self.track).set_latch(latch);
        }
        self.arps
            .engine_at_mut(self.track)
            .set_own_swing(!self.swing_from_transport);
        // Every track's arp free-runs at the app's tempo while stopped.
        for t in 0..mpfree_seq::MAX_TRACKS {
            self.arps.engine_at_mut(t).set_tempo(self.settings.bpm);
        }
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
        Effect::Status(Status {
            shift: self.shift,
            pad_level: self.pad_level,
            repeat: self.repeating(),
            latched: self.latched,
            rate: self.settings.rate.label(),
            bpm: self.settings.bpm,
            mute_keys: self.mute_keys,
        })
    }

    /// The NOTE REPEAT light: on while the pads play through the arp.
    fn repeat_light(&self) -> Option<Effect> {
        self.map.note_repeat.map(|n| {
            Effect::Light(
                n,
                if self.generating() {
                    LIGHT_ON
                } else {
                    LIGHT_OFF
                },
            )
        })
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
            if Some(note) == self.map.erase {
                self.erase_held = false;
            }
            if Some(note) == self.map.note_repeat && self.repeat_held {
                self.repeat_held = false;
                fx.extend(self.arp_changed());
            }
            if note == button::PLAY && self.fill == Fill::Held {
                self.fill = Fill::Off;
                fx.push(Effect::Fill(false));
            }
            return fx;
        }
        if self.view == View::Guide {
            return self.guide_press(note);
        }
        let shift = self.shift;
        if note == button::FULL_LEVEL {
            let want = if shift {
                PadLevel::Half
            } else {
                PadLevel::Full
            };
            self.pad_level = if self.pad_level == want {
                PadLevel::Off
            } else {
                want
            };
            let on = self.pad_level != PadLevel::Off;
            fx.push(Effect::Light(
                button::FULL_LEVEL,
                if on { LIGHT_ON } else { LIGHT_OFF },
            ));
            fx.push(self.status());
            fx.push(Effect::Note(match self.pad_level {
                PadLevel::Off => "pad level: as played".to_string(),
                PadLevel::Full => "pad level: Full Level (every hit 127)".to_string(),
                PadLevel::Half => "pad level: Half Level (every hit 64)".to_string(),
            }));
        } else if note == button::STOP {
            fx.extend(self.fill_off());
            if self.timed {
                fx.push(Effect::Quit);
            } else if shift {
                // The MPC's SHIFT + STOP: stop, silence everything and go
                // back to bar 1 (Jay, 20:40Z). The Console has a chord of
                // its own: SHIFT + ERASE + TAP TEMPO.
                fx.extend(self.stop());
                fx.push(Effect::Time(TimeAction::Transport(Command::StopAndRewind)));
                fx.push(Effect::Note(
                    "SHIFT + STOP: stopped, back to bar 1".to_string(),
                ));
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
        } else if note == button::PLAY && self.rolling {
            // FILL while held; pressed twice, latched; pressed again, off.
            let double = self
                .play_pressed_at
                .is_some_and(|at| t - at <= DOUBLE_PRESS_SECONDS);
            self.play_pressed_at = (!double).then_some(t);
            let (fill, line) = match self.fill {
                Fill::Latched => (Fill::Off, "FILL off"),
                _ if double => (Fill::Latched, "FILL latched (PLAY again: off)"),
                _ => (Fill::Held, "FILL while PLAY is held (twice: latch)"),
            };
            self.fill = fill;
            fx.push(Effect::Fill(fill != Fill::Off));
            fx.push(Effect::Note(line.to_string()));
        } else if note == button::PLAY {
            fx.extend(self.fill_off());
            self.arps.engine_at_mut(self.track).resume();
            fx.push(Effect::Time(TimeAction::Transport(Command::Play)));
        } else if Some(note) == self.map.play_start && shift {
            fx.push(Effect::Metronome);
        } else if Some(note) == self.map.play_start {
            self.arps.engine_at_mut(self.track).resume();
            fx.push(Effect::Time(TimeAction::Transport(Command::PlayStart)));
        } else if Some(note) == self.map.track_mix {
            fx.extend(self.toggle_mixer(shift));
        } else if let Some(e) = self.mode_button(note, shift) {
            fx.extend(e);
        } else if shift && Some(note) == self.map.rec {
            // SHIFT + REC: Capture, on every page (Jay, 20:40Z).
            fx.push(Effect::Time(TimeAction::Capture));
        } else if self.view == View::Looper && Some(note) == self.map.rec {
            fx.extend(self.looper_tap(crate::looper_page::B_RECORD));
        } else if self.view == View::Looper && Some(note) == self.map.overdub {
            fx.extend(self.looper_tap(crate::looper_page::B_OVERDUB));
        } else if self.view == View::Looper && Some(note) == self.map.undo {
            fx.extend(self.looper_tap(if shift {
                crate::looper_page::B_REDO
            } else {
                crate::looper_page::B_UNDO
            }));
        } else if Some(note) == self.map.rec {
            fx.push(Effect::Time(TimeAction::Transport(Command::Record)));
        } else if Some(note) == self.map.overdub {
            fx.push(Effect::Time(TimeAction::Transport(Command::ToggleOverdub)));
        } else if Some(note) == self.map.undo && self.view == View::Sample {
            fx.push(Effect::SampleUndo { redo: shift });
        } else if Some(note) == self.map.undo {
            fx.push(Effect::Time(if shift {
                TimeAction::Redo
            } else {
                TimeAction::Undo
            }));
        } else if Some(note) == self.map.note_repeat {
            let double = self
                .repeat_pressed_at
                .is_some_and(|at| t - at <= DOUBLE_PRESS_SECONDS);
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
            } else if !shift && self.view == View::Prefs && self.prefs_section != PrefsSection::Main
            {
                // MENU in a Prefs section: back to the main rows.
                fx.extend(self.leave_prefs_section());
            } else if shift {
                fx.extend(self.toggle(View::Prefs));
            } else if self.view == View::System {
                // System info opens from Prefs, and MENU goes back there.
                fx.extend(self.show(View::Prefs));
            } else if self.view != View::Pattern {
                fx.extend(self.toggle(View::Options));
            }
        } else if Some(note) == self.map.tap_tempo && shift && self.erase_held {
            // The Console's chord: hold SHIFT and ERASE, then press TAP
            // TEMPO. Nothing else uses it, so a developer opens the Console
            // only on purpose (Jay, 22:12Z); the press is no tempo tap.
            fx.extend(self.toggle(View::Console));
        } else if Some(note) == self.map.tap_tempo {
            fx.extend(self.tap(t));
        } else if let Some(k) = self.map.bank_of(note) {
            fx.push(Effect::Bank { button: k, shift });
        } else if Some(note) == self.map.minus || Some(note) == self.map.plus {
            let d = if Some(note) == self.map.plus { 1 } else { -1 };
            if self.in_menu() {
                fx.extend(self.step_item(self.selected, d));
            } else if self.view == View::Touch && !shift {
                // The value picked on the Touch page, one step.
                fx.push(Effect::TouchStep(d));
            } else if self.view == View::Map && !shift {
                // + listens for the selected control, - skips it.
                fx.extend(if d > 0 {
                    self.map_listen()
                } else {
                    self.map_skip()
                });
            } else if matches!(
                self.view,
                View::Debug
                    | View::Perf
                    | View::Engines
                    | View::Procs
                    | View::Map
                    | View::Touch
                    | View::Console
            ) {
                fx.extend(self.debug_step(d));
            } else if self.repeat_held || self.view == View::Arp {
                fx.extend(self.arp_mode_step(d));
            } else if self.view == View::Synth {
                fx.push(Effect::SynthPage(d));
            } else if self.view == View::Sample {
                fx.push(Effect::SamplePad(d));
            } else {
                fx.push(Effect::Kit(d));
            }
        }
        fx
    }

    /// The mode buttons and ERASE, as the hardware control map has them
    /// (Jay, 20:40Z): each goes to the page with the same job. None when
    /// `note` is not one of them.
    fn mode_button(&mut self, note: u8, shift: bool) -> Option<Vec<Effect>> {
        use crate::play_page::PlayView;
        let m = self.map;
        let is = |b: Option<u8>| b == Some(note);
        let fx = if is(m.erase) {
            // ERASE and SHIFT + ERASE wait for live erase and the FX lock
            // mode (with a step held); alone they do nothing. Held with
            // SHIFT, TAP TEMPO opens the Console.
            self.erase_held = true;
            return Some(Vec::new());
        } else if is(m.main) {
            // MAIN: Play; on Play, the next view (All, Beats, Loop, Drum).
            if self.view == View::Play {
                vec![Effect::PlayView(None)]
            } else {
                self.show(View::Play)
            }
        } else if is(m.step_seq) {
            let mut fx = self.go(View::Play);
            fx.push(Effect::PlayView(Some(PlayView::Beats)));
            fx
        } else if is(m.next_seq) {
            self.go(View::Arrange)
        } else if is(m.sampler) {
            self.go(if shift { View::Looper } else { View::Sample })
        } else if is(m.sample_edit) {
            let mut fx = vec![Effect::SampleEditor(crate::sample_page::Editor::Wave)];
            fx.extend(self.go(View::Sample));
            fx
        } else if is(m.prog_edit) {
            vec![Effect::ProgEdit]
        } else if is(m.q_link) {
            // Q-LINK: the column's next bank (Macros 1-4, 5-8, the page's
            // own knobs), where the column has macro banks (layout spec 4b).
            if matches!(
                self.view,
                View::Play | View::Arrange | View::Chain | View::Macros
            ) {
                vec![Effect::QLinkBank]
            } else {
                Vec::new()
            }
        } else if is(m.track_mute) {
            // TRACK MUTE: the pads mute tracks; SHIFT + TRACK MUTE: the
            // kit's pads. The same press again leaves; the other one
            // switches.
            let want = if shift {
                MuteKeys::Pads
            } else {
                MuteKeys::Tracks
            };
            self.mute_keys = if self.mute_keys == want {
                MuteKeys::Off
            } else {
                want
            };
            let on = self.mute_keys != MuteKeys::Off;
            vec![
                Effect::Light(note, if on { LIGHT_ON } else { LIGHT_OFF }),
                self.status(),
                Effect::Note(
                    match self.mute_keys {
                        MuteKeys::Off => "TRACK MUTE: the pads play again",
                        MuteKeys::Tracks => "TRACK MUTE: pads 1-8 mute tracks 1-8",
                        MuteKeys::Pads => "SHIFT + TRACK MUTE: each pad mutes itself in the kit",
                    }
                    .to_string(),
                ),
            ]
        } else if is(m.sixteen_level) {
            // SHIFT + 16 LEVEL: the synth pads' Chromatic | In key switch.
            // 16 LEVEL itself comes with 16 Levels for kits.
            if !shift {
                return Some(Vec::new());
            }
            let in_key = self.synth_pads.layout() != mpfree_arp::scale::pads::PadLayout::InKey;
            let mut fx = self.synth_pick_layout(in_key);
            if fx.iter().any(|e| matches!(e, Effect::SynthPads(_))) {
                fx.push(Effect::Note(format!(
                    "SHIFT + 16 LEVEL: synth pads {}",
                    if in_key { "In key" } else { "Chromatic" }
                )));
            }
            fx
        } else {
            return None;
        };
        Some(fx)
    }

    /// A page from a mode button: shown, unless it shows already.
    fn go(&mut self, v: View) -> Vec<Effect> {
        if self.view == v {
            Vec::new()
        } else {
            self.show(v)
        }
    }

    /// STOP in the app, with the transport stopped: every sound stops. A
    /// latched arp still playing is silenced and keeps its latch (design
    /// decision 5); STOP again, or STOP with nothing latched, lets go of
    /// the latched pads too. PLAY or a pad press plays the kept latch again.
    fn stop(&mut self) -> Vec<Effect> {
        if self.latched
            && self.generating()
            && !self.arps.engine_at(self.track).hushed()
            && !self.arps.engine_at(self.track).active().is_empty()
        {
            self.arps.engine_at_mut(self.track).hush();
            let mut fx = vec![
                Effect::StopSounds,
                Effect::Note("STOP: arp silenced, latch kept (STOP again clears it)".to_string()),
                self.status(),
            ];
            fx.extend(self.repeat_light());
            return fx;
        }
        self.latched = false;
        self.arps.engine_at_mut(self.track).panic();
        self.update_engine();
        let mut fx = vec![
            Effect::StopSounds,
            Effect::Note("STOP: all sounds stopped".to_string()),
            self.status(),
        ];
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
        let mut fx = vec![
            self.status(),
            Effect::Note(format!("tempo {:.1} BPM (tapped)", self.settings.bpm)),
        ];
        if self.in_menu() {
            fx.push(self.menu_view());
        }
        fx
    }

    /// The tempo, set from elsewhere (the Arrange page): Note Repeat, the arp
    /// and the transport follow it.
    pub fn set_tempo(&mut self, bpm: f64) -> Vec<Effect> {
        self.settings.bpm = bpm.clamp(30.0, 300.0);
        self.update_engine();
        let mut fx = vec![
            self.status(),
            Effect::Note(format!("tempo {:.1} BPM", self.settings.bpm)),
        ];
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
            View::Guide | View::System => Vec::new(),
            // In a menu the wheel moves between rows and - / + change the
            // value, so menus work without touch; SHIFT + wheel also changes
            // the value, in case - and + are not known yet.
            _ if self.in_menu() && self.shift => self.step_item(self.selected, delta.signum()),
            _ if self.in_menu() => {
                let last = self.items().len() as i32 - 1;
                self.select((self.selected as i32 + delta).clamp(0, last) as usize)
            }
            View::Pattern => vec![Effect::WorkspaceWheel(delta)],
            // The mixer: the wheel is the selected strip's volume (Q-Link 1).
            View::Mixer => vec![Effect::MixerTurn { knob: 0, delta }],
            // The chain editor and the macros panel: the wheel is Q-Link 1.
            View::Chain | View::Macros => vec![Effect::ChainTurn { knob: 0, delta }],
            // On the Debug tab SHIFT + wheel picks the page; on Map controls
            // the wheel picks the control.
            View::Debug
            | View::Perf
            | View::Engines
            | View::Procs
            | View::Map
            | View::Touch
            | View::Console
                if self.shift =>
            {
                self.debug_step(delta.signum())
            }
            View::Map => {
                self.mapper.move_by(delta);
                vec![Effect::Map(self.mapper.view())]
            }
            // The Touch page: the wheel picks a value to tune.
            View::Touch => vec![Effect::TouchWheel(delta)],
            // NOTE REPEAT held or the Arp panel: the rate, order or pattern.
            _ if self.repeat_held || self.view == View::Arp => self.arp_wheel(delta),
            View::Arrange => vec![Effect::ClipsWheel {
                delta,
                scroll: self.shift,
            }],
            View::Synth if self.shift => vec![Effect::SynthPage(delta.signum())],
            View::Synth => vec![Effect::SynthWheel(delta)],
            View::Sample if self.shift => vec![Effect::SamplePad(delta.signum())],
            View::Sample => vec![Effect::SampleWheel(delta)],
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
            && matches!(
                self.view,
                View::Play
                    | View::Arrange
                    | View::Synth
                    | View::Looper
                    | View::Sample
                    | View::Debug
                    | View::Console
                    | View::Arp
                    | View::Perf
                    | View::Engines
                    | View::Procs
                    | View::Map
                    | View::Touch
            )
        {
            self.back = self.view;
        }
        let mut fx = Vec::new();
        if self.view == View::Mixer && v != View::Mixer {
            fx.push(Effect::Mixer(None));
        }
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
            self.prefs_section = PrefsSection::Main;
            fx.push(self.menu_view());
        }
        fx.push(Effect::Show(v));
        fx.extend(self.after_view_change());
        fx
    }

    /// Open `v`, or go back if it is open already.
    /// TRACK MIX, SHIFT + TRACK MIX (PAD MIX) or the bar's mixer icon: the
    /// mixer opens over the page on Track Mix (or Pad Mix); again, it closes
    /// back to the page. A tab closes it too.
    pub fn toggle_mixer(&mut self, pads: bool) -> Vec<Effect> {
        use mpfree_mixer_ui::MixView;
        if self.view == View::Guide {
            return Vec::new();
        }
        if self.view == View::Mixer {
            let mut fx = vec![Effect::Mixer(None)];
            fx.extend(self.toggle(View::Mixer));
            return fx;
        }
        let mut fx = self.show(View::Mixer);
        fx.insert(
            0,
            Effect::Mixer(Some(if pads { MixView::Pads } else { MixView::Tracks })),
        );
        fx
    }

    /// The mixer's insert row or "+": the chain editor takes the mixer's
    /// place (or the page's, from elsewhere), and its Back comes back here.
    pub fn open_chain(&mut self) -> Vec<Effect> {
        if matches!(self.view, View::Guide | View::Chain) {
            return Vec::new();
        }
        self.chain_back = self.view;
        self.show(View::Chain)
    }

    /// The chain editor's Back: to the mixer it took the place of, on the
    /// mixer view `mix` it showed, or to the page it opened on.
    pub fn close_chain(&mut self, mix: mpfree_mixer_ui::MixView) -> Vec<Effect> {
        if self.view != View::Chain {
            return Vec::new();
        }
        self.back_to(self.chain_back, mix)
    }

    /// A long press on a macro card: the macros panel over the page.
    pub fn open_macros(&mut self) -> Vec<Effect> {
        if matches!(self.view, View::Guide | View::Macros) {
            return Vec::new();
        }
        self.macros_back = self.view;
        self.show(View::Macros)
    }

    /// The macros panel's Back: to the page it opened over.
    pub fn close_macros(&mut self, mix: mpfree_mixer_ui::MixView) -> Vec<Effect> {
        if self.view != View::Macros {
            return Vec::new();
        }
        self.back_to(self.macros_back, mix)
    }

    /// Back from the chain editor or the macros panel to `v`: the mixer
    /// opens again on its view `mix`.
    fn back_to(&mut self, v: View, mix: mpfree_mixer_ui::MixView) -> Vec<Effect> {
        let v = match v {
            View::Guide | View::Options | View::Prefs => self.back,
            v => v,
        };
        let mut fx = self.show(v);
        if v == View::Mixer {
            fx.insert(0, Effect::Mixer(Some(mix)));
        }
        fx
    }

    fn toggle(&mut self, v: View) -> Vec<Effect> {
        if self.view == v {
            let back = if self.back == v {
                View::Play
            } else {
                self.back
            };
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

    /// A tap on the Dev tab's strip: MIDI, Performance, Map controls or
    /// Console (SHIFT + ERASE + TAP TEMPO opens the Console too).
    pub fn debug_page(&mut self, i: usize) -> Vec<Effect> {
        match DEBUG_PAGES.get(i) {
            Some(&v) if self.settings.debug_mode => self.show(v),
            _ => Vec::new(),
        }
    }

    /// The next or previous page of the Dev tab.
    fn debug_step(&mut self, d: i32) -> Vec<Effect> {
        if !self.settings.debug_mode {
            return Vec::new();
        }
        let at = DEBUG_PAGES
            .iter()
            .position(|&v| v == self.view)
            .unwrap_or(0) as i32;
        let to = (at + d).rem_euclid(DEBUG_PAGES.len() as i32) as usize;
        self.show(DEBUG_PAGES[to])
    }

    fn set_debug_mode(&mut self, on: bool) -> Vec<Effect> {
        if self.settings.debug_mode == on {
            return Vec::new();
        }
        self.settings.debug_mode = on;
        if !on
            && matches!(
                self.back,
                View::Debug
                    | View::Perf
                    | View::Engines
                    | View::Procs
                    | View::Map
                    | View::Touch
                    | View::Console
            )
        {
            self.back = View::Play;
        }
        vec![Effect::Note(format!(
            "developer tools: {}",
            if on { "on" } else { "off" }
        ))]
    }

    /// Where the sound runs in this start (`player_session`, once the
    /// sound's cores are set up).
    pub fn set_sound_cores_now(&mut self, mode: SoundCores) {
        self.sound_cores_now = mode;
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
            self.save_dirty = true;
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
        self.save_dirty = true;
        let mut fx = Vec::new();
        if l.job == mapper::Job::NoteRepeat {
            fx.extend(self.repeat_light());
        }
        if matches!(l.job, mapper::Job::Bank(_)) {
            // The audio loop lights the bank playing.
            fx.push(Effect::Bank {
                button: u8::MAX,
                shift: false,
            });
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
        self.save_dirty = true;
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
                let mut fx = vec![
                    Effect::Looper(LoopAction::Monitor(self.settings.loop_monitor)),
                    Effect::Settings(self.settings.clone()),
                ];
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
        vec![
            Effect::Looper(LoopAction::Source(source)),
            Effect::Looper(LoopAction::Command(LoopCommand::SetLatency(
                self.settings.loop_latency_now(),
            ))),
        ]
    }

    // ------------------------------------------------------------ menus

    /// Where `item` sits in the menu on screen, if it is shown.
    pub fn row_of(&self, item: Item) -> Option<usize> {
        self.items().iter().position(|&i| i == item)
    }

    fn items(&self) -> &'static [Item] {
        match (self.view, self.back) {
            (View::Prefs, _) if self.prefs_section == PrefsSection::Touch => PREFS_TOUCH_ROWS,
            (View::Prefs, _) => PREFS_ROWS,
            (_, View::Looper) => &[
                Item::LoopSource,
                Item::LineInput,
                Item::LoopMonitor,
                Item::LoopBars,
                Item::LoopLatency,
                Item::LoopLevel,
            ],
            (_, View::Debug | View::Perf | View::Engines | View::Procs | View::Map) => {
                &[Item::ShowPressure, Item::MidiLog, Item::ClearLogs]
            }
            // The full-screen touch checks, also in Prefs > Touch screen.
            (_, View::Touch) => &[Item::CalibrateTouch, Item::TouchLab],
            (_, View::Console) => &[Item::ConsoleFollow, Item::ClearConsole],
            (_, View::Arp) => &arp::ARP_OPTIONS,
            (_, View::Synth) => &synth_pads::SYNTH_OPTIONS,
            _ => &[Item::Rate, Item::Tempo, Item::PadLevel, Item::Latch],
        }
    }

    /// Every theme Prefs > Appearance offers, in order: (what
    /// `settings.theme` holds, the row's text). The built-in themes, then
    /// the themes folder's files.
    pub fn theme_choices(&self) -> impl Iterator<Item = (&str, &str)> {
        THEMES.iter().copied().chain(
            self.theme_files
                .iter()
                .map(|(f, n)| (f.as_str(), n.as_str())),
        )
    }

    /// True when `theme` is one Appearance offers (built in or in the
    /// themes folder), so it is kept and restored.
    pub fn offers_theme(&self, theme: &str) -> bool {
        self.theme_choices().any(|t| t.0 == theme)
    }

    fn item_text(&self, item: Item) -> (&'static str, String) {
        let s = &self.settings;
        match item {
            Item::Curve => ("Pad velocity curve", s.curve.label().to_string()),
            Item::ScreenVelocity => ("Screen pad velocity", s.screen_velocity.to_string()),
            Item::LatchMode => (
                "Latch keeps pads",
                match s.latch_mode {
                    LatchMode::Toggle => "Toggle".to_string(),
                    _ => "Replace".to_string(),
                },
            ),
            Item::Buffer => (
                "Audio buffer",
                format!(
                    "{} periods, {:.1} ms",
                    s.periods,
                    s.periods as f64 * 128.0 / 44.1
                ),
            ),
            Item::FindButtons => ("Find the buttons again", "Start".to_string()),
            Item::TouchSection => ("Touch screen", "Open".to_string()),
            Item::Appearance => (
                "Appearance",
                self.theme_choices()
                    .find(|t| t.0 == s.theme)
                    .map(|t| t.1)
                    .unwrap_or("Theme file")
                    .to_string(),
            ),
            Item::PrefsBack => ("Back to Prefs", "Back".to_string()),
            Item::CalibrateTouch => ("Calibrate touch", "Start".to_string()),
            Item::TouchLab => ("Touch lab", "Open".to_string()),
            Item::DebugMode => (
                "Show developer tools",
                if s.debug_mode { "On" } else { "Off" }.to_string(),
            ),
            Item::SoundCores => (
                "Sound cores",
                if s.sound_cores == self.sound_cores_now {
                    s.sound_cores.row().to_string()
                } else {
                    format!("{} (next start)", s.sound_cores.row())
                },
            ),
            Item::MapControls => ("Map controls", "Open".to_string()),
            Item::SystemInfo => ("System info", "Open".to_string()),
            Item::Rate => ("Note repeat rate", s.rate.label().to_string()),
            Item::Tempo => ("Tempo", format!("{:.1} BPM", s.bpm)),
            Item::PadLevel => (
                "Pad level",
                match self.pad_level {
                    PadLevel::Off => "As played".to_string(),
                    PadLevel::Full => "Full (127)".to_string(),
                    PadLevel::Half => "Half (64)".to_string(),
                },
            ),
            Item::Latch => ("Latch", if self.latched { "On" } else { "Off" }.to_string()),
            Item::ShowPressure => (
                "Pad pressure in the MIDI log",
                if s.show_pressure { "Shown" } else { "Hidden" }.to_string(),
            ),
            Item::MidiLog => (
                "MIDI log",
                if s.midi_paused { "Paused" } else { "Running" }.to_string(),
            ),
            Item::ClearLogs => ("Clear the MIDI and event logs", "Clear".to_string()),
            Item::ConsoleFollow => (
                "Console",
                if s.console_paused {
                    "Paused"
                } else {
                    "Following"
                }
                .to_string(),
            ),
            Item::ClearConsole => ("Clear the console", "Clear".to_string()),
            Item::LineInput => (
                "Line input",
                if s.line_input { "On" } else { "Off" }.to_string(),
            ),
            Item::LoopSource => ("Record from", s.loop_source.name().to_string()),
            Item::LoopMonitor => (
                "Monitor the input",
                if s.loop_monitor { "On" } else { "Off" }.to_string(),
            ),
            Item::LoopBars => ("First pass length", crate::looping::bars_text(s.loop_bars)),
            Item::LoopLatency => (
                "Latency to make up for",
                format!(
                    "{} frames, {:.1} ms{}",
                    s.loop_latency,
                    s.loop_latency as f64 / 44.1,
                    if s.loop_latency_set {
                        ""
                    } else {
                        " (estimated)"
                    }
                ),
            ),
            Item::LoopLevel => ("Loop level", format!("{:+.0} dB", s.loop_level_db)),
            Item::Arp(a) => self.arp_item_text(a),
            Item::Synth(a) => self.synth_item_text(a),
        }
    }

    fn item_help(&self, item: Item) -> Vec<&'static str> {
        match item {
            Item::Curve => vec![
                "How hard a pad must be hit for a loud note.",
                "Soft: loud notes come easily. Hard: more of the",
                "pad's travel stays below the top, for more",
                "dynamics. Linear plays the pad's own value.",
                "FULL LEVEL and Half Level override it.",
            ],
            Item::ScreenVelocity => vec![
                "The velocity of pads played on the screen.",
                "The touchscreen cannot sense how hard you tap.",
            ],
            Item::LatchMode => vec![
                "With Latch on (NOTE REPEAT twice, or SHIFT +",
                "NOTE REPEAT), pads keep repeating after you",
                "let go.",
                "Replace: a fresh press starts a new set.",
                "Toggle: each press adds or removes that pad.",
            ],
            Item::Buffer => vec![
                "How much sound is queued ahead of the speakers.",
                "3 periods is what Akai's app uses. 2 periods",
                "plays about 2.9 ms sooner after a hit, with",
                "more risk of clicks (underruns). Changing it",
                "restarts the sound.",
            ],
            Item::FindButtons => vec![
                "Ask for MENU, NOTE REPEAT, TAP TEMPO, - and +,",
                "REC, OVERDUB, UNDO and PLAY START again,",
                "one at a time.",
            ],
            Item::Appearance => vec![
                "The theme: the colours and the panels' style.",
                "The arrows, - / + or SHIFT + wheel step through",
                "Default, Console, Synthwave and Spaceship; the",
                "screen changes at once. The layout and the font",
                "stay the same. It is kept for the next start.",
            ],
            Item::TouchSection => vec![
                "The touch screen's settings: screen pad",
                "velocity, Calibrate touch and the Touch lab.",
            ],
            Item::PrefsBack => vec!["Back to the other Prefs. MENU does the same."],
            Item::CalibrateTouch => vec![
                "Tap a target in each corner to check that",
                "touches land where you touch. This version",
                "only reports and logs the result; nothing",
                "is saved. Exit or MENU leaves it.",
            ],
            Item::TouchLab => vec![
                "Timed touchscreen tests: taps, long presses,",
                "drags, swipes, flings and held fingers. Each",
                "step's result goes to the system log. Exit,",
                "MENU or a page button leaves it.",
            ],
            Item::DebugMode => vec![
                "Adds the Dev tab: the MIDI log, the",
                "Performance page (CPU per thread, memory,",
                "audio render time), Map controls and the",
                "Console. The numbers also go to the log",
                "every 30 s. SHIFT + wheel picks the page.",
            ],
            Item::SoundCores => vec![
                "Which of the MPC's cores make the sound,",
                "from the next start. Shared: as before.",
                "Core 3: the audio on a core of its own.",
                "Cores 3 and 2: some drums and effects on",
                "core 2 too. Cores 3, 2 and 1 (a test): and",
                "the synth on core 1. Dev > CPU by engine.",
            ],
            Item::MapControls => vec![
                "Find the numbers of the MPC's buttons, one at",
                "a time, at your pace: pick a control, press it,",
                "and see which ones are left. Turns on debug",
                "mode. Wheel: pick, +: listen, -: skip.",
            ],
            Item::SystemInfo => vec![
                "The pad player's version and commit, the",
                "operating system, the kernel and the",
                "hardware it runs on, read when it opens.",
                "MENU goes back to Prefs.",
            ],
            Item::Rate => vec![
                "How often a held pad repeats with NOTE REPEAT",
                "or Latch: from quarter notes to 32nd notes,",
                "T for triplets.",
            ],
            Item::Tempo => vec![
                "The tempo the transport, the clips and",
                "Note Repeat follow. TAP TEMPO sets it too:",
                "tap it three or four times.",
            ],
            Item::PadLevel => vec![
                "FULL LEVEL plays every hit at 127,",
                "SHIFT + FULL LEVEL (Half Level) at 64.",
            ],
            Item::Latch => vec![
                "NOTE REPEAT twice, or SHIFT + NOTE REPEAT.",
                "Pads you play keep repeating until Latch goes",
                "off (NOTE REPEAT once) or STOP.",
            ],
            Item::ShowPressure => vec![
                "Pad pressure (aftertouch) sends many messages",
                "while a pad is held. Hidden, they are counted",
                "but not listed.",
            ],
            Item::MidiLog => vec!["Pause the MIDI log to read it."],
            Item::ClearLogs => vec!["Empty the MIDI and event logs."],
            Item::ConsoleFollow => vec![
                "Pause the console to read it; it keeps",
                "collecting lines meanwhile.",
            ],
            Item::ClearConsole => vec!["Empty the console page."],
            Item::LineInput => vec![
                "Opens the MPC's line input for the looper.",
                "Off at every start. Which jacks it reads is",
                "not checked yet (line in L and R, most",
                "likely). Turning it off closes the input.",
            ],
            Item::LoopSource => vec![
                "What the looper records: the pads and the",
                "synth (before the click), or the line input.",
                "Turning Line input on picks it; recording",
                "the pads opens nothing.",
            ],
            Item::LoopMonitor => vec![
                "Hear the line input through the outputs,",
                "about 9 ms late. Leave it off if you hear",
                "the instrument another way.",
            ],
            Item::LoopBars => vec![
                "How long the looper's first pass is: 1, 2,",
                "4 or 8 bars, or free: from the next bar line",
                "until the bar line after Record again.",
            ],
            Item::LoopLatency => vec![
                "How late your playing reaches the looper:",
                "the output buffer, the input buffer and the",
                "converters. Estimated from the audio buffer",
                "until you set it. Measure it with a cable",
                "from an output to the input.",
            ],
            Item::LoopLevel => vec![
                "The loop's playback level, -24 to +6 dB.",
                "0 dB plays it as it was recorded.",
            ],
            Item::Arp(a) => Self::arp_item_help(a),
            Item::Synth(a) => Self::synth_item_help(a),
        }
    }

    pub fn menu_view(&self) -> Effect {
        let items = self.items();
        let selected = self.selected.min(items.len() - 1);
        let title = match (self.view, self.back) {
            (View::Prefs, _) if self.prefs_section == PrefsSection::Touch => {
                "Prefs · Touch screen".to_string()
            }
            (View::Prefs, _) => "Prefs".to_string(),
            (_, View::Arp) => "Arp options".to_string(),
            (_, View::Debug) => "MIDI log options".to_string(),
            (
                _,
                b @ (View::Perf
                | View::Engines
                | View::Procs
                | View::Map
                | View::Touch
                | View::Console),
            ) => {
                format!("{} options", b.name())
            }
            (_, b) => format!("{} options", b.tab_view().unwrap_or(View::Play).name()),
        };
        let (name, _) = self.item_text(items[selected]);
        Effect::Menu(MenuView {
            title,
            rows: items
                .iter()
                .map(|&i| {
                    let (n, v) = self.item_text(i);
                    (n.to_string(), v)
                })
                .collect(),
            selected,
            help_heading: name.to_string(),
            help: self
                .item_help(items[selected])
                .into_iter()
                .map(String::from)
                .collect(),
        })
    }

    /// From a Prefs section back to Prefs' main rows, on the row that opened it.
    fn leave_prefs_section(&mut self) -> Vec<Effect> {
        self.prefs_section = PrefsSection::Main;
        self.selected = PREFS_ROWS
            .iter()
            .position(|&i| i == Item::TouchSection)
            .unwrap_or(0);
        vec![self.menu_view()]
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
        self.apply_item(items[i], delta)
    }

    /// A row of the Looper tab's Recording and input panel stepped by
    /// `delta` (a tap steps +1): the same settings as the looper's options
    /// menu. Only the looper's own rows step here.
    pub fn looper_setting(&mut self, item: Item, delta: i32) -> Vec<Effect> {
        match item {
            Item::LoopSource
            | Item::LineInput
            | Item::LoopMonitor
            | Item::LoopBars
            | Item::LoopLatency
            | Item::LoopLevel => self.apply_item(item, delta),
            _ => Vec::new(),
        }
    }

    /// Step `item` by `delta`'s sign; the settings (and the menu, when it
    /// shows) follow.
    fn apply_item(&mut self, item: Item, delta: i32) -> Vec<Effect> {
        let d = delta.signum();
        let mut fx = Vec::new();
        self.save_dirty = true;
        fn cycle<T: Copy + PartialEq>(all: &[T], now: T, d: i32) -> T {
            let at = all.iter().position(|&x| x == now).unwrap_or(0) as i32;
            all[(at + d).clamp(0, all.len() as i32 - 1) as usize]
        }
        // Appearance steps through these (the theme files can't be borrowed
        // while the settings are).
        let themes: Vec<String> = match item {
            Item::Appearance => self.theme_choices().map(|t| t.0.to_string()).collect(),
            _ => Vec::new(),
        };
        let s = &mut self.settings;
        match item {
            Item::Curve => s.curve = cycle(&Curve::ALL, s.curve, d),
            Item::ScreenVelocity => {
                s.screen_velocity = (s.screen_velocity as i32 + 10 * d).clamp(10, 127) as u8
            }
            Item::LatchMode => {
                s.latch_mode = if s.latch_mode == LatchMode::Toggle {
                    LatchMode::Replace
                } else {
                    LatchMode::Toggle
                };
                self.update_engine();
            }
            Item::Buffer => {
                let p = (s.periods as i32 + d).clamp(2, 3) as u32;
                if p != s.periods {
                    s.periods = p;
                    fx.push(Effect::Audio(p));
                    if !s.loop_latency_set {
                        s.loop_latency = crate::looping::estimated_latency(p);
                        fx.push(Effect::Looper(LoopAction::Command(
                            LoopCommand::SetLatency(s.loop_latency_now()),
                        )));
                    }
                }
            }
            Item::FindButtons => {
                self.open_guide(GuideStep::ALL.to_vec());
                return vec![self.guide_view(), Effect::Show(View::Guide)];
            }
            Item::TouchSection => {
                self.prefs_section = PrefsSection::Touch;
                self.selected = 1;
                return vec![self.menu_view()];
            }
            Item::Appearance => {
                // A theme file from --theme counts as before the first.
                let at = themes.iter().position(|t| *t == s.theme);
                let next = match at {
                    Some(i) => (i as i32 + d).clamp(0, themes.len() as i32 - 1) as usize,
                    None => 0,
                };
                s.theme = themes[next].clone();
            }
            Item::PrefsBack => return self.leave_prefs_section(),
            Item::CalibrateTouch => return vec![Effect::CalibrateTouch],
            Item::TouchLab => return vec![Effect::TouchLab],
            Item::SystemInfo => return self.show(View::System),
            Item::DebugMode => fx.extend(self.set_debug_mode(d > 0)),
            Item::SoundCores => {
                let next = cycle(&SoundCores::ALL, s.sound_cores, d);
                if next != s.sound_cores {
                    s.sound_cores = next;
                    fx.push(Effect::Note(format!(
                        "sound cores: {} from the next start (now {})",
                        next.name(),
                        self.sound_cores_now.name()
                    )));
                }
            }
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
                self.pad_level = cycle(
                    &[PadLevel::Off, PadLevel::Full, PadLevel::Half],
                    self.pad_level,
                    d,
                );
                let on = self.pad_level != PadLevel::Off;
                fx.push(Effect::Light(
                    button::FULL_LEVEL,
                    if on { LIGHT_ON } else { LIGHT_OFF },
                ));
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
                    fx.extend(self.set_loop_source(if on {
                        LoopSource::Line
                    } else {
                        LoopSource::Pads
                    }));
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
                fx.push(Effect::Looper(LoopAction::Command(LoopCommand::SetBars(
                    s.loop_bars,
                ))));
            }
            Item::LoopLatency => {
                s.loop_latency = (s.loop_latency as i32 + 16 * d).clamp(0, 4096) as u32;
                s.loop_latency_set = true;
                fx.push(Effect::Looper(LoopAction::Command(
                    LoopCommand::SetLatency(s.loop_latency_now()),
                )));
            }
            Item::LoopLevel => {
                s.loop_level_db = (s.loop_level_db + d as f32).clamp(-24.0, 6.0);
                fx.push(Effect::Looper(LoopAction::Command(LoopCommand::SetLevel(
                    10f32.powf(s.loop_level_db / 20.0),
                ))));
            }
            Item::Arp(a) => fx.extend(self.step_arp_item(a, d)),
            Item::Synth(a) => fx.extend(self.step_synth_item(a, d)),
        }
        fx.push(Effect::Settings(self.settings.clone()));
        if self.in_menu() {
            fx.push(self.menu_view());
        }
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
        let found = GuideStep::ALL
            .iter()
            .map(|&s| {
                let v = match self.map.get(s) {
                    Some(n) => format!("button {n}"),
                    None if now == Some(s) => "now".to_string(),
                    None => "-".to_string(),
                };
                (s.short().to_string(), v)
            })
            .collect();
        let (heading, mut lines) = match now {
            Some(step) => (
                format!("Press {}", step.label()),
                vec![
                    "MPFree does not know this button's number yet.".to_string(),
                    "Press it once; the number goes in the log for next time.".to_string(),
                    "Not on your MPC, or not sure? Press STOP or tap Skip.".to_string(),
                ],
            ),
            None => (
                "All done".to_string(),
                vec!["Tap Done to play.".to_string()],
            ),
        };
        if !self.guide_note.is_empty() {
            lines.insert(0, self.guide_note.clone());
        }
        Effect::Guide(GuideView {
            heading,
            lines,
            found,
        })
    }

    fn guide_press(&mut self, note: u8) -> Vec<Effect> {
        // STOP skips a button, so the guide can be finished without touch.
        if note == button::STOP {
            return self.guide_button(if self.guide_step().is_some() { 0 } else { 1 });
        }
        let Some(step) = self.guide_step() else {
            return vec![self.guide_view()];
        };
        if let Some(name) = self
            .map
            .name(note)
            .filter(|_| self.map.get(step) != Some(note))
        {
            if self.map.get(step).is_none() || GuideStep::ALL.iter().all(|&s| s.short() != name) {
                self.guide_note = format!("That was {name} (button {note}).");
                return vec![self.guide_view()];
            }
        }
        // Asked again from Prefs: what was pressed replaces what was known.
        self.map.set(step, note);
        self.save_dirty = true;
        self.guide_note = format!("{} is button {note}.", step.short());
        let mut fx = vec![Effect::Note(format!(
            "button guide: {} is button {note}",
            step.short()
        ))];
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
        let mut fx = vec![
            Effect::Note(format!("button guide done: {}", self.map.describe())),
            self.status(),
        ];
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
            let mut fx = vec![Effect::Note(format!(
                "button guide: {} skipped",
                step.short()
            ))];
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
        (
            app.map.rec,
            app.map.overdub,
            app.map.undo,
            app.map.play_start,
        ) = (Some(90), Some(91), Some(92), Some(93));
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

    #[test]
    fn a_pad_layout_feeds_the_arp_its_notes_and_chord_pads_share_notes() {
        use mpfree_arp::scale::pads::{layout, PadLayout, PadSetup};
        use mpfree_arp::scale::{Scale, MAJOR};
        let mut app = ready();
        assert!(!app.arp_plays_notes());
        // Slots without a layout, as before.
        app.pad_down(3, 100);
        assert_eq!(notes(&mut app, 64), [3]);
        app.pad_up(3);
        let setup = PadSetup {
            layout: PadLayout::Chords,
            scale: Some(Scale::new(0, MAJOR)),
            ..PadSetup::default()
        };
        let pads = layout(&setup);
        app.set_pad_layout(Some(pads));
        assert!(app.arp_plays_notes());
        // Pad 1 is C major (root last), pad 3 is E minor: E and G are on both.
        let (one, three) = (pads[0].notes().to_vec(), pads[2].notes().to_vec());
        assert_eq!(one.last(), Some(&48));
        app.pad_down(0, 100);
        let mut heard = notes(&mut app, 64);
        heard.sort_unstable();
        let mut want = one.clone();
        want.sort_unstable();
        assert_eq!(heard, want, "a chord pad presses every note of its chord");
        app.pad_down(2, 100);
        app.pad_up(0);
        for &n in &three {
            assert!(app.eng().active().contains(n), "{n} is still held by pad 3");
        }
        assert!(!app.eng().active().contains(48), "C was only pad 1's");
        assert!(app.active(2) && !app.active(1));
        assert_eq!(app.pad_of_note(48), Some(0));
        // A layout change while pad 3 is down: its pressure still goes to
        // what it pressed, and letting go releases exactly that.
        let held: Vec<u8> = app.pad_held_notes(2).to_vec();
        app.set_pad_layout(Some(layout(&PadSetup {
            layout: PadLayout::Chromatic,
            ..setup
        })));
        assert_eq!(app.pad_held_notes(2), held.as_slice());
        app.pad_up(2);
        assert!(app.pad_held_notes(2).is_empty());
        assert!(app.eng().active().is_empty(), "nothing left held");
        // Back to slots: held pads are let go.
        app.pad_down(5, 100);
        app.set_pad_layout(None);
        assert!(app.eng().active().is_empty());
        assert!(!app.arp_plays_notes());
    }

    fn ons(app: &mut App, frames: u32) -> Vec<(u32, u8, u8)> {
        let mut ev = Events::new();
        app.process(frames, &mut ev);
        ev.as_slice()
            .iter()
            .filter_map(|e| match e.kind {
                EventKind::On { note, velocity } => Some((e.offset, note, velocity)),
                EventKind::Off { .. } => None,
            })
            .collect()
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
        assert!(
            Curve::Hard.apply(120) <= 115,
            "Hard keeps strong hits below the top"
        );
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
        assert!(
            ons(&mut app, 44_100).is_empty(),
            "no repeat without NOTE REPEAT"
        );
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
        assert_eq!(
            ons(&mut app, 11_100).len(),
            3,
            "keeps repeating after release"
        );
        assert!(app.active(4));
        // STOP silences it and keeps the latch; STOP again lets go.
        let fx = press(&mut app, button::STOP);
        assert!(fx.contains(&Effect::StopSounds) && app.repeating());
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

    /// One arp per track: arming another track gives the panel that
    /// track's arp, and a latched arp on the old track keeps running.
    #[test]
    fn another_track_gets_its_own_arp_and_a_latched_one_keeps_running() {
        let mut app = ready();
        app.button(button::SHIFT, true, 0.0);
        press(&mut app, REPEAT);
        app.button(button::SHIFT, false, 0.0);
        app.pad_down(4, 90);
        app.pad_up(4);
        assert!(app.repeating() && app.active(4));
        let fx = app.set_track(2);
        assert!(fx.contains(&Effect::Note(format!(
            "arp: track 3 ({})",
            mode_label(app.arp_mode)
        ))));
        assert_eq!(app.track(), 2);
        // Track 3's panel: not latched, nothing held.
        assert!(!app.repeating() && !app.active(4));
        assert!(ons(&mut app, 11_100).is_empty(), "track 3's arp is quiet");
        // Track 1's arp still holds and plays its latched pad.
        let old = app.arps.engine_at(0);
        assert!(old.latch() != LatchMode::Off && old.active().contains(4));
        let mut ev = Events::new();
        let mut n = 0;
        for _ in 0..100 {
            app.arps.engine_at_mut(0).process(111, &mut ev);
            n += ev
                .as_slice()
                .iter()
                .filter(|e| matches!(e.kind, EventKind::On { .. }))
                .count();
        }
        assert_eq!(n, 3, "still repeating");
        // Back on track 1: its panel and latch again; STOP twice lets go.
        app.set_track(0);
        assert!(app.repeating() && app.active(4));
        press(&mut app, button::STOP);
        press(&mut app, button::STOP);
        assert!(!app.repeating());
        assert!(app.set_track(0).is_empty(), "same track: nothing to do");
    }

    #[test]
    fn stop_ends_the_timed_step() {
        let mut timed = App::new(44_100, true);
        assert_eq!(press(&mut timed, button::STOP), vec![Effect::Quit]);
        let mut held_shift = App::new(44_100, true);
        held_shift.button(button::SHIFT, true, 0.0);
        assert_eq!(press(&mut held_shift, button::STOP), vec![Effect::Quit]);
    }

    /// The Console's chord (Jay, 22:12Z: nothing else may use it): hold
    /// SHIFT, hold ERASE, press TAP TEMPO. Nothing fires on the way in, and
    /// the press is no tempo tap.
    #[test]
    fn shift_erase_tap_tempo_toggles_the_console_and_nothing_else_does() {
        const ERASE: u8 = 52;
        let mut app = ready();
        app.map.learn(mapper::Job::Erase, Some(ERASE));
        let tap = app.map.tap_tempo.unwrap();
        app.button(button::SHIFT, true, 0.0);
        assert!(app.button(ERASE, true, 0.0).is_empty());
        assert_eq!(press(&mut app, tap), vec![Effect::Show(View::Console)]);
        assert!(app.taps.is_empty(), "no tempo tap");
        assert_eq!(press(&mut app, tap), vec![Effect::Show(View::Play)]);
        // Without SHIFT, or once ERASE is let go, TAP TEMPO taps.
        app.button(button::SHIFT, false, 0.0);
        press(&mut app, tap);
        assert_eq!(app.taps.len(), 1);
        app.button(button::SHIFT, true, 0.0);
        app.button(ERASE, false, 0.0);
        assert!(!press(&mut app, tap)
            .iter()
            .any(|e| matches!(e, Effect::Show(View::Console))));
        // SHIFT + STOP is the MPC's now, never the Console.
        assert!(!press(&mut app, button::STOP).contains(&Effect::Show(View::Console)));
    }

    /// Where `item` sits in the menu on screen.
    fn row(app: &App, item: Item) -> usize {
        app.row_of(item).expect("row shown")
    }

    #[test]
    fn prefs_appearance_steps_through_the_themes() {
        let mut app = App::new(44_100, false);
        app.map.menu = Some(MENU);
        app.button(button::SHIFT, true, 0.0);
        press(&mut app, MENU);
        app.button(button::SHIFT, false, 0.0);
        let at = row(&app, Item::Appearance);
        let value = |fx: &[Effect]| match fx.last() {
            Some(Effect::Menu(m)) => m.rows[at].1.clone(),
            _ => panic!("{fx:?}"),
        };
        // One row, its value the theme in use.
        let fx = app.select(at);
        assert_eq!(value(&fx), "Default");
        // + steps to the next theme and hands the GUI the new settings.
        let fx = app.step_item(at, 1);
        assert!(
            fx.iter()
                .any(|e| matches!(e, Effect::Settings(s) if s.theme == "console")),
            "{fx:?}"
        );
        assert_eq!(value(&fx), "Console");
        app.step_item(at, 1);
        assert_eq!(value(&app.step_item(at, 1)), "Spaceship");
        // It stops at the ends.
        assert_eq!(value(&app.step_item(at, 1)), "Spaceship");
        assert_eq!(value(&app.step_item(at, -1)), "Synthwave");
        // It is kept, and comes back at the next start.
        let kept = app.app_settings();
        assert_eq!(kept.get_str("theme"), Some("synthwave"));
        let mut next = App::new(44_100, false);
        next.restore(&kept, &mpfree_store::Settings::default());
        assert_eq!(next.settings.theme, "synthwave");
    }

    #[test]
    fn appearance_offers_the_themes_folder_after_the_built_ins() {
        let mut app = App::new(44_100, false);
        app.theme_files = vec![("sunrise.theme".into(), "Sunrise".into())];
        app.map.menu = Some(MENU);
        app.button(button::SHIFT, true, 0.0);
        press(&mut app, MENU);
        app.button(button::SHIFT, false, 0.0);
        let at = row(&app, Item::Appearance);
        let value = |fx: &[Effect]| match fx.last() {
            Some(Effect::Menu(m)) => m.rows[at].1.clone(),
            _ => panic!("{fx:?}"),
        };
        for _ in 0..3 {
            app.step_item(at, 1);
        }
        // Past Spaceship: the theme file, by its own name, then the end.
        assert_eq!(value(&app.step_item(at, 1)), "Sunrise");
        assert_eq!(app.settings.theme, "sunrise.theme");
        assert_eq!(value(&app.step_item(at, 1)), "Sunrise");
        // Kept by its file name, and restored while the file is there.
        let kept = app.app_settings();
        assert_eq!(kept.get_str("theme"), Some("sunrise.theme"));
        let mut next = App::new(44_100, false);
        next.theme_files = app.theme_files.clone();
        next.restore(&kept, &mpfree_store::Settings::default());
        assert_eq!(next.settings.theme, "sunrise.theme");
        // Gone from the folder: the default theme, and the log says so.
        let mut gone = App::new(44_100, false);
        let notes = gone.restore(&kept, &mpfree_store::Settings::default());
        assert_eq!(gone.settings.theme, "default");
        assert!(
            notes.iter().any(|n| n.contains("theme = sunrise.theme")),
            "{notes:?}"
        );
    }

    #[test]
    fn prefs_sound_cores_is_kept_and_used_from_the_next_start() {
        let mut app = App::new(44_100, false);
        app.map.menu = Some(MENU);
        app.button(button::SHIFT, true, 0.0);
        press(&mut app, MENU);
        app.button(button::SHIFT, false, 0.0);
        // Right after Show developer tools; Touch screen stays the last row.
        let at = row(&app, Item::SoundCores);
        assert_eq!(at, row(&app, Item::DebugMode) + 1);
        assert_eq!(app.items().last(), Some(&Item::TouchSection));
        let value = |fx: &[Effect]| match fx.last() {
            Some(Effect::Menu(m)) => m.rows[at].clone(),
            _ => panic!("{fx:?}"),
        };
        assert_eq!(value(&app.step_item(at, -1)).1, "Shared", "the default");
        let fx = app.step_item(at, 1);
        assert_eq!(
            value(&fx),
            ("Sound cores".to_string(), "Core 3 (next start)".to_string())
        );
        assert!(fx.contains(&Effect::Note(
            "sound cores: core3 from the next start (now shared)".into()
        )));
        assert_eq!(value(&app.step_item(at, 1)).1, "Cores 3 and 2 (next start)");
        assert_eq!(
            value(&app.step_item(at, 1)).1,
            "Cores 3, 2 and 1 (next start)"
        );
        assert_eq!(
            value(&app.step_item(at, 1)).1,
            "Cores 3, 2 and 1 (next start)",
            "the last value"
        );
        assert_eq!(app.settings.sound_cores, SoundCores::Three);
        assert_eq!(
            value(&app.step_item(at, -1)).1,
            "Cores 3 and 2 (next start)"
        );
        assert_eq!(app.settings.sound_cores, SoundCores::Two);
        // Kept, and what the next start runs: then the row says no more.
        let kept = app.app_settings();
        assert_eq!(kept.get_str("sound-cores"), Some("two"));
        let mut next = App::new(44_100, false);
        let notes = next.restore(&kept, &mpfree_store::Settings::default());
        assert_eq!(next.settings.sound_cores, SoundCores::Two);
        assert!(notes[0].ends_with(", sound cores two"), "{notes:?}");
        next.set_sound_cores_now(SoundCores::Two);
        assert_eq!(next.item_text(Item::SoundCores).1, "Cores 3 and 2");
        // A value it doesn't know is left out, and the default stays.
        let mut odd = mpfree_store::Settings::default();
        odd.set_str("sound-cores", "four");
        let mut other = App::new(44_100, false);
        let notes = other.restore(&odd, &mpfree_store::Settings::default());
        assert_eq!(other.settings.sound_cores, SoundCores::Shared);
        assert!(
            notes.iter().any(|n| n.contains("sound-cores = four")),
            "{notes:?}"
        );
    }

    #[test]
    fn appearance_offers_every_built_in_theme() {
        let names: Vec<&str> = THEMES.iter().map(|t| t.0).collect();
        assert_eq!(names, mpfree_ui::theme::BUILTIN);
    }

    #[test]
    fn prefs_keeps_touch_settings_in_their_own_section() {
        let mut app = App::new(44_100, false);
        app.map.menu = Some(MENU);
        let open_prefs = |app: &mut App| {
            app.button(button::SHIFT, true, 0.0);
            press(app, MENU);
            app.button(button::SHIFT, false, 0.0);
        };
        open_prefs(&mut app);
        assert!(
            !app.items().contains(&Item::TouchLab) && !app.items().contains(&Item::ScreenVelocity)
        );
        assert_eq!(app.items().last(), Some(&Item::TouchSection));
        // + on "Touch screen" opens the section on its first setting.
        let fx = app.step_item(row(&app, Item::TouchSection), 1);
        assert!(
            matches!(&fx[..], [Effect::Menu(m)] if m.title == "Prefs · Touch screen" && m.selected == 1
                         && m.rows[1].0 == "Screen pad velocity"),
            "{fx:?}"
        );
        assert_eq!(app.items(), PREFS_TOUCH_ROWS);
        // Its rows work as before.
        app.step_item(row(&app, Item::ScreenVelocity), 1);
        assert_eq!(app.settings.screen_velocity, 110);
        assert_eq!(
            app.step_item(row(&app, Item::TouchLab), 1),
            vec![Effect::TouchLab]
        );
        assert_eq!(
            app.step_item(row(&app, Item::CalibrateTouch), 1),
            vec![Effect::CalibrateTouch]
        );
        // MENU goes back to the main rows, on the row that opened the section.
        let fx = press(&mut app, MENU);
        assert!(
            matches!(&fx[..], [Effect::Menu(m)] if m.title == "Prefs" && m.rows[m.selected].0 == "Touch screen"),
            "{fx:?}"
        );
        assert_eq!(app.view(), View::Prefs);
        // So does its Back row.
        app.step_item(row(&app, Item::TouchSection), 1);
        app.step_item(row(&app, Item::PrefsBack), 1);
        assert_eq!(app.items(), PREFS_ROWS);
        // Prefs opens on its main rows every time.
        app.step_item(row(&app, Item::TouchSection), 1);
        open_prefs(&mut app);
        assert_eq!(app.view(), View::Play);
        open_prefs(&mut app);
        assert_eq!(app.items(), PREFS_ROWS);
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
    fn the_synth_options_pick_the_tuning_its_root_and_a4() {
        use crate::synth::{Builtin, A4_MAX, A4_MIN};
        let mut app = ready();
        app.open_tab(View::Synth);
        press(&mut app, MENU);
        let row = |a: SynthItem| {
            synth_pads::SYNTH_OPTIONS
                .iter()
                .position(|&i| i == Item::Synth(a))
                .unwrap()
        };
        let tuning = |fx: &[Effect]| {
            fx.iter().find_map(|e| match e {
                Effect::SynthPads(p) => Some(p.tuning),
                _ => None,
            })
        };
        // Every built-in tuning in turn, and no further.
        for &want in &Builtin::ALL[1..] {
            let t = tuning(&app.step_item(row(SynthItem::Tuning), 1)).unwrap();
            assert_eq!(t.builtin, want);
        }
        assert_eq!(tuning(&app.step_item(row(SynthItem::Tuning), 1)), None);
        for _ in 0..3 {
            app.step_item(row(SynthItem::Tuning), -1);
        }
        assert_eq!(app.synth_pads.tuning.builtin, Builtin::Just);
        // Root D: the row reads it, now that just intonation uses one.
        app.step_item(row(SynthItem::TuningRoot), 1);
        let fx = app.step_item(row(SynthItem::TuningRoot), 1);
        assert_eq!(tuning(&fx).unwrap().root, 2);
        let Some(Effect::Menu(m)) = fx.last() else {
            panic!("{fx:?}")
        };
        let at = row(SynthItem::Tuning);
        assert_eq!(
            (m.rows[at].1.as_str(), m.rows[at + 1].1.as_str()),
            ("Just (5-limit)", "D")
        );
        // A4 down to 432 Hz, and it stops at the ends.
        for _ in 0..8 {
            app.step_item(row(SynthItem::A4), -1);
        }
        assert_eq!(app.synth_pads.tuning.a4_hz, 432);
        for _ in 0..200 {
            app.step_item(row(SynthItem::A4), 1);
        }
        assert_eq!(app.synth_pads.tuning.a4_hz, A4_MAX);
        assert_eq!(tuning(&app.step_item(row(SynthItem::A4), 1)), None);
        for _ in 0..200 {
            app.step_item(row(SynthItem::A4), -1);
        }
        assert_eq!(app.synth_pads.tuning.a4_hz, A4_MIN);
        // The root stays at B and C; the engine and scale are untouched.
        for _ in 0..20 {
            app.step_item(row(SynthItem::TuningRoot), 1);
        }
        assert_eq!(app.synth_pads.tuning.root, 11);
        assert_eq!(app.synth_pads.engine, crate::synth::Engine::Poly);
        assert_eq!(app.synth_pads.scale, None);
    }

    #[test]
    fn the_synth_options_offer_tuning_files_after_the_built_in_tunings() {
        use crate::synth::Builtin;
        let mut app = ready();
        app.tuning_files = vec![
            ("werckmeister3".to_string(), false),
            ("bohlen-pierce-with-a-long-name".to_string(), true),
        ];
        app.open_tab(View::Synth);
        press(&mut app, MENU);
        let row = |a: SynthItem| {
            synth_pads::SYNTH_OPTIONS
                .iter()
                .position(|&i| i == Item::Synth(a))
                .unwrap()
        };
        let at = row(SynthItem::Tuning);
        let step = |app: &mut App, d: i32| {
            let fx = app.step_item(at, d);
            let tuning = fx.iter().find_map(|e| match e {
                Effect::SynthPads(p) => Some(p.tuning),
                _ => None,
            });
            let rows = match fx.last() {
                Some(Effect::Menu(m)) => m.rows[at..at + 3]
                    .iter()
                    .map(|r| r.1.clone())
                    .collect::<Vec<_>>(),
                _ => Vec::new(),
            };
            (tuning, rows)
        };
        for _ in 1..Builtin::ALL.len() {
            step(&mut app, 1);
        }
        assert_eq!(app.synth_pads.tuning.builtin, Builtin::Equal24);
        // The first file: its root and A4 apply.
        let (t, rows) = step(&mut app, 1);
        assert_eq!(t.unwrap().file, Some(0));
        assert_eq!(rows, ["werckmeister3", "C", "440 Hz"]);
        // The second: its keyboard map sets them, and a long name is cut.
        let (t, rows) = step(&mut app, 1);
        assert_eq!(t.unwrap().file, Some(1));
        assert_eq!(
            rows,
            ["bohlen-pierce-with-…", "set by its .kbm", "set by its .kbm"]
        );
        assert_eq!(step(&mut app, 1).0, None, "no further");
        // Back down past the files to the last built-in tuning.
        step(&mut app, -1);
        let (t, rows) = step(&mut app, -1);
        assert_eq!(t.unwrap().file, None);
        assert_eq!(app.synth_pads.tuning.builtin, Builtin::Equal24);
        assert_eq!(rows, ["24-TET quarter tones", "C (not used)", "440 Hz"]);
    }

    #[test]
    fn the_synth_options_are_kept_between_starts() {
        use crate::synth::{Builtin, Engine};
        use mpfree_arp::scale::pads::{InKeyRows, PadLayout};
        use mpfree_arp::scale::SCALES;
        use mpfree_store::Settings;
        let files = vec![("werckmeister3".to_string(), false)];
        let mut app = ready();
        app.tuning_files = files.clone();
        app.open_tab(View::Synth);
        press(&mut app, MENU);
        let row = |a: SynthItem| {
            synth_pads::SYNTH_OPTIONS
                .iter()
                .position(|&i| i == Item::Synth(a))
                .unwrap()
        };
        // No restore here, so "app" and "map" are handed out once; the
        // Synth tab's options at their defaults are not.
        app.save_dirty = true;
        assert!(
            !app.take_saves().iter().any(|(o, _)| *o == saved::SYNTH),
            "nothing changed yet"
        );
        let mut step = |a: SynthItem, d: i32, times: usize| {
            for _ in 0..times {
                app.step_item(row(a), d);
            }
        };
        step(SynthItem::Engine, 1, 1);
        step(SynthItem::Scale, 1, 2);
        step(SynthItem::Root, 1, 9);
        step(SynthItem::Layout, 1, 2);
        step(SynthItem::Rows, 1, 1);
        step(SynthItem::Tuning, 1, Builtin::ALL.len());
        step(SynthItem::TuningRoot, 1, 2);
        step(SynthItem::A4, -1, 8);
        let saves = app.take_saves();
        assert_eq!(
            saves.iter().map(|(o, _)| *o).collect::<Vec<_>>(),
            vec![saved::SYNTH]
        );
        let kept = saves[0].1.clone();
        for (k, v) in [
            ("engine", "fm"),
            ("scale", SCALES[2].name),
            ("scale-root", "A"),
            ("bank-a", "chords"),
            ("bank-b", "in-key"),
            ("in-key-rows", "fourths"),
            ("tuning", "file werckmeister3"),
            ("tuning-root", "D"),
            ("a4", "432"),
        ] {
            assert_eq!(kept.get_str(k), Some(v), "{k}");
        }
        // The bank playing isn't kept, so switching banks saves nothing.
        app.synth_bank(2);
        assert!(app.take_saves().is_empty());

        // The next start: every option comes back, on bank A.
        let mut next = ready();
        next.tuning_files = files.clone();
        let (notes, fx) = next.restore_synth(&kept);
        let mut want = app.synth_pads;
        want.bank = 0;
        assert_eq!(next.synth_pads, want);
        assert_eq!(next.synth_pads.engine, Engine::Fm);
        assert_eq!(next.synth_pads.rows, InKeyRows::Fourths);
        assert_eq!(next.synth_pads.layout(), PadLayout::Chords);
        assert_eq!(
            (next.pad_scale_pick, next.pad_scale_root),
            (app.pad_scale_pick, app.pad_scale_root)
        );
        assert_eq!(fx, vec![Effect::SynthPads(want)]);
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(
            notes[0].starts_with("saved synth options: FM, scale ")
                && notes[0].ends_with("tuning werckmeister3 (root D, A4 432 Hz)"),
            "{notes:?}"
        );
        let synth_saves = |a: &mut App| {
            a.save_dirty = true;
            a.take_saves()
                .into_iter()
                .filter(|(o, _)| *o == saved::SYNTH)
                .count()
        };
        assert_eq!(synth_saves(&mut next), 0, "what was read is what's kept");

        // A tuning file or scale that's gone, and words not understood, are
        // left out and said; the rest comes back.
        let mut gone = ready();
        let mut odd = kept.clone();
        odd.set_str("engine", "banjo");
        odd.set_str("scale", "No such scale");
        odd.set_i64("a4", 900);
        odd.set_str("bank-c", "sideways");
        let (notes, _) = gone.restore_synth(&odd);
        let left = notes.last().unwrap();
        for k in [
            "engine = banjo",
            "scale = No such scale",
            "tuning = file werckmeister3",
            "a4 = 900",
            "bank-c = sideways",
        ] {
            assert!(left.contains(k), "{k}: {notes:?}");
        }
        let p = gone.synth_pads;
        assert_eq!(
            (p.engine, p.scale, p.tuning.file),
            (Engine::Poly, None, None)
        );
        assert_eq!(
            (p.tuning.a4_hz, p.tuning.root, p.rows),
            (440, 2, InKeyRows::Fourths)
        );
        assert_eq!(
            synth_saves(&mut gone),
            1,
            "the next save writes what is in use"
        );

        // The owner's own scales come back by name; nothing kept, nothing done.
        let mut own = ready();
        own.custom_scales
            .push(("Mine".to_string(), SCALES[3].mask, SCALES[3].down));
        let mut mine = Settings::default();
        mine.set_str("scale", "Mine");
        own.restore_synth(&mine);
        assert_eq!(own.pad_scale_pick, SCALES.len());
        assert!(own.synth_pads.scale.is_some());
        assert_eq!(
            ready().restore_synth(&Settings::default()),
            (vec![], vec![])
        );
    }

    #[test]
    fn the_synth_options_pick_the_engine_the_scale_and_the_pad_layouts() {
        use crate::synth::Engine;
        use mpfree_arp::scale::pads::{InKeyRows, PadLayout};
        let mut app = ready();
        app.open_tab(View::Synth);
        let fx = press(&mut app, MENU);
        let Effect::Menu(m) = &fx[0] else {
            panic!("{fx:?}")
        };
        let names: Vec<&str> = m.rows.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names,
            [
                "Engine",
                "Scale",
                "Scale root",
                "Pad layout",
                "In key rows",
                "Tuning",
                "Tuning root",
                "A4",
                "Note repeat rate",
                "Tempo",
                "Pad level",
                "Latch"
            ]
        );
        assert_eq!(m.rows[0].1, "Poly");
        assert_eq!(m.rows[5].1, "12-TET");
        assert_eq!(m.rows[6].1, "C (not used)");
        assert_eq!(m.rows[7].1, "440 Hz");
        assert_eq!(m.rows[1].1, "Off");
        assert_eq!(m.rows[3].1, "Chromatic (no scale)");
        let row = |a: SynthItem| {
            synth_pads::SYNTH_OPTIONS
                .iter()
                .position(|&i| i == Item::Synth(a))
                .unwrap()
        };
        let pads = |fx: &[Effect]| {
            fx.iter().find_map(|e| match e {
                Effect::SynthPads(p) => Some(*p),
                _ => None,
            })
        };
        // Engine: FM, Drums, Pluck, Wavetable, and no further; then back to Poly.
        for want in [Engine::Fm, Engine::Drums, Engine::Pluck, Engine::Wavetable] {
            let p = pads(&app.step_item(row(SynthItem::Engine), 1)).unwrap();
            assert_eq!(p.engine, want);
        }
        assert_eq!(
            pads(&app.step_item(row(SynthItem::Engine), 1)),
            None,
            "Wavetable is the last"
        );
        for _ in 0..4 {
            app.step_item(row(SynthItem::Engine), -1);
        }
        assert_eq!(app.synth_pads.engine, Engine::Poly);
        // No scale: the pads panel's In key only says so; Chromatic is already on.
        assert!(matches!(
            app.synth_pick_layout(true).as_slice(),
            [Effect::Note(_)]
        ));
        assert!(app.synth_pick_layout(false).is_empty());
        // Scale: Major (Chromatic is skipped; Off stands for it), root D.
        let p = pads(&app.step_item(row(SynthItem::Scale), 1)).unwrap();
        let major = mpfree_arp::scale::by_name("Major").unwrap();
        assert_eq!(p.scale.map(|s| (s.root, s.mask)), Some((0, major.mask)));
        app.step_item(row(SynthItem::Root), 1);
        let fx = app.step_item(row(SynthItem::Root), 1);
        assert_eq!(pads(&fx).unwrap().scale.unwrap().root, 2);
        let Some(Effect::Menu(m)) = fx.last() else {
            panic!("{fx:?}")
        };
        assert_eq!((m.rows[1].1.as_str(), m.rows[2].1.as_str()), ("Major", "D"));
        assert_eq!(m.rows[3].1, "Chromatic (bank A)");
        // Pad layout: bank A to Chord pads; In key rows a fourth apart.
        app.step_item(row(SynthItem::Layout), 1);
        assert_eq!(
            pads(&app.step_item(row(SynthItem::Layout), 1))
                .unwrap()
                .layout(),
            PadLayout::Chords
        );
        assert_eq!(
            pads(&app.step_item(row(SynthItem::Rows), 1)).unwrap().rows,
            InKeyRows::Fourths
        );
        // The pads panel's switch (from Chord pads), and the banks.
        assert_eq!(
            pads(&app.synth_pick_layout(false)).unwrap().layout(),
            PadLayout::Chromatic
        );
        assert_eq!(
            pads(&app.synth_pick_layout(true)).unwrap().layout(),
            PadLayout::InKey
        );
        assert!(app.synth_pick_layout(true).is_empty(), "In key already");
        let p = pads(&app.synth_bank(2)).unwrap();
        assert_eq!((p.bank, p.layout()), (2, PadLayout::Chords));
        assert!(app.synth_bank(2).is_empty());
        assert_eq!(
            pads(&app.synth_bank(0)).unwrap().layout(),
            PadLayout::InKey,
            "bank A kept its layout"
        );
        // Scale Off again: every layout plays as Chromatic.
        let p = pads(&app.step_item(row(SynthItem::Scale), -1)).unwrap();
        assert_eq!(p.scale, None);
    }

    #[test]
    fn the_sample_tab_comes_after_looper_and_takes_the_wheel_the_pads_and_undo() {
        let mut app = ready();
        assert_eq!(
            tabs(false),
            vec![
                View::Play,
                View::Arrange,
                View::Synth,
                View::Looper,
                View::Sample
            ]
        );
        assert_eq!(View::Sample.tab_index(true), Some(4));
        assert_eq!(app.tab(4), vec![Effect::Show(View::Sample)]);
        assert_eq!(app.view(), View::Sample);
        // The wheel zooms; SHIFT + wheel and - / + pick the pad.
        assert_eq!(app.wheel(2), vec![Effect::SampleWheel(2)]);
        app.button(button::SHIFT, true, 0.0);
        assert_eq!(app.wheel(-3), vec![Effect::SamplePad(-1)]);
        // UNDO undoes the Sample page's edits there (SHIFT: redo), and the
        // time engine's takes elsewhere.
        assert_eq!(press(&mut app, 92), vec![Effect::SampleUndo { redo: true }]);
        app.button(button::SHIFT, false, 0.0);
        assert_eq!(
            press(&mut app, 92),
            vec![Effect::SampleUndo { redo: false }]
        );
        app.map.minus = Some(40);
        app.map.plus = Some(41);
        assert_eq!(press(&mut app, 41), vec![Effect::SamplePad(1)]);
        assert_eq!(press(&mut app, 40), vec![Effect::SamplePad(-1)]);
        // MENU opens the options and goes back to the Sample page.
        let fx = press(&mut app, MENU);
        assert!(matches!(&fx[0], Effect::Menu(m) if m.title == "Sample options"));
        press(&mut app, MENU);
        assert_eq!(app.view(), View::Sample);
        app.tab(0);
        assert_eq!(press(&mut app, 92), vec![Effect::Time(TimeAction::Undo)]);
    }

    #[test]
    fn menu_and_shift_menu_open_options_and_prefs() {
        let mut app = ready();
        let fx = press(&mut app, MENU);
        assert_eq!(app.view(), View::Options);
        assert!(
            matches!(&fx[0], Effect::Menu(m) if m.title == "Play options" && m.rows[0].0 == "Note repeat rate")
        );
        // The wheel moves between rows; SHIFT + wheel steps the value.
        let fx = app.wheel(1);
        assert!(matches!(&fx[0], Effect::Menu(m) if m.selected == 1));
        app.wheel(-5);
        app.button(button::SHIFT, true, 0.0);
        let fx = app.wheel(1);
        app.button(button::SHIFT, false, 0.0);
        assert_eq!(app.settings.rate, Rate::SixteenthTriplet);
        assert!(fx
            .iter()
            .any(|e| matches!(e, Effect::Menu(m) if m.selected == 0 && m.rows[0].1 == "1/16T")));
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
        let buffer = row(&app, Item::Buffer);
        assert_eq!(
            app.step_item(buffer, 1)
                .iter()
                .filter(|e| matches!(e, Effect::Audio(_)))
                .count(),
            0
        );
        assert!(app.step_item(buffer, -1).contains(&Effect::Audio(2)));
        // The Debug page's options.
        app.open_tab(View::Debug);
        press(&mut app, MENU);
        assert!(matches!(app.menu_view(), Effect::Menu(m) if m.title == "MIDI log options"));
        // The wheel picks kits outside menus.
        press(&mut app, MENU);
        assert_eq!(app.wheel(-2), vec![Effect::Kit(-2)]);
    }

    #[test]
    fn at_power_on_nothing_asks_and_the_mapper_finds_buttons_at_the_owners_pace() {
        let mut app = App::new(44_100, false);
        assert_eq!(app.map.menu, Some(123));
        assert_eq!(
            (
                app.map.note_repeat,
                app.map.tap_tempo,
                app.map.minus,
                app.map.plus
            ),
            (Some(11), Some(53), Some(55), Some(54))
        );
        let fx = app.start();
        assert!(!fx.iter().any(|e| matches!(e, Effect::Guide(_))));
        assert_eq!(app.view(), View::Play);
        // Only the music tabs until the developer tools are on; Prefs and
        // the Console are not tabs.
        assert_eq!(
            tabs(false),
            vec![
                View::Play,
                View::Arrange,
                View::Synth,
                View::Looper,
                View::Sample
            ]
        );
        // The same names, in the same order, as the bar every preview draws.
        let names: Vec<&str> = tabs(true).iter().map(|v| v.name()).collect();
        assert_eq!(names[..5], mpfree_ui::widgets::APP_TABS[..]);
        assert_eq!(names[5], mpfree_ui::widgets::DEV_TAB);
        assert_eq!(app.tab(5), vec![]);
        app.open_tab(View::Prefs);
        let row = app
            .items()
            .iter()
            .position(|&i| i == Item::MapControls)
            .unwrap();
        let fx = app.step_item(row, 1);
        assert!(app.settings.debug_mode);
        assert_eq!(app.view(), View::Map);
        assert!(fx.contains(&Effect::Note("developer tools: on".into())));
        assert!(fx
            .iter()
            .any(|e| matches!(e, Effect::Map(m) if m.rows[2].value == "button 81" && m.left > 20)));
        // Tap REC: MPFree listens; STOP stops listening without silencing.
        app.map_row(0);
        let fx = press(&mut app, button::STOP);
        assert!(!fx.contains(&Effect::StopSounds), "{fx:?}");
        assert!(fx
            .iter()
            .any(|e| matches!(e, Effect::Map(m) if !m.listening)));
        app.map_row(0);
        let fx = press(&mut app, 73);
        assert!(fx.contains(&Effect::Note("map: REC = button 73".into())));
        assert_eq!(app.map.rec, Some(73));
        // The wheel picks PAD BANK A/E, + listens, and it works at once.
        let at = mapper::CONTROLS
            .iter()
            .position(|c| c.name == "PAD BANK A/E")
            .unwrap() as i32;
        app.wheel(at - app.mapper.selected as i32);
        press(&mut app, button::PLUS);
        let fx = press(&mut app, 35);
        assert!(
            fx.contains(&Effect::Bank {
                button: u8::MAX,
                shift: false
            }),
            "{fx:?}"
        );
        // - skips the next one; leaving the page writes the summary.
        let fx = press(&mut app, button::MINUS);
        assert!(fx
            .iter()
            .any(|e| matches!(e, Effect::Note(n) if n.ends_with("skipped"))));
        let fx = app.open_tab(View::Play);
        assert!(fx.iter().any(|e| matches!(e, Effect::Note(n) if n.starts_with("map summary: ")
                                             && n.contains("REC button 73") && n.contains("PAD BANK A/E button 35"))));
        assert_eq!(
            press(&mut app, 73),
            vec![Effect::Time(TimeAction::Transport(Command::Record))]
        );
        assert_eq!(
            press(&mut app, 35),
            vec![Effect::Bank {
                button: 0,
                shift: false
            }]
        );
        app.button(button::SHIFT, true, 0.0);
        assert_eq!(
            press(&mut app, 35),
            vec![Effect::Bank {
                button: 0,
                shift: true
            }]
        );
        app.button(button::SHIFT, false, 0.0);
        // Debug mode adds the Debug tab, whose pages SHIFT + wheel steps through.
        assert_eq!(tabs(true)[5], View::Debug);
        app.tab(5);
        assert_eq!(app.view(), View::Debug);
        app.button(button::SHIFT, true, 0.0);
        app.wheel(1);
        assert_eq!(app.view(), View::Perf);
        app.wheel(1);
        assert_eq!(app.view(), View::Engines);
        app.wheel(1);
        assert_eq!(app.view(), View::Procs);
        app.wheel(1);
        assert_eq!(app.view(), View::Map);
        app.button(button::SHIFT, false, 0.0);
        assert_eq!(app.debug_page(1), vec![Effect::Show(View::Perf)]);
        assert_eq!(app.debug_page(3), vec![Effect::Show(View::Procs)]);
        assert_eq!(View::Engines.tab_view(), Some(View::Debug));
        assert_eq!(View::Procs.tab_view(), Some(View::Debug));
        // Their options menus are titled after the page.
        app.debug_page(2);
        match app.button(MENU, true, 0.0).into_iter().find_map(|e| {
            if let Effect::Menu(m) = e {
                Some(m)
            } else {
                None
            }
        }) {
            Some(m) => assert_eq!(m.title, "CPU by engine options"),
            None => panic!("MENU opens the options"),
        }
        app.button(MENU, false, 0.0);
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
        assert!(
            fx.contains(&source(LoopSource::Pads)) && fx.contains(&latency(0)),
            "{fx:?}"
        );
        assert!(
            !fx.iter()
                .any(|e| matches!(e, Effect::Looper(LoopAction::LineInput(_)))),
            "nothing opens at start"
        );
        // The Looper tab's options: Record from, then Line input.
        app.open_tab(View::Looper);
        press(&mut app, MENU);
        assert_eq!(app.view(), View::Options);
        assert_eq!(app.items()[..2], [Item::LoopSource, Item::LineInput]);
        let fx = app.step_item(1, 1);
        assert!(fx.contains(&Effect::Looper(LoopAction::LineInput(true))));
        assert!(
            fx.contains(&source(LoopSource::Line)) && fx.contains(&latency(512)),
            "{fx:?}"
        );
        // REC on the Looper tab is the looper's Record.
        press(&mut app, MENU);
        assert_eq!(app.view(), View::Looper);
        assert_eq!(
            press(&mut app, 90),
            vec![Effect::Looper(LoopAction::Command(LoopCommand::Record))]
        );
        // The input fails to open: Off again, and the pads.
        let fx = app.line_input_off();
        assert!(
            fx.contains(&source(LoopSource::Pads)) && fx.contains(&latency(0)),
            "{fx:?}"
        );
        assert!(!app.settings.line_input);
        // Record from can pick the line input by hand; Pads needs no latency.
        press(&mut app, MENU);
        let fx = app.step_item(0, 1);
        assert!(
            fx.contains(&source(LoopSource::Line)) && fx.contains(&latency(512)),
            "{fx:?}"
        );
        let fx = app.step_item(0, -1);
        assert!(
            fx.contains(&source(LoopSource::Pads)) && fx.contains(&latency(0)),
            "{fx:?}"
        );
        assert!(
            app.step_item(0, -1)
                .iter()
                .all(|e| !matches!(e, Effect::Looper(_))),
            "already the pads"
        );
    }

    #[test]
    fn pad_banks_pick_e_to_h_with_shift_or_a_second_press() {
        assert_eq!(bank_target(0, 1, false), 1);
        assert_eq!(bank_target(1, 1, false), 5, "B again: F");
        assert_eq!(bank_target(5, 1, false), 1, "F, then B: B");
        assert_eq!(bank_target(5, 0, false), 0);
        assert_eq!(bank_target(2, 3, true), 7, "SHIFT + D/H: H");
        assert_eq!(
            (bank_letter(0), bank_letter(7), bank_letter(9)),
            ('A', 'H', 'B')
        );
    }

    #[test]
    fn the_guide_learns_buttons_by_name() {
        let mut app = ready();
        app.map = ButtonMap::default();
        app.open_tab(View::Prefs);
        let fx = app.step_item(row(&app, Item::FindButtons), 1);
        assert_eq!(app.view(), View::Guide);
        assert!(fx
            .iter()
            .any(|e| matches!(e, Effect::Guide(g) if g.heading == "Press MENU")));
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
        let fx = app.step_item(row(&app, Item::FindButtons), 1);
        assert_eq!(app.view(), View::Guide);
        assert!(fx.contains(&Effect::Show(View::Guide)));
        assert_eq!(app.guide_steps.len(), 9);
    }

    #[test]
    fn stop_silences_a_latched_arp_and_keeps_the_latch_until_stop_again() {
        let mut app = ready();
        held(&mut app, button::SHIFT, |a| press(a, REPEAT));
        assert!(app.latched, "SHIFT + NOTE REPEAT latches");
        app.pad_down(2, 100);
        app.pad_up(2);
        assert!(!notes(&mut app, 4 * 5513).is_empty(), "latched, it repeats");
        // First STOP: silence, latch kept.
        let fx = press(&mut app, button::STOP);
        assert!(fx.contains(&Effect::StopSounds));
        assert!(fx.contains(&Effect::Note(
            "STOP: arp silenced, latch kept (STOP again clears it)".into()
        )));
        assert!(app.latched && app.eng().hushed() && app.active(2));
        assert!(notes(&mut app, 4 * 5513).is_empty(), "silent");
        // PLAY carries on with the same latch.
        press(&mut app, button::PLAY);
        assert!(!app.eng().hushed());
        assert!(notes(&mut app, 4 * 5513).contains(&2));
        // STOP, then STOP again: the latch goes.
        press(&mut app, button::STOP);
        let fx = press(&mut app, button::STOP);
        assert!(fx.contains(&Effect::Note("STOP: all sounds stopped".into())));
        assert!(!app.latched && !app.active(2));
    }

    #[test]
    fn turning_swing_takes_it_from_the_transport() {
        let mut app = ready();
        assert!(app.swing_from_transport, "follows the transport at first");
        app.button(REPEAT, true, 0.0);
        let mut fx = Vec::new();
        for _ in 0..8 {
            fx.extend(app.qlink(2, 1, None).unwrap());
        }
        assert!(!app.swing_from_transport, "turned: the arp's own swing");
        assert!(app.arp.swing > 50);
        assert!(fx
            .iter()
            .any(|e| matches!(e, Effect::Note(n) if n.starts_with("arp swing from: arp"))));
        // Other knobs leave it alone.
        let mut fresh = ready();
        fresh.button(REPEAT, true, 0.0);
        for _ in 0..8 {
            fresh.qlink(1, 1, None);
        }
        assert!(fresh.swing_from_transport);
    }

    #[test]
    fn note_repeat_turns_on_the_arp_mode_the_panel_picked() {
        let mut app = ready();
        assert_eq!(app.eng().settings().mode, Mode::Direct);
        // Hold NOTE REPEAT: Repeat, and - / + pick the mode, the wheel the order.
        app.button(REPEAT, true, 0.0);
        assert_eq!(app.eng().settings().mode, Mode::Repeat);
        press(&mut app, button::PLUS);
        assert_eq!(app.eng().settings().mode, Mode::Arp);
        app.wheel(1);
        assert_eq!(app.eng().settings().order, Order::ALL[1]);
        let fx = app.button(REPEAT, false, 0.0);
        assert!(fx.contains(&Effect::Light(REPEAT, LIGHT_OFF)));
        assert_eq!(app.eng().settings().mode, Mode::Direct);
        assert_eq!(
            app.wheel(1),
            vec![Effect::Kit(1)],
            "the wheel is the kit again"
        );
        // Held again, a while later: the arp, not Repeat.
        app.pad_down(0, 100);
        app.pad_down(4, 100);
        let fx = app.button(REPEAT, true, 10.0);
        assert!(
            !fx.contains(&Effect::Note("Latch on".into())),
            "too slow for a double press"
        );
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
        assert_eq!(
            ons(&mut app, 11_100).len(),
            3,
            "keeps repeating after release"
        );
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
        assert_eq!(app.eng().settings().mode, Mode::Repeat);
        assert!(app.generating());
        // - / + walk its tabs, Direct included.
        let fx = press(&mut app, button::MINUS);
        assert_eq!(app.eng().settings().mode, Mode::Direct);
        assert!(fx.contains(&Effect::Light(REPEAT, LIGHT_OFF)));
        press(&mut app, button::PLUS);
        press(&mut app, button::PLUS);
        press(&mut app, button::PLUS);
        assert_eq!(app.eng().settings().mode, Mode::Pattern);
        // The wheel picks a pattern, the Q-Links rate, gate, swing and octaves.
        app.wheel(2);
        assert_eq!(app.pattern_index, 2);
        assert_eq!(*app.eng().pattern(), app.patterns[2].1);
        assert_eq!(
            app.qlink(1, 5, None),
            Some(vec![
                Effect::Status(app.status_now()),
                Effect::Light(REPEAT, LIGHT_ON)
            ])
        );
        assert_eq!(app.eng().settings().gate, 60);
        assert_eq!(
            app.qlink(3, 3, None),
            Some(Vec::new()),
            "octaves need a longer turn"
        );
        app.qlink(3, 1, None);
        assert_eq!(app.eng().settings().octaves, 2);
        app.qlink(0, 3, None);
        assert_eq!(app.settings.rate, Rate::SixteenthTriplet);
        app.qlink(2, 0, Some(127));
        assert_eq!(
            app.eng().settings().swing,
            75,
            "absolute positions without Akai's session"
        );
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
        use mpfree_arp::scale::{Scale, Snap, DORIAN, PHRYGIAN};
        let mut app = ready();
        app.toggle_panel();
        let fx = press(&mut app, MENU);
        let Some(Effect::Menu(m)) = fx.into_iter().find(|e| matches!(e, Effect::Menu(_))) else {
            panic!("no menu")
        };
        assert_eq!(m.title, "Arp options");
        assert_eq!(
            m.rows.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(),
            [
                "Tempo",
                "Accent level",
                "Scale",
                "Scale root",
                "Out-of-key notes",
                "Chord on each pad",
                "Scale run",
                "Chords take turns",
                "Strum",
                "New notes restart",
                "Swing from"
            ]
        );
        assert_eq!(m.rows[10].1, "Transport");
        assert_eq!(
            (
                m.rows[5].1.as_str(),
                m.rows[6].1.as_str(),
                m.rows[7].1.as_str()
            ),
            ("Off", "Off", "Off")
        );
        assert_eq!(m.rows[4].1, "Note below");
        assert_eq!(
            m.rows[2].1, "Off",
            "no scale until one is picked: drum kits stay as they are"
        );
        assert!(app.generating(), "the arp plays on under its options");
        let row =
            |app: &App, a: ArpItem| app.items().iter().position(|&i| i == Item::Arp(a)).unwrap();
        // Accent level steps by 8, down from 127.
        let fx = app.step_item(row(&app, ArpItem::Accent), -1);
        assert_eq!(app.eng().settings().accent, 119);
        assert!(fx.contains(&Effect::Note("arp accent level: 119".to_string())));
        // Scale: Off, then the named scales; the root moves it.
        app.step_item(row(&app, ArpItem::Scale), 1);
        app.step_item(row(&app, ArpItem::Scale), 1);
        assert_eq!(app.eng().settings().scale, Some(Scale::new(0, DORIAN)));
        app.step_item(row(&app, ArpItem::Root), 1);
        let fx = app.step_item(row(&app, ArpItem::Root), 1);
        assert_eq!(app.eng().settings().scale, Some(Scale::new(2, DORIAN)));
        assert!(fx.contains(&Effect::Note("arp scale: D Dorian".to_string())));
        assert!(
            matches!(fx.last(), Some(Effect::Menu(m)) if m.rows[3].1 == "D" && m.rows[2].1 == "Dorian")
        );
        app.step_item(row(&app, ArpItem::Scale), 1);
        assert_eq!(app.eng().settings().scale, Some(Scale::new(2, PHRYGIAN)));
        // A scale with other notes coming down keeps them when the root moves.
        let classical = mpfree_arp::scale::SCALES
            .iter()
            .position(|d| d.name == "Mel. minor classical")
            .unwrap();
        for _ in 3..classical {
            app.step_item(row(&app, ArpItem::Scale), 1);
        }
        app.step_item(row(&app, ArpItem::Root), 1);
        let s = app.eng().settings().scale.unwrap();
        assert_eq!(
            (s.root, s.mask, s.down),
            (
                3,
                mpfree_arp::scale::MELODIC_MINOR,
                mpfree_arp::scale::MINOR
            )
        );
        app.step_item(row(&app, ArpItem::Root), -1);
        // Out-of-key notes: below, above, nearest, not played.
        let fx = app.step_item(row(&app, ArpItem::Snap), 1);
        assert_eq!(app.eng().settings().snap, Snap::Up);
        assert!(fx.contains(&Effect::Note(
            "arp out-of-key notes: note above".to_string()
        )));
        for _ in 0..5 {
            app.step_item(row(&app, ArpItem::Snap), 1);
        }
        assert_eq!(app.eng().settings().snap, Snap::Skip);
        // A chord on each pad, and a scale run.
        app.step_item(row(&app, ArpItem::Chord), 1);
        let fx = app.step_item(row(&app, ArpItem::Chord), 1);
        assert_eq!(app.eng().settings().chord, mpfree_arp::Chord::Triad);
        assert!(fx.contains(&Effect::Note(
            "arp chord on each pad: triad (1 3 5)".to_string()
        )));
        app.step_item(row(&app, ArpItem::Run), 1);
        assert!(app.eng().settings().run);
        app.step_item(row(&app, ArpItem::Run), -1);
        // Held chords take turns: a progression.
        app.step_item(row(&app, ArpItem::Turns), 1);
        app.step_item(row(&app, ArpItem::Turns), 1);
        let fx = app.step_item(row(&app, ArpItem::Turns), 1);
        assert_eq!(app.eng().settings().turns, mpfree_arp::Turns::Bar);
        assert!(fx.contains(&Effect::Note(
            "arp chords take turns: every bar".to_string()
        )));
        for _ in 0..4 {
            app.step_item(row(&app, ArpItem::Turns), -1);
        }
        assert_eq!(app.eng().settings().turns, mpfree_arp::Turns::Off);
        app.step_item(row(&app, ArpItem::Chord), -5);
        app.step_item(row(&app, ArpItem::Chord), -1);
        assert_eq!(app.eng().settings().chord, mpfree_arp::Chord::Off);
        // Off keeps the root for next time.
        for _ in 0..classical {
            app.step_item(row(&app, ArpItem::Scale), -1);
        }
        assert_eq!(app.eng().settings().scale, None);
        assert_eq!(
            app.step_item(row(&app, ArpItem::Scale), -1)
                .iter()
                .filter(|e| matches!(e, Effect::Note(_)))
                .count(),
            0,
            "nothing below Off"
        );
        app.step_item(row(&app, ArpItem::Scale), 1);
        assert_eq!(
            app.eng().settings().scale,
            Some(Scale::new(2, mpfree_arp::scale::MAJOR))
        );
        // Strum in tens of percent; New notes restart on and off.
        app.step_item(row(&app, ArpItem::Strum), 1);
        assert_eq!(app.eng().settings().strum, 10);
        app.step_item(row(&app, ArpItem::Retrigger), 1);
        assert!(app.eng().settings().retrigger);
        // The panel's own settings leave them alone.
        app.arp_action(ArpAction::Settings(mpfree_arp::Settings {
            gate: 80,
            ..app.arp
        }));
        let s = app.eng().settings();
        assert_eq!(
            (s.accent, s.strum, s.retrigger, s.gate),
            (119, 10, true, 80)
        );
        // MENU goes back to the panel; the Play page's options are as before.
        press(&mut app, MENU);
        assert_eq!(app.view(), View::Arp);
        app.toggle_panel();
        press(&mut app, MENU);
        assert!(
            matches!(app.menu_view(), Effect::Menu(m) if m.title == "Play options" && m.rows.len() == 4)
        );
    }

    #[test]
    fn the_arp_follows_the_transport_clock_and_its_swing_if_asked() {
        // NOTE REPEAT held on pad 1 at 1/16, 120 BPM: a 16th is 5512.5 frames.
        let gaps = |from_transport: bool, swing: Option<(u32, u8)>, synced: bool| {
            let mut app = ready();
            if !from_transport {
                app.toggle_panel();
                press(&mut app, MENU);
                let row = app
                    .items()
                    .iter()
                    .position(|&i| i == Item::Arp(ArpItem::SwingFrom))
                    .unwrap();
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
                ons.extend(
                    ev.as_slice()
                        .iter()
                        .filter(|e| matches!(e.kind, EventKind::On { .. }))
                        .map(|e| i * 128 + u64::from(e.offset)),
                );
            }
            ons.windows(2)
                .skip(1)
                .take(4)
                .map(|w| w[1] - w[0])
                .collect::<Vec<_>>()
        };
        let straight = gaps(true, None, true);
        assert!(
            straight.iter().all(|&g| g.abs_diff(5512) <= 1),
            "{straight:?}"
        );
        // The transport swings 1/16s at 75 %: long, short, long, short.
        let swung = gaps(true, Some((240, 75)), true);
        assert!(
            swung
                .iter()
                .all(|&g| g.abs_diff(8269) <= 1 || g.abs_diff(2756) <= 1),
            "{swung:?}"
        );
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
        assert!(
            ons(&mut app, 20_000).is_empty(),
            "not latched: nothing after letting go"
        );
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
        assert!(app.latched, "the first STOP keeps the latch");
        press(&mut app, button::STOP);
        assert!(!app.latched);
    }

    #[test]
    fn the_workspace_records_on_the_pads() {
        let mut app = ready();
        app.toggle_panel();
        let fx = app.arp_action(ArpAction::Button(PatternButton::Record));
        assert_eq!(app.view(), View::Pattern);
        assert!(fx
            .iter()
            .any(|e| matches!(e, Effect::Workspace(w) if w.open && !w.recording)));
        assert!(fx
            .iter()
            .any(|e| matches!(e, Effect::Workspace(w) if !w.open && w.recording)));
        assert_eq!(
            app.eng().settings().mode,
            Mode::Direct,
            "pads sound as pressed while recording"
        );
        let len = app.recorder.unwrap().length();
        assert_eq!(len, 8, "at least 8 steps, though Straight has 1");
        // Pad 1 writes "1", pad 16 a rest, pad 13 the next note.
        let fx = app.pad_down(0, 100);
        assert!(fx
            .iter()
            .any(|e| matches!(e, Effect::Recorded(p, 1) if p.len() == len)));
        app.pad_up(0);
        assert_eq!(ons(&mut app, 128).len(), 1, "the pad still sounds");
        app.pad_down(15, 100);
        app.pad_up(15);
        let mut fx = Vec::new();
        for _ in 2..len {
            fx = app.pad_down(12, 120);
            app.pad_up(12);
        }
        assert!(
            fx.iter()
                .any(|e| matches!(e, Effect::Workspace(w) if !w.recording)),
            "full: recording stops"
        );
        assert_eq!(
            app.pattern.steps()[0],
            mpfree_arp::Step::play(mpfree_arp::Pick::Index(1))
        );
        assert_eq!(app.pattern.steps()[1], mpfree_arp::Step::Rest);
        assert!(
            matches!(
                app.pattern.steps()[2],
                mpfree_arp::Step::Play { accent: true, .. }
            ),
            "a hard hit"
        );
        assert_eq!(
            app.eng().settings().mode,
            Mode::Repeat,
            "the panel's mode again"
        );
        // Save keeps it in the list (in memory); Back returns to the panel.
        let p = app.pattern;
        let fx = app.workspace_action(WsAction::Save(p));
        assert!(fx
            .iter()
            .any(|e| matches!(e, Effect::Patterns(l) if l[0].1 == p)));
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
        assert!(fx
            .iter()
            .any(|e| matches!(e, Effect::Workspace(w) if w.free_time && !w.recording)));
        let fx = app.workspace_action(WsAction::Record(true));
        assert!(fx
            .iter()
            .any(|e| matches!(e, Effect::Note(n) if n.starts_with("Free time plays the pattern"))));
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
        assert!(fx
            .iter()
            .any(|e| matches!(e, Effect::Workspace(w) if w.free_time && w.recording)));
        assert_eq!(app.recorder.unwrap().mode(), RecordMode::FreeTime);
        assert_eq!(
            app.eng().settings().mode,
            Mode::Pattern,
            "the pattern keeps playing"
        );
        let _ = ons(&mut app, 3 * 5513 + 400);
        // Pad 2 (the 2nd held note), hit hard, lands on the step nearest now.
        let at = (app.eng_mut().position().round() as usize) % 4;
        let fx = app.pad_down(1, 127);
        assert!(
            fx.iter()
                .any(|e| matches!(e, Effect::Recorded(_, i) if *i == at)),
            "{fx:?}"
        );
        assert!(matches!(
            app.pattern.steps()[at],
            Step::Play {
                pick: Pick::Index(2),
                accent: true,
                ..
            }
        ));
        assert_eq!(
            *app.eng().pattern(),
            app.pattern,
            "the engine plays it at once"
        );
        assert!(app.pad_up(1).is_empty(), "a short press ties nothing");
        let active: Vec<u8> = (0..16)
            .filter(|&s| app.active(s))
            .map(|s| s as u8)
            .collect();
        assert_eq!(active, [0, 4], "record pads don't change the latched chord");
        // Held across two more steps: ties after it.
        let at = (app.eng_mut().position().round() as usize) % 4;
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
        assert_eq!(app.eng().settings().mode, Mode::Direct);
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
        let fx = app.load_patterns(
            "# mine\nSkip = 1 . 3 2\nstraight = x x .\n[scales]\nBright minor = 1 2 b3 4 5 6 7\n",
        );
        assert!(
            fx.contains(&Effect::Note(
                "arp patterns file: 1 added, 1 built-in ones replaced, 1 scales of your own".into()
            )),
            "{fx:?}"
        );
        assert_eq!(app.patterns.last().unwrap().0, "Skip");
        assert_eq!(
            app.patterns[0],
            ("Straight".to_string(), Pattern::parse("x x .").unwrap()),
            "keeps its place"
        );
        assert_eq!(
            *app.eng().pattern(),
            Pattern::parse("x x .").unwrap(),
            "the pattern in use is reloaded"
        );
        let text = app.library_text();
        let lib = mpfree_arp::Library::parse(&text).unwrap();
        let names: Vec<&str> = lib.iter().map(|(n, _)| n).collect();
        assert_eq!(
            names,
            ["Straight", "Skip"],
            "only patterns that differ from the built-ins"
        );
        // A bad file is not used at all.
        let before = app.patterns.clone();
        let fx = app.load_patterns("Fine = 1 2\nBad = 1 z\nNo equals sign\n");
        assert_eq!(app.patterns, before);
        assert!(fx.contains(&Effect::Note(
            "arp patterns file not used (2 bad lines); the built-in patterns stay".into()
        )));
        // Save logs a library line; it saves the library only when the store saves.
        app.toggle_panel();
        app.arp_action(ArpAction::Button(PatternButton::Edit));
        let p = Pattern::parse("1 2 . 3").unwrap();
        let fx = app.workspace_action(WsAction::Save(p));
        assert!(fx.contains(&Effect::Note(
            "arp-pattern-saved: Straight = 1 2 . 3".into()
        )));
        assert!(!fx.iter().any(|e| matches!(e, Effect::SavePatterns(_))));
        app.pattern_file = true;
        let fx = app.workspace_action(WsAction::Save(p));
        let saved = fx
            .iter()
            .find_map(|e| match e {
                Effect::SavePatterns(t) => Some(t.clone()),
                _ => None,
            })
            .unwrap();
        assert!(
            saved.contains("Straight = 1 2 . 3\n") && saved.contains("Skip     = 1 . 3 2\n"),
            "{saved}"
        );
        assert!(
            saved.contains("[scales]\nBright minor = 0 2 3 5 7 9 11\n"),
            "{saved}"
        );
        // The owner's scale comes after the built-in ones in the Scale row.
        app.workspace_action(WsAction::Back);
        press(&mut app, MENU);
        assert_eq!(app.view(), View::Options);
        let row = app
            .items()
            .iter()
            .position(|&i| i == Item::Arp(ArpItem::Scale))
            .unwrap();
        for _ in 1..mpfree_arp::scale::SCALES.len() {
            app.step_item(row, 1);
        }
        let fx = app.step_item(row, 1);
        assert_eq!(
            app.eng().settings().scale,
            Some(mpfree_arp::Scale::new(0, mpfree_arp::scale::MELODIC_MINOR))
        );
        assert!(
            fx.contains(&Effect::Note("arp scale: C Bright minor".into())),
            "{fx:?}"
        );
    }

    #[test]
    fn leaving_the_workspace_stops_recording_and_edits_reach_the_engine() {
        let mut app = ready();
        app.toggle_panel();
        app.arp_action(ArpAction::Button(PatternButton::Edit));
        let edited = Pattern::parse("1 . 2! _").unwrap();
        app.workspace_action(WsAction::Changed(edited));
        assert_eq!(*app.eng().pattern(), edited);
        assert!(matches!(app.arp_view(), Effect::Arp(v) if v.pattern_changed));
        app.workspace_action(WsAction::Record(true));
        assert!(app.recorder.is_some());
        let fx = app.tab(1);
        assert!(app.recorder.is_none());
        assert!(fx
            .iter()
            .any(|e| matches!(e, Effect::Workspace(w) if !w.recording)));
        assert_eq!(
            app.workspace_action(WsAction::Back),
            Vec::new(),
            "only in the workspace"
        );
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
        assert_eq!(
            app.tick(20.61),
            Some(Effect::Light(button::TAP_TEMPO, LIGHT_ON))
        );
        assert_eq!(app.tick(20.62), None);
        assert_eq!(
            app.tick(20.9),
            Some(Effect::Light(button::TAP_TEMPO, LIGHT_OFF))
        );
    }

    #[test]
    fn play_held_while_rolling_is_a_fill_and_twice_latches_it() {
        let mut app = ready();
        // Stopped, PLAY plays.
        assert_eq!(
            press(&mut app, button::PLAY),
            vec![Effect::Time(TimeAction::Transport(Command::Play))]
        );
        app.set_rolling(true);
        let fills = |fx: &[Effect]| {
            fx.iter()
                .filter_map(|e| match e {
                    Effect::Fill(on) => Some(*on),
                    _ => None,
                })
                .collect::<Vec<bool>>()
        };
        // Held: a fill until PLAY lifts.
        let fx = app.button(button::PLAY, true, 10.0);
        assert_eq!(fills(&fx), [true]);
        assert!(!fx.contains(&Effect::Time(TimeAction::Transport(Command::Play))));
        assert_eq!(app.button(button::PLAY, false, 10.1), [Effect::Fill(false)]);
        // Pressed again at once: latched, and the lift leaves it on.
        assert_eq!(fills(&app.button(button::PLAY, true, 10.3)), [true]);
        assert!(app.button(button::PLAY, false, 10.4).is_empty());
        // Once more: off.
        assert_eq!(fills(&app.button(button::PLAY, true, 12.0)), [false]);
        assert!(app.button(button::PLAY, false, 12.1).is_empty());
        // Latched, STOP lets it go with the transport.
        app.button(button::PLAY, true, 20.0);
        app.button(button::PLAY, false, 20.1);
        app.button(button::PLAY, true, 20.2);
        app.button(button::PLAY, false, 20.3);
        let fx = app.button(button::STOP, true, 21.0);
        assert_eq!(fills(&fx), [false]);
        assert!(fx.contains(&Effect::Time(TimeAction::Transport(Command::Stop))));
    }

    /// TRACK MUTE (control map decision 5), once Map controls has found
    /// it: the pads mute tracks over any page, SHIFT + TRACK MUTE the
    /// kit's pads; the same press leaves, the other switches. The page
    /// stays as it was, and the button's light and the hint follow.
    #[test]
    fn track_mute_turns_the_pads_into_mute_keys_over_any_page() {
        const SHIFT: u8 = button::SHIFT;
        const TRACK_MUTE: u8 = 68;
        let mut app = ready();
        assert!(press(&mut app, TRACK_MUTE).is_empty(), "not found yet");
        app.map.learn(mapper::Job::TrackMute, Some(TRACK_MUTE));
        assert_eq!(app.map.name(TRACK_MUTE), Some("TRACK MUTE"));
        app.open_tab(View::Looper);
        let fx = press(&mut app, TRACK_MUTE);
        assert_eq!(app.mute_keys(), MuteKeys::Tracks);
        assert_eq!(fx[0], Effect::Light(TRACK_MUTE, LIGHT_ON));
        assert!(!fx.iter().any(|e| matches!(e, Effect::Show(_))));
        assert_eq!(app.view(), View::Looper, "the page stays");
        assert_eq!(app.status_now().mute_keys, MuteKeys::Tracks);
        // SHIFT + TRACK MUTE: the kit's pads; again: the pads play.
        held(&mut app, SHIFT, |a| press(a, TRACK_MUTE));
        assert_eq!(app.mute_keys(), MuteKeys::Pads);
        let fx = held(&mut app, SHIFT, |a| press(a, TRACK_MUTE));
        assert_eq!(app.mute_keys(), MuteKeys::Off);
        assert_eq!(fx[0], Effect::Light(TRACK_MUTE, LIGHT_OFF));
        assert!(fx.contains(&Effect::Note("TRACK MUTE: the pads play again".into())));
        press(&mut app, TRACK_MUTE);
        press(&mut app, TRACK_MUTE);
        assert_eq!(app.mute_keys(), MuteKeys::Off);
    }

    #[test]
    fn the_transport_buttons_drive_the_time_engine() {
        let mut app = ready();
        let time = |a: TimeAction| Effect::Time(a);
        assert_eq!(
            press(&mut app, button::PLAY),
            vec![time(TimeAction::Transport(Command::Play))]
        );
        assert_eq!(
            press(&mut app, 93),
            vec![time(TimeAction::Transport(Command::PlayStart))]
        );
        assert_eq!(
            press(&mut app, 90),
            vec![time(TimeAction::Transport(Command::Record))]
        );
        assert_eq!(
            press(&mut app, 91),
            vec![time(TimeAction::Transport(Command::ToggleOverdub))]
        );
        assert_eq!(press(&mut app, 92), vec![time(TimeAction::Undo)]);
        assert_eq!(
            held(&mut app, button::SHIFT, |a| press(a, 92)),
            vec![time(TimeAction::Redo)]
        );
        // Rolling, STOP stops the transport and leaves the sounds alone.
        app.set_rolling(true);
        let fx = press(&mut app, button::STOP);
        assert_eq!(fx[0], time(TimeAction::Transport(Command::Stop)));
        assert!(!fx.contains(&Effect::StopSounds));
        // Stopped, STOP silences everything, as before, and goes back to bar 1.
        app.set_rolling(false);
        let fx = press(&mut app, button::STOP);
        assert!(
            fx.contains(&Effect::StopSounds)
                && fx.contains(&time(TimeAction::Transport(Command::Stop)))
        );
    }

    #[test]
    fn the_clips_page_takes_the_wheel_and_the_tempo() {
        let mut app = ready();
        app.open_tab(View::Arrange);
        assert_eq!(app.view(), View::Arrange);
        assert_eq!(
            app.wheel(1),
            vec![Effect::ClipsWheel {
                delta: 1,
                scroll: false
            }]
        );
        assert_eq!(
            held(&mut app, button::SHIFT, |a| a.wheel(-1)),
            vec![Effect::ClipsWheel {
                delta: -1,
                scroll: true
            }]
        );
        // MENU there opens the options and goes back to Arrange.
        press(&mut app, MENU);
        assert_eq!(app.view(), View::Options);
        press(&mut app, MENU);
        assert_eq!(app.view(), View::Arrange);
        // The page's tempo is the app's: Note Repeat and the arp follow it.
        let fx = app.set_tempo(96.5);
        assert_eq!(app.settings.bpm, 96.5);
        assert!(fx.contains(&Effect::Note("tempo 96.5 BPM".into())));
        app.set_tempo(1000.0);
        assert_eq!(app.settings.bpm, 300.0);
        app.open_tab(View::Play);
        assert_eq!(
            app.wheel(1),
            vec![Effect::Kit(1)],
            "the wheel picks kits on Play"
        );
    }

    /// The mode buttons, ERASE and the SHIFT combos from the hardware
    /// control map (Jay, 20:40Z), once Map controls has found them.
    #[test]
    fn the_mode_buttons_go_to_the_page_with_the_same_job() {
        use crate::play_page::PlayView;
        use crate::sample_page::Editor;
        use mapper::Job;
        const SHIFT: u8 = button::SHIFT;
        let mut app = ready();
        let jobs = [
            (Job::Erase, 60),
            (Job::Main, 61),
            (Job::SixteenLevel, 62),
            (Job::NextSeq, 63),
            (Job::StepSeq, 64),
            (Job::Sampler, 65),
            (Job::SampleEdit, 66),
            (Job::ProgEdit, 67),
        ];
        // Not found yet: nothing.
        assert!(press(&mut app, 65).is_empty());
        for (job, n) in jobs {
            app.map.learn(job, Some(n));
            assert_eq!(app.map.of(job), Some(n));
        }
        assert_eq!(app.map.name(65), Some("SAMPLER"));
        assert_eq!(app.map.name(67), Some("PROG EDIT"));
        let shifted = |app: &mut App, n: u8| held(app, SHIFT, |a| press(a, n));
        // SAMPLER: Sample; SHIFT + SAMPLER: Looper.
        assert!(press(&mut app, 65).contains(&Effect::Show(View::Sample)));
        assert!(shifted(&mut app, 65).contains(&Effect::Show(View::Looper)));
        assert_eq!(app.view(), View::Looper);
        // MAIN: Play, and on Play the next view.
        assert!(press(&mut app, 61).contains(&Effect::Show(View::Play)));
        assert_eq!(press(&mut app, 61), [Effect::PlayView(None)]);
        // STEP SEQ: Play's Beats view; NEXT SEQ: Arrange.
        assert_eq!(
            press(&mut app, 64),
            [Effect::PlayView(Some(PlayView::Beats))]
        );
        assert!(press(&mut app, 63).contains(&Effect::Show(View::Arrange)));
        let fx = press(&mut app, 64);
        assert!(fx.contains(&Effect::Show(View::Play)));
        assert!(fx.contains(&Effect::PlayView(Some(PlayView::Beats))));
        // SAMPLE EDIT: the Wave editor; PROG EDIT: the audio loop decides.
        let fx = press(&mut app, 66);
        assert_eq!(fx[0], Effect::SampleEditor(Editor::Wave));
        assert!(fx.contains(&Effect::Show(View::Sample)));
        assert_eq!(press(&mut app, 67), [Effect::ProgEdit]);
        // ERASE and SHIFT + ERASE do nothing yet (Jay, 22:12Z).
        assert!(press(&mut app, 60).is_empty());
        assert!(shifted(&mut app, 60).is_empty());
        // SHIFT + 16 LEVEL: Chromatic | In key (here: no scale yet).
        assert!(press(&mut app, 62).is_empty());
        assert!(shifted(&mut app, 62)
            .iter()
            .any(|e| matches!(e, Effect::Note(n) if n.contains("pick a scale first"))));
    }

    #[test]
    fn shift_stop_goes_back_to_bar_1_and_shift_rec_captures() {
        const SHIFT: u8 = button::SHIFT;
        let mut app = ready();
        let fx = held(&mut app, SHIFT, |a| press(a, button::STOP));
        assert!(fx.contains(&Effect::Time(TimeAction::Transport(Command::StopAndRewind))));
        assert!(fx.contains(&Effect::StopSounds));
        assert!(!fx.contains(&Effect::Show(View::Console)));
        // SHIFT + REC (90 here): Capture, on the Looper tab too.
        for v in [View::Play, View::Looper] {
            app.open_tab(v);
            assert_eq!(
                held(&mut app, SHIFT, |a| press(a, 90)),
                [Effect::Time(TimeAction::Capture)]
            );
        }
        // SHIFT + PLAY START (93): the metronome.
        assert_eq!(held(&mut app, SHIFT, |a| press(a, 93)), [Effect::Metronome]);
    }
}
