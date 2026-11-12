//! The step sequencer's view of a clip (roadmap T7).
//!
//! A [`Grid`] cuts a clip into steps of one size (1/4 to 1/32, triplets
//! too), 16 to a page. A step of a sound (a pad or a note number) is the
//! notes of that sound nearest the step's grid line:
//!
//! - A note belongs to step `i` when its tick is from half a step before
//!   `i * size` to just under half a step after it. How far it is from the
//!   line is its **nudge**.
//! - A note half a step or more after the last step's line belongs to step
//!   0, early: it plays just before the clip comes round again. (In a clip
//!   a whole number of steps long, that is the last half step.)
//! - The step shows the first of its notes (velocity, length, nudge,
//!   chance) and how many there are; a roll recorded live puts several
//!   notes in one step.
//!
//! Steps hold no data of their own. They are the clip's notes, so notes
//! recorded live show up on the steps, and a step entered here plays like
//! any recorded note (quantise and swing still apply on playback).
//!
//! **Pages.** A clip has as many steps as its length holds, so as many
//! pages as it needs: a new step clip is one page ([`Grid::page_length`]),
//! [`Grid::add_page`] adds 16 steps (empty, or repeating the page before
//! them), [`Grid::remove_page`] takes the last 16 off, [`Grid::resize`]
//! sets any number of steps (13 steps for polymeter), and
//! [`Grid::copy_page`] copies one page over another.
//!
//! **Chords.** Every sound has its own steps, so a chord is several sounds
//! on one step. [`Grid::chord`] reads a step across all sounds, and
//! [`Grid::enter`] puts a held chord on a step (step entry).
//!
//! **Parameter locks.** [`Grid::lock`] sets a parameter on a step of a
//! row (a pad or note number), or of the whole step for a chord; the lock
//! takes the tick of the step's first note, so it plays with it, and a step
//! with no note takes a trigless lock on its line. [`Grid::locks`] reads
//! them and [`Grid::lock_row`] counts them for the lock dots. Locks go
//! with their step: tapping a step off takes its row's locks out, a nudge
//! moves them, and pages added, copied or taken off carry them.
//!
//! **Euclidean fill.** [`Euclid`] spreads `hits` as evenly as they go
//! over `steps` steps (the Euclidean rhythms, as on the Digitakt), turned
//! on by `rotate`; [`Grid::euclid`] writes that pattern into one sound's
//! row, repeated from the clip's first step, as one edit.
//!
//! Edits never change the clip themselves: they return an [`Edit`], the
//! notes and locks to take out and to put in, for the control thread to
//! apply as one undoable take (`Takes::edit` in `mpfree-play`).

use crate::{
    touch_room, Clip, Cond, Lock, Note, TouchPoint, MAX_CLIP_TICKS, MAX_RATCHET, MIN_CLIP_TICKS,
};
use mpfree_moves::{Moves, MovesError};
use mpfree_param::RelAddr;
use mpfree_transport::time::{Tick, PPQ};

/// Steps on a page.
pub const PAGE: usize = 16;

/// The step sizes, in ticks (960 to a beat): 1/4, 1/8, 1/8 triplet, 1/16,
/// 1/16 triplet and 1/32.
pub const SIZES: [Tick; 6] = [960, 480, 320, 240, 160, 120];

/// A clip cut into steps of one size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Grid {
    size: Tick,
}

/// What a step shows: its first note, and how many notes it holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Step {
    pub velocity: u8,
    /// In the clip's ticks.
    pub len: Tick,
    /// Ticks from the step's line: negative is early.
    pub nudge: Tick,
    /// Percent, 1 to 100.
    pub chance: u8,
    /// The trig condition (shown as `cond.to_string()`: "1:2", "FILL" ...).
    pub cond: Cond,
    /// Hits, 1 to `MAX_RATCHET`.
    pub ratchet: u8,
    /// Notes of the sound in the step, 1 or more.
    pub notes: usize,
}

/// A change to a clip: take `erase` and `erase_locks` out, change the
/// length from and to `length` when there is one, add `touch` to the
/// clip's pressure, put `add` and `add_locks` in, then change the moves
/// when `moves` says so.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Edit {
    pub erase: Vec<Note>,
    pub add: Vec<Note>,
    pub length: Option<(Tick, Tick)>,
    pub erase_locks: Vec<Lock>,
    pub add_locks: Vec<Lock>,
    /// A page added, taken off or copied takes the clip's moves along.
    pub moves: Option<Box<MovesEdit>>,
    /// Pressure runs for the new notes (half and double speed rescale
    /// their notes' pressure), added to the end of the clip's pressure,
    /// which is `touch_at` values long when the edit is made. The clip's
    /// pressure only grows, so undo and redo leave it: they take out and
    /// put back notes that name these runs.
    pub touch: Vec<TouchPoint>,
    pub touch_at: u32,
}

/// The clip's moves before and after an [`Edit`]: kept whole, since a
/// page edit moves every lane's points about (it is small next to the
/// clip's notes, and made on the control thread).
#[derive(Clone, Debug, PartialEq)]
pub struct MovesEdit {
    pub before: Moves,
    pub after: Moves,
}

// A clip's moves hold only finite values (`Moves::check`), so equality is
// total.
impl Eq for MovesEdit {}

/// The edit of `clip`'s moves that `f` makes, or None when it has none,
/// makes no change or is refused (then the moves stay as they are).
pub(crate) fn moves_edit(
    clip: &Clip,
    f: impl FnOnce(&mut Moves) -> Result<(), MovesError>,
) -> Option<Box<MovesEdit>> {
    if clip.moves.is_empty() {
        return None;
    }
    let mut after = clip.moves.clone();
    if f(&mut after).is_err() || after == clip.moves {
        return None;
    }
    Some(Box::new(MovesEdit {
        before: clip.moves.clone(),
        after,
    }))
}

impl Edit {
    pub fn is_empty(&self) -> bool {
        self.erase.is_empty()
            && self.add.is_empty()
            && self.length.is_none()
            && self.erase_locks.is_empty()
            && self.add_locks.is_empty()
            && self.moves.is_none()
            && self.touch.is_empty()
    }

    /// The edit that puts the clip back as it was.
    pub fn inverse(&self) -> Edit {
        Edit {
            erase: self.add.clone(),
            add: self.erase.clone(),
            length: self.length.map(|(a, b)| (b, a)),
            erase_locks: self.add_locks.clone(),
            add_locks: self.erase_locks.clone(),
            moves: self.moves.as_ref().map(|m| {
                Box::new(MovesEdit {
                    before: m.after.clone(),
                    after: m.before.clone(),
                })
            }),
            // The pressure stays: the clip's only grows.
            touch: Vec::new(),
            touch_at: 0,
        }
    }

    /// Both edits made from the same clip: `other`'s new pressure runs go
    /// after this one's, and its notes name them there.
    fn join(&mut self, mut other: Edit) {
        if self.touch.is_empty() {
            self.touch_at = other.touch_at;
        } else if !other.touch.is_empty() {
            let (from, by) = (other.touch_at, self.touch.len() as u32);
            for n in &mut other.add {
                if !n.touch.is_none() && n.touch.first >= from {
                    n.touch.first += by;
                }
            }
        }
        self.touch.extend(other.touch);
        self.erase.extend(other.erase);
        self.add.extend(other.add);
        self.erase_locks.extend(other.erase_locks);
        self.add_locks.extend(other.add_locks);
        self.moves = match (self.moves.take(), other.moves) {
            (Some(mut a), Some(b)) => {
                a.after = b.after;
                Some(a)
            }
            (a, b) => a.or(b),
        };
    }

    /// Take `old` out and put `new` in, unless they are the same.
    fn move_lock(&mut self, old: Lock, new: Lock) {
        if old != new {
            self.erase_locks.push(old);
            self.add_locks.push(new);
        }
    }
}

