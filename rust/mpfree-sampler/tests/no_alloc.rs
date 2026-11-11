//! The engine's real-time calls (render, apply, schedule) and the rolling
//! recording buffer's writer must not allocate
//! or free. A counting global allocator checks that across a busy session:
//! loops, layers, chokes, stealing, filters, sample-accurate events,
//! granular pads, pad pressure, and swapping prebuilt views (the pad player
//! switches kits that way).
//!
//! This test file is its own program, so its global allocator (the only
//! unsafe code here, a pass-through to the system allocator) does not
//! affect the library, which forbids unsafe code.
//!
//! Only the test's own thread is watched: the test harness's main thread
//! can allocate at any moment (it did on a busy CI runner), and that is
//! not the engine's doing.
//!
//! The master granular effect runs throughout: views move its mix up and
//! back down, and freeze turns on and off on exact frames.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};

use mpfree_sampler::dsp::{Ahdsr, Filter, FilterKind};
use mpfree_sampler::engine::{Engine, EngineView, Event};
use mpfree_sampler::model::*;
use mpfree_sampler::record::rolling;

struct Counting;
thread_local! {
    // A const-initialised Cell needs no allocation or destructor, so it is
    // safe to read from inside the allocator.
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
        let targets = [PressureTarget::Off, PressureTarget::Level, PressureTarget::Pitch, PressureTarget::Filter];
        pad.pressure = PressureRoute { target: targets[i % 4], depth: if i % 2 == 0 { 0.8 } else { -0.6 } };
        p.set_pad(i, Some(pad), None).unwrap();
    }
    // Four granular pads: busy clouds, one reversed, one synced, so the grain
    // budget fills and voices are stolen while grains play.
    for i in 16..20usize {
        let mut r = Region::whole(&s);
        r.mode = PlayMode::Granular;
        r.reverse = i == 17;
        r.tune_cents = (i as i32 - 18) * 700;
        r.grain = GrainParams { spray: 0.6, size_ms: 300.0, density: 1000.0, pitch_spray: 5.0, spread: 1.0, scan: 1.0, ..GrainParams::default() };
        if i == 19 {
            r.grain.window = GrainWindow::Exponential { decay: 6.0 };
            r.grain.sync = Some(TempoSync { frames_per_beat: 22_050.0, grains_per_beat: Some(8.0), size_beats: Some(0.25) });
        }
        let mut pad = Pad::new("g", Layer::new(r));
        pad.polyphony = 2;
        pad.filter = Filter { kind: FilterKind::LowPass, poles: 2, cutoff_hz: 3000.0, resonance: 0.2, env_octaves: 1.0 };
        let targets = [PressureTarget::GrainDensity, PressureTarget::GrainSize, PressureTarget::GrainPosition, PressureTarget::GrainSpray];
        pad.pressure = PressureRoute { target: targets[i - 16], depth: 1.0 };
        p.set_pad(i, Some(pad), None).unwrap();
    }
    let mut e = Engine::new(rate, 24);
    e.set_view(EngineView::build(&p, rate));
    // A second view, swapped in and out while voices play: only the views
    // move, nothing is built or dropped.
    let mut q = Project::new();
    q.add_source(s.clone());
    for i in 0..16usize {
        let mut r = Region::whole(&s);
        r.start = (i as u64) * 1000;
        q.set_pad(i, Some(Pad::new("k", Layer::new(r))), None).unwrap();
    }
    let mut spare = Some(EngineView::build(&q, rate));
    let mut l = vec![0.0f32; 128];
    let mut r = vec![0.0f32; 128];
    e.render(&mut l, &mut r); // warm up outside the watch
    // The rolling "skip back" buffer the audio thread fills every period.
    let (mut rec, reader) = rolling(2, 44_100 * 2);
    // The master effect is made and put in before the watch; inside it, a
    // second view brings its mix up and back down.
    assert!(e.set_master_fx(Engine::make_master_fx(rate)).is_none());
    p.set_master(MasterFx { mix: 0.7, grain: GrainParams { density: 200.0, size_ms: 150.0, spray: 0.3, pitch_spray: 2.0, ..MasterFx::default().grain } }, None).unwrap();
    let mut wet = Some(EngineView::build(&p, rate));
    p.set_master(MasterFx::default(), None).unwrap();
    let dry = EngineView::build(&p, rate);

    // The counter itself works: one Vec is one allocation and one free.
    watch(true);
    drop(std::hint::black_box(vec![0u8; 16]));
    watch(false);
    assert_eq!(CALLS.swap(0, Ordering::SeqCst), 2);

    watch(true);
    for block in 0..2000u32 {
        let pad = (block * 7 % 16) as u8;
        if block % 200 == 10 { wet = Some(e.set_view(wet.take().unwrap())); }
        if block % 64 == 0 { e.schedule(block % 128, Event::Freeze { on: block % 128 == 0 }); }
        e.apply(Event::NoteOn { pad, velocity: (block % 127 + 1) as u8 });
        e.schedule(block % 128, Event::NoteOn { pad: (pad + 3) % 16, velocity: 90 });
        e.schedule(block * 31 % 200, Event::NoteOff { pad: (pad + 5) % 16 });
        let g = 16 + (block % 4) as u8;
        if block % 3 == 0 { e.schedule(block % 100, Event::NoteOn { pad: g, velocity: 100 }); }
        if block % 5 == 0 { e.schedule(block % 90, Event::NoteOff { pad: 16 + ((block + 1) % 4) as u8 }); }
        e.schedule(block * 17 % 128, Event::Pressure { pad: (block % 20) as u8, value: (block * 37 % 128) as u8 });
        if block % 300 == 299 { e.apply(Event::AllOff); }
        if block % 50 == 25 { spare = Some(e.set_view(spare.take().unwrap())); }
        e.render(&mut l, &mut r);
        rec.push_planar(&l, &r);
    }
    // Then only the granular pads, held long enough to fill the grain budget.
    for pad in 16..20u8 {
        e.apply(Event::NoteOn { pad, velocity: 100 });
        e.apply(Event::NoteOn { pad, velocity: 100 });
    }
    for block in 0..200u32 {
        for pad in 16..20u8 { e.schedule(block % 64, Event::Pressure { pad, value: (block * 5 % 128) as u8 }); }
        e.render(&mut l, &mut r);
        rec.push_planar(&l, &r);
    }
    let spawned_before_dry = e.master_fx().unwrap().stats().spawned;
    let _kept = e.set_view(dry); // dry again, so the master hands its grains back
    e.apply(Event::AllOff);
    for _ in 0..100 { e.render(&mut l, &mut r); }
    watch(false);
    // Voices that end drop their handle to the audio; the project still holds
    // it, so that is a counter decrement, not a free.
    assert_eq!(CALLS.load(Ordering::SeqCst), 0, "allocations or frees while rendering");
    assert!(e.stats().hits > 2000);
    assert!(e.stats().skipped_grains > 0, "the grain budget never filled; the test is not busy enough");
    assert_eq!(reader.written(), 2200 * 128);
    assert_eq!(e.live_grains(), 0, "every grain handed back after AllOff");
    assert!(l.iter().chain(r.iter()).all(|x| x.is_finite()));
    assert!(spawned_before_dry > 100, "the master effect made grains: {spawned_before_dry}");
}
