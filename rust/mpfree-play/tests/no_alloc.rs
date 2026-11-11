//! `Player::process`, `ArpHost::process` and `Recorder::process` must not
//! allocate or free. A counting global allocator watches a busy session
//! (five tracks, one of them through the arp, one recording, with loops,
//! tempo changes, locates, punch in and out, stop and panic) at period
//! sizes from 1 to 4096 frames.
//!
//! This test file is its own program, so its global allocator (the only
//! unsafe code here, a pass-through to the system allocator) does not
//! affect the library, which forbids unsafe code. Only the test's own
//! thread is watched. (The same pattern as mpfree-sampler's no_alloc test.)

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};

use mpfree_arp::{Engine, Mode, Rate, Settings};
use mpfree_play::{ArpHost, Event, Launch, Out, PlayView, Player, Recorder, Sink, ALL};
use mpfree_seq::{Clip, Note, Project, Quantise, Speed, Track};
use mpfree_transport::time::Swing;
use mpfree_transport::{Block, Command, Transport};

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

/// A fixed buffer sink, as the audio thread would use. Destination 0
/// (and a panic's `AllOff`) goes to the arp.
struct Counter {
    events: u64,
    last: u64,
    arp: ArpHost,
}
impl Sink for Counter {
    fn send(&mut self, o: Out) {
        assert!(o.frame >= self.last);
        self.last = o.frame;
        self.events += 1;
        if o.dest == 0 || o.dest == ALL {
            self.arp.input(o.offset, o.event);
        }
    }
}

#[test]
fn playing_never_allocates() {
    let mut p = Project::new();
    let mut launches = vec![];
    for i in 0..5usize {
        let mut t = Track::new("t", i as u16);
        let mut c = Clip::new("c", 960 * (i as i64 + 2));
        c.notes = (0..(8 * (i as i64 + 2))).map(|k| Note { tick: k * 120 + (k % 3) * 7, len: 100 + 300 * (k % 4), note: (k % 16) as u8, velocity: 100 }).collect();
        if i == 1 {
            c.speed = Speed::new(3, 2).unwrap();
        }
        if i == 2 {
            c.swing = Some(Swing::new(240, 70).unwrap());
            c.quantise = Quantise::new(120, 60);
        }
        t.slots[i] = Some(c);
        p.tracks.push(t);
        launches.push(Launch { track: i, slot: i, origin: (i as i64 - 2) * 480 });
    }
    let view = PlayView::compile(&p, &launches).unwrap();
    let mut t = Transport::new(44_100, 120.0).unwrap();
    let mut player = Player::new();
    let mut block = Block::new();
    let mut engine = Engine::new(44_100, 120.0);
    engine.set_settings(Settings { mode: Mode::Arp, rate: Rate::ThirtySecond, octaves: 2, ..Settings::default() });
    let mut sink = Counter { events: 0, last: 0, arp: ArpHost::new(engine) };
    let mut arp_notes = 0u64;
    let mut rec = Recorder::new();
    rec.arm(Some(3));
    let mut recorded = 0usize;

    watch(true);
    drop(std::hint::black_box(vec![0u8; 16]));
    watch(false);
    assert_eq!(CALLS.swap(0, Ordering::SeqCst), 2, "the counter works");

    let sizes = [1u32, 7, 128, 333, 4096, 64];
    watch(true);
    t.command(0, Command::SetLoop(Some((0, 7680))));
    t.command(0, Command::Play);
    for i in 0..4000u64 {
        let f = t.frame();
        match i % 500 {
            100 => {
                let _ = t.command(f + 10, Command::SetTempo(80.0 + (i % 7) as f64 * 11.0));
            }
            200 => {
                let _ = t.command(f, Command::Locate((i as i64 * 53) % 6000));
            }
            300 => {
                let _ = t.command(f + 2, Command::Stop);
            }
            310 => {
                let _ = t.command(f + 2, Command::Stop);
            }
            320 => {
                let _ = t.command(f + 2, Command::Play);
            }
            50 | 250 | 400 => {
                let _ = t.command(f + 1, Command::Record);
            }
            _ => {}
        }
        let n = sizes[(i % sizes.len() as u64) as usize];
        t.process(n, &mut block);
        sink.last = block.start_frame;
        player.process(&view, &block, &mut sink);
        sink.arp.process(&block, &mut |_, _| arp_notes += 1);
        // Pads on the recording track: notes held a few periods, some overlapping.
        let note = (i % 7) as u8;
        rec.input((i as u32 * 31) % n, Event::NoteOn { note, velocity: 100 });
        rec.input((i as u32 * 17) % n, Event::NoteOff { note: ((i + 4) % 7) as u8 });
        rec.process(&view, &block);
        recorded += rec.taken().len();
    }
    watch(false);
    assert_eq!(CALLS.load(Ordering::SeqCst), 0, "allocations or frees while playing");
    assert!(sink.events > 2000, "{}", sink.events);
    assert_eq!(player.stats().dropped, 0);
    assert!(arp_notes > 500, "{arp_notes}");
    assert_eq!(sink.arp.dropped(), 0);
    assert!(recorded > 500, "{recorded}");
    assert_eq!(rec.dropped(), 0);
}
