//! The Musical Context: one reading of what was played, made once and
//! read by every stage of the composer.

use crate::chord::ChordTimeline;
use crate::groove::GrooveTemplate;
use crate::key::KeyReading;
use mpfree_transport::time::Tick;

/// A clip, by its lasting id (`mpfree_seq::ClipId`'s number).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ClipRef(pub u32);

/// A version of a clip's contents. Whoever reads a clip names its
/// revision (phase 0B decides how: a hash of what the clip plays, like
/// the play view's stamp, or a counter kept with the clip); the context
/// only compares them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Revision(pub u64);

/// What a part does in the music.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PartRole {
    Melody,
    Bass,
    Chords,
    Drums,
}

/// One clip the reading used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClipRead {
    pub clip: ClipRef,
    pub revision: Revision,
    pub role: PartRole,
}

/// How much a reading may read (contract 12): the note search has a
/// fixed budget whatever was played, but reading grows with the input,
/// so it is capped too.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReadBudget {
    pub max_bars: u16,
    pub max_notes: u32,
}

impl Default for ReadBudget {
    fn default() -> Self {
        ReadBudget {
            max_bars: 16,
            max_notes: 2048,
        }
    }
}

/// A motif the reading found: a stretch of the material read, by its
/// place. Phase 0C fills in its shape (steps in the scale, rhythm,
/// accents, contour).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Motif {
    pub id: u16,
    pub clip: ClipRef,
    pub start: Tick,
    pub len: Tick,
}

/// One reading of what was played.
#[derive(Clone, Debug, PartialEq)]
pub struct MusicalContext {
    pub key: KeyReading,
    /// The chords heard, over the length read.
    pub chords: ChordTimeline,
    /// The drums' groove, when there were drums to read.
    pub groove: Option<GrooveTemplate>,
    /// Every pitch class played (bit 0 = C): what new notes keep to when
    /// there is no clear key or chord.
    pub played_pcs: u16,
    pub motifs: Vec<Motif>,
    /// The clips read, each at its revision.
    pub read: Vec<ClipRead>,
    /// The budget cut the reading short: it read less than all of it,
    /// and says so.
    pub cut_short: bool,
}

impl MusicalContext {
    /// A reading of nothing.
    pub fn empty() -> MusicalContext {
        MusicalContext {
            key: KeyReading::nothing(),
            chords: ChordTimeline::default(),
            groove: None,
            played_pcs: 0,
            motifs: Vec::new(),
            read: Vec::new(),
            cut_short: false,
        }
    }

    /// Whether any clip it read has changed since. `now` gives a clip's
    /// current revision, or `None` if the clip is gone. A stale reading is
    /// never used: the request that made it is dropped.
    pub fn is_stale(&self, now: impl Fn(ClipRef) -> Option<Revision>) -> bool {
        self.read.iter().any(|r| now(r.clip) != Some(r.revision))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reading_goes_stale_when_a_clip_it_read_changes() {
        let mut ctx = MusicalContext::empty();
        ctx.read.push(ClipRead {
            clip: ClipRef(4),
            revision: Revision(7),
            role: PartRole::Melody,
        });
        assert!(!ctx.is_stale(|c| (c == ClipRef(4)).then_some(Revision(7))));
        assert!(ctx.is_stale(|_| Some(Revision(8))));
        assert!(ctx.is_stale(|_| None), "a deleted clip");
        assert!(!MusicalContext::empty().is_stale(|_| None));
    }
}
