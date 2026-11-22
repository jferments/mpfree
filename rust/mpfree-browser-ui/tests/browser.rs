//! The browser from the outside: what taps, the dial and the buttons do to
//! a real library in a scratch folder.

use mpfree_browser_ui::sample::{fill, samples, FACTORY};
use mpfree_browser_ui::state::{when, Key, Modal, SOURCES};
use mpfree_browser_ui::{
    step, BrowseAction, Browser, BrowserView, Button, ExportBits, ExportChoice, ExportFormat,
    ExportInput, ExportPhase, ExportRange, ExportSetup, Filter, Input, Pick, Sort, StemsBy,
    BROWSER,
};
use mpfree_library::{Library, Loc, Meta, Place, Snapshot};
use mpfree_ui::{Fonts, SoftCanvas, Touch};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

fn temp() -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let d = std::env::temp_dir().join(format!(
        "mpfree-browser-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn src(label: &str) -> usize {
    SOURCES.iter().position(|s| s.label() == label).unwrap()
}

fn names(b: &Browser) -> Vec<String> {
    b.screen().rows.iter().map(|r| r.name.clone()).collect()
}

/// Moves the selection to `name` with the dial (a tap would open a folder).
fn select(b: &mut Browser, name: &str) {
    let s = b.screen();
    let i = s.rows.iter().position(|r| r.name == name).unwrap();
    match s.selected {
        Some(at) if at == i => {}
        Some(at) => b.dial(i as i32 - at as i32),
        None => b.dial(i as i32 + 1),
    }
}

fn browser() -> (Browser, Library) {
    let lib = fill(&temp()).unwrap();
    let mut b = Browser::inline(lib.clone());
    b.set_factory(FACTORY.iter().map(|s| s.to_string()).collect());
    (b, lib)
}

#[test]
fn it_opens_on_samples_with_folders_first() {
    let (b, _) = browser();
    assert_eq!(
        names(&b),
        [
            "Drums",
            "Field recordings",
            "Loops",
            "Vox",
            "Rhodes chord.wav"
        ]
    );
    assert_eq!(b.screen().path, "Samples");
    assert!(!b.screen().can_go_back);
}

#[test]
fn folders_open_with_one_tap_or_the_push_and_minus_goes_back() {
    let (mut b, _) = browser();
    select(&mut b, "Loops");
    b.input(Input::Row(0)); // one tap on a folder opens it, selected or not
    assert_eq!(b.screen().path, "Samples / Drums");
    assert_eq!(names(&b)[0], "Clap.wav");
    b.back();
    assert_eq!(b.screen().path, "Samples");
    // Back selects the folder it came out of.
    assert_eq!(b.selected_row().unwrap().name, "Drums");
    b.enter();
    assert_eq!(b.screen().path, "Samples / Drums");
}

#[test]
fn selecting_a_sound_shows_its_details_and_auditions_it() {
    let (mut b, _) = browser();
    select(&mut b, "Rhodes chord.wav");
    let s = b.screen();
    assert!(s
        .detail
        .lines
        .iter()
        .any(|(l, v)| l == "Length" && v == "0:02.72"));
    assert!(s.detail.overview.as_ref().is_some_and(|o| o.len() > 100));
    let a = b.take_actions();
    assert!(
        matches!(&a[..], [BrowseAction::Audition { path }] if path.ends_with("Rhodes chord.wav"))
    );
    // Audition off: selecting says nothing; on again plays the selection.
    b.input(Input::Button(Button::Audition));
    assert_eq!(b.take_actions(), [BrowseAction::StopAudition]);
    b.dial(-1);
    assert!(b.take_actions().is_empty());
}

#[test]
fn load_and_load_to_pad_hand_the_file_to_the_sampler_and_remember_it() {
    let (mut b, _) = browser();
    select(&mut b, "Rhodes chord.wav");
    b.take_actions();
    b.push();
    let a = b.take_actions();
    assert!(
        matches!(&a[0], BrowseAction::LoadSample { loc, .. } if loc.name() == "Rhodes chord.wav")
    );
    assert!(
        matches!(&a[1], BrowseAction::SaveLists(t) if t.contains("recent = Samples/Rhodes chord.wav"))
    );
    b.input(Input::Button(Button::LoadToPad));
    assert!(matches!(
        &b.take_actions()[0],
        BrowseAction::LoadToPad { .. }
    ));
    b.go_source(src("Recent"));
    assert_eq!(names(&b), ["Rhodes chord.wav"]);
    // A preset goes to the synth side with its engine's name.
    b.go_source(src("Instruments"));
    assert_eq!(names(&b), ["FM", "Poly"]);
    select(&mut b, "FM");
    b.push();
    select(&mut b, "Glass bell.preset");
    assert!(b
        .screen()
        .detail
        .lines
        .contains(&("Engine".to_string(), "FM".to_string())));
    b.take_actions();
    b.push();
    assert!(matches!(
        &b.take_actions()[0],
        BrowseAction::LoadPreset { loc, engine, .. }
            if engine == "FM" && loc.display() == "Instruments / FM / Glass bell.preset"
    ));
    // Factory kits load by index.
    b.go_source(src("Factory"));
    b.dial(2);
    b.push();
    assert_eq!(
        b.take_actions(),
        [BrowseAction::LoadFactoryKit {
            index: 2,
            to_pad: false
        }]
    );
}

#[test]
fn rename_through_the_keyboard() {
    let (mut b, lib) = browser();
    select(&mut b, "Vox");
    b.input(Input::Button(Button::Rename));
    assert!(matches!(b.screen().modal, Modal::Keyboard { .. }));
    // Other input waits while the keyboard is up.
    b.input(Input::Source(0));
    assert_eq!(b.screen().path, "Samples");
    for k in [
        Key::Space,
        Key::Shift,
        Key::Char('c'),
        Key::Char('h'),
        Key::Done,
    ] {
        b.input(Input::Key(k));
    }
    assert_eq!(b.screen().modal, Modal::None);
    assert!(names(&b).contains(&"Vox Ch".to_string()));
    assert_eq!(b.selected_row().unwrap().name, "Vox Ch");
    assert!(lib
        .list(&Loc::top(Place::Samples))
        .unwrap()
        .iter()
        .any(|e| e.name == "Vox Ch"));
    // A name that can't be used keeps the keyboard up and says why.
    b.input(Input::Button(Button::NewFolder));
    for _ in 0..20 {
        b.input(Input::Key(Key::Backspace));
    }
    b.input(Input::Key(Key::Char('.')));
    b.input(Input::Key(Key::Done));
    assert!(matches!(b.screen().modal, Modal::Keyboard { .. }));
    assert!(b.screen().status.unwrap().1);
    b.input(Input::Key(Key::Cancel));
    assert_eq!(b.screen().modal, Modal::None);
}

#[test]
fn delete_asks_then_goes_to_the_trash_and_comes_back() {
    let (mut b, _) = browser();
    select(&mut b, "Rhodes chord.wav");
    b.erase();
    assert!(
        matches!(&b.screen().modal, Modal::Confirm { text, .. } if text.contains("Rhodes chord.wav"))
    );
    b.input(Input::ConfirmNo);
    assert!(names(&b).contains(&"Rhodes chord.wav".to_string()));
    b.erase();
    b.push(); // the push answers yes
    assert!(!names(&b).contains(&"Rhodes chord.wav".to_string()));
    b.go_source(src("Trash"));
    assert_eq!(names(&b), ["Rhodes chord.wav", "Old kick.wav"]);
    b.input(Input::Button(Button::Restore));
    b.go_source(src("Samples"));
    assert!(names(&b).contains(&"Rhodes chord.wav".to_string()));
    // Empty Trash asks too.
    b.go_source(src("Trash"));
    b.input(Input::Button(Button::EmptyTrash));
    b.input(Input::ConfirmYes);
    assert!(names(&b).is_empty());
}

#[test]
fn cut_and_paste_moves_and_keeps_favourites_pointing_at_it() {
    let (mut b, lib) = browser();
    b.input(Input::Row(2)); // Loops
    b.push();
    select(&mut b, "Bass loop A.wav");
    b.input(Input::Button(Button::Favourite));
    b.input(Input::Button(Button::Cut));
    b.back();
    assert!(b.screen().list_buttons.contains(&Button::Paste));
    b.input(Input::Button(Button::Paste));
    assert!(names(&b).contains(&"Bass loop A.wav".to_string()));
    assert!(lib
        .list(&Loc::new(Place::Samples, &["Loops"]).unwrap())
        .unwrap()
        .iter()
        .all(|e| e.name != "Bass loop A.wav"));
    assert!(b
        .lists_text()
        .contains("favourite = Samples/Bass loop A.wav"));
    b.go_source(src("Favourites"));
    assert_eq!(names(&b), ["Bass loop A.wav"]);
}

#[test]
fn filters_keep_folders_and_the_right_files() {
    let (mut b, _) = browser();
    b.input(Input::Filter(Filter::Projects));
    assert_eq!(names(&b), ["Drums", "Field recordings", "Loops", "Vox"]);
    b.go_source(src("Projects"));
    assert_eq!(names(&b), ["Live set", "Night drive", "Sunday chops"]);
    // No New folder in Projects: projects are made by saving (New
    // project, Save as and Save instead).
    assert!(!b.screen().list_buttons.contains(&Button::NewFolder));
}

#[test]
fn the_first_save_needs_no_typing_and_later_saves_keep_versions() {
    let (mut b, lib) = browser();
    let snap = |bpm: f64| Snapshot {
        meta: Meta {
            tempo: bpm,
            ..Meta::default()
        },
        parts: vec![("transport".into(), format!("bpm = {bpm}\n"))],
    };
    let t = 1_794_431_040; // 2026-11-11 21:04 UTC
    b.save(snap(90.0), t);
    let a = b.take_actions();
    let loc = Loc::new(Place::Projects, &["Project 2026-11-11 21.04"]).unwrap();
    assert!(a.contains(&BrowseAction::ProjectSaved { loc: loc.clone() }));
    assert_eq!(b.project(), Some(&loc));
    b.save(snap(100.0), t + 60);
    assert_eq!(lib.versions(&loc).unwrap().len(), 1);
    let back = lib.open_project(&loc).unwrap();
    assert_eq!(back.part("transport"), Some("bpm = 100\n"));
    assert_eq!(back.meta.created, t);
    // Opening from the list hands the parts to the app.
    b.go_source(src("Projects"));
    select(&mut b, "Night drive");
    b.push();
    let a = b.take_actions();
    assert!(a
        .iter()
        .any(|x| matches!(x, BrowseAction::ProjectOpened { snap, .. } if snap.part("transport") == Some("bpm = 92\n"))));
    // Save as takes a typed name.
    b.save_as(t);
    b.input(Input::Key(Key::Done));
    assert!(b.take_actions().contains(&BrowseAction::SaveRequested));
}

/// docs/dev/design/project-safety.md: a project whose open left things
/// out (an effect this stick can't make, a take whose file is missing) is
/// never saved over by SHIFT + BROWSE: the save goes into a new project
/// beside it, said, and the one on the stick keeps all it has. Save as
/// saves where it is told; another open, or a refused one, sets the mark
/// as that project has it.
#[test]
fn a_project_whose_open_left_things_out_is_saved_beside_it() {
    let (mut b, lib) = browser();
    let snap = |bpm: f64| Snapshot {
        meta: Meta {
            tempo: bpm,
            ..Meta::default()
        },
        parts: vec![("transport".into(), format!("bpm = {bpm}\n"))],
    };
    let t = 1_794_431_040;
    let night = Loc::new(Place::Projects, &["Night drive"]).unwrap();
    let beside = Loc::new(Place::Projects, &["Night drive 2"]).unwrap();
    let as_it_was = lib.open_project(&night).unwrap();
    let versions = lib.versions(&night).unwrap().len();
    let open_night = |b: &mut Browser| {
        b.go_source(src("Projects"));
        select(b, "Night drive");
        b.push();
        assert!(b
            .take_actions()
            .iter()
            .any(|a| matches!(a, BrowseAction::ProjectOpened { loc, .. } if *loc == night)));
    };
    open_night(&mut b);
    // The app found what it couldn't open, part by part.
    b.left_out(["the effect clap:com.example.verb on Master".to_string()]);
    b.left_out(["a take of audio 2".to_string()]);
    assert_eq!(b.left_out_list().len(), 2);
    b.save(snap(100.0), t);
    assert!(b.take_actions().contains(&BrowseAction::ProjectSaved {
        loc: beside.clone()
    }));
    assert_eq!(b.project(), Some(&beside));
    assert_eq!(
        b.status_line(),
        Some((
            "Saved as Night drive 2: Night drive opened without the effect clap:com.example.verb on Master and 1 more, so it stays as it was",
            false
        ))
    );
    let saved = lib.open_project(&beside).unwrap();
    assert_eq!(saved.part("transport"), Some("bpm = 100\n"));
    assert_eq!(saved.meta.name, "Night drive 2");
    // Night drive is untouched: the same parts, no new version.
    let now = lib.open_project(&night).unwrap();
    assert_eq!(now.parts, as_it_was.parts);
    assert_eq!(lib.versions(&night).unwrap().len(), versions);
    // The new project holds all it has: the next save goes into it.
    assert!(b.left_out_list().is_empty());
    b.save(snap(101.0), t + 60);
    assert_eq!(b.project(), Some(&beside));
    assert_eq!(lib.versions(&beside).unwrap().len(), 1);

    // Opened again, it leaves the effect out again. Save as offers the
    // name beside it; the project's own name, typed, is the musician's
    // choice and saves over it.
    open_night(&mut b);
    assert!(b.left_out_list().is_empty(), "a new open starts clean");
    b.left_out(["the effect clap:com.example.verb on Master".to_string()]);
    b.save_as(t + 120);
    assert!(matches!(&b.screen().modal, Modal::Keyboard { text, .. } if text == "Night drive 3"));
    for _ in 0.."Night drive 3".len() {
        b.input(Input::Key(Key::Backspace));
    }
    for c in "Night drive".chars() {
        b.input(Input::Key(if c == ' ' { Key::Space } else { Key::Char(c) }));
    }
    b.input(Input::Key(Key::Done));
    assert!(b.take_actions().contains(&BrowseAction::SaveRequested));
    assert!(b.left_out_list().is_empty());
    b.save(snap(102.0), t + 180);
    assert_eq!(b.project(), Some(&night));
    assert_eq!(b.status_line(), Some(("Saved Night drive", false)));
    assert_eq!(lib.versions(&night).unwrap().len(), versions + 1);

    // A refused open puts the mark back as it was for the project still
    // open.
    open_night(&mut b);
    b.left_out(["loop 3".to_string()]);
    b.go_source(src("Projects"));
    select(&mut b, "Night drive 2");
    b.push();
    b.take_actions();
    assert!(b.left_out_list().is_empty());
    b.open_refused("Didn't open Night drive 2".into());
    assert_eq!(b.left_out_list(), ["loop 3".to_string()]);
    assert_eq!(b.project(), Some(&night));
    // New project forgets it.
    b.new_project();
    assert!(b.left_out_list().is_empty());
}

#[test]
fn lists_survive_a_restart_and_bad_lines_are_skipped() {
    let (mut b, _) = browser();
    b.load_lists("recent = Samples/Drums/Kick 01.wav\nfavourite = Samples/Loops\nrecent = ../etc/passwd\nfavourite = Nowhere/x\njunk\n");
    assert_eq!(
        b.lists_text(),
        "recent = Samples/Drums/Kick 01.wav\nfavourite = Samples/Loops\n"
    );
}

#[test]
fn the_worker_thread_gives_the_same_lists() {
    let lib = fill(&temp()).unwrap();
    let mut b = Browser::new(lib);
    let mut tries = 0;
    while names(&b).is_empty() && tries < 500 {
        b.poll();
        std::thread::sleep(std::time::Duration::from_millis(2));
        tries += 1;
    }
    assert_eq!(names(&b)[0], "Drums");
}

#[test]
fn taps_and_drags_on_the_widget() {
    let lib = fill(&temp()).unwrap();
    let mut b = Browser::inline(lib);
    let mut ui = mpfree_ui::Ui::new(mpfree_ui::Theme::dark(), Fonts::embedded(), 1280, 800);
    ui.add("header", mpfree_ui::widgets::Header::app("Play"));
    mpfree_browser_ui::add(&mut ui, &b).unwrap();
    ui.show(mpfree_browser_ui::BROWSER_PAGE).unwrap();
    step(&mut ui, &mut b);
    let r = ui.rect_of(BROWSER).unwrap();
    let rows = BrowserView::rows_in(r);
    assert!((8..=14).contains(&rows), "{rows}");
    // Tap the second row of the list: it is selected.
    let x = r.x + 220 + 12 + 100;
    let y0 = r.y + 12 + 56 + 52 / 2;
    let tap = |ui: &mut mpfree_ui::Ui, x: i32, y: i32| {
        ui.touch(1, Touch::Down, x, y);
        ui.touch(1, Touch::Up, x, y);
    };
    // Tap the fifth row, a sound: it is selected.
    tap(&mut ui, x, y0 + 4 * 52);
    step(&mut ui, &mut b);
    assert_eq!(b.selected_row().unwrap().name, "Rhodes chord.wav");
    assert!(!b.selected_row().unwrap().favourite);
    // Its star, at the row's right end, makes it a favourite; again, not.
    let (sx, sy) = BrowserView::star_point(r, 4);
    tap(&mut ui, sx, sy);
    step(&mut ui, &mut b);
    assert!(b.selected_row().unwrap().favourite);
    // New project at the foot of the places column starts one, from
    // whichever place shows.
    let (nx, ny) = BrowserView::new_project_point(r, &b.screen()).unwrap();
    tap(&mut ui, nx, ny);
    assert_eq!(step(&mut ui, &mut b), [BrowseAction::NewProject]);
    assert!(b
        .lists_text()
        .contains("favourite = Samples/Rhodes chord.wav"));
    tap(&mut ui, sx, sy);
    step(&mut ui, &mut b);
    assert!(!b.selected_row().unwrap().favourite);
    // Tap the second row, a folder: one tap opens it.
    tap(&mut ui, x, y0 + 52);
    step(&mut ui, &mut b);
    assert_eq!(b.screen().path, "Samples / Field recordings");
    // Tap Trash in the places.
    let trash_y = {
        let mut y = r.y + 34 + 4;
        for (i, s) in SOURCES.iter().enumerate() {
            if i > 0 && matches!(s.label(), "Recent" | "Trash") {
                y += 14;
            }
            if s.label() == "Trash" {
                break;
            }
            y += 50;
        }
        y + 20
    };
    tap(&mut ui, r.x + 80, trash_y);
    step(&mut ui, &mut b);
    assert_eq!(b.screen().path, "Trash");
    // Draw it, every sample screen too, without a panic.
    let mut c = SoftCanvas::new(1280, 800);
    ui.paint(&mut c);
}

#[test]
fn every_sample_screen_draws() {
    let s = samples(Fonts::embedded, &temp()).unwrap();
    assert!(s.len() >= 10);
    for mut x in s {
        let mut c = SoftCanvas::new(1280, 800);
        assert!(!x.ui.paint(&mut c).is_empty(), "{}", x.name);
    }
}

fn place_labels(b: &Browser) -> Vec<&'static str> {
    let s = b.screen();
    s.sources.iter().map(|i| s.place_label(*i)).collect()
}