/// What to change on a step. Every note in the step takes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// 1 to 127.
    Velocity(u8),
    /// In the clip's ticks, at least 1.
    Length(Tick),
    /// Ticks from the step's line, kept within the step. The notes move
    /// together, so the first one lands here.
    Nudge(Tick),
    /// Percent, 1 to 100.
    Chance(u8),
    /// A trig condition; one out of range leaves the step as it is.
    Cond(Cond),
    /// Hits, kept within 1 to `MAX_RATCHET`.
    Ratchet(u8),
}

impl Change {
    /// A key naming this kind of change on step `i` of sound `pitch`, so
    /// that a Q-Link turned through many values joins one undo (see
    /// `Takes::edit` in `mpfree-play`).
    pub fn key(self, pitch: u8, i: usize) -> u64 {
        let kind = match self {
            Change::Velocity(_) => 1,
            Change::Length(_) => 2,
            Change::Nudge(_) => 3,
            Change::Chance(_) => 4,
            Change::Cond(_) => 6,
            Change::Ratchet(_) => 7,
        };
        (kind << 56) | (u64::from(pitch) << 48) | (i as u64 & 0xFFFF_FFFF_FFFF)
    }

    /// The key for this kind of change on every sound of step `i`
    /// ([`Grid::change_all`]).
    pub fn key_all(self, i: usize) -> u64 {
        self.key(ALL, i)
    }
}

/// A key naming a lock of `addr` on step `i` of row `sound`, so that a
/// Q-Link turned through many values on a held step joins one undo (as
/// [`Change::key`] does for a step's velocity).
pub fn lock_key(sound: Option<u8>, i: usize, addr: RelAddr) -> u64 {
    use mpfree_param::OnTrack;
    let on: u64 = match addr.on {
        OnTrack::Inst => 1,
        OnTrack::Strip => 2,
        OnTrack::Arp => 3,
        OnTrack::Seq => 4,
        OnTrack::Pad(p) => 0x100 | u64::from(p),
        OnTrack::Insert(uid) => 0x1_0000 | u64::from(uid),
    };
    let a = (on.wrapping_mul(0x9E37_79B9) ^ u64::from(addr.param)) & 0xFF_FFFF;
    (5 << 56) | (u64::from(sound.unwrap_or(ALL)) << 48) | (a << 24) | (i as u64 & 0xFF_FFFF)
}

/// No sound has this number: it names every sound of a step in a key.
const ALL: u8 = 255;

/// The most steps a Euclidean pattern spans: four pages.
pub const MAX_EUCLID_STEPS: u8 = 64;

/// A Euclidean rhythm: `hits` spread as evenly as they go over `steps`
/// steps, the first on step 0, then the whole pattern turned `rotate`
/// steps later. 3 of 8 is x..x..x. and 4 of 16 is a hit every beat.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Euclid {
    /// 0 to `steps`.
    pub hits: u8,
    /// 1 to [`MAX_EUCLID_STEPS`].
    pub steps: u8,
    /// Steps later, 0 to `steps - 1`.
    pub rotate: u8,
}

impl Default for Euclid {
    /// A hit every beat in 1/16 steps.
    fn default() -> Euclid {
        Euclid {
            hits: 4,
            steps: 16,
            rotate: 0,
        }
    }
}

impl Euclid {
    /// In range: steps within 1 to [`MAX_EUCLID_STEPS`], hits no more than
    /// the steps, rotate below them.
    pub fn fit(self) -> Euclid {
        let steps = self.steps.clamp(1, MAX_EUCLID_STEPS);
        Euclid {
            hits: self.hits.min(steps),
            steps,
            rotate: self.rotate % steps,
        }
    }

    /// Whether step `i` of the pattern (any step: it repeats) has a hit.
    /// Step `j` before the rotation has one when `j * hits mod steps` is
    /// below `hits`: Bresenham's line, which spreads the hits as evenly as
    /// the Euclidean algorithm does and starts on one.
    pub fn hit(self, i: usize) -> bool {
        let e = self.fit();
        let (k, n) = (usize::from(e.hits), usize::from(e.steps));
        let j = (i % n + n - usize::from(e.rotate)) % n;
        j * k % n < k
    }

    /// The pattern as text, x for a hit and . for a rest (for the log).
    pub fn pattern(self) -> String {
        let e = self.fit();
        (0..usize::from(e.steps))
            .map(|i| if e.hit(i) { 'x' } else { '.' })
            .collect()
    }
}

impl Grid {
    /// Steps of `size` ticks, from a 1/32 to a whole note.
    pub fn new(size: Tick) -> Option<Grid> {
        (MIN_CLIP_TICKS..=4 * Tick::from(PPQ))
            .contains(&size)
            .then_some(Grid { size })
    }

    pub fn size(self) -> Tick {
        self.size
    }

    /// One page of steps, in ticks: the length of a new step clip.
    pub fn page_length(self) -> Tick {
        self.size * PAGE as Tick
    }

    /// The most steps a clip can have.
    pub fn max_steps(self) -> usize {
        (MAX_CLIP_TICKS / self.size) as usize
    }

    /// Steps in a clip `length` ticks long. The last one may be cut short
    /// by the clip's end.
    pub fn steps(self, length: Tick) -> usize {
        ((length.max(1) + self.size - 1) / self.size) as usize
    }

    /// Pages of [`PAGE`] steps, at least 1.
    pub fn pages(self, length: Tick) -> usize {
        self.steps(length).div_ceil(PAGE).max(1)
    }

    /// The step a note on `tick` belongs to in a clip `length` ticks long,
    /// and its nudge.
    pub fn place(self, length: Tick, tick: Tick) -> (usize, Tick) {
        let i = (tick + self.size / 2).div_euclid(self.size);
        if i as usize >= self.steps(length) {
            // Half a step or more past the last line: step 0, early.
            return (0, tick - length);
        }
        (i as usize, tick - i * self.size)
    }

    /// The nudges step `i` allows in a clip `length` ticks long, from and
    /// to (both included).
    pub fn nudges(self, length: Tick, i: usize) -> (Tick, Tick) {
        let line = i as Tick * self.size;
        // Step 0 reaches back over the clip's end as far as the ticks half
        // a step past the last line.
        let lo = if i == 0 {
            (self.steps(length) as Tick * self.size - self.size / 2 - length).min(0)
        } else {
            -(self.size / 2)
        };
        let hi = (self.size / 2 - 1).min(length - 1 - line);
        (lo, hi)
    }

    /// Where a note of step `i` with `nudge` starts.
    pub fn tick(self, length: Tick, i: usize, nudge: Tick) -> Tick {
        (i as Tick * self.size + nudge).rem_euclid(length.max(1))
    }

    /// The notes of sound `pitch` in step `i`, first (earliest) first.
    pub fn notes(self, clip: &Clip, pitch: u8, i: usize) -> Vec<Note> {
        let mut out: Vec<(Tick, Note)> = clip
            .notes
            .iter()
            .filter(|n| n.note == pitch)
            .map(|n| (self.place(clip.length, n.tick), *n))
            .filter(|((step, _), _)| *step == i)
            .map(|((_, nudge), n)| (nudge, n))
            .collect();
        out.sort_by_key(|(nudge, n)| (*nudge, n.len, n.velocity, n.chance));
        out.into_iter().map(|(_, n)| n).collect()
    }

    /// Every step of sound `pitch` in `clip`, all pages: None where a step
    /// is empty.
    pub fn row(self, clip: &Clip, pitch: u8) -> Vec<Option<Step>> {
        let mut row: Vec<Option<Step>> = vec![None; self.steps(clip.length)];
        for n in clip.notes.iter().filter(|n| n.note == pitch) {
            let (i, nudge) = self.place(clip.length, n.tick);
            let Some(slot) = row.get_mut(i) else { continue };
            let step = Step {
                velocity: n.velocity,
                len: n.len,
                nudge,
                chance: n.chance,
                cond: n.cond,
                ratchet: n.ratchet,
                notes: 1,
            };
            *slot = Some(match *slot {
                None => step,
                Some(s) => {
                    let first = if (nudge, n.len, n.velocity, n.chance)
                        < (s.nudge, s.len, s.velocity, s.chance)
                    {
                        step
                    } else {
                        s
                    };
                    Step {
                        notes: s.notes + 1,
                        ..first
                    }
                }
            });
        }
        row
    }

