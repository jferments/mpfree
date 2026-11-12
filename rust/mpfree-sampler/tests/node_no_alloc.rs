//! The sampler node's `process` must not allocate or free: notes on and
//! off on pads and keygroups, pressure, a note held again, "Pad 1 note",
//! Level, kit Tune and Cutoff and every pad setting changing inside a
//! block, all notes off, reset, and views swapped in (and old ones handed
//! back, and the pad settings put on the new ones) while voices play.
//!
//! This test file is its own program, so its global allocator (the only
//! unsafe code here, a pass-through to the system allocator) does not
//! affect the library, which forbids unsafe code. Only the test's own
//! thread is watched, and only inside `process` and `reset`: the control
//! side (building, sending and dropping views) runs between blocks, as it
//! would on the control thread.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};

use mpfree_node::{Ctx, Node, NodeEvent, Timed};
use mpfree_sampler::engine::EngineView;
use mpfree_sampler::model::*;
use mpfree_sampler::node::{field, ids, pad_id, SamplerNode, DEFAULT_BASE_NOTE, LIVE_PADS};

struct Counting;
thread_local! {
    // A const-initialised Cell needs no allocation or destructor, so it is
    // safe to read from inside the allocator.
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

const RATE: u32 = 44_100;
const BLOCK: usize = 128;

fn source(name: &str, hz: f32) -> Source {
    let v = (0..20_000)
        .map(|i| (i as f32 * hz * std::f32::consts::TAU / RATE as f32).sin() * 0.3)
        .collect();
    Source::new(name, RATE, 1, v).unwrap()
}

/// A kit of 16 pads in every play mode, with a keygroup pad.
fn kit() -> Project {
    let mut p = Project::new();
    for i in 0..16usize {
        let s = source(&format!("s{i}"), 100.0 + 30.0 * i as f32);
        p.add_source(s.clone());
        let mut r = Region::whole(&s);
        r.mode = [
            PlayMode::OneShot,
            PlayMode::Gate,
            PlayMode::Loop,
            PlayMode::Toggle,
        ][i % 4];
        r.sustain_loop = Some(Loop {
            start: 2_000,
            end: 12_000,
            crossfade: 100,
            mode: LoopMode::Forward,
        });
        let mut pad = Pad::new("p", Layer::new(r));
        if i % 3 == 0 {
            pad.filter = Filter {
                kind: FilterKind::LowPass,
                poles: 2,
                cutoff_hz: 1_500.0 + 100.0 * i as f32,
                resonance: 0.2,
                env_octaves: 0.0,
            };
        }
        pad.keys = (i == 2).then_some(Keys {
            lo: 0,
            hi: 35,
            root: 24,
        });
        p.set_pad(i, Some(pad), None).unwrap();
    }
    p
}

/// A keygroup program over the whole keyboard.
fn keys() -> Project {
    let s = source("keys", 220.0);
    let mut p = Project::new();
    p.add_source(s.clone());
    let mut r = Region::whole(&s);
    r.mode = PlayMode::Gate;
    p.set_pad(0, Some(Pad::new("keys", Layer::new(r))), None)
        .unwrap();
    p.spread_to_keys(0, 57).unwrap();
    p
}

/// A tiny xorshift, so the session is the same every run.
struct Lcg(u32);

impl Lcg {
    fn next(&mut self, n: u32) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0 % n
    }
}

#[test]
fn process_never_allocates() {
    let (a, b) = (kit(), keys());
    let (mut node, control) = SamplerNode::new(RATE, 24, EngineView::build(&a, RATE));
    let (mut l, mut r) = (vec![0.0f32; BLOCK], vec![0.0f32; BLOCK]);
    let mut events: Vec<Timed> = Vec::with_capacity(64);
    let mut rng = Lcg(0x9e37_79b9);
    let base = DEFAULT_BASE_NOTE;
    let mut sounded = false;
    for block in 0..2_000u32 {
        // The control thread's part: views in now and then, old ones back
        // and dropped here.
        if block % 97 == 50 {
            let p = if (block / 97) % 2 == 0 { &b } else { &a };
            let _ = control.send_view(EngineView::build(p, RATE));
        }
        if block % 5 == 0 {
            while control.take_old().is_some() {}
        }
        // A busy block: notes on pads, below them and above them, note
        // offs, pressure, a held note again, settings.
        events.clear();
        for _ in 0..rng.next(6) {
            let at = rng.next(BLOCK as u32);
            let note = match rng.next(4) {
                0 => rng.next(36) as u8,
                1 => 60 + rng.next(30) as u8,
                _ => base + rng.next(16) as u8,
            };
            let ev = match rng.next(6) {
                0..=2 => NodeEvent::NoteOn {
                    note,
                    velocity: 1 + rng.next(127) as u8,
                },
                3 | 4 => NodeEvent::NoteOff { note },
                _ => NodeEvent::Pressure {
                    note,
                    value: rng.next(128) as u8,
                },
            };
            events.push(Timed::new(at, ev));
        }
        if block % 31 == 7 {
            let at = rng.next(BLOCK as u32);
            let value = -30.0 + rng.next(36) as f32;
            events.push(Timed::new(
                at,
                NodeEvent::Param {
                    id: ids::LEVEL,
                    value,
                },
            ));
        }
        if block % 211 == 100 {
            let value = f32::from(base) + [0.0, 12.0, -12.0][(block / 211) as usize % 3];
            events.push(Timed::new(
                3,
                NodeEvent::Param {
                    id: ids::BASE_NOTE,
                    value,
                },
            ));
        }
        // Pad settings, anywhere in their range or back at their default,
        // and the kit's Tune and Cutoff.
        for _ in 0..rng.next(3) {
            let id = pad_id(
                rng.next(LIVE_PADS as u32) as usize,
                rng.next(field::COUNT as u32) as u16,
            );
            let info = *node.params().iter().find(|p| p.id == id).unwrap();
            let value = match rng.next(4) {
                0 => info.default,
                _ => info.min + (info.max - info.min) * rng.next(1001) as f32 / 1000.0,
            };
            events.push(Timed::new(
                rng.next(BLOCK as u32),
                NodeEvent::Param { id, value },
            ));
        }
        if block % 23 == 11 {
            let (id, value) = match rng.next(2) {
                0 => (ids::TUNE, -12.0 + rng.next(25) as f32),
                _ => (ids::CUTOFF, -4.0 + rng.next(9) as f32),
            };
            events.push(Timed::new(
                rng.next(BLOCK as u32),
                NodeEvent::Param { id, value },
            ));
        }
        if block % 401 == 400 {
            events.push(Timed::new(17, NodeEvent::AllOff));
        }
        events.sort_by_key(|t| t.offset);
        let ctx = Ctx::new(RATE, u64::from(block) * BLOCK as u64);
        watch(true);
        node.process(&ctx, &events, &mut l, &mut r);
        if block % 509 == 508 {
            node.reset();
        }
        let _ = node.param(ids::METER_VOICES);
        watch(false);
        sounded |= l.iter().any(|x| x.abs() > 0.01);
    }
    while control.take_old().is_some() {}
    assert!(sounded, "the session never made a sound");
    assert!(node.engine().view_revision() > 0);
    assert_eq!(
        CALLS.load(Ordering::Relaxed),
        0,
        "process allocated or freed"
    );
}
