//! A track's MIDI effects: the Notes stage that runs before its sound.
//!
//! Host-first, as `mpfree-arp` is: sample-timed notes go in once a period
//! and come out changed, each at its frame. Transpose, Scale, Chord (with
//! voicing, inversion, a highest note and strum), Velocity, Note length,
//! Echo, Humanize, Chance, Strum, Latch and Randomize, up to [`MAX_FX`]
//! in a track's order. No
//! device I/O, and no allocation, locks or I/O once made, so a chain can
//! sit in the sound loop. Design: `docs/dev/design/note-fx.md`.
#![forbid(unsafe_code)]

pub mod chain;
pub mod fx;
pub mod notes;
pub mod settings;
pub mod track;

pub use chain::{Chain, Period, Slot, MAX_FX};
pub use fx::{
    Chord, Echo, Fx, LatchMode, Randomize, Shape, Strum, StrumOrder, Time, Velocity, Voicing,
    MAX_MAP, NOTE_VALUES,
};
pub use mpfree_scale::{Scale, Snap};
pub use notes::{Ev, Kind, Notes, MAX_EVENTS};
pub use settings::Setting;
pub use track::{Item, TrackFx};

#[cfg(test)]
mod tests;
