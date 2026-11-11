//! The FX chain editor and the macros and morph panel for the MPC One,
//! built on `mpfree-ui` over `mpfree-fx`'s [`mpfree_fx::Chain`] and
//! `mpfree-mod`'s [`mpfree_mod::ModEngine`].
//!
//! **Where they open is the Hello MPC thread's call and Jay's** (layout
//! spec, "The FX chain editor and macros"). Hello proposes: the chain
//! editor takes the mixer's place when an insert is tapped there, with a
//! Back to the mixer ([`ChainState::back`]), and sits in the Sample tab's
//! FX sub-tab under the pad bar (with a switch between the pad's chain and
//! the track's, [`ChainState::scopes`], to agree with the sampler thread);
//! Macros 1-4 and 5-8 are banks of the Q-Link column, and a long press on
//! a macro card opens the macros panel over the page under the top bar,
//! with Back in its corner. While the chain editor is open, the Q-Links
//! default to the selected effect's first four knobs: pass
//! [`QLINK_PAGE`] and [`ChainState::qlinks`] to `Bindings::set_page`. This crate is the panel
//! inside whatever area it gets.
//!
//! - [`state`]: what the panels show ([`ChainState`], [`MacrosState`]),
//!   built on the control thread; `from_chain` and `from_engine` read a
//!   chain and an engine directly, for previews and tests.
//! - [`view`]: the chain editor ([`ChainView`]): the inserts, the selected
//!   effect's knobs with modulation rings and macro badges, and the
//!   modulation column (macro, learn to a Q-Link, matrix rows).
//! - [`macros`]: the macros and morph panel ([`MacrosView`]).
//! - [`sample`]: sample screens for the workstation preview
//!   (`mpfree-chain-preview`) and the tests.
//!
//! For the app: [`add`] puts both widgets and their pages in the [`Ui`];
//! [`update_chain`] and [`update_macros`] show new states; after a touch,
//! [`take_chain_actions`] and [`take_macro_actions`] give what to do.
//! Parameter changes go to the mod engine as `ModEvent::hand` gives (a
//! `Set`, or a `Touch` over a move lane or while recording; it sends
//! them on to the chain); chain edits go to the mixer as `Edit`s.

#![forbid(unsafe_code)]

pub mod macros;
pub mod sample;
pub mod state;
pub mod view;

pub use macros::{MacroAction, MacrosView};
pub use state::{macro_color, strip_id, strip_of, ChainState, MacrosState, ParamView, SlotView};
pub use view::{ChainAction, ChainView};

use mpfree_ui::Ui;

/// The widgets' names and pages.
pub const CHAIN: &str = "chain";
pub const CHAIN_PAGE: &str = "chain";
pub const MACROS_W: &str = "macros";
pub const MACROS_PAGE: &str = "macros";
/// The pages: the app's top bar, the panel beside the app's Q-Link column
/// (the Play page's width), the hint line. In the Sample tab the chain
/// editor goes under the pad bar instead, in the sampler's page.
pub const QLINKS_W: i32 = 168;
pub const CHAIN_LAYOUT: &str = "col(header:60, row(chain:*, qlinks:168):*, hint:26)";
pub const MACROS_LAYOUT: &str = "col(header:60, row(macros:*, qlinks:168):*, hint:26)";
/// The chain editor's page key for `Bindings::set_page`: a Q-Link learned
/// here stays on the chain editor.
pub const QLINK_PAGE: &str = "chain";
/// The hint lines: what the buttons do on the page.
pub const CHAIN_HINT: &str = "Q-Links: the first 4 knobs shown  ·  SHIFT + Q-Link: fine  ·  touch a knob, then a macro number, Learn or + LFO";
pub const MACROS_HINT: &str = "Q-Links: the 4 macros shown  ·  SHIFT + Q-Link: fine  ·  tap a target to set its range  ·  Back closes";

/// Adds both widgets and their pages. The app's `header`, `qlinks` and
/// `hint` widgets must be there already.
pub fn add(ui: &mut Ui) -> Result<(), String> {
    ui.add(CHAIN, ChainView::new());
    ui.add(MACROS_W, MacrosView::new());
    ui.add_page(CHAIN_PAGE, CHAIN_LAYOUT)?;
    ui.add_page(MACROS_PAGE, MACROS_LAYOUT)
}

pub fn update_chain(ui: &mut Ui, st: ChainState) {
    if let Some(v) = ui.get_mut::<ChainView>(CHAIN) {
        v.update(st);
    }
}

pub fn update_macros(ui: &mut Ui, st: MacrosState) {
    if let Some(v) = ui.get_mut::<MacrosView>(MACROS_W) {
        v.update(st);
    }
}

pub fn take_chain_actions(ui: &mut Ui) -> Vec<ChainAction> {
    ui.get_mut::<ChainView>(CHAIN)
        .map(|v| v.take_actions())
        .unwrap_or_default()
}

pub fn take_macro_actions(ui: &mut Ui) -> Vec<MacroAction> {
    ui.get_mut::<MacrosView>(MACROS_W)
        .map(|v| v.take_actions())
        .unwrap_or_default()
}
