//! What a change re-fits (contract 2 of the design).
//!
//! Two kinds of change: making new decisions (**regenerating**) and
//! playing the same decisions under new conditions (**re-fitting**, the
//! review's "re-realization"). Changing a chord re-fits the bass; it
//! doesn't write a new one. [`effect`] is the table; [`action`] says what
//! happens to one part, depending on whether it is in a preview, kept, or
//! locked. A clip the musician kept is never rewritten: it is offered
//! "Fit to the new chords" when its notes no longer fit ([`misfits`]).

use mpfree_context::harmony::Harmony;
use mpfree_transport::time::{Tick, PPQ};

use crate::realize::PlainNote;

/// What a musician can change that parts depend on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Change {
    Chord,
    /// The drums the parts follow.
    Groove,
    /// A bar other bars are made from.
    Motif,
    Style,
    Instrument,
    Activity,
    /// Loudness and note lengths.
    Playing,
}

impl Change {
    pub const ALL: [Change; 7] = [
        Change::Chord,
        Change::Groove,
        Change::Motif,
        Change::Style,
        Change::Instrument,
        Change::Activity,
        Change::Playing,
    ];
}

/// Aspects of a part, as a set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Aspects(pub u16);

impl Aspects {
    pub const PITCHES: Aspects = Aspects(1);
    pub const VOICINGS: Aspects = Aspects(1 << 1);
    pub const RHYTHM: Aspects = Aspects(1 << 2);
    pub const CONTOUR: Aspects = Aspects(1 << 3);
    pub const MOTIFS: Aspects = Aspects(1 << 4);
    pub const DRUMS: Aspects = Aspects(1 << 5);
    pub const TIMING: Aspects = Aspects(1 << 6);
    pub const ACCENTS: Aspects = Aspects(1 << 7);
    pub const LOUDNESS: Aspects = Aspects(1 << 8);
    pub const LENGTHS: Aspects = Aspects(1 << 9);
    pub const RANGE: Aspects = Aspects(1 << 10);
    pub const DENSITY: Aspects = Aspects(1 << 11);
    pub const CHORDS: Aspects = Aspects(1 << 12);
    /// Locked bars and notes, and what Keep holds (rhythm, tune shape,
    /// chords, accents).
    pub const YOURS: Aspects = Aspects(1 << 13);

    pub const fn or(self, o: Aspects) -> Aspects {
        Aspects(self.0 | o.0)
    }
    pub const fn has(self, o: Aspects) -> bool {
        self.0 & o.0 == o.0
    }
    pub const fn meets(self, o: Aspects) -> bool {
        self.0 & o.0 != 0
    }
}

/// Re-fit or regenerate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum How {
    /// The same decisions, realised again.
    Refit,
    /// New decisions for what the change touches.
    Regenerate,
}

/// What a change does to the parts that depend on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Effect {
    pub how: How,
    /// What is worked out again.
    pub redo: Aspects,
    /// What stays as it was.
    pub keep: Aspects,
}

/// The table in the design's contract 2.
pub const fn effect(change: Change) -> Effect {
    use Aspects as A;
    match change {
        Change::Chord => Effect {
            how: How::Refit,
            redo: A::PITCHES.or(A::VOICINGS),
            keep: A::RHYTHM
                .or(A::CONTOUR)
                .or(A::MOTIFS)
                .or(A::DRUMS)
                .or(A::TIMING)
                .or(A::LOUDNESS)
                .or(A::YOURS),
        },
        Change::Groove => Effect {
            how: How::Refit,
            redo: A::TIMING.or(A::ACCENTS),
            keep: A::PITCHES.or(A::RHYTHM).or(A::CHORDS).or(A::YOURS),
        },
        Change::Motif => Effect {
            how: How::Regenerate,
            redo: A::MOTIFS.or(A::PITCHES).or(A::RHYTHM),
            keep: A::CHORDS.or(A::DRUMS).or(A::YOURS),
        },
        Change::Style => Effect {
            how: How::Regenerate,
            redo: A::CHORDS
                .or(A::RHYTHM)
                .or(A::PITCHES)
                .or(A::VOICINGS)
                .or(A::LENGTHS)
                .or(A::LOUDNESS)
                .or(A::ACCENTS)
                .or(A::DRUMS),
            keep: A::YOURS,
        },
        Change::Instrument => Effect {
            how: How::Refit,
            redo: A::RANGE.or(A::LENGTHS).or(A::LOUDNESS),
            keep: A::RHYTHM.or(A::CONTOUR).or(A::CHORDS).or(A::YOURS),
        },
        Change::Activity => Effect {
            how: How::Regenerate,
            redo: A::DENSITY.or(A::RHYTHM),
            keep: A::CHORDS.or(A::MOTIFS).or(A::YOURS),
        },
        Change::Playing => Effect {
            how: How::Refit,
            redo: A::LOUDNESS.or(A::LENGTHS),
            keep: A::PITCHES
                .or(A::RHYTHM)
                .or(A::CONTOUR)
                .or(A::CHORDS)
                .or(A::TIMING)
                .or(A::YOURS),
        },
    }
}

