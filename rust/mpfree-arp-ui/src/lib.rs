//! The arpeggiator's screens for the MPC One, built on `mpfree-ui`.
//!
//! - [`widgets`]: the widgets the arp adds: mode tabs ([`widgets::Segments`]),
//!   the quick band's rate buttons ([`widgets::RateBar`]), the Q-Link sliders
//!   ([`widgets::SliderStack`]), the step strip ([`widgets::StepStrip`]),
//!   the pattern buttons ([`widgets::ButtonRow`]) and the workspace's grid
//!   ([`widgets::LaneGrid`]).
//! - [`view`]: the Arp panel, the quick band and the arp chip. The panel adds
//!   its widgets and layouts to the app's [`mpfree_ui::Ui`], shows an
//!   [`view::ArpState`] taken from the engine, and turns touches into
//!   [`view::ArpAction`]s for the app to apply. Neither is a top-level tab:
//!   the arp is a way of playing the pads, so it goes with them.
//! - [`workspace`]: the full-screen pattern workspace: a grid of steps and
//!   their parts, editing by tap, drag and wheel, Rotate, Mutate, Original,
//!   length, Save, and recording on the pads.
//! - [`phrases`]: the phrases a track picks from, as the screens read them:
//!   whose each is, and the list a shelf at a time, in the order that suits
//!   the key.
//! - [`phrase_tab`]: the Arp panel's Phrase tab: the kinds, the shown
//!   kind's phrases with a picture of each, and the phrase that plays with
//!   a cursor where it is, how it follows the held note and the Q-Link
//!   sliders.
//! - [`maker`]: New idea and More like this, one panel in the Phrase tab's
//!   list's place: the rows, the try's status and Try again, ‹ Try N, Back
//!   and Keep.
//! - [`sample`]: sample screens for the workstation preview and the tests.
//!
//! No device I/O: the pad player draws the screens and feeds them touches.

#![forbid(unsafe_code)]

pub mod answer_panel;
pub mod make_sheet;
pub mod maker;
pub mod phrase_tab;
pub mod phrases;
pub mod sample;
pub mod view;
pub mod widgets;
pub mod workspace;

pub use phrases::PhraseList;
pub use view::{ArpAction, ArpPanel, ArpState, PatternButton};
