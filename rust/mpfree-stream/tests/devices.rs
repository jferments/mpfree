//! Two devices (the SD card plan's S2b, phase B2): files under a folder
//! given a device of its own (the card) read there, so a stick that stalls
//! holds up nothing on the card, and each device's gate carries playback's
//! cushion on that device alone, for the recording writer there.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use mpfree_stream::coord::{serve, Coordinator, Device, Inline, JobTier, Plan, ReadReply, ReadReq};
use mpfree_stream::cursor::Cursor;
use mpfree_stream::player::{Config, Player};
use mpfree_stream::testkit::{folder, played, wav, Rig, Store};
use mpfree_stream::writer::Gate;

fn small() -> Config {
    Config {
        slots: 64,
        ..Config::default()
    }
}

/// A device that serves one queued read each time the test gives it a
/// token: with none, a stick that has stalled.
struct Slow {
    queue: VecDeque<ReadReq>,
    tokens: Arc<AtomicUsize>,
    asked: Arc<AtomicUsize>,
}

impl Device for Slow {
    fn submit(&mut self, r: ReadReq) {
        self.asked.fetch_add(1, Ordering::Relaxed);
        self.queue.push_back(r);
    }
    fn poll(&mut self) -> Option<ReadReply> {
        if self.queue.is_empty() || self.tokens.load(Ordering::Relaxed) == 0 {
            return None;
        }
        self.tokens.fetch_sub(1, Ordering::Relaxed);
        self.queue.pop_front().map(serve)
    }
}

/// A device that serves every read at once, counting them.
#[derive(Default)]
struct Counted {
    inline: Inline,
    asked: Arc<AtomicUsize>,
}

impl Device for Counted {
    fn submit(&mut self, r: ReadReq) {
        self.asked.fetch_add(1, Ordering::Relaxed);
        self.inline.submit(r);
    }
    fn poll(&mut self) -> Option<ReadReply> {
        self.inline.poll()
    }
}

/// A rig whose first device (the stick) is `stick`, with the folder `card`
/// read on a device of its own that answers at once.
struct Two {
    rig: Rig,
    card_gate: Arc<Gate>,
    card_reads: Arc<AtomicUsize>,
}

fn two(stick: Box<dyn Device>, card: &std::path::Path) -> Two {
    let cfg = small();
    let (p, rx, tx) = Player::new(cfg);
    let mut c = Coordinator::new(cfg, Plan::default(), rx, tx, stick);
    let card_gate = Arc::new(Gate::new());
    let card_reads = Arc::new(AtomicUsize::new(0));
    let dev = Counted {
        inline: Inline::default(),
        asked: card_reads.clone(),
    };
    assert!(c.add_device(card.to_path_buf(), Box::new(dev), card_gate.clone()));
    // The same folder again keeps its first device.
    assert!(!c.add_device(
        card.to_path_buf(),
        Box::<Inline>::default(),
        Arc::new(Gate::new())
    ));
    Two {
        rig: Rig { p, c },
        card_gate,
        card_reads,
    }
}

