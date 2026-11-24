//! The Sets place and the Set editor (live shows B1) from the outside: a
//! real library in a scratch folder, and what taps, the dial and the
//! buttons do to its sets.

use mpfree_browser_ui::sample::{
    fill, fill_set_files, fill_sets, sample_rig, SAMPLE_CHECKED, SAMPLE_MACHINE,
};
use mpfree_browser_ui::state::{Key, Modal, SOURCES};
use mpfree_browser_ui::CheckMark;
use mpfree_browser_ui::{
    step, BrowseAction, Browser, Input, RigThing, SetButton, SetField, SetInput, SetsScreen,
    SetsView, BROWSER,
};
use mpfree_library::preflight::{AppChecks, AppSong, Area, CheckLine, Mark};
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
            media: None,
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

    // Check set opens the check view (live shows B2): the needed keyboard
    // isn't connected, so song 1 isn't ready; song 2 is.
    press(&mut b, SetButton::CheckSet);
    let s = sets(&b);
    assert_eq!(s.view, SetsView::Check);
    let chips: Vec<String> = s
        .rows
        .iter()
        .map(|r| r.chip.as_ref().unwrap().0.clone())
        .collect();
    assert_eq!(chips, ["NOT READY", "READY"]);
    assert_eq!(s.selected, Some(0), "the song not ready is picked");
    assert!(s
        .detail
        .checks
        .iter()
        .any(|c| c.word == "Keyboards" && c.text == "Keystep 37 isn\u{2019}t connected"));
    assert!(s.detail.line.contains("92 BPM"), "{}", s.detail.line);
    // Back goes to the set's songs, where the song says why.
    tap(&mut b, SetInput::Back);
    let s = sets(&b);
    assert_eq!(s.view, SetsView::Edit);
    assert_eq!(
        s.detail.reason.as_ref().unwrap().0,
        "Keystep 37 isn\u{2019}t connected"
    );

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

/// The app's half of the check, as the app gives it for songs with no
/// plugins (mpfree-app's set_checks.rs).
struct AppHalf;

impl AppChecks for AppHalf {
    fn song(&self, _: &mpfree_library::Snapshot) -> AppSong {
        AppSong {
            lines: vec![
                CheckLine::new(Area::Plugins, Mark::Fine, "it uses none"),
                CheckLine::new(
                    Area::Outputs,
                    Mark::Fine,
                    "the stereo mix on Main out, as this MPC has it",
                ),
            ],
            plugins: 0,
        }
    }
}

/// The check view's short times: today, yesterday, else the date in full
/// (and in full for a check the clock hasn't reached).
#[test]
fn the_check_views_times_say_today_or_yesterday() {
    use mpfree_browser_ui::state::when_short;
    let at = SAMPLE_CHECKED; // 2026-11-23 19:04 UTC
    assert_eq!(when_short(at, at + 3600), "today 19:04 UTC");
    assert_eq!(when_short(at, at + 86_400), "yesterday 19:04 UTC");
    assert_eq!(when_short(at, at + 3 * 86_400), "2026-11-23 19:04 UTC");
    assert_eq!(when_short(at + 86_400, at), "2026-11-24 19:04 UTC");
}

