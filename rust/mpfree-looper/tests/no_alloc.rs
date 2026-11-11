//! The looper's real-time calls (`command`, `process`, `status`, `stats`,
//! `layer`) must not allocate or free. A counting global allocator checks
//! that across a busy session: the first pass, overdubs, undo and redo,
//! clear, mute, level and latency changes, a full memory, locates, a
//! transport loop, tempo changes, stop and count-in, at period sizes from 1
//! to 4096 frames.
//!
//! This test file is its own program, so its global allocator (the only
//! unsafe code here, a pass-through to the system allocator) does not
//! affect the library, which forbids unsafe code. Only the test's own
//! thread is watched. (The same pattern as mpfree-transport's no_alloc
//! test.)

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};

use mpfree_looper::{Command as L, LayerCommand as Lc, Looper, Phase, MAX_LOOPS};
use mpfree_transport::{Block, Command as T, Transport};

struct Counting;
thread_local! {
    static WATCH: Cell<bool> = const { Cell::new(false) };
}
static CALLS: AtomicUsize = AtomicUsize::new(0);

fn count() {
    if WATCH.with(Cell::get) {
        CALLS.fetch_add(1, Ordering::Relaxed);
    }
}
fn watch(on: bool) {
    WATCH.with(|w| w.set(on));
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
fn commands_and_periods_never_allocate() {
    const RATE: u32 = 44_100;
    let mut t = Transport::new(RATE, 120.0).unwrap();
    // 12 seconds: a 2-bar loop at 120 BPM is 4 s, so it fills up after
    // two overdubs and the rest are refused.
    let mut l = Looper::new(RATE, 12);
    let mut b = Block::new();
    let mut input = vec![0.0f32; 2 * 4096];
    let mut out = vec![0.0f32; 2 * 4096];

    // The counter itself works: one Vec is one allocation and one free.
    watch(true);
    drop(std::hint::black_box(vec![0u8; 16]));
    watch(false);
    assert_eq!(CALLS.swap(0, Ordering::SeqCst), 2);

    let sizes = [1usize, 7, 128, 333, 4096, 64, 256];
    let script = [
        L::Overdub,
        L::Undo,
        L::Redo,
        L::Overdub,
        L::Overdub,
        L::Mute(true),
        L::Mute(false),
        L::SetLevel(1.5),
        L::Overdub,
        L::Undo,
        L::Undo,
        L::SetLatency(300),
        L::Record,
        L::Record,
        L::Clear,
        L::Record,
        L::SetBars(None),
        L::Record,
        L::SetBars(Some(1)),
    ];
    let mut seen_playing = false;
    let mut layers = 0usize;
    let mut sum = 0.0f64;
    let mut phase = 0.0f32;
    let mut loops_used = 0u32;
    watch(true);
    t.command(0, T::Play);
    l.command(L::Record);
    for i in 0..6000u64 {
        let f = t.frame();
        // The first pass (2 bars, about 250 periods) and the first
        // overdubs run undisturbed; then the transport moves around.
        match if i < 600 { 0 } else { i % 500 } {
            100 => { let _ = t.command(f + 3, T::Locate((i as i64 * 37) % 9000)); }
            200 => { let _ = t.command(f + 1, T::SetLoop(if i % 1000 == 200 { Some((0, 7680)) } else { None })); }
            300 => { let _ = t.command(f + 2, T::SetTempo(100.0 + (i % 7) as f64 * 5.0)); }
            400 => { let _ = t.command(f + 2, T::Stop); }
            420 => { let _ = t.command(f + 2, T::SetCountIn(1)); let _ = t.command(f + 2, T::Record); }
            _ => {}
        }
        if i >= 290 && i % 37 == 0 {
            let _ = l.command(script[(i / 37 - 8) as usize % script.len()]);
        }
        // The last third moves between three loops (and one past the end).
        if i >= 4000 && i % 211 == 0 {
            let _ = l.select_loop([0, 3, 7, MAX_LOOPS][(i / 211) as usize % 4]);
        }
        // Each layer's controls; deletes only in the second half, so the
        // first half fills the memory and overdubs are refused.
        if i >= 290 && i % 53 == 0 {
            let k = (i / 53) as usize % if i < 3000 { 4 } else { 5 };
            let id = l.layer_info(k % l.status().layers.max(1)).map_or(9, |x| x.id);
            let c = [Lc::Mute(i % 2 == 0), Lc::Solo(i % 3 == 0), Lc::Pan(0.3), Lc::Level(1.4), Lc::Delete][k];
            let _ = l.layer_command(id, c);
        }
        let n = sizes[(i % sizes.len() as u64) as usize];
        for k in 0..n {
            phase += 0.031;
            if phase > std::f32::consts::TAU {
                phase -= std::f32::consts::TAU;
            }
            input[2 * k] = 0.3 * phase.sin();
            input[2 * k + 1] = 0.3 * phase.cos();
        }
        out[..2 * n].fill(0.0);
        assert!(t.process(n as u32, &mut b));
        l.process(&b, &input[..2 * n], &mut out[..2 * n]);
        sum += out[..2 * n].iter().map(|v| v.abs() as f64).sum::<f64>();
        let s = l.status();
        seen_playing |= s.phase == Phase::Playing;
        layers = layers.max(s.layers);
        if let Some(w) = l.layer(0) {
            sum += w[0] as f64 * 0.0;
        }
        let _ = l.stats();
        let m = l.memory();
        assert!(m.used <= m.capacity);
        for k in 0..MAX_LOOPS {
            let s = l.loop_status(k).unwrap();
            loops_used |= u32::from(s.phase != Phase::Empty) << k;
            if let Some(w) = l.loop_layer(k, 0) {
                sum += w[w.len() - 1] as f64 * 0.0;
            }
        }
    }
    watch(false);
    assert_eq!(CALLS.load(Ordering::SeqCst), 0, "allocations or frees in the looper's real-time calls");
    assert!(seen_playing && layers >= 3 && sum > 1000.0, "{seen_playing} {layers} {sum}");
    assert!(l.stats().refused > 0, "{:?}", l.stats());
    assert!(loops_used.count_ones() >= 3, "loops used: {loops_used:b}");
}
