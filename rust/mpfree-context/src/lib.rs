//! The Musical Context: one reading of what was played, shared by every
//! stage of the composer (`mpfree-compose`) and, later, by any other
//! feature that needs to know the key, the chords or the groove.
//!
//! - [`key`]: 72 keys (12 tonics, six modes) and a [`KeyReading`] said in
//!   words ("Likely D minor, also possible D Dorian, not sure yet"), with
//!   "no clear key" as an answer. Odds stay inside, for ranking.
//! - [`chord`]: chords and the [`ChordTimeline`]: spans with their own
//!   start and length, from a beat to the whole clip, and "no clear
//!   chord" as an answer.
//! - [`harmony`]: the one harmony question every part asks, answered from
//!   the held chord, the song section, the row's Harmony line or a
//!   suggestion, in that order ([`Resolver`]), with rows of different
//!   lengths looping as `scene_length` defines them.
//! - [`groove`]: played drums as residual timing against playback's swing
//!   ([`GrooveTemplate`]), so swing is never applied twice.
//! - [`context`]: the [`MusicalContext`] itself, with the clips and
//!   revisions it read, so a stale reading is never used, and the reading
//!   budget.
//! - [`listen`]: the listener: notes read as a key in words and a chord
//!   timeline, inside the reading budget (phase 0C).
//!
//! Design: `docs/dev/design/compose.md`. Plain data: nothing here runs on
//! the audio thread, and nothing else in mpfree depends on it yet.
#![forbid(unsafe_code)]

pub mod chord;
pub mod context;
pub mod groove;
pub mod harmony;
pub mod key;
pub mod listen;

pub use chord::{
    Chord, ChordAnswer, ChordSpan, ChordTimeline, Heard, Quality, TimelineError, Tone,
};
pub use context::{ClipRead, ClipRef, Motif, MusicalContext, PartRole, ReadBudget, Revision};
pub use groove::{unswing, Cell, GrooveTemplate, Hit, Lane, STEP};
pub use harmony::{
    row_tick, Harmony, HarmonyAt, HarmonySource, Playing, Resolver, RowHarmony, Section,
};
pub use key::{pc_name, Candidate, Key, KeyReading, Mode, Sureness};
pub use listen::{chord_of, Played, Reading};
