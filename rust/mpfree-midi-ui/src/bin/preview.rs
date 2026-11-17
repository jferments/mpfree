//! mpfree-midi-preview: WORKSTATION ONLY. Draws the MIDI, Routing, Sync,
//! CV and Monitor sections inside the mixer's real I/O view as PNG files, with a sample
//! rig (the 5-pin jacks, a USB keyboard, a drum machine the project names
//! but that isn't plugged in), so the screens can be looked at before they
//! go on the MPC. Opens no device.
//!
//! Options: `--out DIR` (default `~/mpfree-work/midi-preview-<UTC>`; must be
//! a new or empty folder).

#![forbid(unsafe_code)]

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mpfree_midi::setup::{ClockOut, IgnoreSet, Ignored, TrackOut, TrackRoute, DIN_NAME};
use mpfree_midi_ui::{sample, sections, Built, IoModel};
use mpfree_mixer_ui::sample::{name, play, screen, song_mixer};
use mpfree_mixer_ui::{MixState, MixView};
use mpfree_ui::{Fonts, SoftCanvas, Touch, Ui};

fn tap(ui: &mut Ui, x: i32, y: i32) {
    ui.touch(1, Touch::Down, x, y);
    ui.touch(1, Touch::Up, x, y);
}

struct Shot {
    name: &'static str,
    about: &'static str,
    section: usize,
    model: IoModel,
    /// Data wheel detents after the section is picked (scrolls the list).
    wheel: i32,
    /// Taps after the section is picked.
    taps: Vec<(i32, i32)>,
    /// The app's clock when it is drawn (seconds).
    at: u64,
    /// Then the page of this port, opened by its row's Open ›.
    open: Option<&'static str>,
    /// Then a tap on the control whose label starts with this (a Routing
    /// square: "Keystep 37 → Tracks").
    pick: Option<&'static str>,
}

