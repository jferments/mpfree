//! The time engine's screens for the MPC One, built on `mpfree-ui`.
//!
//! - [`widgets`]: the widgets the time engine adds: the transport bar
//!   ([`widgets::TransportBar`]), the Clips launch grid
//!   ([`widgets::ClipGrid`]) with a progress ring per playing clip, and the
//!   takes list with Undo and Redo ([`widgets::TakeList`]).
//! - [`view`]: the bar and the Clips page. [`view::TimePage`] adds the
//!   widgets and the page to the app's [`mpfree_ui::Ui`] and shows a
//!   [`view::TimeState`] taken from the engine; [`view::action`] turns
//!   touches into [`view::TimeAction`]s for the app to apply. The bar goes
//!   under the header of any page with [`view::with_transport`].
//! - [`song`]: the Song view of the Arrange page: the arrangement's
//!   placements on a timeline ([`song::SongLanes`]).
//! - [`sample`]: sample screens with a real transport, player, recorder and
//!   take undo behind them, for the workstation preview and the tests.
//!
//! No device I/O: the pad player draws the screens and feeds them touches.
//! The design is the time engine plan (UI section) and its mockups in the
//! mpfree-claude notes (`sequencer/mockups`).

#![forbid(unsafe_code)]

pub mod sample;
pub mod song;
pub mod view;
pub mod widgets;

pub use view::{action, TimeAction, TimePage, TimeState, TransportInfo};
