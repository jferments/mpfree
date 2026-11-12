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

struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }
    fn unit(&mut self) -> f32 {
        (self.next() % 1_000_000) as f32 / 1_000_000.0
    }
}

/// Puts out NaN every 50th block, so the chain's guard runs.
struct Glitch {
    n: u64,
}

static GLITCH: NodeInfo = NodeInfo {
    id: "test.glitch",
    name: "Glitch",
    kind: Kind::Effect,
};

impl Node for Glitch {
    fn info(&self) -> &'static NodeInfo {
        &GLITCH
    }
    fn params(&self) -> &'static [ParamInfo] {
        &[]
    }
    fn set_param(&mut self, _id: u16, _value: f32) {}
    fn param(&self, _id: u16) -> f32 {
        f32::NAN
    }
    fn prepare(&mut self, _rate: u32, _max_block: usize) {}
    fn reset(&mut self) {}
    fn process(&mut self, _ctx: &Ctx, _events: &[Timed], left: &mut [f32], _right: &mut [f32]) {
        self.n += 1;
        if self.n.is_multiple_of(50) && !left.is_empty() {
            left[0] = f32::NAN;
        }
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
    chain.push(Box::new(Cloud::new()));
    chain.push(Box::new(Eq::new()));
    chain.push(Box::new(Chorus::new()));
    chain.push(Box::new(Flanger::new()));
    chain.push(Box::new(Phaser::new()));
    chain.push(Box::new(LoFi::new()));
    chain.push(Box::new(Comb::new()));
    chain.push(Box::new(Character::new()));
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
        Box::new(Cloud::new()),
        Box::new(Eq::new()),
        Box::new(Chorus::new()),
        Box::new(Flanger::new()),
        Box::new(Phaser::new()),
        Box::new(LoFi::new()),
        Box::new(Comb::new()),
        Box::new(Character::new()),
    ];
    for n in &mut solo {
        n.prepare(rate, MAX_BLOCK);
    }
    let params: Vec<Vec<ParamInfo>> = (0..chain.len())
        .map(|s| chain.node(s).unwrap().params().to_vec())
        .collect();
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
                    chain.queue(
                        s,
                        Timed::new(
                            at,
                            NodeEvent::Param {
                                id: p.id,
                                value: p.min + (p.max - p.min) * rng.unit(),
                            },
                        ),
                    );
                }
            }
        }
        if rng.next().is_multiple_of(6) {
            let s = rng.next() as usize % chain.len();
            chain.set_on(s, !chain.is_on(s));
        }
        if rng.next().is_multiple_of(300) {
            chain.set_param(4, ids::delay::FREEZE, (rng.next() % 2) as f32);
        }
        if rng.next().is_multiple_of(300) {
            chain.set_param(7, ids::cloud::FREEZE, (rng.next() % 2) as f32);
        }
        let ctx = Ctx {
            bpm: 80.0 + (block % 7) as f64 * 10.0,
            sidechain: Some((&key[..len], &key[..len])),
            ..Ctx::new(rate, frame)
        };
        chain.process(&ctx, &mut l[..len], &mut r[..len]);

        for (node, ps) in solo.iter_mut().zip(&solo_params) {
            events.clear();
            for p in ps.iter().filter(|p| !p.readonly) {
                if rng.next().is_multiple_of(30) && events.len() < events.capacity() {
                    events.push(Timed::new(
                        (rng.next() as usize % len) as u32,
                        NodeEvent::Param {
                            id: p.id,
                            value: p.min + (p.max - p.min) * rng.unit(),
                        },
                    ));
                }
            }
            events.sort_unstable_by_key(|e| e.offset);
            node.process(&ctx, &events, &mut l[..len], &mut r[..len]);
            if block % 997 == 0 {
                node.reset();
            }
            let _ = (node.tail_frames(), node.latency(), node.param(0));
        }
        frame += len as u64;
    }
    watch(false);

    assert_eq!(
        CALLS.load(Ordering::Relaxed),
        0,
        "allocations or frees on the audio path"
    );
    assert!(chain.stats().nan_resets > 0, "the NaN guard ran");
    assert!(l.iter().chain(&r).all(|v| v.is_finite()));
}

#[test]
fn the_mixer_never_allocates() {
    mixer_never_allocates(false);
}

#[test]
fn the_mixer_with_inserts_on_a_second_core_never_allocates() {
    mixer_never_allocates(true);
}

