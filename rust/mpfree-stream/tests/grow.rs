//! A source whose file is still being written (a long take): opened at
//! what its writer has committed, grown as it commits more. What was
//! cached below the old end stays; the unit cut short there is read again
//! whole, even when its fill was on its way to the sound thread.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use mpfree_stream::coord::{Coordinator, Device, Inline, Plan, ReadReply, ReadReq};
use mpfree_stream::cursor::Cursor;
use mpfree_stream::player::{unit_frames, Config, Player};
use mpfree_stream::testkit::{folder, wav, Expect, Rig, Store};

const MARKER: u32 = 8;

/// A marked float WAV of `frames` frames as a take being written has it:
/// its RIFF and data sizes unknown (FFFFFFFF). Gives the bytes and where
/// its audio starts.
fn take(dir: &Path, frames: u64) -> (Vec<u8>, usize) {
    let p = dir.join("whole.wav");
    wav(&p, Store::Float, 2, frames, MARKER);
    let mut b = std::fs::read(&p).expect("read");
    b[4..8].copy_from_slice(&[0xFF; 4]);
    let data = b.windows(4).position(|w| w == b"data").expect("data");
    b[data + 4..data + 8].copy_from_slice(&[0xFF; 4]);
    (b, data + 8)
}

/// The file as written so far: `frames` frames and `extra` bytes more (a
/// write in progress).
fn written(path: &Path, b: &[u8], at: usize, frames: u64, extra: usize) {
    std::fs::write(path, &b[..at + frames as usize * 8 + extra]).expect("write");
}

/// Reads at once, logging where each one starts.
struct Logged {
    inner: Inline,
    log: Arc<Mutex<Vec<u64>>>,
}

impl Device for Logged {
    fn submit(&mut self, r: ReadReq) {
        self.log.lock().expect("log").push(r.at);
        self.inner.submit(r);
    }
    fn poll(&mut self) -> Option<ReadReply> {
        self.inner.poll()
    }
}

fn rig(copy: usize) -> (Rig, Arc<Mutex<Vec<u64>>>) {
    let cfg = Config {
        slots: 64,
        copy,
        ..Config::default()
    };
    let plan = Plan {
        window_s: 1.0,
        min_window_s: 0.5,
        ..Plan::default()
    };
    let (p, rx, tx) = Player::new(cfg);
    let log = Arc::new(Mutex::new(Vec::new()));
    let dev = Logged {
        inner: Inline::default(),
        log: log.clone(),
    };
    let c = Coordinator::new(cfg, plan, rx, tx, Box::new(dev));
    (Rig { p, c }, log)
}

/// Play `n` periods straight on from the old end, before anything is
/// warmed: every frame is the right one, or silent and counted missed.
fn play_on(rig: &mut Rig, k: &mut Cursor, n: usize, frames: u64) {
    let (mut sounding, mut missed) = (0, 0);
    for _ in 0..n {
        let e = Expect::of(k);
        let mut outs = vec![vec![0.0; 512]];
        missed += rig.period(&mut [&mut *k], &mut outs);
        sounding += e.check(&rig.p, &outs[0], frames, |_| MARKER);
    }
    assert_eq!(
        sounding + missed,
        n as u64 * 256,
        "a frame inside the sound was silent and not counted"
    );
    assert!(missed > 0, "the short unit had to be read again");
}

/// Play `n` periods, every frame checked: gives (sounding, missed).
fn play(rig: &mut Rig, k: &mut Cursor, n: usize, frames: u64) -> (u64, u64) {
    let (mut sounding, mut missed) = (0, 0);
    for _ in 0..n {
        let e = Expect::of(k);
        let mut outs = vec![vec![0.0; 512]];
        missed += rig.period(&mut [&mut *k], &mut outs);
        sounding += e.check(&rig.p, &outs[0], frames, |_| MARKER);
    }
    (sounding, missed)
}

