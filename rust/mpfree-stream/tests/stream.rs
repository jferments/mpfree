//! The stream layer end to end: real files read by the coordinator, the
//! player's cache, and cursors playing them. Exact playback in each
//! format, loops and speed, counted misses, faults, and the unload and
//! replace test: a source closed, replaced or invalidated mid-play never
//! plays old or half-written audio.

use std::collections::HashMap;

use mpfree_stream::coord::{Coordinator, Device, HoldTier, IoThread, Plan, ReadReply, ReadReq};
use mpfree_stream::cursor::{Cursor, Read};
use mpfree_stream::player::{unit_frames, Config, Player};
use mpfree_stream::testkit::{
    files, folder, marker, memory, played, played16, wav, Expect, Rig, Store,
};

fn small() -> Config {
    Config {
        slots: 64,
        ..Config::default()
    }
}

/// Play `frames` frames from where the cursor is, 256 at a time, and
/// check each against `want(output frame)`.
fn play_and_check(
    rig: &mut Rig,
    k: &mut Cursor,
    frames: usize,
    mut want: impl FnMut(usize) -> Option<[f32; 2]>,
) -> u64 {
    let mut missed = 0;
    let mut done = 0;
    while done < frames {
        let n = (frames - done).min(256);
        let mut outs = vec![vec![0.0; n * 2]];
        missed += rig.period(&mut [k], &mut outs);
        for j in 0..n {
            if let Some(w) = want(done + j) {
                let got = [outs[0][j * 2], outs[0][j * 2 + 1]];
                assert_eq!(got, w, "output frame {}", done + j);
            }
        }
        done += n;
    }
    missed
}

#[test]
fn every_format_plays_back_exactly() {
    let dir = folder("formats");
    // Four and a bit units of each.
    for (store, channels) in [
        (Store::Pcm16, 2u16),
        (Store::Pcm24, 1),
        (Store::Pcm24, 2),
        (Store::Float, 2),
        (Store::Float, 1),
    ] {
        let frames = unit_frames(channels) * 4 + 777;
        let path = dir.join(format!("{store:?}-{channels}.wav"));
        wav(&path, store, channels, frames, 5);
        let mut rig = Rig::new(small(), Plan::default());
        rig.c.new_project();
        rig.c.open(0, &path).expect("opens");
        let mut k = Cursor::new(0, 0);
        rig.warm(&mut [&mut k], 400);
        let at = |f: u64, c: usize| match store {
            Store::Pcm16 => played16(5, f, c),
            _ => played(5, f, c),
        };
        let before = rig.p.stats().missed;
        let missed = play_and_check(&mut rig, &mut k, frames as usize + 300, |j| {
            let f = j as u64;
            Some(if f >= frames {
                [0.0, 0.0]
            } else if channels == 1 {
                [at(f, 0), at(f, 0)]
            } else {
                [at(f, 0), at(f, 1)]
            })
        });
        assert_eq!(missed, 0, "{store:?} {channels}");
        assert_eq!(rig.p.stats().missed, before);
        let s = rig.c.stats();
        assert_eq!((s.read_errors, s.short_reads, s.stale), (0, 0, 0));
    }
}

