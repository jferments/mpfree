//! Let-gos always land (sampler plan, phase 1 step 2). With the event queue
//! full, a let-go of a pad, a key, the take or all of them still lands on
//! its frame, so no note hangs; pressure and live values merge to make
//! room; a hit the full queue refuses is counted; and none of it
//! allocates.
//!
//! This test file is its own program, so its counting global allocator (a
//! pass-through to the system allocator, the only unsafe code here) does
//! not affect the library. Only the watching test's own thread counts.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};

use mpfree_sampler::dsp::Ahdsr;
use mpfree_sampler::engine::{Engine, EngineView, Event, ProgramField, MAX_EVENTS, TAKE_PAD};
use mpfree_sampler::model::*;
use mpfree_sampler::warp::Warps;

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
static ALLOC: Counting = Counting;

const RATE: u32 = 44_100;
const BLOCK: usize = 128;
/// Kit 0 and 1 hold notes; kit 2 stays empty, for values that change
/// nothing heard.
const KITS: usize = 3;
/// Gate pads each holding kit has.
const HELD_PADS: u8 = 16;
/// Kit 0's keygroup pad, over keys 36 to 96.
const KEYS_PAD: usize = 20;
/// The keys held on it.
const KEYS: std::ops::Range<u8> = 60..68;

fn noise(frames: usize, seed: u32) -> Source {
    let mut x = seed;
    let data = (0..frames * 2).map(|_| {
        x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (x >> 8) as f32 / (1u32 << 24) as f32 - 0.5
    });
    Source::new("noise", RATE, 2, data.collect()).unwrap()
}

/// A Gate sound looping for as long as it is held, with a 50 ms release
/// (so a sound let go is still releasing at the end of the block).
fn held(s: &Source) -> Pad {
    let mut r = Region::whole(s);
    r.mode = PlayMode::Gate;
    r.sustain_loop = Some(Loop {
        start: 1_000,
        end: 9_000,
        crossfade: 200,
        mode: LoopMode::Forward,
    });
    let mut pad = Pad::new("held", Layer::new(r));
    pad.amp = Ahdsr {
        attack_ms: 0.0,
        hold_ms: 0.0,
        decay_ms: 0.0,
        sustain: 1.0,
        release_ms: 50.0,
    };
    pad
}

/// [`HELD_PADS`] held pads; with `keys`, a keygroup pad and a held take too.
fn kit(seed: u32, keys: bool) -> EngineView {
    let s = noise(20_000, seed);
    let mut p = Project::new();
    p.add_source(s.clone());
    for pad in 0..usize::from(HELD_PADS) {
        p.set_pad(pad, Some(held(&s)), None).unwrap();
    }
    if !keys {
        return EngineView::build(&p, RATE);
    }
    let mut k = held(&s);
    k.keys = Some(Keys {
        lo: 36,
        hi: 96,
        root: 60,
    });
    k.polyphony = KEYGROUP_POLYPHONY;
    p.set_pad(KEYS_PAD, Some(k), None).unwrap();
    p.add_take(held(&s)).unwrap();
    EngineView::build_with_take(&p, RATE, &mut Warps::new(120.0), Some(0))
}

fn engine() -> Engine {
    let mut e = Engine::with_kits(RATE, 64, KITS);
    e.set_kit_view(0, kit(1, true));
    e.set_kit_view(1, kit(2, false));
    e
}

fn render(e: &mut Engine, frames: usize, l: &mut [f32], r: &mut [f32]) {
    l[..frames].fill(0.0);
    r[..frames].fill(0.0);
    e.render(&mut l[..frames], &mut r[..frames]);
}

/// Holds every held pad of kits 0 and 1, kit 0's keys and its take, in one
/// block. Returns how many voices that is.
fn hold_everything(e: &mut Engine, l: &mut [f32], r: &mut [f32]) -> usize {
    for kit in 0..2 {
        for pad in 0..HELD_PADS {
            let o = u32::from(pad) * 3 + kit as u32;
            assert!(e.schedule_for(kit, o, Event::NoteOn { pad, velocity: 100 }));
        }
    }
    for note in KEYS {
        assert!(e.schedule_for(0, 60, Event::Key { note, velocity: 90 }));
    }
    assert!(e.schedule_for(0, 70, Event::TakeOn { velocity: 127 }));
    render(e, BLOCK, l, r);
    let n = 2 * usize::from(HELD_PADS) + KEYS.len() + 1;
    assert_eq!(e.held_voices(), n);
    n
}

/// Fills the rest of the queue (`queued` events already in it) with
/// pressure on kit 2's empty pads at frame `at`: none merges with another,
/// so nothing gives way.
fn fill(e: &mut Engine, queued: usize, at: u32) {
    for i in 0..MAX_EVENTS - queued {
        let pad = (i % 128) as u8;
        let ev = if i < 128 {
            Event::Pressure { pad, value: 64 }
        } else {
            Event::PadLive {
                pad,
                field: mpfree_sampler::engine::PadField::Level,
                value: f32::NAN,
            }
        };
        assert!(e.schedule_for(2, at, ev), "queue full after {i}");
    }
}

