//! The arpeggiator's screens for the MPC One, built on `mpfree-ui`.
//!
//! - [`widgets`]: the widgets the arp adds: mode tabs ([`widgets::Segments`]),
//!   the quick band's rate buttons ([`widgets::RateBar`]), the Q-Link sliders
//!   ([`widgets::SliderStack`]), the step strip ([`widgets::StepStrip`]) and
//!   the pattern buttons ([`widgets::ButtonRow`]).
//! - [`view`]: the Arp panel, the quick band and the arp chip. The panel adds
//!   its widgets and layouts to the app's [`mpfree_ui::Ui`], shows an
//!   [`view::ArpState`] taken from the engine, and turns touches into
//!   [`view::ArpAction`]s for the app to apply. Neither is a top-level tab:
//!   the arp is a way of playing the pads, so it goes with them.
//! - [`sample`]: sample screens for the workstation preview and the tests.
//!
//! No device I/O: the pad player draws the screens and feeds them touches.

#![forbid(unsafe_code)]

pub mod sample;
pub mod view;
pub mod widgets;

pub use view::{ArpAction, ArpPanel, ArpState, PatternButton};