fn mixer_never_allocates(lane: bool) {
    let rate = 48_000;
    let max = 512;
    let mut m = Mixer::new(MixerConfig::default(), rate, max);
    m.chain_mut(StripId::Track(0))
        .unwrap()
        .push(Box::new(Compressor::new()));
    m.chain_mut(StripId::Track(1))
        .unwrap()
        .push(Box::new(Drive::new()));
    m.chain_mut(StripId::Track(2))
        .unwrap()
        .push(Box::new(Glitch { n: 0 }));
    m.chain_mut(StripId::Pad(0))
        .unwrap()
        .push(Box::new(Eq::new()));
    m.chain_mut(StripId::Input(0))
        .unwrap()
        .push(Box::new(Chorus::new()));
    m.chain_mut(StripId::Bus(0))
        .unwrap()
        .push(Box::new(Compressor::new()));
    m.chain_mut(StripId::Return(0))
        .unwrap()
        .push(Box::new(Cloud::new()));
    m.chain_mut(StripId::Return(1))
        .unwrap()
        .push(Box::new(Delay::new()));
    m.chain_mut(StripId::Master)
        .unwrap()
        .push(Box::new(Eq::new()));
    m.chain_mut(StripId::Track(0))
        .unwrap()
        .set_param(0, ids::compressor::SIDECHAIN, 1.0);
    // Inserts to hand over while playing, built here, and room for what
    // comes back.
    let mut spare: Vec<Insert> = (0..40)
        .map(|i| -> Insert {
            let node: Box<dyn Node> = match i % 4 {
                0 => Box::new(Drive::new()),
                1 => Box::new(MultiFilter::new()),
                2 => Box::new(Phaser::new()),
                _ => Box::new(MasterSafety::new()),
            };
            Insert::new(node, rate, max)
        })
        .collect();
    let mut back: Vec<Insert> = Vec::with_capacity(64);
    if lane {
        m.set_lane(
            Some(mpfree_lanes::Config::default()),
            std::time::Duration::from_secs(1),
        );
    }
    let strips: Vec<StripId> = m.strips().collect();
    let tracks = [
        StripId::Track(0),
        StripId::Track(3),
        StripId::Pad(1),
        StripId::Bus(2),
        StripId::Input(1),
    ];
    let mut rng = Lcg(23);
    let mut l = vec![0.0f32; max];
    let mut r = vec![0.0f32; max];
    let mut frame = 0u64;

    watch(true);
    for block in 0..3000u64 {
        let len = 1 + (rng.next() as usize) % max;
        for t in 0..m.tracks() {
            if (block / 50 + t as u64).is_multiple_of(3) {
                continue;
            }
            let (il, ir) = m.track_input(t).unwrap();
            for i in 0..len {
                il[i] = 0.4 * (rng.unit() * 2.0 - 1.0);
                ir[i] = il[i];
            }
        }
        if block % 7 == 0 {
            let (pl, _) = m.pad_input(5).unwrap();
            pl[0] = if block % 700 == 0 { f32::NAN } else { 0.8 };
        }
        {
            let (hl, hr) = m.hw_input();
            for i in 0..len {
                hl[i] = 0.3 * (rng.unit() * 2.0 - 1.0);
                hr[i] = 0.3 * (rng.unit() * 2.0 - 1.0);
            }
        }
        for _ in 0..(rng.next() % 6) {
            let s = strips[rng.next() as usize % strips.len()];
            let p = &strip_params()[rng.next() as usize % strip_params().len()];
            let at = (rng.next() as usize % (len + 4)) as u32;
            m.queue(
                s,
                Timed::new(
                    at,
                    NodeEvent::Param {
                        id: p.id,
                        value: p.min + (p.max - p.min) * rng.unit(),
                    },
                ),
            );
        }
        if rng.next().is_multiple_of(40) {
            let s = tracks[rng.next() as usize % tracks.len()];
            let len_now = m.chain(s).unwrap().len();
            let e = match (rng.next() % 4, spare.pop()) {
                (0, Some(ins)) | (1, Some(ins)) => Edit::Insert {
                    strip: s,
                    at: rng.next() as usize % (len_now + 1),
                    insert: ins,
                },
                (2, Some(ins)) if len_now > 0 => Edit::Replace {
                    strip: s,
                    at: rng.next() as usize % len_now,
                    insert: ins,
                },
                (_, ins) => {
                    if let Some(ins) = ins {
                        spare.push(ins);
                    }
                    if len_now > 1 && rng.next().is_multiple_of(2) {
                        Edit::Move {
                            strip: s,
                            from: 0,
                            to: len_now - 1,
                        }
                    } else {
                        Edit::Remove {
                            strip: s,
                            at: rng.next() as usize % (len_now + 1),
                        }
                    }
                }
            };
            if let Err(Edit::Insert { insert, .. } | Edit::Replace { insert, .. }) = m.edit(e) {
                spare.push(insert);
            }
        }
        if rng.next().is_multiple_of(25) {
            let key = if rng.next().is_multiple_of(3) {
                None
            } else {
                Some(strips[rng.next() as usize % strips.len()])
            };
            m.set_key(tracks[rng.next() as usize % tracks.len()], 0, key);
            m.set_key(
                StripId::Track(0),
                0,
                Some(StripId::Track((rng.next() % 16) as usize)),
            );
        }
        if rng.next().is_multiple_of(30) {
            let src = strips[rng.next() as usize % strips.len()];
            let point = [
                CapturePoint::BeforeInserts,
                CapturePoint::BeforeFader,
                CapturePoint::AfterFader,
            ][rng.next() as usize % 3];
            m.set_capture((rng.next() % CAPTURES as u64) as usize, Some((src, point)));
        }
        m.set_capture(7, Some((StripId::Master, CapturePoint::AfterSafety)));
        let ctx = Ctx {
            bpm: 90.0,
            ..Ctx::new(rate, frame)
        };
        m.process(&ctx, &mut l[..len], &mut r[..len]);
        let heard = m
            .capture(7)
            .map_or(0.0, |(a, _)| a.first().copied().unwrap_or(0.0));
        let _ = (
            heard,
            m.capture_latency(3),
            m.latency(),
            m.take_peaks(StripId::Master),
            m.take_input_peaks(StripId::Input(0)),
        );
        let _ = (
            m.stats(),
            m.is_audible(StripId::Track(4)),
            m.destination(StripId::Pad(2)),
            m.param(StripId::Bus(1), 4),
        );
        while let Some(ins) = m.pop_retired() {
            if back.len() < back.capacity() {
                back.push(ins)
            } else {
                spare.push(ins)
            }
        }
        if block % 1111 == 0 {
            m.reset();
        }
        // Profiling on and off while playing, with the screen taking its
        // windows.
        if block % 500 == 0 {
            m.set_profiling(block % 1000 == 0);
        }
        if block % 100 == 0 {
            let _ = (
                m.probe_mut().take_window(),
                m.strip_probe_mut(StripId::Track(0))
                    .map(|p| p.take_window()),
            );
            let _ = m
                .chain_mut(StripId::Track(0))
                .and_then(|c| c.probe_mut(0))
                .map(|p| p.take_window());
        }
        frame += len as u64;
    }
    watch(false);

    assert_eq!(
        CALLS.load(Ordering::Relaxed),
        0,
        "allocations or frees in the mixer"
    );
    assert!(m.stats().bad_input > 0, "the input guard ran");
    assert!(m.stats().nan_resets > 0, "the NaN guard ran");
    assert!(l.iter().chain(&r).all(|v| v.is_finite()));
    assert!(m.probe().lifetime().blocks > 0, "profiling recorded blocks");
    if lane {
        assert!(m.stats().lane_posts > 0, "the lane had inserts");
    }
    drop(back);
}

