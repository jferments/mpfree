//! `Capture::input`, `Capture::process` and `Capture::pending` must not
//! allocate or free. A counting global allocator watches a capture that
//! follows a busy transport (play, loops, tempo changes, locates, stop,
//! free playing while stopped) for long enough that the history wraps,
//! at period sizes from 1 to 4096 frames.
//!
//! This test file is its own program, so its global allocator (the only
//! unsafe code here, a pass-through to the system allocator) does not
//! affect the library, which forbids unsafe code. Only the test's own
//! thread is watched. (The same pattern as mpfree-play's no_alloc test.)

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};

use mpfree_capture::{Capture, MAX_HITS};
use mpfree_play::Event;
use mpfree_transport::{Block, Command, Transport};

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
fn capturing_never_allocates() {
    for period in [1u32, 128, 4096] {
        let mut t = Box::new(Transport::new(44_100, 120.0).unwrap());
        let mut block = Box::new(Block::new());
        let mut c = Box::new(Capture::new());
        let mut kept = 0u64;
        // About 30 seconds: enough periods to fill and wrap the history
        // with a note start and end every 32 frames.
        let periods = (44_100 * 30 / period as usize).max(MAX_HITS * 20 / period as usize + 1);
        let total = periods as u64 * u64::from(period);
        assert!(total / 32 > MAX_HITS as u64, "the history must wrap");
        let cmds = [(0.05, Command::Play), (0.1, Command::SetLoop(Some((0, 3840 * 2)))), (0.2, Command::SetTempo(97.0)),
                    (0.3, Command::Locate(3840 * 5)), (0.4, Command::SetLoop(None)), (0.5, Command::Stop),
                    (0.7, Command::Record), (0.9, Command::Stop)];
        let mut next_cmd = 0;
        watch(true);
        for p in 0..periods as u64 {
            let start = p * u64::from(period);
            while next_cmd < cmds.len() && (cmds[next_cmd].0 * total as f64) as u64 <= start {
                assert!(t.command(start, cmds[next_cmd].1));
                next_cmd += 1;
            }
            for off in 0..period {
                let f = start + u64::from(off);
                if f % 32 == 0 {
                    let note = (f / 32 % 16) as u8;
                    let ev = if f / 32 % 2 == 0 { Event::NoteOn { note, velocity: 100 } } else { Event::NoteOff { note: note - 1 } };
                    c.input(off, ev);
                }
            }
            assert!(t.process(period, &mut block));
            c.process(&block);
            if let Some(x) = c.pending() {
                kept += x.bars as u64;
            }
        }
        watch(false);
        assert_eq!(CALLS.load(Ordering::Relaxed), 0, "period {period}: allocated or freed");
        assert_eq!(c.len(), MAX_HITS);
        assert_eq!(c.dropped(), 0);
        assert!(kept > 0);
        // The phrase is read afterwards, off the watched path.
        assert!(c.phrase().is_some());
    }
}
