//! The pass's sound-thread calls must not allocate or free: attaching,
//! every period with notes coming in, waiting for room in fast mode, and
//! detaching. A counting global allocator watches the test's own thread
//! (the writer thread allocates as it likes).
//!
//! This test file is its own program, so its global allocator (the only
//! unsafe code here, a pass-through to the system allocator) does not
//! affect the library, which forbids unsafe code.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use mpfree_export::mpfree_fx::mpfree_node::Ctx;
use mpfree_export::mpfree_fx::{Mixer, MixerConfig, StripId};
use mpfree_export::mpfree_midi::msg::Msg;
use mpfree_export::*;

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
fn the_pass_never_allocates_on_the_sound_thread() {
    let rate = 48_000;
    let mut m = Mixer::new(
        MixerConfig {
            inputs: 0,
            pads: 0,
            tracks: 2,
            slots: 1,
            max_latency_ms: 20.0,
        },
        rate,
        256,
    );
    let mut l = vec![0.0f32; 256];
    let mut r = vec![0.0f32; 256];
    m.process(&Ctx::new(rate, 0), &mut l, &mut r);
    let folder = std::env::temp_dir().join(format!("mpfree-export-noalloc-{}", std::process::id()));
    let plan = Plan {
        folder: folder.clone(),
        song: "Test".into(),
        streams: vec![
            Stream::mixdown(),
            Stream::stem("A", StripId::Track(0)),
            Stream::stem("B", StripId::Track(1)),
        ],
        midi: Some(MidiPlan {
            tracks: vec!["A".into(), "B".into()],
            tempos: vec![(0, 120.0)],
            meters: vec![(0, 4, 4)],
            end: 1000,
            room: 100,
        }),
        mode: Mode::Fast,
        rate,
        length: None,
        first_slot: 0,
    };
    let (mut pass, job) = start(plan).unwrap();
    let mut frame = 0u64;
    let mut steps = 0;
    watch(true);
    assert!(pass.attach(&mut m));
    let step = loop {
        assert!(pass.wait_for_room(256, Duration::from_secs(5)));
        for t in 0..2 {
            let (il, ir) = m.strip_input(StripId::Track(t)).unwrap();
            for i in 0..256 {
                let v = if frame < 48_000 {
                    0.2 * ((frame + i as u64) as f32 * 0.01 * (t + 1) as f32).sin()
                } else {
                    0.0
                };
                il[i] = v;
                ir[i] = v;
            }
        }
        m.process(&Ctx::new(rate, frame), &mut l, &mut r);
        // More notes than the log has room for: the rest are counted.
        pass.note(
            frame / 25,
            (steps % 2) as u16,
            Msg::NoteOn {
                ch: 0,
                note: 60,
                vel: 100,
            },
        );
        frame += 256;
        steps += 1;
        match pass.period(&m, 256, frame >= 48_000) {
            Step::Go => {}
            s => break s,
        }
    };
    let _ = pass.room();
    pass.detach(&mut m);
    watch(false);
    assert_eq!(step, Step::Done);
    assert_eq!(
        CALLS.load(Ordering::Relaxed),
        0,
        "allocations on the sound thread"
    );
    match job.finish(pass) {
        Outcome::Done(d) => assert_eq!(d.notes_dropped, steps - 100),
        o => panic!("{o:?}"),
    }
    std::fs::remove_dir_all(&folder).unwrap();
}
