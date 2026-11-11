//! The grain engine's real-time calls must not allocate or free. A counting
//! global allocator checks that across busy clouds on a region and on a
//! live buffer: every window, sync, freeze, budget overflow, stop and kill,
//! and the effect as an FX-chain node (parameter events, reset).
//!
//! This test file is its own program, so its global allocator (the only
//! unsafe code here, a pass-through to the system allocator) does not
//! affect the library, which forbids unsafe code. Only the test's own
//! thread is watched, as in mpfree-sampler's no_alloc test.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};

use mpfree_grain::node::param;
use mpfree_grain::*;
use mpfree_node::{Ctx, Node, NodeEvent, Timed};

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
fn grains_never_allocate() {
    let rate = 48_000;
    let sample: Vec<f32> = (0..96_000)
        .map(|i| ((i * 7919) % 2000) as f32 / 1000.0 - 1.0)
        .collect();
    let clip = Clip::new(&sample, 2, 44_100)
        .unwrap()
        .with_range(1000, 40_000);
    let mut live = LiveBuffer::new(2, rate, rate as usize * 4).unwrap();
    let mut voices: Vec<GrainVoice> = (0..6).map(|i| GrainVoice::new(rate, i)).collect();
    let windows = [
        Window::Hann,
        Window::Tukey { taper: 0.4 },
        Window::Trapezoid { ramp: 0.2 },
        Window::Exponential { decay: 8.0 },
    ];
    let mut budget = Budget::new(200);
    let input: Vec<f32> = (0..256).map(|i| (i as f32 * 0.05).sin()).collect();
    let mut l = vec![0.0f32; 128];
    let mut r = vec![0.0f32; 128];
    let mut views = [GrainView::default(); MAX_GRAINS];
    let mut fx = GrainFx::new(rate, 2.0, 9).unwrap();
    fx.set_params(Params {
        position: 0.9,
        spray: 0.2,
        size_ms: 70.0,
        density: 60.0,
        spread: 0.8,
        ..Params::default()
    });
    // The node prepares (allocates) before the watch.
    let mut node = GrainNode::new(5);
    node.prepare(rate, 128);
    node.set_param(param::DENSITY, 120.0);
    let (mut nl, mut nr) = (vec![0.0f32; 128], vec![0.0f32; 128]);

    watch(true);
    let mut t = 0u64;
    for period in 0..2000u32 {
        if period % 250 == 0 {
            for (i, v) in voices.iter_mut().enumerate() {
                let k = (period / 250) as usize + i;
                v.set_params(Params {
                    position: (k % 10) as f32 / 10.0,
                    spray: 0.5,
                    size_ms: 5.0 + (k % 7) as f32 * 60.0,
                    density: 40.0 + (k % 5) as f32 * 80.0,
                    pitch: (k % 9) as f32 - 4.0,
                    pitch_spray: 5.0,
                    scale: if k.is_multiple_of(2) {
                        0b1010_1011_0101
                    } else {
                        0
                    },
                    scan: (k % 5) as f32 - 2.0,
                    window: windows[k % 4],
                    reverse: 0.3,
                    spread: 1.0,
                    gain: 0.5,
                    sync: k.is_multiple_of(3).then_some(TempoSync {
                        frames_per_beat: 24_000.0,
                        grains_per_beat: Some(8.0),
                        size_beats: Some(0.125),
                    }),
                });
                if k % 4 == 3 {
                    v.stop()
                } else {
                    v.start()
                }
            }
        }
        if period == 1200 {
            voices[0].kill(&mut budget);
        }
        live.set_frozen(period % 600 > 400);
        live.write(&input);
        l.fill(0.0);
        r.fill(0.0);
        for (i, v) in voices.iter_mut().enumerate() {
            if i % 2 == 0 {
                v.render(&clip, &mut budget, t, &mut l, &mut r);
            } else {
                v.render(&live, &mut budget, t, &mut l, &mut r);
            }
            v.grains(&mut views);
        }
        live.write_planar(&l, &r);
        // The granular effect on the mix, with mix glides and freeze.
        if period % 100 == 0 {
            fx.set_mix(((period / 100) % 4) as f32 / 3.0);
        }
        fx.set_frozen(period % 300 > 150);
        if period % 500 == 499 {
            fx.set_params(Params {
                position: (period % 7) as f32 / 7.0,
                ..*fx.params()
            });
        }
        fx.process(&mut budget, t, &mut l[..50], &mut r[..50]);
        fx.process(&mut budget, t + 50, &mut l[50..], &mut r[50..]);
        fx.grains(&mut views);
        // The node, with parameter events on odd frames and now and then a reset.
        nl.copy_from_slice(&input[..128]);
        nr.copy_from_slice(&input[128..]);
        let events = [
            Timed::new(
                period % 128,
                NodeEvent::Param {
                    id: param::MIX,
                    value: (period % 5) as f32 * 25.0,
                },
            ),
            Timed::new(
                127,
                NodeEvent::Param {
                    id: param::FREEZE,
                    value: (period % 400 > 200) as u8 as f32,
                },
            ),
        ];
        let events = if events[0].offset <= events[1].offset {
            &events[..]
        } else {
            &events[1..]
        };
        node.process(&Ctx::new(rate, t), events, &mut nl, &mut nr);
        node.set_param(param::SIZE, 20.0 + (period % 9) as f32 * 30.0);
        if period % 700 == 699 {
            node.reset();
        }
        t += 128;
    }
    watch(false);
    assert_eq!(
        CALLS.load(Ordering::Relaxed),
        0,
        "the grain engine allocated or freed"
    );
    assert!(
        budget.skipped > 0,
        "the budget was never full; the test is not busy enough"
    );
    assert!(voices.iter().map(|v| v.stats().spawned).sum::<u64>() > 1000);
    assert!(
        fx.stats().spawned > 100,
        "the effect made grains: {:?}",
        fx.stats()
    );
    assert!(
        node.fx().unwrap().stats().spawned > 100,
        "the node made grains: {:?}",
        node.fx().unwrap().stats()
    );
}
