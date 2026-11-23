//! The filters MPFree's pads and its Filter effect share, so a filter type
//! sounds the same wherever it is picked and every fix lands in both (the
//! filter gap plan, step F2; design: `docs/dev/design/fx-filter.md`).
//!
//! - [`svf`]: the trapezoidal state-variable stage (12 dB an octave), its
//!   designs ([`svf::SvfCoefs`]: bilinear, and the bell and shelves matched
//!   to their analog curves) and its bounded resonance ([`svf::bend`],
//!   [`svf::hard`]).
//! - [`multi`]: the multimode filter built on it (12 or 24 dB, Butterworth
//!   with the resonance on the last stage, the resonant stages bending),
//!   the Filter effect's Multi model and the pads' state-variable types,
//!   with a per-frame glide for a voice ([`multi::Multi`]).
//!
//! No allocation and no dependencies: every call is safe on the audio
//! thread.

pub mod multi;
pub mod svf;

pub use svf::{Svf, SvfCoefs, SvfMode};

/// Decibels to a linear gain.
#[inline]
pub(crate) fn db_to_gain(db: f32) -> f32 {
    10f32.powf(db * 0.05)
}

/// Sets a state too small to hear (under 1e-20) to zero, so a filter left
/// ringing on silence decays to exact zeros instead of running on
/// denormals.
#[inline]
pub(crate) fn flush(x: f32) -> f32 {
    if x.abs() < 1e-20 {
        0.0
    } else {
        x
    }
}
