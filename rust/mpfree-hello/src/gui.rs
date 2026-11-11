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

use crate::app::{
    tabs, ArpView, GuideView, MapView, MenuView, PadLevel, Settings, Status, View, WorkspaceView,
};
use crate::calibrate_page::CalibrateView;
use crate::debug_page::{AudioNumbers, DebugPages};
use crate::looper_page::{LooperPage, LOOPER_PAGE};
use crate::looping::{LoopWave, LooperView};
use crate::mixing::{PlayFx, PlayLevels};
use crate::perf::ProcView;
use crate::play_page::{self, Knob, PadInfo, PadsBar, PlayPage, PlayView, SynthPads, Turn};
use crate::play_widgets::{MiniPads, PlayTransport, StepGrid, Throws, XyPad, YourTiming};
use crate::sample_page::{self, SamplePage};
use crate::sampler::{Playheads, ViewSwap};
use crate::time::{HitInfo, StepAction, StepsData};
use crate::touchlab_page::{TouchLabView, TouchPage};
use mpfree_arp::{Mode, Pattern};
use mpfree_arp_ui::view::{self as arp_view, ArpAction, ArpPanel, ArpState};
use mpfree_arp_ui::widgets::SliderStack;
use mpfree_arp_ui::workspace::{self, Editor, PatternWorkspace, Row, WsAction};
use mpfree_sampler::model::Project;
use mpfree_synth_ui::view as synth_view;
use mpfree_synth_ui::{SynthAction, SynthPanel, SynthState};
use mpfree_time_ui::view::{
    self as time_view, TimePage, TimeState, TransportInfo, CLIPS_HINT, CLIPS_PAGE,
};
use mpfree_time_ui::TimeAction;
use mpfree_touch::mt::Phase;
use mpfree_touch::pointer::PointerEvent;
use mpfree_touch::profile::MPC_ONE;
use mpfree_touch::touchlab::reader_line;
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
pub const GUIDE_LAYOUT: &str = "col(header:60, row(guide:*, found:400):*, hint:26)";

const CONSOLE_HINT: &str =
    "Switch the MPC off with its power switch to return to the stock firmware";
const SHIFT_HINT: &str =
    "SHIFT +   wheel: synth octave   ·   FULL LEVEL: Half Level   ·   NOTE REPEAT: Latch   ·   MENU: Prefs   ·   STOP: console";
const SHIFT_SYNTH_HINT: &str =
    "SHIFT +   wheel: page   ·   FULL LEVEL: Half Level   ·   NOTE REPEAT: Latch   ·   MENU: Prefs   ·   STOP: console";
const SHIFT_SAMPLE_HINT: &str =
    "SHIFT +   wheel: pad   ·   Q-Links: finer   ·   UNDO: redo   ·   MENU: Prefs   ·   STOP: console";
const SHIFT_MENU_HINT: &str =
    "SHIFT +   wheel: change the value   ·   MENU: Prefs   ·   STOP: console";
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
    Session(String, Tone),
    Timing(Timing),
    Show(View),
    Menu(MenuView),
    Guide(GuideView),
    Status(Status),
    Settings(Settings),
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
    Quit,
}

