//! Binding a preview to a clip (phase 0B): which clip, where, and at
//! which revision, so a result made for one version of a clip is never
//! heard or kept over another.
//!
//! A clip's [`Revision`] is a hash of what the clip plays and what the
//! composer reads, worked out from the clip when asked. Nothing is kept
//! with the clip. A counter kept with the clip was the other way, but
//! every path that edits a clip (step edits, recording in place, undo and
//! redo, captures, Mutate, loading a project) would have to move it on,
//! and one that forgot would let a stale result be kept; a hash can't
//! forget. It is the play view's stamp idea (`mpfree_play`'s
//! `ClipView::stamp`), made from the clip as stored rather than as
//! compiled: a sum of one hash per note, so notes stored in another order
//! (undo puts a note back at the end) are the same revision, and the
//! same notes are the same revision however they came back.
//!
//! [`audition`] makes the project as it plays while a result is heard:
//! the result's edit applied to a copy of the bound clip with the same
//! `Clip::apply` that Keep's take uses, so what is heard is what is kept.

use mpfree_context::{ClipRead, ClipRef, Revision};
use mpfree_seq::steps::Edit;
use mpfree_seq::{Clip, Holds, ModelError, Note, Project, Quantise, Speed};
use mpfree_transport::time::Swing;

/// FNV-1a over 64-bit words.
struct Fnv(u64);

impl Fnv {
    fn new() -> Fnv {
        Fnv(0xcbf2_9ce4_8422_2325)
    }

    fn word(&mut self, w: u64) {
        for b in w.to_le_bytes() {
            self.0 ^= u64::from(b);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
}

impl std::hash::Hasher for Fnv {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= u64::from(b);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
}

/// One note's share of a revision. Every field is named, so a field added
/// to [`Note`] doesn't build until it is hashed here.
fn note_hash(n: &Note) -> u64 {
    use std::hash::Hash;
    let Note {
        tick,
        len,
        note,
        velocity,
        chance,
        cond,
        ratchet,
        touch,
        on,
        ramp,
        join,
    } = *n;
    let mut h = Fnv::new();
    h.word(tick as u64);
    h.word(len as u64);
    h.word(
        u64::from(note)
            | u64::from(velocity) << 8
            | u64::from(chance) << 16
            | u64::from(ratchet) << 24
            | u64::from(ramp as u8) << 32
            | u64::from(on) << 40,
    );
    cond.hash(&mut h);
    touch.hash(&mut h);
    join.hash(&mut h);
    // Spread the bits so a sum of many notes stays well mixed.
    splitmix(h.0)
}

fn splitmix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// The revision of what `clip` plays and what the composer reads. Every
/// field of [`Clip`] is named, so a new one has to say whether it is in.
pub fn revision(clip: &Clip) -> Revision {
    use std::hash::Hash;
    let Clip {
        // Where it is bound, not what it holds.
        id: _,
        name: _,
        length,
        speed: Speed { num, den },
        swing,
        quantise,
        notes,
        // Locks set sounds, not notes: the composer doesn't read them and
        // Keep doesn't write them.
        locks: _,
        // Perform FX gestures, likewise.
        moves: _,
        // Pressure only grows, and each note names its run in its own hash.
        touch: _,
        // What the track does after the clip, how it launches and what it
        // sends outside gear: not what it plays inside its pass.
        follow: _,
        launch: _,
        legato: _,
        once: _,
        program: _,
        looped,
        direction,
        // The step grid changes what Steps shows, never when a note plays.
        step: _,
        // Mutate's restore point: Restore changes the notes, which are in.
        original: _,
        holds,
        // An audio clip's regions: the composer reads notes, and an audio
        // clip has none.
        audio: _,
        // The synth its locks were made for: locks are out, so it is too.
        made_for: _,
    } = clip;
    let mut h = Fnv::new();
    h.word(*length as u64);
    h.word(u64::from(*num) | u64::from(*den) << 8);
    h.word(swing.map_or(0, |Swing { grid, amount }| {
        1 << 48 | u64::from(amount) << 32 | u64::from(grid)
    }));
    h.word(quantise.map_or(0, |Quantise { grid, strength }| {
        1 << 48 | u64::from(strength) << 32 | u64::from(grid)
    }));
    h.word(looped.map_or(0, |l| 0x100 | u64::from(l)));
    direction.hash(&mut h);
    h.word(match holds {
        Holds::Hits => 1,
        Holds::Notes => 2,
    });
    let sum = notes
        .iter()
        .map(note_hash)
        .fold(splitmix(h.0), u64::wrapping_add);
    Revision(sum)
}

/// The revision of the clip with lasting id `clip`, wherever it is in the
/// project's grid, or `None` if it is gone. What a reading's
/// [`mpfree_context::MusicalContext::is_stale`] asks.
pub fn revision_in(project: &Project, clip: ClipRef) -> Option<Revision> {
    project
        .cell_clips()
        .find(|(_, _, c)| c.id.0 == clip.0)
        .map(|(_, _, c)| revision(c))
}

/// Why a binding no longer holds, in the words the screen uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stale {
    /// The clip left its slot, or another took its place.
    Gone,
    /// The clip was edited since the result was made.
    Changed,
    /// A clip the reading listened to was edited or went.
    Heard,
}

impl Stale {
    pub fn words(self) -> &'static str {
        match self {
            Stale::Gone => "The clip has gone. Nothing was written.",
            Stale::Changed => "The clip changed. Try again?",
            Stale::Heard => "A clip it listened to changed. Try again?",
        }
    }
}

