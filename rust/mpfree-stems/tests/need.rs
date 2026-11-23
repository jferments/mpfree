//! A separation's memory, as promised before it starts ([`Model::need`]),
//! against what it really holds at its most, on Spleeter's real shapes
//! (seeded weights) for both bands.
//!
//! This file installs its own allocator, which keeps the bytes held by
//! every thread together (a separation's layers allocate on the threads
//! they start) and the most they reached. It holds one test, so nothing
//! else allocates beside the run it measures.

use mpfree_stems::{Band, Model, Separator, Settings, Sink, Stem, RATE};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicI64, Ordering};

static HELD: AtomicI64 = AtomicI64::new(0);
static MOST: AtomicI64 = AtomicI64::new(0);

fn count(bytes: i64) {
    let now = HELD.fetch_add(bytes, Ordering::Relaxed) + bytes;
    MOST.fetch_max(now, Ordering::Relaxed);
}

struct Peak;

// SAFETY: every call goes straight to the system allocator; the counting
// touches only two atomics, which never allocate.
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

/// Keeps nothing: the stems' own memory is the caller's to promise.
struct Drop;

impl Sink for Drop {
    fn put(&mut self, _: Stem, stereo: &[f32]) -> Result<(), String> {
        assert!(stereo.iter().all(|x| x.is_finite()));
        Ok(())
    }
}

#[test]
fn a_separation_holds_no_more_than_its_promise_and_not_much_less() {
    let model = Model::synthetic(11);
    let mut seed = 7u32;
    // 3 s: one segment (it holds the same for any number of them; the
    // crate's own tests run many segments on small networks).
    let input: Vec<f32> = (0..3 * RATE as usize * 2)
        .map(|_| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            ((seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5) * 0.5
        })
        .collect();
    for band in [Band::Full, Band::Spleeter] {
        let settings = Settings { band, threads: 2 };
        let promise = model.need(&settings) as i64;
        let base = HELD.load(Ordering::Relaxed);
        MOST.store(base, Ordering::Relaxed);
        {
            let mut sep = Separator::new(&model, &input, 2, RATE, &settings).unwrap();
            while sep.step(&mut Drop).unwrap() {}
        }
        let most = MOST.load(Ordering::Relaxed) - base;
        // Read before printing: the test harness's captured output grows
        // on the heap too.
        let kept = HELD.load(Ordering::Relaxed) - base;
        let mb = |b: i64| b as f64 / 1e6;
        println!(
            "{band:?}: promised {:.1} MB, held at most {:.1} MB",
            mb(promise),
            mb(most)
        );
        assert!(
            most <= promise,
            "{band:?}: held {most} bytes, promised {promise}"
        );
        assert!(
            most as f64 >= 0.95 * promise as f64,
            "{band:?}: held {most} bytes, promised {promise}: the promise is loose"
        );
        assert_eq!(kept, 0, "{band:?}: it kept {kept} bytes");
    }
}
