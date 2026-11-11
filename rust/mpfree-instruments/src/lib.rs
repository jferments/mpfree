//! Instruments for MPFree: sound presets that any node can save and load.
//!
//! - [`preset`]: a node's settings as plain `key = value` text and back.
//!   A user preset is one such file in the library
//!   (`library/Instruments/<engine>/<name>.preset`); the library crate
//!   writes and lists the files, so this crate does no file I/O.
//!
//! Next here: the catalogue the instrument browser lists (factory presets
//! with tags, user files, sampler programs, plugins), with search,
//! favourites and audition.

#![forbid(unsafe_code)]

pub mod preset;

pub use preset::{Error, Preset};
