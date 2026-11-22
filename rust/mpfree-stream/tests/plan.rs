//! The coordinator's planning (phase P1.5): sounds held in memory, a
//! cursor's end, joined reads, cancelling, holds, launch readiness and
//! admission to the memory budget.

use std::collections::VecDeque;
use std::fs;
use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use mpfree_stream::coord::{serve, Coordinator, Device, Plan, ReadReply, ReadReq, Refusal};
use mpfree_stream::cursor::Cursor;
use mpfree_stream::player::{unit_frames, Config, Player};
use mpfree_stream::testkit::{folder, memory, played, wav, Rig, Store};

fn small() -> Config {
    Config {
        slots: 64,
        ..Config::default()
    }
}

/// Play `periods` periods of 256 frames, checking each frame against
/// `want(output frame)`; gives the frames missed.
fn play(rig: &mut Rig, k: &mut Cursor, periods: usize, want: impl Fn(u64) -> [f32; 2]) -> u64 {
    let mut missed = 0;
    for round in 0..periods {
        let mut outs = vec![vec![0.0; 512]];
        missed += rig.period(&mut [k], &mut outs);
        for j in 0..256 {
            let got = [outs[0][j * 2], outs[0][j * 2 + 1]];
            assert_eq!(
                got,
                want((round * 256 + j) as u64),
                "frame {}",
                round * 256 + j
            );
        }
    }
    missed
}

#[test]
fn a_sound_in_memory_plays_at_once_and_comes_back() {
    let frames = 30_000u64;
    let data = memory(2, frames, 7);
    let mut rig = Rig::new(small(), Plan::default());
    rig.c.new_project();
    rig.c.open_memory(0, 2, data.clone()).expect("opens");
    let mut k = Cursor::new(0, 0);
    // Nothing to read first: the first period already plays.
    let missed = play(&mut rig, &mut k, 150, |f| {
        if f < frames {
            [played(7, f, 0), played(7, f, 1)]
        } else {
            [0.0, 0.0]
        }
    });
    assert_eq!(missed, 0);
    assert_eq!(rig.c.stats().reads, 0, "never read from disk");
    assert_eq!(rig.c.cached(), 0, "takes no room in the cache");
    assert!(rig.c.is_ready(0, 0, frames));
    assert!(rig.p.covers(0, 0, frames));
    // A replace sends it back, and the coordinator lets it go.
    rig.c.open_memory(0, 2, memory(2, 100, 8)).expect("opens");
    rig.p.begin();
    rig.c.step();
    assert_eq!(
        Arc::strong_count(&data),
        1,
        "back and freed off the sound thread"
    );
    // Mono plays in both channels; whole frames only.
    let mono = memory(1, 500, 9);
    rig.c.open_memory(1, 1, mono).expect("opens");
    assert!(rig.c.open_memory(2, 2, memory(1, 501, 9)).is_err());
    assert!(rig.c.open_memory(2, 3, memory(1, 600, 9)).is_err());
    let mut m = Cursor::new(1, 1);
    let missed = play(&mut rig, &mut m, 1, |f| {
        if f < 500 {
            [played(9, f, 0); 2]
        } else {
            [0.0, 0.0]
        }
    });
    assert_eq!(missed, 0);
}

#[test]
fn a_cursor_stops_at_its_end_and_reads_nothing_past_it() {
    let dir = folder("end");
    let path = dir.join("a.wav");
    let frames = 100_000u64;
    wav(&path, Store::Float, 2, frames, 3);
    let mut rig = Rig::new(small(), Plan::default());
    rig.c.new_project();
    rig.c.open(0, &path).expect("opens");
    let mut k = Cursor::new(0, 0);
    k.seek(20_000);
    k.set_end(Some(30_000));
    rig.warm(&mut [&mut k], 400);
    let missed = play(&mut rig, &mut k, 60, |j| {
        let f = 20_000 + j;
        if f < 30_000 {
            [played(3, f, 0), played(3, f, 1)]
        } else {
            [0.0, 0.0]
        }
    });
    assert_eq!(missed, 0, "silence past the end is not a miss");
    let uf = unit_frames(2);
    assert!(rig.c.is_ready(0, 20_000, 30_000));
    assert!(
        !rig.c.is_ready(0, 30_000_u64.div_ceil(uf) * uf, frames),
        "nothing read past the end"
    );
}

