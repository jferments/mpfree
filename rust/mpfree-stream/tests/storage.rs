//! The storage manager: paths kept inside a library, insertion numbers,
//! and what happens when the SD card is pulled while it is being read.

use std::collections::VecDeque;
use std::os::unix::fs::{symlink, MetadataExt};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use mpfree_stream::coord::{Coordinator, Device, Inline, JobTier, Plan, ReadReply, ReadReq};
use mpfree_stream::cursor::Cursor;
use mpfree_stream::player::{Config, Player};
use mpfree_stream::policy::Device as Place;
use mpfree_stream::storage::{CardHandle, Lease, Refusal, Storage};
use mpfree_stream::testkit::{folder, played, wav, PretendCard, Store};

/// A stick folder and a card folder, each with a marked sound.
fn libraries(name: &str, frames: u64) -> (std::path::PathBuf, std::path::PathBuf) {
    let dir = folder(name);
    let (stick, card) = (dir.join("stick"), dir.join("card"));
    std::fs::create_dir_all(stick.join("drums")).expect("stick");
    std::fs::create_dir_all(&card).expect("card");
    wav(&stick.join("drums/loop.wav"), Store::Pcm24, 2, frames, 5);
    wav(&card.join("song.wav"), Store::Float, 2, frames, 3);
    (stick, card)
}

#[test]
fn paths_stay_inside_the_library() {
    let (stick, card) = libraries("storage-paths", 1_000);
    let outside = stick.parent().expect("parent").join("outside.wav");
    wav(&outside, Store::Float, 2, 1_000, 9);
    symlink(&outside, stick.join("escape.wav")).expect("link");
    symlink(stick.parent().expect("parent"), stick.join("up")).expect("link");
    let mut s = Storage::new(&stick);

    let ok = s
        .locate(Place::Stick, Path::new("drums/loop.wav"))
        .expect("a file");
    assert_eq!(ok.path, stick.join("drums/loop.wav"));
    assert!(ok.valid() && ok.card.is_none());
    assert!(s
        .locate(Place::Stick, Path::new("./drums/loop.wav"))
        .is_ok());

    for p in [
        "../outside.wav",
        "drums/../../outside.wav",
        "drums/../drums/loop.wav",
        "",
    ] {
        assert_eq!(
            s.locate(Place::Stick, Path::new(p)).unwrap_err(),
            Refusal::Outside(p.into()),
            "{p:?}"
        );
    }
    let abs = outside.to_string_lossy().into_owned();
    assert_eq!(
        s.locate(Place::Stick, Path::new(&abs)).unwrap_err(),
        Refusal::Outside(abs.clone().into())
    );
    // A link to a file outside, and a link to a folder on the way.
    assert_eq!(
        s.locate(Place::Stick, Path::new("escape.wav")).unwrap_err(),
        Refusal::Link("escape.wav".into())
    );
    assert_eq!(
        s.locate(Place::Stick, Path::new("up/outside.wav"))
            .unwrap_err(),
        Refusal::Link("up/outside.wav".into())
    );
    assert_eq!(
        s.locate(Place::Stick, Path::new("drums")).unwrap_err(),
        Refusal::NotFound("drums".into())
    );
    assert_eq!(
        s.locate(Place::Stick, Path::new("nothing.wav"))
            .unwrap_err(),
        Refusal::NotFound("nothing.wav".into())
    );
    assert_eq!(
        s.locate(Place::Memory, Path::new("drums/loop.wav"))
            .unwrap_err(),
        Refusal::NotStreamed
    );
    // No card yet: nothing on it can be found.
    assert_eq!(
        s.locate(Place::Card, Path::new("song.wav")).unwrap_err(),
        Refusal::NoCard
    );
    // The same rules on the card.
    s.card_inserted(PretendCard::new(&card));
    symlink(&outside, card.join("escape.wav")).expect("link");
    assert!(s.locate(Place::Card, Path::new("song.wav")).is_ok());
    assert_eq!(
        s.locate(Place::Card, Path::new("../stick/drums/loop.wav"))
            .unwrap_err(),
        Refusal::Outside("../stick/drums/loop.wav".into())
    );
    assert_eq!(
        s.locate(Place::Card, Path::new("escape.wav")).unwrap_err(),
        Refusal::Link("escape.wav".into())
    );
}

