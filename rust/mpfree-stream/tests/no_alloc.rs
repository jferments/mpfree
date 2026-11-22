//! The sound thread's calls (`begin`, `end`, `read`, `peek`, `playing`,
//! and a cursor's `play`, seeks, loops and speed) must not allocate or
//! free. A counting global allocator checks that through a busy run:
//! sources opened, replaced and closed mid-play, new projects, fills,
//! refusals and buffers going back, a cache too small for what plays, and
//! period sizes from 1 to 4096 frames.
//!
//! This test file is its own program, so its global allocator (the only
//! unsafe code here, a pass-through to the system allocator) does not
//! affect the library, which forbids unsafe code. Only the test's own
//! thread is watched while it makes the sound thread's calls, so the
//! coordinator's work (which may allocate) and other tests aren't
//! counted.

mod common;

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use common::{folder, wav, Rig, Store};
use mpfree_stream::coord::Plan;
use mpfree_stream::cursor::Cursor;
use mpfree_stream::player::Config;

struct Counting;
thread_local! {
    static WATCH: Cell<bool> = const { Cell::new(false) };
    static CALLS: Cell<usize> = const { Cell::new(0) };
}

fn count() {
    if WATCH.with(Cell::get) {
        CALLS.with(|c| c.set(c.get() + 1));
    }
}
fn watch(on: bool) {
    WATCH.with(|w| w.set(on));
}
fn calls() -> usize {
    CALLS.with(Cell::get)
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        count();
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        count();
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        count();
        unsafe { System.realloc(p, l, n) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

#[test]
fn the_sound_threads_calls_never_allocate() {
    let dir = folder("noalloc");
    let frames = 50_000u64;
    let paths: Vec<_> = (1..=6u32)
        .map(|m| {
            let p = dir.join(format!("{m}.wav"));
            let (store, ch) = match m % 3 {
                0 => (Store::Pcm16, 2),
                1 => (Store::Pcm24, 1),
                _ => (Store::Float, 2),
            };
            wav(&p, store, ch, frames, m);
            p
        })
        .collect();
    let cfg = Config {
        slots: 10,
        copy: 4_096,
        messages: 8,
        buffers: 8,
        ..Config::default()
    };
    let plan = Plan {
        window_s: 1.0,
        in_flight: 6,
    };
    let mut rig = Rig::new(cfg, plan);
    rig.c.new_project();
    for s in 0..3 {
        rig.c.open(s, &paths[usize::from(s)]).expect("opens");
    }
    let mut ks: Vec<Cursor> = (0..5).map(|i| Cursor::new(i % 3, i)).collect();
    let mut outs: Vec<Vec<f32>> = (0..5).map(|_| vec![0.0; 4096 * 2]).collect();
    let mut raw = vec![0.0f32; 4096 * 2];
    let mut seed = 3u64;
    let mut next = |n: u64| {
        seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (seed >> 33) % n
    };
    let mut filled = 0;
    for round in 0..4_000u64 {
        // The coordinator's moves (not watched).
        match next(30) {
            0 => {
                let s = next(3) as u16;
                rig.c.open(s, &paths[next(6) as usize]).expect("opens");
            }
            1 => rig.c.close(next(3) as u16),
            2 if next(20) == 0 => {
                rig.c.new_project();
                for s in 0..3 {
                    rig.c.open(s, &paths[next(6) as usize]).expect("opens");
                }
            }
            _ => {}
        }
        let n = [1usize, 64, 256, 333, 1024, 4096][(round % 6) as usize];
        // The sound thread's period (watched).
        watch(true);
        rig.p.begin();
        for (i, k) in ks.iter_mut().enumerate() {
            match next(60) {
                0 => k.seek(next(frames)),
                1 => {
                    let s = next(frames - 10);
                    k.set_loop(Some((s, s + 1 + next(9_000))));
                }
                2 => k.set_speed([0.5, 1.0, 1.5, 2.0, 4.0][next(5) as usize]),
                3 => k.set_source(next(3) as u16),
                4 => k.set_loop(None),
                _ => {}
            }
            k.play(&mut rig.p, &mut outs[i][..n * 2]);
        }
        rig.p.read(0, next(frames), &mut raw[..n]);
        rig.p.peek(1, next(frames), &mut raw[..n]);
        rig.p.playing(2, 400, next(frames), 1.0, None);
        rig.p.end(n as u32);
        let _ = (
            rig.p.ready(),
            rig.p.stats(),
            rig.p.source(0),
            rig.p.shape(1),
        );
        watch(false);
        assert_eq!(calls(), 0, "round {round}, period {n}");
        rig.c.step();
        filled = rig.p.stats().filled;
    }
    assert!(filled > 200, "the cache was busy: {filled} units filled");
}
