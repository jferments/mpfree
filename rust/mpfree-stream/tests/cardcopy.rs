//! Checked copies from the SD card onto the stick: whole or not at all
//! after a cut at any step, stopped by a pulled card and never resumed,
//! refused on a nearly full stick, named by content so an older version
//! stays, and paced behind playback and recording.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use mpfree_stream::cardcopy::{background, copy, Checked, Copy, Pace, Refused};
use mpfree_stream::fsio::{Fs, StdFs};
use mpfree_stream::policy::Device as Place;
use mpfree_stream::publish::sweep_parts;
use mpfree_stream::storage::{Located, Storage};
use mpfree_stream::testkit::{folder, Cut, ModelStick, Names, PretendCard};
use mpfree_stream::writer::Gate;

/// Bytes a little over four copy blocks, the same every run.
fn content(seed: u32) -> Vec<u8> {
    (0..1_100_000u32)
        .map(|i| (i.wrapping_mul(31).wrapping_add(seed) % 251) as u8)
        .collect()
}

/// A card folder holding `song.wav`, inserted.
fn card(name: &str, bytes: &[u8]) -> (Storage, std::sync::Arc<PretendCard>, PathBuf) {
    let dir = folder(name);
    let (stick, card) = (dir.join("stick"), dir.join("card"));
    std::fs::create_dir_all(&stick).expect("stick");
    std::fs::create_dir_all(&card).expect("card");
    std::fs::write(card.join("song.wav"), bytes).expect("song");
    let mut s = Storage::new(&stick);
    let h = PretendCard::new(&card);
    s.card_inserted(h.clone());
    (s, h, card)
}

const MANAGED: &str = "/m/copies";

/// Copy `loc` onto `stick` with plenty of room, going at every block.
fn run(stick: &mut ModelStick, loc: &Located, job: u32) -> Result<Checked, Refused> {
    let mut src = StdFs::read_only();
    let mut pace = || Pace::Go;
    copy(
        &mut Copy {
            src: &mut src,
            fs: stick,
            dir: Path::new(MANAGED),
            job,
            free: 1 << 30,
            keep: 0,
            pace: &mut pace,
        },
        loc,
    )
}

fn parts(stick: &ModelStick) -> Vec<PathBuf> {
    stick
        .names()
        .into_iter()
        .filter(|p| p.to_string_lossy().ends_with(".part"))
        .collect()
}

#[test]
fn a_checked_copy_is_named_by_content_and_keeps_older_versions() {
    let first = content(1);
    let (mut s, handle, card) = card("copy-named", &first);
    let loc = s.locate(Place::Card, Path::new("song.wav")).expect("song");
    let mut stick = ModelStick::new();
    let a = run(&mut stick, &loc, 1).expect("copies");
    assert!(!a.existed);
    assert_eq!(stick.bytes(&a.path), Some(&first[..]));
    let name = a.path.file_name().unwrap().to_string_lossy().into_owned();
    assert!(
        name.ends_with("-song.wav") && name.len() == 16 + 1 + 8,
        "{name}"
    );
    assert!(parts(&stick).is_empty());

    // Again: the same content is already there, and used.
    let again = run(&mut stick, &loc, 2).expect("copies");
    assert!(again.existed && again.path == a.path && again.hash == a.hash);

    // A computer changes the file on the card; the card comes back as a
    // new insertion. The new version is a new copy; the old one stays.
    handle.pull();
    s.card_removed();
    let second = content(2);
    std::fs::write(card.join("song.wav"), &second).expect("song");
    s.card_inserted(PretendCard::new(&card));
    let loc2 = s.locate(Place::Card, Path::new("song.wav")).expect("song");
    let b = run(&mut stick, &loc2, 3).expect("copies");
    assert_ne!(b.path, a.path);
    assert_eq!(stick.bytes(&b.path), Some(&second[..]));
    assert_eq!(
        stick.bytes(&a.path),
        Some(&first[..]),
        "the old version stays"
    );
}

#[test]
fn a_copy_interrupted_at_every_step_is_whole_or_absent() {
    let bytes = content(3);
    let (s, _handle, _) = card("copy-cut", &bytes);
    let loc = s.locate(Place::Card, Path::new("song.wav")).expect("song");
    let mut dry = ModelStick::new();
    let done = run(&mut dry, &loc, 7).expect("copies");
    assert!(dry.ops > 6, "create, writes, sync, rename, folder sync");
    for n in 0..=dry.ops {
        let mut stick = ModelStick::new();
        stick.cut_after = Some(n);
        let r = run(&mut stick, &loc, 7);
        if n < dry.ops {
            assert!(r.is_err(), "the power went after op {n}");
        }
        for how in [Cut::Lost, Cut::Landed, Cut::Torn(0), Cut::Torn(3)] {
            for names in [Names::Lost, Names::Landed, Names::LastOnly] {
                let mut after = stick.cut(how, names, Default::default());
                if let Some(b) = after.bytes(&done.path) {
                    assert_eq!(b, &bytes[..], "cut after op {n}, {how:?}, {names:?}");
                }
                // At the next start a part file left is removed, never
                // finished; the copy starts again from the beginning.
                sweep_parts(&mut after, Path::new(MANAGED)).expect("sweeps");
                assert!(parts(&after).is_empty(), "op {n}, {how:?}, {names:?}");
                let redo = run(&mut after, &loc, 8).expect("copies again");
                assert_eq!(redo.path, done.path);
                assert_eq!(after.bytes(&done.path), Some(&bytes[..]));
            }
        }
    }
}

