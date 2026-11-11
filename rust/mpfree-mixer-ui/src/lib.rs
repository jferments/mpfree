//! The mixer screen for the MPC One: Track Mix and Pad Mix, built on
//! `mpfree-ui` over `mpfree-fx`'s [`Mixer`](mpfree_fx::Mixer).
//!
//! **A proposal to the Hello MPC thread,** which owns the app's layout. The
//! approved layout spec already places it: TRACK MIX, or the small mixer
//! icon at the top bar's right edge, opens the mixer over the current page,
//! under the top bar, as Prefs does; the same button or icon again, or a
//! tab, closes it; SHIFT + TRACK MIX opens it on the pads. This crate is
//! what opens: the panel inside that area.
//!
//! - [`state`]: what the screen shows ([`MixState`]), built by the control
//!   thread from its copy of the settings and the meters
//!   ([`MixState::from_mixer`] reads a mixer directly, for previews and
//!   tests).
//! - [`view`]: the widget ([`MixerView`]): views (Tracks, Pads, Inputs,
//!   Submixes and returns), pages of ten strips with the master always on
//!   the right, faders with meters, pan, Mute and Solo, sends, inserts and
//!   the output picker; touches become [`MixAction`]s.
//! - [`sample`]: sample screens from a real mixer playing, for the
//!   workstation preview (`mpfree-mixer-preview`) and the tests.
//!
//! For the app: [`add`] puts the widget and its page in the [`Ui`] (the
//! page uses the app's own `header` and `hint` widgets), [`update`] shows a
//! new state (only the meters are drawn again when only they changed),
//! and after a touch event from [`MIXER`], [`take_actions`] gives what to
//! do: [`MixAction::Set`] goes to the mixer as a parameter event
//! ([`apply`] does it on a mixer the caller owns).

#![forbid(unsafe_code)]

pub mod sample;
pub mod state;
pub mod view;

pub use state::{default_name, InputView, InsertView, MixState, MixView, SafetyView, SendView, StripView};
pub use view::{MixAction, MixerIcon, MixerView};

use mpfree_fx::Mixer;
use mpfree_ui::Ui;

/// The widget's name.
pub const MIXER: &str = "mixer";
/// The page's name in the [`Ui`].
pub const MIXER_PAGE: &str = "mixer";
/// The page: the app's header, the mixer, the hint line.
pub const MIXER_LAYOUT: &str = "col(header:60, mixer:*, hint:26)";
/// The hint line while the mixer is open.
pub const MIXER_HINT: &str = "Drag a fader, pan or send  ·  tap M, S or an output  ·  a strip's name selects it  ·  TRACK MIX closes  ·  SHIFT + TRACK MIX: pads";

/// Adds the mixer widget and its page. The app's `header` and `hint`
/// widgets must be there already (the page uses them).
pub fn add(ui: &mut Ui) -> Result<(), String> {
    if ui.get::<mpfree_ui::widgets::Header>("header").is_none() {
        ui.add("header", mpfree_ui::widgets::Header::new(&[]));
    }
    if ui.get::<mpfree_ui::widgets::Label>("hint").is_none() {
        ui.add("hint", mpfree_ui::widgets::Label::new(MIXER_HINT, mpfree_ui::Tone::Dim, mpfree_ui::Align::Left));
    }
    ui.add(MIXER, MixerView::new());
    ui.add_page(MIXER_PAGE, MIXER_LAYOUT)
}

/// Shows a new state on the mixer page.
pub fn update(ui: &mut Ui, st: MixState) {
    if let Some(v) = ui.get_mut::<MixerView>(MIXER) {
        v.update(st);
    }
}

/// What the touches on the mixer meant since the last call.
pub fn take_actions(ui: &mut Ui) -> Vec<MixAction> {
    ui.get_mut::<MixerView>(MIXER).map(|v| v.take_actions()).unwrap_or_default()
}

/// Applies an action to a mixer the caller owns (the preview and tests;
/// the app sends [`MixAction::Set`] to the audio thread instead). True if
/// it was a parameter for the mixer.
pub fn apply(m: &mut Mixer, a: &MixAction) -> bool {
    match *a {
        MixAction::Set { strip, id, value } => m.set(strip, id, value),
        _ => false,
    }
}
