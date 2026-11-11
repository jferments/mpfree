//! The sampler's screens for the MPC One, built on `mpfree-ui`.
//!
//! - [`source`]: what a screen needs to know about a source, worked out once
//!   off the audio thread: its peak pyramid as [`mpfree_ui::wave::Peaks`], a
//!   mono mix for snapping to zero crossings, its tempo and its root note.
//! - [`widgets`]: the widgets the sampler adds: the pad bar with the editor's
//!   tabs ([`widgets::PadBar`]), the Q-Link cards ([`widgets::KnobCard`]) and
//!   the wheel card ([`widgets::InfoCard`]).
//! - [`wave_tab`]: the pad editor's Wave tab: the overview, the waveform with
//!   its Start, End and loop markers, the region's settings and the Q-Links.
//!   It turns touches, Q-Link turns and the wheel into [`WaveAction`]s for
//!   the app to apply to the project, each drag or turn one undo step.
//! - [`sample`]: sample screens for the workstation preview and the tests.
//!
//! No device I/O and no audio: the app draws the screens, feeds them
//! touches and controls, and applies what they ask for.

#![forbid(unsafe_code)]

pub mod sample;
pub mod source;
pub mod wave_tab;
pub mod widgets;

pub use source::{PyramidPeaks, SourceView};
pub use wave_tab::{Mark, Snap, WaveAction, WaveState, WaveTab};
