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
//!                      reads the controller's Private MIDI port and shows pad,
//!                      button, Q-Link and wheel events; with --keepalive it also
//!                      sends Akai's keep-alive message on the Public port every
//!                      5 s (the only thing it ever sends to the controller)
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

mod alsa;
mod sys;

use mpfree_controller::midi::Parser;
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
            "inventory" | "screen" | "touch" | "pads" | "click" if o.command.is_empty() => o.command = arg.clone(),
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
        return Err("usage: mpfree-hello inventory|screen|touch|pads|click [options]".to_string());
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
        println!("{line}");
        let _ = io::stdout().flush();
        self.file_line(line);
        self.view.push(line);
        if self.last_draw.elapsed() >= Duration::from_millis(100) {
            self.draw();
        }
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

fn pads(o: &Options) -> Result<(), String> {
    let heading = if o.keepalive { "pads, with keep-alive" } else { "pads (listen only)" };
    let mut out = Out::new(o, heading, true);
    let card = midi_card()?;
    let (mut input, info) = alsa::open_rawmidi(card, mpc_one::Port::Private as u32, alsa::Direction::Input)
        .map_err(|e| format!("opening the Private MIDI port: {e}"))?;
    out.say(&format!("listening on card {card} subdevice {} \"{}\"", info.subdevice, info.subname));
    if info.subdevice != 1 || info.stream != alsa::RAWMIDI_STREAM_INPUT || !info.subname.contains("Private") {
        return Err("that is not the controller's Private port; stopping".to_string());
    }
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
    out.say(&format!("hit some pads, press a button, turn a Q-Link ({} s)", o.seconds));
    let mut parser = Parser::default();
    let mut buf = [0u8; 256];
    let (mut bytes, mut pad_hits, mut events, mut pings, mut replies) = (0u64, 0u64, 0u64, 0u64, 0u64);
    let mut last_ping: Option<Instant> = None;
    let end = Instant::now() + Duration::from_secs(o.seconds);
    while Instant::now() < end {
        if let Some(f) = output.as_mut() {
            if last_ping.is_none_or(|t| t.elapsed() >= Duration::from_secs(5)) {
                let (_, msg) = mpc_one::keepalive_ping(OBSERVED_KEEPALIVE_TOKEN);
                f.write_all(&msg).map_err(|e| format!("sending the keep-alive: {e}"))?;
                pings += 1;
                last_ping = Some(Instant::now());
                out.say(&format!("sent keep-alive {pings}"));
            }
        }
        match input.read(&mut buf) {
            Ok(0) => thread::sleep(Duration::from_millis(2)),
            Ok(n) => {
                bytes += n as u64;
                for &b in &buf[..n] {
                    if let Some(msg) = parser.feed(b) {
                        if let Some(ev) = mpc_one::decode(&msg) {
                            events += 1;
                            match ev {
                                ControlEvent::PadDown { .. } => pad_hits += 1,
                                ControlEvent::KeepAliveReply { .. } => replies += 1,
                                _ => {}
                            }
                            if !matches!(ev, ControlEvent::PadPressure { .. }) {
                                out.say(&describe_event(&ev));
                            }
                        }
                    }
                }
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock || e.raw_os_error() == Some(EAGAIN) => {
                thread::sleep(Duration::from_millis(2))
            }
            Err(e) => return Err(format!("reading MIDI: {e}")),
        }
    }
    out.say(&format!("pads done: {bytes} bytes, {events} events, {pad_hits} pad hits, \
                      {pings} keep-alives sent, {replies} replies"));
    Ok(())
}

// ---------------------------------------------------------------- click

fn click(o: &Options) -> Result<(), String> {
    let mut out = Out::new(o, "click", true);
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
    let mut engine = ClickEngine::new();
    if !engine.configure(ClickConfig { sample_rate: RATE, bpm: 120.0, beats_per_bar: 4 }) {
        return Err("the click engine refused its settings".to_string());
    }
    let gain = 10f32.powf(o.gain_db / 20.0);
    out.say(&format!("playing a 120 BPM click for {} s at {:.0} dB (peak about {:.0} dBFS); \
                      keep the volume knob low", o.seconds, o.gain_db, 20.0 * (0.47 * gain).log10()));
    out.draw();

    let period_time = Duration::from_secs_f64(PERIOD as f64 / RATE as f64);
    let (mut left, mut right) = ([0f32; PERIOD], [0f32; PERIOD]);
    let mut frames = [0i32; PERIOD * 2];
    let silence = [0i32; PERIOD * 2];
    let total_periods = o.seconds * RATE as u64 / PERIOD as u64;
    let (mut xruns, mut late, mut errors) = (0u64, 0u64, 0u64);
    let mut gaps = Vec::with_capacity(total_periods as usize);
    let mut notes: Vec<String> = Vec::new();
    pcm.prepare().map_err(|e| format!("prepare: {e}"))?;
    // Queue the whole buffer: the stream starts when it is full.
    for _ in 0..PERIODS {
        pcm.write_frames(&silence).map_err(|e| format!("first write: {e}"))?;
    }
    let started = Instant::now();
    let mut prev = Instant::now();
    for i in 0..total_periods {
        engine.render(&mut left, &mut right).map_err(|e| format!("click engine: {e:?}"))?;
        for f in 0..PERIOD {
            frames[2 * f] = float_to_s32(left[f] * gain);
            frames[2 * f + 1] = float_to_s32(right[f] * gain);
        }
        match pcm.write_frames(&frames) {
            Ok(_) => {}
            Err(e) if e.raw_os_error() == Some(EPIPE) => {
                xruns += 1;
                if notes.len() < 20 {
                    notes.push(format!("underrun at period {i} ({:.3} s)", started.elapsed().as_secs_f64()));
                }
                pcm.prepare().map_err(|e| format!("prepare after underrun: {e}"))?;
                for _ in 0..PERIODS {
                    pcm.write_frames(&silence).map_err(|e| format!("refill after underrun: {e}"))?;
                }
            }
            Err(e) => {
                errors += 1;
                if errors > 3 {
                    return Err(format!("write failed: {e}"));
                }
                notes.push(format!("write error at period {i}: {e}"));
            }
        }
        let now = Instant::now();
        let gap = now - prev;
        prev = now;
        if i >= PERIODS as u64 {
            gaps.push(gap);
            if gap > period_time * 2 {
                late += 1;
                if notes.len() < 20 {
                    notes.push(format!("late period {i}: {:.2} ms after the previous one", gap.as_secs_f64() * 1e3));
                }
            }
        }
    }
    if let Err(e) = pcm.drain() {
        out.say(&format!("drain: {e}"));
        let _ = pcm.drop_stream();
    }
    for n in &notes {
        out.say(n);
    }
    let (avg, max) = gap_stats(&gaps);
    out.say(&format!("click done: {total_periods} periods ({:.1} s), {xruns} underruns, {late} late periods \
                      (over 2 periods apart), {errors} other errors; period gap mean {:.3} ms, max {:.3} ms \
                      (one period is {:.3} ms)",
                     started.elapsed().as_secs_f64(), avg * 1e3, max * 1e3, period_time.as_secs_f64() * 1e3));
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

    #[test]
    fn gap_statistics() {
        let (avg, max) = gap_stats(&[Duration::from_millis(2), Duration::from_millis(4)]);
        assert!((avg - 0.003).abs() < 1e-9 && (max - 0.004).abs() < 1e-9);
        assert_eq!(gap_stats(&[]), (0.0, 0.0));
    }
}
