//! The FX chain editor and the macros and morph panel for the MPC One,
//! built on `mpfree-ui` over `mpfree-fx`'s [`mpfree_fx::Chain`] and
//! `mpfree-mod`'s [`mpfree_mod::ModEngine`].
//!
//! **A proposal to the Hello MPC thread,** which owns the app's layout:
//! where these panels open (a mixer strip's insert row, the Sample tab's FX
//! sub-tab, a bank of the Play page's Q-Link column) is its call and Jay's.
//! This crate is the panel inside whatever area it gets.
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
//! Parameter changes go to the mod engine as `ModEvent::Set` (it sends
//! them on to the chain); chain edits go to the mixer as `Edit`s.

#![forbid(unsafe_code)]

pub mod macros;
pub mod sample;
pub mod state;
pub mod view;

pub use macros::{MacroAction, MacrosView};
pub use state::{macro_color, ChainState, MacrosState, ParamView, SlotView};
pub use view::{ChainAction, ChainView};

use mpfree_ui::Ui;

/// The widgets' names and pages.
pub const CHAIN: &str = "chain";
pub const CHAIN_PAGE: &str = "chain";
pub const MACROS_W: &str = "macros";
pub const MACROS_PAGE: &str = "macros";
/// The pages: the app's header, the panel, the hint line. The app puts its
/// Q-Link column wherever its layout has it.
pub const CHAIN_LAYOUT: &str = "col(header:60, chain:*, hint:26)";
pub const MACROS_LAYOUT: &str = "col(header:60, macros:*, hint:26)";
pub const CHAIN_HINT: &str = "Tap an effect to edit it  ·  drag a knob  ·  a knob you touch can go on a macro, a Q-Link or an LFO";
pub const MACROS_HINT: &str = "Drag a macro  ·  tap a target for its range  ·  the morph glides every scene parameter from A to B";

/// Adds both widgets and their pages. The app's `header` and `hint`
/// widgets must be there already.
pub fn add(ui: &mut Ui) -> Result<(), String> {
    ui.add(CHAIN, ChainView::new());
    ui.add(MACROS_W, MacrosView::new());
    ui.add_page(CHAIN_PAGE, CHAIN_LAYOUT)?;
    ui.add_page(MACROS_PAGE, MACROS_LAYOUT)
}

pub fn update_chain(ui: &mut Ui, st: ChainState) {
    if let Some(v) = ui.get_mut::<ChainView>(CHAIN) { v.update(st); }
}

pub fn update_macros(ui: &mut Ui, st: MacrosState) {
    if let Some(v) = ui.get_mut::<MacrosView>(MACROS_W) { v.update(st); }
}

pub fn take_chain_actions(ui: &mut Ui) -> Vec<ChainAction> { ui.get_mut::<ChainView>(CHAIN).map(|v| v.take_actions()).unwrap_or_default() }

pub fn take_macro_actions(ui: &mut Ui) -> Vec<MacroAction> { ui.get_mut::<MacrosView>(MACROS_W).map(|v| v.take_actions()).unwrap_or_default() }