#[test]
fn every_let_go_lands_in_its_period_with_the_queue_full_and_none_allocates() {
    let mut e = engine();
    let (mut l, mut r) = (vec![0.0; BLOCK], vec![0.0; BLOCK]);
    let n = hold_everything(&mut e, &mut l, &mut r);
    assert!(n > 0);

    WATCH.with(|w| w.set(true));
    fill(&mut e, 0, 0);
    // Full: a value for a pad and setting with none waiting is refused.
    assert!(!e.schedule_for(1, 0, Event::Pressure { pad: 127, value: 1 }));
    let refused = e.stats().dropped_events;
    assert_eq!(refused, 1);
    // Let go of everything at once, across the block: pads of two kits,
    // keys and the take.
    for kit in 0..2 {
        for pad in 0..HELD_PADS {
            let o = 5 + u32::from(pad) * 7;
            assert!(e.schedule_for(kit, o, Event::NoteOff { pad }));
        }
    }
    for note in KEYS {
        assert!(e.schedule_for(0, 90, Event::KeyOff { note }));
    }
    assert!(e.schedule_for(0, 120, Event::TakeOff));
    assert_eq!(
        e.late_waiting(),
        n,
        "every let-go waits aside, in its own place"
    );
    render(&mut e, BLOCK, &mut l, &mut r);
    WATCH.with(|w| w.set(false));

    assert_eq!(
        e.held_voices(),
        0,
        "every voice is releasing within the period"
    );
    assert_eq!(e.late_waiting(), 0);
    let s = e.stats();
    assert_eq!(s.late_let_gos, n as u64);
    assert_eq!(s.dropped_events, refused, "no let-go was refused");
    assert_eq!(s.refused_hits, 0);
    assert_eq!(e.pad_voices(TAKE_PAD), 1, "the take is releasing, not gone");
    assert_eq!(CALLS.load(Ordering::Relaxed), 0, "the full queue allocated");
}

/// Kit 0's pads 0 and 1 held, then let go at frames 10 and `BLOCK` + 40;
/// with `full`, the queue is full of values on kit 2 (all at frame 0) when
/// they come. Returns three blocks of output, the last split at frame 40.
fn let_go_twice(full: bool) -> (Vec<f32>, Engine) {
    let mut e = engine();
    let (mut l, mut r) = (vec![0.0; BLOCK], vec![0.0; BLOCK]);
    for pad in 0..2 {
        assert!(e.schedule_for(0, 0, Event::NoteOn { pad, velocity: 100 }));
    }
    render(&mut e, BLOCK, &mut l, &mut r);
    let mut out = l.clone();
    if full {
        fill(&mut e, 0, 0);
    }
    assert!(e.schedule_for(0, BLOCK as u32 + 40, Event::NoteOff { pad: 0 }));
    assert!(e.schedule_for(0, 10, Event::NoteOff { pad: 1 }));
    assert_eq!(e.late_waiting(), if full { 2 } else { 0 });
    render(&mut e, BLOCK, &mut l, &mut r);
    out.extend_from_slice(&l);
    assert_eq!(e.held_voices(), 1, "pad 0's let-go waits for its frame");
    render(&mut e, 40, &mut l, &mut r);
    out.extend_from_slice(&l[..40]);
    assert_eq!(e.held_voices(), 1, "not a frame early");
    render(&mut e, BLOCK - 40, &mut l, &mut r);
    out.extend_from_slice(&l[..BLOCK - 40]);
    assert_eq!(e.held_voices(), 0);
    (out, e)
}

#[test]
fn a_let_go_waiting_aside_lands_on_the_frame_it_would_from_the_queue() {
    let (queued, _) = let_go_twice(false);
    let (aside, e) = let_go_twice(true);
    assert_eq!(e.stats().late_let_gos, 2);
    assert_eq!(queued, aside, "to the sample");
    assert!(queued.iter().any(|&x| x != 0.0));
}

#[test]
fn a_full_queue_merges_values_into_the_newest_for_the_same_pad_and_setting() {
    let mut e = engine();
    let (mut l, mut r) = (vec![0.0; BLOCK], vec![0.0; BLOCK]);
    assert!(e.schedule_for(
        0,
        0,
        Event::NoteOn {
            pad: 0,
            velocity: 100
        }
    ));
    render(&mut e, BLOCK, &mut l, &mut r);

    // Pressure on the held pad, one value a frame: the queue fills.
    for i in 0..MAX_EVENTS as u32 {
        let value = (i % 128) as u8;
        assert!(e.schedule_for(0, i, Event::Pressure { pad: 0, value }));
    }
    // A let-go takes the place of the oldest value, which has newer ones
    // behind it, so it lands from the queue, on its frame.
    assert!(e.schedule_for(0, 300, Event::NoteOff { pad: 0 }));
    assert_eq!(e.late_waiting(), 0);
    assert_eq!(e.stats().merged_values, 1);
    // A newer value merges with the newest one waiting.
    assert!(e.schedule_for(0, 299, Event::Pressure { pad: 0, value: 9 }));
    assert_eq!(e.stats().merged_values, 2);
    assert_eq!(e.stats().dropped_events, 0);
    for _ in 0..3 {
        render(&mut e, BLOCK, &mut l, &mut r);
    }
    assert_eq!(e.held_voices(), 0);

    // A knob: a newer turn takes the older one's place; one that comes
    // late with an older frame gives way to the newer one waiting.
    let mut e = engine();
    fill(&mut e, 1, 0);
    let level = ProgramField::Level;
    assert!(e.schedule_for(
        0,
        5,
        Event::ProgramLive {
            field: level,
            value: 0.1
        }
    ));
    assert!(e.schedule_for(
        0,
        6,
        Event::ProgramLive {
            field: level,
            value: 0.2
        }
    ));
    assert!(e.schedule_for(
        0,
        2,
        Event::ProgramLive {
            field: level,
            value: 0.3
        }
    ));
    assert_eq!(e.stats().merged_values, 2);
    render(&mut e, BLOCK, &mut l, &mut r);
    assert_eq!(e.kit_program_live_value(0, level), 0.2);
}