#[test]
fn loops_seeks_and_speed_land_on_the_right_frames() {
    let dir = folder("loops");
    let path = dir.join("a.wav");
    let frames = 60_000u64;
    wav(&path, Store::Float, 2, frames, 3);
    let mut rig = Rig::new(small(), Plan::default());
    rig.c.new_project();
    rig.c.open(0, &path).expect("opens");
    let at = |f: u64| [played(3, f, 0), played(3, f, 1)];

    // A loop crossing a unit's edge, entered from before it. Where the
    // cursor lands is the point here: Hermite gives a whole frame's own
    // sample at any speed (the band-limited read's are in tests/rate.rs).
    let mut k = Cursor::new(0, 0);
    k.set_read(Read::Hermite);
    let (ls, le) = (7_000u64, 9_500u64);
    k.set_loop(Some((ls, le)));
    rig.warm(&mut [&mut k], 400);
    let wrap = |f: u64| if f < le { f } else { ls + (f - le) % (le - ls) };
    let missed = play_and_check(&mut rig, &mut k, 20_000, |j| Some(at(wrap(j as u64))));
    assert_eq!(missed, 0);

    // A seek into the middle of a unit, at double speed, no loop.
    k.set_loop(None);
    k.seek(20_001);
    k.set_speed(2.0);
    rig.warm(&mut [&mut k], 400);
    let missed = play_and_check(&mut rig, &mut k, 15_000, |j| {
        let f = 20_001 + 2 * j as u64;
        Some(if f < frames { at(f) } else { [0.0, 0.0] })
    });
    assert_eq!(missed, 0);

    // One and a half times, round a loop: every other output frame lands
    // on a whole file frame and is the file's own sample.
    k.seek(30_000);
    k.set_speed(1.5);
    k.set_loop(Some((30_000, 33_001)));
    rig.warm(&mut [&mut k], 400);
    let (ls, le) = (30_000u64, 33_001u64);
    let missed = play_and_check(&mut rig, &mut k, 12_000, |j| {
        (j % 2 == 0).then(|| {
            let f = 30_000 + (j as u64 / 2) * 3;
            at(if f < le { f } else { ls + (f - le) % (le - ls) })
        })
    });
    assert_eq!(missed, 0);

    // Half speed: whole frames on even outputs, in between on odd ones.
    k.set_loop(None);
    k.seek(40_000);
    k.set_speed(0.5);
    rig.warm(&mut [&mut k], 400);
    let missed = play_and_check(&mut rig, &mut k, 4_000, |j| {
        (j % 2 == 0).then(|| at(40_000 + j as u64 / 2))
    });
    assert_eq!(missed, 0);
}

#[test]
fn two_cursors_share_a_source() {
    let dir = folder("two");
    let path = dir.join("a.wav");
    wav(&path, Store::Pcm24, 2, 100_000, 9);
    let mut rig = Rig::new(small(), Plan::default());
    rig.c.new_project();
    rig.c.open(0, &path).expect("opens");
    let mut a = Cursor::new(0, 0);
    let mut b = Cursor::new(0, 1);
    b.seek(50_000);
    rig.warm(&mut [&mut a, &mut b], 600);
    for round in 0..40u64 {
        let mut outs = vec![vec![0.0; 512], vec![0.0; 512]];
        let missed = rig.period(&mut [&mut a, &mut b], &mut outs);
        assert_eq!(missed, 0, "round {round}");
        for j in 0..256u64 {
            let fa = round * 256 + j;
            let fb = 50_000 + fa;
            assert_eq!(outs[0][j as usize * 2], played(9, fa, 0));
            assert_eq!(outs[1][j as usize * 2 + 1], played(9, fb, 1));
        }
    }
}

#[test]
fn a_cold_start_is_silent_and_counted_never_wrong() {
    let dir = folder("cold");
    let path = dir.join("a.wav");
    let frames = 80_000u64;
    wav(&path, Store::Float, 2, frames, 4);
    let mut rig = Rig::new(small(), Plan::default());
    rig.c.new_project();
    rig.c.open(0, &path).expect("opens");
    let mut k = Cursor::new(0, 0);
    let mut missed = 0;
    let mut silent = 0;
    for round in 0..200u64 {
        let mut outs = vec![vec![0.0; 512]];
        missed += rig.period(&mut [&mut k], &mut outs);
        for j in 0..256u64 {
            let f = round * 256 + j;
            let got = [outs[0][j as usize * 2], outs[0][j as usize * 2 + 1]];
            if got == [0.0, 0.0] {
                if f < frames {
                    silent += 1;
                }
            } else {
                assert_eq!(got, [played(4, f, 0), played(4, f, 1)], "frame {f}");
            }
        }
    }
    assert!(missed > 0, "the start can't be ready at once");
    assert_eq!(
        missed, silent,
        "every silent frame inside the sound is counted"
    );
    assert_eq!(rig.p.stats().missed, missed);
}

/// A device that fails, comes up short, or answers late, by a pattern.
struct Faulty {
    inner: mpfree_stream::coord::Inline,
    n: u32,
    held: Vec<ReadReply>,
}

impl Device for Faulty {
    fn submit(&mut self, r: ReadReq) {
        self.n += 1;
        self.inner.submit(r);
        let mut reply = self.inner.poll().expect("inline answers");
        match self.n % 5 {
            1 => reply.got = Err(std::io::Error::other("bad block")),
            2 => {
                if let Ok(n) = reply.got {
                    reply.got = Ok(n / 2);
                }
            }
            _ => {}
        }
        self.held.push(reply);
    }
    fn poll(&mut self) -> Option<ReadReply> {
        // Answer oldest last: replies out of order.
        self.held.pop()
    }
}

