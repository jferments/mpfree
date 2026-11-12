//! A built-in tuning, or a Scala scale moved to a root and A4, is worked
//! out without allocating, so the audio thread can switch tunings (the pad
//! player does, from the Synth tab's options).
//!
//! This test file is its own program, so its global allocator (a counting
//! pass-through to the system allocator) does not affect the library.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use mpfree_tuning::{root_hz, scala, Builtin, Tuning};

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

// One test, since the count is global and tests run side by side.
#[test]
fn built_in_tunings_and_rooted_scales_never_allocate() {
    let scale = scala::parse_scl("Dorian, just\n7\n9/8\n6/5\n4/3\n3/2\n5/3\n9/5\n2/1\n").unwrap();
    let mut sum = 0.0f32;
    WATCH.store(true, Ordering::Relaxed);
    for b in Builtin::ALL {
        for root in 0..12u8 {
            let t: Tuning = b.tuning(root, 400.0 + f32::from(root) * 5.0);
            sum += t.hz(60.0) + t.hz(67.5);
            let _ = b.degrees();
        }
    }
    let builtins = CALLS.load(Ordering::Relaxed);
    for root in 0..12 {
        let key = 60 + root;
        let t: Tuning = scale.rooted(key, root_hz(key, 380.0 + root as f32 * 8.0));
        sum += t.hz(60.0) + t.hz(67.5);
    }
    WATCH.store(false, Ordering::Relaxed);
    assert!(sum.is_finite() && sum > 0.0);
    assert_eq!(builtins, 0, "a built-in tuning allocated");
    assert_eq!(CALLS.load(Ordering::Relaxed), 0, "a rooted scale allocated");
}