#[derive(Clone, Debug, PartialEq)]
pub enum FromUi {
    Pad {
        pad: usize,
        down: bool,
    },
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
    /// A touch on the transport bar or the Arrange page.
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
    /// A button on the Looper tab (`looper_page::BUTTONS`).
    Looper(usize),
    /// A loop layer's control (its speaker on the Play page's loop), by the
    /// layer's id.
    LoopLayer {
        id: u8,
        command: mpfree_looper::LayerCommand,
    },
    /// What a touch on the mixer panel (or a Q-Link while it is open)
    /// meant (`mpfree_mixer_ui::take_actions`, `qlink_turn`), for the mixer
    /// on the audio thread.
    Mixer(mpfree_mixer_ui::MixAction),
    /// The top bar's mixer icon: open or close the mixer (the app decides).
    MixerIcon,
    /// The Play page: a filter, a send, a fader, a speaker, a throw or the
    /// XY pad, for the mixer (`mixing.rs`).
    PlayFx(PlayFx),
    /// The Play page's pattern: a step tapped or changed.
    Steps(StepAction),
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
    /// The widget of the touch page on screen (Touch lab, Calibrate touch),
    /// over `view`, which it returns to.
    touch_page: Option<&'static str>,
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
    /// The time engine's bar and Arrange page, and what they show.
    time_page: TimePage,
    pub time: TimeState,
    /// The Debug tab's Performance and Map controls pages.
    debug: DebugPages,
    /// The audio loop's CPU by engine numbers (None until the loop starts).
    engines: Option<Arc<crate::engines::EngineShared>>,
    /// The Looper tab, and what it shows.
    looper_page: LooperPage,
    pub looper: LooperView,
    /// The Play page: its view (the screen's own choice, kept while other
    /// tabs show), pattern, throws, timing and Q-Links.
    pub play: PlayPage,
    /// When the CPU chip was last read.
    cpu_at: f32,
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
        ui.add("header", header);
        let mut loading = Loading::new("MPFree", "Pad player");
        loading.set_backdrop(backdrop);
        loading.hint = LOADING_HINT.to_string();
        loading.message = "Starting".to_string();
        ui.add("loading", loading);
        ui.add(play_page::PADS, MiniPads::default());
        ui.add("kits", ListView::new("Kits"));
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
        let looper_page = LooperPage::add(&mut ui).expect("looper layout");
        for v in PlayView::ALL {
            ui.add_page(v.page(), &v.layout()).expect("play layout");
        }
        let debug = DebugPages::add(&mut ui).expect("debug layouts");
        ui.add_page("Console", SYSLOG_LAYOUT)
            .expect("console layout");
        ui.add_page("Prefs", OPTIONS_LAYOUT).expect("prefs layout");
        ui.add_page("Options", OPTIONS_LAYOUT)
            .expect("options layout");
        ui.add_page("Guide", GUIDE_LAYOUT).expect("guide layout");
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
        let workspace = PatternWorkspace::add(&mut ui, PATTERN_PAGE, RECORD_PAGE, &name, pattern)
            .expect("workspace layouts");
        ui.add("touchlab", TouchLabView::new());
        ui.add_full_page("Touch lab", "touchlab")
            .expect("touch lab layout");
        ui.add("calibrate", CalibrateView::new());
        ui.add_full_page("Calibrate touch", "calibrate")
            .expect("calibrate layout");
        let sample = SamplePage::add(&mut ui).expect("sample layout");
        // The mixer over the page, under the top bar, with the Q-Link cards
        // beside it (they turn the selected strip).
        mpfree_mixer_ui::add(&mut ui).expect("mixer layout");
        ui.add_page(mpfree_mixer_ui::MIXER_PAGE, MIXER_LAYOUT)
            .expect("mixer layout");
        let _ = ui.show("Loading");
        let status = Status {
            shift: false,
            pad_level: PadLevel::Off,
            repeat: false,
            latched: false,
            rate: "1/16",
            bpm: 120.0,
        };
        let mut p = Player {
            ui,
            kits: Vec::new(),
            kit: 0,
            midi: 0,
            midi_held_back: 0,
            timing: Timing {
                last_ms: f32::NAN,
                ..Timing::default()
            },
            screen_row: "-".to_string(),
            timed,
            ready: false,
            view: View::Play,
            status,
            settings: Settings::default(),
            session: ("Starting".to_string(), Tone::Dim),
            console_from_lines: timed,
            console_backlog: VecDeque::new(),
            touch_page: None,
            synth: None,
            sample,
            arp_panel,
            workspace,
            arp,
            qlink_focus: None,
            arp_chip: None,
            fingers: Vec::new(),
            outbox: Vec::new(),
            time_page,
            time: TimeState::default(),
            debug,
            engines: None,
            looper_page,
            looper: LooperView::default(),
            play: PlayPage::default(),
            cpu_at: 0.0,
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
                // The Drum view says how hits are played (the mockups);
                // the others name the kit.
                None if self.play.view == PlayView::Drum => (
                    format!("Pads · Bank {}", crate::app::bank_letter(self.kit)),
                    match self.status.pad_level {
                        PadLevel::Off => "velocity on",
                        PadLevel::Full => "full level",
                        PadLevel::Half => "half level",
                    }
                    .to_string(),
                    None,
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
    /// sliders show); on Play, Q-Link 1 is the level.
    fn refresh_qlinks(&mut self) {
        let arp = self.view == View::Arp || (self.view == View::Play && self.band_showing());
        let (title, cards) = if self.view == View::Mixer {
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
            ("Q-Links · Play", cards)
        };
        if let Some(q) = self.ui.get_mut::<QLinkColumn>(play_page::QLINKS) {
            if q.title != title || q.cards != cards {
                q.title = title.to_string();
                q.cards = cards;
            }
        }
    }

    /// Q-Link `k` (a knob, a drag on its card, or the wheel as Q-Link 1)
    /// while the mixer is open: the selected strip's Volume, Pan, Send A or
    /// Send B moves on the panel at once, and the mixer hears it.
    fn mixer_qlink(&mut self, k: usize, d: i32) {
        if self.view != View::Mixer || d == 0 {
            return;
        }
        if let Some(a) = mpfree_mixer_ui::qlink_turn(&mut self.ui, k, d, self.status.shift) {
            self.outbox.push(FromUi::Mixer(a));
        }
        self.refresh_qlinks();
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
        let _ = self
            .arp_panel
            .set_quick_layout(&mut self.ui, &v.quick_layout());
        if self.ready && self.view == View::Play {
            self.show_view(View::Play);
        }
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
            });
        if !same {
            if let Some(w) = self.ui.get_mut::<StepGrid>(play_page::STEPS) {
                w.title = g.title;
                w.right = g.right;
                w.lanes = g.lanes;
                w.steps = g.steps;
                w.play = g.play;
                w.selected = g.selected;
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
            let mut label = String::new();
            let mut probe = XyPad::default();
            self.play.fill_xy(&mut probe);
            if xy.label != probe.label {
                label = probe.label;
            }
            if !label.is_empty() {
                if let Some(xy) = self.ui.get_mut::<XyPad>(play_page::XY) {
                    xy.label = label;
                }
            }
        }
        if self.view == View::Play && self.play.view == PlayView::Drum {
            if let Some(t) = self.ui.get_mut::<YourTiming>(play_page::TIMING) {
                self.play.fill_timing(t);
            }
        }
    }

    /// A Q-Link on the Play page (no arp): what it turns there.
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
        if let Some((text, tone)) = self.engines.as_deref().and_then(crate::engines::cpu_chip) {
            chips.push(Chip::new(text, tone));
        }
        if self.session.1 != Tone::Good || !self.ready {
            chips.push(Chip::new(self.session.0.clone(), self.session.1));
        }
        match self.kits.get(self.kit) {
            Some(KitView {
                chip: Some(chip), ..
            }) => chips.push(Chip::new(chip.clone(), Tone::Normal)),
            Some(_) => chips.push(Chip::new(
                format!(
                    "Kit {} of {} · Bank {}",
                    self.kit + 1,
                    self.kits.len(),
                    crate::app::bank_letter(self.kit)
                ),
                Tone::Normal,
            )),
            None => {}
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
        let text = if self.status.shift
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
        } else if self.view == View::Play && self.band_showing() && self.status.latched {
            format!("Latched: {}   ·   NOTE REPEAT: let go   ·   hold NOTE REPEAT for the wheel, - / + and Q-Links   ·   \
                     {stop}", crate::app::mode_label(mode))
        } else if self.view == View::Play && self.band_showing() {
            format!("Wheel: {wheel}   ·   - / +: Repeat, Arp, Pattern   ·   Q-Links: rate, gate, swing, octaves   ·   \
                     + MENU: Arp panel")
        } else {
            match self.view {
                View::Play => format!("Tap All, Beats, Loop or Drum: the view   ·   Wheel or tap: kit   ·   Q-Link 1 or \
                                       the master bar: level   ·   NOTE REPEAT: repeat   ·   MENU: options   ·   {stop}"),
                View::Arrange => CLIPS_HINT.to_string(),
                View::Looper => format!("REC: record   ·   OVERDUB: overdub   ·   UNDO: undo (SHIFT: redo)   ·   PLAY / STOP: \
                                         the transport   ·   MENU: looper options   ·   {stop}"),
                View::Synth => format!("Pads play {}   ·   Q-Links 1-4: the marked knobs   ·   Wheel: the selected knob   ·   \
                                        - / +: page   ·   MENU: engine, scale   ·   {stop}",
                                       self.synth.as_ref().map_or("the synth", |p| p.state.info.name)),
                View::Sample if self.sample.page() == sample_page::PAGE => {
                    "Hit a pad to edit it   ·   Q-Links 1-4: Start, End, loop   ·   Wheel: zoom   ·   - / +: pad   ·   \
                     UNDO: undo   ·   tap the wave: play".to_string()
                }
                View::Sample if self.sample.page() == sample_page::CHOP_PAGE => {
                    "Hit a pad to chop it   ·   Q-Links 1-4: slice, its start, setting, first pad   ·   Wheel: zoom   ·   \
                     - / +: pad   ·   UNDO: undo".to_string()
                }
                View::Sample if self.sample.page() == sample_page::PADS_PAGE => {
                    "Hit or tap a pad to pick it, tap again to hear it   ·   Q-Links 1-4: level, pan, tune, voices   ·   \
                     Wheel: the row picked   ·   UNDO: undo".to_string()
                }
                View::Sample if self.sample.page() == sample_page::PLAY_PAGE => {
                    "Hit a pad: its velocity shows on the layers   ·   Q-Links 1-4: Attack, Decay, Cutoff, Resonance   ·   \
                     Wheel: the row picked   ·   - / +: pad   ·   UNDO: undo".to_string()
                }
                View::Sample => format!("Hit a pad with a sound to edit it   ·   - / +: next pad with a sound   ·   {stop}"),
                View::Debug => format!("Every MIDI message in and out, newest at the bottom   ·   SHIFT + wheel or - / +: \
                                        Dev page   ·   MENU: options   ·   {stop}"),
                View::Perf => format!("CPU per thread and memory every 2 s, audio every 0.25 s; a perf: line goes to \
                                       the log every 30 s   ·   SHIFT + wheel: Dev page   ·   {stop}"),
                View::Engines => format!("Each stage of the audio loop, every 0.25 s; a cpu by engine line goes to the \
                                          log every 30 s   ·   SHIFT + wheel: Dev page   ·   {stop}"),
                View::Procs => format!("Every process on the MPC, read from /proc once a second while this page \
                                        shows   ·   SHIFT + wheel: Dev page   ·   {stop}"),
                View::Map => "Tap a control, then press it   ·   Wheel: pick   ·   +: listen   ·   -: skip   ·   \
                              STOP: stop listening   ·   SHIFT + wheel: Dev page".to_string(),
                View::Console if self.settings.debug_mode => "The system log, as it comes   ·   MENU: pause or clear   ·   \
                                                              SHIFT + wheel: Dev page   ·   SHIFT + STOP: back".to_string(),
                View::Console => "The system log, as it comes   ·   MENU: pause or clear   ·   SHIFT + STOP: back"
                    .to_string(),
                View::System => "Read when the page opens   ·   MENU: back to Prefs   ·   a tab: leave".to_string(),
                View::Prefs | View::Options => OPTIONS_HINT.to_string(),
                View::Mixer => mpfree_mixer_ui::MIXER_HINT.to_string(),
                View::Guide => GUIDE_HINT.to_string(),
                View::Arp => format!("Wheel: {wheel}   ·   - / +: mode   ·   Q-Links: rate, gate, swing, octaves   ·   \
                                      MENU: options   ·   NOTE REPEAT + MENU: close"),
                View::Pattern => self.workspace.hint().to_string(),
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
                self.sample.hit(pad);
                self.sample.hit_velocity(&mut self.ui, pad, velocity);
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
                if self.view == View::Play && self.play.view == PlayView::Drum {
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
                if self.view == View::Play {
                    self.play_qlink(k, ticks);
                }
            }
            ToUi::QLinkTouch(k, on) => {
                if let Some(q) = self
                    .ui
                    .get_mut::<QLinkColumn>(play_page::QLINKS)
                    .filter(|q| q.touched.get(k) != Some(&on))
                {
                    q.touched[k] = on;
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
                self.looper = *v;
                if matches!(self.view, View::Looper | View::Play) {
                    self.looper_page
                        .update(&mut self.ui, &self.looper, &self.settings);
                }
            }
            ToUi::LoopWave(w) => {
                self.looper_page.set_wave(&mut self.ui, w);
                self.looper_page
                    .update(&mut self.ui, &self.looper, &self.settings);
            }
            // Nothing until the panel is in the Ui (mpfree_mixer_ui::add).
            ToUi::Mixer(st) => {
                mpfree_mixer_ui::update(&mut self.ui, *st);
                if self.view == View::Mixer {
                    self.refresh_qlinks();
                }
            }
            ToUi::MixerQLink(k, d) => self.mixer_qlink(k, d),
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
            ToUi::Synth(state) => match self.synth.as_mut() {
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
            },
            ToUi::SynthValues(values) => {
                if let Some(p) = self
                    .synth
                    .as_mut()
                    .filter(|p| p.state.values.len() == values.len())
                {
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
            }
            ToUi::SynthWheel(ticks) => {
                if let Some(a) = self
                    .synth
                    .as_mut()
                    .and_then(|p| p.wheel(&mut self.ui, ticks))
                {
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
            ToUi::SampleUndo(redo) => {
                let before = self.sample.page();
                self.sample.undo(&mut self.ui, redo);
                self.sample_after(before);
            }
            ToUi::SamplePlayheads(h) => {
                if self.view == View::Sample {
                    self.sample.set_playheads(&mut self.ui, &h);
                }
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
            ToUi::Time(st) => {
                self.time = *st;
                self.time_page.update(&mut self.ui, &self.time);
                self.play.set_transport(self.time.transport);
                self.refresh_play();
            }
            ToUi::Transport(info) => {
                self.time.transport = info;
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
                self.time_page.scroll(&self.ui, delta);
                self.time_page.update(&mut self.ui, &self.time);
            }
            ToUi::ClipsWheel {
                delta,
                scroll: false,
            } => {
                // The row last touched (the armed track to begin with).
                let index = self
                    .ui
                    .get::<OptionList>(time_view::RECORD)
                    .map_or(0, |l| l.selected);
                let ev = UiEvent {
                    widget: time_view::RECORD.to_string(),
                    kind: EventKind::Step { index, delta },
                };
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
        if self.ready && self.view == View::Sample && self.touch_page.is_none() {
            self.show_view(View::Sample);
        }
    }

    /// After an edit on the Sample page: show the page again if it changed
    /// (Wave to Chop, or a pad without a sound), and give the Play page the
    /// names of pads a chop filled.
    fn sample_after(&mut self, before: &str) {
        if let Some((kit, pads)) = self.sample.take_renamed() {
            if let Some(k) = self.kits.get_mut(kit) {
                k.pads = pads;
                if kit == self.kit {
                    self.set_kit(kit);
                }
            }
        }
        if self.sample.page() != before {
            self.sample_changed();
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
    }

    /// The quick band shows over the Play page while NOTE REPEAT is held or
    /// pads are latched.
    fn band_showing(&self) -> bool {
        self.status.repeat
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
        std::mem::take(&mut self.outbox)
    }

    fn show_view(&mut self, v: View) {
        self.close_touch_page();
        let page = match v {
            View::Play if self.band_showing() && self.ready => QUICK_PAGE.to_string(),
            View::Play => self.play.view.page().to_string(),
            View::Arrange => CLIPS_PAGE.to_string(),
            View::Synth => "Synth".to_string(),
            View::Looper => LOOPER_PAGE.to_string(),
            View::Sample => {
                self.sample.refresh_if_stale(&mut self.ui);
                self.sample.page().to_string()
            }
            // Without the developer tools the Console (SHIFT + STOP) has no strip.
            View::Console if !self.settings.debug_mode => "Console".to_string(),
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

    /// The header highlights the page's tab, or none (Prefs, menus, the guide).
    fn mark_tab(&mut self) {
        let tab = self.view.tab_index(self.settings.debug_mode);
        let mixer = Some(self.view == View::Mixer);
        if let Some(h) = self.ui.get_mut::<Header>("header") {
            h.active = tab.unwrap_or(usize::MAX);
            if h.mixer_icon != mixer {
                h.mixer_icon = mixer;
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
            DebugPages::set_engines(&mut self.ui, &snap);
        }
    }

    /// One /proc sample for the Processes page.
    pub fn set_procs(&mut self, v: &mpfree_prof::procs::ProcView) {
        DebugPages::set_procs(&mut self.ui, v);
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
        let action = self
            .ui
            .touch(finger, phase, x, y)
            .and_then(|ev| self.on_event(ev));
        if matches!(phase, Touch::Up | Touch::Cancel) && self.view == View::Sample {
            // A drag on the Sample page ends with its finger: one undo step.
            self.sample.lift();
            self.print_sample_lines();
        }
        action
    }

    /// A touch meant something: hand it to the audio thread, which decides.
    pub fn on_event(&mut self, ev: UiEvent) -> Option<FromUi> {
        let synth_widget = [
            synth_view::TITLE,
            synth_view::VISUALS,
            synth_view::TILES,
            synth_view::PRESETS,
            synth_view::QLINKS,
        ]
        .contains(&ev.widget.as_str());
        if synth_widget {
            let p = self.synth.as_mut()?;
            return p.event(&mut self.ui, &ev).map(FromUi::Synth);
        }
        if ev.widget.starts_with(sample_page::PREFIX) {
            let before = self.sample.page();
            self.sample.event(&mut self.ui, &ev, &mut self.outbox);
            self.sample_after(before);
            self.print_sample_lines();
            return None;
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
            ("controls", EventKind::Select(i)) => Some(FromUi::MapRow(i)),
            ("mapcard", EventKind::Tab(i)) => Some(FromUi::MapButton(i)),
            ("lbuttons", EventKind::Tab(i)) => Some(FromUi::Looper(i)),
            (play_page::PADS, EventKind::Pad { index, down }) => {
                Some(FromUi::Pad { pad: index, down })
            }
            (play_page::PADS, EventKind::Tab(i)) => Some(FromUi::PadsLayout { in_key: i == 1 }),
            ("kits", EventKind::Select(k)) => Some(FromUi::Kit(k)),
            (play_page::TRANSPORT, EventKind::Tab(i)) => {
                self.show_play_view(PlayView::from_index(i));
                None
            }
            (play_page::MASTER, EventKind::Value(v)) => {
                Some(FromUi::LevelDb(mpfree_ui::draw::meter_db(v)))
            }
            (play_page::QLINKS, EventKind::Step { index, delta }) if self.view == View::Mixer => {
                self.mixer_qlink(index, delta);
                None
            }
            (play_page::QLINKS, EventKind::Step { index, delta })
                if self.view == View::Play && !self.band_showing() =>
            {
                self.play_qlink(index, delta);
                None
            }
            (play_page::STEPS, EventKind::Cell { row, col }) => {
                let a = self.play.tap_step(row, col);
                self.refresh_play();
                self.refresh_qlinks();
                a.map(FromUi::Steps)
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
                match crate::looper_page::STRIP_TO_BUTTON.get(i).copied()? {
                    Some(b) => Some(FromUi::Looper(b)),
                    None => Some(FromUi::Time(TimeAction::Transport(
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
                self.refresh_qlinks();
                first
            }
            ("header", EventKind::Select(mpfree_ui::widgets::MIXER_ICON)) => {
                Some(FromUi::MixerIcon)
            }
            (play_page::SYNTH_PADS_HEAD, EventKind::Tab(i)) => {
                Some(FromUi::PadsLayout { in_key: i == 1 })
            }
            ("options", EventKind::Select(i)) => Some(FromUi::MenuSelect(i)),
            ("options", EventKind::Step { index, delta }) => Some(FromUi::MenuStep(index, delta)),
            ("guide", EventKind::Tab(i)) => Some(FromUi::GuideButton(i)),
            ("header", EventKind::Select(i)) if Some(i) == self.arp_chip => Some(FromUi::ArpChip),
            (w, _) if w.starts_with("arp-ws-") => {
                // The editor changes at once; the app hears the new pattern.
                let a = self.workspace.action(&ev);
                let playing = self.playing_step();
                self.workspace.update(&mut self.ui, playing);
                self.refresh_hint();
                a.map(|a| FromUi::Workspace(Box::new(a)))
            }
            (w, _) if w.starts_with("arp-") => self
                .arp_panel
                .action(&ev, &self.arp, &mut self.qlink_focus)
                .map(FromUi::Arp),
            (time_view::TRANSPORT | time_view::CLIPS | time_view::RECORD | time_view::TAKES, _) => {
                time_view::action(&ev, &self.time).map(FromUi::Time)
            }
            _ => None,
        }
    }

    /// Advance animations by `dt` seconds.
    pub fn tick(&mut self, dt: f32) {
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
        dev.check_profile(&MPC_ONE)
            .map_err(|e| format!("{path}: {e}"))?;
        TouchIn::from_source(Box::new(dev), size)
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
        })
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
}

/// Run the pad player's screen until `Quit` arrives or the sender goes away.
/// Spawn it before the audio thread raises its own priority, so it stays at
/// normal priority.
///
/// `open_touch` opens the touchscreen. It is tried at once and then every
/// second while there is none (the driver may not have probed yet at power-on,
/// or a read failed), so touch comes back by itself.
pub fn spawn(
    mut player: Player,
    mut fb: Option<FbTarget>,
    mut open_touch: Option<OpenTouch>,
    rx: Receiver<ToUi>,
    tx: SyncSender<FromUi>,
) -> io::Result<JoinHandle<ScreenReport>> {
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
            let mut report = ScreenReport::default();
            let mut total_ms = 0.0;
            let mut last_frame = Instant::now() - Duration::from_secs(1);
            let mut window = (Instant::now(), 0u32, 0f64);
            // Debug mode: /proc and /sys every 2 s, and a perf: line every 30 s.
            let mut perf = crate::perf::Sampler::new();
            let (mut perf_at, mut perf_logged): (Option<Instant>, Option<Instant>) = (None, None);
            // The Processes page: /proc once a second, only while it shows (a
            // fresh table each time the page opens, so CPU starts again).
            let mut procs: Option<(mpfree_prof::procs::ProcTable, Instant)> = None;
            // The CPU by engine page: the audio loop's numbers every 250 ms.
            let mut engines_at: Option<Instant> = None;
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
                    player
                        .set_screen_row(format!("{fps:.0} fps, slowest frame {:.1} ms", window.2));
                    window = (Instant::now(), 0, 0.0);
                }
            }
            report.mean_ms = if report.frames > 0 {
                total_ms / report.frames as f64
            } else {
                0.0
            };
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
              "[    3.871] mpfree-hello[211]: loading drum kits: 8 kits, 39.0 MB",
              "[    4.402] kernel: rk_gmac-dwmac ff290000.ethernet eth0: Link is Up - 1Gbps/Full",
              "[    5.990] mpfree-hello[211]: session accepted; sending Akai's setup exactly as the app did",
              "[    7.315] mpfree-hello[211]: SCHED_FIFO priority 70"] {
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
/// as hands are (mostly early, as mockup I), for the Drum view's timing.
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
        // Bar 10: late in the timing view's two bars, as mockup I.
        PlayView::Drum => 26,
    };
    sample_pattern(&mut p, at);
    // The kits' projects (Boom Bap's pads with sounds behind them), so the
    // Tune and Decay Q-Links read the pads' own settings.
    p.apply(ToUi::SampleKits {
        projects: vec![mpfree_sampler::model::Project::new(), boom_bap_project()],
        rate: 44_100,
    });
    p.play.last_pad = if v == PlayView::Drum { 4 } else { 0 };
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
            p.play.xy = (0.68, 0.62, true);
        }
        PlayView::Beats => {
            p.play.selected = Some((0, 6));
            p.play.throw(2, true);
            p.apply(ToUi::QLinkTouch(0, true));
        }
        PlayView::Loop => {
            p.play.throw(2, true);
            p.play.xy = (0.68, 0.62, true);
        }
        PlayView::Drum => {
            sample_hits(&mut p);
            p.play.throw(2, true);
            p.apply(ToUi::QLinkTouch(2, true));
            p.play.xy = (0.68, 0.62, true);
        }
    }
    if let Some(xy) = p.ui.get_mut::<XyPad>(play_page::XY) {
        (xy.x, xy.y, xy.on) = p.play.xy;
        if xy.on {
            // The finger's way up to where it is, as the mockups draw it.
            let (x, y, _) = p.play.xy;
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
            Effect::CalibrateTouch => p.apply(ToUi::CalibrateTouch),
            Effect::Note(l) => p.apply(ToUi::Line(l)),
            Effect::ClipsWheel { delta, scroll } => p.apply(ToUi::ClipsWheel { delta, scroll }),
            Effect::SynthWheel(d) => p.apply(ToUi::SynthWheel(d)),
            Effect::Map(m) => p.apply(ToUi::Map(Box::new(m))),
            Effect::SynthPage(d) => p.apply(ToUi::SynthPage(d)),
            Effect::SampleWheel(d) => p.apply(ToUi::SampleWheel(d)),
            Effect::SamplePad(d) => p.apply(ToUi::SamplePad(d)),
            Effect::SampleUndo { redo } => p.apply(ToUi::SampleUndo(redo)),
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
            | Effect::Mixer(_) => {}
            Effect::MixerTurn { knob, delta } => p.apply(ToUi::MixerQLink(knob, delta)),
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

/// Every screen of the pad player with sample contents, as (name, player
/// showing it), for previews: loading, play, debug, console, prefs, options,
/// the guide, and the arp's quick band, panel and pattern workspace (each
/// with the real app and engine behind it).
/// The Looper tab with a real loop: two bars of the pads (a kick and hats)
/// and an overdub (a bass line), recorded by the looper on a transport.
fn sample_looper(fonts: Fonts) -> Result<Player, String> {
    use crate::looping::{LoopHost, PERIOD};
    use mpfree_looper::Command as L;
    use mpfree_transport::{Block, Command as T, Transport};
    use std::f32::consts::TAU;
    let mut p = sample_player(fonts);
    p.apply(ToUi::Show(View::Looper));
    let mut h = LoopHost::new(44_100, 20);
    let mut t = Transport::new(44_100, 120.0).map_err(|e| format!("{e:?}"))?;
    let mut b = Block::new();
    h.command(L::SetBars(Some(2)));
    h.command(L::SetLatency(0));
    h.command(L::Record);
    t.command(0, T::Play);
    const BEAT: u64 = 22_050;
    const LOOP: u64 = 8 * BEAT;
    let mut wave = None;
    // Two passes, an overdub on the third, and into the fourth.
    let periods = (3 * LOOP + 5 * BEAT) / PERIOD as u64;
    for i in 0..periods {
        let start = i * PERIOD as u64;
        if start <= 2 * LOOP && 2 * LOOP < start + PERIOD as u64 {
            h.command(L::Overdub);
        }
        if start <= 3 * LOOP && 3 * LOOP < start + PERIOD as u64 {
            h.command(L::Overdub);
        }
        if !t.process(PERIOD as u32, &mut b) {
            return Err("the transport refused a period".into());
        }
        let (mut l, mut r) = ([0.0f32; PERIOD], [0.0f32; PERIOD]);
        for f in 0..PERIOD {
            let n = start + f as u64;
            let in_beat = (n % BEAT) as f32 / 44_100.0;
            let half = (n % (BEAT / 2)) as f32 / 44_100.0;
            let mut v = if n < LOOP {
                // The first pass: a kick on the beats, a hat between.
                let kick = (TAU * (50.0 + 90.0 * (-in_beat * 30.0).exp()) * in_beat).sin()
                    * (-in_beat * 9.0).exp();
                let noise = ((n.wrapping_mul(2_654_435_761) >> 7) % 2000) as f32 / 1000.0 - 1.0;
                let hat = if (n / (BEAT / 2)) % 2 == 1 {
                    noise * (-half * 60.0).exp() * 0.25
                } else {
                    0.0
                };
                0.7 * kick + hat
            } else if (2 * LOOP..3 * LOOP).contains(&n) {
                // The overdub: a bass note on each beat, up a fifth on 3 and 4.
                let note = if (n / BEAT) % 4 >= 2 { 82.4 } else { 55.0 };
                0.35 * (TAU * note * in_beat).sin() * (-in_beat * 3.0).exp()
            } else {
                0.0
            };
            v = v.clamp(-1.0, 1.0);
            (l[f], r[f]) = (v, v);
        }
        h.take_pads(&l, &r);
        h.period(&b, &mut l, &mut r);
        h.changes(String::new);
        if let Some(w) = h.wave_step(i, 344) {
            wave = Some(w);
        }
    }
    p.apply(ToUi::LoopWave(wave.ok_or("no wave")?));
    let mut v = h.view(120.0);
    v.meter_db = [-6.5, -6.5];
    p.apply(ToUi::Looper(Box::new(v)));
    Ok(p)
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
    // The Play page's views, as mockups F to I.
    for (name, v) in [
        ("play-all-loop", PlayView::All),
        ("play-beats", PlayView::Beats),
        ("play-loop", PlayView::Loop),
        ("play-drum", PlayView::Drum),
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
    mixer.apply(ToUi::Show(View::Mixer));
    mixer.apply(ToUi::Mixer(Box::new(st)));
    out.push(("mixer", mixer));
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
            ("mpfree-hello (812)".into(), 31.5),
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
    // round 6's (synth worst near 20 %, sampler near 24 %).
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
        let (s, y, l, t, m) = (
            spike(500, 690, 30),
            spike(400, 570, 55),
            spike(1000, 40, 6),
            spike(900, 25, 4),
            spike(800, 18, 3),
        );
        for (i, d) in [
            (crate::engines::SAMPLER, s),
            (crate::engines::SYNTH, y),
            (crate::engines::LOOPER, l),
            (crate::engines::TIME, t),
            (crate::engines::MIX, m),
            (
                crate::engines::PERIOD,
                s + y + l + t + m + Duration::from_micros(20),
            ),
        ] {
            probes.add(i, d);
        }
        if k == 1_000 {
            probes.publish();
        }
    }
    probes.publish();
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
            row(812, "mpfree-hello", 31.5, 60, 6, "FIFO 70"),
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
    // Prefs > System info, with fixed rows (the host's own would differ).
    let mut system = sample_player(fonts());
    system.apply(ToUi::Show(View::System));
    system.set_system(crate::sysinfo::sample());
    out.push(("system-info", system));
    match sample_clips(fonts()) {
        Ok(p) => out.push(("clips", p)),
        Err(e) => eprintln!("clips sample: {e}"),
    }
    match sample_looper(fonts()) {
        Ok(p) => out.push(("looper", p)),
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
    // The Pads tab: A5 copied onto A2, A5 picked again.
    let mut pads = sample_sampler(fonts());
    pads.on_event(UiEvent {
        widget: mpfree_sampler_ui::wave_tab::BAR.into(),
        kind: EventKind::Tab(mpfree_sampler_ui::pads_tab::TAB),
    });
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
    out.push(("sample-pads", pads));
    // The Chop tab on the same pad: chopped at its hits, slice 3 picked.
    let mut chop = sample_sampler(fonts());
    chop.on_event(UiEvent {
        widget: mpfree_sampler_ui::wave_tab::BAR.into(),
        kind: EventKind::Tab(mpfree_sampler_ui::chop_tab::TAB),
    });
    chop.apply(ToUi::SampleQLink(0, 2));
    chop.take_actions();
    out.push(("sample-chop", chop));
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
            "Clips · slots 2–7 of 16"
        );
        assert!(p.take_actions().is_empty());
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
        assert_eq!(tap(&mut p, seg(3)), None);
        assert_eq!(p.page(), "Play · Drum");
        assert!(p.ui.rect_of(play_page::TIMING).is_some());
        assert_eq!(names(&p), ["Tune", "Decay", "Swing", "FX mix"]);
        // Another tab and back: the view is kept.
        p.apply(ToUi::Show(View::Synth));
        p.apply(ToUi::Show(View::Play));
        assert_eq!(p.page(), "Play · Drum");
        // Each view fits under the transport card, left of the master.
        for v in PlayView::ALL {
            tap(&mut p, seg(v.index()));
            let pads = p.ui.rect_of(play_page::PADS).unwrap();
            assert!(
                pads.y > 168 && pads.bottom() <= 750 && pads.right() <= 1010,
                "{v:?}: {pads:?}"
            );
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
    fn the_drum_view_shows_your_timing() {
        let p = sample_play(Fonts::embedded(), PlayView::Drum);
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
    fn the_pads_heading_switches_the_synth_to_in_key() {
        let mut p = sample_player(Fonts::embedded());
        let synth_kit = sample_kits().len();
        // A kit: its bank and name, no switch.
        let m = p.ui.get::<MiniPads>(play_page::PADS).unwrap();
        assert_eq!(
            (m.title.as_str(), m.right.as_str(), m.synth),
            ("Pads · Bank A", "Classic House", None)
        );
        // The synth with a scale: the switch, on Play and on the Synth tab.
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
        let r = p.ui.rect_of(play_page::PADS).unwrap();
        let [_, in_key] =
            p.ui.get::<MiniPads>(play_page::PADS)
                .unwrap()
                .parts(r)
                .1
                .unwrap();
        assert_eq!(
            p.touch(1, Touch::Down, in_key.x + 10, in_key.y + 10),
            Some(FromUi::PadsLayout { in_key: true })
        );
        p.touch(1, Touch::Up, in_key.x + 10, in_key.y + 10);
        p.apply(ToUi::Show(View::Synth));
        let head = p.ui.rect_of(play_page::SYNTH_PADS_HEAD).unwrap();
        assert_eq!(
            p.ui.get::<PadsBar>(play_page::SYNTH_PADS_HEAD)
                .unwrap()
                .title,
            "Pads · Bank A"
        );
        let [chromatic, _] = PadsBar::halves(head);
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
        // The Play page (mockup F): no big pads; the transport card on
        // top, the master meter and the Q-Link cards at the right edge,
        // beside the knobs.
        let pads = p.ui.rect_of(play_page::PADS).unwrap();
        assert!(pads.w == 270 && pads.h > 250, "{pads:?}");
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
        let pads = p.ui.rect_of("pads").unwrap();
        assert_eq!(
            p.touch(2, Touch::Down, pads.x + 20, pads.bottom() - 20),
            Some(FromUi::Pad { pad: 0, down: true })
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
        let pads = p.ui.rect_of("pads").unwrap();
        // Pad 1 sits bottom left.
        assert_eq!(
            p.touch(1, Touch::Down, pads.x + 20, pads.bottom() - 20),
            Some(FromUi::Pad { pad: 0, down: true })
        );
        assert_eq!(
            p.touch(1, Touch::Up, pads.x + 20, pads.bottom() - 20),
            Some(FromUi::Pad {
                pad: 0,
                down: false
            })
        );
        let header = p.ui.rect_of("header").unwrap();
        let tab1 = p.ui.get::<Header>("header").unwrap().tab_rect(header, 1);
        // The second tab: the app decides; the page stays until it says.
        assert_eq!(
            p.touch(2, Touch::Down, tab1.x + 20, tab1.y + 20),
            Some(FromUi::Tab(1))
        );
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

    fn events_log(p: &Player) -> Vec<String> {
        p.ui.get::<LogPanel>("events")
            .unwrap()
            .lines()
            .map(|(l, _)| l.clone())
            .collect()
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
            4,
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
        // A pad without a sound shows the Pads tab; - / + find the next sound.
        p.apply(ToUi::PadDown {
            pad: 1,
            velocity: 90,
        });
        assert_eq!(p.page(), sample_page::PADS_PAGE);
        assert_eq!(
            p.ui.get::<mpfree_sampler_ui::widgets::PadBar>(mpfree_sampler_ui::pads_tab::BAR)
                .unwrap()
                .title,
            "Pad A2 · no sound"
        );
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
        assert_eq!(p.page(), sample_page::PADS_PAGE, "A8 has no sound");
        p.apply(ToUi::PadDown { pad, velocity: 100 });
        assert_eq!(p.page(), sample_page::CHOP_PAGE);
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
        assert_eq!(p.page(), sample_page::PADS_PAGE);
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
        // The pad bar's first tab opens Pads on the same pad.
        ev(
            &mut p,
            mpfree_sampler_ui::wave_tab::BAR,
            EventKind::Tab(pads_tab::TAB),
        );
        assert_eq!(
            (p.page(), p.sample.editor()),
            (sample_page::PADS_PAGE, sample_page::Editor::Pads)
        );
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
        // With a sound picked, the bar opens the other tabs on it.
        ev(&mut p, pads_tab::MAP, EventKind::Select(pad));
        ev(
            &mut p,
            pads_tab::BAR,
            EventKind::Tab(mpfree_sampler_ui::play_tab::TAB),
        );
        assert_eq!(p.page(), sample_page::PLAY_PAGE);
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
            chips(&p).contains(&"Kit 1 of 7 · Bank A".to_string()),
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
        // The Synth tab is the third; the app decides, then the page shows.
        let header = p.ui.rect_of("header").unwrap();
        let synth_tab = p.ui.get::<Header>("header").unwrap().tab_rect(header, 2);
        assert_eq!(
            p.touch(1, Touch::Down, synth_tab.x + 20, synth_tab.y + 20),
            Some(FromUi::Tab(2))
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
        // A Q-Link turns its knob on screen at once and sends the change.
        assert!(p.take_actions().is_empty());
        p.apply(ToUi::SynthQLink(0, 5));
        let acts = p.take_actions();
        assert!(
            matches!(acts.as_slice(), [FromUi::Synth(SynthAction::Set { .. })]),
            "{acts:?}"
        );
        p.apply(ToUi::SynthWheel(-3));
        assert_eq!(p.take_actions().len(), 1);
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
        p.apply(ToUi::Status(s));
        assert!(p
            .ui
            .get::<Label>("hint")
            .unwrap()
            .text
            .contains("NOTE REPEAT: repeat"));
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
        let pads = p.ui.rect_of("pads").unwrap();
        let mut app = sample_app();
        let fx = app.button(NOTE_REPEAT, true, 0.0);
        apply_effects(&mut p, fx);
        assert_eq!(p.page(), QUICK_PAGE);
        // A slim strip at the top of the view's first panel; the pads, the
        // transport, the master bar and the Q-Links stay put, and the cards
        // turn to the arp's.
        assert_eq!(p.ui.rect_of("pads").unwrap(), pads);
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
        let panel_pads = p.ui.rect_of("pads").unwrap();
        assert_eq!(
            (panel_pads.x, panel_pads.w),
            (pads.x, pads.w),
            "the panel takes the left; the pads stay right"
        );
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
}
