//! Offline audio analysis for the MPFree sampler.
//!
//! - [`PeakPyramid`]: the waveform summary that lets the screen draw any zoom
//!   of a sample with one lookup per pixel column.
//! - [`OnsetAnalysis`]: transient detection for transient chop, with a
//!   sensitivity that can change without re-analysing.
//! - [`estimate_tempo`] and [`loop_tempo`]: tempo for beat chop and warp.
//! - [`sound_spans`]: silence split and trim.
//! - [`root_pitch`]: the note a sample plays (YIN), for tuning and keygroups.
//! - [`nearest_zero_crossing`]: snapping markers so slices don't click.
//!
//! Design: `docs/project/sampler-architecture.md` (Analysis). Nothing here
//! runs on the audio thread; analysis allocates.
#![forbid(unsafe_code)]

pub mod fft;
pub mod onsets;
pub mod peaks;
pub mod pitch;
pub mod silence;
pub mod tempo;
pub mod zero;

pub use onsets::{Onset, OnsetAnalysis};
pub use peaks::{Column, PeakError, PeakPyramid, BASE_BLOCK, LEVELS};
pub use pitch::{note_name, root_pitch, yin, Pitch, PitchSettings};
pub use silence::{sound_spans, SilenceSettings};
pub use tempo::{estimate_tempo, loop_tempo, Tempo};
pub use zero::{nearest_zero_crossing, previous_zero_crossing};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnalysisError {
    BadInput(&'static str),
}

impl std::fmt::Display for AnalysisError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for AnalysisError {}

/// Mixes 1 or 2 interleaved channels to mono. Non-finite samples become 0.
pub fn mono(data: &[f32], channels: usize) -> Result<Vec<f32>, AnalysisError> {
    if channels == 0 || channels > 2 {
        return Err(AnalysisError::BadInput("channels"));
    }
    if !data.len().is_multiple_of(channels) {
        return Err(AnalysisError::BadInput("length"));
    }
    let clean = |x: f32| if x.is_finite() { x } else { 0.0 };
    Ok(data
        .chunks_exact(channels)
        .map(|f| f.iter().map(|x| clean(*x)).sum::<f32>() / channels as f32)
        .collect())
}
