//! MPFree's Latch, Note Repeat, arpeggiator and latch-pattern engine.
//!
//! Host-first: notes go in (from pads, the screen or MIDI), sample-timed
//! note-ons and note-offs come out. No device I/O, and no allocation, locks
//! or I/O once running, so it can sit in the audio thread. Design:
//! `docs/project/arpeggiator.md`.
#![forbid(unsafe_code)]

pub mod engine;
pub mod notes;
pub mod pattern;
pub mod scale;

pub use engine::{Engine, Event, EventKind, Events, Mode, Order, Rate, Settings, Velocity, PPQ};
pub use notes::{Held, LatchMode, NoteSet, Notes, MAX_NOTES};
pub use pattern::{Lane, Lanes, ParseError, Pattern, Pick, Step, BUILT_IN, MAX_STEPS};
pub use scale::{Scale, SCALES};
