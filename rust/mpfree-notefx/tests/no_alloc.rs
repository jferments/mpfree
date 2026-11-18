//! A chain runs in the sound loop: once made, running notes through it,
//! changing its effects, a mute's cut and a panic must not allocate or
//! free.
//!
//! This test file is its own program, so its global allocator (a
//! pass-through to the system allocator) does not affect the library.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};

use mpfree_notefx::{
    Chain, Chord, Echo, Ev, Fx, LatchMode, Notes, Period, Randomize, Scale, Shape, Slot, Snap,
    Strum, StrumOrder, Time, Velocity, Voicing,
};

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

#[test]
fn a_chain_never_allocates_once_made() {
    let fx = [
        Fx::Scale {
            scale: Scale::new(9, mpfree_scale_minor()),
            snap: Snap::Nearest,
        },
        Fx::Chord(Chord {
            shape: Shape::Minor7,
            voicing: Voicing::Drop2,
            strum_ms: 8,
            ..Chord::default()
        }),
        Fx::Transpose { semitones: -12 },
        Fx::Velocity(Velocity {
            curve: 30,
            ..Velocity::default()
        }),
        Fx::Echo(Echo::default()),
        Fx::Humanize {
            time_ms: 8,
            velocity: 10,
        },
        Fx::Chance { percent: 80 },
        Fx::Length {
            time: Time::Ticks(160),
        },
    ];
    let mut chain = Box::new(Chain::new());
    for (i, f) in fx.iter().enumerate() {
        chain.set(i, Some(Slot { fx: *f, on: true }));
    }
    let mut out = Box::new(Notes::new());
    let mut input = [Ev::off(0, 0); 8];
    let block = 256u32;
    CALLS.store(0, Ordering::Relaxed);
    watch(true);
    let mut start = 0u64;
    for k in 0..4000u64 {
        let p = Period {
            start,
            frames: block,
            rate: 48_000,
            bpm: 128.0,
        };
        let note = 40 + (k % 24) as u8;
        input[0] = Ev::on(start + 10, note, 100);
        input[1] = Ev::off(start + 200, note.wrapping_sub(1).max(40));
        out.clear();
        chain.process(&p, &input[..2], &mut out);
        if k % 500 == 250 {
            // A setting changed, an effect switched off and back, a panic.
            chain.set(
                2,
                Some(Slot {
                    fx: Fx::Transpose { semitones: 7 },
                    on: k % 1000 == 250,
                }),
            );
        }
        if k == 1200 {
            // Strum, Latch and Randomize in place of three others.
            let more = [
                Fx::Strum(Strum {
                    time_ms: 12,
                    order: StrumOrder::Down,
                }),
                Fx::Latch {
                    mode: LatchMode::NextChord,
                },
                Fx::Randomize(Randomize {
                    pitch: 7,
                    often: 50,
                    velocity: 12,
                    longer_ms: 40,
                }),
            ];
            for (i, fx) in more.into_iter().enumerate() {
                chain.set(4 + i, Some(Slot { fx, on: true }));
            }
        }
        if k == 1700 {
            chain.set(
                5,
                Some(Slot {
                    fx: Fx::Latch {
                        mode: LatchMode::Toggle,
                    },
                    on: true,
                }),
            );
        }
        if k == 2000 {
            // A mute landing in the next period.
            chain.cut(start + u64::from(block) + 100);
        }
        if k == 3000 {
            chain.reset();
        }
        start += u64::from(block);
    }
    watch(false);
    assert_eq!(CALLS.load(Ordering::Relaxed), 0, "allocations in the loop");
    assert_eq!(chain.dropped(), 0);
}

fn mpfree_scale_minor() -> u16 {
    // A natural minor scale: 0 2 3 5 7 8 10.
    [0u16, 2, 3, 5, 7, 8, 10]
        .iter()
        .fold(0, |m, s| m | (1 << s))
}
