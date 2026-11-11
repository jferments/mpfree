//! Take undo, on the control thread: every note a take recorded or erased,
//! kept by take, so the last take can be undone and redone as a whole.
//!
//! The control thread hands each period's [`Player::erased`] and
//! [`Recorder::taken`] notes to [`Takes::erase`] and [`Takes::add`], which
//! change the clips and remember what changed. [`Takes::undo`] puts the
//! erased notes back and takes the recorded ones out; [`Takes::redo`] does
//! the take again. Notes are named by value, as the clips store them, so
//! edits to other notes in between do not disturb them.
//!
//! A take's notes can still arrive after the next take has begun: a note
//! that quantise moves past a punch out waits for the playhead. Each note
//! carries its take's number, so it joins the right take. A take that is
//! undone while it is still recording goes on as a new take, and redo can
//! no longer bring back the undone part.
//!
//! CAPTURE keeps what was just played as a take of its own
//! ([`Takes::capture_clip`] for a new clip, [`Takes::capture_notes`] for
//! notes added to a clip that was playing), so UNDO takes a capture back
//! like any take. Captures are numbered on their own, from 1.
//!
//! This allocates; it never runs on the audio thread.
//!
//! [`Player::erased`]: crate::Player::erased
//! [`Recorder::taken`]: crate::Recorder::taken

use mpfree_seq::{Clip, ModelError, Note, Project};

use crate::{Erased, Taken};

/// Takes kept for undo when no limit is given.
pub const DEFAULT_LIMIT: usize = 32;

/// What one take changed. Each note is (track, slot, note).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Take {
    pub number: u32,
    /// Made by CAPTURE, not by recording.
    pub captured: bool,
    pub added: Vec<(usize, usize, Note)>,
    pub erased: Vec<(usize, usize, Note)>,
    /// Clips the take put in empty slots, as (track, slot, clip). Undo
    /// empties the slot and keeps the clip as it was then; redo puts it
    /// back if the slot is still empty.
    pub made: Vec<(usize, usize, Clip)>,
}

/// The takes that can be undone, oldest first, and the ones that can be
/// redone.
#[derive(Clone, Debug)]
pub struct Takes {
    done: Vec<Take>,
    undone: Vec<Take>,
    limit: usize,
    /// Captures so far, to number them.
    captures: u32,
}

impl Default for Takes {
    fn default() -> Self {
        Takes::new(DEFAULT_LIMIT)
    }
}

impl Takes {
    /// Keep up to `limit` takes for undo (at least 1); older ones are
    /// forgotten and stay as they are.
    pub fn new(limit: usize) -> Takes {
        Takes { done: vec![], undone: vec![], limit: limit.max(1), captures: 0 }
    }

    /// Add a recorded note to its clip and remember it with its take.
    /// `BadSlot` if the clip is gone; the clip's own refusals as they are.
    pub fn add(&mut self, project: &mut Project, k: &Taken) -> Result<(), ModelError> {
        let clip = clip(project, k.track, k.slot).ok_or(ModelError::BadSlot)?;
        clip.add_note(k.note)?;
        self.entry(k.take).added.push((k.track, k.slot, k.note));
        Ok(())
    }

    /// Remove an erased note from its clip and remember it with its take.
    /// False if the clip does not hold it (it was edited meanwhile); then
    /// nothing is remembered.
    pub fn erase(&mut self, project: &mut Project, e: &Erased) -> bool {
        let Some(clip) = clip(project, e.track, e.slot) else { return false };
        if !clip.remove_note(&e.note) {
            return false;
        }
        self.entry(e.take).erased.push((e.track, e.slot, e.note));
        true
    }

    /// CAPTURE into an empty slot: put `new` in clip `slot` of `track`, as
    /// a take of its own. The capture's number. `BadSlot` if there is no
    /// such slot or it holds a clip; the clip's own refusals as they are.
    /// Nothing changes on an error.
    pub fn capture_clip(&mut self, project: &mut Project, track: usize, slot: usize, new: Clip)
        -> Result<u32, ModelError> {
        new.check()?;
        let s = slot_mut(project, track, slot).ok_or(ModelError::BadSlot)?;
        if s.is_some() {
            return Err(ModelError::BadSlot);
        }
        *s = Some(new.clone());
        let take = self.next_capture();
        take.made.push((track, slot, new));
        Ok(take.number)
    }

