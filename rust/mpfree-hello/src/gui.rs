//! Hello MPC's screens, built from mpfree-ui widgets.
//!
//! - The console (every step but `play`): a header and the step's log.
//! - The pad player (`play`): a Play page (pads lit in the kit's colour, the
//!   kit list, the level slider, timing) and a Debug page (every MIDI message
//!   in and out, events, timing). Tabs in the header switch between them.
//!
//! The pad player's screen runs in its own thread at normal priority, so the
//! audio thread never waits for drawing. The audio thread sends it `ToUi`
//! messages; it sends back `FromUi` actions from touches. It also prints every
//! line to stdout, which reaches the workstation's terminal over SSH.

use mpfree_touch::mt::{InputEvent, MtDecoder, Phase, Step};
use mpfree_touch::pointer::{AxisMap, Orientation, RawRange};
use mpfree_ui::fb::FbTarget;
use mpfree_ui::widgets::{Chip, Fader, Header, Label, ListItem, ListView, LogPanel, PadGrid, StatList};
use mpfree_ui::{Align, Color, EventKind, Fonts, SoftCanvas, Theme, Tone, Touch, Ui, UiEvent};
use std::fs::{File, OpenOptions};
use std::io::{self, ErrorKind, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub const CONSOLE_LAYOUT: &str = "col(header:60, log:*, hint:26)";
pub const PLAY_LAYOUT: &str = "col(header:60, row(pads:*, col(kits:*, level:96, timing:206):460):*, hint:26)";
pub const DEBUG_LAYOUT: &str = "col(header:60, row(midi:*, col(events:*, timing:206):460):*, hint:26)";

const PLAY_HINT: &str = "Wheel or tap: kit   ·   Q-Link 1 or slider: level   ·   Pads or screen: play   ·   STOP: end";
const DEBUG_HINT: &str = "Every MIDI message in and out, newest at the bottom   ·   STOP: end";
const CONSOLE_HINT: &str = "Switch the MPC off with its power switch to return to the stock firmware";

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
    pub seconds_left: u64,
}

pub enum ToUi {
    /// A line for the terminal and the Events log.
    Line(String),
    /// A line for the Events log only (not printed).
    Show(String),
    /// A MIDI message in or out, as one line (also printed).
    Midi(String),
    PadDown { pad: usize, velocity: u8 },
    PadUp(usize),
    Kit(usize),
    /// Level in dB and as 0.0-1.0 of the slider.
    Level { db: f32, value: f32 },
    Session(String, Tone),
    Timing(Timing),
    Quit,
}

#[derive(Clone, Debug, PartialEq)]
pub enum FromUi {
    Pad { pad: usize, down: bool },
    Kit(usize),
    /// Slider position, 0.0-1.0.
    Level(f32),
}

pub struct Player {
    pub ui: Ui,
    kits: Vec<KitView>,
    kit: usize,
    midi: u64,
    timing: Timing,
    screen_row: String,
}

impl Player {
    pub fn new(fonts: Fonts, kits: Vec<KitView>, size: (i32, i32)) -> Player {
        let mut ui = Ui::new(Theme::dark(), fonts, size.0, size.1);
        ui.add("header", Header::new(&["Play", "Debug"]));
        ui.add("pads", PadGrid::new(16, 4));
        let mut list = ListView::new("Kits");
        list.items = kits.iter().map(|k| ListItem {
            text: k.name.clone(),
            detail: format!("{} pads", k.pads.iter().filter(|p| p.is_some()).count()),
            color: Some(Color::from_pad(k.colour)),
        }).collect();
        ui.add("kits", list);
        ui.add("level", Fader::new("Level · Q-Link 1"));
        ui.add("timing", StatList::new("Timing"));
        ui.add("midi", LogPanel::new("MIDI in and out", true, 300));
        ui.add("events", LogPanel::new("Events", false, 200));
        ui.add("hint", Label::new(PLAY_HINT, Tone::Dim, Align::Center));
        ui.add_page("Play", PLAY_LAYOUT).expect("play layout");
        ui.add_page("Debug", DEBUG_LAYOUT).expect("debug layout");
        let mut p = Player { ui, kits, kit: 0, midi: 0, timing: Timing { last_ms: f32::NAN, ..Timing::default() },
                             screen_row: "-".to_string() };
        p.set_kit(0);
        p.set_session("Starting", Tone::Dim);
        p.show_timing();
        p
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
        self.chip(1, format!("Kit {} of {}", k + 1, self.kits.len()), Tone::Accent);
    }