#[test]
fn the_preset_picker_tries_each_preset_and_cancel_puts_the_old_one_back() {
    let (mut b, _) = browser();
    b.open_picker(Pick::Preset {
        engine: "FM".into(),
    });
    let s = b.screen();
    assert_eq!(s.title, "FM presets");
    assert_eq!(
        place_labels(&b),
        ["Factory", "Yours", "Recent", "Favourites"]
    );
    assert!(s.filters.is_empty());
    assert_eq!(s.list_buttons, [Button::SavePreset, Button::Cancel]);
    assert!(!s.can_go_back);
    assert_eq!(s.path, "Yours \u{b7} FM");
    assert_eq!(names(&b), ["Glass bell", "Metal bass", "Soft keys"]);
    // Loading, or a new track with it: nothing is renamed or deleted (the
    // row's star stars it).
    assert_eq!(s.detail.buttons, [Button::Load, Button::NewTrackSound]);
    b.erase();
    b.go_source(src("Samples"));
    assert_eq!(b.screen().modal, Modal::None);
    assert_eq!(b.screen().path, "Yours \u{b7} FM");
    // Turning the dial puts each preset on the track to play.
    b.take_actions();
    b.dial(1);
    assert!(matches!(
        &b.take_actions()[..],
        [BrowseAction::TryPreset { engine, loc, .. }] if engine == "FM" && loc.name() == "Metal bass.preset"
    ));
    b.input(Input::Button(Button::Cancel));
    assert_eq!(
        b.take_actions(),
        [
            BrowseAction::RevertPreset,
            BrowseAction::PickerClosed { loaded: false }
        ]
    );
    // The full browser is back where it was.
    let s = b.screen();
    assert_eq!(s.pick, None);
    assert_eq!(s.path, "Samples");
    assert_eq!(s.sources.len(), SOURCES.len());
    assert!(!s.filters.is_empty());
}