#[test]
fn a_stalled_stick_holds_up_nothing_on_the_card() {
    let dir = folder("two-devices");
    let (stick, card) = (dir.join("stick"), dir.join("card"));
    std::fs::create_dir_all(&stick).unwrap();
    std::fs::create_dir_all(&card).unwrap();
    let frames = 200_000u64;
    wav(&stick.join("a.wav"), Store::Float, 2, frames, 3);
    wav(&card.join("b.wav"), Store::Float, 2, frames, 4);
    let stick_reads = Arc::new(AtomicUsize::new(0));
    let dev = Slow {
        queue: VecDeque::new(),
        tokens: Arc::new(AtomicUsize::new(0)),
        asked: stick_reads.clone(),
    };
    let mut t = two(Box::new(dev), &card);
    let r = &mut t.rig;
    r.c.new_project();
    r.c.open(0, &stick.join("a.wav"))
        .expect("the stick's file opens");
    r.c.open(1, &card.join("b.wav"))
        .expect("the card's file opens");
    let (mut a, mut b) = (Cursor::new(0, 0), Cursor::new(1, 1));
    r.warm(&mut [&mut a, &mut b], 400);
    // The stick took its reads and answered none; the card answered every
    // one of its own.
    let plan = r.c.plan();
    let stuck = stick_reads.load(Ordering::Relaxed);
    assert!((1..=plan.in_flight).contains(&stuck), "{stuck}");
    assert!(t.card_reads.load(Ordering::Relaxed) > 0);
    assert_eq!(r.c.in_flight(), stuck, "only the stick's are left");
    // The card's file plays whole beside the stalled stick.
    let mut missed_card = 0;
    let mut missed_stick = 0;
    for round in 0..200u64 {
        let mut outs = vec![vec![0.0; 512]; 2];
        r.p.begin();
        missed_stick += a.play(&mut r.p, &mut outs[0]);
        missed_card += b.play(&mut r.p, &mut outs[1]);
        r.p.end(256);
        r.c.step();
        for j in 0..256u64 {
            let f = round * 256 + j;
            assert_eq!(
                [outs[1][j as usize * 2], outs[1][j as usize * 2 + 1]],
                [played(4, f, 0), played(4, f, 1)],
                "the card's frame {f}"
            );
        }
    }
    assert_eq!(missed_card, 0);
    assert!(missed_stick > 0, "the stick's file can't play");
    // Each gate carries its own device's cushion: the card's is ahead, the
    // stick's is gone.
    let window = plan.window_s;
    assert!(
        t.card_gate.cushion() > window / 2.0,
        "the card's cushion {} against a window of {window}",
        t.card_gate.cushion()
    );
    assert!(r.c.gate().cushion() < 0.5, "{}", r.c.gate().cushion());
    assert_eq!(r.c.cushion(), r.c.gate().cushion(), "the least of the two");
    // Each device's timings, for Dev and the logs: the stick's reads all
    // wait, none back; the card's came back, none waiting.
    let drives = r.c.drives();
    assert_eq!(drives.len(), 2);
    assert_eq!(drives[0].root, None);
    assert_eq!((drives[0].reads.count, drives[0].queue), (0, stuck));
    assert_eq!(drives[1].root.as_deref(), Some(card.as_path()));
    assert!(drives[1].reads.count > 0 && drives[1].queue == 0);
    assert!(drives[1].reads.median <= drives[1].reads.worst);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn whole_file_reads_go_on_each_device_and_wait_only_for_it() {
    let dir = folder("two-devices-jobs");
    let (stick, card) = (dir.join("stick"), dir.join("card"));
    std::fs::create_dir_all(&stick).unwrap();
    std::fs::create_dir_all(&card).unwrap();
    wav(&stick.join("a.wav"), Store::Float, 2, 200_000, 3);
    wav(&card.join("kit.wav"), Store::Float, 2, 50_000, 5);
    wav(&stick.join("kit.wav"), Store::Float, 2, 50_000, 6);
    let tokens = Arc::new(AtomicUsize::new(0));
    let dev = Slow {
        queue: VecDeque::new(),
        tokens: tokens.clone(),
        asked: Arc::new(AtomicUsize::new(0)),
    };
    let mut t = two(Box::new(dev), &card);
    let r = &mut t.rig;
    r.c.new_project();
    // A file playing from the stalled stick: its playback reads wait.
    r.c.open(0, &stick.join("a.wav")).expect("opens");
    let mut a = Cursor::new(0, 0);
    let on_card =
        r.c.read_file(&card.join("kit.wav"), JobTier::Background)
            .expect("asks");
    let on_stick =
        r.c.read_file(&stick.join("kit.wav"), JobTier::Background)
            .expect("asks");
    let mut outs = vec![vec![0.0; 512]];
    let mut card_bytes = None;
    for _ in 0..400 {
        r.period(&mut [&mut a], &mut outs);
        if let Some(got) = r.c.take_job(on_card) {
            card_bytes = Some(got);
            break;
        }
    }
    let want = std::fs::read(card.join("kit.wav")).unwrap();
    assert_eq!(
        card_bytes.expect("the card's read finished").expect("read"),
        want
    );
    // The stick's waits behind its own playback reads.
    assert!(r.c.take_job(on_stick).is_none());
    assert_eq!(r.c.jobs_waiting(), 1);
    // Recording on the card holds back background reads there only.
    t.card_gate.set_recording(true);
    let again =
        r.c.read_file(&card.join("kit.wav"), JobTier::Background)
            .expect("asks");
    for _ in 0..50 {
        r.period(&mut [&mut a], &mut outs);
    }
    assert!(r.c.take_job(again).is_none(), "it waits for the recording");
    t.card_gate.set_recording(false);
    let mut done = false;
    for _ in 0..400 {
        r.period(&mut [&mut a], &mut outs);
        if let Some(got) = r.c.take_job(again) {
            assert_eq!(got.expect("read"), want);
            done = true;
            break;
        }
    }
    assert!(done, "it reads once the recording is saved");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_file_under_a_folder_opened_before_it_had_a_device_stays_on_the_first() {
    let dir = folder("two-devices-late");
    let card = dir.join("card");
    std::fs::create_dir_all(&card).unwrap();
    wav(&card.join("b.wav"), Store::Float, 2, 100_000, 4);
    let first_reads = Arc::new(AtomicUsize::new(0));
    let dev = Counted {
        inline: Inline::default(),
        asked: first_reads.clone(),
    };
    let cfg = small();
    let (p, rx, tx) = Player::new(cfg);
    let c = Coordinator::new(cfg, Plan::default(), rx, tx, Box::new(dev));
    let mut r = Rig { p, c };
    r.c.new_project();
    r.c.open(0, &card.join("b.wav")).expect("opens");
    let card_reads = Arc::new(AtomicUsize::new(0));
    let late = Counted {
        inline: Inline::default(),
        asked: card_reads.clone(),
    };
    assert!(r
        .c
        .add_device(card.clone(), Box::new(late), Arc::new(Gate::new())));
    let mut b = Cursor::new(0, 0);
    r.warm(&mut [&mut b], 200);
    assert!(first_reads.load(Ordering::Relaxed) > 0);
    assert_eq!(card_reads.load(Ordering::Relaxed), 0);
    // Opened again, it reads on the card's device.
    r.c.open(0, &card.join("b.wav")).expect("opens");
    let mut b = Cursor::new(0, 1);
    r.warm(&mut [&mut b], 200);
    assert!(card_reads.load(Ordering::Relaxed) > 0);
    let _ = std::fs::remove_dir_all(&dir);
}