    fn set_session(&mut self, text: &str, tone: Tone) {
        self.chip(0, text.to_string(), tone);
    }

    /// Chips, right to left: 0 session, 1 kit, 2 level, 3 time left.
    fn chip(&mut self, i: usize, text: String, tone: Tone) {
        if let Some(h) = self.ui.get_mut::<Header>("header") {
            while h.chips.len() <= i {
                h.chips.push(Chip::new("", Tone::Dim));
            }
            h.chips[i] = Chip::new(text, tone);
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
            ("Screen".to_string(), self.screen_row.clone(), Tone::Dim),
        ];
        if let Some(s) = self.ui.get_mut::<StatList>("timing") {
            s.set(rows);
        }
    }

    pub fn apply(&mut self, msg: ToUi) {
        match msg {
            ToUi::Line(l) | ToUi::Show(l) => {
                let tone = tone_of(&l);
                if let Some(e) = self.ui.get_mut::<LogPanel>("events") {
                    e.push(&l, tone);
                }
            }
            ToUi::Midi(l) => {
                self.midi += 1;
                let n = self.midi;
                if let Some(m) = self.ui.get_mut::<LogPanel>("midi") {
                    m.push(&l, if l.contains("->") { Tone::Accent } else { Tone::Normal });
                    m.right = format!("{n} messages");
                }
            }
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
            ToUi::Kit(k) => self.set_kit(k),
            ToUi::Level { db, value } => {
                if let Some(f) = self.ui.get_mut::<Fader>("level") {
                    f.value = value;
                    f.text = format!("{db:.1} dB");
                }
                self.chip(2, format!("{db:.0} dB"), Tone::Dim);
            }
            ToUi::Session(text, tone) => self.set_session(&text, tone),
            ToUi::Timing(t) => {
                self.timing = t;
                self.chip(3, format!("{} s left", t.seconds_left), Tone::Dim);
                self.show_timing();
            }
            ToUi::Quit => {}
        }
    }

    /// A touch meant something: switch pages here, hand the rest to the audio thread.
    pub fn on_event(&mut self, ev: UiEvent) -> Option<FromUi> {
        match (ev.widget.as_str(), ev.kind) {
            ("header", EventKind::Tab(i)) => {
                self.show_page(i);
                None
            }
            ("pads", EventKind::Pad { index, down }) => Some(FromUi::Pad { pad: index, down }),
            ("kits", EventKind::Select(k)) => Some(FromUi::Kit(k)),
            ("level", EventKind::Value(v)) => Some(FromUi::Level(v)),
            _ => None,
        }
    }

    /// Show page 0 (Play) or 1 (Debug).
    pub fn show_page(&mut self, i: usize) {
        let page = if i == 0 { "Play" } else { "Debug" };
        let _ = self.ui.show(page);
        if let Some(h) = self.ui.get_mut::<Header>("header") {
            h.active = i.min(1);
        }
        if let Some(h) = self.ui.get_mut::<Label>("hint") {
            h.text = if i == 0 { PLAY_HINT } else { DEBUG_HINT }.to_string();
        }
    }

    /// Advance animations by `dt` seconds.
    pub fn tick(&mut self, dt: f32) {
        if self.ui.get::<PadGrid>("pads").is_some_and(|g| g.fading()) {
            if let Some(g) = self.ui.get_mut::<PadGrid>("pads") {
                g.fade(dt);
            }
        }
    }

    pub fn set_screen_row(&mut self, text: String) {
        self.screen_row = text;
        self.show_timing();
    }
}

// ---------------------------------------------------------------- touch

/// struct input_event: 16 bytes on the MPC's 32-bit ARM, 24 on a 64-bit host.
const EVENT_SIZE: usize = if cfg!(target_pointer_width = "32") { 16 } else { 24 };
/// The MPC One's touch profile (docs/hardware/data/mpc-one-touch-profile.json, M12).
const RAW: RawRange = RawRange { min: 0, max: 2048 };
const ORIENTATION: Orientation = Orientation { swap_xy: true, flip_x: true, flip_y: false };
const SLOTS: usize = 10;

