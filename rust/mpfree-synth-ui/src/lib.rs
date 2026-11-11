//! The Synth tab for the MPC One, built on `mpfree-ui`.
//!
//! - [`view`]: the panel. It adds its widgets and page to the app's
//!   [`mpfree_ui::Ui`], shows a [`view::SynthState`] taken from any
//!   [`mpfree_node::Node`], and turns touches, Q-Links and the wheel into
//!   [`view::SynthAction`]s for the app to apply to the node.
//! - [`pages`]: which parameters go on which page. Native engines name
//!   their pages (Poly: Osc, Filter, Env, LFO, Voice); any other node, such
//!   as a hosted plugin or an effect, gets pages from its parameter groups.
//! - [`widgets`]: the title bar with page tabs, the visual strip, the tile
//!   grid of knobs, choices and switches, and the Q-Link strip.
//! - [`visual`]: Poly's drawn visuals (oscillators, filter response,
//!   envelopes, LFO), worked out from its values.
//! - [`fm_visual`]: FM's: the algorithm diagram, operator envelopes,
//!   keyboard scaling, the pitch envelope and the LFO.
//! - [`format`]: value text (260 Hz, 2.5 kHz, +3.2 oct).
//! - [`draw`]: anti-aliased lines and arcs through `Canvas::blend_mask`.
//! - [`sample`]: sample screens for the workstation preview and the tests.
//!
//! No device I/O: the app draws the screens and feeds them touches.

#![forbid(unsafe_code)]

pub mod draw;
pub mod fm_visual;
pub mod format;
pub mod pages;
pub mod sample;
pub mod view;
pub mod visual;
pub mod widgets;

pub use view::{SynthAction, SynthPanel, SynthState};
