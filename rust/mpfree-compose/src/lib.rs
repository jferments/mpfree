//! The composer behind Continue, Add a part, Improvise and Answer, and
//! the arp's new phrases.
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
//! - [`keep`]: events as the clip notes Keep writes, all or nothing, and
//!   Continue's edit (a longer clip and its new notes, one take).
//!
//! Phase 0B, the check through real playback:
//!
//! - [`bind`]: a clip's [`mpfree_context::Revision`] (a hash of what it
//!   plays), the [`Binding`] of a preview to one clip at one revision, and
//!   the project as it plays while a result is heard.
//! - [`preview`]: the preview's states, from asking to Keep or Cancel,
//!   with stale results dropped (contract 7).
//!
//! `tests/playback.rs` plays results through the real play view and
//! player and keeps them with the real take undo.
//!
//! Phase 0C, the engine:
//!
//! - [`read`]: clips read into a Musical Context as they play, each at its
//!   revision (the listener itself is `mpfree_context::listen`).
//! - [`settings`]: the controls a request carries.
//! - [`plan`]: what each bar is for: its job, tension, activity and
//!   similarity aim.
//! - [`progression`]: new chords from the style's chord moves, yours kept
//!   as heard, sampled so Try again gives another good progression.
//! - [`motif`]: a tune as places in the scale on the grid, its phrase
//!   breaks, how alike two stretches are, and the classic transforms.
//! - [`rhythm`]: a bar's rhythm kept, varied, given the style's feel, or
//!   made new, for any meter.
//! - [`phrase`]: each new bar's notes by a beam search scored on your
//!   steps, the chords, the style and its neighbours; Try again on one
//!   bar leads into the bar after it (contract 1).
//! - [`playing`]: how new notes are played: loudness, length, and timing
//!   from the drummer's groove or the style's shared drift (contract 3).
//! - [`request`]: Continue end to end, from a clip in a project to the
//!   proposal a preview plays and Keep writes, and Try again on one bar.
//! - [`baseline`]: the simpler ways each job is played beside in the
//!   listening tests.
//!
//! Revision 3, S1, music first: the engine behind Continue since
//! 2026-11-23 (the phase 0C engine above stays as
//! [`request::first_engine`], the baseline beside it):
//!
//! - [`sketch`]: the phrase plan before any note: a gesture per bar,
//!   landing notes that climb to one late high point, rests, and one
//!   placed surprise.
//! - [`draft`]: chords led to the landings, notes filled in between them,
//!   a few whole drafts, the best one offered; locked bars kept, and
//!   Try again on picked bars from the plan.
//! - [`critic`]: a whole result judged on separate checks (goes
//!   somewhere, lands, breathes, yours, repeats, flows), with words.
//!
//! Revision 3, S2, parts that play together (Add a part):
//!
//! - [`row`]: a row read as one picture: each part on the step grid, the
//!   drums by kick, snare and hats, the tune read back into a plan
//!   (landings, high point, close, rests) and each bar's energy.
//! - [`cells`]: rhythm from one-beat cells with named changes (keep,
//!   shift, push, split, join, drop, ghost, echo, mirror): a riff that
//!   comes back changed where the plan asks, and ghost notes.
//! - [`parts`]: a bass, chords, a second line or drums for the row, by
//!   role (Support, Answer, Interlock, Stay out), judged with the row
//!   (together, no clash, room, lands, returns, flows); the best of four
//!   drafts, and the same rhythm re-fitted when a chord changes.
//!
//! Revision 3, S3, playing and sound that move:
//!
//! - [`perform`]: the same notes played, not typed: loudness leaning to
//!   each phrase's high point and easing off, timing that breathes with
//!   the phrase, slurs, held-back bars and rolled, voiced chords, by an
//!   instrument profile ([`perform::Expression`]).
//! - [`sound`]: the sound following the phrase without changing the
//!   patch: brightness opening to the high point, the last notes ringing
//!   on and thrown into the echo, on each preset's safe controls
//!   ([`sound::safe_controls`]), at Off, Subtle or Bold ([`sound_of`]).
//!
//! Phrases, P2, new ideas for the arp's phrase player:
//!
//! - [`idea`]: a bass line, a riff, chord stabs or an arp line of one,
//!   two or four bars made to order as an [`mpfree_arp::Phrase`]: Add a
//!   part for a band that isn't there, every draft written as a phrase
//!   writes notes and then judged, a riff with a tune of its own, and a
//!   Try again that never offers a phrase twice.
//!
//! Nothing here runs on the audio thread. The composer runs as an
//! idle-priority background job (phase 0C onwards).
#![forbid(unsafe_code)]

