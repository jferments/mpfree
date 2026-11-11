//! Sample browser screens over a real library in a scratch folder, for the
//! workstation preview (`mpfree-browser-preview`) and the tests: a few
//! folders of drums and loops (made-up tones, written as real WAV files),
//! projects saved twice (so they have versions), a kit, a recording and
//! an item in the Trash.

use crate::state::{Browser, Button, Filter, Input, Key, SOURCES};
use crate::view::BrowserView;
use crate::{BROWSER, BROWSER_HINT, BROWSER_PAGE};
use mpfree_library::audio::{tone_wav, wav_bytes};
use mpfree_library::{Library, Loc, Meta, Place, Snapshot};
use mpfree_ui::widgets::{Header, Label};
use mpfree_ui::{Align, Fonts, Theme, Tone, Ui};
use std::path::Path;

pub struct Sample {
    pub name: &'static str,
    pub about: &'static str,
    pub ui: Ui,
}

/// The screen size on the MPC One as the player sees it.
pub const W: i32 = 1280;
pub const H: i32 = 800;

/// The factory kits' names as kits.bin would give them (made up here).
pub const FACTORY: [&str; 4] = [
    "Factory kit 1",
    "Factory kit 2",
    "Factory kit 3",
    "Factory kit 4",
];

const RATE: u32 = 44_100;

/// Fills a new library in `data` the way a user's might look.
pub fn fill(data: &Path) -> Result<Library, String> {
    let lib = Library::at(data).map_err(|e| e.to_string())?;
    let e = |e: std::io::Error| e.to_string();
    let s = Loc::top(Place::Samples);
    let drums = lib.new_folder(&s, "Drums").map_err(e)?;
    let loops = lib.new_folder(&s, "Loops").map_err(e)?;
    lib.new_folder(&s, "Vox").map_err(e)?;
    lib.new_folder(&s, "Field recordings").map_err(e)?;
    let tones: [(&str, f32, f32, u32); 8] = [
        ("Kick 01.wav", 55.0, 9.0, 18_000),
        ("Kick 02.wav", 48.0, 7.0, 22_000),
        ("Snare tight.wav", 190.0, 18.0, 12_000),
        ("Hat closed.wav", 7_000.0, 40.0, 4_000),
        ("Hat open.wav", 6_500.0, 8.0, 16_000),
        ("Clap.wav", 1_200.0, 22.0, 9_000),
        ("Perc shaker.wav", 4_400.0, 25.0, 6_000),
        ("Rim.wav", 900.0, 30.0, 5_000),
    ];
    for (n, hz, decay, frames) in tones {
        lib.write_new(&drums, n, &tone_wav(RATE, frames, hz, decay))
            .map_err(e)?;
    }
    lib.write_new(
        &loops,
        "Break 92 bpm.wav",
        &tone_wav(RATE, 115_000, 140.0, 0.6),
    )
    .map_err(e)?;
    lib.write_new(
        &loops,
        "Bass loop A.wav",
        &tone_wav(RATE, 90_000, 82.0, 0.3),
    )
    .map_err(e)?;
    lib.write_new(&s, "Rhodes chord.wav", &tone_wav(RATE, 120_000, 262.0, 0.9))
        .map_err(e)?;
    lib.write_new(&s, "Old kick.wav", &wav_bytes(RATE, 1, 16, 8_000))
        .map_err(e)?;
    lib.write_new(&Loc::top(Place::Kits), "Boom bap.kit", b"mpfree-kit 1\n")
        .map_err(e)?;
    let inst = Loc::top(Place::Instruments);
    for (engine, preset) in [("FM", "Glass bell.preset"), ("Poly", "Warm pad.preset")] {
        let f = lib.new_folder(&inst, engine).map_err(e)?;
        lib.write_new(&f, preset, b"mpfree-preset 1\n").map_err(e)?;
    }
    lib.write_new(
        &Loc::top(Place::Recordings),
        "Take 2026-11-11 20.31.wav",
        &tone_wav(RATE, 80_000, 330.0, 1.5),
    )
    .map_err(e)?;
    let p = Loc::top(Place::Projects);
    for (name, tempo, saves) in [
        ("Night drive", 92.0, 3),
        ("Sunday chops", 88.0, 1),
        ("Live set", 124.0, 2),
    ] {
        let loc = p.join(name).map_err(e)?;
        for i in 0..saves {
            let snap = Snapshot {
                meta: Meta {
                    name: name.into(),
                    tempo,
                    created: 1_794_400_000,
                    saved: 1_794_430_000 + i * 600,
                    app: "preview".into(),
                    extra: Vec::new(),
                },
                parts: vec![("transport".into(), format!("bpm = {tempo}\n"))],
            };
            lib.save_project(&loc, &snap, 10).map_err(e)?;
        }
    }
    let old = s.join("Old kick.wav").map_err(e)?;
    lib.delete(&old).map_err(e)?;
    Ok(lib)
}

