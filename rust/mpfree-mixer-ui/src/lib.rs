//! The mixer screen for the MPC One: Track Mix and Pad Mix, built on
//! `mpfree-ui` over `mpfree-fx`'s [`Mixer`].
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
//! - [`qlinks`](mod@qlinks): the four Q-Links while the mixer is open: the selected
//!   strip's Volume, Pan, Send A and Send B, what each card says and what a
//!   turn does.
//! - [`io`](mod@io): the I/O view ([`MixView::Io`]): where a project's audio,
//!   MIDI, sync and CV go. Its Audio section is the mixer's own; MIDI, sync
//!   and CV come from other crates as [`IoSection`]s ([`add_io_section`]).
//! - [`sample`]: sample screens from a real mixer playing, for the
//!   workstation preview (`mpfree-mixer-preview`) and the tests.
//!
//! For the app: [`add`] puts the widget and its page in the [`Ui`] (the
//! page uses the app's own `header` and `hint` widgets), [`update`] shows a
//! new state (only the meters are drawn again when only they changed),
//! and after a touch event from [`MIXER`], [`take_actions`] gives what to
//! do: [`MixAction::Set`] goes to the mixer as a parameter event
//! ([`apply`] does it on a mixer the caller owns). While the mixer is open,
//! [`qlinks`](fn@qlinks) gives the Q-Link cards' labels and text, and [`qlink_turn`]
//! (or [`qlink_to`]) turns a Q-Link into a [`MixAction::Set`] to send the
//! same way; the widget shows the change at once.
//!
//! The I/O view: the app opens the mixer on [`MixView::Io`] from the
//! [`IoIcon`] beside the mixer icon, and while it shows passes the data
//! wheel, - and + and the page arrows to [`io_input`], Back to [`io_back`],
//! each frame to [`io_tick`], and the Q-Links to [`io_qlinks`] and
//! [`io_qlink_turn`] first (when they give nothing, the Q-Links stay on the
//! selected strip as above). After a touch event, [`take_io_changes`] names
//! the sections whose state to read.

#![forbid(unsafe_code)]

pub mod io;
pub mod qlinks;
pub mod sample;
pub mod state;
pub mod view;

pub use io::{IoInput, IoKnob, IoReply, IoSection};
pub use qlinks::{QLinkView, QLinks};
pub use state::{
    default_name, InputView, InsertView, MixState, MixView, SafetyView, SendView, StripView,
    TakeView,
};
pub use view::{IoIcon, MixAction, MixerIcon, MixerView, StripMenu};

use mpfree_fx::Mixer;
use mpfree_ui::Ui;

/// The widget's name.
pub const MIXER: &str = "mixer";
/// The page's name in the [`Ui`].
pub const MIXER_PAGE: &str = "mixer";
/// The page: the app's header, the mixer, the hint line.
pub const MIXER_LAYOUT: &str = "col(header:60, mixer:*, hint:26)";
/// The hint line while the mixer is open.
pub const MIXER_HINT: &str = "Drag a fader, pan or send  ·  tap M, S or an output  ·  a strip's name selects it for the Q-Links  ·  TRACK MIX closes  ·  SHIFT + TRACK MIX: pads";
/// The hint line once the app offers Freeze, Thaw and Print
/// ([`MixState::takes`]).
pub const MIXER_HINT_TAKES: &str = "Drag a fader, pan or send  ·  tap M, S or an output  ·  a strip's name selects it (again: Freeze, Print)  ·  TRACK MIX closes  ·  SHIFT + TRACK MIX: pads";
/// The hint line while the I/O view shows.
pub const MIXER_HINT_IO: &str = "Tap a row to select it  ·  tap an output to route it  ·  wheel: select  ·  - and +: step its output  ·  the I/O icon or TRACK MIX closes";

