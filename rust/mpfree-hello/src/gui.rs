//! Hello MPC's screens, built from mpfree-ui widgets.
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

use crate::app::{tabs, ArpView, GuideView, MapView, MenuView, PadLevel, Settings, Status, View, WorkspaceView};
use crate::debug_page::{AudioNumbers, DebugPages};
use crate::perf::ProcView;
use crate::sample_page::{self, SamplePage};
use crate::sampler::{Playheads, ViewSwap};
use crate::touchlab_page::TouchLabView;
use mpfree_touch::mt::Phase;
use mpfree_touch::pointer::PointerEvent;
use mpfree_touch::profile::MPC_ONE;
use mpfree_touch::touchlab::reader_line;
use mpfree_touch_evdev::{EvdevFile, TouchReader, TouchSource};
use mpfree_arp::{Mode, Pattern};
use mpfree_arp_ui::view::{self as arp_view, ArpAction, ArpPanel, ArpState};
use mpfree_arp_ui::workspace::{self, Editor, PatternWorkspace, Row, WsAction};
use mpfree_time_ui::view::{self as time_view, TimePage, TimeState, TransportInfo, CLIPS_HINT, CLIPS_PAGE};
use mpfree_time_ui::TimeAction;
use mpfree_ui::fb::FbTarget;
use mpfree_ui::widgets::{Card, Chip, Fader, Header, Label, ListItem, ListView, Loading, LogPanel, OptionList,
                         OptionRow, PadGrid, StatList};
use mpfree_synth_ui::view as synth_view;
use mpfree_synth_ui::{SynthAction, SynthPanel, SynthState};
use mpfree_sampler::model::Project;
use mpfree_ui::{Align, Color, EventKind, Fonts, Image, SoftCanvas, Theme, Tone, Touch, Ui, UiEvent};
use std::collections::VecDeque;
use std::io::{self, Write};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub const CONSOLE_LAYOUT: &str = "col(header:60, log:*, hint:26)";
pub const PLAY_LAYOUT: &str = "col(header:60, row(pads:*, col(kits:*, level:96, timing:232):460):*, hint:26)";

/// A page's layout with the time engine's transport bar under the header
/// (the Play page and the arp's panel and quick band).
pub fn with_bar(layout: &str) -> String {
    time_view::with_transport(layout).unwrap_or_else(|| layout.to_string())
}
pub const SYSLOG_LAYOUT: &str = "col(header:60, syslog:*, hint:26)";
pub const OPTIONS_LAYOUT: &str = "col(header:60, row(options:*, help:470):*, hint:26)";
pub const GUIDE_LAYOUT: &str = "col(header:60, row(guide:*, found:400):*, hint:26)";

const CONSOLE_HINT: &str = "Switch the MPC off with its power switch to return to the stock firmware";
const SHIFT_HINT: &str =
    "SHIFT +   wheel: synth octave   ·   FULL LEVEL: Half Level   ·   NOTE REPEAT: Latch   ·   MENU: Prefs   ·   STOP: console";
const SHIFT_SYNTH_HINT: &str =
    "SHIFT +   wheel: page   ·   FULL LEVEL: Half Level   ·   NOTE REPEAT: Latch   ·   MENU: Prefs   ·   STOP: console";
const SHIFT_SAMPLE_HINT: &str =
    "SHIFT +   wheel: pad   ·   Q-Links: one frame a detent, no snap   ·   UNDO: redo   ·   MENU: Prefs   ·   STOP: console";
const SHIFT_MENU_HINT: &str = "SHIFT +   wheel: change the value   ·   MENU: Prefs   ·   STOP: console";
const OPTIONS_HINT: &str =
    "Wheel or tap: pick a row   ·   - / +, SHIFT + wheel or the arrows: change it   ·   MENU: close";
const GUIDE_HINT: &str = "STOP: skip this button   ·   The pads still play while the guide is open";
const LOADING_HINT: &str = "Tap the screen for the console";

/// The arp's pages: the panel beside the pads, the quick band over them,
/// and the pattern workspace (editing, and recording on the pads).
pub const ARP_PAGE: &str = "Arp";
pub const QUICK_PAGE: &str = "Play, NOTE REPEAT held";
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
        let mut header = Header::new(&["Hello MPC"]);
        header.chips.push(Chip::new(heading, Tone::Accent));
        ui.add("header", header);
        ui.add("log", LogPanel::new("Log", true, 400));
        ui.add("hint", Label::new(CONSOLE_HINT, Tone::Dim, Align::Center));
        ui.add_page("console", CONSOLE_LAYOUT).expect("console layout");
        Console { ui, canvas: SoftCanvas::new(w, h), fb }
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
            Some(fb) if !damage.is_empty() => fb.present(&self.canvas, &damage),
            _ => Ok(()),
        }
    }

    pub fn canvas(&self) -> &SoftCanvas {
        &self.canvas
    }
}

/// A colour for a log line from how it starts.
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

/// What the pad player shows about one kit.
#[derive(Clone, Debug)]
pub struct KitView {
    pub name: String,
    /// The sample on each pad (pad 1 first), if any.
    pub pads: Vec<Option<String>>,
    /// The kit's pad colour, 0-63 per channel as the controller takes it.
    pub colour: [u8; 3],
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

pub enum ToUi {
    /// A line for the terminal and the Events log.
    Line(String),
    /// A line for the Events log only (not printed).
    Event(String),
    /// A MIDI message in or out, as one line (also printed).
    Midi(String),
    /// A line of the system log, for the Console page (not printed).
    Console(String),
    /// Loading: what is happening, a second line, and the progress (0-1).
    Progress { message: String, detail: String, value: f32 },
    /// Loading stopped: what went wrong.
    Failed { message: String, detail: String },
    Kits(Vec<KitView>),
    PadDown { pad: usize, velocity: u8 },
    PadUp(usize),
    /// A repeat of a pad that is not held: light it briefly.
    PadFlash { pad: usize, velocity: u8 },
    Kit(usize),
    /// Level in dB and as 0.0-1.0 of the slider.
    Level { db: f32, value: f32 },
    Session(String, Tone),
    Timing(Timing),
    Show(View),
    Menu(MenuView),
    Guide(GuideView),
    Status(Status),
    Settings(Settings),
    ClearLogs,
    ClearConsole,
    /// The synth: its parameters, values and presets (at the start).
    Synth(SynthState),
    /// Every value of the synth, in its parameter order (after a preset).
    SynthValues(Vec<f32>),
    /// The synth's meters, by parameter index.
    SynthMeters(Vec<(usize, f32)>),
    /// On the Synth page: a Q-Link (0-3) turned by this many detents.
    SynthQLink(usize, i32),
    /// On the Synth page: the wheel turned (the selected knob).
    SynthWheel(i32),
    /// On the Synth page: the next or previous page.
    SynthPage(i32),
    /// The kits' sampler projects, for the Sample tab (once, after
    /// loading), and the rate to build their engine views for.
    SampleKits { projects: Vec<Project>, rate: u32 },
    /// The audio thread swapped in a kit view: the box holds the old one,
    /// to free here.
    SampleViewBack(Box<ViewSwap>),
    /// On the Sample page: a Q-Link (0-3) turned by this many detents.
    SampleQLink(usize, i32),
    /// On the Sample page: a Q-Link let go (the turn is one undo step).
    SampleLift,
    /// On the Sample page: the wheel (zoom).
    SampleWheel(i32),
    /// On the Sample page: the previous or next pad with a sound.
    SamplePad(i32),
    /// On the Sample page: UNDO (false) or redo (true).
    SampleUndo(bool),
    /// Where the playing kit's voices are (while the Sample page shows).
    SamplePlayheads(Playheads),
    /// New pad labels for one kit (the synth's notes, after an octave change).
    KitPads { kit: usize, pads: Vec<Option<String>> },
    /// Open the Touch lab page; any `Show` closes it again.
    TouchLab,
    /// What the arp screens show. (Patterns are large, so they travel
    /// boxed: the channel keeps a slot of the largest message per place.)
    Arp(Box<ArpView>),
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
    /// The data wheel on the Clips page: step the selected record setting,
    /// or scroll the grid.
    ClipsWheel { delta: i32, scroll: bool },
    /// The Map controls page (debug mode).
    Map(Box<MapView>),
    Quit,
}

#[derive(Clone, Debug, PartialEq)]
pub enum FromUi {
    Pad { pad: usize, down: bool },
    Kit(usize),
    /// Slider position, 0.0-1.0.
    Level(f32),
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
    /// A touch on the transport bar or the Clips page.
    Time(TimeAction),
    /// A change on the Synth page, for the synth on the audio thread.
    Synth(SynthAction),
    /// A kit's engine view, built again after an edit on the Sample page,
    /// for the audio thread to swap in (it sends the old one back).
    SampleView(Box<ViewSwap>),
    /// The Debug tab's strip: MIDI, Performance or Map controls.
    DebugPage(usize),
    /// A control tapped in the mapper's list (listen for it).
    MapRow(usize),
    /// The mapper card's buttons: Listen, Skip, Clear.
    MapButton(usize),
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
    screen_row: String,
    /// The timed `play` step (STOP ends it) rather than the app.
    timed: bool,
    /// The app has taken over the pages (after loading).
    ready: bool,
    view: View,
    status: Status,
    settings: Settings,
    session: (String, Tone),
    /// Lines for the Console page come from the system log (the app); in the
    /// timed step there is none, so the step's own lines go there.
    console_from_lines: bool,
    console_backlog: VecDeque<String>,
    /// The Synth page, once the synth's state has come.
    synth: Option<SynthPanel>,
    /// The Sample page and the kits' projects it edits.
    pub sample: SamplePage,
    /// The Touch lab page is on screen (over `view`, which it returns to).
    lab_open: bool,
    arp_panel: ArpPanel,
    pub workspace: PatternWorkspace,
    /// What the arp screens show, from the app.
    pub arp: ArpState,
    /// The Q-Link slider a finger is on, on the panel.
    qlink_focus: Option<usize>,
    /// Where the arp chip is in the header's chips.
    arp_chip: Option<usize>,
    /// Fingers on the screen: a page change lets go of them first, so a pad
    /// held on the screen never sticks.
    fingers: Vec<u64>,
    /// Actions that came from something other than a touch (a page change
    /// letting go of a pad, the wheel in the workspace), for the audio thread.
    outbox: Vec<FromUi>,
    /// The time engine's bar and Clips page, and what they show.
    time_page: TimePage,
    pub time: TimeState,
    /// The Debug tab's Performance and Map controls pages.
    debug: DebugPages,
}

impl Player {
    /// `backdrop`: the picture behind the loading card (the startup screen's
    /// last frame), or None for the built-in one.
    pub fn new(fonts: Fonts, size: (i32, i32), timed: bool, backdrop: Option<Arc<Image>>) -> Player {
        let mut ui = Ui::new(Theme::dark(), fonts, size.0, size.1);
        let names: Vec<&str> = tabs(false).iter().map(|v| v.name()).collect();
        ui.add("header", Header::new(&names));
        let mut loading = Loading::new("MPFree", "Pad player");
        loading.set_backdrop(backdrop);
        loading.hint = LOADING_HINT.to_string();
        loading.message = "Starting".to_string();
        ui.add("loading", loading);
        ui.add("pads", PadGrid::new(16, 4));
        ui.add("kits", ListView::new("Kits"));
        ui.add("level", Fader::new("Level · Q-Link 1"));
        ui.add("timing", StatList::new("Timing"));
        ui.add("midi", LogPanel::new("MIDI in and out", true, 300));
        ui.add("events", LogPanel::new("Events", false, 200));
        ui.add("syslog", LogPanel::new("Console", true, CONSOLE_BACKLOG));
        ui.add("options", OptionList::new("Options"));
        ui.add("help", Card::new("About this setting"));
        ui.add("guide", Card::new("Button guide"));
        ui.add("found", StatList::new("Buttons"));
        ui.add("hint", Label::new("", Tone::Dim, Align::Center));
        let time_page = TimePage::add_to(&mut ui).expect("clips layout");
        ui.add_full_page("Loading", "loading").expect("loading layout");
        ui.add_page("Play", &with_bar(PLAY_LAYOUT)).expect("play layout");
        let debug = DebugPages::add(&mut ui).expect("debug layouts");
        ui.add_page("Console", SYSLOG_LAYOUT).expect("console layout");
        ui.add_page("Prefs", OPTIONS_LAYOUT).expect("prefs layout");
        ui.add_page("Options", OPTIONS_LAYOUT).expect("options layout");
        ui.add_page("Guide", GUIDE_LAYOUT).expect("guide layout");
        let patterns = crate::app::built_in_patterns();
        let pattern = patterns.first().map(|(_, p)| *p).unwrap_or_default();
        let name = patterns.first().map(|(n, _)| n.clone()).unwrap_or_default();
        let arp = ArpState { settings: mpfree_arp::Settings::default(), pattern, patterns, pattern_index: 0,
                             pattern_changed: false, notes: Vec::new(), step: 0, latched: false, bpm: 120.0 };
        let mut arp_panel = ArpPanel::add(&mut ui, ARP_PAGE, QUICK_PAGE, &arp).expect("arp layouts");
        arp_panel.set_layout_wrap(&mut ui, with_bar).expect("arp layouts with the bar");
        let workspace = PatternWorkspace::add(&mut ui, PATTERN_PAGE, RECORD_PAGE, &name, pattern)
            .expect("workspace layouts");
        ui.add("touchlab", TouchLabView::new());
        ui.add_full_page("Touch lab", "touchlab").expect("touch lab layout");
        let sample = SamplePage::add(&mut ui).expect("sample layout");
        let _ = ui.show("Loading");
        let status = Status { shift: false, pad_level: PadLevel::Off, repeat: false, latched: false, rate: "1/16",
                              bpm: 120.0 };
        let mut p = Player { ui, kits: Vec::new(), kit: 0, midi: 0, midi_held_back: 0,
                             timing: Timing { last_ms: f32::NAN, ..Timing::default() }, screen_row: "-".to_string(),
                             timed, ready: false, view: View::Play, status, settings: Settings::default(),
                             session: ("Starting".to_string(), Tone::Dim),
                             console_from_lines: timed, console_backlog: VecDeque::new(), lab_open: false,
                             synth: None, sample, arp_panel, workspace, arp, qlink_focus: None, arp_chip: None,
                             fingers: Vec::new(), outbox: Vec::new(), time_page, time: TimeState::default(), debug };
        p.time_page.update(&mut p.ui, &p.time);
        p.show_timing();
        p.refresh_chips();
        p.refresh_hint();
        p
    }

