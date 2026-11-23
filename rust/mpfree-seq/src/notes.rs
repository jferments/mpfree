//! The note editors' view of a clip (the MPC's Grid View and List Edit):
//! picking notes, editing them, and Timing Correct.
//!
//! Like [`crate::steps`], nothing here changes a clip: every function
//! returns an [`Edit`] (the notes to take out and to put in), which the
//! control thread applies as one undoable take. A **selection** is a list
//! of indexes into `clip.notes` as it is now; an edit made from it names
//! the notes themselves, so it applies (and undoes) whatever order the
//! notes are kept in.
//!
//! **Timing Correct** ([`Tc`], [`timing_correct`]) works as the MPC's TC
//! window does, on all the notes, the selected ones, or a range of ticks
//! and notes ([`Which`]):
//!
//! - **Start** moves each note's start toward the nearest line of the
//!   division (its length stays); **End** moves its end (its start stays);
//!   **Length** makes its length a whole number of divisions; **Legato**
//!   holds each note until the next one starts.
//! - **Swing** (50 to 75 %) moves every second line later: 50 is straight,
//!   about 66 a triplet feel. **Window** (0 to 100 %) corrects only notes
//!   that close to a line (100: every note); **Strength** (0 to 100 %) is
//!   how far of the way they move; **Shift** then moves every chosen note
//!   by that many ticks.
//! - A start that comes to the clip's end wraps to its start, as the clip
//!   goes round.
//!
//! [`Tc::correct`] gives one tick's corrected place, for recording with
//! Timing Correct on.
//!
//! **Locks** go with their notes: a lock on a note's own sound and tick
//! moves when the note moves and goes when it is deleted; a duplicate
//! copies it. Locks on a whole step (no sound) stay on their line, as
//! the step editor keeps them. Double length copies the locks and the FX
//! moves too; half and double speed move the locks, and are refused for a
//! clip with FX moves (moves cannot be stretched yet).
//!
//! **Pressure** ([`crate::Touch`]) rides on its note: moving, Timing
//! Correct, transposing and duplicating keep it (a shortened note just
//! stops sending what falls past its end). A split leaves it on the first
//! part only. Half and double speed stretch it with its note: each note's
//! run comes again with its times doubled or halved, added to the clip's
//! pressure (`Edit::touch`); they are refused for a clip with no room
//! left for that. Pressure drawn on the lane under the roll
//! ([`draw_touch`], piano roll plan 3A.1) gives each note it crosses a
//! new run the same way; the note's old one stays behind for undo.
//!
//! **Locks on notes** (piano roll plan 3A.2): a lock belongs to a step
//! and a row, as the Steps view has it ([`steps_grid`]). The roll's lock
//! lanes read and set the lock where each note plays it ([`lock_place`]):
//! on the note's own pitch when one sits there already, else on its pad
//! for a pad's setting, else on the whole step. Notes that share a step
//! and a row share the lock ([`lock_notes`], [`unlock_notes`]).
//!
//! **Copying between clips** ([`copy`], [`carry`]; piano roll plan 1B.4)
//! takes the notes out as a [`Copied`] of their own, with their pressure
//! and the locks they own, so the copy holds whatever happens to the clip
//! after. Put into a clip, their pressure comes in as new runs at the end
//! of that clip's pressure (a note never names another clip's runs), and
//! their locks come where the clip's track has the same setting
//! ([`same_setting`]: an instrument lock where the track plays the
//! instrument it was made for, refactor F1); the rest are left out and
//! named ([`Carried::left`]).
//! A copied lock takes the place of a lock already in its place, as a
//! clip keeps one lock in each. Pad hits go only into a clip of pad hits,
//! and notes into a clip of notes ([`Holds`]): a kit's pad 3 is no MIDI
//! note 3.
//!
//! **Scales** (piano roll plan 2B.1): with the clip's scale on, the Notes
//! view moves notes by notes of the scale ([`scale_move`],
//! [`scale_steps`], [`shift_in_scale`]) and Fit to scale brings notes
//! onto it ([`pitch_quantize`]). Only the notes going up count (the ones
//! the roll lights), and a note off the scale stays where it is until it
//! is moved: its first step goes to the scale note beside it.
//!
//! **Chords** (piano roll plan 2B.2): a chord drawn goes in whole
//! ([`add_all`]). Strum, Invert and Spread work on the picked notes'
//! chords ([`chords_of`]: notes that sound together), so a run of chords
//! changes chord by chord: [`strum`] starts each note of a chord a little
//! after the one under it (or over it), [`invert`] takes the lowest note
//! up an octave (or the highest down), and [`spread`] opens a chord by
//! taking the note over its lowest up an octave.
//!
//! **Tools** (piano roll plan 2B.3): [`chop`] cuts each note into equal
//! pieces, each keeping its part of the pressure as a run of its own
//! (unlike [`split`], whose later parts have none), and [`legato`] runs
//! each note on toward the next start by a share of the gap.

use crate::inst::Inst;
use crate::steps::{moves_edit, Edit, Grid, DEFAULT_SIZE, SIZES};
use crate::{
    touch_room, Clip, Holds, Lock, Note, Scale, Touch, TouchPoint, MAX_CLIP_TICKS, MAX_LOCKS,
    MAX_NOTES, MIN_CLIP_TICKS,
};
use mpfree_notefx::TrackFx;
use mpfree_param::{OnTrack, RelAddr};
use mpfree_transport::time::{Tick, PPQ};
use std::collections::{HashMap, HashSet};

/// The Timing Correct divisions, in ticks (960 to a beat), with their
/// names: a bar of 4/4 down to a 1/64 note, triplets too.
pub const DIVISIONS: [(Tick, &str); 12] = [
    (3840, "1 bar"),
    (1920, "1/2"),
    (960, "1/4"),
    (640, "1/4T"),
    (480, "1/8"),
    (320, "1/8T"),
    (240, "1/16"),
    (160, "1/16T"),
    (120, "1/32"),
    (80, "1/32T"),
    (60, "1/64"),
    (40, "1/64T"),
];

/// How far a nudge moves with no snap, in ticks (the MPC's "Don't Snap").
pub const FREE_NUDGE: Tick = 4;

/// What Timing Correct changes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TcKind {
    /// Each note's start (its length stays).
    #[default]
    Start,
    /// Each note's end (its start stays).
    End,
    /// Each note's length, to a whole number of divisions.
    Length,
    /// Each note held until the next one starts.
    Legato,
}

/// The Timing Correct settings (see the module notes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tc {
    pub kind: TcKind,
    /// Ticks; one of [`DIVISIONS`] on the screen, any above 0 here.
    pub division: Tick,
    /// 50 (straight) to 75 percent.
    pub swing: u8,
    /// Ticks every chosen note moves after the correction.
    pub shift: Tick,
    /// 0 to 100 percent of half a division: only notes that near a line.
    pub window: u8,
    /// 0 to 100 percent of the way to the line.
    pub strength: u8,
}

impl Default for Tc {
    fn default() -> Tc {
        Tc {
            kind: TcKind::Start,
            division: PPQ as Tick / 4,
            swing: 50,
            shift: 0,
            window: 100,
            strength: 100,
        }
    }
}

/// Which notes an edit takes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Which {
    All,
    /// Indexes into the clip's notes (out-of-range ones are skipped).
    Selected(Vec<usize>),
    /// Notes starting in `from..to` with a note number in `low..=high`.
    Range {
        from: Tick,
        to: Tick,
        low: u8,
        high: u8,
    },
}

impl Which {
    /// The indexes it picks in `clip`, in order, each once.
    pub fn pick(&self, clip: &Clip) -> Vec<usize> {
        let mut v: Vec<usize> = match self {
            Which::All => (0..clip.notes.len()).collect(),
            Which::Selected(s) => s
                .iter()
                .copied()
                .filter(|&i| i < clip.notes.len())
                .collect(),
            Which::Range {
                from,
                to,
                low,
                high,
            } => (0..clip.notes.len())
                .filter(|&i| {
                    let n = &clip.notes[i];
                    (*from..*to).contains(&n.tick) && (*low..=*high).contains(&n.note)
                })
                .collect(),
        };
        v.sort_unstable();
        v.dedup();
        v
    }
}

impl Tc {
    /// The settings in range: the division at least 1 tick, swing 50 to
    /// 75, window and strength at most 100.
    pub fn checked(self) -> Tc {
        Tc {
            division: self.division.max(1),
            swing: self.swing.clamp(50, 75),
            window: self.window.min(100),
            strength: self.strength.min(100),
            ..self
        }
    }

    /// Line `k` of the division, swung.
    fn line(self, k: i64) -> f64 {
        let d = self.division as f64;
        let swing = if k.rem_euclid(2) == 1 {
            (2.0 * f64::from(self.swing) / 100.0 - 1.0) * d
        } else {
            0.0
        };
        k as f64 * d + swing
    }

    /// The nearest (swung) line to `t`.
    fn nearest(self, t: f64) -> f64 {
        let k = (t / self.division as f64).floor() as i64;
        [k - 1, k, k + 1, k + 2]
            .into_iter()
            .map(|k| self.line(k))
            .min_by(|a, b| (a - t).abs().total_cmp(&(b - t).abs()))
            .expect("four lines")
    }

    /// Where a tick goes: toward the nearest line when it is inside the
    /// window, by the strength, then shifted. Not wrapped.
    pub fn correct(self, tick: Tick) -> Tick {
        let tc = self.checked();
        let t = tick as f64;
        let line = tc.nearest(t);
        let near = (line - t).abs() <= tc.division as f64 / 2.0 * f64::from(tc.window) / 100.0;
        let moved = if near {
            t + (line - t) * f64::from(tc.strength) / 100.0
        } else {
            t
        };
        moved.round() as Tick + tc.shift
    }
}

/// Timing Correct `which` notes of `clip` with `tc` (see the module
/// notes). Notes it would not change are left out of the edit.
pub fn timing_correct(clip: &Clip, tc: Tc, which: &Which) -> Edit {
    let tc = tc.checked();
    let picked = which.pick(clip);
    let mut out = Vec::with_capacity(picked.len());
    match tc.kind {
        TcKind::Start => {
            for &i in &picked {
                let n = clip.notes[i];
                out.push((
                    n,
                    Note {
                        tick: wrap(tc.correct(n.tick), clip.length),
                        ..n
                    },
                ));
            }
        }
        TcKind::End => {
            for &i in &picked {
                let n = clip.notes[i];
                let end = tc.correct(n.tick + n.len);
                out.push((
                    n,
                    Note {
                        len: fit_len(end - n.tick),
                        ..n
                    },
                ));
            }
        }
        TcKind::Length => {
            let d = tc.division as f64;
            for &i in &picked {
                let n = clip.notes[i];
                let whole = ((n.len as f64 / d).round().max(1.0) * d) as Tick;
                let len = n.len as f64 + (whole - n.len) as f64 * f64::from(tc.strength) / 100.0;
                out.push((
                    n,
                    Note {
                        len: fit_len(len.round() as Tick),
                        ..n
                    },
                ));
            }
        }
        TcKind::Legato => {
            // Starts in time order; each note lasts until the next start
            // after its own (notes starting together end together); the
            // last ones keep their length.
            let mut starts: Vec<Tick> = picked.iter().map(|&i| clip.notes[i].tick).collect();
            starts.sort_unstable();
            starts.dedup();
            for &i in &picked {
                let n = clip.notes[i];
                let next = starts.iter().find(|&&s| s > n.tick);
                if let Some(&s) = next {
                    out.push((
                        n,
                        Note {
                            len: fit_len(s - n.tick),
                            ..n
                        },
                    ));
                }
            }
        }
    }
    replace(clip, out)
}

fn wrap(tick: Tick, length: Tick) -> Tick {
    tick.rem_euclid(length.max(1))
}

fn fit_len(len: Tick) -> Tick {
    len.clamp(1, MAX_CLIP_TICKS)
}

/// The edit that turns each `(old, new)` into `new`, leaving out the ones
/// that stay the same; each note's own locks go with it. A trigger group
/// moved whole stays one, and notes moved without the rest of theirs
/// decide alone ([`crate::groups::settle_moved`]); refused (no change)
/// when a trigger would need a number its new tick has none left of.
fn replace(clip: &Clip, pairs: Vec<(Note, Note)>) -> Edit {
    let (old, mut new): (Vec<Note>, Vec<Note>) = pairs.into_iter().filter(|(a, b)| a != b).unzip();
    if crate::groups::settle_moved(clip, &old, &old, &mut new).is_err() {
        return Edit::default();
    }
    let mut e = Edit::default();
    let mut used = vec![false; clip.locks.len()];
    for (old, new) in old.into_iter().zip(new) {
        if old == new {
            continue;
        }
        e.erase.push(old);
        e.add.push(new);
        if (old.tick, old.note) == (new.tick, new.note) {
            continue;
        }
        for (k, l) in clip.locks.iter().enumerate() {
            if !used[k] && own(l, &old) {
                used[k] = true;
                e.erase_locks.push(*l);
                e.add_locks.push(Lock {
                    tick: new.tick,
                    sound: Some(new.note),
                    ..*l
                });
            }
        }
    }
    e
}

/// A lock on `n`'s own sound and tick.
fn own(l: &Lock, n: &Note) -> bool {
    l.tick == n.tick && l.sound == Some(n.note)
}

/// How a move snaps (the Grid View's snap mode, or Don't Snap).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Snap {
    /// Onto the division's lines.
    Absolute(Tick),
    /// By whole divisions, keeping each note's offset from its line.
    Relative(Tick),
    /// By [`FREE_NUDGE`] ticks a step.
    Off,
}

/// Move the `sel` notes by `steps` (positive later), each snapping as
/// `snap` says; starts wrap round the clip.
pub fn nudge(clip: &Clip, sel: &[usize], steps: i64, snap: Snap) -> Edit {
    map(clip, sel, |n| {
        let t = match snap {
            Snap::Absolute(d) => {
                let d = d.max(1);
                let on = (n.tick as f64 / d as f64).round() as Tick * d;
                // Off a line, the first step lands on the line that way.
                let first = if on == n.tick || steps == 0 {
                    on
                } else if steps > 0 {
                    n.tick.div_euclid(d) * d + d
                } else {
                    n.tick.div_euclid(d) * d
                };
                if on == n.tick {
                    on + steps * d
                } else {
                    first + (steps - steps.signum()) * d
                }
            }
            Snap::Relative(d) => n.tick + steps * d.max(1),
            Snap::Off => n.tick + steps * FREE_NUDGE,
        };
        Some(Note {
            tick: wrap(t, clip.length),
            ..n
        })
    })
}

/// Move the `sel` notes by `ticks` and `notes` together (a drag); each
/// start wraps, and a note number past 0 or 127 refuses the whole move.
pub fn shift(clip: &Clip, sel: &[usize], ticks: Tick, notes: i32) -> Edit {
    let ok = sel
        .iter()
        .filter_map(|&i| clip.notes.get(i))
        .all(|n| (0..=127).contains(&(i32::from(n.note) + notes)));
    if !ok {
        return Edit::default();
    }
    map(clip, sel, |n| {
        Some(Note {
            tick: wrap(n.tick + ticks, clip.length),
            note: (i32::from(n.note) + notes) as u8,
            ..n
        })
    })
}

/// Move the `sel` notes' starts by `ticks`, their ends staying (Edit
/// Start); a note keeps at least 1 tick.
pub fn edit_start(clip: &Clip, sel: &[usize], ticks: Tick) -> Edit {
    map(clip, sel, |n| {
        let end = n.tick + n.len;
        let t = (n.tick + ticks).min(end - 1).max(0);
        (t < clip.length).then(|| Note {
            tick: t,
            len: end - t,
            ..n
        })
    })
}

/// Move the `sel` notes' ends by `ticks` (Edit End).
pub fn edit_end(clip: &Clip, sel: &[usize], ticks: Tick) -> Edit {
    map(clip, sel, |n| {
        Some(Note {
            len: fit_len(n.len + ticks),
            ..n
        })
    })
}

/// Transpose the `sel` notes, each kept within 0 to 127.
pub fn transpose(clip: &Clip, sel: &[usize], semis: i32) -> Edit {
    map(clip, sel, |n| {
        Some(Note {
            note: (i32::from(n.note) + semis).clamp(0, 127) as u8,
            ..n
        })
    })
}

