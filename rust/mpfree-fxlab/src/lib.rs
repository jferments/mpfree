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
//! - For the FX quality plan (`docs/dev/design/fx-quality.md`): [`echo`]
//!   (a delay's repeats, their timing and tone), [`prototype`] (analog EQ
//!   curves to compare a digital one with), [`mixlaw`] (loudness across
//!   the Mix knob), [`knob`] (Decay and Pre-delay against what they
//!   measure), [`clarity`] (C50, C80, EDT against T30, which side a
//!   reverb starts on), [`flutter`] (wow and flutter of a 3.15 kHz tone),
//!   [`wobble`] (how much modulation smears a held note), and [`quality`]
//!   (every target of a class as one report, and the ratchet that keeps
//!   the numbers from getting worse).
//! - [`sampler`]: the sampler's own quality numbers (pitching, loop seams,
//!   voice stops, the filter against its analog prototype) with the same
//!   ratchet, so the voice is held to the bar the effects are.
//! - [`synth`]: every synth engine and factory preset against the synth
//!   library's targets (tuning, aliasing, velocity, level, stereo, the
//!   filter, glide, envelopes, FM sidebands, drum decays), with the same
//!   ratchet (`docs/dev/design/synth-quality.md`).
//! - [`robust`]: the DSP quality plan's Y4 (every engine, the drums and
//!   every effect stay finite at every setting) and Y7 (the same output
//!   for any block size), `docs/dev/design/dsp-quality.md`.
//! - [`reference`]: the same plan's Y8, each aliasing case against a
//!   render at 8 times the rate brought down to 44.1 kHz.
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
pub mod clarity;
pub mod dut;
pub mod dynamics;
pub mod echo;
pub mod fft;
pub mod flutter;
pub mod grain;
pub mod harmonics;
pub mod knob;
pub mod loudness;
pub mod mixlaw;
pub mod null;
pub mod plan;
pub mod programme;
pub mod prototype;
pub mod quality;
pub mod reference;
pub mod response;
pub mod robust;
pub mod room;
pub mod sampler;
pub mod signals;
pub mod synth;
pub mod wav;
pub mod wobble;
pub mod zipper;