fn shots() -> Vec<Shot> {
    let rig = sample::rig();
    let mut built = sample::rig();
    built.live.built = Built {
        notes_out: true,
        thru: true,
        clock_in: true,
        clock_out: true,
        cv_outs: 4,
        monitor: true,
    };
    // MIDI out on: every port with an output plugged in takes what is sent.
    for p in built.live.ports.iter_mut() {
        p.sending = p.has_out;
    }
    built.live.cv_volts = [2.0, 5.0, 0.0, 0.0, 5.0, 0.0, 0.0, 0.0];
    // Four learned controls: a knob, an endless encoder, a key, and one on
    // the drum machine that isn't plugged in.
    for (text, name) in [
        (
            "port Keystep 37 | ch 1 | cc 74 | t1.fx1.0 | 0 1 | absolute | jump",
            "Cutoff (Filter, Drums)",
        ),
        (
            "port Keystep 37 | ch 1 | cc 16 | t2.mix.0 | 0 1 | relative-64 | jump",
            "Volume (Bass)",
        ),
        (
            "port MIDI | ch 10 | note 36 | t6.mix.2 | 0 1 | toggle | jump",
            "Mute (Perc)",
        ),
        (
            "port Digitakt | ch 1 | cc 1 | t4.mix.6 | 0 1 | absolute | jump",
            "Send A (Lead)",
        ),
    ] {
        let l = mpfree_midi::learn::Saved::parse(text).expect("a learned control");
        built.setup.learned(l);
        built.live.learn_names.push(name.into());
    }
    let rig_port = rig.clone();
    // The keyboard with MIDI out on: the Pads track plays on it and it
    // gets clock while playing, 3 ms early.
    let mut usb_port = sample::monitored(built.clone());
    usb_port.live.early_ms = 8.7;
    usb_port.setup.tracks[2] = Some(TrackOut::new("Keystep 37", 0));
    usb_port.setup.set_port("Keystep 37", |p| {
        p.clock_out = ClockOut::WhilePlaying;
        p.delay_tenths = 30;
    });
    let mut din_port = sample::monitored(built.clone());
    din_port.live.early_ms = 8.7;
    din_port.setup.set_port(DIN_NAME, |p| {
        p.ignore = IgnoreSet::NONE
            .with(Ignored::Clock, true)
            .with(Ignored::Aftertouch, true);
        p.delay_tenths = 65;
    });
    // Routing: the drum machine on the 5-pin jacks' channel 10 plays
    // Drums, and the keyboard (Learn only) plays Bass on channel 2 and Lead
    // on channel 1, whichever track is armed.
    let mut routing = built.clone();
    let route = |port: &str, channel: u8, track: u8| TrackRoute {
        port: port.into(),
        channel: Some(channel),
        track,
    };
    routing.setup.routes = vec![
        route(DIN_NAME, 9, 0),
        route("Keystep 37", 1, 1),
        route("Keystep 37", 0, 3),
    ];
    vec![
        Shot {
            name: "io-midi",
            about: "MIDI: each port's input and thru, and where each track plays (one setting, port and channel together); MIDI out off, so the rows say so",
            section: 1,
            model: rig.clone(),
            wheel: 0,
            taps: vec![],
            at: 1,
            open: None,
            pick: None,
        },
        Shot {
            name: "io-midi-picker",
            about: "MIDI: a track's Plays on picker, the port and the channel on one page",
            section: 1,
            model: rig.clone(),
            // The Lead track's Plays on field (the rows' layout is fixed).
            wheel: 0,
            taps: vec![(186 + 290 + 110, 0)],
            at: 1,
            open: None,
            pick: None,
        },
        Shot {
            name: "io-routing",
            about: "Routing (board 5, without blocks): inputs, the pads and the tracks down the side; the armed track, tracks by channel, Learn, each MIDI out and CV across; the keyboard's card picked: it is Learn only, yet plays Bass on channel 2 and Lead on channel 1",
            section: 2,
            model: routing,
            wheel: 0,
            taps: vec![],
            at: 1,
            open: None,
            pick: Some("Keystep 37 → Tracks"),
        },
        Shot {
            name: "io-sync",
            about: "Sync: tempo from the 5-pin jacks with clock arriving, following, and clock out per port; following not built yet",
            section: 3,
            model: rig.clone(),
            wheel: 0,
            taps: vec![],
            at: 1,
            open: None,
            pick: None,
        },
        Shot {
            name: "io-cv",
            about: "CV: the eight outputs with job, track and main setting; CV out not built yet",
            section: 4,
            model: rig,
            wheel: 0,
            taps: vec![],
            at: 1,
            open: None,
            pick: None,
        },
        Shot {
            name: "io-midi-built",
            about: "MIDI once MIDI out is on: rows say where each track plays",
            section: 1,
            model: built.clone(),
            wheel: 0,
            taps: vec![],
            at: 1,
            open: None,
            pick: None,
        },
        Shot {
            name: "io-midi-learn",
            about: "MIDI, scrolled to MIDI Learn: each learned control, how it works and what it moves, with Forget; one on a device that isn't plugged in",
            section: 1,
            model: built.clone(),
            wheel: 40,
            taps: vec![],
            at: 1,
            open: None,
            pick: None,
        },
        Shot {
            name: "io-cv-built",
            about: "CV once CV out is built: each output's volts now",
            section: 4,
            model: built.clone(),
            wheel: 0,
            taps: vec![],
            at: 1,
            open: None,
            pick: None,
        },
        Shot {
            name: "io-monitor",
            about: "Monitor: a keyboard's chord and mod wheel coming in, the Bass track playing out of the 5-pin jacks, clock arriving with its tempo, each line with where it went; lights per port and channel, in on top and out below",
            section: 5,
            model: sample::monitored(built.clone()),
            wheel: 0,
            taps: vec![],
            at: 1,
            open: None,
            pick: None,
        },
        Shot {
            name: "io-monitor-channel",
            about: "Monitor after a tap on the keyboard's channel 1 light: only what came in on it",
            section: 5,
            model: sample::monitored(built.clone()),
            wheel: 0,
            // y -1: the keyboard's channel 1 light, in.
            taps: vec![(0, -1)],
            at: 1,
            open: None,
            pick: None,
        },
        Shot {
            name: "io-monitor-now",
            about: "Monitor, Now view, 45 s into a session: the keyboard's chord held with E4 pressed harder, the controls that moved (the learned knob says what it moves), program and bank, the clock from the 5-pin jacks with a pulse lost, bend and pressure, and the Bass track's C2 stuck on MIDI out",
            section: 5,
            model: sample::held_now(built),
            wheel: 0,
            // y -2: the Now half of List | Now.
            taps: vec![(0, -2)],
            at: sample::HELD_AT as u64,
            open: None,
            pick: None,
        },
        Shot {
            name: "io-midi-port",
            about: "A port's page (board 6), the USB keyboard: Learn only, nothing ignored, what it does today; Send to it greyed while MIDI out is off",
            section: 1,
            model: rig_port,
            wheel: 0,
            taps: vec![],
            at: 1,
            open: Some("Keystep 37"),
            pick: None,
        },
        Shot {
            name: "io-midi-port-usb",
            about: "The USB keyboard's page with MIDI out on: the Pads track plays on it, clock while playing, the gear 3 ms late so MIDI out sends that early, test note and All notes off",
            section: 1,
            model: usb_port,
            wheel: 0,
            taps: vec![],
            at: 1,
            open: Some("Keystep 37"),
            pick: None,
        },
        Shot {
            name: "io-midi-port-din",
            about: "The 5-pin jacks' page with MIDI out on: clock and aftertouch ignored, the gear 6.5 ms late so MIDI out sends that early, the newest message in and out, test note and All notes off",
            section: 1,
            model: din_port,
            wheel: 0,
            taps: vec![],
            at: 1,
            open: Some(DIN_NAME),
            pick: None,
        },
    ]
}