/// Live shows B2: Check set opens the check view, each song with its chip
/// and numbers, the first song not ready picked with every check; the
/// dial picks another, Check every file reads them all in full, and Back
/// goes back to the sets.
#[test]
fn the_check_view_shows_every_songs_checks() {
    let lib = fill(&temp()).unwrap();
    fill_sets(&lib).unwrap();
    fill_set_files(&lib).unwrap();
    let mut b = Browser::inline(lib.clone());
    b.set_rig(sample_rig());
    b.set_app_checks(std::sync::Arc::new(AppHalf));
    b.set_machine(SAMPLE_MACHINE);
    b.fix_set_clock(SAMPLE_CHECKED);
    b.go_source(src("Sets"));
    let friday = row_of(&b, "Friday at the Lantern");
    tap(&mut b, SetInput::Row(friday));
    assert!(
        !sets(&b).detail.buttons.contains(&SetButton::CheckEveryFile),
        "Check every file moved to the check view"
    );
    assert!(!b.set_check_shown());
    press(&mut b, SetButton::CheckSet);
    assert!(b.set_check_shown());
    let s = sets(&b);
    assert_eq!(s.view, SetsView::Check);
    assert_eq!(s.title, "Check \u{b7} Friday at the Lantern");
    assert!(s.back);
    assert_eq!(s.head_buttons, [SetButton::CheckEveryFile]);
    let (chip, words) = s.sub.clone().unwrap();
    assert_eq!(chip.unwrap().0, "NOT READY");
    // No soundcheck yet, so the line doesn't name one (each row does).
    assert_eq!(
        words,
        "7 ready, 1 with limits, 1 not ready \u{b7} quick check today 19:04 UTC"
    );
    assert_eq!(s.rows.len(), 9);
    let chips: Vec<&str> = s
        .rows
        .iter()
        .map(|r| r.chip.as_ref().unwrap().0.as_str())
        .collect();
    assert_eq!(
        chips,
        [
            "READY",
            "READY",
            "READY",
            "READY",
            "NOT READY",
            "LIMITS",
            "READY",
            "READY",
            "READY"
        ]
    );
    // A row's numbers: its estimate until Soundcheck measures it; the
    // song not ready says why first, in red.
    assert!(
        s.rows[0].sub.starts_with("about 1 s \u{b7} about 24 MB"),
        "{}",
        s.rows[0].sub
    );
    assert!(s.rows[0].sub.ends_with("not soundchecked yet"));
    assert!(
        s.rows[4].sub.starts_with("samples/strings.wav is missing"),
        "{}",
        s.rows[4].sub
    );
    // Song 5 picked: every check, in order, the missing file in red.
    assert_eq!(s.selected, Some(4));
    let d = &s.detail;
    assert_eq!(d.eyebrow, "Song 5 of 9");
    assert_eq!(d.title, "Glass harbour");
    assert_eq!(d.chip.as_ref().unwrap().0, "NOT READY");
    assert_eq!(d.line, "110 BPM \u{b7} checked today 19:04 UTC");
    let words: Vec<&str> = d.checks.iter().map(|c| c.word.as_str()).collect();
    assert_eq!(
        words,
        [
            "Files",
            "Parts",
            "Plugins",
            "Keyboards",
            "Outputs",
            "Memory",
            "Stick",
            "Streams",
            "Soundcheck"
        ],
        "{:#?}",
        d.checks
    );
    assert_eq!(d.checks[0].mark, CheckMark::NotReady);
    assert_eq!(d.checks[0].text, "samples/strings.wav is missing");
    assert_eq!(
        d.checks[6].text,
        "11.8 GB free for takes and recovery copies"
    );
    assert_eq!(d.checks[8].mark, CheckMark::Note);
    assert_eq!(
        d.buttons,
        [SetButton::SoundcheckSong, SetButton::OpenInMake]
    );
    // The quick check's answer went into the set, as B1's does.
    let glass = &set_at(&lib, "Friday at the Lantern.set").songs[4];
    let (at, check) = glass.check.as_ref().unwrap();
    assert_eq!(*at, SAMPLE_CHECKED);
    assert_eq!(check.not_ready, ["the file samples/strings.wav is missing"]);

    // The dial picks the next song: Paper moons, with limits.
    b.dial(1);
    let s = sets(&b);
    assert_eq!(s.selected, Some(5));
    assert_eq!(s.detail.title, "Paper moons");
    assert!(s.detail.checks.iter().any(|c| c.mark == CheckMark::Limits
        && c.text == "Nord Electro isn\u{2019}t connected: it plays without it"));

    // Check every file reads each file in full; the pick stays.
    press(&mut b, SetButton::CheckEveryFile);
    let s = sets(&b);
    assert!(s.sub.unwrap().1.contains("every file read"));
    assert_eq!(s.selected, Some(5));
    assert!(s.rows[0].sub.contains("about"));

    // Back (‹ or –) goes back to the sets, Friday still picked.
    b.input(Input::Back);
    let s = sets(&b);
    assert_eq!(s.view, SetsView::List);
    assert!(!b.set_check_shown());
    assert_eq!(s.selected, Some(friday));
}