#[test]
fn a_swapped_card_is_never_taken_for_the_first() {
    let (stick, card) = libraries("storage-swap", 1_000);
    let mut s = Storage::new(&stick);
    let first = PretendCard::new(&card);
    assert_eq!(s.card_inserted(first.clone()), 1);
    let a = s.locate(Place::Card, Path::new("song.wav")).expect("on it");
    assert_eq!(a.card.as_ref().map(|c| c.insertion), Some(1));
    assert!(a.valid());
    assert!(s.open(&a).is_ok());

    // Pulled: the handle goes at once, before the slot is even told.
    first.pull();
    assert!(!a.valid());
    assert!(s.card().is_none());
    assert_eq!(s.open(&a).unwrap_err(), Refusal::NoCard);
    assert_eq!(
        s.locate(Place::Card, Path::new("song.wav")).unwrap_err(),
        Refusal::NoCard
    );
    s.card_removed();

    // The same card (the same folder) back in: a new insertion. What was
    // found on the first never holds again.
    assert_eq!(s.card_inserted(PretendCard::new(&card)), 2);
    assert!(!a.valid());
    assert_eq!(s.open(&a).unwrap_err(), Refusal::NoCard);
    let b = s.locate(Place::Card, Path::new("song.wav")).expect("on it");
    assert_eq!(b.card.as_ref().map(|c| c.insertion), Some(2));
    assert!(b.valid());
    // The stick doesn't care.
    let d = s
        .locate(Place::Stick, Path::new("drums/loop.wav"))
        .expect("on the stick");
    assert!(d.valid());
}

/// Reads at once, but holds the replies while `hold` is set: reads made
/// while the card was in, answered after it went. Logs each read's file
/// by inode.
struct Held {
    inner: Inline,
    held: VecDeque<ReadReply>,
    hold: Arc<AtomicBool>,
    log: Arc<Mutex<Vec<u64>>>,
}

impl Device for Held {
    fn submit(&mut self, r: ReadReq) {
        self.log
            .lock()
            .expect("log")
            .push(r.file.metadata().expect("meta").ino());
        self.inner.submit(r);
        let reply = self.inner.poll().expect("inline answers");
        self.held.push_back(reply);
    }
    fn poll(&mut self) -> Option<ReadReply> {
        if self.hold.load(Ordering::Relaxed) {
            return None;
        }
        self.held.pop_front()
    }
}

struct Rig {
    p: Player,
    c: Coordinator,
    hold: Arc<AtomicBool>,
    log: Arc<Mutex<Vec<u64>>>,
}

fn rig() -> Rig {
    let cfg = Config {
        slots: 64,
        ..Config::default()
    };
    let plan = Plan {
        window_s: 1.0,
        min_window_s: 0.5,
        ..Plan::default()
    };
    let (p, rx, tx) = Player::new(cfg);
    let hold = Arc::new(AtomicBool::new(false));
    let log = Arc::new(Mutex::new(Vec::new()));
    let dev = Held {
        inner: Inline::default(),
        held: VecDeque::new(),
        hold: hold.clone(),
        log: log.clone(),
    };
    let c = Coordinator::new(cfg, plan, rx, tx, Box::new(dev));
    Rig { p, c, hold, log }
}

impl Rig {
    /// Reads of the file with inode `ino` so far.
    fn reads_of(&self, ino: u64) -> usize {
        self.log
            .lock()
            .expect("log")
            .iter()
            .filter(|&&i| i == ino)
            .count()
    }

    /// Hold the cursors where they are while what they want ahead is
    /// read.
    fn warm(&mut self, ks: &mut [&mut Cursor]) {
        let mut out = vec![0.0f32; 512];
        for _ in 0..300 {
            self.p.begin();
            for k in ks.iter_mut() {
                let at = k.frame();
                k.play(&mut self.p, &mut out);
                k.seek(at);
            }
            self.p.end(256);
            self.c.step();
        }
    }