/// Where a part is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Target {
    /// In a preview, not kept yet.
    Preview,
    /// A clip the musician kept.
    Kept,
    /// Locked bars or notes.
    Locked,
}

/// What happens to one part after a change.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    /// Realise its decisions again, at once.
    Refit,
    /// Make new decisions for what the change touches.
    Regenerate,
    /// Mark it and offer "Fit to the new chords", as one step UNDO takes
    /// back. Never done without asking.
    OfferFit,
    /// Leave it as it is.
    Leave,
}

/// What a change does to one part. `fits` says whether a kept clip's
/// notes still fit after the change (see [`misfits`]).
pub fn action(change: Change, target: Target, fits: bool) -> Action {
    match (target, effect(change).how) {
        (Target::Locked, _) => Action::Leave,
        (Target::Preview, How::Refit) => Action::Refit,
        (Target::Preview, How::Regenerate) => Action::Regenerate,
        (Target::Kept, How::Refit) if change == Change::Chord && !fits => Action::OfferFit,
        (Target::Kept, _) => Action::Leave,
    }
}

/// A strong beat: on a beat of the bar.
fn strong(at: Tick) -> bool {
    at.rem_euclid(PPQ as Tick) == 0
}

/// The notes of a kept clip that fitted the old chords and don't fit the
/// new ones: chord tones on a beat under `old` that aren't under `new`.
/// `origin` places the clip's tick 0 in the harmonies' ticks. Empty means
/// nothing to offer.
pub fn misfits(
    notes: &[PlainNote],
    old: &dyn Harmony,
    new: &dyn Harmony,
    origin: Tick,
) -> Vec<usize> {
    notes
        .iter()
        .enumerate()
        .filter(|(_, n)| strong(n.at))
        .filter(|(_, n)| {
            let p = i32::from(n.pitch);
            let was = old
                .at(origin + n.at)
                .answer
                .chord()
                .is_some_and(|c| c.has(p));
            let is = new
                .at(origin + n.at)
                .answer
                .chord()
                .is_some_and(|c| c.has(p));
            was && !is
        })
        .map(|(i, _)| i)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_never_redoes_what_it_keeps() {
        for c in Change::ALL {
            let e = effect(c);
            assert!(!e.redo.meets(e.keep), "{c:?}");
            assert!(
                e.keep.has(Aspects::YOURS),
                "{c:?} must keep locks and Keeps"
            );
        }
        let chord = effect(Change::Chord);
        assert_eq!(chord.how, How::Refit);
        assert!(chord.keep.has(Aspects::RHYTHM.or(Aspects::CONTOUR)));
        assert!(chord.redo.has(Aspects::PITCHES));
        assert_eq!(effect(Change::Style).how, How::Regenerate);
    }

    #[test]
    fn kept_clips_are_offered_a_fit_never_rewritten() {
        assert_eq!(action(Change::Chord, Target::Preview, false), Action::Refit);
        assert_eq!(action(Change::Chord, Target::Kept, false), Action::OfferFit);
        assert_eq!(action(Change::Chord, Target::Kept, true), Action::Leave);
        assert_eq!(action(Change::Chord, Target::Locked, false), Action::Leave);
        assert_eq!(
            action(Change::Style, Target::Preview, true),
            Action::Regenerate
        );
        for c in Change::ALL {
            assert_ne!(action(c, Target::Kept, false), Action::Refit, "{c:?}");
            assert_ne!(action(c, Target::Kept, false), Action::Regenerate, "{c:?}");
            assert_eq!(action(c, Target::Locked, false), Action::Leave, "{c:?}");
        }
    }
}
