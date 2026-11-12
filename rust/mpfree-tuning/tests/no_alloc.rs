//! A built-in tuning is worked out without allocating, so the audio thread
//! can switch tunings (the pad player does, from the Synth tab's options).
//!
//! This test file is its own program, so its global allocator (a counting
//! pass-through to the system allocator) does not affect the library.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use mpfree_tuning::{Builtin, Tuning};

struct Counting;
static WATCH: AtomicBool = AtomicBool::new(false);
static CALLS: AtomicUsize = AtomicUsize::new(0);

fn count() {
    if WATCH.load(Ordering::Relaxed) {
        CALLS.fetch_add(1, Ordering::Relaxed);
    }
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
fn built_in_tunings_never_allocate() {
    let mut sum = 0.0f32;
    WATCH.store(true, Ordering::Relaxed);
    for b in Builtin::ALL {
        for root in 0..12u8 {
            let t: Tuning = b.tuning(root, 400.0 + f32::from(root) * 5.0);
            sum += t.hz(60.0) + t.hz(67.5);
            let _ = b.degrees();
        }
    }
    WATCH.store(false, Ordering::Relaxed);
    assert!(sum.is_finite() && sum > 0.0);
    assert_eq!(
        CALLS.load(Ordering::Relaxed),
        0,
        "a built-in tuning allocated"
    );
}