#[test]
fn a_take_grows_while_what_was_cached_stays() {
    let dir = folder("grow");
    let full = 300_000u64;
    let (bytes, at) = take(&dir, full);
    let path = dir.join("take.wav.recording");
    let uf = unit_frames(2);
    // Committed: 100,000 frames, the last unit cut short; and half a
    // write beyond it that must never play.
    let first = 100_000u64;
    assert_ne!(first % uf, 0);
    written(&path, &bytes, at, first, 5_000 * 8 + 3);
    let (mut rig, log) = rig(8192);
    rig.c.new_project();
    rig.c.open_growing(0, &path, first).expect("opens");
    assert_eq!(rig.c.frames(0), Some(first));

    // Play the last 20,000 committed frames: whole, then the end.
    let mut k = Cursor::new(0, 0);
    k.seek(first - 20_000);
    rig.warm(&mut [&mut k], 2_000);
    assert_eq!(rig.p.shape(0), Some((2, first)));
    let (sounding, missed) = play(&mut rig, &mut k, 100, first);
    assert_eq!(missed, 0);
    assert_eq!(sounding, 20_000, "to the committed end, never past it");

    // The writer commits up to 200,000 frames.
    let more = 200_000u64;
    written(&path, &bytes, at, more, 0);
    let reads_before = log.lock().unwrap().len();
    rig.c.grow(0, more).expect("grows");
    assert_eq!(rig.c.frames(0), Some(more));
    k.seek(first - 500);
    play_on(&mut rig, &mut k, 30, more);
    k.seek(first - 20_000);
    rig.warm(&mut [&mut k], 2_000);
    assert_eq!(rig.p.shape(0), Some((2, more)));
    let (sounding, missed) = play(&mut rig, &mut k, 200, more);
    assert_eq!(missed, 0);
    assert_eq!(sounding, 200 * 256, "every frame, through the old end");
    // Units wholly below the old end weren't read again; the short one
    // was.
    let short_at = at as u64 + (first / uf) * uf * 8;
    let after: Vec<u64> = log.lock().unwrap()[reads_before..].to_vec();
    assert!(!after.is_empty());
    assert!(
        after.iter().all(|&a| a >= short_at),
        "{after:?} from {short_at}"
    );
    assert!(after.contains(&short_at), "the short unit read again");

    // Refused: shrinking, more than the file holds.
    assert!(rig.c.grow(0, more - 1).is_err());
    assert!(rig.c.grow(0, more + 1).is_err());
    assert!(rig.c.grow(0, more).is_ok(), "the same is nothing to do");
    // The take finishes: renamed in place, grown to its whole length; the
    // same open file serves it.
    written(&path, &bytes, at, full, 0);
    let done = dir.join("take.wav");
    std::fs::rename(&path, &done).expect("renamed");
    rig.c.grow(0, full).expect("grows to the whole take");
    k.seek(full - 30_000);
    rig.warm(&mut [&mut k], 2_000);
    let (sounding, missed) = play(&mut rig, &mut k, 150, full);
    assert_eq!((sounding, missed), (30_000, 0));
    assert_eq!(
        rig.p.stats().short,
        0,
        "nothing read short reached the sound thread"
    );
}

#[test]
fn growing_while_the_short_unit_is_copied_in_never_plays_it_short() {
    let dir = folder("grow-copy");
    let full = 120_000u64;
    let (bytes, at) = take(&dir, full);
    let path = dir.join("take.wav.recording");
    let first = 50_000u64;
    written(&path, &bytes, at, first, 0);
    // A copy budget so small a unit takes many periods to copy in.
    let (mut rig, _) = rig(512);
    rig.c.new_project();
    rig.c.open_growing(0, &path, first).expect("opens");
    let mut k = Cursor::new(0, 0);
    // Inside the short unit (its last 848 frames take four periods to
    // copy in): two periods, and its fill is under way, not finished.
    assert_eq!(first % unit_frames(2), 848);
    k.seek(first - 300);
    let mut outs = vec![vec![0.0; 512]];
    for _ in 0..2 {
        let f = k.frame();
        rig.period(&mut [&mut k], &mut outs);
        k.seek(f);
    }
    written(&path, &bytes, at, full, 0);
    rig.c.grow(0, full).expect("grows");
    let back = k.frame();
    play_on(&mut rig, &mut k, 30, full);
    k.seek(back);
    rig.warm(&mut [&mut k], 4_000);
    let (sounding, missed) = play(&mut rig, &mut k, 100, full);
    assert_eq!(missed, 0);
    assert_eq!(sounding, 100 * 256, "nothing short, nothing silent");
    assert_eq!(
        rig.p.stats().short,
        0,
        "nothing read short reached the sound thread"
    );
}

