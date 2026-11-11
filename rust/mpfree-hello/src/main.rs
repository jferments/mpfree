//! MPFree "Hello MPC" (Phase 1), for the kexec test boot from the USB stick.
//!
//! One program, one step per run, paced from the workstation by
//! tools/mpc-one/host_hello_mpc.py:
//!
//!   inventory          read-only: sound cards, PCM and MIDI devices, input
//!                      devices, framebuffer, panel rotation, RT limits
//!   screen             draws the Hello MPC card on /dev/fb0
//!   touch --seconds N  reads the touchscreen (no grab) and shows the contacts
//!   pads --seconds N [--keepalive]
//!                      reads all three of the controller's MIDI ports (Public,
//!                      Private, MIDI Port) and shows every message in hex, with
//!                      its meaning (pad, button, Q-Link, wheel) where known;
//!                      with --keepalive it also sends Akai's keep-alive message
//!                      on the Public port every 5 s
//!   lights --seconds N Akai's startup pad colours and button lights, then the
//!                      project's colours; pads light brighter while held
//!   setup --seconds N  Akai's session and setup, replayed byte for byte
//!   drums --seconds N [--gain-db G]
//!                      plays the first kit in kits.bin from the pads
//!   play --seconds N [--gain-db G] [--kits FILE]
//!                      the pad player: Akai's session and setup, then every kit
//!                      in kits.bin, picked with the wheel or the screen, played
//!                      from the pads or the screen, level on Q-Link 1 (at most
//!                      G dB), timing of each hit, Note Repeat, Latch, Full and
//!                      Half Level, MENU and Prefs, the button guide; the synth
//!                      (Poly or FM) as the kit list's last entry, its pages on the
//!                      Synth tab with Q-Links 1-4 on the marked knobs; the
//!                      Sample tab, the sampler's pad editor (Wave: Start, End,
//!                      the loop and fades of the pad last hit, Q-Links 1-4 on
//!                      the markers; Chop: slices put on the kit's pads; edits
//!                      stay in memory); STOP ends it
//!   app [--gain-db G] [--kits FILE] [--keep-screen] [--data DIR]
//!                      the same pad player as a program that starts at power-on
//!                      (the test stick's mpfree-app service) and runs until the
//!                      power goes off: a loading page with a progress bar while
//!                      it waits for the controller and the sound card, then the
//!                      player; STOP silences, SHIFT + STOP shows the system log
//!                      (journalctl -f, read-only); --keep-screen keeps the
//!                      startup screen's last frame behind the loading card;
//!                      --data DIR keeps settings and found buttons there
//!                      (mpfree-store; the test stick's partition 4, which
//!                      start-app.sh mounts at /run/mpfree-data)
//!   click --seconds N [--gain-db G]
//!                      plays a 120 BPM click on the ACVA card (stereo S32_LE,
//!                      44.1 kHz, 128-frame periods, 3 periods, as the stock app
//!                      does) and logs every underrun and late period
//!   --preview DIR      host only: draws sample screens to new PNG files in DIR
//!
//! The screen is MPFree's GUI (crate mpfree-ui, screens in gui.rs). Every line
//! also goes to stdout, which the workstation shows, and to the report file.
//! `--font`, `--bold-font` and `--mono-font FILE` swap the built-in fonts.
//!
//! It writes only to: /dev/fb0 and the framebuffer's sysfs `pan` file (the
//! screen), the ACVA PCM device (sound), the controller's Public and Private
//! MIDI ports (only messages Akai's app was seen sending, button lights in the
//! same form with the values it uses, and pad colours made by its rules; never
//! in inventory, screen, touch, or pads without --keepalive) and
//! /run/mpfree-hello/hello.txt (its own report, in RAM; the workstation tool
//! copies it into its report). It reads the screen back once with
//! --keep-screen, and `app` runs `journalctl -f` (read-only) for its console
//! page. With --preview on the workstation it writes only new PNG files. It
//! never touches the MPC's storage (the eMMC is hidden from this kernel
//! anyway). Only with `app --data DIR` does anything survive a power-cycle:
//! settings and found buttons, saved through mpfree-store under DIR alone.

#![deny(unsafe_code)] // the few C calls (and a test-only counting allocator) live in sys.rs, the only module allowed unsafe code

mod akai;
mod app;
mod alsa;
mod calibrate_page;
mod debug_page;
mod engines;
mod gui;
mod kit;
mod looper_page;
mod looping;
mod perf;
mod sample_page;
mod sampler;
mod synth;
mod sys;
mod sysinfo;
mod time;
mod touchlab_page;

use mpfree_controller::midi::{Message, Parser};
use mpfree_controller::mpc_one::{self, ControlEvent, OBSERVED_KEEPALIVE_TOKEN};
use mpfree_core::{float_to_s32, ClickConfig, ClickEngine};
use mpfree_screen_log::Rotation;
use mpfree_time_ui::{TimeAction, TransportInfo};
use mpfree_transport::Command;
use mpfree_synth_ui::SynthAction;
use mpfree_ui::fb::FbTarget;
use mpfree_ui::{Fonts, SoftCanvas, Tone};
use std::fs::{self, File, OpenOptions};
use std::io::{self, ErrorKind, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::thread;
use std::time::{Duration, Instant};

use mpfree_prof::Span;

const REPORT_DIR: &str = "/run/mpfree-hello";
const PANEL_ROTATION: &str = "/proc/device-tree/mipi@ff960000/panel@0/rotation";
const MAX_SECONDS: u64 = 60;
/// The pad player may run longer.
const PLAY_MAX_SECONDS: u64 = 300;
const AUDIO_CARD_ID: &str = "ACVA";
const RATE: u32 = 44_100;
const PERIOD: usize = 128;
const PERIODS: u32 = 3;
const EPIPE: i32 = 32;
const EAGAIN: i32 = 11;
/// The drum kits the host tool copies next to the program (see kit.rs).
const KIT_PATH: &str = "/run/mpfree-hello/kits.bin";
const MAX_KIT_BYTES: u64 = 48 << 20;

struct Options {
    command: String,
    seconds: u64,
    keepalive: bool,
    gain_db: f32,
    fb: PathBuf,
    sysfs: PathBuf,
    preview: Option<PathBuf>,
    font: Option<PathBuf>,
    bold_font: Option<PathBuf>,
    mono_font: Option<PathBuf>,
    kits: PathBuf,
    keep_screen: bool,
    /// An arp pattern library file to read at start (read-only). It takes
    /// the place of the store's arp/patterns.txt for this start.
    patterns: Option<PathBuf>,
    /// Where settings and found buttons are kept (`app` only; mpfree-store).
    data: Option<PathBuf>,
}

fn parse_args(args: &[String]) -> Result<Options, String> {
    let mut o = Options {
        command: String::new(),
        seconds: 10,
        keepalive: false,
        gain_db: -12.0,
        fb: PathBuf::from("/dev/fb0"),
        sysfs: PathBuf::from("/sys/class/graphics/fb0"),
        preview: None,
        font: None,
        bold_font: None,
        mono_font: None,
        kits: PathBuf::from(KIT_PATH),
        keep_screen: false,
        patterns: None,
        data: None,
    };
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let mut value = || it.next().cloned().ok_or_else(|| format!("{arg} needs a value"));
        match arg.as_str() {
            "inventory" | "screen" | "touch" | "pads" | "lights" | "setup" | "drums" | "play" | "app" | "click"
                if o.command.is_empty() => o.command = arg.clone(),
            "--seconds" => o.seconds = value()?.parse().map_err(|_| "--seconds needs a whole number".to_string())?,
            "--keepalive" => o.keepalive = true,
            "--gain-db" => {
                o.gain_db = value()?.parse().map_err(|_| "--gain-db needs a number".to_string())?;
                if !(-40.0..=-6.0).contains(&o.gain_db) {
                    return Err("--gain-db must be between -40 and -6".to_string());
                }
            }
            "--fb" => o.fb = PathBuf::from(value()?),
            "--sysfs" => o.sysfs = PathBuf::from(value()?),
            "--preview" => o.preview = Some(PathBuf::from(value()?)),
            "--font" => o.font = Some(PathBuf::from(value()?)),
            "--bold-font" => o.bold_font = Some(PathBuf::from(value()?)),
            "--mono-font" => o.mono_font = Some(PathBuf::from(value()?)),
            "--kits" => o.kits = PathBuf::from(value()?),
            "--keep-screen" => o.keep_screen = true,
            "--patterns" => o.patterns = Some(PathBuf::from(value()?)),
            "--data" => o.data = Some(PathBuf::from(value()?)),
            _ => return Err(format!("unknown argument {arg}")),
        }
    }
    if o.preview.is_none() && o.command.is_empty() {
        return Err("usage: mpfree-hello inventory|screen|touch|pads|lights|setup|drums|play|app|click [options]"
            .to_string());
    }
    let max = if o.command == "play" { PLAY_MAX_SECONDS } else { MAX_SECONDS };
    if o.command == "app" {
        if args.iter().any(|a| a == "--seconds") {
            return Err("app runs until the power goes off; it takes no --seconds".to_string());
        }
    } else if !(1..=max).contains(&o.seconds) {
        return Err(format!("--seconds must be 1 to {max}"));
    }
    if o.keep_screen && o.command != "app" && o.command != "play" {
        return Err("--keep-screen goes with app or play".to_string());
    }
    if o.keepalive && o.command != "pads" {
        return Err("--keepalive goes with pads".to_string());
    }
    if o.patterns.is_some() && o.command != "app" {
        return Err("--patterns goes with app".to_string());
    }
    if o.data.is_some() && o.command != "app" {
        return Err("--data goes with app".to_string());
    }
    Ok(o)
}

/// The screen, plus the same lines on stdout and in the report file. In the
/// pad player, lines go to the screen thread instead (`sink`), which prints
/// them, so the audio thread never waits for the terminal or the screen.
struct Out {
    console: Option<gui::Console>,
    sink: Option<SyncSender<gui::ToUi>>,
    /// Lines the screen thread had no room for (they still reach the report).
    dropped: u64,
    report: Option<File>,
    last_draw: Instant,
}

impl Out {
    fn new(o: &Options, heading: &str, use_screen: bool) -> Out {
        let report = fs::create_dir_all(REPORT_DIR).ok().and_then(|_| {
            OpenOptions::new().create(true).append(true).mode(0o644).open(Path::new(REPORT_DIR).join("hello.txt")).ok()
        });
        let mut out = Out { console: None, sink: None, dropped: 0, report, last_draw: Instant::now() - Duration::from_secs(1) };
        out.file_line(&format!("===== mpfree-hello {} (uptime {}) =====", heading, uptime()));
        if use_screen {
            match open_screen(o) {
                Ok(fb) => {
                    let fonts = load_fonts(o, &mut out);
                    out.console = Some(gui::Console::new(fonts, heading, Some(fb)));
                }
                Err(e) => out.say(&format!("screen: not used ({e})")),
            }
        }
        out
    }

    fn file_line(&mut self, line: &str) {
        if let Some(f) = self.report.as_mut() {
            let _ = writeln!(f, "{line}");
        }
    }

    fn say(&mut self, line: &str) {
        self.note(line);
        if self.console.is_some() && self.last_draw.elapsed() >= Duration::from_millis(100) {
            self.draw();
        }
    }

    /// Like `say`, but never redraws the screen (a redraw takes longer than an
    /// audio period, so the drum loop only uses this).
    fn note(&mut self, line: &str) {
        self.file_line(line);
        if let Some(s) = self.sink.as_ref() {
            if s.try_send(gui::ToUi::Line(line.to_string())).is_err() {
                self.dropped += 1;
            }
            return;
        }
        println!("{line}");
        let _ = io::stdout().flush();
        if let Some(c) = self.console.as_mut() {
            c.push(line);
        }
    }

    /// A MIDI message in or out: like `note`, but the pad player shows it in
    /// its MIDI monitor rather than the events.
    fn midi(&mut self, line: &str) {
        if self.sink.is_some() {
            self.file_line(line);
            let sent = self.sink.as_ref().is_some_and(|s| s.try_send(gui::ToUi::Midi(line.to_string())).is_ok());
            if !sent {
                self.dropped += 1;
            }
        } else {
            self.note(line);
        }
    }

    /// Something for the pad player's screen only (nothing printed).
    fn show(&mut self, msg: gui::ToUi) {
        if let Some(s) = self.sink.as_ref() {
            if s.try_send(msg).is_err() {
                self.dropped += 1;
            }
        }
    }

    /// Like `show`, but a message that finds no room comes back, so the
    /// audio thread can keep it (and not free it) and try again. The message
    /// comes back by value on purpose: boxing it would allocate on the audio
    /// thread.
    #[allow(clippy::result_large_err)]
    fn try_show(&mut self, msg: gui::ToUi) -> Result<(), gui::ToUi> {
        match self.sink.as_ref() {
            Some(s) => s.try_send(msg).map_err(|e| match e {
                std::sync::mpsc::TrySendError::Full(m) | std::sync::mpsc::TrySendError::Disconnected(m) => m,
            }),
            None => Ok(()),
        }
    }

    fn draw(&mut self) {
        if let Some(c) = self.console.as_mut() {
            c.set_status(&format!("up {} s", uptime()));
            if let Err(e) = c.draw() {
                eprintln!("screen: {e}");
                self.console = None;
            }
        }
        self.last_draw = Instant::now();
    }
}

impl Drop for Out {
    fn drop(&mut self) {
        self.draw();
    }
}

/// The framebuffer, turned as the panel is mounted.
fn open_screen(o: &Options) -> Result<FbTarget, String> {
    let rotation = panel_rotation()?;
    FbTarget::open(&o.fb, &o.sysfs, rotation)
}

/// The built-in fonts, or files given with --font, --bold-font, --mono-font.
fn load_fonts(o: &Options, out: &mut Out) -> Fonts {
    match Fonts::with_files(o.font.as_deref(), o.bold_font.as_deref(), o.mono_font.as_deref()) {
        Ok(f) => f,
        Err(e) => {
            out.say(&format!("font: {e}; using the built-in fonts"));
            Fonts::embedded()
        }
    }
}

fn read_trimmed(path: &Path) -> Result<String, String> {
    fs::read_to_string(path).map(|s| s.trim().to_string()).map_err(|e| format!("{}: {e}", path.display()))
}

fn panel_rotation() -> Result<Rotation, String> {
    let bytes = fs::read(PANEL_ROTATION).map_err(|e| format!("{PANEL_ROTATION}: {e}"))?;
    let degrees = <[u8; 4]>::try_from(bytes.as_slice()).map(u32::from_be_bytes)
        .map_err(|_| format!("{PANEL_ROTATION} is not one 32-bit value"))?;
    Rotation::from_degrees(degrees).ok_or_else(|| format!("unexpected panel rotation {degrees}"))
}

fn uptime() -> String {
    fs::read_to_string("/proc/uptime").ok()
        .and_then(|s| s.split_whitespace().next().map(|v| v.split('.').next().unwrap_or(v).to_string()))
        .unwrap_or_else(|| "?".to_string())
}

// ---------------------------------------------------------------- inventory

/// `/proc/asound/cards` as (number, id, description) per card.
fn parse_cards(text: &str) -> Vec<(u32, String, String)> {
    let mut cards = Vec::new();
    for line in text.lines() {
        let t = line.trim_start();
        let Some((num, rest)) = t.split_once(' ') else { continue };
        let Ok(n) = num.parse::<u32>() else { continue };
        let Some(open) = rest.find('[') else { continue };
        let Some(close) = rest.find(']') else { continue };
        let id = rest[open + 1..close].trim().to_string();
        let desc = rest[close + 1..].trim_start_matches(':').trim().to_string();
        cards.push((n, id, desc));
    }
    cards
}

fn cards() -> Vec<(u32, String, String)> {
    parse_cards(&fs::read_to_string("/proc/asound/cards").unwrap_or_default())
}

fn audio_card() -> Result<u32, String> {
    let all = cards();
    let hits: Vec<u32> = all.iter().filter(|(_, id, _)| id == AUDIO_CARD_ID).map(|c| c.0).collect();
    match hits.as_slice() {
        [n] => Ok(*n),
        _ => Err(format!("expected one sound card with id {AUDIO_CARD_ID}, found {}", hits.len())),
    }
}

/// The internal controller: a USB MIDI card whose description names the MPC One.
fn midi_card() -> Result<u32, String> {
    let all = cards();
    let hits: Vec<u32> = all.iter()
        .filter(|(n, _, desc)| {
            let d = desc.to_ascii_lowercase().replace('_', " ");
            d.contains("mpc one") && Path::new(&format!("/dev/snd/midiC{n}D0")).exists()
        })
        .map(|c| c.0).collect();
    match hits.as_slice() {
        [n] => Ok(*n),
        _ => Err(format!("expected one MPC One MIDI card with /dev/snd/midiC<n>D0, found {}", hits.len())),
    }
}