/// The hint line, as the app sets it after a view or section change.
fn set_hint(ui: &mut Ui) {
    let hint = mpfree_mixer_ui::io_hint(ui).unwrap_or_default();
    if let Some(l) = ui.get_mut::<mpfree_ui::widgets::Label>("hint") {
        l.text = hint;
    }
}

fn render(shot: &Shot) -> Result<SoftCanvas, String> {
    let mut m = song_mixer();
    play(&mut m, 60);
    let st = MixState::from_mixer(&mut m, MixView::Io, &name);
    let mut ui = screen(Fonts::embedded(), st)?;
    let model = Arc::new(Mutex::new(shot.model.clone()));
    for s in sections(&model) {
        mpfree_mixer_ui::add_io_section(&mut ui, s);
    }
    let mut canvas = SoftCanvas::new(1280, 800);
    ui.paint(&mut canvas);
    if !mpfree_mixer_ui::io_select(&mut ui, shot.section) {
        return Err(format!("{}: no section {}", shot.name, shot.section));
    }
    mpfree_mixer_ui::io_tick(&mut ui, Duration::from_secs(shot.at));
    if shot.wheel != 0 {
        mpfree_mixer_ui::io_input(&mut ui, mpfree_mixer_ui::IoInput::Wheel(shot.wheel));
    }
    set_hint(&mut ui);
    ui.paint(&mut canvas);
    for &(x, y) in &shot.taps {
        // y 0: the Plays on field of the fourth track row; y -1: the
        // Monitor's second port's channel 1 light, in; y -2: the Monitor's
        // Now.
        let (x, y) = match y {
            0 => (x, lead_row_y(&ui, &shot.model)),
            -1 => monitor_light(&ui),
            -2 => monitor_now(&ui),
            _ => (x, y),
        };
        tap(&mut ui, x, y);
        set_hint(&mut ui);
        ui.paint(&mut canvas);
    }
    if let Some(port) = shot.open {
        let label = format!("{port}: Open ›");
        let t = ui
            .targets()
            .into_iter()
            .find(|t| t.target.id == "field" && t.target.label == label)
            .ok_or_else(|| format!("{}: no Open › on {port}'s row", shot.name))?;
        let (x, y) = t.target.centre();
        tap(&mut ui, x, y);
        set_hint(&mut ui);
        ui.paint(&mut canvas);
    }
    if let Some(pick) = shot.pick {
        let t = ui
            .targets()
            .into_iter()
            .find(|t| t.target.label.starts_with(pick))
            .ok_or_else(|| format!("{}: nothing labelled {pick}", shot.name))?;
        let (x, y) = t.target.centre();
        tap(&mut ui, x, y);
        set_hint(&mut ui);
        ui.paint(&mut canvas);
    }
    Ok(canvas)
}