#[test]
fn perform_fx_never_allocates() {
    // Every pad pressed and let go at random, with the transport playing
    // and stopped, settings moving, the echo waking and sleeping, resets.
    let rate = 48_000;
    let mut p = Perform::new();
    p.prepare(rate, MAX_BLOCK);
    let params = p.params().to_vec();
    let mut rng = Lcg(23);
    let mut l = vec![0.0f32; 512];
    let mut r = vec![0.0f32; 512];
    let mut events: Vec<Timed> = Vec::with_capacity(64);
    let mut frame = 0u64;
    watch(true);
    for block in 0..6000u64 {
        let len = 1 + (rng.next() as usize) % 512;
        for i in 0..len {
            l[i] = 0.5 * (rng.unit() * 2.0 - 1.0);
            r[i] = 0.5 * (rng.unit() * 2.0 - 1.0);
        }
        events.clear();
        for _ in 0..(rng.next() % 4) {
            let at = (rng.next() as usize % (len + 2)) as u32;
            let note = (rng.next() % 18) as u8;
            let ev = match rng.next() % 6 {
                0 | 1 => NodeEvent::NoteOn {
                    note,
                    velocity: 100,
                },
                2 | 3 => NodeEvent::NoteOff { note },
                4 => {
                    let q = &params[rng.next() as usize % params.len()];
                    NodeEvent::Param {
                        id: q.id,
                        value: q.min + (q.max - q.min) * rng.unit(),
                    }
                }
                _ => NodeEvent::AllOff,
            };
            events.push(Timed::new(at, ev));
        }
        events.sort_unstable_by_key(|e| e.offset);
        let bpm = 70.0 + (block / 500) as f64 * 15.0;
        let ctx = Ctx {
            bpm,
            beats: frame as f64 * bpm / 60.0 / rate as f64,
            playing: (block / 700) % 4 != 3,
            ..Ctx::new(rate, frame)
        };
        p.process(&ctx, &events, &mut l[..len], &mut r[..len]);
        if block % 1999 == 0 {
            p.reset();
        }
        let _ = (p.tail_frames(), p.param(5));
        frame += len as u64;
    }
    watch(false);
    assert_eq!(
        CALLS.load(Ordering::Relaxed),
        0,
        "allocations or frees on the audio path"
    );
    assert!(l.iter().chain(&r).all(|v| v.is_finite()));
}

