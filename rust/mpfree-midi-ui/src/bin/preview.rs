//! mpfree-midi-preview: WORKSTATION ONLY. Draws the MIDI, Sync and CV
//! sections inside the mixer's real I/O view as PNG files, with a sample
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
    /// Taps after the section is picked.
    taps: Vec<(i32, i32)>,
}

fn shots() -> Vec<Shot> {
    let rig = sample::rig();
    let mut built = sample::rig();
    built.live.built = Built {
        notes_out: true,
        clock_in: true,
        clock_out: true,
        cv: true,
    };
    built.live.cv_volts = [2.0, 5.0, 0.0, 0.0, 5.0, 0.0, 0.0, 0.0];
    vec![
        Shot {
            name: "io-midi",
            about: "MIDI: each port's input and thru, and where each track plays (one setting, port and channel together); MIDI out not built yet, so the rows say so",
            section: 1,
            model: rig.clone(),
            taps: vec![],
        },
        Shot {
            name: "io-midi-picker",
            about: "MIDI: a track's Plays on picker, the port and the channel on one page",
            section: 1,
            model: rig.clone(),
            // The Lead track's Plays on field (the rows' layout is fixed).
            taps: vec![(186 + 330 + 135, 0)],
        },
        Shot {
            name: "io-sync",
            about: "Sync: tempo from the 5-pin jacks with clock arriving, following, and clock out per port; following not built yet",
            section: 2,
            model: rig.clone(),
            taps: vec![],
        },
        Shot {
            name: "io-cv",
            about: "CV: the eight outputs with job, track and main setting; CV out not built yet",
            section: 3,
            model: rig,
            taps: vec![],
        },
        Shot {
            name: "io-midi-built",
            about: "MIDI once MIDI out is built: rows say where each track plays",
            section: 1,
            model: built.clone(),
            taps: vec![],
        },
        Shot {
            name: "io-cv-built",
            about: "CV once CV out is built: each output's volts now",
            section: 3,
            model: built,
            taps: vec![],
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
    mpfree_mixer_ui::io_tick(&mut ui, Duration::from_secs(1));
    set_hint(&mut ui);
    ui.paint(&mut canvas);
    for &(x, y) in &shot.taps {
        // y 0: the Plays on field of the fourth track row.
        let y = if y == 0 {
            lead_row_y(&ui, &shot.model)
        } else {
            y
        };
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
