//! In the pad player, `TimeHost` feeds the song recorder from its period,
//! which runs in the audio loop until edits move to a control thread. A
//! period where nothing starts or stops must not allocate or free; only a
//! clip starting or stopping, a pass beginning or ending and writing the
//! song at a stop do (as any edit does there today).
//!
//! This test file is its own program, so its counting global allocator
//! (a pass-through to the system allocator) does not affect the library.
//! Only the test's own thread is watched.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};

use mpfree_perform::SongRecorder;
use mpfree_play::{Launch, Out, PlayView, Player, Sink};
use mpfree_seq::{Clip, Note, Project, Track};
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
static A: Counting = Counting;

struct Null;
impl Sink for Null {
    fn send(&mut self, _: Out) {}
}

#[test]
fn a_period_where_nothing_starts_records_without_allocating() {
    let mut p = Project::new();
    let mut launches = Vec::new();
    for t in 0..4u8 {
        let mut tr = Track::new("t", t.into());
        let mut c = Clip::new("c", 3840);
        c.notes = (0..4)
            .map(|i| Note::new(i * 960, 480, 36 + t, 100))
            .collect();
        tr.slots[0] = Some(c);
        p.tracks.push(tr);
        launches.push(Launch {
            track: t.into(),
            slot: 0,
            origin: 0,
            start: 0,
        });
    }
    let view = PlayView::compile(&p, &launches).unwrap();
    let mut t = Transport::new(48_000, 120.0).unwrap();
    assert!(t.command(0, Command::Play));
    let (mut player, mut block) = (Player::new(), Block::new());
    let mut rec = SongRecorder::default();
    rec.arm(true, &p, &launches, 0.0, (0, 1));
    for period in [1u32, 7, 128, 1000, 4096] {
        // The first period begins the pass (that may allocate).
        assert!(t.process(period, &mut block));
        player.process(&view, &block, &mut Null);
        rec.period(&block, player.started());
        assert!(rec.recording());
        let before = CALLS.load(Ordering::Relaxed);
        WATCH.with(|w| w.set(true));
        for _ in 0..200 {
            assert!(t.process(period, &mut block));
            player.process(&view, &block, &mut Null);
            rec.period(&block, player.started());
        }
        WATCH.with(|w| w.set(false));
        assert_eq!(
            CALLS.load(Ordering::Relaxed) - before,
            0,
            "period {period}: steady recording allocated"
        );
    }
    rec.arm(false, &p, &launches, 0.0, (0, 1));
    let got = rec.finish(&mut p).unwrap();
    assert_eq!(got.passes, 1);
    assert_eq!(got.placements, 4);
}
