//! Hello MPC's screens, built from mpfree-ui widgets.
//!
//! - The console (the one-shot steps): a header and the step's log.
//! - The pad player (`play`, and `app` from power-on): a loading page with a
//!   progress bar; Play (pads lit in the kit's colour, the kit list, the level
//!   slider, timing); Debug (every MIDI message in and out, events, timing);
//!   Console (the system log, raw); Prefs and each page's options (MENU); and
//!   the button guide. Tabs in the header switch between the main pages.
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

use crate::app::{GuideView, MenuView, PadLevel, Settings, Status, View, TABS};
use crate::calibrate_page::CalibrateView;
use crate::touchlab_page::{TouchLabView, TouchPage};
use mpfree_touch::mt::Phase;
use mpfree_touch::pointer::PointerEvent;
use mpfree_touch::profile::MPC_ONE;
use mpfree_touch::touchlab::reader_line;
use mpfree_touch_evdev::{EvdevFile, TouchReader, TouchSource};
use mpfree_ui::fb::FbTarget;
use mpfree_ui::widgets::{Card, Chip, Fader, Header, Label, ListItem, ListView, Loading, LogPanel, OptionList,
                         OptionRow, PadGrid, StatList};
use mpfree_ui::{Align, Color, EventKind, Fonts, Image, SoftCanvas, Theme, Tone, Touch, Ui, UiEvent};
use std::collections::VecDeque;
use std::io::{self, Write};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub const CONSOLE_LAYOUT: &str = "col(header:60, log:*, hint:26)";
pub const PLAY_LAYOUT: &str = "col(header:60, row(pads:*, col(kits:*, level:96, timing:232):460):*, hint:26)";
pub const DEBUG_LAYOUT: &str = "col(header:60, row(midi:*, col(events:*, timing:232):460):*, hint:26)";
pub const SYSLOG_LAYOUT: &str = "col(header:60, syslog:*, hint:26)";
pub const OPTIONS_LAYOUT: &str = "col(header:60, row(options:*, help:470):*, hint:26)";
pub const GUIDE_LAYOUT: &str = "col(header:60, row(guide:*, found:400):*, hint:26)";

const CONSOLE_HINT: &str = "Switch the MPC off with its power switch to return to the stock firmware";
const SHIFT_HINT: &str =
    "SHIFT +   FULL LEVEL: Half Level   ·   NOTE REPEAT: Latch   ·   MENU: Prefs   ·   STOP: console";
const SHIFT_MENU_HINT: &str = "SHIFT +   wheel: change the value   ·   MENU: Prefs   ·   STOP: console";
const OPTIONS_HINT: &str =
    "Wheel or tap: pick a row   ·   - / +, SHIFT + wheel or the arrows: change it   ·   MENU: close";
const GUIDE_HINT: &str = "STOP: skip this button   ·   The pads still play while the guide is open";
const LOADING_HINT: &str = "Tap the screen for the console";

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
    /// Open the Touch lab page; any `Show` closes it again.
    TouchLab,
    /// Open the Calibrate touch page; any `Show` closes it again.
    CalibrateTouch,
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
    /// The widget of the touch page on screen (Touch lab, Calibrate touch),
    /// over `view`, which it returns to.
    touch_page: Option<&'static str>,
}

