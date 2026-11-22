//! The composer behind Continue, Add a part, Improvise and Answer.
//!
//! A pure library: no screen, no sound, no device. A result depends only
//! on what it was given (a [`mpfree_context::MusicalContext`] and the
//! parts' decisions), the settings and a seed, so tests can pin it down.
//! Design: `docs/dev/design/compose.md`, which carries the implementation
//! contracts the second design review asked for.
//!
//! Phase 0A, the musical model:
//!
//! - [`event`]: phrases of events with the role each note plays over the
//!   chord and who made it ([`Source`]); a part's decisions kept apart
//!   from its notes.
//! - [`realize`]: decisions to notes under a harmony, deterministically,
//!   and notes back to decisions, so a chord change re-fits instead of
//!   rewriting (contract 2).
//! - [`refit`]: the table of what each change re-fits or regenerates, and
//!   what happens to parts in a preview, kept or locked (contract 2).
//! - [`dice`]: chance from stable names (contract 2).
//! - [`timing`]: the timing contract: musical position, groove, playing,
//!   then playback's quantise, speed and swing (contract 3).
//! - [`rules`]: structural rules, yours, and preferences, and refusals
//!   that suggest one change (contract 6).
//! - [`style`]: styles made of components, three presets (contract 11).
//! - [`profile`]: five instrument kinds (contract 10).
//! - [`keep`]: events as the clip notes Keep writes, all or nothing.
//!
//! Nothing here runs on the audio thread. The composer runs as an
//! idle-priority background job (phase 0C onwards).
#![forbid(unsafe_code)]

pub mod dice;
pub mod event;
pub mod keep;
pub mod profile;
pub mod realize;
pub mod refit;
pub mod rules;
pub mod style;
pub mod timing;

pub use dice::{Dice, DiceName, Job};
pub use event::{Event, MotifRef, Motion, NoteDecision, Part, PartId, Role, Source};
pub use keep::{clip_notes, KeepError};
pub use profile::{Articulation, InstrumentKind, Profile};
pub use realize::{decisions_of, realize, Fit, PlainNote};
pub use refit::{action, effect, misfits, Action, Aspects, Change, Effect, How, Target};
pub use rules::{RangeSetting, Rule, RuleKind, Suggestion, Unmet, Weights};
pub use style::{Component, PlanShape, Style, GROOVE, JAZZISH, MINIMAL, STYLES};
pub use timing::{check, heard, heard_tick, written, Playback, Timing, TimingNote};