    /// Tap step `i` of sound `pitch`: its notes go, or with none, a note
    /// goes on its line, `velocity` loud, a step long, always played.
    /// Nothing past the clip's last step.
    pub fn toggle(self, clip: &Clip, pitch: u8, i: usize, velocity: u8) -> Edit {
        if i >= self.steps(clip.length) {
            return Edit::default();
        }
        let notes = self.notes(clip, pitch, i);
        if !notes.is_empty() {
            return Edit {
                erase: notes,
                erase_locks: self.locks(clip, Some(pitch), i),
                ..Edit::default()
            };
        }
        let note = Note::new(
            self.tick(clip.length, i, 0),
            self.size,
            pitch.min(127),
            velocity.clamp(1, 127),
        );
        Edit {
            add: vec![note],
            ..Edit::default()
        }
    }

    /// Change step `i` of sound `pitch`. Values are kept in range; an
    /// empty step, or one already so, gives an empty edit.
    pub fn change(self, clip: &Clip, pitch: u8, i: usize, change: Change) -> Edit {
        let notes = self.notes(clip, pitch, i);
        let Some(first) = notes.first() else {
            return Edit::default();
        };
        let (lo, hi) = self.nudges(clip.length, i);
        let shift = match change {
            Change::Nudge(n) => n.clamp(lo, hi) - self.place(clip.length, first.tick).1,
            _ => 0,
        };
        let mut edit = Edit::default();
        for n in notes {
            let mut new = n;
            match change {
                Change::Velocity(v) => new.velocity = v.clamp(1, 127),
                Change::Length(l) => new.len = l.clamp(1, MAX_CLIP_TICKS),
                Change::Chance(c) => new.chance = c.clamp(1, 100),
                Change::Cond(c) if c.ok() => new.cond = c,
                Change::Cond(_) => {}
                Change::Ratchet(r) => new.ratchet = r.clamp(1, MAX_RATCHET),
                Change::Nudge(_) => {
                    let nudge = (self.place(clip.length, n.tick).1 + shift).clamp(lo, hi);
                    new.tick = self.tick(clip.length, i, nudge);
                }
            }
            if new != n {
                edit.erase.push(n);
                edit.add.push(new);
            }
        }
        // The row's locks on the step move with its notes.
        if shift != 0 {
            for l in self.locks(clip, Some(pitch), i) {
                let nudge = (self.place(clip.length, l.tick).1 + shift).clamp(lo, hi);
                let tick = self.tick(clip.length, i, nudge);
                edit.move_lock(l, Lock { tick, ..l });
            }
        }
        edit
    }

    /// Change step `i` of every sound in it, as [`Grid::change`] does for
    /// one sound. Each sound's notes move by their own nudge.
    pub fn change_all(self, clip: &Clip, i: usize, change: Change) -> Edit {
        let mut edit = Edit::default();
        for pitch in self.sounds_in(clip, i) {
            edit.join(self.change(clip, pitch, i, change));
        }
        // The whole step's locks go where the notes' first one does.
        if let (Change::Nudge(n), false) = (change, edit.is_empty()) {
            let (lo, hi) = self.nudges(clip.length, i);
            let tick = self.tick(clip.length, i, n.clamp(lo, hi));
            for l in self.locks(clip, None, i) {
                edit.move_lock(l, Lock { tick, ..l });
            }
        }
        edit
    }

    /// Every note in step `i`, all sounds: lowest sound first, then the
    /// earliest note first. A melodic track's step shows its chord this way.
    pub fn chord(self, clip: &Clip, i: usize) -> Vec<Note> {
        self.sounds_in(clip, i)
            .into_iter()
            .flat_map(|pitch| self.notes(clip, pitch, i))
            .collect()
    }

    /// Step entry: `keys` (sound, velocity), the notes held when step `i`
    /// is pressed, go on its line, a step long, always played, in place of
    /// what the step held. No keys clears the step. Nothing past the last
    /// step.
    pub fn enter(self, clip: &Clip, i: usize, keys: &[(u8, u8)]) -> Edit {
        if i >= self.steps(clip.length) {
            return Edit::default();
        }
        let tick = self.tick(clip.length, i, 0);
        let mut add: Vec<Note> = Vec::new();
        for &(pitch, velocity) in keys {
            if !add.iter().any(|n| n.note == pitch.min(127)) {
                add.push(Note::new(
                    tick,
                    self.size,
                    pitch.min(127),
                    velocity.clamp(1, 127),
                ));
            }
        }
        let mut erase = self.chord(clip, i);
        // A note already there as it would be entered stays.
        erase.retain(|n| match add.iter().position(|a| a == n) {
            Some(j) => {
                add.remove(j);
                false
            }
            None => true,
        });
        let mut edit = Edit {
            erase,
            add,
            ..Edit::default()
        };
        // The step's locks stay with the sounds entered, on the line; the
        // rows of sounds no longer there lose theirs.
        for l in self.step_locks(clip, i) {
            let kept = !keys.is_empty()
                && l.sound
                    .is_none_or(|p| keys.iter().any(|&(k, _)| k.min(127) == p));
            if kept {
                edit.move_lock(l, Lock { tick, ..l });
            } else {
                edit.erase_locks.push(l);
            }
        }
        edit
    }

    /// Add a page: 16 more steps. With `copy`, they repeat the steps of
    /// the page before them (the whole clip, over and over, if it is
    /// shorter than a page); otherwise they are empty.
    pub fn add_page(self, clip: &Clip, copy: bool) -> Edit {
        self.resize(clip, self.steps(clip.length) + PAGE, copy)
    }

    /// Take the last 16 steps off, and their notes. A clip of a page or
    /// less stays as it is.
    pub fn remove_page(self, clip: &Clip) -> Edit {
        let n = self.steps(clip.length);
        if n <= PAGE {
            return Edit::default();
        }
        self.resize(clip, n - PAGE, false)
    }

    /// Make the clip `steps` steps long (1 to [`Grid::max_steps`]). Notes
    /// on steps that go are taken out; notes that play early on step 0
    /// stay just before the clip comes round. New steps are empty, or with
    /// `copy`, repeat the steps of the page before them.
    pub fn resize(self, clip: &Clip, steps: usize, copy: bool) -> Edit {
        let steps = steps.clamp(1, self.max_steps());
        let (old, new) = (clip.length, steps as Tick * self.size);
        if new < MIN_CLIP_TICKS || new == old {
            return Edit::default();
        }
        let n = self.steps(old);
        let mut edit = Edit {
            length: Some((old, new)),
            ..Edit::default()
        };
        for note in &clip.notes {
            let (i, nudge) = self.place(old, note.tick);
            if i >= steps {
                edit.erase.push(*note);
            } else if i == 0 && nudge < 0 {
                // Early on step 0: it stays that far before the clip's end.
                let (lo, _) = self.nudges(new, 0);
                let moved = Note {
                    tick: self.tick(new, 0, nudge.max(lo)),
                    ..*note
                };
                if moved != *note {
                    edit.erase.push(*note);
                    edit.add.push(moved);
                }
            }
        }
        // Locks go, or move, with their steps' notes.
        for l in &clip.locks {
            let (i, nudge) = self.place(old, l.tick);
            if i >= steps {
                edit.erase_locks.push(*l);
            } else if i == 0 && nudge < 0 {
                let (lo, _) = self.nudges(new, 0);
                let tick = self.tick(new, 0, nudge.max(lo));
                edit.move_lock(*l, Lock { tick, ..*l });
            }
        }
        if copy && steps > n {
            let width = n.min(PAGE);
            for j in 0..steps - n {
                let src = n - width + j % width;
                edit.add.extend(self.moved(clip, src, new, n + j));
                edit.add_locks
                    .extend(self.moved_locks(clip, src, new, n + j));
            }
        }
        // The moves are cut off with the steps, or the new steps repeat
        // the moves of the steps they copy.
        let size = self.size;
        edit.moves = moves_edit(clip, |m| {
            let new32 = u32::try_from(new).map_err(|_| MovesError::Bad)?;
            if new < old {
                m.truncate(new32)?;
            }
            if copy && steps > n {
                let width = n.min(PAGE) as Tick;
                let from = (n as Tick - width) * size;
                let to = (n as Tick * size).min(old);
                let mut at = n as Tick * size;
                while at < new {
                    m.copy_range(from as u32, to as u32, at as u32, new32)?;
                    at += width * size;
                }
            }
            Ok(())
        });
        edit
    }