/// Change the `sel` notes' velocity by `by`, each kept within 1 to 127.
pub fn velocity(clip: &Clip, sel: &[usize], by: i32) -> Edit {
    map(clip, sel, |n| {
        Some(Note {
            velocity: (i32::from(n.velocity) + by).clamp(1, 127) as u8,
            ..n
        })
    })
}

/// The lowest and highest of `f` over the `sel` notes, None for none.
fn span_of(clip: &Clip, sel: &[usize], f: impl Fn(&Note) -> i32) -> Option<(i32, i32)> {
    picked(clip, sel).fold(None, |s, (_, n)| {
        let v = f(&n);
        Some(s.map_or((v, v), |(lo, hi): (i32, i32)| (lo.min(v), hi.max(v))))
    })
}

/// Change the `sel` notes' velocity by `by`, each keeping its distance
/// from the others (the note inspector's Q4, piano roll plan 1C.1): the
/// loudest stops at 127 and the softest at 1, and the rest stop with
/// them. Empty when they can't move that way.
pub fn velocity_kept(clip: &Clip, sel: &[usize], by: i32) -> Edit {
    let Some((lo, hi)) = span_of(clip, sel, |n| i32::from(n.velocity)) else {
        return Edit::default();
    };
    let by = by.clamp(1 - lo, 127 - hi);
    if by == 0 {
        return Edit::default();
    }
    velocity(clip, sel, by)
}

/// Widen (`steps` above 0) or narrow the gap between the `sel` notes'
/// softest and loudest velocities, about an eighth of it a step and at
/// least 1 at each end, the notes between keeping their places in the gap
/// (SHIFT + Q4, piano roll plan 1C.1). The ends stay within 1 to 127, and
/// narrowing stops before they meet, so the order is never lost. Empty
/// when nothing moves (one velocity among them, or at the limits).
pub fn velocity_spread(clip: &Clip, sel: &[usize], steps: i32) -> Edit {
    let Some((lo, hi)) = span_of(clip, sel, |n| i32::from(n.velocity)) else {
        return Edit::default();
    };
    if hi == lo {
        return Edit::default();
    }
    let (mut a, mut b) = (lo, hi);
    for _ in 0..steps.unsigned_abs() {
        let s = ((b - a + 4) / 8).max(1);
        if steps > 0 {
            (a, b) = ((a - s).max(1), (b + s).min(127));
        } else {
            let room = (b - a - 1) / 2;
            if room == 0 {
                break;
            }
            (a, b) = (a + s.min(room), b - s.min(room));
        }
    }
    if (a, b) == (lo, hi) {
        return Edit::default();
    }
    let (from, to) = (hi - lo, b - a);
    map(clip, sel, |n| {
        let v = i32::from(n.velocity) - lo;
        let v = a + (2 * v * to + from) / (2 * from);
        (v != i32::from(n.velocity)).then(|| Note {
            velocity: v.clamp(1, 127) as u8,
            ..n
        })
    })
}

/// Set the `sel` notes' velocity (the velocity lane).
pub fn set_velocity(clip: &Clip, sel: &[usize], v: u8) -> Edit {
    map(clip, sel, |n| {
        Some(Note {
            velocity: v.clamp(1, 127),
            ..n
        })
    })
}

/// What a lane of the piano roll sets, a value on each note (piano roll
/// plan 2A.1): its velocity, its chance, how many times it repeats (its
/// ratchet) or its repeats' velocity ramp.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Field {
    #[default]
    Velocity,
    Chance,
    Repeats,
    Ramp,
}

impl Field {
    /// The lanes in the order their chip steps through them; the ramp is
    /// the Repeats lane with SHIFT.
    pub const LANES: [Field; 3] = [Field::Velocity, Field::Chance, Field::Repeats];

    /// The lowest and highest value it takes: velocity 1 to 127, chance 1
    /// to 100 %, repeats 1 to [`MAX_RATCHET`](crate::MAX_RATCHET), a ramp
    /// -[`MAX_RAMP`](crate::MAX_RAMP) to [`MAX_RAMP`](crate::MAX_RAMP).
    pub fn range(self) -> (i32, i32) {
        match self {
            Field::Velocity => (1, 127),
            Field::Chance => (1, 100),
            Field::Repeats => (1, i32::from(crate::MAX_RATCHET)),
            Field::Ramp => (-i32::from(crate::MAX_RAMP), i32::from(crate::MAX_RAMP)),
        }
    }

    /// Note `n`'s value of it.
    pub fn of(self, n: &Note) -> i32 {
        match self {
            Field::Velocity => i32::from(n.velocity),
            Field::Chance => i32::from(n.chance),
            Field::Repeats => i32::from(n.ratchet),
            Field::Ramp => i32::from(n.ramp),
        }
    }

    /// `n` with its value of it set to `v`, kept within [`Field::range`].
    /// None where that changes nothing, and for a ramp on a note that
    /// doesn't repeat (one hit has nothing to ramp).
    pub fn set(self, n: Note, v: i32) -> Option<Note> {
        let (lo, hi) = self.range();
        let v = v.clamp(lo, hi);
        if v == self.of(&n) {
            return None;
        }
        Some(match self {
            Field::Velocity => Note {
                velocity: v as u8,
                ..n
            },
            Field::Chance => Note {
                chance: v as u8,
                ..n
            },
            Field::Repeats => Note {
                ratchet: v as u8,
                ..n
            },
            Field::Ramp if n.ratchet <= 1 => return None,
            Field::Ramp => Note { ramp: v as i8, ..n },
        })
    }

    /// As the lane's chip names it.
    pub fn name(self) -> &'static str {
        match self {
            Field::Velocity => "Velocity",
            Field::Chance => "Chance",
            Field::Repeats => "Repeats",
            Field::Ramp => "Ramp",
        }
    }
}

/// Set each note `set` names (an index into the clip's notes) to its value
/// of `field` (a stroke along one of the piano roll's lanes, plan 2A.1),
/// kept within the field's range. A note named twice takes its last
/// value; a note past the clip's, or one the value leaves as it was, is
/// left out. A chance set on a note of a trigger group (a chord that
/// rolls as one, [`crate::groups`]) is its whole trigger's, the last one
/// the stroke gave any of its notes. One edit, however many notes.
pub fn set_values(clip: &Clip, field: Field, set: &[(usize, i32)]) -> Edit {
    let mut last: Vec<(usize, i32)> = Vec::with_capacity(set.len());
    for &(i, v) in set.iter().rev() {
        if i < clip.notes.len() && !last.iter().any(|&(j, _)| j == i) {
            last.push((i, v));
        }
    }
    if matches!(field, Field::Chance) && clip.holds == Holds::Notes {
        // A trigger group's notes share one chance (`crate::groups`): the
        // value its note was given last in the stroke, for every note of
        // it, so a stroke across a chord never pulls it apart.
        let mut shared: HashMap<(Tick, u8), i32> = HashMap::new();
        for &(i, v) in set {
            if let Some(n) = clip.notes.get(i).filter(|n| n.group != 0) {
                shared.insert((n.tick, n.group), v);
            }
        }
        if !shared.is_empty() {
            last.retain(|&(i, _)| {
                let n = &clip.notes[i];
                n.group == 0 || !shared.contains_key(&(n.tick, n.group))
            });
            for (i, n) in clip.notes.iter().enumerate() {
                if let Some(&v) = shared.get(&(n.tick, n.group)).filter(|_| n.group != 0) {
                    last.push((i, v));
                }
            }
        }
    }
    last.sort_unstable_by_key(|&(i, _)| i);
    replace(
        clip,
        last.into_iter()
            .filter_map(|(i, v)| {
                let n = clip.notes[i];
                field.set(n, v).map(|m| (n, m))
            })
            .collect(),
    )
}

/// Change the `sel` notes' chance by `by` percent, each kept within 1 to
/// 100.
pub fn chance(clip: &Clip, sel: &[usize], by: i32) -> Edit {
    map(clip, sel, |n| {
        Some(Note {
            chance: (i32::from(n.chance) + by).clamp(1, 100) as u8,
            ..n
        })
    })
}

/// Step the `sel` notes' trig conditions `d` places along
/// [`Cond::all`](crate::Cond::all), each from its own.
pub fn condition(clip: &Clip, sel: &[usize], d: i32) -> Edit {
    map(clip, sel, |n| {
        Some(Note {
            cond: n.cond.step(d),
            ..n
        })
    })
}

/// Change the `sel` notes' ratchet by `by` hits, each kept within 1 to
/// [`MAX_RATCHET`](crate::MAX_RATCHET).
pub fn ratchet(clip: &Clip, sel: &[usize], by: i32) -> Edit {
    map(clip, sel, |n| {
        Some(Note {
            ratchet: (i32::from(n.ratchet) + by).clamp(1, i32::from(crate::MAX_RATCHET)) as u8,
            ..n
        })
    })
}

/// Take the `sel` notes out, with their own locks.
pub fn delete(clip: &Clip, sel: &[usize]) -> Edit {
    let erase: Vec<Note> = picked(clip, sel).map(|(_, n)| n).collect();
    let erase_locks = clip
        .locks
        .iter()
        .filter(|l| erase.iter().any(|n| own(l, n)))
        .copied()
        .collect();
    Edit {
        erase,
        erase_locks,
        ..Edit::default()
    }
}

/// A new note (the pencil); refused (empty) when it does not fit.
pub fn add(clip: &Clip, note: Note) -> Edit {
    let mut c = Clip::new("", clip.length);
    if c.add_note(note).is_err() || clip.notes.len() >= MAX_NOTES {
        return Edit::default();
    }
    let mut add = vec![note];
    if crate::groups::settle_new(clip, &[], &mut add, false).is_err() {
        return Edit::default();
    }
    Edit {
        add,
        ..Edit::default()
    }
}

/// Copies of the `sel` notes right after them: moved on by their span
/// (first start to last end) rounded up to whole `division`s. Copies that
/// would start past the clip's end wrap.
pub fn duplicate(clip: &Clip, sel: &[usize], division: Tick) -> Edit {
    let notes = crate::groups::copy_out(clip, sel);
    let (Some(first), Some(end)) = (
        notes.iter().map(|n| n.tick).min(),
        notes.iter().map(|n| n.tick + n.len).max(),
    ) else {
        return Edit::default();
    };
    let d = division.max(1);
    let by = ((end - first) + d - 1).div_euclid(d) * d;
    let mut e = paste(clip, &notes, first + by, first);
    if !e.is_empty() {
        for l in clip
            .locks
            .iter()
            .filter(|l| notes.iter().any(|n| own(l, n)))
        {
            e.add_locks.push(Lock {
                tick: wrap(l.tick + by, clip.length),
                ..*l
            });
        }
    }
    e
}

/// Put `notes` (a copy taken from a clip, `from` its first start, by
/// [`crate::groups::copy_out`]) in at `at`: each keeps its place from
/// `from`; starts wrap round the clip. A trigger group in the copy is a
/// new trigger, with a free number where its tick has its number already
/// (refused, no change, when none is left).
pub fn paste(clip: &Clip, notes: &[Note], at: Tick, from: Tick) -> Edit {
    if clip.notes.len() + notes.len() > MAX_NOTES {
        return Edit::default();
    }
    let mut add: Vec<Note> = notes
        .iter()
        .map(|n| Note {
            tick: wrap(n.tick - from + at, clip.length),
            ..*n
        })
        .collect();
    if crate::groups::settle_new(clip, &[], &mut add, false).is_err() {
        return Edit::default();
    }
    Edit {
        add,
        ..Edit::default()
    }
}

/// Notes copied out of a clip, to go into another or the same one
/// ([`copy`], [`carry`]; piano roll plan 1B.4). Kept apart from the
/// clip, so the copy holds whatever happens to the clip after.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Copied {
    /// The notes in the clip's order, each tick from the first note's
    /// start, each note's pressure naming its run in `touch`.
    pub notes: Vec<Note>,
    /// The notes' pressure runs, one after another.
    pub touch: Vec<TouchPoint>,
    /// The locks the notes own, their ticks from the first note's start.
    pub locks: Vec<Lock>,
    /// Whether the notes are pad hits or MIDI notes, as their clip held.
    pub holds: Holds,
}

/// A copy of the `sel` notes of `clip`, with their pressure and the
/// locks they own (empty for none).
pub fn copy(clip: &Clip, sel: &[usize]) -> Copied {
    // A trigger group copied whole keeps its number; a part of one
    // decides alone ([`crate::groups::copy_out`]).
    let notes = crate::groups::copy_out(clip, sel);
    let Some(first) = notes.iter().map(|n| n.tick).min() else {
        return Copied::default();
    };
    let mut c = Copied {
        holds: clip.holds,
        ..Copied::default()
    };
    // A run several notes share (a chord's) is copied once.
    let mut runs: HashMap<Touch, Touch> = HashMap::new();
    for n in &notes {
        let points = clip.touch_of(n);
        let touch = if points.is_empty() {
            Touch::NONE
        } else {
            *runs.entry(n.touch).or_insert_with(|| {
                let t = Touch {
                    first: c.touch.len() as u32,
                    len: points.len() as u32,
                };
                c.touch.extend_from_slice(points);
                t
            })
        };
        c.notes.push(Note {
            tick: n.tick - first,
            touch,
            ..*n
        });
    }
    // The locks they own, found in one pass (a full clip has 8,192 of
    // each).
    let owners: HashSet<(Tick, u8)> = notes.iter().map(|n| (n.tick, n.note)).collect();
    c.locks = clip
        .locks
        .iter()
        .filter(|l| l.sound.is_some_and(|s| owners.contains(&(l.tick, s))))
        .map(|l| Lock {
            tick: l.tick - first,
            ..*l
        })
        .collect();
    c
}

/// Why [`carry`] put nothing in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CarryError {
    /// Nothing was copied.
    Nothing,
    /// Pad hits into a clip of notes, or notes into a clip of pad hits.
    Holds,
    /// The clip has no room for that many more notes ([`MAX_NOTES`]).
    Notes,
    /// Nor for their locks ([`MAX_LOCKS`]).
    Locks,
    /// Nor for their pressure ([`crate::MAX_TOUCH_POINTS`]).
    Pressure,
    /// A trigger group in the copy needs a free number on the tick it
    /// lands on, and none is left there ([`crate::groups::GroupsFull`]).
    Groups,
}

/// What [`carry`] makes: the edit, one take, and the locks it left out
/// because the clip's track may not have their setting.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Carried {
    pub edit: Edit,
    /// Where they would have gone in the clip.
    pub left: Vec<Lock>,
}

/// Put `copied` into `clip` with its first note at `at`, each note keeping
/// its place from it (starts wrap round the clip). The pressure comes in
/// as new runs at the end of the clip's pressure (made now, so the edit
/// is for the clip as it is); a lock comes when `keep` says the clip's
/// track has its setting ([`same_setting`]), in place of a lock already
/// in its place, and is left out otherwise, as a pad's own lock is in a
/// clip of notes, where it would not play. A trigger group copied whole
/// is one trigger where it lands, never joined to one there. Refused,
/// with nothing put in, when the clip holds the other kind ([`Holds`]) or
/// has no room.
pub fn carry(
    clip: &Clip,
    copied: &Copied,
    at: Tick,
    keep: impl Fn(&Lock) -> bool,
) -> Result<Carried, CarryError> {
    if copied.notes.is_empty() {
        return Err(CarryError::Nothing);
    }
    if copied.holds != clip.holds {
        return Err(CarryError::Holds);
    }
    if clip.notes.len() + copied.notes.len() > MAX_NOTES {
        return Err(CarryError::Notes);
    }
    let touch_at = clip.touch.len() as u32;
    let mut touch: Vec<TouchPoint> = Vec::new();
    let mut runs: HashMap<Touch, Touch> = HashMap::new();
    let mut add = Vec::with_capacity(copied.notes.len());
    for n in &copied.notes {
        let points = copied.touch.get(n.touch.range()).unwrap_or(&[]);
        let run = if points.is_empty() {
            Touch::NONE
        } else {
            *runs.entry(n.touch).or_insert_with(|| {
                let t = Touch {
                    first: touch_at + touch.len() as u32,
                    len: points.len() as u32,
                };
                touch.extend_from_slice(points);
                t
            })
        };
        add.push(Note {
            tick: wrap(at + n.tick, clip.length),
            touch: run,
            ..*n
        });
    }
    if !touch_room(clip.touch.len(), &touch) {
        return Err(CarryError::Pressure);
    }
    // A trigger group in the copy is a new trigger where it lands, with a
    // free number where its tick has its number already: a paste never
    // joins a trigger that is there.
    crate::groups::settle_new(clip, &[], &mut add, false).map_err(|_| CarryError::Groups)?;
    // One lock in each place: a copy longer than the clip wraps onto
    // itself, and the last lock in a place wins, as it would recorded.
    let mut places = HashSet::new();
    let mut add_locks = Vec::new();
    let mut left = Vec::new();
    for l in copied.locks.iter().rev() {
        let l = Lock {
            tick: wrap(at + l.tick, clip.length),
            ..*l
        };
        let pad_in_notes = clip.holds == Holds::Notes && matches!(l.addr.on, OnTrack::Pad(_));
        if pad_in_notes || !keep(&l) {
            left.push(l);
        } else if places.insert((l.tick, l.sound, l.addr)) {
            add_locks.push(l);
        }
    }
    add_locks.reverse();
    left.reverse();
    let erase_locks: Vec<Lock> = clip
        .locks
        .iter()
        .filter(|l| places.contains(&(l.tick, l.sound, l.addr)))
        .copied()
        .collect();
    if clip.locks.len() - erase_locks.len() + add_locks.len() > MAX_LOCKS {
        return Err(CarryError::Locks);
    }
    Ok(Carried {
        edit: Edit {
            add,
            erase_locks,
            add_locks,
            touch,
            touch_at,
            ..Edit::default()
        },
        left,
    })
}