#[test]
fn failed_and_short_reads_are_counted_and_never_played() {
    let dir = folder("faults");
    let path = dir.join("a.wav");
    let frames = 120_000u64;
    wav(&path, Store::Float, 2, frames, 6);
    let cfg = small();
    let (p, rx, tx) = Player::new(cfg);
    let dev = Faulty {
        inner: Default::default(),
        n: 0,
        held: Vec::new(),
    };
    let c = Coordinator::new(cfg, Plan::default(), rx, tx, Box::new(dev));
    let mut rig = Rig { p, c };
    rig.c.new_project();
    rig.c.open(0, &path).expect("opens");
    let mut k = Cursor::new(0, 0);
    rig.warm(&mut [&mut k], 2_000);
    let missed = play_and_check(&mut rig, &mut k, frames as usize, |j| {
        Some([played(6, j as u64, 0), played(6, j as u64, 1)])
    });
    assert_eq!(missed, 0, "asked for again until it came whole");
    let s = rig.c.stats();
    assert!(s.read_errors > 0 && s.short_reads > 0, "{s:?}");
    assert_eq!(
        rig.p.stats().short,
        0,
        "nothing short reached the sound thread"
    );
}

/// A small generator, so the test is the same every run.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self, n: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) % n
    }
}

#[test]
fn unload_and_replace_mid_play_never_play_old_audio() {
    let dir = folder("unload");
    let frames = 40_000u64;
    let paths = files(&dir, frames, 64);
    // A cache too small for everything: slots are reused all the time.
    let cfg = Config {
        slots: 12,
        copy: 4_096,
        ..Config::default()
    };
    let plan = Plan {
        window_s: 0.5,
        in_flight: 4,
        ..Plan::default()
    };
    let mut rig = Rig::new(cfg, plan);
    rig.c.new_project();
    let mut gen = 0u32;
    // Every fourth open is a sound held in memory instead of a file.
    let open = |rig: &mut Rig, s: u16, gen: &mut u32| {
        *gen += 1;
        let m = marker(*gen, 64);
        let tag = if gen.is_multiple_of(4) {
            rig.c.open_memory(s, 2, memory(2, frames, m))
        } else {
            rig.c.open(s, &paths[(m - 1) as usize])
        }
        .expect("opens");
        assert_eq!(tag.gen, *gen);
    };
    for s in 0..4 {
        open(&mut rig, s, &mut gen);
    }
    let mut ks: Vec<Cursor> = (0..6).map(|i| Cursor::new(i % 4, i)).collect();
    let mut r = Lcg(7);
    let mut played_frames = 0u64;
    let mut ops: HashMap<&str, u32> = HashMap::new();
    let mut holds = Vec::new();
    for _ in 0..6_000 {
        match r.next(40) {
            0 => {
                open(&mut rig, r.next(4) as u16, &mut gen);
                *ops.entry("replace").or_default() += 1;
            }
            1 => {
                rig.c.close(r.next(4) as u16);
                *ops.entry("close").or_default() += 1;
            }
            2 => {
                let k = &mut ks[r.next(6) as usize];
                k.seek(r.next(frames));
                *ops.entry("seek").or_default() += 1;
            }
            3 => {
                let k = &mut ks[r.next(6) as usize];
                let s = r.next(frames - 100);
                k.set_loop(Some((s, s + 1 + r.next(20_000))));
                *ops.entry("loop").or_default() += 1;
            }
            4 => {
                let k = &mut ks[r.next(6) as usize];
                let s = r.next(4) as u16;
                k.set_source(s);
                *ops.entry("move").or_default() += 1;
            }
            // A whole new project now and then.
            5 if r.next(10) == 0 => {
                rig.c.new_project();
                for s in 0..4 {
                    open(&mut rig, s, &mut gen);
                }
                *ops.entry("project").or_default() += 1;
            }
            6 => {
                let k = &mut ks[r.next(6) as usize];
                let e = r.next(frames + 5_000);
                k.set_end((r.next(2) == 0).then_some(e));
                *ops.entry("end").or_default() += 1;
            }
            7 => {
                let s = r.next(4) as u16;
                let from = r.next(frames);
                let tier = [
                    HoldTier::Pinned,
                    HoldTier::Cued,
                    HoldTier::Head,
                    HoldTier::Next,
                ][r.next(4) as usize];
                if let Ok(h) = rig.c.hold(s, from, from + 1 + r.next(10_000), None, tier) {
                    holds.push(h);
                    *ops.entry("hold").or_default() += 1;
                }
            }
            8 if !holds.is_empty() => {
                let h = holds.swap_remove(r.next(holds.len() as u64) as usize);
                rig.c.release_hold(h);
            }
            _ => {}
        }
        let n = 1 + r.next(300) as usize;
        rig.p.begin();
        for k in ks.iter_mut() {
            let e = Expect::of(k);
            let mut out = vec![0.0; n * 2];
            k.play(&mut rig.p, &mut out);
            e.check(&rig.p, &out, frames, |g| marker(g, 64));
            played_frames += out.chunks_exact(2).filter(|f| f != &[0.0, 0.0]).count() as u64;
        }
        rig.p.end(n as u32);
        for _ in 0..r.next(3) {
            rig.c.step();
        }
    }
    // It did play, and did all of it.
    assert!(played_frames > 1_000_000, "played {played_frames}");
    for op in [
        "replace", "close", "seek", "loop", "move", "project", "end", "hold",
    ] {
        assert!(ops.get(op).copied().unwrap_or(0) > 3, "{op}: {ops:?}");
    }
    let ps = rig.p.stats();
    assert_eq!((ps.lost, ps.short), (0, 0), "{ps:?}");
}

