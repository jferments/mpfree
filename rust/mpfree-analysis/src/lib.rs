//! Offline audio analysis for the MPFree sampler.
//!
//! So far: [`PeakPyramid`], the waveform summary that lets the screen draw
//! any zoom of a sample with one lookup per pixel column. Onsets, tempo and
//! pitch follow (step S3 in `docs/project/sampler-design.md`).
//!
//! Nothing here runs on the audio thread; building a pyramid allocates.
#![forbid(unsafe_code)]

pub mod peaks;

pub use peaks::{Column, PeakError, PeakPyramid, BASE_BLOCK, LEVELS};