/// A track as a copied lock sees it (plan 1B.4): its MIDI effects, and
/// the instrument its instrument locks are for (refactor F1). For the
/// copy, what the copied clip's instrument locks were made for
/// ([`Clip::made_for`], else what its track played then); for the track
/// a paste goes to, the instrument it plays now ([`crate::Track::plays`]).
/// None where it isn't known.
#[derive(Clone, Copy, Debug)]
pub struct TrackSettings<'a> {
    pub fx: &'a TrackFx,
    pub inst: Option<&'a Inst>,
}

/// Whether a lock on `addr`, copied from a clip of track `from`, sets the
/// same thing on track `to` (`same_track` when it is the same track).
/// On the same track it does, but for an instrument lock made for another
/// instrument than the track plays now. On another, a lock on what every
/// track has alike does: its mixer strip's own settings, its sequencer
/// settings and its arp (fixed id tables); one on a MIDI effect when that
/// track has an effect of the same kind under the same id; and one on the
/// instrument when that track plays the same kind of instrument the lock
/// was made for and the control has the name it was made with (refactor
/// F1; a name not known on either side is taken as the same, as
/// [`Clip::param_plays`] does). A lock on a kit pad, an insert or MIDI
/// out does only on the same track: another track may play another kit,
/// its strip holds its own inserts, and its MIDI out may go to another
/// device's port and channel, where the same controller turns something
/// else.
pub fn same_setting(
    same_track: bool,
    from: TrackSettings,
    to: TrackSettings,
    addr: RelAddr,
) -> bool {
    match addr.on {
        OnTrack::Inst => match (from.inst, to.inst) {
            (Some(made), Some(now)) => {
                made.node == now.node
                    && match (made.param_name(addr.param), now.param_name(addr.param)) {
                        (Some(a), Some(b)) => a == b,
                        _ => true,
                    }
            }
            // An instrument not known (an older clip, a track the app
            // hasn't told of): only on its own track, as before.
            _ => same_track,
        },
        _ if same_track => true,
        OnTrack::Strip | OnTrack::Seq | OnTrack::Arp => true,
        OnTrack::NoteFx(id) => {
            let kind = |fx: &TrackFx| {
                fx.place_of(id)
                    .and_then(|p| fx.slots[p])
                    .map(|s| std::mem::discriminant(&s.fx))
            };
            kind(from.fx).is_some() && kind(from.fx) == kind(to.fx)
        }
        OnTrack::Pad(_) | OnTrack::Insert(_) | OnTrack::Midi => false,
    }
}

/// Twice the clip's length, its notes, locks and moves played twice
/// (Double Length with Events); refused (empty) past the longest clip.
pub fn double_length(clip: &Clip) -> Edit {
    let to = clip.length * 2;
    if to > MAX_CLIP_TICKS || clip.notes.len() * 2 > MAX_NOTES {
        return Edit::default();
    }
    Edit {
        length: Some((clip.length, to)),
        add: clip
            .notes
            .iter()
            .map(|n| Note {
                tick: n.tick + clip.length,
                ..*n
            })
            .collect(),
        add_locks: clip
            .locks
            .iter()
            .map(|l| Lock {
                tick: l.tick + clip.length,
                ..*l
            })
            .collect(),
        moves: moves_edit(clip, |m| {
            m.copy_range(0, clip.length as u32, clip.length as u32, to as u32)
        }),
        ..Edit::default()
    }
}

/// Half-speed events: every start and length twice as far, and the clip
/// twice as long so nothing falls off its end; refused past the longest
/// clip or for a clip with FX moves.
pub fn half_speed(clip: &Clip) -> Edit {
    let to = clip.length * 2;
    if to > MAX_CLIP_TICKS || !clip.moves.is_empty() {
        return Edit::default();
    }
    let Some(mut e) = respace(
        clip,
        |t| t * 2,
        |l| fit_len(l * 2),
        |at| at.saturating_mul(2),
    ) else {
        return Edit::default();
    };
    e.length = Some((clip.length, to));
    e
}

/// Double-speed events: every start and length halved, and the clip half
/// as long; refused below the shortest clip or for a clip with FX moves.
pub fn double_speed(clip: &Clip) -> Edit {
    let to = clip.length / 2;
    if to < MIN_CLIP_TICKS || !clip.moves.is_empty() {
        return Edit::default();
    }
    let Some(mut e) = respace(clip, |t| t / 2, |l| (l / 2).max(1), |at| at / 2) else {
        return Edit::default();
    };
    e.length = Some((clip.length, to));
    e
}

/// Every note and lock moved by `at` and `len`, and each note's pressure
/// run again with its times moved by `touch` (None, refused, when the
/// clip has no room for the new runs).
fn respace(
    clip: &Clip,
    at: impl Fn(Tick) -> Tick,
    len: impl Fn(Tick) -> Tick,
    touch: impl Fn(u32) -> u32,
) -> Option<Edit> {
    let touch_at = clip.touch.len() as u32;
    let mut runs: Vec<TouchPoint> = Vec::new();
    let mut add = Vec::with_capacity(clip.notes.len());
    for n in &clip.notes {
        let mut new = Note {
            tick: at(n.tick),
            len: len(n.len),
            ..*n
        };
        let points = clip.touch_of(n);
        if !points.is_empty() {
            new.touch = Touch {
                first: touch_at + runs.len() as u32,
                len: points.len() as u32,
            };
            runs.extend(points.iter().map(|p| TouchPoint {
                at: touch(p.at),
                ..*p
            }));
        }
        add.push(new);
    }
    if !touch_room(clip.touch.len(), &runs) {
        return None;
    }
    // Two triggers that land on one tick stay two.
    crate::groups::settle_moved(clip, &clip.notes, &clip.notes, &mut add).ok()?;
    Some(Edit {
        erase: clip.notes.clone(),
        add,
        touch: runs,
        touch_at,
        erase_locks: clip.locks.clone(),
        add_locks: clip
            .locks
            .iter()
            .map(|l| Lock {
                tick: at(l.tick),
                ..*l
            })
            .collect(),
        ..Edit::default()
    })
}

/// Split each `sel` note into `parts` equal notes (Split Note Events);
/// refused (empty) for fewer than 2 parts, notes shorter than the parts,
/// or too many notes.
pub fn split(clip: &Clip, sel: &[usize], parts: u32) -> Edit {
    let parts = Tick::from(parts);
    let notes: Vec<Note> = picked(clip, sel).map(|(_, n)| n).collect();
    if parts < 2
        || notes.iter().any(|n| n.len < parts)
        || clip.notes.len() + notes.len() * (parts as usize - 1) > MAX_NOTES
    {
        return Edit::default();
    }
    let mut e = Edit::default();
    // Each part `k` of the notes, made from them: a trigger split whole
    // into equal parts is a trigger at each part.
    let mut rows: Vec<Vec<Note>> = vec![Vec::with_capacity(notes.len()); parts as usize];
    for &n in &notes {
        e.erase.push(n);
        let each = n.len / parts;
        for (k, row) in rows.iter_mut().enumerate() {
            let k = k as Tick;
            let len = if k == parts - 1 {
                n.len - each * k
            } else {
                each
            };
            row.push(Note {
                tick: wrap(n.tick + each * k, clip.length),
                len,
                // The first part keeps the pressure (what falls past its
                // end does not play); the rest would replay its start.
                touch: if k == 0 { n.touch } else { Touch::NONE },
                ..n
            });
        }
    }
    let mut settled = Vec::with_capacity(notes.len() * parts as usize);
    for (k, mut row) in rows.into_iter().enumerate() {
        let ok = if k == 0 {
            crate::groups::settle_moved(clip, &e.erase, &notes, &mut row)
        } else {
            crate::groups::settle_copied_beside(clip, &e.erase, &settled, &notes, &mut row)
        };
        if ok.is_err() {
            return Edit::default();
        }
        settled.extend(row);
    }
    // In the order they were made: each note's parts together.
    let n = notes.len();
    e.add = (0..n)
        .flat_map(|i| (0..parts as usize).map(move |k| (k, i)))
        .map(|(k, i)| settled[k * n + i])
        .collect();
    e
}

/// Humanize settings (the MPC's Humanize window): each is off at 0.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Humanize {
    /// Up to this many ticks either way.
    pub time: Tick,
    /// -100 (ahead of the beat) to 100 (behind): leans the time that way.
    pub eagerness: i8,
    /// Up to this percent of each length, either way.
    pub length: u8,
    /// Up to this percent of each velocity, either way.
    pub velocity: u8,
}

/// Humanize the `sel` notes, the same way for the same `seed`.
pub fn humanize(clip: &Clip, sel: &[usize], h: Humanize, seed: u64) -> Edit {
    let mut x = seed ^ 0x9E37_79B9_7F4A_7C15;
    let mut rand = move || {
        // xorshift64*: -1 to 1.
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        let r = x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11;
        r as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
    };
    let lean = f64::from(h.eagerness.clamp(-100, 100)) / 100.0;
    // A trigger group's notes move by one amount, so they stay one.
    let mut moved: Vec<((Tick, u8), Tick)> = Vec::new();
    let grouped = clip.holds == crate::Holds::Notes;
    map(clip, sel, |n| {
        let (a, b, c) = (rand(), rand(), rand());
        let mut dt = ((a + lean) / (1.0 + lean.abs()) * h.time as f64).round() as Tick;
        if grouped && n.group != 0 {
            match moved.iter().find(|(k, _)| *k == (n.tick, n.group)) {
                Some(&(_, first)) => dt = first,
                None => moved.push(((n.tick, n.group), dt)),
            }
        }
        let dl = (b * f64::from(h.length.min(100)) / 100.0 * n.len as f64).round() as Tick;
        let dv =
            (c * f64::from(h.velocity.min(100)) / 100.0 * f64::from(n.velocity)).round() as i32;
        Some(Note {
            tick: wrap(n.tick + dt, clip.length),
            len: fit_len(n.len + dl),
            velocity: (i32::from(n.velocity) + dv).clamp(1, 127) as u8,
            ..n
        })
    })
}

/// Pitch Quantize: each `sel` note to the nearest note of the scale
/// (`mask`: bit `k` set for `root + k` semitones, 12 bits) inside
/// `low..=high` (the nearer one below on a tie).
pub fn pitch_quantize(clip: &Clip, sel: &[usize], root: u8, mask: u16, low: u8, high: u8) -> Edit {
    let (low, high) = (low.min(127), high.min(127));
    let in_scale = |p: i32| mask & (1 << (p - i32::from(root)).rem_euclid(12)) != 0;
    if mask & 0xFFF == 0 || low > high || !(i32::from(low)..=i32::from(high)).any(in_scale) {
        return Edit::default();
    }
    map(clip, sel, |n| {
        let p = i32::from(n.note).clamp(i32::from(low), i32::from(high));
        let best = (0..=127)
            .filter(|&d: &i32| d >= 0)
            .flat_map(|d| [p - d, p + d])
            .find(|&q| (i32::from(low)..=i32::from(high)).contains(&q) && in_scale(q))
            .expect("a note of the scale in range");
        Some(Note {
            note: best as u8,
            ..n
        })
    })
}

/// The scale `k` with only its notes going up: what the Notes view
/// lights and moves by.
fn lit(k: Scale) -> Scale {
    Scale::new(k.root, k.mask)
}

/// `note` moved `steps` notes of the scale `k` up (down when negative), by
/// its notes going up. A note off the scale counts the scale note beside
/// it on that side as its first step. It may come out past 0 or 127; the
/// caller keeps it in.
pub fn scale_move(note: i32, steps: i32, k: Scale) -> i32 {
    let k = lit(k);
    let n = note.clamp(-1000, 1000) as i16;
    let steps = steps.clamp(-1000, 1000) as i16;
    i32::from(match steps {
        0 => n,
        s if s > 0 => k.walk(k.snap(n), s),
        s => k.walk(k.snap_up(n), s),
    })
}

/// How many notes of the scale `k` a note at `from` moves (negative:
/// down) to land on the scale note nearest `to`, the one nearer `from`
/// when two are as near: a finger dragging a note to the row `to`
/// ([`scale_move`] of the answer is that note).
pub fn scale_steps(from: i32, to: i32, k: Scale) -> i32 {
    let k = lit(k);
    if from == to {
        return 0;
    }
    let (from, to) = (from.clamp(-1000, 1000), to.clamp(-1000, 1000));
    let t = to as i16;
    let (below, above) = (i32::from(k.snap(t)), i32::from(k.snap_up(t)));
    let q = match (to - below).cmp(&(above - to)) {
        std::cmp::Ordering::Less => below,
        std::cmp::Ordering::Greater => above,
        std::cmp::Ordering::Equal if to > from => below,
        std::cmp::Ordering::Equal => above,
    };
    let count = |a: i32, b: i32| (a..=b).filter(|&n| k.contains(n as i16)).count() as i32;
    match q.cmp(&from) {
        std::cmp::Ordering::Greater => count(from + 1, q),
        std::cmp::Ordering::Less => -count(q, from - 1),
        std::cmp::Ordering::Equal => 0,
    }
}

/// Move the `sel` notes by `ticks` and `steps` notes of the scale `k`
/// together (a drag, Q3 with the clip's scale on): each by its own
/// scale's steps, so a chord keeps to the scale (C E G a step up in C
/// major is D F A). Each start wraps; a note past 0 or 127 refuses the
/// whole move.
pub fn shift_in_scale(clip: &Clip, sel: &[usize], ticks: Tick, steps: i32, k: Scale) -> Edit {
    let ok = sel
        .iter()
        .filter_map(|&i| clip.notes.get(i))
        .all(|n| (0..=127).contains(&scale_move(i32::from(n.note), steps, k)));
    if !ok {
        return Edit::default();
    }
    map(clip, sel, |n| {
        Some(Note {
            tick: wrap(n.tick + ticks, clip.length),
            note: scale_move(i32::from(n.note), steps, k) as u8,
            ..n
        })
    })
}

/// Fit to scale (piano roll plan 2B.1): the `sel` notes moved to the
/// nearest note of the scale `k` going up, the lower when two are as
/// near, as one edit ([`pitch_quantize`] over every note).
pub fn fit_to_scale(clip: &Clip, sel: &[usize], k: Scale) -> Edit {
    pitch_quantize(clip, sel, k.root, k.mask, 0, 127)
}

/// Notes put in together, as a chord drawn on the roll is (plan 2B.2):
/// none of them when one can't go in, or the clip would hold more than
/// [`MAX_NOTES`]. In a clip of notes, the notes that start on one tick
/// roll as one (a trigger group, [`crate::groups`]): they take a joined
/// number, a free one where the tick has it already, so a drawn chord
/// plays whole or not at all and never joins a chord there; none go in
/// when the tick has no number left.
pub fn add_all(clip: &Clip, notes: &[Note]) -> Edit {
    let mut c = Clip::new("", clip.length);
    if clip.notes.len() + notes.len() > MAX_NOTES || notes.iter().any(|&n| c.add_note(n).is_err()) {
        return Edit::default();
    }
    let mut add = notes.to_vec();
    let mut on_tick: HashMap<Tick, usize> = HashMap::new();
    for n in &add {
        *on_tick.entry(n.tick).or_default() += 1;
    }
    for n in add.iter_mut() {
        n.group = if on_tick[&n.tick] > 1 {
            *crate::groups::JOINED_GROUPS.start()
        } else {
            0
        };
    }
    if crate::groups::settle_new(clip, &[], &mut add, false).is_err() {
        return Edit::default();
    }
    Edit {
        add,
        ..Edit::default()
    }
}

