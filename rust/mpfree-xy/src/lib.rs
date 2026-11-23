//! The XY controller's engine (the XY controller plan, phase 1): one model
//! for the XY pad, whatever plays it and whatever it plays.
//!
//! - [`XyInput`]: one shape for every controller's input, with its time.
//!   The screen sends it today; a learned MIDI control's XY across and up
//!   send it, stamped when they arrive; later a game controller, a tablet
//!   or OSC send the same.
//! - [`Gesture`]: who owns the pad. The first touch owns it until it ends,
//!   other fingers on the same controller are ignored by touch id, and a
//!   gesture from another controller takes it (the newest gesture wins, as
//!   for any setting held by hand). Only the owner's End or Cancel lets
//!   go.
//! - [`Layout`]: what the pad plays. A name, two [`Axis`] values (across
//!   and up) of up to [`TARGETS`] [`Target`]s each (a setting with a from,
//!   a to and a curve), where it rests, what happens when the finger lifts
//!   ([`Lift`]) and an optional gate (a setting that is 1 while a finger
//!   is down: an XY FX's Active). Checked and compiled on the control side
//!   ([`Layout::compile`]) into a fixed [`Compiled`] list for the sound
//!   loop, so nothing is looked up or allocated while playing.
//! - [`Pad`]: where the pad is: under the finger, gliding back to where it
//!   rests after a lift, or staying.
//!
//! Pure logic: no I/O, no threads, `#![forbid(unsafe_code)]`, and nothing
//! allocates after a [`Compiled`] or a [`Pad`] is made. Recording and
//! playing moves stay in `mpfree-moves`, timing in the time engine.

#![forbid(unsafe_code)]

mod gesture;
mod input;
mod layout;
mod pad;

pub use gesture::{Gesture, Step};
pub use input::{Device, Phase, XyInput};
pub use layout::{
    Axis, Compiled, How, Layout, LayoutError, Lift, Start, Target, GLIDE_MAX_MS, SLOTS, TARGETS,
};
pub use pad::Pad;

pub use mpfree_mod::MapCurve;
pub use mpfree_param::ParamAddr;
