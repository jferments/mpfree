//! Instruments for MPFree: sound presets that any node can save and load.
//!
//! - [`preset`]: a node's settings as plain `key = value` text and back.
//!   A user preset is one such file in the library
//!   (`library/Instruments/<engine>/<name>.preset`); the library crate
//!   writes and lists the files, so this crate does no file I/O.
//!
//! - [`sound`]: a native engine with its own effects and macros, as one
//!   node, and its design as preset lines.
//!
//! Next here: the catalogue the instrument browser lists (factory presets
//! with tags, user files, sampler programs, plugins), with search,
//! favourites and audition.

#![forbid(unsafe_code)]

pub mod preset;
pub mod sound;

pub use preset::{Error, Preset};
pub use sound::{Design, Designed, Macro, MacroMap, Sound, SoundError};
