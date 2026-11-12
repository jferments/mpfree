//! A time engine on two threads (docs/dev/design/time-threads.md): the
//! audio thread makes each recorded change to the view it plays
//! (`PlayView::record`) and numbers it (`Recent`); the control thread makes
//! the same changes to the project (`Takes::record`) when it gets to them,
//! and compiles views that say how many they include. A view that arrives
//! missing the newest changes catches up (`Recent::catch_up`) before it
//! plays. Every view played must equal a fresh compile of the project with
//! every change made so far.

use mpfree_play::{Behind, Cue, Includes, Launch, PlayView, Rec, Recent, Takes, RECENT};
use mpfree_play::{Erased, ErasedLock, Taken, TakenLock};
use mpfree_seq::mpfree_param::{OnTrack, RelAddr};
use mpfree_seq::{Clip, Lock, Note, Project, Quantise, Speed, TouchPoint, Track};

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

/// One recorded change and its pressure.
type Change = (Rec, Vec<TouchPoint>);

/// The two halves: the control thread's project, launches and takes, and
/// the audio thread's view and recent changes, with the changes the
/// control thread has not made yet.
struct World {
    project: Project,
    launches: Vec<Launch>,
    cues: Vec<Cue>,
    takes: Takes,
    /// Changes made to the project so far (the last one's number).
    applied: u64,
    /// Every change, in order (index 0 is number 1).
    log: Vec<Change>,
    view: PlayView,
    recent: Recent,
    room: usize,
    /// Views compiled, and views that could not catch up (by why).
    compiles: usize,
    too_old: usize,
    no_room: usize,
}