/// Notes the pads write at the cursor (piano roll plan 1C.2): each in
/// place of any note already starting on its tick at its pitch, so a pad
/// hit again on the same line changes the note rather than doubling it.
/// The notes it takes the place of keep their locks, which sit on the
/// same step and row. In a clip of notes each note keeps the trigger
/// group it comes with (a pad's, [`crate::groups::pad_group`], so a chord
/// pad's chord plays whole or not at all, as its hit does) and joins what
/// is left of that trigger on its tick, taking its chance, condition and
/// on, as a pad tapped on its step does; a clip of pad hits has none
/// ([`crate::groups::settle_new`]). None go in when one can't, or the
/// clip would hold more than [`MAX_NOTES`].
pub fn write(clip: &Clip, notes: &[Note]) -> Edit {
    let erase: Vec<Note> = clip
        .notes
        .iter()
        .filter(|n| notes.iter().any(|w| w.tick == n.tick && w.note == n.note))
        .copied()
        .collect();
    let mut c = Clip::new("", clip.length);
    if clip.notes.len() - erase.len() + notes.len() > MAX_NOTES
        || notes.is_empty()
        || notes.iter().any(|&n| c.add_note(n).is_err())
    {
        return Edit::default();
    }
    let mut add = notes.to_vec();
    // Joining never runs out of numbers: a note takes its own number,
    // into the trigger there when the tick has it.
    if crate::groups::settle_new(clip, &erase, &mut add, true).is_err() {
        return Edit::default();
    }
    Edit {
        erase,
        add,
        ..Edit::default()
    }
}

/// The `sel` notes in chords (plan 2B.2): the notes that sound together,
/// each chord lowest first, the chords in time. Taken by their starts, a
/// note joins a chord while it starts before the chord's first note ends,
/// so a strummed chord stays one, and a chord starting as the last one
/// ends is the next.
pub fn chords_of(clip: &Clip, sel: &[usize]) -> Vec<Vec<usize>> {
    let mut by_start: Vec<(usize, Note)> = picked(clip, sel).collect();
    by_start.sort_by_key(|&(i, n)| (n.tick, n.note, i));
    let mut out: Vec<Vec<usize>> = Vec::new();
    let mut end = Tick::MIN;
    for (i, n) in by_start {
        match out.last_mut() {
            Some(chord) if n.tick < end => chord.push(i),
            _ => {
                end = n.tick + n.len;
                out.push(vec![i]);
            }
        }
    }
    for chord in &mut out {
        chord.sort_by_key(|&i| (clip.notes[i].note, clip.notes[i].tick, i));
    }
    out
}

/// How far apart a chord's notes start (plan 2B.2): the ticks from its
/// lowest note's start to its highest's, over the steps between them;
/// below 0 when the chord strums down. `chord` is lowest first, as
/// [`chords_of`] gives it.
pub fn strum_of(clip: &Clip, chord: &[usize]) -> Tick {
    match (chord.first(), chord.last()) {
        (Some(&lo), Some(&hi)) if chord.len() > 1 => {
            (clip.notes[hi].tick - clip.notes[lo].tick) / (chord.len() as Tick - 1)
        }
        _ => 0,
    }
}

/// Each chord of the `sel` notes strummed `by` ticks further apart
/// (plans 2B.2 and 2B.3; closer below 0, as far as together), lowest note
/// first or, with `down`, highest first; `down` None keeps each chord's
/// way ([`strum_of`]'s sign; together counts as up). Each note starts its
/// place in the chord times the gap after the chord's first start, its
/// end staying, so a chord strummed back to 0 is as it was; a chord asked
/// for no change is left as it is. Refused whole when a note would not
/// keep a tick of length.
pub fn strum(clip: &Clip, sel: &[usize], by: Tick, down: Option<bool>) -> Edit {
    let mut pairs = Vec::new();
    for chord in chords_of(clip, sel) {
        let n = chord.len() as Tick;
        if n < 2 {
            continue;
        }
        let base = chord.iter().map(|&i| clip.notes[i].tick).min().unwrap_or(0);
        let was = strum_of(clip, &chord);
        let apart = (was.abs() + by).max(0);
        let down = down.unwrap_or(was < 0);
        // No change asked: the chord stays as it is, evenly strummed or
        // not.
        if by == 0 && down == (was < 0) {
            continue;
        }
        for (r, &i) in chord.iter().enumerate() {
            let r = r as Tick;
            let old = clip.notes[i];
            let at = base + if down { (n - 1 - r) * apart } else { r * apart };
            let end = old.tick + old.len;
            if at >= end || at >= clip.length {
                return Edit::default();
            }
            pairs.push((
                old,
                Note {
                    tick: at,
                    len: end - at,
                    ..old
                },
            ));
        }
    }
    replace(clip, pairs)
}

/// The most times a turn inverts or spreads a chord.
const MAX_TIMES: u32 = 16;

/// Each chord of two or more `sel` notes with its pitches changed by
/// `step` `times` over, refused whole when a note would leave 0..=127.
fn revoice(
    clip: &Clip,
    sel: &[usize],
    least: usize,
    times: u32,
    step: impl Fn(&mut [(i32, usize)]),
) -> Edit {
    let mut pairs = Vec::new();
    for chord in chords_of(clip, sel) {
        if chord.len() < least {
            continue;
        }
        let mut pitch: Vec<(i32, usize)> = chord
            .iter()
            .map(|&i| (i32::from(clip.notes[i].note), i))
            .collect();
        for _ in 0..times.min(MAX_TIMES) {
            pitch.sort_unstable();
            step(&mut pitch);
        }
        for (p, i) in pitch {
            if !(0..=127).contains(&p) {
                return Edit::default();
            }
            let old = clip.notes[i];
            pairs.push((
                old,
                Note {
                    note: p as u8,
                    ..old
                },
            ));
        }
    }
    replace(clip, pairs)
}

/// Each chord of the `sel` notes inverted `times` (plan 2B.2): its lowest
/// note up an octave each time, or, with `times` below 0, its highest
/// down one. A note alone stays; refused whole when a note would leave
/// 0..=127.
pub fn invert(clip: &Clip, sel: &[usize], times: i32) -> Edit {
    revoice(clip, sel, 2, times.unsigned_abs(), |p| {
        let k = if times > 0 { 0 } else { p.len() - 1 };
        p[k].0 += if times > 0 { 12 } else { -12 };
    })
}

/// Each chord of three or more `sel` notes spread `times` (plan 2B.2):
/// the note over its lowest goes up an octave each time, for an open
/// chord; with `times` below 0 it closes, the highest note coming down an
/// octave while it stays over the lowest. Refused whole when a note
/// would pass 127.
pub fn spread(clip: &Clip, sel: &[usize], times: i32) -> Edit {
    revoice(clip, sel, 3, times.unsigned_abs(), |p| {
        if times > 0 {
            p[1].0 += 12;
        } else {
            let (lo, k) = (p[0].0, p.len() - 1);
            if p[k].0 - 12 > lo {
                p[k].0 -= 12;
            }
        }
    })
}

/// The most pieces [`chop`] cuts a note into.
pub const MAX_PIECES: u32 = 16;

/// Each `sel` note chopped into `pieces` equal notes (plan 2B.3), each
/// keeping its part of the pressure: the points inside it, from the value
/// held at its cut, as new runs (the first piece keeps the note's own
/// run; what falls past its end does not play). With `reset` no piece
/// carries pressure. Refused (empty) for fewer than 2 pieces or more than
/// [`MAX_PIECES`], a note shorter than its pieces, too many notes, or no
/// room for the pressure.
pub fn chop(clip: &Clip, sel: &[usize], pieces: u32, reset: bool) -> Edit {
    let parts = Tick::from(pieces);
    let notes: Vec<Note> = picked(clip, sel).map(|(_, n)| n).collect();
    if !(2..=MAX_PIECES).contains(&pieces)
        || notes.iter().any(|n| n.len < parts)
        || clip.notes.len() + notes.len() * (pieces as usize - 1) > MAX_NOTES
    {
        return Edit::default();
    }
    let touch_at = clip.touch.len() as u32;
    let mut e = Edit {
        touch_at,
        ..Edit::default()
    };
    // Each piece `k` of the notes, made from them: a trigger chopped
    // whole is a trigger at each piece, as `split`'s parts are.
    let mut rows: Vec<Vec<Note>> = vec![Vec::with_capacity(notes.len()); parts as usize];
    for &n in &notes {
        e.erase.push(n);
        let each = n.len / parts;
        let points = clip.touch_of(&n);
        for (k, row) in rows.iter_mut().enumerate() {
            let k = k as Tick;
            let from = each * k;
            let len = if k == parts - 1 { n.len - from } else { each };
            let touch = if reset {
                Touch::NONE
            } else if k == 0 {
                n.touch
            } else {
                // The last piece takes every point after its cut, as the
                // note did; the others, the points up to the next cut.
                let to = if k == parts - 1 {
                    u32::MAX
                } else {
                    (from + len) as u32
                };
                let run = piece_run(points, from as u32, to);
                let t = Touch {
                    first: touch_at + e.touch.len() as u32,
                    len: run.len() as u32,
                };
                e.touch.extend(run);
                if t.len == 0 {
                    Touch::NONE
                } else {
                    t
                }
            };
            row.push(Note {
                tick: wrap(n.tick + from, clip.length),
                len,
                touch,
                ..n
            });
        }
    }
    if !touch_room(clip.touch.len(), &e.touch) {
        return Edit::default();
    }
    // The first pieces keep their notes' ticks (and triggers); the rest
    // are copies settled beside them.
    let mut settled = Vec::with_capacity(notes.len() * parts as usize);
    for (k, mut row) in rows.into_iter().enumerate() {
        let ok = if k == 0 {
            crate::groups::settle_moved(clip, &e.erase, &notes, &mut row)
        } else {
            crate::groups::settle_copied_beside(clip, &e.erase, &settled, &notes, &mut row)
        };
        if ok.is_err() {
            return Edit::default();
        }
        settled.extend(row);
    }
    // In the order they were made: each note's pieces together.
    let n = notes.len();
    e.add = (0..n)
        .flat_map(|i| (0..parts as usize).map(move |k| (k, i)))
        .map(|(k, i)| settled[k * n + i])
        .collect();
    e
}

/// The part of a note's pressure run from `from` to `to` ticks after its
/// start, as a run of its own starting at 0: the value held at `from`
/// (the last point before it, when no point falls on it), then the
/// points inside.
fn piece_run(points: &[TouchPoint], from: u32, to: u32) -> Vec<TouchPoint> {
    let mut run = Vec::new();
    if !points.iter().any(|p| p.at == from) {
        if let Some(held) = points.iter().rev().find(|p| p.at < from) {
            run.push(TouchPoint {
                at: 0,
                value: held.value,
            });
        }
    }
    run.extend(
        points
            .iter()
            .filter(|p| (from..to).contains(&p.at))
            .map(|p| TouchPoint {
                at: p.at - from,
                value: p.value,
            }),
    );
    run
}

/// Each `sel` note held on toward the next start of a picked note after
/// its own (plan 2B.3): its end moves `fill` percent (0 to 100) of the way
/// to that start, so 100 is Timing Correct's Legato (one that ran past it
/// comes back to it). Notes starting together go to the same start; the
/// last ones keep their length.
pub fn legato(clip: &Clip, sel: &[usize], fill: u8) -> Edit {
    let fill = Tick::from(fill.min(100));
    let mut starts: Vec<Tick> = picked(clip, sel).map(|(_, n)| n.tick).collect();
    starts.sort_unstable();
    starts.dedup();
    map(clip, sel, |n| {
        let &next = starts.iter().find(|&&s| s > n.tick)?;
        let gap = next - (n.tick + n.len);
        // Rounded half away from 0, so a turn either way moves alike.
        let by = (gap * fill + gap.signum() * 50) / 100;
        Some(Note {
            len: fit_len(n.len + by),
            ..n
        })
    })
}

/// Pressure drawn over the `sel` notes (piano roll plan 3A.1), as the
/// lane under the roll draws it: `points` are (clip tick, value), each
/// value held until the next point's tick, the last until `end`. Each
/// picked note the stroke crosses gets a new run: its own points before
/// the stroke, the drawn ones inside it (from the value over its start,
/// when the stroke began before it), then the value it had at `end` and
/// its own points after. A value equal to the one before it is left out,
/// and a run left all 0 is none. The note's old run stays in the clip,
/// since a clip's pressure only grows (undo puts the note back with it),
/// until the clip frees what nothing names (`Takes::reclaim_touch`).
/// Empty when nothing changes or no picked note is under the stroke;
/// None when the clip has no room for the new runs.
pub fn draw_touch(clip: &Clip, sel: &[usize], points: &[(Tick, u8)], end: Tick) -> Option<Edit> {
    let mut drawn: Vec<(Tick, u8)> = points.to_vec();
    drawn.sort_by_key(|p| p.0);
    // Two values at one tick: the later one drawn.
    drawn.reverse();
    drawn.dedup_by_key(|p| p.0);
    drawn.reverse();
    let Some(&(from, _)) = drawn.first() else {
        return Some(Edit::default());
    };
    if end <= from {
        return Some(Edit::default());
    }
    let touch_at = clip.touch.len() as u32;
    let mut e = Edit {
        touch_at,
        ..Edit::default()
    };
    for (_, n) in picked(clip, sel) {
        let (a, b) = (from - n.tick, end - n.tick);
        if b <= 0 || a >= n.len {
            continue;
        }
        let old = clip.touch_of(&n);
        let held = |at: Tick| {
            old.iter()
                .rev()
                .find(|p| Tick::from(p.at) <= at)
                .map_or(0, |p| p.value)
        };
        let mut run: Vec<TouchPoint> = Vec::with_capacity(old.len() + drawn.len() + 1);
        for p in old.iter().filter(|p| Tick::from(p.at) < a) {
            push_point(&mut run, p.at, p.value);
        }
        for (k, &(t, v)) in drawn.iter().enumerate() {
            let next = drawn.get(k + 1).map_or(end, |q| q.0);
            if next <= n.tick || t >= n.tick + n.len {
                continue;
            }
            push_point(&mut run, (t - n.tick).max(0) as u32, v.min(127));
        }
        if b < n.len {
            push_point(&mut run, b as u32, held(b));
            for p in old.iter().filter(|p| Tick::from(p.at) > b) {
                push_point(&mut run, p.at, p.value);
            }
        }
        if run.iter().all(|p| p.value == 0) {
            run.clear();
        }
        let mut was = Vec::with_capacity(old.len());
        for p in old {
            push_point(&mut was, p.at, p.value);
        }
        if was.iter().all(|p| p.value == 0) {
            was.clear();
        }
        // The same pressure, as it plays: nothing to change.
        if run == was {
            continue;
        }
        let touch = if run.is_empty() {
            Touch::NONE
        } else {
            Touch {
                first: touch_at + e.touch.len() as u32,
                len: run.len() as u32,
            }
        };
        e.touch.extend(run);
        e.erase.push(n);
        e.add.push(Note { touch, ..n });
    }
    touch_room(clip.touch.len(), &e.touch).then_some(e)
}

/// Point (`at`, `value`) onto the end of a pressure run: in place of a
/// point at the same tick, and left out when the value held already is
/// the same.
fn push_point(run: &mut Vec<TouchPoint>, at: u32, value: u8) {
    if run.last().is_some_and(|p| p.at == at) {
        run.pop();
    }
    if run.last().is_some_and(|p| p.value == value) {
        return;
    }
    run.push(TouchPoint { at, value });
}

/// The clip's steps as the Steps view cuts them (its step size, or a
/// 1/16 for a size it doesn't know): where its locks sit.
pub fn steps_grid(clip: &Clip) -> Grid {
    Grid::new(clip.step)
        .filter(|_| SIZES.contains(&clip.step))
        .or_else(|| Grid::new(DEFAULT_SIZE))
        .expect("a 1/16 is a step size")
}

