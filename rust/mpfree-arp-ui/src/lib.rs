//! The arpeggiator's screens for the MPC One, built on `mpfree-ui`.
//!
//! - [`widgets`]: the widgets the arp adds: mode tabs ([`widgets::Segments`]),
//!   the Q-Link sliders ([`widgets::SliderStack`]), the step strip
//!   ([`widgets::StepStrip`]) and the pattern buttons ([`widgets::ButtonRow`]).
//! - [`view`]: the Arp page. It adds its widgets and layouts to the app's
//!   [`mpfree_ui::Ui`], shows an [`view::ArpState`] taken from the engine, and
//!   turns touches into [`view::ArpAction`]s for the app to apply.
//! - [`sample`]: sample screens for the workstation preview and the tests.
//!
//! No device I/O: the pad player draws the page and feeds it touches.

#![forbid(unsafe_code)]

pub mod sample;
pub mod view;
pub mod widgets;

pub use view::{ArpAction, ArpPage, ArpState, PatternButton};