#[test]
fn loading_from_the_preset_picker_closes_it() {
    let (mut b, _) = browser();
    b.open_picker(Pick::Preset {
        engine: "FM".into(),
    });
    b.push();
    let a = b.take_actions();
    assert!(matches!(&a[0], BrowseAction::LoadPreset { engine, .. } if engine == "FM"));
    assert!(matches!(&a[1], BrowseAction::SaveLists(_)));
    assert_eq!(a[2], BrowseAction::PickerClosed { loaded: true });
    assert!(b.picking().is_none());
    // An engine with nothing saved yet says so, without an error.
    b.open_picker(Pick::Preset {
        engine: "Wavetable".into(),
    });
    let s = b.screen();
    assert!(s.rows.is_empty());
    let (msg, err) = s.status.unwrap();
    assert!(msg.contains("No Wavetable presets of yours yet"), "{msg}");
    assert!(!err);
    // Recent in a preset picker shows only that engine's presets.
    b.go_source(src("Recent"));
    assert!(names(&b).is_empty());
    b.cancel_picker();
    b.open_picker(Pick::Preset {
        engine: "FM".into(),
    });
    b.go_source(src("Recent"));
    assert_eq!(names(&b), ["Glass bell"]);
}

#[test]
fn the_sample_picker_loads_one_sound_for_the_pad() {
    let (mut b, _) = browser();
    b.go_source(src("Kits"));
    b.open_picker(Pick::Sample);
    assert_eq!(b.screen().title, "Load a sound");
    assert_eq!(
        place_labels(&b),
        ["Samples", "Recordings", "Recent", "Favourites"]
    );
    b.go_source(src("Kits"));
    assert_eq!(b.screen().path, "Samples");
    select(&mut b, "Rhodes chord.wav");
    // A sound auditions as it does in the full browser.
    assert!(matches!(
        &b.take_actions()[..],
        [BrowseAction::Audition { .. }]
    ));
    assert!(!b.screen().detail.buttons.contains(&Button::LoadToPad));
    assert!(!b.screen().detail.buttons.contains(&Button::Delete));
    b.push();
    let a = b.take_actions();
    assert!(
        matches!(&a[0], BrowseAction::LoadSample { loc, .. } if loc.name() == "Rhodes chord.wav")
    );
    assert_eq!(a.last(), Some(&BrowseAction::PickerClosed { loaded: true }));
    assert_eq!(b.screen().path, "Kits");
}

/// The labels of the browser's buttons, as painted.
fn button_labels(ui: &mut mpfree_ui::Ui, b: &mut Browser) -> Vec<String> {
    step(ui, b);
    let mut c = SoftCanvas::new(1280, 800);
    ui.paint(&mut c);
    ui.targets()
        .into_iter()
        .filter(|t| t.widget == BROWSER && t.target.id == "button")
        .map(|t| t.target.label)
        .collect()
}