impl World {
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
        let launch = |track| Launch {
            track,
            slot: 0,
            origin: 0,
            start: 0,
        };
        let mut w = World {
            project,
            launches: vec![launch(0), launch(1)],
            cues: cue
                .map(|slot| Cue {
                    track: 0,
                    slot: Some(slot),
                })
                .into_iter()
                .collect(),
            takes: Takes::default(),
            applied: 0,
            log: Vec::new(),
            view: PlayView::empty(),
            recent: Recent::new(),
            room,
            compiles: 0,
            too_old: 0,
            no_room: 0,
        };
        w.view = w.compile();
        w
    }

    /// The control thread's compile, with room set aside as the pad player
    /// does, saying what it includes.
    fn compile(&mut self) -> PlayView {
        let mut v = self.fresh();
        v.make_room(self.room, self.room);
        for slot in [0, 1] {
            if let Some(c) = self.project.tracks[0].slots[slot].as_mut() {
                c.notes.reserve(self.room);
                c.locks.reserve(self.room);
                c.touch.reserve(4 * self.room);
            }
        }
        self.compiles += 1;
        v
    }

    fn fresh(&self) -> PlayView {
        let mut v = PlayView::compile_cued(&self.project, &self.launches, &self.cues).unwrap();
        v.set_armed(Some(0));
        v.set_includes(Includes {
            recorded: self.applied,
            ..Includes::default()
        });
        v
    }

    /// The control thread catches up with the changes up to `to`.
    fn apply_to(&mut self, to: u64) {
        while self.applied < to {
            let (rec, touch) = &self.log[self.applied as usize];
            self.takes.record(&mut self.project, rec, touch, &[]);
            self.applied += 1;
        }
    }

    /// The project with every change made, compiled: what the audio
    /// thread's view must equal.
    fn truth(&self) -> PlayView {
        let mut w = World {
            project: self.project.clone(),
            launches: self.launches.clone(),
            cues: self.cues.clone(),
            takes: Takes::default(),
            applied: self.applied,
            log: Vec::new(),
            view: PlayView::empty(),
            recent: Recent::new(),
            room: 0,
            compiles: 0,
            too_old: 0,
            no_room: 0,
        };
        for (rec, touch) in &self.log[self.applied as usize..] {
            w.takes.record(&mut w.project, rec, touch, &[]);
            w.applied += 1;
        }
        w.fresh()
    }

    /// The audio thread records one change: into its view in place (a
    /// compile on the spot if there is no room), numbered and kept.
    fn record(&mut self, rec: Rec, touch: Vec<TouchPoint>) {
        let number = self.recent.push(rec, &touch, &[]);
        assert_eq!(number, self.log.len() as u64 + 1);
        let in_place = self.view.record(&rec, &touch, &[]);
        self.log.push((rec, touch));
        if !in_place {
            // The single-thread way, for the test: catch the project up
            // and compile. The two-thread host asks and keeps playing.
            self.apply_to(self.log.len() as u64);
            self.view = self.compile();
        }
    }

    /// A view the control thread compiled after making the changes up to
    /// `to` arrives: it catches up, or the audio thread keeps its own.
    fn arrive(&mut self, to: u64) {
        self.apply_to(to);
        let mut v = self.compile();
        match self.recent.catch_up(&mut v, to) {
            Ok(()) => self.view = v,
            Err(Behind::TooOld) => self.too_old += 1,
            Err(Behind::NoRoom) => self.no_room += 1,
        }
    }

    fn clip(&self, slot: usize) -> &Clip {
        self.project.tracks[0].slots[slot].as_ref().unwrap()
    }

    /// The notes and locks clip `slot` holds once every change is made.
    fn truth_clip(&self, slot: usize) -> Clip {
        let mut p = self.project.clone();
        let mut takes = Takes::default();
        for (rec, touch) in &self.log[self.applied as usize..] {
            takes.record(&mut p, rec, touch, &[]);
        }
        p.tracks[0].slots[slot].clone().unwrap()
    }

    /// One random recorded change to clip `slot` of track 0: a note (now
    /// and then with pressure, or past the clip's end, which both sides
    /// leave out), an erase of a note it holds, a lock (often in the place
    /// of one), or an erase of a lock.
    fn random_change(&mut self, rnd: &mut impl FnMut() -> u64, slot: usize) {
        let take = 1;
        let clip = self.truth_clip(slot);
        let rec = match rnd() % 5 {
            0 | 1 => {
                let tick = if rnd() % 40 == 0 {
                    LEN + 5
                } else {
                    (rnd() % 64) as i64 * 240 + [0, 7, 120][(rnd() % 3) as usize]
                };
                let mut note = Note::new(tick, 1 + (rnd() % 400) as i64, (rnd() % 4) as u8, 100);
                // The recorder's own buffer handle: both sides replace it.
                note.touch.first = 3;
                let n = if rnd() % 2 == 0 { 0 } else { 1 + rnd() % 6 };
                let touch: Vec<TouchPoint> = (0..n)
                    .map(|i| TouchPoint {
                        at: i as u32 * 30,
                        value: (rnd() % 128) as u8,
                    })
                    .collect();
                let k = Taken {
                    track: 0,
                    slot,
                    note,
                    take,
                };
                return self.record(Rec::Note(k), touch);
            }
            2 => {
                if clip.notes.is_empty() {
                    return;
                }
                let note = clip.notes[(rnd() % clip.notes.len() as u64) as usize];
                Rec::Erase(Erased {
                    track: 0,
                    slot,
                    note,
                    take,
                })
            }
            3 => {
                let tick = (rnd() % 16) as i64 * 240 + [0, 60][(rnd() % 2) as usize];
                let sound = [None, Some(0), Some(2)][(rnd() % 3) as usize];
                let addr = ADDRS[(rnd() % 3) as usize];
                let lock = Lock::new(tick, sound, addr, (rnd() % 1000) as f32 / 1000.0);
                Rec::Lock(TakenLock {
                    track: 0,
                    slot,
                    lock,
                    take,
                })
            }
            _ => {
                if clip.locks.is_empty() {
                    return;
                }
                let lock = clip.locks[(rnd() % clip.locks.len() as u64) as usize];
                Rec::EraseLock(ErasedLock {
                    track: 0,
                    slot,
                    lock,
                    take,
                })
            }
        };
        self.record(rec, Vec::new());
    }
}