#[test]
fn only_a_file_at_its_own_rate_grows() {
    let dir = folder("grow-refused");
    let (bytes, at) = take(&dir, 50_000);
    let path = dir.join("take.wav.recording");
    written(&path, &bytes, at, 50_000, 0);
    let (mut rig, _) = rig(8192);
    rig.c.new_project();
    // Opened at what the file holds when asked for more than it has.
    rig.c.open_growing(0, &path, 80_000).expect("opens");
    assert_eq!(rig.c.frames(0), Some(50_000));
    // At the engine's rate (the file says 44.1 kHz; an engine at 48).
    let mut c48 = {
        let cfg = Config::default();
        let (_, rx, tx) = Player::new(cfg);
        Coordinator::new(
            cfg,
            Plan {
                rate: 48_000,
                ..Plan::default()
            },
            rx,
            tx,
            Box::<Inline>::default(),
        )
    };
    c48.new_project();
    c48.open_at_engine_rate(0, &path).expect("opens");
    assert!(c48.grow(0, 50_000).is_err());
    let mem = Arc::<[f32]>::from(vec![0.5f32; 2_000]);
    rig.c.open_memory(1, 2, mem).expect("opens");
    assert!(rig.c.grow(1, 2_000).is_err());
    assert!(rig.c.grow(2, 10).is_err(), "no such source");
}

/// Reads at once but holds the answers while `hold` is set.
struct Held {
    inner: Inline,
    held: VecDeque<ReadReply>,
    hold: Arc<AtomicBool>,
}

impl Device for Held {
    fn submit(&mut self, r: ReadReq) {
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

#[test]
fn a_read_of_the_short_unit_answered_after_a_grow_is_dropped() {
    let dir = folder("grow-flight");
    let full = 120_000u64;
    let (bytes, at) = take(&dir, full);
    let path = dir.join("take.wav.recording");
    let first = 50_000u64;
    written(&path, &bytes, at, first, 0);
    let cfg = Config {
        slots: 64,
        ..Config::default()
    };
    let (p, rx, tx) = Player::new(cfg);
    let hold = Arc::new(AtomicBool::new(true));
    let dev = Held {
        inner: Inline::default(),
        held: VecDeque::new(),
        hold: hold.clone(),
    };
    let c = Coordinator::new(cfg, Plan::default(), rx, tx, Box::new(dev));
    let mut rig = Rig { p, c };
    rig.c.new_project();
    rig.c.open_growing(0, &path, first).expect("opens");
    let mut k = Cursor::new(0, 0);
    k.seek(first - 300);
    // The short unit is asked for (cursors are heard of every 1,024
    // frames); its answer waits.
    let mut outs = vec![vec![0.0; 512]];
    for _ in 0..8 {
        let f = k.frame();
        rig.period(&mut [&mut k], &mut outs);
        k.seek(f);
    }
    assert!(rig.c.in_flight() > 0, "a read waits");
    written(&path, &bytes, at, full, 0);
    rig.c.grow(0, full).expect("grows");
    hold.store(false, Ordering::Relaxed);
    rig.warm(&mut [&mut k], 4_000);
    let (sounding, missed) = play(&mut rig, &mut k, 100, full);
    assert_eq!((sounding, missed), (100 * 256, 0));
    assert!(
        rig.c.stats().stale > 0,
        "the answer for the old length was dropped"
    );
    assert_eq!(
        rig.p.stats().short,
        0,
        "nothing read short reached the sound thread"
    );
}