#[test]
fn load_names_the_pad_picked_and_next_pad_hit_waits_for_a_hit() {
    let (mut b, _) = browser();
    let mut ui = mpfree_ui::Ui::new(mpfree_ui::Theme::dark(), Fonts::embedded(), 1280, 800);
    ui.add("header", mpfree_ui::widgets::Header::app("Play"));
    mpfree_browser_ui::add(&mut ui, &b).unwrap();
    ui.show(mpfree_browser_ui::BROWSER_PAGE).unwrap();
    // Without a pad, plain Load.
    select(&mut b, "Rhodes chord.wav");
    assert!(button_labels(&mut ui, &mut b).contains(&"Load".to_string()));
    // The pad picked names the button, and a pad hit moves it.
    b.set_pad_target(Some("A5".into()));
    assert_eq!(b.screen().load_on.as_deref(), Some("A5"));
    let labels = button_labels(&mut ui, &mut b);
    assert!(
        labels.contains(&"Load on A5".to_string()) && labels.contains(&"Next pad hit".to_string()),
        "{labels:?}"
    );
    b.set_pad_target(Some("B14".into()));
    assert!(button_labels(&mut ui, &mut b).contains(&"Load on B14".to_string()));
    // Load: onto that pad, and the line under the list says so. The
    // browser stays open.
    b.take_actions();
    b.push();
    let a = b.take_actions();
    assert!(
        matches!(&a[0], BrowseAction::LoadSample { loc, .. } if loc.name() == "Rhodes chord.wav"),
        "{a:?}"
    );
    assert_eq!(
        b.screen().status.unwrap().0,
        "Loaded Rhodes chord.wav on B14"
    );
    assert!(b.picking().is_none());
    // Next pad hit: held for the next pad hit.
    b.input(Input::Button(Button::LoadToPad));
    assert!(matches!(
        &b.take_actions()[0],
        BrowseAction::LoadToPad { .. }
    ));
    // A kit loads whole, in place of the kit the pads play: its Load
    // names the armed track whose kit it replaces, plain Load with none.
    b.go_source(src("Kits"));
    let labels = button_labels(&mut ui, &mut b);
    assert!(
        labels.contains(&"Load".to_string()) && !labels.iter().any(|l| l.starts_with("Load on")),
        "{labels:?}"
    );
    b.set_kit_target(Some("Drums".into()));
    assert_eq!(b.screen().kit_on.as_deref(), Some("Drums"));
    let labels = button_labels(&mut ui, &mut b);
    assert!(
        labels.contains(&"Load on Drums".to_string())
            && labels.contains(&"New track with this kit".to_string()),
        "{labels:?}"
    );
    b.take_actions();
    b.push();
    assert!(matches!(&b.take_actions()[0], BrowseAction::LoadKit { .. }));
    assert_eq!(
        b.screen().status.unwrap().0,
        "Loading the kit Boom bap on Drums"
    );
}

#[test]
fn opening_a_project_over_unsaved_work_asks_first() {
    let (mut b, _) = browser();
    b.open_picker(Pick::Project);
    assert_eq!(place_labels(&b), ["Projects", "Recent", "Favourites"]);
    assert_eq!(names(&b), ["Live set", "Night drive", "Sunday chops"]);
    b.set_unsaved(true);
    select(&mut b, "Night drive");
    b.push();
    assert!(
        matches!(&b.screen().modal, Modal::Confirm { text, .. } if text.contains("Night drive") && text.contains("Unsaved"))
    );
    b.input(Input::ConfirmNo);
    assert!(b.picking().is_some());
    b.take_actions();
    b.push();
    b.push(); // the push answers yes
    let a = b.take_actions();
    assert!(matches!(&a[0], BrowseAction::SaveLists(_)));
    assert!(
        matches!(&a[1], BrowseAction::ProjectOpened { loc, .. } if loc.name() == "Night drive")
    );
    assert_eq!(a[2], BrowseAction::PickerClosed { loaded: true });
    // Just opened: nothing unsaved, so the next open doesn't ask.
    b.open_picker(Pick::Project);
    select(&mut b, "Live set");
    b.push();
    assert_eq!(b.screen().modal, Modal::None);
    assert!(b.picking().is_none());
}

#[test]
fn earlier_saves_of_a_project_open_in_its_place() {
    let (mut b, lib) = browser();
    b.go_source(src("Projects"));
    select(&mut b, "Night drive");
    assert!(b.screen().detail.buttons.contains(&Button::Versions));
    b.input(Input::Button(Button::Versions));
    let s = b.screen();
    assert_eq!(s.path, "Night drive / Earlier saves");
    assert!(s.can_go_back);
    assert!(s.list_buttons.is_empty());
    assert_eq!(
        names(&b),
        [
            format!("Saved {}", when(1_794_430_600)),
            format!("Saved {}", when(1_794_430_000))
        ]
    );
    assert_eq!(s.detail.kind, "Earlier save");
    // No Open button: a second tap or the push opens it.
    assert!(s.detail.buttons.is_empty());
    // The row stands for the project's folder: Delete and the rest do nothing.
    let night = Loc::new(Place::Projects, &["Night drive"]).unwrap();
    b.erase();
    b.input(Input::Button(Button::Rename));
    b.input(Input::Button(Button::Favourite));
    assert_eq!(b.screen().modal, Modal::None);
    assert!(lib.open_project(&night).is_ok());
    b.take_actions();
    // The older one: its parts go to the app, and saving it makes it the newest.
    b.dial(1);
    b.push();
    let a = b.take_actions();
    assert!(a.iter().any(|x| matches!(x,
        BrowseAction::ProjectOpened { loc, snap } if *loc == night && snap.part("transport") == Some("bpm = 90\n"))));
    assert!(b
        .screen()
        .status
        .unwrap()
        .0
        .contains("SHIFT + BROWSE saves it as the newest"));
    assert_eq!(b.project(), Some(&night));
    // Not the newest save, so opening another one asks first.
    b.dial(-1);
    b.push();
    assert!(matches!(&b.screen().modal, Modal::Confirm { text, .. } if text.contains("Unsaved")));
    b.input(Input::ConfirmNo);
    // Back to the projects, Night drive still selected.
    b.back();
    assert_eq!(b.screen().path, "Projects");
    assert_eq!(
        b.screen().rows[b.screen().selected.unwrap()].name,
        "Night drive"
    );
    // A project saved once has none yet.
    select(&mut b, "Sunday chops");
    b.input(Input::Button(Button::Versions));
    assert!(names(&b).is_empty());
    assert!(b.screen().detail.kind.starts_with("No earlier saves yet"));
}

#[test]
fn search_looks_through_the_whole_library_but_not_the_trash() {
    let (mut b, _) = browser();
    b.go_source(src("Projects"));
    b.input(Input::Search);
    assert!(
        matches!(&b.screen().modal, Modal::Keyboard { title, .. } if title == "Search the library for")
    );
    for c in "kick".chars() {
        b.input(Input::Key(Key::Char(c)));
    }
    b.input(Input::Key(Key::Done));
    let s = b.screen();
    assert_eq!(s.path, "Search: \u{201c}kick\u{201d}");
    assert_eq!(names(&b), ["Kick 01.wav", "Kick 02.wav"]);
    assert_eq!(s.rows[0].detail, "Samples / Drums");
    assert_eq!(s.status.unwrap().0, "2 found");
    assert!(s.can_go_back);
    // Rename and Cut belong to the folder view.
    assert!(!s.detail.buttons.contains(&Button::Rename));
    assert!(s.detail.buttons.contains(&Button::Load));
    // Filters still apply.
    b.input(Input::Filter(Filter::Projects));
    assert!(names(&b).is_empty());
    b.input(Input::Filter(Filter::All));
    // Again, with the same text ready to change.
    b.input(Input::Search);
    assert!(matches!(&b.screen().modal, Modal::Keyboard { text, .. } if text == "kick"));
    for _ in 0..4 {
        b.input(Input::Key(Key::Backspace));
    }
    for c in "zzz".chars() {
        b.input(Input::Key(Key::Char(c)));
    }
    b.input(Input::Key(Key::Done));
    assert!(names(&b).is_empty());
    assert_eq!(
        b.screen().status.unwrap().0,
        "Nothing called \u{201c}zzz\u{201d}"
    );
    b.back();
    assert_eq!(b.screen().path, "Projects");
    assert_eq!(names(&b), ["Live set", "Night drive", "Sunday chops"]);
}

