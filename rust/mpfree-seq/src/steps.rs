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
//! Edits never change the clip themselves: they return an [`Edit`], the
//! notes to take out and the notes to put in, for the control thread to
//! apply as one undoable take (`Takes::edit` in `mpfree-play`).

use crate::{Clip, Note, MAX_CLIP_TICKS, MIN_CLIP_TICKS};
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
    /// Notes of the sound in the step, 1 or more.
    pub notes: usize,
}

/// A change to a clip: take `erase` out, change the length from and to
/// `length` when there is one, then put `add` in.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Edit {
    pub erase: Vec<Note>,
    pub add: Vec<Note>,
    pub length: Option<(Tick, Tick)>,
}

impl Edit {
    pub fn is_empty(&self) -> bool {
        self.erase.is_empty() && self.add.is_empty() && self.length.is_none()
    }

    /// The edit that puts the clip back as it was.
    pub fn inverse(&self) -> Edit {
        Edit { erase: self.add.clone(), add: self.erase.clone(), length: self.length.map(|(a, b)| (b, a)) }
    }

    fn join(&mut self, other: Edit) {
        self.erase.extend(other.erase);
        self.add.extend(other.add);
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
        };
        (kind << 56) | (u64::from(pitch) << 48) | (i as u64 & 0xFFFF_FFFF_FFFF)
    }

    /// The key for this kind of change on every sound of step `i`
    /// ([`Grid::change_all`]).
    pub fn key_all(self, i: usize) -> u64 {
        self.key(ALL, i)
    }
}

/// No sound has this number: it names every sound of a step in a key.
const ALL: u8 = 255;