    /// CAPTURE into a clip that holds notes already (the one that was
    /// playing): add `notes` to clip `slot` of `track`, as a take of its
    /// own. The capture's number. All the notes go in or none: `BadSlot` if
    /// the clip is gone, `TooManyNotes` or `BadNote` (its index in
    /// `notes`) if one does not fit.
    pub fn capture_notes(&mut self, project: &mut Project, track: usize, slot: usize, notes: &[Note])
        -> Result<u32, ModelError> {
        let c = clip(project, track, slot).ok_or(ModelError::BadSlot)?;
        let before = c.notes.len();
        for (i, n) in notes.iter().enumerate() {
            if let Err(e) = c.add_note(*n) {
                c.notes.truncate(before);
                return Err(match e {
                    ModelError::BadNote(_) => ModelError::BadNote(i),
                    e => e,
                });
            }
        }
        let take = self.next_capture();
        take.added.extend(notes.iter().map(|&n| (track, slot, n)));
        Ok(take.number)
    }

    /// Undo the latest take: its recorded notes come out, its erased notes
    /// go back and the clips it made leave their slots. Its number, or
    /// None if there is nothing to undo. Notes whose clip is gone, or no
    /// longer has room or reach for them (it was shortened), are skipped.
    pub fn undo(&mut self, project: &mut Project) -> Option<u32> {
        let mut take = self.done.pop()?;
        for (track, slot, note) in take.added.iter().rev() {
            if let Some(c) = clip(project, *track, *slot) {
                c.remove_note(note);
            }
        }
        for (track, slot, note) in take.erased.iter().rev() {
            if let Some(c) = clip(project, *track, *slot) {
                let _ = c.add_note(*note);
            }
        }
        for (track, slot, made) in take.made.iter_mut().rev() {
            // Kept as it is now, with any notes recorded into it since.
            if let Some(c) = slot_mut(project, *track, *slot).and_then(Option::take) {
                *made = c;
            }
        }
        let n = take.number;
        self.undone.push(take);
        Some(n)
    }

    /// Redo the latest undone take. Its number, or None.
    pub fn redo(&mut self, project: &mut Project) -> Option<u32> {
        let take = self.undone.pop()?;
        for (track, slot, made) in &take.made {
            // A slot filled meanwhile keeps its clip.
            if let Some(s) = slot_mut(project, *track, *slot).filter(|s| s.is_none()) {
                *s = Some(made.clone());
            }
        }
        for (track, slot, note) in &take.erased {
            if let Some(c) = clip(project, *track, *slot) {
                c.remove_note(note);
            }
        }
        for (track, slot, note) in &take.added {
            if let Some(c) = clip(project, *track, *slot) {
                let _ = c.add_note(*note);
            }
        }
        let n = take.number;
        self.done.push(take);
        Some(n)
    }

    /// The takes that can be undone, oldest first.
    pub fn done(&self) -> &[Take] {
        &self.done
    }

    /// The takes that can be redone, the next one last.
    pub fn undone(&self) -> &[Take] {
        &self.undone
    }

    /// Captures so far: the next one is number `captures() + 1`.
    pub fn captures(&self) -> u32 {
        self.captures
    }

    /// Forget every take, for example when another project is loaded.
    pub fn clear(&mut self) {
        self.done.clear();
        self.undone.clear();
    }

    /// The entry for recorded take `number`: an earlier one still being
    /// finished, or a new one, which ends redo.
    fn entry(&mut self, number: u32) -> &mut Take {
        if let Some(i) = self.done.iter().rposition(|t| t.number == number && !t.captured) {
            return &mut self.done[i];
        }
        self.push(Take { number, ..Take::default() })
    }

    /// A new capture, which ends redo.
    fn next_capture(&mut self) -> &mut Take {
        self.captures += 1;
        self.push(Take { number: self.captures, captured: true, ..Take::default() })
    }

    fn push(&mut self, take: Take) -> &mut Take {
        self.undone.clear();
        if self.done.len() == self.limit {
            self.done.remove(0);
        }
        self.done.push(take);
        self.done.last_mut().expect("just pushed")
    }
}

fn slot_mut(project: &mut Project, track: usize, slot: usize) -> Option<&mut Option<Clip>> {
    project.tracks.get_mut(track)?.slots.get_mut(slot)
}