#[test]
fn newest_first_puts_the_last_changed_first_after_folders() {
    let (mut b, lib) = browser();
    let drums = Loc::new(Place::Samples, &["Drums"]).unwrap();
    let t = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_794_000_000);
    for (i, n) in ["Rim.wav", "Clap.wav"].iter().enumerate() {
        let f = std::fs::File::options()
            .write(true)
            .open(lib.file_path(&drums.join(n).unwrap()).unwrap())
            .unwrap();
        f.set_modified(t + std::time::Duration::from_secs(86_400 * 400 + i as u64))
            .unwrap();
    }
    b.go_source(src("Samples"));
    select(&mut b, "Drums");
    b.push();
    assert_eq!(b.screen().sort, Sort::Name);
    let by_name = names(&b);
    b.input(Input::Sort);
    assert_eq!(b.screen().sort, Sort::Newest);
    assert_eq!(names(&b)[..2], ["Clap.wav", "Rim.wav"]);
    b.input(Input::Sort);
    assert_eq!(names(&b), by_name);
}

#[test]
fn projects_offer_new_save_and_save_as_and_new_asks_over_unsaved_work() {
    let (mut b, _) = browser();
    b.go_source(src("Projects"));
    assert_eq!(
        b.screen().list_buttons,
        [Button::SaveAs, Button::Save, Button::Export]
    );
    // New project is at the foot of the places column, in every place
    // (Jay's device round, 20 Nov).
    assert!(b.screen().new_project);
    // Other places keep their own buttons, and New project.
    b.go_source(src("Samples"));
    assert!(!b.screen().list_buttons.contains(&Button::Save));
    assert!(b.screen().new_project);
    b.go_source(src("Projects"));
    // Save asks the app for the parts.
    b.input(Input::Button(Button::Save));
    assert_eq!(b.take_actions(), [BrowseAction::SaveRequested]);
    // Save as opens the keyboard with a name ready.
    b.input(Input::Button(Button::SaveAs));
    assert!(
        matches!(&b.screen().modal, Modal::Keyboard { text, .. } if text.starts_with("Project "))
    );
    b.input(Input::Key(Key::Cancel));
    // New project with nothing unsaved: at once.
    select(&mut b, "Night drive");
    b.push();
    b.take_actions();
    assert!(b.project().is_some());
    b.input(Input::Button(Button::NewProject));
    assert_eq!(b.take_actions(), [BrowseAction::NewProject]);
    assert_eq!(b.project(), None);
    // Over unsaved work it asks first; No keeps everything.
    b.set_unsaved(true);
    b.input(Input::Button(Button::NewProject));
    assert!(matches!(&b.screen().modal, Modal::Confirm { text, .. } if text.contains("Unsaved")));
    b.input(Input::ConfirmNo);
    assert!(b.take_actions().is_empty());
    assert!(b.unsaved());
    b.input(Input::Button(Button::NewProject));
    b.input(Input::ConfirmYes);
    assert_eq!(b.take_actions(), [BrowseAction::NewProject]);
    assert!(!b.unsaved());
    // A picker offers none of them.
    b.open_picker(Pick::Project);
    assert_eq!(b.screen().list_buttons, [Button::Cancel]);
    assert!(!b.screen().new_project);
    b.input(Input::Button(Button::NewProject));
    assert!(b.take_actions().is_empty());
}

/// Autosave writes the unsaved work into its own project, quietly. At the
/// next start, work newer than the last save is set aside as a project of
/// its own, which opens as unsaved work and saves as a new project.
#[test]
fn autosaved_work_is_set_aside_at_the_next_start_and_opens_unsaved() {
    let (mut b, lib) = browser();
    let snap = |bpm: f64| Snapshot {
        meta: Meta {
            tempo: bpm,
            ..Meta::default()
        },
        parts: vec![("transport".into(), format!("bpm = {bpm}\n"))],
    };
    let t = 1_794_431_040; // 2026-11-11 21:04 UTC
    b.save(snap(90.0), t);
    let mine = b.project().cloned().unwrap();
    let autosave = Loc::new(Place::Projects, &["Autosave"]).unwrap();
    b.take_actions();
    b.set_unsaved(true);
    let status = b.screen().status;
    b.autosave_with(snap(95.0), Vec::new(), t + 120);
    assert_eq!(
        b.take_actions(),
        [BrowseAction::Autosaved {
            loc: autosave.clone()
        }]
    );
    assert_eq!(b.project(), Some(&mine), "the project stays");
    assert!(b.unsaved(), "still unsaved");
    assert_eq!(b.screen().status, status, "quietly");
    let back = lib.open_project(&autosave).unwrap();
    assert!(back.meta.is_autosave());
    assert_eq!(back.meta.name, format!("Autosave of {}", mine.name()));
    assert_eq!(back.part("transport"), Some("bpm = 95\n"));
    // Saved since: the next start leaves autosave's project in place.
    b.save(snap(96.0), t + 180);
    let next_start = |b: &Browser| {
        let mut n = Browser::inline(lib.clone());
        n.load_lists(&b.lists_text());
        n
    };
    assert_eq!(next_start(&b).keep_newer_autosave(), None);
    // Unsaved work autosaved after the last save (then a crash or the
    // power switch): set aside, named for when it was written, once.
    b.set_unsaved(true);
    b.autosave_with(snap(97.0), Vec::new(), t + 240);
    let mut n = next_start(&b);
    let kept = Loc::new(Place::Projects, &["Unsaved 2026-11-11 21.08"]).unwrap();
    assert_eq!(n.keep_newer_autosave(), Some(Ok(kept.clone())));
    assert_eq!(n.keep_newer_autosave(), None);
    assert!(lib.open_project(&autosave).is_err(), "autosave starts anew");
    // It opens as unsaved work, and SHIFT + BROWSE keeps it as a new
    // project, never over the one set aside.
    n.go_source(src("Projects"));
    select(&mut n, kept.name());
    n.push();
    assert!(n
        .take_actions()
        .iter()
        .any(|a| matches!(a, BrowseAction::ProjectOpened { snap, .. } if snap.part("transport") == Some("bpm = 97\n"))));
    assert_eq!(n.project(), None);
    assert!(n.unsaved());
    let said = n.screen().status.unwrap().0;
    assert!(
        said.contains("autosaved") && said.contains("SHIFT + BROWSE"),
        "{said}"
    );
    n.open_landed();
    assert!(n.unsaved(), "still unsaved once the engines hold it");
    n.save(snap(97.0), t + 360);
    assert_eq!(
        n.project().map(|l| l.name().to_string()).as_deref(),
        Some("Project 2026-11-11 21.10")
    );
    assert!(!n.unsaved());
    assert!(lib.open_project(&kept).unwrap().meta.is_autosave());
    // Another project opened lands with nothing unsaved.
    select(&mut n, "Night drive");
    n.push();
    n.take_actions();
    n.open_landed();
    assert!(!n.unsaved());
}

fn export_setup(arrangement: bool) -> ExportSetup {
    ExportSetup {
        arrangement: arrangement.then(|| "3:50, 96 bars".to_string()),
        from_bar: 3,
        mode: "Fast, quiet".into(),
        stems: 5,
        tracks_arrangement: (6, 3),
        tracks_bars: (6, 3),
        folder: "Recordings / Night drive 2026-11-14 01.32".into(),
        format: ExportFormat {
            bits: ExportBits::I24,
            rate: 44_100,
        },
    }
}

