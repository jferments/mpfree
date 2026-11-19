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
//! - [`Drawbar`]: a tonewheel organ, 91 free-running wheels tuned from the
//!   organ's gears, nine drawbars with foldback, uneven key contacts (the
//!   click), leakage, single-trigger percussion, the scanner vibrato and
//!   chorus, drive, and a rotary speaker with horn and drum; and a 1960s
//!   transistor combo organ on the Strings divide-down bank, its drawbars
//!   sent to a near-sine flute and a bright reed; with factory presets.
//! - [`Squelch`]: a mono acid bass, a sawtooth or its shaped square into a
//!   four-pole diode ladder solved with zero-delay feedback, accent (louder,
//!   and a filter sweep that builds up) and slide, with factory presets.
//! - [`Analog`]: a vintage analog poly, sixteen voice cards that differ and
//!   drift (Age), two oscillators with sync and poly-mod, a four-pole OTA
//!   or two-pole state-variable filter, the CEM3310's envelope curve and a
//!   bucket-brigade chorus, with factory presets.
//!
//! - [`Strings`]: a string machine, a divide-down bank with every key
//!   sounding, six registers (violin to trumpet) through their own
//!   filters, a swell per key or shared, bow noise and the Ensemble, with
//!   factory presets.
//!
//! - [`Bass`]: a one-voice bass, synth, 808 or reese, its oscillator, sub
//!   and fifth locked to one counter, a pitch drop that lands exactly on
//!   the note, a saturating four-pole ladder, drive with a low-pass after
//!   it, note priority and glide by time or rate, with factory presets.
//!
//! - [`Wind`]: a wind instrument, a waveguide bore (a clarinet's
//!   cylinder, a sax's cone or a flute's open pipe) excited by a reed or
//!   an air jet and driven by breath (velocity, pad pressure or a breath
//!   controller), with vibrato, growl, tonguing and factory presets.
//!
//! - [`pressure`]: what pad pressure drives in each engine. Both run it
//!   through the shared shaper in [`mpfree_node::pressure`].
//! - [`voice`]: the shared voice core the new engines are built on (held
//!   notes and priority, voice stealing, glide by time or rate, envelopes
//!   whose times mean what they say, velocity, play routes, a DC blocker).
//! - [`osc`]: its oscillators, the classic waves read from band-limited
//!   tables (nothing folds back at any pitch), the pulse as two saws and
//!   the supersaw.
//!
//! The design and roadmap are in `docs/dev/design/synths.md`. Every synth keeps
//! the node rules: no allocation, lock or panic in `process`, and the same
//! output for any block size (`tests/` checks both).
#![forbid(unsafe_code)]

pub mod analog;
pub mod analog_params;
pub mod bank;
pub mod bass;
pub mod bass_params;
pub mod character;
pub mod drawbar;
pub mod drawbar_params;
pub mod drum;
pub mod drum_params;
pub mod dsp;
pub mod dx7;
pub mod expr;
pub mod factory;
pub mod fm;
pub mod fm_params;
pub mod keys;
pub mod keys_params;
pub mod osc;
pub mod params;
pub mod pluck;
pub mod pluck_params;
pub mod poly;
pub mod pressure;
pub mod squelch;
pub mod squelch_params;
pub mod strings;
pub mod strings_params;
pub mod voice;
pub mod wavetable;
pub mod wind;
pub mod wind_params;

pub use analog::Analog;
pub use bass::Bass;
pub use drawbar::Drawbar;
pub use drum::Drums;
pub use fm::Fm;
pub use keys::Keys;
pub use params::{Preset, PARAMS};
pub use pluck::Pluck;
pub use poly::Poly;
pub use squelch::Squelch;
pub use strings::Strings;
pub use wavetable::Wavetable;
pub use wind::Wind;