fn clip(project: &mut Project, track: usize, slot: usize) -> Option<&mut Clip> {
    slot_mut(project, track, slot)?.as_mut()
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_seq::{Track, SLOTS};

    fn project() -> Project {
        let mut p = Project::new();
        let mut t = Track::new("t", 0);
        t.slots[0] = Some(Clip::new("c", 3840));
        p.tracks.push(t);
        p
    }

    fn taken(tick: i64, take: u32) -> Taken {
        Taken { track: 0, slot: 0, note: Note { tick, len: 10, note: 1, velocity: 100 }, take }
    }

    fn ticks(p: &Project) -> Vec<i64> {
        p.tracks[0].slots[0].as_ref().unwrap().notes.iter().map(|n| n.tick).collect()
    }

    #[test]
    fn a_new_take_ends_redo() {
        let mut p = project();
        let mut t = Takes::default();
        t.add(&mut p, &taken(0, 1)).unwrap();
        t.add(&mut p, &taken(100, 2)).unwrap();
        assert_eq!(t.undo(&mut p), Some(2));
        assert_eq!(t.undone().len(), 1);
        t.add(&mut p, &taken(200, 3)).unwrap();
        assert!(t.undone().is_empty());
        assert_eq!(t.redo(&mut p), None);
        assert_eq!(ticks(&p), vec![0, 200]);
    }

    #[test]
    fn old_takes_are_forgotten_past_the_limit() {
        let mut p = project();
        let mut t = Takes::new(2);
        for take in 1..=3 {
            t.add(&mut p, &taken(i64::from(take) * 100, take)).unwrap();
        }
        let kept: Vec<u32> = t.done().iter().map(|t| t.number).collect();
        assert_eq!(kept, vec![2, 3]);
        assert_eq!(t.undo(&mut p), Some(3));
        assert_eq!(t.undo(&mut p), Some(2));
        assert_eq!(t.undo(&mut p), None);
        assert_eq!(ticks(&p), vec![100], "take 1 stays");
    }

    #[test]
    fn a_captured_clip_is_undone_and_redone_as_a_take() {
        let mut p = project();
        let mut t = Takes::default();
        t.add(&mut p, &taken(0, 1)).unwrap();
        let mut c = Clip::new("Capture 1", 3840);
        c.add_note(Note { tick: 240, len: 10, note: 4, velocity: 90 }).unwrap();
        assert_eq!(t.capture_clip(&mut p, 0, 1, c.clone()), Ok(1));
        assert_eq!(p.clip(0, 1), Some(&c));
        // Recorded take 1 goes on: its notes still join take 1, not the
        // capture, though both are number 1.
        t.add(&mut p, &taken(100, 1)).unwrap();
        assert_eq!(t.done().iter().map(|t| (t.number, t.captured, t.added.len())).collect::<Vec<_>>(),
                   vec![(1, false, 2), (1, true, 0)]);
        // Notes recorded into the captured clip are kept with it on undo.
        p.tracks[0].slots[1].as_mut().unwrap().add_note(Note { tick: 0, len: 5, note: 1, velocity: 1 }).unwrap();
        assert_eq!(t.undo(&mut p), Some(1));
        assert!(p.clip(0, 1).is_none());
        assert_eq!(t.redo(&mut p), Some(1));
        assert_eq!(p.clip(0, 1).map(|c| c.notes.len()), Some(2));
        // Undone again, and the slot filled meanwhile: redo leaves it.
        t.undo(&mut p);
        p.tracks[0].slots[1] = Some(Clip::new("mine", 960));
        assert_eq!(t.redo(&mut p), Some(1));
        assert_eq!(p.clip(0, 1).map(|c| c.name.as_str()), Some("mine"));
        // The second capture is number 2; a full slot is refused.
        assert_eq!(t.capture_clip(&mut p, 0, 1, c.clone()), Err(ModelError::BadSlot));
        assert_eq!(t.capture_clip(&mut p, 0, 2, c.clone()), Ok(2));
        assert_eq!(t.capture_clip(&mut p, 0, SLOTS, c), Err(ModelError::BadSlot));
    }

    #[test]
    fn captured_notes_go_in_together_or_not_at_all() {
        let mut p = project();
        let mut t = Takes::default();
        let n = |tick| Note { tick, len: 10, note: 2, velocity: 100 };
        assert_eq!(t.capture_notes(&mut p, 0, 0, &[n(0), n(5000)]), Err(ModelError::BadNote(1)));
        assert!(ticks(&p).is_empty() && t.done().is_empty());
        assert_eq!(t.capture_notes(&mut p, 0, 3, &[n(0)]), Err(ModelError::BadSlot));
        assert_eq!(t.capture_notes(&mut p, 0, 0, &[n(0), n(960)]), Ok(1));
        assert_eq!(ticks(&p), vec![0, 960]);
        assert_eq!(t.undo(&mut p), Some(1));
        assert!(ticks(&p).is_empty());
        assert_eq!(t.redo(&mut p), Some(1));
        assert_eq!(ticks(&p), vec![0, 960]);
    }

    #[test]
    fn missing_clips_and_notes_change_nothing() {
        let mut p = project();
        let mut t = Takes::default();
        let gone = Taken { slot: 5, ..taken(0, 1) };
        assert_eq!(t.add(&mut p, &gone), Err(ModelError::BadSlot));
        let e = Erased { track: 0, slot: 0, note: taken(0, 1).note, take: 1 };
        assert!(!t.erase(&mut p, &e), "the clip does not hold it");
        assert!(t.done().is_empty());
        // A note out of the clip's range is refused, and not remembered.
        assert!(t.add(&mut p, &taken(5000, 1)).is_err());
        assert!(t.done().is_empty());
    }
}