/// Where a lock of `addr` on a note of pitch `note` sits (piano roll plan
/// 3A.2), as (step row) for [`Grid::lock`]: the note's own pitch when
/// `own` (a lock of that setting already sits on its pitch in its step),
/// else a pad's setting on its own pad's row, else the whole step (a
/// synth's or the track's setting, as the Steps' lock lanes set them).
/// None: a pad's setting and a note of another pad, which it doesn't
/// play.
pub fn lock_row(addr: RelAddr, note: u8, own: bool) -> Option<Option<u8>> {
    if own {
        return Some(Some(note));
    }
    match addr.on {
        OnTrack::Pad(p) if p == note => Some(Some(p)),
        OnTrack::Pad(_) => None,
        _ => Some(None),
    }
}

/// Note `n`'s step and the row its lock of `addr` sits on in `clip`
/// ([`lock_row`]); None when the setting isn't its.
pub fn lock_place(clip: &Clip, n: &Note, addr: RelAddr) -> Option<(usize, Option<u8>)> {
    let g = steps_grid(clip);
    let step = g.place(clip.length, n.tick).0;
    let own = clip.locks.iter().any(|l| {
        l.addr == addr && l.sound == Some(n.note) && g.place(clip.length, l.tick).0 == step
    });
    Some((step, lock_row(addr, n.note, own)?))
}

/// The value of `addr` locked where note `n` plays it, if any.
pub fn lock_of(clip: &Clip, n: &Note, addr: RelAddr) -> Option<f32> {
    let (step, row) = lock_place(clip, n, addr)?;
    let g = steps_grid(clip);
    clip.locks
        .iter()
        .find(|l| l.addr == addr && l.sound == row && g.place(clip.length, l.tick).0 == step)
        .map(|l| l.value)
}

/// Locks drawn on a lock lane under the roll (plan 3A.2): `addr` set to
/// each value where each note of `set` (by index) plays it, the step's
/// lock on its row ([`lock_place`]), as the Steps' lock lanes set it, so
/// notes that share a step and a row share the lock (the last value for
/// it wins). Notes the setting isn't for are left out; empty when
/// nothing changes.
pub fn lock_notes(clip: &Clip, addr: RelAddr, set: &[(usize, f32)]) -> Edit {
    let g = steps_grid(clip);
    let mut places: Vec<((usize, Option<u8>), f32)> = Vec::new();
    for &(i, v) in set {
        let Some(place) = clip.notes.get(i).and_then(|n| lock_place(clip, n, addr)) else {
            continue;
        };
        match places.iter_mut().find(|(p, _)| *p == place) {
            Some(e) => e.1 = v,
            None => places.push((place, v)),
        }
    }
    let mut e = Edit::default();
    for ((step, row), v) in places {
        e.join(g.lock(clip, row, step, addr, v));
    }
    e
}

/// The locks of `addr` taken off where the `sel` notes play it: ERASE +
/// a tap on a lock lane (plan 3A.2).
pub fn unlock_notes(clip: &Clip, addr: RelAddr, sel: &[usize]) -> Edit {
    let g = steps_grid(clip);
    let mut places: Vec<(usize, Option<u8>)> = Vec::new();
    for (_, n) in picked(clip, sel) {
        if let Some(p) = lock_place(clip, &n, addr) {
            if !places.contains(&p) {
                places.push(p);
            }
        }
    }
    let mut e = Edit::default();
    for (step, row) in places {
        e.join(g.unlock(clip, row, step, addr));
    }
    e
}

/// The `sel` notes that are in the clip, with their indexes.
fn picked<'a>(clip: &'a Clip, sel: &'a [usize]) -> impl Iterator<Item = (usize, Note)> + 'a {
    let mut s: Vec<usize> = sel
        .iter()
        .copied()
        .filter(|&i| i < clip.notes.len())
        .collect();
    s.sort_unstable();
    s.dedup();
    s.into_iter().map(move |i| (i, clip.notes[i]))
}

/// The edit that changes each `sel` note by `f` (None leaves it).
fn map(clip: &Clip, sel: &[usize], mut f: impl FnMut(Note) -> Option<Note>) -> Edit {
    replace(
        clip,
        picked(clip, sel)
            .filter_map(|(_, n)| f(n).map(|m| (n, m)))
            .collect(),
    )
}

