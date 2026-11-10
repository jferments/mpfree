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
//!                      on the Public port every 5 s (the only thing it ever
//!                      sends to the controller)
//!   click --seconds N [--gain-db G]
//!                      plays a 120 BPM click on the ACVA card (stereo S32_LE,
//!                      44.1 kHz, 128-frame periods, 3 periods, as the stock app
//!                      does) and logs every underrun and late period
//!   --preview FILE.ppm host only: draws a sample card to a file, no devices
//!
//! It writes only to: /dev/fb0 and the framebuffer's sysfs `pan` file (the
//! screen), the ACVA PCM device (the click), the controller's Public MIDI port
//! (keep-alive, only with --keepalive) and /run/mpfree-hello/hello.txt (its own
//! report, in RAM; the stick's log service copies it to the stick). It never
//! touches storage, the eMMC is hidden from this kernel anyway, and nothing it
//! does survives a power-cycle.

#![deny(unsafe_code)] // the few C calls live in sys.rs, the only module allowed unsafe code

mod akai;
mod alsa;
mod kit;
mod sys;

use mpfree_controller::midi::{Message, Parser};
use mpfree_controller::mpc_one::{self, ControlEvent, OBSERVED_KEEPALIVE_TOKEN};
use mpfree_core::{float_to_s32, ClickConfig, ClickEngine};
use mpfree_screen_log::{grid, render, Geometry, LogView, Rotation};
use std::fs::{self, File, OpenOptions};
use std::io::{self, ErrorKind, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

const TITLE: &str = "MPFree: Hello MPC";
const REPORT_DIR: &str = "/run/mpfree-hello";
const PANEL_ROTATION: &str = "/proc/device-tree/mipi@ff960000/panel@0/rotation";
const MAX_SECONDS: u64 = 60;
const AUDIO_CARD_ID: &str = "ACVA";
const RATE: u32 = 44_100;
const PERIOD: usize = 128;
const PERIODS: u32 = 3;
const EPIPE: i32 = 32;
const EAGAIN: i32 = 11;
/// The drum kit the host tool copies next to the program (see kit.rs).
const KIT_PATH: &str = "/run/mpfree-hello/kit.bin";
const MAX_KIT_BYTES: u64 = 48 << 20;

struct Options {
    command: String,
    seconds: u64,
    keepalive: bool,
    gain_db: f32,
    fb: PathBuf,
    sysfs: PathBuf,
    preview: Option<PathBuf>,
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
    };
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let mut value = || it.next().cloned().ok_or_else(|| format!("{arg} needs a value"));
        match arg.as_str() {
            "inventory" | "screen" | "touch" | "pads" | "lights" | "setup" | "drums" | "click" if o.command.is_empty() => {
                o.command = arg.clone()
            }
            "--seconds" => {
                o.seconds = value()?.parse().map_err(|_| "--seconds needs a whole number".to_string())?;
                if !(1..=MAX_SECONDS).contains(&o.seconds) {
                    return Err(format!("--seconds must be 1 to {MAX_SECONDS}"));
                }
            }
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
            _ => return Err(format!("unknown argument {arg}")),
        }
    }
    if o.preview.is_none() && o.command.is_empty() {
        return Err("usage: mpfree-hello inventory|screen|touch|pads|lights|setup|drums|click [options]".to_string());
    }
    if o.keepalive && o.command != "pads" {
        return Err("--keepalive goes with pads".to_string());
    }
    Ok(o)
}

/// The card on the screen, plus the same lines on stdout and in the report file.
struct Out {
    screen: Option<Screen>,
    view: LogView,
    heading: String,
    report: Option<File>,
    last_draw: Instant,
}

struct Screen {
    fb: File,
    geometry: Geometry,
    rotation: Rotation,
    buf: Vec<u8>,
}