    /// Copy page `from` over page `to`: the notes on page `to` go, and the
    /// notes of page `from` come in, step for step. Steps past the clip's
    /// end are left out.
    pub fn copy_page(self, clip: &Clip, from: usize, to: usize) -> Edit {
        let n = self.steps(clip.length);
        if from == to || from * PAGE >= n || to * PAGE >= n {
            return Edit::default();
        }
        let mut edit = Edit::default();
        for k in 0..PAGE {
            let (src, dst) = (from * PAGE + k, to * PAGE + k);
            if dst >= n {
                break;
            }
            edit.erase.extend(self.chord(clip, dst));
            edit.erase_locks.extend(self.step_locks(clip, dst));
            if src < n {
                edit.add.extend(self.moved(clip, src, clip.length, dst));
                edit.add_locks
                    .extend(self.moved_locks(clip, src, clip.length, dst));
            }
        }
        // The page's moves come along, over the same ticks.
        let page = PAGE as Tick * self.size;
        let (a, b) = (
            from as Tick * page,
            ((from + 1) as Tick * page).min(clip.length),
        );
        let at = to as Tick * page;
        edit.moves = moves_edit(clip, |m| {
            m.copy_range(a as u32, b as u32, at as u32, clip.length as u32)
        });
        edit
    }

    /// The notes of step `src` of `clip`, all sounds, as they would be on
    /// step `dst` of a clip `length` ticks long (each keeping its nudge as
    /// far as that step allows).
    fn moved(self, clip: &Clip, src: usize, length: Tick, dst: usize) -> Vec<Note> {
        let (lo, hi) = self.nudges(length, dst);
        self.chord(clip, src)
            .into_iter()
            .map(|n| {
                let nudge = self.place(clip.length, n.tick).1;
                Note {
                    tick: self.tick(length, dst, nudge.clamp(lo, hi)),
                    ..n
                }
            })
            .collect()
    }

    /// The locks of step `src` of `clip`, all rows, as they would be on
    /// step `dst` of a clip `length` ticks long (as [`Grid::moved`]).
    fn moved_locks(self, clip: &Clip, src: usize, length: Tick, dst: usize) -> Vec<Lock> {
        let (lo, hi) = self.nudges(length, dst);
        self.step_locks(clip, src)
            .into_iter()
            .map(|l| {
                let nudge = self.place(clip.length, l.tick).1;
                Lock {
                    tick: self.tick(length, dst, nudge.clamp(lo, hi)),
                    ..l
                }
            })
            .collect()
    }

    /// The locks on step `i` of row `sound` (a pad or note number), or with
    /// None, the whole step's own locks (not the rows'), in the order they
    /// were written.
    pub fn locks(self, clip: &Clip, sound: Option<u8>, i: usize) -> Vec<Lock> {
        clip.locks
            .iter()
            .filter(|l| l.sound == sound && self.place(clip.length, l.tick).0 == i)
            .copied()
            .collect()
    }

    /// Every lock on step `i`, every row and the whole step's.
    fn step_locks(self, clip: &Clip, i: usize) -> Vec<Lock> {
        clip.locks
            .iter()
            .filter(|l| self.place(clip.length, l.tick).0 == i)
            .copied()
            .collect()
    }

    /// How many locks each step of row `sound` shows, all pages: the row's
    /// own and the whole step's. For the lock dots; a step with locks and
    /// no note holds trigless locks.
    pub fn lock_row(self, clip: &Clip, sound: u8) -> Vec<usize> {
        let mut row = vec![0; self.steps(clip.length)];
        for l in clip
            .locks
            .iter()
            .filter(|l| l.sound.is_none_or(|p| p == sound))
        {
            if let Some(n) = row.get_mut(self.place(clip.length, l.tick).0) {
                *n += 1;
            }
        }
        row
    }

    /// Lock `addr` to `value` on step `i` of row `sound` (None: the whole
    /// step, for a chord). The lock takes the tick of the row's first note
    /// in the step (of the chord's earliest note, for the whole step), so
    /// it plays on that note; on a step with no note it is a trigless lock
    /// on the step's line. It replaces a lock of the same parameter there.
    /// Nothing past the last step, and nothing for a value that is not
    /// finite.
    pub fn lock(self, clip: &Clip, sound: Option<u8>, i: usize, addr: RelAddr, value: f32) -> Edit {
        if i >= self.steps(clip.length) || !value.is_finite() {
            return Edit::default();
        }
        let notes = match sound {
            Some(p) => self.notes(clip, p, i),
            None => self.chord(clip, i),
        };
        let tick = notes
            .iter()
            .min_by_key(|n| self.place(clip.length, n.tick).1)
            .map_or(self.tick(clip.length, i, 0), |n| n.tick);
        let new = Lock::new(tick, sound.map(|p| p.min(127)), addr, value);
        let old: Vec<Lock> = self
            .locks(clip, new.sound, i)
            .into_iter()
            .filter(|l| l.addr == addr)
            .collect();
        if old == [new] {
            return Edit::default();
        }
        Edit {
            erase_locks: old,
            add_locks: vec![new],
            ..Edit::default()
        }
    }

    /// Take the lock of `addr` off step `i` of row `sound`.
    pub fn unlock(self, clip: &Clip, sound: Option<u8>, i: usize, addr: RelAddr) -> Edit {
        Edit {
            erase_locks: self
                .locks(clip, sound, i)
                .into_iter()
                .filter(|l| l.addr == addr)
                .collect(),
            ..Edit::default()
        }
    }

    /// Take every lock off step `i` of row `sound`.
    pub fn clear_locks(self, clip: &Clip, sound: Option<u8>, i: usize) -> Edit {
        Edit {
            erase_locks: self.locks(clip, sound, i),
            ..Edit::default()
        }
    }

    /// Write the Euclidean pattern `e` into the row of sound `pitch`,
    /// repeated from the clip's first step to its last: a step with a hit
    /// keeps its notes, or with none gets a note on its line, a step long,
    /// `velocity` loud; a step without one loses its notes and their
    /// locks. Empty when the row already is the pattern.
    pub fn euclid(self, clip: &Clip, pitch: u8, e: Euclid, velocity: u8) -> Edit {
        let e = e.fit();
        let pitch = pitch.min(127);
        let mut edit = Edit::default();
        for i in 0..self.steps(clip.length) {
            let notes = self.notes(clip, pitch, i);
            match (e.hit(i), notes.is_empty()) {
                (true, true) => edit.add.push(Note::new(
                    self.tick(clip.length, i, 0),
                    self.size,
                    pitch,
                    velocity.clamp(1, 127),
                )),
                (false, false) => {
                    edit.erase.extend(notes);
                    edit.erase_locks.extend(self.locks(clip, Some(pitch), i));
                }
                _ => {}
            }
        }
        edit
    }

    /// The sounds with notes in step `i`, lowest first.
    fn sounds_in(self, clip: &Clip, i: usize) -> Vec<u8> {
        let mut s: Vec<u8> = clip
            .notes
            .iter()
            .filter(|n| self.place(clip.length, n.tick).0 == i)
            .map(|n| n.note)
            .collect();
        s.sort_unstable();
        s.dedup();
        s
    }
}

impl Clip {
    /// The sounds (pads or note numbers) the clip plays, lowest first.
    pub fn sounds(&self) -> Vec<u8> {
        let mut s: Vec<u8> = self.notes.iter().map(|n| n.note).collect();
        s.sort_unstable();
        s.dedup();
        s
    }