/// Recorded changes decide alike in the view and the project: with the
/// control thread keeping up, the view always equals a fresh compile,
/// pressure, refused notes and replaced locks included, with no compile
/// after the first.
#[test]
fn a_recorded_change_is_decided_alike_in_the_view_and_the_project() {
    let q = [None, Quantise::new(240, 100), Quantise::new(240, 60)];
    let sp = [Speed::NORMAL, Speed::new(1, 3).unwrap()];
    let mut seed = 1;
    for quantise in q {
        for speed in sp {
            for cue in [None, Some(0), Some(1)] {
                seed += 1;
                let mut rnd = xorshift(seed * 0x9e37_79b9);
                let mut w = World::new(quantise, speed, cue, 4096);
                for i in 0..800 {
                    let slot = if cue == Some(1) && i % 3 == 0 { 1 } else { 0 };
                    w.random_change(&mut rnd, slot);
                    w.apply_to(w.log.len() as u64);
                    let fresh = w.fresh();
                    assert_eq!(
                        w.view, fresh,
                        "change {i}, {quantise:?} {speed:?} cue {cue:?}"
                    );
                }
                assert_eq!(w.compiles, 1, "all in place");
                let notes_touched = w
                    .clip(0)
                    .notes
                    .iter()
                    .filter(|n| !n.touch.is_none())
                    .count();
                assert!(notes_touched > 30, "{notes_touched} notes with pressure");
            }
        }
    }
}

/// The control thread runs late by random amounts: each view it compiles
/// misses some of the newest changes, catches up with them before it
/// plays, and then equals a fresh compile with every change made. One
/// too far behind for `Recent` is refused, and the view playing (which
/// holds everything) stays.
#[test]
fn a_late_view_catches_up_before_it_plays() {
    for (seed, cue) in [(11, None), (12, Some(0)), (13, Some(1))] {
        let mut rnd = xorshift(seed * 0x2545_f491);
        let mut w = World::new(Quantise::new(240, 100), Speed::NORMAL, cue, 4096);
        let mut arrived = 0;
        for i in 0..3000 {
            let slot = if cue == Some(1) && i % 3 == 0 { 1 } else { 0 };
            w.random_change(&mut rnd, slot);
            if rnd() % 7 == 0 {
                // Compiled after making all but the last few changes.
                let behind = rnd() % 12;
                let to = (w.log.len() as u64).saturating_sub(behind).max(w.applied);
                w.arrive(to);
                arrived += 1;
            }
            assert_eq!(w.view, w.truth(), "change {i}, cue {cue:?}");
        }
        assert!(arrived > 300);
        assert_eq!((w.too_old, w.no_room), (0, 0));
    }
    // A view further behind than Recent keeps is refused.
    let mut rnd = xorshift(5);
    let mut w = World::new(None, Speed::NORMAL, None, 8192);
    while w.log.len() <= RECENT + 10 {
        w.random_change(&mut rnd, 0);
    }
    w.arrive(5);
    assert_eq!(w.too_old, 1);
    assert_eq!(w.view, w.truth(), "the view playing stays whole");
    // One just inside it catches up.
    let to = w.log.len() as u64 - RECENT as u64;
    w.arrive(to);
    assert_eq!(w.too_old, 1);
    assert_eq!(w.view, w.truth());
}

/// A late view with little room set aside cannot take the missing
/// changes in place: the catch-up says so, and the view playing stays.
#[test]
fn a_late_view_without_room_is_refused() {
    let mut rnd = xorshift(21);
    let mut w = World::new(None, Speed::NORMAL, None, 4096);
    for _ in 0..200 {
        w.random_change(&mut rnd, 0);
    }
    w.room = 0;
    let to = w.applied;
    w.arrive(to);
    assert_eq!(w.no_room, 1);
    assert_eq!(w.view, w.truth());
}
