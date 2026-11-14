//! The parts that can run on the audio thread (or the MIDI reader thread)
//! must not allocate or free once made: routing input, routing output,
//! writing and pacing a period's bytes, the CV engine, the monitor's feed
//! and its drain into the list, and the export's note log. MIDI Learn is control-thread code and
//! is left out.
//!
//! This test file is its own program, so its global allocator (a
//! pass-through to the system allocator) does not affect the library.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};

use mpfree_midi::cv::{CvAssign, CvEngine, Priority, Scale};
use mpfree_midi::input::{In, InputMode, InputRouter, Listen};
use mpfree_midi::monitor::{Dir, Feed, Monitor, KEEP};
use mpfree_midi::output::{DinPacer, OutQueue, OutRoute, OutputRouter, Timed};
use mpfree_midi::smf::NoteLog;
use mpfree_midi::{Msg, PortId, DIN, MAX_PORTS};

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
fn routing_pacing_and_cv_never_allocate() {
    let mut input = InputRouter::new();
    input.set_mode(InputMode::PerTrack);
    for t in 0..16 {
        input.set_listen(t, Some(Listen::default()));
    }
    input.set_clock_source(Some(DIN));
    let mut out = OutputRouter::new();
    let mut queues: [OutQueue; MAX_PORTS] = std::array::from_fn(|_| OutQueue::new());
    let mut pacer = DinPacer::new(44_100);
    let mut cv = CvEngine::new(44_100);
    let mut bytes = [0u8; 1024];
    let mut status = 0u8;
    let mut seen = 0usize;

    watch(true);
    for t in 0..16u8 {
        out.set_route(
            t,
            Some(OutRoute {
                port: PortId(t % 2),
                channel: t,
            }),
            0,
            &mut queues,
        );
    }
    cv.set_assign(
        0,
        CvAssign::Pitch {
            track: 0,
            scale: Scale::VoltsPerOctave(1.0),
            zero_note: 36,
            bend_semis: 2,
            priority: Priority::Last,
        },
        0,
        &mut |_| {},
    );
    cv.set_assign(
        1,
        CvAssign::Gate {
            track: 0,
            volts: 5.0,
            retrigger_ms: 1.0,
        },
        0,
        &mut |_| {},
    );
    cv.set_assign(
        2,
        CvAssign::Clock {
            ppqn: 24,
            volts: 5.0,
            ms: 2.0,
        },
        0,
        &mut |_| {},
    );
    for period in 0..200u32 {
        for q in queues.iter_mut() {
            q.clear();
        }
        // Two keyboards' worth of input, with clock, Song Position and
        // a SysEx that is skipped.
        let n = (period % 100) as u8;
        let burst = [
            0xF2,
            1,
            0,
            0x90,
            n,
            100,
            n + 4,
            90,
            0xF8,
            0xB0,
            1,
            n,
            0xF0,
            1,
            2,
            3,
            0xF7,
            0x80,
            n,
            0,
            0x90,
            n + 4,
            0,
        ];
        let mut route = |e: In| {
            seen += 1;
            if let In::Track { track, msg, .. } = e {
                out.send(track, msg, 3, &mut queues);
                cv.track_msg(track, msg, 3, &mut |_| {});
            }
        };
        input.feed_all(DIN, &burst, u64::from(period) * 128, &mut route);
        input.feed_all(PortId(1), &burst, u64::from(period) * 128, &mut route);
        cv.clock(24, 0, &mut |_| {});
        if period % 50 == 49 {
            out.all_off(100, &mut queues);
            cv.release_all(100, &mut |_| {});
        }
        queues[0].push(Timed::realtime(64, 0xF8));
        queues[0].push(Timed::msg(10, Msg::PitchBend { ch: 0, value: 100 }));
        queues[0].sort();
        queues[0].write_bytes(&mut bytes, true, &mut status);
        pacer.pace(&queues[0], 128, |_, _| {});
        cv.end_period(128, &mut |_| {});
    }
    input.port_gone(PortId(1), 0, |_| {});
    input.release_all(0, |_| {});
    watch(false);

    assert_eq!(
        CALLS.load(Ordering::Relaxed),
        0,
        "allocations on the audio path"
    );
    assert!(seen > 1000);
    assert!(pacer.stats.messages > 0);
}

#[test]
fn the_monitor_never_allocates_once_made() {
    let mut feed = Feed::new();
    let mut m = Monitor::new();
    let usb = PortId(1);
    watch(true);
    let mut t = 0.0;
    // Past the list's length, so the oldest are dropped too.
    for i in 0..(KEEP / 4 + 50) {
        t = i as f64 * 0.005;
        feed.bytes(usb, Dir::In, t, &[0x90, 60, 100, 0xF8, 0x80, 60, 0]);
        feed.bytes(DIN, Dir::Out, t, &[0xB1, 74, (i % 128) as u8, 0xF8]);
        feed.bytes(DIN, Dir::In, t, &[0xF0, 0x47, 0x00, 0x01, 0xF7, 0xFE]);
        if i % 8 == 0 {
            feed.drain_into(&mut m, t);
        }
    }
    feed.drain_into(&mut m, t + 2.0);
    m.clear();
    feed.bytes(usb, Dir::In, t + 3.0, &[0xC0, 1]);
    feed.drain_into(&mut m, t + 3.0);
    watch(false);
    assert_eq!(CALLS.load(Ordering::Relaxed), 0, "the monitor allocated");
    assert_eq!(m.len(), 1);
    assert!(m.total() > KEEP as u64);
}

#[test]
fn the_note_log_never_allocates_once_made() {
    let mut log = NoteLog::with_capacity(1000);
    watch(true);
    for i in 0..1200u64 {
        let note = (i % 128) as u8;
        log.push(
            i * 120,
            (i % 16) as u16,
            Msg::NoteOn {
                ch: 0,
                note,
                vel: 100,
            },
        );
    }
    log.clear();
    log.push(0, 0, Msg::Program { ch: 0, program: 1 });
    watch(false);
    assert_eq!(CALLS.load(Ordering::Relaxed), 0, "the note log allocated");
    assert_eq!(log.len(), 1);
}
