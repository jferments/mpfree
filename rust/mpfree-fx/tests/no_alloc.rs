//! The effects' and the chain's real-time calls must not allocate or free.
//! A counting global allocator watches thousands of blocks of busy audio
//! with every parameter moving, bypass switching, tails ringing out and
//! resting, resets, and a node that puts out NaN.
//!
//! This test file is its own program, so its global allocator (the only
//! unsafe code here, a pass-through to the system allocator) does not
//! affect the library, which forbids unsafe code. Only the test's own
//! thread is watched, as in mpfree-grain's no_alloc test.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};

use mpfree_fx::mpfree_node::{Ctx, Kind, Node, NodeEvent, NodeInfo, ParamInfo, Timed, MAX_BLOCK};
use mpfree_fx::*;

struct Counting;
thread_local! {
    static WATCH: Cell<bool> = const { Cell::new(false) };
}
static CALLS: AtomicUsize = AtomicUsize::new(0);

fn count() {
    if WATCH.with(Cell::get) { CALLS.fetch_add(1, Ordering::Relaxed); }
}
fn watch(on: bool) { WATCH.with(|w| w.set(on)); }

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

struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }
    fn unit(&mut self) -> f32 { (self.next() % 1_000_000) as f32 / 1_000_000.0 }
}

/// Puts out NaN every 50th block, so the chain's guard runs.
struct Glitch { n: u64 }

static GLITCH: NodeInfo = NodeInfo { id: "test.glitch", name: "Glitch", kind: Kind::Effect };

impl Node for Glitch {
    fn info(&self) -> &'static NodeInfo { &GLITCH }
    fn params(&self) -> &'static [ParamInfo] { &[] }
    fn set_param(&mut self, _id: u16, _value: f32) {}
    fn param(&self, _id: u16) -> f32 { f32::NAN }
    fn prepare(&mut self, _rate: u32, _max_block: usize) {}
    fn reset(&mut self) {}
    fn process(&mut self, _ctx: &Ctx, _events: &[Timed], left: &mut [f32], _right: &mut [f32]) {
        self.n += 1;
        if self.n.is_multiple_of(50) && !left.is_empty() { left[0] = f32::NAN; }
    }
}

#[test]
fn effects_and_the_chain_never_allocate() {
    let rate = 48_000;
    let mut chain = Chain::new();
    chain.prepare(rate, MAX_BLOCK);
    chain.push(Box::new(Drive::new()));
    chain.push(Box::new(MultiFilter::new()));
    chain.push(Box::new(Crush::new()));
    chain.push(Box::new(Glitch { n: 0 }));
    chain.push(Box::new(Delay::new()));
    chain.push(Box::new(Compressor::new()));
    chain.push(Box::new(DjFilter::new()));
    chain.push(Box::new(MasterSafety::new()));
    // The same nodes on their own, outside a chain.
    let mut solo: Vec<Box<dyn Node>> = vec![
        Box::new(Drive::new()),
        Box::new(MultiFilter::new()),
        Box::new(Crush::new()),
        Box::new(Delay::new()),
        Box::new(Compressor::new()),
        Box::new(DjFilter::new()),
        Box::new(MasterSafety::new()),
    ];
    for n in &mut solo { n.prepare(rate, MAX_BLOCK); }
    let params: Vec<Vec<ParamInfo>> = (0..chain.len()).map(|s| chain.node(s).unwrap().params().to_vec()).collect();
    let solo_params: Vec<Vec<ParamInfo>> = solo.iter().map(|n| n.params().to_vec()).collect();
    let mut rng = Lcg(17);
    let mut l = vec![0.0f32; 512];
    let mut r = vec![0.0f32; 512];
    let key = vec![0.5f32; 512];
    let mut events: Vec<Timed> = Vec::with_capacity(64);
    let mut frame = 0u64;

    watch(true);
    for block in 0..4000u64 {
        let len = 1 + (rng.next() as usize) % 512;
        for i in 0..len {
            let loud = if (block / 200) % 3 == 2 { 3.0 } else { 0.5 };
            l[i] = loud * (rng.unit() * 2.0 - 1.0);
            r[i] = loud * (rng.unit() * 2.0 - 1.0);
        }
        // Parameter moves on every slot, some past the end of the block.
        for (s, ps) in params.iter().enumerate() {
            for p in ps.iter().filter(|p| !p.readonly) {
                if rng.next().is_multiple_of(40) {
                    let at = (rng.next() as usize % (len + 4)) as u32;
                    chain.queue(s, Timed::new(at, NodeEvent::Param { id: p.id, value: p.min + (p.max - p.min) * rng.unit() }));
                }
            }
        }
        if rng.next().is_multiple_of(6) {
            let s = rng.next() as usize % chain.len();
            chain.set_on(s, !chain.is_on(s));
        }
        if rng.next().is_multiple_of(300) { chain.set_param(4, ids::delay::FREEZE, (rng.next() % 2) as f32); }
        let ctx = Ctx { bpm: 80.0 + (block % 7) as f64 * 10.0, sidechain: Some((&key[..len], &key[..len])), ..Ctx::new(rate, frame) };
        chain.process(&ctx, &mut l[..len], &mut r[..len]);

        for (node, ps) in solo.iter_mut().zip(&solo_params) {
            events.clear();
            for p in ps.iter().filter(|p| !p.readonly) {
                if rng.next().is_multiple_of(30) && events.len() < events.capacity() {
                    events.push(Timed::new((rng.next() as usize % len) as u32, NodeEvent::Param { id: p.id, value: p.min + (p.max - p.min) * rng.unit() }));
                }
            }
            events.sort_unstable_by_key(|e| e.offset);
            node.process(&ctx, &events, &mut l[..len], &mut r[..len]);
            if block % 997 == 0 { node.reset(); }
            let _ = (node.tail_frames(), node.latency(), node.param(0));
        }
        frame += len as u64;
    }
    watch(false);

    assert_eq!(CALLS.load(Ordering::Relaxed), 0, "allocations or frees on the audio path");
    assert!(chain.stats().nan_resets > 0, "the NaN guard ran");
    assert!(l.iter().chain(&r).all(|v| v.is_finite()));
}
