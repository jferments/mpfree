//! Effect pages for the MPC One, built on `mpfree-ui` with the Synth tab's
//! widgets (the title bar with page tabs, the drawn cards, the tile grid
//! and the Q-Link strip), so an effect's page looks and works like an
//! instrument's.
//!
//! - [`view`]: an effect's page ([`FxPanel`]): its drawings, its
//!   parameters as tiles on named pages, its presets and its Q-Links,
//!   for any node (a hosted plugin gets the Synth tab's pages from its
//!   parameter groups, and no drawing).
//! - [`visual`]: the drawings, from the effects' own curve functions: the
//!   EQ's response with a box per band, the filters' and the DJ filter's
//!   response, the comb's teeth and ring, the drive's curve, the crush's
//!   staircase, the delay's repeats, the chorus, flanger and phaser
//!   sweeps with the comb and notches they make, Lo-fi's wear and tone,
//!   the compressor's curve and timing, the reverb's decay and tone, and
//!   Master safety's curve and catches.
//! - [`pages`]: which parameters go on which page and what the Q-Links
//!   turn there.
//! - [`perform`]: the Perform FX page ([`PerformPanel`]): sixteen pads with
//!   their lights, the XY pad with its trail and Hold, the selected pad's
//!   settings, and the DJ knob on Q-Link 1.
//! - [`state`]: what a page shows ([`FxState`]), the effects by id
//!   ([`make`]) and their presets.
//! - [`sample`]: sample screens for the workstation preview
//!   (`mpfree-fx-preview`) and the tests.
//!
//! **Where the pages open is the Hello thread's call and Jay's.** The FX
//! thread proposes: an effect's page opens from the chain editor (a tap
//! on the selected insert's name) and from the mixer's return and master
//! strips, over the page and under the top bar, with Back; the Perform
//! FX page's pads and XY pad go where Play's views make room for them
//! (Jay, 10:05Z: "the space goes to XY or effects"). This crate is the
//! panels inside whatever area they get.
//!
//! No device I/O: the app draws the screens and feeds them touches.

#![forbid(unsafe_code)]

pub mod pages;
pub mod perform;
pub mod sample;
pub mod state;
pub mod view;
pub mod visual;

pub use perform::{PerformAction, PerformPanel};
pub use state::{make, preset_values, presets, FxState, Scratch, CATALOGUE};
pub use view::{FxAction, FxPanel};
