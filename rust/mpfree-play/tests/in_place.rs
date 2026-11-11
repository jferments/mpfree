//! Recording puts notes and locks into the play view in place
//! (`PlayView::add_note`, `remove_note`, `add_lock`, `remove_lock`), so it
//! need not compile the project on every note. The view must come out
//! exactly as compiling the project again would, and with room set aside
//! (`PlayView::make_room`, and the clip's own lists) it must not allocate.
//!
//! This test file is its own program, so its global allocator (the only
//! unsafe code here, a pass-through to the system allocator) does not
//! affect the library. Only the watching thread is counted.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use mpfree_play::{Cue, Erased, ErasedLock, Launch, PlayView, Taken, TakenLock, Takes};
use mpfree_seq::mpfree_param::{OnTrack, RelAddr};
use mpfree_seq::{Clip, Lock, Note, Project, Quantise, Speed, Track};

struct Counting;
thread_local! {
    static WATCH: Cell<bool> = const { Cell::new(false) };
    static CALLS: Cell<usize> = const { Cell::new(0) };
}

fn count() {
    // `try_with`: the allocator also runs while thread locals are torn down.
    let _ = WATCH.try_with(|w| {
        if w.get() {
            let _ = CALLS.try_with(|c| c.set(c.get() + 1));
        }
    });
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

/// Allocations and frees made by this thread while running `f`.
fn calls<R>(f: impl FnOnce() -> R) -> (R, usize) {
    CALLS.with(|c| c.set(0));
    WATCH.with(|w| w.set(true));
    let r = f();
    WATCH.with(|w| w.set(false));
    (r, CALLS.with(Cell::get))
}

fn xorshift(mut x: u64) -> impl FnMut() -> u64 {
    move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    }
}

const LEN: i64 = 4 * 3840;
const ADDRS: [RelAddr; 3] = [
    RelAddr::new(OnTrack::Inst, 3),
    RelAddr::new(OnTrack::Strip, 0),
    RelAddr::new(OnTrack::Pad(2), 1),
];

/// The project, what plays and the take history, as the pad player keeps
/// them, with its view.
struct World {
    project: Project,
    launches: Vec<Launch>,
    cues: Vec<Cue>,
    takes: Takes,
    view: PlayView,
    room: usize,
    compiles: usize,
}

impl World {
    /// Track 0 (armed) plays slot 0 and has `cue` waiting; track 1 plays a
    /// clip of its own. Every clip uses `quantise` and `speed`.
    fn new(quantise: Option<Quantise>, speed: Speed, cue: Option<usize>, room: usize) -> World {
        let mut project = Project::new();
        for i in 0..2 {
            project.tracks.push(Track::new("t", i));
        }
        let mut rnd = xorshift(99);
        for (track, slot) in [(0, 0), (0, 1), (1, 0)] {
            let mut c = Clip::new("c", LEN);
            c.quantise = quantise;
            c.speed = speed;
            for _ in 0..20 {
                let tick = (rnd() % LEN as u64) as i64;
                c.notes.push(Note::new(tick, 120, (rnd() % 16) as u8, 100));
            }
            c.locks.push(Lock::new(240, None, ADDRS[0], 0.5));
            project.tracks[track].slots[slot] = Some(c);
        }
        let launches = vec![
            Launch {
                track: 0,
                slot: 0,
                origin: 0,
            },
            Launch {
                track: 1,
                slot: 0,
                origin: 0,
            },
        ];
        let cues = cue
            .map(|slot| Cue {
                track: 0,
                slot: Some(slot),
            })
            .into_iter()
            .collect();
        let mut w = World {
            project,
            launches,
            cues,
            takes: Takes::default(),
            view: PlayView::empty(),
            room,
            compiles: 0,
        };
        w.compile();
        w
    }

    /// As the pad player does: compile, then set room aside in the armed
    /// track's clips and in the view.
    fn compile(&mut self) {
        self.view = self.fresh();
        self.view.make_room(self.room, self.room);
        for slot in [0, 1] {
            if let Some(c) = self.project.tracks[0].slots[slot].as_mut() {
                c.notes.reserve(self.room);
                c.locks.reserve(self.room);
            }
        }
        self.compiles += 1;
    }

    fn fresh(&self) -> PlayView {
        let mut v = PlayView::compile_cued(&self.project, &self.launches, &self.cues).unwrap();
        v.set_armed(Some(0));
        v
    }

    fn clip(&self, slot: usize) -> &Clip {
        self.project.tracks[0].slots[slot].as_ref().unwrap()
    }

