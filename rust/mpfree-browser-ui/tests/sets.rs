//! The Sets place and the Set editor (live shows B1) from the outside: a
//! real library in a scratch folder, and what taps, the dial and the
//! buttons do to its sets.

use mpfree_browser_ui::sample::{fill, fill_sets, sample_rig};
use mpfree_browser_ui::state::{Key, Modal, SOURCES};
use mpfree_browser_ui::{
    step, BrowseAction, Browser, Input, RigThing, SetButton, SetField, SetInput, SetsScreen,
    SetsView, BROWSER,
};
use mpfree_library::{Ends, Library, Loc, Need, Place, Set, SetSong, Starts};
use mpfree_ui::{Fonts, SoftCanvas, Touch};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

fn temp() -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let d = std::env::temp_dir().join(format!(
        "mpfree-sets-{}-{}",
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

fn sets(b: &Browser) -> SetsScreen {
    b.screen().sets.expect("the Sets place shows")
}

fn names(b: &Browser) -> Vec<String> {
    sets(b).rows.iter().map(|r| r.name.clone()).collect()
}

fn tap(b: &mut Browser, i: SetInput) {
    b.input(Input::Sets(i));
}

fn press(b: &mut Browser, button: SetButton) {
    tap(b, SetInput::Button(button));
}

fn row_of(b: &Browser, name: &str) -> usize {
    names(b).iter().position(|n| n == name).unwrap()
}

fn type_text(b: &mut Browser, text: &str) {
    for c in text.chars() {
        b.input(Input::Key(if c == ' ' { Key::Space } else { Key::Char(c) }));
    }
    b.input(Input::Key(Key::Done));
}

fn set_at(lib: &Library, name: &str) -> Set {
    lib.load_set(&Loc::top(Place::Sets).join(name).unwrap())
        .unwrap()
}

#[test]
fn sets_is_a_place_after_projects() {
    assert_eq!(SOURCES[0].label(), "Projects");
    assert_eq!(SOURCES[1].label(), "Sets");
    assert_eq!(
        SOURCES[mpfree_browser_ui::state::SAMPLES].label(),
        "Samples"
    );
    let lib = fill(&temp()).unwrap();
    let b = Browser::inline(lib);
    // The full browser still opens on Samples.
    assert_eq!(b.screen().path, "Samples");
    assert!(b.screen().sets.is_none());
}

#[test]
fn a_new_set_is_named_filled_edited_and_saves_itself_after_each_change() {
    let lib = fill(&temp()).unwrap();
    let mut b = Browser::inline(lib.clone());
    b.go_source(src("Sets"));
    let s = sets(&b);
    assert_eq!(s.view, SetsView::List);
    assert!(s.rows.is_empty());
    assert_eq!(s.empty, "No sets yet: New set makes one");

    // New set asks for a name, then opens the empty set.
    press(&mut b, SetButton::NewSet);
    assert!(matches!(b.screen().modal, Modal::Keyboard { .. }));
    type_text(&mut b, "friday");
    assert_eq!(b.screen().modal, Modal::None);
    let s = sets(&b);
    assert_eq!(s.view, SetsView::Edit);
    assert_eq!(s.title, "Friday");
    assert!(s.back);
    assert_eq!(s.head_buttons, [SetButton::AddSongs, SetButton::CheckSet]);
    assert!(set_at(&lib, "Friday.set").songs.is_empty());

    // Add songs lists Projects; a tap on + puts the song after the one
    // picked, and a song can be in a set twice.
    press(&mut b, SetButton::AddSongs);
    assert_eq!(sets(&b).view, SetsView::Add);
    assert_eq!(names(&b), ["Live set", "Night drive", "Sunday chops"]);
    let night = row_of(&b, "Night drive");
    tap(&mut b, SetInput::Add(night));
    let sunday = row_of(&b, "Sunday chops");
    tap(&mut b, SetInput::Add(sunday));
    // The push adds the project picked.
    b.dial(-1);
    b.push();
    assert_eq!(sets(&b).rows[night].sub, "in the set 2 times");
    assert_eq!(
        b.screen().status.unwrap().0,
        "Added Night drive as song 3 of 3"
    );
    // - is Done.
    b.back();
    let s = sets(&b);
    assert_eq!(s.view, SetsView::Edit);
    assert_eq!(names(&b), ["Night drive", "Sunday chops", "Night drive"]);
    assert_eq!(s.selected, Some(2));
    assert_eq!(s.rows[0].sub, "Stopped \u{b7} Let it finish");
    assert_eq!(s.rows[0].chip.as_ref().unwrap().0, "NOT CHECKED");
    let ids: Vec<String> = set_at(&lib, "Friday.set")
        .songs
        .iter()
        .map(|s| s.id.clone())
        .collect();
    assert!(ids.iter().all(|id| id.len() == 32), "{ids:?}");
    assert_eq!(ids[0], ids[2]);

    // The picked song's values: each change is saved at once.
    tap(&mut b, SetInput::Row(0));
    assert_eq!(sets(&b).detail.eyebrow, "Song 1 of 3");
    tap(&mut b, SetInput::Step(SetField::Starts, 1));
    tap(&mut b, SetInput::Step(SetField::Ends, 1));
    tap(&mut b, SetInput::Step(SetField::Fade, 1));
    let song = &set_at(&lib, "Friday.set").songs[0];
    assert_eq!(song.starts, Starts::Playing);
    assert_eq!(song.ends, Ends::Fade(5));
    // Fade applies only while Ends is Fade.
    tap(&mut b, SetInput::Step(SetField::Ends, 1));
    let fade = sets(&b).detail.steppers[2].clone();
    assert!(fade.faint);
    assert_eq!(fade.value, "\u{2014}");
    assert_eq!(set_at(&lib, "Friday.set").songs[0].ends, Ends::Cut);
    // The push walks the values, and the dial turns the one it is on (a
    // tap on a value is on it too; picking a song lets go).
    tap(&mut b, SetInput::Row(0));
    b.push();
    b.dial(1);
    assert_eq!(set_at(&lib, "Friday.set").songs[0].starts, Starts::CountIn);
    b.push();
    b.dial(-1);
    assert_eq!(set_at(&lib, "Friday.set").songs[0].ends, Ends::Fade(4));
    b.push();
    b.dial(3);
    assert_eq!(set_at(&lib, "Friday.set").songs[0].ends, Ends::Fade(7));
    b.push();
    assert!(sets(&b).detail.steppers.iter().all(|s| !s.focused));

    // Notes on the text keyboard: up to 240 characters.
    tap(&mut b, SetInput::Notes);
    type_text(&mut b, "f minor, count in 4");
    assert_eq!(
        set_at(&lib, "Friday.set").songs[0].notes,
        "F minor, count in 4"
    );
    tap(&mut b, SetInput::Notes);
    type_text(&mut b, &"abcdefghij".repeat(30));
    assert_eq!(
        set_at(&lib, "Friday.set").songs[0].notes.chars().count(),
        240
    );

    // + and - move the picked song; Move up is faint on the first.
    assert!(sets(&b).detail.faint.contains(&SetButton::MoveUp));
    b.enter();
    assert_eq!(names(&b), ["Sunday chops", "Night drive", "Night drive"]);
    assert_eq!(sets(&b).selected, Some(1));
    assert_eq!(set_at(&lib, "Friday.set").songs[1].starts, Starts::CountIn);
    b.back();
    assert_eq!(names(&b), ["Night drive", "Sunday chops", "Night drive"]);

    // Remove asks with a second tap.
    tap(&mut b, SetInput::Row(1));
    press(&mut b, SetButton::Remove);
    assert!(sets(&b).detail.buttons.contains(&SetButton::ConfirmRemove));
    press(&mut b, SetButton::ConfirmRemove);
    assert_eq!(names(&b), ["Night drive", "Night drive"]);
    assert_eq!(set_at(&lib, "Friday.set").songs.len(), 2);
    // Another tap first takes the question back.
    press(&mut b, SetButton::Remove);
    tap(&mut b, SetInput::Row(0));
    assert!(sets(&b).detail.buttons.contains(&SetButton::Remove));

    // ‹ goes back to the sets, which show what changed.
    tap(&mut b, SetInput::Back);
    let s = sets(&b);
    assert_eq!(s.view, SetsView::List);
    assert_eq!(names(&b), ["Friday"]);
    assert_eq!(s.rows[0].sub, "2 songs");
    assert_eq!(s.rows[0].chip.as_ref().unwrap().0, "NOT CHECKED");
    // The push and a second tap are Edit set.
    b.push();
    assert_eq!(sets(&b).view, SetsView::Edit);
    tap(&mut b, SetInput::Back);
    tap(&mut b, SetInput::Row(0));
    assert_eq!(sets(&b).view, SetsView::Edit);
}

#[test]
fn a_songs_needs_come_from_the_rig_and_the_check_marks_every_song() {
    let lib = fill(&temp()).unwrap();
    for name in ["Night drive", "Live set"] {
        lib.update_manifest(&Loc::top(Place::Projects).join(name).unwrap())
            .unwrap();
    }
    let id = |name: &str| {
        lib.project_meta(&Loc::top(Place::Projects).join(name).unwrap())
            .unwrap()
            .id
    };
    let loc = lib.new_set("Friday").unwrap();
    lib.save_set(
        &loc,
        &Set {
            songs: vec![
                SetSong::new(&id("Night drive"), "Night drive"),
                SetSong::new(&id("Live set"), "Live set"),
            ],
            extra: Vec::new(),
        },
    )
    .unwrap();
    let mut b = Browser::inline(lib.clone());
    b.set_rig(vec![
        RigThing {
            name: "Keystep 37".into(),
            connected: false,
        },
        RigThing {
            name: "FS-6".into(),
            connected: true,
        },
    ]);
    b.go_source(src("Sets"));
    press(&mut b, SetButton::EditSet);

    // Needs lists the rig: a tap goes Needed, Nice to have, not needed.
    tap(&mut b, SetInput::Step(SetField::Needs, 1));
    let needs = sets(&b).detail.needs.unwrap();
    assert_eq!(needs.len(), 2);
    assert_eq!(needs[0].name, "Keystep 37");
    assert_eq!(needs[0].connected, "not connected");
    tap(&mut b, SetInput::Need(0));
    tap(&mut b, SetInput::Need(1));
    tap(&mut b, SetInput::Need(1));
    let song = &set_at(&lib, "Friday.set").songs[0];
    assert_eq!(
        song.needs,
        [
            Need {
                name: "Keystep 37".into(),
                needed: true
            },
            Need {
                name: "FS-6".into(),
                needed: false
            }
        ]
    );
    let needs = sets(&b).detail.needs.unwrap();
    assert_eq!(needs[0].state, "Needed");
    assert_eq!(needs[1].state, "Nice to have");
    press(&mut b, SetButton::NeedsDone);
    assert_eq!(sets(&b).detail.steppers[3].value, "Keystep 37 and 1 more");

    // Check set: the needed keyboard isn't connected, so song 1 isn't
    // ready; song 2 is.
    press(&mut b, SetButton::CheckSet);
    let s = sets(&b);
    let chips: Vec<String> = s
        .rows
        .iter()
        .map(|r| r.chip.as_ref().unwrap().0.clone())
        .collect();
    assert_eq!(chips, ["NOT READY", "READY"]);
    assert_eq!(
        s.detail.reason.as_ref().unwrap().0,
        "Keystep 37 isn\u{2019}t connected"
    );
    assert!(s.detail.line.contains("92 BPM"), "{}", s.detail.line);

    // The check is kept with the set: the place shows it after a restart.
    let mut again = Browser::inline(lib.clone());
    again.go_source(src("Sets"));
    let s = sets(&again);
    assert_eq!(s.rows[0].chip.as_ref().unwrap().0, "NOT READY");
    let not_ready = s
        .detail
        .lists
        .iter()
        .find(|(h, _, _)| h == "Not ready")
        .unwrap();
    assert_eq!(
        not_ready.2,
        ["1 Night drive: Keystep 37 isn\u{2019}t connected."]
    );
    assert!(s
        .detail
        .facts
        .iter()
        .any(|(_, v, _)| v == "1 ready, 1 not ready"));

    // A song renamed in Projects is found by its id, and the set takes
    // its new name at the next check.
    let live = Loc::top(Place::Projects).join("Live set").unwrap();
    lib.rename(&live, "Live set (encore)").unwrap();
    b.set_rig(vec![RigThing {
        name: "Keystep 37".into(),
        connected: true,
    }]);
    press(&mut b, SetButton::CheckSet);
    assert_eq!(names(&b), ["Night drive", "Live set (encore)"]);
    let song = &set_at(&lib, "Friday.set").songs[1];
    assert_eq!(song.name, "Live set (encore)");
    assert!(song.check.as_ref().unwrap().1.ready());
}

#[test]
fn rename_copy_delete_and_play_a_set() {
    let lib = fill(&temp()).unwrap();
    fill_sets(&lib).unwrap();
    let mut b = Browser::inline(lib.clone());
    b.set_rig(sample_rig());
    b.go_source(src("Sets"));
    assert_eq!(
        names(&b),
        [
            "Festival short set",
            "Friday at the Lantern",
            "Sunday rehearsal"
        ]
    );
    let chips: Vec<String> = sets(&b)
        .rows
        .iter()
        .map(|r| r.chip.as_ref().unwrap().0.clone())
        .collect();
    assert_eq!(chips, ["NOT CHECKED", "NOT READY", "READY"]);

    // Rename: the name typed, .set kept.
    tap(&mut b, SetInput::Row(2));
    press(&mut b, SetButton::Rename);
    for _ in 0..20 {
        b.input(Input::Key(Key::Backspace));
    }
    type_text(&mut b, "Sunday soundcheck");
    let s = sets(&b);
    assert_eq!(s.rows[s.selected.unwrap()].name, "Sunday soundcheck");
    assert!(lib
        .sets()
        .unwrap()
        .iter()
        .any(|e| e.name == "Sunday soundcheck.set"));

    // Copy: beside it, under a free name.
    press(&mut b, SetButton::Copy);
    assert!(names(&b).contains(&"Sunday soundcheck 2".to_string()));

    // Play set hands the set to the app.
    let _ = b.take_actions();
    press(&mut b, SetButton::PlaySet);
    let played = b
        .take_actions()
        .into_iter()
        .find_map(|a| match a {
            BrowseAction::PlaySet { loc, set } => Some((loc, set)),
            _ => None,
        })
        .unwrap();
    assert_eq!(played.1.songs.len(), 3);

    // Delete asks first, then the set goes to the Trash; its songs stay.
    let before = names(&b).len();
    b.erase();
    assert!(matches!(b.screen().modal, Modal::Confirm { .. }));
    b.input(Input::ConfirmYes);
    assert_eq!(names(&b).len(), before - 1);
    assert!(lib
        .project_meta(&Loc::top(Place::Projects).join("Sunday chops").unwrap())
        .is_ok());
}

#[test]
fn the_sets_place_draws_and_answers_touches() {
    let lib = fill(&temp()).unwrap();
    fill_sets(&lib).unwrap();
    let mut b = Browser::inline(lib);
    b.set_rig(sample_rig());
    let mut ui = mpfree_ui::Ui::new(mpfree_ui::Theme::dark(), Fonts::embedded(), 1280, 800);
    ui.add("header", mpfree_ui::widgets::Header::app("Play"));
    mpfree_browser_ui::add(&mut ui, &b).unwrap();
    ui.show(mpfree_browser_ui::BROWSER_PAGE).unwrap();
    b.go_source(src("Sets"));
    step(&mut ui, &mut b);
    let tap_on = |ui: &mut mpfree_ui::Ui, b: &mut Browser, id: &str, label: &str| {
        let t = ui
            .targets()
            .into_iter()
            .find(|t| t.widget == BROWSER && t.target.id == id && t.target.label == label)
            .unwrap_or_else(|| panic!("no {id} {label}"))
            .target;
        let (x, y) = t.centre();
        ui.touch(1, Touch::Down, x, y);
        ui.touch(1, Touch::Up, x, y);
        step(ui, b);
        let mut c = SoftCanvas::new(1280, 800);
        assert!(!ui.paint(&mut c).is_empty());
    };
    tap_on(&mut ui, &mut b, "sets-row", "Friday at the Lantern");
    assert_eq!(sets(&b).selected, Some(1));
    tap_on(&mut ui, &mut b, "sets-button", "Edit set");
    assert_eq!(sets(&b).view, SetsView::Edit);
    tap_on(&mut ui, &mut b, "sets-row", "Night drive");
    tap_on(&mut ui, &mut b, "sets-value", "Starts \u{203a}");
    assert_eq!(sets(&b).detail.steppers[0].value, "Playing");
    tap_on(&mut ui, &mut b, "sets-value", "Needs \u{203a}");
    assert!(sets(&b).detail.needs.is_some());
    tap_on(&mut ui, &mut b, "sets-need", "Nord Electro");
    tap_on(&mut ui, &mut b, "sets-button", "Done");
    tap_on(&mut ui, &mut b, "sets-button", "Add songs");
    assert_eq!(sets(&b).view, SetsView::Add);
    tap_on(&mut ui, &mut b, "sets-add", "Add Undertow");
    assert_eq!(sets(&b).detail.lists[0].0, "10 songs");
    tap_on(&mut ui, &mut b, "sets-button", "Done");
    tap_on(&mut ui, &mut b, "sets-notes", "Notes");
    assert!(matches!(b.screen().modal, Modal::Keyboard { .. }));
    b.input(Input::Key(Key::Cancel));
    step(&mut ui, &mut b);
    tap_on(
        &mut ui,
        &mut b,
        "sets-back",
        "\u{2039} Friday at the Lantern",
    );
    assert_eq!(sets(&b).view, SetsView::List);
}
