//! The MPFree grain engine (sampler step S6).
//!
//! One engine serves both uses in the design (`docs/project/sampler-design.md`,
//! "Granular"):
//! - **Granular playback** of a region of a sample: [`Clip`].
//! - **Granular effect** on a live signal: [`LiveBuffer`], a circular buffer
//!   the audio thread writes into, with freeze. [`GrainFx`] packages it as
//!   an insert with a wet/dry mix, and [`GrainNode`] puts that in any FX
//!   chain as an `mpfree_node::Node`.
//!
//! A [`GrainVoice`] spawns grains from either, each with its own position,
//! length, pitch, direction, pan and window, and mixes them into the output.
//! Architecture: `docs/project/sampler-architecture.md` ("Granular engine").
//!
//! Real-time rules, checked by `tests/no_alloc.rs`:
//! - Grains live in a fixed array ([`MAX_GRAINS`] per voice). A [`Budget`]
//!   shared by the voices caps the grains alive at once. Past the cap,
//!   new grains are skipped, never queued, so the period is never at risk.
//! - `render`, `start`, `stop`, `set_params`, `LiveBuffer::write*` and every
//!   `GrainFx` call after `new` never allocate, free, lock or block.
//! - Randomness comes from a seeded generator per voice, so offline renders
//!   are bit-identical run to run.
#![forbid(unsafe_code)]

pub mod buffer;
pub mod fx;
pub mod node;
pub mod rng;
pub mod voice;
pub mod window;

pub use buffer::{Clip, GrainSource, LiveBuffer};
pub use fx::GrainFx;
pub use node::GrainNode;
pub use rng::Rng;
pub use voice::{quantise, Budget, GrainView, GrainVoice, Params, Stats, TempoSync, MAX_GRAINS};
pub use window::{Window, WindowTable};

/// Four-point Hermite interpolation between `x1` and `x2`, `t` in 0..1.
/// The same curve the sampler's voices use.
#[inline]
pub fn hermite(x0: f32, x1: f32, x2: f32, x3: f32, t: f32) -> f32 {
    let c1 = 0.5 * (x2 - x0);
    let c2 = x0 - 2.5 * x1 + 2.0 * x2 - 0.5 * x3;
    let c3 = 0.5 * (x3 - x0) + 1.5 * (x1 - x2);
    ((c3 * t + c2) * t + c1) * t + x1
}
