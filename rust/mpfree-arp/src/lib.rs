//! MPFree's Latch, Note Repeat, arpeggiator and latch-pattern engine.
//!
//! Host-first: notes go in (from pads, the screen or MIDI), sample-timed
//! note-ons and note-offs come out. No device I/O, and no allocation, locks
//! or I/O once running, so it can sit in the audio thread. Design:
//! `docs/dev/design/arpeggiator.md`.
#![forbid(unsafe_code)]

pub mod ears;
pub mod engine;
pub mod groups;
pub mod hand;
pub mod library;
pub mod midi;
pub mod modifier;
pub mod notes;
pub mod pattern;
pub mod phrase;
pub mod record;
/// Scales come from the shared `mpfree-scale` crate (the synth pads use it too).
pub use mpfree_scale as scale;

pub use ears::{CallNote, Ears, Heard, Light, Turns as AnswerTurns, MAX_CALL_NOTES};
pub use engine::{
    phrase_speed, step_notes, turn_of, turn_steps, Chord, Clock, Engine, Event, EventKind, Events,
    Mode, Order, Ramp, Rate, Settings, SyncStart, Turns, Velocity, PPQ,
};
pub use hand::{Hand, HandOn};
pub use library::{Library, LibraryError, STARTER_PHRASES};
pub use midi::{midi_file, Message, MidiOut, NoteMap};
pub use modifier::{ModSource, Modifier};
pub use notes::{Held, LatchMode, NoteSet, Notes, MAX_NOTES};
pub use pattern::{Lane, Lanes, ParseError, Pattern, Pick, Step, BUILT_IN, MAX_STEPS};
pub use phrase::{
    Feel, Follow, Join, Kind as PhraseKind, Phrase, PhraseNote, Playhead, Speed, Written,
    MAX_PHRASE_NOTES,
};
pub use record::{RecordMode, Recorder, Wrote};
pub use scale::{Scale, Snap, SCALES};
