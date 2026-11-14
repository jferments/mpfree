//! MPFree's native synth instruments, on the shared [`mpfree_node`]
//! interface: anything here can sit on a track, in a chain, or in a test.
//!
//! - [`Poly`]: a subtractive synth (two oscillators with unison, sub and
//!   noise, drive into a ladder or state-variable filter, two envelopes, an
//!   LFO, poly/mono/legato with glide) and its factory presets.
//! - [`Fm`]: a six-operator FM synth in the DX7's mould (32 algorithms,
//!   DX7-style envelopes, keyboard scaling, pitch envelope, LFO), its own
//!   presets, and [`dx7`] to load a player's DX7 banks (.syx).
//!
//! - [`Drums`]: a drum synth with a model per pad (kick, snare, clap, hats,
//!   toms, rim, cowbell, cymbal, zap, FM perc), choke groups and factory kits.
//!
//! - [`Pluck`]: plucked strings (Karplus-Strong) and struck bars and bells
//!   (a modal bank), with factory presets.
//!
//! - [`Wavetable`]: two wavetable oscillators scanning band-limited frame
//!   stacks with phase warps and unison, Poly's filter and envelopes, ten
//!   factory tables, and [`wavetable::import`] for WAVs, Serum-style tables
//!   and a pad's sample.
//!
//! - [`Keys`]: an electric piano, a struck tine or reed (a small modal
//!   bank) in front of a soft, asymmetric pickup that barks when hit hard,
//!   with tone, drive and tremolo, and factory presets.
//!
//! - [`pressure`]: what pad pressure drives in each engine. Both run it
//!   through the shared shaper in [`mpfree_node::pressure`].
//!
//! The design and roadmap are in `docs/dev/design/synths.md`. Every synth keeps
//! the node rules: no allocation, lock or panic in `process`, and the same
//! output for any block size (`tests/` checks both).
#![forbid(unsafe_code)]

pub mod drum;
pub mod drum_params;
pub mod dsp;
pub mod dx7;
pub mod expr;
pub mod fm;
pub mod fm_params;
pub mod keys;
pub mod keys_params;
pub mod params;
pub mod pluck;
pub mod pluck_params;
pub mod poly;
pub mod pressure;
pub mod wavetable;

pub use drum::Drums;
pub use fm::Fm;
pub use keys::Keys;
pub use params::{Preset, PARAMS, PRESETS};
pub use pluck::Pluck;
pub use poly::Poly;
pub use wavetable::Wavetable;