#[test]
fn reads_join_neighbours_up_to_the_limit() {
    let dir = folder("join");
    let path = dir.join("a.wav");
    let frames = 300_000u64;
    wav(&path, Store::Pcm24, 2, frames, 5);
    for (join, joins) in [(512 * 1024, true), (32 * 1024, false)] {
        let plan = Plan {
            join_bytes: join,
            ..Plan::default()
        };
        let mut rig = Rig::new(small(), plan);
        rig.c.new_project();
        rig.c.open(0, &path).expect("opens");
        let mut k = Cursor::new(0, 0);
        rig.warm(&mut [&mut k], 600);
        let s = rig.c.stats();
        assert_eq!(s.joined > 0, joins, "{join}: {s:?}");
        assert!(rig.c.is_ready(0, 0, frames));
        let missed = play(&mut rig, &mut k, 200, |f| {
            [played(5, f, 0), played(5, f, 1)]
        });
        assert_eq!(missed, 0);
    }
}

/// A device that serves one queued read each time the test gives it a
/// token: a slow stick.
struct Slow {
    queue: VecDeque<ReadReq>,
    tokens: Arc<AtomicUsize>,
}

impl Device for Slow {
    fn submit(&mut self, r: ReadReq) {
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

#[test]
fn reads_no_longer_wanted_are_skipped() {
    let dir = folder("cancel");
    let path = dir.join("a.wav");
    let frames = 400_000u64;
    wav(&path, Store::Float, 2, frames, 6);
    let cfg = small();
    let (p, rx, tx) = Player::new(cfg);
    let tokens = Arc::new(AtomicUsize::new(0));
    let dev = Slow {
        queue: VecDeque::new(),
        tokens: tokens.clone(),
    };
    let plan = Plan {
        join_bytes: 0,
        ..Plan::default()
    };
    let c = Coordinator::new(cfg, plan, rx, tx, Box::new(dev));
    let mut rig = Rig { p, c };
    rig.c.new_project();
    rig.c.open(0, &path).expect("opens");
    let mut k = Cursor::new(0, 0);
    // Ask for the start, with the stick answering nothing yet.
    let mut outs = vec![vec![0.0; 512]];
    for _ in 0..8 {
        rig.period(&mut [&mut k], &mut outs);
        k.seek(0);
    }
    assert_eq!(rig.c.in_flight(), 8);
    // Jump far away: the reads for the start are no longer wanted.
    k.seek(300_000);
    for _ in 0..8 {
        rig.period(&mut [&mut k], &mut outs);
        k.seek(300_000);
    }
    tokens.store(10_000, Ordering::Relaxed);
    rig.warm(&mut [&mut k], 800);
    let s = rig.c.stats();
    assert_eq!(s.cancelled, 8, "{s:?}");
    assert!(!rig.c.is_ready(0, 0, 1_000), "the start was never read");
    let missed = play(&mut rig, &mut k, 100, |j| {
        let f = 300_000 + j;
        [played(6, f, 0), played(6, f, 1)]
    });
    assert_eq!(missed, 0);
}

#[test]
fn holds_fetch_starts_whole_or_not_at_all() {
    let dir = folder("holds");
    let frames = 200_000u64;
    let paths: Vec<_> = (1..=3u32)
        .map(|m| {
            let p = dir.join(format!("{m}.wav"));
            wav(&p, Store::Float, 2, frames, m);
            p
        })
        .collect();
    let cfg = Config {
        slots: 16,
        ..Config::default()
    };
    let mut rig = Rig::new(cfg, Plan::default());
    rig.c.new_project();
    for (s, p) in paths.iter().enumerate() {
        rig.c.open(s as u16, p).expect("opens");
    }
    let uf = unit_frames(2);
    // The first two units of each, by a deadline, with nothing playing.
    let parts = [(0, 0, 2 * uf), (1, 0, 2 * uf), (2, 0, 2 * uf)];
    let launch = rig
        .c
        .hold_all(&parts, Some(Duration::from_millis(50)))
        .expect("fits");
    assert_eq!(rig.c.not_ready(&parts), vec![0, 1, 2]);
    for _ in 0..40 {
        rig.p.begin();
        rig.c.step();
    }
    assert!(rig.c.not_ready(&parts).is_empty(), "fetched by the launch");
    assert!(parts.iter().all(|&(s, f, t)| rig.p.covers(s, f, t - f)));
    // Too much for the room left: none of it is held.
    let big = [(0, 0, 4 * uf), (1, 0, 12 * uf)];
    match rig.c.hold_all(&big, None) {
        Err(Refusal::OverBudget { need, room }) => {
            assert_eq!((need, room), (2 + 10, 10));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        rig.c.admission().held,
        6,
        "nothing of the refused group held"
    );
    // A part of a source that isn't open, or is in memory: refused.
    assert_eq!(rig.c.hold(9, 0, 10, None), Err(Refusal::NotOpen(9)));
    rig.c.open_memory(5, 1, memory(1, 10, 1)).expect("opens");
    assert_eq!(rig.c.hold(5, 0, 10, None), Err(Refusal::NotOpen(5)));
    // Replacing a source lets its holds go.
    rig.c.open(1, &paths[2]).expect("opens");
    assert!(rig.c.is_held(launch[0]) && !rig.c.is_held(launch[1]));
    rig.c.release_hold(launch[0]);
    assert!(!rig.c.is_held(launch[0]));
}

/// A sparse WAV header for `frames` frames of 24-bit stereo: the layout
/// is real, the audio all zeros, and it takes no disk room.
fn sparse(path: &std::path::Path, frames: u64) {
    let data = frames * 6;
    let mut h = b"RIFF".to_vec();
    h.extend_from_slice(&(36 + data as u32).to_le_bytes());
    h.extend_from_slice(b"WAVEfmt ");
    h.extend_from_slice(&16u32.to_le_bytes());
    h.extend_from_slice(&1u16.to_le_bytes());
    h.extend_from_slice(&2u16.to_le_bytes());
    h.extend_from_slice(&44_100u32.to_le_bytes());
    h.extend_from_slice(&(44_100u32 * 6).to_le_bytes());
    h.extend_from_slice(&6u16.to_le_bytes());
    h.extend_from_slice(&24u16.to_le_bytes());
    h.extend_from_slice(b"data");
    h.extend_from_slice(&(data as u32).to_le_bytes());
    let mut f = fs::File::create(path).expect("create");
    f.write_all(&h).expect("header");
    f.set_len(h.len() as u64 + data).expect("size");
}

#[test]
fn admission_says_plainly_when_a_workload_does_not_fit() {
    // The plan's window rule.
    assert_eq!(Plan::for_shortfall(20.0).window_s, 41.0);
    assert_eq!(Plan::for_shortfall(2.0).window_s, 5.0);
    assert_eq!(Plan::for_shortfall(0.5).window_s, 4.0);
    assert_eq!(Plan::for_shortfall(f64::NAN).window_s, 4.0);

    // The design page's case: a stick with a 20 s stall wants 41 s ahead,
    // about 231 MB for 16 float stems, so 16 stems don't fit 128 MB.
    let dir = folder("admit");
    let frames = 45 * 44_100;
    for s in 0..16 {
        sparse(&dir.join(format!("{s}.wav")), frames);
    }
    let cfg = Config::default(); // 2048 units: 128 MiB of float
    let run = |plan: Plan, stems: u16, reserve: usize| {
        let mut rig = Rig::new(cfg, plan);
        rig.c.new_project();
        rig.c.set_reserve(reserve);
        let mut ks: Vec<Cursor> = (0..stems)
            .map(|s| {
                rig.c.open(s, &dir.join(format!("{s}.wav"))).expect("opens");
                Cursor::new(s, s)
            })
            .collect();
        let mut refs: Vec<&mut Cursor> = ks.iter_mut().collect();
        let mut outs = vec![vec![0.0; 512]; refs.len()];
        for _ in 0..8 {
            rig.period(&mut refs, &mut outs);
        }
        rig.c.admission()
    };
    let a = run(Plan::for_shortfall(20.0), 16, 0);
    assert_eq!(a.budget, 2048);
    assert!(a.playing > 3_500, "{a:?}");
    assert!(!a.covered && a.window_s < 41.0, "{a:?}");
    // It still plays with what fits (the window is above the safe least),
    // and says it isn't covered.
    assert!(a.safe && a.window_s > 20.0, "{a:?}");
    // 8 stems fit; with a recording's reserve taking most of the room
    // they don't, and below 4 s they aren't safe.
    let b = run(Plan::for_shortfall(20.0), 8, 0);
    assert!(b.covered && b.window_s == 41.0, "{b:?}");
    let c = run(Plan::for_shortfall(20.0), 8, 1_900);
    assert!(!c.covered && !c.safe && c.reserved == 1_900, "{c:?}");
}
