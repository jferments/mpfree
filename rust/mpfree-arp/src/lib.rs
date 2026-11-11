//! MPFree's Latch, Note Repeat, arpeggiator and latch-pattern engine.
//!
//! Host-first: notes go in (from pads, the screen or MIDI), sample-timed
//! note-ons and note-offs come out. No device I/O, and no allocation, locks
//! or I/O once running, so it can sit in the audio thread. Design:
//! `docs/project/arpeggiator.md`.
#![forbid(unsafe_code)]

pub mod engine;
pub mod library;
pub mod midi;
pub mod notes;
pub mod pattern;
pub mod record;
pub mod scale;

pub use engine::{Clock, Engine, Event, EventKind, Events, Mode, Order, Ramp, Rate, Settings, SyncStart, Velocity, PPQ};
pub use library::{Library, LibraryError};
pub use midi::{midi_file, Message, MidiOut, NoteMap};
pub use notes::{Held, LatchMode, NoteSet, Notes, MAX_NOTES};
pub use pattern::{Lane, Lanes, ParseError, Pattern, Pick, Step, BUILT_IN, MAX_STEPS};
pub use record::{RecordMode, Recorder, Wrote};
pub use scale::{Scale, SCALES};
