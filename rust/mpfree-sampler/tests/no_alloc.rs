//! The engine's real-time calls (render, apply, schedule) must not allocate
//! or free. A counting global allocator checks that across a busy session:
//! loops, layers, chokes, stealing, filters and sample-accurate events.
//!
//! This test file is its own program, so its global allocator (the only
//! unsafe code here, a pass-through to the system allocator) does not
//! affect the library, which forbids unsafe code.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use mpfree_sampler::dsp::{Ahdsr, Filter, FilterKind};
use mpfree_sampler::engine::{Engine, EngineView, Event};
use mpfree_sampler::model::*;

struct Counting;
static WATCH: AtomicBool = AtomicBool::new(false);
static CALLS: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        if WATCH.load(Ordering::Relaxed) { CALLS.fetch_add(1, Ordering::Relaxed); }
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        if WATCH.load(Ordering::Relaxed) { CALLS.fetch_add(1, Ordering::Relaxed); }
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        if WATCH.load(Ordering::Relaxed) { CALLS.fetch_add(1, Ordering::Relaxed); }
        unsafe { System.realloc(p, l, n) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

#[test]
fn render_and_events_never_allocate() {
    let rate = 44_100;
    let s = Source::new("noise", rate, 2, (0..40_000).map(|i| ((i * 7919) % 2000) as f32 / 1000.0 - 1.0).collect()).unwrap();
    let mut p = Project::new();
    p.add_source(s.clone());
    for i in 0..16usize {
        let mut r = Region::whole(&s);
        r.mode = [PlayMode::OneShot, PlayMode::Gate, PlayMode::Loop, PlayMode::Toggle][i % 4];
        r.sustain_loop = Some(Loop { start: 5000, end: 9000, crossfade: 200, mode: if i % 3 == 0 { LoopMode::PingPong } else { LoopMode::Forward } });
        r.reverse = i % 5 == 4;
        r.tune_cents = (i as i32 - 8) * 50;
        let mut pad = Pad::new("p", Layer::new(r.clone()));
        pad.layers.push(Layer::new(r));
        pad.select = [LayerSelect::All, LayerSelect::Cycle, LayerSelect::Random][i % 3];
        pad.polyphony = 2;
        pad.mute_group = (i % 4 == 0).then_some(1);
        pad.chokes = vec![(i + 1) % 16];
        pad.filter = Filter { kind: FilterKind::LowPass, poles: 4, cutoff_hz: 1000.0, resonance: 0.5, env_octaves: 3.0 };
        pad.amp = Ahdsr { attack_ms: 2.0, hold_ms: 1.0, decay_ms: 50.0, sustain: 0.6, release_ms: 30.0 };
        pad.filter_env = Ahdsr { attack_ms: 1.0, hold_ms: 0.0, decay_ms: 80.0, sustain: 0.1, release_ms: 30.0 };
        p.set_pad(i, Some(pad), None).unwrap();
    }
    let mut e = Engine::new(rate, 24);
    e.set_view(EngineView::build(&p, rate));
    let mut l = vec![0.0f32; 128];
    let mut r = vec![0.0f32; 128];
    e.render(&mut l, &mut r); // warm up outside the watch

    // The counter itself works: one Vec is one allocation and one free.
    WATCH.store(true, Ordering::SeqCst);
    drop(std::hint::black_box(vec![0u8; 16]));
    WATCH.store(false, Ordering::SeqCst);
    assert_eq!(CALLS.swap(0, Ordering::SeqCst), 2);

    WATCH.store(true, Ordering::SeqCst);
    for block in 0..2000u32 {
        let pad = (block * 7 % 16) as u8;
        e.apply(Event::NoteOn { pad, velocity: (block % 127 + 1) as u8 });
        e.schedule(block % 128, Event::NoteOn { pad: (pad + 3) % 16, velocity: 90 });
        e.schedule(block * 31 % 200, Event::NoteOff { pad: (pad + 5) % 16 });
        if block % 300 == 299 { e.apply(Event::AllOff); }
        e.render(&mut l, &mut r);
    }
    WATCH.store(false, Ordering::SeqCst);
    // Voices that end drop their handle to the audio; the project still holds
    // it, so that is a counter decrement, not a free.
    assert_eq!(CALLS.load(Ordering::SeqCst), 0, "allocations or frees while rendering");
    assert!(e.stats().hits > 2000);
    assert!(l.iter().chain(r.iter()).all(|x| x.is_finite()));
}