impl Out {
    fn new(o: &Options, heading: &str, use_screen: bool) -> Out {
        let report = fs::create_dir_all(REPORT_DIR).ok().and_then(|_| {
            OpenOptions::new().create(true).append(true).mode(0o644).open(Path::new(REPORT_DIR).join("hello.txt")).ok()
        });
        let mut out = Out { screen: None, view: LogView::new(60, 30), heading: heading.to_string(), report,
                            last_draw: Instant::now() - Duration::from_secs(1) };
        out.file_line(&format!("===== mpfree-hello {} (uptime {}) =====", heading, uptime()));
        if use_screen {
            match Screen::open(o) {
                Ok(s) => {
                    let (cols, rows) = grid(&s.geometry, s.rotation);
                    out.view = LogView::new(cols, rows.saturating_sub(2));
                    out.screen = Some(s);
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
        if self.last_draw.elapsed() >= Duration::from_millis(100) {
            self.draw();
        }
    }

    /// Like `say`, but never redraws the screen (a redraw takes longer than an
    /// audio period, so the drum loop only uses this).
    fn note(&mut self, line: &str) {
        println!("{line}");
        let _ = io::stdout().flush();
        self.file_line(line);
        self.view.push(line);
    }

    fn draw(&mut self) {
        let status = format!("{} | up {} s | switch the MPC off to return to the stock firmware", self.heading, uptime());
        if let Some(s) = self.screen.as_mut() {
            if let Err(e) = s.draw(TITLE, &status, &self.view) {
                eprintln!("screen: {e}");
                self.screen = None;
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

impl Screen {
    fn open(o: &Options) -> Result<Screen, String> {
        let geometry = geometry_from_sysfs(&o.sysfs)?;
        let rotation = panel_rotation()?;
        // Write only: no create, no truncate.
        let fb = OpenOptions::new().write(true).open(&o.fb).map_err(|e| format!("{}: {e}", o.fb.display()))?;
        // Ask the kernel to show the framebuffer (a mode set; nothing is stored).
        let _ = OpenOptions::new().write(true).open(o.sysfs.join("pan")).and_then(|mut f| f.write_all(b"0,0\n"));
        let buf = vec![0u8; geometry.frame_len()];
        Ok(Screen { fb, geometry, rotation, buf })
    }

    fn draw(&mut self, title: &str, status: &str, view: &LogView) -> Result<(), String> {
        render(&self.geometry, self.rotation, title, status, view, &mut self.buf)?;
        self.fb.seek(SeekFrom::Start(0)).and_then(|_| self.fb.write_all(&self.buf)).map_err(|e| format!("writing: {e}"))
    }
}

fn read_trimmed(path: &Path) -> Result<String, String> {
    fs::read_to_string(path).map(|s| s.trim().to_string()).map_err(|e| format!("{}: {e}", path.display()))
}

/// The same reading as mpfree-screen-log: size from `modes`, plus stride and depth.
fn geometry_from_sysfs(dir: &Path) -> Result<Geometry, String> {
    let modes = read_trimmed(&dir.join("modes"))?;
    let first = modes.lines().next().unwrap_or("");
    let size = first.split(':').nth(1).unwrap_or("");
    let digits: String = size.chars().take_while(|c| c.is_ascii_digit() || *c == 'x').collect();
    let (w, h) = digits.split_once('x').ok_or_else(|| format!("cannot read a size from modes {first:?}"))?;
    let num = |s: &str, what: &str| s.parse::<usize>().map_err(|_| format!("bad {what} {s:?}"));
    let stride = num(&read_trimmed(&dir.join("stride"))?, "stride")?;
    let bpp = read_trimmed(&dir.join("bits_per_pixel"))?.parse::<u32>().map_err(|e| format!("bits_per_pixel: {e}"))?;
    Geometry::new(num(w, "width")?, num(h, "height")?, stride, bpp)
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
    if out.screen.is_none() {
        return Err("no screen to draw on".to_string());
    }
    for line in [
        "Hello MPC.",
        "",
        "This screen is drawn by MPFree's own code, running from the USB stick.",
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

fn touch(o: &Options) -> Result<(), String> {
    let mut out = Out::new(o, "touch", true);
    let devices = input_devices();
    let hits: Vec<&(String, String)> = devices.iter().filter(|(_, n)| n.to_ascii_lowercase().contains("ili2117")).collect();
    let [(path, name)] = hits.as_slice() else {
        return Err(format!("expected one ILI2117 input device, found {}", hits.len()));
    };
    out.say(&format!("touchscreen: {path} ({name}); read-only, not grabbed"));
    let mut dev = OpenOptions::new().read(true).custom_flags(sys::O_NONBLOCK).open(path)
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
    /// Print without redrawing the screen (the drum loop).
    quiet: bool,
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
        Ok(Ports { inputs, public, private, start: Instant::now(), sent: 0, shown: 0, quiet: false })
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
            self.line(out, &format!("    {text}"));
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
                if !matches!(event, Some(ControlEvent::PadPressure { .. })) {
                    self.shown += 1;
                    if self.shown <= MAX_SHOWN {
                        let text = message_line(t, name, &bytes, truncated, event.as_ref());
                        self.line(out, &text);
                    } else if self.shown == MAX_SHOWN + 1 {
                        self.line(out, &format!("(more than {MAX_SHOWN} messages: only counting from here)"));
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

/// Akai's startup, replayed: identity request and session ping until the
/// controller accepts the token, then the setup messages, lights, and a listening
/// window with pings every 4 s. Stops before any setup message if the session
/// is not accepted within 20 s.
fn setup(o: &Options) -> Result<(), String> {
    let mut out = Out::new(o, "Akai's setup (session)", true);
    let mut ports = Ports::open(&mut out, true)?;
    let mut buf = [0u8; 256];
    out.say(&format!("asking for a session with token {}: identity request, then a ping, every {} s, for up to 20 s",
                     hex(&akai::SESSION_TOKEN), akai::PING_SECONDS));
    let give_up = Instant::now() + Duration::from_secs(20);
    let mut next_ping = Instant::now();
    let mut accepted = None;
    while accepted.is_none() && Instant::now() < give_up {
        if Instant::now() >= next_ping {
            ports.send_batch(&mut out, false, "identity request and session ping",
                             &[&akai::IDENTITY_REQUEST[..], &akai::session_ping(akai::SESSION_TOKEN)[..]])?;
            next_ping = Instant::now() + Duration::from_secs(akai::PING_SECONDS);
        }
        for m in ports.poll(&mut out, &mut buf)? {
            if let Some((token, status)) = akai::session_reply(&m.bytes) {
                if token == akai::SESSION_TOKEN && (status == akai::SESSION_ACCEPTED || status == akai::SESSION_RUNNING) {
                    accepted = Some(status);
                }
            }
        }
        thread::sleep(Duration::from_millis(2));
    }
    if accepted.is_none() {
        return Err("the controller did not accept the session within 20 s; no setup message was sent".to_string());
    }
    out.say("session accepted; sending Akai's setup exactly as the app did");
    ports.send_batch(&mut out, false, "setup, part 1 (as at +25.772 s)", &akai::SETUP_FIRST)?;
    let wait = |out: &mut Out, ports: &mut Ports, ms: u64| -> Result<(), String> {
        let until = Instant::now() + Duration::from_millis(ms);
        let mut b = [0u8; 256];
        while Instant::now() < until {
            ports.poll(out, &mut b)?;
            thread::sleep(Duration::from_millis(1));
        }
        Ok(())
    };
    wait(&mut out, &mut ports, 12)?;
    ports.send_batch(&mut out, false, "setup, part 2 (sent twice, as at +25.784 s)", &[&akai::SETUP_5A[..], &akai::SETUP_5A[..]])?;
    let gradient = pad_colours(&akai::STARTUP_GRADIENT);
    ports.send_batch(&mut out, false, "Akai's startup pad colours", &as_slices(&gradient))?;
    let lights: Vec<[u8; 3]> = akai::STARTUP_BUTTON_LIGHTS.iter().map(|&[n, v]| akai::button_light(n, v)).collect();
    ports.send_batch(&mut out, true, "Akai's startup button lights", &as_slices(&lights))?;
    wait(&mut out, &mut ports, 340)?;
    ports.send_batch(&mut out, false, "setup, part 3 (as at +26.170 s)", &akai::SETUP_LAST)?;
    wait(&mut out, &mut ports, 960)?;
    let rest = pad_colours(&akai::RESTING_COLOURS);
    ports.send_batch(&mut out, false, "your project's pad colours", &as_slices(&rest))?;
    ports.send(false, &akai::session_ping(akai::SESSION_TOKEN))?;
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

/// Play the drum kit in KIT_PATH from the pads, through ACVA.
fn drums(o: &Options) -> Result<(), String> {
    let mut out = Out::new(o, "factory drums", true);
    let size = fs::metadata(KIT_PATH).map_err(|e| format!("{KIT_PATH}: {e} (the host tool copies it there)"))?.len();
    if size > MAX_KIT_BYTES {
        return Err(format!("{KIT_PATH} is {size} bytes; at most {MAX_KIT_BYTES} are allowed"));
    }
    let sounds = kit::parse(&fs::read(KIT_PATH).map_err(|e| format!("{KIT_PATH}: {e}"))?)?;
    let mut rest = [[0u8; 3]; 16];
    out.say(&format!("kit: {} sounds", sounds.len()));
    for s in &sounds {
        match mpc_one::pad_index(s.note) {
            Some(pad) => {
                rest[pad as usize] = akai::RESTING_COLOURS[pad as usize];
                out.say(&format!("  pad {:2} (note {:3}): {} ({:.2} s)", pad + 1, s.note, s.name,
                                 s.frames() as f64 / RATE as f64));
            }
            None => out.say(&format!("  note {}: {} (no pad sends this note)", s.note, s.name)),
        }
    }
    let mut mixer = kit::Mixer::new(sounds);
    let mut ports = Ports::open(&mut out, false)?;
    let colours = pad_colours(&rest);
    ports.send_batch(&mut out, false, "pad colours: lit where the kit has a sound, dark elsewhere", &as_slices(&colours))?;
    let pcm = open_acva(&mut out)?;
    let master = 10f32.powf(o.gain_db / 20.0);
    out.say(&format!("play the pads for {} s; each sound peaks at most {:.0} dB below full scale at full velocity",
                     o.seconds, -o.gain_db));
    out.draw();
    ports.quiet = true;
    let mut play = Playback::start(pcm)?;
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
                        let name = mixer.sounds[k].name.clone();
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

/// Open ACVA at the stock stream settings (S32_LE, 44.1 kHz, 128 x 3 periods),
/// lock memory and ask for SCHED_FIFO. Stops before playing if anything differs.
fn open_acva(out: &mut Out) -> Result<alsa::Pcm, String> {
    let card = audio_card()?;
    let path = format!("/dev/snd/pcmC{card}D0p");
    out.say(&format!("opening {path} ({AUDIO_CARD_ID})"));
    let pcm = alsa::Pcm::open_playback(&path).map_err(|e| format!("{path}: {e}"))?;
    let mut hw = alsa::HwParams::stock_playback(PERIODS);
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
    if buffer as usize != PERIOD * PERIODS as usize {
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
    fn start(pcm: alsa::Pcm) -> Result<Playback, String> {
        pcm.prepare().map_err(|e| format!("prepare: {e}"))?;
        // Queue the whole buffer: the stream starts when it is full.
        for _ in 0..PERIODS {
            pcm.write_frames(&[0i32; PERIOD * 2]).map_err(|e| format!("first write: {e}"))?;
        }
        Ok(Playback { pcm, xruns: 0, late: 0, errors: 0, gaps: Vec::with_capacity(1 << 16), notes: Vec::new(),
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
                for _ in 0..PERIODS {
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
        if i >= PERIODS as u64 {
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
    let pcm = open_acva(&mut out)?;
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
    let mut play = Playback::start(pcm)?;
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

fn preview(o: &Options) -> Result<(), String> {
    let g = Geometry::new(800, 1280, 3200, 32)?;
    let (cols, rows) = grid(&g, Rotation::Deg270);
    let mut view = LogView::new(cols, rows - 2);
    for l in ["Hello MPC.", "", "pad 1 down, velocity 97", "touch: 1 contact(s): slot 0: x 1024 y 512",
              "click done: 3445 periods (10.0 s), 0 underruns, 0 late periods"] {
        view.push(l);
    }
    let mut buf = vec![0u8; g.frame_len()];
    render(&g, Rotation::Deg270, TITLE, "preview | up 42 s", &view, &mut buf)?;
    let mut ppm = format!("P6\n{} {}\n255\n", g.width, g.height).into_bytes();
    for y in 0..g.height {
        for x in 0..g.width {
            let p = &buf[y * g.stride + x * 4..y * g.stride + x * 4 + 3];
            ppm.extend_from_slice(&[p[2], p[1], p[0]]);
        }
    }
    let path = o.preview.as_ref().unwrap();
    OpenOptions::new().write(true).create_new(true).open(path).and_then(|mut f| f.write_all(&ppm))
        .map_err(|e| format!("{}: {e}", path.display()))?;
    println!("preview written: {}", path.display());
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
