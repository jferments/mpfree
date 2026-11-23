//! Kits in a tuning never allocate on the audio thread: setting a kit's
//! tuning between renders, keygroup notes (poly, mono with legato and
//! glide, grains) started and moved in it, the wheel through it and 16
//! Levels of Tune in it.
//!
//! This test file is its own program, so its global allocator (the only
//! unsafe code here, a pass-through to the system allocator) does not
//! affect the library. Only the test's own thread is watched.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};

use mpfree_sampler::engine::{Engine, EngineView, Event};
use mpfree_sampler::model::*;
use mpfree_tuning::{Builtin, Tuning};

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

fn tone(seconds: f32) -> Source {
    let n = (seconds * RATE as f32) as usize;
    let x = (0..n)
        .map(|i| 0.4 * (i as f32 * 220.0 * std::f32::consts::TAU / RATE as f32).sin())
        .collect();
    Source::new("tone", RATE, 1, x).unwrap()
}

/// Pad 0: a poly keygroup; pad 1: a mono one with legato and glide; pad
/// 2: a grain keygroup gliding between keys. All over the whole keyboard.
fn kit() -> Project {
    let s = tone(1.5);
    let mut p = Project::new();
    p.add_source(s.clone());
    let mut gate = Region::whole(&s);
    gate.mode = PlayMode::Gate;
    let mut grains = Region::whole(&s);
    grains.mode = PlayMode::Granular;
    grains.grain_glide_ms = 30.0;
    let mut mono = Pad::new("mono", Layer::new(gate.clone()));
    mono.mono = Mono {
        on: true,
        legato: true,
        glide_ms: 40.0,
        ..Mono::default()
    };
    let mut grain = Pad::new("grain", Layer::new(grains));
    grain.mono = Mono {
        on: true,
        ..Mono::default()
    };
    p.set_pad(0, Some(Pad::new("poly", Layer::new(gate))), None)
        .unwrap();
    p.set_pad(1, Some(mono), None).unwrap();
    p.set_pad(2, Some(grain), None).unwrap();
    for (pad, root) in [(0, 57), (1, 45), (2, 69)] {
        let mut pad_ = p.pad(pad).unwrap().clone();
        pad_.keys = Some(Keys::all(root));
        p.set_pad(pad, Some(pad_), None).unwrap();
    }
    p
}

#[test]
fn tunings_keys_bends_and_levels_never_allocate() {
    let p = kit();
    let mut levels = p.clone();
    levels
        .set_pads_play(PadsPlay::Levels {
            pad: 0,
            by: Levels::Tune,
        })
        .unwrap();
    let tunings = [
        Builtin::Just.tuning(2, 432.0),
        Tuning::standard(),
        Builtin::Meantone.tuning(9, 440.0),
        Tuning::equal(24, 440.0),
        Builtin::Pythagorean.tuning(0, 415.0),
    ];
    let mut e = Engine::with_kits(RATE, 24, 2);
    e.set_kit_view(0, EngineView::build(&p, RATE));
    e.set_kit_view(1, EngineView::build(&levels, RATE));
    let (mut l, mut r) = (vec![0.0f32; 128], vec![0.0f32; 128]);

    watch(true);
    for block in 0..1200u32 {
        let b = block as usize;
        if block % 40 == 0 {
            e.set_tuning(0, &tunings[(b / 40) % tunings.len()]);
            e.set_tuning(1, &tunings[(b / 40 + 2) % tunings.len()]);
        }
        let note = 40 + ((block * 7) % 48) as u8;
        e.schedule_for(
            0,
            block % 128,
            Event::Key {
                note,
                velocity: 100,
            },
        );
        if block >= 3 {
            let old = 40 + (((block - 3) * 7) % 48) as u8;
            e.schedule_for(0, (block * 3) % 128, Event::KeyOff { note: old });
        }
        if block % 9 == 0 {
            let cents = ((block % 5) as f32 - 2.0) * 100.0;
            e.schedule_for(0, 17, Event::Bend { cents });
        }
        let pad = (block % 16) as u8;
        e.schedule_for(1, 5, Event::NoteOn { pad, velocity: 90 });
        e.schedule_for(1, 90, Event::NoteOff { pad });
        e.render(&mut l, &mut r);
    }
    e.apply_for(0, Event::AllOff);
    for _ in 0..50 {
        e.render(&mut l, &mut r);
    }
    watch(false);
    assert_eq!(
        CALLS.load(Ordering::SeqCst),
        0,
        "allocations or frees while rendering in a tuning"
    );
    assert!(e.stats().hits > 2000, "{}", e.stats().hits);
    assert!(l.iter().chain(r.iter()).all(|x| x.is_finite()));
}