#[test]
fn threads_unload_and_replace_while_playing() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    let dir = folder("threads");
    let frames = 40_000u64;
    let paths = files(&dir, frames, 64);
    let cfg = Config {
        slots: 12,
        copy: 4_096,
        ..Config::default()
    };
    let plan = Plan {
        window_s: 0.5,
        in_flight: 4,
        ..Plan::default()
    };
    let (mut p, rx, tx) = Player::new(cfg);
    let dev = IoThread::start("stream-test-io").expect("io thread");
    let mut c = Coordinator::new(cfg, plan, rx, tx, Box::new(dev));
    let stop = Arc::new(AtomicBool::new(false));

    // The coordinator's thread: opens, replaces and closes as it plans.
    let stop_c = stop.clone();
    let coord = std::thread::spawn(move || {
        c.new_project();
        let mut gen = 0u32;
        let mut r = Lcg(11);
        let open = |c: &mut Coordinator, s: u16, gen: &mut u32| {
            *gen += 1;
            let m = marker(*gen, 64);
            if gen.is_multiple_of(4) {
                c.open_memory(s, 2, memory(2, frames, m))
            } else {
                c.open(s, &paths[(m - 1) as usize])
            }
            .expect("opens");
        };
        for s in 0..4 {
            open(&mut c, s, &mut gen);
        }
        let mut changes = 0;
        while !stop_c.load(Ordering::Relaxed) {
            if !c.step() {
                std::thread::sleep(Duration::from_micros(200));
            }
            match r.next(400) {
                0 => {
                    open(&mut c, r.next(4) as u16, &mut gen);
                    changes += 1;
                }
                1 => {
                    c.close(r.next(4) as u16);
                    changes += 1;
                }
                _ => {}
            }
        }
        (changes, c.stats())
    });

    // The sound thread: this one.
    let mut ks: Vec<Cursor> = (0..6).map(|i| Cursor::new(i % 4, i)).collect();
    let mut r = Lcg(5);
    let start = Instant::now();
    let mut played_frames = 0u64;
    while start.elapsed() < Duration::from_secs(3) {
        if r.next(50) == 0 {
            let k = &mut ks[r.next(6) as usize];
            k.seek(r.next(frames));
            if r.next(2) == 0 {
                let s = r.next(frames - 100);
                k.set_loop(Some((s, s + 1 + r.next(20_000))));
            }
        }
        p.begin();
        for k in ks.iter_mut() {
            let e = Expect::of(k);
            let mut out = [0.0f32; 256 * 2];
            k.play(&mut p, &mut out);
            e.check(&p, &out, frames, |g| marker(g, 64));
            played_frames += out.chunks_exact(2).filter(|f| f != &[0.0, 0.0]).count() as u64;
        }
        p.end(256);
        std::thread::sleep(Duration::from_micros(300));
    }
    stop.store(true, Ordering::Relaxed);
    let (changes, cs) = coord.join().expect("coordinator");
    assert!(changes > 10, "changes {changes}");
    assert!(played_frames > 100_000, "played {played_frames}");
    assert_eq!(p.stats().lost, 0);
    assert_eq!(cs.read_errors, 0);
}
