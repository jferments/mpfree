//! The coverage effects' real-time calls never allocate or free: a
//! counting allocator watches busy audio with every parameter moving
//! (choices, sizes and times included), resets, tempo changes and a
//! sidechain, as `no_alloc.rs` does for the older effects. This file is its
//! own program, so its allocator (the only unsafe code here) touches no
//! other test.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};

use mpfree_fx::mpfree_node::{Ctx, NodeEvent, Timed, MAX_BLOCK};
use mpfree_fx::registry;

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

const IDS: &[&str] = &[
    "mpfree.fx.tremolo",
    "mpfree.fx.autopan",
    "mpfree.fx.ringmod",
    "mpfree.fx.rotary",
    "mpfree.fx.limiter",
    "mpfree.fx.gate",
    "mpfree.fx.deesser",
    "mpfree.fx.multiband",
    "mpfree.fx.tape_delay",
    "mpfree.fx.multitap",
    "mpfree.fx.room",
    "mpfree.fx.plate",
    "mpfree.fx.spring",
    "mpfree.fx.amp",
    "mpfree.fx.exciter",
    "mpfree.fx.xy",
    "mpfree.fx.autowah",
    "mpfree.fx.talkbox",
    "mpfree.fx.freqshift",
    "mpfree.fx.width",
    "mpfree.fx.sample_delay",
    "mpfree.fx.channel_strip",
    "mpfree.fx.granulator",
];

#[test]
fn process_and_reset_never_allocate() {
    let mut seed = 99u64;
    let mut rnd = move || {
        seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (seed >> 33) as u32
    };
    for id in IDS {
        let mut n = registry::make(id).unwrap();
        n.prepare(48_000, MAX_BLOCK);
        let params = n.params().to_vec();
        let mut l = vec![0.0f32; MAX_BLOCK];
        let mut r = vec![0.0f32; MAX_BLOCK];
        let key = vec![0.3f32; MAX_BLOCK];
        let mut events = Vec::with_capacity(16);
        CALLS.store(0, Ordering::Relaxed);
        WATCH.with(|w| w.set(true));
        for b in 0..600u64 {
            events.clear();
            for k in 0..3 {
                let p = &params[rnd() as usize % params.len()];
                let t = (rnd() % 1000) as f32 / 1000.0;
                events.push(Timed {
                    offset: (k * 40) as u32,
                    ev: NodeEvent::Param {
                        id: p.id,
                        value: p.from_unit(t),
                    },
                });
            }
            for i in 0..MAX_BLOCK {
                l[i] = ((rnd() % 2000) as f32 / 1000.0 - 1.0) * 0.8;
                r[i] = l[i] * 0.9;
            }
            let mut ctx = Ctx::new(48_000, b * MAX_BLOCK as u64);
            ctx.bpm = 80.0 + (b % 7) as f64 * 15.0;
            ctx.playing = b % 50 > 5;
            ctx.beats = b as f64 * 0.1;
            ctx.sidechain = Some((&key, &key));
            n.process(&ctx, &events, &mut l, &mut r);
            if b % 150 == 149 {
                n.reset();
            }
            let _ = n.param(params[0].id);
        }
        WATCH.with(|w| w.set(false));
        assert_eq!(CALLS.load(Ordering::Relaxed), 0, "{id} allocated");
    }
}