/// Soundcheck (live shows B2b): the card's Soundcheck asks first (every
/// song, Out loud or Muted, about how long), Start hands it to the app,
/// the check view follows the song it's on with its three steps, each
/// song's line goes into the set, and at the end it says how it went.
#[test]
fn soundcheck_asks_runs_and_says_how_it_went() {
    use mpfree_browser_ui::{CheckRow, SoundcheckNow, SOUNDCHECK_HINT};
    let lib = fill(&temp()).unwrap();
    fill_sets(&lib).unwrap();
    fill_set_files(&lib).unwrap();
    let mut b = Browser::inline(lib.clone());
    b.set_rig(sample_rig());
    b.set_app_checks(std::sync::Arc::new(AppHalf));
    b.set_machine(SAMPLE_MACHINE);
    b.fix_set_clock(SAMPLE_CHECKED);
    b.go_source(src("Sets"));
    let friday = row_of(&b, "Friday at the Lantern");
    tap(&mut b, SetInput::Row(friday));
    let card = sets(&b).detail.buttons;
    assert_eq!(card.len(), 7);
    assert_eq!(card[3], SetButton::Soundcheck);
    assert!(!SOUNDCHECK_HINT.is_empty());

    // The question, in the check view's Selected column.
    press(&mut b, SetButton::Soundcheck);
    assert!(b.set_check_shown());
    let d = sets(&b).detail;
    assert_eq!(d.eyebrow, "Soundcheck");
    assert_eq!(d.title, "Friday at the Lantern");
    assert_eq!(d.line, "9 songs");
    let para = d.para.clone().unwrap();
    assert!(
        para.starts_with("Opens each of the 9 songs in turn"),
        "{para}"
    );
    assert!(para.contains("minutes"), "{para}");
    assert_eq!(d.steppers[0].field, SetField::Sound);
    assert_eq!(d.steppers[0].value, "Out loud");
    assert_eq!(
        d.buttons,
        [SetButton::StartSoundcheck, SetButton::CancelSoundcheck]
    );
    tap(&mut b, SetInput::Step(SetField::Sound, 1));
    assert_eq!(sets(&b).detail.steppers[0].value, "Muted");
    // Cancel puts the song's checks back; asking again keeps Muted.
    press(&mut b, SetButton::CancelSoundcheck);
    assert_ne!(sets(&b).detail.eyebrow, "Soundcheck");
    press(&mut b, SetButton::SoundcheckSong);
    let d = sets(&b).detail;
    assert_eq!(d.title, "Glass harbour");
    assert_eq!(d.line, "Song 5 of 9");
    assert!(d.para.unwrap().contains("then its change into Paper moons"));
    assert_eq!(d.steppers[0].value, "Muted");
    let _ = b.take_actions();
    press(&mut b, SetButton::StartSoundcheck);
    let started = b
        .take_actions()
        .into_iter()
        .find_map(|a| match a {
            BrowseAction::Soundcheck {
                loc,
                set,
                only,
                muted,
            } => Some((loc, set, only, muted)),
            _ => None,
        })
        .expect("Start soundcheck hands it to the app");
    let (loc, set, only, muted) = started;
    assert_eq!(loc.name(), "Friday at the Lantern.set");
    assert_eq!((set.songs.len(), only, muted), (9, Some(4), true));

    // Running: the app says where it is, and the view follows it.
    let row = |mark, word: &str, text: &str| CheckRow {
        mark,
        word: word.into(),
        text: text.into(),
    };
    b.set_soundcheck(Some(SoundcheckNow {
        loc: loc.clone(),
        songs: vec![4],
        song: 4,
        pos: 0,
        left_s: 50,
        steps: vec![
            row(CheckMark::Fine, "Opened cold", "3.1 s \u{b7} 540 MB"),
            row(CheckMark::Now, "Busiest part", "Chorus \u{b7} bar 3 of 16"),
            row(CheckMark::Later, "Change", "into Paper moons"),
        ],
        meter: Some((48.0, 63.0)),
        meter_facts: vec![("Longest disk wait".into(), "9 ms".into())],
        row_line: "opened in 3.1 s \u{b7} 540 MB \u{b7} now playing Chorus".into(),
        out_loud: false,
    }));
    assert!(b.soundcheck_running());
    let s = sets(&b);
    assert_eq!(s.head_buttons, [SetButton::StopSoundcheck]);
    let (chip, words) = s.sub.clone().unwrap();
    assert_eq!(chip.unwrap().0, "SOUNDCHECK");
    assert_eq!(words, "song 1 of 1 \u{b7} about a minute left \u{b7} muted");
    assert!(s.rows[4].lit);
    assert_eq!(
        s.rows[4].sub,
        "opened in 3.1 s \u{b7} 540 MB \u{b7} now playing Chorus"
    );
    assert_eq!(s.rows[4].chip.as_ref().unwrap().0, "SOUNDCHECK");
    let d = &s.detail;
    assert_eq!(d.eyebrow, "Soundcheck \u{b7} song 5 of 9");
    assert_eq!(d.line, "110 BPM \u{b7} ends: Let it finish");
    assert_eq!(d.meter_at, 2);
    assert_eq!(d.after, None, "muted: nothing to warn of");
    assert_eq!(d.checks.len(), 3);
    assert_eq!(d.checks[1].mark, CheckMark::Now);
    assert_eq!(d.meter, Some((48.0, 63.0)));
    assert_eq!(d.buttons, [SetButton::StopSoundcheck, SetButton::SkipSong]);
    // While it runs, Play set and another Soundcheck wait.
    b.input(Input::Back);
    let _ = b.take_actions();
    let s = sets(&b);
    assert_eq!(s.view, SetsView::List);
    let d = s.detail;
    assert!(d.faint.contains(&SetButton::PlaySet));
    assert!(d.faint.contains(&SetButton::Soundcheck));
    press(&mut b, SetButton::CheckSet);
    press(&mut b, SetButton::SkipSong);
    press(&mut b, SetButton::StopSoundcheck);
    let acts = b.take_actions();
    assert!(acts.contains(&BrowseAction::SkipSoundcheckSong));
    assert!(acts.contains(&BrowseAction::StopSoundcheck));

    // Its line goes into the set; at the end, how it went, and the set is
    // checked again with it.
    let line = mpfree_library::Soundcheck {
        at: SAMPLE_CHECKED,
        open_ms: 3_100,
        peak_kb: 527_344,
        cpu_x10: 630,
        disk_wait_ms: 0,
        misses: 0,
        xruns: 0,
        change_ms: Some(4_200),
        part: "Chorus".into(),
    };
    b.soundchecked(&loc, 4, line.clone(), &set);
    assert_eq!(
        set_at(&lib, "Friday at the Lantern.set").songs[4].soundcheck,
        Some(line)
    );
    let went = vec![row(CheckMark::Fine, "", "1 song in 52 s. All opened.")];
    b.soundcheck_over(&loc, went.clone(), "Night drive is open again.".into());
    assert!(!b.soundcheck_running());
    let d = sets(&b).detail;
    assert_eq!(d.eyebrow, "Soundcheck");
    assert_eq!(d.checks, went);
    assert_eq!(d.after.as_ref().unwrap().0, "Night drive is open again.");
    assert_eq!(d.buttons, [SetButton::SoundcheckDone]);
    let s = sets(&b);
    assert!(
        !s.rows[4].sub.contains("not soundchecked yet"),
        "{}",
        s.rows[4].sub
    );
    press(&mut b, SetButton::SoundcheckDone);
    assert_eq!(sets(&b).detail.eyebrow, "Song 5 of 9");
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

/// Live shows B1, the browser's part of playing a set: a song opens by its
/// id without asking anything (or says why not), a song's changes made on
/// stage go into a recovery copy the set remembers, and the Set editor
/// keeps or throws away those changes afterwards.
#[test]
fn a_played_sets_songs_open_and_their_changes_on_stage_are_kept() {
    let lib = fill(&temp()).unwrap();
    fill_sets(&lib).unwrap();
    let mut b = Browser::inline(lib.clone());
    b.set_rig(sample_rig());
    let sunday_loc = Loc::top(Place::Sets).join("Sunday rehearsal.set").unwrap();
    let sunday = set_at(&lib, "Sunday rehearsal.set");
    b.set_playing(Some((sunday_loc.clone(), sunday.clone())));
    let chops = Loc::top(Place::Projects).join("Sunday chops").unwrap();

    // A song opens: the marker, then the open itself.
    let _ = b.take_actions();
    b.open_set_song(&sunday.songs[0]);
    let acts = b.take_actions();
    let k = acts
        .iter()
        .position(|a| matches!(a, BrowseAction::SetSongOpened { loc } if *loc == chops))
        .expect("the marker");
    let next_open = acts[k + 1..]
        .iter()
        .find(|a| matches!(a, BrowseAction::ProjectOpened { .. }));
    assert!(
        matches!(next_open, Some(BrowseAction::ProjectOpened { loc, .. }) if *loc == chops),
        "{acts:?}"
    );
    assert!(!b.unsaved());

    // A song that is gone says why, and nothing asks.
    let mut gone = sunday.songs[1].clone();
    gone.id = String::new();
    gone.name = "Not here".into();
    b.open_set_song(&gone);
    let acts = b.take_actions();
    assert!(
        acts.iter().any(|a| matches!(a,
            BrowseAction::SetSongFailed { song, why }
                if song == "Not here" && why.contains("isn\u{2019}t in Projects"))),
        "{acts:?}"
    );
    assert!(matches!(b.screen().modal, Modal::None));

    // Its changes on stage: a recovery copy beside it, remembered by the
    // set, which says so to the Show view.
    let mut snap = lib.open_project(&chops).unwrap();
    snap.parts = vec![("transport".into(), "bpm = 140\n".into())];
    b.on_stage_copy_with("Sunday chops", snap, Vec::new());
    let copy = b
        .take_actions()
        .into_iter()
        .find_map(|a| match a {
            BrowseAction::OnStageCopied { loc } => Some(loc),
            _ => None,
        })
        .expect("copied");
    assert_eq!(copy.name(), "Sunday chops on stage");
    b.mark_on_stage(&sunday_loc, 0, copy.name());
    let acts = b.take_actions();
    assert!(acts.iter().any(|a| matches!(a,
        BrowseAction::SetChanged { loc, set }
            if *loc == sunday_loc && set.songs[0].on_stage == [copy.name().to_string()])));
    assert_eq!(
        set_at(&lib, "Sunday rehearsal.set").songs[0].on_stage,
        [copy.name().to_string()]
    );
    b.set_playing(None);

    // After the show, the Set editor says it changed on stage and keeps it.
    b.go_source(src("Sets"));
    let r = row_of(&b, "Sunday rehearsal");
    tap(&mut b, SetInput::Row(r));
    press(&mut b, SetButton::EditSet);
    tap(&mut b, SetInput::Row(0));
    let s = sets(&b);
    assert_eq!(s.rows[0].sub, "changed on stage");
    assert_eq!(s.detail.lists[0].0, "Changed on stage");
    assert_eq!(
        &s.detail.buttons[..2],
        [SetButton::KeepChanges, SetButton::ThrowChanges]
    );
    press(&mut b, SetButton::KeepChanges);
    assert_eq!(
        lib.open_project(&chops).unwrap().part("transport"),
        Some("bpm = 140\n")
    );
    assert!(lib.project_meta(&copy).is_err(), "the copy is in the Trash");
    assert!(set_at(&lib, "Sunday rehearsal.set").songs[0]
        .on_stage
        .is_empty());
    assert_ne!(sets(&b).rows[0].sub, "changed on stage");

    // Play set over unsaved work asks first.
    b.set_unsaved(true);
    tap(&mut b, SetInput::Back);
    let _ = b.take_actions();
    press(&mut b, SetButton::PlaySet);
    assert!(matches!(b.screen().modal, Modal::Confirm { .. }));
    assert!(!b
        .take_actions()
        .iter()
        .any(|a| matches!(a, BrowseAction::PlaySet { .. })));
    b.input(Input::ConfirmYes);
    assert!(b
        .take_actions()
        .iter()
        .any(|a| matches!(a, BrowseAction::PlaySet { .. })));
}
