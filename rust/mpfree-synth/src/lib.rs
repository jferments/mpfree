//! MPFree's native synth instruments, on the shared [`mpfree_node`]
//! interface: anything here can sit on a track, in a chain, or in a test.
//!
//! - [`Poly`]: a subtractive synth (two oscillators with unison, sub and
//!   noise, drive into a ladder or state-variable filter, two envelopes, an
//!   LFO, poly/mono/legato with glide) and its factory presets.
//!
//! The design and roadmap are in `docs/project/synths.md`. Every synth keeps
//! the node rules: no allocation, lock or panic in `process`, and the same
//! output for any block size (`tests/` checks both).
#![forbid(unsafe_code)]

pub mod dsp;
pub mod params;
pub mod poly;

pub use params::{Preset, PARAMS, PRESETS};
pub use poly::Poly;