impl Player {
    /// `backdrop`: the picture behind the loading card (the startup screen's
    /// last frame), or None for the built-in one.
    pub fn new(fonts: Fonts, size: (i32, i32), timed: bool, backdrop: Option<Arc<Image>>) -> Player {
        let mut ui = Ui::new(Theme::dark(), fonts, size.0, size.1);
        ui.add("header", Header::new(&TABS));
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
        ui.add_full_page("Loading", "loading").expect("loading layout");
        ui.add_page("Play", PLAY_LAYOUT).expect("play layout");
        ui.add_page("Debug", DEBUG_LAYOUT).expect("debug layout");
        ui.add_page("Console", SYSLOG_LAYOUT).expect("console layout");
        ui.add_page("Prefs", OPTIONS_LAYOUT).expect("prefs layout");
        ui.add_page("Options", OPTIONS_LAYOUT).expect("options layout");
        ui.add_page("Guide", GUIDE_LAYOUT).expect("guide layout");
        ui.add("touchlab", TouchLabView::new());
        ui.add_full_page("Touch lab", "touchlab").expect("touch lab layout");
        ui.add("calibrate", CalibrateView::new());
        ui.add_full_page("Calibrate touch", "calibrate").expect("calibrate layout");
        let _ = ui.show("Loading");
        let status = Status { shift: false, pad_level: PadLevel::Off, repeat: false, latched: false, rate: "1/16",
                              bpm: 120.0 };
        let mut p = Player { ui, kits: Vec::new(), kit: 0, midi: 0, midi_held_back: 0,
                             timing: Timing { last_ms: f32::NAN, ..Timing::default() }, screen_row: "-".to_string(),
                             timed, ready: false, view: View::Play, status, settings: Settings::default(),
                             session: ("Starting".to_string(), Tone::Dim),
                             console_from_lines: timed, console_backlog: VecDeque::new(), touch_page: None };
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
        let Some(kit) = self.kits.get(k).cloned() else { return };
        self.kit = k;
        if let Some(g) = self.ui.get_mut::<PadGrid>("pads") {
            for (i, pad) in g.pads.iter_mut().enumerate() {
                let label = kit.pads.get(i).cloned().flatten();
                pad.color = label.as_ref().map(|_| Color::from_pad(kit.colour));
                pad.label = label.unwrap_or_default();
                pad.level = 0.0;
                pad.held = false;
            }
        }
        if let Some(l) = self.ui.get_mut::<ListView>("kits") {
            l.selected = k;
        }
        self.refresh_chips();
    }

