//! A stretch job's memory, as promised before it starts
//! ([`JobSpec::need`]), against what it really holds at its most: every
//! step of the job, and the end, where the worker turns the copy into the
//! `Arc<[f32]>` it hands back (`warp::Done::Copy`). Train 147's CI held a
//! copy's 21 MB over the sampler's memory test when that last copy came
//! outside the promise; a copy longer than its region, or one with a tune,
//! went over its promise the same way.
//!
//! This file installs its own allocator, which keeps each thread's bytes
//! held and the most they reached.

use mpfree_sampler::model::Source;
use mpfree_sampler::warp::JobSpec;
use mpfree_stretch::{Settings, TimeMap};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::Arc;

thread_local! {
    static HELD: Cell<i64> = const { Cell::new(0) };
    static MOST: Cell<i64> = const { Cell::new(0) };
}

fn count(bytes: i64) {
    let _ = HELD.try_with(|h| {
        let now = h.get() + bytes;
        h.set(now);
        let _ = MOST.try_with(|m| m.set(m.get().max(now)));
    });
}

struct Peak;

// SAFETY: every call goes straight to the system allocator; the counting
// touches only const-initialised thread-locals, which never allocate.
#[allow(unsafe_code)]
unsafe impl GlobalAlloc for Peak {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        count(l.size() as i64);
        unsafe { System.alloc(l) }
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        count(l.size() as i64);
        unsafe { System.alloc_zeroed(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        count(-(l.size() as i64));
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        count(n as i64 - l.size() as i64);
        unsafe { System.realloc(p, l, n) }
    }
}

#[global_allocator]
static ALLOC: Peak = Peak;

const RATE: u32 = 44_100;

/// A 16-bit stereo loop of `secs` s with a hit every half second, as a
/// take loaded from a WAV is held.
fn take(secs: usize) -> Source {
    let n = secs * RATE as usize;
    let data: Vec<f32> = (0..2 * n)
        .map(|i| {
            let f = i / 2;
            let t = (f % (RATE as usize / 2)) as f32;
            let x = 0.6 * (-t / 800.0).exp() * (f as f32 * 0.05).sin()
                + 0.05 * (f as f32 * 0.0313 * (1 + i % 2) as f32).sin();
            // + 0.0: no negative zero, which 16 bits can't hold.
            ((x * 32_768.0).round() + 0.0) / 32_768.0
        })
        .collect();
    let s = Source::new("Loop", RATE, 2, data).unwrap();
    assert!(s.data().is_16_bit());
    s
}

/// The most this thread holds above what it held as `f` started, while
/// `f` runs.
fn most_in(f: impl FnOnce()) -> i64 {
    let before = HELD.with(Cell::get);
    MOST.with(|m| m.set(before));
    f();
    MOST.with(Cell::get) - before
}

#[test]
fn a_stretch_job_never_holds_more_than_it_promised() {
    let source = take(6);
    let frames = source.frames();
    // (output length over input, semitones): slower and faster tempos,
    // the same length with a tune, both together, the furthest tunes, and
    // a copy that only moves the pitch back (nothing to stretch).
    let cases: [(f64, f32); 9] = [
        (1.6, 0.0),
        (0.6, 0.0),
        (1.0, 0.0),
        (1.0, 7.0),
        (1.0, -12.0),
        (1.3, 3.0),
        (0.8, -5.0),
        (1.0, 24.0),
        (1.0, -24.0),
    ];
    for (ratio, semitones) in cases {
        let span = frames / 4..frames;
        let input = span.end - span.start;
        let out = (input as f64 * ratio).round() as u64;
        let spec = JobSpec {
            source: source.clone(),
            span,
            map: TimeMap::to_length(input, out).unwrap(),
            settings: Settings {
                semitones,
                ..Settings::default()
            },
        };
        let need = spec.need() as i64;
        let mut copy: Option<Arc<[f32]>> = None;
        let most = most_in(|| {
            let mut job = spec.start().unwrap();
            while !job.run(8192).finished {}
            let made = job.into_output().unwrap();
            // As the worker hands it back (warp.rs, Done::Copy).
            copy = Some(Arc::from(made));
        });
        let copy = copy.unwrap();
        assert_eq!(copy.len() as u64, out * 2);
        println!(
            "x{ratio} {semitones:+} st: at most {:.2} MB, {:.2} MB promised",
            most as f64 / 1e6,
            need as f64 / 1e6
        );
        assert!(
            most <= need,
            "x{ratio} {semitones:+} st holds {most} bytes at its most, over the {need} promised"
        );
        // Close, too: a promise far over what it holds turns loads away
        // that would fit.
        let copy_bytes = out as i64 * 2 * 4;
        assert!(
            need - most <= (1 << 20) + copy_bytes / 10,
            "x{ratio} {semitones:+} st promises {need} bytes for {most}"
        );
    }
}
