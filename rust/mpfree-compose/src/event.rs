//! What the composer makes: phrases of events, each with the role it plays
//! over the chord and who made it, and the decisions behind them.
//!
//! A part keeps its **decisions** (when each note is, how long, how loud,
//! what it does over the chord, which way it moves, which motif it comes
//! from) apart from its **notes**. Turning decisions into notes under a
//! harmony is [`crate::realize`]: when the chords change, the same
//! decisions are realised again (re-fitted), so the bass keeps its rhythm
//! and shape and only its notes move.

use mpfree_context::chord::Tone;
use mpfree_context::groove::Lane;
use mpfree_context::PartRole;
use mpfree_transport::time::Tick;

use crate::profile::InstrumentKind;
use crate::timing::Timing;

/// Who made a note. Only notes a person played are ever learned from,
/// and generated notes are recorded as heard and never sent through the
/// arp again (contract 8 of the design).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    /// A person played or wrote it.
    You,
    /// The composer made it.
    Mpfree,
    /// An arp pattern played it.
    Pattern,
}

impl Source {
    /// Notes the composer may learn from.
    pub const fn learnable(self) -> bool {
        matches!(self, Source::You)
    }
}

/// What a note does over the chord under it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Role {
    /// A chord tone: root, third, fifth or seventh.
    Chord(Tone),
    /// A colour tone over the chord: its 9th or 13th, in key.
    Colour,
    /// A note of the key (or of the notes played) that isn't a chord tone.
    Scale,
    /// A half step into the next note.
    Approach,
    /// A step between the notes either side.
    Passing,
    /// A step away from the note before, and usually back.
    Neighbour,
    /// A quiet repeat of the note before.
    Ghost,
    /// A drum hit: its pitch is the pad's note, fixed.
    Drum(Lane),
}

impl Role {
    /// Roles realised from the chord and key alone; the others lean on the
    /// notes either side.
    pub const fn anchors(self) -> bool {
        matches!(
            self,
            Role::Chord(_) | Role::Colour | Role::Scale | Role::Drum(_)
        )
    }
}

/// Which way a note moves, and roughly how far. An anchor (a chord tone,
/// colour or scale note) moves from the anchor before it; a passing or
/// neighbour note from the note before it; an approach says which side it
/// comes into the next note from (up is from below).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Motion {
    /// -1 down, 0 the same or nearest, 1 up.
    pub dir: i8,
    /// About how many semitones.
    pub size: u8,
}

impl Motion {
    pub const fn up(size: u8) -> Motion {
        Motion { dir: 1, size }
    }
    pub const fn down(size: u8) -> Motion {
        Motion { dir: -1, size }
    }
    pub const NEAREST: Motion = Motion { dir: 0, size: 0 };
}

/// A note of a motif this decision comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MotifRef {
    pub motif: u16,
    pub note: u16,
}

/// One note's decision: everything about it except which pitch it is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NoteDecision {
    /// Musical position, in the part's ticks.
    pub at: Tick,
    pub len: Tick,
    pub velocity: u8,
    pub role: Role,
    pub motion: Motion,
    pub motif: Option<MotifRef>,
    /// A pitch that must not change: a locked note, a note held into a
    /// selection, a drum pad's note.
    pub fixed: Option<u8>,
    /// The drum lane whose groove it follows.
    pub lane: Lane,
    pub timing: Timing,
    pub source: Source,
}

impl NoteDecision {
    /// A plain decision made by the composer, following the kick.
    pub fn new(at: Tick, len: Tick, velocity: u8, role: Role, motion: Motion) -> NoteDecision {
        NoteDecision {
            at,
            len,
            velocity,
            role,
            motion,
            motif: None,
            fixed: None,
            lane: Lane::Kick,
            timing: Timing::default(),
            source: Source::Mpfree,
        }
    }
}

/// A part's lasting id within a request (melody, bass, chords, drums).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PartId(pub u16);

/// A part: what it does, what plays it, where it sits, and its decisions
/// in time order.
#[derive(Clone, Debug, PartialEq)]
pub struct Part {
    pub id: PartId,
    pub role: PartRole,
    pub kind: InstrumentKind,
    /// The note it aims its first note at.
    pub register: u8,
    pub decisions: Vec<NoteDecision>,
}

/// A note as made: a decision realised.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Event {
    pub at: Tick,
    pub len: Tick,
    pub pitch: u8,
    pub velocity: u8,
    pub role: Role,
    pub source: Source,
    pub timing: Timing,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_your_notes_are_learned_from() {
        assert!(Source::You.learnable());
        assert!(!Source::Mpfree.learnable());
        assert!(!Source::Pattern.learnable());
    }

    #[test]
    fn anchors_are_the_roles_that_stand_alone() {
        assert!(Role::Chord(Tone::Root).anchors());
        assert!(Role::Drum(Lane::Kick).anchors());
        assert!(!Role::Approach.anchors());
        assert!(!Role::Ghost.anchors());
    }
}