#[test]
fn export_asks_the_app_and_its_panel_takes_the_screen() {
    let (mut b, lib) = browser();
    b.go_source(src("Projects"));
    b.input(Input::Button(Button::Export));
    assert_eq!(b.take_actions(), [BrowseAction::ExportAsked]);
    assert!(b.screen().export.is_none(), "the app opens it");
    b.open_export(export_setup(false));
    let p = b.screen().export.unwrap();
    assert_eq!(p.range, ExportRange::Bars(8));
    assert!(p.mixdown && p.stems && p.midi);
    // While it shows, the browser under it takes nothing.
    let before = b.screen().selected;
    b.dial(2);
    b.input(Input::Row(1));
    b.input(Input::Button(Button::NewProject));
    assert!(b.take_actions().is_empty());
    assert_eq!(b.screen().selected, before);
    assert_eq!(
        b.screen().export.unwrap().range,
        ExportRange::Bars(10),
        "the wheel sets the bars"
    );
    // Ticks, stems by channel and a format, then Start (the push).
    b.input(Input::Export(ExportInput::Midi));
    assert_eq!(b.screen().export.unwrap().by, StemsBy::Track);
    b.input(Input::Export(ExportInput::ByChannel));
    b.input(Input::Export(ExportInput::Fewer));
    b.input(Input::Export(ExportInput::Bits16));
    b.input(Input::Export(ExportInput::Rate48));
    b.push();
    let want = ExportChoice {
        mixdown: true,
        stems: true,
        midi: false,
        by: StemsBy::Channel,
        range: ExportRange::Bars(9),
        format: ExportFormat {
            bits: ExportBits::I16,
            rate: 48_000,
        },
    };
    assert_eq!(b.take_actions(), [BrowseAction::ExportStart(want)]);
    assert!(b.exporting());
    // Running: only Cancel; - does not close it; it stays through a
    // picker (hidden there) and comes back after.
    b.back();
    b.input(Input::Export(ExportInput::Close));
    assert!(b.exporting());
    b.export_running("0:04 of 0:19".into(), Some(0.25));
    b.open_picker(Pick::Sample);
    assert!(b.screen().export.is_none());
    b.cancel_picker();
    b.take_actions();
    assert_eq!(
        b.screen().export.unwrap().phase,
        ExportPhase::Running {
            words: "0:04 of 0:19".into(),
            part: Some(0.25)
        }
    );
    // Asking again while it runs changes nothing.
    b.open_export(export_setup(true));
    assert!(b.exporting());
    b.input(Input::Export(ExportInput::Cancel));
    assert_eq!(b.take_actions(), [BrowseAction::ExportCancel]);
    b.export_over("Export cancelled, and its files removed".into(), None);
    assert!(!b.exporting());
    assert!(matches!(
        b.screen().export.unwrap().phase,
        ExportPhase::Stopped(_)
    ));
    b.push();
    assert!(b.screen().export.is_none(), "the push closes it");
    // Next time, the last choice is ticked again, in Prefs' format (the
    // panel's format was for that export).
    b.open_export(export_setup(true));
    let again = ExportChoice {
        format: export_setup(true).format,
        ..want
    };
    assert_eq!(b.screen().export.unwrap().choice(), again);
    b.input(Input::Export(ExportInput::Arrangement));
    b.input(Input::Export(ExportInput::Start));
    b.take_actions();
    // Done: Show in BROWSE goes to its folder in Recordings.
    let rec = Loc::top(Place::Recordings);
    let folder = lib
        .new_folder(&rec, "Night drive 2026-11-14 01.32")
        .unwrap();
    b.export_over("Exported 3:52".into(), Some(folder.clone()));
    assert_eq!(b.screen().status, Some(("Exported 3:52".into(), false)));
    b.input(Input::Export(ExportInput::Show));
    assert!(b.screen().export.is_none());
    assert_eq!(b.screen().path, folder.display());
}

#[test]
fn the_export_panel_answers_taps() {
    let lib = fill(&temp()).unwrap();
    let mut b = Browser::inline(lib);
    let mut ui = mpfree_ui::Ui::new(mpfree_ui::Theme::dark(), Fonts::embedded(), 1280, 800);
    ui.add("header", mpfree_ui::widgets::Header::app("Play"));
    mpfree_browser_ui::add(&mut ui, &b).unwrap();
    ui.show(mpfree_browser_ui::BROWSER_PAGE).unwrap();
    b.go_source(src("Projects"));
    b.open_export(export_setup(true));
    step(&mut ui, &mut b);
    // The panel's controls replace the browser's on the page.
    let mut c = SoftCanvas::new(1280, 800);
    ui.paint(&mut c);
    let labels: Vec<String> = ui
        .targets()
        .into_iter()
        .filter(|t| t.widget == BROWSER)
        .map(|t| format!("{}/{}", t.target.id, t.target.label))
        .collect();
    assert_eq!(
        labels,
        [
            "export/Mixdown",
            "export/Stems",
            "export/MIDI file",
            "export/Track",
            "export/Channel",
            "export/Arrangement",
            "export/Bars",
            "export/-",
            "export/+",
            "export/24-bit",
            "export/16-bit",
            "export/32-bit float",
            "export/44.1 kHz",
            "export/48 kHz",
            "export/Close",
            "export/Start"
        ]
    );
    let tap = |ui: &mut mpfree_ui::Ui, x: i32, y: i32| {
        ui.touch(1, Touch::Down, x, y);
        ui.touch(1, Touch::Up, x, y);
    };
    // The card is centred in the browser: Make's buttons are its first
    // row, Start is at the bottom right.
    let r = ui.rect_of(BROWSER).unwrap();
    let (w, h) = (780.min(r.w - 40), 550.min(r.h - 24));
    let card = (r.x + (r.w - w) / 2, r.y + (r.h - h) / 2);
    let (stems_x, make_y) = (
        card.0 + 24 + 120 + (w - 48 - 120) / 2,
        card.1 + 24 + 50 + 28,
    );
    tap(&mut ui, stems_x, make_y);
    step(&mut ui, &mut b);
    assert!(!b.screen().export.unwrap().stems);
    // A tap outside the card does nothing, and the list under it can't be
    // tapped.
    tap(&mut ui, r.x + 30, r.y + 30);
    step(&mut ui, &mut b);
    assert!(b.screen().export.is_some());
    tap(&mut ui, card.0 + w - 24 - 100, card.1 + h - 24 - 28);
    let acts = step(&mut ui, &mut b);
    assert!(
        matches!(acts.as_slice(), [BrowseAction::ExportStart(c)] if !c.stems && c.range == ExportRange::Arrangement),
        "{acts:?}"
    );
    // Stems by: the row under Make, Channel its second button.
    b.open_export(export_setup(true));
    b.export_over("Export cancelled, and its files removed".into(), None);
    b.push();
    b.open_export(export_setup(true));
    step(&mut ui, &mut b);
    let by_y = make_y + 28 + 8 + 28;
    tap(&mut ui, card.0 + 24 + 120 + 140 + 8 + 70, by_y);
    step(&mut ui, &mut b);
    assert_eq!(b.screen().export.unwrap().by, StemsBy::Channel);
    let mut c = SoftCanvas::new(1280, 800);
    ui.paint(&mut c);
}

#[test]
fn a_kit_loads_whole_and_plus_shows_its_sounds() {
    let (mut b, lib) = browser();
    b.go_source(src("Kits"));
    assert_eq!(names(&b), ["Boom bap"]);
    let s = b.screen();
    assert_eq!(s.rows[0].kind, mpfree_library::Kind::Kit);
    assert_eq!(s.list_buttons, [Button::SaveKit, Button::NewFolder]);
    assert!(s.detail.buttons.contains(&Button::Load));
    b.take_actions();
    // The push loads the whole kit: its folder goes to the app.
    b.push();
    let a = b.take_actions();
    let kit = Loc::new(Place::Kits, &["Boom bap"]).unwrap();
    assert!(
        matches!(&a[0], BrowseAction::LoadKit { loc, path } if *loc == kit && path.join("kit.txt").is_file()),
        "{a:?}"
    );
    assert!(lib.kit_text(&kit).unwrap().starts_with("mpfree-sampler 1"));
    // New track with this kit: the same folder, for a new track of its own.
    assert_eq!(
        &b.screen().detail.buttons[..2],
        [Button::Load, Button::NewTrack]
    );
    assert_eq!(Button::NewTrack.label(), "New track with this kit");
    b.input(Input::Button(Button::NewTrack));
    let a = b.take_actions();
    assert!(
        matches!(&a[0], BrowseAction::NewTrackWithKit { loc, path } if *loc == kit && path.join("kit.txt").is_file()),
        "{a:?}"
    );
    assert_eq!(
        b.screen().status.unwrap().0,
        "A new track for the kit Boom bap"
    );
    // + goes into it: its sounds, one by one.
    b.enter();
    assert_eq!(b.screen().path, "Kits / Boom bap");
    assert!(names(&b).contains(&"Kick.wav".to_string()));
    b.back();
    // Save kit asks the app for the kit the pads play, then saves it here.
    b.input(Input::Button(Button::SaveKit));
    assert_eq!(b.take_actions(), [BrowseAction::SaveKitRequested]);
    let wav: Vec<u8> = std::fs::read(lib.kit_sound_path(&kit, "Kick.wav").unwrap()).unwrap();
    b.save_kit(
        "Boom bap",
        "mpfree-sampler 1\nkits = 1\n".into(),
        vec![mpfree_library::NewSample::new(
            "Kick.wav",
            std::sync::Arc::new(move |w: &mut dyn std::io::Write| w.write_all(&wav)),
        )],
    );
    assert_eq!(names(&b), ["Boom bap", "Boom bap 2"]);
    assert_eq!(b.selected_row().unwrap().name, "Boom bap 2");
    assert!(b.screen().status.unwrap().0.contains("Saved the kit"));
    assert!(lib.is_kit(&Loc::new(Place::Kits, &["Boom bap 2"]).unwrap()));
}

