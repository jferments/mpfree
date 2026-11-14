//! mpfree, the app (Phase 1 called it "Hello MPC"), for the kexec test boot from the USB stick.
//!
//! One program, one step per run, paced from the workstation by
//! tools/mpc-one/host_app_mpc.py:
//!
//!   inventory          read-only: sound cards, PCM and MIDI devices, input
//!                      devices, framebuffer, panel rotation, RT limits
//!   screen             draws the mpfree card on /dev/fb0
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
//!   play --seconds N [--gain-db G] [--kits FILE] [--sound-cores MODE]
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
//!   app [--gain-db G] [--kits FILE] [--keep-screen] [--data DIR] [--sound-cores MODE]
//!                      the same pad player as a program that starts at power-on
//!                      (the test stick's mpfree-app service) and runs until the
//!                      power goes off: a loading page with a progress bar while
//!                      it waits for the controller and the sound card, then the
//!                      player; STOP silences, SHIFT + ERASE + TAP TEMPO shows the system log
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
//!   --display-lists DIR
//!                      host only: records each sample page's first two frames
//!                      as display lists, new NAME.mpdl files in DIR (for
//!                      mpfree-gpu-bench, which times them on the MPC's GPU)
//!
//! `--sound-cores shared|core3|two|three` (play and app) picks where the
//! sound runs for this run, in place of Prefs' saved "Sound cores" (shared
//! by default): as before, the audio thread alone on core 3, core 3 plus
//! lanes on core 2 for half the drum voices and part of the mixer's
//! inserts, or (a test) those plus the synth on a lane on core 1 (cores.rs).
//! It changes only this program's threads.
//!
//! The screen is MPFree's GUI (crate mpfree-ui, screens in gui.rs). Every line
//! also goes to stdout, which the workstation shows, and to the report file.
//! `--font`, `--bold-font` and `--mono-font FILE` swap the built-in fonts.
//! `--theme NAME|FILE` picks the look: default, console, synthwave,
//! spaceship, or a theme file (see mpfree-ui's theme.rs). It only reads the file.
//!
//! It writes only to: /dev/fb0 and the framebuffer's sysfs `pan` file (the
//! screen), the ACVA PCM device (sound), the controller's Public and Private
//! MIDI ports (only messages Akai's app was seen sending, button lights in the
//! same form with the values it uses, and pad colours made by its rules; never
//! in inventory, screen, touch, or pads without --keepalive) and
//! /run/mpfree-app/report.txt (its own report, in RAM; the workstation tool
//! copies it into its report). It reads the screen back once with
//! --keep-screen, and `app` runs `journalctl -f` (read-only) for its console
//! page. With --preview or --display-lists on the workstation it writes only
//! new PNG or .mpdl files. It
//! never touches the MPC's storage (the eMMC is hidden from this kernel
//! anyway). Only with `app --data DIR` does anything survive a power-cycle:
//! settings and found buttons, saved through mpfree-store under DIR alone.

#![deny(unsafe_code)] // the few C calls (and a test-only counting allocator) live in sys.rs, the only module allowed unsafe code

mod akai;
mod alsa;
mod app;
mod audiotracks;
mod calibrate_page;
mod chain_page;
mod control;
mod cores;
mod crash;
mod cv_out;
mod debug_page;
mod desk;
mod drawing;
mod engines;
mod export;
mod fxmodes;
mod gui;
mod instruments;
mod kit;
mod looper_page;
mod looper_tab;
mod looping;
mod memory;
mod midi_in;
mod midi_out;
mod midi_saved;
mod mixing;
mod modmix;
mod modpanels;
mod pace;
mod parts;
mod perf;
mod play_page;
mod play_widgets;
mod plugins;
mod port_writer;
mod preview;
mod project;
mod qlink_touch;
#[cfg(test)]
mod rt_safety;
mod sample_page;
mod sampler;
mod screen_cost;
#[cfg(test)]
mod smoke;
mod synth;
mod sys;
mod sysinfo;
mod time;
mod touch_bell;
mod touch_dev;
mod touchlab_page;
mod undo;

use mpfree_controller::midi::{Message, Parser};
use mpfree_controller::mpc_one::{self, ControlEvent, OBSERVED_KEEPALIVE_TOKEN};
use mpfree_core::{float_to_s32, ClickConfig, ClickEngine};
use mpfree_screen_log::Rotation;
use mpfree_synth_ui::{SynthAction, SynthState};
use mpfree_time_ui::view::AudioAction;
use mpfree_time_ui::{TimeAction, TransportInfo};
use mpfree_transport::Command;
use mpfree_ui::fb::FbTarget;
use mpfree_ui::{Fonts, SoftCanvas, Theme, Tone};
use std::fs::{self, File, OpenOptions};
use std::io::{self, ErrorKind, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::thread;
use std::time::{Duration, Instant};

use mpfree_prof::Span;

const REPORT_DIR: &str = "/run/mpfree-app";
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
const KIT_PATH: &str = "/run/mpfree-app/kits.bin";
const MAX_KIT_BYTES: u64 = 48 << 20;

struct Options {
    command: String,
    seconds: u64,
    keepalive: bool,
    gain_db: f32,
    fb: PathBuf,
    sysfs: PathBuf,
    preview: Option<PathBuf>,
    display_lists: Option<PathBuf>,
    font: Option<PathBuf>,
    bold_font: Option<PathBuf>,
    mono_font: Option<PathBuf>,
    theme: Option<String>,
    kits: PathBuf,
    keep_screen: bool,
    /// An arp pattern library file to read at start (read-only). It takes
    /// the place of the store's arp/patterns.txt for this start.
    patterns: Option<PathBuf>,
    /// Where settings and found buttons are kept (`app` only; mpfree-store).
    data: Option<PathBuf>,
    /// Where the sound runs for this run (`play` and `app`; cores.rs);
    /// else Prefs' saved "Sound cores".
    sound_cores: Option<cores::SoundCores>,
    /// Open the 5-pin MIDI Out jack, so tracks set to play on it do
    /// (`app` only; midi_out.rs). Off until a device round has checked it.
    midi_out: bool,
    /// Drive the CV/gate jacks (CV 1 to 4) from the project's CV outputs
    /// (`app` only; cv_out.rs). Off until a device round has checked it.
    cv_out: bool,
    /// Draw the screen on the GPU, in this program of its own
    /// (`mpfree-gpu-screen`; `play` and `app`; drawing.rs). Off until a
    /// device round has checked it; any failure draws in software again.
    gpu_screen: Option<PathBuf>,
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
        display_lists: None,
        font: None,
        bold_font: None,
        mono_font: None,
        theme: None,
        kits: PathBuf::from(KIT_PATH),
        keep_screen: false,
        patterns: None,
        data: None,
        sound_cores: None,
        midi_out: false,
        cv_out: false,
        gpu_screen: None,
    };
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let mut value = || {
            it.next()
                .cloned()
                .ok_or_else(|| format!("{arg} needs a value"))
        };
        match arg.as_str() {
            "inventory" | "screen" | "touch" | "pads" | "lights" | "setup" | "drums" | "play"
            | "app" | "click"
                if o.command.is_empty() =>
            {
                o.command = arg.clone()
            }
            "--seconds" => {
                o.seconds = value()?
                    .parse()
                    .map_err(|_| "--seconds needs a whole number".to_string())?
            }
            "--keepalive" => o.keepalive = true,
            "--gain-db" => {
                o.gain_db = value()?
                    .parse()
                    .map_err(|_| "--gain-db needs a number".to_string())?;
                if !(-40.0..=-6.0).contains(&o.gain_db) {
                    return Err("--gain-db must be between -40 and -6".to_string());
                }
            }
            "--fb" => o.fb = PathBuf::from(value()?),
            "--sysfs" => o.sysfs = PathBuf::from(value()?),
            "--preview" => o.preview = Some(PathBuf::from(value()?)),
            "--display-lists" => o.display_lists = Some(PathBuf::from(value()?)),
            "--font" => o.font = Some(PathBuf::from(value()?)),
            "--bold-font" => o.bold_font = Some(PathBuf::from(value()?)),
            "--mono-font" => o.mono_font = Some(PathBuf::from(value()?)),
            "--theme" => o.theme = Some(value()?),
            "--kits" => o.kits = PathBuf::from(value()?),
            "--keep-screen" => o.keep_screen = true,
            "--patterns" => o.patterns = Some(PathBuf::from(value()?)),
            "--midi-out" => o.midi_out = true,
            "--cv-out" => o.cv_out = true,
            "--data" => o.data = Some(PathBuf::from(value()?)),
            "--gpu-screen" => o.gpu_screen = Some(PathBuf::from(value()?)),
            "--sound-cores" => {
                o.sound_cores =
                    Some(cores::SoundCores::parse(&value()?).ok_or_else(|| {
                        format!("--sound-cores needs {}", cores::SoundCores::NAMES)
                    })?)
            }
            _ => return Err(format!("unknown argument {arg}")),
        }
    }
    if o.preview.is_none() && o.display_lists.is_none() && o.command.is_empty() {
        return Err(
            "usage: mpfree inventory|screen|touch|pads|lights|setup|drums|play|app|click [options]"
                .to_string(),
        );
    }
    let max = if o.command == "play" {
        PLAY_MAX_SECONDS
    } else {
        MAX_SECONDS
    };
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
    if o.midi_out && o.command != "app" {
        return Err("--midi-out goes with app".to_string());
    }
    if o.cv_out && o.command != "app" {
        return Err("--cv-out goes with app".to_string());
    }
    if o.data.is_some() && o.command != "app" {
        return Err("--data goes with app".to_string());
    }
    if args.iter().any(|a| a == "--sound-cores") && o.command != "app" && o.command != "play" {
        return Err("--sound-cores goes with app or play".to_string());
    }
    if o.gpu_screen.is_some() && o.command != "app" && o.command != "play" {
        return Err("--gpu-screen goes with app or play".to_string());
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
    /// While the audio loop runs, the report is written by the
    /// `mpfree-report` thread ([`Out::queue_report`]).
    queued: Option<control::Report<File>>,
    last_draw: Instant,
}

impl Out {
    fn new(o: &Options, heading: &str, use_screen: bool) -> Out {
        let report = fs::create_dir_all(REPORT_DIR).ok().and_then(|_| {
            OpenOptions::new()
                .create(true)
                .append(true)
                .mode(0o644)
                .open(Path::new(REPORT_DIR).join("report.txt"))
                .ok()
        });
        let mut out = Out {
            console: None,
            sink: None,
            dropped: 0,
            report,
            queued: None,
            last_draw: Instant::now() - Duration::from_secs(1),
        };
        out.file_line(&format!(
            "===== mpfree {} (uptime {}) =====",
            heading,
            uptime()
        ));
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
        if let Some(q) = self.queued.as_mut() {
            // A full queue drops the line; `unqueue_report` counts them.
            q.line(line.to_string());
        } else if let Some(f) = self.report.as_mut() {
            let _ = writeln!(f, "{line}");
        }
    }

    /// From now on the report's lines are written by the `mpfree-report`
    /// thread, not the caller (the audio loop): control.rs. Allocates and
    /// starts the thread; without a report file it does nothing.
    fn queue_report(&mut self) {
        // A session that ended early may have left its queue.
        self.unqueue_report();
        let Some(f) = self.report.take() else {
            return;
        };
        match f.try_clone().and_then(control::Report::start) {
            Ok(q) => {
                self.report = Some(f);
                self.queued = Some(q);
            }
            Err(e) => {
                self.report = Some(f);
                self.note(&format!(
                    "report: written by the audio thread ({e} starting its own)"
                ));
            }
        }
    }

    /// Report lines the queue had no room for so far.
    fn report_dropped(&self) -> u64 {
        self.queued.as_ref().map_or(0, |q| q.dropped())
    }

    /// Writes what is queued (up to a second) and goes back to writing the
    /// report here; says how many lines the queue had no room for.
    fn unqueue_report(&mut self) {
        let Some(q) = self.queued.take() else {
            return;
        };
        let dropped = q.dropped();
        let back = q.finish(Duration::from_secs(1)).is_some();
        if dropped > 0 || !back {
            self.note(&format!(
                "report: {dropped} lines dropped with the queue full{}",
                if back {
                    ""
                } else {
                    "; its thread was still writing at the end"
                }
            ));
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
            let sent = self
                .sink
                .as_ref()
                .is_some_and(|s| s.try_send(gui::ToUi::Midi(line.to_string())).is_ok());
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
                std::sync::mpsc::TrySendError::Full(m)
                | std::sync::mpsc::TrySendError::Disconnected(m) => m,
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
        self.unqueue_report();
        self.draw();
    }
}

/// The framebuffer, turned as the panel is mounted.
fn open_screen(o: &Options) -> Result<FbTarget, String> {
    let rotation = panel_rotation()?;
    FbTarget::open(&o.fb, &o.sysfs, rotation)
}

/// The theme from --theme: a built-in name or a theme file (read only).
fn load_theme(o: &Options) -> Result<Theme, String> {
    let Some(name) = &o.theme else {
        return Ok(Theme::dark());
    };
    if let Some(t) = Theme::builtin(name) {
        return Ok(t);
    }
    let text = fs::read_to_string(name).map_err(|e| {
        format!(
            "theme {name}: {e} (built in: {})",
            mpfree_ui::theme::BUILTIN.join(", ")
        )
    })?;
    Theme::parse(&text).map_err(|e| format!("{name}: {e}"))
}

/// The tunings folder's `NAME.scl` scales that parse, in name order (at
/// most [`synth::MAX_TUNING_FILES`]), each with the keyboard map `NAME.kbm`
/// when there is one beside it. A scale that doesn't parse, or whose map
/// doesn't, is left out with a line saying why (the second list, lines for
/// the log), and so is a map with no scale. Text that isn't UTF-8 (many
/// published files are Latin-1) is read with its odd characters replaced:
/// only a description has them. Only reads.
fn load_tuning_files(store: &mpfree_store::Store) -> (Vec<synth::TuningFile>, Vec<String>) {
    let (mut found, mut notes) = (Vec::new(), Vec::new());
    let names = store.list(app::TUNINGS_FOLDER);
    let read = |file: &str| {
        store
            .read(app::TUNINGS_FOLDER, file)
            .map(|b| String::from_utf8_lossy(&b).into_owned())
    };
    for file in &names {
        if let Some(stem) = file.strip_suffix(".kbm") {
            if !names.iter().any(|n| n.strip_suffix(".scl") == Some(stem)) {
                notes.push(format!("tunings: {file} left out: no {stem}.scl beside it"));
            }
            continue;
        }
        let Some(stem) = file.strip_suffix(".scl").filter(|s| !s.is_empty()) else {
            continue;
        };
        if found.len() == synth::MAX_TUNING_FILES {
            notes.push(format!(
                "tunings: only the first {} tuning files are offered",
                synth::MAX_TUNING_FILES
            ));
            break;
        }
        let kbm_name = format!("{stem}.kbm");
        let kbm = if names.contains(&kbm_name) {
            match read(&kbm_name) {
                Some(text) => Some(text),
                None => {
                    notes.push(format!(
                        "tunings: {file} left out: {kbm_name} can't be read"
                    ));
                    continue;
                }
            }
        } else {
            None
        };
        let Some(scl) = read(file) else {
            notes.push(format!("tunings: {file} left out: it can't be read"));
            continue;
        };
        match synth::TuningFile::parse(stem, &scl, kbm.as_deref()) {
            Ok(t) => found.push(t),
            Err(e) => notes.push(format!("tunings: {file} left out: {e}")),
        }
    }
    if !found.is_empty() {
        let names: Vec<String> = found
            .iter()
            .map(|t| {
                let kbm = if t.mapped() { ", with its .kbm" } else { "" };
                format!("{} ({} notes{kbm})", t.name, t.notes())
            })
            .collect();
        notes.push(format!("tunings: found {}", names.join(", ")));
    }
    (found, notes)
}

/// The most theme files the themes folder offers.
const MAX_THEME_FILES: usize = 32;

/// The themes folder's `NAME.theme` files that parse, in name order (at
/// most [`MAX_THEME_FILES`]), each with its theme. A theme without its own
/// `name`, or named like a built-in one, is shown by its file name. A file
/// that doesn't parse is left out with a line saying why (the second list,
/// lines for the log). Only reads.
fn load_theme_files(store: &mpfree_store::Store) -> (Vec<(String, Theme)>, Vec<String>) {
    let (mut found, mut notes) = (Vec::new(), Vec::new());
    for file in store.list(app::THEMES_FOLDER) {
        let Some(stem) = file.strip_suffix(".theme").filter(|s| !s.is_empty()) else {
            continue;
        };
        if found.len() == MAX_THEME_FILES {
            notes.push(format!(
                "themes: only the first {MAX_THEME_FILES} theme files are offered"
            ));
            break;
        }
        let text = store
            .read(app::THEMES_FOLDER, &file)
            .and_then(|b| String::from_utf8(b).ok());
        match text.map(|t| Theme::parse(&t)) {
            Some(Ok(mut t)) => {
                if app::THEMES
                    .iter()
                    .any(|b| b.1.eq_ignore_ascii_case(t.name.trim()))
                    || t.name.trim().is_empty()
                {
                    t.name = stem.to_string();
                }
                found.push((file, t));
            }
            Some(Err(e)) => notes.push(format!("themes: {file} left out: {e}")),
            None => notes.push(format!("themes: {file} left out: not readable text")),
        }
    }
    if !found.is_empty() {
        let names: Vec<&str> = found.iter().map(|(_, t)| t.name.as_str()).collect();
        notes.push(format!("themes: found {}", names.join(", ")));
    }
    (found, notes)
}

/// The built-in fonts, or files given with --font, --bold-font, --mono-font.
fn load_fonts(o: &Options, out: &mut Out) -> Fonts {
    match Fonts::with_files(
        o.font.as_deref(),
        o.bold_font.as_deref(),
        o.mono_font.as_deref(),
    ) {
        Ok(f) => f,
        Err(e) => {
            out.say(&format!("font: {e}; using the built-in fonts"));
            Fonts::embedded()
        }
    }
}

fn read_trimmed(path: &Path) -> Result<String, String> {
    fs::read_to_string(path)
        .map(|s| s.trim().to_string())
        .map_err(|e| format!("{}: {e}", path.display()))
}

fn panel_rotation() -> Result<Rotation, String> {
    let bytes = fs::read(PANEL_ROTATION).map_err(|e| format!("{PANEL_ROTATION}: {e}"))?;
    let degrees = <[u8; 4]>::try_from(bytes.as_slice())
        .map(u32::from_be_bytes)
        .map_err(|_| format!("{PANEL_ROTATION} is not one 32-bit value"))?;
    Rotation::from_degrees(degrees).ok_or_else(|| format!("unexpected panel rotation {degrees}"))
}

fn uptime() -> String {
    fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|s| {
            s.split_whitespace()
                .next()
                .map(|v| v.split('.').next().unwrap_or(v).to_string())
        })
        .unwrap_or_else(|| "?".to_string())
}

// ---------------------------------------------------------------- inventory