#[test]
fn a_refused_hit_is_counted_and_all_notes_off_lands_with_the_queue_full() {
    let mut e = engine();
    let (mut l, mut r) = (vec![0.0; BLOCK], vec![0.0; BLOCK]);
    hold_everything(&mut e, &mut l, &mut r);
    fill(&mut e, 0, 0);
    let before = e.stats();
    assert!(!e.schedule_for(
        1,
        3,
        Event::NoteOn {
            pad: 0,
            velocity: 100
        }
    ));
    assert!(!e.schedule_for(
        0,
        3,
        Event::Key {
            note: 70,
            velocity: 100
        }
    ));
    let s = e.stats();
    assert_eq!(s.refused_hits - before.refused_hits, 2);
    assert_eq!(s.dropped_events - before.dropped_events, 2);

    assert!(e.schedule(20, Event::AllOff));
    assert_eq!(e.late_waiting(), 1);
    render(&mut e, 20, &mut l, &mut r);
    assert!(e.held_voices() > 0, "all notes off waits for its frame");
    render(&mut e, BLOCK - 20, &mut l, &mut r);
    assert_eq!(e.held_voices(), 0, "every kit, every key, the take");

    // The hard stop forgets what waits aside.
    let mut e = engine();
    hold_everything(&mut e, &mut l, &mut r);
    fill(&mut e, 0, 0);
    assert!(e.schedule_for(1, 500, Event::NoteOff { pad: 3 }));
    assert_eq!(e.late_waiting(), 1);
    e.stop_all();
    assert_eq!(e.late_waiting(), 0);
    assert_eq!(e.active_voices(), 0);
}

#[test]
fn two_let_gos_of_one_pad_waiting_aside_land_together_on_the_later_frame() {
    let mut e = engine();
    let (mut l, mut r) = (vec![0.0; BLOCK], vec![0.0; BLOCK]);
    assert!(e.schedule_for(
        0,
        0,
        Event::NoteOn {
            pad: 0,
            velocity: 100
        }
    ));
    render(&mut e, BLOCK, &mut l, &mut r);
    fill(&mut e, 0, 0);
    assert!(e.schedule_for(0, BLOCK as u32 + 10, Event::NoteOff { pad: 0 }));
    render(&mut e, BLOCK, &mut l, &mut r);
    // The queue has room again: the pad is hit again after its let-go…
    assert!(e.schedule_for(
        0,
        20,
        Event::NoteOn {
            pad: 0,
            velocity: 100
        }
    ));
    fill(&mut e, 1, 0);
    // …and let go again while the first let-go still waits: the two land
    // together on frame 30, so the second hit doesn't hang.
    assert!(e.schedule_for(0, 30, Event::NoteOff { pad: 0 }));
    assert_eq!(e.late_waiting(), 1);
    render(&mut e, BLOCK, &mut l, &mut r);
    assert_eq!(e.held_voices(), 0);
    assert_eq!(e.pad_voices(0), 2, "both sounds are releasing");
}

#[test]
fn a_hit_on_the_frame_a_let_go_waiting_aside_lands_on_comes_after_it() {
    let mut e = engine();
    let (mut l, mut r) = (vec![0.0; BLOCK], vec![0.0; BLOCK]);
    assert!(e.schedule_for(
        0,
        0,
        Event::NoteOn {
            pad: 0,
            velocity: 100
        }
    ));
    render(&mut e, BLOCK, &mut l, &mut r);
    fill(&mut e, 0, 0);
    assert!(e.schedule_for(0, BLOCK as u32 + 50, Event::NoteOff { pad: 0 }));
    render(&mut e, BLOCK, &mut l, &mut r);
    // Let go on frame 50, then hit again on frame 50: the hit is the newer.
    assert!(e.schedule_for(
        0,
        50,
        Event::NoteOn {
            pad: 0,
            velocity: 100
        }
    ));
    render(&mut e, BLOCK, &mut l, &mut r);
    assert_eq!(e.held_voices(), 1, "the new hit plays on");
    assert_eq!(e.pad_voices(0), 2, "the old one is releasing");
}
