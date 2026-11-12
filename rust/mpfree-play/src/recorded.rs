//! Recorded changes, numbered, for a time engine run on two threads.
//!
//! Recording changes the clips on the audio thread: each note or lock the
//! recorder takes, each one a replace pass erases, and each part of a
//! gesture is one [`Rec`]. The audio thread makes it to the view it plays
//! in place ([`PlayView::record`]), numbers it and keeps it in [`Recent`];
//! the control thread makes it to the project ([`crate::Takes::record`])
//! in the same order. The view and the project decide alike: a recorded
//! note's pressure goes at the end of its clip's pressure unless that
//! would pass `MAX_TOUCH_POINTS`, a lock takes the place of the one in its
//! place, and a note or lock the clip cannot hold is left out by both
//! (the rules are `Note::fits`, `Lock::fits` and `touch_fits` in
//! `mpfree-seq`).
//!
//! A view the control thread compiles says how many changes it includes
//! ([`Includes::recorded`]). The control thread may run a little late, so
//! a view can arrive missing the newest changes, which the audio thread
//! already made to the view it plays. Before swapping such a view in, the
//! audio thread makes the missing ones to it in order ([`Recent::catch_up`]),
//! so nothing recorded is lost or doubled. [`Recent`] keeps the last
//! [`RECENT`] changes and their points; if the view is older than that, or
//! has no room, the catch-up fails and the host keeps playing the view it
//! has (which holds everything) and asks for a new compile.
//!
//! No allocation after [`Recent::new`]: fixed rings.

use mpfree_seq::mpfree_moves::Kind;
use mpfree_seq::TouchPoint;

use crate::player::{Erased, ErasedLock};
use crate::record::{Taken, TakenLock, TakenMove, MOVE_POINTS, TOUCH_POINTS};
use crate::view::PlayView;

/// Recorded changes [`Recent`] keeps.
pub const RECENT: usize = 1024;
/// Pressure values [`Recent`] keeps with them: four full notes' worth.
pub const RECENT_TOUCH: usize = 4 * TOUCH_POINTS;
/// Move points [`Recent`] keeps with them: four full parts' worth.
pub const RECENT_MOVES: usize = 4 * MOVE_POINTS;

/// One recorded change to a clip. A note's pressure and a gesture's points
/// travel beside it (`Recorder::touch_points`, `Recorder::move_points`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Rec {
    /// A note the recorder took, with its pressure run beside it. Its
    /// `note.touch` is the recorder's own and is replaced.
    Note(Taken),
    /// A note a replace pass erased.
    Erase(Erased),
    /// A lock the recorder took, in place of the one in its place.
    Lock(TakenLock),
    /// A lock a replace pass erased.
    EraseLock(ErasedLock),
    /// A part of a gesture, with its points beside it.
    Move(TakenMove),
}

impl Rec {
    /// The track and clip slot it changes.
    pub fn clip(&self) -> (usize, usize) {
        match self {
            Rec::Note(k) => (k.track, k.slot),
            Rec::Erase(e) => (e.track, e.slot),
            Rec::Lock(k) => (k.track, k.slot),
            Rec::EraseLock(e) => (e.track, e.slot),
            Rec::Move(k) => (k.track, k.slot),
        }
    }
}

/// What a view includes, for a host that compiles it on another thread:
/// the recorded changes made to the project before it compiled, and the
/// cues started on each track that it shows as launches. Unset, a view
/// follows the old rule (a started cue shows once the view's cue is gone),
/// which suits a host that compiles in the same period.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Includes {
    /// The number of the last [`Rec`] made to the project (0: none).
    pub recorded: u64,
    /// Cues started on each track and applied (`Started::apply`).
    pub switches: [u32; mpfree_seq::MAX_TRACKS],
}

/// Why a view could not catch up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Behind {
    /// It needs changes [`Recent`] no longer holds.
    TooOld,
    /// It has no room for one of them in place: compile again.
    NoRoom,
}

#[derive(Clone, Copy, Debug)]
struct Kept {
    rec: Rec,
    /// Where its points start in the pool they go in (counted from the
    /// first point ever kept), and how many.
    from: u64,
    n: u32,
}