fn dump(out: &mut Out, path: &str) {
    match fs::read_to_string(path) {
        Ok(text) => {
            out.say(&format!("--- {path}"));
            for line in text.lines().take(60) {
                out.say(&format!("  {line}"));
            }
        }
        Err(e) => out.say(&format!("--- {path}: {e}")),
    }
}

fn inventory(o: &Options) -> Result<(), String> {
    let mut out = Out::new(o, "inventory (read-only)", false);
    out.say(&format!("kernel: {}", read_trimmed(Path::new("/proc/sys/kernel/osrelease")).unwrap_or_default()));
    for p in ["/proc/asound/cards", "/proc/asound/pcm", "/proc/asound/devices"] {
        dump(&mut out, p);
    }
    for (n, id, _) in cards() {
        for f in ["midi0", "pcm0p/info", "pcm0c/info", "pcm0p/sub0/hw_params"] {
            let p = format!("/proc/asound/card{n}/{f}");
            if Path::new(&p).exists() {
                out.say(&format!("(card {n} = {id})"));
                dump(&mut out, &p);
            }
        }
    }
    match audio_card() {
        Ok(n) => out.say(&format!("audio card for the click: {n} ({AUDIO_CARD_ID})")),
        Err(e) => out.say(&format!("audio card: {e}")),
    }
    match midi_card() {
        Ok(n) => out.say(&format!("controller MIDI card: {n}")),
        Err(e) => out.say(&format!("controller MIDI card: {e}")),
    }
    let mut names: Vec<String> = fs::read_dir("/dev/snd").map(|d| d.filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default();
    names.sort();
    out.say(&format!("/dev/snd: {}", names.join(" ")));
    for (path, name) in input_devices() {
        out.say(&format!("input: {path} = {name}"));
    }
    for f in ["name", "modes", "stride", "bits_per_pixel"] {
        let p = o.sysfs.join(f);
        out.say(&format!("fb0 {f}: {}", read_trimmed(&p).unwrap_or_else(|e| e)));
    }
    if let Ok(d) = fs::read_dir("/sys/class/drm") {
        let mut conns: Vec<PathBuf> = d.filter_map(|e| e.ok()).map(|e| e.path())
            .filter(|p| p.join("status").exists()).collect();
        conns.sort();
        for c in conns {
            out.say(&format!("drm {}: {}", c.file_name().unwrap().to_string_lossy(),
                             read_trimmed(&c.join("status")).unwrap_or_default()));
        }
    }
    out.say(&format!("panel rotation: {}", panel_rotation().map(|r| format!("{r:?}")).unwrap_or_else(|e| e)));
    for p in ["/sys/kernel/realtime", "/proc/sys/kernel/sched_rt_runtime_us", "/proc/sys/kernel/sched_rt_period_us"] {
        out.say(&format!("{p}: {}", read_trimmed(Path::new(p)).unwrap_or_else(|e| e)));
    }
    if let Ok(limits) = fs::read_to_string("/proc/self/limits") {
        for l in limits.lines().filter(|l| l.contains("realtime") || l.contains("locked memory")) {
            out.say(&format!("limit: {}", l.split_whitespace().collect::<Vec<_>>().join(" ")));
        }
    }
    out.say("inventory done");
    Ok(())
}

// ---------------------------------------------------------------- screen

fn screen(o: &Options) -> Result<(), String> {
    let mut out = Out::new(o, "screen", true);
    if out.console.is_none() {
        return Err("no screen to draw on".to_string());
    }
    for line in [
        "Hello MPC.",
        "",
        "This screen is drawn by MPFree's own GUI code, running from the USB stick.",
        "Akai's software is not running, and this system cannot see the MPC's own storage.",
        "",
        "Next: touch the screen, hit the pads, and hear a click, one step at a time,",
        "each started from the workstation.",
        "",
        "Switch the MPC off with its power switch to return to the stock firmware.",
    ] {
        out.say(line);
    }
    out.draw();
    out.say("screen: card drawn");
    Ok(())
}

// ---------------------------------------------------------------- touch

const EV_SYN: u16 = 0;
const EV_ABS: u16 = 3;
const SYN_REPORT: u16 = 0;
const ABS_MT_SLOT: u16 = 0x2f;
const ABS_MT_POSITION_X: u16 = 0x35;
const ABS_MT_POSITION_Y: u16 = 0x36;
const ABS_MT_TRACKING_ID: u16 = 0x39;
const MAX_SLOTS: usize = 10;

/// struct input_event: 16 bytes on the MPC's 32-bit ARM (timeval of two 32-bit fields).
const EVENT_SIZE: usize = if sys::IS_DEVICE_ABI { 16 } else { 24 };

fn parse_event(b: &[u8]) -> (u16, u16, i32) {
    let base = EVENT_SIZE - 8;
    (u16::from_ne_bytes([b[base], b[base + 1]]), u16::from_ne_bytes([b[base + 2], b[base + 3]]),
     i32::from_ne_bytes([b[base + 4], b[base + 5], b[base + 6], b[base + 7]]))
}

fn input_devices() -> Vec<(String, String)> {
    let mut found = Vec::new();
    if let Ok(d) = fs::read_dir("/sys/class/input") {
        for e in d.filter_map(|e| e.ok()) {
            let n = e.file_name().to_string_lossy().into_owned();
            if n.starts_with("event") {
                let name = read_trimmed(&e.path().join("device/name")).unwrap_or_default();
                found.push((format!("/dev/input/{n}"), name));
            }
        }
    }
    found.sort();
    found
}

#[derive(Clone, Copy, Default, PartialEq)]
struct Contact {
    id: i32,
    x: i32,
    y: i32,
}

/// Multitouch type B: slot-addressed contacts, reported at each SYN_REPORT.
struct Touches {
    slot: usize,
    slots: [Option<Contact>; MAX_SLOTS],
}

impl Touches {
    fn new() -> Touches {
        Touches { slot: 0, slots: [None; MAX_SLOTS] }
    }

    /// Feed one event; returns true at a SYN_REPORT.
    fn feed(&mut self, ty: u16, code: u16, value: i32) -> bool {
        match (ty, code) {
            (EV_ABS, ABS_MT_SLOT) => self.slot = (value.max(0) as usize).min(MAX_SLOTS - 1),
            (EV_ABS, ABS_MT_TRACKING_ID) => {
                self.slots[self.slot] = if value < 0 { None } else { Some(Contact { id: value, x: 0, y: 0 }) }
            }
            (EV_ABS, ABS_MT_POSITION_X) => if let Some(c) = self.slots[self.slot].as_mut() { c.x = value },
            (EV_ABS, ABS_MT_POSITION_Y) => if let Some(c) = self.slots[self.slot].as_mut() { c.y = value },
            (EV_SYN, SYN_REPORT) => return true,
            _ => {}
        }
        false
    }

    fn describe(&self) -> String {
        let parts: Vec<String> = self.slots.iter().enumerate()
            .filter_map(|(i, c)| c.map(|c| format!("slot {i}: x {} y {}", c.x, c.y))).collect();
        if parts.is_empty() { "no contact".to_string() } else { format!("{} contact(s): {}", parts.len(), parts.join(", ")) }
    }
}

/// The touchscreen's event device: (path, name).
fn touchscreen() -> Result<(String, String), String> {
    let devices = input_devices();
    let hits: Vec<&(String, String)> = devices.iter().filter(|(_, n)| n.to_ascii_lowercase().contains("ili2117")).collect();
    match hits.as_slice() {
        [one] => Ok((*one).clone()),
        _ => Err(format!("expected one ILI2117 input device, found {}", hits.len())),
    }
}

fn touch(o: &Options) -> Result<(), String> {
    let mut out = Out::new(o, "touch", true);
    let (path, name) = touchscreen()?;
    out.say(&format!("touchscreen: {path} ({name}); read-only, not grabbed"));
    let mut dev = OpenOptions::new().read(true).custom_flags(sys::O_NONBLOCK).open(&path)
        .map_err(|e| format!("{path}: {e}"))?;
    out.say(&format!("touch the screen now, with one finger and then two ({} s)", o.seconds));
    let mut t = Touches::new();
    let mut buf = vec![0u8; EVENT_SIZE * 64];
    let (mut events, mut frames, mut shown) = (0u64, 0u64, 0u64);
    let mut last = String::new();
    let mut last_shown = Instant::now() - Duration::from_secs(1);
    let end = Instant::now() + Duration::from_secs(o.seconds);
    while Instant::now() < end {
        match dev.read(&mut buf) {
            Ok(n) => {
                for ev in buf[..n - n % EVENT_SIZE].chunks(EVENT_SIZE) {
                    let (ty, code, value) = parse_event(ev);
                    events += 1;
                    if t.feed(ty, code, value) {
                        frames += 1;
                        let now = t.describe();
                        // At most 5 lines a second, and every change of the contact count.
                        let count_changed = now.split(' ').next() != last.split(' ').next();
                        if now != last && (count_changed || last_shown.elapsed() >= Duration::from_millis(200)) {
                            out.say(&format!("touch: {now}"));
                            last = now;
                            last_shown = Instant::now();
                            shown += 1;
                        }
                    }
                }
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => thread::sleep(Duration::from_millis(5)),
            Err(e) => return Err(format!("reading {path}: {e}")),
        }
    }
    out.say(&format!("touch done: {events} events, {frames} frames, {shown} lines shown"));
    Ok(())
}

// ---------------------------------------------------------------- pads

fn describe_event(e: &ControlEvent) -> String {
    match *e {
        ControlEvent::PadDown { pad, velocity } => format!("pad {} down, velocity {velocity}", pad + 1),
        ControlEvent::PadUp { pad } => format!("pad {} up", pad + 1),
        ControlEvent::PadPressure { pad, pressure } => format!("pad {} pressure {pressure}", pad + 1),
        ControlEvent::QLinkTurn { knob, delta } => format!("Q-Link {} turned {delta:+}", knob + 1),
        ControlEvent::QLinkTouch { knob, touched } => format!("Q-Link {} {}", knob + 1, if touched { "touched" } else { "released" }),
        ControlEvent::DataWheel { delta } => format!("data wheel {delta:+}"),
        ControlEvent::Button { note, pressed } => format!("button {note} {}", if pressed { "pressed" } else { "released" }),
        ControlEvent::KeepAliveReply { token } => format!("keep-alive reply, token {token:02X?}"),
    }
}

/// The controller's three USB-MIDI cables, as ALSA RawMIDI subdevices of
/// midiC<card>D0, with a word each subdevice's name must contain.
const MIDI_PORTS: [(u32, &str); 3] = [(0, "Public"), (1, "Private"), (2, "MIDI Port")];
/// Lines shown per listening step; everything is still counted after that.
const MAX_SHOWN: u64 = 300;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" ")
}

/// The bytes of one parsed message, as they came in (running status written out).
fn message_bytes(msg: &Message<'_>) -> (Vec<u8>, bool) {
    match *msg {
        Message::Channel { status, data1, data2 } => match status & 0xF0 {
            0xC0 | 0xD0 => (vec![status, data1], false),
            _ => (vec![status, data1, data2], false),
        },
        Message::SysEx { bytes, truncated } => (bytes.to_vec(), truncated),
        Message::RealTime(b) => (vec![b], false),
        Message::Common { status, data1, data2 } => match status {
            0xF1 | 0xF3 => (vec![status, data1], false),
            0xF2 => (vec![status, data1, data2], false),
            _ => (vec![status], false),
        },
    }
}

/// One line for one message: time, port, the bytes in hex, and what it means if known.
fn message_line(t: f64, port: &str, bytes: &[u8], truncated: bool, event: Option<&ControlEvent>) -> String {
    if akai::is_identity_reply(bytes) {
        let shown = &bytes[..bytes.len().min(akai::IDENTITY_SHOWN)];
        return format!("+{t:.3} s {port}: {} ... = identity reply (the rest is the serial number, not shown)", hex(shown));
    }
    if let Some((token, status)) = akai::session_reply(bytes) {
        let what = match status {
            akai::SESSION_HELD => "held by another token",
            akai::SESSION_RUNNING => "running",
            akai::SESSION_ACCEPTED => "accepted",
            _ => "unknown status",
        };
        return format!("+{t:.3} s {port}: {} = session reply, token {}, {what}", hex(bytes), hex(&token));
    }
    let meaning = match event {
        // Without Akai's app the controller sends the Q-Links on Public as
        // absolute values 0-127, not steps (Hello MPC round 2, 2026-11-10).
        Some(ControlEvent::QLinkTurn { knob, .. }) if port == "Public" && bytes.len() == 3 => {
            format!("Q-Link {} at {} (absolute)", knob + 1, bytes[2])
        }
        Some(e) => describe_event(e),
        None => "not recognised".to_string(),
    };
    let cut = if truncated { " (cut short: longer than the parser keeps)" } else { "" };
    format!("+{t:.3} s {port}: {}{cut} = {meaning}", hex(bytes))
}

struct PortIn {
    name: &'static str,
    file: File,
    parser: Parser,
    bytes: u64,
    messages: u64,
}

/// All three of the controller's inputs. Akai's app gets every control on
/// Private; without the app they arrive on Public (Hello MPC round 2).
fn open_inputs(card: u32, out: &mut Out) -> Result<Vec<PortIn>, String> {
    let mut inputs = Vec::new();
    for (sub, word) in MIDI_PORTS {
        let (file, info) = alsa::open_rawmidi(card, sub, alsa::Direction::Input)
            .map_err(|e| format!("opening MIDI input {sub} ({word}): {e}"))?;
        out.say(&format!("listening on card {card} subdevice {} \"{}\"", info.subdevice, info.subname));
        if info.subdevice != sub || info.stream != alsa::RAWMIDI_STREAM_INPUT || !info.subname.contains(word) {
            return Err(format!("that is not the controller's {word} port; stopping"));
        }
        inputs.push(PortIn { name: word, file, parser: Parser::default(), bytes: 0, messages: 0 });
    }
    Ok(inputs)
}

fn open_output(card: u32, sub: u32, word: &str, out: &mut Out) -> Result<File, String> {
    let (file, info) = alsa::open_rawmidi(card, sub, alsa::Direction::Output)
        .map_err(|e| format!("opening MIDI output {sub} ({word}): {e}"))?;
    out.say(&format!("sending to card {card} subdevice {} \"{}\"", info.subdevice, info.subname));
    if info.subdevice != sub || info.stream != alsa::RAWMIDI_STREAM_OUTPUT || !info.subname.contains(word) {
        return Err(format!("that is not the controller's {word} port; stopping before sending anything"));
    }
    Ok(file)
}