/// Where the Lead track's row is in the MIDI section of the sample rig.
fn lead_row_y(ui: &Ui, m: &IoModel) -> i32 {
    let mixer = ui.rect_of(mpfree_mixer_ui::MIXER).expect("the mixer");
    let panel_y = mixer.y + mpfree_mixer_ui::view::HEAD_H + 8;
    let banner = if m.live.built.notes_out { 0 } else { 40 + 8 };
    // Heading, ports (plugged and named), heading, then tracks 1 to 4.
    let ports = 3;
    let top = panel_y + banner + 42 + ports * (56 + 8) + 42;
    top + 3 * (56 + 8) + 28
}

/// The middle of the Monitor's channel 1 light, in, on its second port
/// (the rows' layout is fixed: two rows of chips, the channel numbers,
/// then a row of 40 px and a gap of 6 per port).
fn monitor_light(ui: &Ui) -> (i32, i32) {
    let mixer = ui.rect_of(mpfree_mixer_ui::MIXER).expect("the mixer");
    let x = mixer.x + mpfree_mixer_ui::io::LIST_W + 12;
    let y = mixer.y + mpfree_mixer_ui::view::HEAD_H + 8;
    (x + 170 + 16, y + 44 + 8 + 44 + 12 + 22 + 46 + 9)
}

/// The middle of the Monitor's Now (the second half of List | Now, the
/// first control on its first row).
fn monitor_now(ui: &Ui) -> (i32, i32) {
    let mixer = ui.rect_of(mpfree_mixer_ui::MIXER).expect("the mixer");
    let x = mixer.x + mpfree_mixer_ui::io::LIST_W + 12;
    let y = mixer.y + mpfree_mixer_ui::view::HEAD_H + 8;
    (x + 72 + 36, y + 22)
}

fn utc_stamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{secs}")
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .and_then(|mut f| f.write_all(bytes))
        .map_err(|e| format!("{}: {e}", path.display()))
}

fn run(out: Option<PathBuf>) -> Result<PathBuf, String> {
    let dir = out.unwrap_or_else(|| {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        home.join("mpfree-work")
            .join(format!("midi-preview-{}", utc_stamp()))
    });
    fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    if fs::read_dir(&dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .next()
        .is_some()
    {
        return Err(format!("{} is not empty", dir.display()));
    }
    let mut list = String::new();
    for s in shots() {
        let c = render(&s)?;
        let png = mpfree_ui::png::encode_rgb(1280, 800, &c.to_rgb());
        write_new(&dir.join(format!("{}.png", s.name)), &png)?;
        let line = format!("{}.png  {}", s.name, s.about);
        println!("{line}");
        list.push_str(&line);
        list.push('\n');
    }
    write_new(&dir.join("screens.txt"), list.as_bytes())?;
    Ok(dir)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let out = match args.as_slice() {
        [] => None,
        [o, d] if o == "--out" => Some(PathBuf::from(d)),
        _ => {
            eprintln!("usage: mpfree-midi-preview [--out DIR]");
            std::process::exit(2);
        }
    };
    match run(out) {
        Ok(dir) => println!("\nPNG files and screens.txt are in {}", dir.display()),
        Err(e) => {
            eprintln!("mpfree-midi-preview: {e}");
            std::process::exit(1);
        }
    }
}