/// The last [`RECENT`] recorded changes, numbered from 1, with their
/// points: what a view compiled a little late may still need.
pub struct Recent {
    kept: Vec<Kept>,
    touch: Vec<TouchPoint>,
    moves: Vec<(f64, f32, Kind)>,
    /// The number the last change got (0: none yet).
    last: u64,
    /// Points written to each pool so far.
    touch_written: u64,
    moves_written: u64,
    /// One run's points in a piece, for a run that wrapped round its pool.
    scratch_touch: Vec<TouchPoint>,
    scratch_moves: Vec<(f64, f32, Kind)>,
}

impl Default for Recent {
    fn default() -> Self {
        Recent::new()
    }
}

const NO_REC: Kept = Kept {
    rec: Rec::EraseLock(ErasedLock {
        track: 0,
        slot: 0,
        lock: mpfree_seq::Lock {
            tick: 0,
            sound: None,
            addr: mpfree_param::RelAddr::new(mpfree_param::OnTrack::Inst, 0),
            value: 0.0,
        },
        take: 0,
    }),
    from: 0,
    n: 0,
};

impl Recent {
    /// Empty, with every table allocated.
    pub fn new() -> Recent {
        let zero_touch = TouchPoint { at: 0, value: 0 };
        Recent {
            kept: vec![NO_REC; RECENT],
            touch: vec![zero_touch; RECENT_TOUCH],
            moves: vec![(0.0, 0.0, Kind::Step); RECENT_MOVES],
            last: 0,
            touch_written: 0,
            moves_written: 0,
            scratch_touch: vec![zero_touch; TOUCH_POINTS],
            scratch_moves: vec![(0.0, 0.0, Kind::Step); MOVE_POINTS],
        }
    }

    /// The number of the last change kept (0: none yet).
    pub fn last(&self) -> u64 {
        self.last
    }

    /// Keep `rec` with its points (a note's pressure, `touch`, or a
    /// gesture's, `moves`; the other is empty) and give it the next
    /// number, which it returns. Real-time safe. Points past one note's or
    /// one part's most are cut (the recorder never sends more).
    pub fn push(&mut self, rec: Rec, touch: &[TouchPoint], moves: &[(f64, f32, Kind)]) -> u64 {
        self.last += 1;
        let (from, n) = match rec {
            Rec::Note(_) => {
                let pts = &touch[..touch.len().min(TOUCH_POINTS)];
                let from = self.touch_written;
                for (i, p) in pts.iter().enumerate() {
                    self.touch[(from as usize + i) % RECENT_TOUCH] = *p;
                }
                self.touch_written += pts.len() as u64;
                (from, pts.len() as u32)
            }
            Rec::Move(_) => {
                let pts = &moves[..moves.len().min(MOVE_POINTS)];
                let from = self.moves_written;
                for (i, p) in pts.iter().enumerate() {
                    self.moves[(from as usize + i) % RECENT_MOVES] = *p;
                }
                self.moves_written += pts.len() as u64;
                (from, pts.len() as u32)
            }
            _ => (0, 0),
        };
        self.kept[(self.last % RECENT as u64) as usize] = Kept { rec, from, n };
        self.last
    }

    /// Make the changes after number `after` (the last one `view`
    /// includes, [`Includes::recorded`]) to `view` in order, in place.
    /// Real-time safe. On an error the view may hold some of them; don't
    /// play it.
    pub fn catch_up(&mut self, view: &mut PlayView, after: u64) -> Result<(), Behind> {
        if after >= self.last {
            return Ok(());
        }
        if self.last - after > RECENT as u64 {
            return Err(Behind::TooOld);
        }
        for number in after + 1..=self.last {
            let k = self.kept[(number % RECENT as u64) as usize];
            let ok = match k.rec {
                Rec::Note(_) => {
                    if self.touch_written - k.from > RECENT_TOUCH as u64 {
                        return Err(Behind::TooOld);
                    }
                    let n = k.n as usize;
                    for i in 0..n {
                        self.scratch_touch[i] = self.touch[(k.from as usize + i) % RECENT_TOUCH];
                    }
                    view.record(&k.rec, &self.scratch_touch[..n], &[])
                }
                Rec::Move(_) => {
                    if self.moves_written - k.from > RECENT_MOVES as u64 {
                        return Err(Behind::TooOld);
                    }
                    let n = k.n as usize;
                    for i in 0..n {
                        self.scratch_moves[i] = self.moves[(k.from as usize + i) % RECENT_MOVES];
                    }
                    view.record(&k.rec, &[], &self.scratch_moves[..n])
                }
                _ => view.record(&k.rec, &[], &[]),
            };
            if !ok {
                return Err(Behind::NoRoom);
            }
        }
        Ok(())
    }
}