/// Adds the mixer widget and its page. The app's `header` and `hint`
/// widgets must be there already (the page uses them).
pub fn add(ui: &mut Ui) -> Result<(), String> {
    if ui.get::<mpfree_ui::widgets::Header>("header").is_none() {
        ui.add("header", mpfree_ui::widgets::Header::new(&[]));
    }
    if ui.get::<mpfree_ui::widgets::Label>("hint").is_none() {
        ui.add(
            "hint",
            mpfree_ui::widgets::Label::new(
                MIXER_HINT,
                mpfree_ui::Tone::Dim,
                mpfree_ui::Align::Left,
            ),
        );
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
    ui.get_mut::<MixerView>(MIXER)
        .map(|v| v.take_actions())
        .unwrap_or_default()
}

/// What the four Q-Links' cards say: the selected strip's Volume, Pan,
/// Send A and Send B, as the mixer widget shows them now (so a fader under
/// a finger reads where the finger has it).
pub fn qlinks(ui: &Ui) -> QLinks {
    ui.get::<MixerView>(MIXER).map_or_else(
        || qlinks::show(&MixState::default()),
        |v| qlinks::show(v.state()),
    )
}

/// Q-Link `knob` (0 to 3) turned by `detents`; `fine` while SHIFT is held.
/// The widget shows the change at once; send the action to the mixer as a
/// parameter event. `None` when nothing changes.
pub fn qlink_turn(ui: &mut Ui, knob: usize, detents: i32, fine: bool) -> Option<MixAction> {
    let v = ui.get_mut::<MixerView>(MIXER)?;
    let a = qlinks::turn(v.state(), knob, detents, fine)?;
    v.show(&a);
    Some(a)
}

/// Q-Link `knob` (0 to 3) put at `pos` (0 to 1). As [`qlink_turn`].
pub fn qlink_to(ui: &mut Ui, knob: usize, pos: f32) -> Option<MixAction> {
    let v = ui.get_mut::<MixerView>(MIXER)?;
    let a = qlinks::to(v.state(), knob, pos)?;
    v.show(&a);
    Some(a)
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

/// Adds a section (MIDI, sync, CV) to the I/O view's list, after Audio and
/// the sections added before it. Its index in [`take_io_changes`] is its
/// place in add order, from 0.
pub fn add_io_section(ui: &mut Ui, s: Box<dyn IoSection>) {
    if let Some(v) = ui.get_mut::<MixerView>(MIXER) {
        v.add_io_section(s);
    }
}

/// The data wheel, - and +, or the page arrows while the I/O view shows.
/// On Audio, actions may follow ([`take_actions`]); [`IoReply::Ignored`]
/// when the I/O view is not showing.
pub fn io_input(ui: &mut Ui, ev: IoInput) -> IoReply {
    ui.get_mut::<MixerView>(MIXER)
        .map_or(IoReply::Ignored, |v| v.io_input(ev))
}

/// Each frame, with the time since the app started: the section showing
/// updates its live values. True when the screen needs drawing.
pub fn io_tick(ui: &mut Ui, now: std::time::Duration) -> bool {
    ui.get_mut::<MixerView>(MIXER)
        .is_some_and(|v| v.io_tick(now) == IoReply::Redraw)
}

/// The app's Back while the I/O view shows: true if it closed something
/// (the output picker, or a section's detail); false lets Back do what it
/// does elsewhere.
pub fn io_back(ui: &mut Ui) -> bool {
    ui.get_mut::<MixerView>(MIXER).is_some_and(|v| v.io_back())
}

/// The Q-Link cards of the I/O section showing, if it has its own; `None`
/// leaves them on the selected strip ([`qlinks`](fn@qlinks)).
pub fn io_qlinks(ui: &Ui) -> Option<[IoKnob; 4]> {
    ui.get::<MixerView>(MIXER).and_then(|v| v.io_qlinks())
}

/// A Q-Link turned while [`io_qlinks`] gives cards.
pub fn io_qlink_turn(ui: &mut Ui, knob: usize, detents: i32, fine: bool) -> IoReply {
    ui.get_mut::<MixerView>(MIXER)
        .map_or(IoReply::Ignored, |v| v.io_qlink_turn(knob, detents, fine))
}

/// Sections (by add order) that changed a setting on a touch since the last
/// call: read their state.
pub fn take_io_changes(ui: &mut Ui) -> Vec<usize> {
    ui.get_mut::<MixerView>(MIXER)
        .map(|v| v.take_io_changes())
        .unwrap_or_default()
}