/// A preview's clip: where it is, which clip, and its revision when the
/// request was made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Binding {
    pub track: usize,
    pub slot: usize,
    pub clip: ClipRef,
    pub revision: Revision,
}

impl Binding {
    /// The clip in `slot` of `track` as it is now, or `None` for an empty
    /// slot.
    pub fn of(project: &Project, track: usize, slot: usize) -> Option<Binding> {
        let c = project.clip(track, slot)?;
        Some(Binding {
            track,
            slot,
            clip: ClipRef(c.id.0),
            revision: revision(c),
        })
    }

    /// The bound clip, if it is still in its place at the same revision.
    pub fn check<'p>(&self, project: &'p Project) -> Result<&'p Clip, Stale> {
        let c = project.clip(self.track, self.slot).ok_or(Stale::Gone)?;
        if c.id.0 != self.clip.0 {
            return Err(Stale::Gone);
        }
        if revision(c) != self.revision {
            return Err(Stale::Changed);
        }
        Ok(c)
    }

    /// [`Binding::check`], and every other clip the reading listened to
    /// still at the revision it heard.
    pub fn check_all<'p>(
        &self,
        project: &'p Project,
        read: &[ClipRead],
    ) -> Result<&'p Clip, Stale> {
        let c = self.check(project)?;
        if read
            .iter()
            .any(|r| revision_in(project, r.clip) != Some(r.revision))
        {
            return Err(Stale::Heard);
        }
        Ok(c)
    }
}