    /// One random recording step into clip `slot` of track 0, through the
    /// take history and then the view in place (compiling only if the view
    /// says so). False if nothing changed.
    fn step(&mut self, rnd: &mut impl FnMut() -> u64, slot: usize) -> bool {
        let take = 1;
        let (changed, in_place) = match rnd() % 4 {
            0 => {
                // Ticks on a few lines, so equal places and equal notes happen.
                let tick = (rnd() % 64) as i64 * 240 + [0, 7, 120][(rnd() % 3) as usize];
                let note = Note::new(tick, 1 + (rnd() % 400) as i64, (rnd() % 4) as u8, 100);
                let k = Taken {
                    track: 0,
                    slot,
                    note,
                    take,
                };
                match self.takes.add(&mut self.project, &k) {
                    Ok(()) => (true, self.view.add_note(0, slot, &note)),
                    Err(_) => (false, true),
                }
            }
            1 => {
                let n = self.clip(slot).notes.len();
                if n == 0 {
                    return false;
                }
                let note = self.clip(slot).notes[(rnd() % n as u64) as usize];
                let e = Erased {
                    track: 0,
                    slot,
                    note,
                    take,
                };
                assert!(self.takes.erase(&mut self.project, &e));
                (true, self.view.remove_note(0, slot, &note))
            }
            2 => {
                let tick = (rnd() % 16) as i64 * 240 + [0, 60][(rnd() % 2) as usize];
                let sound = [None, Some(0), Some(2)][(rnd() % 3) as usize];
                let addr = ADDRS[(rnd() % 3) as usize];
                let lock = Lock::new(tick, sound, addr, (rnd() % 1000) as f32 / 1000.0);
                let k = TakenLock {
                    track: 0,
                    slot,
                    lock,
                    take,
                };
                match self.takes.add_lock(&mut self.project, &k) {
                    Ok(old) => (true, self.view.add_lock(0, slot, &lock, old.as_ref())),
                    Err(_) => (false, true),
                }
            }
            _ => {
                let n = self.clip(slot).locks.len();
                if n == 0 {
                    return false;
                }
                let lock = self.clip(slot).locks[(rnd() % n as u64) as usize];
                let e = ErasedLock {
                    track: 0,
                    slot,
                    lock,
                    take,
                };
                assert!(self.takes.erase_lock(&mut self.project, &e));
                (true, self.view.remove_lock(0, slot, &lock))
            }
        };
        if !in_place {
            self.compile();
        }
        changed
    }
}

/// Thousands of random notes and locks recorded and erased, with and
/// without quantise and speed, into the playing clip, a cued one, or the
/// playing clip cued again: the view always equals a fresh compile.
#[test]
fn in_place_edits_match_a_fresh_compile() {
    let q = [None, Quantise::new(240, 100), Quantise::new(240, 60)];
    let sp = [
        Speed::NORMAL,
        Speed::new(2, 1).unwrap(),
        Speed::new(1, 3).unwrap(),
    ];
    let mut seed = 1;
    for quantise in q {
        for speed in sp {
            for cue in [None, Some(0), Some(1)] {
                seed += 1;
                let mut rnd = xorshift(seed * 0x9e37_79b9);
                let mut w = World::new(quantise, speed, cue, 4096);
                let mut changes = 0;
                for i in 0..1500 {
                    let slot = if cue == Some(1) && i % 3 == 0 { 1 } else { 0 };
                    if w.step(&mut rnd, slot) {
                        changes += 1;
                    }
                    assert_eq!(
                        w.view,
                        w.fresh(),
                        "step {i}, {quantise:?} {speed:?} cue {cue:?}"
                    );
                }
                assert!(changes > 1000);
                assert_eq!(w.compiles, 1, "all in place: room was set aside");
            }
        }
    }
}

/// With little room set aside, the view says when it is full, and
/// compiling then gives the same view.
#[test]
fn a_full_view_asks_for_a_compile() {
    let mut rnd = xorshift(7);
    let mut w = World::new(Quantise::new(240, 100), Speed::NORMAL, Some(0), 4);
    for i in 0..600 {
        w.step(&mut rnd, 0);
        assert_eq!(w.view, w.fresh(), "step {i}");
    }
    assert!(
        w.compiles > 1,
        "the view filled up: {} compiles",
        w.compiles
    );
}

/// With room set aside, recording into the view and the clip allocates
/// nothing once the take has begun.
#[test]
fn recording_in_place_does_not_allocate() {
    let mut rnd = xorshift(3);
    let mut w = World::new(Quantise::new(240, 100), Speed::NORMAL, Some(0), 1024);
    // The take's first note makes its entry in the history (that allocates).
    let first = Taken {
        track: 0,
        slot: 0,
        note: Note::new(0, 10, 1, 100),
        take: 1,
    };
    w.takes.add(&mut w.project, &first).unwrap();
    assert!(w.view.add_note(0, 0, &first.note));
    let ((), allocs) = calls(|| {
        for _ in 0..400 {
            w.step(&mut rnd, 0);
        }
    });
    assert_eq!(w.compiles, 1);
    assert_eq!(allocs, 0, "allocations or frees while recording in place");
    assert_eq!(w.view, w.fresh());
}