#[test]
fn a_midi_file_loads_as_clips() {
    let (mut b, lib) = browser();
    let dir = lib.root().join("Patterns");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Sequence01.mid"), b"MThd").unwrap();
    b.go_source(src("Patterns"));
    select(&mut b, "Sequence01.mid");
    let s = b.screen();
    assert_eq!(s.detail.kind, "MIDI file");
    assert_eq!(
        s.detail.buttons,
        [
            Button::Load,
            Button::Rename,
            Button::Copy,
            Button::Cut,
            Button::Delete
        ]
    );
    assert!(s.detail.lines.iter().any(|l| l.1.contains("As clips")));
    b.take_actions();
    b.push();
    let a = b.take_actions();
    let loc = Loc::new(Place::Patterns, &["Sequence01.mid"]).unwrap();
    assert!(
        matches!(&a[0], BrowseAction::LoadMidi { loc: l, path } if *l == loc && path.is_file()),
        "{a:?}"
    );
    // It is remembered in Recent, as a sound is.
    assert!(matches!(&a[1], BrowseAction::SaveLists(t) if t.contains("Patterns/Sequence01.mid")));
    assert!(b.screen().status.unwrap().0.contains("as clips"));
}

#[test]
fn a_tap_on_a_rows_star_toggles_the_favourite() {
    let (mut b, _) = browser();
    let i = b
        .screen()
        .rows
        .iter()
        .position(|r| r.name == "Rhodes chord.wav")
        .unwrap();
    b.input(Input::Star(i));
    assert!(b.screen().rows[i].favourite);
    assert!(
        matches!(&b.take_actions()[..], [BrowseAction::SaveLists(t)] if t.contains("favourite = Samples/Rhodes chord.wav"))
    );
    // A folder can be starred without opening it.
    b.input(Input::Star(0));
    assert_eq!(b.screen().path, "Samples");
    b.go_source(src("Favourites"));
    assert_eq!(names(&b), ["Rhodes chord.wav", "Drums"]);
    b.input(Input::Star(0));
    assert_eq!(names(&b), ["Drums"]);
    // No Open or Favourite buttons anywhere in the list's items.
    b.go_source(src("Samples"));
    for name in ["Drums", "Rhodes chord.wav"] {
        select(&mut b, name);
        let d = b.screen().detail.buttons;
        assert!(
            !d.contains(&Button::Open) && !d.contains(&Button::Favourite),
            "{name}: {d:?}"
        );
    }
}

/// The pad map (the Play, Pads and Sequencer redesign's board 8): under a
/// sound, the kit's bank with the pad picked; a finger on a pad picks it
/// and plays it while held, as a hit on the MPC's pad; the arrows turn
/// the bank. Not in a picker, nor under a folder or a kit.
#[test]
fn the_pad_map_picks_a_pad_and_plays_it_while_held() {
    use mpfree_browser_ui::PadMap;
    let (mut b, _) = browser();
    let mut ui = mpfree_ui::Ui::new(mpfree_ui::Theme::dark(), Fonts::embedded(), 1280, 800);
    ui.add("header", mpfree_ui::widgets::Header::app("Play"));
    mpfree_browser_ui::add(&mut ui, &b).unwrap();
    ui.show(mpfree_browser_ui::BROWSER_PAGE).unwrap();
    let mut pads = vec![None; 128];
    pads[16] = Some("Kick soft".to_string());
    pads[28] = Some("Volca kick".to_string());
    b.set_pad_map(Some(PadMap {
        kit: "Drums' kit".into(),
        pads,
        picked: 29,
    }));
    // A folder selected: no map.
    assert!(b.screen().pad_map.is_none());
    b.go_source(src("Samples"));
    b.input(Input::Row(0));
    select(&mut b, "Kick 01.wav");
    let m = b.screen().pad_map.expect("the map under a sound");
    assert_eq!(m.title, "Drums' kit · bank B · tap a pad");
    assert_eq!((m.bank, m.banks), (1, 8));
    assert_eq!(m.cells[0].label, "B1");
    assert_eq!(m.cells[0].name.as_deref(), Some("Kick soft"));
    assert_eq!(m.cells[12].name.as_deref(), Some("Volca kick"));
    assert!(m.cells[13].picked && m.cells[13].name.is_none());
    // Painted: a target for each pad, pad 13 (B13) top left.
    step(&mut ui, &mut b);
    let mut c = SoftCanvas::new(1280, 800);
    ui.paint(&mut c);
    let targets: Vec<_> = ui
        .targets()
        .into_iter()
        .filter(|t| t.widget == BROWSER && t.target.id == "pad-map")
        .collect();
    assert_eq!(targets.len(), 16);
    assert_eq!(targets[12].target.label, "B13 Volca kick");
    let (t13, t1) = (targets[12].target.rect, targets[0].target.rect);
    assert!(t13.y < t1.y && t13.x == t1.x, "pad 13 over pad 1");
    assert!(t1.h >= 50, "a fingertip's pad: {}", t1.h);
    // A finger on B13: picked and played while it is down, let go on the
    // lift.
    b.take_actions();
    let r = ui.rect_of(BROWSER).unwrap();
    let (x, y) = BrowserView::map_point(r, &b.screen(), 12).unwrap();
    ui.touch(1, Touch::Down, x, y);
    assert_eq!(
        step(&mut ui, &mut b),
        [BrowseAction::PadPress {
            pad: 28,
            down: true
        }]
    );
    ui.touch(1, Touch::Up, x, y);
    assert_eq!(
        step(&mut ui, &mut b),
        [BrowseAction::PadPress {
            pad: 28,
            down: false
        }]
    );
    // The arrows turn the bank, round from H to A; a pad picked in
    // another bank brings that bank.
    b.input(Input::MapBank(1));
    assert_eq!(b.screen().pad_map.unwrap().bank, 2);
    b.input(Input::MapBank(-3));
    assert_eq!(b.screen().pad_map.unwrap().bank, 7);
    b.input(Input::MapPad { pad: 3, down: true });
    b.input(Input::MapPad {
        pad: 0,
        down: false,
    });
    assert_eq!(
        b.take_actions(),
        [
            BrowseAction::PadPress {
                pad: 115,
                down: true
            },
            BrowseAction::PadPress {
                pad: 115,
                down: false
            }
        ]
    );
    let mut pads = vec![None; 128];
    pads[3] = Some("Hat".to_string());
    b.set_pad_map(Some(PadMap {
        kit: "Drums' kit".into(),
        pads,
        picked: 3,
    }));
    assert_eq!(b.screen().pad_map.unwrap().bank, 0);
    // A kit selected, or no kit on the pads: no map.
    b.set_pad_map(None);
    assert!(b.screen().pad_map.is_none());
}

fn keys_now(name: &str, factory: Option<usize>) -> mpfree_browser_ui::PresetNow {
    mpfree_browser_ui::PresetNow {
        name: name.into(),
        factory,
        changed: true,
    }
}