pub mod answer;
pub mod baseline;
pub mod beat;
pub mod bind;
pub mod bridge;
pub mod cells;
pub mod clip_style;
pub mod critic;
pub mod dice;
pub mod draft;
pub mod event;
pub mod fit;
pub mod idea;
pub mod job;
pub mod keep;
pub mod make_phrase;
pub mod motif;
pub mod parts;
pub mod perform;
pub mod phrase;
pub mod phrase_part;
pub mod plan;
pub mod playing;
pub mod preview;
pub mod profile;
pub mod progression;
pub mod read;
pub mod realize;
pub mod refit;
pub mod request;
pub mod rhythm;
pub mod row;
pub mod rules;
pub mod settings;
pub mod sketch;
pub mod sound;
pub mod style;
pub mod timing;

pub use answer::{answer, Answered, Call, Reply};
pub use beat::{continue_drums, Drummed, DrumsAsk, DrumsRefused, Hit as BeatHit};
pub use bind::{audition, revision, revision_in, Binding, Stale};
pub use clip_style::{read_clip, ClipStyle, Read as ClipRead, Subdivision};
pub use dice::{Dice, DiceName, Job};
pub use event::{Event, MotifRef, Motion, NoteDecision, Part, PartId, Role, Source};
pub use fit::{fit_clip, marks, FitRefused};
pub use idea::{
    develop, evolve, new_idea, next_idea, next_rework, rework, reworkable, Develop, Idea,
    IdeaVerdict, NewIdea, Rework,
};
pub use job::{
    lane_named, AddAsk, AddDone, AddPartJob, Ask, ContinueJob, Done, Lanes, PartMade, RowChords,
};
pub use keep::{clip_notes, continuation, KeepError};
pub use make_phrase::{phrase_of, phrase_of_as, Asked, Made, MakeRefused};
pub use motif::{similarity, transform, GridNote, Similarity, Transform};
pub use parts::{
    add_part, add_part_again, AddPart, Added, PartVerdict, Refused as PartRefused, Relation,
};
pub use phrase::{
    continue_line, try_again, try_bars as try_line_bars, try_notes as try_line_notes, Line, NoteAs,
    Phrasing,
};
pub use plan::{plan, BarJob, BarPlan};
pub use preview::{Arrival, Preview, Proposal, Refusal, State};
pub use profile::{Articulation, InstrumentKind, Profile};
pub use progression::{diatonic, progression, progression_keeping};
pub use read::{read, LeftOut, ToRead};
pub use realize::{decisions_of, realize, Fit, PlainNote};
pub use refit::{action, effect, misfits, Action, Aspects, Change, Effect, How, Target};
pub use request::{
    continue_clip, continue_clip_how, continue_clip_read, continue_notes, continue_notes_keeping,
    first_engine, first_engine_keeping, sound_of, try_aimed, try_bar, try_bars, try_notes,
    Continue, Continued, Making, OnClip, Refused,
};
pub use rhythm::{Grid, Onset};
pub use row::{DrumHit, Heard, Row, RowPart, RowPlan, ToHear};
pub use rules::{RangeSetting, Rule, RuleKind, Suggestion, Unmet, Weights};
pub use settings::{Aim, Character, Ending, Keeps, Settings};
pub use sound::{safe_controls, Control, Knob, Safe, SafeControls, SoundLevel, SoundMove, Swing};
pub use style::{Component, PlanShape, Style, GROOVE, JAZZISH, MINIMAL, STYLES};
pub use timing::{check, heard, heard_tick, written, Playback, Timing, TimingNote};