fn ui(fonts: Fonts, theme: Theme, b: &Browser) -> Result<Ui, String> {
    let mut ui = Ui::new(theme, fonts, W, H);
    ui.add("header", Header::app("Sample"));
    ui.add("hint", Label::new(BROWSER_HINT, Tone::Dim, Align::Left));
    crate::add(&mut ui, b)?;
    ui.show(BROWSER_PAGE)?;
    Ok(ui)
}

fn source(label: &str) -> usize {
    SOURCES.iter().position(|s| s.label() == label).unwrap_or(0)
}

fn select(b: &mut Browser, name: &str) {
    let s = b.screen();
    if let Some(i) = s.rows.iter().position(|r| r.name == name) {
        if s.selected != Some(i) {
            b.input(Input::Row(i));
        }
    }
}

fn open(b: &mut Browser, name: &str) {
    select(b, name);
    b.push();
}

/// The sample screens, each with its own browser over a library filled
/// by [`fill`] in `data` (which must be a new or empty folder).
pub fn samples(fonts: impl Fn() -> Fonts, data: &Path) -> Result<Vec<Sample>, String> {
    let lib = fill(data)?;
    let mut out = Vec::new();
    let mut shot = |name: &'static str,
                    about: &'static str,
                    theme: Theme,
                    setup: &dyn Fn(&mut Browser)|
     -> Result<(), String> {
        let mut b = Browser::inline(lib.clone());
        b.set_factory(FACTORY.iter().map(|s| s.to_string()).collect());
        b.load_lists(
            "recent = Projects/Night drive\nrecent = Samples/Drums/Kick 01.wav\nrecent = Samples/Loops/Break 92 bpm.wav\nfavourite = Samples/Drums/Snare tight.wav\nfavourite = Samples/Loops\n",
        );
        let mut u = ui(fonts(), theme, &b)?;
        b.set_visible(BrowserView::rows_in(u.rect_of(BROWSER).unwrap_or_default()));
        setup(&mut b);
        let _ = b.take_actions();
        crate::show(&mut u, &b);
        out.push(Sample { name, about, ui: u });
        Ok(())
    };
    shot(
        "01-samples-drums",
        "Samples / Drums with Kick 01 selected: its length, format and a quick waveform; Load, Load to pad, Audition, Favourite, Rename, Copy, Cut, Delete",
        Theme::dark(),
        &|b| {
            b.go_source(source("Samples"));
            open(b, "Drums");
            select(b, "Kick 01.wav");
        },
    )?;
    shot(
        "02-samples-top",
        "The top of Samples: folders first, then files, in natural order; a favourite folder starred",
        Theme::dark(),
        &|b| {
            b.go_source(source("Samples"));
            select(b, "Rhodes chord.wav");
        },
    )?;
    shot(
        "03-projects",
        "Projects, with Night drive selected: tempo and when it was saved; Open, Favourite, Rename, Copy, Delete",
        Theme::dark(),
        &|b| {
            b.go_source(source("Projects"));
            select(b, "Night drive");
        },
    )?;
    shot(
        "04-rename-keyboard",
        "Rename: the on-screen keyboard over the browser, the name ready to edit",
        Theme::dark(),
        &|b| {
            b.go_source(source("Samples"));
            open(b, "Drums");
            select(b, "Snare tight.wav");
            b.input(Input::Button(Button::Rename));
            b.input(Input::Key(Key::Backspace));
        },
    )?;
    shot(
        "05-delete-asks",
        "Delete asks first, with the name, and moves the item to the Trash",
        Theme::dark(),
        &|b| {
            b.go_source(source("Samples"));
            open(b, "Drums");
            select(b, "Clap.wav");
            b.erase();
        },
    )?;
    shot(
        "06-trash",
        "The Trash: where each item was; Put back, Empty Trash",
        Theme::dark(),
        &|b| b.go_source(source("Trash")),
    )?;
    shot(
        "07-factory",
        "Factory: the kits made from the owner's own backup, read only; Load and Load to pad",
        Theme::dark(),
        &|b| b.go_source(source("Factory")),
    )?;
    shot(
        "08-recent",
        "Recent: what was last loaded, opened or saved, with where it is",
        Theme::dark(),
        &|b| b.go_source(source("Recent")),
    )?;
    shot(
        "09-cut-and-paste",
        "Cut a loop, go to another folder: Paste appears beside New folder; the Audio filter on",
        Theme::dark(),
        &|b| {
            b.go_source(source("Samples"));
            open(b, "Loops");
            select(b, "Bass loop A.wav");
            b.input(Input::Button(Button::Cut));
            b.back();
            b.input(Input::Filter(Filter::Audio));
        },
    )?;
    shot(
        "10-empty-place",
        "An empty place (Patterns)",
        Theme::dark(),
        &|b| b.go_source(source("Patterns")),
    )?;
    shot(
        "11-synthwave",
        "Samples / Drums in the synthwave theme (same font, the theme's colours)",
        Theme::synthwave(),
        &|b| {
            b.go_source(source("Samples"));
            open(b, "Drums");
            select(b, "Hat open.wav");
        },
    )?;
    Ok(out)
}
