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
//! also silences everything, as before); REC 73, OVERDUB 80, UNDO 67
//! (with SHIFT it redoes) and PLAY START 83 are known from the start, from
//! Akai's own transport lights (the control mapper or the button guide in
//! Prefs can still change them). They become `Effect::Time`, which the
//! audio loop hands to its `TimeHost`.
//!
//! "Show developer tools" (Prefs) adds the Dev tab: the MIDI log, the
//! Performance page, the control mapper (the `mapper` submodule) and the
//! Console. Prefs itself is not a tab: SHIFT + MENU opens it. The PAD BANK buttons,
//! once found, pick banks A-H (`Effect::Bank`).

mod add_sound;
mod arp;
mod customise;
pub mod mapper;
mod pad_layers;
mod pad_map_panel;
mod pads_focus;
mod prefs_tabs;
mod route_synths;
pub mod saved;
pub use customise::{ThemeItem, THEME_NAME_MAX};
pub use prefs_tabs::PrefsTab;
pub mod step_pads;
mod synth_pads;
mod synth_pick;
mod tuning_picks;
mod tunings_page;

pub use add_sound::AddSound;
pub use arp::{
    built_in_patterns, mode_label, ArpItem, ArpView, LiveMap, MakeFrom, WorkspaceView, ARP_HEADER,
};
pub use mapper::{Code, MapView, Mark};
use mpfree_arp::scale::pads::Pad as PadNotes;
pub use pad_layers::{layer_names, LayerRec};
pub use pad_map_panel::{MapAct, MapChange, MapRow, MapSound, PadMapView};
pub use pads_focus::{
    layer_takes, synth_track, unfit, BankEntry, LayerEntry, LayerTake, PadFocus, PadMap, RecPlan,
    RecSound, SynthTake,
};
#[cfg(test)]
use pads_focus::{RecFrom, RecMore};
pub use route_synths::REC_MORE;
pub use step_pads::StepPads;
pub use synth_pads::{sent_name, SynthItem};
pub use synth_pick::{KitOn, SynthPickView};

use crate::cores::SoundCores;
use crate::looping::{LoopAction, LoopMemory, LoopSource};
use mpfree_arp::{Events, LatchMode, Mode, Pattern, Rate, RecordMode, Recorder};
use mpfree_controller::mpc_one::button;
use mpfree_looper::Command as LoopCommand;
use mpfree_looper::SmartEnd;
use mpfree_play::TrackArps;
use mpfree_time_ui::TimeAction;
use mpfree_transport::Command;

/// Button lights take the button's note as the CC number on the Private
/// cable. Akai's app sends 1 and 3 (dim and bright, inferred); so do we.
pub const LIGHT_OFF: u8 = 1;
pub const LIGHT_ON: u8 = 3;
/// Fully dark: PLAY, REC and OVERDUB use 0, not 1, for off (Akai's
/// transport lights, live test 3 capture).
pub const LIGHT_DARK: u8 = 0;
/// Akai's transport lights blink about every 0.3 s (each half of the
/// blink; observed in live test 3's capture, tempo not recorded).
pub const BLINK_HALF_SECONDS: f64 = 0.3;

/// What the transport buttons' lights show ([`App::transport_lights`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TransportLights {
    /// The transport plays, records or counts in.
    pub rolling: bool,
    /// The transport records or counts in, or a loop is armed or records
    /// its first pass.
    pub recording: bool,
    /// The transport's overdub is on, or a loop overdubs.
    pub overdub: bool,
}
/// Two NOTE REPEAT presses this close together (in seconds) latch, as
/// SHIFT + NOTE REPEAT does.
pub const DOUBLE_PRESS_SECONDS: f64 = 0.4;

/// TAP TEMPO held this long with no wheel turn opens the tempo's number
/// pad as it is let go (a tap is far shorter).
pub const TAP_HOLD_SECONDS: f64 = 0.6;

/// The pages, in the order of the header's tabs, then the ones without a tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Play,
    /// The pads (the owner's pick, 12 Nov 2026 15:46Z): the pad bank beside
    /// the pad's settings (Pad), whose "Browse for" opens the full browser;
    /// the sampler's pad screens (`mpfree_sampler_ui::pads_tab`).
    Pads,
    /// The Sequencer (the Play, Pads and Sequencer redesign, Jay's go
    /// 18 Nov 00:30Z): the armed track's clip as steps (Steps: the step
    /// grid, its inspector and timing strip, which were Play's Beats view)
    /// or as notes (Notes: the note editor, which was Arrange's). STEP SEQ
    /// opens it.
    Sequencer,
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
    /// Debug mode: the status board raw (on the Debug tab): every problem
    /// MPFree noted this session with its state, count and times, and the
    /// last reports (`mpfree-status`; System status phase 0).
    StatusBoard,
    /// Prefs > System info: the program's version, the system and the
    /// hardware (on the Prefs tab).
    System,
    /// The mixer (mpfree-mixer-ui): the Mix tab, or TRACK MIX (layout
    /// spec 3a; Jay's card, 14 Nov 07:15Z).
    Mixer,
    /// The FX chain editor (mpfree-chain-ui) in the mixer's place: a strip's
    /// insert row or its "+" opens it, and its Back goes back to the mixer
    /// (layout spec 4b, Jay 22:34Z). Not a tab.
    Chain,
    /// The macros and morph panel over the page under the top bar: a long
    /// press on a macro card in the Q-Link column (layout spec 4b). Not a
    /// tab; its Back goes back to the page it opened over.
    Macros,
    /// The browser (mpfree-browser-ui) over the page under the top bar:
    /// BROWSE opens and closes it (controls decision 13; Jay's ask after
    /// live test 3). Projects (New, Open, Save), samples and the rest. Not
    /// a tab.
    Browser,
    /// System status (layout spec 2a): what is wrong, which part, what to
    /// do. The status chip and Prefs > System status open it over the
    /// page; the chip again, MENU or a tab closes it. Not a tab.
    SystemStatus,
}

/// The header's tabs, in the order of the approved layout spec
/// (docs/dev/design/layout-spec.md, approved by the owner 2026-11-11): the music
/// pages, and the Dev tab only with the developer tools on. Prefs is not a
/// tab (SHIFT + MENU).
pub fn tabs(dev: bool) -> Vec<View> {
    let mut t = vec![
        View::Play,
        View::Sequencer,
        View::Looper,
        View::Sample,
        View::Synth,
        View::Arrange,
        View::Pads,
        View::Mixer,
    ];
    if dev {
        t.push(View::Debug);
    }
    t
}

/// The pages of a kit's 128 pads on the 16 pads: PAD BANK A to H.
pub const KIT_PAGES: usize = 8;

/// A kit pad's name: its page's letter and its number there, as Akai's
/// MPC names them ("A1" is pad 1, "B5" pad 21).
pub fn kit_pad_name(pad: usize) -> String {
    format!("{}{}", bank_letter(pad / 16), pad % 16 + 1)
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

/// What TRACK MUTE's pads are in bank `bank` (A = 0) of a project of
/// `tracks` tracks: "bank B: pads 1-8 mute tracks 17-24".
pub fn mute_bank_words(bank: usize, tracks: usize) -> String {
    let first = bank * 16;
    let n = tracks.saturating_sub(first).min(16);
    let letter = bank_letter(bank);
    match n {
        0 => format!(
            "bank {letter}: no tracks here (it is tracks {}-{})",
            first + 1,
            first + 16
        ),
        1 => format!("bank {letter}: pad 1 mutes track {}", first + 1),
        n => format!(
            "bank {letter}: pads 1-{n} mute tracks {}-{}",
            first + 1,
            first + n
        ),
    }
}

/// A kit's bank letter (kits 1-8 are banks A-H, then again from kit 9).
pub fn bank_letter(kit: usize) -> char {
    (b'A' + (kit % 8) as u8) as char
}

/// The Debug tab's pages, in the order of its strip.
pub const DEBUG_PAGES: [View; 8] = [
    View::Debug,
    View::Perf,
    View::Engines,
    View::Procs,
    View::Map,
    View::Touch,
    View::StatusBoard,
    View::Console,
];

impl View {
    /// The tab a page belongs to (the Arp panel to Play, the debug pages to
    /// Debug), if any.
    pub fn tab_view(self) -> Option<View> {
        match self {
            View::Arp | View::Pattern => Some(View::Play),
            View::Perf
            | View::Engines
            | View::Procs
            | View::Map
            | View::Touch
            | View::StatusBoard
            | View::Console => Some(View::Debug),
            // The chain editor is the Mix tab's (Jay's card, 14 Nov 07:15Z).
            View::Chain => Some(View::Mixer),
            View::Options
            | View::Guide
            | View::Prefs
            | View::System
            | View::Macros
            | View::Browser
            | View::SystemStatus => None,
            v => Some(v),
        }
    }

    /// The pages the sampler's screens draw: the Pads tab and the Sample
    /// tab share the kit, the pad picked, edits and undo.
    pub fn sampler(self) -> bool {
        matches!(self, View::Sample | View::Pads)
    }

    /// The tab's place in the header, which depends on debug mode.
    pub fn tab_index(self, debug: bool) -> Option<usize> {
        let v = self.tab_view()?;
        tabs(debug).iter().position(|&t| t == v)
    }

    pub fn name(self) -> &'static str {
        match self {
            View::Play => "Play",
            View::Pads => "Pads",
            View::Sequencer => "Sequencer",
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
            View::StatusBoard => "Status",
            View::System => "System info",
            View::Mixer => "Mix",
            View::Chain => "FX chain",
            View::Macros => "Macros",
            View::Browser => "Browser",
            View::SystemStatus => "System status",
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
    /// TRACK MUTE: the pads mute and unmute 16 tracks, the bank's
    /// ([`App::mute_bank`]: A is tracks 1-16, B 17-32, up to H, 113-128).
    Tracks,
    /// SHIFT + TRACK MUTE: each pad mutes and unmutes itself in the
    /// playing kit (the speakers on Play).
    Pads,
}

/// A pad tapped as a mute key ([`App::mute_tap`]): what it does and when.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MuteTap {
    /// Tracks or the kit's pads.
    pub keys: MuteKeys,
    /// TRACK MUTE's bank ([`App::mute_bank`]).
    pub bank: usize,
    /// The change waits for the launch line (Perform; SHIFT gives the
    /// other timing).
    pub wait: bool,
    /// TRACK MUTE is held: the tap solos the track.
    pub solo: bool,
}

/// TRACK MUTE held (sequencer S4): the pad mode before the press, and
/// whether a pad was tapped while it was held (a solo, after which the
/// mode goes back to what it was).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MuteHold {
    before: MuteKeys,
    used: bool,
}

/// MAIN held: which 16 tracks the pads are (A = 0: tracks 1-16), and
/// whether a pad picked one (then the let-go goes nowhere).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MainHold {
    bank: usize,
    used: bool,
}

/// FULL LEVEL held: SHIFT with it (Half Level on the let-go), and whether
/// a pad hit set the level (then the let-go changes nothing more).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FullHold {
    shift: bool,
    used: bool,
}

/// TAP TEMPO held: since when (`at`, seconds into the session), and
/// whether the wheel turned the tempo (then the let-go opens nothing).
#[derive(Clone, Copy, Debug, PartialEq)]
struct TapHold {
    at: f64,
    used: bool,
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

/// Every button's number: all built in ([`ButtonMap::measured`]), and Map
/// controls or the guide can give one another.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ButtonMap {
    pub menu: Option<u8>,
    pub note_repeat: Option<u8>,
    pub tap_tempo: Option<u8>,
    pub minus: Option<u8>,
    pub plus: Option<u8>,
    /// The transport buttons: known from the start ([`ButtonMap::measured`]),
    /// and Map controls or the guide can give them another number.
    pub rec: Option<u8>,
    pub overdub: Option<u8>,
    pub undo: Option<u8>,
    pub play_start: Option<u8>,
    /// PAD BANK A/E to D/H.
    pub bank: [Option<u8>; 4],
    /// TRACK MIX (SHIFT: PAD MIX).
    pub track_mix: Option<u8>,
    /// The mode buttons and the rest of the hardware control map (Jay,
    /// 20:40Z).
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
    /// The data dial's push.
    pub data_push: Option<u8>,
    /// BROWSE (SHIFT: save the project).
    pub browse: Option<u8>,
}

impl ButtonMap {
    /// The numbers measured on the reference MPC One: every button.
    pub fn measured() -> ButtonMap {
        ButtonMap {
            menu: Some(button::MENU),
            note_repeat: Some(button::NOTE_REPEAT),
            tap_tempo: Some(button::TAP_TEMPO),
            minus: Some(button::MINUS),
            plus: Some(button::PLUS),
            // The transport buttons from Akai's own lights: without them a
            // fresh MPFree (a new stick, no saved map) had REC, OVERDUB,
            // UNDO and PLAY START dead and dark until Map controls found
            // them, which a musician never does.
            rec: Some(button::REC),
            overdub: Some(button::OVERDUB),
            undo: Some(button::UNDO),
            play_start: Some(button::PLAY_START),
            // Every button is the developers' job, never the musician's
            // (Jay, 16 Nov): BROWSE and MAIN from the fourth creator stick's
            // presses, the data dial's push from the Map controls live test
            // of 12 Nov, the rest from the fifth stick's Map controls pass.
            browse: Some(button::BROWSE),
            main: Some(button::MAIN),
            data_push: Some(button::DATA_PUSH),
            bank: button::PAD_BANK.map(Some),
            track_mix: Some(button::TRACK_MIX),
            erase: Some(button::ERASE),
            sixteen_level: Some(button::SIXTEEN_LEVEL),
            next_seq: Some(button::NEXT_SEQ),
            step_seq: Some(button::STEP_SEQ),
            sampler: Some(button::SAMPLER),
            sample_edit: Some(button::SAMPLE_EDIT),
            prog_edit: Some(button::PROG_EDIT),
            q_link: Some(button::Q_LINK),
            track_mute: Some(button::TRACK_MUTE),
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
            Job::DataPush => self.data_push,
            Job::Browse => self.browse,
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
            Job::DataPush => &mut self.data_push,
            Job::Browse => &mut self.browse,
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
    /// How Latch keeps pads: Replace (a fresh press starts over), Toggle
    /// or Add (every press stays until Latch goes).
    pub latch_mode: LatchMode,
    /// The sustain pedal latches the armed track's arp while it plays
    /// (Repeat, Arp or Pattern): down is Latch on, up lets go (the MPC's
    /// Sustain Pedal Latches, 3.10.1 guide p. 83).
    pub pedal_latches: bool,
    /// Quick advance (live shows B1, layout spec 5): with a set playing
    /// and the transport stopped, one press of Next song goes. Off, it
    /// arms and a second press goes, stopped too.
    pub quick_advance: bool,
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
    /// Prefs "Send MIDI and CV out": tracks set to a port play out of it,
    /// clock goes out as each port says, and the CV jacks follow the
    /// project. Off by default; kept between starts once on (Jay's pick on
    /// the MIDI thread's card, 16 Nov 23:16Z). State-changing on the MPC:
    /// bytes leave the MIDI Out jack and the CV jacks, in RAM only, never
    /// SysEx.
    pub sends_midi: bool,
    /// The line input is heard through the outputs.
    pub loop_monitor: bool,
    /// What the looper records: the pads (the default) or the line input.
    pub loop_source: LoopSource,
    /// Bars in the looper's next first pass: the pattern's length
    /// (`Some(PATTERN_BARS)`, the default: looping.rs), None for free
    /// (until STOP or Record again), or a number of bars.
    pub loop_bars: Option<u32>,
    /// Input latency the looper makes up for, in frames; estimated from
    /// the audio buffer until set by hand.
    pub loop_latency: u32,
    pub loop_latency_set: bool,
    pub loop_level_db: f32,
    /// The looper's options "Tempo from the first loop": with the song
    /// stopped, a first loop recorded on Until tap sets the tempo (and
    /// starts the song) when it closes. Off by default; kept in Prefs.
    pub loop_tempo: bool,
    /// The looper's options "Smart loop end": when tempo from the first
    /// loop closes a loop played on the pads (or heard in the line input
    /// or a mix, by ear), Suggest (the default)
    /// closes on your tap and offers the downbeat you played, Auto
    /// listens a moment on and closes on it, Off keeps your tap. Kept in
    /// Prefs.
    pub smart_end: SmartEnd,
    /// Prefs > Appearance: the theme in use, a built-in name (see
    /// [`THEMES`]); `--theme` can also set a theme file for one run.
    pub theme: String,
    /// Prefs "Sound cores": where the sound runs from the next start
    /// (cores.rs); `--sound-cores` can set it for one run.
    pub sound_cores: SoundCores,
    /// Prefs "Loop memory": the looper's size from the next start.
    pub loop_memory: LoopMemory,
    /// The metronome (click while counting in and recording): off by
    /// default (live test 3), Play's options or SHIFT + PLAY START. How
    /// you like to work, so kept in Prefs, not the project.
    pub metronome: bool,
    /// The click's level, dBFS: [`METRONOME_DBS`].
    pub metronome_db: i8,
    /// The velocity every hit plays at with Half Level on: 64, until a
    /// pad hit with FULL LEVEL held makes it that hit's (the MPC's own
    /// level, which takes Half Level's place). Kept in Prefs.
    pub half_level: u8,
    /// Play's options "Timing strip": the Beats view shows Your timing
    /// under the pattern (Jay 13 Nov folded the Drum view into Beats).
    pub timing_strip: bool,
    /// What Play's XY pad plays, picked from its chip's list: None, the
    /// DJ filter and echo; `Some(k)`, the XY FX's effect `k`
    /// (`mpfree_fx::xyfx::EFFECTS`; Jay, 18 Nov 2026 07:34Z).
    pub xy_effect: Option<usize>,
    /// Prefs "Audio track memory": the audio tracks' size from the next
    /// start.
    pub audio_memory: crate::audiotracks::AudioMemory,
    /// Dev's options "Audio tracks from their files": from the next start,
    /// takes play from their files on the drive through the stream layer's
    /// cache (as big as Audio track memory), so memory only holds what
    /// records (the audio tracks rethink, phase 3b). Off by default, and on
    /// the MPC until the stick tests have run with Jay's go.
    pub stream_audio: bool,
    /// Whether takes play from their files in this start (the switch above
    /// as it was at start, if the stream layer started): a project opening
    /// then reads only its takes' headers. Never saved.
    pub audio_streams: bool,
    /// Dev's options "Loops on audio tracks" (the audio tracks rethink,
    /// phase 4, steps 4b to 4d: docs/dev/design/loops-on-audio-tracks.md):
    /// from the next start, a loop a loop cell of an audio track names
    /// plays on that track, where the arranger says. Off by default until
    /// 4e switches it on for good; a project saved with it on is a
    /// development file.
    pub loops_on_tracks: bool,
    /// Whether loops play on audio tracks in this start (the switch above
    /// as it was at start). Never saved.
    pub loops_on: bool,
    /// Prefs "At start": open the last project (true, the default since
    /// round 4 of the creator run: a restart lost the pads the owner had
    /// set) or start with a new one. The screen opens it once the sound is
    /// going.
    pub open_last: bool,
    /// Prefs "Export": Fast, quiet (the default) or Listen (Jay, 14 Nov
    /// 01:32Z). An export that needs outside gear listens anyway.
    pub export_mode: mpfree_export::Mode,
    /// Prefs "Export file" and "Export rate": the WAV files' format each
    /// export starts from (the Export panel can change it for one export;
    /// Jay, 14 Nov 07:51Z).
    pub export_bits: mpfree_fx::Bits,
    pub export_rate: u32,
}

/// Half Level's velocity until FULL LEVEL held and a pad hit set another.
pub const HALF_LEVEL: u8 = 64;

/// Smart loop end's choices, in the order the MENU steps them.
pub const SMART_ENDS: [SmartEnd; 3] = [SmartEnd::Off, SmartEnd::Suggest, SmartEnd::Auto];

/// Smart loop end's choice as the MENU and the settings file name it.
pub fn smart_end_name(m: SmartEnd) -> &'static str {
    match m {
        SmartEnd::Off => "Off",
        SmartEnd::Suggest => "Suggest",
        SmartEnd::Auto => "Auto",
    }
}

/// The rates Prefs > Export rate steps through.
pub const EXPORT_RATES: [u32; 2] = [44_100, 48_000];

/// The click levels Play's options step through, dBFS. -24 by default;
/// -12 was the old fixed level ("far too loud", live test 3).
pub const METRONOME_DBS: [i8; 6] = [-36, -30, -24, -18, -12, -6];

impl Settings {
    /// The latency the looper makes up for: none for the pads or a mix,
    /// which are in time with the transport (the host makes up the mix's
    /// own).
    pub fn loop_latency_now(&self) -> u32 {
        match self.loop_source.line() {
            false => 0,
            true => self.loop_latency,
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
            pedal_latches: false,
            quick_advance: false,
            periods: 3,
            show_pressure: false,
            midi_paused: false,
            console_paused: false,
            debug_mode: false,
            line_input: false,
            sends_midi: false,
            loop_monitor: false,
            loop_source: LoopSource::Pads,
            // REC + PLAY records until STOP, as on the MPC (Jay, 16 Nov).
            loop_bars: Some(crate::looping::PATTERN_BARS),
            loop_latency: crate::looping::estimated_latency(3),
            loop_latency_set: false,
            loop_level_db: 0.0,
            loop_tempo: false,
            smart_end: SmartEnd::Suggest,
            theme: "default".to_string(),
            sound_cores: SoundCores::Shared,
            loop_memory: LoopMemory::default(),
            metronome: false,
            metronome_db: crate::time::CLICK_DB as i8,
            half_level: HALF_LEVEL,
            timing_strip: true,
            xy_effect: None,
            audio_memory: crate::audiotracks::AudioMemory::default(),
            stream_audio: false,
            audio_streams: false,
            loops_on_tracks: false,
            loops_on: false,
            open_last: true,
            export_mode: mpfree_export::Mode::Fast,
            export_bits: mpfree_fx::Bits::I24,
            export_rate: EXPORT_RATES[0],
        }
    }
}

/// The themes Prefs > Appearance offers: the built-in name and its row.
/// The same names, in the same order, as `mpfree_ui::theme::BUILTIN`.
pub const THEMES: [(&str, &str); 7] = [
    ("default", "Default"),
    ("slate", "Slate"),
    ("light", "Light"),
    ("console", "Console"),
    ("synthwave", "Synthwave"),
    ("spaceship", "Spaceship"),
    ("abletown", "AbleTown"),
];

/// The store owner whose files are the themes folder: theme files the
/// user puts in `files/themes/` on the data partition, named `NAME.theme`.
pub const THEMES_FOLDER: &str = "themes";

/// The store owner whose files are the tunings folder: Scala scales the
/// user puts in `files/tunings/` on the data partition (`NAME.scl`, with an
/// optional keyboard map `NAME.kbm`), offered on the Synth tab.
pub const TUNINGS_FOLDER: &str = "tunings";

/// One row of a menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    Curve,
    ScreenVelocity,
    LatchMode,
    /// Prefs and the arp's options: the sustain pedal latches the arp.
    PedalLatch,
    /// Prefs: Quick advance, Off or On (live shows B1).
    QuickAdvance,
    Buffer,
    FindButtons,
    /// That section: a new backup of the MPC at the next start.
    SetupBackup,
    /// That section: Akai's kits and samples at the next start.
    SetupAkaiKits,
    /// Prefs: the theme in use; the arrows step through [`THEMES`].
    Appearance,
    CalibrateTouch,
    TouchLab,
    /// Play's MENU (live shows C1): the patch list, where a song with no
    /// patches gets its first.
    Patches,
    /// The Show view's MENU (live shows B1): the Set editor, the set's
    /// check, and closing the set.
    SetEditor,
    CheckSet,
    CloseSet,
    DebugMode,
    /// Prefs: where the sound runs from the next start (cores.rs).
    SoundCores,
    MapControls,
    SystemInfo,
    SystemStatus,
    Rate,
    Tempo,
    PadLevel,
    Latch,
    ShowPressure,
    ClearLogs,
    /// Dev's options: takes play from their files from the next start.
    StreamAudio,
    /// Dev's options: loops play on audio tracks from the next start.
    LoopsOnTracks,
    ConsoleFollow,
    ClearConsole,
    LineInput,
    /// Prefs: MIDI and CV out on or off.
    SendsMidi,
    LoopSource,
    LoopMonitor,
    LoopBars,
    LoopLatency,
    LoopLevel,
    /// The looper's options: tempo from the first loop, on or off.
    LoopTempo,
    /// The looper's options: smart loop end, Off, Suggest or Auto.
    SmartEnd,
    /// Prefs and the looper's options: the looper's size from the next
    /// start.
    LoopMemory,
    /// Prefs: what opens at start, a new project or the last one.
    AtStart,
    /// Prefs: how Export plays the song, Fast, quiet or Listen.
    Export,
    /// Prefs: the export's WAV files: 24-bit, 16-bit or 32-bit float.
    ExportFile,
    /// Prefs: the export's rate: 44.1 or 48 kHz.
    ExportRate,
    /// Play's options: the metronome on or off, and its level.
    Metronome,
    MetronomeLevel,
    /// Play's options: the Beats view's Your timing strip on or off.
    TimingStrip,
    /// Prefs: the audio tracks' size from the next start.
    AudioMemory,
    /// The arp's own settings (the Options menu opened from the Arp panel).
    Arp(ArpItem),
    /// The synth's engine and pad layouts (the Synth tab's options).
    Synth(SynthItem),
    /// Row `k` of the Tunings page (tunings step T6,
    /// [`tunings_page::TuningsRow`]).
    Tunings(u16),
    /// A row of Prefs › Appearance › Customise.
    Theme(ThemeItem),
}

/// A row whose + opens a page or does a thing once, rather than stepping
/// a value: the wheel's push opens it straight away.
fn opens_something(item: Item) -> bool {
    matches!(
        item,
        Item::FindButtons
            | Item::SetupBackup
            | Item::SetupAkaiKits
            | Item::CalibrateTouch
            | Item::TouchLab
            | Item::Patches
            | Item::SetEditor
            | Item::CheckSet
            | Item::CloseSet
            | Item::MapControls
            | Item::SystemInfo
            | Item::SystemStatus
            | Item::ClearLogs
            | Item::ClearConsole
    )
}

/// The Show view's MENU rows (live shows B1, layout spec 4p).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetMenu {
    /// Open the Set editor on the set playing.
    Editor,
    /// Check the set now.
    Check,
    /// Close the set: the song playing stays open as a plain project.
    Close,
}

/// The wheel, its push and - / + on the Show view (live shows B1), for
/// the screen's set player.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShowKey {
    /// The wheel or - / +: arm the song after (+1) or before (-1).
    Step(i32),
    /// The wheel's push: Go.
    Push,
}

/// The Show view's MENU rows.
const SHOW_OPTIONS: [Item; 3] = [Item::SetEditor, Item::CheckSet, Item::CloseSet];

/// Play's MENU: the options every page has, then Patches (live shows C1).
const PLAY_OPTIONS: [Item; 8] = [
    Item::Rate,
    Item::Tempo,
    Item::Metronome,
    Item::MetronomeLevel,
    Item::TimingStrip,
    Item::PadLevel,
    Item::Latch,
    Item::Patches,
];

/// The menu on screen: its rows (name, value), the selected row and help.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MenuView {
    pub title: String,
    pub rows: Vec<(String, String)>,
    /// What marks each row beyond its name and value, row by row (tunings
    /// step T6: the OWN tag, a line under the name, a value that can't be
    /// changed, a section heading). Empty: no row has any.
    pub marks: Vec<RowMark>,
    pub selected: usize,
    /// The wheel's push opened the selected row: the wheel changes its
    /// value until the next push.
    pub editing: bool,
    pub help_heading: String,
    /// What the selected row's choice means now, over its help, brighter
    /// (the Instrument row: the track's line, step 11d). Mostly empty.
    pub facts: Vec<String>,
    /// The facts warn: drawn in amber (an instrument that can't follow its
    /// tuning).
    pub facts_warn: bool,
    /// A button after the facts that opens a page: its title and the line
    /// under it ("Tunings", "The song's tuning and every instrument's"). A
    /// tap is `FromUi::HelpButton(mpfree_ui::widgets::CARD_LINK)`.
    pub help_link: Option<(String, String)>,
    pub help: Vec<String>,
    /// Buttons under the help ("Keep" and "Go back" while a theme is on
    /// trial); a tap is `FromUi::HelpButton`.
    pub help_buttons: Vec<String>,
    /// Prefs as its own page draws it (layout spec 5): its tabs and cards.
    /// None for a page's options, which are the rows above.
    pub prefs: Option<Box<crate::prefs_page::PrefsView>>,
}

/// One menu row's marks beyond its name and value
/// (`mpfree_ui::widgets::OptionRow`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RowMark {
    /// A chip after the name ("OWN").
    pub tag: &'static str,
    /// A line under the name ("Tracks 3 and 4").
    pub sub: String,
    /// The value can't be changed: amber, no arrows.
    pub fixed: bool,
    /// A section heading: not picked by a tap or the wheel.
    pub heading: bool,
}

/// How long "Keep this theme?" waits before it puts the old theme back.
pub const THEME_TRIAL_S: f64 = 15.0;

/// About as many characters as a line of Prefs' panel holds.
const HELP_CHARS: usize = 48;

/// The most lines of theme problems the Appearance panel lists after its
/// help (the panel holds about 15 lines).
const THEME_PROBLEM_LINES: usize = 10;

/// `text` in lines of at most `width` characters, broken at spaces (a
/// longer word gets a line of its own).
fn wrap_line(text: &str, width: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > width {
            out.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        out.push(line);
    }
    out
}

/// A theme picked in Prefs > Appearance and not kept yet: the theme to go
/// back to, and when ([`THEME_TRIAL_S`] after the last step).
#[derive(Clone, Debug, PartialEq)]
struct ThemeTrial {
    before: String,
    until: f64,
    /// The whole seconds left that the panel last showed.
    shown: u32,
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
    /// TRACK MUTE's bank (A = 0: tracks 1-16).
    pub mute_bank: usize,
    /// The loop keys are on (SHIFT + SAMPLER on the Looper tab): the pads
    /// are the loop keys on the Looper tab and Play.
    pub loop_keys: bool,
    /// The Perform set is on the pads at a page past A (PAD BANK): its
    /// name ("Page B · Throws"), for the chip and the hint.
    pub perform: Option<String>,
    /// The Perform set's page (PAD BANK A = 0 to H = 7): the one on the
    /// pads, and the one the Pads tab's Perform view shows.
    pub perform_bank: usize,
    /// STEP SEQ: the pads as steps, or STEP SEQ held to pick the sound.
    pub steps: Option<StepPads>,
    /// ERASE held: the next tap on the screen deletes what it lands on
    /// (Jay after live test 3).
    pub erase: bool,
    /// COPY held: on Arrange, a tap on a clip copies it and a tap on an
    /// empty slot pastes the copy (sequencer S3); over the kit's pads, a
    /// pad hit copies it, and the pads hit after it take the copy.
    pub copy: bool,
    /// Half Level's velocity ([`Settings::half_level`]): the level chip
    /// names it when it isn't 64.
    pub half_level: u8,
    /// MAIN held: the pads pick a track, of these 16 (A = 0: 1-16).
    pub main_held: Option<usize>,
    /// FULL LEVEL held: a pad hit sets the level every hit plays at.
    pub full_held: bool,
    /// TAP TEMPO held: the wheel turns the tempo.
    pub tap_held: bool,
}

/// What Sample Recall is asked (sampler R5 part 2, sampler decision 3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecallKey {
    /// A pad held + SHIFT + REC: open over the page on the kit's pad
    /// `pad` (a kit pad, 0 to 127).
    Open { pad: usize },
    /// PLAY while it is open: hear the bracket.
    Audition,
    /// REC while it is open: keep the bracket on its pad.
    Keep,
    /// STOP while it is open: close it (the transport plays on).
    Close,
}

/// Jay's wheel navigation (live test 2, 13 Nov) on a page with rows: the
/// Sample and Pads pages' settings lists, the Synth tab's tiles, and
/// Arrange's Record settings or Edit options. As in the menus: the wheel
/// moves the highlight from row to row; - and + step the highlighted
/// row's value; the wheel's push opens the row, so the wheel changes its
/// value, and the next push gives the wheel back to the rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowKey {
    /// Move the highlight this many rows (tiles on the Synth tab).
    Pick(i32),
    /// Step the highlighted row's value.
    Step(i32),
    /// The wheel now changes the value (true), or picks rows again.
    Editing(bool),
    /// – or + on the Sequencer's Notes view, and whether SHIFT is held:
    /// the screen decides (piano roll plan 1C.1). With the Edit options'
    /// drawer closed they nudge the picked notes and SHIFT zooms time;
    /// otherwise they step the row, as [`RowKey::Step`].
    Arrow { delta: i32, shift: bool },
}

/// Jay's wheel navigation on every page (13 Nov, after live test 3): a
/// tap puts an outline on a control, the wheel moves it from control to
/// control in reading order, through the panel and on into the next, and
/// the wheel's push does what a tap does. On a value (a fader, a knob card)
/// the push grabs it: the wheel then turns it until the next push. - and +
/// step an outlined value, or move the outline. The screen keeps the
/// outline (it knows the controls); [`outlined`] says on which pages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Nav {
    /// The wheel turned: move the outline, or turn the grabbed value.
    Turn(i32),
    /// The wheel's push: press the outlined control, or grab or let go a
    /// value.
    Push,
    /// - or +: step the outlined value, or move the outline.
    Step(i32),
    /// SHIFT + - or +: as [`Nav::Step`], in the finer steps SHIFT gives
    /// the wheel where the control has them (the button combos of 19 Nov).
    Fine(i32),
}

/// The pages where the wheel moves an outline between controls ([`Nav`]).
/// The pages with rows (Arrange, Synth, Sample, Pads, the Sequencer's
/// Notes), the menus, the Arp panel, the workspace and the Dev tab keep
/// the wheel's own jobs there. The Sequencer is named for its Steps view;
/// its Notes view has rows, which come first.
pub fn outlined(v: View) -> bool {
    matches!(
        v,
        View::Play | View::Sequencer | View::Looper | View::Mixer | View::Chain | View::Macros
    )
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
    /// The wheel, its push, - or + for the outline ([`Nav`]).
    Nav(Nav),
    /// The wheel or - / + while a finger rests on a control with a value
    /// (Jay, 17 Nov: hold a control and turn the wheel): turn that control
    /// by `delta` detents, `fine` with SHIFT held.
    Held {
        delta: i32,
        fine: bool,
    },
    /// SHIFT + wheel on Play: move the synth's pads by octaves (the kit, as
    /// before, when a drum kit is playing).
    Octave(i32),
    /// 16 LEVEL: 16 Levels on for the pad hit last, with its panel on
    /// Play, or off if they are on (the screen holds the kit and says back
    /// with [`App::levels_shown`]).
    SixteenLevels,
    /// The wheel while the 16 Levels panel shows: the next or previous kind.
    LevelsWheel(i32),
    /// A step edit from the buttons (ERASE + PAD BANK with STEP SEQ on),
    /// for the time engine.
    Steps(crate::time::StepAction),
    /// The Synth tab's options changed: the engine, the scale or the pad
    /// layouts, for the synth on the audio thread.
    SynthPads(crate::synth::PadSettings),
    /// The Synth tab's Instrument row: track `track` plays its own instrument
    /// `id` (an engine's node id, or a plugin synth's), or the kit again
    /// (None).
    TrackSound {
        track: usize,
        id: Option<&'static str>,
    },
    /// The Instrument row: track `track` plays track `to`'s instrument
    /// (instrument redesign step 4), its own instrument taken out first.
    TrackSend {
        track: usize,
        to: usize,
    },
    /// The Instrument row: track `track` gets a copy of track `from`'s
    /// instrument as its own (step 4c, Duplicate): the same engine and
    /// sound, a separate instrument with its own id.
    TrackCopy {
        track: usize,
        from: usize,
    },
    /// Add a part's New track (composing tools S2.6b, app/add_sound.rs):
    /// track `track`, the index Keep would give the new track, plays
    /// holder `to`'s synth by a send, or no longer (None). Quietly: the
    /// Synth tab and what the screen says stay as they are.
    AddPartSend {
        track: usize,
        to: Option<usize>,
    },
    /// What the pads play changed: the synth the Synth tab shows,
    /// whichever track is selected, or the selected track's instrument
    /// again (app/pads_focus.rs).
    PadsFocus(PadFocus),
    /// The list of synths over the Synth tab (step 6, app/synth_pick.rs):
    /// open as it draws, or closed (None).
    SynthPick(Option<SynthPickView>),
    /// What the pads play, the panel over the Synth tab (sampler step
    /// 15e, app/pad_map_panel.rs): open as it draws, or closed (None).
    PadMap(Option<PadMapView>),
    /// REC while the synth is chosen for the pads: this track plays the
    /// synth from now on (the Synth kit, as a pick in the list of kits), so
    /// the take plays the one instance that was heard (app/pads_focus.rs).
    SynthOnTrack(usize),
    /// REC while a kit is chosen for the pads (sampler step 15): this
    /// track plays that kit from now on, as a pick in the list of kits.
    KitOnTrack {
        track: usize,
        kit: usize,
    },
    /// The mixer opened on a view, or closed (`None`): the audio thread
    /// sends the panel its state only while it is open.
    Mixer(Option<mpfree_mixer_ui::MixView>),
    /// The Mix tab tapped: the mixer opens on the view it showed last.
    MixerLast,
    /// TRACK MIX opened the Mix tab: on Faders, whichever look it showed
    /// last (the screen's to do).
    MixerFaders,
    /// While the mixer is open: the data wheel turned by `delta`. On a
    /// strips view it is Q-Link 1, the selected strip's volume; on the I/O
    /// view it moves the section's selection.
    MixerWheel(i32),
    /// While the mixer is open: - (-1) or + (1). On the I/O view it steps
    /// the section's selected row; on a strips view it is [`Nav::Step`].
    MixerStep(i32),
    /// While the chain editor or the macros panel is open: Q-Link `knob`
    /// (or the wheel, as Q-Link 1) turned by `delta`, for the knobs the
    /// Q-Link column shows there.
    ChainTurn {
        knob: usize,
        delta: i32,
    },
    /// The wheel on the chain editor: the effect picker's tiles (SHIFT:
    /// its sections) while it is open, else Q-Link 1 as [`Effect::ChainTurn`].
    ChainWheel(i32),
    /// The wheel's push on the chain editor: the picker's presets, then an
    /// add.
    ChainPush,
    /// Q-LINK: the Q-Link column's next bank.
    QLinkBank,
    /// SHIFT + Q-LINK: the Q-Link column's previous bank, as on the MPC.
    QLinkBankBack,
    /// COPY held, a kit pad hit, then another (the MPC's COPY): kit pad
    /// `from`'s sound and every edit go onto kit pad `to` of the armed
    /// track's kit, one undo step, as Send to pad does.
    PadCopy {
        from: u8,
        to: u8,
    },
    /// SHIFT + COPY held, then a kit pad hit (the MPC's Delete): the pad
    /// is emptied, one undo step, as the Pads tab's Clear.
    PadClear {
        pad: u8,
    },
    /// SHIFT + TC (the MPC's TC On/Off): the armed track's Quantise while
    /// recording, off, or back to the grid it had (1/16 the first time).
    RecordQuantiseToggle,
    /// SHIFT + a kit pad: pad `slot` of the bank shown is picked for the
    /// Sample and Pads tabs and the Q-Links, as a hit picks it, and plays
    /// nothing.
    PadPick {
        slot: u8,
    },
    /// SHIFT + PLAY: play again from where PLAY last started the
    /// transport (while it plays, a jump back there).
    PlayBack,
    /// SHIFT + OVERDUB: the count-in off, or back to the bars it had (1
    /// bar the first time).
    CountInToggle,
    /// SHIFT + SAMPLE EDIT (the MPC's Q-Link Edit): the macros panel on
    /// Macros 1-4, or back from it.
    MacrosPanel,
    /// SHIFT + PROG EDIT (the MPC's MIDI Control settings): the mixer's
    /// I/O view on its Controller section, where controller mode is set up.
    ControllerSettings,
    /// TAP TEMPO held alone and let go: the tempo's number pad.
    TypeTempo,
    /// All four PAD BANK buttons held (the MPC's screenshot): a picture of
    /// the screen, saved in the data folder.
    ScreenPicture,
    /// SHIFT + NEXT SEQ (the MPC's XYFX): the XY pad full screen on Play,
    /// or back to Play's view from it.
    XyFull,
    /// SHIFT + the wheel's push: the control a finger rests on, or else
    /// the outlined one, back to its default (0 dB, the pan in the middle,
    /// an effect setting's own default).
    Reset,
    /// SHIFT + wheel, or SHIFT + - / +, on the Synth page: the next or
    /// previous page.
    SynthPage(i32),
    /// SHIFT + - / + on the Sample page: zoom the waveform (Wave, Warp,
    /// Chop); on the pages without one, step the row picked.
    SampleWheel(i32),
    /// The wheel, - / + and the wheel's push on a page with rows (the
    /// Sample and Pads pages' settings, the Synth tab's tiles, Arrange's
    /// Record settings or Edit options): see [`RowKey`].
    Rows(RowKey),
    /// SHIFT + wheel on the Sample page: the previous or next pad with a
    /// sound.
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
    /// Prefs › Appearance › Customise's copy of a theme drawn on the whole
    /// screen, or (None) the theme in use back.
    LiveTheme(Option<Box<mpfree_ui::Theme>>),
    /// Customise's Save theme: the file in the themes folder, what it
    /// says, and the theme it gives.
    SaveTheme {
        file: String,
        text: String,
        theme: Box<mpfree_ui::Theme>,
    },
    /// Customise's Rename: the text keyboard, with the name now.
    NameTheme(String),
    /// A line for the log that the screen also shows in place, long enough
    /// to read ([`crate::gui::TELL_SECONDS`]): why something did nothing,
    /// or was refused (Jay, 12 Nov: say in place when it would do nothing).
    Tell(String),
    /// UNDO (or SHIFT + UNDO, `redo`) on a page without its own in the
    /// app: the screen knows what the page shows, so it picks the history
    /// (Jay, 13 Nov 13:53Z: UNDO follows the page you're on).
    Undo {
        redo: bool,
    },
    /// The UNDO button went down (true) or up, on every page: held, the
    /// screen opens the page's undo history (Jay after live test 3).
    UndoButton(bool),
    /// REC, OVERDUB, or PLAY or STOP while the notes are ready: for what is
    /// armed ([`crate::rec_arm`], sequencer decision 1). The desk finds the
    /// transport and the looper as they are and carries it out.
    Rec(crate::rec_arm::Press),
    /// REC (without SHIFT) while kit pads are held (the kit's pads, in
    /// pad order): the audio loop, which knows which pads have a sound,
    /// gives [`Effect::PadArm`] for the first empty one (sampler R5.7), or
    /// [`Effect::Rec`] as for REC alone.
    PadRec(Vec<usize>),
    /// Arm a pad take onto pad `pad` of the sampler's kit `kit`
    /// ([`crate::rec_arm::RecArm::arm_pad`]).
    PadArm {
        kit: usize,
        pad: usize,
    },
    /// To the Sample tab's recorder, for the pad take
    /// ([`crate::rec_arm::PadDo`]).
    PadTake(crate::rec_arm::PadDo),
    /// What the arp screens show: the panel, the quick band and the chip.
    Arp(ArpView),
    /// The arp's pattern list (names and patterns), when it changes.
    Patterns(Vec<(String, Pattern)>),
    /// The arp's phrase list, whenever it changes: MPFree's own, the
    /// owner's and the open project's.
    Phrases(mpfree_arp_ui::PhraseList),
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
    /// TC with the note editor open: the screen opens its Edit options'
    /// drawer on Timing Correct's settings, or corrects the notes when the
    /// drawer shows already (piano roll plan 1A.1).
    EditTc,
    /// Keep this library text in the store as arp/patterns.txt (the store's
    /// own thread writes it).
    SavePatterns(String),
    /// Keep this library text, the owner's own phrases, in the store as
    /// arp/phrases.txt.
    SavePhrases(String),
    /// STOP in the timed `play` step: end it.
    Quit,
    /// Open the Touch lab page (it starts at its first step).
    TouchLab,
    /// Open Play's patch list (Play's MENU, live shows C1).
    PatchList,
    /// One of the Show view's MENU rows (live shows B1), for the screen's
    /// set player.
    SetMenu(SetMenu),
    /// The wheel, its push or - / + on the Show view (live shows B1).
    ShowKey(ShowKey),
    /// Open the Calibrate touch page (four corner taps).
    CalibrateTouch,
    /// Prefs > Backup and Akai kits: at the next start, a new backup of
    /// the MPC (`backup`) or Akai's kits and samples, or (`want` false) not
    /// after all. The first-start setup's store changes (setup.rs); the
    /// app is told the new state back ([`App::set_setup`]).
    SetupNextStart {
        backup: bool,
        want: bool,
    },
    /// The Dev tab's Touch page: the wheel picks a value (by detents)...
    TouchWheel(i32),
    /// ...and - / + step it (or run the action picked).
    TouchStep(i32),
    /// For the time engine (transport, clips, takes).
    Time(TimeAction),
    /// What the Map controls page shows.
    Map(MapView),
    /// TRACK MUTE came or went, or picked another bank: the PAD BANK
    /// buttons' lights again.
    BankLights,
    /// A PAD BANK button pressed: 0 = A/E to 3 = D/H, and whether SHIFT
    /// was held (E-H). The audio loop knows the bank playing, so it picks.
    Bank {
        button: u8,
        shift: bool,
    },
    /// For the looper and the line input.
    Looper(LoopAction),
    /// A Perform pad's throw held or let go, for the mixer.
    PlayFx(crate::mixing::PlayFx),
    /// The project's Perform set changed: the screen's copy, after an
    /// edit or a project's (`opened`: a project opened or New).
    PerformSet {
        set: Box<crate::perform_pads::PerformSet>,
        opened: bool,
    },
    /// A Perform pad's track mute: track `track` (0 is Track 1) muted or
    /// unmuted, at the launch line when `wait`.
    TrackMute {
        track: usize,
        wait: bool,
    },
    /// MAIN on the Play page (None: the next view; Play has one, All,
    /// since the looper redesign's step 5) and STEP SEQ (Beats).
    PlayView(Option<crate::play_page::PlayView>),
    /// Arrange's view: the Song view (`Some(true)`), the Clips grid
    /// (`Some(false)`), or the next of the three (`None`: Clips, Song, the
    /// song list): MAIN in Perform.
    ArrangeView(Option<bool>),
    /// SAMPLE EDIT: the Sample tab opens on this editor.
    SampleEditor(crate::sample_page::Editor),
    /// PROG EDIT: the sound the pads play: the Synth tab while the synth
    /// has them, otherwise the Pads tab's Pad view (the audio loop knows
    /// which).
    ProgEdit,
    /// PROG EDIT on the Pads tab: Edit pad on the pad picked, or back to
    /// the pads from it (the screen knows which).
    EditPad,
    /// A control for the browser while it shows (BROWSE, or the Pads
    /// tab's Browse view).
    Browse(BrowseKey),
    /// A control for System status while it shows.
    StatusPage(StatusKey),
    /// SHIFT + BROWSE: save the project (the audio loop gathers its
    /// engines' parts and the screen saves them).
    SaveProject,
    /// Sample Recall (sampler R5 part 2): a pad held + SHIFT + REC opens
    /// it on that pad; while it is open PLAY auditions, REC keeps and STOP
    /// closes ([`RecallKey`]).
    Recall(RecallKey),
    /// The metronome on or off, and its level in dBFS (SHIFT + PLAY
    /// START, Play's options, and at the start from the saved settings).
    Metronome {
        on: bool,
        db: i8,
    },
    /// FILL on or off from the coming period: notes with the FILL and
    /// !FILL trig conditions follow it.
    Fill(bool),
}

/// A hardware control for System status.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusKey {
    /// The wheel, or - / +: the next or previous system.
    Step(i32),
    /// The wheel's push: open the picked system's first card.
    Open,
}

/// What a control does to the Pads tab's browser (Browse view).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrowseKey {
    /// The wheel: the selection.
    Wheel(i32),
    /// SHIFT + wheel: the places.
    Places(i32),
    /// The wheel's push: open a folder, or Load.
    Push,
    /// -: up a folder.
    Back,
    /// +: into the folder.
    Enter,
    /// PLAY: hear the sound picked.
    Play,
    /// ERASE: the item picked to the Trash (asks first). The full browser
    /// only.
    Erase,
}

pub struct App {
    pub settings: Settings,
    /// The tempo was set by hand (tapped, typed, turned, from a remote)
    /// since the project opened; false while it is the project's, the
    /// default or a loop's. A free first loop's reading counts a tempo
    /// you meant strongly (the looper's `Command::TempoMeant`).
    pub tempo_by_hand: bool,
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
    /// A track's arp as New leaves it (on the first built-in pattern), to
    /// tell which tracks the project's arp part needs to list.
    arp_fresh: arp::TrackUi,
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
    /// TRACK MUTE's bank: pads 1-16 are tracks 16 * bank + 1 on.
    mute_bank: usize,
    /// TRACK MUTE held (without SHIFT): a pad tapped solos its track.
    mute_hold: Option<MuteHold>,
    /// MAIN held (without SHIFT), as the MPC's hold MAIN: the pads are
    /// tracks, a pad hit picks that one and PAD BANK which 16; let go with
    /// no pad hit, MAIN goes to its page.
    main_hold: Option<MainHold>,
    /// FULL LEVEL held: a pad hit makes the level every hit plays at
    /// (Half Level's, as the MPC's hold FULL LEVEL sets its own level);
    /// let go with no hit, Full Level (SHIFT: Half Level) on or off.
    full_hold: Option<FullHold>,
    /// COPY held over the kit's pads: the pad hit first, whose sound the
    /// pads hit after it get. None until a pad is hit.
    copy_from: Option<u8>,
    /// SHIFT was held as COPY went down: each pad hit is emptied.
    copy_clears: bool,
    /// TAP TEMPO held: the wheel turns the tempo; held alone, the number
    /// pad opens as it is let go.
    tap_hold: Option<TapHold>,
    /// The PAD BANK buttons held, a bit each (A = bit 0): all four
    /// together save a picture of the screen, as on the MPC.
    banks_down: u8,
    /// The Perform set is on the pads (SHIFT + SAMPLER on the Looper tab,
    /// or the Looper's Pads switch), on the Looper tab and Play.
    loop_keys: bool,
    /// The project's Perform set ([`crate::perform_pads::PerformSet`]):
    /// page A the loop keys, B Play's throws.
    perform_set: crate::perform_pads::PerformSet,
    /// The set's page on the pads (PAD BANK A = 0 to H = 7), kept while
    /// the set is off, so SHIFT + SAMPLER brings it back.
    perform_bank: usize,
    /// The Pads tab shows the Perform set (its Perform view): there the
    /// pads pick the set's pads and PAD BANK its pages.
    perform_shown: bool,
    /// The throws Perform pads play (held or latched), each with the strip
    /// it plays on.
    throws_on: Vec<crate::perform_pads::Playing>,
    /// The macros a Perform pad holds, a bit each (Macro 1 the lowest).
    macros_on: u8,
    /// The kit's page on the pads (PAD BANK A = 0 to H = 7): pad n plays
    /// the kit's pad 16 * page + n (sampler R3 part 2b). One page for
    /// every kit, as Akai's MPC has one pad bank.
    pad_page: usize,
    /// The kit pad each pad held pressed, so it lets go of that one even
    /// if the page turned while it was down.
    kit_held: [Option<u8>; 16],
    /// The project's tracks (the audio loop tells it).
    track_count: usize,
    /// The 16 Levels panel shows on Play: the wheel picks its kind.
    levels_open: bool,
    /// The note editor is open: the Sequencer shows its Notes view, whose
    /// wheel picks and changes the Edit options as Arrange's rows.
    notes_open: bool,
    /// 16 Levels are on (the screen says, with the panel).
    levels_on: bool,
    /// 16 Levels' kind is Transpose: the pads move the armed track
    /// (sequencer S4) instead of playing.
    levels_transpose: bool,
    /// STEP SEQ: the pads as steps (`app/step_pads.rs`).
    step_seq: step_pads::StepSeq,
    view: View,
    /// The page to go back to from Console, Options or the guide.
    back: View,
    /// A set is open, and the Show view shows (live shows B1): MENU on
    /// Arrange gives the Show view's rows.
    set_open: bool,
    show_view: bool,
    /// What the chain editor's Back goes back to (the mixer, or the page
    /// it opened on), and the macros panel's.
    chain_back: View,
    macros_back: View,
    /// The page before the Mix tab, where TRACK MIX goes back to.
    mix_back: View,
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
    /// The Prefs tab showing (prefs_tabs.rs): Prefs opens on it again.
    prefs_tab: PrefsTab,
    /// The arp options show the Modifier's section.
    mod_section: bool,
    /// The first-start setup's state for Prefs > Backup and Akai kits;
    /// None where the stick has no setup (not the creator's, or no data
    /// folder).
    setup: Option<crate::setup::PrefsState>,
    /// The wheel's push opened the selected menu row (see `wheel_push`).
    menu_editing: bool,
    /// The wheel's push opened the picked row of a page with rows: the
    /// wheel changes its value until the next push (see [`RowKey`]).
    page_editing: bool,
    /// Theme files found in the themes folder at start (`files/themes/` in
    /// `--data`): the file name, which is what `settings.theme` holds, and
    /// the theme's own name for the Appearance row. After [`THEMES`].
    pub theme_files: Vec<(String, String)>,
    /// Each theme file's problems, by file name, as lines (a line it
    /// couldn't use, an unknown key, an ignored line, a colour hard to
    /// read): the Appearance panel lists them while that theme is in use.
    pub theme_problems: Vec<(String, Vec<String>)>,
    /// Theme files left out of Appearance at start, and why; the panel
    /// names them.
    pub themes_left_out: Vec<(String, String)>,
    /// Starts where the saved theme couldn't be used (its file gone, or
    /// left out) and the default theme showed: a fallback, logged by
    /// [`App::restore`] and counted on the Debug page.
    pub theme_fallbacks: u64,
    /// Prefs > Appearance just changed the theme: "Keep this theme?" asks
    /// until it is kept, or puts the old one back (owner, 22 Nov 07:17Z).
    theme_trial: Option<ThemeTrial>,
    /// The theme files' themes, by file name, for Customise to copy.
    pub theme_looks: Vec<(String, mpfree_ui::Theme)>,
    /// Customise's copy of a theme, kept until Prefs closes for good.
    custom: Option<customise::Custom>,
    /// Customise's copy is on the screen (Customise is open).
    custom_live: bool,
    /// The time [`App::tick`] was last given, in its seconds: when a
    /// theme trial ends.
    tick_t: f64,
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
    /// What each track plays as its own instrument (None: the kit), as
    /// the audio loop last said ([`App::set_track_sound`]); the Synth tab's
    /// Instrument row shows and sets the armed track's.
    /// A free place's instrument (step 6) is here by its holder number
    /// (`instruments::free`), past the tracks'.
    track_sounds: [Option<crate::instruments::Held>; crate::instruments::HOLDERS],
    /// The track whose instrument each track plays though it has none of
    /// its own (step 4), as the audio loop last said
    /// ([`App::set_track_sends`]).
    track_sends: [Option<u8>; mpfree_seq::MAX_TRACKS],
    /// The selected track's line (the instrument redesign's step 11d), as
    /// the audio loop says it: what it plays and where it is heard,
    /// ["Track 2 plays Track 1's Poly", "In the mixer: Poly · Track 1"].
    /// Shown over the Instrument row's help.
    track_line: Vec<String>,
    /// The track each track's own instrument was copied from on the
    /// Instrument row (step 4c), while both play the same engine, so the
    /// row stays on "Copy of Track 3's FM" ([`App::copied_from`]). Not
    /// saved: an open forgets them ([`App::forget_copies`]).
    track_copied: [Option<u8>; mpfree_seq::MAX_TRACKS],
    /// The plugin synths found (node id, name), offered on the Instrument row
    /// after the built-in engines.
    pub plugin_synths: Vec<(&'static str, &'static str)>,
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
    /// The phrases to pick from, in three runs (`app/arp/phrases.rs`):
    /// MPFree's own (the first `phrases_built`), the owner's (up to
    /// `phrases_own`: the store's arp/phrases.txt), then the ones only the
    /// open project has.
    phrases: Vec<(String, mpfree_arp::Phrase)>,
    phrases_built: usize,
    phrases_own: usize,
    /// The armed track's phrase, as a place in `phrases`.
    phrase_index: usize,
    /// The kind of phrase the Phrase tab's list shows when it is not the
    /// pick's: an empty one stepped to (the owner's, before they have
    /// made a phrase). None: the pick's.
    phrase_shelf: Option<mpfree_arp_ui::phrases::Shelf>,
    /// The phrase last picked on each kind's shelf, so stepping back to a
    /// kind returns to it (`app/arp/phrases.rs`).
    phrase_last: [Option<usize>; mpfree_arp_ui::phrases::Shelf::ALL.len()],
    /// New idea or More like this, while open on the Phrase tab
    /// (`app/arp/maker.rs`).
    maker: Option<arp::Maker>,
    /// The Make a phrase sheet, while open on Arrange or in the browser
    /// (`app/arp/make.rs`).
    sheet: Option<arp::Sheet>,
    /// The choices New idea and More like this keep between openings.
    maker_set: arp::MakerSet,
    /// Counts New idea and More like this's openings: each asks the
    /// composer with the next seed, so each opening's tries are new.
    maker_seed: u64,
    /// Improvise on the Phrase tab: the Plays row and Evolve
    /// (`app/arp/improvise.rs`).
    improvise: arp::Improvise,
    /// The Phrase tab's When I stop: Answer (`app/arp/answer.rs`).
    answer: arp::Answer,
    /// The pads play the shared synth on their own (main.rs
    /// `PadsPreview::Synth`), whose pads press notes, so the pads' arp
    /// plays the armed track's phrase there ([`App::set_pads_own`]).
    pads_own_notes: bool,
    /// Where the pads' arp is in its phrase, while it plays one, in ticks
    /// of the phrase: the Phrase tab's cursor follows it then.
    pads_phrase_at: Option<f64>,
    /// The pattern the engine plays (edited, mutated or recorded from the
    /// list's).
    pattern: Pattern,
    mutations: u32,
    /// Recording a pattern on the pads (Step mode) in the workspace.
    recorder: Option<Recorder>,
    /// How the next recording on the pads goes: Step time or Free time.
    record_mode: RecordMode,
    /// Save also keeps the patterns in the store (arp/patterns.txt, when
    /// the store saves: `app --data DIR`), through [`Effect::SavePatterns`],
    /// and the owner's phrases beside them ([`Effect::SavePhrases`]).
    pub pattern_file: bool,
    /// Q-Link turns not yet used, per knob (rate and octaves need several).
    qlink_turns: [i32; 4],
    /// What the arp screens were last sent: the step and the notes, and
    /// whether anything else changed since.
    sent_step: u64,
    /// The Modifier's push the arp screens last showed, in percent, and
    /// the settings it played (they redraw when either moves).
    sent_push: (u8, mpfree_arp::Settings),
    /// The 1/32 of its phrase the Phrase tab's cursor was last sent at
    /// (while the Arp panel shows), and whether a pick waited for the bar
    /// line.
    sent_phrase: (Option<u32>, bool),
    /// Where the sustain pedal went down, while it is down
    /// ([`App::arp_pedal`]).
    pedal_down: Option<arp::PedalTo>,
    /// Latch as it was before the sustain pedal latched the arp.
    pedal_latch: Option<bool>,
    sent_notes: mpfree_arp::NoteSet,
    arp_dirty: bool,
    /// The transport is rolling (the audio loop says each period).
    rolling: bool,
    /// The project is in Perform, not Make (sequencer S4; the audio loop
    /// says each period, [`App::set_perform`]).
    perform: bool,
    /// What REC records, and whether the notes wait for PLAY
    /// ([`crate::rec_arm`]). Not saved: a new or opened project arms the
    /// notes.
    pub rec: crate::rec_arm::RecArm,
    /// What the transport buttons' lights last showed, by note
    /// ([`App::transport_lights`]); `None` until first sent.
    lights_shown: [Option<u8>; 128],
    /// The page showing has something to undo (the screen says): UNDO's
    /// light is bright while idle.
    undo_ready: bool,
    /// A list or pad over the page is open (the number pad, the undo
    /// history): the wheel, its push and - / + go to it.
    overlay: bool,
    /// Sample Recall is open over the page: PLAY, REC and STOP are its
    /// ([`crate::gui::FromUi::Recall`]).
    recall_open: bool,
    /// The pad slot pressed last while the pads play the kit, for Sample
    /// Recall's held pad.
    kit_last: Option<usize>,
    /// A finger rests on a control with a value: the wheel and - / + turn
    /// it, and the push does nothing ([`crate::gui::FromUi::Held`]).
    held: bool,
    /// The screen shows the wheel's outline on a control ([`Nav`]).
    outline: bool,
    /// The synth's pad layout while the synth is on the pads
    /// ([`App::set_pad_layout`]): the pads then feed the arp the layout's
    /// notes, and its events carry notes rather than pad slots.
    pad_layout: Option<[PadNotes; 16]>,
    /// The selected track's own synth's pad map while it plays notes
    /// ([`App::set_arp_layout`], the instrument redesign's step 5d): the
    /// pads then press its notes into the track's arp, which works in
    /// notes, as with the synth's layout; everything else about the pads
    /// stays as on a kit.
    arp_layout: Option<[PadNotes; 16]>,
    /// What the pads play, in one value (the pad map, sampler step 15b,
    /// app/pads_focus.rs): the base for every bank (the selected track's
    /// instrument, or one chosen for them, [`App::pads_focus`]), a bank's
    /// own entry (step 6c's, the design's rule 10, "a bank points at an
    /// instrument; it holds nothing") and the layers (step 11a,
    /// app/pad_layers.rs). Only a choice of yours changes it.
    pad_map: PadMap,
    /// The project's list of instruments as the audio loop last said
    /// ([`App::set_instrument_list`]): what an instrument chosen for the
    /// pads is, and where it is held (step 6).
    instrument_list: crate::instrument_list::InstrumentList,
    /// The song's tuning and each instrument's own (tunings step T5,
    /// `app/tuning_picks.rs`). The Tunings page sets the song's, a Tuning
    /// row an instrument's own (T6, `app/tunings_page.rs`);
    /// `synth_pads.tuning` is what the shared synth plays.
    tunings: Box<crate::tunings::Tunings>,
    /// A track whose own instrument is a copy (Duplicate), waiting for its
    /// id in the list to take the tuning of the instrument it copies (by
    /// that one's id).
    tuning_copies: Box<[Option<crate::instrument_list::InstrumentId>; mpfree_seq::MAX_TRACKS]>,
    /// The options show the Tunings page (tunings step T6,
    /// app/tunings_page.rs), opened from a Tuning row's help.
    tunings_page: bool,
    /// The tracks' own instruments and the free places' that can't follow
    /// a tuning (a plugin that takes only MIDI notes), a bit each, as the
    /// audio loop last said ([`App::set_tuning_deaf`]).
    tuning_deaf: (u128, u16),
    /// The kit each track plays, as the audio loop last said, and the
    /// kits' names from the start: the Tunings page lists and names kits
    /// by them.
    track_kits: Box<[Option<u8>; mpfree_seq::MAX_TRACKS]>,
    kit_names: Vec<String>,
    /// A free place given a synth for the pads, waiting for its id in the
    /// list: the pads play it once it has one (step 6).
    pads_to: Option<usize>,
    /// A free place given a synth for one bank, waiting for its id in the
    /// list, as `pads_to` for every bank.
    bank_to: [Option<usize>; KIT_PAGES],
    /// A free place given a layer's synth, waiting for its id in the list,
    /// as `pads_to`.
    layer_to: [Option<usize>; pad_layers::MORE],
    /// The synths MIDI routes name (step 11e, `app/route_synths.rs`), row
    /// by row, as MIDI in last said.
    route_synths: [Option<crate::instrument_list::InstrumentId>; route_synths::ROUTE_SYNTHS],
    /// The synth Add a part made for a new track's tries (composing tools
    /// S2.6b, app/add_sound.rs).
    add_sound: Option<add_sound::AddSynth>,
    /// The list of synths, while open (app/synth_pick.rs).
    synth_pick: Option<synth_pick::SynthPick>,
    /// What the pads play is open over the Synth tab (sampler step 15e,
    /// app/pad_map_panel.rs).
    map_panel: bool,
    /// The pad map before each change made by a pick, Follow or Remove,
    /// the last last: what UNDO takes back while the panel is open.
    map_undo: Vec<PadMap>,
    /// The tracks that play the shared synth, a bit each
    /// ([`App::set_synth_tracks`]).
    synth_tracks: u128,
    /// The tracks that play each kit the pad map names (sampler step 15,
    /// its base and banks from step 15c, its layers and the routes' kits
    /// from 15f, [`App::set_kit_tracks`]): the
    /// kit place and a bit for each track; the rest `(usize::MAX, 0)`.
    kit_tracks: [(usize, u128); pads_focus::MAP_KITS],
    /// The tracks with a kit or a MIDI output picked, a bit each
    /// ([`App::set_configured`]).
    configured: u128,
    /// What each held pad pressed into the arp (kept so a layout change
    /// while a pad is down still lets go of the right notes).
    pad_pressed: [PadNotes; 16],
    /// How many held pads and keys press each note (two chord pads, or a
    /// pad and a key, can share one).
    note_holds: [u8; 128],
    /// Keys held on a MIDI keyboard ([`App::key_down`]), by note.
    keys_held: [bool; 128],
    /// The control mapper (Debug mode).
    mapper: mapper::Mapper,
    /// Buttons pressed while the mapper listened: their release is kept
    /// from the rest of the app too.
    swallowed: [bool; 128],
    /// Where the sound runs in this start (cores.rs); Prefs' "Sound cores"
    /// says "(next start)" while the setting differs.
    sound_cores_now: SoundCores,
    /// The looper's size in this start, and the rate it is counted at;
    /// Prefs' "Loop memory" says "(next start)" while the setting differs.
    loop_memory_now: LoopMemory,
    /// The audio tracks' size in this start.
    audio_memory_now: crate::audiotracks::AudioMemory,
    sample_rate: u32,
    /// Prefs or a button changed since the last look (`saved.rs`), and
    /// the text last handed to the store for "app" and "map".
    save_dirty: bool,
    saved: [String; 2],
    /// Something the project keeps (a Synth tab option, the arp) changed
    /// with the controls since the audio loop last looked: the screen
    /// counts the work as unsaved ([`App::take_project_edit`]).
    project_edit: bool,
    /// The arps as the project last saw them (`arp/part.rs`): any change
    /// from the panel, its options, the workspace or a Q-Link is an edit.
    arp_seen: u64,
}

impl App {
    pub fn new(sample_rate: u32, timed: bool) -> App {
        let settings = Settings::default();
        let arps = TrackArps::new(sample_rate, settings.bpm);
        let patterns = arp::built_in_patterns();
        let pattern = patterns.first().map(|(_, p)| *p).unwrap_or_default();
        let phrases = arp::built_in_phrases();
        let sent_notes = arps.engine_at(0).active().clone();
        let mut app = App {
            settings,
            tempo_by_hand: false,
            map: ButtonMap::measured(),
            arps,
            track: 0,
            track_ui: [arp::TrackUi::default(); mpfree_seq::MAX_TRACKS],
            arp_fresh: arp::TrackUi::fresh(pattern),
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
            mute_bank: 0,
            mute_hold: None,
            main_hold: None,
            full_hold: None,
            copy_from: None,
            copy_clears: false,
            tap_hold: None,
            banks_down: 0,
            loop_keys: false,
            perform_set: crate::perform_pads::PerformSet::default(),
            perform_bank: 0,
            perform_shown: false,
            throws_on: Vec::new(),
            macros_on: 0,
            pad_page: 0,
            kit_held: [None; 16],
            track_count: 0,
            levels_open: false,
            notes_open: false,
            levels_on: false,
            levels_transpose: false,
            step_seq: step_pads::StepSeq::default(),
            view: View::Play,
            back: View::Play,
            chain_back: View::Mixer,
            macros_back: View::Play,
            mix_back: View::Play,
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
            track_sounds: [None; crate::instruments::HOLDERS],
            track_sends: [None; mpfree_seq::MAX_TRACKS],
            track_line: Vec::new(),
            track_copied: [None; mpfree_seq::MAX_TRACKS],
            plugin_synths: Vec::new(),
            pad_scale_pick: 0,
            pad_scale_root: 0,
            clock: None,
            swing_from_transport: true,
            patterns,
            pattern_index: 0,
            phrases_built: phrases.len(),
            phrases_own: phrases.len(),
            phrases,
            phrase_index: 0,
            phrase_shelf: None,
            phrase_last: [None; mpfree_arp_ui::phrases::Shelf::ALL.len()],
            maker: None,
            sheet: None,
            maker_set: arp::MakerSet::default(),
            maker_seed: 0,
            improvise: arp::Improvise::default(),
            answer: arp::Answer::default(),
            pads_own_notes: false,
            pads_phrase_at: None,
            pattern,
            mutations: 0,
            recorder: None,
            record_mode: RecordMode::Step,
            pattern_file: false,
            qlink_turns: [0; 4],
            sent_step: 0,
            sent_push: (0, mpfree_arp::Settings::default()),
            sent_phrase: (None, false),
            pedal_down: None,
            pedal_latch: None,
            sent_notes,
            arp_dirty: true,
            rolling: false,
            set_open: false,
            show_view: false,
            perform: false,
            rec: crate::rec_arm::RecArm::default(),
            lights_shown: Self::startup_transport_lights(),
            undo_ready: false,
            overlay: false,
            recall_open: false,
            kit_last: None,
            held: false,
            outline: false,
            pad_layout: None,
            arp_layout: None,
            pad_map: PadMap::default(),
            instrument_list: crate::instrument_list::InstrumentList::new(),
            tunings: Box::new(crate::tunings::Tunings::new()),
            tuning_copies: Box::new([None; mpfree_seq::MAX_TRACKS]),
            tunings_page: false,
            tuning_deaf: (0, 0),
            track_kits: Box::new([None; mpfree_seq::MAX_TRACKS]),
            kit_names: Vec::new(),
            pads_to: None,
            add_sound: None,
            bank_to: [None; KIT_PAGES],
            layer_to: [None; pad_layers::MORE],
            route_synths: [None; route_synths::ROUTE_SYNTHS],
            synth_pick: None,
            map_panel: false,
            map_undo: Vec::new(),
            synth_tracks: 0,
            kit_tracks: [(usize::MAX, 0); pads_focus::MAP_KITS],
            configured: 0,
            pad_pressed: [PadNotes::default(); 16],
            note_holds: [0; 128],
            keys_held: [false; 128],
            mapper: mapper::Mapper::default(),
            swallowed: [false; 128],
            prefs_tab: PrefsTab::Playing,
            mod_section: false,
            setup: None,
            menu_editing: false,
            page_editing: false,
            theme_files: Vec::new(),
            theme_problems: Vec::new(),
            themes_left_out: Vec::new(),
            theme_fallbacks: 0,
            theme_trial: None,
            theme_looks: Vec::new(),
            custom: None,
            custom_live: false,
            tick_t: 0.0,
            tuning_files: Vec::new(),
            sound_cores_now: SoundCores::Shared,
            loop_memory_now: LoopMemory::default(),
            audio_memory_now: crate::audiotracks::AudioMemory::default(),
            sample_rate,
            save_dirty: false,
            saved: Default::default(),
            project_edit: false,
            arp_seen: 0,
        };
        // Every track's arp starts on the first pattern and the first
        // phrase, as the panel does. Settings alone don't start an engine,
        // so this needn't mark all 128 in use.
        let phrase = app.phrases.first().map(|(_, p)| *p).unwrap_or_default();
        for (e, u) in app.arps.engines_mut().zip(app.track_ui.iter_mut()) {
            e.set_pattern(pattern);
            e.set_phrase(phrase);
            u.pattern = pattern;
        }
        app.update_engine();
        app.arp_seen = app.arp_fingerprint();
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

    /// A set is open, and whether the Show view shows (the screen tells
    /// it, live shows B1): MENU on the Show view gives its own rows.
    pub fn set_show_state(&mut self, open: bool, showing: bool) {
        self.set_open = open;
        self.show_view = open && showing;
    }

    /// A set is open (the screen says, live shows B1).
    pub fn set_open(&self) -> bool {
        self.set_open
    }

    /// A learned Show control that is the app's own (live shows B1):
    /// Panic is SHIFT + STOP's (stop, silence everything, back to bar 1),
    /// Click is SHIFT + PLAY START's. None for the set player's controls,
    /// which the screen takes.
    pub fn show_press(&mut self, id: u16) -> Option<Vec<Effect>> {
        use mpfree_play::mpfree_param::show;
        match id {
            show::PANIC => {
                let mut fx = self.stop();
                fx.push(Effect::Looper(LoopAction::StopFree));
                fx.push(Effect::Time(TimeAction::Transport(Command::StopAndRewind)));
                fx.push(Effect::Tell(
                    "Panic: stopped, every sound silenced".to_string(),
                ));
                Some(fx)
            }
            show::CLICK => {
                self.settings.metronome = !self.settings.metronome;
                self.save_dirty = true;
                let mut fx = vec![
                    self.metronome(),
                    Effect::Tell(format!(
                        "Click {}",
                        if self.settings.metronome { "on" } else { "off" }
                    )),
                ];
                fx.push(Effect::Settings(self.settings.clone()));
                Some(fx)
            }
            _ => None,
        }
    }

    /// Whether the transport rolls, as the audio loop last told it.
    pub fn rolling(&self) -> bool {
        self.rolling
    }

    /// Make or Perform, as the project has it (the audio loop tells it
    /// each period). Perform pins the arm (the design's "the arm is pinned
    /// in Perform"), Make lets it follow the page again; true when that
    /// changed, so the screen hears the arm.
    pub fn set_perform(&mut self, on: bool) -> bool {
        if self.perform == on {
            return false;
        }
        self.perform = on;
        self.rec.pinned = on;
        true
    }

    /// The tempo, in BPM.
    pub fn bpm(&self) -> f64 {
        self.settings.bpm
    }

    /// A learned control pressing (`down`) or letting go of one of the
    /// MPC's buttons (MIDI Learn's remote control, `remote.rs`; `id` from
    /// `mpfree_param::transport`): as the button itself, on the page
    /// showing. Not while Map controls listens, which would take it for
    /// the MPC's own button.
    pub fn remote_press(&mut self, id: u16, down: bool, t: f64) -> Vec<Effect> {
        use mpfree_play::mpfree_param::transport as tr;
        let note = match id {
            tr::PLAY => Some(button::PLAY),
            tr::STOP => Some(button::STOP),
            tr::PLAY_START => self.map.play_start,
            tr::REC => self.map.rec,
            tr::OVERDUB => self.map.overdub,
            tr::TAP_TEMPO => self.map.tap_tempo,
            _ => None,
        };
        match note {
            Some(n) if self.view != View::Map => self.button(n, down, t),
            _ => Vec::new(),
        }
    }

    /// Other gear's MMC command (the Sync section's MMC in), as the MPC's
    /// own PLAY, STOP and REC do, through the arm (`rec_arm::Press`, the
    /// sequencer thread's rule, 18 Nov): Play and Deferred Play as PLAY
    /// (the take that is ready starts), Stop and Pause as STOP (an armed
    /// pad take is called off, one that records keeps recording and the
    /// music plays on), Record Strobe as REC (what is armed records). Not
    /// as the buttons: Play while playing is nothing (a DAW's Play is no
    /// FILL), and nothing depends on the page, SHIFT or held pads. Locate
    /// is the desk's (`desk::mmc_locate`); the rest do nothing.
    pub fn mmc(&mut self, c: mpfree_midi::mmc::Mmc) -> Vec<Effect> {
        use crate::rec_arm::Press;
        use mpfree_midi::mmc::Mmc;
        let mut fx = Vec::new();
        match c {
            Mmc::Play | Mmc::DeferredPlay if self.rolling => {}
            Mmc::Play | Mmc::DeferredPlay => {
                fx.extend(self.fill_off());
                self.arps.engine_at_mut(self.track).resume();
                fx.push(if self.rec.ready.is_some() {
                    Effect::Rec(Press::Play { from_start: false })
                } else {
                    Effect::Time(TimeAction::Transport(Command::Play))
                });
            }
            Mmc::Stop | Mmc::Pause => {
                fx.extend(self.fill_off());
                if self.rec.ready.is_some() || self.rec.pad.is_some() {
                    fx.push(Effect::Rec(Press::Stop));
                }
                if self.rec.pad.is_some_and(|p| p.recording) {
                    return fx;
                }
                if self.rolling {
                    fx.push(Effect::Time(TimeAction::Transport(Command::Stop)));
                    fx.push(Effect::Note("STOP: transport stopped".to_string()));
                } else {
                    fx.extend(self.stop());
                    fx.push(Effect::Time(TimeAction::Transport(Command::Stop)));
                }
            }
            Mmc::RecordStrobe => fx.push(Effect::Rec(Press::Rec)),
            Mmc::Locate(_) | Mmc::RecordExit | Mmc::Other(_) => {}
        }
        fx
    }

    /// The project's tracks, for TRACK MUTE's hint line (the audio loop
    /// tells it each period).
    pub fn set_track_count(&mut self, n: usize) {
        self.track_count = n;
    }

    /// TRACK MUTE's bank: pads 1-16 are tracks `16 * bank + 1` to
    /// `16 * bank + 16`.
    pub fn mute_bank(&self) -> usize {
        self.mute_bank
    }

    /// The kit's page on the pads: PAD BANK A = 0 to H = 7.
    pub fn pad_page(&self) -> usize {
        self.pad_page
    }

    /// PAD BANK picked page `page` of the kit (0 = A, up to 7 = H). Pads
    /// held keep the kit pads they pressed until let go. A bank that plays
    /// an instrument of its own (step 6c) puts it on the pads, said to the
    /// screen; nothing allocated otherwise.
    pub fn set_pad_page(&mut self, page: usize) -> Vec<Effect> {
        let was = (self.pads_focus(), self.pads_bank());
        self.pad_page = page.min(KIT_PAGES - 1);
        self.bank_moved(was)
    }

    /// The kit pad that pad `slot` (0 to 15) plays: the one it pressed
    /// while held, else the page's, 16 * page + slot, the page being the
    /// source bank of a bank's own kit ([`App::kit_page`], sampler step
    /// 15e). With the synth's pad layout on the pads, the slot itself.
    pub fn kit_pad(&self, slot: usize) -> usize {
        if self.pad_layout.is_some() {
            return slot;
        }
        match self.kit_held.get(slot).copied().flatten() {
            Some(k) => usize::from(k),
            None => self.kit_page() * 16 + slot,
        }
    }

    /// What TRACK MUTE's pads are in the bank picked, for the log.
    fn mute_bank_line(&self) -> String {
        format!(
            "TRACK MUTE · {}",
            mute_bank_words(self.mute_bank, self.track_count)
        )
    }

    /// The transport lights as Akai's startup leaves them: STOP bright,
    /// PLAY, REC and OVERDUB dark, UNDO dim.
    fn startup_transport_lights() -> [Option<u8>; 128] {
        let mut l = [None; 128];
        l[button::STOP as usize] = Some(LIGHT_ON);
        l[button::PLAY as usize] = Some(LIGHT_DARK);
        l
    }

    /// The transport buttons' lights for what the transport and the
    /// looper are doing now, at `t` seconds, as Akai's app lights them
    /// (live test 3 capture, `docs/dev/hardware/controller-protocol.md`):
    /// - PLAY bright while rolling, dark when stopped; while REC or
    ///   OVERDUB waits for PLAY it blinks bright/dark with them.
    /// - REC blinks bright/dark while armed and while recording; OVERDUB
    ///   the same while armed and overdubbing. Dark otherwise.
    /// - UNDO blinks bright/dim while recording or overdubbing; otherwise
    ///   bright while the page has something to undo, dim when not. (That
    ///   idle light is inferred: an earlier capture saw Akai's app take
    ///   UNDO from dim to bright during Q-Link edits. Not confirmed.)
    /// - STOP dim while rolling, bright when stopped (the MPC One's STOP
    ///   has no LED; harmless).
    ///
    /// Only the lights that changed, as [`Effect::Light`]. REC, OVERDUB and
    /// UNDO are lit on their known buttons (unless Map controls cleared one).
    /// The screen says whether the page showing has something to undo;
    /// the UNDO light follows at the next [`Self::transport_lights`].
    pub fn set_undo_ready(&mut self, ready: bool) {
        self.undo_ready = ready;
    }

    /// The screen opened (true) or closed a list or pad over the page
    /// ([`crate::gui::FromUi::Overlay`]).
    pub fn set_overlay(&mut self, on: bool) {
        self.overlay = on;
    }

    /// The screen opened (true) or closed Sample Recall
    /// ([`crate::gui::FromUi::Recall`]): PLAY, REC and STOP are its while
    /// it is open.
    pub fn set_recall(&mut self, on: bool) {
        self.recall_open = on;
    }

    /// Sample Recall is open over the page.
    pub fn recall_open(&self) -> bool {
        self.recall_open
    }

    /// The kit pad a pad held + SHIFT + REC opens Sample Recall on (sampler
    /// decision 3): the one pressed last of those held while the pads play
    /// the kit. None while they are steps (STEP SEQ), mute keys (TRACK
    /// MUTE), levels or transpose (16 LEVEL), loop keys or the synth's
    /// notes: SHIFT + REC is Capture there, as with no pad held (the
    /// sequencer thread, 18 Nov).
    pub fn recall_pad(&self) -> Option<usize> {
        if self.pad_layout.is_some()
            || self.step_pads().is_some()
            || self.levels_on
            || self.pads_mute()
            || self.pads_are_perform()
        {
            return None;
        }
        let held = |slot: usize| self.kit_held.get(slot).copied().flatten();
        self.kit_last
            .and_then(held)
            .or_else(|| self.kit_held.iter().flatten().next().copied())
            .map(usize::from)
    }

    /// The screen says whether a finger rests on a control with a value
    /// ([`crate::gui::FromUi::Held`]): until it lifts the wheel and - / +
    /// turn that control.
    pub fn set_held(&mut self, on: bool) {
        self.held = on;
    }

    /// The wheel, or - / +, goes to the control a finger rests on: not
    /// while a list or pad covers the page (no finger can be on the page
    /// under it then).
    fn wheel_to_held(&self) -> bool {
        self.held && !self.overlay && !matches!(self.view, View::Guide | View::System)
    }

    /// The screen says whether the wheel's outline is on a control: on the
    /// mixer, the chain editor and the macros panel the wheel then moves it
    /// (and turns their Q-Link 1 when there is none).
    pub fn set_outline(&mut self, shown: bool) {
        self.outline = shown;
    }

    pub fn transport_lights(&mut self, mut now: TransportLights, t: f64) -> Vec<Effect> {
        // The notes ready blink REC (or OVERDUB) and PLAY, as an armed
        // loop does.
        let (rec, dub) = self.rec.ready_lights();
        now.recording |= rec;
        now.overdub |= dub;
        let blink_on = (t / BLINK_HALF_SECONDS).floor().rem_euclid(2.0) < 1.0;
        let blink = |on: bool, low: u8| match (on, blink_on) {
            (false, _) => low,
            (true, true) => LIGHT_ON,
            (true, false) => {
                if low == LIGHT_DARK {
                    LIGHT_DARK
                } else {
                    LIGHT_OFF
                }
            }
        };
        let waiting = !now.rolling && (now.recording || now.overdub);
        let play = if waiting {
            blink(true, LIGHT_DARK)
        } else if now.rolling {
            LIGHT_ON
        } else {
            LIGHT_DARK
        };
        let stop = if now.rolling { LIGHT_OFF } else { LIGHT_ON };
        let mut want: Vec<(u8, u8)> = vec![(button::STOP, stop), (button::PLAY, play)];
        if let Some(rec) = self.map.rec {
            want.push((rec, blink(now.recording, LIGHT_DARK)));
        }
        if let Some(dub) = self.map.overdub {
            want.push((dub, blink(now.overdub, LIGHT_DARK)));
        }
        if let Some(undo) = self.map.undo {
            let on = now.rolling && (now.recording || now.overdub);
            let idle = if self.undo_ready { LIGHT_ON } else { LIGHT_OFF };
            if on || self.undo_ready || self.lights_shown[undo as usize & 0x7F].is_some() {
                want.push((undo, blink(on, idle)));
            }
        }
        let mut fx = Vec::new();
        for (note, v) in want {
            let slot = &mut self.lights_shown[note as usize & 0x7F];
            if *slot != Some(v) {
                *slot = Some(v);
                fx.push(Effect::Light(note, v));
            }
        }
        fx
    }

    /// The metronome as the settings have it, for the time engine.
    fn metronome(&self) -> Effect {
        Effect::Metronome {
            on: self.settings.metronome,
            db: self.settings.metronome_db,
        }
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
            Effect::Phrases(self.phrase_list()),
            self.arp_view(),
        ];
        fx.extend(self.repeat_light());
        fx.push(self.metronome());
        let s = &self.settings;
        fx.push(Effect::Looper(LoopAction::Source(s.loop_source)));
        fx.push(Effect::Looper(LoopAction::Command(LoopCommand::SetBars(
            s.loop_bars,
        ))));
        fx.push(Effect::Looper(LoopAction::Command(
            LoopCommand::SetLatency(s.loop_latency_now()),
        )));
        fx.push(Effect::Looper(LoopAction::Command(LoopCommand::SetLevel(
            10f32.powf(s.loop_level_db / 20.0),
        ))));
        // Every button is built in (ButtonMap::measured): nothing asks for
        // them at power-on, and the control mapper (Debug mode) can still
        // give one another number.
        fx.push(Effect::Show(self.view));
        fx
    }

    /// The velocity a hit plays at, after Full or Half Level and the curve.
    pub fn velocity(&self, v: u8) -> u8 {
        match self.pad_level {
            PadLevel::Full => 127,
            PadLevel::Half => self.settings.half_level,
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
        self.pad_down_on(slot, None, velocity)
    }

    /// [`App::pad_down`], on a kit's pad `on` (bank × 16 + place) in place
    /// of the page's: a Loop pad's hit that waited for the launch line
    /// plays the pad it was made on, though the bank turned since, and
    /// its let-go ([`App::pad_up`]) lets go of that one (sampler R5.3b).
    pub fn pad_down_on(&mut self, slot: usize, on: Option<u8>, velocity: u8) -> Vec<Effect> {
        self.step_pad_played(slot);
        let pad = on.unwrap_or(self.kit_pad(slot) as u8);
        if !self.free_time_recording() {
            // In Free time the pads only write steps; the pattern keeps
            // playing the latched notes.
            self.press_pad_on(slot, on, velocity.max(1));
        }
        self.record_press(pad, velocity)
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
        if slot >= 16 {
            return None;
        }
        // A kit's pad on the page (or the one it pressed), else the slot.
        self.pressed_pad_event(self.kit_pad(slot) as u8, velocity, down)
    }

    /// The same for kit pad `pad` (0 to 127) itself: a key on a kit
    /// ([`App::kit_key_down`]).
    pub fn pressed_pad_event(
        &self,
        note: u8,
        velocity: u8,
        down: bool,
    ) -> Option<mpfree_play::Event> {
        if self.free_time_recording() {
            return None;
        }
        Some(if down {
            mpfree_play::Event::NoteOn {
                note,
                velocity: self.live_map().velocity(velocity.max(1)),
            }
        } else {
            mpfree_play::Event::NoteOff { note }
        })
    }

    /// The pad hits and let-gos made before the next period land `offset`
    /// frames into it on the armed track's arp, not at its top: a hit
    /// that waited for the launch line plays on the line (sampler R5.3b).
    pub fn pads_at(&mut self, offset: u32) {
        self.arps.engine_at_mut(self.track).set_at(offset);
    }

    pub fn pad_up(&mut self, slot: usize) -> Vec<Effect> {
        self.step_pad_let_go(slot);
        let pad = self.kit_pad(slot) as u8;
        if !self.free_time_recording() {
            self.release_pad(slot);
        }
        if let Some(h) = self.kit_held.get_mut(slot) {
            *h = None;
        }
        self.record_release(pad)
    }

    pub fn pressure(&mut self, slot: usize, value: u8) {
        if self.press_layout().is_none() {
            let pad = self.kit_pad(slot) as u8;
            self.arps.engine_at_mut(self.track).pressure(pad, value);
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
        match self.press_layout() {
            None => self
                .arps
                .engine_at(self.track)
                .active()
                .contains(self.kit_pad(slot) as u8),
            Some(l) => l.get(slot).is_some_and(|p| {
                p.notes()
                    .iter()
                    .any(|&n| self.arps.engine_at(self.track).active().contains(n))
            }),
        }
    }

    /// ERASE is held (a step pad pressed now is cleared).
    pub fn erase_held(&self) -> bool {
        self.erase_held
    }

    /// Whether hardware button `note` is ERASE.
    pub fn is_erase(&self, note: u8) -> bool {
        Some(note) == self.map.erase
    }

    /// SHIFT is held (a step pad pressed now on an empty step holds it
    /// with no note, for a lock with no note).
    pub fn shift_held(&self) -> bool {
        self.shift
    }

    /// TRACK MUTE: whether a pad press plays, or mutes a track or a pad
    /// (the audio loop applies it, and lights the pads).
    pub fn mute_keys(&self) -> MuteKeys {
        self.mute_keys
    }

    /// The pads are mute keys: TRACK MUTE's mode is on, or TRACK MUTE is
    /// held (a pad solos its track).
    pub fn pads_mute(&self) -> bool {
        self.mute_keys != MuteKeys::Off || self.mute_hold.is_some()
    }

    /// The kit pads held now, in pad order, while the pads play the kit
    /// (not the synth's layouts, STEP SEQ, TRACK MUTE, 16 Levels or the
    /// loop keys): REC with an empty one held arms a pad take (sampler
    /// R5.7).
    pub fn held_kit_pads(&self) -> Vec<usize> {
        if self.pad_layout.is_some()
            || self.pads_chose_synth()
            || self.pads_are_steps()
            || self.pads_mute()
            || self.levels_on
            || self.pads_are_loop_keys()
        {
            return Vec::new();
        }
        self.kit_held
            .iter()
            .flatten()
            .map(|&k| usize::from(k))
            .collect()
    }

    /// The pads transpose the armed track (16 Levels' Transpose kind,
    /// sequencer S4): pad 9 plays as written, each pad either side one
    /// step more, -8 to +7.
    pub fn pads_transpose(&self) -> bool {
        self.levels_on && self.levels_transpose
    }

    /// The pads are the Perform set now (the loop keys on its page A):
    /// turned on (SHIFT + SAMPLER on the Looper tab, or the deck's Pads
    /// switch), on the Looper tab or Play. Elsewhere they play the kit,
    /// and come back as the set on return.
    pub fn pads_are_loop_keys(&self) -> bool {
        self.loop_keys && matches!(self.view, View::Looper | View::Play)
    }

    /// The Pads tab shows the Perform set: the pads pick its pads (a
    /// throw plays while held, so it can be heard) and PAD BANK turns its
    /// pages. The kit's pads don't play.
    pub fn perform_editing(&self) -> bool {
        self.perform_shown && self.view == View::Pads
    }

    /// The pads are the Perform set's: on the pads (Play, the Looper
    /// tab) or being edited (the Pads tab's Perform view). PAD BANK turns
    /// its pages and the pads light by it.
    pub fn pads_are_perform(&self) -> bool {
        self.pads_are_loop_keys() || self.perform_editing()
    }

    /// The Pads tab shows the Perform set (`shown`), or the kit's pads
    /// again: PAD BANK's lights follow.
    pub fn show_perform(&mut self, shown: bool) -> Vec<Effect> {
        if shown == self.perform_shown {
            return Vec::new();
        }
        self.perform_shown = shown;
        vec![
            self.status(),
            Effect::BankLights,
            Effect::Note(if shown {
                format!(
                    "Pads tab: the Perform set, {} (the pads pick its pads; PAD BANK: its pages)",
                    self.perform_set.page_name(self.perform_bank)
                )
            } else {
                "Pads tab: the kit's pads again".to_string()
            }),
        ]
    }

    /// The Perform view's bank strip: page `page` shown, and on the pads
    /// (as PAD BANK picks it).
    pub fn perform_page_to(&mut self, page: usize) -> Vec<Effect> {
        let page = page.min(crate::perform_pads::PAGES - 1);
        if page == self.perform_bank {
            return Vec::new();
        }
        self.perform_bank = page;
        vec![self.status(), Effect::BankLights]
    }

    /// The Perform view's On the pads switch turned on: the set on the
    /// pads at the page shown (SHIFT + SAMPLER keeps the page too).
    pub fn perform_on(&mut self) -> Vec<Effect> {
        self.set_loop_keys(true)
    }

    /// Pad `pad` of page `page` of the Perform set does `action` now (the
    /// Perform view's edits and its UNDO). A throw the pad played is let
    /// go, so nothing keeps playing that no pad holds. The project is
    /// unsaved; the screen gets the new set.
    pub fn set_perform_pad(
        &mut self,
        page: usize,
        pad: usize,
        action: crate::perform_pads::PadAction,
    ) -> Vec<Effect> {
        use crate::perform_pads::PadAction;
        let Some(slot) = self
            .perform_set
            .pages
            .get_mut(page)
            .and_then(|p| p.pads.get_mut(pad))
        else {
            return Vec::new();
        };
        let before = std::mem::replace(slot, action);
        let mut fx = Vec::new();
        if let PadAction::Throw(t) = before {
            let p = t.playing();
            if before != action && self.throws_on.contains(&p) {
                self.throws_on.retain(|&q| q != p);
                fx.extend(crate::perform_pads::release_fx(p).map(Effect::PlayFx));
            }
        }
        fx.push(Effect::PerformSet {
            set: Box::new(self.perform_set.clone()),
            opened: false,
        });
        fx
    }

    /// The Perform set's page on the pads.
    pub fn perform_page(&self) -> &crate::perform_pads::PerformPage {
        self.perform_set.page(self.perform_bank)
    }

    /// The Perform set's page on the pads: PAD BANK A = 0 to H = 7.
    pub fn perform_bank(&self) -> usize {
        self.perform_bank
    }

    /// The project's Perform set, for its save.
    pub fn perform_set(&self) -> &crate::perform_pads::PerformSet {
        &self.perform_set
    }

    /// A Perform set in place of the project's. A throw a pad was
    /// playing is let go; the page on the pads stays, and with the set on
    /// the screen shows the new page's name.
    pub fn set_perform_set(&mut self, set: crate::perform_pads::PerformSet) -> Vec<Effect> {
        let mut fx = self.throws_off();
        self.perform_set = set;
        if self.loop_keys {
            fx.push(self.status());
        }
        fx.push(Effect::PerformSet {
            set: Box::new(self.perform_set.clone()),
            opened: true,
        });
        fx
    }

    /// A project's Perform set opened (`text`: its `perform` part; None,
    /// New or a project saved before the part, gives a new project's
    /// set): the lines it skipped, for the log, and its effects.
    pub fn open_perform(&mut self, text: Option<&str>) -> (Vec<String>, Vec<Effect>) {
        use crate::perform_pads::PerformSet;
        let (set, skipped) =
            text.map_or_else(|| (PerformSet::default(), Vec::new()), PerformSet::parse);
        (skipped, self.set_perform_set(set))
    }

    /// The throws Perform pads play now (for their lights).
    pub fn throws_on(&self) -> &[crate::perform_pads::Playing] {
        &self.throws_on
    }

    /// A throw pad pressed (`down`) or let go: whether its throw now plays,
    /// when that changed. Held: it plays while the pad is down. Latched:
    /// a press starts it and the next lets it go; the lift does nothing.
    pub fn perform_throw(&mut self, t: crate::perform_pads::Throw, down: bool) -> Option<bool> {
        use crate::perform_pads::Hold;
        let p = t.playing();
        let was = self.throws_on.contains(&p);
        let now = match (t.hold, down) {
            // A bar's throw: lit while held; the Perform FX plays its bar.
            (Hold::WhileHeld | Hold::Bar, d) => d,
            (Hold::Latch, true) => !was,
            (Hold::Latch, false) => was,
        };
        if now == was {
            return None;
        }
        if now {
            self.throws_on.push(p);
        } else {
            self.throws_on.retain(|&q| q != p);
        }
        Some(now)
    }

    /// A macro pad pressed (`down`) or let go: macro `card` held by a pad
    /// or not (for its light).
    pub fn perform_macro(&mut self, card: u8, down: bool) {
        let bit = 1u8.checked_shl(u32::from(card)).unwrap_or(0);
        if down {
            self.macros_on |= bit;
        } else {
            self.macros_on &= !bit;
        }
    }

    /// The macros Perform pads hold now (for their lights).
    pub fn macros_on(&self) -> u8 {
        self.macros_on
    }

    /// Page `page` of the Perform set named `name` (the Perform view's
    /// Name page and its UNDO). The project is unsaved; the screen gets
    /// the new set.
    /// Page `page` of the Perform set is `kind` now (the Arrange
    /// refactor's 5.5): pads of its own, a clip window or a bank of
    /// scenes. A throw or macro its pads held lets go, as when the page
    /// turns.
    pub fn set_perform_kind(
        &mut self,
        page: usize,
        kind: crate::perform_pads::PageKind,
    ) -> Vec<Effect> {
        let Some(p) = self.perform_set.pages.get_mut(page) else {
            return Vec::new();
        };
        p.kind = kind;
        let mut fx = Vec::new();
        if self.loop_keys {
            fx.push(self.status());
        }
        fx.push(Effect::PerformSet {
            set: Box::new(self.perform_set.clone()),
            opened: false,
        });
        fx
    }

    pub fn set_perform_name(&mut self, page: usize, name: &str) -> Vec<Effect> {
        let Some(p) = self.perform_set.pages.get_mut(page) else {
            return Vec::new();
        };
        p.name = name.trim().to_string();
        let mut fx = Vec::new();
        if self.loop_keys {
            fx.push(self.status());
        }
        fx.push(Effect::PerformSet {
            set: Box::new(self.perform_set.clone()),
            opened: false,
        });
        fx
    }

    /// Every throw a Perform pad plays let go (the set off, another
    /// project).
    fn throws_off(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.throws_on)
            .into_iter()
            .filter_map(crate::perform_pads::release_fx)
            .map(Effect::PlayFx)
            .collect()
    }

    /// A track mute pad's change waits for the launch line (Perform;
    /// SHIFT gives the other timing), as TRACK MUTE's pads.
    pub fn mute_waits(&self) -> bool {
        self.perform != self.shift
    }

    /// The Looper's Pads switch (the deck's and the loop row's): the
    /// Perform set on the pads at page A, the loop keys; with page A on
    /// the pads already, off. One pad mode at a time: STEP SEQ, TRACK
    /// MUTE and 16 Levels give the pads up to it.
    pub fn toggle_loop_keys(&mut self) -> Vec<Effect> {
        if self.loop_keys && self.perform_bank == 0 {
            return self.set_loop_keys(false);
        }
        let turned = self.perform_bank != 0;
        self.perform_bank = 0;
        let mut fx = self.set_loop_keys(true);
        if turned && fx.is_empty() {
            // On already, on another page: page A now.
            fx.extend([
                self.status(),
                Effect::BankLights,
                Effect::Note(self.perform_line()),
            ]);
        }
        fx
    }

    /// SHIFT + SAMPLER on the Looper tab: the Perform set on the pads at
    /// the page shown last, or off.
    fn toggle_perform(&mut self) -> Vec<Effect> {
        self.set_loop_keys(!self.loop_keys)
    }

    /// The Perform set off the pads (Play's "Pads: Perform B" chip
    /// tapped): the pads play again.
    pub fn perform_off(&mut self) -> Vec<Effect> {
        self.set_loop_keys(false)
    }

    /// What the pads are with the set on, for the log: page A's loop keys
    /// as the Looper has always said, another page by its name.
    fn perform_line(&self) -> String {
        if self.perform_bank == 0 {
            "SHIFT + SAMPLER: the pads are the loop keys (top two rows: \
             each loop's key; bottom two: its mute)"
                .to_string()
        } else {
            format!(
                "SHIFT + SAMPLER: the pads are the Perform set, {} (PAD BANK: its pages)",
                self.perform_set.page_name(self.perform_bank)
            )
        }
    }

    /// PAD BANK with the Perform set on the pads: its page (SHIFT or a
    /// second press for E to H, as a kit's).
    fn perform_bank_button(&mut self, k: u8, shift: bool) -> Vec<Effect> {
        self.perform_bank = bank_target(self.perform_bank, k, shift);
        vec![
            self.status(),
            Effect::BankLights,
            Effect::Note(format!(
                "Perform: {}",
                self.perform_set.page_name(self.perform_bank)
            )),
        ]
    }

    fn set_loop_keys(&mut self, on: bool) -> Vec<Effect> {
        if on == self.loop_keys {
            return Vec::new();
        }
        let mut fx = Vec::new();
        if on {
            fx.extend(self.steps_off());
            self.mute_hold = None;
            if self.mute_keys != MuteKeys::Off {
                fx.extend(self.set_mute_keys(MuteKeys::Off));
            }
            if self.levels_on {
                // 16 Levels give the pads up too (the screen says back).
                fx.push(Effect::SixteenLevels);
            }
            fx.retain(|e| !matches!(e, Effect::Status(_) | Effect::Note(_)));
        }
        if !on {
            fx.extend(self.throws_off());
        }
        self.loop_keys = on;
        fx.extend([
            self.status(),
            Effect::BankLights,
            Effect::Note(if on {
                self.perform_line()
            } else {
                "SHIFT + SAMPLER: the pads play again".to_string()
            }),
        ]);
        fx
    }

    /// Another pad mode took the pads (STEP SEQ, TRACK MUTE, 16 Levels):
    /// the loop keys go off.
    fn loop_keys_off(&mut self) -> Vec<Effect> {
        if !self.loop_keys {
            return Vec::new();
        }
        self.loop_keys = false;
        let mut fx = self.throws_off();
        fx.push(self.status());
        fx
    }

    /// A pad tapped as a mute key: what it does. In Make a mute acts at
    /// once and in Perform it waits for the launch line; SHIFT gives the
    /// other timing. With TRACK MUTE held the tap solos its track: the
    /// pads show the tracks while it is held, and letting go leaves the
    /// mode as it was before the press (the effects say so). Over the
    /// kit's pad mutes (SHIFT + TRACK MUTE) the hold solos a pad of the
    /// kit instead, and the pads stay the kit's.
    pub fn mute_tap(&mut self) -> (MuteTap, Vec<Effect>) {
        let mut fx = Vec::new();
        let solo = self.mute_hold.is_some();
        if let Some(h) = &mut self.mute_hold {
            h.used = true;
            let keys = match h.before {
                MuteKeys::Pads => MuteKeys::Pads,
                _ => MuteKeys::Tracks,
            };
            if self.mute_keys != keys {
                fx.extend(self.set_mute_keys(keys));
            }
        }
        let tap = MuteTap {
            keys: self.mute_keys,
            bank: self.mute_bank,
            wait: self.perform != self.shift,
            solo,
        };
        (tap, fx)
    }

    /// TRACK MUTE let go: after a solo, the mode it had before the press.
    fn track_mute_up(&mut self) -> Vec<Effect> {
        match self.mute_hold.take() {
            Some(h) if h.used && h.before != self.mute_keys => self.set_mute_keys(h.before),
            _ => Vec::new(),
        }
    }

    /// TRACK MUTE's mode set to `keys`: its light, the status and the
    /// PAD BANK lights follow.
    fn set_mute_keys(&mut self, keys: MuteKeys) -> Vec<Effect> {
        self.mute_keys = keys;
        let mut fx = Vec::new();
        if let Some(n) = self.map.track_mute {
            let on = keys != MuteKeys::Off;
            fx.push(Effect::Light(n, if on { LIGHT_ON } else { LIGHT_OFF }));
        }
        fx.extend([self.status(), Effect::BankLights]);
        fx
    }

    /// NOTE REPEAT is held or Latch is on.
    pub fn repeating(&self) -> bool {
        self.repeat_held || self.latched
    }

    /// Whether the arp keeps its own swing, not the transport's (the
    /// pads' arp takes it, main.rs `PadsArp`).
    pub fn arp_own_swing(&self) -> bool {
        !self.swing_from_transport
    }

    /// The engine makes notes of its own (Repeat, Arp, Pattern or Phrase),
    /// so a hit may sound later than the press, and latched pads stay lit.
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
        for e in self.arps.engines_mut() {
            e.set_tempo(self.settings.bpm);
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

    /// FULL LEVEL (or SHIFT + FULL LEVEL) and the header's level chip: the
    /// level the pads play at, its light, and a line saying it.
    fn set_pad_level(&mut self, level: PadLevel) -> Vec<Effect> {
        self.pad_level = level;
        let on = level != PadLevel::Off;
        vec![
            Effect::Light(button::FULL_LEVEL, if on { LIGHT_ON } else { LIGHT_OFF }),
            self.status(),
            Effect::Note(match level {
                PadLevel::Off => "pad level: as played".to_string(),
                PadLevel::Full => "pad level: Full Level (every hit 127)".to_string(),
                PadLevel::Half => format!(
                    "pad level: Half Level (every hit {})",
                    self.settings.half_level
                ),
            }),
        ]
    }

    /// TC (the MPC's Timing Correct): the note editor on the armed track's
    /// clip, on the Sequencer tab, with its Edit options' drawer open on
    /// Timing Correct's settings; pressed again with the drawer showing, it
    /// corrects (the picked notes, or all), one take that UNDO takes back
    /// (the screen knows the drawer: `Effect::EditTc`). SHIFT + TC (the
    /// MPC's TC On/Off): the armed track's Quantise while recording, off
    /// or back on.
    fn tc_button(&mut self, shift: bool) -> Vec<Effect> {
        use mpfree_time_ui::editor::EditAction;
        if shift {
            return vec![Effect::RecordQuantiseToggle];
        }
        if self.view == View::Sequencer && self.notes_open {
            return vec![Effect::EditTc];
        }
        let mut fx = self.go(View::Sequencer);
        if !self.notes_open {
            fx.push(Effect::Time(TimeAction::Edit(EditAction::Open)));
        }
        fx.push(Effect::EditTc);
        fx.push(Effect::Note(
            "TC: the note editor, Timing Correct's settings first   ·   TC again: correct the notes"
                .to_string(),
        ));
        fx
    }

    /// MAIN's page: Play (one view since the looper redesign, so on Play
    /// it stays); in Perform, Arrange's Clips, and on Arrange the next view
    /// (Clips, Song, the song list).
    fn main_page(&mut self) -> Vec<Effect> {
        if self.perform {
            if self.view == View::Arrange {
                vec![Effect::ArrangeView(None)]
            } else {
                let mut fx = self.show(View::Arrange);
                fx.push(Effect::ArrangeView(Some(false)));
                fx
            }
        } else if self.view == View::Play {
            vec![Effect::PlayView(None)]
        } else {
            self.show(View::Play)
        }
    }

    /// MAIN let go: with no pad hit while it was held, MAIN's page, as a
    /// press always gave; after a pad picked a track, nothing more.
    fn main_up(&mut self) -> Vec<Effect> {
        let Some(h) = self.main_hold.take() else {
            return Vec::new();
        };
        let mut fx = vec![self.status(), Effect::BankLights];
        if !h.used {
            fx.extend(self.main_page());
        }
        fx
    }

    /// MAIN is held: the pads pick a track, of the 16 in this bank (A = 0:
    /// tracks 1-16).
    pub fn main_held(&self) -> Option<usize> {
        self.main_hold.map(|h| h.bank)
    }

    /// Pad `pad` (0 to 15) hit while MAIN is held: that track of the bank
    /// is picked (selected, and armed unless the arm is pinned), as a tap
    /// on its name on Arrange; past the last
    /// track, the hint line says so. Nothing when MAIN isn't held.
    pub fn main_pick(&mut self, pad: usize) -> Vec<Effect> {
        let tracks = self.track_count;
        let Some(h) = self.main_hold.as_mut() else {
            return Vec::new();
        };
        h.used = true;
        let t = h.bank * 16 + pad;
        if t >= tracks {
            return vec![Effect::Tell(format!(
                "There is no track {} yet: hit a lit pad, or add a track on Arrange",
                t + 1
            ))];
        }
        vec![Effect::Time(TimeAction::Pick(t))]
    }

    /// FULL LEVEL let go: with no pad hit while it was held, Full Level
    /// (with SHIFT as it went down, Half Level) on or off, as a press
    /// always did; after a hit, the level it set stays.
    fn full_level_up(&mut self) -> Vec<Effect> {
        let Some(h) = self.full_hold.take() else {
            return Vec::new();
        };
        if h.used {
            return vec![self.status()];
        }
        let want = if h.shift {
            PadLevel::Half
        } else {
            PadLevel::Full
        };
        let level = if self.pad_level == want {
            PadLevel::Off
        } else {
            want
        };
        self.set_pad_level(level)
    }

    /// A pad hit at `velocity` while FULL LEVEL is held (the MPC's own
    /// level): every hit plays at that hit's velocity from now on, as Half
    /// Level, so SHIFT + FULL LEVEL turns it off and on again. Kept in
    /// Prefs. Nothing when FULL LEVEL isn't held. Call it before the hit
    /// plays, so the hit plays at its own level.
    pub fn full_level_hit(&mut self, velocity: u8) -> Vec<Effect> {
        let Some(h) = self.full_hold.as_mut() else {
            return Vec::new();
        };
        h.used = true;
        let v = self.settings.curve.apply(velocity.max(1)).clamp(1, 127);
        self.settings.half_level = v;
        self.save_dirty = true;
        let mut fx = self.set_pad_level(PadLevel::Half);
        fx.push(Effect::Tell(format!(
            "Every hit plays at {v} now   ·   SHIFT + FULL LEVEL turns it off and on"
        )));
        fx
    }

    /// The pads play the kit: no synth, steps, mutes, 16 Levels or Perform
    /// set on them. COPY + a pad copies kit pads only then.
    pub fn pads_play_kit(&self) -> bool {
        !(self.pad_layout.is_some()
            || self.pads_chose_synth()
            || self.step_pads().is_some()
            || self.levels_on
            || self.pads_mute()
            || self.pads_are_perform())
    }

    /// Pad `slot` (0 to 15) hit while COPY is held over the kit's pads
    /// (the MPC's COPY): the first hit picks the pad to copy, each one
    /// after it takes a copy of it; with SHIFT as COPY went down, each
    /// pad hit is emptied (the MPC's Delete). Each is one undo step. None
    /// when the hit isn't COPY's (the pad plays as ever).
    pub fn copy_pad(&mut self, slot: usize) -> Option<Vec<Effect>> {
        if !self.copy_held() || !self.pads_play_kit() || slot >= 16 {
            return None;
        }
        let pad = u8::try_from(self.kit_pad(slot)).ok()?;
        let name = kit_pad_name(usize::from(pad));
        if self.copy_clears {
            return Some(vec![Effect::PadClear { pad }]);
        }
        Some(match self.copy_from {
            None => {
                self.copy_from = Some(pad);
                vec![Effect::Tell(format!(
                    "COPY: pad {name} picked   ·   hit the pads it goes to"
                ))]
            }
            Some(from) if from == pad => vec![Effect::Tell(format!(
                "COPY: pad {name} is the one being copied   ·   hit another pad"
            ))],
            Some(from) => vec![Effect::PadCopy { from, to: pad }],
        })
    }

    /// Pad `slot` (0 to 15) hit with SHIFT held over the kit's pads: it is
    /// picked, as a hit picks it (the Sample and Pads tabs, STEP SEQ's next
    /// sound, the Q-Links), and plays nothing, so a pad is picked in the
    /// middle of a song without being heard. None when the hit isn't
    /// SHIFT's (the pad plays as ever).
    pub fn pick_pad_quietly(&mut self, slot: usize) -> Option<Vec<Effect>> {
        if !self.shift || self.erase_held || !self.pads_play_kit() || slot >= 16 {
            return None;
        }
        self.step_pad_played(slot);
        let name = kit_pad_name(self.kit_pad(slot));
        Some(vec![
            Effect::PadPick { slot: slot as u8 },
            Effect::Tell(format!(
                "SHIFT + pad: pad {name} picked, not played   ·   hit it to hear it"
            )),
        ])
    }

    /// TAP TEMPO held and the wheel turned `delta` detents: the tempo by 1
    /// BPM a detent, 0.1 with SHIFT, as the tempo chip's number pad would
    /// set it.
    fn tempo_step(&mut self, delta: i32) -> Vec<Effect> {
        if let Some(h) = self.tap_hold.as_mut() {
            h.used = true;
        }
        let step = if self.shift { 0.1 } else { 1.0 };
        let bpm = ((self.settings.bpm + f64::from(delta) * step) * 10.0).round() / 10.0;
        let bpm = bpm.clamp(30.0, 300.0);
        vec![
            Effect::Time(TimeAction::Transport(Command::SetTempo(bpm))),
            Effect::Tell(format!(
                "Tempo {bpm:.1} BPM   ·   TAP TEMPO held: the wheel, 1 BPM a detent (SHIFT: 0.1)"
            )),
        ]
    }

    /// TAP TEMPO let go at `t`: held alone for a while, the tempo's number
    /// pad; after a wheel turn, or a tap, nothing more.
    fn tap_up(&mut self, t: f64) -> Vec<Effect> {
        let Some(h) = self.tap_hold.take() else {
            return Vec::new();
        };
        let mut fx = vec![self.status()];
        if !h.used && t - h.at >= TAP_HOLD_SECONDS {
            fx.push(Effect::TypeTempo);
        }
        fx
    }

    /// A tap on the header's Full level or Half level chip: the pads play
    /// as hit again (Jay's card, 13 Nov 14:57Z).
    pub fn pad_level_off(&mut self) -> Vec<Effect> {
        self.set_pad_level(PadLevel::Off)
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
            mute_bank: self.mute_bank,
            // The Looper's Pads switch lights for page A only: another
            // page isn't the loop keys.
            loop_keys: self.loop_keys && self.perform_bank == 0,
            perform: (self.loop_keys && self.perform_bank != 0)
                .then(|| self.perform_set.page_name(self.perform_bank)),
            perform_bank: self.perform_bank,
            steps: self.step_pads(),
            erase: self.erase_held,
            copy: self.copy_held(),
            half_level: self.settings.half_level,
            main_held: self.main_hold.map(|h| h.bank),
            full_held: self.full_hold.is_some(),
            tap_held: self.tap_hold.is_some(),
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
            if Some(note) == self.map.undo {
                fx.push(Effect::UndoButton(false));
            }
            if Some(note) == self.map.erase && self.erase_held {
                self.erase_held = false;
                fx.push(self.status());
            }
            if Some(note) == self.map.note_repeat && self.repeat_held {
                self.repeat_held = false;
                fx.extend(self.arp_changed());
            }
            if note == button::PLAY && self.fill == Fill::Held {
                self.fill = Fill::Off;
                fx.push(Effect::Fill(false));
            }
            if Some(note) == self.map.step_seq {
                fx.extend(self.step_seq_up());
            }
            if Some(note) == self.map.track_mute {
                fx.extend(self.track_mute_up());
            }
            if note == button::COPY && self.copy_held() {
                fx.extend(self.copy_button(false));
            }
            if note == button::FULL_LEVEL {
                fx.extend(self.full_level_up());
            }
            if Some(note) == self.map.main {
                fx.extend(self.main_up());
            }
            if Some(note) == self.map.tap_tempo {
                fx.extend(self.tap_up(t));
            }
            if let Some(k) = self.map.bank_of(note) {
                self.banks_down &= !(1 << k);
            }
            return fx;
        }
        if self.view == View::Guide {
            return self.guide_press(note);
        }
        if note == button::COPY {
            // COPY: STEP SEQ's steps and pages (step_pads.rs).
            return self.copy_button(true);
        }
        if let Some(k) = self.map.bank_of(note) {
            // All four PAD BANK buttons held (the MPC's way): a picture of
            // the screen. The first three did their own jobs as they went
            // down; the fourth does only this.
            self.banks_down |= 1 << k;
            if self.banks_down == 0b1111 {
                return vec![Effect::ScreenPicture];
            }
        }
        let shift = self.shift;
        if self.recall_open {
            // Sample Recall over the page: PLAY auditions, REC keeps, STOP
            // closes it, and the music plays on (the sequencer thread's OK,
            // 18 Nov). SHIFT + REC goes on as ever: Capture.
            let key = if note == button::PLAY {
                Some(RecallKey::Audition)
            } else if Some(note) == self.map.rec && !shift {
                Some(RecallKey::Keep)
            } else if note == button::STOP {
                Some(RecallKey::Close)
            } else {
                None
            };
            if let Some(k) = key {
                fx.push(Effect::Recall(k));
                return fx;
            }
        }
        if note == button::FULL_LEVEL {
            // Held, a pad hit sets the level every hit plays at (the MPC's
            // own level, [`App::full_level_hit`]); Full Level (SHIFT: Half
            // Level) on or off waits for the let-go ([`App::full_level_up`]).
            self.full_hold = Some(FullHold { shift, used: false });
            fx.push(self.status());
        } else if note == button::TC {
            fx.extend(self.tc_button(shift));
        } else if note == button::STOP {
            fx.extend(self.fill_off());
            if self.rec.ready.is_some() || self.rec.pad.is_some() {
                fx.push(Effect::Rec(crate::rec_arm::Press::Stop));
            }
            if self.rec.pad.is_some_and(|p| p.recording) {
                // STOP keeps a pad take; the music plays on (sampler
                // R5.7, as Sample Recall).
                return fx;
            }
            if self.timed {
                fx.push(Effect::Quit);
            } else if shift {
                // The MPC's SHIFT + STOP: stop, silence everything and go
                // back to bar 1 (Jay, 20:40Z), the free loops too (his
                // looper review of 23 Nov, 5.4). The Console has a chord
                // of its own: SHIFT + ERASE + TAP TEMPO.
                fx.extend(self.stop());
                fx.push(Effect::Looper(LoopAction::StopFree));
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
        } else if note == button::PLAY && shift && !self.browsing() && self.rec.ready.is_none() {
            // SHIFT + PLAY: from where PLAY last started, again (the MPC's
            // Play from the last start point); while playing, a jump back.
            fx.extend(self.fill_off());
            self.arps.engine_at_mut(self.track).resume();
            fx.push(Effect::PlayBack);
        } else if note == button::PLAY && self.browsing() {
            // On the Pads tab's browser PLAY hears the sound picked (as in
            // the browser); PLAY START still starts the transport.
            fx.push(Effect::Browse(BrowseKey::Play));
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
        } else if note == button::PLAY && self.rec.ready.is_some() {
            // The notes are ready: PLAY starts the take (after the
            // count-in).
            fx.extend(self.fill_off());
            self.arps.engine_at_mut(self.track).resume();
            fx.push(Effect::Rec(crate::rec_arm::Press::Play {
                from_start: false,
            }));
        } else if note == button::PLAY {
            fx.extend(self.fill_off());
            self.arps.engine_at_mut(self.track).resume();
            fx.push(Effect::Time(TimeAction::Transport(Command::Play)));
        } else if Some(note) == self.map.play_start && shift {
            self.settings.metronome = !self.settings.metronome;
            self.save_dirty = true;
            fx.push(self.metronome());
            fx.push(Effect::Tell(format!(
                "SHIFT + PLAY START: metronome {}",
                if self.settings.metronome { "on" } else { "off" }
            )));
            if self.in_menu() {
                fx.push(self.menu_view());
            }
        } else if Some(note) == self.map.play_start && self.rec.ready.is_some() {
            self.arps.engine_at_mut(self.track).resume();
            fx.push(Effect::Rec(crate::rec_arm::Press::Play {
                from_start: true,
            }));
        } else if Some(note) == self.map.play_start {
            self.arps.engine_at_mut(self.track).resume();
            fx.push(Effect::Time(TimeAction::Transport(Command::PlayStart)));
        } else if Some(note) == self.map.browse && shift {
            // SHIFT + BROWSE: save the project, from any page.
            fx.push(Effect::SaveProject);
        } else if Some(note) == self.map.browse {
            fx.extend(self.toggle(View::Browser));
        } else if Some(note) == self.map.track_mix {
            fx.extend(self.toggle_mixer(shift));
        } else if let Some(e) = self.mode_button(note, shift) {
            fx.extend(e);
        } else if let (true, true, Some(pad)) =
            (shift, Some(note) == self.map.rec, self.recall_pad())
        {
            // A pad held + SHIFT + REC: Sample Recall onto it, on every
            // page (sampler decision 3, 16 Nov 2026).
            fx.push(Effect::Recall(RecallKey::Open { pad }));
        } else if shift && Some(note) == self.map.rec && self.view == View::Looper {
            // SHIFT + REC on the Looper tab: the last bars heard become a
            // loop (the looper's Capture).
            fx.push(Effect::Looper(LoopAction::Capture));
        } else if shift && Some(note) == self.map.rec {
            // SHIFT + REC: Capture, on every page (Jay, 20:40Z).
            fx.push(Effect::Time(TimeAction::Capture));
        } else if Some(note) == self.map.rec {
            // REC records what is armed, on every page (sequencer
            // decision 1); with an empty kit pad held, one take onto it
            // (sampler R5.7). With the synth on the pads by Play on the
            // pads, the take goes where the synth plays (desk.rs).
            let held = self.held_kit_pads();
            fx.push(if held.is_empty() {
                Effect::Rec(crate::rec_arm::Press::Rec)
            } else {
                Effect::PadRec(held)
            });
        } else if shift && Some(note) == self.map.overdub {
            // SHIFT + OVERDUB: the count-in off or on (the MPC's Count In).
            fx.push(Effect::CountInToggle);
        } else if Some(note) == self.map.overdub {
            fx.push(Effect::Rec(crate::rec_arm::Press::Overdub));
        } else if !shift && Some(note) == self.map.undo && self.rec.pad.is_some_and(|p| p.recording)
        {
            // UNDO while a pad take records throws it away, on every page.
            fx.push(Effect::Undo { redo: false });
            fx.push(Effect::UndoButton(true));
        } else if self.view == View::Synth && self.map_panel && Some(note) == self.map.undo {
            // What the pads play (sampler step 15e): the last change to
            // the pad map; it has no redo.
            fx.extend(if shift {
                vec![Effect::Tell(
                    "What the pads play has no redo: pick it again".to_string(),
                )]
            } else {
                self.pad_map_undo()
            });
            fx.push(Effect::UndoButton(true));
        } else if self.view == View::Looper && Some(note) == self.map.undo {
            fx.extend(self.looper_tap(if shift {
                crate::looper_page::B_REDO
            } else {
                crate::looper_page::B_UNDO
            }));
            fx.push(Effect::UndoButton(true));
        } else if Some(note) == self.map.undo && self.view.sampler() {
            fx.push(Effect::SampleUndo { redo: shift });
            fx.push(Effect::UndoButton(true));
        } else if Some(note) == self.map.undo {
            // The Play page's view, Arrange and the rest: the screen picks.
            fx.push(Effect::Undo { redo: shift });
            fx.push(Effect::UndoButton(true));
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
            } else if !shift && self.view == View::Prefs {
                // MENU in Prefs closes it, as its ‹ Back does.
                fx.extend(self.toggle(View::Prefs));
            } else if !shift && self.on_tunings_page() {
                // MENU on the Tunings page: back to the options it opened
                // from.
                fx.extend(self.leave_tunings_page());
            } else if !shift && self.view == View::Options && self.mod_section {
                // And in the arp options' Modifier section.
                fx.extend(self.leave_mod_section());
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
            // A tap, and held, the wheel turns the tempo; held alone, the
            // number pad as it is let go ([`App::tap_up`]).
            self.tap_hold = Some(TapHold { at: t, used: false });
            fx.push(self.status());
            fx.extend(self.tap(t));
        } else if let Some(k) = self.map.bank_of(note).filter(|_| self.main_hold.is_some()) {
            // MAIN held: PAD BANK picks which 16 tracks the pads are (A:
            // 1-16 ... H: 113-128; SHIFT or a second press for E-H).
            if let Some(h) = self.main_hold.as_mut() {
                h.bank = bank_target(h.bank, k, shift);
                h.used = true;
            }
            fx.push(self.status());
            fx.push(Effect::BankLights);
        } else if let Some(k) = self.map.bank_of(note).filter(|_| self.pads_are_steps()) {
            // STEP SEQ: PAD BANK picks the page of steps on the pads.
            fx.extend(self.step_page_button(k, shift));
        } else if let Some(k) = self
            .map
            .bank_of(note)
            .filter(|_| self.mute_keys == MuteKeys::Tracks)
        {
            // TRACK MUTE: PAD BANK picks which 16 tracks the pads mute (as
            // it picks kits: SHIFT or a second press for E-H); the kit
            // playing stays.
            self.mute_bank = bank_target(self.mute_bank, k, shift);
            fx.push(Effect::Note(self.mute_bank_line()));
            fx.push(Effect::BankLights);
        } else if let Some(k) = self.map.bank_of(note).filter(|_| self.pads_are_perform()) {
            // The Perform set on the pads, or on the Pads tab: PAD BANK
            // picks its page.
            fx.extend(self.perform_bank_button(k, shift));
        } else if let Some(k) = self.map.bank_of(note) {
            fx.push(Effect::Bank { button: k, shift });
        } else if Some(note) == self.map.data_push && self.wheel_to_held() {
            // A finger on a control: the push does nothing, so a thumb that
            // slips presses nothing; with SHIFT, the control goes back to
            // its default.
            if shift {
                fx.push(Effect::Reset);
            }
        } else if (Some(note) == self.map.minus || Some(note) == self.map.plus)
            && self.wheel_to_held()
        {
            // A finger on a control: - / + step it, one detent each (SHIFT:
            // the finer steps SHIFT gives the wheel).
            let delta = if Some(note) == self.map.plus { 1 } else { -1 };
            fx.push(Effect::Held { delta, fine: shift });
        } else if Some(note) == self.map.data_push && shift && self.resets_outlined() {
            // SHIFT + the wheel's push: the outlined value back to its
            // default (the button combos of 19 Nov).
            fx.push(Effect::Reset);
        } else if Some(note) == self.map.data_push
            && self.sheet.is_some()
            && matches!(self.view, View::Arrange | View::Browser)
        {
            // The Make a phrase sheet: the push does nothing (Keep is a
            // tap), so it never loads or picks what is under the sheet.
        } else if Some(note) == self.map.data_push && self.browsing() {
            fx.push(Effect::Browse(BrowseKey::Push));
        } else if Some(note) == self.map.data_push && self.view == View::SystemStatus {
            // System status: the push opens the picked system's first card.
            fx.push(Effect::StatusPage(StatusKey::Open));
        } else if Some(note) == self.map.data_push {
            fx.extend(self.wheel_push());
        } else if Some(note) == self.map.minus || Some(note) == self.map.plus {
            let d = if Some(note) == self.map.plus { 1 } else { -1 };
            if self.overlay {
                // A list or pad over the page: - / + step in it.
                fx.push(Effect::Rows(RowKey::Step(d)));
            } else if self.in_menu() {
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
                    | View::StatusBoard
                    | View::Console
            ) {
                fx.extend(self.debug_step(d));
            } else if self.repeat_held || self.view == View::Arp {
                fx.extend(self.arp_mode_step(d));
            } else if self.view == View::Synth && shift {
                fx.push(Effect::SynthPage(d));
            } else if self.sheet.is_some() && matches!(self.view, View::Arrange | View::Browser) {
                // The Make a phrase sheet: the wheel's row.
                fx.extend(self.sheet_action(mpfree_arp_ui::make_sheet::SheetAction::Change(d)));
            } else if self.browsing() {
                fx.push(Effect::Browse(if d > 0 {
                    BrowseKey::Enter
                } else {
                    BrowseKey::Back
                }));
            } else if self.view == View::SystemStatus {
                fx.push(Effect::StatusPage(StatusKey::Step(d)));
            } else if self.view == View::Arrange && self.show_view && !shift {
                // The Show view (live shows B1): the song after or before
                // is armed, as the wheel arms it.
                fx.push(Effect::ShowKey(ShowKey::Step(d)));
            } else if self.view.sampler() && shift {
                // SHIFT + - / +: zoom the waveform (Wave, Warp, Chop).
                fx.push(Effect::SampleWheel(d));
            } else if self.has_rows() {
                // - / + step the highlighted row, as in the menus; on the
                // Notes view the screen knows whether the drawer is open.
                fx.push(Effect::Rows(
                    if self.view == View::Sequencer && self.notes_open {
                        RowKey::Arrow { delta: d, shift }
                    } else {
                        RowKey::Step(d)
                    },
                ));
            } else if self.view == View::Mixer && !self.outline {
                // The I/O view's row; on the strips the outline.
                fx.push(Effect::MixerStep(d));
            } else if outlined(self.view) {
                // The outlined value, or the outline (no longer the kit);
                // with SHIFT, the value in finer steps.
                fx.push(Effect::Nav(if shift { Nav::Fine(d) } else { Nav::Step(d) }));
            }
        }
        fx
    }

    /// The mode buttons and ERASE, as the hardware control map has them
    /// (Jay, 20:40Z): each goes to the page with the same job. None when
    /// `note` is not one of them.
    fn mode_button(&mut self, note: u8, shift: bool) -> Option<Vec<Effect>> {
        let m = self.map;
        let is = |b: Option<u8>| b == Some(note);
        let fx = if is(m.erase) {
            // Held, ERASE makes the next tap on the screen delete what it
            // lands on (Jay after live test 3); the screen knows from the
            // status. In the browser, ERASE moves the item picked to the
            // Trash (asking first). Live erase (ERASE + a pad while
            // recording) and the FX lock mode come later. Held with SHIFT,
            // TAP TEMPO opens the Console.
            self.erase_held = true;
            if self.view == View::Browser && !shift {
                return Some(vec![Effect::Browse(BrowseKey::Erase)]);
            }
            return Some(vec![self.status()]);
        } else if is(m.main) && shift {
            // SHIFT + MAIN: Make or Perform (sequencer S4), as the top
            // bar's chip.
            vec![Effect::Time(TimeAction::Perform(!self.perform))]
        } else if is(m.main) {
            // MAIN held, as the MPC's: the pads are tracks, a pad hit
            // picks one ([`App::main_pick`]) and PAD BANK which 16; MAIN's
            // page waits for the let-go ([`App::main_up`]), so a quick
            // press goes there as ever.
            self.main_hold = Some(MainHold {
                bank: self.track / 16,
                used: false,
            });
            vec![self.status(), Effect::BankLights]
        } else if is(m.step_seq) {
            // STEP SEQ: the pads as steps, on the release (held, a pad
            // tapped picks the sound); SHIFT + STEP SEQ: Follow.
            self.step_seq_down(shift)
        } else if is(m.next_seq) && shift {
            // SHIFT + NEXT SEQ: the XY pad full screen on Play, as the
            // MPC's XYFX (the button combos of 19 Nov), or back from it.
            let mut fx = self.go(View::Play);
            fx.push(Effect::XyFull);
            fx
        } else if is(m.next_seq) {
            self.go(View::Arrange)
        } else if is(m.sampler) {
            // SHIFT + SAMPLER: the Looper tab; pressed again there, the
            // pads become the loop keys, or play again.
            if shift && self.view == View::Looper {
                self.toggle_perform()
            } else {
                self.go(if shift { View::Looper } else { View::Sample })
            }
        } else if is(m.sample_edit) && shift {
            // SHIFT + SAMPLE EDIT: the macros panel (the MPC's Q-Link
            // Edit), or back from it.
            vec![Effect::MacrosPanel]
        } else if is(m.sample_edit) {
            let mut fx = vec![Effect::SampleEditor(crate::sample_page::Editor::Wave)];
            fx.extend(self.go(View::Sample));
            fx
        } else if is(m.prog_edit) && shift {
            // SHIFT + PROG EDIT: controller mode's settings, the I/O view's
            // Controller section (the MPC's MIDI Control).
            vec![Effect::ControllerSettings]
        } else if is(m.prog_edit) && self.view == View::Pads {
            // PROG EDIT on the Pads tab: Edit pad on the pad picked, or
            // back to the pads from it (the one Sample page's step 6b), as
            // the MPC's PROG EDIT opens its Track Edit.
            vec![Effect::EditPad]
        } else if is(m.prog_edit) {
            vec![Effect::ProgEdit]
        } else if is(m.q_link) {
            // Q-LINK: the column's next bank (Macros 1-4, 5-8, the page's
            // own knobs), where the column has macro banks (layout spec 4b);
            // on the Looper tab, loops 1-4's levels, 5-8's, the picked
            // loop's knobs. SHIFT + Q-LINK: the bank before, as on the MPC.
            if matches!(
                self.view,
                View::Play
                    | View::Sequencer
                    | View::Arrange
                    | View::Chain
                    | View::Macros
                    | View::Looper
            ) {
                vec![if shift {
                    Effect::QLinkBankBack
                } else {
                    Effect::QLinkBank
                }]
            } else {
                Vec::new()
            }
        } else if is(m.track_mute) {
            // TRACK MUTE: the pads mute tracks; SHIFT + TRACK MUTE: the
            // kit's pads. The same press again leaves; the other one
            // switches. Held (without SHIFT), a pad solos its track, and
            // letting go leaves the mode as it was (`track_mute_up`).
            self.mute_hold = (!shift).then_some(MuteHold {
                before: self.mute_keys,
                used: false,
            });
            let want = if shift {
                MuteKeys::Pads
            } else {
                MuteKeys::Tracks
            };
            // One pad mode at a time: STEP SEQ and the loop keys give the
            // pads up.
            let mut off = self.steps_off();
            off.extend(self.loop_keys_off());
            self.mute_keys = if self.mute_keys == want {
                MuteKeys::Off
            } else {
                want
            };
            let on = self.mute_keys != MuteKeys::Off;
            off.retain(|e| !matches!(e, Effect::Status(_) | Effect::Note(_)));
            if on && self.levels_on {
                // 16 Levels give the pads up too (the screen says back).
                off.push(Effect::SixteenLevels);
            }
            off.extend([
                Effect::Light(note, if on { LIGHT_ON } else { LIGHT_OFF }),
                self.status(),
                Effect::Note(match self.mute_keys {
                    MuteKeys::Off => "TRACK MUTE: the pads play again".to_string(),
                    MuteKeys::Tracks => self.mute_bank_line(),
                    MuteKeys::Pads => {
                        "SHIFT + TRACK MUTE: each pad mutes itself on this track".to_string()
                    }
                }),
                // The PAD BANK lights show the mute bank, or the kit's again.
                Effect::BankLights,
            ]);
            off
        } else if is(m.sixteen_level) {
            // SHIFT + 16 LEVEL: the synth pads' Chromatic | In key switch.
            // 16 LEVEL: 16 Levels on (with its panel on Play) or off.
            if !shift {
                let mut fx = self.go(View::Play);
                fx.push(Effect::SixteenLevels);
                return Some(fx);
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

    /// The screen turned 16 Levels on or off (`on`), shows its panel or
    /// not (`open`) and picked Transpose or a kit's levels (`transpose`):
    /// 16 LEVEL lights while they are on, and the wheel turns the panel's
    /// kind while it shows.
    pub fn levels_shown(&mut self, on: bool, open: bool, transpose: bool) -> Vec<Effect> {
        self.levels_open = open;
        self.levels_transpose = on && transpose;
        let mut fx = Vec::new();
        if on && !self.levels_on {
            // One pad mode at a time: STEP SEQ, TRACK MUTE and the loop
            // keys give the pads up to 16 Levels.
            fx.extend(self.steps_off());
            fx.extend(self.loop_keys_off());
            self.mute_hold = None;
            if self.mute_keys != MuteKeys::Off {
                self.mute_keys = MuteKeys::Off;
                if let Some(n) = self.map.track_mute {
                    fx.push(Effect::Light(n, LIGHT_OFF));
                }
                fx.push(self.status());
                fx.push(Effect::BankLights);
            }
        }
        self.levels_on = on;
        if let Some(note) = self.map.sixteen_level {
            fx.push(Effect::Light(note, if on { LIGHT_ON } else { LIGHT_OFF }));
        }
        fx
    }

    /// The note editor opened or closed (the time engine says so after
    /// each of its actions: Arrange's Edit row, the Sequencer's Notes, ‹
    /// Steps). Opened, the Sequencer shows it (its Notes view); closed, a
    /// row the wheel was changing closes with it.
    pub fn set_notes_open(&mut self, open: bool) -> Vec<Effect> {
        let was = std::mem::replace(&mut self.notes_open, open);
        if open && !was && self.view != View::Sequencer && self.view != View::Guide {
            return self.show(View::Sequencer);
        }
        if was && !open && self.view == View::Sequencer && self.page_editing {
            self.page_editing = false;
            return vec![Effect::Rows(RowKey::Editing(false))];
        }
        Vec::new()
    }

    /// The note editor is open (the Sequencer's Notes view).
    #[cfg(test)]
    pub fn notes_open(&self) -> bool {
        self.notes_open
    }

    /// The Sequencer tab shows the note editor (its Notes view): the pads
    /// pick and write there (piano roll plan 1C.2).
    pub fn notes_shown(&self) -> bool {
        self.view == View::Sequencer && self.notes_open
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
        // A loop waiting for PLAY (an overdub, or an armed first pass)
        // lets go too, so its lights stop blinking.
        let waiting = Effect::Looper(LoopAction::StopWaiting);
        let mut fx = self.stop_sounds();
        fx.insert(0, waiting);
        fx
    }

    fn stop_sounds(&mut self) -> Vec<Effect> {
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

    /// A tap on Play's tempo (live test 3): a tap of the tempo, as TAP
    /// TEMPO, and the screen says what it did.
    pub fn tap_on_screen(&mut self, t: f64) -> Vec<Effect> {
        let mut fx = self.tap(t);
        let set = fx.iter().any(|e| matches!(e, Effect::Status(_)));
        fx.push(Effect::Tell(if set {
            format!("tempo: {:.1} BPM (tapped)", self.settings.bpm)
        } else {
            "tempo: keep tapping in time to set it".to_string()
        }));
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
        self.tempo_by_hand = true;
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

    /// [`App::set_tempo`], by hand (typed, turned, from a remote): a free
    /// first loop's reading counts it ([`App::tempo_by_hand`]).
    pub fn set_tempo_by_hand(&mut self, bpm: f64) -> Vec<Effect> {
        self.tempo_by_hand = true;
        self.set_tempo(bpm)
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
        self.tick_t = t;
        let note = self.map.tap_tempo?;
        let beats = (t - self.beat_zero) * self.settings.bpm / 60.0;
        let on = beats.rem_euclid(1.0) < 0.2;
        if on == self.tap_light {
            return None;
        }
        self.tap_light = on;
        Some(Effect::Light(note, if on { LIGHT_ON } else { LIGHT_OFF }))
    }

    /// The data wheel: rows in a menu or on a page with rows, the outline
    /// on the pages [`outlined`] names ([`Nav`]), each other page's own job.
    pub fn wheel(&mut self, delta: i32) -> Vec<Effect> {
        if self.tap_hold.is_some() {
            // TAP TEMPO held: the wheel is the tempo.
            return self.tempo_step(delta);
        }
        if self.wheel_to_held() {
            // A finger on a control: the wheel turns it (SHIFT: finer).
            return vec![Effect::Held {
                delta,
                fine: self.shift,
            }];
        }
        match self.view {
            View::Guide | View::System => Vec::new(),
            // A list or pad over the page: the wheel moves in it.
            _ if self.overlay => vec![Effect::Rows(RowKey::Pick(delta))],
            // In a menu the wheel moves between rows and - / + change the
            // value, so menus work without touch; SHIFT + wheel also changes
            // the value, in case - and + are not known yet.
            // Prefs: SHIFT + wheel goes to the next tab or the one before;
            // on Appearance the wheel tries the theme after or before.
            View::Prefs if self.shift => self.step_prefs_tab(delta),
            View::Prefs if self.prefs_tab == PrefsTab::Themes => {
                self.step_item(self.selected, delta.signum())
            }
            _ if self.in_menu() && (self.shift || self.menu_editing) => {
                self.step_item(self.selected, delta.signum())
            }
            _ if self.in_menu() => {
                let at = self.menu_row_from(self.selected, delta);
                self.select(at)
            }
            // The Show view (live shows B1): the wheel arms the song after
            // or before; SHIFT + wheel keeps scrolling Arrange's views.
            View::Arrange if self.show_view && !self.shift && !self.repeat_held => {
                vec![Effect::ShowKey(ShowKey::Step(delta.signum()))]
            }
            // The outline (Jay, 13 Nov): on Play and the Looper tab the
            // wheel always moves it (it no longer steps the kit: the PAD
            // BANK buttons do); on the mixer, the chain editor and the
            // macros panel once a tap or a push has put it on a control.
            // SHIFT + wheel and NOTE REPEAT keep their jobs.
            // (The 16 Levels panel keeps the wheel for its kinds.)
            View::Play if !self.shift && !self.repeat_held && !self.levels_open => {
                vec![Effect::Nav(Nav::Turn(delta))]
            }
            // The Sequencer's Notes view keeps Arrange's wheel (the Edit
            // options; SHIFT scrolls the notes), its Steps view Play's.
            View::Sequencer if self.notes_open && !self.repeat_held && self.shift => {
                vec![Effect::ClipsWheel {
                    delta,
                    scroll: true,
                }]
            }
            View::Sequencer if self.notes_open && !self.repeat_held => {
                vec![Effect::Rows(if self.page_editing {
                    RowKey::Step(delta)
                } else {
                    RowKey::Pick(delta)
                })]
            }
            View::Sequencer if !self.shift && !self.repeat_held => {
                vec![Effect::Nav(Nav::Turn(delta))]
            }
            View::Looper if !self.shift && !self.repeat_held => {
                vec![Effect::Nav(Nav::Turn(delta))]
            }
            v if outlined(v) && self.outline && !self.shift && !self.repeat_held => {
                vec![Effect::Nav(Nav::Turn(delta))]
            }
            View::Pattern => vec![Effect::WorkspaceWheel(delta)],
            // The mixer: the wheel is the selected strip's volume (Q-Link 1),
            // or on the I/O view the section's selection.
            View::Mixer => vec![Effect::MixerWheel(delta)],
            // The chain editor and the macros panel: the wheel is Q-Link 1.
            View::Chain => vec![Effect::ChainWheel(delta)],
            View::Macros => vec![Effect::ChainTurn { knob: 0, delta }],
            // On the Debug tab SHIFT + wheel picks the page; on Map controls
            // the wheel picks the control.
            View::Debug
            | View::Perf
            | View::Engines
            | View::Procs
            | View::Map
            | View::Touch
            | View::StatusBoard
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
            // The Make a phrase sheet, on Arrange or in the browser: its row.
            View::Arrange | View::Browser if self.sheet.is_some() && !self.shift => {
                self.sheet_action(mpfree_arp_ui::make_sheet::SheetAction::Wheel(delta))
            }
            // The 16 Levels panel: what the levels change.
            View::Play if self.levels_open => vec![Effect::LevelsWheel(delta)],
            // SHIFT + wheel keeps each page's second job: scroll Arrange,
            // the Synth tab's pages, the pads with a sound.
            View::Arrange if self.shift => vec![Effect::ClipsWheel {
                delta,
                scroll: true,
            }],
            View::Synth if self.shift => vec![Effect::SynthPage(delta.signum())],
            View::Browser if self.shift => vec![Effect::Browse(BrowseKey::Places(delta.signum()))],
            View::Browser => vec![Effect::Browse(BrowseKey::Wheel(delta))],
            // System status: the wheel steps the systems.
            View::SystemStatus => vec![Effect::StatusPage(StatusKey::Step(delta.signum()))],
            View::Sample | View::Pads if self.shift => vec![Effect::SamplePad(delta.signum())],
            // Pages with rows: the wheel picks a row, or, once the wheel's
            // push opened it, changes its value (RowKey).
            View::Arrange | View::Synth | View::Sample | View::Pads => {
                vec![Effect::Rows(if self.page_editing {
                    RowKey::Step(delta)
                } else {
                    RowKey::Pick(delta)
                })]
            }
            View::Play | View::Sequencer if self.shift => vec![Effect::Octave(delta.signum())],
            _ => Vec::new(),
        }
    }

    // ------------------------------------------------------------ pages

    fn in_menu(&self) -> bool {
        matches!(self.view, View::Options | View::Prefs)
    }

    /// Show `v`. The page left is remembered for going back, unless it is
    /// a menu, the guide or the workspace (whose Back goes to the panel).
    fn show(&mut self, v: View) -> Vec<Effect> {
        // Leaving Prefs while "Keep this theme?" asks: the old theme.
        let mut back = Vec::new();
        if self.view == View::Prefs && v != View::Prefs && self.theme_trial.is_some() {
            back = self.end_theme_trial(false);
        }
        // Customise's copy shows only while Customise is open.
        let opening = v == View::Prefs && self.view != View::Prefs;
        if self.view == View::Prefs && v != View::Prefs {
            back.extend(self.custom_leave());
        }
        let mut fx = self.show_inner(v);
        if opening && self.view == View::Prefs && self.prefs_tab == PrefsTab::Customise {
            back.extend(self.custom_open(false));
        }
        if !back.is_empty() {
            back.append(&mut fx);
            fx = back;
        }
        fx
    }

    fn show_inner(&mut self, v: View) -> Vec<Effect> {
        self.menu_editing = false;
        // The screen closes a list or pad over the page when the page
        // changes, and Sample Recall with it.
        self.overlay = false;
        self.recall_open = false;
        // The list of synths closes with the Synth tab, and the pads keep
        // what they play (step 6, app/synth_pick.rs).
        if v != View::Synth {
            self.synth_pick = None;
            self.map_panel = false;
        }
        let was_editing = std::mem::take(&mut self.page_editing);
        if self.view != v
            && matches!(
                self.view,
                View::Play
                    | View::Pads
                    | View::Sequencer
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
                    | View::StatusBoard
            )
        {
            self.back = self.view;
        }
        if v.tab_view() == Some(View::Mixer) && self.view.tab_view() != Some(View::Mixer) {
            self.mix_back = match self.view {
                View::Guide
                | View::Options
                | View::Prefs
                | View::System
                | View::Macros
                | View::Browser
                | View::SystemStatus => self.back,
                w => w,
            };
        }
        let mut fx = Vec::new();
        if was_editing {
            // The row the wheel was changing closes with its page.
            fx.push(Effect::Rows(RowKey::Editing(false)));
        }
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
        self.tunings_page = false;
        if self.in_menu() {
            self.selected = 0;
            self.mod_section = false;
            fx.push(self.menu_view());
        }
        fx.push(Effect::Show(v));
        fx.extend(self.after_view_change());
        fx
    }

    /// TRACK MIX or SHIFT + TRACK MIX (PAD MIX): the Mix tab opens on
    /// Track Mix (or Pad Mix); pressed on the Mix tab (the mixer or an
    /// effect chain), back to the page before it (Jay's card, 14 Nov
    /// 07:15Z).
    pub fn toggle_mixer(&mut self, pads: bool) -> Vec<Effect> {
        use mpfree_mixer_ui::MixView;
        if self.view == View::Guide {
            return Vec::new();
        }
        if self.view.tab_view() == Some(View::Mixer) {
            let back = match self.mix_back {
                v if v.tab_view() == Some(View::Mixer) => View::Play,
                v => v,
            };
            return self.show(back);
        }
        let mut fx = self.show(View::Mixer);
        fx.insert(
            0,
            Effect::Mixer(Some(if pads { MixView::Pads } else { MixView::Tracks })),
        );
        fx.insert(1, Effect::MixerFaders);
        fx
    }

    /// The bar's folder icon (layout spec, section 2): the browser opens
    /// over the page, or closes back to it, as BROWSE does.
    pub fn toggle_browser(&mut self) -> Vec<Effect> {
        self.toggle(View::Browser)
    }

    /// The status chip: System status opens over the page, or closes back
    /// to it.
    pub fn toggle_status(&mut self) -> Vec<Effect> {
        self.toggle(View::SystemStatus)
    }

    /// A System status card's button: it does only what it says. Silence
    /// all notes is STOP's second press (the latch goes too). Show what's
    /// missing opens the browser, whose status line names what the project
    /// left out. Open the last project comes here only when the screen
    /// couldn't open it at once (the safe start's new project has changes):
    /// the browser opens, where opening a project asks about them first.
    pub fn status_action(&mut self, a: mpfree_status::Action) -> Vec<Effect> {
        match a {
            mpfree_status::Action::SilenceNotes => {
                self.latched = false;
                self.stop_sounds()
            }
            mpfree_status::Action::ShowMissing | mpfree_status::Action::OpenLastProject => {
                self.show(View::Browser)
            }
        }
    }

    /// The Pads tab's Browse for: the browser opens over the page (its
    /// Load puts a sound on the pad picked); BROWSE or the folder icon
    /// closes it back to the Pads tab.
    pub fn open_browser(&mut self) -> Vec<Effect> {
        if self.view == View::Browser {
            return Vec::new();
        }
        self.show(View::Browser)
    }

    /// The Synth tab's preset picker is done (Load or Cancel): back to the
    /// page the browser opened over. Nothing if the browser no longer
    /// shows (BROWSE closed it first).
    pub fn close_browser(&mut self) -> Vec<Effect> {
        if self.view != View::Browser {
            return Vec::new();
        }
        self.toggle(View::Browser)
    }

    /// The tempo chip: the mixer's I/O view, whatever shows (the app picks
    /// the Sync section). It never closes it.
    pub fn open_io(&mut self) -> Vec<Effect> {
        use mpfree_mixer_ui::MixView;
        if self.view == View::Guide {
            return Vec::new();
        }
        if self.view == View::Mixer {
            return vec![Effect::Mixer(Some(MixView::Io))];
        }
        let mut fx = self.show(View::Mixer);
        fx.insert(0, Effect::Mixer(Some(MixView::Io)));
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

    /// An on-screen button that does what MENU does: the Synth tab's gear
    /// opens its options, and the options' Back goes back to the page.
    pub fn options_button(&mut self) -> Vec<Effect> {
        match self.view {
            View::Options if self.tunings_page => self.leave_tunings_page(),
            View::Options if self.mod_section => self.leave_mod_section(),
            View::Prefs => self.toggle(View::Prefs),
            View::System => self.show(View::Prefs),
            // MENU closes System status, back to the page under it.
            View::SystemStatus => self.toggle(View::SystemStatus),
            View::Pattern => Vec::new(),
            _ => self.toggle(View::Options),
        }
    }

    fn toggle(&mut self, v: View) -> Vec<Effect> {
        if self.view == v {
            let back = if self.back == v {
                View::Play
            } else {
                self.back
            };
            // Closing Prefs while "Keep this theme?" asks: the old theme,
            // as leaving it any other way.
            let mut fx = Vec::new();
            if v == View::Prefs && self.theme_trial.is_some() {
                fx = self.end_theme_trial(false);
            }
            if v == View::Prefs {
                fx.extend(self.custom_leave());
            }
            self.menu_editing = false;
            self.view = back;
            fx.push(Effect::Show(back));
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
        // New idea or More like this lives on the Arp panel: another page
        // closes it, as Back.
        if self.view != View::Arp {
            fx.extend(self.close_maker());
        }
        // So does the Make a phrase sheet on another page than its own.
        fx.extend(self.sheet_follows_view());
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
    /// The wheel, its push, - / + and PLAY go to the browser.
    fn browsing(&self) -> bool {
        self.view == View::Browser
    }

    /// A page whose rows the wheel, - / + and the wheel's push work
    /// ([`RowKey`]): the Sample tab, the Pads tab, the Synth tab and
    /// Arrange. Not while NOTE REPEAT holds the wheel for the arp.
    fn has_rows(&self) -> bool {
        !self.repeat_held
            && (matches!(
                self.view,
                View::Sample | View::Synth | View::Arrange | View::Pads
            ) || (self.view == View::Sequencer && self.notes_open))
    }

    pub fn open_tab(&mut self, v: View) -> Vec<Effect> {
        if self.view == View::Guide {
            return Vec::new();
        }
        if v == View::Mixer {
            // The Mix tab: the mixer as it was left (an open chain's
            // editor goes back to it too).
            if self.view == View::Mixer {
                return Vec::new();
            }
            let mut fx = self.show(View::Mixer);
            fx.insert(0, Effect::MixerLast);
            return fx;
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
                    | View::StatusBoard
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

    /// The looper's size in this start (main, once the looper is made).
    pub fn set_loop_memory_now(&mut self, size: LoopMemory) {
        self.loop_memory_now = size;
    }

    /// The audio tracks' size in this start (main, once they are made).
    pub fn set_audio_memory_now(&mut self, size: crate::audiotracks::AudioMemory) {
        self.audio_memory_now = size;
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
            // The picked loop's own mute, turned over (each loop keeps
            // its own).
            B_MUTE => vec![Effect::Looper(LoopAction::ToggleMute(None))],
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

    /// What the XY pad plays, picked on its chip's list: kept with Prefs.
    pub fn set_xy_effect(&mut self, pick: Option<usize>) -> Vec<Effect> {
        let pick = pick.filter(|&k| k < mpfree_fx::xyfx::EFFECTS.len());
        if self.settings.xy_effect == pick {
            return Vec::new();
        }
        self.save_dirty = true;
        self.settings.xy_effect = pick;
        vec![Effect::Settings(self.settings.clone())]
    }

    /// The Looper tab's Length: the picked loop's first pass (None: free).
    /// The setting follows, so MENU's First pass length and the next start
    /// agree with the last choice.
    pub fn set_loop_bars(&mut self, bars: Option<u32>) -> Vec<Effect> {
        self.save_dirty = true;
        self.settings.loop_bars = bars;
        let mut fx = vec![
            Effect::Looper(LoopAction::Command(LoopCommand::SetBars(bars))),
            Effect::Settings(self.settings.clone()),
        ];
        if self.in_menu() {
            fx.push(self.menu_view());
        }
        fx
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

    /// A Loop cell armed on the grid: the looper records its Loop track's
    /// `input` (loop-tracks.md part 4). Line in switches the line input
    /// on as its Settings row does; Resample keeps the source (the desk
    /// makes REC's Record a Resample, which records the other loops).
    pub fn loop_input(&mut self, input: mpfree_seq::LoopInput) -> Vec<Effect> {
        use mpfree_seq::LoopInput;
        let mut fx = match input {
            LoopInput::Line if !self.settings.line_input => {
                return self.looper_setting(Item::LineInput, 1)
            }
            LoopInput::Line => self.set_loop_source(LoopSource::Line),
            LoopInput::Pads => self.set_loop_source(LoopSource::Pads),
            LoopInput::Resample => Vec::new(),
        };
        if !fx.is_empty() {
            self.save_dirty = true;
            fx.push(Effect::Settings(self.settings.clone()));
        }
        fx
    }

    /// An audio track's empty cell armed for a loop (audio tracks phase 4b,
    /// behind Dev's switch): the looper records the track's own input, as
    /// a take on the track would ([`crate::looping::TrackInput`]). A line
    /// pick switches the line input on, as a Loop track's Line in does.
    /// Records from shows it until another source is picked or a Loop
    /// cell is armed.
    pub fn loop_track_input(&mut self, input: crate::looping::TrackInput) -> Vec<Effect> {
        let mut fx = Vec::new();
        if matches!(input.from, crate::looping::TrackFrom::Line(_)) && !self.settings.line_input {
            fx.extend(self.looper_setting(Item::LineInput, 1));
        }
        let set = self.set_loop_source(LoopSource::Track(input));
        if !set.is_empty() {
            self.save_dirty = true;
            fx.extend(set);
            fx.push(Effect::Settings(self.settings.clone()));
        }
        fx
    }

    /// Records from (the Looper tab's picker): a mix, the master or one
    /// strip (a track or a submix), as `mixing::MixHost::loop_take` takes it.
    pub fn loop_mix(&mut self, strip: mpfree_fx::StripId) -> Vec<Effect> {
        let mut fx = self.set_loop_source(LoopSource::Mix(strip));
        if !fx.is_empty() {
            self.save_dirty = true;
            fx.push(Effect::Settings(self.settings.clone()));
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
        if self.on_tunings_page() {
            return &tunings_page::TUNINGS_ROWS[..self.tunings_rows().len()];
        }
        match (self.view, self.back) {
            (View::Prefs, _) => self.prefs_tab.rows(),
            (_, View::Looper) => &[
                Item::LoopSource,
                Item::LineInput,
                Item::LoopMonitor,
                Item::LoopBars,
                Item::LoopLatency,
                Item::LoopLevel,
                Item::LoopTempo,
                Item::SmartEnd,
                Item::LoopMemory,
            ],
            (
                _,
                View::Debug
                | View::Perf
                | View::Engines
                | View::Procs
                | View::Map
                | View::StatusBoard,
            ) => {
                // The MIDI page's monitor has its own Hold.
                &[
                    Item::ShowPressure,
                    Item::ClearLogs,
                    Item::StreamAudio,
                    Item::LoopsOnTracks,
                ]
            }
            // The full-screen touch checks, also in Prefs > Touch screen.
            (_, View::Touch) => &[Item::CalibrateTouch, Item::TouchLab],
            (_, View::Console) => &[Item::ConsoleFollow, Item::ClearConsole],
            (_, View::Arp) if self.mod_section => &arp::MOD_ROWS,
            (_, View::Arp) => &arp::ARP_OPTIONS,
            (_, View::Arrange) if self.show_view => &SHOW_OPTIONS,
            (_, View::Play) => &PLAY_OPTIONS,
            (_, View::Synth) if self.synth_tab_owns_tuning() => &synth_pads::SYNTH_OPTIONS,
            (_, View::Synth) => &synth_pads::SYNTH_OPTIONS_SONGS,
            _ => &[
                Item::Rate,
                Item::Tempo,
                Item::Metronome,
                Item::MetronomeLevel,
                Item::TimingStrip,
                Item::PadLevel,
                Item::Latch,
            ],
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
                    LatchMode::Add => "Add".to_string(),
                    _ => "Replace".to_string(),
                },
            ),
            Item::PedalLatch => (
                "Sustain pedal latches",
                if s.pedal_latches { "On" } else { "Off" }.to_string(),
            ),
            Item::QuickAdvance => (
                "Quick advance",
                if s.quick_advance { "On" } else { "Off" }.to_string(),
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
            Item::SetupBackup => ("Backup of your MPC", self.setup_backup_text()),
            Item::SetupAkaiKits => ("Akai's kits and samples", self.setup_akai_text()),
            Item::Appearance => (
                "Appearance",
                self.theme_choices()
                    .find(|t| t.0 == s.theme)
                    .map(|t| t.1)
                    .unwrap_or("Theme file")
                    .to_string(),
            ),
            Item::CalibrateTouch => ("Calibrate touch", "Start".to_string()),
            Item::TouchLab => ("Touch lab", "Open".to_string()),
            Item::Patches => ("Patches", "Open".to_string()),
            Item::SetEditor => ("Open the Set editor", "Open".to_string()),
            Item::CheckSet => ("Check the set", "Check".to_string()),
            Item::CloseSet => ("Close the set", "Close".to_string()),
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
            Item::SystemStatus => ("System status", "Open".to_string()),
            Item::Rate => ("Note repeat rate", s.rate.label().to_string()),
            Item::Tempo => ("Tempo", format!("{:.1} BPM", s.bpm)),
            Item::Metronome => (
                "Metronome",
                if s.metronome { "On" } else { "Off" }.to_string(),
            ),
            Item::MetronomeLevel => ("Metronome level", format!("{} dB", s.metronome_db)),
            Item::TimingStrip => (
                "Timing strip",
                if s.timing_strip { "On" } else { "Off" }.to_string(),
            ),
            Item::PadLevel => (
                "Pad level",
                match self.pad_level {
                    PadLevel::Off => "As played".to_string(),
                    PadLevel::Full => "Full (127)".to_string(),
                    PadLevel::Half => format!("Half ({})", s.half_level),
                },
            ),
            Item::Latch => ("Latch", if self.latched { "On" } else { "Off" }.to_string()),
            Item::ShowPressure => (
                "Pad pressure in the MIDI log",
                if s.show_pressure { "Shown" } else { "Hidden" }.to_string(),
            ),
            Item::ClearLogs => ("Clear the MIDI and event logs", "Clear".to_string()),
            Item::StreamAudio => (
                "Audio tracks from their files",
                match (s.stream_audio, s.stream_audio == s.audio_streams) {
                    (true, true) => "On",
                    (false, true) => "Off",
                    (true, false) => "On (next start)",
                    (false, false) => "Off (next start)",
                }
                .to_string(),
            ),
            Item::LoopsOnTracks => (
                "Loops on audio tracks",
                match (s.loops_on_tracks, s.loops_on_tracks == s.loops_on) {
                    (true, true) => "On",
                    (false, true) => "Off",
                    (true, false) => "On (next start)",
                    (false, false) => "Off (next start)",
                }
                .to_string(),
            ),
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
            Item::SendsMidi => (
                "Send MIDI and CV out",
                if s.sends_midi {
                    "On (sends MIDI)"
                } else {
                    "Off"
                }
                .to_string(),
            ),
            Item::LoopSource => ("Record from", s.loop_source.name().to_string()),
            Item::LoopMonitor => (
                "Monitor the input",
                if s.loop_monitor { "On" } else { "Off" }.to_string(),
            ),
            Item::LoopBars => ("First pass length", crate::looping::bars_text(s.loop_bars)),
            Item::LoopLatency => (
                if self.view == View::Prefs {
                    "Line input latency"
                } else {
                    "Latency to make up for"
                },
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
            Item::LoopTempo => (
                "Tempo from the first loop",
                if s.loop_tempo { "On" } else { "Off" }.to_string(),
            ),
            Item::SmartEnd => ("Smart loop end", smart_end_name(s.smart_end).to_string()),
            Item::LoopMemory => (
                "Loop memory",
                if s.loop_memory == self.loop_memory_now {
                    s.loop_memory.row(self.sample_rate)
                } else {
                    format!("{} (next start)", s.loop_memory.row(self.sample_rate))
                },
            ),
            Item::AudioMemory => (
                "Audio track memory",
                if s.audio_memory == self.audio_memory_now {
                    s.audio_memory.row(self.sample_rate)
                } else {
                    format!("{} (next start)", s.audio_memory.row(self.sample_rate))
                },
            ),
            Item::AtStart => (
                "At start",
                if s.open_last {
                    "Last project"
                } else {
                    "New project"
                }
                .to_string(),
            ),
            Item::Export => ("Export", s.export_mode.name().to_string()),
            Item::ExportFile => (
                "Export file",
                match s.export_bits {
                    mpfree_fx::Bits::I24 => "24-bit",
                    mpfree_fx::Bits::I16 => "16-bit, dithered",
                    mpfree_fx::Bits::F32 => "32-bit float",
                }
                .to_string(),
            ),
            Item::ExportRate => (
                "Export rate",
                format!("{} kHz", f64::from(s.export_rate) / 1000.0),
            ),
            Item::Arp(a) => self.arp_item_text(a),
            Item::Synth(a) => self.synth_item_text(a),
            // The Tunings page draws its own rows (tunings_page_view).
            Item::Tunings(_) => ("Tunings", String::new()),
            Item::Theme(t) => self.custom_text(t),
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
                "With Latch on (NOTE REPEAT twice, SHIFT +",
                "NOTE REPEAT, or the sustain pedal), pads keep",
                "repeating after you let go.",
                "Replace: a fresh press starts a new set.",
                "Toggle: each press adds or removes that pad.",
                "Add: every press stays until Latch goes.",
            ],
            Item::PedalLatch => vec![
                "On: while the arp plays (Repeat, Arp or",
                "Pattern), the sustain pedal latches it: hold",
                "the pedal and what you play keeps going after",
                "you let go; lift it and the notes you aren't",
                "holding stop. The pedal then doesn't hold the",
                "sound's notes. Direct, the pedal sustains as",
                "it always does.",
            ],
            Item::QuickAdvance => vec![
                "While a set plays: how Next song goes.",
                "Off: Next song arms the next song and a",
                "second press goes, while stopped too.",
                "On: with the transport stopped, one press",
                "goes.",
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
                "Default, Slate, Light, Console, Synthwave,",
                "Spaceship and AbleTown; the screen changes at",
                "once. The layout and the font stay the same. It",
                "is kept for the next start.",
            ],
            Item::SetupBackup => vec![
                "A full copy of your MPC, kept on the stick,",
                "so you can always go back. Push or + makes a",
                "new one at the next start (10 to 20 minutes),",
                "beside any already there; again, or -, takes",
                "it back. Restart with the power button.",
            ],
            Item::SetupAkaiKits => vec![
                "The kits and sounds that came with your MPC,",
                "into BROWSE. Push or + brings them in at the",
                "next start (about 10 minutes); again, or -,",
                "takes it back. Without a backup they are read",
                "from the MPC itself, which is never changed.",
            ],
            Item::CalibrateTouch => vec![
                "Tap a target in each corner to check that",
                "touches land where you touch. This version",
                "only reports and logs the result; nothing",
                "is saved. Exit or MENU leaves it.",
            ],
            Item::SetEditor => vec![
                "Opens the set playing in BROWSE's Sets place:",
                "its songs, how each starts and ends, its notes.",
                "The song playing plays on.",
            ],
            Item::CheckSet => vec![
                "The quick check on every song of the set: every",
                "part reads, every file is there and unchanged,",
                "and the keyboards each needs are connected.",
            ],
            Item::CloseSet => vec![
                "Ends the set. The song playing stays open as a",
                "plain project, and the Show view goes.",
            ],
            Item::Patches => vec![
                "The song's patches: splits and layers for your",
                "keyboard. New patch makes one from a layout;",
                "KEYS PLAY on the transport bar opens them too.",
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
            Item::SystemStatus => vec![
                "What is wrong, if anything: each part of",
                "the MPC and its problems, with what to try.",
                "The status chip at the top right opens it",
                "too. MENU closes it.",
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
            Item::Metronome => vec![
                "A click on each beat while counting in and",
                "recording, higher on the bar. Off to start;",
                "SHIFT + PLAY START turns it on or off too.",
            ],
            Item::MetronomeLevel => vec!["How loud the click is, -36 to -6 dB."],
            Item::TimingStrip => vec![
                "Your timing under the pattern in Beats:",
                "how early or late your hits land, the",
                "spread and the velocity. Off gives the",
                "pattern more lanes.",
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
            Item::ClearLogs => vec!["Empty the MIDI and event logs."],
            Item::StreamAudio => vec![
                "Takes play from their files on the drive",
                "through a cache as big as Audio track",
                "memory, which then only holds what records.",
                "From the next start. A test: off on the MPC",
                "until the stick tests have run.",
            ],
            Item::LoopsOnTracks => vec![
                "A loop a loop cell of an audio track names",
                "plays on that track, where the song and the",
                "launches say. From the next start. Being",
                "built: a project saved with it on may not",
                "open in a later build.",
            ],
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
            Item::SendsMidi => vec![
                "On: tracks set to play on MIDI play out of",
                "the MIDI Out jack, clock goes out as each",
                "port says, and the CV jacks follow the",
                "project's CV outputs. Off: nothing is sent,",
                "every track plays inside MPFree. Kept.",
            ],
            Item::LoopSource => vec![
                "What the looper records: the pads and the",
                "synth (before the click), or the line input.",
                "Turning Line input on picks it; recording",
                "the pads opens nothing. A mix (the master,",
                "a track or a submix) is picked on the",
                "Looper tab, under Records from.",
            ],
            Item::LoopMonitor => vec![
                "Hear the line input through the outputs,",
                "about 9 ms late. Leave it off if you hear",
                "the instrument another way.",
            ],
            Item::LoopBars => vec![
                "How long the looper's first pass is: the",
                "pattern's length (the longest clip playing),",
                "free (until STOP, or the bar line after",
                "Record again) or 1, 2, 4 or 8 bars. STOP",
                "keeps what you played, to the next bar line.",
            ],
            Item::LoopLatency => vec![
                "How late your playing reaches the looper and",
                "the audio tracks: the output buffer, the input",
                "buffer and the converters. Estimated from the",
                "audio buffer until you set it. Measure it with",
                "a cable from an output to the input.",
            ],
            Item::LoopLevel => vec![
                "The loop's playback level, -24 to +6 dB.",
                "0 dB plays it as it was recorded.",
            ],
            Item::LoopTempo => vec![
                "On: with the song stopped, the first loop",
                "records in your own time and its length sets",
                "the tempo: Length's bars at the tempo you play,",
                "or on Until tap the likeliest bars (a close call",
                "says so). The song starts as the loop comes",
                "round. Not while another device's clock leads,",
                "the song has more than one tempo, or exporting.",
            ],
            Item::SmartEnd => vec![
                "When tempo from the first loop closes a loop",
                "on the pads, keys, line input or a mix.",
                "Suggest: it closes on your tap, then offers",
                "the downbeat you played if your tap was off",
                "it. Auto: it listens a moment past your tap",
                "and closes on the downbeat you played; the",
                "song starts in step. Off: your tap stands.",
            ],
            Item::LoopMemory => vec![
                "How much sound the loops can hold, all",
                "loops and layers together: 1 to 20 minutes",
                "of stereo. It is kept in memory from the",
                "start, so a new size applies from the next",
                "start. The Looper tab shows what is left.",
            ],
            Item::AudioMemory => vec![
                "How much sound the audio tracks on the",
                "Song view can hold, every take together:",
                "30 seconds to 10 minutes of stereo. It is",
                "kept in memory from the start, so a new",
                "size applies from the next start.",
            ],
            Item::AtStart => vec![
                "What MPFree opens when it starts.",
                "Last project (the default): the project",
                "you last saved or opened, once the sound",
                "is going. New project: an empty song.",
            ],
            Item::Export => vec![
                "How Export (BROWSE, Projects) plays the song.",
                "Fast, quiet: faster than real time, the",
                "speakers quiet (the default).",
                "Listen: once in real time, heard as it",
                "records. A song that plays outside gear",
                "(MIDI, CV, the line input, plugins) listens",
                "anyway.",
            ],
            Item::ExportFile => vec![
                "The WAV files an export makes. 24-bit (the",
                "default) keeps everything the mixer makes.",
                "16-bit is for CDs and older gear, with a",
                "little noise (dither) so quiet parts stay",
                "smooth. 32-bit float keeps levels over full",
                "scale. The Export panel can change it for",
                "one export.",
            ],
            Item::ExportRate => vec![
                "The export's sample rate. 44.1 kHz is what",
                "MPFree plays at; 48 kHz (for video) is",
                "converted as the files are written, a",
                "little slower. The Export panel can change",
                "it for one export.",
            ],
            Item::Arp(a) => Self::arp_item_help(a),
            Item::Synth(a) => self.synth_item_help(a),
            Item::Tunings(_) | Item::Theme(_) => Vec::new(),
        }
    }

    pub fn menu_view(&self) -> Effect {
        if self.on_tunings_page() {
            return Effect::Menu(self.tunings_page_view());
        }
        let items = self.items();
        let selected = self.selected.min(items.len() - 1);
        let title = match (self.view, self.back) {
            (View::Prefs, _) => match self.prefs_tab.view_name() {
                Some(v) => format!("Prefs · {} · {v}", self.prefs_tab.name()),
                None => format!("Prefs · {}", self.prefs_tab.name()),
            },
            (_, View::Arp) if self.mod_section => "Arp options · Modifier".to_string(),
            (_, View::Arp) => "Arp options".to_string(),
            (_, View::Debug) => "MIDI log options".to_string(),
            (
                _,
                b @ (View::Perf
                | View::Engines
                | View::Procs
                | View::Map
                | View::Touch
                | View::StatusBoard
                | View::Console),
            ) => {
                format!("{} options", b.name())
            }
            (_, b) => format!("{} options", b.tab_view().unwrap_or(View::Play).name()),
        };
        let (name, _) = self.item_text(items[selected]);
        let (help_heading, help, help_buttons) = match self.theme_trial_help() {
            Some(asks) => asks,
            None => {
                let mut help: Vec<String> = self
                    .item_help(items[selected])
                    .into_iter()
                    .map(String::from)
                    .collect();
                if items[selected] == Item::Appearance {
                    help.extend(self.theme_problem_lines());
                }
                (name.to_string(), help, Vec::new())
            }
        };
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
            editing: self.menu_editing,
            help_heading,
            marks: items.iter().map(|&i| self.item_mark(i)).collect(),
            facts: match items[selected] {
                Item::Synth(synth_pads::SynthItem::Engine) => self.track_line.clone(),
                Item::Synth(synth_pads::SynthItem::Tuning) => self.synth_tuning_facts(),
                _ => Vec::new(),
            },
            facts_warn: items[selected] == Item::Synth(synth_pads::SynthItem::Tuning)
                && self.synth_tab_deaf(),
            help_link: match items[selected] {
                Item::Synth(
                    synth_pads::SynthItem::Tuning
                    | synth_pads::SynthItem::TuningRoot
                    | synth_pads::SynthItem::A4,
                ) => Some(tunings_page::link()),
                _ => None,
            },
            help,
            help_buttons,
            prefs: (self.view == View::Prefs).then(|| Box::new(self.prefs_view())),
        })
    }

    /// Row `item`'s marks: the Synth tab's Tuning row's and the Tunings
    /// page's (tunings step T6).
    fn item_mark(&self, item: Item) -> RowMark {
        match item {
            Item::Synth(a) => self.synth_item_mark(a),
            Item::Tunings(k) => self.tunings_item_mark(usize::from(k)),
            _ => RowMark::default(),
        }
    }

    /// The first-start setup's state, for Prefs > Backup and Akai kits
    /// (at start, and after each change there); the menu follows.
    pub fn set_setup(&mut self, state: Option<crate::setup::PrefsState>) -> Vec<Effect> {
        self.setup = state;
        if self.view == View::Prefs {
            vec![self.menu_view()]
        } else {
            Vec::new()
        }
    }

    /// The backup row's value.
    fn setup_backup_text(&self) -> String {
        let Some(st) = &self.setup else {
            return "Not on this stick".to_string();
        };
        let b = &st.backup;
        match (&st.backup_made, b.next_start, b.done, b.left_out) {
            (Some(_), true, _, _) => "New one at the next start".to_string(),
            (None, true, _, _) => "At the next start".to_string(),
            (Some(day), false, true, _) => format!("Made {day}"),
            (_, false, _, true) => "Left out".to_string(),
            _ => "Not made yet".to_string(),
        }
    }

    /// The Akai kits row's value.
    fn setup_akai_text(&self) -> String {
        let Some(st) = &self.setup else {
            return "Not on this stick".to_string();
        };
        let a = &st.akai;
        if a.done {
            "In BROWSE".to_string()
        } else if a.next_start {
            "At the next start".to_string()
        } else if a.left_out {
            "Left out".to_string()
        } else {
            "Not in yet".to_string()
        }
    }

    /// A push or + on a row of Prefs > Backup and Akai kits asks for it at
    /// the next start, or takes the ask back; - takes it back.
    fn setup_step(&mut self, backup: bool, d: i32) -> Vec<Effect> {
        let Some(st) = &self.setup else {
            return vec![Effect::Tell(
                "This stick has no first-start setup: it wasn't made by the USB creator".into(),
            )];
        };
        let c = if backup { &st.backup } else { &st.akai };
        if !backup && c.done {
            return vec![Effect::Tell(
                "Akai's kits and samples are in already: BROWSE, Kits and Samples".into(),
            )];
        }
        let want = d > 0 && !c.next_start;
        if want == c.next_start {
            return Vec::new();
        }
        vec![Effect::SetupNextStart { backup, want }]
    }

    /// A menu row touched.
    pub fn select(&mut self, i: usize) -> Vec<Effect> {
        if !self.in_menu() {
            return Vec::new();
        }
        let items = self.items();
        let i = i.min(items.len() - 1);
        if self.item_mark(items[i]).heading {
            // A heading is no row to pick.
            return Vec::new();
        }
        self.selected = i;
        self.menu_editing = false;
        vec![self.menu_view()]
    }

    /// The row the wheel moves to from row `from`, `delta` rows on: past
    /// the ends it stays at the end, and a heading is passed over the way
    /// it turned (or, at an end, it stays where it was).
    fn menu_row_from(&self, from: usize, delta: i32) -> usize {
        let items = self.items();
        let last = (items.len() as i32 - 1).max(0);
        let mut at = (from as i32 + delta).clamp(0, last);
        let way = delta.signum();
        while way != 0 && self.item_mark(items[at as usize]).heading {
            let next = at + way;
            if !(0..=last).contains(&next) {
                return from.min(last as usize);
            }
            at = next;
        }
        at as usize
    }

    /// SHIFT + the wheel's push goes to the outlined control: a page with
    /// the outline, with no list, menu or rows over it.
    fn resets_outlined(&self) -> bool {
        outlined(self.view)
            && !self.overlay
            && !self.in_menu()
            && !self.browsing()
            && !self.has_rows()
            && !self.repeat_held
    }

    /// The data dial's push. In a menu, on a row with a value: the wheel
    /// now changes that value (as it turns), and the next push sets it and
    /// gives the wheel back to the rows. On a row that opens something
    /// (System info, Touch screen, Back…), it opens it, as + does (Jay,
    /// 12 Nov 08:11Z). On a page with rows, the same for the row
    /// highlighted ([`RowKey`]). Elsewhere it does nothing yet.
    pub fn wheel_push(&mut self) -> Vec<Effect> {
        if self.wheel_to_held() {
            // A finger on a control: the push does nothing, so a thumb
            // that slips presses nothing.
            return Vec::new();
        }
        if self.overlay {
            // A list or pad over the page: the push is its Enter.
            return vec![Effect::Rows(RowKey::Editing(true))];
        }
        if !self.in_menu() && self.view == View::Chain {
            return vec![Effect::ChainPush];
        }
        // The Show view (live shows B1): the push is Go.
        if !self.in_menu() && self.view == View::Arrange && self.show_view {
            return vec![Effect::ShowKey(ShowKey::Push)];
        }
        if !self.in_menu() {
            // A page with rows: open the highlighted row for the wheel, or
            // close it again (RowKey).
            if self.has_rows() {
                self.page_editing = !self.page_editing;
                return vec![Effect::Rows(RowKey::Editing(self.page_editing))];
            }
            // The outlined control: pressed, or grabbed (a value).
            if outlined(self.view) && !self.repeat_held {
                return vec![Effect::Nav(Nav::Push)];
            }
            return Vec::new();
        }
        if self.menu_editing {
            self.menu_editing = false;
            let items = self.items();
            if items[self.selected.min(items.len() - 1)] == Item::Appearance
                && self.theme_trial.is_some()
            {
                // Setting the open Appearance row keeps the theme too.
                return self.end_theme_trial(true);
            }
            return vec![self.menu_view()];
        }
        let items = self.items();
        let i = self.selected.min(items.len() - 1);
        if items[i] == Item::Appearance && self.theme_trial.is_some() {
            // "Keep this theme?": the push keeps it.
            return self.end_theme_trial(true);
        }
        if items[i] == Item::Appearance {
            // The themes: the wheel tries them, so the push opens
            // Customise on the theme in use.
            return self.pick_prefs_tab(PrefsTab::Customise.index());
        }
        if opens_something(items[i]) || self.custom_opens(items[i]) {
            return self.step_item(i, 1);
        }
        if self.item_mark(items[i]).fixed {
            // A value that can't be changed: the push says why.
            return self.step_item(i, 1);
        }
        self.menu_editing = true;
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
        if let Item::Theme(t) = item {
            return self.custom_step(t, delta);
        }
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
                s.latch_mode = cycle(
                    &[LatchMode::Replace, LatchMode::Toggle, LatchMode::Add],
                    s.latch_mode,
                    d,
                );
                self.update_engine();
            }
            Item::PedalLatch => {
                s.pedal_latches = d > 0;
                // Turned off under a held pedal: the pedal lets go now.
                if !s.pedal_latches {
                    fx.extend(self.pedal_lets_go());
                }
            }
            Item::QuickAdvance => s.quick_advance = !s.quick_advance,
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
            Item::SetupBackup => return self.setup_step(true, delta),
            Item::SetupAkaiKits => return self.setup_step(false, delta),
            Item::Appearance => {
                // A theme file from --theme counts as before the first.
                let at = themes.iter().position(|t| *t == s.theme);
                let next = match at {
                    Some(i) => (i as i32 + d).clamp(0, themes.len() as i32 - 1) as usize,
                    None => 0,
                };
                let before = std::mem::replace(&mut s.theme, themes[next].clone());
                // "Keep this theme?": 15 s from this step, back to the
                // theme from before the first step; none once back on it.
                let until = self.tick_t + THEME_TRIAL_S;
                match &mut self.theme_trial {
                    _ if s.theme == before => {}
                    Some(t) if t.before == s.theme => self.theme_trial = None,
                    Some(t) => {
                        t.until = until;
                        t.shown = THEME_TRIAL_S as u32;
                    }
                    None => {
                        self.theme_trial = Some(ThemeTrial {
                            before,
                            until,
                            shown: THEME_TRIAL_S as u32,
                        })
                    }
                }
            }
            Item::CalibrateTouch => return vec![Effect::CalibrateTouch],
            Item::TouchLab => return vec![Effect::TouchLab],
            Item::Patches => {
                let mut fx = self.show(self.back);
                fx.push(Effect::PatchList);
                return fx;
            }
            Item::SetEditor | Item::CheckSet | Item::CloseSet => {
                let m = match item {
                    Item::SetEditor => SetMenu::Editor,
                    Item::CheckSet => SetMenu::Check,
                    _ => SetMenu::Close,
                };
                let mut fx = self.show(self.back);
                fx.push(Effect::SetMenu(m));
                return fx;
            }
            Item::SystemInfo => return self.show(View::System),
            Item::SystemStatus => return self.show(View::SystemStatus),
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
            Item::AtStart => s.open_last = d > 0,
            Item::Export => {
                s.export_mode = if d > 0 {
                    mpfree_export::Mode::Listen
                } else {
                    mpfree_export::Mode::Fast
                }
            }
            Item::ExportFile => s.export_bits = cycle(&mpfree_fx::Bits::ALL, s.export_bits, d),
            Item::ExportRate => s.export_rate = cycle(&EXPORT_RATES, s.export_rate, d),
            Item::LoopMemory => {
                let next = cycle(&LoopMemory::ALL, s.loop_memory, d);
                if next != s.loop_memory {
                    s.loop_memory = next;
                    fx.push(Effect::Note(format!(
                        "loop memory: {} min from the next start (now {} min)",
                        next.minutes(),
                        self.loop_memory_now.minutes()
                    )));
                }
            }
            Item::AudioMemory => {
                let next = cycle(&crate::audiotracks::AudioMemory::ALL, s.audio_memory, d);
                if next != s.audio_memory {
                    s.audio_memory = next;
                    fx.push(Effect::Note(format!(
                        "audio track memory: {} from the next start (now {})",
                        next.time(),
                        self.audio_memory_now.time()
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
                self.tempo_by_hand = true;
                self.update_engine();
                fx.push(self.status());
            }
            Item::Metronome => {
                s.metronome = !s.metronome;
                fx.push(self.metronome());
            }
            Item::MetronomeLevel => {
                s.metronome_db = cycle(&METRONOME_DBS, s.metronome_db, d);
                fx.push(self.metronome());
            }
            Item::TimingStrip => s.timing_strip = !s.timing_strip,
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
            Item::LoopsOnTracks => {
                let on = d > 0;
                if on != s.loops_on_tracks {
                    s.loops_on_tracks = on;
                    fx.push(Effect::Note(format!(
                        "looper: loops play {} from the next start",
                        if on {
                            "on audio tracks"
                        } else {
                            "on the Loop track"
                        }
                    )));
                }
            }
            Item::StreamAudio => {
                let on = d > 0;
                if on != s.stream_audio {
                    s.stream_audio = on;
                    fx.push(Effect::Note(format!(
                        "audio tracks: takes play from {} from the next start",
                        if on { "their files" } else { "memory" }
                    )));
                }
            }
            Item::SendsMidi => {
                let on = d > 0;
                if on != s.sends_midi {
                    s.sends_midi = on;
                    fx.push(Effect::Note(format!(
                        "MIDI and CV out: {}",
                        if on {
                            "on (the MIDI Out jack and the CV jacks send)"
                        } else {
                            "off (nothing is sent)"
                        }
                    )));
                }
            }
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
                const BARS: [Option<u32>; 6] = [
                    Some(crate::looping::PATTERN_BARS),
                    Some(1),
                    Some(2),
                    Some(4),
                    Some(8),
                    None,
                ];
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
            Item::LoopTempo => s.loop_tempo = d > 0,
            Item::SmartEnd => s.smart_end = cycle(&SMART_ENDS, s.smart_end, d),
            Item::LoopLevel => {
                s.loop_level_db = (s.loop_level_db + d as f32).clamp(-24.0, 6.0);
                fx.push(Effect::Looper(LoopAction::Command(LoopCommand::SetLevel(
                    10f32.powf(s.loop_level_db / 20.0),
                ))));
            }
            Item::Arp(a) => fx.extend(self.step_arp_item(a, d)),
            Item::Synth(a) => fx.extend(self.step_synth_item(a, d)),
            Item::Tunings(k) => fx.extend(self.step_tunings_row(usize::from(k), d)),
            Item::Theme(_) => {}
        }
        fx.push(Effect::Settings(self.settings.clone()));
        if self.in_menu() {
            fx.push(self.menu_view());
        }
        fx
    }

    // ------------------------------------------------------- theme trial

    /// The help panel while "Keep this theme?" asks (on every row): its
    /// heading, lines and buttons.
    fn theme_trial_help(&self) -> Option<(String, Vec<String>, Vec<String>)> {
        let t = self.theme_trial.as_ref()?;
        let name = |file: &str| {
            self.theme_choices()
                .find(|c| c.0 == file)
                .map_or(file.to_string(), |c| c.1.to_string())
        };
        let left = (t.until - self.tick_t).max(0.0).ceil() as u32;
        Some((
            format!("Keep {}?", name(&self.settings.theme)),
            vec![
                format!("Back to {} in {left} s.", name(&t.before)),
                "Keep, or push the wheel on Appearance, keeps it.".to_string(),
                "Go back or closing Prefs puts it back now.".to_string(),
            ],
            vec!["Keep".to_string(), "Go back".to_string()],
        ))
    }

    /// The Appearance panel's lines after its help: the problems of the
    /// theme file in use, then the theme files left out and why, each
    /// wrapped to the panel, at most [`THEME_PROBLEM_LINES`] in all.
    fn theme_problem_lines(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some((file, problems)) = self
            .theme_problems
            .iter()
            .find(|(f, _)| *f == self.settings.theme)
        {
            let name = self
                .theme_choices()
                .find(|c| c.0 == file)
                .map_or(file.as_str(), |c| c.1);
            out.push(String::new());
            out.push(format!(
                "{name}: {} problem{}",
                problems.len(),
                if problems.len() == 1 { "" } else { "s" }
            ));
            for p in problems {
                out.extend(wrap_line(p, HELP_CHARS));
            }
        }
        if !self.themes_left_out.is_empty() {
            out.push(String::new());
            out.push("Left out:".to_string());
            for (file, why) in &self.themes_left_out {
                out.extend(wrap_line(&format!("{file}: {why}"), HELP_CHARS));
            }
        }
        if out.len() > THEME_PROBLEM_LINES {
            out.truncate(THEME_PROBLEM_LINES - 1);
            out.push("More in the log (Dev tab, Console).".to_string());
        }
        out
    }

    /// Ends "Keep this theme?": `keep` keeps the theme on screen, else the
    /// theme from before comes back. Either way it is said in the log.
    fn end_theme_trial(&mut self, keep: bool) -> Vec<Effect> {
        let Some(t) = self.theme_trial.take() else {
            return Vec::new();
        };
        let name = |app: &App, file: &str| {
            app.theme_choices()
                .find(|c| c.0 == file)
                .map_or(file.to_string(), |c| c.1.to_string())
        };
        let now = name(self, &self.settings.theme);
        let note = if keep {
            format!("theme: kept {now}")
        } else {
            let line = format!("theme: back to {} ({now} not kept)", name(self, &t.before));
            self.settings.theme = t.before;
            line
        };
        // What the next start opens with changes either way.
        self.save_dirty = true;
        let mut fx = vec![Effect::Note(note), Effect::Settings(self.settings.clone())];
        if self.in_menu() {
            fx.push(self.menu_view());
        }
        fx
    }

    /// The help panel's buttons while "Keep this theme?" asks: Keep (0) or
    /// Go back (1).
    pub fn help_button(&mut self, i: usize) -> Vec<Effect> {
        if i == mpfree_ui::widgets::CARD_LINK {
            return self.follow_help_link();
        }
        if self.theme_trial.is_none() {
            return Vec::new();
        }
        self.end_theme_trial(i == 0)
    }

    /// "Keep this theme?" counting down, from the screen loop after
    /// [`App::tick`]: the panel each second, and the old theme back when
    /// the time is up.
    pub fn theme_tick(&mut self) -> Vec<Effect> {
        let Some(t) = &mut self.theme_trial else {
            return Vec::new();
        };
        if self.tick_t >= t.until {
            return self.end_theme_trial(false);
        }
        let left = (t.until - self.tick_t).ceil() as u32;
        if left == t.shown {
            return Vec::new();
        }
        t.shown = left;
        if self.in_menu() {
            vec![self.menu_view()]
        } else {
            Vec::new()
        }
    }

    /// The theme the next start opens with: the one on screen, or while
    /// "Keep this theme?" asks, the one it would go back to.
    pub fn kept_theme(&self) -> &str {
        self.theme_trial
            .as_ref()
            .map_or(&self.settings.theme, |t| &t.before)
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
    use mpfree_arp::{EventKind, ModSource, Order};
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

    /// A press and its release: what both did (STEP SEQ acts on the
    /// release).
    fn click(app: &mut App, note: u8) -> Vec<Effect> {
        let mut fx = app.button(note, true, 0.0);
        fx.extend(app.button(note, false, 0.0));
        fx
    }

    /// The Sequencer tab (the Play, Pads and Sequencer redesign): its
    /// Steps keep Play's wheel (the outline); the note editor opening
    /// (Arrange's Edit row, or Notes) shows the Sequencer, whose Notes
    /// view has Arrange's rows; STEP SEQ goes back to Steps, closing the
    /// editor (‹ Steps), and a row the wheel was changing closes with it.
    #[test]
    fn the_sequencer_shows_steps_or_the_note_editor() {
        use mpfree_time_ui::editor::EditAction;
        let mut app = ready();
        app.open_tab(View::Sequencer);
        assert_eq!(app.view(), View::Sequencer);
        assert_eq!(app.wheel(1), [Effect::Nav(Nav::Turn(1))]);
        assert_eq!(app.wheel_push(), [Effect::Nav(Nav::Push)]);
        // Arrange's Edit row: the editor opens on the Sequencer.
        app.open_tab(View::Arrange);
        let fx = app.set_notes_open(true);
        assert!(fx.contains(&Effect::Show(View::Sequencer)), "{fx:?}");
        assert!(app.notes_open());
        assert_eq!(app.set_notes_open(true), [], "already open");
        assert_eq!(app.wheel(2), [Effect::Rows(RowKey::Pick(2))]);
        assert_eq!(app.wheel_push(), [Effect::Rows(RowKey::Editing(true))]);
        assert_eq!(app.wheel(-1), [Effect::Rows(RowKey::Step(-1))]);
        // - / + go as their own key, SHIFT with them: the screen knows if
        // the drawer is closed, where they nudge and zoom (plan 1C.1).
        assert_eq!(
            press(&mut app, button::PLUS),
            [Effect::Rows(RowKey::Arrow {
                delta: 1,
                shift: false
            })]
        );
        let fx = held(&mut app, button::SHIFT, |a| press(a, button::MINUS));
        assert_eq!(
            fx,
            [Effect::Rows(RowKey::Arrow {
                delta: -1,
                shift: true
            })]
        );
        let fx = held(&mut app, button::SHIFT, |a| a.wheel(3));
        assert_eq!(
            fx,
            [Effect::ClipsWheel {
                delta: 3,
                scroll: true
            }]
        );
        // Another tab and back: the editor stays open there.
        app.open_tab(View::Synth);
        app.open_tab(View::Sequencer);
        assert_eq!(app.wheel(1), [Effect::Rows(RowKey::Pick(1))]);
        // STEP SEQ: Steps again, the editor closed by its ‹ Steps.
        assert_eq!(app.wheel_push(), [Effect::Rows(RowKey::Editing(true))]);
        let fx = click(&mut app, button::STEP_SEQ);
        assert!(
            fx.contains(&Effect::Time(TimeAction::Edit(EditAction::Button(0)))),
            "{fx:?}"
        );
        assert!(!fx.iter().any(|e| matches!(e, Effect::Show(_))), "{fx:?}");
        assert_eq!(
            app.set_notes_open(false),
            [Effect::Rows(RowKey::Editing(false))]
        );
        assert_eq!(app.wheel(1), [Effect::Nav(Nav::Turn(1))]);
        // Closed elsewhere, nothing moves.
        app.open_tab(View::Arrange);
        app.set_notes_open(true);
        assert_eq!(app.view(), View::Sequencer);
        app.open_tab(View::Mixer);
        assert_eq!(app.set_notes_open(false), []);
        assert_eq!(app.view(), View::Mixer);
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

    /// Step 5d: a track's own synth's pad map makes the pads press its
    /// notes into the track's arp, as the synth's layout does, while
    /// everything else about the pads stays as on a kit; held pads are let
    /// go when it comes or goes, and the synth's layout, when on, wins.
    #[test]
    fn an_own_synths_pad_map_presses_its_notes_into_the_arp() {
        use mpfree_arp::scale::pads::{layout, PadLayout, PadSetup};
        let mut app = ready();
        let pads = layout(&PadSetup {
            layout: PadLayout::Chromatic,
            ..PadSetup::default()
        });
        app.pad_down(3, 100);
        assert_eq!(notes(&mut app, 64), [3]);
        app.set_arp_layout(Some(pads));
        assert!(app.eng().active().is_empty(), "the held pad let go");
        notes(&mut app, 64);
        assert!(!app.arp_plays_notes(), "the pads still play as on a kit");
        app.pad_down(3, 100);
        let mut heard = notes(&mut app, 64);
        heard.sort_unstable();
        let mut want = pads[3].notes().to_vec();
        want.sort_unstable();
        assert_eq!(heard, want, "pad 4's notes");
        assert!(app.live_map().plays_notes());
        assert_eq!(app.pad_of_note(pads[3].notes()[0]), Some(3));
        app.pad_up(3);
        assert!(app.eng().active().is_empty());
        // Gone again: pad numbers, held pads let go.
        app.pad_down(5, 100);
        notes(&mut app, 64);
        app.set_arp_layout(None);
        assert!(app.eng().active().is_empty());
        notes(&mut app, 64);
        app.pad_down(3, 100);
        assert_eq!(notes(&mut app, 64), [3]);
    }

    /// A keyboard plays a track's own synth by note, through its arp, as
    /// it plays the shared synth (the instrument redesign's step 7): not
    /// as the 16 pads the MPC gives its notes.
    #[test]
    fn keys_play_a_tracks_own_synth_by_note() {
        use mpfree_arp::scale::pads::{layout, PadLayout, PadSetup};
        let mut app = ready();
        assert!(!app.keys_play_notes(), "a kit: keys hit its pads");
        let pads = layout(&PadSetup {
            layout: PadLayout::Chromatic,
            ..PadSetup::default()
        });
        app.set_arp_layout(Some(pads));
        assert!(app.keys_play_notes());
        // A key plays its own note, one no pad has too.
        app.key_down(97, 90);
        app.key_down(61, 90);
        let mut heard = notes(&mut app, 64);
        heard.sort_unstable();
        assert_eq!(heard, [61, 97]);
        // A key on a kit pad's number does nothing while keys play notes.
        assert!(app.kit_key_down(3, 100).is_empty());
        app.key_up(97);
        assert!(!app.eng().active().contains(97));
        // The track goes back to a kit: the key still held lets go.
        app.set_arp_layout(None);
        assert!(app.eng().active().is_empty(), "nothing hangs");
        assert!(!app.keys_play_notes());
        app.key_down(62, 90);
        assert!(
            app.eng().active().is_empty(),
            "a kit's keys hit pads instead"
        );
    }

    #[test]
    fn keys_play_notes_and_share_holds_with_pads() {
        use mpfree_arp::scale::pads::{layout, PadLayout, PadSetup};
        use mpfree_arp::scale::{Scale, MAJOR};
        let mut app = ready();
        // With a kit (no pad layout) keys don't press notes here: the app
        // hits pads by the MPC's pad notes instead.
        app.key_down(60, 100);
        assert!(app.eng().active().is_empty());
        app.key_up(60);
        let pads = layout(&PadSetup {
            layout: PadLayout::Chords,
            scale: Some(Scale::new(0, MAJOR)),
            ..PadSetup::default()
        });
        app.set_pad_layout(Some(pads));
        // A key plays its note, even one no pad has.
        app.key_down(97, 90);
        assert_eq!(notes(&mut app, 64), [97]);
        // A second note-on for a held key is ignored; one note-off ends it.
        app.key_down(97, 90);
        app.key_up(97);
        assert!(!app.eng().active().contains(97));
        // Pad 1 holds C (48); the key on C keeps it after the pad lets go,
        // and C ends only when both have let go.
        app.pad_down(0, 100);
        app.key_down(48, 100);
        app.pad_up(0);
        assert!(app.eng().active().contains(48), "the key still holds C");
        app.key_up(48);
        assert!(app.eng().active().is_empty());
        // Keys held when the pads go back to a kit are let go.
        app.key_down(50, 100);
        app.set_pad_layout(None);
        assert!(app.eng().active().is_empty());
        app.set_pad_layout(Some(pads));
        app.key_up(50);
        assert!(app.eng().active().is_empty(), "nothing was left to let go");
    }

    /// With a kit, a key hits the kit pad its note names (A1 to H16,
    /// `profile::kit_pad_of_note`), whatever bank the pads show. It shares
    /// the hold with a pad of the MPC on the same kit pad, and a track
    /// change lets go of the old track's kit pads in every bank.
    #[test]
    fn keys_hit_a_kits_pads_in_every_bank() {
        let mut app = ready();
        // Bank C on the pads: a key on B5 (kit pad 20) still hits B5.
        app.set_pad_page(2);
        app.kit_key_down(20, 100);
        assert_eq!(notes(&mut app, 64), [20]);
        assert!(matches!(
            app.pressed_pad_event(20, 100, true),
            Some(mpfree_play::Event::NoteOn { note: 20, .. })
        ));
        // Bank B: pad 6 and the key on B6 (21) hold it together.
        app.set_pad_page(1);
        app.kit_key_down(21, 100);
        app.pad_down(5, 100);
        app.kit_key_up(21);
        assert!(app.eng().active().contains(21), "pad 6 still holds B6");
        app.pad_up(5);
        assert!(!app.eng().active().contains(21));
        // A second note-on for a held key is ignored; one note-off ends it.
        app.kit_key_down(40, 90);
        app.kit_key_down(40, 90);
        app.kit_key_up(40);
        assert!(!app.eng().active().contains(40));
        // A pad pressed again before it was let go: one let-go ends it.
        app.pad_down(0, 100);
        app.pad_down(0, 100);
        app.pad_up(0);
        assert!(!app.eng().active().contains(16));
        // B5's key is still down, and H16's (127) goes down: a track
        // change lets both go on the old track, and their keys' let-go
        // later changes nothing on the new one.
        app.kit_key_down(127, 100);
        assert!(app.eng().active().contains(20) && app.eng().active().contains(127));
        app.set_track(1);
        assert!(app.arps.engine_at(0).active().is_empty());
        app.kit_key_up(20);
        app.kit_key_up(127);
        assert!(app.eng().active().is_empty());
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
        // On the let-go (held, a pad hit sets the level).
        let fx = click(&mut app, button::FULL_LEVEL);
        assert!(fx.contains(&Effect::Light(button::FULL_LEVEL, LIGHT_ON)));
        assert_eq!(app.velocity(40), 127);
        app.button(button::SHIFT, true, 0.0);
        click(&mut app, button::FULL_LEVEL);
        app.button(button::SHIFT, false, 0.0);
        assert_eq!(app.velocity(127), 64, "SHIFT + FULL LEVEL is Half Level");
        let fx = app.button(button::SHIFT, true, 0.0);
        assert!(matches!(&fx[0], Effect::Status(s) if s.shift));
        let fx = click(&mut app, button::FULL_LEVEL);
        assert!(fx.contains(&Effect::Light(button::FULL_LEVEL, LIGHT_OFF)));
        assert_eq!(app.velocity(40), 40);
    }

    /// Hold FULL LEVEL and hit a pad (the MPC's own level): every hit then
    /// plays as hard as that one, as Half Level, so SHIFT + FULL LEVEL
    /// turns it off and on; the let-go changes nothing more. Kept in Prefs.
    #[test]
    fn full_level_held_and_a_hit_sets_the_level_every_hit_plays_at() {
        let mut app = ready();
        app.button(button::FULL_LEVEL, true, 0.0);
        assert!(app.status_now().full_held);
        let fx = app.full_level_hit(92);
        assert!(fx.contains(&Effect::Light(button::FULL_LEVEL, LIGHT_ON)));
        assert!(fx
            .iter()
            .any(|e| matches!(e, Effect::Tell(t) if t.starts_with("Every hit plays at 92"))));
        assert_eq!(app.velocity(20), 92);
        assert_eq!(app.velocity(127), 92);
        // A second hit while held: that one instead.
        app.full_level_hit(70);
        assert_eq!(app.velocity(20), 70);
        let fx = app.button(button::FULL_LEVEL, false, 0.0);
        assert!(!fx.iter().any(|e| matches!(e, Effect::Light(..))), "{fx:?}");
        assert_eq!(app.velocity(20), 70, "the let-go keeps it");
        assert_eq!(app.settings.half_level, 70);
        let st = app.status_now();
        assert!(!st.full_held);
        assert_eq!((st.pad_level, st.half_level), (PadLevel::Half, 70));
        // SHIFT + FULL LEVEL: off, and on again at 70.
        held(&mut app, button::SHIFT, |a| click(a, button::FULL_LEVEL));
        assert_eq!(app.velocity(20), 20);
        held(&mut app, button::SHIFT, |a| click(a, button::FULL_LEVEL));
        assert_eq!(app.velocity(20), 70);
        // Not held: a hit sets nothing.
        assert!(app.full_level_hit(30).is_empty());
        assert_eq!(app.velocity(20), 70);
    }

    /// SHIFT + Q-LINK steps the Q-Link column back a bank, as on the MPC;
    /// Q-LINK alone forward.
    #[test]
    fn shift_q_link_goes_back_a_bank() {
        let mut app = ready();
        app.open_tab(View::Play);
        assert_eq!(press(&mut app, button::Q_LINK), [Effect::QLinkBank]);
        let fx = held(&mut app, button::SHIFT, |a| press(a, button::Q_LINK));
        assert_eq!(fx, [Effect::QLinkBankBack]);
        app.open_tab(View::Sample);
        assert!(held(&mut app, button::SHIFT, |a| press(a, button::Q_LINK)).is_empty());
    }

    /// TC (the MPC's Timing Correct): the note editor on the Sequencer
    /// tab, then Timing Correct; SHIFT + TC turns Quantise while recording
    /// off or on (the MPC's TC On/Off).
    #[test]
    fn tc_opens_timing_correct_and_shift_tc_turns_quantise_on_and_off() {
        use mpfree_time_ui::editor::EditAction;
        let mut app = ready();
        app.open_tab(View::Play);
        let fx = press(&mut app, button::TC);
        assert!(fx.contains(&Effect::Show(View::Sequencer)), "{fx:?}");
        assert!(fx.contains(&Effect::Time(TimeAction::Edit(EditAction::Open))));
        assert!(
            fx.contains(&Effect::EditTc),
            "the drawer opens on TC's settings"
        );
        app.set_notes_open(true);
        assert_eq!(press(&mut app, button::TC), [Effect::EditTc]);
        assert_eq!(
            held(&mut app, button::SHIFT, |a| press(a, button::TC)),
            [Effect::RecordQuantiseToggle]
        );
    }

    /// Hold MAIN (the MPC's hold MAIN): the pads pick a track, PAD BANK
    /// which 16, and the let-go goes nowhere; a quick press is still
    /// MAIN's page.
    #[test]
    fn main_held_picks_a_track_with_a_pad() {
        let mut app = ready();
        app.set_track_count(20);
        app.open_tab(View::Sample);
        app.button(button::MAIN, true, 0.0);
        assert_eq!(app.main_held(), Some(0));
        assert_eq!(app.status_now().main_held, Some(0));
        assert_eq!(app.main_pick(2), [Effect::Time(TimeAction::Pick(2))]);
        // PAD BANK B: tracks 17-32; track 21 isn't there.
        app.button(button::PAD_BANK[1], true, 0.0);
        app.button(button::PAD_BANK[1], false, 0.0);
        assert_eq!(app.main_held(), Some(1));
        assert_eq!(app.main_pick(3), [Effect::Time(TimeAction::Pick(19))]);
        assert!(matches!(&app.main_pick(4)[..], [Effect::Tell(t)] if t.contains("no track 21")));
        let fx = app.button(button::MAIN, false, 0.0);
        assert!(!fx.iter().any(|e| matches!(e, Effect::Show(_))), "{fx:?}");
        assert_eq!(app.view(), View::Sample);
        assert_eq!(app.main_held(), None);
        assert!(app.main_pick(0).is_empty(), "MAIN let go");
        // A quick press: Play.
        assert!(click(&mut app, button::MAIN).contains(&Effect::Show(View::Play)));
    }

    /// SHIFT + a kit pad picks it without playing it (STEP SEQ's next
    /// sound too); without SHIFT, or with the pads not on the kit, the pad
    /// plays as ever.
    #[test]
    fn shift_and_a_pad_picks_it_without_playing_it() {
        let mut app = ready();
        assert!(app.pick_pad_quietly(3).is_none(), "SHIFT isn't held");
        app.button(button::SHIFT, true, 0.0);
        let fx = app.pick_pad_quietly(3).unwrap();
        assert_eq!(fx[0], Effect::PadPick { slot: 3 });
        assert!(matches!(&fx[1], Effect::Tell(t) if t.contains("pad A4 picked, not played")));
        app.button(button::ERASE, true, 0.0);
        assert!(
            app.pick_pad_quietly(4).is_none(),
            "SHIFT + ERASE is kept free"
        );
        app.button(button::ERASE, false, 0.0);
        app.button(button::SHIFT, false, 0.0);
        // STEP SEQ held takes the pad picked last as its sound.
        app.button(button::STEP_SEQ, true, 0.0);
        assert_eq!(app.step_pads().map(|p| p.sound), Some(3));
    }

    /// SHIFT + PLAY plays from where PLAY last started; SHIFT + OVERDUB
    /// turns the count-in off or on; SHIFT + SAMPLE EDIT is the macros
    /// panel and SHIFT + PROG EDIT controller mode's settings. Without
    /// SHIFT each does what it did.
    #[test]
    fn the_shift_layers_of_play_overdub_sample_edit_and_prog_edit() {
        let mut app = ready();
        app.open_tab(View::Play);
        let shifted = |app: &mut App, note: u8| held(app, button::SHIFT, |a| press(a, note));
        assert!(shifted(&mut app, button::PLAY).contains(&Effect::PlayBack));
        assert!(press(&mut app, button::PLAY)
            .contains(&Effect::Time(TimeAction::Transport(Command::Play))));
        let overdub = app.map.overdub.unwrap();
        assert_eq!(shifted(&mut app, overdub), [Effect::CountInToggle]);
        assert_eq!(
            press(&mut app, overdub),
            [Effect::Rec(crate::rec_arm::Press::Overdub)]
        );
        assert_eq!(
            shifted(&mut app, button::SAMPLE_EDIT),
            [Effect::MacrosPanel]
        );
        assert!(press(&mut app, button::SAMPLE_EDIT)
            .contains(&Effect::SampleEditor(crate::sample_page::Editor::Wave)));
        assert_eq!(
            shifted(&mut app, button::PROG_EDIT),
            [Effect::ControllerSettings]
        );
        // SHIFT + NEXT SEQ: the XY pad full screen, from Play or from
        // another tab (Play first); NEXT SEQ alone still goes to Arrange.
        app.open_tab(View::Play);
        assert_eq!(shifted(&mut app, button::NEXT_SEQ), [Effect::XyFull]);
        app.open_tab(View::Mixer);
        let fx = shifted(&mut app, button::NEXT_SEQ);
        assert_eq!(fx.last(), Some(&Effect::XyFull));
        assert_eq!(app.view, View::Play);
        press(&mut app, button::NEXT_SEQ);
        assert_eq!(app.view, View::Arrange);
    }

    /// TAP TEMPO held: the wheel turns the tempo 1 BPM a detent (SHIFT:
    /// 0.1) and the let-go opens nothing; held alone for a while, the
    /// number pad opens as it is let go; a tap opens nothing.
    #[test]
    fn tap_tempo_held_turns_the_tempo_with_the_wheel_or_types_it() {
        let mut app = ready();
        let bpm = app.settings.bpm;
        app.button(button::TAP_TEMPO, true, 10.0);
        assert!(app.status_now().tap_held);
        let fx = app.wheel(2);
        assert_eq!(
            fx[0],
            Effect::Time(TimeAction::Transport(Command::SetTempo(bpm + 2.0)))
        );
        assert!(matches!(&fx[1], Effect::Tell(t) if t.contains("TAP TEMPO held")));
        app.button(button::SHIFT, true, 10.1);
        assert_eq!(
            app.wheel(-3)[0],
            Effect::Time(TimeAction::Transport(Command::SetTempo(
                ((bpm - 0.3) * 10.0).round() / 10.0
            )))
        );
        app.button(button::SHIFT, false, 10.2);
        let fx = app.button(button::TAP_TEMPO, false, 11.0);
        assert!(!fx.contains(&Effect::TypeTempo), "the wheel turned it");
        assert!(!app.status_now().tap_held);
        // Held alone: the number pad.
        app.button(button::TAP_TEMPO, true, 20.0);
        assert!(app
            .button(button::TAP_TEMPO, false, 20.0 + TAP_HOLD_SECONDS)
            .contains(&Effect::TypeTempo));
        // A tap: nothing more.
        app.button(button::TAP_TEMPO, true, 30.0);
        assert!(!app
            .button(button::TAP_TEMPO, false, 30.1)
            .contains(&Effect::TypeTempo));
    }

    /// All four PAD BANK buttons held take a screen picture (the MPC's
    /// screenshot): the fourth does only that, and again while the other
    /// three stay down; one PAD BANK alone is a bank again.
    #[test]
    fn all_four_pad_banks_take_a_screen_picture() {
        let mut app = ready();
        for k in 0..3 {
            let fx = app.button(button::PAD_BANK[k], true, 0.0);
            assert!(!fx.contains(&Effect::ScreenPicture), "{fx:?}");
        }
        assert_eq!(
            app.button(button::PAD_BANK[3], true, 0.0),
            [Effect::ScreenPicture]
        );
        app.button(button::PAD_BANK[3], false, 0.0);
        assert_eq!(
            app.button(button::PAD_BANK[3], true, 0.0),
            [Effect::ScreenPicture]
        );
        for k in 0..4 {
            app.button(button::PAD_BANK[k], false, 0.0);
        }
        assert!(!app
            .button(button::PAD_BANK[3], true, 0.0)
            .contains(&Effect::ScreenPicture));
    }

    /// COPY held over the kit's pads (the MPC's COPY): the first pad hit
    /// is copied onto each one after it; with SHIFT held as COPY goes down
    /// (the MPC's Delete) each pad hit is emptied. Over STEP SEQ's steps
    /// COPY stays theirs.
    #[test]
    fn copy_held_copies_a_pad_onto_others_and_shift_copy_empties_them() {
        let mut app = ready();
        assert!(app.copy_pad(0).is_none(), "COPY isn't held");
        app.button(button::COPY, true, 0.0);
        assert!(
            matches!(&app.copy_pad(0).unwrap()[..], [Effect::Tell(t)] if t.contains("A1 picked"))
        );
        assert_eq!(
            app.copy_pad(3).unwrap(),
            [Effect::PadCopy { from: 0, to: 3 }]
        );
        assert_eq!(
            app.copy_pad(5).unwrap(),
            [Effect::PadCopy { from: 0, to: 5 }]
        );
        assert!(
            matches!(&app.copy_pad(0).unwrap()[..], [Effect::Tell(t)] if t.contains("being copied"))
        );
        app.button(button::COPY, false, 0.0);
        assert!(app.copy_pad(3).is_none());
        // SHIFT + COPY: each pad hit is emptied.
        app.button(button::SHIFT, true, 0.0);
        app.button(button::COPY, true, 0.0);
        app.button(button::SHIFT, false, 0.0);
        assert_eq!(app.copy_pad(6).unwrap(), [Effect::PadClear { pad: 6 }]);
        app.button(button::COPY, false, 0.0);
        // A new COPY copies again.
        app.button(button::COPY, true, 0.0);
        assert!(matches!(&app.copy_pad(6).unwrap()[..], [Effect::Tell(_)]));
        app.button(button::COPY, false, 0.0);
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
        // ERASE held tells the screen (ERASE + a tap deletes), nothing more.
        assert!(matches!(
            app.button(ERASE, true, 0.0).as_slice(),
            [Effect::Status(s)] if s.erase
        ));
        assert_eq!(press(&mut app, tap), vec![Effect::Show(View::Console)]);
        assert!(app.taps.is_empty(), "no tempo tap");
        assert_eq!(press(&mut app, tap), vec![Effect::Show(View::Play)]);
        // Without SHIFT, or once ERASE is let go, TAP TEMPO taps.
        app.button(button::SHIFT, false, 0.0);
        press(&mut app, tap);
        assert_eq!(app.taps.len(), 1);
        app.button(button::SHIFT, true, 0.0);
        assert!(matches!(
            app.button(ERASE, false, 0.0).as_slice(),
            [Effect::Status(s)] if !s.erase
        ));
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

    /// Prefs' tab with `item` opened, and the item's row there.
    fn go(app: &mut App, item: Item) -> usize {
        app.prefs_go_to(item).expect("row shown")
    }

    /// The Show view (live shows B1): the wheel and - / + arm the song
    /// after or before, the wheel's push is Go, MENU has the view's own
    /// rows, which go back to it and tell the screen, and the learned
    /// Panic and Click are the app's own controls.
    #[test]
    fn the_show_view_takes_the_wheel_its_push_and_menu() {
        use mpfree_play::mpfree_param::show;
        let mut app = App::new(44_100, false);
        app.map.menu = Some(MENU);
        app.open_tab(View::Arrange);
        let show_keys = |fx: &[Effect]| fx.iter().any(|e| matches!(e, Effect::ShowKey(_)));
        // A set open but another of Arrange's views up: as before.
        app.set_show_state(true, false);
        assert!(!show_keys(&app.wheel(1)));
        assert!(!show_keys(&press(&mut app, button::DATA_PUSH)));
        app.set_show_state(true, true);
        assert_eq!(app.wheel(3), [Effect::ShowKey(ShowKey::Step(1))]);
        assert_eq!(app.wheel(-1), [Effect::ShowKey(ShowKey::Step(-1))]);
        assert_eq!(
            press(&mut app, button::PLUS),
            [Effect::ShowKey(ShowKey::Step(1))]
        );
        assert_eq!(
            press(&mut app, button::MINUS),
            [Effect::ShowKey(ShowKey::Step(-1))]
        );
        assert_eq!(
            press(&mut app, button::DATA_PUSH),
            [Effect::ShowKey(ShowKey::Push)]
        );
        // SHIFT + wheel still scrolls.
        let fx = held(&mut app, button::SHIFT, |a| a.wheel(1));
        assert!(!show_keys(&fx), "{fx:?}");
        // MENU: Open the Set editor, Check the set, Close the set.
        press(&mut app, MENU);
        assert_eq!(app.view(), View::Options);
        assert_eq!(app.items(), &SHOW_OPTIONS);
        let fx = app.step_item(row(&app, Item::CloseSet), 1);
        assert!(fx.contains(&Effect::SetMenu(SetMenu::Close)), "{fx:?}");
        assert_eq!(app.view(), View::Arrange);
        // The set closed: Arrange's own rows and wheel again.
        app.set_show_state(false, true);
        assert!(!show_keys(&app.wheel(1)));
        press(&mut app, MENU);
        assert_ne!(app.items(), &SHOW_OPTIONS);
        // Learned Panic and Click are the app's; the rest the screen's.
        let click = app.settings.metronome;
        assert!(app.show_press(show::CLICK).is_some());
        assert_ne!(app.settings.metronome, click);
        let fx = app.show_press(show::PANIC).unwrap();
        assert!(fx.contains(&Effect::Time(TimeAction::Transport(Command::StopAndRewind))));
        assert!(
            fx.contains(&Effect::Looper(LoopAction::StopFree)),
            "free loops too"
        );
        assert!(app.show_press(show::GO).is_none());
        assert!(app.show_press(show::NEXT_SONG).is_none());
    }

    #[test]
    fn prefs_appearance_steps_through_the_themes() {
        let mut app = App::new(44_100, false);
        app.map.menu = Some(MENU);
        app.button(button::SHIFT, true, 0.0);
        press(&mut app, MENU);
        app.button(button::SHIFT, false, 0.0);
        let at = go(&mut app, Item::Appearance);
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
                .any(|e| matches!(e, Effect::Settings(s) if s.theme == "slate")),
            "{fx:?}"
        );
        assert_eq!(value(&fx), "Slate");
        assert_eq!(value(&app.step_item(at, 1)), "Light");
        assert_eq!(value(&app.step_item(at, 1)), "Console");
        app.step_item(at, 1);
        assert_eq!(value(&app.step_item(at, 1)), "Spaceship");
        assert_eq!(value(&app.step_item(at, 1)), "AbleTown");
        // It stops at the ends.
        assert_eq!(value(&app.step_item(at, 1)), "AbleTown");
        assert_eq!(value(&app.step_item(at, -1)), "Spaceship");
        assert_eq!(value(&app.step_item(at, -1)), "Synthwave");
        // Kept (Keep under "Keep this theme?"), and it comes back at the
        // next start.
        app.help_button(0);
        let kept = app.app_settings();
        assert_eq!(kept.get_str("theme"), Some("synthwave"));
        let mut next = App::new(44_100, false);
        next.restore(&kept, &mpfree_store::Settings::default());
        assert_eq!(next.settings.theme, "synthwave");
    }

    /// Jay, 17 Nov: while a finger rests on a control with a value, the
    /// wheel and - / + turn it on every page (SHIFT + wheel finer), the
    /// push does nothing, and on the lift each goes back to its own job.
    /// A list or pad over the page keeps them.
    #[test]
    fn a_held_control_takes_the_wheel_and_minus_plus_until_it_lifts() {
        let mut app = ready();
        let held = |delta, fine| vec![Effect::Held { delta, fine }];
        for v in [
            View::Play,
            View::Sample,
            View::Synth,
            View::Mixer,
            View::Arrange,
        ] {
            app.show(v);
            let before = app.wheel(1);
            app.set_held(true);
            assert_eq!(app.wheel(2), held(2, false), "{v:?}");
            assert_eq!(press(&mut app, button::PLUS), held(1, false), "{v:?}");
            assert_eq!(press(&mut app, button::MINUS), held(-1, false), "{v:?}");
            assert!(
                app.wheel_push().is_empty(),
                "{v:?}: the push presses nothing"
            );
            app.button(button::SHIFT, true, 0.0);
            assert_eq!(app.wheel(-3), held(-3, true), "{v:?}: SHIFT is finer");
            // The button combos of 19 Nov: SHIFT + - / + finer too, and
            // SHIFT + the push back to the default.
            assert_eq!(press(&mut app, button::PLUS), held(1, true), "{v:?}");
            assert_eq!(press(&mut app, button::DATA_PUSH), [Effect::Reset], "{v:?}");
            app.button(button::SHIFT, false, 0.0);
            app.set_held(false);
            assert_eq!(app.wheel(1), before, "{v:?}: let go, the page's own job");
        }
        // A list or pad over the page has them, finger or not.
        app.set_held(true);
        app.set_overlay(true);
        assert_eq!(app.wheel(1), vec![Effect::Rows(RowKey::Pick(1))]);
    }

    /// The button combos of 19 Nov: on a page with the wheel's outline,
    /// SHIFT + the push puts the outlined value back to its default and
    /// SHIFT + - / + step it finer; on a page with rows or a list over the
    /// page they do as without SHIFT.
    #[test]
    fn shift_push_resets_and_shift_minus_plus_step_finer_on_the_outline() {
        let mut app = ready();
        let shifted = |app: &mut App, note: u8| held(app, button::SHIFT, |a| press(a, note));
        for v in [View::Mixer, View::Play, View::Looper] {
            app.open_tab(v);
            app.set_outline(true);
            assert_eq!(
                shifted(&mut app, button::DATA_PUSH),
                [Effect::Reset],
                "{v:?}"
            );
            assert_eq!(
                shifted(&mut app, button::MINUS),
                [Effect::Nav(Nav::Fine(-1))],
                "{v:?}"
            );
            assert_eq!(
                press(&mut app, button::PLUS),
                [Effect::Nav(Nav::Step(1))],
                "{v:?}"
            );
        }
        app.set_overlay(true);
        assert_eq!(
            shifted(&mut app, button::DATA_PUSH),
            [Effect::Rows(RowKey::Editing(true))],
            "a list over the page keeps the push"
        );
    }

    /// Jay, 12 Nov 08:11Z: in Prefs, a push of the wheel opens a row, the
    /// wheel then picks its value, and a second push sets it and gives the
    /// wheel back to the rows. A row that opens something opens at once.
    #[test]
    fn the_wheels_push_opens_a_row_for_the_wheel_and_a_second_push_sets_it() {
        const PUSH: u8 = 111;
        let mut app = App::new(44_100, false);
        app.map.menu = Some(MENU);
        // Found with Map controls, as on Jay's MPC One.
        app.map.learn(mapper::Job::DataPush, Some(PUSH));
        app.button(button::SHIFT, true, 0.0);
        press(&mut app, MENU);
        app.button(button::SHIFT, false, 0.0);
        let at = go(&mut app, Item::LatchMode);
        let menu = |fx: &[Effect]| match fx.last() {
            Some(Effect::Menu(m)) => m.clone(),
            _ => panic!("{fx:?}"),
        };
        // The wheel walks the rows to Latch keeps pads.
        let fx = app.select(0);
        assert!(!menu(&fx).editing);
        let fx = app.wheel(at as i32);
        assert_eq!((menu(&fx).selected, menu(&fx).editing), (at, false));
        // Push: the row opens, and the wheel now changes the setting.
        let fx = press(&mut app, PUSH);
        assert_eq!((menu(&fx).selected, menu(&fx).editing), (at, true));
        let before = menu(&fx).rows[at].1.clone();
        let fx = app.wheel(1);
        assert_ne!(menu(&fx).rows[at].1, before);
        assert_eq!(menu(&fx).selected, at, "the row stays picked");
        let picked = menu(&fx).rows[at].1.clone();
        // Push again: set, and the wheel moves between rows again.
        let fx = press(&mut app, PUSH);
        assert!(!menu(&fx).editing);
        assert_eq!(app.item_text(Item::LatchMode).1, picked);
        let fx = app.wheel(-1);
        assert_eq!((menu(&fx).selected, menu(&fx).editing), (at - 1, false));
        // A tap on a row, or leaving Prefs, closes an open row.
        app.select(at);
        assert!(menu(&press(&mut app, PUSH)).editing);
        let fx = app.select(0);
        assert!(!menu(&fx).editing);
        app.select(at);
        assert!(menu(&press(&mut app, PUSH)).editing);
        press(&mut app, MENU);
        assert!(!matches!(app.menu_view(), Effect::Menu(m) if m.editing));
        // System info opens on the push, as + opens it.
        app.button(button::SHIFT, true, 0.0);
        press(&mut app, MENU);
        app.button(button::SHIFT, false, 0.0);
        let info = go(&mut app, Item::SystemInfo);
        app.select(info);
        let fx = press(&mut app, PUSH);
        assert!(fx.contains(&Effect::Show(View::System)), "{fx:?}");
        // Off the menus, on Play, the push presses the outlined control.
        press(&mut app, MENU);
        app.show(View::Play);
        assert_eq!(press(&mut app, PUSH), vec![Effect::Nav(Nav::Push)]);
    }

    /// Owner, 22 Nov 07:17Z: after a theme change, Prefs' panel asks "Keep
    /// this theme?" for 15 s; Keep or the wheel's push keeps it, Go back,
    /// closing Prefs or the time running out puts the old one back.
    #[test]
    fn keep_this_theme_asks_for_15_seconds_then_goes_back() {
        let mut app = App::new(44_100, false);
        app.map.menu = Some(MENU);
        app.button(button::SHIFT, true, 0.0);
        press(&mut app, MENU);
        app.button(button::SHIFT, false, 0.0);
        let at = go(&mut app, Item::Appearance);
        let menu = |fx: &[Effect]| match fx.iter().rev().find(|e| matches!(e, Effect::Menu(_))) {
            Some(Effect::Menu(m)) => m.clone(),
            _ => panic!("{fx:?}"),
        };
        app.tick(100.0);
        let m = menu(&app.step_item(at, 1));
        assert_eq!(app.settings.theme, "slate");
        assert_eq!(m.help_heading, "Keep Slate?");
        assert_eq!(m.help[0], "Back to Default in 15 s.");
        assert_eq!(m.help_buttons, ["Keep", "Go back"]);
        // Not kept yet: the next start would open with Default.
        assert_eq!(app.app_settings().get_str("theme"), Some("default"));
        // It counts down once a second, on any row.
        app.tick(104.5);
        let fx = app.theme_tick();
        assert_eq!(menu(&fx).help[0], "Back to Default in 11 s.");
        assert!(
            app.theme_tick().is_empty(),
            "nothing new in the same second"
        );
        let m = menu(&app.select(0));
        assert_eq!(m.help_heading, "Keep Slate?");
        // Another step starts the 15 s again, still back to Default.
        app.select(at);
        let m = menu(&app.step_item(at, 1));
        assert_eq!(m.help_heading, "Keep Light?");
        assert_eq!(m.help[0], "Back to Default in 15 s.");
        // Time up: Default is back, and the log says so.
        app.tick(119.4);
        assert_eq!(menu(&app.theme_tick()).help[0], "Back to Default in 1 s.");
        app.tick(119.5);
        let fx = app.theme_tick();
        assert_eq!(app.settings.theme, "default");
        assert!(fx.contains(&Effect::Note(
            "theme: back to Default (Light not kept)".into()
        )));
        assert!(fx
            .iter()
            .any(|e| matches!(e, Effect::Settings(s) if s.theme == "default")));
        assert_eq!(menu(&fx).help_heading, "Appearance");
        assert!(menu(&fx).help_buttons.is_empty());
        // Keep keeps it.
        app.step_item(at, 1);
        let fx = app.help_button(0);
        assert!(fx.contains(&Effect::Note("theme: kept Slate".into())));
        assert_eq!(app.settings.theme, "slate");
        assert_eq!(app.app_settings().get_str("theme"), Some("slate"));
        app.tick(200.0);
        assert!(app.theme_tick().is_empty());
        assert_eq!(app.settings.theme, "slate");
        // Go back puts the old one back at once.
        app.step_item(at, 1);
        app.help_button(1);
        assert_eq!(app.settings.theme, "slate");
        // Stepping back to the theme from before ends the question.
        app.step_item(at, 1);
        let m = menu(&app.step_item(at, -1));
        assert_eq!(m.help_heading, "Appearance");
        // The wheel's push on Appearance keeps it.
        app.step_item(at, 1);
        app.select(at);
        app.wheel_push();
        assert_eq!(app.app_settings().get_str("theme"), Some("light"));
        // Closing Prefs puts it back.
        app.step_item(at, 1);
        assert_eq!(app.settings.theme, "console");
        let fx = press(&mut app, MENU);
        assert_eq!(app.settings.theme, "light");
        assert!(fx.contains(&Effect::Note(
            "theme: back to Light (Console not kept)".into()
        )));
    }

    /// Owner, 22 Nov 07:17Z: the Appearance panel lists the problems of the
    /// theme file in use, and names the files left out and why.
    #[test]
    fn appearance_lists_a_theme_files_problems_and_the_files_left_out() {
        let mut app = App::new(44_100, false);
        app.theme_files = vec![("sunrise.theme".into(), "Sunrise".into())];
        app.theme_problems = vec![(
            "sunrise.theme".into(),
            vec![
                "line 3: accent: expected a colour (#RRGGBB, {name}, mix(), alpha() or \
                 light_dark()), got \"blue\" (kept synthwave's #FF2E97)"
                    .into(),
            ],
        )];
        app.themes_left_out = vec![(
            "fog.theme".into(),
            "nothing could be read: text on panels (surface) 1.0:1, under the 3:1 \
             every theme must reach"
                .into(),
        )];
        app.map.menu = Some(MENU);
        app.button(button::SHIFT, true, 0.0);
        press(&mut app, MENU);
        app.button(button::SHIFT, false, 0.0);
        let at = go(&mut app, Item::Appearance);
        let help = |fx: &[Effect]| match fx.iter().rev().find(|e| matches!(e, Effect::Menu(_))) {
            Some(Effect::Menu(m)) => m.help.clone(),
            _ => panic!("{fx:?}"),
        };
        // On Default: the help, then the files left out.
        let lines = help(&app.select(at));
        let at_left = lines
            .iter()
            .position(|l| l == "Left out:")
            .expect("left out");
        assert_eq!(lines[at_left - 1], "");
        assert!(lines[at_left + 1].starts_with("fog.theme: nothing could be read:"));
        assert!(!lines.iter().any(|l| l.starts_with("Sunrise:")));
        // On Sunrise (kept): its problem too, wrapped to the panel.
        for _ in 0..7 {
            app.step_item(at, 1);
        }
        assert_eq!(app.settings.theme, "sunrise.theme");
        let lines = help(&app.help_button(0));
        let at_name = lines
            .iter()
            .position(|l| l == "Sunrise: 1 problem")
            .expect("its problems");
        assert!(lines[at_name + 1].starts_with("line 3: accent: expected a colour"));
        assert!(
            lines[at_name..]
                .iter()
                .all(|l| l.chars().count() <= HELP_CHARS),
            "{lines:?}"
        );
        assert!(lines.iter().any(|l| l.ends_with("#FF2E97)")), "{lines:?}");
        // Another row's help doesn't list them.
        let curve = go(&mut app, Item::Curve);
        assert!(!help(&app.select(curve)).iter().any(|l| l == "Left out:"));
        assert_eq!(wrap_line("a bb ccc", 4), ["a bb", "ccc"]);
    }

    #[test]
    fn appearance_offers_the_themes_folder_after_the_built_ins() {
        let mut app = App::new(44_100, false);
        app.theme_files = vec![("sunrise.theme".into(), "Sunrise".into())];
        app.map.menu = Some(MENU);
        app.button(button::SHIFT, true, 0.0);
        press(&mut app, MENU);
        app.button(button::SHIFT, false, 0.0);
        let at = go(&mut app, Item::Appearance);
        let value = |fx: &[Effect]| match fx.last() {
            Some(Effect::Menu(m)) => m.rows[at].1.clone(),
            _ => panic!("{fx:?}"),
        };
        for _ in 0..6 {
            app.step_item(at, 1);
        }
        // Past AbleTown: the theme file, by its own name, then the end.
        assert_eq!(value(&app.step_item(at, 1)), "Sunrise");
        assert_eq!(app.settings.theme, "sunrise.theme");
        assert_eq!(value(&app.step_item(at, 1)), "Sunrise");
        // Kept by its file name, and restored while the file is there.
        app.help_button(0);
        let kept = app.app_settings();
        assert_eq!(kept.get_str("theme"), Some("sunrise.theme"));
        let mut next = App::new(44_100, false);
        next.theme_files = app.theme_files.clone();
        next.restore(&kept, &mpfree_store::Settings::default());
        assert_eq!(next.settings.theme, "sunrise.theme");
        assert_eq!(next.theme_fallbacks, 0);
        // Gone from the folder (or left out as unreadable): the default
        // theme, the log says so, and the fallback is counted.
        let mut gone = App::new(44_100, false);
        let notes = gone.restore(&kept, &mpfree_store::Settings::default());
        assert_eq!(gone.settings.theme, "default");
        assert!(
            notes.iter().any(|n| n
                .starts_with("theme: the saved theme sunrise.theme can't be used now")
                && n.ends_with("so the default theme shows")),
            "{notes:?}"
        );
        assert_eq!(gone.theme_fallbacks, 1);
    }

    #[test]
    fn prefs_sound_cores_is_kept_and_used_from_the_next_start() {
        let mut app = App::new(44_100, false);
        app.map.menu = Some(MENU);
        app.button(button::SHIFT, true, 0.0);
        press(&mut app, MENU);
        app.button(button::SHIFT, false, 0.0);
        // On the Sound tab's Audio card, after Audio buffer.
        let at = go(&mut app, Item::SoundCores);
        assert_eq!(app.prefs_tab, PrefsTab::Sound);
        assert_eq!(at, row(&app, Item::Buffer) + 1);
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
        assert!(notes[0].contains(", sound cores two,"), "{notes:?}");
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

    /// Looper redesign step 4c: the Looper tab's options turn Tempo from
    /// the first loop on and off (off at first), kept between starts.
    #[test]
    fn tempo_from_the_first_loop_is_in_the_looper_options_and_kept() {
        let mut app = App::new(44_100, false);
        app.map.menu = Some(MENU);
        app.open_tab(View::Looper);
        press(&mut app, MENU);
        assert_eq!(app.view(), View::Options);
        let at = row(&app, Item::LoopTempo);
        assert_eq!(at, row(&app, Item::LoopLevel) + 1, "before Loop memory");
        assert_eq!(
            app.item_text(Item::LoopTempo),
            ("Tempo from the first loop", "Off".to_string())
        );
        assert!(!app.settings.loop_tempo);
        let fx = app.step_item(at, 1);
        assert!(app.settings.loop_tempo);
        assert!(fx
            .iter()
            .any(|e| matches!(e, Effect::Settings(s) if s.loop_tempo)));
        assert_eq!(app.item_text(Item::LoopTempo).1, "On");
        let kept = app.app_settings();
        assert_eq!(kept.get_bool("loop-tempo"), Some(true));
        let mut next = App::new(44_100, false);
        let notes = next.restore(&kept, &mpfree_store::Settings::default());
        assert!(next.settings.loop_tempo);
        assert!(
            notes[0].contains(", tempo from the first loop on,"),
            "{notes:?}"
        );
        app.step_item(at, -1);
        assert!(!app.settings.loop_tempo);
    }

    /// Smart loop end step 1: the looper's options pick Off, Suggest (at
    /// first) or Auto, under Tempo from the first loop, kept between
    /// starts.
    #[test]
    fn smart_loop_end_is_in_the_looper_options_and_kept() {
        let mut app = App::new(44_100, false);
        app.map.menu = Some(MENU);
        app.open_tab(View::Looper);
        press(&mut app, MENU);
        let at = row(&app, Item::SmartEnd);
        assert_eq!(at, row(&app, Item::LoopTempo) + 1);
        assert_eq!(
            app.item_text(Item::SmartEnd),
            ("Smart loop end", "Suggest".to_string())
        );
        app.step_item(at, 1);
        assert_eq!(app.settings.smart_end, SmartEnd::Auto);
        app.step_item(at, 1);
        assert_eq!(app.settings.smart_end, SmartEnd::Auto, "the last stays");
        let kept = app.app_settings();
        assert_eq!(kept.get_str("smart-loop-end"), Some("auto"));
        let mut next = App::new(44_100, false);
        let notes = next.restore(&kept, &mpfree_store::Settings::default());
        assert_eq!(next.settings.smart_end, SmartEnd::Auto);
        assert!(notes[0].contains(", smart loop end auto,"), "{notes:?}");
        app.step_item(at, -1);
        app.step_item(at, -1);
        assert_eq!(app.item_text(Item::SmartEnd).1, "Off");
    }

    /// Live test 2: a minute of loops was too little. Prefs (and the
    /// looper's options) pick 1 to 20 minutes, kept, from the next start.
    #[test]
    fn loop_memory_is_kept_and_used_from_the_next_start() {
        let mut app = App::new(44_100, false);
        app.map.menu = Some(MENU);
        app.button(button::SHIFT, true, 0.0);
        press(&mut app, MENU);
        app.button(button::SHIFT, false, 0.0);
        let at = go(&mut app, Item::LoopMemory);
        assert_eq!(app.prefs_tab, PrefsTab::Sound);
        let value = |fx: &[Effect]| match fx.last() {
            Some(Effect::Menu(m)) => m.rows[at].1.clone(),
            _ => panic!("{fx:?}"),
        };
        assert_eq!(app.item_text(Item::LoopMemory).1, "5 min, 131 MB");
        let fx = app.step_item(at, 1);
        assert_eq!(value(&fx), "10 min, 240 MB (next start)");
        assert!(fx.contains(&Effect::Note(
            "loop memory: 10 min from the next start (now 5 min)".into()
        )));
        assert_eq!(value(&app.step_item(at, 1)), "20 min, 458 MB (next start)");
        assert_eq!(value(&app.step_item(at, 1)), "20 min, 458 MB (next start)");
        for _ in 0..5 {
            app.step_item(at, -1);
        }
        assert_eq!(app.settings.loop_memory, LoopMemory::Min1);
        assert_eq!(
            app.item_text(Item::LoopMemory).1,
            "1 min, 44 MB (next start)"
        );
        // Kept as minutes; the next start makes it and says no more.
        let kept = app.app_settings();
        assert_eq!(kept.get_i64("loop-memory"), Some(1));
        let mut next = App::new(44_100, false);
        let notes = next.restore(&kept, &mpfree_store::Settings::default());
        assert_eq!(next.settings.loop_memory, LoopMemory::Min1);
        assert!(notes[0].contains(", loop memory 1 min,"), "{notes:?}");
        next.set_loop_memory_now(LoopMemory::Min1);
        assert_eq!(next.item_text(Item::LoopMemory).1, "1 min, 44 MB");
        // A size it doesn't offer is left out, and the default stays.
        let mut odd = mpfree_store::Settings::default();
        odd.set_i64("loop-memory", 7);
        let mut other = App::new(44_100, false);
        let notes = other.restore(&odd, &mpfree_store::Settings::default());
        assert_eq!(other.settings.loop_memory, LoopMemory::Min5);
        assert!(
            notes.iter().any(|n| n.contains("loop-memory = 7")),
            "{notes:?}"
        );
        // The looper's own options have the same row.
        let mut l = App::new(44_100, false);
        l.view = View::Looper;
        l.back = View::Looper;
        assert_eq!(l.items().last(), Some(&Item::LoopMemory));
    }

    /// Prefs "Send MIDI and CV out" (layout spec 5, Jay's card of 16 Nov
    /// 23:16Z): the Sound tab's last card, off until turned on, and it says
    /// what it does when it changes.
    #[test]
    fn prefs_send_midi_and_cv_out_is_off_until_turned_on() {
        let mut app = App::new(44_100, false);
        app.map.menu = Some(MENU);
        app.button(button::SHIFT, true, 0.0);
        press(&mut app, MENU);
        app.button(button::SHIFT, false, 0.0);
        let at = go(&mut app, Item::SendsMidi);
        assert_eq!(app.prefs_tab, PrefsTab::Sound);
        assert_eq!(app.items().last(), Some(&Item::SendsMidi), "the last card");
        assert!(!app.settings.sends_midi);
        assert_eq!(app.item_text(Item::SendsMidi).0, "Send MIDI and CV out");
        assert_eq!(app.item_text(Item::SendsMidi).1, "Off");
        let fx = app.step_item(at, 1);
        assert!(app.settings.sends_midi);
        assert_eq!(app.item_text(Item::SendsMidi).1, "On (sends MIDI)");
        assert!(fx
            .iter()
            .any(|e| matches!(e, Effect::Note(n) if n.starts_with("MIDI and CV out: on"))));
        app.step_item(at, -1);
        assert!(!app.settings.sends_midi);
    }

    /// Layout spec 4c: Prefs "Line input latency", under Line input on the
    /// Sound tab, is the looper's "Latency to make up for": one
    /// setting, which the audio tracks follow too.
    #[test]
    fn prefs_line_input_latency_is_the_loopers_latency() {
        let mut app = App::new(44_100, false);
        app.map.menu = Some(MENU);
        app.button(button::SHIFT, true, 0.0);
        press(&mut app, MENU);
        app.button(button::SHIFT, false, 0.0);
        let at = go(&mut app, Item::LoopLatency);
        assert_eq!(at, row(&app, Item::LineInput) + 1);
        assert_eq!(app.item_text(Item::LoopLatency).0, "Line input latency");
        let before = app.settings.loop_latency;
        let fx = app.step_item(at, 1);
        assert_eq!(app.settings.loop_latency, before + 16);
        assert!(app.settings.loop_latency_set);
        assert!(fx.iter().any(|e| matches!(
            e,
            Effect::Looper(LoopAction::Command(LoopCommand::SetLatency(_)))
        )));
        assert!(!app.item_text(Item::LoopLatency).1.contains("estimated"));
    }

    /// The audio tracks rethink, phase 3b: Dev's options "Audio tracks
    /// from their files", off by default, kept, from the next start.
    #[test]
    fn audio_tracks_from_their_files_is_a_dev_option_from_the_next_start() {
        let mut app = App::new(44_100, false);
        app.set_debug_mode(true);
        app.debug_page(1);
        let fx = app.button(MENU, true, 0.0);
        assert!(fx.iter().any(|e| matches!(e, Effect::Menu(_))), "{fx:?}");
        app.button(MENU, false, 0.0);
        let at = row(&app, Item::StreamAudio);
        assert_eq!(
            app.item_text(Item::StreamAudio),
            ("Audio tracks from their files", "Off".to_string())
        );
        let fx = app.step_item(at, 1);
        assert!(app.settings.stream_audio);
        assert_eq!(app.item_text(Item::StreamAudio).1, "On (next start)");
        assert!(fx.contains(&Effect::Note(
            "audio tracks: takes play from their files from the next start".into()
        )));
        // Kept; the next start turns it on and says no more.
        let kept = app.app_settings();
        assert_eq!(kept.get_bool("stream-audio"), Some(true));
        let mut next = App::new(44_100, false);
        next.restore(&kept, &mpfree_store::Settings::default());
        assert!(next.settings.stream_audio && !next.settings.audio_streams);
        next.settings.audio_streams = true;
        assert_eq!(next.item_text(Item::StreamAudio).1, "On");
        // Off again: from the next start too.
        next.settings.stream_audio = false;
        assert_eq!(next.item_text(Item::StreamAudio).1, "Off (next start)");
    }

    /// The audio tracks rethink, phase 4b: Dev's options "Loops on audio
    /// tracks", off by default, kept, from the next start.
    #[test]
    fn loops_on_audio_tracks_is_a_dev_option_from_the_next_start() {
        let mut app = App::new(44_100, false);
        app.set_debug_mode(true);
        app.debug_page(1);
        app.button(MENU, true, 0.0);
        app.button(MENU, false, 0.0);
        let at = row(&app, Item::LoopsOnTracks);
        assert_eq!(
            app.item_text(Item::LoopsOnTracks),
            ("Loops on audio tracks", "Off".to_string())
        );
        let fx = app.step_item(at, 1);
        assert!(app.settings.loops_on_tracks && !app.settings.loops_on);
        assert_eq!(app.item_text(Item::LoopsOnTracks).1, "On (next start)");
        assert!(fx.contains(&Effect::Note(
            "looper: loops play on audio tracks from the next start".into()
        )));
        let kept = app.app_settings();
        assert_eq!(kept.get_bool("loops-on-tracks"), Some(true));
        let mut next = App::new(44_100, false);
        next.restore(&kept, &mpfree_store::Settings::default());
        assert!(next.settings.loops_on_tracks && !next.settings.loops_on);
        next.settings.loops_on = true;
        assert_eq!(next.item_text(Item::LoopsOnTracks).1, "On");
        next.settings.loops_on_tracks = false;
        assert_eq!(next.item_text(Item::LoopsOnTracks).1, "Off (next start)");
    }

    /// Layout spec 4c: Prefs "Audio track memory", right after Loop
    /// memory, picks 30 s to 10 min of stereo, kept, from the next start.
    #[test]
    fn audio_track_memory_is_kept_and_used_from_the_next_start() {
        use crate::audiotracks::AudioMemory;
        let mut app = App::new(44_100, false);
        app.map.menu = Some(MENU);
        app.button(button::SHIFT, true, 0.0);
        press(&mut app, MENU);
        app.button(button::SHIFT, false, 0.0);
        let at = go(&mut app, Item::AudioMemory);
        assert_eq!(at, row(&app, Item::LoopMemory) + 1);
        let value = |fx: &[Effect]| match fx.last() {
            Some(Effect::Menu(m)) => m.rows[at].1.clone(),
            _ => panic!("{fx:?}"),
        };
        assert_eq!(app.item_text(Item::AudioMemory).0, "Audio track memory");
        assert_eq!(app.item_text(Item::AudioMemory).1, "1:00 stereo, 22 MB");
        let fx = app.step_item(at, 1);
        assert_eq!(value(&fx), "2:00 stereo, 43 MB (next start)");
        assert!(fx.contains(&Effect::Note(
            "audio track memory: 2:00 from the next start (now 1:00)".into()
        )));
        for _ in 0..6 {
            app.step_item(at, 1);
        }
        assert_eq!(app.settings.audio_memory, AudioMemory::Min10);
        for _ in 0..6 {
            app.step_item(at, -1);
        }
        assert_eq!(app.settings.audio_memory, AudioMemory::Sec30);
        // Kept as seconds; the next start makes it and says no more.
        let kept = app.app_settings();
        assert_eq!(kept.get_i64("audio-memory"), Some(30));
        let mut next = App::new(44_100, false);
        next.restore(&kept, &mpfree_store::Settings::default());
        assert_eq!(next.settings.audio_memory, AudioMemory::Sec30);
        next.set_audio_memory_now(AudioMemory::Sec30);
        assert_eq!(next.item_text(Item::AudioMemory).1, "0:30 stereo, 11 MB");
        // A size it doesn't offer is left out, and the default stays.
        let mut odd = mpfree_store::Settings::default();
        odd.set_i64("audio-memory", 45);
        let mut other = App::new(44_100, false);
        let notes = other.restore(&odd, &mpfree_store::Settings::default());
        assert_eq!(other.settings.audio_memory, AudioMemory::Min1);
        assert!(
            notes.iter().any(|n| n.contains("audio-memory = 45")),
            "{notes:?}"
        );
    }

    #[test]
    fn appearance_offers_every_built_in_theme() {
        let names: Vec<&str> = THEMES.iter().map(|t| t.0).collect();
        assert_eq!(names, mpfree_ui::theme::BUILTIN);
    }

    /// Prefs' Playing tab has the touch screen's settings on a card of
    /// their own (layout spec 5), where the section had them.
    #[test]
    fn prefs_keeps_touch_settings_on_the_playing_tabs_touch_screen_card() {
        let mut app = App::new(44_100, false);
        app.map.menu = Some(MENU);
        app.button(button::SHIFT, true, 0.0);
        press(&mut app, MENU);
        app.button(button::SHIFT, false, 0.0);
        assert_eq!(
            app.prefs_tab,
            PrefsTab::Playing,
            "a new stick opens on Playing"
        );
        let touch = |fx: &Effect| match fx {
            Effect::Menu(m) => m.prefs.as_ref().and_then(|p| {
                let card = p.cards.iter().find(|c| c.title == "Touch screen")?;
                Some(
                    card.rows
                        .iter()
                        .map(|&r| p.rows[r].name.clone())
                        .collect::<Vec<_>>(),
                )
            }),
            _ => None,
        };
        assert_eq!(
            touch(&app.menu_view()),
            Some(vec![
                "Screen pad velocity".to_string(),
                "Calibrate touch".to_string(),
                "Touch lab".to_string()
            ])
        );
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
    }

    /// Prefs' tabs (layout spec 5): SHIFT + the wheel or a tap on the
    /// sidebar picks one, stopping at the ends; the wheel alone walks the
    /// tab's rows; MENU closes Prefs, which opens again on the tab last
    /// used.
    #[test]
    fn prefs_tabs_follow_shift_and_the_wheel_and_prefs_opens_on_the_last_one() {
        let mut app = App::new(44_100, false);
        app.map.menu = Some(MENU);
        let open = |app: &mut App| {
            app.button(button::SHIFT, true, 0.0);
            let fx = press(app, MENU);
            app.button(button::SHIFT, false, 0.0);
            fx
        };
        let tab = |fx: &[Effect]| match fx.iter().rev().find(|e| matches!(e, Effect::Menu(_))) {
            Some(Effect::Menu(m)) => (m.title.clone(), m.prefs.as_ref().map(|p| p.tab)),
            _ => panic!("{fx:?}"),
        };
        assert_eq!(
            tab(&open(&mut app)),
            ("Prefs · Playing".to_string(), Some(0))
        );
        app.button(button::SHIFT, true, 0.0);
        assert_eq!(tab(&app.wheel(1)), ("Prefs · Sound".to_string(), Some(1)));
        assert_eq!(tab(&app.wheel(3)).1, Some(2), "a tab a click");
        assert_eq!(tab(&app.wheel(-1)).1, Some(1));
        assert_eq!(tab(&app.wheel(-1)).1, Some(0));
        assert!(app.wheel(-1).is_empty(), "it stops at the first");
        app.button(button::SHIFT, false, 0.0);
        // The wheel alone moves between the tab's rows.
        let fx = app.wheel(1);
        assert!(
            matches!(fx.last(), Some(Effect::Menu(m)) if m.selected == 1 && m.title == "Prefs · Playing"),
            "{fx:?}"
        );
        // A tap on the sidebar, the wheel back on the tab's first row.
        let system = PrefsTab::System.index();
        let fx = app.pick_prefs_tab(system);
        assert!(
            matches!(fx.last(), Some(Effect::Menu(m)) if m.title == "Prefs · System" && m.selected == 0),
            "{fx:?}"
        );
        assert!(app.pick_prefs_tab(system).is_empty(), "the tab showing");
        assert!(
            app.pick_prefs_tab(PrefsTab::ALL.len()).is_empty(),
            "no such tab"
        );
        // MENU closes Prefs, and it opens again on System.
        assert!(press(&mut app, MENU).contains(&Effect::Show(View::Play)));
        assert_eq!(app.view(), View::Play);
        assert!(app.pick_prefs_tab(0).is_empty(), "Prefs closed");
        assert_eq!(tab(&open(&mut app)).1, Some(system));
    }

    /// The Appearance tab: a tap on a theme tries it, the wheel alone steps
    /// through them, and leaving the tab while "Keep this theme?" asks puts
    /// the old theme back, as closing Prefs does.
    #[test]
    fn leaving_the_appearance_tab_puts_the_old_theme_back() {
        let mut app = App::new(44_100, false);
        app.open_tab(View::Prefs);
        let at = go(&mut app, Item::Appearance);
        let light = app.theme_choices().position(|t| t.0 == "light").unwrap();
        let fx = app.choose(at, light);
        assert_eq!(app.settings.theme, "light");
        let trial = |fx: &[Effect]| match fx.iter().rev().find(|e| matches!(e, Effect::Menu(_))) {
            Some(Effect::Menu(m)) => m
                .prefs
                .as_ref()
                .and_then(|p| p.gallery.as_ref())
                .and_then(|g| g.trial.clone()),
            _ => panic!("{fx:?}"),
        };
        assert_eq!(trial(&fx).map(|t| t.0), Some("Keep Light?".to_string()));
        assert!(
            app.choose(at, light)
                .iter()
                .all(|e| matches!(e, Effect::Menu(_))),
            "in use"
        );
        // The wheel alone steps through the themes, still back to Default.
        app.wheel(-1);
        assert_eq!(app.settings.theme, "slate");
        let fx = app.pick_prefs_tab(0);
        assert_eq!(app.settings.theme, "default");
        assert!(fx.contains(&Effect::Note(
            "theme: back to Default (Slate not kept)".into()
        )));
        // With no theme on trial, the push on the tab opens Customise on
        // a copy of the theme in use, drawn on the whole screen.
        go(&mut app, Item::Appearance);
        let fx = app.wheel_push();
        assert!(
            matches!(fx.last(), Some(Effect::Menu(m)) if m.title == "Prefs · Appearance · Customise"),
            "{fx:?}"
        );
        assert!(fx
            .iter()
            .any(|e| matches!(e, Effect::LiveTheme(Some(t)) if t.name == "My theme")));
    }

    /// A tap on a choice or a tile sets it at once, telling the rest of the
    /// app once; the slider and the knob set their values.
    #[test]
    fn prefs_choices_sliders_and_knobs_set_their_settings() {
        let mut app = App::new(44_100, false);
        app.open_tab(View::Prefs);
        let latch = go(&mut app, Item::LatchMode);
        let fx = app.choose(latch, 2);
        assert_eq!(app.settings.latch_mode, LatchMode::Add);
        let count = |fx: &[Effect], f: fn(&Effect) -> bool| fx.iter().filter(|e| f(e)).count();
        assert_eq!(count(&fx, |e| matches!(e, Effect::Settings(_))), 1);
        assert_eq!(count(&fx, |e| matches!(e, Effect::Menu(_))), 1);
        assert_eq!(
            app.choose(latch, 2),
            vec![app.menu_view()],
            "already chosen"
        );
        app.choose(latch, 0);
        assert_eq!(app.settings.latch_mode, LatchMode::Replace);
        // Screen pad velocity's slider: 10 to 127.
        let screen = go(&mut app, Item::ScreenVelocity);
        app.set_value(screen, 1000);
        assert_eq!(app.settings.screen_velocity, 127);
        app.set_value(screen, 0);
        assert_eq!(app.settings.screen_velocity, 10);
        assert!(
            app.set_value(screen, 0).is_empty(),
            "no change, nothing said"
        );
        // Line input latency's knob, in 16-frame steps, tells the looper.
        let latency = go(&mut app, Item::LoopLatency);
        let fx = app.set_value(latency, 500);
        assert_eq!(app.settings.loop_latency, 1024);
        assert!(fx.iter().any(|e| matches!(e, Effect::Looper(_))), "{fx:?}");
        // Off Prefs, nothing.
        app.open_tab(View::Play);
        assert!(app.choose(0, 1).is_empty());
        assert!(app.set_value(0, 10).is_empty());
    }

    /// Prefs > System > Backup and Akai kits (Jay, 17 Nov 18:12Z): each
    /// row asks for its step at the next start, or takes the ask back.
    #[test]
    fn prefs_offers_the_backup_and_akais_kits_for_the_next_start() {
        use crate::setup::{Choice, PrefsState};
        let mut app = App::new(44_100, false);
        app.map.menu = Some(MENU);
        app.button(button::SHIFT, true, 0.0);
        press(&mut app, MENU);
        app.button(button::SHIFT, false, 0.0);
        let backup = go(&mut app, Item::SetupBackup);
        let akai = row(&app, Item::SetupAkaiKits);
        assert_eq!(app.prefs_tab, PrefsTab::System);
        assert_eq!(akai, backup + 1, "one card");
        let fx = app.select(backup);
        assert!(
            matches!(&fx[..], [Effect::Menu(m)] if m.title == "Prefs · System" && m.selected == backup
                         && m.rows[backup] == ("Backup of your MPC".to_string(), "Not on this stick".to_string())
                         && m.rows[akai] == ("Akai's kits and samples".to_string(), "Not on this stick".to_string())),
            "{fx:?}"
        );
        // Without a creator's stick it says so and changes nothing.
        assert!(matches!(&app.step_item(backup, 1)[..], [Effect::Tell(_)]));
        // Both left out at the first start.
        let left = Choice {
            left_out: true,
            done: false,
            next_start: false,
        };
        app.set_setup(Some(PrefsState {
            backup: left.clone(),
            akai: left.clone(),
            backup_made: None,
        }));
        assert_eq!(app.item_text(Item::SetupBackup).1, "Left out");
        assert_eq!(app.item_text(Item::SetupAkaiKits).1, "Left out");
        assert_eq!(
            app.step_item(akai, 1),
            vec![Effect::SetupNextStart {
                backup: false,
                want: true
            }]
        );
        assert_eq!(
            app.step_item(backup, -1),
            vec![],
            "- takes back only what was asked"
        );
        // Asked: the row says so, and the push takes it back.
        let asked = Choice {
            left_out: false,
            done: false,
            next_start: true,
        };
        let fx = app.set_setup(Some(PrefsState {
            backup: left.clone(),
            akai: asked,
            backup_made: None,
        }));
        assert!(
            matches!(&fx[..], [Effect::Menu(m)] if m.rows[akai].1 == "At the next start"),
            "{fx:?}"
        );
        assert_eq!(
            app.step_item(akai, 1),
            vec![Effect::SetupNextStart {
                backup: false,
                want: false
            }]
        );
        // Done: a backup made, the kits in.
        let done = Choice {
            left_out: false,
            done: true,
            next_start: false,
        };
        app.set_setup(Some(PrefsState {
            backup: done.clone(),
            akai: done.clone(),
            backup_made: Some("17 Nov 2026".into()),
        }));
        assert_eq!(app.item_text(Item::SetupBackup).1, "Made 17 Nov 2026");
        assert_eq!(app.item_text(Item::SetupAkaiKits).1, "In BROWSE");
        assert!(matches!(&app.step_item(akai, 1)[..], [Effect::Tell(_)]));
        assert_eq!(
            app.step_item(backup, 1),
            vec![Effect::SetupNextStart {
                backup: true,
                want: true
            }],
            "a new backup, beside the one there"
        );
        app.set_setup(Some(PrefsState {
            backup: Choice {
                next_start: true,
                done: false,
                left_out: false,
            },
            akai: done,
            backup_made: Some("17 Nov 2026".into()),
        }));
        assert_eq!(
            app.item_text(Item::SetupBackup).1,
            "New one at the next start"
        );
    }

    /// Live test 2 (Jay, 13 Nov): on a page with rows the wheel scrolls
    /// through them, - / + cycle the highlighted row's options, and the
    /// wheel's push makes the wheel cycle them until the next push. The
    /// same on the Sample tab, the Pads tab, the Synth tab and Arrange;
    /// SHIFT keeps each page's second job.
    #[test]
    fn the_wheel_picks_rows_and_its_push_opens_one_on_every_page_with_rows() {
        let mut app = ready();
        let pick = |d| vec![Effect::Rows(RowKey::Pick(d))];
        let step = |d| vec![Effect::Rows(RowKey::Step(d))];
        for v in [View::Sample, View::Synth, View::Arrange, View::Pads] {
            app.open_tab(v);
            assert_eq!(app.view(), v);
            assert_eq!(app.wheel(1), pick(1), "{v:?}");
            assert_eq!(app.wheel(-3), pick(-3), "{v:?}");
            assert_eq!(press(&mut app, button::PLUS), step(1), "{v:?}");
            assert_eq!(press(&mut app, button::MINUS), step(-1), "{v:?}");
            // The push opens the row: the wheel changes it, - / + still do.
            assert_eq!(
                press(&mut app, button::DATA_PUSH),
                vec![Effect::Rows(RowKey::Editing(true))]
            );
            assert_eq!(app.wheel(2), step(2), "{v:?}");
            assert_eq!(press(&mut app, button::MINUS), step(-1), "{v:?}");
            // The next push gives the wheel back to the rows.
            assert_eq!(
                press(&mut app, button::DATA_PUSH),
                vec![Effect::Rows(RowKey::Editing(false))]
            );
            assert_eq!(app.wheel(1), pick(1), "{v:?}");
        }
        // SHIFT keeps the second jobs: pads, pages, scrolling, zoom.
        app.open_tab(View::Sample);
        app.button(button::SHIFT, true, 0.0);
        assert_eq!(app.wheel(2), vec![Effect::SamplePad(1)]);
        assert_eq!(press(&mut app, button::PLUS), vec![Effect::SampleWheel(1)]);
        app.button(button::SHIFT, false, 0.0);
        // A row left open closes with its page.
        press(&mut app, button::DATA_PUSH);
        let fx = app.open_tab(View::Play);
        assert!(fx.contains(&Effect::Rows(RowKey::Editing(false))), "{fx:?}");
        assert_eq!(app.wheel(1), vec![Effect::Nav(Nav::Turn(1))]);
        app.open_tab(View::Sample);
        assert_eq!(app.wheel(1), pick(1));
        // A menu's own rows are as before.
        press(&mut app, MENU);
        assert!(matches!(app.wheel(1).last(), Some(Effect::Menu(_))));
    }

    #[test]
    fn the_chain_editor_hands_the_wheel_and_its_push_to_the_gui() {
        let mut app = ready();
        app.open_chain();
        assert_eq!(app.view(), View::Chain);
        // The screen gives them to the picker while it is open, else the
        // wheel is Q-Link 1 (gui.rs).
        assert_eq!(app.wheel(2), vec![Effect::ChainWheel(2)]);
        assert_eq!(app.wheel_push(), vec![Effect::ChainPush]);
    }

    #[test]
    fn the_synth_tab_takes_the_wheel_and_shift_wheel_moves_octaves_on_play() {
        let mut app = ready();
        // SHIFT + wheel on Play: octaves (main.rs moves the kit when drums play).
        app.button(button::SHIFT, true, 0.0);
        assert_eq!(app.wheel(3), vec![Effect::Octave(1)]);
        app.button(button::SHIFT, false, 0.0);
        assert_eq!(app.wheel(-2), vec![Effect::Nav(Nav::Turn(-2))]);
        assert_eq!(app.open_tab(View::Synth), vec![Effect::Show(View::Synth)]);
        assert_eq!(app.view(), View::Synth);
        // The wheel picks tiles (RowKey; see the wheel navigation test).
        assert_eq!(app.wheel(2), vec![Effect::Rows(RowKey::Pick(2))]);
        app.button(button::SHIFT, true, 0.0);
        assert_eq!(app.wheel(-4), vec![Effect::SynthPage(-1)]);
        // SHIFT + - and + step pages.
        assert_eq!(press(&mut app, button::PLUS), vec![Effect::SynthPage(1)]);
        assert_eq!(press(&mut app, button::MINUS), vec![Effect::SynthPage(-1)]);
        app.button(button::SHIFT, false, 0.0);
        assert_eq!(
            press(&mut app, button::PLUS),
            vec![Effect::Rows(RowKey::Step(1))]
        );
        // MENU on the Synth page opens its options and goes back to it.
        let fx = press(&mut app, MENU);
        assert!(matches!(&fx[0], Effect::Menu(m) if m.title == "Synth options"));
        press(&mut app, MENU);
        assert_eq!(app.view(), View::Synth);
        assert_eq!(app.tab(0), vec![Effect::Show(View::Play)]);
    }

    /// Tunings step T5 in the app: the Synth tab's Tuning rows set the
    /// song's tuning; the shared synth plays its own when it has one; an
    /// instrument's own goes with its id, and a project's tunings part
    /// gives them back, an instrument the list hasn't got said.
    #[test]
    fn each_instrument_has_its_own_tuning_or_the_songs() {
        use crate::instrument_list::{InstrumentId, InstrumentList};
        use crate::synth::{Builtin, TuningPick};
        let mut app = ready();
        let mut list = InstrumentList::new();
        list.follow_kits(3);
        list.follow(0b10);
        app.set_instrument_list(&list);
        let (shared, kit2, track2) = (
            InstrumentId::SHARED_SYNTH,
            list.of_kit(1).unwrap(),
            list.own_of(1).unwrap(),
        );
        let synth_tuning = |fx: &[Effect]| {
            fx.iter().find_map(|e| match e {
                Effect::SynthPads(p) => Some(p.tuning),
                _ => None,
            })
        };
        let meantone = TuningPick {
            builtin: Builtin::Meantone,
            a4_hz: 415,
            ..TuningPick::default()
        };
        // The shared synth in its own meantone: the synth plays it, the
        // song stays 12-TET.
        let fx = app.set_own_tuning(shared, Some(meantone));
        assert_eq!(synth_tuning(&fx), Some(meantone));
        assert!(fx.contains(&Effect::Note(
            "tunings: the shared synth plays its own tuning, Meantone 1/4 comma on C, A4 415 Hz"
                .to_string()
        )));
        assert_eq!(app.tunings().song(), TuningPick::default());
        assert_eq!(app.synth_settings().get_str("tuning"), Some("12-tet"));
        // The song's Tuning row (the Tunings page): the song goes to just,
        // said to the synth only as a change of the options (the shared
        // synth keeps its own).
        open_tunings_page(&mut app);
        let fx = app.step_item(1, 1);
        assert_eq!(synth_tuning(&fx), Some(meantone));
        assert_eq!(app.tunings().song().builtin, Builtin::Just);
        assert_eq!(app.synth_settings().get_str("tuning"), Some("just"));
        // Back on Song's: the synth plays just.
        let fx = app.set_own_tuning(shared, None);
        assert_eq!(synth_tuning(&fx).map(|t| t.builtin), Some(Builtin::Just));
        assert!(app.set_own_tuning(shared, None).is_empty());
        // An instrument the list hasn't got: nothing.
        let none = InstrumentId::from_number(99).unwrap();
        assert!(app.set_own_tuning(none, Some(meantone)).is_empty());
        assert_eq!(app.tunings().owned(), 0);

        // Kit 2 and Track 2's own in tunings of their own, saved.
        app.set_own_tuning(kit2, Some(meantone));
        app.set_own_tuning(
            track2,
            Some(TuningPick {
                builtin: Builtin::Equal24,
                ..TuningPick::default()
            }),
        );
        let part = app.tunings_part();
        assert_eq!(part.lines().count(), 3, "{part}");
        // Track 2's instrument taken out: its tuning goes.
        list.follow(0);
        app.set_instrument_list(&list);
        assert_eq!(app.tunings().own(track2), None);
        assert_eq!(app.tunings().own(kit2), Some(meantone));

        // Opened in a new app: the synth part's song, the part's own. The
        // instrument the list hasn't got any more is said and left out.
        let mut next = ready();
        let (_, _) = next.open_synth(&app.synth_settings());
        next.set_instrument_list(&list);
        let (notes, fx) = next.open_tunings(Some(&part));
        assert_eq!(next.tunings().song().builtin, Builtin::Just);
        assert_eq!(next.tunings().own(kit2), Some(meantone));
        assert_eq!(next.tunings().own(track2), None);
        assert_eq!(synth_tuning(&fx).map(|t| t.builtin), Some(Builtin::Just));
        assert_eq!(
            notes,
            [
                format!("tunings: left out the tuning of instrument {track2}, not in the project's list"),
                "tunings: the song plays Just (5-limit) on C; with their own: Kit 2 Meantone 1/4 comma on C, A4 415 Hz"
                    .to_string(),
            ]
        );
        // A new project's synth part: every instrument on Song's again.
        next.open_synth(&Default::default());
        let (notes, _) = next.open_tunings(None);
        assert!(notes.is_empty());
        assert_eq!(next.tunings(), &crate::tunings::Tunings::new());
        // A part that doesn't read is said, and changes nothing.
        let (notes, _) = next.open_tunings(Some("mpfree-tunings 1\nsong root C a4 440 just\n"));
        assert_eq!(notes.len(), 1);
        assert!(
            notes[0].starts_with("tunings: its tunings can't be read"),
            "{notes:?}"
        );
        assert_eq!(next.tunings().owned(), 0);
    }

    /// Duplicate copies the tuning (tunings step T5): a copy of Track 2's
    /// own takes its tuning once the list gives it its id, and a copy into
    /// a track that has its own already takes it at once.
    #[test]
    fn a_copy_takes_the_tuning_of_the_instrument_it_copies() {
        use crate::instrument_list::InstrumentList;
        use crate::synth::{Builtin, TuningPick};
        let mut app = ready();
        let mut list = InstrumentList::new();
        list.follow_kits(1);
        list.follow(0b10);
        app.set_instrument_list(&list);
        let track2 = list.own_of(1).unwrap();
        let poly = crate::instruments::Held {
            id: "mpfree.poly",
            name: "Poly",
            instance: 0,
        };
        app.set_track_sound(1, Some(poly));
        let meantone = TuningPick {
            builtin: Builtin::Meantone,
            ..TuningPick::default()
        };
        app.set_own_tuning(track2, Some(meantone));
        // Track 3 takes a copy: no id yet, so it waits.
        app.track = 2;
        app.copy_track_sound(1);
        assert_eq!(app.tunings().owned(), 1);
        list.follow(0b110);
        app.set_instrument_list(&list);
        let track3 = list.own_of(2).unwrap();
        assert_eq!(app.tunings().own(track3), Some(meantone));
        // Track 4 has its own already (on Song's): the copy's tuning at once.
        list.follow(0b1110);
        app.set_instrument_list(&list);
        let track4 = list.own_of(3).unwrap();
        app.track = 3;
        app.copy_track_sound(1);
        assert_eq!(app.tunings().own(track4), Some(meantone));
        // A copy of one on Song's puts the copy on Song's.
        app.set_own_tuning(track2, None);
        app.copy_track_sound(1);
        assert_eq!(app.tunings().own(track4), None);
    }

    /// The Synth tab's options open the Tunings page from the Tuning row's
    /// help (tunings step T6): its song's Tuning, Tuning root and A4 rows
    /// come first, under their heading.
    fn open_tunings_page(app: &mut App) -> MenuView {
        app.open_tab(View::Synth);
        press(app, MENU);
        let at = app.row_of(Item::Synth(SynthItem::Tuning)).unwrap();
        app.select(at);
        let fx = app.help_button(mpfree_ui::widgets::CARD_LINK);
        let Some(Effect::Menu(m)) = fx.last() else {
            panic!("{fx:?}")
        };
        assert_eq!(m.title, "Tunings");
        assert_eq!(
            m.rows[..4].iter().map(|r| r.0.as_str()).collect::<Vec<_>>(),
            ["The song's tuning", "Tuning", "Tuning root", "A4"]
        );
        assert!(m.marks[0].heading && !m.marks[1].heading);
        assert_eq!(m.selected, 1);
        m.clone()
    }

    /// The Tunings page's rows of the song's tuning (the Synth tab's
    /// before tunings step T6).
    #[derive(Clone, Copy)]
    enum SongRow {
        Tuning = 1,
        TuningRoot,
        A4,
    }

    #[test]
    fn the_tunings_page_picks_the_songs_tuning_its_root_and_a4() {
        use crate::synth::{Builtin, A4_MAX, A4_MIN};
        let mut app = ready();
        open_tunings_page(&mut app);
        let row = |a: SongRow| a as usize;
        let tuning = |fx: &[Effect]| {
            fx.iter().find_map(|e| match e {
                Effect::SynthPads(p) => Some(p.tuning),
                _ => None,
            })
        };
        // Every built-in tuning in turn, and no further.
        for &want in &Builtin::ALL[1..] {
            let t = tuning(&app.step_item(row(SongRow::Tuning), 1)).unwrap();
            assert_eq!(t.builtin, want);
        }
        assert_eq!(tuning(&app.step_item(row(SongRow::Tuning), 1)), None);
        for _ in 0..3 {
            app.step_item(row(SongRow::Tuning), -1);
        }
        assert_eq!(app.synth_pads.tuning.builtin, Builtin::Just);
        // Root D: the row reads it, now that just intonation uses one.
        app.step_item(row(SongRow::TuningRoot), 1);
        let fx = app.step_item(row(SongRow::TuningRoot), 1);
        assert_eq!(tuning(&fx).unwrap().root, 2);
        let Some(Effect::Menu(m)) = fx.last() else {
            panic!("{fx:?}")
        };
        let at = row(SongRow::Tuning);
        assert_eq!(
            (m.rows[at].1.as_str(), m.rows[at + 1].1.as_str()),
            ("Just (5-limit)", "D")
        );
        // A4 down to 432 Hz, and it stops at the ends.
        for _ in 0..8 {
            app.step_item(row(SongRow::A4), -1);
        }
        assert_eq!(app.synth_pads.tuning.a4_hz, 432);
        for _ in 0..200 {
            app.step_item(row(SongRow::A4), 1);
        }
        assert_eq!(app.synth_pads.tuning.a4_hz, A4_MAX);
        assert_eq!(tuning(&app.step_item(row(SongRow::A4), 1)), None);
        for _ in 0..200 {
            app.step_item(row(SongRow::A4), -1);
        }
        assert_eq!(app.synth_pads.tuning.a4_hz, A4_MIN);
        // The root stays at B and C; the engine and scale are untouched.
        for _ in 0..20 {
            app.step_item(row(SongRow::TuningRoot), 1);
        }
        assert_eq!(app.synth_pads.tuning.root, 11);
        assert_eq!(app.synth_pads.engine, crate::synth::Engine::Poly);
        assert_eq!(app.synth_pads.scale, None);
    }

    #[test]
    fn the_tunings_page_offers_tuning_files_after_the_built_in_tunings() {
        use crate::synth::Builtin;
        let mut app = ready();
        app.tuning_files = vec![
            ("werckmeister3".to_string(), false),
            ("bohlen-pierce-with-a-long-name".to_string(), true),
        ];
        open_tunings_page(&mut app);
        let at = SongRow::Tuning as usize;
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

    /// The last menu in `fx`.
    fn last_menu(fx: &[Effect]) -> MenuView {
        match fx.iter().rev().find(|e| matches!(e, Effect::Menu(_))) {
            Some(Effect::Menu(m)) => m.clone(),
            _ => panic!("no menu: {fx:?}"),
        }
    }

    /// Tunings step T6, the Synth tab's Tuning row: it is the instrument's
    /// own. Song's first ("Song's: 12-TET"), then its own, tagged OWN with
    /// its own root and A4 under it; back to Song's takes them away. The
    /// help says what it and the song play in, with the Tunings button;
    /// the song and the shared synth are untouched.
    #[test]
    fn the_synth_tuning_row_gives_the_instrument_its_own_tuning() {
        use crate::instrument_list::{InstrumentId, InstrumentList};
        use crate::synth::{Builtin, TuningPick};
        let mut app = ready();
        let mut list = InstrumentList::new();
        list.follow_kits(1);
        list.follow(0b100);
        app.set_instrument_list(&list);
        let fm = crate::instruments::Held {
            id: "mpfree.fm",
            name: "FM",
            instance: 0,
        };
        app.set_track_sound(2, Some(fm));
        let track3 = list.own_of(2).unwrap();
        app.track = 2;
        app.open_tab(View::Synth);
        press(&mut app, MENU);
        let at = app.row_of(Item::Synth(SynthItem::Tuning)).unwrap();
        let m = last_menu(&app.select(at));
        assert_eq!(m.rows.len(), synth_pads::SYNTH_OPTIONS_SONGS.len());
        assert_eq!(
            m.rows[at],
            ("Tuning".to_string(), "Song's: 12-TET".to_string())
        );
        assert_eq!(m.marks[at], RowMark::default());
        assert_eq!(
            m.facts,
            [
                "FM · Track 3 plays the song's tuning",
                "The song: 12-TET, A4 = 440 Hz"
            ]
        );
        assert!(!m.facts_warn);
        assert_eq!(m.help_link, Some(tunings_page::link()));
        assert!(m.help[0].starts_with("Song's: this instrument plays"));

        // Its own: 12-TET first, tagged, its root and A4 under it.
        let fx = app.step_item(at, 1);
        assert_eq!(app.tunings().own(track3), Some(TuningPick::default()));
        assert!(fx.contains(&Effect::Note(
            "tunings: Track 3's own instrument plays its own tuning, 12-TET".to_string()
        )));
        let m = last_menu(&fx);
        assert_eq!(m.rows.len(), synth_pads::SYNTH_OPTIONS.len());
        assert_eq!(m.rows[at].1, "12-TET");
        assert_eq!(m.marks[at].tag, "own");
        assert_eq!(
            (m.rows[at + 1].clone(), m.rows[at + 2].clone()),
            (
                ("Tuning root".to_string(), "C (not used)".to_string()),
                ("A4".to_string(), "440 Hz".to_string())
            )
        );
        // Just on D at 432 Hz, its own.
        app.step_item(at, 1);
        app.step_item(at + 1, 1);
        app.step_item(at + 1, 1);
        for _ in 0..8 {
            app.step_item(at + 2, -1);
        }
        let just = TuningPick {
            builtin: Builtin::Just,
            root: 2,
            a4_hz: 432,
            ..TuningPick::default()
        };
        assert_eq!(app.tunings().own(track3), Some(just));
        assert_eq!(app.tunings().song(), TuningPick::default());
        assert_eq!(app.tunings().own(InstrumentId::SHARED_SYNTH), None);
        let m = last_menu(&app.select(at));
        assert_eq!(
            m.facts.join(" "),
            "FM · Track 3 plays its own tuning: Just (5-limit) on D, A4 = 432 Hz The song: 12-TET, A4 = 440 Hz"
        );
        assert!(m.help[0].starts_with("Its own tuning stays"));
        assert!(app.take_project_edit());
        // Back to Song's: the rows go.
        let fx = app.step_item(at, -1);
        assert_eq!(
            app.tunings().own(track3).map(|p| p.builtin),
            Some(Builtin::Equal12)
        );
        assert_eq!(last_menu(&fx).rows.len(), synth_pads::SYNTH_OPTIONS.len());
        let fx = app.step_item(at, -1);
        assert_eq!(app.tunings().own(track3), None);
        assert!(fx.contains(&Effect::Note(
            "tunings: Track 3's own instrument plays the song's tuning, 12-TET".to_string()
        )));
        let m = last_menu(&fx);
        assert_eq!(m.rows.len(), synth_pads::SYNTH_OPTIONS_SONGS.len());
        assert_eq!(m.rows[at].1, "Song's: 12-TET");
        let fx = app.step_item(at, -1);
        assert!(
            !fx.iter().any(|e| matches!(e, Effect::Note(_))),
            "no further"
        );
    }

    /// A plugin that takes only MIDI notes (tunings step T6): its Tuning
    /// row says "12-TET (can't follow)" in amber and can't be stepped (a
    /// step or a push says why), and the Tunings page lists it so.
    #[test]
    fn an_instrument_that_cant_follow_says_so_and_cant_be_stepped() {
        use crate::instrument_list::InstrumentList;
        let mut app = ready();
        let mut list = InstrumentList::new();
        list.follow_kits(1);
        list.follow(1 << 5);
        app.set_instrument_list(&list);
        let bassline = crate::instruments::Held {
            id: "clap.bassline",
            name: "Bassline",
            instance: 0,
        };
        app.set_track_sound(5, Some(bassline));
        let track6 = list.own_of(5).unwrap();
        app.track = 5;
        app.open_tab(View::Synth);
        press(&mut app, MENU);
        let at = app.row_of(Item::Synth(SynthItem::Tuning)).unwrap();
        app.select(at);
        let fx = app.set_tuning_facts((1 << 5, 0), [None; mpfree_seq::MAX_TRACKS]);
        let m = last_menu(&fx);
        assert_eq!(m.rows[at].1, "12-TET (can't follow)");
        assert!(m.marks[at].fixed && m.marks[at].tag.is_empty());
        assert!(m.facts_warn);
        assert_eq!(m.facts[0], "Bassline takes MIDI notes only, so it plays");
        assert!(app
            .set_tuning_facts((1 << 5, 0), [None; mpfree_seq::MAX_TRACKS])
            .is_empty());
        let says = |fx: &[Effect]| {
            fx.iter().any(
                |e| matches!(e, Effect::Tell(t) if t.starts_with("Bassline takes MIDI notes only")),
            )
        };
        assert!(says(&app.step_item(at, 1)));
        assert!(says(&press(&mut app, button::DATA_PUSH)));
        assert!(!app.menu_editing, "a push opens nothing to change");
        assert_eq!(app.tunings().own(track6), None);
        // The Tunings page: the same.
        let fx = app.help_button(mpfree_ui::widgets::CARD_LINK);
        let m = last_menu(&fx);
        let k = m
            .rows
            .iter()
            .position(|r| r.0 == "Bassline · Track 6")
            .unwrap();
        assert_eq!(m.rows[k].1, "12-TET (can't follow)");
        assert!(m.marks[k].fixed);
        assert_eq!(m.marks[k].sub, "plugin, takes MIDI only");
        assert!(says(&app.step_item(k, 1)));
    }

    /// The Tunings page (tunings step T6): the song's rows, then every
    /// instrument with what it plays in (the kits the tracks play, the
    /// shared synth, the tracks' own); a kit's own root and A4 under its
    /// row; the wheel passes over the headings; a synth's "Open its
    /// options" arms its track on the Synth options' Tuning row; Back and
    /// MENU go back there.
    #[test]
    fn the_tunings_page_lists_every_instrument_and_goes_back() {
        use crate::instrument_list::InstrumentList;
        use crate::synth::{Builtin, TuningPick};
        let mut app = ready();
        let mut list = InstrumentList::new();
        list.follow_kits(2);
        list.follow(0b100);
        app.set_instrument_list(&list);
        app.set_kit_names(vec!["Classic House".to_string(), String::new()]);
        let fm = crate::instruments::Held {
            id: "mpfree.fm",
            name: "FM",
            instance: 0,
        };
        app.set_track_sound(2, Some(fm));
        let mut sends = [None; mpfree_seq::MAX_TRACKS];
        sends[3] = Some(2);
        app.set_track_sends(&sends);
        let mut kit_of = [None; mpfree_seq::MAX_TRACKS];
        kit_of[0] = Some(0);
        app.set_tuning_facts((0, 0), kit_of);
        let (kit1, track3) = (list.of_kit(0).unwrap(), list.own_of(2).unwrap());
        let m = open_tunings_page(&mut app);
        assert_eq!(m.rows[4].0, "Instruments · what each plays in");
        assert!(m.marks[4].heading);
        assert_eq!(
            m.rows[5],
            ("Classic House · kit".to_string(), "Song's".to_string())
        );
        assert_eq!(m.marks[5].sub, "Track 1 · keygroups, Notes and 16 Levels");
        assert!(m.rows[6].0.starts_with("Poly · "), "{:?}", m.rows[6]);
        assert_eq!(m.marks[6].sub, "on the pads too");
        assert_eq!(
            m.rows[7],
            ("FM · Tracks 3 and 4".to_string(), "Song's".to_string())
        );
        assert_eq!(m.rows.len(), 8, "Kit 2 plays nowhere: not listed");
        assert!(m.help.last().unwrap().starts_with("Gear on MIDI out"));
        // FM in its own just intonation.
        app.step_item(7, 1);
        app.step_item(7, 1);
        let m = last_menu(&app.select(7));
        assert_eq!(
            app.tunings().own(track3).map(|p| p.builtin),
            Some(Builtin::Just)
        );
        assert_eq!(m.rows[7].1, "Just (5-limit) on C");
        assert_eq!(m.marks[7].sub, "its own");
        assert_eq!(m.help_heading, "FM · Tracks 3 and 4");
        assert_eq!(m.facts, ["Its own: Just (5-limit) on C, A4 = 440 Hz"]);
        assert_eq!(m.help_link.as_ref().unwrap().0, "Open its options");
        // The kit in its own 12-TET: its root and A4 come under it.
        let fx = app.step_item(5, 1);
        assert_eq!(app.tunings().own(kit1), Some(TuningPick::default()));
        let m = last_menu(&fx);
        assert_eq!(m.rows.len(), 10);
        assert_eq!(
            (m.rows[6].clone(), m.marks[6].sub.clone()),
            (
                ("Tuning root".to_string(), "C (not used)".to_string()),
                "Classic House's own".to_string()
            )
        );
        assert_eq!(m.rows[7].0, "A4");
        app.step_item(6, 1);
        app.step_item(7, -1);
        assert_eq!(
            app.tunings().own(kit1).map(|p| (p.root, p.a4_hz)),
            Some((1, 439))
        );
        assert!(app.select(5).iter().any(|e| matches!(e, Effect::Menu(_))));
        assert!(
            last_menu(&app.select(5)).help_link.is_none(),
            "a kit has no options to open"
        );
        // The wheel passes over the headings and stops at the ends.
        app.select(3);
        assert_eq!(last_menu(&app.wheel(1)).selected, 5);
        assert_eq!(last_menu(&app.wheel(-1)).selected, 3);
        app.select(1);
        assert_eq!(last_menu(&app.wheel(-1)).selected, 1);
        assert!(app.select(0).is_empty(), "a heading is no row to pick");
        // Back and MENU: the Synth options, on the Tuning row.
        let fx = app.options_button();
        let m = last_menu(&fx);
        assert_eq!(m.title, "Synth options");
        assert_eq!(
            m.selected,
            app.row_of(Item::Synth(SynthItem::Tuning)).unwrap()
        );
        assert_eq!(app.view(), View::Options);
        app.help_button(mpfree_ui::widgets::CARD_LINK);
        let m = last_menu(&press(&mut app, MENU));
        assert_eq!(m.title, "Synth options");
        // FM's options: Track 3 armed, its own Tuning row.
        app.help_button(mpfree_ui::widgets::CARD_LINK);
        let k = 9;
        assert_eq!(last_menu(&app.select(k)).rows[k].0, "FM · Tracks 3 and 4");
        let fx = app.help_button(mpfree_ui::widgets::CARD_LINK);
        assert_eq!(app.track, 2);
        let m = last_menu(&fx);
        assert_eq!(m.title, "Synth options");
        let at = app.row_of(Item::Synth(SynthItem::Tuning)).unwrap();
        assert_eq!((m.selected, m.rows[at].1.as_str()), (at, "Just (5-limit)"));
        assert_eq!(m.marks[at].tag, "own");
    }

    /// Tunings step T6, board 4 (layout spec 4e): the Pads tab's Edit pad
    /// steps a kit's own tuning as its row on the Tunings page does, and
    /// each kit's is named for its Tuning and its pad line.
    #[test]
    fn the_pads_tab_steps_a_kits_own_tuning_and_names_each_kits() {
        use crate::instrument_list::InstrumentList;
        use crate::synth::{Builtin, TuningPick};
        let mut app = ready();
        let mut list = InstrumentList::new();
        list.follow_kits(2);
        app.set_instrument_list(&list);
        let (kit1, kit2) = (list.of_kit(0).unwrap(), list.of_kit(1).unwrap());
        let song = TuningPick {
            builtin: Builtin::Just,
            root: 2,
            ..TuningPick::default()
        };
        app.set_song_tuning(song);
        let key = app.kit_tunings_key();
        let names = app.kit_tunings();
        assert_eq!(names[0].value, "Song's: Just (5-limit) on D");
        assert_eq!(names[0].line, "tuning: song's");
        assert_eq!(names[1], names[0]);
        // Kit 2 on: its own, from the song's root and A4.
        let fx = app.step_kit_tuning(1, 1);
        assert!(
            fx.iter()
                .any(|e| matches!(e, Effect::Note(l) if l.starts_with("tunings: Kit 2 plays its own tuning"))),
            "{fx:?}"
        );
        assert_eq!(app.tunings().own(kit1), None);
        let own = app.tunings().own(kit2).expect("kit 2's own");
        assert_eq!((own.builtin, own.root), (Builtin::Equal12, 2));
        assert_ne!(app.kit_tunings_key(), key, "the loop sees it");
        app.step_kit_tuning(1, 2);
        let names = app.kit_tunings();
        assert_eq!(names[1].value, "Pythagorean on D");
        assert_eq!(names[1].line, "tuning: own");
        assert_eq!(names[0].value, "Song's: Just (5-limit) on D");
        // Back to the start: Song's again. A kit the list hasn't got does
        // nothing.
        app.step_kit_tuning(1, -9);
        assert_eq!(app.tunings().own(kit2), None);
        assert_eq!(app.kit_tunings_key(), key);
        assert!(app.step_kit_tuning(7, 1).is_empty());
    }

    #[test]
    fn the_synth_options_go_in_the_project_not_in_prefs() {
        use crate::synth::{Builtin, Engine, PadSettings};
        use mpfree_arp::scale::pads::{InKeyRows, PadLayout};
        use mpfree_arp::scale::SCALES;
        use mpfree_store::Settings;
        let files = vec![("werckmeister3".to_string(), false)];
        let mut app = ready();
        app.tuning_files = files.clone();
        app.open_tab(View::Synth);
        press(&mut app, MENU);
        let row = |a: SynthItem| {
            synth_pads::SYNTH_OPTIONS_SONGS
                .iter()
                .position(|&i| i == Item::Synth(a))
                .unwrap()
        };
        // No restore here, so "app" and "map" are handed out once.
        app.save_dirty = true;
        app.take_saves();
        assert!(!app.take_project_edit(), "nothing changed yet");
        // The Instrument row now picks the selected track's own sound; the
        // shared synth's engine is still kept and restored.
        app.synth_pads.engine = Engine::Fm;
        let mut step = |a: SynthItem, d: i32, times: usize| {
            for _ in 0..times {
                app.step_item(row(a), d);
            }
        };
        step(SynthItem::Scale, 1, 2);
        step(SynthItem::Root, 1, 9);
        step(SynthItem::Layout, 1, 2);
        step(SynthItem::Rows, 1, 1);
        // The song's tuning, on the Tunings page (tunings step T6).
        open_tunings_page(&mut app);
        let mut step = |a: SongRow, d: i32, times: usize| {
            for _ in 0..times {
                app.step_item(a as usize, d);
            }
        };
        step(SongRow::Tuning, 1, Builtin::ALL.len());
        step(SongRow::TuningRoot, 1, 2);
        step(SongRow::A4, -1, 8);
        assert!(app.take_saves().is_empty(), "they aren't Prefs");
        assert!(app.take_project_edit(), "they are the project's");
        assert!(!app.take_project_edit(), "said once");
        let kept = app.synth_settings();
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
        // The bank playing isn't kept, so switching banks is no edit.
        app.synth_bank(2);
        assert!(!app.take_project_edit());

        // Opening the project: every option comes back, on the bank playing.
        let mut next = ready();
        next.tuning_files = files.clone();
        let (notes, fx) = next.open_synth(&kept);
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
            notes[0].starts_with("synth options: FM, scale ")
                && notes[0].ends_with("tuning werckmeister3 (root D, A4 432 Hz)"),
            "{notes:?}"
        );
        assert!(!next.take_project_edit(), "an open is no edit");
        assert!(next.take_saves().is_empty(), "nor a Prefs change");

        // Another project whose tuning file or scale is gone, or with words
        // not understood: those are said and left at the defaults, never
        // the last project's.
        next.tuning_files.clear();
        let mut odd = kept.clone();
        odd.set_str("engine", "banjo");
        odd.set_str("scale", "No such scale");
        odd.set_i64("a4", 900);
        odd.set_str("bank-c", "sideways");
        let (notes, _) = next.open_synth(&odd);
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
        let p = next.synth_pads;
        let fresh = PadSettings::default();
        assert_eq!(
            (p.engine, p.scale, p.tuning.file, p.tuning.builtin),
            (Engine::Poly, None, None, fresh.tuning.builtin)
        );
        assert_eq!(
            (p.tuning.a4_hz, p.tuning.root, p.rows),
            (440, 2, InKeyRows::Fourths)
        );
        assert_eq!(p.banks[2].0, fresh.banks[2].0);
        assert_eq!(next.pad_scale_pick, 0);

        // A project with no options at all (New): the defaults.
        let (_, fx) = next.open_synth(&Settings::default());
        assert_eq!(fx, vec![Effect::SynthPads(fresh)]);
        assert_eq!((next.pad_scale_pick, next.pad_scale_root), (0, 0));

        // The owner's own scales come back by name; nothing given, nothing
        // done.
        let mut own = ready();
        own.custom_scales
            .push(("Mine".to_string(), SCALES[3].mask, SCALES[3].down));
        let mut mine = Settings::default();
        mine.set_str("scale", "Mine");
        own.open_synth(&mine);
        assert_eq!(own.pad_scale_pick, SCALES.len());
        assert!(own.synth_pads.scale.is_some());
        assert_eq!(
            ready().restore_synth(&Settings::default()),
            (vec![], vec![])
        );
    }

    /// A tap on track 3's sound line on Arrange: the Synth tab's MENU
    /// opens on the Instrument row, named for track 3, and MENU goes back
    /// to the Synth tab.
    #[test]
    fn a_tracks_sound_line_opens_its_instrument_row_on_the_synth_tab() {
        let mut app = ready();
        app.open_tab(View::Arrange);
        let fx = app.open_track_instrument(2);
        assert_eq!(app.track(), 2);
        assert_eq!(app.view(), View::Options);
        let m = fx
            .iter()
            .find_map(|e| match e {
                Effect::Menu(m) => Some(m),
                _ => None,
            })
            .expect("the menu");
        assert_eq!(m.title, "Synth options");
        assert_eq!(m.selected, 0);
        assert_eq!(
            m.rows[0],
            ("Track 3 instrument".to_string(), "Kit".to_string())
        );
        assert!(fx.contains(&Effect::Show(View::Synth)), "{fx:?}");
        press(&mut app, MENU);
        assert_eq!(app.view(), View::Synth);
    }

    /// The Synth tab's gear opens its options, as MENU does, and the
    /// options' Back goes back to the tab; Back in Prefs closes it.
    #[test]
    fn the_gear_and_back_do_what_menu_does() {
        let mut app = ready();
        app.open_tab(View::Synth);
        let fx = app.options_button();
        assert_eq!(app.view(), View::Options);
        assert!(
            fx.iter()
                .any(|e| matches!(e, Effect::Menu(m) if m.title == "Synth options")),
            "{fx:?}"
        );
        app.options_button();
        assert_eq!(app.view(), View::Synth);
        app.button(MENU, true, 0.0);
        app.button(MENU, false, 0.0);
        assert_eq!(app.view(), View::Options);
        app.options_button();
        assert_eq!(app.view(), View::Synth, "MENU's way in, Back's way out");
    }

    /// Pad focus (the instrument redesign's step 3): the synth chosen for
    /// the pads stays whichever track is selected, until Follow track;
    /// REC records it where it plays and never moves the pads.
    #[test]
    fn the_synth_chosen_for_the_pads_stays_until_follow_track() {
        use crate::rec_arm::Press;
        let mut app = ready();
        app.open_tab(View::Synth);
        assert!(app.pads_play_kit() && app.pads_focus() == PadFocus::Track);
        let fx = app.toggle_pads_synth();
        assert_eq!(fx[0], Effect::PadsFocus(PadFocus::SYNTH), "{fx:?}");
        assert!(
            matches!(&fx[1], Effect::Note(n) if n.contains("Poly on every track") && n.contains("Track 1 keeps its kit")),
            "{fx:?}"
        );
        assert!(app.pads_chose_synth() && !app.pads_play_kit());
        assert!(app.held_kit_pads().is_empty(), "no pad take from the synth");
        assert_eq!(app.track_sound(0), None, "the track still plays its kit");
        // The button again: Follow track.
        let fx = app.toggle_pads_synth();
        assert_eq!(
            fx,
            [
                Effect::PadsFocus(PadFocus::Track),
                Effect::Note("The pads follow the track: Track 1's instrument".into())
            ]
        );
        assert!(app.pads_play_kit());
        assert!(app.pads_follow_track().is_empty(), "once");
        // Another track selected: the pads keep the synth (rule 2).
        app.toggle_pads_synth();
        let fx = app.set_track(2);
        assert!(
            !fx.iter().any(|e| matches!(e, Effect::PadsFocus(_))),
            "{fx:?}"
        );
        assert!(app.pads_chose_synth());
        // REC and OVERDUB press as ever; the desk sends the take where the
        // synth plays (rec_synth).
        assert_eq!(press(&mut app, 90), [Effect::Rec(Press::Rec)]);
        assert_eq!(press(&mut app, 91), [Effect::Rec(Press::Overdub)]);
        // The track it goes to plays the same synth (no copy), the pads
        // keep it (rule 1), and the next press records there.
        assert!(!app.plays_pads_synth(4));
        let fx = app.rec_synth_on(4, false);
        assert_eq!(
            fx,
            [
                Effect::SynthOnTrack(4),
                Effect::Note(
                    "REC records Poly on Track 5: the track plays the same Poly as the pads".into()
                )
            ]
        );
        assert_eq!(app.track_sound(4), None, "no copy on the track");
        assert!(app.pads_chose_synth());
        assert!(app.plays_pads_synth(4));
        assert!(matches!(
            &app.rec_synth_on(9, true)[1],
            Effect::Note(n) if n == "REC records Poly on a new track, Track 10: the track plays the same Poly as the pads"
        ));
        // What the audio loop says wins: a kit or MIDI out picked over it.
        app.set_synth_tracks(1 << 4 | 1 << 6);
        assert!(app.plays_pads_synth(4) && !app.plays_pads_synth(9));
        app.set_configured(1 << 6);
        assert!(!app.plays_pads_synth(6));
        // A track's own instrument: the button chooses nothing while the
        // pads follow it, and with the synth chosen they keep it there too.
        let poly = crate::instruments::Held {
            id: crate::synth::Engine::Poly.node_id(),
            name: "Poly",
            instance: 0,
        };
        app.set_track_sound(2, Some(poly));
        assert!(!app.plays_pads_synth(2), "its own, not the pads' synth");
        app.toggle_pads_synth();
        assert!(app.pads_focus() == PadFocus::Track);
        assert!(app.toggle_pads_synth().is_empty());
        assert!(app.pads_focus() == PadFocus::Track);
        app.set_track(0);
        app.toggle_pads_synth();
        app.set_track(2);
        assert!(app.pads_chose_synth());
    }

    /// The list of synths on its own (step 6): a tap fills a free place
    /// and the pads play it once the audio loop lists it; opened again,
    /// the one they play is kept for Cancel, so the next tap fills another;
    /// Cancel gives it back and the synth tried goes once nothing plays it;
    /// Follow track lets that one go too; and with every place taken a tap
    /// says why.
    #[test]
    fn the_list_of_synths_fills_a_free_place_and_keeps_the_pads_one_for_cancel() {
        use crate::instrument_list::InstrumentList;
        use crate::instruments::{free, Held, SLOTS};
        use crate::synth::Engine;
        let mut app = ready();
        assert!(app.open_synth_pick().is_empty(), "only on the Synth tab");
        app.open_tab(View::Synth);
        let view = |fx: &[Effect]| match fx.last() {
            Some(Effect::SynthPick(v)) => v.clone(),
            other => panic!("{other:?}"),
        };
        let v = view(&app.open_synth_pick()).expect("open");
        assert_eq!(
            v.choices[0],
            ("Track 1's".into(), "The pads follow the track again".into())
        );
        assert_eq!(v.choices[2], ("FM".into(), "Electric pianos, bells".into()));
        assert_eq!((v.now, v.had), (Some(0), Some(0)));
        let fx = app.pick_pads_synth(2);
        let fm_id = Some(Engine::Fm.node_id());
        assert_eq!(
            fx[0],
            Effect::TrackSound {
                track: free(0),
                id: fm_id
            }
        );
        assert_eq!(view(&fx).expect("open").now, Some(2));
        // Listed: the pads play it. Keep.
        let mut list = InstrumentList::new();
        list.follow_free(1);
        let fm = list.free_of(0).expect("an id");
        assert_eq!(
            app.set_instrument_list(&list),
            [Effect::PadsFocus(PadFocus::Instrument(fm))]
        );
        assert_eq!(app.close_synth_pick(true), [Effect::SynthPick(None)]);
        assert!(app.put_away_free().is_empty(), "the pads play it");
        // Opened again: FM is kept for Cancel, so Pluck fills the next place.
        app.open_synth_pick();
        let fx = app.pick_pads_synth(4);
        assert_eq!(
            fx[0],
            Effect::TrackSound {
                track: free(1),
                id: Some(Engine::Pluck.node_id())
            }
        );
        assert!(app.put_away_free().is_empty(), "both kept while it is open");
        // Cancel: FM again, and Pluck goes once nothing plays it.
        assert_eq!(
            app.close_synth_pick(false),
            [
                Effect::PadsFocus(PadFocus::Instrument(fm)),
                Effect::SynthPick(None)
            ]
        );
        let fx = app.put_away_free();
        assert_eq!(
            fx[0],
            Effect::TrackSound {
                track: free(1),
                id: None
            }
        );
        // Follow track (the button under the card): FM goes too.
        app.toggle_pads_synth();
        assert_eq!(app.pads_focus(), PadFocus::Track);
        assert_eq!(
            app.put_away_free()[0],
            Effect::TrackSound {
                track: free(0),
                id: None
            }
        );
        // Every place taken: a tap says why, and nothing changes.
        let poly = Held {
            id: Engine::Poly.node_id(),
            name: "Poly",
            instance: 0,
        };
        for t in 0..SLOTS {
            app.set_track_sound(t, Some(poly));
        }
        app.open_synth_pick();
        let fx = app.pick_pads_synth(2);
        assert!(
            matches!(&fx[0], Effect::Tell(t) if t.starts_with("No room for FM")),
            "{fx:?}"
        );
        assert_eq!(app.track_sound(free(0)), None);
        // Leaving the tab closes the list: the card opens it again.
        app.open_tab(View::Play);
        app.open_tab(View::Synth);
        assert!(view(&app.open_synth_pick()).is_some());
    }

    /// Composing tools S2.6b, Add a part's New track: a try puts the
    /// part's synth in a free place nothing else uses, kept there while it
    /// tries (not put away, not taken by the list of synths); once the
    /// audio loop lists it the track Keep would add plays it, once;
    /// another sound goes into the same place and the send stays; a track
    /// added ends the send until Try again asks for the new index; Back
    /// ends it and puts the synth away; Kept leaves the new track playing
    /// it; with every place taken a try says why.
    #[test]
    fn new_tracks_synth_tries_in_a_free_place_and_goes_with_back() {
        use crate::instrument_list::InstrumentList;
        use crate::instruments::{free, Held, SLOTS};
        use crate::synth::Engine;
        let mut app = ready();
        let (bass, squelch) = (Engine::Bass.node_id(), Engine::Squelch.node_id());
        let fx = app.add_part_sound(AddSound::Try { track: 8, id: bass });
        assert_eq!(
            fx,
            [Effect::TrackSound {
                track: free(0),
                id: Some(bass)
            }]
        );
        assert_eq!(app.add_sound_holder(), Some(free(0)));
        assert!(app.put_away_free().is_empty(), "kept while it tries");
        assert!(app.add_sound_follow(8).is_empty(), "not in the list yet");
        // The list of synths takes another place.
        app.open_tab(View::Synth);
        app.open_synth_pick();
        assert_eq!(
            app.pick_pads_synth(2)[0],
            Effect::TrackSound {
                track: free(1),
                id: Some(Engine::Fm.node_id())
            }
        );
        app.close_synth_pick(false);
        // Listed: track 9 plays it by a send, once.
        let mut list = InstrumentList::new();
        list.follow_free(1);
        app.set_instrument_list(&list);
        let send = |to| Effect::AddPartSend { track: 8, to };
        assert_eq!(app.add_sound_follow(8), [send(Some(free(0)))]);
        assert_eq!(app.track_send(8), Some(free(0)));
        assert!(app.add_sound_follow(8).is_empty(), "once");
        assert!(app.put_away_free().iter().all(|e| !matches!(
            e,
            Effect::TrackSound { track, .. } if *track == free(0)
        )));
        // Another sound: the same place, the send stays.
        assert_eq!(
            app.add_part_sound(AddSound::Try {
                track: 8,
                id: squelch
            }),
            [Effect::TrackSound {
                track: free(0),
                id: Some(squelch)
            }]
        );
        assert!(app.add_sound_follow(8).is_empty(), "still sent");
        // A track added (+ Track): the send ends; Try again on the next
        // index plays there.
        assert_eq!(app.add_sound_follow(9), [send(None)]);
        assert_eq!(app.track_send(8), None);
        assert!(app.add_sound_follow(9).is_empty());
        assert!(app
            .add_part_sound(AddSound::Try {
                track: 9,
                id: squelch
            })
            .is_empty());
        assert_eq!(
            app.add_sound_follow(9),
            [Effect::AddPartSend {
                track: 9,
                to: Some(free(0))
            }]
        );
        // Back: the send ends and the synth is put away.
        let fx = app.add_part_sound(AddSound::Back);
        assert_eq!(
            fx[..2],
            [
                Effect::AddPartSend { track: 9, to: None },
                Effect::TrackSound {
                    track: free(0),
                    id: None
                }
            ]
        );
        assert_eq!(app.add_sound_holder(), None);
        assert_eq!(app.track_sound(free(0)), None);
        // Kept: the new track plays it on.
        app.add_part_sound(AddSound::Try { track: 8, id: bass });
        assert_eq!(app.add_sound_follow(8), [send(Some(free(0)))]);
        assert!(app.add_part_sound(AddSound::Kept).is_empty());
        assert_eq!(app.add_sound_holder(), None);
        assert!(app.put_away_free().iter().all(|e| !matches!(
            e,
            Effect::TrackSound { track, .. } if *track == free(0)
        )));
        // Every place taken: a try says why.
        let poly = Held {
            id: Engine::Poly.node_id(),
            name: "Poly",
            instance: 0,
        };
        for t in 0..SLOTS {
            app.set_track_sound(t, Some(poly));
        }
        let fx = app.add_part_sound(AddSound::Try { track: 9, id: bass });
        assert!(
            matches!(&fx[..], [Effect::Tell(t)] if t.starts_with("No room for the Bass synth")),
            "{fx:?}"
        );
        assert_eq!(app.add_sound_holder(), None);
    }

    /// The synths New track offers (mpfree-time-ui's add_view lists them
    /// by node id and name) are this app's engines, named as it names
    /// them.
    #[test]
    fn new_tracks_synths_are_the_apps_engines() {
        use crate::synth::Engine;
        use mpfree_time_ui::add_view::{BASS_SOUNDS, CHORDS_SOUNDS, DRUMS_SOUNDS, MELODY_SOUNDS};
        for (id, name) in BASS_SOUNDS
            .iter()
            .chain(&CHORDS_SOUNDS)
            .chain(&MELODY_SOUNDS)
            .chain(&DRUMS_SOUNDS)
        {
            let e = Engine::from_node_id(id).unwrap_or_else(|| panic!("{id}"));
            assert_eq!((e.node_id(), e.name()), (*id, *name));
        }
    }

    /// Step 6c, banks point at instruments (the design's rule 10): with
    /// "Bank B only" at the top of the list a tap gives the bank the pads
    /// are on a synth of its own and the other banks play on; PAD BANK puts
    /// it on the pads and takes it off, and nothing puts it away while the
    /// bank points at it; it is saved and comes back (a bank line that
    /// doesn't fit is said, the rest opens); All banks gives a bank's own
    /// up and Cancel gives it back; the bank's first choice gives it to the
    /// track whatever the other banks play; Follow track gives every bank
    /// back; and a bank's synth gone from the list is said.
    #[test]
    fn bank_b_only_gives_one_bank_a_synth_of_its_own() {
        use crate::instrument_list::InstrumentList;
        use crate::instruments::free;
        use crate::synth::Engine;
        let mut app = ready();
        app.open_tab(View::Synth);
        let view = |fx: &[Effect]| match fx.last() {
            Some(Effect::SynthPick(v)) => v.clone(),
            other => panic!("{other:?}"),
        };
        assert!(app.set_pad_page(1).is_empty(), "nothing of its own yet");
        let v = view(&app.open_synth_pick()).expect("open");
        assert_eq!(v.bank, Some(('B', false)), "All banks to start with");
        let v = view(&app.synth_pick_scope(true)).expect("open");
        assert_eq!(v.bank, Some(('B', true)));
        assert_eq!(v.choices[0].1, "Bank B follows the track");
        // FM on bank B alone: a free place, then its id.
        let fx = app.pick_pads_synth(2);
        assert_eq!(
            fx[0],
            Effect::TrackSound {
                track: free(0),
                id: Some(Engine::Fm.node_id())
            }
        );
        assert!(
            matches!(&fx[1], Effect::Note(n) if n.starts_with("Bank B plays FM · the other banks play as they did")),
            "{fx:?}"
        );
        let mut list = InstrumentList::new();
        list.follow_free(1);
        let fm = list.free_of(0).expect("an id");
        assert_eq!(
            app.set_instrument_list(&list),
            [Effect::PadsFocus(PadFocus::Instrument(fm))]
        );
        assert_eq!(
            (app.pads_focus(), app.pads_bank(), app.pads_holder()),
            (PadFocus::Instrument(fm), Some(1), Some(free(0)))
        );
        assert_eq!(view(&app.pick_pads_synth(2)).expect("open").now, Some(2));
        assert_eq!(app.close_synth_pick(true), [Effect::SynthPick(None)]);
        // Bank A plays the track's kit; FM stays while bank B points at it.
        assert_eq!(app.set_pad_page(0), [Effect::PadsFocus(PadFocus::Track)]);
        assert_eq!(
            (app.pads_focus(), app.pads_bank(), app.pads_holder()),
            (PadFocus::Track, None, None)
        );
        assert!(app.put_away_free().is_empty(), "bank B points at FM");
        assert_eq!(
            app.set_pad_page(1),
            [Effect::PadsFocus(PadFocus::Instrument(fm))]
        );
        // Saved, and back.
        let text = app.pads_part();
        assert_eq!(
            text,
            format!("mpfree-pads 2\nplay track\nbank B instrument {fm}\n")
        );
        let mut other = ready();
        other.set_instrument_list(&list);
        let (notes, _) = other.open_pads(Some(&text));
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(other.bank_focus(1), Some(PadFocus::Instrument(fm)));
        assert_eq!(other.pads_focus(), PadFocus::Track, "on bank A");
        other.set_pad_page(1);
        assert_eq!(other.pads_holder(), Some(free(0)));
        let (notes, _) = other.open_pads(Some(
            "mpfree-pads 2\nplay track\nbank B track\nbank C instrument 9\n",
        ));
        assert!(
            matches!(&notes[..], [n] if n.contains("Bank C's instrument 9 isn't in the project's list")),
            "{notes:?}"
        );
        assert_eq!(other.bank_focus(1), Some(PadFocus::Track), "the rest opens");
        // Kept, missing (sampler step 15b): bank C plays the base meanwhile.
        assert_eq!(other.bank_focus(2), None);
        assert!(other.pad_map().banks[2].is_some());
        assert!(other.pads_part().contains("\nbank C instrument 9\n"));
        for (bad, why) in [
            ("mpfree-pads 2\nplay track\nbank I track\n", "doesn't read"),
            ("mpfree-pads 2\nplay track\nbank B drums\n", "doesn't read"),
            (
                "mpfree-pads 2\nplay track\nbank B track\nbank B track\n",
                "twice what bank B plays",
            ),
            ("mpfree-pads 2\nbank B track\n", "doesn't say"),
        ] {
            let (notes, _) = other.open_pads(Some(bad));
            assert!(
                matches!(&notes[..], [n] if n.contains(why)),
                "{bad:?}: {notes:?}"
            );
            assert_eq!(other.bank_focus(1), None, "{bad:?}");
        }
        // Opened on bank B, the list starts on Bank B only. All banks:
        // Pluck plays on every bank, bank B's own given up; FM's place is
        // kept for Cancel.
        let v = view(&app.open_synth_pick()).expect("open");
        assert_eq!(v.bank, Some(('B', true)));
        app.synth_pick_scope(false);
        let fx = app.pick_pads_synth(4);
        assert_eq!(
            fx[0],
            Effect::TrackSound {
                track: free(1),
                id: Some(Engine::Pluck.node_id())
            }
        );
        assert_eq!(app.bank_focus(1), None);
        // Cancel: bank B has FM again, and Pluck goes.
        app.close_synth_pick(false);
        assert_eq!(app.bank_focus(1), Some(PadFocus::Instrument(fm)));
        assert_eq!(app.pads_focus(), PadFocus::Instrument(fm));
        assert_eq!(
            app.put_away_free()[0],
            Effect::TrackSound {
                track: free(1),
                id: None
            }
        );
        // Every bank plays the shared synth; bank B's first choice gives it
        // to the track (a bank line of its own), and FM goes.
        app.set_pad_page(0);
        app.toggle_pads_synth();
        assert!(app.pads_chose_synth());
        app.set_pad_page(1);
        assert_eq!(app.pads_focus(), PadFocus::Instrument(fm));
        app.open_synth_pick();
        let fx = app.pick_pads_synth(0);
        assert!(
            fx.contains(&Effect::Note(
                "Bank B follows the track: Track 1's instrument".into()
            )),
            "{fx:?}"
        );
        assert_eq!(app.pads_focus(), PadFocus::Track);
        app.close_synth_pick(true);
        assert_eq!(
            app.pads_part(),
            "mpfree-pads 2\nplay instrument 1\nbank B track\n"
        );
        assert_eq!(
            app.put_away_free()[0],
            Effect::TrackSound {
                track: free(0),
                id: None
            }
        );
        app.set_pad_page(0);
        assert!(app.pads_chose_synth(), "bank A plays the shared synth");
        // Follow track: every bank.
        app.pads_follow_track();
        assert_eq!(app.pads_part(), "mpfree-pads 2\nplay track\n");
        // A bank's synth gone from the list: its entry stays, missing
        // (sampler step 15b), said once; the bank plays what the others
        // play meanwhile, and again once the synth is back.
        other.open_pads(Some(&text));
        other.set_pad_page(1);
        let fx = other.set_instrument_list(&InstrumentList::new());
        assert_eq!(
            fx,
            [
                Effect::PadsFocus(PadFocus::Track),
                Effect::Note(format!(
                    "Bank B's instrument {fm} is gone: the bank plays what the other banks play until you pick another"
                ))
            ]
        );
        assert_eq!(other.bank_focus(1), None);
        assert!(
            other.set_instrument_list(&InstrumentList::new()).is_empty(),
            "once"
        );
        assert_eq!(other.pads_part(), text, "kept");
        let fx = other.set_instrument_list(&list);
        assert_eq!(fx, [Effect::PadsFocus(PadFocus::Instrument(fm))]);
    }

    /// The pads part: what the pads play is saved with the project, and
    /// a part that doesn't read is said, the pads following the track.
    #[test]
    fn the_pads_focus_is_saved_and_a_part_that_doesnt_read_is_said() {
        let mut app = ready();
        assert_eq!(app.pads_part(), "mpfree-pads 2\nplay track\n");
        app.open_tab(View::Synth);
        app.toggle_pads_synth();
        let text = app.pads_part();
        assert_eq!(text, "mpfree-pads 2\nplay instrument 1\n");
        let mut other = ready();
        let (notes, fx) = other.open_pads(Some(&text));
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(fx, [Effect::PadsFocus(PadFocus::SYNTH)]);
        assert!(other.pads_chose_synth());
        // None: New, or a project from before the part.
        let (notes, fx) = other.open_pads(None);
        assert!(notes.is_empty());
        assert_eq!(fx, [Effect::PadsFocus(PadFocus::Track)]);
        for (bad, why) in [
            ("mpfree-pads 1\nplay track\n", "its first line is"),
            ("mpfree-pads 2\nplay instrument 0\n", "doesn't read"),
            ("mpfree-pads 2\nplay drums\n", "doesn't read"),
            ("mpfree-pads 2\nplay track\nplay track\n", "twice"),
            ("mpfree-pads 2\n", "doesn't say"),
        ] {
            other.open_pads(Some(&text));
            let (notes, fx) = other.open_pads(Some(bad));
            assert!(
                matches!(&notes[..], [n] if n.contains(why) && n.ends_with("the pads follow the track")),
                "{bad:?}: {notes:?}"
            );
            assert_eq!(fx, [Effect::PadsFocus(PadFocus::Track)], "{bad:?}");
        }
        // An instrument the list hasn't got: kept, missing (sampler step
        // 15b), said; the pads play the track's instrument meanwhile.
        let (notes, fx) = other.open_pads(Some("mpfree-pads 2\nplay instrument 2\n"));
        assert!(
            matches!(&notes[..], [n] if n.contains("The pads' instrument 2 isn't in the project's list")),
            "{notes:?}"
        );
        assert_eq!(fx, [Effect::PadsFocus(PadFocus::Track)]);
        assert_eq!(other.pads_part(), "mpfree-pads 2\nplay instrument 2\n");
        // An instrument the project's list has: a free place's (step 6).
        let mut list = crate::instrument_list::InstrumentList::new();
        list.follow_free(1);
        other.set_instrument_list(&list);
        let (notes, fx) = other.open_pads(Some("mpfree-pads 2\nplay instrument 2\n"));
        assert!(notes.is_empty(), "{notes:?}");
        let id = list.free_of(0).expect("an id");
        assert_eq!(fx, [Effect::PadsFocus(PadFocus::Instrument(id))]);
    }

    /// Where REC records the pads' synth (the design review of 20 Nov,
    /// points A and B): the selected track when nobody set it up, else the
    /// first such track, else a new one; a track with no clips but set up
    /// (renamed, a setting moved, or what the app holds: a kit or MIDI out
    /// picked, an instrument, mixer settings) is never taken; a pinned arm
    /// is kept, or REC goes nowhere; and a full project takes nothing.
    #[test]
    fn rec_records_the_pads_synth_on_a_track_nobody_set_up() {
        use super::pads_focus::unfit;
        use mpfree_seq::{Clip, Project, Track};
        let mut p = Project::new();
        for n in 0..6 {
            p.tracks.push(Track::new(&format!("Track {}", n + 1), n));
        }
        let free = |_| None;
        assert_eq!(
            synth_track(&p, 2, false, free),
            SynthTake::On(2),
            "the selected one"
        );
        p.tracks[2].slots[1] = Some(Clip::new("Beat", 384));
        assert_eq!(unfit(&p, 2, free), Some("has clips"));
        assert_eq!(
            synth_track(&p, 2, false, free),
            SynthTake::On(0),
            "the first free one"
        );
        // Empty but set up: renamed, a setting moved.
        p.tracks[0].name = "Bass".into();
        p.tracks[1].record_quantise = Some(96);
        assert_eq!(unfit(&p, 0, free), Some("has settings of its own"));
        assert_eq!(unfit(&p, 1, free), Some("has settings of its own"));
        // Renamed to look like a default name: still set up. Only the
        // track's own default ("Track 6" for the sixth) is free.
        for name in ["Track 99", "Track 06", "Track +6", "Track 5", "Track 6 "] {
            p.tracks[5].name = name.into();
            assert_eq!(
                unfit(&p, 5, free),
                Some("has settings of its own"),
                "{name}"
            );
        }
        p.tracks[5].name = "Track 6".into();
        assert_eq!(unfit(&p, 5, free), None);
        // What the app holds: a kit picked on track 4, mixer settings on 5.
        let app_side = |t: usize| match t {
            3 => Some("has a kit or a MIDI output picked"),
            4 => Some("has mixer settings"),
            _ => None,
        };
        assert_eq!(synth_track(&p, 2, false, app_side), SynthTake::On(5));
        // A pinned arm stays where it is, or REC goes nowhere.
        assert_eq!(
            synth_track(&p, 2, true, app_side),
            SynthTake::Pinned {
                track: 2,
                why: "has clips"
            }
        );
        assert_eq!(synth_track(&p, 5, true, app_side), SynthTake::On(5));
        // Every track set up: a new one, unless the project is full.
        p.tracks[5].follows = true;
        assert_eq!(synth_track(&p, 2, false, app_side), SynthTake::New);
        let last = mpfree_seq::MAX_TRACKS as u16;
        for n in p.tracks.len() as u16..last {
            let mut t = Track::new(&format!("Track {}", n + 1), n);
            t.mute = true;
            p.tracks.push(t);
        }
        assert_eq!(synth_track(&p, 2, false, app_side), SynthTake::Full);
        // A Loop track is never one.
        let loops = Track::new_loops("Track 7", 0);
        p.tracks[6] = loops;
        assert_eq!(unfit(&p, 6, free), Some("is a Loop track"));
    }

    /// The recording plan's words on the REC chip (step 5b): where REC
    /// will record the pads' synth, by the track's name, before you press.
    #[test]
    fn the_rec_chip_names_where_rec_will_record_the_pads_synth() {
        let name = |t: usize| {
            if t == 1 {
                "Bass".to_string()
            } else {
                format!("Track {}", t + 1)
            }
        };
        let plan = |take| RecPlan {
            take,
            synth: "Poly",
            holder: None,
            ready: false,
            kit: None,
            layers: [None; REC_MORE],
        };
        assert_eq!(plan(SynthTake::On(2)).chip(name), "Track 3 · Poly");
        assert_eq!(plan(SynthTake::On(1)).chip(name), "Bass · Poly");
        assert_eq!(plan(SynthTake::New).chip(name), "New track · Poly");
        assert_eq!(
            plan(SynthTake::Pinned {
                track: 0,
                why: "has clips"
            })
            .chip(name),
            "Track 1 · not free"
        );
        assert_eq!(plan(SynthTake::Full).chip(name), "No free track · Poly");
        // Layers (step 11c): the first by name, the rest counted; a layer
        // with no place isn't.
        let layer = |take, from| {
            Some(LayerTake {
                take,
                synth: "Pluck",
                sound: RecSound::Synth(Some(3)),
                from,
                plays: false,
            })
        };
        let mut two = plan(SynthTake::On(1));
        two.layers[0] = layer(SynthTake::On(2), RecFrom::Layer(0));
        assert_eq!(two.chip(name), "Bass · Poly + 1 more");
        two.layers[1] = layer(SynthTake::New, RecFrom::Layer(1));
        assert_eq!(two.chip(name), "Bass · Poly + 2 more");
        // A sound with no track: REC records nothing, and the chip names
        // it (sampler step 15c).
        two.layers[1] = layer(SynthTake::Full, RecFrom::Bank(1));
        assert_eq!(two.chip(name), "No free track · Bank B's Pluck");
        let mut pinned = two;
        pinned.take = SynthTake::Pinned {
            track: 0,
            why: "has clips",
        };
        assert_eq!(pinned.chip(name), "Track 1 · not free", "nothing records");
    }

    /// Where REC records the pads' layers (step 11c): an armed track that
    /// plays a layer's synth already keeps it; else the first free track
    /// that the first and the layers before haven't taken; else a new one
    /// after the first's, while the project holds more; else none.
    #[test]
    fn each_layer_records_where_its_synth_plays_or_on_a_free_track() {
        let mut layers = [None; REC_MORE];
        let more = |name, sound, from| Some(RecMore { name, sound, from });
        layers[0] = more("Pluck", RecSound::Synth(Some(130)), RecFrom::Layer(0));
        layers[1] = more("Poly", RecSound::Synth(None), RecFrom::Base);
        // Tracks 0 to 4: Track 3 plays Pluck; Tracks 2 and 4 are free.
        let plays = |t: usize, s: RecSound| match s {
            RecSound::Synth(Some(130)) => t == 3,
            _ => false,
        };
        let fits = |t: usize| t == 2 || t == 4;
        // Track 3 armed too: Pluck stays there; Poly takes Track 4, as the
        // first took Track 2.
        let got = layer_takes(layers, 5, Some(2), 1 << 3, plays, fits);
        let take = |l: Option<LayerTake>| l.map(|l| (l.take, l.plays));
        assert_eq!(take(got[0]), Some((SynthTake::On(3), true)));
        assert_eq!(take(got[1]), Some((SynthTake::On(4), false)));
        assert_eq!(got[2], None);
        assert_eq!(got[1].unwrap().synth, "Poly");
        // Track 3 not armed: Pluck takes the free Track 4, Poly a new one.
        let got = layer_takes(layers, 5, Some(2), 0, plays, fits);
        assert_eq!(take(got[0]), Some((SynthTake::On(4), false)));
        assert_eq!(take(got[1]), Some((SynthTake::New, false)));
        // The first on a new track: the layers' new ones come after it,
        // until the project holds no more.
        let max = mpfree_seq::MAX_TRACKS;
        let got = layer_takes(layers, max - 2, None, 0, plays, |_| false);
        assert_eq!(take(got[0]), Some((SynthTake::New, false)));
        assert_eq!(take(got[1]), Some((SynthTake::Full, false)));
    }

    /// The arp and the synth chosen for the pads (sampler step 15c, after
    /// the design review of 20 Nov, point C): NOTE REPEAT, Latch and the
    /// arp never give the pads back, and the button chooses the synth with
    /// the arp on; the pads' arp in the audio loop plays it (main.rs,
    /// `PadsArp`). Over a track that plays the synth, the track's arp
    /// plays it, as before.
    #[test]
    fn the_arp_never_gives_the_pads_back() {
        let mut app = ready();
        app.open_tab(View::Synth);
        app.toggle_pads_synth();
        assert!(app.pads_chose_synth());
        // NOTE REPEAT held: the pads keep the synth.
        app.button(REPEAT, true, 0.0);
        assert!(app.generating());
        assert!(app.pads_chose_synth());
        // Held still: the button gives the pads back when you press it, and
        // chooses the synth again.
        app.toggle_pads_synth();
        assert!(!app.pads_chose_synth() && app.pads_play_kit());
        let fx = app.toggle_pads_synth();
        assert!(app.pads_chose_synth(), "{fx:?}");
        app.button(REPEAT, false, 0.0);
        assert!(!app.generating());
        assert!(app.pads_chose_synth(), "the arp let go: nothing moves");
    }

    /// Step 4: the Instrument row offers the other tracks' own
    /// instruments right after Kit ("Track 3's FM"): a pick plays that one
    /// from the selected track, and Kit gives it its kit back. A track
    /// that has its own and picks another's gives up its own, and the
    /// tracks that played it play their kits.
    #[test]
    fn the_instrument_row_copies_another_tracks_instrument_only_while_theres_room() {
        use crate::instruments::{Held, SLOTS};
        use crate::synth::Engine;
        let mut app = ready();
        app.open_tab(View::Synth);
        press(&mut app, MENU);
        let row = synth_pads::SYNTH_OPTIONS
            .iter()
            .position(|&i| i == Item::Synth(SynthItem::Engine))
            .unwrap();
        let poly = Held {
            id: Engine::Poly.node_id(),
            name: "Poly",
            instance: 0,
        };
        // Every place taken but track 20's, which sends to track 1's Poly.
        for t in 0..SLOTS {
            app.set_track_sound(t, Some(poly));
        }
        app.set_track_sends(&std::array::from_fn(|t| (t == 19).then_some(0)));
        app.set_track(19);
        let fx = app.step_item(row, 1);
        assert!(
            fx.iter()
                .any(|e| matches!(e, Effect::Tell(s) if s.contains("the most at once"))),
            "{fx:?}"
        );
        assert!(!fx.iter().any(|e| matches!(e, Effect::TrackCopy { .. })));
        assert_eq!(app.track_send(19), Some(0), "it keeps sending");
        assert_eq!(app.track_sound(19), None);
        // A track with its own already takes a copy in its place: back
        // from Poly, the copy of the last track's.
        app.set_track(1);
        let fx = app.step_item(row, -1);
        assert!(
            fx.iter()
                .any(|e| matches!(e, Effect::TrackCopy { track: 1, from: 15 })),
            "{fx:?}"
        );
    }

    #[test]
    fn the_instrument_row_plays_another_tracks_instrument() {
        use crate::instruments::Held;
        use crate::synth::Engine;
        let mut app = ready();
        app.open_tab(View::Synth);
        press(&mut app, MENU);
        let row = synth_pads::SYNTH_OPTIONS
            .iter()
            .position(|&i| i == Item::Synth(SynthItem::Engine))
            .unwrap();
        let send = |fx: &[Effect]| {
            fx.iter().find_map(|e| match e {
                Effect::TrackSend { track, to } => Some((*track, *to)),
                _ => None,
            })
        };
        let fm = Held {
            id: Engine::Fm.node_id(),
            name: "FM",
            instance: 0,
        };
        app.set_track_sound(2, Some(fm));
        app.set_track(4);
        let _ = app.take_project_edit();
        let fx = app.step_item(row, 1);
        assert_eq!(send(&fx), Some((4, 2)));
        assert_eq!(
            app.synth_item_text(SynthItem::Engine),
            ("Track 5 instrument", "Track 3's FM".to_string())
        );
        assert_eq!(app.track_send(4), Some(2));
        assert!(app.track_plays_instrument(4) && app.take_project_edit());
        // On past it, a copy of it as its own (Duplicate, step 4c).
        let fx = app.step_item(row, 1);
        assert!(
            fx.iter()
                .any(|e| matches!(e, Effect::TrackCopy { track: 4, from: 2 })),
            "{fx:?}"
        );
        assert_eq!(app.track_send(4), None);
        assert_eq!(app.track_sound(4).map(|h| h.id), Some(Engine::Fm.node_id()));
        assert_eq!(
            app.synth_item_text(SynthItem::Engine).1,
            "Copy of Track 3's FM"
        );
        assert!(app.take_project_edit());
        // The loop says it went in: still the copy.
        app.set_track_sound(4, Some(fm));
        assert_eq!(
            app.synth_item_text(SynthItem::Engine).1,
            "Copy of Track 3's FM"
        );
        // On past the copy, a new instrument of its own: Poly first.
        let fx = app.step_item(row, 1);
        assert!(fx.iter().any(|e| matches!(
            e,
            Effect::TrackSound { track: 4, id: Some(id) } if *id == Engine::Poly.node_id()
        )));
        assert_eq!(app.synth_item_text(SynthItem::Engine).1, "Poly");
        // Kit again, past the copy and Track 3's FM.
        app.step_item(row, -1);
        app.step_item(row, -1);
        app.step_item(row, -1);
        assert_eq!(app.synth_item_text(SynthItem::Engine).1, "Kit");
        // A copy stops being one when its track gets another engine, or a
        // project opens.
        app.step_item(row, 1);
        app.step_item(row, 1);
        assert_eq!(
            app.synth_item_text(SynthItem::Engine).1,
            "Copy of Track 3's FM"
        );
        app.forget_copies();
        assert_eq!(app.synth_item_text(SynthItem::Engine).1, "FM");
        app.step_item(row, -1);
        app.step_item(row, -1);
        app.step_item(row, -1);
        app.step_item(row, -1);
        assert_eq!(app.synth_item_text(SynthItem::Engine).1, "Kit");
        assert!(!app.track_plays_instrument(4));
        // Track 3 isn't offered its own; picking track 5's instead gives
        // its own up, and track 2, which played track 3's, plays its kit.
        app.set_track_sound(4, Some(fm));
        app.set_track_sends(&std::array::from_fn(|t| (t == 1).then_some(2)));
        app.set_track(2);
        app.step_item(row, -1);
        assert_eq!(
            app.track_send(1),
            Some(2),
            "a new sound in it keeps track 2 on it"
        );
        // So does a copy of Track 5's FM in it.
        app.step_item(row, -1);
        assert_eq!(app.track_send(1), Some(2));
        let fx = app.step_item(row, -1);
        assert_eq!(
            send(&fx),
            Some((2, 4)),
            "Kit, Track 5's FM, a copy of it, Poly, FM ..."
        );
        assert_eq!(app.track_sound(2), None);
        assert_eq!((app.track_send(1), app.track_send(2)), (None, Some(4)));
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
                "Track 1 instrument",
                "Scale",
                "Scale root",
                "Pad layout",
                "In key rows",
                "Tuning",
                "Note repeat rate",
                "Tempo",
                "Pad level",
                "Latch"
            ]
        );
        assert_eq!(m.rows[0].1, "Kit");
        assert_eq!(m.rows[5].1, "Song's: 12-TET");
        assert_eq!(m.rows[1].1, "Off");
        assert_eq!(m.rows[3].1, "Chromatic (no scale)");
        let row = |a: SynthItem| {
            synth_pads::SYNTH_OPTIONS_SONGS
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
        // Instrument: the selected track's own sound. Kit, then Poly, FM,
        // Drums, Pluck, Keys, Drawbar, Squelch, Analog, Strings, Bass, Wind,
        // Choir and the plugin synths; then back to Kit.
        let sound = |fx: &[Effect]| {
            fx.iter().find_map(|e| match e {
                Effect::TrackSound { track, id } => Some((*track, *id)),
                _ => None,
            })
        };
        let _ = app.take_project_edit();
        for want in Engine::ALL {
            let fx = app.step_item(row(SynthItem::Engine), 1);
            assert_eq!(sound(&fx), Some((0, Some(want.node_id()))));
            assert_eq!(pads(&fx), None, "the shared synth is left alone");
            assert!(app.take_project_edit(), "the project keeps it");
        }
        assert_eq!(
            sound(&app.step_item(row(SynthItem::Engine), 1)),
            None,
            "Choir is the last with no plugin synths"
        );
        app.plugin_synths = vec![("clap:org.example.synth", "Example Synth")];
        assert_eq!(
            sound(&app.step_item(row(SynthItem::Engine), 1)),
            Some((0, Some("clap:org.example.synth")))
        );
        assert_eq!(app.synth_item_text(SynthItem::Engine).1, "Example Synth");
        // Each track keeps its own: track 2 still plays the kit, and the
        // row names the track it sets.
        app.set_track(1);
        assert_eq!(
            app.synth_item_text(SynthItem::Engine),
            ("Track 2 instrument", "Kit".to_string())
        );
        app.set_track(0);
        for _ in 0..Engine::ALL.len() + 1 {
            app.step_item(row(SynthItem::Engine), -1);
        }
        assert_eq!(app.track_sound(0), None, "back to the kit");
        assert_eq!(app.synth_pads.engine, Engine::Poly);
        // Chromatic is already on: a tap says so.
        assert!(matches!(
            app.synth_pick_layout(false).as_slice(),
            [Effect::Tell(t)] if t.contains("already Chromatic")
        ));
        // No scale: In key picks Major on the root and says so (live test 3).
        let fx = app.synth_pick_layout(true);
        let p = pads(&fx).unwrap();
        assert_eq!(p.layout(), PadLayout::InKey);
        assert!(p.scale.is_some());
        assert!(
            fx.iter()
                .any(|e| matches!(e, Effect::Tell(t) if t.contains("In key, C Major"))),
            "{fx:?}"
        );
        // Back to no scale and Chromatic for the rest.
        app.step_item(row(SynthItem::Scale), -1);
        assert!(app.synth_pads.scale.is_none());
        app.synth_pick_layout(false);
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
        assert!(
            !app.synth_pick_layout(true)
                .iter()
                .any(|e| matches!(e, Effect::SynthPads(_))),
            "In key already"
        );
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

    /// A list or pad over the page (the number pad, the undo history)
    /// takes the wheel, its push and - / + on every page, Play's too;
    /// UNDO says when it goes down and up, so the screen can tell it is
    /// held (Jay after live test 3: hold UNDO for the page's history).
    #[test]
    fn an_overlay_takes_the_wheel_and_undo_says_when_it_goes_down_and_up() {
        let mut app = ready();
        app.open_tab(View::Play);
        assert_eq!(app.wheel(1), vec![Effect::Nav(Nav::Turn(1))]);
        app.set_overlay(true);
        assert_eq!(app.wheel(2), vec![Effect::Rows(RowKey::Pick(2))]);
        assert_eq!(app.wheel_push(), vec![Effect::Rows(RowKey::Editing(true))]);
        assert_eq!(
            press(&mut app, button::PLUS),
            vec![Effect::Rows(RowKey::Step(1))]
        );
        assert_eq!(
            press(&mut app, button::MINUS),
            vec![Effect::Rows(RowKey::Step(-1))]
        );
        // Another page: the screen closes it, so the wheel is the page's.
        app.open_tab(View::Arrange);
        app.open_tab(View::Play);
        assert_eq!(app.wheel(1), vec![Effect::Nav(Nav::Turn(1))]);
        // UNDO down and up, here and on the Looper tab.
        assert_eq!(
            app.button(92, true, 0.0),
            vec![Effect::Undo { redo: false }, Effect::UndoButton(true)]
        );
        assert_eq!(app.button(92, false, 0.7), vec![Effect::UndoButton(false)]);
        app.open_tab(View::Looper);
        let fx = app.button(92, true, 1.0);
        assert_eq!(fx.last(), Some(&Effect::UndoButton(true)));
        assert_eq!(fx.len(), 2, "the loop's undo, then the button: {fx:?}");
    }

    /// The Mix tab (Jay's card, 14 Nov 07:15Z): its tab opens the mixer as
    /// it was left, TRACK MIX opens it on Track Mix, and TRACK MIX on the
    /// tab (the mixer or an effect chain) goes back to the page before it.
    #[test]
    fn the_mix_tab_holds_the_mixer_and_its_chains_and_track_mix_goes_back() {
        use mpfree_mixer_ui::MixView;
        let mut app = ready();
        app.map.learn(crate::app::mapper::Job::TrackMix, Some(70));
        app.open_tab(View::Arrange);
        assert_eq!(View::Mixer.tab_index(false), Some(7));
        assert_eq!(View::Chain.tab_view(), Some(View::Mixer));
        assert_eq!(
            app.tab(7),
            vec![Effect::MixerLast, Effect::Show(View::Mixer)]
        );
        assert_eq!(app.tab(7), vec![], "a second tap changes nothing");
        // An effect's chain opened from the mixer is still the Mix tab.
        app.open_chain();
        assert_eq!(app.view(), View::Chain);
        let fx = app.button(70, true, 0.0);
        app.button(70, false, 0.0);
        assert!(fx.contains(&Effect::Show(View::Arrange)), "{fx:?}");
        assert_eq!(app.view(), View::Arrange);
        // From there TRACK MIX opens it on Track Mix; again, back.
        let fx = app.button(70, true, 0.0);
        app.button(70, false, 0.0);
        assert_eq!(fx.first(), Some(&Effect::Mixer(Some(MixView::Tracks))));
        let fx = app.button(70, true, 0.0);
        app.button(70, false, 0.0);
        assert_eq!(fx.first(), Some(&Effect::Mixer(None)), "{fx:?}");
        assert_eq!(app.view(), View::Arrange);
        // From a menu over a page, back goes to the page under it.
        app.open_tab(View::Synth);
        app.open_tab(View::Prefs);
        app.button(70, true, 0.0);
        app.button(70, false, 0.0);
        app.button(70, true, 0.0);
        app.button(70, false, 0.0);
        assert_eq!(app.view(), View::Synth);
    }

    #[test]
    fn the_sample_tab_comes_after_looper_and_takes_the_wheel_the_pads_and_undo() {
        let mut app = ready();
        assert_eq!(
            tabs(false),
            vec![
                View::Play,
                View::Sequencer,
                View::Looper,
                View::Sample,
                View::Synth,
                View::Arrange,
                View::Pads,
                View::Mixer
            ]
        );
        assert_eq!(View::Sample.tab_index(true), Some(3));
        assert_eq!(app.tab(3), vec![Effect::Show(View::Sample)]);
        assert_eq!(app.view(), View::Sample);
        // The wheel picks a row (RowKey); SHIFT + wheel picks the pad.
        assert_eq!(app.wheel(2), vec![Effect::Rows(RowKey::Pick(2))]);
        app.button(button::SHIFT, true, 0.0);
        assert_eq!(app.wheel(-3), vec![Effect::SamplePad(-1)]);
        // UNDO undoes the Sample page's edits there (SHIFT: redo), and the
        // time engine's takes elsewhere.
        assert_eq!(
            press(&mut app, 92),
            vec![Effect::SampleUndo { redo: true }, Effect::UndoButton(true)]
        );
        app.button(button::SHIFT, false, 0.0);
        assert_eq!(
            press(&mut app, 92),
            vec![Effect::SampleUndo { redo: false }, Effect::UndoButton(true)]
        );
        // - / + step the row picked.
        assert_eq!(
            press(&mut app, button::PLUS),
            vec![Effect::Rows(RowKey::Step(1))]
        );
        assert_eq!(
            press(&mut app, button::MINUS),
            vec![Effect::Rows(RowKey::Step(-1))]
        );
        // MENU opens the options and goes back to the Sample page.
        let fx = press(&mut app, MENU);
        assert!(matches!(&fx[0], Effect::Menu(m) if m.title == "Sample options"));
        press(&mut app, MENU);
        assert_eq!(app.view(), View::Sample);
        app.tab(0);
        // Back on Play, the screen picks the history (the Play view's).
        assert_eq!(
            press(&mut app, 92),
            vec![Effect::Undo { redo: false }, Effect::UndoButton(true)]
        );
    }

    /// The Pads tab has no browser of its own (Browse for opens BROWSE's,
    /// 18 Nov): the wheel picks its rows, - and + change them, and PLAY
    /// is the transport's, as on the Sample tab.
    #[test]
    fn the_pads_tab_follows_play_and_takes_the_sample_tabs_controls() {
        let mut app = ready();
        assert_eq!(View::Pads.tab_index(false), Some(6));
        assert_eq!(app.tab(6), vec![Effect::Show(View::Pads)]);
        assert!(View::Pads.sampler() && View::Sample.sampler() && !View::Play.sampler());
        app.map.data_push = Some(111);
        assert_eq!(app.wheel(2), vec![Effect::Rows(RowKey::Pick(2))]);
        assert_eq!(
            press(&mut app, button::PLUS),
            vec![Effect::Rows(RowKey::Step(1))]
        );
        assert!(press(&mut app, button::PLAY)
            .contains(&Effect::Time(TimeAction::Transport(Command::Play))));
        // UNDO undoes the pads' edits, as on Sample.
        assert_eq!(
            press(&mut app, 92),
            vec![Effect::SampleUndo { redo: false }, Effect::UndoButton(true)]
        );
        // Browse for asks for the browser over the page; asked again, it
        // stays.
        assert_eq!(app.open_browser(), vec![Effect::Show(View::Browser)]);
        assert!(app.open_browser().is_empty());
        assert_eq!(app.view(), View::Browser);
    }

    #[test]
    fn browse_opens_the_browser_over_the_page_and_shift_browse_saves() {
        let mut app = ready();
        // Not found yet: BROWSE does nothing.
        assert!(press(&mut app, 120).is_empty());
        app.map.browse = Some(120);
        app.map.erase = Some(121);
        app.map.data_push = Some(111);
        app.tab(View::Arrange.tab_index(false).unwrap());
        assert_eq!(app.view(), View::Arrange);
        assert_eq!(press(&mut app, 120), vec![Effect::Show(View::Browser)]);
        assert_eq!(View::Browser.tab_index(false), None);
        // The wheel, its push, SHIFT + wheel and ERASE are the browser's.
        assert_eq!(app.wheel(1), vec![Effect::Browse(BrowseKey::Wheel(1))]);
        assert_eq!(press(&mut app, 111), vec![Effect::Browse(BrowseKey::Push)]);
        assert_eq!(press(&mut app, 121), vec![Effect::Browse(BrowseKey::Erase)]);
        app.button(button::SHIFT, true, 0.0);
        assert_eq!(app.wheel(-2), vec![Effect::Browse(BrowseKey::Places(-1))]);
        // SHIFT + BROWSE saves, from here or any page, and stays.
        assert_eq!(press(&mut app, 120), vec![Effect::SaveProject]);
        app.button(button::SHIFT, false, 0.0);
        assert_eq!(app.view(), View::Browser);
        // BROWSE again: back to the page it opened over.
        assert_eq!(press(&mut app, 120)[0], Effect::Show(View::Arrange));
        app.button(button::SHIFT, true, 0.0);
        assert_eq!(press(&mut app, 120), vec![Effect::SaveProject]);
        app.button(button::SHIFT, false, 0.0);
        assert_eq!(app.view(), View::Arrange);
        // ERASE elsewhere is still ERASE.
        assert!(!press(&mut app, 121).contains(&Effect::Browse(BrowseKey::Erase)));
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
        let buffer = go(&mut app, Item::Buffer);
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
        // Outside the menu the Debug page's wheel has no job (it no
        // longer steps the kit: the PAD BANK buttons do).
        press(&mut app, MENU);
        assert_eq!(app.wheel(-2), Vec::<Effect>::new());
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
                View::Sequencer,
                View::Looper,
                View::Sample,
                View::Synth,
                View::Arrange,
                View::Pads,
                View::Mixer
            ]
        );
        // The same names, in the same order, as the bar every preview draws.
        let names: Vec<&str> = tabs(true).iter().map(|v| v.name()).collect();
        assert_eq!(names[..8], mpfree_ui::widgets::APP_TABS[..]);
        assert_eq!(names[8], mpfree_ui::widgets::DEV_TAB);
        assert_eq!(app.tab(8), vec![]);
        app.open_tab(View::Prefs);
        let row = go(&mut app, Item::MapControls);
        let fx = app.step_item(row, 1);
        assert!(app.settings.debug_mode);
        assert_eq!(app.view(), View::Map);
        assert!(fx.contains(&Effect::Note("developer tools: on".into())));
        // Every button is built in: only the ten spare Other rows are left.
        assert!(fx.iter().any(|e| matches!(e, Effect::Map(m)
            if m.rows[2].value == "button 81" && m.found == 32 && m.left == 10)));
        // Tap REC: MPFree listens; STOP stops listening without silencing.
        app.map_row(0);
        let fx = press(&mut app, button::STOP);
        assert!(!fx.contains(&Effect::StopSounds), "{fx:?}");
        assert!(fx
            .iter()
            .any(|e| matches!(e, Effect::Map(m) if !m.listening)));
        // REC is known from the start (button 73): pressed again it is
        // found again and nothing changes.
        assert_eq!(app.map.rec, Some(73));
        app.map_row(0);
        let fx = press(&mut app, 73);
        assert!(
            !fx.iter()
                .any(|e| matches!(e, Effect::Note(n) if n.starts_with("map: REC"))),
            "{fx:?}"
        );
        assert_eq!(app.map.rec, Some(73));
        // The wheel picks PAD BANK A/E and Clear forgets it; + listens, and
        // found again it works at once.
        let at = mapper::CONTROLS
            .iter()
            .position(|c| c.name == "PAD BANK A/E")
            .unwrap() as i32;
        app.wheel(at - app.mapper.selected as i32);
        app.map_button(2);
        assert_eq!(app.map.bank[0], None);
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
        // REC records what is armed.
        assert_eq!(
            press(&mut app, 73),
            vec![Effect::Rec(crate::rec_arm::Press::Rec)]
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
        assert_eq!(tabs(true)[8], View::Debug);
        app.tab(8);
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
        // REC on the Looper tab records what is armed (the screen arms
        // the loop there).
        press(&mut app, MENU);
        assert_eq!(app.view(), View::Looper);
        assert_eq!(
            press(&mut app, 90),
            vec![Effect::Rec(crate::rec_arm::Press::Rec)]
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
        // A mix, picked on the Looper tab: in time, so no latency; Prefs
        // says so, and steps back to the pads or the line input from it.
        let strip = mpfree_fx::StripId::Bus(2);
        let fx = app.loop_mix(strip);
        assert!(
            fx.contains(&source(LoopSource::Mix(strip))) && fx.contains(&latency(0)),
            "{fx:?}"
        );
        assert!(app.loop_mix(strip).is_empty(), "already that mix");
        assert_eq!(app.item_text(Item::LoopSource).1, "A mix (Looper tab)");
        let fx = app.step_item(0, 1);
        assert!(fx.contains(&source(LoopSource::Line)), "{fx:?}");
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

    /// R3 part 2b: PAD BANK pages a kit's 128 pads. Pad 5 on bank B is
    /// the kit's pad B5 (21) for the arp, the recorder and STEP SEQ, and a
    /// pad held while the page turns lets go of the pad it pressed.
    #[test]
    fn pad_bank_pages_the_kit_and_a_held_pad_lets_go_where_it_pressed() {
        let mut app = ready();
        assert_eq!((app.pad_page(), app.kit_pad(4)), (0, 4));
        assert_eq!(
            (kit_pad_name(0), kit_pad_name(20), kit_pad_name(127)),
            ("A1".into(), "B5".into(), "H16".into())
        );
        app.set_pad_page(1);
        assert_eq!(app.kit_pad(4), 20);
        assert_eq!(
            app.pressed_event(4, 100, true),
            Some(mpfree_play::Event::NoteOn {
                note: 20,
                velocity: 100
            })
        );
        app.pad_down(4, 100);
        assert!(app.eng().active().contains(20) && !app.eng().active().contains(4));
        assert!(app.active(4), "pad 5 lights for B5");
        // The page turns while it is down: it still lets go of B5.
        app.set_pad_page(0);
        assert_eq!(app.kit_pad(4), 20, "held: the pad it pressed");
        assert_eq!(
            app.pressed_event(4, 0, false),
            Some(mpfree_play::Event::NoteOff { note: 20 })
        );
        app.pad_up(4);
        assert!(app.eng().active().is_empty());
        assert_eq!(app.kit_pad(4), 4, "let go: bank A's again");
        // STEP SEQ's sound is the pad played last, on any bank.
        app.set_pad_page(7);
        app.pad_down(15, 100);
        app.pad_up(15);
        assert_eq!(app.step_pads(), None);
        let fx = click(&mut app, button::STEP_SEQ);
        assert_eq!(app.step_pads().map(|s| s.sound), Some(127));
        assert!(fx.contains(&Effect::Note(
            "STEP SEQ: the pads are pad H16's steps, page 1".into()
        )));
        // The page never goes past H.
        app.set_pad_page(9);
        assert_eq!(app.pad_page(), 7);
    }

    #[test]
    fn the_guide_learns_buttons_by_name() {
        let mut app = ready();
        app.map = ButtonMap::default();
        app.open_tab(View::Prefs);
        let find = go(&mut app, Item::FindButtons);
        let fx = app.step_item(find, 1);
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
        let find = go(&mut app, Item::FindButtons);
        let fx = app.step_item(find, 1);
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
            vec![Effect::Nav(Nav::Turn(1))],
            "the wheel moves the outline again"
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
    fn shift_and_the_wheel_jump_through_the_pattern_groups() {
        let mut app = ready();
        app.load_patterns("Skip = 1 . 3 2 . 3 x:2 .\n");
        app.toggle_panel();
        app.arp_action(ArpAction::Mode(Mode::Pattern));
        assert_eq!(app.eng().settings().mode, Mode::Pattern);
        let groups = mpfree_arp::groups::starts();
        app.wheel(3);
        assert_eq!(app.pattern_index, 3);
        app.button(button::SHIFT, true, 0.0);
        // On: the next group's first pattern, each turn a group.
        app.wheel(1);
        assert_eq!(app.pattern_index, groups[1].1);
        assert_eq!(app.patterns[app.pattern_index].0, "Offbeats");
        app.wheel(1);
        app.wheel(1);
        assert_eq!(app.pattern_index, groups[3].1);
        assert_eq!(*app.eng().pattern(), app.patterns[groups[3].1].1);
        // Back: from inside a group to its first pattern, then the group
        // before.
        app.button(button::SHIFT, false, 0.0);
        app.wheel(2);
        app.button(button::SHIFT, true, 0.0);
        app.wheel(-1);
        assert_eq!(app.pattern_index, groups[3].1);
        app.wheel(-1);
        assert_eq!(app.pattern_index, groups[2].1);
        // The owner's patterns are the last group; past it nothing moves.
        for _ in 0..20 {
            app.wheel(1);
        }
        assert_eq!(app.patterns[app.pattern_index].0, "Skip");
        assert!(app.wheel(1).is_empty());
        for _ in 0..20 {
            app.wheel(-1);
        }
        assert_eq!(app.pattern_index, 0);
        assert!(app.wheel(-1).is_empty());
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
                "Swing from",
                "Modifier",
                "Latch keeps pads",
                "Sustain pedal latches"
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
            matches!(app.menu_view(), Effect::Menu(m) if m.title == "Play options" && m.rows.len() == PLAY_OPTIONS.len())
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

    /// The Modifier (MPC 3.10.1 guide pp. 83-84): a keyboard's mod wheel
    /// pushes the armed track's arp by the amounts its options set, from
    /// the next step, and the screens hear the push; other controllers,
    /// and the wheel with another source, push nothing.
    #[test]
    fn the_mod_wheel_pushes_the_armed_tracks_arp() {
        use mpfree_node::Ctl;
        let mut app = ready();
        app.toggle_panel();
        app.arp_action(ArpAction::Mode(Mode::Arp));
        assert_eq!(app.arp.modifier.source, ModSource::Off);
        app.arp_controller(Ctl::ModWheel, 1.0);
        assert_eq!(app.eng().live().rate, Rate::Sixteenth, "off: no push");
        // MENU: the Modifier's row, stepped to the mod wheel.
        app.step_arp_item(ArpItem::ModSource, 1);
        assert_eq!(app.arp.modifier.source, ModSource::ModWheel);
        assert_eq!(app.eng().settings().modifier.source, ModSource::ModWheel);
        app.arp_poll();
        app.arp_controller(Ctl::Breath, 1.0);
        app.arp_controller(Ctl::Sustain, 1.0);
        assert_eq!(app.eng().modifier(), 0.0);
        assert!(app.arp_poll().is_none(), "nothing moved");
        app.arp_controller(Ctl::ModWheel, 1.0);
        // The defaults: two rates faster and an octave more.
        let live = app.eng().live();
        assert_eq!((live.rate, live.octaves), (Rate::ThirtySecond, 2));
        assert_eq!(app.arp.rate, Rate::Sixteenth, "the setting stays");
        let Some(Effect::Arp(v)) = app.arp_poll() else {
            panic!("the screens hear the push");
        };
        assert_eq!((v.push, v.live.rate), (1.0, Rate::ThirtySecond));
        app.arp_controller(Ctl::ModWheel, 0.0);
        assert_eq!(app.eng().live(), app.eng().settings(), "let back");
        // The rows say what they push, in words.
        assert_eq!(
            app.arp_item_text(ArpItem::ModRate),
            ("Pushes rate", "+2 steps faster".to_string())
        );
        app.step_arp_item(ArpItem::ModOctaves, -1);
        app.step_arp_item(ArpItem::ModOctaves, -1);
        assert_eq!(app.arp_item_text(ArpItem::ModOctaves).1, "-1 octave");
        app.step_arp_item(ArpItem::ModGate, -1);
        assert_eq!(app.arp_item_text(ArpItem::ModGate).1, "-5 %");
        for _ in 0..12 {
            app.step_arp_item(ArpItem::ModRolls, 1);
        }
        assert_eq!(app.arp_item_text(ArpItem::ModRolls).1, "+7 hits a step");
        app.step_arp_item(ArpItem::ModAccents, 1);
        assert_eq!(app.arp_item_text(ArpItem::ModAccents).1, "10 % of steps");
        assert!(app.take_project_edit(), "the Modifier is the song's");
    }

    /// A knob MIDI Learn gives the Modifier (`mpfree_param::arp::MODIFIER`)
    /// pushes its track's arp while the Modifier's controller is Learned
    /// knob, and reads back where it is.
    #[test]
    fn a_learned_knob_pushes_the_modifier() {
        use mpfree_play::mpfree_param::arp as ids;
        let mut app = ready();
        let (info, v) = app.arp_param(0, ids::MODIFIER).unwrap();
        assert_eq!((info.name, v), ("Modifier", 0.0));
        assert!(app.set_arp_param(0, ids::MODIFIER, 100.0).is_empty());
        assert_eq!(app.eng().modifier(), 0.0, "not the Modifier's controller");
        for _ in 0..5 {
            app.step_arp_item(ArpItem::ModSource, 1);
        }
        assert_eq!(app.arp.modifier.source, ModSource::Knob);
        app.set_arp_param(0, ids::MODIFIER, 50.0);
        assert_eq!(app.eng().modifier(), 0.5);
        assert_eq!(app.arp_param(0, ids::MODIFIER).unwrap().1, 50.0);
        // Another track's knob: that track's arp.
        assert_eq!(app.arp_param(2, ids::MODIFIER).unwrap().1, 0.0);
        assert!(app
            .arp_param(mpfree_seq::MAX_TRACKS, ids::MODIFIER)
            .is_none());
    }

    /// The XY pad's hand on a track's arp (`App::arp_hand`, `arp_beneath`,
    /// `keep_arp`): the pad's gate plays and the gate as set stays, the
    /// push (in percent) pushes even with no controller picked, a Q-Link
    /// turn takes the gate from the pad, Stay sets the gate as a learned
    /// control does, and another track's arp is its own.
    #[test]
    fn the_xy_pads_hand_reaches_a_tracks_arp() {
        use mpfree_play::mpfree_param::arp as ids;
        let mut app = ready();
        app.toggle_panel();
        app.arp_action(ArpAction::Mode(Mode::Arp));
        let set = app.eng().settings();
        assert!(app.arp_hand(0, ids::GATE, Some(90.0)));
        assert_eq!(app.eng().live().gate, 90);
        assert_eq!(app.arp_param(0, ids::GATE).unwrap().1, f32::from(set.gate));
        assert_eq!(
            app.arp_beneath(0, ids::GATE),
            Some((f32::from(set.gate), f32::from(set.gate)))
        );
        let Some(Effect::Arp(v)) = app.arp_poll() else {
            panic!("the panel hears what plays");
        };
        assert_eq!((v.live.gate, v.settings.gate), (90, set.gate));
        // The push, in percent: the pad is the controller.
        assert!(app.arp_hand(0, ids::MODIFIER, Some(100.0)));
        assert_eq!(app.eng().live().rate, Rate::ThirtySecond);
        assert_eq!(app.arp_beneath(0, ids::MODIFIER), Some((0.0, 0.0)));
        app.arp_hand(0, ids::MODIFIER, None);
        assert_eq!(app.eng().live().rate, set.rate);
        // A Q-Link turn of the gate takes it from the pad.
        app.qlink(1, 0, Some(20)).expect("the panel takes Q-Link 2");
        let turned = app.eng().settings().gate;
        assert_ne!(turned, set.gate);
        assert_eq!(app.eng().live().gate, turned);
        app.arp_hand(0, ids::GATE, Some(95.0));
        assert_eq!(app.eng().live().gate, turned, "kept until the pad lets go");
        app.arp_hand(0, ids::GATE, None);
        // Stay: the pad's step as set.
        app.arp_hand(0, ids::GATE, Some(70.4));
        assert_eq!(app.keep_arp(0, ids::GATE, 70.4).unwrap().0, 70.0);
        assert_eq!(app.arp.gate, 70);
        app.arp_hand(0, ids::GATE, None);
        assert_eq!(app.eng().live().gate, 70);
        assert!(app.eng().hand().is_empty());
        // Another track's arp, and what isn't one.
        assert!(app.arp_hand(2, ids::OCTAVES, Some(3.0)));
        assert_eq!(app.arps.engine_at(2).live().octaves, 3);
        assert_eq!(app.eng().live().octaves, set.octaves);
        assert!(!app.arp_hand(0, 99, Some(1.0)));
        assert!(!app.arp_hand(mpfree_seq::MAX_TRACKS, ids::GATE, Some(1.0)));
        assert!(app.keep_arp(0, 99, 1.0).is_none());
    }

    /// A tap on the Modifier box opens the arp's options on its row; the
    /// pedal's and Latch's rows are there too.
    #[test]
    fn the_modifier_box_opens_its_options() {
        let mut app = ready();
        app.toggle_panel();
        app.arp_action(ArpAction::Modifier);
        assert_eq!(app.view(), View::Options);
        assert_eq!(app.items(), arp::MOD_ROWS);
        assert_eq!(app.items()[app.selected], Item::Arp(ArpItem::ModSource));
        let Effect::Menu(m) = app.menu_view() else {
            panic!()
        };
        assert_eq!(m.title, "Arp options · Modifier");
        // MENU (or Back): the arp options, on the Modifier's row, with
        // the pedal's and Latch's rows.
        press(&mut app, MENU);
        assert_eq!(app.view(), View::Options);
        assert_eq!(app.items(), arp::ARP_OPTIONS);
        assert_eq!(app.items()[app.selected], Item::Arp(ArpItem::ModSection));
        assert_eq!(app.item_text(Item::Arp(ArpItem::ModSection)).1, "Off");
        assert!(app.row_of(Item::PedalLatch).is_some());
        assert!(app.row_of(Item::LatchMode).is_some());
        // Its arrow opens the section again; Back leaves it.
        let at = app.selected;
        app.step_item(at, 1);
        assert_eq!(app.items(), arp::MOD_ROWS);
        app.options_button();
        assert_eq!(app.items(), arp::ARP_OPTIONS);
        // MENU closes the options as before; opened again, they start
        // at the top, not in the section.
        app.arp_action(ArpAction::Modifier);
        press(&mut app, MENU);
        press(&mut app, MENU);
        assert_eq!(app.view(), View::Arp);
        press(&mut app, MENU);
        assert_eq!(app.items(), arp::ARP_OPTIONS);
    }

    /// The sustain pedal latches the arp (the MPC's Sustain Pedal Latches,
    /// 3.10.1 guide p. 83) only with Prefs' row on and the arp playing;
    /// it comes up where it went down.
    #[test]
    fn the_sustain_pedal_latches_the_arp_when_prefs_say_so() {
        use mpfree_node::Ctl;
        let mut app = ready();
        app.toggle_panel();
        app.arp_action(ArpAction::Mode(Mode::Arp));
        // Off in Prefs: the sound's pedal, as before.
        assert!(app.arp_pedal(Ctl::Sustain, 1.0).is_none());
        assert!(app.arp_pedal(Ctl::Sustain, 0.0).is_none());
        // The arp's options have the row (the arp plays on under them).
        press(&mut app, MENU);
        let at = app.row_of(Item::PedalLatch).unwrap();
        app.step_item(at, 1);
        assert!(app.settings.pedal_latches);
        assert_eq!(app.item_text(Item::PedalLatch).1, "On");
        // Other controllers aren't the pedal.
        assert!(app.arp_pedal(Ctl::ModWheel, 1.0).is_none());
        // Down: Latch on; a half pedal's next values stay with the arp.
        let fx = app.arp_pedal(Ctl::Sustain, 1.0).expect("the arp's");
        assert!(fx.contains(&Effect::Note("Latch on (sustain pedal)".into())));
        assert!(app.latched);
        assert_eq!(app.eng().latch(), app.settings.latch_mode);
        assert_eq!(app.arp_pedal(Ctl::Sustain, 0.8), Some(Vec::new()));
        app.pad_down(0, 100);
        app.pad_up(0);
        assert!(!app.eng().active().is_empty(), "latched by the pedal");
        // Up: Latch as it was, and the notes let go stop.
        assert!(app.arp_pedal(Ctl::Sustain, 0.0).is_some());
        assert!(!app.latched);
        assert!(app.eng().active().is_empty());
        // Latched already: the pedal leaves it latched.
        app.toggle_latch();
        app.arp_pedal(Ctl::Sustain, 1.0).unwrap();
        app.arp_pedal(Ctl::Sustain, 0.0).unwrap();
        assert!(app.latched);
        app.toggle_latch();
        // Down to the sound (Direct), then the arp plays: the pedal's
        // values stay the sound's until it comes up.
        app.arp_action(ArpAction::Mode(Mode::Direct));
        assert!(app.arp_pedal(Ctl::Sustain, 1.0).is_none());
        app.arp_action(ArpAction::Mode(Mode::Arp));
        assert!(app.arp_pedal(Ctl::Sustain, 0.9).is_none());
        assert!(app.arp_pedal(Ctl::Sustain, 0.0).is_none());
        assert!(!app.latched);
        // Another track armed under the pedal: its latch lets go there.
        app.arp_pedal(Ctl::Sustain, 1.0).unwrap();
        app.set_track(2);
        app.set_track(0);
        assert!(!app.latched, "let go on the old track");
        app.set_track(2);
        assert!(
            app.arp_pedal(Ctl::Sustain, 0.0).is_none(),
            "the new track's"
        );
        // Prefs turned off under a held pedal lets go too.
        app.set_track(0);
        app.arp_pedal(Ctl::Sustain, 1.0).unwrap();
        app.show(View::Prefs);
        let at = app.row_of(Item::PedalLatch).unwrap();
        app.step_item(at, -1);
        assert!(!app.latched);
    }

    /// A keyboard's aftertouch on the keys held reaches the arp while the
    /// pads play notes, so a Modifier on pressure follows it.
    #[test]
    fn key_pressure_pushes_a_modifier_on_pressure() {
        use mpfree_arp::scale::pads::{layout, PadLayout, PadSetup};
        let mut app = ready();
        app.set_arp_layout(Some(layout(&PadSetup {
            layout: PadLayout::Chromatic,
            ..PadSetup::default()
        })));
        app.toggle_panel();
        app.arp_action(ArpAction::Mode(Mode::Arp));
        app.step_arp_item(ArpItem::ModSource, 1);
        app.step_arp_item(ArpItem::ModSource, 1);
        assert_eq!(app.arp.modifier.source, ModSource::Pressure);
        app.key_pressure(61, 127);
        assert_eq!(app.eng().modifier(), 0.0, "a key not held");
        app.key_down(61, 90);
        app.key_pressure(61, 127);
        assert_eq!(app.eng().modifier(), 1.0);
        app.key_up(61);
        assert_eq!(app.eng().modifier(), 0.0, "let go: nothing pressed");
    }

    /// The manual's drum example: Note Repeat on a pad, the Modifier on
    /// Pressure adding rolls with no rate push. Pressing harder into the
    /// pad rolls it; easing off, or letting go, takes the rolls back. The
    /// rate stays as set throughout.
    #[test]
    fn pad_pressure_rolls_note_repeat() {
        let mut app = ready();
        app.toggle_panel();
        app.arp_action(ArpAction::Mode(Mode::Repeat));
        app.step_arp_item(ArpItem::ModSource, 1);
        app.step_arp_item(ArpItem::ModSource, 1);
        assert_eq!(app.arp.modifier.source, ModSource::Pressure);
        for _ in 0..2 {
            app.step_arp_item(ArpItem::ModRate, -1);
        }
        app.step_arp_item(ArpItem::ModOctaves, -1);
        for _ in 0..3 {
            app.step_arp_item(ArpItem::ModRolls, 1);
        }
        let m = app.arp.modifier;
        assert_eq!((m.rate, m.octaves, m.rolls), (0, 0, 3));
        app.pressure(2, 127);
        assert_eq!(app.eng().modifier(), 0.0, "a pad not held presses nothing");
        app.pad_down(2, 100);
        app.pressure(2, 127);
        assert_eq!(app.eng().modifier(), 1.0);
        assert_eq!(app.eng().rolls(), 3);
        assert_eq!(app.eng().live().rate, app.arp.rate, "the rate stays");
        app.pressure(2, 20);
        assert_eq!(app.eng().rolls(), 0, "eased off");
        app.pressure(2, 127);
        assert_eq!(app.eng().rolls(), 3);
        app.pad_up(2);
        assert_eq!(app.eng().modifier(), 0.0, "let go: nothing pressed");
        assert_eq!(app.eng().rolls(), 0);
    }

    /// Prefs' Latch keeps pads steps through Replace, Toggle and Add, and
    /// both latch rows come back at the next start.
    #[test]
    fn latch_add_and_the_pedal_latch_are_kept() {
        let mut app = ready();
        app.show(View::Prefs);
        let at = app.row_of(Item::LatchMode).unwrap();
        app.step_item(at, 1);
        app.step_item(at, 1);
        assert_eq!(app.settings.latch_mode, LatchMode::Add);
        assert_eq!(app.item_text(Item::LatchMode).1, "Add");
        app.step_item(at, 1);
        assert_eq!(app.settings.latch_mode, LatchMode::Add, "the last");
        app.settings.pedal_latches = true;
        let kept = app.app_settings();
        assert_eq!(kept.get_str("latch-keeps-pads"), Some("add"));
        let mut next = App::new(44_100, false);
        next.restore(&kept, &mpfree_store::Settings::default());
        assert_eq!(next.settings.latch_mode, LatchMode::Add);
        assert!(next.settings.pedal_latches);
    }

    /// The Modifier is kept with the project, per track; a push out of
    /// range is said and left at its default.
    #[test]
    fn the_modifier_is_kept_with_the_project() {
        let mut app = ready();
        app.step_arp_item(ArpItem::ModSource, 1);
        app.step_arp_item(ArpItem::ModRolls, 1);
        app.step_arp_item(ArpItem::ModRolls, 1);
        let part = app.arp_part();
        for line in [
            "modifier = modwheel",
            "modifier-push = rate 2 octaves 1 gate 0 swing 0 rolls 2 accents 0",
        ] {
            assert!(part.lines().any(|l| l == line), "{line}: {part}");
        }
        let mut next = ready();
        let (notes, _) = next.open_arp(Some(&part));
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(next.arp.modifier, app.arp.modifier);
        assert_eq!(next.eng().settings().modifier, app.arp.modifier);
        let (notes, _) = next.open_arp(Some(
            "mpfree-arp 1\ntrack 1\nmodifier = knob\nmodifier-push = rate 9\n",
        ));
        assert!(
            notes[0].contains("track 1 modifier-push = rate 9"),
            "{notes:?}"
        );
        assert_eq!(next.arp.modifier.source, ModSource::Knob);
        assert_eq!(next.arp.modifier.rate, 2, "left at its default");
    }

    /// MIDI Learn turns a track's arp as its Q-Links do: the armed
    /// track's through the panel, another track's in place, which the
    /// panel shows once that track is armed.
    #[test]
    fn a_learned_control_turns_a_tracks_arp() {
        use mpfree_play::mpfree_param::arp as ids;
        let mut app = ready();
        let (info, v) = app.arp_param(0, ids::GATE).unwrap();
        assert_eq!((info.name, v), ("Gate", 50.0));
        let (rate, v) = app.arp_param(0, ids::RATE).unwrap();
        assert_eq!(mpfree_mod::text::value_text(&rate, v), "1/16");
        assert!(app.arp_param(0, 9).is_none());
        assert!(app.set_arp_param(0, 9, 1.0).is_empty());
        // The armed track: as Q-Link 2 sets it, and the panel hears.
        assert!(!app.set_arp_param(0, ids::GATE, 80.0).is_empty());
        assert_eq!((app.arp.gate, app.eng().settings().gate), (80, 80));
        app.set_arp_param(0, ids::RATE, 1.0);
        assert_eq!(app.settings.rate, Rate::Eighth);
        assert!(
            app.set_arp_param(0, ids::RATE, 1.2).is_empty(),
            "the same step"
        );
        app.set_arp_param(0, ids::OCTAVES, 9.0);
        assert_eq!(app.arp.octaves, 4, "kept to the range");
        assert!(app.take_project_edit());
        // Track 3, not armed: its engine plays on with it.
        assert!(app.set_arp_param(2, ids::SWING, 60.0).is_empty());
        assert_eq!(app.arps.engine_at(2).settings().swing, 60);
        assert_eq!(app.arp_param(2, ids::SWING).unwrap().1, 60.0);
        assert_eq!(app.arp.swing, 50, "the armed track's arp is as it was");
        assert!(app.take_project_edit());
        app.set_track(2);
        assert_eq!((app.arp.swing, app.arp.gate), (60, 50));
        assert!(!app.swing_from_transport, "a turned swing is the arp's own");
    }

    #[test]
    fn the_arps_go_in_the_project_and_an_open_puts_them_back() {
        let mut app = ready();
        assert!(!app.take_project_edit(), "nothing changed yet");
        // Track 1: Arp mode at 1/8, Down. Then track 3: D Dorian (or the
        // second scale), its own swing, latched.
        app.toggle_panel();
        app.arp_action(ArpAction::Mode(Mode::Arp));
        app.arp_action(ArpAction::Rate(Rate::Eighth));
        app.arp_action(ArpAction::Order(Order::Down));
        assert!(app.take_project_edit(), "an arp change is an edit");
        app.set_track(2);
        assert!(!app.take_project_edit(), "arming a track isn't");
        app.step_arp_item(ArpItem::Scale, 1);
        app.step_arp_item(ArpItem::Root, 1);
        app.step_arp_item(ArpItem::Root, 1);
        app.step_arp_item(ArpItem::SwingFrom, -1);
        app.arp_action(ArpAction::Rate(Rate::SixteenthTriplet));
        app.arp_action(ArpAction::Latch);
        assert!(app.take_project_edit());
        let part = app.arp_part();
        assert!(
            part.starts_with("mpfree-arp 1\ntrack 1\nmode = arp\n"),
            "{part}"
        );
        for line in [
            "rate = 1/8",
            "order = down",
            "root = D",
            "swing-from = arp",
            "rate = 1/16T",
            "track 3",
        ] {
            assert!(part.lines().any(|l| l == line), "{line}: {part}");
        }
        // Tracks left at the default aren't listed (a project can have
        // 128); an open gives them the default.
        for line in ["track 2", "track 16", "track 128"] {
            assert!(!part.lines().any(|l| l == line), "{line}: {part}");
        }

        // Another start opens it: every track's arp as it was, Latch off.
        let mut next = ready();
        let (notes, fx) = next.open_arp(Some(&part));
        assert!(notes.is_empty(), "{notes:?}");
        assert!(fx.iter().any(|e| matches!(e, Effect::Arp(_))));
        assert_eq!(next.arp_fingerprint(), app.arp_fingerprint());
        assert!(!next.take_project_edit(), "an open isn't an edit");
        assert!(!next.latched);
        // Track 3 isn't armed there: its arp plays as it did, at 1/16T.
        let (a, b) = (
            app.arps.engine_at(2).settings(),
            next.arps.engine_at(2).settings(),
        );
        assert_eq!(
            (b.mode, b.rate, b.scale),
            (a.mode, Rate::SixteenthTriplet, a.scale)
        );
        assert!(b.scale.is_some_and(|s| s.root == 2));
        assert_eq!(next.arps.engine_at(2).latch(), LatchMode::Off);
        // The armed track (1) takes its own back.
        assert_eq!(
            (next.arp_mode, next.settings.rate, next.arp.order),
            (Mode::Arp, Rate::Eighth, Order::Down)
        );
        next.set_track(2);
        assert_eq!(next.scale_root, 2);
        assert!(!next.swing_from_transport);

        // New: every arp at its default, not the last project's.
        next.open_arp(None);
        assert_eq!(next.arp_fingerprint(), ready().arp_fingerprint());

        // Words not understood are said and left at their defaults; a part
        // that isn't one is refused whole.
        let (notes, _) = next.open_arp(Some(
            "mpfree-arp 1\ntrack 2\ngate = 300\nscale = Nope\nmode = arp\n",
        ));
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(
            notes[0].contains("track 2 gate = 300") && notes[0].contains("track 2 scale = Nope"),
            "{notes:?}"
        );
        next.set_track(1);
        assert_eq!((next.arp_mode, next.arp.gate), (Mode::Arp, 50));
        let (notes, _) = next.open_arp(Some("junk"));
        assert!(
            notes[0].starts_with("project: the arps weren't opened"),
            "{notes:?}"
        );
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

    /// MIDI Learn's remote control presses the MPC's buttons as they are,
    /// on the page showing; never while Map controls listens, which would
    /// take a press for the MPC's own button.
    #[test]
    fn a_learned_control_presses_the_mpcs_transport_buttons() {
        use mpfree_play::mpfree_param::transport as tr;
        let mut app = ready();
        let cmd = |fx: Vec<Effect>| {
            fx.into_iter().find_map(|e| match e {
                Effect::Time(TimeAction::Transport(c)) => Some(c),
                _ => None,
            })
        };
        assert_eq!(
            cmd(app.remote_press(tr::PLAY, true, 1.0)),
            Some(Command::Play)
        );
        app.remote_press(tr::PLAY, false, 1.1);
        app.set_rolling(true);
        assert!(app.rolling());
        assert_eq!(
            cmd(app.remote_press(tr::STOP, true, 2.0)),
            Some(Command::Stop)
        );
        app.remote_press(tr::STOP, false, 2.1);
        app.set_rolling(false);
        assert_eq!(
            cmd(app.remote_press(tr::PLAY_START, true, 3.0)),
            Some(Command::PlayStart)
        );
        app.remote_press(tr::PLAY_START, false, 3.1);
        // REC on the Play page, as the MPC's own REC there.
        assert_eq!(
            app.remote_press(tr::REC, true, 4.0),
            vec![Effect::Rec(crate::rec_arm::Press::Rec)]
        );
        app.remote_press(tr::REC, false, 4.1);
        for k in 0..4 {
            let t = 10.0 + k as f64 * 0.5;
            app.remote_press(tr::TAP_TEMPO, true, t);
            app.remote_press(tr::TAP_TEMPO, false, t + 0.1);
        }
        assert_eq!(app.bpm(), 120.0);
        // The tempo is no button.
        assert!(app.remote_press(tr::TEMPO, true, 20.0).is_empty());
        app.view = View::Map;
        assert!(app.remote_press(tr::PLAY, true, 30.0).is_empty());
    }

    /// Live test 3: taps on Play's tempo tap the tempo, and say so.
    #[test]
    fn taps_on_plays_tempo_set_it() {
        let mut app = ready();
        let fx = app.tap_on_screen(10.0);
        assert!(fx
            .iter()
            .any(|e| matches!(e, Effect::Tell(t) if t.contains("keep tapping"))));
        app.tap_on_screen(10.5);
        app.tap_on_screen(11.0);
        let fx = app.tap_on_screen(11.5);
        assert_eq!(app.settings.bpm, 120.0);
        assert!(fx
            .iter()
            .any(|e| matches!(e, Effect::Tell(t) if t == "tempo: 120.0 BPM (tapped)")));
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

    /// The transport buttons' lights follow the transport and the loops
    /// as Akai's app lights them (live test 3 capture), only when they
    /// change: PLAY steady while rolling; REC and OVERDUB blink 3/0 while
    /// armed (PLAY blinking with them) and while recording; UNDO blinks
    /// 3/1 while recording.
    #[test]
    fn the_transport_lights_follow_play_record_and_overdub_as_akais_do() {
        const REC: u8 = 73;
        const OVERDUB: u8 = 80;
        const UNDO: u8 = 67;
        let mut app = ready();
        let lights = |rolling, recording, overdub| TransportLights {
            rolling,
            recording,
            overdub,
        };
        // Stopped, as Akai's startup left them: nothing to send.
        app.map.learn(mapper::Job::Rec, None);
        app.map.learn(mapper::Job::Overdub, None);
        app.map.learn(mapper::Job::Undo, None);
        assert!(app
            .transport_lights(lights(false, false, false), 0.0)
            .is_empty());
        // Playing: PLAY bright, STOP dim.
        assert_eq!(
            app.transport_lights(lights(true, false, false), 1.0),
            vec![
                Effect::Light(button::STOP, LIGHT_OFF),
                Effect::Light(button::PLAY, LIGHT_ON),
            ]
        );
        assert!(app
            .transport_lights(lights(true, false, false), 1.4)
            .is_empty());
        app.transport_lights(lights(false, false, false), 2.0);
        app.map.learn(mapper::Job::Rec, Some(REC));
        app.map.learn(mapper::Job::Overdub, Some(OVERDUB));
        app.map.learn(mapper::Job::Undo, Some(UNDO));
        // REC armed while stopped: REC and PLAY blink 3/0 together.
        let fx = app.transport_lights(lights(false, true, false), 3.0);
        assert!(
            fx.contains(&Effect::Light(button::PLAY, LIGHT_ON)),
            "{fx:?}"
        );
        assert!(fx.contains(&Effect::Light(REC, LIGHT_ON)), "{fx:?}");
        assert!(fx.contains(&Effect::Light(OVERDUB, LIGHT_DARK)), "{fx:?}");
        let fx = app.transport_lights(lights(false, true, false), 3.0 + BLINK_HALF_SECONDS);
        assert_eq!(
            fx,
            vec![
                Effect::Light(button::PLAY, LIGHT_DARK),
                Effect::Light(REC, LIGHT_DARK)
            ]
        );
        // Recording: PLAY steady, REC still blinking, UNDO blinking 3/1.
        let fx = app.transport_lights(lights(true, true, false), 3.0 + 3.0 * BLINK_HALF_SECONDS);
        assert!(
            fx.contains(&Effect::Light(button::PLAY, LIGHT_ON)),
            "{fx:?}"
        );
        assert!(fx.contains(&Effect::Light(UNDO, LIGHT_OFF)), "{fx:?}");
        assert!(
            !fx.contains(&Effect::Light(REC, LIGHT_ON)),
            "REC on its dark half"
        );
        let fx = app.transport_lights(lights(true, true, false), 3.0 + 4.0 * BLINK_HALF_SECONDS);
        assert_eq!(
            fx,
            vec![Effect::Light(REC, LIGHT_ON), Effect::Light(UNDO, LIGHT_ON)]
        );
        // STOP: everything back, UNDO dim.
        let fx = app.transport_lights(lights(false, false, false), 9.0);
        for want in [
            Effect::Light(button::STOP, LIGHT_ON),
            Effect::Light(button::PLAY, LIGHT_DARK),
            Effect::Light(REC, LIGHT_DARK),
            Effect::Light(UNDO, LIGHT_OFF),
        ] {
            assert!(fx.contains(&want), "{want:?} in {fx:?}");
        }
        // Overdub armed: OVERDUB and PLAY blink.
        let fx = app.transport_lights(lights(false, false, true), 9.0);
        assert!(fx.contains(&Effect::Light(OVERDUB, LIGHT_ON)), "{fx:?}");
        assert!(
            fx.contains(&Effect::Light(button::PLAY, LIGHT_ON)),
            "{fx:?}"
        );
        // Idle, UNDO is bright while the page has something to undo
        // (inferred from Akai's), and dim again when it has not.
        app.transport_lights(lights(false, false, false), 10.0);
        app.set_undo_ready(true);
        assert_eq!(
            app.transport_lights(lights(false, false, false), 10.1),
            vec![Effect::Light(UNDO, LIGHT_ON)]
        );
        app.set_undo_ready(false);
        assert_eq!(
            app.transport_lights(lights(false, false, false), 10.2),
            vec![Effect::Light(UNDO, LIGHT_OFF)]
        );
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

    /// SHIFT + SAMPLER pressed again on the Looper tab: the pads are the
    /// loop keys (the Looper as an instrument, step 2), on the Looper tab
    /// and Play and not elsewhere, until SHIFT + SAMPLER again. One pad
    /// mode at a time: the loop keys take the pads from TRACK MUTE and
    /// STEP SEQ, and give them up to either.
    #[test]
    fn shift_sampler_again_on_the_looper_tab_makes_the_pads_loop_keys() {
        const SHIFT: u8 = button::SHIFT;
        const SAMPLER: u8 = button::SAMPLER;
        const STEP_SEQ: u8 = button::STEP_SEQ;
        let mut app = ready();
        // SHIFT + SAMPLER elsewhere opens the Looper tab and leaves the
        // pads as they were.
        held(&mut app, SHIFT, |a| press(a, SAMPLER));
        assert_eq!(app.view(), View::Looper);
        assert!(!app.pads_are_loop_keys());
        // Again, there: the loop keys.
        let fx = held(&mut app, SHIFT, |a| press(a, SAMPLER));
        assert!(app.pads_are_loop_keys());
        assert!(app.status_now().loop_keys);
        assert!(
            fx.iter()
                .any(|e| matches!(e, Effect::Note(n) if n.contains("the pads are the loop keys"))),
            "{fx:?}"
        );
        assert_eq!(
            app.perform_page().action(12),
            crate::perform_pads::PadAction::LoopKey(0),
            "pad 13 is Loop 1's key"
        );
        // On Play they stay keys; on Sample the pads play, and back on
        // the Looper tab they are keys again.
        app.open_tab(View::Play);
        assert!(app.pads_are_loop_keys());
        app.open_tab(View::Sample);
        assert!(!app.pads_are_loop_keys());
        app.open_tab(View::Looper);
        assert!(app.pads_are_loop_keys());
        // TRACK MUTE takes the pads, and the loop keys go off.
        const TRACK_MUTE: u8 = 68;
        app.map.learn(mapper::Job::TrackMute, Some(TRACK_MUTE));
        press(&mut app, TRACK_MUTE);
        assert_eq!(app.mute_keys(), MuteKeys::Tracks);
        assert!(!app.pads_are_loop_keys());
        // The loop keys take them back from TRACK MUTE.
        held(&mut app, SHIFT, |a| press(a, SAMPLER));
        assert!(app.pads_are_loop_keys());
        assert_eq!(app.mute_keys(), MuteKeys::Off);
        // STEP SEQ takes them too (and opens the Sequencer's Steps, as
        // always).
        click(&mut app, STEP_SEQ);
        assert!(app.pads_are_steps());
        assert!(!app.pads_are_loop_keys());
        assert_eq!(app.view(), View::Sequencer);
        // SHIFT + SAMPLER on the Sequencer opens the Looper tab; again
        // there, the loop keys.
        held(&mut app, SHIFT, |a| press(a, SAMPLER));
        assert_eq!(app.view(), View::Looper);
        assert!(!app.pads_are_loop_keys());
        held(&mut app, SHIFT, |a| press(a, SAMPLER));
        assert!(app.pads_are_loop_keys());
        assert!(!app.pads_are_steps(), "and give STEP SEQ's pads back");
        // SHIFT + SAMPLER again: the pads play.
        let fx = held(&mut app, SHIFT, |a| press(a, SAMPLER));
        assert!(!app.pads_are_loop_keys());
        assert!(fx.contains(&Effect::Note("SHIFT + SAMPLER: the pads play again".into())));
        assert_eq!(app.view(), View::Looper);
    }

    /// The Perform set on the pads (step 6, board 7): PAD BANK picks its
    /// page (SHIFT or a second press for E to H, as a kit's), and only
    /// page A is the loop keys. SHIFT + SAMPLER brings the set back at the
    /// page shown last; the Looper's Pads switch brings page A. A throw
    /// pad plays while held, or latched until its next hit, and whatever
    /// plays lets go when the set goes off. Out of the set PAD BANK picks
    /// kits again.
    #[test]
    fn pad_bank_pages_the_perform_set_and_its_throws_let_go_with_it() {
        use crate::perform_pads::{Hold, PadAction, Throw};
        const SHIFT: u8 = button::SHIFT;
        const SAMPLER: u8 = button::SAMPLER;
        const BANK_A: u8 = 35;
        const BANK_B: u8 = 36;
        let mut app = ready();
        app.map.learn(mapper::Job::Bank(0), Some(BANK_A));
        app.map.learn(mapper::Job::Bank(1), Some(BANK_B));
        app.open_tab(View::Looper);
        held(&mut app, SHIFT, |a| press(a, SAMPLER));
        assert!(app.pads_are_loop_keys());
        assert_eq!(app.perform_bank(), 0);
        // PAD BANK B: page B, Play's throws; no kit changes.
        let fx = press(&mut app, BANK_B);
        assert_eq!(app.perform_bank(), 1);
        assert!(
            fx.contains(&Effect::Note("Perform: Page B · Throws".into())),
            "{fx:?}"
        );
        assert!(fx.contains(&Effect::BankLights));
        assert!(!fx.iter().any(|e| matches!(e, Effect::Bank { .. })));
        assert!(!app.status_now().loop_keys, "page B isn't the loop keys");
        assert!(app.pads_are_loop_keys(), "the set is still on the pads");
        let reverb = Throw::new(1, Hold::WhileHeld);
        assert_eq!(app.perform_page().action(1), PadAction::Throw(reverb));
        // Held: on with the press, off with the lift, once each.
        assert_eq!(app.perform_throw(reverb, true), Some(true));
        assert_eq!(app.throws_on(), [(1, mpfree_fx::StripId::Master)]);
        assert_eq!(app.perform_throw(reverb, true), None);
        assert_eq!(app.perform_throw(reverb, false), Some(false));
        assert!(app.throws_on().is_empty());
        // Latched: a hit starts it, the lift does nothing, the next hit
        // lets it go.
        let brake = Throw::new(7, Hold::Latch);
        assert_eq!(app.perform_throw(brake, true), Some(true));
        assert_eq!(app.perform_throw(brake, false), None);
        assert_eq!(app.throws_on(), [(7, mpfree_fx::StripId::Master)]);
        // The same throw on Drums is another: it plays beside it, and
        // lets go there.
        let on_drums = Throw {
            on: mpfree_fx::StripId::Track(0),
            ..brake
        };
        assert_eq!(app.perform_throw(on_drums, true), Some(true));
        assert_eq!(
            app.throws_on(),
            [
                (7, mpfree_fx::StripId::Master),
                (7, mpfree_fx::StripId::Track(0))
            ]
        );
        assert_eq!(app.perform_throw(on_drums, true), Some(false));
        assert_eq!(app.throws_on(), [(7, mpfree_fx::StripId::Master)]);
        assert_eq!(app.perform_throw(brake, true), Some(false));
        assert_eq!(app.perform_throw(brake, false), None);
        // A bar's: lit while held (the Perform FX plays the bar out); a
        // macro pad's light follows the finger.
        let bar = Throw::new(2, Hold::Bar);
        assert_eq!(app.perform_throw(bar, true), Some(true));
        assert_eq!(app.perform_throw(bar, false), Some(false));
        app.perform_macro(3, true);
        assert_eq!(app.macros_on(), 0b1000);
        app.perform_macro(3, false);
        assert_eq!(app.macros_on(), 0);
        // Page C named: the screen gets the set with its name.
        let fx = app.set_perform_name(2, " Drops ");
        assert_eq!(app.perform_set().pages[2].name, "Drops");
        assert!(fx
            .iter()
            .any(|e| matches!(e, Effect::PerformSet { opened: false, .. })));
        // Latched again, then SHIFT + SAMPLER: the set goes off and the
        // throw lets go.
        app.perform_throw(brake, true);
        let fx = held(&mut app, SHIFT, |a| press(a, SAMPLER));
        assert!(!app.pads_are_loop_keys());
        let let_go = crate::perform_pads::throw_fx(7, false, 0).unwrap();
        assert!(fx.contains(&Effect::PlayFx(let_go)), "{fx:?}");
        assert!(app.throws_on().is_empty());
        // Out of the set, PAD BANK picks kits.
        assert_eq!(
            press(&mut app, BANK_B),
            vec![Effect::Bank {
                button: 1,
                shift: false
            }]
        );
        // SHIFT + SAMPLER again: back at page B, and the log says so.
        let fx = held(&mut app, SHIFT, |a| press(a, SAMPLER));
        assert_eq!(app.perform_bank(), 1);
        assert!(
            fx.iter()
                .any(|e| matches!(e, Effect::Note(n) if n.contains("Page B · Throws"))),
            "{fx:?}"
        );
        // A second press of B: page F, empty; SHIFT + A: page E.
        press(&mut app, BANK_B);
        assert_eq!(app.perform_bank(), 5);
        assert_eq!(app.perform_page().action(0), PadAction::None);
        held(&mut app, SHIFT, |a| press(a, BANK_A));
        assert_eq!(app.perform_bank(), 4);
        // The Looper's Pads switch: page A, the loop keys; again, off.
        let fx = app.toggle_loop_keys();
        assert_eq!(app.perform_bank(), 0);
        assert!(app.status_now().loop_keys);
        assert!(fx.contains(&Effect::BankLights));
        app.toggle_loop_keys();
        assert!(!app.pads_are_loop_keys());
        // A project opened: its set, and nothing left playing.
        app.toggle_loop_keys();
        press(&mut app, BANK_B);
        app.perform_throw(reverb, true);
        let (skipped, fx) = app.open_perform(Some("mpfree-perform 1\npad B1 = scene 1\nbad\n"));
        assert_eq!(skipped, ["perform: skipped \"bad\""]);
        assert!(fx.contains(&Effect::PlayFx(
            crate::perform_pads::throw_fx(1, false, 0).unwrap()
        )));
        assert!(app.status_now().perform.is_some(), "the screen hears");
        assert!(app.throws_on().is_empty());
        assert_eq!(app.perform_page().action(0), PadAction::Scene(0));
        assert_eq!(app.perform_page().action(1), PadAction::None);
        // A project saved before the part: a new project's set.
        let (skipped, _) = app.open_perform(None);
        assert!(skipped.is_empty());
        assert_eq!(
            app.perform_set(),
            &crate::perform_pads::PerformSet::default()
        );
    }

    /// The Pads tab's Perform view (board 7): while it shows, PAD BANK
    /// turns the set's pages (the kit's bank stays) and the pads are the
    /// set's; an edit changes the set, gives the screen the new one and
    /// lets go of a throw the pad had latched; On the pads puts the set on
    /// at the page shown. Another tab, or the kit's view, gives PAD BANK
    /// back to the kit.
    #[test]
    fn the_perform_view_pages_and_edits_the_set() {
        use crate::perform_pads::{Hold, PadAction, Throw};
        const BANK_C: u8 = 37;
        let mut app = ready();
        app.map.learn(mapper::Job::Bank(2), Some(BANK_C));
        app.open_tab(View::Pads);
        assert!(!app.perform_editing());
        assert!(!app.show_perform(true).is_empty());
        assert!(app.perform_editing());
        assert!(app.pads_are_perform());
        assert!(!app.pads_are_loop_keys(), "not on the pads for Play");
        // PAD BANK C: page C of the set, no kit bank.
        let fx = press(&mut app, BANK_C);
        assert_eq!(app.perform_bank(), 2);
        assert!(
            fx.contains(&Effect::Note("Perform: Page C".into())),
            "{fx:?}"
        );
        assert!(!fx.iter().any(|e| matches!(e, Effect::Bank { .. })));
        assert_eq!(app.status_now().perform_bank, 2);
        // The strip's page B.
        assert!(!app.perform_page_to(1).is_empty());
        assert!(app.perform_page_to(1).is_empty(), "already there");
        // B8, Vinyl brake, latched and playing: made a scene, it lets go.
        let brake = Throw::new(7, Hold::Latch);
        app.set_perform_pad(1, 7, PadAction::Throw(brake));
        app.perform_throw(brake, true);
        let fx = app.set_perform_pad(1, 7, PadAction::Scene(2));
        let let_go = crate::perform_pads::throw_fx(7, false, 0).unwrap();
        assert!(fx.contains(&Effect::PlayFx(let_go)), "{fx:?}");
        assert!(app.throws_on().is_empty());
        assert!(fx.iter().any(
            |e| matches!(e, Effect::PerformSet { set, opened: false } if set.page(1).action(7) == PadAction::Scene(2))
        ));
        assert!(app.perform_set().text().contains("pad B8 = scene 3"));
        // On the pads: the set on at page B.
        app.perform_on();
        assert_eq!(app.status_now().perform.as_deref(), Some("Page B · Throws"));
        // The Looper tab: the set plays there; PAD BANK still pages it.
        app.open_tab(View::Looper);
        assert!(!app.perform_editing());
        assert!(app.pads_are_loop_keys());
        app.perform_off();
        // The kit's view again: PAD BANK picks kit banks.
        app.open_tab(View::Pads);
        app.show_perform(false);
        assert!(!app.pads_are_perform());
        assert_eq!(
            press(&mut app, BANK_C),
            vec![Effect::Bank {
                button: 2,
                shift: false
            }]
        );
    }

    /// TRACK MUTE held with a pad tapped: a solo, and letting go leaves
    /// the pad mode as it was before the press. A quick press still
    /// toggles the mode. In Make a tap acts at once and in Perform it
    /// waits; SHIFT gives the other timing.
    #[test]
    fn track_mute_held_with_a_pad_solos_and_leaves_the_mode_as_it_was() {
        const SHIFT: u8 = button::SHIFT;
        const TRACK_MUTE: u8 = 68;
        let mut app = ready();
        app.map.learn(mapper::Job::TrackMute, Some(TRACK_MUTE));
        assert!(!app.pads_mute());
        // From Off: held, the pads mute; a tap solos; let go, Off again.
        app.button(TRACK_MUTE, true, 0.0);
        assert!(app.pads_mute());
        let (tap, fx) = app.mute_tap();
        assert!(fx.is_empty(), "the pads show the tracks already");
        assert_eq!(
            tap,
            MuteTap {
                keys: MuteKeys::Tracks,
                bank: 0,
                wait: false,
                solo: true
            }
        );
        let fx = app.button(TRACK_MUTE, false, 0.0);
        assert_eq!(app.mute_keys(), MuteKeys::Off);
        assert_eq!(fx[0], Effect::Light(TRACK_MUTE, LIGHT_OFF));
        assert!(fx.contains(&Effect::BankLights));
        assert!(!app.pads_mute());
        // From Tracks: the press turns them off, a tap while held brings
        // the tracks back to solo, and letting go leaves them on.
        press(&mut app, TRACK_MUTE);
        assert_eq!(app.mute_keys(), MuteKeys::Tracks);
        app.button(TRACK_MUTE, true, 0.0);
        assert_eq!(app.mute_keys(), MuteKeys::Off);
        let (tap, fx) = app.mute_tap();
        assert!(tap.solo && tap.keys == MuteKeys::Tracks);
        assert_eq!(fx[0], Effect::Light(TRACK_MUTE, LIGHT_ON));
        app.button(TRACK_MUTE, false, 0.0);
        assert_eq!(app.mute_keys(), MuteKeys::Tracks);
        // Not held: a plain tap mutes, at once in Make, waiting with SHIFT.
        let (tap, _) = app.mute_tap();
        assert!(!tap.solo && !tap.wait);
        app.button(SHIFT, true, 0.0);
        assert!(app.mute_tap().0.wait);
        app.button(SHIFT, false, 0.0);
        // Perform: the tap waits, and SHIFT acts at once.
        app.set_perform(true);
        assert!(app.mute_tap().0.wait);
        app.button(SHIFT, true, 0.0);
        assert!(!app.mute_tap().0.wait);
        app.button(SHIFT, false, 0.0);
        // SHIFT + TRACK MUTE held is no solo: the kit's pads mute.
        app.button(SHIFT, true, 0.0);
        app.button(TRACK_MUTE, true, 0.0);
        app.button(SHIFT, false, 0.0);
        assert_eq!(app.mute_keys(), MuteKeys::Pads);
        assert!(!app.mute_tap().0.solo);
        app.button(TRACK_MUTE, false, 0.0);
        assert_eq!(app.mute_keys(), MuteKeys::Pads);
        // Over the kit's pad mutes, TRACK MUTE held solos a pad of the kit:
        // the press shows the tracks, the tap brings the kit's pads back,
        // and letting go leaves them.
        app.button(TRACK_MUTE, true, 0.0);
        assert_eq!(app.mute_keys(), MuteKeys::Tracks);
        let (tap, fx) = app.mute_tap();
        assert!(tap.solo && tap.keys == MuteKeys::Pads, "{tap:?}");
        assert!(fx.contains(&Effect::BankLights), "{fx:?}");
        assert_eq!(app.mute_keys(), MuteKeys::Pads);
        assert!(app.mute_tap().0.wait, "in Perform it waits");
        app.button(TRACK_MUTE, false, 0.0);
        assert_eq!(app.mute_keys(), MuteKeys::Pads);
        assert!(!app.mute_tap().0.solo);
    }

    /// TRACK MUTE with more than 16 tracks (up to 128, Jay 14 Nov): PAD
    /// BANK picks which 16 tracks the pads mute, as it picks kits (SHIFT or
    /// a second press for E-H), and the kit playing stays. Out of TRACK
    /// MUTE the buttons pick kits again.
    #[test]
    fn track_mute_banks_pick_which_16_tracks_the_pads_mute() {
        const TRACK_MUTE: u8 = 68;
        const BANK_B: u8 = 36;
        let mut app = ready();
        app.map.learn(mapper::Job::TrackMute, Some(TRACK_MUTE));
        app.map.learn(mapper::Job::Bank(1), Some(BANK_B));
        app.set_track_count(24);
        let fx = press(&mut app, TRACK_MUTE);
        assert!(
            fx.contains(&Effect::Note(
                "TRACK MUTE · bank A: pads 1-16 mute tracks 1-16".into()
            )),
            "{fx:?}"
        );
        assert!(fx.contains(&Effect::BankLights));
        let fx = press(&mut app, BANK_B);
        assert_eq!(app.mute_bank(), 1);
        assert_eq!(app.status_now().mute_bank, 1);
        assert_eq!(
            fx,
            vec![
                Effect::Note("TRACK MUTE · bank B: pads 1-8 mute tracks 17-24".into()),
                Effect::BankLights
            ],
            "no kit change"
        );
        // A second press: bank F, tracks 81-96, none in this project.
        let fx = press(&mut app, BANK_B);
        assert_eq!(app.mute_bank(), 5);
        assert!(fx.contains(&Effect::Note(
            "TRACK MUTE · bank F: no tracks here (it is tracks 81-96)".into()
        )));
        press(&mut app, TRACK_MUTE);
        assert_eq!(app.mute_keys(), MuteKeys::Off);
        assert_eq!(
            press(&mut app, BANK_B),
            vec![Effect::Bank {
                button: 1,
                shift: false
            }]
        );
        // The bank is kept for the next TRACK MUTE.
        let fx = press(&mut app, TRACK_MUTE);
        assert!(fx
            .iter()
            .any(|e| matches!(e, Effect::Note(n) if n.contains("bank F"))));
    }

    /// Sampler R5.7: REC with kit pads held asks the audio loop for a pad
    /// take (it knows which are empty); STOP while one records keeps it
    /// with the music playing on, and UNDO throws it on every page.
    #[test]
    fn rec_with_kit_pads_held_asks_for_a_pad_take() {
        use crate::rec_arm::{PadTake, Press};
        let mut app = App::new(44_100, false);
        app.start();
        app.set_pad_page(1);
        app.pad_down(4, 100);
        app.pad_down(1, 100);
        assert_eq!(app.held_kit_pads(), [17, 20]);
        assert_eq!(press(&mut app, button::REC), [Effect::PadRec(vec![17, 20])]);
        // SHIFT + REC with a kit pad held is Sample Recall onto the first
        // (sampler R5, layout spec section 6): the past, not a take.
        app.button(button::SHIFT, true, 0.0);
        assert_eq!(
            press(&mut app, button::REC),
            [Effect::Recall(RecallKey::Open { pad: 17 })]
        );
        app.button(button::SHIFT, false, 0.0);
        // 16 Levels has the pads: REC as alone.
        app.levels_on = true;
        assert_eq!(press(&mut app, button::REC), [Effect::Rec(Press::Rec)]);
        app.levels_on = false;
        app.pad_up(4);
        app.pad_up(1);
        assert_eq!(press(&mut app, button::REC), [Effect::Rec(Press::Rec)]);
        // A pad take armed: STOP calls it off and stops as usual; while it
        // records, STOP keeps it and the transport plays on.
        app.set_rolling(true);
        app.rec.pad = Some(PadTake {
            kit: 0,
            pad: 4,
            id: 1,
            started: true,
            recording: false,
            here: false,
        });
        let fx = press(&mut app, button::STOP);
        assert_eq!(fx[0], Effect::Rec(Press::Stop));
        assert!(fx.contains(&Effect::Time(TimeAction::Transport(Command::Stop))));
        app.rec.pad = app.rec.pad.map(|p| PadTake {
            recording: true,
            ..p
        });
        assert_eq!(press(&mut app, button::STOP), [Effect::Rec(Press::Stop)]);
        // UNDO throws it, on the Sample tab too (not the kit's undo).
        app.open_tab(View::Sample);
        assert_eq!(
            press(&mut app, button::UNDO),
            [Effect::Undo { redo: false }, Effect::UndoButton(true)]
        );
    }

    /// Other gear's MMC goes through the arm as PLAY, STOP and REC do
    /// (the sequencer thread's check): the same effects as the buttons in
    /// each state, a pad take armed and recording too; Play while playing
    /// is nothing rather than FILL.
    #[test]
    fn mmc_goes_through_the_arm_as_play_stop_and_rec_do() {
        use crate::rec_arm::{PadTake, Press};
        use mpfree_midi::mmc::Mmc;
        let mut app = App::new(44_100, false);
        app.start();
        let same = |app: &mut App, c: Mmc, b: u8| {
            let mmc = app.mmc(c);
            assert_eq!(mmc, press(app, b), "{c:?}");
            mmc
        };
        assert_eq!(
            same(&mut app, Mmc::Play, button::PLAY),
            [Effect::Time(TimeAction::Transport(Command::Play))]
        );
        assert_eq!(
            same(&mut app, Mmc::RecordStrobe, button::REC),
            [Effect::Rec(Press::Rec)]
        );
        // The notes ready: Play starts the take; Stop lets it go.
        app.rec.ready = Some(crate::rec_arm::Take::Replace);
        assert_eq!(
            same(&mut app, Mmc::DeferredPlay, button::PLAY),
            [Effect::Rec(Press::Play { from_start: false })]
        );
        app.rec.ready = None;
        // A pad take armed: REC is REC (the arm decides), Stop calls it
        // off and stops; while it records, Stop keeps it.
        app.set_rolling(true);
        app.rec.pad = Some(PadTake {
            kit: 0,
            pad: 4,
            id: 1,
            started: true,
            recording: false,
            here: false,
        });
        assert_eq!(
            same(&mut app, Mmc::RecordStrobe, button::REC),
            [Effect::Rec(Press::Rec)]
        );
        let stop = app.mmc(Mmc::Stop);
        assert_eq!(stop[0], Effect::Rec(Press::Stop));
        assert!(stop.contains(&Effect::Time(TimeAction::Transport(Command::Stop))));
        assert_eq!(stop, press(&mut app, button::STOP));
        app.rec.pad = app.rec.pad.map(|p| PadTake {
            recording: true,
            ..p
        });
        assert_eq!(
            same(&mut app, Mmc::Pause, button::STOP),
            [Effect::Rec(Press::Stop)]
        );
        // Playing: a DAW's Play is nothing (PLAY would be FILL).
        app.rec.pad = None;
        assert!(app.mmc(Mmc::Play).is_empty());
        assert!(app.mmc(Mmc::RecordExit).is_empty());
    }

    /// The device as it starts on a fresh stick: no saved map, no Map
    /// controls. REC, OVERDUB, UNDO and PLAY START must work and light on
    /// their real numbers (the bug Jay hit three times: the tests gave them
    /// made-up numbers, the MPC had none). Every light sent for them is one
    /// main.rs lets through to the controller (`akai::app_lights`).
    #[test]
    fn a_fresh_mpfree_knows_rec_overdub_undo_and_play_start() {
        let mut app = App::new(44_100, false);
        assert!(app.start().contains(&Effect::Show(View::Play)));
        let time = |a: TimeAction| Effect::Time(a);
        // REC and OVERDUB go to what is armed, on every page.
        use crate::rec_arm::Press;
        assert_eq!(press(&mut app, button::REC), vec![Effect::Rec(Press::Rec)]);
        assert_eq!(
            press(&mut app, button::OVERDUB),
            vec![Effect::Rec(Press::Overdub)]
        );
        assert_eq!(
            press(&mut app, button::PLAY_START),
            vec![time(TimeAction::Transport(Command::PlayStart))]
        );
        assert_eq!(
            press(&mut app, button::UNDO),
            vec![Effect::Undo { redo: false }, Effect::UndoButton(true)]
        );
        app.open_tab(View::Looper);
        assert_eq!(press(&mut app, button::REC), vec![Effect::Rec(Press::Rec)]);
        app.open_tab(View::Arrange);
        assert_eq!(press(&mut app, button::REC), vec![Effect::Rec(Press::Rec)]);
        // The notes ready (the desk said so): PLAY and PLAY START start the
        // take, STOP lets go too, and REC and PLAY blink.
        app.rec.ready = Some(crate::rec_arm::Take::Replace);
        assert_eq!(
            press(&mut app, button::PLAY),
            vec![Effect::Rec(Press::Play { from_start: false })]
        );
        assert_eq!(
            press(&mut app, button::PLAY_START),
            vec![Effect::Rec(Press::Play { from_start: true })]
        );
        let stop = press(&mut app, button::STOP);
        assert!(stop.contains(&Effect::Rec(Press::Stop)), "{stop:?}");
        let fx = app.transport_lights(TransportLights::default(), 0.0);
        assert!(fx.contains(&Effect::Light(button::REC, LIGHT_ON)), "{fx:?}");
        assert!(
            fx.contains(&Effect::Light(button::PLAY, LIGHT_ON)),
            "{fx:?}"
        );
        app.rec.ready = None;
        let fx = app.transport_lights(TransportLights::default(), 0.0);
        assert!(
            fx.contains(&Effect::Light(button::REC, LIGHT_DARK)),
            "{fx:?}"
        );
        // Their lights: REC blinks while armed and recording, OVERDUB while
        // overdubbing, PLAY while they wait, and every one is sent.
        let lights = |rolling, recording, overdub| TransportLights {
            rolling,
            recording,
            overdub,
        };
        let fx = app.transport_lights(lights(false, true, false), 0.0);
        assert!(fx.contains(&Effect::Light(button::REC, LIGHT_ON)), "{fx:?}");
        assert!(
            fx.contains(&Effect::Light(button::PLAY, LIGHT_ON)),
            "{fx:?}"
        );
        let fx = app.transport_lights(lights(true, false, true), 2.5 * BLINK_HALF_SECONDS);
        assert!(
            fx.contains(&Effect::Light(button::OVERDUB, LIGHT_ON)),
            "{fx:?}"
        );
        assert!(
            fx.contains(&Effect::Light(button::UNDO, LIGHT_ON)),
            "{fx:?}"
        );
        for n in [
            button::REC,
            button::OVERDUB,
            button::UNDO,
            button::PLAY,
            button::PLAY_START,
        ] {
            assert!(crate::akai::app_lights(n), "button {n}'s light is sent");
        }
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
        // REC and OVERDUB record what is armed, on Play and Arrange alike.
        use crate::rec_arm::Press;
        assert_eq!(press(&mut app, 90), vec![Effect::Rec(Press::Rec)]);
        assert_eq!(press(&mut app, 91), vec![Effect::Rec(Press::Overdub)]);
        app.open_tab(View::Arrange);
        assert_eq!(press(&mut app, 90), vec![Effect::Rec(Press::Rec)]);
        assert_eq!(press(&mut app, 91), vec![Effect::Rec(Press::Overdub)]);
        app.open_tab(View::Play);
        // UNDO follows the page: the screen knows the Play view, so it picks.
        assert_eq!(
            press(&mut app, 92),
            vec![Effect::Undo { redo: false }, Effect::UndoButton(true)]
        );
        assert_eq!(
            held(&mut app, button::SHIFT, |a| press(a, 92)),
            vec![Effect::Undo { redo: true }, Effect::UndoButton(true)]
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
        assert_eq!(app.wheel(1), vec![Effect::Rows(RowKey::Pick(1))]);
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
            vec![Effect::Nav(Nav::Turn(1))],
            "the wheel moves the outline on Play"
        );
    }

    /// Jay's wheel navigation (13 Nov): on Play and the Looper tab the
    /// wheel, its push and - / + go to the outline, never to the kit; on
    /// the mixer and the chain editor once the outline is on a control.
    #[test]
    fn the_wheel_moves_the_outline_and_never_the_kit() {
        let mut app = ready();
        let nav = |n| vec![Effect::Nav(n)];
        for v in [View::Play, View::Looper] {
            app.open_tab(v);
            assert_eq!(app.wheel(2), nav(Nav::Turn(2)), "{v:?}");
            assert_eq!(app.wheel_push(), nav(Nav::Push), "{v:?}");
            assert_eq!(press(&mut app, button::PLUS), nav(Nav::Step(1)), "{v:?}");
            assert_eq!(press(&mut app, button::MINUS), nav(Nav::Step(-1)), "{v:?}");
        }
        // SHIFT + wheel and NOTE REPEAT keep their jobs on Play.
        app.open_tab(View::Play);
        assert_eq!(
            held(&mut app, button::SHIFT, |a| a.wheel(1)),
            vec![Effect::Octave(1)]
        );
        app.button(REPEAT, true, 0.0);
        assert!(!app.wheel(1).iter().any(|e| matches!(e, Effect::Nav(_))));
        assert_eq!(app.wheel_push(), Vec::<Effect>::new());
        app.button(REPEAT, false, 0.0);
        // The mixer: the selected strip's volume and the I/O rows until a
        // tap puts the outline on a control.
        app.toggle_mixer(false);
        assert_eq!(app.view(), View::Mixer);
        assert_eq!(app.wheel(1), vec![Effect::MixerWheel(1)]);
        assert_eq!(press(&mut app, button::PLUS), vec![Effect::MixerStep(1)]);
        app.set_outline(true);
        assert_eq!(app.wheel(1), nav(Nav::Turn(1)));
        assert_eq!(app.wheel_push(), nav(Nav::Push));
        assert_eq!(press(&mut app, button::MINUS), nav(Nav::Step(-1)));
        // The chain editor: the screen's (the effect picker, else Q-Link
        // 1) without an outline; the push is the screen's too, which gives
        // it to the picker or the outline (gui.rs).
        app.set_outline(false);
        app.open_chain();
        assert_eq!(app.view(), View::Chain);
        assert_eq!(app.wheel(1), vec![Effect::ChainWheel(1)]);
        assert_eq!(app.wheel_push(), vec![Effect::ChainPush]);
        app.set_outline(true);
        assert_eq!(app.wheel(-1), nav(Nav::Turn(-1)));
        // A page with rows keeps its rows, outline or not.
        app.open_tab(View::Synth);
        assert_eq!(app.wheel(1), vec![Effect::Rows(RowKey::Pick(1))]);
        assert_eq!(app.wheel_push(), vec![Effect::Rows(RowKey::Editing(true))]);
    }

    /// BROWSE and MAIN work on a new stick, with no saved map, and a
    /// page's button leaves Prefs for that page (Jay, round 4); SHIFT +
    /// MENU still opens and closes it.
    #[test]
    fn a_page_button_leaves_prefs() {
        let mut app = ready();
        assert_eq!(app.map.browse, Some(button::BROWSE));
        assert_eq!(app.map.main, Some(button::MAIN));
        assert_eq!(app.map.data_push, Some(button::DATA_PUSH));
        for tab in PrefsTab::ALL {
            app.open_tab(View::Prefs);
            app.prefs_tab = tab;
            assert!(press(&mut app, button::BROWSE).contains(&Effect::Show(View::Browser)));
            assert_eq!(app.view(), View::Browser, "{tab:?}");
            app.open_tab(View::Prefs);
            app.prefs_tab = tab;
            assert!(click(&mut app, button::MAIN).contains(&Effect::Show(View::Play)));
            assert_eq!(app.view(), View::Play, "{tab:?}");
        }
        app.map.learn(mapper::Job::Sampler, Some(65));
        app.open_tab(View::Prefs);
        assert!(press(&mut app, 65).contains(&Effect::Show(View::Sample)));
        assert!(!app.in_menu());
    }

    /// The mode buttons, ERASE and the SHIFT combos from the hardware
    /// control map (Jay, 20:40Z), once Map controls has found them.
    #[test]
    fn the_mode_buttons_go_to_the_page_with_the_same_job() {
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
        // MAIN: Play, and on Play the next view (the screen keeps All),
        // on the let-go (held, the pads pick a track).
        assert!(click(&mut app, 61).contains(&Effect::Show(View::Play)));
        assert!(click(&mut app, 61).contains(&Effect::PlayView(None)));
        // STEP SEQ: the pads become steps and the Sequencer shows its
        // Steps (on the release); NEXT SEQ: Arrange.
        let fx = click(&mut app, 64);
        assert!(fx.contains(&Effect::Show(View::Sequencer)), "{fx:?}");
        assert!(app.pads_are_steps());
        assert!(press(&mut app, 63).contains(&Effect::Show(View::Arrange)));
        let fx = click(&mut app, 64);
        assert!(!app.pads_are_steps(), "STEP SEQ again gives the pads back");
        assert!(!fx.iter().any(|e| matches!(e, Effect::Show(_))), "{fx:?}");
        let fx = click(&mut app, 64);
        assert!(fx.contains(&Effect::Show(View::Sequencer)));
        click(&mut app, 64);
        // SAMPLE EDIT: the Wave editor; PROG EDIT: the audio loop decides.
        let fx = press(&mut app, 66);
        assert_eq!(fx[0], Effect::SampleEditor(Editor::Wave));
        assert!(fx.contains(&Effect::Show(View::Sample)));
        assert_eq!(press(&mut app, 67), [Effect::ProgEdit]);
        // On the Pads tab, PROG EDIT is Edit pad's, in and out (the one
        // Sample page's step 6b): the screen knows which.
        app.open_tab(View::Pads);
        assert_eq!(press(&mut app, 67), [Effect::EditPad]);
        assert_eq!(app.view(), View::Pads);
        // ERASE and SHIFT + ERASE only tell the screen while they are held
        // (ERASE + a tap deletes); they go to no page.
        let only_status = |fx: Vec<Effect>| fx.iter().all(|e| matches!(e, Effect::Status(_)));
        assert!(only_status(press(&mut app, 60)));
        assert!(only_status(shifted(&mut app, 60)));
        assert!(!app.erase_held, "let go");
        // 16 LEVEL: Play, and 16 Levels on or off (the screen holds the
        // kit and says back what it did: the light and the wheel follow).
        let fx = press(&mut app, 62);
        assert!(fx.contains(&Effect::Show(View::Play)), "{fx:?}");
        assert_eq!(fx.last(), Some(&Effect::SixteenLevels));
        assert_eq!(press(&mut app, 62), [Effect::SixteenLevels]);
        assert_eq!(
            app.levels_shown(true, true, false),
            [Effect::Light(62, LIGHT_ON)]
        );
        assert_eq!(app.wheel(-1), [Effect::LevelsWheel(-1)]);
        assert_eq!(
            app.levels_shown(true, false, false),
            [Effect::Light(62, LIGHT_ON)]
        );
        assert!(!app.wheel(1).contains(&Effect::LevelsWheel(1)));
        assert_eq!(
            app.levels_shown(false, false, false),
            [Effect::Light(62, LIGHT_OFF)]
        );
        // Transpose (sequencer S4): the pads move the armed track while
        // 16 Levels are on with that kind.
        assert!(!app.pads_transpose());
        app.levels_shown(true, true, true);
        assert!(app.pads_transpose());
        app.levels_shown(false, false, true);
        assert!(!app.pads_transpose());
        // SHIFT + 16 LEVEL: Chromatic | In key (no scale yet: Major).
        assert!(shifted(&mut app, 62)
            .iter()
            .any(|e| matches!(e, Effect::Tell(n) if n.contains("In key, C Major"))));
    }

    /// Make and Perform (sequencer S4, Jay's decision 5): SHIFT + MAIN
    /// switches, as the top bar's chip does; in Perform MAIN goes to
    /// Arrange's Clips grid and then steps its views, and the arm is pinned
    /// until Make again.
    #[test]
    fn shift_main_switches_make_and_perform_and_perform_moves_main_home() {
        const SHIFT: u8 = button::SHIFT;
        let mut app = ready();
        app.open_tab(View::Sample);
        assert!(click(&mut app, button::MAIN).contains(&Effect::Show(View::Play)));
        // SHIFT + MAIN asks for Perform; the project says so next period.
        let fx = held(&mut app, SHIFT, |a| press(a, button::MAIN));
        assert_eq!(fx, [Effect::Time(TimeAction::Perform(true))]);
        assert!(!app.rec.pinned);
        assert!(app.set_perform(true));
        assert!(app.rec.pinned, "Perform pins the arm");
        assert!(!app.set_perform(true), "no change, nothing to show");
        // MAIN: Arrange's Clips grid, then the next of its views.
        let fx = click(&mut app, button::MAIN);
        assert!(fx.contains(&Effect::Show(View::Arrange)));
        assert!(fx.contains(&Effect::ArrangeView(Some(false))));
        assert_eq!(app.view(), View::Arrange);
        assert!(click(&mut app, button::MAIN).contains(&Effect::ArrangeView(None)));
        // A hand-made unpin stays until the mode changes again.
        app.rec.pinned = false;
        assert!(!app.set_perform(true));
        assert!(!app.rec.pinned);
        // SHIFT + MAIN again: Make, the arm follows the page, MAIN is Play.
        let fx = held(&mut app, SHIFT, |a| press(a, button::MAIN));
        assert_eq!(fx, [Effect::Time(TimeAction::Perform(false))]);
        app.rec.pinned = true;
        assert!(app.set_perform(false));
        assert!(!app.rec.pinned);
        assert!(click(&mut app, button::MAIN).contains(&Effect::Show(View::Play)));
    }

    /// STEP SEQ on the synth (the sequencer design's "Synth tracks"): the
    /// pads are the whole line's steps, and the pen starts as the pad
    /// played last. Held, the pads play the synth: pads held together are
    /// the new pen (a chord), a pad played once all are up starts a new
    /// one; its notes name it.
    #[test]
    fn step_seq_on_the_synth_takes_the_line_and_the_pen_from_the_pads_held() {
        use crate::time::LINE;
        use mpfree_arp::scale::pads::{layout, PadLayout, PadSetup};
        const STEP_SEQ: u8 = button::STEP_SEQ;
        let mut app = ready();
        app.set_pad_layout(Some(layout(&PadSetup {
            layout: PadLayout::Chromatic,
            ..PadSetup::default()
        })));
        app.pad_down(2, 100);
        app.pad_up(2);
        let fx = click(&mut app, STEP_SEQ);
        let s = app.step_pads().expect("the pads are steps");
        assert_eq!((s.sound, s.pen.pads()), (LINE, &[2][..]));
        assert!(s.line());
        let name = app.pen_name(&s.pen);
        assert!(
            fx.iter().any(
                |e| matches!(e, Effect::Note(n) if n.contains("line's steps") && n.contains(&name))
            ),
            "{fx:?}"
        );
        // Held: pads 1 and 5 together are the pen; then 8 alone replaces it.
        let t = 0.0;
        app.button(STEP_SEQ, true, t);
        assert!(!app.pads_are_steps(), "held, the pads play the synth");
        assert!(app.step_pads().is_some_and(|p| p.picking && p.line()));
        app.pad_down(0, 100);
        app.pad_down(4, 100);
        app.pad_down(4, 100);
        assert_eq!(app.step_pads().unwrap().pen.pads(), [0, 4]);
        app.pad_up(0);
        app.pad_up(4);
        app.pad_down(7, 100);
        app.pad_up(7);
        app.button(STEP_SEQ, false, t);
        let s = app.step_pads().expect("still steps");
        assert!(app.pads_are_steps());
        assert_eq!(s.pen.pads(), [7]);
        // A chord, named by its notes, lowest first.
        app.button(STEP_SEQ, true, t);
        for p in [4, 0, 7] {
            app.pad_down(p, 100);
        }
        for p in [4, 0, 7] {
            app.pad_up(p);
        }
        app.button(STEP_SEQ, false, t);
        let s = app.step_pads().unwrap();
        assert_eq!(s.pen.pads(), [4, 0, 7]);
        let name = app.pen_name(&s.pen);
        assert_eq!(name.split(' ').count(), 3, "{name}");
        // A kit on the pads again: a sound's steps, the pen kept.
        app.set_pad_layout(None);
        assert_ne!(app.step_pads().unwrap().sound, LINE);
        assert_eq!(app.step_pads().unwrap().pen.pads(), [4, 0, 7]);
    }

    /// STEP SEQ (sequencer decision 2, Jay 16 Nov): the pads are the
    /// steps of the pad played last, over any page; PAD BANK picks pages
    /// (a second press or SHIFT: 5 to 8); SHIFT + STEP SEQ is Follow;
    /// held, a pad tapped picks the sound; ERASE + PAD BANK clears a
    /// page; one pad mode at a time with TRACK MUTE and 16 LEVEL.
    #[test]
    fn step_seq_turns_the_pads_into_steps_until_pressed_again() {
        use crate::time::StepAction;
        const SHIFT: u8 = button::SHIFT;
        const STEP_SEQ: u8 = button::STEP_SEQ;
        const TRACK_MUTE: u8 = button::TRACK_MUTE;
        let [bank_a, bank_b, bank_c, _] = button::PAD_BANK;
        let mut app = ready();
        let view = |app: &App| app.step_pads().expect("the pads are steps");
        app.pad_down(4, 100);
        app.pad_up(4);
        app.open_tab(View::Looper);
        // A quick press: the pad played last (pad 5), page 1, on the
        // Sequencer's Steps.
        let fx = click(&mut app, STEP_SEQ);
        assert!(fx.contains(&Effect::Light(STEP_SEQ, LIGHT_ON)), "{fx:?}");
        assert!(fx.contains(&Effect::Show(View::Sequencer)));
        assert!(fx.contains(&Effect::BankLights));
        assert!(app.pads_are_steps());
        assert_eq!(
            app.status_now().steps,
            Some(StepPads {
                sound: 4,
                pen: step_pads::Pen::of(4),
                page: 0,
                follow: false,
                picking: false,
                copying: false,
                copy_page: None,
            })
        );
        // On any page: the Looper tab keeps them.
        app.open_tab(View::Looper);
        assert!(app.pads_are_steps());
        // PAD BANK: the page; again: page 6; SHIFT + C: page 7.
        let fx = press(&mut app, bank_b);
        assert_eq!(view(&app).page, 1);
        assert!(fx.contains(&Effect::Note("STEP SEQ · page 2 (steps 17-32)".into())));
        press(&mut app, bank_b);
        assert_eq!(view(&app).page, 5);
        held(&mut app, SHIFT, |a| press(a, bank_c));
        assert_eq!(view(&app).page, 6);
        // SHIFT + STEP SEQ: Follow; the playhead turns the page; a page
        // picked by hand turns it off.
        held(&mut app, SHIFT, |a| click(a, STEP_SEQ));
        assert!(view(&app).follow);
        assert!(app.pads_are_steps(), "SHIFT + STEP SEQ leaves the mode on");
        assert!(!app.follow_page(2).is_empty());
        assert_eq!(view(&app).page, 2);
        assert!(app.follow_page(2).is_empty(), "no change, nothing sent");
        let fx = press(&mut app, bank_a);
        assert!(fx
            .iter()
            .any(|e| matches!(e, Effect::Note(n) if n.ends_with("Follow off"))));
        assert!(!view(&app).follow);
        assert!(app.follow_page(3).is_empty(), "Follow is off");
        // The screen's ‹ › turn the pads' page too.
        app.set_step_page(1);
        assert_eq!(view(&app).page, 1);
        // Held: the pads play the kit, and the pad tapped is the sound.
        app.button(STEP_SEQ, true, 0.0);
        assert!(!app.pads_are_steps());
        assert!(view(&app).picking);
        assert_eq!(
            press(&mut app, bank_b),
            [Effect::Bank {
                button: 1,
                shift: false
            }]
        );
        app.pad_down(9, 100);
        app.pad_up(9);
        let fx = app.button(STEP_SEQ, false, 0.0);
        assert!(app.pads_are_steps(), "still on");
        assert_eq!(view(&app).sound, 9);
        assert_eq!(view(&app).page, 1, "the page stays");
        assert!(!fx
            .iter()
            .any(|e| matches!(e, Effect::Show(_) | Effect::PlayView(_))));
        // ERASE + PAD BANK: that page of pad 10's steps.
        app.button(button::ERASE, true, 0.0);
        assert_eq!(
            press(&mut app, bank_c),
            [Effect::Steps(StepAction::ClearSteps {
                pad: 9,
                from: 32,
                to: 48
            })]
        );
        app.button(button::ERASE, false, 0.0);
        assert_eq!(view(&app).page, 1, "ERASE + PAD BANK turns no page");
        // COPY + PAD BANK picks a page; another PAD BANK takes the copy
        // (pad 10's steps), and each one after takes it too.
        let fx = app.button(button::COPY, true, 0.0);
        assert!(view(&app).copying);
        assert!(fx.iter().any(|e| matches!(e, Effect::Status(_))));
        let fx = press(&mut app, bank_a);
        assert!(fx.contains(&Effect::Tell(
            "COPY: page 1 picked; press the PAD BANK it goes to".into()
        )));
        assert_eq!(view(&app).copy_page, Some(0));
        assert_eq!(
            press(&mut app, bank_c),
            [Effect::Steps(StepAction::CopySteps {
                pad: 9,
                from: 0,
                to: 32,
                count: 16
            })]
        );
        assert_eq!(view(&app).page, 1, "COPY + PAD BANK turns no page");
        app.button(button::COPY, false, 0.0);
        assert!(!app.copy_held());
        assert_eq!(view(&app).copy_page, None, "let go: forgotten");
        // TRACK MUTE takes the pads; STEP SEQ takes them back.
        let fx = press(&mut app, TRACK_MUTE);
        assert!(fx.contains(&Effect::Light(STEP_SEQ, LIGHT_OFF)));
        assert_eq!(app.mute_keys(), MuteKeys::Tracks);
        assert!(!app.pads_are_steps());
        let fx = click(&mut app, STEP_SEQ);
        assert!(fx.contains(&Effect::Light(TRACK_MUTE, LIGHT_OFF)));
        assert_eq!(app.mute_keys(), MuteKeys::Off);
        assert!(app.pads_are_steps());
        // 16 Levels coming on takes them, from TRACK MUTE too; STEP SEQ
        // and TRACK MUTE turn 16 Levels off.
        press(&mut app, TRACK_MUTE);
        press(&mut app, TRACK_MUTE);
        click(&mut app, STEP_SEQ);
        let fx = app.levels_shown(true, true, false);
        assert!(fx.contains(&Effect::Light(STEP_SEQ, LIGHT_OFF)));
        assert!(!app.pads_are_steps());
        assert!(click(&mut app, STEP_SEQ).contains(&Effect::SixteenLevels));
        app.levels_shown(false, false, false);
        click(&mut app, STEP_SEQ);
        app.levels_shown(true, true, false);
        assert!(press(&mut app, TRACK_MUTE).contains(&Effect::SixteenLevels));
        app.levels_shown(false, false, false);
        press(&mut app, TRACK_MUTE);
        click(&mut app, STEP_SEQ);
        // The chip, or STEP SEQ again: the pads play.
        assert!(!app.steps_off().is_empty());
        assert_eq!(app.step_pads(), None);
        assert!(app.steps_off().is_empty());
        assert_eq!(
            press(&mut app, bank_b),
            [Effect::Bank {
                button: 1,
                shift: false
            }]
        );
    }

    #[test]
    fn shift_stop_goes_back_to_bar_1_and_shift_rec_captures() {
        const SHIFT: u8 = button::SHIFT;
        let mut app = ready();
        let fx = held(&mut app, SHIFT, |a| press(a, button::STOP));
        assert!(fx.contains(&Effect::Time(TimeAction::Transport(Command::StopAndRewind))));
        assert!(fx.contains(&Effect::StopSounds));
        assert!(
            fx.contains(&Effect::Looper(LoopAction::StopFree)),
            "free loops too"
        );
        assert!(!fx.contains(&Effect::Show(View::Console)));
        // SHIFT + REC (90 here): Capture of notes; on the Looper tab, of
        // what the looper heard.
        app.open_tab(View::Play);
        assert_eq!(
            held(&mut app, SHIFT, |a| press(a, 90)),
            [Effect::Time(TimeAction::Capture)]
        );
        app.open_tab(View::Looper);
        assert_eq!(
            held(&mut app, SHIFT, |a| press(a, 90)),
            [Effect::Looper(LoopAction::Capture)]
        );
        // SHIFT + PLAY START (93): the metronome.
        let fx = held(&mut app, SHIFT, |a| press(a, 93));
        assert_eq!(fx[0], Effect::Metronome { on: true, db: -24 });
        assert!(app.save_dirty, "the toggle is saved");
        let fx = held(&mut app, SHIFT, |a| press(a, 93));
        assert_eq!(fx[0], Effect::Metronome { on: false, db: -24 });
    }

    /// Sample Recall (sampler R5 part 2): a kit pad held + SHIFT + REC
    /// opens it on any page; while it is open PLAY, REC and STOP are its,
    /// and SHIFT + REC alone is still Capture. With STEP SEQ on, a held pad
    /// is a step, so SHIFT + REC stays Capture (the sequencer thread's
    /// condition, 18 Nov).
    #[test]
    fn a_pad_held_with_shift_rec_opens_sample_recall_and_it_takes_play_rec_and_stop() {
        const SHIFT: u8 = button::SHIFT;
        let mut app = ready();
        app.set_pad_layout(None);
        app.open_tab(View::Arrange);
        app.pad_down(5, 100);
        assert_eq!(app.recall_pad(), Some(5));
        assert_eq!(
            held(&mut app, SHIFT, |a| press(a, 90)),
            [Effect::Recall(RecallKey::Open { pad: 5 })]
        );
        app.pad_up(5);
        assert_eq!(app.recall_pad(), None);
        // The screen opened it.
        app.set_overlay(true);
        app.set_recall(true);
        assert_eq!(
            press(&mut app, button::PLAY),
            [Effect::Recall(RecallKey::Audition)]
        );
        assert_eq!(press(&mut app, 90), [Effect::Recall(RecallKey::Keep)]);
        assert_eq!(
            press(&mut app, button::STOP),
            [Effect::Recall(RecallKey::Close)]
        );
        assert_eq!(
            held(&mut app, SHIFT, |a| press(a, 90)),
            [Effect::Time(TimeAction::Capture)]
        );
        // A change of page closes it: PLAY plays again.
        app.open_tab(View::Play);
        assert!(!app.recall_open());
        assert!(!press(&mut app, button::PLAY)
            .iter()
            .any(|e| matches!(e, Effect::Recall(_))));
        // STEP SEQ: the held pad is a step.
        click(&mut app, button::STEP_SEQ);
        app.pad_down(5, 100);
        assert_eq!(app.recall_pad(), None);
        assert_eq!(
            held(&mut app, SHIFT, |a| press(a, 90)),
            [Effect::Time(TimeAction::Capture)]
        );
    }

    /// Layers (the instrument redesign's step 11a and 11b): with Layer on
    /// in the list of synths, a tap adds a synth to what the pads play,
    /// from a free place of its own, up to four; a second tap takes it
    /// off; with Layer off a tap plays one synth alone again. The layers
    /// keep their places from being put away, and Cancel gives back the
    /// layers the pads had.
    #[test]
    fn layer_on_adds_synths_to_the_pads_up_to_four_and_takes_them_off() {
        use crate::instrument_list::InstrumentList;
        use crate::instruments::free;
        use crate::synth::Engine;
        let mut app = ready();
        app.open_tab(View::Synth);
        let view = |fx: &[Effect]| match fx.last() {
            Some(Effect::SynthPick(v)) => v.clone().expect("open"),
            other => panic!("{other:?}"),
        };
        app.open_synth_pick();
        app.pick_pads_synth(2);
        let mut list = InstrumentList::new();
        list.follow_free(1);
        let fm = list.free_of(0).expect("an id");
        app.set_instrument_list(&list);
        assert_eq!(app.pads_focus(), PadFocus::Instrument(fm));
        assert!(!view(&app.synth_pick_layer(true)).bank.unwrap().1);
        // Pluck: a place of its own, named before it is in.
        let fx = app.pick_pads_synth(4);
        assert_eq!(
            fx[0],
            Effect::TrackSound {
                track: free(1),
                id: Some(Engine::Pluck.node_id())
            },
            "{fx:?}"
        );
        assert!(
            fx.iter().any(|e| matches!(e, Effect::Note(n) if n == "The pads play FM + Pluck · each on its own strip in the mixer")),
            "{fx:?}"
        );
        assert_eq!(app.pads_layer_names().as_deref(), Some("FM + Pluck"));
        list.follow_free(0b11);
        let pluck = list.free_of(1).expect("an id");
        app.set_instrument_list(&list);
        assert_eq!(app.pad_layers().collect::<Vec<_>>(), [pluck]);
        assert_eq!(app.pads_layer_holders(), [Some(free(1)), None, None]);
        let v = view(&app.pick_pads_synth(5));
        assert_eq!(
            (v.now, v.layers.clone()),
            (Some(2), vec![4, 5]),
            "Keys too, going in"
        );
        assert!(
            app.put_away_free().is_empty(),
            "the layers keep their places"
        );
        // Pluck again: off.
        let fx = app.pick_pads_synth(4);
        assert!(
            fx.iter().any(|e| matches!(e, Effect::Note(n) if n == "Pluck is off the pads · they play FM + Keys")),
            "{fx:?}"
        );
        // Four at most.
        app.pick_pads_synth(6);
        app.pick_pads_synth(7);
        let fx = app.pick_pads_synth(8);
        assert!(
            matches!(&fx[0], Effect::Tell(t) if t == "The pads play 4 synths, the most: take one off first"),
            "{fx:?}"
        );
        // "Bank B only" turns Layer off; with it off a tap plays FM alone.
        assert!(!view(&app.synth_pick_scope(true)).layer);
        app.synth_pick_scope(false);
        let fx = app.pick_pads_synth(2);
        assert!(
            fx.iter()
                .any(|e| matches!(e, Effect::Note(n) if n.starts_with("The pads play FM alone"))),
            "{fx:?}"
        );
        assert!(!app.has_layers() && app.pads_layer_names().is_none());
        // Keep, then open again with layers: Layer is on, and Cancel
        // gives them back.
        app.close_synth_pick(true);
        app.toggle_pad_layer(pluck);
        let v = view(&app.open_synth_pick());
        assert!(v.layer && v.layers == [4], "{v:?}");
        app.pick_pads_synth(4);
        assert!(!app.has_layers());
        app.close_synth_pick(false);
        assert_eq!(app.pad_layers().collect::<Vec<_>>(), [pluck]);
        assert_eq!(app.pads_focus(), PadFocus::Instrument(fm));
    }

    /// Step 11a: a layer by its id. While the pads follow a track on a
    /// kit, the synth is what they play, alone (a kit can't be a layer);
    /// a second one layers it; taking off the first gives its place to
    /// the next; the last one stays. Follow track gives the layers up, and
    /// so does the arp starting.
    #[test]
    fn a_layer_goes_with_a_chosen_synth_and_never_with_a_kit_or_the_arp() {
        use crate::instrument_list::{InstrumentId, InstrumentList};
        let mut app = ready();
        app.open_tab(View::Synth);
        let mut list = InstrumentList::new();
        list.follow_free(1);
        let fm = list.free_of(0).expect("an id");
        app.set_track_sound(
            crate::instruments::free(0),
            Some(crate::instruments::Held {
                id: crate::synth::Engine::Fm.node_id(),
                name: "FM",
                instance: 0,
            }),
        );
        app.set_instrument_list(&list);
        let shared = InstrumentId::SHARED_SYNTH;
        let fx = app.toggle_pad_layer(shared);
        assert_eq!(fx[0], Effect::PadsFocus(PadFocus::SYNTH), "{fx:?}");
        assert!(
            matches!(&fx[1], Effect::Note(n) if n.contains("a kit can't be a layer")),
            "{fx:?}"
        );
        assert!(!app.has_layers());
        app.toggle_pad_layer(fm);
        assert_eq!(app.pads_layer_names().as_deref(), Some("Poly + FM"));
        assert!(!app.pads_layer_synth(), "Poly is the first");
        assert_eq!(
            app.pads_layer_holders()[0],
            Some(crate::instruments::free(0))
        );
        // Poly off: FM is the first.
        app.toggle_pad_layer(shared);
        assert_eq!(app.pads_focus(), PadFocus::Instrument(fm));
        assert!(!app.has_layers());
        let fx = app.toggle_pad_layer(fm);
        assert!(
            matches!(&fx[..], [Effect::Tell(t)] if t.starts_with("FM is all the pads play")),
            "{fx:?}"
        );
        // Poly as a layer now: the audio loop plays it there.
        app.toggle_pad_layer(shared);
        assert!(app.pads_layer_synth());
        assert_eq!(app.pads_layer_holders(), [None; 3]);
        // Follow track: gone.
        app.pads_follow_track();
        assert!(!app.has_layers() && !app.pads_layer_synth());
        // The arp starting keeps them (sampler step 15c), and a layer can
        // be added with it on.
        app.toggle_pad_layer(fm);
        app.button(REPEAT, true, 0.0);
        assert!(app.generating());
        assert!(!app.has_layers());
        app.toggle_pad_layer(shared);
        assert!(app.has_layers() && app.pads_layer_synth());
        assert_eq!(app.pads_focus(), PadFocus::Instrument(fm));
    }

    /// Sampler step 15b: the layers are saved in the pads part with the
    /// rest of the pad map, and come back with it; a layer the list hasn't
    /// got stays in the map, missing and silent, said; a part that doesn't
    /// read is said and the pads follow the track.
    #[test]
    fn the_layers_save_and_open_with_the_pad_map() {
        use crate::instrument_list::{InstrumentId, InstrumentList};
        let mut app = ready();
        let mut list = InstrumentList::new();
        list.follow_free(0b11);
        let (fm, pluck) = (list.free_of(0).unwrap(), list.free_of(1).unwrap());
        app.set_instrument_list(&list);
        assert_eq!(app.pads_part(), "mpfree-pads 2\nplay track\n");
        app.toggle_pad_layer(fm);
        app.toggle_pad_layer(pluck);
        app.toggle_pad_layer(InstrumentId::SHARED_SYNTH);
        let pads = app.pads_part();
        assert_eq!(
            pads,
            format!(
                "mpfree-pads 2\nplay instrument {fm}\nlayer instrument {pluck}\nlayer instrument 1\n"
            )
        );
        let mut other = ready();
        other.open_list(&list);
        let (notes, _) = other.open_pads(Some(&pads));
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(other.pad_map(), app.pad_map());
        assert_eq!(
            other.pad_layers().collect::<Vec<_>>(),
            [pluck, InstrumentId::SHARED_SYNTH]
        );
        // A layer the list hasn't got: kept, silent, said.
        let (notes, _) = other.open_pads(Some(&format!(
            "mpfree-pads 2\nplay instrument {fm}\nlayer instrument 9\nlayer instrument {pluck}\n"
        )));
        assert!(
            matches!(&notes[..], [n] if n.contains("A layer's instrument isn't in the project's list")),
            "{notes:?}"
        );
        assert_eq!(other.pad_layers().count(), 2, "kept");
        assert_eq!(other.pads_layer_holders()[0], None, "silent");
        assert!(other.pads_layer_holders()[1].is_some());
        assert!(other.pads_part().contains("layer instrument 9\n"), "saved");
        for (bad, why) in [
            ("mpfree-pads 1\nplay track\n", "its first line"),
            ("mpfree-pads 2\nplay track\nlayer kit 2\n", "doesn't read"),
            (
                "mpfree-pads 2\nplay track\nlayer instrument 3\nlayer instrument 3\n",
                "twice",
            ),
            (
                "mpfree-pads 2\nplay track\nlayer instrument 3\nlayer instrument 4\nlayer instrument 5\nlayer instrument 6\n",
                "more than 3 layers",
            ),
        ] {
            let (notes, _) = other.open_pads(Some(bad));
            assert!(
                matches!(&notes[..], [n] if n.contains(why) && n.contains("the pads follow the track")),
                "{bad:?}: {notes:?}"
            );
            assert!(!other.has_layers());
            assert_eq!(other.pad_map(), PadMap::default());
        }
    }

    /// Sampler step 15b: the pad map's saved form carries every entry: a
    /// whole kit as the base, a bank's own with a kit's source bank, a
    /// layer on one bank, and reads back the same.
    #[test]
    fn the_pad_map_reads_back_what_it_writes() {
        use crate::instrument_list::InstrumentId;
        let id = |n| InstrumentId::from_number(n).unwrap();
        let mut map = PadMap {
            base: PadFocus::Instrument(id(7)),
            ..PadMap::default()
        };
        map.banks[1] = Some(BankEntry {
            play: PadFocus::Instrument(id(9)),
            from: Some(0),
        });
        map.banks[2] = Some(BankEntry::of(PadFocus::Track));
        map.layers[0] = Some(LayerEntry::every(id(3)));
        map.layers[1] = Some(LayerEntry {
            id: id(4),
            reach: Some(1),
            from: Some(2),
        });
        let text = map.text();
        assert_eq!(
            text,
            "mpfree-pads 2\nplay instrument 7\nbank B instrument 9 from A\nbank C track\nlayer instrument 3\nlayer instrument 4 bank B from C\n"
        );
        assert_eq!(PadMap::parse(&text), Ok(map));
        assert!(map.layers[1].unwrap().reaches(1) && !map.layers[1].unwrap().reaches(0));
        for bad in [
            "mpfree-pads 2\nbank B track\n",
            "mpfree-pads 2\nplay track\nplay track\n",
            "mpfree-pads 2\nplay track\nbank I track\n",
            "mpfree-pads 2\nplay track\nbank B track\nbank B track\n",
            "mpfree-pads 2\nplay track\nbank B instrument 9 from Z\n",
            "mpfree-pads 2\nplay track\nlayer instrument 3 from\n",
        ] {
            assert!(PadMap::parse(bad).is_err(), "{bad:?}");
        }
    }
}