    /// The page on screen ("Loading" until the app is ready).
    #[cfg(test)]
    pub fn page(&self) -> &str {
        self.ui.page()
    }

    fn set_kits(&mut self, kits: Vec<KitView>) {
        if let Some(l) = self.ui.get_mut::<ListView>("kits") {
            l.items = kits.iter().map(|k| ListItem {
                text: k.name.clone(),
                detail: format!("{} pads", k.pads.iter().filter(|p| p.is_some()).count()),
                color: Some(Color::from_pad(k.colour)),
            }).collect();
        }
        self.kits = kits;
        self.set_kit(0);
    }

    fn set_kit(&mut self, k: usize) {
        if k >= self.kits.len() {
            return;
        }
        self.kit = k;
        if let Some(g) = self.ui.get_mut::<PadGrid>("pads") {
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
    }

    /// The pads' names: the kit's sounds, or what each pad writes while a
    /// pattern is recorded on the pads.
    fn label_pads(&mut self) {
        let recording = self.view == View::Pattern && self.workspace.recording;
        let kit = self.kits.get(self.kit).cloned();
        let Some(g) = self.ui.get_mut::<PadGrid>("pads") else { return };
        if recording {
            let (notes, marks) = (Color::from_pad([0x00, 0x22, 0x3F]), Color::from_pad([0x3F, 0x22, 0x00]));
            for (i, (pad, label)) in g.pads.iter_mut().zip(workspace::record_pad_labels()).enumerate() {
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
        let mut chips = vec![Chip::new(self.session.0.clone(), self.session.1)];
        match self.kits.get(self.kit) {
            Some(k) if k.name == crate::synth::KIT_NAME => {
                let note = |i: usize| k.pads.get(i).cloned().flatten().unwrap_or_default();
                chips.push(Chip::new(format!("Poly · {} to {}", note(0), note(15)), Tone::Accent));
            }
            Some(_) => chips.push(Chip::new(format!("Kit {} of {} · Bank {}", self.kit + 1, self.kits.len(),
                                                    crate::app::bank_letter(self.kit)), Tone::Accent)),
            None => {}
        }
        // The level in dB is on the slider and the tempo only matters while
        // the arp plays, so the header keeps to the chips that change how the
        // pads play: four tabs leave room for about five chips. The arp chip
        // is always there, so a tap on it can open the panel.
        let mut arp = arp_view::chip(&self.arp);
        if self.arp.settings.mode != Mode::Direct {
            arp.text = format!("{} · {:.0} BPM", arp.text, self.arp.bpm);
        }
        self.arp_chip = Some(chips.len());
        chips.push(arp);
        let s = &self.status;
        match s.pad_level {
            PadLevel::Full => chips.push(Chip::new("Full level", Tone::Warn)),
            PadLevel::Half => chips.push(Chip::new("Half level", Tone::Warn)),
            PadLevel::Off => {}
        }
        if let Some(left) = self.timing.seconds_left {
            chips.push(Chip::new(format!("{left} s left"), Tone::Dim));
        }
        if let Some(h) = self.ui.get_mut::<Header>("header") {
            if h.chips != chips {
                h.chips = chips;
            }
        }
    }

    fn refresh_hint(&mut self) {
        let stop = if self.timed { "STOP: end" } else { "STOP: silence" };
        let mode = self.arp.settings.mode;
        let wheel = match mode {
            Mode::Arp => "order",
            Mode::Pattern => "pattern",
            Mode::Direct | Mode::Repeat => "rate",
        };
        let text = if self.status.shift && self.ready && matches!(self.view, View::Prefs | View::Options) {
            SHIFT_MENU_HINT.to_string()
        } else if self.status.shift && self.ready && self.view == View::Synth {
            SHIFT_SYNTH_HINT.to_string()
        } else if self.status.shift && self.ready && self.view == View::Sample {
            SHIFT_SAMPLE_HINT.to_string()
        } else if self.status.shift && self.ready {
            SHIFT_HINT.to_string()
        } else if self.view == View::Play && self.band_showing() && self.status.latched {
            format!("Latched: {}   ·   NOTE REPEAT: let go   ·   hold NOTE REPEAT for the wheel, - / + and Q-Links   ·   \
                     {stop}", crate::app::mode_label(mode))
        } else if self.view == View::Play && self.band_showing() {
            format!("Wheel: {wheel}   ·   - / +: Repeat, Arp, Pattern   ·   Q-Links: rate, gate, swing, octaves   ·   \
                     + MENU: Arp panel")
        } else {
            match self.view {
                View::Play => format!("Wheel or tap: kit   ·   Q-Link 1 or slider: level   ·   NOTE REPEAT: repeat   ·   \
                                       MENU: options   ·   {stop}"),
                View::Clips => CLIPS_HINT.to_string(),
                View::Synth => format!("Pads play Poly   ·   Q-Links 1-4: the marked knobs   ·   Wheel: the selected knob   ·   \
                                        - / +: page   ·   {stop}"),
                View::Sample if self.sample.page() == sample_page::PAGE => {
                    "Hit a pad to edit it   ·   Q-Links 1-4: Start, End, loop   ·   Wheel: zoom   ·   - / +: pad   ·   \
                     UNDO: undo   ·   tap the wave: play".to_string()
                }
                View::Sample => format!("Hit a pad with a sound to edit it   ·   - / +: next pad with a sound   ·   {stop}"),
                View::Debug => format!("Every MIDI message in and out, newest at the bottom   ·   SHIFT + wheel or - / +: \
                                        Debug page   ·   MENU: options   ·   {stop}"),
                View::Perf => format!("CPU per thread and memory every 2 s, audio every 0.25 s; a perf: line goes to \
                                       the log every 30 s   ·   SHIFT + wheel: Debug page   ·   {stop}"),
                View::Map => "Tap a control, then press it   ·   Wheel: pick   ·   +: listen   ·   -: skip   ·   \
                              STOP: stop listening   ·   SHIFT + wheel: Debug page".to_string(),
                View::Console => "The system log, as it comes   ·   MENU: pause or clear   ·   SHIFT + STOP: back"
                    .to_string(),
                View::Prefs | View::Options => OPTIONS_HINT.to_string(),
                View::Guide => GUIDE_HINT.to_string(),
                View::Arp => format!("Wheel: {wheel}   ·   - / +: mode   ·   Q-Links: rate, gate, swing, octaves   ·   \
                                      MENU: options   ·   NOTE REPEAT + MENU: close"),
                View::Pattern => workspace::hint(self.workspace.recording).to_string(),
            }
        };
        if let Some(h) = self.ui.get_mut::<Label>("hint") {
            if h.text != text {
                h.text = text;
            }
        }
    }

    fn show_timing(&mut self) {
        let t = self.timing;
        let ms = |v: f32| if v.is_finite() { format!("{v:.1} ms") } else { "no hit yet".to_string() };
        let rows = vec![
            ("Pad to sound, last".to_string(), ms(t.last_ms), Tone::Normal),
            ("Mean · range".to_string(),
             if t.mean_ms.is_finite() && t.hits > 0 { format!("{:.1} · {:.1} to {:.1} ms", t.mean_ms, t.min_ms, t.max_ms) }
             else { "-".to_string() }, Tone::Normal),
            ("Underruns".to_string(), format!("{} in {} periods", t.underruns, t.periods),
             if t.underruns == 0 { Tone::Good } else { Tone::Bad }),
            ("Hits · voices · peak".to_string(), format!("{} · {} · {:.0} dBFS", t.hits, t.voices, t.peak_db), Tone::Normal),
            ("Audio buffer".to_string(), format!("{} periods, {:.1} ms", t.periods_queued,
                                                 t.periods_queued as f32 * 128.0 / 44.1), Tone::Dim),
            ("Screen".to_string(), self.screen_row.clone(), Tone::Dim),
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
        self.settings = s;
        if retab {
            let debug = self.settings.debug_mode;
            let active = self.view.tab_index(debug);
            if let Some(h) = self.ui.get_mut::<Header>("header") {
                h.tabs = tabs(debug).iter().map(|v| v.name().to_string()).collect();
                if let Some(t) = active {
                    h.active = t;
                }
            }
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
                m.push(&format!("({n} messages while paused, not shown)"), Tone::Dim);
            }
        }
    }

    pub fn apply(&mut self, msg: ToUi) {
        match msg {
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
                    m.push(&l, if l.contains("->") { Tone::Accent } else { Tone::Normal });
                    m.right = format!("{n} messages");
                }
            }
            ToUi::Console(l) => self.push_console(&l),
            ToUi::Progress { message, detail, value } => {
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
                self.sample.set_names(kits.iter().map(|k| k.name.clone()).collect());
                self.set_kits(kits);
                self.sample_changed();
            }
            ToUi::PadDown { pad, velocity } => {
                if let Some(g) = self.ui.get_mut::<PadGrid>("pads") {
                    g.hit(pad, velocity);
                }
                // The Sample page edits the pad hit last.
                self.sample.hit(pad);
                self.sample_changed();
            }
            ToUi::PadUp(pad) => {
                if let Some(g) = self.ui.get_mut::<PadGrid>("pads") {
                    g.release(pad);
                }
            }
            ToUi::PadFlash { pad, velocity } => {
                if let Some(g) = self.ui.get_mut::<PadGrid>("pads") {
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
            ToUi::Level { db, value } => {
                if let Some(f) = self.ui.get_mut::<Fader>("level") {
                    f.value = value;
                    f.text = format!("{db:.1} dB");
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
            ToUi::Show(v) => {
                self.ready = true;
                self.show_view(v);
            }
            ToUi::Menu(m) => {
                if let Some(o) = self.ui.get_mut::<OptionList>("options") {
                    o.title = m.title;
                    o.rows = m.rows.into_iter().map(|(name, value)| OptionRow { name, value }).collect();
                    o.selected = m.selected;
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
                    s.set(g.found.into_iter().map(|(k, v)| {
                        let tone = if v == "now" { Tone::Accent } else if v.starts_with("button") { Tone::Good }
                                   else { Tone::Dim };
                        (k, v, tone)
                    }).collect());
                }
            }
            ToUi::Status(s) => {
                self.status = s;
                if self.ready && self.view == View::Play {
                    // The quick band comes and goes with NOTE REPEAT.
                    self.show_view(View::Play);
                }
                self.refresh_chips();
                self.refresh_hint();
            }
            ToUi::Settings(s) => self.set_settings(s),
            ToUi::ClearLogs => {
                for name in ["midi", "events"] {
                    let title = self.ui.get::<LogPanel>(name).map(|l| l.title.clone()).unwrap_or_default();
                    self.ui.add(name, LogPanel::new(&title, name == "midi", if name == "midi" { 300 } else { 200 }));
                }
                self.midi_held_back = 0;
            }
            ToUi::ClearConsole => {
                self.ui.add("syslog", LogPanel::new("Console", true, CONSOLE_BACKLOG));
                self.console_backlog.clear();
            }
            ToUi::Synth(state) => match self.synth.as_mut() {
                Some(p) => {
                    let _ = p.load(&mut self.ui, state);
                }
                None => match SynthPanel::add(&mut self.ui, "Synth", state) {
                    Ok(p) => self.synth = Some(p),
                    Err(e) => println!("screen: no Synth page ({e})"),
                },
            },
            ToUi::SynthValues(values) => {
                if let Some(p) = self.synth.as_mut().filter(|p| p.state.values.len() == values.len()) {
                    p.state.values = values;
                    p.refresh(&mut self.ui);
                }
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
            ToUi::SynthQLink(n, ticks) => {
                if let Some(a) = self.synth.as_mut().and_then(|p| p.qlink(&mut self.ui, n, ticks)) {
                    self.outbox.push(FromUi::Synth(a));
                }
            }
            ToUi::SynthWheel(ticks) => {
                if let Some(a) = self.synth.as_mut().and_then(|p| p.wheel(&mut self.ui, ticks)) {
                    self.outbox.push(FromUi::Synth(a));
                }
            }
            ToUi::SynthPage(delta) => {
                if let Some(p) = self.synth.as_mut() {
                    p.step_page(&mut self.ui, delta);
                }
            }
            ToUi::SampleKits { projects, rate } => {
                self.sample.set_kits(&mut self.ui, projects, rate);
                self.sample_changed();
            }
            ToUi::SampleViewBack(swap) => self.sample.view_back(swap),
            ToUi::SampleQLink(k, ticks) => {
                if self.view == View::Sample {
                    self.sample.qlink(&mut self.ui, k, ticks, self.status.shift);
                }
            }
            ToUi::SampleLift => self.sample.lift(),
            ToUi::SampleWheel(d) => self.sample.wheel(&mut self.ui, d),
            ToUi::SamplePad(d) => {
                self.sample.step_pad(d);
                self.sample_changed();
            }
            ToUi::SampleUndo(redo) => self.sample.undo(&mut self.ui, redo),
            ToUi::SamplePlayheads(h) => {
                if self.view == View::Sample {
                    self.sample.set_playheads(&mut self.ui, &h);
                }
            }
            ToUi::KitPads { kit, pads } => {
                if let Some(k) = self.kits.get_mut(kit) {
                    k.pads = pads;
                    if kit == self.kit {
                        self.set_kit(kit);
                    }
                }
            }
            ToUi::TouchLab => {
                self.ready = true;
                if let Some(lab) = self.ui.get_mut::<TouchLabView>("touchlab") {
                    lab.reset();
                }
                self.switch_page("Touch lab");
                self.lab_open = true;
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
            ToUi::Time(st) => {
                self.time = *st;
                self.time_page.update(&mut self.ui, &self.time);
            }
            ToUi::Transport(info) => {
                self.time.transport = info;
                self.time_page.update(&mut self.ui, &self.time);
            }
            ToUi::ClipsWheel { delta, scroll: true } => {
                self.time_page.scroll(&self.ui, delta);
                self.time_page.update(&mut self.ui, &self.time);
            }
            ToUi::ClipsWheel { delta, scroll: false } => {
                // The row last touched (the armed track to begin with).
                let index = self.ui.get::<OptionList>(time_view::RECORD).map_or(0, |l| l.selected);
                let ev = UiEvent { widget: time_view::RECORD.to_string(), kind: EventKind::Step { index, delta } };
                if let Some(a) = time_view::action(&ev, &self.time) {
                    self.outbox.push(FromUi::Time(a));
                }
            }
            ToUi::Quit => {}
        }
        self.print_sample_lines();
    }

    /// The Sample page's kit, pad or project changed: show it again if it
    /// is on screen (it may be the page for a pad without a sound now).
    fn sample_changed(&mut self) {
        if self.ready && self.view == View::Sample && !self.lab_open {
            self.show_view(View::Sample);
        }
    }

    /// The Sample page's log lines, to the terminal (the system log) and
    /// the Events log.
    fn print_sample_lines(&mut self) {
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
        if let FromUi::SampleView(swap) = a {
            self.sample.unsent(swap.kit);
        }
    }

    /// Close the Touch lab if it is open (a step in progress is logged).
    fn close_lab(&mut self) {
        if !self.lab_open {
            return;
        }
        self.lab_open = false;
        if let Some(lab) = self.ui.get_mut::<TouchLabView>("touchlab") {
            lab.leave();
        }
        self.print_lab_lines();
    }

    fn print_lab_lines(&mut self) {
        if !self.ui.get::<TouchLabView>("touchlab").is_some_and(|l| l.has_lines()) {
            return;
        }
        let lines = self.ui.get_mut::<TouchLabView>("touchlab").map(|l| l.take_lines()).unwrap_or_default();
        for l in lines {
            println!("{l}");
            if let Some(e) = self.ui.get_mut::<LogPanel>("events") {
                e.push(&l, Tone::Dim);
            }
        }
        let _ = io::stdout().flush();
    }

    /// True while the Touch lab page is on screen.
    #[cfg(test)]
    pub fn lab_open(&self) -> bool {
        self.lab_open
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
        if self.lab_open {
            if let Some(lab) = self.ui.get_mut::<TouchLabView>("touchlab") {
                lab.set_event_time(time_us);
            }
        }
    }

    /// The quick band shows over the Play page while NOTE REPEAT is held or
    /// pads are latched.
    fn band_showing(&self) -> bool {
        self.status.repeat
    }

    /// The pattern step the engine is on, while a pattern plays.
    fn playing_step(&self) -> Option<usize> {
        let a = &self.arp;
        (a.settings.mode == Mode::Pattern && !a.notes.is_empty() && a.step > 0 && !a.pattern.steps().is_empty())
            .then(|| ((a.step - 1) as usize) % a.pattern.len())
    }

    /// Show the arp state on the panel, the band, the workspace and the chip.
    fn refresh_arp(&mut self) {
        let _ = self.arp_panel.update(&mut self.ui, &self.arp);
        let playing = self.playing_step();
        self.workspace.update(&mut self.ui, playing);
        self.refresh_chips();
        self.refresh_hint();
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
        std::mem::take(&mut self.outbox)
    }

    fn show_view(&mut self, v: View) {
        self.close_lab();
        let page = match v {
            View::Play if self.band_showing() && self.ready => QUICK_PAGE.to_string(),
            View::Play => "Play".to_string(),
            View::Clips => CLIPS_PAGE.to_string(),
            View::Synth => "Synth".to_string(),
            View::Sample => {
                self.sample.refresh_if_stale(&mut self.ui);
                self.sample.page().to_string()
            }
            View::Debug | View::Perf | View::Map => {
                DebugPages::set_strip(&mut self.ui, v);
                DebugPages::page(v).unwrap_or("Debug").to_string()
            }
            View::Console => "Console".to_string(),
            View::Prefs => "Prefs".to_string(),
            View::Options => "Options".to_string(),
            View::Guide => "Guide".to_string(),
            View::Arp => ARP_PAGE.to_string(),
            View::Pattern => self.workspace.page().to_string(),
        };
        let relabel = (self.view == View::Pattern) != (v == View::Pattern);
        if self.view == View::Sample && v != View::Sample {
            // A pad a tap on the waveform holds is let go.
            self.sample.leave(&mut self.outbox);
        }
        self.switch_page(&page);
        self.view = v;
        if v == View::Clips {
            // How many slots fit is known once the grid is on screen.
            self.time_page.update(&mut self.ui, &self.time);
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
        let tab = v.tab_index(self.settings.debug_mode);
        if let (Some(h), Some(t)) = (self.ui.get_mut::<Header>("header"), tab) {
            h.active = t;
        }
        self.refresh_hint();
    }

    /// Debug mode is on (the screen thread then reads /proc for the
    /// Performance page and the `perf:` lines).
    pub fn debug_mode(&self) -> bool {
        self.settings.debug_mode
    }

    /// Process and system numbers, from the screen thread's sampler.
    pub fn set_proc(&mut self, v: ProcView) {
        self.debug.set_proc(&mut self.ui, v);
    }

    /// A touch: what it means for the app, if anything. Before the app is
    /// ready, the pages are switched here (loading and console only).
    pub fn touch(&mut self, finger: u64, phase: Touch, x: i32, y: i32) -> Option<FromUi> {
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
                } else if matches!(ev, Some(UiEvent { kind: EventKind::Tab(_), .. })) {
                    let _ = self.ui.show("Loading");
                }
            }
            return None;
        }
        if self.lab_open {
            // The lab page takes every finger and means nothing to the app.
            let _ = self.ui.touch(finger, phase, x, y);
            self.print_lab_lines();
            if self.ui.get::<TouchLabView>("touchlab").is_some_and(|l| l.wants_exit()) {
                self.show_view(self.view);
            }
            return None;
        }
        let action = self.ui.touch(finger, phase, x, y).and_then(|ev| self.on_event(ev));
        if matches!(phase, Touch::Up | Touch::Cancel) && self.view == View::Sample {
            // A drag on the Sample page ends with its finger: one undo step.
            self.sample.lift();
            self.print_sample_lines();
        }
        action
    }

    /// A touch meant something: hand it to the audio thread, which decides.
    pub fn on_event(&mut self, ev: UiEvent) -> Option<FromUi> {
        let synth_widget = [synth_view::TITLE, synth_view::VISUALS, synth_view::TILES, synth_view::PRESETS,
                            synth_view::QLINKS].contains(&ev.widget.as_str());
        if synth_widget {
            let p = self.synth.as_mut()?;
            return p.event(&mut self.ui, &ev).map(FromUi::Synth);
        }
        if ev.widget.starts_with(sample_page::PREFIX) {
            self.sample.event(&mut self.ui, &ev, &mut self.outbox);
            self.print_sample_lines();
            return None;
        }
        match (ev.widget.as_str(), ev.kind.clone()) {
            ("header", EventKind::Tab(i)) => {
                // The header moved its own highlight; the page follows when the app says.
                let tab = self.view.tab_index(self.settings.debug_mode);
                if let (Some(h), Some(t)) = (self.ui.get_mut::<Header>("header"), tab) {
                    h.active = t;
                }
                Some(FromUi::Tab(i))
            }
            (crate::debug_page::STRIP, EventKind::Tab(i)) => {
                DebugPages::set_strip(&mut self.ui, self.view);
                Some(FromUi::DebugPage(i))
            }
            ("controls", EventKind::Select(i)) => Some(FromUi::MapRow(i)),
            ("mapcard", EventKind::Tab(i)) => Some(FromUi::MapButton(i)),
            ("pads", EventKind::Pad { index, down }) => Some(FromUi::Pad { pad: index, down }),
            ("kits", EventKind::Select(k)) => Some(FromUi::Kit(k)),
            ("level", EventKind::Value(v)) => Some(FromUi::Level(v)),
            ("options", EventKind::Select(i)) => Some(FromUi::MenuSelect(i)),
            ("options", EventKind::Step { index, delta }) => Some(FromUi::MenuStep(index, delta)),
            ("guide", EventKind::Tab(i)) => Some(FromUi::GuideButton(i)),
            ("header", EventKind::Select(i)) if Some(i) == self.arp_chip => Some(FromUi::ArpChip),
            (w, _) if w.starts_with("arp-ws-") => {
                // The editor changes at once; the app hears the new pattern.
                let a = self.workspace.action(&ev);
                let playing = self.playing_step();
                self.workspace.update(&mut self.ui, playing);
                a.map(|a| FromUi::Workspace(Box::new(a)))
            }
            (w, _) if w.starts_with("arp-") => self.arp_panel.action(&ev, &self.arp, &mut self.qlink_focus).map(FromUi::Arp),
            (time_view::TRANSPORT | time_view::CLIPS | time_view::RECORD | time_view::TAKES, _) => {
                time_view::action(&ev, &self.time).map(FromUi::Time)
            }
            _ => None,
        }
    }

    /// Advance animations by `dt` seconds.
    pub fn tick(&mut self, dt: f32) {
        if self.ui.get::<PadGrid>("pads").is_some_and(|g| g.fading()) {
            if let Some(g) = self.ui.get_mut::<PadGrid>("pads") {
                g.fade(dt);
            }
        }
        if self.lab_open && self.ui.get::<TouchLabView>("touchlab").is_some_and(mpfree_ui::Widget::animating) {
            if let Some(lab) = self.ui.get_mut::<TouchLabView>("touchlab") {
                lab.tick();
            }
            self.print_lab_lines();
        }
    }

    pub fn set_screen_row(&mut self, text: String) {
        self.screen_row = text;
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
/// checks against docs/hardware/data/mpc-one-touch-profile.json.
pub struct TouchIn {
    reader: TouchReader<Box<dyn TouchSource + Send>>,
    events: Vec<PointerEvent>,
    pub contacts: u64,
}

impl TouchIn {
    pub fn open(path: &str, size: (i32, i32)) -> Result<TouchIn, String> {
        let dev = EvdevFile::open(path).map_err(|e| format!("{path}: {e}"))?;
        dev.check_profile(&MPC_ONE).map_err(|e| format!("{path}: {e}"))?;
        TouchIn::from_source(Box::new(dev), size)
    }

    fn from_source(source: Box<dyn TouchSource + Send>, size: (i32, i32)) -> Result<TouchIn, String> {
        let reader = TouchReader::new(source, &MPC_ONE, size.0 as u32, size.1 as u32).map_err(|e| e.to_string())?;
        Ok(TouchIn { reader, events: Vec::new(), contacts: 0 })
    }

    /// Raw input events read so far.
    pub fn raw_events(&self) -> u64 {
        self.reader.stats().raw_events
    }

    /// The reader's state as one journal line (`touch-reader: ...`).
    pub fn status_line(&self) -> String {
        let s = self.reader.stats();
        let mut line = reader_line(s.seeded, s.slot_at_open, s.fingers_at_open, s.resyncs, s.resync_fallbacks,
                                   s.drained_events, s.contacts, s.raw_events);
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
        Ok(self.events.iter().map(|p| {
            let phase = match p.phase {
                Phase::Down => Touch::Down,
                Phase::Move => Touch::Move,
                Phase::Up => Touch::Up,
                Phase::Cancel => Touch::Cancel,
            };
            (p.id.0, phase, p.x.round() as i32, p.y.round() as i32, p.time_us)
        }).collect())
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

// ---------------------------------------------------------------- the screen thread

/// What the screen thread did, for the step's report.
#[derive(Clone, Debug, Default)]
pub struct ScreenReport {
    pub frames: u64,
    pub mean_ms: f64,
    pub max_ms: f64,
    pub touches: u64,
    pub error: Option<String>,
}

/// Run the pad player's screen until `Quit` arrives or the sender goes away.
/// Spawn it before the audio thread raises its own priority, so it stays at
/// normal priority.
///
/// `open_touch` opens the touchscreen. It is tried at once and then every
/// second while there is none (the driver may not have probed yet at power-on,
/// or a read failed), so touch comes back by itself.
pub fn spawn(mut player: Player, mut fb: Option<FbTarget>,
             mut open_touch: Option<OpenTouch>, rx: Receiver<ToUi>,
             tx: SyncSender<FromUi>) -> io::Result<JoinHandle<ScreenReport>> {
    thread::Builder::new().name("mpfree-screen".into()).spawn(move || {
        let mut touch: Option<TouchIn> = None;
        let mut touch_tried: Option<Instant> = None;
        let mut touch_error = String::new();
        let mut touch_total = 0u64;
        let mut touch_status = (Instant::now(), 0u64);
        let (w, h) = player.ui.size();
        let mut canvas = SoftCanvas::new(w, h);
        let mut report = ScreenReport::default();
        let mut total_ms = 0.0;
        let mut last_frame = Instant::now() - Duration::from_secs(1);
        let mut window = (Instant::now(), 0u32, 0f64);
        // Debug mode: /proc and /sys every 2 s, and a perf: line every 30 s.
        let mut perf = crate::perf::Sampler::new();
        let (mut perf_at, mut perf_logged): (Option<Instant>, Option<Instant>) = (None, None);
        let mut quit = false;
        while !quit {
            match rx.recv_timeout(Duration::from_millis(4)) {
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
                            player.set_touch_status(&t.status_line());
                            touch_status = (Instant::now(), t.raw_events());
                            touch = Some(t);
                        }
                        // Say why once, and again only if the reason changes.
                        Err(e) if first || e != touch_error => {
                            println!("touchscreen: not available yet ({e}); trying every second");
                            player.set_touch_status(&format!("touchscreen: not available yet ({e})"));
                            touch_error = e;
                        }
                        Err(_) => {}
                    }
                }
            }
            if let Some(t) = touch.as_mut() {
                match t.read() {
                    Ok(list) => {
                        for (finger, phase, x, y, time_us) in list {
                            player.set_touch_time(time_us);
                            if let Some(a) = player.touch(finger, phase, x, y) {
                                let _ = tx.try_send(a);
                            }
                        }
                        report.touches = touch_total + t.contacts;
                        if touch_status.0.elapsed() >= TOUCH_STATUS_EVERY {
                            if t.raw_events() != touch_status.1 {
                                println!("{}", t.status_line());
                            }
                            player.set_touch_status(&t.status_line());
                            touch_status = (Instant::now(), t.raw_events());
                        }
                    }
                    Err(e) => {
                        println!("touch: {e} (reopening)");
                        println!("{}", t.status_line());
                        player.set_touch_status(&format!("touch: {e} (reopening); {}", t.status_line()));
                        touch_total += t.contacts;
                        touch = None;
                    }
                }
            }
            send_actions(&mut player, &tx);
            if !player.debug_mode() {
                perf_at = None;
            } else if perf_at.is_none_or(|t| t.elapsed() >= PERF_EVERY) {
                perf_at = Some(Instant::now());
                let v = perf.sample();
                if v.over_s > 0.0 && perf_logged.is_none_or(|t| t.elapsed() >= PERF_LINE_EVERY) {
                    perf_logged = Some(Instant::now());
                    println!("{}", v.line());
                }
                player.set_proc(v);
            }
            let since = last_frame.elapsed();
            if since < Duration::from_millis(33) && !quit {
                continue;
            }
            player.tick(since.as_secs_f32().min(0.1));
            if !player.ui.needs_paint() {
                continue;
            }
            let t0 = Instant::now();
            let damage = player.ui.paint(&mut canvas);
            if let Some(f) = fb.as_mut() {
                if let Err(e) = f.present(&canvas, &damage) {
                    println!("screen: {e} (drawing stopped)");
                    report.error = Some(e);
                    fb = None;
                }
            }
            let ms = t0.elapsed().as_secs_f64() * 1e3;
            last_frame = Instant::now();
            report.frames += 1;
            total_ms += ms;
            report.max_ms = report.max_ms.max(ms);
            window.1 += 1;
            window.2 = window.2.max(ms);
            if window.0.elapsed() >= Duration::from_secs(1) {
                let fps = window.1 as f64 / window.0.elapsed().as_secs_f64();
                player.set_screen_row(format!("{fps:.0} fps, slowest frame {:.1} ms", window.2));
                window = (Instant::now(), 0, 0.0);
            }
        }
        report.mean_ms = if report.frames > 0 { total_ms / report.frames as f64 } else { 0.0 };
        report
    })
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
        _ => {}
    }
    player.apply(m);
    false
}

// ---------------------------------------------------------------- previews

/// Sample kits for previews and tests (made-up names, no Akai content).
pub fn sample_kits() -> Vec<KitView> {
    let names = ["Kick", "Snare", "Closed Hat", "Open Hat", "Clap", "Rim", "Tom Low", "Tom Hi", "Crash", "Ride",
                 "Shaker", "Perc 1", "Perc 2", "", "Sub Hit", ""];
    let kit = |name: &str, colour: [u8; 3], n: usize| KitView {
        name: name.to_string(),
        pads: names.iter().take(n).map(|s| if s.is_empty() { None } else { Some(s.to_string()) })
            .chain(std::iter::repeat(None)).take(16).collect(),
        colour,
    };
    vec![kit("Classic House", [0x3F, 0x22, 0x00], 16), kit("Boom Bap", [0x00, 0x2A, 0x3F], 12),
         kit("Trap", [0x2C, 0x00, 0x3F], 16), kit("Lo-Fi", [0x04, 0x3F, 0x00], 9),
         kit("Techno", [0x3F, 0x00, 0x00], 16), kit("Afrobeat", [0x3B, 0x3B, 0x09], 14)]
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
    p.apply(ToUi::Level { db: -18.0, value: 0.79 });
    p.apply(ToUi::Status(Status { shift: false, pad_level: PadLevel::Full, repeat: false, latched: false,
                                  rate: "1/16", bpm: 120.0 }));
    for (pad, v) in [(0usize, 112u8), (5, 87), (10, 64)] {
        p.apply(ToUi::PadDown { pad, velocity: v });
    }
    p.apply(ToUi::PadUp(10));
    if let Some(g) = p.ui.get_mut::<PadGrid>("pads") {
        g.pads[10].level = 0.5;
    }
    p.apply(ToUi::Timing(Timing { hits: 42, periods: 10_335, underruns: 0, voices: 3, peak_db: -9.0, last_ms: 7.9,
                                  min_ms: 6.2, mean_ms: 7.8, max_ms: 9.6, seconds_left: None, periods_queued: 3,
                                  audio: AudioNumbers { render_mean_us: 310.0, render_max_us: 540.0, synth_mean_us: 0.0,
                                                        synth_max_us: 0.0, worst_us: 905.0, steals: 0, dropped: 0,
                                                        peak_grains: 0 } }));
    p.set_screen_row("30 fps, slowest frame 6.4 ms".into());
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
    for l in ["pad player: 6 kits, 81 sounds", "session accepted; sending Akai's setup exactly as the app did",
              "opening /dev/snd/pcmC1D0p (ACVA)", "SCHED_FIFO priority 70", "kit 2: Boom Bap (12 pads)",
              "kit 1: Classic House (16 pads)", "pad level: Full Level (every hit 127)"] {
        p.apply(ToUi::Line(l.into()));
    }
    for l in ["[    2.104] systemd[1]: Started MPFree test boot: copy logs to the USB stick's log partition.",
              "[    2.230] systemd[1]: Starting MPFree test boot: the pad player...",
              "[    3.871] mpfree-hello[211]: loading drum kits: 8 kits, 39.0 MB",
              "[    4.402] kernel: rk_gmac-dwmac ff290000.ethernet eth0: Link is Up - 1Gbps/Full",
              "[    5.990] mpfree-hello[211]: session accepted; sending Akai's setup exactly as the app did",
              "[    7.315] mpfree-hello[211]: SCHED_FIFO priority 70"] {
        p.apply(ToUi::Console(l.into()));
    }
    p.apply(ToUi::Show(View::Play));
    p
}

/// Hands the app's effects to the screen as the audio thread does (lights,
/// sounds and kit changes are left out), for previews and tests.
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
            Effect::Patterns(l) => p.apply(ToUi::Patterns(l)),
            Effect::Workspace(w) => p.apply(ToUi::Workspace(Box::new(w))),
            Effect::Recorded(pat, next) => p.apply(ToUi::Recorded(Box::new(pat), next)),
            Effect::WorkspaceWheel(d) => p.apply(ToUi::WorkspaceWheel(d)),
            Effect::ClearLogs => p.apply(ToUi::ClearLogs),
            Effect::ClearConsole => p.apply(ToUi::ClearConsole),
            Effect::TouchLab => p.apply(ToUi::TouchLab),
            Effect::Note(l) => p.apply(ToUi::Line(l)),
            Effect::ClipsWheel { delta, scroll } => p.apply(ToUi::ClipsWheel { delta, scroll }),
            Effect::SynthWheel(d) => p.apply(ToUi::SynthWheel(d)),
            Effect::Map(m) => p.apply(ToUi::Map(Box::new(m))),
            Effect::SynthPage(d) => p.apply(ToUi::SynthPage(d)),
            Effect::SampleWheel(d) => p.apply(ToUi::SampleWheel(d)),
            Effect::SamplePad(d) => p.apply(ToUi::SamplePad(d)),
            Effect::SampleUndo { redo } => p.apply(ToUi::SampleUndo(redo)),
            Effect::Light(..) | Effect::StopSounds | Effect::Kit(_) | Effect::Audio(_) | Effect::Quit
            | Effect::Time(_) | Effect::Octave(_) | Effect::Bank { .. } => {}
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
    if let Some(g) = p.ui.get_mut::<PadGrid>("pads") {
        for pad in g.pads.iter_mut() {
            pad.held = false;
            pad.level = 0.0;
        }
    }
    p
}

/// The Clips page with the real time engine behind it: tapping Track 1's
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
        for (tick, pad, velocity) in [(0, 0, 118), (1920, 0, 104), (2400, 0, 90), (960, 1, 112), (2880, 1, 116)] {
            hits.push((bar * BAR + tick, pad, velocity));
        }
        for eighth in 0..8 {
            hits.push((bar * BAR + eighth * 480, 2, if eighth % 2 == 0 { 96 } else { 70 }));
        }
    }
    let mut notes: Vec<(i64, Event)> = hits.iter()
        .flat_map(|&(t, pad, velocity)| [(t, Event::NoteOn { note: pad, velocity }), (t + 120, Event::NoteOff { note: pad })])
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
    p.apply(ToUi::Show(View::Clips));
    Ok(p)
}

/// The pad player on the Sample page: kit 1's pad 5 holds the sampler UI's
/// made-up drum loop ("Break 92", trimmed, looping, with fades), and a voice
/// is playing it. Kit 1's other pads are empty here.
pub fn sample_sampler(fonts: Fonts) -> Player {
    let mut p = sample_player(fonts);
    let (project, _) = mpfree_sampler_ui::sample::project();
    let pad = mpfree_sampler_ui::sample::PAD;
    let mut kits = p.kits.clone();
    kits[0].name = "Breaks".to_string();
    kits[0].pads = (0..16).map(|i| (i == pad).then(|| "Break 92".to_string())).collect();
    p.apply(ToUi::Kits(kits));
    p.apply(ToUi::SampleKits { projects: vec![project], rate: mpfree_sampler_ui::sample::RATE });
    p.apply(ToUi::PadDown { pad, velocity: 100 });
    p.apply(ToUi::PadUp(pad));
    p.apply(ToUi::Show(View::Sample));
    let beat = f64::from(mpfree_sampler_ui::sample::RATE) * 60.0 / mpfree_sampler_ui::sample::BPM;
    let mut heads = Playheads { kit: 0, len: 1, ..Playheads::default() };
    heads.pad[0] = pad as u8;
    heads.frame[0] = (beat * 4.6) as u32;
    p.apply(ToUi::SamplePlayheads(heads));
    p
}

/// A sample player with an app behind it (every button known, so no guide).
fn sample_app() -> crate::app::App {
    let mut app = crate::app::App::new(44_100, false);
    (app.map.rec, app.map.overdub, app.map.undo, app.map.play_start) = (Some(90), Some(91), Some(92), Some(93));
    app.start();
    app
}

/// Every screen of the pad player with sample contents, as (name, player
/// showing it), for previews: loading, play, debug, console, prefs, options,
/// the guide, and the arp's quick band, panel and pattern workspace (each
/// with the real app and engine behind it).
pub fn sample_screens(fonts: impl Fn() -> Fonts) -> Vec<(&'static str, Player)> {
    let mut out = Vec::new();
    let mut loading = Player::new(fonts(), SCREEN, false, None);
    loading.apply(ToUi::Progress { message: "Loading drum kits".into(), detail: "Step 2 of 6   ·   18.4 of 39.0 MB".into(),
                                   value: 0.31 });
    out.push(("loading", loading));
    let mut failed = Player::new(fonts(), SCREEN, false, None);
    failed.apply(ToUi::Failed { message: "No sound card".into(),
                                detail: "No sound card called ACVA after 30 s. Tap for the console.".into() });
    out.push(("loading-failed", failed));
    out.push(("play", sample_player(fonts())));
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
    // The guide from Prefs ("Find the buttons again"), PLAY pressed when it
    // asks for MENU.
    let mut guide = sample_player(fonts());
    let mut g = crate::app::App::new(44_100, false);
    let mut fx = g.start();
    fx.extend(g.open_tab(View::Prefs));
    fx.extend(g.step_item(4, 1));
    fx.extend(g.button(button_play(), true, 0.0));
    apply_effects(&mut guide, fx);
    out.push(("guide", guide));
    // Debug mode: Map controls, REC found, listening for OVERDUB.
    let mut map = sample_player(fonts());
    let mut m = sample_app();
    (m.map.rec, m.map.overdub) = (None, None);
    let mut fx = m.open_tab(View::Prefs);
    fx.extend(m.step_item(6, 1));
    fx.extend(m.map_row(0));
    fx.extend(m.button(73, true, 0.0));
    fx.extend(m.button(73, false, 0.0));
    fx.extend(m.map_row(1));
    apply_effects(&mut map, fx);
    out.push(("debug-map", map));
    // Debug mode: the Performance page.
    let mut perf = sample_player(fonts());
    perf.apply(ToUi::Settings(Settings { debug_mode: true, ..Settings::default() }));
    perf.apply(ToUi::Show(View::Perf));
    for k in 0..200u32 {
        let wave = (k as f32 * 0.21).sin().abs();
        perf.apply(ToUi::Timing(Timing { periods_queued: 3, audio: AudioNumbers {
            render_mean_us: 280.0 + 120.0 * wave, render_max_us: 450.0 + 500.0 * wave, synth_mean_us: 90.0,
            synth_max_us: 160.0, worst_us: 1210.0, steals: 2, dropped: 0, peak_grains: 48 }, ..Timing::default() }));
    }
    perf.set_proc(ProcView {
        over_s: 2.0,
        threads: vec![("mpfree-hello (812)".into(), 31.5), ("mpfree-screen (815)".into(), 9.0),
                      ("mpfree-journal (816)".into(), 0.5), ("touch-reader (817)".into(), 0.2)],
        process_cpu: 41.2, system_cpu: Some(14.8), cores: 4, rss_kb: Some(61_440), peak_rss_kb: Some(64_512),
        thread_count: 5, mem_total_kb: Some(2_042_000), mem_available_kb: Some(1_610_000),
        load: Some("0.61 0.48 0.30".into()), temps: vec![("soc-thermal".into(), 47.8)], mhz: vec![1608, 1608, 1608, 1608],
    });
    out.push(("debug-perf", perf));
    match sample_clips(fonts()) {
        Ok(p) => out.push(("clips", p)),
        Err(e) => eprintln!("clips sample: {e}"),
    }
    // The synth on the pads (the kit list's last entry), then its page.
    let synth_kit = sample_kits().len();
    let mut play_synth = sample_player(fonts());
    play_synth.apply(ToUi::Status(Status { shift: false, pad_level: PadLevel::Off, repeat: false, latched: false,
                                           rate: "1/16", bpm: 120.0 }));
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
    // The Sample page: the loop on kit 1's pad 5, then zoomed round the
    // loop start after a Q-Link 3 turn, then a pad without a sound.
    out.push(("sample", sample_sampler(fonts())));
    let mut zoomed = sample_sampler(fonts());
    zoomed.apply(ToUi::SampleQLink(2, 3));
    for _ in 0..10 {
        zoomed.apply(ToUi::SampleWheel(1));
    }
    zoomed.take_actions();
    out.push(("sample-zoomed", zoomed));
    let mut empty = sample_sampler(fonts());
    empty.apply(ToUi::PadDown { pad: 1, velocity: 90 });
    empty.apply(ToUi::PadUp(1));
    out.push(("sample-no-sound", empty));
    let mut lab = sample_player(fonts());
    lab.apply(ToUi::Show(View::Prefs));
    lab.apply(ToUi::TouchLab);
    lab.set_touch_status("touch-reader: seeded=1 slot=0 fingers=0 resyncs=0 fallbacks=0 drained=0 contacts=4 events=212");
    out.push(("touchlab", lab));
    // NOTE REPEAT held with the closed hat: the quick band.
    let mut quick = quiet_player(fonts());
    let mut app = sample_app();
    let mut fx = app.button(NOTE_REPEAT, true, 0.0);
    fx.extend(app.pad_down(2, 100));
    apply_effects(&mut quick, fx);
    quick.apply(ToUi::PadDown { pad: 2, velocity: 100 });
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
    if let Some(g) = panel.ui.get_mut::<PadGrid>("pads") {
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
    fx.extend(app.arp_action(ArpAction::Button(mpfree_arp_ui::view::PatternButton::Record)));
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

    #[test]
    fn the_clips_page_shows_the_engine_and_turns_taps_into_actions() {
        use mpfree_time_ui::widgets::{BarButton, ClipGrid, TransportBar};
        use mpfree_transport::Command;
        let mut p = sample_clips(Fonts::embedded()).unwrap();
        assert_eq!(p.page(), CLIPS_PAGE);
        // Recorded from the pads: two bars of kick, snare and hats.
        assert_eq!(p.time.done.iter().map(|t| (t.number, t.added)).collect::<Vec<_>>(), vec![(1, 26)]);
        assert_eq!((p.time.armed, p.time.launch(0).map(|l| l.slot)), (Some(1), Some(0)));
        let grid = p.ui.rect_of(time_view::CLIPS).unwrap();
        let cell = |p: &Player, track, slot| {
            let r = p.ui.get::<ClipGrid>(time_view::CLIPS).unwrap().cell_rect(grid, track, slot).unwrap();
            (r.x + r.w / 2, r.y + r.h / 2)
        };
        let tap = |p: &mut Player, (x, y): (i32, i32)| {
            let down = p.touch(1, Touch::Down, x, y);
            down.or(p.touch(1, Touch::Up, x, y))
        };
        let empty = cell(&p, 1, 0);
        assert_eq!(tap(&mut p, empty), Some(FromUi::Time(TimeAction::NewClip { track: 1, slot: 0 })));
        let playing = cell(&p, 0, 0);
        assert_eq!(tap(&mut p, playing), Some(FromUi::Time(TimeAction::StopClip(0))));
        // The bar knows its buttons once drawn, as on the screen.
        p.ui.paint(&mut SoftCanvas::new(1280, 800));
        let play = TransportBar::button_rect(p.ui.rect_of(time_view::TRANSPORT).unwrap(), BarButton::Play).unwrap();
        assert_eq!(tap(&mut p, (play.x + play.w / 2, play.y + play.h / 2)),
                   Some(FromUi::Time(TimeAction::Transport(Command::Play))));
        // The wheel steps the selected record setting: the armed track first.
        p.apply(ToUi::ClipsWheel { delta: 1, scroll: false });
        assert_eq!(p.take_actions(), vec![FromUi::Time(TimeAction::Arm(Some(2)))]);
        // SHIFT + wheel scrolls the grid.
        p.apply(ToUi::ClipsWheel { delta: 1, scroll: true });
        assert_eq!(p.ui.get::<ClipGrid>(time_view::CLIPS).unwrap().title, "Clips · slots 2–7 of 16");
        assert!(p.take_actions().is_empty());
    }

    #[test]
    fn layouts_fit_the_screen() {
        let mut p = Player::new(Fonts::embedded(), SCREEN, false, None);
        assert_eq!(p.ui.rect_of("loading"), Some(mpfree_ui::Rect::new(0, 0, 1280, 800)));
        p.apply(ToUi::Show(View::Play));
        let pads = p.ui.rect_of("pads").unwrap();
        assert!(pads.w > 700 && pads.h > 600, "{pads:?}");
        // Under the transport bar four kits show at once; the list scrolls
        // to the selected kit.
        let kits = p.ui.rect_of("kits").unwrap();
        assert!(kits.h >= 14 + 34 + 4 * 46, "{kits:?}");
        assert_eq!(p.ui.rect_of(time_view::TRANSPORT).map(|r| (r.y, r.h)), Some((84, 44)));
        p.apply(ToUi::Show(View::Clips));
        let clips = p.ui.rect_of(time_view::CLIPS).unwrap();
        assert!(clips.w > 780 && clips.h > 560, "{clips:?}");
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
        // The Touch lab is the last Prefs row; + on it opens the lab.
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
        assert!(p.lab_open());
        // Touches go to the lab, not the app. Exit (top right) goes back to Prefs.
        assert_eq!(p.touch(1, Touch::Down, 1280 - 80, 38), None);
        assert_eq!(p.touch(1, Touch::Up, 1280 - 80, 38), None);
        assert_eq!(p.page(), "Prefs");
        assert!(!p.lab_open());
        // Open again; a page change from the app (a hardware button) closes it.
        p.apply(ToUi::TouchLab);
        assert_eq!(p.page(), "Touch lab");
        p.apply(ToUi::Show(View::Play));
        assert_eq!(p.page(), "Play");
        assert!(!p.lab_open());
        // Kernel times reach the lab without upsetting ordinary touches.
        p.set_touch_time(123);
        let pads = p.ui.rect_of("pads").unwrap();
        assert_eq!(p.touch(2, Touch::Down, pads.x + 10, pads.bottom() - 10), Some(FromUi::Pad { pad: 0, down: true }));
    }

    #[test]
    fn before_ready_a_tap_shows_the_console_and_a_tab_goes_back() {
        let mut p = Player::new(Fonts::embedded(), SCREEN, false, None);
        assert_eq!(p.page(), "Loading");
        assert_eq!(p.touch(1, Touch::Down, 600, 300), None);
        assert_eq!(p.page(), "Console");
        p.touch(1, Touch::Up, 600, 300);
        let header = p.ui.rect_of("header").unwrap();
        assert_eq!(p.touch(2, Touch::Down, header.x + 170 + 20, header.y + 30), None);
        assert_eq!(p.page(), "Loading");
    }

    #[test]
    fn touches_become_actions_once_ready() {
        let mut p = sample_player(Fonts::embedded());
        let pads = p.ui.rect_of("pads").unwrap();
        // Pad 1 sits bottom left.
        assert_eq!(p.touch(1, Touch::Down, pads.x + 10, pads.bottom() - 10), Some(FromUi::Pad { pad: 0, down: true }));
        assert_eq!(p.touch(1, Touch::Up, pads.x + 10, pads.bottom() - 10), Some(FromUi::Pad { pad: 0, down: false }));
        let header = p.ui.rect_of("header").unwrap();
        // The Debug tab: the app decides; the page stays until it says.
        assert_eq!(p.touch(2, Touch::Down, header.x + 170 + 118 + 20, header.y + 30), Some(FromUi::Tab(1)));
        assert_eq!(p.page(), "Play");
        p.apply(ToUi::Show(View::Debug));
        assert!(p.ui.rect_of("midi").is_some() && p.ui.rect_of("pads").is_none());
        // A menu row's arrow steps its value.
        let mut app = sample_app();
        let fx = app.button(mpfree_controller::mpc_one::button::MENU, true, 0.0);
        apply_effects(&mut p, fx);
        assert_eq!(p.page(), "Options");
        let r = p.ui.rect_of("options").unwrap();
        let o = p.ui.get::<OptionList>("options").unwrap();
        let (_, value) = o.row_rects(r, 1).unwrap();
        assert_eq!(p.touch(3, Touch::Down, value.right() - 5, value.y + 10), Some(FromUi::MenuStep(1, 1)));
    }

    #[test]
    fn kit_changes_relabel_the_pads() {
        let mut p = sample_player(Fonts::embedded());
        p.apply(ToUi::Kit(3));
        let g = p.ui.get::<PadGrid>("pads").unwrap();
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

    fn events_log(p: &Player) -> Vec<String> {
        p.ui.get::<LogPanel>("events").unwrap().lines().map(|(l, _)| l.clone()).collect()
    }

    #[test]
    fn the_sample_page_edits_the_pad_hit_last_and_sends_one_view_at_a_time() {
        use mpfree_sampler_ui::wave_tab::{BAR, QLINKS};
        use mpfree_sampler_ui::widgets::{KnobCard, PadBar};
        let pad = mpfree_sampler_ui::sample::PAD;
        let mut p = sample_sampler(Fonts::embedded());
        assert_eq!(p.page(), sample_page::PAGE);
        assert_eq!(p.ui.get::<Header>("header").unwrap().active, 3, "the Sample tab, after Synth");
        assert_eq!(p.ui.get::<PadBar>(BAR).unwrap().title, "Pad A5 · Break 92");
        assert!(p.ui.get::<Label>("hint").unwrap().text.contains("Q-Links 1-4: Start, End, loop"));
        let (start, end) = (p.sample.state().unwrap().region.start, p.sample.state().unwrap().region.end);
        // Q-Link 1 moves Start; the kit's view goes to the audio thread.
        p.apply(ToUi::SampleQLink(0, 4));
        let moved = p.sample.state().unwrap().region.start;
        assert!(moved > start);
        assert_eq!(p.sample.project(0).unwrap().pad(pad).unwrap().layers[0].region.start, moved);
        let (sent, rest) = views(p.take_actions());
        assert!(rest.is_empty());
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].kit, 0);
        assert_eq!(sent[0].view.as_ref().unwrap().revision, p.sample.project(0).unwrap().revision());
        // While it travels, more turns wait; they go together once it is back.
        p.apply(ToUi::SampleQLink(0, 2));
        let moved = p.sample.state().unwrap().region.start;
        p.apply(ToUi::SampleQLink(1, -3));
        assert!(p.sample.state().unwrap().region.end < end);
        assert!(views(p.take_actions()).0.is_empty());
        p.apply(ToUi::SampleViewBack(sent.into_iter().next().unwrap()));
        let (sent, _) = views(p.take_actions());
        assert_eq!(sent[0].view.as_ref().unwrap().revision, p.sample.project(0).unwrap().revision());
        // A view that finds no room is built again.
        p.unsent(FromUi::SampleView(sent.into_iter().next().unwrap()));
        let (sent, _) = views(p.take_actions());
        assert_eq!(sent.len(), 1);
        p.apply(ToUi::SampleViewBack(sent.into_iter().next().unwrap()));
        assert!(views(p.take_actions()).0.is_empty(), "nothing new to send");
        // Each turn is one log line, written when the next one starts or
        // the Q-Link is let go.
        p.apply(ToUi::SampleLift);
        let lines: Vec<String> = events_log(&p).into_iter().filter(|l| l.starts_with("sample: ")).collect();
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(lines[0].starts_with("sample: kit 1 (Breaks) pad A5 Break 92: Start 0.163 s -> "), "{lines:?}");
        assert!(lines[1].starts_with("sample: kit 1 (Breaks) pad A5 Break 92: End 4.891 s -> "), "{lines:?}");
        // Each turn is one undo step: End first, then both Start turns.
        p.apply(ToUi::SampleUndo(false));
        assert_eq!((p.sample.state().unwrap().region.start, p.sample.state().unwrap().region.end), (moved, end));
        p.apply(ToUi::SampleUndo(false));
        p.apply(ToUi::SampleUndo(false));
        assert_eq!(p.sample.state().unwrap().region.start, start);
        assert!(events_log(&p).iter().any(|l| l.starts_with("sample: nothing to undo in kit 1")));
        assert_eq!(views(p.take_actions()).0.len(), 1);
        // A pad without a sound shows its own page; - / + find the next sound.
        p.apply(ToUi::PadDown { pad: 1, velocity: 90 });
        assert_eq!(p.page(), sample_page::EMPTY_PAGE);
        assert!(p.ui.get::<Card>(sample_page::EMPTY).unwrap().heading.starts_with("Pad A2 has no sound in kit 1"));
        p.apply(ToUi::SamplePad(1));
        assert_eq!((p.page(), p.sample.state().map(|s| s.pad)), (sample_page::PAGE, Some(pad)));
        // Q-Link 3 is the loop start; another kit names its bank.
        assert!(p.ui.get::<KnobCard>(QLINKS[2]).unwrap().enabled);
        p.apply(ToUi::Kit(1));
        assert_eq!(p.page(), sample_page::EMPTY_PAGE, "kit 2 has no project here");
        p.apply(ToUi::Kit(sample_kits().len()));
        assert_eq!(p.ui.get::<Card>(sample_page::EMPTY).unwrap().heading, "The pads play the synth");
    }

    #[test]
    fn a_tap_on_the_wave_plays_the_pad_and_a_looping_one_until_the_next_tap() {
        use mpfree_sampler_ui::wave_tab::WAVE;
        let pad = mpfree_sampler_ui::sample::PAD;
        let mut p = sample_sampler(Fonts::embedded());
        let w = p.ui.rect_of(WAVE).unwrap();
        let tap = |p: &mut Player, f: u64| {
            assert_eq!(p.touch(f, Touch::Down, w.x + w.w * 3 / 4, w.y + w.h / 2), None);
            assert_eq!(p.touch(f, Touch::Up, w.x + w.w * 3 / 4, w.y + w.h / 2), None);
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
            let ev = UiEvent { widget: mpfree_sampler_ui::wave_tab::REGION.into(), kind: EventKind::Step { index: 0, delta: -1 } };
            p.on_event(ev);
        }
        assert_eq!(p.sample.state().unwrap().region.mode, mpfree_sampler::model::PlayMode::OneShot);
        assert_eq!(tap(&mut p, 4), vec![FromUi::Pad { pad, down: true }, FromUi::Pad { pad, down: false }]);
    }

    #[test]
    fn the_synth_page_shows_poly_and_sends_its_changes() {
        let mut p = sample_player(Fonts::embedded());
        let synth_kit = sample_kits().len();
        let chips = |p: &Player| -> Vec<String> {
            p.ui.get::<Header>("header").unwrap().chips.iter().map(|c| c.text.clone()).collect()
        };
        assert!(chips(&p).contains(&"Kit 1 of 7 · Bank A".to_string()), "{:?}", chips(&p));
        p.apply(ToUi::Kit(synth_kit));
        assert!(chips(&p).contains(&"Poly · C3 to D#4".to_string()), "{:?}", chips(&p));
        assert_eq!(p.ui.get::<PadGrid>("pads").unwrap().pads[1].label, "C#3");
        // The Synth tab is the second; the app decides, then the page shows.
        let header = p.ui.rect_of("header").unwrap();
        assert_eq!(p.touch(1, Touch::Down, header.x + 170 + 118 + 20, header.y + 30), Some(FromUi::Tab(1)));
        p.apply(ToUi::Show(View::Synth));
        assert_eq!(p.page(), "Synth");
        assert!(p.ui.rect_of(synth_view::TILES).is_some_and(|r| r.h > 300));
        assert!(p.ui.get::<Label>("hint").unwrap().text.contains("Q-Links 1-4"));
        // A Q-Link turns its knob on screen at once and sends the change.
        assert!(p.take_actions().is_empty());
        p.apply(ToUi::SynthQLink(0, 5));
        let acts = p.take_actions();
        assert!(matches!(acts.as_slice(), [FromUi::Synth(SynthAction::Set { .. })]), "{acts:?}");
        p.apply(ToUi::SynthWheel(-3));
        assert_eq!(p.take_actions().len(), 1);
        // A preset touched goes to the audio thread, whose values come back.
        let ev = UiEvent { widget: synth_view::PRESETS.to_string(), kind: EventKind::Select(3) };
        assert_eq!(p.on_event(ev), Some(FromUi::Synth(SynthAction::Preset(3))));
        let mut s = crate::synth::Synth::new(44_100);
        s.load_preset(3);
        p.apply(ToUi::SynthValues(s.values()));
        assert_eq!(p.synth.as_ref().unwrap().state.values, s.values());
        // An octave up relabels the synth's pads and its chip.
        s.shift_octave(1);
        p.apply(ToUi::KitPads { kit: synth_kit, pads: s.pad_labels() });
        assert!(chips(&p).contains(&"Poly · C4 to D#5".to_string()), "{:?}", chips(&p));
        assert_eq!(p.ui.get::<PadGrid>("pads").unwrap().pads[0].label, "C4");
        // Meters update the page's copy.
        let i = p.synth.as_ref().unwrap().state.index(mpfree_synth::params::METER_VOICES).unwrap();
        p.apply(ToUi::SynthMeters(vec![(i, 3.0)]));
        assert_eq!(p.synth.as_ref().unwrap().state.values[i], 3.0);
    }

    #[test]
    fn a_paused_console_keeps_lines_for_later() {
        let mut p = sample_player(Fonts::embedded());
        let count = |p: &Player| p.ui.get::<LogPanel>("syslog").unwrap().lines().count();
        let before = count(&p);
        let paused = Settings { console_paused: true, ..Settings::default() };
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
        let mut s = Status { shift: true, pad_level: PadLevel::Off, repeat: false, latched: true, rate: "1/8",
                             bpm: 96.0 };
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
        let chips: Vec<String> = p.ui.get::<Header>("header").unwrap().chips.iter().map(|c| c.text.clone()).collect();
        assert!(chips.contains(&"Repeat 1/8, latched · 96 BPM".to_string()), "{chips:?}");
        assert!(!chips.iter().any(|c| c.ends_with(" dB")), "the level is on the slider: {chips:?}");
        s.shift = false;
        p.apply(ToUi::Status(s));
        assert!(p.ui.get::<Label>("hint").unwrap().text.contains("NOTE REPEAT: repeat"));
        p.apply(ToUi::PadFlash { pad: 3, velocity: 90 });
        let g = p.ui.get::<PadGrid>("pads").unwrap();
        assert!(!g.pads[3].held && g.pads[3].level == 1.0);
    }

    #[test]
    fn the_quick_band_comes_with_note_repeat_and_the_chip_opens_the_panel() {
        use mpfree_controller::mpc_one::button::NOTE_REPEAT;
        let mut p = sample_player(Fonts::embedded());
        let mut app = sample_app();
        let fx = app.button(NOTE_REPEAT, true, 0.0);
        apply_effects(&mut p, fx);
        assert_eq!(p.page(), QUICK_PAGE);
        assert!(p.ui.rect_of(arp_view::RATES).is_some() && p.ui.rect_of("kits").is_none());
        // A rate button.
        let rates = p.ui.rect_of(arp_view::RATES).unwrap();
        let a = p.touch(7, Touch::Down, rates.x + 20, rates.y + rates.h / 2);
        assert_eq!(a, Some(FromUi::Arp(ArpAction::Rate(mpfree_arp::Rate::Quarter))));
        p.touch(7, Touch::Up, rates.x + 20, rates.y + rates.h / 2);
        let fx = app.button(NOTE_REPEAT, false, 0.0);
        apply_effects(&mut p, fx);
        assert_eq!(p.page(), "Play");
        // The arp chip, drawn once, opens the panel.
        let mut c = SoftCanvas::new(SCREEN.0, SCREEN.1);
        p.ui.paint(&mut c);
        let chip = p.ui.get::<Header>("header").unwrap().chip_rect(p.arp_chip.unwrap()).unwrap();
        assert_eq!(p.touch(8, Touch::Down, chip.x + 10, chip.y + 10), Some(FromUi::ArpChip));
        p.touch(8, Touch::Up, chip.x + 10, chip.y + 10);
        let mut fx = app.toggle_panel();
        fx.extend(app.arp_poll());
        apply_effects(&mut p, fx);
        assert_eq!(p.page(), ARP_PAGE);
        assert_eq!(p.ui.get::<Header>("header").unwrap().active, 0, "the panel belongs to Play");
        let modes = p.ui.rect_of(arp_view::MODES).unwrap();
        let a = p.touch(9, Touch::Down, modes.right() - 10, modes.y + modes.h / 2);
        assert_eq!(a, Some(FromUi::Arp(ArpAction::Mode(Mode::Pattern))));
        p.touch(9, Touch::Up, modes.right() - 10, modes.y + modes.h / 2);
        assert!(p.ui.get::<Label>("hint").unwrap().text.contains("NOTE REPEAT + MENU: close"));
    }

    #[test]
    fn a_page_change_lets_go_of_pads_held_on_the_screen() {
        let mut p = sample_player(Fonts::embedded());
        let pads = p.ui.rect_of("pads").unwrap();
        assert_eq!(p.touch(1, Touch::Down, pads.x + 10, pads.bottom() - 10), Some(FromUi::Pad { pad: 0, down: true }));
        let mut app = sample_app();
        let fx = app.button(mpfree_controller::mpc_one::button::NOTE_REPEAT, true, 0.0);
        apply_effects(&mut p, fx);
        assert_eq!(p.page(), QUICK_PAGE);
        assert_eq!(p.take_actions(), vec![FromUi::Pad { pad: 0, down: false }]);
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
        let Some(FromUi::Workspace(ws)) = a else { panic!("{a:?}") };
        let WsAction::Changed(pat) = *ws else { panic!("{ws:?}") };
        assert_eq!(pat.to_text(), "x!");
        p.touch(2, Touch::Up, grid.x + 122, grid.y + 38);
        apply_effects(&mut p, app.workspace_action(WsAction::Changed(pat)));
        // The wheel turns the selected cell (here the note pick) through the app and back.
        p.workspace.editor.select(Row::Note, 0);
        apply_effects(&mut p, app.wheel(1));
        let back = p.take_actions();
        assert!(matches!(back.as_slice(), [FromUi::Workspace(a)] if matches!(**a, WsAction::Changed(_))), "{back:?}");
        // Recording: the pads say what they write.
        let fx = app.workspace_action(WsAction::Record(true));
        apply_effects(&mut p, fx);
        assert_eq!(p.page(), RECORD_PAGE);
        assert_eq!(p.ui.get::<PadGrid>("pads").unwrap().pads[12].label, "Next");
        let fx = app.workspace_action(WsAction::Back);
        apply_effects(&mut p, fx);
        assert_eq!(p.page(), ARP_PAGE);
        assert_eq!(p.ui.get::<PadGrid>("pads").unwrap().pads[0].label, "Kick", "the kit's names again");
    }

    #[test]
    fn every_sample_screen_draws() {
        for (name, mut p) in sample_screens(Fonts::embedded) {
            let mut c = SoftCanvas::new(SCREEN.0, SCREEN.1);
            let d = p.ui.paint(&mut c);
            assert_eq!(d.len(), 1, "{name}: a first paint covers the screen");
        }
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
        assert_eq!(parse_input_event(&b), InputEvent::new(2_000_500, 3, 0x35, -5));
    }

    /// A stand-in touchscreen: queued reads, and no slot snapshot (as on a
    /// node that refuses the ioctl), so the reader runs unseeded.
    struct FakeTouch(VecDeque<Vec<u8>>);

    impl TouchSource for FakeTouch {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let Some(chunk) = self.0.pop_front() else { return Err(ErrorKind::WouldBlock.into()) };
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
        for (t, c, v) in [(3, 0x2f, 0), (3, 0x39, 5), (3, 0x35, 1024), (3, 0x36, 2048), (0, 0, 0)] {
            evs.extend(ev(t, c, v));
        }
        let t = TouchIn::from_source(Box::new(FakeTouch(VecDeque::new())), SCREEN).unwrap();
        assert!(t.status_line().starts_with("touch-reader: seeded=0 "), "{}", t.status_line());
        assert!(t.status_line().contains("running unseeded"));
        // The report arrives after open, so the open-time drain doesn't drop it:
        // the fake's first read is the drain's (empty), the second is ours.
        let mut t2 = TouchIn::from_source(Box::new(FakeTouch(VecDeque::from([Vec::new(), evs]))), SCREEN).unwrap();
        let got = t2.read().unwrap();
        assert_eq!(got.len(), 1);
        let (_, phase, x, y, _) = got[0];
        assert_eq!(phase, Touch::Down);
        // Raw Y runs right to left (2048 = left edge); raw X runs top to bottom.
        assert_eq!((x, y), (0, 400));
        assert_eq!(t2.contacts, 1);
    }
}
