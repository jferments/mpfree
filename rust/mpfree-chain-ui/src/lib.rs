//! The FX chain editor and the macros and morph panel for the MPC One,
//! built on `mpfree-ui` over `mpfree-fx`'s [`mpfree_fx::Chain`] and
//! `mpfree-mod`'s [`mpfree_mod::ModEngine`].
//!
//! **Where they open** (layout spec section 4b, approved by Jay
//! 2026-11-11 22:34Z): the chain editor takes the mixer's place when an
//! insert row is tapped there, with a Back to the mixer
//! ([`ChainState::back`]); the mixer's "+" on a strip opens it with the
//! effect picker up ([`ChainState::picking`]). It also sits in the Sample
//! tab's FX sub-tab under the pad bar, with a switch between the pad's
//! chain and the track's ([`ChainState::scopes`]). Macros 1-4 and 5-8 are
//! banks of the Q-Link column, and a long press on a macro card opens the
//! macros panel over the page under the top bar, with Back in its corner.
//! While the chain editor is open, the Q-Links default to the selected
//! effect's first four knobs: pass [`QLINK_PAGE`] and
//! [`ChainState::qlinks`] to `Bindings::set_page`. This crate is the panel
//! inside whatever area it gets.
//!
//! - [`state`]: what the panels show ([`ChainState`], [`MacrosState`]),
//!   built on the control thread; `from_chain` and `from_engine` read a
//!   chain and an engine directly, for previews and tests.
//! - [`sections`]: the selected effect's page as labelled sections, from
//!   its plan (`mpfree_fx::page_plan`), or by group for a plugin.
//! - [`view`]: the chain editor ([`ChainView`]): the strip's effects, the
//!   effect picker, the selected effect's page (its Mode chooser or page
//!   tabs, its sections drawn by the Synth tab's section widget with the
//!   effect page's drawings, modulation rings and macro badges) and the
//!   Modulate row under it (macro, learn to a Q-Link, + LFO, + Follower,
//!   the matrix rows over the page).
//! - [`macros`]: the macros and morph panel ([`MacrosView`]), with the
//!   target editor (range, curve, flip, remove).
//! - [`sample`]: sample screens for the workstation preview
//!   (`mpfree-chain-preview`) and the tests.
//!
//! For the app: [`add`] puts both widgets and their pages in the [`Ui`];
//! [`update_chain`] and [`update_macros`] show new states, and
//! [`set_chain_picture`] the selected effect's drawings
//! ([`state::picture`], every one its effect page draws: each section
//! shows those its plan names); after a touch, [`take_chain_actions`] and
//! [`take_macro_actions`] give what to do. Parameter changes go to the mod
//! engine as `ModEvent::hand` gives (a `Set`, or a `Touch` over a move lane
//! or while recording, with `Untouch` on [`ChainAction::Release`]; it sends
//! them on to the chain). Set-up changes (macro targets, matrix rows,
//! scenes) go to the audio thread's engine as `mpfree_mod::Change`s, chain
//! edits to the mixer as `Edit`s, and [`ChainAction::Add`] names the effect
//! for `mpfree_fx_ui::make`.

#![forbid(unsafe_code)]

pub mod macros;
pub mod picker;
pub mod sample;
pub mod sections;
pub mod state;
pub mod view;

pub use macros::{MacroAction, MacrosView};
pub use state::{
    macro_color, picks, picture, strip_id, strip_of, ChainState, MacrosState, ParamView, Pick,
    SlotView, StripList, StripPick, TargetView, STRIP_GROUPS,
};
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
pub const CHAIN_HINT: &str = "Q-Links: the effect's four, on every page  ·  SHIFT + Q-Link: fine  ·  drag an effect: move it  ·  touch a control: macro, Learn, + LFO  ·  strip button: other strips";
/// The hint line while the strip list is open.
pub const STRIPS_HINT: &str =
    "Tap a strip: its chain  ·  Tracks, Pads and the rest: those strips  ·  Back: keep this one";
/// The hint while the effect picker is open.
pub const PICKER_HINT: &str = "Wheel: effect  ·  SHIFT + wheel: category  ·  push: its presets, then add  ·  hold a tile: presets  ·  star: favourite";
pub const MACROS_HINT: &str = "Q-Links: the 4 macros shown  ·  SHIFT + Q-Link: fine  ·  tap a target to edit its range and curve  ·  Back closes";

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

/// Shows the selected effect's drawings in the chain editor
/// ([`state::picture`] with page `""`: every one its effect page draws;
/// empty for none).
pub fn set_chain_picture(ui: &mut Ui, cards: Vec<mpfree_synth_ui::widgets::Card>) {
    if let Some(v) = ui.get_mut::<ChainView>(CHAIN) {
        v.set_picture(cards);
    }
}

pub fn update_macros(ui: &mut Ui, st: MacrosState) {
    if let Some(v) = ui.get_mut::<MacrosView>(MACROS_W) {
        v.update(st);
    }
}

/// The data wheel on the chain editor: the picker takes it while it is
/// open ([`ChainView::wheel`]); false means it didn't.
pub fn chain_wheel(ui: &mut Ui, delta: i32, shift: bool) -> bool {
    ui.get_mut::<ChainView>(CHAIN)
        .is_some_and(|v| v.wheel(delta, shift))
}

/// The data wheel's push on the chain editor ([`ChainView::push_wheel`]);
/// false when the picker is shut.
pub fn chain_push(ui: &mut Ui) -> bool {
    ui.get_mut::<ChainView>(CHAIN)
        .is_some_and(|v| v.push_wheel())
}

/// A finger held on a picker tile past the hold time opens its presets
/// while it is still down ([`ChainView::hold_tick`]); call it every frame
/// while the chain editor shows. True when it did (draw again).
pub fn chain_hold(ui: &mut Ui) -> bool {
    if !ui.get::<ChainView>(CHAIN).is_some_and(|v| v.hold_due()) {
        return false;
    }
    ui.get_mut::<ChainView>(CHAIN)
        .is_some_and(|v| v.hold_tick())
}

/// The effect picker is open.
pub fn chain_picking(ui: &Ui) -> bool {
    ui.get::<ChainView>(CHAIN)
        .is_some_and(|v| v.state().picking)
}

/// The chain editor's hint line: [`PICKER_HINT`] while the effect picker
/// is open, [`STRIPS_HINT`] while the strip list is, else [`CHAIN_HINT`].
pub fn chain_hint(ui: &Ui) -> &'static str {
    match ui.get::<ChainView>(CHAIN) {
        Some(v) if v.state().picking => PICKER_HINT,
        Some(v) if v.state().list.is_some() => STRIPS_HINT,
        _ => CHAIN_HINT,
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