    /// One period of 256 frames for each cursor, each checked frame by
    /// frame against its marker: silence, or exactly the frame it should
    /// be. Gives (sounding, missed) for each.
    fn period(&mut self, ks: &mut [(&mut Cursor, u32)]) -> Vec<(u64, u64)> {
        self.p.begin();
        let mut got = Vec::new();
        for (k, marker) in ks.iter_mut() {
            let from = k.frame();
            let mut out = vec![0.0f32; 512];
            let missed = k.play(&mut self.p, &mut out);
            let mut sounding = 0;
            for (j, f) in out.chunks_exact(2).enumerate() {
                if f != [0.0, 0.0] {
                    let at = from + j as u64;
                    assert_eq!(
                        f,
                        [played(*marker, at, 0), played(*marker, at, 1)],
                        "marker {marker}, frame {at}"
                    );
                    sounding += 1;
                }
            }
            got.push((sounding, missed));
        }
        self.p.end(256);
        self.c.step();
        got
    }
}

#[test]
fn the_card_pulled_mid_play() {
    let frames = 400_000u64;
    let (stick, card) = libraries("storage-pulled", frames);
    let mut s = Storage::new(&stick);
    let handle = PretendCard::new(&card);
    s.card_inserted(handle.clone());
    let song = s.locate(Place::Card, Path::new("song.wav")).expect("song");
    let drums = s
        .locate(Place::Stick, Path::new("drums/loop.wav"))
        .expect("drums");
    let song_ino = std::fs::metadata(&song.path).expect("meta").ino();

    let mut r = rig();
    r.c.new_project();
    let tag = r.c.open_located(0, &song, false).expect("song opens");
    r.c.open_located(1, &drums, false).expect("drums open");
    let (mut k0, mut k1) = (Cursor::new(0, 0), Cursor::new(1, 1));
    // Warm up: both ready before they play.
    r.warm(&mut [&mut k0, &mut k1]);
    let mut drums_missed = 0;
    for _ in 0..100 {
        let got = r.period(&mut [(&mut k0, 3), (&mut k1, 5)]);
        assert_eq!(
            got[0],
            (256, 0),
            "the song plays whole while the card is in"
        );
        drums_missed += got[1].1;
    }

    // Reads go out but their answers wait: then the card is pulled.
    r.hold.store(true, Ordering::Relaxed);
    let before = r.reads_of(song_ino);
    for _ in 0..40 {
        let got = r.period(&mut [(&mut k0, 3), (&mut k1, 5)]);
        assert_eq!(got[0].1, 0);
        drums_missed += got[1].1;
    }
    assert!(
        r.reads_of(song_ino) > before,
        "the song read on while the answers waited"
    );
    handle.pull();
    s.card_removed();
    r.hold.store(false, Ordering::Relaxed);
    let at_pull = r.reads_of(song_ino);
    assert!(r.c.lost().is_none(), "not reported before a round has run");

    // What was in memory plays out; nothing more is read from the card;
    // the stick plays on.
    let pulled_at = k0.frame();
    let (mut sounded, mut missed, mut quiet_since) = (0u64, 0u64, None);
    for i in 0..600 {
        let got = r.period(&mut [(&mut k0, 3), (&mut k1, 5)]);
        sounded += got[0].0;
        missed += got[0].1;
        drums_missed += got[1].1;
        if got[0].0 > 0 {
            assert!(
                quiet_since.is_none(),
                "the song came back after going quiet"
            );
        } else if quiet_since.is_none() {
            quiet_since = Some(i);
        }
        if i == 0 {
            assert_eq!(r.c.lost(), Some(tag), "the song lost its source");
            assert_eq!(r.c.lost(), None, "reported once");
        }
    }
    assert_eq!(
        r.reads_of(song_ino),
        at_pull,
        "no read of the card after the pull"
    );
    assert!(sounded > 0, "the audio in memory played out");
    assert!(quiet_since.is_some(), "and then stopped");
    assert!(
        pulled_at + sounded < frames,
        "it stopped where memory ran out, not at the end"
    );
    assert!(missed > 0, "the frames it couldn't play are counted");
    assert!(
        r.c.stats().card_gone > 0,
        "answers that came after the pull were dropped: {:?}",
        r.c.stats()
    );
    assert_eq!(drums_missed, 0, "the stick played on without a gap");
    assert_eq!(r.c.lost(), None);

    // The card back in is a new insertion: the old source stays as it is,
    // and the old find can't open anything.
    assert_eq!(s.card_inserted(PretendCard::new(&card)), 2);
    assert!(r.c.open_located(0, &song, false).is_err());
    for _ in 0..50 {
        let got = r.period(&mut [(&mut k0, 3), (&mut k1, 5)]);
        assert_eq!(got[0].0, 0, "never refilled from the new insertion");
    }
    assert_eq!(r.reads_of(song_ino), at_pull);

    // Found again on the new insertion, it opens and plays.
    let again = s.locate(Place::Card, Path::new("song.wav")).expect("song");
    let tag2 = r.c.open_located(0, &again, false).expect("opens again");
    assert_ne!(tag2, tag);
    let mut k2 = Cursor::new(0, 2);
    r.warm(&mut [&mut k2]);
    for _ in 0..20 {
        let got = r.period(&mut [(&mut k2, 3)]);
        assert_eq!(got[0], (256, 0));
    }
}