#[test]
fn a_card_pulled_mid_copy_stops_it_and_a_new_card_never_resumes_it() {
    let bytes = content(4);
    let (mut s, handle, card) = card("copy-pulled", &bytes);
    let loc = s.locate(Place::Card, Path::new("song.wav")).expect("song");
    let mut stick = ModelStick::new();
    let mut src = StdFs::read_only();
    let mut blocks = 0;
    let mut pace = || {
        blocks += 1;
        if blocks == 3 {
            handle.pull();
        }
        Pace::Go
    };
    let r = copy(
        &mut Copy {
            src: &mut src,
            fs: &mut stick,
            dir: Path::new(MANAGED),
            job: 1,
            free: 1 << 30,
            keep: 0,
            pace: &mut pace,
        },
        &loc,
    );
    assert_eq!(r.unwrap_err(), Refused::CardGone);
    assert!(
        stick.names().is_empty(),
        "nothing left: {:?}",
        stick.names()
    );
    s.card_removed();

    // What was found on the pulled card can't be copied; found again on
    // the new insertion, the copy starts from the beginning.
    s.card_inserted(PretendCard::new(&card));
    assert_eq!(run(&mut stick, &loc, 2).unwrap_err(), Refused::CardGone);
    let loc2 = s.locate(Place::Card, Path::new("song.wav")).expect("song");
    let done = run(&mut stick, &loc2, 3).expect("copies");
    assert_eq!(stick.bytes(&done.path), Some(&bytes[..]));
    assert_eq!(stick.names(), vec![done.path]);
}

#[test]
fn a_nearly_full_stick_refuses_before_writing_and_cleans_up_if_it_fills() {
    let bytes = content(5);
    let (s, _handle, _) = card("copy-full", &bytes);
    let loc = s.locate(Place::Card, Path::new("song.wav")).expect("song");
    let size = bytes.len() as u64;

    // Not enough room beside what is kept: nothing is touched.
    let mut stick = ModelStick::new();
    let mut src = StdFs::read_only();
    let mut pace = || Pace::Go;
    let r = copy(
        &mut Copy {
            src: &mut src,
            fs: &mut stick,
            dir: Path::new(MANAGED),
            job: 1,
            free: size + 400_000_000,
            keep: 512_000_000,
            pace: &mut pace,
        },
        &loc,
    );
    let e = r.unwrap_err();
    assert_eq!(
        e,
        Refused::Space {
            need: size,
            free: size + 400_000_000,
            keep: 512_000_000
        }
    );
    assert_eq!(
        e.to_string(),
        "not enough room on the stick: the copy needs about 2 MB, 402 MB free, 512 MB kept for \
         recording and saves"
    );
    assert_eq!(stick.ops, 0);

    // The stick fills anyway (another writer took the room): the part
    // file goes and nothing is published.
    let mut stick = ModelStick::new();
    stick.capacity = Some(size / 2);
    let r = run(&mut stick, &loc, 2);
    assert!(matches!(r, Err(Refused::Io(_))), "{r:?}");
    assert!(stick.names().is_empty(), "{:?}", stick.names());
}