/// After `edit` is applied, the indexes of the notes it put in (to keep
/// them selected). A note put in that equals one already there gives the
/// first such index.
pub fn added_indexes(clip: &Clip, edit: &Edit) -> Vec<usize> {
    let mut used = vec![false; clip.notes.len()];
    let mut out = Vec::with_capacity(edit.add.len());
    for a in &edit.add {
        if let Some(i) = (0..clip.notes.len()).find(|&i| !used[i] && clip.notes[i] == *a) {
            used[i] = true;
            out.push(i);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const Q: Tick = 240;

    fn clip(notes: &[(Tick, Tick, u8)]) -> Clip {
        let mut c = Clip::new("t", 4 * 960);
        for &(t, l, n) in notes {
            c.add_note(Note::new(t, l, n, 100)).unwrap();
        }
        c
    }

    fn applied(c: &Clip, e: &Edit) -> Clip {
        let mut d = c.clone();
        d.apply(e).unwrap();
        let mut back = d.clone();
        back.apply(&e.inverse()).unwrap();
        let mut a = back.notes.clone();
        let mut b = c.notes.clone();
        a.sort_by_key(|n| (n.tick, n.note, n.len));
        b.sort_by_key(|n| (n.tick, n.note, n.len));
        assert_eq!(a, b, "the edit undoes");
        d
    }

    /// The note with number `n` (the tests give each its own).
    fn by(c: &Clip, n: u8) -> Note {
        *c.notes.iter().find(|x| x.note == n).expect("the note")
    }

    fn ticks(c: &Clip) -> Vec<Tick> {
        let mut v: Vec<Tick> = c.notes.iter().map(|n| n.tick).collect();
        v.sort_unstable();
        v
    }

    #[test]
    fn start_moves_notes_onto_the_division_and_wraps_at_the_end() {
        let c = clip(&[(10, 100, 36), (230, 100, 38), (500, 50, 36), (3835, 50, 40)]);
        let d = applied(&c, &timing_correct(&c, Tc::default(), &Which::All));
        assert_eq!(ticks(&d), vec![0, 0, 240, 480]);
        // Lengths stay.
        assert!(d.notes.iter().any(|n| n.tick == 480 && n.len == 50));
        // A note already on a line is left out of the edit.
        let e = timing_correct(&d, Tc::default(), &Which::All);
        assert!(e.is_empty());
    }

    #[test]
    fn strength_window_swing_and_shift() {
        let c = clip(&[(100, 10, 36), (300, 10, 36)]);
        let half = Tc {
            strength: 50,
            ..Tc::default()
        };
        // 100 goes halfway to 0, 300 halfway to 240.
        assert_eq!(
            ticks(&applied(&c, &timing_correct(&c, half, &Which::All))),
            vec![50, 270]
        );
        // A window of 50 % reaches 60 ticks either side of a 1/16 line:
        // 100 is 100 from 0 and 140 from 240, so it stays; 300 moves.
        let narrow = Tc {
            window: 50,
            ..Tc::default()
        };
        assert_eq!(
            ticks(&applied(&c, &timing_correct(&c, narrow, &Which::All))),
            vec![100, 240]
        );
        // 66 % swing puts every second 1/16 line 77 ticks late (240 * 0.32).
        let swung = Tc {
            swing: 66,
            ..Tc::default()
        };
        assert_eq!(swung.correct(250), 317);
        assert_eq!(swung.correct(470), 480);
        // Shift moves every chosen note after the correction.
        let late = Tc {
            shift: 12,
            ..Tc::default()
        };
        assert_eq!(
            ticks(&applied(&c, &timing_correct(&c, late, &Which::All))),
            vec![12, 252]
        );
    }

    #[test]
    fn end_length_and_legato() {
        let c = clip(&[(0, 200, 36), (240, 300, 38), (960, 100, 40)]);
        let end = Tc {
            kind: TcKind::End,
            ..Tc::default()
        };
        let d = applied(&c, &timing_correct(&c, end, &Which::All));
        let lens = |d: &Clip| {
            let mut v: Vec<(Tick, Tick)> = d.notes.iter().map(|n| (n.tick, n.len)).collect();
            v.sort_unstable();
            v
        };
        assert_eq!(lens(&d), vec![(0, 240), (240, 240), (960, 1)]);
        let length = Tc {
            kind: TcKind::Length,
            division: 480,
            ..Tc::default()
        };
        let d = applied(&c, &timing_correct(&c, length, &Which::All));
        assert_eq!(lens(&d), vec![(0, 480), (240, 480), (960, 480)]);
        let legato = Tc {
            kind: TcKind::Legato,
            ..Tc::default()
        };
        let c2 = clip(&[(0, 10, 36), (0, 50, 38), (240, 10, 40), (960, 77, 41)]);
        let d = applied(&c2, &timing_correct(&c2, legato, &Which::All));
        assert_eq!(lens(&d), vec![(0, 240), (0, 240), (240, 720), (960, 77)]);
    }

    #[test]
    fn which_picks_selected_or_a_range() {
        let c = clip(&[(10, 10, 36), (250, 10, 38), (490, 10, 60)]);
        let sel = Which::Selected(vec![2, 0, 0, 9]);
        assert_eq!(sel.pick(&c), vec![0, 2]);
        let range = Which::Range {
            from: 0,
            to: 480,
            low: 36,
            high: 37,
        };
        assert_eq!(range.pick(&c), vec![0]);
        let d = applied(&c, &timing_correct(&c, Tc::default(), &range));
        assert_eq!(ticks(&d), vec![0, 250, 490]);
    }

    #[test]
    fn nudges_snap_absolute_relative_or_not_at_all() {
        let c = clip(&[(250, 10, 36), (480, 10, 38)]);
        let on = |e: &Edit| ticks(&applied(&c, e));
        assert_eq!(
            on(&nudge(&c, &[0, 1], 1, Snap::Absolute(Q))),
            vec![480, 720]
        );
        assert_eq!(
            on(&nudge(&c, &[0, 1], -1, Snap::Absolute(Q))),
            vec![240, 240]
        );
        assert_eq!(on(&nudge(&c, &[0], 2, Snap::Relative(Q))), vec![480, 730]);
        assert_eq!(on(&nudge(&c, &[0], -3, Snap::Off)), vec![238, 480]);
        // Round the clip's start.
        assert_eq!(on(&nudge(&c, &[1], -3, Snap::Relative(Q))), vec![250, 3600]);
    }

    #[test]
    fn a_chance_stroke_gives_a_chord_that_rolls_as_one_one_chance() {
        // A three-note chord that rolls as one (group 129) and a note of
        // its own beside it.
        let mut c = clip(&[(0, Q, 60), (0, Q, 64), (0, Q, 67), (Q, Q, 72)]);
        c.holds = Holds::Notes;
        for n in c.notes.iter_mut().take(3) {
            n.group = 129;
        }
        // A stroke across the chord gives its notes 80, 40 and 60, and
        // the note beside it 30: the chord takes the last, 60, all three.
        let e = set_values(&c, Field::Chance, &[(0, 80), (1, 40), (2, 60), (3, 30)]);
        let mut d = c.clone();
        d.apply(&e).unwrap();
        assert!(d.groups_ok());
        let mut chances: Vec<(u8, u8)> = d.notes.iter().map(|n| (n.note, n.chance)).collect();
        chances.sort_unstable();
        assert_eq!(chances, [(60, 60), (64, 60), (67, 60), (72, 30)]);
        // One note of the chord named: the whole chord takes it.
        let e = set_values(&c, Field::Chance, &[(1, 25)]);
        let mut d = c.clone();
        d.apply(&e).unwrap();
        assert!(d
            .notes
            .iter()
            .filter(|n| n.group == 129)
            .all(|n| n.chance == 25));
        assert_eq!(d.notes.iter().filter(|n| n.group == 129).count(), 3);
        // Velocity stays each note's own.
        let e = set_values(&c, Field::Velocity, &[(0, 10), (1, 90)]);
        let mut d = c.clone();
        d.apply(&e).unwrap();
        let mut v: Vec<(u8, u8)> = d.notes.iter().map(|n| (n.note, n.velocity)).collect();
        v.sort_unstable();
        assert_eq!(&v[..2], [(60, 10), (64, 90)]);
    }

    #[test]
    fn a_lane_stroke_sets_each_note_it_names_in_one_edit() {
        let mut c = clip(&[(0, 240, 36), (240, 240, 38), (480, 240, 40)]);
        let cut = RelAddr::new(OnTrack::Inst, 1);
        c.add_lock(Lock::new(240, Some(38), cut, 800.0)).unwrap();
        let vels = |d: &Clip| [by(d, 36).velocity, by(d, 38).velocity, by(d, 40).velocity];
        // A crescendo: each note its own value, one edit, the lock kept.
        let e = set_values(&c, Field::Velocity, &[(0, 40), (1, 80), (2, 120)]);
        assert_eq!(e.erase.len(), 3);
        assert!(e.erase_locks.is_empty(), "no note moved: its locks stay");
        let d = applied(&c, &e);
        assert_eq!(vels(&d), [40, 80, 120]);
        assert_eq!(d.locks.len(), 1);
        // Kept in range, the last value for a note named twice, notes past
        // the clip and unchanged notes left out.
        let e = set_values(&c, Field::Velocity, &[(0, 300), (0, 0), (9, 50), (1, 100)]);
        assert_eq!(vels(&applied(&c, &e)), [1, 100, 100]);
        assert_eq!(e.erase.len(), 1, "the second was already 100");
        assert!(set_values(&c, Field::Velocity, &[(2, 100)]).is_empty());
        // Chance 1 to 100, repeats 1 to 8.
        let d = applied(&c, &set_values(&c, Field::Chance, &[(0, 0), (1, 150)]));
        assert_eq!((by(&d, 36).chance, by(&d, 38).chance), (1, 100));
        let d = applied(&c, &set_values(&c, Field::Repeats, &[(0, 3), (1, 20)]));
        assert_eq!(
            (by(&d, 36).ratchet, by(&d, 38).ratchet),
            (3, crate::MAX_RATCHET)
        );
        // A ramp only on a note that repeats.
        let at = |d: &Clip, n: u8| d.notes.iter().position(|m| m.note == n).unwrap();
        let e = set_values(&d, Field::Ramp, &[(at(&d, 36), -40), (at(&d, 40), 40)]);
        let r = applied(&d, &e);
        assert_eq!((by(&r, 36).ramp, by(&r, 40).ramp), (-40, 0));
        assert_eq!(e.erase.len(), 1);
        assert_eq!(Field::Ramp.range(), (-126, 126));
        assert_eq!(
            Field::LANES.map(Field::name),
            ["Velocity", "Chance", "Repeats"]
        );
    }

    #[test]
    fn the_inspectors_velocity_keeps_the_differences_and_spreads_them() {
        let mut c = clip(&[(0, 240, 36), (240, 240, 38), (480, 240, 40)]);
        for (n, v) in [(0, 80), (1, 92), (2, 104)] {
            c.notes[n].velocity = v;
        }
        let vels = |d: &Clip| [by(d, 36).velocity, by(d, 38).velocity, by(d, 40).velocity];
        let all = [0, 1, 2];
        // Up and down, every note by the same: the loudest stops at 127
        // and the others with it.
        let d = applied(&c, &velocity_kept(&c, &all, 10));
        assert_eq!(vels(&d), [90, 102, 114]);
        let d = applied(&c, &velocity_kept(&c, &all, 40));
        assert_eq!(vels(&d), [103, 115, 127]);
        let d = applied(&c, &velocity_kept(&c, &all, -100));
        assert_eq!(vels(&d), [1, 13, 25]);
        let mut top = c.clone();
        top.notes[2].velocity = 127;
        assert!(
            velocity_kept(&top, &all, 1).is_empty(),
            "the loudest is at 127"
        );
        // Wider: an eighth of the gap at each end, the middle note kept
        // in its place between them.
        let d = applied(&c, &velocity_spread(&c, &all, 1));
        assert_eq!(vels(&d), [77, 92, 107]);
        let d = applied(&c, &velocity_spread(&c, &all, -1));
        assert_eq!(vels(&d), [83, 92, 101]);
        // Narrowing stops before the ends meet; widening stops at 1 and
        // 127.
        let d = applied(&c, &velocity_spread(&c, &all, -40));
        let v = vels(&d);
        assert!(v[0] < v[2] && v[0] <= v[1] && v[1] <= v[2], "{v:?}");
        assert!(
            velocity_spread(&d, &all, -1).is_empty(),
            "as narrow as it goes"
        );
        let d = applied(&c, &velocity_spread(&c, &all, 60));
        assert_eq!((vels(&d)[0], vels(&d)[2]), (1, 127));
        // One velocity among them: nothing to spread.
        assert!(velocity_spread(&c, &[1], 1).is_empty());
        assert!(velocity_kept(&c, &[], 1).is_empty());
    }

    #[test]
    fn grid_edits() {
        let c = clip(&[(0, 240, 36), (480, 240, 38)]);
        let d = applied(&c, &shift(&c, &[0, 1], 120, 2));
        assert_eq!(
            d.notes.iter().map(|n| (n.tick, n.note)).collect::<Vec<_>>(),
            vec![(120, 38), (600, 40)]
        );
        assert!(shift(&c, &[0], 0, -40).is_empty(), "past note 0");
        let d = applied(&c, &edit_start(&c, &[0], 100));
        assert_eq!((by(&d, 36).tick, by(&d, 36).len), (100, 140));
        let d = applied(&c, &edit_start(&c, &[0], 1000));
        assert_eq!((by(&d, 36).tick, by(&d, 36).len), (239, 1));
        let d = applied(&c, &edit_end(&c, &[1], -300));
        assert_eq!(by(&d, 38).len, 1);
        let d = applied(&c, &transpose(&c, &[0, 1], 100));
        assert!(d.notes.iter().all(|n| n.note == 127));
        let d = applied(&c, &velocity(&c, &[0], -200));
        assert_eq!(by(&d, 36).velocity, 1);
        let d = applied(&c, &set_velocity(&c, &[1], 64));
        assert_eq!(by(&d, 38).velocity, 64);
        // Chance, condition and ratchet, each kept in range.
        let d = applied(&c, &chance(&c, &[0], -30));
        assert_eq!((by(&d, 36).chance, by(&d, 38).chance), (70, 100));
        assert_eq!(by(&applied(&c, &chance(&c, &[0], -300)), 36).chance, 1);
        assert!(chance(&c, &[0], 5).is_empty(), "already 100 %");
        let d = applied(&c, &condition(&c, &[0, 1], 2));
        assert!(d
            .notes
            .iter()
            .all(|n| n.cond == crate::Cond::Every { a: 2, b: 2 }));
        assert!(condition(&c, &[0], -1).is_empty(), "always is the first");
        let d = applied(&c, &ratchet(&c, &[1], 3));
        assert_eq!(by(&d, 38).ratchet, 4);
        let d = applied(&c, &ratchet(&c, &[1], 30));
        assert_eq!(by(&d, 38).ratchet, crate::MAX_RATCHET);
        let d = applied(&c, &delete(&c, &[1]));
        assert_eq!(d.notes.len(), 1);
        let d = applied(&c, &add(&c, Note::new(960, 120, 42, 90)));
        assert_eq!(d.notes.len(), 3);
        assert!(add(&c, Note::new(5000, 120, 42, 90)).is_empty());
        // Duplicate: the span 0..720 rounds to 720 (three 1/16s).
        let d = applied(&c, &duplicate(&c, &[0, 1], Q));
        assert_eq!(ticks(&d), vec![0, 480, 720, 1200]);
        // Paste at a tick keeps the spacing.
        let notes = vec![c.notes[0], c.notes[1]];
        let d = applied(&c, &paste(&c, &notes, 1920, 0));
        assert_eq!(ticks(&d), vec![0, 480, 1920, 2400]);
        let e = add(&c, Note::new(960, 120, 42, 90));
        let mut d = c.clone();
        d.apply(&e).unwrap();
        assert_eq!(added_indexes(&d, &e), vec![2]);
    }

    #[test]
    fn locks_go_with_their_notes() {
        use mpfree_param::{OnTrack, RelAddr};
        let cut = RelAddr::new(OnTrack::Inst, 1);
        let mut c = clip(&[(250, 100, 36), (250, 100, 38)]);
        let own36 = Lock::new(250, Some(36), cut, 800.0);
        let step = Lock::new(250, None, cut, 300.0);
        c.add_lock(own36).unwrap();
        c.add_lock(step).unwrap();
        let locks = |d: &Clip| {
            let mut v: Vec<(Tick, Option<u8>)> =
                d.locks.iter().map(|l| (l.tick, l.sound)).collect();
            v.sort_unstable();
            v
        };
        // Timing Correct moves the note and its own lock; the step's stays.
        let d = applied(
            &c,
            &timing_correct(&c, Tc::default(), &Which::Selected(vec![0])),
        );
        assert_eq!(locks(&d), vec![(240, Some(36)), (250, None)]);
        // A drag up a tone takes the lock to the new sound.
        let d = applied(&c, &shift(&c, &[0], 0, 2));
        assert_eq!(locks(&d), vec![(250, None), (250, Some(38))]);
        // Delete takes it out; duplicate copies it.
        let d = applied(&c, &delete(&c, &[0]));
        assert_eq!(locks(&d), vec![(250, None)]);
        let d = applied(&c, &duplicate(&c, &[0], Q));
        assert_eq!(
            locks(&d),
            vec![(250, None), (250, Some(36)), (490, Some(36))]
        );
        // Double length and the speeds take every lock.
        let d = applied(&c, &double_length(&c));
        assert_eq!(locks(&d).len(), 4);
        let d = applied(&c, &half_speed(&c));
        assert_eq!(locks(&d), vec![(500, None), (500, Some(36))]);
    }

    #[test]
    fn whole_clip_edits() {
        let c = clip(&[(0, 240, 36), (960, 480, 38)]);
        let d = applied(&c, &double_length(&c));
        assert_eq!((d.length, ticks(&d)), (7680, vec![0, 960, 3840, 4800]));
        let d = applied(&c, &half_speed(&c));
        assert_eq!(d.length, 7680);
        assert_eq!(
            d.notes.iter().map(|n| (n.tick, n.len)).collect::<Vec<_>>(),
            vec![(0, 480), (1920, 960)]
        );
        let d = applied(&c, &double_speed(&c));
        assert_eq!(d.length, 1920);
        assert_eq!(
            d.notes.iter().map(|n| (n.tick, n.len)).collect::<Vec<_>>(),
            vec![(0, 120), (480, 240)]
        );
        let d = applied(&c, &split(&c, &[1], 3));
        assert_eq!(ticks(&d), vec![0, 960, 1120, 1280]);
        assert!(split(&c, &[1], 1).is_empty());
        // A split leaves a note's pressure on its first part.
        let mut c = clip(&[(0, 480, 36)]);
        let t = c
            .add_touch(&[crate::TouchPoint { at: 0, value: 50 }])
            .unwrap();
        c.notes[0].touch = t;
        let d = applied(&c, &split(&c, &[0], 2));
        let mut parts: Vec<_> = d.notes.iter().map(|n| (n.tick, n.touch)).collect();
        parts.sort_by_key(|p| p.0);
        assert_eq!(parts, vec![(0, t), (240, Touch::NONE)]);
    }

    /// Half and double speed stretch each note's pressure with it: its
    /// run comes again with its times doubled or halved, added to the
    /// clip's pressure, and the old run stays for undo. Two notes keep
    /// their own runs; a clip with no room for the new runs refuses.
    #[test]
    fn half_and_double_speed_stretch_the_pressure_with_its_note() {
        use crate::TouchPoint;
        let p = |at, value| TouchPoint { at, value };
        let mut c = clip(&[(0, 480, 36), (960, 240, 38)]);
        let a = c.add_touch(&[p(0, 10), p(100, 60), p(479, 90)]).unwrap();
        let b = c.add_touch(&[p(0, 30), p(239, 5)]).unwrap();
        c.notes[0].touch = a;
        c.notes[1].touch = b;
        let runs = |d: &Clip| -> Vec<(Tick, Vec<(u32, u8)>)> {
            let mut r: Vec<_> = d
                .notes
                .iter()
                .map(|n| {
                    (
                        n.tick,
                        d.touch_of(n).iter().map(|q| (q.at, q.value)).collect(),
                    )
                })
                .collect();
            r.sort_by_key(|x| x.0);
            r
        };
        let d = applied(&c, &half_speed(&c));
        assert_eq!(
            runs(&d),
            vec![
                (0, vec![(0, 10), (200, 60), (958, 90)]),
                (1920, vec![(0, 30), (478, 5)])
            ]
        );
        assert_eq!(d.touch.len(), 10, "the old runs stay, for undo");
        let d = applied(&c, &double_speed(&c));
        assert_eq!(
            runs(&d),
            vec![
                (0, vec![(0, 10), (50, 60), (239, 90)]),
                (480, vec![(0, 30), (119, 5)])
            ]
        );
        // Undone (the inverse), the notes name their old runs again.
        let mut e = c.clone();
        let edit = half_speed(&c);
        e.apply(&edit).unwrap();
        e.apply(&edit.inverse()).unwrap();
        assert_eq!(runs(&e), runs(&c));
        // No room for the new runs: refused, as with FX moves.
        let mut full = c.clone();
        full.touch
            .resize(crate::MAX_TOUCH_POINTS - 4, TouchPoint::default());
        assert!(half_speed(&full).is_empty());
        // An edit made for a clip whose pressure has grown since is
        // refused and leaves the clip as it was.
        let mut grown = c.clone();
        grown.add_touch(&[p(0, 1)]).unwrap();
        let before = grown.clone();
        assert!(grown.apply(&edit).is_err());
        assert_eq!(grown, before);
    }

    #[test]
    fn humanize_is_the_same_for_a_seed_and_stays_in_range() {
        let c = clip(&[(0, 240, 36), (960, 480, 38), (1920, 10, 40)]);
        let h = Humanize {
            time: 20,
            eagerness: 50,
            length: 30,
            velocity: 40,
        };
        let a = humanize(&c, &[0, 1, 2], h, 7);
        assert_eq!(a, humanize(&c, &[0, 1, 2], h, 7));
        assert_ne!(a, humanize(&c, &[0, 1, 2], h, 8));
        let d = applied(&c, &a);
        assert!(d.check().is_ok());
        for n in &c.notes {
            let m = by(&d, n.note);
            let dt = (m.tick - n.tick + 1920).rem_euclid(3840) - 1920;
            assert!(dt.abs() <= 20, "{dt}");
        }
        assert!(humanize(&c, &[0], Humanize::default(), 1).is_empty());
    }

    #[test]
    fn pitch_quantize_to_a_scale_in_a_range() {
        // C major: C D E F G A B.
        let major = 0b1010_1011_0101;
        let c = clip(&[(0, 10, 61), (10, 10, 66), (20, 10, 30), (30, 10, 100)]);
        let d = applied(&c, &pitch_quantize(&c, &[0, 1, 2, 3], 0, major, 48, 72));
        assert_eq!(
            d.notes.iter().map(|n| n.note).collect::<Vec<_>>(),
            vec![60, 65, 48, 72]
        );
        assert!(pitch_quantize(&c, &[0], 0, 0, 0, 127).is_empty());
        assert!(
            pitch_quantize(&c, &[0], 1, 1, 60, 60).is_empty(),
            "no C# in 60..=60"
        );
    }

    /// Moves by notes of a scale (piano roll plan 2B.1): a scale note
    /// steps to the next, a note off the scale first to the one beside it
    /// on the way; a finger on a row lands on its nearest scale note,
    /// the nearer one to where the note was on a tie; a chord moves each
    /// note by its own steps; Fit to scale takes the nearest.
    #[test]
    fn moves_go_by_notes_of_the_clips_scale() {
        let c_major = Scale::new(0, mpfree_scale::MAJOR);
        // C D E F G A B.
        assert_eq!(scale_move(60, 1, c_major), 62);
        assert_eq!(scale_move(64, 1, c_major), 65);
        assert_eq!(scale_move(60, -1, c_major), 59);
        assert_eq!(scale_move(60, 7, c_major), 72);
        assert_eq!(scale_move(61, 1, c_major), 62, "C# up: D");
        assert_eq!(scale_move(61, -1, c_major), 60, "C# down: C");
        assert_eq!(scale_move(61, 0, c_major), 61, "unmoved, it stays off");
        // Only the notes going up count: the classical melodic minor
        // moves by its rising notes both ways.
        let classical = Scale::of(9, mpfree_scale::by_name("Mel. minor classical").unwrap());
        assert_eq!(
            scale_move(78, -1, classical),
            76,
            "F# down: E (F only comes down)"
        );
        assert_eq!(scale_steps(60, 62, c_major), 1);
        assert_eq!(scale_steps(60, 61, c_major), 0, "C# is as near C as D: C");
        assert_eq!(scale_steps(62, 61, c_major), 0);
        assert_eq!(scale_steps(60, 63, c_major), 1, "D# is as near D as E: D");
        assert_eq!(scale_steps(67, 63, c_major), -2, "... and from above, E");
        assert_eq!(scale_steps(60, 72, c_major), 7);
        assert_eq!(scale_steps(60, 48, c_major), -7);
        assert_eq!(scale_steps(61, 62, c_major), 1);
        assert_eq!(scale_steps(61, 60, c_major), -1);
        for from in 40..80 {
            for to in 40..80 {
                let steps = scale_steps(from, to, c_major);
                let landed = scale_move(from, steps, c_major);
                if from != to {
                    assert!(c_major.contains(landed as i16) || steps == 0, "{from} {to}");
                    assert!((landed - to).abs() <= 1, "{from} → {to}: {landed}");
                }
            }
        }
        let c = clip(&[(0, Q, 60), (0, Q, 64), (0, Q, 67), (Q, Q, 61)]);
        let up = applied(&c, &shift_in_scale(&c, &[0, 1, 2, 3], Q, 1, c_major));
        let mut got: Vec<(Tick, u8)> = up.notes.iter().map(|n| (n.tick, n.note)).collect();
        got.sort_unstable();
        assert_eq!(got, vec![(Q, 62), (Q, 65), (Q, 69), (2 * Q, 62)]);
        let high = clip(&[(0, Q, 127), (0, Q, 60)]);
        assert!(shift_in_scale(&high, &[0, 1], 0, 1, c_major).is_empty());
        let fit = applied(&c, &fit_to_scale(&c, &[3], c_major));
        assert!(fit
            .notes
            .iter()
            .all(|n| c_major.contains(i16::from(n.note))));
        assert_eq!(
            fit.notes.iter().filter(|n| n.note == 60).count(),
            2,
            "C# fits to C"
        );
    }

    /// Copying between clips (piano roll plan 1B.4): the copy keeps the
    /// notes' places from the first, their pressure and the locks they
    /// own; put in another clip it wraps round it, the pressure comes as
    /// new runs after that clip's own, a lock the track may not have is
    /// left out and named, and a copied lock takes its place's lock. One
    /// take, which undoes; refused whole when the clip has no room.
    /// A track with MIDI effects `fx` and no instrument known.
    fn at(fx: &TrackFx) -> TrackSettings<'_> {
        TrackSettings { fx, inst: None }
    }

    #[test]
    fn a_copied_trigger_stays_one_where_it_lands_and_a_part_of_one_decides_alone() {
        // A clip of notes with a three-note chord that rolls as one
        // (group 129) on the first beat, and the same number on beat 3.
        let mut c = clip(&[(0, Q, 60), (0, Q, 64), (0, Q, 67), (2 * Q, Q, 72)]);
        c.holds = Holds::Notes;
        for n in c.notes.iter_mut() {
            n.group = 129;
        }
        assert!(c.groups_ok());
        // Copied whole, the chord keeps its number...
        let whole = copy(&c, &[0, 1, 2]);
        assert!(whole.notes.iter().all(|n| n.group == 129));
        // ...and pasted on beat 3, where 129 is taken, it is a trigger of
        // its own there with a free number, never joined to the note
        // already there.
        let into = carry(&c, &whole, 2 * Q, |_| true).unwrap();
        let g: Vec<u8> = into.edit.add.iter().map(|n| n.group).collect();
        assert!(
            g.iter().all(|&x| x == g[0]) && g[0] != 129 && g[0] != 0,
            "{g:?}"
        );
        let mut d = c.clone();
        d.apply(&into.edit).unwrap();
        assert!(d.groups_ok());
        assert_eq!(d.trigger(2 * Q, 129).len(), 1);
        assert_eq!(d.trigger(2 * Q, g[0]).len(), 3);
        // Pasted on an empty beat it keeps its number.
        let into = carry(&c, &whole, Q, |_| true).unwrap();
        assert!(into.edit.add.iter().all(|n| n.group == 129));
        // Two notes of the chord copied: they decide alone.
        let part = copy(&c, &[0, 1]);
        assert!(part.notes.iter().all(|n| n.group == 0));
        // Into a clip of pad hits nothing grouped goes (and notes don't
        // go there at all).
        let mut hits = clip(&[]);
        hits.holds = Holds::Hits;
        assert_eq!(carry(&hits, &whole, 0, |_| true), Err(CarryError::Holds));
    }

    #[test]
    fn notes_copied_carry_their_pressure_and_the_locks_their_track_has() {
        use mpfree_notefx::fx::{Fx, Velocity};
        let p = |at, value| TouchPoint { at, value };
        let cut = RelAddr::new(OnTrack::Inst, 1);
        let level = RelAddr::new(OnTrack::Strip, 0);
        let mut a = clip(&[(960, 240, 36), (1200, 480, 38), (2000, 240, 40)]);
        a.notes[0].touch = a.add_touch(&[p(0, 20), p(100, 90)]).unwrap();
        for l in [
            Lock::new(960, Some(36), cut, 800.0),
            Lock::new(1200, Some(38), level, 0.5),
            Lock::new(960, None, cut, 300.0),
            Lock::new(2000, Some(40), level, 0.1),
        ] {
            a.add_lock(l).unwrap();
        }
        let copied = copy(&a, &[1, 0, 1]);
        assert_eq!(
            copied
                .notes
                .iter()
                .map(|n| (n.tick, n.note))
                .collect::<Vec<_>>(),
            [(0, 36), (240, 38)],
            "from the first start, in the clip's order, once each"
        );
        assert_eq!(copied.touch, [p(0, 20), p(100, 90)]);
        assert_eq!(
            copied.locks,
            [
                Lock::new(0, Some(36), cut, 800.0),
                Lock::new(240, Some(38), level, 0.5)
            ],
            "only the locks the notes own"
        );
        assert_eq!(copy(&a, &[9]), Copied::default());
        // A chord's shared run is copied once, and goes in once.
        let mut chord = clip(&[(0, 480, 60), (0, 480, 64)]);
        let run = chord.add_touch(&[p(0, 70)]).unwrap();
        chord.notes[0].touch = run;
        chord.notes[1].touch = run;
        let both = copy(&chord, &[0, 1]);
        assert_eq!(
            (both.touch.len(), both.notes[0].touch),
            (1, both.notes[1].touch)
        );
        let into = carry(&chord, &both, 960, |_| true).unwrap();
        assert_eq!(into.edit.touch, [p(0, 70)]);
        // Pad hits go only into pad hits, notes into notes; a pad's own
        // lock stays out of a clip of notes.
        let mut notes = clip(&[]);
        notes.holds = Holds::Notes;
        assert_eq!(carry(&notes, &both, 0, |_| true), Err(CarryError::Holds));
        let mut tune = chord.clone();
        tune.add_lock(Lock::new(
            0,
            Some(60),
            RelAddr::new(OnTrack::Pad(2), 0),
            3.0,
        ))
        .unwrap();
        let mut held = copy(&tune, &[0]);
        held.holds = Holds::Notes;
        let into = carry(&notes, &held, 0, |_| true).unwrap();
        assert_eq!((into.edit.add_locks.len(), into.left.len()), (0, 1));

        // Into another track's clip, near its end: the second note wraps.
        let mut b = clip(&[(480, 240, 50)]);
        b.notes[0].touch = b.add_touch(&[p(0, 1), p(10, 2), p(20, 3)]).unwrap();
        let there = Lock::new(0, Some(38), level, 0.9);
        b.add_lock(there).unwrap();
        let fx = TrackFx::default();
        let other = |l: &Lock| same_setting(false, at(&fx), at(&fx), l.addr);
        let c = carry(&b, &copied, 3600, other).unwrap();
        assert_eq!(
            c.left,
            [Lock::new(3600, Some(36), cut, 800.0)],
            "the synth's"
        );
        assert_eq!(c.edit.erase_locks, [there], "its place's lock goes");
        assert_eq!(c.edit.touch_at, 3);
        let d = applied(&b, &c.edit);
        let pasted = |d: &Clip, n: u8| {
            let x = by(d, n);
            (x.tick, x.len, d.touch_of(&x).to_vec())
        };
        assert_eq!(pasted(&d, 36), (3600, 240, vec![p(0, 20), p(100, 90)]));
        assert_eq!(pasted(&d, 38), (0, 480, vec![]));
        assert_eq!(
            pasted(&d, 50),
            (480, 240, vec![p(0, 1), p(10, 2), p(20, 3)])
        );
        assert_eq!(d.locks, [Lock::new(0, Some(38), level, 0.5)]);
        let mut back = d.clone();
        back.apply(&c.edit.inverse()).unwrap();
        assert_eq!((back.notes.len(), back.locks.clone()), (1, vec![there]));
        // Into the same track's clip, every lock comes.
        let c = carry(&b, &copied, 0, |l| {
            same_setting(true, at(&fx), at(&fx), l.addr)
        })
        .unwrap();
        assert!(c.left.is_empty());
        assert_eq!(c.edit.add_locks.len(), 2);

        // No room: nothing goes in.
        assert_eq!(
            carry(&b, &Copied::default(), 0, other),
            Err(CarryError::Nothing)
        );
        let mut full = b.clone();
        full.touch
            .resize(crate::MAX_TOUCH_POINTS - 1, TouchPoint::default());
        assert_eq!(carry(&full, &copied, 0, other), Err(CarryError::Pressure));
        let mut full = b.clone();
        full.locks = (0..MAX_LOCKS as Tick)
            .map(|t| Lock::new(t % 3840, Some((t / 3840) as u8), level, 0.0))
            .collect();
        full.locks[0] = Lock::new(240, Some(38), level, 0.0);
        assert_eq!(carry(&full, &copied, 1000, other), Err(CarryError::Locks));
        assert!(
            carry(&full, &copied, 0, other).is_ok(),
            "in place of its own"
        );
        let mut full = b.clone();
        full.notes = vec![Note::new(0, 1, 60, 100); MAX_NOTES - 1];
        assert_eq!(carry(&full, &copied, 0, other), Err(CarryError::Notes));

        // What another track has alike, and its MIDI effects by id.
        for on in [OnTrack::Strip, OnTrack::Seq, OnTrack::Arp] {
            assert!(
                same_setting(false, at(&fx), at(&fx), RelAddr::new(on, 3)),
                "{on:?}"
            );
        }
        for on in [
            OnTrack::Inst,
            OnTrack::Pad(2),
            OnTrack::Insert(1),
            OnTrack::Midi,
        ] {
            assert!(
                !same_setting(false, at(&fx), at(&fx), RelAddr::new(on, 3)),
                "{on:?}"
            );
            assert!(
                same_setting(true, at(&fx), at(&fx), RelAddr::new(on, 3)),
                "{on:?}"
            );
        }
        let mut vel = TrackFx::default();
        vel.add(Fx::Velocity(Velocity::default())).unwrap();
        let mut moved = TrackFx::default();
        moved.add(Fx::Transpose { semitones: 2 }).unwrap();
        let fx1 = RelAddr::new(OnTrack::NoteFx(1), 0);
        assert!(same_setting(false, at(&vel), at(&vel.clone()), fx1));
        assert!(
            !same_setting(false, at(&vel), at(&moved), fx1),
            "another kind"
        );
        assert!(!same_setting(false, at(&vel), at(&fx), fx1), "none there");
        assert!(!same_setting(false, at(&fx), at(&fx), fx1));

        // An instrument lock goes to a track that plays the same kind of
        // instrument, its control named the same (refactor F1).
        let poly = |name: &str| Inst {
            node: "mpfree.poly".into(),
            name: "Poly".into(),
            params: vec![(3, name.into())],
        };
        let fm = Inst {
            node: "mpfree.fm".into(),
            name: "FM".into(),
            params: vec![(3, "Ratio".into())],
        };
        let inst = |made: Option<&Inst>, now: Option<&Inst>, same| {
            let side = |inst| TrackSettings { fx: &fx, inst };
            same_setting(same, side(made), side(now), RelAddr::new(OnTrack::Inst, 3))
        };
        let (cutoff, drive) = (poly("Cutoff"), poly("Drive"));
        let unnamed = Inst {
            params: vec![],
            ..cutoff.clone()
        };
        assert!(
            inst(Some(&cutoff), Some(&poly("Cutoff")), false),
            "the same synth"
        );
        assert!(!inst(Some(&cutoff), Some(&fm), false), "another synth");
        assert!(
            !inst(Some(&cutoff), Some(&fm), true),
            "nor on its own track, now playing another"
        );
        assert!(
            !inst(Some(&cutoff), Some(&drive), false),
            "a control renamed"
        );
        assert!(
            inst(Some(&unnamed), Some(&cutoff), false),
            "a name not known"
        );
        assert!(
            !inst(None, Some(&cutoff), false) && inst(None, Some(&cutoff), true),
            "made for what isn't known: its own track only"
        );
        assert!(!inst(Some(&cutoff), None, false) && inst(Some(&cutoff), None, true));
    }

    /// Chords (plan 2B.2): notes that sound together are a chord, a
    /// strummed one too, and the next chord starts as it ends; strum
    /// starts each note a gap after the one under it (or over it), ends
    /// staying, and strummed back it is as it was; invert and spread move
    /// notes by octaves, chord by chord; a chord drawn goes in whole.
    #[test]
    fn a_drawn_chord_rolls_as_one_and_a_strummed_one_note_by_note() {
        let mut c = clip(&[(0, Q, 48)]);
        c.holds = Holds::Notes;
        let chord = |tick: Tick| -> Vec<Note> {
            [60, 64, 67]
                .iter()
                .map(|&k| Note::new(tick, Q, k, 100))
                .collect()
        };
        // Drawn together, the chord's notes are one trigger.
        let e = add_all(&c, &chord(0));
        assert!(e
            .add
            .iter()
            .all(|n| n.group == e.add[0].group && n.group != 0));
        let mut d = c.clone();
        d.apply(&e).unwrap();
        assert!(d.groups_ok());
        let g = e.add[0].group;
        assert_eq!(d.trigger(0, g).len(), 3);
        // A second chord drawn on the same tick is a trigger of its own.
        let e2 = add_all(&d, &[Note::new(0, Q, 72, 100), Note::new(0, Q, 76, 100)]);
        assert!(e2.add.iter().all(|n| n.group != g && n.group != 0));
        // One note drawn alone decides alone, as before.
        assert_eq!(add_all(&c, &[Note::new(Q, Q, 50, 100)]).add[0].group, 0);
        // In a clip of pad hits nothing is grouped.
        let mut hits = clip(&[]);
        hits.holds = Holds::Hits;
        assert!(add_all(&hits, &chord(0)).add.iter().all(|n| n.group == 0));
        // Strummed, the chord's notes start apart and each rolls alone:
        // no tick holds more than one of them in a trigger.
        let sel: Vec<usize> = (0..d.notes.len())
            .filter(|&i| d.notes[i].group == g)
            .collect();
        let e = strum(&d, &sel, 20, None);
        let mut s = d.clone();
        s.apply(&e).unwrap();
        assert!(s.groups_ok());
        for n in s.notes.iter().filter(|n| n.note >= 60) {
            assert!(
                n.group == 0 || s.trigger(n.tick, n.group).len() == 1,
                "{n:?}"
            );
        }
    }

    #[test]
    fn chords_strum_invert_and_spread_chord_by_chord() {
        // C E G for a beat, then F A C, and a strummed D F A.
        let c = clip(&[
            (0, 960, 60),
            (0, 960, 64),
            (0, 960, 67),
            (960, 960, 65),
            (960, 960, 69),
            (960, 960, 72),
            (1920, 960, 62),
            (1950, 930, 66),
            (1980, 900, 70),
        ]);
        let all: Vec<usize> = (0..c.notes.len()).collect();
        let names = |c: &Clip, chord: &[usize]| -> Vec<u8> {
            chord.iter().map(|&i| c.notes[i].note).collect()
        };
        let chords = chords_of(&c, &all);
        assert_eq!(chords.len(), 3);
        assert_eq!(names(&c, &chords[0]), [60, 64, 67]);
        assert_eq!(names(&c, &chords[2]), [62, 66, 70]);
        assert_eq!(strum_of(&c, &chords[0]), 0);
        assert_eq!(strum_of(&c, &chords[2]), 30);
        // Strum up 20: the first chord 0, 20, 40, the third 50 apart.
        let d = applied(&c, &strum(&c, &all, 20, None));
        assert_eq!(
            (by(&d, 60).tick, by(&d, 64).tick, by(&d, 67).tick),
            (0, 20, 40)
        );
        assert_eq!(by(&d, 67).tick + by(&d, 67).len, 960, "the end stays");
        assert_eq!((by(&d, 66).tick, by(&d, 70).tick), (1970, 2020));
        // Down: the highest first.
        let d = applied(&c, &strum(&c, &all[..3], 20, Some(true)));
        assert_eq!(
            (by(&d, 67).tick, by(&d, 64).tick, by(&d, 60).tick),
            (0, 20, 40)
        );
        // Back to 0: as it was.
        let up = applied(&c, &strum(&c, &all[..3], 20, None));
        let all_up: Vec<usize> = (0..up.notes.len()).collect();
        let back = applied(&up, &strum(&up, &all_up, -20, None));
        for n in [60, 64, 67] {
            assert_eq!(by(&back, n), by(&c, n));
        }
        // Closer than together stops there; turned over, down keeps the
        // gap.
        let flat = applied(&up, &strum(&up, &all_up, -50, None));
        assert_eq!(strum_of(&flat, &chords_of(&flat, &all_up)[0]), 0);
        let over = applied(&up, &strum(&up, &all_up, 0, Some(true)));
        let chord = &chords_of(&over, &all_up)[0];
        assert_eq!(strum_of(&over, chord), -20);
        // A strum past a note's length is refused whole.
        assert!(strum(&c, &all, 500, None).add.is_empty());
        // A note alone doesn't strum.
        assert!(strum(&c, &[0], 20, None).add.is_empty());
        // Invert: C E G to E G C, and back down.
        let d = applied(&c, &invert(&c, &all[..3], 1));
        let mut got: Vec<u8> = d
            .notes
            .iter()
            .filter(|n| n.tick == 0)
            .map(|n| n.note)
            .collect();
        got.sort_unstable();
        assert_eq!(got, [64, 67, 72]);
        let d = applied(&c, &invert(&c, &all[..3], -1));
        let mut got: Vec<u8> = d
            .notes
            .iter()
            .filter(|n| n.tick == 0)
            .map(|n| n.note)
            .collect();
        got.sort_unstable();
        assert_eq!(got, [55, 60, 64]);
        assert!(invert(&c, &[0], 1).add.is_empty(), "a note alone stays");
        // Spread: C G E', then closed again.
        let d = applied(&c, &spread(&c, &all[..3], 1));
        let mut got: Vec<u8> = d
            .notes
            .iter()
            .filter(|n| n.tick == 0)
            .map(|n| n.note)
            .collect();
        got.sort_unstable();
        assert_eq!(got, [60, 67, 76]);
        let d_all: Vec<usize> = (0..d.notes.len()).collect();
        let d0: Vec<usize> = d_all
            .iter()
            .copied()
            .filter(|&i| d.notes[i].tick == 0)
            .collect();
        let e = applied(&d, &spread(&d, &d0, -1));
        let mut got: Vec<u8> = e
            .notes
            .iter()
            .filter(|n| n.tick == 0)
            .map(|n| n.note)
            .collect();
        got.sort_unstable();
        assert_eq!(got, [60, 64, 67]);
        // Past 127, refused whole.
        let high = clip(&[(0, 960, 120), (0, 960, 124), (0, 960, 127)]);
        assert!(invert(&high, &[0, 1, 2], 1).add.is_empty());
        // A chord drawn goes in whole, or not at all past the clip's end.
        let chord = [Note::new(0, 240, 50, 90), Note::new(0, 240, 53, 90)];
        assert_eq!(add_all(&c, &chord).add, chord);
        assert!(add_all(&c, &[Note::new(4 * 960, 240, 50, 90)])
            .add
            .is_empty());
    }

    /// The pads write in place of a note on the same tick and pitch (plan
    /// 1C.2): hit again, a note changes rather than doubles; its lock
    /// stays; a pad past the clip's end, or nothing, writes nothing.
    #[test]
    fn a_pad_writes_in_place_of_a_note_on_its_line() {
        let mut c = clip(&[(0, 240, 36), (0, 480, 38), (480, 240, 36)]);
        c.add_lock(Lock::new(0, Some(36), RelAddr::new(OnTrack::Inst, 1), 0.5))
            .unwrap();
        let e = write(&c, &[Note::new(0, 120, 36, 70)]);
        assert_eq!(e.erase, [Note::new(0, 240, 36, 100)]);
        assert!(e.erase_locks.is_empty(), "its lock stays");
        let d = applied(&c, &e);
        assert_eq!(d.notes.len(), 3);
        assert!(d.notes.contains(&Note::new(0, 120, 36, 70)));
        assert!(d.notes.contains(&Note::new(480, 240, 36, 100)));
        // A chord: each of its notes in place of its own.
        let e = write(&c, &[Note::new(0, 240, 38, 90), Note::new(0, 240, 42, 90)]);
        assert_eq!(e.erase, [Note::new(0, 480, 38, 100)]);
        assert_eq!(e.add.len(), 2);
        assert!(write(&c, &[Note::new(4 * 960, 240, 36, 90)]).is_empty());
        assert!(write(&c, &[]).is_empty());
    }

    /// The pads write their trigger groups (#827): in a clip of notes a
    /// chord pad's notes are its trigger, its number plus one, as a
    /// recorded hit's are; written again on its step, a note joins what
    /// is left of the trigger there and takes its chance; another pad's
    /// notes on the step are a trigger of their own; a clip of pad hits
    /// has no groups.
    #[test]
    fn a_pad_writes_its_chord_as_one_trigger() {
        let pad = |tick, note, g: u8| Note {
            group: g,
            ..Note::new(tick, 240, note, 90)
        };
        let g = crate::groups::pad_group(4);
        let mut c = clip(&[]);
        c.holds = Holds::Notes;
        let chord = [pad(0, 60, g), pad(0, 64, g), pad(0, 67, g)];
        let mut d = applied(&c, &write(&c, &chord));
        assert!(d.notes.iter().all(|n| n.group == g));
        // A chance on the trigger (all its notes), then the pad's E again
        // with a softer hit: it joins the trigger and takes its chance.
        for n in d.notes.iter_mut() {
            n.chance = 40;
        }
        d.check().unwrap();
        let e = write(&d, &[pad(0, 64, g)]);
        assert_eq!(e.erase.len(), 1);
        assert_eq!((e.add[0].group, e.add[0].chance), (g, 40));
        let d = applied(&d, &e);
        assert_eq!(d.check(), Ok(()));
        // Another pad on the step: its own trigger, its own chance.
        let h = crate::groups::pad_group(5);
        let e = write(&d, &[pad(0, 72, h)]);
        assert_eq!((e.add[0].group, e.add[0].chance), (h, 100));
        assert_eq!(applied(&d, &e).check(), Ok(()));
        // A clip of pad hits: none.
        let mut hits = clip(&[]);
        hits.holds = Holds::Hits;
        let e = write(&hits, &[pad(0, 4, g)]);
        assert_eq!(e.add[0].group, 0);
    }

    /// Chop cuts each picked note into equal pieces, each with its part
    /// of the pressure from the value held at its cut (Reset: none);
    /// Legato runs each note toward the next start by a share of the gap.
    #[test]
    fn chop_shares_the_pressure_and_legato_fills_part_of_the_gap() {
        use crate::TouchPoint;
        let p = |at, value| TouchPoint { at, value };
        let mut c = clip(&[(0, 960, 60), (960, 240, 62)]);
        let run = c
            .add_touch(&[p(0, 10), p(100, 40), p(500, 90), p(700, 20)])
            .unwrap();
        c.notes[0].touch = run;
        type Runs = Vec<(Tick, Tick, Vec<(u32, u8)>)>;
        let runs = |d: &Clip| -> Runs {
            let mut r: Vec<_> = d
                .notes
                .iter()
                .filter(|n| n.note == 60)
                .map(|n| {
                    (
                        n.tick,
                        n.len,
                        d.touch_of(n).iter().map(|q| (q.at, q.value)).collect(),
                    )
                })
                .collect();
            r.sort();
            r
        };
        // Four pieces of 240: the first keeps the note's run; the second
        // holds 40 (since 100); the third starts at 40, then 90 and 20 at
        // 500 and 700 (20 and 220 into it); the last holds 20.
        let d = applied(&c, &chop(&c, &[0], 4, false));
        assert_eq!(
            runs(&d),
            [
                (0, 240, vec![(0, 10), (100, 40), (500, 90), (700, 20)]),
                (240, 240, vec![(0, 40)]),
                (480, 240, vec![(0, 40), (20, 90), (220, 20)]),
                (720, 240, vec![(0, 20)]),
            ]
        );
        assert_eq!(d.touch.len(), 4 + 1 + 3 + 1, "new runs at the end");
        // A point on a cut starts that piece by itself.
        let mut e = clip(&[(0, 960, 60)]);
        e.notes[0].touch = e.add_touch(&[p(0, 5), p(480, 70)]).unwrap();
        let f = applied(&e, &chop(&e, &[0], 2, false));
        assert_eq!(runs(&f)[1], (480, 480, vec![(0, 70)]));
        // Reset: no piece carries pressure; the other note is left.
        let d = applied(&c, &chop(&c, &[0], 3, true));
        assert!(runs(&d).iter().all(|(_, _, r)| r.is_empty()));
        assert_eq!(d.notes.len(), 4);
        assert_eq!(d.touch.len(), 4, "nothing added");
        // The last piece takes what is left over.
        let d = applied(&c, &chop(&c, &[1], 7, false));
        let mut lens: Vec<Tick> = d
            .notes
            .iter()
            .filter(|n| n.note == 62)
            .map(|n| n.len)
            .collect();
        lens.sort_unstable();
        assert_eq!(lens, [34, 34, 34, 34, 34, 34, 36]);
        // Refused: one piece, past the most, a note shorter than its
        // pieces.
        assert!(chop(&c, &[0], 1, false).is_empty());
        assert!(chop(&c, &[0], MAX_PIECES + 1, false).is_empty());
        let short = clip(&[(0, 3, 60)]);
        assert!(chop(&short, &[0], 4, false).is_empty());

        // Legato: C (0, 240) and E (0, 120) toward G at 960; G alone
        // after stays.
        let c = clip(&[(0, 240, 60), (0, 120, 64), (960, 240, 67), (480, 960, 69)]);
        let all: Vec<usize> = (0..c.notes.len()).collect();
        let d = applied(&c, &legato(&c, &all, 50));
        // The next start after 0 is A's 480: C 240 + 120, E 120 + 180.
        assert_eq!((by(&d, 60).len, by(&d, 64).len), (360, 300));
        // A ran past G's start: half the way back.
        assert_eq!(by(&d, 69).len, 960 - 240);
        assert_eq!(by(&d, 67), by(&c, 67), "the last keeps its length");
        let d = applied(&c, &legato(&c, &all, 100));
        assert_eq!(
            (by(&d, 60).len, by(&d, 64).len, by(&d, 69).len),
            (480, 480, 480)
        );
        assert!(legato(&c, &all, 0).is_empty());
        // Only the picks count: C alone has no next start.
        assert!(legato(&c, &[0], 100).is_empty());
    }

    /// Chop and trigger groups (#827): a chord that rolls as one, chopped
    /// whole, rolls as one at each piece; one of its notes chopped alone
    /// keeps its trigger in its first piece, and its other pieces decide
    /// alone. A clip of pad hits has no groups.
    #[test]
    fn a_chopped_trigger_stays_one_at_each_piece() {
        let mut c = clip(&[(0, 960, 60), (0, 960, 64), (0, 960, 67)]);
        c.holds = Holds::Notes;
        for n in c.notes.iter_mut() {
            n.group = 129;
            n.chance = 50;
        }
        c.check().unwrap();
        let groups = |d: &Clip| -> Vec<(Tick, u8, u8)> {
            let mut g: Vec<_> = d.notes.iter().map(|n| (n.tick, n.note, n.group)).collect();
            g.sort_unstable();
            g
        };
        let d = applied(&c, &chop(&c, &[0, 1, 2], 2, false));
        let mut want = Vec::new();
        for tick in [0, 480] {
            for note in [60, 64, 67] {
                want.push((tick, note, 129));
            }
        }
        assert_eq!(groups(&d), want);
        // E alone: its first piece stays in the chord's trigger.
        let d = applied(&c, &chop(&c, &[1], 2, false));
        assert_eq!(
            groups(&d),
            [(0, 60, 129), (0, 64, 129), (0, 67, 129), (480, 64, 0)]
        );
        assert_eq!(d.check(), Ok(()));
        let mut hits = c.clone();
        hits.holds = Holds::Hits;
        for n in hits.notes.iter_mut() {
            n.group = 0;
        }
        let d = applied(&hits, &chop(&hits, &[0, 1, 2], 2, false));
        assert!(d.notes.iter().all(|n| n.group == 0));
    }

    /// Lock lanes (plan 3A.2): the chord's notes share the step's lock;
    /// a note with its own lock on its pitch reads and sets that one; two
    /// notes of one pitch in one step share it; a pad's setting is only
    /// its pad's; ERASE + a tap takes the lock off; undo puts it back.
    #[test]
    fn locks_on_notes_are_the_steps_on_their_rows() {
        let synth = RelAddr::new(OnTrack::Inst, 7);
        // A chord at step 0 (C, E, and a second E 5 ticks late), G at
        // step 4; a whole-step lock on step 0.
        let mut c = clip(&[(0, 240, 60), (0, 240, 64), (5, 200, 64), (960, 240, 67)]);
        c.add_lock(Lock::new(0, None, synth, 0.3)).unwrap();
        let of = |c: &Clip, i: usize| lock_of(c, &c.notes[i], synth);
        assert_eq!(
            (of(&c, 0), of(&c, 1), of(&c, 3)),
            (Some(0.3), Some(0.3), None)
        );
        assert_eq!(lock_place(&c, &c.notes[0], synth), Some((0, None)));
        // Set on C: the step's lock, which E shares.
        let d = applied(&c, &lock_notes(&c, synth, &[(0, 0.5)]));
        assert_eq!(d.locks.len(), 1);
        assert_eq!(of(&d, 1), Some(0.5), "shared");
        // G's step gets a lock of its own, on its first note's tick.
        let d = applied(&c, &lock_notes(&c, synth, &[(3, 0.8)]));
        assert_eq!((of(&d, 3), d.locks.len()), (Some(0.8), 2));
        assert!(d.locks.iter().any(|l| (l.tick, l.sound) == (960, None)));
        // E's own lock on its pitch: both Es read and set it, C the
        // step's.
        c.add_lock(Lock::new(0, Some(64), synth, 0.9)).unwrap();
        assert_eq!(
            (of(&c, 0), of(&c, 1), of(&c, 2)),
            (Some(0.3), Some(0.9), Some(0.9))
        );
        let d = applied(&c, &lock_notes(&c, synth, &[(2, 0.1)]));
        assert_eq!((of(&d, 0), of(&d, 1)), (Some(0.3), Some(0.1)));
        // Both Es named: one lock, the last value.
        let e = lock_notes(&c, synth, &[(1, 0.2), (2, 0.4)]);
        assert_eq!(e.add_locks.len(), 1);
        assert_eq!(e.add_locks[0].value, 0.4);
        // The same value again: nothing.
        assert!(lock_notes(&c, synth, &[(0, 0.3)]).is_empty());
        // ERASE + a tap on C: the step's lock comes off; the Es keep
        // their own.
        let d = applied(&c, &unlock_notes(&c, synth, &[0]));
        assert_eq!((of(&d, 0), of(&d, 1)), (None, Some(0.9)));
        assert!(unlock_notes(&c, synth, &[3]).is_empty(), "G has none");
        // A pad's setting: only its pad's notes, on its row.
        let pad = RelAddr::new(OnTrack::Pad(2), 1);
        let k = clip(&[(0, 120, 2), (0, 120, 3)]);
        assert_eq!(lock_place(&k, &k.notes[0], pad), Some((0, Some(2))));
        assert_eq!(lock_place(&k, &k.notes[1], pad), None);
        let e = lock_notes(&k, pad, &[(0, 6.0), (1, 9.0)]);
        assert_eq!(e.add_locks, [Lock::new(0, Some(2), pad, 6.0)]);
    }

    /// Pressure drawn on the lane (plan 3A.1): each picked note under
    /// the stroke gets a new run, its own points either side kept, the
    /// value it had at the stroke's end coming back after it; undo puts
    /// the old run back.
    #[test]
    fn pressure_drawn_over_notes_is_a_new_run_and_undo_brings_the_old_back() {
        let p = |at, value| TouchPoint { at, value };
        // The run of the note at pitch `note` (an edit puts its notes in
        // at the end).
        let pts = |d: &Clip, note: u8| -> Vec<(u32, u8)> {
            let n = d.notes.iter().find(|n| n.note == note).unwrap();
            d.touch_of(n).iter().map(|q| (q.at, q.value)).collect()
        };
        // C (0, 960) with pressure; E (960, 480) with none; G unpicked.
        let mut c = clip(&[(0, 960, 60), (960, 480, 64), (0, 1920, 67)]);
        c.notes[0].touch = c
            .add_touch(&[p(0, 10), p(200, 50), p(600, 90), p(800, 30)])
            .unwrap();
        let had = c.touch.len();
        // From 100 to 400 on C: 70 then 80 from 250.
        let e = draw_touch(&c, &[0, 1], &[(100, 70), (250, 80)], 400).unwrap();
        assert_eq!(e.erase.len(), 1, "only C is under the stroke");
        let d = applied(&c, &e);
        assert_eq!(
            pts(&d, 60),
            [
                (0, 10),
                (100, 70),
                (250, 80),
                (400, 50),
                (600, 90),
                (800, 30)
            ],
            "50 (since 200) comes back at the stroke's end"
        );
        assert_eq!(d.touch.len(), had + 6, "a new run, the old one kept");
        let mut back = d.clone();
        back.apply(&e.inverse()).unwrap();
        assert_eq!(pts(&back, 60), pts(&c, 60), "UNDO: the old pressure");
        // Across both notes, from before C's end into E: E had none, so
        // it gets the drawn values and 0 after; a value equal to the one
        // before is left out.
        let e = draw_touch(&c, &[0, 1], &[(900, 40), (1000, 40), (1100, 20)], 1200).unwrap();
        let d = applied(&c, &e);
        assert_eq!(
            pts(&d, 60),
            [(0, 10), (200, 50), (600, 90), (800, 30), (900, 40)]
        );
        // E starts in the first value's span: from 0 into it.
        assert_eq!(pts(&d, 64), [(0, 40), (140, 20), (240, 0)]);
        // The stroke over the whole of E: nothing comes back after it.
        let e = draw_touch(&c, &[1], &[(900, 64)], 1500).unwrap();
        let d = applied(&c, &e);
        assert_eq!(pts(&d, 64), [(0, 64)]);
        // Drawn all 0 over E: no pressure, as it was, so no change; over
        // all of C, its run goes.
        assert!(draw_touch(&c, &[1], &[(960, 0)], 1440).unwrap().is_empty());
        let e = draw_touch(&c, &[0], &[(0, 0)], 960).unwrap();
        let d = applied(&c, &e);
        assert!(d
            .notes
            .iter()
            .find(|n| n.note == 60)
            .unwrap()
            .touch
            .is_none());
        // The same line again changes nothing; no picks, a stroke ending
        // where it starts, or off every note: nothing.
        let e = draw_touch(&c, &[0], &[(200, 50), (600, 90)], 800).unwrap();
        assert!(e.is_empty(), "{e:?}");
        assert!(draw_touch(&c, &[], &[(0, 99)], 960).unwrap().is_empty());
        assert!(draw_touch(&c, &[0], &[(100, 99)], 100).unwrap().is_empty());
        assert!(draw_touch(&c, &[1], &[(0, 99)], 960).unwrap().is_empty());
        // No room for the new runs: refused.
        let mut full = c.clone();
        full.touch.resize(crate::MAX_TOUCH_POINTS, p(0, 0));
        assert_eq!(draw_touch(&full, &[0], &[(100, 99)], 200), None);
    }
}