#[test]
fn freezing_and_printing_never_allocate() {
    // Takes armed, recorded, failed and handed out; strips frozen, thawed
    // and given back; the transport stopping and starting; the freeze
    // crossfades in every block size.
    let rate = 48_000;
    let cfg = MixerConfig {
        inputs: 0,
        pads: 2,
        tracks: 2,
        slots: 2,
        max_latency_ms: 20.0,
    };
    let mut m = Mixer::new(cfg, rate, MAX_BLOCK);
    m.chain_mut(StripId::Track(0))
        .unwrap()
        .push(Box::new(Delay::new()));
    m.set_capture(0, Some((StripId::Track(0), CapturePoint::BeforeFader)));
    let mut prints: Vec<Print> = (0..4).map(|_| Print::new(rate, 3.0)).collect();
    let mut taken: Vec<Printed> = Vec::with_capacity(8);
    let mut back: Vec<Frozen> = Vec::with_capacity(8);
    let mut spare: Vec<Frozen> = (0..4)
        .map(|_| Frozen {
            left: vec![0.25; 9_000],
            right: vec![-0.25; 9_000],
            origin: 0.0,
            beats: 1.0,
            bpm: 120.0,
            rate,
        })
        .collect();
    let mut rng = Lcg(31);
    let mut l = vec![0.0f32; MAX_BLOCK];
    let mut r = vec![0.0f32; MAX_BLOCK];
    let (mut frame, mut beats) = (0u64, 0.0f64);
    let mut which = 0;
    watch(true);
    for block in 0..4000u64 {
        let len = 1 + (rng.next() as usize) % 700;
        let playing = (block / 600) % 5 != 4;
        let bpm = if (block / 1300) % 2 == 0 {
            120.0
        } else {
            100.0
        };
        if m.needs_input(StripId::Track(0)) {
            let (il, ir) = m.strip_input(StripId::Track(0)).unwrap();
            for i in 0..len {
                il[i] = 0.3 * (rng.unit() * 2.0 - 1.0);
                ir[i] = 0.3 * (rng.unit() * 2.0 - 1.0);
            }
        }
        match rng.next() % 300 {
            0 => {
                let p = &mut prints[which % 4];
                let _ = p.arm(PrintJob {
                    start: beats.floor() + 1.0,
                    beats: 0.5 + (rng.next() % 4) as f64 * 0.5,
                    steady: rng.next().is_multiple_of(2),
                    tail: if rng.next().is_multiple_of(2) {
                        Tail::Cut
                    } else {
                        Tail::UntilSilent { max_ms: 200.0 }
                    },
                });
            }
            1 => prints[which % 4].cancel(),
            2 => {
                if let Some(f) = spare.pop() {
                    let s = if rng.next().is_multiple_of(2) {
                        StripId::Track(0)
                    } else {
                        StripId::Pad(1)
                    };
                    if let Err(f) = m.freeze(s, f) {
                        spare.push(f);
                    }
                }
            }
            3 => {
                m.thaw(StripId::Track(0));
                m.thaw(StripId::Pad(1));
            }
            4 => {
                m.set_frozen_origin(StripId::Track(0), beats.floor());
            }
            _ => {}
        }
        let ctx = Ctx {
            bpm,
            beats,
            playing,
            ..Ctx::new(rate, frame)
        };
        m.process(&ctx, &mut l[..len], &mut r[..len]);
        for p in prints.iter_mut() {
            p.process(&ctx, m.capture(0), m.capture_latency(0).unwrap_or(0));
            let _ = (p.progress(), p.state(), p.start_beat());
        }
        let p = &mut prints[which % 4];
        if let Some(t) = p.take() {
            if taken.len() < taken.capacity() {
                taken.push(t);
            }
            which += 1;
        }
        while let Some(f) = m.pop_thawed() {
            if back.len() < back.capacity() {
                back.push(f);
            }
        }
        let _ = (m.freeze_state(StripId::Track(0)), m.frozen(StripId::Pad(1)));
        frame += len as u64;
        if playing {
            beats += len as f64 * bpm / 60.0 / rate as f64;
        }
    }
    watch(false);
    assert_eq!(
        CALLS.load(Ordering::Relaxed),
        0,
        "allocations or frees on the audio path"
    );
    assert!(l.iter().chain(&r).all(|v| v.is_finite()));
    assert!(!taken.is_empty(), "some takes finished");
    drop((taken, back, spare, prints));
}
