//! The mixer screen for the MPC One: Track Mix and Pad Mix, built on
//! `mpfree-ui` over `mpfree-fx`'s [`Mixer`].
//!
//! The Mix tab (layout spec section 3a) holds it: TRACK MIX opens the tab
//! on the tracks and SHIFT + TRACK MIX on the pads, both on Faders; the
//! same button again goes back to the page before. This crate is the
//! panel under the top bar, in four looks of the same strips
//! ([`Look`]): Faders, the strips side by side (drawn for Make or for
//! Perform, [`set_perform`]); Sends, each strip's pan, sends and output as
//! a row ([`sends_look`]); Effects, each strip's chain as a row of slots;
//! and EQ, each strip's EQ as a curve ([`eq_look`]).
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
//! [`IoIcon`] at the top bar's right edge, and while it shows passes the data
//! wheel, - and + and the page arrows to [`io_input`], Back to [`io_back`],
//! each frame to [`io_tick`], and the Q-Links to [`io_qlinks`] and
//! [`io_qlink_turn`] first (when they give nothing, the Q-Links stay on the
//! selected strip as above). After a touch, a key or a Q-Link,
//! [`take_io_changes`] names the sections whose state to read.

#![forbid(unsafe_code)]

pub mod eq_look;
pub mod io;
pub mod qlinks;
pub mod sample;
pub mod sends_look;
pub mod state;
pub mod view;

pub use io::{IoInput, IoKnob, IoReply, IoSection};
pub use qlinks::{QLinkView, QLinks};
pub use state::{
    default_name, InputView, InsertView, MixState, MixView, SafetyView, SendView, StripView,
    TakeView, TrackWait,
};
pub use view::{IoIcon, Look, MixAction, MixerView, StripMenu, FX_COLS};

use mpfree_fx::Mixer;
use mpfree_ui::Ui;

/// The widget's name.
pub const MIXER: &str = "mixer";
/// The page's name in the [`Ui`].
pub const MIXER_PAGE: &str = "mixer";
/// The page: the app's header, the mixer, the hint line.
pub const MIXER_LAYOUT: &str = "col(header:60, mixer:*, hint:26)";
/// The hint line while the mixer is open on Faders.
pub const MIXER_HINT: &str = "Drag a fader  ·  tap M or S  ·  a strip's name: its Q-Links  ·  FX or sends: their look  ·  the level: type it  ·  TRACK MIX: back";
/// The hint line on Faders once the app offers Freeze, Thaw and Print
/// ([`MixState::takes`]).
pub const MIXER_HINT_TAKES: &str = "Drag a fader  ·  tap M or S  ·  a strip's name: its Q-Links (again: Freeze, Print)  ·  FX or sends: their look  ·  the level: type it  ·  TRACK MIX: back";
/// The hint line on Sends.
pub const MIXER_HINT_SENDS: &str = "Drag a pan or a send  ·  tap pre: before the fader  ·  an output: where it goes  ·  a strip's name: its Q-Links  ·  Faders: the levels  ·  TRACK MIX: back";
/// The hint line on Faders in Perform.
pub const MIXER_HINT_PERFORM: &str = "A track's MUTE or SOLO: on the launch line (SHIFT: at once)  ·  drag a fader or a pan  ·  a strip's name: its Q-Links  ·  TRACK MIX: back";
/// The hint line on Effects.
pub const MIXER_HINT_EFFECTS: &str = "Tap + FX: add an effect  ·  tap an effect: its knobs, again: its chain  ·  drag one along its row: move it  ·  Faders: the levels  ·  TRACK MIX: back";
/// The hint line on EQ.
pub const MIXER_HINT_EQ: &str = "Tap a curve: its band on the Q-Links, again: open it  ·  drag a dot: move that band  ·  a strip's name: its Q-Links  ·  + EQ: add one  ·  TRACK MIX: back";
/// The hint line while the I/O view shows.
pub const MIXER_HINT_IO: &str = "Tap a row to select it  ·  tap an output to route it  ·  wheel: select  ·  - and +: step its output  ·  TRACK MIX: back";

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

/// The strip parameter the control `t` sets, for SHIFT + a touch's MIDI
/// Learn: (strip, parameter id) for a strip's fader, pan, send, M or S on
/// the Faders look, else None.
pub fn param_of(ui: &Ui, t: &mpfree_ui::Target) -> Option<(mpfree_fx::StripId, u16)> {
    let r = ui.rect_of(MIXER)?;
    ui.get::<MixerView>(MIXER)?.param_of(t, r)
}

/// The track whose mute and solo strip `s`'s M and S are
/// ([`StripView::track`], set by the app: one mute per track), if any.
pub fn track_of(ui: &Ui, s: mpfree_fx::StripId) -> Option<usize> {
    let v = ui.get::<MixerView>(MIXER)?;
    v.state().strips.iter().find(|v| v.id == s)?.track
}

