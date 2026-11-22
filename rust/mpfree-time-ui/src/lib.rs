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
//! - [`list`]: the song list, Arrange's third view: scenes and how many
//!   times each plays ([`list::SongListView`]).
//! - [`show`]: the Show view, Arrange's fourth view while a set is open
//!   (live shows B1): the set column, the song playing and its parts, the
//!   next-song card with Go, the song's notes, keyboards and click
//!   ([`show::ShowView`]).
//! - [`editor`]: the note editor of the Arrange page (a piano roll or a
//!   list, with Timing Correct): [`editor::Editor`], [`editor::NoteGrid`].
//! - [`lane`]: a lane of bars a finger draws, one value each (the
//!   Sequencer's Velocity and Chance lanes, the piano roll's lanes):
//!   [`lane::Lane`].
//! - [`sample`]: sample screens with a real transport, player, recorder and
//!   take undo behind them, for the workstation preview and the tests.
//!
//! No device I/O: the pad player draws the screens and feeds them touches.
//! The design is the time engine plan (UI section) and its mockups in the
//! mpfree-claude notes (`sequencer/mockups`).

#![forbid(unsafe_code)]

pub mod editor;
pub mod lane;
pub mod list;
pub mod sample;
pub mod show;
pub mod song;
pub mod view;
pub mod widgets;

pub use view::{
    action, BarsEdit, MarkerEdit, TempoEdit, TimeAction, TimePage, TimeState, TransportInfo,
};