impl Grid {
    /// Steps of `size` ticks, from a 1/32 to a whole note.
    pub fn new(size: Tick) -> Option<Grid> {
        (MIN_CLIP_TICKS..=4 * Tick::from(PPQ)).contains(&size).then_some(Grid { size })
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
        let lo = if i == 0 { (self.steps(length) as Tick * self.size - self.size / 2 - length).min(0) } else { -(self.size / 2) };
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
            let step = Step { velocity: n.velocity, len: n.len, nudge, chance: n.chance, notes: 1 };
            *slot = Some(match *slot {
                None => step,
                Some(s) => {
                    let first = if (nudge, n.len, n.velocity, n.chance) < (s.nudge, s.len, s.velocity, s.chance) { step } else { s };
                    Step { notes: s.notes + 1, ..first }
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
            return Edit { erase: notes, ..Edit::default() };
        }
        let note = Note::new(self.tick(clip.length, i, 0), self.size, pitch.min(127), velocity.clamp(1, 127));
        Edit { add: vec![note], ..Edit::default() }
    }

    /// Change step `i` of sound `pitch`. Values are kept in range; an
    /// empty step, or one already so, gives an empty edit.
    pub fn change(self, clip: &Clip, pitch: u8, i: usize, change: Change) -> Edit {
        let notes = self.notes(clip, pitch, i);
        let Some(first) = notes.first() else { return Edit::default() };
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
        edit
    }

    /// Change step `i` of every sound in it, as [`Grid::change`] does for
    /// one sound. Each sound's notes move by their own nudge.
    pub fn change_all(self, clip: &Clip, i: usize, change: Change) -> Edit {
        let mut edit = Edit::default();
        for pitch in self.sounds_in(clip, i) {
            edit.join(self.change(clip, pitch, i, change));
        }
        edit
    }

    /// Every note in step `i`, all sounds: lowest sound first, then the
    /// earliest note first. A melodic track's step shows its chord this way.
    pub fn chord(self, clip: &Clip, i: usize) -> Vec<Note> {
        self.sounds_in(clip, i).into_iter().flat_map(|pitch| self.notes(clip, pitch, i)).collect()
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
                add.push(Note::new(tick, self.size, pitch.min(127), velocity.clamp(1, 127)));
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
        Edit { erase, add, length: None }
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
        let mut edit = Edit { length: Some((old, new)), ..Edit::default() };
        for note in &clip.notes {
            let (i, nudge) = self.place(old, note.tick);
            if i >= steps {
                edit.erase.push(*note);
            } else if i == 0 && nudge < 0 {
                // Early on step 0: it stays that far before the clip's end.
                let (lo, _) = self.nudges(new, 0);
                let moved = Note { tick: self.tick(new, 0, nudge.max(lo)), ..*note };
                if moved != *note {
                    edit.erase.push(*note);
                    edit.add.push(moved);
                }
            }
        }
        if copy && steps > n {
            let width = n.min(PAGE);
            for j in 0..steps - n {
                edit.add.extend(self.moved(clip, n - width + j % width, new, n + j));
            }
        }
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
            if src < n {
                edit.add.extend(self.moved(clip, src, clip.length, dst));
            }
        }
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
                Note { tick: self.tick(length, dst, nudge.clamp(lo, hi)), ..n }
            })
            .collect()
    }

    /// The sounds with notes in step `i`, lowest first.
    fn sounds_in(self, clip: &Clip, i: usize) -> Vec<u8> {
        let mut s: Vec<u8> =
            clip.notes.iter().filter(|n| self.place(clip.length, n.tick).0 == i).map(|n| n.note).collect();
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

    /// Apply `edit`: take its notes out, change the length, then put its
    /// new ones in. All or nothing: if a note to take out is not in the
    /// clip (`MissingNote`), the clip is not `length.0` long or a note
    /// would be past its new end (`ClipLength`), or a new note does not
    /// fit, the clip is unchanged and the error says which.
    pub fn apply(&mut self, edit: &Edit) -> Result<(), crate::ModelError> {
        let (notes, length) = (self.notes.clone(), self.length);
        let result = self.apply_all(edit);
        if result.is_err() {
            (self.notes, self.length) = (notes, length);
        }
        result
    }

    fn apply_all(&mut self, edit: &Edit) -> Result<(), crate::ModelError> {
        for n in &edit.erase {
            if !self.remove_note(n) {
                return Err(crate::ModelError::MissingNote);
            }
        }
        if let Some((from, to)) = edit.length {
            let fits = (MIN_CLIP_TICKS..=MAX_CLIP_TICKS).contains(&to) && self.notes.iter().all(|n| n.tick < to);
            if self.length != from || !fits {
                return Err(crate::ModelError::ClipLength);
            }
            self.length = to;
        }
        for n in &edit.add {
            self.add_note(*n)?;
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
        c.notes = notes.iter().map(|&(tick, note)| Note::new(tick, 60, note, 100)).collect();
        c
    }

    #[test]
    fn every_tick_has_one_step_and_comes_back_from_it() {
        for size in SIZES {
            for length in [BAR, 2 * BAR, 1000, 120, 3 * 320 + 7, MIN_CLIP_TICKS] {
                let g = Grid::new(size).unwrap();
                for tick in 0..length {
                    let (i, nudge) = g.place(length, tick);
                    assert!(i < g.steps(length), "size {size} length {length} tick {tick}");
                    let (lo, hi) = g.nudges(length, i);
                    assert!((lo..=hi).contains(&nudge), "size {size} length {length} tick {tick}: {nudge} not in {lo}..={hi}");
                    assert_eq!(g.tick(length, i, nudge), tick);
                }
                // Every nudge a step allows lands back on that step.
                for i in 0..g.steps(length) {
                    let (lo, hi) = g.nudges(length, i);
                    for nudge in [lo, hi, 0.clamp(lo, hi)] {
                        assert_eq!(g.place(length, g.tick(length, i, nudge)), (i, nudge), "size {size} length {length} step {i}");
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
        assert_eq!((g.steps(1000), g.pages(1000)), (5, 1), "the last step is cut short");
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
        let mut c = clip(BAR, &[(0, 36), (490, 36), (1440, 36), (1500, 36), (3800, 36), (240, 38)]);
        c.notes[3].velocity = 70;
        let row = g.row(&c, 36);
        assert_eq!(row.len(), 16);
        let on: Vec<(usize, Tick, usize)> =
            row.iter().enumerate().filter_map(|(i, s)| s.map(|s| (i, s.nudge, s.notes))).collect();
        assert_eq!(on, vec![(0, -40, 2), (2, 10, 1), (6, 0, 2)]);
        assert_eq!(row[6].unwrap().velocity, 100, "the first note of the roll");
        assert_eq!(g.notes(&c, 36, 0).iter().map(|n| n.tick).collect::<Vec<_>>(), vec![3800, 0]);
        assert_eq!(c.sounds(), vec![36, 38]);
    }

    #[test]
    fn tapping_a_step_adds_a_note_or_takes_its_notes_out() {
        let g = Grid::new(240).unwrap();
        let mut c = clip(BAR, &[(1440, 36), (1500, 36)]);
        let on = g.toggle(&c, 36, 4, 90);
        assert_eq!(on, Edit { add: vec![Note::new(960, 240, 36, 90)], ..Edit::default() });
        c.apply(&on).unwrap();
        assert_eq!(g.row(&c, 36)[4].map(|s| (s.velocity, s.len, s.nudge, s.chance)), Some((90, 240, 0, 100)));
        let off = g.toggle(&c, 36, 6, 90);
        assert_eq!(off.erase.len(), 2, "the whole roll");
        c.apply(&off).unwrap();
        assert_eq!(c.notes, vec![Note::new(960, 240, 36, 90)]);
        assert!(g.toggle(&c, 36, 16, 90).is_empty(), "past the last step");
    }

    #[test]
    fn changes_reach_every_note_in_the_step_and_stay_in_range() {
        let g = Grid::new(240).unwrap();
        let c = clip(BAR, &[(1440, 36), (1500, 36), (1440, 38)]);
        let e = g.change(&c, 36, 6, Change::Velocity(200));
        assert_eq!(e.add.iter().map(|n| n.velocity).collect::<Vec<_>>(), vec![127, 127]);
        assert!(e.add.iter().all(|n| n.note == 36), "only the step's sound");
        assert_eq!(g.change(&c, 36, 6, Change::Chance(0)).add[0].chance, 1);
        assert_eq!(g.change(&c, 36, 6, Change::Length(0)).add[0].len, 1);
        assert!(g.change(&c, 36, 6, Change::Velocity(100)).is_empty(), "already so");
        assert!(g.change(&c, 36, 5, Change::Velocity(1)).is_empty(), "an empty step");
        // A nudge moves the roll together, and no note leaves the step.
        let e = g.change(&c, 36, 6, Change::Nudge(100));
        assert_eq!(e.add.iter().map(|n| n.tick).collect::<Vec<_>>(), vec![1540, 1559]);
        let e = g.change(&c, 36, 6, Change::Nudge(-500));
        assert_eq!(e.add.iter().map(|n| n.tick).collect::<Vec<_>>(), vec![1320, 1380]);
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
        assert_eq!(g.nudges(1000, 4), (-120, 39), "the last step ends with the clip");
    }

    #[test]
    fn an_edit_applies_whole_or_not_at_all() {
        let mut c = clip(BAR, &[(0, 36)]);
        let bad = Edit { erase: vec![Note::new(0, 60, 36, 100)], add: vec![Note::new(BAR, 60, 36, 100)], length: None };
        assert!(c.apply(&bad).is_err());
        assert_eq!(c.notes, vec![Note::new(0, 60, 36, 100)]);
        let missing = Edit { erase: vec![Note::new(5, 60, 36, 100)], ..Edit::default() };
        assert!(c.apply(&missing).is_err());
        assert_eq!(c.notes.len(), 1);
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
        assert_eq!(ticks_of(&c), vec![(0, 36), (1920, 36), (BAR - 30, 42), (BAR, 36), (BAR + 1920, 36), (2 * BAR - 30, 42)]);
        let on = |c: &Clip, p| g.row(c, p).iter().enumerate().filter_map(|(i, s)| s.map(|s| (i, s.nudge))).collect::<Vec<_>>();
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
        assert_eq!(ticks_of(&c), vec![(0, 36), (5 * 240 - 30, 42)], "step 8's kick went");
        assert!(g.resize(&c, 5, true).is_empty());
        assert_eq!(g.resize(&c, usize::MAX, false).length, Some((5 * 240, MAX_CLIP_TICKS)));
    }

    #[test]
    fn a_short_clip_repeats_to_fill_a_copied_page() {
        let g = Grid::new(240).unwrap();
        // Three steps: kick, hat 10 ticks late, nothing.
        let mut c = clip(3 * 240, &[(0, 36), (250, 42)]);
        c.apply(&g.add_page(&c, true)).unwrap();
        assert_eq!(g.steps(c.length), 19);
        let row = |p| g.row(&c, p).iter().enumerate().filter_map(|(i, s)| s.map(|s| (i, s.nudge))).collect::<Vec<_>>();
        assert_eq!(row(36), vec![(0, 0), (3, 0), (6, 0), (9, 0), (12, 0), (15, 0), (18, 0)]);
        assert_eq!(row(42), vec![(1, 10), (4, 10), (7, 10), (10, 10), (13, 10), (16, 10)]);
    }

    #[test]
    fn a_page_copies_over_another() {
        let g = Grid::new(240).unwrap();
        let mut c = clip(2 * BAR, &[(0, 36), (250, 42), (BAR + 480, 38)]);
        c.apply(&g.copy_page(&c, 0, 1)).unwrap();
        assert_eq!(ticks_of(&c), vec![(0, 36), (250, 42), (BAR, 36), (BAR + 250, 42)]);
        assert!(g.copy_page(&c, 0, 2).is_empty(), "no page 3");
        assert!(g.copy_page(&c, 1, 1).is_empty());
    }

    #[test]
    fn a_held_chord_goes_on_a_step() {
        let g = Grid::new(240).unwrap();
        let mut c = clip(BAR, &[(240, 60)]);
        // C minor held on step 1: C stays as it was, E flat and G come in.
        let e = g.enter(&c, 1, &[(60, 100), (63, 90), (67, 80), (63, 90)]);
        assert_eq!(e.erase, vec![Note::new(240, 60, 60, 100)], "the old C was 60 ticks long");
        assert_eq!(e.add.len(), 3);
        c.apply(&e).unwrap();
        let chord: Vec<(u8, u8)> = g.chord(&c, 1).iter().map(|n| (n.note, n.velocity)).collect();
        assert_eq!(chord, vec![(60, 100), (63, 90), (67, 80)]);
        // The whole chord's velocity and nudge change together.
        c.apply(&g.change_all(&c, 1, Change::Nudge(-20))).unwrap();
        assert!(g.chord(&c, 1).iter().all(|n| n.tick == 220));
        assert_ne!(Change::Velocity(1).key_all(1), Change::Velocity(1).key(127, 1));
        // No keys clears the step; nothing past the end.
        c.apply(&g.enter(&c, 1, &[])).unwrap();
        assert!(c.notes.is_empty());
        assert!(g.enter(&c, 16, &[(60, 100)]).is_empty());
    }

    #[test]
    fn a_length_change_applies_whole_or_not_at_all() {
        let mut c = clip(2 * BAR, &[(0, 36), (BAR + 10, 38)]);
        let shrink = Edit { length: Some((2 * BAR, BAR)), ..Edit::default() };
        assert_eq!(c.apply(&shrink), Err(crate::ModelError::ClipLength), "a note past the new end");
        let stale = Edit { length: Some((BAR, 2 * BAR)), ..Edit::default() };
        assert_eq!(c.apply(&stale), Err(crate::ModelError::ClipLength), "not that long");
        let whole = Edit { erase: vec![Note::new(BAR + 10, 60, 38, 100)], length: Some((2 * BAR, BAR)), ..Edit::default() };
        c.apply(&whole).unwrap();
        assert_eq!((c.length, c.notes.len()), (BAR, 1));
        c.apply(&whole.inverse()).unwrap();
        assert_eq!(ticks_of(&c), vec![(0, 36), (BAR + 10, 38)]);
        assert_eq!(c.length, 2 * BAR);
    }
}