#[test]
fn a_whole_file_read_from_the_card_fails_when_the_card_goes() {
    let (stick, card) = libraries("storage-jobs", 50_000);
    let mut s = Storage::new(&stick);
    let handle = PretendCard::new(&card);
    s.card_inserted(handle.clone());
    let song = s.locate(Place::Card, Path::new("song.wav")).expect("song");
    let drums = s
        .locate(Place::Stick, Path::new("drums/loop.wav"))
        .expect("drums");
    let mut r = rig();
    r.c.new_project();

    // One read already under way when the card goes, one not yet started,
    // and one from the stick.
    let started = r.c.read_located(&song, JobTier::NextSong).expect("starts");
    r.hold.store(true, Ordering::Relaxed);
    r.c.step();
    let waiting = r.c.read_located(&song, JobTier::Background).expect("waits");
    let stick_job =
        r.c.read_located(&drums, JobTier::Background)
            .expect("waits");
    handle.pull();
    r.hold.store(false, Ordering::Relaxed);
    for _ in 0..200 {
        r.c.step();
    }
    for id in [started, waiting] {
        let got = r.c.take_job(id).expect("finished");
        assert!(
            got.as_ref().is_err_and(|e| e.contains("SD card")),
            "{got:?}"
        );
    }
    let bytes =
        r.c.take_job(stick_job)
            .expect("finished")
            .expect("the stick read whole");
    assert_eq!(
        bytes.len() as u64,
        std::fs::metadata(&drums.path).expect("meta").len()
    );
    // Nothing found on the pulled card can be read or opened again.
    assert!(r.c.read_located(&song, JobTier::Background).is_err());
    assert!(r.c.open_located(0, &song, false).is_err());
}

#[test]
fn a_take_still_being_written_on_the_card_is_read_only_while_its_lease_holds() {
    let (stick, card) = libraries("storage-growing", 50_000);
    let mut s = Storage::new(&stick);
    let lease = Lease::new(&card);
    assert_eq!(lease.root(), card);
    s.card_inserted(lease.clone());
    let take = s.locate(Place::Card, Path::new("song.wav")).expect("take");
    let mut r = rig();
    r.c.new_project();
    let tag =
        r.c.open_located_growing(0, &take, 10_000)
            .expect("opens as it grows");
    assert_eq!(r.c.frames(0), Some(10_000), "what is committed, no more");
    r.c.grow(0, 20_000).expect("grows");
    assert_eq!(r.c.frames(0), Some(20_000));

    // The slot hears the card closed: its lease goes, and with it the
    // source.
    lease.revoke();
    s.card_removed();
    assert!(!lease.valid());
    let mut k = Cursor::new(0, 0);
    r.period(&mut [(&mut k, 3)]);
    assert_eq!(r.c.lost(), Some(tag));
    assert!(r.c.open_located_growing(0, &take, 10_000).is_err());

    // A lease is one opening: the same card opened again is another.
    let again = Lease::new(&card);
    assert!(*again != *lease);
    assert!(*lease == *lease);
}
