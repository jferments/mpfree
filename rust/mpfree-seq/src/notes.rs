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
//! left for that.

use crate::steps::{moves_edit, Edit};
use crate::{
    touch_room, Clip, Lock, Note, Touch, TouchPoint, MAX_CLIP_TICKS, MAX_NOTES, MIN_CLIP_TICKS,
};
use mpfree_transport::time::{Tick, PPQ};

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
/// that stay the same; each note's own locks go with it.
fn replace(clip: &Clip, pairs: Vec<(Note, Note)>) -> Edit {
    let mut e = Edit::default();
    let mut used = vec![false; clip.locks.len()];
    for (old, new) in pairs {
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

/// Set the `sel` notes' velocity (the velocity lane).
pub fn set_velocity(clip: &Clip, sel: &[usize], v: u8) -> Edit {
    map(clip, sel, |n| {
        Some(Note {
            velocity: v.clamp(1, 127),
            ..n
        })
    })
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
    Edit {
        add: vec![note],
        ..Edit::default()
    }
}

/// Copies of the `sel` notes right after them: moved on by their span
/// (first start to last end) rounded up to whole `division`s. Copies that
/// would start past the clip's end wrap.
pub fn duplicate(clip: &Clip, sel: &[usize], division: Tick) -> Edit {
    let notes: Vec<Note> = picked(clip, sel).map(|(_, n)| n).collect();
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

/// Put `notes` (a copy taken from a clip, `from` its first start) in at
/// `at`: each keeps its place from `from`; starts wrap round the clip.
pub fn paste(clip: &Clip, notes: &[Note], at: Tick, from: Tick) -> Edit {
    if clip.notes.len() + notes.len() > MAX_NOTES {
        return Edit::default();
    }
    Edit {
        add: notes
            .iter()
            .map(|n| Note {
                tick: wrap(n.tick - from + at, clip.length),
                ..*n
            })
            .collect(),
        ..Edit::default()
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
    for n in notes {
        e.erase.push(n);
        let each = n.len / parts;
        for k in 0..parts {
            let len = if k == parts - 1 {
                n.len - each * k
            } else {
                each
            };
            e.add.push(Note {
                tick: wrap(n.tick + each * k, clip.length),
                len,
                // The first part keeps the pressure (what falls past its
                // end does not play); the rest would replay its start.
                touch: if k == 0 { n.touch } else { Touch::NONE },
                ..n
            });
        }
    }
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
    map(clip, sel, |n| {
        let (a, b, c) = (rand(), rand(), rand());
        let dt = ((a + lean) / (1.0 + lean.abs()) * h.time as f64).round() as Tick;
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
}
