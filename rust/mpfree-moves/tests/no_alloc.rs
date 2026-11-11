//! Playing moves, chasing them and logging them must not allocate or
//! free; nor must writing a gesture into a lane with room, keeping what
//! it replaced, or putting that back.
//!
//! This test file is its own program, so its global allocator (a
//! pass-through to the system allocator) does not affect the library.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};

use mpfree_moves::*;
use mpfree_param::{Owner, ParamAddr, Strip};
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

#[test]
fn playing_logging_and_writing_with_room_never_allocate() {
    let len = 4 * 3840;
    let addr = |p: u16| ParamAddr::new(Owner::Insert(Strip::Master, 1), p);
    let mut moves = Moves::new();
    for p in 0..8u16 {
        let pts: Vec<_> = (0..400)
            .map(|i| {
                let t = f64::from(p) * 1500.0 + f64::from(i) * 9.0;
                (
                    t,
                    ((i as f32) * 0.03 + f32::from(p)).sin() * 0.5 + 0.5,
                    Kind::Ramp,
                )
            })
            .chain(std::iter::once((
                f64::from(p) * 1500.0 + 3700.0,
                0.0,
                Kind::End,
            )))
            .collect();
        moves
            .put(Target::Fixed(addr(p)), Map::linear(0.0, 1.0), &pts, len)
            .unwrap();
    }
    moves
        .reserve(Target::Fixed(addr(0)), Map::linear(0.0, 1.0), 4096)
        .unwrap();
    let gesture: Vec<_> = (0..300)
        .map(|i| {
            (
                5000.0 + f64::from(i) * 7.0,
                (i as f32 * 0.1).cos() * 0.5 + 0.5,
                Kind::Ramp,
            )
        })
        .collect();

    // Room to keep what the gesture writes over, for an undo.
    let mut undo: Vec<Point> = Vec::with_capacity(8192);
    let mut ranges = [(0u32, 0u32, 0usize, 0usize); 2];
    let mut n_ranges = 0;
    let before = moves.clone();

    let mut t = Box::new(Transport::new(48_000, 128.0).unwrap());
    let mut block = Box::new(Block::new());
    let mut log = Box::new(MoveLog::new());
    assert!(t.command(0, Command::Play));
    let mut sent = 0u64;
    let mut tick = 0.0f64;
    watch(true);
    for period in 0..20_000u32 {
        // Moves into the log: two knobs, ending now and then.
        if period % 3 == 0 {
            log.input(
                period % 128,
                Input::Move {
                    addr: addr(period as u16 % 2),
                    value: (period % 100) as f32 / 100.0,
                    map: Map::linear(0.0, 1.0),
                },
            );
        }
        if period % 500 == 0 {
            log.input(0, Input::End { addr: addr(0) });
        }
        if period % 4000 == 3999 {
            t.command(t.frame(), Command::Stop);
        }
        if period % 4000 == 0 {
            t.command(t.frame(), Command::Play);
        }
        assert!(t.process(128, &mut block));
        log.process(&block);
        // A clip's pass, a period at a time.
        let step = 128.0 * 128.0 / 60.0 * 960.0 / 48_000.0;
        let next = tick + step;
        if next >= f64::from(len) {
            moves.play(tick, f64::from(len), |_, _, _| sent += 1);
            moves.play(0.0, next - f64::from(len), |_, _, _| sent += 1);
            tick = next - f64::from(len);
        } else {
            moves.play(tick, next, |_, _, _| sent += 1);
            tick = next;
        }
        if period % 1000 == 0 {
            moves.chase(tick, |_, _| sent += 1);
            let _ = (
                log.seq(),
                log.moving(),
                log.since(log.seq().saturating_sub(10)).count(),
            );
        }
        if period == 10_000 {
            // A gesture into a lane with room, keeping what it replaces.
            assert!(moves.fits(Target::Fixed(addr(0)), gesture.len()));
            moves
                .put_saving(
                    Target::Fixed(addr(0)),
                    Map::linear(0.0, 1.0),
                    &gesture,
                    len,
                    |a, b, old| {
                        let at = undo.len();
                        undo.extend_from_slice(old);
                        ranges[n_ranges] = (a, b, at, undo.len());
                        n_ranges += 1;
                    },
                )
                .unwrap();
        }
        if period == 15_000 {
            // And undone.
            for &(a, b, i, j) in ranges[..n_ranges].iter().rev() {
                moves
                    .restore(
                        Target::Fixed(addr(0)),
                        Map::linear(0.0, 1.0),
                        a,
                        b,
                        &undo[i..j],
                    )
                    .unwrap();
            }
        }
    }
    watch(false);
    assert_eq!(CALLS.load(Ordering::Relaxed), 0, "allocations or frees");
    assert!(n_ranges > 0);
    assert!(moves == before, "the undo put the lane back");
    assert!(sent > 10_000);
    assert!(log.seq() > 1000);
    assert_eq!(log.dropped(), 0);
}