    /// The header's chips, right to left: session, kit, level, tempo, then
    /// the modes that are on, and the time left in the timed step.
    fn refresh_chips(&mut self) {
        let mut chips = vec![Chip::new(self.session.0.clone(), self.session.1)];
        if !self.kits.is_empty() {
            chips.push(Chip::new(format!("Kit {} of {}", self.kit + 1, self.kits.len()), Tone::Accent));
        }
        // The level in dB is on the slider and the tempo only matters while
        // pads repeat, so the header keeps to the chips that change how the
        // pads play: four tabs leave room for about five chips.
        let s = &self.status;
        if s.latched {
            chips.push(Chip::new(format!("Latch {} · {:.0} BPM", s.rate, s.bpm), Tone::Warn));
        } else if s.repeat {
            chips.push(Chip::new(format!("Repeat {} · {:.0} BPM", s.rate, s.bpm), Tone::Warn));
        }
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
        let text = if self.status.shift && self.ready && matches!(self.view, View::Prefs | View::Options) {
            SHIFT_MENU_HINT.to_string()
        } else if self.status.shift && self.ready {
            SHIFT_HINT.to_string()
        } else {
            match self.view {
                View::Play => format!("Wheel or tap: kit   ·   Q-Link 1 or slider: level   ·   NOTE REPEAT: repeat   ·   \
                                       MENU: options   ·   {stop}"),
                View::Debug => format!("Every MIDI message in and out, newest at the bottom   ·   MENU: options   ·   {stop}"),
                View::Console => "The system log, as it comes   ·   MENU: pause or clear   ·   SHIFT + STOP: back"
                    .to_string(),
                View::Prefs | View::Options => OPTIONS_HINT.to_string(),
                View::Guide => GUIDE_HINT.to_string(),
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
        self.settings = s;
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
            ToUi::Kits(kits) => self.set_kits(kits),
            ToUi::PadDown { pad, velocity } => {
                if let Some(g) = self.ui.get_mut::<PadGrid>("pads") {
                    g.hit(pad, velocity);
                }
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
            ToUi::Kit(k) => self.set_kit(k),
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
            }
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
            ToUi::TouchLab => self.open_touch_page("touchlab", "Touch lab"),
            ToUi::CalibrateTouch => self.open_touch_page("calibrate", "Calibrate touch"),
            ToUi::Quit => {}
        }
    }

    fn open_touch_page(&mut self, widget: &'static str, page: &str) {
        self.ready = true;
        self.close_touch_page();
        self.touch_page = Some(widget);
        self.with_touch_page(|p| p.open());
        let _ = self.ui.show(page);
        self.print_touch_page_lines();
    }

    /// The open touch page, to read (nothing is redrawn).
    fn touch_page_ref(&self) -> Option<&dyn TouchPage> {
        match self.touch_page? {
            "touchlab" => self.ui.get::<TouchLabView>("touchlab").map(|p| p as &dyn TouchPage),
            "calibrate" => self.ui.get::<CalibrateView>("calibrate").map(|p| p as &dyn TouchPage),
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

    fn show_view(&mut self, v: View) {
        self.close_touch_page();
        let page = match v {
            View::Play => "Play",
            View::Debug => "Debug",
            View::Console => "Console",
            View::Prefs => "Prefs",
            View::Options => "Options",
            View::Guide => "Guide",
        };
        let _ = self.ui.show(page);
        self.view = v;
        if let Some(h) = self.ui.get_mut::<Header>("header") {
            if let Some(t) = v.tab() {
                h.active = t;
            }
        }
        self.refresh_hint();
    }

    /// A touch: what it means for the app, if anything. Before the app is
    /// ready, the pages are switched here (loading and console only).
    pub fn touch(&mut self, finger: u64, phase: Touch, x: i32, y: i32) -> Option<FromUi> {
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
        if self.touch_page.is_some() {
            // A touch page takes every finger and means nothing to the app.
            let _ = self.ui.touch(finger, phase, x, y);
            self.print_touch_page_lines();
            if self.touch_page_ref().is_some_and(|p| p.wants_exit()) {
                self.show_view(self.view);
            }
            return None;
        }
        let ev = self.ui.touch(finger, phase, x, y)?;
        self.on_event(ev)
    }

    /// A touch meant something: hand it to the audio thread, which decides.
    pub fn on_event(&mut self, ev: UiEvent) -> Option<FromUi> {
        match (ev.widget.as_str(), ev.kind) {
            ("header", EventKind::Tab(i)) => {
                // The header moved its own highlight; the page follows when the app says.
                if let (Some(h), Some(t)) = (self.ui.get_mut::<Header>("header"), self.view.tab()) {
                    h.active = t;
                }
                Some(FromUi::Tab(i))
            }
            ("pads", EventKind::Pad { index, down }) => Some(FromUi::Pad { pad: index, down }),
            ("kits", EventKind::Select(k)) => Some(FromUi::Kit(k)),
            ("level", EventKind::Value(v)) => Some(FromUi::Level(v)),
            ("options", EventKind::Select(i)) => Some(FromUi::MenuSelect(i)),
            ("options", EventKind::Step { index, delta }) => Some(FromUi::MenuStep(index, delta)),
            ("guide", EventKind::Tab(i)) => Some(FromUi::GuideButton(i)),
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
    p.apply(ToUi::Kits(sample_kits()));
    p.apply(ToUi::Session("Akai session".into(), Tone::Good));
    p.apply(ToUi::Kit(0));
    p.apply(ToUi::Level { db: -18.0, value: 0.79 });
    p.apply(ToUi::Status(Status { shift: false, pad_level: PadLevel::Full, repeat: true, latched: false,
                                  rate: "1/16", bpm: 120.0 }));
    for (pad, v) in [(0usize, 112u8), (5, 87), (10, 64)] {
        p.apply(ToUi::PadDown { pad, velocity: v });
    }
    p.apply(ToUi::PadUp(10));
    if let Some(g) = p.ui.get_mut::<PadGrid>("pads") {
        g.pads[10].level = 0.5;
    }
    p.apply(ToUi::Timing(Timing { hits: 42, periods: 10_335, underruns: 0, voices: 3, peak_db: -9.0, last_ms: 7.9,
                                  min_ms: 6.2, mean_ms: 7.8, max_ms: 9.6, seconds_left: None, periods_queued: 3 }));
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

/// Every screen of the pad player with sample contents, as (name, player
/// showing it), for previews: loading, play, debug, console, prefs, options
/// and the guide.
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
    // The sample MENU and NOTE REPEAT notes are made up: the real ones are
    // still unknown and the guide learns them on the device.
    const MENU: u8 = 1;
    let mut app = crate::app::App::new(44_100, false);
    app.map.menu = Some(MENU);
    app.map.note_repeat = Some(2);
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
    let mut guide = sample_player(fonts());
    let mut g = crate::app::App::new(44_100, false);
    let mut fx = g.start();
    fx.extend(g.button(123, true, 0.0));
    for e in fx {
        match e {
            crate::app::Effect::Guide(v) => guide.apply(ToUi::Guide(v)),
            crate::app::Effect::Show(v) => guide.apply(ToUi::Show(v)),
            _ => {}
        }
    }
    out.push(("guide", guide));
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
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_touch::mt::InputEvent;
    use std::io::ErrorKind;

    #[test]
    fn layouts_fit_the_screen() {
        let mut p = Player::new(Fonts::embedded(), SCREEN, false, None);
        assert_eq!(p.ui.rect_of("loading"), Some(mpfree_ui::Rect::new(0, 0, 1280, 800)));
        p.apply(ToUi::Show(View::Play));
        let pads = p.ui.rect_of("pads").unwrap();
        assert!(pads.w > 700 && pads.h > 600, "{pads:?}");
        let kits = p.ui.rect_of("kits").unwrap();
        assert!(kits.h >= 300, "{kits:?}");
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
        assert_eq!(p.touch(2, Touch::Down, pads.x + 10, pads.bottom() - 10), Some(FromUi::Pad { pad: 0, down: true }));
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
        // Calibrate touch is the row before the Touch lab; + on it opens the page.
        fx.extend(app.step_item(app_rows(&app) - 2, 1));
        assert_eq!(fx.last(), Some(&crate::app::Effect::CalibrateTouch), "{fx:?}");
        for e in fx {
            match e {
                crate::app::Effect::Menu(m) => p.apply(ToUi::Menu(m)),
                crate::app::Effect::Show(v) => p.apply(ToUi::Show(v)),
                crate::app::Effect::CalibrateTouch => p.apply(ToUi::CalibrateTouch),
                _ => {}
            }
        }
        assert_eq!((p.page(), p.touch_page()), ("Calibrate touch", Some("calibrate")));
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
        let mut app = crate::app::App::new(44_100, false);
        app.map.menu = Some(1);
        app.map.note_repeat = Some(2);
        app.start();
        for fx in app.button(1, true, 0.0) {
            match fx {
                crate::app::Effect::Menu(m) => p.apply(ToUi::Menu(m)),
                crate::app::Effect::Show(v) => p.apply(ToUi::Show(v)),
                _ => {}
            }
        }
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
        let chips: Vec<String> = p.ui.get::<Header>("header").unwrap().chips.iter().map(|c| c.text.clone()).collect();
        assert!(chips.contains(&"Latch 1/8 · 96 BPM".to_string()), "{chips:?}");
        assert!(!chips.iter().any(|c| c.ends_with(" dB")), "the level is on the slider: {chips:?}");
        s.shift = false;
        p.apply(ToUi::Status(s));
        assert!(p.ui.get::<Label>("hint").unwrap().text.contains("NOTE REPEAT: repeat"));
        p.apply(ToUi::PadFlash { pad: 3, velocity: 90 });
        let g = p.ui.get::<PadGrid>("pads").unwrap();
        assert!(!g.pads[3].held && g.pads[3].level == 1.0);
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
