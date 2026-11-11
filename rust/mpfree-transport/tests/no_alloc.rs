//! The transport's real-time calls (`command`, `process`) and reading a
//! block (spans, ticks, grid lines, markers) must not allocate or free. A
//! counting global allocator checks that across a busy session: count-in,
//! tempo segments, loops, live tempo changes, locates, stop and panic, at
//! period sizes from 1 to 4096 frames.
//!
//! This test file is its own program, so its global allocator (the only
//! unsafe code here, a pass-through to the system allocator) does not
//! affect the library, which forbids unsafe code. Only the test's own
//! thread is watched; the test harness's main thread may allocate at any
//! moment. (The same pattern as mpfree-sampler's no_alloc test.)

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};

use mpfree_transport::time::{Segment, Swing, TempoMap};
use mpfree_transport::{Block, Command, Mark, Transport};

struct Counting;
thread_local! {
    // A const-initialised Cell needs no allocation or destructor, so it is
    // safe to read from inside the allocator.
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
    let mut t = Transport::new(44_100, 120.0).unwrap();
    let map = TempoMap::new(&[
        Segment {
            start: 0,
            bpm: 120.0,
        },
        Segment {
            start: 1920,
            bpm: 90.0,
        },
        Segment {
            start: 5760,
            bpm: 140.0,
        },
    ])
    .unwrap();
    t.set_tempo_map(map).unwrap();
    let mut b = Block::new();

    // The counter itself works: one Vec is one allocation and one free.
    watch(true);
    drop(std::hint::black_box(vec![0u8; 16]));
    watch(false);
    assert_eq!(CALLS.swap(0, Ordering::SeqCst), 2);

    let sizes = [1u32, 7, 128, 333, 4096, 64];
    let mut beats = 0u64;
    let mut sixteenths = 0u64;
    let mut ticks = 0i64;
    let mut marks = 0usize;
    watch(true);
    t.command(0, Command::SetCountIn(1));
    t.command(0, Command::SetLoop(Some((3840, 7680))));
    t.command(0, Command::SetSwing(Swing::new(240, 62).unwrap()));
    t.command(500, Command::Record);
    for i in 0..3000u64 {
        let f = t.frame();
        match i % 400 {
            100 => {
                let _ = t.command(f + 10, Command::SetTempo(80.0 + (i % 7) as f64 * 9.0));
            }
            150 => {
                let _ = t.command(f + 3, Command::ToggleOverdub);
            }
            200 => {
                let _ = t.command(f, Command::Locate((i as i64 * 37) % 9000 - 500));
            }
            250 => {
                let _ = t.command(f + 1, Command::Record);
            }
            300 => {
                let _ = t.command(f + 2, Command::Stop);
            }
            310 => {
                let _ = t.command(f + 2, Command::Stop);
            }
            320 => {
                let _ = t.command(f + 2, Command::Play);
            }
            399 => {
                let _ = t.command(
                    f,
                    Command::SetLoop(if i % 800 == 399 {
                        None
                    } else {
                        Some((0, 3840))
                    }),
                );
            }
            _ => {}
        }
        assert!(t.process(sizes[(i % sizes.len() as u64) as usize], &mut b));
        for s in b.spans() {
            beats += s.lines(s.meter.beat_ticks()).count() as u64;
            for (_, o) in s.lines(240) {
                assert!(o >= s.offset && o < s.offset + s.len);
                sixteenths += 1;
            }
            let r = s.ticks();
            ticks += r.end - r.start;
            if let Some(o) = s.offset_of(s.swing.apply(r.start as f64)) {
                assert!(o < b.frames);
            }
            let _ = s.tick_at(s.frame);
        }
        marks += b
            .markers()
            .iter()
            .filter(|m| !matches!(m.mark, Mark::Swing(_)))
            .count();
        let _ = (t.position(), t.state(), t.bpm(), t.stats());
    }
    watch(false);
    assert_eq!(
        CALLS.load(Ordering::SeqCst),
        0,
        "allocations or frees in the transport's real-time calls"
    );
    assert!(
        beats > 50 && sixteenths > beats && ticks > 10_000 && marks > 40,
        "{beats} {sixteenths} {ticks} {marks}"
    );
    assert_eq!(t.stats().block_full, 0);
}