pub fn parse_input_event(b: &[u8]) -> InputEvent {
    let word = EVENT_SIZE / 2 - 4; // bytes per timeval field: 4 or 8
    let field = |at: usize| -> u64 {
        let mut v = [0u8; 8];
        v[..word].copy_from_slice(&b[at..at + word]);
        u64::from_ne_bytes(v)
    };
    let (sec, usec) = (field(0), field(word));
    let base = EVENT_SIZE - 8;
    InputEvent::new(sec.wrapping_mul(1_000_000).wrapping_add(usec),
                    u16::from_ne_bytes([b[base], b[base + 1]]), u16::from_ne_bytes([b[base + 2], b[base + 3]]),
                    i32::from_ne_bytes([b[base + 4], b[base + 5], b[base + 6], b[base + 7]]))
}

/// The touchscreen, read without grabbing it, turned into finger events in
/// screen pixels.
pub struct TouchIn {
    file: File,
    decoder: MtDecoder,
    map: AxisMap,
    buf: Vec<u8>,
    pub contacts: u64,
}

impl TouchIn {
    pub fn open(path: &str, size: (i32, i32)) -> Result<TouchIn, String> {
        let file = OpenOptions::new().read(true).custom_flags(crate::sys::O_NONBLOCK).open(path)
            .map_err(|e| format!("{path}: {e}"))?;
        Ok(TouchIn::from_file(file, size))
    }

    fn from_file(file: File, size: (i32, i32)) -> TouchIn {
        let decoder = MtDecoder::new(SLOTS).expect("10 slots");
        let map = AxisMap::new(RAW, RAW, ORIENTATION, size.0 as u32, size.1 as u32).expect("touch map");
        TouchIn { file, decoder, map, buf: vec![0u8; EVENT_SIZE * 64], contacts: 0 }
    }

