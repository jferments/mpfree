//! The sampler's screens for the MPC One, built on `mpfree-ui`.
//!
//! - [`source`]: what a screen needs to know about a source, worked out once
//!   off the audio thread: its peak pyramid as [`mpfree_ui::wave::Peaks`], a
//!   mono mix for snapping to zero crossings, its tempo and its root note.
//! - [`widgets`]: the widgets the sampler adds: the pad bar with the editor's
//!   tabs ([`widgets::PadBar`]), the Q-Link cards ([`widgets::KnobCard`]) and
//!   the wheel card ([`widgets::InfoCard`]) and the pad map
//!   ([`widgets::PadMap`]).
//! - [`wave_tab`]: the pad editor's Wave tab: the overview, the waveform with
//!   its Start, End and loop markers, the region's settings and the Q-Links.
//!   It turns touches, Q-Link turns and the wheel into [`WaveAction`]s for
//!   the app to apply to the project, each drag or turn one undo step.
//! - [`warp_tab`]: the pad editor's Warp tab: the layer's region drawn in
//!   its beats, with hits pinned to beats by a tap, a drag or Pin hits (a
//!   loop played loosely made tight), each change a [`WaveAction`]. Its
//!   picture is in [`warp_widgets`].
//! - [`pads_tab`]: the pad editor's Pads tab, first in its bar: the kit's
//!   pads on a compact map to pick one, copy, paste and clear it, its main
//!   settings, and the kit's sounds to put on it ([`PadsAction`]).
//! - [`chop_tab`]: the pad editor's Chop tab: slices of the pad's sound by
//!   transients, beats, a grid, silence or by hand, split, merged and put on
//!   pads as one undo step ([`ChopAction`]).
//! - [`play_tab`]: the pad editor's Play tab: the layers' velocities, the
//!   amp envelope drawn over the sound, the filter's curve and the pad's
//!   other settings, each change a [`PlayAction`]. Its pictures are in
//!   [`play_widgets`].
//! - [`grain_tab`]: the pad editor's Grain tab: the layer's region with the
//!   grain cloud the engine plays over it, a touch pad for position and
//!   size, and every grain setting, each change a [`GrainAction`]. Its
//!   pictures are in [`grain_widgets`].
//! - [`rec_tab`]: the pad editor's Rec tab: the line input's meters and
//!   its last half minute, takes that start now, over a threshold or on the
//!   next bar and end on a press, silence or a length, and Skip back over
//!   the last seconds or bars, each onto a pad ([`RecAction`]). Its picture
//!   is in [`rec_widgets`].
//! - [`auto_tab`]: the Rec tab's Auto-sample mode: the synth on the pads
//!   sampled note by note into a keygroup instrument, with the keyboard,
//!   the velocity layers and the plan's cost shown first ([`AutoAction`]).
//!   Its picture is in [`auto_widgets`].
//! - [`sample`]: sample screens for the workstation preview and the tests.
//!
//! No device I/O and no audio: the app draws the screens, feeds them
//! touches and controls, and applies what they ask for.

#![forbid(unsafe_code)]

pub mod auto_tab;
pub mod auto_widgets;
pub mod chop_tab;
pub mod grain_tab;
pub mod grain_widgets;
pub mod keys_widgets;
pub mod pads_tab;
pub mod play_tab;
pub mod play_widgets;
pub mod rec_tab;
pub mod rec_widgets;
pub mod sample;
pub mod source;
pub mod warp_tab;
pub mod warp_widgets;
pub mod wave_tab;
pub mod widgets;

pub use auto_tab::{AutoAction, AutoState, AutoTab};
pub use chop_tab::{ChopAction, ChopState, ChopTab};
pub use grain_tab::{GrainAction, GrainState, GrainTab};
pub use mpfree_analysis::note_name;
pub use pads_tab::{PadsAction, PadsState, PadsTab};
pub use play_tab::{PlayAction, PlayState, PlayTab};
pub use rec_tab::{RecAction, RecSettings, RecState, RecTab};
pub use source::{PyramidPeaks, SourceView};
pub use warp_tab::WarpTab;
pub use wave_tab::{Mark, Snap, WaveAction, WaveState, WaveTab};
