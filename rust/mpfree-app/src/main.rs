//! mpfree, the app (Phase 1 called it "Hello MPC"), for the kexec test boot from the USB stick.
//!
//! One program, one step per run, paced from the workstation by
//! tools/mpc-one/host_app_mpc.py:
//!
//!   inventory          read-only: sound cards, PCM and MIDI devices, the
//!                      sample rates the audio driver would accept (asked,
//!                      never set), input devices, framebuffer, panel
//!                      rotation, RT limits
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
//!   play --seconds N [--gain-db G] [--kits FILE] [--sound-cores MODE] [--time-thread]
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
//!       [--time-thread]
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
//!                      mpfree-gpu-bench, which times them on the MPC's GPU);
//!                      with --theme all, once per built-in theme, in
//!                      DIR/THEME/NAME.mpdl (for the contrast check)
//!
//! `--sound-cores shared|core3|two|three` (play and app) picks where the
//! sound runs for this run, in place of Prefs' saved "Sound cores" (shared
//! by default): as before, the audio thread alone on core 3, core 3 plus
//! lanes on core 2 for half the drum voices and part of the mixer's
//! inserts, or (a test) those plus the synth on a lane on core 1 (cores.rs).
//! It changes only this program's threads.
//!
//! `--time-thread` (play and app) runs the time engine's editing side (the
//! clips, the song, the takes and every compile) on a thread of its own
//! instead of the audio thread (Arrange 2.6b, time-threads.md "Apart"). Off
//! until a device round has compared the two.
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
mod bundled;
mod calibrate_page;
mod chain_page;
mod control;
mod controller_out;
mod cores;
mod crash;
mod cv_out;
mod debug_page;
pub mod demo_songs;
mod desk;
mod drawing;
mod engines;
mod export;
mod expr_rec;
mod fxmodes;
mod gui;
mod hand;
mod hearing;
mod heat;
mod imagecopy;
mod instrument_list;
mod instruments;
mod kernel_log;
mod kit;
mod locks;
mod loop_row;
mod looper_deck;
mod looper_page;
mod looper_tab;
mod looping;
mod loops_on_tracks;
mod memory;
mod midi_file;
mod midi_in;
mod midi_log;
mod midi_out;
mod midi_saved;
mod mixing;
mod mixturns;
mod modmix;
mod modpanels;
mod one_undo;
mod open_clock;
mod pace;
mod pad_synths;
mod parts;
mod patch_in;
mod patch_page;
mod patch_screen;
mod perf;
mod perform_pads;
mod perform_tab;
mod play_page;
mod play_widgets;
mod plugin_page;
mod plugin_store;
mod plugins;
mod port_writer;
mod power;
mod presets;
mod preview;
mod project;
mod qlink_touch;
mod rate_probe;
mod rec_arm;
mod rec_list;
mod remote;
#[cfg(test)]
mod rt_safety;
mod sample_page;
mod sampler;
mod sampler_commands;
mod screen_cost;
mod screen_picture;
mod script;
// The SD card's read-only window, the pad player's side, for the Browser's "SD card" place.
mod sd_card;
mod set_checks;
mod set_play;
mod setup;
mod shed;
mod show;
mod skin_page;
#[cfg(test)]
mod smoke;
mod song_end;
mod soundcheck;
mod status_report;
mod status_screen;
mod status_watch;
mod step_keys;
mod steps_continue;
mod synth;
mod sys;
mod sysinfo;
mod takes;
mod time;
mod touch_bell;
mod touch_dev;
mod touchlab_page;
mod track_costs;
mod tunings;
mod undo;
mod views;
mod waiting;
mod xy_layouts;
mod xy_list;
mod xy_own;
mod xy_play;
mod xy_rec;
mod xy_setup;
mod xy_slot;
mod xy_song;
mod xy_widgets;

use mpfree_arp::scale::{pads::MAX_PAD_NOTES, Shift};
use mpfree_audiotracks::host::CueAudio;
use mpfree_controller::midi::{Message, Parser};
use mpfree_controller::mpc_one::{self, ControlEvent, OBSERVED_KEEPALIVE_TOKEN};
use mpfree_core::{float_to_s32, ClickConfig, ClickEngine};
use mpfree_midi::monitor::{Dir, Event, Mpc, To};
use mpfree_midi::profile::{self, KIT_PADS};
use mpfree_screen_log::Rotation;
use mpfree_synth_ui::{SynthAction, SynthState};
use mpfree_time_ui::continuing::ContinueAction;
use mpfree_time_ui::view::AudioAction;
use mpfree_time_ui::{TempoEdit, TimeAction, TransportInfo};
use mpfree_transport::Command;
use mpfree_ui::fb::FbTarget;
use mpfree_ui::{Fonts, SoftCanvas, Theme, Tone};
use std::fs::{self, File, OpenOptions};
use std::io::{self, ErrorKind, Read, Write};
use std::ops::Range;
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
    /// `--gain-db`: the top of the level (play, app) or the level itself
    /// (drums, click); see [`Options::gain_db`].
    gain_db: Option<f32>,
    fb: PathBuf,
    sysfs: PathBuf,
    preview: Option<PathBuf>,
    display_lists: Option<PathBuf>,
    /// `--check-theme FILE`: a theme file's problems and its unreadable
    /// text on every sample screen ([`check_theme`]).
    check_theme: Option<PathBuf>,
    /// `--contact-sheet DIR`: every sample screen in `--theme` beside the
    /// theme it starts from, with a page to look at them ([`contact_sheet`]).
    contact_sheet: Option<PathBuf>,
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
    /// The time engine's control half on a thread of its own (`play` and
    /// `app`; Arrange 2.6b). Off until a device round has compared it.
    time_thread: bool,
    /// Draw the screen on the GPU, in this program of its own
    /// (`mpfree-gpu-screen`; `play` and `app`; drawing.rs). Off until a
    /// device round has checked it; any failure draws in software again.
    gpu_screen: Option<PathBuf>,
    /// The power button (`app` only; power.rs): a short press opens the
    /// power card, and Shut down or Restart writes its word to this file
    /// for the system to act on (the stick's root path unit). Without it
    /// the button is not read.
    power_request: Option<PathBuf>,
    /// A workflow script (`play` only; script.rs): the app plays its steps
    /// by itself, as the device round's scripted runs do, and the run lasts
    /// as long as the script. Read in `main`, into `workflow`.
    script: Option<PathBuf>,
    workflow: Option<script::Script>,
}

/// The loudest the drums and click checks play (`--gain-db` there): their
/// sounds peak at most this far below full scale.
const TEST_TOP_DB: f32 = -6.0;
/// The drums and click checks' level without `--gain-db`.
const TEST_GAIN_DB: f32 = -12.0;
/// The top of the pad player's level without `--gain-db`: 0 dB, as the
/// mixer's master fader and Akai's own. It starts 6 dB under, at the
/// browser's audition level ([`preview::LEVEL_DB`]), so a sound heard in
/// the browser plays as loud on the pads. Until round 4 the top was
/// -12 dB, a first-test precaution, and pads played 12 dB under the
/// audition (Jay: "much quieter" on Play).
const PLAYER_TOP_DB: f32 = 0.0;

impl Options {
    /// The pad player's top level (play, app), or the checks' level
    /// (drums, click).
    fn gain_db(&self) -> f32 {
        let default = if self.command == "app" || self.command == "play" {
            PLAYER_TOP_DB
        } else {
            TEST_GAIN_DB
        };
        self.gain_db.unwrap_or(default)
    }
}

fn parse_args(args: &[String]) -> Result<Options, String> {
    let mut o = Options {
        command: String::new(),
        seconds: 10,
        keepalive: false,
        gain_db: None,
        fb: PathBuf::from("/dev/fb0"),
        sysfs: PathBuf::from("/sys/class/graphics/fb0"),
        preview: None,
        display_lists: None,
        check_theme: None,
        contact_sheet: None,
        font: None,
        bold_font: None,
        mono_font: None,
        theme: None,
        kits: PathBuf::from(KIT_PATH),
        keep_screen: false,
        patterns: None,
        data: None,
        sound_cores: None,
        time_thread: false,
        gpu_screen: None,
        power_request: None,
        script: None,
        workflow: None,
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
                let db: f32 = value()?
                    .parse()
                    .map_err(|_| "--gain-db needs a number".to_string())?;
                if !(-40.0..=0.0).contains(&db) {
                    return Err("--gain-db must be between -40 and 0".to_string());
                }
                o.gain_db = Some(db);
            }
            "--fb" => o.fb = PathBuf::from(value()?),
            "--sysfs" => o.sysfs = PathBuf::from(value()?),
            "--preview" => o.preview = Some(PathBuf::from(value()?)),
            "--display-lists" => o.display_lists = Some(PathBuf::from(value()?)),
            "--check-theme" => o.check_theme = Some(PathBuf::from(value()?)),
            "--contact-sheet" => o.contact_sheet = Some(PathBuf::from(value()?)),
            "--font" => o.font = Some(PathBuf::from(value()?)),
            "--bold-font" => o.bold_font = Some(PathBuf::from(value()?)),
            "--mono-font" => o.mono_font = Some(PathBuf::from(value()?)),
            "--theme" => o.theme = Some(value()?),
            "--kits" => o.kits = PathBuf::from(value()?),
            "--keep-screen" => o.keep_screen = true,
            "--time-thread" => o.time_thread = true,
            "--patterns" => o.patterns = Some(PathBuf::from(value()?)),
            "--data" => o.data = Some(PathBuf::from(value()?)),
            "--gpu-screen" => o.gpu_screen = Some(PathBuf::from(value()?)),
            "--power-request" => o.power_request = Some(PathBuf::from(value()?)),
            "--script" => o.script = Some(PathBuf::from(value()?)),
            "--sound-cores" => {
                o.sound_cores =
                    Some(cores::SoundCores::parse(&value()?).ok_or_else(|| {
                        format!("--sound-cores needs {}", cores::SoundCores::NAMES)
                    })?)
            }
            _ => return Err(format!("unknown argument {arg}")),
        }
    }
    if o.preview.is_none()
        && o.display_lists.is_none()
        && o.check_theme.is_none()
        && o.contact_sheet.is_none()
        && o.command.is_empty()
    {
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
    if o.script.is_some() {
        if o.command != "play" {
            return Err("--script goes with play".to_string());
        }
        if args.iter().any(|a| a == "--seconds") {
            return Err("a script sets its own length; --script takes no --seconds".to_string());
        }
    } else if o.command == "app" {
        if args.iter().any(|a| a == "--seconds") {
            return Err("app runs until the power goes off; it takes no --seconds".to_string());
        }
    } else if !(1..=max).contains(&o.seconds) {
        return Err(format!("--seconds must be 1 to {max}"));
    }
    if o.gain_db.is_some_and(|db| db > TEST_TOP_DB) && o.command != "app" && o.command != "play" {
        return Err(format!(
            "--gain-db must be between -40 and {TEST_TOP_DB} for {}",
            o.command
        ));
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
    if o.power_request.is_some() && o.command != "app" {
        return Err("--power-request goes with app".to_string());
    }
    if o.data.is_some() && o.command != "app" && o.script.is_none() {
        return Err("--data goes with app, or with play and --script".to_string());
    }
    if args.iter().any(|a| a == "--sound-cores") && o.command != "app" && o.command != "play" {
        return Err("--sound-cores goes with app or play".to_string());
    }
    if o.gpu_screen.is_some() && o.command != "app" && o.command != "play" {
        return Err("--gpu-screen goes with app or play".to_string());
    }
    if o.time_thread && o.command != "app" && o.command != "play" {
        return Err("--time-thread goes with app or play".to_string());
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

    /// Something of the project just opened that the audio loop couldn't
    /// put back, for the screen ([`gui::ToUi::LeftOut`]): the project
    /// isn't saved over. Allocates nothing more than the line already did.
    fn left_out(&mut self, what: String) {
        if let Some(s) = self.sink.as_ref() {
            if s.try_send(gui::ToUi::LeftOut(what)).is_err() {
                self.dropped += 1;
            }
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

    /// What a loop that starts when you play said ([`LoopStart`]): the
    /// log, and the screen too for a start refused.
    fn loop_start(&mut self, said: LoopStart) {
        if let Some((line, tell)) = said {
            self.note(&line);
            if tell {
                self.show(gui::ToUi::Tell(line));
            }
        }
    }

    /// What smart loop end made of a closing tap
    /// ([`looping::LoopHost::take_smart`]): the log, and the screen when
    /// your tap stood.
    fn smart_end(&mut self, said: Option<(String, Option<String>)>) {
        if let Some((line, tell)) = said {
            self.note(&line);
            if let Some(words) = tell {
                self.show(gui::ToUi::Tell(words));
            }
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

/// What the themes folder holds, read at start ([`load_theme_files`]).
#[derive(Default)]
struct ThemeFolder {
    /// The file name and theme of each theme offered, in name order.
    found: Vec<(String, Theme)>,
    /// Each offered file's problems, as Prefs > Appearance lists them:
    /// lines it couldn't use, unknown keys, ignored lines, colours hard to
    /// read.
    problems: Vec<(String, Vec<String>)>,
    /// Files left out, and why.
    left_out: Vec<(String, String)>,
    /// Lines of offered files that couldn't be used (a mistake, an unknown
    /// key): each used the base theme's value, a fallback.
    lines_fell_back: u64,
    /// Lines for the log.
    notes: Vec<String>,
}

/// The themes folder's `NAME.theme` files, in name order (at most
/// [`MAX_THEME_FILES`]), each read the forgiving way ([`Theme::load`]): a
/// line with a mistake uses the base theme's value, an unknown key is
/// skipped, and each such line is logged, counted and kept for Prefs >
/// Appearance to list (owner, 22 Nov 07:17Z). Only a theme no one could
/// read ([`Theme::unusable`]), or a file that isn't text, is left out, with
/// a line saying why. A theme without its own `name`, or named like a
/// built-in one, is shown by its file name. Only reads.
fn load_theme_files(store: &mpfree_store::Store) -> ThemeFolder {
    use mpfree_ui::theme::ProblemKind;
    let mut folder = ThemeFolder::default();
    for file in store.list(app::THEMES_FOLDER) {
        let Some(stem) = file.strip_suffix(".theme").filter(|s| !s.is_empty()) else {
            continue;
        };
        if folder.found.len() == MAX_THEME_FILES {
            folder.notes.push(format!(
                "themes: only the first {MAX_THEME_FILES} theme files are offered"
            ));
            break;
        }
        let Some(text) = store
            .read(app::THEMES_FOLDER, &file)
            .and_then(|b| String::from_utf8(b).ok())
        else {
            folder
                .notes
                .push(format!("themes: {file} left out: not readable text"));
            folder
                .left_out
                .push((file, "not readable text".to_string()));
            continue;
        };
        let (mut t, problems) = Theme::load(&text);
        if let Some(why) = problems
            .iter()
            .find(|p| p.kind == ProblemKind::Unusable)
            .map(|p| p.text.clone())
        {
            folder.notes.push(format!("themes: {file} left out: {why}"));
            folder.left_out.push((file, why));
            continue;
        }
        if app::THEMES
            .iter()
            .any(|b| b.1.eq_ignore_ascii_case(t.name.trim()))
            || t.name.trim().is_empty()
        {
            t.name = stem.to_string();
        }
        let mut lines = Vec::new();
        for p in &problems {
            let kind = match p.kind {
                ProblemKind::Mistake | ProblemKind::Unknown => {
                    folder.lines_fell_back += 1;
                    "used the base for"
                }
                ProblemKind::Ignored => "ignored",
                ProblemKind::HardToRead => "hard to read",
                ProblemKind::Unusable => unreachable!("left out above"),
            };
            folder.notes.push(format!("themes: {file}: {kind}: {p}"));
            lines.push(p.to_string());
        }
        if !lines.is_empty() {
            folder.problems.push((file.clone(), lines));
        }
        folder.found.push((file, t));
    }
    if !folder.found.is_empty() {
        let names: Vec<&str> = folder.found.iter().map(|(_, t)| t.name.as_str()).collect();
        folder
            .notes
            .push(format!("themes: found {}", names.join(", ")));
    }
    folder
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
        Ok(n) => {
            out.say(&format!("audio card for the click: {n} ({AUDIO_CARD_ID})"));
            // DSP quality step 0.3: which rates the driver would accept,
            // asked with HW_REFINE only (rate_probe.rs keeps the rules).
            for dir in ['p', 'c'] {
                for line in rate_probe::probe_path(&format!("/dev/snd/pcmC{n}D0{dir}")) {
                    out.say(&line);
                }
            }
        }
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

/// Opens MIDI out and CV out (`on`), or closes them without waiting: Prefs'
/// "Send MIDI and CV out". Each says in the log what it did; a port that
/// won't open is said once and left closed until the switch goes off and
/// on again (or, for a USB device, until it is plugged in again). Switched
/// on again within a moment, each takes back the writer it was letting go,
/// rather than opening its port a second time. Opening the jacks and CV
/// runs on the sound loop, once per switch: a period may run late then,
/// which the log's underrun count would show. Each USB device's output is
/// opened by its own writer thread ([`usb_output`]).
fn apply_sends(
    on: bool,
    midi_out: &mut midi_out::MidiOut,
    cv_out: &mut cv_out::CvOut,
    private: Option<&File>,
    started: Instant,
    out: &mut Out,
) {
    if !on {
        if midi_out.sending() || cv_out.is_open() {
            midi_out.switch_off();
            cv_out.switch_off();
            out.midi(
                "MIDI and CV out off (Prefs): notes on ended, clock stopped, CV 1 to 4 at 0 V; \
                 tracks play inside MPFree",
            );
        }
        return;
    }
    // USB devices plugged in: each writer takes back its open output or
    // opens it, and says so in the log.
    midi_out.switch_on(started);
    if midi_out.reopen() {
        out.midi("MIDI Out on again (Prefs): the port was still open");
    } else if !midi_out.is_open() {
        match midi_card().and_then(|card| open_output(card, MUSIC_CABLE, "MIDI Port", out)) {
            Ok(port) => match midi_out.open(port, started) {
                Ok(()) => out.midi(&format!(
                    "MIDI Out open (Prefs): tracks set to play on MIDI play there, {:.1} ms \
                     after their frame is made (the sound's lead), less the jacks' gear delay",
                    MIDI_OUT_LEAD.as_secs_f64() * 1000.0
                )),
                Err(e) => out.midi(&format!("MIDI Out not started: {e}")),
            },
            Err(e) => out.midi(&format!(
                "MIDI Out not opened: {e}; tracks set to play on MIDI play on the pads \
                 (turn Send MIDI and CV out off and on to try again)"
            )),
        }
    }
    if cv_out.reopen() {
        out.midi("CV out on again (Prefs): CV 1 to 4 follow the project's CV outputs");
    } else if !cv_out.is_open() {
        match private.map(File::try_clone) {
            Some(Ok(port)) => match cv_out.open(port, started) {
                Ok(()) => out.midi(&format!(
                    "CV out open (Prefs): CV 1 to {} follow the project's CV outputs, {:.1} ms \
                     after their frame is made; CV {} to 8 wait for a check",
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
    /// Debug mode: the lights, pad colours and pings written, for the MIDI
    /// monitor (taken each period); nothing is kept while it is off.
    tap_on: bool,
    tapped: Vec<(f64, mpfree_midi::monitor::Mpc)>,
}

/// What [`Ports::tapped`] keeps between two periods; more are not listed.
const TAPPED: usize = 128;

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
            tap_on: false,
            tapped: Vec::with_capacity(TAPPED),
        })
    }

    /// Debug mode: a message written to the controller, for the MIDI
    /// monitor (`akai::monitor_out` names it). Never allocates.
    fn tap(&mut self, private: bool, msg: &[u8]) {
        if !self.tap_on || self.tapped.len() >= TAPPED {
            return;
        }
        if let Some(m) = akai::monitor_out(private, msg) {
            let t = self.t();
            self.tapped.push((t, m));
        }
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

    /// `when` on the same clock as [`Ports::t`].
    fn at(&self, when: Instant) -> f64 {
        when.saturating_duration_since(self.start).as_secs_f64()
    }

    fn line(&self, out: &mut Out, text: &str) {
        if self.quiet {
            out.note(text);
        } else {
            out.say(text);
        }
    }

    fn send(&mut self, private: bool, msg: &[u8]) -> Result<(), String> {
        self.tap(private, msg);
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

/// A USB-MIDI device's port the scanner thread found, its input opened
/// for reading if it has one.
struct UsbFound {
    card: u32,
    port: midi_in::UsbPort,
    file: Option<File>,
}

/// What the scanner thread tells the audio loop.
enum Scan {
    Found(UsbFound),
    /// A card went from `/proc/asound/cards`: its ports go, the ones
    /// without an input too.
    Gone(u32),
    /// A line for the MIDI log.
    Said(String),
}

/// One port of a USB device in the audio loop: read when it has an input.
struct UsbIn {
    card: u32,
    port: mpfree_midi::port::PortId,
    file: Option<File>,
}

/// USB-MIDI devices on the USB-A socket. A thread looks at
/// `/proc/asound/cards` every two seconds and, for each new MIDI card,
/// opens every input of its first MIDI device, read only, and counts its
/// outputs from `/proc/asound/cardN/midi0` (never opening one); each input
/// is a port of its own, and so is each output past the inputs
/// (`midi_in::usb_ports`). The audio loop reads them without blocking, as
/// it reads the controller, and hands each port with an output to MIDI
/// out (`MidiOut::plugged`), whose writer opens it only while Prefs' "Send
/// MIDI and CV out" is on ([`usb_output`]). A read error (the device was
/// unplugged) or the card leaving the list closes its ports, lets go of
/// their keys and tells MIDI out. A card that can't be opened is logged
/// once and tried again only after it goes away and comes back.
struct UsbKeys {
    found: Receiver<Scan>,
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

/// Opens every input of `card`'s first MIDI device, read only: the files
/// and ALSA's name for each, in order. Stops at the first that won't open
/// (said in `said`).
fn usb_inputs(card: u32, name: &str, said: &mut Vec<String>) -> Vec<(File, String)> {
    let mut got = Vec::new();
    let mut count = 1;
    let mut k = 0;
    while k < count {
        match alsa::open_rawmidi(card, k, alsa::Direction::Input) {
            Ok((file, info))
                if info.stream == alsa::RAWMIDI_STREAM_INPUT && info.subdevice == k =>
            {
                if k == 0 {
                    count = info.subdevices.clamp(1, MAX_USB_SUBS);
                }
                got.push((file, info.subname));
            }
            Ok(_) => {
                if k > 0 {
                    said.push(format!(
                        "USB MIDI {name} (card {card}): input {} is not what ALSA said; the rest not used",
                        k + 1
                    ));
                }
                break;
            }
            Err(e) => {
                if k > 0 || e.kind() != ErrorKind::NotFound {
                    said.push(format!(
                        "USB MIDI {name} (card {card}): could not open input {}: {e}",
                        k + 1
                    ));
                }
                break;
            }
        }
        k += 1;
    }
    got
}

/// The most inputs read from one USB device.
const MAX_USB_SUBS: u32 = 8;

/// How MIDI out opens output `sub` of USB card `card` (one the scanner
/// found: never the MPC's own controller or sound card), in its writer's
/// thread. **State-changing on the gear** (notes, clock, program changes:
/// RAM only, never SysEx), so MIDI out calls it only while Prefs' "Send
/// MIDI and CV out" is on. The substream ALSA opened is checked before
/// anything is written; another one is closed unwritten.
fn usb_output(card: u32, sub: u32) -> midi_out::Opener {
    std::sync::Arc::new(move || {
        let (file, info) = alsa::open_rawmidi(card, sub, alsa::Direction::Output)?;
        if info.stream != alsa::RAWMIDI_STREAM_OUTPUT || info.subdevice != sub {
            return Err(io::Error::other(format!(
                "card {card}: ALSA opened output {} (\"{}\"), not output {}; nothing sent",
                info.subdevice + 1,
                info.subname,
                sub + 1
            )));
        }
        Ok(Box::new(file) as Box<dyn Write + Send>)
    })
}

impl UsbKeys {
    /// Start the scanner. `skip` holds the MPC's own cards (the
    /// controller and the sound card).
    fn start(skip: Vec<u32>) -> UsbKeys {
        let (tx, rx) = sync_channel(32);
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
                    for card in &seen {
                        if !now.iter().any(|(c, _)| c == card)
                            && tx.send(Scan::Gone(*card)).is_err()
                        {
                            return;
                        }
                    }
                    for (card, name) in &now {
                        if seen.contains(card) {
                            continue;
                        }
                        let mut said = Vec::new();
                        let inputs = usb_inputs(*card, name, &mut said);
                        let outputs = midi_in::proc_midi_outputs(
                            &fs::read_to_string(format!("/proc/asound/card{card}/midi0"))
                                .unwrap_or_default(),
                        );
                        let subnames: Vec<String> = inputs.iter().map(|i| i.1.clone()).collect();
                        let ports = midi_in::usb_ports(name, &subnames, outputs);
                        if ports.is_empty() {
                            said.push(format!(
                                "USB MIDI {name} (card {card}): no input or output found; not used"
                            ));
                        }
                        let mut files: Vec<Option<File>> =
                            inputs.into_iter().map(|i| Some(i.0)).collect();
                        let mut msgs: Vec<Scan> = said.into_iter().map(Scan::Said).collect();
                        for port in ports {
                            let file = port
                                .input
                                .and_then(|k| files.get_mut(k as usize))
                                .and_then(Option::take);
                            msgs.push(Scan::Found(UsbFound {
                                card: *card,
                                port,
                                file,
                            }));
                        }
                        for m in msgs {
                            if tx.send(m).is_err() {
                                return;
                            }
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

    /// Closes the port at `k` in `open`: its keys let go, the monitor
    /// told, MIDI out forgets it.
    fn close(
        &mut self,
        k: usize,
        music: &mut midi_in::MusicIn,
        midi_out: &mut midi_out::MidiOut,
        t: f64,
        keys: &mut Vec<midi_in::Key>,
    ) {
        let u = self.open.swap_remove(k);
        midi_out.unplugged(u.port);
        let ended = music.unplugged(u.port, keys);
        music.feed.event(
            t,
            Event::Unplugged {
                port: u.port,
                ended,
            },
        );
    }

    /// Take newly found ports and read every open one. Never blocks.
    fn poll(
        &mut self,
        music: &mut midi_in::MusicIn,
        midi_out: &mut midi_out::MidiOut,
        buf: &mut [u8],
        t: f64,
        keys: &mut Vec<midi_in::Key>,
        out: &mut Out,
    ) {
        while let Ok(scan) = self.found.try_recv() {
            match scan {
                Scan::Found(f) => match music.plug(&f.port) {
                    Some(port) => {
                        music.feed.event(t, Event::Plugged(port));
                        if let Some(sub) = f.port.output {
                            midi_out.plugged(port, &f.port.name, usb_output(f.card, sub));
                        }
                        let what = match (f.file.is_some(), f.port.output.is_some()) {
                            (true, true) => {
                                "listening, read only; MIDI out sends to it while Send MIDI \
                                 and CV out is on"
                            }
                            (true, false) => "listening, read only",
                            (false, _) => {
                                "an output only: MIDI out sends to it while Send MIDI and CV \
                                 out is on"
                            }
                        };
                        out.midi(&format!(
                            "+{t:.3} s {}: plugged in ({}, card {}); {what}",
                            f.port.name, f.port.about, f.card
                        ));
                        self.open.push(UsbIn {
                            card: f.card,
                            port,
                            file: f.file,
                        });
                    }
                    None => out.midi(&format!(
                        "+{t:.3} s {}: plugged in, but every MIDI port number is taken; not used",
                        f.port.name
                    )),
                },
                Scan::Gone(card) => {
                    let mut k = 0;
                    while k < self.open.len() {
                        if self.open[k].card == card {
                            out.midi(&format!(
                                "+{t:.3} s {}: gone (card {card} left the list); its keys let go",
                                music.label(self.open[k].port)
                            ));
                            self.close(k, music, midi_out, t, keys);
                        } else {
                            k += 1;
                        }
                    }
                }
                Scan::Said(line) => out.midi(&format!("+{t:.3} s {line}")),
            }
        }
        let mut k = 0;
        while k < self.open.len() {
            let Some(file) = self.open[k].file.as_mut() else {
                k += 1;
                continue;
            };
            let gone = match file.read(buf) {
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
                self.close(k, music, midi_out, t, keys);
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

/// How keys play the armed track: by note while the pads press notes (a
/// synth pad layout or the track's own synth's pad map, `key_notes`), the
/// kit as its Keys play (`kit`) sends each note while it plays a kit (not
/// the synth, nor an instrument of its own), else as the pad (1 to 16) the
/// MPC gives each note (an instrument whose arp works in pads, such as the
/// Drums engine).
fn keys_play(
    synth_on: bool,
    key_notes: bool,
    own: bool,
    kit: mpfree_sampler::engine::KitKeys,
) -> midi_in::KeysPlay {
    if key_notes {
        midi_in::KeysPlay::Notes
    } else if !synth_on && !own {
        midi_in::KeysPlay::KitPads(kit)
    } else {
        midi_in::KeysPlay::Pads
    }
}

/// Controller mode as the screen shows it, while it is on: the header's
/// chip ("Controller → Juno-X · ch 3 · A", or why nothing goes) and the
/// Q-Link column's cards.
fn controller_show(c: &controller_out::Controller, sending: bool) -> Option<gui::ControllerShow> {
    let p = c.plan()?;
    let bank = (b'A' + c.pad_bank() as u8) as char;
    let chip = if p.setup.port.is_empty() {
        "Controller: pick its port on I/O".to_string()
    } else if p.to.is_none() {
        format!("Controller → {}: not plugged in", p.shown)
    } else if !sending {
        "Controller: MIDI out is off in Prefs".to_string()
    } else {
        format!(
            "Controller → {} · ch {} · {bank}",
            p.shown,
            p.setup.channel + 1
        )
    };
    let knobs = c
        .knobs()
        .map(|(cc, name, v)| (name.unwrap_or_else(|| format!("CC {cc}")), cc, v));
    Some(gui::ControllerShow {
        chip,
        bank: c.bank(),
        knobs,
    })
}

/// A pad that sends a note in controller mode: blue, the Routing
/// section's colour for "sends out" (bright while held).
const CONTROLLER_PAD: [u8; 3] = [0x00, 0x16, 0x3F];

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
/// clip, dim green with nothing launched, red while muted or silent while
/// others are soloed, lit blue while soloed, and amber blinking while a
/// change waits in the mute queue (`blink` is the phase); dark with no
/// track. The kit's pads (`pad(p)`): lit in the kit's colour (`kit`)
/// while they play, red while muted or silent by a solo, lit blue while
/// soloed, amber blinking while a change waits, dark without a sound.
/// `track(t)` is None past the last track. Only colours Akai's app was
/// seen sending (a kit's resting colour, or its hit): there is no dim
/// red, so silent by solo is the muted red.
fn mute_colours(
    keys: app::MuteKeys,
    kit: [[u8; 3]; 16],
    track: impl Fn(usize) -> Option<time::MuteLight>,
    pad: impl Fn(usize) -> time::MuteLight,
    blink: bool,
) -> [[u8; 3]; 16] {
    let [amber, blue, _, green, red, ..] = akai::KIT_COLOURS;
    std::array::from_fn(|p| match keys {
        app::MuteKeys::Off => kit[p],
        app::MuteKeys::Tracks => match track(p) {
            Some(l) if l.waiting && blink => akai::hit_colour(amber),
            Some(l) if l.waiting => amber,
            Some(l) if l.solo => akai::hit_colour(blue),
            Some(l) if l.silent => red,
            Some(l) if l.launched => akai::hit_colour(green),
            Some(_) => green,
            None => [0; 3],
        },
        app::MuteKeys::Pads if kit[p] == [0; 3] => [0; 3],
        app::MuteKeys::Pads => match pad(p) {
            l if l.waiting && blink => akai::hit_colour(amber),
            l if l.waiting => amber,
            l if l.solo => akai::hit_colour(blue),
            l if l.silent => red,
            _ => akai::hit_colour(kit[p]),
        },
    })
}

/// MAIN held (the MPC's hold MAIN): the pads as the 16 tracks of a bank,
/// `track(p)` None past the last. The picked track (`picked`, its pad in
/// the bank) is white, a track playing a clip bright green, the others dim
/// green, and pads with no track dark.
fn track_pick_colours(
    picked: Option<usize>,
    track: impl Fn(usize) -> Option<time::MuteLight>,
) -> [[u8; 3]; 16] {
    let green = akai::KIT_COLOURS[3];
    std::array::from_fn(|p| match track(p) {
        _ if picked == Some(p) => step_keys::PLAYHEAD,
        Some(l) if l.launched => akai::hit_colour(green),
        Some(_) => green,
        None => [0; 3],
    })
}

/// 16 LEVEL with the Drums engine on the pads: its pads are sounds with no
/// kit levels, and nothing to transpose.
const DRUMS_NO_LEVELS: &str =
    "16 LEVEL: the Drums engine's pads have no 16 Levels; pick a kit with the wheel first";

/// Pad `pad` on 16 Levels' Transpose (sequencer S4): the armed track, and
/// the tracks that follow it, move to the pad's step, -8 to +7 (pad 9 as
/// written), by notes of the key while the synth's pads play In key or
/// chords of a scale, else by semitones. The notes already sounding keep
/// their pitch; the next ones move. A track that plays a kit, the Drums
/// engine or loops has no notes to move. The line for the log.
fn transpose_pad(
    pad: usize,
    time: &mut time::TimeHost,
    synth: &synth::Synth,
    kits: &sampler::TrackKits,
    own: &instruments::Instruments,
) -> String {
    let Some(track) = time.armed() else {
        return "transpose: arm the track to transpose".to_string();
    };
    let Some(t) = time.project().tracks.get(track) else {
        return "transpose: arm the track to transpose".to_string();
    };
    if !plays_notes(t, track, synth, kits, own) {
        return format!(
            "transpose: {} plays a kit; arm a track that plays notes",
            t.name
        );
    }
    let to = mpfree_seq::Transpose {
        by: (pad.min(15) as i8) - 8,
        in_key: synth.transpose_key().is_some(),
    };
    time.act(TimeAction::Transpose { track, to })
        .unwrap_or_else(|e| e)
}

/// Track `track` (`t`) plays notes, so its live transpose moves them: its
/// own instrument, or the synth if the track plays it (`kits`), but not
/// the Drums engine, a kit's sounds, loops or an audio track.
fn plays_notes(
    t: &mpfree_seq::Track,
    track: usize,
    synth: &synth::Synth,
    kits: &sampler::TrackKits,
    own: &instruments::Instruments,
) -> bool {
    match t.kind {
        mpfree_seq::TrackKind::Loops(_) | mpfree_seq::TrackKind::Audio => false,
        mpfree_seq::TrackKind::Notes if own.plays(track) => own.plays_notes(track),
        mpfree_seq::TrackKind::Notes => kits.plays_synth(track) && synth.plays_notes(),
    }
}

/// 16 Levels' Transpose on the pads (S4): the step the armed track is on
/// (`by`) bright, pad 9 (as written) dim white, the rest dim blue; all
/// dark with nothing to transpose (None).
fn transpose_colours(by: Option<i8>) -> [[u8; 3]; 16] {
    let Some(by) = by else {
        return [[0; 3]; 16];
    };
    let on = usize::try_from(i32::from(by) + 8).ok().filter(|&k| k < 16);
    std::array::from_fn(|k| match k {
        _ if on == Some(k) => [0x3F, 0x28, 0x00],
        8 => [0x0C, 0x0C, 0x0C],
        _ => [0x00, 0x06, 0x0E],
    })
}

/// What SHIFT + TRACK MUTE's key for pad `p` of track `track`'s kit
/// shows (the instrument redesign's step 10: a pad mute is the track's):
/// muted or silent by a solo, soloed, or waiting in the mute queue.
/// Nothing while the track plays notes (`off`), which have no pad mutes.
fn pad_light(time: &time::TimeHost, track: usize, p: usize, off: bool) -> time::MuteLight {
    let (mutes, solos) = time.pad_mutes(track).unwrap_or((0, 0));
    let silent = mutes | if solos == 0 { 0 } else { !solos };
    let bit = |b: u128| !off && p < 128 && b >> p & 1 == 1;
    time::MuteLight {
        silent: bit(silent),
        solo: bit(solos),
        waiting: !off && time.pad_waiting(track, p),
        launched: false,
    }
}

/// TRACK MUTE's key `pad` pressed (`tap` from [`app::App::mute_tap`]):
/// its track in the tap's bank (A = 0: tracks 1-16, B 17-32 ...) muted or
/// let play, or soloed with TRACK MUTE held, at once or on the launch
/// line ([`time::TimeHost::mute_or_solo`]); or, with SHIFT + TRACK MUTE,
/// the pad itself on track `track`, on the kit's page `page` (PAD BANK),
/// muted or let play, or soloed with TRACK MUTE held, at once or on the
/// line with the tracks ([`time::TimeHost::pad_mute_or_solo`]; step 10:
/// on that track alone, every bank and kit). `no_mutes`: why the pads
/// have no mutes ([`no_mutes`]: the track plays notes), so a pad's key
/// mutes nothing and says so. The line for the log.
fn mute_key(
    tap: app::MuteTap,
    pad: usize,
    track: usize,
    page: usize,
    no_mutes: Option<String>,
    time: &mut time::TimeHost,
) -> String {
    match (tap.keys, no_mutes) {
        (app::MuteKeys::Off, _) => String::new(),
        (app::MuteKeys::Tracks, _) => {
            let say = time::Say::Note {
                ok: "",
                err: "track mute: ",
            };
            match time.mute_or_solo_then(tap.bank * 16 + pad, tap.wait, tap.solo, say) {
                Some(r) => r.unwrap_or_else(|e| format!("track mute: {e}")),
                // Apart, said when the answer comes.
                None => String::new(),
            }
        }
        (app::MuteKeys::Pads, Some(why)) => why,
        (app::MuteKeys::Pads, None) => {
            match time.pad_mute_or_solo(track, page * 16 + pad, tap.wait, tap.solo) {
                Ok(time::PadTap::Waits(line) | time::PadTap::Now(line)) => line,
                Err(e) => e,
            }
        }
    }
}

/// The drum pads of kit `k` that a Perform pad's Drop drums mutes.
fn kit_drums(kits: &[kit::Kit], k: usize) -> u16 {
    kits.get(k).map_or(0, |kit| {
        perform_pads::drum_pads(
            kit.sounds
                .iter()
                .filter_map(|s| Some((usize::from(mpc_one::pad_index(s.note)?), s.name.as_str()))),
        )
    })
}

/// Why SHIFT + TRACK MUTE's pads mute nothing on track `track`: it plays
/// notes (the synth, or an instrument: `notes`, a bit each by track, as
/// [`instruments::Instruments::note_tracks`]), whose pads are no kit's
/// ([`gui::no_pad_mutes`]). None on a kit, any kit and bank (step 10).
fn no_mutes(kits: &sampler::TrackKits, notes: u128, track: usize) -> Option<String> {
    let plays_notes = notes.checked_shr(track as u32).is_some_and(|b| b & 1 == 1);
    gui::no_pad_mutes("pad mute", kits.kit_of(track).filter(|_| !plays_notes))
}

/// A project saved before pad mutes were a track's (step 10) muted and
/// soloed kit 1's A1-A16 in the mixer: each track that plays kit 1 (not
/// notes) takes them as its own. What it says, or None for nothing to
/// carry.
fn carry_pad_mutes(
    mix: &mut mixing::MixHost,
    kits: &sampler::TrackKits,
    time: &mut time::TimeHost,
) -> Option<String> {
    let (mutes, solos) = mix.take_old_pad_mutes()?;
    let notes = mix.instruments.note_tracks();
    let tracks = (0..time.track_count().min(128))
        .filter(|&t| kits.kit_of(t) == Some(0) && notes >> t & 1 == 0)
        .fold(0u128, |bits, t| bits | 1 << t);
    let n = time.carry_pad_mutes(tracks, mutes.into(), solos.into());
    let names = |bits: u16| {
        (0..16)
            .filter(|p| bits >> p & 1 == 1)
            .map(mpfree_time_ui::view::pad_name)
            .collect::<Vec<_>>()
            .join(" ")
    };
    let mut what = Vec::new();
    if mutes != 0 {
        what.push(format!("muted {}", names(mutes)));
    }
    if solos != 0 {
        what.push(format!("soloed {}", names(solos)));
    }
    let what = what.join(", ");
    Some(match n {
        0 => format!("project: kit 1's pads ({what}) play again: no track plays kit 1"),
        1 => format!("project: kit 1's pads ({what}) are now its track's own"),
        n => format!("project: kit 1's pads ({what}) are now each of its {n} tracks' own"),
    })
}

/// STEP SEQ: pad `pad` pressed as a step, `velocity` loud as it plays
/// (step_keys.rs). ERASE held clears the step; COPY held picks it to copy,
/// or copies the step picked onto it.
fn step_press(
    app: &app::App,
    keys: &mut step_keys::StepKeys,
    time: &mut time::TimeHost,
    pad: usize,
    velocity: u8,
    rolling: bool,
) -> step_keys::Pressed {
    let Some(s) = app.step_pads() else {
        return step_keys::Pressed::default();
    };
    let p = step_keys::Press {
        velocity,
        erase: app.erase_held(),
        copy: app.copy_held(),
        shift: app.shift_held(),
        rolling,
    };
    keys.press(s, pad, p, time)
}

/// A keyboard's key (`velocity` None: let go) while the pads are the
/// synth's line's steps: with step pads held, its note goes on them
/// (step_keys.rs). Some when it did, or to say why it can't.
fn step_key(
    app: &app::App,
    keys: &mut step_keys::StepKeys,
    time: &mut time::TimeHost,
    note: u8,
    velocity: Option<u8>,
) -> Option<step_keys::Pressed> {
    if !app.pads_are_steps() {
        return None;
    }
    match (velocity, app.pad_of_key(note)) {
        (None, Some(pad)) => {
            keys.key_up(pad);
            None
        }
        (Some(v), Some(pad)) => keys.key_down(pad, app.velocity(v), time),
        (Some(_), None) if keys.line_held() => Some(step_keys::Pressed {
            tell: Some(format!(
                "STEP SEQ: {} isn't a note of the pads, so a step can't hold it",
                synth::note_name(note)
            )),
            ..step_keys::Pressed::default()
        }),
        _ => None,
    }
}

/// STEP SEQ: ERASE pressed while step pads are held takes their locks
/// off (step_keys.rs); what to tell.
fn erase_held_steps(
    app: &app::App,
    keys: &mut step_keys::StepKeys,
    time: &mut time::TimeHost,
    note: u8,
    pressed: bool,
) -> Option<app::Effect> {
    if !(pressed && app.pads_are_steps() && app.is_erase(note)) {
        return None;
    }
    keys.erase_locks(time).map(app::Effect::Tell)
}

/// What the pads play: the one place in the app that answers it, as an
/// instrument, never a track (Jay, 20 Nov: "Instruments generate sound.
/// Tracks organize and record performances. The pads are controllers that
/// play instruments."). The looper's Records from: Pads
/// records it; the mixer says where it sounds
/// ([`mixing::MixHost::instrument_strip`]). The synth chosen for the
/// pads (the Synth tab, app/pads_focus.rs) is what they play whatever the
/// track plays; otherwise the pads follow the selected track and play its
/// instrument: MIDI out, its own or the one it plays of another track's
/// (step 4), its kit or the synth. `midi_out`: the selected track sends
/// MIDI out.
fn pads_instrument(
    app: &app::App,
    mix: &mixing::MixHost,
    kits: &sampler::TrackKits,
    midi_out: bool,
) -> looping::Instrument {
    use looping::Instrument;
    if app.pads_chose_synth() {
        return Instrument::Synth;
    }
    // An instrument of the host's chosen for the pads (step 6).
    if let Some(h) = mix.instruments.pads_play_at() {
        return Instrument::Own(h);
    }
    // A kit chosen for the pads (sampler step 15).
    if let Some(k) = app.pads_kit() {
        return Instrument::Kit(k);
    }
    let t = app.track();
    if midi_out {
        Instrument::MidiOut
    } else if let Some(owner) = mix.instruments.owner(t) {
        Instrument::Own(owner)
    } else {
        kits.kit_of(t).map_or(Instrument::Synth, Instrument::Kit)
    }
}

/// What audio track `t`'s input list offers (phase 4, board 4), from the
/// mixer: each strip an instrument plays into named by its track (a kit
/// or the synth by its own name), the Loop strip only while the project
/// has a Loop track, the pads' instrument as the looper was last told it,
/// and the mixer's refusals for a recording into the track's channel.
fn input_choices(
    time: &time::TimeHost,
    tracks: &mut audiotracks::TrackHost,
    looper: &looping::LoopHost,
    mix: &mixing::MixHost,
    t: usize,
) -> Result<mpfree_time_ui::inputs::InputChoices, String> {
    use audiotracks::InputSource;
    use mpfree_fx::StripId;
    let project = time.project();
    let p = tracks
        .place(t)
        .ok_or_else(|| format!("there is no audio track {}", t + 1))?;
    let channel = StripId::Track(mix.track_channel(p));
    let owner = |s: StripId| {
        (0..project.tracks.len()).find(|&q| {
            !project.tracks[q].is_audio()
                && mix.instrument_strip(looping::Instrument::Own(q), &[]) == Some(s)
        })
    };
    let named = |s: StripId, name: String| {
        let o = owner(s);
        InputSource {
            strip: s,
            name: o.map_or(name, |q| project.tracks[q].name.clone()),
            owner: o,
        }
    };
    let loop_track = project.tracks.iter().any(|x| x.is_loops());
    let sources: Vec<InputSource> = mix
        .inside_sources()
        .into_iter()
        .filter(|(_, s)| loop_track || *s != StripId::Track(mixing::LOOP))
        .map(|(name, s)| named(s, name))
        .collect();
    let pads = looper.pads_play().and_then(|(i, s)| {
        let s = s?;
        let name = match i {
            looping::Instrument::Kit(k) => format!("Kit {}", k + 1),
            _ => named(s, mix.strip_name(s)).name,
        };
        Some(InputSource {
            name,
            ..named(s, String::new())
        })
    });
    tracks.input_choices(t, &sources, pads, channel, |from| {
        mix.capture_refusal(from, channel)
    })
}

/// A looper line for the log as the screen says it: without "looper: ",
/// from a capital ("The take records silence from here...").
fn looper_says(line: &str) -> String {
    let words = line.strip_prefix("looper: ").unwrap_or(line);
    let mut c = words.chars();
    c.next()
        .map(|f| f.to_uppercase().chain(c).collect())
        .unwrap_or_default()
}

/// The looper's sound for one period from inside mpfree, from the mix of
/// the last period, as main.rs and the test session both run it: what the
/// pads play now (`pads`) is told first, then a take's source (held from
/// its start, `LoopHost::settle`) is taken from the mixer: the pads'
/// instrument's channel (`homes` say where a kit plays) or the mix picked.
/// Either way before the master, so the Level is applied once. A source
/// that can't be had (the pads on MIDI out, a strip gone, the master
/// with the loop in it) is silence, said, never another sound in its
/// place, and no take starts on it (`LoopHost::refuse`). The line input is
/// the looper's own: nothing from the mix. Lines for the log, and whether
/// each goes on the screen too.
fn loop_input(
    looper: &mut looping::LoopHost,
    mix: &mut mixing::MixHost,
    pads: looping::Instrument,
    homes: &[u8],
    left: &mut [f32; PERIOD],
    right: &mut [f32; PERIOD],
) -> [Option<(String, bool)>; 2] {
    use looping::LoopSource;
    let moved = looper.follow_pads(pads);
    looper.set_pads_strip(mix.instrument_strip(pads, homes));
    let how = match looper.takes_from() {
        (LoopSource::Mix(strip), _) => mix.loop_take(Some(strip), left, right),
        // A loop into an audio track's cell (phase 4b): the track's input,
        // inside the mix where the mixer says, or its line pick.
        (LoopSource::Track(t), _) => match t.from {
            looping::TrackFrom::Mix(strip, point) => {
                mix.track_loop_take((strip, point), t.channel, left, right)
            }
            looping::TrackFrom::Line(_) => Ok(0),
        },
        (LoopSource::Pads, Some(i)) => match mix.instrument_strip(i, homes) {
            Some(strip) => mix.loop_take(Some(strip), left, right),
            None => Err(looping::PADS_ON_MIDI_OUT),
        },
        // Before the pads were first told: nothing yet.
        (LoopSource::Pads, None) => Ok(0),
        (LoopSource::Line, _) => Ok(0),
    };
    if how.is_err()
        || looper.takes_from().0.line()
        || matches!(looper.takes_from(), (LoopSource::Pads, None))
    {
        left.fill(0.0);
        right.fill(0.0);
    }
    let took = looper.take_pads_as(left, right, how);
    [moved, took]
}

/// What a loop that starts when you play (the Looper tab's Starts: When I
/// play) said: a line for the log, and whether the screen says it too
/// (a start refused).
type LoopStart = Option<(String, bool)>;

/// Starts: When I play, a pad played. Pads are read at the period's top,
/// before the transport runs, so the hit is on the coming period's first
/// frame: a loop listening for the pads starts on it, and with the song
/// stopped the song starts on it too (from the bar it stopped in), so
/// bar 1 is your note; never for a free loop, which keeps its own time.
/// `take`: a take ready on the armed track (as for
/// [`time::TimeHost::take_waits`]) records from there too.
fn pad_starts_loop(
    looper: &mut looping::LoopHost,
    time: &mut time::TimeHost,
    take: Option<bool>,
    exporting: bool,
) -> LoopStart {
    let at = time.next_frame();
    let n = looper.listens_for_pads(at)?;
    let mut line = format!("looper: Loop {} starts on your note", n + 1);
    if looper.on_free_clock(n) {
        // A free loop keeps its own time: the song stays as it is.
        line.push_str(if looper.records_free(n) {
            ", free in your own time: REC again closes it, and its length sets the free loops' tempo"
        } else {
            ", on the free loops' clock (the song stays as it is)"
        });
    } else if looper.records_free(n) {
        // Tempo from the first loop: it records free, and closing it
        // starts the song (loop_sets_tempo).
        line.push_str(", free: REC again closes it and sets the tempo");
    } else if time.info().state == mpfree_transport::State::Stopped {
        match start_song_on_note(time, at, None, take, exporting, 0) {
            Ok(more) => {
                line.push_str(", and the song with it");
                if let Some(m) = more {
                    line = format!("{line}; {m}");
                }
            }
            Err(why) => return Some((looper.note_refused(n, &why), true)),
        }
    }
    if !looper.played(at) {
        return Some((
            format!(
                "looper: Loop {}: NOT started, the command queue is full",
                n + 1
            ),
            true,
        ));
    }
    Some((line, false))
}

/// Sampler R5.3b: whether a live hit or let-go `w` (on its kit pad)
/// waits for the launch line, and if so it waits. It does while the pads
/// play a kit (not the synth, not Free time's steps), the transport plays
/// with a launch quantise and the pad starts on the line (a Loop pad), or
/// when something played on the same pad waits already, so they keep
/// their order. A full table plays it at once: counted, logged and said.
/// True too for the let-go of a hit STOP dropped: it plays nothing.
fn waits_for_line(
    time: &mut time::TimeHost,
    player: &sampler::Player,
    app: &app::App,
    synth_on: bool,
    w: time::PadWait,
    full: &mut u64,
    out: &mut Out,
) -> bool {
    // The let-go of a hit STOP dropped while its pad was held: nothing
    // sounded, so there is nothing to let go of.
    if time.lets_go_of_dropped(w) {
        return true;
    }
    let starts = !synth_on
        && !app.free_time_recording()
        && time.pads_wait()
        && player.starts_on_line(player.kit(), usize::from(w.kit_pad));
    if !starts && !time.waits_on(w.from) {
        return false;
    }
    if time.wait_pad(w) {
        return true;
    }
    *full += 1;
    let s = format!(
        "Pads: {} hits already wait for the launch line, so this one plays at once ({} so far)",
        time::MAX_WAITS,
        *full
    );
    out.note(&s);
    out.show(gui::ToUi::Tell(s));
    false
}

/// Sampler R5.3b: STOP dropped `n` Loop pad hits still waiting for the
/// launch line (with their let-gos): counted in `total` and logged.
fn note_stop_dropped(n: usize, total: &mut usize, out: &mut Out) {
    if n == 0 {
        return;
    }
    *total += n;
    out.note(&format!(
        "pads: STOP dropped {n} Loop pad hit(s) still waiting for the launch line ({total} so far)"
    ));
}

/// Sampler R5.3b: a hit or let-go that waited for the launch line, played
/// `offset` frames into the period on the kit pad it was made on, as it
/// would have been when it was made: the clip recorder hears it as pressed there, the armed track's
/// arp plays it there, and the screen shows the pad down when it sounds
/// and up when its let-go does.
fn play_waited(
    app: &mut app::App,
    time: &mut time::TimeHost,
    awaiting: &mut [bool; 16],
    w: time::PadWait,
    offset: u32,
    out: &mut Out,
) -> Vec<app::Effect> {
    app.pads_at(offset);
    match w.from {
        time::PadFrom::Pad(slot) => {
            // The kit pad it was made on, though PAD BANK turned since: the
            // hit plays that one, and its let-go lets go of it (`kit_held`).
            let slot = usize::from(slot);
            if let Some(ev) = app.pressed_pad_event(w.kit_pad, w.velocity, w.down) {
                time.pad_pressed(offset, ev);
            }
            // The screen shows it down while its bank shows.
            let shown = usize::from(w.kit_pad) / 16 == app.kit_page();
            if let Some(a) = awaiting.get_mut(slot) {
                *a = w.down && shown;
            }
            if w.down {
                app.pad_down_on(slot, Some(w.kit_pad), w.velocity)
            } else {
                out.show(gui::ToUi::PadUp(slot));
                app.pad_up(slot)
            }
        }
        time::PadFrom::Key(pad) => {
            if let Some(ev) = app.pressed_pad_event(pad, w.velocity, w.down) {
                time.pad_pressed(offset, ev);
            }
            if w.down {
                app.kit_key_down(pad, w.velocity)
            } else {
                app.kit_key_up(pad)
            }
        }
    }
}

/// Starts: When I play, after the looper's period: a loop that heard the
/// line input with the song stopped starts the song on the next period's
/// first frame (its first pass lines up with that, from your note).
fn loop_starts_song(
    looper: &mut looping::LoopHost,
    time: &mut time::TimeHost,
    take: Option<bool>,
    exporting: bool,
) -> LoopStart {
    let n = looper.take_start()?;
    Some(
        match start_song_on_note(time, time.next_frame(), None, take, exporting, 0) {
            Ok(more) => (
                format!(
                    "looper: Loop {} heard you: the song starts on your note{}",
                    n + 1,
                    more.map(|m| format!("; {m}")).unwrap_or_default()
                ),
                false,
            ),
            Err(why) => (looper.start_refused(n, &why), true),
        },
    )
}

/// The song starts on a note on frame `at` (at tempo `bpm`, if given,
/// `into` ticks into the bar it stopped in), unless an export runs (the
/// transport's own refusals: [`time::TimeHost::start_at`]).
fn start_song_on_note(
    time: &mut time::TimeHost,
    at: u64,
    bpm: Option<f64>,
    take: Option<bool>,
    exporting: bool,
    into: mpfree_transport::time::Tick,
) -> Result<Option<String>, String> {
    if exporting {
        return Err("an export is running".to_string());
    }
    time.start_at(at, bpm, take, into)
}

/// Tempo from the first loop (the looper's options): whether the song can
/// take its tempo from a loop now. Not while another device's clock leads
/// (its tempo arriving, or PLAY waiting for its Start), the song has more
/// than one tempo, or an export runs. The looper is told every period.
fn tempo_from_loop_allowed(on: bool, time: &time::TimeHost, exporting: bool) -> bool {
    on && time.followed_bpm().is_none()
        && !time.clock_waits()
        && time.tempo_map().segments().len() <= 1
        && !exporting
}

/// Tempo from the first loop, after the looper's period: a free first
/// pass closed, so the song starts on the frame it comes round, at the
/// tempo its length sets, from the start of the bar it stopped in (the
/// loop's first bar). With a take ready it records from there too. The
/// line for the log (on the screen too when the song can't start: then
/// the loop plays at the song's tempo when it starts), and the tempo the
/// app takes from the next period on, so it doesn't ask for its old one.
fn loop_sets_tempo(
    looper: &mut looping::LoopHost,
    time: &mut time::TimeHost,
    take: Option<bool>,
    exporting: bool,
) -> (LoopStart, Option<f64>) {
    let Some((n, r, at, into)) = looper.take_tempo() else {
        return (None, None);
    };
    match start_song_on_note(time, at, Some(r.bpm), take, exporting, into) {
        Ok(more) => {
            // A close call, or a Length it couldn't keep: on the screen too.
            let note = looping::reading_note(&r);
            (
                Some((
                    format!(
                        "looper: Loop {} is {}: the song starts at that tempo{}{}",
                        n + 1,
                        looping::reading_words(&r),
                        more.map(|m| format!("; {m}")).unwrap_or_default(),
                        note.as_ref().map(|w| format!(" ({w})")).unwrap_or_default()
                    ),
                    note.is_some(),
                )),
                Some(r.bpm),
            )
        }
        Err(why) => (
            Some((looper.tempo_refused(n, r.bars, r.bpm, &why), true)),
            None,
        ),
    }
}

/// Live erase (sequencer roadmap S1, the MPC's way): whether a pad
/// pressed now erases in place of playing: ERASE is held while the
/// transport records, or while it only plays with the kit on the pads (the
/// button combos of 19 Nov).
fn live_erase_starts(app: &app::App, time: &time::TimeHost) -> bool {
    use mpfree_transport::State;
    app.erase_held()
        && match time.info().state {
            State::Recording => true,
            State::Playing => app.pads_play_kit(),
            State::Stopped | State::CountIn => false,
        }
}

/// Live erase: pad `pad` pressed with ERASE held while the transport
/// records or plays. Its notes on the armed track come out as the
/// playhead reaches them, until the pad or ERASE is let go; it neither
/// sounds nor records. What to tell.
fn live_erase(time: &mut time::TimeHost, pad: usize) -> Option<app::Effect> {
    let p = u8::try_from(pad).ok()?;
    let first = !time.erasing();
    time.erase_sound(p, true);
    first.then(|| {
        app::Effect::Tell(format!(
            "ERASE: pad {}'s notes come out of the armed track's clip as the playhead passes them; UNDO brings them back",
            app::kit_pad_name(pad)
        ))
    })
}

/// ERASE + kit pad `pad` while the transport is stopped (the button
/// combos of 19 Nov): every note of that pad comes out of the armed
/// track's clip at once, one take UNDO brings back, as ERASE + a lane's
/// name on Play. What to tell.
fn erase_pad_notes(time: &mut time::TimeHost, pad: usize) -> app::Effect {
    let name = app::kit_pad_name(pad);
    let Ok(pad) = u8::try_from(pad) else {
        return app::Effect::Tell(format!("ERASE: pad {name} has no notes to erase"));
    };
    app::Effect::Tell(match time.step_action(time::StepAction::ClearPad { pad }) {
        Ok(line) if line.is_empty() => {
            format!("ERASE: the armed track's clip has no notes on pad {name}")
        }
        Ok(_) => format!(
            "ERASE: pad {name}'s notes came out of the armed track's clip   ·   UNDO brings them back"
        ),
        Err(e) => format!("ERASE: {e}"),
    })
}

/// The note editor's pads (piano roll plan 1C.2): pad `pad` (0 to 15)
/// pressed or let go (`ev`, its kit pad's note-on or note-off), playing
/// `notes` from its instrument's pad map, while the Sequencer shows the
/// Notes view. With Pads write on, a pad going down writes at the
/// editor's cursor, but not while the transport records (the take has the
/// pads then); with it off, a pad picks its notes on a clip of pad hits.
/// The pad plays either way, and its let-go always goes, so a chord ends.
fn editor_pad(
    app: &app::App,
    time: &mut time::TimeHost,
    out: &mut Out,
    pad: usize,
    ev: mpfree_play::Event,
    notes: &[u8],
) -> Vec<app::Effect> {
    use mpfree_play::Event;
    use mpfree_time_ui::editor::EditAction;
    let (hit, velocity, down) = match ev {
        Event::NoteOn { note, velocity } => (note, velocity, true),
        Event::NoteOff { note } => (note, 0, false),
        _ => return Vec::new(),
    };
    if !app.notes_shown() || !time.editing() {
        return Vec::new();
    }
    let recording = time.info().state == mpfree_transport::State::Recording;
    if down && recording && time.pads_write() {
        return Vec::new();
    }
    let a = EditAction::Pad {
        key: pad as u8,
        hit,
        notes: notes.to_vec(),
        velocity,
        down,
    };
    let say = time::Say::NoteOrTell { err: "" };
    match time.act_then(TimeAction::Edit(a), say.clone()) {
        Some(r) => time_said(out, say, r),
        None => Vec::new(),
    }
}

/// ERASE let go: live erase stops for every pad.
fn live_erase_let_go(app: &app::App, time: &mut time::TimeHost) {
    if !app.erase_held() && time.erasing() {
        time.erase_none();
    }
}

/// Knob recording (sequencer roadmap S1, the Elektron way): the parameter
/// a screen action turns and its new value, as the recorder takes a knob
/// move: a knob of the selected track's instrument (the Synth tab, its
/// Q-Links), a strip control or an insert's knob on the mixer, or an
/// effect's knob in the chain editor. Only parameters on a track; the
/// recorder keeps the armed track's while the transport records, as a
/// lock on the step it belongs to.
fn knob_turn(
    a: &gui::FromUi,
    app: &app::App,
    mix: &mixing::MixHost,
) -> Option<(mpfree_play::mpfree_param::ParamAddr, f32)> {
    use mpfree_play::mpfree_param::{Owner, ParamAddr};
    let (addr, value) = match a {
        gui::FromUi::Synth(SynthAction::Set { id, value }) => (
            ParamAddr::new(Owner::Inst(u8::try_from(app.track()).ok()?), *id),
            *value,
        ),
        gui::FromUi::Mixer(mpfree_mixer_ui::MixAction::Set { strip, id, value }) => (
            ParamAddr::new(Owner::Strip(modmix::strip_of(*strip)?), *id),
            *value,
        ),
        gui::FromUi::Mixer(mpfree_mixer_ui::MixAction::SetInsert {
            strip,
            slot,
            id,
            value,
        }) => {
            let uid = mix.mixer.chain(*strip)?.uid(*slot)?;
            (
                ParamAddr::new(Owner::Insert(modmix::strip_of(*strip)?, uid), *id),
                *value,
            )
        }
        gui::FromUi::Chain(mpfree_chain_ui::ChainAction::Set { addr, value }) => (*addr, *value),
        // The Perform view's Q-Links on a throw's Perform FX.
        gui::FromUi::ThrowKnob { addr, value } => (*addr, *value),
        _ => return None,
    };
    // A MIDI effect's setting records as a lock too: the mod engine plays
    // it back onto the track's chain.
    (addr.owner.track().is_some() && value.is_finite()).then_some((addr, value))
}

/// Knob recording: a screen action's knob turn goes to the recorder while
/// the transport records (see [`knob_turn`]).
fn record_knob(a: &gui::FromUi, app: &app::App, mix: &mixing::MixHost, time: &mut time::TimeHost) {
    if time.info().state != mpfree_transport::State::Recording {
        return;
    }
    if let Some((addr, value)) = knob_turn(a, app, mix) {
        time.knob(0, addr, value);
    }
}

/// A Q-Link turn of the steps held on the step pads (or of the step
/// picked on Beats): the steps held stay on their release.
fn step_turn(a: &gui::FromUi) -> bool {
    matches!(
        a,
        gui::FromUi::Steps(
            time::StepAction::Change { .. }
                | time::StepAction::ChangeSteps { .. }
                | time::StepAction::Lock { .. }
        )
    )
}

/// What a step press did: its lines, and its sound once while stopped.
fn sound_step(
    p: step_keys::Pressed,
    out: &mut Out,
    player: &mut sampler::Player,
    synth: &mut synth::Synth,
    synth_on: bool,
) -> Option<app::Effect> {
    if !p.log.is_empty() {
        out.note(&p.log);
    }
    for (sound, v) in p.sound {
        if synth_on {
            synth.note_on(usize::from(sound), v, 0);
        } else {
            player.hit_at(usize::from(sound), v, 0);
        }
    }
    p.tell.map(app::Effect::Tell)
}

/// A scale row's name pressed (`down`) or let go on Beats: the synth's pad
/// (or the kit's) sounds while it is held, as a step pad's press does
/// while stopped, recording nothing.
fn audition(
    pad: usize,
    down: bool,
    player: &mut sampler::Player,
    synth: &mut synth::Synth,
    synth_on: bool,
) {
    let v = play_page::TAP_VELOCITY;
    match (synth_on, down) {
        (true, true) => synth.note_on(pad, v, 0),
        (true, false) => synth.note_off(pad, 0),
        (false, true) => {
            player.hit_at(pad, v, 0);
        }
        (false, false) => player.release_at(pad, 0),
    }
}

/// What plays the notes the note editor hears (hearing.rs, piano roll
/// plan 1C.2): a track's instrument (its own or another's) as the
/// audition's own player; the synth on the pads by note number, counted
/// with its other holders; a kit track's pad, as Steps' audition hits it.
/// A track on MIDI out hears nothing: its notes are its gear's.
struct HearOn<'a> {
    own: &'a mut instruments::Instruments,
    synth: &'a mut synth::Synth,
    player: &'a mut sampler::Player,
    kits: &'a sampler::TrackKits,
    midi: &'a midi_out::MidiOut,
}

impl hearing::Sound for HearOn<'_> {
    fn start(
        &mut self,
        track: usize,
        hits: bool,
        key: u8,
        velocity: u8,
        out: &mut Vec<(hearing::On, u8)>,
    ) {
        use hearing::On;
        let v = velocity.max(1);
        if u16::try_from(track).map_or(true, |t| self.midi.takes(t)) {
            return;
        }
        let mut notes = [0u8; time::HEARD_NOTES];
        let mut fill = |from: &[u8]| {
            let n = from.len().min(notes.len());
            notes[..n].copy_from_slice(&from[..n]);
            n
        };
        if self.own.plays(track) {
            // A pad: the notes its pad map plays, as written.
            let n = match hits {
                true => fill(self.own.pad_notes(track, usize::from(key))),
                false => fill(&[key]),
            };
            for &note in &notes[..n] {
                if self.own.audition_on(track, note, v) {
                    out.push((On::Instrument, note));
                }
            }
        } else if self.kits.plays_synth(track) {
            let n = match hits {
                true => fill(self.synth.notes(usize::from(key))),
                false => fill(&[key]),
            };
            for &note in &notes[..n] {
                self.synth.note_on_note(note, v, 0);
                out.push((On::Synth, note));
            }
        } else if hits {
            let kit = self.kits.kit_of(track).unwrap_or(0);
            if self.player.hit_kit_at(kit, usize::from(key), v, 0) {
                out.push((On::Kit(kit), key));
            }
        }
    }

    fn end(&mut self, on: hearing::On, key: u8) {
        match on {
            hearing::On::Instrument => {
                self.own.audition_off(key);
            }
            hearing::On::Synth => self.synth.note_off_note(key, 0),
            hearing::On::Kit(kit) => self.player.release_kit_at(kit, usize::from(key), 0),
        }
    }
}

/// A step pad let go: its line, and the sound its press played stops.
fn let_go_of_step(
    r: step_keys::Released,
    out: &mut Out,
    player: &mut sampler::Player,
    synth: &mut synth::Synth,
    synth_on: bool,
) {
    if !r.log.is_empty() {
        out.note(&r.log);
    }
    for sound in r.silence {
        if synth_on {
            synth.note_off(usize::from(sound), 0);
        } else {
            player.release_at(usize::from(sound), 0);
        }
    }
}

/// What PAD BANK says when it turns the kit's page: "pad bank B: pads
/// B1-B16 of Drums (3 sounds)".
fn pad_page_note(page: usize, kit: &str, sounds: u128) -> String {
    let on_page = (sounds >> (page * 16)) as u16;
    let n = on_page.count_ones();
    let first = page * 16;
    format!(
        "pad bank {}: pads {}-{} of {kit} ({})",
        app::bank_letter(page),
        app::kit_pad_name(first),
        app::kit_pad_name(first + 15),
        match n {
            0 => "no sounds yet".to_string(),
            1 => "1 sound".to_string(),
            n => format!("{n} sounds"),
        }
    )
}

/// Which of the 16 pads shows kit pad `pad` while a kit is on the pads
/// (`kit`): one on page `page` (PAD BANK A = 0), pads 16 * page to
/// 16 * page + 15. The synth's pads are its 16 slots. None off the page.
fn pad_cell(pad: usize, page: usize, kit: bool) -> Option<usize> {
    let cell = if kit {
        pad.checked_sub(page * 16)?
    } else {
        pad
    };
    (cell < 16).then_some(cell)
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
    let master = 10f32.powf(o.gain_db() / 20.0);
    out.say(&format!("play the pads for {} s; each sound peaks at most {:.0} dB below full scale at full velocity",
                     o.seconds, -o.gain_db()));
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
/// Kits `users` are the empty slots kept for New kit
/// ([`sampler::USER_KITS`]).
fn kit_views(kits: &[kit::Kit], users: Range<usize>, synth: &synth::Synth) -> Vec<gui::KitView> {
    let mut views: Vec<gui::KitView> = kits
        .iter()
        .enumerate()
        .map(|(k, kit)| {
            // A kit's 128 pads; its sounds start on pads 1 to 16.
            let mut pads = vec![None; 128];
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
                pitches: Vec::new(),
                user: users.contains(&k),
                key: None,
                tags: Vec::new(),
            }
        })
        .collect();
    views.push(synth.kit_view());
    views
}

/// The entry SHIFT + wheel turns to from `from`, `d` detents on among `n`
/// (the kits, then the synth): a user kit slot (`users`) counts only once
/// it is in use, with sounds (`sounds(k)`) or a track playing it, so the
/// wheel never stops on an empty spare slot, nor on a kit set aside with
/// a track UNDO took away (`TrackKits::set_aside`).
fn kit_turned(
    from: usize,
    d: i32,
    n: usize,
    users: &Range<usize>,
    sounds: impl Fn(usize) -> u128,
    kits: &sampler::TrackKits,
) -> usize {
    let n = n.max(1);
    let skip =
        |k: usize| kits.set_aside(k) || (users.contains(&k) && sounds(k) == 0 && !kits.plays(k));
    let mut at = from;
    for _ in 0..d.unsigned_abs() {
        for _ in 0..n {
            at = (at as i64 + i64::from(d.signum())).rem_euclid(n as i64) as usize;
            if !skip(at) {
                break;
            }
        }
    }
    at
}

/// The loading steps, for the progress bar.
const STEPS: usize = 6;

/// The first start on a stick the USB creator made ([`setup`]): only with
/// `--data`, never in `play`, and only where the stick has the backup
/// service and the store saves there. What it does is saved through
/// `store` as each step ends. Returns the MPC's own kits file (when there
/// is one, from this start or an earlier one).
fn first_start(
    o: &Options,
    out: &mut Out,
    timed: bool,
    store: &mpfree_store::Store,
) -> Option<PathBuf> {
    let data = o.data.as_deref().filter(|_| !timed)?;
    let paths = setup::Paths::on_the_stick(data, Path::new(REPORT_DIR));
    let conf = data.join("settings").join(format!("{}.conf", setup::OWNER));
    let mut saved = mpfree_store::Settings::parse(&fs::read_to_string(conf).unwrap_or_default());
    // What the creator left out, kept before anything runs, so an update of
    // the stick (which doesn't say) and Prefs go on from it.
    if let Some(s) = setup::remember_left_out(&paths, &saved).filter(|_| store.saves()) {
        match store.save_settings(setup::OWNER, &s) {
            Ok(()) => {
                store.flush();
                out.say(&format!(
                    "first start: left out: {}",
                    s.get_str("leave-out").unwrap_or("none")
                ));
                saved = s;
            }
            Err(e) => out.say(&format!(
                "first start: what was left out was not saved ({e})"
            )),
        }
    }
    let mut made = None;
    if setup::needed(&paths, &saved) && !store.saves() {
        // Nothing it did would be kept, so it would run again at every start.
        out.say("first start: not run: saving is off on the data folder (see the storage line)");
    } else if setup::needed(&paths, &saved) {
        let mut lines = Vec::new();
        let mut not_saved = None;
        let outcome = setup::run(
            &paths,
            &saved,
            &setup::Timing::on_the_mpc(),
            &mut |s| {
                out.show(gui::ToUi::Progress {
                    message: s.message,
                    detail: s.detail,
                    value: s.value,
                })
            },
            &mut |line| lines.push(line),
            // Each step is on the stick before the next one starts.
            &mut |s| match store.save_settings(setup::OWNER, s) {
                Ok(()) => store.flush(),
                Err(e) => not_saved = Some(e),
            },
        );
        for line in lines {
            out.say(&line);
        }
        if let Some(e) = not_saved {
            out.say(&format!("first start: a step was not saved ({e})"));
        }
        if let Err(e) = store.save_settings(setup::OWNER, &outcome.saved) {
            out.say(&format!(
                "first start: not saved ({e}); it runs again at the next start"
            ));
        }
        store.flush();
        if let Some(why) = &outcome.failed {
            out.show(gui::ToUi::Progress {
                message: "The first-start setup did not finish".into(),
                detail: format!("{why}   ·   it tries again at the next start"),
                value: 0.0,
            });
            thread::sleep(Duration::from_secs(5));
        }
        made = outcome.kits;
    }
    made.or_else(|| {
        let lib = mpfree_library::Library::at(data).ok()?;
        let loc =
            mpfree_library::Loc::new(mpfree_library::Place::Kits, &[setup::KITS_FILE]).ok()?;
        lib.file_path(&loc).ok()
    })
}

/// Prefs > Backup and Akai kits' state, from the first-start setup's
/// store (None without a data folder or a creator's stick).
fn setup_prefs(o: &Options, store: &mpfree_store::Store) -> Option<setup::PrefsState> {
    let data = o.data.as_deref().filter(|_| store.saves())?;
    let paths = setup::Paths::on_the_stick(data, Path::new(REPORT_DIR));
    let conf = data.join("settings").join(format!("{}.conf", setup::OWNER));
    let saved = mpfree_store::Settings::parse(&fs::read_to_string(conf).unwrap_or_default());
    setup::prefs_state(&paths, &saved)
}

/// Prefs > Backup and Akai kits: a new backup (`backup`) or Akai's kits and
/// samples at the next start, or (`want` false) not after all. Saved in
/// the setup's store; the new state and a line for the musician.
fn setup_next_start(
    o: &Options,
    store: &mpfree_store::Store,
    backup: bool,
    want: bool,
) -> (Option<setup::PrefsState>, String) {
    let Some(data) = o.data.as_deref().filter(|_| store.saves()) else {
        return (
            None,
            "Prefs: nothing to change without a data folder that saves".into(),
        );
    };
    let paths = setup::Paths::on_the_stick(data, Path::new(REPORT_DIR));
    let conf = data.join("settings").join(format!("{}.conf", setup::OWNER));
    let saved = mpfree_store::Settings::parse(&fs::read_to_string(conf).unwrap_or_default());
    let next = if backup {
        setup::ask_backup(&paths, &saved, want)
    } else {
        setup::ask_akai_kits(&paths, &saved, want)
    };
    let what = if backup {
        "A new backup of your MPC"
    } else {
        "Akai's kits and samples"
    };
    let said = match store.save_settings(setup::OWNER, &next) {
        Ok(()) => {
            store.flush();
            if want {
                format!("{what}: at the next start, with the stick in. Press the power button and choose Restart to start it now.")
            } else {
                format!("{what}: not at the next start after all.")
            }
        }
        Err(e) => format!("{what}: not saved ({e})"),
    };
    (
        setup::prefs_state(&paths, &next).or_else(|| setup_prefs(o, store)),
        said,
    )
}

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
    // A scripted run is the app as a musician has it (STOP stops, it
    // doesn't end the run); only its length is the script's.
    let mut player = gui::Player::new(fonts, size, timed && o.workflow.is_none(), backdrop);
    if let crash::LastRun::Crashed(why) = &last_run {
        player.safe_start();
        player.set_crash_why(why.clone());
    }
    match load_theme(o) {
        Ok(t) => player.ui.set_theme(t),
        Err(e) => {
            out.say(&format!("{e}; using the default theme"));
            player.apply(gui::ToUi::ThemeFallbacks {
                left_out: 0,
                lines_fell_back: 0,
                default_shown: 1,
            });
        }
    }
    let (to_ui, from_audio) = sync_channel::<gui::ToUi>(4096);
    let (to_audio, from_ui) = sync_channel::<gui::FromUi>(256);
    // The screen's XY pad as one value beside that queue (xy_slot.rs).
    let xy = std::sync::Arc::new(xy_slot::XySlot::default());
    player.set_xy_slot(std::sync::Arc::clone(&xy));
    // The power button: only when the system can act on a request
    // (`--power-request`, the stick's root path unit). Read-only, not
    // grabbed, on a thread of its own (power.rs).
    if let Some(path) = &o.power_request {
        player.set_power_request(path.clone());
        match power::spawn_reader(to_ui.clone(), || gui::ToUi::PowerPress, gui::ToUi::Line) {
            Ok(()) => out.say(&format!(
                "power button: a short press opens the power card; shut down and restart go to {}",
                path.display()
            )),
            Err(e) => out.say(&format!("power button: not read ({e})")),
        }
    } else {
        out.say("power button: not read (no --power-request); hold it to switch off");
    }
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
        let kernel = follow_kernel(to_ui.clone());
        follow_journal(to_ui.clone(), kernel);
    }
    let result = player_session(o, &mut out, timed, &from_ui, &xy);
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

/// The kernel's stick and memory errors, followed (read-only) onto the
/// status board (`kernel_log`); without `/dev/kmsg`, what the system log's
/// follower feeds them from.
fn follow_kernel(tx: SyncSender<gui::ToUi>) -> Option<kernel_log::FromJournal> {
    let lines = tx.clone();
    kernel_log::follow(
        move |l| {
            println!("{l}");
            let _ = io::stdout().flush();
            let _ = lines.try_send(gui::ToUi::Line(l));
        },
        move |r| tx.send(gui::ToUi::Problem(r)).is_ok(),
    )
}

/// The system log, followed (`journalctl -f`, read-only) into the Console page,
/// and, when `/dev/kmsg` couldn't be read, its kernel lines to the status
/// board's kernel watch.
fn follow_journal(tx: SyncSender<gui::ToUi>, kernel: Option<kernel_log::FromJournal>) {
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
                if let Some(k) = &kernel {
                    k.line(&l);
                }
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
        "sampler: {}, {} steals, {} events dropped ({} hits), {} let-gos kept aside, {} values merged, \
         at most {} grains ({} skipped, {} faded to make room, {} clouds held back)",
        time.describe(),
        c.steals,
        c.dropped_events,
        c.refused_hits,
        c.late_let_gos,
        c.merged_values,
        c.peak_grains,
        c.skipped_grains,
        c.faded_grains,
        c.held_back_clouds
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
/// page with its loop row.
fn strip_shown(view: app::View) -> bool {
    matches!(view, app::View::Looper | app::View::Play)
}
/// How often the Play page's master meter and Q-Link values are sent.
const METER_EVERY: Duration = Duration::from_millis(50);
/// How often the Sample page's playheads are sent while it shows.
const SAMPLE_HEADS_EVERY: Duration = Duration::from_millis(40);
/// While overdubbing, the loop's waveform is redrawn about once a second.
const LOOP_WAVE_PERIODS: u64 = 344;

/// The sequencer's notes on the pads: each track's own kit, or the synth
/// for the tracks that play it (`kits`). Each hit is noted so the screen's
/// pads can flash.
struct SeqPads<'a> {
    player: &'a mut sampler::Player,
    /// The synth, which the tracks that play it play (`kits`).
    synth: &'a mut synth::Synth,
    /// What each track plays: a kit or the synth.
    kits: &'a sampler::TrackKits,
    /// What the pads show: the armed track's entry. Only the hits of the
    /// tracks that play it flash them.
    shown: usize,
    /// By kit pad (or the synth's pad): the screen shows those on the
    /// page.
    flashes: [u8; 128],
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
    /// The mod engine, read: whether something besides a clip moves a
    /// track's MIDI out control (S5's controls ahead).
    modu: &'a modmix::ModMix,
    /// Synth-voice pads play their own synths (pad_synths.rs).
    pad_synths: &'a mut pad_synths::PadSynths,
    /// While an export runs, every event goes to its MIDI file too, as
    /// MIDI Out would send it (export.rs).
    tap: &'a mut export::Tap,
    /// While an export's replay plays one track alone: the other tracks
    /// on the pads are kept quiet (`export::quiets`).
    only: Option<mpfree_export::Only>,
    /// Each track's live transpose (sequencer S4), and the key it moves
    /// through In key: where the pads become notes, a track that plays
    /// notes shifts them ([`SeqPads::shift`]).
    shifts: [mpfree_seq::Transpose; mpfree_seq::MAX_TRACKS],
    key: Option<mpfree_arp::scale::Scale>,
    /// Each track's arp notes by number (a synth pad layout), and what
    /// they became under its shift, so their ends and pressure find them.
    by_note: &'a mut [mpfree_arp::scale::Held],
}

/// A key a route sends to a track that isn't armed (the I/O view's
/// Routing): played at once where the track's clips play, on MIDI out, its
/// own instrument, or what it plays: the synth by note when it plays the
/// synth (`plays_synth`), else its kit as the kit's Keys play sends the
/// note (sampler step 15, `sampler::Player::note_on_to`): the pad that
/// sends it (`profile::kit_pad_of_note`) as that pad's kind has it, as a
/// clip's note on it would be ([`play_routed_pad`]), or the keygroups that
/// hold it. And on CV out if it follows
/// the track. It goes through no arp, and nothing records it: REC records
/// the armed track.
// What a track plays comes in pieces the sound loop holds apart.
#[allow(clippy::too_many_arguments)]
fn play_routed(
    track: u8,
    key: midi_in::Key,
    midi: &mut midi_out::MidiOut,
    cv: &mut cv_out::CvOut,
    own: &mut instruments::Instruments,
    pad_synths: &mut pad_synths::PadSynths,
    synth: &mut synth::Synth,
    plays_synth: bool,
    player: &mut sampler::Player,
    kit: usize,
) {
    use mpfree_play::Event;
    let t = usize::from(track);
    // Expression out: the track's gear on MIDI out and its CV outputs take
    // the keys' wheel, pressure and MPE, and their going down (before the
    // note, so its bend leads it) and up (after the note-off, below).
    let expr = key.expr_out();
    let up = matches!(key, midi_in::Key::Up { .. });
    if let Some(i) = expr.filter(|_| !up) {
        cv.expression(u16::from(track), i);
    }
    let gear = !up && expr.is_some_and(|i| midi.expression(u16::from(track), i));
    let event = match key {
        midi_in::Key::Down { note, velocity } => Event::NoteOn {
            note,
            velocity: velocity.max(1),
        },
        midi_in::Key::Up { note } => Event::NoteOff { note },
        midi_in::Key::Pressure { note, value } => Event::Pressure { note, value },
        // The wheel and an MPE note's own slide and tone: the track's
        // gear (above), else its instrument, else the synth (Thru passes a
        // keyboard's on as it came).
        midi_in::Key::Bend { value } => {
            if !gear && !own.bend(t, value) {
                if plays_synth {
                    synth.bend(value);
                } else {
                    // A kit's keygroups bend (sampler step 15).
                    player.bend_kit(kit, value);
                }
            }
            return;
        }
        midi_in::Key::Expr { note, kind, value } => {
            if !gear && !own.note_expression(t, note, kind, value) && plays_synth {
                synth.expression_note(note, kind, value);
            }
            return;
        }
        // A pedal, the mod wheel, breath or a foot controller: the track's
        // gear, else its instrument, else the synth (a kit takes none).
        midi_in::Key::Control { ctl, value } => {
            if !midi.keys_control(u16::from(track), ctl, value)
                && !own.control(t, ctl, value, 0)
                && plays_synth
            {
                synth.control(ctl, value, 0);
            }
            return;
        }
    };
    let o = mpfree_play::Out {
        offset: 0,
        frame: 0,
        track: u16::from(track),
        dest: Default::default(),
        event,
        notes: false,
    };
    // A key's pressure reached the CV outputs above, as the hardest key's.
    if !matches!(event, Event::Pressure { .. }) {
        cv.send(&o, midi_out::NoteIs::Note);
    }
    if let Some(i) = expr.filter(|_| up) {
        cv.expression(u16::from(track), i);
    }
    if gear && matches!(event, Event::Pressure { .. }) {
        return;
    }
    if midi.send(&o, midi_out::NoteIs::Note) {
        if let Some(i) = expr.filter(|_| up) {
            midi.expression(u16::from(track), i);
        }
        return;
    }
    if own.plays(t) {
        match event {
            Event::NoteOn { note, velocity } => {
                own.note_on(t, note, velocity, 0);
            }
            Event::NoteOff { note } => {
                own.note_off(t, note, 0);
            }
            Event::Pressure { note, value } => {
                own.note_pressure(t, note, value);
            }
            _ => {}
        }
        return;
    }
    if plays_synth {
        match event {
            Event::NoteOn { note, velocity } => synth.note_on_note(note, velocity, 0),
            Event::NoteOff { note } => synth.note_off_note(note, 0),
            Event::Pressure { note, value } => synth.pressure_note(note, value),
            _ => {}
        }
        return;
    }
    // A kit, as its Keys play sends the note (sampler step 15): the pad
    // that sends it, in any bank, or the keygroups that hold it. A let-go
    // and pressure go where the note went down.
    let to = match event {
        Event::NoteOn { note, .. } => player.note_on_to(kit, note),
        Event::NoteOff { note } => player.note_off_to(kit, note),
        Event::Pressure { note, .. } => player.note_held_to(kit, note),
        _ => return,
    };
    match (to, event) {
        (sampler::NoteTo::Pad(pad), _) => {
            play_routed_pad(track, pad, event, midi, (pad_synths, synth), player, kit);
        }
        (sampler::NoteTo::Keys(n), Event::NoteOn { velocity, .. }) => {
            player.key_kit_at(kit, n, velocity, 0);
        }
        (sampler::NoteTo::Keys(n), Event::NoteOff { .. }) => player.key_off_kit_at(kit, n, 0),
        // A keygroup's key takes no pressure yet.
        _ => {}
    }
}

/// A synth-voice pad's (the Pads tab's Instrument pad's) hit, let-go or
/// pressure, track `track`'s on pad `pad` of kit `kit`: on the pad's own
/// copy of its engine, into the pad's strip (`pad_synths.rs`), or on the
/// shared synth's engine (the Synth channel) while the pad has none (past
/// the sixteen, or while its copy is being made; counted). A hit ends what
/// the same track's last hit on the pad started on either. Never
/// allocates.
fn pad_voice(
    pads: &mut pad_synths::PadSynths,
    synth: &mut synth::Synth,
    track: u16,
    (kit, pad): (usize, usize),
    (engine, note): (synth::Engine, u8),
    event: mpfree_play::Event,
    offset: u32,
) {
    use mpfree_play::Event;
    match event {
        Event::NoteOn { velocity, .. } => {
            if pads.note_on(track, kit, pad, engine, note, velocity, offset) {
                synth.pad_voice_off(track, pad, offset);
            } else {
                pads.note_off(track, pad, offset);
                synth.pad_voice_on(track, pad, (engine, note), velocity, offset);
            }
        }
        Event::NoteOff { .. } => {
            pads.note_off(track, pad, offset);
            synth.pad_voice_off(track, pad, offset);
        }
        // The pad's own copy takes it, else the shared synth.
        Event::Pressure { value, .. } if !pads.pressure(track, pad, value, offset) => {
            synth.pad_voice_pressure(track, pad, value, offset);
        }
        _ => {}
    }
}

/// A routed key's hit, let-go or pressure on pad `pad` of track `track`'s
/// kit `kit`, as the pad's kind has it (the Pads tab's, the redesign's
/// 6b), as `SeqPads::routed` plays a clip's: a MIDI out pad to its own
/// port, channel and note (`MidiOut::pad_to`), an Instrument pad as a
/// voice of its engine (`Synth::pad_voice_on`), and a Sample pad (no
/// route) on the sampler. `event`'s note is the key's; the pad's own
/// note goes out. Never allocates.
fn play_routed_pad(
    track: u8,
    pad: usize,
    event: mpfree_play::Event,
    midi: &mut midi_out::MidiOut,
    (pad_synths, synth): (&mut pad_synths::PadSynths, &mut synth::Synth),
    player: &mut sampler::Player,
    kit: usize,
) {
    play_routed_pad_at(track, pad, event, 0, midi, (pad_synths, synth), player, kit);
}

/// [`play_routed_pad`] `offset` frames into the next period: the pads'
/// arp's notes ([`PadsArp`], sampler step 15c).
#[allow(clippy::too_many_arguments)]
fn play_routed_pad_at(
    track: u8,
    pad: usize,
    event: mpfree_play::Event,
    offset: u32,
    midi: &mut midi_out::MidiOut,
    (pad_synths, synth): (&mut pad_synths::PadSynths, &mut synth::Synth),
    player: &mut sampler::Player,
    kit: usize,
) {
    use mpfree_play::Event;
    let track = u16::from(track);
    match player.pad_route(kit, pad) {
        Some(sampler::PadRoute::Midi {
            port,
            channel,
            note,
        }) => {
            // `pad_to` reads the pad from the event's note.
            let p = pad.min(usize::from(u8::MAX)) as u8;
            let event = match event {
                Event::NoteOn { velocity, .. } => Event::NoteOn { note: p, velocity },
                Event::NoteOff { .. } => Event::NoteOff { note: p },
                Event::Pressure { value, .. } => Event::Pressure { note: p, value },
                _ => return,
            };
            let o = mpfree_play::Out {
                offset,
                frame: 0,
                track,
                dest: Default::default(),
                event,
                notes: false,
            };
            midi.pad_to(&o, port, *channel, *note);
        }
        Some(&sampler::PadRoute::Voice(engine, note)) => {
            pad_voice(
                pad_synths,
                synth,
                track,
                (kit, pad),
                (engine, note),
                event,
                offset,
            );
        }
        None => match event {
            Event::NoteOn { velocity, .. } => {
                player.hit_kit_at(kit, pad, velocity, offset);
            }
            Event::NoteOff { .. } => player.release_kit_at(kit, pad, offset),
            Event::Pressure { value, .. } => player.pressure_kit_at(kit, pad, value, offset),
            _ => {}
        },
    }
}

/// What plays outside the MPC, for the Export panel's Fast or Listen
/// (export.rs), gathered when Export is pressed.
fn export_rig(
    time: &time::TimeHost,
    mix: &mixing::MixHost,
    midi_out: &midi_out::MidiOut,
    music: &midi_in::MusicIn,
    cv_out: &cv_out::CvOut,
    app: &app::App,
    looper: &looping::LoopHost,
) -> mpfree_export::Rig {
    let tracks = export::track_names(time);
    mpfree_export::Rig {
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
    }
}

/// A learned MIDI control's value (0 to 1 over the parameter's range) set
/// as a chain editor knob sets it (M5): the value set, in the parameter's
/// own units. None when MPFree has no way to set that parameter from a
/// control (a synth's goes through [`learned_synth`]).
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

/// A kit pad's setting described with its value now, for MIDI Learn: the
/// pad of a kit, by its track's kit (`Owner::Pad`) or by the kit's number
/// in the project's list (`Owner::KitPad`, sampler step 16), and the
/// setting's `PadField` number ([`pad_describe`]).
type PadNow<'a> =
    &'a dyn Fn(mpfree_play::mpfree_param::Owner, u16) -> Option<(mpfree_node::ParamInfo, f32)>;

/// What a target that names an instrument by its number in the project's
/// list (sampler step 16: `Owner::KitPad`, `Owner::Sound`) reaches now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Named {
    /// The kit in this kit place.
    Kit(usize),
    /// The shared synth (the Synth strip's).
    SharedSynth,
    /// This holder of the instrument host: a track's own instrument, or a
    /// free place's (`instruments::free`).
    Holder(usize),
}

/// Instrument `n` of `listed`, as [`Named`]; None for a number the list
/// hasn't got.
fn named(listed: &instrument_list::InstrumentList, n: u16) -> Option<Named> {
    use instrument_list::Kind;
    let id = instrument_list::InstrumentId::from_number(u32::from(n))?;
    Some(match listed.kind(id)? {
        Kind::SharedSynth => Named::SharedSynth,
        Kind::Own { track } => Named::Holder(track),
        Kind::Free { place } => Named::Holder(instruments::free(place)),
        Kind::Kit { kit } => Named::Kit(kit),
    })
}

/// The kit a learned pad's target reaches now: its track's kit
/// (`Owner::Pad`), or the kit its number names (`Owner::KitPad`). None for
/// any other owner, a track that plays no kit, or a number the list
/// hasn't got.
fn learned_kit(
    listed: &instrument_list::InstrumentList,
    kits: &sampler::TrackKits,
    owner: mpfree_play::mpfree_param::Owner,
) -> Option<(usize, u8)> {
    use mpfree_play::mpfree_param::Owner;
    match owner {
        Owner::Pad(t, pad) => Some((kits.kit_of(usize::from(t))?, pad)),
        Owner::KitPad(n, pad) => match named(listed, n)? {
            Named::Kit(kit) => Some((kit, pad)),
            _ => None,
        },
        _ => None,
    }
}

/// A parameter MIDI Learn can move, described with its value now: the
/// mixer's and its effects' ([`modmix::describe`]), a track's synth's
/// (`Owner::Inst`, [`synth_describe`]); a track's arp's rate, gate, swing
/// and octaves (`Owner::Arp`, [`app::App::arp_param`]); a kit pad's own
/// setting (`Owner::Pad`, from `pads`); or the transport's tempo and PLAY
/// and STOP ([`remote::describe`], from `now`), a track's mute or solo on
/// the Clips grid ([`remote::describe_clips`]), or a loop's mute
/// ([`remote::describe_looper`]).
fn learn_describe(
    mix: &mixing::MixHost,
    synth: &synth::Synth,
    app: &app::App,
    listed: &instrument_list::InstrumentList,
    pads: PadNow<'_>,
    now: remote::Now,
    addr: mpfree_play::mpfree_param::ParamAddr,
) -> Option<(mpfree_node::ParamInfo, f32)> {
    use mpfree_play::mpfree_param::Owner;
    let t = match addr.owner {
        Owner::Inst(t) => t,
        Owner::Sound(n) => return sound_describe(mix, synth, named(listed, n)?, addr.param),
        Owner::Arp(t) => return app.arp_param(usize::from(t), addr.param),
        Owner::Pad(..) | Owner::KitPad(..) => return pads(addr.owner, addr.param),
        Owner::Transport => return remote::describe(addr.param, now),
        Owner::Clips => return remote::describe_clips(addr.param, now),
        Owner::Looper => return remote::describe_looper(addr.param, now),
        // A Perform pad is pressed, with no value to read.
        Owner::Perform => return None,
        Owner::Play => return remote::describe_play(addr.param, now),
        // A set's control is pressed, with no value to read.
        Owner::Show => return None,
        // A track's controller, bend or pressure out: where the engine has
        // it, else its default.
        Owner::Midi(_) => {
            let (info, d) = modmix::midi_param(addr)?;
            return Some((info, mix.modu.engine().value(addr).unwrap_or(d)));
        }
        _ => return modmix::describe(&mix.mixer, addr),
    };
    synth_describe(mix, synth, t, addr.param)
}

/// Kit `kit`'s pad `pad`'s own setting `param` (a `PadField` number)
/// with its value now, as the kit's view has it ([`sampler::pad_info`]):
/// None for a pad without a sound, or a number that names no setting.
fn pad_describe(
    player: &sampler::Player,
    kit: usize,
    pad: u8,
    param: u16,
) -> Option<(mpfree_node::ParamInfo, f32)> {
    let p = sampler::PadParam::of(param)?;
    let pad = usize::from(pad);
    let v = player.pad_param_now(kit, pad, p)?;
    let info = p.info(player.pad_slices(kit, pad));
    Some((info, info.clamp(v)))
}

/// A kit pad's parameter as the I/O view names it: "Tune (A5, Kit 2)",
/// or a layer's grain setting "Grain size (A5 layer 2, Kit 2)" (sampler
/// refactor step 17), with `whose` the kit or the track.
fn pad_label(param: u16, pad: u8, whose: &str) -> String {
    let p = sampler::PadParam::of(param);
    let name = p.map_or("?", |p| p.info(1).name);
    let pad = mpfree_midi::profile::pad_name(usize::from(pad));
    match p.and_then(sampler::PadParam::layer_shown) {
        Some(l) => format!("{name} ({pad} layer {l}, {whose})"),
        None => format!("{name} ({pad}, {whose})"),
    }
}

/// Track `t`'s synth's parameter `param` with its value now: the
/// instrument it plays (its own or another track's), else the synth on
/// the pads, as the Synth tab shows it.
fn synth_describe(
    mix: &mixing::MixHost,
    synth: &synth::Synth,
    t: u8,
    param: u16,
) -> Option<(mpfree_node::ParamInfo, f32)> {
    if let Some(owner) = mix.instruments.owner(usize::from(t)) {
        let node = mix.instruments.node(owner)?;
        let info = *node.params().iter().find(|p| p.id == param)?;
        return Some((info, node.param(param)));
    }
    Some((*synth.param_info(param)?, synth.value(param)))
}

/// A named instrument's parameter `param` with its value now (sampler step
/// 16, `Owner::Sound`): the shared synth's, or a holder's while it holds
/// an instrument. None for a kit.
fn sound_describe(
    mix: &mixing::MixHost,
    synth: &synth::Synth,
    to: Named,
    param: u16,
) -> Option<(mpfree_node::ParamInfo, f32)> {
    match to {
        Named::SharedSynth => Some((*synth.param_info(param)?, synth.value(param))),
        Named::Holder(h) => {
            let node = mix.instruments.node(h)?;
            let info = *node.params().iter().find(|p| p.id == param)?;
            Some((info, node.param(param)))
        }
        Named::Kit(_) => None,
    }
}

/// What a learned control moves, named for the I/O view and the hint line:
/// "Poly cutoff (Track 2)" for a synth's knob, "FM cutoff (FM · Pads)" for
/// one named by the instrument ([`sound_label`]), "Arp gate (Bass)" for a
/// track's arp, "Tune (A5, Drums)" for a kit pad's setting, "Tune (A5,
/// Kit 2)" for one named by the kit (sampler step 16), "PLAY and
/// STOP" or "Tempo" for the transport, "Launch Bass · row 2" or "Mute
/// Bass" on the Clips grid ([`clips_label`]), "Looper · Loop 3 key" or
/// "Looper · picked loop's key" on the looper, "Perform pads B2" on the
/// Perform pads, "Play · Swing" on Play's own controls, else as the chain
/// editor names it ([`modpanels::label`]).
fn learn_label(
    mix: &mixing::MixHost,
    synth: &synth::Synth,
    time: &time::TimeHost,
    listed: &instrument_list::InstrumentList,
    addr: mpfree_play::mpfree_param::ParamAddr,
) -> String {
    use mpfree_play::mpfree_param::{arp, looper, play, transport, Owner};
    let t = match addr.owner {
        Owner::Inst(t) => t,
        Owner::Sound(n) => return sound_label(mix, synth, named(listed, n), addr.param),
        Owner::KitPad(n, pad) => {
            let kit = match named(listed, n) {
                Some(Named::Kit(k)) => format!("Kit {}", k + 1),
                _ => format!("instrument {n}, gone"),
            };
            return pad_label(addr.param, pad, &kit);
        }
        Owner::Arp(t) => {
            return format!(
                "Arp {} ({})",
                arp::name(addr.param).unwrap_or("?").to_lowercase(),
                track_name(time.project(), t)
            )
        }
        Owner::Pad(t, pad) => return pad_label(addr.param, pad, &track_name(time.project(), t)),
        Owner::Transport => return transport::name(addr.param).unwrap_or("?").to_string(),
        Owner::Clips => return clips_label(time.project(), addr.param),
        Owner::Looper => {
            let name = looper::name(addr.param);
            return format!("Looper · {}", name.as_deref().unwrap_or("?"));
        }
        Owner::Perform => {
            let name = mpfree_play::mpfree_param::perform::name(addr.param);
            return format!("Perform pads {}", name.as_deref().unwrap_or("?"));
        }
        Owner::Play => return format!("Play · {}", play::name(addr.param).unwrap_or("?")),
        Owner::Show => {
            let name = mpfree_play::mpfree_param::show::name(addr.param);
            return format!("Show · {}", name.unwrap_or("?"));
        }
        _ => return modpanels::label(mix, addr),
    };
    // As the time engine and the I/O view name the tracks.
    let track = format!("Track {}", usize::from(t) + 1);
    let own = mix
        .instruments
        .holds(usize::from(t))
        .then(|| mix.instruments.node(usize::from(t)))
        .flatten();
    let (engine, param) = match own {
        Some(n) => (
            n.info().name,
            n.params()
                .iter()
                .find(|p| p.id == addr.param)
                .map(|p| p.name),
        ),
        None => (
            synth.engine_name(),
            synth.param_info(addr.param).map(|p| p.name),
        ),
    };
    format!("{engine} {} ({track})", param.unwrap_or("?").to_lowercase())
}

/// A knob of an instrument named by its number (sampler step 16), as the
/// I/O view and the hint line name it: the engine and the knob, then the
/// strip the instrument sounds on, "FM cutoff (FM · Pads)", "Poly cutoff
/// (Synth)"; "(gone)" for a number the list no longer has or a holder with
/// nothing in it.
fn sound_label(
    mix: &mixing::MixHost,
    synth: &synth::Synth,
    to: Option<Named>,
    param: u16,
) -> String {
    use looping::Instrument;
    let strip = |i: Instrument| {
        mix.instrument_strip(i, &[])
            .map_or_else(|| "gone".to_string(), |s| mix.strip_name(s))
    };
    let (engine, knob, place) = match to {
        Some(Named::SharedSynth) => (
            synth.engine_name(),
            synth.param_info(param).map(|p| p.name),
            strip(Instrument::Synth),
        ),
        Some(Named::Holder(h)) => match mix.instruments.node(h) {
            Some(n) => (
                n.info().name,
                n.params().iter().find(|p| p.id == param).map(|p| p.name),
                strip(Instrument::Own(h)),
            ),
            None => ("Synth", None, "gone".to_string()),
        },
        _ => ("Synth", None, "gone".to_string()),
    };
    format!("{engine} {} ({place})", knob.unwrap_or("?").to_lowercase())
}

/// The I/O view's track names (`Live::tracks`, one a track it counts) as
/// the project names its tracks; a track the project hasn't got, or one
/// without a name, is "Track N". Allocates only for a name that changed.
fn io_track_names(shown: &mut [String], tracks: &[mpfree_seq::Track]) {
    for (i, s) in shown.iter_mut().enumerate() {
        match tracks
            .get(i)
            .map(|t| t.name.as_str())
            .filter(|n| !n.is_empty())
        {
            Some(name) if s != name => *s = name.to_string(),
            Some(_) => {}
            None => {
                let numbered = s
                    .strip_prefix("Track ")
                    .and_then(|n| n.parse::<usize>().ok())
                    == Some(i + 1);
                if !numbered {
                    *s = format!("Track {}", i + 1);
                }
            }
        }
    }
}

/// The synths a MIDI route can name past the tracks (the instrument
/// redesign's step 11e), for the Routing card's stepper and the monitor:
/// the shared synth, then each free place's synth (one picked in the list
/// of synths), by the project's instrument number and channel name ("FM ·
/// Pads"). `shown` changes only when the list does; twice a second, with
/// the tracks' names.
fn io_route_synths(
    shown: &mut Vec<(u32, String)>,
    listed: &instrument_list::InstrumentList,
    mix: &mixing::MixHost,
) {
    use looping::Instrument;
    let name = |i: Instrument| {
        mix.instrument_strip(i, &[])
            .map_or_else(String::new, |s| mix.strip_name(s))
    };
    let shared = instrument_list::InstrumentId::SHARED_SYNTH;
    let mut now = vec![(shared.number(), name(Instrument::Synth))];
    for n in 0..instruments::FREE {
        let holder = instruments::free(n);
        if let (Some(id), true) = (listed.free_of(n), mix.instruments.holds(holder)) {
            now.push((id.number(), name(Instrument::Own(holder))));
        }
    }
    if *shown != now {
        *shown = now;
    }
}

/// Track `t` as the project names it; one past the project's tracks is
/// named by its number.
fn track_name(project: &mpfree_seq::Project, t: u8) -> String {
    project.tracks.get(usize::from(t)).map_or_else(
        || format!("Track {}", u16::from(t) + 1),
        |tr| tr.name.clone(),
    )
}

/// A Clips grid parameter named as the Learn section and the hint line
/// show it: "Launch Bass · row 2", "Scene 3 · Verse", "Stop Bass", "Mute
/// Bass", "Solo Bass", "Arm Bass", "Stop all", "Back to song", "Capture
/// scene". A track past the project's is named by its number.
fn clips_label(project: &mpfree_seq::Project, id: u16) -> String {
    use mpfree_play::mpfree_param::clips::{self, Clips};
    let track = |t: u8| track_name(project, t);
    match clips::of(id) {
        None => "?".to_string(),
        Some(Clips::StopAll) => "Stop all".to_string(),
        Some(Clips::BackToSong) => "Back to song".to_string(),
        Some(Clips::CaptureScene) => "Capture scene".to_string(),
        Some(Clips::Scene(r)) => match project.scenes.get(usize::from(r)).map(|s| &s.name) {
            Some(name) if !name.is_empty() => format!("Scene {} · {name}", u16::from(r) + 1),
            _ => format!("Scene {}", u16::from(r) + 1),
        },
        Some(Clips::Clip(t, r)) => format!("Launch {} · row {}", track(t), u16::from(r) + 1),
        Some(Clips::Stop(t)) => format!("Stop {}", track(t)),
        Some(Clips::Mute(t)) => format!("Mute {}", track(t)),
        Some(Clips::Solo(t)) => format!("Solo {}", track(t)),
        Some(Clips::Arm(t)) => format!("Arm {}", track(t)),
    }
}

/// A learned control's 0 to 1 on a track's synth (`Owner::Inst`), set as
/// the Synth tab's knob sets it: the track's own instrument while it holds
/// one, else the synth on the pads (an undoable turn, as on the tab). None
/// for a parameter the synth doesn't have, or one that only shows a
/// reading.
fn learned_synth(
    synth: &mut synth::Synth,
    mix: &mut mixing::MixHost,
    listed: &instrument_list::InstrumentList,
    w: mpfree_midi::learn::Write,
) -> Option<f32> {
    use mpfree_play::mpfree_param::Owner;
    let t = match w.target.owner {
        Owner::Inst(t) => t,
        // Named by the instrument (sampler step 16): that one, whichever
        // track plays it, as the Synth tab turns it.
        Owner::Sound(n) => {
            let to = named(listed, n)?;
            let (info, _) = sound_describe(mix, synth, to, w.target.param)?;
            if info.readonly {
                return None;
            }
            let value = info.from_unit(w.value);
            return match to {
                Named::SharedSynth => {
                    synth.set(w.target.param, value);
                    Some(value)
                }
                Named::Holder(h) => mix
                    .instruments
                    .param(h, w.target.param, value)
                    .then_some(value),
                Named::Kit(_) => None,
            };
        }
        _ => return None,
    };
    let (info, _) = synth_describe(mix, synth, t, w.target.param)?;
    if info.readonly {
        return None;
    }
    let value = info.from_unit(w.value);
    let t = usize::from(t);
    if mix.instruments.plays(t) {
        return mix
            .instruments
            .param(t, w.target.param, value)
            .then_some(value);
    }
    synth.set(w.target.param, value);
    Some(value)
}

/// A learned target as the armed track records it (sampler step 16): one
/// named by its kit (`Owner::KitPad`) or its instrument (`Owner::Sound`)
/// becomes that pad or instrument on the armed track while the armed
/// track plays it, so REC keeps the move as the track's lock; else it
/// stays as it is, and [`record_learned`] records nothing of it. Every
/// other target stays as it is.
fn learned_on_armed(
    addr: mpfree_play::mpfree_param::ParamAddr,
    armed: Option<usize>,
    kits: &sampler::TrackKits,
    own: &instruments::Instruments,
    listed: &instrument_list::InstrumentList,
) -> mpfree_play::mpfree_param::ParamAddr {
    use mpfree_play::mpfree_param::{Owner, ParamAddr};
    let Some(a) = armed else { return addr };
    let Ok(t) = u8::try_from(a) else { return addr };
    let plays = |to: Named| match to {
        Named::Kit(k) => !own.plays(a) && kits.kit_of(a) == Some(k),
        Named::SharedSynth => !own.plays(a) && kits.plays_synth(a),
        Named::Holder(h) => own.owner(a) == Some(h),
    };
    match addr.owner {
        Owner::KitPad(n, pad) if named(listed, n).is_some_and(plays) => {
            ParamAddr::new(Owner::Pad(t, pad), addr.param)
        }
        Owner::Sound(n) if named(listed, n).is_some_and(plays) => {
            ParamAddr::new(Owner::Inst(t), addr.param)
        }
        _ => addr,
    }
}

/// Knob recording for a learned control (the MIDI plan's 2.5): while the
/// transport records, the value it set goes to the recorder as a screen
/// knob's turn does ([`record_knob`]), for a parameter on a track that a
/// clip's lock plays: a synth's knob, a kit pad's own setting, a track
/// strip's control or its insert's knob, a MIDI effect's setting or the
/// track's MIDI out. The recorder keeps the armed track's, as a lock on
/// the step it belongs to; the arp's settings, the transport, the Clips
/// grid, the looper, the mixer's other strips and the macros record
/// nothing.
fn record_learned(
    time: &mut time::TimeHost,
    addr: mpfree_play::mpfree_param::ParamAddr,
    value: f32,
    recording: bool,
) {
    if recording && learn_records(addr) && value.is_finite() {
        time.knob(0, addr, value);
    }
}

/// Whether a learned control's move on `addr` records as a lock
/// ([`record_learned`]): a parameter on a track that a clip's lock plays,
/// a layer's grain setting among a pad's (sampler refactor step 17b).
fn learn_records(addr: mpfree_play::mpfree_param::ParamAddr) -> bool {
    use mpfree_play::mpfree_param::{Owner, Strip};
    match addr.owner {
        Owner::Pad(..) => sampler::PadParam::of(addr.param).is_some(),
        Owner::Inst(_)
        | Owner::Strip(Strip::Track(_))
        | Owner::Insert(Strip::Track(_), _)
        | Owner::NoteFx(..)
        | Owner::Midi(_) => true,
        _ => false,
    }
}

/// The values the Synth tab shows for track `track`: the instrument's it
/// plays (its own or another track's), else the synth on the pads' (as
/// [`synth_tab`]).
fn synth_tab_values(synth: &synth::Synth, mix: &mixing::MixHost, track: usize) -> Vec<f32> {
    match mix
        .instruments
        .node(instrument_track(&mix.instruments, track))
    {
        Some(n) => n.params().iter().map(|p| n.param(p.id)).collect(),
        None => synth.values(),
    }
}

/// The track whose instrument track `track` plays: the one it sends to
/// (step 4), else itself. The Synth tab, its presets and a plugin's skin
/// show that track's instrument.
fn instrument_track(own: &instruments::Instruments, track: usize) -> usize {
    own.owner(track).unwrap_or(track)
}

/// Whose instrument the Synth tab shows and edits: the one the pads play
/// by the host's pads player while the selected track doesn't play it
/// (step 6, a synth picked in the list), else the selected track
/// ([`instrument_track`] goes on from there).
fn tab_who(app: &app::App, own: &instruments::Instruments) -> usize {
    match own.pads_play_at() {
        Some(h) if own.owner(app.track()) != Some(h) => h,
        _ => app.track(),
    }
}

/// Who plays the instrument the Synth tab shows (the instrument
/// redesign's rule 13, "Poly: the pads, Track 2, Track 4", step 8b):
/// None while only the pads, or only the selected track, play it, which
/// the hint line already says. Tracks by their names, the first four,
/// then how many more. Allocates only when called; the loop calls it a
/// few times a second and sends it when it changes.
fn who_plays(
    app: &app::App,
    own: &instruments::Instruments,
    kits: &sampler::TrackKits,
    names: &[mpfree_seq::Track],
    synth_name: &str,
) -> Option<String> {
    use app::PadFocus;
    let shown = instrument_track(own, tab_who(app, own));
    let (name, tracks, (every, banks)) = match own.held(shown) {
        // An instrument the host holds: its own track's, then those that
        // send to it; the pads by their choice (a bank's own too) or by
        // following the track.
        Some(h) => {
            let mut bits = own.senders(shown);
            if shown < instruments::TRACKS {
                bits |= 1 << shown;
            }
            let pads = app.pads_who(|f| match f {
                PadFocus::Instrument(id) => app.instrument_holder(id) == Some(shown),
                PadFocus::Track => own.owner(app.track()) == Some(shown),
            });
            (h.name, bits, pads)
        }
        // The shared synth: the tracks on the Synth kit; the pads while
        // chosen for it or following such a track.
        None => {
            let bits = (0..instruments::TRACKS)
                .filter(|&t| !own.plays(t) && kits.plays_synth(t))
                .fold(0u128, |b, t| b | 1 << t);
            let pads = app.pads_who(|f| match f {
                PadFocus::Instrument(_) => f == PadFocus::SYNTH,
                PadFocus::Track => !own.plays(app.track()) && kits.plays_synth(app.track()),
            });
            (synth_name, bits, pads)
        }
    };
    let pads = every || banks != 0;
    let count = tracks.count_ones() as usize + usize::from(pads);
    if count < 2 {
        return None;
    }
    let track_name = |t: usize| {
        names
            .get(t)
            .map(|n| n.name.trim())
            .filter(|n| !n.is_empty())
            .map_or_else(|| format!("Track {}", t + 1), str::to_string)
    };
    let mut who: Vec<String> = Vec::new();
    if every {
        who.push("the pads".to_string());
    } else if banks != 0 {
        // Only banks with one of their own (step 6c): "pads bank B",
        // "pads banks B and D", "pads banks A, B and D".
        let letters: Vec<String> = (0..8)
            .filter(|&b| banks >> b & 1 == 1)
            .map(|b| app::bank_letter(b).to_string())
            .collect();
        who.push(match letters.split_last() {
            Some((last, [])) => format!("pads bank {last}"),
            Some((last, rest)) => format!("pads banks {} and {last}", rest.join(", ")),
            None => String::new(),
        });
    }
    let listed: Vec<usize> = (0..instruments::TRACKS)
        .filter(|&t| tracks >> t & 1 == 1)
        .collect();
    let room = 4 - who.len();
    who.extend(listed.iter().take(room).map(|&t| track_name(t)));
    if listed.len() > room {
        who.push(format!("{} more", listed.len() - room));
    }
    Some(format!("{name}: {}", who.join(", ")))
}

/// A track's line (the instrument redesign's step 11d, the plan's "a
/// track's line"): what track `t` plays and where it is heard, shown over
/// the Instrument row's help in the Synth tab's options: ["Track 2 plays
/// Track 1's Poly", "In the mixer: Poly · Track 1"]. The mixer names the
/// channel (`MixHost::instrument_strip`, `MixHost::strip_name`), so it
/// reads as the Mix tab does. `homes` say where each kit plays
/// (`TrackKits::homes`); `midi_out`: the track sends MIDI out. Allocates;
/// the loop asks a few times a second while the options show.
fn track_line(
    t: usize,
    mix: &mixing::MixHost,
    kits: &sampler::TrackKits,
    homes: &[u8],
    synth_name: &str,
    midi_out: bool,
) -> Vec<String> {
    use looping::Instrument;
    let own = &mix.instruments;
    let who = format!("Track {}", t + 1);
    let i = if midi_out {
        Instrument::MidiOut
    } else if let Some(o) = own.owner(t) {
        Instrument::Own(o)
    } else {
        kits.kit_of(t).map_or(Instrument::Synth, Instrument::Kit)
    };
    let plays = match i {
        Instrument::Own(o) => {
            let name = own.held(o).map_or("an instrument", |h| h.name);
            if o == t {
                format!("{who} plays its own {name}")
            } else if o < instruments::TRACKS {
                format!("{who} plays Track {}'s {name}", o + 1)
            } else {
                format!("{who} plays {name}, from the list of synths")
            }
        }
        Instrument::Kit(k) => format!("{who} plays Kit {}", k + 1),
        Instrument::Synth => format!("{who} plays the shared {synth_name}"),
        Instrument::MidiOut => format!("{who} plays gear on MIDI out"),
    };
    let heard = match mix.instrument_strip(i, homes) {
        Some(s) => format!("In the mixer: {}", mix.strip_name(s)),
        None => "In the mixer: nothing, it sounds outside mpfree".to_string(),
    };
    vec![plays, heard]
}

/// The synths MIDI routes name (the instrument redesign's step 11e,
/// app/route_synths.rs): each route row of the instrument host
/// (`instruments::routed`) points at the synth its routes name, or plays
/// the shared synth by note; and while REC records one, the route's keys
/// go into the track that records it, as the pads' layers' do
/// (`PadLayers`). A key's let-go, pressure and slide reach where it went
/// though the row moved on meanwhile. Main.rs and the test session both
/// use it. Allocates nothing.
struct RouteSynths {
    /// The rows that play the shared synth, a bit each.
    synth: u8,
    /// The keys each row holds on the shared synth, a bit each.
    synth_keys: [u128; RouteSynths::ROWS],
    /// The pedals each row's keys put down.
    pedals: [midi_in::PedalsDown; RouteSynths::ROWS],
    /// The track recording each row's synth now (`App::route_recs`).
    rec: [Option<u8>; RouteSynths::ROWS],
    /// The pad each key went to on the track it went into.
    keys: midi_in::RoutedPads,
    /// The tracks each row's keys went into, a bit each, for their
    /// let-go.
    key_tracks: [u128; RouteSynths::ROWS],
    /// The kit each row names (sampler step 15f, `App::route_synth_kits`).
    kits: [Option<u8>; RouteSynths::ROWS],
    /// Where each row's keys went on its kit, (kit, where) per note, for
    /// their let-go and pressure though the row moved on since.
    kit_keys: [[Option<(u8, sampler::NoteTo)>; 128]; RouteSynths::ROWS],
    /// The kits the rows' wheels bent, to bend back once no row names
    /// them.
    bent: [Option<u8>; RouteSynths::ROWS],
    /// A kit row's kit as the track recording it takes notes.
    rec_kits: [Option<LayerKit>; RouteSynths::ROWS],
}

impl RouteSynths {
    const ROWS: usize = instruments::ROUTE_SYNTHS;

    fn new() -> RouteSynths {
        RouteSynths {
            synth: 0,
            synth_keys: [0; RouteSynths::ROWS],
            pedals: std::array::from_fn(|_| midi_in::PedalsDown::default()),
            rec: [None; RouteSynths::ROWS],
            keys: midi_in::RoutedPads::new(),
            key_tracks: [0; RouteSynths::ROWS],
            kits: [None; RouteSynths::ROWS],
            kit_keys: [[None; 128]; RouteSynths::ROWS],
            bent: [None; RouteSynths::ROWS],
            rec_kits: [None; RouteSynths::ROWS],
        }
    }

    /// The kits the rows name as the app has them now
    /// (`App::route_synth_kits`, sampler step 15f): each row's keys play
    /// its kit from then on; a key held goes on where it went. Returns the
    /// kits the wheel bent that no row names any more, to bend back.
    fn kits_to(
        &mut self,
        kits: [Option<u8>; RouteSynths::ROWS],
    ) -> [Option<u8>; RouteSynths::ROWS] {
        self.kits = kits;
        let gone = self.bent.map(|b| b.filter(|k| !kits.contains(&Some(*k))));
        self.bent = self.bent.map(|b| b.filter(|k| kits.contains(&Some(*k))));
        gone
    }

    /// Route row `j`'s key `k` on the kit it names (sampler step 15f), as
    /// the kit's Keys play sends it (`sampler::Player::note_to`: the pad
    /// that sends the note, or the keygroups that hold it): (kit, where),
    /// kept for its let-go and pressure, which go where it went though the
    /// row moved on since. None for a row that names no kit, a second
    /// note-on for a key held there, the wheel and the pedals.
    fn kit_key(
        &mut self,
        j: usize,
        k: midi_in::Key,
        player: &sampler::Player,
    ) -> Option<(u8, sampler::NoteTo)> {
        use midi_in::Key;
        let kit = self.kits.get(j).copied().flatten();
        let held = self.kit_keys.get_mut(j)?;
        match k {
            Key::Down { note, .. } => {
                let at = &mut held[usize::from(note & 0x7F)];
                if at.is_some() {
                    return None;
                }
                let kit = kit?;
                *at = Some((kit, player.note_to(usize::from(kit), note)));
                *at
            }
            Key::Up { note } => held[usize::from(note & 0x7F)].take(),
            Key::Pressure { note, .. } => held[usize::from(note & 0x7F)],
            _ => None,
        }
    }

    /// The kit route row `j`'s wheel bends now (its keygroups), kept so
    /// it is bent back once no row names it ([`RouteSynths::kits_to`]).
    fn kit_bend(&mut self, j: usize) -> Option<u8> {
        let kit = self.kits.get(j).copied().flatten()?;
        self.bent[j] = Some(kit);
        Some(kit)
    }

    /// The keys held on the rows' kits now (for the tests).
    #[cfg(test)]
    fn kit_keys_held(&self) -> usize {
        self.kit_keys
            .iter()
            .flatten()
            .filter(|k| k.is_some())
            .count()
    }

    /// The rows as the app has them now (`App::route_synth_holders`,
    /// `App::route_synth_shared`): each pointed at its holder, the shared
    /// synth's rows on or off. The notes and pedals a row leaves end
    /// there first. Nothing changes when nothing did.
    fn follow(
        &mut self,
        holders: [Option<usize>; RouteSynths::ROWS],
        shared: u8,
        synth: &mut synth::Synth,
        own: &mut instruments::Instruments,
    ) {
        for (j, h) in holders.into_iter().enumerate() {
            let to = h.filter(|&h| own.holds(h));
            let (on, was) = (shared >> j & 1 == 1, self.synth >> j & 1 == 1);
            if own.routed_at(j) == to && on == was {
                continue;
            }
            for ctl in self.pedals[j].lift() {
                own.control(instruments::routed(j), ctl, 0.0, 0);
                if was {
                    synth.control(ctl, 0.0, 0);
                }
            }
            if own.routed_at(j) != to {
                own.set_routed(j, to);
            }
            if was && !on {
                let held = std::mem::take(&mut self.synth_keys[j]);
                for note in (0..128u8).filter(|&n| held >> n & 1 == 1) {
                    synth.note_off_note(note, 0);
                }
            }
            self.synth = self.synth & !(1 << j) | u8::from(on) << j;
        }
    }

    /// Route row `j`'s key `k` on its synth, each key its own note at its
    /// own velocity, as a route to a track plays it. A second note-on for
    /// a key held there is ignored; a let-go, pressure or slide reaches
    /// where the key went.
    fn key(
        &mut self,
        j: usize,
        k: midi_in::Key,
        synth: &mut synth::Synth,
        own: &mut instruments::Instruments,
    ) {
        use midi_in::Key;
        if j >= RouteSynths::ROWS {
            return;
        }
        let row = instruments::routed(j);
        let on = self.synth >> j & 1 == 1;
        let bit = |note: u8| 1u128 << (note & 0x7F);
        let held = &mut self.synth_keys[j];
        match k {
            Key::Down { note, velocity } => {
                let vel = velocity.max(1);
                if !own.key_held(row, note) {
                    own.note_on(row, note, vel, 0);
                }
                if on && *held & bit(note) == 0 {
                    synth.note_on_note(note, vel, 0);
                    *held |= bit(note);
                }
            }
            Key::Up { note } => {
                if own.key_held(row, note) {
                    own.note_off(row, note, 0);
                }
                if *held & bit(note) != 0 {
                    synth.note_off_note(note, 0);
                    *held &= !bit(note);
                }
            }
            Key::Pressure { note, value } => {
                if own.key_held(row, note) {
                    own.note_pressure(row, note, value);
                }
                if *held & bit(note) != 0 {
                    synth.pressure_note(note, value);
                }
            }
            Key::Expr { note, kind, value } => {
                if own.key_held(row, note) {
                    own.note_expression(row, note, kind, value);
                }
                if *held & bit(note) != 0 {
                    synth.expression_note(note, kind, value);
                }
            }
            Key::Bend { value } => {
                if !own.bend(row, value) && on {
                    synth.bend(value);
                }
            }
            Key::Control { ctl, value } => {
                if !own.control(row, ctl, value, 0) && on {
                    synth.control(ctl, value, 0);
                }
                self.pedals[j].key(ctl, value);
            }
        }
    }

    /// The tracks recording the rows' synths now (`App::route_recs`), a
    /// kit row's with how its kit takes notes now (`keys`,
    /// `sampler::Player::kit_keys`; after [`RouteSynths::kits_to`]).
    fn rec_to(
        &mut self,
        recs: [Option<u8>; RouteSynths::ROWS],
        keys: impl Fn(usize) -> mpfree_sampler::engine::KitKeys,
    ) {
        self.rec = recs;
        self.rec_kits = self.kits.map(|k| {
            k.map(|kit| LayerKit {
                kit,
                page: 0,
                keys: keys(usize::from(kit)),
            })
        });
    }

    /// Route row `j`'s key into the track recording its synth, as the pad
    /// its note plays there (`midi_file::pad_on_track`), or as a kit
    /// row's kit takes it ([`LayerKit::key_note`], sampler step 15f); its
    /// end and pressure where it went.
    fn rec_key(
        &mut self,
        j: usize,
        k: midi_in::Key,
        time: &mut time::TimeHost,
        own: &instruments::Instruments,
        synth: &synth::Synth,
    ) {
        use midi_in::Key;
        if j >= RouteSynths::ROWS {
            return;
        }
        match k {
            Key::Down { .. } => {
                let Some(t) = self.rec[j] else {
                    return;
                };
                let kit = self.rec_kits[j];
                let pad = |n| match kit {
                    Some(k) => k.key_note(n),
                    None => midi_file::pad_on_track(own, Some(synth), usize::from(t), n),
                };
                if let Some(ev) = self.keys.event(t, k, pad) {
                    time.routed_played(t, 0, ev);
                    self.key_tracks[j] |= 1 << t;
                }
            }
            Key::Up { .. } | Key::Pressure { .. } => {
                let mut went = self.key_tracks[j];
                while went != 0 {
                    let t = went.trailing_zeros() as u8;
                    went &= went - 1;
                    if let Some(ev) = self.keys.event(t, k, |_| None) {
                        time.routed_played(t, 0, ev);
                    }
                }
            }
            _ => {}
        }
    }
}

/// What the pads play on their own, before the arp and the recorder,
/// while the armed track plays something else (app/pads_focus.rs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PadsPreview {
    /// The shared synth, chosen with Play on the pads.
    Synth,
    /// An instrument the host holds, by its pads player (step 6).
    Host,
    /// A kit chosen for the pads (sampler step 15): its pads play, each
    /// hit at the pad of the kit's page the pads are on.
    Kit(u8),
}

/// A keyboard's keys while the pads play a synth on their own (the
/// instrument redesign's step 7b): they play it too, each key its own
/// note, before the arp and the recorder, as the pads' hits do, so "Pads:
/// FM" names what both play. Each held key remembers where it went, so
/// its let-go, pressure and slide reach the same synth though the pads
/// moved on meanwhile (rule 8). Main.rs and the test session both use it.
/// Allocates nothing.
struct KeysOnPads {
    held: [Option<PadsPreview>; 128],
}

impl KeysOnPads {
    fn new() -> KeysOnPads {
        KeysOnPads { held: [None; 128] }
    }

    /// Key `k`, while the pads play `preview` on their own (None: they
    /// play through the armed track): true when it went to the pads'
    /// synth, so the armed track doesn't hear it. `v` shapes a velocity as
    /// the pads' (`App::velocity`).
    fn key(
        &mut self,
        k: midi_in::Key,
        preview: Option<PadsPreview>,
        v: impl Fn(u8) -> u8,
        synth: &mut synth::Synth,
        own: &mut instruments::Instruments,
    ) -> bool {
        use midi_in::Key;
        let pads = instruments::THE_PADS;
        let at = |note: u8| usize::from(note & 0x7F);
        match k {
            Key::Down { note, velocity } => {
                let Some(p) = preview else {
                    return false;
                };
                // A second note-on for a key held there is ignored.
                if self.held[at(note)].is_some() {
                    return true;
                }
                let vel = v(velocity);
                match p {
                    PadsPreview::Host => {
                        own.note_on(pads, note, vel, 0);
                    }
                    PadsPreview::Synth => synth.note_on_note(note, vel, 0),
                    // A kit on the pads: the keys play the armed track as
                    // before (sampler step 15 leaves them there).
                    PadsPreview::Kit(_) => return false,
                }
                self.held[at(note)] = Some(p);
                true
            }
            Key::Up { note } => match self.held[at(note)].take() {
                Some(PadsPreview::Host) => {
                    own.note_off(pads, note, 0);
                    true
                }
                Some(PadsPreview::Synth) => {
                    synth.note_off_note(note, 0);
                    true
                }
                Some(PadsPreview::Kit(_)) | None => false,
            },
            Key::Pressure { note, value } => match self.held[at(note)] {
                Some(PadsPreview::Host) => {
                    own.note_pressure(pads, note, value);
                    true
                }
                Some(PadsPreview::Synth) => {
                    synth.pressure_note(note, value);
                    true
                }
                Some(PadsPreview::Kit(_)) | None => false,
            },
            Key::Expr { note, kind, value } => match self.held[at(note)] {
                Some(PadsPreview::Host) => {
                    own.note_expression(pads, note, kind, value);
                    true
                }
                Some(PadsPreview::Synth) => {
                    synth.expression_note(note, kind, value);
                    true
                }
                Some(PadsPreview::Kit(_)) | None => false,
            },
            Key::Bend { value } => match preview {
                Some(PadsPreview::Host) => {
                    own.bend(pads, value);
                    true
                }
                Some(PadsPreview::Synth) => {
                    synth.bend(value);
                    true
                }
                Some(PadsPreview::Kit(_)) | None => false,
            },
            Key::Control { ctl, value } => KeysOnPads::control(preview, ctl, value, synth, own),
        }
    }

    /// A pedal, the mod wheel or a breath for the pads' synth (also a
    /// pedal coming up as the pads move on): true when it went there.
    fn control(
        to: Option<PadsPreview>,
        ctl: mpfree_node::Ctl,
        value: f32,
        synth: &mut synth::Synth,
        own: &mut instruments::Instruments,
    ) -> bool {
        match to {
            Some(PadsPreview::Host) => {
                own.control(instruments::THE_PADS, ctl, value, 0);
                true
            }
            Some(PadsPreview::Synth) => {
                synth.control(ctl, value, 0);
                true
            }
            Some(PadsPreview::Kit(_)) | None => false,
        }
    }

    /// How many keys play the pads' synth now (for the tests).
    #[cfg(test)]
    fn held(&self) -> usize {
        self.held.iter().filter(|h| h.is_some()).count()
    }
}

/// A note the pads' arp plays ([`PadsArp`]), `offset` frames into the
/// period: a kit's pad, the synth's note or a host instrument's pad, as
/// what it plays has it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ArpNote {
    On { note: u8, velocity: u8, offset: u32 },
    Off { note: u8, offset: u32 },
}

/// The pads' arp (sampler step 15c, the plan's "Note Repeat and the arp
/// work on any pad"): NOTE REPEAT, Latch and the arp while the pads play
/// an instrument on their own ([`PadsPreview`]: a synth, a whole kit, a
/// bank's own), so the arp never gives the pads back. It is the selected
/// track's arp as the Arp panel and NOTE REPEAT set it (its settings,
/// latch, pattern, swing and STOP's hush, taken each period by
/// [`PadsArp::follow`]) on an engine of its own, so the track's clips
/// through its arp never mix with what the pads play. Each hit goes in
/// as what it plays: a kit's pad of the page the pads are on, the synth's
/// notes for the pad (the destination synth's notes, as its pad layout
/// has them), or a host instrument's pad; and each note it makes plays
/// there. When the pads move to another instrument, or back to the track,
/// its notes end where they played and nothing stays held or latched
/// ([`PadsArp::leave`]); a pad still held then lets go quietly. While a
/// track records what the pads play on their own (REC set it up, step
/// 15c's second part: `App::pads_own_rec`), each note goes there too, as
/// heard, and its end goes where it went: the `play` closure gets the
/// track with each note. Main.rs and the test session both use it.
/// Allocates nothing once made.
struct PadsArp {
    host: Box<mpfree_play::ArpHost>,
    /// What its notes play now (None until a hit comes in).
    to: Option<PadsPreview>,
    /// The track recording what the pads play now ([`PadsArp::rec_to`]).
    rec: Option<u8>,
    /// The track each sounding note went into ([`NO_REC`]: none).
    fed: [u8; 128],
    /// The notes each pad pressed into it, and how many (0: none).
    pressed: [([u8; MAX_PAD_NOTES], u8); 16],
    /// The pads whose hit was in it when the pads moved on: their let-go
    /// is taken quietly. A bit each.
    quiet: u16,
    /// Its notes sounding now, a bit each.
    on: u128,
}

impl PadsArp {
    fn new(sample_rate: u32, bpm: f64) -> PadsArp {
        PadsArp {
            host: Box::new(mpfree_play::ArpHost::new(mpfree_arp::Engine::new(
                sample_rate,
                bpm,
            ))),
            to: None,
            rec: None,
            fed: [NO_REC; 128],
            pressed: [([0; MAX_PAD_NOTES], 0); 16],
            quiet: 0,
            on: 0,
        }
    }

    /// The track recording what the pads play on their own now (None:
    /// none, or REC writes steps): the notes that start from now go into it
    /// too.
    fn rec_to(&mut self, rec: Option<u8>) {
        self.rec = rec;
    }

    /// The track note `note`'s start went into, forgotten.
    fn fed_off(&mut self, note: u8) -> Option<u8> {
        let t = std::mem::replace(&mut self.fed[usize::from(note & 127)], NO_REC);
        (t != NO_REC).then_some(t)
    }

    /// Takes the selected track's arp as the app set it (`armed`, the
    /// engine the pads play through on a track): its settings, latch,
    /// pattern, phrase and key, tempo and swing (`own_swing`: its own, not
    /// the transport's), and STOP's hush. The mode is `mode`, the app's
    /// word for what the pads play on their own (`App::pads_own_mode`):
    /// the track's, but Phrase only on the shared synth, whose pads press
    /// notes; on a kit or a host's pads they play as pressed. A phrase the
    /// track picked comes in here on its line, as on the track. Allocates
    /// nothing.
    fn follow(&mut self, armed: &mpfree_arp::Engine, mode: mpfree_arp::Mode, own_swing: bool) {
        let e = &mut self.host.engine;
        let settings = mpfree_arp::Settings {
            mode,
            ..armed.settings()
        };
        if e.settings() != settings {
            e.set_settings(settings);
        }
        if e.phrase_picked() != armed.phrase_picked() {
            e.set_phrase(*armed.phrase_picked());
        }
        if e.key() != armed.key() {
            e.set_key(armed.key());
        }
        if e.latch() != armed.latch() {
            e.set_latch(armed.latch());
        }
        if e.pattern() != armed.pattern() {
            e.set_pattern(*armed.pattern());
        }
        e.set_tempo(armed.bpm());
        e.set_own_swing(own_swing);
        if armed.hushed() && !e.hushed() {
            e.hush();
        } else if !armed.hushed() {
            e.resume();
        }
    }

    /// Where its phrase is, in the phrase's ticks, while it plays one (for
    /// the Phrase tab's cursor, `App::set_pads_own`).
    fn phrase_at(&self) -> Option<f64> {
        self.host.engine.phrase_at()
    }

    /// Whether pad `pad`'s hit went into the arp: its let-go and pressure
    /// come here.
    fn holds(&self, pad: usize) -> bool {
        self.pressed.get(pad).is_some_and(|p| p.1 > 0) || self.quiet & bit16(pad) != 0
    }

    /// Pad `pad` pressed at `velocity` while the pads play `to` on their
    /// own: `notes` go into the arp (a kit's pad, the synth's notes for the
    /// pad, a host's pad; the first [`MAX_PAD_NOTES`]). On another
    /// instrument than the arp plays, its notes there end first
    /// ([`PadsArp::leave`]). A pad pressed again lets go first.
    fn down(
        &mut self,
        pad: usize,
        notes: &[u8],
        velocity: u8,
        to: PadsPreview,
        play: &mut impl FnMut(PadsPreview, ArpNote, Option<u8>),
    ) {
        use mpfree_play::Event;
        if self.to != Some(to) {
            self.leave(play);
            self.to = Some(to);
        }
        if pad >= 16 {
            return;
        }
        self.up(pad);
        let n = notes.len().min(MAX_PAD_NOTES);
        let velocity = velocity.max(1);
        for &note in &notes[..n] {
            let on = Event::NoteOn { note, velocity };
            self.host.input_from(mpfree_play::arp::PADS, 0, on);
        }
        self.pressed[pad].0[..n].copy_from_slice(&notes[..n]);
        self.pressed[pad].1 = n as u8;
    }

    /// Pad `pad` let go: its notes leave the arp (a latched arp plays on).
    fn up(&mut self, pad: usize) {
        use mpfree_play::Event;
        self.quiet &= !bit16(pad);
        let Some((notes, n)) = self.pressed.get_mut(pad) else {
            return;
        };
        for &note in &notes[..usize::from(*n)] {
            self.host
                .input_from(mpfree_play::arp::PADS, 0, Event::NoteOff { note });
        }
        *n = 0;
    }

    /// A held pad's pressure, for the arp's notes of it.
    fn pressure(&mut self, pad: usize, value: u8) {
        use mpfree_play::Event;
        let Some((notes, n)) = self.pressed.get(pad) else {
            return;
        };
        for &note in &notes[..usize::from(*n)] {
            let p = Event::Pressure { note, value };
            self.host.input_from(mpfree_play::arp::PADS, 0, p);
        }
    }

    /// The pads move to another instrument, or back to the track: every
    /// note the arp has sounding ends where it plays, now, and nothing
    /// stays held or latched; a pad still held lets go quietly.
    fn leave(&mut self, play: &mut impl FnMut(PadsPreview, ArpNote, Option<u8>)) {
        if let Some(to) = self.to.take() {
            let on = self.on;
            for note in (0..128u8).filter(|&n| on & 1 << n != 0) {
                let rec = self.fed_off(note);
                play(to, ArpNote::Off { note, offset: 0 }, rec);
            }
        }
        self.on = 0;
        self.fed = [NO_REC; 128];
        for pad in 0..16 {
            if self.pressed[pad].1 > 0 {
                self.quiet |= bit16(pad);
            }
            self.pressed[pad].1 = 0;
        }
        self.host
            .input_from(mpfree_play::arp::PADS, 0, mpfree_play::Event::AllOff);
        self.host.engine.panic();
    }

    /// One period of `block` (the transport's, as the clips' arps had it):
    /// each note the arp plays goes where the pads play.
    fn period(
        &mut self,
        block: &mpfree_transport::Block,
        play: &mut impl FnMut(PadsPreview, ArpNote, Option<u8>),
    ) {
        use mpfree_arp::EventKind;
        let (to, rec) = (self.to, self.rec);
        let (on, fed) = (&mut self.on, &mut self.fed);
        self.host.process(block, &mut |offset, kind| match kind {
            EventKind::On { note, velocity } => {
                let Some(to) = to else { return };
                *on |= 1 << (note & 127);
                fed[usize::from(note & 127)] = rec.unwrap_or(NO_REC);
                play(
                    to,
                    ArpNote::On {
                        note,
                        velocity,
                        offset,
                    },
                    rec,
                );
            }
            EventKind::Off { note } => {
                let b = 1u128 << (note & 127);
                if let Some(to) = to.filter(|_| *on & b != 0) {
                    *on &= !b;
                    let went = std::mem::replace(&mut fed[usize::from(note & 127)], NO_REC);
                    play(
                        to,
                        ArpNote::Off { note, offset },
                        (went != NO_REC).then_some(went),
                    );
                }
            }
        });
    }

    /// The notes it has sounding now (for the tests).
    #[cfg(test)]
    fn sounding(&self) -> u32 {
        self.on.count_ones()
    }
}

/// A note of the pads' arp into `rec`, the track recording what the pads
/// play on their own (sampler step 15c's second part,
/// `App::pads_own_rec`): the synth's by its number, into a clip of notes
/// (`TimeHost::routed_note`); a host instrument's or a kit's as its pad
/// (`TimeHost::routed_played`). Nothing without one. Never allocates.
fn rec_pads_arp(to: PadsPreview, n: ArpNote, rec: Option<u8>, time: &mut time::TimeHost) {
    use mpfree_play::Event;
    let Some(t) = rec else {
        return;
    };
    let (offset, ev) = match n {
        ArpNote::On {
            note,
            velocity,
            offset,
        } => (offset, Event::NoteOn { note, velocity }),
        ArpNote::Off { note, offset } => (offset, Event::NoteOff { note }),
    };
    match to {
        PadsPreview::Synth => time.routed_note(t, offset, ev),
        PadsPreview::Host | PadsPreview::Kit(_) => time.routed_played(t, offset, ev),
    }
}

/// No track: what [`PadsArp`] keeps for a note nothing records.
const NO_REC: u8 = u8::MAX;

/// A kit layer as its track records it (sampler step 15f): the kit, the
/// bank of it its hits take, and how the kit takes notes now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LayerKit {
    kit: u8,
    page: u8,
    keys: mpfree_sampler::engine::KitKeys,
}

impl LayerKit {
    /// The note pad `pad` (0 to 15)'s hit records as: the kit's pad of the
    /// bank, or the note that pad plays on a kit played as notes (its
    /// track holds notes, `sampler::KitNotes`).
    fn pad_note(self, pad: usize) -> Option<u8> {
        let kp = usize::from(self.page) * 16 + pad;
        if self.keys.takes_notes() {
            self.keys.note_of_pad(kp)
        } else {
            u8::try_from(kp).ok().filter(|&p| p < 128)
        }
    }

    /// The note a key `note` records as: itself on a kit played as notes,
    /// else the pad that sends it (the MPC note map, any bank); None for a
    /// key the kit's keygroups take on a kit of pads (it plays but isn't
    /// kept, as on the armed track).
    fn key_note(self, note: u8) -> Option<u8> {
        if self.keys.takes_notes() {
            Some(note)
        } else if self.keys.to_keygroups(note) {
            None
        } else {
            Some(mpfree_midi::profile::kit_pad_of_note(note) as u8)
        }
    }
}

/// A keyboard's key `k` on the kit layers (sampler step 15f,
/// [`PadLayers::kit_key`]): the pad it reaches as that pad's kind has it
/// ([`play_routed_pad`]), or the keygroups as a key; the wheel bends their
/// keygroups. `v` shapes a velocity as the pads'. None of it reaches the
/// kit the armed track's keys play already (`armed_kit`). Never allocates.
#[allow(clippy::too_many_arguments)]
fn play_kit_layer_key(
    layers: &mut PadLayers,
    k: midi_in::Key,
    v: impl Fn(u8) -> u8,
    armed_kit: Option<usize>,
    track: u8,
    midi: &mut midi_out::MidiOut,
    (pad_synths, synth): (&mut pad_synths::PadSynths, &mut synth::Synth),
    player: &mut sampler::Player,
) {
    if let midi_in::Key::Bend { value } = k {
        for kit in layers.kit_bend(armed_kit).into_iter().flatten() {
            player.bend_kit(usize::from(kit), value);
        }
        return;
    }
    for (kit, to) in layers.kit_key(k, armed_kit, player).into_iter().flatten() {
        let kit = usize::from(kit);
        play_kit_key(kit, to, k, &v, track, midi, (pad_synths, synth), player);
    }
}

/// Route row `j`'s key `k` on the kit it names (sampler step 15f,
/// [`RouteSynths::kit_key`]), each key at its own velocity as a route to
/// a track plays it; the wheel bends the kit's keygroups. Nothing for a
/// row that names no kit. Never allocates.
#[allow(clippy::too_many_arguments)]
fn play_route_kit_key(
    routes: &mut RouteSynths,
    j: usize,
    k: midi_in::Key,
    track: u8,
    midi: &mut midi_out::MidiOut,
    (pad_synths, synth): (&mut pad_synths::PadSynths, &mut synth::Synth),
    player: &mut sampler::Player,
) {
    if let midi_in::Key::Bend { value } = k {
        if let Some(kit) = routes.kit_bend(j) {
            player.bend_kit(usize::from(kit), value);
        }
        return;
    }
    if let Some((kit, to)) = routes.kit_key(j, k, player) {
        let v = |v: u8| v.max(1);
        play_kit_key(
            usize::from(kit),
            to,
            k,
            &v,
            track,
            midi,
            (pad_synths, synth),
            player,
        );
    }
}

/// Key `k` on kit `kit`, where `sampler::Player::note_to` sent it (`to`):
/// the pad as its kind has it ([`play_routed_pad`]), or the keygroups as
/// a key (they take no pressure yet). `v` gives a key's velocity. Never
/// allocates.
#[allow(clippy::too_many_arguments)]
fn play_kit_key(
    kit: usize,
    to: sampler::NoteTo,
    k: midi_in::Key,
    v: &impl Fn(u8) -> u8,
    track: u8,
    midi: &mut midi_out::MidiOut,
    (pad_synths, synth): (&mut pad_synths::PadSynths, &mut synth::Synth),
    player: &mut sampler::Player,
) {
    use midi_in::Key;
    use mpfree_play::Event;
    match (to, k) {
        (sampler::NoteTo::Pad(p), _) => {
            let note = p.min(127) as u8;
            let event = match k {
                Key::Down { velocity, .. } => Event::NoteOn {
                    note,
                    velocity: v(velocity),
                },
                Key::Up { .. } => Event::NoteOff { note },
                Key::Pressure { value, .. } => Event::Pressure { note, value },
                _ => return,
            };
            play_routed_pad(track, p, event, midi, (pad_synths, synth), player, kit);
        }
        (sampler::NoteTo::Keys(n), Key::Down { velocity, .. }) => {
            player.key_kit_at(kit, n, v(velocity), 0);
        }
        (sampler::NoteTo::Keys(n), Key::Up { .. }) => player.key_off_kit_at(kit, n, 0),
        _ => {}
    }
}

/// The kit layers' pads a hit played or lets go of, (kit, kit pad) each
/// ([`PadLayers::kit_pads_on`], sampler step 15f): `event` of each pad on
/// its kit, as its kind has it ([`play_routed_pad`]). Never allocates.
fn play_kit_layers(
    pads: [Option<(u8, u8)>; PadLayers::MORE],
    event: impl Fn(u8) -> mpfree_play::Event,
    track: u8,
    midi: &mut midi_out::MidiOut,
    (pad_synths, synth): (&mut pad_synths::PadSynths, &mut synth::Synth),
    player: &mut sampler::Player,
) {
    for (kit, kp) in pads.into_iter().flatten() {
        let (pad, kit) = (usize::from(kp), usize::from(kit));
        play_routed_pad(
            track,
            pad,
            event(kp),
            midi,
            (pad_synths, synth),
            player,
            kit,
        );
    }
}

/// Pad `pad`'s bit in a 16-pad mask (none past 16).
fn bit16(pad: usize) -> u16 {
    1u16.checked_shl(pad as u32).unwrap_or(0)
}

/// A note of the pads' arp ([`PadsArp`]) where the pads play on their
/// own: the shared synth by note, a host instrument's pad on the pads'
/// player, a kit's pad as its kind has it ([`play_routed_pad_at`]).
/// Returns the pad on the page it lights, if one does. Never allocates.
#[allow(clippy::too_many_arguments)]
fn play_pads_arp(
    to: PadsPreview,
    n: ArpNote,
    track: u8,
    page: usize,
    synth: &mut synth::Synth,
    own: &mut instruments::Instruments,
    midi: &mut midi_out::MidiOut,
    pad_synths: &mut pad_synths::PadSynths,
    player: &mut sampler::Player,
) -> Option<usize> {
    use mpfree_play::Event;
    let pads = instruments::THE_PADS;
    match (to, n) {
        (
            PadsPreview::Synth,
            ArpNote::On {
                note,
                velocity,
                offset,
            },
        ) => {
            synth.note_on_note(note, velocity, offset);
            (0..16).find(|&p| synth.notes(p).contains(&note))
        }
        (PadsPreview::Synth, ArpNote::Off { note, offset }) => {
            synth.note_off_note(note, offset);
            None
        }
        (
            PadsPreview::Host,
            ArpNote::On {
                note,
                velocity,
                offset,
            },
        ) => {
            let none = Shift::NONE;
            own.pad_on_shifted(pads, usize::from(note), velocity, offset, &none);
            (usize::from(note) < 16).then_some(usize::from(note))
        }
        (PadsPreview::Host, ArpNote::Off { note, offset }) => {
            own.pad_off(pads, usize::from(note), offset);
            None
        }
        (
            PadsPreview::Kit(k),
            ArpNote::On {
                note,
                velocity,
                offset,
            },
        ) => {
            let on = Event::NoteOn { note, velocity };
            let s = (&mut *pad_synths, &mut *synth);
            play_routed_pad_at(
                track,
                usize::from(note),
                on,
                offset,
                midi,
                s,
                player,
                usize::from(k),
            );
            let p = usize::from(note);
            (p / 16 == page).then_some(p % 16)
        }
        (PadsPreview::Kit(k), ArpNote::Off { note, offset }) => {
            let off = Event::NoteOff { note };
            let s = (&mut *pad_synths, &mut *synth);
            play_routed_pad_at(
                track,
                usize::from(note),
                off,
                offset,
                midi,
                s,
                player,
                usize::from(k),
            );
            None
        }
    }
}

/// The pads' hits while they play an instrument on their own (a synth, a
/// whole kit, a bank's own: [`PadsPreview`]), into the track REC set up
/// for it (sampler step 15c's second part, "REC records what is heard":
/// `App::pads_own_rec`), as the layers' hits go into theirs
/// ([`PadLayers::rec_on`]): each hit as the pad that plays it there (a
/// kit's pad of the page the pads are on, the synth's or a host
/// instrument's pad), its let-go and pressure where it went though the
/// pads moved on meanwhile; and each key that plays the pads' synth on its
/// own, as the pad its note plays there (CAPTURE's rule: a note no pad
/// plays alone isn't kept). The pads' arp's notes go there by
/// [`PadsArp`]. Main.rs and the test session both use it. Allocates
/// nothing once made.
struct PadsRec {
    /// The track recording what the pads play now.
    rec: Option<u8>,
    /// Where each pad's hit went: the track, and the pad it went in as.
    fed: [Option<(u8, u8)>; 16],
    /// The pad each key went to on the track it went into.
    keys: midi_in::RoutedPads,
    /// The tracks a key went into, a bit each, for its let-go.
    key_tracks: u128,
}

impl PadsRec {
    fn new() -> PadsRec {
        PadsRec {
            rec: None,
            fed: [None; 16],
            keys: midi_in::RoutedPads::new(),
            key_tracks: 0,
        }
    }

    /// The track recording what the pads play on their own now
    /// (`App::pads_own_rec`): the hits and keys that follow go into it.
    fn rec_to(&mut self, rec: Option<u8>) {
        self.rec = rec;
    }

    /// Pad `pad` (0 to 15) down as pad `note` of what the pads play, at
    /// `velocity` as the recorder takes it (`App::layer_rec_velocity`;
    /// None: nothing records the pads now).
    fn on(&mut self, pad: usize, note: u8, velocity: Option<u8>, time: &mut time::TimeHost) {
        use mpfree_play::Event;
        self.off(pad, time);
        let (Some(t), Some(velocity), Some(fed)) = (self.rec, velocity, self.fed.get_mut(pad))
        else {
            return;
        };
        *fed = Some((t, note));
        time.routed_played(t, 0, Event::NoteOn { note, velocity });
    }

    /// Pad `pad` up: its end where its hit went.
    fn off(&mut self, pad: usize, time: &mut time::TimeHost) {
        if let Some((t, note)) = self.fed.get_mut(pad).and_then(Option::take) {
            time.routed_played(t, 0, mpfree_play::Event::NoteOff { note });
        }
    }

    /// Pad `pad`'s pressure where its hit went.
    fn pressure(&mut self, pad: usize, value: u8, time: &mut time::TimeHost) {
        if let Some(&Some((t, note))) = self.fed.get(pad) {
            time.routed_played(t, 0, mpfree_play::Event::Pressure { note, value });
        }
    }

    /// A keyboard's key that played the pads' synth on its own
    /// (`KeysOnPads`), into the track recording it, as the pad its note
    /// plays there (`midi_file::pad_on_track`); its end and pressure where
    /// it went. `velocity` is as the recorder takes it (None: nothing
    /// records the pads now).
    fn key(
        &mut self,
        k: midi_in::Key,
        velocity: Option<u8>,
        time: &mut time::TimeHost,
        own: &instruments::Instruments,
        synth: &synth::Synth,
    ) {
        use midi_in::Key;
        match k {
            Key::Down { note, .. } => {
                let (Some(t), Some(velocity)) = (self.rec, velocity) else {
                    return;
                };
                let key = Key::Down { note, velocity };
                let pad = |n| midi_file::pad_on_track(own, Some(synth), usize::from(t), n);
                if let Some(ev) = self.keys.event(t, key, pad) {
                    time.routed_played(t, 0, ev);
                    self.key_tracks |= 1 << t;
                }
            }
            Key::Up { .. } | Key::Pressure { .. } => {
                let mut went = self.key_tracks;
                while went != 0 {
                    let t = went.trailing_zeros() as u8;
                    went &= went - 1;
                    if let Some(ev) = self.keys.event(t, k, |_| None) {
                        time.routed_played(t, 0, ev);
                    }
                }
            }
            _ => {}
        }
    }
}

/// The pads' layers beyond the first (the instrument redesign's step 11a,
/// app/pad_layers.rs): each pad hit plays them too, whichever way the
/// first plays (on its own, or through the armed track), each by its own
/// pad map, before the arp and the recorder; and so does each key of a
/// keyboard. The instrument host's layer players (`instruments::layer`)
/// play its synths, pointed each period ([`PadLayers::follow`]); the
/// shared synth plays here. A hit or key remembers whether it started on
/// the shared synth, so its let-go ends it there though the layers
/// changed meanwhile; the host's players remember theirs. The pedals a
/// keyboard put down on the layers come up as they move on, so nothing
/// hangs. While a track armed besides the armed one plays a layer's synth
/// (step 11c: REC sets them up, `desk.rs`), each hit and key goes into
/// that track's recorder too, as a routed keyboard's does
/// (`TimeHost::routed_played`), and its let-go goes where it went. Main.rs
/// and the test session both use it. Allocates nothing once made.
struct PadLayers {
    /// The shared synth is a layer now.
    synth: bool,
    /// The pads whose hit started on the shared synth as a layer, a bit
    /// each.
    synth_pads: u16,
    /// The keys held on the shared synth as a layer, a bit each.
    synth_keys: u128,
    /// The pedals the keys put down on the layers.
    pedals: midi_in::PedalsDown,
    /// The track recording each layer now (`App::pads_layer_recs`), with
    /// a kit layer's kit (its track records that bank's pad, or the note
    /// it plays, sampler step 15f).
    rec: [Option<(u8, Option<LayerKit>)>; PadLayers::MORE],
    /// The track each pad's hit went into for each layer, and the note it
    /// went in as, for its let-go and pressure.
    fed: [[Option<(u8, u8)>; PadLayers::MORE]; 16],
    /// The kit each kit layer plays and its bank (`App::pads_layer_kits`,
    /// sampler step 15f).
    kits: [Option<(u8, u8)>; PadLayers::MORE],
    /// The kit pad each pad's hit played on each kit layer, (kit, kit
    /// pad), for its let-go and pressure though the layers changed since.
    kit_held: [[Option<(u8, u8)>; PadLayers::MORE]; 16],
    /// Where each key went on each kit layer, (kit, where), for its
    /// let-go and pressure though the layers changed since.
    kit_keys: [[Option<(u8, sampler::NoteTo)>; PadLayers::MORE]; 128],
    /// The kit layers the wheel bent, to bend back once they are layers
    /// no more.
    bent: [Option<u8>; PadLayers::MORE],
    /// The pad each key went to on each track it went into, as a route's
    /// (CAPTURE's rule: a note no pad plays alone isn't kept).
    keys: midi_in::RoutedPads,
    /// The tracks a key went into, a bit each, for its let-go.
    key_tracks: u128,
}

impl PadLayers {
    /// The layers beyond the first.
    const MORE: usize = instruments::PAD_LAYERS - 1;

    fn new() -> PadLayers {
        PadLayers {
            synth: false,
            synth_pads: 0,
            synth_keys: 0,
            pedals: midi_in::PedalsDown::default(),
            rec: [None; PadLayers::MORE],
            fed: [[None; PadLayers::MORE]; 16],
            kits: [None; PadLayers::MORE],
            kit_held: [[None; PadLayers::MORE]; 16],
            kit_keys: [[None; PadLayers::MORE]; 128],
            bent: [None; PadLayers::MORE],
            keys: midi_in::RoutedPads::new(),
            key_tracks: 0,
        }
    }

    /// The tracks recording the layers now (`App::pads_layer_recs`), a
    /// kit layer's with how its kit takes notes now (`keys`,
    /// `sampler::Player::kit_keys`): the hits and keys that follow go
    /// into them. A hit or key held goes on where it went.
    fn rec_to(
        &mut self,
        recs: [Option<app::LayerRec>; PadLayers::MORE],
        keys: impl Fn(usize) -> mpfree_sampler::engine::KitKeys,
    ) {
        self.rec = recs.map(|r| {
            r.map(|(t, kit)| {
                let kit = kit.map(|(kit, page)| LayerKit {
                    kit,
                    page,
                    keys: keys(usize::from(kit)),
                });
                (t, kit)
            })
        });
    }

    /// The kit layers as the app has them now (`App::pads_layer_kits`):
    /// each hit plays its pad of their bank on them too, and each key as
    /// the kit takes it. A hit or key held goes on where it went. Returns
    /// the kits the wheel bent that are layers no more, to bend back.
    fn kits_to(
        &mut self,
        kits: [Option<(u8, u8)>; PadLayers::MORE],
    ) -> [Option<u8>; PadLayers::MORE] {
        self.kits = kits;
        let now = kits.map(|l| l.map(|(kit, _)| kit));
        let gone = self.bent.map(|b| b.filter(|k| !now.contains(&Some(*k))));
        self.bent = self.bent.map(|b| b.filter(|k| now.contains(&Some(*k))));
        gone
    }

    /// A keyboard's key `k` on the kit layers (sampler step 15f), (kit,
    /// where it goes) each, as each kit's Keys play sends it
    /// (`sampler::Player::note_to`: the pad that sends the note, or the
    /// keygroups that hold it). A key going down is kept for its let-go and
    /// pressure, which go where it went though the layers changed since. A
    /// kit layer that is the kit the armed track's keys play already
    /// (`armed_kit`) takes none, so one key never plays a kit twice; nor
    /// does a second note-on for a key held there. Never allocates.
    fn kit_key(
        &mut self,
        k: midi_in::Key,
        armed_kit: Option<usize>,
        player: &sampler::Player,
    ) -> [Option<(u8, sampler::NoteTo)>; PadLayers::MORE] {
        use midi_in::Key;
        let none = [None; PadLayers::MORE];
        let at = |note: u8| usize::from(note & 0x7F);
        match k {
            Key::Down { note, .. } => {
                let kits = self.kits;
                let held = &mut self.kit_keys[at(note)];
                if held.iter().any(Option::is_some) {
                    return none;
                }
                *held = kits.map(|l| {
                    let (kit, _) = l?;
                    let k = usize::from(kit);
                    (Some(k) != armed_kit).then(|| (kit, player.note_to(k, note)))
                });
                *held
            }
            Key::Up { note } => std::mem::take(&mut self.kit_keys[at(note)]),
            Key::Pressure { note, .. } => self.kit_keys[at(note)],
            _ => none,
        }
    }

    /// The kit layers the wheel bends now (their keygroups,
    /// `sampler::Player::bend_kit`): every one but the kit the armed
    /// track's keys play already (`armed_kit`), kept so a kit that stops
    /// being a layer is bent back ([`PadLayers::kits_to`]).
    fn kit_bend(&mut self, armed_kit: Option<usize>) -> [Option<u8>; PadLayers::MORE] {
        let bent = self.kits.map(|l| {
            l.map(|(kit, _)| kit)
                .filter(|&k| Some(usize::from(k)) != armed_kit)
        });
        self.bent = bent;
        bent
    }

    /// The keys held on the kit layers now (for the tests).
    #[cfg(test)]
    fn kit_keys_held(&self) -> usize {
        self.kit_keys
            .iter()
            .filter(|k| k.iter().any(Option::is_some))
            .count()
    }

    /// Pad `pad` (0 to 15) down: the kit layers' pads it plays, (kit, kit
    /// pad) each, kept for its let-go and pressure.
    fn kit_pads_on(&mut self, pad: usize) -> [Option<(u8, u8)>; PadLayers::MORE] {
        let kits = self.kits;
        let Some(held) = self.kit_held.get_mut(pad) else {
            return [None; PadLayers::MORE];
        };
        *held = kits.map(|k| k.map(|(kit, page)| (kit, page * 16 + pad as u8)));
        *held
    }

    /// Pad `pad` up: the kit layers' pads its hit played, now forgotten.
    fn kit_pads_off(&mut self, pad: usize) -> [Option<(u8, u8)>; PadLayers::MORE] {
        self.kit_held
            .get_mut(pad)
            .map_or([None; PadLayers::MORE], std::mem::take)
    }

    /// The kit layers' pads pad `pad`'s hit plays, for its pressure.
    fn kit_pads_held(&self, pad: usize) -> [Option<(u8, u8)>; PadLayers::MORE] {
        self.kit_held
            .get(pad)
            .copied()
            .unwrap_or([None; PadLayers::MORE])
    }

    /// Pad `pad` (0 to 15) down: into each track recording a layer, at
    /// `velocity` as the recorder takes it (`App::layer_rec_velocity`;
    /// None: nothing records the pads now).
    fn rec_on(&mut self, pad: usize, velocity: Option<u8>, time: &mut time::TimeHost) {
        let (Some(velocity), Some(fed)) = (velocity, self.fed.get_mut(pad)) else {
            return;
        };
        *fed = self.rec.map(|r| {
            let (t, kit) = r?;
            let note = kit.map_or(Some(pad as u8), |k| k.pad_note(pad))?;
            Some((t, note))
        });
        for &(t, note) in fed.iter().flatten() {
            time.routed_played(t, 0, mpfree_play::Event::NoteOn { note, velocity });
        }
    }

    /// Pad `pad` up: its end where its hit went.
    fn rec_off(&mut self, pad: usize, time: &mut time::TimeHost) {
        let Some(fed) = self.fed.get_mut(pad) else {
            return;
        };
        for (t, note) in std::mem::take(fed).into_iter().flatten() {
            time.routed_played(t, 0, mpfree_play::Event::NoteOff { note });
        }
    }

    /// Pad `pad`'s pressure where its hit went.
    fn rec_pressure(&mut self, pad: usize, value: u8, time: &mut time::TimeHost) {
        let Some(fed) = self.fed.get(pad) else {
            return;
        };
        for &(t, note) in fed.iter().flatten() {
            time.routed_played(t, 0, mpfree_play::Event::Pressure { note, value });
        }
    }

    /// A keyboard's key into each track recording a layer, as the pad its
    /// note plays there (`midi_file::pad_on_track`: the track plays its
    /// layer's synth, its own pad map's or the shared synth's), or as a
    /// kit layer's kit takes it ([`LayerKit::key_note`], sampler step
    /// 15f); its end and pressure where it went. `velocity` is as the recorder takes it
    /// (None: nothing records the pads now).
    fn rec_key(
        &mut self,
        k: midi_in::Key,
        velocity: Option<u8>,
        time: &mut time::TimeHost,
        own: &instruments::Instruments,
        synth: &synth::Synth,
    ) {
        use midi_in::Key;
        match k {
            Key::Down { note, .. } => {
                let Some(velocity) = velocity else {
                    return;
                };
                for &(t, kit) in self.rec.iter().flatten() {
                    let key = Key::Down { note, velocity };
                    let pad = |n| match kit {
                        Some(k) => k.key_note(n),
                        None => midi_file::pad_on_track(own, Some(synth), usize::from(t), n),
                    };
                    if let Some(ev) = self.keys.event(t, key, pad) {
                        time.routed_played(t, 0, ev);
                        self.key_tracks |= 1 << t;
                    }
                }
            }
            Key::Up { .. } | Key::Pressure { .. } => {
                let mut went = self.key_tracks;
                while went != 0 {
                    let t = went.trailing_zeros() as u8;
                    went &= went - 1;
                    if let Some(ev) = self.keys.event(t, k, |_| None) {
                        time.routed_played(t, 0, ev);
                    }
                }
            }
            _ => {}
        }
    }

    /// The layers as the app has them now (`App::pads_layer_holders`,
    /// `App::pads_layer_synth`): the host's layer players pointed at the
    /// holders it holds, the shared synth's layer on or off. The notes
    /// and pedals left on a layer that goes end there first. Nothing
    /// changes (and nothing allocates) when nothing did.
    fn follow(
        &mut self,
        holders: [Option<usize>; PadLayers::MORE],
        synth_on: bool,
        synth: &mut synth::Synth,
        own: &mut instruments::Instruments,
    ) {
        let at = holders.map(|h| h.filter(|&h| own.holds(h)));
        let moved = (0..PadLayers::MORE).any(|i| own.layer_at(i) != at[i]);
        if !moved && synth_on == self.synth {
            return;
        }
        for ctl in self.pedals.lift() {
            for i in 0..PadLayers::MORE {
                own.control(instruments::layer(i), ctl, 0.0, 0);
            }
            if self.synth {
                synth.control(ctl, 0.0, 0);
            }
        }
        for (i, &to) in at.iter().enumerate() {
            if own.layer_at(i) != to {
                own.set_layer(i, to);
            }
        }
        if self.synth && !synth_on {
            for pad in (0..16).filter(|&p| self.synth_pads >> p & 1 == 1) {
                synth.note_off(pad, 0);
            }
            for note in (0..128u8).filter(|&n| self.synth_keys >> n & 1 == 1) {
                synth.note_off_note(note, 0);
            }
            self.synth_pads = 0;
            self.synth_keys = 0;
        }
        self.synth = synth_on;
    }

    /// Pad `pad` (0 to 15) down at velocity `v` (shaped): every layer
    /// plays it.
    fn pad_on(
        &mut self,
        pad: usize,
        v: u8,
        synth: &mut synth::Synth,
        own: &mut instruments::Instruments,
    ) {
        let none = mpfree_arp::scale::Shift::NONE;
        for i in 0..PadLayers::MORE {
            own.pad_on_shifted(instruments::layer(i), pad, v, 0, &none);
        }
        if self.synth && pad < 16 {
            synth.note_on(pad, v, 0);
            self.synth_pads |= 1 << pad;
        }
    }

    /// Pad `pad` up: the notes its hit started on the layers end there.
    /// Nothing for a hit that started none.
    fn pad_off(
        &mut self,
        pad: usize,
        synth: &mut synth::Synth,
        own: &mut instruments::Instruments,
    ) {
        for i in 0..PadLayers::MORE {
            own.pad_off(instruments::layer(i), pad, 0);
        }
        if pad < 16 && self.synth_pads >> pad & 1 == 1 {
            synth.note_off(pad, 0);
            self.synth_pads &= !(1 << pad);
        }
    }

    /// Pad `pad`'s pressure on the notes its hit started on the layers.
    fn pad_pressure(
        &mut self,
        pad: usize,
        value: u8,
        synth: &mut synth::Synth,
        own: &mut instruments::Instruments,
    ) {
        for i in 0..PadLayers::MORE {
            own.pad_pressure(instruments::layer(i), pad, value, 0);
        }
        if pad < 16 && self.synth_pads >> pad & 1 == 1 {
            synth.pressure(pad, value);
        }
    }

    /// A keyboard's key `k` on the layers too, each key its own note (`v`
    /// shapes a velocity as the pads'). A second note-on for a key held
    /// there is ignored; a let-go, pressure or slide reaches where the key
    /// went.
    fn key(
        &mut self,
        k: midi_in::Key,
        v: impl Fn(u8) -> u8,
        synth: &mut synth::Synth,
        own: &mut instruments::Instruments,
    ) {
        use midi_in::Key;
        let bit = |note: u8| 1u128 << (note & 0x7F);
        match k {
            Key::Down { note, velocity } => {
                let vel = v(velocity);
                for i in 0..PadLayers::MORE {
                    if !own.key_held(instruments::layer(i), note) {
                        own.note_on(instruments::layer(i), note, vel, 0);
                    }
                }
                if self.synth && self.synth_keys & bit(note) == 0 {
                    synth.note_on_note(note, vel, 0);
                    self.synth_keys |= bit(note);
                }
            }
            Key::Up { note } => {
                for i in 0..PadLayers::MORE {
                    if own.key_held(instruments::layer(i), note) {
                        own.note_off(instruments::layer(i), note, 0);
                    }
                }
                if self.synth_keys & bit(note) != 0 {
                    synth.note_off_note(note, 0);
                    self.synth_keys &= !bit(note);
                }
            }
            Key::Pressure { note, value } => {
                for i in 0..PadLayers::MORE {
                    if own.key_held(instruments::layer(i), note) {
                        own.note_pressure(instruments::layer(i), note, value);
                    }
                }
                if self.synth_keys & bit(note) != 0 {
                    synth.pressure_note(note, value);
                }
            }
            Key::Expr { note, kind, value } => {
                for i in 0..PadLayers::MORE {
                    if own.key_held(instruments::layer(i), note) {
                        own.note_expression(instruments::layer(i), note, kind, value);
                    }
                }
                if self.synth_keys & bit(note) != 0 {
                    synth.expression_note(note, kind, value);
                }
            }
            Key::Bend { value } => {
                for i in 0..PadLayers::MORE {
                    own.bend(instruments::layer(i), value);
                }
                if self.synth {
                    synth.bend(value);
                }
            }
            Key::Control { ctl, value } => {
                for i in 0..PadLayers::MORE {
                    own.control(instruments::layer(i), ctl, value, 0);
                }
                if self.synth {
                    synth.control(ctl, value, 0);
                }
                self.pedals.key(ctl, value);
            }
        }
    }

    /// Whether a hit or key plays the shared synth as a layer now (for
    /// the tests).
    #[cfg(test)]
    fn holds_synth(&self) -> bool {
        self.synth_pads != 0 || self.synth_keys != 0
    }
}

/// A holder's name for the screen: "Track 3", or "The pads' synth" for a
/// free place's (step 6).
fn holder_says(holder: usize) -> String {
    if holder < instruments::TRACKS {
        format!("Track {}", holder + 1)
    } else {
        "The pads' synth".to_string()
    }
}

/// A project's sends (step 4), once its instruments are in: each track
/// plays the other track's instrument it saved, or its kit, said. The
/// lines for the log. Main.rs and the test session both run it.
fn open_sends(own: &mut instruments::Instruments, sends: &[(usize, usize)]) -> Vec<String> {
    let mut lines = Vec::new();
    for &(track, to) in sends {
        let why = match own.set_send(track, Some(to)) {
            Ok(()) => continue,
            Err(instruments::SendRefused::HasOwn) => "it has its own",
            Err(instruments::SendRefused::NothingThere) => "that track has no instrument",
            Err(_) => "no such track",
        };
        lines.push(format!(
            "project: track {} plays its kit, not track {}'s instrument ({why})",
            track + 1,
            to + 1
        ));
    }
    if !sends.is_empty() {
        let n = (0..instruments::TRACKS)
            .filter(|&t| own.send(t).is_some())
            .count();
        lines.push(format!(
            "project: {n} tracks play another track's instrument"
        ));
    }
    lines
}

/// Each track's lasting id by place (0 for none), for knob owners
/// (`locks.rs`): the lowest owns a knob two tracks lock. Never allocates.
fn track_ranks(project: &mpfree_seq::Project) -> [u32; locks::TRACKS] {
    let mut ids = [0; locks::TRACKS];
    for (id, t) in ids.iter_mut().zip(&project.tracks) {
        *id = t.id.0;
    }
    ids
}

/// Each track's send now ([`instruments::Instruments::send`]), for the
/// app and the screen. Never allocates.
fn sends_now(own: &instruments::Instruments) -> gui::TrackSends {
    std::array::from_fn(|t| own.send(t).map(|o| o as u8))
}

/// What the screen says when track `owner`'s instrument went and the
/// tracks that played it (`senders`, a bit each) play their kits again.
/// None when none did.
fn sends_ended(senders: u128, owner: usize) -> Option<String> {
    if senders == 0 {
        return None;
    }
    let names: Vec<String> = (0..instruments::TRACKS)
        .filter(|&t| senders >> t & 1 == 1)
        .map(|t| (t + 1).to_string())
        .collect();
    let (s, verb) = if names.len() == 1 {
        ("", "plays its kit")
    } else {
        ("s", "play their kits")
    };
    let gone = if owner < instruments::TRACKS {
        format!("Track {}'s instrument", owner + 1)
    } else {
        "the pads' synth".to_string()
    };
    Some(format!(
        "Track{s} {} {verb} again: {gone} is gone",
        names.join(", ")
    ))
}

/// Track `track` plays track `to`'s instrument (the Instrument row, step
/// 4): its own instrument is taken out first, and the tracks that played
/// that go back to their kits. The lines for the log, and what the screen
/// says when it can't (or what went with it). Main.rs and the test
/// session both run it.
fn send_track(
    own: &mut instruments::Instruments,
    track: usize,
    to: usize,
) -> (Vec<String>, Option<String>) {
    let mut lines = Vec::new();
    let mut tell = None;
    let senders = own.senders(track);
    if own.holds(track) && !own.take_out(track) {
        let s = format!(
            "Track {}: its last sound is still ringing out; try again in a moment",
            track + 1
        );
        lines.push(s.clone());
        return (lines, Some(s));
    }
    if let Some(s) = sends_ended(senders, track) {
        lines.push(s.clone());
        tell = Some(s);
    }
    match own.set_send(track, Some(to)) {
        Ok(()) => {
            let name = own.held(to).map_or("instrument", |h| h.name);
            lines.push(format!(
                "track {}: plays {}",
                track + 1,
                app::sent_name(to, name)
            ));
        }
        Err(why) => {
            let s = match why {
                instruments::SendRefused::NothingThere => format!(
                    "Track {} plays its kit: Track {} has no instrument of its own to play",
                    track + 1,
                    to + 1
                ),
                _ => format!("Track {} plays its kit", track + 1),
            };
            lines.push(s.clone());
            tell = Some(s);
        }
    }
    (lines, tell)
}

/// Track `from`'s own instrument's sound now, to copy (step 4c,
/// Duplicate): its node id, and its extras and values as a preset keeps
/// them. None when it has none. Allocates (an effect's work, as a preset
/// try's). Main.rs and the test session both run it.
fn sound_of_track(
    own: &instruments::Instruments,
    from: usize,
) -> Option<(&'static str, synth::SoundCopy)> {
    let held = own.held(from)?;
    let node = own.node(from)?;
    let values = node.params().iter().map(|p| node.param(p.id)).collect();
    Some((held.id, (node.save_extras(), values)))
}

/// What the screen says once track `track` has a copy of track `from`'s
/// instrument `name` (step 4c).
fn copied_note(track: usize, from: usize, name: &str) -> String {
    format!(
        "Track {} plays its own copy of {}: change one and the other stays",
        track + 1,
        app::sent_name(from, name)
    )
}

/// What pad `pad` plays on MIDI Out, CV and an export: the synth's pad
/// layout's notes while the synth has the pads, else the kit's pad (the
/// note its port's device profile gives it, or its own,
/// [`midi_out::kit_notes`]).
fn pad_is(synth: Option<&synth::Synth>, pad: usize) -> midi_out::NoteIs<'_> {
    match synth {
        Some(s) => midi_out::NoteIs::Pad(s.notes(pad)),
        None => midi_out::NoteIs::Kit,
    }
}

/// What the Synth tab shows: the selected track's own synth when it has
/// one (instruments.rs), or the one it plays of another track's, named by
/// the preset it plays (`presets`), else the shared synth. A plugin synth lists its own presets once they're listed
/// ([`mixing::MixHost::plugin_presets`]).
fn synth_tab(
    synth: &synth::Synth,
    mix: &mixing::MixHost,
    track: usize,
    presets: &presets::TrackPresets,
) -> SynthState {
    let own = &mix.instruments;
    let track = instrument_track(own, track);
    match (own.held(track), own.node(track)) {
        (Some(h), Some(node)) => {
            let engine = synth::Engine::from_node_id(h.id);
            // A free place's (step 6): the pads play it, and no track has
            // it as its own.
            let label = if track < instruments::TRACKS {
                format!("Track {}", track + 1)
            } else {
                "Pads, no track".to_string()
            };
            let mut st = synth.state_of(node, engine, &label, presets.index(track));
            st.own = presets.own(track).map(str::to_string);
            if engine.is_none() {
                st.presets = mix
                    .plugin_presets(h.id)
                    .unwrap_or_default()
                    .iter()
                    .map(plugin_preset_name)
                    .collect();
            }
            st
        }
        _ => synth.state(),
    }
}

/// A plugin preset's line in the Synth tab's list: "Bass / Attacky", its
/// folder or the plugin's category first, so the list reads in groups.
fn plugin_preset_name(p: &mpfree_plughost::Preset) -> String {
    if p.category.is_empty() {
        p.name.clone()
    } else {
        format!("{} / {}", p.category, p.name)
    }
}

/// Asks for the presets of every plugin synth a track has (listed once,
/// on the builder thread), so they're there when the Synth tab opens.
fn ask_plugin_presets(mix: &mut mixing::MixHost) {
    for t in 0..instruments::HOLDERS {
        if let Some(id) = mix.instruments.held(t).map(|h| h.id) {
            mix.ask_presets(id);
        }
    }
}

/// What the audio loop last handed each instrument for its tuning
/// (tunings step T5, docs/dev/design/tunings.md). Every period
/// [`TuningsHanded::follow`] works out what each one plays in (its own
/// tuning or the song's, `App::tunings`) and hands the tuning to those
/// whose pick changed: the tracks' own instruments and the free places'
/// (by holder), every kit place's keygroups, and each synth pad's copy
/// (its kit's). The shared synth plays its pick through the pads'
/// settings (`App::shared_tuning_follows`). A new instrument or copy put
/// in is handed its pick on the next period. Plain data: it never
/// allocates but for the log line when the song's tuning changes.
struct TuningsHanded {
    /// Each holder's instrument's id and pick, as last handed.
    holders:
        [Option<(Option<instrument_list::InstrumentId>, synth::TuningPick)>; instruments::HOLDERS],
    /// Each kit place's pick, as last handed.
    kits: [Option<synth::TuningPick>; instrument_list::KITS],
    /// Each synth pad slot's kit and pick, as last handed.
    pads: [Option<(usize, synth::TuningPick)>; pad_synths::HOST_SLOTS],
    /// The instruments and synth pad copies put in, as last seen.
    puts: (u64, u64),
    /// The song's tuning, as last seen.
    song: Option<synth::TuningPick>,
    /// Each holder whose instrument can't follow its tuning, as the log
    /// last said: its instrument's id, its pick and its node's id.
    twelve: [Option<(
        Option<instrument_list::InstrumentId>,
        synth::TuningPick,
        &'static str,
    )>; instruments::HOLDERS],
    /// The tracks' and free places' instruments that can't follow a
    /// tuning, a bit each ([`TuningsHanded::deaf`]).
    deaf: (u128, u16),
}

impl TuningsHanded {
    fn new() -> TuningsHanded {
        TuningsHanded {
            holders: [None; instruments::HOLDERS],
            kits: [None; instrument_list::KITS],
            pads: [None; pad_synths::HOST_SLOTS],
            puts: (u64::MAX, u64::MAX),
            song: None,
            twelve: [None; instruments::HOLDERS],
            deaf: (0, 0),
        }
    }

    /// The tracks' own instruments and the free places' that can't follow
    /// a tuning (a plugin that takes only MIDI notes), a bit each, as of
    /// the last [`TuningsHanded::follow`]: the tuning screens say so
    /// (tunings step T6, `App::set_tuning_facts`).
    fn deaf(&self) -> (u128, u16) {
        self.deaf
    }

    /// Holder `h`'s bit in [`TuningsHanded::deaf`], set or cleared.
    fn set_deaf(&mut self, h: usize, deaf: bool) {
        if h < instruments::TRACKS {
            let bit = 1u128 << h;
            self.deaf.0 = if deaf {
                self.deaf.0 | bit
            } else {
                self.deaf.0 & !bit
            };
        } else if let Some(bit) = 1u16.checked_shl((h - instruments::TRACKS) as u32) {
            self.deaf.1 = if deaf {
                self.deaf.1 | bit
            } else {
                self.deaf.1 & !bit
            };
        }
    }

    /// Hands each instrument whose pick changed its tuning; with `sampler`
    /// None (a session with no kits), the kits wait. Returns the lines for
    /// the log: when the song's tuning changed (none for 12-TET at the
    /// start), "sampler: the kits' keygroups play Just (5-limit) on D (pad
    /// hits and slices as tuned on the pad)"; once for an instrument that
    /// can't follow its tuning (tunings step T6), "tunings: Track 3's own
    /// instrument, Surge XT, takes only MIDI notes, so it plays 12-TET, not
    /// Just (5-limit) on D". With nothing to say it allocates nothing.
    fn follow(
        &mut self,
        tunings: &tunings::Tunings,
        listed: &instrument_list::InstrumentList,
        synth: &synth::Synth,
        mix: &mut mixing::MixHost,
        sampler: Option<&mut sampler::Player>,
    ) -> Vec<String> {
        let mut said = Vec::new();
        let files = synth.tuning_files();
        // One tuning made for each pick handed this period.
        let mut made: Option<(synth::TuningPick, mpfree_tuning::Tuning)> = None;
        let mut tuning = |p: synth::TuningPick| -> mpfree_tuning::Tuning {
            match &made {
                Some((q, t)) if *q == p => t.clone(),
                _ => {
                    let t = p.tuning(files);
                    made = Some((p, t.clone()));
                    t
                }
            }
        };
        let puts = (mix.instruments.puts(), mix.pad_synths.puts());
        if puts.0 != self.puts.0 {
            self.holders = [None; instruments::HOLDERS];
        }
        if puts.1 != self.puts.1 {
            self.pads = [None; pad_synths::HOST_SLOTS];
        }
        self.puts = puts;
        let (tracks, free) = (mix.instruments.held_tracks(), mix.instruments.held_free());
        for h in 0..instruments::HOLDERS {
            let held = if h < instruments::TRACKS {
                tracks >> h & 1 == 1
            } else {
                free >> (h - instruments::TRACKS) & 1 == 1
            };
            if !held {
                self.holders[h] = None;
                self.twelve[h] = None;
                self.set_deaf(h, false);
                continue;
            }
            let id = listed.of_holder(h);
            let pick = tunings.pick(id);
            if self.holders[h] != Some((id, pick)) {
                let t = tuning(pick);
                mix.instruments.set_tuning(h, &t);
                self.holders[h] = Some((id, pick));
                self.set_deaf(h, !mix.instruments.follows_tuning(h));
                // One that plays 12-TET whatever it's handed says so once.
                let node = mix.instruments.held(h).map_or("", |i| i.id);
                if t.is_standard() || mix.instruments.follows_tuning(h) {
                    self.twelve[h] = None;
                } else if self.twelve[h] != Some((id, pick, node)) {
                    self.twelve[h] = Some((id, pick, node));
                    let who = if h < instruments::TRACKS {
                        instrument_list::Kind::Own { track: h }
                    } else {
                        instrument_list::Kind::Free {
                            place: h - instruments::TRACKS,
                        }
                    };
                    let name = mix.instruments.held(h).map_or("", |i| i.name);
                    said.push(format!(
                        "tunings: {}, {name}, takes only MIDI notes, so it plays 12-TET, not {}",
                        who.words(),
                        pick.label(files)
                    ));
                }
            }
        }
        for s in 0..pad_synths::HOST_SLOTS {
            let Some(kit) = mix.pad_synths.kit_of(s) else {
                self.pads[s] = None;
                continue;
            };
            let pick = tunings.pick(listed.of_kit(kit));
            if self.pads[s] != Some((kit, pick)) {
                mix.pad_synths.set_tuning(s, &tuning(pick));
                self.pads[s] = Some((kit, pick));
            }
        }
        let mut own = 0;
        if let Some(player) = sampler {
            for k in 0..player.kits().min(instrument_list::KITS) {
                let id = listed.of_kit(k);
                let pick = tunings.pick(id);
                own += usize::from(id.is_some_and(|id| tunings.own(id).is_some()));
                if self.kits[k] != Some(pick) {
                    player.set_kit_tuning(k, &tuning(pick));
                    self.kits[k] = Some(pick);
                }
            }
        }
        let song = tunings.song();
        if self.song == Some(song) {
            return said;
        }
        let was = self.song.replace(song);
        if was.is_some() || song != synth::TuningPick::default() {
            let mut line = format!(
                "sampler: the kits' keygroups play {} (pad hits and slices as tuned on the pad)",
                song.label(files)
            );
            if own > 0 {
                line.push_str(&format!("; {own} with a tuning of their own keep it"));
            }
            said.insert(0, line);
        }
        said
    }

    /// A new sampler player (the test session's: main.rs keeps one):
    /// its kits are handed their tunings again.
    #[cfg(test)]
    fn new_player(&mut self) {
        self.kits = [None; instrument_list::KITS];
    }
}

/// The tracks' own synths follow the Synth tab's pad layout and octave
/// (Drums keeps its kit notes); their tunings are their own or the song's
/// ([`TuningsHanded`]).
fn own_follow_synth(synth: &synth::Synth, own: &mut instruments::Instruments) {
    for t in 0..instruments::HOLDERS {
        if let Some(h) = own.held(t) {
            let e = synth::Engine::from_node_id(h.id).unwrap_or(synth::Engine::Poly);
            own.set_pads(t, synth.pads_for(e));
        }
    }
}

/// What the screen says when track `track` can't have the instrument
/// `name` because [`instruments::SLOTS`] other tracks already have their
/// own.
fn full_note(track: usize, name: &str) -> String {
    if track >= instruments::TRACKS {
        return format!(
            "No room for {name}: {} instruments are loaded, the most at once. Set a track to Kit first",
            instruments::SLOTS
        );
    }
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

/// How often (in periods) the synth-voice pads that want a synth of
/// their own are looked at again: about every 93 ms at 128 frames.
const PAD_SYNTH_PLAN: u64 = 32;

/// Asks the builder for the synth-voice pads' copies they lack
/// (`PadSynths::plan`): each a copy of its engine as the Synth tab has it
/// now. One past the sixteen is logged and said once, as it starts
/// playing through the Synth channel. Allocates only to ask.
fn plan_pad_synths(
    player: &sampler::Player,
    synth: &synth::Synth,
    mix: &mut mixing::MixHost,
    out: &mut Out,
) {
    let plan = mix.pad_synths.plan(player.voice_pads(), synth);
    for p in plan.asks.iter().flatten() {
        let (id, extras, values) = synth.sound_of(p.engine);
        let job = control::InstrumentJob {
            track: 0,
            pad: Some(*p),
            id,
            extras,
            values,
            pads: synth.pads_for(p.engine),
            tuning: synth.tuning_now(),
        };
        // Busy: asked again at the next look.
        if !mix.build_instrument(job) {
            mix.pad_synths.unask(*p);
        }
    }
    if let Some((kit, pad)) = plan.newly_over {
        let s = format!(
            "Kit {} pad {} plays through the Synth channel: {} synth-voice pads already have a synth of their own, the most ({} past them now)",
            kit + 1,
            profile::pad_name(pad),
            pad_synths::SLOTS,
            plan.over
        );
        out.note(&format!("pads: {s}"));
        out.show(gui::ToUi::Tell(s));
    }
    // The Pads tab says under such a pad where it plays.
    if plan.changed {
        let on: Vec<(u8, u8)> = mix
            .pad_synths
            .on_synth()
            .iter()
            .enumerate()
            .flat_map(|(k, bits)| {
                (0..128u8)
                    .filter(move |p| bits & 1 << p != 0)
                    .map(move |p| (k as u8, p))
            })
            .collect();
        out.show(gui::ToUi::PadsOnSynth(on));
    }
}

/// The log's line for a synth-voice pad's copy the builder made.
fn note_pad_landed(landed: mixing::PadLanded, out: &mut Out) {
    let name = |p: &pad_synths::PadFor| {
        format!(
            "kit {} pad {}",
            p.kit + 1,
            profile::pad_name(usize::from(p.pad))
        )
    };
    match landed {
        mixing::PadLanded::Playing { pad, name: synth } => out.note(&format!(
            "pads: {} plays its own {synth}, through the pad's own channel",
            name(&pad)
        )),
        mixing::PadLanded::Failed { pad } => {
            let s = format!(
                "{}: its own {} couldn't be made (see the Console); it plays through the Synth channel",
                name(&pad),
                pad.engine.name()
            );
            out.note(&format!("pads: {s}"));
            out.show(gui::ToUi::Tell(s));
        }
        mixing::PadLanded::Unwanted { pad } => out.note(&format!(
            "pads: {}'s own {} came after the pad changed; dropped",
            name(&pad),
            pad.engine.name()
        )),
    }
}

/// A job for track `track`'s own instrument `id`: an engine's copy of the
/// shared synth's sound with its design (or `preset`, a factory preset's
/// design lines and values, [`synth::Synth::preset_values`]), or a plugin
/// synth, with the pad layout and tuning now.
fn instrument_job(
    synth: &synth::Synth,
    track: usize,
    id: &'static str,
    preset: Option<synth::SoundCopy>,
) -> control::InstrumentJob {
    let engine = synth::Engine::from_node_id(id);
    let (extras, values) = match (preset, engine) {
        (Some(p), _) => p,
        (None, Some(e)) => {
            let (_, extras, values) = synth.sound_of(e);
            (extras, values)
        }
        (None, None) => (Vec::new(), Vec::new()),
    };
    control::InstrumentJob {
        track,
        pad: None,
        id,
        extras,
        values,
        pads: synth.pads_for(engine.unwrap_or(synth::Engine::Poly)),
        tuning: synth.tuning_now(),
    }
}

/// Pad notes from the arps one period can show; more still sound.
const MAX_LIVE_HITS: usize = 64;

impl time::Pads for SeqPads<'_> {
    /// Where track `track` is heard while it plays an instrument (its own,
    /// or another track's, step 4a): a clip's "this track's strip" goes
    /// there (time/strips.rs). A track on MIDI out sounds outside mpfree,
    /// and one on the pads (a kit, the synth) on its own strip, as before.
    fn sounds_on(&self, track: u16) -> Option<u8> {
        if self.midi.takes(track) {
            return None;
        }
        self.own
            .heard_on(usize::from(track))
            .and_then(|s| u8::try_from(s).ok())
    }
    fn hit_at(&mut self, track: u16, pad: usize, velocity: u8, offset: u32) -> bool {
        let kit = self.kit_of(track);
        // A clip's note on the synth's pads moves by its track's live
        // transpose (S4); the pad remembers the notes it started, so its
        // note-off and pressure reach them.
        let shift = self.shift(track);
        let on = mpfree_play::Event::NoteOn {
            note: pad.min(127) as u8,
            velocity,
        };
        let sounded = if let Some(s) = self.synth_of(track) {
            if pad < 16 {
                let by = synth::Player::Clip(usize::from(track));
                s.note_on_shifted(by, pad, velocity, offset, &shift);
            }
            pad < 16
        } else if self.routed(track, pad, on, offset) {
            true
        } else {
            self.player.hit_kit_at(kit, pad, velocity, offset)
        };
        if sounded {
            self.hits += 1;
            if self.kits.entry(usize::from(track)) == self.shown {
                if let Some(f) = self.flashes.get_mut(pad) {
                    *f = velocity.max(1);
                }
            }
        }
        sounded
    }
    fn release_at(&mut self, track: u16, pad: usize, offset: u32) {
        let kit = self.kit_of(track);
        let off = mpfree_play::Event::NoteOff {
            note: pad.min(127) as u8,
        };
        if let Some(s) = self.synth_of(track) {
            s.note_off_by(synth::Player::Clip(usize::from(track)), pad, offset);
        } else if !self.routed(track, pad, off, offset) {
            self.player.release_kit_at(kit, pad, offset);
        }
    }
    /// A panic: every kit, the synth and every track's own instrument.
    fn stop_all(&mut self) {
        self.synth.all_off();
        self.pad_synths.all_off();
        self.player.stop_all();
        self.own.all_off_every();
        // Every note ends, wherever its shift took it.
        for h in self.by_note.iter_mut() {
            h.clear();
        }
    }
    fn pressure_at(&mut self, track: u16, pad: usize, value: u8, offset: u32) {
        let by = synth::Player::Clip(usize::from(track));
        self.pressure_by(by, track, pad, value, offset);
    }

    /// An arp's note: the pads played live (through the armed track's arp,
    /// a latched arp, or a clip through its arp). It gets the velocity
    /// curve; with a synth pad layout it is a MIDI note for the synth and
    /// its pad (if any) is what shows and records; otherwise it is a pad.
    /// Returns what the armed track records as heard: the pad, and the
    /// notes as written for a take of notes (step 5c).
    fn live(&mut self, o: &mpfree_play::Out) -> time::Heard {
        use mpfree_play::Event;
        use time::Heard;
        if self.quiet(o) {
            return Heard::NONE;
        }
        // An arp that works in notes says so (step 5d); the synth's
        // layout on the pads presses notes too.
        let notes =
            o.notes || (self.kits.plays_synth(usize::from(o.track)) && self.live.plays_notes());
        let shift = self.shift(o.track);
        // By note number, a note sounds where its track's shift took it
        // when it started (`sound`), and shows and records as written
        // (`o`). By pad, the pad's notes move as they start.
        let sound = if notes {
            match self.sounding(o, &shift) {
                Some(s) => s,
                None => return self.live_unheard(o),
            }
        } else {
            *o
        };
        self.cv_live(&sound, notes, &shift);
        if self.midi.takes(o.track) {
            return self.live_out(&sound, o, notes, &shift);
        }
        if self.own.plays(usize::from(o.track)) {
            return self.live_own(&sound, o, notes, &shift);
        }
        // A kit played as notes (sampler step 15c, part 3): its notes play
        // the kit as its Keys play sends them, never the synth.
        let kit = self.kit_of(o.track);
        if notes
            && is_note(o.event)
            && self.plays_kit(o.track)
            && self.player.kit_keys(kit).takes_notes()
        {
            return self.live_kit_note(&sound, o);
        }
        match (o.event, sound.event) {
            (Event::NoteOn { note, velocity }, Event::NoteOn { note: heard, .. }) => {
                let v = self.live.velocity(velocity);
                let on = Event::NoteOn { note, velocity: v };
                if notes {
                    // A note no pad has (an arp octave up, a scale run)
                    // still plays, and a take of notes records it; only a
                    // pad's own note shows, and records as a pad.
                    self.synth.note_on_note(heard, v, o.offset);
                    self.hits += 1;
                    let pad = self.live.pad_of_note(note);
                    if let Some(pad) = pad {
                        self.live_hit(pad, v);
                    }
                    return Heard::of(pad.map(|p| pad_on(p, v)), on, &[note]);
                }
                let pad = {
                    // A kit's pad, 1 to 128 (the page's, as the pads
                    // pressed it), or the synth's slot.
                    let pad = usize::from(note);
                    if pad >= 128 {
                        return Heard::NONE;
                    }
                    let kit = self.kit_of(o.track);
                    let on = Event::NoteOn {
                        note: pad as u8,
                        velocity: v,
                    };
                    if let Some(s) = self.synth_of(o.track) {
                        let by = synth::Player::Arp(usize::from(o.track));
                        s.note_on_shifted(by, pad, v, o.offset, &shift);
                        self.hits += 1;
                    } else if self.routed(o.track, pad, on, o.offset) {
                        // An Instrument or MIDI out pad: its voice, its note out.
                        self.hits += 1;
                    } else if self.player.live_hit_at(kit, pad, v, o.offset) {
                        // While live chop takes cuts, a hit marks one.
                        self.hits += 1;
                    } else {
                        self.silent += 1;
                    }
                    pad
                };
                self.live_hit(pad, v);
                let (written, n) = self.pad_notes(o.track, pad);
                Heard::of(Some(pad_on(pad, v)), on, &written[..n])
            }
            (Event::NoteOff { note }, Event::NoteOff { note: heard }) if notes => {
                self.synth.note_off_note(heard, o.offset);
                let pad = self.live.pad_of_note(note);
                Heard::of(pad.map(pad_off), o.event, &[note])
            }
            (Event::NoteOff { note }, _) => {
                // The synth's note ends; a Grains pad stops when let go;
                // one-shot sounds ignore it.
                let kit = self.kit_of(o.track);
                if let Some(s) = self.synth_of(o.track) {
                    let by = synth::Player::Arp(usize::from(o.track));
                    s.note_off_by(by, usize::from(note), o.offset);
                } else if !self.routed(o.track, usize::from(note), o.event, o.offset) {
                    self.player
                        .live_release_at(kit, usize::from(note), o.offset);
                }
                let (written, n) = self.pad_notes(o.track, usize::from(note));
                Heard::of(Some(o.event), o.event, &written[..n])
            }
            (Event::AllOff, _) => {
                self.midi.all_off(o.offset);
                self.stop_all();
                Heard::NONE
            }
            (Event::Pressure { note, value }, _) => {
                let by = synth::Player::Arp(usize::from(o.track));
                self.pressure_by(by, o.track, usize::from(note), value, o.offset);
                Heard::NONE
            }
            (Event::NoteOn { .. }, _)
            | (
                Event::Lock { .. }
                | Event::Lane { .. }
                | Event::Unlock { .. }
                | Event::Program { .. },
                _,
            ) => Heard::NONE,
        }
    }

    /// A clip's lock on a sound: a track's own instrument, the synth while
    /// it has the pads, or the playing kit's pad. A track that plays MIDI
    /// Out has none of these (its locks go on to the mixer).
    fn lock_at(
        &mut self,
        addr: mpfree_play::mpfree_param::ParamAddr,
        value: Option<f32>,
        offset: u32,
    ) -> bool {
        use mpfree_play::mpfree_param::Owner;
        if addr
            .owner
            .track()
            .is_some_and(|t| self.midi.takes(u16::from(t)))
        {
            return false;
        }
        match addr.owner {
            Owner::Inst(t) if self.own.plays(usize::from(t)) => {
                self.own.lock(usize::from(t), addr.param, value, offset)
            }
            Owner::Inst(t) => self
                .synth_of(u16::from(t))
                .is_some_and(|s| s.lock(usize::from(t), addr.param, value, offset)),
            Owner::Pad(t, pad)
                if !self.own.plays(usize::from(t)) && !self.kits.plays_synth(usize::from(t)) =>
            {
                let kit = self.kit_of(u16::from(t));
                sampler::PadParam::of(addr.param).is_some_and(|p| {
                    self.player
                        .pad_param_lock(kit, usize::from(pad), p, value, offset)
                })
            }
            _ => false,
        }
    }

    /// A clip's event on a track that plays on MIDI Out: sent there, and
    /// the pads don't play it (its pad still flashes). A panic ends MIDI
    /// Out's notes and goes on to the pads.
    fn out(&mut self, o: &mpfree_play::Out) -> bool {
        if self.quiet(o) {
            return true;
        }
        // A clip of notes (step 5c): its notes are never pads.
        if o.notes && is_note(o.event) {
            return self.out_note(o);
        }
        let pad = event_pad(o.event);
        // A track that plays notes plays its pad's notes moved by its live
        // transpose (S4); a note-off goes to what its note-on sent.
        let shift = self.shift(o.track);
        let mut buf = [0; MAX_PAD_NOTES];
        let is = shifted_is(
            pad_is(synth_seen(self.kits, self.synth, o.track), pad),
            &shift,
            &mut buf,
        );
        self.cv.send(o, is);
        self.tap.send(o, is);
        let taken = self.midi.send(o, is) || self.clip_own(o, &shift);
        if taken {
            self.flash(o);
        }
        taken
    }

    /// A clip's event on a track whose gear needs more than the sound's
    /// lead (sequencer S5), before its period: MIDI out alone sends it
    /// now, early enough for the gear's delay.
    fn out_ahead(&mut self, o: &mpfree_play::Out) -> bool {
        if self.quiet(o) {
            return false;
        }
        if mpfree_play::is_gear_control(o) {
            return self.control_ahead(o);
        }
        // A clip's note by number goes where its track's shift takes it
        // as it starts, remembered for its end (`sounding`). Only once
        // MIDI out will take it, so a note left for the real pass isn't
        // counted twice.
        if o.notes && is_note(o.event) {
            if !self.midi.takes(o.track) {
                return false;
            }
            let shift = self.note_shift(o.track);
            return match self.sounding(o, &shift) {
                Some(sound) => self.midi.send(&sound, midi_out::NoteIs::Note),
                None => false,
            };
        }
        let shift = self.shift(o.track);
        let mut buf = [0; MAX_PAD_NOTES];
        let is = shifted_is(
            pad_is(
                synth_seen(self.kits, self.synth, o.track),
                event_pad(o.event),
            ),
            &shift,
            &mut buf,
        );
        self.midi.send(o, is)
    }

    /// The same event in its own period, MIDI out having sent it ahead:
    /// the rest of what `out` does (CV out, an export's MIDI file, the
    /// pad's flash).
    fn out_sent(&mut self, o: &mpfree_play::Out) {
        if self.quiet(o) {
            return;
        }
        // A clip's note by number: where it went when it went ahead.
        if o.notes && is_note(o.event) {
            let sound = self.went(o);
            self.cv.send(&sound, midi_out::NoteIs::Note);
            self.tap.send(&sound, midi_out::NoteIs::Note);
            self.flash_note(o);
            return;
        }
        let shift = self.shift(o.track);
        let mut buf = [0; MAX_PAD_NOTES];
        let is = shifted_is(
            pad_is(
                synth_seen(self.kits, self.synth, o.track),
                event_pad(o.event),
            ),
            &shift,
            &mut buf,
        );
        self.cv.send(o, is);
        self.tap.send(o, is);
        // A lock or lane on the track's gear that MIDI out sent ahead
        // never reaches the mod engine: the file takes it here, on its
        // frame (an unlock as the control's own value, as it went out).
        if mpfree_play::is_gear_control(o) {
            self.tap
                .gear_control(o, |addr| self.modu.engine().base(addr));
        }
        self.flash(o);
    }
}

/// Pad `pad`'s hit at velocity `v`, as a take of pad hits records it.
fn pad_on(pad: usize, v: u8) -> mpfree_play::Event {
    mpfree_play::Event::NoteOn {
        note: pad as u8,
        velocity: v,
    }
}

/// Pad `pad`'s let-go, as a take of pad hits records it.
fn pad_off(pad: usize) -> mpfree_play::Event {
    mpfree_play::Event::NoteOff { note: pad as u8 }
}

/// A note's start, end or pressure (not a panic, lock or program).
fn is_note(e: mpfree_play::Event) -> bool {
    use mpfree_play::Event;
    matches!(
        e,
        Event::NoteOn { .. } | Event::NoteOff { .. } | Event::Pressure { .. }
    )
}

/// The pad a clip's note event is on (0 for the rest).
fn event_pad(e: mpfree_play::Event) -> usize {
    use mpfree_play::Event;
    match e {
        Event::NoteOn { note, .. } | Event::NoteOff { note } | Event::Pressure { note, .. } => {
            usize::from(note)
        }
        _ => 0,
    }
}

/// The synth, if track `track` plays it (`kits`): to read its pad notes.
fn synth_seen<'s>(
    kits: &sampler::TrackKits,
    synth: &'s synth::Synth,
    track: u16,
) -> Option<&'s synth::Synth> {
    kits.plays_synth(usize::from(track)).then_some(synth)
}

/// The instrument each track plays now (step sequencer refactor F1), into
/// `plays` by track, as `SeqPads::lock_at` finds a lock's sound: the
/// track's own instrument, the synth while it has the pads, or none (a
/// kit, or a track MIDI Out takes, whose instrument locks reach nothing).
/// Never allocates.
fn track_instruments(
    plays: &mut [Option<time::InstRef>],
    own: &instruments::Instruments,
    kits: &sampler::TrackKits,
    synth: &synth::Synth,
    midi: &midi_out::MidiOut,
) {
    for (t, p) in plays.iter_mut().enumerate() {
        *p = if midi.takes(t as u16) {
            None
        } else if own.plays(t) {
            own.node(t).map(|n| (n.info(), n.params()))
        } else if kits.plays_synth(t) {
            Some(synth.node_ref())
        } else {
            None
        };
    }
}

impl SeqPads<'_> {
    /// Pressure on `pad`'s sounding note of `track`, from `by` (its clip,
    /// or its arp): the synth's notes that source started, else the pad's
    /// kind (an Instrument or MIDI out pad), else the kit's pad.
    fn pressure_by(&mut self, by: synth::Player, track: u16, pad: usize, value: u8, offset: u32) {
        let kit = self.kit_of(track);
        let press = mpfree_play::Event::Pressure {
            note: pad.min(127) as u8,
            value,
        };
        if let Some(s) = self.synth_of(track) {
            s.pressure_at(by, pad, value, offset);
        } else if !self.routed(track, pad, press, offset) {
            self.player.pressure_kit_at(kit, pad, value, offset);
        }
    }

    /// A clip's lock or lane on the track's own MIDI out control, from a
    /// track that plays ahead: MIDI out sends it, ahead or (missed by the
    /// forecast) on its frame, unless something else moves the control
    /// (a macro, an LFO row, the morph, a hand, or a lock the mod engine
    /// still holds): then it goes to the mod engine a period at a time,
    /// as before. An unlock sends the control's own value, the engine's.
    fn control_ahead(&mut self, o: &mpfree_play::Out) -> bool {
        use mpfree_play::Event;
        let (addr, value, lane) = match o.event {
            Event::Lock { addr, value } => (addr, Some(value), false),
            Event::Lane { addr, value } => (addr, Some(value), true),
            Event::Unlock { addr } => (addr, None, false),
            _ => return false,
        };
        // A control the mod engine can't hold either (the pressure, which
        // has no own value to go back to) is left to it, as before.
        let Some((_, default)) = modmix::midi_param(addr) else {
            return false;
        };
        let e = self.modu.engine();
        if e.is_moved(addr) || e.is_touched(addr) || e.is_locked(addr) {
            self.midi.release_ahead(addr);
            return false;
        }
        let v = value.unwrap_or_else(|| e.base(addr).unwrap_or(default));
        self.midi
            .control_ahead(o.offset, addr, v, value.is_some(), lane)
    }

    /// A clip's note by number (a clip of notes, step 5c), wherever its
    /// track plays. MIDI out, CV and an export's file get the note as a
    /// note. A track's own instrument or the synth plays it by number,
    /// moved by the track's live transpose as it started (`sounding`, so
    /// its end and pressure reach that note). A kit answers it as it
    /// answers MIDI in, by its Keys play (sampler step 15): each note on
    /// one pad (`mpfree_midi::profile::kit_pad_of_note`), or through the
    /// keygroups that hold it. Always taken: the pads never read a note
    /// as a pad number.
    fn out_note(&mut self, o: &mpfree_play::Out) -> bool {
        use mpfree_play::Event;
        let track = usize::from(o.track);
        if self.plays_kit(o.track) {
            // As the kit's Keys play sends the note (sampler step 15): its
            // keygroups take it as a key, by pitch, or the pad that sends
            // it plays; its let-go and pressure go where it went down.
            let kit = self.kit_of(o.track);
            let to = match o.event {
                Event::NoteOn { note, .. } => Some(self.player.note_on_to(kit, note)),
                Event::NoteOff { note } => Some(self.player.note_off_to(kit, note)),
                Event::Pressure { note, .. } => Some(self.player.note_held_to(kit, note)),
                _ => None,
            };
            if let Some(sampler::NoteTo::Keys(n)) = to {
                self.cv.send(o, midi_out::NoteIs::Note);
                self.tap.send(o, midi_out::NoteIs::Note);
                match o.event {
                    Event::NoteOn { velocity, .. } => {
                        // A note no keygroup holds plays nothing.
                        let holds = self.player.kit_keys(kit).holds(n);
                        if self.player.key_kit_at(kit, n, velocity, o.offset) && holds {
                            self.hits += 1;
                        } else {
                            self.silent += 1;
                        }
                    }
                    Event::NoteOff { .. } => self.player.key_off_kit_at(kit, n, o.offset),
                    // A keygroup's key takes no pressure yet.
                    _ => {}
                }
                return true;
            }
            let pad = |note: u8| match to {
                Some(sampler::NoteTo::Pad(p)) => p,
                _ => mpfree_midi::profile::kit_pad_of_note(note),
            };
            let event = match o.event {
                Event::NoteOn { note, velocity } => Event::NoteOn {
                    note: pad(note) as u8,
                    velocity,
                },
                Event::NoteOff { note } => Event::NoteOff {
                    note: pad(note) as u8,
                },
                Event::Pressure { note, value } => Event::Pressure {
                    note: pad(note) as u8,
                    value,
                },
                e => e,
            };
            let hit = mpfree_play::Out {
                event,
                notes: false,
                ..*o
            };
            self.cv.send(&hit, midi_out::NoteIs::Kit);
            self.tap.send(&hit, midi_out::NoteIs::Kit);
            match event {
                Event::NoteOn { note, velocity } => {
                    time::Pads::hit_at(self, o.track, usize::from(note), velocity, o.offset);
                }
                Event::NoteOff { note } => {
                    time::Pads::release_at(self, o.track, usize::from(note), o.offset)
                }
                Event::Pressure { note, value } => {
                    time::Pads::pressure_at(self, o.track, usize::from(note), value, o.offset)
                }
                _ => {}
            }
            return true;
        }
        let shift = self.note_shift(o.track);
        // Nothing sounds: it fell off the keyboard, or ended already.
        let Some(sound) = self.sounding(o, &shift) else {
            return true;
        };
        self.cv.send(&sound, midi_out::NoteIs::Note);
        self.tap.send(&sound, midi_out::NoteIs::Note);
        if self.midi.send(&sound, midi_out::NoteIs::Note) {
            self.flash_note(o);
            return true;
        }
        let own = self.own.plays(track);
        match sound.event {
            Event::NoteOn { note, velocity } => {
                if own {
                    self.own.note_on(track, note, velocity, o.offset);
                } else {
                    self.synth.note_on_note(note, velocity, o.offset);
                }
                self.hits += 1;
                self.flash_note(o);
            }
            Event::NoteOff { note } if own => {
                self.own.note_off(track, note, o.offset);
            }
            Event::NoteOff { note } => self.synth.note_off_note(note, o.offset),
            Event::Pressure { note, value } if own => {
                self.own.note_pressure_at(track, note, value, o.offset);
            }
            Event::Pressure { note, value } => self.synth.pressure_note_at(note, value, o.offset),
            _ => {}
        }
        true
    }

    /// Track `track` plays a kit's sounds on the pads: not MIDI out, its
    /// own instrument or another track's, or the synth.
    fn plays_kit(&self, track: u16) -> bool {
        let t = usize::from(track);
        !self.midi.takes(track) && !self.own.plays(t) && !self.kits.plays_synth(t)
    }

    /// Track `track`'s live transpose (S4) for a clip of notes: its notes
    /// are notes whatever the pads show, so they move, unless they go to
    /// the Drums engine (or a kit, which never gets here).
    fn note_shift(&self, track: u16) -> Shift {
        let t = usize::from(track);
        let Some(&to) = self.shifts.get(t) else {
            return Shift::NONE;
        };
        let drums = if self.midi.takes(track) {
            false
        } else if self.own.plays(t) {
            !self.own.plays_notes(t)
        } else {
            !self.synth.plays_notes()
        };
        if to.is_none() || drums {
            return Shift::NONE;
        }
        Shift {
            by: to.by,
            key: self.key.filter(|_| to.in_key),
        }
    }

    /// A clip's note by number that went out ahead (S5): where its track's
    /// shift took it then, for the rest of what it does in its own period.
    fn went(&self, o: &mpfree_play::Out) -> mpfree_play::Out {
        use mpfree_play::Event;
        let Some(held) = self.by_note.get(usize::from(o.track)) else {
            return *o;
        };
        let event = match o.event {
            Event::NoteOn { note, velocity } => Event::NoteOn {
                note: held.went(note),
                velocity,
            },
            Event::NoteOff { note } => Event::NoteOff {
                note: held.went(note),
            },
            Event::Pressure { note, value } => Event::Pressure {
                note: held.went(note),
                value,
            },
            e => e,
        };
        mpfree_play::Out { event, ..*o }
    }

    /// A clip's note-on by number flashes the pad that plays the note in
    /// the pads' layout, while its track's pads are the ones shown.
    fn flash_note(&mut self, o: &mpfree_play::Out) {
        if let mpfree_play::Event::NoteOn { note, velocity } = o.event {
            if self.kits.entry(usize::from(o.track)) != self.shown {
                return;
            }
            if let Some(f) = self
                .live
                .pad_of_note(note)
                .and_then(|p| self.flashes.get_mut(p))
            {
                *f = velocity.max(1);
            }
        }
    }

    /// A note-on that went out elsewhere (MIDI out, a track's own
    /// instrument) still flashes its pad.
    fn flash(&mut self, o: &mpfree_play::Out) {
        if let mpfree_play::Event::NoteOn { velocity, .. } = o.event {
            if let Some(f) = self.flashes.get_mut(event_pad(o.event)) {
                *f = velocity.max(1);
            }
        }
    }

    /// The synth, if track `track` plays it rather than a kit.
    fn synth_of(&mut self, track: u16) -> Option<&mut synth::Synth> {
        if self.kits.plays_synth(usize::from(track)) {
            Some(&mut *self.synth)
        } else {
            None
        }
    }

    /// The kit track `track` plays (kit 1 if it plays the synth).
    fn kit_of(&self, track: u16) -> usize {
        self.kits.kit_of(usize::from(track)).unwrap_or(0)
    }

    /// A hit, end or pressure (`event`) on pad `pad` of the kit track
    /// `track` plays, when the pad is of another kind (the Pads tab's
    /// Instrument and MIDI out pads, [`sampler::PadRoute`]): one voice of
    /// a synth engine, or a note out to the pad's own port, channel and
    /// note (`MidiOut::pad_to`, whatever the track's Plays on). True when
    /// the pad is one of those, sent or not (a port that isn't plugged in
    /// sends nothing, as the Pads tab says); false leaves it to the
    /// sampler. Never allocates.
    fn routed(&mut self, track: u16, pad: usize, event: mpfree_play::Event, offset: u32) -> bool {
        let kit = self.kit_of(track);
        let Some(route) = self.player.pad_route(kit, pad) else {
            return false;
        };
        match route {
            sampler::PadRoute::Midi {
                port,
                channel,
                note,
            } => {
                let o = mpfree_play::Out {
                    offset,
                    frame: 0,
                    track,
                    dest: Default::default(),
                    event,
                    notes: false,
                };
                self.midi.pad_to(&o, port, *channel, *note);
            }
            &sampler::PadRoute::Voice(engine, note) => pad_voice(
                self.pad_synths,
                self.synth,
                track,
                (kit, pad),
                (engine, note),
                event,
                offset,
            ),
        }
        true
    }

    /// An export's replay plays one track alone: another track's note on
    /// the pads is kept quiet (taken, so nothing plays or records it).
    fn quiet(&self, o: &mpfree_play::Out) -> bool {
        self.only.is_some() && {
            let t = usize::from(o.track);
            let outside = self.midi.takes(o.track);
            // Track 1's own instrument shares Drums with the tracks on
            // kit 1 (R3 part 4b), so their replays keep it quiet too, and
            // a track's on another track's instrument.
            let on_pads = !outside
                && self
                    .own
                    .owner(t)
                    .is_none_or(|o| o == 0 && self.own.channel(0) == mixing::DRUMS);
            export::quiets(self.only, o.track, on_pads, o.event)
        }
    }

    /// Track `track`'s live transpose (S4) as a shift of its notes: none
    /// for a track whose pads are a kit's sounds (or the Drums engine's),
    /// which never transposes.
    fn shift(&self, track: u16) -> Shift {
        let t = usize::from(track);
        let Some(&to) = self.shifts.get(t) else {
            return Shift::NONE;
        };
        let synth_notes = self.kits.plays_synth(t) && self.synth.plays_notes();
        let notes = if self.midi.takes(track) || !self.own.plays(t) {
            synth_notes
        } else {
            self.own.plays_notes(t)
        };
        if to.is_none() || !notes {
            return Shift::NONE;
        }
        Shift {
            by: to.by,
            key: self.key.filter(|_| to.in_key),
        }
    }

    /// An arp's note by number (a synth pad layout) as it sounds: where
    /// its track's shift took it when it started, so its end and pressure
    /// reach that note. None when nothing sounds: it fell off the
    /// keyboard, or it already ended.
    fn sounding(&mut self, o: &mpfree_play::Out, shift: &Shift) -> Option<mpfree_play::Out> {
        use mpfree_play::Event;
        let Some(held) = self.by_note.get_mut(usize::from(o.track)) else {
            return Some(*o);
        };
        let event = match o.event {
            Event::NoteOn { note, velocity } => Event::NoteOn {
                note: held.on(note, shift)?,
                velocity,
            },
            Event::NoteOff { note } => Event::NoteOff {
                note: held.off(note)?,
            },
            Event::Pressure { note, value } => Event::Pressure {
                note: held.sounding(note)?,
                value,
            },
            e => e,
        };
        Some(mpfree_play::Out { event, ..*o })
    }

    /// An arp's note by number that sounds nowhere (its shift took it off
    /// the keyboard): its pad still shows, and it records as written.
    fn live_unheard(&mut self, o: &mpfree_play::Out) -> time::Heard {
        use mpfree_play::Event;
        use time::Heard;
        match o.event {
            Event::NoteOn { note, velocity } => {
                let v = self.live.velocity(velocity);
                let pad = self.live.pad_of_note(note);
                if let Some(pad) = pad {
                    self.live_hit(pad, v);
                }
                let on = Event::NoteOn { note, velocity: v };
                Heard::of(pad.map(|p| pad_on(p, v)), on, &[note])
            }
            Event::NoteOff { note } => {
                let pad = self.live.pad_of_note(note);
                Heard::of(pad.map(pad_off), o.event, &[note])
            }
            _ => Heard::NONE,
        }
    }

    /// A live hit on `pad` at velocity `v`, for the pads' flashes and the
    /// Play page.
    /// A live note (`o` as written, `sound` as it sounds) on a track
    /// playing a kit played as notes (sampler step 15c, part 3): it plays
    /// the pad that plays its note, or the keygroups that hold it, as the
    /// kit's Keys play sends it (`sampler::Player::note_on_to`, as a clip's
    /// note does in `out_note`), and records as written; the pad that plays
    /// it flashes. Its let-go and pressure go where it went down.
    fn live_kit_note(&mut self, sound: &mpfree_play::Out, o: &mpfree_play::Out) -> time::Heard {
        use mpfree_play::Event;
        use time::Heard;
        let kit = self.kit_of(o.track);
        let keys = self.player.kit_keys(kit);
        match (o.event, sound.event) {
            (Event::NoteOn { note, velocity }, Event::NoteOn { note: heard, .. }) => {
                let v = self.live.velocity(velocity);
                let played = match self.player.note_on_to(kit, heard) {
                    sampler::NoteTo::Pad(p) => self.player.live_hit_at(kit, p, v, o.offset),
                    sampler::NoteTo::Keys(n) => {
                        // A note no keygroup holds plays nothing.
                        self.player.key_kit_at(kit, n, v, o.offset) && keys.holds(n)
                    }
                };
                if played {
                    self.hits += 1;
                } else {
                    self.silent += 1;
                }
                let pad = keys.pad_of_note(note).map(usize::from);
                if let Some(p) = pad {
                    self.live_hit(p, v);
                }
                let on = Event::NoteOn { note, velocity: v };
                Heard::of(pad.map(|p| pad_on(p, v)), on, &[note])
            }
            (Event::NoteOff { note }, Event::NoteOff { note: heard }) => {
                match self.player.note_off_to(kit, heard) {
                    sampler::NoteTo::Pad(p) => self.player.live_release_at(kit, p, o.offset),
                    sampler::NoteTo::Keys(n) => self.player.key_off_kit_at(kit, n, o.offset),
                }
                let pad = keys.pad_of_note(note).map(usize::from);
                Heard::of(pad.map(pad_off), o.event, &[note])
            }
            (Event::Pressure { value, .. }, Event::Pressure { note: heard, .. }) => {
                // A keygroup's key takes no pressure yet.
                if let sampler::NoteTo::Pad(p) = self.player.note_held_to(kit, heard) {
                    self.player.pressure_kit_at(kit, p, value, o.offset);
                }
                Heard::NONE
            }
            _ => Heard::NONE,
        }
    }

    fn live_hit(&mut self, pad: usize, v: u8) {
        if self.n_live < MAX_LIVE_HITS {
            self.live_hits[self.n_live] = (pad as u8, v);
            self.n_live += 1;
        }
    }

    /// The notes pad `pad` of track `track` plays as written (before its
    /// live transpose), for a take of notes (step 5c): its own instrument's
    /// pad map (or the one it plays of another track's), the synth's pad
    /// layout while the track plays the synth, else the kit pad's note as
    /// MIDI out and MIDI in read it (`mpfree_midi::profile::kit_note`).
    fn pad_notes(&self, track: u16, pad: usize) -> ([u8; time::HEARD_NOTES], usize) {
        let t = usize::from(track);
        let mut out = [0; time::HEARD_NOTES];
        let kit;
        let notes: &[u8] = if self.own.plays(t) {
            self.own.pad_notes(t, pad)
        } else if self.kits.plays_synth(t) {
            self.synth.notes(pad)
        } else {
            kit = mpfree_midi::profile::kit_note(pad);
            kit.as_slice()
        };
        let n = notes.len().min(out.len());
        out[..n].copy_from_slice(&notes[..n]);
        (out, n)
    }

    /// A clip's event on a track with its own instrument, or another
    /// track's (step 4): played there, not on the pads, its notes moved
    /// by the track's live transpose. A panic or a lock goes on as usual.
    fn clip_own(&mut self, o: &mpfree_play::Out, shift: &Shift) -> bool {
        use mpfree_play::Event;
        let track = usize::from(o.track);
        if !self.own.plays(track) {
            return false;
        }
        match o.event {
            Event::NoteOn { note, velocity } => {
                if self
                    .own
                    .pad_on_shifted(track, usize::from(note), velocity, o.offset, shift)
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
            // A clip's program change is for outside gear (S5), not the
            // track's own instrument.
            Event::Lock { .. }
            | Event::Lane { .. }
            | Event::Unlock { .. }
            | Event::Program { .. } => false,
        }
    }

    /// An arp's note (the pads played live) on a track with its own
    /// instrument: played there with the pads' treatment. With a synth pad
    /// layout (`notes`) it is a MIDI note, `sound` where the track's shift
    /// took it and `o` as written; otherwise a pad, which the track's own
    /// pad map turns into notes, moved by `shift`. Returns what records.
    fn live_own(
        &mut self,
        sound: &mpfree_play::Out,
        o: &mpfree_play::Out,
        notes: bool,
        shift: &Shift,
    ) -> time::Heard {
        use mpfree_play::Event;
        use time::Heard;
        let track = usize::from(o.track);
        match (o.event, sound.event) {
            (Event::NoteOn { note, velocity }, Event::NoteOn { note: heard, .. }) => {
                let v = self.live.velocity(velocity);
                let on = Event::NoteOn { note, velocity: v };
                if notes {
                    self.own.note_on(track, heard, v, o.offset);
                    self.hits += 1;
                    let pad = self.live.pad_of_note(note);
                    if let Some(pad) = pad {
                        self.live_hit(pad, v);
                    }
                    return Heard::of(pad.map(|p| pad_on(p, v)), on, &[note]);
                }
                let pad = usize::from(note);
                if pad >= 16 {
                    return Heard::NONE;
                }
                self.own.pad_on_shifted(track, pad, v, o.offset, shift);
                self.hits += 1;
                self.live_hit(pad, v);
                let (written, n) = self.pad_notes(o.track, pad);
                Heard::of(Some(pad_on(pad, v)), on, &written[..n])
            }
            (Event::NoteOff { note }, Event::NoteOff { note: heard }) if notes => {
                self.own.note_off(track, heard, o.offset);
                let pad = self.live.pad_of_note(note);
                Heard::of(pad.map(pad_off), o.event, &[note])
            }
            (Event::NoteOff { note }, _) => {
                self.own.pad_off(track, usize::from(note), o.offset);
                let (written, n) = self.pad_notes(o.track, usize::from(note));
                Heard::of(Some(o.event), o.event, &written[..n])
            }
            (Event::AllOff, _) => {
                self.midi.all_off(o.offset);
                self.own.all_off(track);
                time::Pads::stop_all(self);
                Heard::NONE
            }
            (Event::Pressure { note, value }, _) if !notes => {
                self.own
                    .pad_pressure(track, usize::from(note), value, o.offset);
                Heard::NONE
            }
            (Event::NoteOn { .. }, _)
            | (
                Event::Pressure { .. }
                | Event::Lock { .. }
                | Event::Lane { .. }
                | Event::Unlock { .. }
                | Event::Program { .. },
                _,
            ) => Heard::NONE,
        }
    }

    /// An arp's note to the CV outputs, with the pads' velocity curve, as
    /// MIDI Out would read it (`notes`: MIDI notes from a synth pad
    /// layout, `o` where the track's shift took them; else pads, whose
    /// notes move by `shift`).
    fn cv_live(&mut self, o: &mpfree_play::Out, notes: bool, shift: &Shift) {
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
        let mut buf = [0; MAX_PAD_NOTES];
        let is = match (notes, event) {
            (true, _) => NoteIs::Note,
            (false, Event::NoteOn { .. }) if pad < KIT_PADS => shifted_is(
                pad_is(synth_seen(self.kits, self.synth, o.track), pad),
                shift,
                &mut buf,
            ),
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
    /// MIDI notes, from a synth pad layout, `sound` where the track's
    /// shift took them and `o` as written; else pads, whose notes move by
    /// `shift`).
    fn live_out(
        &mut self,
        sound: &mpfree_play::Out,
        o: &mpfree_play::Out,
        notes: bool,
        shift: &Shift,
    ) -> time::Heard {
        use midi_out::NoteIs;
        use mpfree_play::Event;
        use time::Heard;
        match (o.event, sound.event) {
            (Event::NoteOn { note, velocity }, Event::NoteOn { note: heard, .. }) => {
                let v = self.live.velocity(velocity);
                let on = Event::NoteOn { note, velocity: v };
                if notes {
                    let played = mpfree_play::Out {
                        event: Event::NoteOn {
                            note: heard,
                            velocity: v,
                        },
                        ..*o
                    };
                    self.midi.send(&played, NoteIs::Note);
                    let pad = self.live.pad_of_note(note);
                    if let Some(pad) = pad {
                        self.live_hit(pad, v);
                    }
                    return Heard::of(pad.map(|p| pad_on(p, v)), on, &[note]);
                }
                let pad = {
                    let pad = usize::from(note);
                    if pad >= KIT_PADS {
                        return Heard::NONE;
                    }
                    let played = mpfree_play::Out {
                        event: Event::NoteOn { note, velocity: v },
                        ..*o
                    };
                    let mut buf = [0; MAX_PAD_NOTES];
                    let is = shifted_is(
                        pad_is(synth_seen(self.kits, self.synth, o.track), pad),
                        shift,
                        &mut buf,
                    );
                    self.midi.send(&played, is);
                    pad
                };
                self.live_hit(pad, v);
                let (written, n) = self.pad_notes(o.track, pad);
                Heard::of(Some(pad_on(pad, v)), on, &written[..n])
            }
            (Event::NoteOff { note }, _) if notes => {
                self.midi.send(sound, NoteIs::Note);
                let pad = self.live.pad_of_note(note);
                Heard::of(pad.map(pad_off), o.event, &[note])
            }
            (Event::NoteOff { note }, _) => {
                self.midi.send(o, NoteIs::Pad(&[]));
                let (written, n) = self.pad_notes(o.track, usize::from(note));
                Heard::of(Some(o.event), o.event, &written[..n])
            }
            (Event::Pressure { .. }, _) => {
                if notes {
                    self.midi.send(sound, NoteIs::Note);
                } else {
                    self.midi.send(o, NoteIs::Pad(&[]));
                }
                Heard::NONE
            }
            (Event::AllOff, _) => {
                self.midi.all_off(o.offset);
                time::Pads::stop_all(self);
                Heard::NONE
            }
            (Event::NoteOn { .. }, _)
            | (
                Event::Lock { .. }
                | Event::Lane { .. }
                | Event::Unlock { .. }
                | Event::Program { .. },
                _,
            ) => Heard::NONE,
        }
    }
}

/// [`shifted`] on a synth pad layout's notes; a kit's pad (`NoteIs::Kit`)
/// isn't moved (its track's shift is none, [`SeqPads::shift`]).
fn shifted_is<'a>(
    is: midi_out::NoteIs<'a>,
    shift: &Shift,
    buf: &'a mut [u8; MAX_PAD_NOTES],
) -> midi_out::NoteIs<'a> {
    match is {
        midi_out::NoteIs::Pad(notes) => midi_out::NoteIs::Pad(shifted(notes, shift, buf)),
        is => is,
    }
}

/// A pad's notes moved by its track's live transpose (S4), into `buf`:
/// one that falls off the keyboard is left out. As written when the shift
/// moves nothing.
fn shifted<'a>(notes: &'a [u8], shift: &Shift, buf: &'a mut [u8; MAX_PAD_NOTES]) -> &'a [u8] {
    if shift.is_none() {
        return notes;
    }
    let mut n = 0;
    for to in notes.iter().filter_map(|&k| shift.apply(k)) {
        if n == buf.len() {
            break;
        }
        buf[n] = to;
        n += 1;
    }
    &buf[..n]
}

/// The length of the pattern playing, in whole bars, for the looper's
/// Length: Pattern: the longest clip playing on a notes track (a Loop
/// track's cells are loops, not patterns), rounded up. None when nothing
/// plays.
fn pattern_bars(time: &time::TimeHost) -> Option<u32> {
    let bar = time.info().meter.bar_ticks().max(1) as f64;
    let tracks = &time.project().tracks;
    (0..tracks.len())
        .filter(|&t| tracks[t].plays_notes())
        .filter_map(|t| time.now_playing(t))
        .filter(|p| p.song_len > 0.0)
        .map(|p| (p.song_len / bar - 1e-6).ceil().max(1.0) as u32)
        .max()
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
        looping::LoopAction::Command(c) => match looper.refuse(c) {
            // The source can't be had: a take would be silence.
            Some(why) => {
                out.note(&format!("looper: {why}"));
                vec![app::Effect::Tell(why)]
            }
            None => {
                out.note(&looper.command(c));
                Vec::new()
            }
        },
        looping::LoopAction::Steps(n) => {
            out.note(&looper.steps(n));
            Vec::new()
        }
        looping::LoopAction::Capture => vec![app::Effect::Tell(looper.capture())],
        // Smart loop end's chip, the loop point view and the hint line's
        // question.
        looping::LoopAction::Snap(s) => {
            out.note(&looper.snap(s));
            Vec::new()
        }
        looping::LoopAction::Side(side, keep) => {
            out.note(&looper.side(side, keep));
            Vec::new()
        }
        looping::LoopAction::Ab(on) => {
            let line = looper.set_ab(on);
            out.note(&line);
            vec![app::Effect::Tell(line)]
        }
        looping::LoopAction::MakeSong => {
            out.note(&looper.make_song());
            Vec::new()
        }
        looping::LoopAction::Answer(anyway) => {
            // A first pass waiting on its Length is asked before a snap.
            let line = match (looper.length_asking().is_some(), anyway) {
                (true, true) => looper.use_reading(),
                (true, false) => looper.discard_waiting(),
                (false, true) => looper.snap_anyway(),
                (false, false) => looper.snap_not_now(),
            };
            line.map(|l| {
                out.note(&l);
                app::Effect::Tell(l)
            })
            .into_iter()
            .collect()
        }
        looping::LoopAction::PointView(open) => {
            looper.set_point_open(open);
            Vec::new()
        }
        looping::LoopAction::Choose(side) => {
            let line = looper.choose(side);
            out.note(&line);
            vec![app::Effect::Tell(line)]
        }
        looping::LoopAction::CaptureSpan(end, bars) => {
            out.note(&looper.set_capture_span(end, bars));
            Vec::new()
        }
        looping::LoopAction::Source(s) => {
            // A take keeps its source: the pick is the next take's, said.
            let held = looper.set_source(s);
            let what = match s {
                looping::LoopSource::Mix(mpfree_fx::StripId::Master) => {
                    "the master: all you hear but the loops".to_string()
                }
                looping::LoopSource::Mix(m) => {
                    format!("{}: what it sends on, after its fader", mixing::name(m))
                }
                s => s.name().to_lowercase(),
            };
            out.note(&format!("looper: records from {what}"));
            match held {
                Some((line, true)) => {
                    out.note(&line);
                    vec![app::Effect::Tell(looper_says(&line))]
                }
                Some((line, false)) => {
                    out.note(&line);
                    Vec::new()
                }
                None => Vec::new(),
            }
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
        looping::LoopAction::SelectForCell(n) => {
            out.note(&looper.select_for_cell(n));
            Vec::new()
        }
        looping::LoopAction::ToggleMute(n) => {
            out.note(&looper.toggle_mute(n));
            Vec::new()
        }
        looping::LoopAction::Mute(n, on) => {
            out.note(&looper.set_mute(n, on));
            Vec::new()
        }
        looping::LoopAction::ToggleSolo(n) => {
            out.note(&looper.toggle_solo(n));
            Vec::new()
        }
        looping::LoopAction::Level(n, level) => {
            out.note(&looper.set_level(n, level));
            Vec::new()
        }
        looping::LoopAction::RecordLoops(on) => {
            looper.resample = on;
            out.note(&format!(
                "looper: records from {}",
                if on {
                    "the loops: a new loop resamples them"
                } else {
                    "the input"
                }
            ));
            Vec::new()
        }
        looping::LoopAction::Listen(what) => vec![app::Effect::Tell(looper.listen(what))],
        looping::LoopAction::StopWaiting => {
            if let Some(line) = looper.stop_waiting() {
                out.note(&line);
            }
            Vec::new()
        }
        looping::LoopAction::StopFree => {
            if let Some(line) = looper.stop_free() {
                out.note(&line);
            }
            Vec::new()
        }
        // A dragged layer says what it did (or why not) in place of the
        // hint.
        looping::LoopAction::CopyLayer { from, id, to, keep } => {
            vec![app::Effect::Tell(looper.copy_layer(from, id, to, keep))]
        }
    }
}

/// A Fit to tempo ended ([`audiotracks::TrackHost::tend_clip_fit`]): a
/// fitted clip goes in its cell as one UNDO step, or its copies are
/// forgotten if the cell changed meanwhile. The hint line.
fn fit_ended(
    time: &mut time::TimeHost,
    tracks: &mut audiotracks::TrackHost,
    done: audiotracks::ClipFitDone,
) -> String {
    match done {
        audiotracks::ClipFitDone::Fitted {
            track,
            slot,
            id,
            name,
            audio,
            bpm,
            copies,
        } => match time.fit_clip(track, slot, id, audio) {
            Ok(_) => format!(
                "{name}: fitted to {} · UNDO goes back",
                audiotracks::bpm_words(bpm)
            ),
            Err(e) => {
                tracks.forget_copies(&copies);
                format!("Fit to tempo: {e}")
            }
        },
        audiotracks::ClipFitDone::Failed { name } => {
            format!("Fit to tempo: {name} couldn't be fitted; nothing changed")
        }
    }
}

/// A sound Put in a cell came in ([`audiotracks::TrackHost::tend_cell_sound`]):
/// its clip goes in its track's first empty cell, as an edit UNDO takes
/// away, or its take is forgotten if the track has no room now. The hint
/// line.
fn cell_sound_in(
    time: &mut time::TimeHost,
    tracks: &mut audiotracks::TrackHost,
    done: Result<(usize, mpfree_seq::Clip), String>,
) -> String {
    let (track, clip) = match done {
        Ok(d) => d,
        Err(e) => return format!("Put in a cell: {e}"),
    };
    let name = clip.name.clone();
    let rec = clip
        .audio
        .as_ref()
        .and_then(|a| a.regions.first())
        .map(|r| r.rec);
    match time.put_audio_clip(track, clip) {
        Ok(row) => {
            let who = time
                .project()
                .tracks
                .get(track)
                .map_or("", |t| t.name.as_str());
            format!("{name} into {who}, row {} · UNDO takes it out", row + 1)
        }
        Err(e) => {
            if let Some(rec) = rec {
                tracks.forget_recording(rec);
            }
            format!("Put in a cell: {e}")
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
    // While song mode plays the song's tempo points, the tempo turned
    // sets the point in effect at the playhead (Jay's card, 18 Nov) and
    // the project's tempo stays; otherwise it is the project's.
    let a = match a {
        TimeAction::Transport(Command::SetTempo(bpm)) if time.song_tempo_plays() => {
            app.tempo_by_hand = true;
            TimeAction::SongTempo(TempoEdit::AtPlayhead(bpm))
        }
        TimeAction::Transport(Command::SetTempo(bpm)) => return app.set_tempo_by_hand(bpm),
        // A track picked while REC's arm is pinned is selected alone: the
        // Sequencer edits it and the pads follow it, and REC stays where
        // it was pinned (refactor C1). Only a tap on Arm or a row of REC's
        // list moves a pinned arm.
        TimeAction::Pick(t) if app.rec.pinned => TimeAction::Select(t),
        a => a,
    };
    // An audio track's mute and solo are the track's (phase 1a), as on
    // any track.
    let a = match a {
        TimeAction::Audio(AudioAction::Mute { track, on })
        | TimeAction::Audio(AudioAction::Solo { track, on }) => {
            let solo = matches!(a, TimeAction::Audio(AudioAction::Solo { .. }));
            let Some(t) = tracks.place(track) else {
                out.note(&format!(
                    "audio tracks: there is no audio track {}",
                    track + 1
                ));
                return Vec::new();
            };
            let now = time
                .project()
                .tracks
                .get(t)
                .map(|tr| if solo { tr.solo } else { tr.mute });
            if now != Some(on) {
                match time.mute_or_solo(t, false, solo) {
                    Ok(line) => out.note(&line),
                    Err(e) => out.note(&format!("audio tracks: {e}")),
                }
            }
            tracks.sync(time.project(), time.silenced(), true);
            return Vec::new();
        }
        a => a,
    };
    // The panel's level and pan, and Q1 and Q2 while it is open, are the
    // track's mixer strip's (phase 1b): the same fader and pan as on the
    // Mix tab, moved from either place.
    if let TimeAction::Audio(AudioAction::Turn {
        track,
        knob: knob @ (0 | 1),
        ticks,
        fine,
        ..
    }) = a
    {
        let Some(p) = tracks.place(track) else {
            out.note(&format!(
                "audio tracks: there is no audio track {}",
                track + 1
            ));
            return Vec::new();
        };
        let strip = mpfree_fx::StripId::Track(mix.track_channel(p));
        let (id, value) = if knob == 0 {
            let step = if fine { 0.1 } else { 0.5 };
            // From silent, the first step is at -60 dB, as the panel's
            // level always started.
            let db = mix.wanted(strip, mpfree_fx::ids::mixer::VOLUME).max(-60.0);
            let db = db + step * ticks as f32;
            let db = if db <= -60.0 {
                mpfree_fx::SILENT_DB
            } else {
                db.min(6.0)
            };
            (mpfree_fx::ids::mixer::VOLUME, db)
        } else {
            let step = if fine { 1.0 } else { 2.0 };
            let v = (mix.wanted(strip, mpfree_fx::ids::mixer::PAN) + step * ticks as f32)
                .clamp(-100.0, 100.0);
            // Snap to the middle near it, so centre is easy to find.
            (
                mpfree_fx::ids::mixer::PAN,
                if v.abs() < 0.5 { 0.0 } else { v },
            )
        };
        mix.action(&mpfree_mixer_ui::MixAction::Set { strip, id, value });
        return Vec::new();
    }
    // The panel's "Its strip in Mix ›": the Mix tab's tracks, with the
    // track's own strip picked (the mixer pages to it).
    if let TimeAction::Audio(AudioAction::Strip(track)) = a {
        let Some(p) = tracks.place(track) else {
            out.note(&format!(
                "audio tracks: there is no audio track {}",
                track + 1
            ));
            return Vec::new();
        };
        let strip = mpfree_fx::StripId::Track(mix.track_channel(p));
        mix.action(&mpfree_mixer_ui::MixAction::Select(strip));
        let mut fx = app.open_tab(app::View::Mixer);
        fx.push(app::Effect::Mixer(Some(mpfree_mixer_ui::MixView::Tracks)));
        return fx;
    }
    if matches!(a, TimeAction::AddAudioTrack) && tracks.free_slot().is_none() {
        out.note(&format!(
            "audio tracks: {} audio tracks is the most this MPFree plays",
            mpfree_audiotracks::MAX_TRACKS
        ));
        return Vec::new();
    }
    // "+ Track"'s Audio track: armed once it is there (layout spec 4c).
    if matches!(a, TimeAction::AddAudioTrack) {
        tracks.arm_next();
    }
    // The audio lanes' touches go to the audio tracks; a pick stays with
    // the time engine, which keeps the selection.
    // To a cell (audio tracks phase 2): the picked region as an audio
    // clip in its track's first empty cell; the song doesn't change.
    // Fit to tempo on an audio clip's card (audio tracks phase 2): fitted
    // copies of its recordings start; the clip plays them once they are
    // in (the main loop, `fit_ended`).
    if let TimeAction::FitClip { track, slot } = a {
        let bpm = time.info().bpm;
        let said = match time.project().clip(track, slot) {
            Some(c) => tracks.fit_clip(track, slot, c, bpm),
            None => Err(format!("cell {} of track {} is empty", slot + 1, track + 1)),
        };
        let line = said.unwrap_or_else(|e| format!("Fit to tempo: {e}"));
        out.note(&format!("audio tracks: {line}"));
        out.show(gui::ToUi::Tell(line));
        return Vec::new();
    }
    if let TimeAction::Audio(AudioAction::ToCell(r)) = a {
        let said = tracks
            .region_clip(r, time.tempo_map(), time.info().meter)
            .and_then(|(track, clip, place)| {
                let row = time.put_audio_clip(track, clip)?;
                Ok(format!("{place}: in row {} · UNDO takes it out", row + 1))
            });
        let line = said.unwrap_or_else(|e| format!("To a cell: {e}"));
        out.note(&format!("audio tracks: {line}"));
        out.show(gui::ToUi::Tell(line));
        return Vec::new();
    }
    if let TimeAction::Audio(aa) = a {
        // The audio tracks' own Undo and Redo at a song edit's step the
        // song no longer reaches: the step is forgotten (Arrange 3.4).
        if matches!(aa, AudioAction::Undo | AudioAction::Redo) {
            let undo = matches!(aa, AudioAction::Undo);
            if let Some(line) = tracks.forget_song_step(undo, |n| time.song_reaches(n, undo)) {
                out.note(&format!("audio tracks: {line}"));
                return Vec::new();
            }
            // At a song edit's step the song still reaches, UNDO (or SHIFT
            // + UNDO) on Arrange takes the song edit back with its regions
            // (layout spec 4c: UNDO follows the region edits down to it).
            let tr = &tracks.host.tracks;
            let song = if undo {
                tr.undo_is_song()
            } else {
                tr.redo_is_song()
            };
            if song.is_some() {
                let a = if undo {
                    TimeAction::Undo
                } else {
                    TimeAction::Redo
                };
                return time_action(out, time, tracks, looper, app, mix, a);
            }
        }
        // An audio track's input list (phase 4, board 4): what the mixer
        // offers the track, for the screen.
        if let AudioAction::Inputs(t) = aa {
            match input_choices(time, tracks, looper, mix, t) {
                Ok(c) => out.show(gui::ToUi::InputList(Box::new(c))),
                Err(e) => out.show(gui::ToUi::Tell(e)),
            }
            return Vec::new();
        }
        if !matches!(aa, AudioAction::Pick(_)) {
            // What the panel's Mix offers comes from the mixer (step 9).
            if matches!(aa, AudioAction::Mix { .. } | AudioAction::Panel(_)) {
                tracks.offer(mix.inside_sources());
            }
            match tracks.action(aa, &time.info(), looper.line.is_open()) {
                Ok(line) => {
                    // An audio track armed: the Clip tab shows its clip.
                    if let AudioAction::Arm { track, on } = aa {
                        if let Some(row) = tracks.place(track) {
                            time.focus_audio(row, on);
                        }
                    }
                    if !line.is_empty() {
                        out.note(&line);
                    }
                }
                Err(e) => out.note(&format!("audio tracks: {e}")),
            }
            // An erase leaves the picked region's place empty.
            if matches!(aa, AudioAction::Erase(_)) {
                let _ = time.act(TimeAction::Audio(AudioAction::Pick(None)));
            }
            return Vec::new();
        }
    }
    // ERASE + a clip, COPY + a clip or a slot, COPY + a note or empty roll
    // in the note editor, a clip settings card change, a drop on the Clips
    // grid or the Song view, a block deleted, UNDO and REDO: said in place
    // of the hint.
    let undo = matches!(a, TimeAction::Undo | TimeAction::Redo);
    let erase = matches!(
        a,
        TimeAction::Edit(
            mpfree_time_ui::editor::EditAction::Copy { .. }
                | mpfree_time_ui::editor::EditAction::Paste { .. }
        ) | TimeAction::DeleteClip { .. }
            | TimeAction::ClipSettings { .. }
            | TimeAction::ClipLength { .. }
            | TimeAction::AudioClip { .. }
            | TimeAction::Mutate { .. }
            | TimeAction::Restore { .. }
            | TimeAction::ConvertPreview { .. }
            | TimeAction::Convert { .. }
            | TimeAction::ConvertCancel
            | TimeAction::Continue(
                ContinueAction::Ask(_)
                    | ContinueAction::Step { .. }
                    | ContinueAction::Keep
                    | ContinueAction::Back
                    | ContinueAction::Open { .. }
            )
            | TimeAction::Add(_)
            | TimeAction::Chords(_)
            | TimeAction::CopyClip { .. }
            | TimeAction::PasteClip { .. }
            | TimeAction::DropClip { .. }
            | TimeAction::DropRow { .. }
            | TimeAction::CaptureScene
            | TimeAction::InsertRow(_)
            | TimeAction::DeleteRow(_)
            | TimeAction::MovePlacement { .. }
            | TimeAction::StretchPlacement { .. }
            | TimeAction::MoveSection { .. }
            | TimeAction::SongBars(_)
            | TimeAction::SongMarker(_)
            | TimeAction::DeletePlacement(_)
            | TimeAction::SplitPlacement(_)
            | TimeAction::DuplicatePlacement(_)
            | TimeAction::TrimPlacement { .. }
            | TimeAction::SlidePlacement { .. }
            | TimeAction::MovePlacements { .. }
            | TimeAction::DuplicatePlacements(_)
            | TimeAction::DeletePlacements(_)
            | TimeAction::SplitPlacements(_)
            | TimeAction::Perform(_)
    ) || matches!(a, TimeAction::SongTempo(e) if !matches!(e, TempoEdit::Pick(_)))
        // A linked song list's step edits are the song's (Arrange 4b.2).
        || (time.project().arrangement.list.linked && time::edits_steps(&a))
        // Link's Song from the list puts room at the song's start (4b.5).
        || matches!(a, TimeAction::ListLink(_));
    // PLAY or PLAY START while the transport follows another device's
    // Start and Stop and it is stopped: its Start plays, and the screen
    // says so.
    let waiting = match a {
        TimeAction::Transport(Command::Play | Command::PlayStart)
            if !time.info().state.rolling() =>
        {
            time.waiting_for().map(str::to_string)
        }
        _ => None,
    };
    let tracks_change = matches!(
        a,
        TimeAction::AddAudioTrack | TimeAction::Undo | TimeAction::Redo
    );
    let convert = matches!(
        a,
        TimeAction::ScenesToSong | TimeAction::ListToSong | TimeAction::SongToScenes
    );
    let say = match waiting {
        Some(leader) => time::Say::Waiting(leader),
        None if erase => time::Say::Tell,
        None if convert => time::Say::Convert,
        None if undo => time::Say::Undo,
        None => time::Say::Note {
            ok: "time: ",
            err: "time: ",
        },
    };
    let r = match a {
        // The scene morph runs on the mixer's mod engine.
        TimeAction::MorphRows { .. }
        | TimeAction::Morph(_)
        | TimeAction::SnapBack
        | TimeAction::KeepMorph
        | TimeAction::SaveScene(_) => Some(time.morph(a, &mut mix.modu, &mix.mixer)),
        // A song time edit (a linked list's step edits too, Arrange
        // 4b.2), UNDO and REDO with audio regions (or a song edit's step
        // on their history): the regions move with the song in the same
        // period, or nothing moves (Arrange 3.4). Apart too, the control
        // half borrowed back for it. Song → scenes asks them for their
        // regions, to turn into audio clips (phase 2, step 2d).
        TimeAction::MoveSection { .. }
        | TimeAction::SongBars(_)
        | TimeAction::ListTimes { .. }
        | TimeAction::ListScene { .. }
        | TimeAction::ListAdd
        | TimeAction::ListRemove(_)
        | TimeAction::ListMove { .. }
        | TimeAction::ListBack(_)
        | TimeAction::SongToScenes
        | TimeAction::Undo
        | TimeAction::Redo
            if tracks.host.tracks.has_regions() || tracks.host.tracks.has_song_steps() =>
        {
            Some(time.act_with_audio(a, tracks))
        }
        // Apart (--time-thread), the action goes to the editing side's
        // thread and what it says comes back later ([`time_said`]).
        _ => time.act_then(a, say.clone()),
    };
    // A track added, or taken away or brought back: the audio tracks'
    // table follows at once (apart, within a few periods).
    if tracks_change {
        tracks.sync(time.project(), time.silenced(), true);
    }
    // The note editor opened or closed: the Sequencer's Notes view
    // follows (apart, again when the answer comes).
    let mut fx = app.set_notes_open(time.editing());
    if let Some(r) = r {
        fx.extend(time_said(out, say, r));
    }
    fx
}

/// What the time engine answered, said as the caller asked (`say`): the
/// same words whether the answer came at once (together) or later from
/// the editing side's own thread (apart, [`time::TimeHost::take_said`]).
fn time_said(out: &mut Out, say: time::Say, r: Result<String, String>) -> Vec<app::Effect> {
    match (say, r) {
        (time::Say::Waiting(leader), Ok(line)) => {
            out.note(&format!("time: {line}"));
            vec![app::Effect::Tell(format!(
                "Waiting for clock: PLAY starts when {leader} starts"
            ))]
        }
        (time::Say::Waiting(_), Err(e)) => {
            out.note(&format!("time: {e}"));
            Vec::new()
        }
        (time::Say::Tell, Ok(line) | Err(line)) => vec![app::Effect::Tell(line)],
        (time::Say::Undo, Ok(line) | Err(line)) => vec![app::Effect::Tell(undo_said(&line))],
        (time::Say::Note { ok, .. }, Ok(line)) => {
            if !line.is_empty() {
                out.note(&format!("{ok}{line}"));
            }
            Vec::new()
        }
        (time::Say::Note { err, .. }, Err(e)) => {
            out.note(&format!("{err}{e}"));
            Vec::new()
        }
        (time::Say::NoteOrTell { .. }, Ok(line)) => {
            if !line.is_empty() {
                out.note(&line);
            }
            Vec::new()
        }
        (time::Say::NoteOrTell { err }, Err(e)) => vec![app::Effect::Tell(format!("{err}{e}"))],
        // An exact conversion opens nothing: its line shows in place of the
        // hint. One with differences opens the report card (the time
        // engine's `TimeState::report`), whose own hint shows.
        (time::Say::Convert, Ok(line)) => {
            out.note(&format!("time: {line}"));
            if line.ends_with(time::EXACT) {
                vec![app::Effect::Tell(line)]
            } else {
                Vec::new()
            }
        }
        (time::Say::Convert, Err(e)) => {
            out.note(&format!("time: {e}"));
            vec![app::Effect::Tell(e)]
        }
    }
}

/// The time engine's editing side onto a thread of its own
/// (`--time-thread`), or left on the audio thread with why, said.
fn time_apart(out: &mut Out, time: time::TimeHost) -> time::TimeHost {
    let (time, why) = time.into_apart();
    match why {
        None => out.note("time: the editing side runs on a thread of its own (--time-thread)"),
        Some(why) => out.note(&format!("time: {why}; it runs on the audio thread")),
    }
    time
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

/// Your XY layouts with what an action on them reads and changes: the
/// screen's `FromUi::Xy`, and Learn's XY Hold and layout buttons (the
/// smoke session's too).
struct XyHere<'a> {
    song: &'a mut xy_song::XySong,
    mix: &'a mut mixing::MixHost,
    synth: &'a synth::Synth,
    app: &'a app::App,
    /// The project's instruments, by which a learned pad's setting or synth
    /// knob names its instrument (sampler step 16).
    listed: &'a instrument_list::InstrumentList,
    pads: PadNow<'a>,
    time: &'a time::TimeHost,
}

impl XyHere<'_> {
    /// An action on your layouts: the mixer plays what it asks; the lines
    /// to tell.
    fn act(&mut self, act: xy_song::XyAction) -> Vec<String> {
        let track = u8::try_from(self.app.track()).unwrap_or(u8::MAX);
        let (mix, synth, app, listed, pads, time) = (
            &*self.mix,
            self.synth,
            self.app,
            self.listed,
            self.pads,
            self.time,
        );
        let look = |a| xy_seen(mix, synth, app, listed, pads, time, a);
        let mut told = Vec::new();
        let done = self.song.act(
            act,
            track,
            |pick, name| mix.xy_ready(pick, name),
            &look,
            &mut |l| told.push(l),
        );
        match done {
            Ok(d) => self.mix.xy_done(d),
            Err(e) => told.push(e),
        }
        told
    }

    /// Learn's XY buttons pressed (the Play card): Hold switches on a
    /// layout of yours (on a ready-made one the line says why it does
    /// nothing), and the layout buttons step through the song's layouts
    /// of yours, or the ready-made ones while there are none: the pick
    /// for Prefs to keep, which the screen and the mixer then follow.
    fn pressed(&mut self, p: remote::XyPresses) -> (Vec<String>, Option<Option<usize>>) {
        let mut lines = Vec::new();
        let mut pick = None;
        if p.step != 0 {
            match self.song.stepped(self.app.settings.xy_effect, p.step) {
                xy_song::Stepped::Yours(i) => lines.extend(self.act(xy_song::XyAction::Play(i))),
                xy_song::Stepped::Ready(e) => {
                    let name = play_page::XY_CHOICES[e.map_or(0, |k| k + 1)];
                    lines.push(format!("XY pad: {name}"));
                    pick = Some(e);
                }
            }
        }
        if p.hold {
            if self.song.playing.is_some() {
                let on = !self.mix.xy_held();
                lines.extend(self.act(xy_song::XyAction::Hold(on)));
            } else {
                lines.push(xy_song::NO_HOLD.to_string());
            }
        }
        (lines, pick)
    }

    /// The screen's view of your layouts now.
    fn view(&mut self) -> gui::ToUi {
        xy_view(
            self.song,
            self.mix,
            self.synth,
            self.app,
            self.listed,
            self.pads,
            self.time,
        )
    }
}

/// Your XY layouts as the screen shows them ([`gui::ToUi::Xy`]), each
/// setting named afresh.
/// The XY pad's Set moves this period go where they record
/// (`xy_rec.rs`): into the clip REC records and the move log
/// (`TimeHost::gesture`), into the song's lanes while Rec song records
/// (`TimeHost::song_gesture`), or nowhere. Nothing allocates.
fn xy_record(
    rec: &mut xy_rec::XyRec,
    mix: &mut mixing::MixHost,
    synth: &synth::Synth,
    app: &app::App,
    listed: &instrument_list::InstrumentList,
    pads: PadNow<'_>,
    time: &mut time::TimeHost,
) {
    use xy_rec::Goes;
    let mut moves = [None; xy_play::REC_MOVES];
    let mut n = 0;
    mix.take_xy_moves(|m| {
        if n < moves.len() {
            moves[n] = Some(m);
            n += 1;
        }
    });
    if n == 0 {
        return;
    }
    let armed = time.armed().and_then(|t| u8::try_from(t).ok());
    let now = xy_rec::Now {
        armed,
        rec: time.info().state == mpfree_transport::State::Recording,
        song: time.rec_song(),
        rolling: app.rolling(),
    };
    let ins = &mix.instruments;
    let plays = |t: u8| {
        armed.is_some_and(|a| {
            xy_rec::plays_synth(a, t, |t| ins.owner(t), |t| app.plays_pads_synth(t))
        })
    };
    let play_now = desk::play_now(time, mix);
    for m in moves[..n].iter().flatten() {
        let map = |a| {
            learn_describe(mix, synth, app, listed, pads, play_now, a)
                .map(|(info, _)| mpfree_seq::mpfree_moves::Map::of(&info))
        };
        rec.feed(*m, &now, plays, map, |goes, input| match goes {
            Goes::Clip | Goes::Log => time.gesture(0, input),
            Goes::Song => time.song_gesture(0, input),
            Goes::Not => {}
        });
    }
}

fn xy_view(
    song: &mut xy_song::XySong,
    mix: &mixing::MixHost,
    synth: &synth::Synth,
    app: &app::App,
    listed: &instrument_list::InstrumentList,
    pads: PadNow<'_>,
    time: &time::TimeHost,
) -> gui::ToUi {
    let look = |a| xy_seen(mix, synth, app, listed, pads, time, a);
    gui::ToUi::Xy(Box::new(song.view(mix.xy_held(), &look)))
}

/// A Stay lift as the XY pad's undo history lists it: "Stay: Cutoff,
/// Send A (2 settings)".
fn xy_turn_row(
    mix: &mixing::MixHost,
    synth: &synth::Synth,
    time: &time::TimeHost,
    listed: &instrument_list::InstrumentList,
    t: &undo::Turn<xy_song::XyKey, xy_song::XyValues>,
) -> String {
    let names: Vec<String> = t
        .key
        .iter()
        .flatten()
        .map(|&a| learn_label(mix, synth, time, listed, a))
        .collect();
    let n = names.len();
    let shown: Vec<&str> = names.iter().take(3).map(String::as_str).collect();
    let more = if n > 3 { ", …" } else { "" };
    format!(
        "Stay: {}{more} ({n} setting{})",
        shown.join(", "),
        if n == 1 { "" } else { "s" }
    )
}

/// A synth's, a pad's and an arp's own settings as the XY pad's hand
/// reaches them (`xy_own.rs`), routed as a clip's lock is
/// (`SeqPads::lock_at`): a track's own instrument (or the one it plays),
/// else the synth on the pads; a kit pad's own setting on the kit its
/// track plays; a track's arp in the App ([`app::App::arp_hand`]). Stay
/// sets one as a turn of its knob would: the Synth tab's turn (the tab
/// follows, `synth_turned`), a pad's on its kit by the screen, as a
/// learned control sets it (`pads_learned`, `ToUi::PadsLearned`), and an
/// arp's as a learned control sets it (the panel's update in `arp_fx`).
struct OwnReach<'a> {
    synth: &'a mut synth::Synth,
    own: &'a mut instruments::Instruments,
    /// The kits' sounds (None: none play, as in a smoke test without them).
    player: Option<&'a mut sampler::Player>,
    kits: &'a sampler::TrackKits,
    /// The project's instruments, for a setting named by its kit or its
    /// instrument (sampler step 16: `Owner::KitPad`, `Owner::Sound`).
    listed: &'a instrument_list::InstrumentList,
    pads_learned: &'a mut Vec<(usize, u8, sampler::PadParam, f32)>,
    synth_turned: &'a mut bool,
    /// The tracks' arps (None: none here).
    app: Option<&'a mut app::App>,
    /// What an arp's Stay tells the screens, for the caller to send.
    arp_fx: Vec<app::Effect>,
}

/// What the XY pad's hand reaches for a synth's or a pad's own setting.
enum OwnAt {
    /// The synth on the pads (the shared synth).
    Synth,
    /// The instrument this holder (a track, or a free place) holds.
    Holder(usize),
    /// A kit's pad's own setting. A layer's grain setting isn't reached by
    /// the hand yet: only a pad's own `PadField`s.
    Pad(usize, usize, mpfree_sampler::engine::PadField),
    /// A track's arp (XY 2.4: its settings live in the App).
    Arp(usize),
}

impl OwnReach<'_> {
    /// Where `addr` lives now: a track's synth (`Owner::Inst`) on its own
    /// instrument while it has one, else the synth on the pads; a track's
    /// pad on the kit it plays (`Owner::Pad`); an instrument named by its
    /// number (`Owner::Sound`), or a kit's pad (`Owner::KitPad`). None:
    /// nothing there, a track that plays an instrument or the synth for a
    /// pad's setting, or a number the list hasn't got.
    fn at(&self, addr: mpfree_play::mpfree_param::ParamAddr) -> Option<OwnAt> {
        use mpfree_play::mpfree_param::Owner;
        match addr.owner {
            Owner::Inst(t) if self.own.plays(usize::from(t)) => Some(OwnAt::Holder(usize::from(t))),
            Owner::Inst(_) => Some(OwnAt::Synth),
            Owner::Pad(t, pad) => {
                let t = usize::from(t);
                if self.own.plays(t) || self.kits.plays_synth(t) {
                    return None;
                }
                let f = sampler::pad_field(addr.param)?;
                Some(OwnAt::Pad(self.kits.kit_of(t)?, usize::from(pad), f))
            }
            Owner::Sound(n) => match named(self.listed, n)? {
                Named::SharedSynth => Some(OwnAt::Synth),
                Named::Holder(h) => Some(OwnAt::Holder(h)),
                Named::Kit(_) => None,
            },
            Owner::KitPad(n, pad) => match named(self.listed, n)? {
                Named::Kit(kit) => Some(OwnAt::Pad(
                    kit,
                    usize::from(pad),
                    sampler::pad_field(addr.param)?,
                )),
                _ => None,
            },
            Owner::Arp(t) => Some(OwnAt::Arp(usize::from(t))),
            _ => None,
        }
    }
}

impl xy_own::OwnSide for OwnReach<'_> {
    fn hold(
        &mut self,
        addr: mpfree_play::mpfree_param::ParamAddr,
        value: Option<f32>,
        over: bool,
    ) -> bool {
        match self.at(addr) {
            Some(OwnAt::Holder(h)) => self.own.pad_hand(h, addr.param, value, over),
            Some(OwnAt::Synth) => self.synth.pad_hand(addr.param, value, over),
            Some(OwnAt::Pad(kit, pad, f)) => match self.player.as_mut() {
                Some(p) => p.pad_hand(kit, pad, f, value, over),
                None => false,
            },
            Some(OwnAt::Arp(t)) => self
                .app
                .as_mut()
                .is_some_and(|a| a.arp_hand(t, addr.param, value)),
            None => false,
        }
    }

    fn beneath(&self, addr: mpfree_play::mpfree_param::ParamAddr) -> Option<(f32, f32)> {
        match self.at(addr)? {
            OwnAt::Holder(h) => self.own.pad_beneath(h, addr.param),
            OwnAt::Synth => self
                .synth
                .param_info(addr.param)
                .map(|_| self.synth.pad_beneath(addr.param)),
            OwnAt::Pad(kit, pad, f) => {
                let p = self.player.as_ref()?;
                let own = p.pad_own(kit, pad, f)?;
                Some((p.pad_beneath(kit, pad, f)?, own))
            }
            OwnAt::Arp(t) => self.app.as_ref()?.arp_beneath(t, addr.param),
        }
    }

    fn keep(&mut self, addr: mpfree_play::mpfree_param::ParamAddr, value: f32) -> Option<f32> {
        match self.at(addr)? {
            OwnAt::Holder(h) => {
                let ok = self.own.param(h, addr.param, value);
                *self.synth_turned |= ok;
                ok.then_some(value)
            }
            OwnAt::Synth => {
                self.synth.param_info(addr.param)?;
                self.synth.set(addr.param, value);
                *self.synth_turned = true;
                Some(value)
            }
            OwnAt::Pad(kit, pad, f) => {
                let pad = u8::try_from(pad).unwrap_or(u8::MAX);
                self.pads_learned
                    .push((kit, pad, sampler::PadParam::Own(f), value));
                Some(value)
            }
            OwnAt::Arp(t) => {
                let (v, fx) = self.app.as_mut()?.keep_arp(t, addr.param, value)?;
                self.arp_fx.extend(fx);
                Some(v)
            }
        }
    }
}

/// A setting as the XY pad's Setup and its waiting banner name it
/// (`xy_song.rs`): its name, where it is ("Bass · Poly", "Master · Room,
/// effect 1", "Bass strip", "Bass · Arp"), its colour, its range, and
/// whether it is a synth's, a pad's or an arp's own (no Add yet). Err with
/// why it isn't there: its track removed, its effect taken out.
fn xy_seen(
    mix: &mixing::MixHost,
    synth: &synth::Synth,
    app: &app::App,
    listed: &instrument_list::InstrumentList,
    pads: PadNow<'_>,
    time: &time::TimeHost,
    addr: mpfree_play::mpfree_param::ParamAddr,
) -> Result<xy_song::Seen, String> {
    use mpfree_play::mpfree_param::Owner;
    let project = time.project();
    let track = |t: u8| {
        let n = track_name(project, t);
        if n.is_empty() {
            format!("Track {}", u16::from(t) + 1)
        } else {
            n
        }
    };
    if let Some((t, _)) = addr.owner.split() {
        if usize::from(t) >= project.tracks.len() {
            return Err(format!("Track {} was removed.", u16::from(t) + 1));
        }
    }
    let now = desk::play_now(time, mix);
    let Some((info, _)) = learn_describe(mix, synth, app, listed, pads, now, addr) else {
        return Err(match addr.owner {
            Owner::Insert(..) => "Its effect was taken out, or has no such setting now.",
            Owner::Pad(..) => "Its pad has no sound now.",
            _ => "It isn't there any more.",
        }
        .to_string());
    };
    let (name, place) = match addr.owner {
        Owner::Inst(t) => {
            let own = mix
                .instruments
                .holds(usize::from(t))
                .then(|| mix.instruments.node(usize::from(t)))
                .flatten();
            let engine = own.map_or_else(|| synth.engine_name(), |n| n.info().name);
            (info.name.to_string(), format!("{} · {engine}", track(t)))
        }
        Owner::Pad(t, pad) => (
            info.name.to_string(),
            format!(
                "{} · pad {}",
                track(t),
                mpfree_midi::profile::pad_name(usize::from(pad))
            ),
        ),
        Owner::Arp(t) => (info.name.to_string(), format!("{} · Arp", track(t))),
        Owner::Strip(s) => (
            info.name.to_string(),
            format!("{} strip", mixing::name(modmix::strip_id(s))),
        ),
        Owner::Insert(s, uid) => {
            let sid = modmix::strip_id(s);
            let (effect, slot) = mix
                .mixer
                .chain(sid)
                .and_then(|c| {
                    let i = c.slot_of(uid)?;
                    Some((c.node(i)?.info().name, i))
                })
                .unwrap_or(("?", 0));
            (
                info.name.to_string(),
                format!("{} · {effect}, effect {}", mixing::name(sid), slot + 1),
            )
        }
        _ => {
            // "CC 74 (Track 3)", "Macro 2 · Grit": the name, and where in
            // the brackets.
            let label = learn_label(mix, synth, time, listed, addr);
            match label.rsplit_once(" (") {
                Some((n, p)) => (n.to_string(), p.trim_end_matches(')').to_string()),
                None => (label, String::new()),
            }
        }
    };
    Ok(xy_song::Seen {
        info,
        name,
        place,
        dot: xy_song::dot_of(addr),
        own: xy_own::is_own(addr),
    })
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

/// A block and a region never overlap on a lane (the audio tracks
/// rethink's phase 2, step 2d), each period: Rec song's new blocks of
/// audio clips replace the regions under them (Joshi, 23 Nov), as steps
/// of its song edit; and sound put on an audio track's lane since the last
/// period (a take, a swipe on a take lane, a region moved, a sound brought
/// in) cuts the song's blocks under it, as a song edit its undo step is
/// then tied to. Either way either UNDO takes back both.
fn cut_blocks_under_takes(
    out: &mut Out,
    time: &mut time::TimeHost,
    tracks: &mut audiotracks::TrackHost,
) {
    if let Some((song, spans)) = time.take_song_placed() {
        let (done, left) = tracks.replace_under(song, &spans);
        if done > 0 && !time.tie_audio(song) {
            out.note(
                "audio tracks: Rec song's blocks replaced recorded regions, but its song edit \
                 is gone, so UNDO can't bring them back",
            );
        }
        if left > 0 {
            out.note(&format!(
                "audio tracks: the queue was full: {left} of Rec song's blocks still sit over \
                 recorded regions"
            ));
        }
    }
    while let Some((track, span, step)) = tracks.take_took() {
        match time.audio_took(track, span) {
            Ok(Some((song, line))) => {
                out.note(&line);
                if let Err(e) = tracks.tie_step(step, song) {
                    out.note(&format!("audio tracks: {e}"));
                }
            }
            Ok(None) => {}
            Err(e) => out.note(&e),
        }
    }
}

/// Everything the time pages show, with the audio tracks' lanes.
fn time_state(time: &time::TimeHost, tracks: &audiotracks::TrackHost) -> mpfree_time_ui::TimeState {
    let mut st = time.state();
    tracks.fill(&mut st, time.tempo_map());
    tracks.fill_cells(&mut st, time.project());
    tracks.fill_blocks(&mut st, time.project(), time.tempo_map());
    st
}

/// The synth's part of the status line.
fn synth_status(synth: &synth::Synth, time: &RenderTime) -> String {
    let c = synth.counters();
    let mut line = format!(
        "synth: {} ({}), {}, {} notes, at most {} voices ({} set), {} notes dropped, \
         {} new notes or locks left out of moments too busy, {} endings lost, {} resets",
        synth.engine().name(),
        synth.preset_name(),
        time.describe(),
        c.notes,
        c.peak_voices,
        synth.polyphony(),
        c.dropped,
        c.busy,
        c.ends_lost,
        c.resets
    );
    // Knob owners (step 4b): locks another track or the hand kept waiting,
    // and any a full table lost.
    if let Some(locks) = synth.locks_line() {
        line.push_str(", ");
        line.push_str(&locks);
    }
    line
}

/// The log line for the pads moving to the synth, by an octave or to
/// another layout: "pads: Poly synth, C3 to D#4 (pad 1 = C3; …)", with the
/// scale and layout when a scale is on.
/// The notes each of track `track`'s 16 pads plays, as a clip of notes on
/// the track reads them (step 5c): what the pad plays on the track's own
/// synth (or the one it plays) or on the shared synth, a chord's root
/// last, the note a kit played as notes plays on the pad (bank A's;
/// sampler step 15c, part 3), or a kit pad's note in bank A. A pad that
/// plays nothing (its notes outside 0 to 127) has none, and the grid shows
/// its row on its own number. Never allocates.
fn pad_rows(
    own: &instruments::Instruments,
    synth: &synth::Synth,
    kits: &sampler::TrackKits,
    kit_notes: &sampler::KitNotes,
    track: usize,
) -> time::PadRows {
    let mut rows = time::PadRows::PADS;
    for p in 0..16 {
        if own.plays(track) {
            rows.set(p, own.pad_notes(track, p));
        } else if kits.plays_synth(track) {
            rows.set(p, synth.notes(p));
        } else if kit_notes.of_track(kits, track).is_some() {
            let n = kit_notes.note(kits, track, p);
            rows.set(p, n.as_slice());
        } else if let Some(n) = mpfree_midi::profile::kit_note(p) {
            rows.set(p, &[n]);
        }
    }
    rows
}

/// The tracks whose takes hold notes, a bit each (instrument redesign
/// step 5c): one whose own instrument, or the one it plays of another
/// track's, plays notes, not the Drums engine's sounds; or one that plays
/// the shared synth while it plays notes (step sequencer refactor E4); or
/// one that plays a kit played as notes (sampler step 15c, part 3).
/// Never allocates.
fn notes_tracks(
    own: &instruments::Instruments,
    kits: &sampler::TrackKits,
    synth: &synth::Synth,
    kit_notes: &sampler::KitNotes,
    tracks: usize,
) -> u128 {
    let synth_notes = synth.plays_notes();
    (0..tracks.min(128))
        .filter(|&t| !own.plays(t))
        .filter(|&t| (synth_notes && kits.plays_synth(t)) || kit_notes.of_track(kits, t).is_some())
        .fold(own.note_tracks(), |bits, t| bits | 1 << t)
}

/// Whether track `track`'s arp works in notes (step 5d): its own synth
/// (or the one it plays) plays notes, not the Drums engine's sounds, or it
/// plays the shared synth, whose pads press notes, or a kit played as
/// notes (sampler step 15c, part 3); not while its notes go to MIDI out,
/// which plays the pads as its profile says. Never allocates.
fn arp_on_notes(
    own: &instruments::Instruments,
    kits: &sampler::TrackKits,
    kit_notes: &sampler::KitNotes,
    midi: &midi_out::MidiOut,
    track: usize,
) -> bool {
    if midi.takes(track as u16) {
        return false;
    }
    if own.plays(track) {
        own.plays_notes(track)
    } else {
        kits.plays_synth(track) || kit_notes.of_track(kits, track).is_some()
    }
}

/// Which tracks' takes hold notes, each track's pad rows, and whether its
/// arp works in notes, from what the tracks play now: a clip of notes shows
/// each pad's note on the step grid's pad rows (step 5c), and a track whose
/// instrument plays notes has its arp work in notes, so the live pads, a
/// clip of notes and an old clip of pad hits meet in it as notes (step
/// 5d); a kit played as notes too (sampler step 15c, part 3). Each period;
/// never allocates.
fn sync_notes(
    time: &mut time::TimeHost,
    app: &mut app::App,
    own: &instruments::Instruments,
    kits: &sampler::TrackKits,
    kit_notes: &sampler::KitNotes,
    synth: &synth::Synth,
    midi: &midi_out::MidiOut,
) {
    let n = time.track_count();
    time.set_notes_tracks(notes_tracks(own, kits, synth, kit_notes, n));
    for t in 0..n.min(mpfree_seq::MAX_TRACKS) {
        let rows = pad_rows(own, synth, kits, kit_notes, t);
        time.set_pad_rows(t, rows);
        app.arps
            .set_on_notes(t, arp_on_notes(own, kits, kit_notes, midi, t));
        app.arps.set_pads(t, &rows);
    }
}

/// What the pads press into the selected track's arp: its own synth's pad
/// map while that plays notes (step 5d), or its kit's bank's notes while
/// that kit is played as notes (sampler step 15c, part 3). None (they
/// press pads, or the shared synth's layout) otherwise, and while the pads
/// play something of their own (`pads_own`, or a synth chosen for them).
fn press_layout(
    app: &app::App,
    own: &instruments::Instruments,
    kits: &sampler::TrackKits,
    kit_notes: &sampler::KitNotes,
    midi: &midi_out::MidiOut,
    pads_own: bool,
) -> Option<[mpfree_arp::scale::pads::Pad; 16]> {
    let sel = app.track();
    if pads_own || app.pads_chose_synth() || !arp_on_notes(own, kits, kit_notes, midi, sel) {
        return None;
    }
    if own.plays(sel) {
        own.pad_map(sel)
    } else {
        kit_press_layout(kits, kit_notes, sel, app.kit_page())
    }
}

/// What the pads press into the arp of track `track` playing a kit played
/// as notes (sampler step 15c, part 3): bank `bank`'s 16 notes, as the kit
/// plays them, so the pads and the keys play the track as notes, like a
/// synth's. None for any other track. Never allocates.
fn kit_press_layout(
    kits: &sampler::TrackKits,
    kit_notes: &sampler::KitNotes,
    track: usize,
    bank: usize,
) -> Option<[mpfree_arp::scale::pads::Pad; 16]> {
    kit_notes.of_track(kits, track)?;
    Some(std::array::from_fn(|s| {
        let n = kit_notes.note(kits, track, bank * 16 + s);
        n.map_or_else(mpfree_arp::scale::pads::Pad::default, |n| {
            mpfree_arp::scale::pads::Pad::single(n.into())
        })
    }))
}

/// The hint the Synth tab shows while the armed track (`track`, from 0)
/// plays a kit: opening the tab leaves what the track plays alone.
fn synth_tab_hint(track: usize) -> String {
    format!(
        "Track {}'s instrument is a kit: tap Play on the pads under the synth's name to hear the synth, or pick Synth in the list of kits to put it on the track",
        track + 1
    )
}

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
            out.show(gui::ToUi::Problem(mpfree_status::Report::happened(
                &mpfree_status::kinds::SAVE_FAILED,
                1,
            )));
        }
    }
}

fn player_session(
    o: &Options,
    out: &mut Out,
    timed: bool,
    from_ui: &Receiver<gui::FromUi>,
    xy: &xy_slot::XySlot,
) -> Result<(), String> {
    // The store (--data: the stick's partition 4) opens first, before the
    // first start writes anything there: it marks a new, empty data folder as
    // MPFree's (FORMAT), and uses no folder that is neither empty nor marked.
    // (The third creator stick's first start wrote its backup and library
    // before the store opened, 16 Nov, and the store then refused the
    // folder.) It saves on its own thread and reports each save as a line;
    // the settings are read at step 6, before the sound starts.
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
    store.flush();
    // A project save makes its sounds and parts durable with one sync of
    // the disks, not a sync per file (a save took 64 s on the stick in the
    // fourth creator round, 16 Nov).
    mpfree_library::set_flush(sys::sync_disks);
    // 0. The first start on a stick the USB creator made: the eMMC backup,
    // then the MPC's own kits and samples from it (setup.rs), saved as each
    // step ends.
    let akai_kits = first_start(o, out, timed, &store);
    // 1. Kits: the MPC's own (from the first start), then MPFree's.
    progress(out, 1, "Loading drum kits", "Reading the kits file", 0.02);
    let own = load_kits(&o.kits, &mut |done, total| {
        let value = 0.02 + 0.38 * done as f32 / total.max(1) as f32;
        progress(
            out,
            1,
            "Loading drum kits",
            &format!("{:.1} of {:.1} MB", done as f64 / 1e6, total as f64 / 1e6),
            value,
        );
    })?;
    let mut kits = match &akai_kits {
        Some(path) => match load_kits(path, &mut |_, _| {}) {
            Ok(akai) => {
                out.say(&format!(
                    "kits: {} of the MPC's own, from {}, before MPFree's {}",
                    akai.len(),
                    path.display(),
                    own.len()
                ));
                setup::combine(akai, own, kit::MAX_KITS)
            }
            Err(e) => {
                out.say(&format!("kits: the MPC's own kits not loaded ({e})"));
                own
            }
        },
        None => own,
    };
    // The sampler engine plays them (and adds the Grains kit when there is room).
    progress(
        out,
        1,
        "Loading drum kits",
        "Handing the sounds to the sampler",
        0.4,
    );
    let mut player =
        sampler::Player::with_user_kits(&mut kits, &mpc_one::PAD_NOTES, RATE, sampler::USER_KITS)?;
    // The empty kits kept for New kit, after the others and Grains.
    let user_kits = player.user_kits();
    // What each instrument plays in, as the loop last handed it over
    // (tunings step T5).
    // Boxed: its tables would sit on the loop's stack.
    let mut tunings_handed = Box::new(TuningsHanded::new());
    let sounds: usize = kits.iter().map(|k| k.sounds.len()).sum();
    out.say(&format!("pad player: {} kits, {sounds} sounds", kits.len()));
    for (k, kit) in kits
        .iter()
        .enumerate()
        .filter(|(k, _)| !user_kits.contains(k))
    {
        out.say(&format!(
            "kit {}: {} ({} sounds)",
            k + 1,
            kit.name,
            kit.sounds.len()
        ));
    }
    if !user_kits.is_empty() {
        out.say(&format!(
            "kits {} to {}: empty, for the kit list's New kit",
            user_kits.start + 1,
            user_kits.end
        ));
    }
    // The synth: one more entry after the kits, and the Synth tab.
    // The synth sits in a slot that can hand it to its own lane (Prefs
    // "Sound cores": Cores 3, 2 and 1); between periods it is always here.
    let mut synth = cores::SynthSlot::new(synth::Synth::new(RATE));
    out.say(&format!("synth: {}, preset {}, as kit list entry {} ({}); the Synth tab edits it and picks Poly or FM",
                     synth.engine().name(), synth.preset_name(), kits.len() + 1, synth.range_text()));
    out.show(gui::ToUi::Kits(kit_views(&kits, user_kits.clone(), &synth)));
    out.show(gui::ToUi::Synth(synth.state()));
    // The Sample tab edits the kits' projects on the screen thread.
    out.show(gui::ToUi::SampleKits {
        projects: player.take_projects(),
        rate: RATE,
    });
    let drum_kits = kits.len();
    let synth_kit = drum_kits;
    let n = drum_kits + 1;
    // Whether the armed track plays the synth: the pads play it then.
    let mut synth_on = false;
    // The synth chosen for the pads while the armed track plays something
    // else (app/pads_focus.rs): the pads play it on their own, before the
    // arp and the recorder; nothing records them. While the track plays
    // the synth, they play it through the track. The pads a hit started
    // there, so their let-go ends it there.
    // From step 6, an instrument of the host's chosen for them (a free
    // place's, picked in the list of synths) the same way, by the host's
    // pads player (`instruments::THE_PADS`), while the armed track doesn't
    // play it.
    let mut pads_preview: Option<PadsPreview> = None;
    // The source bank the screen was last told the pads play (sampler
    // step 15e), while it isn't the bank shown.
    let mut kit_source: Option<usize> = None;
    // The pads' kit layers the screen was last told of (sampler step 15e's
    // follow-up).
    let mut layer_kits_shown = None;
    // The name of the instrument the host's pads player plays, as the
    // screen last heard it (step 6).
    let mut pads_host_shown: Option<&'static str> = None;
    // What the pads play with their layers, as the screen last heard it
    // (step 11b).
    let mut layers_shown = [None; instruments::PAD_LAYERS];
    // What the screen shows the pads play, and their bank's letter when it
    // is the bank's own (step 6c).
    let mut pads_shown: (app::PadFocus, Option<usize>) = (app::PadFocus::Track, None);
    // Each kit's tuning as the Pads tab's Edit pad last showed it.
    let mut kit_tunings_shown = None;
    // Who plays the Synth tab's instrument, as last shown (step 8b), and
    // when to look again.
    let mut who_shown: Option<String> = None;
    // When the selected track's line was last asked for (step 11d).
    let mut line_at = 0.0;
    let mut who_at = 0.0f64;
    let mut preview_held: [Option<PadsPreview>; 16] = [None; 16];
    // Every kit strip was taken when a kit wanted one (sampler step 15d),
    // said once each time it starts.
    let mut strips_full = false;
    // The pad of the kit each pad held on a kit chosen for them hit
    // (sampler step 15), so its let-go and pressure find it.
    let mut preview_pad = [0u8; 16];
    // The notes the note editor hears (piano roll plan 1C.2), and whether
    // the transport rolled when they started: it starting or stopping
    // lets them go.
    let mut hearing = hearing::Hearing::default();
    let mut hear_rolling = false;
    // The list of instruments changed since the app last had it (step 6).
    let mut list_changed = true;
    // What each track plays: a kit, the Grains kit or the synth (kit 1 on
    // every track until a pick). The pads play the armed track's; each
    // track's clips play its own.
    let mut track_kits = sampler::TrackKits::new(drum_kits);
    // The kits played as notes, read from the player each period (sampler
    // step 15c, part 3).
    let mut kit_notes = sampler::KitNotes::default();
    // The project's instruments, each with its lasting id (the shared
    // synth is instrument 1). Made here, before the sound starts; an open
    // swaps in the project's, and this one goes back to be freed on the
    // screen thread. Each kit place has its id (phase 3 step 14): the
    // places are fixed while mpfree runs.
    let mut listed = instrument_list::InstrumentList::new();
    listed.follow_kits(drum_kits);
    out.show(gui::ToUi::KitIds(Box::new(listed.kit_ids())));
    // The time engine's REDOs seen so far: a track one brings back gets its
    // kit back (`TrackKits::follow`).
    let mut redos_seen = 0u64;
    // The armed track changed: next period the pads follow it to what it
    // plays (as a pick would, but nothing changes what a track plays).
    let mut kit_follow: Option<usize> = None;
    // Each track's own instrument's preset, as last picked (instruments.rs).
    let mut track_presets = presets::TrackPresets::default();
    // When the Synth tab looks again at a track whose plugin just loaded a
    // preset (its values may come later).
    let mut preset_looks: Vec<(usize, Instant)> = Vec::with_capacity(8);
    let mut param_log = synth::ParamLog::default();
    // UNDO on the Synth tab and the mixer: their knob and fader turns.
    let mut synth_turns: undo::Turns<u16> = undo::Turns::default();
    let mut mix_turns: mixturns::MixTurns = undo::Turns::default();
    // Your XY layouts in this song, and Stay's lifts for UNDO.
    let mut xy_yours = xy_song::XySong::default();
    let mut xy_turns: xy_song::XyTurns = undo::Turns::default();
    // A project's layouts came in: the one it played plays once its mixer
    // and mod set-up are in.
    let mut xy_opening = false;
    // What the screen was last told about them, for the UNDO light.
    let mut turns_told = (false, false, false);
    let mut synth_time = RenderTime::default();
    // Q-Links on the Public port (no Akai session) send positions; the Synth
    // page wants turns, so the last position of each is kept.
    let mut qlink_at: [Option<u8>; 4] = [None; 4];
    let mut synth_meters = synth.meters();
    // What each track plays, as the screen last heard it (the line under
    // its name on Arrange).
    let mut sounds_shown: gui::TrackSounds = [None; instruments::HOLDERS];
    let mut sends_shown: gui::TrackSends = [None; instruments::TRACKS];
    // Where REC would record the pads' synth, as the REC chip last named it
    // (the recording plan, `desk::rec_plan`).
    let mut plan_shown: Option<app::RecPlan> = None;
    let mut kits_shown = sampler::TrackKits::default();
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
    // All four PAD BANK buttons: the screen pictures' names this run.
    let mut picture_names = screen_picture::Names::default();
    let mut report_names = status_report::Names::default();
    let mut recolour = false;
    let (mut shown_heads, mut heads_at) = (sampler::Playheads::default(), Instant::now());
    // The Grain tab's cloud box, while the audio thread holds it: filled and
    // sent back with the playheads (never freed on this thread).
    let mut cloud: Option<Box<sampler::Cloud>> = None;
    // The sampler's line input, while it is on; and one handed back, on
    // its way to the screen thread to be freed there.
    let mut rec_feed: Option<Box<sampler::RecFeed>> = None;
    let mut rec_back: Option<Box<sampler::RecFeed>> = None;
    // What the sampler's input records from (the recorder's Source).
    let mut rec_source = sampler::RecSource::Line;
    // The recorder's Gain on the line input, linear (one-page step 2d),
    // and where the last period's ramp ended.
    let mut rec_gain = 1.0f32;
    let mut rec_gain_was = 1.0f32;
    // Monitor's level (the line input strip's fader) the screen was last
    // told, dB.
    let mut line_level_said: Option<f32> = None;
    // Sample Recall (sampler decision 3): the master as heard, its last two
    // minutes always kept, made here once and never freed; its reading end
    // goes to the screen as soon as there is room.
    let (recall_feed, recall_input) =
        mpfree_sampler::record::input16(sample_page::RECALL_SECONDS, RATE);
    let mut recall_feed = sampler::RecFeed(recall_feed);
    let mut recall_out = Some(gui::ToUi::SampleRecall(Box::new(recall_input)));
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
    let mut app = app::App::new(RATE, timed && o.workflow.is_none());
    // The kits by name, for the Tunings page (an empty kit for New kit has
    // none).
    app.set_kit_names(
        kits.iter()
            .enumerate()
            .map(|(k, kit)| {
                if user_kits.contains(&k) && kit.sounds.is_empty() {
                    String::new()
                } else {
                    kit.name.clone()
                }
            })
            .collect(),
    );
    // Prefs > Backup and Akai kits: what the first start did or left out.
    app.set_setup(setup_prefs(o, &store));
    // Saved settings and found buttons, read now from the store (opened
    // before the first start), before the sound starts with the saved audio
    // buffer.
    // The themes folder: theme files the user put in files/themes/ (read
    // only), offered in Prefs > Appearance after the built-in themes. Listed
    // before restore so a saved theme file comes back.
    let folder = load_theme_files(&store);
    for line in &folder.notes {
        out.say(line);
    }
    app.theme_files = folder
        .found
        .iter()
        .map(|(f, t)| (f.clone(), t.name.clone()))
        .collect();
    app.theme_problems = folder.problems;
    app.themes_left_out = folder.left_out;
    let themes_left_out = app.themes_left_out.len() as u64;
    let theme_lines_fell_back = folder.lines_fell_back;
    out.show(gui::ToUi::ThemeFiles(folder.found));
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
    // The MPC plugins' folders, where the Synth tab finds their skins.
    out.show(gui::ToUi::PluginFolders(skin_page::folders(&found_plugins)));
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
    // A scripted run presses every button by name: the ones Map controls
    // hasn't found get numbers of their own for this run (as the smoke
    // session's), never saved to the stick.
    if let Some(w) = &o.workflow {
        out.say(&script::map_unmapped(&mut app));
        out.say(&format!(
            "script: {:?}, {} steps, {:.1} s; stands in for {}",
            w.title,
            w.lines.len(),
            w.seconds(),
            w.covers.join(", ")
        ));
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
    // Theme files left out, and a saved theme that couldn't be used (the
    // default shows): counted on the Debug page.
    if themes_left_out > 0 || theme_lines_fell_back > 0 || app.theme_fallbacks > 0 {
        out.show(gui::ToUi::ThemeFallbacks {
            left_out: themes_left_out,
            lines_fell_back: theme_lines_fell_back,
            default_shown: app.theme_fallbacks,
        });
    }
    // A new project's Perform set, for the Pads tab's Perform view (a
    // project opened brings its own).
    out.show(gui::ToUi::PerformSet {
        set: Box::new(app.perform_set().clone()),
        opened: true,
    });
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
    // The time engine (transport, clips, takes) runs in this loop too;
    // with --time-thread its editing side has a thread of its own.
    let mut time = time::TimeHost::new_project(RATE, app.settings.bpm)?;
    if o.time_thread {
        time = time_apart(out, time);
    }
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
    // System status's stick meter reads the drive's looks the audio
    // tracks take of the stick.
    out.show(gui::ToUi::Drive(tracks.host.drive_seen()));
    out.say(&format!(
        "audio tracks: {} of stereo memory for takes ({} MB, Prefs > Audio track memory)",
        audio_memory.time(),
        audio_memory.megabytes(RATE)
    ));
    // Dev's "Audio tracks from their files" (off by default): started
    // before the takes record to files, so the writer waits for playback's
    // reads; the screen opens projects without reading their takes then.
    // A script that needs stream-audio has it for its run only (the saved
    // setting stays as it is).
    let script_streams = o
        .workflow
        .as_ref()
        .is_some_and(|w| w.needs.iter().any(|n| n == script::STREAM_AUDIO));
    if app.settings.stream_audio || script_streams {
        if let Some(bytes) = tracks.stream(audio_memory.seconds()) {
            out.say(&format!(
                "audio tracks: takes play from their files ({}), through a {} MB cache",
                if app.settings.stream_audio {
                    "Dev, MENU: Audio tracks from their files"
                } else {
                    "the script needs stream-audio, for this run"
                },
                bytes >> 20
            ));
        }
        // A stream test that plays from memory would measure nothing: it
        // stops here, before a step runs, and says why.
        if script_streams && !tracks.streams() {
            return Err(
                "the script needs stream-audio, but takes can't play from their files in this run"
                    .into(),
            );
        }
        app.settings.audio_streams = tracks.streams();
        out.show(gui::ToUi::Settings(app.settings.clone()));
    }
    // Loops on audio tracks (Dev's switch, phase 4b): for this start.
    if app.settings.loops_on_tracks {
        app.settings.loops_on = true;
        out.say("looper: loops play on audio tracks (Dev, MENU: Loops on audio tracks)");
        out.show(gui::ToUi::Settings(app.settings.clone()));
    }
    let loops_on = app.settings.loops_on;
    time.set_loops_on(loops_on);
    // Layers as recordings (phase 4c.2) come with the same switch.
    looper.set_recordings(loops_on);
    // And the one UNDO across the takes, the loops and the audio tracks
    // (phase 4d.1), followed once a period.
    let mut one_undo = one_undo::OneUndo::default();
    // With takes streaming, a cue that launches audio waits until the
    // start of its audio is in memory (the audio tracks rethink's phase
    // 2): the needs go to the tracks whenever they change.
    time.set_audio_waits(tracks.streams());
    let mut needs_rev = 0u64;
    // A Fit to tempo ended, or a sound Put in a cell came in: its clip
    // waits while the time engine records.
    let mut fit_done: Option<audiotracks::ClipFitDone> = None;
    let mut cell_done: Option<Result<(usize, mpfree_seq::Clip), String>> = None;
    let st = time_state(&time, &tracks);
    // What the Perform pads' clip windows and scene pads light by, kept
    // from each state the screen gets (the Arrange refactor's 5.5).
    let mut pad_grid = perform_pads::Grid::of(&st);
    out.show(gui::ToUi::Time(Box::new(st)));
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
    let max_db = o.gain_db();
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
    cores.start_tracks(&mut mix.instruments);
    cores.start_pads(&mut mix.pad_synths);
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
    // The engines an open replaced wait here, two at most: the screen keeps
    // at most two opens on their way (gui::OPENS_ON_THEIR_WAY), so a box
    // always finds a slot and none is ever freed on this thread.
    let mut project_back: [Option<gui::ToUi>; gui::OPENS_ON_THEIR_WAY] = Default::default();
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
    // A print for the screen to save, kept until the screen has room (its
    // audio is never freed on this thread).
    let mut print_out: Option<gui::ToUi> = None;
    // MIDI Learn: values for parameters a control can't set yet, and the
    // I/O view's names taken at this generation of the model.
    let mut learn_misses = 0u64;
    // Learned controls' kit pad settings this pass, for the screen
    // (`ToUi::PadsLearned`).
    let mut pads_learned: Vec<(usize, u8, sampler::PadParam, f32)> = Vec::new();
    // Learned controls on the transport (remote.rs): the presses they hold,
    // and whether the log said a learned tempo waits for a followed clock.
    let mut remote = remote::Remote::new();
    // Keys learned on the Perform pads (Learn a key): the pads they hold.
    let mut perform_keys = remote::PerformKeys::default();
    let mut remote_tempo_waits = false;
    // Learned controls on Play's Swing, Drums filter and XY pad (the Learn
    // section's Play card), done once a pass.
    let mut play_moves = remote::PlayMoves::default();
    // Next and Previous patch pressed this pass (the Play card, live
    // shows C1): a step and whether it waits for the hands, done once the
    // learned controls are read.
    let mut patch_steps: Vec<(i32, bool)> = Vec::with_capacity(8);
    let mut learn_names_gen: Option<u64> = None;
    let mut learn_now_at = 0.0_f64;
    // The gear lanes' pink ~: the MIDI out controls the mod matrix moves,
    // as last shown, and the next look (LFO to CC).
    let (mut gear_moved, mut gear_moved_now) = (Vec::new(), Vec::new());
    let mut gear_moved_at = 0.0_f64;
    let mut track_names_at = 0.0_f64;
    // What the screen was last told of the keys (live shows C1).
    let mut keys_told: (Option<usize>, Option<Option<usize>>, u32) = (None, None, 0);
    let mut learning_shown: Option<(mpfree_play::mpfree_param::ParamAddr, midi_in::ArmedBy)> = None;
    // A learned control moved a synth's knob: the Synth tab is told, at
    // most every 50 ms.
    let (mut synth_learned, mut synth_learned_at) = (false, 0.0f64);
    // A followed clock's tempo for the app, taken at the next turn.
    let mut followed_tempo: Option<f64> = None;
    // The tempo a free first loop set (tempo from the first loop), for the
    // app at the next turn, before the transport follows the app's tempo.
    let mut loop_tempo: Option<f64> = None;
    // The port the tempo chip last said the tempo comes from.
    let mut clock_from: Option<String> = None;
    // What each track's instrument is, told to the time engine each period.
    let mut inst_now: [Option<time::InstRef>; mpfree_seq::MAX_TRACKS] =
        [None; mpfree_seq::MAX_TRACKS];
    out.show(gui::ToUi::Macros(Box::new(panels.macros_state(&mix))));
    let mut shown_levels = mix.play_levels();
    out.show(gui::ToUi::PlayLevels(shown_levels));
    let mut meter_at = Instant::now();
    // A set's song's ending (live shows B1): its gain on the output.
    let mut song_end = song_end::SongEnd::new(RATE);
    // Soundcheck's Muted (soundcheck.rs): silence after every measure.
    let mut outputs_muted = false;
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
    // The owner's own phrases, kept beside the patterns (arp/phrases.txt).
    match store
        .read(app::saved::ARP, app::saved::ARP_PHRASES)
        .map(String::from_utf8)
    {
        Some(Ok(text)) => fx.extend(app.load_phrases(&text)),
        Some(Err(_)) => out.note("arp phrases: the saved file is not text; MPFree's phrases stay"),
        None => {}
    }
    app.pattern_file = store.saves();
    // The Synth tab's options are the project's (project.rs): they start at
    // their defaults, and an open puts back the project's.
    let (mut left, mut right) = ([0f32; PERIOD], [0f32; PERIOD]);
    // Each engine's period, for its track, and the pads' sound for the looper.
    let (mut drums_l, mut drums_r) = ([0f32; PERIOD], [0f32; PERIOD]);
    let (mut synth_l, mut synth_r) = ([0f32; PERIOD], [0f32; PERIOD]);
    let (mut loop_l, mut loop_r) = ([0f32; PERIOD], [0f32; PERIOD]);
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
    let mut stopped_by = if o.workflow.is_some() {
        "script"
    } else {
        "time"
    };
    // A scripted run's steps, handed out as their periods come; a mark
    // asks for the 30 s status lines at once, so the tools can read each
    // stretch's totals.
    let mut script_run = o.workflow.as_ref().map(script::Runner::new);
    let script_end = o
        .workflow
        .as_ref()
        .map_or(u64::MAX, |w| w.periods + script::periods_in(1.0));
    let mut status_now = false;
    let mut songs_put = false;
    // Pads held (on the controller or the screen), pressed but not yet sounded
    // (Note Repeat starts a pad on the next step of its grid), and lit.
    let mut held = [false; 16];
    let mut awaiting = [false; 16];
    // Each pad's colour over its resting one (None: resting).
    let mut lit: [Option<[u8; 3]>; 16] = [None; 16];
    // Live hits on Loop pads waiting for the launch line (sampler R5.3b):
    // those that play each period, how many found the table full and
    // played at once, how many STOP dropped, and the pads shown waiting
    // on screen.
    let mut due_pads = time::DuePads::default();
    let mut waits_full = 0u64;
    let mut stop_dropped = 0usize;
    let mut waiting_shown = 0u16;
    // TRACK MUTE: pads pressed as mute keys, or as 16 Levels' Transpose
    // steps (their release plays nothing), and the colours the mute keys
    // show (None: the pads show as usual).
    let mut mute_pressed = [false; 16];
    // The loop keys held (SHIFT + SAMPLER on the Looper tab).
    let mut keys_held = perform_pads::Held::default();
    let mut mute_shown: Option<[[u8; 3]; 16]> = None;
    // Live transpose (S4): each track's arp notes by number, and where its
    // shift took them (SeqPads::sounding).
    let mut by_note = vec![mpfree_arp::scale::Held::new(); mpfree_seq::MAX_TRACKS];
    // The tracks silent last period (`TimeHost::silenced`).
    let mut silent_was = 0u128;
    // STEP SEQ (step_keys.rs): the pads held as steps, the steps the pads
    // show (kept from the last change), and the PAD BANK lights sent while
    // they are pages.
    let mut step_keys = step_keys::StepKeys::default();
    let mut step_view: Option<time::StepsData> = None;
    let mut pages_shown: Option<[u8; 4]> = None;
    // Controller mode (controller_out.rs): the pads and Q-Links as a MIDI
    // controller out of one port, while the I/O view's Controller section
    // has it on.
    let mut controller = controller_out::Controller::new();
    let mut controller_sending = false;
    // Buttons whose light is never sent, noted once each.
    let mut unlit = [false; 128];
    // Repeats per second, for the log (the paced Latch check reads these).
    let (mut window_repeats, mut window_pads, mut last_repeat_note) =
        (0u64, [false; 128], Instant::now());
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
    // System status's tracks using the most: counted one period in a few,
    // published with the engines (track_costs.rs).
    let mut track_costs = track_costs::TrackCosts::new();
    cores.show(&engine_shared.cores);
    // MIDI from outside (midi_in.rs): the 5-pin MIDI In (cable 2, already
    // open with the controller's other cables) and USB keyboards, read only,
    // as keys for the armed track.
    let mut music = midi_in::MusicIn::new();
    // What the mixer's I/O view shows and edits (MIDI, Sync and CV). The
    // loop only ever try_locks it (MusicIn::sync).
    let io_model = midi_in::io_model();
    // The I/O view's rig as it was left (midi_saved.rs): each port's input,
    // what it ignores and its delay, for every project. The song's MIDI settings come with the
    // project (its `midi` part).
    let rig_text = store
        .read(midi_saved::OWNER, midi_saved::RIG)
        .map(|b| String::from_utf8_lossy(&b).into_owned());
    for w in midi_saved::restore(&io_model, rig_text.as_deref()) {
        out.note(&format!("midi: setting not read: {w}"));
    }
    // Device profiles the musician put in files/midi-profiles/ (read
    // only), beside the built-in General MIDI: they name a port's controls
    // in the Monitor and Learn.
    for line in midi_saved::load_profiles(&store, &io_model) {
        out.note(&line);
    }
    let mut midi_saver = midi_saved::Saver::new(&io_model);
    // The Monitor's Save log (through the store, files/midi-logs/) and its
    // lines in RAM for the workstation console while the developer tools
    // are on (midi_log.rs): one thread, stopped when the session ends.
    io_model
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .live
        .saves_logs = store.saves();
    let _midi_log =
        match midi_log::Writer::start(Path::new(REPORT_DIR), io_model.clone(), store.clone()) {
            Ok(w) => Some(w),
            Err(e) => {
                out.note(&e);
                None
            }
        };
    out.show(gui::ToUi::IoModel(io_model.clone()));
    // MIDI Out (midi_out.rs): tracks whose "Plays on" is the 5-pin jacks play
    // there. CV out (cv_out.rs): CV 1 to 4 follow the project's CV outputs.
    // State-changing on the MPC (note and clock bytes on cable 2, control
    // changes on cable 1, never SysEx), so both open only while Prefs >
    // "Send MIDI and CV out" is on (`apply_sends`, in the loop), and close
    // when it goes off.
    let mut midi_out = midi_out::MidiOut::new(RATE, MIDI_OUT_LEAD);
    // Export (export.rs): a pass the screen sends plays the song into its
    // files; its MIDI file hears the notes as MIDI Out would send them.
    let mut export = export::Host::new();
    let mut cv_out = cv_out::CvOut::new(RATE, MIDI_OUT_LEAD);
    // Prefs' "Send MIDI and CV out" as last applied (None: not yet).
    let mut sends: Option<bool> = None;
    let mut usb = UsbKeys::start(
        [Some(audio), midi_card().ok()]
            .into_iter()
            .flatten()
            .collect(),
    );
    let mut keys: Vec<midi_in::Key> = Vec::with_capacity(64);
    // Keys a route sends to a track that isn't armed, played at once.
    // Room for a period's routed keys and everything a patch can send in
    // one (live shows C1), so the sound loop never grows it.
    let mut routed_keys: Vec<(u8, midi_in::Key)> = Vec::with_capacity(64 + patch_in::OUTS);
    let mut routed_held = midi_in::RoutedHeld::new();
    let mut routed_pads = midi_in::RoutedPads::new();
    // The track the keys' expression last went to on MIDI out: when
    // another is armed, its gear's bend and pressure go back to rest.
    let mut expr_track = 0usize;
    // And the synth the pads played on their own then (step 7b), whose
    // pedals come up as the pads move on.
    let mut expr_pads: Option<PadsPreview> = None;
    // The pedals the keys put down on it (`expr_track`).
    let mut pedals_down = midi_in::PedalsDown::default();
    // Keys playing the pads' synth while the pads play it on their own.
    let mut keys_on_pads = KeysOnPads::new();
    // The pads' layers beyond the first (step 11a), which the pads' hits
    // and the keys play too.
    let mut pad_layers = PadLayers::new();
    // NOTE REPEAT, Latch and the arp while the pads play an instrument on
    // their own (sampler step 15c).
    let mut pads_arp = PadsArp::new(RATE, app.bpm());
    // The pads' own hits and keys into the track recording what they play
    // on their own (sampler step 15c).
    let mut pads_rec = PadsRec::new();
    let mut route_synths = RouteSynths::new();
    // The keys' expression on its gear, thinned for the recorder and the
    // move log (the MIDI plan's 3.7).
    let mut expr_rec = expr_rec::ExprRec::new();
    let mut xy_rec = xy_rec::XyRec::new();
    let mut key_misses = 0u64;
    // Keys the armed track's kit played on its keygroups at once, past the
    // arp and REC (sampler step 15; they record from step 15c).
    let mut keys_direct = 0u64;
    // Messages each port's MIDI out writer dropped with its queue full, as
    // the monitor was last told.
    let mut out_full = [0u64; mpfree_midi::MAX_PORTS];
    // The lanes' deadline for this period (cores.rs): when the sound card's
    // queue would run dry, less a period. The buffer starts full.
    let mut lane_until = cores::lane_deadline(Instant::now(), None, periods);
    // Load shedding (shed.rs): a period behind with the queue running down
    // gives up the voices nobody hears. Each period's work runs from the
    // last write to this one.
    let mut shed = shed::Shed::new(PERIOD, RATE, periods);
    let mut last_written = Instant::now();
    let mut i = 0u64;
    while i < total_periods {
        let t_read = Instant::now();
        let period_span = Span::start();
        parts.begin();
        let t = started.elapsed().as_secs_f64();
        let mut kit_to: Option<usize> = kit_follow.take();
        let mut audio_change: Option<u32> = None;
        let mut bank_lights = false;
        // PAD BANK turned the kit's page: the pads' colours follow.
        let mut page_turned = false;
        // Hits are timed only when they sound at once (not on the arp's grid).
        let direct = !app.generating();
        while let Ok(a) = from_ui.try_recv() {
            // The Q-Links' values on screen, for MIDI Follow's Send values
            // back.
            if let gui::FromUi::QLinkValues(v) = a {
                music.set_qlink_values(v);
                continue;
            }
            // Controller mode: a drag on a Q-Link card and the screen's
            // pads are the controller's too.
            if controller.on() {
                let mut send = |to, msg| {
                    midi_out.controller(to, msg);
                };
                match a {
                    gui::FromUi::QLink { knob, delta } => {
                        controller.turn(knob, delta, &mut send);
                        continue;
                    }
                    gui::FromUi::Pad { pad, down: true } => {
                        let v = app.velocity(app.settings.screen_velocity);
                        controller.pad_down(pad, v, &mut send);
                        continue;
                    }
                    gui::FromUi::Pad { pad, down: false } => {
                        controller.pad_up(pad, &mut send);
                        continue;
                    }
                    _ => {}
                }
            }
            let (t_item, item) = (Instant::now(), parts::Name::of(&a));
            // A ready-made layout picked on the XY pad's list: none of
            // yours plays from now.
            let xy_picked = matches!(a, gui::FromUi::PlayFx(mixing::PlayFx::XyEffect(_)));
            let mix_turn = mixturns::before_ui(&a, &mix, app.view() == app::View::Mixer);
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
                listed: &listed,
                level_db: &mut level_db,
                max_db,
                card: audio,
                undo: &mut one_undo,
            };
            if step_turn(&a) {
                step_keys.turned();
            }
            record_knob(&a, desk.app, desk.mix, desk.time);
            let a = desk.screen(a, t, out, &mut fx);
            save_shelf(&mut mix, &store, out);
            if xy_picked && xy_yours.playing.take().is_some() {
                let pad_now = |owner, id: u16| {
                    let (kit, pad) = learned_kit(&listed, &track_kits, owner)?;
                    pad_describe(&player, kit, pad, id)
                };
                out.show(xy_view(
                    &mut xy_yours,
                    &mix,
                    &synth,
                    &app,
                    &listed,
                    &pad_now,
                    &time,
                ));
            }
            // UNDO on the mixer: a strip control's turn, from its value
            // before the desk set it.
            if let Some(turn) = mix_turn {
                mixturns::record(&mut mix_turns, turn, level_db, t_read);
            }
            let Some(a) = a else {
                parts.item_named(parts::SCREEN_ACTIONS, t_item, &item);
                continue;
            };
            match a {
                // The Controller chip: controller mode off (its section's
                // switch, in the I/O model, which MusicIn::sync hands back).
                gui::FromUi::ControllerOff => {
                    if let Ok(mut m) = io_model.lock() {
                        m.setup.controller.on = false;
                        m.gen += 1;
                    }
                }
                gui::FromUi::Pad { pad, down: true } if pad < 16 && app.pads_are_steps() => {
                    let v = app.velocity(app.settings.screen_velocity);
                    let rolling = time.info().state.rolling();
                    let p = step_press(&app, &mut step_keys, &mut time, pad, v, rolling);
                    fx.extend(sound_step(p, out, &mut player, &mut synth, synth_on));
                }
                gui::FromUi::Pad { pad, down: false } if pad < 16 && step_keys.held(pad) => {
                    let r = step_keys.release(pad, &mut time);
                    let_go_of_step(r, out, &mut player, &mut synth, synth_on);
                    out.show(gui::ToUi::PadUp(pad));
                }
                gui::FromUi::Pad { pad, down: true } if pad < 16 && app.pads_are_loop_keys() => {
                    let action = app.perform_page().action(pad);
                    keys_held.press_action(pad, t, action);
                    let drums = kit_drums(&kits, player.kit());
                    let (more, line) =
                        desk::perform_pad_on(&mut app, &time, &looper, out, action, drums);
                    fx.extend(more);
                    if !line.is_empty() {
                        out.note(&line);
                    }
                }
                gui::FromUi::Pad { pad, down: false }
                    if pad < 16 && keys_held.action(pad).is_some() =>
                {
                    if let Some(action) = keys_held.release_action(pad) {
                        fx.extend(desk::perform_pad_up(&mut app, action));
                    }
                    out.show(gui::ToUi::PadUp(pad));
                }
                gui::FromUi::Pad { pad, down: true } if pad < 16 && app.pads_mute() => {
                    mute_pressed[pad] = true;
                    let (tap, more) = app.mute_tap();
                    fx.extend(more);
                    let why = no_mutes(&track_kits, mix.instruments.note_tracks(), app.track());
                    let (t, page) = (app.track(), app.pad_page());
                    out.note(&mute_key(tap, pad, t, page, why, &mut time));
                }
                gui::FromUi::Pad { pad, down: true } if pad < 16 && app.pads_transpose() => {
                    mute_pressed[pad] = true;
                    let own = &mix.instruments;
                    out.note(&transpose_pad(pad, &mut time, &synth, &track_kits, own));
                }
                gui::FromUi::Pad { pad, down: false } if pad < 16 && mute_pressed[pad] => {
                    mute_pressed[pad] = false;
                    out.show(gui::ToUi::PadUp(pad));
                }
                gui::FromUi::Pad { pad, down: true } if pad < 16 => {
                    let v = app.settings.screen_velocity;
                    let take = app.rec.ready.map(|take| take == rec_arm::Take::Add);
                    out.loop_start(pad_starts_loop(
                        &mut looper,
                        &mut time,
                        take,
                        export.running(),
                    ));
                    held[pad] = true;
                    screen_hits += 1;
                    // A Loop pad's hit waits for the launch line (R5.3b).
                    let w = time::PadWait {
                        from: time::PadFrom::Pad(pad as u8),
                        down: true,
                        velocity: v,
                        kit_pad: app.kit_pad(pad) as u8,
                    };
                    if !waits_for_line(&mut time, &player, &app, synth_on, w, &mut waits_full, out)
                    {
                        if let Some(ev) = app.pressed_event(pad, v, true) {
                            // With the notes it plays, for a take of notes (step 5d):
                            // on a kit played as notes, the bank's (step 15c).
                            let t = app.track();
                            let rows =
                                pad_rows(&mix.instruments, &synth, &track_kits, &kit_notes, t);
                            let kit_note = kit_notes.note(&track_kits, t, app.kit_pad(pad));
                            let notes = kit_note
                                .as_ref()
                                .map_or(rows.notes(pad), std::slice::from_ref);
                            time.pad_pressed_notes(0, ev, notes);
                        }
                        fx.extend(app.pad_down(pad, v));
                        awaiting[pad] = true;
                        if direct {
                            pending.push(t_read);
                        }
                    }
                }
                gui::FromUi::Pad { pad, down: false } if pad < 16 => {
                    held[pad] = false;
                    let w = time::PadWait {
                        from: time::PadFrom::Pad(pad as u8),
                        down: false,
                        velocity: 0,
                        kit_pad: app.kit_pad(pad) as u8,
                    };
                    if !waits_for_line(&mut time, &player, &app, synth_on, w, &mut waits_full, out)
                    {
                        if let Some(ev) = app.pressed_event(pad, 0, false) {
                            // With the notes it plays, for a take of notes (step 5d):
                            // on a kit played as notes, the ones it pressed (step 15c).
                            let t = app.track();
                            let rows =
                                pad_rows(&mix.instruments, &synth, &track_kits, &kit_notes, t);
                            let notes = match kit_notes.of_track(&track_kits, t) {
                                Some(_) => app.pad_held_notes(pad),
                                None => rows.notes(pad),
                            };
                            time.pad_pressed_notes(0, ev, notes);
                        }
                        fx.extend(app.pad_up(pad));
                        awaiting[pad] = false;
                        out.show(gui::ToUi::PadUp(pad));
                    }
                }
                gui::FromUi::Pad { .. } => {}
                // A scale row's name on Beats: the pad sounds while held.
                gui::FromUi::Audition { pad, down } if pad < 128 => {
                    audition(pad, down, &mut player, &mut synth, synth_on);
                }
                gui::FromUi::Audition { .. } => {}
                // The note editor hears notes picked, drawn or moved.
                gui::FromUi::Hear(ask) => {
                    let mut on = HearOn {
                        own: &mut mix.instruments,
                        synth: &mut synth,
                        player: &mut player,
                        kits: &track_kits,
                        midi: &midi_out,
                    };
                    hearing.hear(&ask, t, &mut on);
                    hear_rolling = time.info().state.rolling();
                }
                // The Pads tab's strip: the kit's bank on the pads, as PAD
                // BANK turns it (the synth's pads have layouts, not banks).
                gui::FromUi::PadBank(b) if b < app::KIT_PAGES && !synth_on => {
                    fx.extend(app.set_pad_page(b));
                    let kit = kit_to.unwrap_or(player.kit());
                    out.note(&pad_page_note(b, &kits[kit].name, player.sounds(kit)));
                    page_turned = true;
                    bank_lights = true;
                }
                gui::FromUi::PadBank(_) => {}
                // A pick in the list of kits: what the track plays. The
                // pads' own choice stays (sampler step 15b: nothing implicit
                // moves the pads); while they follow the track, they play
                // its new kit.
                gui::FromUi::Kit(k) => {
                    kit_to = Some(k.min(n - 1));
                }
                // Your XY layouts (xy_song.rs): the loop names each
                // setting, the mixer plays the layout, and the screen sees
                // the song's layouts again.
                gui::FromUi::Xy(act) => {
                    let pad_now = |owner, id: u16| {
                        let (kit, pad) = learned_kit(&listed, &track_kits, owner)?;
                        pad_describe(&player, kit, pad, id)
                    };
                    let mut xy = XyHere {
                        song: &mut xy_yours,
                        mix: &mut mix,
                        synth: &synth,
                        app: &app,
                        listed: &listed,
                        pads: &pad_now,
                        time: &time,
                    };
                    for line in xy.act(act) {
                        out.note(&format!("xy: {line}"));
                        out.show(gui::ToUi::Tell(line));
                    }
                    out.show(xy.view());
                    // Save to browser: the layout's text, for the screen's
                    // browser to keep.
                    if let Some((name, text)) = xy_yours.saving.take() {
                        out.show(gui::ToUi::XyLayoutText { name, text });
                    }
                }
                // UNDO on the Synth tab and the mixer: their turns' history.
                gui::FromUi::UndoHistory(on) => {
                    let (rows, now) = match on {
                        gui::UndoOn::Synth => synth_turns
                            .rows("Start: the sound before these turns", |t| {
                                synth_turn_row(&synth, t.key, t.to)
                            }),
                        gui::UndoOn::Mixer => {
                            mix_turns.rows("Start: the mix before these turns", mixturns::row)
                        }
                        gui::UndoOn::Xy => xy_turns.rows("Start: before these lifts", |t| {
                            xy_turn_row(&mix, &synth, &time, &listed, t)
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
                        gui::UndoOn::Xy => {
                            let turn = if redo {
                                xy_turns.redo()
                            } else {
                                xy_turns.undo()
                            };
                            turn.map(|t| {
                                for (addr, v) in xy_song::turn_values(&t, redo) {
                                    if xy_own::is_own(addr) {
                                        let mut side = OwnReach {
                                            synth: &mut synth,
                                            own: &mut mix.instruments,
                                            player: Some(&mut player),
                                            kits: &track_kits,
                                            listed: &listed,
                                            pads_learned: &mut pads_learned,
                                            synth_turned: &mut synth_learned,
                                            app: Some(&mut app),
                                            arp_fx: Vec::new(),
                                        };
                                        xy_own::OwnSide::keep(&mut side, addr, v);
                                        fx.extend(side.arp_fx);
                                    } else {
                                        panels.set_param(&mut mix, addr, v, false);
                                    }
                                }
                                format!(
                                    "{}: {}",
                                    if redo { "redo" } else { "undo" },
                                    xy_turn_row(&mix, &synth, &time, &listed, &t)
                                )
                            })
                        }
                        gui::UndoOn::Mixer => {
                            let turn = if redo {
                                mix_turns.redo()
                            } else {
                                mix_turns.undo()
                            };
                            turn.map(|t| {
                                let (line, level) = mixturns::apply(&mut mix, &t, redo);
                                if let Some(db) = level {
                                    level_db = db;
                                    out.show(gui::ToUi::Level { db: level_db });
                                }
                                line
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
                // picked (selected, and armed unless the arm is pinned) and
                // the Synth tab's MENU opens on its Instrument row.
                gui::FromUi::TrackInstrument(track) if track < time.project().tracks.len() => {
                    if time.selected() != Some(track) {
                        fx.push(app::Effect::Time(TimeAction::Pick(track)));
                    }
                    fx.extend(app.open_track_instrument(track));
                    out.show(gui::ToUi::Synth(synth_tab(
                        &synth,
                        &mix,
                        track,
                        &track_presets,
                    )));
                }
                gui::FromUi::TrackInstrument(track) => {
                    out.note(&format!("arrange: no track {}", track + 1));
                }
                // A MIDI file from BROWSE: its tracks as clips, as a take
                // there would hold them: notes on a track that plays notes,
                // else each note on the pad that plays it (midi_file.rs).
                gui::FromUi::MidiFile(m) => {
                    let n = time.track_count();
                    let notes = notes_tracks(&mix.instruments, &track_kits, &synth, &kit_notes, n);
                    let notes_on = |t: usize| t < 128 && notes >> t & 1 == 1;
                    let r = midi_file::put(&mut time, &m, notes_on, |t, n| {
                        let synth_has = track_kits.plays_synth(t).then_some(&*synth);
                        midi_file::pad_on_track(&mix.instruments, synth_has, t, n)
                    });
                    let (Ok(line) | Err(line)) = r;
                    out.note(&line);
                    out.show(gui::ToUi::Tell(line));
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
                        out.show(gui::ToUi::Problem(mpfree_status::Report::happened(
                            &mpfree_status::kinds::SAVE_FAILED,
                            1,
                        )));
                    }
                }
                // The selected track's own synth, when it has one; else the
                // shared synth.
                gui::FromUi::Synth(SynthAction::Set { id, value })
                    if mix.instruments.plays(tab_who(&app, &mix.instruments)) =>
                {
                    let who = tab_who(&app, &mix.instruments);
                    mix.instruments.param(who, id, value);
                }
                // A plugin skin showing asks for the plugin's values and
                // texts (skin_page.rs).
                gui::FromUi::SkinValues => {
                    let who = tab_who(&app, &mix.instruments);
                    mix.ask_values(instrument_track(&mix.instruments, who));
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
                // Every kit is in the engine: the Sample tab plays any
                // kit's pad, as that kit's clips would.
                gui::FromUi::SampleHit {
                    kit,
                    pad,
                    velocity,
                    down,
                } => {
                    if down {
                        player.hit_kit_at(kit, pad, velocity, 0);
                    } else {
                        player.release_kit_at(kit, pad, 0);
                    }
                }
                // The take the Wave page shows, kept off the pads. A hit
                // the engine's full queue refuses is counted there.
                gui::FromUi::SampleTake { kit, down } => {
                    player.take_at(kit, down, 0);
                }
                // The Sample tab's keys play that kit's keygroups, any kit.
                gui::FromUi::SampleKey {
                    kit,
                    note,
                    velocity,
                    down,
                } => {
                    if down {
                        player.key_kit_at(kit, note, velocity, 0);
                    } else {
                        player.key_off_kit_at(kit, note, 0);
                    }
                }
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
                    let rig = export_rig(&time, &mix, &midi_out, &music, &cv_out, &app, &looper);
                    // Stems by track: each track on the pads plays the
                    // synth, kit 1 (Drums) or another kit, in the channel
                    // of the first track that plays it; the audio tracks
                    // that sound.
                    let homes = track_kits
                        .homes(|t| !mix.instruments.plays(t) && !midi_out.takes(t as u16));
                    let pads = |t: usize| {
                        mpfree_fx::StripId::Track(match track_kits.kit_of(t) {
                            None => mixing::SYNTH,
                            Some(k) => match homes.get(k).copied().unwrap_or(0) {
                                0 => mixing::DRUMS,
                                h => mix.track_channel(usize::from(h) - 1),
                            },
                        })
                    };
                    let f = export::Facts::gather(
                        &time,
                        &mix,
                        rig,
                        &|t| midi_out.takes(t as u16),
                        &pads,
                        &tracks.heard(time.project(), time.silenced(), &mix),
                    );
                    out.show(gui::ToUi::ExportFacts(Box::new(f)));
                }
                gui::FromUi::Export(run) => match export.begin(run, &mut mix.mixer, &mut time) {
                    Ok(()) => {
                        // The tracks' controls as they are: MIDI out sends
                        // them again as the song starts, and so the file
                        // has them at the range's start.
                        for t in 0..mpfree_midi::output::MAX_TRACKS {
                            for id in 0..mpfree_play::mpfree_param::midi::COUNT {
                                if let Some(v) = midi_out.value(t, id) {
                                    export.tap.at_start(t as u16, id, v);
                                }
                            }
                        }
                        // With replays, the arps' random notes start the
                        // same in every pass.
                        if export.replays() {
                            app.arps.reseed();
                        }
                        out.note("export: the pass started");
                    }
                    Err(run) => out.show(gui::ToUi::ExportOver(run)),
                },
                // The loops go in once the old ones have faded out.
                gui::FromUi::OpenLoops(l) => looper.open(*l),
                // The audio tracks' part goes in once they are empty.
                gui::FromUi::OpenAudio(a) => tracks.open(*a),
                gui::FromUi::SongEnd(end) => {
                    // Step 1 of every ending: no new notes (the transport's
                    // STOP, which keeps a take recording; never the stopped
                    // transport's panic, which would cut the tails).
                    if end != song_end::End::Now
                        && !song_end.active()
                        && time.info().state.rolling()
                    {
                        fx.push(app::Effect::Time(TimeAction::Transport(Command::Stop)));
                    }
                    song_end.start(end);
                    out.note(&format!("show: the song ends: {end:?}"));
                }
                gui::FromUi::SongEndReset => song_end.reset(),
                gui::FromUi::Muted(m) => {
                    outputs_muted = m;
                    out.note(&format!(
                        "show: the outputs are {}",
                        if m { "muted" } else { "on" }
                    ));
                }
                // An edit on the patch screens: the keys play the song's
                // patches as they are now; the old ones go back to be
                // freed there.
                gui::FromUi::Patches(mut p) => {
                    music.swap_patches(&mut p);
                    if let Ok(m) = io_model.lock() {
                        app.set_route_synths(
                            midi_in::route_synths_of(&m.setup, music.patch_synths()).0,
                        );
                    }
                    out.show(gui::ToUi::PatchesBack(p));
                }
                gui::FromUi::PickPatch(to) => {
                    music.pick_patch(to, mpfree_patch::keys::When::Now);
                }
                // Each take to a file of its own from now on (phase 3a),
                // and where a project save put them.
                gui::FromUi::AudioFiles(dir) => tracks.record_to(dir),
                gui::FromUi::AudioSavedInto(dir) => tracks.saved_into(dir),
                gui::FromUi::OpenProject(mut o) => {
                    // Everything stops first: the clips change under the
                    // transport, and held notes would hang.
                    fx.push(app::Effect::Time(TimeAction::Transport(Command::Stop)));
                    fx.push(app::Effect::StopSounds);
                    // A set's song ended into silence: the new one sounds.
                    song_end.reset();
                    // A new or opened project arms the notes (rec_arm.rs).
                    app.rec = rec_arm::RecArm::default();
                    out.show(gui::ToUi::Rec(app.rec));
                    // The synths its MIDI routes name (step 11e), from the
                    // I/O setup the screen opened before sending the
                    // project, so a synth only a route plays stays while
                    // MIDI in catches up.
                    // Its patches (live shows C1): the old ones go back in
                    // the box. Every sound stops above, so the table starts
                    // empty.
                    music.patches_stopped();
                    if let Some(p) = o.patches.as_mut() {
                        music.swap_patches(p);
                    }
                    // The keys play its patch 1 (keys as routed without).
                    if !music.pick_patch(Some(0), mpfree_patch::keys::When::Now) {
                        music.pick_patch(None, mpfree_patch::keys::When::Now);
                    }
                    if let Ok(m) = io_model.lock() {
                        app.set_route_synths(
                            midi_in::route_synths_of(&m.setup, music.patch_synths()).0,
                        );
                    }
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
                    // The project's tempo is left from before, not set now.
                    app.tempo_by_hand = false;
                    if let Some(mut t) = o.time.take() {
                        // The metronome is the system's: it stays as it is.
                        (t.metronome, t.click_gain) = (time.metronome, time.click_gain);
                        // Its editing side runs where the last one's did.
                        if time.apart() {
                            *t = time_apart(out, *t);
                        }
                        std::mem::swap(&mut time, &mut *t);
                        // Its cues wait for their audio as the last one's
                        // did; none of the last one's waits any more.
                        time.set_audio_waits(tracks.streams());
                        tracks.forget_waits();
                        needs_rev = 0;
                        // The old engine goes back in the box, to be freed
                        // on the screen thread.
                        o.time = Some(t);
                        out.show(gui::ToUi::Time(Box::new(time.state())));
                        // A project opens without the note editor.
                        fx.extend(app.set_notes_open(time.editing()));
                    }
                    if let Some(s) = o.swing {
                        fx.push(app::Effect::Time(TimeAction::Transport(Command::SetSwing(
                            s,
                        ))));
                    }
                    // The XY pad's layout of yours lets go and its rows go
                    // before the mixer and the mod engine change; the
                    // project's layouts come in, and the one it played
                    // plays once its mixer is in.
                    mix.xy_yours(None);
                    let (song, notes) = xy_song::XySong::parse(o.xy.as_deref().unwrap_or(""));
                    for line in notes {
                        out.note(&format!("project: {line}"));
                    }
                    xy_yours = song;
                    xy_turns.clear();
                    xy_opening = true;
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
                    // The old project's sends end first (their notes with
                    // them); the project's come once its instruments are in.
                    mix.instruments.clear_sends();
                    let loaded_for = |o: &project::Opening, track: usize| {
                        o.instruments
                            .iter()
                            .position(|l| l.as_ref().is_some_and(|l| l.track == track))
                    };
                    // The tracks' own, then the free places' (step 6).
                    for track in 0..instruments::HOLDERS {
                        track_presets.set_index(track, None);
                        if loaded_for(&o, track).is_some() || !mix.instruments.holds(track) {
                            continue;
                        }
                        changed += 1;
                        if !mix.instruments.take_out(track) {
                            out.note(&format!(
                                "project: {} keeps its instrument (its last sound is still ringing out)",
                                holder_says(track)
                            ));
                            // The last project's instrument plays on in
                            // this one: never saved over it (live shows A1).
                            out.left_out(format!(
                                "track {}'s instrument (the last project's was still ringing out)",
                                track + 1
                            ));
                        }
                        app.set_track_sound(track, mix.instruments.held(track));
                    }
                    // The pads play the project's choice once its list is in
                    // (below).
                    mix.instruments.pads_play(None);
                    for track in 0..instruments::HOLDERS {
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
                                    "project: {}: {} not put in ({why}); it keeps what it had",
                                    holder_says(track),
                                    held.name
                                ));
                                out.left_out(format!("{} on track {}", held.name, track + 1));
                            }
                        }
                        app.set_track_sound(track, mix.instruments.held(track));
                    }
                    if changed > 0 {
                        out.note(&format!(
                            "project: {playing} tracks play their own instrument, the rest the kit"
                        ));
                    }
                    for line in open_sends(&mut mix.instruments, &o.sends) {
                        out.note(&line);
                    }
                    app.set_track_sends(&sends_now(&mix.instruments));
                    // What each track plays: the project's picks, and kit 1
                    // on every other track. The pads follow the armed
                    // track's.
                    track_kits = sampler::TrackKits::new(drum_kits);
                    for &(track, kit) in &o.track_kits {
                        let entry = kit.unwrap_or(track_kits.synth());
                        if kit.is_some_and(|k| k >= drum_kits) || !track_kits.set(track, entry) {
                            out.note(&format!(
                                "project: track {} plays kit 1 (its kit {} isn't here: {drum_kits} kits)",
                                track + 1,
                                entry + 1
                            ));
                        }
                    }
                    kit_to = Some(track_kits.entry(app.track()));
                    // A project saved before pad mutes were a track's had
                    // kit 1's A1-A16 muted in the mixer: each track that
                    // plays kit 1 takes them (step 10).
                    if let Some(line) = carry_pad_mutes(&mut mix, &track_kits, &mut time) {
                        out.note(&line);
                    }
                    // The project's list of instruments; the old one goes
                    // back in the box, to be freed on the screen thread.
                    // Every kit place here has an id: one the project
                    // hadn't got gets a new one, and a kit's past the
                    // places here is gone (a track that played it was
                    // said above).
                    std::mem::swap(&mut listed, &mut o.instrument_list);
                    let gone = listed.kit_ids().listed_from(drum_kits);
                    if gone > 0 {
                        out.note(&format!(
                            "project: {gone} of its kits aren't here ({drum_kits} kits), so they are gone from its list of instruments"
                        ));
                    }
                    listed.follow_kits(drum_kits);
                    out.show(gui::ToUi::KitIds(Box::new(listed.kit_ids())));
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
                        ask_plugin_presets(&mut mix);
                        out.show(gui::ToUi::Synth(synth_tab(
                            &synth,
                            &mix,
                            app.track(),
                            &track_presets,
                        )));
                    }
                    // The arps: the project's, or every one at its default.
                    let (notes, arp_fx) = app.open_arp(o.arp.as_deref());
                    for line in notes {
                        out.note(&line);
                    }
                    fx.extend(arp_fx);
                    // The Perform set: the project's, or a new project's.
                    let (notes, perform_fx) = app.open_perform(o.perform.as_deref());
                    for line in notes {
                        out.note(&line);
                    }
                    fx.extend(perform_fx);
                    // What the pads play: the project's, or the track, once
                    // the app has the project's list of instruments.
                    app.forget_copies();
                    app.open_list(&listed);
                    list_changed = false;
                    let (notes, pads_fx) = app.open_pads(o.pads.as_deref());
                    for line in notes {
                        out.note(&line);
                    }
                    fx.extend(pads_fx);
                    // The instruments' own tunings, after the synth part
                    // and the list (tunings step T5).
                    let (notes, tunings_fx) = app.open_tunings(o.tunings.as_deref());
                    for line in notes {
                        out.note(&line);
                    }
                    fx.extend(tunings_fx);
                    if o.clear_loops {
                        out.note(&looper.clear_all());
                    }
                    if o.clear_audio {
                        // Recordings end; the tracks empty once they have,
                        // each engine track for the project's audio track
                        // it plays from now on.
                        tracks.bind(o.audio_slots);
                        tracks.clear();
                    }
                    for line in o.notes.drain(..) {
                        out.note(&line);
                    }
                    out.note(&format!("project: {} is in", o.what));
                    // The replaced engines go back to the screen thread to
                    // be freed there (live shows A2). The screen never has
                    // more opens on their way than there are slots, so one
                    // is always free.
                    match project_back.iter_mut().find(|b| b.is_none()) {
                        Some(slot) => *slot = Some(gui::ToUi::ProjectBack(o)),
                        None => {
                            debug_assert!(false, "more opens on their way than slots");
                            drop(o);
                            out.note("project: more opens going back than slots; one freed on the audio thread (not expected)");
                        }
                    }
                }
                // A plugin installed from BROWSE's Plugins place: the Synth
                // tab offers the rack's plugin synths again, and their skins.
                gui::FromUi::PluginsChanged => {
                    app.plugin_synths = found_plugins
                        .instruments()
                        .map(|e| (e.node_id, e.name))
                        .collect();
                    out.show(gui::ToUi::PluginFolders(skin_page::folders(&found_plugins)));
                    out.note(&format!(
                        "plugins: {} offered after an install",
                        found_plugins.entries().len()
                    ));
                }
                // All four PAD BANK buttons: the picture of the screen goes to
                // the store (files/screens/).
                gui::FromUi::ScreenPicture(png) => {
                    let line = screen_picture::save(&store, &mut picture_names, png);
                    out.note(&line);
                    out.show(gui::ToUi::Tell(line));
                }
                // System status's Save a report: to the store
                // (files/reports/).
                gui::FromUi::SaveReport(text) => {
                    let line = status_report::save(&store, &mut report_names, text);
                    out.note(&line);
                    out.show(gui::ToUi::Tell(line));
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
                gui::FromUi::SampleGain(db) => rec_gain = 10f32.powf(db / 20.0),
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
                // The preset picker the Synth tab's title opens (presets.rs):
                // the selected track's own synth (or the one it plays of
                // another track's), else the synth on the pads.
                gui::FromUi::Preset(ask) => {
                    if !mix.instruments.plays(tab_who(&app, &mix.instruments))
                        && ask.changes_sound()
                    {
                        if let Some(line) = param_log.flush(&synth) {
                            out.note(&line);
                        }
                        // The turns were of the old sound.
                        synth_turns.clear();
                    }
                    let who = tab_who(&app, &mix.instruments);
                    let track = instrument_track(&mix.instruments, who);
                    track_presets.act(ask, track, &mut synth, &mut mix, out);
                }
                // The preset picker's New track (step 4c): the new track's
                // own synth, made as a preset's sound; it goes in once made.
                gui::FromUi::NewTrackSound {
                    track,
                    engine,
                    give,
                } => {
                    if let Some(held) =
                        track_presets.give(track, &engine, give, &synth, &mut mix, out)
                    {
                        app.set_track_sound(track, Some(held));
                        app.mark_project_edit();
                    }
                }
                // A plugin synth's own preset: loaded into the running
                // plugin on the builder thread (the plugin's own code
                // reads the file); the news comes back below.
                gui::FromUi::Synth(SynthAction::Preset(k))
                    if mix
                        .instruments
                        .held(instrument_track(
                            &mix.instruments,
                            tab_who(&app, &mix.instruments),
                        ))
                        .is_some_and(|h| plugins::Plugins::is_plugin(h.id)) =>
                {
                    let who = tab_who(&app, &mix.instruments);
                    let track = instrument_track(&mix.instruments, who);
                    if !mix.load_plugin_preset(track, k) {
                        let s = format!(
                            "Track {}: preset {} wasn't asked for: the plugin's presets aren't listed yet, or the builder is busy; try again in a moment",
                            track + 1,
                            k + 1
                        );
                        out.note(&s);
                        out.show(gui::ToUi::Tell(s));
                        out.show(gui::ToUi::Synth(synth_tab(
                            &synth,
                            &mix,
                            track,
                            &track_presets,
                        )));
                    }
                }
                // A preset on a track's own synth (or the one it plays of
                // another track's): a new copy with the preset's sound goes
                // in, and the old one rings out.
                gui::FromUi::Synth(SynthAction::Preset(k))
                    if mix.instruments.plays(tab_who(&app, &mix.instruments)) =>
                {
                    let who = tab_who(&app, &mix.instruments);
                    let track = instrument_track(&mix.instruments, who);
                    let held = mix.instruments.held(track);
                    let engine = held.and_then(|h| synth::Engine::from_node_id(h.id));
                    match (held, engine.and_then(|e| synth.preset_values(e, k))) {
                        (Some(h), Some((name, extras, values))) => {
                            let job = instrument_job(&synth, track, h.id, Some((extras, values)));
                            if mix.build_instrument(job) {
                                track_presets.set_index(track, Some(k));
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
        // The screen's XY pad as it stands: a lift whose action never came
        // lets go here (xy_slot.rs).
        if mix.screen_xy(xy.read()) {
            out.note(&format!(
                "XY pad: let go from the screen's state, its lift never came ({} so far)",
                mix.xy_found_up()
            ));
        }
        // MIDI Learn listens only while the chain editor's learn is armed:
        // Back or closing the editor stops it too. The I/O view's Learn
        // again stays armed until a control takes it or its Cancel.
        if music.armed().is_some() && !panels.learning() && !music.waits() {
            music.disarm();
        }
        if let Some(line) = param_log.poll(&synth, t_read) {
            out.note(&line);
        }
        let turns_ready = (
            !synth_turns.done().is_empty(),
            !mix_turns.done().is_empty(),
            !xy_turns.done().is_empty(),
        );
        if turns_ready != turns_told {
            turns_told = turns_ready;
            out.show(gui::ToUi::TurnsReady {
                synth: turns_ready.0,
                mixer: turns_ready.1,
                xy: turns_ready.2,
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
        for b in project_back.iter_mut() {
            if let Some(m) = b.take() {
                *b = out.try_show(m).err();
            }
        }
        parts.lap(parts::SCREEN_ACTIONS);
        ports.show_pressure = app.settings.show_pressure;
        // The MIDI monitor: where keys go, and the debug sources while the
        // developer tools are on.
        music.set_armed(
            app.track() as u8,
            if pads_preview.is_some() {
                midi_in::KeysPlay::PadsSynth
            } else {
                keys_play(
                    synth_on,
                    app.keys_play_notes(),
                    mix.instruments.plays(app.track()),
                    player.kit_keys(player.kit()),
                )
            },
        );
        music.feed.debug = app.settings.debug_mode;
        // Prefs' "Send MIDI and CV out" turned on or off (and its kept
        // value at the first period).
        if sends != Some(app.settings.sends_midi) {
            sends = Some(app.settings.sends_midi);
            apply_sends(
                app.settings.sends_midi,
                &mut midi_out,
                &mut cv_out,
                ports.private.as_ref(),
                started,
                out,
            );
        }
        ports.tap_on = app.settings.debug_mode;
        let mut got = ports.poll(out, &mut buf)?;
        // A scripted run's steps go in beside what the controller and MIDI
        // In sent (script.rs): from here on, the app's own code.
        if let Some(run) = script_run.as_mut() {
            for step in run.due(i) {
                out.note(&script::said(step));
                let event = match &step.act {
                    script::Act::Button { name, down } => match script::button_note(&app, name) {
                        Some(note) => Some(ControlEvent::Button {
                            note,
                            pressed: *down,
                        }),
                        None => {
                            out.note(&format!(
                                "script: line {}: {name} has no button number; not pressed",
                                step.line
                            ));
                            None
                        }
                    },
                    script::Act::Pad {
                        pad,
                        velocity,
                        down: true,
                    } => Some(ControlEvent::PadDown {
                        pad: *pad,
                        velocity: *velocity,
                    }),
                    script::Act::Pad { pad, .. } => Some(ControlEvent::PadUp { pad: *pad }),
                    script::Act::QLink { knob, delta } => Some(ControlEvent::QLinkTurn {
                        knob: *knob,
                        delta: *delta,
                    }),
                    script::Act::Wheel { delta } => Some(ControlEvent::DataWheel { delta: *delta }),
                    script::Act::Gear(bytes) => {
                        got.push(Incoming {
                            port: MUSIC_CABLE as usize,
                            bytes: bytes.clone(),
                            event: None,
                            music: true,
                        });
                        None
                    }
                    script::Act::Tap { widget, id, label } => {
                        out.show(gui::ToUi::ScriptTap(gui::ScriptTap {
                            widget: widget.clone(),
                            id: id.clone(),
                            label: label.clone(),
                            line: step.line,
                        }));
                        None
                    }
                    script::Act::Mark(_) => {
                        status_now = true;
                        None
                    }
                    script::Act::Expect { .. } => None,
                };
                if let Some(event) = event {
                    got.push(Incoming {
                        port: MUSIC_CABLE as usize,
                        bytes: Vec::new(),
                        event: Some(event),
                        music: false,
                    });
                }
            }
        }
        if i == script_end {
            out.note("script: done");
            status_now = true;
        }
        let mt = ports.t();
        got.retain(|m| {
            if m.music {
                music.bytes(midi_in::DIN, &m.bytes, mt, &mut keys, &mut |l| out.midi(&l));
            }
            !m.music
        });
        for m in &got {
            // Pad pressure as the MIDI log has it (MENU on the Dev tab).
            let pressure = matches!(m.event, Some(ControlEvent::PadPressure { .. }));
            if let Some(ev) = m.event.filter(|_| app.settings.show_pressure || !pressure) {
                let mut bytes = [0u8; 3];
                let n = m.bytes.len().min(3);
                bytes[..n].copy_from_slice(&m.bytes[..n]);
                music.feed.mpc(Dir::In, mt, Mpc::Control { ev, bytes });
            }
        }
        for (t, m) in ports.tapped.drain(..) {
            music.feed.mpc(Dir::Out, t, m);
        }
        usb.poll(&mut music, &mut midi_out, &mut buf, mt, &mut keys, out);
        music.tick(mt, &mut |l| out.midi(&l));
        let mut stopped = [false; mpfree_midi::MAX_PORTS];
        midi_out.tick(&mut |l| out.midi(&l), &mut |p| {
            if let Some(s) = stopped.get_mut(usize::from(p.0)) {
                *s = true;
            }
        });
        for (i, was) in out_full.iter_mut().enumerate() {
            let port = mpfree_midi::PortId(i as u8);
            if stopped[i] {
                music.feed.event(mt, Event::OutStopped { port });
            }
            let full = midi_out.dropped_full(port);
            if full > *was {
                let n = u32::try_from(full - *was).unwrap_or(u32::MAX);
                music.feed.event(mt, Event::Dropped { port, n });
                *was = full;
            }
        }
        cv_out.tick(&mut |l| out.midi(&l));
        // The Sample tab's auto-sample of outside gear plays through the
        // jacks while they are open and sending: the screen hears when
        // that comes or goes (allocates only then).
        if let Some(g) = midi_out.gear_changed() {
            out.show(gui::ToUi::Gear(g));
        }
        music.sync(&io_model, mt);
        // Controller mode on the screen: its chip and Q-Link cards, when
        // they change (or MIDI out goes on or off).
        if controller.take_changed() || controller_sending != midi_out.sending() {
            controller_sending = midi_out.sending();
            out.show(gui::ToUi::Controller(controller_show(
                &controller,
                controller_sending,
            )));
        }
        if let Some(plan) = music.take_controller() {
            let was = controller.on();
            controller.set(plan, &mut |to, m| {
                midi_out.controller(to, m);
            });
            if controller.on() != was {
                // The PAD BANK lights: the controller's bank, or the kit's
                // again.
                bank_lights = true;
                out.midi(if controller.on() {
                    "controller mode on: the pads and Q-Links play outside gear"
                } else {
                    "controller mode off: the pads and Q-Links are MPFree's again"
                });
            }
        }
        // The Monitor's "Send note-offs to MIDI out": only the notes MPFree
        // left on there, as STOP's panic sends them.
        if music.take_note_offs() {
            midi_out.all_off(0);
            out.midi("midi out: note-offs sent for the notes still on (the Monitor asked)");
        }
        // MIDI Learn (M5): a control that took the armed knob ends the
        // chain editor's learn; a learned control sets its parameter as a
        // chain editor knob does.
        let recording = time.info().state == mpfree_transport::State::Recording;
        mix.set_recording(recording);
        mix.set_xy_track(app.track());
        if xy_opening && !mix.opening() {
            xy_opening = false;
            if let Some(c) = xy_yours.opening() {
                mix.xy_done(xy_song::Done {
                    load: Some(Some(c)),
                    hold: None,
                });
            }
            let pad_now = |owner, id: u16| {
                let (kit, pad) = learned_kit(&listed, &track_kits, owner)?;
                pad_describe(&player, kit, pad, id)
            };
            out.show(xy_view(
                &mut xy_yours,
                &mix,
                &synth,
                &app,
                &listed,
                &pad_now,
                &time,
            ));
        }
        // The XY pad's hand on a synth's, a pad's and an arp's own
        // settings: this period's moves, glides back and Stay go to their
        // owners before they play (`xy_own.rs`); Stay's turns join the
        // lift's UNDO step.
        if mix.xy_own().holding() > 0 {
            let mut hands = mix.take_xy_own();
            let none = mpfree_play::mpfree_param::ParamAddr::new(
                mpfree_play::mpfree_param::Owner::Play,
                0,
            );
            let mut kept = [(none, 0.0, 0.0); mpfree_xy::SLOTS];
            let mut n = 0;
            let mut side = OwnReach {
                synth: &mut synth,
                own: &mut mix.instruments,
                player: Some(&mut player),
                kits: &track_kits,
                listed: &listed,
                pads_learned: &mut pads_learned,
                synth_turned: &mut synth_learned,
                app: Some(&mut app),
                arp_fx: Vec::new(),
            };
            hands.apply(PERIOD as u32, &mut side, |a, before, after| {
                if n < kept.len() {
                    kept[n] = (a, before, after);
                    n += 1;
                }
            });
            fx.extend(side.arp_fx);
            mix.put_xy_own(hands, &kept[..n]);
        }
        // The XY pad's Stay: the settings it kept at a lift, one UNDO step.
        if let Some((key, from, to)) = xy_song::kept_turn(mix.take_xy_kept()) {
            xy_turns.turned(key, from, to, t_read);
        }
        // The XY pad's Set moves record where `xy_rec.rs` says (phase 3).
        {
            let pad_now = |owner, id: u16| {
                let (kit, pad) = learned_kit(&listed, &track_kits, owner)?;
                pad_describe(&player, kit, pad, id)
            };
            xy_record(
                &mut xy_rec,
                &mut mix,
                &synth,
                &app,
                &listed,
                &pad_now,
                &mut time,
            );
        }
        let by_touch = learning_shown.is_some_and(|(_, by)| by == midi_in::ArmedBy::Touch);
        let following = music.leader_name().is_some();
        // A controller pulled out never lets go of what it held: the
        // looper's learned keys and the Clips grid's presses let go,
        // clearing nothing, and a transport button it held down (REC)
        // comes up.
        if music.take_unplugged() {
            for id in remote.let_go() {
                fx.extend(app.remote_press(id, false, t));
            }
            // And the Perform pads its keys held let go, so no throw
            // stays on.
            for action in perform_keys.let_go() {
                fx.extend(desk::perform_pad_up(&mut app, action));
            }
        }
        // Other gear's MMC and program changes (the Sync section's MMC in
        // and Program change in).
        fx.extend(desk::remote_in(&mut music, &time, &mut app, out));
        for l in music.learning() {
            match l {
                midi_in::Learning::Took { .. } if panels.learning() => {
                    panels.learned();
                    out.show(gui::ToUi::ChainLearnDone);
                }
                // SHIFT + a touch armed it: the line in place of the hint
                // says what the control moves now.
                midi_in::Learning::Took { target, control } if by_touch => {
                    let name = learn_label(&mix, &synth, &time, &listed, target);
                    out.show(gui::ToUi::Tell(format!("Learned: {control} moves {name}")));
                }
                // The I/O view's Learn again: its section says so.
                midi_in::Learning::Took { .. } => {}
                // The transport: PLAY, STOP and the rest as the MPC's own
                // buttons, and the tempo as Play's.
                midi_in::Learning::Set(w)
                    if w.target.owner == mpfree_play::mpfree_param::Owner::Transport =>
                {
                    let now = remote::Now {
                        rolling: app.rolling(),
                        // While song mode plays the song's tempo points,
                        // the point's in effect at the playhead.
                        bpm: time.tempo_here().unwrap_or(app.bpm()),
                        ..remote::Now::default()
                    };
                    match remote.write(w.target.param, w.value, now) {
                        remote::Act::Nothing => {}
                        remote::Act::Press(id, down) => fx.extend(app.remote_press(id, down, t)),
                        remote::Act::Tap(id) => {
                            fx.extend(app.remote_press(id, true, t));
                            fx.extend(app.remote_press(id, false, t));
                        }
                        // Following another device's clock, its tempo wins.
                        remote::Act::Tempo(_) if following => {
                            if !remote_tempo_waits {
                                remote_tempo_waits = true;
                                out.midi(
                                    "MIDI Learn: the tempo follows another device's clock, \
                                     so a learned tempo control does nothing until it stops",
                                );
                            }
                        }
                        // While song mode plays the song's tempo points,
                        // the point in effect at the playhead takes it
                        // (one undo while the knob turns).
                        remote::Act::Tempo(bpm) if time.song_tempo_plays() => {
                            remote_tempo_waits = false;
                            if let Err(e) =
                                time.act(TimeAction::SongTempo(TempoEdit::AtPlayhead(bpm)))
                            {
                                out.midi(&format!("MIDI Learn: {e}"));
                            }
                        }
                        remote::Act::Tempo(bpm) => {
                            remote_tempo_waits = false;
                            // Without set_tempo's log line: a knob would
                            // fill the log.
                            fx.extend(
                                app.set_tempo_by_hand(bpm)
                                    .into_iter()
                                    .filter(|e| !matches!(e, app::Effect::Note(_))),
                            );
                        }
                    }
                }
                // The Clips grid (sequencer S5): a press does what a tap on
                // the grid does now; a mute or solo goes as TRACK MUTE's
                // pads do, on the launch line in Perform.
                midi_in::Learning::Set(w)
                    if w.target.owner == mpfree_play::mpfree_param::Owner::Clips =>
                {
                    let (muted, soloed) = time.mute_bits();
                    let now = remote::Now {
                        muted,
                        soloed,
                        ..remote::Now::default()
                    };
                    match remote.clips_write(w.target.param, w.value, now) {
                        remote::ClipsAct::Nothing => {}
                        remote::ClipsAct::Mute { track, solo, .. } => {
                            let wait = time.perform();
                            let say = time::Say::Note {
                                ok: "",
                                err: "track mute: ",
                            };
                            if let Some(r) = time.mute_or_solo_then(track, wait, solo, say.clone())
                            {
                                fx.extend(time_said(out, say, r));
                            }
                        }
                        remote::ClipsAct::Press(c) => {
                            let st = time.state();
                            match mpfree_time_ui::view::clips_press(&st, c) {
                                Some(a) => fx.extend(time_action(
                                    out,
                                    &mut time,
                                    &mut tracks,
                                    &looper,
                                    &mut app,
                                    &mut mix,
                                    a,
                                )),
                                None => out.note(&format!(
                                    "MIDI Learn: {} isn't on the Clips grid",
                                    clips_label(time.project(), w.target.param)
                                )),
                            }
                        }
                    }
                }
                // A track's arp: as its Q-Links turn it, and the panel
                // and the band follow when it is the armed track's.
                midi_in::Learning::Set(w)
                    if matches!(w.target.owner, mpfree_play::mpfree_param::Owner::Arp(_)) =>
                {
                    let mpfree_play::mpfree_param::Owner::Arp(t) = w.target.owner else {
                        continue;
                    };
                    let t = usize::from(t);
                    match app.arp_param(t, w.target.param) {
                        Some((info, _)) => {
                            fx.extend(app.set_arp_param(t, w.target.param, info.from_unit(w.value)))
                        }
                        None => {
                            learn_misses += 1;
                            if learn_misses == 1 {
                                out.midi(&format!(
                                    "MIDI Learn: {} isn't one of the arp's settings \
                                     (counted from here)",
                                    w.target
                                ));
                            }
                        }
                    }
                }
                // A kit pad's own setting, on its track's kit or the kit
                // it names (sampler step 16): the screen sets it on the kit
                // as the Play tab's knob does (one undo step a turn), and
                // its new view comes back to the sampler. The last value
                // for each goes, once a pass.
                midi_in::Learning::Set(w)
                    if matches!(
                        w.target.owner,
                        mpfree_play::mpfree_param::Owner::Pad(..)
                            | mpfree_play::mpfree_param::Owner::KitPad(..)
                    ) =>
                {
                    let to = learned_kit(&listed, &track_kits, w.target.owner)
                        .and_then(|(kit, pad)| {
                            Some((pad_describe(&player, kit, pad, w.target.param)?, (kit, pad)))
                        })
                        .zip(sampler::PadParam::of(w.target.param));
                    match to {
                        Some((((info, _), (kit, pad)), field)) => {
                            let value = info.from_unit(w.value);
                            let rec = learned_on_armed(
                                w.target,
                                time.armed(),
                                &track_kits,
                                &mix.instruments,
                                &listed,
                            );
                            record_learned(&mut time, rec, value, recording);
                            match pads_learned
                                .iter_mut()
                                .find(|(k, p, f, _)| (*k, *p, *f) == (kit, pad, field))
                            {
                                Some(set) => set.3 = value,
                                None => pads_learned.push((kit, pad, field, value)),
                            }
                        }
                        None => {
                            learn_misses += 1;
                            if learn_misses == 1 {
                                out.midi(&format!(
                                    "MIDI Learn: {} is no pad with a sound on that kit \
                                     (counted from here)",
                                    w.target
                                ));
                            }
                        }
                    }
                }
                // Next and Previous patch (the Play card, live shows C1): a
                // press, once, stepping the song's patches when the pass is
                // done.
                midi_in::Learning::Set(w)
                    if w.target.owner == mpfree_play::mpfree_param::Owner::Play
                        && mpfree_play::mpfree_param::play::patch_step(w.target.param)
                            .is_some() =>
                {
                    if let Some(step) = remote.patch_write(w.target.param, w.value) {
                        if patch_steps.len() < patch_steps.capacity() {
                            patch_steps.push(step);
                        }
                    }
                }
                // Play's own controls (the Learn section's Play card): the
                // last value each was given goes once the pass is done.
                midi_in::Learning::Set(w)
                    if w.target.owner == mpfree_play::mpfree_param::Owner::Play =>
                {
                    play_moves.set(w.target.param, w.value);
                }
                // The looper (its footswitch): a loop's key does what its
                // pad does, a mute mutes or lets the loop play, and the
                // presses are the Looper tab's buttons, on the picked loop.
                midi_in::Learning::Set(w)
                    if w.target.owner == mpfree_play::mpfree_param::Owner::Looper =>
                {
                    let now = desk::looper_now(&looper);
                    let act = remote.looper_write(w.target.param, w.value, now, t);
                    fx.extend(desk::looper_remote(&mut app, &time, &looper, out, act));
                }
                // A set's control (the Learn section's Show card): a press
                // as it goes down, once, for the screen's set player (with
                // no set open, its hint line says so).
                midi_in::Learning::Set(w)
                    if w.target.owner == mpfree_play::mpfree_param::Owner::Show =>
                {
                    if let Some(id) = remote.show_write(w.target.param, w.value) {
                        if !app.set_open() {
                            out.show(gui::ToUi::Tell(remote::NO_SET.to_string()));
                        } else if let Some(e) = app.show_press(id) {
                            fx.extend(e);
                        } else {
                            out.show(gui::ToUi::ShowPress(id));
                        }
                    }
                }
                // A Perform pad (Learn a key): played as a hit on it plays
                // it, whatever the pads show, and let go of what it pressed.
                midi_in::Learning::Set(w)
                    if w.target.owner == mpfree_play::mpfree_param::Owner::Perform =>
                {
                    let set = app.perform_set();
                    let act = perform_keys.write(w.target.param, w.value, t, |page, pad| {
                        set.page(page).action(pad)
                    });
                    let drums = kit_drums(&kits, player.kit());
                    fx.extend(desk::perform_remote(
                        &mut app, &time, &looper, out, act, drums,
                    ));
                }
                // A synth's knob: as the Synth tab turns it, and the tab
                // follows (a few times a second at most).
                midi_in::Learning::Set(w)
                    if matches!(
                        w.target.owner,
                        mpfree_play::mpfree_param::Owner::Inst(_)
                            | mpfree_play::mpfree_param::Owner::Sound(_)
                    ) =>
                {
                    match learned_synth(&mut synth, &mut mix, &listed, w) {
                        Some(value) => {
                            synth_learned = true;
                            let rec = learned_on_armed(
                                w.target,
                                time.armed(),
                                &track_kits,
                                &mix.instruments,
                                &listed,
                            );
                            record_learned(&mut time, rec, value, recording);
                        }
                        None => {
                            learn_misses += 1;
                            if learn_misses == 1 {
                                out.midi(&format!(
                                    "MIDI Learn: {} isn't a parameter of that synth \
                                     (counted from here)",
                                    w.target
                                ));
                            }
                        }
                    }
                }
                midi_in::Learning::Set(w) => {
                    if let Some(value) = learned_set(&mut panels, &mut mix, w, recording) {
                        record_learned(&mut time, w.target, value, recording);
                    } else {
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
        if !pads_learned.is_empty() {
            out.show(gui::ToUi::PadsLearned(std::mem::take(&mut pads_learned)));
        }
        // Play's Swing, Drums filter and XY pad, once a pass.
        let (play_fx, xy_presses) = desk::play_remote(&time, &mut mix, out, &mut play_moves);
        fx.extend(play_fx);
        if xy_presses != remote::XyPresses::default() {
            let pad_now = |owner, id: u16| {
                let (kit, pad) = learned_kit(&listed, &track_kits, owner)?;
                pad_describe(&player, kit, pad, id)
            };
            let mut xy = XyHere {
                song: &mut xy_yours,
                mix: &mut mix,
                synth: &synth,
                app: &app,
                listed: &listed,
                pads: &pad_now,
                time: &time,
            };
            let (lines, pick) = xy.pressed(xy_presses);
            for line in lines {
                out.note(&format!("xy: {line}"));
                out.show(gui::ToUi::Tell(line));
            }
            out.show(xy.view());
            if let Some(e) = pick {
                fx.extend(app.set_xy_effect(e));
            }
        }
        // Next and Previous patch: the keys' new patch shows on KEYS PLAY;
        // nowhere to go, the hint line says why.
        for (d, hands_up) in patch_steps.drain(..) {
            if let Some(why) = desk::step_patch(&mut music, d, hands_up) {
                out.show(gui::ToUi::Tell(why));
            }
        }
        if synth_learned && mt >= synth_learned_at {
            synth_learned = false;
            synth_learned_at = mt + 0.05;
            out.show(gui::ToUi::SynthValues(synth_tab_values(
                &synth,
                &mix,
                tab_who(&app, &mix.instruments),
            )));
        }
        // The gear lanes' pink ~ (LFO to CC): what the mod matrix moves of
        // the tracks' MIDI out, four times a second, sent when it changes
        // (a row added or taken off, a project loaded, an undo).
        if mt >= gear_moved_at {
            gear_moved_at = mt + 0.25;
            mix.modu.midi_moved_into(&mut gear_moved_now);
            if gear_moved_now != gear_moved {
                gear_moved.clone_from(&gear_moved_now);
                out.show(gui::ToUi::GearMoved(gear_moved.clone()));
            }
        }
        // Where each learned control's parameter stands, when something
        // else moved it: an endless encoder steps on from there.
        let (muted, soloed) = time.mute_bits();
        let remote_now = remote::Now {
            rolling: app.rolling(),
            bpm: app.bpm(),
            muted,
            soloed,
            picked: looper.picked(),
            loops_muted: looper.mute_bits(),
            ..desk::play_now(&time, &mix)
        };
        let pad_now = |owner, id: u16| {
            let (kit, pad) = learned_kit(&listed, &track_kits, owner)?;
            pad_describe(&player, kit, pad, id)
        };
        music.observe_learned(mt, |addr| {
            learn_describe(&mix, &synth, &app, &listed, &pad_now, remote_now, addr)
        });
        // The I/O view names what each learned control moves, whenever
        // the project's learned controls change.
        if let Ok(mut m) = io_model.try_lock() {
            let mpfree_midi_ui::IoModel {
                setup, live, gen, ..
            } = &mut *m;
            if learn_names_gen != Some(*gen) {
                learn_names_gen = Some(*gen);
                live.learn_names = setup
                    .all_learned()
                    .map(|(_, l)| learn_label(&mix, &synth, &time, &listed, l.target))
                    .collect();
                // The ends of each one's range, in its units (the Learn
                // section's card).
                live.learn_live = setup
                    .all_learned()
                    .map(|(_, l)| {
                        match learn_describe(
                            &mix, &synth, &app, &listed, &pad_now, remote_now, l.target,
                        ) {
                            Some((info, _)) => mpfree_midi_ui::LearnLive {
                                now: None,
                                lo_text: mpfree_mod::text::value_text(&info, info.from_unit(l.lo)),
                                hi_text: mpfree_mod::text::value_text(&info, info.from_unit(l.hi)),
                            },
                            None => mpfree_midi_ui::LearnLive::default(),
                        }
                    })
                    .collect();
                learn_now_at = 0.0;
            }
            // The project's tracks by name, for the sections that name
            // them (the MIDI section, Routing, CV and the Learn section's
            // Clips and scenes card): twice a second, allocating only when
            // a track was renamed.
            if mt >= track_names_at {
                track_names_at = mt + 0.5;
                io_track_names(&mut live.tracks, &time.project().tracks);
                io_route_synths(&mut live.synths, &listed, &mix);
            }
            // Where each one's parameter stands, a few times a second, in
            // place (no allocation).
            if mt >= learn_now_at {
                learn_now_at = mt + 0.25;
                for ((_, l), v) in setup.all_learned().zip(live.learn_live.iter_mut()) {
                    v.now =
                        learn_describe(&mix, &synth, &app, &listed, &pad_now, remote_now, l.target)
                            .map(|(i, x)| i.to_unit(x));
                }
            }
            // What Learn waits to give a control, named: on the Learn
            // section's banner, and on the hint line when SHIFT + a touch
            // armed it.
            let armed = music.armed().zip(music.armed_by());
            if armed != learning_shown {
                learning_shown = armed;
                live.learning = armed.map(|(a, _)| learn_label(&mix, &synth, &time, &listed, a));
                let touched = armed.filter(|(_, by)| *by == midi_in::ArmedBy::Touch);
                out.show(gui::ToUi::Learning(touched.and(live.learning.clone())));
            }
        }
        // Keys a route sends to a track (the I/O view's Routing): the
        // armed track's join the keys below; another track's play it at
        // once, after them.
        let armed = app.track();
        for (track, k) in music.take_routed() {
            // A route that names a synth (step 11e): its row plays it, and
            // the track recording it takes the key.
            if let Some(j) = track.checked_sub(midi_in::SYNTH_ROUTES) {
                let j = usize::from(j);
                route_synths.key(j, k, &mut synth, &mut mix.instruments);
                // A route that names a kit plays it (sampler step 15f).
                play_route_kit_key(
                    &mut route_synths,
                    j,
                    k,
                    armed as u8,
                    &mut midi_out,
                    (&mut mix.pad_synths, &mut synth),
                    &mut player,
                );
                route_synths.rec_key(j, k, &mut time, &mix.instruments, &synth);
                continue;
            }
            if routed_held.direct(track, k, usize::from(track) == armed) {
                routed_keys.push((track, k));
            } else {
                keys.push(k);
            }
        }
        // What the song's patch plays (live shows C1): at once, as a
        // route's keys to a track that isn't armed are, whichever track
        // that is (REC with zones is C3), and a synth no track has on its
        // route row.
        for (track, k) in music.take_patched() {
            if let Some(j) = track.checked_sub(midi_in::SYNTH_ROUTES) {
                route_synths.key(usize::from(j), k, &mut synth, &mut mix.instruments);
                continue;
            }
            routed_keys.push((track, k));
        }
        // The patch screens follow what the keys do, told when it changes.
        let at = music.keys_at();
        if at != keys_told {
            keys_told = at;
            out.show(gui::ToUi::Keys(Box::new(music.keys_now())));
        }
        // Keys play the armed track: by note while the pads press notes (a
        // synth pad layout, or the track's own synth's pad map, step 7); on
        // a kit, as its Keys play sends each note (sampler step 15): the
        // kit pad that sends it (`profile::kit_pad_of_note`, A1 to H16
        // whatever bank the pads show) or the keygroups that hold it; else
        // as the pad the MPC gives that note.
        let key_notes = app.keys_play_notes();
        // The armed track's kit played as notes (sampler step 15c, part
        // 3): its keys press notes into its arp as a synth's do, but the
        // wheel bends its keygroups and the shared synth takes nothing.
        let kit_notes_armed =
            !mix.instruments.plays(armed) && kit_notes.of_track(&track_kits, armed).is_some();
        let kit_keys = matches!(
            keys_play(
                synth_on,
                key_notes,
                mix.instruments.plays(armed),
                player.kit_keys(player.kit()),
            ),
            midi_in::KeysPlay::KitPads(_)
        );
        // The kit the armed track's keys play, which the kit layers leave
        // to it (sampler step 15f); none while the pads' synth takes the
        // keys (`KeysOnPads`).
        let armed_kit = (kit_keys || kit_notes_armed)
            .then(|| player.kit())
            .filter(|_| !matches!(pads_preview, Some(PadsPreview::Synth | PadsPreview::Host)));
        if armed != expr_track || pads_preview != expr_pads {
            // Its gear's pedals come up with its expression; its
            // instrument's and the synth's here (or the pads' synth's, as
            // the pads move on), so nothing they held hangs.
            midi_out.rest_expression(expr_track);
            cv_out.rest_expression(expr_track);
            for ctl in pedals_down.lift() {
                let pads_had =
                    KeysOnPads::control(expr_pads, ctl, 0.0, &mut synth, &mut mix.instruments);
                if !pads_had && !mix.instruments.control(expr_track, ctl, 0.0, 0) {
                    synth.control(ctl, 0.0, 0);
                }
            }
            expr_track = armed;
            expr_pads = pads_preview;
        }
        for k in keys.drain(..) {
            // While the pads play a synth on their own, the keys play it
            // too, by note, and the armed track hears none of them (step
            // 7b); a key held there lets go there.
            let shape = |v| app.velocity(v);
            // The pads' layers (step 11a) play each key too, by note,
            // whichever way the first plays it.
            pad_layers.key(k, shape, &mut synth, &mut mix.instruments);
            // A kit layer plays each key too, as its kit's Keys play sends
            // it (sampler step 15f).
            play_kit_layer_key(
                &mut pad_layers,
                k,
                shape,
                armed_kit,
                armed as u8,
                &mut midi_out,
                (&mut mix.pad_synths, &mut synth),
                &mut player,
            );
            // And into each track recording a layer (step 11c).
            let rv = match k {
                midi_in::Key::Down { velocity, .. } => app.layer_rec_velocity(velocity),
                _ => None,
            };
            pad_layers.rec_key(k, rv, &mut time, &mix.instruments, &synth);
            if keys_on_pads.key(k, pads_preview, shape, &mut synth, &mut mix.instruments) {
                if let midi_in::Key::Control { ctl, value } = k {
                    pedals_down.key(ctl, value);
                }
                // Into the track recording what the pads play (sampler
                // step 15c).
                pads_rec.key(k, rv, &mut time, &mix.instruments, &synth);
                continue;
            }
            // The armed track's arp hears the keys' pressure and
            // controllers: its Modifier follows the one it is set to (the
            // sound still gets them, below), and with Prefs' Sustain pedal
            // latches on, the pedal latches the arp while it plays instead
            // of holding the sound's notes.
            match k {
                midi_in::Key::Pressure { note, value } => app.key_pressure(note, value),
                midi_in::Key::Control { ctl, value } => {
                    if let Some(latch) = app.arp_pedal(ctl, value) {
                        fx.extend(latch);
                        continue;
                    }
                    app.arp_controller(ctl, value);
                }
                _ => {}
            }
            // Expression out: the armed track's CV outputs, and its gear on
            // MIDI out, take the keys' wheel, pressure and MPE (the keys
            // going down and up say whose count).
            let expr = k.expr_out().filter(|_| key_notes);
            if let Some(i) = expr {
                cv_out.expression(armed as u16, i);
            }
            let to_gear = expr.is_some_and(|i| midi_out.expression(armed as u16, i));
            let pad_event = match k {
                midi_in::Key::Down { note, velocity } if key_notes => {
                    // A key is a note too, for a loop that starts when you
                    // play (with a kit, keys come back as pads, below).
                    let take = app.rec.ready.map(|take| take == rec_arm::Take::Add);
                    out.loop_start(pad_starts_loop(
                        &mut looper,
                        &mut time,
                        take,
                        export.running(),
                    ));
                    app.key_down(note, velocity);
                    // Step pads of the line held: the key goes on them.
                    let p = step_key(&app, &mut step_keys, &mut time, note, Some(velocity));
                    if let Some(p) = p {
                        fx.extend(sound_step(p, out, &mut player, &mut synth, synth_on));
                    }
                    None
                }
                midi_in::Key::Up { note } if key_notes => {
                    app.key_up(note);
                    step_key(&app, &mut step_keys, &mut time, note, None);
                    None
                }
                // Pressure, the wheel and an MPE note's own slide and tone:
                // the armed track's gear (above), else its instrument, else
                // the synth (the notes it holds as they came: a note the
                // MIDI effects moved takes none of its key's).
                midi_in::Key::Pressure { .. }
                | midi_in::Key::Bend { .. }
                | midi_in::Key::Expr { .. }
                    if to_gear =>
                {
                    None
                }
                // A keygroup's key takes no pressure or expression yet.
                midi_in::Key::Pressure { .. } | midi_in::Key::Expr { .. }
                    if key_notes && kit_notes_armed =>
                {
                    None
                }
                midi_in::Key::Pressure { note, value } if key_notes => {
                    if !mix.instruments.note_pressure(armed, note, value) {
                        synth.pressure_note(note, value);
                    }
                    None
                }
                midi_in::Key::Bend { value } if key_notes && kit_notes_armed => {
                    if let Some(kit) = track_kits.kit_of(armed) {
                        player.bend_kit(kit, value);
                    }
                    None
                }
                midi_in::Key::Bend { value } if key_notes => {
                    if !mix.instruments.bend(armed, value) {
                        synth.bend(value);
                        // The keygroups of the kit the pads play bend too
                        // (sampler step 15).
                        if !synth_on {
                            player.bend(value);
                        }
                    }
                    None
                }
                midi_in::Key::Expr { note, kind, value } if key_notes => {
                    if !mix.instruments.note_expression(armed, note, kind, value) {
                        synth.expression_note(note, kind, value);
                    }
                    None
                }
                // A pedal, the mod wheel, breath or a foot controller: the
                // armed track's gear, else its instrument, else the synth.
                // A pedal coming up always goes, so nothing it held hangs
                // after the pads stopped playing notes.
                midi_in::Key::Control { ctl, value }
                    if key_notes || (ctl.is_pedal() && !mpfree_node::Ctl::down(value)) =>
                {
                    pedals_down.key(ctl, value);
                    if midi_out.keys_control(armed as u16, ctl, value) {
                        // The gear's controller, for recording (a pedal is
                        // not followed).
                        if let Some(v) = mpfree_node::Ctl::clamp(value) {
                            expr_rec.control(ctl.cc(), (v * 127.0).round() as u8);
                        }
                    } else if !mix.instruments.control(armed, ctl, value, 0) && !kit_notes_armed {
                        // A kit's keygroups take no pedal.
                        synth.control(ctl, value, 0);
                    }
                    None
                }
                // A kit, as its Keys play sends the key (sampler step 15,
                // `sampler::Player::note_on_to`): the kit pad that plays it
                // goes through the track's arp (and records), as a pad
                // would, but in any bank and never as STEP SEQ's, TRACK
                // MUTE's or the loop keys' pads; a key on the keygroups
                // that no pad plays sounds at once, past the arp and REC
                // (a kit played as notes takes every key through its arp
                // above, step 15c's third part), and is counted. A let-go and pressure
                // go where the key went down.
                midi_in::Key::Down { note, velocity } if kit_keys => {
                    let kit = player.kit();
                    match player.note_on_to(kit, note) {
                        sampler::NoteTo::Pad(pad) => {
                            let pad = pad as u8;
                            // A Loop pad's hit waits for the launch line
                            // (R5.3b).
                            let w = time::PadWait {
                                from: time::PadFrom::Key(pad),
                                down: true,
                                velocity,
                                kit_pad: pad,
                            };
                            if !waits_for_line(
                                &mut time,
                                &player,
                                &app,
                                synth_on,
                                w,
                                &mut waits_full,
                                out,
                            ) {
                                if let Some(ev) = app.pressed_pad_event(pad, velocity, true) {
                                    time.pad_pressed(0, ev);
                                }
                                fx.extend(app.kit_key_down(pad, velocity));
                            }
                        }
                        sampler::NoteTo::Keys(n) => {
                            if player.key_kit_at(kit, n, app.velocity(velocity), 0) {
                                keys_direct += 1;
                                if keys_direct == 1 {
                                    out.midi(&format!(
                                        "+{mt:.3} s a key on the kit's keygroups: it plays at once, \
                                         not through the arp, and doesn't record (a kit whose pads \
                                         play notes takes every key through its arp); such keys are counted"
                                    ));
                                }
                            }
                        }
                    }
                    None
                }
                midi_in::Key::Up { note } if kit_keys => {
                    let kit = player.kit();
                    match player.note_off_to(kit, note) {
                        sampler::NoteTo::Pad(pad) => {
                            let pad = pad as u8;
                            let w = time::PadWait {
                                from: time::PadFrom::Key(pad),
                                down: false,
                                velocity: 0,
                                kit_pad: pad,
                            };
                            if !waits_for_line(
                                &mut time,
                                &player,
                                &app,
                                synth_on,
                                w,
                                &mut waits_full,
                                out,
                            ) {
                                if let Some(ev) = app.pressed_pad_event(pad, 0, false) {
                                    time.pad_pressed(0, ev);
                                }
                                fx.extend(app.kit_key_up(pad));
                            }
                        }
                        sampler::NoteTo::Keys(n) => player.key_off_kit_at(kit, n, 0),
                    }
                    None
                }
                midi_in::Key::Pressure { note, value } if kit_keys => {
                    // A keygroup's key takes no pressure yet.
                    if let sampler::NoteTo::Pad(pad) = player.note_held_to(player.kit(), note) {
                        let pad = pad as u8;
                        time.pad_pressure(0, pad, value);
                        app.kit_key_pressure(pad, value);
                        player.pressure(usize::from(pad), value);
                    }
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
                // A kit's pads take no bend, and no pedal.
                midi_in::Key::Bend { .. }
                | midi_in::Key::Expr { .. }
                | midi_in::Key::Control { .. } => None,
            };
            match pad_event {
                Some(event) => got.push(Incoming {
                    port: MUSIC_CABLE as usize,
                    bytes: Vec::new(),
                    event: Some(event),
                    music: true,
                }),
                None if !key_notes && !kit_keys && matches!(k, midi_in::Key::Down { .. }) => {
                    key_misses += 1;
                    if key_misses == 1 {
                        out.midi(&format!(
                            "+{mt:.3} s a key with no pad: on 16 pads (no pad layout), keys play the pads by the MPC's pad notes ({}); \
                             other notes are counted",
                            pad_note_names()
                        ));
                    }
                }
                None => {}
            }
        }
        // Expression recorded (the MIDI plan's 3.7): the bend, pressure and
        // controllers the armed track's gear was sent from the keys, as
        // moves on its MIDI lanes while the transport rolls (the recorder
        // keeps them while it records, the move log for CAPTURE).
        let gear = midi_out.expression_sent(armed as u16);
        if let Some((bend, pressure)) = gear {
            expr_rec.sent(bend, pressure);
        }
        let gear_track = gear.and_then(|_| u8::try_from(armed).ok());
        expr_rec.period(gear_track, app.rolling(), mt, |i| time.gesture(0, i));
        // MIDI Follow: the controller's knobs turn the Q-Links, as the
        // MPC's own Q-Links turned by as much.
        for (knob, d) in music.take_knob_turns() {
            got.push(Incoming {
                port: MUSIC_CABLE as usize,
                bytes: Vec::new(),
                event: Some(ControlEvent::QLinkTurn {
                    knob,
                    delta: d.clamp(-127, 127) as i8,
                }),
                music: true,
            });
        }
        // Its Send values back: the Q-Links' values to the controller's
        // lights or motor faders, when they change.
        music.send_back(mt, |to, msg| midi_out.send_back(to, msg));
        // What thru passes on (the I/O view's Routing): out of its output
        // now, if MIDI out is on.
        for p in music.take_thru() {
            midi_out.thru(p.from, p.to, p.msg);
        }
        for (track, k) in routed_keys.drain(..) {
            // CAPTURE keeps the key as the pad that plays its note on the
            // track, and puts it in that track.
            let t = usize::from(track);
            let synth_has = track_kits.plays_synth(t).then_some(&*synth);
            let kept = routed_pads.event(track, k, |n| {
                midi_file::pad_on_track(&mix.instruments, synth_has, t, n)
            });
            if let Some(ev) = kept {
                time.routed_played(track, 0, ev);
            }
            play_routed(
                track,
                k,
                &mut midi_out,
                &mut cv_out,
                &mut mix.instruments,
                &mut mix.pad_synths,
                &mut synth,
                track_kits.plays_synth(usize::from(track)),
                &mut player,
                track_kits.kit_of(usize::from(track)).unwrap_or(0),
            );
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
            // The pads' layers (step 11a): a pad's let-go and pressure
            // reach the notes its hit started there, whichever arm below
            // takes them. Keys play the layers by note (above), so a key
            // that plays a pad doesn't.
            match m.event {
                Some(ControlEvent::PadUp { pad }) if (pad as usize) < 16 && !m.music => {
                    pad_layers.pad_off(pad as usize, &mut synth, &mut mix.instruments);
                    pad_layers.rec_off(pad as usize, &mut time);
                    play_kit_layers(
                        pad_layers.kit_pads_off(pad as usize),
                        |note| mpfree_play::Event::NoteOff { note },
                        app.track() as u8,
                        &mut midi_out,
                        (&mut mix.pad_synths, &mut synth),
                        &mut player,
                    );
                }
                Some(ControlEvent::PadPressure { pad, pressure })
                    if (pad as usize) < 16 && !m.music =>
                {
                    let own = &mut mix.instruments;
                    pad_layers.pad_pressure(pad as usize, pressure, &mut synth, own);
                    pad_layers.rec_pressure(pad as usize, pressure, &mut time);
                    play_kit_layers(
                        pad_layers.kit_pads_held(pad as usize),
                        |note| mpfree_play::Event::Pressure {
                            note,
                            value: pressure,
                        },
                        app.track() as u8,
                        &mut midi_out,
                        (&mut mix.pad_synths, &mut synth),
                        &mut player,
                    );
                }
                _ => {}
            }
            let mut send = |to, msg| {
                midi_out.controller(to, msg);
            };
            match m.event {
                // Controller mode: the pads, the Q-Links, Q-LINK and PAD
                // BANK are the controller's.
                Some(ControlEvent::PadDown { pad, velocity }) if controller.on() => {
                    controller.pad_down(pad as usize, app.velocity(velocity), &mut send);
                }
                Some(ControlEvent::PadUp { pad }) if controller.on() => {
                    controller.pad_up(pad as usize, &mut send);
                }
                Some(ControlEvent::PadPressure { pad, pressure }) if controller.on() => {
                    controller.pressure(pad as usize, pressure, &mut send);
                }
                Some(ControlEvent::QLinkTurn { knob, delta }) if controller.on() => {
                    controller.turn(knob as usize, delta as i32, &mut send);
                }
                Some(ControlEvent::Button {
                    note,
                    pressed: true,
                }) if controller.on() && Some(note) == app.map.q_link => {
                    if app.shift_held() {
                        controller.prev_bank();
                    } else {
                        controller.next_bank();
                    }
                }
                Some(ControlEvent::Button {
                    note,
                    pressed: true,
                }) if controller.on() && app.map.bank_of(note).is_some() => {
                    let b = app.map.bank_of(note).unwrap_or(0);
                    controller.pad_bank_button(b, app.shift_held());
                }
                Some(ControlEvent::Button { note, .. })
                    if controller.on()
                        && (Some(note) == app.map.q_link || app.map.bank_of(note).is_some()) => {}
                // MAIN held (the MPC's hold MAIN): a pad picks that track;
                // it plays nothing.
                Some(ControlEvent::PadDown { pad, .. })
                    if (pad as usize) < 16 && app.main_held().is_some() =>
                {
                    mute_pressed[pad as usize] = true;
                    fx.extend(app.main_pick(pad as usize));
                }
                // COPY held over the kit's pads (the MPC's COPY, and with
                // SHIFT its Delete): the first pad is copied onto the next
                // ones, or each pad is emptied; they play nothing.
                Some(ControlEvent::PadDown { pad, .. })
                    if (pad as usize) < 16 && app.copy_held() && app.pads_play_kit() =>
                {
                    mute_pressed[pad as usize] = true;
                    if let Some(more) = app.copy_pad(pad as usize) {
                        fx.extend(more);
                    }
                }
                // The Pads tab's Perform view: a pad picks the set's pad
                // there; a throw plays while held (or its bar), so it can
                // be heard (held even when the pad latches, so nothing keeps
                // playing), and a macro pad turns its macro while held. The
                // kit's pads don't play.
                Some(ControlEvent::PadDown { pad, .. })
                    if (pad as usize) < 16 && app.perform_editing() =>
                {
                    let pad = pad as usize;
                    out.show(gui::ToUi::PerformHit(pad));
                    let heard = match app.perform_page().action(pad) {
                        perform_pads::PadAction::Throw(t)
                            if t.hold == perform_pads::Hold::Latch =>
                        {
                            perform_pads::PadAction::Throw(perform_pads::Throw {
                                hold: perform_pads::Hold::WhileHeld,
                                ..t
                            })
                        }
                        a @ (perform_pads::PadAction::Throw(_)
                        | perform_pads::PadAction::Macro { .. }) => a,
                        _ => perform_pads::PadAction::None,
                    };
                    keys_held.press_action(pad, t, heard);
                    let drums = kit_drums(&kits, player.kit());
                    let (more, line) =
                        desk::perform_pad_on(&mut app, &time, &looper, out, heard, drums);
                    fx.extend(more);
                    if !line.is_empty() {
                        out.note(&line);
                    }
                }
                Some(ControlEvent::PadDown { pad, velocity })
                    if (pad as usize) < 16 && app.pads_are_steps() =>
                {
                    let v = app.velocity(velocity);
                    let rolling = time.info().state.rolling();
                    let p = step_press(&app, &mut step_keys, &mut time, pad as usize, v, rolling);
                    fx.extend(sound_step(p, out, &mut player, &mut synth, synth_on));
                }
                Some(ControlEvent::PadUp { pad }) if step_keys.held(pad as usize) => {
                    let r = step_keys.release(pad as usize, &mut time);
                    let_go_of_step(r, out, &mut player, &mut synth, synth_on);
                    out.show(gui::ToUi::PadUp(pad as usize));
                }
                Some(ControlEvent::PadDown { pad, .. })
                    if (pad as usize) < 16 && app.pads_are_loop_keys() =>
                {
                    let pad = pad as usize;
                    let action = app.perform_page().action(pad);
                    keys_held.press_action(pad, t, action);
                    let drums = kit_drums(&kits, player.kit());
                    let (more, line) =
                        desk::perform_pad_on(&mut app, &time, &looper, out, action, drums);
                    fx.extend(more);
                    if !line.is_empty() {
                        out.note(&line);
                    }
                }
                Some(ControlEvent::PadUp { pad }) if keys_held.action(pad as usize).is_some() => {
                    if let Some(action) = keys_held.release_action(pad as usize) {
                        fx.extend(desk::perform_pad_up(&mut app, action));
                    }
                    out.show(gui::ToUi::PadUp(pad as usize));
                }
                Some(ControlEvent::PadDown { pad, .. })
                    if (pad as usize) < 16 && app.pads_mute() =>
                {
                    mute_pressed[pad as usize] = true;
                    let (tap, more) = app.mute_tap();
                    fx.extend(more);
                    let why = no_mutes(&track_kits, mix.instruments.note_tracks(), app.track());
                    let (t, page) = (app.track(), app.pad_page());
                    out.note(&mute_key(tap, pad as usize, t, page, why, &mut time));
                }
                Some(ControlEvent::PadDown { pad, .. })
                    if (pad as usize) < 16 && app.pads_transpose() =>
                {
                    let pad = pad as usize;
                    mute_pressed[pad] = true;
                    let own = &mix.instruments;
                    out.note(&transpose_pad(pad, &mut time, &synth, &track_kits, own));
                }
                // The synth chosen for the pads (or, from step 6, an
                // instrument of the host's): it plays the hit, before the
                // arp and the recorder; the armed track keeps its kit.
                // With NOTE REPEAT, Latch or the arp on, the pads' arp
                // takes it and plays it there (sampler step 15c).
                Some(ControlEvent::PadDown { pad, velocity })
                    if (pad as usize) < 16 && pads_preview.is_some() =>
                {
                    let pad = pad as usize;
                    let v = app.velocity(velocity);
                    // On a kit or a host's pads a phrase is the track's:
                    // here they play as pressed, or repeat; on the shared
                    // synth the pads' arp plays it (`App::pads_own_mode`).
                    let arp_on = app.pads_own_mode() != mpfree_arp::Mode::Direct;
                    let arp_to = pads_preview.filter(|_| arp_on);
                    if let Some(to) = arp_to {
                        let mut buf = [0u8; MAX_PAD_NOTES];
                        let n = match to {
                            PadsPreview::Host => {
                                buf[0] = pad as u8;
                                1
                            }
                            PadsPreview::Kit(_) => {
                                let kit_pad = app.kit_pad(pad);
                                preview_pad[pad] = kit_pad as u8;
                                buf[0] = kit_pad as u8;
                                1
                            }
                            PadsPreview::Synth => {
                                let notes = synth.notes(pad);
                                let n = notes.len().min(MAX_PAD_NOTES);
                                buf[..n].copy_from_slice(&notes[..n]);
                                n
                            }
                        };
                        let (track, page) = (app.track() as u8, app.kit_page());
                        pads_arp.down(pad, &buf[..n], v, to, &mut |to, n, rec| {
                            play_pads_arp(
                                to,
                                n,
                                track,
                                page,
                                &mut synth,
                                &mut mix.instruments,
                                &mut midi_out,
                                &mut mix.pad_synths,
                                &mut player,
                            );
                            rec_pads_arp(to, n, rec, &mut time);
                        });
                    }
                    match pads_preview.filter(|_| arp_to.is_none()) {
                        Some(PadsPreview::Host) => {
                            let none = mpfree_arp::scale::Shift::NONE;
                            mix.instruments
                                .pad_on_shifted(instruments::THE_PADS, pad, v, 0, &none);
                        }
                        // A kit chosen for the pads (sampler step 15): the
                        // pad of its page they are on, as its kind has it.
                        Some(PadsPreview::Kit(k)) => {
                            let kit_pad = app.kit_pad(pad);
                            preview_pad[pad] = kit_pad as u8;
                            let hit = mpfree_play::Event::NoteOn {
                                note: kit_pad as u8,
                                velocity: v,
                            };
                            play_routed_pad(
                                app.track() as u8,
                                kit_pad,
                                hit,
                                &mut midi_out,
                                (&mut mix.pad_synths, &mut synth),
                                &mut player,
                                usize::from(k),
                            );
                        }
                        Some(PadsPreview::Synth) => synth.note_on(pad, v, 0),
                        None => {}
                    }
                    // Into the track recording what the pads play (sampler
                    // step 15c), as the pad it plays there; the arp's notes
                    // go there instead while it takes the hit.
                    if arp_to.is_none() {
                        let note = match pads_preview {
                            Some(PadsPreview::Kit(_)) => preview_pad[pad],
                            _ => pad as u8,
                        };
                        pads_rec.on(pad, note, app.layer_rec_velocity(velocity), &mut time);
                    }
                    if !m.music {
                        pad_layers.pad_on(pad, v, &mut synth, &mut mix.instruments);
                        pad_layers.rec_on(pad, app.layer_rec_velocity(velocity), &mut time);
                        play_kit_layers(
                            pad_layers.kit_pads_on(pad),
                            |note| mpfree_play::Event::NoteOn { note, velocity: v },
                            app.track() as u8,
                            &mut midi_out,
                            (&mut mix.pad_synths, &mut synth),
                            &mut player,
                        );
                    }
                    preview_held[pad] = pads_preview;
                    out.show(gui::ToUi::PadDown { pad, velocity: v });
                    if mute_shown.is_none() {
                        ports.send(
                            false,
                            &akai::pad_colour(pad as u8, akai::hit_colour(rest[pad])),
                        )?;
                    }
                }
                Some(ControlEvent::PadUp { pad })
                    if (pad as usize) < 16 && preview_held[pad as usize].is_some() =>
                {
                    let pad = pad as usize;
                    // A hit the pads' arp took lets go there (sampler
                    // step 15c).
                    let held = preview_held[pad].take().filter(|_| !pads_arp.holds(pad));
                    pads_arp.up(pad);
                    pads_rec.off(pad, &mut time);
                    match held {
                        Some(PadsPreview::Host) => {
                            mix.instruments.pad_off(instruments::THE_PADS, pad, 0);
                        }
                        // On the kit and pad it started, though the pads
                        // moved on meanwhile (rule 8).
                        Some(PadsPreview::Kit(k)) => {
                            let kit_pad = usize::from(preview_pad[pad]);
                            let off = mpfree_play::Event::NoteOff {
                                note: kit_pad as u8,
                            };
                            play_routed_pad(
                                app.track() as u8,
                                kit_pad,
                                off,
                                &mut midi_out,
                                (&mut mix.pad_synths, &mut synth),
                                &mut player,
                                usize::from(k),
                            );
                        }
                        Some(PadsPreview::Synth) => synth.note_off(pad, 0),
                        None => {}
                    }
                    out.show(gui::ToUi::PadUp(pad));
                    if mute_shown.is_none() {
                        ports.send(false, &akai::pad_colour(pad as u8, rest[pad]))?;
                    }
                }
                Some(ControlEvent::PadPressure { pad, pressure })
                    if (pad as usize) < 16 && preview_held[pad as usize].is_some() =>
                {
                    let pad = pad as usize;
                    let held = preview_held[pad].filter(|_| !pads_arp.holds(pad));
                    pads_arp.pressure(pad, pressure);
                    pads_rec.pressure(pad, pressure, &mut time);
                    match held {
                        Some(PadsPreview::Host) => {
                            mix.instruments
                                .pad_pressure(instruments::THE_PADS, pad, pressure, 0);
                        }
                        Some(PadsPreview::Kit(k)) => {
                            let kit_pad = usize::from(preview_pad[pad]);
                            let press = mpfree_play::Event::Pressure {
                                note: kit_pad as u8,
                                value: pressure,
                            };
                            play_routed_pad(
                                app.track() as u8,
                                kit_pad,
                                press,
                                &mut midi_out,
                                (&mut mix.pad_synths, &mut synth),
                                &mut player,
                                usize::from(k),
                            );
                        }
                        Some(PadsPreview::Synth) => synth.pressure(pad, pressure),
                        None => {}
                    }
                }
                Some(ControlEvent::PadUp { pad })
                    if (pad as usize) < 16 && mute_pressed[pad as usize] =>
                {
                    mute_pressed[pad as usize] = false;
                    out.show(gui::ToUi::PadUp(pad as usize));
                }
                Some(ControlEvent::PadDown { pad, .. })
                    if (pad as usize) < 16 && live_erase_starts(&app, &time) =>
                {
                    fx.extend(live_erase(&mut time, app.kit_pad(pad as usize)));
                }
                // ERASE + a kit pad while stopped: its notes out of the
                // armed track's clip at once; it plays nothing.
                Some(ControlEvent::PadDown { pad, .. })
                    if (pad as usize) < 16
                        && app.erase_held()
                        && app.pads_play_kit()
                        && !time.info().state.rolling() =>
                {
                    mute_pressed[pad as usize] = true;
                    fx.push(erase_pad_notes(&mut time, app.kit_pad(pad as usize)));
                }
                // SHIFT + a kit pad: picked, not played (the MPC's way to
                // pick a pad in the middle of a song).
                Some(ControlEvent::PadDown { pad, .. })
                    if (pad as usize) < 16
                        && app.shift_held()
                        && !app.erase_held()
                        && app.pads_play_kit() =>
                {
                    mute_pressed[pad as usize] = true;
                    if let Some(more) = app.pick_pad_quietly(pad as usize) {
                        fx.extend(more);
                    }
                }
                Some(ControlEvent::PadUp { pad })
                    if (pad as usize) < 16 && time.erases(app.kit_pad(pad as usize) as u8) =>
                {
                    time.erase_sound(app.kit_pad(pad as usize) as u8, false);
                    out.show(gui::ToUi::PadUp(pad as usize));
                }
                Some(ControlEvent::PadDown { pad, velocity }) if (pad as usize) < 16 => {
                    let pad = pad as usize;
                    // FULL LEVEL held: this hit sets the level every hit
                    // plays at, this one first.
                    fx.extend(app.full_level_hit(velocity));
                    let take = app.rec.ready.map(|take| take == rec_arm::Take::Add);
                    out.loop_start(pad_starts_loop(
                        &mut looper,
                        &mut time,
                        take,
                        export.running(),
                    ));
                    held[pad] = true;
                    // A Loop pad's hit waits for the launch line (R5.3b).
                    let w = time::PadWait {
                        from: time::PadFrom::Pad(pad as u8),
                        down: true,
                        velocity,
                        kit_pad: app.kit_pad(pad) as u8,
                    };
                    if !waits_for_line(&mut time, &player, &app, synth_on, w, &mut waits_full, out)
                    {
                        if let Some(ev) = app.pressed_event(pad, velocity, true) {
                            // With the notes it plays, for a take of notes (step 5d):
                            // on a kit played as notes, the bank's (step 15c).
                            let t = app.track();
                            let rows =
                                pad_rows(&mix.instruments, &synth, &track_kits, &kit_notes, t);
                            let kit_note = kit_notes.note(&track_kits, t, app.kit_pad(pad));
                            let notes = kit_note
                                .as_ref()
                                .map_or(rows.notes(pad), std::slice::from_ref);
                            time.pad_pressed_notes(0, ev, notes);
                            // The note editor's pads: write or pick.
                            fx.extend(editor_pad(&app, &mut time, out, pad, ev, notes));
                        }
                        // The pads' layers (step 11a) play the hit too.
                        if !m.music {
                            let v = app.velocity(velocity);
                            pad_layers.pad_on(pad, v, &mut synth, &mut mix.instruments);
                            pad_layers.rec_on(pad, app.layer_rec_velocity(velocity), &mut time);
                            play_kit_layers(
                                pad_layers.kit_pads_on(pad),
                                |note| mpfree_play::Event::NoteOn { note, velocity: v },
                                app.track() as u8,
                                &mut midi_out,
                                (&mut mix.pad_synths, &mut synth),
                                &mut player,
                            );
                        }
                        fx.extend(app.pad_down(pad, velocity));
                        awaiting[pad] = true;
                        if direct {
                            pending.push(t_read);
                        }
                    }
                }
                Some(ControlEvent::PadUp { pad }) if (pad as usize) < 16 => {
                    let pad = pad as usize;
                    held[pad] = false;
                    let w = time::PadWait {
                        from: time::PadFrom::Pad(pad as u8),
                        down: false,
                        velocity: 0,
                        kit_pad: app.kit_pad(pad) as u8,
                    };
                    if !waits_for_line(&mut time, &player, &app, synth_on, w, &mut waits_full, out)
                    {
                        if let Some(ev) = app.pressed_event(pad, 0, false) {
                            // With the notes it plays, for a take of notes (step 5d):
                            // on a kit played as notes, the ones it pressed (step 15c).
                            let t = app.track();
                            let rows =
                                pad_rows(&mix.instruments, &synth, &track_kits, &kit_notes, t);
                            let notes = match kit_notes.of_track(&track_kits, t) {
                                Some(_) => app.pad_held_notes(pad),
                                None => rows.notes(pad),
                            };
                            time.pad_pressed_notes(0, ev, notes);
                            fx.extend(editor_pad(&app, &mut time, out, pad, ev, &[]));
                        }
                        fx.extend(app.pad_up(pad));
                        awaiting[pad] = false;
                        out.show(gui::ToUi::PadUp(pad));
                    }
                }
                // A Perform pad's pressure: it plays no note, so it presses
                // nothing (and records nothing).
                Some(ControlEvent::PadPressure { pad, .. })
                    if keys_held.action(pad as usize).is_some() => {}
                Some(ControlEvent::PadPressure { pad, pressure }) if (pad as usize) < 16 => {
                    // Recorded onto the pad's note while the armed track
                    // records pressure.
                    let kit_pad = app.kit_pad(pad as usize);
                    time.pad_pressure(0, kit_pad as u8, pressure);
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
                        player.pressure(kit_pad, pressure);
                    }
                }
                Some(ControlEvent::DataWheel { delta }) => fx.extend(app.wheel(delta as i32)),
                Some(ControlEvent::QLinkTurn { knob, delta }) => {
                    // A turn with no finger on the knob (MIDI Follow) lets
                    // go half a second after the last (qlink_touch.rs).
                    touch_hold.turned(knob as usize, t);
                    // On the Synth page Q-Links 1-4 turn the marked knobs; while NOTE REPEAT is held
                    // or the Arp panel is open they set the arp; otherwise Q-Link 1 is the level.
                    let public = MIDI_PORTS[m.port].1 == "Public" && m.bytes.len() == 3;
                    // On the Sample page they move Start, End and the loop.
                    let page = app.view();
                    // While the mixer is open they move the selected strip; on the chain
                    // editor and the macros panel, the knobs or macros the column shows. In
                    // the browser: scroll, filter, preview level, place. Over any page,
                    // Sample Recall's: Length, Ends at, Fade and Pad.
                    if app.recall_open() && knob < 4 {
                        let k = knob as usize;
                        let ticks = if public {
                            let was = qlink_at[k].replace(m.bytes[2]);
                            was.map_or(0, |w| m.bytes[2] as i32 - w as i32)
                        } else {
                            delta as i32
                        };
                        if ticks != 0 {
                            out.show(gui::ToUi::RecallQLink(k, ticks));
                        }
                    } else if matches!(
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
                    } else if matches!(
                        page,
                        app::View::Play | app::View::Sequencer | app::View::Arrange
                    ) && knob < 4
                    {
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
                    fx.extend(erase_held_steps(
                        &app,
                        &mut step_keys,
                        &mut time,
                        note,
                        pressed,
                    ));
                    fx.extend(app.button(note, pressed, t));
                    live_erase_let_go(&app, &mut time);
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
        // Letting go of a Q-Link unlights its card, ends a turn on the
        // Sample page (one undo step) and lets go of a knob's setting; so
        // do turns with no finger, half a second after the last.
        for (k, _) in touch_hold.due(t).iter().enumerate().filter(|(_, &d)| d) {
            out.show(gui::ToUi::QLinkTouch(k, false));
            if app.view().sampler() {
                out.show(gui::ToUi::SampleLift);
            }
        }
        // A loop key held for a second clears its loop, as a ring held on
        // the Looper tab does.
        for pad in keys_held.holds(t) {
            if app.pads_are_loop_keys() {
                let action = keys_held.action(pad).unwrap_or_default();
                let (more, line) = desk::perform_pad_held(action);
                fx.extend(more);
                if !line.is_empty() {
                    out.note(&line);
                }
            }
        }
        // And a learned loop key (a footswitch), on any page.
        for n in remote.loop_holds(t, perform_pads::CLEAR_HOLD_S) {
            let key = perform_pads::PadAction::LoopKey(n as u8);
            let (more, line) = desk::perform_pad_held(key);
            fx.extend(more);
            out.note(&format!("{line}, from a learned control"));
        }
        // And a key learned on a Perform pad that is a loop key.
        for key in perform_keys.holds(t, perform_pads::CLEAR_HOLD_S) {
            let (more, line) = desk::perform_pad_held(key);
            fx.extend(more);
            if !line.is_empty() {
                out.note(&format!("{line}, from a learned key"));
            }
        }
        fx.extend(app.tick(t));
        // "Keep this theme?" counting down in Prefs > Appearance.
        fx.extend(app.theme_tick());
        // The notes the note editor hears let go in time, and all at once
        // when the transport starts or stops.
        if !hearing.is_empty() {
            let mut on = HearOn {
                own: &mut mix.instruments,
                synth: &mut synth,
                player: &mut player,
                kits: &track_kits,
                midi: &midi_out,
            };
            if time.info().state.rolling() != hear_rolling {
                hearing.stop(&mut on);
            } else {
                hearing.tick(t, &mut on);
            }
        }
        // As set, not rounded: the song plays at it, and a project saves it
        // as it is.
        if let Some(bpm) = loop_tempo.take() {
            fx.extend(app.set_tempo(bpm));
            // The loop's, not set by hand.
            app.tempo_by_hand = false;
        }
        // The time engine's editing side on its own thread
        // (--time-thread): what its answers say, as each caller asked,
        // and the Sequencer's Notes view following the note editor.
        if time.apart() {
            for (say, r) in time.take_said() {
                fx.extend(time_said(out, say, r));
            }
            fx.extend(app.set_notes_open(time.editing()));
        }
        // A scene's tempo landed on its line (Arrange 5.2b): the app's
        // tempo becomes it, as if turned there.
        if let Some(bpm) = time.take_scene_tempo() {
            fx.extend(app.set_tempo(bpm));
        }
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
        // What REC must leave alone while the synth is chosen for the pads
        // (app/pads_focus.rs): kits and MIDI outputs picked; and the tracks
        // that play the synth, where REC records the pads as they are.
        app.set_configured(track_kits.picked() | midi_out.routed());
        app.set_synth_tracks(track_kits.on_synth());
        app.set_kit_tracks(|k| track_kits.on_kit(k));
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
                listed: &listed,
                level_db: &mut level_db,
                max_db,
                card: audio,
                undo: &mut one_undo,
            };
            let Some(e) = desk.effect(e, out, &mut later) else {
                parts.item_named(parts::APP, t_item, &item);
                continue;
            };
            match e {
                app::Effect::Light(note, value) if akai::app_lights(note) => {
                    // Through CV out's writer while it shares the port.
                    let light = akai::button_light(note, value);
                    if cv_out.light(&light) {
                        ports.tap(true, &light);
                    } else {
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
                        || mix.instruments.plays(tab_who(&app, &mix.instruments)) =>
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
                        lit = [None; 16];
                        mute_shown = None;
                    }
                }
                // SHIFT + wheel with a drum kit: the kit, as before (past
                // the empty kits New kit hasn't used yet).
                app::Effect::Octave(d) => {
                    let from = kit_to.unwrap_or(player.kit());
                    let sounds = |k| player.sounds(k);
                    kit_to = Some(kit_turned(from, d, n, &user_kits, sounds, &track_kits));
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
                        lit = [None; 16];
                        mute_shown = None;
                        bank_lights = true;
                    }
                }
                // The Instrument row: the selected track's own instrument, or
                // the kit again.
                app::Effect::TrackSound { track, id } => match id {
                    None => {
                        let senders = mix.instruments.senders(track);
                        match mix.instruments.take_out(track) {
                            true => {
                                if track < instruments::TRACKS {
                                    out.note(&format!("track {}: plays the kit", track + 1));
                                } else {
                                    out.note(&format!("{}: put away", holder_says(track)));
                                }
                                if let Some(s) = sends_ended(senders, track) {
                                    out.note(&s);
                                    out.show(gui::ToUi::Tell(s));
                                }
                            }
                            false => {
                                let s = format!(
                                    "{}: its last sound is still ringing out; try again in a moment",
                                    holder_says(track)
                                );
                                out.note(&s);
                                out.show(gui::ToUi::Tell(s));
                            }
                        }
                        app.set_track_sound(track, mix.instruments.held(track));
                        app.set_track_sends(&sends_now(&mix.instruments));
                        track_presets.set_index(track, None);
                        // The pads may play another now (a free place's
                        // put away, step 6): the tab's from the next pass.
                        let who = if track < instruments::TRACKS {
                            track
                        } else {
                            app.track()
                        };
                        out.show(gui::ToUi::Synth(synth_tab(
                            &synth,
                            &mix,
                            who,
                            &track_presets,
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
                                "{}: busy making another sound; {id} not made, try again",
                                holder_says(track)
                            );
                            out.note(&s);
                            out.show(gui::ToUi::Tell(s));
                            app.set_track_sound(track, mix.instruments.held(track));
                        } else {
                            // A copy of the pads' sound: no preset of the
                            // musician's, and a try on the old one is over.
                            track_presets.new_sound(track);
                        }
                    }
                },
                // The Instrument row: the track plays another track's
                // instrument (step 4), its own taken out first.
                app::Effect::TrackSend { track, to } => {
                    let (lines, tell) = send_track(&mut mix.instruments, track, to);
                    for line in &lines {
                        out.note(line);
                    }
                    if let Some(s) = tell {
                        out.show(gui::ToUi::Tell(s));
                    }
                    app.set_track_sound(track, mix.instruments.held(track));
                    app.set_track_sends(&sends_now(&mix.instruments));
                    if !mix.instruments.holds(track) {
                        track_presets.set_index(track, None);
                    }
                    out.show(gui::ToUi::Synth(synth_tab(
                        &synth,
                        &mix,
                        track,
                        &track_presets,
                    )));
                }
                // The Instrument row: a copy of another track's instrument
                // as the track's own (step 4c, Duplicate), made as a new
                // sound is; it goes in once made (Landed, below).
                app::Effect::TrackCopy { track, from } => {
                    let name = mix.instruments.held(from).map_or("instrument", |h| h.name);
                    let tell = match sound_of_track(&mix.instruments, from) {
                        None => Some(format!(
                            "Track {}: Track {} has no instrument of its own to copy",
                            track + 1,
                            from + 1
                        )),
                        Some(_) if !mix.instruments.has_room(track) => Some(full_note(track, name)),
                        Some((id, sound)) => {
                            let job = instrument_job(&synth, track, id, Some(sound));
                            if mix.build_instrument(job) {
                                track_presets.copy(from, track);
                                let s = copied_note(track, from, name);
                                out.note(&s);
                                out.show(gui::ToUi::Tell(s));
                                None
                            } else {
                                Some(format!(
                                    "Track {}: busy making another sound; the copy of {} not made, try again",
                                    track + 1,
                                    app::sent_name(from, name)
                                ))
                            }
                        }
                    };
                    if let Some(s) = tell {
                        out.note(&s);
                        out.show(gui::ToUi::Tell(s));
                        app.set_track_sound(track, mix.instruments.held(track));
                        app.set_track_sends(&sends_now(&mix.instruments));
                    }
                }
                // What the pads play changed: the screen names it; the pads
                // follow below, once the kit has.
                app::Effect::PadsFocus(f) => {
                    out.show(gui::ToUi::PadsFocus(f));
                    pads_shown.0 = f;
                }
                // REC while the pads play the synth (desk::rec_synth): the
                // track plays the same synth from now on, as a pick of
                // Synth in the list of kits; the selected one takes it at
                // once, another once it is armed (its kit follows).
                app::Effect::SynthOnTrack(t) => {
                    track_kits.set(t, track_kits.synth());
                    if t == app.track() {
                        kit_to = Some(track_kits.synth());
                    }
                }
                // REC while a kit is chosen for the pads (sampler step 15):
                // the track plays that kit from now on, as a pick.
                app::Effect::KitOnTrack { track, kit } => {
                    track_kits.set(track, kit);
                    if track == app.track() {
                        kit_to = Some(kit);
                    }
                }
                app::Effect::StopSounds => {
                    player.stop_all();
                    synth.stop();
                    mix.instruments.stop();
                    mix.pad_synths.stop();
                    music.patches_stopped();
                    // STOP while stopped turns the XY pad's Hold off (its
                    // layout lets go as it says).
                    if mix.xy_held() {
                        mix.xy_hold(false);
                        let pad_now = |owner, id: u16| {
                            let (kit, pad) = learned_kit(&listed, &track_kits, owner)?;
                            pad_describe(&player, kit, pad, id)
                        };
                        out.show(xy_view(
                            &mut xy_yours,
                            &mix,
                            &synth,
                            &app,
                            &listed,
                            &pad_now,
                            &time,
                        ));
                    }
                }
                app::Effect::Audio(p) => audio_change = Some(p),
                app::Effect::Quit => stop = true,
                app::Effect::Show(v) => {
                    // Opening a page never changes what a track plays; only
                    // a pick does. On the Synth tab, a track that plays a
                    // kit is told how to play the synth instead.
                    if v == app::View::Synth
                        && !kit_to.map_or(synth_on, |k| k == synth_kit)
                        && !mix.instruments.plays(tab_who(&app, &mix.instruments))
                    {
                        out.show(gui::ToUi::Tell(synth_tab_hint(app.track())));
                    }
                    out.show(gui::ToUi::Show(v));
                }
                // 16 LEVEL: the screen holds the kits. With the synth on the
                // pads it opens Transpose (sequencer S4); the Drums
                // engine's pads have neither.
                app::Effect::SixteenLevels
                    if (synth_on || kit_to == Some(synth_kit)) && !synth.plays_notes() =>
                {
                    out.note(DRUMS_NO_LEVELS)
                }
                app::Effect::SixteenLevels => out.show(gui::ToUi::SixteenLevels),
                // REC with kit pads held: an empty one arms a pad take
                // (sampler R5.7); otherwise REC as alone.
                app::Effect::PadRec(held) => later.push(desk::pad_rec(
                    &held,
                    track_kits.kit_of(app.track()),
                    |k, p| player.has_sound(k, p),
                )),
                app::Effect::SaveProject => save_asked = true,
                // Prefs > Backup and Akai kits: the first-start setup's store.
                app::Effect::SetupNextStart { backup, want } => {
                    let (state, said) = setup_next_start(o, &store, backup, want);
                    out.note(&said);
                    out.show(gui::ToUi::Tell(said));
                    later.extend(app.set_setup(state));
                }
                // PROG EDIT: the sound the pads play (Jay, 20:40Z).
                app::Effect::ProgEdit => {
                    let synth_now = kit_to.map_or(synth_on, |k| k == synth_kit);
                    if synth_now {
                        later.extend(app.open_tab(app::View::Synth));
                    } else {
                        // The Pads tab: the pad's settings.
                        later.extend(app.open_tab(app::View::Pads));
                    }
                }
                // A PAD BANK button: banks A-H are the pages of the armed
                // track's kit, its pads 1-16 to 113-128 (sampler R3, Jay's
                // decision 1, 16 Nov). While the synth has the pads, PAD
                // BANK A-H picks a bank's pad layout (Jay, 09:55Z).
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
                    if button < 4 {
                        let page = app::bank_target(app.pad_page(), button, shift);
                        later.extend(app.set_pad_page(page));
                        let kit = kit_to.unwrap_or(player.kit());
                        out.note(&pad_page_note(page, &kits[kit].name, player.sounds(kit)));
                        page_turned = true;
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
                app::Effect::SavePhrases(text) => {
                    if let Err(e) =
                        store.write(app::saved::ARP, app::saved::ARP_PHRASES, text.into_bytes())
                    {
                        out.note(&format!("arp phrases: not saved ({e})"));
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
            last_written = Instant::now();
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
        // The list of instruments follows the tracks' own and the free
        // places' (step 6): the app has it when it changes, so an
        // instrument picked for the pads is theirs once it has an id, and
        // a free place's that nothing plays goes away. The pads play the
        // instrument chosen for them by the host's pads player once it is
        // in. Nothing allocated while nothing changes.
        list_changed |= listed.follow(mix.instruments.held_tracks())
            | listed.follow_free(mix.instruments.held_free());
        if std::mem::take(&mut list_changed) {
            fx.extend(app.set_instrument_list(&listed));
        }
        // The synths MIDI routes name (step 11e) as MIDI in applies a
        // changed setup, before a free place nothing else plays is put
        // away.
        if let Some(ids) = music.take_route_synths() {
            app.set_route_synths(ids);
        }
        // Add a part's New track synth, once in its place, plays on the
        // track Keep would add (composing tools S2.6b).
        fx.extend(app.add_sound_follow(time.project().tracks.len()));
        fx.extend(app.put_away_free());
        let pads_at = app.pads_holder().filter(|&h| mix.instruments.holds(h));
        // The pads' layers (step 11a) follow the app's, as the pads do.
        pad_layers.follow(
            app.pads_layer_holders(),
            app.pads_layer_synth(),
            &mut synth,
            &mut mix.instruments,
        );
        // The armed tracks that record a layer (step 11c).
        pad_layers.rec_to(app.pads_layer_recs(time.also()), |kit| player.kit_keys(kit));
        for kit in pad_layers
            .kits_to(app.pads_layer_kits())
            .into_iter()
            .flatten()
        {
            player.bend_kit(usize::from(kit), 0.0);
        }
        // And what the pads play on their own, the pads' arp's notes too,
        // into the track REC set up for it (sampler step 15c).
        let own_rec = app.pads_own_rec(time.also());
        pads_rec.rec_to(own_rec);
        pads_arp.rec_to(own_rec.filter(|_| app.layer_rec_velocity(1).is_some()));
        // The synths MIDI routes name (step 11e), and where REC takes
        // their keys.
        route_synths.follow(
            app.route_synth_holders(),
            app.route_synth_shared(),
            &mut synth,
            &mut mix.instruments,
        );
        for kit in route_synths
            .kits_to(app.route_synth_kits())
            .into_iter()
            .flatten()
        {
            player.bend_kit(usize::from(kit), 0.0);
        }
        route_synths.rec_to(app.route_recs(time.also()), |kit| player.kit_keys(kit));
        if mix.instruments.pads_play_at() != pads_at {
            mix.instruments.pads_play(pads_at);
            // The Synth tab shows the pads' instrument while they play it
            // on their own.
            out.show(gui::ToUi::Synth(synth_tab(
                &synth,
                &mix,
                tab_who(&app, &mix.instruments),
                &track_presets,
            )));
        }
        // Its name for the screen's "Pads: FM", also when a second tap in
        // the list put another synth in the same place.
        let host_name = mix
            .instruments
            .pads_play_at()
            .and_then(|h| mix.instruments.held(h))
            .map(|h| h.name);
        if host_name != pads_host_shown {
            pads_host_shown = host_name;
            out.show(gui::ToUi::PadsHost(host_name));
        }
        // What the pads play with their layers (step 11b): "Pads: FM +
        // Pluck". Nothing allocated while nothing changes.
        let layers_now = app.pads_layer_list();
        if layers_now != layers_shown {
            layers_shown = layers_now;
            out.show(gui::ToUi::PadsLayers(app::layer_names(&layers_now)));
        }
        // What the pads play now and the bank whose own it is (step 6c):
        // PAD BANK, the synth's layout coming or going, a bank's synth
        // given its id. Nothing allocated while nothing changes.
        let pads_now = (app.pads_focus(), app.pads_bank());
        if pads_now != pads_shown {
            if pads_now.0 != pads_shown.0 {
                out.show(gui::ToUi::PadsFocus(pads_now.0));
            }
            if pads_now.1 != pads_shown.1 {
                out.show(gui::ToUi::PadsBank(pads_now.1));
            }
            pads_shown = pads_now;
        }
        // Each kit's tuning for the Pads tab's Edit pad (tunings step T6):
        // nothing made while nothing changes.
        let kit_tunings_now = Some(app.kit_tunings_key());
        if kit_tunings_now != kit_tunings_shown {
            kit_tunings_shown = kit_tunings_now;
            out.show(gui::ToUi::KitTunings(app.kit_tunings()));
        }
        // Who plays the Synth tab's instrument (step 8b), a few times a
        // second while the tab shows.
        if app.view() == app::View::Synth && mt >= who_at {
            who_at = mt + 0.25;
            let who = who_plays(
                &app,
                &mix.instruments,
                &track_kits,
                &time.project().tracks,
                synth.engine_name(),
            );
            if who != who_shown {
                who_shown.clone_from(&who);
                out.show(gui::ToUi::WhoPlays(who));
            }
        }
        // The selected track's line over the Instrument row (step 11d),
        // as often, while the options show.
        if app.view() == app::View::Options && mt >= line_at {
            line_at = mt + 0.25;
            let t = app.track();
            let to_gear = |t: usize| midi_out.takes(t as u16);
            let homes = track_kits.homes(|t| !mix.instruments.plays(t) && !to_gear(t));
            let name = synth.engine_name();
            let line = track_line(t, &mix, &track_kits, &homes, name, to_gear(t));
            fx.extend(app.set_track_line(line));
            // What the tuning screens say (tunings step T6): who can't
            // follow a tuning, and the kit each track plays.
            let kit_of = std::array::from_fn(|t| {
                track_kits
                    .kit_of(t)
                    .and_then(|k| u8::try_from(k).ok())
                    .filter(|_| !to_gear(t))
            });
            fx.extend(app.set_tuning_facts(tunings_handed.deaf(), kit_of));
        }
        let playing_kit = if synth_on { synth_kit } else { player.kit() };
        let mut kit_switched = false;
        if let Some(k) = kit_to.filter(|&k| k != playing_kit) {
            used[k] = true;
            // A pick: the armed track plays it from now on. (When the pads
            // followed the armed track, it already does.)
            track_kits.set(app.track(), k);
            // The pads held now let go where they played; the other
            // tracks' notes play on.
            for pad in (0..16).filter(|&p| held[p]) {
                if !synth_on {
                    player.release_kit_at(playing_kit, app.kit_pad(pad), 0);
                } else if app.arp_plays_notes() {
                    for &note in app.pad_held_notes(pad) {
                        synth.note_off_note(note, 0);
                    }
                } else {
                    // The pads play the synth through the track's arp.
                    synth.note_off_by(synth::Player::Arp(app.track()), pad, 0);
                }
            }
            if k == synth_kit {
                // Every kit plays on; its sounds finish.
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
                // Other tracks may play the synth: it plays on.
                synth_on = false;
                player.select(k);
                rest = sound_colours(player.page_sounds(k, app.kit_page()), k);
                out.note(&format!(
                    "kit {}: {} ({} sounds)",
                    k + 1,
                    kits[k].name,
                    kits[k].sounds.len()
                ));
                // With the synth chosen for the pads, its colours stay
                // (below).
                if !app.pads_chose_synth() && app.pads_holder().is_none() {
                    ports.send_batch(
                        out,
                        false,
                        &format!("kit {}'s pad colours", k + 1),
                        &as_slices(&pad_colours(&rest)),
                    )?;
                }
            }
            lit = [None; 16];
            mute_shown = None;
            // The arp plays the synth's layout notes, or a kit's pad slots.
            app.set_pad_layout(synth_on.then(|| synth.pad_layout()));
            out.show(gui::ToUi::Kit(k));
            bank_lights = true;
            kit_switched = true;
        }
        // What the pads play (app/pads_focus.rs): the synth chosen for them
        // plays on its own while the selected track plays something else,
        // and through the track while it plays the synth; otherwise the
        // track's instrument. A pad still held on the synth ends its note
        // as the pads leave it, and its let-go is taken quietly (above).
        // From step 6 the pads play an instrument of the host's chosen for
        // them by its pads player (set where the app's effects are taken):
        // on their own while the armed track doesn't play it.
        let host_now = mix
            .instruments
            .pads_play_at()
            .is_some_and(|h| mix.instruments.owner(app.track()) != Some(h));
        // A kit chosen for them (sampler step 15) the same way: on their
        // own while the armed track doesn't play that kit itself.
        let sel = app.track();
        let kit_now = app.pads_kit().filter(|&k| {
            let through = !mix.instruments.plays(sel) && !midi_out.takes(sel as u16);
            !(through && track_kits.kit_of(sel) == Some(k))
        });
        let preview_now = if app.pads_chose_synth() && !synth_on {
            Some(PadsPreview::Synth)
        } else if host_now {
            Some(PadsPreview::Host)
        } else {
            kit_now.map(|k| PadsPreview::Kit(k.min(usize::from(u8::MAX)) as u8))
        };
        if preview_now != pads_preview || (preview_now.is_some() && kit_switched) {
            // The pads' arp's notes end where they played (sampler step
            // 15c); its next hit starts it on what the pads play now.
            let (track, page) = (sel as u8, app.kit_page());
            pads_arp.leave(&mut |to, n, rec| {
                play_pads_arp(
                    to,
                    n,
                    track,
                    page,
                    &mut synth,
                    &mut mix.instruments,
                    &mut midi_out,
                    &mut mix.pad_synths,
                    &mut player,
                );
                rec_pads_arp(to, n, rec, &mut time);
            });
            if preview_now != Some(PadsPreview::Synth) {
                for pad in (0..16).filter(|&p| preview_held[p] == Some(PadsPreview::Synth)) {
                    synth.note_off(pad, 0);
                }
            }
            // The pads held on a kit they leave let go there, as the
            // synth's (their let-go is taken quietly, above).
            for pad in 0..16 {
                if let Some(PadsPreview::Kit(k)) = preview_held[pad] {
                    if preview_now != preview_held[pad] {
                        let kit_pad = usize::from(preview_pad[pad]);
                        let off = mpfree_play::Event::NoteOff {
                            note: kit_pad as u8,
                        };
                        play_routed_pad(
                            sel as u8,
                            kit_pad,
                            off,
                            &mut midi_out,
                            (&mut mix.pad_synths, &mut synth),
                            &mut player,
                            usize::from(k),
                        );
                    }
                }
            }
            pads_preview = preview_now;
            // The Sample page follows a hit on a kit the pads play on
            // their own (sampler step 15d).
            out.show(gui::ToUi::PadsKit(match pads_preview {
                Some(PadsPreview::Kit(k)) => Some(usize::from(k)),
                _ => None,
            }));
            let preview_on = pads_preview.is_some();
            if let Some(PadsPreview::Kit(k)) = pads_preview {
                let k = usize::from(k);
                rest = sound_colours(player.page_sounds(k, app.kit_page()), k);
            } else if preview_on {
                rest = synth.pad_colours();
            } else if !synth_on {
                rest = sound_colours(
                    player.page_sounds(player.kit(), app.kit_page()),
                    player.kit(),
                );
            }
            if mute_shown.is_none() {
                ports.send_batch(
                    out,
                    false,
                    "the pads' colours",
                    &as_slices(&pad_colours(&rest)),
                )?;
                lit = [None; 16];
            }
        }
        if page_turned && !synth_on {
            // The kit's next page: its sounds' colours, unless the pads
            // are in a mode (they come back with these) or play the synth
            // chosen for them (they come back with these once they follow
            // the track, above).
            // A kit chosen for the pads turns its own pages (sampler step
            // 15).
            let kit_on_pads = match pads_preview {
                Some(PadsPreview::Kit(k)) => Some(usize::from(k)),
                None => Some(player.kit()),
                _ => None,
            };
            if let Some(k) = kit_on_pads {
                rest = sound_colours(player.page_sounds(k, app.kit_page()), k);
            }
            if mute_shown.is_none() && kit_on_pads.is_some() {
                ports.send_batch(
                    out,
                    false,
                    "the page's pad colours",
                    &as_slices(&pad_colours(&rest)),
                )?;
                lit = [None; 16];
            }
            out.show(gui::ToUi::PadPage(app.pad_page()));
        }
        // A bank's own kit from another of its banks (sampler step 15e):
        // the screen's Sample page edits the pads of that bank.
        let source_now = Some(app.kit_page()).filter(|&p| p != app.pad_page());
        if source_now != kit_source {
            kit_source = source_now;
            out.show(gui::ToUi::KitSource(source_now));
        }
        // The pads' kit layers: the Sample page's bar names them after a
        // hit (sampler step 15e's follow-up).
        let layer_kits = app.pads_layer_kits();
        if layer_kits_shown != Some(layer_kits) {
            layer_kits_shown = Some(layer_kits);
            out.show(gui::ToUi::PadsLayerKits(layer_kits));
        }
        // The PAD BANK buttons found so far: the bank playing is lit, or
        // TRACK MUTE's bank while the pads mute tracks.
        if bank_lights && ((app.pads_are_steps() && app.main_held().is_none()) || controller.on()) {
            // STEP SEQ's pages, or controller mode's banks: sent with the
            // pads' lights below.
            pages_shown = None;
        } else if bank_lights {
            let bank = Some(if let Some(b) = app.main_held() {
                // MAIN held: which 16 tracks the pads pick from.
                b % 4
            } else if app.mute_keys() == app::MuteKeys::Tracks {
                app.mute_bank() % 4
            } else if app.pads_are_perform() {
                // The Perform set's page.
                app.perform_bank() % 4
            } else if synth_on {
                synth.settings().bank % 4
            } else {
                app.pad_page() % 4
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
                    if cv_out.light(&light) {
                        ports.tap(true, &light);
                    } else {
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
        if app.set_perform(time.perform()) {
            out.show(gui::ToUi::Rec(app.rec));
        }
        // Perform: a take brings Rec song with it (sequencer S4).
        if let Some(line) = time.take_follows(desk::rec_now(&time, &looper).taking()) {
            out.note(&format!("time: {line}"));
        }
        app.set_track_count(time.track_count());
        // Knob owners (step 4b): the tracks' lasting ids say which track's
        // lock moves a knob two tracks lock.
        let ranks = track_ranks(time.project());
        mix.instruments.set_ranks(&ranks);
        synth.set_ranks(&ranks);
        // A track UNDO took away sets its kit aside (its place plays kit 1
        // again, for a track added there later), and a REDO that brings it
        // back gives it its kit again, so the two go together (the screen
        // hears of it with the track kits below).
        let (redos, can_redo) = time.redos();
        track_kits.follow(time.track_count(), redos != redos_seen, can_redo);
        redos_seen = redos;
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
        // before the transport runs, stamped half a period before its top
        // (they came during the period before). A take ready now records
        // from the leader's Start (sequencer S5).
        time.set_leader(music.leader_name());
        time.take_waits(app.rec.ready.map(|take| take == rec_arm::Take::Add));
        let mut took = None;
        music.leader_clock(|follow, bytes| {
            time.follow(follow);
            if !bytes.is_empty() {
                took = time.clock_read(PERIOD as u32, bytes);
            }
        });
        if let Some(line) = took {
            out.note(&line);
        }
        // Ignored while a followed clock arrives.
        time.follow_tempo(app.settings.bpm);
        // The transport runs first; the clips and the arps follow its clock.
        time.advance(PERIOD as u32);
        // An export's MIDI file: each control may take a value again this
        // period (before the player's pass, which taps a slow-gear track's
        // locks and lanes, and the mixer's, which taps the mod engine's).
        export.tap.period(time.block().start_frame);
        // A take the leader's Start began isn't ready any more.
        if app.rec.ready.is_some() && time.info().state == mpfree_transport::State::Recording {
            app.rec.ready = None;
            out.show(gui::ToUi::Rec(app.rec));
        }
        // A pad take gets its go once the music plays.
        fx.extend(desk::pad_rolled(&mut app, &time, &looper, out));
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
        // The tracks on gear slower than the sound's lead play their clips
        // that much ahead (sequencer S5).
        time.set_ahead(&midi_out.ahead());
        // A clip's instrument locks play only on the kind of instrument
        // they were made for (step sequencer refactor F1).
        let n = time.track_count().min(inst_now.len());
        track_instruments(
            &mut inst_now[..n],
            &mix.instruments,
            &track_kits,
            &synth,
            &midi_out,
        );
        time.set_instruments(&inst_now[..n]);
        // A take on a track whose instrument plays notes records notes
        // (instrument redesign step 5c), the shared synth's too (step
        // sequencer refactor E4).
        // A kit played as notes makes its tracks notes tracks (sampler
        // step 15c, part 3).
        kit_notes = sampler::KitNotes::of(&player);
        let own = &mix.instruments;
        sync_notes(
            &mut time,
            &mut app,
            own,
            &track_kits,
            &kit_notes,
            &synth,
            &midi_out,
        );
        // The note editor's Scale option offers the pads' scale and the
        // owner's own scales (piano roll plan 2B.1).
        let (pads, own) = app.scale_offers();
        time.set_scale_offers(pads, own);
        // CV out takes the outputs' jobs; run, reset and clock follow the
        // transport.
        cv_out.sync(&io_model);
        cv_out.clock(time.block());
        // The selected track is the one the pads and the arp screens play
        // (the armed one unless a track was picked with the arm pinned).
        let selected = time.selected().unwrap_or(0);
        if selected != app.track() {
            let before = app.track();
            fx.extend(app.set_track(selected));
            // The pads play what the newly selected track plays.
            kit_follow = Some(track_kits.entry(selected));
            // The Synth tab follows the selected track's own synth (or the
            // pads' instrument of the host's, step 6).
            if mix.instruments.plays(selected)
                || mix.instruments.plays(before)
                || mix.instruments.pads_play_at().is_some()
            {
                out.show(gui::ToUi::Synth(synth_tab(
                    &synth,
                    &mix,
                    tab_who(&app, &mix.instruments),
                    &track_presets,
                )));
            }
        }
        // The pads press the notes of the selected track's own synth into
        // its arp while that plays notes (step 5d), or of its kit while
        // that is played as notes, the bank's (sampler step 15c, part 3).
        let own = &mix.instruments;
        let pads_own = pads_preview.is_some();
        let layout = press_layout(&app, own, &track_kits, &kit_notes, &midi_out, pads_own);
        app.set_arp_layout(layout);
        if let Some(app::Effect::Arp(v)) = app.arp_poll() {
            out.show(gui::ToUi::Arp(Box::new(v)));
        }
        // A track's own instrument the builder made goes in (the old one
        // rings out), and retired ones go back to be dropped; before the
        // notes, so it plays this period's. The synth is home here.
        if let Some(landed) = mix.poll_instruments() {
            match landed {
                mixing::Landed::Playing { track, name } => {
                    if track < instruments::TRACKS {
                        out.note(&format!("track {}: plays {name}", track + 1));
                    } else {
                        let n = track - instruments::TRACKS + 1;
                        out.note(&format!("free place {n}: plays {name}"));
                    }
                    ask_plugin_presets(&mut mix);
                    let shown = instrument_track(&mix.instruments, tab_who(&app, &mix.instruments));
                    if track == shown || track == app.track() {
                        out.show(gui::ToUi::Synth(synth_tab(
                            &synth,
                            &mix,
                            track,
                            &track_presets,
                        )));
                    }
                }
                mixing::Landed::Failed { track, id } => {
                    let s = format!(
                        "{}: {id} couldn't be made (see the Console); it keeps what it had",
                        holder_says(track)
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
            // A track that played another's instrument plays its own now.
            app.set_track_sends(&sends_now(&mix.instruments));
        }
        // The synth-voice pads' own synths: what the builder made goes in,
        // a few times a second the pads that want one are looked at again
        // (a pad made or changed on the Pads tab, a kit loaded, a new
        // design on the Synth tab), and the copies follow the Synth tab's
        // values.
        if let Some(landed) = mix.take_pad_landed() {
            note_pad_landed(landed, out);
        }
        if i.is_multiple_of(PAD_SYNTH_PLAN) {
            plan_pad_synths(&player, &synth, &mut mix, out);
        }
        mix.pad_synths.follow(&synth);
        // A preset the preset picker tried while the builder was busy.
        track_presets.poll(&synth, &mut mix);
        // A plugin synth's presets listed, or one loaded, on the builder
        // thread. A plugin may load in the background and send its new
        // values over the next blocks, so the Synth tab looks again a
        // second and three seconds later.
        if let Some(news) = mix.poll_presets() {
            let shown = match news {
                control::PresetNews::Listed { id, list } => {
                    if let Err(e) = &list {
                        out.note(&format!("plugins: {id}: presets not listed: {e}"));
                    }
                    mix.instruments
                        .held(app.track())
                        .is_some_and(|h| h.id == id)
                        .then_some(app.track())
                }
                control::PresetNews::Loaded {
                    track,
                    instance,
                    index,
                    name,
                    result,
                } => {
                    let same = mix
                        .instruments
                        .held(track)
                        .is_some_and(|h| h.instance == instance);
                    match result {
                        Ok(()) if same => {
                            track_presets.set_index(track, Some(index));
                            out.note(&format!("track {}: preset {name}", track + 1));
                            preset_looks.retain(|(t, _)| *t != track);
                            preset_looks.push((track, t_read + Duration::from_secs(1)));
                            preset_looks.push((track, t_read + Duration::from_secs(3)));
                        }
                        Ok(()) => out.note(&format!(
                            "track {}: preset {name} went into a plugin the track no longer plays",
                            track + 1
                        )),
                        Err(e) => {
                            let s = format!("Track {}: preset {name} didn't load: {e}", track + 1);
                            out.note(&s);
                            out.show(gui::ToUi::Tell(s));
                        }
                    }
                    (track == app.track()).then_some(track)
                }
                control::PresetNews::Values {
                    track,
                    instance,
                    values,
                } => {
                    let held = mix
                        .instruments
                        .held(track)
                        .filter(|h| h.instance == instance && track == app.track());
                    // No answer still clears the ask, so the next one goes.
                    let (values, texts) = values.unwrap_or_default();
                    if let Some(h) = held {
                        out.show(gui::ToUi::SkinValues {
                            id: h.id.to_string(),
                            values,
                            texts,
                        });
                    } else {
                        out.show(gui::ToUi::SkinValues {
                            id: String::new(),
                            values: Vec::new(),
                            texts: Vec::new(),
                        });
                    }
                    None
                }
            };
            if let Some(track) = shown {
                out.show(gui::ToUi::Synth(synth_tab(
                    &synth,
                    &mix,
                    track,
                    &track_presets,
                )));
            }
        }
        if preset_looks.iter().any(|(_, at)| *at <= t_read) {
            let track = app.track();
            if preset_looks
                .iter()
                .any(|(t, at)| *t == track && *at <= t_read)
            {
                out.show(gui::ToUi::Synth(synth_tab(
                    &synth,
                    &mix,
                    track,
                    &track_presets,
                )));
            }
            preset_looks.retain(|(_, at)| *at > t_read);
        }
        // The mod engine knows the tracks' MIDI effects as the sound loop
        // has them, and its values on their settings (macros, LFOs, step
        // locks) from the last period reach the arps' chains in this one.
        if let Some(fx) = time.note_fx_moved() {
            mix.modu.set_note_fx(fx);
        }
        for (addr, value) in mix.modu.take_note_values() {
            time.modulate_note_fx(addr, Some(value));
        }
        // Sampler R5.3b: the live hits on Loop pads that waited for the
        // launch line play on it, through the arp as when they were made.
        let dropped = time.due_pads(&mut due_pads);
        note_stop_dropped(dropped, &mut stop_dropped, out);
        for &(w, offset) in due_pads.as_slice() {
            fx.extend(play_waited(
                &mut app,
                &mut time,
                &mut awaiting,
                w,
                offset,
                out,
            ));
        }
        // The clips and every track's arp (the pads go through the armed
        // track's) play into the kit or the synth: the clips as recorded,
        // the arps' notes as live playing (SeqPads::live).
        // A track gone silent (muted, or another soloed) ends the notes its
        // kit's Instrument and MIDI out pads hold: they sound outside its
        // mixer strip, so its mute wouldn't reach them.
        let silent = time.silenced();
        let went = silent & !silent_was;
        silent_was = silent;
        for t in (0..128).filter(|t| went & 1 << t != 0) {
            midi_out.end_pads(t, 0);
            synth.end_pad_voices(t as u16, 0);
            mix.pad_synths.end_track(t as u16, 0);
        }
        let key = synth.transpose_key();
        let mut seq = SeqPads {
            player: &mut player,
            synth: &mut synth,
            kits: &track_kits,
            shown: track_kits.entry(app.track()),
            flashes: [0; 128],
            hits: 0,
            live: app.live_map(),
            live_hits: [(0, 0); MAX_LIVE_HITS],
            n_live: 0,
            silent: 0,
            midi: &mut midi_out,
            cv: &mut cv_out,
            own: &mut mix.instruments,
            modu: &mix.modu,
            pad_synths: &mut mix.pad_synths,
            only: export.only(),
            tap: &mut export.tap,
            shifts: time.transposes(),
            key,
            by_note: &mut by_note,
        };
        time.period_arps(PERIOD as u32, &mut seq, &mut app.arps);
        // MIDI out's period ends after the mixer's (below), which has the
        // tracks' controllers out.
        seq.cv.period_end(PERIOD as u32, t_read);
        for (due, out, value) in seq.cv.sent() {
            music
                .feed
                .mpc(Dir::Out, ports.at(due), Mpc::Cv { out, value });
        }
        // The time engine: the transport, the clips and every track's arp
        // (the pads' notes start here; their sounds are the sampler's or
        // the synth's).
        engines.stop(engines::TIME, time_span);
        parts.begin();
        // The pads' own hits: the first sound of a press shows the pad
        // down; the arp's repeats flash it.
        let page = app.kit_page();
        for &(pad, v) in &seq.live_hits[..seq.n_live] {
            let pad = usize::from(pad);
            // The pad it is on the page; a repeat latched on another page
            // plays unseen.
            let cell = pad_cell(pad, page, !synth_on);
            if let Some(c) = cell.filter(|&c| awaiting[c]) {
                awaiting[c] = false;
                out.show(gui::ToUi::PadDown {
                    pad: c,
                    velocity: v,
                });
            } else {
                repeats += 1;
                window_repeats += 1;
                window_pads[pad.min(127)] = true;
                if let Some(c) = cell {
                    out.show(gui::ToUi::PadFlash {
                        pad: c,
                        velocity: v,
                    });
                }
            }
        }
        silent_hits += seq.silent;
        hits += seq.hits;
        // The pads the clips lit, kept so the mixer can be reached below.
        let seq_flashes = seq.flashes;
        // The pads' arp (sampler step 15c): the selected track's arp as the
        // app set it, on the period the clips' arps had, playing where the
        // pads play on their own; its notes flash their pads. On the shared
        // synth, whose pads press notes, it plays the track's phrase, and
        // the Phrase tab's cursor follows it.
        app.set_pads_own(
            matches!(pads_preview, Some(PadsPreview::Synth)),
            pads_arp.phrase_at(),
        );
        pads_arp.follow(
            app.arps.engine_at(app.track()),
            app.pads_own_mode(),
            app.arp_own_swing(),
        );
        {
            let (track, page) = (app.track() as u8, app.kit_page());
            // A copy (no heap): the notes record into `time` as they play.
            let block = time.block().clone();
            pads_arp.period(&block, &mut |to, n, rec| {
                let lit = play_pads_arp(
                    to,
                    n,
                    track,
                    page,
                    &mut synth,
                    &mut mix.instruments,
                    &mut midi_out,
                    &mut mix.pad_synths,
                    &mut player,
                );
                rec_pads_arp(to, n, rec, &mut time);
                if let (Some(pad), ArpNote::On { velocity, .. }) = (lit, n) {
                    hits += 1;
                    repeats += 1;
                    window_repeats += 1;
                    out.show(gui::ToUi::PadFlash { pad, velocity });
                }
            });
        }
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
            let steps = time.steps();
            if app.step_pads().is_some() {
                // The step pads' lights read them too.
                step_view = Some(steps.clone());
            } else {
                step_view = None;
            }
            out.show(gui::ToUi::Steps(Box::new(steps)));
        }
        for (pad, &v) in seq_flashes.iter().enumerate().filter(|(_, v)| **v > 0) {
            if let Some(c) = pad_cell(pad, app.kit_page(), !synth_on) {
                out.show(gui::ToUi::PadFlash {
                    pad: c,
                    velocity: v,
                });
            }
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
        if let Some(line) = time.take_later_note() {
            out.note(&format!("time: {line}"));
        }
        // What the stream thread says about the waiting cues' audio.
        tracks.wait_answers(|key, a| match a {
            CueAudio::Held => {
                time.audio_held(key, true);
            }
            CueAudio::Short => {}
            CueAudio::GivenBack => {
                time.audio_given_back(key);
            }
        });
        // The audio tracks' regions as the time engine refuses blocks
        // over them (phase 2, step 2d), when they changed.
        if let Some(spans) = tracks.spans_if_changed(time.project()) {
            time.set_audio_spans(spans);
        }
        cut_blocks_under_takes(out, &mut time, &mut tracks);
        // Fit to tempo on an audio clip's card: once its copies are in,
        // the clip plays them as one UNDO step, held while the time engine
        // records (an edit then would go among the take's).
        if fit_done.is_none() {
            fit_done = tracks.tend_clip_fit();
        }
        let recording = matches!(
            time.info().state,
            mpfree_transport::State::Recording | mpfree_transport::State::CountIn
        );
        if let Some(done) = fit_done.take_if(|_| !recording) {
            let line = fit_ended(&mut time, &mut tracks, done);
            out.note(&format!("audio tracks: {line}"));
            out.show(gui::ToUi::Tell(line));
        }
        // Put in a cell: once the sound is in, its clip goes in the cell.
        if cell_done.is_none() {
            cell_done = tracks.tend_cell_sound(time.info().bpm);
        }
        if let Some(done) = cell_done.take_if(|_| !recording) {
            let line = cell_sound_in(&mut time, &mut tracks, done);
            out.note(&format!("audio tracks: {line}"));
            out.show(gui::ToUi::Tell(line));
        }
        // A region just moved is picked again where it went.
        if let Some(r) = tracks.take_repick() {
            let _ = time.act(TimeAction::Audio(AudioAction::Pick(Some(r))));
        }
        // `|`, not `||`: both flags are taken.
        if time.take_changed() | tracks.take_changed() {
            // The takes the clips name stay (the audio tracks rethink,
            // phase 2); the next look tells the tracks.
            tracks.keep_named(time.recordings());
            tracks.clip_starts(time.project());
            let st = time_state(&time, &tracks);
            if st.audio_waits.needs_rev != needs_rev {
                needs_rev = st.audio_waits.needs_rev;
                tracks.hold_waits(&time.audio_needs());
            }
            pad_grid = perform_pads::Grid::of(&st);
            out.show(gui::ToUi::Time(Box::new(st)));
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
            let fresh = sound_colours(
                player.page_sounds(player.kit(), app.kit_page()),
                player.kit(),
            );
            for pad in 0..16 {
                if fresh[pad] != rest[pad] {
                    rest[pad] = fresh[pad];
                    if lit[pad].is_none() && mute_shown.is_none() {
                        ports.send(false, &akai::pad_colour(pad as u8, rest[pad]))?;
                    }
                }
            }
        }
        // STEP SEQ: the steps held go to the screen (the Q-Links set
        // them); COPY let go forgets the step picked.
        if !app.copy_held() {
            step_keys.forget_copy();
        }
        if let Some(held) = step_keys.held_news() {
            out.show(gui::ToUi::StepsHeld(held));
        }
        // STEP SEQ: the pads show the steps of the sound on them, and with
        // Follow on, turn the page with the playhead.
        let keys = app.mute_keys();
        let step_pads = app.step_pads().filter(|_| app.pads_are_steps());
        if app.step_pads().is_none() {
            // Off: fetched again when they come back.
            step_view = None;
        }
        if controller.on() {
            // Controller mode: the pads that send a note in the bank
            // showing are blue, bright while held; PAD BANK shows the bank.
            let want: [[u8; 3]; 16] =
                std::array::from_fn(|pad| match (controller.plays(pad), controller.held(pad)) {
                    (false, _) => [0; 3],
                    (true, false) => CONTROLLER_PAD,
                    (true, true) => akai::hit_colour(CONTROLLER_PAD),
                });
            for pad in 0..16 {
                if mute_shown.is_none_or(|s| s[pad] != want[pad]) {
                    ports.send(false, &akai::pad_colour(pad as u8, want[pad]))?;
                }
            }
            mute_shown = Some(want);
            let pages = step_keys::bank_lights(controller.pad_bank(), None, false);
            if pages_shown != Some(pages) {
                for (b, note) in app.map.bank.iter().enumerate() {
                    let Some(note) = *note else { continue };
                    if pages_shown.is_some_and(|w| w[b] == pages[b]) || !akai::app_lights(note) {
                        continue;
                    }
                    let light = akai::button_light(note, pages[b]);
                    if cv_out.light(&light) {
                        ports.tap(true, &light);
                    } else {
                        ports.send(true, &light)?;
                    }
                }
                pages_shown = Some(pages);
            }
        } else if let Some(bank) = app.main_held() {
            // MAIN held: the pads are tracks, the one picked white.
            pages_shown = None;
            let first = bank * 16;
            let want = track_pick_colours(time.selected().map(|a| a.wrapping_sub(first)), |p| {
                time.mute_light(first + p)
            });
            for pad in 0..16 {
                if mute_shown.is_none_or(|s| s[pad] != want[pad]) {
                    ports.send(false, &akai::pad_colour(pad as u8, want[pad]))?;
                }
            }
            mute_shown = Some(want);
        } else if let Some(s) = step_pads {
            let steps = step_view.get_or_insert_with(|| time.steps());
            let playing = step_keys::playing(steps, time.info().position);
            if let Some(p) = playing.filter(|_| s.follow) {
                fx.extend(app.follow_page(p / app::step_pads::PAGE));
            }
            // The sound's colour: the armed track's kit's (any of its 128
            // pads), or on the synth's line its pen's pad.
            let colour = akai::hit_colour(if synth_on {
                s.pen
                    .pads()
                    .first()
                    .and_then(|&p| rest.get(usize::from(p)))
                    .copied()
                    .filter(|c| *c != [0; 3])
                    .unwrap_or(akai::kit_colour(synth_kit))
            } else {
                akai::kit_colour(player.kit())
            });
            let want = step_keys::colours(steps.clip.as_ref(), s, colour, playing);
            for pad in 0..16 {
                if mute_shown.is_none_or(|s| s[pad] != want[pad]) {
                    ports.send(false, &akai::pad_colour(pad as u8, want[pad]))?;
                }
            }
            mute_shown = Some(want);
            // The PAD BANK lights: the page shown, and the page playing
            // blinks (twice a second).
            let playing_page = playing.map(|p| p / app::step_pads::PAGE);
            let blink = ((t * 4.0) as u64).is_multiple_of(2);
            let pages = step_keys::bank_lights(s.page, playing_page, blink);
            if pages_shown != Some(pages) {
                for (b, note) in app.map.bank.iter().enumerate() {
                    let Some(note) = *note else { continue };
                    if pages_shown.is_some_and(|w| w[b] == pages[b]) || !akai::app_lights(note) {
                        continue;
                    }
                    let light = akai::button_light(note, pages[b]);
                    if cv_out.light(&light) {
                        ports.tap(true, &light);
                    } else {
                        ports.send(true, &light)?;
                    }
                }
                pages_shown = Some(pages);
            }
        } else if app.pads_are_perform() {
            // The loop keys, lit by their loops' states (and the set's
            // other pads by theirs), on the pads or on the Pads tab.
            pages_shown = None;
            let blink = ((t * 4.0) as u64).is_multiple_of(2);
            let lit = perform_pads::Lit {
                throws: app.throws_on(),
                macros: app.macros_on(),
                muted: (0..time.project().tracks.len().min(128))
                    .filter(|&t| time.muted(t) == Some(true))
                    .fold(0u128, |m, t| m | 1 << t),
                grid: pad_grid.lights(app.perform_page(), time.info().position),
            };
            let want = perform_pads::colours(
                app.perform_page(),
                &perform_pads::lights(&looper),
                &lit,
                blink,
            );
            for pad in 0..16 {
                if mute_shown.is_none_or(|s| s[pad] != want[pad]) {
                    ports.send(false, &akai::pad_colour(pad as u8, want[pad]))?;
                }
            }
            mute_shown = Some(want);
        } else if keys != app::MuteKeys::Off {
            pages_shown = None;
            let kit = player.kit();
            let first = app.mute_bank() * 16;
            let track = |t: usize| time.mute_light(first + t);
            let blink = ((t * 4.0) as u64).is_multiple_of(2);
            // Only kit 1's pads 1-16 have strips, so mutes.
            let off = no_mutes(&track_kits, mix.instruments.note_tracks(), app.track()).is_some();
            let (t, page) = (app.track(), app.kit_page());
            let pad = |p: usize| pad_light(&time, t, page * 16 + p, off);
            let want = mute_colours(
                keys,
                sound_colours(player.page_sounds(kit, app.kit_page()), kit),
                track,
                pad,
                blink,
            );
            for pad in 0..16 {
                if mute_shown.is_none_or(|s| s[pad] != want[pad]) {
                    ports.send(false, &akai::pad_colour(pad as u8, want[pad]))?;
                }
            }
            mute_shown = Some(want);
        } else if app.pads_transpose() {
            // 16 Levels' Transpose: the step the armed track is on.
            pages_shown = None;
            let by = time.armed().and_then(|a| {
                let t = time.project().tracks.get(a)?;
                let own = &mix.instruments;
                plays_notes(t, a, &synth, &track_kits, own).then(|| time.transposes()[a].by)
            });
            let want = transpose_colours(by);
            for pad in 0..16 {
                if mute_shown.is_none_or(|s| s[pad] != want[pad]) {
                    ports.send(false, &akai::pad_colour(pad as u8, want[pad]))?;
                }
            }
            mute_shown = Some(want);
        } else if mute_shown.take().is_some() {
            pages_shown = None;
            // Left: the pads' own colours again, lit below where they play.
            ports.send_batch(
                out,
                false,
                "the pads' colours",
                &as_slices(&pad_colours(&rest)),
            )?;
            lit = [None; 16];
        }
        // Pad lights: lit while held, while the arp keeps them going (latched, or held on its grid),
        // or while a pad set to Latch in the sampler plays on.
        // The page's pads latched (pads 16 * page + 1 on).
        let latched = if synth_on {
            0
        } else {
            (player.latched_pads() >> (app.kit_page() * 16)) as u16
        };
        // A Loop pad's hit waiting for the launch line blinks amber (twice
        // a second, as a cued clip's mute key does) until it sounds, and
        // the screen's pad has an amber outline (sampler R5.3b).
        let waiting = time.hits_waiting(app.kit_page());
        if waiting != waiting_shown {
            waiting_shown = waiting;
            out.show(gui::ToUi::PadsWaiting(waiting));
        }
        let amber = akai::KIT_COLOURS[0];
        let blink = ((t * 4.0) as u64).is_multiple_of(2);
        for pad in 0..16 {
            if mute_shown.is_some() {
                break;
            }
            let want = if waiting & (1 << pad) != 0 {
                Some(if blink {
                    akai::hit_colour(amber)
                } else {
                    amber
                })
            } else if held[pad]
                || (app.generating() && app.active(pad))
                || latched & (1 << pad) != 0
            {
                Some(akai::hit_colour(rest[pad]))
            } else {
                None
            };
            if want != lit[pad] {
                lit[pad] = want;
                let colour = want.unwrap_or(rest[pad]);
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
        // Each instrument plays its own tuning or the song's (tunings
        // step T5), handed over when its pick changes.
        for line in
            tunings_handed.follow(app.tunings(), &listed, &synth, &mut mix, Some(&mut player))
        {
            out.note(&line);
        }
        // A synced pad LFO keeps to the beats while the transport rolls.
        player.set_beat(time.block().spans().first().and_then(|s| {
            s.state
                .rolling()
                .then(|| s.tick_at(s.frame) / f64::from(mpfree_transport::PPQ))
        }));
        // Each pad of kit 1 into its pad channel (Pad Mix), which all go
        // to the Drums track; another kit into the channel of the first
        // track that plays it.
        // With a synth lane (Cores 3, 2 and 1) the synth starts on its core
        // now and renders beside the drums; otherwise this only notes the
        // frame and tempo, and it renders below as before.
        // The synth's and the effects' tempo is the transport's: the
        // project's, a followed clock's, or the song's tempo points and
        // ramps while song mode plays them.
        synth.start(i * PERIOD as u64, time.info().bpm);
        // The list of instruments follows the tracks' own and the free
        // places': a new one gets an id, one taken out loses its (the app
        // hears of it next period, above). Never allocates.
        list_changed |= listed.follow(mix.instruments.held_tracks())
            | listed.follow_free(mix.instruments.held_free());
        let homes = track_kits.homes(|t| !mix.instruments.plays(t) && !midi_out.takes(t as u16));
        // A kit no track plays sounds on a kit strip of its own (sampler
        // step 15d, part 2).
        mix.strip_kits(&app.named_kits(), &homes);
        if mix.strips_full() && !strips_full {
            out.note(
                "mixer: every kit strip is taken; a kit no track plays sounds on the pad channels",
            );
        }
        strips_full = mix.strips_full();
        if let Some((kit, track)) = mix.take_kit_moved() {
            out.note(&format!(
                "mixer: kit {kit}'s strip settings moved with it to track {track}'s strip"
            ));
        }
        // One period in a few, the kits' voices, the instruments and the
        // inserts count their cost for System status (track_costs.rs);
        // the period after, they stop.
        match i % track_costs::EVERY {
            0 => {
                player.count_kits(true);
                mix.set_spending(true);
            }
            1 => {
                player.count_kits(false);
                mix.set_spending(false);
            }
            _ => {}
        }
        let mut pads = mix.pads_homed(&homes);
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
        // What the looper records: the channel of what the pads play, or
        // the mix picked, from the mix of the last period, a take's source
        // held to its end (`loop_input`).
        let midi = midi_out.takes(app.track() as u16);
        let pads = pads_instrument(&app, &mix, &track_kits, midi);
        for (line, tell) in loop_input(
            &mut looper,
            &mut mix,
            pads,
            &homes,
            &mut pads_l,
            &mut pads_r,
        )
        .into_iter()
        .flatten()
        {
            out.note(&line);
            if tell {
                out.show(gui::ToUi::Tell(looper_says(&line)));
            }
        }
        // Length: Pattern follows the pattern playing, a few times a
        // second (it changes with a launch or an edit of a clip's length).
        if i.is_multiple_of(32) {
            looper.set_pattern_bars(pattern_bars(&time));
        }
        // The loop, for its track; the line input comes with it. First
        // the Loop tracks' launches, so a loop goes on or off on its bar
        // line's own frame (loop-tracks.md).
        let t_loop = Instant::now();
        for line in looper.follow_plan(&time.loop_plan()) {
            out.note(&line);
        }
        looper.set_tempo_from_loop(tempo_from_loop_allowed(
            app.settings.loop_tempo,
            &time,
            export.running(),
        ));
        looper.set_tempo_meant(app.tempo_by_hand);
        looper.set_smart_end(app.settings.smart_end);
        // The period's notes, for smart loop end.
        if let Some(at) =
            time.take_notes(|frame, track, key, velocity| looper.note(frame, track, key, velocity))
        {
            looper.notes_lost(at);
        }
        // A snap: its tempo asked of the time engine, or, committed, the
        // loop told to change on the same frame.
        out.loop_start(looper.tend_snap(&mut time, tracks.host.tracks.has_regions()));
        // Make song: the song started on the free loops' bar line (or its
        // tempo set), or its UNDO.
        let (said, song_tempo) = looper.tend_song(&mut time, export.running(), loops_on);
        out.loop_start(said);
        // Loops on audio tracks (Dev's switch, phase 4b): the track
        // player plans where each track's loops sound from what the
        // arranger says, and each loop a loop cell names is told its runs
        // before the looper runs.
        if loops_on {
            tracks.sync(time.project(), time.silenced(), false);
            tracks.loops_moving(&looper.moving_ids());
            tracks.plan_loops(time.block(), &|p| time.audio(p));
            looper.give_loop_id(|| tracks.fresh_loop_id());
            looper.give_layer_recs(|| tracks.fresh_recording_id());
            // Closed layers to their files soon after they close (4c.3).
            if let Some(line) = looper.write_layers(&mut tracks, time.block().frames as usize) {
                out.note(&line);
            }
            looper.tell_tracks(time.project(), tracks.loop_player());
        }
        let line_note = looper.run(time.block(), &mut loop_l, &mut loop_r);
        // A loop that heard the line input with the song stopped starts it.
        let take = app.rec.ready.map(|take| take == rec_arm::Take::Add);
        out.loop_start(loop_starts_song(
            &mut looper,
            &mut time,
            take,
            export.running(),
        ));
        // A free first loop closed: the song starts at its tempo.
        let (said, tempo) = loop_sets_tempo(&mut looper, &mut time, take, export.running());
        out.loop_start(said);
        out.smart_end(looper.take_smart());
        // Or it waits on its Length: the hint line asks (after what smart
        // loop end said, so the question is what stays).
        out.loop_start(looper.take_asks());
        // A snap landed: the song's tempo changed with the loop.
        let (said, snapped) = looper.take_snap();
        out.loop_start(said);
        // Make song landed, or why not.
        out.loop_start(looper.take_song());
        loop_tempo = tempo.or(snapped).or(song_tempo).or(loop_tempo);
        // The audio tracks record the same line input onto the Song view's
        // lanes, and each plays into its own mixer channel (song mode only).
        tracks.follow_latency(app.settings.loop_latency);
        // The audio tracks in the track list (phase 1a): each engine track
        // follows its project track, and is silent while that is muted or
        // left out by a solo.
        tracks.sync(time.project(), time.silenced(), false);
        // Tracks recording from inside the mix (step 9) take the mixer's
        // last block for them, before they run.
        tracks.feed_inside(&mut mix);
        // What each audio track plays (its clips, its song or nothing)
        // comes from the player's last period, on its frames.
        if loops_on {
            // Each loop on a track into its track (phase 4b).
            tracks.period_with_loops(
                time.block(),
                time.tempo_map(),
                time.song_on(),
                looper.input(),
                &|p| time.audio(p),
                &|id| looper.track_loop(id),
            );
        } else {
            tracks.period(
                time.block(),
                time.tempo_map(),
                time.song_on(),
                looper.input(),
                Some(&|p| time.audio(p)),
            );
        }
        // Each audio track into its own channel (phase 1b).
        tracks.play_into(&mut mix);
        // The strip's waveform, built a slice at a time: its cost counts in
        // the looper's stage, so the stage numbers show it.
        let loop_wave = looper.wave_step(i, LOOP_WAVE_PERIODS);
        let loop_took = t_loop.elapsed();
        loop_time.add(loop_took);
        engines.add(engines::LOOPER, loop_took);
        // The same line input into the sampler's buffer, with the bar lines
        // (the Sample page's recorder). A point in the mix
        // goes in after the mixer, below.
        // Turned by the recorder's Gain, ramped over the period.
        let gain = (rec_gain_was, rec_gain);
        rec_gain_was = rec_gain;
        if let (Some(f), sampler::RecSource::Line) = (rec_feed.as_mut(), rec_source) {
            let t_rec = Instant::now();
            let (l, r) = looper.input();
            f.period_gain(time.block(), l, r, 0, gain);
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
        // A Perform FX the set put on and no throw plays on now, taken
        // off once it was in; one put on that came in: the Perform view's
        // Q-Links can turn its throws now.
        if desk::tell_throw_said(&mut mix, out) {
            out.show(desk::throw_knobs(&mix, app.perform_set()));
        }
        // A switch of mode that came in: Q-Links and MIDI controls learned
        // on the insert's controls follow them.
        for landed in mix.modes.take_landed() {
            music.retarget(&landed);
            out.show(gui::ToUi::ParamsMoved(Box::new(landed)));
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
            let defaults = project::Defaults {
                tempo: app.settings.bpm,
                mixer: project::mixer_text(&mix.mixer, mix.pad_mutes()),
                modu: mix.modu.save(),
                synth: project::synth_text(&app.synth_settings(), &synth.sounds()),
            };
            // A scripted run that needs the demo songs: put in the data
            // folder's library with these parts, on a thread of its own.
            if let (Some(w), Some(dir), false) = (&o.workflow, &o.data, songs_put) {
                if w.needs.iter().any(|n| n == script::DEMO_SONGS) {
                    songs_put = true;
                    script::put_demo_songs(dir.clone(), defaults.clone(), out.sink.clone());
                }
            }
            defaults_sent = out
                .try_show(gui::ToUi::ProjectDefaults(Box::new(defaults)))
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
        if let Some(line) = tracks.take_file_line() {
            out.note(&line);
        }
        while let Some(r) = tracks.take_problem() {
            out.show(gui::ToUi::Problem(r));
        }
        if let Some(what) = tracks.take_left_out() {
            out.left_out(what);
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
            audio_held = tracks.take_saved(time.project());
            audio_saving = audio_held.is_none();
        }
        if collect_now && !loops_saving && !audio_saving {
            collect_now = false;
            let (loops, audio) = (loops_held.take(), audio_held.take());
            parts_out = Some(gui::ToUi::ProjectParts(Box::new(project::Collected {
                tempo: app.settings.bpm,
                swing: time.info().swing,
                clips: time.project().clone(),
                mixer: mix.mixer_text(),
                modu: mix.modu.save(),
                synth: project::synth_text(&app.synth_settings(), &synth.sounds()),
                arp: app.arp_part(),
                perform: app.perform_set().text(),
                xy: xy_yours.text(),
                instruments: project::instruments_text(&mix.instruments),
                track_kits: project::track_kits_text(
                    &track_kits,
                    &mix.instruments,
                    &listed,
                    time.project().tracks.len(),
                ),
                instrument_list: listed.text(),
                pads: app.pads_part(),
                tunings: app.tunings_part(),
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
        // Monitor's level, for the Sample page's recorder, when it moves.
        let line_level = mix.line_level();
        if line_level_said != Some(line_level)
            && out.try_show(gui::ToUi::LineLevel(line_level)).is_ok()
        {
            line_level_said = Some(line_level);
        }
        let ctx = mixing::ctx(RATE, i * PERIOD as u64, time.info().bpm, time.block());
        // The mixer's lanes (each stage's inserts, the returns), the
        // tracks' instruments' lanes and the pads' synths' keep the same
        // deadline.
        mix.mixer.set_lane_deadline(Some(lane_until));
        mix.instruments.set_lane_deadline(Some(lane_until));
        mix.pad_synths.set_lane_deadline(Some(lane_until));
        let (p, caught) = mix.period(
            &ctx,
            (&synth_l, &synth_r),
            (&loop_l, &loop_r),
            looper.input(),
            &mut left,
            &mut right,
        );
        if caught {
            clipped += 1;
        }
        // A set's song ending: its fade, and silence once over, before
        // Sample Recall and the metronome (song_end.rs).
        if song_end.active() {
            if let Some(n) = song_end.period(p, &mut left[..], &mut right[..]) {
                out.show(gui::ToUi::SongEnd(n));
            }
        }
        // The kit's pads whose mute or solo came due land from the next
        // period, which holds their line.
        // MIDI out's period: the tracks' controllers, bend and pressure the
        // mod engine moved in the mixer's (a step's lock, a lane, a macro,
        // a learned knob) join this period's notes on their frames, a
        // step's ahead of its note.
        // Each track's CC scale and CC smooth as they are now (macros and
        // LFOs on their settings too) shape its controls from here: only
        // the tracks with a port and channel, so 128 tracks cost what
        // the routed ones do.
        let mut routed = midi_out.routed();
        while routed != 0 {
            let t = routed.trailing_zeros() as usize;
            routed &= routed - 1;
            midi_out.set_note_fx(t, &time.note_fx_now(t));
        }
        // An export's MIDI file takes them too, as the lanes give them.
        let period_frame = time.block().start_frame;
        for (offset, addr, value) in mix.modu.take_midi_values() {
            export
                .tap
                .control(period_frame + u64::from(offset), addr, value);
            midi_out.control(offset, addr, value);
        }
        midi_out.period_end(PERIOD as u32, t_read);
        // The MIDI monitor: what went out, when it leaves.
        for (due, port, m) in midi_out.sent() {
            let to = if m.is_from_page() {
                To::PortPage
            } else if m.is_from_sampler() {
                To::Sampler
            } else if m.is_from_controller() {
                To::Controller
            } else if m.is_sent_back() {
                To::SentBack
            } else if let Some(from) = m.thru_from() {
                To::ThruFrom(from)
            } else {
                m.track().map_or(To::Clock, To::SentBy)
            };
            music.feed.sent(port, ports.at(due), m.bytes(), to);
        }
        // A capture of an effect's memory that has ended goes to the
        // Sample page, which frees it; with no room it waits a period.
        if let Some((held, end)) = mix.take_captured() {
            if let Err(gui::ToUi::Captured(held, end)) =
                out.try_show(gui::ToUi::Captured(held, end))
            {
                mix.keep_captured(held, end);
            }
        }
        // Sample Recall keeps the master as heard (after Master safety,
        // before the metronome), with the bar lines as late as it hears them.
        let t_rec = Instant::now();
        recall_feed.period(time.block(), &left, &right, mix.mixer.latency());
        engines.add(engines::SAMPLER, t_rec.elapsed());
        if let Some(m) = recall_out.take() {
            recall_out = out.try_show(m).err();
        }
        // The metronome goes on top; it keeps the samples within ±1 itself.
        let p = p.max(time.mix_click(&mut left, &mut right));
        engines.stop(engines::MIX, t_mix);
        // A counted period: where its time went, by track (track_costs.rs).
        // A kit goes to the first track that plays it (kit 1 too, which
        // sounds on the pad channels), the Synth tab's synth likewise.
        if track_costs::TrackCosts::counts(i) {
            let plays = |t: usize| !mix.instruments.plays(t) && !midi_out.takes(t as u16);
            let mut owners = homes;
            if owners[0] == 0 {
                if let Some(t) = track_kits.first_on(0, plays) {
                    owners[0] = (t + 1) as u8;
                }
            }
            let synth_by = track_kits
                .first_on(track_kits.synth(), plays)
                .map(|t| (t, synth_took.as_nanos() as u64));
            let pads_ns = (t_synth - t_render).as_nanos() as u64;
            track_costs.add_period(
                |u| player.take_kit_counts(u),
                pads_ns,
                synth_by,
                &owners,
                |sink| mix.take_spent(sink),
            );
        }
        // Export: the pass takes this period's capture points and notes;
        // once it is over, the transport goes back to where it was and the
        // pass goes to the screen to finish (export.rs).
        if export.running() {
            export.notes(time.block());
            if let Some(run) = export.period(&mut mix.mixer, &mut time) {
                out.show(gui::ToUi::ExportOver(run));
            }
        }
        // Freeze and Print: the take under way and the frozen strips
        // (takes.rs). A print goes to the screen to be saved.
        let n_tracks = time.project().tracks.len();
        match mix.takes_period(&ctx, n_tracks, &|t| time.now_playing(t)) {
            Some(takes::Said::Note(l)) => {
                let l = format!("mixer: {l}");
                out.note(&l);
                out.show(gui::ToUi::Tell(l));
            }
            Some(takes::Said::Printed(name, p)) if print_out.is_none() => {
                print_out = Some(gui::ToUi::Printed(name, p));
            }
            Some(takes::Said::Printed(name, p)) => {
                // One waits already (the screen is that busy): this one
                // is dropped off this thread with the builder's next job.
                out.note(&format!(
                    "mixer: {name}: print not saved, the screen was busy"
                ));
                mix.drop_printed(p);
            }
            None => {}
        }
        if let Some(m) = print_out.take() {
            print_out = out.try_show(m).err();
        }
        // Resampling: the point in the mix the recorder records (before the
        // metronome), with the bar lines as late as that point hears them.
        if let (Some(f), sampler::RecSource::Mix(..)) = (rec_feed.as_mut(), rec_source) {
            if let Some((l, r, latency)) = mix.rec_capture() {
                let t_rec = Instant::now();
                f.period(time.block(), l, r, latency);
                engines.add(engines::SAMPLER, t_rec.elapsed());
            }
        }
        parts.begin();
        if let Some(mut st) = mix.poll_state(Instant::now()) {
            // The audio tracks' strips, named and lit as their tracks.
            tracks.dress_strips(&mut st, time.project(), time.silenced(), &mix);
            // Loops on audio tracks (Dev's switch, phase 4, board 6).
            if app.settings.loops_on {
                let plays = |t: usize| {
                    let a = time.audio(t);
                    match a.changes().last().map_or(a.start, |c| c.plays) {
                        mpfree_play::AudioPlays::Loop { clip, .. } => Some((true, clip)),
                        mpfree_play::AudioPlays::Clip { clip, .. } => Some((false, clip)),
                        _ => None,
                    }
                };
                tracks.dress_loop_strips(&mut st, time.project(), plays, &mix);
            }
            out.show(gui::ToUi::Mixer(Box::new(st)));
        }
        panels.poll(
            app.view(),
            &mix,
            &|t| modpanels::NotesSource::of(&app, &time, t),
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
        // A loop recorded lands on a Loop track (or, with loops on audio
        // tracks, in an audio track's cell armed for it, by its lasting
        // id); its cells follow it.
        if let Some(now) = looper.grid_now() {
            let lines = if loops_on {
                time.loops_are_on(&now, &looper.loop_ids())
            } else {
                time.loops_are(&now)
            };
            if loops_on && !lines.is_empty() {
                looper.look_again();
            }
            for line in lines {
                out.note(&format!("time: {line}"));
            }
        }
        // A loop's copy onto another Loop track ended.
        if let Some(end) = looper.copy_ended() {
            fx.extend(desk::copy_ended(&mut time, end));
        }
        // The one UNDO follows each engine's steps (phase 4d.1), and asks
        // them to forget what it can no longer reach (4d.3). With the
        // switch off it still brings back a layer that left memory while
        // it was on, for the loop's own UNDO (4d.4).
        if loops_on {
            if let Some(next) = one_undo.follow(&time, &tracks, &looper) {
                out.show(gui::ToUi::OneUndo(next));
            }
            fx.extend(one_undo.tend(out, &mut time, &mut tracks, &mut looper));
        } else {
            fx.extend(one_undo.tend_back(out, &mut tracks, &mut looper));
        }
        if let Some(wave) = loop_wave {
            out.show(gui::ToUi::LoopWave(wave));
        }
        // The Looper tab and Play's loop row follow the loops as often
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
        // Soundcheck's Muted: everything above ran and was measured;
        // nothing leaves the outputs (Main out and the headphones).
        if outputs_muted {
            left.fill(0.0);
            right.fill(0.0);
        }
        for f in 0..PERIOD {
            frames[2 * f] = float_to_s32(left[f]);
            frames[2 * f + 1] = float_to_s32(right[f]);
        }
        let xruns = play.xruns;
        parts.lap(parts::SCREEN);
        engines.stop(engines::PERIOD, period_span);
        let work = last_written.elapsed();
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
        last_written = written;
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
        // A synth, a track's instrument or a pad's synth left out of this
        // period (late) comes back before anything plays it again.
        synth.settle(lane_until);
        mix.instruments.settle(lane_until);
        mix.pad_synths.settle(lane_until);
        // A period behind, with the queue running down: give up the voices
        // nobody hears (shed.rs). An underrun in this write emptied the
        // queue. A fast export is never behind (the card is stopped and
        // the loop waits for the files), so its file keeps every voice.
        shed.set_queued(PERIOD, play.queued);
        if !fast && work > period_time() {
            let full = i64::from(play.queued) * PERIOD as i64;
            let short = match &delay {
                Ok(d) if play.xruns == xruns => Some(full - i64::from(*d)),
                Ok(_) => Some(full),
                Err(_) => None,
            };
            let behind = shed::Period {
                work,
                short,
                voices: player.playing() + synth.active_voices() + mix.instruments.voices(),
                voice_time: (t_synth - t_render) + synth_took + mix.instrument_time,
            };
            let n = shed.decide(&behind);
            if n > 0 {
                let mut engines: [&mut dyn shed::Voices; 3] =
                    [&mut player, &mut *synth, &mut mix.instruments];
                shed.give_up(n, &mut engines);
            }
        }
        if let Some(line) = shed.note(written) {
            out.note(&line);
        }
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
                // Kit pads by number (pads 1 to 128), or the synth's.
                let list: Vec<String> = (0..128)
                    .filter(|&p| window_pads[p])
                    .map(|p| (p + 1).to_string())
                    .collect();
                let latched: Vec<String> = (0..16)
                    .filter(|&p| app.active(p) && !held[p])
                    .map(|p| (app.kit_pad(p) + 1).to_string())
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
            window_pads = [false; 128];
        }
        // Every 30 s, the totals so far, for the system log (the app runs until the power goes off, so it
        // never prints a final summary; the workstation's paced check reads these).
        if status_now || last_status.elapsed() >= Duration::from_secs(30) {
            last_status = Instant::now();
            status_now = false;
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
            out.note(&format!("status: {}", mix.describe_xy()));
            if let Some(line) = found_plugins.summary().and_then(|s| s.line()) {
                out.note(&format!("status: plugins: {line}"));
            }
            if let Some(line) = mix.instruments.describe() {
                out.note(&format!("status: tracks' own instruments: {line}"));
            }
            if let Some(line) = mix.pad_synths.describe() {
                out.note(&format!("status: synth-voice pads' own synths: {line}"));
            }
            out.note(&format!(
                "status: {}",
                engines::log_line(&engines.lifetime())
            ));
            out.note(&format!(
                "status: {}",
                cores.status(cores::Counts::read(&player, &mix, &synth))
            ));
            out.note(&format!("status: {}", parts.status()));
            out.note(&format!("status: {}", shed.describe()));
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
                        given_up: shed.totals().given_up,
                        dropped: c.dropped_events + app.arps.dropped(),
                        late_let_gos: c.late_let_gos,
                        merged_values: c.merged_values,
                        ends_lost: time.ends_lost()
                            + synth.counters().ends_lost
                            + mix.instruments.ends_lost()
                            + midi_out.ends_lost(),
                        peak_grains: c.peak_grains,
                        window_waits: c.window_waits,
                        track_files: tracks.stream_numbers(),
                        faded_grains: c.faded_grains,
                        held_back_clouds: c.held_back_clouds,
                        crowded_s: c.crowded_ms as f32 / 1000.0,
                        clock_lost: time.clock_lost(),
                    }
                },
            }));
            window_peak = 0.0;
            engines.publish();
            track_costs.publish(&engine_shared.tracks);
            let (failed, dropped, stalls) = ports.queue_counts();
            engine_shared
                .parts
                .set_counts(failed, dropped, stalls, out.report_dropped());
            // The second core's late parts: on the Dev page, and in the log
            // within a second (a fallback is never silent).
            let counts = cores::Counts::read(&player, &mix, &synth);
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
                        out.show(gui::ToUi::Problem(mpfree_status::Report::happened(
                            &mpfree_status::kinds::SAVE_FAILED,
                            1,
                        )));
                    }
                }
            }
            // The I/O view's rig changed: kept for the next start.
            if let Some(text) = midi_saver.take(&io_model) {
                if store.saves() {
                    let file = midi_saved::RIG;
                    if let Err(e) = store.write(midi_saved::OWNER, file, text.into_bytes()) {
                        out.note(&format!("not saved: midi/{file}: {e}"));
                        out.show(gui::ToUi::Problem(mpfree_status::Report::happened(
                            &mpfree_status::kinds::SAVE_FAILED,
                            1,
                        )));
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
            let sends = sends_now(&mix.instruments);
            if sends != sends_shown {
                sends_shown = sends;
                out.show(gui::ToUi::TrackSends(Box::new(sends)));
            }
            let plan = desk::rec_plan(&app, &time, &mix);
            if plan != plan_shown {
                plan_shown = plan;
                out.show(gui::ToUi::RecPlan(plan.map(Box::new)));
            }
            if track_kits != kits_shown {
                kits_shown = track_kits.clone();
                out.show(gui::ToUi::TrackKits(Box::new(track_kits.clone())));
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
    let ahead = time.ahead_counts();
    if ahead != Default::default() {
        out.say(&format!(
            "MIDI out ahead (gear slower than the sound's lead): {} clip events sent early, {} \
             late (changed inside the look-ahead), {} notes ended (sent early, then not played), \
             {} found no room and went late",
            ahead.ahead, ahead.late, ahead.wrong, ahead.dropped
        ));
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
    if keys_direct > 0 {
        out.say(&format!(
            "MIDI in: {keys_direct} keys played the kit's keygroups at once (not recorded)"
        ));
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
    out.say(&mix.describe_xy());
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
    out.say(&cores.status(cores::Counts::read(&player, &mix, &synth)));
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
    // DSP quality step 0.3: on a stick that starts the pad player at power-on
    // it holds this device, so inventory sees it busy; ask the same read-only
    // questions here, on the device just opened, before the stream is set up.
    for line in rate_probe::probe(&pcm) {
        out.say(&format!("rate probe {path}: {line}"));
    }
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
    let gain = 10f32.powf(o.gain_db() / 20.0);
    out.say(&format!(
        "playing a 120 BPM click for {} s at {:.0} dB (peak about {:.0} dBFS); \
                      keep the volume knob low",
        o.seconds,
        o.gain_db(),
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
/// the MPC's GPU and in software. With `--theme all` each page is drawn in
/// every built-in theme in turn, into DIR/THEME/NAME.mpdl, building the
/// screens once (tests/theme_contrast.rs). No devices are opened; it writes
/// only new files.
fn display_lists(o: &Options) -> Result<(), String> {
    let dir = o.display_lists.as_ref().unwrap();
    let fonts = || {
        Fonts::with_files(
            o.font.as_deref(),
            o.bold_font.as_deref(),
            o.mono_font.as_deref(),
        )
    };
    let themes: Vec<(PathBuf, Theme)> = if o.theme.as_deref() == Some("all") {
        mpfree_ui::theme::BUILTIN
            .iter()
            .filter_map(|&n| Some((dir.join(n), Theme::builtin(n)?)))
            .collect()
    } else {
        vec![(dir.clone(), load_theme(o)?)]
    };
    for (d, _) in &themes {
        fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    fonts()?;
    let mut pages = 0;
    for (name, mut p) in gui::sample_screens(|| fonts().unwrap_or_else(|_| Fonts::embedded())) {
        for (d, theme) in &themes {
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
            let path = d.join(format!("{name}.mpdl"));
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .and_then(|mut f| f.write_all(&bytes))
                .map_err(|e| format!("{}: {e}", path.display()))?;
        }
        pages += 1;
    }
    println!("display lists written: {pages} pages in {}", dir.display());
    Ok(())
}

/// The runs of text under 3:1 on page `p` drawn in theme `t` (the
/// contrast check's measure, `mpfree_ui::lint`).
fn unreadable_runs(p: &mut gui::Player, t: &Theme) -> Result<Vec<mpfree_ui::lint::Run>, String> {
    use mpfree_ui::lint::{text_runs, FLOOR};
    p.ui.set_theme(t.clone());
    let (w, h) = p.ui.size();
    let mut rec = mpfree_ui::record::Recorder::new(w, h);
    let mut bytes = Vec::new();
    rec.begin_frame();
    p.ui.paint(&mut rec);
    rec.encode_frame(&mut bytes);
    let frames = mpfree_ui::record::Frame::decode_all(&bytes)?;
    Ok(text_runs(&frames[0])
        .into_iter()
        .filter(|r| r.ratio < FLOOR)
        .collect())
}

/// The built-in theme a theme file starts from (its first line's `base`),
/// by name, or the default theme.
fn base_theme(text: &str) -> (String, Theme) {
    text.lines()
        .map(|l| l.split('#').next().unwrap_or("").trim())
        .find(|l| !l.is_empty())
        .and_then(|l| l.split_once('='))
        .filter(|(k, _)| k.trim() == "base")
        .and_then(|(_, v)| {
            let v = v.trim().to_ascii_lowercase();
            Theme::builtin(&v).map(|t| (v, t))
        })
        .unwrap_or_else(|| ("default".to_string(), Theme::dark()))
}

/// The page `--contact-sheet` writes: each screen's name over the two
/// pictures, `theme/NAME.png` (labelled `theme`) and `base/NAME.png`
/// (labelled `base`), side by side. One self-contained file with no
/// scripts, readable in light and dark browsers, that opens from disk.
fn contact_sheet_html(theme: &str, base: &str, names: &[&str]) -> String {
    let esc = |t: &str| {
        t.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    };
    let (theme, base) = (esc(theme), esc(base));
    let mut html = format!(
        "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <title>{theme} beside {base}</title>\n<style>\n\
         :root {{ color-scheme: light dark; --bg: #EEF0F3; --card: #FFFFFF; --line: #D3D8E0; --fg: #161A21; --dim: #4D5565; }}\n\
         @media (prefers-color-scheme: dark) {{ :root {{ --bg: #0F1217; --card: #181C23; --line: #2A303B; --fg: #E8EBF0; --dim: #A2AABA; }} }}\n\
         body {{ margin: 0; background: var(--bg); color: var(--fg); font: 15px/1.5 system-ui, sans-serif; }}\n\
         main {{ max-width: 1480px; margin: 0 auto; padding: 24px 16px 48px; }}\n\
         h1 {{ font-size: 26px; margin: 0 0 6px; }}\n\
         p {{ margin: 0 0 18px; color: var(--dim); }}\n\
         .grid {{ display: grid; gap: 16px; grid-template-columns: repeat(auto-fill, minmax(min(100%, 640px), 1fr)); }}\n\
         section {{ background: var(--card); border: 1px solid var(--line); border-radius: 10px; padding: 10px; }}\n\
         h2 {{ font: 500 14px ui-monospace, monospace; margin: 0 0 8px; }}\n\
         .pair {{ display: grid; gap: 8px; grid-template-columns: 1fr 1fr; }}\n\
         @media (max-width: 560px) {{ .pair {{ grid-template-columns: 1fr; }} }}\n\
         figure {{ margin: 0; }}\n\
         img {{ display: block; width: 100%; height: auto; border: 1px solid var(--line); border-radius: 6px; }}\n\
         figcaption {{ font: 12px ui-monospace, monospace; color: var(--dim); margin-top: 4px; }}\n\
         </style></head><body><main>\n\
         <h1>{theme} beside {base}</h1>\n\
         <p>Every sample screen MPFree draws ({n}), {theme} on the left and {base} on the right. \
         Open a picture for its full size (1280 &times; 800).</p>\n<div class=\"grid\">\n",
        n = names.len()
    );
    for name in names {
        let name = esc(name);
        html.push_str(&format!(
            "<section><h2>{name}</h2><div class=\"pair\">\
             <figure><a href=\"theme/{name}.png\"><img loading=\"lazy\" src=\"theme/{name}.png\" alt=\"{name} in {theme}\"></a><figcaption>{theme}</figcaption></figure>\
             <figure><a href=\"base/{name}.png\"><img loading=\"lazy\" src=\"base/{name}.png\" alt=\"{name} in {base}\"></a><figcaption>{base}</figcaption></figure>\
             </div></section>\n"
        ));
    }
    html.push_str("</div>\n</main></body></html>\n");
    html
}

/// `--contact-sheet DIR` with `--theme NAME|FILE`: every sample screen in
/// the theme and in the theme it starts from (a file's `base`, else
/// Default; Light for Default itself), as new PNG files in DIR/theme and
/// DIR/base, and DIR/index.html to look at them side by side. What a
/// theme writer looks over before sharing a theme, and what the owner saw
/// Light on. Writes only new files; opens no devices.
fn contact_sheet(o: &Options) -> Result<(), String> {
    let dir = o.contact_sheet.as_ref().unwrap();
    let theme = load_theme(o)?;
    let given = o.theme.clone().unwrap_or_else(|| "default".to_string());
    let (base_name, base) = match Theme::builtin(&given) {
        Some(_) if given.eq_ignore_ascii_case("default") => ("light".to_string(), Theme::light()),
        Some(_) => ("default".to_string(), Theme::dark()),
        None => base_theme(&fs::read_to_string(&given).map_err(|e| format!("{given}: {e}"))?),
    };
    let label = if theme.name.trim().is_empty() {
        given.clone()
    } else {
        theme.name.clone()
    };
    let base_label = Theme::builtin(&base_name).map_or(base_name.clone(), |t| t.name);
    let fonts = || {
        Fonts::with_files(
            o.font.as_deref(),
            o.bold_font.as_deref(),
            o.mono_font.as_deref(),
        )
    };
    fonts()?;
    for sub in ["theme", "base"] {
        let d = dir.join(sub);
        fs::create_dir_all(&d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    let mut names = Vec::new();
    for (name, mut p) in gui::sample_screens(|| fonts().unwrap_or_else(|_| Fonts::embedded())) {
        for (sub, t) in [("theme", &theme), ("base", &base)] {
            p.ui.set_theme(t.clone());
            let (w, h) = p.ui.size();
            let mut canvas = SoftCanvas::new(w, h);
            p.ui.paint(&mut canvas);
            write_png(&dir.join(sub).join(format!("{name}.png")), &canvas)?;
        }
        names.push(name);
    }
    let page = dir.join("index.html");
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&page)
        .and_then(|mut f| f.write_all(contact_sheet_html(&label, &base_label, &names).as_bytes()))
        .map_err(|e| format!("{}: {e}", page.display()))?;
    println!(
        "contact sheet written: {} screens, {label} beside {base_label}, {}",
        names.len(),
        page.display()
    );
    Ok(())
}

/// `--check-theme FILE`: what someone writing a theme needs to know,
/// without an MPC. Every problem the file has, by line (`Theme::load`);
/// whether the app would offer it in Prefs > Appearance or leave it out
/// (as [`load_theme_files`] does: only below the floor); and every sample
/// screen where
/// more text is under 3:1 than in the theme the file starts from, each
/// run with its colours and place. Only reads FILE; opens no devices.
/// Ends with an error when the app would leave the file out.
fn check_theme(o: &Options) -> Result<(), String> {
    let path = o.check_theme.as_ref().unwrap();
    let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let (theme, problems) = Theme::load(&text);
    let name = if theme.name.trim().is_empty() {
        path.display().to_string()
    } else {
        theme.name.clone()
    };
    // The report is printed at the end, after the lines the sample
    // screens print as they are built.
    let mut out = vec![format!("theme {name} ({})", path.display())];
    for p in &problems {
        let kind = match p.kind {
            mpfree_ui::theme::ProblemKind::Mistake => "mistake",
            mpfree_ui::theme::ProblemKind::Unknown => "unknown key",
            mpfree_ui::theme::ProblemKind::HardToRead => "hard to read",
            mpfree_ui::theme::ProblemKind::Ignored => "ignored",
            mpfree_ui::theme::ProblemKind::Unusable => "unusable",
        };
        out.push(format!("  {kind}: {p}"));
    }
    if problems.is_empty() {
        out.push("  no problems".to_string());
    }
    // As the themes folder reads it: only a theme no one could read is
    // left out; a line with a mistake uses the base's value.
    let refused = problems
        .iter()
        .find(|p| p.kind == mpfree_ui::theme::ProblemKind::Unusable)
        .map(|p| p.text.clone());
    match &refused {
        Some(e) => out.push(format!(
            "the app leaves this file out of Prefs > Appearance: {e}"
        )),
        None => out.push("the app offers it in Prefs > Appearance".to_string()),
    }
    // The theme the file starts from, for a fair comparison: faint text
    // that is under 3:1 there too is the screen's, not the file's.
    let base = base_theme(&text);
    let fonts = || {
        Fonts::with_files(
            o.font.as_deref(),
            o.bold_font.as_deref(),
            o.mono_font.as_deref(),
        )
    };
    fonts()?;
    let (mut pages, mut worse, mut runs, mut base_runs) = (0, 0, 0, 0);
    for (page, mut p) in gui::sample_screens(|| fonts().unwrap_or_else(|_| Fonts::embedded())) {
        let mine = unreadable_runs(&mut p, &theme)?;
        let theirs = unreadable_runs(&mut p, &base.1)?;
        pages += 1;
        runs += mine.len();
        base_runs += theirs.len();
        if mine.len() > theirs.len() {
            worse += 1;
            out.push(format!(
                "{page}: {} run(s) of text under 3:1 ({}: {})",
                mine.len(),
                base.0,
                theirs.len()
            ));
            for r in &mine {
                out.push(format!(
                    "  #{:06X} on #{:06X} at {},{} ({} glyph(s), {:.1}:1)",
                    r.ink.xrgb(),
                    r.under.xrgb(),
                    r.x,
                    r.y,
                    r.glyphs,
                    r.ratio
                ));
            }
        }
    }
    out.push(format!(
        "screens: {pages} drawn, {runs} run(s) of text under 3:1 ({}: {base_runs}), \
         {worse} page(s) with more than {}",
        base.0, base.0
    ));
    println!("{}", out.join("\n"));
    match refused {
        Some(e) => Err(format!("{}: {e}", path.display())),
        None => Ok(()),
    }
}

/// `--script FILE`: the workflow, read and checked before anything starts,
/// and the run's length from it (a few seconds more, for the last status
/// lines and the sounds' tails).
fn read_script(mut o: Options) -> Result<Options, String> {
    let Some(path) = &o.script else {
        return Ok(o);
    };
    let text = fs::read_to_string(path).map_err(|e| format!("--script {}: {e}", path.display()))?;
    let w =
        script::Script::parse(&text).map_err(|e| format!("--script {}: {e}", path.display()))?;
    o.seconds = w.seconds().ceil() as u64 + script::TAIL_S;
    o.workflow = Some(w);
    Ok(o)
}

fn main() {
    // `mpfree plughost ...`: this program serving as one plugin's process
    // for the plugin sandbox (plugins.rs), before anything else starts.
    let mut raw = std::env::args_os().skip(1);
    match raw.next() {
        Some(a) if a == plugins::SUBCOMMAND => std::process::exit(plugins::serve(raw)),
        // `mpfree copy-image FROM TO`: a creator stick's backup copy, run by
        // its root service (imagecopy.rs), before anything else starts.
        Some(a) if a == imagecopy::SUBCOMMAND => std::process::exit(imagecopy::main(raw)),
        _ => {}
    }
    // A panic says where and why in the report before the program ends
    // (crash.rs): from here on, for every command.
    crash::note_panics(
        Path::new(REPORT_DIR).join("report.txt"),
        Path::new(REPORT_DIR).join(crash::RUNNING),
    );
    // The sampler's job service, before anything asks it for work.
    mpfree_sampler::jobs::configure(sample_page::jobs_config());
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = parse_args(&args).and_then(read_script).and_then(|o| {
        // Both draw every sample screen, whose one frame needs more stack
        // than the main thread has in a debug build (gui::SCREENS_STACK).
        if o.preview.is_some() {
            return gui::on_screens_stack(|| preview(&o));
        }
        if o.display_lists.is_some() {
            return gui::on_screens_stack(|| display_lists(&o));
        }
        if o.check_theme.is_some() {
            return gui::on_screens_stack(|| check_theme(&o));
        }
        if o.contact_sheet.is_some() {
            return gui::on_screens_stack(|| contact_sheet(&o));
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

    /// Sampler step 15f: a kit layer's track records a hit as the kit's
    /// pad of the layer's bank, and a key as the pad that sends it; on a
    /// kit played as notes, both as the notes they play. A key its
    /// keygroups take on a kit of pads plays but isn't kept.
    #[test]
    fn a_kit_layer_records_a_hit_and_a_key_as_its_kit_takes_them() {
        use mpfree_sampler::engine::KitKeys;
        use mpfree_sampler::model::KeysPlay;
        let pads = LayerKit {
            kit: 1,
            page: 2,
            keys: KitKeys::PADS,
        };
        assert_eq!(pads.pad_note(3), Some(35), "bank C's pad 4");
        assert_eq!(pads.key_note(37), Some(0), "C#2 sends A1");
        let both = LayerKit {
            keys: KitKeys {
                play: KeysPlay::Both,
                held: 1 << 60,
                pads_from: None,
            },
            ..pads
        };
        assert_eq!(both.key_note(60), None, "its keygroup's key isn't kept");
        assert_eq!(both.key_note(37), Some(0), "the rest send their pads");
        let notes = LayerKit {
            keys: KitKeys {
                play: KeysPlay::Keygroups,
                held: 0,
                pads_from: Some(48),
            },
            ..pads
        };
        assert_eq!(notes.pad_note(3), Some(48 + 35), "the note C4 plays");
        assert_eq!(notes.key_note(61), Some(61), "a key as itself");
    }

    /// 16 Levels' Transpose on the pads (S4): the step the armed track is
    /// on is lit, pad 9 dim white, the rest dim blue; past the pads none
    /// is lit, and with nothing to transpose they are dark.
    #[test]
    fn transpose_lights_the_step_the_armed_track_is_on() {
        let on = [0x3F, 0x28, 0x00];
        let c = transpose_colours(Some(5));
        assert_eq!((c[13], c[8], c[0]), (on, [0x0C; 3], [0x00, 0x06, 0x0E]));
        assert_eq!(transpose_colours(Some(0))[8], on);
        assert_eq!(transpose_colours(Some(-8))[0], on);
        assert!(!transpose_colours(Some(12)).contains(&on), "past the pads");
        assert_eq!(transpose_colours(None), [[0; 3]; 16]);
    }

    /// Runs `f` on the clips' and the arps' way to the sounds
    /// ([`SeqPads`]) with `tracks` on the Synth kit, and gives back what
    /// the synth was asked to start and end, in order.
    fn on_the_synth(tracks: &[usize], f: impl FnOnce(&mut SeqPads)) -> Vec<mpfree_node::NodeEvent> {
        on_the_pads(tracks, &[], f)
    }

    /// [`on_the_synth`], with the tracks `own` playing their own Poly
    /// synth (a copy of the synth's sound, as the builder thread makes).
    fn on_the_pads(
        tracks: &[usize],
        own: &[usize],
        f: impl FnOnce(&mut SeqPads),
    ) -> Vec<mpfree_node::NodeEvent> {
        let tone: Vec<i16> = (0..2_000)
            .flat_map(|i| [((i as f32 * 0.03).sin() * 12_000.0) as i16; 2])
            .collect();
        let kick = kit::build(&[(mpc_one::PAD_NOTES[4], "Kick", &tone[..])]);
        let mut ks = kit::parse_kits(&kit::build_kits(&[("House", kick)])).unwrap();
        let mut player = sampler::Player::new(&mut ks, &mpc_one::PAD_NOTES, RATE).unwrap();
        let mut synth = synth::Synth::new(RATE);
        let mut kits = sampler::TrackKits::new(1);
        for &t in tracks {
            assert!(kits.set(t, kits.synth()));
        }
        let mut mix = mixing::MixHost::new(RATE, 0.0, 6.0);
        for &t in own {
            let engine = synth::Engine::Poly;
            let (id, extras, values) = synth.sound_of(engine);
            let plugins = plugins::Plugins::none();
            let made = instruments::make(&plugins, id, &extras, &values, RATE, PERIOD).unwrap();
            let pads = synth.pads_for(engine);
            let tuning = synth.tuning_now();
            assert!(mix.instruments.put(t, made, pads, &tuning).is_ok());
        }
        let mut midi = midi_out::MidiOut::new(RATE, MIDI_OUT_LEAD);
        let mut cv = cv_out::CvOut::new(RATE, MIDI_OUT_LEAD);
        let mut tap = export::Tap::new();
        let mut by_note = vec![mpfree_arp::scale::Held::new(); mpfree_seq::MAX_TRACKS];
        let live = app::App::new(RATE, false).live_map();
        assert!(
            !live.plays_notes(),
            "the synth's pads by pad, as on the Synth kit"
        );
        let mut seq = SeqPads {
            player: &mut player,
            synth: &mut synth,
            kits: &kits,
            shown: 0,
            flashes: [0; 128],
            hits: 0,
            live,
            live_hits: [(0, 0); MAX_LIVE_HITS],
            n_live: 0,
            silent: 0,
            midi: &mut midi,
            cv: &mut cv,
            own: &mut mix.instruments,
            modu: &mix.modu,
            pad_synths: &mut mix.pad_synths,
            only: None,
            tap: &mut tap,
            shifts: [mpfree_seq::Transpose::default(); mpfree_seq::MAX_TRACKS],
            key: None,
            by_note: &mut by_note,
        };
        f(&mut seq);
        synth
            .queued()
            .into_iter()
            .filter(|e| {
                matches!(
                    e,
                    mpfree_node::NodeEvent::NoteOn { .. } | mpfree_node::NodeEvent::NoteOff { .. }
                )
            })
            .collect()
    }

    /// Step sequencer refactor E4: a take on a track that plays the shared
    /// synth holds notes while the synth plays notes, as one on a track's
    /// own synth does; a track on a kit holds pads.
    #[test]
    fn a_track_on_the_shared_synth_records_notes() {
        let synth = synth::Synth::new(RATE);
        let mut kits = sampler::TrackKits::new(1);
        assert!(kits.set(2, kits.synth()));
        let mut mix = mixing::MixHost::new(RATE, 0.0, 6.0);
        // Track 4 plays its own Poly.
        let engine = synth::Engine::Poly;
        let (id, extras, values) = synth.sound_of(engine);
        let made = instruments::make(
            &plugins::Plugins::none(),
            id,
            &extras,
            &values,
            RATE,
            PERIOD,
        )
        .unwrap();
        let pads = synth.pads_for(engine);
        let tuning = synth.tuning_now();
        assert!(mix.instruments.put(3, made, pads, &tuning).is_ok());
        assert!(synth.plays_notes());
        let none = sampler::KitNotes::default();
        assert_eq!(
            notes_tracks(&mix.instruments, &kits, &synth, &none, 8),
            1 << 2 | 1 << 3,
            "Track 3 on the shared synth, Track 4 on its own"
        );
        // Kit 1 played as notes (sampler step 15c, part 3): every track on
        // it too.
        let mut kit_notes = none;
        kit_notes.set(0, Some(36));
        assert_eq!(
            notes_tracks(&mix.instruments, &kits, &synth, &kit_notes, 8),
            0xFF
        );
    }

    /// Sampler step 15c, part 3: on a track playing a kit played as notes
    /// the pads press the bank's notes into the arp, as the kit plays them
    /// (none past note 127); on any other track they press pads.
    #[test]
    fn a_kit_played_as_notes_gives_the_pads_its_banks_notes() {
        let mut kits = sampler::TrackKits::new(2);
        assert!(kits.set(1, 1));
        let mut kit_notes = sampler::KitNotes::default();
        kit_notes.set(1, Some(36));
        assert!(kit_press_layout(&kits, &kit_notes, 0, 0).is_none(), "kit 1");
        let a = kit_press_layout(&kits, &kit_notes, 1, 0).unwrap();
        assert_eq!((a[0].notes(), a[15].notes()), (&[36][..], &[51][..]));
        let b = kit_press_layout(&kits, &kit_notes, 1, 1).unwrap();
        assert_eq!(b[0].notes(), &[52]);
        let h = kit_press_layout(&kits, &kit_notes, 1, 7).unwrap();
        assert_eq!(h[0].notes(), &[] as &[u8], "past note 127");
        kit_notes.set(1, None);
        assert!(kit_press_layout(&kits, &kit_notes, 1, 0).is_none());
    }

    /// Finding 9 of the step sequencer refactor (20 Nov 2026): two tracks
    /// on the Synth kit play pad 3 at once. Each note lasts until its own
    /// track lets go: the second hit doesn't cut the first, one track's
    /// note-off doesn't end the other's note, and the note ends once both
    /// have let go. Fails until the instrument redesign's step 4 keeps the
    /// synth's notes by source (track and pad); it lands with that fix.
    #[test]
    fn two_tracks_on_the_synth_keep_their_own_notes_on_one_pad() {
        use mpfree_node::NodeEvent::{NoteOff, NoteOn};
        use time::Pads;
        let ev = on_the_synth(&[0, 1], |seq| {
            assert!(seq.hit_at(0, 3, 100, 0));
            assert!(seq.hit_at(1, 3, 90, 10));
            seq.release_at(0, 3, 20);
        });
        let ons = ev.iter().filter(|e| matches!(e, NoteOn { .. })).count();
        assert!(
            ons >= 2 && ons % 2 == 0,
            "both tracks started pad 3: {ev:?}"
        );
        assert!(
            !ev.iter().any(|e| matches!(e, NoteOff { .. })),
            "track 2's note still sounds: {ev:?}"
        );
        let ev = on_the_synth(&[0, 1], |seq| {
            seq.hit_at(0, 3, 100, 0);
            seq.hit_at(1, 3, 90, 10);
            seq.release_at(1, 3, 20);
            seq.release_at(0, 3, 30);
        });
        let offs = ev.iter().filter(|e| matches!(e, NoteOff { .. })).count();
        assert_eq!(
            2 * offs,
            ons,
            "each note ends once, when both let go: {ev:?}"
        );
        assert!(matches!(ev.last(), Some(NoteOff { .. })), "{ev:?}");
    }

    /// Finding 9 with the live pads: a clip on track 2 plays pad 3 while
    /// you play pad 3 on the armed track. Letting go of your pad ends your
    /// note, not the clip's. Lands with the instrument redesign's step 4,
    /// as above.
    #[test]
    fn a_live_pad_on_the_synth_leaves_a_clips_note_on_the_same_pad() {
        use mpfree_node::NodeEvent::{NoteOff, NoteOn};
        use mpfree_play::{Event, Out};
        use time::Pads;
        let live = |offset: u32, event| Out {
            offset,
            frame: u64::from(offset),
            track: 0,
            dest: 0,
            event,
            notes: false,
        };
        let ev = on_the_synth(&[0, 1], |seq| {
            assert!(seq.hit_at(1, 3, 100, 0));
            seq.live(&live(
                10,
                Event::NoteOn {
                    note: 3,
                    velocity: 100,
                },
            ));
            seq.live(&live(20, Event::NoteOff { note: 3 }));
        });
        assert!(
            ev.iter().filter(|e| matches!(e, NoteOn { .. })).count() >= 2,
            "{ev:?}"
        );
        assert!(
            !ev.iter().any(|e| matches!(e, NoteOff { .. })),
            "the clip's note still sounds: {ev:?}"
        );
    }

    /// Instrument redesign step 5a, the gap left after finding 9: a clip
    /// on track 1 plays pad 3 while you play pad 3 through the same
    /// track's arp. The clip's note and yours end by their own source
    /// (`synth::Player::Clip`, `synth::Player::Arp`): the clip letting go
    /// leaves your note, and yours letting go ends it once both have.
    #[test]
    fn a_clip_and_the_live_pads_on_one_track_keep_their_own_notes() {
        use mpfree_node::NodeEvent::{NoteOff, NoteOn};
        use mpfree_play::{Event, Out};
        use time::Pads;
        let live = |offset: u32, event| Out {
            offset,
            frame: u64::from(offset),
            track: 0,
            dest: 0,
            event,
            notes: false,
        };
        let on = Event::NoteOn {
            note: 3,
            velocity: 100,
        };
        let ev = on_the_synth(&[0], |seq| {
            assert!(seq.hit_at(0, 3, 100, 0));
            seq.live(&live(10, on));
            seq.release_at(0, 3, 20);
        });
        let ons = ev.iter().filter(|e| matches!(e, NoteOn { .. })).count();
        assert!(ons >= 2 && ons % 2 == 0, "both started pad 3: {ev:?}");
        assert!(
            !ev.iter().any(|e| matches!(e, NoteOff { .. })),
            "your note still sounds: {ev:?}"
        );
        // The other way round: your let-go leaves the clip's note.
        let ev = on_the_synth(&[0], |seq| {
            seq.live(&live(0, on));
            assert!(seq.hit_at(0, 3, 100, 10));
            seq.live(&live(20, Event::NoteOff { note: 3 }));
        });
        assert!(
            !ev.iter().any(|e| matches!(e, NoteOff { .. })),
            "the clip's note still sounds: {ev:?}"
        );
        // Both let go: each note ends once.
        let ev = on_the_synth(&[0], |seq| {
            seq.live(&live(0, on));
            seq.hit_at(0, 3, 100, 10);
            seq.live(&live(20, Event::NoteOff { note: 3 }));
            seq.release_at(0, 3, 30);
        });
        let offs = ev.iter().filter(|e| matches!(e, NoteOff { .. })).count();
        assert_eq!(2 * offs, ons, "{ev:?}");
        assert!(matches!(ev.last(), Some(NoteOff { .. })), "{ev:?}");
    }

    /// A clip's event of the kind `notes` on track `track` (step 5c).
    fn clip_out(
        track: u16,
        offset: u32,
        event: mpfree_play::Event,
        notes: bool,
    ) -> mpfree_play::Out {
        mpfree_play::Out {
            offset,
            frame: u64::from(offset),
            track,
            dest: 0,
            event,
            notes,
        }
    }

    /// Instrument redesign step 5c: a clip of notes plays its notes by
    /// number on the synth, moved by its track's live transpose as each
    /// starts, and ends the note it started though the transpose moved
    /// in between. The same number from a clip of pad hits is a pad (pad
    /// 61 is no pad of the synth's, so it plays nothing).
    #[test]
    fn a_clip_of_notes_plays_the_synth_by_note() {
        use mpfree_node::NodeEvent::{NoteOff, NoteOn};
        use mpfree_play::Event;
        use time::Pads;
        let on = Event::NoteOn {
            note: 61,
            velocity: 100,
        };
        let off = Event::NoteOff { note: 61 };
        let ev = on_the_synth(&[0], |seq| {
            assert!(seq.out(&clip_out(0, 0, on, true)), "taken: never a pad");
            assert_eq!(seq.hits, 1);
            seq.shifts[0] = mpfree_seq::Transpose::default();
            assert!(seq.out(&clip_out(0, 10, off, true)));
        });
        assert_eq!(
            ev,
            vec![
                NoteOn {
                    note: 61,
                    velocity: 100
                },
                NoteOff { note: 61 }
            ]
        );
        let ev = on_the_synth(&[0], |seq| {
            seq.shifts[0] = mpfree_seq::Transpose::semitones(2);
            seq.out(&clip_out(0, 0, on, true));
            // The transpose goes back while the note sounds.
            seq.shifts[0] = mpfree_seq::Transpose::default();
            seq.out(&clip_out(0, 10, off, true));
        });
        assert_eq!(
            ev,
            vec![
                NoteOn {
                    note: 63,
                    velocity: 100
                },
                NoteOff { note: 63 }
            ]
        );
        // A clip of pad hits: not taken, and pad 61 plays nothing.
        let ev = on_the_synth(&[0], |seq| {
            assert!(!seq.out(&clip_out(0, 0, on, false)));
            assert!(!seq.hit_at(0, 61, 100, 0));
        });
        assert!(ev.is_empty(), "{ev:?}");
    }

    /// Step 5c on a track's own instrument: a clip of notes starts and
    /// ends the note itself, not the note of the pad with its number.
    #[test]
    fn a_clip_of_notes_plays_a_tracks_own_instrument_by_note() {
        use mpfree_play::Event;
        use time::Pads;
        on_the_pads(&[], &[1], |seq| {
            let on = Event::NoteOn {
                note: 50,
                velocity: 100,
            };
            assert!(seq.out(&clip_out(1, 0, on, true)));
            assert!(seq.own.holds_note(1, 50));
            assert_eq!(seq.hits, 1);
            let press = Event::Pressure {
                note: 50,
                value: 70,
            };
            assert!(seq.out(&clip_out(1, 5, press, true)));
            assert!(seq.out(&clip_out(1, 10, Event::NoteOff { note: 50 }, true)));
            assert!(!seq.own.holds_note(1, 50));
            // Pad 3 of a clip of hits plays the pad's own note.
            let pad = Event::NoteOn {
                note: 3,
                velocity: 100,
            };
            assert!(seq.out(&clip_out(1, 20, pad, false)));
            assert!(!seq.own.holds_note(1, 3));
        });
    }

    /// Step 5c: the step grid's pad rows on a clip of notes are the notes
    /// the track's pads play: a track's own synth's pads (a chord's
    /// root), the shared synth's, or a kit's pads' notes, as a clip of
    /// notes plays them back.
    #[test]
    fn the_grids_pad_rows_are_the_notes_the_pads_play() {
        on_the_pads(&[2], &[1], |seq| {
            let none = sampler::KitNotes::default();
            let rows = |seq: &SeqPads, t| pad_rows(seq.own, seq.synth, seq.kits, &none, t);
            let own = rows(seq, 1);
            for p in 0..16 {
                assert_eq!(own.notes(p), seq.own.pad_notes(1, p));
            }
            assert!(own.roots()[0] >= 16, "notes, not pads: {own:?}");
            let shared = rows(seq, 2);
            for p in 0..16 {
                assert_eq!(shared.notes(p), seq.synth.notes(p));
            }
            assert_eq!(
                rows(seq, 0).roots(),
                mpc_one::PAD_NOTES,
                "a kit's pads' notes"
            );
            // A kit played as notes: the notes its pads play in bank A
            // (sampler step 15c, part 3).
            let mut kit_notes = none;
            kit_notes.set(seq.kits.kit_of(0).unwrap(), Some(48));
            let notes = pad_rows(seq.own, seq.synth, seq.kits, &kit_notes, 0);
            for p in 0..16 {
                assert_eq!(notes.notes(p), &[48 + p as u8]);
            }
        });
    }

    /// Step 5d: what an arp that works in notes plays on a track with its
    /// own synth sounds by note on that synth and records as notes, though
    /// the pads show a kit; a track's arp works in notes while its synth
    /// plays notes, not while its notes go to MIDI out or it plays a kit.
    #[test]
    fn an_arp_of_notes_plays_a_tracks_own_synth_by_note() {
        use mpfree_play::Event;
        use time::Pads;
        on_the_pads(&[2], &[1], |seq| {
            assert!(!seq.live.plays_notes(), "the pads show a kit");
            let on = Event::NoteOn {
                note: 52,
                velocity: 100,
            };
            let heard = seq.live(&clip_out(1, 0, on, true));
            assert!(seq.own.holds_note(1, 52));
            assert_eq!(heard.pad, None, "no pad of the kit's");
            assert_eq!(heard.notes().len(), 1);
            seq.live(&clip_out(1, 10, Event::NoteOff { note: 52 }, true));
            assert!(!seq.own.holds_note(1, 52));
            let none = sampler::KitNotes::default();
            assert!(
                arp_on_notes(seq.own, seq.kits, &none, seq.midi, 1),
                "own synth"
            );
            assert!(
                arp_on_notes(seq.own, seq.kits, &none, seq.midi, 2),
                "shared synth"
            );
            assert!(
                !arp_on_notes(seq.own, seq.kits, &none, seq.midi, 0),
                "a kit"
            );
            // A kit played as notes works in notes (sampler step 15c).
            let mut kit_notes = none;
            kit_notes.set(seq.kits.kit_of(0).unwrap(), Some(48));
            assert!(arp_on_notes(seq.own, seq.kits, &kit_notes, seq.midi, 0));
        });
    }

    /// Step 5c on a kit: a clip's note plays the pad MIDI in would play    /// Step 5c on a kit: a clip's note plays the pad MIDI in would play
    /// for it (the kick on pad A5 is note `PAD_NOTES[4]`); a note on an
    /// empty pad plays nothing.
    #[test]
    fn a_clip_of_notes_plays_a_kits_pad_by_its_note() {
        use mpfree_play::Event;
        use time::Pads;
        on_the_pads(&[], &[], |seq| {
            let kick = Event::NoteOn {
                note: mpc_one::PAD_NOTES[4],
                velocity: 100,
            };
            assert!(seq.out(&clip_out(0, 0, kick, true)));
            assert_eq!(seq.hits, 1, "the kick");
            assert_eq!(seq.flashes[4], 100);
            let empty = Event::NoteOn {
                note: mpc_one::PAD_NOTES[5],
                velocity: 100,
            };
            assert!(seq.out(&clip_out(0, 10, empty, true)));
            assert_eq!(seq.hits, 1, "an empty pad");
        });
    }

    /// Sampler step 15: a clip's notes reach a kit as its Keys play sends
    /// them. A kit of keygroups only plays them by pitch, in their range,
    /// and a note outside every keygroup plays nothing (not the drum pad
    /// that sends it).
    #[test]
    fn a_clip_of_notes_plays_a_kits_keygroups_by_pitch() {
        use mpfree_play::Event;
        use mpfree_sampler::model::Keys;
        use time::Pads;
        on_the_pads(&[], &[], |seq| {
            let mut projects = seq.player.take_projects();
            projects[0]
                .edit_pad(4, None, |pad| {
                    pad.keys = Some(Keys {
                        lo: 48,
                        hi: 72,
                        root: 60,
                    })
                })
                .unwrap();
            let mut swap = Box::new(sampler::ViewSwap {
                kit: 0,
                view: Some(mpfree_sampler::engine::EngineView::build(
                    &projects[0],
                    RATE,
                )),
                routes: sampler::pad_routes(&projects[0]),
            });
            assert!(seq.player.swap_view(&mut swap));
            let note = |note| Event::NoteOn {
                note,
                velocity: 100,
            };
            assert!(seq.out(&clip_out(0, 0, note(60), true)));
            assert_eq!((seq.hits, seq.silent), (1, 0), "the keygroup's root");
            assert_eq!(seq.flashes[4], 0, "no pad");
            assert!(seq.out(&clip_out(0, 10, Event::NoteOff { note: 60 }, true)));
            // The kick's own note is outside the keygroup: nothing.
            assert!(seq.out(&clip_out(0, 20, note(mpc_one::PAD_NOTES[4]), true)));
            assert_eq!((seq.hits, seq.silent), (1, 1));
        });
    }

    /// Sampler step 15c, part 3: on a track whose kit is played as notes
    /// (keys on the keygroups, the pads playing notes from C3) a live note
    /// from its arp, the pads or the keys plays the kit, never the synth:
    /// one the pads reach through its pad, which flashes, one they don't
    /// through the keygroups. Each records as written, and lets go where it
    /// went.
    #[test]
    fn a_live_note_plays_a_kit_played_as_notes() {
        use mpfree_play::Event;
        use mpfree_sampler::model::{Keys, KeysPlay, PadsPlay};
        use time::Pads;
        on_the_pads(&[], &[], |seq| {
            let mut projects = seq.player.take_projects();
            projects[0]
                .edit_pad(4, None, |pad| {
                    pad.keys = Some(Keys {
                        lo: 36,
                        hi: 84,
                        root: 60,
                    })
                })
                .unwrap();
            projects[0]
                .set_pads_play(PadsPlay::Keys { from: 60 })
                .unwrap();
            projects[0].set_keys_play(Some(KeysPlay::Keygroups));
            let mut swap = Box::new(sampler::ViewSwap {
                kit: 0,
                view: Some(mpfree_sampler::engine::EngineView::build(
                    &projects[0],
                    RATE,
                )),
                routes: sampler::pad_routes(&projects[0]),
            });
            assert!(seq.player.swap_view(&mut swap));
            assert!(seq.player.kit_keys(0).takes_notes());
            let on = |note| Event::NoteOn {
                note,
                velocity: 100,
            };
            let written = |h: &time::Heard| -> Vec<u8> {
                h.notes()
                    .iter()
                    .filter_map(|e| match *e {
                        Event::NoteOn { note, .. } | Event::NoteOff { note } => Some(note),
                        _ => None,
                    })
                    .collect()
            };
            // Pad 3 plays D3 (62): its note, through the pad.
            let heard = seq.live(&clip_out(0, 0, on(62), true));
            assert_eq!(written(&heard), [62], "records as written");
            assert!(
                matches!(heard.pad, Some(Event::NoteOn { note: 2, .. })),
                "pad 3"
            );
            assert_eq!((seq.hits, seq.silent), (1, 0));
            assert_eq!(seq.n_live, 1, "pad 3 flashes");
            // C2 (48), below the pads: the keygroup, by pitch.
            let heard = seq.live(&clip_out(0, 5, on(48), true));
            assert_eq!(written(&heard), [48]);
            assert_eq!(heard.pad, None, "no pad plays it");
            assert_eq!((seq.hits, seq.silent), (2, 0), "both on the kit");
            // The let-go goes where each went.
            for (n, at) in [(62, 10), (48, 12)] {
                let off = seq.live(&clip_out(0, at, Event::NoteOff { note: n }, true));
                assert_eq!(written(&off), [n]);
            }
        });
    }

    /// Step 5c, recording: a live pad says what a take records of it both
    /// ways. On a track's own instrument the notes are its pad map's as
    /// written; on a kit, the pad's note as MIDI in and out read it.
    #[test]
    fn a_live_pad_records_as_its_pad_or_as_its_notes() {
        use mpfree_play::Event;
        use time::Pads;
        let live = |track: u16, event| clip_out(track, 0, event, false);
        let on = Event::NoteOn {
            note: 3,
            velocity: 100,
        };
        on_the_pads(&[], &[1], |seq| {
            let written = seq.own.pad_notes(1, 3).to_vec();
            assert!(!written.is_empty());
            let h = seq.live(&live(1, on));
            assert!(matches!(h.pad, Some(Event::NoteOn { note: 3, .. })));
            let notes: Vec<u8> = h
                .notes()
                .iter()
                .map(|e| match *e {
                    Event::NoteOn { note, .. } => note,
                    _ => 255,
                })
                .collect();
            assert_eq!(notes, written, "the pad map's notes");
            let h = seq.live(&live(1, Event::NoteOff { note: 3 }));
            assert_eq!(h.pad, Some(Event::NoteOff { note: 3 }));
            assert_eq!(h.notes().len(), written.len());
            // A kit: the pad's note.
            let h = seq.live(&live(0, on));
            assert_eq!(
                h.notes(),
                [Event::NoteOn {
                    note: mpfree_midi::profile::kit_note(3).unwrap(),
                    velocity: seq.live.velocity(100),
                }]
            );
        });
    }

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    /// The I/O view names the project's tracks as the project does (the
    /// Learn section's Clips and scenes card, Routing, CV), "Track N" past
    /// them or for a track without a name.
    #[test]
    fn the_io_view_names_the_projects_tracks() {
        let model = midi_in::io_model();
        let mut shown = model.lock().unwrap().live.tracks.clone();
        assert_eq!(shown.len(), 16);
        let mut tracks = vec![
            mpfree_seq::Track::new("Drums", 0),
            mpfree_seq::Track::new("", 1),
            mpfree_seq::Track::new("Bass", 2),
        ];
        io_track_names(&mut shown, &tracks);
        assert_eq!(shown[..4], ["Drums", "Track 2", "Bass", "Track 4"]);
        assert_eq!(shown[15], "Track 16");
        // Renamed, and one fewer: the rest go back to their numbers.
        tracks[0].name = "Kit".into();
        tracks.pop();
        io_track_names(&mut shown, &tracks);
        assert_eq!(shown[..3], ["Kit", "Track 2", "Track 3"]);
    }

    /// UNDO on the Synth tab and the mixer names the knob or fader.
    #[test]
    fn undo_of_a_turn_names_it() {
        assert_eq!(
            mixturns::said(
                &mixturns::MixKey::Strip(
                    mpfree_fx::StripId::Track(mixing::DRUMS),
                    mpfree_fx::ids::mixer::VOLUME
                ),
                false
            ),
            "Undid Drums Volume"
        );
        assert_eq!(
            mixturns::said(
                &mixturns::MixKey::Insert {
                    strip: mpfree_fx::StripId::Track(mixing::DRUMS),
                    uid: 1,
                    ids: [mpfree_fx::ids::eq::freq(1), mpfree_fx::ids::eq::gain(1)],
                },
                true
            ),
            "Redid Drums EQ Band 2"
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
    /// ERASE + a kit pad while stopped: that pad's notes come out of the
    /// armed track's clip at once, one take UNDO brings back; a pad with
    /// no notes says so. Live erase starts while recording, or while only
    /// playing with the kit on the pads, never while stopped.
    #[test]
    fn erase_and_a_pad_takes_its_notes_out_while_stopped() {
        use mpfree_seq::{Clip, Note};
        let mut time = time::TimeHost::new(RATE, 120.0).unwrap();
        let mut beat = Clip::new("beat", 3840);
        beat.notes = vec![
            Note::new(0, 100, 0, 100),
            Note::new(960, 100, 2, 100),
            Note::new(1920, 100, 0, 90),
        ];
        time.put_clip(beat).unwrap();
        time.act(TimeAction::Launch { track: 0, slot: 0 }).unwrap();
        let told = |e: app::Effect| match e {
            app::Effect::Tell(t) => t,
            e => panic!("{e:?}"),
        };
        let line = told(erase_pad_notes(&mut time, 0));
        assert!(line.contains("pad A1's notes came out"), "{line}");
        let line = told(erase_pad_notes(&mut time, 0));
        assert!(line.contains("no notes on pad A1"), "{line}");
        let undo = time.act(TimeAction::Undo).unwrap();
        assert!(undo.starts_with("undo: edit"), "{undo}");
        assert!(told(erase_pad_notes(&mut time, 0)).contains("came out"));
        // When live erase starts.
        let mut app = app::App::new(RATE, false);
        app.start();
        assert!(!live_erase_starts(&app, &time), "ERASE isn't held");
        app.button(app.map.erase.unwrap(), true, 0.0);
        assert!(!live_erase_starts(&app, &time), "stopped");
    }

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

    /// The audio tracks rethink, phase 1b: an audio track's panel level
    /// and pan (and Q1, Q2) move its own mixer strip, and "Its strip in
    /// Mix ›" opens the Mix tab's tracks with that strip picked.
    #[test]
    fn an_audio_track_s_level_pan_and_strip_button_are_its_mixer_strip_s() {
        use mpfree_fx::ids::mixer as mx;
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
        let mut act =
            |time: &mut time::TimeHost,
             tracks: &mut audiotracks::TrackHost,
             app: &mut app::App,
             mix: &mut mixing::MixHost,
             a| { time_action(&mut out, time, tracks, &looper, app, mix, a) };
        act(
            &mut time,
            &mut tracks,
            &mut app,
            &mut mix,
            TimeAction::AddAudioTrack,
        );
        tracks.sync(time.project(), time.silenced(), true);
        let place = tracks
            .place(0)
            .expect("the audio track has an engine track");
        let strip = mpfree_fx::StripId::Track(mix.track_channel(place));
        let turn = |knob, ticks| {
            TimeAction::Audio(AudioAction::Turn {
                track: 0,
                region: None,
                knob,
                ticks,
                fine: false,
            })
        };
        // Two turns before the next period add up.
        let before = mix.value(strip, mx::VOLUME);
        act(&mut time, &mut tracks, &mut app, &mut mix, turn(0, -4));
        act(&mut time, &mut tracks, &mut app, &mut mix, turn(0, -2));
        assert_eq!(mix.wanted(strip, mx::VOLUME), before - 3.0);
        act(&mut time, &mut tracks, &mut app, &mut mix, turn(1, 5));
        assert_eq!(mix.wanted(strip, mx::PAN), 10.0);
        let fx = act(
            &mut time,
            &mut tracks,
            &mut app,
            &mut mix,
            TimeAction::Audio(AudioAction::Strip(0)),
        );
        assert_eq!(app.view(), app::View::Mixer);
        assert_eq!(
            fx.last(),
            Some(&app::Effect::Mixer(Some(mpfree_mixer_ui::MixView::Tracks)))
        );
        mix.show(Some(mpfree_mixer_ui::MixView::Tracks));
        let st = mix.poll_state(Instant::now()).expect("the mixer shows");
        assert_eq!(st.selected, Some(strip));
    }

    /// Arrange 2.6b: with the editing side on its own thread
    /// (--time-thread), ERASE + a clip and UNDO return no words at once;
    /// the answers come back later and say the same as together.
    #[test]
    fn apart_erase_and_undo_say_what_they_did_when_the_answer_comes() {
        let mut out = Out {
            console: None,
            sink: None,
            dropped: 0,
            report: None,
            queued: None,
            last_draw: Instant::now(),
        };
        let mut time = time::TimeHost::new(RATE, 120.0).unwrap();
        time.put_clip(mpfree_seq::Clip::new("beat", 3840)).unwrap();
        let mut time = time_apart(&mut out, time);
        assert!(time.apart());
        let mut app = app::App::new(RATE, false);
        let mut mix = mixing::MixHost::new(RATE, -6.0, -6.0);
        let mut tracks = audiotracks::TrackHost::new(RATE, 1);
        let looper = looping::LoopHost::new(RATE, 1);
        let mut said = Vec::new();
        for a in [
            TimeAction::DeleteClip { track: 0, slot: 0 },
            TimeAction::Undo,
        ] {
            let fx = time_action(
                &mut out,
                &mut time,
                &mut tracks,
                &looper,
                &mut app,
                &mut mix,
                a,
            );
            assert!(fx.is_empty(), "nothing said yet: {fx:?}");
        }
        let t0 = Instant::now();
        while said.len() < 2 {
            assert!(t0.elapsed() < Duration::from_secs(10), "{said:?}");
            for (say, r) in time.take_said() {
                said.extend(time_said(&mut out, say, r));
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(
            matches!(said.as_slice(), [app::Effect::Tell(d), app::Effect::Tell(u)]
                if d.contains("deleted") && u.starts_with("Undid delete ")
                    && u.ends_with("Track 1 / 1")),
            "{said:?}"
        );
    }

    /// The Song view with audio regions (Arrange 3.4): the pad loop's
    /// engines, a period at a time.
    struct SongRig {
        out: Out,
        time: time::TimeHost,
        app: app::App,
        mix: mixing::MixHost,
        tracks: audiotracks::TrackHost,
        looper: looping::LoopHost,
    }

    impl SongRig {
        const BAR: mpfree_transport::Tick = 3840;

        /// Markers A, B and C at bars 1, 3 and 4 and a bar of audio on
        /// audio 1 in B (a take put back from a save), apart (the editing
        /// side on its own thread) or together.
        fn new(apart: bool) -> SongRig {
            let mut out = Out {
                console: None,
                sink: None,
                dropped: 0,
                report: None,
                queued: None,
                last_draw: Instant::now(),
            };
            let time = time::TimeHost::new(RATE, 120.0).unwrap();
            let time = if apart {
                time_apart(&mut out, time)
            } else {
                time
            };
            assert_eq!(time.apart(), apart);
            let mut r = SongRig {
                out,
                time,
                app: app::App::new(RATE, false),
                mix: mixing::MixHost::new(RATE, -6.0, -6.0),
                tracks: audiotracks::TrackHost::new(RATE, 10),
                looper: looping::LoopHost::new(RATE, 1),
            };
            let frames = 2 * RATE as usize;
            let take = mpfree_audiotracks::save::TakeFile {
                id: 0,
                rec: mpfree_audiotracks::RecordingId::NONE,
                track: 0,
                group: 0,
                pass: 0,
                bpm: 120.0,
                anchor: (2 * Self::BAR) as f64,
                frames,
                channels: 1,
                file: String::new(),
                path: None,
                audio: vec![0.25; frames],
            };
            assert!(r.tracks.host.tracks.load_take(take).is_ok());
            while r.tracks.host.tracks.loading() > 0 {
                r.period();
            }
            let map = *r.time.tempo_map();
            assert!(r.tracks.host.tracks.place(0, &[], &map));
            for bar in [0, 2, 3] {
                let add = mpfree_time_ui::view::MarkerEdit::Add(bar * Self::BAR);
                r.act(TimeAction::SongMarker(add));
            }
            r.settle(|r| r.markers() == "A B C");
            assert_eq!(r.audio(), 2 * Self::BAR);
            r
        }

        /// A period as the pad loop runs one: the transport, the clips,
        /// then the audio tracks into their mixer channels.
        fn period(&mut self) {
            struct Quiet;
            impl time::Pads for Quiet {
                fn hit_at(&mut self, _: u16, _: usize, _: u8, _: u32) -> bool {
                    true
                }
                fn release_at(&mut self, _: u16, _: usize, _: u32) {}
                fn stop_all(&mut self) {}
                fn pressure_at(&mut self, _: u16, _: usize, _: u8, _: u32) {}
            }
            let z = [0f32; PERIOD];
            self.time.advance(PERIOD as u32);
            self.time.period(PERIOD as u32, &mut Quiet);
            let (block, map) = (self.time.block(), self.time.tempo_map());
            let song_on = self.time.song_on();
            let time = &self.time;
            self.tracks
                .period(block, map, song_on, (&z, &z), Some(&|p| time.audio(p)));
            self.tracks.play_into(&mut self.mix);
        }

        /// The action as the screen sends it, and what it said (apart, the
        /// answer comes later: [`SongRig::settle`] collects it).
        fn act(&mut self, a: TimeAction) -> Vec<app::Effect> {
            let r = self;
            time_action(
                &mut r.out,
                &mut r.time,
                &mut r.tracks,
                &r.looper,
                &mut r.app,
                &mut r.mix,
                a,
            )
        }

        /// Periods until `done` holds (apart, the answers come back).
        fn settle(&mut self, done: impl Fn(&SongRig) -> bool) {
            let t0 = Instant::now();
            while !done(self) {
                assert!(t0.elapsed() < Duration::from_secs(10), "{}", self.markers());
                for (say, r) in self.time.take_said() {
                    time_said(&mut self.out, say, r);
                }
                self.period();
                std::thread::sleep(Duration::from_micros(200));
            }
        }

        /// The song's markers, in order.
        fn markers(&self) -> String {
            let m = &self.time.project().arrangement.markers;
            m.iter()
                .map(|m| m.name.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        }

        /// Where audio 1's first region starts.
        fn audio(&self) -> mpfree_transport::Tick {
            self.tracks.host.tracks.regions(0)[0].start
        }

        /// What an action told the screen.
        fn told(fx: &[app::Effect]) -> String {
            match fx {
                [app::Effect::Tell(l)] => l.clone(),
                fx => panic!("{fx:?}"),
            }
        }
    }

    /// Audio tracks phase 2, step 2b: PLAY starts the focused cell's
    /// audio clip (the Play page's pattern) from the first frame, with
    /// the usual fade in, and a clip launched in another cell takes over
    /// on the frame its bar line lands on, as the player says it (song
    /// mode off: the clips play without it). The track host maps the
    /// project track to its engine track.
    #[test]
    fn an_audio_clip_launched_in_its_cell_plays_from_its_line() {
        clip_launch(false);
    }

    /// The same with takes streaming (the time engine's audio waits on):
    /// the launch waits for its audio, the tracks hold it (here it is all
    /// in memory, so at once) and say so, and it lands on its line all the
    /// same, as the app's loop does it (`TrackHost::hold_waits`,
    /// `TrackHost::wait_answers`).
    #[test]
    fn a_clip_launch_that_waits_for_its_audio_lands_on_its_line_once_held() {
        clip_launch(true);
    }

    struct QuietPads;
    impl time::Pads for QuietPads {
        fn hit_at(&mut self, _: u16, _: usize, _: u8, _: u32) -> bool {
            true
        }
        fn release_at(&mut self, _: u16, _: usize, _: u32) {}
        fn stop_all(&mut self) {}
        fn pressure_at(&mut self, _: u16, _: usize, _: u8, _: u32) {}
    }
    /// Audio tracks tests: runs `periods` periods, keeping engine track
    /// `s`'s left channel in `out`, and in `at` the frame song tick `tick`
    /// lands on.
    fn run_audio(
        time: &mut time::TimeHost,
        tracks: &mut audiotracks::TrackHost,
        periods: usize,
        s: usize,
        tick: f64,
        out: &mut Vec<f32>,
        at: &mut Option<usize>,
    ) {
        let z = [0f32; PERIOD];
        for _ in 0..periods {
            time.advance(PERIOD as u32);
            time.period(PERIOD as u32, &mut QuietPads);
            tracks.sync(time.project(), time.silenced(), true);
            let t: &time::TimeHost = time;
            tracks.period(
                t.block(),
                t.tempo_map(),
                t.song_on(),
                (&z, &z),
                Some(&|p| t.audio(p)),
            );
            for sp in t.block().spans() {
                if let (None, Some(o)) = (*at, sp.offset_of(tick)) {
                    *at = Some(out.len() + o as usize);
                }
            }
            let o = tracks.host.tracks.track_out(s, PERIOD);
            out.extend(o.chunks_exact(2).map(|f| f[0]));
        }
    }

    /// Audio tracks phase 2, step 2c: To a cell makes the picked region a
    /// clip, in its track's first empty cell, that plays what the song
    /// played there: exactly, past the entry fade (its first 64 frames,
    /// never louder than the song) and before the song region's own fade
    /// out at its end. The clip is named after the track and bar, and the
    /// song keeps its region.
    #[test]
    fn a_region_put_in_a_cell_plays_what_the_song_played_there() {
        use mpfree_audiotracks::RecordingId;
        use mpfree_seq::{Project, Track};
        use mpfree_time_ui::view::RegionId;
        use mpfree_transport::Command;
        let bar = 2 * RATE as usize;
        let mut p = Project::new();
        p.tracks.push(Track::new_audio("Vocal", 0));
        p.give_track_ids();
        let mut time = time::TimeHost::with_project(RATE, 120.0, p).unwrap();
        let mut tracks = audiotracks::TrackHost::new(RATE, 10);
        tracks.sync(time.project(), 0, true);
        let s = (0..mpfree_audiotracks::MAX_TRACKS)
            .find(|&s| tracks.place(s) == Some(0))
            .expect("an engine track for Vocal");
        // A bar of audio that changes every frame, recorded at bar 3.
        let audio: Vec<f32> = (0..bar)
            .map(|i| ((i % 97) as f32 / 97.0 - 0.5) * 0.8)
            .collect();
        let take = mpfree_audiotracks::save::TakeFile {
            id: 0,
            rec: RecordingId(42),
            track: s,
            group: 0,
            pass: 0,
            bpm: 120.0,
            anchor: (2 * 3840) as f64,
            frames: bar,
            channels: 1,
            file: String::new(),
            path: None,
            audio,
        };
        assert!(tracks.host.tracks.load_take(take).is_ok());
        let (mut out, mut at) = (Vec::new(), None);
        for _ in 0..100 {
            if tracks.host.tracks.loading() == 0 {
                break;
            }
            run_audio(&mut time, &mut tracks, 10, s, 0.0, &mut out, &mut at);
        }
        let map = *time.tempo_map();
        assert!(tracks.host.tracks.place(0, &[], &map));
        // The song, bar 3.
        time.act(TimeAction::SongMode(true)).unwrap();
        time.act(TimeAction::Transport(Command::PlayStart)).unwrap();
        let (mut song, mut line) = (Vec::new(), None);
        let periods = 3 * bar / PERIOD + 4;
        run_audio(
            &mut time,
            &mut tracks,
            periods,
            s,
            7680.0,
            &mut song,
            &mut line,
        );
        let line = line.expect("bar 3 came");
        let song = song[line..line + bar].to_vec();
        assert!(song[200..].iter().any(|&v| v != 0.0), "the song plays it");
        time.act(TimeAction::Transport(Command::Stop)).unwrap();
        time.act(TimeAction::SongMode(false)).unwrap();
        let mut rest = Vec::new();
        run_audio(&mut time, &mut tracks, 4, s, 0.0, &mut rest, &mut None);
        // To a cell.
        let r = RegionId { track: s, index: 0 };
        let (track, clip, place) = tracks
            .region_clip(r, &map, time.info().meter)
            .expect("a clip of it");
        assert_eq!(
            (track, clip.name.as_str(), place.as_str(), clip.length),
            (0, "Vocal 3", "Vocal, bar 3", 3840)
        );
        let a = clip.audio.clone().expect("an audio clip");
        assert_eq!(
            (a.bpm, a.last_frame, a.regions.len()),
            (120.0, bar as i64, 1)
        );
        assert_eq!(time.put_audio_clip(track, clip), Ok(0));
        assert_eq!(tracks.host.tracks.regions(s).len(), 1, "the song keeps it");
        // PLAY launches it, the focused cell, from the first frame.
        let said = time.act(TimeAction::Transport(Command::PlayStart)).unwrap();
        assert!(said.contains("slot 1 of track 1 launched"), "{said}");
        let (mut cell, mut start) = (Vec::new(), None);
        run_audio(
            &mut time,
            &mut tracks,
            bar / PERIOD + 4,
            s,
            0.0,
            &mut cell,
            &mut start,
        );
        let start = start.expect("its line came");
        let cell = &cell[start..start + bar];
        for i in 0..64 {
            assert!(
                cell[i].abs() <= song[i].abs() + 1e-6,
                "{i}: {} {}",
                cell[i],
                song[i]
            );
        }
        let first = (64..bar - 64).find(|&i| cell[i] != song[i]);
        assert_eq!(first, None, "the clip plays what the song played");
    }

    /// Audio tracks phase 2, step 2c: Put in a cell from the Sample page.
    /// The sound goes in the armed audio track's first empty cell, else the
    /// first audio track's with one; as a take the song doesn't play, and
    /// a clip that loops at its tempo (to the nearest beat), or with no
    /// tempo plays once, up to the next beat at the project's. UNDO takes
    /// it out.
    #[test]
    fn a_sound_put_in_a_cell_becomes_a_clip_the_song_doesnt_play() {
        use mpfree_seq::{AudioClip, Clip, Project, RecId, Track};
        let mut p = Project::new();
        p.tracks.push(Track::new("Keys", 0));
        let mut vocal = Track::new_audio("Vocal", 0);
        for (slot, name) in [(0, "Verse vox"), (1, "Hook vox")] {
            let mut c = Clip::new(name, 3840);
            c.audio = Some(AudioClip::of(RecId(7), 0, 88_200, 3840, 120.0));
            vocal.slots[slot] = Some(c);
        }
        p.tracks.push(vocal);
        p.tracks.push(Track::new_audio("Guitar", 0));
        p.give_track_ids();
        let mut time = time::TimeHost::with_project(RATE, 120.0, p).unwrap();
        let mut tracks = audiotracks::TrackHost::new(RATE, 30);
        tracks.sync(time.project(), 0, true);
        let s = (0..mpfree_audiotracks::MAX_TRACKS)
            .find(|&s| tracks.place(s) == Some(1))
            .expect("an engine track for Vocal");
        // Keys is armed, and plays notes: Vocal's first empty cell, row 3.
        time.act(TimeAction::Arm(Some(0))).unwrap();
        assert_eq!(time.cell_for_sound(), Some((1, 2)));
        // Two bars at 92 BPM.
        let frames = (f64::from(RATE) * 8.0 * 60.0 / 92.0).round() as usize;
        let sound = |name: &str, frames: usize, bpm: f64| audiotracks::Sound {
            name: name.into(),
            channels: 1,
            audio: vec![0.25; frames],
            cuts: vec![1000],
            bpm,
            cell: true,
        };
        let arrive = |time: &mut time::TimeHost, tracks: &mut audiotracks::TrackHost| {
            let t0 = std::time::Instant::now();
            loop {
                run_audio(time, tracks, 1, s, 0.0, &mut Vec::new(), &mut None);
                if let Some(d) = tracks.tend_cell_sound(time.info().bpm) {
                    break d;
                }
                assert!(t0.elapsed().as_secs() < 30, "the sound never came in");
            }
        };
        let said = tracks.bring_to_cell(sound("Break 92", frames, 92.0), 1, time.project());
        assert_eq!(said, Ok("Break 92 going into Vocal".to_string()));
        let done = arrive(&mut time, &mut tracks);
        let (track, clip) = done.clone().expect("its clip");
        assert_eq!((track, clip.length, clip.once), (1, 8 * 960, false));
        let a = clip.audio.clone().unwrap();
        assert_eq!(
            (a.bpm, a.regions.len(), a.last_frame),
            (92.0, 1, frames as i64)
        );
        assert_eq!(
            cell_sound_in(&mut time, &mut tracks, done),
            "Break 92 into Vocal, row 3 · UNDO takes it out"
        );
        assert_eq!(
            time.project().clip(1, 2).map(|c| c.name.as_str()),
            Some("Break 92")
        );
        assert!(tracks.host.tracks.regions(s).is_empty(), "not on the song");
        let rec = mpfree_audiotracks::RecordingId(a.regions[0].rec.0);
        assert!(tracks.host.tracks.take_of(rec).is_some(), "its take is in");
        // No tempo: it plays once, 5.2 s up to the next beat at 120.
        let secs = (f64::from(RATE) * 5.2) as usize;
        tracks
            .bring_to_cell(sound("Riser", secs, 0.0), 1, time.project())
            .unwrap();
        let done = arrive(&mut time, &mut tracks);
        let (_, clip) = done.clone().expect("its clip");
        assert_eq!((clip.length, clip.once), (11 * 960, true));
        assert_eq!(clip.audio.as_ref().map(|a| a.bpm), Some(120.0));
        assert_eq!(
            cell_sound_in(&mut time, &mut tracks, done),
            "Riser into Vocal, row 4 · UNDO takes it out"
        );
        // UNDO takes it out.
        time.act(TimeAction::Undo).unwrap();
        assert!(time.project().clip(1, 3).is_none());
        assert!(time.project().clip(1, 2).is_some());
    }

    /// Audio tracks phase 2, step 2c: Fit to tempo on a clip made at 120
    /// BPM, in a project at 90, plays it on a fitted copy of its recording
    /// that fills its length (at 90 the original leaves a gap each pass),
    /// as one UNDO step; the song keeps playing the original.
    #[test]
    fn a_clip_fitted_to_tempo_fills_its_length_and_undo_goes_back() {
        use mpfree_audiotracks::RecordingId;
        use mpfree_seq::{AudioClip, Clip, Project, RecId, Track};
        use mpfree_transport::Command;
        // One bar at 120 (2 s); at 90 a bar is 8/3 s.
        let made = 2 * RATE as usize;
        let mut p = Project::new();
        p.tracks.push(Track::new_audio("Vocal", 0));
        p.give_track_ids();
        let mut time = time::TimeHost::with_project(RATE, 90.0, p).unwrap();
        let mut tracks = audiotracks::TrackHost::new(RATE, 20);
        tracks.sync(time.project(), 0, true);
        let s = (0..mpfree_audiotracks::MAX_TRACKS)
            .find(|&s| tracks.place(s) == Some(0))
            .expect("an engine track for Vocal");
        let take = mpfree_audiotracks::save::TakeFile {
            id: 0,
            rec: RecordingId(42),
            track: s,
            group: 0,
            pass: 0,
            bpm: 120.0,
            anchor: 0.0,
            frames: made,
            channels: 1,
            file: String::new(),
            path: None,
            audio: vec![0.5; made],
        };
        assert!(tracks.host.tracks.load_take(take).is_ok());
        let mut rest = Vec::new();
        while tracks.host.tracks.loading() != 0 {
            run_audio(&mut time, &mut tracks, 10, s, 0.0, &mut rest, &mut None);
        }
        let map = *time.tempo_map();
        assert!(tracks.host.tracks.place(0, &[], &map));
        let song_take = tracks.host.tracks.regions(s)[0].take;
        let mut clip = Clip::new("Riff", 3840);
        clip.audio = Some(AudioClip::of(RecId(42), 0, made as i64, 3840, 120.0));
        assert_eq!(time.put_audio_clip(0, clip.clone()), Ok(0));
        // Fit to tempo: once the copy is in, the clip plays it.
        let c = time.project().clip(0, 0).cloned().expect("the clip");
        assert_eq!(
            tracks.fit_clip(0, 0, &c, time.info().bpm),
            Ok("Riff: fitting to 90 BPM…".to_string())
        );
        assert!(tracks.fit_clip(0, 0, &c, 90.0).is_err(), "one at a time");
        let t0 = std::time::Instant::now();
        let done = loop {
            run_audio(&mut time, &mut tracks, 1, s, 0.0, &mut rest, &mut None);
            if let Some(d) = tracks.tend_clip_fit() {
                break d;
            }
            assert!(t0.elapsed().as_secs() < 30, "the fit never came");
            assert!(tracks.clip_fit_progress().is_some());
            std::thread::sleep(std::time::Duration::from_millis(1));
        };
        assert!(tracks.clip_fit_progress().is_none());
        assert_eq!(
            fit_ended(&mut time, &mut tracks, done),
            "Riff: fitted to 90 BPM · UNDO goes back"
        );
        let a = time
            .project()
            .clip(0, 0)
            .and_then(|c| c.audio.clone())
            .unwrap();
        let fitted = made as i64 * 4 / 3;
        assert_eq!(a.bpm, 90.0);
        assert_ne!(a.regions[0].rec, RecId(42), "on the copy");
        assert!((a.last_frame - fitted).abs() <= 1, "{}", a.last_frame);
        let copy = tracks
            .host
            .tracks
            .take_of(RecordingId(a.regions[0].rec.0))
            .and_then(|id| tracks.host.tracks.take(id))
            .expect("the copy is in");
        assert!((copy.frames as i64 - fitted).abs() <= 2, "{}", copy.frames);
        assert_eq!(
            tracks.host.tracks.regions(s)[0].take,
            song_take,
            "the song keeps it"
        );
        // Launched at 90, it sounds to the end of its bar.
        time.act(TimeAction::Transport(Command::PlayStart)).unwrap();
        let bar = made * 4 / 3;
        let (mut cell, mut start) = (Vec::new(), None);
        run_audio(
            &mut time,
            &mut tracks,
            bar / PERIOD + 4,
            s,
            0.0,
            &mut cell,
            &mut start,
        );
        let start = start.expect("its line came");
        let late = &cell[start + made + 100..start + bar - 200];
        assert!(late.iter().all(|&v| v.abs() > 0.1), "no gap past 2 s");
        time.act(TimeAction::Transport(Command::Stop)).unwrap();
        // UNDO goes back to the clip made at 120.
        time.act(TimeAction::Undo).unwrap();
        let back = time
            .project()
            .clip(0, 0)
            .and_then(|c| c.audio.clone())
            .unwrap();
        assert_eq!((back.bpm, back.regions[0].rec), (120.0, RecId(42)));
    }

    /// [`an_audio_clip_launched_in_its_cell_plays_from_its_line`], with the
    /// cues waiting for their audio if `waits`.
    fn clip_launch(waits: bool) {
        use mpfree_seq::{AudioClip, Clip, Project, RecId, Track};
        struct Quiet;
        impl time::Pads for Quiet {
            fn hit_at(&mut self, _: u16, _: usize, _: u8, _: u32) -> bool {
                true
            }
            fn release_at(&mut self, _: u16, _: usize, _: u32) {}
            fn stop_all(&mut self) {}
            fn pressure_at(&mut self, _: u16, _: usize, _: u8, _: u32) {}
        }
        let bar_frames = 2 * RATE as usize;
        let mut p = Project::new();
        let mut t = Track::new_audio("Loop", 0);
        for (slot, rec) in [(0, 7), (1, 8)] {
            let mut clip = Clip::new("Loop", 3840);
            clip.audio = Some(AudioClip::of(RecId(rec), 0, bar_frames as i64, 3840, 120.0));
            t.slots[slot] = Some(clip);
        }
        p.tracks.push(t);
        p.give_track_ids();
        let at = p.tracks.len() - 1;
        let mut time = time::TimeHost::with_project(RATE, 120.0, p).unwrap();
        assert!(!time.song_on(), "clip mode");
        let mut tracks = audiotracks::TrackHost::new(RATE, 10);
        for (id, rec, level) in [(0, 7, 0.25), (1, 8, 0.5)] {
            let take = mpfree_audiotracks::save::TakeFile {
                id,
                rec: mpfree_audiotracks::RecordingId(rec),
                track: 0,
                group: 0,
                pass: id,
                bpm: 120.0,
                anchor: 0.0,
                frames: bar_frames,
                channels: 1,
                file: String::new(),
                path: None,
                audio: vec![level; bar_frames],
            };
            assert!(tracks.host.tracks.load_take(take).is_ok());
        }
        /// Runs `periods` periods, keeping engine track `s`'s left
        /// channel in `out` and the frame bar 2's line lands on in `line`;
        /// what waiting cues need goes to the tracks when it changes, and
        /// what they say back to the time engine.
        fn run(
            time: &mut time::TimeHost,
            tracks: &mut audiotracks::TrackHost,
            periods: usize,
            s: usize,
            out: &mut Vec<f32>,
            line: &mut Option<usize>,
        ) {
            let mut needs_rev = 0;
            let z = [0f32; PERIOD];
            for _ in 0..periods {
                time.advance(PERIOD as u32);
                time.period(PERIOD as u32, &mut Quiet);
                tracks.sync(time.project(), time.silenced(), true);
                let t: &time::TimeHost = time;
                tracks.period(
                    t.block(),
                    t.tempo_map(),
                    t.song_on(),
                    (&z, &z),
                    Some(&|p| t.audio(p)),
                );
                for sp in t.block().spans() {
                    if let Some(o) = sp.offset_of(3840.0) {
                        *line = Some(out.len() + o as usize);
                    }
                }
                let o = tracks.host.tracks.track_out(s, PERIOD);
                out.extend(o.chunks_exact(2).map(|f| f[0]));
                let rev = time.state().audio_waits.needs_rev;
                if rev != needs_rev {
                    needs_rev = rev;
                    tracks.hold_waits(&time.audio_needs());
                }
                tracks.wait_answers(|key, a| {
                    if a == CueAudio::Held {
                        assert!(time.audio_held(key, true));
                    }
                });
            }
        }
        let (mut out, mut line) = (Vec::new(), None);
        for _ in 0..100 {
            if tracks.host.tracks.loading() == 0 {
                break;
            }
            run(&mut time, &mut tracks, 10, 0, &mut out, &mut line);
        }
        assert_eq!(tracks.host.tracks.loading(), 0);
        assert!(out.iter().all(|&v| v == 0.0), "nothing plays while stopped");
        // The takes sit on engine tracks 0 and 1, so the track gets
        // another.
        let s = (0..mpfree_audiotracks::MAX_TRACKS)
            .find(|&s| tracks.place(s) == Some(at))
            .expect("an engine track for it");
        out.clear();
        line = None;
        time.set_audio_waits(waits);
        let said = time
            .act(TimeAction::Transport(mpfree_transport::Command::Play))
            .unwrap();
        assert!(said.contains("slot 1 of track 1 launched"), "{said}");
        // Half a bar of the first clip, then the second one's launch
        // waits for bar 2's line.
        run(
            &mut time,
            &mut tracks,
            bar_frames / 2 / PERIOD,
            s,
            &mut out,
            &mut line,
        );
        assert!(out[0] < 0.25 && out[0] >= 0.0, "fading in: {}", out[0]);
        assert!((out[200] - 0.25).abs() < 1e-6, "{}", out[200]);
        time.act(TimeAction::Launch { track: at, slot: 1 }).unwrap();
        run(
            &mut time,
            &mut tracks,
            bar_frames / PERIOD + 20,
            s,
            &mut out,
            &mut line,
        );
        let line = line.expect("bar 2's line came");
        assert!(
            out[200..line].iter().all(|&v| (v - 0.25).abs() < 1e-6),
            "the first clip up to its line {line}: first change at {:?}",
            (200..line).find(|&i| (out[i] - 0.25).abs() >= 1e-6)
        );
        assert!(out[line + 1] > 0.25, "fading over from its line");
        assert!((out[line + 200] - 0.5).abs() < 1e-6, "{}", out[line + 200]);
        let w = time.state().audio_waits;
        assert_eq!(
            (w.waiting, w.kept, w.held, w.moved),
            if waits { (0, 1, 1, 0) } else { (0, 0, 0, 0) },
            "{w:?}"
        );
    }

    /// Phase 2, step 2d: with Rec song on, an audio clip launched on the
    /// grid goes into the song as a block on its track's lane, and song
    /// mode plays it back from the block sample for sample, through the
    /// time engine and the track host.
    #[test]
    fn an_audio_launch_recorded_with_rec_song_plays_the_same_from_its_block() {
        use mpfree_seq::{AudioClip, Clip, Project, RecId, Track};
        struct Quiet;
        impl time::Pads for Quiet {
            fn hit_at(&mut self, _: u16, _: usize, _: u8, _: u32) -> bool {
                true
            }
            fn release_at(&mut self, _: u16, _: usize, _: u32) {}
            fn stop_all(&mut self) {}
            fn pressure_at(&mut self, _: u16, _: usize, _: u8, _: u32) {}
        }
        let bar_frames = 2 * RATE as usize;
        let mut p = Project::new();
        let mut t = Track::new_audio("Loop", 0);
        let mut clip = Clip::new("Loop", 3840);
        clip.audio = Some(AudioClip::of(RecId(7), 0, bar_frames as i64, 3840, 120.0));
        t.slots[0] = Some(clip);
        p.tracks.push(t);
        p.give_track_ids();
        p.give_clip_ids();
        let at = p.tracks.len() - 1;
        let mut time = time::TimeHost::with_project(RATE, 120.0, p).unwrap();
        let mut tracks = audiotracks::TrackHost::new(RATE, 10);
        // Audio that changes every frame, so a frame out shows.
        let audio: Vec<f32> = (0..bar_frames).map(|i| (i % 997) as f32 / 2000.0).collect();
        let take = mpfree_audiotracks::save::TakeFile {
            id: 0,
            rec: mpfree_audiotracks::RecordingId(7),
            track: 0,
            group: 0,
            pass: 0,
            bpm: 120.0,
            anchor: 0.0,
            frames: bar_frames,
            channels: 1,
            file: String::new(),
            path: None,
            audio,
        };
        assert!(tracks.host.tracks.load_take(take).is_ok());
        fn run(
            time: &mut time::TimeHost,
            tracks: &mut audiotracks::TrackHost,
            periods: usize,
            s: Option<usize>,
            out: &mut Vec<f32>,
        ) {
            let z = [0f32; PERIOD];
            for _ in 0..periods {
                time.advance(PERIOD as u32);
                time.period(PERIOD as u32, &mut Quiet);
                tracks.sync(time.project(), time.silenced(), true);
                let t: &time::TimeHost = time;
                tracks.period(
                    t.block(),
                    t.tempo_map(),
                    t.song_on(),
                    (&z, &z),
                    Some(&|p| t.audio(p)),
                );
                if let Some(s) = s {
                    let o = tracks.host.tracks.track_out(s, PERIOD);
                    out.extend(o.chunks_exact(2).map(|f| f[0]));
                }
            }
        }
        let mut out = Vec::new();
        for _ in 0..100 {
            if tracks.host.tracks.loading() == 0 {
                break;
            }
            run(&mut time, &mut tracks, 10, None, &mut out);
        }
        assert_eq!(tracks.host.tracks.loading(), 0);
        let s = (0..mpfree_audiotracks::MAX_TRACKS)
            .find(|&s| tracks.place(s) == Some(at))
            .expect("an engine track for it");
        // Rec song: the clip launched, PLAY, two and a half bars, STOP.
        time.act(TimeAction::Launch { track: at, slot: 0 }).unwrap();
        time.act(TimeAction::RecSong(true)).unwrap();
        time.act(TimeAction::Transport(mpfree_transport::Command::Play))
            .unwrap();
        let periods = 5 * bar_frames / 2 / PERIOD;
        let mut heard = Vec::new();
        run(&mut time, &mut tracks, periods, Some(s), &mut heard);
        time.act(TimeAction::Transport(mpfree_transport::Command::Stop))
            .unwrap();
        run(&mut time, &mut tracks, 4, None, &mut out);
        let note = time.take_later_note().unwrap_or_default();
        assert!(note.starts_with("rec song: 1 placements"), "{note}");
        let song = &time.project().arrangement;
        assert_eq!(song.placements.len(), 1);
        let block = &song.placements[0];
        assert_eq!((block.track, block.start), (at, 0));
        assert!(block.length >= 2 * 3840, "{block:?}");
        // Song mode, nothing launched, from the top: the block plays it.
        time.act(TimeAction::RecSong(false)).unwrap();
        time.act(TimeAction::StopAll).unwrap();
        time.act(TimeAction::SongMode(true)).unwrap();
        time.command(mpfree_transport::Command::Locate(0));
        run(&mut time, &mut tracks, 4, None, &mut out);
        time.act(TimeAction::Transport(mpfree_transport::Command::Play))
            .unwrap();
        let mut again = Vec::new();
        run(&mut time, &mut tracks, periods, Some(s), &mut again);
        assert!(heard[2000..].iter().any(|&v| v != 0.0), "the clip played");
        let n = heard.len().min(again.len());
        let first = (0..n).find(|&i| (heard[i] - again[i]).abs() > 1e-6);
        assert_eq!(first, None, "the block plays what the launch played");
    }

    /// Phase 2, step 2d: the song's audio, round the trip. Song → scenes
    /// turns an audio lane's regions into a clip in each section's row,
    /// and Scenes → song puts the rows back as blocks after the
    /// recordings' end; those blocks play what the regions played, sample
    /// for sample: plain regions, a comped one, silence, fades across
    /// section edges and clips starting partway into a recording. At 120
    /// BPM bar lines fall on whole frames; at 117 they don't.
    #[test]
    fn song_to_scenes_and_back_plays_the_songs_audio_sample_for_sample() {
        song_to_scenes_and_back_at(120.0);
        song_to_scenes_and_back_at(117.0);
    }

    fn song_to_scenes_and_back_at(bpm: f64) {
        use mpfree_seq::{Project, Track};
        use mpfree_transport::Tick;
        struct Quiet;
        impl time::Pads for Quiet {
            fn hit_at(&mut self, _: u16, _: usize, _: u8, _: u32) -> bool {
                true
            }
            fn release_at(&mut self, _: u16, _: usize, _: u32) {}
            fn stop_all(&mut self) {}
            fn pressure_at(&mut self, _: u16, _: usize, _: u8, _: u32) {}
        }
        let bar: Tick = 3840;
        let bar_frames = (4.0 * 60.0 / bpm * RATE as f64).ceil() as usize;
        let mut p = Project::new();
        p.tracks.push(Track::new_audio("Vocal", 0));
        p.give_track_ids();
        let at = p.tracks.len() - 1;
        let mut time = time::TimeHost::with_project(RATE, bpm, p).unwrap();
        let mut tracks = audiotracks::TrackHost::new(RATE, 30);
        let mut out = Out {
            console: None,
            sink: None,
            dropped: 0,
            report: None,
            queued: None,
            last_draw: Instant::now(),
        };
        fn run(
            time: &mut time::TimeHost,
            tracks: &mut audiotracks::TrackHost,
            out: &mut Out,
            periods: usize,
            heard: Option<(usize, &mut Vec<f32>)>,
        ) {
            let z = [0f32; PERIOD];
            let mut heard = heard;
            for _ in 0..periods {
                time.advance(PERIOD as u32);
                time.period(PERIOD as u32, &mut Quiet);
                tracks.sync(time.project(), time.silenced(), true);
                let t: &time::TimeHost = time;
                tracks.period(
                    t.block(),
                    t.tempo_map(),
                    t.song_on(),
                    (&z, &z),
                    Some(&|p| t.audio(p)),
                );
                if let Some((s, h)) = heard.as_mut() {
                    let o = tracks.host.tracks.track_out(*s, PERIOD);
                    h.extend(o.chunks_exact(2).map(|f| f[0]));
                }
                if let Some(spans) = tracks.spans_if_changed(time.project()) {
                    time.set_audio_spans(spans);
                }
                cut_blocks_under_takes(out, time, tracks);
            }
        }
        run(&mut time, &mut tracks, &mut out, 1, None);
        let s = (0..mpfree_audiotracks::MAX_TRACKS)
            .find(|&s| tracks.place(s) == Some(at))
            .expect("an engine track for it");
        // Two takes of six bars, each frame different, so a frame out shows.
        for (id, rec, k) in [(0u16, 7u128, 997usize), (1, 8, 613)] {
            // Six bars, silent from the frame after the last region's
            // last (its end, rounded: a region plays the frames before
            // it): its fade past the end, under the first block, reads
            // nothing.
            let frames = 6 * bar_frames;
            let end = (6.0 * 4.0 * 60.0 / bpm * RATE as f64).round() as usize;
            let take = mpfree_audiotracks::save::TakeFile {
                id,
                rec: mpfree_audiotracks::RecordingId(rec),
                track: s,
                group: id,
                pass: 0,
                bpm,
                anchor: 0.0,
                frames,
                channels: 1,
                file: String::new(),
                path: None,
                audio: (0..frames)
                    .map(|i| {
                        if i >= end {
                            0.0
                        } else {
                            (i % k) as f32 / (2.0 * k as f32)
                        }
                    })
                    .collect(),
            };
            assert!(tracks.host.tracks.load_take(take).is_ok());
        }
        for _ in 0..200 {
            if tracks.host.tracks.loading() == 0 {
                break;
            }
            run(&mut time, &mut tracks, &mut out, 10, None);
        }
        assert_eq!(tracks.host.tracks.loading(), 0);
        let map = *time.tempo_map();
        use mpfree_audiotracks::Command as A;
        assert!(tracks.host.tracks.place(0, &[], &map));
        // Bar 3 comped from the other take; silence over the second half
        // of bar 5; fades on the first region, the fade out running
        // across bar 2's edge (from 1.5 s to its end at 4 s).
        for c in [
            A::Comp {
                take: 1,
                from: 2 * bar,
                to: 3 * bar,
            },
            A::Erase {
                track: s,
                from: 4 * bar + bar / 2,
                to: 5 * bar,
            },
            A::SetFades {
                track: s,
                at: bar / 2,
                fade_in: RATE / 2,
                fade_out: 5 * RATE / 2,
            },
        ] {
            assert!(tracks.host.command(c), "{c:?}");
            run(&mut time, &mut tracks, &mut out, 2, None);
        }
        let regions: Vec<(Tick, Tick)> = tracks
            .host
            .tracks
            .regions(s)
            .iter()
            .map(|r| (r.start, r.end))
            .collect();
        assert_eq!(
            regions,
            [
                (0, 2 * bar),
                (2 * bar, 3 * bar),
                (3 * bar, 4 * bar + bar / 2),
                (5 * bar, 6 * bar)
            ]
        );
        // Sections at bars 1, 2, 4 and 6: bar 2's edge cuts the faded
        // region, bar 4's starts a clip partway into the first take.
        for b in [0, 1, 3, 5] {
            let add = mpfree_time_ui::view::MarkerEdit::Add(b * bar);
            time.act(TimeAction::SongMarker(add)).unwrap();
        }
        // The song, from the top in song mode.
        time.act(TimeAction::SongMode(true)).unwrap();
        time.command(mpfree_transport::Command::Locate(0));
        run(&mut time, &mut tracks, &mut out, 4, None);
        time.act(TimeAction::Transport(mpfree_transport::Command::Play))
            .unwrap();
        let periods = 6 * bar_frames / PERIOD;
        let mut song = Vec::new();
        run(
            &mut time,
            &mut tracks,
            &mut out,
            periods,
            Some((s, &mut song)),
        );
        time.act(TimeAction::Transport(mpfree_transport::Command::Stop))
            .unwrap();
        run(&mut time, &mut tracks, &mut out, 4, None);
        assert!(song.iter().any(|&v| v != 0.0), "the song played");
        // Song → scenes: a row a section, each with its audio clip.
        let said = time
            .act_with_audio(TimeAction::SongToScenes, &mut tracks)
            .unwrap();
        assert!(said.contains("exact"), "{said}");
        let clips = (0..4)
            .filter(|&r| {
                time.project()
                    .clip(at, r)
                    .is_some_and(|c| c.audio.is_some())
            })
            .count();
        assert_eq!(clips, 4, "{said}");
        // Scenes → song: the rows as blocks after the recordings' end.
        let said = time.act(TimeAction::ScenesToSong).unwrap();
        assert!(said.contains("from bar 7"), "{said}");
        let blocks: Vec<(Tick, Tick)> = time
            .project()
            .arrangement
            .placements
            .iter()
            .map(|p| (p.start, p.end()))
            .collect();
        assert_eq!(
            blocks,
            [
                (6 * bar, 7 * bar),
                (7 * bar, 9 * bar),
                (9 * bar, 11 * bar),
                (11 * bar, 12 * bar)
            ]
        );
        // The blocks, from bar 7: the song's audio again.
        time.command(mpfree_transport::Command::Locate(6 * bar));
        run(&mut time, &mut tracks, &mut out, 4, None);
        time.act(TimeAction::Transport(mpfree_transport::Command::Play))
            .unwrap();
        let mut again = Vec::new();
        run(
            &mut time,
            &mut tracks,
            &mut out,
            periods,
            Some((s, &mut again)),
        );
        let n = song.len().min(again.len());
        // At 117 BPM a section's edge falls between frames; the parts cut
        // there keep their place between frames (their phase), so they
        // play as the region did, to float rounding, as at 120 where the
        // edges fall on frames.
        let tol = 1e-6;
        let first = (0..n).find(|&i| (song[i] - again[i]).abs() > tol);
        assert_eq!(
            first, None,
            "the blocks play what the regions played at {bpm} BPM ({n} frames)"
        );
    }

    /// Phase 2, step 2d (Joshi's pick, 23 Nov): what Rec song records
    /// plays back as it was heard, sample for sample, edges and all. The
    /// track's regions play, its clip is launched over them for a bar and
    /// stopped, and its regions play on; the block that replaced the
    /// regions under it, and the regions either side of it, then play the
    /// same three bars again.
    #[test]
    fn rec_song_plays_back_what_was_heard_at_the_blocks_edges() {
        use mpfree_seq::{AudioClip, Clip, Project, RecId, Track};
        use mpfree_transport::Tick;
        struct Quiet;
        impl time::Pads for Quiet {
            fn hit_at(&mut self, _: u16, _: usize, _: u8, _: u32) -> bool {
                true
            }
            fn release_at(&mut self, _: u16, _: usize, _: u32) {}
            fn stop_all(&mut self) {}
            fn pressure_at(&mut self, _: u16, _: usize, _: u8, _: u32) {}
        }
        let bar: Tick = 3840;
        let bar_frames = 2 * RATE as usize;
        let mut p = Project::new();
        let mut t = Track::new_audio("Loop", 0);
        let mut clip = Clip::new("Loop", bar);
        clip.audio = Some(AudioClip::of(RecId(7), 0, bar_frames as i64, bar, 120.0));
        t.slots[0] = Some(clip);
        p.tracks.push(t);
        p.give_track_ids();
        p.give_clip_ids();
        let at = p.tracks.len() - 1;
        let mut time = time::TimeHost::with_project(RATE, 120.0, p).unwrap();
        let mut tracks = audiotracks::TrackHost::new(RATE, 10);
        let mut out = Out {
            console: None,
            sink: None,
            dropped: 0,
            report: None,
            queued: None,
            last_draw: Instant::now(),
        };
        fn run(
            time: &mut time::TimeHost,
            tracks: &mut audiotracks::TrackHost,
            out: &mut Out,
            periods: usize,
            heard: Option<(usize, &mut Vec<f32>)>,
        ) {
            let z = [0f32; PERIOD];
            let mut heard = heard;
            for _ in 0..periods {
                time.advance(PERIOD as u32);
                time.period(PERIOD as u32, &mut Quiet);
                tracks.sync(time.project(), time.silenced(), true);
                let t: &time::TimeHost = time;
                tracks.period(
                    t.block(),
                    t.tempo_map(),
                    t.song_on(),
                    (&z, &z),
                    Some(&|p| t.audio(p)),
                );
                if let Some((s, h)) = heard.as_mut() {
                    let o = tracks.host.tracks.track_out(*s, PERIOD);
                    h.extend(o.chunks_exact(2).map(|f| f[0]));
                }
                if let Some(spans) = tracks.spans_if_changed(time.project()) {
                    time.set_audio_spans(spans);
                }
                cut_blocks_under_takes(out, time, tracks);
            }
        }
        run(&mut time, &mut tracks, &mut out, 1, None);
        let s = (0..mpfree_audiotracks::MAX_TRACKS)
            .find(|&s| tracks.place(s) == Some(at))
            .expect("an engine track for it");
        // The clip's recording, and four bars the track recorded from the
        // top: each frame different, so a frame out shows.
        for (id, rec, k, frames) in [
            (0u16, 7u128, 997usize, bar_frames),
            (1, 8, 613, 4 * bar_frames),
        ] {
            let take = mpfree_audiotracks::save::TakeFile {
                id,
                rec: mpfree_audiotracks::RecordingId(rec),
                track: s,
                group: id,
                pass: 0,
                bpm: 120.0,
                anchor: 0.0,
                frames,
                channels: 1,
                file: String::new(),
                path: None,
                audio: (0..frames)
                    .map(|i| (i % k) as f32 / (2.0 * k as f32))
                    .collect(),
            };
            assert!(tracks.host.tracks.load_take(take).is_ok());
        }
        for _ in 0..200 {
            if tracks.host.tracks.loading() == 0 {
                break;
            }
            run(&mut time, &mut tracks, &mut out, 10, None);
        }
        assert_eq!(tracks.host.tracks.loading(), 0);
        let map = *time.tempo_map();
        assert!(tracks.host.tracks.place(1, &[], &map));
        run(&mut time, &mut tracks, &mut out, 2, None);
        let regions = |tracks: &audiotracks::TrackHost| -> Vec<(Tick, Tick)> {
            let tr = &tracks.host.tracks;
            tr.regions(s).iter().map(|r| (r.start, r.end)).collect()
        };
        assert_eq!(regions(&tracks), [(0, 4 * bar)]);
        // Rec song from the top: the clip launched in bar 2 and stopped
        // for bar 3, three bars heard.
        time.act(TimeAction::SongMode(true)).unwrap();
        time.act(TimeAction::RecSong(true)).unwrap();
        time.command(mpfree_transport::Command::Locate(0));
        run(&mut time, &mut tracks, &mut out, 4, None);
        time.act(TimeAction::Transport(mpfree_transport::Command::Play))
            .unwrap();
        let periods = |frames: usize| frames / PERIOD;
        let mut heard = Vec::new();
        run(
            &mut time,
            &mut tracks,
            &mut out,
            periods(bar_frames / 2),
            Some((s, &mut heard)),
        );
        time.act(TimeAction::Launch { track: at, slot: 0 }).unwrap();
        run(
            &mut time,
            &mut tracks,
            &mut out,
            periods(bar_frames),
            Some((s, &mut heard)),
        );
        time.act(TimeAction::StopClip(at)).unwrap();
        let rest = 3 * bar_frames - heard.len();
        run(
            &mut time,
            &mut tracks,
            &mut out,
            periods(rest) + 1,
            Some((s, &mut heard)),
        );
        time.act(TimeAction::Transport(mpfree_transport::Command::Stop))
            .unwrap();
        time.act(TimeAction::RecSong(false)).unwrap();
        run(&mut time, &mut tracks, &mut out, 4, None);
        let blocks: Vec<(Tick, Tick)> = time
            .project()
            .arrangement
            .placements
            .iter()
            .map(|p| (p.start, p.end()))
            .collect();
        assert_eq!(blocks, [(bar, 2 * bar)]);
        assert_eq!(regions(&tracks), [(0, bar), (2 * bar, 4 * bar)]);
        // The song again from the top.
        time.command(mpfree_transport::Command::Locate(0));
        run(&mut time, &mut tracks, &mut out, 4, None);
        time.act(TimeAction::Transport(mpfree_transport::Command::Play))
            .unwrap();
        let mut again = Vec::new();
        run(
            &mut time,
            &mut tracks,
            &mut out,
            periods(3 * bar_frames) + 1,
            Some((s, &mut again)),
        );
        let n = 3 * bar_frames;
        assert!(heard.len() >= n && again.len() >= n);
        let diff = |i: usize| (heard[i] - again[i]).abs() > 1e-6;
        let bad: Vec<usize> = (0..n).filter(|&i| diff(i)).collect();
        assert!(
            bad.is_empty(),
            "the song plays what Rec song heard: {} frames differ, from {:?} to {:?}",
            bad.len(),
            bad.first(),
            bad.last()
        );
    }

    /// Phase 2, step 2d (Joshi's pick, 23 Nov): an audio clip launched
    /// with Rec song over what its track recorded goes into the song as a
    /// block that replaces the regions under it, and the song's UNDO takes
    /// back both (the block gone, the regions back), REDO both again.
    #[test]
    fn rec_songs_audio_block_replaces_the_regions_under_it_and_undo_takes_back_both() {
        use mpfree_seq::{AudioClip, Clip, Project, RecId, Track};
        use mpfree_transport::Tick;
        struct Quiet;
        impl time::Pads for Quiet {
            fn hit_at(&mut self, _: u16, _: usize, _: u8, _: u32) -> bool {
                true
            }
            fn release_at(&mut self, _: u16, _: usize, _: u32) {}
            fn stop_all(&mut self) {}
            fn pressure_at(&mut self, _: u16, _: usize, _: u8, _: u32) {}
        }
        let bar = 3840;
        let bar_frames = 2 * RATE as usize;
        let mut p = Project::new();
        let mut t = Track::new_audio("Loop", 0);
        let mut clip = Clip::new("Loop", bar);
        clip.audio = Some(AudioClip::of(RecId(7), 0, bar_frames as i64, bar, 120.0));
        t.slots[0] = Some(clip);
        p.tracks.push(t);
        p.give_track_ids();
        p.give_clip_ids();
        let at = p.tracks.len() - 1;
        let mut time = time::TimeHost::with_project(RATE, 120.0, p).unwrap();
        let mut tracks = audiotracks::TrackHost::new(RATE, 10);
        let mut out = Out {
            console: None,
            sink: None,
            dropped: 0,
            report: None,
            queued: None,
            last_draw: Instant::now(),
        };
        let run = |time: &mut time::TimeHost,
                   tracks: &mut audiotracks::TrackHost,
                   out: &mut Out,
                   periods: usize| {
            let z = [0f32; PERIOD];
            for _ in 0..periods {
                time.advance(PERIOD as u32);
                time.period(PERIOD as u32, &mut Quiet);
                tracks.sync(time.project(), time.silenced(), true);
                let t: &time::TimeHost = time;
                tracks.period(
                    t.block(),
                    t.tempo_map(),
                    t.song_on(),
                    (&z, &z),
                    Some(&|p| t.audio(p)),
                );
                cut_blocks_under_takes(out, time, tracks);
            }
        };
        run(&mut time, &mut tracks, &mut out, 1);
        let s = (0..mpfree_audiotracks::MAX_TRACKS)
            .find(|&s| tracks.place(s) == Some(at))
            .expect("an engine track for it");
        // The clip's recording, and four bars the track recorded on the
        // song from bar 1.
        for (id, rec, frames) in [(0, 7, bar_frames), (1, 8, 4 * bar_frames)] {
            let take = mpfree_audiotracks::save::TakeFile {
                id,
                rec: mpfree_audiotracks::RecordingId(rec),
                track: s as _,
                group: id,
                pass: 0,
                bpm: 120.0,
                anchor: 0.0,
                frames,
                channels: 1,
                file: String::new(),
                path: None,
                audio: vec![0.25; frames],
            };
            assert!(tracks.host.tracks.load_take(take).is_ok());
        }
        for _ in 0..100 {
            if tracks.host.tracks.loading() == 0 {
                break;
            }
            run(&mut time, &mut tracks, &mut out, 10);
        }
        assert_eq!(tracks.host.tracks.loading(), 0);
        let map = *time.tempo_map();
        assert!(tracks.host.tracks.place(1, &[], &map));
        let regions = |tracks: &audiotracks::TrackHost| -> Vec<(Tick, Tick)> {
            let tr = &tracks.host.tracks;
            tr.regions(s).iter().map(|r| (r.start, r.end)).collect()
        };
        assert_eq!(regions(&tracks), [(0, 4 * bar)]);
        // Rec song: the clip launched, PLAY, two and a half bars, STOP.
        time.act(TimeAction::Launch { track: at, slot: 0 }).unwrap();
        time.act(TimeAction::RecSong(true)).unwrap();
        time.act(TimeAction::Transport(mpfree_transport::Command::Play))
            .unwrap();
        run(
            &mut time,
            &mut tracks,
            &mut out,
            5 * bar_frames / 2 / PERIOD,
        );
        time.act(TimeAction::Transport(mpfree_transport::Command::Stop))
            .unwrap();
        run(&mut time, &mut tracks, &mut out, 4);
        let song = time.project().arrangement.clone();
        assert_eq!(song.placements.len(), 1);
        let end = song.placements[0].end();
        assert!(end > 2 * bar && end < 4 * bar, "{end}");
        assert_eq!(
            regions(&tracks),
            [(end, 4 * bar)],
            "replaced under the block"
        );
        // The song's UNDO takes back both; REDO brings both back.
        time.act_with_audio(TimeAction::Undo, &mut tracks).unwrap();
        run(&mut time, &mut tracks, &mut out, 4);
        assert!(time.project().arrangement.placements.is_empty());
        assert_eq!(regions(&tracks), [(0, 4 * bar)], "the regions back");
        time.act_with_audio(TimeAction::Redo, &mut tracks).unwrap();
        run(&mut time, &mut tracks, &mut out, 4);
        assert_eq!(time.project().arrangement, song);
        assert_eq!(regions(&tracks), [(end, 4 * bar)]);
    }

    /// Phase 2, step 2d: a take recorded over a block of an audio clip
    /// cuts the block at the take's edges, as a song edit the take's undo
    /// step is tied to: the song's UNDO takes back both (the block whole
    /// again, the take's region gone) and REDO brings both back, while
    /// the audio tracks' own Undo stops at the step.
    #[test]
    fn a_take_over_a_block_cuts_it_and_either_undo_takes_back_both() {
        use mpfree_seq::{AudioClip, Clip, Placement, Project, RecId, Track};
        use mpfree_transport::Tick;
        struct Quiet;
        impl time::Pads for Quiet {
            fn hit_at(&mut self, _: u16, _: usize, _: u8, _: u32) -> bool {
                true
            }
            fn release_at(&mut self, _: u16, _: usize, _: u32) {}
            fn stop_all(&mut self) {}
            fn pressure_at(&mut self, _: u16, _: usize, _: u8, _: u32) {}
        }
        let bar = 3840;
        let bar_frames = 2 * RATE as usize;
        let mut p = Project::new();
        let mut t = Track::new_audio("Vocal", 0);
        let mut clip = Clip::new("Loop", bar);
        clip.audio = Some(AudioClip::of(RecId(7), 0, bar_frames as i64, bar, 120.0));
        t.slots[0] = Some(clip);
        p.tracks.push(t);
        p.give_track_ids();
        p.give_clip_ids();
        let at = p.tracks.len() - 1;
        // A four-bar block of the clip on its lane.
        let block = Placement::of(&p.tracks, at, 0, 0, 4 * bar);
        p.arrangement.placements.push(block);
        let mut time = time::TimeHost::with_project(RATE, 120.0, p).unwrap();
        let mut tracks = audiotracks::TrackHost::new(RATE, 10);
        let mut out = Out {
            console: None,
            sink: None,
            dropped: 0,
            report: None,
            queued: None,
            last_draw: Instant::now(),
        };
        let take = mpfree_audiotracks::save::TakeFile {
            id: 0,
            rec: mpfree_audiotracks::RecordingId(7),
            track: 0,
            group: 0,
            pass: 0,
            bpm: 120.0,
            anchor: 0.0,
            frames: bar_frames,
            channels: 1,
            file: String::new(),
            path: None,
            audio: vec![0.25; bar_frames],
        };
        assert!(tracks.host.tracks.load_take(take).is_ok());
        let run = |time: &mut time::TimeHost,
                   tracks: &mut audiotracks::TrackHost,
                   out: &mut Out,
                   periods: usize| {
            let z = [0f32; PERIOD];
            for _ in 0..periods {
                time.advance(PERIOD as u32);
                time.period(PERIOD as u32, &mut Quiet);
                tracks.sync(time.project(), time.silenced(), true);
                let t: &time::TimeHost = time;
                tracks.period(
                    t.block(),
                    t.tempo_map(),
                    t.song_on(),
                    (&z, &z),
                    Some(&|p| t.audio(p)),
                );
                cut_blocks_under_takes(out, time, tracks);
            }
        };
        run(&mut time, &mut tracks, &mut out, 4);
        let s = (0..mpfree_audiotracks::MAX_TRACKS)
            .find(|&s| tracks.place(s) == Some(at))
            .expect("an engine track for it");
        let lane = |time: &time::TimeHost| -> Vec<(Tick, Tick)> {
            let song = &time.project().arrangement;
            song.placements
                .iter()
                .filter(|p| p.track == at)
                .map(|p| (p.start, p.end()))
                .collect()
        };
        let regions = |tracks: &audiotracks::TrackHost| -> Vec<(Tick, Tick)> {
            let tr = &tracks.host.tracks;
            tr.regions(s).iter().map(|r| (r.start, r.end)).collect()
        };
        assert_eq!(lane(&time), [(0, 4 * bar)]);
        // Song mode, armed, recorded from bar 2 for a bar.
        time.act(TimeAction::SongMode(true)).unwrap();
        let info = time.info();
        tracks
            .action(AudioAction::Arm { track: s, on: true }, &info, false)
            .unwrap();
        time.command(mpfree_transport::Command::Locate(bar));
        run(&mut time, &mut tracks, &mut out, 40);
        time.act(TimeAction::Transport(mpfree_transport::Command::Record))
            .unwrap();
        run(&mut time, &mut tracks, &mut out, bar_frames / PERIOD);
        time.act(TimeAction::Transport(mpfree_transport::Command::Stop))
            .unwrap();
        run(&mut time, &mut tracks, &mut out, 40);
        let took = regions(&tracks);
        assert_eq!(took.len(), 1, "the take's region");
        let (from, to) = took[0];
        assert!(from >= bar && from < bar + bar / 4, "{from}");
        assert!(to > from + bar / 2, "{to}");
        assert_eq!(lane(&time), [(0, from), (to, 4 * bar)], "cut at its edges");
        let cut = time.project().arrangement.clone();
        // The audio tracks' own Undo stops at the step.
        let refused = tracks.host.tracks.stats().refused;
        assert!(tracks.host.command(mpfree_audiotracks::Command::Undo));
        run(&mut time, &mut tracks, &mut out, 2);
        assert_eq!(tracks.host.tracks.stats().refused, refused + 1);
        assert_eq!(regions(&tracks), took);
        // The song's UNDO takes back both; REDO brings both back.
        time.act_with_audio(TimeAction::Undo, &mut tracks).unwrap();
        run(&mut time, &mut tracks, &mut out, 4);
        assert_eq!(lane(&time), [(0, 4 * bar)], "the block whole again");
        assert!(regions(&tracks).is_empty(), "the take's region gone");
        time.act_with_audio(TimeAction::Redo, &mut tracks).unwrap();
        run(&mut time, &mut tracks, &mut out, 4);
        assert_eq!(time.project().arrangement, cut);
        assert_eq!(regions(&tracks), took);
    }

    /// Arrange 3.4: a section moved or + Bar moves the audio regions in it
    /// in the first period after, with the song; UNDO and REDO move them
    /// back and on with it; the audio row's own Undo stops at the song's
    /// edit; a recording, or an audio change waiting for the period,
    /// refuses the edit and nothing moves anywhere; UNDO with the audio
    /// tracks' step gone leaves the regions, says so and counts it.
    #[test]
    fn song_edits_move_the_audio_regions_with_the_song() {
        let bar = SongRig::BAR;
        let mut r = SongRig::new(false);
        // B moved to the start: the audio in it goes with it, in the first
        // period after.
        let fx = r.act(TimeAction::MoveSection {
            marker: 1,
            gap: 0,
            copy: false,
        });
        assert!(SongRig::told(&fx).contains("moved"), "{fx:?}");
        assert_eq!(r.markers(), "B A C");
        r.period();
        assert_eq!(r.audio(), 0);
        // The audio row's Undo stops at it; the song's UNDO takes both back.
        let info = r.time.info();
        let e = r.tracks.action(AudioAction::Undo, &info, true).unwrap_err();
        assert!(e.contains("UNDO takes it back, with the song"), "{e}");
        let fx = r.act(TimeAction::Undo);
        assert!(SongRig::told(&fx).starts_with("Undid "), "{fx:?}");
        assert_eq!(r.markers(), "A B C");
        r.period();
        assert_eq!(r.audio(), 2 * bar);
        let e = r.tracks.action(AudioAction::Redo, &info, true).unwrap_err();
        assert!(e.contains("SHIFT + UNDO brings it back"), "{e}");
        r.act(TimeAction::Redo);
        r.period();
        assert_eq!((r.markers().as_str(), r.audio()), ("B A C", 0));
        // + Bar before B, twice: one UNDO takes both bars back, from the
        // song and the audio.
        for _ in 0..2 {
            r.act(TimeAction::SongBars(
                mpfree_time_ui::view::BarsEdit::RoomBefore(0),
            ));
            r.period();
        }
        assert_eq!(r.audio(), 2 * bar);
        r.act(TimeAction::Undo);
        r.period();
        assert_eq!(r.audio(), 0);
        r.act(TimeAction::Redo);
        r.period();
        assert_eq!(r.audio(), 2 * bar);
        // An audio change waiting for the period: the edit waits, nothing
        // moves anywhere.
        let edits = r.time.project().arrangement.clone();
        assert!(r.tracks.host.command(mpfree_audiotracks::Command::SetGain {
            track: 0,
            at: 2 * bar + 10,
            gain: 0.5,
        }));
        let fx = r.act(TimeAction::MoveSection {
            marker: 1,
            gap: 3,
            copy: false,
        });
        assert!(
            SongRig::told(&fx).contains("taking the last change"),
            "{fx:?}"
        );
        let fx = r.act(TimeAction::Undo);
        assert!(SongRig::told(&fx).starts_with("UNDO waits: "), "{fx:?}");
        r.period();
        assert!(r.time.project().arrangement == edits);
        assert_eq!(r.audio(), 2 * bar);
        // Edited since: UNDO waits for the audio's own Undo.
        let fx = r.act(TimeAction::Undo);
        assert!(
            SongRig::told(&fx).contains("pick a region, then Undo"),
            "{fx:?}"
        );
        r.tracks.action(AudioAction::Undo, &info, true).unwrap();
        r.period();
        // The tracks' history cleared: UNDO takes the song back, the
        // regions stay, and it is said and counted.
        r.tracks
            .host
            .command(mpfree_audiotracks::Command::ClearHistory);
        r.period();
        let fx = r.act(TimeAction::Undo);
        let said = SongRig::told(&fx);
        assert!(
            said.starts_with("Undid ") && said.contains("stay where they are"),
            "{said}"
        );
        r.period();
        assert_eq!((r.markers().as_str(), r.audio()), ("B A C", 2 * bar));
        assert!(
            r.time
                .describe()
                .contains("1 song edits undone with the audio regions left where they were"),
            "{}",
            r.time.describe()
        );
        // REDO makes the same edit again on the regions as they are.
        r.act(TimeAction::Redo);
        r.period();
        assert_eq!(r.audio(), 4 * bar);
        // While audio 1 records, a section move is refused, and nothing
        // moves anywhere.
        r.act(TimeAction::SongMode(true));
        r.tracks
            .action(AudioAction::Arm { track: 0, on: true }, &info, true)
            .unwrap();
        r.period();
        assert!(r.time.command(mpfree_transport::Command::Record));
        r.settle(|r| r.tracks.host.tracks.recording(0).is_some());
        let before = r.time.project().arrangement.clone();
        let fx = r.act(TimeAction::MoveSection {
            marker: 0,
            gap: 3,
            copy: false,
        });
        assert_eq!(
            SongRig::told(&fx),
            "song: nothing moved: Audio 1 is recording (stop it first)"
        );
        r.period();
        assert!(r.time.project().arrangement == before);
        assert_eq!(r.audio(), 4 * bar);
    }

    /// A bar put in before a section, an audio change, then another bar
    /// there: two UNDOs, each taking back its own bar from the song and
    /// the audio (one couldn't take both back from the regions past the
    /// change).
    #[test]
    fn a_bar_put_in_after_an_audio_change_is_an_undo_of_its_own() {
        let bar = SongRig::BAR;
        let mut r = SongRig::new(false);
        let b_at = |r: &SongRig| r.time.project().arrangement.markers[1].at;
        let room = TimeAction::SongBars(mpfree_time_ui::view::BarsEdit::RoomBefore(1));
        r.act(room.clone());
        r.period();
        assert!(r.tracks.host.command(mpfree_audiotracks::Command::SetGain {
            track: 0,
            at: 3 * bar + 10,
            gain: 0.8,
        }));
        r.period();
        r.act(room);
        r.period();
        assert_eq!((b_at(&r), r.audio()), (4 * bar, 4 * bar));
        r.act(TimeAction::Undo);
        r.period();
        assert_eq!((b_at(&r), r.audio()), (3 * bar, 3 * bar));
        let info = r.time.info();
        r.tracks.action(AudioAction::Undo, &info, true).unwrap();
        r.period();
        r.act(TimeAction::Undo);
        r.period();
        assert_eq!((b_at(&r), r.audio()), (2 * bar, 2 * bar));
    }

    /// The one history (audio tracks phase 4d.1), as the roadmap tests
    /// it: a take, an overdub, a region edit, then a song edit over the
    /// regions; UNDO four times takes them back newest first, each from
    /// its own engine (the song edit with its regions), and REDO brings
    /// them back in order. Nothing is left to undo but what was there
    /// before; with REDO done there is nothing to redo.
    #[test]
    fn the_one_undo_takes_back_the_newest_step_in_any_engine_and_redo_brings_it_back() {
        use mpfree_looper::{Command as L, Phase};
        use mpfree_time_ui::view::BarsEdit;
        use one_undo::{OneUndo, Part};
        let bar = SongRig::BAR;
        let mut r = SongRig::new(false);
        r.looper = looping::LoopHost::new(RATE, 20);
        let mut one = OneUndo::default();
        // Periods as the audio loop runs them, the looper too (the pads
        // give it a steady 0.25 while `sound`), then the one history
        // follows.
        let run = |r: &mut SongRig, one: &mut OneUndo, n: usize, sound: bool| {
            for _ in 0..n {
                r.period();
                for line in r.looper.follow_plan(&r.time.loop_plan()) {
                    assert!(!line.contains("refused"), "{line}");
                }
                let x = if sound { 0.25 } else { 0.0 };
                r.looper.take_pads(&[x; PERIOD], &[-x; PERIOD]);
                let (mut left, mut right) = ([0.0; PERIOD], [0.0; PERIOD]);
                r.looper.run(r.time.block(), &mut left, &mut right);
                one.follow(&r.time, &r.tracks, &r.looper);
                let _ = one.tend(&mut r.out, &mut r.time, &mut r.tracks, &mut r.looper);
            }
        };
        run(&mut r, &mut one, 2, false);
        let before = one.next(false).map(|e| e.txn);
        let tracks = r.time.project().tracks.len();
        // A take: + Audio track.
        r.act(TimeAction::AddAudioTrack);
        run(&mut r, &mut one, 2, false);
        assert_eq!(r.time.project().tracks.len(), tracks + 1);
        // An overdub on a loop of one bar.
        assert!(r.time.command(mpfree_transport::Command::Play));
        r.looper.command(L::SetBars(Some(1)));
        r.looper.command(L::Record);
        let bar_periods = 2 * RATE as usize / PERIOD + 1;
        for _ in 0..3 * bar_periods {
            run(&mut r, &mut one, 1, true);
            if r.looper.looper.status().phase == Phase::Playing {
                break;
            }
        }
        assert_eq!(r.looper.looper.status().phase, Phase::Playing);
        r.looper.command(L::Overdub);
        run(&mut r, &mut one, 50, true);
        r.looper.command(L::Overdub);
        run(&mut r, &mut one, 20, false);
        assert!(r.time.command(mpfree_transport::Command::Stop));
        run(&mut r, &mut one, 2, false);
        let layers = |r: &SongRig| r.looper.looper.status().layers;
        assert_eq!(layers(&r), 2);
        // A region edit, then a song edit over the regions.
        assert!(r.tracks.host.command(mpfree_audiotracks::Command::SetGain {
            track: 0,
            at: 2 * bar + 10,
            gain: 0.5,
        }));
        run(&mut r, &mut one, 2, false);
        let gain_steps = |r: &SongRig| r.tracks.host.tracks.history();
        let (gains_done, _) = gain_steps(&r);
        r.act(TimeAction::SongBars(BarsEdit::RoomBefore(1)));
        run(&mut r, &mut one, 2, false);
        let b_at = |r: &SongRig| r.time.project().arrangement.markers[1].at;
        assert_eq!((b_at(&r), r.audio()), (3 * bar, 3 * bar));

        // UNDO four times: newest first, wherever each is.
        let press = |r: &mut SongRig, one: &mut OneUndo, redo: bool| {
            let fx = one_undo::press(
                one,
                redo,
                &mut r.out,
                &mut r.time,
                &mut r.tracks,
                &mut r.looper,
                &mut r.app,
                &mut r.mix,
            );
            run(r, one, 3, false);
            SongRig::told(&fx)
        };
        let next = |one: &OneUndo, redo: bool| one.next(redo).map(|e| e.part);
        assert_eq!(next(&one, false), Some(Part::Time));
        assert!(press(&mut r, &mut one, false).starts_with("Undid "));
        assert_eq!(
            (b_at(&r), r.audio()),
            (2 * bar, 2 * bar),
            "with its regions"
        );
        assert_eq!(next(&one, false), Some(Part::Audio));
        let said = press(&mut r, &mut one, false);
        assert!(said.starts_with("Undid region edit"), "{said}");
        assert_eq!(gain_steps(&r).0, gains_done - 1);
        assert_eq!(next(&one, false), Some(Part::Loop(0)));
        let said = press(&mut r, &mut one, false);
        assert_eq!(said, "Undid Overdub · Layer 2 · Loop 1");
        assert_eq!(layers(&r), 1);
        assert_eq!(next(&one, false), Some(Part::Time));
        assert!(press(&mut r, &mut one, false).starts_with("Undid new track"));
        assert_eq!(r.time.project().tracks.len(), tracks);
        // What was there before is next; the four are to redo.
        assert_eq!(one.next(false).map(|e| e.txn), before);
        // REDO: in the order they were made.
        assert!(press(&mut r, &mut one, true).starts_with("Redid new track"));
        assert_eq!(r.time.project().tracks.len(), tracks + 1);
        assert_eq!(
            press(&mut r, &mut one, true),
            "Redid Overdub · Layer 2 · Loop 1"
        );
        assert_eq!(layers(&r), 2);
        assert!(press(&mut r, &mut one, true).starts_with("Redid region edit"));
        assert_eq!(gain_steps(&r).0, gains_done);
        assert!(press(&mut r, &mut one, true).starts_with("Redid "));
        assert_eq!((b_at(&r), r.audio()), (3 * bar, 3 * bar));
        assert_eq!(gain_steps(&r).1, 0, "the song edit's regions with it");
        assert_eq!(next(&one, true), None);
        assert_eq!(press(&mut r, &mut one, true), "Nothing to redo");

        // Two UNDOs (the song edit with its regions, the region edit),
        // then a new step on the loop: nothing is left to redo, and every
        // engine forgets its own redo steps (phase 4d.3).
        press(&mut r, &mut one, false);
        press(&mut r, &mut one, false);
        let take_redo = |r: &SongRig| {
            let (mut done, mut redo) = (Vec::new(), Vec::new());
            r.time.take_steps(&mut done, &mut redo);
            redo.len()
        };
        assert_eq!(take_redo(&r), 1);
        assert!(gain_steps(&r).1 >= 2, "the region edit and the song's");
        r.looper.command(L::Edit(mpfree_looper::Edit::Reverse));
        run(&mut r, &mut one, 10, false);
        assert_eq!(next(&one, false), Some(Part::Loop(0)));
        assert_eq!(next(&one, true), None);
        assert_eq!((take_redo(&r), gain_steps(&r).1), (0, 0));
        assert_eq!((b_at(&r), r.audio()), (2 * bar, 2 * bar), "nothing moved");
        assert_eq!(press(&mut r, &mut one, true), "Nothing to redo");
    }

    /// A period as the audio loop runs one with Dev's switch on, for phase
    /// 4d.4's tests: layers get recordings and go to their files, the
    /// looper runs on the pads' steady `x`, and the one history follows
    /// and tends. What it says comes back.
    fn period_with_files(r: &mut SongRig, one: &mut one_undo::OneUndo, x: f32) -> Option<String> {
        r.period();
        let _ = r.looper.follow_plan(&r.time.loop_plan());
        let tracks = &mut r.tracks;
        r.looper.give_layer_recs(|| tracks.fresh_recording_id());
        assert_eq!(r.looper.write_layers(&mut r.tracks, PERIOD), None);
        r.looper.take_pads(&[x; PERIOD], &[-x; PERIOD]);
        let (mut left, mut right) = ([0.0; PERIOD], [0.0; PERIOD]);
        r.looper.run(r.time.block(), &mut left, &mut right);
        one.follow(&r.time, &r.tracks, &r.looper);
        let fx = one.tend(&mut r.out, &mut r.time, &mut r.tracks, &mut r.looper);
        (!fx.is_empty()).then(|| SongRig::told(&fx))
    }

    /// Periods until every layer of loop 1 is in its synced file
    /// (`synced`), or until the one history says something: the worker
    /// is a thread of its own.
    fn until_worker(r: &mut SongRig, one: &mut one_undo::OneUndo, synced: bool) -> String {
        let t0 = Instant::now();
        loop {
            assert!(
                t0.elapsed() < Duration::from_secs(30),
                "the worker never answered"
            );
            let told = period_with_files(r, one, 0.0);
            if synced && r.looper.unsaved_layers(0) == 0 {
                return String::new();
            }
            if let (false, Some(line)) = (synced, told) {
                return line;
            }
            std::thread::sleep(Duration::from_micros(200));
        }
    }

    /// UNDO (or REDO) as the desk sends it with the switch on, and what
    /// it said.
    fn press_one(r: &mut SongRig, one: &mut one_undo::OneUndo, redo: bool) -> String {
        let fx = one_undo::press(
            one,
            redo,
            &mut r.out,
            &mut r.time,
            &mut r.tracks,
            &mut r.looper,
            &mut r.app,
            &mut r.mix,
        );
        SongRig::told(&fx)
    }

    /// Phase 4d.4's rig: on the real engines, with the audio tracks'
    /// worker writing into `dir`, a loop of three layers, its second
    /// deleted, a fourth overdub that took its memory (the deleted layer
    /// is away, in its file) and every layer synced. Gives the rig, the
    /// one history, the deleted layer, its size and its audio.
    fn a_layer_in_its_file_only(
        dir: &std::path::Path,
    ) -> (
        Box<SongRig>,
        one_undo::OneUndo,
        mpfree_looper::LayerInfo,
        usize,
        Vec<f32>,
    ) {
        use mpfree_looper::{Command as L, LayerCommand as Lc, Phase};
        let _ = std::fs::remove_dir_all(dir);
        std::fs::create_dir_all(dir).unwrap();
        // On the heap: the rig is big for a test thread's stack.
        let mut rig = Box::new(SongRig::new(false));
        let r = &mut *rig;
        // Seven seconds: a first pass of a bar (two seconds) with what it
        // keeps and two overdubs, not a third.
        r.looper = looping::LoopHost::new(RATE, 7);
        r.looper.set_recordings(true);
        r.tracks.record_to(dir.to_path_buf());
        let mut one = one_undo::OneUndo::default();
        let run = |r: &mut SongRig, one: &mut one_undo::OneUndo, n: usize, x: f32| {
            for _ in 0..n {
                period_with_files(r, one, x);
            }
        };
        assert!(r.time.command(mpfree_transport::Command::Play));
        r.looper.command(L::SetBars(Some(1)));
        r.looper.command(L::Record);
        let bar_periods = 2 * RATE as usize / PERIOD + 1;
        for _ in 0..3 * bar_periods {
            run(&mut *r, &mut one, 1, 0.25);
            if r.looper.looper.status().phase == Phase::Playing {
                break;
            }
        }
        assert_eq!(r.looper.looper.status().phase, Phase::Playing);
        let overdub = |r: &mut SongRig, one: &mut one_undo::OneUndo, x: f32| {
            r.looper.command(L::Overdub);
            run(r, one, bar_periods, x);
            r.looper.command(L::Overdub);
            run(r, one, 20, 0.0);
        };
        overdub(&mut *r, &mut one, 0.125);
        overdub(&mut *r, &mut one, 0.0625);
        assert_eq!(r.looper.looper.status().layers, 3);
        until_worker(&mut *r, &mut one, true);
        // The second layer deleted: only its UNDO needs it now.
        let second = r.looper.looper.layer_info(1).unwrap();
        let size = r.looper.looper.raw_of(0, second.born, 0, &mut []).unwrap();
        let mut raw = vec![0.0; 2 * size];
        r.looper.looper.raw_of(0, second.born, 0, &mut raw);
        r.looper.layer_command(second.id, Lc::Delete);
        run(&mut *r, &mut one, 20, 0.0);
        // A new overdub needs its room: it leaves memory.
        overdub(&mut *r, &mut one, 0.5);
        assert_eq!(r.looper.looper.status().layers, 3, "the overdub had room");
        assert!(r.looper.looper.is_away(0, second.born));
        until_worker(&mut *r, &mut one, true);
        (rig, one, second, size, raw)
    }

    /// Phase 4d.4 on the real engines: a layer only UNDO needs, written to
    /// its file and synced, leaves memory when a new overdub needs the
    /// room. The one UNDO that needs it says it is bringing it back (and
    /// another UNDO meanwhile that it still is), the overdub kept for
    /// REDO makes room for it, the audio tracks' worker reads its file,
    /// it goes in a piece a period, and then the step is taken back, the
    /// layer exactly as it was. REDO brings back the overdub in turn.
    #[test]
    fn the_one_undo_brings_a_layer_back_from_its_file_before_its_step() {
        use one_undo::Part;
        let dir = std::env::temp_dir().join(format!("mpfree-one-undo-back-{}", std::process::id()));
        let (mut rig, mut one, second, size, raw) = a_layer_in_its_file_only(&dir);
        let r = &mut *rig;
        let run = |r: &mut SongRig, one: &mut one_undo::OneUndo, n: usize| {
            for _ in 0..n {
                assert_eq!(period_with_files(r, one, 0.0), None);
            }
        };
        let layers = |r: &SongRig| r.looper.looper.status().layers;
        // UNDO: the new overdub, needing nothing.
        let said = press_one(&mut *r, &mut one, false);
        assert!(said.starts_with("Undid Overdub"), "{said}");
        run(&mut *r, &mut one, 3);
        assert_eq!(layers(r), 2);
        // UNDO: the Delete waits for its layer.
        assert_eq!(one.next(false).map(|e| e.part), Some(Part::Loop(0)));
        let delete = one.next(false).map(|e| e.words.clone()).unwrap();
        assert_eq!(
            press_one(&mut *r, &mut one, false),
            "Bringing back Loop 1's layer"
        );
        assert_eq!(
            press_one(&mut *r, &mut one, false),
            "Still bringing back Loop 1's layer"
        );
        let said = until_worker(&mut *r, &mut one, false);
        assert_eq!(said, format!("Undid {delete}"));
        run(&mut *r, &mut one, 3);
        assert_eq!(layers(r), 3, "the deleted layer is back");
        let mut again = vec![0.0; 2 * size];
        assert_eq!(
            r.looper.looper.raw_of(0, second.born, 0, &mut again),
            Some(size)
        );
        assert_eq!(again, raw, "exactly as it was");
        // REDO: the Delete again, needing nothing; then the overdub, which
        // made room and comes back from its file in turn.
        let said = press_one(&mut *r, &mut one, true);
        assert_eq!(said, format!("Redid {delete}"));
        run(&mut *r, &mut one, 3);
        assert_eq!(layers(r), 2);
        assert_eq!(
            press_one(&mut *r, &mut one, true),
            "Bringing back Loop 1's layer"
        );
        let said = until_worker(&mut *r, &mut one, false);
        assert!(said.starts_with("Redid Overdub"), "{said}");
        run(&mut *r, &mut one, 3);
        assert_eq!(layers(r), 3, "the overdub is back on top");
        assert_eq!(one.next(true).map(|e| e.txn), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Phase 4d.4: the Looper tab's history list (holding UNDO) goes back
    /// two steps at once: the overdub, then, once the deleted layer it
    /// waits for is brought back from its file, the Delete; and forward
    /// again, the overdub coming back in turn. With Dev's switch turned
    /// off, the loop's own UNDO of a step whose layer is away waits for it
    /// the same way rather than being refused.
    #[test]
    fn the_looper_tabs_history_list_waits_for_a_layer_away_and_goes_on() {
        use mpfree_looper::Command as L;
        let dir = std::env::temp_dir().join(format!("mpfree-one-undo-list-{}", std::process::id()));
        let (mut rig, mut one, second, size, raw) = a_layer_in_its_file_only(&dir);
        let r = &mut *rig;
        let layers = |r: &SongRig| r.looper.looper.status().layers;
        // Until loop 1's history has `done` steps to undo and nothing is
        // coming back: the worker is a thread; what is said on the way
        // comes back.
        let settle = |r: &mut SongRig, one: &mut one_undo::OneUndo, done: usize| {
            let t0 = Instant::now();
            let mut told = Vec::new();
            while r.looper.looper.history().done().len() != done
                || r.looper.looper.bringing_back()
                || r.looper.waits_for_layer().is_some()
            {
                assert!(t0.elapsed() < Duration::from_secs(30), "{told:?}");
                told.extend(period_with_files(r, one, 0.0));
                std::thread::sleep(Duration::from_micros(200));
            }
            for _ in 0..20 {
                told.extend(period_with_files(r, one, 0.0));
            }
            told
        };
        // Back two: the overdub, then the Delete, which waits.
        assert_eq!(r.looper.steps(-2), "looper: undo history: back 2 steps");
        let told = settle(r, &mut one, 2);
        assert_eq!(layers(r), 3);
        assert_eq!(told, ["Bringing back Loop 1's layer"]);
        let mut again = vec![0.0; 2 * size];
        assert_eq!(
            r.looper.looper.raw_of(0, second.born, 0, &mut again),
            Some(size)
        );
        assert_eq!(again, raw, "the deleted layer, exactly as it was");
        // Forward two: the Delete, then the overdub, which made room and
        // comes back from its file in turn.
        assert_eq!(r.looper.steps(2), "looper: undo history: forward 2 steps");
        let told = settle(r, &mut one, 4);
        assert_eq!(layers(r), 3);
        assert_eq!(told, ["Bringing back Loop 1's layer"]);
        assert_eq!(r.looper.looper.history().redo().len(), 0);
        assert!(r.looper.looper.is_away(0, second.born), "it made room");
        // The switch off: the loop's own UNDO of the Delete waits for its
        // layer as the list does, rather than being refused.
        r.looper.set_recordings(false);
        r.looper.command(L::Undo);
        settle(r, &mut one, 3);
        assert_eq!(layers(r), 2);
        r.looper.command(L::Undo);
        let told = settle(r, &mut one, 2);
        assert_eq!(layers(r), 3);
        assert_eq!(told, ["Bringing back Loop 1's layer"]);
        assert!(!r.looper.looper.is_away(0, second.born));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Phase 4d.4: a layer whose file is gone, or shorter than the layer,
    /// can't come back. The one UNDO says why, its memory goes back, the
    /// layer stays away and its step stays next, so UNDO tries again.
    #[test]
    fn a_layer_whose_file_is_gone_or_short_cant_come_back_and_its_step_stays() {
        let dir = std::env::temp_dir().join(format!("mpfree-one-undo-gone-{}", std::process::id()));
        let (mut rig, mut one, second, _, raw) = a_layer_in_its_file_only(&dir);
        let r = &mut *rig;
        assert!(press_one(&mut *r, &mut one, false).starts_with("Undid Overdub"));
        for _ in 0..3 {
            assert_eq!(period_with_files(&mut *r, &mut one, 0.0), None);
        }
        let delete = one.next(false).map(|e| e.txn);
        let rec = r.looper.layer_rec(0, second.born);
        let file = dir.join(mpfree_audiotracks::files::layer_file_name(
            mpfree_audiotracks::RecordingId(rec),
        ));
        assert!(file.is_file(), "{file:?}");
        let used = r.looper.looper.memory().used;
        let refused = |r: &mut SongRig, one: &mut one_undo::OneUndo| {
            assert_eq!(press_one(r, one, false), "Bringing back Loop 1's layer");
            let said = until_worker(r, one, false);
            assert!(
                said.starts_with("Loop 1's layer can't come back: "),
                "{said}"
            );
            assert!(!r.looper.looper.bringing_back());
            assert!(r.looper.looper.is_away(0, second.born));
            assert_eq!(
                one.next(false).map(|e| e.txn),
                delete,
                "the step stays next"
            );
            assert_eq!(r.looper.looper.status().layers, 2);
            said
        };
        // Gone: the folder emptied and made again (main.rs never removes a
        // file by name; the MPC inventory check holds it to that).
        std::fs::remove_dir_all(&dir).unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!file.exists());
        let said = refused(&mut *r, &mut one);
        assert!(said.contains("loop_"), "it names the file: {said}");
        // Short: half of its audio.
        let half = &raw[..raw.len() / 2];
        std::fs::write(&file, mpfree_audiotracks::save::wav(half, 2, RATE)).unwrap();
        let said = refused(&mut *r, &mut one);
        assert!(said.ends_with("its file is short"), "{said}");
        // Each time, the memory it took went back (the overdub kept for
        // REDO went away to make room, so there is more free now).
        assert!(r.looper.looper.memory().used <= used);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The same apart (--time-thread): the control half is lent for the
    /// edit, so the song and the audio still move in the same period.
    #[test]
    fn apart_song_edits_move_the_audio_regions_with_the_song() {
        let bar = SongRig::BAR;
        let mut r = SongRig::new(true);
        let fx = r.act(TimeAction::MoveSection {
            marker: 1,
            gap: 0,
            copy: false,
        });
        assert!(SongRig::told(&fx).contains("moved"), "said at once: {fx:?}");
        r.period();
        assert_eq!(r.audio(), 0);
        r.settle(|r| r.markers() == "B A C");
        r.act(TimeAction::Undo);
        r.period();
        assert_eq!(r.audio(), 2 * bar);
        r.settle(|r| r.markers() == "A B C");
    }

    #[test]
    fn a_plugin_preset_reads_with_its_folder_first() {
        let mut p = mpfree_plughost::Preset {
            name: "Attacky".into(),
            place: mpfree_plughost::Place::Plugin,
            load_key: None,
            plugins: Vec::new(),
            collection: String::new(),
            category: "Bass".into(),
            creator: String::new(),
            flags: 0,
        };
        assert_eq!(plugin_preset_name(&p), "Bass / Attacky");
        p.category.clear();
        assert_eq!(plugin_preset_name(&p), "Attacky");
        // A track without its own synth shows the shared synth's presets.
        let synth = synth::Synth::new(RATE);
        let mix = mixing::MixHost::new(RATE, -6.0, -6.0);
        assert_eq!(
            synth_tab(&synth, &mix, 0, &presets::TrackPresets::default()).presets,
            synth.state().presets
        );
    }

    /// A project saved before step 10 kept kit 1's pad mutes and solos in
    /// the mixer: as it opens, each track playing kit 1 takes them as its
    /// own, a track on another kit or playing notes doesn't, and the log
    /// says so. Taken once; a new project has none to carry.
    #[test]
    fn an_old_projects_kit_1_pad_mutes_go_to_each_track_on_kit_1() {
        let mut time = time::TimeHost::new(RATE, 120.0).unwrap();
        let mut mix = mixing::MixHost::new(RATE, -6.0, 0.0);
        let text = project::mixer_text(&mix.mixer, 0).replace(
            &format!("{}\n", project::MIXER_FORMAT),
            &format!("{}\npadmutes 5\npadsolos 16\n", project::MIXER_FORMAT),
        );
        let load = project::mixer_load(&text, None, RATE, &plugins::Plugins::none()).unwrap();
        mix.open(Box::new(load));
        // Tracks 1 and 3 on kit 1, track 2 on kit 2, track 4 the synth.
        let mut kits = sampler::TrackKits::new(2);
        kits.set(1, 1);
        kits.set(3, kits.synth());
        let tracks = time.track_count();
        let line = carry_pad_mutes(&mut mix, &kits, &mut time).unwrap();
        let kit_1 = (0..tracks).filter(|&t| t != 1 && t != 3).count();
        assert_eq!(
            line,
            format!("project: kit 1's pads (muted A1 A3, soloed A5) are now each of its {kit_1} tracks' own")
        );
        for t in [0, 2] {
            assert_eq!(time.pad_mutes(t), Some((5, 16)), "track {}", t + 1);
        }
        for t in [1, 3] {
            assert_eq!(time.pad_mutes(t), Some((0, 0)), "track {}", t + 1);
        }
        assert_eq!(mix.pad_mutes(), 0, "no channel muted");
        assert_eq!(carry_pad_mutes(&mut mix, &kits, &mut time), None, "once");
    }

    /// TRACK MUTE's keys mute a track, or a pad of the track's kit on any
    /// bank (step 10: the track's alone), and the pads
    /// show it: green playing (dim with nothing launched), red muted or
    /// silent by solo, blue soloed, amber blinking while waiting, dark
    /// where there is nothing.
    #[test]
    fn the_mute_keys_mute_tracks_or_pads_and_light_what_plays() {
        use app::{MuteKeys, MuteTap};
        let mut time = time::TimeHost::new(RATE, 120.0).unwrap();
        let tap = |keys, bank| MuteTap {
            keys,
            bank,
            wait: false,
            solo: false,
        };
        assert_eq!(
            mute_key(tap(MuteKeys::Tracks, 0), 2, 0, 0, None, &mut time),
            "track mute: Track 3 muted"
        );
        assert_eq!(time.muted(2), Some(true));
        assert_eq!(
            mute_key(tap(MuteKeys::Tracks, 0), 9, 0, 0, None, &mut time),
            "track mute: no track 10"
        );
        // Bank B: pad 3 is track 19, once there is one.
        assert_eq!(
            mute_key(tap(MuteKeys::Tracks, 1), 2, 0, 0, None, &mut time),
            "track mute: no track 19"
        );
        while time.track_count() < 24 {
            time.act(TimeAction::AddTrack).unwrap();
        }
        assert_eq!(
            mute_key(tap(MuteKeys::Tracks, 1), 2, 0, 0, None, &mut time),
            "track mute: Track 19 muted"
        );
        assert_eq!(time.muted(18), Some(true));
        assert_eq!(time.muted(2), Some(true), "bank A's track 3 is still muted");
        // Held, the tap solos: the others go silent.
        let solo = MuteTap {
            solo: true,
            ..tap(MuteKeys::Tracks, 0)
        };
        assert_eq!(
            mute_key(solo, 0, 0, 0, None, &mut time),
            "track mute: Track 1 solo"
        );
        let l = time.mute_light(1).unwrap();
        assert!(l.silent && !l.solo && !l.waiting);
        assert!(time.mute_light(0).unwrap().solo);
        assert_eq!(
            mute_key(solo, 0, 0, 0, None, &mut time),
            "track mute: Track 1 solo off"
        );
        assert!(!time.mute_light(1).unwrap().silent);
        // Stopped, a tap that would wait acts at once.
        let wait = MuteTap {
            wait: true,
            ..tap(MuteKeys::Tracks, 0)
        };
        assert_eq!(
            mute_key(wait, 2, 0, 0, None, &mut time),
            "track mute: Track 3 plays"
        );
        // SHIFT + TRACK MUTE: the pads mute the selected track's pads, on
        // the bank PAD BANK shows (page 1 is bank B: pad 5 is B5).
        assert_eq!(
            mute_key(tap(MuteKeys::Pads, 0), 4, 1, 1, None, &mut time),
            "pad mute: Track 2 B5 muted"
        );
        assert_eq!(time.pad_mutes(1), Some((1 << 20, 0)));
        assert_eq!(time.pad_mutes(0), Some((0, 0)), "track 1's B5 plays");
        assert!(pad_light(&time, 1, 20, false).silent, "its light");
        assert!(!pad_light(&time, 0, 20, false).silent);
        assert_eq!(
            mute_key(tap(MuteKeys::Pads, 0), 4, 1, 1, None, &mut time),
            "pad mute: Track 2 B5 plays"
        );
        assert_eq!(time.pad_mutes(1), Some((0, 0)));
        // TRACK MUTE held over the pads: a pad solos (at once, as the
        // transport is stopped), and again lets go of it.
        let solo = MuteTap {
            solo: true,
            ..tap(MuteKeys::Pads, 0)
        };
        assert_eq!(
            mute_key(solo, 2, 0, 7, None, &mut time),
            "pad mute: Track 1 H3 solo"
        );
        assert_eq!(time.pad_mutes(0), Some((0, 1 << 114)));
        assert!(pad_light(&time, 0, 0, false).silent, "silent by the solo");
        assert_eq!(
            mute_key(solo, 2, 0, 7, None, &mut time),
            "pad mute: Track 1 H3 solo off"
        );
        assert_eq!(time.pad_mutes(0), Some((0, 0)));
        // Any kit's pads mute (kit 2 here); a track playing notes has no
        // pads of a kit to mute, and says so.
        let mut kits = sampler::TrackKits::new(2);
        kits.set(0, 1);
        assert_eq!(no_mutes(&kits, 0, 0), None, "kit 2");
        assert!(
            no_mutes(&kits, 1, 0).is_some(),
            "its instrument plays notes"
        );
        kits.set(0, kits.synth());
        let why = no_mutes(&kits, 0, 0);
        assert!(why.is_some(), "the synth plays notes");
        assert_eq!(
            mute_key(tap(MuteKeys::Pads, 0), 4, 0, 0, why.clone(), &mut time),
            why.unwrap()
        );
        assert_eq!(time.pad_mutes(0), Some((0, 0)));

        // MAIN held: the picked track white, a track playing bright green,
        // the others dim green, no track dark.
        let lights = |t: usize| {
            (t < 6).then_some(time::MuteLight {
                launched: t == 1,
                ..time::MuteLight::default()
            })
        };
        let c = track_pick_colours(Some(2), lights);
        let green = akai::KIT_COLOURS[3];
        assert_eq!(
            (c[1], c[2], c[3], c[6]),
            (akai::hit_colour(green), step_keys::PLAYHEAD, green, [0; 3])
        );
        assert_eq!(track_pick_colours(None, lights)[2], green);

        let kit = sound_colours(std::array::from_fn(|p| p < 12), 1);
        let [amber, blue, _, green, red, ..] = akai::KIT_COLOURS;
        let track = |t: usize| {
            (t < 8).then_some(time::MuteLight {
                silent: t == 2 || t == 4,
                solo: t == 3,
                waiting: t == 5,
                launched: t == 0,
            })
        };
        let none = |_| time::MuteLight::default();
        let c = mute_colours(MuteKeys::Tracks, kit, track, none, true);
        assert_eq!(
            (c[0], c[1], c[2], c[3], c[4], c[5], c[8]),
            (
                akai::hit_colour(green),
                green,
                red,
                akai::hit_colour(blue),
                red,
                akai::hit_colour(amber),
                [0; 3]
            )
        );
        let c = mute_colours(MuteKeys::Tracks, kit, track, none, false);
        assert_eq!(c[5], amber, "waiting blinks");
        // The kit's pads: pad 5 muted, pad 2 soloed (pad 4 silent by it),
        // pad 7 waiting; pad 13 has no sound.
        let pad = |p: usize| time::MuteLight {
            silent: p == 4 || p == 3,
            solo: p == 1,
            waiting: p == 6,
            launched: false,
        };
        let c = mute_colours(MuteKeys::Pads, kit, track, pad, false);
        assert_eq!(
            (c[0], c[1], c[3], c[4], c[6], c[12]),
            (
                akai::hit_colour(kit[0]),
                akai::hit_colour(blue),
                red,
                red,
                amber,
                [0; 3]
            )
        );
        let c = mute_colours(MuteKeys::Pads, kit, track, pad, true);
        assert_eq!(c[6], akai::hit_colour(amber), "waiting blinks");
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
    fn a_contact_sheet_pairs_each_screen_with_its_base() {
        let html = contact_sheet_html("Sunrise <2>", "Synthwave", &["play", "mixer-eq"]);
        assert!(html.starts_with("<!doctype html>"));
        assert!(html.contains("<title>Sunrise &lt;2&gt; beside Synthwave</title>"));
        assert!(html.contains("Every sample screen MPFree draws (2)"));
        for name in ["play", "mixer-eq"] {
            assert!(
                html.contains(&format!("src=\"theme/{name}.png\"")),
                "{name}"
            );
            assert!(html.contains(&format!("src=\"base/{name}.png\"")), "{name}");
        }
        assert!(!html.contains("<script"));
        // The base is the file's first line, else Default.
        assert_eq!(
            base_theme("# mine\nbase = Light\naccent = #FF0000\n").0,
            "light"
        );
        assert_eq!(base_theme("accent = #FF0000\nbase = light\n").0, "default");
        assert_eq!(base_theme("base = vaporwave\n").0, "default");
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
        // Text the colour of the panels: no one could read it.
        fs::write(t.join("fog.theme"), "text = #151922\n").unwrap();
        fs::write(t.join("notes.txt"), "not a theme\n").unwrap();
        let store = mpfree_store::Store::at(&d, |_| {});
        let folder = load_theme_files(&store);
        let names: Vec<(&str, &str)> = folder
            .found
            .iter()
            .map(|(f, t)| (f.as_str(), t.name.as_str()))
            .collect();
        // Named like a built-in theme (Console, from its base), or with no
        // name: its file name. A mistake in a line doesn't leave the file
        // out: that line uses the base's value, and it is listed.
        assert_eq!(
            names,
            vec![
                ("broken.theme", "broken"),
                ("plain.theme", "plain"),
                ("sunrise.theme", "Sunrise")
            ]
        );
        assert_eq!(folder.lines_fell_back, 1);
        assert_eq!(folder.problems.len(), 1);
        assert_eq!(folder.problems[0].0, "broken.theme");
        assert!(
            folder.problems[0].1[0].starts_with("line 1: accent: expected a colour"),
            "{:?}",
            folder.problems
        );
        assert!(
            folder.notes.iter().any(|n| n.starts_with(
                "themes: broken.theme: used the base for: line 1: accent: expected a colour"
            )),
            "{:?}",
            folder.notes
        );
        // Only a theme no one could read is left out.
        assert_eq!(folder.left_out.len(), 1);
        assert_eq!(folder.left_out[0].0, "fog.theme");
        assert!(
            folder.notes.iter().any(
                |n| n.starts_with("themes: fog.theme left out: nothing could be read: text on")
            ),
            "{:?}",
            folder.notes
        );
        assert!(
            folder
                .notes
                .iter()
                .any(|n| n == "themes: found broken, plain, Sunrise"),
            "{:?}",
            folder.notes
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
        assert!(parse_args(&args("drums --seconds 5 --gain-db -1")).is_err());
        assert!(parse_args(&args("app --gain-db 1")).is_err());
        let o = parse_args(&args("click --seconds 5")).unwrap();
        assert_eq!(o.gain_db(), -12.0, "the click check stays where it was");
        let o = parse_args(&args("app")).unwrap();
        assert_eq!(o.gain_db(), 0.0, "the pad player's top is 0 dB");
        assert_eq!(
            o.gain_db() - 6.0,
            preview::LEVEL_DB,
            "and it starts at the audition's level"
        );
        let o = parse_args(&args("app --gain-db -12")).unwrap();
        assert_eq!(o.gain_db(), -12.0);
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
        // MIDI and CV out are Prefs' "Send MIDI and CV out" now, not
        // flags (Jay, 16 Nov).
        assert!(parse_args(&args("app --midi-out")).is_err());
        assert!(parse_args(&args("app --cv-out")).is_err());
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
        // The time engine's editing side on its own thread (Arrange 2.6b).
        assert!(parse_args(&args("app --time-thread")).unwrap().time_thread);
        assert!(!parse_args(&args("app")).unwrap().time_thread);
        assert!(parse_args(&args("click --seconds 5 --time-thread")).is_err());
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
                mix.period(&ctx, (&z, &z), (&z, &z), (&z, &z), &mut l, &mut r);
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

    /// A learned control on a track's synth (`Owner::Inst`) turns the
    /// synth on the pads over the knob's range, as the Synth tab does, and
    /// is named for the I/O view; a reading or a parameter the synth
    /// doesn't have is refused.
    #[test]
    fn a_learned_control_turns_a_synths_knob() {
        use mpfree_midi::learn::Write;
        use mpfree_play::mpfree_param::{Owner, ParamAddr};
        let mut mix = mixing::MixHost::new(44_100, 0.0, 6.0);
        let mut synth = synth::Synth::new(44_100);
        let app = app::App::new(44_100, false);
        let info = synth
            .param_info_list()
            .into_iter()
            .find(|p| !p.readonly && p.max > p.min)
            .expect("a knob");
        let knob = ParamAddr::new(Owner::Inst(1), info.id);
        for unit in [0.0, 1.0, 0.5] {
            let w = Write {
                target: knob,
                value: unit,
            };
            assert_eq!(
                learned_synth(
                    &mut synth,
                    &mut mix,
                    &instrument_list::InstrumentList::new(),
                    w
                ),
                Some(info.from_unit(unit))
            );
            let (got_info, got) = learn_describe(
                &mix,
                &synth,
                &app,
                &instrument_list::InstrumentList::new(),
                &|_, _| None,
                remote::Now::default(),
                knob,
            )
            .unwrap();
            assert_eq!(got_info.id, info.id);
            let want = info.from_unit(unit);
            assert!(
                (got - want).abs() <= (info.max - info.min) * 0.01,
                "{unit}: {got} != {want}"
            );
        }
        let time = time::TimeHost::new(44_100, 120.0).unwrap();
        let name = learn_label(
            &mix,
            &synth,
            &time,
            &instrument_list::InstrumentList::new(),
            knob,
        );
        assert!(name.starts_with(synth.engine_name()), "{name}");
        assert!(name.ends_with("(Track 2)"), "{name}");
        let nothing = Write {
            target: ParamAddr::new(Owner::Inst(1), 9999),
            value: 1.0,
        };
        assert_eq!(
            learned_synth(
                &mut synth,
                &mut mix,
                &instrument_list::InstrumentList::new(),
                nothing
            ),
            None
        );
        if let Some(r) = synth.param_info_list().into_iter().find(|p| p.readonly) {
            let reading = Write {
                target: ParamAddr::new(Owner::Inst(1), r.id),
                value: 1.0,
            };
            assert_eq!(
                learned_synth(
                    &mut synth,
                    &mut mix,
                    &instrument_list::InstrumentList::new(),
                    reading
                ),
                None
            );
        }
    }

    /// A learned control's moves record as a screen knob's do (the MIDI
    /// plan's 2.5): while the transport records, a value it sets on the
    /// armed track's synth becomes a lock on the note being played. The
    /// arp's settings and the transport record nothing, and nothing
    /// records while the transport only plays.
    #[test]
    fn a_learned_controls_moves_record_into_the_clip() {
        use mpfree_play::mpfree_param::{arp, transport, OnTrack, Owner, ParamAddr, RelAddr};
        use mpfree_play::Event;
        struct Quiet;
        impl time::Pads for Quiet {
            fn hit_at(&mut self, _: u16, _: usize, _: u8, _: u32) -> bool {
                true
            }
            fn release_at(&mut self, _: u16, _: usize, _: u32) {}
            fn stop_all(&mut self) {}
            fn pressure_at(&mut self, _: u16, _: usize, _: u8, _: u32) {}
        }
        let run = |h: &mut time::TimeHost, periods: usize| {
            for _ in 0..periods {
                h.period(PERIOD as u32, &mut Quiet);
            }
        };
        let locks = |h: &time::TimeHost| -> Vec<(RelAddr, f32)> {
            h.project().tracks[2]
                .slots
                .clips()
                .flat_map(|c| c.locks.iter().map(|l| (l.addr, l.value)))
                .collect()
        };
        let mut h = time::TimeHost::new(44_100, 120.0).unwrap();
        h.act(TimeAction::NewClip { track: 2, slot: 0 }).unwrap();
        run(&mut h, 12);
        assert_eq!(h.info().state, mpfree_transport::State::Recording);
        h.pad(
            0,
            Event::NoteOn {
                note: 4,
                velocity: 100,
            },
        );
        run(&mut h, 2);
        let cutoff = RelAddr::new(OnTrack::Inst, 1);
        record_learned(&mut h, cutoff.on(2), 0.25, true);
        record_learned(&mut h, cutoff.on(2), 0.6, true);
        // The arp and the transport have no locks.
        record_learned(&mut h, ParamAddr::new(Owner::Arp(2), arp::GATE), 30.0, true);
        record_learned(
            &mut h,
            ParamAddr::new(Owner::Transport, transport::TEMPO),
            90.0,
            true,
        );
        // Not recording: only a turn.
        record_learned(&mut h, RelAddr::new(OnTrack::Inst, 2).on(2), 0.5, false);
        run(&mut h, 2);
        h.pad(0, Event::NoteOff { note: 4 });
        // Past the bar, so the lock's step has gone by.
        run(&mut h, 2 * 88_200 / PERIOD);
        assert_eq!(locks(&h), vec![(cutoff, 0.6)]);
    }

    /// Sampler refactor step 17b: a learned move on a layer's grain
    /// setting records as a lock on the kit pad's note, as a pad's own
    /// setting does, and a clip plays it through the kit (`lock_at`,
    /// [`sampler::Player::pad_param_lock`]).
    #[test]
    fn a_learned_layer_grain_setting_records_as_a_lock() {
        use mpfree_play::mpfree_param::{pad, OnTrack, RelAddr};
        use mpfree_play::Event;
        use mpfree_sampler::grain_field::GrainField;
        struct Quiet;
        impl time::Pads for Quiet {
            fn hit_at(&mut self, _: u16, _: usize, _: u8, _: u32) -> bool {
                true
            }
            fn release_at(&mut self, _: u16, _: usize, _: u32) {}
            fn stop_all(&mut self) {}
            fn pressure_at(&mut self, _: u16, _: usize, _: u8, _: u32) {}
        }
        let run = |h: &mut time::TimeHost, periods: usize| {
            for _ in 0..periods {
                h.period(PERIOD as u32, &mut Quiet);
            }
        };
        let mut h = time::TimeHost::new(44_100, 120.0).unwrap();
        h.act(TimeAction::NewClip { track: 2, slot: 0 }).unwrap();
        run(&mut h, 12);
        assert_eq!(h.info().state, mpfree_transport::State::Recording);
        h.pad(
            0,
            Event::NoteOn {
                note: 4,
                velocity: 100,
            },
        );
        run(&mut h, 2);
        let size = pad::layer(1, GrainField::Size as u8).unwrap();
        let at = RelAddr::new(OnTrack::Pad(4), size);
        record_learned(&mut h, at.on(2), 320.0, true);
        run(&mut h, 2);
        h.pad(0, Event::NoteOff { note: 4 });
        run(&mut h, 2 * 88_200 / PERIOD);
        let locks: Vec<(RelAddr, f32)> = h.project().tracks[2]
            .slots
            .clips()
            .flat_map(|c| c.locks.iter().map(|l| (l.addr, l.value)))
            .collect();
        assert_eq!(locks, vec![(at, 320.0)]);
    }

    /// The Clips grid's learned controls (sequencer S5) are named by
    /// where they are, and a track's mute and solo read from the bits the
    /// time engine gives.
    #[test]
    fn the_clips_grid_is_named_and_read_for_learn() {
        use mpfree_play::mpfree_param::{clips, Owner, ParamAddr};
        let mix = mixing::MixHost::new(44_100, 0.0, 6.0);
        let synth = synth::Synth::new(44_100);
        let app = app::App::new(44_100, false);
        let mut time = time::TimeHost::new(44_100, 120.0).unwrap();
        let at = |id| ParamAddr::new(Owner::Clips, id);
        let name = |time: &time::TimeHost, id| {
            learn_label(
                &mix,
                &synth,
                time,
                &instrument_list::InstrumentList::new(),
                at(id),
            )
        };
        let second = time.project().tracks[1].name.clone();
        assert_eq!(
            name(&time, clips::clip(1, 2)),
            format!("Launch {second} · row 3")
        );
        assert_eq!(name(&time, clips::mute(1)), format!("Mute {second}"));
        assert_eq!(name(&time, clips::solo(1)), format!("Solo {second}"));
        assert_eq!(name(&time, clips::arm(1)), format!("Arm {second}"));
        assert_eq!(name(&time, clips::stop(1)), format!("Stop {second}"));
        assert_eq!(
            name(&time, clips::stop(127)),
            "Stop Track 128",
            "past the tracks"
        );
        assert_eq!(name(&time, clips::STOP_ALL), "Stop all");
        assert_eq!(name(&time, clips::BACK_TO_SONG), "Back to song");
        assert_eq!(name(&time, clips::CAPTURE_SCENE), "Capture scene");
        assert_eq!(name(&time, clips::scene(2)), "Scene 3");
        time.act(TimeAction::SceneSettings {
            row: 2,
            card_row: mpfree_time_ui::view::SCENE_NAME,
            settings: Box::new(mpfree_seq::scenes::SceneSettings {
                name: "Verse".into(),
                ..Default::default()
            }),
        })
        .unwrap();
        assert_eq!(name(&time, clips::scene(2)), "Scene 3 · Verse");
        assert_eq!(name(&time, 9), "?");
        // Track 2 muted at once (stopped): read as on.
        time.mute_or_solo(1, false, false).unwrap();
        let (muted, soloed) = time.mute_bits();
        let now = remote::Now {
            muted,
            soloed,
            ..remote::Now::default()
        };
        let (info, v) = learn_describe(
            &mix,
            &synth,
            &app,
            &instrument_list::InstrumentList::new(),
            &|_, _| None,
            now,
            at(clips::mute(1)),
        )
        .unwrap();
        assert_eq!(
            (info.name, info.curve, v),
            ("Mute", mpfree_node::Curve::Toggle, 1.0)
        );
        assert_eq!(
            learn_describe(
                &mix,
                &synth,
                &app,
                &instrument_list::InstrumentList::new(),
                &|_, _| None,
                now,
                at(clips::solo(1))
            )
            .unwrap()
            .1,
            0.0
        );
        assert!(learn_describe(
            &mix,
            &synth,
            &app,
            &instrument_list::InstrumentList::new(),
            &|_, _| None,
            now,
            at(clips::clip(1, 2))
        )
        .is_none());
        assert!(midi_in::key_toggles(at(clips::mute(1))));
        assert!(!midi_in::key_toggles(at(clips::clip(1, 2))));
    }

    /// Play's learned controls (the Learn section's Play card) are named
    /// "Play · Swing" and read where Play has them; a key on one holds it
    /// while held, and they are never switches.
    #[test]
    fn plays_controls_are_named_and_read_for_learn() {
        use mpfree_play::mpfree_param::{play, Owner, ParamAddr};
        let mix = mixing::MixHost::new(44_100, 0.0, 6.0);
        let synth = synth::Synth::new(44_100);
        let time = time::TimeHost::new(44_100, 120.0).unwrap();
        let app = app::App::new(44_100, false);
        let at = |id| ParamAddr::new(Owner::Play, id);
        let name = |id| {
            learn_label(
                &mix,
                &synth,
                &time,
                &instrument_list::InstrumentList::new(),
                at(id),
            )
        };
        assert_eq!(name(play::SWING), "Play · Swing");
        assert_eq!(name(play::FILTER), "Play · Filter");
        assert_eq!(name(play::XY_ACROSS), "Play · XY across");
        assert_eq!(name(play::XY_UP), "Play · XY up");
        assert_eq!(name(play::NEXT_PATCH), "Play · Next patch");
        assert_eq!(name(play::PREVIOUS_PATCH_HANDS_UP), "Play · Previous patch");
        assert_eq!(name(99), "Play · ?");
        let now = desk::play_now(&time, &mix);
        assert_eq!((now.swing, now.drums_filter, now.xy), (50, 0.0, (0.5, 0.0)));
        let read = |id| {
            learn_describe(
                &mix,
                &synth,
                &app,
                &instrument_list::InstrumentList::new(),
                &|_, _| None,
                now,
                at(id),
            )
        };
        let (info, v) = read(play::SWING).unwrap();
        assert_eq!(
            (info.name, info.min, info.max, v),
            ("Swing", 50.0, 75.0, 50.0)
        );
        assert_eq!(read(play::XY_ACROSS).unwrap().1, 0.5);
        assert!(read(99).is_none());
        assert!(!midi_in::key_toggles(at(play::XY_UP)));
    }

    /// A key learned on a Perform pad (Learn a key) is named by the pad,
    /// has no value to read, and plays while held whatever sends it: a
    /// key, a footswitch's CC or a pad.
    #[test]
    fn a_perform_pads_learned_key_is_named_by_its_pad_and_plays_while_held() {
        use mpfree_midi::learn::{Mode, Source};
        use mpfree_play::mpfree_param::{perform, Owner, ParamAddr};
        let mix = mixing::MixHost::new(44_100, 0.0, 6.0);
        let synth = synth::Synth::new(44_100);
        let time = time::TimeHost::new(44_100, 120.0).unwrap();
        let app = app::App::new(44_100, false);
        let b2 = ParamAddr::new(Owner::Perform, perform::pad(1, 1));
        assert_eq!(
            learn_label(
                &mix,
                &synth,
                &time,
                &instrument_list::InstrumentList::new(),
                b2
            ),
            "Perform pads B2"
        );
        assert_eq!(
            learn_label(
                &mix,
                &synth,
                &time,
                &instrument_list::InstrumentList::new(),
                ParamAddr::new(Owner::Perform, 200)
            ),
            "Perform pads ?"
        );
        assert!(learn_describe(
            &mix,
            &synth,
            &app,
            &instrument_list::InstrumentList::new(),
            &|_, _| None,
            remote::Now::default(),
            b2
        )
        .is_none());
        for source in [Source::Note(40), Source::Cc(64)] {
            assert_eq!(midi_in::learned_mode(&source, b2), Some(Mode::Momentary));
        }
        assert!(!midi_in::key_toggles(b2));
    }

    /// Starts: When I play on a free loop (Sync: Free), with the song and
    /// the free loops stopped: the pad's note starts the loop on the
    /// stopped free clock, and never the song.
    #[test]
    fn a_pad_starts_a_free_loop_and_never_the_song() {
        use mpfree_looper::{Clock, Command as L, Phase, Starts};
        let mut looper = looping::LoopHost::new(RATE, 8);
        let mut time = time::TimeHost::new(RATE, 120.0).unwrap();
        let mut t = mpfree_transport::Transport::new(RATE, 120.0).unwrap();
        let mut b = mpfree_transport::Block::new();
        let mut run = |looper: &mut looping::LoopHost, periods: usize, sound: bool| {
            for k in 0..periods {
                assert!(t.process(PERIOD as u32, &mut b));
                let x = if sound {
                    0.2 + 0.1 * (k % 3) as f32
                } else {
                    0.0
                };
                looper.take_pads(&[x; PERIOD], &[-x; PERIOD]);
                let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
                looper.period(&b, &mut l, &mut r);
            }
        };
        // Loop 1, free in your own time: about two seconds, closed by REC.
        looper.command(L::SetClock(Clock::Free));
        run(&mut looper, 1, false);
        looper.command(L::Record);
        run(&mut looper, 700, true);
        looper.command(L::Record);
        run(&mut looper, 4, true);
        let s = looper.loop_status(0).unwrap();
        assert_eq!((s.phase, s.clock), (Phase::Playing, Clock::Free));
        // The free loops stop, at their tops.
        looper.command(L::FreeStop);
        run(&mut looper, 2, false);
        // Loop 2 takes Sync: Free, and starts when you play.
        looper.select(1);
        looper.command(L::SetStarts(Starts::Play));
        run(&mut looper, 1, false);
        looper.command(L::Record);
        run(&mut looper, 1, false);
        let s = looper.loop_status(1).unwrap();
        assert!(s.listening && s.clock == Clock::Free && !s.free, "{s:?}");
        let (line, refused) =
            pad_starts_loop(&mut looper, &mut time, None, false).expect("it starts");
        assert!(!refused, "{line}");
        assert!(line.contains("on the free loops' clock"), "{line}");
        assert!(!line.contains("the song with it"), "{line}");
        run(&mut looper, 2, true);
        assert_eq!(looper.loop_status(1).unwrap().phase, Phase::Recording);
        // The song's transport, a period on: still stopped.
        assert!(time.advance(PERIOD as u32));
        assert_eq!(time.info().state, mpfree_transport::State::Stopped);
    }

    /// The looper's learned controls (its footswitch) are named so the
    /// picked loop's never looks like a loop's number, and a loop's mute
    /// reads from the loops' bits.
    #[test]
    fn the_loopers_footswitch_is_named_and_read_for_learn() {
        use mpfree_play::mpfree_param::{looper, Owner, ParamAddr};
        let mix = mixing::MixHost::new(44_100, 0.0, 6.0);
        let synth = synth::Synth::new(44_100);
        let time = time::TimeHost::new(44_100, 120.0).unwrap();
        let app = app::App::new(44_100, false);
        let at = |id| ParamAddr::new(Owner::Looper, id);
        let name = |id| {
            learn_label(
                &mix,
                &synth,
                &time,
                &instrument_list::InstrumentList::new(),
                at(id),
            )
        };
        assert_eq!(name(looper::key(2)), "Looper · Loop 3 key");
        assert_eq!(name(looper::PICKED_KEY), "Looper · picked loop's key");
        assert_eq!(name(looper::mute(0)), "Looper · Loop 1 mute");
        assert_eq!(name(looper::PICKED_MUTE), "Looper · picked loop's mute");
        assert_eq!(name(looper::NEXT_LOOP), "Looper · Next loop");
        assert_eq!(name(looper::CLEAR), "Looper · Clear");
        assert_eq!(name(looper::SNAP), "Looper · Snap");
        assert_eq!(name(looper::USE_MY_TAP), "Looper · Use my tap");
        assert_eq!(name(99), "Looper · ?");
        let now = remote::Now {
            picked: 2,
            loops_muted: 1 << 2,
            ..remote::Now::default()
        };
        let (info, v) = learn_describe(
            &mix,
            &synth,
            &app,
            &instrument_list::InstrumentList::new(),
            &|_, _| None,
            now,
            at(looper::mute(2)),
        )
        .unwrap();
        assert_eq!(
            (info.name, info.curve, v),
            ("Mute", mpfree_node::Curve::Toggle, 1.0)
        );
        assert_eq!(
            learn_describe(
                &mix,
                &synth,
                &app,
                &instrument_list::InstrumentList::new(),
                &|_, _| None,
                now,
                at(looper::PICKED_MUTE)
            )
            .unwrap()
            .1,
            1.0
        );
        assert_eq!(
            learn_describe(
                &mix,
                &synth,
                &app,
                &instrument_list::InstrumentList::new(),
                &|_, _| None,
                now,
                at(looper::mute(0))
            )
            .unwrap()
            .1,
            0.0
        );
        assert!(learn_describe(
            &mix,
            &synth,
            &app,
            &instrument_list::InstrumentList::new(),
            &|_, _| None,
            now,
            at(looper::key(2))
        )
        .is_none());
        assert!(midi_in::key_toggles(at(looper::mute(2))));
        assert!(!midi_in::key_toggles(at(looper::key(2))));
        // A footswitch's CC on a press works while held; on a mute, as a
        // knob or fader (on from the middle up); a key on a mute flips it.
        use mpfree_midi::learn::{Mode, Source};
        let cc = Source::Cc(64);
        let key = Source::Note(40);
        assert_eq!(
            midi_in::learned_mode(&cc, at(looper::key(2))),
            Some(Mode::Momentary)
        );
        assert_eq!(
            midi_in::learned_mode(&cc, at(looper::UNDO)),
            Some(Mode::Momentary)
        );
        assert_eq!(midi_in::learned_mode(&cc, at(looper::mute(2))), None);
        assert_eq!(
            midi_in::learned_mode(&key, at(looper::mute(2))),
            Some(Mode::Toggle)
        );
        assert_eq!(
            midi_in::learned_mode(&key, at(looper::PICKED_KEY)),
            Some(Mode::Momentary)
        );
        let tempo = ParamAddr::new(
            Owner::Transport,
            mpfree_play::mpfree_param::transport::TEMPO,
        );
        assert_eq!(midi_in::learned_mode(&cc, tempo), None);
    }

    /// The transport's learned controls are named for the I/O view and
    /// read from where the transport stands; a mute reads from the mixer.
    /// R3 part 2b: the 16 pads show the kit's page; PAD BANK says which
    /// pads it put there and how many sounds they hold.
    #[test]
    fn shift_wheel_passes_the_empty_user_kits_until_one_is_in_use() {
        // Kits 0-2, user kits 3-5, the synth 6.
        let users = 3..6;
        let mut kits = sampler::TrackKits::new(6);
        let none = |_| 0u128;
        assert_eq!(
            kit_turned(2, 1, 7, &users, none, &kits),
            6,
            "on to the synth"
        );
        assert_eq!(kit_turned(6, -1, 7, &users, none, &kits), 2, "and back");
        assert_eq!(kit_turned(6, 1, 7, &users, none, &kits), 0, "round");
        // A user kit with sounds, or one a track plays, is a stop.
        let b1 = |k| if k == 4 { 1u128 << 16 } else { 0 };
        assert_eq!(kit_turned(2, 1, 7, &users, b1, &kits), 4);
        assert!(kits.set(3, 3));
        assert_eq!(kit_turned(2, 1, 7, &users, none, &kits), 3);
        assert_eq!(
            kit_turned(2, 3, 7, &users, b1, &kits),
            6,
            "3, 4, then past 5 to the synth"
        );
        // A kit set aside with a track UNDO took away is no stop, even
        // with sounds, until REDO brings the track back.
        assert!(kits.follow(3, false, true));
        let b0 = |k| if k == 3 { 1u128 } else { 0 };
        assert_eq!(kit_turned(2, 1, 7, &users, b0, &kits), 6);
        assert!(kits.follow(4, true, true));
        assert_eq!(kit_turned(2, 1, 7, &users, b0, &kits), 3);
    }

    #[test]
    fn the_page_shows_its_own_pads_and_says_what_it_holds() {
        assert_eq!(pad_cell(20, 1, true), Some(4));
        assert_eq!(pad_cell(20, 0, true), None, "B5 isn't on bank A");
        assert_eq!(pad_cell(3, 1, true), None);
        assert_eq!(pad_cell(127, 7, true), Some(15));
        assert_eq!(
            pad_cell(3, 1, false),
            Some(3),
            "the synth's pads are its 16"
        );
        assert_eq!(pad_cell(16, 0, false), None);
        assert_eq!(
            pad_page_note(1, "Drums", 0xFFFF),
            "pad bank B: pads B1-B16 of Drums (no sounds yet)"
        );
        assert_eq!(
            pad_page_note(0, "Drums", 0xFFFF),
            "pad bank A: pads A1-A16 of Drums (16 sounds)"
        );
        assert_eq!(
            pad_page_note(7, "Chops", 1 << 127),
            "pad bank H: pads H1-H16 of Chops (1 sound)"
        );
    }

    /// A track's arp settings are named by the project's track and read
    /// from the app, the armed track's as its panel holds them.
    #[test]
    fn a_tracks_arp_is_named_and_read_for_learn() {
        use mpfree_play::mpfree_param::{arp, Owner, ParamAddr};
        let mix = mixing::MixHost::new(44_100, 0.0, 6.0);
        let synth = synth::Synth::new(44_100);
        let mut app = app::App::new(44_100, false);
        let time = time::TimeHost::new(44_100, 120.0).unwrap();
        let second = time.project().tracks[1].name.clone();
        let at = |t, id| ParamAddr::new(Owner::Arp(t), id);
        assert_eq!(
            learn_label(
                &mix,
                &synth,
                &time,
                &instrument_list::InstrumentList::new(),
                at(1, arp::GATE)
            ),
            format!("Arp gate ({second})")
        );
        assert_eq!(
            learn_label(
                &mix,
                &synth,
                &time,
                &instrument_list::InstrumentList::new(),
                at(127, arp::RATE)
            ),
            "Arp rate (Track 128)"
        );
        app.set_arp_param(0, arp::SWING, 66.0);
        let now = remote::Now::default();
        let (info, v) = learn_describe(
            &mix,
            &synth,
            &app,
            &instrument_list::InstrumentList::new(),
            &|_, _| None,
            now,
            at(0, arp::SWING),
        )
        .unwrap();
        assert_eq!((info.name, v), ("Swing", 66.0));
        assert_eq!(mpfree_mod::text::value_text(&info, v), "66 %");
        assert!(learn_describe(
            &mix,
            &synth,
            &app,
            &instrument_list::InstrumentList::new(),
            &|_, _| None,
            now,
            at(0, 7)
        )
        .is_none());
    }

    /// Sampler refactor step 17: a layer's grain setting is a pad's
    /// parameter from 256 (`mpfree_param::pad::layer`), named with its
    /// layer and read from the kit's view as edited. A layer the pad
    /// hasn't got reads nothing, and a move on one records no lock yet.
    #[test]
    fn a_layers_grain_setting_is_named_and_read_for_learn() {
        use mpfree_play::mpfree_param::{pad, Owner, ParamAddr};
        use mpfree_sampler::grain_field::GrainField;
        let mix = mixing::MixHost::new(44_100, 0.0, 6.0);
        let synth = synth::Synth::new(44_100);
        let app = app::App::new(44_100, false);
        let time = time::TimeHost::new(44_100, 120.0).unwrap();
        let tone: Vec<i16> = (0..2_000)
            .flat_map(|i| [((i as f32 * 0.03).sin() * 12_000.0) as i16; 2])
            .collect();
        let kick = kit::build(&[(mpc_one::PAD_NOTES[4], "Kick", &tone[..])]);
        let mut ks = kit::parse_kits(&kit::build_kits(&[("House", kick)])).unwrap();
        let player = sampler::Player::new(&mut ks, &mpc_one::PAD_NOTES, 44_100).unwrap();
        let kits = sampler::TrackKits::new(1);
        let listed = instrument_list::InstrumentList::new();
        let grain = |layer, f: GrainField| pad::layer(layer, f as u8).unwrap();
        let at = |layer, f| ParamAddr::new(Owner::Pad(0, 4), grain(layer, f));
        assert_eq!(
            sampler::PadParam::of(grain(0, GrainField::Size)),
            Some(sampler::PadParam::Grain {
                layer: 0,
                field: GrainField::Size
            })
        );
        assert_eq!(
            sampler::PadParam::of(pad::LAYER_BASE + 63),
            None,
            "no such setting"
        );
        let first = time.project().tracks[0].name.clone();
        assert_eq!(
            learn_label(&mix, &synth, &time, &listed, at(0, GrainField::Size)),
            format!("Grain size (A5 layer 1, {first})")
        );
        let pads = |owner, id| {
            let (kit, pad) = learned_kit(&listed, &kits, owner)?;
            pad_describe(&player, kit, pad, id)
        };
        let read = |addr| {
            learn_describe(
                &mix,
                &synth,
                &app,
                &listed,
                &pads,
                remote::Now::default(),
                addr,
            )
        };
        let (info, ms) = read(at(0, GrainField::Size)).unwrap();
        assert_eq!((info.name, info.min, info.max), ("Grain size", 2.0, 2000.0));
        assert_eq!(info.id, grain(0, GrainField::Size));
        assert_eq!(ms, 80.0, "the layer's own, as edited");
        let (info, db) = read(at(0, GrainField::Level)).unwrap();
        assert_eq!((info.name, db), ("Grain level", 0.0));
        assert!(read(at(1, GrainField::Size)).is_none(), "one layer");
        assert!(
            read(ParamAddr::new(Owner::Pad(0, 5), grain(0, GrainField::Size))).is_none(),
            "no sound"
        );
        // While recording, a pad's own setting goes to the recorder as a
        // lock would, and so does a layer's grain setting (step 17b); a
        // number that names no setting doesn't.
        assert!(learn_records(at(0, GrainField::Size)));
        assert!(!learn_records(ParamAddr::new(
            Owner::Pad(0, 4),
            pad::LAYER_BASE + 63
        )));
        let own = ParamAddr::new(
            Owner::Pad(0, 4),
            mpfree_sampler::engine::PadField::Tune as u16,
        );
        assert!(learn_records(own));
    }

    /// A kit pad's own setting is named by its pad and the project's
    /// track, and read from the kit the track plays: None for a pad
    /// without a sound, a track on the synth, or a number past the
    /// settings.
    #[test]
    fn a_kit_pads_setting_is_named_and_read_for_learn() {
        use mpfree_play::mpfree_param::{Owner, ParamAddr};
        use mpfree_sampler::engine::PadField;
        let mix = mixing::MixHost::new(44_100, 0.0, 6.0);
        let synth = synth::Synth::new(44_100);
        let app = app::App::new(44_100, false);
        let time = time::TimeHost::new(44_100, 120.0).unwrap();
        let tone: Vec<i16> = (0..2_000)
            .flat_map(|i| [((i as f32 * 0.03).sin() * 12_000.0) as i16; 2])
            .collect();
        let kick = kit::build(&[(mpc_one::PAD_NOTES[4], "Kick", &tone[..])]);
        let mut ks = kit::parse_kits(&kit::build_kits(&[("House", kick)])).unwrap();
        let player = sampler::Player::new(&mut ks, &mpc_one::PAD_NOTES, 44_100).unwrap();
        let mut kits = sampler::TrackKits::new(1);
        let at = |t, pad, f: PadField| ParamAddr::new(Owner::Pad(t, pad), f as u16);
        let first = time.project().tracks[0].name.clone();
        assert_eq!(
            learn_label(
                &mix,
                &synth,
                &time,
                &instrument_list::InstrumentList::new(),
                at(0, 4, PadField::Tune)
            ),
            format!("Tune (A5, {first})")
        );
        let pads = |owner, id| {
            let (kit, pad) = learned_kit(&instrument_list::InstrumentList::new(), &kits, owner)?;
            pad_describe(&player, kit, pad, id)
        };
        let now = remote::Now::default();
        let read = |addr| {
            learn_describe(
                &mix,
                &synth,
                &app,
                &instrument_list::InstrumentList::new(),
                &pads,
                now,
                addr,
            )
        };
        let (info, db) = read(at(0, 4, PadField::Level)).unwrap();
        assert_eq!((info.name, info.min, info.max), ("Level", -48.0, 12.0));
        assert_eq!(db, player.pad_own(0, 4, PadField::Level).unwrap());
        // No filter on the kit's pads: the cutoff reads open.
        let (info, hz) = read(at(0, 4, PadField::Cutoff)).unwrap();
        assert_eq!((info.name, hz), ("Cutoff", info.max));
        let (info, slice) = read(at(0, 4, PadField::Slice)).unwrap();
        assert_eq!((info.max, slice), (0.0, 0.0), "one slice");
        assert!(read(at(0, 5, PadField::Level)).is_none(), "no sound");
        assert!(read(ParamAddr::new(Owner::Pad(0, 4), 99)).is_none());
        kits.set(0, kits.synth());
        let pads = |owner, id| {
            let (kit, pad) = learned_kit(&instrument_list::InstrumentList::new(), &kits, owner)?;
            pad_describe(&player, kit, pad, id)
        };
        assert!(
            learn_describe(
                &mix,
                &synth,
                &app,
                &instrument_list::InstrumentList::new(),
                &pads,
                now,
                at(0, 4, PadField::Level)
            )
            .is_none(),
            "the track plays the synth"
        );
    }

    /// Sampler step 16: a kit pad's setting named by its kit's number in
    /// the list (`Owner::KitPad`) reads that kit whichever kit the track
    /// plays, and is named for the kit; a number the list hasn't got reads
    /// nothing. REC turns it into the armed track's pad only while the
    /// armed track plays that kit, and a synth's knob named by the shared
    /// synth into the armed track's while it plays the synth.
    #[test]
    fn a_kit_pads_setting_named_by_its_kit_stays_on_that_kit() {
        use mpfree_play::mpfree_param::{Owner, ParamAddr};
        use mpfree_sampler::engine::PadField;
        let mix = mixing::MixHost::new(44_100, 0.0, 6.0);
        let synth = synth::Synth::new(44_100);
        let app = app::App::new(44_100, false);
        let time = time::TimeHost::new(44_100, 120.0).unwrap();
        let tone: Vec<i16> = (0..2_000)
            .flat_map(|i| [((i as f32 * 0.03).sin() * 12_000.0) as i16; 2])
            .collect();
        // Kit 1 fills A5, kit 2 fills A6 only.
        let kick = kit::build(&[(mpc_one::PAD_NOTES[4], "Kick", &tone[..])]);
        let snare = kit::build(&[(mpc_one::PAD_NOTES[5], "Snare", &tone[..])]);
        let mut ks =
            kit::parse_kits(&kit::build_kits(&[("House", kick), ("Tones", snare)])).unwrap();
        let player = sampler::Player::new(&mut ks, &mpc_one::PAD_NOTES, 44_100).unwrap();
        let mut kits = sampler::TrackKits::new(2);
        let mut listed = instrument_list::InstrumentList::new();
        listed.follow_kits(2);
        let n = |k| u16::try_from(listed.of_kit(k).unwrap().number()).unwrap();
        let (kit_1, kit_2) = (n(0), n(1));
        let at = |k, pad, f: PadField| ParamAddr::new(Owner::KitPad(k, pad), f as u16);
        assert_eq!(
            learn_label(&mix, &synth, &time, &listed, at(kit_2, 5, PadField::Tune)),
            "Tune (A6, Kit 2)"
        );
        assert_eq!(
            learn_label(&mix, &synth, &time, &listed, at(99, 5, PadField::Tune)),
            "Tune (A6, instrument 99, gone)"
        );
        let pads = |owner, id| {
            let (kit, pad) = learned_kit(&listed, &kits, owner)?;
            pad_describe(&player, kit, pad, id)
        };
        let now = remote::Now::default();
        let read = |addr| learn_describe(&mix, &synth, &app, &listed, &pads, now, addr);
        // Track 1 plays kit 1, and kit 2's A6 still reads.
        assert_eq!(kits.kit_of(0), Some(0));
        let (info, db) = read(at(kit_2, 5, PadField::Level)).unwrap();
        assert_eq!(info.name, "Level");
        assert_eq!(db, player.pad_own(1, 5, PadField::Level).unwrap());
        assert!(
            read(at(kit_2, 4, PadField::Level)).is_none(),
            "kit 2's A5 is empty"
        );
        assert!(read(at(kit_1, 4, PadField::Level)).is_some());
        assert!(
            read(at(99, 4, PadField::Level)).is_none(),
            "no such instrument"
        );
        // REC: the armed track's pad while it plays the kit, else as it is.
        let rec = |addr, armed| learned_on_armed(addr, armed, &kits, &mix.instruments, &listed);
        let a5 = at(kit_1, 4, PadField::Tune);
        assert_eq!(
            rec(a5, Some(0)),
            ParamAddr::new(Owner::Pad(0, 4), PadField::Tune as u16)
        );
        assert_eq!(
            rec(at(kit_2, 5, PadField::Tune), Some(0)).owner,
            Owner::KitPad(kit_2, 5)
        );
        assert_eq!(rec(a5, None), a5, "nothing armed");
        let knob = ParamAddr::new(Owner::Sound(1), 5);
        assert_eq!(rec(knob, Some(0)), knob, "track 1 plays a kit");
        kits.set(0, kits.synth());
        let rec = |addr, armed| learned_on_armed(addr, armed, &kits, &mix.instruments, &listed);
        assert_eq!(rec(knob, Some(0)), ParamAddr::new(Owner::Inst(0), 5));
        assert_eq!(rec(a5, Some(0)), a5, "track 1 plays the synth now");
    }

    /// Sampler step 16: a synth's knob named by its instrument
    /// (`Owner::Sound`) turns that synth, the shared one here, and is
    /// named for the strip it sounds on; a number the list hasn't got, or
    /// a kit's, turns nothing.
    #[test]
    fn a_synths_knob_named_by_its_instrument_turns_that_synth() {
        use mpfree_midi::learn::Write;
        use mpfree_play::mpfree_param::{Owner, ParamAddr};
        let mut mix = mixing::MixHost::new(44_100, 0.0, 6.0);
        let mut synth = synth::Synth::new(44_100);
        let app = app::App::new(44_100, false);
        let time = time::TimeHost::new(44_100, 120.0).unwrap();
        let mut listed = instrument_list::InstrumentList::new();
        listed.follow_kits(1);
        let info = synth
            .param_info_list()
            .into_iter()
            .find(|p| !p.readonly && p.max > p.min)
            .expect("a knob");
        let knob = ParamAddr::new(Owner::Sound(1), info.id);
        let w = Write {
            target: knob,
            value: 1.0,
        };
        assert_eq!(
            learned_synth(&mut synth, &mut mix, &listed, w),
            Some(info.from_unit(1.0))
        );
        let (got, v) = learn_describe(
            &mix,
            &synth,
            &app,
            &listed,
            &|_, _| None,
            remote::Now::default(),
            knob,
        )
        .unwrap();
        assert_eq!(got.id, info.id);
        assert!((v - info.from_unit(1.0)).abs() <= (info.max - info.min) * 0.01);
        let name = learn_label(&mix, &synth, &time, &listed, knob);
        assert!(name.starts_with(synth.engine_name()), "{name}");
        assert!(name.ends_with("(Synth)"), "{name}");
        let kit_1 = u16::try_from(listed.of_kit(0).unwrap().number()).unwrap();
        for gone in [99, kit_1] {
            let w = Write {
                target: ParamAddr::new(Owner::Sound(gone), info.id),
                value: 0.0,
            };
            assert_eq!(
                learned_synth(&mut synth, &mut mix, &listed, w),
                None,
                "{gone}"
            );
            assert!(learn_label(&mix, &synth, &time, &listed, w.target).ends_with("(gone)"));
        }
    }

    #[test]
    fn the_transport_and_a_mute_are_named_and_read_for_learn() {
        use mpfree_play::mpfree_param::{transport, Owner, ParamAddr, Strip};
        let mix = mixing::MixHost::new(44_100, 0.0, 6.0);
        let synth = synth::Synth::new(44_100);
        let app = app::App::new(44_100, false);
        let time = time::TimeHost::new(44_100, 120.0).unwrap();
        let now = remote::Now {
            rolling: true,
            bpm: 98.5,
            ..remote::Now::default()
        };
        let at = |id| ParamAddr::new(Owner::Transport, id);
        assert_eq!(
            learn_label(
                &mix,
                &synth,
                &time,
                &instrument_list::InstrumentList::new(),
                at(transport::PLAYING)
            ),
            "PLAY and STOP"
        );
        assert_eq!(
            learn_label(
                &mix,
                &synth,
                &time,
                &instrument_list::InstrumentList::new(),
                at(transport::TAP_TEMPO)
            ),
            "TAP TEMPO"
        );
        let (info, bpm) = learn_describe(
            &mix,
            &synth,
            &app,
            &instrument_list::InstrumentList::new(),
            &|_, _| None,
            now,
            at(transport::TEMPO),
        )
        .unwrap();
        assert_eq!((info.name, bpm), ("Tempo", 98.5));
        let (_, on) = learn_describe(
            &mix,
            &synth,
            &app,
            &instrument_list::InstrumentList::new(),
            &|_, _| None,
            now,
            at(transport::PLAYING),
        )
        .unwrap();
        assert_eq!(on, 1.0);
        assert!(learn_describe(
            &mix,
            &synth,
            &app,
            &instrument_list::InstrumentList::new(),
            &|_, _| None,
            now,
            at(transport::REC)
        )
        .is_none());
        let mute = ParamAddr::new(Owner::Strip(Strip::Track(0)), mpfree_fx::ids::mixer::MUTE);
        let (info, v) = learn_describe(
            &mix,
            &synth,
            &app,
            &instrument_list::InstrumentList::new(),
            &|_, _| None,
            now,
            mute,
        )
        .unwrap();
        assert_eq!((info.curve, v), (mpfree_node::Curve::Toggle, 0.0));
    }

    #[test]
    fn each_instrument_plays_its_own_tuning_or_the_songs() {
        let tone: Vec<i16> = (0..2_000)
            .flat_map(|i| [((i as f32 * 0.03).sin() * 12_000.0) as i16; 2])
            .collect();
        let kick = kit::build(&[(mpc_one::PAD_NOTES[4], "Kick", &tone[..])]);
        let mut ks = kit::parse_kits(&kit::build_kits(&[("House", kick)])).unwrap();
        let mut player = sampler::Player::new(&mut ks, &mpc_one::PAD_NOTES, RATE).unwrap();
        let synth = synth::Synth::new(RATE);
        let mut mix = mixing::MixHost::new(RATE, 0.0, 6.0);
        let put = |mix: &mut mixing::MixHost, track: usize| {
            let (id, extras, values) = synth.sound_of(synth::Engine::Poly);
            let made = instruments::make(
                &plugins::Plugins::none(),
                id,
                &extras,
                &values,
                RATE,
                PERIOD,
            )
            .unwrap();
            let pads = synth.pads_for(synth::Engine::Poly);
            let plain = mpfree_tuning::Tuning::standard();
            assert!(mix.instruments.put(track, made, pads, &plain).is_ok());
        };
        // Tracks 1 and 3 have their own Poly.
        put(&mut mix, 0);
        put(&mut mix, 2);
        let mut listed = instrument_list::InstrumentList::new();
        listed.follow_kits(player.kits());
        listed.follow(mix.instruments.held_tracks());
        let (track1, track3) = (listed.own_of(0).unwrap(), listed.own_of(2).unwrap());
        let kit2 = listed.of_kit(1).unwrap();
        let mut tunings = tunings::Tunings::new();
        let mut handed = TuningsHanded::new();
        let kits = |player: &sampler::Player| {
            (0..player.kits())
                .map(|k| player.kit_tuning(k).cloned().unwrap())
                .collect::<Vec<_>>()
        };
        let pick = |builtin, root, a4_hz| synth::TuningPick {
            builtin,
            root,
            a4_hz,
            file: None,
        };
        // 12-TET at the start: handed over, said nothing.
        assert_eq!(
            handed.follow(&tunings, &listed, &synth, &mut mix, Some(&mut player)),
            Vec::<String>::new()
        );
        assert!(kits(&player).iter().all(|t| t.is_standard()));
        // The song in just on D; Track 3's Poly in its own meantone at 415.
        let just_d = pick(synth::Builtin::Just, 2, 440);
        let meantone = pick(synth::Builtin::Meantone, 0, 415);
        tunings.set_song(just_d);
        tunings.set_own(track3, Some(meantone));
        let said = handed.follow(&tunings, &listed, &synth, &mut mix, Some(&mut player));
        assert_eq!(
            said,
            ["sampler: the kits' keygroups play Just (5-limit) on D (pad hits and slices as tuned on the pad)"]
        );
        let (just, mean) = (just_d.tuning(&[]), meantone.tuning(&[]));
        assert_eq!(mix.instruments.tuning(0), Some(&just));
        assert_eq!(mix.instruments.tuning(2), Some(&mean));
        assert!(kits(&player).iter().all(|t| *t == just));
        // Nothing changed: nothing to say.
        assert_eq!(
            handed.follow(&tunings, &listed, &synth, &mut mix, Some(&mut player)),
            Vec::<String>::new()
        );
        // Kit 2 has its own 24-TET; the song goes Pythagorean on E: only
        // those on Song's move.
        let quarter = pick(synth::Builtin::Equal24, 0, 440);
        tunings.set_own(kit2, Some(quarter));
        let pyth = pick(synth::Builtin::Pythagorean, 4, 440);
        tunings.set_song(pyth);
        let said = handed.follow(&tunings, &listed, &synth, &mut mix, Some(&mut player));
        assert!(
            said.len() == 1 && said[0].ends_with("; 1 with a tuning of their own keep it"),
            "{said:?}"
        );
        let pyth_t = pyth.tuning(&[]);
        assert_eq!(mix.instruments.tuning(0), Some(&pyth_t));
        assert_eq!(mix.instruments.tuning(2), Some(&mean));
        let all = kits(&player);
        assert_eq!(all[1], quarter.tuning(&[]));
        assert!(all.iter().enumerate().all(|(k, t)| k == 1 || *t == pyth_t));
        // A new sound in Track 3 (the id stays) is handed its own tuning.
        put(&mut mix, 2);
        assert_eq!(
            mix.instruments.tuning(2),
            Some(&mpfree_tuning::Tuning::standard())
        );
        handed.follow(&tunings, &listed, &synth, &mut mix, Some(&mut player));
        assert_eq!(mix.instruments.tuning(2), Some(&mean));
        // Track 3's taken out: its tuning goes with its id, and its next
        // instrument plays the song's.
        assert!(mix.instruments.take_out(2));
        listed.follow(mix.instruments.held_tracks());
        assert!(tunings.follow(&listed));
        assert_eq!(tunings.own(track3), None);
        put(&mut mix, 2);
        listed.follow(mix.instruments.held_tracks());
        assert_ne!(listed.own_of(2), Some(track3));
        handed.follow(&tunings, &listed, &synth, &mut mix, Some(&mut player));
        assert_eq!(mix.instruments.tuning(2), Some(&pyth_t));
        assert_eq!(tunings.pick(Some(track1)), pyth);
    }

    /// A plugin instrument that takes only MIDI notes: it plays 12-TET.
    struct MidiOnly;

    static MIDI_ONLY: mpfree_node::NodeInfo = mpfree_node::NodeInfo {
        id: "clap:test.midi-only",
        name: "Midi Synth",
        kind: mpfree_node::Kind::Instrument,
    };

    impl mpfree_node::Node for MidiOnly {
        fn info(&self) -> &'static mpfree_node::NodeInfo {
            &MIDI_ONLY
        }
        fn params(&self) -> &'static [mpfree_node::ParamInfo] {
            &[]
        }
        fn param(&self, _: u16) -> f32 {
            0.0
        }
        fn set_param(&mut self, _: u16, _: f32) {}
        fn prepare(&mut self, _: u32, _: usize) {}
        fn reset(&mut self) {}
        fn process(
            &mut self,
            _: &mpfree_node::Ctx,
            _: &[mpfree_node::Timed],
            _: &mut [f32],
            _: &mut [f32],
        ) {
        }
        fn follows_tuning(&self) -> bool {
            false
        }
    }

    #[test]
    fn a_plugin_that_takes_only_midi_says_once_it_plays_12_tet() {
        let synth = synth::Synth::new(RATE);
        let mut mix = mixing::MixHost::new(RATE, 0.0, 6.0);
        let plain = mpfree_tuning::Tuning::standard();
        let put = |mix: &mut mixing::MixHost, track: usize| {
            let made = (
                mpfree_track::Instrument::new(Box::new(MidiOnly), RATE, PERIOD),
                instruments::Held {
                    id: MIDI_ONLY.id,
                    name: MIDI_ONLY.name,
                    instance: 0,
                },
            );
            let pads = synth.pads_for(synth::Engine::Poly);
            assert!(mix.instruments.put(track, made, pads, &plain).is_ok());
        };
        put(&mut mix, 2);
        assert!(!mix.instruments.follows_tuning(2));
        assert!(
            mix.instruments.follows_tuning(0),
            "no instrument: nothing to say"
        );
        let mut listed = instrument_list::InstrumentList::new();
        listed.follow(mix.instruments.held_tracks());
        let mut tunings = tunings::Tunings::new();
        let mut handed = TuningsHanded::new();
        // In 12-TET there's nothing to say.
        assert!(handed
            .follow(&tunings, &listed, &synth, &mut mix, None)
            .is_empty());
        let just_d = synth::TuningPick {
            builtin: synth::Builtin::Just,
            root: 2,
            a4_hz: 440,
            file: None,
        };
        tunings.set_song(just_d);
        let said = handed.follow(&tunings, &listed, &synth, &mut mix, None);
        assert_eq!(
            said,
            [
                "sampler: the kits' keygroups play Just (5-limit) on D (pad hits and slices as tuned on the pad)",
                "tunings: Track 3's own instrument, Midi Synth, takes only MIDI notes, so it plays 12-TET, not Just (5-limit) on D",
            ]
        );
        // Said once: another sound put in elsewhere hands every holder its
        // tuning again, and nothing more is said.
        put(&mut mix, 5);
        listed.follow(mix.instruments.held_tracks());
        let again = handed.follow(&tunings, &listed, &synth, &mut mix, None);
        assert_eq!(
            again,
            ["tunings: Track 6's own instrument, Midi Synth, takes only MIDI notes, so it plays 12-TET, not Just (5-limit) on D"]
        );
        // Back to 12-TET, then just again: it says so again.
        tunings.set_song(synth::TuningPick::default());
        assert_eq!(
            handed
                .follow(&tunings, &listed, &synth, &mut mix, None)
                .len(),
            1
        );
        tunings.set_song(just_d);
        assert_eq!(
            handed
                .follow(&tunings, &listed, &synth, &mut mix, None)
                .len(),
            3
        );
    }
}
