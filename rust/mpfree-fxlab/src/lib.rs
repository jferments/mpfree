//! fxlab: measuring effects, MPFree's and reference ones, by the same
//! numbers.
//!
//! - [`signals`]: sines on exact FFT bins, exponential sweeps, noise,
//!   impulses, level steps.
//! - [`dut`]: the device under test; [`dut::NodeDut`] runs any MPFree node.
//! - Analyses: [`harmonics`] (distortion and aliasing), [`response`]
//!   (frequency response), [`loudness`] (BS.1770 LUFS, true peak),
//!   [`null`] (aligned residuals), [`room`] (decay per octave, echo
//!   density, metallic tails, stereo correlation), [`dynamics`] (static
//!   curve, attack and release), [`zipper`] (clicks and zipper noise on
//!   parameter sweeps).
//! - [`programme`]: every stimulus in one recording with a sync chirp and
//!   a schedule, for capturing a reference device and cutting it back up.
//! - [`card`]: report cards over the registry, written as JSON.
//! - [`capture`]: the same cards read from a recording of the programme.
//! - [`plan`]: sweep plans over an effect's controls and contexts.
//!
//! Design and how to run it: `docs/dev/design/fxlab.md`. Desktop only;
//! nothing here runs on the MPC or on an audio thread.

pub mod capture;
pub mod card;
pub mod dut;
pub mod dynamics;
pub mod fft;
pub mod harmonics;
pub mod loudness;
pub mod null;
pub mod plan;
pub mod programme;
pub mod response;
pub mod room;
pub mod signals;
pub mod wav;
pub mod zipper;
