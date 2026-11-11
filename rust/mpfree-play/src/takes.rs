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
//! This allocates; it never runs on the audio thread.
//!
//! [`Player::erased`]: crate::Player::erased
//! [`Recorder::taken`]: crate::Recorder::taken

use mpfree_seq::{Clip, ModelError, Note, Project};

use crate::{Erased, Taken};

/// Takes kept for undo when no limit is given.
pub const DEFAULT_LIMIT: usize = 32;

/// What one take changed. Each note is (track, slot, note).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Take {
    pub number: u32,
    pub added: Vec<(usize, usize, Note)>,
    pub erased: Vec<(usize, usize, Note)>,
}

/// The takes that can be undone, oldest first, and the ones that can be
/// redone.
#[derive(Clone, Debug)]
pub struct Takes {
    done: Vec<Take>,
    undone: Vec<Take>,
    limit: usize,
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
        Takes { done: vec![], undone: vec![], limit: limit.max(1) }
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

    /// Undo the latest take: its recorded notes come out and its erased
    /// notes go back. Its number, or None if there is nothing to undo.
    /// Notes whose clip is gone, or no longer has room or reach for them
    /// (it was shortened), are skipped.
    pub fn undo(&mut self, project: &mut Project) -> Option<u32> {
        let take = self.done.pop()?;
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
        let n = take.number;
        self.undone.push(take);
        Some(n)
    }

    /// Redo the latest undone take. Its number, or None.
    pub fn redo(&mut self, project: &mut Project) -> Option<u32> {
        let take = self.undone.pop()?;
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

    /// Forget every take, for example when another project is loaded.
    pub fn clear(&mut self) {
        self.done.clear();
        self.undone.clear();
    }

    /// The entry for take `number`: an earlier one still being finished, or
    /// a new one, which ends redo.
    fn entry(&mut self, number: u32) -> &mut Take {
        if let Some(i) = self.done.iter().rposition(|t| t.number == number) {
            return &mut self.done[i];
        }
        self.undone.clear();
        if self.done.len() == self.limit {
            self.done.remove(0);
        }
        self.done.push(Take { number, ..Take::default() });
        self.done.last_mut().expect("just pushed")
    }
}

fn clip(project: &mut Project, track: usize, slot: usize) -> Option<&mut Clip> {
    project.tracks.get_mut(track)?.slots.get_mut(slot)?.as_mut()
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_seq::Track;

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
