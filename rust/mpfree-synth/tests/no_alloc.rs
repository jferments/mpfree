//! Poly's, FM's, Drums' and Pluck's `process` must not allocate or free. A counting global allocator
//! watches a busy session: chords past the voice limit, mono and legato,
//! glide, bend, pressure, automation of every parameter, AllOff, reset.
//!
//! This file is its own program, so its global allocator (the only unsafe
//! code here, a pass-through to the system allocator) does not affect the
//! library, which forbids unsafe code. Only the test's thread is watched.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};

use mpfree_node::{Ctx, Node, NodeEvent, Timed};
use std::sync::Arc;

use mpfree_synth::wavetable::table::{Table, FRAME};
use mpfree_synth::{Drums, Fm, Pluck, Poly, Wavetable, PARAMS};

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

/// A busy session for any instrument: random notes, pressure, bend,
/// automation of every parameter, AllOff, and a reset half way.
fn exercise(s: &mut dyn Node, params: &[mpfree_node::ParamInfo], rate: u32) -> usize {
    let mut l = vec![0.0f32; 128];
    let mut r = vec![0.0f32; 128];
    let mut events: Vec<Timed> = Vec::with_capacity(64);
    let mut seed = 99u64;
    let mut rnd = move || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (seed >> 33) as u32
    };
    let before = CALLS.load(Ordering::Relaxed);
    watch(true);
    for block in 0..4000u64 {
        events.clear();
        for _ in 0..rnd() % 6 {
            let off = rnd() % 128;
            let note = 24 + (rnd() % 80) as u8;
            let ev = match rnd() % 9 {
                0..=2 => NodeEvent::NoteOn { note, velocity: 1 + (rnd() % 127) as u8 },
                3..=4 => NodeEvent::NoteOff { note },
                5 => NodeEvent::Pressure { note, value: (rnd() % 128) as u8 },
                6 => NodeEvent::Bend { value: (rnd() % 200) as f32 / 100.0 - 1.0 },
                7 => {
                    let p = &params[rnd() as usize % params.len()];
                    NodeEvent::Param { id: p.id, value: p.min + (p.max - p.min) * (rnd() % 1000) as f32 / 1000.0 }
                }
                _ => NodeEvent::AllOff,
            };
            events.push(Timed::new(off, ev));
        }
        events.sort_by_key(|e| e.offset);
        let ctx = Ctx { rate, frame: block * 128, bpm: 120.0, beats: block as f64 * 128.0 * 2.0 / rate as f64, playing: true, sidechain: None };
        l.fill(0.0);
        r.fill(0.0);
        s.process(&ctx, &events, &mut l, &mut r);
        if block == 2000 { s.reset(); }
    }
    watch(false);
    assert!(l.iter().all(|x| x.is_finite()));
    CALLS.load(Ordering::Relaxed) - before
}

#[test]
fn fm_process_never_allocates() {
    let mut s = Fm::new(44_100);
    let params = s.params();
    assert_eq!(exercise(&mut s, params, 44_100), 0, "Fm::process allocated");
}

#[test]
fn wavetable_process_never_allocates() {
    let mut s = Wavetable::new(44_100);
    let params = s.params();
    assert_eq!(exercise(&mut s, params, 44_100), 0, "Wavetable::process allocated");
    // Swapping a user table in on the audio thread neither allocates nor
    // frees: the old one comes back to the caller.
    let saw: Vec<f32> = (0..FRAME).map(|i| 1.0 - 2.0 * i as f32 / FRAME as f32).collect();
    let a = Arc::new(Table::from_frames("a", &saw, FRAME).unwrap());
    let b = Arc::new(Table::from_frames("b", &saw, FRAME).unwrap());
    s.set_user_table(0, a.clone());
    let before = CALLS.load(Ordering::Relaxed);
    watch(true);
    s.set_param(mpfree_synth::wavetable::params::TABLE, mpfree_synth::wavetable::params::USER_TABLE);
    let old = s.set_user_table(0, b.clone());
    watch(false);
    assert_eq!(CALLS.load(Ordering::Relaxed) - before, 0, "set_user_table allocated or freed");
    drop(old);
}

#[test]
fn drums_process_never_allocates() {
    let mut s = Drums::new(48_000);
    let params = s.params();
    assert_eq!(exercise(&mut s, params, 48_000), 0, "Drums::process allocated");
}

#[test]
fn pluck_process_never_allocates() {
    let mut s = Pluck::new(48_000);
    let params = s.params();
    assert_eq!(exercise(&mut s, params, 48_000), 0, "Pluck::process allocated");
    // And as a mallet.
    let mut s = Pluck::new(48_000);
    s.set_param(mpfree_synth::pluck_params::MODEL, 1.0);
    assert_eq!(exercise(&mut s, params, 48_000), 0, "Pluck::process (mallet) allocated");
}

#[test]
fn process_never_allocates() {
    let rate = 48_000;
    let mut s = Poly::new(rate);
    let mut l = vec![0.0f32; 128];
    let mut r = vec![0.0f32; 128];
    let mut events: Vec<Timed> = Vec::with_capacity(64);
    let mut seed = 99u64;
    let mut rnd = move || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (seed >> 33) as u32
    };

    watch(true);
    for block in 0..4000u64 {
        events.clear();
        for _ in 0..rnd() % 6 {
            let off = rnd() % 128;
            let note = 24 + (rnd() % 80) as u8;
            let ev = match rnd() % 9 {
                0..=2 => NodeEvent::NoteOn { note, velocity: 1 + (rnd() % 127) as u8 },
                3..=4 => NodeEvent::NoteOff { note },
                5 => NodeEvent::Pressure { note, value: (rnd() % 128) as u8 },
                6 => NodeEvent::Bend { value: (rnd() % 200) as f32 / 100.0 - 1.0 },
                7 => {
                    let p = &PARAMS[rnd() as usize % PARAMS.len()];
                    NodeEvent::Param { id: p.id, value: p.min + (p.max - p.min) * (rnd() % 1000) as f32 / 1000.0 }
                }
                _ => NodeEvent::AllOff,
            };
            events.push(Timed::new(off, ev));
        }
        events.sort_by_key(|e| e.offset);
        let ctx = Ctx { rate, frame: block * 128, bpm: 120.0, beats: block as f64 * 128.0 * 2.0 / rate as f64, playing: true, sidechain: None };
        l.fill(0.0);
        r.fill(0.0);
        s.process(&ctx, &events, &mut l, &mut r);
        if block == 2000 { s.reset(); }
    }
    watch(false);
    assert_eq!(CALLS.load(Ordering::Relaxed), 0, "process allocated");
    assert!(l.iter().all(|x| x.is_finite()));
}
