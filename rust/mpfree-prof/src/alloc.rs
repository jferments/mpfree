//! An allocator that counts, per thread, so the audio thread can prove it
//! never allocates.
//!
//! It exists only with the crate's `counting-alloc` feature (mpfree-bench
//! turns it on; the pad player does not), so the unsafe code below is not
//! even compiled into programs that do not ask for it. Without the feature
//! [`thread_count`] is always 0 and [`installed`] is false.
//!
//! A program opts in by enabling the feature and installing it:
//!
//! ```ignore
//! #[global_allocator]
//! static ALLOC: mpfree_prof::alloc::CountingAlloc = mpfree_prof::alloc::CountingAlloc;
//! ```
//!
//! It passes every call to the system allocator and adds one to a counter
//! that belongs to the calling thread: a `const`-initialised thread-local
//! `Cell`, which needs no allocation and no destructor, so reading it inside
//! the allocator is safe. The cost is one thread-local add per call, so it
//! can stay installed in device builds. [`crate::Span`] and
//! [`crate::Profiled`] read the counter around each block; without the
//! allocator the count stays 0 and [`installed`] says so.
#![cfg_attr(feature = "counting-alloc", allow(unsafe_code))]

#[cfg(feature = "counting-alloc")]
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicU64, Ordering};

thread_local! {
    static CALLS: Cell<u64> = const { Cell::new(0) };
}

static TOTAL: AtomicU64 = AtomicU64::new(0);

#[cfg(feature = "counting-alloc")]
#[inline]
fn count() {
    // `try_with`: a thread that is being torn down may still free memory.
    let _ = CALLS.try_with(|c| c.set(c.get().wrapping_add(1)));
    TOTAL.fetch_add(1, Ordering::Relaxed);
}

/// The counting pass-through allocator. See the module docs.
#[cfg(feature = "counting-alloc")]
pub struct CountingAlloc;

#[cfg(feature = "counting-alloc")]
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        count();
        unsafe { System.alloc(l) }
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        count();
        unsafe { System.alloc_zeroed(l) }
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

/// Allocator calls (alloc, free, realloc) made by this thread so far. 0
/// forever when [`CountingAlloc`] is not installed.
#[inline]
pub fn thread_count() -> u64 { CALLS.try_with(Cell::get).unwrap_or(0) }

/// Allocator calls by every thread so far.
pub fn total_count() -> u64 { TOTAL.load(Ordering::Relaxed) }

/// Whether [`CountingAlloc`] is the program's allocator: makes and drops one
/// small box and looks for it in the counter. Not for the audio thread.
pub fn installed() -> bool {
    let before = thread_count();
    drop(std::hint::black_box(Box::new(0u64)));
    thread_count() != before
}

/// Runs `f` and returns its result with the allocator calls this thread
/// made inside it.
pub fn count_in<R>(f: impl FnOnce() -> R) -> (R, u64) {
    let before = thread_count();
    let r = f();
    (r, thread_count().wrapping_sub(before))
}