#[test]
fn the_preset_picker_opens_on_the_engines_own_presets_and_tries_them() {
    let (mut b, _) = browser();
    let own: Vec<String> = ["Init", "Soft Tines", "Bright Tines", "Reed Bite"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    b.open_preset_picker("Keys", own.clone(), keys_now("Reed Bite", Some(3)));
    let s = b.screen();
    assert_eq!(s.title, "Keys presets");
    assert_eq!(
        place_labels(&b),
        ["Factory", "Yours", "Recent", "Favourites"]
    );
    assert_eq!(SOURCES[s.source].label(), "Factory");
    assert_eq!(s.path, "Factory \u{b7} Keys");
    assert_eq!(names(&b), own);
    assert_eq!(s.selected, Some(3));
    assert_eq!(s.list_buttons, [Button::SavePreset, Button::Cancel]);
    assert_eq!(s.detail.title, "Reed Bite");
    assert_eq!(s.detail.kind, "Comes with the engine");
    assert!(s
        .detail
        .lines
        .contains(&("You had".into(), "Reed Bite (edited)".into())));
    assert_eq!(s.detail.buttons, [Button::Load, Button::NewTrackSound]);
    // Opening asks for nothing: the sound only changes when it turns.
    assert!(b.take_actions().is_empty());
    b.dial(-1);
    assert_eq!(
        b.take_actions(),
        [BrowseAction::TryFactoryPreset {
            engine: "Keys".into(),
            index: 2
        }]
    );
    b.input(Input::Button(Button::Cancel));
    assert_eq!(
        b.take_actions(),
        [
            BrowseAction::RevertPreset,
            BrowseAction::PickerClosed { loaded: false }
        ]
    );
    // Load (the push) keeps the one picked, as the side list's tap does.
    b.open_preset_picker("Keys", own.clone(), keys_now("Reed Bite", Some(3)));
    b.dial(-2);
    b.take_actions();
    b.push();
    assert_eq!(
        b.take_actions(),
        [
            BrowseAction::LoadFactoryPreset {
                engine: "Keys".into(),
                index: 1
            },
            BrowseAction::PickerClosed { loaded: true }
        ]
    );
    // One of the musician's opens on Yours, picked; the places tap across.
    b.open_preset_picker("FM", vec!["Init".into()], keys_now("Metal bass", None));
    let s = b.screen();
    assert_eq!(SOURCES[s.source].label(), "Instruments");
    assert_eq!(s.detail.title, "Metal bass");
    assert_eq!(s.detail.kind, "Your preset");
    b.input(Input::Source(src("Factory")));
    assert_eq!(names(&b), ["Init"]);
    b.input(Input::Source(src("Instruments")));
    assert_eq!(names(&b), ["Glass bell", "Metal bass", "Soft keys"]);
    // - and + step between the places, as the hint line says: + on a
    // preset goes on to the next place, - at the top of one goes back.
    let place = |b: &Browser| SOURCES[b.screen().source].label();
    b.back();
    assert_eq!(place(&b), "Factory");
    b.enter();
    assert_eq!(place(&b), "Instruments");
    b.enter();
    assert_eq!(place(&b), "Recent");
    b.back();
    assert_eq!(place(&b), "Instruments");
    assert!(b.take_actions().is_empty(), "stepping places tries nothing");
    // An engine with no presets of its own says so.
    b.cancel_picker();
    b.open_preset_picker("Plugin", Vec::new(), keys_now("", Some(0)));
    b.input(Input::Source(src("Factory")));
    assert!(b.screen().detail.kind.contains("no presets of its own"));
}

#[test]
fn new_track_with_this_sound_puts_the_tried_sound_back_and_asks_for_a_track() {
    use mpfree_browser_ui::PresetFrom;
    let (mut b, lib) = browser();
    let own: Vec<String> = ["Init", "Soft Tines", "Bright Tines", "Reed Bite"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    // One of the engine's own, tried by the wheel first: its track gets
    // its sound back, and a new track is asked for with the one picked.
    b.open_preset_picker("Keys", own, keys_now("Reed Bite", Some(3)));
    b.dial(-1);
    b.take_actions();
    b.input(Input::Button(Button::NewTrackSound));
    assert_eq!(
        b.take_actions(),
        [
            BrowseAction::RevertPreset,
            BrowseAction::NewTrackWithPreset {
                engine: "Keys".into(),
                name: "Bright Tines".into(),
                from: PresetFrom::Factory(2),
            },
            BrowseAction::PickerClosed { loaded: true },
        ]
    );
    assert!(b.picking().is_none());
    // One of the musician's, by its file; nothing tried, nothing put back.
    b.open_preset_picker("FM", vec!["Init".into()], keys_now("Metal bass", None));
    b.take_actions();
    b.input(Input::Button(Button::NewTrackSound));
    let path = lib
        .root()
        .join("Instruments")
        .join("FM")
        .join("Metal bass.preset");
    assert_eq!(
        b.take_actions(),
        [
            BrowseAction::NewTrackWithPreset {
                engine: "FM".into(),
                name: "Metal bass".into(),
                from: PresetFrom::File(path),
            },
            // Like Load, a file used goes in Recent.
            BrowseAction::SaveLists("recent = Instruments/FM/Metal bass.preset\n".into()),
            BrowseAction::PickerClosed { loaded: true },
        ]
    );
    // BROWSE's own Instruments place has no such button: the picker's.
    assert!(!b.screen().detail.buttons.contains(&Button::NewTrackSound));
}

#[test]
fn save_preset_names_the_sound_and_asks_before_replacing_one_of_yours() {
    let (mut b, lib) = browser();
    let file = |engine: &str, name: &str| {
        lib.root()
            .join("Instruments")
            .join(engine)
            .join(format!("{name}.preset"))
    };
    b.open_preset_picker("FM", vec!["Init".into()], keys_now("Glass bell", None));
    b.take_actions();
    b.input(Input::Button(Button::SavePreset));
    match b.screen().modal {
        Modal::Keyboard { title, text, .. } => {
            assert!(title.contains("Yours, for FM"), "{title}");
            assert_eq!(text, "Glass bell");
        }
        m => panic!("{m:?}"),
    }
    b.input(Input::Key(Key::Done));
    assert_eq!(
        b.take_actions(),
        [BrowseAction::SavePresetRequested {
            engine: "FM".into(),
            name: "Glass bell".into()
        }]
    );
    // The name is one of the musician's: it asks, and No leaves it.
    let before = std::fs::read(file("FM", "Glass bell")).unwrap();
    b.save_preset("FM", "Glass bell", "new text\n".into());
    assert!(matches!(b.screen().modal, Modal::Confirm { .. }));
    b.input(Input::ConfirmNo);
    assert_eq!(std::fs::read(file("FM", "Glass bell")).unwrap(), before);
    assert!(b.screen().status.unwrap().0.contains("wasn't saved"));
    assert!(b.take_actions().is_empty());
    // Yes replaces it; the old one is in the Trash.
    b.save_preset("FM", "Glass bell", "new text\n".into());
    b.input(Input::ConfirmYes);
    assert_eq!(
        std::fs::read_to_string(file("FM", "Glass bell")).unwrap(),
        "new text\n"
    );
    assert_eq!(lib.trash().unwrap()[0].name, "Glass bell.preset");
    let a = b.take_actions();
    assert!(a.iter().any(
        |x| matches!(x, BrowseAction::PresetSaved { engine, name, .. }
        if engine == "FM" && name == "Glass bell")
    ));
    let s = b.screen();
    assert_eq!(s.status.unwrap().0, "Saved Glass bell in Yours");
    assert_eq!(s.detail.title, "Glass bell");
    assert!(s
        .detail
        .lines
        .contains(&("You had".into(), "Glass bell".into())));
    // A first preset for an engine makes its folder, and shows on Yours.
    b.cancel_picker();
    b.open_preset_picker("Keys", vec!["Init".into()], keys_now("Init", Some(0)));
    b.save_preset("Keys", "Night Tines", "keys text\n".into());
    assert_eq!(
        std::fs::read_to_string(file("Keys", "Night Tines")).unwrap(),
        "keys text\n"
    );
    let s = b.screen();
    assert_eq!(SOURCES[s.source].label(), "Instruments");
    assert_eq!(names(&b), ["Night Tines"]);
    assert_eq!(s.selected, Some(0));
    // The app couldn't give the sound: said, nothing written.
    b.preset_not_saved("the synth is busy");
    assert!(b.screen().status.unwrap().1);
}