    /// Apply `edit`: take its notes and locks out, change the length, then
    /// put its new ones in. All or nothing: if a note or lock to take out
    /// is not in the clip (`MissingNote`, `MissingLock`), the clip is not
    /// `length.0` long or a note or lock would be past its new end
    /// (`ClipLength`), or a new note or lock does not fit, the clip is
    /// unchanged and the error says which.
    pub fn apply(&mut self, edit: &Edit) -> Result<(), crate::ModelError> {
        let (notes, locks, length) = (self.notes.clone(), self.locks.clone(), self.length);
        let moves = edit.moves.as_ref().map(|_| self.moves.clone());
        let touch = self.touch.len();
        let result = self.apply_all(edit);
        if result.is_err() {
            (self.notes, self.locks, self.length) = (notes, locks, length);
            self.touch.truncate(touch);
            if let Some(moves) = moves {
                self.moves = moves;
            }
        }
        result
    }

    fn apply_all(&mut self, edit: &Edit) -> Result<(), crate::ModelError> {
        for n in &edit.erase {
            if !self.remove_note(n) {
                return Err(crate::ModelError::MissingNote);
            }
        }
        for l in &edit.erase_locks {
            if !self.remove_lock(l) {
                return Err(crate::ModelError::MissingLock);
            }
        }
        if let Some((from, to)) = edit.length {
            let fits = (MIN_CLIP_TICKS..=MAX_CLIP_TICKS).contains(&to)
                && self.notes.iter().all(|n| n.tick < to)
                && self.locks.iter().all(|l| l.tick < to);
            if self.length != from || !fits {
                return Err(crate::ModelError::ClipLength);
            }
            self.length = to;
        }
        if !edit.touch.is_empty() {
            if self.touch.len() != edit.touch_at as usize
                || !touch_room(self.touch.len(), &edit.touch)
            {
                return Err(crate::ModelError::BadTouch);
            }
            self.touch.extend_from_slice(&edit.touch);
        }
        for n in &edit.add {
            self.add_note(*n)?;
        }
        for l in &edit.add_locks {
            self.add_lock(*l)?;
        }
        if let Some(m) = &edit.moves {
            if self.moves != m.before {
                return Err(crate::ModelError::BadMoves);
            }
            self.moves = m.after.clone();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BAR: Tick = 3840;

    fn clip(length: Tick, notes: &[(Tick, u8)]) -> Clip {
        let mut c = Clip::new("steps", length);
        c.notes = notes
            .iter()
            .map(|&(tick, note)| Note::new(tick, 60, note, 100))
            .collect();
        c
    }

    #[test]
    fn euclidean_patterns_spread_their_hits_evenly() {
        let e = |hits, steps, rotate| Euclid {
            hits,
            steps,
            rotate,
        };
        assert_eq!(e(3, 8, 0).pattern(), "x..x..x.");
        assert_eq!(e(4, 16, 0).pattern(), "x...x...x...x...");
        assert_eq!(e(5, 8, 0).pattern(), "x.x.xx.x");
        assert_eq!(e(3, 8, 1).pattern(), ".x..x..x");
        assert_eq!(e(0, 4, 0).pattern(), "....");
        assert_eq!(e(4, 4, 0).pattern(), "xxxx");
        // Out of range: hits cut to the steps, rotate wraps, steps capped.
        assert_eq!(e(9, 4, 6).fit(), e(4, 4, 2));
        assert_eq!(e(1, 0, 0).fit().steps, 1);
        assert_eq!(e(1, 200, 0).fit().steps, MAX_EUCLID_STEPS);
        // Every hits-of-steps pattern has exactly that many hits, and the
        // gaps between them differ by at most one step (maximally even).
        for n in 1..=32u8 {
            for k in 0..=n {
                let p = e(k, n, 0);
                let at: Vec<usize> = (0..usize::from(n)).filter(|&i| p.hit(i)).collect();
                assert_eq!(at.len(), usize::from(k), "{k} of {n}");
                if k >= 2 {
                    let gaps: Vec<usize> = (0..at.len())
                        .map(|j| (at[(j + 1) % at.len()] + usize::from(n) - at[j]) % usize::from(n))
                        .map(|g| if g == 0 { usize::from(n) } else { g })
                        .collect();
                    let (lo, hi) = (gaps.iter().min().unwrap(), gaps.iter().max().unwrap());
                    assert!(hi - lo <= 1, "{k} of {n}: gaps {gaps:?}");
                }
            }
        }
    }

    #[test]
    fn a_euclidean_fill_writes_one_row_and_undoes() {
        use mpfree_param::{OnTrack, RelAddr};
        let g = Grid::new(240).unwrap();
        // Pad 2 on steps 0 and 1 (with a lock on step 1), pad 5 on step 2.
        let mut c = clip(BAR, &[(0, 2), (250, 2), (480, 5)]);
        c.notes[0].velocity = 90;
        let cut = RelAddr::new(OnTrack::Inst, 1);
        c.apply(&g.lock(&c, Some(2), 1, cut, 0.5)).unwrap();
        let before = c.clone();
        let e = Euclid {
            hits: 3,
            steps: 8,
            rotate: 0,
        };
        let edit = g.euclid(&c, 2, e, 90);
        c.apply(&edit).unwrap();
        // x..x..x. twice: steps 0, 3, 6, 8, 11, 14. Step 0's note stays,
        // step 1's goes with its lock, pad 5 is left alone.
        let row: Vec<usize> = g
            .row(&c, 2)
            .iter()
            .enumerate()
            .filter_map(|(i, s)| s.map(|_| i))
            .collect();
        assert_eq!(row, vec![0, 3, 6, 8, 11, 14]);
        assert!(c.notes.iter().any(|n| n.note == 5 && n.tick == 480));
        assert!(c.locks.is_empty());
        assert!(c
            .notes
            .iter()
            .filter(|n| n.note == 2)
            .all(|n| n.velocity == 90 && n.len >= 60));
        assert!(g.euclid(&c, 2, e, 90).is_empty(), "already the pattern");
        // Undo puts the row and its lock back.
        c.apply(&edit.inverse()).unwrap();
        let mut a = c.notes.clone();
        let mut b = before.notes.clone();
        a.sort_by_key(|n| (n.tick, n.note));
        b.sort_by_key(|n| (n.tick, n.note));
        assert_eq!((a, c.locks.clone()), (b, before.locks.clone()));
        // A pattern longer than the clip is cut at its end.
        let short = clip(4 * 240, &[]);
        let edit = g.euclid(&short, 0, Euclid::default(), 100);
        assert_eq!(edit.add.len(), 1);
    }

    #[test]
    fn every_tick_has_one_step_and_comes_back_from_it() {
        for size in SIZES {
            for length in [BAR, 2 * BAR, 1000, 120, 3 * 320 + 7, MIN_CLIP_TICKS] {
                let g = Grid::new(size).unwrap();
                for tick in 0..length {
                    let (i, nudge) = g.place(length, tick);
                    assert!(
                        i < g.steps(length),
                        "size {size} length {length} tick {tick}"
                    );
                    let (lo, hi) = g.nudges(length, i);
                    assert!(
                        (lo..=hi).contains(&nudge),
                        "size {size} length {length} tick {tick}: {nudge} not in {lo}..={hi}"
                    );
                    assert_eq!(g.tick(length, i, nudge), tick);
                }
                // Every nudge a step allows lands back on that step.
                for i in 0..g.steps(length) {
                    let (lo, hi) = g.nudges(length, i);
                    for nudge in [lo, hi, 0.clamp(lo, hi)] {
                        assert_eq!(
                            g.place(length, g.tick(length, i, nudge)),
                            (i, nudge),
                            "size {size} length {length} step {i}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn steps_and_pages() {
        let g = Grid::new(240).unwrap();
        assert_eq!((g.steps(BAR), g.pages(BAR)), (16, 1));
        assert_eq!((g.steps(4 * BAR), g.pages(4 * BAR)), (64, 4));
        assert_eq!(
            (g.steps(1000), g.pages(1000)),
            (5, 1),
            "the last step is cut short"
        );
        let t = Grid::new(320).unwrap();
        assert_eq!(t.steps(BAR), 12, "1/8 triplets: 12 to a bar");
        assert_eq!(Grid::new(119), None);
        assert_eq!(Grid::new(3841), None);
    }

    #[test]
    fn a_row_shows_each_steps_first_note_and_its_nudge() {
        let g = Grid::new(240).unwrap();
        // A kick on 1 and a little late on 3, a two-note roll on step 6,
        // and one early in the last half step: step 0 again.
        let mut c = clip(
            BAR,
            &[
                (0, 36),
                (490, 36),
                (1440, 36),
                (1500, 36),
                (3800, 36),
                (240, 38),
            ],
        );
        c.notes[3].velocity = 70;
        let row = g.row(&c, 36);
        assert_eq!(row.len(), 16);
        let on: Vec<(usize, Tick, usize)> = row
            .iter()
            .enumerate()
            .filter_map(|(i, s)| s.map(|s| (i, s.nudge, s.notes)))
            .collect();
        assert_eq!(on, vec![(0, -40, 2), (2, 10, 1), (6, 0, 2)]);
        assert_eq!(row[6].unwrap().velocity, 100, "the first note of the roll");
        assert_eq!(
            g.notes(&c, 36, 0)
                .iter()
                .map(|n| n.tick)
                .collect::<Vec<_>>(),
            vec![3800, 0]
        );
        assert_eq!(c.sounds(), vec![36, 38]);
    }

    #[test]
    fn tapping_a_step_adds_a_note_or_takes_its_notes_out() {
        let g = Grid::new(240).unwrap();
        let mut c = clip(BAR, &[(1440, 36), (1500, 36)]);
        let on = g.toggle(&c, 36, 4, 90);
        assert_eq!(
            on,
            Edit {
                add: vec![Note::new(960, 240, 36, 90)],
                ..Edit::default()
            }
        );
        c.apply(&on).unwrap();
        assert_eq!(
            g.row(&c, 36)[4].map(|s| (s.velocity, s.len, s.nudge, s.chance)),
            Some((90, 240, 0, 100))
        );
        let off = g.toggle(&c, 36, 6, 90);
        assert_eq!(off.erase.len(), 2, "the whole roll");
        c.apply(&off).unwrap();
        assert_eq!(c.notes, vec![Note::new(960, 240, 36, 90)]);
        assert!(g.toggle(&c, 36, 16, 90).is_empty(), "past the last step");
    }

    /// Page edits take the clip's moves along: a copied page copies them,
    /// a page added with copy repeats them, a page taken off cuts them
    /// off, and the inverse puts them back.
    #[test]
    fn page_edits_take_the_moves_along() {
        use mpfree_moves::{Kind, Map, Target};
        use mpfree_param::OnTrack;
        let g = Grid::new(240).unwrap();
        let page = 16 * 240;
        let mut c = clip(2 * page, &[(0, 36)]);
        let target = Target::Track(RelAddr::new(OnTrack::Inst, 1));
        // A sweep up over the first page's first beat, then let go.
        c.moves
            .put(
                target,
                Map::linear(0.0, 1.0),
                &[
                    (0.0, 0.0, Kind::Ramp),
                    (960.0, 1.0, Kind::Ramp),
                    (961.0, 1.0, Kind::End),
                ],
                2 * page as u32,
            )
            .unwrap();
        let at = |c: &Clip, t: Tick| c.moves.lane(target).and_then(|l| l.value_at(t as f64));
        assert_eq!(at(&c, 480), Some(0.5));
        assert_eq!(at(&c, page + 480), None, "page 2 is free");

        // Copy page 1 over page 2.
        let e = g.copy_page(&c, 0, 1);
        let before = c.clone();
        c.apply(&e).unwrap();
        assert_eq!(at(&c, page + 480), Some(0.5));
        assert_eq!(at(&c, 480), Some(0.5), "page 1 keeps its own");
        c.apply(&e.inverse()).unwrap();
        assert_eq!(c, before, "the inverse puts the moves back");

        // Add a page that copies the one before it (page 2, which is free).
        c.apply(&g.copy_page(&c, 0, 1)).unwrap();
        let e = g.add_page(&c, true);
        c.apply(&e).unwrap();
        assert_eq!(c.length, 3 * page);
        assert_eq!(at(&c, 2 * page + 480), Some(0.5), "page 3 repeats page 2");
        assert!(c.check().is_ok());

        // Take two pages off: the moves end with the clip.
        c.apply(&g.remove_page(&c)).unwrap();
        c.apply(&g.remove_page(&c)).unwrap();
        assert_eq!(c.length, page);
        assert!(c.check().is_ok());
        assert_eq!(at(&c, 480), Some(0.5));

        // A clip with no moves gets no moves edit.
        let plain = clip(page, &[(0, 36)]);
        assert!(g.add_page(&plain, true).moves.is_none());
        // An edit made for other moves is refused, and changes nothing.
        let mut other = c.clone();
        other.moves = mpfree_moves::Moves::new();
        let e = g.copy_page(
            &{
                let mut two = c.clone();
                two.apply(&g.add_page(&c, false)).unwrap();
                two
            },
            0,
            1,
        );
        if e.moves.is_some() {
            let kept = other.clone();
            assert!(other.apply(&e).is_err());
            assert_eq!(other, kept);
        }
    }

    #[test]
    fn a_step_takes_a_trig_condition_and_a_ratchet() {
        let g = Grid::new(240).unwrap();
        let mut c = clip(BAR, &[(1440, 36), (1500, 36), (1440, 38)]);
        let quarter = Cond::Every { a: 1, b: 4 };
        let e = g.change(&c, 36, 6, Change::Cond(quarter));
        assert_eq!(e.add.len(), 2, "every note of the step");
        c.apply(&e).unwrap();
        let e = g.change(&c, 36, 6, Change::Ratchet(20));
        assert!(e.add.iter().all(|n| n.ratchet == MAX_RATCHET));
        c.apply(&e).unwrap();
        let s = g.row(&c, 36)[6].unwrap();
        assert_eq!((s.cond, s.ratchet, s.notes), (quarter, MAX_RATCHET, 2));
        assert_eq!(s.cond.to_string(), "1:4");
        let other = g.row(&c, 38)[6].unwrap();
        assert_eq!(
            (other.cond, other.ratchet),
            (Cond::Always, 1),
            "only the step's sound"
        );
        assert!(
            g.change(&c, 36, 6, Change::Cond(Cond::Every { a: 5, b: 4 }))
                .is_empty(),
            "out of range"
        );
        assert_eq!(g.change(&c, 36, 6, Change::Ratchet(0)).add[0].ratchet, 1);
        assert!(
            g.change(&c, 36, 6, Change::Cond(quarter)).is_empty(),
            "already so"
        );
        // Each kind joins its own undo.
        let keys = [
            Change::Cond(Cond::Fill),
            Change::Ratchet(2),
            Change::Chance(50),
        ]
        .map(|ch| ch.key(36, 6));
        assert!(keys[0] != keys[1] && keys[1] != keys[2] && keys[0] != keys[2]);
    }

    #[test]
    fn changes_reach_every_note_in_the_step_and_stay_in_range() {
        let g = Grid::new(240).unwrap();
        let c = clip(BAR, &[(1440, 36), (1500, 36), (1440, 38)]);
        let e = g.change(&c, 36, 6, Change::Velocity(200));
        assert_eq!(
            e.add.iter().map(|n| n.velocity).collect::<Vec<_>>(),
            vec![127, 127]
        );
        assert!(e.add.iter().all(|n| n.note == 36), "only the step's sound");
        assert_eq!(g.change(&c, 36, 6, Change::Chance(0)).add[0].chance, 1);
        assert_eq!(g.change(&c, 36, 6, Change::Length(0)).add[0].len, 1);
        assert!(
            g.change(&c, 36, 6, Change::Velocity(100)).is_empty(),
            "already so"
        );
        assert!(
            g.change(&c, 36, 5, Change::Velocity(1)).is_empty(),
            "an empty step"
        );
        // A nudge moves the roll together, and no note leaves the step.
        let e = g.change(&c, 36, 6, Change::Nudge(100));
        assert_eq!(
            e.add.iter().map(|n| n.tick).collect::<Vec<_>>(),
            vec![1540, 1559]
        );
        let e = g.change(&c, 36, 6, Change::Nudge(-500));
        assert_eq!(
            e.add.iter().map(|n| n.tick).collect::<Vec<_>>(),
            vec![1320, 1380]
        );
    }

    #[test]
    fn step_zero_nudged_early_wraps_to_the_clips_end() {
        let g = Grid::new(240).unwrap();
        let mut c = clip(2 * BAR, &[(0, 36)]);
        let e = g.change(&c, 36, 0, Change::Nudge(-30));
        assert_eq!(e.add, vec![Note::new(2 * BAR - 30, 60, 36, 100)]);
        c.apply(&e).unwrap();
        assert_eq!(g.row(&c, 36)[0].map(|s| s.nudge), Some(-30));
        // A clip that is not a whole number of steps long: no early step 0.
        let c = clip(1000, &[(0, 36)]);
        assert_eq!(g.nudges(1000, 0), (0, 119));
        assert!(g.change(&c, 36, 0, Change::Nudge(-30)).is_empty());
        assert_eq!(
            g.nudges(1000, 4),
            (-120, 39),
            "the last step ends with the clip"
        );
    }

    #[test]
    fn an_edit_applies_whole_or_not_at_all() {
        let mut c = clip(BAR, &[(0, 36)]);
        let bad = Edit {
            erase: vec![Note::new(0, 60, 36, 100)],
            add: vec![Note::new(BAR, 60, 36, 100)],
            ..Edit::default()
        };
        assert!(c.apply(&bad).is_err());
        assert_eq!(c.notes, vec![Note::new(0, 60, 36, 100)]);
        let missing = Edit {
            erase: vec![Note::new(5, 60, 36, 100)],
            ..Edit::default()
        };
        assert!(c.apply(&missing).is_err());
        assert_eq!(c.notes.len(), 1);
    }

    const CUT: RelAddr = RelAddr::new(mpfree_param::OnTrack::Inst, 12);
    const TUNE: RelAddr = RelAddr::new(mpfree_param::OnTrack::Pad(2), 0);

    fn lock_ticks(c: &Clip) -> Vec<(Tick, Option<u8>, f32)> {
        let mut v: Vec<_> = c.locks.iter().map(|l| (l.tick, l.sound, l.value)).collect();
        v.sort_by_key(|a| (a.0, a.1));
        v
    }

    #[test]
    fn a_lock_sits_on_its_steps_note_or_on_the_line() {
        let g = Grid::new(240).unwrap();
        // A kick nudged late on step 2, nothing on step 5.
        let mut c = clip(BAR, &[(490, 36)]);
        let on_note = g.lock(&c, Some(36), 2, CUT, 800.0);
        assert_eq!(
            on_note.add_locks,
            vec![Lock::new(490, Some(36), CUT, 800.0)]
        );
        c.apply(&on_note).unwrap();
        let trigless = g.lock(&c, Some(36), 5, CUT, 300.0);
        assert_eq!(
            trigless.add_locks,
            vec![Lock::new(1200, Some(36), CUT, 300.0)]
        );
        c.apply(&trigless).unwrap();
        // The same parameter again replaces it; the same value is no edit.
        let again = g.lock(&c, Some(36), 2, CUT, 900.0);
        assert_eq!(again.erase_locks.len(), 1);
        c.apply(&again).unwrap();
        assert!(g.lock(&c, Some(36), 2, CUT, 900.0).is_empty());
        assert_eq!(
            lock_ticks(&c),
            vec![(490, Some(36), 900.0), (1200, Some(36), 300.0)]
        );
        assert_eq!(g.locks(&c, Some(36), 2).len(), 1);
        assert!(
            g.locks(&c, None, 2).is_empty(),
            "the row's, not the whole step's"
        );
        let dots = g.lock_row(&c, 36);
        assert_eq!((dots[2], dots[5], dots.iter().sum::<usize>()), (1, 1, 2));
        // Nothing past the last step, and no NaN.
        assert!(g.lock(&c, Some(36), 16, CUT, 1.0).is_empty());
        assert!(g.lock(&c, Some(36), 3, CUT, f32::NAN).is_empty());
        // Unlock and clear.
        c.apply(&g.unlock(&c, Some(36), 5, CUT)).unwrap();
        assert_eq!(c.locks.len(), 1);
        c.apply(&g.clear_locks(&c, Some(36), 2)).unwrap();
        assert!(c.locks.is_empty());
    }

    #[test]
    fn locks_go_with_their_steps() {
        let g = Grid::new(240).unwrap();
        let mut c = clip(BAR, &[(480, 36), (480, 38), (960, 36)]);
        for e in [
            g.lock(&c, Some(36), 2, CUT, 1.0),
            g.lock(&c, Some(38), 2, TUNE, 2.0),
            g.lock(&c, None, 2, CUT, 3.0),
            g.lock(&c, Some(36), 4, CUT, 4.0),
        ] {
            c.apply(&e).unwrap();
        }
        // A nudge of one row moves its locks; of the whole step, all.
        c.apply(&g.change(&c, 36, 2, Change::Nudge(30))).unwrap();
        assert_eq!(
            lock_ticks(&c),
            vec![
                (480, None, 3.0),
                (480, Some(38), 2.0),
                (510, Some(36), 1.0),
                (960, Some(36), 4.0)
            ]
        );
        c.apply(&g.change_all(&c, 2, Change::Nudge(-20))).unwrap();
        assert_eq!(
            lock_ticks(&c)[..3],
            [(460, None, 3.0), (460, Some(36), 1.0), (460, Some(38), 2.0)]
        );
        // Tapping a row's step off takes that row's locks with it.
        let off = g.toggle(&c, 38, 2, 100);
        assert_eq!(off.erase_locks, vec![Lock::new(460, Some(38), TUNE, 2.0)]);
        c.apply(&off).unwrap();
        // Undo puts notes and locks back.
        c.apply(&off.inverse()).unwrap();
        assert_eq!(c.locks.len(), 4);
        // Copying page 1 to a new page 2 carries the locks.
        let add = g.add_page(&c, true);
        c.apply(&add).unwrap();
        assert_eq!(c.locks.len(), 8);
        assert_eq!(g.locks(&c, Some(36), 18).len(), 1);
        assert_eq!(g.locks(&c, Some(36), 20)[0].tick, 960 + BAR);
        // Taking the page off takes them out again.
        c.apply(&g.remove_page(&c)).unwrap();
        assert_eq!(c.locks.len(), 4);
        // Copy page 0 over itself is nothing; step entry keeps the chord's
        // locks for the sounds entered, on the line.
        let entered = g.enter(&c, 2, &[(36, 100)]);
        c.apply(&entered).unwrap();
        assert_eq!(
            lock_ticks(&c)[..2],
            [(480, None, 3.0), (480, Some(36), 1.0)]
        );
        assert_eq!(c.locks.len(), 3, "pad 38's lock went with its note");
        c.apply(&g.enter(&c, 2, &[])).unwrap();
        assert_eq!(c.locks.len(), 1, "a cleared step loses every lock");
    }

    #[test]
    fn a_lock_out_of_range_is_refused() {
        let mut c = clip(BAR, &[]);
        assert_eq!(
            c.add_lock(Lock::new(BAR, None, CUT, 1.0)),
            Err(crate::ModelError::BadLock(0))
        );
        assert!(c.add_lock(Lock::new(0, None, CUT, f32::INFINITY)).is_err());
        assert!(c.add_lock(Lock::new(0, Some(128), CUT, 1.0)).is_err());
        c.add_lock(Lock::new(300, None, CUT, 1.0)).unwrap();
        assert!(c.check().is_ok());
        // Shortening past a lock is refused like past a note.
        let short = Edit {
            length: Some((BAR, 240)),
            ..Edit::default()
        };
        assert_eq!(c.apply(&short), Err(crate::ModelError::ClipLength));
        let missing = Edit {
            erase_locks: vec![Lock::new(301, None, CUT, 1.0)],
            ..Edit::default()
        };
        assert_eq!(c.apply(&missing), Err(crate::ModelError::MissingLock));
        assert_eq!(c.locks.len(), 1);
    }

    fn ticks_of(c: &Clip) -> Vec<(Tick, u8)> {
        let mut t: Vec<(Tick, u8)> = c.notes.iter().map(|n| (n.tick, n.note)).collect();
        t.sort_unstable();
        t
    }

    #[test]
    fn pages_go_on_and_come_off() {
        let g = Grid::new(240).unwrap();
        // A new step clip is one page; a kick on 1 and 3, a hat early on 1.
        assert_eq!(g.page_length(), BAR);
        let mut c = clip(g.page_length(), &[(0, 36), (1920, 36), (BAR - 30, 42)]);
        assert_eq!(g.row(&c, 42)[0].map(|s| s.nudge), Some(-30));
        // A copied page repeats page 1, the early hat too: 32 steps.
        let e = g.add_page(&c, true);
        assert_eq!(e.length, Some((BAR, 2 * BAR)));
        c.apply(&e).unwrap();
        assert_eq!((g.steps(c.length), g.pages(c.length)), (32, 2));
        assert_eq!(
            ticks_of(&c),
            vec![
                (0, 36),
                (1920, 36),
                (BAR - 30, 42),
                (BAR, 36),
                (BAR + 1920, 36),
                (2 * BAR - 30, 42)
            ]
        );
        let on = |c: &Clip, p| {
            g.row(c, p)
                .iter()
                .enumerate()
                .filter_map(|(i, s)| s.map(|s| (i, s.nudge)))
                .collect::<Vec<_>>()
        };
        assert_eq!(on(&c, 42), vec![(0, -30), (16, -30)]);
        // An empty page, then a copy of it: 64 steps on 4 pages. The early
        // hat stays just before the end.
        c.apply(&g.add_page(&c, false)).unwrap();
        c.apply(&g.add_page(&c, true)).unwrap();
        assert_eq!((g.steps(c.length), g.pages(c.length)), (64, 4));
        assert_eq!(on(&c, 36), vec![(0, 0), (8, 0), (16, 0), (24, 0)]);
        assert_eq!(on(&c, 42), vec![(0, -30), (16, -30)]);
        assert!(c.notes.iter().any(|n| n.tick == 4 * BAR - 30));
        // Taking pages off takes their notes; the early hat moves with the end.
        c.apply(&g.remove_page(&c)).unwrap();
        c.apply(&g.remove_page(&c)).unwrap();
        c.apply(&g.remove_page(&c)).unwrap();
        assert_eq!(ticks_of(&c), vec![(0, 36), (1920, 36), (BAR - 30, 42)]);
        assert!(g.remove_page(&c).is_empty(), "a clip of one page keeps it");
        // Any number of steps: 13, for polymeter.
        c.apply(&g.resize(&c, 13, false)).unwrap();
        assert_eq!((c.length, g.steps(c.length)), (13 * 240, 13));
        assert_eq!(ticks_of(&c), vec![(0, 36), (1920, 36), (13 * 240 - 30, 42)]);
        c.apply(&g.resize(&c, 5, false)).unwrap();
        assert_eq!(
            ticks_of(&c),
            vec![(0, 36), (5 * 240 - 30, 42)],
            "step 8's kick went"
        );
        assert!(g.resize(&c, 5, true).is_empty());
        assert_eq!(
            g.resize(&c, usize::MAX, false).length,
            Some((5 * 240, MAX_CLIP_TICKS))
        );
    }

    #[test]
    fn a_short_clip_repeats_to_fill_a_copied_page() {
        let g = Grid::new(240).unwrap();
        // Three steps: kick, hat 10 ticks late, nothing.
        let mut c = clip(3 * 240, &[(0, 36), (250, 42)]);
        c.apply(&g.add_page(&c, true)).unwrap();
        assert_eq!(g.steps(c.length), 19);
        let row = |p| {
            g.row(&c, p)
                .iter()
                .enumerate()
                .filter_map(|(i, s)| s.map(|s| (i, s.nudge)))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            row(36),
            vec![(0, 0), (3, 0), (6, 0), (9, 0), (12, 0), (15, 0), (18, 0)]
        );
        assert_eq!(
            row(42),
            vec![(1, 10), (4, 10), (7, 10), (10, 10), (13, 10), (16, 10)]
        );
    }

    #[test]
    fn a_page_copies_over_another() {
        let g = Grid::new(240).unwrap();
        let mut c = clip(2 * BAR, &[(0, 36), (250, 42), (BAR + 480, 38)]);
        c.apply(&g.copy_page(&c, 0, 1)).unwrap();
        assert_eq!(
            ticks_of(&c),
            vec![(0, 36), (250, 42), (BAR, 36), (BAR + 250, 42)]
        );
        assert!(g.copy_page(&c, 0, 2).is_empty(), "no page 3");
        assert!(g.copy_page(&c, 1, 1).is_empty());
    }

    #[test]
    fn a_held_chord_goes_on_a_step() {
        let g = Grid::new(240).unwrap();
        let mut c = clip(BAR, &[(240, 60)]);
        // C minor held on step 1: C stays as it was, E flat and G come in.
        let e = g.enter(&c, 1, &[(60, 100), (63, 90), (67, 80), (63, 90)]);
        assert_eq!(
            e.erase,
            vec![Note::new(240, 60, 60, 100)],
            "the old C was 60 ticks long"
        );
        assert_eq!(e.add.len(), 3);
        c.apply(&e).unwrap();
        let chord: Vec<(u8, u8)> = g
            .chord(&c, 1)
            .iter()
            .map(|n| (n.note, n.velocity))
            .collect();
        assert_eq!(chord, vec![(60, 100), (63, 90), (67, 80)]);
        // The whole chord's velocity and nudge change together.
        c.apply(&g.change_all(&c, 1, Change::Nudge(-20))).unwrap();
        assert!(g.chord(&c, 1).iter().all(|n| n.tick == 220));
        assert_ne!(
            Change::Velocity(1).key_all(1),
            Change::Velocity(1).key(127, 1)
        );
        // No keys clears the step; nothing past the end.
        c.apply(&g.enter(&c, 1, &[])).unwrap();
        assert!(c.notes.is_empty());
        assert!(g.enter(&c, 16, &[(60, 100)]).is_empty());
    }

    #[test]
    fn a_length_change_applies_whole_or_not_at_all() {
        let mut c = clip(2 * BAR, &[(0, 36), (BAR + 10, 38)]);
        let shrink = Edit {
            length: Some((2 * BAR, BAR)),
            ..Edit::default()
        };
        assert_eq!(
            c.apply(&shrink),
            Err(crate::ModelError::ClipLength),
            "a note past the new end"
        );
        let stale = Edit {
            length: Some((BAR, 2 * BAR)),
            ..Edit::default()
        };
        assert_eq!(
            c.apply(&stale),
            Err(crate::ModelError::ClipLength),
            "not that long"
        );
        let whole = Edit {
            erase: vec![Note::new(BAR + 10, 60, 38, 100)],
            length: Some((2 * BAR, BAR)),
            ..Edit::default()
        };
        c.apply(&whole).unwrap();
        assert_eq!((c.length, c.notes.len()), (BAR, 1));
        c.apply(&whole.inverse()).unwrap();
        assert_eq!(ticks_of(&c), vec![(0, 36), (BAR + 10, 38)]);
        assert_eq!(c.length, 2 * BAR);
    }
}