/// Write one whole message to a non-blocking MIDI output, waiting up to half a
/// second for room in the kernel's buffer.
fn send_all(file: &mut File, msg: &[u8]) -> io::Result<()> {
    let mut done = 0;
    let give_up = Instant::now() + Duration::from_millis(500);
    while done < msg.len() {
        match file.write(&msg[done..]) {
            Ok(0) => return Err(io::Error::new(ErrorKind::WriteZero, "the MIDI port took nothing")),
            Ok(n) => done += n,
            Err(e) if (e.kind() == ErrorKind::WouldBlock || e.raw_os_error() == Some(EAGAIN)) && Instant::now() < give_up => {
                thread::sleep(Duration::from_millis(1))
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// One message that came in: which input, its bytes, and what it means if known.
struct Incoming {
    port: usize,
    bytes: Vec<u8>,
    event: Option<ControlEvent>,
}

/// The controller's inputs and outputs for the steps that send something.
struct Ports {
    inputs: Vec<PortIn>,
    public: File,
    private: Option<File>,
    start: Instant,
    sent: u64,
    shown: u64,
    /// Messages shown before only counting.
    max_shown: u64,
    /// Print without redrawing the screen (the drum loop).
    quiet: bool,
    /// List pad pressure (aftertouch) too; it is only counted otherwise.
    show_pressure: bool,
}

impl Ports {
    fn open(out: &mut Out, with_private: bool) -> Result<Ports, String> {
        let card = midi_card()?;
        let inputs = open_inputs(card, out)?;
        let public = open_output(card, mpc_one::Port::Public as u32, "Public", out)?;
        let private = if with_private {
            Some(open_output(card, mpc_one::Port::Private as u32, "Private", out)?)
        } else {
            None
        };
        Ok(Ports { inputs, public, private, start: Instant::now(), sent: 0, shown: 0, max_shown: MAX_SHOWN, quiet: false,
                   show_pressure: false })
    }

    fn t(&self) -> f64 {
        self.start.elapsed().as_secs_f64()
    }

    fn line(&self, out: &mut Out, text: &str) {
        if self.quiet {
            out.note(text);
        } else {
            out.say(text);
        }
    }

    fn send(&mut self, private: bool, msg: &[u8]) -> Result<(), String> {
        let (file, name) = if private {
            (self.private.as_mut().ok_or("the Private output is not open")?, "Private")
        } else {
            (&mut self.public, "Public")
        };
        send_all(file, msg).map_err(|e| format!("sending {} on {name}: {e}", hex(msg)))?;
        self.sent += 1;
        Ok(())
    }

    /// Send a list of messages and show them all, six to a line.
    fn send_batch(&mut self, out: &mut Out, private: bool, what: &str, msgs: &[&[u8]]) -> Result<(), String> {
        let port = if private { "Private" } else { "Public" };
        self.line(out, &format!("+{:.3} s sending {} messages on {port}: {what}", self.t(), msgs.len()));
        for chunk in msgs.chunks(6) {
            for m in chunk {
                self.send(private, m)?;
            }
            let text = chunk.iter().map(|m| hex(m)).collect::<Vec<_>>().join(" | ");
            out.midi(&format!("    -> {port}: {text}"));
        }
        Ok(())
    }

    /// Read whatever has arrived on the three inputs, show each message (pad
    /// pressure only counted), and return them.
    fn poll(&mut self, out: &mut Out, buf: &mut [u8]) -> Result<Vec<Incoming>, String> {
        let mut got = Vec::new();
        let t = self.t();
        for i in 0..self.inputs.len() {
            let n = match self.inputs[i].file.read(buf) {
                Ok(n) => n,
                Err(e) if e.kind() == ErrorKind::WouldBlock || e.raw_os_error() == Some(EAGAIN) => 0,
                Err(e) => return Err(format!("reading MIDI ({}): {e}", self.inputs[i].name)),
            };
            self.inputs[i].bytes += n as u64;
            let name = self.inputs[i].name;
            for &b in &buf[..n] {
                let Some(msg) = self.inputs[i].parser.feed(b) else { continue };
                let event = mpc_one::decode(&msg);
                let (bytes, truncated) = message_bytes(&msg);
                self.inputs[i].messages += 1;
                if self.show_pressure || !matches!(event, Some(ControlEvent::PadPressure { .. })) {
                    self.shown += 1;
                    if self.shown <= self.max_shown {
                        let text = message_line(t, name, &bytes, truncated, event.as_ref());
                        out.midi(&text);
                    } else if self.shown == self.max_shown + 1 {
                        let text = format!("(more than {} messages: only counting from here)", self.max_shown);
                        self.line(out, &text);
                    }
                }
                got.push(Incoming { port: i, bytes, event });
            }
        }
        Ok(got)
    }

    /// Light a pad brighter while it is held, as Akai's app does.
    fn light_hit(&mut self, event: &Option<ControlEvent>, rest: &[[u8; 3]; 16]) -> Result<(), String> {
        match *event {
            Some(ControlEvent::PadDown { pad, .. }) if pad < 16 => {
                self.send(false, &akai::pad_colour(pad, akai::hit_colour(rest[pad as usize])))
            }
            Some(ControlEvent::PadUp { pad }) if pad < 16 => self.send(false, &akai::pad_colour(pad, rest[pad as usize])),
            _ => Ok(()),
        }
    }

    fn counts(&self, out: &mut Out) {
        for p in &self.inputs {
            out.say(&format!("{}: {} bytes, {} messages", p.name, p.bytes, p.messages));
        }
    }
}

fn pad_colours(colours: &[[u8; 3]; 16]) -> Vec<[u8; 12]> {
    (0..16u8).map(|pad| akai::pad_colour(pad, colours[pad as usize])).collect()
}

fn as_slices<const N: usize>(msgs: &[[u8; N]]) -> Vec<&[u8]> {
    msgs.iter().map(|m| &m[..]).collect()
}

/// Listen for `seconds`, lighting pads as they are hit and, with a session,
/// pinging every few seconds. Returns pad hits per input port.
fn listen_and_light(out: &mut Out, ports: &mut Ports, seconds: u64, rest: &[[u8; 3]; 16],
                    ping: bool) -> Result<[u64; 3], String> {
    let mut buf = [0u8; 256];
    let mut hits = [0u64; 3];
    let mut last_ping = Instant::now();
    let end = Instant::now() + Duration::from_secs(seconds);
    while Instant::now() < end {
        if ping && last_ping.elapsed() >= Duration::from_secs(akai::PING_SECONDS) {
            ports.send(false, &akai::session_ping(akai::SESSION_TOKEN))?;
            last_ping = Instant::now();
        }
        let got = ports.poll(out, &mut buf)?;
        for m in &got {
            if matches!(m.event, Some(ControlEvent::PadDown { .. })) {
                hits[m.port] += 1;
            }
            ports.light_hit(&m.event, rest)?;
        }
        if got.is_empty() {
            thread::sleep(Duration::from_millis(2));
        }
    }
    Ok(hits)
}

// ---------------------------------------------------------------- lights

/// Pad colours and button lights, with no session: Akai's startup gradient and
/// button lights, then the resting colours of the owner's project, then 15 s
/// where each pad lights brighter while it is held.
fn lights(o: &Options) -> Result<(), String> {
    let mut out = Out::new(o, "lights (no session)", true);
    let mut ports = Ports::open(&mut out, true)?;
    out.say("every message sent is one Akai's app sent at startup (Hello MPC round 2), or a pad colour");
    out.say("made by the app's own rule for a hit pad");
    let gradient = pad_colours(&akai::STARTUP_GRADIENT);
    ports.send_batch(&mut out, false, "Akai's startup pad colours, orange (pad 1) to teal (pad 16)", &as_slices(&gradient))?;
    let lights: Vec<[u8; 3]> = akai::STARTUP_BUTTON_LIGHTS.iter().map(|&[n, v]| akai::button_light(n, v)).collect();
    ports.send_batch(&mut out, true, "Akai's startup button lights", &as_slices(&lights))?;
    out.say("holding these for 4 s");
    listen_and_light(&mut out, &mut ports, 4, &akai::STARTUP_GRADIENT, false)?;
    let rest = pad_colours(&akai::RESTING_COLOURS);
    ports.send_batch(&mut out, false, "your project's pad colours, as Akai's app left them", &as_slices(&rest))?;
    out.say(&format!("hit pads: each should light brighter while held ({} s)", o.seconds));
    out.draw();
    let hits = listen_and_light(&mut out, &mut ports, o.seconds, &akai::RESTING_COLOURS, false)?;
    ports.counts(&mut out);
    out.say(&format!("lights done: {} messages sent, pad hits on Public {}, Private {}, MIDI Port {}",
                     ports.sent, hits[0], hits[1], hits[2]));
    Ok(())
}

// ---------------------------------------------------------------- setup

/// Ask for a session as Akai's app does: identity request and session ping,
/// every 4 s, until the controller accepts our token, for up to 20 s.
/// Returns whether it was accepted. Sends nothing else.
fn start_session(out: &mut Out, ports: &mut Ports) -> Result<bool, String> {
    let mut buf = [0u8; 256];
    out.say(&format!("asking for a session with token {}: identity request, then a ping, every {} s, for up to 20 s",
                     hex(&akai::SESSION_TOKEN), akai::PING_SECONDS));
    let give_up = Instant::now() + Duration::from_secs(20);
    let mut next_ping = Instant::now();
    let mut accepted = None;
    while accepted.is_none() && Instant::now() < give_up {
        if Instant::now() >= next_ping {
            ports.send_batch(out, false, "identity request and session ping",
                             &[&akai::IDENTITY_REQUEST[..], &akai::session_ping(akai::SESSION_TOKEN)[..]])?;
            next_ping = Instant::now() + Duration::from_secs(akai::PING_SECONDS);
        }
        for m in ports.poll(out, &mut buf)? {
            if let Some((token, status)) = akai::session_reply(&m.bytes) {
                if token == akai::SESSION_TOKEN && (status == akai::SESSION_ACCEPTED || status == akai::SESSION_RUNNING) {
                    accepted = Some(status);
                }
            }
        }
        thread::sleep(Duration::from_millis(2));
    }
    Ok(accepted.is_some())
}

/// Listen (and show what comes in) for `ms` milliseconds.
fn listen_for(out: &mut Out, ports: &mut Ports, ms: u64) -> Result<(), String> {
    let until = Instant::now() + Duration::from_millis(ms);
    let mut b = [0u8; 256];
    while Instant::now() < until {
        ports.poll(out, &mut b)?;
        thread::sleep(Duration::from_millis(1));
    }
    Ok(())
}

/// Akai's setup after the session was accepted, byte for byte and in the
/// app's order and timing (Hello MPC round 2), then `colours` as the pads'
/// resting colours, and a ping.
fn send_setup(out: &mut Out, ports: &mut Ports, colours: &[[u8; 3]; 16], what: &str) -> Result<(), String> {
    out.say("session accepted; sending Akai's setup exactly as the app did");
    ports.send_batch(out, false, "setup, part 1 (as at +25.772 s)", &akai::SETUP_FIRST)?;
    listen_for(out, ports, 12)?;
    ports.send_batch(out, false, "setup, part 2 (sent twice, as at +25.784 s)", &[&akai::SETUP_5A[..], &akai::SETUP_5A[..]])?;
    let gradient = pad_colours(&akai::STARTUP_GRADIENT);
    ports.send_batch(out, false, "Akai's startup pad colours", &as_slices(&gradient))?;
    let lights: Vec<[u8; 3]> = akai::STARTUP_BUTTON_LIGHTS.iter().map(|&[n, v]| akai::button_light(n, v)).collect();
    ports.send_batch(out, true, "Akai's startup button lights", &as_slices(&lights))?;
    listen_for(out, ports, 340)?;
    ports.send_batch(out, false, "setup, part 3 (as at +26.170 s)", &akai::SETUP_LAST)?;
    listen_for(out, ports, 960)?;
    let rest = pad_colours(colours);
    ports.send_batch(out, false, what, &as_slices(&rest))?;
    ports.send(false, &akai::session_ping(akai::SESSION_TOKEN))?;
    Ok(())
}

/// Akai's startup, replayed: identity request and session ping until the
/// controller accepts the token, then the setup messages, lights, and a listening
/// window with pings every 4 s. Stops before any setup message if the session
/// is not accepted within 20 s.
fn setup(o: &Options) -> Result<(), String> {
    let mut out = Out::new(o, "Akai's setup (session)", true);
    let mut ports = Ports::open(&mut out, true)?;
    if !start_session(&mut out, &mut ports)? {
        return Err("the controller did not accept the session within 20 s; no setup message was sent".to_string());
    }
    send_setup(&mut out, &mut ports, &akai::RESTING_COLOURS, "your project's pad colours")?;
    out.say(&format!("hit pads, turn Q-Link 1 and the wheel, press a few buttons ({} s); pings go on every {} s",
                     o.seconds, akai::PING_SECONDS));
    out.draw();
    let hits = listen_and_light(&mut out, &mut ports, o.seconds, &akai::RESTING_COLOURS, true)?;
    ports.counts(&mut out);
    out.say(&format!("setup done: {} messages sent, pad hits on Public {}, Private {}, MIDI Port {}",
                     ports.sent, hits[0], hits[1], hits[2]));
    out.say("no more pings: the controller should drop the session by itself (inferred: within about 10 s)");
    Ok(())
}

// ---------------------------------------------------------------- drums

/// The kits file (by default where the host tool copies it; the test stick
/// keeps its own), read in steps so `step(done, total)` can show progress.
fn load_kits(path: &Path, step: &mut dyn FnMut(u64, u64)) -> Result<Vec<kit::Kit>, String> {
    let shown = path.display();
    let mut f = File::open(path).map_err(|e| format!("{shown}: {e}"))?;
    let size = f.metadata().map_err(|e| format!("{shown}: {e}"))?.len();
    if size > MAX_KIT_BYTES {
        return Err(format!("{shown} is {size} bytes; at most {MAX_KIT_BYTES} are allowed"));
    }
    let mut data = Vec::with_capacity(size as usize);
    let mut chunk = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut chunk).map_err(|e| format!("{shown}: {e}"))?;
        if n == 0 {
            break;
        }
        data.extend_from_slice(&chunk[..n]);
        if data.len() as u64 > MAX_KIT_BYTES {
            return Err(format!("{shown} grew past {MAX_KIT_BYTES} bytes while being read"));
        }
        step(data.len() as u64, size);
    }
    kit::parse_kits(&data)
}

/// Resting pad colours for kit number `k`: its colour where it has a sound, dark elsewhere.
fn kit_pad_colours(kit: &kit::Kit, k: usize) -> [[u8; 3]; 16] {
    let mut rest = [[0u8; 3]; 16];
    for s in &kit.sounds {
        if let Some(pad) = mpc_one::pad_index(s.note) {
            rest[pad as usize] = akai::kit_colour(k);
        }
    }
    rest
}

/// Resting pad colours for kit number `k` from which pads have a sound
/// (`sampler::Player::sounds`, which follows the Sample tab's chops).
fn sound_colours(has: [bool; 16], k: usize) -> [[u8; 3]; 16] {
    has.map(|h| if h { akai::kit_colour(k) } else { [0; 3] })
}

/// List a kit's sounds by pad.
fn show_kit(out: &mut Out, kit: &kit::Kit) {
    for s in &kit.sounds {
        match mpc_one::pad_index(s.note) {
            Some(pad) => out.say(&format!("  pad {:2} (note {:3}): {} ({:.2} s)", pad + 1, s.note, s.name,
                                          s.frames() as f64 / RATE as f64)),
            None => out.say(&format!("  note {}: {} (no pad sends this note)", s.note, s.name)),
        }
    }
}

/// Play the first kit in the kits file from the pads, through ACVA.
fn drums(o: &Options) -> Result<(), String> {
    let mut out = Out::new(o, "factory drums", true);
    let mut kits = load_kits(&o.kits, &mut |_, _| {})?;
    kits.truncate(1);
    out.say(&format!("kit: {} ({} sounds)", kits[0].name, kits[0].sounds.len()));
    show_kit(&mut out, &kits[0]);
    let mut rest = [[0u8; 3]; 16];
    for s in &kits[0].sounds {
        if let Some(pad) = mpc_one::pad_index(s.note) {
            rest[pad as usize] = akai::RESTING_COLOURS[pad as usize];
        }
    }
    let mut mixer = kit::Mixer::new(kits);
    let mut ports = Ports::open(&mut out, false)?;
    let colours = pad_colours(&rest);
    ports.send_batch(&mut out, false, "pad colours: lit where the kit has a sound, dark elsewhere", &as_slices(&colours))?;
    let pcm = open_acva(&mut out, PERIODS)?;
    let master = 10f32.powf(o.gain_db / 20.0);
    out.say(&format!("play the pads for {} s; each sound peaks at most {:.0} dB below full scale at full velocity",
                     o.seconds, -o.gain_db));
    out.draw();
    ports.quiet = true;
    let mut play = Playback::start(pcm, PERIODS)?;
    let (mut left, mut right) = ([0f32; PERIOD], [0f32; PERIOD]);
    let mut frames = [0i32; PERIOD * 2];
    let mut buf = [0u8; 256];
    let total_periods = o.seconds * RATE as u64 / PERIOD as u64;
    let (mut hits, mut silent_hits, mut clipped, mut peak, mut voices) = (0u64, 0u64, 0u64, 0f32, 0usize);
    for i in 0..total_periods {
        for m in ports.poll(&mut out, &mut buf)? {
            if let Some(ControlEvent::PadDown { pad, velocity }) = m.event {
                let note = mpc_one::PAD_NOTES.get(pad as usize).copied().unwrap_or(0);
                match mixer.hit(note, velocity) {
                    Some(k) => {
                        hits += 1;
                        let name = mixer.sounds()[k].name.clone();
                        out.note(&format!("    pad {} -> {name}", pad + 1));
                    }
                    None => silent_hits += 1,
                }
            }
            ports.light_hit(&m.event, &rest)?;
        }
        voices = voices.max(mixer.playing());
        let p = mixer.render(&mut left, &mut right, master);
        peak = peak.max(p);
        if p > 1.0 {
            clipped += 1;
        }
        for f in 0..PERIOD {
            frames[2 * f] = float_to_s32(left[f]);
            frames[2 * f + 1] = float_to_s32(right[f]);
        }
        play.write(&frames, i)?;
    }
    let summary = play.finish(&mut out)?;
    ports.quiet = false;
    ports.counts(&mut out);
    out.say(&format!("drums done: {hits} pad hits played, {silent_hits} on pads without a sound, at most {voices} \
                      sounds at once, peak {:.1} dBFS, {clipped} periods clipped; {summary}",
                     20.0 * peak.max(1e-6).log10()));
    Ok(())
}

// ---------------------------------------------------------------- play and app

/// The quietest level the pad player goes down to; the loudest is --gain-db.
const LEVEL_MIN_DB: f32 = -40.0;
/// How far one step of Q-Link 1 moves the level.
const LEVEL_STEP_DB: f32 = 0.5;
/// How long the app waits for each device at power-on.
const DEVICE_WAIT: Duration = Duration::from_secs(30);

/// Pad hit to sound, in milliseconds.
#[derive(Clone, Copy, Debug)]
struct Latency {
    n: u64,
    sum: f64,
    min: f64,
    max: f64,
    last: f64,
}

impl Latency {
    fn new() -> Latency {
        Latency { n: 0, sum: 0.0, min: f64::INFINITY, max: 0.0, last: f64::NAN }
    }

    fn add(&mut self, ms: f64) {
        self.n += 1;
        self.sum += ms;
        self.min = self.min.min(ms);
        self.max = self.max.max(ms);
        self.last = ms;
    }

    fn mean(&self) -> f64 {
        if self.n == 0 { f64::NAN } else { self.sum / self.n as f64 }
    }

    fn describe(&self) -> String {
        if self.n == 0 {
            return "no hits timed".to_string();
        }
        format!("{} hits timed, mean {:.2} ms, min {:.2} ms, max {:.2} ms", self.n, self.mean(), self.min, self.max)
    }
}

/// The time from reading a hit to its first frame leaving the sound card's
/// buffer: the wait until the period holding it was written, plus the frames
/// queued ahead of that period. `delay` is the PCM delay right after the write.
fn hit_latency_ms(read_to_written: Duration, delay: i32) -> f64 {
    let ahead = (delay - PERIOD as i32).max(0) as f64;
    read_to_written.as_secs_f64() * 1e3 + ahead * 1e3 / RATE as f64
}

/// Level from Q-Link 1. On Private (after Akai's setup) it sends steps; on
/// Public (no setup) its absolute position, 0-127 (Hello MPC round 2).
fn qlink_level(level: f32, max: f32, port: &str, bytes: &[u8], delta: i8) -> f32 {
    let next = if port == "Public" && bytes.len() == 3 {
        LEVEL_MIN_DB + (max - LEVEL_MIN_DB) * bytes[2] as f32 / 127.0
    } else {
        level + LEVEL_STEP_DB * delta as f32
    };
    next.clamp(LEVEL_MIN_DB, max)
}

fn level_value(db: f32, max: f32) -> f32 {
    ((db - LEVEL_MIN_DB) / (max - LEVEL_MIN_DB)).clamp(0.0, 1.0)
}

/// What the pad player's screen shows about each kit, then the synth.
fn kit_views(kits: &[kit::Kit], synth: &synth::Synth) -> Vec<gui::KitView> {
    let mut views: Vec<gui::KitView> = kits.iter().enumerate().map(|(k, kit)| {
        let mut pads = vec![None; 16];
        for s in &kit.sounds {
            if let Some(pad) = mpc_one::pad_index(s.note) {
                pads[pad as usize] = Some(s.name.clone());
            }
        }
        gui::KitView { name: kit.name.clone(), pads, colour: akai::kit_colour(k), chip: None }
    }).collect();
    views.push(synth.kit_view());
    views
}

/// The loading steps, for the progress bar.
const STEPS: usize = 6;

/// One loading step on the screen (and in the log).
fn progress(out: &mut Out, step: usize, message: &str, detail: &str, value: f32) {
    out.show(gui::ToUi::Progress { message: message.to_string(),
                                   detail: format!("Step {step} of {STEPS}   ·   {detail}"), value });
}

/// Wait for `find` to succeed, for up to `wait`, showing how long it has been.
fn wait_for<T>(out: &mut Out, step: usize, message: &str, value: f32, wait: Duration,
               mut find: impl FnMut() -> Result<T, String>) -> Result<T, String> {
    let start = Instant::now();
    let mut shown = u64::MAX;
    loop {
        match find() {
            Ok(v) => return Ok(v),
            Err(e) if start.elapsed() >= wait => return Err(e),
            Err(_) => {
                let s = start.elapsed().as_secs();
                if s != shown {
                    shown = s;
                    progress(out, step, message, &format!("Waiting for it to appear ({s} s)"), value);
                }
                thread::sleep(Duration::from_millis(200));
            }
        }
    }
}

/// The pad player: `play` (timed, from the workstation; STOP ends it) or
/// `app` (from power-on, until the power goes off).
fn player(o: &Options, timed: bool) -> Result<(), String> {
    let mut out = Out::new(o, if timed { "pad player" } else { "app" }, false);
    let fonts = load_fonts(o, &mut out);
    let mut fb = None;
    let fb_wait = if timed { Duration::ZERO } else { Duration::from_secs(20) };
    let start = Instant::now();
    loop {
        match open_screen(o) {
            Ok(f) => {
                fb = Some(f);
                break;
            }
            Err(e) if start.elapsed() >= fb_wait => {
                out.say(&format!("screen: not used ({e})"));
                break;
            }
            Err(_) => thread::sleep(Duration::from_millis(250)),
        }
    }
    let size = fb.as_ref().map(|f| f.logical_size()).unwrap_or(gui::SCREEN);
    // The startup screen's last frame stays on the panel; carry on from it.
    let backdrop = match (&fb, o.keep_screen) {
        (Some(f), true) => match mpfree_ui::fb::capture(&o.fb, &f.layout) {
            Ok(img) => Some(std::sync::Arc::new(img)),
            Err(e) => {
                out.say(&format!("screen: could not read the startup screen back ({e}); using the built-in backdrop"));
                None
            }
        },
        _ => None,
    };
    // The screen thread opens the touchscreen, and keeps trying every second
    // until it can: at power-on the app may start before the driver probes.
    let open_touch: gui::OpenTouch = Box::new(move || {
        let (path, name) = touchscreen()?;
        let t = gui::TouchIn::open(&path, size)?;
        println!("touchscreen: {path} ({name}); read-only, not grabbed");
        Ok(t)
    });
    let player = gui::Player::new(fonts, size, timed, backdrop);
    let (to_ui, from_audio) = sync_channel::<gui::ToUi>(4096);
    let (to_audio, from_ui) = sync_channel::<gui::FromUi>(256);
    // The screen thread starts before this thread asks for real-time priority,
    // so it keeps normal priority.
    let screen = gui::spawn(player, fb, Some(open_touch), from_audio, to_audio).map_err(|e| format!("screen thread: {e}"))?;
    out.sink = Some(to_ui.clone());
    if !timed {
        follow_journal(to_ui.clone());
    }
    let result = player_session(o, &mut out, timed, &from_ui);
    if let Err(e) = &result {
        // On the screen only: main prints the STOP line as for every step.
        out.show(gui::ToUi::Session("Stopped".into(), Tone::Bad));
        out.show(gui::ToUi::Failed { message: "Stopped".into(), detail: e.clone() });
        out.show(gui::ToUi::Event(format!("STOP: {e}")));
        if !timed {
            // Keep the screen: the error stays up and the console is a tap away.
            // The power switch is the way out, as for the whole test build.
            out.note(&format!("STOP: {e}"));
            out.note("the app stopped; the screen stays up until the power goes off");
            loop {
                thread::sleep(Duration::from_secs(3600));
            }
        }
    }
    let _ = to_ui.send(gui::ToUi::Quit);
    let report = screen.join().map_err(|_| "the screen thread stopped with an error".to_string())?;
    out.sink = None;
    let drawn = match &report.error {
        Some(e) => format!(" (stopped: {e})"),
        None => String::new(),
    };
    out.say(&format!("screen: {} frames, mean {:.1} ms, slowest {:.1} ms to draw and copy; {} touches; \
                      {} lines the screen had no room for{drawn}",
                     report.frames, report.mean_ms, report.max_ms, report.touches, out.dropped));
    result
}

/// The largest arp pattern library file read (a 64-step pattern with five
/// full lanes is about 1 KB, so this is hundreds of patterns).
const PATTERNS_MAX_BYTES: u64 = 256 * 1024;

/// Reads the arp pattern library file (read-only).
fn read_patterns(path: &Path) -> Result<String, String> {
    let f = File::open(path).map_err(|e| e.to_string())?;
    let mut text = String::new();
    f.take(PATTERNS_MAX_BYTES + 1).read_to_string(&mut text).map_err(|e| e.to_string())?;
    if text.len() as u64 > PATTERNS_MAX_BYTES {
        return Err(format!("larger than {} KB", PATTERNS_MAX_BYTES / 1024));
    }
    Ok(text)
}

/// The system log, followed (`journalctl -f`, read-only) into the Console page.
fn follow_journal(tx: SyncSender<gui::ToUi>) {
    let spawned = thread::Builder::new().name("mpfree-journal".into()).spawn(move || {
        let child = std::process::Command::new("journalctl")
            .args(["-b", "-f", "-n", "300", "--no-pager", "-o", "short-monotonic", "--no-hostname"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn();
        let mut child = match child {
            Ok(c) => c,
            Err(e) => {
                let _ = tx.try_send(gui::ToUi::Console(format!("journalctl: {e} (no system log here)")));
                return;
            }
        };
        let Some(stdout) = child.stdout.take() else { return };
        for line in io::BufRead::lines(io::BufReader::new(stdout)) {
            let Ok(l) = line else { break };
            if let Err(std::sync::mpsc::TrySendError::Disconnected(_)) = tx.try_send(gui::ToUi::Console(l)) {
                break;
            }
        }
        let _ = child.kill();
        let _ = child.wait();
    });
    if let Err(e) = spawned {
        println!("console: {e}");
    }
}

/// Totals over every audio stream of a session (the buffer can change).
#[derive(Default)]
struct AudioTotals {
    periods: u64,
    xruns: u64,
    summaries: Vec<String>,
}

/// How long the sampler engine takes to render a period (the RK3288's figure
/// sets the voice and grain limits).
#[derive(Default)]
struct RenderTime {
    periods: u64,
    total: Duration,
    max: Duration,
    /// Since the last `take_window` (the Performance page's 250 ms).
    window: (u32, Duration, Duration),
}

impl RenderTime {
    fn add(&mut self, d: Duration) {
        self.periods += 1;
        self.total += d;
        self.max = self.max.max(d);
        let w = &mut self.window;
        *w = (w.0 + 1, w.1 + d, w.2.max(d));
    }

    /// Mean and largest render time since the last call, µs.
    fn take_window(&mut self) -> (f32, f32) {
        let (n, total, max) = std::mem::take(&mut self.window);
        let mean = if n == 0 { 0.0 } else { total.as_secs_f32() * 1e6 / n as f32 };
        (mean, max.as_secs_f32() * 1e6)
    }

    fn describe(&self) -> String {
        let period_us = PERIOD as f64 * 1e6 / RATE as f64;
        let mean = if self.periods == 0 { 0.0 } else { self.total.as_secs_f64() * 1e6 / self.periods as f64 };
        let max = self.max.as_secs_f64() * 1e6;
        format!("render mean {mean:.0} us, max {max:.0} us ({:.1} % and {:.1} % of a period)",
                100.0 * mean / period_us, 100.0 * max / period_us)
    }
}

/// The sampler's part of the status line.
fn sampler_status(player: &sampler::Player, time: &RenderTime) -> String {
    let c = player.counters();
    format!("sampler: {}, {} steals, {} events dropped, at most {} grains ({} skipped)",
            time.describe(), c.steals, c.dropped_events, c.peak_grains, c.skipped_grains)
}

/// How often the screen hears the song position while it moves (the bar
/// shows bar.beat.tick; state changes go at once).
const TRANSPORT_EVERY: Duration = Duration::from_millis(33);
/// How often the Sample page's playheads are sent while it shows.
const SAMPLE_HEADS_EVERY: Duration = Duration::from_millis(40);
/// Seconds of stereo audio the looper keeps for all its layers (about
/// 21 MB, allocated once at the start).
const LOOP_SECONDS: u32 = 60;
/// While overdubbing, the loop's waveform is redrawn about once a second.
const LOOP_WAVE_PERIODS: u64 = 344;

/// The sequencer's notes on the pads: the sampler's kit, or the synth while the
/// synth has the pads. Each hit is noted so the screen's pads can flash.
struct SeqPads<'a> {
    player: &'a mut sampler::Player,
    synth: Option<&'a mut synth::Synth>,
    flashes: [u8; 16],
    hits: u64,
}

impl time::Pads for SeqPads<'_> {
    fn hit_at(&mut self, pad: usize, velocity: u8, offset: u32) -> bool {
        let sounded = match self.synth.as_mut() {
            Some(s) if pad < 16 => {
                s.note_on(pad, velocity, offset);
                true
            }
            Some(_) => false,
            None => self.player.hit_at(pad, velocity, offset),
        };
        if sounded {
            self.hits += 1;
            if let Some(f) = self.flashes.get_mut(pad) {
                *f = velocity.max(1);
            }
        }
        sounded
    }
    fn release_at(&mut self, pad: usize, offset: u32) {
        match self.synth.as_mut() {
            Some(s) => s.note_off(pad, offset),
            None => self.player.release_at(pad, offset),
        }
    }
    fn stop_all(&mut self) {
        match self.synth.as_mut() {
            Some(s) => s.all_off(),
            None => self.player.stop_all(),
        }
    }
}

/// The looper's presses and settings, and the line input switched on or
/// off (opening the capture device; a failure leaves it off and says why).
fn looper_action(out: &mut Out, looper: &mut looping::LoopHost, app: &mut app::App, card: u32,
                 a: looping::LoopAction) -> Vec<app::Effect> {
    match a {
        looping::LoopAction::Command(c) => {
            out.note(&looper.command(c));
            Vec::new()
        }
        looping::LoopAction::Source(s) => {
            looper.source = s;
            out.note(&format!("looper: records from {}", s.name().to_lowercase()));
            Vec::new()
        }
        looping::LoopAction::Monitor(on) => {
            looper.monitor = on;
            out.note(&format!("looper: monitor {}", if on { "on: the line input is heard" } else { "off" }));
            Vec::new()
        }
        looping::LoopAction::LineInput(true) => match looper.line.open(card) {
            Ok(line) => {
                out.note(&line);
                Vec::new()
            }
            Err(e) => {
                out.note(&e);
                app.line_input_off()
            }
        },
        looping::LoopAction::LineInput(false) => {
            out.note(&looper.line.close());
            Vec::new()
        }
    }
}

/// A touch or a button for the time engine. The tempo belongs to the app
/// (Note Repeat and the arp follow it too), so the Clips page's tempo goes
/// there; the transport follows it each period. Returns the app's effects.
fn time_action(out: &mut Out, time: &mut time::TimeHost, app: &mut app::App, a: TimeAction) -> Vec<app::Effect> {
    if let TimeAction::Transport(Command::SetTempo(bpm)) = a {
        return app.set_tempo(bpm);
    }
    match time.act(a) {
        Ok(line) if !line.is_empty() => out.note(&format!("time: {line}")),
        Ok(_) => {}
        Err(e) => out.note(&format!("time: {e}")),
    }
    Vec::new()
}

/// The synth's part of the status line.
fn synth_status(synth: &synth::Synth, time: &RenderTime) -> String {
    let c = synth.counters();
    format!("synth: {} ({}), {}, {} notes, at most {} voices ({} set), {} notes dropped, {} resets",
            synth.engine().name(), synth.preset_name(), time.describe(), c.notes, c.peak_voices, synth.polyphony(),
            c.dropped, c.resets)
}

/// The log line for the pads moving to the synth, by an octave or to
/// another layout: "pads: Poly synth, C3 to D#4 (pad 1 = C3; …)", with the
/// scale and layout when a scale is on.
fn synth_pads_note(synth: &synth::Synth) -> String {
    let setup = synth.setup();
    let layout = match setup.scale {
        Some(s) => format!("{}, {} (bank {}), ", synth::scale_name(&s), setup.layout.label(),
                           app::bank_letter(synth.settings().bank)),
        None => String::new(),
    };
    let pad1 = synth.root(0).map_or("no note".to_string(), synth::note_name);
    format!("pads: {} synth, {layout}{} (pad 1 = {pad1}; SHIFT + wheel on Play: octave)", synth.engine().name(),
            synth.range_text())
}

fn player_session(o: &Options, out: &mut Out, timed: bool, from_ui: &Receiver<gui::FromUi>) -> Result<(), String> {
    // 1. Kits.
    progress(out, 1, "Loading drum kits", "Reading the kits file", 0.02);
    let mut kits = load_kits(&o.kits, &mut |done, total| {
        let value = 0.02 + 0.38 * done as f32 / total.max(1) as f32;
        progress(out, 1, "Loading drum kits", &format!("{:.1} of {:.1} MB", done as f64 / 1e6, total as f64 / 1e6), value);
    })?;
    // The sampler engine plays them (and adds the Grains kit when there is room).
    progress(out, 1, "Loading drum kits", "Handing the sounds to the sampler", 0.4);
    let mut player = sampler::Player::new(&mut kits, &mpc_one::PAD_NOTES, RATE)?;
    let sounds: usize = kits.iter().map(|k| k.sounds.len()).sum();
    out.say(&format!("pad player: {} kits, {sounds} sounds", kits.len()));
    for (k, kit) in kits.iter().enumerate() {
        out.say(&format!("kit {}: {} ({} sounds)", k + 1, kit.name, kit.sounds.len()));
    }
    // The synth: one more entry after the kits, and the Synth tab.
    let mut synth = synth::Synth::new(RATE);
    out.say(&format!("synth: {}, preset {}, as kit list entry {} ({}); the Synth tab edits it and picks Poly or FM",
                     synth.engine().name(), synth.preset_name(), kits.len() + 1, synth.range_text()));
    out.show(gui::ToUi::Kits(kit_views(&kits, &synth)));
    out.show(gui::ToUi::Synth(synth.state()));
    // The Sample tab edits the kits' projects on the screen thread.
    out.show(gui::ToUi::SampleKits { projects: player.take_projects(), rate: RATE });
    let drum_kits = kits.len();
    let synth_kit = drum_kits;
    let n = drum_kits + 1;
    let mut synth_on = false;
    let mut param_log = synth::ParamLog::default();
    let mut synth_time = RenderTime::default();
    // Q-Links on the Public port (no Akai session) send positions; the Synth
    // page wants turns, so the last position of each is kept.
    let mut qlink_at: [Option<u8>; 4] = [None; 4];
    let mut synth_meters = synth.meters();
    // Each sounding voice's pressure, for the Synth tab's Pressure page.
    let mut synth_pressures = [0f32; 16];
    let mut shown_pressures = 0usize;
    // The Sample tab: a kit view the screen had no room to take back yet
    // (kept here, never freed on this thread), how many views were swapped
    // in, its playheads, and whether a view changed the playing kit's pads.
    let mut view_back: Option<gui::ToUi> = None;
    let mut views_swapped = 0u64;
    let mut recolour = false;
    let (mut shown_heads, mut heads_at) = (sampler::Playheads::default(), Instant::now());
    // 2. The controller (USB; it may still be starting at power-on).
    let wait = if timed { Duration::ZERO } else { DEVICE_WAIT };
    progress(out, 2, "Finding the pads and buttons", "The controller is a USB device inside the MPC", 0.42);
    let card = wait_for(out, 2, "Finding the pads and buttons", 0.42, wait, midi_card)?;
    out.say(&format!("controller MIDI card: {card}"));
    // 3. The sound card (its driver loads at power-on).
    progress(out, 3, "Finding the sound card", "ACVA, the MPC's own outputs", 0.5);
    let audio = wait_for(out, 3, "Finding the sound card", 0.5, wait, audio_card)?;
    out.say(&format!("sound card: {audio} ({AUDIO_CARD_ID})"));
    // 4. Akai's session, then 5. its setup.
    let mut ports = Ports::open(out, true)?;
    ports.max_shown = u64::MAX;
    let mut rest = kit_pad_colours(&kits[0], 0);
    progress(out, 4, "Connecting to the pads and buttons", "Asking the controller for a session", 0.58);
    let session = start_session(out, &mut ports)?;
    if session {
        progress(out, 5, "Lighting the pads", "Sending Akai's setup, as Akai's app does", 0.72);
        send_setup(out, &mut ports, &rest, "kit 1's pad colours")?;
        out.show(gui::ToUi::Session("Akai session".into(), Tone::Good));
    } else {
        out.say("no session within 20 s: playing without Akai's setup (controls on the Public port, Q-Links absolute); \
                 no setup message was sent");
        ports.send_batch(out, false, "kit 1's pad colours", &as_slices(&pad_colours(&rest)))?;
        out.show(gui::ToUi::Session("No session".into(), Tone::Warn));
    }
    // 6. Sound.
    progress(out, 6, "Starting the sound", "Opening the sound card", 0.9);
    let mut app = app::App::new(RATE, timed);
    // Saved settings and found buttons (--data: the stick's partition 4), read
    // now, before the sound starts with the saved audio buffer. The store
    // saves on its own thread and reports each save as a line.
    let store = match &o.data {
        Some(dir) => {
            let sink = out.sink.clone();
            mpfree_store::Store::at(dir, move |line| match &sink {
                Some(s) => {
                    let _ = s.try_send(gui::ToUi::Line(line));
                }
                None => println!("{line}"),
            })
        }
        None => mpfree_store::Store::none(),
    };
    for line in app.restore(&store.settings(app::saved::APP), &store.settings(app::saved::MAP)) {
        out.say(&line);
    }
    out.say(&match (&o.data, store.saves()) {
        (None, _) => "saving: off (no --data); settings and found buttons reset at every start".to_string(),
        (Some(d), true) => format!("saving: settings and found buttons are kept in {}", d.display()),
        (Some(d), false) => format!("saving: off; {} cannot be used (see the storage line above)", d.display()),
    });
    // The time engine (transport, clips, takes) runs in this loop too.
    let mut time = time::TimeHost::new(RATE, app.settings.bpm)?;
    // The looper follows the transport; the line input stays closed until
    // the Prefs row "Line input" opens it.
    let mut looper = looping::LoopHost::new(RATE, LOOP_SECONDS);
    let mut loop_time = RenderTime::default();
    let mut loop_shown_at = Instant::now();
    out.show(gui::ToUi::Time(Box::new(time.state())));
    let (mut shown_transport, mut transport_shown_at) = (time.info(), Instant::now());
    let mut periods = app.settings.periods;
    let mut play = Playback::start(open_acva(out, periods)?, periods)?;
    progress(out, 6, "Ready", "Have fun", 1.0);
    let max_db = o.gain_db;
    let mut level_db = (max_db - 6.0).max(LEVEL_MIN_DB);
    out.show(gui::ToUi::Level { db: level_db, value: level_value(level_db, max_db) });
    let limit = if timed { format!("for up to {} s", o.seconds) } else { "until the power goes off".to_string() };
    out.say(&format!("playing {limit}: pads or the screen play; the wheel or a tap picks the kit or the synth; Q-Link 1 \
                      or the slider sets the level (now {level_db:.1} dB, at most {max_db:.1} dB), Q-Links 1-4 the \
                      Synth tab's marked knobs; {}",
                     if timed { "STOP ends" } else { "STOP silences" }));
    ports.quiet = true;
    let mut fx = app.start();
    // The arp's own patterns: --patterns FILE (read-only) for this start, or
    // else the ones Save kept in the store (arp/patterns.txt). Save in the
    // pattern workspace writes them back to the store when it saves.
    match &o.patterns {
        Some(path) => match read_patterns(path) {
            Ok(text) => fx.extend(app.load_patterns(&text)),
            Err(e) => out.note(&format!("arp patterns file {}: {e}; the built-in patterns stay", path.display())),
        },
        None => match store.read(app::saved::ARP, app::saved::ARP_PATTERNS).map(String::from_utf8) {
            Some(Ok(text)) => fx.extend(app.load_patterns(&text)),
            Some(Err(_)) => out.note("arp patterns: the saved file is not text; the built-in patterns stay"),
            None => {}
        },
    }
    app.pattern_file = store.saves();
    let (mut left, mut right) = ([0f32; PERIOD], [0f32; PERIOD]);
    let mut frames = [0i32; PERIOD * 2];
    let mut buf = [0u8; 256];
    let mut events = mpfree_arp::Events::new();
    let total_periods = if timed { o.seconds * RATE as u64 / PERIOD as u64 } else { u64::MAX };
    let (mut hits, mut screen_hits, mut silent_hits, mut clipped, mut repeats) = (0u64, 0u64, 0u64, 0u64, 0u64);
    let (mut peak, mut window_peak, mut voices) = (0f32, 0f32, 0usize);
    let mut used = vec![false; n];
    used[0] = true;
    let mut lat = Latency::new();
    let mut pending: Vec<Instant> = Vec::with_capacity(32);
    let mut delay_error = false;
    let (mut pings, mut last_ping, mut last_reply, mut lost) = (0u64, Instant::now(), Instant::now(), false);
    let mut last_timing = Instant::now();
    let mut last_status = Instant::now();
    let started = Instant::now();
    let mut stopped_by = "time";
    // Pads held (on the controller or the screen), pressed but not yet sounded
    // (Note Repeat starts a pad on the next step of its grid), and lit.
    let mut held = [false; 16];
    let mut awaiting = [false; 16];
    let mut lit = [false; 16];
    // Buttons whose light is never sent, noted once each.
    let mut unlit = [false; 128];
    // Repeats per second, for the log (the paced Latch check reads these).
    let (mut window_repeats, mut window_pads, mut last_repeat_note) = (0u64, [false; 16], Instant::now());
    let mut totals = AudioTotals::default();
    let mut render_time = RenderTime::default();
    // CPU by engine: a probe per stage, published every 250 ms for the Dev
    // tab's page (engines.rs). Made here, before the loop: nothing in the
    // loop allocates for it.
    let mut engines = engines::EngineProbes::new();
    out.show(gui::ToUi::Engines(engines.shared()));
    let mut i = 0u64;
    while i < total_periods {
        let t_read = Instant::now();
        let period_span = Span::start();
        let t = started.elapsed().as_secs_f64();
        let mut kit_to: Option<usize> = None;
        let mut audio_change: Option<u32> = None;
        let mut bank_lights = false;
        // Hits are timed only when they sound at once (not on the arp's grid).
        let direct = !app.generating();
        while let Ok(a) = from_ui.try_recv() {
            match a {
                gui::FromUi::Pad { pad, down: true } if pad < 16 => {
                    fx.extend(app.pad_down(pad, app.settings.screen_velocity));
                    held[pad] = true;
                    awaiting[pad] = true;
                    screen_hits += 1;
                    if direct {
                        pending.push(t_read);
                    }
                }
                gui::FromUi::Pad { pad, down: false } if pad < 16 => {
                    fx.extend(app.pad_up(pad));
                    held[pad] = false;
                    awaiting[pad] = false;
                    out.show(gui::ToUi::PadUp(pad));
                }
                gui::FromUi::Pad { .. } => {}
                gui::FromUi::Kit(k) => kit_to = Some(k.min(n - 1)),
                gui::FromUi::Level(v) => {
                    level_db = LEVEL_MIN_DB + v.clamp(0.0, 1.0) * (max_db - LEVEL_MIN_DB);
                    out.show(gui::ToUi::Level { db: level_db, value: level_value(level_db, max_db) });
                }
                gui::FromUi::Tab(k) => fx.extend(app.tab(k)),
                gui::FromUi::MenuSelect(k) => fx.extend(app.select(k)),
                gui::FromUi::MenuStep(k, d) => fx.extend(app.step_item(k, d)),
                gui::FromUi::GuideButton(k) => fx.extend(app.guide_button(k)),
                gui::FromUi::Arp(a) => fx.extend(app.arp_action(a)),
                gui::FromUi::Workspace(a) => fx.extend(app.workspace_action(*a)),
                gui::FromUi::ArpChip => fx.extend(app.toggle_panel()),
                gui::FromUi::Time(a) => fx.extend(time_action(out, &mut time, &mut app, a)),
                gui::FromUi::DebugPage(k) => fx.extend(app.debug_page(k)),
                gui::FromUi::MapRow(k) => fx.extend(app.map_row(k)),
                gui::FromUi::MapButton(k) => fx.extend(app.map_button(k)),
                gui::FromUi::Looper(k) => fx.extend(app.looper_tap(k)),
                gui::FromUi::Synth(SynthAction::Set { id, value }) => {
                    synth.set(id, value);
                    if let Some(line) = param_log.set(&synth, id, value, t_read) {
                        out.note(&line);
                    }
                }
                gui::FromUi::SampleView(mut swap) => {
                    if player.swap_view(&mut swap) {
                        views_swapped += 1;
                        recolour |= swap.kit == player.kit();
                    }
                    // Back to the screen thread, which frees the old view.
                    if let Some(old) = view_back.take() {
                        // Only one view travels at a time, so this is not
                        // expected; the older one is freed here.
                        drop(old);
                        out.note("sample: two kit views waiting to go back; one freed on the audio thread");
                    }
                    view_back = Some(gui::ToUi::SampleViewBack(swap));
                }
                gui::FromUi::Synth(SynthAction::Preset(k)) => {
                    if let Some(line) = param_log.flush(&synth) {
                        out.note(&line);
                    }
                    if let Some(name) = synth.load_preset(k).map(str::to_string) {
                        out.note(&format!("synth: preset {} of {}, {name}", k + 1, synth.preset_count()));
                        out.show(gui::ToUi::SynthValues(synth.values()));
                    }
                }
            }
        }
        if let Some(line) = param_log.poll(&synth, t_read) {
            out.note(&line);
        }
        if let Some(m) = view_back.take() {
            view_back = out.try_show(m).err();
        }
        ports.show_pressure = app.settings.show_pressure;
        for m in ports.poll(out, &mut buf)? {
            if let Some((token, status)) = akai::session_reply(&m.bytes) {
                if token == akai::SESSION_TOKEN && status == akai::SESSION_RUNNING {
                    last_reply = Instant::now();
                    if lost {
                        lost = false;
                        out.show(gui::ToUi::Session("Akai session".into(), Tone::Good));
                    }
                }
            }
            match m.event {
                Some(ControlEvent::PadDown { pad, velocity }) if (pad as usize) < 16 => {
                    let pad = pad as usize;
                    fx.extend(app.pad_down(pad, velocity));
                    held[pad] = true;
                    awaiting[pad] = true;
                    if direct {
                        pending.push(t_read);
                    }
                }
                Some(ControlEvent::PadUp { pad }) if (pad as usize) < 16 => {
                    fx.extend(app.pad_up(pad as usize));
                    held[pad as usize] = false;
                    awaiting[pad as usize] = false;
                    out.show(gui::ToUi::PadUp(pad as usize));
                }
                Some(ControlEvent::PadPressure { pad, pressure }) if (pad as usize) < 16 => {
                    app.pressure(pad as usize, pressure);
                    if synth_on && app.arp_plays_notes() {
                        // The pad's layout notes, which the arp pressed by number.
                        let notes = synth.pad_layout()[pad as usize];
                        for &note in notes.notes() {
                            synth.pressure_note(note, pressure);
                        }
                    } else if synth_on {
                        synth.pressure(pad as usize, pressure);
                    } else {
                        player.pressure(pad as usize, pressure);
                    }
                }
                Some(ControlEvent::DataWheel { delta }) => fx.extend(app.wheel(delta as i32)),
                Some(ControlEvent::QLinkTurn { knob, delta }) => {
                    // On the Synth page Q-Links 1-4 turn the marked knobs; while NOTE REPEAT is held
                    // or the Arp panel is open they set the arp; otherwise Q-Link 1 is the level.
                    let public = MIDI_PORTS[m.port].1 == "Public" && m.bytes.len() == 3;
                    // On the Sample page they move Start, End and the loop.
                    let page = app.view();
                    if matches!(page, app::View::Synth | app::View::Sample) && knob < 4 {
                        let k = knob as usize;
                        let ticks = if public {
                            let was = qlink_at[k].replace(m.bytes[2]);
                            was.map_or(0, |w| m.bytes[2] as i32 - w as i32)
                        } else {
                            delta as i32
                        };
                        if ticks != 0 && page == app::View::Synth {
                            out.show(gui::ToUi::SynthQLink(k, ticks));
                        } else if ticks != 0 {
                            out.show(gui::ToUi::SampleQLink(k, ticks));
                        }
                    } else if let Some(e) = app.qlink(knob as usize, delta as i32, public.then(|| m.bytes[2])) {
                        fx.extend(e);
                    } else if knob == 0 {
                        level_db = qlink_level(level_db, max_db, MIDI_PORTS[m.port].1, &m.bytes, delta);
                        if public {
                            qlink_at[0] = Some(m.bytes[2]);
                        }
                        out.show(gui::ToUi::Level { db: level_db, value: level_value(level_db, max_db) });
                    }
                }
                Some(ControlEvent::Button { note, pressed }) => fx.extend(app.button(note, pressed, t)),
                // The control mapper names the Q-Links and takes CCs nothing else knows.
                Some(ControlEvent::QLinkTouch { knob, touched: true }) if (knob as usize) < 4 => {
                    fx.extend(app.known_control(app::Code::Button(mpc_one::QLINK_TOUCH_NOTES[knob as usize])));
                }
                // Letting go of a Q-Link ends its turn on the Sample page (one undo step).
                Some(ControlEvent::QLinkTouch { knob, touched: false }) if (knob as usize) < 4
                    && app.view() == app::View::Sample => out.show(gui::ToUi::SampleLift),
                None if m.bytes.len() == 3 && m.bytes[0] == 0xB0 => fx.extend(app.unknown_cc(m.bytes[1])),
                _ => {}
            }
            if let Some(ControlEvent::QLinkTurn { knob, .. }) = m.event {
                if let Some(&cc) = mpc_one::QLINK_TURN_CC.get(knob as usize) {
                    fx.extend(app.known_control(app::Code::Cc(cc)));
                }
            }
        }
        fx.extend(app.tick(t));
        let mut stop = false;
        let mut later = Vec::new();
        for e in fx.drain(..) {
            match e {
                app::Effect::Light(note, value) if akai::app_lights(note) => {
                    ports.send(true, &akai::button_light(note, value))?
                }
                app::Effect::Light(note, _) => {
                    if !unlit[note as usize & 0x7F] {
                        unlit[note as usize & 0x7F] = true;
                        out.note(&format!("button {note}: no light sent (Akai's app was never seen lighting it)"));
                    }
                }
                app::Effect::Kit(d) => {
                    let now = if synth_on { synth_kit } else { player.kit() };
                    let from = kit_to.unwrap_or(now) as i64;
                    kit_to = Some((from + d as i64).rem_euclid(n as i64) as usize);
                }
                app::Effect::Octave(d) if synth_on || kit_to == Some(synth_kit) => {
                    if synth.shift_octave(d) {
                        if synth_on {
                            app.set_pad_layout(Some(synth.pad_layout()));
                        }
                        out.note(&synth_pads_note(&synth));
                        out.show(gui::ToUi::KitChanged { kit: synth_kit, view: synth.kit_view() });
                        rest = synth.pad_colours();
                        ports.send_batch(out, false, "the synth's pad colours", &as_slices(&pad_colours(&rest)))?;
                        lit = [false; 16];
                    }
                }
                // SHIFT + wheel with a drum kit: the kit, as before.
                app::Effect::Octave(d) => {
                    let from = kit_to.unwrap_or(player.kit()) as i64;
                    kit_to = Some((from + d as i64).rem_euclid(n as i64) as usize);
                }
                // The Synth tab's options: the engine, the scale and the
                // pad layouts.
                app::Effect::SynthPads(p) => {
                    if let Some(line) = param_log.flush(&synth) {
                        out.note(&line);
                    }
                    if synth.set_settings(p) {
                        out.note(&format!("synth: {} on the pads, preset {}", synth.engine().name(),
                                          synth.preset_name()));
                        out.show(gui::ToUi::Synth(synth.state()));
                    }
                    out.note(&synth_pads_note(&synth));
                    out.show(gui::ToUi::KitChanged { kit: synth_kit, view: synth.kit_view() });
                    if synth_on {
                        app.set_pad_layout(Some(synth.pad_layout()));
                        rest = synth.pad_colours();
                        ports.send_batch(out, false, "the synth's pad colours", &as_slices(&pad_colours(&rest)))?;
                        lit = [false; 16];
                        bank_lights = true;
                    }
                }
                app::Effect::SynthWheel(d) => out.show(gui::ToUi::SynthWheel(d)),
                app::Effect::SynthPage(d) => out.show(gui::ToUi::SynthPage(d)),
                app::Effect::SampleWheel(d) => out.show(gui::ToUi::SampleWheel(d)),
                app::Effect::SamplePad(d) => out.show(gui::ToUi::SamplePad(d)),
                app::Effect::SampleUndo { redo } => out.show(gui::ToUi::SampleUndo(redo)),
                app::Effect::StopSounds => {
                    player.stop_all();
                    synth.stop();
                }
                app::Effect::Audio(p) => audio_change = Some(p),
                app::Effect::Note(s) => out.note(&s),
                app::Effect::Quit => stop = true,
                app::Effect::Show(v) => {
                    // Opening the Synth tab hands the pads to the synth, and
                    // opening the Sample tab gives them back to the drum kit.
                    if v == app::View::Synth && !synth_on {
                        kit_to = Some(synth_kit);
                    } else if v == app::View::Sample && synth_on {
                        kit_to = Some(player.kit());
                    }
                    out.show(gui::ToUi::Show(v));
                }
                app::Effect::Menu(m) => out.show(gui::ToUi::Menu(m)),
                app::Effect::Guide(g) => out.show(gui::ToUi::Guide(g)),
                app::Effect::Status(s) => out.show(gui::ToUi::Status(s)),
                app::Effect::Settings(s) => out.show(gui::ToUi::Settings(s)),
                app::Effect::ClearLogs => out.show(gui::ToUi::ClearLogs),
                app::Effect::ClearConsole => out.show(gui::ToUi::ClearConsole),
                app::Effect::TouchLab => out.show(gui::ToUi::TouchLab),
                app::Effect::CalibrateTouch => out.show(gui::ToUi::CalibrateTouch),
                app::Effect::Arp(v) => out.show(gui::ToUi::Arp(Box::new(v))),
                app::Effect::Patterns(list) => out.show(gui::ToUi::Patterns(list)),
                app::Effect::Workspace(w) => out.show(gui::ToUi::Workspace(Box::new(w))),
                app::Effect::Recorded(p, next) => out.show(gui::ToUi::Recorded(Box::new(p), next)),
                app::Effect::WorkspaceWheel(d) => out.show(gui::ToUi::WorkspaceWheel(d)),
                app::Effect::ClipsWheel { delta, scroll } => out.show(gui::ToUi::ClipsWheel { delta, scroll }),
                app::Effect::Map(m) => out.show(gui::ToUi::Map(Box::new(m))),
                // A PAD BANK button: banks A-H are kits 1-8 (9-16 and the
                // Grains kit are banks A-H again, from kit 9 on).
                // While the synth has the pads, PAD BANK A-H picks a bank's
                // pad layout (Jay, 09:55Z), not a drum kit.
                app::Effect::Bank { button, shift }
                    if button < 4 && kit_to.unwrap_or(if synth_on { synth_kit } else { player.kit() }) == synth_kit =>
                {
                    let target = app::bank_target(synth.settings().bank % 8, button, shift);
                    later.extend(app.synth_bank(target));
                    bank_lights = true;
                }
                app::Effect::Bank { button, shift } => {
                    let now = kit_to.unwrap_or(if synth_on { synth_kit } else { player.kit() });
                    if button < 4 {
                        let group = if now >= drum_kits { 0 } else { now / 8 * 8 };
                        let target = group + app::bank_target(now % 8, button, shift);
                        if target < drum_kits {
                            kit_to = Some(target);
                            out.note(&format!("bank: {} (kit {}: {})", app::bank_letter(target), target + 1,
                                              kits[target].name));
                        } else {
                            out.note(&format!("bank {}: no kit there ({} kits)",
                                              app::bank_letter(group + app::bank_target(now % 8, button, shift)),
                                              drum_kits));
                        }
                    }
                    bank_lights = true;
                }
                // What it gives back (the tempo's status line and note) is
                // handled with the next period's effects.
                app::Effect::Time(a) => later.extend(time_action(out, &mut time, &mut app, a)),
                app::Effect::Looper(a) => later.extend(looper_action(out, &mut looper, &mut app, audio, a)),
                app::Effect::SavePatterns(text) => {
                    // The store's own thread writes it and reports the save.
                    if let Err(e) = store.write(app::saved::ARP, app::saved::ARP_PATTERNS, text.into_bytes()) {
                        out.note(&format!("arp patterns: not saved ({e})"));
                    }
                }
            }
        }
        fx.append(&mut later);
        if let Some(p) = audio_change {
            totals.periods += play.periods;
            totals.xruns += play.xruns;
            totals.summaries.push(format!("{periods} periods: {}", play.finish(out)?));
            periods = p;
            out.note(&format!("audio buffer: {p} periods ({:.1} ms); starting the sound again",
                              p as f64 * period_time().as_secs_f64() * 1e3));
            play = Playback::start(open_acva(out, p)?, p)?;
            pending.clear();
        }
        let playing_kit = if synth_on { synth_kit } else { player.kit() };
        if let Some(k) = kit_to.filter(|&k| k != playing_kit) {
            used[k] = true;
            if k == synth_kit {
                // The sampler keeps its kit; its sounds finish.
                synth_on = true;
                rest = synth.pad_colours();
                out.note(&synth_pads_note(&synth));
                ports.send_batch(out, false, "the synth's pad colours", &as_slices(&pad_colours(&rest)))?;
            } else {
                if synth_on {
                    synth_on = false;
                    synth.all_off();
                }
                player.select(k);
                rest = sound_colours(player.sounds(k), k);
                out.note(&format!("kit {}: {} ({} sounds)", k + 1, kits[k].name, kits[k].sounds.len()));
                ports.send_batch(out, false, &format!("kit {}'s pad colours", k + 1), &as_slices(&pad_colours(&rest)))?;
            }
            lit = [false; 16];
            // The arp plays the synth's layout notes, or a kit's pad slots.
            app.set_pad_layout(synth_on.then(|| synth.pad_layout()));
            out.show(gui::ToUi::Kit(k));
            bank_lights = true;
        }
        // The PAD BANK buttons found so far: the bank playing is lit.
        if bank_lights {
            let bank = Some(if synth_on { synth.settings().bank % 4 } else { player.kit() % 4 });
            for (b, note) in app.map.bank.iter().enumerate() {
                let Some(note) = *note else { continue };
                let value = if bank == Some(b) { app::LIGHT_ON } else { app::LIGHT_OFF };
                if akai::app_lights(note) {
                    ports.send(true, &akai::button_light(note, value))?;
                } else if !unlit[note as usize & 0x7F] {
                    unlit[note as usize & 0x7F] = true;
                    out.note(&format!("button {note}: no light sent (Akai's app was never seen lighting it)"));
                }
            }
        }
        let time_span = Span::start();
        app.set_rolling(time.info().state.rolling());
        time.follow_tempo(app.settings.bpm);
        // The transport runs first, so the arp can follow this period's
        // clock (TimeHost::arp_clock); the clips play and record after.
        time.advance(PERIOD as u32);
        // The arp follows this period's transport (None while stopped: it runs on its own).
        app.set_clock(time.arp_clock());
        app.process(PERIOD as u32, &mut events);
        engines.stop(engines::TIME, time_span);
        if let Some(app::Effect::Arp(v)) = app.arp_poll() {
            out.show(gui::ToUi::Arp(Box::new(v)));
        }
        // With the synth on the pads the arp's events carry MIDI notes (its
        // pad layout); the screen and the clips get the pad a note is on.
        let notes = synth_on && app.arp_plays_notes();
        for ev in events.as_slice() {
            let (slot, velocity) = match ev.kind {
                mpfree_arp::EventKind::On { note, velocity } => (note, velocity),
                mpfree_arp::EventKind::Off { note } if notes => {
                    synth.note_off_note(note, ev.offset);
                    if let Some(pad) = app.pad_of_note(note) {
                        time.pad(ev.offset, mpfree_play::Event::NoteOff { note: pad as u8 });
                    }
                    continue;
                }
                // The synth's note ends; a Grains pad stops when let go; one-shot sounds ignore it.
                mpfree_arp::EventKind::Off { note } if synth_on => {
                    synth.note_off(note as usize, ev.offset);
                    time.pad(ev.offset, mpfree_play::Event::NoteOff { note });
                    continue;
                }
                mpfree_arp::EventKind::Off { note } => {
                    player.release_at(note as usize, ev.offset);
                    time.pad(ev.offset, mpfree_play::Event::NoteOff { note });
                    continue;
                }
            };
            let v = app.velocity(velocity);
            let pad = if notes {
                // A note no pad has (an arp octave up, a scale run) still
                // plays; only a pad's own note shows and records.
                synth.note_on_note(slot, v, ev.offset);
                hits += 1;
                let Some(pad) = app.pad_of_note(slot) else { continue };
                time.pad(ev.offset, mpfree_play::Event::NoteOn { note: pad as u8, velocity: v });
                pad
            } else {
                slot as usize
            };
            if pad >= 16 {
                continue;
            }
            if !notes {
                // Recorded into the armed track while the transport records.
                time.pad(ev.offset, mpfree_play::Event::NoteOn { note: slot, velocity: v });
                if synth_on {
                    synth.note_on(pad, v, ev.offset);
                    hits += 1;
                } else if player.hit_at(pad, v, ev.offset) {
                    hits += 1;
                } else {
                    silent_hits += 1;
                }
            }
            if awaiting[pad] {
                awaiting[pad] = false;
                out.show(gui::ToUi::PadDown { pad, velocity: v });
            } else {
                repeats += 1;
                window_repeats += 1;
                window_pads[pad] = true;
                out.show(gui::ToUi::PadFlash { pad, velocity: v });
            }
        }
        // The clips play the kit's pads; the screen's pads flash with them.
        let mut seq = SeqPads { player: &mut player, synth: synth_on.then_some(&mut synth), flashes: [0; 16], hits: 0 };
        time.period(PERIOD as u32, &mut seq);
        hits += seq.hits;
        for (pad, &v) in seq.flashes.iter().enumerate().filter(|(_, v)| **v > 0) {
            out.show(gui::ToUi::PadFlash { pad, velocity: v });
        }
        // The Sample page's playheads, about 25 times a second while it shows.
        if app.view() == app::View::Sample && heads_at.elapsed() >= SAMPLE_HEADS_EVERY {
            let heads = player.playheads();
            if heads != shown_heads {
                out.show(gui::ToUi::SamplePlayheads(heads));
                shown_heads = heads;
            }
            heads_at = Instant::now();
        }
        if time.take_changed() {
            out.show(gui::ToUi::Time(Box::new(time.state())));
            (shown_transport, transport_shown_at) = (time.info(), Instant::now());
        } else {
            let now = time.info();
            let moved = now.position != shown_transport.position;
            let changed = TransportInfo { position: shown_transport.position, ..now } != shown_transport;
            if changed || moved && transport_shown_at.elapsed() >= TRANSPORT_EVERY {
                out.show(gui::ToUi::Transport(now));
                (shown_transport, transport_shown_at) = (now, Instant::now());
            }
        }
        // A chop (or its undo) changed which pads of the playing kit have a
        // sound: their resting colours follow.
        if std::mem::take(&mut recolour) && !synth_on {
            let fresh = sound_colours(player.sounds(player.kit()), player.kit());
            for pad in 0..16 {
                if fresh[pad] != rest[pad] {
                    rest[pad] = fresh[pad];
                    if !lit[pad] {
                        ports.send(false, &akai::pad_colour(pad as u8, rest[pad]))?;
                    }
                }
            }
        }
        // Pad lights: lit while held, or while the arp keeps them going (latched, or held on its grid).
        for pad in 0..16 {
            let want = held[pad] || (app.generating() && app.active(pad));
            if want != lit[pad] {
                lit[pad] = want;
                let colour = if want { akai::hit_colour(rest[pad]) } else { rest[pad] };
                ports.send(false, &akai::pad_colour(pad as u8, colour))?;
            }
        }
        if session && last_ping.elapsed() >= Duration::from_secs(akai::PING_SECONDS) {
            let ping = akai::session_ping(akai::SESSION_TOKEN);
            ports.send(false, &ping)?;
            pings += 1;
            last_ping = Instant::now();
            out.midi(&format!("+{:.3} s -> Public: {} = session ping", ports.t(), hex(&ping)));
        }
        if session && !lost && last_reply.elapsed() > Duration::from_secs(10) {
            lost = true;
            out.note("no session reply for 10 s");
            out.show(gui::ToUi::Session("No reply".into(), Tone::Bad));
        }
        voices = voices.max(player.playing() + synth.active_voices());
        let t_render = Instant::now();
        player.render_dry(&mut left, &mut right);
        let t_synth = Instant::now();
        render_time.add(t_synth - t_render);
        engines.add(engines::SAMPLER, t_synth - t_render);
        synth.render(i * PERIOD as u64, app.settings.bpm, &mut left, &mut right);
        let synth_took = t_synth.elapsed();
        synth_time.add(synth_took);
        engines.add(engines::SYNTH, synth_took);
        let t_mix = Span::start();
        let p = sampler::master_out(&mut left, &mut right, 10f32.powf(level_db / 20.0));
        // What the looper records from the pads: before the click.
        looper.take_pads(&left, &right);
        // The metronome goes on top; it keeps the samples within ±1 itself.
        let p = p.max(time.mix_click(&mut left, &mut right));
        engines.stop(engines::MIX, t_mix);
        // The loop (and the input, with Monitor on) goes on top too.
        let t_loop = Instant::now();
        let (loop_peak, line_note) = looper.period(time.block(), &mut left, &mut right);
        let loop_took = t_loop.elapsed();
        loop_time.add(loop_took);
        engines.add(engines::LOOPER, loop_took);
        let p = p.max(loop_peak);
        if let Some(note) = line_note {
            out.note(&note);
            // The input closed itself after errors: its row says Off again
            // and the looper records the pads (next period's effects).
            if !looper.line.is_open() && app.settings.line_input {
                fx.extend(app.line_input_off());
            }
        }
        for line in looper.changes(|| time.position_text()) {
            out.note(&line);
        }
        if let Some(wave) = looper.wave_step(i, LOOP_WAVE_PERIODS) {
            out.show(gui::ToUi::LoopWave(wave));
        }
        if app.view() == app::View::Looper && loop_shown_at.elapsed() >= TRANSPORT_EVERY {
            loop_shown_at = Instant::now();
            out.show(gui::ToUi::Looper(Box::new(looper.view(app.settings.bpm))));
        }
        peak = peak.max(p);
        window_peak = window_peak.max(p);
        if p > 1.0 {
            clipped += 1;
        }
        for f in 0..PERIOD {
            frames[2 * f] = float_to_s32(left[f]);
            frames[2 * f + 1] = float_to_s32(right[f]);
        }
        let xruns = play.xruns;
        engines.stop(engines::PERIOD, period_span);
        play.write(&frames, i)?;
        if !pending.is_empty() {
            let written = Instant::now();
            match play.pcm.delay() {
                // An underrun in this write dropped the period: no timing for it.
                Ok(d) if play.xruns == xruns => {
                    for t in pending.drain(..) {
                        lat.add(hit_latency_ms(written - t, d));
                    }
                }
                Ok(_) => pending.clear(),
                Err(e) => {
                    pending.clear();
                    if !delay_error {
                        delay_error = true;
                        out.note(&format!("PCM delay: {e} (no timing)"));
                    }
                }
            }
        }
        if last_repeat_note.elapsed() >= Duration::from_secs(1) {
            last_repeat_note = Instant::now();
            if window_repeats > 0 {
                let list: Vec<String> = (0..16).filter(|&p| window_pads[p]).map(|p| (p + 1).to_string()).collect();
                let latched: Vec<String> = (0..16).filter(|&p| app.active(p) && !held[p]).map(|p| (p + 1).to_string())
                    .collect();
                out.note(&format!("repeat: {window_repeats} hits in the last second on pads {}; latched: {}; \
                                   engine dropped {} events, overflow {}",
                                  list.join(" "), if latched.is_empty() { "none".to_string() } else { latched.join(" ") },
                                  events.dropped, app.engine.overflow));
            }
            window_repeats = 0;
            window_pads = [false; 16];
        }
        // Every 30 s, the totals so far, for the system log (the app runs until the power goes off, so it
        // never prints a final summary; the workstation's paced check reads these).
        if last_status.elapsed() >= Duration::from_secs(30) {
            last_status = Instant::now();
            out.note(&format!("status: {hits} hits ({screen_hits} from the screen), {repeats} repeats, {} underruns \
                               in {} periods, at most {voices} sounds at once, peak {:.1} dBFS, {clipped} periods \
                               clipped; pad to sound {}; note engine dropped {} events, overflow {}; {}; {}",
                              totals.xruns + play.xruns, totals.periods + play.periods,
                              20.0 * peak.max(1e-6).log10(), lat.describe(), events.dropped, app.engine.overflow,
                              sampler_status(&player, &render_time), time.describe()));
            out.note(&format!("status: {}", synth_status(&synth, &synth_time)));
            out.note(&format!("status: {}; {}", looper.describe(), loop_time.describe()));
            out.note(&format!("status: {}", engines::log_line(&engines.lifetime())));
        }
        if last_timing.elapsed() >= Duration::from_millis(250) {
            last_timing = Instant::now();
            let seconds_left = timed.then(|| o.seconds.saturating_sub(started.elapsed().as_secs()));
            out.show(gui::ToUi::Timing(gui::Timing {
                hits, periods: totals.periods + play.periods, underruns: totals.xruns + play.xruns,
                voices: player.playing() + synth.active_voices(), peak_db: 20.0 * window_peak.max(1e-6).log10(), last_ms: lat.last as f32,
                min_ms: lat.min as f32, mean_ms: lat.mean() as f32, max_ms: lat.max as f32, seconds_left,
                periods_queued: periods, audio: {
                    let (render_mean_us, render_max_us) = render_time.take_window();
                    let (synth_mean_us, synth_max_us) = synth_time.take_window();
                    let c = player.counters();
                    debug_page::AudioNumbers {
                        render_mean_us, render_max_us, synth_mean_us, synth_max_us,
                        worst_us: (render_time.max + synth_time.max).as_secs_f32() * 1e6, steals: c.steals,
                        dropped: c.dropped_events + events.dropped, peak_grains: c.peak_grains,
                    }
                },
            }));
            window_peak = 0.0;
            engines.publish();
            // Prefs or found buttons changed: the store's thread writes them.
            for (owner, settings) in app.take_saves() {
                if store.saves() {
                    if let Err(e) = store.save_settings(owner, &settings) {
                        out.note(&format!("not saved: {owner}: {e}"));
                    }
                }
            }
            if app.view() != app::View::Looper {
                out.show(gui::ToUi::Looper(Box::new(looper.view(app.settings.bpm))));
            }
            let meters = synth.meters();
            if meters != synth_meters {
                out.show(gui::ToUi::SynthMeters(meters.clone()));
                synth_meters = meters;
            }
            let mut bars = [0f32; 16];
            let n = synth.pressures(&mut bars);
            if (n, bars) != (shown_pressures, synth_pressures) {
                out.show(gui::ToUi::SynthPressures(bars[..n].to_vec()));
                (shown_pressures, synth_pressures) = (n, bars);
            }
        }
        i += 1;
        if stop {
            stopped_by = "STOP";
            break;
        }
    }
    let summary = play.finish(out)?;
    totals.summaries.push(format!("{periods} periods: {summary}"));
    ports.quiet = false;
    ports.counts(out);
    let kits_used: Vec<String> = used.iter().enumerate().filter(|(_, u)| **u).map(|(k, _)| (k + 1).to_string()).collect();
    out.say(&format!("pad to sound (read to leaving the card's buffer; plus up to one period, {:.1} ms, before a \
                      message is read): {}", period_time().as_secs_f64() * 1e3, lat.describe()));
    out.say(&format!("buttons: {}", app.map.describe()));
    out.say(&format!("note engine: {repeats} repeats, {} events dropped, {} notes that could not be queued",
                     events.dropped, app.engine.overflow));
    out.say(&format!("play done ({stopped_by}): {hits} hits played ({screen_hits} from the screen), {silent_hits} on \
                      pads without a sound, kits used {}, level {level_db:.1} dB, at most {voices} sounds at once, \
                      peak {:.1} dBFS, {clipped} periods clipped, {pings} pings; {}",
                     kits_used.join(" "), 20.0 * peak.max(1e-6).log10(), totals.summaries.join("; ")));
    out.say(&sampler_status(&player, &render_time));
    if views_swapped > 0 {
        out.say(&format!("sample tab: {views_swapped} edited kit views swapped in"));
    }
    out.say(&time.describe());
    if looper.line.is_open() {
        out.say(&looper.line.close());
    }
    out.say(&format!("{}; {}", looper.describe(), loop_time.describe()));
    if let Some(line) = param_log.flush(&synth) {
        out.say(&line);
    }
    out.say(&synth_status(&synth, &synth_time));
    // Whatever is still queued is written before the session ends.
    for (owner, settings) in app.take_saves() {
        if store.saves() {
            let _ = store.save_settings(owner, &settings);
        }
    }
    store.flush();
    out.say(&engines::log_line(&engines.lifetime()));
    if session {
        out.say("no more pings: the controller should drop the session by itself (inferred: within about 10 s)");
    }
    Ok(())
}

fn pads(o: &Options) -> Result<(), String> {
    let heading = if o.keepalive { "pads, with keep-alive" } else { "pads (listen only)" };
    let mut out = Out::new(o, heading, true);
    let card = midi_card()?;
    let mut inputs = open_inputs(card, &mut out)?;
    let mut output = None;
    if o.keepalive {
        let (f, oi) = alsa::open_rawmidi(card, mpc_one::Port::Public as u32, alsa::Direction::Output)
            .map_err(|e| format!("opening the Public MIDI port: {e}"))?;
        out.say(&format!("keep-alive goes to card {card} subdevice {} \"{}\" every 5 s", oi.subdevice, oi.subname));
        if oi.subdevice != 0 || oi.stream != alsa::RAWMIDI_STREAM_OUTPUT || !oi.subname.contains("Public") {
            return Err("that is not the controller's Public port; stopping without sending".to_string());
        }
        output = Some(f);
    }
    out.say("every message is shown in hex, with its meaning where we know it");
    out.say(&format!("hit some pads, press a button, turn a Q-Link and the wheel ({} s)", o.seconds));
    let mut buf = [0u8; 256];
    let (mut pad_hits, mut pings, mut replies, mut unknown, mut pressure, mut shown) = (0u64, 0u64, 0u64, 0u64, 0u64, 0u64);
    let mut last_ping: Option<Instant> = None;
    let start = Instant::now();
    let end = start + Duration::from_secs(o.seconds);
    while Instant::now() < end {
        if let Some(f) = output.as_mut() {
            if last_ping.is_none_or(|t| t.elapsed() >= Duration::from_secs(5)) {
                let (_, msg) = mpc_one::keepalive_ping(OBSERVED_KEEPALIVE_TOKEN);
                f.write_all(&msg).map_err(|e| format!("sending the keep-alive: {e}"))?;
                pings += 1;
                last_ping = Some(Instant::now());
                out.say(&format!("+{:.3} s sent keep-alive {pings} on Public: {}", start.elapsed().as_secs_f64(), hex(&msg)));
            }
        }
        let mut got = false;
        for port in inputs.iter_mut() {
            match port.file.read(&mut buf) {
                Ok(0) => {}
                Ok(n) => {
                    got = true;
                    port.bytes += n as u64;
                    let t = start.elapsed().as_secs_f64();
                    for &b in &buf[..n] {
                        if let Some(msg) = port.parser.feed(b) {
                            port.messages += 1;
                            let event = mpc_one::decode(&msg);
                            let (bytes, truncated) = message_bytes(&msg);
                            match event {
                                Some(ControlEvent::PadDown { .. }) => pad_hits += 1,
                                Some(ControlEvent::KeepAliveReply { .. }) => replies += 1,
                                Some(ControlEvent::PadPressure { .. }) => pressure += 1,
                                None => unknown += 1,
                                _ => {}
                            }
                            if matches!(event, Some(ControlEvent::PadPressure { .. })) {
                                continue;
                            }
                            shown += 1;
                            if shown <= MAX_SHOWN {
                                out.say(&message_line(t, port.name, &bytes, truncated, event.as_ref()));
                            } else if shown == MAX_SHOWN + 1 {
                                out.say(&format!("(more than {MAX_SHOWN} messages: only counting from here)"));
                            }
                        }
                    }
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock || e.raw_os_error() == Some(EAGAIN) => {}
                Err(e) => return Err(format!("reading MIDI ({}): {e}", port.name)),
            }
        }
        if !got {
            thread::sleep(Duration::from_millis(2));
        }
    }
    for port in &inputs {
        out.say(&format!("{}: {} bytes, {} messages, {} stray bytes", port.name, port.bytes, port.messages, port.parser.stray_bytes));
    }
    let bytes: u64 = inputs.iter().map(|p| p.bytes).sum();
    let messages: u64 = inputs.iter().map(|p| p.messages).sum();
    out.say(&format!("pads done: {bytes} bytes, {messages} messages, {pad_hits} pad hits, {pressure} pressure, \
                      {unknown} not recognised, {pings} keep-alives sent, {replies} replies"));
    drop(inputs);
    drop(output);
    dump(&mut out, &format!("/proc/asound/card{card}/midi0"));
    Ok(())
}

// ---------------------------------------------------------------- click

/// Open ACVA at the stock stream settings (S32_LE, 44.1 kHz, 128-frame
/// periods; 3 periods as the stock app, or 2 from the pad player's Prefs),
/// lock memory and ask for SCHED_FIFO. Stops before playing if anything differs.
fn open_acva(out: &mut Out, periods: u32) -> Result<alsa::Pcm, String> {
    let card = audio_card()?;
    let path = format!("/dev/snd/pcmC{card}D0p");
    out.say(&format!("opening {path} ({AUDIO_CARD_ID})"));
    let pcm = alsa::Pcm::open_playback(&path).map_err(|e| format!("{path}: {e}"))?;
    let mut hw = alsa::HwParams::stock_playback(periods);
    if let Err(e) = pcm.hw_params(&mut hw) {
        out.say(&format!("the stock stream settings were refused: {e}"));
        let mut any = alsa::HwParams::format_only();
        match pcm.refine(&mut any) {
            Ok(()) => out.say(&format!("what the card allows with stereo S32_LE at 44.1 kHz: period size {}, \
                                        periods {}, buffer size {}",
                                       any.interval(alsa::PARAM_PERIOD_SIZE).describe(),
                                       any.interval(alsa::PARAM_PERIODS).describe(),
                                       any.interval(alsa::PARAM_BUFFER_SIZE).describe())),
            Err(e2) => out.say(&format!("asking what it allows failed too: {e2}")),
        }
        return Err("no sound played".to_string());
    }
    if !hw.mask_has(alsa::PARAM_ACCESS, alsa::ACCESS_RW_INTERLEAVED) || !hw.mask_has(alsa::PARAM_FORMAT, alsa::FORMAT_S32_LE) {
        return Err("the card did not keep interleaved S32_LE; stopping before playing".to_string());
    }
    let buffer = hw.interval(alsa::PARAM_BUFFER_SIZE).single().unwrap_or(0);
    let (num, den) = hw.rate_fraction();
    out.say(&format!("stream: rate {} ({num}/{den}), channels {}, period {} frames, periods {}, buffer {} frames, \
                      msbits {}, fifo {}",
                     hw.interval(alsa::PARAM_RATE).describe(), hw.interval(alsa::PARAM_CHANNELS).describe(),
                     hw.interval(alsa::PARAM_PERIOD_SIZE).describe(), hw.interval(alsa::PARAM_PERIODS).describe(),
                     hw.interval(alsa::PARAM_BUFFER_SIZE).describe(), hw.msbits(), hw.fifo_size()));
    if buffer as usize != PERIOD * periods as usize {
        return Err(format!("unexpected buffer size {buffer}; stopping before playing"));
    }
    let mut sw = alsa::SwParams::new(PERIOD as u32, buffer, buffer);
    pcm.sw_params(&mut sw).map_err(|e| format!("sw params: {e}"))?;
    match sys::lock_memory() {
        Ok(()) => out.say("memory locked (mlockall)"),
        Err(e) => out.say(&format!("mlockall failed: {e} (continuing)")),
    }
    match sys::realtime(70) {
        Ok(()) => out.say("SCHED_FIFO priority 70"),
        Err(e) => out.say(&format!("SCHED_FIFO failed: {e} (continuing at normal priority)")),
    }
    Ok(pcm)
}

/// A running ACVA stream: writes one period at a time, recovers from
/// underruns, and keeps the timing of every period.
struct Playback {
    pcm: alsa::Pcm,
    /// Periods in the buffer.
    queued: u32,
    xruns: u64,
    late: u64,
    errors: u64,
    gaps: Vec<Duration>,
    notes: Vec<String>,
    started: Instant,
    prev: Instant,
    periods: u64,
}

impl Playback {
    fn start(pcm: alsa::Pcm, queued: u32) -> Result<Playback, String> {
        pcm.prepare().map_err(|e| format!("prepare: {e}"))?;
        // Queue the whole buffer: the stream starts when it is full.
        for _ in 0..queued {
            pcm.write_frames(&[0i32; PERIOD * 2]).map_err(|e| format!("first write: {e}"))?;
        }
        Ok(Playback { pcm, queued, xruns: 0, late: 0, errors: 0, gaps: Vec::with_capacity(1 << 16), notes: Vec::new(),
                      started: Instant::now(), prev: Instant::now(), periods: 0 })
    }

    fn write(&mut self, frames: &[i32; PERIOD * 2], i: u64) -> Result<(), String> {
        match self.pcm.write_frames(frames) {
            Ok(_) => {}
            Err(e) if e.raw_os_error() == Some(EPIPE) => {
                self.xruns += 1;
                if self.notes.len() < 20 {
                    self.notes.push(format!("underrun at period {i} ({:.3} s)", self.started.elapsed().as_secs_f64()));
                }
                self.pcm.prepare().map_err(|e| format!("prepare after underrun: {e}"))?;
                for _ in 0..self.queued {
                    self.pcm.write_frames(&[0i32; PERIOD * 2]).map_err(|e| format!("refill after underrun: {e}"))?;
                }
            }
            Err(e) => {
                self.errors += 1;
                if self.errors > 3 {
                    return Err(format!("write failed: {e}"));
                }
                self.notes.push(format!("write error at period {i}: {e}"));
            }
        }
        let now = Instant::now();
        let gap = now - self.prev;
        self.prev = now;
        if i >= self.queued as u64 {
            self.gaps.push(gap);
            if gap > period_time() * 2 {
                self.late += 1;
                if self.notes.len() < 20 {
                    self.notes.push(format!("late period {i}: {:.2} ms after the previous one", gap.as_secs_f64() * 1e3));
                }
            }
        }
        self.periods += 1;
        Ok(())
    }

    /// Drain, show the notes, and return the timing summary.
    fn finish(self, out: &mut Out) -> Result<String, String> {
        if let Err(e) = self.pcm.drain() {
            out.say(&format!("drain: {e}"));
            let _ = self.pcm.drop_stream();
        }
        for n in &self.notes {
            out.say(n);
        }
        let (avg, max) = gap_stats(&self.gaps);
        Ok(format!("{} periods ({:.1} s), {} underruns, {} late periods (over 2 periods apart), {} other errors; \
                    period gap mean {:.3} ms, max {:.3} ms (one period is {:.3} ms)",
                   self.periods, self.started.elapsed().as_secs_f64(), self.xruns, self.late, self.errors,
                   avg * 1e3, max * 1e3, period_time().as_secs_f64() * 1e3))
    }
}

fn period_time() -> Duration {
    Duration::from_secs_f64(PERIOD as f64 / RATE as f64)
}

fn click(o: &Options) -> Result<(), String> {
    let mut out = Out::new(o, "click", true);
    let pcm = open_acva(&mut out, PERIODS)?;
    let mut engine = ClickEngine::new();
    if !engine.configure(ClickConfig { sample_rate: RATE, bpm: 120.0, beats_per_bar: 4 }) {
        return Err("the click engine refused its settings".to_string());
    }
    let gain = 10f32.powf(o.gain_db / 20.0);
    out.say(&format!("playing a 120 BPM click for {} s at {:.0} dB (peak about {:.0} dBFS); \
                      keep the volume knob low", o.seconds, o.gain_db, 20.0 * (0.47 * gain).log10()));
    out.draw();

    let (mut left, mut right) = ([0f32; PERIOD], [0f32; PERIOD]);
    let mut frames = [0i32; PERIOD * 2];
    let total_periods = o.seconds * RATE as u64 / PERIOD as u64;
    let mut play = Playback::start(pcm, PERIODS)?;
    for i in 0..total_periods {
        engine.render(&mut left, &mut right).map_err(|e| format!("click engine: {e:?}"))?;
        for f in 0..PERIOD {
            frames[2 * f] = float_to_s32(left[f] * gain);
            frames[2 * f + 1] = float_to_s32(right[f] * gain);
        }
        play.write(&frames, i)?;
    }
    let summary = play.finish(&mut out)?;
    out.say(&format!("click done: {summary}"));
    Ok(())
}

fn gap_stats(gaps: &[Duration]) -> (f64, f64) {
    if gaps.is_empty() {
        return (0.0, 0.0);
    }
    let sum: f64 = gaps.iter().map(|g| g.as_secs_f64()).sum();
    let max = gaps.iter().map(|g| g.as_secs_f64()).fold(0.0, f64::max);
    (sum / gaps.len() as f64, max)
}

// ---------------------------------------------------------------- preview (host)

fn write_png(path: &Path, canvas: &SoftCanvas) -> Result<(), String> {
    let png = mpfree_ui::png::encode_rgb(canvas.width() as u32, canvas.height() as u32, &canvas.to_rgb());
    // New files only: never overwrite.
    OpenOptions::new().write(true).create_new(true).open(path).and_then(|mut f| f.write_all(&png))
        .map_err(|e| format!("{}: {e}", path.display()))?;
    println!("preview written: {}", path.display());
    Ok(())
}

/// Sample screens as PNG files in a new or empty folder: the one-shot steps'
/// console, and every page of the pad player. No devices are opened.
fn preview(o: &Options) -> Result<(), String> {
    let dir = o.preview.as_ref().unwrap();
    fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let fonts = || Fonts::with_files(o.font.as_deref(), o.bold_font.as_deref(), o.mono_font.as_deref());
    let mut c = gui::Console::new(fonts()?, "touch", None);
    for l in ["===== mpfree-hello touch =====", "touchscreen: /dev/input/event0 (ili2117); read-only, not grabbed",
              "touch the screen now, with one finger and then two (10 s)", "touch: 1 contact(s): slot 0: x 1024 y 512",
              "touch: 2 contact(s): slot 0: x 1020 y 515, slot 1: x 300 y 1700", "touch: no contact",
              "touch done: 412 events, 96 frames, 3 lines shown"] {
        c.push(l);
    }
    c.set_status("up 42 s");
    c.draw()?;
    write_png(&dir.join("console.png"), c.canvas())?;
    let font_error = fonts().err();
    if let Some(e) = font_error {
        return Err(e);
    }
    for (name, mut p) in gui::sample_screens(|| fonts().unwrap_or_else(|_| Fonts::embedded())) {
        let (w, h) = p.ui.size();
        let mut canvas = SoftCanvas::new(w, h);
        p.ui.paint(&mut canvas);
        write_png(&dir.join(format!("{name}.png")), &canvas)?;
    }
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = parse_args(&args).and_then(|o| {
        if o.preview.is_some() {
            return preview(&o);
        }
        match o.command.as_str() {
            "inventory" => inventory(&o),
            "screen" => screen(&o),
            "touch" => touch(&o),
            "pads" => pads(&o),
            "lights" => lights(&o),
            "setup" => setup(&o),
            "drums" => drums(&o),
            "play" => player(&o, true),
            "app" => player(&o, false),
            "click" => click(&o),
            _ => unreachable!(),
        }
    });
    if let Err(e) = result {
        println!("STOP: {e}");
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(Path::new(REPORT_DIR).join("hello.txt")) {
            let _ = writeln!(f, "STOP: {e}");
        }
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn arguments() {
        let o = parse_args(&args("pads --seconds 8 --keepalive")).unwrap();
        assert_eq!((o.command.as_str(), o.seconds, o.keepalive), ("pads", 8, true));
        assert!(parse_args(&args("click --keepalive")).is_err());
        assert!(parse_args(&args("click --gain-db -3")).is_err());
        assert!(parse_args(&args("click --seconds 61")).is_err());
        assert!(parse_args(&args("click --seconds 0")).is_err());
        assert!(parse_args(&args("format")).is_err());
        assert!(parse_args(&args("")).is_err());
        assert!(parse_args(&args("pads click")).is_err());
        let o = parse_args(&args("app --patterns /tmp/p.txt")).unwrap();
        assert_eq!(o.patterns, Some(PathBuf::from("/tmp/p.txt")));
        assert!(parse_args(&args("app --save-patterns")).is_err(), "the store saves patterns now");
        assert!(parse_args(&args("play --seconds 5 --patterns /tmp/p.txt")).is_err());
    }

    #[test]
    fn saved_patterns_go_through_the_store_and_a_patterns_file_is_only_read() {
        let dir = std::env::temp_dir().join(format!("mpfree-patterns-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        // What Save hands the store comes back at the next start.
        let store = mpfree_store::Store::at(&dir, |_| {});
        assert!(store.saves());
        assert_eq!(store.read(app::saved::ARP, app::saved::ARP_PATTERNS), None, "nothing saved yet");
        store.write(app::saved::ARP, app::saved::ARP_PATTERNS, b"Skip = 1 . 3\n".to_vec()).unwrap();
        store.flush();
        let again = mpfree_store::Store::at(&dir, |_| {});
        assert_eq!(again.read(app::saved::ARP, app::saved::ARP_PATTERNS).as_deref(), Some(&b"Skip = 1 . 3\n"[..]));
        assert!(dir.join("files/arp/patterns.txt").is_file());
        // Without a data folder nothing is kept.
        assert!(!mpfree_store::Store::none().saves());
        // --patterns FILE is read, never written.
        let file = dir.join("mine.txt");
        fs::write(&file, "Skip = 1 . 3\n").unwrap();
        assert_eq!(read_patterns(&file).unwrap(), "Skip = 1 . 3\n");
        fs::write(&file, vec![b'#'; PATTERNS_MAX_BYTES as usize + 1]).unwrap();
        assert!(read_patterns(&file).unwrap_err().contains("larger than"));
        assert!(read_patterns(&dir.join("none.txt")).is_err());
        fs::remove_dir_all(&dir).unwrap();
        for step in ["lights", "setup", "drums"] {
            let o = parse_args(&args(&format!("{step} --seconds 20"))).unwrap();
            assert_eq!((o.command.as_str(), o.seconds), (step, 20));
            assert!(parse_args(&args(&format!("{step} --keepalive"))).is_err());
        }
    }

    #[test]
    fn cards_from_proc() {
        let text = " 0 [MIDI           ]: USB-Audio - MPC One MIDI\n                      Akai MPC One MIDI at usb-ff540000.usb-1, full speed\n 1 [ACVA           ]: jp07-audio - ACVA\n                      ACVA\n";
        let c = parse_cards(text);
        assert_eq!(c, vec![(0, "MIDI".to_string(), "USB-Audio - MPC One MIDI".to_string()),
                           (1, "ACVA".to_string(), "jp07-audio - ACVA".to_string())]);
    }

    #[test]
    fn touches_follow_slots() {
        let mut t = Touches::new();
        let feed = |t: &mut Touches, evs: &[(u16, u16, i32)]| evs.iter().map(|&(a, b, c)| t.feed(a, b, c)).last().unwrap();
        assert!(feed(&mut t, &[(3, 0x2f, 0), (3, 0x39, 7), (3, 0x35, 100), (3, 0x36, 200), (0, 0, 0)]));
        assert_eq!(t.describe(), "1 contact(s): slot 0: x 100 y 200");
        assert!(feed(&mut t, &[(3, 0x2f, 1), (3, 0x39, 8), (3, 0x35, 300), (3, 0x36, 400), (0, 0, 0)]));
        assert_eq!(t.describe(), "2 contact(s): slot 0: x 100 y 200, slot 1: x 300 y 400");
        assert!(feed(&mut t, &[(3, 0x2f, 0), (3, 0x39, -1), (0, 0, 0)]));
        assert_eq!(t.describe(), "1 contact(s): slot 1: x 300 y 400");
        assert!(!t.feed(1, 0x14a, 1));
        t.feed(3, 0x2f, 99); // out of range slot is clamped, not a panic
    }

    #[test]
    fn event_layout() {
        let mut b = vec![0u8; EVENT_SIZE];
        let base = EVENT_SIZE - 8;
        b[base..base + 2].copy_from_slice(&3u16.to_ne_bytes());
        b[base + 2..base + 4].copy_from_slice(&0x35u16.to_ne_bytes());
        b[base + 4..base + 8].copy_from_slice(&(-5i32).to_ne_bytes());
        assert_eq!(parse_event(&b), (3, 0x35, -5));
    }

    #[test]
    fn describes_events() {
        assert_eq!(describe_event(&ControlEvent::PadDown { pad: 0, velocity: 90 }), "pad 1 down, velocity 90");
        assert_eq!(describe_event(&ControlEvent::QLinkTurn { knob: 2, delta: -1 }), "Q-Link 3 turned -1");
    }

    fn lines_for(port: &str, input: &[u8]) -> Vec<String> {
        let mut parser = Parser::default();
        let mut lines = Vec::new();
        for &b in input {
            if let Some(msg) = parser.feed(b) {
                let event = mpc_one::decode(&msg);
                let (bytes, truncated) = message_bytes(&msg);
                lines.push(message_line(1.5, port, &bytes, truncated, event.as_ref()));
            }
        }
        lines
    }

    #[test]
    fn shows_every_message_in_hex() {
        // The reply round 1 understood, and the same reply with another last byte, which it dropped.
        let known = [0xF0, 0x47, 0x7F, 0x46, 0x53, 0x00, 0x05, 0x7D, 0x6D, 0x57, 0x35, 0x01, 0xF7];
        let mut other = known;
        other[11] = 0x00;
        let mut input = known.to_vec();
        input.extend_from_slice(&other);
        input.extend_from_slice(&[0x99, 0x25, 0x40, 0x25, 0x00, 0xB0, 0x64, 0x7F, 0xC3, 0x05]);
        assert_eq!(lines_for("Private", &input), vec![
            "+1.500 s Private: F0 47 7F 46 53 00 05 7D 6D 57 35 01 F7 = session reply, token 7D 6D 57 35, running",
            "+1.500 s Private: F0 47 7F 46 53 00 05 7D 6D 57 35 00 F7 = session reply, token 7D 6D 57 35, held by another token",
            "+1.500 s Private: 99 25 40 = pad 1 down, velocity 64",
            "+1.500 s Private: 99 25 00 = pad 1 up",
            "+1.500 s Private: B0 64 7F = data wheel -1",
            "+1.500 s Private: C3 05 = not recognised",
        ]);
    }

    #[test]
    fn hides_the_serial_number() {
        let mut reply = vec![0xF0, 0x7E, 0x00, 0x06, 0x02, 0x47, 0x46, 0x00, 0x19, 0x00, 0x00, 0x01, 0x00, 0x7F, 0x00, 0x00,
                             0x00, 0x00];
        reply.extend_from_slice(b"XX000000000"); // a made-up serial
        reply.extend_from_slice(&[0x00, 0xF7]);
        let lines = lines_for("Public", &reply);
        assert_eq!(lines, vec!["+1.500 s Public: F0 7E 00 06 02 47 46 00 19 00 00 01 00 7F 00 00 00 00 ... = identity reply \
                                (the rest is the serial number, not shown)"]);
        assert!(!lines[0].contains("58 58"));
    }

    #[test]
    fn public_qlinks_are_absolute() {
        // Round 2: Q-Link 1 on Public counted up 01, 02, ... and back down.
        let input = [0x90, 0x54, 0x7F, 0xB0, 0x10, 0x35, 0xB0, 0x10, 0x34];
        assert_eq!(lines_for("Public", &input), vec![
            "+1.500 s Public: 90 54 7F = Q-Link 1 touched",
            "+1.500 s Public: B0 10 35 = Q-Link 1 at 53 (absolute)",
            "+1.500 s Public: B0 10 34 = Q-Link 1 at 52 (absolute)",
        ]);
        // On Private (with Akai's setup) they are steps.
        assert_eq!(lines_for("Private", &[0xB0, 0x10, 0x02]), vec!["+1.500 s Private: B0 10 02 = Q-Link 1 turned +2"]);
    }

    #[test]
    fn marks_long_sysex_as_cut_short() {
        let mut input = vec![0xF0];
        input.extend(std::iter::repeat_n(0x11, 100));
        input.push(0xF7);
        let lines = lines_for("Public", &input);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("(cut short: longer than the parser keeps) = not recognised"), "{}", lines[0]);
    }

    #[test]
    fn port_names_match_the_controller() {
        assert_eq!(MIDI_PORTS.map(|p| p.0), [0, 1, 2]);
        assert_eq!(MIDI_PORTS[mpc_one::Port::Public as usize].1, "Public");
        assert_eq!(MIDI_PORTS[mpc_one::Port::Private as usize].1, "Private");
    }

    #[test]
    fn gap_statistics() {
        let (avg, max) = gap_stats(&[Duration::from_millis(2), Duration::from_millis(4)]);
        assert!((avg - 0.003).abs() < 1e-9 && (max - 0.004).abs() < 1e-9);
        assert_eq!(gap_stats(&[]), (0.0, 0.0));
    }
}