#[test]
fn copies_wait_behind_playback_and_recording_and_can_be_stopped() {
    let bytes = content(6);
    let (s, _handle, _) = card("copy-pace", &bytes);
    let loc = s.locate(Place::Card, Path::new("song.wav")).expect("song");

    // Waits are asked again; the copy finishes whole.
    let mut stick = ModelStick::new();
    let mut src = StdFs::read_only();
    let mut asked = 0;
    let mut pace = || {
        asked += 1;
        if asked % 2 == 1 {
            Pace::Wait
        } else {
            Pace::Go
        }
    };
    let done = copy(
        &mut Copy {
            src: &mut src,
            fs: &mut stick,
            dir: Path::new(MANAGED),
            job: 1,
            free: 1 << 30,
            keep: 0,
            pace: &mut pace,
        },
        &loc,
    )
    .expect("copies");
    assert_eq!(stick.bytes(&done.path), Some(&bytes[..]));
    assert!(asked >= 10, "asked before every block: {asked}");

    // Stopped part way: nothing left.
    let mut stick = ModelStick::new();
    let mut asked = 0;
    let mut pace = || {
        asked += 1;
        if asked > 2 {
            Pace::Stop
        } else {
            Pace::Go
        }
    };
    let r = copy(
        &mut Copy {
            src: &mut src,
            fs: &mut stick,
            dir: Path::new(MANAGED),
            job: 2,
            free: 1 << 30,
            keep: 0,
            pace: &mut pace,
        },
        &loc,
    );
    assert_eq!(r.unwrap_err(), Refused::Stopped);
    assert!(stick.names().is_empty());

    // Behind recording and playback.
    let gate = Gate::new();
    assert_eq!(background(&gate, 2.0), Pace::Go, "nothing playing");
    gate.set_cushion(1.0);
    assert_eq!(
        background(&gate, 2.0),
        Pace::Wait,
        "playback's cushion is short"
    );
    gate.set_cushion(5.0);
    assert_eq!(background(&gate, 2.0), Pace::Go);
    gate.set_capture_fill(0.8);
    assert_eq!(background(&gate, 2.0), Pace::Wait, "recording presses");
}

#[test]
fn only_part_files_are_swept() {
    let mut stick = ModelStick::new();
    let dir = Path::new(MANAGED);
    for n in [
        "a.wav",
        "a.wav.3.part",
        "b.wav.12.part",
        "c.part",
        "d.wav.x.part",
    ] {
        stick.create(&dir.join(n), true).expect("made");
    }
    assert_eq!(sweep_parts(&mut stick, dir).expect("sweeps"), 2);
    let left: Vec<String> = stick
        .names()
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(left, ["a.wav", "c.part", "d.wav.x.part"]);
}

#[test]
fn the_card_is_read_through_a_read_only_layer_and_copies_are_read_back_fresh() {
    let dir = folder("copy-layers");
    let p = dir.join("x.bin");
    let bytes = content(7);
    std::fs::write(&p, &bytes).expect("written");
    let mut ro = StdFs::read_only();
    let mut buf = vec![0u8; 10_000];
    assert_eq!(ro.read_at(&p, 4096, &mut buf).expect("reads"), 10_000);
    assert_eq!(buf, bytes[4096..14_096]);
    assert_eq!(
        ro.write_at(&p, 0, b"x").unwrap_err().kind(),
        ErrorKind::PermissionDenied
    );
    assert_eq!(
        ro.create(&dir.join("new"), true).unwrap_err().kind(),
        ErrorKind::PermissionDenied
    );
    assert_eq!(
        ro.rename(&p, &dir.join("y")).unwrap_err().kind(),
        ErrorKind::PermissionDenied
    );
    assert_eq!(
        ro.remove(&p).unwrap_err().kind(),
        ErrorKind::PermissionDenied
    );
    assert_eq!(std::fs::read(&p).expect("still there"), bytes);

    // A fresh read gives the same bytes: past the cache where the file
    // system allows, through it (counted) where it doesn't, or when the
    // read isn't aligned.
    let mut fs = StdFs::new();
    let mut fresh = vec![0u8; 300_000];
    assert_eq!(fs.read_fresh(&p, 0, &mut fresh).expect("reads"), 300_000);
    assert_eq!(fresh, bytes[..300_000]);
    let tail = bytes.len() - 1_048_576;
    assert_eq!(
        fs.read_fresh(&p, 1_048_576, &mut fresh).expect("reads"),
        tail
    );
    assert_eq!(fresh[..tail], bytes[1_048_576..]);
    let before = fs.fresh_fallbacks();
    assert_eq!(fs.read_fresh(&p, 100, &mut buf).expect("reads"), 10_000);
    assert_eq!(buf, bytes[100..10_100]);
    assert_eq!(
        fs.fresh_fallbacks(),
        before + 1,
        "an unaligned read is counted"
    );
}

#[test]
fn a_copy_onto_a_real_folder() {
    let bytes = content(8);
    let (s, _handle, _) = card("copy-real", &bytes);
    let loc = s.locate(Place::Card, Path::new("song.wav")).expect("song");
    let managed = folder("copy-real-managed");
    let mut src = StdFs::read_only();
    let mut fs = StdFs::new();
    let mut pace = || Pace::Go;
    let done = copy(
        &mut Copy {
            src: &mut src,
            fs: &mut fs,
            dir: &managed,
            job: 1,
            free: 1 << 30,
            keep: 0,
            pace: &mut pace,
        },
        &loc,
    )
    .expect("copies");
    assert_eq!(std::fs::read(&done.path).expect("copied"), bytes);
    let names: Vec<_> = std::fs::read_dir(&managed)
        .expect("listed")
        .map(|e| e.expect("entry").file_name())
        .collect();
    assert_eq!(names, [done.path.file_name().expect("name").to_owned()]);
}