/// `/proc/asound/cards` as (number, id, description) per card.
fn parse_cards(text: &str) -> Vec<(u32, String, String)> {
    let mut cards = Vec::new();
    for line in text.lines() {
        let t = line.trim_start();
        let Some((num, rest)) = t.split_once(' ') else {
            continue;
        };
        let Ok(n) = num.parse::<u32>() else { continue };
        let Some(open) = rest.find('[') else { continue };
        let Some(close) = rest.find(']') else {
            continue;
        };
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
    let hits: Vec<u32> = all
        .iter()
        .filter(|(_, id, _)| id == AUDIO_CARD_ID)
        .map(|c| c.0)
        .collect();
    match hits.as_slice() {
        [n] => Ok(*n),
        _ => Err(format!(
            "expected one sound card with id {AUDIO_CARD_ID}, found {}",
            hits.len()
        )),
    }
}

/// The internal controller: a USB MIDI card whose description names the MPC One.
fn midi_card() -> Result<u32, String> {
    let all = cards();
    let hits: Vec<u32> = all
        .iter()
        .filter(|(n, _, desc)| {
            let d = desc.to_ascii_lowercase().replace('_', " ");
            d.contains("mpc one") && Path::new(&format!("/dev/snd/midiC{n}D0")).exists()
        })
        .map(|c| c.0)
        .collect();
    match hits.as_slice() {
        [n] => Ok(*n),
        _ => Err(format!(
            "expected one MPC One MIDI card with /dev/snd/midiC<n>D0, found {}",
            hits.len()
        )),
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
    out.say(&format!(
        "kernel: {}",
        read_trimmed(Path::new("/proc/sys/kernel/osrelease")).unwrap_or_default()
    ));
    for p in [
        "/proc/asound/cards",
        "/proc/asound/pcm",
        "/proc/asound/devices",
    ] {
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
    let mut names: Vec<String> = fs::read_dir("/dev/snd")
        .map(|d| {
            d.filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    out.say(&format!("/dev/snd: {}", names.join(" ")));
    for (path, name) in input_devices() {
        out.say(&format!("input: {path} = {name}"));
    }
    for f in ["name", "modes", "stride", "bits_per_pixel"] {
        let p = o.sysfs.join(f);
        out.say(&format!(
            "fb0 {f}: {}",
            read_trimmed(&p).unwrap_or_else(|e| e)
        ));
    }
    if let Ok(d) = fs::read_dir("/sys/class/drm") {
        let mut conns: Vec<PathBuf> = d
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.join("status").exists())
            .collect();
        conns.sort();
        for c in conns {
            out.say(&format!(
                "drm {}: {}",
                c.file_name().unwrap().to_string_lossy(),
                read_trimmed(&c.join("status")).unwrap_or_default()
            ));
        }
    }
    out.say(&format!(
        "panel rotation: {}",
        panel_rotation()
            .map(|r| format!("{r:?}"))
            .unwrap_or_else(|e| e)
    ));
    for p in [
        "/sys/kernel/realtime",
        "/proc/sys/kernel/sched_rt_runtime_us",
        "/proc/sys/kernel/sched_rt_period_us",
    ] {
        out.say(&format!(
            "{p}: {}",
            read_trimmed(Path::new(p)).unwrap_or_else(|e| e)
        ));
    }
    if let Ok(limits) = fs::read_to_string("/proc/self/limits") {
        for l in limits
            .lines()
            .filter(|l| l.contains("realtime") || l.contains("locked memory"))
        {
            out.say(&format!(
                "limit: {}",
                l.split_whitespace().collect::<Vec<_>>().join(" ")
            ));
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
        "mpfree.",
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
    (
        u16::from_ne_bytes([b[base], b[base + 1]]),
        u16::from_ne_bytes([b[base + 2], b[base + 3]]),
        i32::from_ne_bytes([b[base + 4], b[base + 5], b[base + 6], b[base + 7]]),
    )
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
        Touches {
            slot: 0,
            slots: [None; MAX_SLOTS],
        }
    }

    /// Feed one event; returns true at a SYN_REPORT.
    fn feed(&mut self, ty: u16, code: u16, value: i32) -> bool {
        match (ty, code) {
            (EV_ABS, ABS_MT_SLOT) => self.slot = (value.max(0) as usize).min(MAX_SLOTS - 1),
            (EV_ABS, ABS_MT_TRACKING_ID) => {
                self.slots[self.slot] = if value < 0 {
                    None
                } else {
                    Some(Contact {
                        id: value,
                        x: 0,
                        y: 0,
                    })
                }
            }
            (EV_ABS, ABS_MT_POSITION_X) => {
                if let Some(c) = self.slots[self.slot].as_mut() {
                    c.x = value
                }
            }
            (EV_ABS, ABS_MT_POSITION_Y) => {
                if let Some(c) = self.slots[self.slot].as_mut() {
                    c.y = value
                }
            }
            (EV_SYN, SYN_REPORT) => return true,
            _ => {}
        }
        false
    }

    fn describe(&self) -> String {
        let parts: Vec<String> = self
            .slots
            .iter()
            .enumerate()
            .filter_map(|(i, c)| c.map(|c| format!("slot {i}: x {} y {}", c.x, c.y)))
            .collect();
        if parts.is_empty() {
            "no contact".to_string()
        } else {
            format!("{} contact(s): {}", parts.len(), parts.join(", "))
        }
    }
}

/// The touchscreen's event device: (path, name).
fn touchscreen() -> Result<(String, String), String> {
    let devices = input_devices();
    let hits: Vec<&(String, String)> = devices
        .iter()
        .filter(|(_, n)| n.to_ascii_lowercase().contains("ili2117"))
        .collect();
    match hits.as_slice() {
        [one] => Ok((*one).clone()),
        _ => Err(format!(
            "expected one ILI2117 input device, found {}",
            hits.len()
        )),
    }
}

fn touch(o: &Options) -> Result<(), String> {
    let mut out = Out::new(o, "touch", true);
    let (path, name) = touchscreen()?;
    out.say(&format!(
        "touchscreen: {path} ({name}); read-only, not grabbed"
    ));
    let mut dev = OpenOptions::new()
        .read(true)
        .custom_flags(sys::O_NONBLOCK)
        .open(&path)
        .map_err(|e| format!("{path}: {e}"))?;
    out.say(&format!(
        "touch the screen now, with one finger and then two ({} s)",
        o.seconds
    ));
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
                        if now != last
                            && (count_changed || last_shown.elapsed() >= Duration::from_millis(200))
                        {
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
    out.say(&format!(
        "touch done: {events} events, {frames} frames, {shown} lines shown"
    ));
    Ok(())
}

// ---------------------------------------------------------------- pads

fn describe_event(e: &ControlEvent) -> String {
    match *e {
        ControlEvent::PadDown { pad, velocity } => {
            format!("pad {} down, velocity {velocity}", pad + 1)
        }
        ControlEvent::PadUp { pad } => format!("pad {} up", pad + 1),
        ControlEvent::PadPressure { pad, pressure } => {
            format!("pad {} pressure {pressure}", pad + 1)
        }
        ControlEvent::QLinkTurn { knob, delta } => format!("Q-Link {} turned {delta:+}", knob + 1),
        ControlEvent::QLinkTouch { knob, touched } => format!(
            "Q-Link {} {}",
            knob + 1,
            if touched { "touched" } else { "released" }
        ),
        ControlEvent::DataWheel { delta } => format!("data wheel {delta:+}"),
        ControlEvent::Button { note, pressed } => format!(
            "button {note} {}",
            if pressed { "pressed" } else { "released" }
        ),
        ControlEvent::KeepAliveReply { token } => format!("keep-alive reply, token {token:02X?}"),
    }
}

/// The controller's three USB-MIDI cables, as ALSA RawMIDI subdevices of
/// midiC<card>D0, with a word each subdevice's name must contain.
const MIDI_PORTS: [(u32, &str); 3] = [(0, "Public"), (1, "Private"), (2, "MIDI Port")];
/// The cable that is the 5-pin MIDI In and Out jacks, not the MPC's own
/// controls (the owner's read-only captures, 2026-11-12). What comes in on
/// it is music for `midi_in`, never pads, knobs or buttons.
const MUSIC_CABLE: u32 = 2;
/// How long after a period is made its sound is heard: the sound card's
/// buffer (PERIODS periods). MIDI Out sends each message that much after
/// its frame is made, so the jack lines up with the sound. To be measured
/// on the MPC.
const MIDI_OUT_LEAD: Duration =
    Duration::from_micros(PERIODS as u64 * PERIOD as u64 * 1_000_000 / RATE as u64);
/// Lines shown per listening step; everything is still counted after that.
const MAX_SHOWN: u64 = 300;

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The bytes of one parsed message, as they came in (running status written out).
fn message_bytes(msg: &Message<'_>) -> (Vec<u8>, bool) {
    match *msg {
        Message::Channel {
            status,
            data1,
            data2,
        } => match status & 0xF0 {
            0xC0 | 0xD0 => (vec![status, data1], false),
            _ => (vec![status, data1, data2], false),
        },
        Message::SysEx { bytes, truncated } => (bytes.to_vec(), truncated),
        Message::RealTime(b) => (vec![b], false),
        Message::Common {
            status,
            data1,
            data2,
        } => match status {
            0xF1 | 0xF3 => (vec![status, data1], false),
            0xF2 => (vec![status, data1, data2], false),
            _ => (vec![status], false),
        },
    }
}

/// One line for one message: time, port, the bytes in hex, and what it means if known.
fn message_line(
    t: f64,
    port: &str,
    bytes: &[u8],
    truncated: bool,
    event: Option<&ControlEvent>,
) -> String {
    if akai::is_identity_reply(bytes) {
        let shown = &bytes[..bytes.len().min(akai::IDENTITY_SHOWN)];
        return format!(
            "+{t:.3} s {port}: {} ... = identity reply (the rest is the serial number, not shown)",
            hex(shown)
        );
    }
    if let Some((token, status)) = akai::session_reply(bytes) {
        let what = match status {
            akai::SESSION_HELD => "held by another token",
            akai::SESSION_RUNNING => "running",
            akai::SESSION_ACCEPTED => "accepted",
            _ => "unknown status",
        };
        return format!(
            "+{t:.3} s {port}: {} = session reply, token {}, {what}",
            hex(bytes),
            hex(&token)
        );
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
    let cut = if truncated {
        " (cut short: longer than the parser keeps)"
    } else {
        ""
    };
    format!("+{t:.3} s {port}: {}{cut} = {meaning}", hex(bytes))
}

struct PortIn {
    name: &'static str,
    /// The 5-pin MIDI In ([`MUSIC_CABLE`]): its bytes go to `midi_in` as
    /// they came, not through the controller's decoder.
    music: bool,
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
        out.say(&format!(
            "listening on card {card} subdevice {} \"{}\"",
            info.subdevice, info.subname
        ));
        if info.subdevice != sub
            || info.stream != alsa::RAWMIDI_STREAM_INPUT
            || !info.subname.contains(word)
        {
            return Err(format!(
                "that is not the controller's {word} port; stopping"
            ));
        }
        inputs.push(PortIn {
            name: word,
            music: sub == MUSIC_CABLE,
            file,
            parser: Parser::default(),
            bytes: 0,
            messages: 0,
        });
    }
    Ok(inputs)
}

fn open_output(card: u32, sub: u32, word: &str, out: &mut Out) -> Result<File, String> {
    let (file, info) = alsa::open_rawmidi(card, sub, alsa::Direction::Output)
        .map_err(|e| format!("opening MIDI output {sub} ({word}): {e}"))?;
    out.say(&format!(
        "sending to card {card} subdevice {} \"{}\"",
        info.subdevice, info.subname
    ));
    if info.subdevice != sub
        || info.stream != alsa::RAWMIDI_STREAM_OUTPUT
        || !info.subname.contains(word)
    {
        return Err(format!(
            "that is not the controller's {word} port; stopping before sending anything"
        ));
    }
    Ok(file)
}

/// Write one whole message to a non-blocking MIDI output, waiting up to half a
/// second for room in the kernel's buffer. Not for the audio loop, which
/// queues its messages ([`Ports::queue`]).
fn send_all(file: &mut File, msg: &[u8]) -> io::Result<()> {
    control::send_all(file, msg, Duration::from_millis(500))
}

/// One message that came in: which input, its bytes, and what it means if
/// known. From the 5-pin MIDI In (`music`), the bytes as read, unparsed and
/// with no event.
struct Incoming {
    port: usize,
    bytes: Vec<u8>,
    event: Option<ControlEvent>,
    music: bool,
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
    /// The audio loop's outputs: queued for the `mpfree-lights` thread
    /// ([`Ports::queue`]) instead of written here.
    lights: Option<control::Lights>,
}

impl Ports {
    fn open(out: &mut Out, with_private: bool) -> Result<Ports, String> {
        let card = midi_card()?;
        let inputs = open_inputs(card, out)?;
        let public = open_output(card, mpc_one::Port::Public as u32, "Public", out)?;
        let private = if with_private {
            Some(open_output(
                card,
                mpc_one::Port::Private as u32,
                "Private",
                out,
            )?)
        } else {
            None
        };
        Ok(Ports {
            inputs,
            public,
            private,
            start: Instant::now(),
            sent: 0,
            shown: 0,
            max_shown: MAX_SHOWN,
            quiet: false,
            show_pressure: false,
            lights: None,
        })
    }

    /// From now on [`Ports::send`] queues each message for the
    /// `mpfree-lights` thread, which writes it, so the audio loop never
    /// waits for room in the MIDI port's buffer (control.rs). Allocates and
    /// starts the thread: call it before the loop.
    fn queue(&mut self) -> Result<(), String> {
        let public = self
            .public
            .try_clone()
            .map_err(|e| format!("the Public output for the lights' thread: {e}"))?;
        let private = match &self.private {
            Some(f) => Some(
                f.try_clone()
                    .map_err(|e| format!("the Private output for the lights' thread: {e}"))?,
            ),
            None => None,
        };
        self.lights = Some(
            control::Lights::start(public, private)
                .map_err(|e| format!("the lights' thread: {e}"))?,
        );
        Ok(())
    }

    /// A message the lights' thread could not write, for the log; None
    /// most periods, without allocating.
    fn queue_error(&mut self) -> Option<String> {
        self.lights.as_mut()?.error()
    }

    /// Messages the queue had no room for so far.
    fn queue_dropped(&self) -> u64 {
        self.lights.as_ref().map_or(0, |l| l.dropped())
    }

    /// The lights' thread's counts for the Dev page: refused by the port,
    /// dropped with the queue full, stalls.
    fn queue_counts(&self) -> (u64, u64, u64) {
        self.lights
            .as_ref()
            .map_or((0, 0, 0), |l| (l.failed(), l.dropped(), l.stalls()))
    }

    /// The lights' thread's watchdog (control.rs): a line when it has
    /// written nothing for 2 s with messages waiting, and when it writes
    /// again. Allocates only for those lines.
    fn queue_watch(&mut self, now: Instant) -> Option<String> {
        match self.lights.as_mut()?.watch(now)? {
            control::LightsWatch::Stalled { waiting } => Some(format!(
                "controller output: the lights' thread has written nothing for {} s with {waiting} messages \
                 waiting (the port takes nothing); the lights and the session ping are late",
                control::STALL.as_secs()
            )),
            control::LightsWatch::Resumed { after } => Some(format!(
                "controller output: the lights' thread is writing again, {:.1} s after the stall was seen",
                after.as_secs_f64()
            )),
        }
    }

    /// Stops queueing: the lights' thread writes what is left (up to a
    /// second) and the line says what it did.
    fn unqueue(&mut self) -> Option<String> {
        let s = self.lights.take()?.finish(Duration::from_secs(1));
        Some(format!(
            "controller output (its own thread): {} messages written, {} refused by the port, {} dropped with \
             the queue full, {} stalls{}",
            s.written,
            s.failed,
            s.dropped,
            s.stalls,
            if s.unfinished {
                "; still writing at the end"
            } else {
                ""
            }
        ))
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
        if let Some(l) = self.lights.as_mut() {
            if private && self.private.is_none() {
                return Err("the Private output is not open".into());
            }
            // A full queue drops the message; the lights' thread's line at
            // the end counts it.
            l.send(private, msg);
            self.sent += 1;
            return Ok(());
        }
        let (file, name) = if private {
            (
                self.private
                    .as_mut()
                    .ok_or("the Private output is not open")?,
                "Private",
            )
        } else {
            (&mut self.public, "Public")
        };
        send_all(file, msg).map_err(|e| format!("sending {} on {name}: {e}", hex(msg)))?;
        self.sent += 1;
        Ok(())
    }

    /// Send a list of messages and show them all, six to a line.
    fn send_batch(
        &mut self,
        out: &mut Out,
        private: bool,
        what: &str,
        msgs: &[&[u8]],
    ) -> Result<(), String> {
        let port = if private { "Private" } else { "Public" };
        self.line(
            out,
            &format!(
                "+{:.3} s sending {} messages on {port}: {what}",
                self.t(),
                msgs.len()
            ),
        );
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
                Err(e) if e.kind() == ErrorKind::WouldBlock || e.raw_os_error() == Some(EAGAIN) => {
                    0
                }
                Err(e) => return Err(format!("reading MIDI ({}): {e}", self.inputs[i].name)),
            };
            self.inputs[i].bytes += n as u64;
            let name = self.inputs[i].name;
            if self.inputs[i].music {
                // MIDI In: the caller hands these to `midi_in`, which logs
                // them (clock counted, not listed).
                if n > 0 {
                    got.push(Incoming {
                        port: i,
                        bytes: buf[..n].to_vec(),
                        event: None,
                        music: true,
                    });
                }
                continue;
            }
            for &b in &buf[..n] {
                let Some(msg) = self.inputs[i].parser.feed(b) else {
                    continue;
                };
                let event = mpc_one::decode(&msg);
                let (bytes, truncated) = message_bytes(&msg);
                self.inputs[i].messages += 1;
                if self.show_pressure || !matches!(event, Some(ControlEvent::PadPressure { .. })) {
                    self.shown += 1;
                    if self.shown <= self.max_shown {
                        let text = message_line(t, name, &bytes, truncated, event.as_ref());
                        out.midi(&text);
                    } else if self.shown == self.max_shown + 1 {
                        let text = format!(
                            "(more than {} messages: only counting from here)",
                            self.max_shown
                        );
                        self.line(out, &text);
                    }
                }
                got.push(Incoming {
                    port: i,
                    bytes,
                    event,
                    music: false,
                });
            }
        }
        Ok(got)
    }

    /// Light a pad brighter while it is held, as Akai's app does.
    fn light_hit(
        &mut self,
        event: &Option<ControlEvent>,
        rest: &[[u8; 3]; 16],
    ) -> Result<(), String> {
        match *event {
            Some(ControlEvent::PadDown { pad, .. }) if pad < 16 => self.send(
                false,
                &akai::pad_colour(pad, akai::hit_colour(rest[pad as usize])),
            ),
            Some(ControlEvent::PadUp { pad }) if pad < 16 => {
                self.send(false, &akai::pad_colour(pad, rest[pad as usize]))
            }
            _ => Ok(()),
        }
    }

    fn counts(&self, out: &mut Out) {
        for p in &self.inputs {
            out.say(&format!(
                "{}: {} bytes, {} messages",
                p.name, p.bytes, p.messages
            ));
        }
    }
}

// ---------------------------------------------------------------- USB keyboards

/// A USB-MIDI device the scanner thread opened for reading.
struct UsbFound {
    card: u32,
    name: String,
    file: File,
}

/// One open USB keyboard in the audio loop.
struct UsbIn {
    card: u32,
    port: mpfree_midi::port::PortId,
    file: File,
}

/// USB-MIDI keyboards on the USB-A socket, read only. A thread looks at
/// `/proc/asound/cards` every two seconds and opens each new MIDI card's
/// first input (never an output); the audio loop reads them without
/// blocking, as it reads the controller. A read error (the keyboard was
/// unplugged) closes it and lets go of its keys. A card that can't be
/// opened is logged once and tried again only after it goes away and comes
/// back.
struct UsbKeys {
    found: Receiver<Result<UsbFound, String>>,
    open: Vec<UsbIn>,
    /// Tells the scanner thread to end (when the session ends).
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Drop for UsbKeys {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

const USB_SCAN: Duration = Duration::from_secs(2);

impl UsbKeys {
    /// Start the scanner. `skip` holds the MPC's own cards (the
    /// controller and the sound card).
    fn start(skip: Vec<u32>) -> UsbKeys {
        let (tx, rx) = sync_channel(8);
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stopped = stop.clone();
        thread::Builder::new()
            .name("usb-midi-scan".into())
            .spawn(move || {
                let _ = cores::background();
                // Cards seen last time (open or not): a card is tried once
                // each time it appears.
                let mut seen: Vec<u32> = Vec::new();
                while !stopped.load(std::sync::atomic::Ordering::Relaxed) {
                    let now = midi_in::usb_midi_cards(&cards(), &skip, |n| {
                        Path::new(&format!("/dev/snd/midiC{n}D0")).exists()
                    });
                    for (card, name) in &now {
                        if seen.contains(card) {
                            continue;
                        }
                        let found = match alsa::open_rawmidi(*card, 0, alsa::Direction::Input) {
                            Ok((file, info)) if info.stream == alsa::RAWMIDI_STREAM_INPUT => {
                                Ok(UsbFound {
                                    card: *card,
                                    name: name.clone(),
                                    file,
                                })
                            }
                            Ok(_) => Err(format!(
                                "USB MIDI {name} (card {card}): its first port is not an input; not used"
                            )),
                            Err(e) => Err(format!(
                                "USB MIDI {name} (card {card}): could not open its input: {e}"
                            )),
                        };
                        if tx.send(found).is_err() {
                            return;
                        }
                    }
                    seen = now.into_iter().map(|(c, _)| c).collect();
                    thread::sleep(USB_SCAN);
                }
            })
            .ok();
        UsbKeys {
            found: rx,
            open: Vec::new(),
            stop,
        }
    }

    /// Take newly opened keyboards and read every open one. Never blocks.
    fn poll(
        &mut self,
        music: &mut midi_in::MusicIn,
        buf: &mut [u8],
        t: f64,
        keys: &mut Vec<midi_in::Key>,
        out: &mut Out,
    ) {
        while let Ok(found) = self.found.try_recv() {
            match found {
                Ok(f) => match music.plugged(&f.name) {
                    Some(port) => {
                        out.midi(&format!(
                            "+{t:.3} s {}: plugged in (USB, card {}); listening, read only",
                            f.name, f.card
                        ));
                        self.open.push(UsbIn {
                            card: f.card,
                            port,
                            file: f.file,
                        });
                    }
                    None => out.midi(&format!(
                        "+{t:.3} s {}: plugged in, but every MIDI port number is taken; not used",
                        f.name
                    )),
                },
                Err(e) => out.midi(&format!("+{t:.3} s {e}")),
            }
        }
        let mut k = 0;
        while k < self.open.len() {
            let gone = match self.open[k].file.read(buf) {
                Ok(0) => false,
                Ok(n) => {
                    let port = self.open[k].port;
                    music.bytes(port, &buf[..n], t, keys, &mut |l| out.midi(&l));
                    false
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock || e.raw_os_error() == Some(EAGAIN) => {
                    false
                }
                Err(e) => {
                    let u = &self.open[k];
                    out.midi(&format!(
                        "+{t:.3} s {}: gone (card {}: {e}); its keys let go",
                        music.label(u.port),
                        u.card
                    ));
                    true
                }
            };
            if gone {
                let u = self.open.swap_remove(k);
                music.unplugged(u.port, keys);
            } else {
                k += 1;
            }
        }
    }
}

/// The notes that play pads 1 to 16 from a keyboard, as the Synth tab
/// names notes: the notes the MPC's own pads send (`mpc_one::PAD_NOTES`,
/// Akai's classic pad map).
fn pad_note_names() -> String {
    mpc_one::PAD_NOTES
        .iter()
        .enumerate()
        .map(|(i, &n)| format!("pad {} {}", i + 1, synth::note_name(n)))
        .collect::<Vec<_>>()
        .join(", ")
}

fn pad_colours(colours: &[[u8; 3]; 16]) -> Vec<[u8; 12]> {
    (0..16u8)
        .map(|pad| akai::pad_colour(pad, colours[pad as usize]))
        .collect()
}

fn as_slices<const N: usize>(msgs: &[[u8; N]]) -> Vec<&[u8]> {
    msgs.iter().map(|m| &m[..]).collect()
}

/// Listen for `seconds`, lighting pads as they are hit and, with a session,
/// pinging every few seconds. Returns pad hits per input port.
fn listen_and_light(
    out: &mut Out,
    ports: &mut Ports,
    seconds: u64,
    rest: &[[u8; 3]; 16],
    ping: bool,
) -> Result<[u64; 3], String> {
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
    out.say(
        "every message sent is one Akai's app sent at startup (Hello MPC round 2), or a pad colour",
    );
    out.say("made by the app's own rule for a hit pad");
    let gradient = pad_colours(&akai::STARTUP_GRADIENT);
    ports.send_batch(
        &mut out,
        false,
        "Akai's startup pad colours, orange (pad 1) to teal (pad 16)",
        &as_slices(&gradient),
    )?;
    let lights: Vec<[u8; 3]> = akai::STARTUP_BUTTON_LIGHTS
        .iter()
        .map(|&[n, v]| akai::button_light(n, v))
        .collect();
    ports.send_batch(
        &mut out,
        true,
        "Akai's startup button lights",
        &as_slices(&lights),
    )?;
    out.say("holding these for 4 s");
    listen_and_light(&mut out, &mut ports, 4, &akai::STARTUP_GRADIENT, false)?;
    let rest = pad_colours(&akai::RESTING_COLOURS);
    ports.send_batch(
        &mut out,
        false,
        "your project's pad colours, as Akai's app left them",
        &as_slices(&rest),
    )?;
    out.say(&format!(
        "hit pads: each should light brighter while held ({} s)",
        o.seconds
    ));
    out.draw();
    let hits = listen_and_light(
        &mut out,
        &mut ports,
        o.seconds,
        &akai::RESTING_COLOURS,
        false,
    )?;
    ports.counts(&mut out);
    out.say(&format!(
        "lights done: {} messages sent, pad hits on Public {}, Private {}, MIDI Port {}",
        ports.sent, hits[0], hits[1], hits[2]
    ));
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
            ports.send_batch(
                out,
                false,
                "identity request and session ping",
                &[
                    &akai::IDENTITY_REQUEST[..],
                    &akai::session_ping(akai::SESSION_TOKEN)[..],
                ],
            )?;
            next_ping = Instant::now() + Duration::from_secs(akai::PING_SECONDS);
        }
        for m in ports.poll(out, &mut buf)? {
            if let Some((token, status)) = akai::session_reply(&m.bytes) {
                if token == akai::SESSION_TOKEN
                    && (status == akai::SESSION_ACCEPTED || status == akai::SESSION_RUNNING)
                {
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
fn send_setup(
    out: &mut Out,
    ports: &mut Ports,
    colours: &[[u8; 3]; 16],
    what: &str,
) -> Result<(), String> {
    out.say("session accepted; sending Akai's setup exactly as the app did");
    ports.send_batch(
        out,
        false,
        "setup, part 1 (as at +25.772 s)",
        &akai::SETUP_FIRST,
    )?;
    listen_for(out, ports, 12)?;
    ports.send_batch(
        out,
        false,
        "setup, part 2 (sent twice, as at +25.784 s)",
        &[&akai::SETUP_5A[..], &akai::SETUP_5A[..]],
    )?;
    let gradient = pad_colours(&akai::STARTUP_GRADIENT);
    ports.send_batch(
        out,
        false,
        "Akai's startup pad colours",
        &as_slices(&gradient),
    )?;
    let lights: Vec<[u8; 3]> = akai::STARTUP_BUTTON_LIGHTS
        .iter()
        .map(|&[n, v]| akai::button_light(n, v))
        .collect();
    ports.send_batch(
        out,
        true,
        "Akai's startup button lights",
        &as_slices(&lights),
    )?;
    listen_for(out, ports, 340)?;
    ports.send_batch(
        out,
        false,
        "setup, part 3 (as at +26.170 s)",
        &akai::SETUP_LAST,
    )?;
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
        return Err(
            "the controller did not accept the session within 20 s; no setup message was sent"
                .to_string(),
        );
    }
    send_setup(
        &mut out,
        &mut ports,
        &akai::RESTING_COLOURS,
        "your project's pad colours",
    )?;
    out.say(&format!(
        "hit pads, turn Q-Link 1 and the wheel, press a few buttons ({} s); pings go on every {} s",
        o.seconds,
        akai::PING_SECONDS
    ));
    out.draw();
    let hits = listen_and_light(
        &mut out,
        &mut ports,
        o.seconds,
        &akai::RESTING_COLOURS,
        true,
    )?;
    ports.counts(&mut out);
    out.say(&format!(
        "setup done: {} messages sent, pad hits on Public {}, Private {}, MIDI Port {}",
        ports.sent, hits[0], hits[1], hits[2]
    ));
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
        return Err(format!(
            "{shown} is {size} bytes; at most {MAX_KIT_BYTES} are allowed"
        ));
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
            return Err(format!(
                "{shown} grew past {MAX_KIT_BYTES} bytes while being read"
            ));
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

/// TRACK MUTE's pad colours (control map decision 5: lit = playing).
/// Tracks: each pad lit green while its track (in the mute bank) plays a
/// clip, dim green with nothing launched, red while muted, dark with no
/// track. The kit's pads:
/// lit in the kit's colour (`kit`) while they play, red while muted, dark
/// without a sound. `track(t)` is (muted, launched), None past the last
/// track. Only colours Akai's app was seen sending (a kit's resting
/// colour, or its hit).
fn mute_colours(
    keys: app::MuteKeys,
    kit: [[u8; 3]; 16],
    track: impl Fn(usize) -> Option<(bool, bool)>,
    pad_mutes: u16,
) -> [[u8; 3]; 16] {
    let (green, red) = (akai::KIT_COLOURS[3], akai::KIT_COLOURS[4]);
    std::array::from_fn(|pad| match keys {
        app::MuteKeys::Off => kit[pad],
        app::MuteKeys::Tracks => match track(pad) {
            Some((false, true)) => akai::hit_colour(green),
            Some((false, false)) => green,
            Some((true, _)) => red,
            None => [0; 3],
        },
        app::MuteKeys::Pads if kit[pad] == [0; 3] => [0; 3],
        app::MuteKeys::Pads if pad_mutes & (1 << pad) != 0 => red,
        app::MuteKeys::Pads => akai::hit_colour(kit[pad]),
    })
}

/// TRACK MUTE's key `pad` pressed: its track in bank `bank` (A = 0:
/// tracks 1-16, B 17-32 ...), or the pad itself in the kit, muted or let
/// play. The line for the log.
fn mute_key(
    keys: app::MuteKeys,
    bank: usize,
    pad: usize,
    time: &mut time::TimeHost,
    mix: &mut mixing::MixHost,
) -> String {
    match keys {
        app::MuteKeys::Off => String::new(),
        app::MuteKeys::Tracks => time
            .toggle_mute(bank * 16 + pad)
            .unwrap_or_else(|e| format!("track mute: {e}")),
        app::MuteKeys::Pads => {
            let mute = mix.pad_mutes() & (1 << pad) == 0;
            mix.play(mixing::PlayFx::PadMute { pad, mute });
            format!(
                "pad mute: pad {} {}",
                pad + 1,
                if mute { "muted" } else { "plays" }
            )
        }
    }
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
            Some(pad) => out.say(&format!(
                "  pad {:2} (note {:3}): {} ({:.2} s)",
                pad + 1,
                s.note,
                s.name,
                s.frames() as f64 / RATE as f64
            )),
            None => out.say(&format!(
                "  note {}: {} (no pad sends this note)",
                s.note, s.name
            )),
        }
    }
}

/// Play the first kit in the kits file from the pads, through ACVA.
fn drums(o: &Options) -> Result<(), String> {
    let mut out = Out::new(o, "factory drums", true);
    let mut kits = load_kits(&o.kits, &mut |_, _| {})?;
    kits.truncate(1);
    out.say(&format!(
        "kit: {} ({} sounds)",
        kits[0].name,
        kits[0].sounds.len()
    ));
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
    ports.send_batch(
        &mut out,
        false,
        "pad colours: lit where the kit has a sound, dark elsewhere",
        &as_slices(&colours),
    )?;
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
    let (mut hits, mut silent_hits, mut clipped, mut peak, mut voices) =
        (0u64, 0u64, 0u64, 0f32, 0usize);
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
        Latency {
            n: 0,
            sum: 0.0,
            min: f64::INFINITY,
            max: 0.0,
            last: f64::NAN,
        }
    }

    fn add(&mut self, ms: f64) {
        self.n += 1;
        self.sum += ms;
        self.min = self.min.min(ms);
        self.max = self.max.max(ms);
        self.last = ms;
    }

    fn mean(&self) -> f64 {
        if self.n == 0 {
            f64::NAN
        } else {
            self.sum / self.n as f64
        }
    }

    fn describe(&self) -> String {
        if self.n == 0 {
            return "no hits timed".to_string();
        }
        format!(
            "{} hits timed, mean {:.2} ms, min {:.2} ms, max {:.2} ms",
            self.n,
            self.mean(),
            self.min,
            self.max
        )
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

/// What the pad player's screen shows about each kit, then the synth.
fn kit_views(kits: &[kit::Kit], synth: &synth::Synth) -> Vec<gui::KitView> {
    let mut views: Vec<gui::KitView> = kits
        .iter()
        .enumerate()
        .map(|(k, kit)| {
            let mut pads = vec![None; 16];
            for s in &kit.sounds {
                if let Some(pad) = mpc_one::pad_index(s.note) {
                    pads[pad as usize] = Some(s.name.clone());
                }
            }
            gui::KitView {
                name: kit.name.clone(),
                pads,
                colour: akai::kit_colour(k),
                chip: None,
                synth: None,
            }
        })
        .collect();
    views.push(synth.kit_view());
    views
}

/// The loading steps, for the progress bar.
const STEPS: usize = 6;

/// One loading step on the screen (and in the log).
fn progress(out: &mut Out, step: usize, message: &str, detail: &str, value: f32) {
    out.show(gui::ToUi::Progress {
        message: message.to_string(),
        detail: format!("Step {step} of {STEPS}   ·   {detail}"),
        value,
    });
}

/// Wait for `find` to succeed, for up to `wait`, showing how long it has been.
fn wait_for<T>(
    out: &mut Out,
    step: usize,
    message: &str,
    value: f32,
    wait: Duration,
    mut find: impl FnMut() -> Result<T, String>,
) -> Result<T, String> {
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
                    progress(
                        out,
                        step,
                        message,
                        &format!("Waiting for it to appear ({s} s)"),
                        value,
                    );
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
    // Did the last run in this boot crash? Then this is a safe start
    // (crash.rs). `_running` is this run's mark, removed when it ends well.
    let (last_run, _running) = match crash::start(Path::new(REPORT_DIR)) {
        (last, Ok(r)) => (last, Some(r)),
        (last, Err(e)) => {
            out.say(&format!(
                "safe start: off for the next start (no {REPORT_DIR}/{}: {e})",
                crash::RUNNING
            ));
            (last, None)
        }
    };
    if let Some(line) = crash::report_line(&last_run) {
        out.say(&line);
    }
    // Long jobs run at idle priority, after the screen and every other
    // thread (mpfree_lanes::os::idle_job): set before any of them starts.
    mpfree_lanes::os::set_idle_jobs(true);
    out.say(
        "threads: long jobs (stretching, looper prints, sampling a synth) at idle priority, \
         after the screen",
    );
    let fonts = load_fonts(o, &mut out);
    let mut fb = None;
    let fb_wait = if timed {
        Duration::ZERO
    } else {
        Duration::from_secs(20)
    };
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
    // After a crash the panel holds the crashed run's last frame instead,
    // so the built-in backdrop is used.
    let backdrop = match (&fb, o.keep_screen) {
        (Some(f), true) if last_run == crash::LastRun::Fine => {
            match mpfree_ui::fb::capture(&o.fb, &f.layout) {
                Ok(img) => Some(std::sync::Arc::new(img)),
                Err(e) => {
                    out.say(&format!("screen: could not read the startup screen back ({e}); using the built-in backdrop"));
                    None
                }
            }
        }
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
    let mut player = gui::Player::new(fonts, size, timed, backdrop);
    if last_run != crash::LastRun::Fine {
        player.safe_start();
    }
    match load_theme(o) {
        Ok(t) => player.ui.set_theme(t),
        Err(e) => out.say(&format!("{e}; using the default theme")),
    }
    let (to_ui, from_audio) = sync_channel::<gui::ToUi>(4096);
    let (to_audio, from_ui) = sync_channel::<gui::FromUi>(256);
    // The screen thread starts before this thread asks for real-time priority,
    // so it keeps normal priority.
    // With --gpu-screen, the GPU draws the screen in a process of its own,
    // turned as the framebuffer is; without a framebuffer there is nothing
    // to fall back to, so the software draws (or nothing does).
    let gpu = match (&o.gpu_screen, &fb) {
        (Some(program), Some(f)) => Some(drawing::GpuStart::new(program, f.layout.rotation)),
        (Some(_), None) => {
            out.say("screen: not drawn on the GPU (no framebuffer to fall back to)");
            None
        }
        (None, _) => None,
    };
    let screen = gui::spawn(
        player,
        fb,
        gpu,
        Some(open_touch),
        Some(to_ui.clone()),
        from_audio,
        to_audio,
    )
    .map_err(|e| format!("screen thread: {e}"))?;
    out.sink = Some(to_ui.clone());
    if !timed {
        follow_journal(to_ui.clone());
    }
    let result = player_session(o, &mut out, timed, &from_ui);
    if let Err(e) = &result {
        // On the screen only: main prints the STOP line as for every step.
        out.show(gui::ToUi::Session("Stopped".into(), Tone::Bad));
        out.show(gui::ToUi::Failed {
            message: "Stopped".into(),
            detail: e.clone(),
        });
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
    let report = screen
        .join()
        .map_err(|_| "the screen thread stopped with an error".to_string())?;
    out.sink = None;
    let drawn = match &report.error {
        Some(e) => format!(" (stopped: {e})"),
        None => String::new(),
    };
    out.say(&format!(
        "screen: {} frames, mean {:.1} ms, slowest {:.1} ms to draw and copy; {} touches; \
                      {} lines the screen had no room for{drawn}",
        report.frames, report.mean_ms, report.max_ms, report.touches, out.dropped
    ));
    result
}

/// The largest arp pattern library file read (a 64-step pattern with five
/// full lanes is about 1 KB, so this is hundreds of patterns).
const PATTERNS_MAX_BYTES: u64 = 256 * 1024;

/// Reads the arp pattern library file (read-only).
fn read_patterns(path: &Path) -> Result<String, String> {
    let f = File::open(path).map_err(|e| e.to_string())?;
    let mut text = String::new();
    f.take(PATTERNS_MAX_BYTES + 1)
        .read_to_string(&mut text)
        .map_err(|e| e.to_string())?;
    if text.len() as u64 > PATTERNS_MAX_BYTES {
        return Err(format!("larger than {} KB", PATTERNS_MAX_BYTES / 1024));
    }
    Ok(text)
}

/// The system log, followed (`journalctl -f`, read-only) into the Console page.
fn follow_journal(tx: SyncSender<gui::ToUi>) {
    let spawned = thread::Builder::new()
        .name("mpfree-journal".into())
        .spawn(move || {
            let child = std::process::Command::new("journalctl")
                .args([
                    "-b",
                    "-f",
                    "-n",
                    "300",
                    "--no-pager",
                    "-o",
                    "short-monotonic",
                    "--no-hostname",
                ])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .spawn();
            let mut child = match child {
                Ok(c) => c,
                Err(e) => {
                    let _ = tx.try_send(gui::ToUi::Console(format!(
                        "journalctl: {e} (no system log here)"
                    )));
                    return;
                }
            };
            let Some(stdout) = child.stdout.take() else {
                return;
            };
            for line in io::BufRead::lines(io::BufReader::new(stdout)) {
                let Ok(l) = line else { break };
                if let Err(std::sync::mpsc::TrySendError::Disconnected(_)) =
                    tx.try_send(gui::ToUi::Console(l))
                {
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
        let mean = if n == 0 {
            0.0
        } else {
            total.as_secs_f32() * 1e6 / n as f32
        };
        (mean, max.as_secs_f32() * 1e6)
    }

    fn describe(&self) -> String {
        let period_us = PERIOD as f64 * 1e6 / RATE as f64;
        let mean = if self.periods == 0 {
            0.0
        } else {
            self.total.as_secs_f64() * 1e6 / self.periods as f64
        };
        let max = self.max.as_secs_f64() * 1e6;
        format!(
            "render mean {mean:.0} us, max {max:.0} us ({:.1} % and {:.1} % of a period)",
            100.0 * mean / period_us,
            100.0 * max / period_us
        )
    }
}

/// The sampler's part of the status line.
fn sampler_status(player: &sampler::Player, time: &RenderTime) -> String {
    let c = player.counters();
    format!(
        "sampler: {}, {} steals, {} events dropped, at most {} grains ({} skipped)",
        time.describe(),
        c.steals,
        c.dropped_events,
        c.peak_grains,
        c.skipped_grains
    )
}

/// How often the screen hears the song position while it moves (the bar
/// shows bar.beat.tick; state changes go at once).
const TRANSPORT_EVERY: Duration = Duration::from_millis(33);

/// Whether the mixer is open on its I/O view.
fn io_shown(mix: &mixing::MixHost) -> bool {
    mix.shown() == Some(mpfree_mixer_ui::MixView::Io)
}

/// Whether `view` shows the loop's playhead: the Looper tab, or the Play
/// page with its loop strip.
fn strip_shown(view: app::View) -> bool {
    matches!(view, app::View::Looper | app::View::Play)
}
/// How often the Play page's master meter and Q-Link values are sent.
const METER_EVERY: Duration = Duration::from_millis(50);
/// How often the Sample page's playheads are sent while it shows.
const SAMPLE_HEADS_EVERY: Duration = Duration::from_millis(40);
/// While overdubbing, the loop's waveform is redrawn about once a second.
const LOOP_WAVE_PERIODS: u64 = 344;

/// The sequencer's notes on the pads: the sampler's kit, or the synth while the
/// synth has the pads. Each hit is noted so the screen's pads can flash.
struct SeqPads<'a> {
    player: &'a mut sampler::Player,
    synth: Option<&'a mut synth::Synth>,
    flashes: [u8; 16],
    hits: u64,
    /// The live treatment's inputs (velocity curve, pad layout).
    live: app::LiveMap,
    /// The pads the arps' notes started this period, and their velocity,
    /// for the screen (pad down, or a repeat's flash).
    live_hits: [(u8, u8); MAX_LIVE_HITS],
    n_live: usize,
    /// Arp notes on a pad with no sound.
    silent: u64,
    /// Tracks whose "Plays on" is MIDI Out play there instead.
    midi: &'a mut midi_out::MidiOut,
    /// The CV/gate outputs follow the tracks' notes wherever they play.
    cv: &'a mut cv_out::CvOut,
    /// Tracks with their own instrument play it (instruments.rs), not the
    /// pads.
    own: &'a mut instruments::Instruments,
    /// While an export runs, every event goes to its MIDI file too, as
    /// MIDI Out would send it (export.rs).
    tap: &'a mut export::Tap,
}

/// What the Export panel and plan need to know about the song (export.rs),
/// gathered when Export is pressed.
fn export_facts(
    time: &time::TimeHost,
    mix: &mixing::MixHost,
    midi_out: &midi_out::MidiOut,
    music: &midi_in::MusicIn,
    cv_out: &cv_out::CvOut,
    app: &app::App,
    looper: &looping::LoopHost,
) -> export::Facts {
    let tracks = export::track_names(time);
    let rig = mpfree_export::Rig {
        external_midi: (0..tracks.len())
            .filter(|&t| midi_out.takes(t as u16))
            .map(|t| tracks[t].clone())
            .collect(),
        line_input: app.settings.line_input || looper.line.is_open(),
        cv: cv_out.in_use(),
        clock_out: midi_out.sends_clock(),
        clock_in: music.leader_name().map(str::to_string),
        offload: false,
        plugins: export::plugins_in(&mix.mixer, &mix.instruments),
    };
    export::Facts::gather(time, mix, rig)
}

/// A learned MIDI control's value (0 to 1 over the parameter's range) set
/// as a chain editor knob sets it (M5): the value set, in the parameter's
/// own units. None when MPFree has no way to set that parameter from a
/// control yet (a synth's, for now).
fn learned_set(
    panels: &mut modpanels::Panels,
    mix: &mut mixing::MixHost,
    w: mpfree_midi::learn::Write,
    recording: bool,
) -> Option<f32> {
    use mpfree_play::mpfree_param::Owner;
    let value = match w.target.owner {
        Owner::Macro(_) => w.value,
        _ => {
            let (info, _) = modmix::describe(&mix.mixer, w.target)?;
            info.from_unit(w.value)
        }
    };
    panels.set_param(mix, w.target, value, recording);
    Some(value)
}

/// The notes pad `pad` plays on MIDI Out: the synth's pad layout while the
/// synth has the pads, else the MPC's own pad note.
fn pad_notes(synth: Option<&synth::Synth>, pad: usize) -> &[u8] {
    match synth {
        Some(s) => s.notes(pad),
        None => midi_out::kit_notes(pad),
    }
}

/// What the Synth tab shows: the selected track's own synth when it has
/// one (instruments.rs), else the shared synth.
fn synth_tab(
    synth: &synth::Synth,
    own: &instruments::Instruments,
    track: usize,
    preset: Option<usize>,
) -> SynthState {
    match (own.held(track), own.node(track)) {
        (Some(h), Some(node)) => synth.state_of(
            node,
            synth::Engine::from_node_id(h.id),
            &format!("Track {}", track + 1),
            preset,
        ),
        _ => synth.state(),
    }
}

/// The tracks' own synths follow the Synth tab's pad layout, octave and
/// tuning (Drums keeps its kit notes).
fn own_follow_synth(synth: &synth::Synth, own: &mut instruments::Instruments) {
    let tuning = synth.tuning_now();
    for t in 0..instruments::TRACKS {
        if let Some(h) = own.held(t) {
            let e = synth::Engine::from_node_id(h.id).unwrap_or(synth::Engine::Poly);
            own.set_pads(t, synth.pads_for(e));
            own.set_tuning(t, &tuning);
        }
    }
}

/// What the screen says when track `track` can't have the instrument
/// `name` because [`instruments::SLOTS`] other tracks already have their
/// own.
fn full_note(track: usize, name: &str) -> String {
    format!(
        "Track {} plays the kit: {} tracks already have their own instrument, the most at once. Set one of them to Kit to give {name} its place",
        track + 1,
        instruments::SLOTS
    )
}

/// Instrument `id`'s name for the screen: an engine's or a plugin's.
fn instrument_name(plugins: &plugins::Plugins, id: &'static str) -> &'static str {
    synth::Engine::from_node_id(id)
        .map(|e| e.name())
        .or_else(|| plugins.find(id).map(|e| e.name))
        .unwrap_or(id)
}

/// A job for track `track`'s own instrument `id`: an engine's copy of the
/// shared synth's sound (or `values`), or a plugin synth, with the pad
/// layout and tuning now.
fn instrument_job(
    synth: &synth::Synth,
    track: usize,
    id: &'static str,
    values: Option<Vec<f32>>,
) -> control::InstrumentJob {
    let engine = synth::Engine::from_node_id(id);
    let values = match (values, engine) {
        (Some(v), _) => v,
        (None, Some(e)) => synth.sound_of(e).1,
        (None, None) => Vec::new(),
    };
    control::InstrumentJob {
        track,
        id,
        values,
        pads: synth.pads_for(engine.unwrap_or(synth::Engine::Poly)),
        tuning: synth.tuning_now(),
    }
}

/// Pad notes from the arps one period can show; more still sound.
const MAX_LIVE_HITS: usize = 64;

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
        self.own.all_off_every();
    }
    fn pressure_at(&mut self, pad: usize, value: u8, offset: u32) {
        match self.synth.as_mut() {
            Some(s) => s.pressure_at(pad, value, offset),
            None => self.player.pressure_at(pad, value, offset),
        }
    }

    /// An arp's note: the pads played live (through the armed track's arp,
    /// a latched arp, or a clip through its arp). It gets the velocity
    /// curve; with a synth pad layout it is a MIDI note for the synth and
    /// its pad (if any) is what shows and records; otherwise it is a pad.
    /// Returns what the armed track records as heard.
    fn live(&mut self, o: &mpfree_play::Out) -> Option<mpfree_play::Event> {
        use mpfree_play::Event;
        let notes = self.synth.is_some() && self.live.plays_notes();
        self.cv_live(o, notes);
        if self.midi.takes(o.track) {
            return self.live_out(o, notes);
        }
        if self.own.holds(usize::from(o.track)) {
            return self.live_own(o, notes);
        }
        match o.event {
            Event::NoteOn { note, velocity } => {
                let v = self.live.velocity(velocity);
                let pad = if notes {
                    // A note no pad has (an arp octave up, a scale run)
                    // still plays; only a pad's own note shows and records.
                    if let Some(s) = self.synth.as_mut() {
                        s.note_on_note(note, v, o.offset);
                    }
                    self.hits += 1;
                    self.live.pad_of_note(note)?
                } else {
                    let pad = usize::from(note);
                    if pad >= 16 {
                        return None;
                    }
                    match self.synth.as_mut() {
                        Some(s) => {
                            s.note_on(pad, v, o.offset);
                            self.hits += 1;
                        }
                        // While live chop takes cuts, a hit marks one.
                        None if self.player.live_hit_at(pad, v, o.offset) => self.hits += 1,
                        None => self.silent += 1,
                    }
                    pad
                };
                if self.n_live < MAX_LIVE_HITS {
                    self.live_hits[self.n_live] = (pad as u8, v);
                    self.n_live += 1;
                }
                Some(Event::NoteOn {
                    note: pad as u8,
                    velocity: v,
                })
            }
            Event::NoteOff { note } if notes => {
                if let Some(s) = self.synth.as_mut() {
                    s.note_off_note(note, o.offset);
                }
                let pad = self.live.pad_of_note(note)?;
                Some(Event::NoteOff { note: pad as u8 })
            }
            Event::NoteOff { note } => {
                // The synth's note ends; a Grains pad stops when let go;
                // one-shot sounds ignore it.
                match self.synth.as_mut() {
                    Some(s) => s.note_off(usize::from(note), o.offset),
                    None => self.player.live_release_at(usize::from(note), o.offset),
                }
                Some(o.event)
            }
            Event::AllOff => {
                self.midi.all_off(o.offset);
                self.stop_all();
                None
            }
            Event::Pressure { note, value } => {
                self.pressure_at(usize::from(note), value, o.offset);
                None
            }
            Event::Lock { .. } | Event::Lane { .. } | Event::Unlock { .. } => None,
        }
    }

    /// A clip's event on a track that plays on MIDI Out: sent there, and
    /// the pads don't play it (its pad still flashes). A panic ends MIDI
    /// Out's notes and goes on to the pads.
    fn out(&mut self, o: &mpfree_play::Out) -> bool {
        use mpfree_play::Event;
        let pad = match o.event {
            Event::NoteOn { note, .. } | Event::NoteOff { note } | Event::Pressure { note, .. } => {
                usize::from(note)
            }
            _ => 0,
        };
        let is = midi_out::NoteIs::Pad(pad_notes(self.synth.as_deref(), pad));
        self.cv.send(o, is);
        self.tap.send(o, is);
        let taken = self.midi.send(o, is) || self.clip_own(o);
        if let (true, Event::NoteOn { velocity, .. }) = (taken, o.event) {
            if let Some(f) = self.flashes.get_mut(pad) {
                *f = velocity.max(1);
            }
        }
        taken
    }
}

impl SeqPads<'_> {
    /// A clip's event on a track with its own instrument: played there,
    /// not on the pads. A panic or a lock goes on as usual.
    fn clip_own(&mut self, o: &mpfree_play::Out) -> bool {
        use mpfree_play::Event;
        let track = usize::from(o.track);
        if !self.own.holds(track) {
            return false;
        }
        match o.event {
            Event::NoteOn { note, velocity } => {
                if self
                    .own
                    .pad_on(track, usize::from(note), velocity, o.offset)
                {
                    self.hits += 1;
                }
                true
            }
            Event::NoteOff { note } => self.own.pad_off(track, usize::from(note), o.offset),
            Event::Pressure { note, value } => {
                self.own
                    .pad_pressure(track, usize::from(note), value, o.offset)
            }
            Event::AllOff => {
                self.own.all_off(track);
                false
            }
            Event::Lock { .. } | Event::Lane { .. } | Event::Unlock { .. } => false,
        }
    }

    /// An arp's note (the pads played live) on a track with its own
    /// instrument: played there with the pads' treatment. With a synth pad
    /// layout (`notes`) it is a MIDI note; otherwise a pad, which the
    /// track's own pad map turns into notes. Returns what records.
    fn live_own(&mut self, o: &mpfree_play::Out, notes: bool) -> Option<mpfree_play::Event> {
        use mpfree_play::Event;
        let track = usize::from(o.track);
        match o.event {
            Event::NoteOn { note, velocity } => {
                let v = self.live.velocity(velocity);
                let pad = if notes {
                    self.own.note_on(track, note, v, o.offset);
                    self.hits += 1;
                    self.live.pad_of_note(note)?
                } else {
                    let pad = usize::from(note);
                    if pad >= 16 {
                        return None;
                    }
                    self.own.pad_on(track, pad, v, o.offset);
                    self.hits += 1;
                    pad
                };
                if self.n_live < MAX_LIVE_HITS {
                    self.live_hits[self.n_live] = (pad as u8, v);
                    self.n_live += 1;
                }
                Some(Event::NoteOn {
                    note: pad as u8,
                    velocity: v,
                })
            }
            Event::NoteOff { note } if notes => {
                self.own.note_off(track, note, o.offset);
                let pad = self.live.pad_of_note(note)?;
                Some(Event::NoteOff { note: pad as u8 })
            }
            Event::NoteOff { note } => {
                self.own.pad_off(track, usize::from(note), o.offset);
                Some(o.event)
            }
            Event::AllOff => {
                self.midi.all_off(o.offset);
                self.own.all_off(track);
                time::Pads::stop_all(self);
                None
            }
            Event::Pressure { note, value } if !notes => {
                self.own
                    .pad_pressure(track, usize::from(note), value, o.offset);
                None
            }
            Event::Pressure { .. }
            | Event::Lock { .. }
            | Event::Lane { .. }
            | Event::Unlock { .. } => None,
        }
    }

    /// An arp's note to the CV outputs, with the pads' velocity curve, as
    /// MIDI Out would read it (`notes`: MIDI notes from a synth pad
    /// layout; else pads).
    fn cv_live(&mut self, o: &mpfree_play::Out, notes: bool) {
        use midi_out::NoteIs;
        use mpfree_play::Event;
        let (event, pad) = match o.event {
            Event::NoteOn { note, velocity } => (
                Event::NoteOn {
                    note,
                    velocity: self.live.velocity(velocity),
                },
                usize::from(note),
            ),
            e => (e, 16),
        };
        let is = match (notes, event) {
            (true, _) => NoteIs::Note,
            (false, Event::NoteOn { .. }) if pad < 16 => {
                NoteIs::Pad(pad_notes(self.synth.as_deref(), pad))
            }
            // A pad's note-off and pressure go to what its note-on sent.
            (false, _) => NoteIs::Pad(&[]),
        };
        let o = mpfree_play::Out { event, ..*o };
        self.cv.send(&o, is);
        self.tap.send(&o, is);
    }

    /// An arp's note on a track that plays on MIDI Out: sent there with
    /// the pads' velocity curve, shown on the pads, and recorded as
    /// [`time::Pads::live`] would record it (`notes`: the arp's notes are
    /// MIDI notes, from a synth pad layout).
    fn live_out(&mut self, o: &mpfree_play::Out, notes: bool) -> Option<mpfree_play::Event> {
        use midi_out::NoteIs;
        use mpfree_play::Event;
        match o.event {
            Event::NoteOn { note, velocity } => {
                let v = self.live.velocity(velocity);
                let played = mpfree_play::Out {
                    event: Event::NoteOn { note, velocity: v },
                    ..*o
                };
                let pad = if notes {
                    self.midi.send(&played, NoteIs::Note);
                    self.live.pad_of_note(note)?
                } else {
                    let pad = usize::from(note);
                    if pad >= 16 {
                        return None;
                    }
                    let is = NoteIs::Pad(pad_notes(self.synth.as_deref(), pad));
                    self.midi.send(&played, is);
                    pad
                };
                if self.n_live < MAX_LIVE_HITS {
                    self.live_hits[self.n_live] = (pad as u8, v);
                    self.n_live += 1;
                }
                Some(Event::NoteOn {
                    note: pad as u8,
                    velocity: v,
                })
            }
            Event::NoteOff { note } if notes => {
                self.midi.send(o, NoteIs::Note);
                let pad = self.live.pad_of_note(note)?;
                Some(Event::NoteOff { note: pad as u8 })
            }
            Event::NoteOff { .. } => {
                self.midi.send(o, NoteIs::Pad(&[]));
                Some(o.event)
            }
            Event::Pressure { .. } => {
                let is = if notes {
                    NoteIs::Note
                } else {
                    NoteIs::Pad(&[])
                };
                self.midi.send(o, is);
                None
            }
            Event::AllOff => {
                self.midi.all_off(o.offset);
                time::Pads::stop_all(self);
                None
            }
            Event::Lock { .. } | Event::Lane { .. } | Event::Unlock { .. } => None,
        }
    }
}

/// The looper's presses and settings, and the line input switched on or
/// off (opening the capture device; a failure leaves it off and says why).
fn looper_action(
    out: &mut Out,
    looper: &mut looping::LoopHost,
    app: &mut app::App,
    card: u32,
    a: looping::LoopAction,
) -> Vec<app::Effect> {
    match a {
        // UNDO and REDO say what they did in place of the hint.
        looping::LoopAction::Command(
            c @ (mpfree_looper::Command::Undo | mpfree_looper::Command::Redo),
        ) => {
            vec![app::Effect::Tell(looper.command(c))]
        }
        looping::LoopAction::Command(c) => {
            out.note(&looper.command(c));
            Vec::new()
        }
        looping::LoopAction::Resample(n) => {
            out.note(&looper.resample_into(n));
            Vec::new()
        }
        looping::LoopAction::Steps(n) => {
            out.note(&looper.steps(n));
            Vec::new()
        }
        looping::LoopAction::Source(s) => {
            looper.source = s;
            out.note(&format!("looper: records from {}", s.name().to_lowercase()));
            Vec::new()
        }
        looping::LoopAction::Monitor(on) => {
            looper.monitor = on;
            out.note(&format!(
                "looper: monitor {}",
                if on {
                    "on: the line input is heard"
                } else {
                    "off"
                }
            ));
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
        looping::LoopAction::Layer(id, c) => {
            out.note(&looper.layer_command(id, c));
            Vec::new()
        }
        looping::LoopAction::Select(n) => {
            out.note(&looper.select(n));
            Vec::new()
        }
        looping::LoopAction::ToggleMute(n) => {
            out.note(&looper.toggle_mute(n));
            Vec::new()
        }
        // A dragged layer says what it did (or why not) in place of the
        // hint.
        looping::LoopAction::CopyLayer { from, id, to, keep } => {
            vec![app::Effect::Tell(looper.copy_layer(from, id, to, keep))]
        }
    }
}

/// A touch or a button for the time engine. The tempo belongs to the app
/// (Note Repeat and the arp follow it too), so the Arrange page's tempo goes
/// there; the transport follows it each period. Returns the app's effects.
fn time_action(
    out: &mut Out,
    time: &mut time::TimeHost,
    tracks: &mut audiotracks::TrackHost,
    looper: &looping::LoopHost,
    app: &mut app::App,
    mix: &mut mixing::MixHost,
    a: TimeAction,
) -> Vec<app::Effect> {
    if let TimeAction::Transport(Command::SetTempo(bpm)) = a {
        return app.set_tempo(bpm);
    }
    // The audio lanes' touches go to the audio tracks; a pick stays with
    // the time engine, which keeps the selection.
    if let TimeAction::Audio(aa) = a {
        if !matches!(aa, AudioAction::Pick(_)) {
            match tracks.action(aa, &time.info(), looper.line.is_open()) {
                Ok(line) if !line.is_empty() => out.note(&line),
                Ok(_) => {}
                Err(e) => out.note(&format!("audio tracks: {e}")),
            }
            // An erase leaves the picked region's place empty.
            if matches!(aa, AudioAction::Erase(_)) {
                let _ = time.act(TimeAction::Audio(AudioAction::Pick(None)));
            }
            return Vec::new();
        }
    }
    // ERASE + a clip, a drop on the Clips grid or the Song view, a block
    // deleted, UNDO and REDO: said in place of the hint.
    let undo = matches!(a, TimeAction::Undo | TimeAction::Redo);
    let erase = matches!(
        a,
        TimeAction::DeleteClip { .. }
            | TimeAction::DropClip { .. }
            | TimeAction::DropRow { .. }
            | TimeAction::MovePlacement { .. }
            | TimeAction::StretchPlacement { .. }
            | TimeAction::MoveSection { .. }
            | TimeAction::DeletePlacement(_)
    );
    let r = match a {
        // The scene morph runs on the mixer's mod engine.
        TimeAction::MorphRows { .. }
        | TimeAction::Morph(_)
        | TimeAction::SnapBack
        | TimeAction::KeepMorph
        | TimeAction::SaveScene(_) => time.morph(a, &mut mix.modu, &mix.mixer),
        _ => time.act(a),
    };
    match r {
        Ok(line) | Err(line) if erase => return vec![app::Effect::Tell(line)],
        Ok(line) | Err(line) if undo => return vec![app::Effect::Tell(undo_said(&line))],
        Ok(line) if !line.is_empty() => out.note(&format!("time: {line}")),
        Ok(_) => {}
        Err(e) => out.note(&format!("time: {e}")),
    }
    Vec::new()
}

/// The time engine's "undo: take 3 · Drums / 1" as the screen says it:
/// "Undid take 3 · Drums / 1", or that there is nothing to undo.
fn undo_said(line: &str) -> String {
    match line.split_once(": ") {
        Some(("undo", "nothing to undo")) => "Nothing to undo in the takes".into(),
        Some(("redo", "nothing to redo")) => "Nothing to redo in the takes".into(),
        Some(("undo", what)) => format!("Undid {what}"),
        Some(("redo", what)) => format!("Redid {what}"),
        _ => line.to_string(),
    }
}

/// What UNDO (or REDO) of a synth knob says: "Undid Filter Cutoff (now
/// 1.2 kHz)".
fn synth_turn_said(synth: &synth::Synth, id: u16, redo: bool) -> String {
    let done = if redo { "Redid" } else { "Undid" };
    match synth.param_info(id) {
        Some(p) => {
            let name = if p.group == p.name {
                p.name.to_string()
            } else {
                format!("{} {}", p.group, p.name)
            };
            let now = mpfree_synth_ui::format::value_line(p, synth.value(id));
            format!("{done} {name} (now {now})")
        }
        None => format!("{done} a synth knob"),
    }
}

/// A synth knob's turn as the undo history lists it: "Filter Cutoff to
/// 1.2 kHz".
fn synth_turn_row(synth: &synth::Synth, id: u16, to: f32) -> String {
    match synth.param_info(id) {
        Some(p) => {
            let name = if p.group == p.name {
                p.name.to_string()
            } else {
                format!("{} {}", p.group, p.name)
            };
            format!("{name} to {}", mpfree_synth_ui::format::value_line(p, to))
        }
        None => "A synth knob".to_string(),
    }
}

/// A mixer control's turn as the undo history lists it: "Drums Volume to
/// -6.0 dB".
fn mix_turn_row(strip: mpfree_fx::StripId, id: u16, to: f32) -> String {
    match mpfree_fx::strip_params().iter().find(|p| p.id == id) {
        Some(p) => format!(
            "{} {} to {}",
            mixing::name(strip),
            p.name,
            mpfree_synth_ui::format::value_line(p, to)
        ),
        None => format!("{} control", mixing::name(strip)),
    }
}

/// What UNDO (or REDO) of a mixer control says: "Undid Drums Volume".
fn mix_turn_said(strip: mpfree_fx::StripId, id: u16, redo: bool) -> String {
    let done = if redo { "Redid" } else { "Undid" };
    let name = mpfree_fx::strip_params()
        .iter()
        .find(|p| p.id == id)
        .map_or("a control", |p| p.name);
    format!("{done} {} {name}", mixing::name(strip))
}

/// Everything the time pages show, with the audio tracks' lanes.
fn time_state(time: &time::TimeHost, tracks: &audiotracks::TrackHost) -> mpfree_time_ui::TimeState {
    let mut st = time.state();
    tracks.fill(&mut st, time.tempo_map());
    st
}

/// The synth's part of the status line.
fn synth_status(synth: &synth::Synth, time: &RenderTime) -> String {
    let c = synth.counters();
    format!(
        "synth: {} ({}), {}, {} notes, at most {} voices ({} set), {} notes dropped, {} resets",
        synth.engine().name(),
        synth.preset_name(),
        time.describe(),
        c.notes,
        c.peak_voices,
        synth.polyphony(),
        c.dropped,
        c.resets
    )
}

/// The log line for the pads moving to the synth, by an octave or to
/// another layout: "pads: Poly synth, C3 to D#4 (pad 1 = C3; …)", with the
/// scale and layout when a scale is on.
fn synth_pads_note(synth: &synth::Synth) -> String {
    if synth.engine() == synth::Engine::Drums {
        return format!(
            "pads: Drums, {}, {} (pad 1 = {}; Pad 1 note on the Synth tab's Kit page)",
            synth.preset_name(),
            synth.range_text(),
            synth
                .root(0)
                .map_or("no note".to_string(), synth::note_name)
        );
    }
    let setup = synth.setup();
    let layout = match setup.scale {
        Some(s) => format!(
            "{}, {} (bank {}), ",
            synth::scale_name(&s),
            setup.layout.label(),
            app::bank_letter(synth.settings().bank)
        ),
        None => String::new(),
    };
    let pad1 = synth
        .root(0)
        .map_or("no note".to_string(), synth::note_name);
    let tuning = if synth.settings().tuning == synth::TuningPick::default() {
        String::new()
    } else {
        format!(", tuned {}", synth.tuning_label())
    };
    format!(
        "pads: {} synth, {layout}{}{tuning} (pad 1 = {pad1}; SHIFT + wheel on Play: octave)",
        synth.engine().name(),
        synth.range_text()
    )
}

/// A star or an add in the effect picker changed its favourites or
/// recents: both files are saved whole, on the store's writer thread.
fn save_shelf(mix: &mut mixing::MixHost, store: &mpfree_store::Store, out: &mut Out) {
    if !std::mem::take(&mut mix.shelf_dirty) {
        return;
    }
    use mpfree_fx::picker as fxp;
    for (name, text) in [
        (fxp::FAVOURITES, mix.shelf.favourites_text()),
        (fxp::RECENT, mix.shelf.recent_text()),
    ] {
        if let Err(e) = store.write(fxp::OWNER, name, text.into_bytes()) {
            out.note(&format!("fx: {name} not saved: {e}"));
        }
    }
}

fn player_session(
    o: &Options,
    out: &mut Out,
    timed: bool,
    from_ui: &Receiver<gui::FromUi>,
) -> Result<(), String> {
    // 1. Kits.
    progress(out, 1, "Loading drum kits", "Reading the kits file", 0.02);
    let mut kits = load_kits(&o.kits, &mut |done, total| {
        let value = 0.02 + 0.38 * done as f32 / total.max(1) as f32;
        progress(
            out,
            1,
            "Loading drum kits",
            &format!("{:.1} of {:.1} MB", done as f64 / 1e6, total as f64 / 1e6),
            value,
        );
    })?;
    // The sampler engine plays them (and adds the Grains kit when there is room).
    progress(
        out,
        1,
        "Loading drum kits",
        "Handing the sounds to the sampler",
        0.4,
    );
    let mut player = sampler::Player::new(&mut kits, &mpc_one::PAD_NOTES, RATE)?;
    let sounds: usize = kits.iter().map(|k| k.sounds.len()).sum();
    out.say(&format!("pad player: {} kits, {sounds} sounds", kits.len()));
    for (k, kit) in kits.iter().enumerate() {
        out.say(&format!(
            "kit {}: {} ({} sounds)",
            k + 1,
            kit.name,
            kit.sounds.len()
        ));
    }
    // The synth: one more entry after the kits, and the Synth tab.
    // The synth sits in a slot that can hand it to its own lane (Prefs
    // "Sound cores": Cores 3, 2 and 1); between periods it is always here.
    let mut synth = cores::SynthSlot::new(synth::Synth::new(RATE));
    out.say(&format!("synth: {}, preset {}, as kit list entry {} ({}); the Synth tab edits it and picks Poly or FM",
                     synth.engine().name(), synth.preset_name(), kits.len() + 1, synth.range_text()));
    out.show(gui::ToUi::Kits(kit_views(&kits, &synth)));
    out.show(gui::ToUi::Synth(synth.state()));
    // The Sample tab edits the kits' projects on the screen thread.
    out.show(gui::ToUi::SampleKits {
        projects: player.take_projects(),
        rate: RATE,
    });
    let drum_kits = kits.len();
    let synth_kit = drum_kits;
    let n = drum_kits + 1;
    let mut synth_on = false;
    // Each track's own instrument's preset, as last picked (instruments.rs).
    let mut track_presets: [Option<usize>; instruments::TRACKS] = [None; instruments::TRACKS];
    let mut param_log = synth::ParamLog::default();
    // UNDO on the Synth tab and the mixer: their knob and fader turns.
    let mut synth_turns: undo::Turns<u16> = undo::Turns::default();
    let mut mix_turns: undo::Turns<(mpfree_fx::StripId, u16)> = undo::Turns::default();
    // What the screen was last told about them, for the UNDO light.
    let mut turns_told = (false, false);
    let mut synth_time = RenderTime::default();
    // Q-Links on the Public port (no Akai session) send positions; the Synth
    // page wants turns, so the last position of each is kept.
    let mut qlink_at: [Option<u8>; 4] = [None; 4];
    let mut synth_meters = synth.meters();
    // What each track plays, as the screen last heard it (the line under
    // its name on Arrange).
    let mut sounds_shown: gui::TrackSounds = [None; instruments::TRACKS];
    // Each sounding voice's pressure, for the Synth tab's Pressure page.
    let mut synth_pressures = [0f32; 16];
    let mut shown_pressures = 0usize;
    // The Sample tab: a kit view the screen had no room to take back yet
    // (kept here, never freed on this thread), how many views were swapped
    // in, its playheads, and whether a view changed the playing kit's pads.
    let mut view_back: Option<gui::ToUi> = None;
    // The browser's preview playing, and one going back to be freed.
    let mut preview: Option<Box<preview::Preview>> = None;
    let mut preview_back: Option<Box<preview::Preview>> = None;
    let mut views_swapped = 0u64;
    let mut recolour = false;
    let (mut shown_heads, mut heads_at) = (sampler::Playheads::default(), Instant::now());
    // The Grain tab's cloud box, while the audio thread holds it: filled and
    // sent back with the playheads (never freed on this thread).
    let mut cloud: Option<Box<sampler::Cloud>> = None;
    // The sampler's line input, while it is on; and one handed back, on
    // its way to the screen thread to be freed there.
    let mut rec_feed: Option<Box<sampler::RecFeed>> = None;
    let mut rec_back: Option<Box<sampler::RecFeed>> = None;
    // What the sampler's input records from (the Rec tab's Source).
    let mut rec_source = sampler::RecSource::Line;
    // 2. The controller (USB; it may still be starting at power-on).
    let wait = if timed { Duration::ZERO } else { DEVICE_WAIT };
    progress(
        out,
        2,
        "Finding the pads and buttons",
        "The controller is a USB device inside the MPC",
        0.42,
    );
    let card = wait_for(
        out,
        2,
        "Finding the pads and buttons",
        0.42,
        wait,
        midi_card,
    )?;
    out.say(&format!("controller MIDI card: {card}"));
    // 3. The sound card (its driver loads at power-on).
    progress(
        out,
        3,
        "Finding the sound card",
        "ACVA, the MPC's own outputs",
        0.5,
    );
    let audio = wait_for(out, 3, "Finding the sound card", 0.5, wait, audio_card)?;
    out.say(&format!("sound card: {audio} ({AUDIO_CARD_ID})"));
    // 4. Akai's session, then 5. its setup.
    let mut ports = Ports::open(out, true)?;
    ports.max_shown = u64::MAX;
    let mut rest = kit_pad_colours(&kits[0], 0);
    progress(
        out,
        4,
        "Connecting to the pads and buttons",
        "Asking the controller for a session",
        0.58,
    );
    let session = start_session(out, &mut ports)?;
    if session {
        progress(
            out,
            5,
            "Lighting the pads",
            "Sending Akai's setup, as Akai's app does",
            0.72,
        );
        send_setup(out, &mut ports, &rest, "kit 1's pad colours")?;
        out.show(gui::ToUi::Session("Akai session".into(), Tone::Good));
    } else {
        out.say("no session within 20 s: playing without Akai's setup (controls on the Public port, Q-Links absolute); \
                 no setup message was sent");
        ports.send_batch(
            out,
            false,
            "kit 1's pad colours",
            &as_slices(&pad_colours(&rest)),
        )?;
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
    // The themes folder: theme files the user put in files/themes/ (read
    // only), offered in Prefs > Appearance after the built-in themes. Listed
    // before restore so a saved theme file comes back.
    let (theme_files, notes) = load_theme_files(&store);
    for line in notes {
        out.say(&line);
    }
    app.theme_files = theme_files
        .iter()
        .map(|(f, t)| (f.clone(), t.name.clone()))
        .collect();
    out.show(gui::ToUi::ThemeFiles(theme_files));
    // The library in the data folder (its places are made there if they
    // are missing), for the Pads tab's browser, with its Recent and
    // Favourites from the store. Without --data the browser says so.
    if let (Some(dir), true) = (&o.data, store.saves()) {
        let lists = store
            .read(app::saved::BROWSER, app::saved::BROWSER_LISTS)
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .unwrap_or_default();
        out.show(gui::ToUi::Library {
            dir: dir.clone(),
            lists,
        });
    }
    // The tunings folder: Scala files the user put in files/tunings/ (read
    // only), offered on the Synth tab after the built-in tunings. The scales
    // are read and the keyboard maps worked out here, before the sound
    // starts.
    // CLAP plugins in files/plugins/ (plugins.rs), each listed by its own
    // plugin process so a bad file can't take the app with it. Done here,
    // before the sound starts, since a file may take a while to answer.
    // Each process lives in files/plugin-home/ (made by the store), so what
    // a plugin keeps for itself goes on the data partition.
    let (found_plugins, notes) = plugins::scan(
        plugins::program(&plugins::app_path()).with_home(&plugins::home(&store)),
        o.data.as_deref(),
    );
    for line in notes {
        out.say(&line);
    }
    let found_plugins = std::sync::Arc::new(found_plugins);
    let (tuning_files, notes) = load_tuning_files(&store);
    for line in notes {
        out.say(&line);
    }
    synth.set_tuning_files(tuning_files);
    // The menu's list is the synth's, so a pick always names its file.
    app.tuning_files = synth
        .tuning_files()
        .iter()
        .map(|t| (t.name.clone(), t.mapped()))
        .collect();
    for line in app.restore(
        &store.settings(app::saved::APP),
        &store.settings(app::saved::MAP),
    ) {
        out.say(&line);
    }
    // Touch tuning (the Dev tab's Touch page), kept as the "touch" settings.
    let touch = store.settings(app::saved::TOUCH);
    if !touch.is_empty() {
        let t = mpfree_touch::tuning::Tuning::from_lookup(|k| touch.get_f64(k));
        out.show(gui::ToUi::TouchTuning(t));
    }
    // --theme wins over the saved theme for this run; otherwise the screen
    // switches to the saved one (Prefs > Appearance).
    if let Some(t) = &o.theme {
        app.settings.theme = t.clone();
    }
    out.show(gui::ToUi::Settings(app.settings.clone()));
    out.say(&match (&o.data, store.saves()) {
        (None, _) => {
            "saving: off (no --data); settings and found buttons reset at every start".to_string()
        }
        (Some(d), true) => format!(
            "saving: settings and found buttons are kept in {}",
            d.display()
        ),
        (Some(d), false) => format!(
            "saving: off; {} cannot be used (see the storage line above)",
            d.display()
        ),
    });
    // The time engine (transport, clips, takes) runs in this loop too.
    let mut time = time::TimeHost::new(RATE, app.settings.bpm)?;
    // The looper follows the transport; the line input stays closed until
    // the Prefs row "Line input" opens it. Its memory (Prefs "Loop memory",
    // as saved) is set aside once, here, and locked in RAM with the rest.
    let loop_memory = app.settings.loop_memory;
    let mut looper = looping::LoopHost::new(RATE, loop_memory.seconds());
    app.set_loop_memory_now(loop_memory);
    out.say(&format!(
        "looper: {} min of loop memory ({} MB, Prefs > Loop memory)",
        loop_memory.minutes(),
        loop_memory.megabytes(RATE)
    ));
    let mut loop_time = RenderTime::default();
    let mut loop_shown_at = Instant::now();
    // The audio tracks on the Song view: their memory is set aside here too.
    let audio_memory = app.settings.audio_memory;
    let mut tracks = audiotracks::TrackHost::new(RATE, audio_memory.seconds());
    app.set_audio_memory_now(audio_memory);
    out.say(&format!(
        "audio tracks: {} of stereo memory for takes ({} MB, Prefs > Audio track memory)",
        audio_memory.time(),
        audio_memory.megabytes(RATE)
    ));
    out.show(gui::ToUi::Time(Box::new(time_state(&time, &tracks))));
    let (mut shown_transport, mut transport_shown_at) = (time.info(), Instant::now());
    let mut periods = app.settings.periods;
    // Where the sound runs (Prefs "Sound cores" as saved, or --sound-cores
    // for this run; cores.rs): this thread is the audio thread, and the
    // sampler's lane starts before the sound does.
    let sound_cores = o.sound_cores.unwrap_or(app.settings.sound_cores);
    let mut cores = cores::Cores::start(sound_cores, &mut player, &mut synth);
    app.set_sound_cores_now(sound_cores);
    let mut play = Playback::start(open_acva(out, periods)?, periods)?;
    progress(out, 6, "Ready", "Have fun", 1.0);
    let max_db = o.gain_db;
    let mut level_db = (max_db - 6.0).max(LEVEL_MIN_DB);
    out.show(gui::ToUi::Level { db: level_db });
    // The mixer sums everything; the Level is its master fader (mixing.rs).
    let mut mix = mixing::MixHost::new(RATE, level_db, max_db);
    mix.set_plugins(std::sync::Arc::clone(&found_plugins));
    // The effect picker's favourites and recents (files/fx/).
    let shelf_file = |name| {
        store
            .read(mpfree_fx::picker::OWNER, name)
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .unwrap_or_default()
    };
    mix.shelf = mpfree_fx::picker::Shelf::read(
        &shelf_file(mpfree_fx::picker::FAVOURITES),
        &shelf_file(mpfree_fx::picker::RECENT),
    );
    // The screen opens projects' plugins with the same rack.
    out.show(gui::ToUi::Plugins(std::sync::Arc::clone(&found_plugins)));
    // The plugin synths, for the Synth tab's Instrument row after the engines.
    app.plugin_synths = found_plugins
        .instruments()
        .map(|e| (e.node_id, e.name))
        .collect();
    cores.start_mixer(&mut mix.mixer);
    for line in cores.take_lines() {
        out.say(&line);
    }
    // The chain editor and the macros panel (layout spec 4b): the mixer's
    // insert row opens the editor; a long press on a macro card the panel.
    // The macros' values go out once for the Q-Link column's macro banks.
    let mut panels = modpanels::Panels::new();
    // Projects (project.rs): the parts New project goes back to, sent to
    // the screen once the mixer's start-up inserts are in; a save asked
    // for (SHIFT + BROWSE, or the browser's Save), done after the period's
    // effects; and the engines an open replaced, on their way back to the
    // screen thread to be freed there.
    let mut defaults_sent = false;
    let mut save_asked = false;
    let mut project_back: Option<gui::ToUi> = None;
    // A save: the Looper's loops and the audio tracks' takes being read,
    // what of them is read already, whether the parts are to be collected
    // (once both are), and the parts waiting for room on the way to the
    // screen.
    let mut loops_saving = false;
    let mut audio_saving = false;
    let mut loops_held: Option<mpfree_looper::save::Saved> = None;
    let mut audio_held: Option<mpfree_audiotracks::save::Saved> = None;
    let mut collect_now = false;
    let mut parts_out: Option<gui::ToUi> = None;
    // MIDI Learn: values for parameters a control can't set yet, and the
    // I/O view's names taken at this generation of the model.
    let mut learn_misses = 0u64;
    let mut learn_names_gen: Option<u64> = None;
    // A followed clock's tempo for the app, taken at the next turn.
    let mut followed_tempo: Option<f64> = None;
    // The port the tempo chip last said the tempo comes from.
    let mut clock_from: Option<String> = None;
    out.show(gui::ToUi::Macros(Box::new(panels.macros_state(&mix))));
    let mut shown_levels = mix.play_levels();
    out.show(gui::ToUi::PlayLevels(shown_levels));
    let mut meter_at = Instant::now();
    out.say(&format!("mixer: the kit's 16 pads on pad channels into track 1 (drums), synth and loop on tracks 2-3, the \
                      line input on input 1 (heard with the looper's Monitor), Master safety {} at {:.1} dBFS",
                     "safe limiter", -0.3));
    let limit = if timed {
        format!("for up to {} s", o.seconds)
    } else {
        "until the power goes off".to_string()
    };
    out.say(&format!("playing {limit}: pads or the screen play; the wheel or a tap picks the kit or the synth; Q-Link 1 \
                      or the slider sets the level (now {level_db:.1} dB, at most {max_db:.1} dB), Q-Links 1-4 the \
                      Synth tab's marked knobs; {}",
                     if timed { "STOP ends" } else { "STOP silences" }));
    ports.quiet = true;
    // The control side (control.rs): from here the controller's lights,
    // the report's lines and the chain editor's effects are written and
    // built by threads of their own, never in the audio loop's period.
    if let Err(e) = ports.queue() {
        out.note(&format!(
            "controller output: written by the audio thread ({e})"
        ));
    }
    if let Err(e) = mix.build_off_thread() {
        out.note(&format!(
            "chain editor: effects built on the audio thread (the builder thread: {e})"
        ));
    }
    out.queue_report();
    let (mut lights_dropped, mut lights_noted) = (0u64, Instant::now());
    let mut fx = app.start();
    // The arp's own patterns: --patterns FILE (read-only) for this start, or
    // else the ones Save kept in the store (arp/patterns.txt). Save in the
    // pattern workspace writes them back to the store when it saves.
    match &o.patterns {
        Some(path) => match read_patterns(path) {
            Ok(text) => fx.extend(app.load_patterns(&text)),
            Err(e) => out.note(&format!(
                "arp patterns file {}: {e}; the built-in patterns stay",
                path.display()
            )),
        },
        None => match store
            .read(app::saved::ARP, app::saved::ARP_PATTERNS)
            .map(String::from_utf8)
        {
            Some(Ok(text)) => fx.extend(app.load_patterns(&text)),
            Some(Err(_)) => {
                out.note("arp patterns: the saved file is not text; the built-in patterns stay")
            }
            None => {}
        },
    }
    app.pattern_file = store.saves();
    // The Synth tab's options are the project's (project.rs): they start at
    // their defaults, and an open puts back the project's.
    let (mut left, mut right) = ([0f32; PERIOD], [0f32; PERIOD]);
    // Each engine's period, for its track, and the pads' sound for the looper.
    let (mut drums_l, mut drums_r) = ([0f32; PERIOD], [0f32; PERIOD]);
    let (mut synth_l, mut synth_r) = ([0f32; PERIOD], [0f32; PERIOD]);
    let (mut loop_l, mut loop_r) = ([0f32; PERIOD], [0f32; PERIOD]);
    let (mut audio_l, mut audio_r) = ([0f32; PERIOD], [0f32; PERIOD]);
    let (mut pads_l, mut pads_r) = ([0f32; PERIOD], [0f32; PERIOD]);
    let mut frames = [0i32; PERIOD * 2];
    let mut buf = [0u8; 256];
    let total_periods = if timed {
        o.seconds * RATE as u64 / PERIOD as u64
    } else {
        u64::MAX
    };
    let (mut hits, mut screen_hits, mut silent_hits, mut clipped, mut repeats) =
        (0u64, 0u64, 0u64, 0u64, 0u64);
    let (mut peak, mut window_peak, mut voices) = (0f32, 0f32, 0usize);
    let mut used = vec![false; n];
    used[0] = true;
    let mut lat = Latency::new();
    let mut pending: Vec<Instant> = Vec::with_capacity(32);
    let mut delay_error = false;
    let (mut pings, mut last_ping, mut last_reply, mut lost) =
        (0u64, Instant::now(), Instant::now(), false);
    let mut last_timing = Instant::now();
    // Long jobs refused idle priority, as last logged.
    let mut idle_noted = 0;
    let mut last_status = Instant::now();
    // The Q-Links' touch sensors chatter: a release counts after 150 ms off.
    let mut touch_hold = qlink_touch::TouchHold::default();
    let started = Instant::now();
    let mut stopped_by = "time";
    // Pads held (on the controller or the screen), pressed but not yet sounded
    // (Note Repeat starts a pad on the next step of its grid), and lit.
    let mut held = [false; 16];
    let mut awaiting = [false; 16];
    let mut lit = [false; 16];
    // TRACK MUTE: pads pressed as mute keys (their release plays nothing),
    // and the colours the mute keys show (None: the pads show as usual).
    let mut mute_pressed = [false; 16];
    let mut mute_shown: Option<[[u8; 3]; 16]> = None;
    // Buttons whose light is never sent, noted once each.
    let mut unlit = [false; 128];
    // Repeats per second, for the log (the paced Latch check reads these).
    let (mut window_repeats, mut window_pads, mut last_repeat_note) =
        (0u64, [false; 16], Instant::now());
    let mut totals = AudioTotals::default();
    let mut render_time = RenderTime::default();
    // CPU by engine: a probe per stage, published every 250 ms for the Dev
    // tab's page (engines.rs). Made here, before the loop: nothing in the
    // loop allocates for it.
    let mut engines = engines::EngineProbes::new();
    out.show(gui::ToUi::Engines(engines.shared()));
    let engine_shared = engines.shared();
    // The period's parts outside the engines, timed so a slow period says
    // where its time went (parts.rs).
    let mut parts = parts::Parts::new(engines.shared());
    cores.show(&engine_shared.cores);
    // MIDI from outside (midi_in.rs): the 5-pin MIDI In (cable 2, already
    // open with the controller's other cables) and USB keyboards, read only,
    // as keys for the armed track.
    let mut music = midi_in::MusicIn::new();
    // What the mixer's I/O view shows and edits (MIDI, Sync and CV). The
    // loop only ever try_locks it (MusicIn::sync).
    let io_model = midi_in::io_model();
    // The I/O view's rig as it was left (midi_saved.rs): each port's input
    // and thru, for every project. The song's MIDI settings come with the
    // project (its `midi` part).
    let rig_text = store
        .read(midi_saved::OWNER, midi_saved::RIG)
        .map(|b| String::from_utf8_lossy(&b).into_owned());
    for w in midi_saved::restore(&io_model, rig_text.as_deref()) {
        out.note(&format!("midi: setting not read: {w}"));
    }
    let mut midi_saver = midi_saved::Saver::new(&io_model);
    out.show(gui::ToUi::IoModel(io_model.clone()));
    // MIDI Out (midi_out.rs): tracks whose "Plays on" is the 5-pin jacks play
    // there. State-changing on the MPC (note bytes on cable 2, never SysEx),
    // so it opens only with --midi-out until a device round has checked it.
    let mut midi_out = midi_out::MidiOut::new(RATE, MIDI_OUT_LEAD);
    // Export (export.rs): a pass the screen sends plays the song into its
    // files; its MIDI file hears the notes as MIDI Out would send them.
    let mut export = export::Host::new();
    if o.midi_out {
        match midi_card().and_then(|card| open_output(card, MUSIC_CABLE, "MIDI Port", out)) {
            Ok(port) => match midi_out.open(port, started) {
                Ok(()) => out.midi(&format!(
                    "MIDI Out open: tracks set to play on MIDI play there, {:.1} ms after \
                     their frame is made (the sound's lead)",
                    MIDI_OUT_LEAD.as_secs_f64() * 1000.0
                )),
                Err(e) => out.midi(&format!("MIDI Out not started: {e}")),
            },
            Err(e) => out.midi(&format!(
                "MIDI Out not opened: {e}; tracks set to play on MIDI play on the pads"
            )),
        }
    }
    // CV out (cv_out.rs): CV 1 to 4 follow the project's CV outputs.
    // State-changing on the MPC (control changes on cable 1, never SysEx),
    // so it opens only with --cv-out until a device round with a meter has
    // checked it. Its writer has a copy of the Private port, and the
    // button lights go through it while it is open.
    let mut cv_out = cv_out::CvOut::new(RATE, MIDI_OUT_LEAD);
    if o.cv_out {
        match ports.private.as_ref().map(File::try_clone) {
            Some(Ok(port)) => match cv_out.open(port, started) {
                Ok(()) => out.midi(&format!(
                    "CV out open: CV 1 to {} follow the project's CV outputs, {:.1} ms after \
                     their frame is made; CV {} to 8 wait for a check",
                    cv_out::CHECKED_OUTS,
                    MIDI_OUT_LEAD.as_secs_f64() * 1000.0,
                    cv_out::CHECKED_OUTS + 1,
                )),
                Err(e) => out.midi(&format!("CV out not started: {e}")),
            },
            Some(Err(e)) => out.midi(&format!("CV out not opened: {e}")),
            None => out.midi("CV out not opened: the Private port is not open"),
        }
    }
    let mut usb = UsbKeys::start(
        [Some(audio), midi_card().ok()]
            .into_iter()
            .flatten()
            .collect(),
    );
    let mut keys: Vec<midi_in::Key> = Vec::with_capacity(64);
    let mut key_misses = 0u64;
    // The lanes' deadline for this period (cores.rs): when the sound card's
    // queue would run dry, less a period. The buffer starts full.
    let mut lane_until = cores::lane_deadline(Instant::now(), None, periods);
    let mut i = 0u64;
    while i < total_periods {
        let t_read = Instant::now();
        let period_span = Span::start();
        parts.begin();
        let t = started.elapsed().as_secs_f64();
        let mut kit_to: Option<usize> = None;
        let mut audio_change: Option<u32> = None;
        let mut bank_lights = false;
        // Hits are timed only when they sound at once (not on the arp's grid).
        let direct = !app.generating();
        while let Ok(a) = from_ui.try_recv() {
            let (t_item, item) = (Instant::now(), parts::Name::of(&a));
            let mix_turn = match &a {
                gui::FromUi::Mixer(mpfree_mixer_ui::MixAction::Set { strip, id, value }) => {
                    Some((*strip, *id, mix.value(*strip, *id), *value))
                }
                _ => None,
            };
            // The app, the mixer, the time engine, the looper and the chain
            // editor take theirs (desk.rs); the rest is this loop's.
            let mut desk = desk::Desk {
                app: &mut app,
                mix: &mut mix,
                time: &mut time,
                panels: &mut panels,
                looper: &mut looper,
                tracks: &mut tracks,
                music: &mut music,
                level_db: &mut level_db,
                max_db,
                card: audio,
            };
            let a = desk.screen(a, t, out, &mut fx);
            save_shelf(&mut mix, &store, out);
            // UNDO on the mixer: a strip control's turn, from its value
            // before the desk set it.
            if let Some((strip, id, from, value)) = mix_turn {
                let to =
                    if (strip, id) == (mpfree_fx::StripId::Master, mpfree_fx::ids::mixer::VOLUME) {
                        level_db
                    } else {
                        value
                    };
                mix_turns.turned((strip, id), from, to, t_read);
            }
            let Some(a) = a else {
                parts.item_named(parts::SCREEN_ACTIONS, t_item, &item);
                continue;
            };
            match a {
                gui::FromUi::Pad { pad, down: true }
                    if pad < 16 && app.mute_keys() != app::MuteKeys::Off =>
                {
                    mute_pressed[pad] = true;
                    out.note(&mute_key(
                        app.mute_keys(),
                        app.mute_bank(),
                        pad,
                        &mut time,
                        &mut mix,
                    ));
                }
                gui::FromUi::Pad { pad, down: false } if pad < 16 && mute_pressed[pad] => {
                    mute_pressed[pad] = false;
                    out.show(gui::ToUi::PadUp(pad));
                }
                gui::FromUi::Pad { pad, down: true } if pad < 16 => {
                    let v = app.settings.screen_velocity;
                    if let Some(ev) = app.pressed_event(pad, v, true) {
                        time.pad_pressed(0, ev);
                    }
                    fx.extend(app.pad_down(pad, v));
                    held[pad] = true;
                    awaiting[pad] = true;
                    screen_hits += 1;
                    if direct {
                        pending.push(t_read);
                    }
                }
                gui::FromUi::Pad { pad, down: false } if pad < 16 => {
                    if let Some(ev) = app.pressed_event(pad, 0, false) {
                        time.pad_pressed(0, ev);
                    }
                    fx.extend(app.pad_up(pad));
                    held[pad] = false;
                    awaiting[pad] = false;
                    out.show(gui::ToUi::PadUp(pad));
                }
                gui::FromUi::Pad { .. } => {}
                gui::FromUi::Kit(k) => kit_to = Some(k.min(n - 1)),
                // UNDO on the Synth tab and the mixer: their turns' history.
                gui::FromUi::UndoHistory(on) => {
                    let (rows, now) = match on {
                        gui::UndoOn::Synth => synth_turns
                            .rows("Start: the sound before these turns", |t| {
                                synth_turn_row(&synth, t.key, t.to)
                            }),
                        gui::UndoOn::Mixer => mix_turns
                            .rows("Start: the mix before these turns", |t| {
                                mix_turn_row(t.key.0, t.key.1, t.to)
                            }),
                    };
                    out.show(gui::ToUi::UndoHistory { on, rows, now });
                }
                gui::FromUi::UndoOn { on, redo } => {
                    let line = match on {
                        gui::UndoOn::Synth => {
                            let turn = if redo {
                                synth_turns.redo()
                            } else {
                                synth_turns.undo()
                            };
                            turn.map(|t| {
                                let v = if redo { t.to } else { t.from };
                                synth.set(t.key, v);
                                out.show(gui::ToUi::SynthValues(synth.values()));
                                synth_turn_said(&synth, t.key, redo)
                            })
                        }
                        gui::UndoOn::Mixer => {
                            let turn = if redo {
                                mix_turns.redo()
                            } else {
                                mix_turns.undo()
                            };
                            turn.map(|t| {
                                let (strip, id) = t.key;
                                let value = if redo { t.to } else { t.from };
                                if let Some(db) = mix.action(&mpfree_mixer_ui::MixAction::Set {
                                    strip,
                                    id,
                                    value,
                                }) {
                                    level_db = db;
                                    out.show(gui::ToUi::Level { db: level_db });
                                }
                                mix_turn_said(strip, id, redo)
                            })
                        }
                    };
                    let line = line.unwrap_or_else(|| {
                        format!(
                            "Nothing to {} on this page",
                            if redo { "redo" } else { "undo" }
                        )
                    });
                    fx.push(app::Effect::Tell(line));
                }
                // A tap on a track's sound line on Arrange: the track is
                // selected (armed, as the Record panel's Armed row) and the
                // Synth tab's MENU opens on its Instrument row.
                gui::FromUi::TrackInstrument(track) if track < time.project().tracks.len() => {
                    if time.armed() != Some(track) {
                        fx.push(app::Effect::Time(TimeAction::Arm(Some(track))));
                    }
                    fx.extend(app.open_track_instrument(track));
                    out.show(gui::ToUi::Synth(synth_tab(
                        &synth,
                        &mix.instruments,
                        track,
                        track_presets.get(track).copied().flatten(),
                    )));
                }
                gui::FromUi::TrackInstrument(track) => {
                    out.note(&format!("arrange: no track {}", track + 1));
                }
                gui::FromUi::SaveTouch(t) => {
                    let mut settings = mpfree_store::Settings::default();
                    for (k, v) in t.to_pairs() {
                        settings.set_str(k, &v);
                    }
                    if !store.saves() {
                        out.note("touch tuning: not saved (no data folder); it stays until the power goes off");
                    } else if let Err(e) = store.save_settings(app::saved::TOUCH, &settings) {
                        out.note(&format!("not saved: {}: {e}", app::saved::TOUCH));
                    }
                }
                // The selected track's own synth, when it has one; else the
                // shared synth.
                gui::FromUi::Synth(SynthAction::Set { id, value })
                    if mix.instruments.holds(app.track()) =>
                {
                    mix.instruments.param(app.track(), id, value);
                }
                gui::FromUi::Synth(SynthAction::Set { id, value }) => {
                    let from = synth.value(id);
                    synth.set(id, value);
                    synth_turns.turned(id, from, synth.value(id), t_read);
                    if let Some(line) = param_log.set(&synth, id, value, t_read) {
                        out.note(&line);
                    }
                }
                gui::FromUi::Slice {
                    kit,
                    pad,
                    layer,
                    start,
                    end,
                } => {
                    player.audition(kit, pad, layer, start, end, app.settings.screen_velocity);
                }
                gui::FromUi::SampleLiveChop {
                    kit,
                    pad,
                    layer,
                    start,
                    end,
                    on,
                } => {
                    let v = app.settings.screen_velocity;
                    player.live_chop(kit, pad, layer, start, end, on, v);
                }
                gui::FromUi::SampleHit {
                    kit,
                    pad,
                    velocity,
                    down,
                } if kit == player.kit() => {
                    if down {
                        player.hit_at(pad, velocity, 0);
                    } else {
                        player.release_at(pad, 0);
                    }
                }
                gui::FromUi::SampleHit { .. } => {}
                gui::FromUi::SampleKey {
                    kit,
                    note,
                    velocity,
                    down,
                } if kit == player.kit() => {
                    if down {
                        player.key_at(note, velocity, 0);
                    } else {
                        player.key_off_at(note, 0);
                    }
                }
                gui::FromUi::SampleKey { .. } => {}
                gui::FromUi::SamplePadLive { kit, values } => player.pad_live(kit, &values),
                // The browser's preview: a new sound replaces the one
                // playing, which goes back to be freed.
                gui::FromUi::Preview(p) => {
                    if let Some(old) = preview.replace(p) {
                        if preview_back.replace(old).is_some() {
                            out.note(
                                "preview: two sounds going back; one freed on the audio thread",
                            );
                        }
                    }
                }
                gui::FromUi::PreviewStop => {
                    if let Some(p) = preview.as_mut() {
                        p.stop();
                    }
                }
                gui::FromUi::SaveProject => save_asked = true,
                gui::FromUi::ExportAsk => {
                    let f = export_facts(&time, &mix, &midi_out, &music, &cv_out, &app, &looper);
                    out.show(gui::ToUi::ExportFacts(Box::new(f)));
                }
                gui::FromUi::Export(run) => match export.begin(run, &mut mix.mixer, &mut time) {
                    Ok(()) => out.note("export: the pass started"),
                    Err(run) => out.show(gui::ToUi::ExportOver(run)),
                },
                // The loops go in once the old ones have faded out.
                gui::FromUi::OpenLoops(l) => looper.open(*l),
                // The audio tracks' part goes in once they are empty.
                gui::FromUi::OpenAudio(a) => tracks.open(*a),
                gui::FromUi::OpenProject(mut o) => {
                    // Everything stops first: the clips change under the
                    // transport, and held notes would hang.
                    fx.push(app::Effect::Time(TimeAction::Transport(Command::Stop)));
                    fx.push(app::Effect::StopSounds);
                    // A save under way stops: the project changes under it.
                    if collect_now {
                        looper.cancel_save();
                        tracks.host.cancel_save();
                        if let Some(l) = loops_held.take() {
                            looper.drop_saved(l);
                        }
                        if let Some(a) = audio_held.take() {
                            tracks.host.drop_saved(a);
                        }
                        (loops_saving, audio_saving, collect_now) = (false, false, false);
                        out.note("project: the save stopped, a project is opening");
                    }
                    if let Some(bpm) = o.tempo {
                        fx.extend(app.set_tempo(bpm));
                    }
                    if let Some(mut t) = o.time.take() {
                        // The metronome is the system's: it stays as it is.
                        (t.metronome, t.click_gain) = (time.metronome, time.click_gain);
                        std::mem::swap(&mut time, &mut *t);
                        // The old engine goes back in the box, to be freed
                        // on the screen thread.
                        o.time = Some(t);
                        out.show(gui::ToUi::Time(Box::new(time.state())));
                    }
                    if let Some(s) = o.swing {
                        fx.push(app::Effect::Time(TimeAction::Transport(Command::SetSwing(
                            s,
                        ))));
                    }
                    if let Some(m) = o.mix.take() {
                        mix.open(m);
                    }
                    // Each track's own instrument: the project's (made on
                    // the screen thread), or none, so the track plays the
                    // kit. The old ones ring out. First the ones the
                    // project hasn't got come out, so their places are free
                    // for the project's. One the host can't take goes back
                    // in the box, to be freed on the screen thread.
                    let mut changed = 0;
                    let mut playing = 0;
                    let loaded_for = |o: &project::Opening, track: usize| {
                        o.instruments
                            .iter()
                            .position(|l| l.as_ref().is_some_and(|l| l.track == track))
                    };
                    for track in 0..instruments::TRACKS {
                        track_presets[track] = None;
                        if loaded_for(&o, track).is_some() || !mix.instruments.holds(track) {
                            continue;
                        }
                        changed += 1;
                        if !mix.instruments.take_out(track) {
                            out.note(&format!(
                                "project: track {} keeps its instrument (its last sound is still ringing out)",
                                track + 1
                            ));
                        }
                        app.set_track_sound(track, mix.instruments.held(track));
                    }
                    for track in 0..instruments::TRACKS {
                        let Some(k) = loaded_for(&o, track) else {
                            continue;
                        };
                        let Some(l) = o.instruments[k].take() else {
                            continue;
                        };
                        changed += 1;
                        let engine = synth::Engine::from_node_id(l.held.id);
                        let pads = synth.pads_for(engine.unwrap_or(synth::Engine::Poly));
                        let held = l.held;
                        let put = (l.instrument, held);
                        match mix.instruments.put(track, put, pads, &synth.tuning_now()) {
                            Ok(()) => playing += 1,
                            Err((why, back)) => {
                                if let Some(instrument) = back {
                                    o.instruments[k] = Some(project::Loaded {
                                        track,
                                        instrument,
                                        held,
                                    });
                                }
                                let why = match why {
                                    instruments::Refused::Full => format!(
                                        "{} tracks already play their own",
                                        instruments::SLOTS
                                    ),
                                    _ => "its last sound is still ringing out".to_string(),
                                };
                                out.note(&format!(
                                    "project: track {}: {} not put in ({why}); it keeps what it had",
                                    track + 1,
                                    held.name
                                ));
                            }
                        }
                        app.set_track_sound(track, mix.instruments.held(track));
                    }
                    if changed > 0 {
                        out.note(&format!(
                            "project: {playing} tracks play their own instrument, the rest the kit"
                        ));
                    }
                    if let Some((options, sounds)) = o.synth.take() {
                        for line in synth.set_sounds(&sounds) {
                            out.note(&line);
                        }
                        let (notes, synth_fx) = app.open_synth(&options);
                        for line in notes {
                            out.note(&format!("project: {line}"));
                        }
                        fx.extend(synth_fx);
                        out.show(gui::ToUi::Synth(synth.state()));
                        // The options and sounds are dropped with the box,
                        // on the screen thread.
                        o.synth = Some((options, sounds));
                    }
                    if changed > 0 {
                        out.show(gui::ToUi::Synth(synth_tab(
                            &synth,
                            &mix.instruments,
                            app.track(),
                            None,
                        )));
                    }
                    // The arps: the project's, or every one at its default.
                    let (notes, arp_fx) = app.open_arp(o.arp.as_deref());
                    for line in notes {
                        out.note(&line);
                    }
                    fx.extend(arp_fx);
                    if o.clear_loops {
                        out.note(&looper.clear_all());
                    }
                    if o.clear_audio {
                        // Recordings end; the tracks empty once they have.
                        tracks.clear();
                    }
                    for line in o.notes.drain(..) {
                        out.note(&line);
                    }
                    out.note(&format!("project: {} is in", o.what));
                    if let Some(old) = project_back.replace(gui::ToUi::ProjectBack(o)) {
                        // Only one open travels at a time; an older one is
                        // freed here (not expected).
                        drop(old);
                        out.note("project: two opens going back; one freed on the audio thread");
                    }
                }
                gui::FromUi::SaveBrowserLists(text) => {
                    if !store.saves() {
                        out.note("browser: Recent and Favourites not kept (no data folder)");
                    } else if let Err(e) = store.write(
                        app::saved::BROWSER,
                        app::saved::BROWSER_LISTS,
                        text.into_bytes(),
                    ) {
                        out.note(&format!("browser: Recent and Favourites not kept ({e})"));
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
                gui::FromUi::SampleInput(feed) => {
                    let old = match feed {
                        Some(f) => rec_feed.replace(f),
                        None => rec_feed.take(),
                    };
                    if let Some(old) = old {
                        // Only one goes back at a time; an older one is
                        // freed here (not expected).
                        if rec_back.replace(old).is_some() {
                            out.note("sample: two line input buffers going back; one freed on the audio thread");
                        }
                    }
                }
                gui::FromUi::SampleSource(src) => {
                    if src != rec_source {
                        rec_source = src;
                        if !mix.set_rec_source(src) {
                            out.note("sample: the mixer has no such strip to resample");
                        }
                        // What comes in from now on is from the new source.
                        if let Some(f) = rec_feed.as_mut() {
                            f.restart();
                        }
                    }
                }
                gui::FromUi::SampleGrains(c) => {
                    // Only one box exists, so this is not expected; the
                    // older one is freed here.
                    if cloud.replace(c).is_some() {
                        out.note("sample: two grain clouds on the audio thread; one freed here");
                    }
                }
                // A preset on a track's own synth: a new copy with the
                // preset's sound goes in, and the old one rings out.
                gui::FromUi::Synth(SynthAction::Preset(k))
                    if mix.instruments.holds(app.track()) =>
                {
                    let track = app.track();
                    let held = mix.instruments.held(track);
                    let engine = held.and_then(|h| synth::Engine::from_node_id(h.id));
                    match (held, engine.and_then(|e| synth.preset_values(e, k))) {
                        (Some(h), Some((name, values))) => {
                            let job = instrument_job(&synth, track, h.id, Some(values));
                            if mix.build_instrument(job) {
                                if let Some(p) = track_presets.get_mut(track) {
                                    *p = Some(k);
                                }
                                out.note(&format!("track {}: preset {name}", track + 1));
                            }
                        }
                        _ => out.note(&format!("track {}: no preset {}", track + 1, k + 1)),
                    }
                }
                gui::FromUi::Synth(SynthAction::Preset(k)) => {
                    if let Some(line) = param_log.flush(&synth) {
                        out.note(&line);
                    }
                    // The turns were of the old sound.
                    synth_turns.clear();
                    if let Some(name) = synth.load_preset(k).map(str::to_string) {
                        out.note(&format!(
                            "synth: preset {} of {}, {name}",
                            k + 1,
                            synth.preset_count()
                        ));
                        out.show(gui::ToUi::SynthValues(synth.values()));
                    }
                }
                // The desk's, taken above.
                _ => {}
            }
            parts.item_named(parts::SCREEN_ACTIONS, t_item, &item);
        }
        // MIDI Learn listens only while the chain editor's learn is armed:
        // Back or closing the editor stops it too.
        if music.armed().is_some() && !panels.learning() {
            music.disarm();
        }
        if let Some(line) = param_log.poll(&synth, t_read) {
            out.note(&line);
        }
        let turns_ready = (!synth_turns.done().is_empty(), !mix_turns.done().is_empty());
        if turns_ready != turns_told {
            turns_told = turns_ready;
            out.show(gui::ToUi::TurnsReady {
                synth: turns_ready.0,
                mixer: turns_ready.1,
            });
        }
        if let Some(b) = preview_back.take() {
            if let Err(gui::ToUi::PreviewBack(b)) = out.try_show(gui::ToUi::PreviewBack(b)) {
                preview_back = Some(b);
            }
        }
        if let Some(m) = view_back.take() {
            view_back = out.try_show(m).err();
        }
        if let Some(m) = project_back.take() {
            project_back = out.try_show(m).err();
        }
        parts.lap(parts::SCREEN_ACTIONS);
        ports.show_pressure = app.settings.show_pressure;
        let mut got = ports.poll(out, &mut buf)?;
        let mt = ports.t();
        got.retain(|m| {
            if m.music {
                music.bytes(midi_in::DIN, &m.bytes, mt, &mut keys, &mut |l| out.midi(&l));
            }
            !m.music
        });
        usb.poll(&mut music, &mut buf, mt, &mut keys, out);
        music.tick(mt, &mut |l| out.midi(&l));
        midi_out.tick(&mut |l| out.midi(&l));
        cv_out.tick(&mut |l| out.midi(&l));
        music.sync(&io_model, mt);
        // MIDI Learn (M5): a control that took the armed knob ends the
        // chain editor's learn; a learned control sets its parameter as a
        // chain editor knob does.
        let recording = time.info().state == mpfree_transport::State::Recording;
        for l in music.learning() {
            match l {
                midi_in::Learning::Took { .. } => {
                    panels.learned();
                    out.show(gui::ToUi::ChainLearnDone);
                }
                midi_in::Learning::Set(w) => {
                    if learned_set(&mut panels, &mut mix, w, recording).is_none() {
                        learn_misses += 1;
                        if learn_misses == 1 {
                            out.midi(&format!(
                                "MIDI Learn: {} isn't a parameter MPFree can set from a \
                                 control yet (counted from here)",
                                w.target
                            ));
                        }
                    }
                }
            }
        }
        // Where each learned control's parameter stands, when something
        // else moved it: an endless encoder steps on from there.
        music.observe_learned(mt, |addr| modmix::describe(&mix.mixer, addr));
        // The I/O view names what each learned control moves, whenever
        // the project's learned controls change.
        if let Ok(mut m) = io_model.try_lock() {
            if learn_names_gen != Some(m.gen) {
                learn_names_gen = Some(m.gen);
                let names = m
                    .setup
                    .learn
                    .iter()
                    .map(|l| modpanels::label(&mix, l.target))
                    .collect();
                m.live.learn_names = names;
            }
        }
        // Keys play the armed track: by note while the pads play notes (a
        // synth pad layout), else as the pad the MPC gives that note.
        let key_notes = synth_on && app.arp_plays_notes();
        for k in keys.drain(..) {
            let pad_event = match k {
                midi_in::Key::Down { note, velocity } if key_notes => {
                    app.key_down(note, velocity);
                    None
                }
                midi_in::Key::Up { note } if key_notes => {
                    app.key_up(note);
                    None
                }
                midi_in::Key::Pressure { note, value } if key_notes => {
                    synth.pressure_note(note, value);
                    None
                }
                midi_in::Key::Down { note, velocity } => {
                    mpc_one::pad_index(note).map(|pad| ControlEvent::PadDown { pad, velocity })
                }
                midi_in::Key::Up { note } => {
                    mpc_one::pad_index(note).map(|pad| ControlEvent::PadUp { pad })
                }
                midi_in::Key::Pressure { note, value } => {
                    mpc_one::pad_index(note).map(|pad| ControlEvent::PadPressure {
                        pad,
                        pressure: value,
                    })
                }
            };
            match pad_event {
                Some(event) => got.push(Incoming {
                    port: MUSIC_CABLE as usize,
                    bytes: Vec::new(),
                    event: Some(event),
                    music: true,
                }),
                None if !key_notes && matches!(k, midi_in::Key::Down { .. }) => {
                    key_misses += 1;
                    if key_misses == 1 {
                        out.midi(&format!(
                            "+{mt:.3} s a key with no pad: with a kit, keys play the pads by the MPC's pad notes ({}); \
                             other notes are counted",
                            pad_note_names()
                        ));
                    }
                }
                None => {}
            }
        }
        for m in got {
            let (t_item, event) = (Instant::now(), m.event);
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
                Some(ControlEvent::PadDown { pad, .. })
                    if (pad as usize) < 16 && app.mute_keys() != app::MuteKeys::Off =>
                {
                    mute_pressed[pad as usize] = true;
                    out.note(&mute_key(
                        app.mute_keys(),
                        app.mute_bank(),
                        pad as usize,
                        &mut time,
                        &mut mix,
                    ));
                }
                Some(ControlEvent::PadUp { pad })
                    if (pad as usize) < 16 && mute_pressed[pad as usize] =>
                {
                    mute_pressed[pad as usize] = false;
                    out.show(gui::ToUi::PadUp(pad as usize));
                }
                Some(ControlEvent::PadDown { pad, velocity }) if (pad as usize) < 16 => {
                    let pad = pad as usize;
                    if let Some(ev) = app.pressed_event(pad, velocity, true) {
                        time.pad_pressed(0, ev);
                    }
                    fx.extend(app.pad_down(pad, velocity));
                    held[pad] = true;
                    awaiting[pad] = true;
                    if direct {
                        pending.push(t_read);
                    }
                }
                Some(ControlEvent::PadUp { pad }) if (pad as usize) < 16 => {
                    if let Some(ev) = app.pressed_event(pad as usize, 0, false) {
                        time.pad_pressed(0, ev);
                    }
                    fx.extend(app.pad_up(pad as usize));
                    held[pad as usize] = false;
                    awaiting[pad as usize] = false;
                    out.show(gui::ToUi::PadUp(pad as usize));
                }
                Some(ControlEvent::PadPressure { pad, pressure }) if (pad as usize) < 16 => {
                    // Recorded onto the pad's note while the armed track
                    // records pressure.
                    time.pad_pressure(0, pad, pressure);
                    app.pressure(pad as usize, pressure);
                    if synth_on && app.arp_plays_notes() {
                        // The notes this pad pressed (by number), even if the
                        // layout moved while it was held.
                        for &note in app.pad_held_notes(pad as usize) {
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
                    // While the mixer is open they move the selected strip; on the chain
                    // editor and the macros panel, the knobs or macros the column shows. In
                    // the browser: scroll, filter, preview level, place.
                    if matches!(
                        page,
                        app::View::Synth
                            | app::View::Sample
                            | app::View::Pads
                            | app::View::Browser
                            | app::View::Mixer
                            | app::View::Looper
                            | app::View::Chain
                            | app::View::Macros
                    ) && knob < 4
                    {
                        let k = knob as usize;
                        let ticks = if public {
                            let was = qlink_at[k].replace(m.bytes[2]);
                            was.map_or(0, |w| m.bytes[2] as i32 - w as i32)
                        } else {
                            delta as i32
                        };
                        // The page's screen decides what each knob turns (desk.rs).
                        if let Some(m) = desk::qlink_to_screen(page, k, ticks) {
                            out.show(m);
                        }
                    } else if let Some(e) =
                        app.qlink(knob as usize, delta as i32, public.then(|| m.bytes[2]))
                    {
                        fx.extend(e);
                    } else if matches!(page, app::View::Play | app::View::Arrange) && knob < 4 {
                        // The Play page decides what each knob turns there (its view, the step picked);
                        // Arrange has the same column and knobs (layout spec section 4c).
                        let k = knob as usize;
                        let ticks = if public {
                            let was = qlink_at[k].replace(m.bytes[2]);
                            was.map_or(0, |w| m.bytes[2] as i32 - w as i32)
                        } else {
                            delta as i32
                        };
                        if let Some(m) = desk::qlink_to_screen(page, k, ticks) {
                            out.show(m);
                        }
                    } else if knob == 0 {
                        level_db =
                            qlink_level(level_db, max_db, MIDI_PORTS[m.port].1, &m.bytes, delta);
                        mix.set_level(level_db);
                        if public {
                            qlink_at[0] = Some(m.bytes[2]);
                        }
                        out.show(gui::ToUi::Level { db: level_db });
                    }
                }
                Some(ControlEvent::Button { note, pressed }) => {
                    fx.extend(app.button(note, pressed, t))
                }
                // A finger on a Q-Link lights its card. The control mapper names the
                // Q-Links. The release waits until the finger has been off for 150 ms
                // (the sensor chatters; qlink_touch.rs), below.
                Some(ControlEvent::QLinkTouch { knob, touched }) if (knob as usize) < 4 => {
                    if touch_hold.event(knob as usize, touched, t) {
                        out.show(gui::ToUi::QLinkTouch(knob as usize, true));
                        fx.extend(app.known_control(app::Code::Button(
                            mpc_one::QLINK_TOUCH_NOTES[knob as usize],
                        )));
                    }
                }
                None if m.bytes.len() == 3 && m.bytes[0] == 0xB0 => {
                    fx.extend(app.unknown_cc(m.bytes[1]))
                }
                _ => {}
            }
            if let Some(ControlEvent::QLinkTurn { knob, .. }) = m.event {
                if let Some(&cc) = mpc_one::QLINK_TURN_CC.get(knob as usize) {
                    fx.extend(app.known_control(app::Code::Cc(cc)));
                }
            }
            parts.item(parts::MIDI_IN, t_item, &event);
        }
        parts.lap(parts::MIDI_IN);
        // Letting go of a Q-Link unlights its card, and ends a turn on the
        // Sample page (one undo step).
        for (k, _) in touch_hold.due(t).iter().enumerate().filter(|(_, &d)| d) {
            out.show(gui::ToUi::QLinkTouch(k, false));
            if app.view().sampler() {
                out.show(gui::ToUi::SampleLift);
            }
        }
        fx.extend(app.tick(t));
        if let Some(bpm) = followed_tempo.take() {
            let bpm = (bpm * 10.0).round() / 10.0;
            // Without the log line set_tempo writes: the tempo follows
            // another device's, and the log would fill.
            fx.extend(
                app.set_tempo(bpm)
                    .into_iter()
                    .filter(|e| !matches!(e, app::Effect::Note(_))),
            );
        }
        let mut stop = false;
        let mut later = Vec::new();
        for e in fx.drain(..) {
            // - and + on the mixer step the I/O view's row; on a strips view
            // they step the kit, as elsewhere.
            let e = match e {
                app::Effect::MixerStep(d) if !io_shown(&mix) => app::Effect::Nav(app::Nav::Step(d)),
                e => e,
            };
            let (t_item, item) = (Instant::now(), parts::Name::of(&e));
            let mut desk = desk::Desk {
                app: &mut app,
                mix: &mut mix,
                time: &mut time,
                panels: &mut panels,
                looper: &mut looper,
                tracks: &mut tracks,
                music: &mut music,
                level_db: &mut level_db,
                max_db,
                card: audio,
            };
            let Some(e) = desk.effect(e, out, &mut later) else {
                parts.item_named(parts::APP, t_item, &item);
                continue;
            };
            match e {
                app::Effect::Light(note, value) if akai::app_lights(note) => {
                    // Through CV out's writer while it shares the port.
                    let light = akai::button_light(note, value);
                    if !cv_out.light(&light) {
                        ports.send(true, &light)?
                    }
                }
                app::Effect::Light(note, _) => {
                    if !unlit[note as usize & 0x7F] {
                        unlit[note as usize & 0x7F] = true;
                        out.note(&format!(
                            "button {note}: no light sent (Akai's app was never seen lighting it)"
                        ));
                    }
                }
                app::Effect::Octave(d)
                    if synth_on
                        || kit_to == Some(synth_kit)
                        || mix.instruments.holds(app.track()) =>
                {
                    if synth.shift_octave(d) {
                        own_follow_synth(&synth, &mut mix.instruments);
                        if synth_on {
                            app.set_pad_layout(Some(synth.pad_layout()));
                        }
                        out.note(&synth_pads_note(&synth));
                        out.show(gui::ToUi::KitChanged {
                            kit: synth_kit,
                            view: synth.kit_view(),
                        });
                        rest = synth.pad_colours();
                        ports.send_batch(
                            out,
                            false,
                            "the synth's pad colours",
                            &as_slices(&pad_colours(&rest)),
                        )?;
                        lit = [false; 16];
                        mute_shown = None;
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
                        out.note(&format!(
                            "synth: {} on the pads, preset {}",
                            synth.engine().name(),
                            synth.preset_name()
                        ));
                        out.show(gui::ToUi::Synth(synth.state()));
                    }
                    own_follow_synth(&synth, &mut mix.instruments);
                    out.note(&synth_pads_note(&synth));
                    out.show(gui::ToUi::KitChanged {
                        kit: synth_kit,
                        view: synth.kit_view(),
                    });
                    if synth_on {
                        app.set_pad_layout(Some(synth.pad_layout()));
                        rest = synth.pad_colours();
                        ports.send_batch(
                            out,
                            false,
                            "the synth's pad colours",
                            &as_slices(&pad_colours(&rest)),
                        )?;
                        lit = [false; 16];
                        mute_shown = None;
                        bank_lights = true;
                    }
                }
                // The Instrument row: the selected track's own instrument, or
                // the kit again.
                app::Effect::TrackSound { track, id } => match id {
                    None => {
                        match mix.instruments.take_out(track) {
                            true => out.note(&format!("track {}: plays the kit", track + 1)),
                            false => {
                                let s = format!(
                                    "Track {}: its last sound is still ringing out; try again in a moment",
                                    track + 1
                                );
                                out.note(&s);
                                out.show(gui::ToUi::Tell(s));
                            }
                        }
                        app.set_track_sound(track, mix.instruments.held(track));
                        if let Some(p) = track_presets.get_mut(track) {
                            *p = None;
                        }
                        out.show(gui::ToUi::Synth(synth_tab(
                            &synth,
                            &mix.instruments,
                            track,
                            None,
                        )));
                    }
                    // Not made at all while every place is taken.
                    Some(id) if !mix.instruments.has_room(track) => {
                        let s = full_note(track, instrument_name(mix.plugins(), id));
                        out.note(&s);
                        out.show(gui::ToUi::Tell(s));
                        app.set_track_sound(track, mix.instruments.held(track));
                    }
                    Some(id) => {
                        let job = instrument_job(&synth, track, id, None);
                        if !mix.build_instrument(job) {
                            let s = format!(
                                "Track {}: busy making another sound; {id} not made, try again",
                                track + 1
                            );
                            out.note(&s);
                            out.show(gui::ToUi::Tell(s));
                            app.set_track_sound(track, mix.instruments.held(track));
                        }
                    }
                },
                app::Effect::StopSounds => {
                    player.stop_all();
                    synth.stop();
                    mix.instruments.stop();
                }
                app::Effect::Audio(p) => audio_change = Some(p),
                app::Effect::Quit => stop = true,
                app::Effect::Show(v) => {
                    // Opening the Synth tab hands the pads to the synth, and
                    // opening the Sample tab gives them back to the drum kit.
                    if v == app::View::Synth && !synth_on {
                        kit_to = Some(synth_kit);
                    } else if v.sampler() && synth_on {
                        kit_to = Some(player.kit());
                    }
                    out.show(gui::ToUi::Show(v));
                }
                // 16 LEVEL: the screen holds the kits; the synth's pads have
                // no 16 Levels.
                app::Effect::SixteenLevels if synth_on || kit_to == Some(synth_kit) => {
                    out.note("16 LEVEL: 16 Levels plays a kit; pick a kit with the wheel first")
                }
                app::Effect::SixteenLevels => out.show(gui::ToUi::SixteenLevels),
                app::Effect::SaveProject => save_asked = true,
                // PROG EDIT: the sound the pads play (Jay, 20:40Z).
                app::Effect::ProgEdit => {
                    let synth_now = kit_to.map_or(synth_on, |k| k == synth_kit);
                    if synth_now {
                        later.extend(app.open_tab(app::View::Synth));
                    } else {
                        // The Pads tab on its Pad view: the pad's settings.
                        app.set_pads_browse(false);
                        out.show(gui::ToUi::PadsView(mpfree_sampler_ui::pads_tab::PAD_VIEW));
                        later.extend(app.open_tab(app::View::Pads));
                    }
                }
                // A PAD BANK button: banks A-H are kits 1-8 (9-16 and the
                // Grains kit are banks A-H again, from kit 9 on).
                // While the synth has the pads, PAD BANK A-H picks a bank's
                // pad layout (Jay, 09:55Z), not a drum kit.
                app::Effect::Bank { button, shift }
                    if button < 4
                        && kit_to.unwrap_or(if synth_on { synth_kit } else { player.kit() })
                            == synth_kit =>
                {
                    let target = app::bank_target(synth.settings().bank % 8, button, shift);
                    later.extend(app.synth_bank(target));
                    bank_lights = true;
                }
                // TRACK MUTE came or went, or its bank changed.
                app::Effect::BankLights => bank_lights = true,
                app::Effect::Bank { button, shift } => {
                    let now = kit_to.unwrap_or(if synth_on { synth_kit } else { player.kit() });
                    if button < 4 {
                        let group = if now >= drum_kits { 0 } else { now / 8 * 8 };
                        let target = group + app::bank_target(now % 8, button, shift);
                        if target < drum_kits {
                            kit_to = Some(target);
                            out.note(&format!(
                                "bank: {} (kit {}: {})",
                                app::bank_letter(target),
                                target + 1,
                                kits[target].name
                            ));
                        } else {
                            out.note(&format!(
                                "bank {}: no kit there ({} kits)",
                                app::bank_letter(group + app::bank_target(now % 8, button, shift)),
                                drum_kits
                            ));
                        }
                    }
                    bank_lights = true;
                }
                app::Effect::SavePatterns(text) => {
                    // The store's own thread writes it and reports the save.
                    if let Err(e) =
                        store.write(app::saved::ARP, app::saved::ARP_PATTERNS, text.into_bytes())
                    {
                        out.note(&format!("arp patterns: not saved ({e})"));
                    }
                }
                // The desk's, taken above.
                _ => {}
            }
            parts.item_named(parts::APP, t_item, &item);
        }
        fx.append(&mut later);
        if let Some(p) = audio_change {
            totals.periods += play.periods;
            totals.xruns += play.xruns;
            totals
                .summaries
                .push(format!("{periods} periods: {}", play.finish(out)?));
            periods = p;
            out.note(&format!(
                "audio buffer: {p} periods ({:.1} ms); starting the sound again",
                p as f64 * period_time().as_secs_f64() * 1e3
            ));
            play = Playback::start(open_acva(out, p)?, p)?;
            lane_until = cores::lane_deadline(Instant::now(), None, p);
            pending.clear();
        }
        // Drums' Pad 1 note, a pad's sound or another kit: the pad grid
        // and the arp take the pads' new names and notes.
        if synth.take_pads_changed() {
            out.show(gui::ToUi::KitChanged {
                kit: synth_kit,
                view: synth.kit_view(),
            });
            if synth_on {
                app.set_pad_layout(Some(synth.pad_layout()));
            }
        }
        let playing_kit = if synth_on { synth_kit } else { player.kit() };
        if let Some(k) = kit_to.filter(|&k| k != playing_kit) {
            used[k] = true;
            if k == synth_kit {
                // The sampler keeps its kit; its sounds finish.
                synth_on = true;
                rest = synth.pad_colours();
                out.note(&synth_pads_note(&synth));
                ports.send_batch(
                    out,
                    false,
                    "the synth's pad colours",
                    &as_slices(&pad_colours(&rest)),
                )?;
            } else {
                if synth_on {
                    synth_on = false;
                    synth.all_off();
                }
                player.select(k);
                rest = sound_colours(player.sounds(k), k);
                out.note(&format!(
                    "kit {}: {} ({} sounds)",
                    k + 1,
                    kits[k].name,
                    kits[k].sounds.len()
                ));
                ports.send_batch(
                    out,
                    false,
                    &format!("kit {}'s pad colours", k + 1),
                    &as_slices(&pad_colours(&rest)),
                )?;
            }
            lit = [false; 16];
            mute_shown = None;
            // The arp plays the synth's layout notes, or a kit's pad slots.
            app.set_pad_layout(synth_on.then(|| synth.pad_layout()));
            out.show(gui::ToUi::Kit(k));
            bank_lights = true;
        }
        // The PAD BANK buttons found so far: the bank playing is lit, or
        // TRACK MUTE's bank while the pads mute tracks.
        if bank_lights {
            let bank = Some(if app.mute_keys() == app::MuteKeys::Tracks {
                app.mute_bank() % 4
            } else if synth_on {
                synth.settings().bank % 4
            } else {
                player.kit() % 4
            });
            for (b, note) in app.map.bank.iter().enumerate() {
                let Some(note) = *note else { continue };
                let value = if bank == Some(b) {
                    app::LIGHT_ON
                } else {
                    app::LIGHT_OFF
                };
                if akai::app_lights(note) {
                    let light = akai::button_light(note, value);
                    if !cv_out.light(&light) {
                        ports.send(true, &light)?;
                    }
                } else if !unlit[note as usize & 0x7F] {
                    unlit[note as usize & 0x7F] = true;
                    out.note(&format!(
                        "button {note}: no light sent (Akai's app was never seen lighting it)"
                    ));
                }
            }
        }
        parts.lap(parts::APP);
        let time_span = Span::start();
        app.set_rolling(time.info().state.rolling());
        app.set_track_count(time.track_count());
        // The transport buttons' lights follow the transport and the
        // loops (only the ones that changed; next period's effects).
        {
            let info = time.info();
            let (loop_rec, loop_dub) = looper.recording();
            fx.extend(app.transport_lights(
                app::TransportLights {
                    rolling: info.state.rolling(),
                    recording: loop_rec
                        || matches!(
                            info.state,
                            mpfree_transport::State::Recording | mpfree_transport::State::CountIn
                        ),
                    overdub: loop_dub || info.overdub,
                },
                t,
            ));
        }
        // Another device's clock, when the project's tempo is from a port
        // (M4): the leader's bytes read this period go to the follower
        // before the transport runs, as read at the period's top.
        music.leader_clock(|follow, bytes| {
            time.follow(follow);
            if !bytes.is_empty() {
                let frame = time.next_frame();
                time.clock_in(frame, bytes);
            }
        });
        // Ignored while a followed clock arrives.
        time.follow_tempo(app.settings.bpm);
        // The transport runs first; the clips and the arps follow its clock.
        time.advance(PERIOD as u32);
        // A followed clock's tempo becomes the app's (the tempo chip, Note
        // Repeat, the arps, tempo-synced effects) when it has moved more
        // than 0.08 BPM from it, so the chip doesn't flicker.
        if let Some(bpm) = time.followed_bpm() {
            if (bpm - app.settings.bpm).abs() > 0.08 {
                followed_tempo = Some(bpm);
            }
        }
        // The tempo chip says where the tempo comes from while a followed
        // clock sets it ("120.0 from MIDI In"), and goes back when it
        // stops. Sent only when that changes, so the name is built only
        // then.
        let from = time.followed_bpm().and(music.leader_name());
        if from != clock_from.as_deref() {
            clock_from = from.map(str::to_string);
            out.show(gui::ToUi::ClockFrom(clock_from.clone()));
        }
        // MIDI Out takes the project's track outputs, and sends programs as
        // the song starts, before this period's notes.
        midi_out.sync(&io_model, time.info().state.rolling());
        midi_out.clock(time.block());
        // CV out takes the outputs' jobs; run, reset and clock follow the
        // transport.
        cv_out.sync(&io_model);
        cv_out.clock(time.block());
        // The armed track is the one the pads and the arp screens play.
        let armed = time.armed().unwrap_or(0);
        if armed != app.track() {
            let before = app.track();
            fx.extend(app.set_track(armed));
            // The Synth tab follows the selected track's own synth.
            if mix.instruments.holds(armed) || mix.instruments.holds(before) {
                let preset = track_presets.get(armed).copied().flatten();
                out.show(gui::ToUi::Synth(synth_tab(
                    &synth,
                    &mix.instruments,
                    armed,
                    preset,
                )));
            }
        }
        if let Some(app::Effect::Arp(v)) = app.arp_poll() {
            out.show(gui::ToUi::Arp(Box::new(v)));
        }
        // A track's own instrument the builder made goes in (the old one
        // rings out), and retired ones go back to be dropped; before the
        // notes, so it plays this period's. The synth is home here.
        if let Some(landed) = mix.poll_instruments() {
            match landed {
                mixing::Landed::Playing { track, name } => {
                    out.note(&format!("track {}: plays {name}", track + 1));
                    if track == app.track() {
                        let preset = track_presets.get(track).copied().flatten();
                        out.show(gui::ToUi::Synth(synth_tab(
                            &synth,
                            &mix.instruments,
                            track,
                            preset,
                        )));
                    }
                }
                mixing::Landed::Failed { track, id } => {
                    let s = format!(
                        "Track {}: {id} couldn't be made (see the Console); it keeps what it had",
                        track + 1
                    );
                    out.note(&s);
                    out.show(gui::ToUi::Tell(s));
                }
                mixing::Landed::Full { track, name } => {
                    let s = full_note(track, name);
                    out.note(&s);
                    out.show(gui::ToUi::Tell(s));
                }
            }
            let (mixing::Landed::Playing { track, .. }
            | mixing::Landed::Failed { track, .. }
            | mixing::Landed::Full { track, .. }) = landed;
            app.set_track_sound(track, mix.instruments.held(track));
        }
        // The clips and every track's arp (the pads go through the armed
        // track's) play into the kit or the synth: the clips as recorded,
        // the arps' notes as live playing (SeqPads::live).
        let mut seq = SeqPads {
            player: &mut player,
            synth: synth_on.then_some(&mut synth),
            flashes: [0; 16],
            hits: 0,
            live: app.live_map(),
            live_hits: [(0, 0); MAX_LIVE_HITS],
            n_live: 0,
            silent: 0,
            midi: &mut midi_out,
            cv: &mut cv_out,
            own: &mut mix.instruments,
            tap: &mut export.tap,
        };
        time.period_arps(PERIOD as u32, &mut seq, &mut app.arps);
        seq.midi.period_end(PERIOD as u32, t_read);
        seq.cv.period_end(PERIOD as u32, t_read);
        // The time engine: the transport, the clips and every track's arp
        // (the pads' notes start here; their sounds are the sampler's or
        // the synth's).
        engines.stop(engines::TIME, time_span);
        parts.begin();
        // The pads' own hits: the first sound of a press shows the pad
        // down; the arp's repeats flash it.
        for &(pad, v) in &seq.live_hits[..seq.n_live] {
            let pad = usize::from(pad);
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
        silent_hits += seq.silent;
        hits += seq.hits;
        // The pads the clips lit, kept so the mixer can be reached below.
        let seq_flashes = seq.flashes;
        // The clips' parameter locks: to the mixer's mod engine, on their
        // frames of this period.
        for &(offset, addr, value) in time.locks() {
            mix.lock(offset, addr, value);
        }
        for &(offset, addr, value) in time.lanes() {
            mix.lane(offset, addr, value);
        }
        time.clear_locks();
        // The Play page: where the hits fell (Beats' timing strip), and the pattern.
        for h in time.hits() {
            out.show(gui::ToUi::Hit(*h));
        }
        time.clear_hits();
        if time.take_steps_changed() {
            out.show(gui::ToUi::Steps(Box::new(time.steps())));
        }
        for (pad, &v) in seq_flashes.iter().enumerate().filter(|(_, v)| **v > 0) {
            out.show(gui::ToUi::PadFlash { pad, velocity: v });
        }
        // The Sample page's playheads, about 25 times a second while it shows.
        if app.view() == app::View::Sample && heads_at.elapsed() >= SAMPLE_HEADS_EVERY {
            let heads = player.playheads();
            if heads != shown_heads {
                out.show(gui::ToUi::SamplePlayheads(heads));
                shown_heads = heads;
            }
            // The Grain tab's cloud, if the screen sent its box. A full
            // channel hands the box back, to keep for the next time.
            if let Some(mut c) = cloud.take() {
                player.fill_cloud(&mut c);
                if let Err(gui::ToUi::SampleGrains(c)) = out.try_show(gui::ToUi::SampleGrains(c)) {
                    cloud = Some(c);
                }
            }
            heads_at = Instant::now();
        }
        if let Some(line) = time.take_song_note() {
            out.note(&format!("time: {line}"));
        }
        // A region just moved is picked again where it went.
        if let Some(r) = tracks.take_repick() {
            let _ = time.act(TimeAction::Audio(AudioAction::Pick(Some(r))));
        }
        // `|`, not `||`: both flags are taken.
        if time.take_changed() | tracks.take_changed() {
            out.show(gui::ToUi::Time(Box::new(time_state(&time, &tracks))));
            (shown_transport, transport_shown_at) = (time.info(), Instant::now());
        } else {
            let now = time.info();
            let moved = now.position != shown_transport.position;
            let changed = TransportInfo {
                position: shown_transport.position,
                ..now
            } != shown_transport;
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
                    if !lit[pad] && mute_shown.is_none() {
                        ports.send(false, &akai::pad_colour(pad as u8, rest[pad]))?;
                    }
                }
            }
        }
        // TRACK MUTE: the pads show the mute keys in place of the sounds.
        let keys = app.mute_keys();
        if keys != app::MuteKeys::Off {
            let kit = player.kit();
            let first = app.mute_bank() * 16;
            let track = |t: usize| time.muted(first + t).map(|m| (m, time.launched(first + t)));
            let want = mute_colours(
                keys,
                sound_colours(player.sounds(kit), kit),
                track,
                mix.pad_mutes(),
            );
            for pad in 0..16 {
                if mute_shown.is_none_or(|s| s[pad] != want[pad]) {
                    ports.send(false, &akai::pad_colour(pad as u8, want[pad]))?;
                }
            }
            mute_shown = Some(want);
        } else if mute_shown.take().is_some() {
            // Left: the pads' own colours again, lit below where they play.
            ports.send_batch(
                out,
                false,
                "the pads' colours",
                &as_slices(&pad_colours(&rest)),
            )?;
            lit = [false; 16];
        }
        // Pad lights: lit while held, while the arp keeps them going (latched, or held on its grid),
        // or while a pad set to Latch in the sampler plays on.
        let latched = if synth_on { 0 } else { player.latched_pads() };
        for pad in 0..16 {
            if mute_shown.is_some() {
                break;
            }
            let want =
                held[pad] || (app.generating() && app.active(pad)) || latched & (1 << pad) != 0;
            if want != lit[pad] {
                lit[pad] = want;
                let colour = if want {
                    akai::hit_colour(rest[pad])
                } else {
                    rest[pad]
                };
                ports.send(false, &akai::pad_colour(pad as u8, colour))?;
            }
        }
        if session && last_ping.elapsed() >= Duration::from_secs(akai::PING_SECONDS) {
            let ping = akai::session_ping(akai::SESSION_TOKEN);
            ports.send(false, &ping)?;
            pings += 1;
            last_ping = Instant::now();
            out.midi(&format!(
                "+{:.3} s -> Public: {} = session ping",
                ports.t(),
                hex(&ping)
            ));
        }
        if session && !lost && last_reply.elapsed() > Duration::from_secs(10) {
            lost = true;
            out.note("no session reply for 10 s");
            out.show(gui::ToUi::Session("No reply".into(), Tone::Bad));
        }
        voices = voices.max(player.playing() + synth.active_voices());
        parts.lap(parts::PADS);
        let t_render = Instant::now();
        // Warped pads play at the transport's tempo.
        player.set_tempo(time.info().bpm);
        // Each pad of the playing kit into its pad channel (Pad Mix); they
        // all go to the Drums track.
        // With a synth lane (Cores 3, 2 and 1) the synth starts on its core
        // now and renders beside the drums; otherwise this only notes the
        // frame and tempo, and it renders below as before.
        synth.start(i * PERIOD as u64, app.settings.bpm);
        let mut pads = mix.pads();
        // With the drums split (Cores 3 and 2), the lane's voices are waited
        // for until the period's deadline at most.
        player.render_routed_until(PERIOD, &mut pads, lane_until);
        let played = pads.played();
        mix.drums(played, &mut drums_l, &mut drums_r);
        let t_synth = Instant::now();
        render_time.add(t_synth - t_render);
        engines.add(engines::SAMPLER, t_synth - t_render);
        synth_l.fill(0.0);
        synth_r.fill(0.0);
        // Renders here, or (with a synth lane) waits for the lane until the
        // period's deadline and adds what it made; then the time is only
        // the wait. Past the deadline the synth is left out of this period
        // (counted, cores.rs) and comes back after the write.
        synth.finish(lane_until, &mut synth_l, &mut synth_r);
        let synth_took = t_synth.elapsed();
        synth_time.add(synth_took);
        engines.add(engines::SYNTH, synth_took);
        // What the looper records from the pads: the drums and the synth as
        // the engines make them, before the mixer (the loop plays back
        // through the mixer, so the Level is applied once).
        for f in 0..PERIOD {
            pads_l[f] = drums_l[f] + synth_l[f];
            pads_r[f] = drums_r[f] + synth_r[f];
        }
        looper.take_pads(&pads_l, &pads_r);
        // The loop, for its track; the line input comes with it.
        let t_loop = Instant::now();
        let line_note = looper.run(time.block(), &mut loop_l, &mut loop_r);
        // The audio tracks record the same line input onto the Song view's
        // lanes, and play into the mixer's Audio track (song mode only).
        tracks.follow_latency(app.settings.loop_latency);
        tracks.period(
            time.block(),
            time.tempo_map(),
            time.song_on(),
            looper.input(),
            (&mut audio_l, &mut audio_r),
        );
        // The strip's waveform, built a slice at a time: its cost counts in
        // the looper's stage, so the stage numbers show it.
        let loop_wave = looper.wave_step(i, LOOP_WAVE_PERIODS);
        let loop_took = t_loop.elapsed();
        loop_time.add(loop_took);
        engines.add(engines::LOOPER, loop_took);
        // The same line input into the sampler's buffer, with the bar lines
        // (the Sample page's Rec tab and Skip back). A point in the mix
        // goes in after the mixer, below.
        if let (Some(f), sampler::RecSource::Line) = (rec_feed.as_mut(), rec_source) {
            let t_rec = Instant::now();
            let (l, r) = looper.input();
            f.period(time.block(), l, r, 0);
            engines.add(engines::SAMPLER, t_rec.elapsed());
        }
        if let Some(f) = rec_back.take() {
            if let Err(gui::ToUi::SampleInputBack(f)) = out.try_show(gui::ToUi::SampleInputBack(f))
            {
                rec_back = Some(f);
            }
        }
        // The mixer: every sound summed, the Level (master fader), Master
        // safety. The input strip follows the line input and Monitor.
        // Effects the builder made go into their chains; the ones edits took
        // out go back to it to be dropped.
        parts.begin();
        if let Some(line) = mix.poll_built() {
            out.note(line);
        }
        // What the plugin rack said (a plugin started, crashed, restarted
        // or was left off): never hidden. No allocation when quiet.
        if let Some(lines) = found_plugins.take_lines() {
            for line in lines {
                out.note(&line);
            }
        }
        if mix.opening() {
            if let Some(line) = mix.tend_open() {
                out.note(&line);
            }
        }
        // The parts New project goes back to: the engines as they started,
        // once the mixer's own inserts are in (a few small allocations,
        // once, before anything can have been played).
        // A screen too busy to take them gets them made again next period.
        if !defaults_sent && mix.mixer.edits_pending() == 0 && mix.inserts_building() == 0 {
            defaults_sent = out
                .try_show(gui::ToUi::ProjectDefaults(Box::new(project::Defaults {
                    tempo: app.settings.bpm,
                    mixer: project::mixer_text(&mix.mixer, mix.pad_mutes()),
                    modu: mix.modu.save(),
                    synth: project::synth_text(&app.synth_settings(), &synth.sounds()),
                })))
                .is_ok();
        }
        // A save: what the engines hold, copied for the screen thread,
        // which writes the parts and hands them to the library's worker.
        // Small allocations, once a save; nothing is written here.
        // The Looper's loops are read first, as they play (a few periods,
        // longer while a loop records); the rest is copied once they are.
        if looper.opening() {
            if let Some(line) = looper.tend_open() {
                out.note(&line);
            }
        }
        if let Some(line) = tracks.take_note() {
            out.note(&line);
        }
        if std::mem::take(&mut save_asked) {
            if mix.opening() || looper.opening() || tracks.opening() {
                out.note("project: not saved, a project is still opening (try again in a moment)");
            } else if collect_now || parts_out.is_some() {
                out.note("project: already saving");
            } else {
                let line = looper.save();
                loops_saving = looper.saving().is_some();
                if !loops_saving {
                    out.note(&format!("project: saving without the loops ({line})"));
                }
                // The takes are read once no track records.
                audio_saving = tracks.host.save();
                if !audio_saving {
                    out.note(
                        "project: saving without the audio tracks (their worker did not start)",
                    );
                } else if (0..mpfree_audiotracks::MAX_TRACKS)
                    .any(|t| tracks.host.tracks.recording(t).is_some())
                {
                    out.note("project: saving once the audio tracks stop recording");
                }
                collect_now = true;
            }
        }
        if loops_saving {
            loops_held = looper.take_saved();
            loops_saving = loops_held.is_none();
        }
        if audio_saving {
            audio_held = tracks.host.take_saved();
            audio_saving = audio_held.is_none();
        }
        if collect_now && !loops_saving && !audio_saving {
            collect_now = false;
            let (loops, audio) = (loops_held.take(), audio_held.take());
            parts_out = Some(gui::ToUi::ProjectParts(Box::new(project::Collected {
                tempo: app.settings.bpm,
                swing: time.info().swing,
                clips: time.project().clone(),
                mixer: project::mixer_text(&mix.mixer, mix.pad_mutes()),
                modu: mix.modu.save(),
                synth: project::synth_text(&app.synth_settings(), &synth.sounds()),
                arp: app.arp_part(),
                instruments: project::instruments_text(&mix.instruments),
                loops,
                audio,
            })));
        }
        // A screen too busy keeps them here (the loops' audio is never
        // freed on this thread) until it has room.
        if let Some(m) = parts_out.take() {
            parts_out = out.try_show(m).err();
        }
        parts.lap(parts::PADS);
        let t_mix = Span::start();
        mix.follow(looper.line.is_open(), looper.monitor);
        let ctx = mixing::ctx(RATE, i * PERIOD as u64, app.settings.bpm, time.block());
        // The mixer's lanes (each stage's inserts, the returns) keep the
        // same deadline.
        mix.mixer.set_lane_deadline(Some(lane_until));
        let (p, caught) = mix.period(
            &ctx,
            (&synth_l, &synth_r),
            (&loop_l, &loop_r),
            (&audio_l, &audio_r),
            looper.input(),
            &mut left,
            &mut right,
        );
        if caught {
            clipped += 1;
        }
        // The metronome goes on top; it keeps the samples within ±1 itself.
        let p = p.max(time.mix_click(&mut left, &mut right));
        engines.stop(engines::MIX, t_mix);
        // Export: the pass takes this period's capture points and notes;
        // once it is over, the transport goes back to where it was and the
        // pass goes to the screen to finish (export.rs).
        if export.running() {
            export.notes(time.block());
            if let Some(run) = export.period(&mut mix.mixer, &mut time) {
                out.show(gui::ToUi::ExportOver(run));
            }
        }
        // Resampling: the point in the mix the Rec tab records (before the
        // metronome), with the bar lines as late as that point hears them.
        if let (Some(f), sampler::RecSource::Mix(..)) = (rec_feed.as_mut(), rec_source) {
            if let Some((l, r, latency)) = mix.rec_capture() {
                let t_rec = Instant::now();
                f.period(time.block(), l, r, latency);
                engines.add(engines::SAMPLER, t_rec.elapsed());
            }
        }
        parts.begin();
        if let Some(st) = mix.poll_state(Instant::now()) {
            out.show(gui::ToUi::Mixer(Box::new(st)));
        }
        panels.poll(
            app.view(),
            &mix,
            time.info().bpm,
            Instant::now(),
            &mut |m| out.show(m),
        );
        // The Play page's master meter and what its Q-Links show.
        if meter_at.elapsed() >= METER_EVERY {
            meter_at = Instant::now();
            let pk = mix.mixer.take_peaks(mpfree_fx::StripId::Master);
            let db = |v: f32| if v > 1e-6 { 20.0 * v.log10() } else { -90.0 };
            out.show(gui::ToUi::Meter {
                peak_db: [db(pk[0]), db(pk[1])],
                limit_db: mix.gain_reduction_db(),
            });
            let levels = mix.play_levels();
            if levels != shown_levels {
                shown_levels = levels;
                out.show(gui::ToUi::PlayLevels(levels));
            }
            // The audio tracks' level bars on Arrange: eight numbers, not a
            // whole song state.
            if app.view() == app::View::Arrange {
                if let Some(l) = tracks.take_levels() {
                    out.show(gui::ToUi::AudioLevels(l));
                }
            }
        }
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
        if let Some(wave) = loop_wave {
            out.show(gui::ToUi::LoopWave(wave));
        }
        // The Looper tab and the Play page's strip follow the loop as often
        // as the transport card, so the playhead moves smoothly; elsewhere
        // four times a second is enough.
        if strip_shown(app.view()) && loop_shown_at.elapsed() >= TRANSPORT_EVERY {
            loop_shown_at = Instant::now();
            out.show(gui::ToUi::Looper(Box::new(looper.view(app.settings.bpm))));
        }
        peak = peak.max(p);
        window_peak = window_peak.max(p);
        // The browser's preview, over everything on the main out.
        if let Some(pv) = preview.as_mut() {
            if pv.mix(RATE, &mut left, &mut right) {
                if let Some(done) = preview.take() {
                    if preview_back.replace(done).is_some() {
                        out.note("preview: two sounds going back; one freed on the audio thread");
                    }
                }
            }
        }
        for f in 0..PERIOD {
            frames[2 * f] = float_to_s32(left[f]);
            frames[2 * f + 1] = float_to_s32(right[f]);
        }
        let xruns = play.xruns;
        parts.lap(parts::SCREEN);
        engines.stop(engines::PERIOD, period_span);
        // A fast export runs the periods back to back with the card
        // stopped (the speakers quiet), as fast as its files take them;
        // after it the card fills and plays again (export.rs).
        let fast = export.fast();
        if fast != play.paused {
            if fast {
                play.pause()?;
            } else {
                play.resume()?;
            }
        }
        if fast {
            export.wait_for_room(i);
        } else {
            play.write(&frames, i)?;
        }
        // How much the card has queued now sets the next period's deadline
        // for the lanes, and times the hits of this period. In a fast
        // export the lanes are waited for, never left out.
        let written = Instant::now();
        let delay = play.pcm.delay();
        lane_until = if fast {
            written + export::FAST_LANE_WAIT
        } else {
            cores::lane_deadline(
                written,
                delay.as_ref().ok().map(|&d| i64::from(d)),
                play.queued,
            )
        };
        // A synth left out of this period (late) comes back before anything
        // plays it again.
        synth.settle(lane_until);
        parts.begin();
        match delay {
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
                    out.note(&format!(
                        "PCM delay: {e} (no timing; the lanes' deadline assumes a full buffer)"
                    ));
                }
            }
        }
        // What the lights' thread could not write, and lights the queue had
        // no room for (at most a line a second).
        if let Some(e) = ports.queue_error() {
            out.note(&e);
        }
        if let Some(line) = ports.queue_watch(Instant::now()) {
            out.note(&line);
        }
        let d = ports.queue_dropped();
        if d != lights_dropped && lights_noted.elapsed() >= Duration::from_secs(1) {
            out.note(&format!(
                "controller output: {} messages dropped with the queue full ({d} since the start)",
                d - lights_dropped
            ));
            (lights_dropped, lights_noted) = (d, Instant::now());
        }
        if last_repeat_note.elapsed() >= Duration::from_secs(1) {
            last_repeat_note = Instant::now();
            if window_repeats > 0 {
                let list: Vec<String> = (0..16)
                    .filter(|&p| window_pads[p])
                    .map(|p| (p + 1).to_string())
                    .collect();
                let latched: Vec<String> = (0..16)
                    .filter(|&p| app.active(p) && !held[p])
                    .map(|p| (p + 1).to_string())
                    .collect();
                out.note(&format!(
                    "repeat: {window_repeats} hits in the last second on pads {}; latched: {}; \
                                   engine dropped {} events, overflow {}",
                    list.join(" "),
                    if latched.is_empty() {
                        "none".to_string()
                    } else {
                        latched.join(" ")
                    },
                    app.arps.dropped(),
                    app.arps.overflow()
                ));
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
                              20.0 * peak.max(1e-6).log10(), lat.describe(), app.arps.dropped(), app.arps.overflow(),
                              sampler_status(&player, &render_time), time.describe()));
            out.note(&format!("status: {}", synth_status(&synth, &synth_time)));
            out.note(&format!(
                "status: {}; {}",
                looper.describe(),
                loop_time.describe()
            ));
            out.note(&format!("status: {}", tracks.describe()));
            out.note(&format!("status: {}", mix.modu.describe()));
            if let Some(line) = found_plugins.summary().and_then(|s| s.line()) {
                out.note(&format!("status: plugins: {line}"));
            }
            if let Some(line) = mix.instruments.describe() {
                out.note(&format!("status: tracks' own instruments: {line}"));
            }
            out.note(&format!(
                "status: {}",
                engines::log_line(&engines.lifetime())
            ));
            out.note(&format!(
                "status: {}",
                cores.status(cores::Counts::read(&player, &mix.mixer, &synth))
            ));
            out.note(&format!("status: {}", parts.status()));
            out.note(&format!("status: {}", touch_hold.describe()));
        }
        if last_timing.elapsed() >= Duration::from_millis(250) {
            last_timing = Instant::now();
            let seconds_left = timed.then(|| o.seconds.saturating_sub(started.elapsed().as_secs()));
            out.show(gui::ToUi::Timing(gui::Timing {
                hits,
                periods: totals.periods + play.periods,
                underruns: totals.xruns + play.xruns,
                voices: player.playing() + synth.active_voices(),
                peak_db: 20.0 * window_peak.max(1e-6).log10(),
                last_ms: lat.last as f32,
                min_ms: lat.min as f32,
                mean_ms: lat.mean() as f32,
                max_ms: lat.max as f32,
                seconds_left,
                periods_queued: periods,
                audio: {
                    let (render_mean_us, render_max_us) = render_time.take_window();
                    let (synth_mean_us, synth_max_us) = synth_time.take_window();
                    let c = player.counters();
                    debug_page::AudioNumbers {
                        render_mean_us,
                        render_max_us,
                        synth_mean_us,
                        synth_max_us,
                        worst_us: (render_time.max + synth_time.max).as_secs_f32() * 1e6,
                        steals: c.steals,
                        dropped: c.dropped_events + app.arps.dropped(),
                        peak_grains: c.peak_grains,
                    }
                },
            }));
            window_peak = 0.0;
            engines.publish();
            let (failed, dropped, stalls) = ports.queue_counts();
            engine_shared
                .parts
                .set_counts(failed, dropped, stalls, out.report_dropped());
            // The second core's late parts: on the Dev page, and in the log
            // within a second (a fallback is never silent).
            let counts = cores::Counts::read(&player, &mix.mixer, &synth);
            cores.publish(&engine_shared.cores, counts);
            if let Some(line) = cores.late_note(counts, last_timing) {
                out.note(&line);
            }
            // A long job refused idle priority: in the log (and the
            // console) within a second, with the count.
            let idle = mpfree_lanes::os::idle_counts();
            if idle.refused != idle_noted {
                idle_noted = idle.refused;
                out.note(&cores::idle_note(idle));
            }
            // Prefs or found buttons changed: the store's thread writes them.
            for (owner, settings) in app.take_saves() {
                if store.saves() {
                    if let Err(e) = store.save_settings(owner, &settings) {
                        out.note(&format!("not saved: {owner}: {e}"));
                    }
                }
            }
            // The I/O view's rig changed: kept for the next start.
            if let Some(text) = midi_saver.take(&io_model) {
                if store.saves() {
                    let file = midi_saved::RIG;
                    if let Err(e) = store.write(midi_saved::OWNER, file, text.into_bytes()) {
                        out.note(&format!("not saved: midi/{file}: {e}"));
                    }
                }
            }
            // A Synth tab option or the arp changed with the controls: the
            // project has unsaved work.
            if app.take_project_edit() {
                out.show(gui::ToUi::Edited);
            }
            if !strip_shown(app.view()) {
                out.show(gui::ToUi::Looper(Box::new(looper.view(app.settings.bpm))));
            }
            let sounds: gui::TrackSounds = std::array::from_fn(|t| mix.instruments.held(t));
            if sounds != sounds_shown {
                sounds_shown = sounds;
                out.show(gui::ToUi::TrackSounds(Box::new(sounds)));
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
        parts.lap(parts::AFTER);
        if let Some(line) = parts.end(Instant::now(), t) {
            out.note(&line);
        }
        i += 1;
        if stop {
            stopped_by = "STOP";
            break;
        }
    }
    let summary = play.finish(out)?;
    totals
        .summaries
        .push(format!("{periods} periods: {summary}"));
    ports.quiet = false;
    if let Some(line) = ports.unqueue() {
        out.say(&line);
    }
    ports.counts(out);
    for line in music.summary() {
        out.say(&line);
    }
    // The notes still on end before the port lets go.
    let summary = midi_out.summary();
    if !midi_out.close() {
        out.say("MIDI Out: the last messages may not have been written");
    }
    for line in summary {
        out.say(&line);
    }
    // CV 1 to 4 go to 0 V before the port lets go.
    let summary = cv_out.summary();
    if !cv_out.close() {
        out.say("CV out: the last values may not have been written");
    }
    for line in summary {
        out.say(&line);
    }
    if key_misses > 0 {
        out.say(&format!("MIDI in: {key_misses} keys had no pad in the kit"));
    }
    drop(usb);
    let kits_used: Vec<String> = used
        .iter()
        .enumerate()
        .filter(|(_, u)| **u)
        .map(|(k, _)| (k + 1).to_string())
        .collect();
    out.say(&format!("pad to sound (read to leaving the card's buffer; plus up to one period, {:.1} ms, before a \
                      message is read): {}", period_time().as_secs_f64() * 1e3, lat.describe()));
    out.say(&format!("buttons: {}", app.map.describe()));
    out.say(&format!(
        "note engine: {repeats} repeats, {} events dropped, {} notes that could not be queued",
        app.arps.dropped(),
        app.arps.overflow()
    ));
    out.say(&format!("play done ({stopped_by}): {hits} hits played ({screen_hits} from the screen), {silent_hits} on \
                      pads without a sound, kits used {}, level {level_db:.1} dB, at most {voices} sounds at once, \
                      peak {:.1} dBFS, {clipped} periods clipped, {pings} pings; {}",
                     kits_used.join(" "), 20.0 * peak.max(1e-6).log10(), totals.summaries.join("; ")));
    out.say(&sampler_status(&player, &render_time));
    if views_swapped > 0 {
        out.say(&format!(
            "sample tab: {views_swapped} edited kit views swapped in"
        ));
    }
    out.say(&time.describe());
    if looper.line.is_open() {
        out.say(&looper.line.close());
    }
    out.say(&format!("{}; {}", looper.describe(), loop_time.describe()));
    out.say(&tracks.describe());
    out.say(&mix.modu.describe());
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
    if let Some(text) = midi_saver.take(&io_model) {
        if store.saves() {
            let _ = store.write(midi_saved::OWNER, midi_saved::RIG, text.into_bytes());
        }
    }
    store.flush();
    out.say(&engines::log_line(&engines.lifetime()));
    out.say(&cores.status(cores::Counts::read(&player, &mix.mixer, &synth)));
    if session {
        out.say("no more pings: the controller should drop the session by itself (inferred: within about 10 s)");
    }
    // The report is written here again once the loop is over.
    out.unqueue_report();
    Ok(())
}

fn pads(o: &Options) -> Result<(), String> {
    let heading = if o.keepalive {
        "pads, with keep-alive"
    } else {
        "pads (listen only)"
    };
    let mut out = Out::new(o, heading, true);
    let card = midi_card()?;
    let mut inputs = open_inputs(card, &mut out)?;
    let mut output = None;
    if o.keepalive {
        let (f, oi) =
            alsa::open_rawmidi(card, mpc_one::Port::Public as u32, alsa::Direction::Output)
                .map_err(|e| format!("opening the Public MIDI port: {e}"))?;
        out.say(&format!(
            "keep-alive goes to card {card} subdevice {} \"{}\" every 5 s",
            oi.subdevice, oi.subname
        ));
        if oi.subdevice != 0
            || oi.stream != alsa::RAWMIDI_STREAM_OUTPUT
            || !oi.subname.contains("Public")
        {
            return Err(
                "that is not the controller's Public port; stopping without sending".to_string(),
            );
        }
        output = Some(f);
    }
    out.say("every message is shown in hex, with its meaning where we know it");
    out.say(&format!(
        "hit some pads, press a button, turn a Q-Link and the wheel ({} s)",
        o.seconds
    ));
    let mut buf = [0u8; 256];
    let (mut pad_hits, mut pings, mut replies, mut unknown, mut pressure, mut shown) =
        (0u64, 0u64, 0u64, 0u64, 0u64, 0u64);
    let mut last_ping: Option<Instant> = None;
    let start = Instant::now();
    let end = start + Duration::from_secs(o.seconds);
    while Instant::now() < end {
        if let Some(f) = output.as_mut() {
            if last_ping.is_none_or(|t| t.elapsed() >= Duration::from_secs(5)) {
                let (_, msg) = mpc_one::keepalive_ping(OBSERVED_KEEPALIVE_TOKEN);
                f.write_all(&msg)
                    .map_err(|e| format!("sending the keep-alive: {e}"))?;
                pings += 1;
                last_ping = Some(Instant::now());
                out.say(&format!(
                    "+{:.3} s sent keep-alive {pings} on Public: {}",
                    start.elapsed().as_secs_f64(),
                    hex(&msg)
                ));
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
                            // MIDI In carries a keyboard, not the MPC's controls.
                            let event = if port.music {
                                None
                            } else {
                                mpc_one::decode(&msg)
                            };
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
                                out.say(&message_line(
                                    t,
                                    port.name,
                                    &bytes,
                                    truncated,
                                    event.as_ref(),
                                ));
                            } else if shown == MAX_SHOWN + 1 {
                                out.say(&format!(
                                    "(more than {MAX_SHOWN} messages: only counting from here)"
                                ));
                            }
                        }
                    }
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock || e.raw_os_error() == Some(EAGAIN) => {
                }
                Err(e) => return Err(format!("reading MIDI ({}): {e}", port.name)),
            }
        }
        if !got {
            thread::sleep(Duration::from_millis(2));
        }
    }
    for port in &inputs {
        out.say(&format!(
            "{}: {} bytes, {} messages, {} stray bytes",
            port.name, port.bytes, port.messages, port.parser.stray_bytes
        ));
    }
    let bytes: u64 = inputs.iter().map(|p| p.bytes).sum();
    let messages: u64 = inputs.iter().map(|p| p.messages).sum();
    out.say(&format!(
        "pads done: {bytes} bytes, {messages} messages, {pad_hits} pad hits, {pressure} pressure, \
                      {unknown} not recognised, {pings} keep-alives sent, {replies} replies"
    ));
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
            Ok(()) => out.say(&format!(
                "what the card allows with stereo S32_LE at 44.1 kHz: period size {}, \
                                        periods {}, buffer size {}",
                any.interval(alsa::PARAM_PERIOD_SIZE).describe(),
                any.interval(alsa::PARAM_PERIODS).describe(),
                any.interval(alsa::PARAM_BUFFER_SIZE).describe()
            )),
            Err(e2) => out.say(&format!("asking what it allows failed too: {e2}")),
        }
        return Err("no sound played".to_string());
    }
    if !hw.mask_has(alsa::PARAM_ACCESS, alsa::ACCESS_RW_INTERLEAVED)
        || !hw.mask_has(alsa::PARAM_FORMAT, alsa::FORMAT_S32_LE)
    {
        return Err(
            "the card did not keep interleaved S32_LE; stopping before playing".to_string(),
        );
    }
    let buffer = hw.interval(alsa::PARAM_BUFFER_SIZE).single().unwrap_or(0);
    let (num, den) = hw.rate_fraction();
    out.say(&format!("stream: rate {} ({num}/{den}), channels {}, period {} frames, periods {}, buffer {} frames, \
                      msbits {}, fifo {}",
                     hw.interval(alsa::PARAM_RATE).describe(), hw.interval(alsa::PARAM_CHANNELS).describe(),
                     hw.interval(alsa::PARAM_PERIOD_SIZE).describe(), hw.interval(alsa::PARAM_PERIODS).describe(),
                     hw.interval(alsa::PARAM_BUFFER_SIZE).describe(), hw.msbits(), hw.fifo_size()));
    if buffer as usize != PERIOD * periods as usize {
        return Err(format!(
            "unexpected buffer size {buffer}; stopping before playing"
        ));
    }
    let mut sw = alsa::SwParams::new(PERIOD as u32, buffer, buffer);
    pcm.sw_params(&mut sw)
        .map_err(|e| format!("sw params: {e}"))?;
    match sys::lock_memory() {
        Ok(()) => out.say("memory locked (mlockall)"),
        Err(e) => out.say(&format!("mlockall failed: {e} (continuing)")),
    }
    match sys::realtime(70) {
        Ok(()) => out.say("SCHED_FIFO priority 70"),
        Err(e) => out.say(&format!(
            "SCHED_FIFO failed: {e} (continuing at normal priority)"
        )),
    }
    Ok(pcm)
}

/// A running ACVA stream: writes one period at a time, recovers from
/// underruns, and keeps the timing of every period (a running mean and
/// worst, [`Gaps`]).
struct Playback {
    pcm: alsa::Pcm,
    /// Periods in the buffer.
    queued: u32,
    xruns: u64,
    late: u64,
    errors: u64,
    gaps: Gaps,
    notes: Vec<String>,
    started: Instant,
    prev: Instant,
    periods: u64,
    /// A fast export runs: the stream is stopped and nothing is written
    /// ([`Playback::pause`]).
    paused: bool,
}

impl Playback {
    /// A fast export (export.rs): the stream stops at once, what it had
    /// queued dropped, so the speakers go quiet and no underrun is
    /// counted while the loop runs ahead of the card.
    fn pause(&mut self) -> Result<(), String> {
        self.pcm
            .drop_stream()
            .map_err(|e| format!("pause for the export: {e}"))?;
        self.paused = true;
        Ok(())
    }

    /// The export is over: the stream is prepared and filled with silence
    /// again, as at the start, and plays on from the next write.
    fn resume(&mut self) -> Result<(), String> {
        self.pcm
            .prepare()
            .map_err(|e| format!("prepare after the export: {e}"))?;
        for _ in 0..self.queued {
            self.pcm
                .write_frames(&[0i32; PERIOD * 2])
                .map_err(|e| format!("refill after the export: {e}"))?;
        }
        self.paused = false;
        self.prev = Instant::now();
        Ok(())
    }

    fn start(pcm: alsa::Pcm, queued: u32) -> Result<Playback, String> {
        pcm.prepare().map_err(|e| format!("prepare: {e}"))?;
        // Queue the whole buffer: the stream starts when it is full.
        for _ in 0..queued {
            pcm.write_frames(&[0i32; PERIOD * 2])
                .map_err(|e| format!("first write: {e}"))?;
        }
        Ok(Playback {
            pcm,
            queued,
            xruns: 0,
            late: 0,
            errors: 0,
            gaps: Gaps::default(),
            notes: Vec::new(),
            started: Instant::now(),
            prev: Instant::now(),
            periods: 0,
            paused: false,
        })
    }

    fn write(&mut self, frames: &[i32; PERIOD * 2], i: u64) -> Result<(), String> {
        match self.pcm.write_frames(frames) {
            Ok(_) => {}
            Err(e) if e.raw_os_error() == Some(EPIPE) => {
                self.xruns += 1;
                if self.notes.len() < 20 {
                    self.notes.push(format!(
                        "underrun at period {i} ({:.3} s)",
                        self.started.elapsed().as_secs_f64()
                    ));
                }
                self.pcm
                    .prepare()
                    .map_err(|e| format!("prepare after underrun: {e}"))?;
                for _ in 0..self.queued {
                    self.pcm
                        .write_frames(&[0i32; PERIOD * 2])
                        .map_err(|e| format!("refill after underrun: {e}"))?;
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
            self.gaps.add(gap);
            if gap > period_time() * 2 {
                self.late += 1;
                if self.notes.len() < 20 {
                    self.notes.push(format!(
                        "late period {i}: {:.2} ms after the previous one",
                        gap.as_secs_f64() * 1e3
                    ));
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
        let (avg, max) = self.gaps.mean_max();
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
    if !engine.configure(ClickConfig {
        sample_rate: RATE,
        bpm: 120.0,
        beats_per_bar: 4,
    }) {
        return Err("the click engine refused its settings".to_string());
    }
    let gain = 10f32.powf(o.gain_db / 20.0);
    out.say(&format!(
        "playing a 120 BPM click for {} s at {:.0} dB (peak about {:.0} dBFS); \
                      keep the volume knob low",
        o.seconds,
        o.gain_db,
        20.0 * (0.47 * gain).log10()
    ));
    out.draw();

    let (mut left, mut right) = ([0f32; PERIOD], [0f32; PERIOD]);
    let mut frames = [0i32; PERIOD * 2];
    let total_periods = o.seconds * RATE as u64 / PERIOD as u64;
    let mut play = Playback::start(pcm, PERIODS)?;
    for i in 0..total_periods {
        engine
            .render(&mut left, &mut right)
            .map_err(|e| format!("click engine: {e:?}"))?;
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

/// The time between period writes: a count, a sum and the worst, kept on
/// the audio thread with no allocation. (Until 13 Nov every gap went into a
/// `Vec` that was never emptied: each time it doubled, past 2^16 periods,
/// the audio thread copied it, 8 MB at 2^19 periods and 16 MB at 2^20 on
/// the MPC, and live test 3's only two underruns came 4 periods after
/// 2^19 and 2^20, right after those copies.)
#[derive(Clone, Copy, Debug, Default)]
struct Gaps {
    n: u64,
    sum: Duration,
    max: Duration,
}

impl Gaps {
    fn add(&mut self, gap: Duration) {
        self.n += 1;
        self.sum += gap;
        self.max = self.max.max(gap);
    }

    /// The mean and the worst gap in seconds; 0 and 0 before any.
    fn mean_max(&self) -> (f64, f64) {
        if self.n == 0 {
            return (0.0, 0.0);
        }
        (
            self.sum.as_secs_f64() / self.n as f64,
            self.max.as_secs_f64(),
        )
    }
}

// ---------------------------------------------------------------- preview (host)

fn write_png(path: &Path, canvas: &SoftCanvas) -> Result<(), String> {
    let png = mpfree_ui::png::encode_rgb(
        canvas.width() as u32,
        canvas.height() as u32,
        &canvas.to_rgb(),
    );
    // New files only: never overwrite.
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .and_then(|mut f| f.write_all(&png))
        .map_err(|e| format!("{}: {e}", path.display()))?;
    println!("preview written: {}", path.display());
    Ok(())
}

/// Sample screens as PNG files in a new or empty folder: the one-shot steps'
/// console, and every page of the pad player. No devices are opened.
fn preview(o: &Options) -> Result<(), String> {
    let dir = o.preview.as_ref().unwrap();
    fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let fonts = || {
        Fonts::with_files(
            o.font.as_deref(),
            o.bold_font.as_deref(),
            o.mono_font.as_deref(),
        )
    };
    let theme = load_theme(o)?;
    let mut c = gui::Console::new(fonts()?, "touch", None);
    for l in [
        "===== mpfree touch =====",
        "touchscreen: /dev/input/event0 (ili2117); read-only, not grabbed",
        "touch the screen now, with one finger and then two (10 s)",
        "touch: 1 contact(s): slot 0: x 1024 y 512",
        "touch: 2 contact(s): slot 0: x 1020 y 515, slot 1: x 300 y 1700",
        "touch: no contact",
        "touch done: 412 events, 96 frames, 3 lines shown",
    ] {
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
        p.ui.set_theme(theme.clone());
        let (w, h) = p.ui.size();
        let mut canvas = SoftCanvas::new(w, h);
        p.ui.paint(&mut canvas);
        write_png(&dir.join(format!("{name}.png")), &canvas)?;
    }
    Ok(())
}

/// Each sample page's first two frames as display lists (mpfree_ui::record's
/// wire format), one new NAME.mpdl file per page: the page as first drawn,
/// then what it redraws a tick (33 ms) later. mpfree-gpu-bench plays them on
/// the MPC's GPU and in software. No devices are opened; it writes only new
/// files.
fn display_lists(o: &Options) -> Result<(), String> {
    let dir = o.display_lists.as_ref().unwrap();
    fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let fonts = || {
        Fonts::with_files(
            o.font.as_deref(),
            o.bold_font.as_deref(),
            o.mono_font.as_deref(),
        )
    };
    let theme = load_theme(o)?;
    fonts()?;
    let mut pages = 0;
    for (name, mut p) in gui::sample_screens(|| fonts().unwrap_or_else(|_| Fonts::embedded())) {
        p.ui.set_theme(theme.clone());
        let (w, h) = p.ui.size();
        let mut rec = mpfree_ui::record::Recorder::new(w, h);
        let mut bytes = Vec::new();
        rec.begin_frame();
        p.ui.paint(&mut rec);
        rec.encode_frame(&mut bytes);
        p.tick(0.033);
        rec.begin_frame();
        p.ui.paint(&mut rec);
        rec.encode_frame(&mut bytes);
        let path = dir.join(format!("{name}.mpdl"));
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .and_then(|mut f| f.write_all(&bytes))
            .map_err(|e| format!("{}: {e}", path.display()))?;
        pages += 1;
    }
    println!("display lists written: {pages} pages in {}", dir.display());
    Ok(())
}

fn main() {
    // `mpfree plughost ...`: this program serving as one plugin's process
    // for the plugin sandbox (plugins.rs), before anything else starts.
    let mut raw = std::env::args_os().skip(1);
    if raw.next().is_some_and(|a| a == plugins::SUBCOMMAND) {
        std::process::exit(plugins::serve(raw));
    }
    // A panic says where and why in the report before the program ends
    // (crash.rs): from here on, for every command.
    crash::note_panics(
        Path::new(REPORT_DIR).join("report.txt"),
        Path::new(REPORT_DIR).join(crash::RUNNING),
    );
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = parse_args(&args).and_then(|o| {
        if o.preview.is_some() {
            return preview(&o);
        }
        if o.display_lists.is_some() {
            return display_lists(&o);
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
        if let Ok(mut f) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(Path::new(REPORT_DIR).join("report.txt"))
        {
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

    /// UNDO on the Synth tab and the mixer names the knob or fader.
    #[test]
    fn undo_of_a_turn_names_it() {
        assert_eq!(
            mix_turn_said(
                mpfree_fx::StripId::Track(mixing::DRUMS),
                mpfree_fx::ids::mixer::VOLUME,
                false
            ),
            "Undid Drums Volume"
        );
        let synth = synth::Synth::new(RATE);
        let p = (0..256u16).find_map(|id| synth.param_info(id)).unwrap();
        let said = synth_turn_said(&synth, p.id, true);
        assert!(
            said.starts_with("Redid ") && said.contains(p.name) && said.contains("(now "),
            "{said}"
        );
    }

    /// UNDO on the takes says what it took back, in place of the hint.
    #[test]
    fn undo_and_redo_say_what_they_did() {
        assert_eq!(
            undo_said("undo: take 3 · Drums / 1"),
            "Undid take 3 · Drums / 1"
        );
        assert_eq!(
            undo_said("redo: delete 6 · Bass / 2"),
            "Redid delete 6 · Bass / 2"
        );
        assert_eq!(
            undo_said("undo: nothing to undo"),
            "Nothing to undo in the takes"
        );
        assert_eq!(
            undo_said("redo: nothing to redo"),
            "Nothing to redo in the takes"
        );
        // ERASE + a clip, then UNDO: the clip is back, and the line says so.
        let mut out = Out {
            console: None,
            sink: None,
            dropped: 0,
            report: None,
            queued: None,
            last_draw: Instant::now(),
        };
        let mut time = time::TimeHost::new(RATE, 120.0).unwrap();
        let mut app = app::App::new(RATE, false);
        let mut mix = mixing::MixHost::new(RATE, -6.0, -6.0);
        let mut tracks = audiotracks::TrackHost::new(RATE, 1);
        let looper = looping::LoopHost::new(RATE, 1);
        time.put_clip(mpfree_seq::Clip::new("beat", 3840)).unwrap();
        let fx = time_action(
            &mut out,
            &mut time,
            &mut tracks,
            &looper,
            &mut app,
            &mut mix,
            TimeAction::DeleteClip { track: 0, slot: 0 },
        );
        assert!(
            matches!(fx.as_slice(), [app::Effect::Tell(l)] if l.contains("deleted")),
            "{fx:?}"
        );
        let fx = time_action(
            &mut out,
            &mut time,
            &mut tracks,
            &looper,
            &mut app,
            &mut mix,
            TimeAction::Undo,
        );
        assert!(
            matches!(fx.as_slice(), [app::Effect::Tell(l)]
                if l.starts_with("Undid delete ") && l.ends_with("Track 1 / 1")),
            "{fx:?}"
        );
        assert!(time.state().clip(0, 0).is_some());
    }

    /// TRACK MUTE's keys mute a track, or a pad in the kit, and the pads
    /// show it: green playing (dim with nothing launched), red muted,
    /// dark where there is nothing.
    #[test]
    fn the_mute_keys_mute_tracks_or_pads_and_light_what_plays() {
        use app::MuteKeys;
        let mut time = time::TimeHost::new(RATE, 120.0).unwrap();
        let mut mix = mixing::MixHost::new(RATE, -6.0, -6.0);
        assert_eq!(
            mute_key(MuteKeys::Tracks, 0, 2, &mut time, &mut mix),
            "track mute: Track 3 muted"
        );
        assert_eq!(time.muted(2), Some(true));
        assert_eq!(
            mute_key(MuteKeys::Tracks, 0, 9, &mut time, &mut mix),
            "track mute: no track 10"
        );
        // Bank B: pad 3 is track 19, once there is one.
        assert_eq!(
            mute_key(MuteKeys::Tracks, 1, 2, &mut time, &mut mix),
            "track mute: no track 19"
        );
        while time.track_count() < 24 {
            time.act(TimeAction::AddTrack).unwrap();
        }
        assert_eq!(
            mute_key(MuteKeys::Tracks, 1, 2, &mut time, &mut mix),
            "track mute: Track 19 muted"
        );
        assert_eq!(time.muted(18), Some(true));
        assert_eq!(time.muted(2), Some(true), "bank A's track 3 is still muted");
        assert_eq!(
            mute_key(MuteKeys::Pads, 0, 4, &mut time, &mut mix),
            "pad mute: pad 5 muted"
        );
        assert_eq!(mix.pad_mutes(), 1 << 4);
        assert_eq!(mix.play_levels().pad_mutes, 1 << 4, "the speakers show it");
        assert_eq!(
            mute_key(MuteKeys::Pads, 0, 4, &mut time, &mut mix),
            "pad mute: pad 5 plays"
        );
        assert_eq!(mix.pad_mutes(), 0);

        let kit = sound_colours(std::array::from_fn(|p| p < 12), 1);
        let (green, red) = (akai::KIT_COLOURS[3], akai::KIT_COLOURS[4]);
        let track = |t: usize| (t < 8).then_some((t == 2, t == 0));
        let c = mute_colours(MuteKeys::Tracks, kit, track, 0);
        assert_eq!(
            (c[0], c[1], c[2], c[8]),
            (akai::hit_colour(green), green, red, [0; 3])
        );
        let c = mute_colours(MuteKeys::Pads, kit, track, 1 << 4);
        assert_eq!((c[0], c[4], c[12]), (akai::hit_colour(kit[0]), red, [0; 3]));
    }

    #[test]
    fn the_tunings_folder_offers_scala_files_that_parse() {
        let d = std::env::temp_dir().join(format!("mpfree-tunings-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        let t = d.join("files/tunings");
        fs::create_dir_all(&t).unwrap();
        fs::write(d.join("FORMAT"), "mpfree-data 1\n").unwrap();
        // A Latin-1 description (0xe9 is e acute) still reads.
        let mut just = b"! just.scl\nJust intonation, by Zarlino \xe9\n12\n".to_vec();
        just.extend_from_slice(
            b"16/15\n9/8\n6/5\n5/4\n4/3\n45/32\n3/2\n8/5\n5/3\n9/5\n15/8\n2/1\n",
        );
        fs::write(t.join("just.scl"), just).unwrap();
        fs::write(
            t.join("dorian.scl"),
            "Dorian\n7\n9/8\n6/5\n4/3\n3/2\n5/3\n9/5\n2/1\n",
        )
        .unwrap();
        fs::write(
            t.join("dorian.kbm"),
            "12\n0\n127\n62\n69\n432.0\n7\n0\nx\n1\n2\nx\n3\nx\n4\nx\n5\n6\nx\n",
        )
        .unwrap();
        fs::write(t.join("broken.scl"), "desc\n2\n3/2\n").unwrap();
        fs::write(
            t.join("badmap.scl"),
            "Dorian\n7\n9/8\n6/5\n4/3\n3/2\n5/3\n9/5\n2/1\n",
        )
        .unwrap();
        fs::write(t.join("badmap.kbm"), "12\n0\n127\n60\n").unwrap();
        fs::write(t.join("lonely.kbm"), "0\n0\n127\n60\n69\n440.0\n0\n").unwrap();
        fs::write(t.join("readme.txt"), "not a tuning\n").unwrap();
        let store = mpfree_store::Store::at(&d, |_| {});
        let (found, notes) = load_tuning_files(&store);
        let names: Vec<(&str, bool)> = found
            .iter()
            .map(|t| (t.name.as_str(), t.mapped()))
            .collect();
        assert_eq!(names, vec![("dorian", true), ("just", false)]);
        for want in [
            "tunings: badmap.scl left out: its .kbm: ",
            "tunings: broken.scl left out: ",
            "tunings: lonely.kbm left out: no lonely.scl beside it",
            "tunings: found dorian (7 notes, with its .kbm), just (12 notes)",
        ] {
            assert!(
                notes.iter().any(|n| n.starts_with(want)),
                "{want}: {notes:?}"
            );
        }
        assert_eq!(notes.len(), 4, "{notes:?}");
        // The synth plays them: the map's A4 is 432 Hz.
        let mut s = synth::Synth::new(44_100);
        s.set_tuning_files(found);
        let mut set = s.settings();
        set.tuning.file = Some(0);
        s.set_settings(set);
        assert_eq!(s.tuning_label(), "dorian");
        // No store, no files.
        assert!(load_tuning_files(&mpfree_store::Store::none()).0.is_empty());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn the_themes_folder_offers_theme_files_that_parse() {
        let d = std::env::temp_dir().join(format!("mpfree-themes-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        let t = d.join("files/themes");
        fs::create_dir_all(&t).unwrap();
        fs::write(d.join("FORMAT"), "mpfree-data 1\n").unwrap();
        fs::write(
            t.join("sunrise.theme"),
            "base = synthwave\nname = Sunrise\n",
        )
        .unwrap();
        fs::write(t.join("plain.theme"), "base = console\n").unwrap();
        fs::write(t.join("broken.theme"), "accent = blue\n").unwrap();
        fs::write(t.join("notes.txt"), "not a theme\n").unwrap();
        let store = mpfree_store::Store::at(&d, |_| {});
        let (found, notes) = load_theme_files(&store);
        let names: Vec<(&str, &str)> = found
            .iter()
            .map(|(f, t)| (f.as_str(), t.name.as_str()))
            .collect();
        // Named like a built-in theme (Console, from its base): its file name.
        assert_eq!(
            names,
            vec![("plain.theme", "plain"), ("sunrise.theme", "Sunrise")]
        );
        assert!(
            notes
                .iter()
                .any(|n| n.starts_with("themes: broken.theme left out")),
            "{notes:?}"
        );
        assert!(
            notes.iter().any(|n| n == "themes: found plain, Sunrise"),
            "{notes:?}"
        );
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn arguments() {
        let o = parse_args(&args("pads --seconds 8 --keepalive")).unwrap();
        assert_eq!(
            (o.command.as_str(), o.seconds, o.keepalive),
            ("pads", 8, true)
        );
        assert!(parse_args(&args("click --keepalive")).is_err());
        assert!(parse_args(&args("click --gain-db -3")).is_err());
        assert!(parse_args(&args("click --seconds 61")).is_err());
        assert!(parse_args(&args("click --seconds 0")).is_err());
        assert!(parse_args(&args("format")).is_err());
        assert!(parse_args(&args("")).is_err());
        assert!(parse_args(&args("pads click")).is_err());
        let o = parse_args(&args("app --patterns /tmp/p.txt")).unwrap();
        assert_eq!(o.patterns, Some(PathBuf::from("/tmp/p.txt")));
        assert!(
            parse_args(&args("app --save-patterns")).is_err(),
            "the store saves patterns now"
        );
        assert!(parse_args(&args("play --seconds 5 --patterns /tmp/p.txt")).is_err());
        // MIDI Out opens only when asked, and only for the app.
        assert!(!parse_args(&args("app")).unwrap().midi_out);
        assert!(parse_args(&args("app --midi-out")).unwrap().midi_out);
        assert!(parse_args(&args("play --seconds 5 --midi-out")).is_err());
        assert!(!parse_args(&args("app")).unwrap().cv_out);
        assert!(parse_args(&args("app --cv-out")).unwrap().cv_out);
        assert!(parse_args(&args("pads --cv-out")).is_err());
        // The GPU draws the screen only when asked, for the pad player.
        assert_eq!(parse_args(&args("app")).unwrap().gpu_screen, None);
        let o = parse_args(&args("app --gpu-screen /run/mpfree-app/mpfree-gpu-screen")).unwrap();
        assert_eq!(
            o.gpu_screen,
            Some(PathBuf::from("/run/mpfree-app/mpfree-gpu-screen"))
        );
        assert!(parse_args(&args("play --seconds 5 --gpu-screen g")).is_ok());
        assert!(parse_args(&args("pads --gpu-screen g")).is_err());
        assert!(parse_args(&args("app --gpu-screen")).is_err());
        // Where the sound runs: Prefs' setting unless asked for this run, and
        // only for the pad player.
        assert_eq!(parse_args(&args("app")).unwrap().sound_cores, None);
        let o = parse_args(&args("play --seconds 120 --sound-cores two")).unwrap();
        assert_eq!(o.sound_cores, Some(cores::SoundCores::Two));
        let o = parse_args(&args("app --sound-cores core3")).unwrap();
        assert_eq!(o.sound_cores, Some(cores::SoundCores::Core3));
        let o = parse_args(&args("app --sound-cores three")).unwrap();
        assert_eq!(o.sound_cores, Some(cores::SoundCores::Three));
        assert!(parse_args(&args("app --sound-cores 2")).is_err());
        assert!(parse_args(&args("app --sound-cores")).is_err());
        assert!(parse_args(&args("click --seconds 5 --sound-cores two")).is_err());
    }

    #[test]
    fn saved_patterns_go_through_the_store_and_a_patterns_file_is_only_read() {
        let dir = std::env::temp_dir().join(format!("mpfree-patterns-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        // What Save hands the store comes back at the next start.
        let store = mpfree_store::Store::at(&dir, |_| {});
        assert!(store.saves());
        assert_eq!(
            store.read(app::saved::ARP, app::saved::ARP_PATTERNS),
            None,
            "nothing saved yet"
        );
        store
            .write(
                app::saved::ARP,
                app::saved::ARP_PATTERNS,
                b"Skip = 1 . 3\n".to_vec(),
            )
            .unwrap();
        store.flush();
        let again = mpfree_store::Store::at(&dir, |_| {});
        assert_eq!(
            again
                .read(app::saved::ARP, app::saved::ARP_PATTERNS)
                .as_deref(),
            Some(&b"Skip = 1 . 3\n"[..])
        );
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
        assert_eq!(
            c,
            vec![
                (
                    0,
                    "MIDI".to_string(),
                    "USB-Audio - MPC One MIDI".to_string()
                ),
                (1, "ACVA".to_string(), "jp07-audio - ACVA".to_string())
            ]
        );
    }

    #[test]
    fn touches_follow_slots() {
        let mut t = Touches::new();
        let feed = |t: &mut Touches, evs: &[(u16, u16, i32)]| {
            evs.iter().map(|&(a, b, c)| t.feed(a, b, c)).last().unwrap()
        };
        assert!(feed(
            &mut t,
            &[
                (3, 0x2f, 0),
                (3, 0x39, 7),
                (3, 0x35, 100),
                (3, 0x36, 200),
                (0, 0, 0)
            ]
        ));
        assert_eq!(t.describe(), "1 contact(s): slot 0: x 100 y 200");
        assert!(feed(
            &mut t,
            &[
                (3, 0x2f, 1),
                (3, 0x39, 8),
                (3, 0x35, 300),
                (3, 0x36, 400),
                (0, 0, 0)
            ]
        ));
        assert_eq!(
            t.describe(),
            "2 contact(s): slot 0: x 100 y 200, slot 1: x 300 y 400"
        );
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
        assert_eq!(
            describe_event(&ControlEvent::PadDown {
                pad: 0,
                velocity: 90
            }),
            "pad 1 down, velocity 90"
        );
        assert_eq!(
            describe_event(&ControlEvent::QLinkTurn { knob: 2, delta: -1 }),
            "Q-Link 3 turned -1"
        );
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
        let known = [
            0xF0, 0x47, 0x7F, 0x46, 0x53, 0x00, 0x05, 0x7D, 0x6D, 0x57, 0x35, 0x01, 0xF7,
        ];
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
        let mut reply = vec![
            0xF0, 0x7E, 0x00, 0x06, 0x02, 0x47, 0x46, 0x00, 0x19, 0x00, 0x00, 0x01, 0x00, 0x7F,
            0x00, 0x00, 0x00, 0x00,
        ];
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
        assert_eq!(
            lines_for("Public", &input),
            vec![
                "+1.500 s Public: 90 54 7F = Q-Link 1 touched",
                "+1.500 s Public: B0 10 35 = Q-Link 1 at 53 (absolute)",
                "+1.500 s Public: B0 10 34 = Q-Link 1 at 52 (absolute)",
            ]
        );
        // On Private (with Akai's setup) they are steps.
        assert_eq!(
            lines_for("Private", &[0xB0, 0x10, 0x02]),
            vec!["+1.500 s Private: B0 10 02 = Q-Link 1 turned +2"]
        );
    }

    #[test]
    fn marks_long_sysex_as_cut_short() {
        let mut input = vec![0xF0];
        input.extend(std::iter::repeat_n(0x11, 100));
        input.push(0xF7);
        let lines = lines_for("Public", &input);
        assert_eq!(lines.len(), 1);
        assert!(
            lines[0].contains("(cut short: longer than the parser keeps) = not recognised"),
            "{}",
            lines[0]
        );
    }

    #[test]
    fn port_names_match_the_controller() {
        assert_eq!(MIDI_PORTS.map(|p| p.0), [0, 1, 2]);
        assert_eq!(MIDI_PORTS[mpc_one::Port::Public as usize].1, "Public");
        assert_eq!(MIDI_PORTS[mpc_one::Port::Private as usize].1, "Private");
        // The 5-pin MIDI In is the third cable, and only it is music.
        assert_eq!(MIDI_PORTS[MUSIC_CABLE as usize].1, "MIDI Port");
        assert!(pad_note_names().starts_with("pad 1 C#2, pad 2 C2"));
    }

    #[test]
    fn gap_statistics() {
        let mut g = Gaps::default();
        assert_eq!(g.mean_max(), (0.0, 0.0));
        g.add(Duration::from_millis(2));
        g.add(Duration::from_millis(4));
        let (avg, max) = g.mean_max();
        assert!((avg - 0.003).abs() < 1e-9 && (max - 0.004).abs() < 1e-9);
    }

    /// Keeping the gaps never allocates, however long the sound plays
    /// (past 2^20 periods, where a growing list of them used to be copied
    /// on the audio thread).
    #[test]
    fn keeping_the_gaps_never_allocates() {
        let mut g = Gaps::default();
        let gap = Duration::from_micros(2902);
        let ((), calls) = crate::sys::alloc_count::calls(|| {
            for _ in 0..(1u32 << 20) + 8 {
                g.add(gap);
            }
        });
        assert_eq!(calls, 0);
        assert_eq!(g.n, (1 << 20) + 8);
        let (avg, max) = g.mean_max();
        assert!((avg - 0.002902).abs() < 1e-9 && (max - 0.002902).abs() < 1e-12);
    }

    /// A learned control's 0 to 1 sets a mixer value over its own range,
    /// as a chain editor knob does; one that reaches nothing says so (M5).
    #[test]
    fn a_learned_control_sets_its_value_over_the_range() {
        use mpfree_fx::ids::mixer as mx;
        use mpfree_fx::StripId;
        use mpfree_midi::learn::Write;
        use mpfree_play::mpfree_param::{Owner, ParamAddr, Strip};
        const RATE: u32 = 44_100;
        let mut mix = mixing::MixHost::new(RATE, 0.0, 6.0);
        let mut panels = modpanels::Panels::new();
        let mut at = 0u64;
        let mut run = |mix: &mut mixing::MixHost, periods: u64| {
            let z = [0f32; PERIOD];
            let (mut l, mut r) = ([0f32; PERIOD], [0f32; PERIOD]);
            for _ in 0..periods {
                let ctx = mpfree_node::Ctx::new(RATE, at * PERIOD as u64);
                mix.period(&ctx, (&z, &z), (&z, &z), (&z, &z), (&z, &z), &mut l, &mut r);
                at += 1;
            }
        };
        run(&mut mix, 20);
        let track = mixing::LOOP;
        let vol = ParamAddr::new(Owner::Strip(Strip::Track(track as u8)), mx::VOLUME);
        let (info, _) = modmix::describe(&mix.mixer, vol).unwrap();
        for (unit, want) in [(0.0, info.min), (1.0, info.max), (0.5, info.from_unit(0.5))] {
            let w = Write {
                target: vol,
                value: unit,
            };
            assert_eq!(learned_set(&mut panels, &mut mix, w, false), Some(want));
            run(&mut mix, 20);
            let got = mix.mixer.param(StripId::Track(track), mx::VOLUME);
            assert!((got - want).abs() < 0.01, "{unit}: {got} != {want}");
        }
        let nothing = Write {
            target: ParamAddr::new(Owner::Strip(Strip::Track(track as u8)), 9999),
            value: 1.0,
        };
        assert_eq!(learned_set(&mut panels, &mut mix, nothing, false), None);
    }
}