/// The look showing: Faders or Effects. The widget keeps it, so the Mix
/// tab opens on the look used last.
pub fn look(ui: &Ui) -> Look {
    ui.get::<MixerView>(MIXER)
        .map_or(Look::Faders, |v| v.look())
}

/// The effect picked on the Effects look, (strip, slot): while there is
/// one the Q-Links turn its first four knobs (the chain editor's, which
/// the app keeps for it after [`MixAction::PickInsert`]).
pub fn pick(ui: &Ui) -> Option<(mpfree_fx::StripId, usize)> {
    ui.get::<MixerView>(MIXER).and_then(|v| v.pick())
}

/// Shows look `l` (TRACK MIX opens the tab on Faders).
pub fn set_look(ui: &mut Ui, l: Look) {
    if let Some(v) = ui.get_mut::<MixerView>(MIXER) {
        v.set_look(l);
    }
}

/// The hint line for the strip views: [`MIXER_HINT_SENDS`] on Sends,
/// [`MIXER_HINT_EFFECTS`] on Effects, [`MIXER_HINT_EQ`] on EQ, and on
/// Faders [`MIXER_HINT_PERFORM`] in Perform, else [`MIXER_HINT_TAKES`] or
/// [`MIXER_HINT`]. The I/O view has its own ([`io_hint`]).
pub fn hint(ui: &Ui) -> &'static str {
    ui.get::<MixerView>(MIXER).map_or(MIXER_HINT, |v| {
        if v.look() == Look::Faders && v.perform() {
            MIXER_HINT_PERFORM
        } else {
            hint_for(v.look(), v.state().takes)
        }
    })
}

/// The hint line for look `l` in Make; `takes` as [`MixState::takes`].
pub fn hint_for(l: Look, takes: bool) -> &'static str {
    match l {
        Look::Sends => MIXER_HINT_SENDS,
        Look::Effects => MIXER_HINT_EFFECTS,
        Look::Eq => MIXER_HINT_EQ,
        Look::Faders if takes => MIXER_HINT_TAKES,
        Look::Faders => MIXER_HINT,
    }
}

/// Faders draws its strips for Perform (`on`) or for Make
/// ([`MixerView::set_perform`]): the app follows the project's Perform.
pub fn set_perform(ui: &mut Ui, on: bool) {
    if let Some(v) = ui.get_mut::<MixerView>(MIXER) {
        v.set_perform(on);
    }
}

/// The tracks' mutes and solos waiting for the launch line
/// ([`MixerView::set_waits`]): the app gives them from the time engine's
/// mute queue, and a strip whose [`StripView::track`] is one shows it.
pub fn set_waits(ui: &mut Ui, waits: Vec<TrackWait>) {
    if let Some(v) = ui.get_mut::<MixerView>(MIXER) {
        v.set_waits(waits);
    }
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

/// Shows section `section` of the I/O view: 0 is Audio, then the added
/// sections in add order from 1 (a tap on the tempo chip opens Sync this
/// way). As a tap on the list: the old section's detail closes, and once
/// the mixer shows [`MixView::Io`] the section is told it shows. The view
/// itself is the app's to pick ([`update`] with [`MixView::Io`]); the
/// section stays picked until another is. False if there is no such
/// section.
pub fn io_select(ui: &mut Ui, section: usize) -> bool {
    ui.get_mut::<MixerView>(MIXER)
        .is_some_and(|v| v.show_io_section(section))
}

/// The hint line while the I/O view shows: the section's own, or
/// [`MIXER_HINT_IO`]; `None` away from the I/O view (keep the mixer's
/// hint). Set the `hint` label to it after a view or section change.
pub fn io_hint(ui: &Ui) -> Option<String> {
    ui.get::<MixerView>(MIXER).and_then(|v| v.io_hint())
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
    // Marked for drawing only when something changed: called every frame,
    // `get_mut` alone had the screen paint (nothing) at every frame.
    let redraw = ui
        .get_mut_quiet::<MixerView>(MIXER)
        .is_some_and(|v| v.io_tick(now) == IoReply::Redraw);
    if redraw {
        ui.get_mut::<MixerView>(MIXER);
    }
    redraw
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

/// What the I/O section's last touch, key or Q-Link couldn't do, and why,
/// once: say it on the hint line ([`IoSection::take_note`]).
pub fn take_io_note(ui: &mut Ui) -> Option<String> {
    // Taking the note changes nothing the view draws (the app says it on
    // its hint line), and the app asks every frame: no redraw for it.
    ui.get_mut_quiet::<MixerView>(MIXER)
        .and_then(|v| v.take_io_note())
}

/// Sections (by add order) that replied [`IoReply::Redraw`] to a touch, a
/// key ([`io_input`]) or a Q-Link ([`io_qlink_turn`]) since the last call:
/// read their state.
pub fn take_io_changes(ui: &mut Ui) -> Vec<usize> {
    ui.get_mut::<MixerView>(MIXER)
        .map(|v| v.take_io_changes())
        .unwrap_or_default()
}