/// The project as it plays while a result is heard: `edit` applied to a
/// copy of the bound clip, by the same `Clip::apply` that Keep's take
/// uses, so the audition and the kept clip play the same notes. All or
/// nothing: on a refusal the project is as it was. This copies the whole
/// project; phase 1 can compile the one clip in place instead if the copy
/// shows in the timings.
pub fn audition(project: &Project, b: &Binding, edit: &Edit) -> Result<Project, ModelError> {
    b.check(project).map_err(|_| ModelError::BadSlot)?;
    let mut p = project.clone();
    let clip = p
        .tracks
        .get_mut(b.track)
        .and_then(|t| t.slots.get_mut(b.slot))
        .and_then(|s| s.as_mut())
        .ok_or(ModelError::BadSlot)?;
    clip.apply(edit)?;
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_seq::{Lock, Track};

    fn clip() -> Clip {
        let mut c = Clip::new("bass", 3840);
        c.notes = vec![
            Note::new(0, 240, 36, 100),
            Note::new(960, 240, 43, 90),
            Note::new(1920, 240, 41, 100),
        ];
        c
    }

    #[test]
    fn the_revision_follows_what_the_clip_plays() {
        let c = clip();
        let r = revision(&c);
        // Not what it is called or the sounds its locks set.
        let mut named = c.clone();
        named.name = "Bass line".into();
        assert_eq!(revision(&named), r);
        let mut locked = c.clone();
        let cutoff =
            mpfree_seq::mpfree_param::RelAddr::new(mpfree_seq::mpfree_param::OnTrack::Inst, 3);
        locked.locks.push(Lock::new(0, None, cutoff, 0.5));
        assert_eq!(revision(&locked), r);
        // Not the order the notes are stored in.
        let mut shuffled = c.clone();
        shuffled.notes.reverse();
        assert_eq!(revision(&shuffled), r);
        // Every note's every field, the length and the playback settings.
        let mut louder = c.clone();
        louder.notes[1].velocity = 91;
        assert_ne!(revision(&louder), r);
        let mut chance = c.clone();
        chance.notes[2].chance = 50;
        assert_ne!(revision(&chance), r);
        let mut longer = c.clone();
        longer.length = 7680;
        assert_ne!(revision(&longer), r);
        let mut swung = c.clone();
        swung.swing = Swing::new(240, 60);
        assert_ne!(revision(&swung), r);
        let mut notes = c.clone();
        notes.holds = Holds::Notes;
        assert_ne!(revision(&notes), r);
        // Two notes swapping pitches is a different clip.
        let mut swapped = c.clone();
        swapped.notes[0].note = 43;
        swapped.notes[1].note = 36;
        assert_ne!(revision(&swapped), r);
    }

    fn project() -> Project {
        let mut p = Project::new();
        let mut t = Track::new("Bass", 0);
        t.slots[0] = Some(clip());
        p.tracks.push(t);
        p.give_clip_ids();
        p
    }

    #[test]
    fn a_binding_holds_until_its_clip_changes_or_goes() {
        let mut p = project();
        let b = Binding::of(&p, 0, 0).unwrap();
        assert!(b.check(&p).is_ok());
        assert_eq!(revision_in(&p, b.clip), Some(b.revision));
        p.tracks[0].slots[0].as_mut().unwrap().notes[0].velocity = 80;
        assert_eq!(b.check(&p).unwrap_err(), Stale::Changed);
        // Put back as it was, it is the same revision again.
        p.tracks[0].slots[0].as_mut().unwrap().notes[0].velocity = 100;
        assert!(b.check(&p).is_ok());
        // Another clip in its place is not it, even with the same notes.
        let mut other = clip();
        other.id = mpfree_seq::ClipId(b.clip.0 + 7);
        p.tracks[0].slots[0] = Some(other);
        assert_eq!(b.check(&p).unwrap_err(), Stale::Gone);
        p.tracks[0].slots[0] = None;
        assert_eq!(b.check(&p).unwrap_err(), Stale::Gone);
        assert_eq!(Binding::of(&p, 0, 0), None);
    }

    #[test]
    fn a_clip_the_reading_heard_counts_too() {
        let mut p = project();
        let mut drums = Track::new("Drums", 1);
        let mut d = Clip::new("beat", 3840);
        d.notes = vec![Note::new(0, 60, 0, 120)];
        drums.slots[0] = Some(d);
        p.tracks.push(drums);
        p.give_clip_ids();
        let b = Binding::of(&p, 0, 0).unwrap();
        let heard = Binding::of(&p, 1, 0).unwrap();
        let read = [ClipRead {
            clip: heard.clip,
            revision: heard.revision,
            role: mpfree_context::PartRole::Drums,
        }];
        assert!(b.check_all(&p, &read).is_ok());
        p.tracks[1].slots[0].as_mut().unwrap().notes[0].tick = 10;
        assert_eq!(b.check_all(&p, &read).unwrap_err(), Stale::Heard);
        assert!(b.check(&p).is_ok(), "the bound clip itself is as it was");
    }

    #[test]
    fn an_audition_is_the_kept_clip_and_leaves_the_project_alone() {
        let p = project();
        let b = Binding::of(&p, 0, 0).unwrap();
        let edit = Edit {
            length: Some((3840, 7680)),
            add: vec![Note::new(3840, 240, 38, 100)],
            ..Edit::default()
        };
        let heard = audition(&p, &b, &edit).unwrap();
        let mut kept = p.clone();
        kept.tracks[0].slots[0]
            .as_mut()
            .unwrap()
            .apply(&edit)
            .unwrap();
        assert_eq!(heard.clip(0, 0), kept.clip(0, 0));
        assert!(b.check(&p).is_ok(), "the project itself is untouched");
        // Refused whole: a note past the clip's end with no new length.
        let bad = Edit {
            add: vec![Note::new(3840, 240, 38, 100)],
            ..Edit::default()
        };
        assert!(audition(&p, &b, &bad).is_err());
    }
}
