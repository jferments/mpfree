//! The app's screens, built from mpfree-ui widgets.
//!
//! - The console (the one-shot steps): a header and the step's log.
//! - The pad player (`play`, and `app` from power-on): a loading page with a
//!   progress bar; Play (pads lit in the kit's colour, the kit list, the level
//!   slider, timing); Synth (the synth's pages, from mpfree-synth-ui); Sample
//!   (the sampler's pad editor, from mpfree-sampler-ui; `sample_page.rs`); Debug
//!   (every MIDI message in and out, events, timing);
//!   Console (the system log, raw); Prefs and each page's options (MENU); and
//!   the button guide. Tabs in the header switch between the main pages.
//! - The arp (`mpfree-arp-ui`): the quick band over the pads while NOTE
//!   REPEAT is held or pads are latched, the Arp panel beside the pads, the
//!   pattern workspace, and the arp chip in the header (a tap opens the
//!   panel). None is a tab: they belong to Play.
//!
//! The pad player's screen runs in its own thread at normal priority, so the
//! audio thread never waits for drawing. The audio thread sends it `ToUi`
//! messages; it sends back `FromUi` actions from touches. It also prints every
//! line to stdout: over SSH that is the workstation's terminal, and when the
//! app runs as a service it is the system log.
//!
//! Until the app is ready the screen thread decides the pages itself: a tap on
//! the loading page shows the console, and a tab goes back. After that,
//! `app.rs` decides, and touches only report what was touched.

use crate::app::{
    tabs, ArpView, GuideView, MapView, MenuView, MuteKeys, PadLevel, Settings, Status, View,
    WorkspaceView,
};
use crate::calibrate_page::CalibrateView;
use crate::chain_page::{ChainPage, Turned};
use crate::debug_page::{AudioNumbers, DebugPages};
use crate::looper_page::{LooperPage, LOOPER_PAGE};
use crate::looping::{LoopWave, LooperView};
use crate::mixing::{PlayFx, PlayLevels};
use crate::pace::{Pace, TOUCH_POLL};
use crate::perf::ProcView;
use crate::play_page::{self, Knob, PadInfo, PadsBar, PlayPage, PlayView, SynthPads, Turn};
use crate::play_widgets::{self, MiniPads, PlayTransport, StepGrid, Throws, XyPad, YourTiming};
use crate::sample_page::{self, SamplePage};
use crate::sampler::{Cloud, Playheads, ViewSwap};
use crate::time::{HitInfo, StepAction, StepsData};
use crate::touch_bell::Doorbell;
use crate::touch_dev::{self, TouchTest, TuneList};
use crate::touchlab_page::{TouchLabView, TouchPage};
use mpfree_arp::{Mode, Pattern};
use mpfree_arp_ui::view::{self as arp_view, ArpAction, ArpPanel, ArpState};
use mpfree_arp_ui::widgets::SliderStack;
use mpfree_arp_ui::workspace::{self, Editor, PatternWorkspace, Row, WsAction};
use mpfree_chain_ui::{ChainAction, ChainState, MacroAction, MacrosState};
use mpfree_fx_ui::FxState;
use mpfree_play::mpfree_param::ParamAddr;
use mpfree_sampler::model::Levels;
use mpfree_sampler::model::Project;
use mpfree_sampler_ui::levels_panel::{self, LevelsAction, LevelsModel, LevelsPanel};
use mpfree_synth_ui::view as synth_view;
use mpfree_synth_ui::{SynthAction, SynthPanel, SynthState};
use mpfree_time_ui::editor::Lens;
use mpfree_time_ui::view::{
    self as time_view, AudioAction, TimePage, TimeState, TransportInfo, CLIPS_HINT, CLIPS_PAGE,
    SONG_HINT,
};
use mpfree_time_ui::TimeAction;
use mpfree_touch::mt::Phase;
use mpfree_touch::pointer::PointerEvent;
use mpfree_touch::profile::MPC_ONE;
use mpfree_touch::touchlab::reader_line;
use mpfree_touch::tuning::{Filter, Param, Tuning};
use mpfree_touch_evdev::{EvdevFile, TouchReader, TouchSource};
use mpfree_ui::fb::FbTarget;
use mpfree_ui::qlink::{MasterBar, QLinkCard, QLinkColumn};
use mpfree_ui::widgets::{
    Card, Chip, Header, Label, ListItem, ListView, Loading, LogPanel, OptionList, OptionRow,
    StatList,
};
use mpfree_ui::{
    Align, Color, EventKind, Fonts, Image, SoftCanvas, Theme, Tone, Touch, Ui, UiEvent,
};
use std::collections::VecDeque;
use std::io::{self, Write};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub const CONSOLE_LAYOUT: &str = "col(header:60, log:*, hint:26)";
/// The mixer, and the Q-Link column beside it as on the Play page.
pub const MIXER_LAYOUT: &str = "col(header:60, row(mixer:*, qlinks:168):*, hint:26)";
pub const SYSLOG_LAYOUT: &str = "col(header:60, syslog:*, hint:26)";
/// Prefs > System info: the software on the left, the hardware on the right.
pub const SYSTEM_LAYOUT: &str = "col(header:60, row(syssw:*, syshw:*):*, hint:26)";
pub const OPTIONS_LAYOUT: &str = "col(header:60, row(options:*, help:470):*, hint:26)";
/// The number pad (Jay after live test 3: type a number for a field with a
/// wide range), over the page it was asked from.
pub const NUMPAD: &str = "numpad";
pub const NUMPAD_PAGE: &str = "Type a number";
const NUMPAD_LAYOUT: &str = "col(header:60, numpad:*, hint:26)";
const NUMPAD_HINT: &str = "Tap the number, then Enter   ·   wheel or - / +: step it   ·   push the wheel: Enter   ·   Cancel: leave it as it was";
/// How long a finger stays on Play's BPM to type the tempo, seconds.
const TEMPO_HOLD: f32 = 0.6;
/// The undo history (Jay after live test 3: hold UNDO to see the page's
/// history and jump back several steps at once), over the page it was
/// opened from.
pub const HISTORY: &str = "history";
pub const HISTORY_PAGE: &str = "Undo history";
const HISTORY_LAYOUT: &str = "col(header:60, history:*, hint:26)";
const HISTORY_HINT: &str = "Tap a step to go back or forward to it   ·   wheel or - / +: pick a step   ·   push the wheel: go there   ·   \
                            UNDO: one step back (SHIFT: forward)   ·   Close: done";
/// How long UNDO is held to open the page's undo history, seconds.
pub const UNDO_HOLD: f32 = 0.6;
/// How often the Synth tab's or the mixer's history is asked for again
/// while its list is open, seconds.
const HISTORY_ASK: f32 = 0.5;
pub const GUIDE_LAYOUT: &str = "col(header:60, row(guide:*, found:400):*, hint:26)";
/// The list of kits, over whatever page showed: a tap on the header's kit
/// chip opens it (Jay's card, 13 Nov 14:57Z).
pub const KITS_PAGE: &str = "Kits";
pub const KITS_LAYOUT: &str = "col(header:60, row(kits:*, kits-card:400):*, hint:26)";
const KITS_HINT: &str =
    "Tap a kit: load it on the pads   ·   Back, the kit chip or a tab: close the list";

const CONSOLE_HINT: &str =
    "Switch the MPC off with its power switch to return to the stock firmware";
/// The hint line while ERASE is held (Jay after live test 3: ERASE + a
/// tap deletes what is tapped).
pub const ERASE_HINT: &str = "ERASE held: tap a clip (Arrange), a lane's name (Play) or a layer (Looper) to delete it   ·   \
                              UNDO brings it back";
const SHIFT_HINT: &str =
    "SHIFT +   wheel: synth octave   ·   FULL LEVEL: Half Level   ·   NOTE REPEAT: Latch   ·   MENU: Prefs   ·   STOP: back to bar 1";
/// TRACK MUTE's keys, over whatever page shows (control map decision 5).
/// The hint while the 16 Levels panel shows on Play.
const LEVELS_HINT: &str = "16 LEVEL or Off: back to the kit   ·   Wheel or tap: what the levels change   ·   \
                           Pick pad, then hit one: the pad they play   ·   Close: keep playing them";

/// The 16 Levels panel's state on the screen thread. Whether 16 Levels are
/// on is the playing kit's (`SamplePage::levels`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct LevelsUi {
    /// The panel shows on Play.
    open: bool,
    /// The kind the levels step through next time they are turned on.
    by: Levels,
    /// Pick pad: the next hit picks the pad the levels play.
    picking: bool,
    /// The pad held now, lit bright on the panel.
    lit: Option<usize>,
    /// What the app was last told (on, open), so it hears each change once.
    told: Option<(bool, bool)>,
}

const TRACK_MUTE_HINT: &str = "TRACK MUTE:   pads 1-8 mute and unmute tracks 1-8   ·   green: playing, red: muted   ·   \
                               SHIFT + TRACK MUTE: the kit's pads   ·   TRACK MUTE: the pads play again";
const PAD_MUTE_HINT: &str =
    "SHIFT + TRACK MUTE:   a pad mutes and unmutes itself in the kit   ·   red: muted   ·   \
                             TRACK MUTE: tracks   ·   SHIFT + TRACK MUTE: the pads play again";
const SHIFT_SYNTH_HINT: &str =
    "SHIFT +   wheel: page   ·   FULL LEVEL: Half Level   ·   NOTE REPEAT: Latch   ·   MENU: Prefs   ·   STOP: back to bar 1";
const SHIFT_SAMPLE_HINT: &str =
    "SHIFT +   wheel: pad   ·   Q-Links: finer   ·   UNDO: redo   ·   MENU: Prefs   ·   STOP: back to bar 1";
const SHIFT_MENU_HINT: &str =
    "SHIFT +   wheel: change the value   ·   MENU: Prefs   ·   STOP: back to bar 1";
/// The browser's hint without a library (no --data).
const NO_LIBRARY_HINT: &str =
    "No library: the app was started without a data folder, so nothing can be saved or opened   ·   BROWSE closes";

/// After an open lands, the engines' news for this long is the open's
/// own, not an edit.
const OPEN_SETTLE: Duration = Duration::from_millis(1500);

/// An opened project's loops as read, with lines about any left out.
type LoopsRead = Result<(mpfree_looper::save::Saved, Vec<String>), String>;
/// An opened project's audio tracks, read on their own thread, with lines
/// for the log.
type AudioRead = Result<(mpfree_audiotracks::save::Saved, Vec<String>), String>;

/// Seconds since 1970 (UTC) now: when a project is saved.
fn now_utc() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

const OPTIONS_HINT: &str =
    "Wheel or tap: pick a row   ·   push the wheel, - / + or the arrows: change it   ·   MENU: close";
/// A menu row the wheel's push opened.
/// The wheel on a page with rows (`app::RowKey`), for the hint lines.
const ROWS_HINT: &str = "Wheel: pick a row   ·   - / +, or push the wheel and turn: change it";
const OPTIONS_EDIT_HINT: &str = "Wheel: change it   ·   push the wheel: set it   ·   - / + or the arrows: change it   ·   MENU: close";
const GUIDE_HINT: &str = "STOP: skip this button   ·   The pads still play while the guide is open";
const LOADING_HINT: &str = "Tap the screen for the console";

/// The arp's pages: the panel beside the pads, the quick band over them,
/// and the pattern workspace (editing, and recording on the pads).
pub const ARP_PAGE: &str = "Arp";
pub const QUICK_PAGE: &str = "Play, NOTE REPEAT held";
/// Play with the 16 Levels panel under the transport card (16 LEVEL).
pub const LEVELS_PAGE: &str = "Play · 16 Levels";

pub const PATTERN_PAGE: &str = "Pattern";
pub const RECORD_PAGE: &str = "Pattern, recording";

/// The screen's size as the player sees it (the MPC One's panel, turned).
pub const SCREEN: (i32, i32) = (1280, 800);

// ---------------------------------------------------------------- console

/// The screen for the one-shot steps: a header and a log.
pub struct Console {
    pub ui: Ui,
    canvas: SoftCanvas,
    fb: Option<FbTarget>,
}

impl Console {
    pub fn new(fonts: Fonts, heading: &str, fb: Option<FbTarget>) -> Console {
        let (w, h) = fb.as_ref().map(|f| f.logical_size()).unwrap_or(SCREEN);
        let mut ui = Ui::new(Theme::dark(), fonts, w, h);
        let mut header = Header::new(&["mpfree"]);
        header.chips.push(Chip::new(heading, Tone::Accent));
        ui.add("header", header);
        ui.add("log", LogPanel::new("Log", true, 400));
        ui.add("hint", Label::new(CONSOLE_HINT, Tone::Dim, Align::Center));
        ui.add_page("console", CONSOLE_LAYOUT)
            .expect("console layout");
        Console {
            ui,
            canvas: SoftCanvas::new(w, h),
            fb,
        }
    }

    pub fn push(&mut self, line: &str) {
        if let Some(log) = self.ui.get_mut::<LogPanel>("log") {
            log.push(line, tone_of(line));
        }
    }

    pub fn set_status(&mut self, text: &str) {
        if let Some(h) = self.ui.get_mut::<Header>("header") {
            h.chips.truncate(1);
            h.chips.push(Chip::new(text, Tone::Dim));
        }
    }

    /// Draw what changed and copy it to the screen.
    pub fn draw(&mut self) -> Result<(), String> {
        let damage = self.ui.paint(&mut self.canvas);
        match self.fb.as_mut() {
            Some(fb) if !damage.is_empty() => fb.present(&self.canvas, &damage).map(|_| ()),
            _ => Ok(()),
        }
    }

    pub fn canvas(&self) -> &SoftCanvas {
        &self.canvas
    }
}

/// A colour for a log line from how it starts.
/// The RAM chip's colour: green under 70 %, amber under 85 %, red above.
fn ram_tone(pct: f64) -> Tone {
    if pct < 70.0 {
        Tone::Good
    } else if pct < 85.0 {
        Tone::Warn
    } else {
        Tone::Bad
    }
}

/// A kit's name as the header's chip shows it: words, not dashes
/// (`ClassicHouse-Kit-Classy` is "ClassicHouse Kit Classy"), at most 28
/// characters; "Kit N" for a kit with no name.
pub fn chip_kit_name(name: &str, kit: usize) -> String {
    let words: String = name
        .chars()
        .map(|c| if c == '-' || c == '_' { ' ' } else { c })
        .collect();
    let words = words.split_whitespace().collect::<Vec<_>>().join(" ");
    if words.is_empty() {
        return format!("Kit {}", kit + 1);
    }
    if words.chars().count() > 28 {
        let cut: String = words.chars().take(27).collect();
        format!("{}…", cut.trim_end())
    } else {
        words
    }
}

/// The pages whose UNDO takes back knob and fader turns ([`crate::undo`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UndoOn {
    /// The Synth tab's sound.
    Synth,
    /// The mixer's strips.
    Mixer,
}

/// How long a told line ([`ToUi::Tell`]) stays in place of the hint.
pub const TELL_SECONDS: f32 = 4.0;
/// Autosave's period while there is unsaved work (Jay's pick, 14 Nov:
/// every 2 minutes, into one Autosave project).
pub const AUTOSAVE_S: f32 = 120.0;

/// A log line as the screen says it: without the log's `area:` tag, a
/// model's refusal in words, starting with a capital.
pub fn tell_text(line: &str) -> String {
    let mut l = line.trim();
    if let Some((tag, rest)) = l.split_once(": ") {
        if !tag.is_empty() && tag.chars().all(|c| c.is_ascii_lowercase() || c == '-') {
            l = rest;
        }
    }
    let mut out = l.to_string();
    // `edit of pad A7 not made (BadRegion("loop mode needs loop points"))`
    if let (Some(a), Some(b)) = (out.find("(BadRegion(\""), out.rfind("\"))")) {
        let why = out[a + 12..b].to_string();
        out = format!("{}: {why}", out[..a].trim_end());
    }
    let mut c = out.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => out,
    }
}

pub fn tone_of(line: &str) -> Tone {
    let l = line.trim_start();
    if l.starts_with("STOP") || l.contains("failed") || l.contains("underrun at") {
        Tone::Bad
    } else if l.contains(" done") || l.starts_with("session accepted") {
        Tone::Good
    } else if l.starts_with("+") || l.starts_with("    ") {
        Tone::Dim
    } else {
        Tone::Normal
    }
}

// ---------------------------------------------------------------- pad player

/// Whose history UNDO and the undo history list follow on the page
/// showing (Jay, 13 Nov 13:53Z: UNDO follows the page you're on).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HistoryOf {
    /// The picked loop's.
    Loop,
    /// The sequencer's takes.
    Takes,
    /// The playing kit's sample edits.
    Sample,
    /// The Synth tab's or the mixer's turns, which the app keeps.
    Turns(UndoOn),
}

/// The undo history open over a page: whose, the page to go back to, and
/// (for the app's histories) when to ask for it again.
#[derive(Clone, Debug)]
struct HistoryOpen {
    of: HistoryOf,
    back: String,
    ask_in: f32,
}

/// Where a typed number goes, and the page the number pad was opened over.
#[derive(Clone, Debug)]
struct Typing {
    to: TypeTo,
    back: String,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum TypeTo {
    /// The Sample tab's field that asked ([`sample_page::SamplePage::typed`]).
    Sample,
    /// The tempo (a long press on Play's BPM).
    Tempo,
}

/// What the pad player shows about one kit.
#[derive(Clone, Debug)]
pub struct KitView {
    pub name: String,
    /// The sample on each pad (pad 1 first), if any.
    pub pads: Vec<Option<String>>,
    /// The kit's pad colour, 0-63 per channel as the controller takes it.
    pub colour: [u8; 3],
    /// The header chip while it plays, if not "Kit N of M · Bank X" (the
    /// synth: "Poly · C3 to D#4").
    pub chip: Option<String>,
    /// The synth's pad layout, for the pads panel's Chromatic | In key
    /// switch; None for a kit.
    pub synth: Option<SynthPads>,
}

/// Timing numbers from the audio thread.
#[derive(Clone, Copy, Debug, Default)]
pub struct Timing {
    pub hits: u64,
    pub periods: u64,
    pub underruns: u64,
    pub voices: usize,
    pub peak_db: f32,
    /// Pad hit read to its first frame leaving the sound card's buffer (ms):
    /// last, smallest, mean, largest. NaN before the first hit.
    pub last_ms: f32,
    pub min_ms: f32,
    pub mean_ms: f32,
    pub max_ms: f32,
    /// Seconds left in the timed `play` step; None in the app.
    pub seconds_left: Option<u64>,
    /// The audio buffer, in periods.
    pub periods_queued: u32,
    /// Render times and engine counters, for the Performance page.
    pub audio: AudioNumbers,
}

/// What each track plays as its own instrument (None: the kit).
pub type TrackSounds = [Option<crate::instruments::Held>; crate::instruments::TRACKS];

/// Writes what each track plays under its name: its own instrument's
/// name, or "Kit".
fn fill_sounds(st: &mut TimeState, sounds: &TrackSounds) {
    for (i, t) in st.tracks.iter_mut().enumerate() {
        let name = sounds.get(i).copied().flatten().map_or("Kit", |h| h.name);
        if t.sound != name {
            t.sound = name.to_string();
        }
    }
}

pub enum ToUi {
    /// A line for the terminal and the Events log.
    Line(String),
    /// A save: what the audio loop's engines hold, for the screen to write
    /// with its own parts and hand to the library (project.rs).
    ProjectParts(Box<crate::project::Collected>),
    /// The parts New project goes back to (sent once, early).
    ProjectDefaults(Box<crate::project::Defaults>),
    /// The CLAP plugins found at start (sent once, early): an open makes a
    /// project's plugins with them, here on the screen thread.
    Plugins(std::sync::Arc<crate::plugins::Plugins>),
    /// What each track plays as its own instrument (None: the kit), for
    /// the line under its name on Arrange. Sent when it changes.
    TrackSounds(Box<TrackSounds>),
    /// An open done: the engines it replaced, to free here.
    ProjectBack(Box<crate::project::Opening>),
    /// Something the project keeps changed with the controls (a Synth tab
    /// option, the arp): the work is unsaved.
    Edited,
    /// A line shown in place of the hint for [`TELL_SECONDS`] (it was
    /// logged as a `Line` too): why something did nothing.
    Tell(String),
    /// UNDO (true: SHIFT + UNDO, redo) on a page whose history the screen
    /// picks ([`Player::undo`]).
    Undo(bool),
    /// The UNDO button went down (true) or up, on any page: held for
    /// [`UNDO_HOLD`], it opens the page's undo history.
    UndoButton(bool),
    /// The Synth tab's or the mixer's undo history, as asked for with
    /// [`FromUi::UndoHistory`]: its rows and the row that is now.
    UndoHistory {
        on: UndoOn,
        rows: Vec<String>,
        now: usize,
    },
    /// REC (false) or OVERDUB (true) on the Play page ([`Player::play_rec`]).
    PlayRec(bool),
    /// Whether the Synth tab's and the mixer's turns ([`crate::undo`])
    /// have something to undo, when that changes.
    TurnsReady {
        synth: bool,
        mixer: bool,
    },
    /// A line for the Events log only (not printed).
    Event(String),
    /// A MIDI message in or out, as one line (also printed).
    Midi(String),
    /// A line of the system log, for the Console page (not printed).
    Console(String),
    /// Loading: what is happening, a second line, and the progress (0-1).
    Progress {
        message: String,
        detail: String,
        value: f32,
    },
    /// Loading stopped: what went wrong.
    Failed {
        message: String,
        detail: String,
    },
    Kits(Vec<KitView>),
    PadDown {
        pad: usize,
        velocity: u8,
    },
    PadUp(usize),
    /// A repeat of a pad that is not held: light it briefly.
    PadFlash {
        pad: usize,
        velocity: u8,
    },
    Kit(usize),
    /// The master fader (the Level), dB.
    Level {
        db: f32,
    },
    /// A finger on Q-Link `0` (0 to 3) or off it: its card lights.
    QLinkTouch(usize, bool),
    /// While the mixer is open: Q-Link `0` (or the wheel) turned by `1`
    /// detents, for the selected strip.
    MixerQLink(usize, i32),
    /// While the mixer's I/O view shows: the data wheel, - or +, for the
    /// section showing.
    IoInput(mpfree_mixer_ui::IoInput),
    /// What the chain editor shows (a few times a second while it is open,
    /// and after each change), boxed as the mixer's state is.
    Chain(Box<ChainState>),
    /// The chain editor's selected effect's values, for its drawings (None:
    /// none selected, or one the pages don't draw).
    ChainFx(Option<Box<FxState>>),
    /// What the macros panel shows and the macros' values (for the macro
    /// banks of the Q-Link column too).
    Macros(Box<MacrosState>),
    /// On the chain editor or the macros panel: Q-Link `0` (or the wheel)
    /// turned by `1` detents.
    ChainQLink(usize, i32),
    /// The wheel on the chain editor: the effect picker's while it is open
    /// (SHIFT: its sections), else Q-Link 1.
    ChainWheel(i32),
    /// The wheel's push on the chain editor (the picker's presets, an add).
    ChainPush,
    /// Learn armed on the chain editor: the next Q-Link turned takes this
    /// knob.
    ChainLearn(ParamAddr),
    /// A MIDI control took the knob learn was armed for (M5): the Q-Links
    /// stop waiting for it.
    ChainLearnDone,
    /// Q-LINK: the Q-Link column's next bank.
    QLinkBank,
    /// The tempo follows an outside clock arriving on this port ("MIDI
    /// In"), or none does: the bar's tempo chip (layout spec, section 4d).
    ClockFrom(Option<String>),
    /// The model the mixer's I/O view's MIDI, Sync and CV sections show
    /// and edit, shared with the audio loop (midi_in.rs, midi_out.rs),
    /// which only ever try_locks it. Once, at start: the sections join the
    /// I/O view after Audio.
    IoModel(mpfree_midi_ui::Shared),
    Session(String, Tone),
    Timing(Timing),
    Show(View),
    Menu(MenuView),
    Guide(GuideView),
    Status(Status),
    Settings(Settings),
    /// The themes folder's theme files, read at start: the file name
    /// (what `Settings::theme` holds when one is picked) and the theme.
    ThemeFiles(Vec<(String, Theme)>),
    ClearLogs,
    ClearConsole,
    /// The synth: its parameters, values and presets (at the start, and
    /// when the Synth tab's options pick the other engine).
    Synth(SynthState),
    /// Every value of the synth, in its parameter order (after a preset).
    SynthValues(Vec<f32>),
    /// The synth's meters, by parameter index.
    SynthMeters(Vec<(usize, f32)>),
    /// Each sounding voice's pressure, 0 to 1, for the Pressure page's
    /// live bars (a few times a second).
    SynthPressures(Vec<f32>),
    /// On the Synth page: a Q-Link (0-3) turned by this many detents.
    SynthQLink(usize, i32),
    /// On the Synth page: the wheel turned (the selected knob).
    SynthWheel(i32),
    /// On the Synth page: the next or previous page.
    SynthPage(i32),
    /// The kits' sampler projects, for the Sample tab (once, after
    /// loading), and the rate to build their engine views for.
    SampleKits {
        projects: Vec<Project>,
        rate: u32,
    },
    /// The audio thread swapped in a kit view: the box holds the old one,
    /// to free here.
    SampleViewBack(Box<ViewSwap>),
    /// On the Sample page: a Q-Link (0-3) turned by this many detents.
    SampleQLink(usize, i32),
    /// On the Looper tab: a Q-Link (0-3) turned by this many detents.
    LooperQLink(usize, i32),
    /// MAIN on the Play page (None: the next view), STEP SEQ (Beats).
    PlayView(Option<PlayView>),
    /// SAMPLE EDIT: the Sample tab's editor to open.
    SampleEditor(sample_page::Editor),
    /// The data folder's library, for the Pads tab's browser, with its
    /// Recent and Favourites from the store (sent once at start, with
    /// --data).
    Library {
        dir: std::path::PathBuf,
        lists: String,
    },
    /// A control for the Pads tab's browser (its Browse view).
    Browse(crate::app::BrowseKey),
    /// The Pads tab's view to show (PROG EDIT: the Pad view).
    PadsView(usize),
    /// The browser's preview, played out or replaced, to free here.
    PreviewBack(Box<crate::preview::Preview>),
    /// On the Sample page: a Q-Link let go (the turn is one undo step).
    SampleLift,
    /// On the Sample page: SHIFT + - / + (zoom on Wave, Warp and Chop).
    SampleWheel(i32),
    /// The wheel, - / + or the wheel's push on a page with rows (the
    /// Sample and Pads pages, the Synth tab, Arrange): `app::RowKey`.
    Rows(crate::app::RowKey),
    /// On the Sample page: the previous or next pad with a sound.
    SamplePad(i32),
    /// On the Sample page: UNDO (false) or redo (true).
    SampleUndo(bool),
    /// Where the playing kit's voices are (while the Sample page shows).
    SamplePlayheads(Playheads),
    /// The grains of the pad on the Grain tab, filled in by the audio
    /// thread; the box goes back with the next request.
    SampleGrains(Box<Cloud>),
    /// The sampler's input writer, handed back once the line input is off,
    /// for the Sample page to free.
    SampleInputBack(Box<crate::sampler::RecFeed>),
    /// One kit's entry changed: its name, pad labels and chip (the synth,
    /// after an octave, engine or pad layout change).
    KitChanged {
        kit: usize,
        view: KitView,
    },
    /// Open the Touch lab page; any `Show` closes it again.
    TouchLab,
    /// Open the Calibrate touch page; any `Show` closes it again.
    CalibrateTouch,
    /// The touch tuning kept in the store (at the start).
    TouchTuning(Tuning),
    /// On the Dev tab's Touch page: the wheel (pick a value)...
    TouchWheel(i32),
    /// ...and - / + (step it, or run the action picked).
    TouchStep(i32),
    /// What the arp screens show. (Patterns are large, so they travel
    /// boxed: the channel keeps a slot of the largest message per place.)
    Arp(Box<ArpView>),
    /// 16 LEVEL: 16 Levels on the playing kit, of the pad hit last, with
    /// the panel on Play; or, while they are on, off.
    SixteenLevels,
    /// The wheel while the 16 Levels panel shows: the next or previous
    /// kind.
    LevelsWheel(i32),
    /// The arp's pattern list.
    Patterns(Vec<(String, Pattern)>),
    Workspace(Box<WorkspaceView>),
    /// A step recorded on the pads: the pattern so far and the next step.
    Recorded(Box<Pattern>, usize),
    /// Everything the time engine's screens show, when the clips, the
    /// launches or the takes change.
    Time(Box<TimeState>),
    /// The transport alone (the position moves every period).
    Transport(TransportInfo),
    /// The data wheel in the workspace.
    WorkspaceWheel(i32),
    /// The data wheel on the Arrange page: step the selected record setting,
    /// or scroll the grid.
    ClipsWheel {
        delta: i32,
        scroll: bool,
    },
    /// The Map controls page (debug mode).
    Map(Box<MapView>),
    /// The audio loop's CPU by engine numbers, sent once at the start; the
    /// page reads them while it shows.
    Engines(Arc<crate::engines::EngineShared>),
    /// What the Looper tab shows: the looper, the line input's meter.
    Looper(Box<LooperView>),
    /// The audio tracks' level bars, 0 to 1, by track (Arrange's Song
    /// view, a few times a second).
    AudioLevels([f32; mpfree_audiotracks::MAX_TRACKS]),
    /// The loop's waveform (no frames: the loop was cleared).
    LoopWave(Arc<LoopWave>),
    /// What the mixer panel shows (its strips and meters), a few times a
    /// second while it is open (`mixing.rs`).
    Mixer(Box<mpfree_mixer_ui::MixState>),
    /// The Play page's pattern: the armed track's clip, when it changes.
    Steps(Box<StepsData>),
    /// A pad hit while the transport rolled, for the Play page's timing.
    Hit(HitInfo),
    /// The mixer's values the Play page's Q-Links and speakers show.
    PlayLevels(PlayLevels),
    /// On the Play page (no arp): Q-Link `0` turned by `1` detents.
    PlayQLink(usize, i32),
    /// The master meter: each side's peak and Master safety's gain
    /// reduction, dB (a few times a second).
    Meter {
        peak_db: [f32; 2],
        limit_db: f32,
    },
    /// The touchscreen has something to read (touch_bell.rs): the screen
    /// thread reads it on this pass.
    TouchBell,
    Quit,
}

/// Not `Clone`: [`FromUi::SampleInput`] carries the one writer of the
/// sampler's input buffer.
#[derive(Debug, PartialEq)]
pub enum FromUi {
    Pad {
        pad: usize,
        down: bool,
    },
    /// Save the project: the audio loop sends its engines' parts back
    /// (`ToUi::ProjectParts`).
    SaveProject,
    /// Open a project (or a new one): the pieces built here, swapped in
    /// between two periods (project.rs).
    OpenProject(Box<crate::project::Opening>),
    /// A tap on track `n`'s sound line on Arrange: the track is selected
    /// (armed) and the Synth tab's MENU opens on its Instrument row.
    TrackInstrument(usize),
    /// An opened project's loops, read on their own thread: they go in
    /// once the old ones have faded out.
    OpenLoops(Box<mpfree_looper::save::Saved>),
    /// An opened project's audio tracks with their takes' audio, read on
    /// their own thread: they go in once the old ones are emptied.
    OpenAudio(Box<mpfree_audiotracks::save::Saved>),
    /// The Pads tab's switch: Browse or Pad (`mpfree_sampler_ui::pads_tab`).
    PadsView(usize),
    /// The browser's preview: hear this sound once (it replaces one
    /// playing).
    Preview(Box<crate::preview::Preview>),
    /// The browser's preview: stop (it fades out).
    PreviewStop,
    /// The browser's Recent and Favourites changed: keep this text in the
    /// store.
    SaveBrowserLists(String),
    Kit(usize),
    /// The master bar dragged: the master fader, dB.
    LevelDb(f32),
    /// A drag on a Q-Link card: turn that knob by `delta` detents, as the
    /// knob itself does.
    QLink {
        knob: usize,
        delta: i32,
    },
    /// The pads panel's switch while the synth has the pads: Chromatic
    /// (false) or In key (true).
    PadsLayout {
        in_key: bool,
    },
    /// A tap on Play's tempo: a tap of the tempo, as TAP TEMPO.
    TapTempo,
    Tab(usize),
    MenuSelect(usize),
    MenuStep(usize, i32),
    GuideButton(usize),
    /// A touch on the Arp panel or the quick band.
    Arp(ArpAction),
    /// A touch in the pattern workspace (its editor has changed already).
    Workspace(Box<WsAction>),
    /// The arp chip in the header: open or close the panel.
    ArpChip,
    /// The Full level or Half level chip in the header: the pads play as
    /// hit again.
    PadLevelOff,
    /// 16 Levels are on the playing kit (`on`; 16 LEVEL lights) and the
    /// panel shows on Play (`open`; the wheel turns its kind).
    Levels {
        on: bool,
        open: bool,
    },
    /// A touch on the transport bar or the Arrange page.
    Time(TimeAction),
    /// A change on the Synth page, for the synth on the audio thread.
    Synth(SynthAction),
    /// UNDO (or SHIFT + UNDO, `redo`) on a page whose turns the audio
    /// thread keeps ([`crate::undo`]).
    UndoOn {
        on: UndoOn,
        redo: bool,
    },
    /// Whether the page showing has something to undo, when that changes
    /// (for the UNDO light).
    UndoReady(bool),
    /// The Synth tab's or the mixer's undo history, for the list that
    /// holding UNDO opens: the app answers with [`ToUi::UndoHistory`].
    UndoHistory(UndoOn),
    /// A list or pad over the page (the number pad, the undo history) is
    /// open (true) or closed: while open, the wheel, its push and - / +
    /// are its (as [`crate::app::RowKey`]s) on every page.
    Overlay(bool),
    /// A kit's engine view, built again after an edit on the Sample page,
    /// for the audio thread to swap in (it sends the old one back).
    SampleView(Box<ViewSwap>),
    /// The Grain tab asks for a pad's grains: the audio thread fills the box
    /// and sends it back.
    SampleGrains(Box<Cloud>),
    /// A tap on a slice on the Chop page: play source frames `start..end`
    /// of pad `pad`'s layer `layer` once, if kit `kit` is still playing.
    Slice {
        kit: usize,
        pad: usize,
        layer: usize,
        start: u64,
        end: u64,
    },
    /// Live chop on the Sample page's Chop tab: pad `pad`'s layer `layer`
    /// plays source frames `start..end` once and the pads played live mark
    /// cuts in it (`on`), or that stops.
    SampleLiveChop {
        kit: usize,
        pad: usize,
        layer: usize,
        start: u64,
        end: u64,
        on: bool,
    },
    /// Flatten on the Sample page's Rec tab: pad `pad` of kit `kit` hit
    /// at `velocity` (`down`) or let go, straight on the sampler (not
    /// through the pads' layout, Note Repeat or the arp), if kit `kit` is
    /// still playing.
    SampleHit {
        kit: usize,
        pad: usize,
        velocity: u8,
        down: bool,
    },
    /// The scene crossfader's values for kit `kit`'s pads (`pad`, field,
    /// value; NaN back to the pad's own), while that kit is the one
    /// playing (`mpfree_sampler::scene`).
    SamplePadLive {
        kit: usize,
        values: Vec<(u8, mpfree_sampler::engine::PadField, f32)>,
    },
    /// A key pressed (`down`) or let go on the Play tab's keyboard: kit
    /// `kit`'s keygroups play `note`, while that kit is the one playing.
    SampleKey {
        kit: usize,
        note: u8,
        velocity: u8,
        down: bool,
    },
    /// The Debug tab's strip: MIDI, Performance or Map controls.
    DebugPage(usize),
    /// A control tapped in the mapper's list (listen for it).
    MapRow(usize),
    /// The mapper card's buttons: Listen, Skip, Clear.
    MapButton(usize),
    /// A button on the Looper tab (`looper_page::BUTTONS`).
    Looper(usize),
    /// A loop layer's control (its speaker on the Play page's loop, its row
    /// on the Looper tab), by the layer's id.
    LoopLayer {
        id: u8,
        command: mpfree_looper::LayerCommand,
    },
    /// The Looper tab: pick loop `n` for the controls.
    LoopPick(usize),
    /// Loop `n`'s mute turned over without picking it (the picked loop's
    /// with None).
    LoopMute(Option<usize>),
    /// The Looper tab: a command for the picked loop (an edit tool, Fade
    /// old, the seam fade, After recording).
    LoopCommand(mpfree_looper::Command),
    /// The Looper tab's undo history: back (below 0) or forward this many
    /// steps.
    LoopSteps(i32),
    /// Use the loop's Resample: pick loop `n` (empty) and record the
    /// other loops into it, as heard.
    LoopResample(usize),
    /// The Looper tab's Recording and input panel: a setting stepped (the
    /// same settings as the looper's options menu).
    LooperSetting(crate::app::Item, i32),
    /// What a touch on the mixer panel (or a Q-Link while it is open)
    /// meant (`mpfree_mixer_ui::take_actions`, `qlink_turn`), for the mixer
    /// on the audio thread.
    Mixer(mpfree_mixer_ui::MixAction),
    /// The top bar's mixer icon: open or close the mixer (the app decides).
    MixerIcon,
    /// The top bar's I/O icon: open the mixer on its I/O view, or close it
    /// (the app decides).
    IoIcon,
    /// Open the mixer's I/O view on the section already picked (the tempo
    /// chip: Sync); it never closes it.
    OpenIo,
    /// A touch on the chain editor, or a Q-Link turned while its knobs
    /// show (`modpanels::ChainHost::action`).
    Chain(ChainAction),
    /// A touch on the macros panel, or a macro bank's Q-Link turned
    /// (`modpanels::MacrosHost::action`).
    Macros(MacroAction),
    /// A long press on a macro card: open the macros panel on this bank
    /// (0: Macros 1-4, 1: 5-8).
    OpenMacros(usize),
    /// A Q-Link took the knob learn was armed for.
    ChainLearned,
    /// The Play page: a filter, a send, a fader, a speaker, a throw or the
    /// XY pad, for the mixer (`mixing.rs`).
    PlayFx(PlayFx),
    /// The Play page's pattern: a step tapped or changed.
    Steps(StepAction),
    /// The sampler's line input: its writing end, for the audio thread to
    /// fill each period (`sampler::RecFeed`); None asks for it back, to be
    /// freed here ([`ToUi::SampleInputBack`]).
    SampleInput(Option<Box<crate::sampler::RecFeed>>),
    /// What the sampler's input records from: the line input or a point in
    /// the mix (the Rec tab's Source). The audio thread points the mixer's
    /// sampler capture point at it and starts the buffer afresh.
    SampleSource(crate::sampler::RecSource),
    /// The Sample tab's FX tab on the playing kit's pad `pad`: the chain
    /// editor on its channel, then the Drums track
    /// ([`crate::modpanels::Panels::open_pad`]).
    SampleFx {
        pad: usize,
    },
    /// A phrase chopped on the Sample page, for the armed track's first
    /// empty slot ([`crate::time::TimeHost::put_clip`]).
    SamplePhrase(Box<mpfree_seq::Clip>),
    /// Save on the Touch page: keep this tuning in the store.
    SaveTouch(Tuning),
}

/// Lines the console keeps while paused, to show when it carries on.
const CONSOLE_BACKLOG: usize = 600;

pub struct Player {
    pub ui: Ui,
    kits: Vec<KitView>,
    kit: usize,
    midi: u64,
    midi_held_back: u64,
    timing: Timing,
    /// The Timing panel's Screen row: how the screen is drawn, and how fast.
    screen_row: (String, Tone),
    /// The timed `play` step (STOP ends it) rather than the app.
    timed: bool,
    /// The menu's selected row is open for the wheel (its push opened it).
    menu_editing: bool,
    /// The Sample page's Play tab showed its keyboard, and a row that pad
    /// hits edit (each changes its hint).
    sample_keys: (bool, bool),
    /// The Chop tab chopped live (its hint says how).
    sample_live: bool,
    /// The view the mixer last said it shows (its state), if it ever did.
    mix_view: Option<mpfree_mixer_ui::MixView>,
    /// The I/O view's sections after Audio, by title, in their order there
    /// (Audio is 0, these 1 on): [`add_io_sections`].
    io_sections: Vec<String>,
    /// Time the screen has run, for the I/O sections' ticks.
    io_clock: Duration,
    /// The app has taken over the pages (after loading).
    ready: bool,
    view: View,
    status: Status,
    settings: Settings,
    /// The themes folder's themes, for Prefs > Appearance.
    theme_files: Vec<(String, Theme)>,
    session: (String, Tone),
    /// Lines for the Console page come from the system log (the app); in the
    /// timed step there is none, so the step's own lines go there.
    console_from_lines: bool,
    console_backlog: VecDeque<String>,
    /// The Synth page, once the synth's state has come.
    synth: Option<SynthPanel>,
    /// The Sample page and the kits' projects it edits (the Pads tab's
    /// screens too).
    pub sample: SamplePage,
    /// The browser, once the app has a library (--data): the Pads tab's
    /// picker, and the whole library over every page (BROWSE).
    browser: Option<mpfree_browser_ui::Browser>,
    /// The I/O view's model (its MIDI setup is the project's `midi` part).
    io_model: Option<mpfree_midi_ui::Shared>,
    /// The parts New project goes back to, once the audio loop sent them.
    project_defaults: Option<crate::project::Defaults>,
    /// The plugins an open makes a project's plugin effects and synths
    /// with ([`ToUi::Plugins`]; none until then).
    project_plugins: std::sync::Arc<crate::plugins::Plugins>,
    /// What each track plays ([`ToUi::TrackSounds`]), written under its
    /// name on Arrange.
    track_sounds: TrackSounds,
    /// Prefs > At start was looked at (once, when the sound is going).
    start_checked: bool,
    /// The last run crashed: Prefs > At start opens nothing this time
    /// (crash.rs, the safe start).
    crashed: bool,
    /// Seconds of unsaved work until the next autosave, or, while one is
    /// asked for, until it is given up as lost.
    autosave_in: f32,
    /// The parts asked for are for autosave, not for a save.
    autosaving: bool,
    /// An edit since autosave last asked for the parts: idle unsaved work
    /// is autosaved once, not every 2 minutes.
    autosave_dirty: bool,
    /// Autosave is off this run: the last run's unsaved work couldn't be
    /// set aside, and autosave would write over it.
    autosave_off: bool,
    /// The sound's rate, for an opened project's engines.
    project_rate: u32,
    /// When the last open landed ([`OPEN_SETTLE`]).
    opened_at: Option<Instant>,
    /// How many of the actions in the outbox an open or New project put
    /// there (with any before them): the open's own, never an edit,
    /// however long its own work (the kits, say) took on a slow machine.
    open_actions: usize,
    /// An opened project's loops being read (the files can be big).
    loops_load: Option<std::sync::mpsc::Receiver<LoopsRead>>,
    /// An opened project's audio tracks being read (the takes' files can
    /// be big).
    audio_load: Option<std::sync::mpsc::Receiver<AudioRead>>,
    /// The browser's preview level, in dB (its Q-Link 3).
    preview_db: f32,
    /// A sound file being read for the preview.
    preview_load: Option<std::sync::mpsc::Receiver<sample_page::Loaded>>,
    /// The widget of the touch page on screen (Touch lab, Calibrate touch),
    /// over `view`, which it returns to.
    touch_page: Option<&'static str>,
    arp_panel: ArpPanel,
    /// The 16 Levels panel's state (16 LEVEL).
    levels: LevelsUi,
    pub workspace: PatternWorkspace,
    /// What the arp screens show, from the app.
    pub arp: ArpState,
    /// The Q-Link slider a finger is on, on the panel.
    qlink_focus: Option<usize>,
    /// Where the arp chip is in the header's chips.
    arp_chip: Option<usize>,
    /// Where an outside clock comes from, while the tempo follows one.
    clock_from: Option<String>,
    /// The tempo chip's place among the header's chips, while it shows.
    tempo_chip: Option<usize>,
    /// The kit chip's place among the header's chips: a tap opens the list
    /// of kits over the page.
    kit_chip: Option<usize>,
    /// The Full level or Half level chip's place: a tap turns it off.
    level_chip: Option<usize>,
    /// The list of kits is open over the page (the kit chip); the view
    /// underneath stays `view`.
    kit_list: bool,
    /// What each header chip that only shows (CPU, RAM, session, time
    /// left) says when tapped, by its place; empty for the others.
    chip_why: Vec<String>,
    /// Fingers on the screen: a page change lets go of them first, so a pad
    /// held on the screen never sticks.
    fingers: Vec<u64>,
    /// Actions that came from something other than a touch (a page change
    /// letting go of a pad, the wheel in the workspace), for the audio thread.
    outbox: Vec<FromUi>,
    /// The time engine's bar and Arrange page, and what they show.
    time_page: TimePage,
    pub time: TimeState,
    /// The Debug tab's Performance and Map controls pages.
    debug: DebugPages,
    /// The audio loop's CPU by engine numbers (None until the loop starts).
    engines: Option<Arc<crate::engines::EngineShared>>,
    /// The Looper tab, and what it shows.
    // Boxed: it holds every loop's status and layer rows, and a Player is
    // moved by value often enough (sample_screens) that its size matters.
    looper_page: Box<LooperPage>,
    pub looper: LooperView,
    /// The Play page: its view (the screen's own choice, kept while other
    /// tabs show), pattern, throws, timing and Q-Links.
    pub play: PlayPage,
    /// The chain editor and the macros panel, and the Q-Link column's
    /// macro banks. Boxed for the same reason as the Looper tab.
    chain_page: Box<ChainPage>,
    /// When the CPU chip was last read.
    cpu_at: f32,
    /// The memory in use, % of all ([`Player::set_ram`]); None until read.
    ram_pct: Option<f64>,
    /// Touch tuning: every finger goes through the filter first (the
    /// correction and the hold filter); `tuning` is what it applies and
    /// `saved_tuning` what the store has.
    filter: Filter,
    tuning: Tuning,
    saved_tuning: Tuning,
    /// A line said in place of the hint ([`ToUi::Tell`]) and the seconds
    /// it has left.
    told: Option<(String, f32)>,
    /// Which of the Play page's histories changed last, for UNDO on the
    /// All view: the loop's (true) or the takes' (false); and the counts
    /// each was last seen at.
    undo_loop: bool,
    seen_takes: (usize, usize),
    /// The number pad is open: where the number goes, and the page to go
    /// back to.
    typing: Option<Typing>,
    /// A finger on Play's BPM: which, how long (s) and where it landed.
    tempo_press: Option<(u64, f32, i32, i32)>,
    /// The undo history is open over a page.
    history: Option<HistoryOpen>,
    /// UNDO is held, and for how long (s).
    undo_held: Option<f32>,
    seen_loop: (usize, usize, usize),
    /// The Synth tab's and the mixer's turns can be undone (the audio
    /// thread keeps them), and what the UNDO light was last told.
    turns_ready: (bool, bool),
    undo_ready: bool,
    /// How many times something was said in place (the control audit
    /// counts a tell as a press that did something, even when the same
    /// words were already showing).
    tells: u64,
}

impl Player {
    /// `backdrop`: the picture behind the loading card (the startup screen's
    /// last frame), or None for the built-in one.
    pub fn new(
        fonts: Fonts,
        size: (i32, i32),
        timed: bool,
        backdrop: Option<Arc<Image>>,
    ) -> Player {
        let mut ui = Ui::new(Theme::dark(), fonts, size.0, size.1);
        let names: Vec<&str> = tabs(false).iter().map(|v| v.name()).collect();
        let mut header = Header::new(&names);
        header.mixer_icon = Some(false);
        header.io_icon = Some(false);
        ui.add("header", header);
        let mut loading = Loading::new("MPFree", "Pad player");
        loading.set_backdrop(backdrop);
        loading.hint = LOADING_HINT.to_string();
        loading.message = "Starting".to_string();
        ui.add("loading", loading);
        ui.add(play_page::PADS, MiniPads::default());
        ui.add("kits", ListView::new("Kits"));
        let mut kits_card = Card::new("Load a kit");
        kits_card.heading = "Kits".to_string();
        kits_card.lines = vec![
            (
                "Tap a kit to load it on the pads.".to_string(),
                Tone::Normal,
            ),
            (
                "The list closes and the page comes back.".to_string(),
                Tone::Dim,
            ),
            ("Back keeps the kit that is playing.".to_string(), Tone::Dim),
        ];
        kits_card.buttons = vec!["Back".to_string()];
        ui.add("kits-card", kits_card);
        ui.add("timing", StatList::new("Timing"));
        ui.add(play_page::TRANSPORT, PlayTransport::default());
        ui.add(play_page::MASTER, MasterBar::new());
        ui.add(play_page::QLINKS, QLinkColumn::new());
        ui.add(play_page::STEPS, StepGrid::default());
        ui.add(play_page::THROWS, Throws::default());
        ui.add(play_page::XY, XyPad::default());
        ui.add(play_page::TIMING, YourTiming::default());
        ui.add(play_page::SYNTH_PADS_HEAD, PadsBar::default());
        ui.add("midi", LogPanel::new("MIDI in and out", true, 300));
        ui.add("events", LogPanel::new("Events", false, 200));
        ui.add("syslog", LogPanel::new("Console", true, CONSOLE_BACKLOG));
        ui.add("options", OptionList::new("Options"));
        ui.add("help", Card::new("About this setting"));
        ui.add(NUMPAD, mpfree_ui::numpad::NumPad::default());
        ui.add(HISTORY, mpfree_ui::history::HistoryPanel::default());
        ui.add("guide", Card::new("Button guide"));
        ui.add("found", StatList::new("Buttons"));
        // The hint line starts where the panels do (the mockups).
        ui.add("hint", Label::new("", Tone::Dim, Align::Left).inset(18));
        ui.add("syssw", StatList::new("MPFree and the system"));
        ui.add("syshw", StatList::new("Hardware"));
        let time_page = TimePage::add_to(&mut ui).expect("clips layout");
        ui.add_full_page("Loading", "loading")
            .expect("loading layout");
        // The Looper tab's widgets first: the Play views show some of them.
        let looper_page = Box::new(LooperPage::add(&mut ui).expect("looper layout"));
        for v in PlayView::ALL {
            ui.add_page(v.page(), &v.layout()).expect("play layout");
        }
        ui.add_page(
            play_page::BEATS_TIMING_PAGE,
            &PlayView::Beats.layout_for(true),
        )
        .expect("beats with timing layout");
        let debug = DebugPages::add(&mut ui).expect("debug layouts");
        ui.add_page("Console", SYSLOG_LAYOUT)
            .expect("console layout");
        ui.add_page("Prefs", OPTIONS_LAYOUT).expect("prefs layout");
        ui.add_page("Options", OPTIONS_LAYOUT)
            .expect("options layout");
        ui.add_page("Guide", GUIDE_LAYOUT).expect("guide layout");
        ui.add_page(NUMPAD_PAGE, NUMPAD_LAYOUT)
            .expect("number pad layout");
        ui.add_page(HISTORY_PAGE, HISTORY_LAYOUT)
            .expect("undo history layout");
        ui.add_page(KITS_PAGE, KITS_LAYOUT).expect("kits layout");
        ui.add_page("System info", SYSTEM_LAYOUT)
            .expect("system info layout");
        let patterns = crate::app::built_in_patterns();
        let pattern = patterns.first().map(|(_, p)| *p).unwrap_or_default();
        let name = patterns.first().map(|(n, _)| n.clone()).unwrap_or_default();
        let arp = ArpState {
            settings: mpfree_arp::Settings::default(),
            pattern,
            patterns,
            pattern_index: 0,
            pattern_changed: false,
            notes: Vec::new(),
            step: 0,
            latched: false,
            bpm: 120.0,
        };
        let mut arp_panel =
            ArpPanel::add(&mut ui, ARP_PAGE, QUICK_PAGE, &arp).expect("arp layouts");
        arp_panel
            .set_layout_wrap(&mut ui, play_page::framed)
            .expect("arp layouts in the Play frame");
        arp_panel
            .set_quick_layout(&mut ui, &PlayView::All.quick_layout())
            .expect("quick band layout");
        ui.add(levels_panel::PANEL, LevelsPanel::new());
        ui.add_page(
            LEVELS_PAGE,
            &play_page::frame(&format!("{}:*", levels_panel::PANEL)),
        )
        .expect("16 Levels layout");
        let workspace = PatternWorkspace::add(&mut ui, PATTERN_PAGE, RECORD_PAGE, &name, pattern)
            .expect("workspace layouts");
        ui.add("touchlab", TouchLabView::new());
        ui.add_full_page("Touch lab", "touchlab")
            .expect("touch lab layout");
        ui.add("calibrate", CalibrateView::new());
        ui.add_full_page("Calibrate touch", "calibrate")
            .expect("calibrate layout");
        let sample = SamplePage::add(&mut ui).expect("sample layout");
        // The browser's widget, shown beside the pads on the Pads tab
        // once there is a library.
        ui.add(
            mpfree_browser_ui::BROWSER,
            mpfree_browser_ui::BrowserView::new(mpfree_browser_ui::Screen::empty()),
        );
        // The mixer over the page, under the top bar, with the Q-Link cards
        // beside it (they turn the selected strip).
        mpfree_mixer_ui::add(&mut ui).expect("mixer layout");
        ui.add_page(mpfree_mixer_ui::MIXER_PAGE, MIXER_LAYOUT)
            .expect("mixer layout");
        // The browser over the page, under the top bar (BROWSE): the same
        // widget as the Pads tab's Browse view.
        ui.add_page(
            mpfree_browser_ui::BROWSER_PAGE,
            mpfree_browser_ui::BROWSER_LAYOUT,
        )
        .expect("browser layout");
        // The I/O view's MIDI, Sync and CV sections join when the model
        // they edit arrives (ToUi::IoModel).
        let io_sections = Vec::new();
        // The chain editor in the mixer's place and the macros panel over
        // the page, with the same Q-Link column (layout spec 4b).
        ChainPage::add(&mut ui).expect("chain layouts");
        let _ = ui.show("Loading");
        let status = Status {
            shift: false,
            pad_level: PadLevel::Off,
            repeat: false,
            latched: false,
            rate: "1/16",
            bpm: 120.0,
            mute_keys: MuteKeys::Off,
            erase: false,
        };
        let mut p = Player {
            told: None,
            undo_loop: false,
            seen_takes: (0, 0),
            typing: None,
            tempo_press: None,
            history: None,
            undo_held: None,
            seen_loop: (0, 0, 0),
            turns_ready: (false, false),
            undo_ready: false,
            tells: 0,
            ui,
            kits: Vec::new(),
            kit: 0,
            midi: 0,
            midi_held_back: 0,
            timing: Timing {
                last_ms: f32::NAN,
                ..Timing::default()
            },
            screen_row: ("-".to_string(), Tone::Dim),
            timed,
            menu_editing: false,
            sample_keys: (false, false),
            sample_live: false,
            mix_view: None,
            io_sections,
            io_clock: Duration::ZERO,
            ready: false,
            view: View::Play,
            status,
            settings: Settings::default(),
            theme_files: Vec::new(),
            session: ("Starting".to_string(), Tone::Dim),
            console_from_lines: timed,
            console_backlog: VecDeque::new(),
            touch_page: None,
            synth: None,
            sample,
            browser: None,
            io_model: None,
            project_defaults: None,
            project_plugins: std::sync::Arc::new(crate::plugins::Plugins::none()),
            track_sounds: [None; crate::instruments::TRACKS],
            start_checked: false,
            crashed: false,
            autosave_in: AUTOSAVE_S,
            autosaving: false,
            autosave_dirty: false,
            autosave_off: false,
            project_rate: 44_100,
            opened_at: None,
            open_actions: 0,
            loops_load: None,
            audio_load: None,
            preview_db: crate::preview::LEVEL_DB,
            preview_load: None,
            arp_panel,
            // 16 Levels start off: the app hears only a change.
            levels: LevelsUi {
                told: Some((false, false)),
                ..LevelsUi::default()
            },
            workspace,
            arp,
            qlink_focus: None,
            arp_chip: None,
            clock_from: None,
            tempo_chip: None,
            kit_chip: None,
            level_chip: None,
            kit_list: false,
            chip_why: Vec::new(),
            fingers: Vec::new(),
            outbox: Vec::new(),
            time_page,
            time: TimeState::default(),
            debug,
            engines: None,
            looper_page,
            looper: LooperView::default(),
            play: PlayPage::default(),
            chain_page: Box::default(),
            cpu_at: 0.0,
            ram_pct: None,
            filter: Filter::new((size.0 as f32, size.1 as f32)),
            tuning: Tuning::default(),
            saved_tuning: Tuning::default(),
        };
        p.time_page.update(&mut p.ui, &p.time);
        p.show_timing();
        p.refresh_chips();
        p.refresh_hint();
        p.refresh_qlinks();
        p.refresh_play();
        p
    }

    /// The page on screen ("Loading" until the app is ready).
    #[cfg(test)]
    pub fn page(&self) -> &str {
        self.ui.page()
    }

    fn set_kits(&mut self, kits: Vec<KitView>) {
        if let Some(l) = self.ui.get_mut::<ListView>("kits") {
            l.items = kits
                .iter()
                .map(|k| ListItem {
                    text: k.name.clone(),
                    detail: format!("{} pads", k.pads.iter().filter(|p| p.is_some()).count()),
                    color: Some(Color::from_pad(k.colour)),
                })
                .collect();
        }
        self.kits = kits;
        self.set_kit(0);
    }

    fn set_kit(&mut self, k: usize) {
        if k >= self.kits.len() {
            return;
        }
        self.kit = k;
        if let Some(g) = self
            .ui
            .get_mut::<MiniPads>(play_page::PADS)
            .map(|m| &mut m.grid)
        {
            for pad in g.pads.iter_mut() {
                pad.level = 0.0;
                pad.held = false;
            }
        }
        self.label_pads();
        if let Some(l) = self.ui.get_mut::<ListView>("kits") {
            l.selected = k;
        }
        self.refresh_chips();
        self.refresh_pads_bar();
    }

    /// The pads panel's heading: the bank and the kit, or the synth with
    /// its Chromatic | In key switch (not for Drums, whose pads are
    /// sounds). The Synth tab's copy always shows the synth. The Play
    /// page's pattern names its lanes after the kit's pads.
    fn refresh_pads_bar(&mut self) {
        let drums_engine = self
            .synth
            .as_ref()
            .is_some_and(|p| p.state.info.id == "mpfree.drums");
        let switch = |s: Option<SynthPads>| s.filter(|_| !drums_engine);
        let now = self.kits.get(self.kit);
        let (title, right, synth) = match now {
            Some(kit) => match kit.synth {
                Some(s) => (
                    format!("Synth · Bank {}", crate::app::bank_letter(s.bank)),
                    kit.name.clone(),
                    switch(Some(s)),
                ),
                None => (
                    format!("Pads · Bank {}", crate::app::bank_letter(self.kit)),
                    kit.name.clone(),
                    None,
                ),
            },
            None => ("Pads".to_string(), String::new(), None),
        };
        if let Some(m) = self.ui.get::<MiniPads>(play_page::PADS) {
            if m.title != title || m.right != right || m.synth != synth {
                if let Some(m) = self.ui.get_mut::<MiniPads>(play_page::PADS) {
                    m.title = title;
                    m.right = right;
                    m.synth = synth;
                }
            }
        }
        // The Synth tab's is narrower: the bank only.
        let mut bar = PadsBar::default();
        if let Some((_, k)) = self
            .kits
            .iter()
            .enumerate()
            .find(|(_, k)| k.synth.is_some())
        {
            if let Some(s) = k.synth {
                bar = PadsBar {
                    title: format!("Pads · Bank {}", crate::app::bank_letter(s.bank)),
                    synth: switch(Some(s)),
                };
            }
        }
        if let Some(b) = self.ui.get::<PadsBar>(play_page::SYNTH_PADS_HEAD) {
            if *b != bar {
                if let Some(b) = self.ui.get_mut::<PadsBar>(play_page::SYNTH_PADS_HEAD) {
                    *b = bar;
                }
            }
        }
        // The pattern's lanes.
        let pads: Vec<PadInfo> = match now {
            Some(kit) => (0..16)
                .map(|i| PadInfo {
                    label: kit.pads.get(i).cloned().flatten(),
                    color: Some(Color::from_pad(kit.colour)),
                })
                .collect(),
            None => vec![PadInfo::default(); 16],
        };
        if self.play.pads != pads {
            self.play.pads = pads;
            self.refresh_play();
        }
    }

    /// The Q-Link cards: what each knob does on the page shown. While NOTE
    /// REPEAT is held or the Arp panel is open they set the arp (as its
    /// sliders show); on Play, Q-Link 1 is the level, unless Q-LINK shows a
    /// macro bank; on the chain editor, the selected effect's knobs.
    /// Arrange has the same column with Play's knobs and banks (layout spec
    /// section 4c: every sequencer track plays the kit, so there is no
    /// track strip of its own to turn yet).
    fn refresh_qlinks(&mut self) {
        let play_like = matches!(self.view, View::Play | View::Arrange);
        let arp = self.view == View::Arp || (play_like && self.band_showing());
        // The chain editor, the macros panel, and Play on a macro bank.
        let chain = (!arp && self.chain_page.drives(self.view))
            .then(|| self.chain_page.cards(&self.ui.theme));
        let audio = self.audio_cards().map(|k| (k.title(), k));
        let (title, cards) = if let Some((title, cards)) = &chain {
            (title.as_str(), cards.clone())
        } else if let Some(knobs) = self
            .io_showing()
            .then(|| mpfree_mixer_ui::io_qlinks(&self.ui))
            .flatten()
        {
            // An I/O section's own knobs.
            let mut cards: [QLinkCard; 4] = Default::default();
            for (card, k) in cards.iter_mut().zip(&knobs).filter(|(_, k)| k.enabled) {
                *card = QLinkCard::new(&k.label, "I/O", &k.text, k.pos);
            }
            ("Q-Links · I/O", cards)
        } else if self.view == View::Mixer {
            // The selected strip's Volume, Pan, Send A and Send B.
            let q = mpfree_mixer_ui::qlinks(&self.ui);
            let mut cards: [QLinkCard; 4] = Default::default();
            for (card, k) in cards.iter_mut().zip(&q.knobs).filter(|(_, k)| k.enabled()) {
                *card = QLinkCard::new(k.label, &q.name, &k.text, k.pos);
                card.bipolar = k.label == "Pan";
            }
            (
                if q.name.is_empty() {
                    "Q-Links · tap a strip's name"
                } else {
                    "Q-Links · Mixer"
                },
                cards,
            )
        } else if let Some(p) = self.synth.as_ref().filter(|_| self.view == View::Synth) {
            // The Synth tab: what the page's Q-Links turn, named as their
            // sections show them (the badges Q1 to Q4 on the page).
            ("Q-Links · Synth", p.qlink_cards())
        } else if let Some((title, knobs)) = &audio {
            // An audio region picked on Arrange's Song view, or a track
            // panel open (layout spec 4c): the track's level and pan, the
            // region's gain and fades.
            (title.as_str(), knobs.cards.clone())
        } else if arp {
            let rows = self
                .ui
                .get::<SliderStack>(arp_view::QLINKS)
                .map(|q| q.rows.clone())
                .unwrap_or_default();
            let mut cards: [QLinkCard; 4] = Default::default();
            for (card, row) in cards.iter_mut().zip(&rows) {
                let name = row.label.split(" · ").next().unwrap_or(&row.label);
                *card = QLinkCard::new(name, "Arp", &row.text, row.value);
            }
            ("Q-Links · Arp", cards)
        } else {
            let knobs = self.play.knobs();
            let pad = self.sample.pad_knobs(self.play.last_pad);
            let cards: [QLinkCard; 4] = std::array::from_fn(|i| self.play.card(knobs[i], pad));
            if self.view == View::Arrange {
                ("Q-Links · Arrange", cards)
            } else {
                ("Q-Links · Play", cards)
            }
        };
        if let Some(q) = self.ui.get_mut::<QLinkColumn>(play_page::QLINKS) {
            if q.title != title || q.cards != cards {
                q.title = title.to_string();
                q.cards = cards;
            }
        }
    }

    /// Whether the mixer shows its I/O view.
    pub fn io_showing(&self) -> bool {
        self.view == View::Mixer && self.mix_view == Some(mpfree_mixer_ui::MixView::Io)
    }

    /// The data wheel, - or + while the I/O view shows: the section's.
    fn io_input(&mut self, ev: mpfree_mixer_ui::IoInput) {
        if !self.io_showing() {
            return;
        }
        mpfree_mixer_ui::io_input(&mut self.ui, ev);
        self.after_io();
    }

    /// After a key, a Q-Link or a tick on the I/O view: Audio's routing
    /// changes go to the mixer, and the cards and the hint follow the
    /// section (it may have opened or closed a detail).
    fn after_io(&mut self) {
        for a in mpfree_mixer_ui::take_actions(&mut self.ui) {
            self.outbox.push(FromUi::Mixer(a));
        }
        // The sections keep their own settings; the app reads them where
        // they live, so the list of who changed is only a cue to redraw.
        let _ = mpfree_mixer_ui::take_io_changes(&mut self.ui);
        self.refresh_qlinks();
        self.refresh_hint();
        self.say_io_note();
    }

    /// What the I/O section couldn't do (a value at its end, a field that
    /// can't change), said in place.
    fn say_io_note(&mut self) {
        if let Some(n) = mpfree_mixer_ui::take_io_note(&mut self.ui) {
            self.tell(&n);
        }
    }

    /// Opens the mixer's I/O view on the section titled `title` (the tempo
    /// chip: "Sync"). False, and nothing opens, when there is no such
    /// section.
    pub fn open_io_section(&mut self, title: &str) -> bool {
        let Some(i) = self.io_sections.iter().position(|t| t == title) else {
            return false;
        };
        if !mpfree_mixer_ui::io_select(&mut self.ui, i + 1) {
            return false;
        }
        self.outbox.push(FromUi::OpenIo);
        true
    }

    /// Whether the Q-Link column's card `k` is blank: nothing on this page
    /// for it to move.
    fn qlink_blank(&self, k: usize) -> bool {
        self.ui
            .get::<QLinkColumn>(play_page::QLINKS)
            .is_some_and(|q| q.cards.get(k).is_some_and(|c| c.name.is_empty()))
    }

    /// Why a blank Q-Link card has nothing to move, for the hint line.
    fn qlink_blank_why(&self) -> &'static str {
        match self.view {
            View::Mixer if mpfree_mixer_ui::qlinks(&self.ui).name.is_empty() => {
                "Tap a strip's name: its Volume, Pan and sends go on the Q-Links"
            }
            View::Mixer => "That strip has nothing on this Q-Link",
            View::Chain
                if self
                    .ui
                    .get::<mpfree_chain_ui::ChainView>(mpfree_chain_ui::CHAIN)
                    .is_none_or(|v| v.state().selected.is_none()) =>
            {
                "Pick an effect in the chain: its knobs go on the Q-Links"
            }
            View::Chain => "This page of the effect has fewer knobs than Q-Links",
            _ => "Nothing on this Q-Link on this page",
        }
    }

    /// Q-Link `k` (a knob, a drag on its card, or the wheel as Q-Link 1)
    /// while the mixer is open: the selected strip's Volume, Pan, Send A or
    /// Send B moves on the panel at once, and the mixer hears it.
    fn mixer_qlink(&mut self, k: usize, d: i32) {
        if self.view != View::Mixer || d == 0 {
            return;
        }
        // An I/O section with its own Q-Link cards takes the turn.
        if self.io_showing() && mpfree_mixer_ui::io_qlinks(&self.ui).is_some() {
            mpfree_mixer_ui::io_qlink_turn(&mut self.ui, k, d, self.status.shift);
            self.after_io();
            return;
        }
        if let Some(a) = mpfree_mixer_ui::qlink_turn(&mut self.ui, k, d, self.status.shift) {
            self.outbox.push(FromUi::Mixer(a));
        }
        self.refresh_qlinks();
    }

    /// Q-Link `k` (a knob, a drag on its card, or the wheel as Q-Link 1)
    /// where the chain page's bindings drive the column: the chain editor's
    /// knobs, a macro, or the knob a learn was waiting for. The card moves
    /// at once; the mixer and the mod engine hear it.
    fn chain_qlink(&mut self, k: usize, d: i32) {
        if d == 0 || !self.chain_page.drives(self.view) {
            return;
        }
        match self.chain_page.turn(k, d, self.status.shift) {
            Some(Turned::Chain(a)) => self.outbox.push(FromUi::Chain(a)),
            Some(Turned::Macro(a)) => self.outbox.push(FromUi::Macros(a)),
            Some(Turned::Learned) => self.outbox.push(FromUi::ChainLearned),
            None => {}
        }
        self.refresh_qlinks();
    }

    /// Q-LINK: the column's next bank where it has macro banks (Play, the
    /// chain editor, the macros panel).
    fn next_qlink_bank(&mut self) {
        let play = matches!(self.view, View::Play | View::Arrange) && !self.band_showing();
        if !(play || matches!(self.view, View::Chain | View::Macros)) {
            return;
        }
        if let Some(Turned::Macro(a)) = self.chain_page.next_bank(self.view) {
            self.outbox.push(FromUi::Macros(a));
        }
        println!("q-link bank: {}", self.chain_page.cards(&self.ui.theme).0);
        self.refresh_qlinks();
        self.refresh_hint();
    }

    /// One tap on the switch: the Play page shows view `v`.
    fn show_play_view(&mut self, v: PlayView) {
        if v != self.play.view {
            println!("play view: {}", v.name());
        }
        self.play.view = v;
        if v != PlayView::Beats {
            self.play.selected = None;
        }
        self.refresh_pads_bar();
        self.refresh_play();
        self.refresh_qlinks();
        let timing = self.settings.timing_strip;
        let _ = self
            .arp_panel
            .set_quick_layout(&mut self.ui, &v.quick_layout_for(timing));
        if self.ready && self.view == View::Play {
            self.show_view(View::Play);
        }
    }

    /// Whether Your timing is on screen: Play's Beats view with the
    /// timing strip on.
    fn timing_shown(&self) -> bool {
        self.view == View::Play && self.play.view == PlayView::Beats && self.settings.timing_strip
    }

    /// The Play view showing (for tests and the app check's log).
    #[cfg(test)]
    pub fn play_view(&self) -> PlayView {
        self.play.view
    }

    /// The Play page's panels, from its state: the transport card, the
    /// pattern, the throws, the XY pad and the timing. Only what changed
    /// is marked for drawing.
    fn refresh_play(&mut self) {
        let look = self.play.transport_look();
        if self
            .ui
            .get::<PlayTransport>(play_page::TRANSPORT)
            .is_some_and(|t| t.look != look)
        {
            if let Some(t) = self.ui.get_mut::<PlayTransport>(play_page::TRANSPORT) {
                t.look = look;
            }
        }
        let mut g = StepGrid::default();
        if let Some(old) = self.ui.get::<StepGrid>(play_page::STEPS) {
            g.title.clone_from(&old.title);
            g.right.clone_from(&old.right);
            g.lanes.clone_from(&old.lanes);
            g.steps = old.steps;
            g.play = old.play;
            g.selected = old.selected;
            (g.page, g.pages, g.length) = (old.page, old.pages, old.length);
        }
        self.play.fill_steps(&mut g);
        let same = self
            .ui
            .get::<StepGrid>(play_page::STEPS)
            .is_some_and(|old| {
                old.title == g.title
                    && old.right == g.right
                    && old.lanes == g.lanes
                    && old.steps == g.steps
                    && old.play == g.play
                    && old.selected == g.selected
                    && (old.page, old.pages, old.length) == (g.page, g.pages, g.length)
            });
        if !same {
            if let Some(w) = self.ui.get_mut::<StepGrid>(play_page::STEPS) {
                w.title = g.title;
                w.right = g.right;
                w.lanes = g.lanes;
                w.steps = g.steps;
                w.play = g.play;
                w.selected = g.selected;
                (w.page, w.pages, w.length) = (g.page, g.pages, g.length);
            }
        }
        let mut th = Throws::default();
        self.play.fill_throws(&mut th);
        let same = self
            .ui
            .get::<Throws>(play_page::THROWS)
            .is_some_and(|old| old.items == th.items && old.right == th.right);
        if !same {
            if let Some(w) = self.ui.get_mut::<Throws>(play_page::THROWS) {
                w.items = th.items;
                w.right = th.right;
            }
        }
        if let Some(xy) = self.ui.get::<XyPad>(play_page::XY) {
            let mut probe = XyPad::default();
            self.play.fill_xy(&mut probe);
            // A finger on the pad puts the dot; otherwise it glides home.
            let at = (!xy.on).then_some((probe.x, probe.y));
            let changed = xy.label != probe.label || at.is_some_and(|a| a != (xy.x, xy.y));
            if changed {
                if let Some(xy) = self.ui.get_mut::<XyPad>(play_page::XY) {
                    xy.label = probe.label;
                    if let Some((x, y)) = at {
                        (xy.x, xy.y) = (x, y);
                    }
                }
            }
        }
        if self.timing_shown() {
            // Compare first: reaching the widget mutably redraws it, and
            // with the transport stopped nothing changes.
            let fresh = self.ui.get::<YourTiming>(play_page::TIMING).and_then(|t| {
                let mut new = t.clone();
                self.play.fill_timing(&mut new);
                (new != *t).then_some(new)
            });
            if let (Some(new), Some(t)) = (fresh, self.ui.get_mut::<YourTiming>(play_page::TIMING))
            {
                *t = new;
            }
        }
    }

    /// A Q-Link on the Play page (no arp): what it turns there.
    /// The Looper tab and the Play strip from the last looper view.
    fn refresh_looper(&mut self) {
        self.looper_page
            .update(&mut self.ui, &self.looper, &self.settings);
    }

    /// A Q-Link on the Looper tab: the picked layer's level and pan, how
    /// much each overdub fades the old layers, the seam fade.
    fn looper_qlink(&mut self, k: usize, ticks: i32) {
        use crate::looper_page::LoopTurn;
        let fine = self.status.shift;
        match self.looper_page.qlink(&mut self.ui, k, ticks, fine) {
            Some(LoopTurn::Layer(l, command)) => {
                if let Some(info) = self.looper_page.layer_info(l) {
                    self.outbox.push(FromUi::LoopLayer {
                        id: info.id,
                        command,
                    });
                }
            }
            Some(LoopTurn::FadeOld(k)) => self
                .outbox
                .push(FromUi::LoopCommand(mpfree_looper::Command::SetFadeOld(k))),
            Some(LoopTurn::Seam(f)) => {
                self.outbox
                    .push(FromUi::LoopCommand(mpfree_looper::Command::Edit(
                        mpfree_looper::Edit::SeamFade(f),
                    )))
            }
            None => {}
        }
    }

    /// ERASE held (Jay after live test 3): a tap on something whose delete
    /// UNDO takes back deletes it. `Some` is the answer to the touch; `None`
    /// lets the tap do what it always does. Only deletes with an undo are
    /// here: an Arrange clip (the takes' undo), a Play lane's steps (the
    /// same), and a Looper tab layer (the looper's undo). Song blocks,
    /// whole loops, effects and macros have no undo yet, so ERASE leaves
    /// them alone.
    fn erase_tap(&mut self, ev: &UiEvent) -> Option<Option<FromUi>> {
        match (ev.widget.as_str(), &ev.kind) {
            (play_page::STEPS, EventKind::Toggle(pad)) if *pad < 16 => {
                Some(Some(FromUi::Steps(StepAction::ClearPad {
                    pad: *pad as u8,
                })))
            }
            (time_view::CLIPS, EventKind::Select(i)) => {
                let slots = mpfree_seq::SLOTS;
                let (track, slot) = (i / slots, i % slots);
                if self.time.clip(track, slot).is_some() {
                    Some(Some(FromUi::Time(TimeAction::DeleteClip { track, slot })))
                } else {
                    self.tell("ERASE: that slot is empty, nothing to delete");
                    Some(None)
                }
            }
            _ => None,
        }
    }

    /// What the Looper tab's panels asked for: the first as the answer to
    /// the touch, the rest after it. With ERASE held, a layer tapped is
    /// deleted (the looper's UNDO brings it back).
    fn looper_acts(&mut self) -> Option<FromUi> {
        use crate::looper_tab::LoopAct;
        let mut out = Vec::new();
        let erase = self.status.erase;
        // A tap on a layer's speaker also picks the row: one delete.
        let mut erased = Vec::new();
        for a in crate::looper_tab::take_actions(&mut self.ui) {
            let a = match a {
                LoopAct::Layer(k) | LoopAct::LayerCmd(k, _) if erase => {
                    if erased.contains(&k) {
                        continue;
                    }
                    erased.push(k);
                    LoopAct::LayerCmd(k, mpfree_looper::LayerCommand::Delete)
                }
                LoopAct::Pick(_) | LoopAct::Clear if erase => {
                    self.tell(
                        "ERASE: a whole loop has no undo yet, so ERASE leaves it; Clear removes it",
                    );
                    continue;
                }
                a => a,
            };
            match a {
                LoopAct::Pick(n) => out.push(FromUi::LoopPick(n)),
                LoopAct::NewLoop => {
                    if let Some(n) = self.looper_page.first_empty() {
                        out.push(FromUi::LoopPick(n));
                    }
                }
                LoopAct::Mute(n) => out.push(FromUi::LoopMute(Some(n))),
                LoopAct::Clear => out.push(FromUi::LoopCommand(mpfree_looper::Command::Clear)),
                LoopAct::Layer(k) => self.looper_page.pick_layer(&mut self.ui, k),
                LoopAct::LayerCmd(k, command) => {
                    if let Some(l) = self.looper_page.layer_info(k) {
                        out.push(FromUi::LoopLayer { id: l.id, command });
                    }
                }
                LoopAct::Setting(i, d) => {
                    if let Some(c) = crate::looper_page::setting_command(i, &self.looper.status) {
                        out.push(FromUi::LoopCommand(c));
                    } else if let Some((item, d)) =
                        crate::looper_page::setting_item(i, d, &self.settings)
                    {
                        out.push(FromUi::LooperSetting(item, d));
                    }
                }
                LoopAct::Tool(crate::looper_page::TOOL_HISTORY) => {
                    self.looper_page.toggle_history(&mut self.ui);
                    self.refresh_hint();
                }
                LoopAct::Trim(from, to) => {
                    if let Some(c) = crate::looper_page::trim_command(from, to, &self.looper.status)
                    {
                        out.push(FromUi::LoopCommand(c));
                    }
                }
                LoopAct::Step(row) => {
                    if let Some(n) = self.looper_page.history_steps(row) {
                        out.push(FromUi::LoopSteps(n));
                    }
                }
                LoopAct::Tool(i) => {
                    let rate = self.looper.rate;
                    if let Some(c) = crate::looper_page::tool_command(i, &self.looper.status, rate)
                    {
                        out.push(FromUi::LoopCommand(c));
                    }
                }
                LoopAct::Use(crate::looper_page::USE_RESAMPLE) => {
                    if let Some(n) = self.looper_page.first_empty() {
                        out.push(FromUi::LoopResample(n));
                    }
                }
                // The bounce to a pad, a clip or a file comes next.
                LoopAct::Use(_) => {}
                // Faint ones say why in place (Jay, 12 Nov).
                LoopAct::FaintTool(i) => {
                    self.tell(crate::looper_page::why_no_tool(i, &self.looper.status));
                }
                LoopAct::FaintUse(i) => {
                    self.tell(crate::looper_page::why_no_use(i, &self.looper));
                }
            }
        }
        let mut it = out.into_iter();
        let first = it.next();
        self.outbox.extend(it);
        first
    }

    fn play_qlink(&mut self, k: usize, ticks: i32) {
        match self.play.turn(k, ticks, self.status.shift) {
            Some(Turn::Fx(fx)) => self.outbox.push(FromUi::PlayFx(fx)),
            Some(Turn::Step(a)) => {
                self.outbox.push(FromUi::Steps(a));
                self.refresh_play();
            }
            Some(Turn::Swing(amount)) => {
                let grid = self.time.transport.swing.grid;
                self.outbox.push(FromUi::Time(TimeAction::Transport(
                    mpfree_transport::Command::SetSwing(mpfree_transport::time::Swing {
                        grid,
                        amount,
                    }),
                )));
            }
            Some(Turn::Pad(knob, _)) if self.sample.pad_knobs(self.play.last_pad).is_none() => {
                // No sound on the pad: say so in place (its card shows "-").
                let what = if knob == Knob::Tune {
                    "tune"
                } else {
                    "shorten"
                };
                self.tell(&format!(
                    "Pad {} has no sound to {what}: hit a pad with one, or load one on the Pads tab",
                    self.play.last_pad + 1
                ));
            }
            Some(Turn::Pad(knob, d)) => {
                let pad = self.play.last_pad;
                let which = if knob == Knob::Tune {
                    sample_page::PadKnob::Tune
                } else {
                    sample_page::PadKnob::Decay
                };
                self.sample.turn_pad(pad, which, d, self.status.shift);
                self.print_sample_lines();
            }
            None => {}
        }
        self.refresh_qlinks();
    }

    /// The pads' names: the kit's sounds, or what each pad writes while a
    /// pattern is recorded on the pads.
    fn label_pads(&mut self) {
        let recording = self.view == View::Pattern && self.workspace.recording;
        let kit = self.kits.get(self.kit).cloned();
        let Some(g) = self
            .ui
            .get_mut::<MiniPads>(play_page::PADS)
            .map(|m| &mut m.grid)
        else {
            return;
        };
        if recording {
            let (notes, marks) = (
                Color::from_pad([0x00, 0x22, 0x3F]),
                Color::from_pad([0x3F, 0x22, 0x00]),
            );
            for (i, (pad, label)) in g
                .pads
                .iter_mut()
                .zip(workspace::record_pad_labels())
                .enumerate()
            {
                pad.label = label.to_string();
                pad.color = (!label.is_empty()).then_some(if i < 9 { notes } else { marks });
            }
        } else if let Some(kit) = kit {
            for (i, pad) in g.pads.iter_mut().enumerate() {
                let label = kit.pads.get(i).cloned().flatten();
                pad.color = label.as_ref().map(|_| Color::from_pad(kit.colour));
                pad.label = label.unwrap_or_default();
            }
        }
    }

    /// The header's chips, right to left: session, kit, level, tempo, then
    /// the modes that are on, and the time left in the timed step.
    fn refresh_chips(&mut self) {
        // The first chip sits at the right end (the mockups: the CPU, then
        // the kit). The session shows only when something is wrong with it.
        let mut chips = Vec::new();
        // A tap on a chip that only shows says what it shows.
        let mut why = Vec::new();
        if let Some((text, tone)) = self.engines.as_deref().and_then(crate::engines::cpu_chip) {
            why.push(format!("{text}: how much of the processor the sound uses"));
            chips.push(Chip::new(text, tone));
        }
        if let Some(p) = self.ram_pct {
            why.push(format!("RAM {p:.0} %: how much of the memory is in use"));
            chips.push(Chip::new(format!("RAM {p:.0} %"), ram_tone(p)));
        }
        if self.session.1 != Tone::Good || !self.ready {
            why.push(format!(
                "{}: the state of the audio session",
                self.session.0
            ));
            chips.push(Chip::new(self.session.0.clone(), self.session.1));
        }
        // A tap on the kit chip opens the list of kits (Jay's card, 13 Nov
        // 14:57Z).
        self.kit_chip = self.kits.get(self.kit).map(|_| chips.len());
        match self.kits.get(self.kit) {
            Some(KitView {
                chip: Some(chip), ..
            }) => chips.push(Chip::new(chip.clone(), Tone::Normal)),
            // The kit's name (live test 3: "Kit 1 of 10" said nothing about
            // which kit was Grains), and its bank.
            Some(k) => chips.push(Chip::new(
                format!(
                    "{} · Bank {}",
                    chip_kit_name(&k.name, self.kit),
                    crate::app::bank_letter(self.kit)
                ),
                Tone::Normal,
            )),
            None => {}
        }
        // While an outside clock sets the tempo, the tempo chip says so,
        // with a green dot (layout spec, section 4d); a tap opens the I/O
        // view at Sync.
        self.tempo_chip = None;
        if let Some(port) = &self.clock_from {
            self.tempo_chip = Some(chips.len());
            chips.push(Chip::new(
                format!("{:.1} from {port}", self.status.bpm),
                Tone::Good,
            ));
        }
        // The level is on the master bar and the tempo on the transport, so
        // the header keeps to the chips that change how the pads play. The
        // arp chip shows while the arp, a repeat or a pattern is on (a tap
        // opens the panel; NOTE REPEAT + MENU opens it too).
        self.arp_chip = None;
        let s = &self.status;
        if self.arp.settings.mode != Mode::Direct || self.arp.latched || s.repeat || s.latched {
            let mut arp = arp_view::chip(&self.arp);
            arp.text = format!("{} · {:.0} BPM", arp.text, self.arp.bpm);
            self.arp_chip = Some(chips.len());
            chips.push(arp);
        }
        // A tap on the level chip turns that level off (Jay's card, 13 Nov
        // 14:57Z).
        self.level_chip = None;
        let level = match self.status.pad_level {
            PadLevel::Full => Some("Full level"),
            PadLevel::Half => Some("Half level"),
            PadLevel::Off => None,
        };
        if let Some(text) = level {
            self.level_chip = Some(chips.len());
            chips.push(Chip::new(text, Tone::Warn));
        }
        why.resize(chips.len(), String::new());
        // 16 LEVEL: the kit's pads play 16 Levels of one pad. A tap says
        // so, and how to turn them off.
        if let Some((pad, by)) = self.sample.levels() {
            let by = by.label().to_lowercase();
            why.push(format!(
                "16 Levels of pad {} by {by}: 16 LEVEL turns them off",
                self.sample.pad_names(pad).0
            ));
            chips.push(Chip::new(format!("16 Levels · {by}"), Tone::Warn));
        }
        if let Some(left) = self.timing.seconds_left {
            why.push(format!("{left} s left in this timed run"));
            chips.push(Chip::new(format!("{left} s left"), Tone::Dim));
        }
        self.chip_why = why;
        // Only when a chip changed: this runs every second, and the header
        // spans the screen, so redrawing it rewrites nearly every row of the
        // panel (it is mounted turned).
        if self
            .ui
            .get::<Header>("header")
            .is_some_and(|h| h.chips != chips)
        {
            if let Some(h) = self.ui.get_mut::<Header>("header") {
                h.chips = chips;
            }
        }
    }

    fn refresh_hint(&mut self) {
        let stop = if self.timed {
            "STOP: end"
        } else {
            "STOP: silence"
        };
        let mode = self.arp.settings.mode;
        let wheel = match mode {
            Mode::Arp => "order",
            Mode::Pattern => "pattern",
            Mode::Direct | Mode::Repeat => "rate",
        };
        let text = if self.typing.is_some() {
            NUMPAD_HINT.to_string()
        } else if self.history.is_some() {
            HISTORY_HINT.to_string()
        } else if self.kit_list {
            KITS_HINT.to_string()
        } else if self.status.shift
            && self.ready
            && matches!(self.view, View::Prefs | View::Options)
        {
            SHIFT_MENU_HINT.to_string()
        } else if self.status.shift && self.ready && self.view == View::Synth {
            SHIFT_SYNTH_HINT.to_string()
        } else if self.status.shift && self.ready && self.view == View::Sample {
            SHIFT_SAMPLE_HINT.to_string()
        } else if self.status.shift && self.ready {
            SHIFT_HINT.to_string()
        } else if self.status.erase && self.ready {
            ERASE_HINT.to_string()
        } else if self.status.mute_keys == MuteKeys::Tracks {
            TRACK_MUTE_HINT.to_string()
        } else if self.status.mute_keys == MuteKeys::Pads {
            PAD_MUTE_HINT.to_string()
        } else if self.view == View::Play && self.band_showing() && self.status.latched {
            format!("Latched: {}   ·   NOTE REPEAT: let go   ·   hold NOTE REPEAT for the wheel, - / + and Q-Links   ·   \
                     {stop}", crate::app::mode_label(mode))
        } else if self.view == View::Play && self.band_showing() {
            format!("Wheel: {wheel}   ·   - / +: Repeat, Arp, Pattern   ·   Q-Links: rate, gate, swing, octaves   ·   \
                     + MENU: Arp panel")
        } else if self.view == View::Play && self.levels.open {
            LEVELS_HINT.to_string()
        } else if self.view == View::Play && self.chain_page.drives(View::Play) {
            format!("Q-Links: {}   ·   Q-LINK: next bank   ·   hold a macro's card: the macros panel   ·   \
                     Wheel: kit   ·   {stop}", self.chain_page.bind.title())
        } else {
            match self.view {
                // Play has no kit list to tap since 12 Nov, and Q-Link 1
                // is Swing: the wheel picks the kit, the master bar is the
                // level. The Drum view is folded into Beats (13 Nov).
                View::Play => format!("Tap All, Beats or Loop: the view   ·   Wheel: kit   ·   the master bar: \
                                       level   ·   NOTE REPEAT: repeat   ·   MENU: options   ·   {stop}"),
                View::Arrange if self.time.edit.is_some() => {
                    time_view::edit_hint(self.time.edit.as_ref().map_or(Lens::Grid, |e| e.editor.lens))
                        .to_string()
                }
                View::Arrange if self.time.song_view => SONG_HINT.to_string(),
                View::Arrange => CLIPS_HINT.to_string(),
                View::Looper if self.looper_page.history_open() => format!("Tap a step: back or forward to it   ·   \
                                         drag: scroll   ·   Undo history: close   ·   UNDO: undo (SHIFT: redo)   ·   {stop}"),
                View::Looper => format!("Tap a loop or a layer: pick it   ·   Q-Links: layer level and pan, Fade old, \
                                         seam fade   ·   REC: record   ·   OVERDUB: overdub   ·   UNDO: undo (SHIFT: redo)   ·   \
                                         {stop}"),
                View::Synth => format!("Pads play {}   ·   Q-Links 1-4: the marked knobs   ·   Wheel: pick a knob   ·   \
                                        - / +, or push the wheel and turn: change it   ·   SHIFT + wheel: page   ·   {stop}",
                                       self.synth.as_ref().map_or("the synth", |p| p.state.info.name)),
                View::Sample if self.sample.page() == sample_page::PAGE => {
                    format!("Hit a pad to edit it   ·   Q-Links 1-4: Start, End, loop   ·   {ROWS_HINT}   ·   \
                     SHIFT + - / +: zoom")
                }
                View::Sample if self.sample.page() == sample_page::WARP_PAGE => {
                    format!("Tap a hit: pin it to the grid   ·   Q-Links 1-4: pin, its beat, warp tempo, scroll   ·   \
                     {ROWS_HINT}   ·   SHIFT + - / +: zoom")
                }
                View::Sample if self.sample.chop_live_method() => {
                    "Start plays the sound: hit pads in time to drop cuts   ·   Pull to hits: each onto the nearest hit   ·   \
                     Stop ends the take   ·   then Chop to: onto the pads".to_string()
                }
                View::Sample if self.sample.page() == sample_page::CHOP_PAGE => {
                    format!("Hit a pad to chop it   ·   tap a slice: hear it   ·   Q-Links 1-4: slice, its start, setting, first pad   ·   \
                     {ROWS_HINT}   ·   SHIFT + - / +: zoom")
                }
                View::Pads if self.sample.page() == sample_page::PADS_BROWSE_PAGE => {
                    "Hit or tap a pad to pick it   ·   Load: onto the pad picked   ·   Load to pad: onto the next pad hit   ·   \
                     PLAY: hear it   ·   Wheel: move   ·   push: open or load   ·   - / +: up / into a folder".to_string()
                }
                View::Pads if self.sample.page() == sample_page::PADS_PAGE => {
                    format!("Hit or tap a pad to pick it, again to hear it   ·   Q-Links 1-4: level, pan, tune, voices   ·   \
                     {ROWS_HINT}")
                }
                View::Pads => format!("Pick a drum kit on Play to see its pads here   ·   {stop}"),
                View::Sample if self.sample.play_takes_pads(&self.ui) => {
                    "Hit pads to add them to the row picked or take them out   ·   Wheel: the next pad   ·   \
                     tap another row when done   ·   UNDO: undo".to_string()
                }
                View::Sample if self.sample.page() == sample_page::PLAY_PAGE && self.sample.play_keys_shown() => {
                    format!("Press keys to hear the kit   ·   drag the zone edges and root   ·   \
                     Q-Links 1-4: Attack, Decay, Cutoff, Resonance   ·   {ROWS_HINT}")
                }
                View::Sample if self.sample.page() == sample_page::PLAY_PAGE => {
                    format!("Hit a pad: its velocity shows on the layers   ·   Q-Links 1-4: Attack, Decay, Cutoff, Resonance   ·   \
                     {ROWS_HINT}")
                }
                View::Sample if self.sample.page() == sample_page::GRAIN_PAGE => {
                    format!("Hit the pad to see its grains   ·   Q-Links 1-4: position, size, density, pitch (SHIFT: sprays, \
                     finer)   ·   {ROWS_HINT}")
                }
                View::Sample if self.sample.page() == sample_page::REC_PAGE => {
                    format!("Record or Skip back onto the pad named   ·   {ROWS_HINT}   ·   UNDO: take a take off")
                }
                View::Sample if self.sample.page() == sample_page::AUTO_PAGE => {
                    format!("Start samples the Synth tab's patch or your gear onto the pads named   ·   {ROWS_HINT}")
                }
                View::Sample => format!("Hit a pad with a sound to edit it   ·   - / + or SHIFT + wheel: next pad with a \
                                         sound   ·   {stop}"),
                View::Debug => format!("Every MIDI message in and out, newest at the bottom   ·   SHIFT + wheel or - / +: \
                                        Dev page   ·   MENU: options   ·   {stop}"),
                View::Perf => format!("CPU per thread and memory every 2 s, audio every 0.25 s; a perf: line goes to \
                                       the log every 30 s   ·   SHIFT + wheel: Dev page   ·   {stop}"),
                View::Engines => format!("Each stage of the audio loop, every 0.25 s; a cpu by engine line goes to the \
                                          log every 30 s   ·   SHIFT + wheel: Dev page   ·   {stop}"),
                View::Procs => format!("Every process on the MPC, read from /proc once a second while this page \
                                        shows   ·   SHIFT + wheel: Dev page   ·   {stop}"),
                View::Touch => "Touch the test area: it is measured before the tuning   ·   Wheel: pick a value   ·   - / +: change it \
                                (applies at once)   ·   MENU: corner check, Touch lab   ·   SHIFT + wheel: Dev page".to_string(),
                View::Map => "Tap a control, then press it   ·   Wheel: pick   ·   +: listen   ·   -: skip   ·   \
                              STOP: stop listening   ·   SHIFT + wheel: Dev page".to_string(),
                View::Console if self.settings.debug_mode => "The system log, as it comes   ·   MENU: pause or clear   ·   \
                                                              SHIFT + wheel: Dev page   ·   SHIFT + ERASE + TAP TEMPO: back".to_string(),
                View::Console => "The system log, as it comes   ·   MENU: pause or clear   ·   SHIFT + ERASE + TAP TEMPO: back"
                    .to_string(),
                View::System => "Read when the page opens   ·   MENU: back to Prefs   ·   a tab: leave".to_string(),
                View::Prefs | View::Options if self.menu_editing => OPTIONS_EDIT_HINT.to_string(),
                View::Prefs | View::Options => OPTIONS_HINT.to_string(),
                View::Mixer => mpfree_mixer_ui::io_hint(&self.ui)
                    .unwrap_or_else(|| mpfree_mixer_ui::MIXER_HINT.to_string()),
                View::Chain if mpfree_chain_ui::chain_picking(&self.ui) => {
                    mpfree_chain_ui::PICKER_HINT.to_string()
                }
                View::Chain => mpfree_chain_ui::CHAIN_HINT.to_string(),
                View::Macros => mpfree_chain_ui::MACROS_HINT.to_string(),
                View::Guide => GUIDE_HINT.to_string(),
                View::Arp => format!("Wheel: {wheel}   ·   - / +: mode   ·   Q-Links: rate, gate, swing, octaves   ·   \
                                      MENU: options   ·   NOTE REPEAT + MENU: close"),
                View::Pattern => self.workspace.hint().to_string(),
                View::Browser if self.browser.is_none() => NO_LIBRARY_HINT.to_string(),
                View::Browser => mpfree_browser_ui::BROWSER_HINT.to_string(),
            }
        };
        let (text, tone) = match &self.told {
            Some((t, _)) => (t.clone(), Tone::Warn),
            None => (text, Tone::Dim),
        };
        if let Some(h) = self.ui.get_mut::<Label>("hint") {
            if h.text != text || h.tone != tone {
                h.text = text;
                h.tone = tone;
            }
        }
    }

    fn show_timing(&mut self) {
        let t = self.timing;
        let ms = |v: f32| {
            if v.is_finite() {
                format!("{v:.1} ms")
            } else {
                "no hit yet".to_string()
            }
        };
        let rows = vec![
            (
                "Pad to sound, last".to_string(),
                ms(t.last_ms),
                Tone::Normal,
            ),
            (
                "Mean · range".to_string(),
                if t.mean_ms.is_finite() && t.hits > 0 {
                    format!("{:.1} · {:.1} to {:.1} ms", t.mean_ms, t.min_ms, t.max_ms)
                } else {
                    "-".to_string()
                },
                Tone::Normal,
            ),
            (
                "Underruns".to_string(),
                format!("{} in {} periods", t.underruns, t.periods),
                if t.underruns == 0 {
                    Tone::Good
                } else {
                    Tone::Bad
                },
            ),
            (
                "Hits · voices · peak".to_string(),
                format!("{} · {} · {:.0} dBFS", t.hits, t.voices, t.peak_db),
                Tone::Normal,
            ),
            (
                "Audio buffer".to_string(),
                format!(
                    "{} periods, {:.1} ms",
                    t.periods_queued,
                    t.periods_queued as f32 * 128.0 / 44.1
                ),
                Tone::Dim,
            ),
            (
                "Screen".to_string(),
                self.screen_row.0.clone(),
                self.screen_row.1,
            ),
        ];
        if let Some(s) = self.ui.get_mut::<StatList>("timing") {
            s.set(rows);
        }
    }

    fn push_console(&mut self, line: &str) {
        if self.settings.console_paused {
            if self.console_backlog.len() >= CONSOLE_BACKLOG {
                self.console_backlog.pop_front();
            }
            self.console_backlog.push_back(line.to_string());
            let n = self.console_backlog.len();
            if let Some(c) = self.ui.get_mut::<LogPanel>("syslog") {
                c.right = format!("paused, {n} new lines");
            }
            return;
        }
        if let Some(c) = self.ui.get_mut::<LogPanel>("syslog") {
            c.push(line, console_tone(line));
        }
    }

    fn set_settings(&mut self, s: Settings) {
        let resumed = self.settings.console_paused && !s.console_paused;
        let retab = self.settings.debug_mode != s.debug_mode;
        // Prefs > Appearance picked a theme: switch to it now. (A theme file
        // from --theme is neither built in nor in the themes folder, and the
        // screen keeps it.)
        if self.settings.theme != s.theme {
            let file = || {
                self.theme_files
                    .iter()
                    .find(|(f, _)| *f == s.theme)
                    .map(|(_, t)| t.clone())
            };
            if let Some(t) = Theme::builtin(&s.theme).or_else(file) {
                self.ui.set_theme(t);
            }
        }
        let timing = self.settings.timing_strip != s.timing_strip;
        self.settings = s;
        if timing {
            // Play's options turned the Beats view's timing strip on or
            // off: its page changes.
            let v = self.play.view;
            let _ = self.arp_panel.set_quick_layout(
                &mut self.ui,
                &v.quick_layout_for(self.settings.timing_strip),
            );
            self.refresh_play();
            if self.ready && self.view == View::Play {
                self.show_view(View::Play);
            }
        }
        // The sampler keeps the line input only while it is on.
        let line = self.settings.line_input;
        self.sample.set_line_input(&mut self.ui, line);
        self.print_sample_lines();
        if retab {
            let debug = self.settings.debug_mode;
            if let Some(h) = self.ui.get_mut::<Header>("header") {
                h.tabs = tabs(debug).iter().map(|v| v.name().to_string()).collect();
            }
            self.mark_tab();
        }
        if resumed {
            let lines: Vec<String> = self.console_backlog.drain(..).collect();
            if let Some(c) = self.ui.get_mut::<LogPanel>("syslog") {
                for l in &lines {
                    c.push(l, console_tone(l));
                }
                c.right = String::new();
            }
        }
        if !self.settings.midi_paused && self.midi_held_back > 0 {
            let n = self.midi_held_back;
            self.midi_held_back = 0;
            if let Some(m) = self.ui.get_mut::<LogPanel>("midi") {
                m.push(
                    &format!("({n} messages while paused, not shown)"),
                    Tone::Dim,
                );
            }
        }
    }

    /// The last run crashed: Prefs > At start opens nothing this time, and
    /// once the sound is going the screen says why (crash.rs).
    pub fn safe_start(&mut self) {
        self.crashed = true;
    }

    /// Shows `line` in place of the hint for [`TELL_SECONDS`].
    /// UNDO, or SHIFT + UNDO (`redo`), on a page whose history the screen
    /// picks (the Looper tab and the Sample and Pads tabs have theirs in
    /// the app). It follows what the page shows (Jay, 13 Nov 13:53Z): the
    /// loop on Play's Loop view, the takes on Beats, on Arrange and on the
    /// Arp and pattern screens, and on All whichever of the two changed
    /// last. Other pages say they have nothing to undo.
    fn undo(&mut self, redo: bool) {
        use mpfree_looper::Command;
        let to = match self.undo_source() {
            Some(HistoryOf::Loop) => {
                FromUi::LoopCommand(if redo { Command::Redo } else { Command::Undo })
            }
            Some(HistoryOf::Takes) => FromUi::Time(if redo {
                TimeAction::Redo
            } else {
                TimeAction::Undo
            }),
            Some(HistoryOf::Turns(on)) => FromUi::UndoOn { on, redo },
            Some(HistoryOf::Sample) => {
                let before = self.sample.page();
                self.sample.undo(&mut self.ui, redo);
                self.sample_after(before);
                return;
            }
            None => {
                self.tell(if redo {
                    "Nothing to redo on this page"
                } else {
                    "Nothing to undo on this page"
                });
                return;
            }
        };
        self.outbox.push(to);
        // The app's histories: the list, when open, asks again at once.
        if let Some(h) = self.history.as_mut() {
            h.ask_in = 0.0;
        }
    }

    /// Whose history UNDO follows on the page showing: the loop on Play's
    /// Loop view and the Looper tab, the takes on Beats, Arrange and the
    /// Arp and pattern screens, on All whichever of the two changed last,
    /// the kit's edits on the Sample and Pads tabs, and the turns on the
    /// Synth tab and the mixer. None elsewhere.
    fn undo_source(&self) -> Option<HistoryOf> {
        Some(match self.view {
            View::Play => match self.play.view {
                PlayView::Loop => HistoryOf::Loop,
                PlayView::Beats => HistoryOf::Takes,
                PlayView::All if self.undo_loop => HistoryOf::Loop,
                PlayView::All => HistoryOf::Takes,
            },
            View::Arrange | View::Arp | View::Pattern => HistoryOf::Takes,
            View::Looper => HistoryOf::Loop,
            View::Sample | View::Pads => HistoryOf::Sample,
            View::Synth => HistoryOf::Turns(UndoOn::Synth),
            View::Mixer => HistoryOf::Turns(UndoOn::Mixer),
            _ => return None,
        })
    }

    /// Opens the undo history of the page showing (UNDO held for
    /// [`UNDO_HOLD`]); on a page without one it says so.
    fn open_history(&mut self) {
        let Some(of) = self.undo_source() else {
            self.tell("Nothing to undo on this page");
            return;
        };
        if self.history.is_some() {
            return;
        }
        self.close_numpad();
        let back = self.ui.page().to_string();
        if let Some(p) = self.ui.get_mut::<mpfree_ui::history::HistoryPanel>(HISTORY) {
            p.clear();
        }
        if self.ui.show(HISTORY_PAGE).is_ok() {
            self.history = Some(HistoryOpen {
                of,
                back,
                ask_in: 0.0,
            });
            self.outbox.push(FromUi::Overlay(true));
            self.refresh_history();
            self.refresh_hint();
        }
    }

    /// Closes the undo history and shows the page it was opened over.
    fn close_history(&mut self) {
        let Some(h) = self.history.take() else {
            return;
        };
        if let Some(p) = self.ui.get_mut::<mpfree_ui::history::HistoryPanel>(HISTORY) {
            p.clear();
        }
        let _ = self.ui.show(&h.back);
        self.outbox.push(FromUi::Overlay(false));
        self.refresh_hint();
    }

    /// The open list follows its history: the screen's own (the loop, the
    /// takes, the kit's edits) as they stand, the app's (the turns) asked
    /// for every [`HISTORY_ASK`] and on each change made here.
    fn refresh_history(&mut self) {
        let Some(h) = self.history.as_mut() else {
            return;
        };
        let of = h.of;
        if let HistoryOf::Turns(on) = of {
            if h.ask_in <= 0.0 {
                h.ask_in = HISTORY_ASK;
                self.outbox.push(FromUi::UndoHistory(on));
            }
            return;
        }
        let Some((title, rows, now)) = self.history_rows(of) else {
            return;
        };
        self.show_history(&title, rows, now);
    }

    /// Puts `rows` in the list when they are not what it shows.
    fn show_history(&mut self, title: &str, rows: Vec<String>, now: usize) {
        let Some(p) = self.ui.get::<mpfree_ui::history::HistoryPanel>(HISTORY) else {
            return;
        };
        if p.title == title && p.rows == rows && p.now == now {
            return;
        }
        if let Some(p) = self.ui.get_mut::<mpfree_ui::history::HistoryPanel>(HISTORY) {
            p.set(title, rows, now);
        }
    }

    /// The screen's own histories as the list shows them: its title, its
    /// rows ("Start", the steps UNDO takes back, oldest first, then REDO's)
    /// and the row that is now.
    fn history_rows(&self, of: HistoryOf) -> Option<(String, Vec<String>, usize)> {
        let title = |what: &str| format!("Undo history · {what}");
        match of {
            HistoryOf::Loop => {
                let (name, h) = self.looper_page.history_rows();
                Some((title(&name), h.rows, h.now))
            }
            HistoryOf::Takes => {
                let st = &self.time;
                let name =
                    |r: &mpfree_time_ui::view::TakeRow| mpfree_time_ui::view::take_title(st, r);
                let mut rows = vec!["Start: before these takes".to_string()];
                rows.extend(st.done.iter().map(name));
                let now = rows.len() - 1;
                rows.extend(st.undone.iter().rev().map(name));
                Some((title("Takes"), rows, now))
            }
            HistoryOf::Sample => {
                let (kit, rows, now) = self.sample.history_rows()?;
                Some((title(&kit), rows, now))
            }
            HistoryOf::Turns(_) => None,
        }
    }

    /// A row of the undo history picked (a tap, or the wheel's push): back
    /// or forward to just after that step, as many steps as it takes.
    fn history_jump(&mut self, row: usize) {
        let Some(h) = self.history.as_ref() else {
            return;
        };
        let of = h.of;
        let Some(p) = self.ui.get::<mpfree_ui::history::HistoryPanel>(HISTORY) else {
            return;
        };
        if row >= p.rows.len() {
            return;
        }
        let steps = row as i32 - p.now as i32;
        if steps == 0 {
            return;
        }
        let redo = steps > 0;
        let n = steps.unsigned_abs() as usize;
        match of {
            HistoryOf::Loop => self.outbox.push(FromUi::LoopSteps(steps)),
            HistoryOf::Takes => {
                let a = if redo {
                    TimeAction::Redo
                } else {
                    TimeAction::Undo
                };
                self.outbox.extend((0..n).map(|_| FromUi::Time(a)));
            }
            HistoryOf::Sample => {
                let before = self.sample.page();
                for _ in 0..n {
                    self.sample.undo(&mut self.ui, redo);
                }
                self.sample_after(before);
                self.print_sample_lines();
            }
            HistoryOf::Turns(on) => {
                self.outbox
                    .extend((0..n).map(|_| FromUi::UndoOn { on, redo }));
                if let Some(h) = self.history.as_mut() {
                    h.ask_in = 0.0;
                }
            }
        }
        self.refresh_history();
    }

    /// REC (or OVERDUB, `overdub`) on the Play page (Jay, 13 Nov 14:37Z):
    /// the looper on the All and Loop views, as the loop strip's own
    /// Record and Overdub; the takes on Beats.
    fn play_rec(&mut self, overdub: bool) {
        use crate::looper_page::{B_OVERDUB, B_RECORD};
        let a = if self.view == View::Play && self.play.view != PlayView::Beats {
            FromUi::Looper(if overdub { B_OVERDUB } else { B_RECORD })
        } else {
            FromUi::Time(TimeAction::Transport(if overdub {
                mpfree_transport::Command::ToggleOverdub
            } else {
                mpfree_transport::Command::Record
            }))
        };
        self.outbox.push(a);
    }

    /// Whether UNDO on the page showing has something to take back, as
    /// [`Self::undo`] picks the history (the Looper tab and the Sample and
    /// Pads tabs too, whose UNDO the app sends straight on).
    fn undo_ready(&self) -> bool {
        match self.undo_source() {
            Some(HistoryOf::Loop) => !self.looper.history.done().is_empty(),
            Some(HistoryOf::Takes) => !self.time.done.is_empty(),
            Some(HistoryOf::Sample) => self.sample.can_undo(),
            Some(HistoryOf::Turns(UndoOn::Synth)) => self.turns_ready.0,
            Some(HistoryOf::Turns(UndoOn::Mixer)) => self.turns_ready.1,
            None => false,
        }
    }

    pub fn tell(&mut self, line: &str) {
        self.told = Some((tell_text(line), TELL_SECONDS));
        self.tells += 1;
        self.refresh_hint();
    }

    /// Opens the number pad for `ask` over the page showing.
    fn open_numpad(&mut self, ask: mpfree_ui::numpad::NumberAsk, to: TypeTo) {
        let back = self.ui.page().to_string();
        if let Some(p) = self.ui.get_mut::<mpfree_ui::numpad::NumPad>(NUMPAD) {
            p.open(ask);
        }
        if self.ui.show(NUMPAD_PAGE).is_ok() {
            self.typing = Some(Typing { to, back });
            self.outbox.push(FromUi::Overlay(true));
            self.refresh_hint();
        }
    }

    /// Closes the number pad (nothing typed goes anywhere) and shows the
    /// page it was opened over.
    fn close_numpad(&mut self) {
        let Some(t) = self.typing.take() else {
            return;
        };
        if let Some(p) = self.ui.get_mut::<mpfree_ui::numpad::NumPad>(NUMPAD) {
            p.close();
        }
        let _ = self.ui.show(&t.back);
        self.outbox.push(FromUi::Overlay(false));
        self.refresh_hint();
    }

    /// Enter on the number pad: the number goes to its field and the pad
    /// closes; out of range, the pad stays and says the range; with
    /// nothing typed, it closes.
    fn numpad_enter(&mut self) -> Option<FromUi> {
        let p = self.ui.get_mut::<mpfree_ui::numpad::NumPad>(NUMPAD)?;
        let value = p.entered();
        if value.is_none() && !p.warning.is_empty() {
            return None;
        }
        let key = p.ask.as_ref().map(|a| a.key.clone()).unwrap_or_default();
        let to = self.typing.as_ref().map(|t| t.to);
        self.close_numpad();
        let v = value?;
        match to? {
            TypeTo::Sample => {
                self.sample.typed(&mut self.ui, &key, v, &mut self.outbox);
                self.print_sample_lines();
                None
            }
            TypeTo::Tempo => {
                self.tell(&format!("Tempo: {v:.1} BPM"));
                Some(FromUi::Time(TimeAction::Transport(
                    mpfree_transport::Command::SetTempo(v),
                )))
            }
        }
    }

    /// The number pad for the tempo (a long press on Play's BPM).
    fn type_tempo(&mut self) {
        self.open_numpad(
            mpfree_ui::numpad::NumberAsk {
                key: "tempo".to_string(),
                title: "Tempo".to_string(),
                unit: "BPM".to_string(),
                value: (self.play.transport.bpm * 10.0).round() / 10.0,
                min: mpfree_transport::time::MIN_BPM,
                max: mpfree_transport::time::MAX_BPM,
                decimals: 1,
            },
            TypeTo::Tempo,
        );
    }

    /// How many times [`Player::tell`] has said something (the smoke tests
    /// count a told line as the press doing something).
    #[cfg(test)]
    pub fn tells(&self) -> u64 {
        self.tells
    }

    pub fn apply(&mut self, msg: ToUi) {
        // The number pad takes the wheel, - / + and the push while open;
        // a change of page closes it.
        if self.typing.is_some() {
            use crate::app::RowKey;
            let nudge = match &msg {
                ToUi::SampleWheel(d) | ToUi::Rows(RowKey::Pick(d) | RowKey::Step(d)) => Some(*d),
                _ => None,
            };
            if let Some(d) = nudge {
                if let Some(p) = self.ui.get_mut::<mpfree_ui::numpad::NumPad>(NUMPAD) {
                    p.nudge(d);
                }
                return;
            }
            if matches!(msg, ToUi::Rows(RowKey::Editing(_))) {
                if let Some(a) = self.numpad_enter() {
                    self.outbox.push(a);
                }
                return;
            }
            if matches!(msg, ToUi::Show(_) | ToUi::PlayView(_)) {
                self.close_numpad();
            }
        }
        // The undo history takes them too: the wheel and - / + move its
        // outline, the push goes to the step outlined.
        if self.history.is_some() {
            use crate::app::RowKey;
            match &msg {
                ToUi::SampleWheel(d) | ToUi::Rows(RowKey::Pick(d) | RowKey::Step(d)) => {
                    if let Some(p) = self.ui.get_mut::<mpfree_ui::history::HistoryPanel>(HISTORY) {
                        p.wheel(*d);
                    }
                    return;
                }
                ToUi::Rows(RowKey::Editing(_)) => {
                    let picked = self
                        .ui
                        .get::<mpfree_ui::history::HistoryPanel>(HISTORY)
                        .map(|p| p.picked);
                    if let Some(row) = picked {
                        self.history_jump(row);
                    }
                    return;
                }
                ToUi::Show(_) | ToUi::PlayView(_) => self.close_history(),
                _ => {}
            }
        }
        // The engines' news of an edit made with the controls (a clip
        // recorded, a knob turned): the work is unsaved.
        if matches!(
            msg,
            ToUi::Time(_)
                | ToUi::Steps(_)
                | ToUi::SynthValues(_)
                | ToUi::Chain(_)
                | ToUi::Macros(_)
                | ToUi::Edited
        ) {
            self.mark_unsaved();
        }
        match msg {
            ToUi::Tell(l) => self.tell(&l),
            ToUi::Undo(redo) => self.undo(redo),
            ToUi::UndoButton(down) => {
                self.undo_held = (down && self.typing.is_none()).then_some(0.0);
            }
            ToUi::UndoHistory { on, rows, now } => {
                if self
                    .history
                    .as_ref()
                    .is_some_and(|h| h.of == HistoryOf::Turns(on))
                {
                    let title = match on {
                        UndoOn::Synth => "Undo history · Synth sound",
                        UndoOn::Mixer => "Undo history · Mixer",
                    };
                    self.show_history(title, rows, now);
                }
            }
            ToUi::PlayRec(overdub) => self.play_rec(overdub),
            ToUi::TurnsReady { synth, mixer } => self.turns_ready = (synth, mixer),
            ToUi::Line(l) | ToUi::Event(l) => {
                let tone = tone_of(&l);
                if let Some(e) = self.ui.get_mut::<LogPanel>("events") {
                    e.push(&l, tone);
                }
                if self.console_from_lines {
                    self.push_console(&l);
                }
            }
            ToUi::Midi(l) => {
                self.midi += 1;
                let n = self.midi;
                if self.settings.midi_paused {
                    self.midi_held_back += 1;
                    if let Some(m) = self.ui.get_mut::<LogPanel>("midi") {
                        m.right = format!("{n} messages, paused");
                    }
                } else if let Some(m) = self.ui.get_mut::<LogPanel>("midi") {
                    m.push(
                        &l,
                        if l.contains("->") {
                            Tone::Accent
                        } else {
                            Tone::Normal
                        },
                    );
                    m.right = format!("{n} messages");
                }
            }
            ToUi::Console(l) => self.push_console(&l),
            ToUi::Progress {
                message,
                detail,
                value,
            } => {
                if let Some(w) = self.ui.get_mut::<Loading>("loading") {
                    w.message = message;
                    w.detail = detail;
                    w.progress = value.clamp(0.0, 1.0);
                    w.failed = false;
                }
            }
            ToUi::Failed { message, detail } => {
                if let Some(w) = self.ui.get_mut::<Loading>("loading") {
                    w.message = message;
                    w.detail = detail;
                    w.failed = true;
                }
            }
            ToUi::Kits(kits) => {
                self.sample
                    .set_names(kits.iter().map(|k| k.name.clone()).collect());
                self.set_kits(kits);
                self.sample_changed();
            }
            ToUi::PadDown { pad, velocity } => {
                if let Some(g) = self
                    .ui
                    .get_mut::<MiniPads>(play_page::PADS)
                    .map(|m| &mut m.grid)
                {
                    g.hit(pad, velocity);
                }
                // With Drums on the pads, the Synth tab shows the page of
                // the pad hit last, as the Sample page edits it.
                let synth_plays = self.kits.get(self.kit).is_some_and(|k| k.synth.is_some());
                if let Some(p) = self
                    .synth
                    .as_mut()
                    .filter(|p| synth_plays && p.state.info.id == "mpfree.drums")
                {
                    let page = mpfree_synth_ui::pages::drum_page(pad);
                    if p.page() != page {
                        p.show_page(&mut self.ui, page);
                    }
                }
                // The Sample page edits the pad hit last, and the Play
                // page's Tune and Decay turn it.
                let shown = self.view == View::Sample;
                self.sample.press(&mut self.ui, pad, velocity, shown);
                self.levels_hit(pad, true);
                // Load to pad or Send to pad may have put a sound on it.
                self.sample_names();
                self.sample_changed();
                if self.play.last_pad != pad {
                    self.play.last_pad = pad;
                    self.refresh_qlinks();
                }
                // A loop layer recording now takes its name from what is
                // played into it.
                let group = if synth_plays {
                    Some("Synth")
                } else {
                    self.play
                        .pads
                        .get(pad)
                        .and_then(|p| p.label.as_deref())
                        .and_then(mpfree_ui::draw::sound_group)
                };
                if let Some(g) = group {
                    self.looper_page.hit(g);
                }
            }
            ToUi::PadUp(pad) => {
                if let Some(g) = self
                    .ui
                    .get_mut::<MiniPads>(play_page::PADS)
                    .map(|m| &mut m.grid)
                {
                    g.release(pad);
                }
                self.sample.pad_up(&mut self.ui, pad);
                self.levels_hit(pad, false);
            }
            ToUi::PadFlash { pad, velocity } => {
                if let Some(g) = self
                    .ui
                    .get_mut::<MiniPads>(play_page::PADS)
                    .map(|m| &mut m.grid)
                {
                    let held = g.pads.get(pad).is_some_and(|p| p.held);
                    g.hit(pad, velocity);
                    if !held {
                        g.release(pad);
                    }
                }
            }
            ToUi::Kit(k) => {
                self.set_kit(k);
                self.sample.set_kit(k);
                self.sample_changed();
            }
            ToUi::Level { db, .. } => {
                let text = format!("{db:.1}");
                if self
                    .ui
                    .get::<MasterBar>(play_page::MASTER)
                    .is_some_and(|m| m.fader_db != db || m.text != text)
                {
                    if let Some(m) = self.ui.get_mut::<MasterBar>(play_page::MASTER) {
                        m.fader_db = db;
                        m.text = text;
                    }
                }
            }
            ToUi::Meter { peak_db, limit_db } => {
                let changed = self
                    .ui
                    .get::<MasterBar>(play_page::MASTER)
                    .is_some_and(|m| m.peak_db != peak_db || m.limit_db != limit_db);
                if changed {
                    if let Some(m) = self.ui.get_mut::<MasterBar>(play_page::MASTER) {
                        m.feed(peak_db);
                        m.limit_db = limit_db;
                    }
                }
            }
            ToUi::Steps(st) => {
                self.play.set_steps(*st);
                self.refresh_play();
                if self.view == View::Play {
                    self.refresh_qlinks();
                }
            }
            ToUi::Hit(h) => {
                self.play.hit(h);
                if self.timing_shown() {
                    self.refresh_play();
                }
            }
            ToUi::PlayLevels(l) => {
                if self.play.levels != l {
                    self.play.set_levels(l);
                    self.refresh_play();
                    if self.view == View::Play {
                        self.refresh_qlinks();
                    }
                }
            }
            ToUi::PlayQLink(k, ticks) => {
                if self.chain_page.drives(self.view) {
                    // Q-LINK shows a macro bank on Play or Arrange.
                    self.chain_qlink(k, ticks);
                } else if self.audio_qlink(k, ticks) {
                    // An audio region's knobs.
                } else if matches!(self.view, View::Play | View::Arrange) {
                    self.play_qlink(k, ticks);
                }
            }
            ToUi::QLinkTouch(k, on) => {
                if !on && self.chain_page.drives(self.view) {
                    // A knob of the effect lets go after a recorded move.
                    if let Some(Turned::Chain(a)) = self.chain_page.released(k) {
                        self.outbox.push(FromUi::Chain(a));
                    }
                }
                for column in [play_page::QLINKS, crate::looper_tab::QLINKS] {
                    if let Some(q) = self
                        .ui
                        .get_mut::<QLinkColumn>(column)
                        .filter(|q| q.touched.get(k) != Some(&on))
                    {
                        q.touched[k] = on;
                    }
                }
            }
            ToUi::PlayView(v) => {
                let v = v.unwrap_or_else(|| {
                    let at = PlayView::ALL.iter().position(|&x| x == self.play.view);
                    PlayView::from_index(at.map_or(0, |i| (i + 1) % PlayView::ALL.len()))
                });
                self.show_play_view(v);
            }
            ToUi::Library { dir, lists } => self.open_library(&dir, &lists),
            ToUi::Browse(k) => self.browse_key(k),
            ToUi::PadsView(v) => {
                self.sample.set_pads_view(&mut self.ui, v);
                if self.ready && self.view == View::Pads {
                    self.show_view(View::Pads);
                }
            }
            // Played out or replaced: freed here, not on the audio thread.
            ToUi::PreviewBack(p) => drop(p),
            ToUi::SampleEditor(e) => {
                self.sample.open_editor(&mut self.ui, e);
                if self.ready && self.view == View::Sample {
                    self.show_view(View::Sample);
                }
            }
            ToUi::LooperQLink(k, ticks) => {
                if self.view == View::Looper {
                    self.looper_qlink(k, ticks);
                }
            }
            ToUi::Session(text, tone) => {
                self.session = (text, tone);
                self.refresh_chips();
            }
            ToUi::Timing(t) => {
                self.timing = t;
                self.show_timing();
                self.refresh_chips();
                self.debug.set_audio(&mut self.ui, t.audio);
            }
            ToUi::Map(m) => DebugPages::set_map(&mut self.ui, &m),
            ToUi::Engines(shared) => self.engines = Some(shared),
            ToUi::Looper(v) => {
                // The picked loop's history moved: UNDO on All takes it.
                let h = &v.history;
                let seen = (v.selected, h.done().len(), h.redo().len());
                if seen.0 == self.seen_loop.0 && seen != self.seen_loop {
                    self.undo_loop = true;
                }
                self.seen_loop = seen;
                self.looper = *v;
                if matches!(self.view, View::Looper | View::Play) {
                    self.refresh_looper();
                }
            }
            ToUi::LoopWave(w) => {
                self.looper_page.set_wave(&mut self.ui, w);
                self.refresh_looper();
            }
            // Nothing until the panel is in the Ui (mpfree_mixer_ui::add).
            ToUi::Mixer(st) => {
                let turned = self.mix_view != Some(st.view);
                self.mix_view = Some(st.view);
                mpfree_mixer_ui::update(&mut self.ui, *st);
                if self.view == View::Mixer {
                    self.refresh_qlinks();
                    if turned {
                        // On the I/O view or off it: the icons and the hint.
                        self.mark_tab();
                        self.refresh_hint();
                    }
                }
            }
            ToUi::MixerQLink(k, d) => self.mixer_qlink(k, d),
            ToUi::IoInput(ev) => self.io_input(ev),
            ToUi::Chain(st) => {
                self.chain_page.set_chain(&mut self.ui, *st);
                if self.view == View::Chain {
                    self.refresh_qlinks();
                }
            }
            ToUi::ChainFx(fx) => {
                let theme = self.ui.theme.clone();
                self.chain_page.set_fx(&mut self.ui, fx.map(|b| *b), &theme);
            }
            ToUi::Macros(st) => {
                self.chain_page.set_macros(&mut self.ui, *st);
                if self.chain_page.drives(self.view) {
                    self.refresh_qlinks();
                }
            }
            ToUi::ChainQLink(k, d) => {
                if matches!(self.view, View::Chain | View::Macros) {
                    self.chain_qlink(k, d);
                }
            }
            ToUi::ChainWheel(d) => {
                if self.view == View::Chain
                    && !mpfree_chain_ui::chain_wheel(&mut self.ui, d, self.status.shift)
                {
                    self.chain_qlink(0, d);
                }
            }
            ToUi::ChainPush => {
                if self.view == View::Chain && mpfree_chain_ui::chain_push(&mut self.ui) {
                    let acts = mpfree_chain_ui::take_chain_actions(&mut self.ui);
                    self.outbox.extend(acts.into_iter().map(FromUi::Chain));
                }
            }
            ToUi::ChainLearn(addr) => {
                self.chain_page.learn(addr);
                self.refresh_qlinks();
            }
            ToUi::ChainLearnDone => {
                self.chain_page.cancel_learn();
                self.refresh_qlinks();
            }
            ToUi::QLinkBank => self.next_qlink_bank(),
            ToUi::ClockFrom(port) => {
                if self.clock_from != port {
                    self.clock_from = port;
                    self.refresh_chips();
                }
            }
            ToUi::IoModel(model) => {
                if self.io_sections.is_empty() {
                    self.io_sections = add_io_sections(&mut self.ui, &model);
                }
                self.io_model = Some(model);
            }
            ToUi::ProjectParts(c) => self.save_project(c),
            ToUi::ProjectDefaults(d) => self.project_defaults = Some(*d),
            ToUi::Plugins(p) => self.project_plugins = p,
            ToUi::TrackSounds(s) => {
                self.track_sounds = *s;
                fill_sounds(&mut self.time, &self.track_sounds);
                self.time_page.update(&mut self.ui, &self.time);
            }
            ToUi::Edited => {}
            // The engines an open replaced: freed here. The open is in:
            // nothing is unsaved (its own news in the next moments isn't
            // an edit).
            ToUi::ProjectBack(old) => {
                drop(old);
                self.opened_at = Some(Instant::now());
                if let Some(b) = self.browser.as_mut() {
                    b.open_landed();
                }
            }
            ToUi::Show(v) => {
                self.ready = true;
                self.show_view(v);
            }
            ToUi::Menu(m) => {
                if let Some(o) = self.ui.get_mut::<OptionList>("options") {
                    o.title = m.title;
                    o.rows = m
                        .rows
                        .into_iter()
                        .map(|(name, value)| OptionRow { name, value })
                        .collect();
                    o.selected = m.selected;
                    o.editing = m.editing;
                }
                if self.menu_editing != m.editing {
                    self.menu_editing = m.editing;
                    self.refresh_hint();
                }
                if let Some(c) = self.ui.get_mut::<Card>("help") {
                    c.heading = m.help_heading;
                    c.lines = m.help.into_iter().map(|l| (l, Tone::Dim)).collect();
                }
            }
            ToUi::Guide(g) => {
                if let Some(c) = self.ui.get_mut::<Card>("guide") {
                    c.heading = g.heading;
                    c.lines = g.lines.into_iter().map(|l| (l, Tone::Normal)).collect();
                    c.buttons = vec!["Skip".to_string(), "Done".to_string()];
                }
                if let Some(s) = self.ui.get_mut::<StatList>("found") {
                    s.set(
                        g.found
                            .into_iter()
                            .map(|(k, v)| {
                                let tone = if v == "now" {
                                    Tone::Accent
                                } else if v.starts_with("button") {
                                    Tone::Good
                                } else {
                                    Tone::Dim
                                };
                                (k, v, tone)
                            })
                            .collect(),
                    );
                }
            }
            ToUi::Status(s) => {
                self.status = s;
                self.refresh_pads_bar();
                if self.ready && self.view == View::Play {
                    // The quick band comes and goes with NOTE REPEAT.
                    self.show_view(View::Play);
                }
                self.refresh_chips();
                self.refresh_hint();
                self.refresh_qlinks();
            }
            ToUi::Settings(s) => self.set_settings(s),
            ToUi::ThemeFiles(t) => self.theme_files = t,
            ToUi::ClearLogs => {
                for name in ["midi", "events"] {
                    let title = self
                        .ui
                        .get::<LogPanel>(name)
                        .map(|l| l.title.clone())
                        .unwrap_or_default();
                    self.ui.add(
                        name,
                        LogPanel::new(
                            &title,
                            name == "midi",
                            if name == "midi" { 300 } else { 200 },
                        ),
                    );
                }
                self.midi_held_back = 0;
            }
            ToUi::ClearConsole => {
                self.ui
                    .add("syslog", LogPanel::new("Console", true, CONSOLE_BACKLOG));
                self.console_backlog.clear();
            }
            ToUi::Synth(state) => {
                match self.synth.as_mut() {
                    Some(p) => {
                        let _ = p.load(&mut self.ui, state);
                    }
                    None => match SynthPanel::add(&mut self.ui, "Synth", state) {
                        Ok(mut p) => {
                            let _ = p.set_layout_wrap(&mut self.ui, play_page::synth_with_pads);
                            self.synth = Some(p);
                        }
                        Err(e) => println!("screen: no Synth page ({e})"),
                    },
                }
                self.refresh_qlinks();
            }
            ToUi::SynthValues(values) => {
                if let Some(p) = self
                    .synth
                    .as_mut()
                    .filter(|p| p.state.values.len() == values.len())
                {
                    p.state.values = values;
                    p.refresh(&mut self.ui);
                }
                self.refresh_qlinks();
            }
            ToUi::SynthMeters(meters) => {
                if let Some(p) = self.synth.as_mut() {
                    let mut changed = false;
                    for (i, v) in meters {
                        if let Some(old) = p.state.values.get_mut(i).filter(|old| **old != v) {
                            *old = v;
                            changed = true;
                        }
                    }
                    if changed && self.view == View::Synth {
                        p.refresh(&mut self.ui);
                    }
                }
            }
            ToUi::SynthPressures(values) => {
                if let Some(p) = self.synth.as_mut() {
                    p.set_pressures(&mut self.ui, &values);
                }
            }
            ToUi::SynthQLink(n, ticks) => {
                if let Some(a) = self
                    .synth
                    .as_mut()
                    .and_then(|p| p.qlink(&mut self.ui, n, ticks))
                {
                    self.outbox.push(FromUi::Synth(a));
                }
                self.refresh_qlinks();
            }
            ToUi::SynthWheel(ticks) => {
                if let Some(a) = self
                    .synth
                    .as_mut()
                    .and_then(|p| p.wheel(&mut self.ui, ticks))
                {
                    self.outbox.push(FromUi::Synth(a));
                }
                self.refresh_qlinks();
            }
            ToUi::SynthPage(delta) => {
                if let Some(p) = self.synth.as_mut() {
                    p.step_page(&mut self.ui, delta);
                }
                self.refresh_qlinks();
            }
            ToUi::SampleKits { projects, rate } => {
                self.project_rate = rate;
                self.sample.set_kits(&mut self.ui, projects, rate);
                self.sample_changed();
            }
            ToUi::SampleViewBack(swap) => self.sample.view_back(swap),
            ToUi::SampleQLink(k, ticks) => {
                if self.browsing() {
                    self.browser_qlink(k, ticks);
                } else if self.view.sampler() {
                    self.sample.qlink(&mut self.ui, k, ticks, self.status.shift);
                }
            }
            ToUi::SampleLift => self.sample.lift(),
            ToUi::SampleWheel(d) => self.sample.wheel(&mut self.ui, d),
            ToUi::Rows(k) => self.rows(k),
            ToUi::SamplePad(d) => {
                self.sample.step_pad(d);
                self.sample_changed();
            }
            ToUi::SampleUndo(redo) => {
                let before = self.sample.page();
                self.sample.undo(&mut self.ui, redo);
                self.sample_after(before);
                self.refresh_levels();
            }
            ToUi::SamplePlayheads(h) => {
                if self.view == View::Sample {
                    self.sample.set_playheads(&mut self.ui, &h);
                }
            }
            ToUi::SampleGrains(c) => self.sample.cloud_back(&mut self.ui, c),
            ToUi::SampleInputBack(f) => {
                self.sample.input_back(f);
                self.print_sample_lines();
            }
            ToUi::KitChanged { kit, view } => {
                if let Some(item) = self
                    .ui
                    .get_mut::<ListView>("kits")
                    .and_then(|l| l.items.get_mut(kit))
                {
                    item.text = view.name.clone();
                }
                if let Some(k) = self.kits.get_mut(kit) {
                    *k = view;
                    if kit == self.kit {
                        self.set_kit(kit);
                    }
                }
                self.refresh_pads_bar();
            }
            ToUi::TouchLab => self.open_touch_page("touchlab", "Touch lab"),
            ToUi::CalibrateTouch => self.open_touch_page("calibrate", "Calibrate touch"),
            ToUi::TouchTuning(t) => {
                self.saved_tuning = t;
                self.set_tuning(t);
                self.say(format!("touch-tune: loaded {}", t.line()));
            }
            ToUi::TouchWheel(d) => {
                if let Some(l) = self.ui.get_mut::<TuneList>(touch_dev::TUNE) {
                    let last = (l.len() as i32 - 1).max(0);
                    l.selected = (l.selected as i32 + d).clamp(0, last) as usize;
                }
            }
            ToUi::TouchStep(d) => {
                let sel = self
                    .ui
                    .get::<TuneList>(touch_dev::TUNE)
                    .map_or(0, |l| l.selected);
                match Param::ALL.get(sel) {
                    Some(&p) => self.tune_step(p, d),
                    // + runs the action picked; - does nothing there.
                    None if d > 0 => self.tune_action(sel - Param::ALL.len()),
                    None => {}
                }
            }
            ToUi::Arp(v) => {
                let v = *v;
                let a = &mut self.arp;
                a.settings = v.settings;
                a.pattern = v.pattern;
                a.pattern_index = v.pattern_index;
                a.pattern_changed = v.pattern_changed;
                a.notes = v.notes;
                a.step = v.step;
                a.latched = v.latched;
                a.bpm = v.bpm;
                self.refresh_arp();
            }
            ToUi::SixteenLevels => self.toggle_levels(),
            ToUi::LevelsWheel(d) => {
                let by = LevelsPanel::wheel(self.levels.by, d);
                self.levels_by(by);
            }
            ToUi::Patterns(list) => {
                self.arp.patterns = list;
                self.refresh_arp();
            }
            ToUi::Workspace(w) => {
                let w = *w;
                if w.open {
                    self.workspace.editor = Editor::new(w.pattern);
                    self.workspace.name = w.name;
                }
                self.workspace.recording = w.recording;
                self.workspace.free_time = w.free_time;
                if self.view == View::Pattern {
                    let page = self.workspace.page().to_string();
                    self.switch_page(&page);
                }
                self.label_pads();
                self.refresh_arp();
            }
            ToUi::Recorded(p, next) => {
                self.workspace.recorded(*p);
                self.workspace.editor.select(Row::Step, next);
                self.refresh_arp();
            }
            ToUi::WorkspaceWheel(d) => {
                if let Some(a) = self.workspace.wheel(d) {
                    self.outbox.push(FromUi::Workspace(Box::new(a)));
                }
                self.refresh_arp();
            }
            ToUi::Time(mut st) => {
                // The lens the note editor shows, if it is open.
                let lens = |t: &TimeState| t.edit.as_ref().map(|e| e.editor.lens);
                let swapped = st.song_view != self.time.song_view || lens(&st) != lens(&self.time);
                // A take made, undone or redone: UNDO on All takes the takes.
                let seen = (st.done.len(), st.undone.len());
                if seen != self.seen_takes {
                    self.undo_loop = false;
                    self.seen_takes = seen;
                }
                fill_sounds(&mut st, &self.track_sounds);
                self.time = *st;
                self.sample.set_tempo(self.time.transport.bpm);
                self.time_page.update(&mut self.ui, &self.time);
                self.play.set_transport(self.time.transport);
                self.refresh_play();
                if self.view == View::Arrange {
                    // A region picked or let go, or its values changed.
                    self.refresh_qlinks();
                }
                if swapped {
                    // The Song view and the note editor have their own
                    // hint lines.
                    self.refresh_hint();
                }
            }
            ToUi::AudioLevels(levels) => {
                let mut moved = false;
                for lane in &mut self.time.audio {
                    let l = levels.get(lane.track).copied().unwrap_or(0.0);
                    moved |= lane.level != l;
                    lane.level = l;
                }
                if moved && self.view == View::Arrange {
                    self.time_page.update(&mut self.ui, &self.time);
                }
            }
            ToUi::Transport(info) => {
                self.time.transport = info;
                self.sample.set_tempo(info.bpm);
                self.time_page.update(&mut self.ui, &self.time);
                let swing = self.play.transport.swing;
                self.play.set_transport(info);
                self.refresh_play();
                if self.view == View::Play && self.play.transport.swing != swing {
                    self.refresh_qlinks();
                }
            }
            ToUi::ClipsWheel {
                delta,
                scroll: true,
            } => {
                if let Some(a) = self.time_page.scroll(&self.ui, delta) {
                    self.outbox.push(FromUi::Time(a));
                }
                self.time_page.update(&mut self.ui, &self.time);
            }
            ToUi::ClipsWheel {
                delta,
                scroll: false,
            } => {
                // The row last touched (the armed track to begin with), of
                // the record settings or, in the note editor, its options.
                let list = self.time_page.wheel_list(&self.ui);
                let index = self.ui.get::<OptionList>(list).map_or(0, |l| l.selected);
                let ev = UiEvent {
                    widget: list.to_string(),
                    kind: EventKind::Step { index, delta },
                };
                if let Some(a) = time_view::action(&ev, &self.time) {
                    self.outbox.push(FromUi::Time(a));
                } else if let Some(why) = time_view::why_not(&ev, &self.time) {
                    self.tell(&why);
                }
            }
            ToUi::TouchBell | ToUi::Quit => {}
        }
        self.print_sample_lines();
    }

    /// The wheel, - / + or the wheel's push on a page with rows
    /// (`app::RowKey`): pick a row, step it, or open it for the wheel.
    fn rows(&mut self, k: crate::app::RowKey) {
        use crate::app::RowKey;
        if let RowKey::Editing(on) = k {
            // Every page with rows keeps it, so the page that shows next
            // agrees with the app (which closes it on a page change).
            self.sample.set_editing(&mut self.ui, on);
            if let Some(p) = self.synth.as_mut() {
                p.set_editing(&mut self.ui, on);
            }
            for list in [time_view::RECORD, time_view::EDIT_OPTIONS] {
                if let Some(l) = self.ui.get_mut::<OptionList>(list) {
                    l.editing = on;
                }
            }
            return;
        }
        match (self.view, k) {
            (View::Synth, RowKey::Pick(d)) => {
                if let Some(p) = self.synth.as_mut() {
                    p.pick_tile(&mut self.ui, d);
                }
            }
            (View::Synth, RowKey::Step(d)) => self.apply(ToUi::SynthWheel(d)),
            (View::Arrange, RowKey::Pick(d)) => {
                let list = self.time_page.wheel_list(&self.ui);
                let Some(l) = self.ui.get_mut::<OptionList>(list) else {
                    return;
                };
                let last = l.rows.len().saturating_sub(1) as i64;
                let k = (l.selected as i64 + i64::from(d)).clamp(0, last) as usize;
                l.selected = k;
                // As a tap on the row: the note editor keeps its row.
                let ev = UiEvent {
                    widget: list.to_string(),
                    kind: EventKind::Select(k),
                };
                if let Some(a) = time_view::action(&ev, &self.time) {
                    self.outbox.push(FromUi::Time(a));
                }
            }
            (View::Arrange, RowKey::Step(d)) => self.apply(ToUi::ClipsWheel {
                delta: d,
                scroll: false,
            }),
            (v, RowKey::Pick(d)) if v.sampler() => {
                if let Some(ev) = self.sample.pick_row(&mut self.ui, d) {
                    self.on_event(ev);
                }
            }
            (v, RowKey::Step(d)) if v.sampler() && !self.sample.has_list() => {
                // A pad with no sound has no rows: - / + go to the next pad
                // with one, as they used to.
                self.apply(ToUi::SamplePad(d.signum()));
            }
            (v, RowKey::Step(d)) if v.sampler() => {
                // A page whose wheel stepped the row picked does as it did
                // (one undo step a turn); the others step it as its arrow.
                match self.sample.step_row(&mut self.ui, d) {
                    Some(ev) => {
                        self.on_event(ev);
                    }
                    None => self.sample.wheel(&mut self.ui, d),
                }
                self.sample.show_editing(&mut self.ui);
            }
            _ => {}
        }
    }

    /// The Sample page's kit, pad or project changed: show it again if it
    /// is on screen (it may be the page for a pad without a sound now).
    fn sample_changed(&mut self) {
        if self.ready && self.view.sampler() && self.touch_page.is_none() {
            self.show_view(self.view);
        }
        // The Play tab's Pads play row, an undo or another kit may have
        // turned 16 Levels on or off.
        self.refresh_levels();
    }

    /// The Synth tab's patch by engine and preset ("Poly Pluck"), for the
    /// Sample page's Auto-sample mode; None before the synth is up.
    fn auto_synth_name(&self) -> Option<String> {
        let st = &self.synth.as_ref()?.state;
        let preset = st.preset.and_then(|i| st.presets.get(i));
        Some(match preset {
            Some(p) => format!("{} {p}", st.info.name),
            None => st.info.name.to_string(),
        })
    }

    /// The Play page's pad names, for kits whose pads the Sample page
    /// changed.
    fn sample_names(&mut self) {
        while let Some((kit, pads)) = self.sample.take_renamed() {
            if let Some(k) = self.kits.get_mut(kit) {
                k.pads = pads;
                if kit == self.kit {
                    self.set_kit(kit);
                }
            }
        }
    }

    /// After an edit on the Sample page: show the page again if it changed
    /// (Wave to Chop, or a pad without a sound), and give the Play page the
    /// names of pads a chop filled.
    fn sample_after(&mut self, before: &str) {
        self.sample_names();
        if self.sample.page() != before {
            self.sample_changed();
        }
        // The keyboard came or went on the Play tab, a row that pad hits
        // edit was picked, or the Chop tab went live: its hint with it.
        let keys = (
            self.sample.play_keys_shown(),
            self.sample.play_takes_pads(&self.ui),
        );
        let live = self.sample.chop_live_method();
        if keys != self.sample_keys || live != self.sample_live {
            self.sample_keys = keys;
            self.sample_live = live;
            if self.view == View::Sample {
                self.refresh_hint();
            }
        }
    }

    /// The Sample page's log lines, to the terminal (the system log) and
    /// the Events log.
    fn print_sample_lines(&mut self) {
        if let Some(t) = self.sample.take_told().pop() {
            self.tell(&t);
        }
        let lines = self.sample.take_lines();
        if lines.is_empty() {
            return;
        }
        for l in lines {
            println!("{l}");
            if let Some(e) = self.ui.get_mut::<LogPanel>("events") {
                e.push(&l, Tone::Dim);
            }
        }
        let _ = io::stdout().flush();
    }

    /// An action that found no room on its way to the audio thread.
    pub fn unsent(&mut self, a: FromUi) {
        match a {
            FromUi::SampleView(swap) => self.sample.unsent(swap.kit),
            FromUi::SampleGrains(c) => self.sample.cloud_unsent(c),
            FromUi::SampleInput(f) => self.sample.input_unsent(f),
            FromUi::SampleSource(src) => self.sample.source_unsent(src),
            FromUi::SamplePhrase(clip) => {
                println!("sample: phrase {:?} found no room on its way to the clips; the pads have the slices", clip.name);
                let _ = io::stdout().flush();
            }
            _ => {}
        }
    }

    // ------------------------------------------------------------ browser

    /// The library in the data folder: the Pads tab's browser over it
    /// (sounds only; it stays open beside the pads).
    fn open_library(&mut self, dir: &std::path::Path, lists: &str) {
        match mpfree_library::Library::at(dir) {
            Ok(lib) => {
                let root = lib.root().display().to_string();
                self.set_browser(mpfree_browser_ui::Browser::new(lib), lists);
                self.say(format!("browser: the library is {root}"));
            }
            Err(e) => self.say(format!(
                "browser: no library in {} ({e}); the Pads tab's Browse view says so",
                dir.display()
            )),
        }
    }

    /// The Pads tab's browser is `b`, with its Recent and Favourites from
    /// `lists`.
    pub(crate) fn set_browser(&mut self, mut b: mpfree_browser_ui::Browser, lists: &str) {
        b.load_lists(lists);
        b.open_picker(mpfree_browser_ui::Pick::Pads);
        self.browser = Some(b);
        // The start's own news (the empty song, the first states) is not
        // an edit.
        self.opened_at = Some(Instant::now());
        self.sample
            .set_browse_with(&mut self.ui, mpfree_browser_ui::BROWSER);
        self.step_browser();
    }

    /// The Pads tab shows its browser.
    fn browsing(&self) -> bool {
        (self.view == View::Pads && self.sample.page() == sample_page::PADS_BROWSE_PAGE)
            || self.view == View::Browser
    }

    /// A control on the Pads tab's browser (the app sends them only while
    /// it shows).
    fn browse_key(&mut self, k: crate::app::BrowseKey) {
        use crate::app::BrowseKey;
        let Some(b) = self.browser.as_mut() else {
            return;
        };
        match k {
            BrowseKey::Wheel(d) => b.dial(d),
            BrowseKey::Places(d) => b.dial_places(d),
            BrowseKey::Push => b.push(),
            BrowseKey::Back => b.back(),
            BrowseKey::Enter => b.enter(),
            BrowseKey::Play => b.play(),
            BrowseKey::Erase => b.erase(),
        }
        mpfree_browser_ui::show(&mut self.ui, b);
        self.step_browser();
    }

    /// Touches on the browser, the worker's answers and what they ask of
    /// the app (once a frame while it shows, and after a control).
    pub(crate) fn step_browser(&mut self) {
        let Some(b) = self.browser.as_mut() else {
            return;
        };
        for a in mpfree_browser_ui::step(&mut self.ui, b) {
            self.browse_action(a);
        }
    }

    fn browse_action(&mut self, a: mpfree_browser_ui::BrowseAction) {
        use mpfree_browser_ui::BrowseAction as A;
        match a {
            // Load: onto the pad picked. Load to pad: onto the next pad hit.
            A::LoadSample { path, .. } => self.sample.load_file(path, false),
            A::LoadToPad { path, .. } => self.sample.load_file(path, true),
            A::Audition { path } => match sample_page::read_in_thread(path) {
                Ok(rx) => self.preview_load = Some(rx),
                Err(e) => self.say(format!("preview: not heard ({e})")),
            },
            A::StopAudition => {
                self.preview_load = None;
                self.outbox.push(FromUi::PreviewStop);
            }
            A::SaveLists(text) => self.outbox.push(FromUi::SaveBrowserLists(text)),
            A::PickerClosed { .. } => {}
            // Projects (project.rs).
            A::SaveRequested => self.ask_save(),
            A::NewProject => self.new_project(),
            A::ProjectOpened { loc, snap } => self.open_project(&loc, &snap),
            A::ProjectSaved { loc } => self.say(format!("project: saved {}", loc.name())),
            // Every 2 minutes while editing: the terminal and the system
            // log only, not the Events log.
            A::Autosaved { loc } => {
                println!("project: autosaved into {}", loc.name());
                let _ = io::stdout().flush();
            }
            A::LastNotOpened(e) => self.say(format!(
                "project: the last project wasn't opened ({e}); the new one stays"
            )),
            // The Pads tab's browser offers sounds only; anything else
            // is said, never dropped quietly.
            other => self.say(format!(
                "browser: {} is not wired on the Pads tab",
                match other {
                    A::LoadKit { .. } | A::LoadFactoryKit { .. } => "a kit",
                    A::LoadPreset { .. } | A::TryPreset { .. } | A::RevertPreset => "a preset",
                    _ => "a project",
                }
            )),
        }
    }

    /// The preview's file, once read: to the audio thread.
    fn poll_preview(&mut self) {
        let Some(rx) = &self.preview_load else {
            return;
        };
        let r = match rx.try_recv() {
            Ok(r) => r,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Err("its thread stopped".into()),
        };
        self.preview_load = None;
        match r {
            Ok((src, _)) => {
                let p = crate::preview::Preview::new(&src, self.preview_db);
                self.outbox.push(FromUi::Preview(Box::new(p)));
            }
            Err(e) => self.say(format!("preview: not heard ({e})")),
        }
    }

    // ------------------------------------------------------------ projects

    /// SHIFT + BROWSE or Save: the audio loop is asked for its parts. A
    /// save the musician asks for wins over an autosave asked for at the
    /// same moment (the parts that come serve the save).
    fn ask_save(&mut self) {
        self.autosaving = false;
        self.autosave_dirty = false;
        self.autosave_in = AUTOSAVE_S;
        self.outbox.push(FromUi::SaveProject);
    }

    /// Autosave (Jay's pick, 14 Nov): once there has been unsaved work,
    /// edited since the last autosave, for [`AUTOSAVE_S`], the audio loop
    /// is asked for its parts as for a save,
    /// and [`Player::save_project`] hands them to autosave's own project
    /// (`Browser::autosave_with`). It waits while a project opens; parts
    /// asked for that never come (an open stopped the save) are given up
    /// after the same time, and asked for again.
    fn autosave_tick(&mut self, dt: f32) {
        let unsaved = self.browser.as_ref().is_some_and(|b| b.unsaved());
        if self.autosaving {
            self.autosave_in -= dt.max(0.0);
            if self.autosave_in <= 0.0 {
                self.autosaving = false;
                self.autosave_dirty = true;
                self.autosave_in = AUTOSAVE_S;
                println!("project: autosave's parts never came; asking again later");
            }
            return;
        }
        let opening = !self.start_checked
            || self.loops_load.is_some()
            || self.audio_load.is_some()
            || self.project_defaults.is_none()
            || self.opened_at.is_some_and(|t| t.elapsed() < OPEN_SETTLE);
        if !unsaved || !self.autosave_dirty || opening || self.autosave_off {
            self.autosave_in = AUTOSAVE_S;
            return;
        }
        self.autosave_in -= dt.max(0.0);
        if self.autosave_in <= 0.0 {
            self.autosaving = true;
            self.autosave_dirty = false;
            self.autosave_in = AUTOSAVE_S;
            self.outbox.push(FromUi::SaveProject);
        }
    }

    /// The audio loop's parts of a save came: the screen's own (the kits
    /// with their sounds, the I/O setup) join them, and the browser's
    /// worker saves the lot, whole or not at all (mpfree-library).
    fn save_project(&mut self, mut c: Box<crate::project::Collected>) {
        use mpfree_library::Part as _;
        // Autosave's parts go to autosave's project, quietly; a reason not
        // to save is said only for a save the musician asked for.
        let auto = std::mem::take(&mut self.autosaving);
        if auto {
            self.autosave_in = AUTOSAVE_S;
        }
        if self.browser.is_none() {
            if !auto {
                self.say(
                    "project: not saved, there is no library (the app was started without --data)"
                        .to_string(),
                );
            }
            return;
        }
        if self.loops_load.is_some() {
            if !auto {
                self.say(
                    "project: not saved, the loops are still opening (try again in a moment)"
                        .into(),
                );
            }
            return;
        }
        if self.audio_load.is_some() {
            if !auto {
                self.say(
                    "project: not saved, the audio tracks are still opening (try again in a moment)"
                        .into(),
                );
            }
            return;
        }
        let mut extra = Vec::new();
        let (text, sounds) = self.sample.save_part();
        extra.push((crate::project::SAMPLER.to_string(), text));
        if let Some(m) = &self.io_model {
            extra.push((
                crate::project::MIDI.to_string(),
                mpfree_midi_ui::MidiPart(m.clone()).save(),
            ));
        }
        let mut samples: Vec<_> = sounds
            .into_iter()
            .map(|(file, src)| {
                mpfree_library::NewSample::new(
                    file,
                    std::sync::Arc::new(move |w: &mut dyn std::io::Write| {
                        mpfree_sampler::wav::write(&src, w)
                    }),
                )
            })
            .collect();
        // The loops: their part, and a WAV a layer.
        if let Some(loops) = c.loops.take() {
            let rate = loops.rate;
            let (text, files) = crate::project::loops_part(loops);
            extra.push((crate::project::LOOPER.to_string(), text));
            samples.extend(files.into_iter().map(|(file, audio)| {
                mpfree_library::NewSample::new(
                    file,
                    std::sync::Arc::new(move |w: &mut dyn std::io::Write| {
                        w.write_all(&mpfree_looper::save::wav(&audio, rate))
                    }),
                )
            }));
        }
        // The audio tracks: their part, and a WAV a take, written a piece
        // at a time.
        if let Some(audio) = c.audio.take() {
            let rate = audio.rate;
            let (text, files) = crate::project::audio_part(audio);
            extra.push((crate::project::AUDIOTRACKS.to_string(), text));
            samples.extend(files.into_iter().map(|(file, audio, channels)| {
                mpfree_library::NewSample::new(
                    file,
                    std::sync::Arc::new(move |w: &mut dyn std::io::Write| {
                        mpfree_audiotracks::save::write_wav(w, &audio, channels, rate)
                    }),
                )
            }));
        }
        let snap = crate::project::snapshot(&c, extra);
        let Some(b) = self.browser.as_mut() else {
            return;
        };
        if auto {
            b.autosave_with(snap, samples, now_utc());
            return;
        }
        b.save_with(snap, samples, now_utc());
        mpfree_browser_ui::show(&mut self.ui, b);
        let line = match b.project() {
            Some(l) => format!("project: saving {}", l.name()),
            None => "project: saving".to_string(),
        };
        self.say(line);
    }

    /// An opened project's parts: the screen's own go to the Sample tab and
    /// the I/O setup here; the audio loop's are read and built here and
    /// sent in one piece (project.rs).
    fn open_project(&mut self, loc: &mpfree_library::Loc, snap: &mpfree_library::Snapshot) {
        if let Some(text) = snap.part(crate::project::SAMPLER) {
            if let Some(lib) = self.browser.as_ref().map(|b| b.library().clone()) {
                let loc = loc.clone();
                self.sample.open_part(
                    text.to_string(),
                    Box::new(move |f| {
                        lib.sample_path(&loc, f)
                            .and_then(std::fs::read)
                            .map_err(|e| e.to_string())
                    }),
                );
            }
        }
        self.open_midi(snap.part(crate::project::MIDI));
        let o = crate::project::opening(snap, self.project_rate, loc.name(), &self.project_plugins);
        self.outbox.push(FromUi::OpenProject(Box::new(o)));
        // The loops, on a thread: the layers' files can be big. A read
        // still going for an earlier open is dropped with its channel.
        self.loops_load = None;
        if let (Some(text), Some(lib)) = (
            snap.part(crate::project::LOOPER),
            self.browser.as_ref().map(|b| b.library().clone()),
        ) {
            let (text, loc) = (text.to_string(), loc.clone());
            let (tx, rx) = std::sync::mpsc::channel();
            let read = std::thread::Builder::new()
                .name("loops-open".into())
                .spawn(move || {
                    let r = crate::project::loops_open(&text, |f| {
                        lib.sample_path(&loc, f)
                            .and_then(std::fs::read)
                            .map_err(|e| e.to_string())
                    });
                    let _ = tx.send(r);
                });
            match read {
                Ok(_) => self.loops_load = Some(rx),
                Err(e) => self.say(format!("project: the loops weren't opened ({e})")),
            }
        }
        // The audio tracks, on a thread too: a take's file can be minutes
        // long.
        self.audio_load = None;
        if let (Some(text), Some(lib)) = (
            snap.part(crate::project::AUDIOTRACKS),
            self.browser.as_ref().map(|b| b.library().clone()),
        ) {
            let (text, loc, rate) = (text.to_string(), loc.clone(), self.project_rate);
            let (tx, rx) = std::sync::mpsc::channel();
            let read = std::thread::Builder::new()
                .name("audio-open".into())
                .spawn(move || {
                    let r = crate::project::audio_open(&text, rate, |f| {
                        lib.sample_path(&loc, f)
                            .and_then(std::fs::read)
                            .map_err(|e| e.to_string())
                    });
                    let _ = tx.send(r);
                });
            match read {
                Ok(_) => self.audio_load = Some(rx),
                Err(e) => self.say(format!("project: the audio tracks weren't opened ({e})")),
            }
        }
        // The kits and engines coming in are the open's own news.
        self.opened_at = Some(Instant::now());
        self.open_actions = self.outbox.len();
        self.say(format!("project: opening {}", loc.name()));
    }

    /// The I/O setup's song settings (tracks' outputs, Tempo from, clock
    /// out, CV jobs, learned controls): the project's `midi` part, or the
    /// defaults for New or a project without one, never the last
    /// project's. The rig's (each port's input and thru) stay as they are.
    /// The audio loop follows the model's change.
    fn open_midi(&mut self, text: Option<&str>) {
        use mpfree_library::Part as _;
        let Some(m) = &self.io_model else {
            return;
        };
        let fresh;
        let text = match text {
            Some(t) => t,
            None => {
                fresh = mpfree_midi_ui::MidiSetup::default().project_text();
                &fresh
            }
        };
        if let Err(e) = mpfree_midi_ui::MidiPart(m.clone()).load(text) {
            self.say(format!("project: the MIDI setup wasn't opened ({e})"));
        }
    }

    /// New project: the parts the app started with, the factory kits.
    fn new_project(&mut self) {
        let Some(d) = self.project_defaults.as_ref() else {
            self.say(
                "project: not ready for a new project yet (the sound is still starting)"
                    .to_string(),
            );
            return;
        };
        let snap = crate::project::new_snapshot(d);
        let mut o = crate::project::opening(
            &snap,
            self.project_rate,
            "a new project",
            &self.project_plugins,
        );
        (o.clear_loops, o.clear_audio) = (true, true);
        self.open_midi(None);
        self.outbox.push(FromUi::OpenProject(Box::new(o)));
        (self.loops_load, self.audio_load) = (None, None);
        self.opened_at = Some(Instant::now());
        if !self.sample.reset_kits(&mut self.ui) {
            self.say("project: the kits haven't come yet; they stay as they are".to_string());
        }
        self.sample_changed();
        // The factory kits' sound goes out with New's own actions.
        if let Some(a) = self.sample.take_view() {
            self.outbox.push(a);
        }
        self.open_actions = self.outbox.len();
        self.say("project: new project".to_string());
    }

    /// Prefs > At start, once the sound is going (the start's parts have
    /// come): the last project saved or opened opens, or the new, empty
    /// one the app started with stays.
    fn open_at_start(&mut self) {
        if self.start_checked || self.project_defaults.is_none() {
            return;
        }
        self.start_checked = true;
        // The last run's unsaved work, autosaved after its last save (it
        // ended with a crash or the power switch), is set aside first, so
        // that this run's autosave can't write over it.
        let kept = match self.browser.as_mut().and_then(|b| b.keep_newer_autosave()) {
            Some(Ok(l)) => Some(l.name().to_string()),
            Some(Err(e)) => {
                self.autosave_off = true;
                self.say(format!(
                    "project: autosave is off this run, so as not to write over the last \
                     run's unsaved work, which couldn't be set aside ({e})"
                ));
                None
            }
            None => None,
        };
        if self.crashed {
            // A safe start: the new project stays, and the screen says why.
            let last = match (&self.browser, self.settings.open_last) {
                (Some(b), true) => b.last_project().map(|l| l.name().to_string()),
                _ => None,
            };
            let words = crate::crash::screen_line(
                self.settings.open_last,
                last.as_deref(),
                kept.as_deref(),
            );
            self.say(format!("project: safe start: {words}"));
            self.tell(&words);
            return;
        }
        if let Some(k) = &kept {
            let words = format!("Unsaved work from the last run is in Projects > {k}.");
            self.say(format!("project: {words}"));
            self.tell(&words);
        }
        if !self.settings.open_last {
            return;
        }
        let Some(b) = self.browser.as_mut() else {
            self.say(
                "project: At start is Last project, but there is no library (no --data)".into(),
            );
            return;
        };
        let line = match b.last_project().map(|l| l.name().to_string()) {
            Some(name) if b.open_last() => {
                format!("project: opening the last project, {name} (Prefs > At start)")
            }
            _ => "project: no project saved or opened yet; a new one (Prefs > At start)".into(),
        };
        self.say(line);
        self.step_browser();
    }

    /// An opened project's loops, once read: to the audio thread.
    fn poll_loops(&mut self) {
        let Some(rx) = &self.loops_load else {
            return;
        };
        let r = match rx.try_recv() {
            Ok(r) => r,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Err("its thread stopped".into()),
        };
        self.loops_load = None;
        match r {
            Ok((saved, notes)) => {
                for n in notes {
                    self.say(n);
                }
                self.outbox.push(FromUi::OpenLoops(Box::new(saved)));
                self.opened_at = Some(Instant::now());
            }
            Err(e) => self.say(format!("project: the loops weren't opened ({e})")),
        }
    }

    /// An opened project's audio tracks, once read: to the audio thread.
    fn poll_audio(&mut self) {
        let Some(rx) = &self.audio_load else {
            return;
        };
        let r = match rx.try_recv() {
            Ok(r) => r,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Err("its thread stopped".into()),
        };
        self.audio_load = None;
        match r {
            Ok((saved, notes)) => {
                for n in notes {
                    self.say(n);
                }
                self.outbox.push(FromUi::OpenAudio(Box::new(saved)));
                self.opened_at = Some(Instant::now());
            }
            Err(e) => self.say(format!("project: the audio tracks weren't opened ({e})")),
        }
    }

    /// A Q-Link on the Pads tab's browser: 1 scrolls, 3 is the preview
    /// level, 4 the places.
    fn browser_qlink(&mut self, knob: usize, ticks: i32) {
        let Some(b) = self.browser.as_mut() else {
            return;
        };
        if let mpfree_browser_ui::QLinkEffect::PreviewLevel(d) =
            mpfree_browser_ui::qlink_turn(b, knob, ticks)
        {
            self.preview_db = (self.preview_db + d)
                .clamp(crate::preview::LEVEL_MIN_DB, crate::preview::LEVEL_MAX_DB);
            let line = format!("preview level: {:.0} dB", self.preview_db);
            self.say(line);
        }
        self.step_browser();
    }

    // ------------------------------------------------------------ touch tuning

    /// A line for the terminal and the Events log.
    fn say(&mut self, l: String) {
        println!("{l}");
        let _ = io::stdout().flush();
        if let Some(e) = self.ui.get_mut::<LogPanel>("events") {
            e.push(&l, Tone::Dim);
        }
    }

    /// The tuning in use (applied to every finger from the next event).
    #[cfg(test)]
    pub fn tuning(&self) -> &Tuning {
        &self.tuning
    }

    fn set_tuning(&mut self, t: Tuning) {
        self.tuning = t;
        self.filter.set_tuning(t);
        if let Some(test) = self.ui.get_mut::<TouchTest>(touch_dev::TEST) {
            test.set_config(t.gestures);
        }
        if let Some(lab) = self.ui.get_mut::<TouchLabView>("touchlab") {
            lab.set_config(t.gestures);
        }
        self.refresh_tune();
    }

    fn screen_size(&self) -> (f32, f32) {
        let (w, h) = self.ui.size();
        (w as f32, h as f32)
    }

    /// The tuning list: values now, the suggestion and the status.
    fn refresh_tune(&mut self) {
        let size = self.screen_size();
        let Some(test) = self.ui.get::<TouchTest>(touch_dev::TEST) else {
            return;
        };
        let (sug, why) = touch_dev::suggestion(&self.tuning, test, size);
        let rows = touch_dev::rows(&self.tuning, &sug, &why);
        let status = if self.tuning != self.saved_tuning {
            (
                "Changed: applies now; Save keeps it for the next start".to_string(),
                Tone::Warn,
            )
        } else if self.tuning == Tuning::default() {
            (
                "Built-in values (nothing saved changes them)".to_string(),
                Tone::Dim,
            )
        } else {
            (
                "Saved values, applied to the whole app".to_string(),
                Tone::Good,
            )
        };
        if let Some(l) = self.ui.get_mut::<TuneList>(touch_dev::TUNE) {
            if l.rows != rows || l.status != status {
                l.rows = rows;
                l.status = status;
            }
        }
    }

    fn tune_select(&mut self, i: usize) {
        if let Some(l) = self.ui.get_mut::<TuneList>(touch_dev::TUNE) {
            l.selected = i.min(l.len() - 1);
        }
    }

    fn tune_step(&mut self, p: Param, d: i32) {
        let mut t = self.tuning;
        t.step(p, d);
        if p == Param::LongestTap && t.gestures.long_press_us < t.gestures.tap_max_us {
            t.gestures.long_press_us = t.gestures.tap_max_us;
        }
        if p == Param::LongPress && t.gestures.tap_max_us > t.gestures.long_press_us {
            t.gestures.tap_max_us = t.gestures.long_press_us;
        }
        if t != self.tuning {
            self.say(format!(
                "touch-tune: set {}={}",
                p.key(),
                p.show(t.get(p)).replace(' ', "")
            ));
            self.set_tuning(t);
        }
    }

    /// The list's actions: 0 Use suggested, 1 Defaults, 2 Save.
    fn tune_action(&mut self, i: usize) {
        match i {
            0 => {
                let size = self.screen_size();
                let Some(test) = self.ui.get::<TouchTest>(touch_dev::TEST) else {
                    return;
                };
                let (t, _) = touch_dev::suggestion(&self.tuning, test, size);
                self.say(format!("touch-tune: use-suggested {}", t.line()));
                self.set_tuning(t);
            }
            1 => {
                self.say("touch-tune: defaults".to_string());
                self.set_tuning(Tuning::default());
            }
            2 => {
                let t = self.tuning;
                self.say(format!("touch-tune: save {}", t.line()));
                self.saved_tuning = t;
                self.outbox.push(FromUi::SaveTouch(t));
                self.refresh_tune();
            }
            _ => {}
        }
    }

    /// The Touch page's page: Targets takes the whole page until its nine
    /// taps are done.
    fn touch_dev_page(&self) -> &'static str {
        match self.ui.get::<TouchTest>(touch_dev::TEST) {
            Some(t) if t.wants_whole_page() => touch_dev::TARGETS_PAGE,
            _ => touch_dev::PAGE,
        }
    }

    /// After a finger on the Touch page: its lines, the suggestion, and the
    /// page (Targets mode starting or done).
    fn after_touch_dev(&mut self) {
        if self.view != View::Touch || self.touch_page.is_some() {
            return;
        }
        let lines = match self.ui.get::<TouchTest>(touch_dev::TEST) {
            Some(t) if t.has_lines() => self
                .ui
                .get_mut::<TouchTest>(touch_dev::TEST)
                .map(|t| t.take_lines())
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        if !lines.is_empty() {
            for l in lines {
                self.say(l);
            }
            self.refresh_tune();
        }
        let page = self.touch_dev_page();
        if self.ui.page() != page && self.fingers.is_empty() {
            self.switch_page(page);
        }
    }

    /// Leaving the Touch page: back to Free, and a summary line.
    fn leave_touch_dev(&mut self) {
        let size = self.screen_size();
        let tuning = self.tuning;
        let Some(t) = self.ui.get_mut::<TouchTest>(touch_dev::TEST) else {
            return;
        };
        t.leave();
        let mut lines = t.take_lines();
        lines.push(t.summary_line(&tuning, size));
        lines.push("touch-tune-exit".to_string());
        for l in lines {
            self.say(l);
        }
    }

    fn open_touch_page(&mut self, widget: &'static str, page: &str) {
        self.ready = true;
        self.close_touch_page();
        self.touch_page = Some(widget);
        self.with_touch_page(|p| p.open());
        self.switch_page(page);
        self.print_touch_page_lines();
    }

    /// The open touch page, to read (nothing is redrawn).
    fn touch_page_ref(&self) -> Option<&dyn TouchPage> {
        match self.touch_page? {
            "touchlab" => self
                .ui
                .get::<TouchLabView>("touchlab")
                .map(|p| p as &dyn TouchPage),
            "calibrate" => self
                .ui
                .get::<CalibrateView>("calibrate")
                .map(|p| p as &dyn TouchPage),
            _ => None,
        }
    }

    /// Change the open touch page (it is redrawn).
    fn with_touch_page<R>(&mut self, f: impl FnOnce(&mut dyn TouchPage) -> R) -> Option<R> {
        match self.touch_page? {
            "touchlab" => self.ui.get_mut::<TouchLabView>("touchlab").map(|p| f(p)),
            "calibrate" => self.ui.get_mut::<CalibrateView>("calibrate").map(|p| f(p)),
            _ => None,
        }
    }

    /// Close the touch page if one is open (a step in progress is logged).
    fn close_touch_page(&mut self) {
        if self.touch_page.is_none() {
            return;
        }
        self.with_touch_page(|p| p.leave());
        self.print_touch_page_lines();
        self.touch_page = None;
    }

    fn print_touch_page_lines(&mut self) {
        if !self.touch_page_ref().is_some_and(|p| p.has_lines()) {
            return;
        }
        let lines = self.with_touch_page(|p| p.take_lines()).unwrap_or_default();
        for l in lines {
            println!("{l}");
            if let Some(e) = self.ui.get_mut::<LogPanel>("events") {
                e.push(&l, Tone::Dim);
            }
        }
        let _ = io::stdout().flush();
    }

    /// The widget of the touch page on screen, if any.
    #[cfg(test)]
    pub fn touch_page(&self) -> Option<&'static str> {
        self.touch_page
    }

    /// The touchscreen reader's latest state line, for the Touch lab's top bar.
    pub fn set_touch_status(&mut self, line: &str) {
        if let Some(lab) = self.ui.get_mut::<TouchLabView>("touchlab") {
            lab.set_reader(line);
        }
    }

    /// The kernel's timestamp of the finger event about to be passed to
    /// `touch` (the Touch lab measures speeds and jitter with it).
    pub fn set_touch_time(&mut self, time_us: u64) {
        self.with_touch_page(|p| p.set_event_time(time_us));
        if self.view == View::Touch {
            if let Some(t) = self.ui.get_mut::<TouchTest>(touch_dev::TEST) {
                t.set_event_time(time_us);
            }
        }
    }

    /// The quick band shows over the Play page while NOTE REPEAT is held or
    /// pads are latched.
    fn band_showing(&self) -> bool {
        self.status.repeat
    }

    /// The Q-Link cards for the audio region picked on Arrange's Song
    /// view, if one is (and NOTE REPEAT doesn't hold the knobs).
    fn audio_cards(&self) -> Option<crate::audiotracks::AudioKnobs> {
        if self.view != View::Arrange || self.band_showing() || !self.time.song_view {
            return None;
        }
        crate::audiotracks::audio_knobs(&self.time)
    }

    /// Q-Link `k` turned `ticks` with an audio region picked or a track
    /// panel open: to the audio tracks. False (nothing sent) when their
    /// cards do not show.
    fn audio_qlink(&mut self, k: usize, ticks: i32) -> bool {
        let Some(knobs) = self.audio_cards() else {
            return false;
        };
        self.outbox
            .push(FromUi::Time(TimeAction::Audio(AudioAction::Turn {
                track: knobs.track,
                region: knobs.region,
                knob: k,
                ticks,
                fine: self.status.shift,
            })));
        true
    }

    /// The pattern step the engine is on, while a pattern plays.
    fn playing_step(&self) -> Option<usize> {
        let a = &self.arp;
        (a.settings.mode == Mode::Pattern
            && !a.notes.is_empty()
            && a.step > 0
            && !a.pattern.steps().is_empty())
        .then(|| ((a.step - 1) as usize) % a.pattern.len())
    }

    /// Show the arp state on the panel, the band, the workspace and the chip.
    /// 16 LEVEL: on (of the pad hit last, by the kind picked last, with the
    /// panel) or, while on, off.
    fn toggle_levels(&mut self) {
        let want = match self.sample.levels() {
            Some(_) => None,
            None => Some((self.sample.pad(), self.levels.by)),
        };
        if !self.sample.set_levels(&mut self.ui, want) {
            let l = "16 LEVEL: there is no kit to play 16 Levels of yet";
            println!("{l}");
            if let Some(e) = self.ui.get_mut::<LogPanel>("events") {
                e.push(l, Tone::Dim);
            }
            return;
        }
        self.levels.open = want.is_some();
        self.levels.picking = false;
        self.after_levels_change();
    }

    /// The levels step through `by` from now on.
    fn levels_by(&mut self, by: Levels) {
        self.levels.by = by;
        if let Some((pad, _)) = self.sample.levels() {
            self.sample.set_levels(&mut self.ui, Some((pad, by)));
            self.after_levels_change();
        } else {
            self.refresh_levels();
        }
    }

    /// A pad down (`down`) or up: lit on the panel while held, and with
    /// Pick pad armed, the pad the levels play from now on.
    fn levels_hit(&mut self, pad: usize, down: bool) {
        if pad >= 16 || self.sample.levels().is_none() {
            return;
        }
        if down && self.levels.picking {
            self.levels.picking = false;
            self.sample
                .set_levels(&mut self.ui, Some((pad, self.levels.by)));
            self.after_levels_change();
        }
        self.levels.lit = if down {
            Some(pad)
        } else if self.levels.lit == Some(pad) {
            None
        } else {
            self.levels.lit
        };
        self.refresh_levels();
    }

    /// The kit's pads changed between their own sounds and 16 Levels: the
    /// panel, its page, the names on Play and the edits go out.
    fn after_levels_change(&mut self) {
        let on = self.sample.levels().is_some();
        if !on {
            self.levels.open = false;
            self.levels.picking = false;
            self.levels.lit = None;
        }
        self.sample_names();
        self.sample_changed();
        self.refresh_levels();
        if self.view == View::Play {
            self.show_view(View::Play);
        }
        self.refresh_hint();
    }

    /// The panel shows the kit as it is; the app hears when 16 Levels or
    /// the panel turned on or off.
    fn refresh_levels(&mut self) {
        let levels = self.sample.levels();
        if levels.is_none() && self.levels.open {
            self.levels.open = false;
            self.levels.picking = false;
            if self.view == View::Play {
                self.show_view(View::Play);
            }
        }
        if let Some((_, by)) = levels {
            self.levels.by = by;
        }
        let pad = levels.map_or(self.sample.pad(), |(p, _)| p);
        let (pad_label, pad_name) = self.sample.pad_names(pad);
        let model = LevelsModel {
            pad_label,
            pad_name,
            by: self.levels.by,
            picking: self.levels.picking,
            lit: self.levels.lit,
            colour: crate::akai::kit_colour(self.sample.kit()),
        };
        if let Some(w) = self.ui.get_mut::<LevelsPanel>(levels_panel::PANEL) {
            if w.model != model {
                w.model = model;
            }
        }
        let now = (levels.is_some(), self.levels.open);
        if self.levels.told != Some(now) {
            self.levels.told = Some(now);
            // The 16 Levels chip comes or goes.
            self.refresh_chips();
            self.outbox.push(FromUi::Levels {
                on: now.0,
                open: now.1,
            });
        }
    }

    fn refresh_arp(&mut self) {
        let _ = self.arp_panel.update(&mut self.ui, &self.arp);
        let playing = self.playing_step();
        self.workspace.played = playing.map(|_| self.arp.step - 1);
        self.workspace.update(&mut self.ui, playing);
        self.refresh_chips();
        self.refresh_hint();
        self.refresh_qlinks();
    }

    /// Change page, first letting go of every finger on the screen (the
    /// page's widgets would never hear them lift).
    fn switch_page(&mut self, page: &str) {
        if self.ui.page() == page {
            return;
        }
        for f in std::mem::take(&mut self.fingers) {
            if let Some(ev) = self.ui.touch(f, Touch::Cancel, 0, 0) {
                if let Some(a) = self.on_event(ev) {
                    self.outbox.push(a);
                }
            }
        }
        let _ = self.ui.show(page);
    }

    /// Actions that did not come from a touch, for the audio thread.
    pub fn take_actions(&mut self) -> Vec<FromUi> {
        if let Some(a) = self.sample.take_view() {
            self.outbox.push(a);
        }
        if let Some(a) = self.sample.take_input() {
            self.outbox.push(a);
        }
        // Flatten's hits go after the source it records from.
        let hits = self.sample.take_hits();
        self.outbox.extend(hits);
        if self.view == View::Sample {
            if let Some(a) = self.sample.take_cloud() {
                self.outbox.push(a);
            }
        }
        // A stretched copy coming in says so.
        self.print_sample_lines();
        let out = std::mem::take(&mut self.outbox);
        let own = std::mem::take(&mut self.open_actions).min(out.len());
        for a in &out[own..] {
            self.saw_action(a);
        }
        out
    }

    /// The work changed since it was saved or opened when the screen sent
    /// an edit (not a pad played, a page or kit picked, the Level, the
    /// transport, a preview), so opening another project or New asks
    /// first.
    pub fn saw_action(&mut self, a: &FromUi) {
        // The swing is the project's; play, stop and the rest aren't.
        if let FromUi::Time(TimeAction::Transport(c)) = a {
            if matches!(c, mpfree_transport::Command::SetSwing(_)) {
                self.mark_unsaved();
            }
            return;
        }
        let edit = !matches!(
            a,
            FromUi::Pad { .. }
                | FromUi::PadsView(_)
                | FromUi::Preview(_)
                | FromUi::PreviewStop
                | FromUi::SaveBrowserLists(_)
                | FromUi::SaveProject
                | FromUi::OpenProject(_)
                | FromUi::OpenLoops(_)
                | FromUi::OpenAudio(_)
                | FromUi::Kit(_)
                | FromUi::LevelDb(_)
                | FromUi::Tab(_)
                | FromUi::MenuSelect(_)
                | FromUi::GuideButton(_)
        );
        if edit {
            self.mark_unsaved();
        }
    }

    /// The work changed (an edit seen on the screen, or the engines saying
    /// so), unless an open has only just landed (its own news).
    fn mark_unsaved(&mut self) {
        if self.opened_at.is_some_and(|t| t.elapsed() < OPEN_SETTLE) {
            return;
        }
        if let Some(b) = self.browser.as_mut() {
            b.set_unsaved(true);
            self.autosave_dirty = true;
        }
    }

    fn show_view(&mut self, v: View) {
        self.close_touch_page();
        // Any page change closes the list of kits.
        self.kit_list = false;
        let page = match v {
            View::Play if self.band_showing() && self.ready => QUICK_PAGE.to_string(),
            View::Play if self.levels.open && self.ready => LEVELS_PAGE.to_string(),
            View::Play => self
                .play
                .view
                .page_for(self.settings.timing_strip)
                .to_string(),
            View::Arrange => CLIPS_PAGE.to_string(),
            View::Synth => "Synth".to_string(),
            View::Looper => LOOPER_PAGE.to_string(),
            View::Sample => {
                let name = self.auto_synth_name();
                self.sample.set_auto_synth(&mut self.ui, name);
                self.sample.show_sample(&mut self.ui);
                self.sample.page().to_string()
            }
            View::Pads => {
                self.sample.show_pads(&mut self.ui);
                self.sample.page().to_string()
            }
            // Without the developer tools the Console has no strip.
            View::Console if !self.settings.debug_mode => "Console".to_string(),
            View::Touch => {
                DebugPages::set_strip(&mut self.ui, v);
                self.touch_dev_page().to_string()
            }
            View::Debug | View::Perf | View::Engines | View::Procs | View::Map | View::Console => {
                DebugPages::set_strip(&mut self.ui, v);
                DebugPages::page(v).unwrap_or("Debug").to_string()
            }
            View::System => {
                self.set_system(crate::sysinfo::read(std::path::Path::new("/")));
                "System info".to_string()
            }
            View::Prefs => "Prefs".to_string(),
            View::Mixer => mpfree_mixer_ui::MIXER_PAGE.to_string(),
            View::Chain => mpfree_chain_ui::CHAIN_PAGE.to_string(),
            View::Macros => mpfree_chain_ui::MACROS_PAGE.to_string(),
            View::Options => "Options".to_string(),
            View::Guide => "Guide".to_string(),
            View::Arp => ARP_PAGE.to_string(),
            View::Pattern => self.workspace.page().to_string(),
            View::Browser => {
                // The same browser as the Pads tab's, its picker put
                // away: the whole library, Projects with New, Save and
                // Save as.
                if let Some(b) = self.browser.as_mut() {
                    if b.picking().is_some() {
                        b.cancel_picker();
                        b.take_actions();
                    }
                } else {
                    self.say("browser: no library (the app was started without --data); nothing can be saved or opened".to_string());
                }
                mpfree_browser_ui::BROWSER_PAGE.to_string()
            }
        };
        if self.view == View::Browser && v != View::Browser {
            // The Pads tab's picker again, where it was.
            if let Some(b) = self.browser.as_mut() {
                b.open_picker(mpfree_browser_ui::Pick::Pads);
            }
        }
        let relabel = (self.view == View::Pattern) != (v == View::Pattern);
        if self.view.sampler() && v != self.view {
            // A pad a tap on the waveform (or the pad map) holds is let go.
            self.sample.leave(&mut self.outbox);
        }
        let (was, now) = (self.view == View::Touch, v == View::Touch);
        if was && !now {
            self.leave_touch_dev();
        }
        self.switch_page(&page);
        self.view = v;
        self.chain_page.entered(v);
        if now && !was {
            self.say("touch-tune-open".to_string());
            self.refresh_tune();
        }
        if self.browsing() {
            // The list's rows fit the room the browser has now.
            self.step_browser();
        }
        if v == View::Arrange {
            // How many slots fit is known once the grid is on screen.
            self.time_page.update(&mut self.ui, &self.time);
        }
        if matches!(v, View::Looper | View::Play) {
            self.looper_page
                .update(&mut self.ui, &self.looper, &self.settings);
        }
        if relabel {
            self.label_pads();
        }
        if v == View::Synth {
            // Meters changed while the page was hidden.
            if let Some(p) = self.synth.as_mut() {
                p.refresh(&mut self.ui);
            }
        }
        self.mark_tab();
        self.refresh_hint();
        self.refresh_qlinks();
    }

    /// The kit chip: the list of kits over the page, or the page again.
    fn toggle_kit_list(&mut self) {
        if self.kit_list {
            self.show_view(self.view);
            return;
        }
        self.close_touch_page();
        self.switch_page(KITS_PAGE);
        self.kit_list = true;
        self.refresh_hint();
    }

    /// The header highlights the page's tab, or none (Prefs, menus, the guide).
    fn mark_tab(&mut self) {
        let tab = self.view.tab_index(self.settings.debug_mode);
        // One icon lit: the I/O icon on the I/O view, the mixer's on the
        // others.
        let io = self.io_showing();
        let mixer = Some(self.view == View::Mixer && !io);
        if let Some(h) = self.ui.get_mut::<Header>("header") {
            h.active = tab.unwrap_or(usize::MAX);
            if h.mixer_icon != mixer {
                h.mixer_icon = mixer;
            }
            if h.io_icon.is_some() && h.io_icon != Some(io) {
                h.io_icon = Some(io);
            }
        }
    }

    /// Prefs > System info's rows.
    pub fn set_system(&mut self, info: crate::sysinfo::SystemInfo) {
        for (name, rows) in [("syssw", info.software), ("syshw", info.hardware)] {
            if let Some(l) = self.ui.get_mut::<StatList>(name) {
                l.set(
                    rows.into_iter()
                        .map(|(k, v)| (k, v, Tone::Normal))
                        .collect(),
                );
            }
        }
    }

    /// Debug mode is on (the screen thread then reads /proc for the
    /// Performance page and the `perf:` lines).
    pub fn debug_mode(&self) -> bool {
        self.settings.debug_mode
    }

    /// The page showing now.
    pub fn view(&self) -> View {
        self.view
    }

    /// The CPU by engine page shows: read the audio loop's latest numbers.
    pub fn refresh_engines(&mut self) {
        if let Some(shared) = self.engines.as_ref() {
            let snap = crate::engines::read(shared);
            let mut rows = crate::cores::rows(&shared.cores);
            rows.extend(crate::parts::rows(&shared.parts));
            let (rss_kb, total_kb) = self.debug.memory_kb();
            let memory = crate::memory::rows(&crate::memory::Held {
                samples: self.sample.memory(),
                loops: (self.looper.memory.used, self.looper.memory.capacity),
                synth: mpfree_synth::wavetable::factory::factory_bytes(),
                rss_kb,
                total_kb,
            });
            DebugPages::set_engines(&mut self.ui, &snap, memory, rows);
        }
    }

    /// One /proc sample for the Processes page.
    pub fn set_procs(&mut self, v: &mpfree_prof::procs::ProcView) {
        DebugPages::set_procs(&mut self.ui, v);
    }

    /// The memory in use, % of all, for the header's RAM chip (the screen
    /// thread reads /proc/meminfo every 2 s).
    pub fn set_ram(&mut self, pct: f64) {
        let changed = self.ram_pct.is_none_or(|p| p.round() != pct.round());
        self.ram_pct = Some(pct);
        if changed {
            self.refresh_chips();
        }
    }

    /// Process and system numbers, from the screen thread's sampler.
    pub fn set_proc(&mut self, v: ProcView) {
        self.debug.set_proc(&mut self.ui, v);
    }

    /// A touch: what it means for the app, if anything. Before the app is
    /// ready, the pages are switched here (loading and console only).
    pub fn touch(&mut self, finger: u64, phase: Touch, x: i32, y: i32) -> Option<FromUi> {
        // The Touch page measures every finger before the tuning.
        if self.ready && self.touch_page.is_none() && self.view == View::Touch {
            let area = self
                .ui
                .rect_of(touch_dev::TEST)
                .unwrap_or(mpfree_ui::Rect::new(0, 0, 0, 0));
            if let Some(t) = self.ui.get_mut::<TouchTest>(touch_dev::TEST) {
                t.observe(finger, phase, x as f32, y as f32, area);
            }
        }
        // The tuning: the correction, and the hold filter's dropped moves.
        let (x, y) = match self
            .filter
            .apply(finger, touch_phase(phase), x as f32, y as f32)
        {
            Some((fx, fy)) => (fx.round() as i32, fy.round() as i32),
            None => {
                self.after_touch_dev();
                return None;
            }
        };
        let action = self.touch_filtered(finger, phase, x, y);
        self.after_touch_dev();
        action
    }

    fn touch_filtered(&mut self, finger: u64, phase: Touch, x: i32, y: i32) -> Option<FromUi> {
        match phase {
            Touch::Down => self.fingers.push(finger),
            Touch::Up | Touch::Cancel => self.fingers.retain(|f| *f != finger),
            Touch::Move => {}
        }
        if !self.ready {
            let on_loading = self.ui.page() == "Loading";
            let ev = self.ui.touch(finger, phase, x, y);
            if phase == Touch::Down {
                if on_loading {
                    self.show_view(View::Console);
                } else if matches!(
                    ev,
                    Some(UiEvent {
                        kind: EventKind::Tab(_),
                        ..
                    })
                ) {
                    let _ = self.ui.show("Loading");
                }
            }
            return None;
        }
        if self.touch_page.is_some() {
            // A touch page takes every finger and means nothing to the app.
            let _ = self.ui.touch(finger, phase, x, y);
            self.print_touch_page_lines();
            if self.touch_page_ref().is_some_and(|p| p.wants_exit()) {
                self.show_view(self.view);
            }
            return None;
        }
        if matches!(self.view, View::Play | View::Arrange | View::Chain) {
            self.watch_card_press(finger, phase, x, y);
        }
        let action = self
            .ui
            .touch(finger, phase, x, y)
            .and_then(|ev| self.on_event(ev));
        self.watch_tempo_press(finger, phase, x, y, &action);
        if matches!(phase, Touch::Up | Touch::Cancel) && self.view.sampler() {
            // A drag on the Sample page ends with its finger: one undo step.
            self.sample.lift();
            self.print_sample_lines();
        }
        if self.browsing() {
            // A tap on the browser shows at once, not a frame later.
            self.step_browser();
        }
        action
    }

    /// A finger on a Q-Link card, for the long press on a macro card that
    /// opens the macros panel ([`ChainPage::held`], from `tick`).
    fn watch_card_press(&mut self, finger: u64, phase: Touch, x: i32, y: i32) {
        match phase {
            Touch::Down => {
                let card = self
                    .ui
                    .rect_of(play_page::QLINKS)
                    .and_then(|r| QLinkColumn::card_at(r, x, y));
                if card.is_some() {
                    self.chain_page.press(finger, card, y);
                }
            }
            Touch::Move => self.chain_page.moved(finger, y),
            Touch::Up | Touch::Cancel => self.chain_page.lifted(finger),
        }
    }

    /// A finger on Play's BPM (its tap taps the tempo): held still for
    /// [`TEMPO_HOLD`], it opens the number pad for the tempo
    /// ([`Self::tick`]). A slide or a lift lets it go.
    fn watch_tempo_press(
        &mut self,
        finger: u64,
        phase: Touch,
        x: i32,
        y: i32,
        action: &Option<FromUi>,
    ) {
        match phase {
            Touch::Down if matches!(action, Some(FromUi::TapTempo)) => {
                self.tempo_press = Some((finger, 0.0, x, y));
            }
            Touch::Move => {
                if let Some((f, _, x0, y0)) = self.tempo_press {
                    if f == finger && ((x - x0).pow(2) + (y - y0).pow(2)) > 20 * 20 {
                        self.tempo_press = None;
                    }
                }
            }
            Touch::Up | Touch::Cancel if self.tempo_press.is_some_and(|p| p.0 == finger) => {
                self.tempo_press = None;
            }
            _ => {}
        }
    }

    /// A touch meant something: hand it to the audio thread, which decides.
    pub fn on_event(&mut self, ev: UiEvent) -> Option<FromUi> {
        let synth_widget = [
            synth_view::TITLE,
            synth_view::INST,
            synth_view::TILES,
            synth_view::PRESETS,
        ]
        .contains(&ev.widget.as_str());
        if synth_widget {
            let p = self.synth.as_mut()?;
            let a = p.event(&mut self.ui, &ev).map(FromUi::Synth);
            // The page or a value changed: the Q-Link column follows.
            self.refresh_qlinks();
            return a;
        }
        if ev.widget.starts_with(sample_page::PREFIX) {
            let before = self.sample.page();
            let name = self.auto_synth_name();
            self.sample.set_auto_synth(&mut self.ui, name);
            self.sample.event(&mut self.ui, &ev, &mut self.outbox);
            if let Some(ask) = self.sample.take_number_ask() {
                self.open_numpad(ask, TypeTo::Sample);
            }
            if let Some(plan) = self.sample.take_auto_request() {
                // A copy of the patch, made here: the audio thread keeps
                // playing its own and is never asked for anything.
                let node = self.synth.as_ref().and_then(|p| {
                    crate::synth::fresh_node(p.state.info.id, &p.state.values, plan.rate)
                });
                self.sample.start_auto(&mut self.ui, plan, node);
            }
            self.sample_after(before);
            self.print_sample_lines();
            return None;
        }
        if self.status.erase {
            if let Some(answer) = self.erase_tap(&ev) {
                return answer;
            }
        }
        match (ev.widget.as_str(), ev.kind.clone()) {
            ("header", EventKind::Tab(i)) => {
                // The header moved its own highlight; the page follows when the app says.
                self.mark_tab();
                Some(FromUi::Tab(i))
            }
            (crate::debug_page::STRIP, EventKind::Tab(i)) => {
                DebugPages::set_strip(&mut self.ui, self.view);
                Some(FromUi::DebugPage(i))
            }
            (touch_dev::TUNE, EventKind::Select(i)) => {
                self.tune_select(i);
                None
            }
            (touch_dev::TUNE, EventKind::Step { index, delta }) => {
                if let Some(&p) = Param::ALL.get(index) {
                    self.tune_select(index);
                    self.tune_step(p, delta);
                }
                None
            }
            (touch_dev::TUNE, EventKind::Tab(i)) => {
                self.tune_select(Param::ALL.len() + i);
                self.tune_action(i);
                None
            }
            ("controls", EventKind::Select(i)) => Some(FromUi::MapRow(i)),
            ("mapcard", EventKind::Tab(i)) => Some(FromUi::MapButton(i)),
            (
                crate::looper_tab::LOOPS
                | crate::looper_tab::EDITOR
                | crate::looper_tab::RECORDING
                | crate::looper_tab::USE,
                _,
            ) => self.looper_acts(),
            (crate::looper_tab::QLINKS, EventKind::Step { index, delta }) => {
                self.looper_qlink(index, delta);
                None
            }
            (play_page::PADS, EventKind::Pad { index, down }) => {
                Some(FromUi::Pad { pad: index, down })
            }
            (play_page::PADS, EventKind::Tab(i)) => Some(FromUi::PadsLayout { in_key: i == 1 }),
            ("kits", EventKind::Select(k)) => {
                if self.kit_list {
                    self.show_view(self.view);
                }
                Some(FromUi::Kit(k))
            }
            // The list of kits' Back.
            ("kits-card", EventKind::Tab(_)) => {
                self.show_view(self.view);
                None
            }
            (play_page::TRANSPORT, EventKind::Select(crate::play_widgets::TAP_BPM)) => {
                Some(FromUi::TapTempo)
            }
            (NUMPAD, EventKind::Tab(mpfree_ui::numpad::KEY_ENTER)) => self.numpad_enter(),
            (NUMPAD, EventKind::Tab(mpfree_ui::numpad::KEY_CANCEL)) => {
                self.close_numpad();
                None
            }
            (NUMPAD, _) => None,
            (HISTORY, EventKind::Select(row)) => {
                self.history_jump(row);
                None
            }
            (HISTORY, EventKind::Tab(mpfree_ui::history::TAB_CLOSE)) => {
                self.close_history();
                None
            }
            (HISTORY, _) => None,
            (play_page::TRANSPORT, EventKind::Tab(i)) => {
                self.show_play_view(PlayView::from_index(i));
                None
            }
            (play_page::MASTER, EventKind::Value(v)) => {
                Some(FromUi::LevelDb(mpfree_ui::draw::meter_db(v)))
            }
            // A blank card has nothing to move here: say why in place.
            (play_page::QLINKS, EventKind::Step { index, .. }) if self.qlink_blank(index) => {
                let why = self.qlink_blank_why();
                self.tell(why);
                None
            }
            (play_page::QLINKS, EventKind::Step { index, delta }) if self.view == View::Mixer => {
                self.mixer_qlink(index, delta);
                None
            }
            (play_page::QLINKS, EventKind::Step { index, delta }) if self.view == View::Synth => {
                let a = self.synth.as_mut()?.qlink(&mut self.ui, index, delta);
                self.refresh_qlinks();
                a.map(FromUi::Synth)
            }
            (play_page::QLINKS, EventKind::Step { index, delta })
                if self.chain_page.drives(self.view) && !self.band_showing() =>
            {
                self.chain_qlink(index, delta);
                None
            }
            (play_page::QLINKS, EventKind::Step { index, delta })
                if matches!(self.view, View::Play | View::Arrange) && !self.band_showing() =>
            {
                if !self.audio_qlink(index, delta) {
                    self.play_qlink(index, delta);
                }
                None
            }
            (play_page::STEPS, EventKind::Cell { row, col }) => {
                let a = self.play.tap_step(row, col);
                self.refresh_play();
                self.refresh_qlinks();
                a.map(FromUi::Steps)
            }
            // The pattern head: the page, and the length (32, 64, 128 steps).
            (play_page::STEPS, EventKind::Tab(k))
                if k == play_widgets::HEAD_PREV || k == play_widgets::HEAD_NEXT =>
            {
                let d = if k == play_widgets::HEAD_PREV { -1 } else { 1 };
                if !self.play.turn_page(d) {
                    self.tell(if d < 0 {
                        "steps: this is the first page"
                    } else {
                        "steps: this is the last page (+ adds steps)"
                    });
                }
                self.refresh_play();
                self.refresh_qlinks();
                None
            }
            (play_page::STEPS, EventKind::Tab(k)) => {
                match self.play.length_action(k == play_widgets::HEAD_LONGER) {
                    Ok(a) => Some(FromUi::Steps(a)),
                    Err(why) => {
                        self.tell(&why);
                        None
                    }
                }
            }
            (play_page::STEPS, EventKind::Toggle(pad)) if pad < 16 => {
                let mute = self.play.levels.pad_mutes & (1 << pad) == 0;
                self.play.levels.pad_mutes ^= 1 << pad;
                self.refresh_play();
                Some(FromUi::PlayFx(PlayFx::PadMute { pad, mute }))
            }
            (play_page::THROWS, EventKind::Pad { index, down }) => {
                let fx = self.play.throw(index, down);
                self.refresh_play();
                fx.map(FromUi::PlayFx)
            }
            (play_page::XY, EventKind::Xy { x, y, down }) => {
                let fx = self.play.xy(x, y, down);
                self.refresh_play();
                Some(FromUi::PlayFx(fx))
            }
            (crate::looper_page::LOOP_STRIP, EventKind::Toggle(k)) => {
                let l = self.looper_page.layer_info(k)?;
                Some(FromUi::LoopLayer {
                    id: l.id,
                    command: mpfree_looper::LayerCommand::Mute(!l.muted),
                })
            }
            (crate::looper_page::LOOP_STRIP, EventKind::Tab(i)) => {
                use crate::looper_page::StripPress;
                match crate::looper_page::STRIP_PRESSES.get(i).copied()? {
                    StripPress::Looper(b) => Some(FromUi::Looper(b)),
                    StripPress::Play => Some(FromUi::Time(TimeAction::Transport(
                        mpfree_transport::Command::Play,
                    ))),
                    StripPress::Stop => Some(FromUi::Time(TimeAction::Transport(
                        mpfree_transport::Command::Stop,
                    ))),
                }
            }
            (play_page::QLINKS, EventKind::Step { index, delta }) => {
                Some(FromUi::QLink { knob: index, delta })
            }
            (mpfree_mixer_ui::MIXER, _) => {
                let mut acts = mpfree_mixer_ui::take_actions(&mut self.ui)
                    .into_iter()
                    .map(FromUi::Mixer);
                let first = acts.next();
                self.outbox.extend(acts);
                if self.io_showing() {
                    // A section picked, or a detail opened: its hint.
                    let _ = mpfree_mixer_ui::take_io_changes(&mut self.ui);
                    self.refresh_hint();
                    self.say_io_note();
                }
                self.refresh_qlinks();
                first
            }
            (levels_panel::PANEL, _) => {
                let actions = self
                    .ui
                    .get_mut::<LevelsPanel>(levels_panel::PANEL)
                    .map(LevelsPanel::take_actions)
                    .unwrap_or_default();
                for a in actions {
                    match a {
                        LevelsAction::By(by) => self.levels_by(by),
                        LevelsAction::PickPad => {
                            self.levels.picking = !self.levels.picking;
                            self.refresh_levels();
                        }
                        LevelsAction::Close => {
                            self.levels.open = false;
                            self.levels.picking = false;
                            self.refresh_levels();
                            if self.view == View::Play {
                                self.show_view(View::Play);
                            }
                            self.refresh_hint();
                        }
                        LevelsAction::Off => {
                            self.sample.set_levels(&mut self.ui, None);
                            self.after_levels_change();
                        }
                    }
                }
                None
            }
            (mpfree_chain_ui::CHAIN, _) => {
                let mut acts = mpfree_chain_ui::take_chain_actions(&mut self.ui)
                    .into_iter()
                    .map(FromUi::Chain);
                let first = acts.next();
                self.outbox.extend(acts);
                first
            }
            (mpfree_chain_ui::MACROS_W, _) => {
                let acts = mpfree_chain_ui::take_macro_actions(&mut self.ui);
                for a in &acts {
                    if let MacroAction::Bank(b) = a {
                        self.chain_page.panel_bank(*b);
                        self.refresh_qlinks();
                    }
                }
                let mut acts = acts.into_iter().map(FromUi::Macros);
                let first = acts.next();
                self.outbox.extend(acts);
                first
            }
            ("header", EventKind::Select(mpfree_ui::widgets::MIXER_ICON)) => {
                Some(FromUi::MixerIcon)
            }
            ("header", EventKind::Select(mpfree_ui::widgets::IO_ICON)) => Some(FromUi::IoIcon),
            (play_page::SYNTH_PADS_HEAD, EventKind::Tab(i)) => {
                Some(FromUi::PadsLayout { in_key: i == 1 })
            }
            ("options", EventKind::Select(i)) => Some(FromUi::MenuSelect(i)),
            ("options", EventKind::Step { index, delta }) => Some(FromUi::MenuStep(index, delta)),
            ("guide", EventKind::Tab(i)) => Some(FromUi::GuideButton(i)),
            ("header", EventKind::Select(i)) if Some(i) == self.arp_chip => Some(FromUi::ArpChip),
            ("header", EventKind::Select(i)) if Some(i) == self.kit_chip => {
                self.toggle_kit_list();
                None
            }
            ("header", EventKind::Select(i)) if Some(i) == self.level_chip => {
                Some(FromUi::PadLevelOff)
            }
            ("header", EventKind::Select(i))
                if self.chip_why.get(i).is_some_and(|w| !w.is_empty()) =>
            {
                let why = self.chip_why[i].clone();
                self.tell(&why);
                None
            }
            ("header", EventKind::Select(i)) if Some(i) == self.tempo_chip => {
                // The I/O view at Sync; before the Sync section exists, the
                // I/O view as it was left.
                if !self.open_io_section(IO_SYNC) {
                    self.outbox.push(FromUi::OpenIo);
                }
                None
            }
            (w, _) if w.starts_with("arp-ws-") => {
                // The editor changes at once; the app hears the new pattern.
                let a = self.workspace.action(&ev);
                let playing = self.playing_step();
                self.workspace.update(&mut self.ui, playing);
                self.refresh_hint();
                a.map(|a| FromUi::Workspace(Box::new(a)))
            }
            (w, _) if w.starts_with("arp-") => {
                let a = self.arp_panel.action(&ev, &self.arp, &mut self.qlink_focus);
                if a.is_none() {
                    if let Some(why) = self.arp_panel.why_not(&ev, &self.arp) {
                        self.tell(why);
                    }
                }
                a.map(FromUi::Arp)
            }
            // A track's sound line: its instrument on the Synth tab.
            (time_view::CLIPS, EventKind::Tab(k)) if k >= mpfree_time_ui::widgets::SOUND_TAP => {
                Some(FromUi::TrackInstrument(
                    k - mpfree_time_ui::widgets::SOUND_TAP,
                ))
            }
            (
                time_view::TRANSPORT
                | time_view::CLIPS
                | time_view::RECORD
                | time_view::TAKES
                | time_view::MORPH
                | time_view::SONG
                | time_view::NOTES
                | time_view::EDIT_OPTIONS,
                _,
            ) => {
                let a = time_view::action(&ev, &self.time);
                // A faint button or a setting at its end says why in place.
                if a.is_none() {
                    if let Some(why) = time_view::why_not(&ev, &self.time) {
                        self.tell(&why);
                    }
                }
                // A swipe across a take lane and a region's drag are free
                // with SHIFT held, and SHIFT + Undo redoes (layout spec 4c).
                let fine = self.status.shift;
                a.map(|a| {
                    let a = match a {
                        TimeAction::Audio(AudioAction::Undo) if fine => {
                            TimeAction::Audio(AudioAction::Redo)
                        }
                        TimeAction::Audio(AudioAction::Comp { take, from, to, .. }) => {
                            TimeAction::Audio(AudioAction::Comp {
                                take,
                                from,
                                to,
                                fine,
                            })
                        }
                        TimeAction::Audio(AudioAction::Move { region, to, .. }) => {
                            TimeAction::Audio(AudioAction::Move { region, to, fine })
                        }
                        a => a,
                    };
                    FromUi::Time(a)
                })
            }
            _ => None,
        }
    }

    /// Advance animations by `dt` seconds.
    pub fn tick(&mut self, dt: f32) {
        // UNDO held: the page's undo history.
        if let Some(held) = self.undo_held {
            let held = held + dt.max(0.0);
            if held >= UNDO_HOLD {
                self.undo_held = None;
                self.open_history();
            } else {
                self.undo_held = Some(held);
            }
        }
        // The undo history open: it follows its history.
        if let Some(h) = self.history.as_mut() {
            h.ask_in -= dt.max(0.0);
            self.refresh_history();
        }
        // A finger held on Play's BPM: type the tempo.
        if let Some((f, held, x, y)) = self.tempo_press {
            let held = held + dt.max(0.0);
            if held >= TEMPO_HOLD {
                self.tempo_press = None;
                if self.view == View::Play && self.typing.is_none() {
                    self.type_tempo();
                }
            } else {
                self.tempo_press = Some((f, held, x, y));
            }
        }
        // The UNDO light follows whether the page has something to undo.
        let ready = self.undo_ready();
        if ready != self.undo_ready {
            self.undo_ready = ready;
            self.outbox.push(FromUi::UndoReady(ready));
        }
        // A told line goes back to the hint once read.
        if let Some((_, left)) = self.told.as_mut() {
            *left -= dt.max(0.0);
            if *left <= 0.0 {
                self.told = None;
                self.refresh_hint();
            }
        }
        // The I/O section showing updates its live values (lights, the
        // clock's tempo); it marks what it needs drawn.
        self.io_clock += Duration::from_secs_f32(dt.max(0.0));
        if self.io_showing() {
            mpfree_mixer_ui::io_tick(&mut self.ui, self.io_clock);
        }
        // The XY pad glides home after the finger lifts.
        if let Some(fx) = self.play.tick_xy(dt * 1000.0) {
            self.outbox.push(FromUi::PlayFx(fx));
            self.refresh_play();
        }
        // A finger held on an effect in the picker opens its presets.
        if self.view == View::Chain && mpfree_chain_ui::chain_hold(&mut self.ui) {
            let acts = mpfree_chain_ui::take_chain_actions(&mut self.ui);
            self.outbox.extend(acts.into_iter().map(FromUi::Chain));
        }
        // A finger held still on a macro card opens the macros panel.
        if self.chain_page.drives(self.view) && !self.band_showing() {
            if let Some(bank) = self.chain_page.held(dt) {
                self.outbox.push(FromUi::OpenMacros(bank));
            }
        }
        // A take records on, whatever shows; it lands on its pad here.
        let before = self.sample.page();
        self.sample.tick(&mut self.ui, dt, self.view.sampler());
        // The browser: its touches and the worker's answers (a save's too,
        // made from another page); a preview's file and an opened
        // project's loops once read.
        if self.browsing() || self.browser.as_ref().is_some_and(|b| b.busy()) {
            self.step_browser();
        }
        self.poll_preview();
        self.poll_loops();
        self.poll_audio();
        self.open_at_start();
        self.autosave_tick(dt);
        if let Some(k) = self.sample.take_wanted_kit() {
            // An opened project's kit: the audio thread switches and says so.
            self.outbox.push(FromUi::Kit(k));
        }
        self.sample_after(before);
        self.print_sample_lines();
        self.cpu_at += dt;
        if self.cpu_at >= 1.0 {
            self.cpu_at = 0.0;
            self.refresh_chips();
        }
        if self
            .ui
            .get::<MiniPads>(play_page::PADS)
            .map(|m| &m.grid)
            .is_some_and(|g| g.fading())
        {
            if let Some(g) = self
                .ui
                .get_mut::<MiniPads>(play_page::PADS)
                .map(|m| &mut m.grid)
            {
                g.fade(dt);
            }
        }
        if self.touch_page_ref().is_some_and(|p| p.animating()) {
            self.with_touch_page(|p| p.tick());
            self.print_touch_page_lines();
        }
        if self.view == View::Touch
            && self.touch_page.is_none()
            && self
                .ui
                .get::<TouchTest>(touch_dev::TEST)
                .is_some_and(mpfree_ui::Widget::animating)
        {
            if let Some(t) = self.ui.get_mut::<TouchTest>(touch_dev::TEST) {
                t.tick();
            }
        }
    }

    pub fn set_screen_row(&mut self, text: String, tone: Tone) {
        self.screen_row = (text, tone);
        self.show_timing();
    }
}

/// A colour for a system log line.
fn console_tone(line: &str) -> Tone {
    let l = line.to_ascii_lowercase();
    if l.contains("error") || l.contains("fail") || l.contains("stop:") {
        Tone::Bad
    } else if l.contains("warn") {
        Tone::Warn
    } else {
        Tone::Normal
    }
}

// ---------------------------------------------------------------- touch

#[cfg(test)]
use mpfree_touch_evdev::{parse_input_event, EVENT_SIZE};

/// The touchscreen, read without grabbing it, turned into finger events in
/// screen pixels. M13's reader: seeded from the kernel's slot state at open,
/// resynced after an overflow, and still working (unseeded) if those queries
/// fail. The profile comes from `mpfree_touch::profile::MPC_ONE`, which a test
/// checks against docs/dev/hardware/data/mpc-one-touch-profile.json.
pub struct TouchIn {
    reader: TouchReader<Box<dyn TouchSource + Send>>,
    events: Vec<PointerEvent>,
    pub contacts: u64,
    /// The device node, for the doorbell's own handle (touch_bell.rs).
    path: Option<String>,
}

impl TouchIn {
    pub fn open(path: &str, size: (i32, i32)) -> Result<TouchIn, String> {
        let dev = EvdevFile::open(path).map_err(|e| format!("{path}: {e}"))?;
        dev.check_profile(&MPC_ONE)
            .map_err(|e| format!("{path}: {e}"))?;
        let mut t = TouchIn::from_source(Box::new(dev), size)?;
        t.path = Some(path.to_string());
        Ok(t)
    }

    fn from_source(
        source: Box<dyn TouchSource + Send>,
        size: (i32, i32),
    ) -> Result<TouchIn, String> {
        let reader = TouchReader::new(source, &MPC_ONE, size.0 as u32, size.1 as u32)
            .map_err(|e| e.to_string())?;
        Ok(TouchIn {
            reader,
            events: Vec::new(),
            contacts: 0,
            path: None,
        })
    }

    /// A second handle on the same touchscreen for the doorbell: read-only
    /// and blocking, never grabbed (touch_bell.rs). None for a stand-in.
    fn bell_handle(&self) -> Option<Result<std::fs::File, String>> {
        let path = self.path.as_deref()?;
        Some(std::fs::File::open(path).map_err(|e| format!("{path}: {e}")))
    }

    /// Raw input events read so far.
    pub fn raw_events(&self) -> u64 {
        self.reader.stats().raw_events
    }

    /// The reader's state as one journal line (`touch-reader: ...`).
    pub fn status_line(&self) -> String {
        let s = self.reader.stats();
        let mut line = reader_line(
            s.seeded,
            s.slot_at_open,
            s.fingers_at_open,
            s.resyncs,
            s.resync_fallbacks,
            s.drained_events,
            s.contacts,
            s.raw_events,
        );
        if let Some(e) = self.reader.seed_error() {
            line.push_str(&format!(" (seed: {e})"));
        }
        line
    }

    /// Everything that happened since the last call.
    pub fn read(&mut self) -> io::Result<Vec<FingerEvent>> {
        self.events.clear();
        self.reader.read_into(&mut self.events)?;
        self.contacts = self.reader.stats().contacts;
        Ok(self
            .events
            .iter()
            .map(|p| {
                let phase = match p.phase {
                    Phase::Down => Touch::Down,
                    Phase::Move => Touch::Move,
                    Phase::Up => Touch::Up,
                    Phase::Cancel => Touch::Cancel,
                };
                (
                    p.id.0,
                    phase,
                    p.x.round() as i32,
                    p.y.round() as i32,
                    p.time_us,
                )
            })
            .collect())
    }
}

/// A screen finger's phase as the touch crate names it.
fn touch_phase(t: Touch) -> Phase {
    match t {
        Touch::Down => Phase::Down,
        Touch::Move => Phase::Move,
        Touch::Up => Phase::Up,
        Touch::Cancel => Phase::Cancel,
    }
}

/// One finger event in screen pixels: (finger, phase, x, y, the kernel's
/// timestamp in microseconds).
pub type FingerEvent = (u64, Touch, i32, i32, u64);

/// Opens the touchscreen for the screen thread (tried again while it fails).
pub type OpenTouch = Box<dyn FnMut() -> Result<TouchIn, String> + Send>;

/// How often the screen thread tries to open the touchscreen while it has
/// none: at power-on the app can start before the ILI2117 driver has probed.
const TOUCH_RETRY: Duration = Duration::from_secs(1);
/// While raw touch events keep arriving, the reader line is logged this often,
/// so a round's journal shows which layer touches stop at.
const TOUCH_STATUS_EVERY: Duration = Duration::from_secs(30);
/// Debug mode: how often the screen thread reads /proc, and how often the
/// numbers go to the log.
const PERF_EVERY: Duration = Duration::from_secs(2);
const PERF_LINE_EVERY: Duration = Duration::from_secs(30);
/// The Processes page reads /proc this often while it shows.
const PROCS_EVERY: Duration = Duration::from_secs(1);
/// The CPU by engine page reads the audio loop's numbers this often.
const ENGINES_EVERY: Duration = Duration::from_millis(250);

// ---------------------------------------------------------------- the screen thread

/// What the screen thread did, for the step's report.
#[derive(Clone, Debug, Default)]
pub struct ScreenReport {
    pub frames: u64,
    pub mean_ms: f64,
    pub max_ms: f64,
    pub touches: u64,
    pub error: Option<String>,
    /// Passes of the screen loop, the pages' ticks and the seconds those
    /// ticks moved the pages' clocks on (the same as the time the thread
    /// ran, when the clock keeps time).
    pub wakes: u64,
    pub ticks: u64,
    pub ticked_s: f64,
}

/// Run the pad player's screen until `Quit` arrives or the sender goes away.
/// Spawn it before the audio thread raises its own priority, so it stays at
/// normal priority.
///
/// `open_touch` opens the touchscreen. It is tried at once and then every
/// second while there is none (the driver may not have probed yet at power-on,
/// or a read failed), so touch comes back by itself.
///
/// With `gpu`, the GPU draws the screen in a process of its own, and any
/// failure there draws in software again (drawing.rs).
///
/// `bell` is a sender to `rx` for the touchscreen's doorbell
/// (touch_bell.rs), which wakes the thread when the panel sends something;
/// between frames the thread sleeps until then, a message or the next tick
/// (pace.rs). Without it the thread looks at the panel every 4 ms. The
/// bell's thread holds that sender, so with a bell the screen thread stops
/// on `Quit` (which the app always sends before it joins the thread), not
/// when the other senders go.
pub fn spawn(
    mut player: Player,
    mut fb: Option<FbTarget>,
    gpu: Option<crate::drawing::GpuStart>,
    mut open_touch: Option<OpenTouch>,
    bell: Option<SyncSender<ToUi>>,
    rx: Receiver<ToUi>,
    tx: SyncSender<FromUi>,
) -> io::Result<JoinHandle<ScreenReport>> {
    let bell = match bell {
        Some(to_screen) => Some(Doorbell::start(move || {
            !matches!(
                to_screen.try_send(ToUi::TouchBell),
                Err(TrySendError::Disconnected(_))
            )
        })?),
        None => None,
    };
    thread::Builder::new()
        .name("mpfree-screen".into())
        .spawn(move || {
            let mut touch: Option<TouchIn> = None;
            let mut touch_tried: Option<Instant> = None;
            let mut touch_error = String::new();
            let mut touch_total = 0u64;
            let mut touch_status = (Instant::now(), 0u64);
            let (w, h) = player.ui.size();
            let mut canvas = SoftCanvas::new(w, h);
            let mut drawing =
                crate::drawing::Drawing::start(gpu.as_ref(), &mut player, fb.as_ref());
            let mut report = ScreenReport::default();
            let mut total_ms = 0.0;
            let mut pace = Pace::new(Instant::now());
            // What each frame costs: the Performance page's Screen row every
            // second, and a status line every 30 s (screen_cost.rs).
            let mut cost = crate::screen_cost::ScreenCost::new(Instant::now());
            // Debug mode: /proc and /sys every 2 s, and a perf: line every 30 s.
            let mut perf = crate::perf::Sampler::new();
            let (mut perf_at, mut perf_logged): (Option<Instant>, Option<Instant>) = (None, None);
            let mut ram_at: Option<Instant> = None;
            // The Processes page: /proc once a second, only while it shows (a
            // fresh table each time the page opens, so CPU starts again).
            let mut procs: Option<(mpfree_prof::procs::ProcTable, Instant)> = None;
            // The CPU by engine page: the audio loop's numbers every 250 ms.
            let mut engines_at: Option<Instant> = None;
            let mut quit = false;
            while !quit {
                cost.wake();
                report.wakes += 1;
                // Sleep until the next tick, a message or the doorbell;
                // without the doorbell, look at the touchscreen every 4 ms.
                let mut wait = pace.wait(Instant::now());
                if touch.is_some() && !bell.as_ref().is_some_and(Doorbell::ringing) {
                    wait = wait.min(TOUCH_POLL);
                }
                match rx.recv_timeout(wait) {
                    Ok(m) => quit |= take(&mut player, m),
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => quit = true,
                }
                while let Ok(m) = rx.try_recv() {
                    quit |= take(&mut player, m);
                }
                send_actions(&mut player, &tx);
                if touch.is_none() && touch_tried.is_none_or(|t| t.elapsed() >= TOUCH_RETRY) {
                    if let Some(open) = open_touch.as_mut() {
                        let first = touch_tried.is_none();
                        touch_tried = Some(Instant::now());
                        match open() {
                            Ok(t) => {
                                println!("{}", t.status_line());
                                if let (Some(b), Some(h)) = (bell.as_ref(), t.bell_handle()) {
                                    match h {
                                        Ok(f) => {
                                            b.give(Box::new(f));
                                            println!(
                                                "touch: doorbell on; the screen sleeps until \
                                                 the panel sends something"
                                            );
                                        }
                                        Err(e) => println!(
                                            "touch: no doorbell ({e}); reading the panel \
                                             every 4 ms"
                                        ),
                                    }
                                }
                                player.set_touch_status(&touch_status_text(&t, bell.as_ref()));
                                touch_status = (Instant::now(), t.raw_events());
                                touch = Some(t);
                            }
                            // Say why once, and again only if the reason changes.
                            Err(e) if first || e != touch_error => {
                                println!(
                                    "touchscreen: not available yet ({e}); trying every second"
                                );
                                player.set_touch_status(&format!(
                                    "touchscreen: not available yet ({e})"
                                ));
                                touch_error = e;
                            }
                            Err(_) => {}
                        }
                    }
                }
                if let Some(t) = touch.as_mut() {
                    if let Some(b) = bell.as_ref() {
                        b.answer();
                    }
                    match t.read() {
                        Ok(list) => {
                            for (finger, phase, x, y, time_us) in list {
                                player.set_touch_time(time_us);
                                if let Some(a) = player.touch(finger, phase, x, y) {
                                    player.saw_action(&a);
                                    let _ = tx.try_send(a);
                                }
                            }
                            report.touches = touch_total + t.contacts;
                            if touch_status.0.elapsed() >= TOUCH_STATUS_EVERY {
                                if t.raw_events() != touch_status.1 {
                                    println!("{}", t.status_line());
                                }
                                player.set_touch_status(&touch_status_text(t, bell.as_ref()));
                                touch_status = (Instant::now(), t.raw_events());
                            }
                        }
                        Err(e) => {
                            println!("touch: {e} (reopening)");
                            println!("{}", t.status_line());
                            player.set_touch_status(&format!(
                                "touch: {e} (reopening); {}",
                                t.status_line()
                            ));
                            touch_total += t.contacts;
                            touch = None;
                        }
                    }
                }
                send_actions(&mut player, &tx);
                // The header's RAM chip, every 2 s, whatever shows.
                if ram_at.is_none_or(|t| t.elapsed() >= PERF_EVERY) {
                    ram_at = Some(Instant::now());
                    if let Some(p) = std::fs::read_to_string("/proc/meminfo")
                        .ok()
                        .and_then(|m| crate::perf::ram_used_pct(&m))
                    {
                        player.set_ram(p);
                    }
                }
                if !player.debug_mode() {
                    perf_at = None;
                } else if perf_at.is_none_or(|t| t.elapsed() >= PERF_EVERY) {
                    perf_at = Some(Instant::now());
                    let v = perf.sample();
                    if v.over_s > 0.0 && perf_logged.is_none_or(|t| t.elapsed() >= PERF_LINE_EVERY)
                    {
                        perf_logged = Some(Instant::now());
                        println!("{}", v.line());
                    }
                    player.set_proc(v);
                }
                if player.debug_mode() && player.view() == View::Procs {
                    let due = procs
                        .as_ref()
                        .is_none_or(|(_, at)| at.elapsed() >= PROCS_EVERY);
                    if due {
                        let (table, at) = procs.get_or_insert_with(|| {
                            (mpfree_prof::procs::ProcTable::new(), Instant::now())
                        });
                        *at = Instant::now();
                        let v = table.sample();
                        player.set_procs(&v);
                    }
                } else {
                    procs = None;
                }
                if player.debug_mode() && player.view() == View::Engines {
                    if engines_at.is_none_or(|t| t.elapsed() >= ENGINES_EVERY) {
                        engines_at = Some(Instant::now());
                        player.refresh_engines();
                    }
                } else {
                    engines_at = None;
                }
                drawing.poll(&mut player, fb.as_ref());
                // The pages tick every frame, drawn or not, by the time since
                // the last tick, so their clocks keep time (pace.rs).
                let Some(step) = pace.tick(Instant::now(), quit) else {
                    continue;
                };
                player.tick(step);
                report.ticks += 1;
                report.ticked_s += f64::from(step);
                // What the tick asked for goes now, not a frame later.
                send_actions(&mut player, &tx);
                if !player.ui.needs_paint() {
                    continue;
                }
                let t0 = Instant::now();
                // On the GPU, painting records the frame's list and the
                // "copy" is sending it; in software, the copy to the panel.
                let (t1, rows) = match drawing.frame(&mut player, fb.as_ref()) {
                    Some(painted) => (t0 + painted, 0),
                    None => {
                        let damage = player.ui.paint(&mut canvas);
                        let t1 = Instant::now();
                        (t1, present(&mut fb, &canvas, &damage, &mut report))
                    }
                };
                let t2 = Instant::now();
                pace.drawn(t2);
                let ms = (t2 - t0).as_secs_f64() * 1e3;
                report.frames += 1;
                total_ms += ms;
                report.max_ms = report.max_ms.max(ms);
                cost.frame(player.view(), t1 - t0, t2 - t1, rows);
                if let Some(second) = cost.second(t2) {
                    let (row, tone) = drawing.screen_row(&second);
                    player.set_screen_row(row, tone);
                }
                if let Some(line) = cost.status(t2) {
                    println!("{line}");
                }
            }
            drawing.finish(&mut player, &mut canvas, fb.as_mut());
            report.mean_ms = if report.frames > 0 {
                total_ms / report.frames as f64
            } else {
                0.0
            };
            report
        })
}

/// The touch status on the Touch lab: the reader's line, and how the screen
/// thread learns of touches (the doorbell, or a look every 4 ms).
fn touch_status_text(t: &TouchIn, bell: Option<&Doorbell>) -> String {
    match bell {
        Some(b) => format!("{}; {}", t.status_line(), b.note()),
        None => t.status_line(),
    }
}

/// Copies a painted frame's changed rectangles to the panel; says how many
/// native rows it wrote. The one place a frame leaves the screen thread
/// (where a GPU drawing process would take it instead). A screen that
/// fails stops drawing, and says why once.
fn present(
    fb: &mut Option<FbTarget>,
    canvas: &SoftCanvas,
    damage: &[mpfree_ui::Rect],
    report: &mut ScreenReport,
) -> usize {
    let Some(f) = fb.as_mut() else {
        return 0;
    };
    match f.present(canvas, damage) {
        Ok(rows) => rows,
        Err(e) => {
            println!("screen: {e} (drawing stopped)");
            report.error = Some(e);
            *fb = None;
            0
        }
    }
}

/// Hands the screen's actions to the audio thread. A kit view that finds no
/// room goes back to the Sample page, which builds it again later.
fn send_actions(player: &mut Player, tx: &SyncSender<FromUi>) {
    for a in player.take_actions() {
        match tx.try_send(a) {
            Ok(()) => {}
            Err(TrySendError::Full(a) | TrySendError::Disconnected(a)) => player.unsent(a),
        }
    }
}

/// Print a line message and hand any message to the player. True on Quit.
fn take(player: &mut Player, m: ToUi) -> bool {
    match &m {
        ToUi::Line(l) | ToUi::Midi(l) => {
            println!("{l}");
            let _ = io::stdout().flush();
        }
        ToUi::Quit => return true,
        ToUi::TouchBell => return false,
        _ => {}
    }
    player.apply(m);
    false
}

// ---------------------------------------------------------------- previews

/// Sample kits for previews and tests (made-up names, no Akai content).
pub fn sample_kits() -> Vec<KitView> {
    let names = [
        "Kick",
        "Snare",
        "Closed Hat",
        "Open Hat",
        "Clap",
        "Rim",
        "Tom Low",
        "Tom Hi",
        "Crash",
        "Ride",
        "Shaker",
        "Perc 1",
        "Perc 2",
        "",
        "Sub Hit",
        "",
    ];
    let kit = |name: &str, colour: [u8; 3], n: usize| KitView {
        name: name.to_string(),
        pads: names
            .iter()
            .take(n)
            .map(|s| {
                if s.is_empty() {
                    None
                } else {
                    Some(s.to_string())
                }
            })
            .chain(std::iter::repeat(None))
            .take(16)
            .collect(),
        colour,
        chip: None,
        synth: None,
    };
    // Kit 2 is the Play mockups' kit (F to I), so the previews show it.
    let mut boom_bap = kit("Boom Bap", [0x00, 0x2A, 0x3F], 16);
    boom_bap.pads = [
        "Kick", "Kick 2", "Snare", "Rim", "Hat", "Hat op", "Shaker", "Clap", "Conga", "Tom",
        "Perc", "Crash", "Bass", "Chop 1", "Chop 2", "Vox",
    ]
    .iter()
    .map(|s| Some(s.to_string()))
    .collect();
    vec![
        kit("Classic House", [0x3F, 0x22, 0x00], 16),
        boom_bap,
        kit("Trap", [0x2C, 0x00, 0x3F], 16),
        kit("Lo-Fi", [0x04, 0x3F, 0x00], 9),
        kit("Techno", [0x3F, 0x00, 0x00], 16),
        kit("Afrobeat", [0x3B, 0x3B, 0x09], 14),
    ]
}

/// The pad player in a busy moment, for screenshots: a few pads lit, a
/// level, timing, MIDI lines and a few system log lines.
pub fn sample_player(fonts: Fonts) -> Player {
    let mut p = Player::new(fonts, SCREEN, false, None);
    let synth = crate::synth::Synth::new(44_100);
    let mut kits = sample_kits();
    kits.push(synth.kit_view());
    p.apply(ToUi::Kits(kits));
    p.apply(ToUi::Synth(synth.state()));
    p.apply(ToUi::Session("Akai session".into(), Tone::Good));
    p.apply(ToUi::Kit(0));
    p.apply(ToUi::Level { db: -18.0 });
    p.apply(ToUi::Status(Status {
        shift: false,
        pad_level: PadLevel::Full,
        repeat: false,
        latched: false,
        rate: "1/16",
        bpm: 120.0,
        mute_keys: MuteKeys::Off,
        erase: false,
    }));
    for (pad, v) in [(0usize, 112u8), (5, 87), (10, 64)] {
        p.apply(ToUi::PadDown { pad, velocity: v });
    }
    p.apply(ToUi::PadUp(10));
    if let Some(g) =
        p.ui.get_mut::<MiniPads>(play_page::PADS)
            .map(|m| &mut m.grid)
    {
        g.pads[10].level = 0.5;
    }
    p.apply(ToUi::Timing(Timing {
        hits: 42,
        periods: 10_335,
        underruns: 0,
        voices: 3,
        peak_db: -9.0,
        last_ms: 7.9,
        min_ms: 6.2,
        mean_ms: 7.8,
        max_ms: 9.6,
        seconds_left: None,
        periods_queued: 3,
        audio: AudioNumbers {
            render_mean_us: 310.0,
            render_max_us: 540.0,
            synth_mean_us: 0.0,
            synth_max_us: 0.0,
            worst_us: 905.0,
            steals: 0,
            dropped: 0,
            peak_grains: 0,
        },
    }));
    p.set_screen_row(
        "30 fps · paint 1.2 ms · copy 3.1 ms · slowest 6.4 ms".into(),
        Tone::Dim,
    );
    for l in ["+0.012 s -> Public: F0 7E 00 06 01 F7 = identity request",
              "+0.013 s -> Public: F0 47 00 46 52 00 04 15 21 6D 43 F7 = session ping",
              "+0.020 s Private: F0 47 7F 46 53 00 05 15 21 6D 43 02 F7 = session reply, token 15 21 6D 43, accepted",
              "+4.211 s Private: 99 25 70 = pad 1 down, velocity 112",
              "+4.302 s Private: 89 25 00 = pad 1 up",
              "+5.018 s Private: B0 64 01 = data wheel +1",
              "+5.640 s Private: B0 10 02 = Q-Link 1 turned +2",
              "+6.101 s Private: 99 2E 57 = pad 6 down, velocity 87",
              "+6.900 s Private: 90 27 7F = button 39 pressed"] {
        p.apply(ToUi::Midi(l.into()));
    }
    for l in [
        "pad player: 6 kits, 81 sounds",
        "session accepted; sending Akai's setup exactly as the app did",
        "opening /dev/snd/pcmC1D0p (ACVA)",
        "SCHED_FIFO priority 70",
        "kit 2: Boom Bap (12 pads)",
        "kit 1: Classic House (16 pads)",
        "pad level: Full Level (every hit 127)",
    ] {
        p.apply(ToUi::Line(l.into()));
    }
    for l in ["[    2.104] systemd[1]: Started MPFree test boot: copy logs to the USB stick's log partition.",
              "[    2.230] systemd[1]: Starting MPFree test boot: the pad player...",
              "[    3.871] mpfree[211]: loading drum kits: 8 kits, 39.0 MB",
              "[    4.402] kernel: rk_gmac-dwmac ff290000.ethernet eth0: Link is Up - 1Gbps/Full",
              "[    5.990] mpfree[211]: session accepted; sending Akai's setup exactly as the app did",
              "[    7.315] mpfree[211]: SCHED_FIFO priority 70"] {
        p.apply(ToUi::Console(l.into()));
    }
    p.apply(ToUi::Show(View::Play));
    p
}

/// A real loop for the Play page's screenshots, as mockups F and H show
/// one: four bars at 92 BPM recorded by the looper on a transport, a drum
/// pass, then a bass and a chop overdubbed (three layers), playing on.
/// Recorded once and kept (it takes a moment).
fn recorded_loop() -> &'static (Arc<LoopWave>, LooperView) {
    use crate::looping::{LoopHost, PERIOD};
    use mpfree_looper::Command as L;
    use mpfree_transport::{Block, Command as T, Transport};
    use std::f32::consts::TAU;
    use std::sync::OnceLock;
    static LOOP: OnceLock<(Arc<LoopWave>, LooperView)> = OnceLock::new();
    LOOP.get_or_init(|| {
        const RATE: u32 = 44_100;
        const BPM: f64 = 92.0;
        let beat = (f64::from(RATE) * 60.0 / BPM) as u64;
        let bar = 4 * beat;
        let len = 4 * bar;
        // Room for the three layers (each the loop's length).
        let mut h = LoopHost::new(RATE, 40);
        let mut t = Transport::new(RATE, BPM).expect("a transport");
        let mut b = Block::new();
        h.command(L::SetBars(Some(4)));
        h.command(L::SetLatency(0));
        h.command(L::Record);
        t.command(0, T::Play);
        let mut wave = None;
        let noise = |n: u64| ((n.wrapping_mul(2_654_435_761) >> 7) % 2000) as f32 / 1000.0 - 1.0;
        // Three passes and an eighth of the fourth.
        let periods = (3 * len + len / 12) / PERIOD as u64;
        for i in 0..periods {
            let start = i * PERIOD as u64;
            // Overdub the bass over the second pass and the chop over the
            // third (a press each to start and to stop, a moment into the
            // pass, each in a period of its own).
            let p = PERIOD as u64;
            for at in [len + p, 2 * len + p, 2 * len + 8 * p, 3 * len + p] {
                if (start..start + p).contains(&at) {
                    h.command(L::Overdub);
                }
            }
            let _ = t.process(PERIOD as u32, &mut b);
            let (mut l, mut r) = ([0.0f32; PERIOD], [0.0f32; PERIOD]);
            for f in 0..PERIOD {
                let n = start + f as u64;
                let pass = n / len;
                let in_beat = (n % beat) as f32 / RATE as f32;
                let in_half = (n % (beat / 2)) as f32 / RATE as f32;
                let beat_no = (n % bar) / beat;
                let v = match pass {
                    // A kick on 1 and 3, a snare on 2 and 4, hats between.
                    0 => {
                        let kick = if beat_no.is_multiple_of(2) {
                            (TAU * (48.0 + 80.0 * (-in_beat * 30.0).exp()) * in_beat).sin()
                                * (-in_beat * 8.0).exp()
                        } else {
                            noise(n) * (-in_beat * 18.0).exp() * 0.6
                        };
                        let hat = noise(n + 7) * (-in_half * 70.0).exp() * 0.15;
                        0.5 * kick + hat
                    }
                    // A bass line, a note each half beat.
                    1 => {
                        let note = [55.0, 55.0, 65.4, 49.0][(beat_no % 4) as usize];
                        0.24 * (TAU * note * in_half).sin()
                            * (in_half * 40.0).min(1.0)
                            * (-in_half * 2.0).exp()
                    }
                    // A chop phrase in bars 2 and 4.
                    2 => {
                        let bar_no = (n % len) / bar;
                        let in_bar = (n % bar) as f32 / beat as f32;
                        if bar_no % 2 == 1 && (1.5..3.6).contains(&in_bar) {
                            0.22 * (TAU * 330.0 * in_beat).sin()
                                * (0.6 + 0.4 * ((in_bar - 1.5) * 9.0).sin().abs())
                        } else {
                            0.0
                        }
                    }
                    _ => 0.0,
                };
                let v = v.clamp(-1.0, 1.0);
                (l[f], r[f]) = (v, v);
            }
            h.take_pads(&l, &r);
            h.period(&b, &mut l, &mut r);
            h.changes(String::new);
            if let Some(w) = h.wave_step(i, 64) {
                wave = Some(w);
            }
        }
        let wave = wave.unwrap_or_else(|| Arc::new(LoopWave::default()));
        (wave, h.view(BPM))
    })
}

/// The recorded loop on the Play page and the Looper tab.
pub fn sample_loop(p: &mut Player) {
    let (wave, view) = recorded_loop();
    p.apply(ToUi::Looper(Box::new(view.clone())));
    // Named as they would be from the pads played into them.
    p.looper_page.set_names(&["Drums", "Bass", "Chop"]);
    p.apply(ToUi::LoopWave(Arc::clone(wave)));
}

/// A beat programmed on the Play page's pattern with the real sequencer
/// (mockup F's: kick, snare, hats, a clap, perc, bass and a chop), playing
/// at 92 BPM until `sixteenths` into bar 9, and the transport.
pub fn sample_pattern(p: &mut Player, sixteenths: i64) {
    use crate::time::{Pads, StepAction, TimeHost, STEP_TICKS};
    struct Quiet;
    impl Pads for Quiet {
        fn hit_at(&mut self, _: usize, _: u8, _: u32) -> bool {
            true
        }
        fn release_at(&mut self, _: usize, _: u32) {}
        fn stop_all(&mut self) {}
        fn pressure_at(&mut self, _: usize, _: u8, _: u32) {}
    }
    let Ok(mut h) = TimeHost::new(44_100, 92.0) else {
        return;
    };
    let rows: [(u8, &str); 8] = [
        (0, "X... ..x. X.x. ...."),
        (2, ".... X... .... X..o"),
        (4, "x.xo x.xo x.xo x.Xo"),
        (5, ".... ...x .... ...."),
        (7, ".... X... .... X..."),
        (10, "..o. .o.. ..o. o..o"),
        (12, "X..x ..X. X... ..x."),
        (13, "X... .... ..x. ...."),
    ];
    for (pad, steps) in rows {
        for (i, c) in steps.chars().filter(|c| !c.is_whitespace()).enumerate() {
            let velocity = match c {
                'X' => 120,
                'x' => 84,
                'o' => 52,
                _ => continue,
            };
            let _ = h.step_action(StepAction::Tap {
                pad,
                step: i,
                velocity,
            });
        }
    }
    // The mockups' step 7 of the kick (velocity 96, 1/32 long, 4 ms
    // early) and their swing (56 %).
    for change in [
        mpfree_seq::steps::Change::Velocity(96),
        mpfree_seq::steps::Change::Length(STEP_TICKS / 2),
        mpfree_seq::steps::Change::Nudge(-6),
    ] {
        let _ = h.step_action(StepAction::Change {
            pad: 0,
            step: 6,
            change,
        });
    }
    h.command(mpfree_transport::Command::SetSwing(
        mpfree_transport::time::Swing {
            grid: 240,
            amount: 56,
        },
    ));
    h.command(mpfree_transport::Command::Play);
    let target = 8 * 3840 + sixteenths * STEP_TICKS;
    let mut q = Quiet;
    while h.info().position < target {
        h.advance(128);
        h.period(128, &mut q);
    }
    p.apply(ToUi::Time(Box::new(h.state())));
    p.apply(ToUi::Steps(Box::new(h.steps())));
}

/// Hits played on the pads over the last two bars, a little off the grid
/// as hands are (mostly early, as mockup I), for Beats' timing strip.
pub fn sample_hits(p: &mut Player) {
    let t = p.play.transport;
    let bar = t.meter.bar_ticks() as f64;
    let now = t.position as f64;
    let hash = |i: u32| {
        let x = i.wrapping_mul(2_654_435_761).rotate_left(13) ^ 0x9E37_79B9;
        (x % 1000) as f64 / 1000.0
    };
    let lanes: [(u8, &str); 4] = [
        (0, "X... ..x. X.x. ...."),
        (2, ".... X... .... X..."),
        (4, "x.x. x.x. x.x. x.x."),
        (13, ".... ..x. ..x. ...."),
    ];
    let mut k = 0u32;
    let start = (now / (2.0 * bar)).floor() * 2.0 * bar - 2.0 * bar;
    for b in 0..4 {
        for (pad, steps) in lanes {
            for (i, c) in steps.chars().filter(|c| !c.is_whitespace()).enumerate() {
                if c == '.' {
                    continue;
                }
                k += 1;
                let grid = start + b as f64 * bar + i as f64 * 240.0;
                // About 6 ms early on average, spread ±9 ms, at 92 BPM.
                let ms = -6.0 + (hash(k) - 0.5) * 18.0;
                let tick = grid + ms * 92.0 * 960.0 / 60_000.0;
                if tick > now || tick < now - 2.0 * bar {
                    continue;
                }
                let velocity = if c == 'X' { 112 } else { 70 } + (hash(k + 99) * 14.0) as u8;
                p.apply(ToUi::Hit(HitInfo {
                    pad,
                    velocity,
                    tick,
                    bpm: 92.0,
                    bar_ticks: bar as i64,
                }));
            }
        }
    }
}

/// The Play page as mockups F to I show it, with the app's real state
/// behind it: kit 2, the recorded loop, the programmed beat playing, the
/// master near full with the limiter just catching, a throw held, and the
/// view's own touches (a step picked in Beats, the hits in Drum).
pub fn sample_play(fonts: Fonts, v: PlayView) -> Player {
    let mut p = sample_player(fonts);
    p.apply(ToUi::Kit(1));
    for pad in 0..16 {
        p.apply(ToUi::PadUp(pad));
    }
    if let Some(g) =
        p.ui.get_mut::<MiniPads>(play_page::PADS)
            .map(|m| &mut m.grid)
    {
        for pad in g.pads.iter_mut() {
            pad.held = false;
            pad.level = 0.0;
        }
        g.pads[4].level = 0.9;
        g.pads[13].level = 0.55;
    }
    p.apply(ToUi::Status(Status {
        shift: false,
        pad_level: PadLevel::Off,
        repeat: false,
        latched: false,
        rate: "1/16",
        bpm: 92.0,
        mute_keys: MuteKeys::Off,
        erase: false,
    }));
    sample_loop(&mut p);
    if v == PlayView::Loop {
        // Mockup H: the chop layer muted from its speaker.
        let mut view = recorded_loop().1.clone();
        if let Some(l) = view.layers[2].as_mut() {
            l.muted = true;
            l.audible = false;
        }
        p.apply(ToUi::Looper(Box::new(view)));
    }
    let at = match v {
        PlayView::All => 1,
        PlayView::Beats => 6,
        PlayView::Loop => 8,
    };
    sample_pattern(&mut p, at);
    // The kits' projects (Boom Bap's pads with sounds behind them), so the
    // Tune and Decay Q-Links read the pads' own settings.
    p.apply(ToUi::SampleKits {
        projects: vec![mpfree_sampler::model::Project::new(), boom_bap_project()],
        rate: 44_100,
    });
    p.play.last_pad = 0;
    p.apply(ToUi::Level { db: -1.5 });
    for _ in 0..3 {
        p.apply(ToUi::Meter {
            peak_db: [-4.5, -6.0],
            limit_db: 1.2,
        });
    }
    let mut levels = p.play.levels;
    levels.pad_mutes = 1 << 7;
    levels.drums_filter = -16.0;
    levels.loop_db = -3.0;
    levels.line_db = 6.0;
    levels.loop_filter = -22.0;
    levels.drums_send_db = 20.0 * 0.22f32.log10();
    p.apply(ToUi::PlayLevels(levels));
    let shared = Arc::new(crate::engines::EngineShared::default());
    let mut probes = crate::engines::EngineProbes::with_shared(Arc::clone(&shared));
    for _ in 0..200 {
        probes.add(crate::engines::PERIOD, Duration::from_micros(610));
    }
    probes.publish();
    p.apply(ToUi::Engines(shared));
    p.show_play_view(v);
    match v {
        PlayView::All => {
            p.play.throw(2, true);
            p.play.xy.touch(0.68, 0.62);
        }
        PlayView::Beats => {
            // Your timing's strip, from two bars of hits.
            sample_hits(&mut p);
            p.play.selected = Some((0, 6));
            p.play.throw(2, true);
            p.apply(ToUi::QLinkTouch(0, true));
        }
        PlayView::Loop => {
            p.play.throw(2, true);
            p.play.xy.touch(0.68, 0.62);
        }
    }
    if let Some(xy) = p.ui.get_mut::<XyPad>(play_page::XY) {
        (xy.x, xy.y) = p.play.xy.position();
        xy.on = p.play.xy.touched();
        if xy.on {
            // The finger's way up to where it is, as the mockups draw it.
            let (x, y) = p.play.xy.position();
            let trail: Vec<(f32, f32)> = (0..10)
                .map(|k| {
                    let f = k as f32 / 10.0;
                    (0.3 + (x - 0.3) * f, 0.12 + (y - 0.12) * f * f)
                })
                .collect();
            xy.set_trail(&trail);
        }
    }
    p.refresh_play();
    p.refresh_chips();
    p.refresh_qlinks();
    p
}

/// Boom Bap's sampler project for the previews: a short made-up sound on
/// each of its 16 pads, with the mockups' settings on Kick (-2 st) and Hat
/// (+2 st, 180 ms decay).
fn boom_bap_project() -> mpfree_sampler::model::Project {
    use mpfree_sampler::model::{Layer, Pad, Project, Region, Source};
    let mut p = Project::new();
    let names = [
        "Kick", "Kick 2", "Snare", "Rim", "Hat", "Hat op", "Shaker", "Clap", "Conga", "Tom",
        "Perc", "Crash", "Bass", "Chop 1", "Chop 2", "Vox",
    ];
    for (i, name) in names.iter().enumerate() {
        let data: Vec<f32> = (0..2_000)
            .map(|n| (n as f32 * 0.05 * (i + 1) as f32).sin() * (1.0 - n as f32 / 2_000.0))
            .collect();
        let source = Source::new(name, 44_100, 1, data).expect("a sound");
        let region = Region::whole(&source);
        p.add_source(source);
        let mut pad = Pad::new(name, Layer::new(region));
        match i {
            0 => pad.tune_cents = -200,
            4 => {
                pad.tune_cents = 200;
                pad.amp.sustain = 0.0;
                pad.amp.decay_ms = 180.0;
            }
            _ => {}
        }
        p.set_pad(i, Some(pad), None).expect("a pad");
    }
    p
}

/// Hands the app's effects to the screen as the audio thread does (lights,
/// sounds and kit changes are left out), for previews and tests.
/// The Touch page's two screens, built in their own frame: `sample_screens`
/// already holds many players on its stack.
#[inline(never)]
fn push_touch_dev_samples(out: &mut Vec<(&'static str, Player)>, fonts: &impl Fn() -> Fonts) {
    out.push(("debug-touch", touch_dev_sample(fonts(), false)));
    out.push(("debug-touch-targets", touch_dev_sample(fonts(), true)));
}

/// The Touch page with made-up measurements, on a clock of its own (so the
/// picture is the same every time).
#[inline(never)]
pub fn touch_dev_sample(fonts: Fonts, targets: bool) -> Player {
    use std::sync::atomic::{AtomicU64, Ordering};
    let mut p = sample_player(fonts);
    p.apply(ToUi::Settings(Settings {
        debug_mode: true,
        ..Settings::default()
    }));
    let mut saved = Tuning::default();
    saved.set(Param::OffsetY, 2.0);
    p.apply(ToUi::TouchTuning(saved));
    p.apply(ToUi::Show(View::Touch));
    let now = Arc::new(AtomicU64::new(1_000_000_000));
    let c = now.clone();
    if let Some(t) = p.ui.get_mut::<TouchTest>(touch_dev::TEST) {
        t.set_clock(Box::new(move || c.load(Ordering::SeqCst)));
    }
    let at = |p: &mut Player, f: u64, ph: Touch, x: i32, y: i32, dt: u64| {
        let t = now.fetch_add(dt, Ordering::SeqCst) + dt;
        p.set_touch_time(t);
        let _ = p.touch(f, ph, x, y);
    };
    let area =
        p.ui.rect_of(touch_dev::TEST)
            .unwrap_or(mpfree_ui::Rect::new(0, 112, 760, 662));
    let (bar, body) = TouchTest::body(area);
    let cells = TouchTest::bar_cells(bar);
    let tap_bar = |p: &mut Player, i: usize| {
        let c = cells[i];
        at(p, 99, Touch::Down, c.x + c.w / 2, c.y + c.h / 2, 300_000);
        at(p, 99, Touch::Up, c.x + c.w / 2, c.y + c.h / 2, 90_000);
    };
    if targets {
        tap_bar(&mut p, 1);
        let area = p.ui.rect_of(touch_dev::TEST).unwrap_or(area);
        let spots = TouchTest::targets(TouchTest::body(area).1);
        for (i, &(x, y)) in spots.iter().take(4).enumerate() {
            let (x, y) = (x + 4 - i as i32 * 2, y - 5 + i as i32);
            at(&mut p, 10 + i as u64, Touch::Down, x, y, 700_000);
            at(&mut p, 10 + i as u64, Touch::Up, x, y, 90_000);
        }
        return p;
    }
    for i in 0..10i32 {
        let (x, y) = (body.x + 80 + (i % 5) * 120, body.y + 120 + (i / 5) * 100);
        at(&mut p, 1, Touch::Down, x, y, 400_000);
        at(&mut p, 1, Touch::Move, x + i % 3, y + 1, 40_000);
        at(
            &mut p,
            1,
            Touch::Up,
            x + i % 3,
            y + 1,
            50_000 + i as u64 * 9_000,
        );
    }
    for i in 0..3i32 {
        let (x, y) = (body.x + 200 + i * 150, body.y + 360);
        at(&mut p, 2, Touch::Down, x, y, 400_000);
        for k in 1..=8 {
            at(
                &mut p,
                2,
                Touch::Move,
                x + [0, 2, 0, -2][k % 4],
                y + [0, 1, 0, -1][(k + 1) % 4],
                110_000,
            );
        }
        at(&mut p, 2, Touch::Up, x, y, 20_000);
    }
    tap_bar(&mut p, 2);
    for i in 0..4i32 {
        let y = body.y + 160 + i * 40;
        at(&mut p, 3, Touch::Down, body.x + 120, y, 500_000);
        for k in 1..=18 {
            at(
                &mut p,
                3,
                Touch::Move,
                body.x + 120 + k * 24,
                y + k / 6,
                9_000,
            );
        }
        at(&mut p, 3, Touch::Up, body.x + 120 + 18 * 24, y + 3, 2_000);
    }
    tap_bar(&mut p, 0);
    // A finger down, dragged in an arc.
    at(&mut p, 4, Touch::Down, body.x + 180, body.y + 300, 400_000);
    for k in 1..=24 {
        let a = k as f32 * 0.09;
        let (x, y) = (
            body.x + 180 + (a.sin() * 220.0) as i32,
            body.y + 300 - ((1.0 - a.cos()) * 140.0) as i32,
        );
        at(&mut p, 4, Touch::Move, x, y, 9_000);
    }
    p.apply(ToUi::TouchWheel(5));
    p
}

pub fn apply_effects(p: &mut Player, fx: Vec<crate::app::Effect>) {
    use crate::app::Effect;
    for e in fx {
        match e {
            Effect::Show(v) => p.apply(ToUi::Show(v)),
            Effect::Menu(m) => p.apply(ToUi::Menu(m)),
            Effect::Guide(g) => p.apply(ToUi::Guide(g)),
            Effect::Status(s) => p.apply(ToUi::Status(s)),
            Effect::Settings(s) => p.apply(ToUi::Settings(s)),
            Effect::Arp(v) => p.apply(ToUi::Arp(Box::new(v))),
            Effect::SixteenLevels => p.apply(ToUi::SixteenLevels),
            Effect::LevelsWheel(d) => p.apply(ToUi::LevelsWheel(d)),
            Effect::Patterns(l) => p.apply(ToUi::Patterns(l)),
            Effect::Workspace(w) => p.apply(ToUi::Workspace(Box::new(w))),
            Effect::Recorded(pat, next) => p.apply(ToUi::Recorded(Box::new(pat), next)),
            Effect::WorkspaceWheel(d) => p.apply(ToUi::WorkspaceWheel(d)),
            Effect::ClearLogs => p.apply(ToUi::ClearLogs),
            Effect::ClearConsole => p.apply(ToUi::ClearConsole),
            Effect::TouchLab => p.apply(ToUi::TouchLab),
            Effect::CalibrateTouch => p.apply(ToUi::CalibrateTouch),
            Effect::TouchWheel(d) => p.apply(ToUi::TouchWheel(d)),
            Effect::TouchStep(d) => p.apply(ToUi::TouchStep(d)),
            Effect::Note(l) => p.apply(ToUi::Line(l)),
            Effect::Tell(l) => {
                p.apply(ToUi::Line(l.clone()));
                p.apply(ToUi::Tell(l));
            }
            Effect::Undo { redo } => p.apply(ToUi::Undo(redo)),
            Effect::UndoButton(down) => p.apply(ToUi::UndoButton(down)),
            Effect::PlayRec { overdub } => p.apply(ToUi::PlayRec(overdub)),
            Effect::ClipsWheel { delta, scroll } => p.apply(ToUi::ClipsWheel { delta, scroll }),
            Effect::Map(m) => p.apply(ToUi::Map(Box::new(m))),
            Effect::SynthPage(d) => p.apply(ToUi::SynthPage(d)),
            Effect::SampleWheel(d) => p.apply(ToUi::SampleWheel(d)),
            Effect::Rows(k) => p.apply(ToUi::Rows(k)),
            Effect::SamplePad(d) => p.apply(ToUi::SamplePad(d)),
            Effect::SampleUndo { redo } => p.apply(ToUi::SampleUndo(redo)),
            Effect::PlayView(v) => p.apply(ToUi::PlayView(v)),
            Effect::SampleEditor(e) => p.apply(ToUi::SampleEditor(e)),
            Effect::Browse(k) => p.apply(ToUi::Browse(k)),
            Effect::SaveProject => p.ask_save(),
            Effect::Light(..)
            | Effect::StopSounds
            | Effect::Kit(_)
            | Effect::Audio(_)
            | Effect::Quit
            | Effect::Time(_)
            | Effect::Octave(_)
            | Effect::Bank { .. }
            | Effect::Looper(_)
            | Effect::SavePatterns(_)
            | Effect::SynthPads(_)
            | Effect::TrackSound { .. }
            | Effect::ProgEdit
            | Effect::Metronome { .. }
            | Effect::Fill(_)
            | Effect::Mixer(_) => {}
            Effect::MixerWheel(d) => p.apply(if p.io_showing() {
                ToUi::IoInput(mpfree_mixer_ui::IoInput::Wheel(d))
            } else {
                ToUi::MixerQLink(0, d)
            }),
            Effect::MixerStep(d) => {
                if p.io_showing() {
                    p.apply(ToUi::IoInput(mpfree_mixer_ui::IoInput::Step(d)));
                }
            }
            Effect::ChainTurn { knob, delta } => p.apply(ToUi::ChainQLink(knob, delta)),
            Effect::ChainWheel(d) => p.apply(ToUi::ChainWheel(d)),
            Effect::ChainPush => p.apply(ToUi::ChainPush),
            Effect::QLinkBank => p.apply(ToUi::QLinkBank),
        }
    }
}

/// The app's arp, run for `frames` frames, and its view on the screen.
fn run_arp(p: &mut Player, app: &mut crate::app::App, frames: u32) {
    let mut ev = mpfree_arp::Events::new();
    for _ in 0..frames / 128 {
        app.process(128, &mut ev);
    }
    let fx: Vec<_> = app.arp_poll().into_iter().collect();
    apply_effects(p, fx);
}

/// The sample player with no pad held or fading, for the arp's screens.
fn quiet_player(fonts: Fonts) -> Player {
    let mut p = sample_player(fonts);
    if let Some(g) =
        p.ui.get_mut::<MiniPads>(play_page::PADS)
            .map(|m| &mut m.grid)
    {
        for pad in g.pads.iter_mut() {
            pad.held = false;
            pad.level = 0.0;
        }
    }
    p
}

/// The Arrange page with the real time engine behind it: tapping Track 1's
/// first slot recorded a two-bar beat from the pads (kick, snare, hats),
/// REC punched out at bar 3, the clip plays on at 3.2, and Track 2 is
/// armed for the next one.
pub fn sample_clips(fonts: Fonts) -> Result<Player, String> {
    use crate::time::{Pads, TimeHost};
    use mpfree_play::Event;
    struct Silent;
    impl Pads for Silent {
        fn hit_at(&mut self, _pad: usize, _velocity: u8, _offset: u32) -> bool {
            true
        }
        fn release_at(&mut self, _pad: usize, _offset: u32) {}
        fn stop_all(&mut self) {}
        fn pressure_at(&mut self, _pad: usize, _value: u8, _offset: u32) {}
    }
    const RATE: u32 = 44_100;
    const PERIOD: u32 = 128;
    const BAR: i64 = 4 * 960;
    let mut host = TimeHost::new(RATE, 120.0)?;
    host.act(TimeAction::NewClip { track: 0, slot: 0 })?;
    // (song tick, pad, velocity): kick on 1 and 3 (and the "and" of 3),
    // snare on 2 and 4, closed hat on every eighth; twice.
    let mut hits = Vec::new();
    for bar in 0..2 {
        for (tick, pad, velocity) in [
            (0, 0, 118),
            (1920, 0, 104),
            (2400, 0, 90),
            (960, 1, 112),
            (2880, 1, 116),
        ] {
            hits.push((bar * BAR + tick, pad, velocity));
        }
        for eighth in 0..8 {
            hits.push((
                bar * BAR + eighth * 480,
                2,
                if eighth % 2 == 0 { 96 } else { 70 },
            ));
        }
    }
    let mut notes: Vec<(i64, Event)> = hits
        .iter()
        .flat_map(|&(t, pad, velocity)| {
            [
                (
                    t,
                    Event::NoteOn {
                        note: pad,
                        velocity,
                    },
                ),
                (t + 120, Event::NoteOff { note: pad }),
            ]
        })
        .collect();
    notes.sort_by_key(|n| n.0);
    let ticks_per_frame = 960.0 * 120.0 / 60.0 / f64::from(RATE);
    let mut next = 0;
    let mut run_to = |host: &mut TimeHost, end: i64| -> Result<(), String> {
        for _ in 0..100_000 {
            let at = host.info().position;
            if at >= end {
                return Ok(());
            }
            let period_end = at as f64 + f64::from(PERIOD) * ticks_per_frame;
            while next < notes.len() && (notes[next].0 as f64) < period_end {
                let offset = ((notes[next].0 - at).max(0) as f64 / ticks_per_frame) as u32;
                host.pad(offset.min(PERIOD - 1), notes[next].1);
                next += 1;
            }
            host.period(PERIOD, &mut Silent);
        }
        Err(format!("the sample never reached tick {end}"))
    };
    // The first period starts recording at bar 1.
    run_to(&mut host, 2 * BAR - 120)?;
    host.act(TimeAction::Transport(mpfree_transport::Command::Record))?;
    run_to(&mut host, 2 * BAR + 960 + 240)?;
    host.act(TimeAction::Arm(Some(1)))?;
    run_to(&mut host, 2 * BAR + 960 + 300)?;
    let mut p = quiet_player(fonts);
    p.apply(ToUi::Time(Box::new(host.state())));
    p.apply(ToUi::Show(View::Arrange));
    Ok(p)
}

/// The pad player on the Sample page: kit 1's pad 5 holds the sampler UI's
/// made-up drum loop ("Break 92", trimmed, looping, with fades), and a voice
/// is playing it. Kit 1's other pads are empty here.
/// The Pads tab on `view` (of `pads_tab::VIEWS`), as a tap on its tab
/// and its switch would show it.
pub fn open_pads(p: &mut Player, view: usize) {
    p.apply(ToUi::PadsView(view));
    p.apply(ToUi::Show(View::Pads));
}

/// The Sample tab on `editor`, as SAMPLE EDIT or a tap on its bar would.
pub fn open_sample(p: &mut Player, editor: sample_page::Editor) {
    p.apply(ToUi::Show(View::Sample));
    p.apply(ToUi::SampleEditor(editor));
}

pub fn sample_sampler(fonts: Fonts) -> Player {
    let mut p = sample_player(fonts);
    let (project, _) = mpfree_sampler_ui::sample::project();
    let pad = mpfree_sampler_ui::sample::PAD;
    let mut kits = p.kits.clone();
    kits[0].name = "Breaks".to_string();
    kits[0].pads = (0..16)
        .map(|i| (i == pad).then(|| "Break 92".to_string()))
        .collect();
    p.apply(ToUi::Kits(kits));
    p.apply(ToUi::SampleKits {
        projects: vec![project],
        rate: mpfree_sampler_ui::sample::RATE,
    });
    p.apply(ToUi::PadDown { pad, velocity: 100 });
    p.apply(ToUi::PadUp(pad));
    p.apply(ToUi::Show(View::Sample));
    let beat = f64::from(mpfree_sampler_ui::sample::RATE) * 60.0 / mpfree_sampler_ui::sample::BPM;
    let mut heads = Playheads {
        kit: 0,
        len: 1,
        ..Playheads::default()
    };
    heads.pad[0] = pad as u8;
    heads.frame[0] = (beat * 4.6) as u32;
    p.apply(ToUi::SamplePlayheads(heads));
    p
}

/// A sample player with an app behind it (every button known, so no guide).
fn sample_app() -> crate::app::App {
    let mut app = crate::app::App::new(44_100, false);
    (
        app.map.rec,
        app.map.overdub,
        app.map.undo,
        app.map.play_start,
    ) = (Some(90), Some(91), Some(92), Some(93));
    app.start();
    app
}

/// The mixer's Tracks view over the real mixer with tracks 1 and 2 playing
/// their own instruments (Poly, FM): a chord each for a few periods, the
/// costs fixed so the picture is the same on every run.
fn sample_instrument_mixer(synth: &crate::synth::Synth) -> mpfree_mixer_ui::MixState {
    use crate::synth::Engine;
    let mut mix = crate::mixing::MixHost::new(44_100, -18.0, -12.0);
    let plugins = std::sync::Arc::clone(mix.plugins());
    for (track, engine, notes) in [
        (0usize, Engine::Poly, [57u8, 60, 64]),
        (1, Engine::Fm, [45, 52, 57]),
    ] {
        let (id, values) = synth.sound_of(engine);
        if let Some(made) = crate::instruments::make(&plugins, id, &values, 44_100, crate::PERIOD) {
            let pads = synth.pads_for(engine);
            let _ = mix.instruments.put(track, made, pads, &synth.tuning_now());
        }
        for n in notes {
            mix.instruments.note_on(track, n, 100, 0);
        }
    }
    let silent = [0f32; crate::PERIOD];
    let (mut l, mut r) = ([0f32; crate::PERIOD], [0f32; crate::PERIOD]);
    for k in 0..40u64 {
        let ctx = mpfree_node::Ctx::new(44_100, k * crate::PERIOD as u64);
        mix.period(
            &ctx,
            (&silent, &silent),
            (&silent, &silent),
            (&silent, &silent),
            (&silent, &silent),
            &mut l,
            &mut r,
        );
    }
    mix.show(Some(mpfree_mixer_ui::MixView::Tracks));
    let mut st = mix
        .poll_state(std::time::Instant::now())
        .expect("the mixer shows");
    st.selected = Some(mpfree_fx::StripId::Track(crate::mixing::TRACKS + 1));
    st.cost_pct = st.cost_pct.map(|_| 12.0);
    for (k, v) in st.strips.iter_mut().chain([&mut st.master]).enumerate() {
        v.cost_pct = v.cost_pct.map(|_| 0.5 + k as f32 * 0.25);
        for i in &mut v.inserts {
            i.cost_pct = i.cost_pct.map(|_| 1.5);
        }
    }
    st
}

/// Every screen of the pad player with sample contents, as (name, player
/// showing it), for previews: loading, play, debug, console, prefs, options,
/// the guide, and the arp's quick band, panel and pattern workspace (each
/// with the real app and engine behind it).
/// The Looper tab with a real loop: two bars of the pads (a kick and hats)
/// and an overdub (a bass line), recorded by the looper on a transport.
/// The Looper tab as mockup J: loop 1 is mockup F's beat (drums, bass and
/// a chop, the chop muted, the bass on the Q-Links), loop 2 two bars of
/// hats, muted.
fn sample_looper(fonts: Fonts) -> Result<Player, String> {
    use crate::looping::{LoopHost, PERIOD};
    use mpfree_looper::{Command as L, Phase};
    use mpfree_transport::{Block, Command as T, Transport};
    const RATE: u32 = 44_100;
    let mut p = sample_player(fonts);
    p.apply(ToUi::Show(View::Looper));
    // Loop 2, on a looper of its own (the view takes its line).
    let mut h = LoopHost::new(RATE, 20);
    let mut t = Transport::new(RATE, 92.0).map_err(|e| format!("{e:?}"))?;
    let mut b = Block::new();
    h.select(1);
    h.command(L::SetBars(Some(2)));
    h.command(L::SetLatency(0));
    h.command(L::Record);
    t.command(0, T::Play);
    let beat = (f64::from(RATE) * 60.0 / 92.0) as u64;
    let len = 8 * beat;
    for i in 0..(len + len / 8) / PERIOD as u64 {
        if i == len / PERIOD as u64 {
            h.command(L::Mute(true));
        }
        if !t.process(PERIOD as u32, &mut b) {
            return Err("the transport refused a period".into());
        }
        let (mut l, mut r) = ([0.0f32; PERIOD], [0.0f32; PERIOD]);
        for f in 0..PERIOD {
            let n = i * PERIOD as u64 + f as u64;
            let half = (n % (beat / 2)) as f32 / RATE as f32;
            let noise = ((n.wrapping_mul(2_654_435_761) >> 7) % 2000) as f32 / 1000.0 - 1.0;
            let v = noise * (-half * 50.0).exp() * 0.3;
            (l[f], r[f]) = (v, v);
        }
        h.take_pads(&l, &r);
        h.period(&b, &mut l, &mut r);
        h.changes(String::new);
    }
    let second = h.view(92.0);
    if second.loops[1].phase != Phase::Playing || !second.loops[1].muted {
        return Err(format!("loop 2 is {:?}", second.loops[1].phase));
    }
    // Loop 1 and its layers, as the Play page's previews have them.
    let (wave, view) = recorded_loop();
    let mut v = view.clone();
    v.loops[1] = second.loops[1];
    v.memory.used += second.memory.used;
    // The memory the app gives the loops (Prefs "Loop memory" as it starts).
    v.memory.capacity = crate::looping::LoopMemory::default().seconds() as usize * RATE as usize;
    v.meter_db = [-6.5, -6.5];
    if let Some(l) = v.layers[2].as_mut() {
        l.muted = true;
        l.audible = false;
    }
    if let Some(l) = v.layers[1].as_mut() {
        l.level = 10f32.powf(-3.5 / 20.0);
        l.pan = -0.24;
    }
    if let Some(l) = v.layers[2].as_mut() {
        l.level = 10f32.powf(-6.0 / 20.0);
    }
    p.apply(ToUi::Looper(Box::new(v)));
    p.looper_page.set_names(&["Drums", "Bass", "Chop"]);
    p.apply(ToUi::LoopWave(Arc::clone(wave)));
    p.looper_page.pick_layer(&mut p.ui, 1);
    Ok(p)
}

/// The chain editor (`View::Chain`) or the macros panel (`View::Macros`)
/// with the chain-ui crate's sample song: the drum bus chain, Drive
/// selected and its Tone touched, as the audio thread sends them.
fn sample_panels(p: &mut Player, view: View) {
    use mpfree_chain_ui::sample::{Song, DRUMS};
    use mpfree_play::mpfree_param::Owner;
    let song = Song::new();
    p.apply(ToUi::Show(view));
    if view == View::Chain {
        let tone = song
            .drums
            .uid(1)
            .map(|uid| ParamAddr::new(Owner::Insert(DRUMS, uid), mpfree_fx::ids::drive::TONE));
        let mut st = song.chain_state(DRUMS, 1, 0, tone);
        st.back = Some("Mixer".into());
        let fx = song
            .chain(DRUMS)
            .node(1)
            .map(|n| Box::new(FxState::from_node(n, "", Color::rgb(0), 120.0, 44_100)));
        p.apply(ToUi::Chain(Box::new(st)));
        p.apply(ToUi::ChainFx(fx));
    }
    p.apply(ToUi::Macros(Box::new(song.macros_state(0))));
}

/// The Pads tab's Browse view over a library filled in a scratch folder
/// (removed again once the browser has read what it shows).
fn pads_browse(fonts: Fonts) -> Result<Player, String> {
    use mpfree_browser_ui::sample::{fill, open, select, source};
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir =
        std::env::temp_dir().join(format!("mpfree-pads-browse-{}-{nanos}", std::process::id()));
    let built = (|| {
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let lib = fill(&dir)?;
        let mut p = sample_sampler(fonts);
        p.set_browser(
            mpfree_browser_ui::Browser::inline(lib),
            "recent = Samples/Drums/Kick 01.wav\nfavourite = Samples/Drums/Snare tight.wav\n",
        );
        open_pads(&mut p, mpfree_sampler_ui::pads_tab::BROWSE);
        let b = p.browser.as_mut().ok_or("no browser")?;
        b.go_source(source("Samples"));
        open(b, "Drums");
        select(b, "Kick 01.wav");
        p.step_browser();
        p.take_actions();
        Ok(p)
    })();
    let _ = std::fs::remove_dir_all(&dir);
    built
}

/// The I/O view's Sync section's title.
const IO_SYNC: &str = mpfree_midi_ui::SYNC;

/// The mixer's I/O view's sections after Audio (layout spec, section 2):
/// MIDI, Sync and CV (`mpfree-midi-ui`), all on `model`. Returns their
/// titles in order, Audio being 0 and these 1 on, so the tempo chip can
/// open Sync by its title ([`Player::open_io_section`]).
fn add_io_sections(ui: &mut Ui, model: &mpfree_midi_ui::Shared) -> Vec<String> {
    let sections = mpfree_midi_ui::sections(model);
    let mut titles = Vec::new();
    for s in sections {
        titles.push(s.label().to_string());
        mpfree_mixer_ui::add_io_section(ui, s);
    }
    titles
}

pub fn sample_screens(fonts: impl Fn() -> Fonts) -> Vec<(&'static str, Player)> {
    let mut out = Vec::new();
    let mut loading = Player::new(fonts(), SCREEN, false, None);
    loading.apply(ToUi::Progress {
        message: "Loading drum kits".into(),
        detail: "Step 2 of 6   ·   18.4 of 39.0 MB".into(),
        value: 0.31,
    });
    out.push(("loading", loading));
    let mut failed = Player::new(fonts(), SCREEN, false, None);
    failed.apply(ToUi::Failed {
        message: "No sound card".into(),
        detail: "No sound card called ACVA after 30 s. Tap for the console.".into(),
    });
    out.push(("loading-failed", failed));
    out.push(("play", sample_player(fonts())));
    // The list of kits the kit chip opens over the page (Jay's card, 13 Nov
    // 14:57Z).
    let mut kits = sample_player(fonts());
    kits.toggle_kit_list();
    out.push(("kits", kits));
    // The Play page's views, as mockups F to I.
    for (name, v) in [
        ("play-all-loop", PlayView::All),
        ("play-beats", PlayView::Beats),
        ("play-loop", PlayView::Loop),
    ] {
        out.push((name, sample_play(fonts(), v)));
    }
    // The synth on the pads, A minor, In key: the pads panel's switch.
    let mut synth = sample_player(fonts());
    let mut s = crate::synth::Synth::new(44_100);
    let mut set = s.settings();
    set.scale = mpfree_arp::scale::by_name("Minor")
        .map(|d| mpfree_arp::scale::Scale::with_down(9, d.mask, d.down));
    set.set_layout(mpfree_arp::scale::pads::PadLayout::InKey);
    s.set_settings(set);
    let synth_kit = sample_kits().len();
    synth.apply(ToUi::KitChanged {
        kit: synth_kit,
        view: s.kit_view(),
    });
    synth.apply(ToUi::Kit(synth_kit));
    sample_loop(&mut synth);
    out.push(("play-synth-in-key", synth));
    // The mixer over Play (TRACK MIX), a strip selected for the Q-Links.
    let mut mixer = sample_player(fonts());
    let mut m = mpfree_mixer_ui::sample::song_mixer();
    mpfree_mixer_ui::sample::play(&mut m, 40);
    let mut st = mpfree_mixer_ui::MixState::from_mixer(
        &mut m,
        mpfree_mixer_ui::MixView::Tracks,
        &mpfree_mixer_ui::sample::name,
    );
    st.selected = Some(mpfree_fx::StripId::Track(1));
    // The costs are timed on this computer: fixed ones, so the picture is
    // the same on every run (golden/screens.txt).
    st.cost_pct = st.cost_pct.map(|_| 18.0);
    for (k, v) in st.strips.iter_mut().chain([&mut st.master]).enumerate() {
        v.cost_pct = v.cost_pct.map(|_| 0.5 + k as f32 * 0.25);
        for i in &mut v.inserts {
            i.cost_pct = i.cost_pct.map(|_| 1.5);
        }
    }
    let mut io_state = mpfree_mixer_ui::MixState::from_mixer(
        &mut m,
        mpfree_mixer_ui::MixView::Io,
        &mpfree_mixer_ui::sample::name,
    );
    io_state.selected = Some(mpfree_fx::StripId::Track(1));
    io_state.cost_pct = io_state.cost_pct.map(|_| 18.0);
    for (k, v) in io_state
        .strips
        .iter_mut()
        .chain([&mut io_state.master])
        .enumerate()
    {
        v.cost_pct = v.cost_pct.map(|_| 0.5 + k as f32 * 0.25);
        for i in &mut v.inserts {
            i.cost_pct = i.cost_pct.map(|_| 1.5);
        }
    }
    mixer.apply(ToUi::Show(View::Mixer));
    mixer.apply(ToUi::Mixer(Box::new(st)));
    out.push(("mixer", mixer));
    // The mixer's I/O view (the I/O icon lit), while the tempo follows a
    // clock on MIDI In: the tempo chip with its green dot.
    let mut io = sample_player(fonts());
    io.apply(ToUi::IoModel(std::sync::Arc::new(std::sync::Mutex::new(
        mpfree_midi_ui::sample::rig(),
    ))));
    io.apply(ToUi::ClockFrom(Some("MIDI In".to_string())));
    io.apply(ToUi::Show(View::Mixer));
    io.apply(ToUi::Mixer(Box::new(io_state)));
    out.push(("mixer-io", io));
    // The FX chain editor in the mixer's place (a strip's insert row): the
    // drum bus, Drive selected, Tone touched; and the macros panel over the
    // page (a long press on a macro card), layout spec 4b.
    for (name, view) in [("chain", View::Chain), ("macros", View::Macros)] {
        let mut p = sample_player(fonts());
        sample_panels(&mut p, view);
        out.push((name, p));
    }
    // Q-LINK on Play: Macros 1-4 in the Q-Link column.
    let mut macros = sample_player(fonts());
    macros.apply(ToUi::Macros(Box::new(
        mpfree_chain_ui::sample::Song::new().macros_state(0),
    )));
    macros.apply(ToUi::QLinkBank);
    out.push(("play-macros", macros));
    let mut debug = sample_player(fonts());
    debug.apply(ToUi::Show(View::Debug));
    out.push(("debug", debug));
    let mut console = sample_player(fonts());
    console.apply(ToUi::Show(View::Console));
    out.push(("syslog", console));
    use mpfree_controller::mpc_one::button::{MENU, NOTE_REPEAT, PLUS};
    let mut app = sample_app();
    for (name, shift) in [("prefs", true), ("options", false)] {
        let mut p = sample_player(fonts());
        if shift {
            app.button(mpfree_controller::mpc_one::button::SHIFT, true, 0.0);
        }
        for fx in app.button(MENU, true, 0.0) {
            match fx {
                crate::app::Effect::Menu(m) => p.apply(ToUi::Menu(m)),
                crate::app::Effect::Show(v) => p.apply(ToUi::Show(v)),
                _ => {}
            }
        }
        app.button(MENU, false, 0.0);
        // A second press closes the menu again, ready for the next sample.
        app.button(MENU, true, 0.0);
        app.button(MENU, false, 0.0);
        app.button(mpfree_controller::mpc_one::button::SHIFT, false, 0.0);
        out.push((name, p));
    }
    // Prefs' touch screen section.
    let mut touch = sample_player(fonts());
    let mut t = crate::app::App::new(44_100, false);
    t.map.menu = Some(MENU);
    t.button(mpfree_controller::mpc_one::button::SHIFT, true, 0.0);
    let mut fx = t.button(MENU, true, 0.0);
    t.button(MENU, false, 0.0);
    t.button(mpfree_controller::mpc_one::button::SHIFT, false, 0.0);
    fx.extend(t.step_item(usize::MAX, 1));
    for e in fx {
        match e {
            crate::app::Effect::Menu(m) => touch.apply(ToUi::Menu(m)),
            crate::app::Effect::Show(v) => touch.apply(ToUi::Show(v)),
            _ => {}
        }
    }
    out.push(("prefs-touch", touch));
    // The number pad, typing Play's tempo (a long press on its BPM).
    let mut typing = sample_player(fonts());
    typing.apply(ToUi::Show(View::Play));
    typing.type_tempo();
    if let Some(n) = typing.ui.get_mut::<mpfree_ui::numpad::NumPad>(NUMPAD) {
        n.typed = "98.5".to_string();
    }
    out.push(("numpad-tempo", typing));
    // UNDO held on Play's Beats view: the takes' history, two undone, the
    // wheel's outline one step back from now.
    let mut undo = sample_player(fonts());
    undo.apply(ToUi::Show(View::Play));
    undo.apply(ToUi::PlayView(Some(PlayView::Beats)));
    let mut st = undo.time.clone();
    let take = |n: u32, track: usize, captured: bool, edited: bool| mpfree_time_ui::view::TakeRow {
        number: n,
        captured,
        edited,
        made: n == 1,
        at: Some((track, 0)),
        added: 4 + n as usize,
        erased: usize::from(edited),
        deleted: false,
    };
    st.done = vec![
        take(1, 0, false, false),
        take(2, 0, false, true),
        take(3, 1, true, false),
        take(4, 0, false, true),
    ];
    st.undone = vec![take(6, 1, false, false), take(5, 0, false, true)];
    undo.apply(ToUi::Time(Box::new(st)));
    undo.apply(ToUi::UndoButton(true));
    undo.tick(UNDO_HOLD);
    undo.apply(ToUi::Rows(crate::app::RowKey::Pick(-1)));
    out.push(("undo-history", undo));
    // Prefs with the Appearance row selected, stepped to Synthwave.
    let mut look = sample_player(fonts());
    let mut l = crate::app::App::new(44_100, false);
    let mut fx = l.open_tab(View::Prefs);
    let at = l.row_of(crate::app::Item::Appearance).unwrap_or(0);
    fx.extend(l.step_item(at, 1));
    fx.extend(l.step_item(at, 1));
    apply_effects(&mut look, fx);
    out.push(("prefs-appearance", look));
    // The guide from Prefs ("Find the buttons again"), PLAY pressed when it
    // asks for MENU.
    let mut guide = sample_player(fonts());
    let mut g = crate::app::App::new(44_100, false);
    let mut fx = g.start();
    fx.extend(g.open_tab(View::Prefs));
    fx.extend(g.step_item(g.row_of(crate::app::Item::FindButtons).unwrap_or(0), 1));
    fx.extend(g.button(button_play(), true, 0.0));
    apply_effects(&mut guide, fx);
    out.push(("guide", guide));
    // Debug mode: Map controls, REC found, listening for OVERDUB.
    let mut map = sample_player(fonts());
    let mut m = sample_app();
    (m.map.rec, m.map.overdub) = (None, None);
    let mut fx = m.open_tab(View::Prefs);
    fx.extend(m.step_item(m.row_of(crate::app::Item::MapControls).unwrap_or(0), 1));
    fx.extend(m.map_row(0));
    fx.extend(m.button(73, true, 0.0));
    fx.extend(m.button(73, false, 0.0));
    fx.extend(m.map_row(1));
    apply_effects(&mut map, fx);
    out.push(("debug-map", map));
    // Debug mode: the Performance page.
    let mut perf = sample_player(fonts());
    perf.apply(ToUi::Settings(Settings {
        debug_mode: true,
        ..Settings::default()
    }));
    perf.apply(ToUi::Show(View::Perf));
    for k in 0..200u32 {
        let wave = (k as f32 * 0.21).sin().abs();
        perf.apply(ToUi::Timing(Timing {
            periods_queued: 3,
            audio: AudioNumbers {
                render_mean_us: 280.0 + 120.0 * wave,
                render_max_us: 450.0 + 500.0 * wave,
                synth_mean_us: 90.0,
                synth_max_us: 160.0,
                worst_us: 1210.0,
                steals: 2,
                dropped: 0,
                peak_grains: 48,
            },
            ..Timing::default()
        }));
    }
    perf.set_proc(ProcView {
        over_s: 2.0,
        threads: vec![
            ("mpfree (812)".into(), 31.5),
            ("mpfree-screen (815)".into(), 9.0),
            ("mpfree-journal (816)".into(), 0.5),
            ("touch-reader (817)".into(), 0.2),
        ],
        process_cpu: 41.2,
        system_cpu: Some(14.8),
        cores: 4,
        rss_kb: Some(61_440),
        peak_rss_kb: Some(64_512),
        thread_count: 5,
        mem_total_kb: Some(2_042_000),
        mem_available_kb: Some(1_610_000),
        load: Some("0.61 0.48 0.30".into()),
        temps: vec![("soc-thermal".into(), 47.8)],
        mhz: vec![1608, 1608, 1608, 1608],
    });
    out.push(("debug-perf", perf));
    // Debug mode: CPU by engine, from made-up probe numbers shaped like
    // round 6's (synth worst near 20 %, sampler near 24 %), with the sound
    // on two cores and a few late mixer stages.
    let mut eng = sample_player(fonts());
    eng.apply(ToUi::Settings(Settings {
        debug_mode: true,
        ..Settings::default()
    }));
    let mut probes = crate::engines::EngineProbes::new();
    for k in 0..2_000u64 {
        let spike = |every: u64, us: u64, base: u64| {
            Duration::from_micros(if k % every == 0 { us } else { base + k % 7 })
        };
        let (s, y, l, t, m, c) = (
            spike(500, 690, 30),
            spike(400, 570, 55),
            spike(1000, 40, 6),
            spike(900, 25, 4),
            spike(800, 18, 3),
            spike(700, 310, 45),
        );
        // The period is the engines plus the rest (the control row).
        for (i, d) in [
            (crate::engines::SAMPLER, s),
            (crate::engines::SYNTH, y),
            (crate::engines::LOOPER, l),
            (crate::engines::TIME, t),
            (crate::engines::MIX, m),
            (crate::engines::PERIOD, s + y + l + t + m + c),
        ] {
            probes.add(i, d);
        }
        if k == 1_000 {
            probes.publish();
        }
    }
    probes.publish();
    // Where the sound runs, as the MPC shows it with --sound-cores two.
    crate::cores::sample(&probes.shared().cores);
    // A few slow periods outside the engines, the longest part name, so
    // the picture shows the widest the row gets.
    {
        use std::sync::atomic::Ordering::Relaxed;
        let parts = &probes.shared().parts;
        parts.slow.store(12_345, Relaxed);
        parts.worst_ns.store(12_340_000, Relaxed);
        parts
            .worst_part
            .store(crate::parts::MIDI_IN as u64, Relaxed);
    }
    // Memory by engine: Boom Bap's sounds, and a resident size like live
    // test 3's peak (195.8 MB of 2016.7 MB).
    eng.apply(ToUi::SampleKits {
        projects: vec![boom_bap_project()],
        rate: 44_100,
    });
    eng.set_proc(ProcView {
        rss_kb: Some(191_211),
        mem_total_kb: Some(1_969_436),
        ..ProcView::default()
    });
    eng.apply(ToUi::Engines(probes.shared()));
    eng.apply(ToUi::Show(View::Engines));
    eng.refresh_engines();
    out.push(("debug-engines", eng));
    // Debug mode: Processes, from a made-up /proc sample.
    let mut pr = sample_player(fonts());
    pr.apply(ToUi::Settings(Settings {
        debug_mode: true,
        ..Settings::default()
    }));
    pr.apply(ToUi::Show(View::Procs));
    let row = |pid: u32, name: &str, cpu: f64, mb: u64, threads: u64, policy: &str| {
        mpfree_prof::procs::ProcRow {
            pid,
            name: name.into(),
            state: 'S',
            cpu_pct: cpu,
            rss_kb: mb * 1024,
            threads,
            policy: policy.into(),
            nice: 0,
        }
    };
    pr.set_procs(&mpfree_prof::procs::ProcView {
        over_s: 1.0,
        rows: vec![
            row(812, "mpfree", 31.5, 60, 6, "FIFO 70"),
            row(160, "kworker/u8:2", 2.0, 0, 1, "normal"),
            row(98, "irq/45-ff0c0000", 1.0, 0, 1, "FIFO 50"),
            row(1, "systemd", 0.0, 9, 1, "normal"),
            row(230, "systemd-journal", 0.0, 12, 1, "normal"),
            row(402, "dbus-daemon", 0.0, 3, 1, "normal"),
            row(415, "sshd", 0.0, 6, 1, "normal"),
        ],
        total_cpu_pct: Some(9.1),
        core_cpu_pct: vec![32.0, 2.5, 1.0, 1.0],
    });
    out.push(("debug-procs", pr));
    // Debug mode: the Touch page after some taps, holds and swipes, a finger
    // down, and a small saved correction; then Targets mode, four done.
    push_touch_dev_samples(&mut out, &fonts);
    // Prefs > System info, with fixed rows (the host's own would differ).
    let mut system = sample_player(fonts());
    system.apply(ToUi::Show(View::System));
    system.set_system(crate::sysinfo::sample());
    out.push(("system-info", system));
    match sample_clips(fonts()) {
        Ok(p) => out.push(("clips", p)),
        Err(e) => eprintln!("clips sample: {e}"),
    }
    // Each track's instrument under its name: Track 2 plays FM, Track 4
    // Poly and Track 6 a plugin synth; the rest the kit.
    match sample_clips(fonts()) {
        Ok(mut p) => {
            let mut s: TrackSounds = [None; crate::instruments::TRACKS];
            for (t, id, name) in [
                (1, "mpfree.fm", "FM"),
                (3, "mpfree.poly", "Poly"),
                (5, "clap:org.example.synth", "Example Synth"),
            ] {
                s[t] = Some(crate::instruments::Held { id, name });
            }
            p.apply(ToUi::TrackSounds(Box::new(s)));
            out.push(("arrange-instruments", p));
        }
        Err(e) => eprintln!("clips sample: {e}"),
    }
    // The note editor on the Arrange page: the piano roll, the list, then
    // the tracker on a bar of drums.
    for (name, lens) in [
        ("arrange-notes", Some(Lens::Grid)),
        ("arrange-notes-list", Some(Lens::List)),
        ("arrange-notes-tracker", None),
    ] {
        let state = match lens {
            Some(lens) => mpfree_time_ui::sample::notes_state(lens),
            None => mpfree_time_ui::sample::tracker_state(),
        };
        match state {
            Ok(state) => {
                let mut p = quiet_player(fonts());
                p.apply(ToUi::Time(Box::new(state)));
                p.apply(ToUi::Show(View::Arrange));
                out.push((name, p));
            }
            Err(e) => eprintln!("{name} sample: {e}"),
        }
    }
    // The Arrange page's Song view: rows made into a song, song mode on,
    // Rec song armed.
    match mpfree_time_ui::sample::song_state() {
        Ok(state) => {
            let mut p = quiet_player(fonts());
            p.apply(ToUi::Time(Box::new(state)));
            p.apply(ToUi::Show(View::Arrange));
            out.push(("arrange-song", p));
        }
        Err(e) => eprintln!("arrange-song sample: {e}"),
    }
    // The same with two audio lanes under the song (layout spec 4c): a
    // vocal punched in over bars 5-8 and that region picked, a guitar
    // armed. Real takes from the audio tracks engine.
    match mpfree_time_ui::sample::song_audio_state() {
        Ok(state) => {
            let mut p = quiet_player(fonts());
            p.apply(ToUi::Time(Box::new(state)));
            p.apply(ToUi::Show(View::Arrange));
            out.push(("arrange-song-audio", p));
        }
        Err(e) => eprintln!("arrange-song-audio sample: {e}"),
    }
    // The same with the guitar's track panel open (layout spec 4c), its
    // input on ST, soloed, and nothing picked: the Q-Links are its level
    // and pan.
    match mpfree_time_ui::sample::song_audio_state() {
        Ok(mut state) => {
            state.region = None;
            state.audio_panel = Some(1);
            if let Some(l) = state.audio.iter_mut().find(|l| l.track == 1) {
                l.solo = true;
                l.volume = 0.71;
                l.pan = 0.3;
            }
            let mut p = quiet_player(fonts());
            p.apply(ToUi::Time(Box::new(state)));
            p.apply(ToUi::Show(View::Arrange));
            out.push(("arrange-song-panel", p));
        }
        Err(e) => eprintln!("arrange-song-panel sample: {e}"),
    }
    // The vocal's take lanes open under its lane (layout spec 4c): take 2
    // plays over bars 5-8, take 1 everywhere else.
    match mpfree_time_ui::sample::song_audio_state() {
        Ok(mut state) => {
            state.audio_takes = Some(0);
            let mut p = quiet_player(fonts());
            p.apply(ToUi::Time(Box::new(state)));
            p.apply(ToUi::Show(View::Arrange));
            out.push(("arrange-song-takes", p));
        }
        Err(e) => eprintln!("arrange-song-takes sample: {e}"),
    }
    match sample_looper(fonts()) {
        Ok(p) => out.push(("looper", p)),
        Err(e) => eprintln!("looper sample: {e}"),
    }
    // The same with the undo history open: the bass and the chop overdubs.
    match sample_looper(fonts()) {
        Ok(mut p) => {
            p.looper_page.toggle_history(&mut p.ui);
            p.refresh_hint();
            out.push(("looper-history", p));
        }
        Err(e) => eprintln!("looper sample: {e}"),
    }
    // The synth on the pads (the kit list's last entry), then its page.
    let synth_kit = sample_kits().len();
    let mut play_synth = sample_player(fonts());
    play_synth.apply(ToUi::Status(Status {
        shift: false,
        pad_level: PadLevel::Off,
        repeat: false,
        latched: false,
        rate: "1/16",
        bpm: 120.0,
        mute_keys: MuteKeys::Off,
        erase: false,
    }));
    play_synth.apply(ToUi::Kit(synth_kit));
    for (pad, v) in [(0usize, 96u8), (4, 90), (7, 101)] {
        play_synth.apply(ToUi::PadDown { pad, velocity: v });
    }
    out.push(("play-synth", play_synth));
    for (name, page, qlink) in [("synth", 0, (2, 12)), ("synth-filter", 1, (0, -20))] {
        let mut p = sample_player(fonts());
        p.apply(ToUi::Kit(synth_kit));
        p.apply(ToUi::Show(View::Synth));
        p.apply(ToUi::SynthPage(page));
        p.apply(ToUi::SynthQLink(qlink.0, qlink.1));
        p.take_actions();
        out.push((name, p));
    }
    // FM on the pads with D Dorian chord pads, three chords held; FM's Synth
    // tab; and the Synth tab's options with FM and a scale picked.
    let mut fm = crate::synth::Synth::new(44_100);
    let mut set = fm.settings();
    set.engine = crate::synth::Engine::Fm;
    set.scale = mpfree_arp::scale::by_name("Dorian")
        .map(|d| mpfree_arp::scale::Scale::with_down(2, d.mask, d.down));
    set.set_layout(mpfree_arp::scale::pads::PadLayout::Chords);
    fm.set_settings(set);
    let mut chords = sample_player(fonts());
    chords.apply(ToUi::Status(Status {
        shift: false,
        pad_level: PadLevel::Off,
        repeat: false,
        latched: false,
        rate: "1/16",
        bpm: 120.0,
        mute_keys: MuteKeys::Off,
        erase: false,
    }));
    chords.apply(ToUi::KitChanged {
        kit: synth_kit,
        view: fm.kit_view(),
    });
    chords.apply(ToUi::Synth(fm.state()));
    chords.apply(ToUi::Kit(synth_kit));
    for (pad, v) in [(0usize, 96u8), (3, 90), (12, 101)] {
        chords.apply(ToUi::PadDown { pad, velocity: v });
    }
    out.push(("play-synth-chords", chords));
    let mut fm_page = sample_player(fonts());
    fm_page.apply(ToUi::KitChanged {
        kit: synth_kit,
        view: fm.kit_view(),
    });
    fm_page.apply(ToUi::Synth(fm.state()));
    fm_page.apply(ToUi::Kit(synth_kit));
    fm_page.apply(ToUi::Show(View::Synth));
    out.push(("synth-fm", fm_page));
    let mut options = sample_player(fonts());
    let mut a = sample_app();
    let mut fx = a.open_tab(View::Synth);
    fx.extend(a.button(mpfree_controller::mpc_one::button::MENU, true, 0.0));
    fx.extend(a.button(mpfree_controller::mpc_one::button::MENU, false, 0.0));
    fx.extend(a.step_item(0, 1));
    for _ in 0..2 {
        fx.extend(a.step_item(1, 1));
    }
    fx.extend(a.step_item(2, 1));
    fx.extend(a.step_item(2, 1));
    fx.extend(a.step_item(3, 1));
    fx.extend(a.select(3));
    apply_effects(&mut options, fx);
    out.push(("synth-options", options));
    // The Instrument row on track 2 stepped past the engines to a plugin
    // synth (two found), its help open.
    let mut inst = sample_player(fonts());
    let mut a = sample_app();
    a.plugin_synths = vec![
        ("clap:com.example.dexed", "Dexed"),
        ("clap:com.example.surge-xt", "Surge XT"),
    ];
    a.set_track(1);
    let mut fx = a.open_tab(View::Synth);
    fx.extend(a.button(mpfree_controller::mpc_one::button::MENU, true, 0.0));
    fx.extend(a.button(mpfree_controller::mpc_one::button::MENU, false, 0.0));
    // Kit, the engines, Dexed, Surge XT.
    for _ in 0..crate::synth::Engine::ALL.len() + 2 {
        fx.extend(a.step_item(0, 1));
    }
    fx.extend(a.select(0));
    apply_effects(&mut inst, fx);
    out.push(("synth-instrument", inst));
    // The Synth tab on track 2's own FM, its second preset loaded.
    let mut own = sample_player(fonts());
    let (id, values) = fm
        .preset_values(crate::synth::Engine::Fm, 1)
        .map(|(_, v)| (crate::synth::Engine::Fm.node_id(), v))
        .unwrap_or_else(|| fm.sound_of(crate::synth::Engine::Fm));
    if let Some(node) = crate::synth::fresh_node(id, &values, 44_100) {
        own.apply(ToUi::Synth(fm.state_of(
            node.as_ref(),
            Some(crate::synth::Engine::Fm),
            "Track 2",
            Some(1),
        )));
    }
    own.apply(ToUi::Show(View::Synth));
    out.push(("synth-track", own));
    // The mixer with two tracks playing their own instruments (Poly on
    // track 1, FM on track 2): their channels after Drums, Synth, Loop and Audio.
    let mut mixer = sample_player(fonts());
    mixer.apply(ToUi::Show(View::Mixer));
    mixer.apply(ToUi::Mixer(Box::new(sample_instrument_mixer(&fm))));
    out.push(("mixer-instruments", mixer));
    // The Sample page: the loop on kit 1's pad 5, then zoomed round the
    // loop start after a Q-Link 3 turn, then a pad without a sound, then
    // the Chop tab.
    out.push(("sample", sample_sampler(fonts())));
    let mut zoomed = sample_sampler(fonts());
    zoomed.apply(ToUi::SampleQLink(2, 3));
    for _ in 0..10 {
        zoomed.apply(ToUi::SampleWheel(1));
    }
    zoomed.take_actions();
    out.push(("sample-zoomed", zoomed));
    let mut empty = sample_sampler(fonts());
    empty.apply(ToUi::PadDown {
        pad: 1,
        velocity: 90,
    });
    empty.apply(ToUi::PadUp(1));
    out.push(("sample-no-sound", empty));
    // The Pads tab's Pad view: A5 copied onto A2, A5 picked again.
    let mut pads = sample_sampler(fonts());
    open_pads(&mut pads, mpfree_sampler_ui::pads_tab::PAD_VIEW);
    pads.on_event(UiEvent {
        widget: mpfree_sampler_ui::pads_tab::BUTTONS.into(),
        kind: EventKind::Tab(0),
    });
    pads.on_event(UiEvent {
        widget: mpfree_sampler_ui::pads_tab::MAP.into(),
        kind: EventKind::Select(1),
    });
    pads.on_event(UiEvent {
        widget: mpfree_sampler_ui::pads_tab::BUTTONS.into(),
        kind: EventKind::Tab(1),
    });
    pads.on_event(UiEvent {
        widget: mpfree_sampler_ui::pads_tab::MAP.into(),
        kind: EventKind::Select(mpfree_sampler_ui::sample::PAD),
    });
    pads.take_actions();
    out.push(("pads-pad", pads));
    // The Pads tab's Browse view: the browser beside the pads over a
    // scratch library, Samples / Drums with Kick 01 selected.
    match pads_browse(fonts()) {
        Ok(browse) => out.push(("pads-browse", browse)),
        Err(e) => eprintln!("pads-browse: not drawn ({e})"),
    }
    // The Chop tab on the same pad: chopped at its hits, slice 3 picked.
    let mut chop = sample_sampler(fonts());
    chop.on_event(UiEvent {
        widget: mpfree_sampler_ui::wave_tab::BAR.into(),
        kind: EventKind::Tab(mpfree_sampler_ui::chop_tab::TAB),
    });
    chop.apply(ToUi::SampleQLink(0, 2));
    chop.take_actions();
    out.push(("sample-chop", chop));
    // The Warp tab on the same pad: warped to stretch, its hits pinned to
    // the sixteenths, the fourth pin picked.
    let mut warp = sample_sampler(fonts());
    warp.on_event(UiEvent {
        widget: mpfree_sampler_ui::wave_tab::BAR.into(),
        kind: EventKind::Tab(mpfree_sampler_ui::warp_tab::TAB),
    });
    warp.on_event(UiEvent {
        widget: mpfree_sampler_ui::warp_tab::LIST.into(),
        kind: EventKind::Step {
            index: mpfree_sampler_ui::warp_tab::Row::Warp.index(),
            delta: 1,
        },
    });
    warp.apply(ToUi::SampleLift);
    warp.on_event(UiEvent {
        widget: mpfree_sampler_ui::warp_tab::BUTTONS.into(),
        kind: EventKind::Tab(0),
    });
    warp.apply(ToUi::SampleQLink(0, 4));
    warp.take_actions();
    out.push(("sample-warp", warp));
    // The Play tab on the same pad: hit at 100, Decay turned (Q-Link 2),
    // the filter turned on from the list.
    let mut play = sample_sampler(fonts());
    play.on_event(UiEvent {
        widget: mpfree_sampler_ui::wave_tab::BAR.into(),
        kind: EventKind::Tab(mpfree_sampler_ui::play_tab::TAB),
    });
    play.apply(ToUi::PadDown {
        pad: mpfree_sampler_ui::sample::PAD,
        velocity: 100,
    });
    play.apply(ToUi::PadUp(mpfree_sampler_ui::sample::PAD));
    play.apply(ToUi::SampleQLink(1, 40));
    play.apply(ToUi::SampleLift);
    let filter = mpfree_sampler_ui::play_tab::Row::ALL
        .iter()
        .position(|r| *r == mpfree_sampler_ui::play_tab::Row::Filter)
        .unwrap_or(0);
    play.on_event(UiEvent {
        widget: mpfree_sampler_ui::play_tab::LIST.into(),
        kind: EventKind::Step {
            index: filter,
            delta: 1,
        },
    });
    play.take_actions();
    out.push(("sample-play", play));
    // The scene strip: A stored, the pad turned down, B stored, and the
    // crossfader 40 % of the way to B.
    {
        use mpfree_sampler_ui::play_tab::{Row, FADER, LIST, SCENES};
        let mut scenes = sample_sampler(fonts());
        scenes.on_event(UiEvent {
            widget: mpfree_sampler_ui::wave_tab::BAR.into(),
            kind: EventKind::Tab(mpfree_sampler_ui::play_tab::TAB),
        });
        let tap = |p: &mut Player, widget: &str, kind: EventKind| {
            p.on_event(UiEvent {
                widget: widget.into(),
                kind,
            });
        };
        tap(&mut scenes, SCENES, EventKind::Tab(0));
        let level = Row::ALL.iter().position(|r| *r == Row::Level).unwrap_or(0);
        for _ in 0..4 {
            tap(
                &mut scenes,
                LIST,
                EventKind::Step {
                    index: level,
                    delta: -1,
                },
            );
        }
        scenes.apply(ToUi::SampleLift);
        tap(&mut scenes, SCENES, EventKind::Tab(1));
        if let Some(f) = scenes.ui.get_mut::<mpfree_ui::widgets::Fader>(FADER) {
            f.value = 0.4;
        }
        tap(&mut scenes, FADER, EventKind::Value(0.4));
        scenes.take_actions();
        out.push(("sample-play-scenes", scenes));
    }
    // The Grain tab on the same pad, set to play grains: the engine's own
    // cloud 1.5 s after a hit, as the audio thread sends it.
    let mut grain = sample_sampler(fonts());
    let (project, _) = mpfree_sampler_ui::sample::grain_project();
    let (grains, head, live) = mpfree_sampler_ui::sample::grains_after(&project, 1.5);
    let region = project
        .pad(mpfree_sampler_ui::sample::PAD)
        .and_then(|pd| pd.layers.first())
        .map(|l| l.region.clone());
    grain.apply(ToUi::SampleKits {
        projects: vec![project],
        rate: mpfree_sampler_ui::sample::RATE,
    });
    grain.on_event(UiEvent {
        widget: mpfree_sampler_ui::wave_tab::BAR.into(),
        kind: EventKind::Tab(mpfree_sampler_ui::grain_tab::TAB),
    });
    if let Some(FromUi::SampleGrains(mut c)) = grain
        .take_actions()
        .into_iter()
        .find(|a| matches!(a, FromUi::SampleGrains(_)))
    {
        c.len = grains.len().min(crate::sampler::CLOUD_GRAINS);
        c.grains[..c.len].copy_from_slice(&grains[..c.len]);
        c.head = head
            .zip(region)
            .map(|(h, r)| r.start + (f64::from(h) * r.frames() as f64) as u64);
        c.live = live;
        grain.apply(ToUi::SampleGrains(c));
    }
    out.push(("sample-grain", grain));
    // The Rec tab with the line input on: 40 s of drums came in with the
    // transport playing, and a take on the next bar is two bars in.
    let mut rec = sample_sampler(fonts());
    rec.on_event(UiEvent {
        widget: mpfree_sampler_ui::wave_tab::BAR.into(),
        kind: EventKind::Tab(mpfree_sampler_ui::rec_tab::TAB),
    });
    let mut s = rec.settings.clone();
    s.line_input = true;
    rec.apply(ToUi::Settings(s));
    if let Some(FromUi::SampleInput(Some(mut feed))) = rec
        .take_actions()
        .into_iter()
        .find(|a| matches!(a, FromUi::SampleInput(_)))
    {
        use mpfree_sampler_ui::rec_tab::{Row, LIST};
        use mpfree_sampler_ui::sample::{bar_frames, feed_input, RATE};
        feed_input(&mut feed.0, 40.0, 0.4, true);
        rec.tick(0.033);
        // Start: the next bar; End: after the length (4 bars).
        for (row, delta) in [(Row::Start, 2), (Row::End, 2)] {
            let index = Row::ALL.iter().position(|r| *r == row).unwrap_or(0);
            rec.on_event(UiEvent {
                widget: LIST.into(),
                kind: EventKind::Step { index, delta },
            });
        }
        rec.on_event(UiEvent {
            widget: mpfree_sampler_ui::rec_tab::BUTTONS.into(),
            kind: EventKind::Tab(0),
        });
        feed_input(&mut feed.0, 2.2 * bar_frames() / f64::from(RATE), 0.4, true);
        rec.tick(0.033);
        rec.take_actions();
    }
    out.push(("sample-rec", rec));
    // The Rec tab's Auto-sample mode on the Synth tab's patch: every 5
    // semitones, 10 notes onto the empty pads after the break.
    let mut auto = sample_sampler(fonts());
    {
        use mpfree_sampler_ui::auto_tab::{Row, LIST};
        auto.on_event(UiEvent {
            widget: mpfree_sampler_ui::wave_tab::BAR.into(),
            kind: EventKind::Tab(mpfree_sampler_ui::rec_tab::TAB),
        });
        auto.on_event(UiEvent {
            widget: mpfree_sampler_ui::rec_tab::BUTTONS.into(),
            kind: EventKind::Tab(3),
        });
        auto.on_event(UiEvent {
            widget: LIST.into(),
            kind: EventKind::Step {
                index: Row::Every.index(),
                delta: 2,
            },
        });
        auto.take_actions();
    }
    out.push(("sample-auto", auto));
    // The Play tab with the pad a keygroup and the pads playing notes, the
    // list on the Keys rows.
    {
        use mpfree_sampler_ui::play_tab::{Row, LIST};
        use mpfree_sampler_ui::widgets::SettingList;
        let mut keys = sample_sampler(fonts());
        keys.on_event(UiEvent {
            widget: mpfree_sampler_ui::wave_tab::BAR.into(),
            kind: EventKind::Tab(mpfree_sampler_ui::play_tab::TAB),
        });
        let index = |row: Row| Row::ALL.iter().position(|r| *r == row).unwrap_or(0);
        for (row, delta) in [(Row::Keys, 1), (Row::LowKey, 36), (Row::HighKey, -43)] {
            keys.on_event(UiEvent {
                widget: LIST.into(),
                kind: EventKind::Step {
                    index: index(row),
                    delta,
                },
            });
            keys.apply(ToUi::SampleLift);
        }
        keys.on_event(UiEvent {
            widget: LIST.into(),
            kind: EventKind::Step {
                index: index(Row::PadsPlay),
                delta: 1,
            },
        });
        // Pads play picked, as a tap on the row picks it.
        if let Some(list) = keys.ui.get_mut::<SettingList>(LIST) {
            list.selected = index(Row::PadsPlay);
        }
        keys.on_event(UiEvent {
            widget: LIST.into(),
            kind: EventKind::Select(index(Row::PadsPlay)),
        });
        keys.take_actions();
        out.push(("sample-play-keys", keys));
    }
    // A bank mapped to keys on the Pads tab: five pads named by their
    // notes, one keygroup instrument; the Play tab's keyboard with A5's
    // zone picked and E3 held.
    {
        let mut mapped = sample_sampler(fonts());
        let (mut project, _) = mpfree_sampler_ui::sample::project();
        let pad = mpfree_sampler_ui::sample::PAD;
        if let Some(sound) = project.pad(pad).cloned() {
            for (k, name) in ["Pluck C2", "Pluck G2", "Pluck C3", "Pluck G3", "Pluck C4"]
                .into_iter()
                .enumerate()
            {
                let mut q = sound.clone();
                q.name = name.to_string();
                let _ = project.set_pad(k, Some(q), None);
            }
        }
        mapped.apply(ToUi::SampleKits {
            projects: vec![project],
            rate: mpfree_sampler_ui::sample::RATE,
        });
        let ev = |p: &mut Player, widget: &str, kind: EventKind| {
            p.on_event(UiEvent {
                widget: widget.into(),
                kind,
            });
        };
        use mpfree_sampler_ui::{pads_tab, play_tab};
        open_pads(&mut mapped, pads_tab::PAD_VIEW);
        ev(&mut mapped, pads_tab::BUTTONS, EventKind::Tab(3));
        open_sample(&mut mapped, sample_page::Editor::Play);
        let held = mapped.ui.rect_of(play_tab::KEYS).and_then(|r| {
            mapped
                .ui
                .get::<mpfree_sampler_ui::keys_widgets::KeyZones>(play_tab::KEYS)?
                .key_point(r, 52)
        });
        if let Some((x, y)) = held {
            mapped.touch(3, Touch::Down, x, y);
        }
        mapped.take_actions();
        out.push(("sample-keys-mapped", mapped));
    }
    // The same instrument with Key fade at 4 notes: the zones that met now
    // overlap by 4 and crossfade, the Key fade row picked.
    {
        let mut faded = sample_sampler(fonts());
        let (mut project, _) = mpfree_sampler_ui::sample::project();
        let pad = mpfree_sampler_ui::sample::PAD;
        if let Some(sound) = project.pad(pad).cloned() {
            for (k, name) in ["Pluck C2", "Pluck G2", "Pluck C3", "Pluck G3", "Pluck C4"]
                .into_iter()
                .enumerate()
            {
                let mut q = sound.clone();
                q.name = name.to_string();
                let _ = project.set_pad(k, Some(q), None);
            }
        }
        faded.apply(ToUi::SampleKits {
            projects: vec![project],
            rate: mpfree_sampler_ui::sample::RATE,
        });
        let ev = |p: &mut Player, widget: &str, kind: EventKind| {
            p.on_event(UiEvent {
                widget: widget.into(),
                kind,
            });
        };
        use mpfree_sampler_ui::pads_tab;
        use mpfree_sampler_ui::play_tab::{Row, LIST};
        use mpfree_sampler_ui::widgets::SettingList;
        open_pads(&mut faded, pads_tab::PAD_VIEW);
        ev(&mut faded, pads_tab::BUTTONS, EventKind::Tab(3));
        open_sample(&mut faded, sample_page::Editor::Play);
        let index = Row::ALL
            .iter()
            .position(|r| *r == Row::KeyFade)
            .unwrap_or(0);
        if let Some(list) = faded.ui.get_mut::<SettingList>(LIST) {
            list.selected = index;
        }
        ev(&mut faded, LIST, EventKind::Select(index));
        ev(&mut faded, LIST, EventKind::Step { index, delta: 4 });
        faded.apply(ToUi::SampleLift);
        faded.take_actions();
        out.push(("sample-keys-crossfade", faded));
    }
    // The Play tab with Also plays picked: hits on A1 and A3 added them,
    // and the pad latches.
    {
        use mpfree_sampler_ui::play_tab::{self, Row, LIST};
        use mpfree_sampler_ui::widgets::SettingList;
        let mut links = sample_sampler(fonts());
        links.on_event(UiEvent {
            widget: mpfree_sampler_ui::wave_tab::BAR.into(),
            kind: EventKind::Tab(play_tab::TAB),
        });
        let index = |row: Row| Row::ALL.iter().position(|r| *r == row).unwrap_or(0);
        for (row, step) in [(Row::Latch, 1), (Row::Links, 0)] {
            if let Some(list) = links.ui.get_mut::<SettingList>(LIST) {
                list.selected = index(row);
            }
            let kind = if step == 0 {
                EventKind::Select(index(row))
            } else {
                EventKind::Step {
                    index: index(row),
                    delta: step,
                }
            };
            links.on_event(UiEvent {
                widget: LIST.into(),
                kind,
            });
        }
        for pad in [0, 2] {
            links.apply(ToUi::PadDown { pad, velocity: 100 });
            links.apply(ToUi::PadUp(pad));
        }
        links.take_actions();
        out.push(("sample-play-links", links));
    }
    // The Play tab with the pads playing 16 Levels of the pad, by tune.
    {
        use mpfree_sampler_ui::play_tab::{Row, LIST};
        use mpfree_sampler_ui::widgets::SettingList;
        let mut levels = sample_sampler(fonts());
        levels.on_event(UiEvent {
            widget: mpfree_sampler_ui::wave_tab::BAR.into(),
            kind: EventKind::Tab(mpfree_sampler_ui::play_tab::TAB),
        });
        let index = |row: Row| Row::ALL.iter().position(|r| *r == row).unwrap_or(0);
        for (row, delta) in [(Row::PadsPlay, -1), (Row::LevelsBy, 1)] {
            levels.on_event(UiEvent {
                widget: LIST.into(),
                kind: EventKind::Step {
                    index: index(row),
                    delta,
                },
            });
        }
        if let Some(list) = levels.ui.get_mut::<SettingList>(LIST) {
            list.selected = index(Row::LevelsBy);
        }
        levels.on_event(UiEvent {
            widget: LIST.into(),
            kind: EventKind::Select(index(Row::LevelsBy)),
        });
        levels.take_actions();
        out.push(("sample-play-levels", levels));
    }
    // 16 LEVEL on Play: the 16 Levels panel, by tune, pad 12 held.
    {
        let mut levels = sample_sampler(fonts());
        levels.apply(ToUi::Show(View::Play));
        levels.apply(ToUi::SixteenLevels);
        levels.apply(ToUi::LevelsWheel(1));
        levels.apply(ToUi::PadDown {
            pad: 11,
            velocity: 100,
        });
        levels.take_actions();
        out.push(("play-16-levels", levels));
    }
    // The Chop tab putting four slices on keys.
    {
        use mpfree_sampler_ui::chop_tab::{Row, LIST};
        let mut keys = sample_sampler(fonts());
        keys.on_event(UiEvent {
            widget: mpfree_sampler_ui::wave_tab::BAR.into(),
            kind: EventKind::Tab(mpfree_sampler_ui::chop_tab::TAB),
        });
        let index = |row: Row| Row::ALL.iter().position(|r| *r == row).unwrap_or(0);
        for (row, delta) in [
            (Row::Method, 1),
            (Row::Method, 1),
            (Row::Amount, -4),
            (Row::Put, 1i32),
        ] {
            for _ in 0..delta.abs() {
                keys.on_event(UiEvent {
                    widget: LIST.into(),
                    kind: EventKind::Step {
                        index: index(row),
                        delta: delta.signum(),
                    },
                });
            }
        }
        keys.take_actions();
        out.push(("sample-chop-keys", keys));
    }
    // The Chop tab putting every eighth on pads with a phrase.
    {
        use mpfree_sampler_ui::chop_tab::{Row, LIST};
        use mpfree_sampler_ui::widgets::SettingList;
        let mut phrase = sample_sampler(fonts());
        phrase.on_event(UiEvent {
            widget: mpfree_sampler_ui::wave_tab::BAR.into(),
            kind: EventKind::Tab(mpfree_sampler_ui::chop_tab::TAB),
        });
        let index = |row: Row| Row::ALL.iter().position(|r| *r == row).unwrap_or(0);
        for (row, delta) in [(Row::Method, 1), (Row::Amount, -1), (Row::Put, -1i32)] {
            phrase.on_event(UiEvent {
                widget: LIST.into(),
                kind: EventKind::Step {
                    index: index(row),
                    delta,
                },
            });
        }
        phrase.apply(ToUi::SampleQLink(3, -16));
        if let Some(list) = phrase.ui.get_mut::<SettingList>(LIST) {
            list.selected = index(Row::Put);
        }
        phrase.on_event(UiEvent {
            widget: LIST.into(),
            kind: EventKind::Select(index(Row::Put)),
        });
        phrase.take_actions();
        out.push(("sample-chop-phrase", phrase));
    }
    // Live chop on the same pad: Start pressed, the break playing halfway,
    // and four pads hit so far.
    {
        use mpfree_sampler_ui::chop_tab::{Row, BUTTONS, LIST};
        let mut live = sample_sampler(fonts());
        live.on_event(UiEvent {
            widget: mpfree_sampler_ui::wave_tab::BAR.into(),
            kind: EventKind::Tab(mpfree_sampler_ui::chop_tab::TAB),
        });
        live.on_event(UiEvent {
            widget: LIST.into(),
            kind: EventKind::Step {
                index: Row::ALL.iter().position(|r| *r == Row::Method).unwrap_or(0),
                delta: -1,
            },
        });
        live.on_event(UiEvent {
            widget: BUTTONS.into(),
            kind: EventKind::Tab(0),
        });
        let (project, _) = mpfree_sampler_ui::sample::project();
        if let Some(r) = project
            .pad(mpfree_sampler_ui::sample::PAD)
            .and_then(|p| p.layers.first())
            .map(|l| (l.region.start, l.region.end))
        {
            let (start, end) = r;
            let len = end - start;
            let mut heads = Playheads {
                kit: 0,
                len: 1,
                marking: true,
                marks_made: 4,
                ..Playheads::default()
            };
            heads.pad[0] = mpfree_sampler_ui::sample::PAD as u8;
            heads.frame[0] = (start + len * 9 / 16) as u32;
            for (i, at) in [1u64, 3, 5, 8].into_iter().enumerate() {
                heads.marks[i] = (start + len * at / 16) as u32;
            }
            live.apply(ToUi::SamplePlayheads(heads));
        }
        live.take_actions();
        out.push(("sample-chop-live", live));
    }
    let mut lab = sample_player(fonts());
    lab.apply(ToUi::Show(View::Prefs));
    lab.apply(ToUi::TouchLab);
    lab.set_touch_status("touch-reader: seeded=1 slot=0 fingers=0 resyncs=0 fallbacks=0 drained=0 contacts=4 events=212");
    out.push(("touchlab", lab));
    let mut cal = sample_player(fonts());
    cal.apply(ToUi::Show(View::Prefs));
    cal.apply(ToUi::CalibrateTouch);
    // The top-left corner done; top-right asked for next.
    cal.touch(1, Touch::Down, 70, 70);
    cal.touch(1, Touch::Up, 70, 70);
    out.push(("calibrate", cal));
    // NOTE REPEAT held with the closed hat: the quick band.
    let mut quick = quiet_player(fonts());
    let mut app = sample_app();
    let mut fx = app.button(NOTE_REPEAT, true, 0.0);
    fx.extend(app.pad_down(2, 100));
    apply_effects(&mut quick, fx);
    quick.apply(ToUi::PadDown {
        pad: 2,
        velocity: 100,
    });
    run_arp(&mut quick, &mut app, 5 * 5513 + 2000);
    out.push(("arp-quick-band", quick));
    // The Arp panel (NOTE REPEAT + MENU), Arp mode over pads 1 3 5, latched.
    let mut panel = quiet_player(fonts());
    let mut app = sample_app();
    let mut fx = app.button(NOTE_REPEAT, true, 0.0);
    fx.extend(app.button(MENU, true, 0.0));
    fx.extend(app.button(MENU, false, 0.0));
    fx.extend(app.button(NOTE_REPEAT, false, 0.0));
    fx.extend(app.button(PLUS, true, 0.0));
    fx.extend(app.arp_action(ArpAction::Latch));
    fx.extend(app.qlink(3, 4, None).unwrap_or_default());
    for pad in [0, 2, 4] {
        fx.extend(app.pad_down(pad, 100));
    }
    for pad in [0, 2, 4] {
        app.pad_up(pad);
    }
    apply_effects(&mut panel, fx);
    // The pad the last step played is lit, the latched ones glow.
    for pad in [0, 2, 4] {
        panel.apply(ToUi::PadFlash { pad, velocity: 100 });
    }
    if let Some(g) = panel
        .ui
        .get_mut::<MiniPads>(play_page::PADS)
        .map(|m| &mut m.grid)
    {
        for pad in [0, 2, 4] {
            g.pads[pad].level = 0.35;
        }
        g.pads[2].level = 1.0;
    }
    run_arp(&mut panel, &mut app, 3 * 5513 + 2000);
    out.push(("arp-panel", panel));
    // The pattern workspace, recording on the pads, 3 of 8 steps written.
    let mut ws = quiet_player(fonts());
    let mut app = sample_app();
    let mut fx = app.toggle_panel();
    fx.extend(app.arp_action(ArpAction::Button(
        mpfree_arp_ui::view::PatternButton::Record,
    )));
    for (pad, v) in [(0, 100), (2, 120), (15, 90)] {
        fx.extend(app.pad_down(pad, v));
        app.pad_up(pad);
    }
    apply_effects(&mut ws, fx);
    out.push(("arp-workspace-record", ws));
    out
}

fn button_play() -> u8 {
    mpfree_controller::mpc_one::button::PLAY
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_touch::mt::InputEvent;
    use std::io::ErrorKind;

    /// Under each track's name on Arrange: what it plays ("Kit" until it
    /// has its own instrument), kept as the time engine's news comes in;
    /// a tap on that line asks for the track's instrument.
    #[test]
    fn each_track_says_what_it_plays_and_a_tap_on_it_opens_its_instrument() {
        use mpfree_time_ui::widgets::ClipGrid;
        let mut p = sample_clips(Fonts::embedded()).unwrap();
        let sounds = |p: &Player| {
            p.ui.get::<ClipGrid>(time_view::CLIPS)
                .unwrap()
                .tracks
                .iter()
                .map(|t| t.sound.clone())
                .collect::<Vec<_>>()
        };
        assert!(sounds(&p).iter().all(|s| s == "Kit"), "{:?}", sounds(&p));
        let mut s: TrackSounds = [None; crate::instruments::TRACKS];
        s[2] = Some(crate::instruments::Held {
            id: "mpfree.fm",
            name: "FM",
        });
        p.apply(ToUi::TrackSounds(Box::new(s)));
        assert_eq!(sounds(&p)[..3], ["Kit", "Kit", "FM"]);
        // The time engine's next state keeps them.
        let mut st = p.time.clone();
        for t in &mut st.tracks {
            t.sound.clear();
        }
        p.apply(ToUi::Time(Box::new(st)));
        assert_eq!(sounds(&p)[..3], ["Kit", "Kit", "FM"]);
        let grid = p.ui.rect_of(time_view::CLIPS).unwrap();
        let line =
            p.ui.get::<ClipGrid>(time_view::CLIPS)
                .unwrap()
                .sound_rect(grid, 2)
                .unwrap();
        let (x, y) = (line.x + line.w / 2, line.y + line.h / 2);
        let down = p.touch(1, Touch::Down, x, y);
        assert_eq!(
            down.or(p.touch(1, Touch::Up, x, y)),
            Some(FromUi::TrackInstrument(2))
        );
    }

    #[test]
    fn the_clips_page_shows_the_engine_and_turns_taps_into_actions() {
        use mpfree_time_ui::widgets::{BarButton, ClipGrid, TransportBar};
        use mpfree_transport::Command;
        let mut p = sample_clips(Fonts::embedded()).unwrap();
        assert_eq!(p.page(), CLIPS_PAGE);
        // Recorded from the pads: two bars of kick, snare and hats.
        assert_eq!(
            p.time
                .done
                .iter()
                .map(|t| (t.number, t.added))
                .collect::<Vec<_>>(),
            vec![(1, 26)]
        );
        assert_eq!(
            (p.time.armed, p.time.launch(0).map(|l| l.slot)),
            (Some(1), Some(0))
        );
        let grid = p.ui.rect_of(time_view::CLIPS).unwrap();
        let cell = |p: &Player, track, slot| {
            let r =
                p.ui.get::<ClipGrid>(time_view::CLIPS)
                    .unwrap()
                    .cell_rect(grid, track, slot)
                    .unwrap();
            (r.x + r.w / 2, r.y + r.h / 2)
        };
        let tap = |p: &mut Player, (x, y): (i32, i32)| {
            let down = p.touch(1, Touch::Down, x, y);
            down.or(p.touch(1, Touch::Up, x, y))
        };
        let empty = cell(&p, 1, 0);
        assert_eq!(
            tap(&mut p, empty),
            Some(FromUi::Time(TimeAction::NewClip { track: 1, slot: 0 }))
        );
        let playing = cell(&p, 0, 0);
        assert_eq!(
            tap(&mut p, playing),
            Some(FromUi::Time(TimeAction::StopClip(0)))
        );
        // The bar knows its buttons once drawn, as on the screen.
        p.ui.paint(&mut SoftCanvas::new(1280, 800));
        let play =
            TransportBar::button_rect(p.ui.rect_of(time_view::TRANSPORT).unwrap(), BarButton::Play)
                .unwrap();
        assert_eq!(
            tap(&mut p, (play.x + play.w / 2, play.y + play.h / 2)),
            Some(FromUi::Time(TimeAction::Transport(Command::Play)))
        );
        // The wheel steps the selected record setting: the armed track first.
        p.apply(ToUi::ClipsWheel {
            delta: 1,
            scroll: false,
        });
        assert_eq!(
            p.take_actions(),
            vec![FromUi::Time(TimeAction::Arm(Some(2)))]
        );
        // SHIFT + wheel scrolls the grid.
        p.apply(ToUi::ClipsWheel {
            delta: 1,
            scroll: true,
        });
        assert_eq!(
            p.ui.get::<ClipGrid>(time_view::CLIPS).unwrap().title,
            "Clips · slots 2–6 of 16"
        );
        assert!(p.take_actions().is_empty());
        // The morph bar under the grid: Save stores the A row (the
        // crossfader is at A).
        let morph = p.ui.rect_of(time_view::MORPH).unwrap();
        let save = mpfree_time_ui::widgets::MorphBar::parts(morph).3[2];
        assert_eq!(
            tap(&mut p, (save.x + save.w / 2, save.y + save.h / 2)),
            Some(FromUi::Time(TimeAction::SaveScene(0)))
        );
        // The Song view in the grid's place: its Clips button goes back.
        p.time.song_view = true;
        p.time_page.update(&mut p.ui, &p.time);
        p.ui.paint(&mut SoftCanvas::new(1280, 800));
        let song = p.ui.rect_of(time_view::SONG).unwrap();
        let back =
            p.ui.get::<mpfree_time_ui::song::SongLanes>(time_view::SONG)
                .unwrap()
                .button_rects(song)[0];
        assert_eq!(
            tap(&mut p, (back.x + back.w / 2, back.y + back.h / 2)),
            Some(FromUi::Time(TimeAction::SongView(false)))
        );
        // Edit › on the grid opens the note editor.
        use mpfree_time_ui::editor::{EditAction, Editor, NoteGrid};
        p.time.song_view = false;
        p.time_page.update(&mut p.ui, &p.time);
        p.ui.paint(&mut SoftCanvas::new(1280, 800));
        let clips = p.ui.rect_of(time_view::CLIPS).unwrap();
        let edit =
            p.ui.get::<ClipGrid>(time_view::CLIPS)
                .unwrap()
                .edit_rect(clips);
        assert_eq!(
            tap(&mut p, (edit.x + edit.w / 2, edit.y + edit.h / 2)),
            Some(FromUi::Time(TimeAction::Edit(EditAction::Open)))
        );
        // Open, the piano roll takes the grid's place: a tap on a note
        // picks it, and TC corrects the timing.
        let mut clip = mpfree_seq::Clip::new("Beat", 3840);
        clip.notes = vec![
            mpfree_seq::Note::new(0, 240, 0, 100),
            mpfree_seq::Note::new(970, 240, 1, 90),
        ];
        p.time.edit = Some(Box::new(time_view::EditInfo {
            editor: Editor::new(0, 0, &clip),
            clip,
            track_name: "Track 1".into(),
            bar: 3840,
        }));
        p.time_page.update(&mut p.ui, &p.time);
        p.ui.paint(&mut SoftCanvas::new(1280, 800));
        let area = p.ui.rect_of(time_view::NOTES).unwrap();
        let grid = p.ui.get::<NoteGrid>(time_view::NOTES).unwrap();
        let note = grid.note_rect(area, 1).unwrap();
        let tc = grid.button_rects(area)[7];
        assert_eq!(
            tap(&mut p, (note.x + note.w / 2, note.y + note.h / 2)),
            Some(FromUi::Time(TimeAction::Edit(EditAction::Note(1))))
        );
        assert_eq!(
            tap(&mut p, (tc.x + tc.w / 2, tc.y + tc.h / 2)),
            Some(FromUi::Time(TimeAction::Edit(EditAction::Button(7))))
        );
        // The list and the tracker have hint lines of their own, which say
        // SHIFT + wheel scrolls them.
        let mut st = p.time.clone();
        if let Some(e) = st.edit.as_mut() {
            e.editor.lens = Lens::Tracker;
        }
        p.apply(ToUi::Time(Box::new(st)));
        let hint = &p.ui.get::<Label>("hint").unwrap().text;
        assert_eq!(hint, time_view::EDIT_TRACKER_HINT);
        assert!(hint.contains("SHIFT + wheel: scroll"));
    }

    /// Jay, 12 Nov 08:11Z: Record has a red dot, Stop a square and Play a
    /// triangle. The Loop view's buttons, left to right, do their jobs, and
    /// the Record button draws its dot in the theme's red.
    #[test]
    fn the_loop_strips_buttons_carry_their_symbols_and_play_and_stop_the_transport() {
        use crate::looper_page::{LOOP_STRIP, STRIP_BUTTONS};
        let mut p = sample_player(Fonts::embedded());
        let tap = |p: &mut Player, (x, y): (i32, i32)| {
            let down = p.touch(1, Touch::Down, x, y);
            down.or(p.touch(1, Touch::Up, x, y))
        };
        let card = p.ui.rect_of(play_page::TRANSPORT).unwrap();
        let (_, segs) = PlayTransport::switch(card);
        tap(&mut p, (segs[2].x + 10, segs[2].y + 10));
        sample_loop(&mut p);
        let mut canvas = SoftCanvas::new(SCREEN.0, SCREEN.1);
        p.ui.paint(&mut canvas);
        let strip = p.ui.rect_of(LOOP_STRIP).unwrap();
        // The buttons' row is the strip's bottom 44 px, inside its panel.
        let y = strip.bottom() - 30;
        let mut got: Vec<FromUi> = Vec::new();
        for x in (strip.x..strip.right()).step_by(6) {
            if let Some(m) = tap(&mut p, (x, y)) {
                if got.last() != Some(&m) {
                    got.push(m);
                }
            }
        }
        let transport = |c| FromUi::Time(TimeAction::Transport(c));
        use crate::looper_page::{B_CLEAR, B_MUTE, B_OVERDUB, B_RECORD, B_REDO, B_UNDO};
        assert_eq!(STRIP_BUTTONS.len(), 8);
        assert_eq!(
            got,
            [
                FromUi::Looper(B_RECORD),
                FromUi::Looper(B_OVERDUB),
                transport(mpfree_transport::Command::Play),
                transport(mpfree_transport::Command::Stop),
                FromUi::Looper(B_UNDO),
                FromUi::Looper(B_REDO),
                FromUi::Looper(B_MUTE),
                FromUi::Looper(B_CLEAR),
            ]
        );
        // The Record button (the row's first eighth) has the red dot.
        let red = p.ui.theme.bad;
        let w = strip.w / 8;
        let reds = (strip.x..strip.x + w)
            .flat_map(|x| (strip.bottom() - 44..strip.bottom()).map(move |y| (x, y)))
            .filter(|&(x, y)| canvas.pixel(x, y) == red)
            .count();
        assert!(reds > 40, "{reds} red pixels in the Record button");
    }

    /// Mockup H: a speaker beside each loop layer's name; a tap mutes
    /// that layer (by its id, for the looper), and the next view shows it
    /// crossed out and grey.
    #[test]
    fn a_loop_layers_speaker_mutes_it() {
        use crate::looper_page::{LoopStrip, LOOP_STRIP};
        let mut p = sample_player(Fonts::embedded());
        let tap = |p: &mut Player, (x, y): (i32, i32)| {
            let down = p.touch(1, Touch::Down, x, y);
            down.or(p.touch(1, Touch::Up, x, y))
        };
        let card = p.ui.rect_of(play_page::TRANSPORT).unwrap();
        let (_, segs) = PlayTransport::switch(card);
        tap(&mut p, (segs[2].x + 10, segs[2].y + 10));
        sample_loop(&mut p);
        p.ui.paint(&mut SoftCanvas::new(SCREEN.0, SCREEN.1));
        let strip = p.ui.rect_of(LOOP_STRIP).unwrap();
        // The third layer's speaker: the strip's heading, right of the
        // names; find it by tapping along the heading.
        let y = strip.y + 19;
        let mut hits = Vec::new();
        for x in (strip.x..strip.x + strip.w / 2).step_by(4) {
            if let Some(FromUi::LoopLayer { id, command }) = tap(&mut p, (x, y)) {
                if hits.last().map(|h: &(i32, u8, _)| h.1) != Some(id) {
                    hits.push((x, id, command));
                }
            }
        }
        let (_, view) = recorded_loop();
        let ids: Vec<u8> = (0..3).map(|k| view.layers[k].unwrap().id).collect();
        assert_eq!(
            hits.iter().map(|h| (h.1, h.2)).collect::<Vec<_>>(),
            ids.iter()
                .map(|&id| (id, mpfree_looper::LayerCommand::Mute(true)))
                .collect::<Vec<_>>(),
            "one speaker per layer, in order"
        );
        // The looper says the third is muted: the strip crosses it out.
        let mut v = view.clone();
        if let Some(l) = v.layers[2].as_mut() {
            l.muted = true;
            l.audible = false;
        }
        p.apply(ToUi::Looper(Box::new(v)));
        let st = p.ui.get::<LoopStrip>(LOOP_STRIP).unwrap();
        assert_eq!(st.muted, [false, false, true]);
        assert_eq!(st.heard, [true, true, false]);
        // Its speaker now asks to unmute.
        assert_eq!(
            tap(&mut p, (hits[2].0, y)),
            Some(FromUi::LoopLayer {
                id: ids[2],
                command: mpfree_looper::LayerCommand::Mute(false),
            })
        );
    }

    /// MAIN steps the Play page's views; STEP SEQ goes to Beats; SAMPLE
    /// EDIT opens the Sample tab's editors; PROG EDIT the Pads tab's Pad
    /// view.
    #[test]
    fn the_mode_buttons_pick_play_views_and_sample_editors() {
        let mut p = sample_player(Fonts::embedded());
        assert_eq!(p.play_view(), PlayView::All);
        p.apply(ToUi::PlayView(None));
        assert_eq!(p.play_view(), PlayView::Beats);
        p.apply(ToUi::PlayView(None));
        assert_eq!(p.play_view(), PlayView::Loop);
        p.apply(ToUi::PlayView(None));
        assert_eq!(p.play_view(), PlayView::All, "after Loop comes All again");
        let mut p = sample_sampler(Fonts::embedded());
        assert_eq!(p.page(), sample_page::PAGE);
        p.apply(ToUi::SampleEditor(sample_page::Editor::Chop));
        assert_eq!(p.page(), sample_page::CHOP_PAGE, "the page follows at once");
        p.apply(ToUi::SampleEditor(sample_page::Editor::Wave));
        assert_eq!(p.page(), sample_page::PAGE);
        // PROG EDIT, as the audio loop sends it with a kit on the pads.
        p.apply(ToUi::PadsView(mpfree_sampler_ui::pads_tab::PAD_VIEW));
        p.apply(ToUi::Show(View::Pads));
        assert_eq!(p.page(), sample_page::PADS_PAGE);
    }

    /// The Looper tab (mockup J): a loop tapped is picked, a layer's
    /// speaker works it by its id, a Q-Link turns the picked layer, and a
    /// setting steps the looper's option.
    #[test]
    fn the_looper_tab_picks_loops_and_works_layers() {
        use crate::looper_tab::{LoopEditor, LoopList, LoopSettings, EDITOR, LOOPS, RECORDING};
        let mut p = sample_looper(Fonts::embedded()).unwrap();
        p.ui.paint(&mut SoftCanvas::new(SCREEN.0, SCREEN.1));
        assert!(p.take_actions().is_empty());
        let tap = |p: &mut Player, (x, y): (i32, i32)| {
            let down = p.touch(1, Touch::Down, x, y);
            down.or(p.touch(1, Touch::Up, x, y))
        };
        // Loop 2, then a new loop (loop 3, the first empty one).
        let r = p.ui.rect_of(LOOPS).unwrap();
        let parts = p.ui.get::<LoopList>(LOOPS).unwrap().parts(r);
        let rows = parts.rows;
        assert_eq!(rows.len(), 4);
        assert_eq!(
            tap(&mut p, (rows[1].x + 10, rows[1].y + 10)),
            Some(FromUi::LoopPick(1))
        );
        let new = parts.new.unwrap();
        assert_eq!(
            tap(&mut p, (new.x + 10, new.y + 10)),
            Some(FromUi::LoopPick(2))
        );
        // Loop 2's speaker (it is muted) lets it play, without picking it;
        // Clear loop clears the picked loop.
        let (sx, sy) = crate::looper_tab::LoopList::speaker_at(rows[1]);
        assert_eq!(tap(&mut p, (sx, sy)), Some(FromUi::LoopMute(Some(1))));
        let clear = parts.clear.expect("loop 1 has sound");
        assert_eq!(
            tap(&mut p, (clear.x + 10, clear.y + 10)),
            Some(FromUi::LoopCommand(mpfree_looper::Command::Clear))
        );
        // The chop's speaker (it is muted) unmutes it by its id.
        let (_, view) = recorded_loop();
        let ids: Vec<u8> = (0..3).map(|k| view.layers[k].unwrap().id).collect();
        let r = p.ui.rect_of(EDITOR).unwrap();
        let e = p.ui.get::<LoopEditor>(EDITOR).unwrap();
        assert_eq!(e.picked, 1, "the bass is on the Q-Links");
        let (_, rr) = e.row_rects(LoopEditor::parts(r).2)[2];
        let parts = LoopEditor::row_parts(rr);
        assert_eq!(
            tap(&mut p, parts.speaker),
            Some(FromUi::LoopLayer {
                id: ids[2],
                command: mpfree_looper::LayerCommand::Mute(false),
            })
        );
        assert!(p.take_actions().is_empty());
        // The chop is picked now: Q1 turns its level.
        p.apply(ToUi::LooperQLink(0, 2));
        let acts = p.take_actions();
        assert!(
            matches!(
                acts.as_slice(),
                [FromUi::LoopLayer { id, command: mpfree_looper::LayerCommand::Level(_) }]
                    if *id == ids[2]
            ),
            "{acts:?}"
        );
        // Q3 sets how much each overdub fades the old layers.
        p.apply(ToUi::LooperQLink(2, -1));
        assert!(matches!(
            p.take_actions().as_slice(),
            [FromUi::LoopCommand(mpfree_looper::Command::SetFadeOld(_))]
        ));
        // Line input, right half: on.
        let r = p.ui.rect_of(RECORDING).unwrap();
        let rects = p.ui.get::<LoopSettings>(RECORDING).unwrap().row_rects(r);
        let line = rects[5];
        assert_eq!(
            tap(&mut p, (line.right() - 6, line.y + 6)),
            Some(FromUi::LooperSetting(crate::app::Item::LineInput, 1))
        );
        // The hint says what the tab's controls do.
        let hint = &p.ui.get::<Label>("hint").unwrap().text;
        assert!(hint.contains("Tap a loop or a layer"), "{hint}");
    }

    /// UNDO follows the page (Jay, 13 Nov 13:53Z): the loop on the Loop
    /// view, the takes on Beats, and on All whichever changed last.
    /// Jay after live test 3: 32, 64 and 128 steps, with a clear way to
    /// get them. The pattern's head has ‹ › for the page and - + for the
    /// length.
    #[test]
    fn the_patterns_head_turns_pages_and_doubles_or_halves_its_steps() {
        use play_widgets::{HEAD_LONGER, HEAD_NEXT, HEAD_PREV, HEAD_SHORTER};
        let mut p = sample_play(Fonts::embedded(), PlayView::Beats);
        p.take_actions();
        let r = p.ui.rect_of(play_page::STEPS).unwrap();
        let tap = |p: &mut Player, k: usize| {
            let (x, y) = StepGrid::head_at(r, k);
            let a = p.touch(3, Touch::Down, x, y);
            p.touch(3, Touch::Up, x, y);
            a
        };
        let grid = |p: &Player| {
            let g = p.ui.get::<StepGrid>(play_page::STEPS).unwrap();
            (g.page, g.pages, g.length)
        };
        assert_eq!(grid(&p), (0, 1, 16));
        // + asks for twice the steps, repeating the ones there are.
        assert_eq!(
            tap(&mut p, HEAD_LONGER),
            Some(FromUi::Steps(StepAction::Length {
                steps: 32,
                copy: true
            }))
        );
        // One page: › says so in place of the hint.
        assert_eq!(tap(&mut p, HEAD_NEXT), None);
        let hint = |p: &Player| p.ui.get::<Label>("hint").unwrap().text.clone();
        assert!(hint(&p).contains("last page"), "{}", hint(&p));
        // The time engine made it 32 steps: › shows the second page.
        let mut st = p.play.steps.clone();
        let c = st.clip.as_mut().unwrap();
        c.steps = 32;
        for (_, row) in &mut c.rows {
            let first = row.clone();
            row.extend(first);
        }
        p.apply(ToUi::Steps(Box::new(st)));
        assert_eq!(grid(&p), (0, 2, 32));
        assert_eq!(tap(&mut p, HEAD_NEXT), None);
        assert_eq!(grid(&p), (1, 2, 32));
        // A tap on the first step there is step 17 (an empty lane's).
        assert_eq!(
            p.play.tap_step(15, 0),
            Some(StepAction::Tap {
                pad: 15,
                step: 16,
                velocity: play_page::TAP_VELOCITY
            })
        );
        assert_eq!(tap(&mut p, HEAD_PREV), None);
        assert_eq!(grid(&p).0, 0);
        // - halves it, down to 16; at 16 it says why not.
        assert_eq!(
            tap(&mut p, HEAD_SHORTER),
            Some(FromUi::Steps(StepAction::Length {
                steps: 16,
                copy: false
            }))
        );
        let mut st = p.play.steps.clone();
        st.clip.as_mut().unwrap().steps = 16;
        p.apply(ToUi::Steps(Box::new(st)));
        assert_eq!(tap(&mut p, HEAD_SHORTER), None);
        assert!(hint(&p).contains("16 steps is the fewest"), "{}", hint(&p));
    }

    /// Jay after live test 3: hold UNDO to see the page's history and
    /// jump back several steps at once.
    #[test]
    fn holding_undo_opens_the_pages_history_and_a_tap_jumps_several_steps() {
        use crate::app::RowKey;
        use mpfree_time_ui::view::TakeRow;
        use mpfree_ui::history::HistoryPanel;
        let mut p = sample_play(Fonts::embedded(), PlayView::Beats);
        let take = |n: u32| TakeRow {
            number: n,
            captured: false,
            edited: true,
            made: false,
            at: Some((0, 0)),
            added: 1,
            erased: 0,
            deleted: false,
        };
        let mut st = p.time.clone();
        st.done = vec![take(1), take(2), take(3)];
        st.undone = vec![take(5), take(4)];
        p.apply(ToUi::Time(Box::new(st)));
        p.take_actions();
        // A quick press only undoes (the app sent that already).
        p.apply(ToUi::UndoButton(true));
        p.tick(0.3);
        p.apply(ToUi::UndoButton(false));
        p.tick(1.0);
        assert_ne!(p.page(), HISTORY_PAGE);
        // Held: the takes' history, now after take 3, redo's next first.
        let beats = p.page().to_string();
        p.apply(ToUi::UndoButton(true));
        p.tick(0.3);
        p.tick(0.4);
        assert_eq!(p.page(), HISTORY_PAGE);
        assert!(p.take_actions().contains(&FromUi::Overlay(true)));
        assert!(p
            .ui
            .get::<Label>("hint")
            .unwrap()
            .text
            .starts_with("Tap a step"));
        p.apply(ToUi::UndoButton(false));
        let panel = |p: &Player| {
            let h = p.ui.get::<HistoryPanel>(HISTORY).unwrap();
            (h.title.clone(), h.rows.len(), h.now)
        };
        assert_eq!(panel(&p), ("Undo history · Takes".to_string(), 6, 3));
        let rows = p.ui.get::<HistoryPanel>(HISTORY).unwrap().rows.clone();
        assert_eq!(rows[0], "Start: before these takes");
        assert!(rows[3].starts_with("Edit 3") && rows[4].starts_with("Edit 4"));
        // A tap on take 1: two takes undone at once.
        p.ui.paint(&mut SoftCanvas::new(SCREEN.0, SCREEN.1));
        let r = p.ui.rect_of(HISTORY).unwrap();
        let tap = |p: &mut Player, (x, y): (i32, i32)| {
            p.touch(5, Touch::Down, x, y);
            p.touch(5, Touch::Up, x, y);
            p.take_actions()
        };
        let at = |p: &Player, k: usize| {
            let (_, rr) =
                p.ui.get::<HistoryPanel>(HISTORY)
                    .unwrap()
                    .row_rects(r)
                    .into_iter()
                    .find(|(i, _)| *i == k)
                    .unwrap();
            (rr.x + rr.w / 2, rr.y + rr.h / 2)
        };
        let undo = || FromUi::Time(TimeAction::Undo);
        let redo = || FromUi::Time(TimeAction::Redo);
        let at1 = at(&p, 1);
        let acts = tap(&mut p, at1);
        assert_eq!(acts, [undo(), undo()]);
        // The wheel outlines a row and its push goes there: forward to 5.
        p.apply(ToUi::Rows(RowKey::Pick(2)));
        p.apply(ToUi::Rows(RowKey::Step(-1)));
        p.apply(ToUi::Rows(RowKey::Pick(3)));
        p.apply(ToUi::Rows(RowKey::Editing(true)));
        assert_eq!(p.take_actions(), [redo(), redo()]);
        // Close goes back to the page it was opened over.
        let c = HistoryPanel::close_rect(r);
        tap(&mut p, (c.x + 10, c.y + 10));
        assert_eq!(p.page(), beats);
        // The mixer: the app keeps its turns, so the list asks for them.
        p.apply(ToUi::Show(View::Mixer));
        p.take_actions();
        p.apply(ToUi::UndoButton(true));
        p.tick(0.7);
        assert!(p
            .take_actions()
            .contains(&FromUi::UndoHistory(UndoOn::Mixer)));
        p.apply(ToUi::UndoHistory {
            on: UndoOn::Mixer,
            rows: vec![
                "Start".into(),
                "Drums Volume to -6.0 dB".into(),
                "Bass Pan to L 20".into(),
            ],
            now: 2,
        });
        assert_eq!(panel(&p), ("Undo history · Mixer".to_string(), 3, 2));
        let at0 = at(&p, 0);
        let acts = tap(&mut p, at0);
        let back = || FromUi::UndoOn {
            on: UndoOn::Mixer,
            redo: false,
        };
        assert_eq!(acts, [back(), back(), FromUi::UndoHistory(UndoOn::Mixer)]);
        // Another page closes the list.
        p.apply(ToUi::Show(View::Play));
        assert_ne!(p.page(), HISTORY_PAGE);
        assert!(p.take_actions().contains(&FromUi::Overlay(false)));
        // A page with no history says so and opens nothing.
        p.apply(ToUi::Show(View::Prefs));
        p.apply(ToUi::UndoButton(true));
        p.tick(0.7);
        assert_ne!(p.page(), HISTORY_PAGE);
        assert_eq!(
            p.ui.get::<Label>("hint").unwrap().text,
            "Nothing to undo on this page"
        );
    }

    /// The Sample tab's list: the kit's edits, named by their pads; a tap
    /// on Start takes them all back.
    #[test]
    fn the_sample_tabs_history_lists_the_kits_edits() {
        use mpfree_sampler_ui::wave_tab::{Row, REGION};
        use mpfree_ui::history::HistoryPanel;
        let mut p = sample_sampler(Fonts::embedded());
        let row = |r: Row| Row::ALL.iter().position(|k| *k == r).unwrap();
        let warp = |p: &mut Player, delta: i32| {
            p.on_event(UiEvent {
                widget: REGION.into(),
                kind: EventKind::Step {
                    index: row(Row::Warp),
                    delta,
                },
            });
            p.apply(ToUi::SampleLift);
        };
        let warped = |p: &Player| p.sample.state().unwrap().region.warp.is_some();
        assert!(!warped(&p));
        warp(&mut p, 1);
        assert!(warped(&p));
        p.apply(ToUi::UndoButton(true));
        p.tick(0.7);
        assert_eq!(p.page(), HISTORY_PAGE);
        let h = p.ui.get::<HistoryPanel>(HISTORY).unwrap();
        assert!(h.title.starts_with("Undo history · Kit 1"), "{}", h.title);
        assert_eq!(h.rows.len(), 2);
        assert!(h.rows[1].starts_with("Pad A"), "{:?}", h.rows);
        assert_eq!(h.now, 1);
        // The wheel's push on Start: the edit is taken back, the list
        // follows (now on Start, the edit to redo).
        p.apply(ToUi::Rows(crate::app::RowKey::Pick(-5)));
        p.apply(ToUi::Rows(crate::app::RowKey::Editing(true)));
        assert!(!warped(&p));
        p.tick(0.0);
        let h = p.ui.get::<HistoryPanel>(HISTORY).unwrap();
        assert_eq!((h.rows.len(), h.now), (2, 0));
    }

    #[test]
    fn undo_follows_the_page_and_the_play_view() {
        use mpfree_time_ui::view::TakeRow;
        let mut p = sample_play(Fonts::embedded(), PlayView::All);
        p.take_actions();
        let undo = |p: &mut Player, redo: bool| {
            p.apply(ToUi::Undo(redo));
            p.take_actions()
        };
        let looper = |c| vec![FromUi::LoopCommand(c)];
        let takes = |a| vec![FromUi::Time(a)];
        use mpfree_looper::Command as L;
        // All: a take was made last, so the takes.
        let mut st = p.time.clone();
        st.done.push(TakeRow {
            number: 9,
            captured: false,
            edited: true,
            made: false,
            at: Some((0, 0)),
            added: 1,
            erased: 0,
            deleted: false,
        });
        p.apply(ToUi::Time(Box::new(st.clone())));
        assert_eq!(undo(&mut p, false), takes(TimeAction::Undo));
        // The loop's history moved since: the loop, both ways.
        p.apply(ToUi::Looper(Box::default()));
        assert_eq!(undo(&mut p, false), looper(L::Undo));
        assert_eq!(undo(&mut p, true), looper(L::Redo));
        // The same takes again change nothing; a new take does.
        p.apply(ToUi::Time(Box::new(st.clone())));
        assert_eq!(undo(&mut p, false), looper(L::Undo));
        st.undone.push(st.done.pop().unwrap());
        p.apply(ToUi::Time(Box::new(st)));
        assert_eq!(undo(&mut p, true), takes(TimeAction::Redo));
        // The Loop view: the loop; Beats: the takes; Arrange: the takes.
        p.apply(ToUi::PlayView(Some(PlayView::Loop)));
        assert_eq!(undo(&mut p, false), looper(L::Undo));
        p.apply(ToUi::PlayView(Some(PlayView::Beats)));
        assert_eq!(undo(&mut p, false), takes(TimeAction::Undo));
        p.apply(ToUi::Show(View::Arrange));
        assert_eq!(undo(&mut p, false), takes(TimeAction::Undo));
        // REC and OVERDUB on Play: the loop on All and Loop, takes on Beats.
        let rec = |p: &mut Player, overdub: bool| {
            p.apply(ToUi::PlayRec(overdub));
            p.take_actions()
        };
        use crate::looper_page::{B_OVERDUB, B_RECORD};
        p.apply(ToUi::Show(View::Play));
        p.apply(ToUi::PlayView(Some(PlayView::All)));
        assert_eq!(rec(&mut p, false), [FromUi::Looper(B_RECORD)]);
        p.apply(ToUi::PlayView(Some(PlayView::Loop)));
        assert_eq!(rec(&mut p, true), [FromUi::Looper(B_OVERDUB)]);
        p.apply(ToUi::PlayView(Some(PlayView::Beats)));
        assert_eq!(
            rec(&mut p, false),
            takes(TimeAction::Transport(mpfree_transport::Command::Record))
        );
        // The Synth tab and the mixer: their own turns.
        p.apply(ToUi::Show(View::Synth));
        assert_eq!(
            undo(&mut p, true),
            [FromUi::UndoOn {
                on: UndoOn::Synth,
                redo: true
            }]
        );
        p.apply(ToUi::Show(View::Mixer));
        assert_eq!(
            undo(&mut p, false),
            [FromUi::UndoOn {
                on: UndoOn::Mixer,
                redo: false
            }]
        );
        // The UNDO light: the mixer's turns, as the audio thread says.
        let light = |p: &mut Player| {
            p.tick(0.0);
            p.take_actions()
                .into_iter()
                .filter_map(|a| match a {
                    FromUi::UndoReady(r) => Some(r),
                    _ => None,
                })
                .next_back()
        };
        light(&mut p);
        p.apply(ToUi::TurnsReady {
            synth: false,
            mixer: true,
        });
        assert_eq!(light(&mut p), Some(true));
        assert_eq!(light(&mut p), None, "said once");
        p.apply(ToUi::Show(View::Prefs));
        assert_eq!(light(&mut p), Some(false), "Prefs has none");
        // A page with no history of its own says so and does nothing.
        p.apply(ToUi::Show(View::Prefs));
        assert!(undo(&mut p, false).is_empty());
        assert_eq!(
            p.told.as_ref().map(|t| t.0.as_str()),
            Some("Nothing to undo on this page")
        );
    }

    /// ERASE held (Jay after live test 3): the tap deletes what it lands
    /// on, where UNDO brings it back; elsewhere a tap does what it does.
    #[test]
    fn erase_held_deletes_a_clip_a_lanes_steps_or_a_layer() {
        let tap = |p: &mut Player, (x, y): (i32, i32)| {
            let down = p.touch(1, Touch::Down, x, y);
            down.or(p.touch(1, Touch::Up, x, y))
        };
        let erase = |p: &mut Player, on: bool| {
            let mut s = p.status.clone();
            s.erase = on;
            p.apply(ToUi::Status(s));
        };
        let hint = |p: &Player| p.ui.get::<Label>("hint").unwrap().text.clone();
        // Play: a lane's name clears its steps; its mute stays as it was.
        let mut p = sample_play(Fonts::embedded(), PlayView::All);
        p.ui.paint(&mut SoftCanvas::new(SCREEN.0, SCREEN.1));
        erase(&mut p, true);
        assert!(hint(&p).starts_with("ERASE held"), "{}", hint(&p));
        let r = p.ui.rect_of(play_page::STEPS).unwrap();
        let g = p.ui.get::<StepGrid>(play_page::STEPS).unwrap();
        let (lane1, name1) = (g.lanes_shown(r)[1], g.name_at(r, 1));
        let mutes = p.play.levels.pad_mutes;
        assert_eq!(
            tap(&mut p, name1),
            Some(FromUi::Steps(StepAction::ClearPad { pad: lane1 as u8 }))
        );
        assert_eq!(p.play.levels.pad_mutes, mutes, "no mute while erasing");
        // Let go of ERASE: the name mutes again.
        erase(&mut p, false);
        assert!(!hint(&p).starts_with("ERASE held"));
        assert!(matches!(
            tap(&mut p, name1),
            Some(FromUi::PlayFx(PlayFx::PadMute { .. }))
        ));
        // Arrange: a clip is deleted; an empty slot says so and makes none.
        use mpfree_time_ui::widgets::ClipGrid;
        let mut p = sample_clips(Fonts::embedded()).unwrap();
        erase(&mut p, true);
        let grid = p.ui.rect_of(time_view::CLIPS).unwrap();
        let cell = |p: &Player, track, slot| {
            let r =
                p.ui.get::<ClipGrid>(time_view::CLIPS)
                    .unwrap()
                    .cell_rect(grid, track, slot)
                    .unwrap();
            (r.x + r.w / 2, r.y + r.h / 2)
        };
        let playing = cell(&p, 0, 0);
        assert_eq!(
            tap(&mut p, playing),
            Some(FromUi::Time(TimeAction::DeleteClip { track: 0, slot: 0 }))
        );
        let empty = cell(&p, 1, 0);
        assert_eq!(tap(&mut p, empty), None, "no new clip while erasing");
        assert!(hint(&p).contains("empty"), "{}", hint(&p));
        // Looper tab: a layer's row or its speaker deletes it; a loop is
        // left alone (Clear loop has no undo).
        use crate::looper_tab::{LoopEditor, LoopList, EDITOR, LOOPS};
        let mut p = sample_looper(Fonts::embedded()).unwrap();
        p.ui.paint(&mut SoftCanvas::new(SCREEN.0, SCREEN.1));
        erase(&mut p, true);
        let (_, view) = recorded_loop();
        let chop = view.layers[2].unwrap().id;
        let r = p.ui.rect_of(EDITOR).unwrap();
        let e = p.ui.get::<LoopEditor>(EDITOR).unwrap();
        let (_, rr) = e.row_rects(LoopEditor::parts(r).2)[2];
        let parts = LoopEditor::row_parts(rr);
        assert_eq!(
            tap(&mut p, parts.speaker),
            Some(FromUi::LoopLayer {
                id: chop,
                command: mpfree_looper::LayerCommand::Delete,
            })
        );
        let r = p.ui.rect_of(LOOPS).unwrap();
        let row = p.ui.get::<LoopList>(LOOPS).unwrap().parts(r).rows[1];
        assert_eq!(tap(&mut p, (row.x + 10, row.y + 10)), None);
        assert!(hint(&p).contains("Clear"), "{}", hint(&p));
        assert_eq!(p.take_actions(), vec![]);
    }

    #[test]
    fn one_tap_switches_the_play_view_and_the_q_links_follow_it() {
        let mut p = sample_player(Fonts::embedded());
        let tap = |p: &mut Player, (x, y): (i32, i32)| {
            let down = p.touch(1, Touch::Down, x, y);
            down.or(p.touch(1, Touch::Up, x, y))
        };
        let card = p.ui.rect_of(play_page::TRANSPORT).unwrap();
        let (_, segs) = PlayTransport::switch(card);
        let seg = |i: usize| (segs[i].x + segs[i].w / 2, segs[i].y + segs[i].h / 2);
        let names = |p: &Player| -> Vec<String> {
            p.ui.get::<QLinkColumn>(play_page::QLINKS)
                .unwrap()
                .cards
                .iter()
                .map(|c| c.name.clone())
                .collect()
        };
        assert_eq!(names(&p), ["Swing", "Tune", "Filter", "Delay send"]);
        assert_eq!(tap(&mut p, seg(2)), None, "the screen's own choice");
        assert_eq!((p.page(), p.play_view()), ("Play · Loop", PlayView::Loop));
        assert!(p.ui.rect_of(crate::looper_page::LOOP_STRIP).is_some());
        assert_eq!(
            names(&p),
            ["Loop level", "Delay send", "Line level", "Filter"]
        );
        assert_eq!(
            p.ui.get::<PlayTransport>(play_page::TRANSPORT)
                .unwrap()
                .look
                .view,
            2
        );
        // Beats, with Your timing as a strip under the pattern (Jay 13
        // Nov: the Drum view folded into Beats).
        assert_eq!(segs.len(), 3, "All, Beats, Loop: no Drum view");
        assert_eq!(tap(&mut p, seg(1)), None);
        assert_eq!(p.page(), play_page::BEATS_TIMING_PAGE);
        assert!(p.ui.rect_of(play_page::TIMING).is_some());
        assert_eq!(names(&p), ["Swing", "Tune", "Filter", "Delay send"]);
        // Another tab and back: the view is kept.
        p.apply(ToUi::Show(View::Synth));
        p.apply(ToUi::Show(View::Play));
        assert_eq!(p.page(), play_page::BEATS_TIMING_PAGE);
        // Play's options turn the strip off: the pattern has the room.
        let mut s = p.settings.clone();
        s.timing_strip = false;
        p.apply(ToUi::Settings(s));
        assert_eq!(p.page(), "Play · Beats");
        assert!(p.ui.rect_of(play_page::TIMING).is_none());
        // Each view fits under the transport card, left of the master.
        for v in PlayView::ALL {
            tap(&mut p, seg(v.index()));
            let throws = p.ui.rect_of(play_page::THROWS).unwrap();
            assert!(
                throws.y > 168 && throws.bottom() <= 750 && throws.right() <= 1010,
                "{v:?}: {throws:?}"
            );
            assert!(p.ui.rect_of(play_page::PADS).is_none(), "{v:?}: no pads");
        }
        tap(&mut p, seg(0));
        assert_eq!(p.page(), "Play");
        // A drag up a card turns that knob: Swing, on the transport.
        let column = p.ui.rect_of(play_page::QLINKS).unwrap();
        let c = QLinkColumn::card_rects(column)[0];
        let (x, y) = (c.x + 20, c.y + c.h / 2);
        assert_eq!(p.touch(3, Touch::Down, x, y), None);
        assert_eq!(
            p.touch(3, Touch::Move, x, y - 2 * mpfree_ui::qlink::DRAG_DETENT),
            None
        );
        p.touch(3, Touch::Up, x, y);
        let sent = p.take_actions();
        assert!(
            matches!(
                sent.as_slice(),
                [FromUi::Time(TimeAction::Transport(
                    mpfree_transport::Command::SetSwing(mpfree_transport::time::Swing {
                        amount: 52,
                        ..
                    })
                ))]
            ),
            "{sent:?}"
        );
        assert_eq!(
            p.ui.get::<QLinkColumn>(play_page::QLINKS).unwrap().cards[0].value,
            "52 %"
        );
        // The same from the knob itself, through the audio thread.
        p.apply(ToUi::PlayQLink(2, -10));
        assert!(matches!(
            p.take_actions().as_slice(),
            [FromUi::PlayFx(PlayFx::Filter { position, .. })] if *position == -20.0
        ));
        assert_eq!(
            p.ui.get::<QLinkColumn>(play_page::QLINKS).unwrap().cards[2].value,
            "LP 6.6 kHz"
        );
        // A finger on the knob lights its card.
        p.apply(ToUi::QLinkTouch(2, true));
        assert!(p.ui.get::<QLinkColumn>(play_page::QLINKS).unwrap().touched[2]);
        p.apply(ToUi::QLinkTouch(2, false));
        assert!(!p.ui.get::<QLinkColumn>(play_page::QLINKS).unwrap().touched[2]);
        // The master bar sets the master fader in dB, and shows the one the app keeps.
        let master = p.ui.rect_of(play_page::MASTER).unwrap();
        let top = MasterBar::track(master);
        assert_eq!(tap(&mut p, (top.x + 4, top.y)), Some(FromUi::LevelDb(6.0)));
        p.apply(ToUi::Level { db: -6.0 });
        let m = p.ui.get::<MasterBar>(play_page::MASTER).unwrap();
        assert_eq!((m.text.as_str(), m.fader_db), ("-6.0", -6.0));
    }

    /// Jay after live test 3: several throws at once, a finger on each.
    /// Each finger holds its own throw, and lifting one leaves the others.
    #[test]
    fn two_fingers_hold_two_throws_at_once() {
        let mut p = sample_play(Fonts::embedded(), PlayView::All);
        p.play.throw(2, false);
        p.ui.paint(&mut SoftCanvas::new(SCREEN.0, SCREEN.1));
        let r = p.ui.rect_of(play_page::THROWS).unwrap();
        let cells = p.ui.get::<Throws>(play_page::THROWS).unwrap().cells(r);
        let (a, b) = (cells[0], cells[1]);
        let on = |p: &Player| -> Vec<bool> {
            let t = p.ui.get::<Throws>(play_page::THROWS).unwrap();
            t.items.iter().take(2).map(|i| i.on).collect()
        };
        let throw = |a: Option<FromUi>| match a {
            Some(FromUi::PlayFx(PlayFx::Throw { fx, down })) => Some((fx, down)),
            _ => None,
        };
        use crate::mixing::ThrowFx::{Delay, Reverb};
        let first = p.touch(11, Touch::Down, a.x + 8, a.y + 8);
        assert_eq!(throw(first), Some((Delay, true)));
        let second = p.touch(12, Touch::Down, b.x + 8, b.y + 8);
        assert_eq!(throw(second), Some((Reverb, true)));
        assert_eq!(on(&p), [true, true]);
        // The first finger slides a little and lifts; the second holds on.
        assert_eq!(throw(p.touch(11, Touch::Move, a.x + 14, a.y + 10)), None);
        assert_eq!(
            throw(p.touch(11, Touch::Up, a.x + 14, a.y + 10)),
            Some((Delay, false))
        );
        assert_eq!(on(&p), [false, true]);
        assert_eq!(
            throw(p.touch(12, Touch::Up, b.x + 8, b.y + 8)),
            Some((Reverb, false))
        );
        assert_eq!(on(&p), [false, false]);
    }

    #[test]
    fn the_pattern_throws_and_xy_pad_reach_the_engines() {
        let mut p = sample_play(Fonts::embedded(), PlayView::All);
        p.play.throw(2, false);
        p.ui.paint(&mut SoftCanvas::new(SCREEN.0, SCREEN.1));
        // The pattern: the programmed beat's lanes, the clap muted.
        let g = p.ui.get::<StepGrid>(play_page::STEPS).unwrap();
        assert_eq!(g.title, "Pattern 1 · Track 1");
        let r = p.ui.rect_of(play_page::STEPS).unwrap();
        assert_eq!(
            g.lanes_shown(r),
            [0, 2, 4, 5, 7, 10],
            "the sounds with notes, in pad order"
        );
        assert!(g.lanes.iter().find(|l| l.pad == 7).unwrap().muted);
        assert_eq!(g.lanes[0].steps[0], 120, "the kick on the one");
        assert_eq!(g.play, Some(1));
        // A step tapped: on, for the sequencer.
        let tap = |p: &mut Player, (x, y): (i32, i32)| {
            let down = p.touch(1, Touch::Down, x, y);
            down.or(p.touch(1, Touch::Up, x, y))
        };
        let cell = |p: &Player, lane: usize, step: usize| {
            let r = p.ui.rect_of(play_page::STEPS).unwrap();
            let (_, body) = mpfree_ui::draw::card_head(r);
            let grid = body.take_left(118).1;
            let cw = (grid.w - 3 * 10 - 15 * 4) / 16;
            let rh = ((grid.h - 5 * 5) / 6).min(44);
            (
                grid.x + step as i32 * (cw + 4) + (step as i32 / 4) * 6 + cw / 2,
                grid.y + lane as i32 * (rh + 5) + rh / 2,
            )
        };
        let at = cell(&p, 0, 1);
        assert_eq!(
            tap(&mut p, at),
            Some(FromUi::Steps(StepAction::Tap {
                pad: 0,
                step: 1,
                velocity: play_page::TAP_VELOCITY
            }))
        );
        // A speaker: the pad's mute, for the mixer.
        let (_, body) = mpfree_ui::draw::card_head(r);
        let names = body.take_left(118).0;
        let (_, y) = cell(&p, 1, 0);
        assert_eq!(
            tap(&mut p, (names.right() - 14, y)),
            Some(FromUi::PlayFx(PlayFx::PadMute { pad: 2, mute: true }))
        );
        // A throw held and let go: the performance effect on the master.
        let throws = p.ui.rect_of(play_page::THROWS).unwrap();
        let t0 = p.ui.get::<Throws>(play_page::THROWS).unwrap().cells(throws)[0];
        let down = p.touch(5, Touch::Down, t0.x + 5, t0.y + 5);
        assert!(
            matches!(
                down,
                Some(FromUi::PlayFx(PlayFx::Throw {
                    fx: crate::mixing::ThrowFx::Delay,
                    down: true
                }))
            ),
            "{down:?}"
        );
        assert!(p.ui.get::<Throws>(play_page::THROWS).unwrap().items[0].on);
        let up = p.touch(5, Touch::Up, t0.x + 5, t0.y + 5);
        assert!(matches!(
            up,
            Some(FromUi::PlayFx(PlayFx::Throw { down: false, .. }))
        ));
        // Drop drums mutes the kicks, snares and hats while held.
        assert_eq!(p.play.drum_pads(), 0x00FF, "pads 1 to 8 of Boom Bap");
        // The XY pad: where the finger is, and on.
        let xy = p.ui.rect_of(play_page::XY).unwrap();
        let first = p.touch(6, Touch::Down, xy.x + xy.w / 2, xy.bottom() - 30);
        assert!(
            matches!(first, Some(FromUi::PlayFx(PlayFx::Xy { x, y, down: true })) if (x - 0.5).abs() < 0.02 && y < 0.2),
            "{first:?}"
        );
    }

    #[test]
    fn beats_picks_a_step_for_the_q_links() {
        let mut p = sample_play(Fonts::embedded(), PlayView::Beats);
        let names: Vec<String> =
            p.ui.get::<QLinkColumn>(play_page::QLINKS)
                .unwrap()
                .cards
                .iter()
                .map(|c| format!("{} {}", c.name, c.value))
                .collect();
        assert_eq!(
            names,
            ["Velocity 96", "Length 1/32", "Nudge -4 ms", "Chance 100 %"]
        );
        assert_eq!(
            p.ui.get::<QLinkColumn>(play_page::QLINKS).unwrap().cards[0].scope,
            "STEP 7 · KICK"
        );
        // Velocity down 4 detents: on screen at once, and to the sequencer as one undo.
        p.apply(ToUi::PlayQLink(0, -4));
        let sent = p.take_actions();
        assert!(
            matches!(
                sent.as_slice(),
                [FromUi::Steps(StepAction::Change {
                    pad: 0,
                    step: 6,
                    change: mpfree_seq::steps::Change::Velocity(88)
                })]
            ),
            "{sent:?}"
        );
        assert_eq!(
            p.ui.get::<QLinkColumn>(play_page::QLINKS).unwrap().cards[0].value,
            "88"
        );
    }

    #[test]
    fn the_beats_view_shows_your_timing() {
        let p = sample_play(Fonts::embedded(), PlayView::Beats);
        assert_eq!(p.page(), play_page::BEATS_TIMING_PAGE);
        let t = p.ui.get::<YourTiming>(play_page::TIMING).unwrap();
        assert_eq!(
            t.lanes.iter().map(|l| l.1.as_str()).collect::<Vec<_>>(),
            ["Kick", "Snare", "Hat", "Chop 1"]
        );
        let avg = t.average_ms.unwrap();
        assert!((-10.0..-2.0).contains(&avg), "about 6 ms early: {avg}");
        // Last time round's hits, ahead of the playhead, are drawn faint.
        let now = t.now.unwrap();
        for (_, _, _, hits) in &t.lanes {
            for h in hits {
                assert_eq!(h.old, h.at > now + 0.5, "{h:?} now {now}");
            }
        }
        assert!(t.spread_ms.unwrap() > 1.0 && t.histogram.iter().sum::<u32>() > 10);
        assert!(t.now.is_some());
    }

    #[test]
    fn track_mix_opens_the_mixer_and_the_q_links_turn_the_selected_strip() {
        use mpfree_mixer_ui::{MixAction, MixState, MixView, MIXER_PAGE};
        let mut p = sample_player(Fonts::embedded());
        let mut app = sample_app();
        app.map.learn(crate::app::mapper::Job::TrackMix, Some(70));
        // TRACK MIX: the mixer over Play, on Track Mix; the icon lights.
        let fx = app.button(70, true, 0.0);
        app.button(70, false, 0.0);
        assert_eq!(
            fx.first(),
            Some(&crate::app::Effect::Mixer(Some(MixView::Tracks))),
            "{fx:?}"
        );
        apply_effects(&mut p, fx);
        assert_eq!(p.page(), MIXER_PAGE);
        assert_eq!(p.ui.get::<Header>("header").unwrap().mixer_icon, Some(true));
        assert!(
            p.ui.rect_of(play_page::QLINKS).is_some(),
            "the Q-Link cards stay beside the mixer"
        );
        // The audio thread's state, with a strip selected: the cards name it.
        let mut m = mpfree_mixer_ui::sample::song_mixer();
        let mut st = MixState::from_mixer(&mut m, MixView::Tracks, &mpfree_mixer_ui::sample::name);
        st.selected = Some(mpfree_fx::StripId::Track(1));
        p.apply(ToUi::Mixer(Box::new(st)));
        let q = p.ui.get::<QLinkColumn>(play_page::QLINKS).unwrap();
        assert_eq!(
            (q.cards[0].name.as_str(), q.cards[1].name.as_str()),
            ("Volume", "Pan")
        );
        assert!(q.cards[1].bipolar && !q.cards[0].scope.is_empty());
        // A Q-Link (or the wheel) moves the strip's volume on the panel and tells the mixer.
        let fx = app.wheel(2);
        apply_effects(&mut p, fx);
        let sent = p.take_actions();
        assert!(
            matches!(
                sent.as_slice(),
                [FromUi::Mixer(MixAction::Set {
                    strip: mpfree_fx::StripId::Track(1),
                    ..
                })]
            ),
            "{sent:?}"
        );
        // TRACK MIX again closes it, back to Play; the icon goes out.
        let fx = app.button(70, true, 0.0);
        assert_eq!(fx.first(), Some(&crate::app::Effect::Mixer(None)));
        apply_effects(&mut p, fx);
        assert_eq!(p.page(), "Play");
        assert_eq!(
            p.ui.get::<Header>("header").unwrap().mixer_icon,
            Some(false)
        );
        // The icon in the bar asks the app.
        p.ui.paint(&mut SoftCanvas::new(SCREEN.0, SCREEN.1));
        let r = p.ui.rect_of("header").unwrap();
        let icon = p.ui.get::<Header>("header").unwrap().icon_rect(r).unwrap();
        assert_eq!(
            p.touch(4, Touch::Down, icon.x + 10, icon.y + 10),
            Some(FromUi::MixerIcon)
        );
        // SHIFT + TRACK MIX: Pad Mix; a tab closes the mixer.
        app.button(70, false, 0.0);
        app.button(mpfree_controller::mpc_one::button::SHIFT, true, 0.0);
        let fx = app.button(70, true, 0.0);
        assert_eq!(
            fx.first(),
            Some(&crate::app::Effect::Mixer(Some(MixView::Pads)))
        );
        app.button(mpfree_controller::mpc_one::button::SHIFT, false, 0.0);
        let fx = app.tab(2);
        assert!(fx.contains(&crate::app::Effect::Mixer(None)), "{fx:?}");
    }

    #[test]
    fn the_io_icon_opens_the_mixers_io_view_and_the_tempo_chip_opens_it_too() {
        use mpfree_mixer_ui::{MixState, MixView, MIXER_HINT_IO, MIXER_PAGE};
        let mut p = sample_player(Fonts::embedded());
        let mut app = sample_app();
        // The I/O icon sits left of the mixer icon and asks the app.
        p.ui.paint(&mut SoftCanvas::new(SCREEN.0, SCREEN.1));
        let r = p.ui.rect_of("header").unwrap();
        let h = p.ui.get::<Header>("header").unwrap();
        let (io, mixer) = (h.io_icon_rect(r).unwrap(), h.icon_rect(r).unwrap());
        assert!(io.right() < mixer.x);
        assert_eq!(
            p.touch(1, Touch::Down, io.x + 10, io.y + 10),
            Some(FromUi::IoIcon)
        );
        // From Play: the mixer opens on I/O.
        let fx = app.toggle_io(false);
        assert_eq!(
            fx.first(),
            Some(&crate::app::Effect::Mixer(Some(MixView::Io)))
        );
        apply_effects(&mut p, fx);
        assert_eq!(p.page(), MIXER_PAGE);
        let mut m = mpfree_mixer_ui::sample::song_mixer();
        let st = MixState::from_mixer(&mut m, MixView::Io, &mpfree_mixer_ui::sample::name);
        p.apply(ToUi::Mixer(Box::new(st)));
        // One icon lit, the I/O view's hint.
        let h = p.ui.get::<Header>("header").unwrap();
        assert_eq!((h.io_icon, h.mixer_icon), (Some(true), Some(false)));
        assert_eq!(p.ui.get::<Label>("hint").unwrap().text, MIXER_HINT_IO);
        assert!(p.io_showing());
        // The wheel is the I/O view's now, not Q-Link 1's: nothing is sent
        // to the mixer for a turn of the selection.
        let fx = app.wheel(1);
        assert_eq!(fx, [crate::app::Effect::MixerWheel(1)]);
        apply_effects(&mut p, fx);
        assert!(
            !p.take_actions()
                .iter()
                .any(|a| matches!(a, FromUi::Mixer(mpfree_mixer_ui::MixAction::Set { .. }))),
            "the wheel moved no fader"
        );
        // - and + on the mixer are the I/O view's too.
        let mut fx = Vec::new();
        app.map.learn(crate::app::mapper::Job::Plus, Some(55));
        fx.extend(app.button(55, true, 0.0));
        assert_eq!(fx, [crate::app::Effect::MixerStep(1)]);
        // The I/O icon again closes it, back to Play.
        let fx = app.toggle_io(p.io_showing());
        assert_eq!(fx.first(), Some(&crate::app::Effect::Mixer(None)));
        apply_effects(&mut p, fx);
        assert_eq!(p.page(), "Play");
        let h = p.ui.get::<Header>("header").unwrap();
        assert_eq!((h.io_icon, h.mixer_icon), (Some(false), Some(false)));
        // While an outside clock sets the tempo, the tempo chip shows; a
        // tap opens the I/O view (at Sync once that section is there).
        p.apply(ToUi::ClockFrom(Some("MIDI In".into())));
        let chips = &p.ui.get::<Header>("header").unwrap().chips;
        let at = chips
            .iter()
            .position(|c| c.text.ends_with("from MIDI In"))
            .expect("the tempo chip");
        p.ui.paint(&mut SoftCanvas::new(SCREEN.0, SCREEN.1));
        let cr = p.ui.get::<Header>("header").unwrap().chip_rect(at).unwrap();
        assert_eq!(p.touch(2, Touch::Down, cr.x + 5, cr.y + 5), None);
        assert_eq!(p.take_actions(), [FromUi::OpenIo]);
        assert!(!p.open_io_section("Sync"), "no Sync section yet");
        // The model arrives (once, at start): MIDI, Sync, CV and Monitor
        // join the list after Audio, and the chip's tap opens Sync. A
        // second model adds nothing.
        let model = std::sync::Arc::new(std::sync::Mutex::new(mpfree_midi_ui::sample::rig()));
        p.apply(ToUi::IoModel(model.clone()));
        p.apply(ToUi::IoModel(model));
        assert_eq!(
            p.io_sections,
            [
                mpfree_midi_ui::MIDI,
                mpfree_midi_ui::SYNC,
                mpfree_midi_ui::CV,
                mpfree_midi_ui::MONITOR
            ]
        );
        assert!(p.open_io_section("Sync"));
        let v =
            p.ui.get::<mpfree_mixer_ui::MixerView>(mpfree_mixer_ui::MIXER);
        assert_eq!(v.map(|v| v.io_section()), Some(2), "Audio, MIDI, Sync");
        let fx = app.open_io();
        assert_eq!(
            fx.first(),
            Some(&crate::app::Effect::Mixer(Some(MixView::Io)))
        );
        p.apply(ToUi::ClockFrom(None));
        assert!(!p
            .ui
            .get::<Header>("header")
            .unwrap()
            .chips
            .iter()
            .any(|c| c.text.contains("from MIDI In")));
    }

    /// A strip's insert row: the chain editor takes the mixer's place with
    /// the same Q-Link column, on the selected effect's first four knobs;
    /// Q-LINK steps to Macros 1-4; a long press on a macro card opens the
    /// macros panel; Back goes back to the mixer on the view it showed.
    #[test]
    fn the_chain_editor_takes_the_mixers_place_and_the_q_links_turn_its_knobs() {
        use crate::app::Effect;
        use mpfree_mixer_ui::{MixView, MIXER_PAGE};
        let mut p = sample_player(Fonts::embedded());
        let mut app = sample_app();
        app.map.learn(crate::app::mapper::Job::TrackMix, Some(70));
        app.map.learn(crate::app::mapper::Job::QLink, Some(71));
        app.button(mpfree_controller::mpc_one::button::SHIFT, true, 0.0);
        apply_effects(&mut p, app.button(70, true, 0.0));
        app.button(70, false, 0.0);
        app.button(mpfree_controller::mpc_one::button::SHIFT, false, 0.0);
        assert_eq!(p.page(), MIXER_PAGE);
        // The audio thread opens the editor (`Panels::open_insert`).
        let fx = app.open_chain();
        assert!(fx.contains(&Effect::Mixer(None)), "{fx:?}");
        apply_effects(&mut p, fx);
        sample_panels(&mut p, View::Chain);
        assert_eq!(p.page(), mpfree_chain_ui::CHAIN_PAGE);
        assert_eq!(p.ui.get::<Header>("header").unwrap().active, usize::MAX);
        let q = p.ui.get::<QLinkColumn>(play_page::QLINKS).unwrap();
        assert!(q.title.starts_with("Q-Links · "), "{}", q.title);
        assert!(!q.cards[1].is_free(), "{:?}", q.cards);
        // The wheel is Q-Link 1: the effect's first knob, for the chain.
        apply_effects(&mut p, app.wheel(1));
        let sent = p.take_actions();
        assert!(
            matches!(sent.as_slice(), [FromUi::Chain(ChainAction::Set { .. })]),
            "{sent:?}"
        );
        // Q-LINK: Macros 1-4; a turn moves macro 2.
        let fx = app.button(71, true, 0.0);
        app.button(71, false, 0.0);
        assert_eq!(fx, vec![Effect::QLinkBank]);
        apply_effects(&mut p, fx);
        let q = p.ui.get::<QLinkColumn>(play_page::QLINKS).unwrap();
        assert_eq!(q.title, "Q-Links · Macros 1-4");
        assert_eq!(q.cards[1].scope, "Macro 2");
        p.apply(ToUi::ChainQLink(1, 3));
        let sent = p.take_actions();
        assert!(
            matches!(
                sent.as_slice(),
                [FromUi::Macros(MacroAction::Macro { m: 1, .. })]
            ),
            "{sent:?}"
        );
        // A finger held still on macro 3's card opens the macros panel on
        // Macros 1-4; a drag on it turns it instead.
        p.ui.paint(&mut SoftCanvas::new(SCREEN.0, SCREEN.1));
        let r = p.ui.rect_of(play_page::QLINKS).unwrap();
        let c = QLinkColumn::card_rects(r)[2];
        let (x, y) = (c.x + c.w / 2, c.y + c.h / 2);
        p.touch(5, Touch::Down, x, y);
        p.tick(0.3);
        assert!(p.take_actions().is_empty());
        p.tick(0.3);
        assert_eq!(p.take_actions(), vec![FromUi::OpenMacros(0)]);
        p.touch(5, Touch::Up, x, y);
        p.touch(6, Touch::Down, x, y);
        p.touch(6, Touch::Move, x, y - 3 * mpfree_ui::qlink::DRAG_DETENT);
        p.tick(1.0);
        let sent = p.take_actions();
        assert!(
            matches!(
                sent.as_slice(),
                [FromUi::Macros(MacroAction::Macro { m: 2, .. })]
            ),
            "{sent:?}"
        );
        p.touch(6, Touch::Up, x, y);
        // The panel over the editor; its Back comes back to the editor...
        apply_effects(&mut p, app.open_macros());
        assert_eq!(p.page(), mpfree_chain_ui::MACROS_PAGE);
        assert_eq!(
            p.ui.get::<QLinkColumn>(play_page::QLINKS).unwrap().title,
            "Q-Links · Macros 1-4"
        );
        apply_effects(&mut p, app.close_macros(MixView::Pads));
        assert_eq!(p.page(), mpfree_chain_ui::CHAIN_PAGE);
        // ...and the editor's goes back to the mixer, on Pad Mix again.
        let fx = app.close_chain(MixView::Pads);
        assert_eq!(fx.first(), Some(&Effect::Mixer(Some(MixView::Pads))));
        apply_effects(&mut p, fx);
        assert_eq!(p.page(), MIXER_PAGE);
    }

    /// Live test 3: the kit chip names the kit (not "Kit 1 of 10"), and a
    /// RAM chip sits beside the CPU's.
    #[test]
    fn the_header_names_the_kit_and_shows_ram() {
        let mut p = sample_player(Fonts::embedded());
        p.set_ram(17.4);
        let chips: Vec<String> =
            p.ui.get::<Header>("header")
                .unwrap()
                .chips
                .iter()
                .map(|c| c.text.clone())
                .collect();
        assert!(chips.iter().any(|c| c == "RAM 17 %"), "{chips:?}");
        assert!(!chips.iter().any(|c| c.contains(" of ")), "{chips:?}");
        assert_eq!(
            chip_kit_name("ClassicHouse-Kit-Classy", 0),
            "ClassicHouse Kit Classy"
        );
        assert_eq!(chip_kit_name("", 2), "Kit 3");
        assert_eq!(
            chip_kit_name("A-very-long-kit-name-that-goes-on-and-on", 0),
            "A very long kit name that g…"
        );
        assert_eq!(
            crate::perf::ram_used_pct("MemTotal: 2000 kB\nMemAvailable: 1500 kB\n"),
            Some(25.0)
        );
    }

    /// Jay after live test 3: the Warp tempo can be typed. A tap on its
    /// value opens the number pad; Enter sets it (one undo step), a number
    /// out of range keeps the pad open and says why, Cancel changes
    /// nothing, and the wheel steps the number with its push as Enter.
    #[test]
    fn the_warp_tempo_can_be_typed_on_the_number_pad() {
        use crate::app::RowKey;
        use mpfree_sampler_ui::wave_tab::{Row, REGION};
        use mpfree_sampler_ui::widgets::SettingList;
        use mpfree_ui::numpad::{NumPad, KEY_CANCEL, KEY_ENTER, KEY_POINT};
        let mut p = sample_sampler(Fonts::embedded());
        let row = |r: Row| Row::ALL.iter().position(|k| *k == r).unwrap();
        p.on_event(UiEvent {
            widget: REGION.into(),
            kind: EventKind::Step {
                index: row(Row::Warp),
                delta: 1,
            },
        });
        p.apply(ToUi::SampleLift);
        let bpm = |p: &Player| p.sample.state().unwrap().region.warp.as_ref().unwrap().bpm;
        let before = bpm(&p);
        let tap = |p: &mut Player, (x, y): (i32, i32)| {
            let a = p.touch(4, Touch::Down, x, y);
            p.touch(4, Touch::Up, x, y);
            a
        };
        let open = |p: &mut Player| {
            p.ui.paint(&mut SoftCanvas::new(SCREEN.0, SCREEN.1));
            let r = p.ui.rect_of(REGION).unwrap();
            let list = p.ui.get::<SettingList>(REGION).unwrap();
            assert_eq!(list.typed, [row(Row::WarpTempo)], "warped: it can be typed");
            let (_, value) = list.row_rects(r, row(Row::WarpTempo)).expect("in view");
            tap(p, (value.x + value.w / 2, value.y + value.h / 2));
            assert_eq!(p.page(), NUMPAD_PAGE);
        };
        let key = |p: &mut Player, k: usize| {
            let r = p.ui.rect_of(NUMPAD).unwrap();
            tap(p, NumPad::key_at(r, k))
        };
        open(&mut p);
        assert!(p
            .ui
            .get::<Label>("hint")
            .unwrap()
            .text
            .starts_with("Tap the number"));
        for k in [1, 2, 8, KEY_POINT, 5] {
            key(&mut p, k);
        }
        assert_eq!(key(&mut p, KEY_ENTER), None);
        assert_eq!(p.page(), sample_page::PAGE);
        assert_eq!(bpm(&p), 128.5);
        // One undo step takes it back.
        p.apply(ToUi::SampleUndo(false));
        assert_eq!(bpm(&p), before);
        // Out of range: the pad stays and says the range.
        open(&mut p);
        for k in [1, 5, 0, 0] {
            key(&mut p, k);
        }
        key(&mut p, KEY_ENTER);
        assert_eq!(p.page(), NUMPAD_PAGE);
        assert_eq!(
            p.ui.get::<NumPad>(NUMPAD).unwrap().warning,
            "Warp tempo: type 20 to 999 BPM"
        );
        key(&mut p, KEY_CANCEL);
        assert_eq!((p.page(), bpm(&p)), (sample_page::PAGE, before));
        // The wheel steps from the tempo now (a tenth a detent here) and
        // its push is Enter.
        open(&mut p);
        p.apply(ToUi::SampleWheel(3));
        p.apply(ToUi::Rows(RowKey::Editing(true)));
        assert_eq!(p.page(), sample_page::PAGE);
        assert!((bpm(&p) - (before + 0.3)).abs() < 0.001, "{}", bpm(&p));
    }

    /// Jay after live test 3: a finger held on Play's BPM types the tempo.
    #[test]
    fn a_long_press_on_plays_tempo_types_it() {
        use mpfree_ui::numpad::{NumPad, KEY_ENTER};
        let mut p = sample_player(Fonts::embedded());
        p.apply(ToUi::Show(View::Play));
        p.ui.paint(&mut SoftCanvas::new(SCREEN.0, SCREEN.1));
        let b =
            p.ui.get::<PlayTransport>(play_page::TRANSPORT)
                .unwrap()
                .bpm_area()
                .unwrap();
        let (x, y) = (b.x + b.w / 2, b.y + b.h / 2);
        // A quick tap only taps.
        p.touch(1, Touch::Down, x, y);
        p.tick(0.2);
        p.touch(1, Touch::Up, x, y);
        p.tick(1.0);
        assert_ne!(p.page(), NUMPAD_PAGE);
        // Held: the number pad, for the tempo.
        assert_eq!(p.touch(2, Touch::Down, x, y), Some(FromUi::TapTempo));
        p.tick(0.4);
        p.tick(0.3);
        assert_eq!(p.page(), NUMPAD_PAGE);
        // The app hands it the wheel and - / + (on Play they pick kits).
        assert!(p.take_actions().contains(&FromUi::Overlay(true)));
        p.touch(2, Touch::Up, x, y);
        let r = p.ui.rect_of(NUMPAD).unwrap();
        let tap = |p: &mut Player, k: usize| {
            let (x, y) = NumPad::key_at(r, k);
            let a = p.touch(3, Touch::Down, x, y);
            p.touch(3, Touch::Up, x, y);
            a
        };
        for k in [9, 5] {
            tap(&mut p, k);
        }
        assert_eq!(
            tap(&mut p, KEY_ENTER),
            Some(FromUi::Time(TimeAction::Transport(
                mpfree_transport::Command::SetTempo(95.0)
            )))
        );
        assert_eq!(p.page(), "Play");
    }

    /// Live test 3: a tap on Play's tempo is a tap of the tempo.
    #[test]
    fn a_tap_on_plays_tempo_taps_it() {
        let mut p = sample_player(Fonts::embedded());
        p.apply(ToUi::Show(View::Play));
        p.ui.paint(&mut SoftCanvas::new(SCREEN.0, SCREEN.1));
        let b =
            p.ui.get::<PlayTransport>(play_page::TRANSPORT)
                .unwrap()
                .bpm_area()
                .expect("the transport was drawn");
        assert!(b.w >= 64);
        let got = p.touch(1, Touch::Down, b.x + b.w / 2, b.y + b.h / 2);
        p.touch(1, Touch::Up, b.x + b.w / 2, b.y + b.h / 2);
        assert_eq!(got, Some(FromUi::TapTempo));
    }

    /// Live test 3: a refusal used to go only to the Dev tab's log. Now
    /// it shows in place of the hint, long enough to read, then the hint
    /// comes back.
    #[test]
    fn a_refusal_is_said_in_place_then_the_hint_comes_back() {
        let mut p = sample_player(Fonts::embedded());
        let hint = p.ui.get::<Label>("hint").unwrap().text.clone();
        p.apply(ToUi::Tell(
            "sample: edit of pad A7 not made (BadRegion(\"loop mode needs loop points\"))".into(),
        ));
        let l = p.ui.get::<Label>("hint").unwrap();
        assert_eq!(
            l.text,
            "Edit of pad A7 not made: loop mode needs loop points"
        );
        assert_eq!(l.tone, Tone::Warn);
        p.tick(TELL_SECONDS - 1.0);
        assert!(p
            .ui
            .get::<Label>("hint")
            .unwrap()
            .text
            .starts_with("Edit of pad A7"));
        p.tick(1.1);
        let l = p.ui.get::<Label>("hint").unwrap();
        assert_eq!(l.text, hint);
        assert_eq!(l.tone, Tone::Dim);
        assert_eq!(tell_text("pads: In key, C Major"), "In key, C Major");
        assert_eq!(tell_text("The chain is full"), "The chain is full");
    }

    /// Live test 3: every tap in the chain editor and the macros panel
    /// reaches the app (they used to stop in the widget). A tap on a
    /// row, + Add effect, a pick, Cancel and Back, each through
    /// `Player::touch` as the screen sends them.
    #[test]
    fn the_chain_editors_taps_reach_the_app() {
        use mpfree_chain_ui::ChainView;
        let mut p = sample_player(Fonts::embedded());
        sample_panels(&mut p, View::Chain);
        assert_eq!(p.page(), mpfree_chain_ui::CHAIN_PAGE);
        p.ui.paint(&mut SoftCanvas::new(SCREEN.0, SCREEN.1));
        let r = p.ui.rect_of(mpfree_chain_ui::CHAIN).unwrap();
        let tap = |p: &mut Player, x: i32, y: i32| {
            let mut got: Vec<FromUi> = Vec::new();
            got.extend(p.touch(1, Touch::Down, x, y));
            got.extend(p.touch(1, Touch::Up, x, y));
            got.extend(p.take_actions());
            got
        };
        let l =
            p.ui.get::<ChainView>(mpfree_chain_ui::CHAIN)
                .unwrap()
                .layout(r);
        let row = l.rows[0];
        let got = tap(&mut p, row.x + 20, row.y + row.h / 2);
        assert_eq!(got, vec![FromUi::Chain(ChainAction::Select(0))]);
        let add = l.add.expect("the chain has room for + Add effect");
        let got = tap(&mut p, add.x + 10, add.y + 10);
        assert_eq!(got, vec![FromUi::Chain(ChainAction::Picker(true))]);
        let l =
            p.ui.get::<ChainView>(mpfree_chain_ui::CHAIN)
                .unwrap()
                .layout(r);
        let c = l.cancel.expect("the picker has a Cancel");
        let got = tap(&mut p, c.x + 5, c.y + 5);
        assert_eq!(got, vec![FromUi::Chain(ChainAction::Picker(false))]);
        let got = tap(&mut p, add.x + 10, add.y + 10);
        assert_eq!(got, vec![FromUi::Chain(ChainAction::Picker(true))]);
        let l =
            p.ui.get::<ChainView>(mpfree_chain_ui::CHAIN)
                .unwrap()
                .layout(r);
        let pick = l.picks[0];
        let got = tap(&mut p, pick.x + 10, pick.y + 10);
        assert!(
            matches!(got.as_slice(), [FromUi::Chain(ChainAction::Add(_))]),
            "{got:?}"
        );
        let back = l.back.expect("opened from the mixer: ‹ Mixer");
        let got = tap(&mut p, back.x + 5, back.y + 5);
        assert_eq!(got, vec![FromUi::Chain(ChainAction::Back)]);
        // The macros panel's Back too.
        p.apply(ToUi::Show(View::Macros));
        p.ui.paint(&mut SoftCanvas::new(SCREEN.0, SCREEN.1));
        let r = p.ui.rect_of(mpfree_chain_ui::MACROS_W).unwrap();
        let b = mpfree_chain_ui::macros::layout(r).back;
        let got = tap(&mut p, b.x + 5, b.y + 5);
        assert_eq!(got, vec![FromUi::Macros(MacroAction::Back)]);
    }

    /// The wheel on the chain editor: Q-Link 1 while the picker is shut;
    /// while it is open, the picker's tiles, and its push opens the
    /// presets and then adds.
    #[test]
    fn the_wheel_works_the_effect_picker_while_it_is_open() {
        use mpfree_chain_ui::ChainView;
        let mut p = sample_player(Fonts::embedded());
        sample_panels(&mut p, View::Chain);
        p.ui.paint(&mut SoftCanvas::new(SCREEN.0, SCREEN.1));
        p.apply(ToUi::ChainWheel(1));
        let got = p.take_actions();
        assert!(
            matches!(got.as_slice(), [FromUi::Chain(ChainAction::Set { .. })]),
            "Q-Link 1 while the picker is shut: {got:?}"
        );
        let r = p.ui.rect_of(mpfree_chain_ui::CHAIN).unwrap();
        let add =
            p.ui.get::<ChainView>(mpfree_chain_ui::CHAIN)
                .unwrap()
                .layout(r)
                .add
                .unwrap();
        p.touch(1, Touch::Down, add.x + 10, add.y + 10);
        p.touch(1, Touch::Up, add.x + 10, add.y + 10);
        p.take_actions();
        p.apply(ToUi::ChainWheel(1));
        assert!(p.take_actions().is_empty(), "the picker took the wheel");
        let v = p.ui.get::<ChainView>(mpfree_chain_ui::CHAIN).unwrap();
        assert_eq!(v.picker().highlight, Some(0));
        let first = v.picker().picks(v.state())[0].id;
        p.apply(ToUi::ChainPush);
        let v = p.ui.get::<ChainView>(mpfree_chain_ui::CHAIN).unwrap();
        assert_eq!(v.picker().presets, Some(first), "its presets open");
        p.apply(ToUi::ChainPush);
        assert_eq!(
            p.take_actions(),
            [FromUi::Chain(ChainAction::Add(first))],
            "Defaults adds it as it starts"
        );
    }

    /// Q-LINK on Play: Macros 1-4, Macros 5-8, then Play's own knobs; the
    /// knobs turn the macros meanwhile. Other pages keep their Q-Links.
    #[test]
    fn q_link_steps_plays_column_through_the_macro_banks() {
        use crate::app::Effect;
        let mut p = sample_player(Fonts::embedded());
        let mut app = sample_app();
        app.map.learn(crate::app::mapper::Job::QLink, Some(71));
        p.apply(ToUi::Macros(Box::new(
            mpfree_chain_ui::sample::Song::new().macros_state(0),
        )));
        let title = |p: &Player| {
            p.ui.get::<QLinkColumn>(play_page::QLINKS)
                .unwrap()
                .title
                .clone()
        };
        assert_eq!(title(&p), "Q-Links · Play");
        let mut q_link = |p: &mut Player| {
            let fx = app.button(71, true, 0.0);
            app.button(71, false, 0.0);
            apply_effects(p, fx);
        };
        q_link(&mut p);
        assert_eq!(title(&p), "Q-Links · Macros 1-4");
        p.apply(ToUi::PlayQLink(3, 1));
        let sent = p.take_actions();
        assert!(
            matches!(
                sent.as_slice(),
                [FromUi::Macros(MacroAction::Macro { m: 3, .. })]
            ),
            "{sent:?}"
        );
        q_link(&mut p);
        assert_eq!(title(&p), "Q-Links · Macros 5-8");
        q_link(&mut p);
        assert_eq!(title(&p), "Q-Links · Play");
        // On the Synth tab Q-LINK does nothing yet.
        apply_effects(&mut p, app.tab(3));
        let fx = app.button(71, true, 0.0);
        assert!(!fx.contains(&Effect::QLinkBank), "{fx:?}");
    }

    /// Arrange has Play's column (layout spec 4c): the same knobs, turned
    /// by the Q-Links and by a drag on a card, and Q-LINK steps the same
    /// macro banks, which Play then still shows.
    #[test]
    /// Layout spec 4c: with an audio region picked on the Song view, the
    /// Q-Links are the track's level and pan and the region's gain and
    /// fades, and a turn goes to the audio tracks. Letting the region go
    /// gives Play's knobs back.
    fn an_audio_region_picked_on_arrange_takes_the_q_links() {
        use mpfree_time_ui::song::{AudioBlock, AudioLane};
        let mut p = sample_player(Fonts::embedded());
        let mut app = sample_app();
        apply_effects(&mut p, app.tab(2));
        assert_eq!(p.view, View::Arrange);
        let title = |p: &Player| {
            p.ui.get::<QLinkColumn>(play_page::QLINKS)
                .unwrap()
                .title
                .clone()
        };
        let mut st = p.time.clone();
        st.song_view = true;
        st.audio = vec![AudioLane {
            track: 1,
            name: "Audio 2".into(),
            color: time_view::audio_color(1),
            armed: false,
            input: mpfree_audiotracks::Input::Left,
            level: 0.0,
            volume: 0.5,
            pan: -0.2,
            monitor: mpfree_audiotracks::Monitor::Auto,
            mute: false,
            solo: false,
            blocks: vec![AudioBlock {
                start: 0,
                end: 3840,
                take: 0,
                tag: String::new(),
                src: 0,
                frames: 88_200.0,
                gain: 2.0,
                fade_in: 441,
                fade_out: 0,
            }],
            takes: Vec::new(),
            recording: None,
        }];
        p.apply(ToUi::Time(Box::new(st.clone())));
        assert_eq!(
            title(&p),
            "Q-Links · Arrange",
            "nothing picked: Play's knobs"
        );
        st.region = Some(time_view::RegionId { track: 1, index: 0 });
        p.apply(ToUi::Time(Box::new(st.clone())));
        assert_eq!(title(&p), "Q-Links · Audio region");
        let cards =
            p.ui.get::<QLinkColumn>(play_page::QLINKS)
                .unwrap()
                .cards
                .clone();
        let shown: Vec<_> = cards
            .iter()
            .map(|c| (c.name.as_str(), c.scope.as_str(), c.value.as_str()))
            .collect();
        assert_eq!(
            shown,
            [
                ("Level", "AUDIO 2", "-6.0 dB"),
                ("Pan", "AUDIO 2", "L20"),
                ("Gain", "REGION", "6.0 dB"),
                ("Fades", "REGION", "10 ms"),
            ]
        );
        assert!(cards[1].bipolar);
        p.take_actions();
        p.apply(ToUi::PlayQLink(2, 3));
        assert_eq!(
            p.take_actions(),
            vec![FromUi::Time(TimeAction::Audio(AudioAction::Turn {
                track: 1,
                region: Some(0),
                knob: 2,
                ticks: 3,
                fine: false
            }))]
        );
        // A drag on a card is a turn too.
        let _ = p.on_event(mpfree_ui::UiEvent {
            widget: play_page::QLINKS.to_string(),
            kind: mpfree_ui::EventKind::Step {
                index: 0,
                delta: -1,
            },
        });
        assert_eq!(
            p.take_actions(),
            vec![FromUi::Time(TimeAction::Audio(AudioAction::Turn {
                track: 1,
                region: Some(0),
                knob: 0,
                ticks: -1,
                fine: false
            }))]
        );
        st.region = None;
        p.apply(ToUi::Time(Box::new(st.clone())));
        assert_eq!(title(&p), "Q-Links · Arrange");
        // The track panel open: the track's level and pan, and Q3 and Q4
        // ask for a region.
        st.audio_panel = Some(1);
        p.apply(ToUi::Time(Box::new(st)));
        assert_eq!(title(&p), "Q-Links · Audio 2");
        p.take_actions();
        p.apply(ToUi::PlayQLink(1, 2));
        assert_eq!(
            p.take_actions(),
            vec![FromUi::Time(TimeAction::Audio(AudioAction::Turn {
                track: 1,
                region: None,
                knob: 1,
                ticks: 2,
                fine: false
            }))]
        );
    }

    /// Layout spec 4c: a swipe across a take lane snaps to the grid, and
    /// with SHIFT held it is free.
    #[test]
    fn the_audio_level_bars_come_on_their_own() {
        let mut p = sample_player(Fonts::embedded());
        let st = mpfree_time_ui::sample::song_audio_state().unwrap();
        p.apply(ToUi::Time(Box::new(st)));
        p.apply(ToUi::Show(View::Arrange));
        assert!(!p.time.audio.is_empty());
        let mut levels = [0.0; mpfree_audiotracks::MAX_TRACKS];
        for (t, l) in levels.iter_mut().enumerate() {
            *l = t as f32 / 10.0;
        }
        p.apply(ToUi::AudioLevels(levels));
        for lane in &p.time.audio {
            assert_eq!(lane.level, levels[lane.track]);
        }
    }

    #[test]
    fn a_take_lane_swipe_with_shift_is_free() {
        let mut p = sample_player(Fonts::embedded());
        let swipe = |p: &mut Player| {
            p.on_event(mpfree_ui::UiEvent {
                widget: time_view::SONG.to_string(),
                kind: mpfree_ui::EventKind::Span {
                    index: 2,
                    from: 100,
                    to: 900,
                },
            })
        };
        let comp = |fine| {
            Some(FromUi::Time(TimeAction::Audio(AudioAction::Comp {
                take: 2,
                from: 100,
                to: 900,
                fine,
            })))
        };
        assert_eq!(swipe(&mut p), comp(false));
        p.status.shift = true;
        assert_eq!(swipe(&mut p), comp(true));
        // A region's drag too.
        let moved = p.on_event(mpfree_ui::UiEvent {
            widget: time_view::SONG.to_string(),
            kind: mpfree_ui::EventKind::Marker {
                index: mpfree_time_ui::song::REGION_SLOTS + 2,
                frame: 3850,
            },
        });
        assert_eq!(
            moved,
            Some(FromUi::Time(TimeAction::Audio(AudioAction::Move {
                region: time_view::RegionId { track: 1, index: 2 },
                to: 3850,
                fine: true,
            })))
        );
        // SHIFT + Undo on the audio row redoes (layout spec 4c).
        p.time.region = Some(time_view::RegionId { track: 1, index: 0 });
        let undo = |p: &mut Player| {
            p.on_event(mpfree_ui::UiEvent {
                widget: time_view::SONG.to_string(),
                kind: mpfree_ui::EventKind::Tab(5),
            })
        };
        let audio = |a| Some(FromUi::Time(TimeAction::Audio(a)));
        assert_eq!(undo(&mut p), audio(AudioAction::Redo));
        p.status.shift = false;
        assert_eq!(undo(&mut p), audio(AudioAction::Undo));
    }

    #[test]
    fn arrange_has_plays_q_links_and_their_banks() {
        let mut p = sample_player(Fonts::embedded());
        let mut app = sample_app();
        app.map.learn(crate::app::mapper::Job::QLink, Some(71));
        p.apply(ToUi::Macros(Box::new(
            mpfree_chain_ui::sample::Song::new().macros_state(0),
        )));
        let title = |p: &Player| {
            p.ui.get::<QLinkColumn>(play_page::QLINKS)
                .unwrap()
                .title
                .clone()
        };
        let cards = |p: &Player| {
            p.ui.get::<QLinkColumn>(play_page::QLINKS)
                .unwrap()
                .cards
                .clone()
        };
        let play_cards = cards(&p);
        apply_effects(&mut p, app.tab(2));
        assert_eq!(p.view, View::Arrange);
        assert_eq!(title(&p), "Q-Links · Arrange");
        assert_eq!(cards(&p), play_cards, "Play's knobs");
        let r =
            p.ui.rect_of(play_page::QLINKS)
                .expect("the column is on Arrange");
        assert_eq!((r.x, r.w), (1100, 168), "at the right edge, 168 px");
        // Q-Link 1 is Play's first knob, and a turn of it does what it does on Play.
        p.apply(ToUi::PlayQLink(0, 6));
        assert!(
            !p.take_actions().is_empty(),
            "a turn on Arrange does something"
        );
        // A drag on a card is a turn too.
        let ev = mpfree_ui::UiEvent {
            widget: play_page::QLINKS.to_string(),
            kind: mpfree_ui::EventKind::Step { index: 0, delta: 2 },
        };
        let _ = p.on_event(ev);
        assert!(
            !p.take_actions().is_empty(),
            "a card drag on Arrange does something"
        );
        // Q-LINK steps the macro banks on Arrange.
        let fx = app.button(71, true, 0.0);
        app.button(71, false, 0.0);
        apply_effects(&mut p, fx);
        assert_eq!(title(&p), "Q-Links · Macros 1-4");
        p.apply(ToUi::PlayQLink(3, 1));
        let sent = p.take_actions();
        assert!(
            matches!(
                sent.as_slice(),
                [FromUi::Macros(MacroAction::Macro { m: 3, .. })]
            ),
            "{sent:?}"
        );
        // Play shares the bank.
        apply_effects(&mut p, app.tab(0));
        assert_eq!(title(&p), "Q-Links · Macros 1-4");
    }

    #[test]
    fn the_pads_heading_switches_the_synth_to_in_key() {
        let mut p = sample_player(Fonts::embedded());
        let synth_kit = sample_kits().len();
        // The synth with a scale: the switch, on the Synth tab's heading.
        let mut s = crate::synth::Synth::new(44_100);
        let mut set = s.settings();
        set.scale = mpfree_arp::scale::by_name("Minor")
            .map(|d| mpfree_arp::scale::Scale::with_down(9, d.mask, d.down));
        s.set_settings(set);
        p.apply(ToUi::KitChanged {
            kit: synth_kit,
            view: s.kit_view(),
        });
        p.apply(ToUi::Kit(synth_kit));
        p.apply(ToUi::Show(View::Synth));
        let head = p.ui.rect_of(play_page::SYNTH_PADS_HEAD).unwrap();
        assert_eq!(
            p.ui.get::<PadsBar>(play_page::SYNTH_PADS_HEAD)
                .unwrap()
                .title,
            "Pads · Bank A"
        );
        let [chromatic, in_key] = PadsBar::halves(head);
        assert_eq!(
            p.touch(1, Touch::Down, in_key.x + 10, in_key.y + 10),
            Some(FromUi::PadsLayout { in_key: true })
        );
        p.touch(1, Touch::Up, in_key.x + 10, in_key.y + 10);
        assert_eq!(
            p.touch(2, Touch::Down, chromatic.x + 10, chromatic.y + 10),
            Some(FromUi::PadsLayout { in_key: false })
        );
    }

    #[test]
    fn layouts_fit_the_screen() {
        let mut p = Player::new(Fonts::embedded(), SCREEN, false, None);
        assert_eq!(
            p.ui.rect_of("loading"),
            Some(mpfree_ui::Rect::new(0, 0, 1280, 800))
        );
        p.apply(ToUi::Show(View::Play));
        // The Play page (mockup F, pick A): no pads at all; the transport
        // card on top, the pattern across the room, the master meter and
        // the Q-Link cards at the right edge, beside the knobs.
        assert!(p.ui.rect_of(play_page::PADS).is_none());
        let steps = p.ui.rect_of(play_page::STEPS).unwrap();
        assert!(steps.w == 998 && steps.h > 250, "{steps:?}");
        assert_eq!(
            p.ui.rect_of(play_page::TRANSPORT).map(|r| (r.y, r.h)),
            Some((84, 84))
        );
        assert_eq!(
            p.ui.rect_of(play_page::QLINKS)
                .map(|r| (r.right(), r.y, r.bottom())),
            Some((1268, 84, 750))
        );
        assert!(p
            .ui
            .rect_of(crate::looper_page::LOOP_STRIP)
            .is_some_and(|r| r.w > 900));
        p.apply(ToUi::Show(View::Arrange));
        // The grid over the scene morph bar.
        let clips = p.ui.rect_of(time_view::CLIPS).unwrap();
        assert!(clips.w > 780 && clips.h > 500, "{clips:?}");
        p.apply(ToUi::Show(View::Prefs));
        assert!(p.ui.rect_of("options").unwrap().h > 600);
        let c = Console::new(Fonts::embedded(), "touch", None);
        assert!(c.ui.rect_of("log").unwrap().h > 600);
    }

    #[test]
    fn the_touch_lab_opens_from_prefs_and_exit_or_a_page_change_closes_it() {
        let mut p = sample_player(Fonts::embedded());
        let mut app = crate::app::App::new(44_100, false);
        app.map.menu = Some(1);
        app.button(mpfree_controller::mpc_one::button::SHIFT, true, 0.0);
        let mut fx = app.button(1, true, 0.0);
        app.button(1, false, 0.0);
        app.button(mpfree_controller::mpc_one::button::SHIFT, false, 0.0);
        // Prefs' last row opens the touch screen section; its last row, + on it, opens the lab.
        fx.extend(app.step_item(usize::MAX, 1));
        fx.extend(app.step_item(usize::MAX, 1));
        assert!(fx.contains(&crate::app::Effect::TouchLab), "{fx:?}");
        for e in fx {
            match e {
                crate::app::Effect::Menu(m) => p.apply(ToUi::Menu(m)),
                crate::app::Effect::Show(v) => p.apply(ToUi::Show(v)),
                crate::app::Effect::TouchLab => p.apply(ToUi::TouchLab),
                _ => {}
            }
        }
        assert_eq!(p.page(), "Touch lab");
        assert_eq!(p.touch_page(), Some("touchlab"));
        // Touches go to the lab, not the app. Exit (top right) goes back to Prefs.
        assert_eq!(p.touch(1, Touch::Down, 1280 - 80, 38), None);
        assert_eq!(p.touch(1, Touch::Up, 1280 - 80, 38), None);
        assert_eq!(p.page(), "Prefs");
        assert_eq!(p.touch_page(), None);
        // Open again; a page change from the app (a hardware button) closes it.
        p.apply(ToUi::TouchLab);
        assert_eq!(p.page(), "Touch lab");
        p.apply(ToUi::Show(View::Play));
        assert_eq!(p.page(), "Play");
        assert_eq!(p.touch_page(), None);
        // Kernel times reach the lab without upsetting ordinary touches.
        p.set_touch_time(123);
        let header = p.ui.rect_of("header").unwrap();
        let tab1 = p.ui.get::<Header>("header").unwrap().tab_rect(header, 1);
        assert_eq!(
            p.touch(2, Touch::Down, tab1.x + 20, tab1.y + 20),
            Some(FromUi::Tab(1))
        );
    }

    #[test]
    fn calibrate_touch_opens_from_prefs_and_takes_every_finger() {
        let mut p = sample_player(Fonts::embedded());
        let mut app = crate::app::App::new(44_100, false);
        app.map.menu = Some(1);
        app.button(mpfree_controller::mpc_one::button::SHIFT, true, 0.0);
        let mut fx = app.button(1, true, 0.0);
        app.button(1, false, 0.0);
        app.button(mpfree_controller::mpc_one::button::SHIFT, false, 0.0);
        // In the touch screen section, Calibrate touch is the row before the Touch lab.
        fx.extend(app.step_item(usize::MAX, 1));
        fx.extend(app.step_item(app_rows(&app) - 2, 1));
        assert_eq!(
            fx.last(),
            Some(&crate::app::Effect::CalibrateTouch),
            "{fx:?}"
        );
        for e in fx {
            match e {
                crate::app::Effect::Menu(m) => p.apply(ToUi::Menu(m)),
                crate::app::Effect::Show(v) => p.apply(ToUi::Show(v)),
                crate::app::Effect::CalibrateTouch => p.apply(ToUi::CalibrateTouch),
                _ => {}
            }
        }
        assert_eq!(
            (p.page(), p.touch_page()),
            ("Calibrate touch", Some("calibrate"))
        );
        // A corner tap goes to the page, not the app; the lab can't be open at the same time.
        assert_eq!(p.touch(1, Touch::Down, 70, 70), None);
        assert_eq!(p.touch(1, Touch::Up, 70, 70), None);
        p.apply(ToUi::TouchLab);
        assert_eq!(p.touch_page(), Some("touchlab"));
        p.apply(ToUi::CalibrateTouch);
        assert_eq!(p.touch_page(), Some("calibrate"));
        // Exit sits right of the middle.
        assert_eq!(p.touch(2, Touch::Down, 640 + 60, 400 + 70), None);
        assert_eq!(p.touch(2, Touch::Up, 640 + 60, 400 + 70), None);
        assert_eq!((p.page(), p.touch_page()), ("Prefs", None));
    }

    #[test]
    fn the_touch_page_tunes_every_touch_and_saves_on_request() {
        let mut p = sample_player(Fonts::embedded());
        p.apply(ToUi::Settings(Settings {
            debug_mode: true,
            ..Settings::default()
        }));
        p.apply(ToUi::Show(View::Touch));
        assert_eq!(p.page(), touch_dev::PAGE);
        // The first row is Shift across: + five times moves touches 5 px right.
        for _ in 0..5 {
            p.apply(ToUi::TouchStep(1));
        }
        assert_eq!(p.tuning().offset_x, 5.0);
        assert_eq!(
            p.filter.apply(7, Phase::Down, 100.0, 100.0),
            Some((105.0, 100.0))
        );
        p.filter.apply(7, Phase::Up, 100.0, 100.0);
        // The wheel to the last row (Save), and +: the tuning goes to the store.
        p.apply(ToUi::TouchWheel(99));
        p.apply(ToUi::TouchStep(1));
        let saved: Vec<FromUi> = p
            .take_actions()
            .into_iter()
            .filter(|a| matches!(a, FromUi::SaveTouch(_)))
            .collect();
        assert_eq!(saved.len(), 1);
        assert!(matches!(&saved[0], FromUi::SaveTouch(t) if t.offset_x == 5.0));
        // Defaults by touch: the Defaults button under the rows.
        let r = p.ui.rect_of(touch_dev::TUNE).unwrap();
        let mut hit = None;
        'find: for y in (r.y..r.bottom()).step_by(4) {
            for x in (r.x..r.right()).step_by(8) {
                let mut probe = p.ui.get::<TuneList>(touch_dev::TUNE).unwrap().clone();
                if mpfree_ui::Widget::touch(&mut probe, 1, Touch::Down, x, y, r)
                    == Some(EventKind::Tab(1))
                {
                    hit = Some((x, y));
                    break 'find;
                }
            }
        }
        let (x, y) = hit.expect("a Defaults button");
        // The touch is shifted 5 px right on its way in; the button is wide.
        p.touch(1, Touch::Down, x, y);
        p.touch(1, Touch::Up, x, y);
        assert_eq!(*p.tuning(), Tuning::default());
        // Targets take the whole page until the nine taps are in.
        let area = p.ui.rect_of(touch_dev::TEST).unwrap();
        let (bar, _) = TouchTest::body(area);
        let c = TouchTest::bar_cells(bar)[1];
        p.touch(2, Touch::Down, c.x + c.w / 2, c.y + c.h / 2);
        p.touch(2, Touch::Up, c.x + c.w / 2, c.y + c.h / 2);
        assert_eq!(p.page(), touch_dev::TARGETS_PAGE);
        let area = p.ui.rect_of(touch_dev::TEST).unwrap();
        for (i, (x, y)) in TouchTest::targets(TouchTest::body(area).1)
            .into_iter()
            .enumerate()
        {
            p.touch(10 + i as u64, Touch::Down, x, y);
            p.touch(10 + i as u64, Touch::Up, x, y);
        }
        assert_eq!(p.page(), touch_dev::PAGE);
        // Leaving keeps the tuning and goes back to Free.
        p.apply(ToUi::Show(View::Play));
        assert_eq!(
            p.ui.get::<TouchTest>(touch_dev::TEST).map(|t| t.mode),
            Some(touch_dev::Mode::Free)
        );
    }

    fn app_rows(app: &crate::app::App) -> usize {
        match app.menu_view() {
            crate::app::Effect::Menu(m) => m.rows.len(),
            _ => 0,
        }
    }

    #[test]
    fn before_ready_a_tap_shows_the_console_and_a_tab_goes_back() {
        let mut p = Player::new(Fonts::embedded(), SCREEN, false, None);
        assert_eq!(p.page(), "Loading");
        assert_eq!(p.touch(1, Touch::Down, 600, 300), None);
        assert_eq!(p.page(), "Console");
        p.touch(1, Touch::Up, 600, 300);
        let header = p.ui.rect_of("header").unwrap();
        assert_eq!(
            p.touch(2, Touch::Down, header.x + 170 + 20, header.y + 30),
            None
        );
        assert_eq!(p.page(), "Loading");
    }

    #[test]
    fn touches_become_actions_once_ready() {
        let mut p = sample_player(Fonts::embedded());
        // A step of the pattern on Play: on, then nothing more on the lift.
        let steps = p.ui.rect_of(play_page::STEPS).unwrap();
        assert!(p
            .touch(1, Touch::Down, steps.x + steps.w / 2, steps.y + steps.h / 2)
            .is_some());
        p.touch(1, Touch::Up, steps.x + steps.w / 2, steps.y + steps.h / 2);
        let header = p.ui.rect_of("header").unwrap();
        let tab1 = p.ui.get::<Header>("header").unwrap().tab_rect(header, 1);
        // The second tab: the app decides; the page stays until it says.
        assert_eq!(
            p.touch(2, Touch::Down, tab1.x + 20, tab1.y + 20),
            Some(FromUi::Tab(1))
        );
        assert_eq!(p.page(), "Play");
        p.apply(ToUi::Show(View::Debug));
        assert!(p.ui.rect_of("midi").is_some() && p.ui.rect_of(play_page::STEPS).is_none());
        // A menu row's arrow steps its value.
        let mut app = sample_app();
        let fx = app.button(mpfree_controller::mpc_one::button::MENU, true, 0.0);
        apply_effects(&mut p, fx);
        assert_eq!(p.page(), "Options");
        let r = p.ui.rect_of("options").unwrap();
        let o = p.ui.get::<OptionList>("options").unwrap();
        let (_, value) = o.row_rects(r, 1).unwrap();
        assert_eq!(
            p.touch(3, Touch::Down, value.right() - 5, value.y + 10),
            Some(FromUi::MenuStep(1, 1))
        );
    }

    #[test]
    fn kit_changes_relabel_the_pads() {
        let mut p = sample_player(Fonts::embedded());
        p.apply(ToUi::Kit(3));
        let g =
            p.ui.get::<MiniPads>(play_page::PADS)
                .map(|m| &m.grid)
                .unwrap();
        assert_eq!(g.pads[0].label, "Kick");
        assert!(g.pads[9].color.is_none(), "Lo-Fi has 9 pads");
        assert_eq!(p.ui.get::<ListView>("kits").unwrap().selected, 3);
        p.apply(ToUi::Kit(99)); // ignored
        assert_eq!(p.ui.get::<ListView>("kits").unwrap().selected, 3);
    }

    /// The kit views in the actions, still boxed (as they go back), and the rest.
    #[allow(clippy::vec_box)]
    fn views(acts: Vec<FromUi>) -> (Vec<Box<ViewSwap>>, Vec<FromUi>) {
        let (mut v, mut rest) = (Vec::new(), Vec::new());
        for a in acts {
            match a {
                FromUi::SampleView(s) => v.push(s),
                a => rest.push(a),
            }
        }
        (v, rest)
    }

    #[test]
    fn a_warped_pad_follows_the_tempo_and_plays_its_stretched_copy_when_ready() {
        use mpfree_sampler_ui::wave_tab::{Row, REGION};
        let pad = mpfree_sampler_ui::sample::PAD;
        let mut p = sample_sampler(Fonts::embedded());
        let warp_row = Row::ALL.iter().position(|r| *r == Row::Warp).unwrap();
        p.on_event(UiEvent {
            widget: REGION.into(),
            kind: EventKind::Step {
                index: warp_row,
                delta: 1,
            },
        });
        let region = p.sample.state().unwrap().region.clone();
        let w = region.warp.clone().expect("warped");
        assert_eq!(w.method, mpfree_sampler::model::WarpMethod::Stretch);
        // The first view repitches (no copy yet) and asks for the copy.
        let (sent, _) = views(p.take_actions());
        let layer = sent[0].view.as_ref().unwrap().layer(pad, 0).unwrap();
        assert_eq!(layer.stretched_frames(), None);
        p.apply(ToUi::SampleViewBack(sent.into_iter().next().unwrap()));
        // The copy comes back from the stretch thread: a new view plays it.
        let t = Instant::now();
        let sent = loop {
            let (sent, _) = views(p.take_actions());
            if !sent.is_empty() {
                break sent;
            }
            assert!(t.elapsed() < Duration::from_secs(30), "no copy");
            std::thread::sleep(Duration::from_millis(5));
        };
        let len = sent[0]
            .view
            .as_ref()
            .unwrap()
            .layer(pad, 0)
            .unwrap()
            .stretched_frames()
            .expect("the copy");
        let want = region.frames() as f64 * f64::from(w.bpm) / 120.0;
        assert!((len as f64 - want).abs() <= 1.0, "{len} vs {want}");
        assert!(
            events_log(&p)
                .iter()
                .any(|l| l.starts_with("sample: stretched")),
            "{:?}",
            events_log(&p)
        );
        p.apply(ToUi::SampleViewBack(sent.into_iter().next().unwrap()));
        // A new tempo: a view at once (repitching), then its own copy.
        let info = TransportInfo {
            bpm: 100.0,
            ..TransportInfo::default()
        };
        p.apply(ToUi::Transport(info));
        assert_eq!(p.sample.tempo(), 100.0);
        let (sent, _) = views(p.take_actions());
        assert_eq!(sent.len(), 1);
        assert_eq!(
            sent[0]
                .view
                .as_ref()
                .unwrap()
                .layer(pad, 0)
                .unwrap()
                .stretched_frames(),
            None
        );
    }

    #[test]
    fn the_warp_tab_pins_the_hits_and_the_copy_goes_through_them() {
        use mpfree_sampler_ui::warp_tab::{self, Row};
        use mpfree_sampler_ui::widgets::PadBar;
        let pad = mpfree_sampler_ui::sample::PAD;
        let mut p = sample_sampler(Fonts::embedded());
        // The pad bar's Warp tab, after Wave.
        let r = p.ui.rect_of(mpfree_sampler_ui::wave_tab::BAR).unwrap();
        let t =
            p.ui.get::<PadBar>(mpfree_sampler_ui::wave_tab::BAR)
                .unwrap()
                .tab_rect(r, warp_tab::TAB);
        assert_eq!(p.touch(9, Touch::Down, t.x + 5, t.y + 5), None);
        p.touch(9, Touch::Up, t.x + 5, t.y + 5);
        assert_eq!(
            (p.page(), p.sample.editor()),
            (sample_page::WARP_PAGE, sample_page::Editor::Warp)
        );
        assert!(p
            .ui
            .get::<Label>("hint")
            .unwrap()
            .text
            .contains("Tap a hit: pin it to the grid"));
        // Warp on (Stretch), then Pin hits.
        p.on_event(UiEvent {
            widget: warp_tab::LIST.into(),
            kind: EventKind::Step {
                index: Row::Warp.index(),
                delta: 1,
            },
        });
        let (sent, _) = views(p.take_actions());
        p.apply(ToUi::SampleViewBack(sent.into_iter().next().unwrap()));
        p.apply(ToUi::SampleLift);
        p.on_event(UiEvent {
            widget: warp_tab::BUTTONS.into(),
            kind: EventKind::Tab(0),
        });
        let w = p.sample.state().unwrap().region.warp.clone().unwrap();
        assert!(w.pins.len() >= 4, "{:?}", w.pins);
        // The wheel navigation (app::RowKey, live test 2): the wheel picks
        // the list's rows, its push opens one, and the wheel or - / + then
        // step it, as a tap on its arrow; SHIFT + - / + still zoom.
        {
            use crate::app::RowKey;
            use mpfree_sampler_ui::widgets::SettingList;
            let list = |p: &Player| {
                let l = p.ui.get::<SettingList>(warp_tab::LIST).unwrap();
                (
                    l.selected,
                    l.editing,
                    l.rows[Row::Grid.index()].value.clone(),
                )
            };
            assert_eq!(list(&p).0, Row::Warp.index());
            p.apply(ToUi::Rows(RowKey::Pick(Row::Grid.index() as i32)));
            assert_eq!(list(&p).0, Row::Grid.index());
            p.apply(ToUi::Rows(RowKey::Pick(-1)));
            p.apply(ToUi::Rows(RowKey::Pick(1)));
            let (at, editing, grid) = list(&p);
            assert_eq!((at, editing), (Row::Grid.index(), false));
            p.apply(ToUi::Rows(RowKey::Editing(true)));
            assert!(list(&p).1);
            p.apply(ToUi::Rows(RowKey::Step(1)));
            assert_ne!(list(&p).2, grid, "the wheel stepped Grid");
            p.apply(ToUi::Rows(RowKey::Step(-1)));
            assert_eq!(list(&p).2, grid, "and back");
            p.apply(ToUi::Rows(RowKey::Editing(false)));
            assert!(!list(&p).1);
            p.apply(ToUi::SampleWheel(1));
            p.apply(ToUi::Rows(RowKey::Pick(-99)));
            assert_eq!(list(&p).0, 0);
        }
        // The copy made through the pins comes back and is played.
        let t = Instant::now();
        let sent = loop {
            let (sent, _) = views(p.take_actions());
            if sent.iter().any(|s| {
                s.view
                    .as_ref()
                    .and_then(|v| v.layer(pad, 0))
                    .is_some_and(|l| l.stretched_frames().is_some())
            }) {
                break sent;
            }
            for s in sent {
                p.apply(ToUi::SampleViewBack(s));
            }
            assert!(t.elapsed() < Duration::from_secs(30), "no copy");
            std::thread::sleep(Duration::from_millis(5));
        };
        for s in sent {
            p.apply(ToUi::SampleViewBack(s));
        }
        // UNDO takes the pins off again.
        p.apply(ToUi::SampleLift);
        p.apply(ToUi::SampleUndo(false));
        let w = p.sample.state().unwrap().region.warp.clone().unwrap();
        assert!(w.pins.is_empty());
        assert_eq!(p.page(), sample_page::WARP_PAGE);
    }

    fn events_log(p: &Player) -> Vec<String> {
        p.ui.get::<LogPanel>("events")
            .unwrap()
            .lines()
            .map(|(l, _)| l.clone())
            .collect()
    }

    /// Ticks until `done` (the sample page's threads finish), for at most
    /// five seconds.
    fn tick_until(p: &mut Player, mut done: impl FnMut(&mut Player) -> bool) {
        let end = Instant::now() + Duration::from_secs(5);
        while !done(p) {
            assert!(Instant::now() < end, "{:?}", events_log(p));
            std::thread::sleep(Duration::from_millis(5));
            p.tick(0.033);
        }
    }

    #[test]
    fn the_kits_save_with_their_sounds_and_open_again() {
        use mpfree_sampler::{save, wav};
        use std::collections::HashMap;
        let pad = mpfree_sampler_ui::sample::PAD;
        let p = sample_sampler(Fonts::embedded());
        let (text, sounds) = p.sample.save_part();
        assert!(text.starts_with(save::FORMAT));
        let saved = p.sample.project(0).unwrap().clone();
        assert_eq!(sounds.len(), saved.sources().len());
        // What the library would keep in the project's samples/.
        let files: HashMap<String, Vec<u8>> = sounds
            .iter()
            .map(|(f, s)| (f.clone(), wav::bytes(s)))
            .collect();
        // Another session: the same kit slots, nothing on them.
        let mut q = sample_player(Fonts::embedded());
        q.apply(ToUi::SampleKits {
            projects: vec![Project::new()],
            rate: mpfree_sampler_ui::sample::RATE,
        });
        q.apply(ToUi::Kit(0));
        assert!(q.sample.project(0).unwrap().pad(pad).is_none());
        let _ = q.take_actions();
        q.sample.open_part(
            text,
            Box::new(move |f| files.get(f).cloned().ok_or_else(|| "missing".to_string())),
        );
        tick_until(&mut q, |q| q.sample.project(0).unwrap().pad(pad).is_some());
        let got = q.sample.project(0).unwrap();
        assert_eq!(got.pad(pad), saved.pad(pad));
        assert_eq!(got.chop_sets, saved.chop_sets);
        assert!(!got.can_undo(), "UNDO doesn't take the opened kit apart");
        let log = events_log(&q);
        assert!(log.iter().any(|l| l == "sample: 1 kit opened"), "{log:?}");
        // The Play page's names follow, and the audio thread gets the kit.
        assert_eq!(q.kits[0].pads[pad].as_deref(), Some("Break 92"));
        assert!(q.sample.take_view().is_some());
        // A part that isn't one says so and leaves the kits alone.
        q.sample
            .open_part("hello".into(), Box::new(|_| Err("no".to_string())));
        tick_until(&mut q, |q| {
            events_log(q)
                .iter()
                .any(|l| l.starts_with("sample: the project's kits weren't opened"))
        });
        assert!(q.sample.project(0).unwrap().pad(pad).is_some());
    }

    /// BROWSE, SHIFT + BROWSE, Open and New project through the screen, on
    /// a library in a scratch folder.
    #[test]
    fn a_project_saves_opens_again_and_new_starts_empty() {
        use crate::project::{self, Collected, Defaults};
        use mpfree_browser_ui::sample::{select, source};
        use mpfree_browser_ui::{Button, Input};
        use mpfree_fx::ids::mixer as mx;
        use mpfree_fx::StripId;
        use mpfree_midi::setup::{PortInput, TempoSource, DIN_NAME};
        let dir = std::env::temp_dir().join(format!("mpfree-project-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let lib = mpfree_library::Library::at(&dir).unwrap();
        let mut p = sample_sampler(Fonts::embedded());
        p.set_browser(mpfree_browser_ui::Browser::inline(lib.clone()), "");
        p.take_actions();
        // The I/O setup: its song settings go in the project (Tempo from a
        // port, here), the rig's (a port's input) stay with the MPC.
        let io: mpfree_midi_ui::Shared =
            std::sync::Arc::new(std::sync::Mutex::new(mpfree_midi_ui::sample::rig()));
        p.apply(ToUi::IoModel(io.clone()));
        let keystep = || io.lock().unwrap().setup.port("Keystep 37").input;
        let tempo_from = || io.lock().unwrap().setup.tempo.clone();
        assert_eq!(keystep(), PortInput::LearnOnly);
        // BROWSE: the whole browser over the page.
        p.apply(ToUi::Show(View::Browser));
        assert_eq!(p.page(), mpfree_browser_ui::BROWSER_PAGE);
        // SHIFT + BROWSE: the loop's parts come, the screen adds its own and
        // the lot is saved, named for the time.
        let mut mix = crate::mixing::MixHost::new(44_100, -6.0, 0.0);
        mix.mixer.set_now(StripId::Track(2), mx::VOLUME, -3.0);
        for _ in 0..50 {
            mix.run_silent_period();
        }
        let mut clips = crate::time::TimeHost::new(44_100, 98.0)
            .unwrap()
            .project()
            .clone();
        let mut clip = mpfree_seq::Clip::new("Groove", 1920);
        clip.add_note(mpfree_seq::Note::new(0, 240, 36, 100))
            .unwrap();
        clips.tracks[0].slots[0] = Some(clip);
        let synth = crate::synth::Synth::new(44_100);
        // Loop 2 with one layer, as the looper's save gives it.
        let layer: Vec<f32> = (0..2 * 4410).map(|k| (k % 97) as f32 / 200.0).collect();
        let loops = mpfree_looper::save::Saved {
            rate: 44_100,
            selected: 1,
            loops: vec![(
                1,
                mpfree_looper::save::LoopFile {
                    frames: 4410,
                    ticks: 96,
                    origin: 0,
                    level: 0.8,
                    muted: false,
                    bars: Some(1),
                    after: mpfree_looper::After::Play,
                    fade_old: 1.0,
                    layers: vec![mpfree_looper::save::LayerFile {
                        level: 0.5,
                        pan: -0.25,
                        muted: false,
                        solo: false,
                        faded: 1.0,
                        file: String::new(),
                        audio: layer.clone(),
                    }],
                },
            )],
        };
        // Audio 2 with a region of a mono take, as the tracks' save gives it.
        let take: Vec<f32> = (0..4410).map(|k| (k % 89) as f32 / 300.0).collect();
        let audio = mpfree_audiotracks::save::Saved {
            rate: 44_100,
            tracks: vec![mpfree_audiotracks::save::TrackFile {
                index: 1,
                input: mpfree_audiotracks::Input::Right,
                monitor: mpfree_audiotracks::Monitor::Auto,
                level: 0.5,
                pan: 0.0,
                muted: false,
                solo: true,
                regions: vec![mpfree_audiotracks::Region {
                    start: 0,
                    end: 960,
                    take: 4,
                    src: 0,
                    gain: 1.0,
                    fade_in: 0,
                    fade_out: 0,
                }],
            }],
            takes: vec![mpfree_audiotracks::save::TakeFile {
                id: 4,
                track: 1,
                group: 1,
                pass: 0,
                bpm: 98.0,
                anchor: 0.0,
                frames: 4410,
                channels: 1,
                file: String::new(),
                audio: take.clone(),
            }],
        };
        let c = Collected {
            tempo: 98.0,
            swing: mpfree_transport::time::Swing::new(240, 62).unwrap(),
            clips: clips.clone(),
            mixer: project::mixer_text(&mix.mixer, 0),
            modu: String::new(),
            synth: project::synth_text(&Default::default(), &synth.sounds()),
            arp: "mpfree-arp 1\ntrack 1\nmode = arp\n".to_string(),
            instruments: String::new(),
            loops: Some(loops),
            audio: Some(audio),
        };
        p.apply(ToUi::ProjectParts(Box::new(c)));
        p.step_browser();
        let loc = p
            .browser
            .as_ref()
            .unwrap()
            .project()
            .cloned()
            .expect("saved");
        let snap = lib.open_project(&loc).unwrap();
        for part in [
            project::CLIPS,
            project::MIXER,
            project::MOD,
            project::SYNTH,
            project::SAMPLER,
            project::LOOPER,
            project::ARP,
            project::MIDI,
            project::AUDIOTRACKS,
        ] {
            assert!(snap.part(part).is_some(), "{part}");
        }
        assert_eq!(snap.meta.tempo, 98.0);
        assert!(events_log(&p)
            .iter()
            .any(|l| *l == format!("project: saved {}", loc.name())));
        // Open it from the Projects list: the engines come built, for the
        // loop to swap in, and the I/O setup's song settings come back.
        io.lock().unwrap().setup.tempo = TempoSource::Internal;
        let b = p.browser.as_mut().unwrap();
        b.go_source(source("Projects"));
        select(b, loc.name());
        b.push();
        p.step_browser();
        let opened = p
            .take_actions()
            .into_iter()
            .find_map(|a| match a {
                FromUi::OpenProject(o) => Some(o),
                _ => None,
            })
            .expect("opened");
        assert_eq!(tempo_from(), TempoSource::Port(DIN_NAME.into()));
        assert_eq!(opened.tempo, Some(98.0));
        assert_eq!(opened.swing, mpfree_transport::time::Swing::new(240, 62));
        assert!(opened.notes.is_empty(), "{:?}", opened.notes);
        assert_eq!(opened.time.as_ref().unwrap().project(), &clips);
        let mixed = opened.mix.as_ref().unwrap();
        let t2 = mixed
            .strips
            .iter()
            .find(|(s, _)| *s == StripId::Track(2))
            .unwrap();
        assert!(t2.1.contains(&(mx::VOLUME, -3.0)));
        assert_eq!(opened.synth.as_ref().unwrap().1, synth.sounds());
        assert!(opened.clear_loops);
        assert!(opened.clear_audio);
        assert_eq!(
            opened.arp.as_deref(),
            Some("mpfree-arp 1\ntrack 1\nmode = arp\n")
        );
        p.apply(ToUi::ProjectBack(opened));
        // The loops and the audio tracks follow, each read on its own
        // thread, with their audio.
        let end = Instant::now() + Duration::from_secs(5);
        let (mut loops, mut audio) = (None, None);
        while loops.is_none() || audio.is_none() {
            p.tick(0.033);
            for a in p.take_actions() {
                match a {
                    FromUi::OpenLoops(l) => loops = Some(l),
                    FromUi::OpenAudio(a) => audio = Some(a),
                    _ => {}
                }
            }
            assert!(Instant::now() < end, "{:?}", events_log(&p));
            std::thread::sleep(Duration::from_millis(5));
        }
        let (loops, audio) = (loops.unwrap(), audio.unwrap());
        assert_eq!((loops.selected, loops.loops.len()), (1, 1));
        let (n, f) = &loops.loops[0];
        assert_eq!((*n, f.frames, f.level), (1, 4410, 0.8));
        assert_eq!(f.layers[0].pan, -0.25);
        assert_eq!(f.layers[0].audio, layer);
        assert!(f.layers[0].file.starts_with("loop-"));
        // The audio tracks, with their take's audio from its WAV.
        assert_eq!(audio.tracks.len(), 1);
        let t = &audio.tracks[0];
        assert_eq!(
            (t.index, t.input, t.level, t.solo),
            (1, mpfree_audiotracks::Input::Right, 0.5, true)
        );
        assert_eq!(t.regions[0].take, 4);
        let k = &audio.takes[0];
        assert_eq!((k.id, k.frames, k.channels), (4, 4410, 1));
        assert_eq!(k.audio, take);
        assert!(
            k.file.starts_with("take-") && k.file.ends_with(".wav"),
            "{}",
            k.file
        );
        assert!(lib.sample_path(&loc, &k.file).is_ok());
        assert!(!p.browser.as_ref().unwrap().unsaved());
        // Once the open settles, a pad played isn't an edit; a new clip is.
        p.opened_at = None;
        p.saw_action(&FromUi::PreviewStop);
        p.saw_action(&FromUi::Time(TimeAction::Transport(
            mpfree_transport::Command::Play,
        )));
        assert!(!p.browser.as_ref().unwrap().unsaved());
        p.saw_action(&FromUi::Time(TimeAction::NewClip { track: 0, slot: 1 }));
        assert!(p.browser.as_ref().unwrap().unsaved());
        // A Synth tab option or the arp changed with the controls.
        p.browser.as_mut().unwrap().set_unsaved(false);
        p.apply(ToUi::Edited);
        assert!(p.browser.as_ref().unwrap().unsaved());
        // New project before the sound has started says so.
        let new = |p: &mut Player| {
            let b = p.browser.as_mut().unwrap();
            b.go_source(source("Projects"));
            b.input(Input::Button(Button::NewProject));
            // Over unsaved work it asks first; the push says yes.
            b.push();
            p.step_browser();
            // As on a slow machine: the kits took longer than the settle
            // time. What New sent is still its own, not an edit.
            p.opened_at = p.opened_at.and_then(|t| t.checked_sub(OPEN_SETTLE * 2));
            p.take_actions().into_iter().find_map(|a| match a {
                FromUi::OpenProject(o) => Some(o),
                _ => None,
            })
        };
        assert!(new(&mut p).is_none());
        assert!(events_log(&p)
            .iter()
            .any(|l| l.starts_with("project: not ready for a new project")));
        // With the start's parts: an empty song of 8 tracks, the factory
        // kits, and no project (Save names a new one).
        let defaults = || {
            Box::new(Defaults {
                tempo: 120.0,
                mixer: project::mixer_text(
                    &crate::mixing::MixHost::new(44_100, -6.0, 0.0).mixer,
                    0,
                ),
                modu: String::new(),
                synth: project::synth_text(&Default::default(), &synth.sounds()),
            })
        };
        p.apply(ToUi::ProjectDefaults(defaults()));
        p.browser.as_mut().unwrap().set_unsaved(true);
        let fresh = new(&mut p).expect("a new project");
        assert_eq!(fresh.what, "a new project");
        assert!(fresh.clear_loops);
        assert!(fresh.clear_audio);
        assert_eq!(fresh.tempo, Some(120.0));
        assert_eq!(fresh.swing, Some(Default::default()));
        assert_eq!(fresh.arp, None, "every arp at its default");
        // The I/O setup's song settings at their defaults; the rig's kept.
        assert_eq!(tempo_from(), TempoSource::Internal);
        assert!(io.lock().unwrap().setup.tracks.iter().all(Option::is_none));
        assert_eq!(keystep(), PortInput::LearnOnly);
        let song = fresh.time.as_ref().unwrap().project();
        assert_eq!(song.tracks.len(), crate::time::TRACKS);
        assert!(song
            .tracks
            .iter()
            .all(|t| t.slots.iter().all(Option::is_none)));
        assert!(fresh.notes.is_empty(), "{:?}", fresh.notes);
        assert!(p.browser.as_ref().unwrap().project().is_none());
        assert!(!p.browser.as_ref().unwrap().unsaved());

        // The next start, with Prefs > At start on Last project: once the
        // sound is going, the project saved above opens, and the Pads
        // tab's picker stays where it is.
        let lists = p.browser.as_ref().unwrap().lists_text();
        assert!(
            lists.contains(&format!("project = {}\n", loc.to_line())),
            "{lists}"
        );
        let opens = |q: &mut Player| {
            q.tick(0.033);
            q.take_actions().into_iter().find_map(|a| match a {
                FromUi::OpenProject(o) => Some(o),
                _ => None,
            })
        };
        let last = Settings {
            open_last: true,
            ..Default::default()
        };
        let mut q = sample_sampler(Fonts::embedded());
        q.set_browser(mpfree_browser_ui::Browser::inline(lib.clone()), &lists);
        q.take_actions();
        q.apply(ToUi::Settings(last.clone()));
        assert!(opens(&mut q).is_none(), "not before the sound is going");
        q.apply(ToUi::ProjectDefaults(defaults()));
        let at_start = opens(&mut q).expect("the last project opens");
        assert_eq!(at_start.what, loc.name());
        assert!(q.browser.as_ref().unwrap().picking().is_some());
        assert!(opens(&mut q).is_none(), "once");
        // New project (the default) leaves the empty one.
        let mut n = sample_sampler(Fonts::embedded());
        n.set_browser(mpfree_browser_ui::Browser::inline(lib.clone()), &lists);
        n.take_actions();
        n.apply(ToUi::ProjectDefaults(defaults()));
        assert!(opens(&mut n).is_none());
        // After a crash (a safe start, crash.rs) the last project does
        // not open, and the screen says so.
        let mut c = sample_sampler(Fonts::embedded());
        c.set_browser(mpfree_browser_ui::Browser::inline(lib.clone()), &lists);
        c.take_actions();
        c.safe_start();
        c.apply(ToUi::Settings(last.clone()));
        c.apply(ToUi::ProjectDefaults(defaults()));
        assert!(opens(&mut c).is_none(), "a safe start opens nothing");
        let said = crate::crash::screen_line(true, Some(loc.name()), None);
        assert!(said.contains(loc.name()), "{said}");
        assert!(
            events_log(&c)
                .iter()
                .any(|l| *l == format!("project: safe start: {said}")),
            "{:?}",
            events_log(&c)
        );
        assert_eq!(
            c.told.as_ref().map(|t| t.0.clone()),
            Some(tell_text(&said)),
            "on the hint line"
        );
        assert!(opens(&mut c).is_none(), "nor later");
        // A last project gone since is said; the new one stays.
        let mut r = sample_sampler(Fonts::embedded());
        r.set_browser(
            mpfree_browser_ui::Browser::inline(lib.clone()),
            "project = Projects/Nowhere\n",
        );
        r.take_actions();
        r.apply(ToUi::Settings(last));
        r.apply(ToUi::ProjectDefaults(defaults()));
        assert!(opens(&mut r).is_none());
        assert!(
            events_log(&r)
                .iter()
                .any(|l| l.starts_with("project: the last project wasn't opened")),
            "{:?}",
            events_log(&r)
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Autosave (Jay's pick, 14 Nov): unsaved work edited since the last
    /// autosave goes, every 2 minutes, into autosave's own project; a save
    /// the musician asks for wins. At the next start, work autosaved after
    /// the last save is set aside and the screen says where.
    #[test]
    fn unsaved_work_is_autosaved_and_set_aside_at_the_next_start() {
        use crate::project::{self, Collected, Defaults};
        use mpfree_library::project::AUTOSAVE;
        use mpfree_library::{Loc, Meta, Place, Snapshot};
        let dir = std::env::temp_dir().join(format!("mpfree-autosave-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let lib = mpfree_library::Library::at(&dir).unwrap();
        let synth = crate::synth::Synth::new(44_100);
        let mixer = project::mixer_text(&crate::mixing::MixHost::new(44_100, -6.0, 0.0).mixer, 0);
        let synth_text = project::synth_text(&Default::default(), &synth.sounds());
        let defaults = || {
            Box::new(Defaults {
                tempo: 120.0,
                mixer: mixer.clone(),
                modu: String::new(),
                synth: synth_text.clone(),
            })
        };
        let parts = |tempo: f64| {
            Box::new(Collected {
                tempo,
                swing: Default::default(),
                clips: crate::time::TimeHost::new(44_100, tempo)
                    .unwrap()
                    .project()
                    .clone(),
                mixer: mixer.clone(),
                modu: String::new(),
                synth: synth_text.clone(),
                arp: String::new(),
                instruments: String::new(),
                loops: None,
                audio: None,
            })
        };
        // Seconds of ticks, one a second: whether the parts were asked for.
        let asks = |p: &mut Player, secs: usize| {
            let mut asked = false;
            for _ in 0..secs {
                p.tick(1.0);
                asked |= p
                    .take_actions()
                    .iter()
                    .any(|a| matches!(a, FromUi::SaveProject));
            }
            asked
        };
        let edit = |p: &mut Player| {
            p.opened_at = None;
            p.saw_action(&FromUi::Time(TimeAction::NewClip { track: 0, slot: 1 }));
            assert!(p.browser.as_ref().unwrap().unsaved());
        };
        let autosave = Loc::top(Place::Projects).join(AUTOSAVE).unwrap();
        let mut p = sample_sampler(Fonts::embedded());
        p.set_browser(mpfree_browser_ui::Browser::inline(lib.clone()), "");
        p.take_actions();
        p.apply(ToUi::ProjectDefaults(defaults()));
        // Nothing unsaved, nothing autosaved.
        assert!(!asks(&mut p, 130));
        // Two minutes after an edit, the parts are asked for, and go to
        // autosave's project, quietly: the work stays unsaved and new.
        edit(&mut p);
        assert!(!asks(&mut p, 119));
        assert!(asks(&mut p, 2));
        let log = events_log(&p).len();
        p.apply(ToUi::ProjectParts(parts(101.0)));
        p.step_browser();
        let back = lib.open_project(&autosave).unwrap();
        assert!(back.meta.is_autosave());
        assert_eq!(back.meta.tempo, 101.0);
        assert_eq!(back.meta.name, "Autosave of a new project");
        assert_eq!(events_log(&p).len(), log, "not in the Events log");
        let b = p.browser.as_ref().unwrap();
        assert!(b.unsaved() && b.project().is_none());
        // No edit since: nothing more.
        assert!(!asks(&mut p, 130));
        // Parts asked for that never come are asked for again later; a
        // save the musician asks for meanwhile wins.
        edit(&mut p);
        assert!(asks(&mut p, 121));
        assert!(!asks(&mut p, 237));
        assert!(asks(&mut p, 4));
        p.ask_save();
        assert!(p
            .take_actions()
            .iter()
            .any(|a| matches!(a, FromUi::SaveProject)));
        p.apply(ToUi::ProjectParts(parts(102.0)));
        p.step_browser();
        let saved = p
            .browser
            .as_ref()
            .unwrap()
            .project()
            .cloned()
            .expect("saved");
        assert!(saved.name().starts_with("Project "), "{}", saved.name());
        assert_eq!(lib.open_project(&saved).unwrap().meta.tempo, 102.0);
        assert_eq!(lib.open_project(&autosave).unwrap().meta.tempo, 101.0);
        assert!(!p.browser.as_ref().unwrap().unsaved());
        assert!(!asks(&mut p, 130), "saved: nothing to autosave");

        // The next start, after a crash: the work autosaved after the last
        // save is set aside before anything autosaves, and the safe start
        // says where.
        let at = |secs: u64, tempo: f64| Snapshot {
            meta: Meta {
                tempo,
                saved: secs,
                ..Meta::default()
            },
            parts: vec![("transport".into(), format!("bpm = {tempo}\n"))],
        };
        let old = Loc::top(Place::Projects).join("Old").unwrap();
        lib.save_project(&old, &at(1_000, 90.0), 0).unwrap();
        let autosaved = |secs: u64| {
            let mut s = at(secs, 95.0);
            s.meta.mark_autosave();
            lib.save_project(&lib.autosave_project().unwrap(), &s, 0)
                .unwrap();
        };
        autosaved(2_000); // 1970-01-01 00:33
        let lists = "project = Projects/Old\n";
        let last = Settings {
            open_last: true,
            ..Default::default()
        };
        let opens = |q: &mut Player| {
            q.tick(0.033);
            q.take_actions().into_iter().find_map(|a| match a {
                FromUi::OpenProject(o) => Some(o),
                _ => None,
            })
        };
        let mut c = sample_sampler(Fonts::embedded());
        c.set_browser(mpfree_browser_ui::Browser::inline(lib.clone()), lists);
        c.take_actions();
        c.safe_start();
        c.apply(ToUi::Settings(last.clone()));
        c.apply(ToUi::ProjectDefaults(defaults()));
        assert!(opens(&mut c).is_none());
        let kept = Loc::top(Place::Projects)
            .join("Unsaved 1970-01-01 00.33")
            .unwrap();
        assert_eq!(lib.open_project(&kept).unwrap().meta.tempo, 95.0);
        assert!(lib.open_project(&autosave).is_err());
        let said = crate::crash::screen_line(true, Some("Old"), Some(kept.name()));
        assert!(
            events_log(&c)
                .iter()
                .any(|l| *l == format!("project: safe start: {said}")),
            "{:?}",
            events_log(&c)
        );
        assert_eq!(c.told.as_ref().map(|t| t.0.clone()), Some(tell_text(&said)));
        // After the power switch (a normal start): set aside too, said,
        // and the last project opens as Prefs > At start says.
        autosaved(3_000); // 00:50
        let mut n = sample_sampler(Fonts::embedded());
        n.set_browser(mpfree_browser_ui::Browser::inline(lib.clone()), lists);
        n.take_actions();
        n.apply(ToUi::Settings(last));
        n.apply(ToUi::ProjectDefaults(defaults()));
        assert_eq!(opens(&mut n).expect("the last project").what, "Old");
        let words = "Unsaved work from the last run is in Projects > Unsaved 1970-01-01 00.50.";
        assert!(
            events_log(&n)
                .iter()
                .any(|l| *l == format!("project: {words}")),
            "{:?}",
            events_log(&n)
        );
        assert!(lib
            .open_project(
                &Loc::top(Place::Projects)
                    .join("Unsaved 1970-01-01 00.50")
                    .unwrap()
            )
            .is_ok());
        // Autosaved before the last save: nothing to set aside or say.
        autosaved(500);
        let mut s = sample_sampler(Fonts::embedded());
        s.set_browser(mpfree_browser_ui::Browser::inline(lib.clone()), lists);
        s.take_actions();
        s.apply(ToUi::ProjectDefaults(defaults()));
        assert!(opens(&mut s).is_none());
        assert!(!events_log(&s).iter().any(|l| l.contains("Unsaved work")));
        assert!(lib.open_project(&autosave).is_ok(), "left in place");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_exported_pad_loads_back_with_its_settings_and_slices() {
        use mpfree_sampler_ui::chop_tab::{self, Row};
        let pad = mpfree_sampler_ui::sample::PAD;
        let mut p = sample_sampler(Fonts::embedded());
        let put_on = |p: &mut Player, delta: i32| {
            let index = Row::ALL.iter().position(|r| *r == Row::Put).unwrap();
            p.on_event(UiEvent {
                widget: chop_tab::LIST.into(),
                kind: EventKind::Step { index, delta },
            });
        };
        p.on_event(UiEvent {
            widget: mpfree_sampler_ui::wave_tab::BAR.into(),
            kind: EventKind::Tab(chop_tab::TAB),
        });
        // Files is offered only once the app can write them.
        put_on(&mut p, -1);
        assert_ne!(p.sample.chop().unwrap().0.put_text(), "Files");
        put_on(&mut p, 1);
        p.sample.set_files(&mut p.ui, true);
        put_on(&mut p, -1);
        assert_eq!(p.sample.chop().unwrap().0.put_text(), "Files");
        let n = p.sample.chop().unwrap().0.slices();
        p.on_event(UiEvent {
            widget: chop_tab::BUTTONS.into(),
            kind: EventKind::Tab(2),
        });
        let exports = p.sample.take_exports();
        assert_eq!(exports.len(), 1);
        let e = &exports[0];
        assert_eq!((e.name.as_str(), e.slices), ("Break 92", n));
        assert_eq!(e.files.len(), 2 + n, "the sound, its settings, the slices");
        assert!(
            events_log(&p).iter().any(|l| l
                == &format!(
                    "sample: pad A5 exported as {} files: Break 92.wav with its settings \
                     (Break 92.mpfree) and {n} slices (Break 92 01.wav to Break 92 {n:02}.wav)",
                    2 + n
                )),
            "{:?}",
            events_log(&p)
        );
        assert!(views(p.take_actions()).0.is_empty(), "the kit is as it was");
        // Written into a folder of their own, Load to pad brings the pad
        // back on another pad, slices and all, as one undo step.
        let dir = std::env::temp_dir().join(format!("mpfree-export-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for (name, bytes) in &e.files {
            std::fs::write(dir.join(name), bytes).unwrap();
        }
        let chops = p.sample.project(0).unwrap().chop_sets.len();
        p.sample.load_file(dir.join("Break 92.wav"), true);
        tick_until(&mut p, |p| p.sample.assigning().is_some());
        p.apply(ToUi::PadDown {
            pad: 9,
            velocity: 100,
        });
        p.apply(ToUi::PadUp(9));
        p.tick(0.033);
        let kit = p.sample.project(0).unwrap();
        assert_eq!(kit.pad(9), kit.pad(pad), "the pad as it was");
        assert_eq!(kit.chop_sets.len(), chops + 1);
        assert_eq!(kit.chop_sets.last().unwrap().slices(), n);
        assert_eq!(p.kits[0].pads[9].as_deref(), Some("Break 92"));
        assert!(events_log(&p).iter().any(|l| l
            == "sample: loaded: Break 92 on pad A10, with its settings and its slices from Break 92.mpfree"));
        p.apply(ToUi::SampleUndo(false));
        let kit = p.sample.project(0).unwrap();
        assert!(kit.pad(9).is_none());
        assert_eq!(kit.chop_sets.len(), chops);
        // A slice loads as a sound of its own; settings that can't be
        // read leave the sound alone, and say so.
        p.sample.load_file(dir.join("Break 92 02.wav"), false);
        tick_until(&mut p, |p| {
            events_log(p)
                .iter()
                .any(|l| l.starts_with("sample: loaded: Break 92 02, "))
        });
        std::fs::write(dir.join("Break 92.mpfree"), "junk").unwrap();
        p.sample.load_file(dir.join("Break 92.wav"), false);
        tick_until(&mut p, |p| {
            events_log(p).iter().any(|l| {
                l == "sample: Break 92: its settings (Break 92.mpfree) not used \
                      (not an mpfree-sampler 1 part); the sound alone is loaded"
            })
        });
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn load_puts_a_sound_on_the_pad_shown_and_load_to_pad_on_the_next_hit() {
        use mpfree_sampler::model::Source;
        use mpfree_sampler::wav;
        let dir = std::env::temp_dir().join(format!("mpfree-sample-load-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let wav_of = |name: &str, n: usize| {
            let data = (0..n)
                .map(|i| ((i % 200) as f32 - 100.0) / 32_768.0)
                .collect();
            let s = Source::new("x", 44_100, 1, data).unwrap();
            let path = dir.join(name);
            std::fs::write(&path, wav::bytes(&s)).unwrap();
            path
        };
        let pad = mpfree_sampler_ui::sample::PAD;
        let mut p = sample_sampler(Fonts::embedded());
        // Load: onto the pad shown, named for the file, one undo step.
        p.sample.load_file(wav_of("Clap 2.wav", 4_410), false);
        tick_until(&mut p, |p| {
            p.sample.project(0).unwrap().pad(pad).unwrap().name == "Clap 2"
        });
        assert!(events_log(&p)
            .iter()
            .any(|l| l.starts_with("sample: loaded: Clap 2, ") && l.contains("mono on pad A")));
        assert_eq!(p.kits[0].pads[pad].as_deref(), Some("Clap 2"));
        // Load to pad: held until a pad is hit, then it lands there.
        p.sample.load_file(wav_of("Rim.WAV", 2_000), true);
        tick_until(&mut p, |p| p.sample.assigning().is_some());
        assert_eq!(p.sample.assigning(), Some("Rim"));
        assert!(events_log(&p)
            .iter()
            .any(|l| l == "sample: hit a pad to put Rim on it"));
        p.apply(ToUi::PadDown {
            pad: 9,
            velocity: 100,
        });
        p.apply(ToUi::PadUp(9));
        p.tick(0.033);
        assert_eq!(p.sample.assigning(), None);
        assert_eq!(p.sample.project(0).unwrap().pad(9).unwrap().name, "Rim");
        assert_eq!(p.kits[0].pads[9].as_deref(), Some("Rim"));
        // A FLAC loads too, onto the pad shown (now the one just hit),
        // known by what it holds; one cut short says so.
        let flac = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../mpfree-flac/tests/data/s16-stereo-auto.flac"
        ))
        .unwrap();
        std::fs::write(dir.join("Loop.flac"), &flac[..flac.len() * 3 / 4]).unwrap();
        p.sample.load_file(dir.join("Loop.flac"), false);
        tick_until(&mut p, |p| p.kits[0].pads[9].as_deref() == Some("Loop"));
        let log = events_log(&p);
        assert!(log
            .iter()
            .any(|l| l == "sample: Loop: the file is cut short: its first 2304 frames"));
        assert!(log
            .iter()
            .any(|l| l == "sample: loaded: Loop, 52 ms stereo on pad A10"));
        // What it can't load, it says.
        std::fs::write(dir.join("Pad.aif"), b"not a sound").unwrap();
        p.sample.load_file(dir.join("Pad.aif"), false);
        tick_until(&mut p, |p| {
            events_log(p)
                .iter()
                .any(|l| l == "sample: not loaded (not a WAV, AIFF or FLAC file)")
        });
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_sample_page_edits_the_pad_hit_last_and_sends_one_view_at_a_time() {
        use mpfree_sampler_ui::wave_tab::{BAR, QLINKS};
        use mpfree_sampler_ui::widgets::{KnobCard, PadBar};
        let pad = mpfree_sampler_ui::sample::PAD;
        let mut p = sample_sampler(Fonts::embedded());
        assert_eq!(p.page(), sample_page::PAGE);
        assert_eq!(
            p.ui.get::<Header>("header").unwrap().active,
            5,
            "the Sample tab, after Looper"
        );
        assert_eq!(p.ui.get::<PadBar>(BAR).unwrap().title, "Pad A5 · Break 92");
        assert!(p
            .ui
            .get::<Label>("hint")
            .unwrap()
            .text
            .contains("Q-Links 1-4: Start, End, loop"));
        let (start, end) = (
            p.sample.state().unwrap().region.start,
            p.sample.state().unwrap().region.end,
        );
        // Q-Link 1 moves Start; the kit's view goes to the audio thread.
        p.apply(ToUi::SampleQLink(0, 4));
        let moved = p.sample.state().unwrap().region.start;
        assert!(moved > start);
        assert_eq!(
            p.sample.project(0).unwrap().pad(pad).unwrap().layers[0]
                .region
                .start,
            moved
        );
        let (sent, rest) = views(p.take_actions());
        assert!(rest.is_empty());
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].kit, 0);
        assert_eq!(
            sent[0].view.as_ref().unwrap().revision,
            p.sample.project(0).unwrap().revision()
        );
        // While it travels, more turns wait; they go together once it is back.
        p.apply(ToUi::SampleQLink(0, 2));
        let moved = p.sample.state().unwrap().region.start;
        p.apply(ToUi::SampleQLink(1, -3));
        assert!(p.sample.state().unwrap().region.end < end);
        assert!(views(p.take_actions()).0.is_empty());
        p.apply(ToUi::SampleViewBack(sent.into_iter().next().unwrap()));
        let (sent, _) = views(p.take_actions());
        assert_eq!(
            sent[0].view.as_ref().unwrap().revision,
            p.sample.project(0).unwrap().revision()
        );
        // A view that finds no room is built again.
        p.unsent(FromUi::SampleView(sent.into_iter().next().unwrap()));
        let (sent, _) = views(p.take_actions());
        assert_eq!(sent.len(), 1);
        p.apply(ToUi::SampleViewBack(sent.into_iter().next().unwrap()));
        assert!(views(p.take_actions()).0.is_empty(), "nothing new to send");
        // Each turn is one log line, written when the next one starts or
        // the Q-Link is let go.
        p.apply(ToUi::SampleLift);
        let lines: Vec<String> = events_log(&p)
            .into_iter()
            .filter(|l| l.starts_with("sample: "))
            .collect();
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(
            lines[0].starts_with("sample: kit 1 (Breaks) pad A5 Break 92: Start 0.163 s -> "),
            "{lines:?}"
        );
        assert!(
            lines[1].starts_with("sample: kit 1 (Breaks) pad A5 Break 92: End 4.891 s -> "),
            "{lines:?}"
        );
        // Each turn is one undo step: End first, then both Start turns.
        p.apply(ToUi::SampleUndo(false));
        assert_eq!(
            (
                p.sample.state().unwrap().region.start,
                p.sample.state().unwrap().region.end
            ),
            (moved, end)
        );
        p.apply(ToUi::SampleUndo(false));
        p.apply(ToUi::SampleUndo(false));
        assert_eq!(p.sample.state().unwrap().region.start, start);
        assert!(events_log(&p)
            .iter()
            .any(|l| l.starts_with("sample: nothing to undo in kit 1")));
        assert_eq!(views(p.take_actions()).0.len(), 1);
        // A pad without a sound says so and where to give it one; - / +
        // find the next sound.
        p.apply(ToUi::PadDown {
            pad: 1,
            velocity: 90,
        });
        assert_eq!(p.page(), sample_page::EMPTY_PAGE);
        let card = p.ui.get::<Card>(sample_page::EMPTY).unwrap();
        assert_eq!(card.heading, "Pad A2 has no sound in kit 1 (Breaks)");
        assert!(card.lines.iter().any(|(l, _)| l.contains("the Pads tab")));
        p.apply(ToUi::SamplePad(1));
        assert_eq!(
            (p.page(), p.sample.state().map(|s| s.pad)),
            (sample_page::PAGE, Some(pad))
        );
        // Q-Link 3 is the loop start; another kit names its bank.
        assert!(p.ui.get::<KnobCard>(QLINKS[2]).unwrap().enabled);
        p.apply(ToUi::Kit(1));
        assert_eq!(
            p.page(),
            sample_page::EMPTY_PAGE,
            "kit 2 has no project here"
        );
        p.apply(ToUi::Kit(sample_kits().len()));
        assert_eq!(
            p.ui.get::<Card>(sample_page::EMPTY).unwrap().heading,
            "The pads play the synth"
        );
    }

    #[test]
    fn a_tap_on_the_wave_plays_the_pad_and_a_looping_one_until_the_next_tap() {
        use mpfree_sampler_ui::wave_tab::WAVE;
        let pad = mpfree_sampler_ui::sample::PAD;
        let mut p = sample_sampler(Fonts::embedded());
        let w = p.ui.rect_of(WAVE).unwrap();
        let tap = |p: &mut Player, f: u64| {
            assert_eq!(
                p.touch(f, Touch::Down, w.x + w.w * 3 / 4, w.y + w.h / 2),
                None
            );
            assert_eq!(
                p.touch(f, Touch::Up, w.x + w.w * 3 / 4, w.y + w.h / 2),
                None
            );
            views(p.take_actions()).1
        };
        // The loop plays while held: the first tap starts it, the next lets go.
        assert_eq!(tap(&mut p, 1), vec![FromUi::Pad { pad, down: true }]);
        assert_eq!(tap(&mut p, 2), vec![FromUi::Pad { pad, down: false }]);
        // Leaving the page lets go of a pad a tap holds.
        tap(&mut p, 3);
        p.apply(ToUi::Show(View::Play));
        assert_eq!(p.take_actions(), vec![FromUi::Pad { pad, down: false }]);
        // A one-shot plays whole from one tap.
        p.apply(ToUi::Show(View::Sample));
        for _ in 0..3 {
            let ev = UiEvent {
                widget: mpfree_sampler_ui::wave_tab::REGION.into(),
                kind: EventKind::Step {
                    index: 0,
                    delta: -1,
                },
            };
            p.on_event(ev);
        }
        assert_eq!(
            p.sample.state().unwrap().region.mode,
            mpfree_sampler::model::PlayMode::OneShot
        );
        assert_eq!(
            tap(&mut p, 4),
            vec![
                FromUi::Pad { pad, down: true },
                FromUi::Pad { pad, down: false }
            ]
        );
    }

    #[test]
    fn scenes_morph_the_playing_kit_and_return_puts_the_pads_back() {
        use mpfree_sampler::engine::PadField;
        use mpfree_sampler_ui::play_tab::{Row, FADER, LIST, SCENES};
        let pad = mpfree_sampler_ui::sample::PAD;
        let mut p = sample_sampler(Fonts::embedded());
        let ev = |p: &mut Player, widget: &str, kind: EventKind| {
            p.on_event(UiEvent {
                widget: widget.into(),
                kind,
            });
        };
        ev(
            &mut p,
            mpfree_sampler_ui::wave_tab::BAR,
            EventKind::Tab(mpfree_sampler_ui::play_tab::TAB),
        );
        let level = Row::ALL.iter().position(|r| *r == Row::Level).unwrap();
        let turn_down = |p: &mut Player| {
            p.on_event(UiEvent {
                widget: LIST.into(),
                kind: EventKind::Step {
                    index: level,
                    delta: -1,
                },
            });
            p.apply(ToUi::SampleLift);
        };
        let level_db = |p: &Player| p.sample.project(0).unwrap().pad(pad).unwrap().level_db;
        let a_db = level_db(&p);
        ev(&mut p, SCENES, EventKind::Tab(0));
        assert!(events_log(&p)
            .iter()
            .any(|l| l == "sample: kit 1 (Breaks): scene A stored (1 pad)"));
        turn_down(&mut p);
        let b_db = level_db(&p);
        assert!(b_db < a_db);
        ev(&mut p, SCENES, EventKind::Tab(1));
        // The engine takes the turned-down kit's view (B as it plays).
        let (sent, _) = views(p.take_actions());
        assert_eq!(sent.len(), 1);
        p.apply(ToUi::SampleViewBack(sent.into_iter().next().unwrap()));
        // Half way: the pad's level between A's and B's, for the playing kit.
        ev(&mut p, FADER, EventKind::Value(0.5));
        let (sent, rest) = views(p.take_actions());
        assert!(sent.is_empty(), "the kit doesn't change");
        assert_eq!(
            rest,
            [FromUi::SamplePadLive {
                kit: 0,
                values: vec![(pad as u8, PadField::Level, (a_db + b_db) / 2.0)]
            }]
        );
        // An edit while morphing: its view, then the values again.
        turn_down(&mut p);
        let mangled = level_db(&p);
        let (sent, rest) = views(p.take_actions());
        assert_eq!(sent.len(), 1);
        assert!(matches!(&rest[..], [FromUi::SamplePadLive { kit: 0, .. }]));
        p.apply(ToUi::SampleViewBack(sent.into_iter().next().unwrap()));
        // Return to A: the values let go and the pad is A's again.
        ev(&mut p, SCENES, EventKind::Tab(2));
        let (sent, rest) = views(p.take_actions());
        assert_eq!(sent.len(), 1);
        match &rest[..] {
            [FromUi::SamplePadLive { kit: 0, values }] => {
                assert!(!values.is_empty() && values.iter().all(|v| v.2.is_nan()))
            }
            r => panic!("{r:?}"),
        }
        assert_eq!(level_db(&p), a_db);
        assert!(events_log(&p)
            .iter()
            .any(|l| l == "sample: kit 1 (Breaks): back to scene A (1 pad put back)"));
        p.apply(ToUi::SampleViewBack(sent.into_iter().next().unwrap()));
        // No more values after the next view; UNDO takes the return back.
        p.apply(ToUi::SampleUndo(false));
        assert_eq!(level_db(&p), mangled);
        let (sent, rest) = views(p.take_actions());
        assert_eq!((sent.len(), rest.len()), (1, 0));
    }

    #[test]
    fn the_chop_tab_puts_slices_on_the_kits_pads_and_undo_takes_them_back() {
        use mpfree_sampler_ui::chop_tab::{self, Row};
        use mpfree_sampler_ui::widgets::{ButtonRow, PadBar};
        let pad = mpfree_sampler_ui::sample::PAD;
        let mut p = sample_sampler(Fonts::embedded());
        let tap = |p: &mut Player, widget: &str, i: usize| {
            let r = p.ui.rect_of(widget).unwrap();
            let t = match widget {
                chop_tab::BUTTONS => p.ui.get::<ButtonRow>(widget).unwrap().button_rect(r, i),
                _ => p.ui.get::<PadBar>(widget).unwrap().tab_rect(r, i),
            };
            assert_eq!(p.touch(9, Touch::Down, t.x + 5, t.y + 5), None);
            p.touch(9, Touch::Up, t.x + 5, t.y + 5);
        };
        // The pad bar's Chop tab opens the Chop page on the same pad.
        tap(&mut p, mpfree_sampler_ui::wave_tab::BAR, chop_tab::TAB);
        assert_eq!(
            (p.page(), p.sample.editor()),
            (sample_page::CHOP_PAGE, sample_page::Editor::Chop)
        );
        assert!(p
            .ui
            .get::<Label>("hint")
            .unwrap()
            .text
            .contains("Hit a pad to chop it"));
        assert_eq!(
            p.ui.get::<PadBar>(chop_tab::BAR).unwrap().title,
            "Pad A5 · Break 92"
        );
        // Grid of 4 from the list, then Q-Link 4 to A1: the slices go on A1-A4.
        let step = |p: &mut Player, row: Row, delta: i32| {
            let index = Row::ALL.iter().position(|r| *r == row).unwrap();
            p.on_event(UiEvent {
                widget: chop_tab::LIST.into(),
                kind: EventKind::Step { index, delta },
            });
        };
        step(&mut p, Row::Method, 1);
        step(&mut p, Row::Method, 1);
        for _ in 0..4 {
            step(&mut p, Row::Amount, -1);
        }
        p.apply(ToUi::SampleQLink(3, -16));
        let (tab, st) = p.sample.chop().unwrap();
        assert_eq!((tab.slices(), tab.pads(st)), (4, Some(0..4)));
        let bounds = tab.bounds(st);
        assert_eq!(
            p.ui.get::<ButtonRow>(chop_tab::BUTTONS).unwrap().labels[2],
            "Chop to A1–A4"
        );
        assert!(
            views(p.take_actions()).0.is_empty(),
            "trying chops changes nothing"
        );
        assert!(p
            .ui
            .get::<Label>("hint")
            .unwrap()
            .text
            .contains("tap a slice: hear it"));
        // A tap on the wave plays the slice under it, once.
        p.on_event(UiEvent {
            widget: chop_tab::WAVE.into(),
            kind: EventKind::Frame(bounds[1] + 5),
        });
        assert_eq!(
            p.take_actions(),
            vec![FromUi::Slice {
                kit: 0,
                pad,
                layer: 0,
                start: bounds[1],
                end: bounds[2]
            }]
        );
        tap(&mut p, chop_tab::BUTTONS, 2);
        let kit = p.sample.project(0).unwrap();
        assert_eq!(kit.pad(2).unwrap().name, "Break 92 3");
        assert_eq!(kit.pad(2).unwrap().layers[0].region.start, bounds[2]);
        assert_eq!(
            kit.pad(pad).unwrap().name,
            "Break 92",
            "A5 is left as it was"
        );
        // The kit's new view goes to the audio thread; the Play page names the pads.
        let (sent, _) = views(p.take_actions());
        let view = sent[0].view.as_ref().unwrap();
        assert!((0..4).all(|i| view.has_pad(i)) && view.has_pad(pad) && !view.has_pad(5));
        assert_eq!(p.kits[0].pads[1].as_deref(), Some("Break 92 2"));
        assert_eq!(
            p.ui.get::<MiniPads>(play_page::PADS)
                .map(|m| &m.grid)
                .unwrap()
                .pads[3]
                .label,
            "Break 92 4"
        );
        assert!(
            events_log(&p).iter().any(|l| l
                .starts_with("sample: kit 1 (Breaks) pad A5 Break 92: chopped ")
                && l.ends_with("into 4 slices (Grid) on pads A1 to A4")),
            "{:?}",
            events_log(&p)
        );
        p.apply(ToUi::SampleViewBack(sent.into_iter().next().unwrap()));
        // UNDO takes the four pads away again, and the Play page follows.
        p.apply(ToUi::SampleUndo(false));
        assert!(p.sample.project(0).unwrap().pad(0).is_none());
        assert_eq!(p.kits[0].pads[0], None);
        let (sent, _) = views(p.take_actions());
        assert!(!sent[0].view.as_ref().unwrap().has_pad(0));
        assert_eq!(p.page(), sample_page::CHOP_PAGE);
        // The Wave tab goes back to Wave; another pad stays on the tab open.
        tap(&mut p, chop_tab::BAR, mpfree_sampler_ui::wave_tab::TAB);
        assert_eq!(p.page(), sample_page::PAGE);
        tap(&mut p, mpfree_sampler_ui::wave_tab::BAR, chop_tab::TAB);
        p.apply(ToUi::PadDown {
            pad: 7,
            velocity: 100,
        });
        assert_eq!(p.page(), sample_page::EMPTY_PAGE, "A8 has no sound");
        p.apply(ToUi::PadDown { pad, velocity: 100 });
        assert_eq!(p.page(), sample_page::CHOP_PAGE);
    }

    #[test]
    fn live_chop_drops_cuts_where_the_sampler_marked_them_and_ends_with_the_sound() {
        use mpfree_sampler_ui::chop_tab::{self, Method, Row};
        use mpfree_sampler_ui::widgets::{ButtonRow, PadBar};
        let pad = mpfree_sampler_ui::sample::PAD;
        let mut p = sample_sampler(Fonts::embedded());
        let tap = |p: &mut Player, widget: &str, i: usize| {
            let r = p.ui.rect_of(widget).unwrap();
            let t = match widget {
                chop_tab::BUTTONS => p.ui.get::<ButtonRow>(widget).unwrap().button_rect(r, i),
                _ => p.ui.get::<PadBar>(widget).unwrap().tab_rect(r, i),
            };
            assert_eq!(p.touch(9, Touch::Down, t.x + 5, t.y + 5), None);
            p.touch(9, Touch::Up, t.x + 5, t.y + 5);
        };
        let label =
            |p: &Player| p.ui.get::<ButtonRow>(chop_tab::BUTTONS).unwrap().labels[0].clone();
        tap(&mut p, mpfree_sampler_ui::wave_tab::BAR, chop_tab::TAB);
        // Transient steps back round to Live, and the hint says how it goes.
        let index = Row::ALL.iter().position(|r| *r == Row::Method).unwrap();
        p.on_event(UiEvent {
            widget: chop_tab::LIST.into(),
            kind: EventKind::Step { index, delta: -1 },
        });
        assert_eq!(p.sample.chop().unwrap().0.method(), Method::Live);
        assert!(p.sample.chop_live_method());
        let hint = p.ui.get::<Label>("hint").unwrap().text.clone();
        assert!(hint.contains("hit pads in time to drop cuts"), "{hint}");
        p.take_actions();
        // Start: the pad's region plays, marking.
        assert_eq!(label(&p), "Start");
        tap(&mut p, chop_tab::BUTTONS, 0);
        let (start, end) = {
            let (tab, st) = p.sample.chop().unwrap();
            assert!(tab.live());
            assert_eq!(tab.slices(), 1, "a take starts bare");
            (st.region.start, st.region.end)
        };
        assert_eq!(
            p.take_actions(),
            vec![FromUi::SampleLiveChop {
                kit: 0,
                pad,
                layer: 0,
                start,
                end,
                on: true
            }]
        );
        assert_eq!(label(&p), "Stop");
        // Pads hit mark cuts on the audio thread; here they leave the pad
        // being chopped where it is.
        p.apply(ToUi::PadDown {
            pad: 0,
            velocity: 100,
        });
        p.apply(ToUi::PadUp(0));
        assert_eq!(p.page(), sample_page::CHOP_PAGE);
        assert!(p.sample.chop().unwrap().0.live());
        // Two pads hit: the sampler's marks come back with the playheads
        // and become cuts.
        let len = end - start;
        let mut heads = Playheads {
            kit: 0,
            marking: true,
            marks_made: 2,
            ..Playheads::default()
        };
        heads.marks[0] = (start + len / 4) as u32;
        heads.marks[1] = (start + len * 3 / 4) as u32;
        p.apply(ToUi::SamplePlayheads(heads));
        assert_eq!(p.sample.chop().unwrap().0.slices(), 3);
        // The same marks again add nothing; the sound over ends the take.
        p.apply(ToUi::SamplePlayheads(heads));
        assert_eq!(p.sample.chop().unwrap().0.slices(), 3);
        heads.marking = false;
        p.apply(ToUi::SamplePlayheads(heads));
        assert!(!p.sample.chop().unwrap().0.live());
        assert_eq!(label(&p), "Start");
        assert!(matches!(
            p.take_actions()[..],
            [FromUi::SampleLiveChop { on: false, .. }]
        ));
        assert!(
            events_log(&p)
                .iter()
                .any(|l| l == "sample: live chop over: the sound ended on pad A5: 2 cuts made"),
            "{:?}",
            events_log(&p)
        );
        // A new take, then the Wave tab: it stops, and the cuts it made stay
        // for Chop to.
        tap(&mut p, chop_tab::BUTTONS, 0);
        p.take_actions();
        tap(&mut p, chop_tab::BAR, mpfree_sampler_ui::wave_tab::TAB);
        assert!(matches!(
            p.take_actions()[..],
            [FromUi::SampleLiveChop { on: false, .. }]
        ));
        assert!(events_log(&p)
            .iter()
            .any(|l| l == "sample: live chop stopped on pad A5: 0 cuts made"));
    }

    #[test]
    fn to_keys_puts_slices_on_notes_and_the_play_tab_turns_the_pads_back() {
        use mpfree_sampler::model::{Keys, PadsPlay};
        use mpfree_sampler_ui::chop_tab::{self, Row};
        use mpfree_sampler_ui::play_tab;
        use mpfree_sampler_ui::widgets::{ButtonRow, PadBar};
        let pad = mpfree_sampler_ui::sample::PAD;
        let mut p = sample_sampler(Fonts::embedded());
        let tap = |p: &mut Player, widget: &str, i: usize| {
            let r = p.ui.rect_of(widget).unwrap();
            let t = match widget {
                chop_tab::BUTTONS => p.ui.get::<ButtonRow>(widget).unwrap().button_rect(r, i),
                _ => p.ui.get::<PadBar>(widget).unwrap().tab_rect(r, i),
            };
            assert_eq!(p.touch(9, Touch::Down, t.x + 5, t.y + 5), None);
            p.touch(9, Touch::Up, t.x + 5, t.y + 5);
        };
        tap(&mut p, mpfree_sampler_ui::wave_tab::BAR, chop_tab::TAB);
        // Grid of 4 on A1-A4, as keys from C2.
        let step = |p: &mut Player, row: Row, delta: i32| {
            let index = Row::ALL.iter().position(|r| *r == row).unwrap();
            p.on_event(UiEvent {
                widget: chop_tab::LIST.into(),
                kind: EventKind::Step { index, delta },
            });
        };
        step(&mut p, Row::Method, 1);
        step(&mut p, Row::Method, 1);
        for _ in 0..4 {
            step(&mut p, Row::Amount, -1);
        }
        p.apply(ToUi::SampleQLink(3, -16));
        step(&mut p, Row::Put, 1);
        p.take_actions();
        tap(&mut p, chop_tab::BUTTONS, 2);
        let kit = p.sample.project(0).unwrap();
        assert_eq!(kit.pad(1).unwrap().keys, Some(Keys::one(37)));
        assert_eq!(kit.pads_play(), PadsPlay::Keys { from: 36 });
        assert!(
            events_log(&p).iter().any(|l| l.starts_with("sample: kit 1 (Breaks) pad A5 Break 92: chopped ")
                && l.ends_with("into 4 slices (Grid) on keys C2 to D#2 (pads A1 to A4; the pads now play notes)")),
            "{:?}",
            events_log(&p)
        );
        // The pads play C2 up: A5 is E2, which no keygroup holds.
        let (sent, _) = views(p.take_actions());
        let view = sent[0].view.as_ref().unwrap();
        assert!((0..4).all(|i| view.has_pad(i)) && !view.has_pad(pad));
        p.apply(ToUi::SampleViewBack(sent.into_iter().next().unwrap()));
        // On Play, A2 is picked by its hit; Pads play turns the pads back to their sounds.
        tap(&mut p, chop_tab::BAR, play_tab::TAB);
        p.apply(ToUi::PadDown {
            pad: 1,
            velocity: 100,
        });
        p.apply(ToUi::PadUp(1));
        assert_eq!(p.sample.play().unwrap().1.pad, 1);
        let index = play_tab::Row::ALL
            .iter()
            .position(|r| *r == play_tab::Row::PadsPlay)
            .unwrap();
        p.on_event(UiEvent {
            widget: play_tab::LIST.into(),
            kind: EventKind::Step { index, delta: -1 },
        });
        assert_eq!(p.sample.project(0).unwrap().pads_play(), PadsPlay::Pads);
        assert!(
            events_log(&p)
                .iter()
                .any(|l| l == "sample: kit 1 (Breaks): the pads play their own sounds"),
            "{:?}",
            events_log(&p)
        );
        let (sent, _) = views(p.take_actions());
        assert!(sent[0].view.as_ref().unwrap().has_pad(pad));
        p.apply(ToUi::SampleViewBack(sent.into_iter().next().unwrap()));
        p.apply(ToUi::SampleUndo(false));
        assert_eq!(
            p.sample.project(0).unwrap().pads_play(),
            PadsPlay::Keys { from: 36 }
        );
    }

    #[test]
    fn pads_and_phrase_sends_a_clip_that_lands_in_the_armed_tracks_first_empty_slot() {
        use mpfree_sampler_ui::chop_tab::{self, Row};
        use mpfree_sampler_ui::widgets::{ButtonRow, PadBar};
        let mut p = sample_sampler(Fonts::embedded());
        let tap = |p: &mut Player, widget: &str, i: usize| {
            let r = p.ui.rect_of(widget).unwrap();
            let t = match widget {
                chop_tab::BUTTONS => p.ui.get::<ButtonRow>(widget).unwrap().button_rect(r, i),
                _ => p.ui.get::<PadBar>(widget).unwrap().tab_rect(r, i),
            };
            assert_eq!(p.touch(9, Touch::Down, t.x + 5, t.y + 5), None);
            p.touch(9, Touch::Up, t.x + 5, t.y + 5);
        };
        tap(&mut p, mpfree_sampler_ui::wave_tab::BAR, chop_tab::TAB);
        let step = |p: &mut Player, row: Row, delta: i32| {
            let index = Row::ALL.iter().position(|r| *r == row).unwrap();
            p.on_event(UiEvent {
                widget: chop_tab::LIST.into(),
                kind: EventKind::Step { index, delta },
            });
        };
        // Every eighth of the loop's beat grid, on A1-A15, with a phrase.
        step(&mut p, Row::Method, 1);
        step(&mut p, Row::Amount, -1);
        p.apply(ToUi::SampleQLink(3, -16));
        step(&mut p, Row::Put, -1);
        p.take_actions();
        tap(&mut p, chop_tab::BUTTONS, 2);
        let (sent, rest) = views(p.take_actions());
        assert_eq!(sent.len(), 1, "the pads go to the audio thread as ever");
        let [FromUi::SamplePhrase(clip)] = &rest[..] else {
            panic!("{rest:?}")
        };
        assert_eq!(
            (clip.name.as_str(), clip.length),
            ("Break 92 phrase", 8 * 960)
        );
        assert_eq!(clip.notes.len(), 15);
        assert_eq!((clip.notes[0].tick, clip.notes[0].note), (0, 0));
        assert_eq!(clip.notes[14].note, 14);
        assert!(clip.notes.windows(2).all(|w| w[0].tick < w[1].tick));
        assert!(
            events_log(&p).iter().any(|l| l
                == "sample: phrase \"Break 92 phrase\": 15 hits over 8 beats, for the armed track"),
            "{:?}",
            events_log(&p)
        );
        // The audio loop puts it in the first empty slot; it doesn't launch.
        let mut time = crate::time::TimeHost::new(48_000, 92.0).unwrap();
        assert_eq!(
            time.put_clip((**clip).clone()).unwrap(),
            "phrase: 15 notes in slot 1 of Track 1"
        );
    }

    #[test]
    fn sixteen_levels_make_every_pad_play_the_pad_and_hits_pick_it() {
        use mpfree_sampler::model::{Levels, PadsPlay};
        use mpfree_sampler_ui::play_tab::{self, Row};
        let pad = mpfree_sampler_ui::sample::PAD;
        let mut p = sample_sampler(Fonts::embedded());
        p.on_event(UiEvent {
            widget: mpfree_sampler_ui::wave_tab::BAR.into(),
            kind: EventKind::Tab(play_tab::TAB),
        });
        p.take_actions();
        let step = |p: &mut Player, row: Row, delta: i32| {
            let index = Row::ALL.iter().position(|r| *r == row).unwrap();
            p.on_event(UiEvent {
                widget: play_tab::LIST.into(),
                kind: EventKind::Step { index, delta },
            });
        };
        step(&mut p, Row::PadsPlay, -1);
        step(&mut p, Row::LevelsBy, 1);
        assert_eq!(
            p.sample.project(0).unwrap().pads_play(),
            PadsPlay::Levels {
                pad: pad as u8,
                by: Levels::Tune
            }
        );
        assert!(
            events_log(&p)
                .iter()
                .any(|l| l == "sample: kit 1 (Breaks): the pads play 16 Levels of pad A5 by tune"),
            "{:?}",
            events_log(&p)
        );
        // Every pad sounds, and a hit on any of them shows A5.
        let (sent, _) = views(p.take_actions());
        let view = sent.last().unwrap().view.as_ref().unwrap();
        assert!((0..16).all(|i| view.has_pad(i)));
        p.apply(ToUi::PadDown {
            pad: 0,
            velocity: 100,
        });
        p.apply(ToUi::PadUp(0));
        assert_eq!(p.sample.play().unwrap().1.pad, pad);
        assert_eq!(p.page(), sample_page::PLAY_PAGE);
    }

    #[test]
    fn sixteen_level_turns_levels_on_with_the_panel_and_off_again() {
        use mpfree_sampler::model::PadsPlay;
        let pad = mpfree_sampler_ui::sample::PAD;
        let mut p = sample_sampler(Fonts::embedded());
        p.apply(ToUi::Show(View::Play));
        p.take_actions();
        let told = |acts: &[FromUi]| {
            acts.iter()
                .rev()
                .filter_map(|a| match a {
                    FromUi::Levels { on, open } => Some((*on, *open)),
                    _ => None,
                })
                .next()
        };
        // 16 LEVEL: 16 Levels of the pad hit last, by velocity, with the panel.
        p.apply(ToUi::SixteenLevels);
        let kit = || PadsPlay::Levels {
            pad: pad as u8,
            by: Levels::Velocity,
        };
        assert_eq!(p.sample.project(0).unwrap().pads_play(), kit());
        assert_eq!(p.page(), LEVELS_PAGE);
        let (sent, rest) = views(p.take_actions());
        assert!(
            !sent.is_empty(),
            "the kit's new view goes to the audio thread"
        );
        assert_eq!(told(&rest), Some((true, true)));
        let panel = |p: &Player| {
            p.ui.get::<LevelsPanel>(levels_panel::PANEL)
                .unwrap()
                .model
                .clone()
        };
        assert_eq!(panel(&p).pad_label, "A5");
        assert_eq!(panel(&p).pad_name, "Break 92");
        assert_eq!(panel(&p).colour, crate::akai::kit_colour(0));
        let chips = |p: &Player| -> Vec<String> {
            p.ui.get::<Header>("header")
                .unwrap()
                .chips
                .iter()
                .map(|c| c.text.clone())
                .collect()
        };
        assert!(chips(&p).contains(&"16 Levels · velocity".to_string()));
        // The wheel: Tune.
        p.apply(ToUi::LevelsWheel(1));
        assert_eq!(
            p.sample.project(0).unwrap().pads_play(),
            PadsPlay::Levels {
                pad: pad as u8,
                by: Levels::Tune
            }
        );
        assert_eq!(panel(&p).by, Levels::Tune);
        // A held pad lights its cell.
        p.apply(ToUi::PadDown {
            pad: 3,
            velocity: 90,
        });
        assert_eq!(panel(&p).lit, Some(3));
        p.apply(ToUi::PadUp(3));
        assert_eq!(panel(&p).lit, None);
        // Pick pad, then a hit: the levels play pad 2.
        let r = p.ui.rect_of(levels_panel::PANEL).unwrap();
        let tap = |p: &mut Player, b: mpfree_ui::Rect| {
            let (x, y) = (b.x + b.w / 2, b.y + b.h / 2);
            p.touch(1, Touch::Down, x, y);
            p.touch(1, Touch::Up, x, y);
        };
        tap(&mut p, LevelsPanel::button_rect(r, 0));
        assert!(panel(&p).picking);
        p.apply(ToUi::PadDown {
            pad: 1,
            velocity: 90,
        });
        p.apply(ToUi::PadUp(1));
        assert!(!panel(&p).picking);
        assert_eq!(
            p.sample.project(0).unwrap().pads_play(),
            PadsPlay::Levels {
                pad: 1,
                by: Levels::Tune
            }
        );
        // A tap on Decay.
        tap(&mut p, LevelsPanel::kind_rect(r, Levels::Decay));
        assert_eq!(
            p.sample.project(0).unwrap().pads_play(),
            PadsPlay::Levels {
                pad: 1,
                by: Levels::Decay
            }
        );
        // Close: the panel goes, the levels stay.
        p.take_actions();
        tap(&mut p, LevelsPanel::button_rect(r, 1));
        assert_ne!(p.page(), LEVELS_PAGE);
        assert!(p.sample.levels().is_some());
        assert_eq!(told(&p.take_actions()), Some((true, false)));
        // 16 LEVEL again: the pads play their own sounds.
        p.apply(ToUi::SixteenLevels);
        assert_eq!(p.sample.project(0).unwrap().pads_play(), PadsPlay::Pads);
        assert_eq!(told(&p.take_actions()), Some((false, false)));
        assert!(!chips(&p).iter().any(|c| c.starts_with("16 Levels")));
        // On again keeps the kind picked last; Off on the panel ends it.
        p.apply(ToUi::SixteenLevels);
        assert_eq!(p.page(), LEVELS_PAGE);
        assert_eq!(panel(&p).by, Levels::Decay);
        tap(&mut p, LevelsPanel::button_rect(r, 2));
        assert_eq!(p.sample.project(0).unwrap().pads_play(), PadsPlay::Pads);
        assert_ne!(p.page(), LEVELS_PAGE);
    }

    #[test]
    fn key_fade_is_the_kits_and_vel_fade_the_pads_each_one_undo_step() {
        use mpfree_sampler_ui::play_tab::{self, Row, LIST};
        let pad = mpfree_sampler_ui::sample::PAD;
        let mut p = sample_sampler(Fonts::embedded());
        let ev = |p: &mut Player, widget: &str, kind: EventKind| {
            p.on_event(UiEvent {
                widget: widget.into(),
                kind,
            })
        };
        let index = |row: Row| Row::ALL.iter().position(|r| *r == row).unwrap();
        let step = |p: &mut Player, row: Row, delta: i32| {
            ev(
                p,
                LIST,
                EventKind::Step {
                    index: index(row),
                    delta,
                },
            );
            p.apply(ToUi::SampleLift);
        };
        ev(
            &mut p,
            mpfree_sampler_ui::wave_tab::BAR,
            EventKind::Tab(play_tab::TAB),
        );
        let settings = |p: &Player| p.sample.project(0).unwrap().pad(pad).unwrap().clone();
        // No keygroup yet: Key fade has nothing to fade.
        step(&mut p, Row::KeyFade, 1);
        assert_eq!(settings(&p).key_fade, 0);
        // Keys on, then Key fade: the kit's keygroups fade over 2 notes.
        step(&mut p, Row::Keys, 1);
        let (sent, _) = views(p.take_actions());
        p.apply(ToUi::SampleViewBack(sent.into_iter().next().unwrap()));
        step(&mut p, Row::KeyFade, 2);
        assert_eq!(settings(&p).key_fade, 2);
        assert!(
            events_log(&p).iter().any(|l| l
                == "sample: kit 1 (Breaks): key fade 2 notes, the zones that meet crossfade over them"),
            "{:?}",
            events_log(&p)
        );
        let (sent, _) = views(p.take_actions());
        assert_eq!(sent.len(), 1, "the kit sounds with the fade");
        p.apply(ToUi::SampleViewBack(sent.into_iter().next().unwrap()));
        // Vel fade is the pad's own, with a line of its own.
        step(&mut p, Row::VelFade, 4);
        assert_eq!(settings(&p).vel_fade, 8);
        assert!(
            events_log(&p)
                .iter()
                .any(|l| l == "sample: kit 1 (Breaks) pad A5 Break 92: vel fade Off -> 8"),
            "{:?}",
            events_log(&p)
        );
        // UNDO: the velocity fade, then the key fade.
        p.apply(ToUi::SampleUndo(false));
        assert_eq!(settings(&p).vel_fade, 0);
        assert_eq!(settings(&p).key_fade, 2);
        p.apply(ToUi::SampleUndo(false));
        assert_eq!(settings(&p).key_fade, 0);
        assert!(settings(&p).keys.is_some());
    }

    #[test]
    fn hits_on_pads_edit_stops_and_also_plays_and_latch_is_a_row() {
        use mpfree_sampler_ui::play_tab::{self, Row, LIST};
        use mpfree_sampler_ui::widgets::SettingList;
        let pad = mpfree_sampler_ui::sample::PAD;
        let mut p = sample_sampler(Fonts::embedded());
        let ev = |p: &mut Player, widget: &str, kind: EventKind| {
            p.on_event(UiEvent {
                widget: widget.into(),
                kind,
            })
        };
        let index = |row: Row| Row::ALL.iter().position(|r| *r == row).unwrap();
        ev(
            &mut p,
            mpfree_sampler_ui::wave_tab::BAR,
            EventKind::Tab(play_tab::TAB),
        );
        let settings = |p: &Player| p.sample.project(0).unwrap().pad(pad).unwrap().clone();
        let hint = |p: &Player| p.ui.get::<Label>("hint").unwrap().text.clone();
        // A tap on a row picks it, as the list does.
        let pick = |p: &mut Player, row: Row| {
            p.ui.get_mut::<SettingList>(LIST).unwrap().selected = index(row);
            ev(p, LIST, EventKind::Select(index(row)));
        };
        assert!(
            hint(&p).starts_with("Hit a pad: its velocity"),
            "{}",
            hint(&p)
        );
        // Also plays picked: the hint says pad hits edit it.
        pick(&mut p, Row::Links);
        assert!(hint(&p).starts_with("Hit pads to add them"), "{}", hint(&p));
        for k in [0, 2, pad] {
            p.apply(ToUi::PadDown {
                pad: k,
                velocity: 100,
            });
            p.apply(ToUi::PadUp(k));
        }
        assert_eq!(settings(&p).links, [0, 2], "the pad itself is not added");
        assert_eq!(p.sample.play().unwrap().1.pad, pad, "the pad shown stays");
        assert!(
            events_log(&p)
                .iter()
                .any(|l| l == "sample: kit 1 (Breaks) pad A5 Break 92: also plays A1 -> A1 A3"),
            "{:?}",
            events_log(&p)
        );
        let (sent, _) = views(p.take_actions());
        assert_eq!(sent.len(), 1, "the kit sounds with its links");
        p.apply(ToUi::SampleViewBack(sent.into_iter().next().unwrap()));
        // UNDO takes one hit back.
        p.apply(ToUi::SampleUndo(false));
        assert_eq!(settings(&p).links, [0]);
        // Stops takes pads the same way; Latch is a row.
        pick(&mut p, Row::Chokes);
        p.apply(ToUi::PadDown {
            pad: 7,
            velocity: 100,
        });
        assert_eq!(settings(&p).chokes, [7]);
        pick(&mut p, Row::Latch);
        ev(
            &mut p,
            LIST,
            EventKind::Step {
                index: index(Row::Latch),
                delta: 1,
            },
        );
        assert!(settings(&p).latch);
        assert!(
            hint(&p).starts_with("Hit a pad: its velocity"),
            "{}",
            hint(&p)
        );
        // Another row picked: hits pick the pad shown again.
        p.apply(ToUi::PadDown {
            pad: 0,
            velocity: 100,
        });
        assert_eq!(settings(&p).chokes, [7]);
        assert_eq!(p.page(), sample_page::EMPTY_PAGE, "A1 has no sound yet");
    }

    #[test]
    fn the_play_tab_changes_how_the_pad_plays_and_undo_takes_it_back() {
        use mpfree_sampler_ui::play_tab::{self, Row};
        use mpfree_sampler_ui::play_widgets::LayerMap;
        use mpfree_sampler_ui::widgets::PadBar;
        let pad = mpfree_sampler_ui::sample::PAD;
        let mut p = sample_sampler(Fonts::embedded());
        let tap_tab = |p: &mut Player, bar: &str, i: usize| {
            let r = p.ui.rect_of(bar).unwrap();
            let t = p.ui.get::<PadBar>(bar).unwrap().tab_rect(r, i);
            assert_eq!(p.touch(9, Touch::Down, t.x + 5, t.y + 5), None);
            p.touch(9, Touch::Up, t.x + 5, t.y + 5);
        };
        // The pad bar's Play tab opens the Play page on the same pad.
        tap_tab(&mut p, mpfree_sampler_ui::wave_tab::BAR, play_tab::TAB);
        assert_eq!(
            (p.page(), p.sample.editor()),
            (sample_page::PLAY_PAGE, sample_page::Editor::Play)
        );
        assert!(p
            .ui
            .get::<Label>("hint")
            .unwrap()
            .text
            .contains("Q-Links 1-4: Attack, Decay, Cutoff, Resonance"));
        assert_eq!(
            p.ui.get::<PadBar>(play_tab::BAR).unwrap().title,
            "Pad A5 · Break 92"
        );
        // A hit shows its velocity on the layer map.
        p.apply(ToUi::PadDown { pad, velocity: 77 });
        p.apply(ToUi::PadUp(pad));
        assert_eq!(
            p.ui.get::<LayerMap>(play_tab::LAYERS)
                .unwrap()
                .last_velocity,
            Some(77)
        );
        p.take_actions();
        // Q-Link 2 turns Decay; letting go writes the log line and the view goes out.
        p.apply(ToUi::SampleQLink(1, 8));
        p.apply(ToUi::SampleLift);
        let decay = |p: &Player| p.sample.project(0).unwrap().pad(pad).unwrap().amp.decay_ms;
        assert_eq!(decay(&p), 8.0);
        assert!(
            events_log(&p)
                .iter()
                .any(|l| l == "sample: kit 1 (Breaks) pad A5 Break 92: decay 0 ms -> 8 ms"),
            "{:?}",
            events_log(&p)
        );
        let (sent, _) = views(p.take_actions());
        assert_eq!(sent.len(), 1);
        p.apply(ToUi::SampleViewBack(sent.into_iter().next().unwrap()));
        // The wheel changes the row picked (Decay, by the turn): two detents, one undo step.
        assert_eq!(p.sample.play().unwrap().0.row(&p.ui), Row::Decay);
        p.apply(ToUi::SampleWheel(1));
        p.apply(ToUi::SampleWheel(1));
        assert_eq!(decay(&p), 20.0);
        p.apply(ToUi::SampleUndo(false));
        assert_eq!(decay(&p), 8.0);
        p.apply(ToUi::SampleUndo(false));
        assert_eq!(decay(&p), 0.0);
        assert_eq!(p.page(), sample_page::PLAY_PAGE);
        // A pad without a sound shows the empty page; back on A5, Play again.
        p.apply(ToUi::PadDown {
            pad: 7,
            velocity: 100,
        });
        assert_eq!(p.page(), sample_page::EMPTY_PAGE);
        p.apply(ToUi::PadDown { pad, velocity: 100 });
        assert_eq!(p.page(), sample_page::PLAY_PAGE);
        assert_eq!(
            p.ui.get::<LayerMap>(play_tab::LAYERS)
                .unwrap()
                .last_velocity,
            Some(100)
        );
        tap_tab(&mut p, play_tab::BAR, mpfree_sampler_ui::wave_tab::TAB);
        assert_eq!(p.page(), sample_page::PAGE);
    }

    #[test]
    fn the_rec_tab_records_the_line_input_onto_a_pad_and_frees_the_buffer_when_it_goes_off() {
        use mpfree_sampler_ui::rec_tab;
        use mpfree_sampler_ui::rec_widgets::InputView;
        use mpfree_sampler_ui::sample::feed_input;
        use mpfree_sampler_ui::widgets::{ButtonRow, PadBar};
        let mut p = sample_sampler(Fonts::embedded());
        let ev = |p: &mut Player, widget: &str, kind: EventKind| {
            p.on_event(UiEvent {
                widget: widget.into(),
                kind,
            })
        };
        // Actions for the audio thread: views go back at once; the rest stay.
        let take = |p: &mut Player| {
            let (sent, rest) = views(p.take_actions());
            for v in sent {
                p.apply(ToUi::SampleViewBack(v));
            }
            rest
        };
        let pad_name = |p: &Player, pad: usize| {
            p.sample
                .project(0)
                .unwrap()
                .pad(pad)
                .map(|pd| pd.name.clone())
        };
        // The pad bar's Rec tab: the line input is off, so nothing to record.
        ev(
            &mut p,
            mpfree_sampler_ui::wave_tab::BAR,
            EventKind::Tab(rec_tab::TAB),
        );
        assert_eq!(
            (p.page(), p.sample.editor()),
            (sample_page::REC_PAGE, sample_page::Editor::Rec)
        );
        assert!(p
            .ui
            .get::<Label>("hint")
            .unwrap()
            .text
            .starts_with("Record or Skip back onto the pad named"));
        assert_eq!(
            p.ui.get::<PadBar>(rec_tab::BAR).unwrap().title,
            "Record onto pad A6"
        );
        assert_eq!(
            p.ui.get::<InputView>(rec_tab::VIEW).unwrap().note,
            rec_tab::OFF_NOTE
        );
        assert_eq!(
            p.ui.get::<ButtonRow>(rec_tab::BUTTONS).unwrap().enabled,
            [false, false, true, true],
            "Flatten and Auto-sample don't need the line input"
        );
        assert!(take(&mut p).is_empty(), "no input buffer while it is off");
        // The line input on (the Looper tab's MENU): the buffer is made once
        // and its writer goes to the audio thread.
        let mut s = p.settings.clone();
        s.line_input = true;
        p.apply(ToUi::Settings(s.clone()));
        assert!(events_log(&p)
            .iter()
            .any(|l| l == "sample: the line input feeds the sampler (75 s kept, 26.5 MB)"));
        let mut acts = take(&mut p);
        assert_eq!(acts.len(), 1);
        let Some(FromUi::SampleInput(Some(mut feed))) = acts.pop() else {
            panic!("the input's writer");
        };
        assert!(take(&mut p).is_empty(), "sent once");
        // The audio thread fills it: 12 s of drums with the transport
        // playing, and the tab draws it.
        feed_input(&mut feed.0, 12.0, 0.4, true);
        p.tick(0.033);
        let v = p.ui.get::<InputView>(rec_tab::VIEW).unwrap();
        assert!(
            v.note.is_empty() && v.meter_db[0] > -12.0,
            "{:?}",
            v.meter_db
        );
        assert_eq!(v.skip.as_ref().unwrap().label, "Skip back 10 s");
        // Record, a second and a half, Stop: Take 1 lands on A6, one undo
        // step, and the editor picks it; the Play page's pad names follow.
        ev(&mut p, rec_tab::BUTTONS, EventKind::Tab(0));
        feed_input(&mut feed.0, 1.5, 0.4, true);
        p.tick(0.033);
        assert_eq!(
            p.ui.get::<ButtonRow>(rec_tab::BUTTONS).unwrap().labels[0],
            "Stop"
        );
        ev(&mut p, rec_tab::BUTTONS, EventKind::Tab(0));
        let log = events_log(&p);
        assert!(
            log.iter()
                .any(|l| l == "sample: take armed for pad A6: start now, end stop, left + right"),
            "{log:?}"
        );
        assert!(
            log.iter()
                .any(|l| l.starts_with("sample: recorded: Take 1, 1.5")
                    && l.ends_with(" stereo on pad A6")),
            "{log:?}"
        );
        assert_eq!(pad_name(&p, 5).as_deref(), Some("Take 1"));
        assert_eq!(p.kits[0].pads[5].as_deref(), Some("Take 1"));
        assert_eq!(
            p.ui.get::<PadBar>(rec_tab::BAR).unwrap().title,
            "Record onto pad A7",
            "the next take goes after it"
        );
        // Skip back keeps the last 10 s onto A7.
        ev(&mut p, rec_tab::BUTTONS, EventKind::Tab(1));
        assert!(events_log(&p)
            .iter()
            .any(|l| l == "sample: skip back 10 s: Take 2, 10.00 s stereo on pad A7"));
        let src = {
            let pr = p.sample.project(0).unwrap();
            let id = pr.pad(6).unwrap().layers[0].region.source;
            pr.source(id).unwrap().frames()
        };
        assert_eq!(src, 441_000);
        // UNDO takes it off again.
        p.apply(ToUi::SampleUndo(false));
        assert_eq!(pad_name(&p, 6), None);
        assert_eq!(pad_name(&p, 5).as_deref(), Some("Take 1"));
        // The line input off: the audio thread is asked for the writer, and
        // it is freed here when it comes back.
        s.line_input = false;
        p.apply(ToUi::Settings(s));
        let mut acts = take(&mut p);
        assert!(matches!(acts.pop(), Some(FromUi::SampleInput(None))));
        p.apply(ToUi::SampleInputBack(feed));
        assert!(events_log(&p)
            .iter()
            .any(|l| l.starts_with("sample: the sampler's input buffer was freed (")));
        assert_eq!(
            p.ui.get::<InputView>(rec_tab::VIEW).unwrap().note,
            rec_tab::OFF_NOTE
        );
    }

    #[test]
    fn the_rec_tab_resamples_the_master_and_lets_go_of_it_for_the_line_input() {
        use crate::sampler::RecSource;
        use mpfree_fx::{CapturePoint, StripId};
        use mpfree_sampler_ui::rec_tab::{self, Row};
        use mpfree_sampler_ui::rec_widgets::InputView;
        use mpfree_sampler_ui::sample::feed_input;
        use mpfree_sampler_ui::widgets::PadBar;
        let mut p = sample_sampler(Fonts::embedded());
        let ev = |p: &mut Player, widget: &str, kind: EventKind| {
            p.on_event(UiEvent {
                widget: widget.into(),
                kind,
            })
        };
        let take = |p: &mut Player| {
            let (sent, rest) = views(p.take_actions());
            for v in sent {
                p.apply(ToUi::SampleViewBack(v));
            }
            rest
        };
        ev(
            &mut p,
            mpfree_sampler_ui::wave_tab::BAR,
            EventKind::Tab(rec_tab::TAB),
        );
        assert!(take(&mut p).is_empty(), "the line input is off: no buffer");
        // Source: Master. The audio thread is told, then gets the buffer's
        // writer (the line input stays off).
        let source = Row::ALL.iter().position(|r| *r == Row::Source).unwrap();
        ev(
            &mut p,
            rec_tab::LIST,
            EventKind::Step {
                index: source,
                delta: 1,
            },
        );
        let master = RecSource::Mix(StripId::Master, CapturePoint::AfterSafety);
        let acts = take(&mut p);
        assert!(matches!(acts.as_slice(), [FromUi::SampleSource(s)] if *s == master));
        let mut acts = take(&mut p);
        let Some(FromUi::SampleInput(Some(mut feed))) = acts.pop() else {
            panic!("the input's writer");
        };
        assert!(take(&mut p).is_empty(), "each sent once");
        let log = events_log(&p);
        assert!(
            log.iter()
                .any(|l| l == "sample: the Rec tab resamples Master")
                && log
                    .iter()
                    .any(|l| l == "sample: the sampler resamples Master (75 s kept, 26.5 MB)"),
            "{log:?}"
        );
        // The audio thread fills it from the master; Skip back keeps it.
        feed_input(&mut feed.0, 12.0, 0.4, true);
        p.tick(0.033);
        let v = p.ui.get::<InputView>(rec_tab::VIEW).unwrap();
        assert_eq!(v.title, "Master");
        assert!(v.note.is_empty(), "{}", v.note);
        assert_eq!(
            p.ui.get::<PadBar>(rec_tab::BAR).unwrap().subtitle,
            "Left + right · resampling Master"
        );
        ev(&mut p, rec_tab::BUTTONS, EventKind::Tab(1));
        assert!(
            events_log(&p)
                .iter()
                .any(|l| l == "sample: skip back 10 s: Master take 1, 10.00 s stereo on pad A6"),
            "{:?}",
            events_log(&p)
        );
        // Back to the line input, which is off: the source goes back and
        // the writer is asked for, to be freed here.
        ev(
            &mut p,
            rec_tab::LIST,
            EventKind::Step {
                index: source,
                delta: -1,
            },
        );
        let acts = take(&mut p);
        assert!(matches!(
            acts.as_slice(),
            [FromUi::SampleSource(RecSource::Line)]
        ));
        let mut acts = take(&mut p);
        assert!(matches!(acts.pop(), Some(FromUi::SampleInput(None))));
        p.apply(ToUi::SampleInputBack(feed));
        assert!(events_log(&p)
            .iter()
            .any(|l| l.starts_with("sample: the sampler's input buffer was freed (")));
        assert_eq!(
            p.ui.get::<InputView>(rec_tab::VIEW).unwrap().note,
            rec_tab::OFF_NOTE
        );
    }

    #[test]
    fn bars_printed_to_a_pad_loop_and_land_warped_at_the_project_tempo() {
        use mpfree_sampler_ui::rec_tab::{self, Row, Skip, TailAt};
        use mpfree_sampler_ui::sample::feed_input;
        let mut p = sample_sampler(Fonts::embedded());
        let ev = |p: &mut Player, widget: &str, kind: EventKind| {
            p.on_event(UiEvent {
                widget: widget.into(),
                kind,
            })
        };
        let take = |p: &mut Player| {
            let (sent, rest) = views(p.take_actions());
            for v in sent {
                p.apply(ToUi::SampleViewBack(v));
            }
            rest
        };
        let step = |p: &mut Player, row: Row, delta: i32| {
            let index = Row::ALL.iter().position(|r| *r == row).unwrap();
            p.on_event(UiEvent {
                widget: rec_tab::LIST.into(),
                kind: EventKind::Step { index, delta },
            })
        };
        ev(
            &mut p,
            mpfree_sampler_ui::wave_tab::BAR,
            EventKind::Tab(rec_tab::TAB),
        );
        // Resample the master: one bar from the next bar line, as a loop.
        step(&mut p, Row::Source, 1);
        take(&mut p);
        let mut acts = take(&mut p);
        let Some(FromUi::SampleInput(Some(mut feed))) = acts.pop() else {
            panic!("the input's writer");
        };
        step(&mut p, Row::Start, 2);
        step(&mut p, Row::End, 2);
        step(&mut p, Row::Length, -2);
        step(&mut p, Row::Tail, 2);
        let tab = p.sample.rec_settings().unwrap();
        assert_eq!((tab.bars, tab.tail), (1, TailAt::Loop));
        feed_input(&mut feed.0, 2.0, 0.4, true);
        p.tick(0.033);
        ev(&mut p, rec_tab::BUTTONS, EventKind::Tab(0));
        feed_input(&mut feed.0, 6.0, 0.4, true);
        p.tick(0.033);
        let log = events_log(&p);
        assert!(
            log.iter().any(|l| l
                == "sample: take armed for pad A6: start next bar, end 1 bar then loop, left + right"),
            "{log:?}"
        );
        // The drums were already playing: their ringing is in its start,
        // so the bar is cut as it was, and it follows the project tempo.
        let tempo = p.sample.tempo();
        let line = format!(
            "sample: recorded as a loop, cut: it was going round: Master take 1, 2.61 s stereo on \
             pad A6, warped at {tempo:.1} BPM"
        );
        assert!(log.contains(&line), "{line}\n{log:?}");
        let region = |p: &Player, pad: usize| {
            p.sample.project(0).unwrap().pad(pad).unwrap().layers[0]
                .region
                .clone()
        };
        let r = region(&p, 5);
        assert_eq!(r.warp.map(|w| w.bpm), Some(tempo as f32));
        // Skip back by bars lands warped too; by seconds it doesn't.
        step(&mut p, Row::SkipBack, 4);
        assert_eq!(p.sample.rec_settings().unwrap().skip, Skip::Bars(1));
        ev(&mut p, rec_tab::BUTTONS, EventKind::Tab(1));
        assert_eq!(region(&p, 6).warp.map(|w| w.bpm), Some(tempo as f32));
        step(&mut p, Row::SkipBack, -4);
        ev(&mut p, rec_tab::BUTTONS, EventKind::Tab(1));
        assert_eq!(region(&p, 7).warp, None);
    }

    #[test]
    fn flatten_records_the_pads_own_channel_as_it_plays_once_and_keeps_it_on_the_next_pad() {
        use crate::sampler::{RecSource, CENTRE_DB};
        use mpfree_fx::{CapturePoint, StripId};
        use mpfree_sampler_ui::rec_tab;
        use mpfree_sampler_ui::sample::{feed_input, PAD};
        use mpfree_sampler_ui::widgets::{ButtonRow, PadBar};
        let mut p = sample_sampler(Fonts::embedded());
        let ev = |p: &mut Player, widget: &str, kind: EventKind| {
            p.on_event(UiEvent {
                widget: widget.into(),
                kind,
            })
        };
        let take = |p: &mut Player| {
            let (sent, rest) = views(p.take_actions());
            for v in sent {
                p.apply(ToUi::SampleViewBack(v));
            }
            rest
        };
        ev(
            &mut p,
            mpfree_sampler_ui::wave_tab::BAR,
            EventKind::Tab(rec_tab::TAB),
        );
        assert!(take(&mut p).is_empty());
        let b = p.ui.get::<ButtonRow>(rec_tab::BUTTONS).unwrap();
        assert_eq!(b.labels[2], "Flatten");
        assert_eq!(
            b.enabled,
            [false, false, true, true],
            "Flatten needs no line input"
        );
        // Flatten: the input turns to A5's own channel, after its inserts.
        ev(&mut p, rec_tab::BUTTONS, EventKind::Tab(2));
        let channel = RecSource::Mix(StripId::Pad(PAD), CapturePoint::BeforeFader);
        let acts = take(&mut p);
        assert!(
            matches!(acts.as_slice(), [FromUi::SampleSource(s)] if *s == channel),
            "{acts:?}"
        );
        let mut acts = take(&mut p);
        let Some(FromUi::SampleInput(Some(mut feed))) = acts.pop() else {
            panic!("the input's writer");
        };
        assert_eq!(
            p.ui.get::<PadBar>(rec_tab::BAR).unwrap().title,
            "Flattening A5 onto A6"
        );
        assert!(events_log(&p).iter().any(|l| l
            == "sample: flatten A5 onto A6: velocity 127, held 1000 ms, from its channel after its inserts"));
        // Nothing has come from the channel yet: no hit.
        p.tick(0.033);
        assert!(take(&mut p).is_empty());
        // It has: the pad is hit at 127, straight on the sampler.
        feed_input(&mut feed.0, 0.1, 0.0, false);
        p.tick(0.033);
        let hit = |down| FromUi::SampleHit {
            kit: 0,
            pad: PAD,
            velocity: 127,
            down,
        };
        assert_eq!(take(&mut p), [hit(true)]);
        // Record, Skip back and Auto-sample wait; the first button stops it.
        let b = p.ui.get::<ButtonRow>(rec_tab::BUTTONS).unwrap();
        assert_eq!(
            (b.labels[0].as_str(), b.enabled.clone()),
            ("Stop", vec![true, false, false, false])
        );
        // The pad sounds; after the hold it is let go.
        feed_input(&mut feed.0, 0.6, 0.5, false);
        p.tick(0.6);
        assert!(take(&mut p).is_empty());
        feed_input(&mut feed.0, 0.5, 0.5, false);
        p.tick(0.5);
        assert_eq!(take(&mut p), [hit(false)]);
        // A second of silence ends it: kept on A6 as a pad of its own, and
        // the Rec tab's own source (the line input, off) comes back.
        feed_input(&mut feed.0, 1.2, 0.0, false);
        p.tick(0.033);
        let log = events_log(&p);
        let kept = log
            .iter()
            .find(|l| l.starts_with("sample: flatten A5 kept: Break 92 flat, "))
            .unwrap_or_else(|| panic!("{log:?}"));
        assert!(kept.ends_with(" on pad A6"), "{kept}");
        let pad = p.sample.project(0).unwrap().pad(PAD + 1).unwrap();
        assert_eq!(pad.name, "Break 92 flat");
        assert_eq!(pad.level_db, CENTRE_DB);
        let frames = pad.layers[0].region.end - pad.layers[0].region.start;
        let rate = u64::from(mpfree_sampler_ui::sample::RATE);
        // From the first loud frame to the last of the 1.1 s it sounded
        // (the drum loop has quiet gaps at both ends).
        assert!((rate / 2..=rate * 11 / 10).contains(&frames), "{frames}");
        let acts = take(&mut p);
        assert!(
            matches!(acts.as_slice(), [FromUi::SampleSource(RecSource::Line)]),
            "{acts:?}"
        );
        let mut acts = take(&mut p);
        assert!(matches!(acts.pop(), Some(FromUi::SampleInput(None))));
        p.apply(ToUi::SampleInputBack(feed));
        assert_eq!(
            p.ui.get::<PadBar>(rec_tab::BAR).unwrap().title,
            "Record onto pad A7",
            "the pad picked is the new one"
        );
    }

    #[test]
    fn flatten_stops_cleanly_and_gives_up_on_a_pad_that_makes_no_sound() {
        use crate::sampler::RecSource;
        use mpfree_sampler_ui::rec_tab;
        use mpfree_sampler_ui::sample::{feed_input, PAD};
        let mut p = sample_sampler(Fonts::embedded());
        let ev = |p: &mut Player, widget: &str, kind: EventKind| {
            p.on_event(UiEvent {
                widget: widget.into(),
                kind,
            })
        };
        let take = |p: &mut Player| {
            let (sent, rest) = views(p.take_actions());
            for v in sent {
                p.apply(ToUi::SampleViewBack(v));
            }
            rest
        };
        ev(
            &mut p,
            mpfree_sampler_ui::wave_tab::BAR,
            EventKind::Tab(rec_tab::TAB),
        );
        ev(&mut p, rec_tab::BUTTONS, EventKind::Tab(2));
        take(&mut p);
        let mut acts = take(&mut p);
        let Some(FromUi::SampleInput(Some(mut feed))) = acts.pop() else {
            panic!("the input's writer");
        };
        feed_input(&mut feed.0, 0.1, 0.0, false);
        p.tick(0.033);
        assert_eq!(take(&mut p).len(), 1, "the hit");
        // Stop while it sounds: the pad is let go, nothing is kept.
        feed_input(&mut feed.0, 0.3, 0.5, false);
        p.tick(0.3);
        ev(&mut p, rec_tab::BUTTONS, EventKind::Tab(0));
        let acts = take(&mut p);
        assert!(
            matches!(
                acts.as_slice(),
                [FromUi::SampleSource(RecSource::Line), FromUi::SampleHit { pad, down: false, .. }]
                    if *pad == PAD
            ),
            "{acts:?}"
        );
        assert!(events_log(&p)
            .iter()
            .any(|l| l == "sample: flatten A5 stopped: nothing kept"));
        assert!(p.sample.project(0).unwrap().pad(PAD + 1).is_none());
        let mut acts = take(&mut p);
        assert!(matches!(acts.pop(), Some(FromUi::SampleInput(None))));
        p.apply(ToUi::SampleInputBack(feed));
        // Again, and the pad makes no sound: it gives up after the hold
        // and two seconds more.
        ev(&mut p, rec_tab::BUTTONS, EventKind::Tab(2));
        take(&mut p);
        let mut acts = take(&mut p);
        let Some(FromUi::SampleInput(Some(mut feed))) = acts.pop() else {
            panic!("the input's writer");
        };
        feed_input(&mut feed.0, 0.1, 0.0, false);
        p.tick(0.033);
        take(&mut p);
        for _ in 0..4 {
            feed_input(&mut feed.0, 1.0, 0.0, false);
            p.tick(1.0);
        }
        assert!(
            events_log(&p)
                .iter()
                .any(|l| l == "sample: flatten A5 stopped: the pad made no sound"),
            "{:?}",
            events_log(&p)
        );
        let acts = take(&mut p);
        assert!(acts
            .iter()
            .any(|a| matches!(a, FromUi::SampleSource(RecSource::Line))));
    }

    #[test]
    fn the_grain_tab_sets_the_grains_and_draws_the_cloud_the_audio_thread_fills() {
        use mpfree_sampler::model::PlayMode;
        use mpfree_sampler_ui::grain_tab::{self, Row};
        use mpfree_sampler_ui::grain_widgets::GrainCloud;
        use mpfree_sampler_ui::widgets::PadBar;
        let pad = mpfree_sampler_ui::sample::PAD;
        let mut p = sample_sampler(Fonts::embedded());
        let ev = |p: &mut Player, widget: &str, kind: EventKind| {
            p.on_event(UiEvent {
                widget: widget.into(),
                kind,
            })
        };
        // Takes the actions: the kit views (sent back at once) and the cloud.
        let take = |p: &mut Player| {
            let (sent, rest) = views(p.take_actions());
            for v in sent {
                p.apply(ToUi::SampleViewBack(v));
            }
            let mut clouds = Vec::new();
            for a in rest {
                if let FromUi::SampleGrains(c) = a {
                    clouds.push(c);
                }
            }
            clouds
        };
        assert!(take(&mut p).is_empty(), "no cloud off the Grain tab");
        // The pad bar's Grain tab opens the Grain page on the same pad.
        ev(
            &mut p,
            mpfree_sampler_ui::wave_tab::BAR,
            EventKind::Tab(grain_tab::TAB),
        );
        assert_eq!(
            (p.page(), p.sample.editor()),
            (sample_page::GRAIN_PAGE, sample_page::Editor::Grain)
        );
        assert!(p
            .ui
            .get::<Label>("hint")
            .unwrap()
            .text
            .contains("Q-Links 1-4: position, size, density, pitch"));
        assert_eq!(
            p.ui.get::<PadBar>(grain_tab::BAR).unwrap().title,
            "Pad A5 · Break 92"
        );
        // Mode, stepped from the list, makes the loop play grains: one log line.
        let mode = Row::ALL.iter().position(|r| *r == Row::Mode).unwrap();
        ev(
            &mut p,
            grain_tab::LIST,
            EventKind::Step {
                index: mode,
                delta: 1,
            },
        );
        p.apply(ToUi::SampleLift);
        let region = |p: &Player| {
            p.sample.project(0).unwrap().pad(pad).unwrap().layers[0]
                .region
                .clone()
        };
        assert_eq!(region(&p).mode, PlayMode::Granular);
        assert!(
            events_log(&p)
                .iter()
                .any(|l| l == "sample: kit 1 (Breaks) pad A5 Break 92: mode Loop -> Granular"),
            "{:?}",
            events_log(&p)
        );
        // The cloud's box goes to the audio thread, naming the pad; one at a time.
        let mut clouds = take(&mut p);
        assert_eq!(clouds.len(), 1);
        let mut c = clouds.pop().unwrap();
        assert_eq!((c.kit, c.pad, c.layer), (0, pad as u8, 0));
        assert!(take(&mut p).is_empty(), "the box is away");
        // Filled there, it comes back and the cloud draws it.
        let r = region(&p);
        c.len = 3;
        for (k, g) in c.grains[..3].iter_mut().enumerate() {
            g.position = 0.2 * (k + 1) as f32;
            g.level = 0.5;
        }
        c.head = Some(r.start + r.frames() / 2);
        c.live = 7;
        p.apply(ToUi::SampleGrains(c));
        let cloud = p.ui.get::<GrainCloud>(grain_tab::CLOUD).unwrap();
        assert_eq!(cloud.grains.len(), 3);
        assert!((cloud.playhead.unwrap() - 0.5).abs() < 0.01);
        assert_eq!(cloud.right, "7 grains on all pads");
        assert!(
            cloud.caption.contains("3 grains playing"),
            "{}",
            cloud.caption
        );
        // Back again for the next fill; a full channel keeps it here.
        let mut clouds = take(&mut p);
        assert_eq!(clouds.len(), 1);
        p.unsent(FromUi::SampleGrains(clouds.pop().unwrap()));
        assert_eq!(take(&mut p).len(), 1);
        // A cloud for another pad isn't drawn (the pad changed on the way).
        let mut other = Box::<crate::sampler::Cloud>::default();
        other.pad = 2;
        other.len = 1;
        p.apply(ToUi::SampleGrains(other));
        assert_eq!(
            p.ui.get::<GrainCloud>(grain_tab::CLOUD)
                .unwrap()
                .grains
                .len(),
            3
        );
        // Q-Link 1 turns the position; letting go writes the line; undo takes it back.
        p.apply(ToUi::SampleQLink(0, 4));
        p.apply(ToUi::SampleLift);
        assert!((region(&p).grain.position - 0.02).abs() < 1e-6);
        let (tab, st) = p.sample.grain().unwrap();
        assert_eq!(tab.row(&p.ui), Row::Position, "Q-Link 1 picks its row");
        assert_eq!(st.region, region(&p));
        assert!(
            events_log(&p)
                .iter()
                .any(|l| l == "sample: kit 1 (Breaks) pad A5 Break 92: position 0.0 % -> 2.0 %"),
            "{:?}",
            events_log(&p)
        );
        // SHIFT + Q-Link 1 turns the spray instead.
        p.status.shift = true;
        p.apply(ToUi::SampleQLink(0, 3));
        p.apply(ToUi::SampleLift);
        p.status.shift = false;
        assert!((region(&p).grain.spray - 0.03).abs() < 1e-6);
        p.apply(ToUi::SampleUndo(false));
        p.apply(ToUi::SampleUndo(false));
        assert_eq!(region(&p).grain.position, 0.0);
        p.apply(ToUi::SampleUndo(false));
        assert_eq!(region(&p).mode, PlayMode::Loop);
        // Still on Grain: the tab says how to make the loop play grains.
        assert_eq!(p.page(), sample_page::GRAIN_PAGE);
        assert!(!p.ui.get::<GrainCloud>(grain_tab::CLOUD).unwrap().granular);
        // Off the tab, the box stays on the screen thread.
        ev(
            &mut p,
            grain_tab::BAR,
            EventKind::Tab(mpfree_sampler_ui::wave_tab::TAB),
        );
        assert_eq!(p.page(), sample_page::PAGE);
        assert!(take(&mut p).is_empty());
    }

    #[test]
    fn the_fx_tab_asks_for_the_chain_editor_on_the_pads_channel() {
        use mpfree_sampler_ui::wave_tab::{BAR, FX_TAB};
        use mpfree_sampler_ui::widgets::PadBar;
        let pad = mpfree_sampler_ui::sample::PAD;
        let mut p = sample_sampler(Fonts::embedded());
        p.take_actions();
        assert!(
            p.ui.get::<PadBar>(BAR).unwrap().enabled[FX_TAB],
            "FX is built"
        );
        let (page, editor) = (p.page().to_string(), p.sample.editor());
        p.on_event(UiEvent {
            widget: BAR.into(),
            kind: EventKind::Tab(FX_TAB),
        });
        let (_, rest) = views(p.take_actions());
        assert_eq!(rest, [FromUi::SampleFx { pad }]);
        // The sampler's own page stays under the editor, for Back.
        assert_eq!((p.page(), p.sample.editor()), (page.as_str(), editor));
        assert!(events_log(&p)
            .iter()
            .any(|l| l == "sample: pad A5's effects: its own channel, then the Drums track"));
    }

    #[test]
    fn auto_sample_plays_external_gear_over_midi_and_records_it_from_the_line_input() {
        use mpfree_sampler_ui::auto_tab::{self, Row};
        use mpfree_sampler_ui::rec_tab;
        use mpfree_sampler_ui::widgets::{ButtonRow, PadBar, SettingList};
        use std::sync::{Arc, Mutex};
        let mut p = sample_sampler(Fonts::embedded());
        let ev = |p: &mut Player, widget: &str, kind: EventKind| {
            p.on_event(UiEvent {
                widget: widget.into(),
                kind,
            })
        };
        let step = |p: &mut Player, row: Row, delta: i32| {
            ev(
                p,
                auto_tab::LIST,
                EventKind::Step {
                    index: row.index(),
                    delta,
                },
            );
        };
        let source = |p: &Player| {
            p.ui.get::<SettingList>(auto_tab::LIST).unwrap().rows[Row::Source.index()]
                .value
                .clone()
        };
        let title = |p: &Player| p.ui.get::<PadBar>(auto_tab::BAR).unwrap().title.clone();
        ev(
            &mut p,
            mpfree_sampler_ui::wave_tab::BAR,
            EventKind::Tab(rec_tab::TAB),
        );
        ev(&mut p, rec_tab::BUTTONS, EventKind::Tab(3));
        // No MIDI out yet: the Source row keeps the synth.
        step(&mut p, Row::Source, 1);
        assert_eq!(source(&p), "Poly Pluck");
        // The app opens a MIDI out for gear: the gear on channel 2, an
        // octave apart, one layer, short notes onto A1-A4.
        let sent = Arc::new(Mutex::new(Vec::<Vec<u8>>::new()));
        let to = sent.clone();
        p.sample.set_gear(
            &mut p.ui,
            Some(sample_page::GearOut {
                name: "MIDI Out (DIN)".into(),
                send: Arc::new(move |m: &[u8]| to.lock().unwrap().push(m.to_vec())),
            }),
        );
        step(&mut p, Row::Source, 2);
        assert_eq!(source(&p), "Gear · MIDI ch 2");
        step(&mut p, Row::Every, 9);
        step(&mut p, Row::Layers, -2);
        step(&mut p, Row::Hold, -3);
        step(&mut p, Row::Tail, -2);
        assert_eq!(title(&p), "Auto-sample the gear onto pads A1–A4");
        // The line input is off: Start says so and sends nothing.
        ev(&mut p, auto_tab::BUTTONS, EventKind::Tab(0));
        assert!(!p.sample.auto_running());
        assert!(events_log(&p)
            .iter()
            .any(|l| l == "sample: auto-sample of the gear not started (the line input is off)"));
        assert!(sent.lock().unwrap().is_empty());
        // On: its writer goes to the audio thread (here, this test).
        let mut s = p.settings.clone();
        s.line_input = true;
        p.apply(ToUi::Settings(s));
        let (views_sent, mut acts) = views(p.take_actions());
        for v in views_sent {
            p.apply(ToUi::SampleViewBack(v));
        }
        let Some(FromUi::SampleInput(Some(mut feed))) = acts.pop() else {
            panic!("the input's writer");
        };
        assert!(p.ui.get::<ButtonRow>(auto_tab::BUTTONS).unwrap().enabled[0]);
        ev(&mut p, auto_tab::BUTTONS, EventKind::Tab(0));
        assert!(p.sample.auto_running());
        let log = events_log(&p);
        assert!(
            log.iter().any(|l| l
                == "sample: auto-sample Gear (MIDI channel 2 on MIDI Out (DIN)) started: 4 notes (C2 to C5), 4 \
                    takes, 4 s of notes, up to 2 MB"),
            "{log:?}"
        );
        // The gear: a sine for each note held, falling away when let go,
        // heard on the line input. The audio thread writes 256 frames at a
        // time, about twice as fast as real time.
        let rate = 48_000.0f32;
        let (mut held, mut env, mut phase) = (None::<u8>, 0.0f32, 0.0f32);
        let mut midi = Vec::new();
        let t = Instant::now();
        while p.sample.auto_running() {
            assert!(t.elapsed() < Duration::from_secs(60), "it finishes");
            for m in sent.lock().unwrap().drain(..) {
                assert_eq!(m.len(), 3);
                assert_eq!(m[0] & 0x0f, 1, "channel 2");
                match m[0] & 0xf0 {
                    0x90 => held = Some(m[1]),
                    0x80 if held == Some(m[1]) => held = None,
                    _ => {}
                }
                midi.push(m);
            }
            let hz = held.map(|n| 440.0 * ((f32::from(n) - 69.0) / 12.0).exp2());
            let mut l = [0.0f32; 256];
            for x in l.iter_mut() {
                env = match hz {
                    Some(_) => (env + 0.01).min(1.0),
                    None => env * 0.999,
                };
                phase = (phase + hz.unwrap_or(0.0) / rate).fract();
                *x = 0.3 * env * (std::f32::consts::TAU * phase).sin();
            }
            feed.0.audio.push_planar(&l, &l);
            std::thread::sleep(Duration::from_millis(2));
            p.tick(0.033);
        }
        // What the job sent after the last look, before it ended: once it
        // has landed, everything it sent is in.
        midi.extend(sent.lock().unwrap().drain(..));
        assert!(midi.iter().all(|m| m[0] & 0x0f == 1), "channel 2");
        // Two level tests and four notes, each on and off, then All Notes Off.
        let ons = midi.iter().filter(|m| m[0] == 0x91).count();
        let offs = midi.iter().filter(|m| m[0] == 0x81).count();
        assert_eq!((ons, offs), (6, 6));
        assert_eq!(midi.last().map(|m| m.as_slice()), Some(&[0xb1, 123, 0][..]));
        let log = events_log(&p);
        assert!(
            log.iter().any(|l| l.starts_with(
                "sample: auto-sampled Gear onto kit 1 (Breaks) pads A1 to A4: 4 notes, 4 takes, 0 looped, "
            ) && l.contains("s of notes recorded")),
            "{log:?}"
        );
        let pad_name = |pad: usize| {
            p.sample
                .project(0)
                .unwrap()
                .pad(pad)
                .map(|pd| pd.name.clone())
        };
        assert_eq!(pad_name(0).as_deref(), Some("Gear C2"));
        assert_eq!(pad_name(3).as_deref(), Some("Gear C5"));
        // Again, onto the next empty pads, then Cancel: the job lets go of
        // the gear (its note off, then All Notes Off) and ends.
        ev(&mut p, auto_tab::BUTTONS, EventKind::Tab(0));
        assert!(p.sample.auto_running());
        let t = Instant::now();
        while sent.lock().unwrap().is_empty() {
            assert!(
                t.elapsed() < Duration::from_secs(10),
                "the first note goes out"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        ev(&mut p, auto_tab::BUTTONS, EventKind::Tab(0));
        while p.sample.auto_running() {
            assert!(t.elapsed() < Duration::from_secs(10), "it stops");
            std::thread::sleep(Duration::from_millis(5));
            p.tick(0.033);
        }
        let sent = sent.lock().unwrap();
        assert_eq!(
            sent.iter().map(|m| m.as_slice()).collect::<Vec<_>>(),
            [&[0x91, 60, 127][..], &[0x81, 60, 64], &[0xb1, 123, 0]],
            "the first test note, let go"
        );
        assert!(events_log(&p)
            .iter()
            .any(|l| l == "sample: auto-sample cancelled, keeping the notes done"));
    }

    #[test]
    fn auto_sample_plays_a_copy_of_the_synth_onto_empty_pads_and_undo_takes_it_off() {
        use mpfree_sampler::model::PadsPlay;
        use mpfree_sampler_ui::auto_tab::{self, Row};
        use mpfree_sampler_ui::auto_widgets::KeyMapView;
        use mpfree_sampler_ui::rec_tab;
        use mpfree_sampler_ui::widgets::{ButtonRow, PadBar};
        let mut p = sample_sampler(Fonts::embedded());
        let ev = |p: &mut Player, widget: &str, kind: EventKind| {
            p.on_event(UiEvent {
                widget: widget.into(),
                kind,
            })
        };
        let step = |p: &mut Player, row: Row, delta: i32| {
            ev(
                p,
                auto_tab::LIST,
                EventKind::Step {
                    index: row.index(),
                    delta,
                },
            );
        };
        let title = |p: &Player| p.ui.get::<PadBar>(auto_tab::BAR).unwrap().title.clone();
        let pad_name = |p: &Player, pad: usize| {
            p.sample
                .project(0)
                .unwrap()
                .pad(pad)
                .map(|pd| pd.name.clone())
        };
        // The Rec tab's fourth button opens the mode, with the Synth tab's
        // patch as its source.
        ev(
            &mut p,
            mpfree_sampler_ui::wave_tab::BAR,
            EventKind::Tab(rec_tab::TAB),
        );
        ev(&mut p, rec_tab::BUTTONS, EventKind::Tab(3));
        assert_eq!(
            (p.page(), p.sample.editor()),
            (sample_page::AUTO_PAGE, sample_page::Editor::Auto)
        );
        assert!(p
            .ui
            .get::<Label>("hint")
            .unwrap()
            .text
            .starts_with("Start samples the Synth tab's patch"));
        // 16 notes need the whole kit: the break on A5 is in the way.
        assert_eq!(title(&p), "No 16 empty pads in a row on this kit");
        assert!(!p.ui.get::<ButtonRow>(auto_tab::BUTTONS).unwrap().enabled[0]);
        // An octave apart, one layer, short notes: 4 takes onto A1-A4.
        step(&mut p, Row::Every, 9);
        step(&mut p, Row::Layers, -2);
        step(&mut p, Row::Hold, -3);
        step(&mut p, Row::Tail, -2);
        assert_eq!(title(&p), "Auto-sample Poly Pluck onto pads A1–A4");
        let v = p.ui.get::<KeyMapView>(auto_tab::VIEW).unwrap();
        assert_eq!((v.zones.len(), v.layers.len()), (4, 1));
        assert!(p.ui.get::<ButtonRow>(auto_tab::BUTTONS).unwrap().enabled[0]);
        // Start: a copy of the patch plays on a worker thread; the audio
        // thread is asked for nothing.
        ev(&mut p, auto_tab::BUTTONS, EventKind::Tab(0));
        assert!(p.sample.auto_running());
        let log = events_log(&p);
        assert!(
            log.iter().any(|l| l
                == "sample: auto-sample Poly Pluck started: 4 notes (C2 to C5), 4 takes, 4 s of notes, up to 2 MB"),
            "{log:?}"
        );
        let (sent, rest) = views(p.take_actions());
        assert!(sent.is_empty() && rest.is_empty(), "{rest:?}");
        let t = Instant::now();
        while p.sample.auto_running() {
            assert!(t.elapsed() < Duration::from_secs(60), "it finishes");
            std::thread::sleep(Duration::from_millis(5));
            p.tick(0.033);
        }
        let log = events_log(&p);
        assert!(
            log.iter().any(|l| l.starts_with(
                "sample: auto-sampled Poly Pluck onto kit 1 (Breaks) pads A1 to A4: 4 notes, 4 takes, 0 looped, "
            ) && l.ends_with("; the pads play notes")),
            "{log:?}"
        );
        // The instrument is on the kit, one keygroup a note, and the Play
        // page's pad names follow.
        assert_eq!(pad_name(&p, 0).as_deref(), Some("Poly Pluck C2"));
        assert_eq!(pad_name(&p, 3).as_deref(), Some("Poly Pluck C5"));
        assert_eq!(pad_name(&p, 4).as_deref(), Some("Break 92"));
        assert_eq!(p.kits[0].pads[0].as_deref(), Some("Poly Pluck C2"));
        assert!(matches!(
            p.sample.project(0).unwrap().pads_play(),
            PadsPlay::Keys { .. }
        ));
        assert_eq!(p.page(), sample_page::AUTO_PAGE);
        assert!(p
            .ui
            .get::<PadBar>(auto_tab::BAR)
            .unwrap()
            .subtitle
            .starts_with("Poly Pluck: 4 notes, 4 takes on pads A1 to A4"));
        // The kit sounds again with the new view.
        let (sent, _) = views(p.take_actions());
        assert_eq!(sent.len(), 1);
        // UNDO takes the whole instrument off, and the pads play pads.
        p.apply(ToUi::SampleUndo(false));
        assert_eq!(pad_name(&p, 0), None);
        assert_eq!(pad_name(&p, 3), None);
        assert_eq!(pad_name(&p, 4).as_deref(), Some("Break 92"));
        assert!(matches!(
            p.sample.project(0).unwrap().pads_play(),
            PadsPlay::Pads
        ));
        // Rec goes back to recording.
        ev(&mut p, auto_tab::BUTTONS, EventKind::Tab(1));
        assert_eq!(p.page(), sample_page::REC_PAGE);
    }

    #[test]
    fn send_to_pad_copies_the_pad_onto_the_next_hit_and_chop_to_pads_opens_the_chop_tab() {
        use mpfree_sampler_ui::send;
        use mpfree_sampler_ui::widgets::ButtonRow;
        let pad = mpfree_sampler_ui::sample::PAD;
        let mut p = sample_sampler(Fonts::embedded());
        let press = |p: &mut Player, i: usize| {
            p.on_event(UiEvent {
                widget: send::SEND.into(),
                kind: EventKind::Tab(i),
            })
        };
        let label = |p: &Player| p.ui.get::<ButtonRow>(send::SEND).unwrap().labels[0].clone();
        assert_eq!(p.page(), sample_page::PAGE);
        assert_eq!(
            p.ui.get::<ButtonRow>(send::SEND).unwrap().enabled,
            [true, true]
        );
        // A second press calls it off; a third waits for the hit.
        press(&mut p, send::SEND_TO_PAD);
        assert_eq!(label(&p), send::SENDING);
        press(&mut p, send::SEND_TO_PAD);
        assert_eq!(label(&p), send::BUTTONS[0]);
        press(&mut p, send::SEND_TO_PAD);
        // The hit on A2 gets A5 with its edits, and the editor follows it.
        let a5 = p.sample.project(0).unwrap().pad(pad).unwrap().clone();
        p.apply(ToUi::PadDown {
            pad: 1,
            velocity: 90,
        });
        p.apply(ToUi::PadUp(1));
        assert_eq!(p.sample.project(0).unwrap().pad(1), Some(&a5));
        assert_eq!(p.kits[0].pads[1].as_deref(), Some("Break 92"));
        assert_eq!(
            (p.page(), p.sample.state().map(|s| s.pad)),
            (sample_page::PAGE, Some(1))
        );
        assert_eq!(label(&p), send::BUTTONS[0]);
        assert!(
            events_log(&p)
                .iter()
                .any(|l| l == "sample: pad A5 sent to pad A2"),
            "{:?}",
            events_log(&p)
        );
        // One undo step takes it back.
        p.apply(ToUi::SampleUndo(false));
        assert!(p.sample.project(0).unwrap().pad(1).is_none());
        // Chop to pads opens the Chop tab; there, it chops.
        p.apply(ToUi::PadDown { pad, velocity: 90 });
        p.apply(ToUi::PadUp(pad));
        press(&mut p, send::CHOP_TO_PADS);
        assert_eq!(p.page(), sample_page::CHOP_PAGE);
        assert!(
            p.sample.project(0).unwrap().pad(1).is_none(),
            "nothing chopped yet"
        );
        press(&mut p, send::CHOP_TO_PADS);
        assert!(
            p.sample.project(0).unwrap().pad(1).is_some(),
            "chopped onto pads"
        );
        // A pad without a sound has nothing to send: the buttons are faint.
        p.apply(ToUi::SampleUndo(false));
        p.apply(ToUi::PadDown {
            pad: 2,
            velocity: 90,
        });
        assert_eq!(p.page(), sample_page::EMPTY_PAGE);
        p.on_event(UiEvent {
            widget: "header".into(),
            kind: EventKind::Tab(5),
        });
        assert_eq!(
            p.ui.get::<ButtonRow>(send::SEND).unwrap().enabled,
            [false, false]
        );
    }

    #[test]
    fn a_waiting_send_to_pad_takes_the_hit_before_the_play_tabs_stops_row() {
        use mpfree_sampler_ui::play_tab::{self, Row, LIST};
        use mpfree_sampler_ui::send;
        use mpfree_sampler_ui::widgets::SettingList;
        let pad = mpfree_sampler_ui::sample::PAD;
        let mut p = sample_sampler(Fonts::embedded());
        let ev = |p: &mut Player, widget: &str, kind: EventKind| {
            p.on_event(UiEvent {
                widget: widget.into(),
                kind,
            })
        };
        ev(
            &mut p,
            mpfree_sampler_ui::wave_tab::BAR,
            EventKind::Tab(play_tab::TAB),
        );
        let stops = Row::ALL.iter().position(|r| *r == Row::Chokes).unwrap();
        p.ui.get_mut::<SettingList>(LIST).unwrap().selected = stops;
        ev(&mut p, LIST, EventKind::Select(stops));
        assert!(p.sample.play_takes_pads(&p.ui));
        // Send to pad pressed: the next hit is the pad the copy goes on,
        // not a pad that stops this one.
        ev(&mut p, send::SEND, EventKind::Tab(send::SEND_TO_PAD));
        p.apply(ToUi::PadDown {
            pad: 1,
            velocity: 90,
        });
        p.apply(ToUi::PadUp(1));
        let a5 = p.sample.project(0).unwrap().pad(pad).unwrap().clone();
        assert!(a5.chokes.is_empty(), "{:?}", a5.chokes);
        assert_eq!(p.sample.project(0).unwrap().pad(1), Some(&a5));
        assert!(
            events_log(&p)
                .iter()
                .any(|l| l == "sample: pad A5 sent to pad A2"),
            "{:?}",
            events_log(&p)
        );
    }

    #[test]
    fn the_pads_tab_picks_copies_and_clears_pads_and_undo_takes_it_back() {
        use mpfree_sampler_ui::pads_tab::{self, Row};
        use mpfree_sampler_ui::widgets::{PadBar, PadMap};
        let pad = mpfree_sampler_ui::sample::PAD;
        let mut p = sample_sampler(Fonts::embedded());
        let ev = |p: &mut Player, widget: &str, kind: EventKind| {
            p.on_event(UiEvent {
                widget: widget.into(),
                kind,
            })
        };
        // The Pads tab opens on the same pad; its switch shows the Pad
        // view, and the app's controls follow it.
        open_pads(&mut p, pads_tab::BROWSE);
        assert_eq!(
            (p.page(), p.sample.editor()),
            (sample_page::PADS_BROWSE_PAGE, sample_page::Editor::Pads)
        );
        assert_eq!(p.ui.get::<Header>("header").unwrap().active, 1);
        ev(&mut p, pads_tab::BAR, EventKind::Tab(pads_tab::PAD_VIEW));
        assert_eq!(p.take_actions(), vec![FromUi::PadsView(pads_tab::PAD_VIEW)]);
        assert_eq!(p.page(), sample_page::PADS_PAGE);
        assert!(p
            .ui
            .get::<Label>("hint")
            .unwrap()
            .text
            .contains("Q-Links 1-4: level, pan, tune, voices"));
        assert_eq!(
            p.ui.get::<PadMap>(pads_tab::MAP).unwrap().title,
            "Kit 1 · Breaks"
        );
        assert_eq!(p.ui.get::<PadMap>(pads_tab::MAP).unwrap().selected, pad);
        // A hit lights its pad on the map until it lifts.
        p.apply(ToUi::PadDown { pad, velocity: 90 });
        assert!(p.ui.get::<PadMap>(pads_tab::MAP).unwrap().lit[pad]);
        p.apply(ToUi::PadUp(pad));
        assert!(!p.ui.get::<PadMap>(pads_tab::MAP).unwrap().lit[pad]);
        p.take_actions();
        // A tap on the map's A5 (picked) plays it while held.
        let r = p.ui.rect_of(pads_tab::MAP).unwrap();
        let a5 = p.ui.get::<PadMap>(pads_tab::MAP).unwrap().cell_rect(r, pad);
        p.touch(4, Touch::Down, a5.x + 10, a5.y + 10);
        p.touch(4, Touch::Up, a5.x + 10, a5.y + 10);
        let pads: Vec<(usize, bool)> = p
            .take_actions()
            .into_iter()
            .filter_map(|a| match a {
                FromUi::Pad { pad, down } => Some((pad, down)),
                _ => None,
            })
            .collect();
        assert_eq!(pads, vec![(pad, true), (pad, false)]);
        // Copy A5, pick A2 on the map, paste: A2 plays the loop, the Play
        // page names it, the view goes out, and the log says so.
        ev(&mut p, pads_tab::BUTTONS, EventKind::Tab(0));
        ev(&mut p, pads_tab::MAP, EventKind::Select(1));
        assert_eq!(
            p.ui.get::<PadBar>(pads_tab::BAR).unwrap().title,
            "Pad A2 · no sound"
        );
        assert_eq!(p.page(), sample_page::PADS_PAGE);
        ev(&mut p, pads_tab::BUTTONS, EventKind::Tab(1));
        assert_eq!(
            p.sample.project(0).unwrap().pad(1).unwrap().name,
            "Break 92"
        );
        assert_eq!(p.kits[0].pads[1].as_deref(), Some("Break 92"));
        assert!(
            events_log(&p)
                .iter()
                .any(|l| l == "sample: kit 1 (Breaks) pad A2: now plays Break 92"),
            "{:?}",
            events_log(&p)
        );
        let (sent, _) = views(p.take_actions());
        assert!(sent[0].view.as_ref().unwrap().has_pad(1));
        p.apply(ToUi::SampleViewBack(sent.into_iter().next().unwrap()));
        // Q-Link 1 turns A2's level: one undo step, one log line.
        p.apply(ToUi::SampleQLink(0, -4));
        p.apply(ToUi::SampleLift);
        assert_eq!(p.sample.project(0).unwrap().pad(1).unwrap().level_db, -2.0);
        assert_eq!(p.sample.pads().unwrap().0.row(&p.ui), Row::Level);
        assert!(
            events_log(&p)
                .iter()
                .any(|l| l == "sample: kit 1 (Breaks) pad A2: level +0.0 dB -> -2.0 dB"),
            "{:?}",
            events_log(&p)
        );
        // Clear it, then UNDO twice: the level back, then the copy back.
        ev(&mut p, pads_tab::BUTTONS, EventKind::Tab(2));
        assert!(p.sample.project(0).unwrap().pad(1).is_none());
        assert_eq!(p.kits[0].pads[1], None);
        p.apply(ToUi::SampleUndo(false));
        assert_eq!(p.sample.project(0).unwrap().pad(1).unwrap().level_db, -2.0);
        p.apply(ToUi::SampleUndo(false));
        p.apply(ToUi::SampleUndo(false));
        assert!(
            p.sample.project(0).unwrap().pad(1).is_none(),
            "the paste is undone too"
        );
        assert_eq!(p.page(), sample_page::PADS_PAGE);
        // The pad picked there is the one the Sample tab edits, on the
        // editor it had.
        ev(&mut p, pads_tab::MAP, EventKind::Select(pad));
        p.apply(ToUi::Show(View::Sample));
        assert_eq!(p.page(), sample_page::PAGE);
        assert_eq!(p.sample.state().unwrap().pad, pad);
        p.apply(ToUi::SampleEditor(sample_page::Editor::Play));
        assert_eq!(p.page(), sample_page::PLAY_PAGE);
        // And back: the Pads tab on the view it had.
        p.apply(ToUi::Show(View::Pads));
        assert_eq!(p.page(), sample_page::PADS_PAGE);
        p.apply(ToUi::Show(View::Sample));
        assert_eq!(p.page(), sample_page::PLAY_PAGE);
    }

    #[test]
    fn map_to_keys_makes_the_bank_one_instrument_and_the_play_tab_plays_its_keys() {
        use mpfree_sampler::model::PadsPlay;
        use mpfree_sampler_ui::keys_widgets::KeyZones;
        use mpfree_sampler_ui::widgets::PadBar;
        use mpfree_sampler_ui::{pads_tab, play_tab};
        let pad = mpfree_sampler_ui::sample::PAD;
        let mut p = sample_sampler(Fonts::embedded());
        let ev = |p: &mut Player, widget: &str, kind: EventKind| {
            p.on_event(UiEvent {
                widget: widget.into(),
                kind,
            })
        };
        // A1-A3 play the loop under names with notes; A5 keeps "Break 92",
        // whose pitch is heard as G#1.
        let (mut project, _) = mpfree_sampler_ui::sample::project();
        let sound = project.pad(pad).unwrap().clone();
        for (k, name) in ["Keys C2", "Keys G2", "Keys C3"].into_iter().enumerate() {
            let mut q = sound.clone();
            q.name = name.to_string();
            project.set_pad(k, Some(q), None).unwrap();
        }
        p.apply(ToUi::SampleKits {
            projects: vec![project],
            rate: mpfree_sampler_ui::sample::RATE,
        });
        let keys = |p: &Player, pad: usize| p.sample.project(0).unwrap().pad(pad).unwrap().keys;
        open_pads(&mut p, pads_tab::PAD_VIEW);
        assert_eq!(p.page(), sample_page::PADS_PAGE);
        p.take_actions();
        // Map to keys: each pad at its sound's note, spread halfway to the
        // next; the pads play notes; the log says where the notes came from.
        ev(&mut p, pads_tab::BUTTONS, EventKind::Tab(3));
        let roots: Vec<Option<u8>> = [0, 1, 2, pad]
            .into_iter()
            .map(|k| keys(&p, k).map(|z| z.root))
            .collect();
        assert_eq!(roots, vec![Some(36), Some(43), Some(48), Some(32)]);
        assert_eq!(
            [keys(&p, pad).unwrap().lo, keys(&p, 0).unwrap().lo],
            [0, 34]
        );
        assert_eq!(keys(&p, 2).unwrap().hi, 127);
        assert!(matches!(
            p.sample.project(0).unwrap().pads_play(),
            PadsPlay::Keys { .. }
        ));
        assert!(
            events_log(&p).iter().any(|l| l
                == "sample: kit 1 (Breaks): mapped to keys: 4 pads from G#1 to C3 \
                    (3 by its name, 1 by its pitch), the pads play notes"),
            "{:?}",
            events_log(&p)
        );
        let (sent, _) = views(p.take_actions());
        assert_eq!(sent.len(), 1);
        p.apply(ToUi::SampleViewBack(sent.into_iter().next().unwrap()));
        // The Sample tab's Play tab shows the kit's keyboard, with its own
        // hint.
        open_sample(&mut p, sample_page::Editor::Play);
        assert_eq!(p.page(), sample_page::PLAY_PAGE);
        assert!(p.sample.play_keys_shown());
        assert!(p
            .ui
            .get::<Label>("hint")
            .unwrap()
            .text
            .starts_with("Press keys to hear the kit"));
        assert_eq!(p.ui.get::<KeyZones>(play_tab::KEYS).unwrap().zones.len(), 4);
        // A key plays the kit's keygroups while held.
        let r = p.ui.rect_of(play_tab::KEYS).unwrap();
        let (x, y) =
            p.ui.get::<KeyZones>(play_tab::KEYS)
                .unwrap()
                .key_point(r, 45)
                .unwrap();
        p.touch(3, Touch::Down, x, y);
        p.touch(3, Touch::Up, x, y);
        let keys_sent: Vec<(usize, u8, u8, bool)> = p
            .take_actions()
            .into_iter()
            .filter_map(|a| match a {
                FromUi::SampleKey {
                    kit,
                    note,
                    velocity,
                    down,
                } => Some((kit, note, velocity, down)),
                _ => None,
            })
            .collect();
        assert_eq!(
            keys_sent,
            vec![
                (0, 45, play_tab::KEY_VELOCITY, true),
                (0, 45, play_tab::KEY_VELOCITY, false)
            ]
        );
        // A tap on A1's zone picks A1.
        let z = p.ui.get::<KeyZones>(play_tab::KEYS).unwrap();
        let i = z.zones.iter().position(|z| z.pad == 0).unwrap();
        let zr = z.zone_rect(r, i).unwrap();
        p.touch(3, Touch::Down, zr.x + zr.w / 2, zr.y + zr.h / 2);
        p.touch(3, Touch::Up, zr.x + zr.w / 2, zr.y + zr.h / 2);
        assert_eq!(
            p.ui.get::<PadBar>(play_tab::BAR).unwrap().title,
            "Pad A1 · Keys C2"
        );
        // Dragging its right edge to G#2 is one undo step.
        let z = p.ui.get::<KeyZones>(play_tab::KEYS).unwrap();
        let zr = z.zone_rect(r, i).unwrap();
        let (to, _) = z.key_point(r, 44).unwrap();
        let edge = zr.x + zr.w;
        p.touch(3, Touch::Down, edge, zr.y + zr.h / 2);
        p.touch(3, Touch::Move, (edge + to) / 2, zr.y + zr.h / 2);
        p.touch(3, Touch::Move, to, zr.y + zr.h / 2);
        p.touch(3, Touch::Up, to, zr.y + zr.h / 2);
        p.apply(ToUi::SampleLift);
        assert_eq!(keys(&p, 0).unwrap().hi, 44);
        p.apply(ToUi::SampleUndo(false));
        assert_eq!(keys(&p, 0).unwrap().hi, 39);
        // One more UNDO takes the whole mapping back.
        p.apply(ToUi::SampleUndo(false));
        assert!([0, 1, 2, pad].iter().all(|&k| keys(&p, k).is_none()));
        assert!(matches!(
            p.sample.project(0).unwrap().pads_play(),
            PadsPlay::Pads
        ));
        assert!(!p.sample.play_keys_shown());
        assert!(p
            .ui
            .get::<Label>("hint")
            .unwrap()
            .text
            .starts_with("Hit a pad: its velocity shows"));
    }

    #[test]
    fn with_drums_on_the_pads_the_synth_tab_follows_the_pad_hit_last() {
        let mut p = sample_player(Fonts::embedded());
        let synth_kit = sample_kits().len();
        let mut s = crate::synth::Synth::new(44_100);
        let mut set = s.settings();
        set.engine = crate::synth::Engine::Drums;
        s.set_settings(set);
        p.apply(ToUi::Synth(s.state()));
        p.apply(ToUi::KitChanged {
            kit: synth_kit,
            view: s.kit_view(),
        });
        let page = |p: &Player| p.synth.as_ref().unwrap().page();
        // A sample kit plays the pads: the Synth tab stays put.
        p.apply(ToUi::PadDown {
            pad: 5,
            velocity: 100,
        });
        assert_eq!(page(&p), 0);
        p.apply(ToUi::Kit(synth_kit));
        assert_eq!(
            p.ui.get::<MiniPads>(play_page::PADS)
                .map(|m| &m.grid)
                .unwrap()
                .pads[0]
                .label,
            "Kick"
        );
        p.apply(ToUi::PadDown {
            pad: 5,
            velocity: 100,
        });
        assert_eq!(page(&p), mpfree_synth_ui::pages::drum_page(5));
        assert_eq!(p.synth.as_ref().unwrap().pages()[page(&p)].name, "Pad 6");
        // Another engine's pages don't move with the pads.
        set.engine = crate::synth::Engine::Pluck;
        s.set_settings(set);
        p.apply(ToUi::Synth(s.state()));
        p.apply(ToUi::PadDown {
            pad: 2,
            velocity: 100,
        });
        assert_eq!(page(&p), 0);
    }

    #[test]
    fn the_synth_kit_and_page_follow_an_engine_switch_and_a_scale() {
        let mut p = sample_player(Fonts::embedded());
        let synth_kit = sample_kits().len();
        let chips = |p: &Player| -> Vec<String> {
            p.ui.get::<Header>("header")
                .unwrap()
                .chips
                .iter()
                .map(|c| c.text.clone())
                .collect()
        };
        p.apply(ToUi::Kit(synth_kit));
        let mut s = crate::synth::Synth::new(44_100);
        let mut set = s.settings();
        set.engine = crate::synth::Engine::Fm;
        s.set_settings(set);
        p.apply(ToUi::KitChanged {
            kit: synth_kit,
            view: s.kit_view(),
        });
        p.apply(ToUi::Synth(s.state()));
        assert_eq!(
            p.ui.get::<ListView>("kits").unwrap().items[synth_kit].text,
            "Synth: FM"
        );
        assert!(
            chips(&p).contains(&"FM · C3 to D#4".to_string()),
            "{:?}",
            chips(&p)
        );
        assert_eq!(p.synth.as_ref().unwrap().state.info.id, "mpfree.fm");
        // A scale and Chord pads: the chip says so and the pads name their chords.
        set.scale = mpfree_arp::scale::by_name("Minor")
            .map(|d| mpfree_arp::scale::Scale::with_down(9, d.mask, d.down));
        set.set_layout(mpfree_arp::scale::pads::PadLayout::Chords);
        s.set_settings(set);
        p.apply(ToUi::KitChanged {
            kit: synth_kit,
            view: s.kit_view(),
        });
        assert!(
            chips(&p).contains(&"FM · A Minor · chord pads".to_string()),
            "{:?}",
            chips(&p)
        );
        assert_eq!(
            p.ui.get::<MiniPads>(play_page::PADS)
                .map(|m| &m.grid)
                .unwrap()
                .pads[0]
                .label,
            "i Am"
        );
        // The pads panel's heading has the switch, neither half lit for Chord pads.
        let m = p.ui.get::<MiniPads>(play_page::PADS).unwrap();
        assert_eq!(
            m.synth.map(|s| (s.layout, s.scale)),
            Some((mpfree_arp::scale::pads::PadLayout::Chords, true))
        );
        assert_eq!(
            (m.title.as_str(), m.right.as_str()),
            ("Synth · Bank A", "Synth: FM")
        );
        // The Pressure page's live bars.
        p.apply(ToUi::SynthPressures(vec![0.5, 0.25]));
        assert_eq!(p.synth.as_ref().unwrap().state.pressures, vec![0.5, 0.25]);
    }

    #[test]
    fn the_synth_page_shows_poly_and_sends_its_changes() {
        let mut p = sample_player(Fonts::embedded());
        let synth_kit = sample_kits().len();
        let chips = |p: &Player| -> Vec<String> {
            p.ui.get::<Header>("header")
                .unwrap()
                .chips
                .iter()
                .map(|c| c.text.clone())
                .collect()
        };
        assert!(
            chips(&p).contains(&"Classic House · Bank A".to_string()),
            "{:?}",
            chips(&p)
        );
        p.apply(ToUi::Kit(synth_kit));
        assert!(
            chips(&p).contains(&"Poly · C3 to D#4".to_string()),
            "{:?}",
            chips(&p)
        );
        assert_eq!(
            p.ui.get::<MiniPads>(play_page::PADS)
                .map(|m| &m.grid)
                .unwrap()
                .pads[1]
                .label,
            "C#3"
        );
        // The Synth tab is the fourth; the app decides, then the page shows.
        let header = p.ui.rect_of("header").unwrap();
        let synth_tab = p.ui.get::<Header>("header").unwrap().tab_rect(header, 3);
        assert_eq!(
            p.touch(1, Touch::Down, synth_tab.x + 20, synth_tab.y + 20),
            Some(FromUi::Tab(3))
        );
        p.apply(ToUi::Show(View::Synth));
        assert_eq!(p.page(), "Synth");
        assert!(p.ui.rect_of(synth_view::TILES).is_some_and(|r| r.h > 300));
        assert!(p
            .ui
            .get::<Label>("hint")
            .unwrap()
            .text
            .contains("Q-Links 1-4"));
        // The shared Q-Link column shows what the page's Q-Links turn.
        let column = |p: &Player| {
            let q = p.ui.get::<QLinkColumn>(play_page::QLINKS).unwrap();
            (q.title.clone(), q.cards.clone())
        };
        assert!(p.ui.rect_of(play_page::QLINKS).is_some_and(|r| r.w == 168));
        assert_eq!(column(&p).0, "Q-Links · Synth");
        assert_eq!(column(&p).1, p.synth.as_ref().unwrap().qlink_cards());
        let before = column(&p).1[0].value.clone();
        // A Q-Link turns its knob on screen at once and sends the change.
        assert!(p.take_actions().is_empty());
        p.apply(ToUi::SynthQLink(0, 5));
        let acts = p.take_actions();
        assert!(
            matches!(acts.as_slice(), [FromUi::Synth(SynthAction::Set { .. })]),
            "{acts:?}"
        );
        assert_ne!(column(&p).1[0].value, before, "the card follows");
        // A drag on a card does the same.
        let ev = UiEvent {
            widget: play_page::QLINKS.to_string(),
            kind: EventKind::Step {
                index: 0,
                delta: -5,
            },
        };
        assert!(matches!(
            p.on_event(ev),
            Some(FromUi::Synth(SynthAction::Set { .. }))
        ));
        assert_eq!(column(&p).1[0].value, before);
        p.apply(ToUi::SynthWheel(-3));
        assert_eq!(p.take_actions().len(), 1);
        // The wheel navigation (app::RowKey): the wheel picks tiles, - / +
        // and an opened tile's wheel turn the tile picked.
        use crate::app::RowKey;
        // The control picked, as a place in the page's order of controls.
        let tiles = |p: &Player| {
            let v =
                p.ui.get::<mpfree_synth_ui::section_view::SectionView>(synth_view::TILES)
                    .unwrap();
            let order = p.synth.as_ref().unwrap().order();
            let at = v.selected.and_then(|i| order.iter().position(|&o| o == i));
            (at, v.editing, order.len())
        };
        let (at, _, n) = tiles(&p);
        let at = at.unwrap();
        p.apply(ToUi::Rows(RowKey::Pick(1)));
        assert_eq!(tiles(&p).0, Some((at + 1).min(n - 1)));
        p.apply(ToUi::Rows(RowKey::Pick(-99)));
        assert_eq!(tiles(&p).0, Some(0));
        assert!(p.take_actions().is_empty(), "picking changes nothing");
        p.apply(ToUi::Rows(RowKey::Editing(true)));
        assert!(tiles(&p).1);
        p.apply(ToUi::Rows(RowKey::Step(1)));
        assert_eq!(p.take_actions().len(), 1);
        p.apply(ToUi::Rows(RowKey::Editing(false)));
        assert_eq!(tiles(&p), (Some(0), false, n));
        // A preset touched goes to the audio thread, whose values come back.
        let ev = UiEvent {
            widget: synth_view::PRESETS.to_string(),
            kind: EventKind::Select(3),
        };
        assert_eq!(p.on_event(ev), Some(FromUi::Synth(SynthAction::Preset(3))));
        let mut s = crate::synth::Synth::new(44_100);
        s.load_preset(3);
        p.apply(ToUi::SynthValues(s.values()));
        assert_eq!(p.synth.as_ref().unwrap().state.values, s.values());
        // An octave up relabels the synth's pads and its chip.
        s.shift_octave(1);
        p.apply(ToUi::KitChanged {
            kit: synth_kit,
            view: s.kit_view(),
        });
        assert!(
            chips(&p).contains(&"Poly · C4 to D#5".to_string()),
            "{:?}",
            chips(&p)
        );
        assert_eq!(
            p.ui.get::<MiniPads>(play_page::PADS)
                .map(|m| &m.grid)
                .unwrap()
                .pads[0]
                .label,
            "C4"
        );
        // Meters update the page's copy.
        let i = p
            .synth
            .as_ref()
            .unwrap()
            .state
            .index(mpfree_synth::params::METER_VOICES)
            .unwrap();
        p.apply(ToUi::SynthMeters(vec![(i, 3.0)]));
        assert_eq!(p.synth.as_ref().unwrap().state.values[i], 3.0);
    }

    #[test]
    fn appearance_switches_the_theme_live() {
        let mut p = sample_player(Fonts::embedded());
        let pick = |name: &str| Settings {
            theme: name.to_string(),
            ..Settings::default()
        };
        p.apply(ToUi::Settings(pick("console")));
        assert_eq!(p.ui.theme, Theme::builtin("console").unwrap());
        // A name that is not built in (a theme file from --theme) leaves it.
        p.apply(ToUi::Settings(pick("/run/mpfree-data/mine.theme")));
        assert_eq!(p.ui.theme, Theme::builtin("console").unwrap());
        p.apply(ToUi::Settings(pick("default")));
        assert_eq!(p.ui.theme, Theme::dark());
    }

    #[test]
    fn appearance_switches_to_a_themes_folder_theme() {
        let mut p = sample_player(Fonts::embedded());
        let mine = Theme::parse("base = synthwave\nname = Sunrise\naccent = #FF6C11\n").unwrap();
        p.apply(ToUi::ThemeFiles(vec![(
            "sunrise.theme".into(),
            mine.clone(),
        )]));
        p.apply(ToUi::Settings(Settings {
            theme: "sunrise.theme".into(),
            ..Settings::default()
        }));
        assert_eq!(p.ui.theme, mine);
    }

    #[test]
    fn a_paused_console_keeps_lines_for_later() {
        let mut p = sample_player(Fonts::embedded());
        let count = |p: &Player| p.ui.get::<LogPanel>("syslog").unwrap().lines().count();
        let before = count(&p);
        let paused = Settings {
            console_paused: true,
            ..Settings::default()
        };
        p.apply(ToUi::Settings(paused));
        p.apply(ToUi::Console("one".into()));
        p.apply(ToUi::Console("two".into()));
        assert_eq!(count(&p), before);
        p.apply(ToUi::Settings(Settings::default()));
        assert_eq!(count(&p), before + 2);
    }

    #[test]
    fn shift_shows_what_it_does_and_repeats_flash_pads() {
        let mut p = sample_player(Fonts::embedded());
        let mut s = Status {
            shift: true,
            pad_level: PadLevel::Off,
            repeat: false,
            latched: true,
            rate: "1/8",
            bpm: 96.0,
            mute_keys: MuteKeys::Off,
            erase: false,
        };
        p.apply(ToUi::Status(s.clone()));
        assert_eq!(p.ui.get::<Label>("hint").unwrap().text, SHIFT_HINT);
        let mut app = sample_app();
        app.settings.rate = mpfree_arp::Rate::Eighth;
        app.settings.bpm = 96.0;
        let fx = app.button(mpfree_controller::mpc_one::button::SHIFT, true, 0.0);
        let mut fx2 = app.button(mpfree_controller::mpc_one::button::NOTE_REPEAT, true, 0.0);
        fx2.extend(app.arp_poll());
        apply_effects(&mut p, fx);
        apply_effects(&mut p, fx2);
        let chips: Vec<String> =
            p.ui.get::<Header>("header")
                .unwrap()
                .chips
                .iter()
                .map(|c| c.text.clone())
                .collect();
        assert!(
            chips.contains(&"Repeat 1/8, latched · 96 BPM".to_string()),
            "{chips:?}"
        );
        assert!(
            !chips.iter().any(|c| c.ends_with(" dB")),
            "the level is on the slider: {chips:?}"
        );
        s.shift = false;
        p.apply(ToUi::Status(s.clone()));
        assert!(p
            .ui
            .get::<Label>("hint")
            .unwrap()
            .text
            .contains("NOTE REPEAT: repeat"));
        // TRACK MUTE: the hint says what the pads are now, on any page.
        for (keys, hint) in [
            (MuteKeys::Tracks, TRACK_MUTE_HINT),
            (MuteKeys::Pads, PAD_MUTE_HINT),
        ] {
            s.mute_keys = keys;
            p.apply(ToUi::Status(s.clone()));
            assert_eq!(p.ui.get::<Label>("hint").unwrap().text, hint);
        }
        s.mute_keys = MuteKeys::Off;
        p.apply(ToUi::Status(s));
        p.apply(ToUi::PadFlash {
            pad: 3,
            velocity: 90,
        });
        let g =
            p.ui.get::<MiniPads>(play_page::PADS)
                .map(|m| &m.grid)
                .unwrap();
        assert!(!g.pads[3].held && g.pads[3].level == 1.0);
    }

    #[test]
    fn the_quick_band_comes_with_note_repeat_and_the_chip_opens_the_panel() {
        use mpfree_controller::mpc_one::button::NOTE_REPEAT;
        let mut p = sample_player(Fonts::embedded());
        let throws = p.ui.rect_of(play_page::THROWS).unwrap();
        let mut app = sample_app();
        let fx = app.button(NOTE_REPEAT, true, 0.0);
        apply_effects(&mut p, fx);
        assert_eq!(p.page(), QUICK_PAGE);
        // A slim strip at the top of the view's first panel; the throws,
        // the transport, the master bar and the Q-Links stay put, and the
        // cards turn to the arp's.
        assert_eq!(p.ui.rect_of(play_page::THROWS).unwrap(), throws);
        for name in [play_page::MASTER, play_page::QLINKS, play_page::TRANSPORT] {
            assert!(p.ui.rect_of(name).is_some(), "{name} stays");
        }
        let q = p.ui.get::<QLinkColumn>(play_page::QLINKS).unwrap();
        assert_eq!(
            (q.cards[0].name.as_str(), q.cards[3].name.as_str()),
            ("Rate", "Octaves")
        );
        let strip = p.ui.rect_of(arp_view::MINI).unwrap();
        assert_eq!(strip.h, arp_view::MINI_H);
        // Its + is a faster rate.
        let bar =
            p.ui.get::<mpfree_arp_ui::widgets::MiniBar>(arp_view::MINI)
                .unwrap();
        assert_eq!(bar.rate, "1/16");
        let faster = bar.parts(strip)[mpfree_arp_ui::widgets::MINI_FASTER];
        let a = p.touch(7, Touch::Down, faster.x + 10, faster.y + faster.h / 2);
        assert_eq!(
            a,
            Some(FromUi::Arp(ArpAction::Rate(
                mpfree_arp::Rate::SixteenthTriplet
            )))
        );
        p.touch(7, Touch::Up, faster.x + 10, faster.y + faster.h / 2);
        let fx = app.button(NOTE_REPEAT, false, 0.0);
        apply_effects(&mut p, fx);
        assert_eq!(p.page(), "Play");
        // With the repeat let go the header keeps to its chips; a double
        // press latches it, and the arp chip shows.
        assert_eq!(p.arp_chip, None);
        let mut fx = app.button(NOTE_REPEAT, true, 0.1);
        fx.extend(app.button(NOTE_REPEAT, false, 0.15));
        apply_effects(&mut p, fx);
        // The arp chip, drawn once, opens the panel.
        let mut c = SoftCanvas::new(SCREEN.0, SCREEN.1);
        p.ui.paint(&mut c);
        let chip =
            p.ui.get::<Header>("header")
                .unwrap()
                .chip_rect(p.arp_chip.unwrap())
                .unwrap();
        assert_eq!(
            p.touch(8, Touch::Down, chip.x + 10, chip.y + 10),
            Some(FromUi::ArpChip)
        );
        p.touch(8, Touch::Up, chip.x + 10, chip.y + 10);
        let mut fx = app.toggle_panel();
        fx.extend(app.arp_poll());
        apply_effects(&mut p, fx);
        assert_eq!(p.page(), ARP_PAGE);
        // The panel takes the whole view; there are no pads on Play.
        assert!(p.ui.rect_of(play_page::PADS).is_none());
        assert!(p.ui.rect_of(arp_view::MODES).unwrap().x < 100);
        assert_eq!(
            p.ui.get::<Header>("header").unwrap().active,
            0,
            "the panel belongs to Play"
        );
        let modes = p.ui.rect_of(arp_view::MODES).unwrap();
        let a = p.touch(9, Touch::Down, modes.right() - 10, modes.y + modes.h / 2);
        assert_eq!(a, Some(FromUi::Arp(ArpAction::Mode(Mode::Pattern))));
        p.touch(9, Touch::Up, modes.right() - 10, modes.y + modes.h / 2);
        assert!(p
            .ui
            .get::<Label>("hint")
            .unwrap()
            .text
            .contains("NOTE REPEAT + MENU: close"));
    }

    #[test]
    fn a_page_change_lets_go_of_pads_held_on_the_screen() {
        let mut p = sample_player(Fonts::embedded());
        // Play has no pads now (Jay, 12 Nov); a page with them stands in.
        p.ui.add_page("pads-test", "col(header:60, pads:*, hint:26)")
            .unwrap();
        p.ui.show("pads-test").unwrap();
        let pads = p.ui.rect_of("pads").unwrap();
        assert_eq!(
            p.touch(1, Touch::Down, pads.x + 20, pads.bottom() - 20),
            Some(FromUi::Pad { pad: 0, down: true })
        );
        let mut app = sample_app();
        let fx = app.button(mpfree_controller::mpc_one::button::NOTE_REPEAT, true, 0.0);
        apply_effects(&mut p, fx);
        assert_eq!(p.page(), QUICK_PAGE);
        assert_eq!(
            p.take_actions(),
            vec![FromUi::Pad {
                pad: 0,
                down: false
            }]
        );
        assert!(p.take_actions().is_empty());
    }

    #[test]
    fn the_workspace_edits_by_touch_and_shows_what_the_pads_write() {
        let mut p = sample_player(Fonts::embedded());
        let mut app = sample_app();
        let mut fx = app.toggle_panel();
        fx.extend(app.arp_action(ArpAction::Button(mpfree_arp_ui::view::PatternButton::Edit)));
        apply_effects(&mut p, fx);
        assert_eq!(p.page(), PATTERN_PAGE);
        assert!(p.ui.rect_of("pads").is_none(), "editing is full screen");
        // A tap on the first step of the Step row: an accent on Straight's "x".
        let grid = p.ui.rect_of(workspace::GRID).unwrap();
        let a = p.touch(2, Touch::Down, grid.x + 112 + 10, grid.y + 28 + 10);
        let Some(FromUi::Workspace(ws)) = a else {
            panic!("{a:?}")
        };
        let WsAction::Changed(pat) = *ws else {
            panic!("{ws:?}")
        };
        assert_eq!(pat.to_text(), "x!");
        p.touch(2, Touch::Up, grid.x + 122, grid.y + 38);
        apply_effects(&mut p, app.workspace_action(WsAction::Changed(pat)));
        // The wheel turns the selected cell (here the note pick) through the app and back.
        p.workspace.editor.select(Row::Note, 0);
        apply_effects(&mut p, app.wheel(1));
        let back = p.take_actions();
        assert!(
            matches!(back.as_slice(), [FromUi::Workspace(a)] if matches!(**a, WsAction::Changed(_))),
            "{back:?}"
        );
        // Recording: the pads say what they write.
        let fx = app.workspace_action(WsAction::Record(true));
        apply_effects(&mut p, fx);
        assert_eq!(p.page(), RECORD_PAGE);
        assert_eq!(
            p.ui.get::<MiniPads>(play_page::PADS)
                .map(|m| &m.grid)
                .unwrap()
                .pads[12]
                .label,
            "Next"
        );
        let fx = app.workspace_action(WsAction::Back);
        apply_effects(&mut p, fx);
        assert_eq!(p.page(), ARP_PAGE);
        assert_eq!(
            p.ui.get::<MiniPads>(play_page::PADS)
                .map(|m| &m.grid)
                .unwrap()
                .pads[0]
                .label,
            "Kick",
            "the kit's names again"
        );
    }

    #[test]
    fn every_sample_screen_draws() {
        // sample_screens builds every screen's Player in one frame, which in
        // a debug build outgrows the 2 MiB test thread (the binary's
        // --screens runs on the 8 MiB main thread), so draw them on a thread
        // with the main thread's stack.
        std::thread::Builder::new()
            .stack_size(8 << 20)
            .spawn(|| {
                for (name, mut p) in sample_screens(Fonts::embedded) {
                    let mut c = SoftCanvas::new(SCREEN.0, SCREEN.1);
                    let d = p.ui.paint(&mut c);
                    assert_eq!(d.len(), 1, "{name}: a first paint covers the screen");
                }
            })
            .unwrap()
            .join()
            .unwrap();
    }

    /// The GPU seam (mpfree_ui::record): every sample screen recorded as a
    /// display list, sent through the wire format (as `--display-lists`
    /// writes it and a renderer in another process reads it) and played
    /// back is the same picture, bit for bit, as drawn straight; and what
    /// each list sends (`--nocapture` prints the table).
    #[test]
    fn every_sample_screen_plays_back_the_same_from_its_display_list() {
        std::thread::Builder::new()
            .stack_size(8 << 20)
            .spawn(|| {
                let direct = sample_screens(Fonts::embedded);
                let recorded = sample_screens(Fonts::embedded);
                let mut sent = 0usize;
                eprintln!(
                    "screen                     ops  masks  new items  KiB sent  next frame KiB"
                );
                for ((name, mut a), (_, mut b)) in direct.into_iter().zip(recorded) {
                    let mut straight = SoftCanvas::new(SCREEN.0, SCREEN.1);
                    a.ui.paint(&mut straight);
                    let mut rec = mpfree_ui::record::Recorder::new(SCREEN.0, SCREEN.1);
                    let mut copy = mpfree_ui::record::Store::new(usize::MAX);
                    let mut wire = Vec::new();
                    // Sent as bytes, read back, and played from the copy.
                    let mut send = |rec: &mut mpfree_ui::record::Recorder,
                                    played: &mut SoftCanvas| {
                        wire.clear();
                        rec.encode_frame(&mut wire);
                        let (frame, _) = mpfree_ui::record::Frame::decode(&wire).unwrap();
                        assert_eq!(frame.ops, rec.ops());
                        copy.apply(&frame);
                        mpfree_ui::record::replay(&frame.ops, &copy, played);
                    };
                    rec.begin_frame();
                    b.ui.paint(&mut rec);
                    let first = rec.stats();
                    let mut played = SoftCanvas::new(SCREEN.0, SCREEN.1);
                    send(&mut rec, &mut played);
                    assert!(
                        straight.px == played.px,
                        "{name}: the played-back picture differs"
                    );
                    // The next frame, a tick later, as the screen loop paints it.
                    a.tick(0.033);
                    b.tick(0.033);
                    a.ui.paint(&mut straight);
                    rec.begin_frame();
                    b.ui.paint(&mut rec);
                    let next = rec.stats();
                    send(&mut rec, &mut played);
                    assert!(straight.px == played.px, "{name}: the next frame differs");
                    sent += first.sent_bytes();
                    eprintln!(
                        "{name:<26} {:>4} {:>6} {:>10} {:>9.1} {:>15.1}",
                        first.ops,
                        first.masks,
                        first.new_items,
                        first.sent_bytes() as f64 / 1024.0,
                        next.sent_bytes() as f64 / 1024.0
                    );
                }
                assert!(sent > 0);
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn input_events_parse_for_this_build() {
        let mut b = vec![0u8; EVENT_SIZE];
        let word = EVENT_SIZE / 2 - 4;
        b[..word].copy_from_slice(&2u64.to_ne_bytes()[..word]);
        b[word..2 * word].copy_from_slice(&500u64.to_ne_bytes()[..word]);
        let base = EVENT_SIZE - 8;
        b[base..base + 2].copy_from_slice(&3u16.to_ne_bytes());
        b[base + 2..base + 4].copy_from_slice(&0x35u16.to_ne_bytes());
        b[base + 4..].copy_from_slice(&(-5i32).to_ne_bytes());
        assert_eq!(
            parse_input_event(&b),
            InputEvent::new(2_000_500, 3, 0x35, -5)
        );
    }

    /// A stand-in touchscreen: queued reads, and no slot snapshot (as on a
    /// node that refuses the ioctl), so the reader runs unseeded.
    struct FakeTouch(VecDeque<Vec<u8>>);

    impl TouchSource for FakeTouch {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let Some(chunk) = self.0.pop_front() else {
                return Err(ErrorKind::WouldBlock.into());
            };
            buf[..chunk.len()].copy_from_slice(&chunk);
            Ok(chunk.len())
        }
        fn snapshot(&mut self, _slots: usize) -> io::Result<mpfree_touch::mt::MtSnapshot> {
            Err(ErrorKind::Unsupported.into())
        }
    }

    #[test]
    fn touch_reports_turn_into_screen_fingers() {
        // One finger down near raw (1024, 2048).
        let mut evs = Vec::new();
        let ev = |t: u16, c: u16, v: i32| {
            let mut b = vec![0u8; EVENT_SIZE];
            let base = EVENT_SIZE - 8;
            b[base..base + 2].copy_from_slice(&t.to_ne_bytes());
            b[base + 2..base + 4].copy_from_slice(&c.to_ne_bytes());
            b[base + 4..].copy_from_slice(&v.to_ne_bytes());
            b
        };
        for (t, c, v) in [
            (3, 0x2f, 0),
            (3, 0x39, 5),
            (3, 0x35, 1024),
            (3, 0x36, 2048),
            (0, 0, 0),
        ] {
            evs.extend(ev(t, c, v));
        }
        let t = TouchIn::from_source(Box::new(FakeTouch(VecDeque::new())), SCREEN).unwrap();
        assert!(
            t.status_line().starts_with("touch-reader: seeded=0 "),
            "{}",
            t.status_line()
        );
        assert!(t.status_line().contains("running unseeded"));
        // The report arrives after open, so the open-time drain doesn't drop it:
        // the fake's first read is the drain's (empty), the second is ours.
        let mut t2 = TouchIn::from_source(
            Box::new(FakeTouch(VecDeque::from([Vec::new(), evs]))),
            SCREEN,
        )
        .unwrap();
        let got = t2.read().unwrap();
        assert_eq!(got.len(), 1);
        let (_, phase, x, y, _) = got[0];
        assert_eq!(phase, Touch::Down);
        // Raw Y runs right to left (2048 = left edge); raw X runs top to bottom.
        assert_eq!((x, y), (0, 400));
        assert_eq!(t2.contacts, 1);
    }

    /// One finger down near raw (1024, 2048), as the panel reports it.
    fn one_finger_report() -> Vec<u8> {
        let ev = |t: u16, c: u16, v: i32| {
            let mut b = vec![0u8; EVENT_SIZE];
            let base = EVENT_SIZE - 8;
            b[base..base + 2].copy_from_slice(&t.to_ne_bytes());
            b[base + 2..base + 4].copy_from_slice(&c.to_ne_bytes());
            b[base + 4..].copy_from_slice(&v.to_ne_bytes());
            b
        };
        let mut evs = Vec::new();
        for (t, c, v) in [
            (3, 0x2f, 0),
            (3, 0x39, 5),
            (3, 0x35, 1024),
            (3, 0x36, 2048),
            (0, 0, 0),
        ] {
            evs.extend(ev(t, c, v));
        }
        evs
    }

    /// Runs the screen thread headless for `run`, with `open_touch`, and
    /// returns its report and how long it ran.
    fn run_screen(run: Duration, open_touch: Option<OpenTouch>) -> (ScreenReport, f64) {
        let (to_ui, rx) = std::sync::mpsc::sync_channel(64);
        let (tx, _from_ui) = std::sync::mpsc::sync_channel(64);
        let p = Player::new(Fonts::embedded(), SCREEN, false, None);
        let started = Instant::now();
        let h = spawn(p, None, None, open_touch, Some(to_ui.clone()), rx, tx).unwrap();
        std::thread::sleep(run);
        to_ui.send(ToUi::Quit).unwrap();
        let r = h.join().unwrap();
        (r, started.elapsed().as_secs_f64())
    }

    /// The screen thread left alone for a second with nothing to draw: the
    /// pages' clock keeps time, and the thread sleeps between ticks. Until
    /// 13 Nov it woke every 4 ms and, while nothing redrew, ticked on each
    /// of those passes by the time since the last frame drawn: in one idle
    /// second on the build workstation, 241 wakeups and 233 ticks that
    /// moved the pages' clock 22.8 s (a told line's 4 s went in about 0.2 s).
    #[test]
    fn the_idle_screen_thread_keeps_time_and_sleeps() {
        let (r, took) = run_screen(Duration::from_secs(1), None);
        assert!(
            r.ticked_s <= took + 0.01 && r.ticked_s >= took * 0.5,
            "{:.3} s ticked in {took:.3} s",
            r.ticked_s
        );
        // About 31 ticks (one every 33 ms), and a wakeup for each and for Quit.
        assert!((20..=35).contains(&r.ticks), "{} ticks", r.ticks);
        assert!(r.wakes <= 45, "{} wakeups", r.wakes);
    }

    /// A touchscreen with no doorbell (a stand-in has no device node for
    /// one): the screen thread still reads it every 4 ms.
    #[test]
    fn without_the_doorbell_the_screen_thread_reads_the_panel_every_4_ms() {
        let open: OpenTouch = Box::new(|| {
            TouchIn::from_source(
                Box::new(FakeTouch(VecDeque::from([Vec::new(), one_finger_report()]))),
                SCREEN,
            )
        });
        let (r, took) = run_screen(Duration::from_millis(500), Some(open));
        assert_eq!(r.touches, 1);
        assert!(
            r.wakes as f64 / took > 100.0,
            "{} wakeups in {took:.3} s",
            r.wakes
        );
        assert!(r.ticked_s <= took + 0.01, "{:.3} s ticked", r.ticked_s);
    }

    /// No page redraws while nothing happens, except the ones that show
    /// something live. Each idle redraw costs the screen thread a paint and
    /// a copy, 30 times a second on the MPC: the Pads tab's browser did
    /// that until 13 Nov, and the top bar redrew every second whether or
    /// not a chip changed (and, spanning the turned panel, rewrote nearly
    /// every row of it).
    #[test]
    fn no_page_redraws_while_idle() {
        // The sample screens outgrow the 2 MiB test thread in a debug build
        // (as above).
        std::thread::Builder::new()
            .stack_size(8 << 20)
            .spawn(idle_pages_stay_still)
            .unwrap()
            .join()
            .unwrap();
    }

    fn idle_pages_stay_still() {
        // Pages whose live parts redraw on their own, and why. (The I/O
        // view redraws only when it changes, since 13 Nov, and here
        // nothing does.)
        const LIVE: [&str; 2] = [
            // The touch test's trails fade.
            "debug-touch",
            // The Rec tab's meters fall back after the input stops (the
            // view redraws only when its picture changes, since 13 Nov).
            "sample-rec",
        ];
        let mut busy = Vec::new();
        for (name, mut p) in sample_screens(Fonts::embedded) {
            let (w, h) = p.ui.size();
            let mut canvas = SoftCanvas::new(w, h);
            // Settle: what the sample's set-up left to draw, the first
            // second's chips, and a line the set-up told on the hint line
            // (it goes back to the page's hint after TELL_SECONDS).
            let mut settle = 0;
            while settle < 40 || (p.told.is_some() && settle < 200) {
                p.tick(0.033);
                p.ui.paint(&mut canvas);
                settle += 1;
            }
            let mut frames = 0;
            // Three seconds of frames with nothing happening.
            for _ in 0..90 {
                p.tick(0.033);
                if p.ui.needs_paint() {
                    frames += 1;
                    p.ui.paint(&mut canvas);
                }
            }
            if LIVE.contains(&name) {
                assert!(
                    frames > 0,
                    "{name} shows something live: listed for nothing"
                );
            } else if frames > 0 {
                busy.push(format!("{name}: {frames} of 90 frames"));
            }
        }
        assert!(busy.is_empty(), "redrawn while idle: {busy:?}");
    }

    /// Memory by engine counts a sound two kits share once, and the rows
    /// land under the audio loop's rows on CPU by engine.
    #[test]
    fn memory_by_engine_counts_shared_sounds_once() {
        let mut p = sample_player(Fonts::embedded());
        let bb = boom_bap_project();
        let one = {
            let mut q = sample_player(Fonts::embedded());
            q.apply(ToUi::SampleKits {
                projects: vec![bb.clone()],
                rate: 44_100,
            });
            q.sample.memory()
        };
        assert!(one.count > 0 && one.sounds > 0, "{one:?}");
        // The same project in two kits: its sounds are the same buffers.
        p.apply(ToUi::SampleKits {
            projects: vec![bb.clone(), bb],
            rate: 44_100,
        });
        assert_eq!(p.sample.memory(), one);
        let rows = crate::memory::rows(&crate::memory::Held {
            samples: one,
            ..Default::default()
        });
        assert_eq!(
            rows[1].1,
            format!("{:.1} MB · {} sounds", one.sounds as f64 / 1e6, one.count)
        );
    }
}
