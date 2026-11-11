//! A lane hands blocks back and forth without allocating, on either side.
//! This file installs the counting allocator, so it holds the tests that
//! read it (the counters are per thread, so tests running side by side do
//! not disturb each other).

use std::time::{Duration, Instant};

use mpfree_lanes::{Config, Lane};
use mpfree_prof::alloc::{self, CountingAlloc};

#[global_allocator]
static ALLOC: CountingAlloc = CountingAlloc;

fn soon() -> Instant {
    Instant::now() + Duration::from_secs(5)
}

#[test]
fn posting_and_joining_allocate_nothing_on_either_thread() {
    assert!(alloc::installed());
    for config in [
        Config::default(),
        Config {
            worker_spin: Duration::from_millis(20),
            join_spin: Duration::ZERO,
            ..Config::default()
        },
        Config {
            threaded: false,
            ..Config::default()
        },
    ] {
        let mut out = vec![0.0f32; 128];
        out[0] = 1.0;
        let mut lane = Lane::new(
            move |b: u64| {
                for (i, s) in out.iter_mut().enumerate() {
                    *s = (b as f32 + i as f32).sin();
                }
            },
            config,
        );
        // Warm up: the first wake-ups may touch lazily made things.
        for b in 0..10 {
            lane.post(b).unwrap();
            lane.join(soon()).unwrap();
        }
        let before = alloc::thread_count();
        for b in 10..2_010 {
            lane.post(b).unwrap();
            lane.join(soon()).unwrap();
        }
        assert_eq!(
            alloc::thread_count() - before,
            0,
            "the posting thread allocated"
        );
        assert_eq!(lane.stats().worker_allocs, 0, "the worker allocated");
    }
}

#[test]
fn a_job_that_allocates_is_counted() {
    let mut lane = Lane::new(
        |b: u64| {
            std::hint::black_box(vec![b; 4]);
        },
        Config::default(),
    );
    for b in 0..5 {
        lane.post(b).unwrap();
        lane.join(soon()).unwrap();
    }
    // Each block allocates once and frees once: two allocator calls.
    assert_eq!(lane.stats().worker_allocs, 10);
}
