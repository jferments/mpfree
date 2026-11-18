//! A sound's `process` may not allocate or free: notes, every setting of
//! the engine, its effects and its macros automated, macros moving their
//! settings, AllOff, a reset and load shedding, all on the audio thread.
//!
//! This file is its own program, so its global allocator (the only unsafe
//! code here, a pass-through to the system allocator) does not affect the
//! library. Only the test's thread is watched.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};

use mpfree_instruments::sound::{FX_BASE, FX_SPAN};
use mpfree_instruments::{Design, Macro, MacroMap, Sound};
use mpfree_mod::MapCurve;
use mpfree_node::{Ctl, Ctx, Node, NodeEvent, Timed};
use mpfree_synth::keys_params as kp;
use mpfree_synth::{Keys, Poly};

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

const RATE: u32 = 48_000;

/// Three effects (the most a sound holds) and two macros, one moving an
/// engine setting and two effects' settings.
fn design() -> Design {
    let fx = |slot: u16, id: u16| FX_BASE + slot * FX_SPAN + id;
    Design {
        fx: vec![
            "mpfree.fx.ensemble".into(),
            "mpfree.fx.delay".into(),
            "mpfree.fx.plate".into(),
        ],
        macros: vec![
            Macro {
                name: "Space".into(),
                maps: vec![
                    MacroMap {
                        target: fx(0, 7),
                        from: 0.0,
                        to: 80.0,
                        curve: MapCurve::S,
                    },
                    MacroMap {
                        target: fx(1, 6),
                        from: 0.0,
                        to: 40.0,
                        curve: MapCurve::Exp,
                    },
                ],
            },
            Macro {
                name: "Tone".into(),
                maps: vec![MacroMap {
                    target: kp::TONE,
                    from: -40.0,
                    to: 40.0,
                    curve: MapCurve::Linear,
                }],
            },
        ],
    }
}

/// A busy session: random notes, pressure, bend, controllers, every
/// setting automated (the macros among them), AllOff, a reset half way
/// and load shedding between blocks. Returns the allocations counted.
fn exercise(s: &mut dyn Node) -> usize {
    let params = s.params();
    let mut l = vec![0.0f32; 128];
    let mut r = vec![0.0f32; 128];
    let mut events: Vec<Timed> = Vec::with_capacity(64);
    let mut seed = 7u64;
    let mut rnd = move || {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (seed >> 33) as u32
    };
    let before = CALLS.load(Ordering::Relaxed);
    watch(true);
    for block in 0..3000u64 {
        events.clear();
        for _ in 0..rnd() % 6 {
            let off = rnd() % 128;
            let note = 24 + (rnd() % 80) as u8;
            let ev = match rnd() % 10 {
                0..=2 => NodeEvent::NoteOn {
                    note,
                    velocity: 1 + (rnd() % 127) as u8,
                },
                3..=4 => NodeEvent::NoteOff { note },
                5 => NodeEvent::Pressure {
                    note,
                    value: (rnd() % 128) as u8,
                },
                6 => NodeEvent::Bend {
                    value: (rnd() % 200) as f32 / 100.0 - 1.0,
                },
                7 => {
                    let p = &params[rnd() as usize % params.len()];
                    NodeEvent::Param {
                        id: p.id,
                        value: p.min + (p.max - p.min) * (rnd() % 1000) as f32 / 1000.0,
                    }
                }
                8 => NodeEvent::Control {
                    ctl: Ctl::ALL[rnd() as usize % Ctl::ALL.len()],
                    value: (rnd() % 128) as f32 / 127.0,
                },
                _ => NodeEvent::AllOff,
            };
            events.push(Timed::new(off, ev));
        }
        events.sort_by_key(|e| e.offset);
        let ctx = Ctx {
            rate: RATE,
            frame: block * 128,
            bpm: 120.0,
            beats: block as f64 * 128.0 * 2.0 / RATE as f64,
            playing: true,
            sidechain: None,
        };
        l.fill(0.0);
        r.fill(0.0);
        s.process(&ctx, &events, &mut l, &mut r);
        if block == 1500 {
            s.reset();
        }
        if block % 29 == 0 && s.cull_candidate().is_some() {
            s.cull();
        }
        let _ = s.sounding_voices();
    }
    watch(false);
    assert!(l.iter().chain(&r).all(|x| x.is_finite()));
    CALLS.load(Ordering::Relaxed) - before
}

#[test]
fn a_sounds_process_never_allocates() {
    for (name, engine) in [
        ("Keys", Box::new(Keys::new(RATE)) as Box<dyn Node>),
        ("Poly", Box::new(Poly::new(RATE))),
    ] {
        // Poly has no Tone setting: its macro moves the effects only.
        let mut d = design();
        if name == "Poly" {
            d.macros.truncate(1);
        }
        let mut s = Sound::new(engine, &d).unwrap();
        s.prepare(RATE, 128);
        assert_eq!(exercise(&mut s), 0, "{name}: Sound::process allocated");
    }
}

#[test]
fn a_loaded_sounds_process_never_allocates() {
    let saved = {
        let s = Sound::new(Box::new(Keys::new(RATE)), &design()).unwrap();
        s.to_text("Space Keys", &[], &[])
    };
    let (mut s, _) = Sound::from_preset(Box::new(Keys::new(RATE)), &saved).unwrap();
    s.prepare(RATE, 128);
    assert_eq!(exercise(&mut s), 0, "a loaded sound allocated");
}
