//! The browser from the outside: what taps, the dial and the buttons do to
//! a real library in a scratch folder.

use mpfree_browser_ui::sample::{fill, samples, FACTORY};
use mpfree_browser_ui::state::{Key, Modal, SOURCES};
use mpfree_browser_ui::{step, BrowseAction, Browser, BrowserView, Button, Filter, Input, BROWSER};
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

fn select(b: &mut Browser, name: &str) {
    let s = b.screen();
    let i = s.rows.iter().position(|r| r.name == name).unwrap();
    if s.selected != Some(i) {
        b.input(Input::Row(i));
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
fn folders_open_with_a_second_tap_or_the_push_and_minus_goes_back() {
    let (mut b, _) = browser();
    select(&mut b, "Drums");
    b.input(Input::Row(0)); // a tap on the selected row opens it
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
    // No New folder in Projects: projects are made by saving.
    assert!(b.screen().list_buttons.is_empty());
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
    tap(&mut ui, x, y0 + 52);
    step(&mut ui, &mut b);
    assert_eq!(b.selected_row().unwrap().name, "Field recordings");
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