    /// Everything that happened since the last call: (finger, phase, x, y).
    pub fn read(&mut self) -> io::Result<Vec<(u64, Touch, i32, i32)>> {
        let mut out = Vec::new();
        loop {
            let n = match self.file.read(&mut self.buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e) => return Err(e),
            };
            for chunk in self.buf[..n - n % EVENT_SIZE].chunks(EVENT_SIZE) {
                let ev = parse_input_event(chunk);
                match self.decoder.push(ev) {
                    Step::Frame => {
                        for ce in self.decoder.events() {
                            let p = self.map.pointer(ce);
                            let phase = match ce.phase {
                                Phase::Down => {
                                    self.contacts += 1;
                                    Touch::Down
                                }
                                Phase::Move => Touch::Move,
                                Phase::Up => Touch::Up,
                                Phase::Cancel => Touch::Cancel,
                            };
                            out.push((p.id.0, phase, p.x.round() as i32, p.y.round() as i32));
                        }
                    }
                    Step::ResyncRequired => {
                        if self.decoder.resync_without_snapshot(ev.time_us) == Step::Frame {
                            for ce in self.decoder.events() {
                                out.push((ce.contact.id.0, Touch::Cancel, 0, 0));
                            }
                        }
                    }
                    Step::Pending => {}
                }
            }
            if n < self.buf.len() {
                break;
            }
        }
        Ok(out)
    }
}

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
pub fn spawn(mut player: Player, mut fb: Option<FbTarget>, mut touch: Option<TouchIn>, rx: Receiver<ToUi>,
             tx: SyncSender<FromUi>) -> io::Result<JoinHandle<ScreenReport>> {
    thread::Builder::new().name("mpfree-screen".into()).spawn(move || {
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
            if let Some(t) = touch.as_mut() {
                match t.read() {
                    Ok(list) => {
                        for (finger, phase, x, y) in list {
                            if let Some(ev) = player.ui.touch(finger, phase, x, y) {
                                if let Some(a) = player.on_event(ev) {
                                    let _ = tx.try_send(a);
                                }
                            }
                        }
                        report.touches = t.contacts;
                    }
                    Err(e) => {
                        println!("touch: {e} (touch input stopped)");
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
/// level, timing and MIDI lines.
pub fn sample_player(fonts: Fonts) -> Player {
    let mut p = Player::new(fonts, sample_kits(), SCREEN);
    p.apply(ToUi::Session("Akai session".into(), Tone::Good));
    p.apply(ToUi::Kit(0));
    p.apply(ToUi::Level { db: -18.0, value: 0.79 });
    for (pad, v) in [(0usize, 112u8), (5, 87), (10, 64)] {
        p.apply(ToUi::PadDown { pad, velocity: v });
    }
    p.apply(ToUi::PadUp(10));
    if let Some(g) = p.ui.get_mut::<PadGrid>("pads") {
        g.pads[10].level = 0.5;
    }
    p.apply(ToUi::Timing(Timing { hits: 42, periods: 10_335, underruns: 0, voices: 3, peak_db: -9.0, last_ms: 7.9,
                                  min_ms: 6.2, mean_ms: 7.8, max_ms: 9.6, seconds_left: 87 }));
    p.set_screen_row("30 fps, slowest frame 6.4 ms".into());
    for l in ["+0.012 s -> Public: F0 7E 00 06 01 F7 = identity request",
              "+0.013 s -> Public: F0 47 00 46 52 00 04 15 21 6D 43 F7 = session ping",
              "+0.020 s Private: F0 47 7F 46 53 00 05 15 21 6D 43 02 F7 = session reply, token 15 21 6D 43, accepted",
              "+4.211 s Private: 99 25 70 = pad 1 down, velocity 112",
              "+4.302 s Private: 89 25 00 = pad 1 up",
              "+5.018 s Private: B0 64 01 = data wheel +1",
              "+5.640 s Private: B0 10 02 = Q-Link 1 turned +2",
              "+6.101 s Private: 99 2E 57 = pad 6 down, velocity 87",
              "+6.900 s Private: 90 51 7F = button 81 pressed"] {
        p.apply(ToUi::Midi(l.into()));
    }
    for l in ["pad player: 6 kits, 81 sounds", "session accepted; sending Akai's setup exactly as the app did",
              "opening /dev/snd/pcmC1D0p (ACVA)", "SCHED_FIFO priority 70", "kit 2: Boom Bap (12 pads)",
              "kit 1: Classic House (16 pads)", "level -18.0 dB"] {
        p.apply(ToUi::Line(l.into()));
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layouts_fit_the_screen() {
        let p = Player::new(Fonts::embedded(), sample_kits(), SCREEN);
        let pads = p.ui.rect_of("pads").unwrap();
        assert!(pads.w > 700 && pads.h > 600, "{pads:?}");
        let kits = p.ui.rect_of("kits").unwrap();
        assert!(kits.h >= 300, "{kits:?}");
        let c = Console::new(Fonts::embedded(), "touch", None);
        assert!(c.ui.rect_of("log").unwrap().h > 600);
    }

    #[test]
    fn touches_become_actions_and_tabs_switch_pages() {
        let mut p = Player::new(Fonts::embedded(), sample_kits(), SCREEN);
        let pads = p.ui.rect_of("pads").unwrap();
        // Pad 1 sits bottom left.
        let ev = p.ui.touch(1, Touch::Down, pads.x + 10, pads.bottom() - 10).unwrap();
        assert_eq!(p.on_event(ev), Some(FromUi::Pad { pad: 0, down: true }));
        let ev = p.ui.touch(1, Touch::Up, pads.x + 10, pads.bottom() - 10).unwrap();
        assert_eq!(p.on_event(ev), Some(FromUi::Pad { pad: 0, down: false }));
        let header = p.ui.rect_of("header").unwrap();
        let ev = p.ui.touch(2, Touch::Down, header.x + 170 + 118 + 20, header.y + 30).unwrap();
        assert_eq!(p.on_event(ev), None);
        assert_eq!(p.ui.page(), "Debug");
        assert!(p.ui.rect_of("midi").is_some() && p.ui.rect_of("pads").is_none());
    }

    #[test]
    fn kit_changes_relabel_the_pads() {
        let mut p = Player::new(Fonts::embedded(), sample_kits(), SCREEN);
        p.apply(ToUi::Kit(3));
        let g = p.ui.get::<PadGrid>("pads").unwrap();
        assert_eq!(g.pads[0].label, "Kick");
        assert!(g.pads[9].color.is_none(), "Lo-Fi has 9 pads");
        assert_eq!(p.ui.get::<ListView>("kits").unwrap().selected, 3);
        p.apply(ToUi::Kit(99)); // ignored
        assert_eq!(p.ui.get::<ListView>("kits").unwrap().selected, 3);
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

    #[test]
    fn touch_reports_turn_into_screen_fingers() {
        // A pipe stands in for the evdev node: one finger down near raw (1024, 2048).
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
        let (reader, mut writer) = std::io::pipe().unwrap();
        writer.write_all(&evs).unwrap();
        drop(writer);
        let mut t = TouchIn::from_file(File::from(std::os::fd::OwnedFd::from(reader)), SCREEN);
        let got = t.read().unwrap();
        assert_eq!(got.len(), 1);
        let (_, phase, x, y) = got[0];
        assert_eq!(phase, Touch::Down);
        // Raw Y runs right to left (2048 = left edge); raw X runs top to bottom.
        assert_eq!((x, y), (0, 400));
    }
}
