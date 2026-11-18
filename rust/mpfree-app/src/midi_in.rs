//! MIDI from outside the MPC: the 5-pin MIDI In jack and USB-MIDI keyboards
//! on the USB-A socket. Read only: nothing here ever sends a byte. What thru
//! passes on (the Routing section's squares from a port to an output) is
//! handed to the app ([`MusicIn::take_thru`]), whose `midi_out` sends it,
//! only while Prefs' "Send MIDI and CV out" is on.
//!
//! - **MIDI In** is the controller's cable 2 ("MIDI Port"), seen in the
//!   owner's read-only captures of 2026-11-12
//!   (docs/dev/hardware/controller-protocol.md). The app already reads that
//!   cable with the controller's other two; [`MusicIn::bytes`] takes what
//!   came in on it. Its bytes are never read as the MPC's own pads, knobs or
//!   buttons: a keyboard on channel 1 would otherwise press buttons, and one
//!   on channel 10 would hit pads.
//! - **USB keyboards** are ALSA cards of their own. [`usb_midi_cards`] finds
//!   them in `/proc/asound/cards`, the app opens each MIDI port's input for
//!   reading ([`usb_ports`]), and [`MusicIn::plugged`] /
//!   [`MusicIn::unplugged`] keep the port table (a keyboard plugged back in
//!   gets its old port number). A port's output is never opened here: the
//!   app hands it to `midi_out`, which opens it only while sending is on.
//!
//! Every message goes through `mpfree_midi`'s `InputRouter` in its
//! Selected mode: whatever plays on any port and channel plays the armed
//! track, as on Akai's MPC ("Keys play: the track on screen", agreed by the
//! owner on 2026-11-12). A note-off always ends what its note-on started.
//! The app turns the [`Key`]s into notes for the armed track.
//!
//! Clock (F8) and active sensing (FE) are counted, not listed: a keyboard
//! sending clock the whole time otherwise filled the log with 83,000 lines
//! in 29 minutes on the owner's unit (2026-11-12). The log says when clock
//! starts and stops arriving instead. No port is the clock source yet, so
//! nothing follows that clock; its tempo is measured for the I/O view.
//!
//! **The mixer's I/O view** (`mpfree-midi-ui`) shows and edits the project's
//! MIDI settings. [`MusicIn::sync`] hands it the live facts (ports plugged
//! in, when each last received something, the clock arriving) and takes a
//! port's input setting when it changes: a port set to "Learn only" plays
//! no notes (a note-off still ends a note it started). `sync` runs in the
//! sound loop, so it never waits for the screen: when the screen holds the
//! model, that period's update is skipped and counted.
//!
//! **The MIDI monitor** (`mpfree_midi::monitor`): every message in goes
//! to [`MusicIn::feed`] as it comes, with where it went: the armed track
//! (and the pad its note plays with a kit), MIDI Learn and what it moved,
//! the clock follower, or nothing. The app adds what MIDI out sends and, in
//! debug mode, the MPC's own controller and its events; [`MusicIn::sync`]
//! moves the lines to the I/O view's Monitor.
//!
//! **MIDI Learn** (step M5): the app arms a parameter ([`MusicIn::arm`],
//! with the chain editor's Learn), and the next press or move on a port
//! whose input is "Learn only" or "Both" takes it. Learned controls are the
//! project's (`MidiSetup::learn`, by device name), so `sync` builds the
//! `LearnMap` from them and hands each new one back. What Learn takes
//! (an armed press, a mapped control) does not also play;
//! [`MusicIn::learning`] gives the app what was learned and the values to
//! set, and [`MusicIn::observe_learned`] where each parameter stands, so
//! an endless encoder steps on from a value set on the screen.

use std::sync::TryLockError;
use std::time::Duration;

use mpfree_midi::input::{ChanFilter, In, InputRouter, PortFilter, Route, Thru};
use mpfree_midi::learn::{Input, LearnMap, Mode, Saved, Source, Write};
use mpfree_midi::monitor::{Dir, Feed, Steady, To, What};
use mpfree_midi::mpe::{MpeIn, Took};
use mpfree_midi::msg::Msg;
pub use mpfree_midi::port::DIN;
use mpfree_midi::port::{PortId, Ports, MAX_PORTS};
use mpfree_midi::setup::{Ignored, Kept, PortInput, TempoSource, FOLLOW_KNOBS};
use mpfree_midi_clock::ClockIn;
use mpfree_midi_ui::{PortLive, Shared};
use mpfree_node::{Ctl, ParamInfo};
use mpfree_play::mpfree_param::{transport, Owner, ParamAddr};
use mpfree_play::Event;

use crate::time::Follow;

/// The clock's frames are microseconds of the app's clock.
const CLOCK_RATE: u32 = 1_000_000;

/// What a keyboard did, for the armed track.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Key {
    Down {
        note: u8,
        velocity: u8,
    },
    Up {
        note: u8,
    },
    /// Poly aftertouch on a held key (or a channel's pressure, or an MPE
    /// note's own: [`mpfree_midi::mpe`]).
    Pressure {
        note: u8,
        value: u8,
    },
    /// The pitch wheel: every note, -1 to 1 of the instrument's bend range.
    Bend {
        value: f32,
    },
    /// An MPE note's own slide (semitones) or tone (0 to 1).
    Expr {
        note: u8,
        kind: mpfree_node::Expr,
        value: f32,
    },
    /// A pedal (sustain, sostenuto, soft), the mod wheel, a breath or a
    /// foot controller: CC 64, 66, 67, 1, 2 or 11, 0 to 1, for the whole
    /// track ([`Ctl`]). Other CCs are MIDI Learn's.
    Control {
        ctl: Ctl,
        value: f32,
    },
}

impl Key {
    /// The key as expression out takes it for a track's gear
    /// ([`mpfree_midi::expr_out`]); None for an expression it doesn't
    /// send (a release velocity).
    pub fn expr_out(self) -> Option<mpfree_midi::expr_out::In> {
        use mpfree_midi::expr_out::In;
        Some(match self {
            Key::Down { note, .. } => In::Down { note },
            Key::Up { note } => In::Up { note },
            Key::Pressure { note, value } => In::Pressure { note, value },
            Key::Bend { value } => In::Wheel(value),
            Key::Expr {
                note,
                kind: mpfree_node::Expr::Slide,
                value,
            } => In::Slide { note, semis: value },
            Key::Expr {
                note,
                kind: mpfree_node::Expr::Tone,
                value,
            } => In::Tone { note, value },
            // Gear takes its pedals and wheels as they came
            // (`MidiOut::keys_control`), not through expression out.
            Key::Expr { .. } | Key::Control { .. } => return None,
        })
    }
}

/// What an expression does, as a key.
fn expr_key(e: mpfree_midi::mpe::Expr) -> Key {
    use mpfree_midi::mpe::Expr;
    match e {
        Expr::Bend(value) => Key::Bend { value },
        Expr::Slide { note, semis } => Key::Expr {
            note,
            kind: mpfree_node::Expr::Slide,
            value: semis,
        },
        Expr::Tone { note, value } => Key::Expr {
            note,
            kind: mpfree_node::Expr::Tone,
            value,
        },
        Expr::Pressure { note, value } => Key::Pressure { note, value },
    }
}

/// The tracks a route may name (`mpfree_midi::setup::ROUTE_TRACKS`).
const ROUTE_TRACKS: usize = mpfree_midi::setup::ROUTE_TRACKS;

/// Where each routed key went down: played on its own track at once, or
/// with the armed track's keys (its track was armed then). Its note-off and
/// pressure follow it there, even if the armed track changed while it was
/// held, so no note hangs.
pub struct RoutedHeld {
    /// Per track, a bit per note played on it at once.
    direct: Box<[u128]>,
}

impl Default for RoutedHeld {
    fn default() -> Self {
        Self::new()
    }
}

impl RoutedHeld {
    pub fn new() -> RoutedHeld {
        RoutedHeld {
            direct: vec![0; ROUTE_TRACKS].into_boxed_slice(),
        }
    }

    /// Whether `key`, routed to `track`, plays on that track at once
    /// (true) or goes with the armed track's keys (false); `armed`: the
    /// track is the armed one now.
    pub fn direct(&mut self, track: u8, key: Key, armed: bool) -> bool {
        let Some(held) = self.direct.get_mut(usize::from(track)) else {
            return false;
        };
        let bit = |note: u8| 1u128 << (note & 0x7F);
        match key {
            Key::Down { note, .. } if armed => {
                *held &= !bit(note);
                false
            }
            Key::Down { note, .. } => {
                *held |= bit(note);
                true
            }
            // A key that went down with the armed keys ends there; one
            // whose track was armed and isn't now was let go when the
            // armed track changed, so its note-off is harmless here.
            Key::Up { note } => {
                let was = *held & bit(note) != 0;
                *held &= !bit(note);
                was || !armed
            }
            Key::Pressure { note, .. } | Key::Expr { note, .. } => *held & bit(note) != 0 || !armed,
            // The wheels and pedals move the track they are routed to, as
            // it is now.
            Key::Bend { .. } | Key::Control { .. } => !armed,
        }
    }
}

/// No pad: the key's note plays no pad alone on its track.
const NO_PAD: u8 = u8::MAX;

/// The pad each key a route played on a track (not the armed one) went to,
/// for CAPTURE: the key's end and pressure go to the pad its note went to,
/// even if the track's pads changed while it was held.
pub struct RoutedPads {
    /// Per track a pad per note, [`NO_PAD`] for none.
    pads: Box<[u8]>,
}

impl Default for RoutedPads {
    fn default() -> Self {
        Self::new()
    }
}

impl RoutedPads {
    pub fn new() -> RoutedPads {
        RoutedPads {
            pads: vec![NO_PAD; ROUTE_TRACKS * 128].into_boxed_slice(),
        }
    }

    /// What CAPTURE keeps of `key`, routed to `track`: its note as the pad
    /// `pad_of` says plays it there (`midi_file::pad_on_track`), and its
    /// end and pressure on that pad. None for the wheel, an MPE note's own
    /// slide and tone, a note no pad plays alone (as the armed track's
    /// keys, it plays but isn't kept), and an end or pressure whose note
    /// went to no pad. Allocates nothing.
    pub fn event(
        &mut self,
        track: u8,
        key: Key,
        pad_of: impl FnOnce(u8) -> Option<u8>,
    ) -> Option<Event> {
        let t = usize::from(track);
        if t >= ROUTE_TRACKS {
            return None;
        }
        let at = |note: u8| t * 128 + usize::from(note & 0x7F);
        match key {
            Key::Down { note, velocity } => {
                let pad = pad_of(note).filter(|&p| p < 128);
                self.pads[at(note)] = pad.unwrap_or(NO_PAD);
                pad.map(|note| Event::NoteOn {
                    note,
                    velocity: velocity.max(1),
                })
            }
            Key::Up { note } => {
                let pad = std::mem::replace(&mut self.pads[at(note)], NO_PAD);
                (pad != NO_PAD).then_some(Event::NoteOff { note: pad })
            }
            Key::Pressure { note, value } => {
                let pad = self.pads[at(note)];
                (pad != NO_PAD).then_some(Event::Pressure { note: pad, value })
            }
            Key::Bend { .. } | Key::Expr { .. } | Key::Control { .. } => None,
        }
    }
}

/// What MIDI Learn did, for the app.
#[derive(Clone, Debug, PartialEq)]
pub enum Learning {
    /// A control took the armed parameter: what it is, in words.
    Took { target: ParamAddr, control: String },
    /// A learned control moved: set this.
    Set(Write),
}

/// Whether a key learned on `target` flips it on each press (as the MPC's
/// TRACK MUTE pads do) rather than holding it while held: PLAY and STOP,
/// a mixer strip's mute and solo, a track's mute and solo on the Clips
/// grid, and a loop's mute. Works as changes it on the Learn section.
pub fn key_toggles(target: ParamAddr) -> bool {
    use mpfree_fx::ids::mixer as mx;
    match target.owner {
        Owner::Transport => target.param == transport::PLAYING,
        Owner::Clips => mpfree_play::mpfree_param::clips::is_value(target.param),
        Owner::Looper => mpfree_play::mpfree_param::looper::is_value(target.param),
        Owner::Strip(_) => matches!(target.param, mx::MUTE | mx::SOLO),
        _ => false,
    }
}

/// How a control learned on `target` from `source` works, where it isn't
/// Learn's own choice (a key: while held; anything else: a knob or
/// fader): a key on a switch you flip flips it ([`key_toggles`]), and
/// the looper's presses (a loop's key, Next loop, Undo ...) work while
/// held whatever sends them, so a footswitch's CC goes down and lets go
/// as a pad does.
pub fn learned_mode(source: &Source, target: ParamAddr) -> Option<Mode> {
    use mpfree_play::mpfree_param::looper;
    if matches!(source, Source::Note(_)) && key_toggles(target) {
        return Some(Mode::Toggle);
    }
    (target.owner == Owner::Looper && !looper::is_value(target.param)).then_some(Mode::Momentary)
}

/// Clock counted as "stopped" this long after its last tick.
const CLOCK_GAP_S: f64 = 1.0;

/// How often Learn reads where its parameters stand
/// ([`MusicIn::observe_learned`]).
const LEARN_LOOK_S: f64 = 0.05;
/// How long after a learned control set a parameter Learn leaves it
/// alone: the mixer may not have taken the value yet.
const LEARN_QUIET_S: f64 = 0.25;

/// The followed port's clock bytes kept for the time engine in one
/// period: at 300 BPM a pulse comes every 8 ms, so a few at most.
const LEADER_BYTES: usize = 64;

/// Routed keys kept for the app between its takes; a period's keys past
/// this still go in (the Vec grows), so none is lost.
const ROUTED_KEYS: usize = 256;

/// Messages thru passes on, kept for the app between its takes; a period's
/// past this are dropped and counted, so the sound loop never grows the
/// list (a port's MIDI out takes 256 a period at most).
const THRU_MSGS: usize = 256;

/// The most Q-Link turns MIDI Follow keeps for the app between two looks.
const KNOB_TURNS: usize = 64;

/// MIDI Follow on this run's port numbers (the rig's `MidiSetup::follow`):
/// the controller's knobs turn the Q-Links, each by as much as it moved.
#[derive(Debug)]
struct Knobs {
    /// The controller, while Follow is on and it is plugged in.
    port: Option<PortId>,
    /// Its knobs' CCs, in Q-Link order.
    ccs: [Option<u8>; FOLLOW_KNOBS],
    /// Each knob's last value: the next turns its Q-Link by the change.
    at: [Option<u8>; FOLLOW_KNOBS],
    /// Turns (Q-Link, detents) until the app takes them; fixed room.
    turns: Vec<(u8, i32)>,
    /// Learn knobs listens: the knobs found so far, in order.
    learning: bool,
    found: [Option<u8>; FOLLOW_KNOBS],
    /// A knob was found since the setup last took them.
    found_new: bool,
    /// Send values back: the controller's output, while Follow and it
    /// are on and it is plugged in.
    back: Option<PortId>,
    /// The channel each knob last came in on (its value goes back there).
    ch: [u8; FOLLOW_KNOBS],
    /// When each knob last turned its Q-Link (the app's clock, s).
    turned: [f64; FOLLOW_KNOBS],
    /// Each Q-Link's value as the screen shows it, 0 to 127.
    want: [Option<u8>; FOLLOW_KNOBS],
    /// The value last sent back on each knob.
    sent: [Option<u8>; FOLLOW_KNOBS],
}

/// How long after a knob turns before its Q-Link's value goes back to
/// it: the hand is on it, its own turns moved the value, and a motor
/// fader would fight the finger.
pub const HOLD_BACK: f64 = 0.3;

impl Knobs {
    fn new() -> Knobs {
        Knobs {
            port: None,
            ccs: [None; FOLLOW_KNOBS],
            at: [None; FOLLOW_KNOBS],
            turns: Vec::with_capacity(KNOB_TURNS),
            learning: false,
            found: [None; FOLLOW_KNOBS],
            found_new: false,
            back: None,
            ch: [0; FOLLOW_KNOBS],
            turned: [f64::NEG_INFINITY; FOLLOW_KNOBS],
            want: [None; FOLLOW_KNOBS],
            sent: [None; FOLLOW_KNOBS],
        }
    }

    /// A CC from `port`: the Q-Link it turns, or the knob Learn knobs
    /// found it as; None when it isn't Follow's. A knob's first value
    /// only says where it is.
    fn cc(&mut self, port: PortId, ch: u8, cc: u8, value: u8, t: f64) -> Option<u8> {
        if self.port != Some(port) || cc >= 120 {
            return None;
        }
        if self.learning {
            if let Some(k) = self.found.iter().position(|f| *f == Some(cc)) {
                return Some(k as u8);
            }
            let n = self.found.iter().flatten().count();
            self.found[n] = Some(cc);
            self.found_new = true;
            if n + 1 == FOLLOW_KNOBS {
                self.learning = false;
            }
            return Some(n as u8);
        }
        let k = self.ccs.iter().position(|c| *c == Some(cc))?;
        self.ch[k] = ch;
        self.turned[k] = t;
        if let Some(was) = self.at[k].replace(value) {
            let d = i32::from(value) - i32::from(was);
            if d != 0 && self.turns.len() < self.turns.capacity() {
                self.turns.push((k as u8, d));
            }
        }
        Some(k as u8)
    }
}

/// A message thru passes on: as it came in on `from`, for the output
/// port `to`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Passed {
    pub from: PortId,
    pub to: PortId,
    pub msg: Msg,
}

/// What a learned control last set a parameter to (0 to 1), and when.
#[derive(Clone, Copy, Debug)]
struct LearnedSet {
    target: ParamAddr,
    unit: f32,
    at: f64,
}

/// Notes what a learned control set, in room made at the start.
/// A learned control after a switch of mode (`landed`): on the new
/// engine's matching parameter, keeping its range, or false to unlearn it.
fn follow(s: &mut Saved, landed: &crate::fxmodes::Landed) -> bool {
    match landed.target(s.target) {
        mpfree_mod::Moved::Stays => true,
        mpfree_mod::Moved::To(to, _) => {
            s.target = to;
            true
        }
        mpfree_mod::Moved::Gone => false,
    }
}

/// What Learn and MIDI Follow do with a message as it came in on `port`
/// (before any route's blocks): true when one of them took it, so it
/// plays nothing and goes nowhere else.
#[allow(clippy::too_many_arguments)]
fn learn_or_follow(
    learn: &mut LearnMap,
    knobs: &mut Knobs,
    learning: &mut Vec<Learning>,
    sets: &mut Vec<LearnedSet>,
    port: PortId,
    raw: &Msg,
    t: f64,
    learns: bool,
    to: &mut To,
    learned: &mut Option<usize>,
) -> bool {
    if learns && learn.takes(port, raw) {
        if let Some(target) = learn.target_of(port, raw) {
            *to = To::Learned(target);
        }
        learn.input(port, raw, |i| match i {
            Input::Learned(i) => *learned = Some(i),
            Input::Write(w) => {
                *to = To::Learned(w.target);
                note_set(sets, w.target, w.value, t);
                learning.push(Learning::Set(w))
            }
        });
        return true;
    }
    // MIDI Follow: the controller's knob turns a Q-Link (a learned
    // control above wins), and plays nothing.
    if let Msg::Cc { ch, cc, value } = *raw {
        if let Some(k) = knobs.cc(port, ch, cc, value, t) {
            *to = To::QLink(k);
            return true;
        }
    }
    false
}

fn note_set(sets: &mut Vec<LearnedSet>, target: ParamAddr, unit: f32, at: f64) {
    let room = sets.len() < sets.capacity();
    match sets.iter_mut().find(|s| s.target == target) {
        Some(s) => {
            s.unit = unit;
            s.at = at;
        }
        None if room => sets.push(LearnedSet { target, unit, at }),
        None => {}
    }
}

/// Per port: what the log counts instead of listing.
#[derive(Clone, Copy, Debug, Default)]
struct Seen {
    clock_ticks: u64,
    sensing: u64,
    clock_running: bool,
    last_tick: f64,
    /// Inside a SysEx message, and its length so far.
    sysex: Option<usize>,
    sysex_messages: u64,
    messages: u64,
}

/// Who armed MIDI Learn ([`MusicIn::arm_from`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArmedBy {
    /// The chain editor's Learn: it ends when the editor closes.
    Chain,
    /// The I/O view's Learn again: until a control takes it or Cancel.
    Io,
    /// SHIFT + a touch on a value: until a control takes it, the same
    /// value again, or the Learn section's Cancel.
    Touch,
}

pub struct MusicIn {
    router: InputRouter,
    ports: Ports,
    seen: [Seen; MAX_PORTS],
    /// Per port: what its input does (the project's setting).
    input: [PortInput; MAX_PORTS],
    /// Per port: when a message last arrived (lights the I/O view).
    last_in: [Option<f64>; MAX_PORTS],
    /// Per port: the clock arriving, for its tempo.
    clock: [ClockIn; MAX_PORTS],
    /// The I/O view's setup generation taken last.
    applied: Option<u64>,
    /// The port table changed since the I/O view was last told.
    ports_changed: bool,
    /// The clock tempo the I/O view shows: port and tenths of a BPM.
    shown_clock: Option<(usize, i64)>,
    /// The names given to ports on their pages, by id (the rig's; only
    /// the ports renamed), for what the screen says when Learn takes a
    /// control. The log keeps each port's own name.
    shown: Vec<(PortId, String)>,
    /// Periods whose I/O view update was skipped because the screen held
    /// the model.
    busy: u64,
    /// The project's learned controls, on this run's port numbers.
    learn: LearnMap,
    /// What Learn did since the app last asked.
    learning: Vec<Learning>,
    /// Controls learned but not yet in the project (the screen held it).
    unsaved: Vec<Saved>,
    /// The I/O view's Learn again: the learned control the next one
    /// learned replaces (its range, takeover and where it is kept carry
    /// over). Learn armed from the I/O view stays armed until a control
    /// takes it or Cancel; the chain editor's ends when the editor does.
    relearn: Option<(Kept, Saved)>,
    /// Who armed Learn, which says how long it waits.
    armed_by: ArmedBy,
    /// Switches of mode whose learned controls follow their parameters,
    /// not yet in the project (the screen held it).
    moved: Vec<crate::fxmodes::Landed>,
    /// Learned controls the project couldn't keep (past its limit).
    unkept: u64,
    /// A port went away since the app last asked
    /// ([`MusicIn::take_unplugged`]): a learned control on it never sends
    /// its let-go.
    unplugged: bool,
    /// What learned controls last set (room for every learned control,
    /// so the sound loop never grows it).
    learn_sets: Vec<LearnedSet>,
    /// When Learn next reads where its parameters stand.
    next_look: f64,
    /// Following another device's clock (the project's "Tempo from" a
    /// port): what follows, and the port (None while it isn't plugged in).
    follow: Option<Follow>,
    leader: Option<PortId>,
    /// The leader's clock, Start, Continue, Stop and Song Position bytes
    /// since the time engine last took them.
    leader_bytes: [u8; LEADER_BYTES],
    leader_len: usize,
    /// Song Position data bytes still to come from the leader.
    leader_sp: u8,
    /// Leader bytes past [`LEADER_BYTES`] in a period, dropped.
    leader_dropped: u64,
    /// The MIDI monitor's feed: every message in, each with where it
    /// went; the app adds what goes out ([`Feed::sent`]) and, in debug
    /// mode, the MPC's own controller and MPFree's events. [`MusicIn::sync`]
    /// moves it to the I/O view.
    pub feed: Feed,
    /// The armed track (from 0), and whether keys play it by note (a
    /// synth's pad layout) or as the kit pad each note plays.
    armed: (u8, KeysPlay),
    /// The Monitor asked for MIDI Out's note-offs ([`MusicIn::take_note_offs`]).
    note_offs: bool,
    /// Per port: what its page says about it ("USB · port 2 of 4 on
    /// MIO4"), set when it is plugged in.
    about: Vec<String>,
    /// Keys a route sends to its track, whichever track is armed, until
    /// the app takes them ([`MusicIn::take_routed`]).
    routed: Vec<(u8, Key)>,
    /// The pedals each port holds down and where they went, so that
    /// unplugging it lets them up there.
    pedals: Vec<PedalDown>,
    /// What thru passes on, until the app takes it
    /// ([`MusicIn::take_thru`]), and how many were dropped for want of
    /// room.
    thru: Vec<Passed>,
    thru_dropped: u64,
    /// MIDI Follow: the controller's knobs on the Q-Links.
    knobs: Knobs,
    /// Per port: its pitch wheel, channel pressure and MPE.
    mpe: Vec<MpeIn>,
    /// Controller mode's plan as the setup last gave it (None: off), and
    /// whether the app has it yet.
    controller: Option<crate::controller_out::Plan>,
    controller_new: bool,
}

impl Default for MusicIn {
    fn default() -> Self {
        Self::new()
    }
}

impl MusicIn {
    pub fn new() -> MusicIn {
        let mut router = InputRouter::new();
        // The app plays keys on the armed track itself, so the router only
        // needs one track to route to; it still pairs every note-off with
        // its note-on.
        router.set_selected(Some(0));
        let mut ports = Ports::new();
        ports.set_din(true);
        MusicIn {
            router,
            ports,
            seen: [Seen::default(); MAX_PORTS],
            input: [PortInput::default(); MAX_PORTS],
            last_in: [None; MAX_PORTS],
            clock: std::array::from_fn(|_| ClockIn::new(CLOCK_RATE)),
            applied: None,
            ports_changed: true,
            shown_clock: None,
            shown: Vec::new(),
            busy: 0,
            learn: LearnMap::new(),
            learning: Vec::new(),
            unsaved: Vec::new(),
            relearn: None,
            armed_by: ArmedBy::Chain,
            moved: Vec::new(),
            unkept: 0,
            unplugged: false,
            // The song's and the rig's.
            learn_sets: Vec::with_capacity(2 * mpfree_midi::setup::MAX_LEARNED),
            next_look: 0.0,
            follow: None,
            leader: None,
            leader_bytes: [0; LEADER_BYTES],
            leader_len: 0,
            leader_sp: 0,
            leader_dropped: 0,
            feed: Feed::new(),
            armed: (0, KeysPlay::Notes),
            note_offs: false,
            about: vec![String::new(); MAX_PORTS],
            routed: Vec::with_capacity(ROUTED_KEYS),
            pedals: Vec::new(),
            thru: Vec::with_capacity(THRU_MSGS),
            thru_dropped: 0,
            knobs: Knobs::new(),
            mpe: (0..MAX_PORTS).map(|_| MpeIn::new()).collect(),
            controller: None,
            controller_new: false,
        }
    }

    /// The names given to ports on their pages (the rig's), for the
    /// Monitor's lines and lights and what Learn says it took: only the
    /// ports renamed are listed. Runs when the setup or the ports change,
    /// so it allocates only then (and only with a port renamed).
    fn shown_names(&mut self, m: &mut mpfree_midi_ui::IoModel) {
        let shown: Vec<(PortId, String)> = self
            .ports
            .all()
            .iter()
            .filter(|p| m.setup.shown(&p.name) != p.name)
            .map(|p| (p.id, m.setup.shown(&p.name).to_string()))
            .collect();
        if shown != self.shown {
            self.shown = shown.clone();
            m.monitor.set_shown(shown);
        }
    }

    /// The four Q-Links' values as the screen shows them (0 to 127; None
    /// where a Q-Link turns nothing on the page), for MIDI Follow's Send
    /// values back.
    pub fn set_qlink_values(&mut self, values: [Option<u8>; FOLLOW_KNOBS]) {
        self.knobs.want = values;
    }

    /// MIDI Follow's Send values back, at `t` (the app's clock, s): each
    /// knob whose Q-Link's value differs from the one it was last sent,
    /// as its CC on the channel it last came in on, through `send`
    /// (false: nothing could go, so it tries again later). Not while the
    /// knob turned in the last [`HOLD_BACK`] s. A knob's next value counts
    /// from the one sent, since its lights or fader show it now.
    pub fn send_back(&mut self, t: f64, mut send: impl FnMut(PortId, Msg) -> bool) {
        let k = &mut self.knobs;
        let Some(port) = k.back else { return };
        for i in 0..FOLLOW_KNOBS {
            let (Some(cc), Some(value)) = (k.ccs[i], k.want[i]) else {
                continue;
            };
            if k.sent[i] == Some(value) || t - k.turned[i] < HOLD_BACK {
                continue;
            }
            if send(
                port,
                Msg::Cc {
                    ch: k.ch[i],
                    cc,
                    value,
                },
            ) {
                k.sent[i] = Some(value);
                k.at[i] = Some(value);
            }
        }
    }

    /// MIDI Follow's Q-Link turns since the last call: each knob of the
    /// controller that moved, as (Q-Link 0 to 3, detents). The app turns
    /// the Q-Links by them, as the MPC's own.
    pub fn take_knob_turns(&mut self) -> std::vec::Drain<'_, (u8, i32)> {
        self.knobs.turns.drain(..)
    }

    /// Whether the Monitor asked for the notes MPFree left on MIDI Out to
    /// end, since the last call (at most once a sync).
    pub fn take_note_offs(&mut self) -> bool {
        std::mem::take(&mut self.note_offs)
    }

    /// The keys routes sent to their tracks since the last take (the I/O
    /// view's Routing: a port and channel that play one track, whichever
    /// track is armed), each with its track, from 0.
    pub fn take_routed(&mut self) -> std::vec::Drain<'_, (u8, Key)> {
        self.routed.drain(..)
    }

    /// Controller mode's plan, once each time the setup changes it (None
    /// inside: off).
    pub fn take_controller(&mut self) -> Option<Option<crate::controller_out::Plan>> {
        std::mem::take(&mut self.controller_new).then(|| self.controller.clone())
    }

    /// What thru passed on since the app last took it, in the order it
    /// came, for MIDI out.
    pub fn take_thru(&mut self) -> std::vec::Drain<'_, Passed> {
        self.thru.drain(..)
    }

    /// Once a period, before the bytes: the track keys play (from 0), and
    /// how ([`KeysPlay`]). It only names where the monitor says a key went;
    /// the app plays the keys itself.
    pub fn set_armed(&mut self, track: u8, play: KeysPlay) {
        self.armed = (track, play);
    }

    /// Once a period, before the transport runs: what following another
    /// device's clock does (None: not following) and the leader's clock
    /// bytes since the last call, for `TimeHost::follow` and
    /// `TimeHost::clock_in`.
    pub fn leader_clock(&mut self, f: impl FnOnce(Option<Follow>, &[u8])) {
        f(self.follow, &self.leader_bytes[..self.leader_len]);
        self.leader_len = 0;
    }

    /// The followed port's name for the tempo chip ("MIDI In" for the
    /// 5-pin jack, a USB device's given or own name), while the project's tempo is
    /// from a port and that port is plugged in.
    pub fn leader_name(&self) -> Option<&str> {
        let id = self.leader?;
        let p = self.ports.all().iter().find(|p| p.id == id)?;
        if let Some((_, n)) = self.shown.iter().find(|(i, _)| *i == id) {
            return Some(n);
        }
        Some(if p.name == mpfree_midi::setup::DIN_NAME {
            "MIDI In"
        } else {
            &p.name
        })
    }

    /// Keeps what the time engine's follower reads of a leader's byte:
    /// clock, Start, Continue, Stop, and Song Position with its two data
    /// bytes. Anything else is left out (and ends a Song Position cut
    /// short, which the follower is told).
    fn keep_leader_byte(&mut self, b: u8) {
        let keep = match b {
            0xF8 | 0xFA | 0xFB | 0xFC => true,
            0xF2 => {
                self.leader_sp = 2;
                true
            }
            0x00..=0x7F if self.leader_sp > 0 => {
                self.leader_sp -= 1;
                true
            }
            0x80..=0xF7 if self.leader_sp > 0 => {
                self.leader_sp = 0;
                true
            }
            _ => false,
        };
        if !keep {
            return;
        }
        if self.leader_len < LEADER_BYTES {
            self.leader_bytes[self.leader_len] = b;
            self.leader_len += 1;
        } else {
            self.leader_dropped += 1;
        }
    }

    /// MIDI Learn: the next press or move on a port that learns takes
    /// `target`, for the chain editor's Learn (it ends with the editor).
    pub fn arm(&mut self, target: ParamAddr) {
        self.arm_from(target, ArmedBy::Chain);
    }

    /// MIDI Learn armed by `by`: the next press or move on a port that
    /// learns takes `target`.
    pub fn arm_from(&mut self, target: ParamAddr, by: ArmedBy) {
        self.learn.arm(target);
        self.armed_by = by;
        self.relearn = None;
    }

    pub fn disarm(&mut self) {
        self.learn.disarm();
        self.armed_by = ArmedBy::Chain;
        self.relearn = None;
    }

    /// Who armed the Learn waiting now; None when none is.
    pub fn armed_by(&self) -> Option<ArmedBy> {
        self.learn.armed().map(|_| self.armed_by)
    }

    /// Learn waits until a control takes it or it is cancelled: armed from
    /// the I/O view (Learn again) or by SHIFT + a touch, not the chain
    /// editor.
    pub fn waits(&self) -> bool {
        self.armed_by().is_some_and(|b| b != ArmedBy::Chain)
    }

    pub fn armed(&self) -> Option<ParamAddr> {
        self.learn.armed()
    }

    /// An insert switched to another engine with its mode: each control
    /// learned on one of its parameters follows it to the new engine's
    /// matching one, or is unlearned when there is none
    /// (`fxmodes::Landed`); an armed learn follows too. The project's list
    /// takes it at the next [`MusicIn::sync`].
    pub fn retarget(&mut self, landed: &crate::fxmodes::Landed) {
        use mpfree_mod::Moved;
        if let Some(a) = self.learn.armed() {
            match landed.target(a) {
                Moved::Stays => {}
                Moved::To(to, _) => self.learn.arm(to),
                Moved::Gone => self.learn.disarm(),
            }
        }
        self.moved.push(landed.clone());
    }

    /// What Learn did since the last call, in order.
    pub fn learning(&mut self) -> std::vec::Drain<'_, Learning> {
        self.learning.drain(..)
    }

    /// Every [`LEARN_LOOK_S`]: tells Learn where each learned control's
    /// parameter stands, when something other than the control moved it
    /// (or nothing has yet), so an endless encoder steps from there and a
    /// button's next press flips it from there. `read` gives a
    /// parameter's description and value (None: not one the app can
    /// read). Never allocates.
    pub fn observe_learned(
        &mut self,
        t: f64,
        mut read: impl FnMut(ParamAddr) -> Option<(ParamInfo, f32)>,
    ) {
        if t < self.next_look {
            return;
        }
        self.next_look = t + LEARN_LOOK_S;
        let mut i = 0;
        while let Some(m) = self.learn.get(i) {
            let target = m.target;
            i += 1;
            let Some((info, value)) = read(target) else {
                continue;
            };
            // What the control set, as the parameter holds it (a stepped
            // one rounds, so an encoder's part steps are kept).
            let ours = self.learn_sets.iter().find(|s| s.target == target);
            let same = |s: &LearnedSet| {
                let set = info.from_unit(s.unit);
                (set - value).abs() <= 1e-4 * set.abs().max(1.0)
            };
            if ours.is_some_and(|s| t - s.at < LEARN_QUIET_S || same(s)) {
                continue;
            }
            self.learn.observe(target, info.to_unit(value));
        }
    }

    /// Builds the learn map from the project's learned controls, on this
    /// run's port numbers (a device not seen yet listens once plugged in).
    fn build_learn(&mut self, learned: &[Saved]) {
        let armed = self.learn.armed();
        let mut map = LearnMap::new();
        for l in learned {
            let port = match &l.port {
                None => Some(PortFilter::Any),
                Some(n) => self
                    .ports
                    .all()
                    .iter()
                    .find(|p| &p.name == n)
                    .map(|p| PortFilter::One(p.id)),
            };
            if let Some(port) = port {
                map.add(l.mapping(port));
            }
        }
        if let Some(a) = armed {
            map.arm(a);
        }
        self.learn = map;
    }

    /// Once a period: tells the I/O view the ports plugged in, when each
    /// last received something and the clock arriving, and takes the
    /// project's port settings when they changed. Never waits for the
    /// screen; a period it can't update is counted ([`MusicIn::summary`]).
    pub fn sync(&mut self, model: &Shared, t: f64) {
        let mut m = match model.try_lock() {
            Ok(m) => m,
            Err(TryLockError::Poisoned(e)) => e.into_inner(),
            Err(TryLockError::WouldBlock) => {
                self.busy += 1;
                return;
            }
        };
        if !self.unsaved.is_empty() {
            for mut l in self.unsaved.drain(..) {
                // The transport, the tempo, the looper's footswitch and
                // Play's own controls work in every song; the rest
                // belongs to this one.
                let mut kept = if matches!(
                    l.target.owner,
                    Owner::Transport | Owner::Looper | Owner::Play
                ) {
                    Kept::Rig
                } else {
                    Kept::Project
                };
                // Learn again: the new control takes the old one's place,
                // with its range, how it takes over and where it is kept;
                // the way it works carries over when it is the same kind
                // of control (a key stays a button).
                if let Some((k, old)) = self.relearn.take().filter(|(_, o)| o.target == l.target) {
                    let list = match k {
                        Kept::Project => &mut m.setup.learn,
                        Kept::Rig => &mut m.setup.rig_learn,
                    };
                    list.retain(|o| *o != old);
                    let key = |s: &Saved| matches!(s.source, mpfree_midi::learn::Source::Note(_));
                    if key(&old) == key(&l) {
                        l.mode = old.mode;
                    }
                    (l.lo, l.hi, l.takeover) = (old.lo, old.hi, old.takeover);
                    kept = k;
                }
                if !m.setup.learned_in(kept, l) {
                    self.unkept += 1;
                }
            }
            self.armed_by = ArmedBy::Chain;
            m.gen += 1;
        }
        // The Learn section's Learn again, its transport buttons and
        // Cancel.
        if let Some(target) = m.asks.learn_target.take() {
            self.arm_from(target, ArmedBy::Io);
        }
        if let Some(k) = m.asks.learn_again.take() {
            if let Some((kept, l)) = m.setup.learned_at(k) {
                self.learn.arm(l.target);
                self.relearn = Some((kept, l.clone()));
                self.armed_by = ArmedBy::Io;
            }
        }
        if std::mem::take(&mut m.asks.cancel_learn) {
            self.disarm();
        }
        // MIDI Follow's Learn knobs and Done.
        match m.asks.follow_learn.take() {
            Some(true) => {
                self.knobs.learning = true;
                self.knobs.found = [None; FOLLOW_KNOBS];
            }
            Some(false) => self.knobs.learning = false,
            None => {}
        }
        if std::mem::take(&mut self.knobs.found_new) {
            m.setup.follow.knobs = self.knobs.found.iter().flatten().copied().collect();
            m.gen += 1;
        }
        let learning = self
            .knobs
            .learning
            .then(|| self.knobs.found.iter().flatten().count());
        if m.live.follow_learning != learning {
            m.live.follow_learning = learning;
        }
        if !self.moved.is_empty() {
            for landed in self.moved.drain(..) {
                m.setup.learn.retain_mut(|s| follow(s, &landed));
                m.setup.rig_learn.retain_mut(|s| follow(s, &landed));
            }
            // The learn map is built again from the list.
            m.gen += 1;
        }
        if self.applied != Some(m.gen) {
            for p in self.ports.all() {
                let set = m.setup.port(&p.name);
                if let Some(i) = self.input.get_mut(usize::from(p.id.0)) {
                    *i = set.input;
                }
                self.router.set_ignore(p.id, set.ignore);
                self.router.set_port_blocks(p.id, set.blocks);
                // Its MPE zone, as its gear's profile says (else what the
                // controller itself says).
                if let Some(x) = self.mpe.get_mut(usize::from(p.id.0)) {
                    x.set_zone(m.profile(&p.name).and_then(|f| f.mpe).unwrap_or_default());
                }
            }
            // Routes to tracks, on this run's port numbers: a route whose
            // port hasn't been plugged in yet waits for it (plugging a
            // port in applies the setup again).
            let ports = &self.ports;
            let routed = &mut self.routed;
            self.router.set_routes(
                m.setup.routes.iter().filter_map(|r| {
                    let id = ports.all().iter().find(|p| p.name == r.port)?.id;
                    Some(Route {
                        port: PortFilter::One(id),
                        channel: r.channel.map_or(ChanFilter::Omni, ChanFilter::One),
                        track: r.track,
                        blocks: r.blocks,
                    })
                }),
                0,
                |e| routed_key(e, routed),
            );
            // Thru, on this run's port numbers: one whose input or output
            // isn't plugged in yet waits for it.
            let (thru, dropped) = (&mut self.thru, &mut self.thru_dropped);
            self.router.set_thru(
                m.setup.thru.iter().filter_map(|r| {
                    let all = ports.all();
                    let from = all.iter().find(|p| p.name == r.from && p.has_in)?.id;
                    let to = all.iter().find(|p| p.name == r.to && p.has_out)?.id;
                    Some(Thru {
                        port: PortFilter::One(from),
                        channel: r.channel.map_or(ChanFilter::Omni, ChanFilter::One),
                        to,
                        blocks: r.blocks,
                    })
                }),
                0,
                |e| passed(e, thru, dropped),
            );
            // The song's learned controls, then the rig's it doesn't
            // override.
            self.build_learn(&m.setup.in_use());
            // "Tempo from" a port: follow its clock, by name, so a USB
            // device plugged in later is found.
            let leader = match &m.setup.tempo {
                TempoSource::Internal => None,
                TempoSource::Port(n) => Some(n),
            };
            self.follow = leader.map(|_| Follow {
                start_stop: m.setup.follow_start_stop,
                position: m.setup.follow_position,
            });
            let was = self.leader;
            self.leader =
                leader.and_then(|n| self.ports.all().iter().find(|p| &p.name == n).map(|p| p.id));
            if self.leader != was {
                self.leader_len = 0;
                self.leader_sp = 0;
            }
            // MIDI Follow: its controller by name, its knobs (its own,
            // else its profile's).
            let f = &m.setup.follow;
            self.knobs.port =
                f.on.then(|| {
                    self.ports
                        .all()
                        .iter()
                        .find(|p| p.name == f.port && p.has_in)
                        .map(|p| p.id)
                })
                .flatten();
            let ccs = f.knobs_of(m.profile(&f.port));
            let ccs: [Option<u8>; FOLLOW_KNOBS] = std::array::from_fn(|i| ccs.get(i).copied());
            self.knobs.at = [None; FOLLOW_KNOBS];
            // Send values back: its output (a USB device's is under the
            // same name), and every value again when it or the knobs
            // change.
            let back = (f.on && f.send_back)
                .then(|| {
                    self.ports
                        .all()
                        .iter()
                        .find(|p| p.name == f.port && p.has_out)
                        .map(|p| p.id)
                })
                .flatten();
            if back != self.knobs.back || ccs != self.knobs.ccs {
                self.knobs.sent = [None; FOLLOW_KNOBS];
            }
            self.knobs.back = back;
            self.knobs.ccs = ccs;
            // Controller mode: its output by name (on this run's number,
            // None until it is plugged in) and the profile that names its
            // notes and controls.
            let c = &m.setup.controller;
            let plan = c.on.then(|| crate::controller_out::Plan {
                setup: c.clone(),
                to: self
                    .ports
                    .all()
                    .iter()
                    .find(|p| p.name == c.port && p.has_out)
                    .map(|p| p.id),
                profile: m.profile(&c.port).cloned(),
                shown: m.setup.shown(&c.port).to_string(),
            });
            if plan != self.controller {
                self.controller = plan;
                self.controller_new = true;
            }
            // The app follows a clock now (M4).
            m.live.built.clock_in = true;
            self.applied = Some(m.gen);
            self.shown_names(&mut m);
        }
        if self.ports_changed {
            m.live.ports = self
                .ports
                .all()
                .iter()
                .filter(|p| p.connected)
                .map(|p| PortLive {
                    name: p.name.clone(),
                    kind: if p.id == DIN { "5-pin jacks" } else { "USB" }.to_string(),
                    about: if p.id == DIN {
                        "The MPC's 5-pin MIDI In and Out jacks".to_string()
                    } else {
                        self.about
                            .get(usize::from(p.id.0))
                            .filter(|a| !a.is_empty())
                            .cloned()
                            .unwrap_or_else(|| "USB".to_string())
                    },
                    has_in: p.has_in,
                    has_out: p.has_out,
                    // MIDI out says which it sends to (MidiOut::sync).
                    sending: false,
                    last_in: None,
                    clock_measured: None,
                })
                .collect();
            m.monitor.set_ports(self.ports.all());
            self.shown_names(&mut m);
            self.ports_changed = false;
        }
        // The monitor: what came and went since the last period.
        self.feed.drain_into(&mut m.monitor, t);
        m.monitor.set_epoch(t);
        m.live.built.monitor = true;
        m.live.debug = self.feed.debug;
        // The Monitor's "Send note-offs to MIDI out": the app sends them
        // (MidiOut::all_off, only the notes it left on); the monitor's out
        // notes go up now, so the view doesn't wait for them.
        if std::mem::take(&mut m.asks.note_offs) {
            m.monitor.out_notes_up();
            self.note_offs = true;
        }
        for lp in m.live.ports.iter_mut() {
            if let Some(p) = self.ports.all().iter().find(|p| p.name == lp.name) {
                lp.last_in = self.last_in[usize::from(p.id.0)].map(Duration::from_secs_f64);
            }
        }
        let now = (t.max(0.0) * f64::from(CLOCK_RATE)) as u64;
        let arriving = self
            .ports
            .all()
            .iter()
            .filter(|p| p.connected)
            .find_map(|p| {
                let i = usize::from(p.id.0);
                self.clock[i].state(now).bpm.map(|bpm| (i, bpm))
            });
        let key = arriving.map(|(i, bpm)| (i, (bpm * 10.0).round() as i64));
        if key != self.shown_clock {
            m.live.clock_in = arriving.and_then(|(i, bpm)| {
                self.ports
                    .get(PortId(i as u8))
                    .map(|p| (p.name.clone(), bpm as f32))
            });
            self.shown_clock = key;
        }
    }

    /// The name the log uses for a port.
    pub fn label(&self, port: PortId) -> String {
        if port == DIN {
            return "MIDI In".to_string();
        }
        self.ports
            .get(port)
            .map_or_else(|| format!("port {}", port.0), |p| p.name.clone())
    }

    /// A USB device's port was found (its input opened for reading, if it
    /// has one): its port number, or None when every port number is taken.
    pub fn plug(&mut self, p: &UsbPort) -> Option<PortId> {
        self.ports_changed = true;
        // Its input setting is the rig's, by name.
        self.applied = None;
        let id = self
            .ports
            .plugged(&p.name, p.input.is_some(), p.output.is_some())?;
        if let Some(a) = self.about.get_mut(usize::from(id.0)) {
            a.clone_from(&p.about);
        }
        Some(id)
    }

    /// A USB keyboard with one port, its input open (the tests' way).
    #[cfg(test)]
    pub fn plugged(&mut self, name: &str) -> Option<PortId> {
        self.plug(&UsbPort {
            name: name.to_string(),
            input: Some(0),
            output: None,
            about: "USB".to_string(),
        })
    }

    /// A USB keyboard went away: every key it held is let go, and every
    /// pedal it held comes up. Returns how many keys were let go.
    pub fn unplugged(&mut self, port: PortId, keys: &mut Vec<Key>) -> u16 {
        let held = keys.len();
        let routed = &mut self.routed;
        let routed_held = routed.len();
        let (thru, dropped) = (&mut self.thru, &mut self.thru_dropped);
        self.router.port_gone(port, 0, |e| {
            key_of(e, keys);
            routed_key(e, routed);
            passed(e, thru, dropped);
        });
        let routed_ended = self.routed[routed_held..]
            .iter()
            .filter(|(_, k)| matches!(k, Key::Up { .. }))
            .count();
        let ended = keys[held..]
            .iter()
            .filter(|k| matches!(k, Key::Up { .. }))
            .count()
            + routed_ended;
        // Its wheel let go too, and its pedals come up where they went
        // down.
        if self
            .mpe
            .get_mut(usize::from(port.0))
            .is_some_and(MpeIn::reset)
        {
            keys.push(Key::Bend { value: 0.0 });
        }
        let routed = &mut self.routed;
        self.pedals.retain(|&(p, to, ctl)| {
            if p != port {
                return true;
            }
            let up = Key::Control { ctl, value: 0.0 };
            match to {
                None => keys.push(up),
                Some(track) => routed.push((track, up)),
            }
            false
        });
        self.ports.unplugged(port);
        self.ports_changed = true;
        self.unplugged = true;
        if let Some(c) = self.clock.get_mut(usize::from(port.0)) {
            *c = ClockIn::new(CLOCK_RATE);
        }
        if let Some(s) = self.seen.get_mut(usize::from(port.0)) {
            s.clock_running = false;
            s.sysex = None;
        }
        u16::try_from(ended).unwrap_or(u16::MAX)
    }

    /// Whether a port went away since the last call: a learned control
    /// held down on it (a footswitch on the looper) will never come up,
    /// so the app lets go of what it holds (`remote::Remote::let_go`).
    pub fn take_unplugged(&mut self) -> bool {
        std::mem::take(&mut self.unplugged)
    }

    /// Bytes that came in on `port` at `t` seconds: the keys they play go
    /// to `keys`, and the log gets one line per message (clock and active
    /// sensing only counted).
    pub fn bytes(
        &mut self,
        port: PortId,
        data: &[u8],
        t: f64,
        keys: &mut Vec<Key>,
        log: &mut dyn FnMut(String),
    ) {
        let p = usize::from(port.0);
        if p >= MAX_PORTS {
            return;
        }
        // A port set to ignore clock leads nothing.
        let clock_ignored = self.router.ignores(port).has(Ignored::Clock);
        if self.leader == Some(port) && !clock_ignored {
            for &b in data {
                self.keep_leader_byte(b);
            }
        }
        // Borrowed, not built: a followed clock reads here every period.
        let label: std::borrow::Cow<str> = match self.ports.get(port) {
            _ if port == DIN => "MIDI In".into(),
            Some(p) => p.name.as_str().into(),
            None => self.label(port).into(),
        };
        let learn_only = self.input[p] == PortInput::LearnOnly;
        let learns = self.input[p] != PortInput::PlaysTracks;
        let frame = (t.max(0.0) * f64::from(CLOCK_RATE)) as u64;
        let armed = self.armed;
        let (keys_at, routed_at) = (keys.len(), self.routed.len());
        // What following this port takes: its clock, and its Start, Stop
        // and Song Position as the project says.
        let led = self
            .follow
            .filter(|_| self.leader == Some(port) && !clock_ignored);
        for &b in data {
            // The monitor's line, if this byte ends a message; where it
            // went is said below, once the message is routed.
            let listed = self.feed.bytes(port, Dir::In, t, &[b]);
            self.clock[p].byte(frame, b);
            let s = &mut self.seen[p];
            match b {
                0xF8 => {
                    s.clock_ticks += 1;
                    s.last_tick = t;
                    if !s.clock_running {
                        s.clock_running = true;
                        let follows = if clock_ignored {
                            "ignored: this port's setting"
                        } else if self.leader == Some(port) {
                            "MPFree follows it"
                        } else {
                            "not followed: the tempo isn't from this port"
                        };
                        log(format!(
                            "+{t:.3} s {label}: clock arriving (each tick is counted, not \
                             listed); {follows}"
                        ));
                    }
                }
                0xFE => {
                    s.sensing += 1;
                    if s.sensing == 1 {
                        log(format!(
                            "+{t:.3} s {label}: active sensing (counted, not listed)"
                        ));
                    }
                }
                0xFA | 0xFB | 0xFC | 0xFF => {
                    let what = match b {
                        0xFA => "start",
                        0xFB => "continue",
                        0xFC => "stop",
                        _ => "reset",
                    };
                    s.messages += 1;
                    log(format!("+{t:.3} s {label}: {b:02X} = {what}"));
                }
                0xF9 | 0xFD => {}
                0xF0 => s.sysex = Some(1),
                0xF7 => {
                    if let Some(n) = s.sysex.take() {
                        s.sysex_messages += 1;
                        s.messages += 1;
                        log(format!(
                            "+{t:.3} s {label}: SysEx, {} bytes = ignored",
                            n + 1
                        ));
                    }
                }
                0x80..=0xF6 => s.sysex = None,
                _ => {
                    if let Some(n) = s.sysex.as_mut() {
                        *n += 1;
                    }
                }
            }
            let mut seen = None;
            let mut got = false;
            let mut ignored = false;
            let mut learned = None;
            // Where a channel message went, for the monitor.
            let mut to = if learn_only {
                To::LearnOnly
            } else {
                To::Nothing
            };
            let (learn, learning, sets, routed) = (
                &mut self.learn,
                &mut self.learning,
                &mut self.learn_sets,
                &mut self.routed,
            );
            let (thru, dropped) = (&mut self.thru, &mut self.thru_dropped);
            let knobs = &mut self.knobs;
            // The port's wheel, channel pressure and MPE.
            let mpe = &mut self.mpe[p];
            let mut zone_said = None;
            // Learn or MIDI Follow took the message: it reaches nothing
            // else, however many routes and thru name its channel. They
            // hear it as it came, before any route's blocks.
            let mut took = false;
            self.router.feed(port, b, 0, |e| match e {
                In::Seen { msg, .. } => seen = Some(msg),
                // The port's "Ignore coming in": listed, nothing else.
                In::Ignored { .. } => {
                    got = true;
                    ignored = true;
                    to = To::PortIgnores;
                }
                other => {
                    got = true;
                    // What Learn takes doesn't play, or go thru.
                    if let In::Track { msg, .. } | In::Routed { msg, .. } | In::Thru { msg, .. } =
                        &other
                    {
                        if took {
                            return;
                        }
                        let raw = seen.unwrap_or(*msg);
                        if learn_or_follow(
                            learn,
                            knobs,
                            learning,
                            sets,
                            port,
                            &raw,
                            t,
                            learns,
                            &mut to,
                            &mut learned,
                        ) {
                            took = true;
                            return;
                        }
                    }
                    // Thru sends it on as it came, whatever the port's
                    // input (MIDI out sends it if it is on).
                    if let In::Thru { to: out, .. } = other {
                        to = To::Thru { to: out };
                        passed(other, thru, dropped);
                        return;
                    }
                    // A route plays its track, whatever the port's input:
                    // its notes, pedals and wheels. Its other CCs (a knob)
                    // nothing takes yet.
                    if let In::Routed { track, msg, .. } = other {
                        to = if key(msg).is_some() {
                            To::Routed { track }
                        } else {
                            To::Nothing
                        };
                        routed_key(other, routed);
                        // Its bend and pressure (after its note, which
                        // they may come with).
                        match mpe.msg(msg, &mut |e| routed.push((track, expr_key(e)))) {
                            Took::Expr => to = To::Routed { track },
                            Took::Zone(z) => zone_said = Some(z),
                            Took::No => {}
                        }
                        return;
                    }
                    // "Learn only" plays nothing, but a note-off still
                    // ends a note this port started before.
                    if !(learn_only && starts_or_presses(&other)) {
                        if !learn_only {
                            if let Some(note) = key_note(&other) {
                                to = armed_to(armed, note);
                            } else if moves_track(&other) && armed.1 == KeysPlay::Notes {
                                to = To::Track {
                                    track: armed.0,
                                    pad: None,
                                };
                            }
                        }
                        key_of(other, keys);
                        // Its bend and pressure, on the armed track (after
                        // its note, which they may come with).
                        if let (false, In::Track { msg, .. }) = (learn_only, other) {
                            match mpe.msg(msg, &mut |e| keys.push(expr_key(e))) {
                                Took::Expr if armed.1 == KeysPlay::Notes => {
                                    to = To::Track {
                                        track: armed.0,
                                        pad: None,
                                    }
                                }
                                Took::Zone(z) => zone_said = Some(z),
                                _ => {}
                            }
                        }
                    }
                }
            });
            if let Some(z) = zone_said {
                log(format!("+{t:.3} s {label}: {} (it said so)", z.words()));
            }
            // Nothing played it (a route's blocks dropped it, or no track
            // takes it): Learn and MIDI Follow still hear it.
            if let (Some(raw), false) = (seen, got) {
                learn_or_follow(
                    &mut self.learn,
                    &mut self.knobs,
                    &mut self.learning,
                    &mut self.learn_sets,
                    port,
                    &raw,
                    t,
                    learns,
                    &mut to,
                    &mut learned,
                );
            }
            if listed {
                let to = match self.feed.listed() {
                    Some(What::Channel { .. }) => match learned.and_then(|i| self.learn.get(i)) {
                        Some(m) => To::Took(m.target),
                        None => to,
                    },
                    Some(
                        What::Arriving(Steady::Clock)
                        | What::Start
                        | What::Continue
                        | What::Stop
                        | What::SongPosition(_),
                    ) if clock_ignored => To::PortIgnores,
                    Some(What::Arriving(Steady::Clock)) => follows(led.is_some()),
                    Some(What::Start | What::Continue | What::Stop) => {
                        follows(led.is_some_and(|f| f.start_stop))
                    }
                    Some(What::SongPosition(_)) => follows(led.is_some_and(|f| f.position)),
                    _ => To::Ignored,
                };
                self.feed.to(to);
            }
            // A key learned on a switch you flip (PLAY and STOP, a mute,
            // a solo) flips it on each press, before the key comes up; a
            // looper press works while held ([`learned_mode`]).
            if let Some(i) = learned {
                let mode = self
                    .learn
                    .get(i)
                    .and_then(|m| learned_mode(&m.control.source, m.target));
                if let Some(mode) = mode {
                    self.learn.set_mode(i, mode);
                }
            }
            if let Some(m) = learned.and_then(|i| self.learn.get(i).copied()) {
                let name = self.ports.get(port).map(|p| p.name.clone());
                let control =
                    mpfree_midi_ui::control_text(&m.control.source, m.control.channel, &label);
                log(format!(
                    "+{t:.3} s {label}: MIDI Learn: {control} takes {}",
                    m.target
                ));
                self.unsaved.push(Saved::of(&m, name));
                // The screen says the port by the name given to it.
                let control = match self.shown.iter().find(|(id, _)| *id == port) {
                    Some((_, n)) => {
                        mpfree_midi_ui::control_text(&m.control.source, m.control.channel, n)
                    }
                    None => control,
                };
                self.learning.push(Learning::Took {
                    target: m.target,
                    control,
                });
            }
            if got || seen.is_some() {
                self.last_in[p] = Some(t);
            }
            if let Some(msg) = seen {
                self.seen[p].messages += 1;
                let (bytes, n) = msg.encode();
                log(format!(
                    "+{t:.3} s {label}: {} = {}{}",
                    hex(&bytes[..n]),
                    describe(&msg),
                    if ignored {
                        " (ignored: this port's setting)"
                    } else {
                        ""
                    }
                ));
            }
        }
        // The pedals it holds down now, and where they went.
        for &k in &keys[keys_at..] {
            hold_pedal(&mut self.pedals, port, None, k);
        }
        for &(track, k) in &self.routed[routed_at..] {
            hold_pedal(&mut self.pedals, port, Some(track), k);
        }
    }

    /// Once in a while: say when a port's clock stopped arriving.
    pub fn tick(&mut self, t: f64, log: &mut dyn FnMut(String)) {
        for p in 0..MAX_PORTS {
            let s = &mut self.seen[p];
            if s.clock_running && t - s.last_tick > CLOCK_GAP_S {
                s.clock_running = false;
                let ticks = s.clock_ticks;
                let label = self.label(PortId(p as u8));
                log(format!(
                    "+{t:.3} s {label}: clock stopped arriving ({ticks} ticks so far)"
                ));
            }
        }
    }

    /// One line per port that saw anything, for the report at the end.
    pub fn summary(&self) -> Vec<String> {
        let mut lines = Vec::new();
        for p in 0..MAX_PORTS {
            let s = &self.seen[p];
            if s.messages + s.clock_ticks + s.sensing == 0 {
                continue;
            }
            lines.push(format!(
                "{}: {} messages ({} SysEx, ignored), {} clock ticks, {} active sensing",
                self.label(PortId(p as u8)),
                s.messages,
                s.sysex_messages,
                s.clock_ticks,
                s.sensing
            ));
        }
        if self.busy > 0 {
            lines.push(format!(
                "I/O view: {} periods skipped its update because the screen held it",
                self.busy
            ));
        }
        if self.leader_dropped > 0 {
            lines.push(format!(
                "Clock in: {} bytes from the followed port dropped (more than {LEADER_BYTES} in a period)",
                self.leader_dropped
            ));
        }
        if self.unkept > 0 {
            lines.push(format!(
                "MIDI Learn: {} learned controls not kept (the project has {} already)",
                self.unkept,
                mpfree_midi::setup::MAX_LEARNED
            ));
        }
        if self.thru_dropped > 0 {
            lines.push(format!(
                "Thru: {} messages dropped (more than {THRU_MSGS} in a period)",
                self.thru_dropped
            ));
        }
        let st = self.router.stats;
        if st.unrouted + st.bad_port > 0 {
            lines.push(format!(
                "MIDI in: {} messages no track took, {} for a port past the table",
                st.unrouted, st.bad_port
            ));
        }
        lines
    }
}

/// The model the mixer's I/O view and [`MusicIn::sync`] share, with the
/// project's 16 tracks named as the time engine names them.
pub fn io_model() -> Shared {
    let mut m = mpfree_midi_ui::IoModel::default();
    m.live.tracks = (1..=mpfree_midi::output::MAX_TRACKS)
        .map(|i| format!("Track {i}"))
        .collect();
    std::sync::Arc::new(std::sync::Mutex::new(m))
}

/// The note a key message plays (note on, note off, pressure).
/// Whether `e` is a pedal, the mod wheel, breath or a foot controller for
/// the armed track ([`Key::Control`]).
fn moves_track(e: &In) -> bool {
    matches!(e, In::Track { msg: Msg::Cc { cc, .. }, .. } if Ctl::from_cc(*cc).is_some())
}

fn key_note(e: &In) -> Option<u8> {
    match e {
        In::Track {
            msg:
                Msg::NoteOn { note, .. } | Msg::NoteOff { note, .. } | Msg::PolyPressure { note, .. },
            ..
        } => Some(*note),
        _ => None,
    }
}

/// How keys play the armed track (main.rs's `keys_play`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KeysPlay {
    /// By note: the synth's pad layout is on the pads.
    #[default]
    Notes,
    /// A kit: each note its kit pad, A1 to H16
    /// (`mpfree_midi::profile::kit_pad_of_note`).
    KitPads,
    /// As the pad (1 to 16) the MPC gives each note
    /// (`mpc_one::pad_index`): the synth or a track's own instrument
    /// without a pad layout.
    Pads,
}

/// Where a key went, for the monitor: the armed track, by note or as the
/// pad its note plays.
fn armed_to((track, play): (u8, KeysPlay), note: u8) -> To {
    let pad = match play {
        KeysPlay::Notes => return To::Track { track, pad: None },
        KeysPlay::KitPads => Some(mpfree_midi::profile::kit_pad_of_note(note) as u8),
        KeysPlay::Pads => mpfree_controller::mpc_one::pad_index(note),
    };
    match pad {
        Some(pad) => To::Track {
            track,
            pad: Some(pad),
        },
        None => To::NoPad { track },
    }
}

/// Clock, start, stop or song position: followed or not.
fn follows(yes: bool) -> To {
    if yes {
        To::Follows
    } else {
        To::NotFollowed
    }
}

/// A note-on or pressure: what a "Learn only" port doesn't play.
fn starts_or_presses(e: &In) -> bool {
    match e {
        In::Track {
            msg: Msg::NoteOn { .. } | Msg::PolyPressure { .. },
            ..
        } => true,
        // A pedal coming up still lets go of what it held; anything else
        // a controller moves plays.
        In::Track {
            msg: Msg::Cc { cc, value, .. },
            ..
        } => !Ctl::from_cc(*cc).is_some_and(|c| c.is_pedal() && *value < 64),
        _ => false,
    }
}

/// The router's notes, pedals, mod wheel, breath and foot controller as
/// keys; other CCs and program changes wait for MIDI Learn and the track
/// settings, and bends and pressure go through MPE ([`MpeIn`]).
fn key_of(e: In, keys: &mut Vec<Key>) {
    let In::Track { msg, .. } = e else {
        return;
    };
    if let Some(k) = key(msg) {
        keys.push(k);
    }
}

/// What thru passes on, kept for the app; past [`THRU_MSGS`] dropped and
/// counted.
fn passed(e: In, thru: &mut Vec<Passed>, dropped: &mut u64) {
    let In::Thru { from, to, msg, .. } = e else {
        return;
    };
    if thru.len() < THRU_MSGS {
        thru.push(Passed { from, to, msg });
    } else {
        *dropped += 1;
    }
}

/// A route's notes and controllers as keys, with the track they play.
fn routed_key(e: In, routed: &mut Vec<(u8, Key)>) {
    let In::Routed { track, msg, .. } = e else {
        return;
    };
    if let Some(k) = key(msg) {
        routed.push((track, k));
    }
}

fn key(msg: Msg) -> Option<Key> {
    Some(match msg {
        Msg::NoteOn { note, vel, .. } => Key::Down {
            note,
            velocity: vel,
        },
        Msg::NoteOff { note, .. } => Key::Up { note },
        Msg::PolyPressure { note, value, .. } => Key::Pressure { note, value },
        Msg::Cc { cc, value, .. } => Key::Control {
            ctl: Ctl::from_cc(cc)?,
            value: f32::from(value) / 127.0,
        },
        _ => return None,
    })
}

/// Where a pedal a port holds down went: the armed track's keys (None)
/// or a route's track.
type PedalDown = (PortId, Option<u8>, Ctl);

/// Keeps the pedals each port holds down, and where they went, from a key
/// that port played.
fn hold_pedal(down: &mut Vec<PedalDown>, port: PortId, to: Option<u8>, key: Key) {
    let Key::Control { ctl, value } = key else {
        return;
    };
    if !ctl.is_pedal() {
        return;
    }
    let at = (port, to, ctl);
    match (Ctl::down(value), down.iter().position(|d| *d == at)) {
        (true, None) => down.push(at),
        (false, Some(i)) => {
            down.swap_remove(i);
        }
        _ => {}
    }
}

/// The pedals down on the armed track from the keys, so that they come up
/// there when another track is armed and nothing they hold hangs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PedalsDown(u8);

impl PedalsDown {
    /// A pedal (or another controller, which it ignores) went to the
    /// armed track.
    pub fn key(&mut self, ctl: Ctl, value: f32) {
        let Some(i) = Ctl::ALL.iter().position(|&c| c == ctl && c.is_pedal()) else {
            return;
        };
        if Ctl::down(value) {
            self.0 |= 1 << i;
        } else {
            self.0 &= !(1 << i);
        }
    }

    /// The pedals that were down, now up.
    pub fn lift(&mut self) -> impl Iterator<Item = Ctl> {
        let down = std::mem::take(&mut self.0);
        Ctl::ALL
            .into_iter()
            .enumerate()
            .filter(move |(i, _)| down & (1 << i) != 0)
            .map(|(_, c)| c)
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// A message in words, channels counted from 1 and notes named as the
/// Synth tab names them (48 is C3).
pub fn describe(msg: &Msg) -> String {
    let name = |n: u8| crate::synth::note_name(n);
    match *msg {
        Msg::NoteOn { ch, note, vel } => {
            format!("note on · ch {} · {} · vel {vel}", ch + 1, name(note))
        }
        Msg::NoteOff { ch, note, .. } => format!("note off · ch {} · {}", ch + 1, name(note)),
        Msg::PolyPressure { ch, note, value } => {
            format!("pressure · ch {} · {} · {value}", ch + 1, name(note))
        }
        Msg::Cc { ch, cc, value } => format!("CC {cc} · ch {} · {value}", ch + 1),
        Msg::Program { ch, program } => format!("program {} · ch {}", program + 1, ch + 1),
        Msg::ChanPressure { ch, value } => format!("channel pressure · ch {} · {value}", ch + 1),
        Msg::PitchBend { ch, value } => format!("pitch bend · ch {} · {value:+}", ch + 1),
    }
}

/// One port of a USB MIDI device, as the scanner names it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsbPort {
    /// The name a project uses for it: the device's own name when it has
    /// one port, else each port's own name ("MIO4 MIDI 2").
    pub name: String,
    /// The input substream to read, if it has an input.
    pub input: Option<u32>,
    /// The output substream MIDI out sends to, if it has an output.
    pub output: Option<u32>,
    /// What the port's page says about it ("USB · port 2 of 4 on MIO4").
    pub about: String,
}

/// The ports of one USB MIDI card named `card` (its description's name):
/// one for each input substream, named by `subnames` (ALSA's names for
/// them, in order), paired with the output of the same number, then one
/// for each output past the inputs (`outputs` in all). A device with one
/// port keeps the device's name, as before devices with several ports
/// were read; one with several gets each port's own name, or "card N"
/// when ALSA gives none or gives one twice.
pub fn usb_ports(card: &str, subnames: &[String], outputs: usize) -> Vec<UsbPort> {
    let n = subnames.len().max(outputs);
    let mut out: Vec<UsbPort> = Vec::with_capacity(n);
    for k in 0..n {
        let name = if n == 1 {
            card.to_string()
        } else {
            let own = subnames.get(k).map(|s| s.trim()).unwrap_or("");
            if own.is_empty() || out.iter().any(|p| p.name == own) || own == card {
                format!("{card} {}", k + 1)
            } else {
                own.to_string()
            }
        };
        let about = if n == 1 {
            "USB".to_string()
        } else {
            format!("USB · port {} of {n} on {card}", k + 1)
        };
        out.push(UsbPort {
            name,
            input: (k < subnames.len()).then_some(k as u32),
            output: (k < outputs).then_some(k as u32),
            about,
        });
    }
    out
}

/// How many outputs a card's first MIDI device has, from its
/// `/proc/asound/cardN/midi0` (one "Output N" block for each). Read only:
/// nothing is opened.
pub fn proc_midi_outputs(text: &str) -> usize {
    text.lines()
        .filter(|l| {
            l.strip_prefix("Output ")
                .is_some_and(|n| n.trim().parse::<u32>().is_ok())
        })
        .count()
}

/// USB-MIDI cards to listen to: every card in `/proc/asound/cards` (as
/// `(number, id, description)`) that has a MIDI device and is not one of
/// `skip` (the MPC's controller and its sound card), with the name it
/// gives itself.
pub fn usb_midi_cards(
    cards: &[(u32, String, String)],
    skip: &[u32],
    has_midi: impl Fn(u32) -> bool,
) -> Vec<(u32, String)> {
    cards
        .iter()
        .filter(|(n, _, _)| !skip.contains(n) && has_midi(*n))
        .map(|(n, id, desc)| {
            // "USB-Audio - Keystation 49" names the device; the id is the
            // short form ("Keystation").
            let name = desc.split(" - ").nth(1).unwrap_or(desc).trim();
            let name = if name.is_empty() { id.as_str() } else { name };
            (*n, name.to_string())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(m: &mut MusicIn, port: PortId, data: &[u8]) -> (Vec<Key>, Vec<String>) {
        let mut keys = Vec::new();
        let mut lines = Vec::new();
        m.bytes(port, data, 1.0, &mut keys, &mut |l| lines.push(l));
        (keys, lines)
    }

    fn model() -> Shared {
        std::sync::Arc::new(std::sync::Mutex::new(mpfree_midi_ui::IoModel::default()))
    }

    #[test]
    fn the_io_view_sees_ports_lights_and_clock() {
        let mut m = MusicIn::new();
        let io = model();
        m.sync(&io, 0.5);
        {
            let v = io.lock().unwrap();
            let names: Vec<&str> = v.live.ports.iter().map(|p| p.name.as_str()).collect();
            assert_eq!(names, [mpfree_midi::setup::DIN_NAME]);
            assert_eq!(v.live.ports[0].kind, "5-pin jacks");
            assert_eq!(v.live.ports[0].last_in, None);
            assert_eq!(v.live.clock_in, None);
        }
        let usb = m.plugged("Keystep 37").unwrap();
        // A note on the 5-pin jacks at 1 s lights it.
        run(&mut m, DIN, &[0x90, 60, 100]);
        // Clock at 120 BPM on the USB keyboard: a pulse every 1/48 s.
        let mut keys = Vec::new();
        for i in 0..30 {
            let t = 1.0 + i as f64 / 48.0;
            m.bytes(usb, &[0xF8], t, &mut keys, &mut |_| {});
        }
        m.sync(&io, 1.62);
        let v = io.lock().unwrap();
        assert_eq!(v.live.ports.len(), 2);
        assert_eq!(v.live.ports[1].kind, "USB");
        assert!(!v.live.ports[1].has_out, "this keyboard has no output");
        assert_eq!(v.live.ports[0].last_in, Some(Duration::from_secs(1)));
        assert_eq!(v.live.ports[1].last_in, None, "clock lights nothing");
        let (port, bpm) = v.live.clock_in.clone().unwrap();
        assert_eq!(port, "Keystep 37");
        assert!((bpm - 120.0).abs() < 0.1, "{bpm}");
    }

    #[test]
    fn a_learn_only_port_plays_nothing_but_lets_go() {
        let mut m = MusicIn::new();
        let io = model();
        // A key held before the setting changes.
        let (keys, _) = run(&mut m, DIN, &[0x90, 60, 100]);
        assert_eq!(keys.len(), 1);
        {
            let mut v = io.lock().unwrap();
            v.setup.set_port(mpfree_midi::setup::DIN_NAME, |p| {
                p.input = PortInput::LearnOnly
            });
            v.gen += 1;
        }
        m.sync(&io, 1.0);
        let (keys, lines) = run(&mut m, DIN, &[0x90, 62, 100, 0xA0, 62, 30, 0x80, 60, 0]);
        assert_eq!(keys, vec![Key::Up { note: 60 }]);
        // Still logged.
        assert_eq!(lines.len(), 3);
    }

    /// Thru sends a keyboard's channel 10 to the 5-pin jacks as it came,
    /// and the rest of its keys play the armed track; a control Learn has
    /// on that channel moves its value instead of going thru, and taking
    /// the thru away while a key is down ends that note at the jacks.
    #[test]
    fn thru_sends_a_channel_on_as_it_came_and_learn_keeps_what_it_learned() {
        use mpfree_midi::setup::{ThruRoute, DIN_NAME};
        let mut m = MusicIn::new();
        let io = model();
        {
            let mut v = io.lock().unwrap();
            v.setup.thru = vec![ThruRoute {
                from: "Keystep 37".to_string(),
                channel: Some(9),
                to: DIN_NAME.to_string(),
                blocks: mpfree_midi::blocks::Blocks::NONE,
            }];
            let cutoff =
                Saved::parse("port Keystep 37 | ch 10 | cc 74 | t1.fx1.0 | 0 1 | absolute | jump")
                    .expect("a learned control");
            assert!(v.setup.learned(cutoff));
            v.gen += 1;
        }
        let keystep = m.plugged("Keystep 37").unwrap();
        m.sync(&io, 0.5);
        let (keys, _) = run(
            &mut m,
            keystep,
            &[0x99, 36, 100, 0xB9, 74, 64, 0xB9, 1, 10, 0x90, 60, 100],
        );
        assert_eq!(
            keys,
            vec![Key::Down {
                note: 60,
                velocity: 100
            }]
        );
        let passed = |m: &mut MusicIn| -> Vec<(PortId, PortId, Msg)> {
            m.take_thru().map(|p| (p.from, p.to, p.msg)).collect()
        };
        assert_eq!(
            passed(&mut m),
            vec![
                (
                    keystep,
                    DIN,
                    Msg::NoteOn {
                        ch: 9,
                        note: 36,
                        vel: 100
                    }
                ),
                (
                    keystep,
                    DIN,
                    Msg::Cc {
                        ch: 9,
                        cc: 1,
                        value: 10
                    }
                ),
            ]
        );
        assert!(m
            .learning()
            .any(|l| matches!(l, Learning::Set(w) if w.target.to_string() == "t1.fx1.0")));
        m.sync(&io, 0.75);
        {
            let v = io.lock().unwrap();
            let went: Vec<To> = v.monitor.newest().map(|(_, l)| l.to).collect();
            assert!(went.contains(&To::Thru { to: DIN }), "{went:?}");
        }
        // The thru taken away with 36 still down: its note-off goes to
        // the jacks now, and the key's own note-off later goes nowhere.
        {
            let mut v = io.lock().unwrap();
            v.setup.thru.clear();
            v.gen += 1;
        }
        m.sync(&io, 1.0);
        assert_eq!(
            passed(&mut m),
            vec![(
                keystep,
                DIN,
                Msg::NoteOff {
                    ch: 9,
                    note: 36,
                    vel: 64
                }
            )]
        );
        let (keys, _) = run(&mut m, keystep, &[0x89, 36, 0]);
        assert!(keys.is_empty());
        assert!(passed(&mut m).is_empty());
    }

    /// A route's blocks shape what it plays and its Split leaves the rest
    /// of the keys to the armed track; the port's own blocks shape the
    /// armed track's keys; Learn hears a knob as it came, even one the
    /// port's Filter drops.
    #[test]
    fn blocks_shape_a_route_and_the_armed_track_and_learn_hears_what_came() {
        use mpfree_midi::blocks::Blocks;
        use mpfree_midi::setup::{TrackRoute, DIN_NAME};
        let mut m = MusicIn::new();
        let io = model();
        {
            let mut v = io.lock().unwrap();
            v.setup.routes = vec![TrackRoute {
                port: DIN_NAME.to_string(),
                channel: Some(9),
                track: 40,
                blocks: Blocks::parse("split 36 47; transpose 12").unwrap(),
            }];
            v.setup.set_port(DIN_NAME, |p| {
                p.blocks = Blocks::parse("filter controls program; velocity fixed 100").unwrap();
            });
            v.gen += 1;
        }
        m.sync(&io, 0.5);
        // In the split: track 41, an octave up. Outside it: the armed
        // track, at the port's fixed velocity.
        let (keys, _) = run(&mut m, DIN, &[0x99, 36, 64, 0x99, 60, 30]);
        assert_eq!(
            m.take_routed().collect::<Vec<_>>(),
            vec![(
                40,
                Key::Down {
                    note: 48,
                    velocity: 64
                }
            )]
        );
        assert_eq!(
            keys,
            vec![Key::Down {
                note: 60,
                velocity: 100
            }]
        );
        let (keys, _) = run(&mut m, DIN, &[0x89, 36, 0, 0x89, 60, 0]);
        assert_eq!(
            m.take_routed().collect::<Vec<_>>(),
            vec![(40, Key::Up { note: 48 })]
        );
        assert_eq!(keys, vec![Key::Up { note: 60 }]);
        // A knob the port's Filter drops still reaches Learn, as it came.
        m.arm_from(cutoff(), ArmedBy::Touch);
        let (keys, _) = run(&mut m, DIN, &[0xB0, 74, 64]);
        assert!(keys.is_empty());
        let got: Vec<_> = m.learning().collect();
        assert!(
            matches!(got[..], [Learning::Took { target, .. }] if target == cutoff()),
            "{got:?}"
        );
        m.sync(&io, 0.75);
        let v = io.lock().unwrap();
        assert_eq!(v.setup.learn.len(), 1);
        assert_eq!(v.setup.learn[0].source, mpfree_midi::learn::Source::Cc(74));
    }

    #[test]
    fn a_route_plays_its_track_whatever_the_ports_input_and_the_rest_play_the_armed_one() {
        use mpfree_midi::setup::{TrackRoute, DIN_NAME};
        let mut m = MusicIn::new();
        let io = model();
        {
            let mut v = io.lock().unwrap();
            // Channel 10 of the 5-pin jack plays track 41; a route from a
            // keyboard that isn't plugged in yet waits for it.
            v.setup.routes = vec![
                TrackRoute {
                    port: DIN_NAME.to_string(),
                    channel: Some(9),
                    track: 40,
                    blocks: mpfree_midi::blocks::Blocks::NONE,
                },
                TrackRoute {
                    port: "Keystep 37".to_string(),
                    channel: None,
                    track: 2,
                    blocks: mpfree_midi::blocks::Blocks::NONE,
                },
            ];
            v.setup
                .set_port(DIN_NAME, |p| p.input = PortInput::LearnOnly);
            v.gen += 1;
        }
        m.sync(&io, 0.5);
        // A Learn only port plays nothing on the armed track, but its
        // route still plays track 41.
        let (keys, _) = run(&mut m, DIN, &[0x99, 36, 100, 0x90, 60, 100]);
        assert!(keys.is_empty(), "{keys:?}");
        let routed: Vec<_> = m.take_routed().collect();
        assert_eq!(
            routed,
            vec![(
                40,
                Key::Down {
                    note: 36,
                    velocity: 100
                }
            )]
        );
        // The monitor says the route played track 41, not the armed one.
        m.sync(&io, 0.75);
        {
            let v = io.lock().unwrap();
            let went: Vec<To> = v.monitor.newest().map(|(_, l)| l.to).collect();
            assert!(went.contains(&To::Routed { track: 40 }), "{went:?}");
            assert!(went.contains(&To::LearnOnly), "{went:?}");
        }
        // The keyboard's route applies once it is plugged in; its keys
        // go to track 3, none to the armed track.
        let keystep = m.plugged("Keystep 37").unwrap();
        m.sync(&io, 1.0);
        let (keys, _) = run(&mut m, keystep, &[0x93, 64, 90]);
        assert!(keys.is_empty());
        assert_eq!(
            m.take_routed().collect::<Vec<_>>(),
            vec![(
                2,
                Key::Down {
                    note: 64,
                    velocity: 90
                }
            )]
        );
        // Unplugged while held: the key is let go on its track.
        let mut keys = Vec::new();
        assert_eq!(m.unplugged(keystep, &mut keys), 1);
        // The app hears of it once, so a learned footswitch held on it
        // lets go (remote::Remote::let_go).
        assert!(m.take_unplugged());
        assert!(!m.take_unplugged());
        assert_eq!(
            m.take_routed().collect::<Vec<_>>(),
            vec![(2, Key::Up { note: 64 })]
        );
        // A route taken away lets its held keys go on the track they
        // played (36 is still held from above).
        run(&mut m, DIN, &[0x99, 38, 100]);
        m.take_routed().for_each(drop);
        {
            let mut v = io.lock().unwrap();
            v.setup.routes.clear();
            v.gen += 1;
        }
        m.sync(&io, 2.0);
        assert_eq!(
            m.take_routed().collect::<Vec<_>>(),
            vec![(40, Key::Up { note: 36 }), (40, Key::Up { note: 38 })]
        );
    }

    #[test]
    fn a_routed_keys_note_off_follows_its_note_on_when_the_armed_track_changes() {
        let mut h = RoutedHeld::new();
        let down = Key::Down {
            note: 60,
            velocity: 100,
        };
        let up = Key::Up { note: 60 };
        // Down while its track isn't armed: played at once; the track is
        // armed before the key comes up: it still ends where it began.
        assert!(h.direct(5, down, false));
        assert!(h.direct(5, Key::Pressure { note: 60, value: 9 }, true));
        assert!(h.direct(5, up, true));
        // Down while armed: with the armed keys, and so its note-off.
        assert!(!h.direct(5, down, true));
        assert!(!h.direct(5, up, true));
        // Armed at the key's down, not at its up: the armed keys let it
        // go when the armed track changed, so a note-off here is harmless.
        assert!(!h.direct(5, down, true));
        assert!(h.direct(5, up, false));
        // A track past the most a route names goes with the armed keys.
        assert!(!h.direct(200, down, false));
    }

    #[test]
    fn capture_keeps_a_routed_key_on_the_pad_its_note_went_to() {
        let mut r = RoutedPads::new();
        let down = |note| Key::Down { note, velocity: 0 };
        // Note 60 plays pad 3 on track 5, then the pads change while it is
        // held: its pressure and end still go to pad 3.
        let ev = r.event(5, down(60), |n| (n == 60).then_some(3));
        assert_eq!(
            ev,
            Some(Event::NoteOn {
                note: 3,
                velocity: 1
            })
        );
        let pressure = Key::Pressure {
            note: 60,
            value: 70,
        };
        assert_eq!(
            r.event(5, pressure, |_| Some(9)),
            Some(Event::Pressure { note: 3, value: 70 })
        );
        assert_eq!(
            r.event(5, Key::Up { note: 60 }, |_| Some(9)),
            Some(Event::NoteOff { note: 3 })
        );
        // Ended: nothing more for it.
        assert_eq!(r.event(5, Key::Up { note: 60 }, |_| Some(9)), None);
        assert_eq!(r.event(5, pressure, |_| Some(9)), None);
        // The same note on another track is its own.
        assert!(r.event(6, down(60), |_| Some(4)).is_some());
        assert_eq!(r.event(5, Key::Up { note: 60 }, |_| None), None);
        // A note no pad plays alone isn't kept, nor its end; nor the wheel,
        // nor a track past the most a route names.
        assert_eq!(r.event(5, down(61), |_| None), None);
        assert_eq!(r.event(5, Key::Up { note: 61 }, |_| Some(1)), None);
        assert_eq!(r.event(5, Key::Bend { value: 0.5 }, |_| Some(1)), None);
        assert_eq!(r.event(200, down(60), |_| Some(1)), None);
    }

    /// A routed key held when its keyboard is unplugged, or when its route
    /// is taken away, ends in CAPTURE's history too: the key's end comes
    /// from the router, through `RoutedHeld` and `RoutedPads`, as the
    /// end of the pad its note went to, so no routed note is left held.
    #[test]
    fn a_routed_key_let_go_by_an_unplug_or_a_route_change_ends_for_capture() {
        use mpfree_midi::setup::TrackRoute;
        let mut m = MusicIn::new();
        let io = model();
        let keystep = m.plugged("Keystep 37").unwrap();
        {
            let mut v = io.lock().unwrap();
            v.setup.routes = vec![TrackRoute {
                port: "Keystep 37".to_string(),
                channel: None,
                track: 2,
                blocks: mpfree_midi::blocks::Blocks::NONE,
            }];
            v.gen += 1;
        }
        m.sync(&io, 0.5);
        let mut held = RoutedHeld::new();
        let mut pads = RoutedPads::new();
        // What the app gives CAPTURE for the routed keys so far (track 3
        // isn't armed; note n plays pad n - 60).
        let mut capture = |m: &mut MusicIn| -> Vec<(u8, Event)> {
            let routed: Vec<_> = m.take_routed().collect();
            routed
                .into_iter()
                .filter(|&(t, k)| held.direct(t, k, false))
                .filter_map(|(t, k)| Some((t, pads.event(t, k, |n| n.checked_sub(60))?)))
                .collect()
        };
        run(&mut m, keystep, &[0x90, 64, 90]);
        assert_eq!(
            capture(&mut m),
            vec![(
                2,
                Event::NoteOn {
                    note: 4,
                    velocity: 90
                }
            )]
        );
        m.unplugged(keystep, &mut Vec::new());
        assert_eq!(capture(&mut m), vec![(2, Event::NoteOff { note: 4 })]);
        // Back in, a key held, and the route taken away.
        let keystep = m.plugged("Keystep 37").unwrap();
        m.sync(&io, 1.0);
        run(&mut m, keystep, &[0x90, 67, 90]);
        assert_eq!(capture(&mut m).len(), 1);
        {
            let mut v = io.lock().unwrap();
            v.setup.routes.clear();
            v.gen += 1;
        }
        m.sync(&io, 1.5);
        assert_eq!(capture(&mut m), vec![(2, Event::NoteOff { note: 7 })]);
    }

    #[test]
    fn a_busy_screen_skips_the_update_and_counts_it() {
        let mut m = MusicIn::new();
        let io = model();
        let held = io.lock().unwrap();
        m.sync(&io, 1.0);
        drop(held);
        assert!(m
            .summary()
            .iter()
            .any(|l| l == "I/O view: 1 periods skipped its update because the screen held it"));
    }

    #[test]
    fn notes_on_any_channel_play_and_are_named() {
        let mut m = MusicIn::new();
        // Jay's keyboard in the first capture: channel 6, C3 at velocity 48.
        let (keys, lines) = run(&mut m, DIN, &[0x95, 0x30, 0x30, 0x85, 0x30, 0x00]);
        assert_eq!(
            keys,
            vec![
                Key::Down {
                    note: 48,
                    velocity: 48
                },
                Key::Up { note: 48 }
            ]
        );
        assert_eq!(lines.len(), 2);
        assert_eq!(
            lines[0],
            "+1.000 s MIDI In: 95 30 30 = note on · ch 6 · C3 · vel 48"
        );
        assert_eq!(
            lines[1],
            "+1.000 s MIDI In: 85 30 00 = note off · ch 6 · C3"
        );
    }

    #[test]
    fn channel_one_and_ten_are_keys_not_mpc_controls() {
        // On cable 1 and 0 these would be a button press and a pad hit; on
        // MIDI In they are just notes.
        let mut m = MusicIn::new();
        let (keys, _) = run(&mut m, DIN, &[0x90, 0x31, 0x7F, 0x99, 0x25, 0x40]);
        assert_eq!(
            keys,
            vec![
                Key::Down {
                    note: 0x31,
                    velocity: 0x7F
                },
                Key::Down {
                    note: 0x25,
                    velocity: 0x40
                }
            ]
        );
    }

    #[test]
    fn running_status_and_velocity_zero_note_off() {
        let mut m = MusicIn::new();
        let (keys, _) = run(&mut m, DIN, &[0x90, 60, 100, 64, 90, 60, 0, 64, 0]);
        assert_eq!(
            keys,
            vec![
                Key::Down {
                    note: 60,
                    velocity: 100
                },
                Key::Down {
                    note: 64,
                    velocity: 90
                },
                Key::Up { note: 60 },
                Key::Up { note: 64 }
            ]
        );
    }

    #[test]
    fn clock_is_counted_not_listed() {
        let mut m = MusicIn::new();
        let mut ticks = vec![0xF8; 2400];
        // A note in the middle of the clock still comes through whole.
        ticks.splice(1000..1000, [0x90, 60, 100]);
        let (keys, lines) = run(&mut m, DIN, &ticks);
        assert_eq!(keys.len(), 1);
        assert_eq!(
            lines.len(),
            2,
            "one line for the clock, one for the note: {lines:?}"
        );
        assert!(lines[0].contains("clock arriving"));
        let mut later = Vec::new();
        m.tick(1.5, &mut |l| later.push(l));
        assert!(later.is_empty(), "still within a second of the last tick");
        m.tick(2.5, &mut |l| later.push(l));
        assert_eq!(later.len(), 1);
        assert!(later[0].contains("clock stopped arriving (2400 ticks so far)"));
        assert!(m.summary()[0].contains("2400 clock ticks"));
    }

    #[test]
    fn active_sensing_once_then_counted() {
        let mut m = MusicIn::new();
        let (_, lines) = run(&mut m, DIN, &[0xFE; 50]);
        assert_eq!(lines.len(), 1);
        assert!(m.summary()[0].contains("50 active sensing"));
    }

    #[test]
    fn start_stop_and_sysex_are_listed() {
        let mut m = MusicIn::new();
        let (keys, lines) = run(
            &mut m,
            DIN,
            &[0xFA, 0xF0, 0x7E, 0x00, 0x06, 0x01, 0xF7, 0xFC],
        );
        assert!(keys.is_empty());
        assert_eq!(
            lines,
            vec![
                "+1.000 s MIDI In: FA = start".to_string(),
                "+1.000 s MIDI In: SysEx, 6 bytes = ignored".to_string(),
                "+1.000 s MIDI In: FC = stop".to_string(),
            ]
        );
    }

    #[test]
    fn unplugging_lets_go_of_held_keys() {
        let mut m = MusicIn::new();
        let usb = m.plugged("Keystation 49").expect("a port number");
        assert_ne!(usb, DIN);
        let (keys, lines) = run(&mut m, usb, &[0x90, 60, 100, 62, 80]);
        assert_eq!(keys.len(), 2);
        assert!(lines[0].contains("Keystation 49: 90 3C 64"));
        let mut gone = Vec::new();
        assert_eq!(m.unplugged(usb, &mut gone), 2, "both held notes let go");
        gone.sort_by_key(|k| match k {
            Key::Up { note } => *note,
            _ => 0,
        });
        assert_eq!(gone, vec![Key::Up { note: 60 }, Key::Up { note: 62 }]);
        // Plugged back in, it keeps its number.
        assert_eq!(m.plugged("Keystation 49"), Some(usb));
    }

    #[test]
    fn pressure_reaches_the_track_but_ccs_wait_for_learn() {
        let mut m = MusicIn::new();
        let (keys, lines) = run(&mut m, DIN, &[0x90, 60, 100, 0xA0, 60, 33, 0xB0, 74, 64]);
        assert_eq!(
            keys,
            vec![
                Key::Down {
                    note: 60,
                    velocity: 100
                },
                Key::Pressure {
                    note: 60,
                    value: 33
                }
            ]
        );
        assert!(lines[2].ends_with("B0 4A 40 = CC 74 · ch 1 · 64"));
    }

    #[test]
    fn describe_every_kind() {
        assert_eq!(
            describe(&Msg::Program { ch: 0, program: 11 }),
            "program 12 · ch 1"
        );
        assert_eq!(
            describe(&Msg::PitchBend { ch: 2, value: -100 }),
            "pitch bend · ch 3 · -100"
        );
        assert_eq!(
            describe(&Msg::ChanPressure { ch: 15, value: 7 }),
            "channel pressure · ch 16 · 7"
        );
    }

    /// A device with one port keeps its name; one with several gets each
    /// port's own, paired with its output; outputs past the inputs are
    /// ports too.
    #[test]
    fn each_port_of_a_usb_device_is_a_port_of_its_own() {
        let one = usb_ports("Keystep 37", &["Keystep 37 MIDI 1".to_string()], 1);
        assert_eq!(
            one,
            vec![UsbPort {
                name: "Keystep 37".into(),
                input: Some(0),
                output: Some(0),
                about: "USB".into()
            }]
        );
        let subs: Vec<String> = ["MIO4 MIDI 1", "MIO4 MIDI 2", "", "MIO4 MIDI 2"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let four = usb_ports("MIO4", &subs, 5);
        let names: Vec<&str> = four.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(
            names,
            ["MIO4 MIDI 1", "MIO4 MIDI 2", "MIO4 3", "MIO4 4", "MIO4 5"]
        );
        assert_eq!(four[1].input, Some(1));
        assert_eq!(
            four.iter().map(|p| p.output).collect::<Vec<_>>(),
            [Some(0), Some(1), Some(2), Some(3), Some(4)]
        );
        assert_eq!(four[4].input, None);
        assert_eq!(four[2].about, "USB · port 3 of 5 on MIO4");
        // An output-only cable.
        let out = usb_ports("UM-ONE", &[], 1);
        assert_eq!(out[0].name, "UM-ONE");
        assert_eq!(out[0].input, None);
        assert_eq!(out[0].output, Some(0));
        assert!(usb_ports("Nothing", &[], 0).is_empty());
    }

    #[test]
    fn outputs_are_counted_from_proc() {
        let text = "Arturia KeyStep 37\n\nOutput 0\n  Tx bytes     : 0\nOutput 1\n  Tx bytes     : 0\nInput 0\n  Rx bytes     : 12\n  Buffer size  : 4096\n";
        assert_eq!(proc_midi_outputs(text), 2);
        assert_eq!(
            proc_midi_outputs("MPK mini\n\nInput 0\n  Rx bytes : 3\n"),
            0
        );
        assert_eq!(proc_midi_outputs(""), 0);
    }

    #[test]
    fn usb_cards_skip_the_mpc_and_need_a_midi_device() {
        let cards = vec![
            (0, "MPCOne".to_string(), "USB-Audio - MPC One".to_string()),
            (
                1,
                "rockchiprk817co".to_string(),
                "rockchip,rk817-codec".to_string(),
            ),
            (
                2,
                "Keystation".to_string(),
                "USB-Audio - Keystation 49 MK3".to_string(),
            ),
            (
                3,
                "Camera".to_string(),
                "USB-Audio - USB Camera".to_string(),
            ),
        ];
        let found = usb_midi_cards(&cards, &[0, 1], |n| n != 3);
        assert_eq!(found, vec![(2, "Keystation 49 MK3".to_string())]);
    }

    fn cutoff() -> ParamAddr {
        "t1.inst.12".parse().unwrap()
    }

    /// Expression in (3.7): any keyboard's wheel and channel pressure
    /// reach the armed track; an MPE controller's member channels slide,
    /// press and shade their own notes, each after its note; a route gets
    /// its port's; unplugging puts the wheel back.
    #[test]
    fn a_keyboards_wheel_and_an_mpe_controllers_notes_reach_the_tracks() {
        use mpfree_midi::mpe::Zone;
        use mpfree_midi::profile::Profile;
        use mpfree_midi::setup::TrackRoute;
        use mpfree_node::Expr;
        let mut m = MusicIn::new();
        let io = model();
        let usb = m.plugged("Keystep 37").unwrap();
        let mpe = m.plugged("Seaboard").unwrap();
        {
            let mut v = io.lock().unwrap();
            let (p, _) = Profile::parse(
                "mpfree-profile 1\nname = Seaboard\nmatch = Seaboard\nmpe = lower 15\n",
            )
            .unwrap();
            v.profiles.add(p).unwrap();
            v.gen += 1;
        }
        m.sync(&io, 0.5);
        assert_eq!(m.mpe[usize::from(mpe.0)].zone(), Zone::Lower(15));
        assert_eq!(m.mpe[usize::from(usb.0)].zone(), Zone::Off);
        // Any keyboard: a note, the wheel half up, the channel pressed.
        let (keys, _) = run(&mut m, usb, &[0x90, 60, 96, 0xE0, 0, 0x60, 0xD0, 70]);
        assert_eq!(
            keys,
            [
                Key::Down {
                    note: 60,
                    velocity: 96
                },
                Key::Bend { value: 0.5 },
                Key::Pressure {
                    note: 60,
                    value: 70
                },
            ]
        );
        // MPE: channel 2's slide and tone come just after its note, then
        // its own bend slides only it; the master's bend bends them all.
        let (keys, _) = run(
            &mut m,
            mpe,
            &[
                0xE1, 0, 0x50, 0xB1, 74, 127, 0x91, 64, 80, 0x92, 67, 80, 0xE1, 0, 0x30, 0xE0, 0,
                0x30,
            ],
        );
        assert_eq!(
            keys,
            [
                Key::Down {
                    note: 64,
                    velocity: 80
                },
                Key::Expr {
                    note: 64,
                    kind: Expr::Slide,
                    value: 12.0
                },
                Key::Expr {
                    note: 64,
                    kind: Expr::Tone,
                    value: 1.0
                },
                Key::Down {
                    note: 67,
                    velocity: 80
                },
                Key::Expr {
                    note: 64,
                    kind: Expr::Slide,
                    value: -12.0
                },
                Key::Bend { value: -0.25 },
            ]
        );
        // Unplugged: its keys up, and the wheel it left bent put back.
        let mut keys = Vec::new();
        m.unplugged(mpe, &mut keys);
        assert_eq!(keys.last(), Some(&Key::Bend { value: 0.0 }));
        assert!(keys.contains(&Key::Up { note: 64 }) && keys.contains(&Key::Up { note: 67 }));
        // A route takes its port's wheel, for its own track.
        {
            let mut v = io.lock().unwrap();
            v.setup.routes = vec![TrackRoute {
                port: mpfree_midi::setup::DIN_NAME.to_string(),
                channel: Some(3),
                track: 5,
                blocks: Default::default(),
            }];
            v.gen += 1;
        }
        m.sync(&io, 1.0);
        let (keys, _) = run(&mut m, DIN, &[0xE3, 0x7F, 0x7F]);
        assert!(keys.is_empty(), "{keys:?}");
        let routed: Vec<_> = m.take_routed().collect();
        assert_eq!(routed.len(), 1);
        assert!(matches!(routed[0], (5, Key::Bend { value }) if value > 0.99));
    }

    /// A keyboard's sustain pedal, mod wheel, breath and foot controller
    /// reach the armed track as keys, and a route's its own track; other
    /// CCs don't. A pedal held when the keyboard is unplugged comes up
    /// where it went down; one let go first doesn't come up again.
    #[test]
    fn pedals_and_the_mod_wheel_reach_the_tracks_and_come_up_on_an_unplug() {
        use mpfree_midi::setup::TrackRoute;
        let mut m = MusicIn::new();
        let io = model();
        let usb = m.plugged("Keystep 37").unwrap();
        let (keys, _) = run(
            &mut m,
            usb,
            &[
                0xB0, 64, 127, 0xB0, 1, 127, 0xB0, 2, 0, 0xB0, 11, 127, 0xB0, 74, 9, 0xB0, 66, 127,
            ],
        );
        let ctl = |ctl, value| Key::Control { ctl, value };
        assert_eq!(
            keys,
            [
                ctl(Ctl::Sustain, 1.0),
                ctl(Ctl::ModWheel, 1.0),
                ctl(Ctl::Breath, 0.0),
                ctl(Ctl::Foot, 1.0),
                ctl(Ctl::Sostenuto, 1.0),
            ]
        );
        // Sostenuto lets go; a route on channel 4 holds its own track's
        // sustain.
        {
            let mut v = io.lock().unwrap();
            v.setup.routes = vec![TrackRoute {
                port: "Keystep 37".to_string(),
                channel: Some(3),
                track: 5,
                blocks: Default::default(),
            }];
            v.gen += 1;
        }
        m.sync(&io, 1.0);
        let (keys, _) = run(&mut m, usb, &[0xB0, 66, 0, 0xB3, 64, 100, 0xB3, 1, 64]);
        assert_eq!(keys, [ctl(Ctl::Sostenuto, 0.0)]);
        let routed: Vec<_> = m.take_routed().collect();
        assert_eq!(
            routed,
            [
                (5, ctl(Ctl::Sustain, 100.0 / 127.0)),
                (5, ctl(Ctl::ModWheel, 64.0 / 127.0))
            ]
        );
        // Unplugged: the armed track's sustain and the route's come up;
        // the wheels stay where they are, as a real wheel does.
        let mut keys = Vec::new();
        assert_eq!(m.unplugged(usb, &mut keys), 0, "no key was held");
        assert_eq!(keys, [ctl(Ctl::Sustain, 0.0)]);
        let routed: Vec<_> = m.take_routed().collect();
        assert_eq!(routed, [(5, ctl(Ctl::Sustain, 0.0))]);
        // Nothing is left to come up.
        m.plugged("Keystep 37").unwrap();
        let mut keys = Vec::new();
        m.unplugged(usb, &mut keys);
        assert!(keys.is_empty(), "{keys:?}");
    }

    /// On a "Learn only" port a pedal going down plays nothing, but one
    /// coming up still lets go of what it held.
    #[test]
    fn a_learn_only_port_lets_a_pedal_up() {
        let mut m = MusicIn::new();
        let io = model();
        let (keys, _) = run(&mut m, DIN, &[0xB0, 64, 127]);
        assert_eq!(keys.len(), 1);
        {
            let mut v = io.lock().unwrap();
            v.setup.set_port(mpfree_midi::setup::DIN_NAME, |p| {
                p.input = PortInput::LearnOnly
            });
            v.gen += 1;
        }
        m.sync(&io, 1.0);
        let (keys, _) = run(&mut m, DIN, &[0xB0, 66, 127, 0xB0, 1, 30, 0xB0, 64, 0]);
        assert_eq!(
            keys,
            [Key::Control {
                ctl: Ctl::Sustain,
                value: 0.0
            }]
        );
    }

    /// The pedals down on the armed track come up when another is armed;
    /// the wheels aren't pedals.
    #[test]
    fn the_armed_tracks_pedals_come_up_once() {
        let mut d = PedalsDown::default();
        d.key(Ctl::Sustain, 1.0);
        d.key(Ctl::ModWheel, 1.0);
        d.key(Ctl::Soft, 0.6);
        d.key(Ctl::Sostenuto, 1.0);
        d.key(Ctl::Sostenuto, 0.2);
        assert_eq!(d.lift().collect::<Vec<_>>(), [Ctl::Sustain, Ctl::Soft]);
        assert_eq!(d.lift().count(), 0);
    }

    /// An MPE controller that says its zone (RPN 6 on channel 1) is heard
    /// as one, and the log says so.
    #[test]
    fn an_mpe_controller_that_says_its_zone_is_heard() {
        use mpfree_node::Expr;
        let mut m = MusicIn::new();
        let usb = m.plugged("Linnstrument").unwrap();
        let (keys, lines) = run(&mut m, usb, &[0xB0, 101, 0, 0xB0, 100, 6, 0xB0, 6, 7]);
        assert!(keys.is_empty());
        assert!(
            lines
                .iter()
                .any(|l| l.ends_with("MPE lower zone, channels 2 to 8 (it said so)")),
            "{lines:?}"
        );
        let (keys, _) = run(&mut m, usb, &[0x93, 50, 90, 0xB3, 74, 0]);
        assert_eq!(
            keys[1],
            Key::Expr {
                note: 50,
                kind: Expr::Tone,
                value: 0.0
            }
        );
    }

    /// The MIDI monitor: each message in is listed once, with where it
    /// went, and `sync` moves the lines to the I/O view and says the
    /// monitor is fed.
    #[test]
    fn the_monitor_says_where_each_message_went() {
        let mut m = MusicIn::new();
        let usb = m.plugged("Keystep 37").unwrap();
        let s = following_din();
        m.sync(&s, 0.0);
        assert!(s.lock().unwrap().live.built.monitor);
        assert!(s
            .lock()
            .unwrap()
            .monitor
            .shown_ports()
            .any(|p| p.name == "Keystep 37"));
        // A kit on track 3: a key plays the kit pad that sends its note,
        // in any bank (20 is H2, 113). On the synth without a pad layout,
        // the pad the MPC gives it, or none.
        m.set_armed(2, KeysPlay::KitPads);
        run(&mut m, usb, &[0x99, 37, 100, 0x99, 20, 90]);
        m.set_armed(2, KeysPlay::Pads);
        run(&mut m, usb, &[0x99, 20, 90]);
        // A synth on track 2: by note, and its wheel.
        m.set_armed(1, KeysPlay::Notes);
        run(&mut m, usb, &[0x90, 60, 96, 0xE0, 0, 0x50]);
        // Learn: armed, a knob takes the parameter; then it moves it.
        m.arm(cutoff());
        run(&mut m, usb, &[0xB2, 74, 10]);
        m.sync(&s, 0.5);
        run(&mut m, usb, &[0xB2, 74, 127]);
        // The tempo is from the 5-pin jacks: their clock and Start are
        // followed; a clock on USB isn't; a SysEx is never read.
        run(&mut m, DIN, &[0xF8, 0xFA]);
        run(&mut m, usb, &[0xF8, 0xF0, 0x41, 0x10, 0xF7]);
        m.sync(&s, 1.5);
        let v = s.lock().unwrap();
        let mut went: Vec<(PortId, To)> = v.monitor.newest().map(|(_, l)| (l.port, l.to)).collect();
        went.reverse();
        assert_eq!(
            went,
            [
                (
                    usb,
                    To::Track {
                        track: 2,
                        pad: Some(0)
                    }
                ),
                (
                    usb,
                    To::Track {
                        track: 2,
                        pad: Some(113)
                    }
                ),
                (usb, To::NoPad { track: 2 }),
                (
                    usb,
                    To::Track {
                        track: 1,
                        pad: None
                    }
                ),
                (
                    usb,
                    To::Track {
                        track: 1,
                        pad: None
                    }
                ),
                (usb, To::Took(cutoff())),
                (usb, To::Learned(cutoff())),
                (DIN, To::Follows),
                (DIN, To::Follows),
                (usb, To::NotFollowed),
                (usb, To::Ignored),
            ]
        );
        drop(v);
        // A "Learn only" port's keys: nothing, said so.
        {
            let mut v = s.lock().unwrap();
            v.setup
                .set_port("Keystep 37", |p| p.input = PortInput::LearnOnly);
            v.gen += 1;
        }
        m.sync(&s, 2.0);
        run(&mut m, usb, &[0x90, 64, 90]);
        m.sync(&s, 2.1);
        let v = s.lock().unwrap();
        // (Newer: the clocks stopping, which no message took.)
        let key = v
            .monitor
            .newest()
            .find(|(_, l)| matches!(l.what, What::Channel { .. }))
            .unwrap();
        assert_eq!(key.1.to, To::LearnOnly);
    }

    /// A port set to ignore clock, program and aftertouch (its page): the
    /// monitor still lists each, marked as the port's setting, but the
    /// track, Learn and the clock follower get none of it; the port's notes
    /// still play.
    #[test]
    fn a_port_that_ignores_clock_and_programs_follows_none() {
        use mpfree_midi::setup::IgnoreSet;
        let mut m = MusicIn::new();
        let s = following_din();
        {
            let mut v = s.lock().unwrap();
            v.setup.set_port(mpfree_midi::setup::DIN_NAME, |p| {
                p.ignore = IgnoreSet::NONE
                    .with(Ignored::Clock, true)
                    .with(Ignored::Program, true)
                    .with(Ignored::Aftertouch, true)
            });
            v.gen += 1;
        }
        m.sync(&s, 0.0);
        let (keys, lines) = run(
            &mut m,
            DIN,
            &[0xF8, 0xFA, 0xC0, 5, 0x90, 60, 100, 0xA0, 60, 30, 0xD0, 40],
        );
        // The note plays; the key's pressure doesn't reach it.
        assert_eq!(keys.len(), 1, "{keys:?}");
        assert!(matches!(keys[0], Key::Down { note: 60, .. }), "{keys:?}");
        assert!(
            lines
                .iter()
                .any(|l| l.ends_with("(ignored: this port's setting)")),
            "{lines:?}"
        );
        // The tempo still comes from the port, but its clock and Start
        // never reach the follower.
        assert_eq!(leader_bytes(&mut m).1, Vec::<u8>::new());
        m.sync(&s, 0.5);
        let v = s.lock().unwrap();
        let mut went: Vec<To> = v.monitor.newest().map(|(_, l)| l.to).collect();
        went.reverse();
        assert_eq!(
            went,
            [
                To::PortIgnores,
                To::PortIgnores,
                To::PortIgnores,
                To::Track {
                    track: 0,
                    pad: None
                },
                To::PortIgnores,
                To::PortIgnores,
            ]
        );
    }

    /// The Monitor's "Send note-offs to MIDI out": taken once, and the
    /// monitor's out notes go up; the developer tools' switch reaches the
    /// model.
    #[test]
    fn the_monitors_note_offs_are_taken_once() {
        let mut m = MusicIn::new();
        let s = following_din();
        m.feed.sent(DIN, 0.1, &[0x91, 36, 110], To::SentBy(1));
        m.feed.debug = true;
        m.sync(&s, 0.2);
        {
            let mut v = s.lock().unwrap();
            assert_eq!(v.monitor.still_on().len(), 1);
            assert!(v.live.debug);
            v.asks.note_offs = true;
        }
        assert!(!m.take_note_offs());
        m.sync(&s, 0.3);
        assert!(m.take_note_offs());
        assert!(!m.take_note_offs());
        let v = s.lock().unwrap();
        assert!(v.monitor.still_on().is_empty());
        assert!(!v.asks.note_offs);
    }

    /// Armed, a knob on a keyboard takes the parameter; the project keeps
    /// it by the keyboard's name at the next sync; then the knob sets the
    /// parameter and plays nothing.
    #[test]
    fn learn_a_knob_keep_it_and_turn_it() {
        let mut m = MusicIn::new();
        let usb = m.plugged("Keystep 37").unwrap();
        let s = model();
        m.sync(&s, 0.0);
        m.arm(cutoff());
        let (keys, lines) = run(&mut m, usb, &[0xB2, 74, 10]);
        assert!(keys.is_empty());
        assert!(lines
            .iter()
            .any(|l| l.ends_with("MIDI Learn: CC 74 · ch 3 · Keystep 37 takes t1.inst.12")));
        let took: Vec<_> = m.learning().collect();
        assert_eq!(
            took,
            [Learning::Took {
                target: cutoff(),
                control: "CC 74 · ch 3 · Keystep 37".into()
            }]
        );
        assert_eq!(m.armed(), None);
        let gen = s.lock().unwrap().gen;
        m.sync(&s, 0.1);
        {
            let model = s.lock().unwrap();
            assert_eq!(model.gen, gen + 1);
            assert_eq!(model.setup.learn.len(), 1);
            assert_eq!(model.setup.learn[0].port.as_deref(), Some("Keystep 37"));
        }
        let (keys, _) = run(&mut m, usb, &[0xB2, 74, 127]);
        assert!(keys.is_empty());
        let set: Vec<_> = m.learning().collect();
        assert!(matches!(set[..], [Learning::Set(w)] if w.target == cutoff() && w.value == 1.0));
        // Another CC still does nothing, and keys still play.
        let (keys, _) = run(&mut m, usb, &[0xB2, 75, 1, 0x92, 60, 90]);
        assert_eq!(keys.len(), 1);
        assert_eq!(m.learning().count(), 0);
    }

    /// MIDI Follow's Send values back: each knob's Q-Link value goes to
    /// the controller's output, on the channel the knob came in on, once
    /// per change; not while the knob turns; the knob's next value counts
    /// from the one sent; nothing with it off, or with no output.
    #[test]
    fn midi_follow_sends_values_back_when_asked() {
        use mpfree_midi::setup::FollowSetup;
        let mut m = MusicIn::new();
        let usb = m
            .plug(&UsbPort {
                name: "Twister".into(),
                input: Some(0),
                output: Some(0),
                about: "USB".into(),
            })
            .unwrap();
        let s = model();
        let set = |back: bool| {
            let mut v = s.lock().unwrap();
            v.setup.follow = FollowSetup {
                on: true,
                port: "Twister".into(),
                knobs: vec![10, 11, 12, 13],
                send_back: back,
            };
            v.gen += 1;
        };
        let sent = |m: &mut MusicIn, t: f64| {
            let mut out = Vec::new();
            m.send_back(t, |to, msg| {
                out.push((to, msg));
                true
            });
            out
        };
        set(false);
        m.sync(&s, 0.0);
        m.set_qlink_values([Some(64), Some(0), None, Some(127)]);
        assert!(sent(&mut m, 0.0).is_empty(), "off by default");
        set(true);
        m.sync(&s, 0.1);
        let cc = |cc, value| (usb, Msg::Cc { ch: 0, cc, value });
        assert_eq!(
            sent(&mut m, 0.1),
            [cc(10, 64), cc(11, 0), cc(13, 127)],
            "a blank card sends nothing"
        );
        assert!(sent(&mut m, 0.2).is_empty(), "once per change");
        // Knob 1 turns on channel 3: its Q-Link moves, but nothing goes
        // back while the hand is on it.
        run(&mut m, usb, &[0xB2, 10, 64, 0xB2, 10, 70]);
        assert_eq!(m.take_knob_turns().collect::<Vec<_>>(), [(0, 6)]);
        m.set_qlink_values([Some(69), Some(0), None, Some(127)]);
        assert!(sent(&mut m, 0.0).is_empty());
        let later = sent(&mut m, 1.0 + HOLD_BACK + 0.01);
        assert_eq!(
            later,
            [(
                usb,
                Msg::Cc {
                    ch: 2,
                    cc: 10,
                    value: 69
                }
            )]
        );
        // Its next value counts from the one sent.
        run(&mut m, usb, &[0xB2, 10, 71]);
        assert_eq!(m.take_knob_turns().collect::<Vec<_>>(), [(0, 2)]);
        // A send that couldn't go goes again.
        m.set_qlink_values([Some(69), Some(5), None, Some(127)]);
        let mut tries = 0;
        m.send_back(10.0, |_, _| {
            tries += 1;
            false
        });
        assert_eq!(tries, 1);
        assert_eq!(sent(&mut m, 10.1), [cc(11, 5)]);
        // A keyboard with no output: nothing goes back.
        let kb = m.plugged("Keystep 37").unwrap();
        assert_ne!(kb, usb);
        {
            let mut v = s.lock().unwrap();
            v.setup.follow.port = "Keystep 37".into();
            v.gen += 1;
        }
        m.sync(&s, 11.0);
        m.set_qlink_values([Some(1), Some(2), Some(3), Some(4)]);
        assert!(sent(&mut m, 12.0).is_empty());
    }

    /// MIDI Follow: the keyboard's knobs turn the Q-Links by as much as
    /// they move and play nothing; its other CCs and keys, and another
    /// port's, play as before; a learned control wins; Learn knobs takes
    /// the next new CCs as its knobs; off, they are anyone's again.
    #[test]
    fn midi_follow_turns_the_q_links_and_learns_its_knobs() {
        use mpfree_midi::setup::FollowSetup;
        let mut m = MusicIn::new();
        let usb = m.plugged("Keystep 37").unwrap();
        let s = model();
        {
            let mut v = s.lock().unwrap();
            v.setup.follow = FollowSetup {
                on: true,
                port: "Keystep 37".into(),
                knobs: vec![74, 71],
                ..FollowSetup::default()
            };
            v.gen += 1;
        }
        m.sync(&s, 0.0);
        let turns = |m: &mut MusicIn| m.take_knob_turns().collect::<Vec<_>>();
        // A knob's first value only says where it is.
        let (keys, _) = run(
            &mut m,
            usb,
            &[
                0xB0, 74, 64, 0xB0, 74, 70, 0xB0, 71, 0, 0xB0, 71, 127, 0xB0, 74, 60,
            ],
        );
        assert!(keys.is_empty());
        assert_eq!(turns(&mut m), [(0, 6), (1, 127), (0, -10)]);
        assert!(turns(&mut m).is_empty());
        // The mod wheel isn't a knob: it reaches the track, as the key does.
        let (keys, _) = run(&mut m, usb, &[0xB0, 1, 5, 0x90, 60, 90]);
        assert_eq!(keys.len(), 2);
        assert!(matches!(
            keys[0],
            Key::Control {
                ctl: Ctl::ModWheel,
                ..
            }
        ));
        run(&mut m, DIN, &[0xB0, 74, 0, 0xB0, 74, 99]);
        assert!(turns(&mut m).is_empty());
        // A control learned on CC 71 moves its value, not Q-Link 2.
        m.arm(cutoff());
        run(&mut m, usb, &[0xB0, 71, 10]);
        m.sync(&s, 0.1);
        let _ = m.learning().count();
        run(&mut m, usb, &[0xB0, 71, 20, 0xB0, 71, 30]);
        assert!(turns(&mut m).is_empty());
        assert_eq!(m.learning().count(), 2);
        // Learn knobs: each new CC is the next knob, and plays nothing.
        s.lock().unwrap().asks.follow_learn = Some(true);
        m.sync(&s, 0.2);
        assert_eq!(s.lock().unwrap().live.follow_learning, Some(0));
        run(&mut m, usb, &[0xB0, 20, 1, 0xB0, 20, 3, 0xB0, 21, 1]);
        assert!(turns(&mut m).is_empty());
        m.sync(&s, 0.3);
        {
            let v = s.lock().unwrap();
            assert_eq!(v.setup.follow.knobs, [20, 21]);
            assert_eq!(v.live.follow_learning, Some(2));
        }
        // Done keeps them.
        s.lock().unwrap().asks.follow_learn = Some(false);
        m.sync(&s, 0.4);
        assert_eq!(s.lock().unwrap().live.follow_learning, None);
        run(&mut m, usb, &[0xB0, 21, 50, 0xB0, 21, 52]);
        assert_eq!(turns(&mut m), [(1, 2)]);
        // Four found stops it by itself.
        s.lock().unwrap().asks.follow_learn = Some(true);
        m.sync(&s, 0.5);
        run(
            &mut m,
            usb,
            &[
                0xB0, 30, 1, 0xB0, 31, 1, 0xB0, 32, 1, 0xB0, 33, 1, 0xB0, 34, 1,
            ],
        );
        m.sync(&s, 0.6);
        {
            let v = s.lock().unwrap();
            assert_eq!(v.setup.follow.knobs, [30, 31, 32, 33]);
            assert_eq!(v.live.follow_learning, None);
        }
        // Off: the knobs are anyone's again.
        {
            let mut v = s.lock().unwrap();
            v.setup.follow.on = false;
            v.gen += 1;
        }
        m.sync(&s, 0.7);
        run(&mut m, usb, &[0xB0, 30, 60, 0xB0, 30, 70]);
        assert!(turns(&mut m).is_empty());
    }

    /// The I/O view's Learn again: the next control moved replaces the
    /// learned one, keeping its range, takeover and where it is kept;
    /// Cancel keeps the old one. A control kept for every project works
    /// in any song, unless the song has its own for the same control.
    #[test]
    fn learn_again_replaces_a_control_and_the_rigs_work_in_every_song() {
        use mpfree_midi::learn::Takeover;
        let mut m = MusicIn::new();
        let usb = m.plugged("Keystep 37").unwrap();
        let s = model();
        let old = Saved::parse(&format!(
            "port Keystep 37 | ch 3 | cc 20 | {} | 0.2 0.8 | absolute | pickup",
            cutoff()
        ))
        .unwrap();
        s.lock().unwrap().setup.learned_in(Kept::Rig, old.clone());
        s.lock().unwrap().gen += 1;
        m.sync(&s, 0.0);
        // The rig's control works with no song mapping.
        run(&mut m, usb, &[0xB2, 20, 127]);
        assert!(
            matches!(m.learning().collect::<Vec<_>>()[..], [Learning::Set(w)] if w.target == cutoff())
        );
        // Learn again, then Cancel: nothing changes.
        s.lock().unwrap().asks.learn_again = Some(0);
        m.sync(&s, 0.1);
        assert_eq!(m.armed(), Some(cutoff()));
        assert!(m.waits());
        s.lock().unwrap().asks.cancel_learn = true;
        m.sync(&s, 0.2);
        assert_eq!(m.armed(), None);
        assert_eq!(
            s.lock().unwrap().setup.rig_learn,
            std::slice::from_ref(&old)
        );
        // Learn again, then a knob: it takes the old one's place in the
        // rig, with its range and takeover.
        s.lock().unwrap().asks.learn_again = Some(0);
        m.sync(&s, 0.3);
        run(&mut m, usb, &[0xB2, 21, 64]);
        assert!(matches!(m.learning().next(), Some(Learning::Took { .. })));
        m.sync(&s, 0.4);
        {
            let v = s.lock().unwrap();
            assert!(v.setup.learn.is_empty());
            assert_eq!(v.setup.rig_learn.len(), 1);
            let new = &v.setup.rig_learn[0];
            assert_eq!(new.source, mpfree_midi::learn::Source::Cc(21));
            assert_eq!((new.lo, new.hi, new.takeover), (0.2, 0.8, Takeover::Pickup));
        }
        assert!(!m.waits());
        // The song's own mapping for CC 21 wins over the rig's.
        let mut song = s.lock().unwrap().setup.rig_learn[0].clone();
        song.target = "t2.inst.3".parse().unwrap();
        s.lock().unwrap().setup.learned(song);
        s.lock().unwrap().gen += 1;
        m.sync(&s, 0.5);
        run(&mut m, usb, &[0xB2, 21, 127]);
        let set: Vec<_> = m.learning().collect();
        assert!(
            matches!(set[..], [Learning::Set(w)] if w.target.to_string() == "t2.inst.3"),
            "{set:?}"
        );
    }

    /// SHIFT + a touch arms Learn until a control takes it: a synth's
    /// knob, learned into the song, then moved by the control.
    #[test]
    fn a_touch_arms_learn_until_a_control_takes_it() {
        let mut m = MusicIn::new();
        let usb = m.plugged("Keystep 37").unwrap();
        let s = model();
        m.sync(&s, 0.0);
        let knob: ParamAddr = "t2.inst.3".parse().unwrap();
        m.arm_from(knob, ArmedBy::Touch);
        assert_eq!(m.armed_by(), Some(ArmedBy::Touch));
        assert!(m.waits(), "it waits, with no chain editor open");
        run(&mut m, usb, &[0xB0, 71, 30]);
        assert!(matches!(
            m.learning().collect::<Vec<_>>()[..],
            [Learning::Took { target, .. }] if target == knob
        ));
        m.sync(&s, 0.1);
        assert_eq!(m.armed_by(), None);
        assert!(!m.waits());
        assert_eq!(s.lock().unwrap().setup.learn.len(), 1);
        assert_eq!(s.lock().unwrap().setup.learn[0].target, knob);
        run(&mut m, usb, &[0xB0, 71, 127]);
        assert!(matches!(
            m.learning().collect::<Vec<_>>()[..],
            [Learning::Set(w)] if w.target == knob && w.value == 1.0
        ));
        // The chain editor's Learn doesn't wait.
        m.arm(cutoff());
        assert_eq!(m.armed_by(), Some(ArmedBy::Chain));
        assert!(!m.waits());
    }

    /// The Learn section's transport card arms Learn on PLAY and STOP; a
    /// key takes it as a button that flips it, kept for every project. A
    /// key on a track's mute flips it too and stays with the song; a key on
    /// anything else works while held.
    #[test]
    fn a_key_on_play_and_stop_or_a_mute_flips_it_and_the_transport_is_the_rigs() {
        use mpfree_midi::learn::Mode;
        let mut m = MusicIn::new();
        let usb = m.plugged("Keystep 37").unwrap();
        let s = model();
        m.sync(&s, 0.0);
        let play_stop = ParamAddr::new(Owner::Transport, transport::PLAYING);
        s.lock().unwrap().asks.learn_target = Some(play_stop);
        m.sync(&s, 0.1);
        assert_eq!(m.armed(), Some(play_stop));
        assert_eq!(m.armed_by(), Some(ArmedBy::Io), "it waits");
        // Note on, then off: the release flips nothing.
        run(&mut m, usb, &[0x90, 48, 100]);
        run(&mut m, usb, &[0x80, 48, 0]);
        let got: Vec<_> = m.learning().collect();
        assert!(
            matches!(got[..], [Learning::Took { target, .. }] if target == play_stop),
            "{got:?}"
        );
        m.sync(&s, 0.2);
        {
            let v = s.lock().unwrap();
            assert!(v.setup.learn.is_empty());
            assert_eq!(v.setup.rig_learn.len(), 1);
            assert_eq!(v.setup.rig_learn[0].mode, Mode::Toggle);
        }
        // Each press flips it: on, then off.
        for want in [1.0, 0.0] {
            run(&mut m, usb, &[0x90, 48, 100]);
            run(&mut m, usb, &[0x80, 48, 0]);
            let got: Vec<_> = m.learning().collect();
            assert!(
                matches!(got[..], [Learning::Set(w)] if w.target == play_stop && w.value == want),
                "{got:?}"
            );
        }
        // A track's mute: a toggle too, in the song.
        let mute = ParamAddr::new(
            Owner::Strip(mpfree_play::mpfree_param::Strip::Track(1)),
            mpfree_fx::ids::mixer::MUTE,
        );
        m.arm_from(mute, ArmedBy::Touch);
        run(&mut m, usb, &[0x90, 50, 100]);
        m.sync(&s, 0.3);
        // A synth's knob on a key works while held.
        m.arm_from(cutoff(), ArmedBy::Touch);
        run(&mut m, usb, &[0x90, 52, 100]);
        m.sync(&s, 0.4);
        let v = s.lock().unwrap();
        let modes: Vec<_> = v.setup.learn.iter().map(|l| (l.target, l.mode)).collect();
        assert_eq!(modes, [(mute, Mode::Toggle), (cutoff(), Mode::Momentary)]);
        assert!(key_toggles(mute) && key_toggles(play_stop));
        assert!(!key_toggles(ParamAddr::new(
            Owner::Transport,
            transport::REC
        )));
    }

    /// A switch of mode moves a control learned on the insert to the new
    /// engine's matching parameter, in the project's list and the learn
    /// map, and unlearns one with no match; an armed learn follows too.
    #[test]
    fn a_learned_control_follows_its_parameter_to_a_new_engine() {
        use crate::fxmodes::Landed;
        use mpfree_node::{Curve, ParamInfo, Unit};
        let mut m = MusicIn::new();
        let usb = m.plugged("Keystep 37").unwrap();
        let s = model();
        m.sync(&s, 0.0);
        let other = ParamAddr::new(cutoff().owner, 13);
        for (target, cc) in [(cutoff(), 74), (other, 75)] {
            m.arm(target);
            run(&mut m, usb, &[0xB2, cc, 10]);
        }
        assert_eq!(m.learning().count(), 2);
        m.sync(&s, 0.1);
        m.arm(cutoff());
        let info = ParamInfo {
            id: 7,
            name: "Cutoff",
            group: "Filter",
            min: 0.0,
            max: 1.0,
            default: 0.5,
            unit: Unit::None,
            curve: Curve::Linear,
            readonly: false,
        };
        m.retarget(&Landed {
            owner: cutoff().owner,
            moves: vec![(12, Some(info)), (13, None)],
        });
        let moved = ParamAddr::new(cutoff().owner, 7);
        assert_eq!(m.armed(), Some(moved), "the armed learn follows");
        m.disarm();
        m.sync(&s, 0.2);
        let kept: Vec<ParamAddr> = s
            .lock()
            .unwrap()
            .setup
            .learn
            .iter()
            .map(|l| l.target)
            .collect();
        assert_eq!(kept, [moved], "CC 75's parameter went nowhere");
        run(&mut m, usb, &[0xB2, 74, 127, 0xB2, 75, 127]);
        let set: Vec<_> = m.learning().collect();
        assert!(matches!(set[..], [Learning::Set(w)] if w.target == moved && w.value == 1.0));
    }

    /// The project's tempo from the 5-pin jacks, following start, stop
    /// and position.
    fn following_din() -> Shared {
        let s = model();
        {
            let mut m = s.lock().unwrap();
            m.setup.tempo = TempoSource::Port(mpfree_midi::setup::DIN_NAME.into());
            m.gen += 1;
        }
        s
    }

    fn leader_bytes(m: &mut MusicIn) -> (Option<Follow>, Vec<u8>) {
        let mut got = (None, Vec::new());
        m.leader_clock(|f, b| got = (f, b.to_vec()));
        got
    }

    /// With the tempo from a port, its clock, Start, Continue, Stop and
    /// Song Position go to the time engine once a period, and nothing
    /// else does; another port's clock doesn't; with MPFree's own tempo,
    /// nothing follows.
    #[test]
    fn the_followed_port_hands_its_clock_on() {
        let mut m = MusicIn::new();
        let usb = m.plugged("Beatstep Pro").unwrap();
        let s = following_din();
        m.sync(&s, 0.0);
        assert!(s.lock().unwrap().live.built.clock_in);
        // The tempo chip names the jack.
        assert_eq!(m.leader_name(), Some("MIDI In"));
        let all = Follow {
            start_stop: true,
            position: true,
        };
        // A note, Song Position bar 3 (32 sixteenths) with a clock inside
        // it, Continue, active sensing, a clock; a Song Position cut short.
        run(
            &mut m,
            DIN,
            &[
                0x90, 60, 100, 0xF2, 0x20, 0xF8, 0x00, 0xFB, 0xFE, 0xF8, 0xF2, 0x01, 0x80,
            ],
        );
        run(&mut m, usb, &[0xF8, 0xFA]);
        assert_eq!(
            leader_bytes(&mut m),
            (
                Some(all),
                vec![0xF2, 0x20, 0xF8, 0x00, 0xFB, 0xF8, 0xF2, 0x01, 0x80]
            )
        );
        assert_eq!(leader_bytes(&mut m), (Some(all), vec![]));
        {
            let mut v = s.lock().unwrap();
            v.setup.follow_position = false;
            v.gen += 1;
        }
        m.sync(&s, 0.1);
        run(&mut m, DIN, &[0xFC]);
        assert_eq!(
            leader_bytes(&mut m),
            (
                Some(Follow {
                    start_stop: true,
                    position: false
                }),
                vec![0xFC]
            )
        );
        {
            let mut v = s.lock().unwrap();
            v.setup.tempo = TempoSource::Internal;
            v.gen += 1;
        }
        m.sync(&s, 0.2);
        run(&mut m, DIN, &[0xF8]);
        assert_eq!(leader_bytes(&mut m), (None, vec![]));
        assert_eq!(m.leader_name(), None);
        // A USB device is named as it names itself.
        {
            let mut v = s.lock().unwrap();
            v.setup.tempo = TempoSource::Port("Beatstep Pro".into());
            v.gen += 1;
        }
        m.sync(&s, 0.3);
        assert_eq!(m.leader_name(), Some("Beatstep Pro"));
        // Named on its page: the chip and the Monitor say the name given;
        // the setup still names it by its own.
        {
            let mut v = s.lock().unwrap();
            v.setup.set_shown("Beatstep Pro", "BSP", []).unwrap();
            v.gen += 1;
        }
        m.sync(&s, 0.4);
        assert_eq!(m.leader_name(), Some("BSP"));
        {
            let v = s.lock().unwrap();
            assert_eq!(v.monitor.port_name(usb), Some("BSP"));
            assert_eq!(v.monitor.own_name(usb), Some("Beatstep Pro"));
        }
        // Its own name again.
        {
            let mut v = s.lock().unwrap();
            v.setup.set_shown("Beatstep Pro", "", []).unwrap();
            v.gen += 1;
        }
        m.sync(&s, 0.5);
        assert_eq!(m.leader_name(), Some("Beatstep Pro"));
        assert_eq!(
            s.lock().unwrap().monitor.port_name(usb),
            Some("Beatstep Pro")
        );
    }

    /// The app's turn each period, as main.rs takes it: a keyboard's
    /// clock at 100 BPM read once a period, through MusicIn to the time
    /// engine's follower, sets the tempo and starts the transport.
    #[test]
    fn a_clock_on_the_jacks_sets_the_tempo_and_starts_the_song() {
        const RATE: u32 = 44_100;
        const PERIOD: u32 = 128;
        let mut m = MusicIn::new();
        let s = following_din();
        m.sync(&s, 0.0);
        let mut time = crate::time::TimeHost::new(RATE, 120.0).unwrap();
        let pulse = 60.0 * f64::from(RATE) / (100.0 * 24.0);
        let mut next = 1000.0;
        let mut pulses = 0;
        let mut frame = 0u64;
        let mut keys = Vec::new();
        while pulses < 24 * 12 {
            let mut b = Vec::new();
            while next < (frame + u64::from(PERIOD)) as f64 {
                if pulses == 24 {
                    b.push(0xFA);
                }
                b.push(0xF8);
                pulses += 1;
                next += pulse;
            }
            let t = frame as f64 / f64::from(RATE);
            m.bytes(DIN, &b, t, &mut keys, &mut |_| {});
            m.leader_clock(|f, b| {
                time.follow(f);
                if !b.is_empty() {
                    let at = time.next_frame();
                    time.clock_in(at, b);
                }
            });
            time.follow_tempo(120.0);
            time.advance(PERIOD);
            frame += u64::from(PERIOD);
        }
        let bpm = time.followed_bpm().expect("following");
        assert!((bpm - 100.0).abs() < 0.1, "{bpm}");
        assert!(time.info().state.rolling(), "{:?}", time.info().state);
        assert!(keys.is_empty());
    }

    fn info(curve: mpfree_node::Curve, min: f32, max: f32) -> ParamInfo {
        ParamInfo {
            id: 12,
            name: "Test",
            group: "",
            min,
            max,
            default: min,
            unit: mpfree_node::Unit::None,
            curve,
            readonly: false,
        }
    }

    /// The value an endless encoder's turn sets.
    fn turned(m: &mut MusicIn, cc_value: u8) -> f32 {
        run(m, DIN, &[0xB0, 16, cc_value]);
        match m.learning().collect::<Vec<_>>()[..] {
            [Learning::Set(w)] => w.value,
            ref other => panic!("{other:?}"),
        }
    }

    /// An endless encoder steps on from where the parameter stands: from
    /// its value at the start, through values the mixer hasn't taken yet,
    /// and from a value something else set. A stepped parameter keeps
    /// the encoder's part steps between its whole ones.
    #[test]
    fn an_endless_encoder_steps_from_where_the_value_stands() {
        use mpfree_midi::learn::{Encoding, Mode};
        let step = 1.0 / mpfree_midi::learn::RELATIVE_STEPS;
        let mut m = MusicIn::new();
        let s = model();
        m.sync(&s, 0.0);
        m.arm(cutoff());
        run(&mut m, DIN, &[0xB0, 16, 1]);
        m.learning().count();
        m.sync(&s, 0.0);
        {
            let mut v = s.lock().unwrap();
            v.setup.learn[0].mode = Mode::Relative(Encoding::TwosComplement);
            v.gen += 1;
        }
        m.sync(&s, 0.0);
        let lin = info(mpfree_node::Curve::Linear, 0.0, 100.0);
        // The mixer has it at 50 before the encoder moves.
        m.observe_learned(0.5, |_| Some((lin, 50.0)));
        assert!((turned(&mut m, 1) - (0.5 + step)).abs() < 1e-6);
        // The mixer hasn't taken it yet: the next turn still steps on.
        m.observe_learned(1.1, |_| Some((lin, 50.0)));
        assert!((turned(&mut m, 1) - (0.5 + 2.0 * step)).abs() < 1e-6);
        // Taken, it stands; set to 20 on the screen, the encoder goes on
        // from 20.
        m.observe_learned(2.0, |_| Some((lin, lin.from_unit(0.5 + 2.0 * step))));
        assert!((turned(&mut m, 127) - (0.5 + step)).abs() < 1e-6);
        m.observe_learned(3.0, |_| Some((lin, 20.0)));
        assert!((turned(&mut m, 1) - (0.2 + step)).abs() < 1e-6);
        // A parameter with four steps: slow turns add up to the next one.
        let four = info(mpfree_node::Curve::Stepped, 0.0, 3.0);
        let mut held = 0.0;
        let mut t = 4.0;
        m.observe_learned(t, |_| Some((four, held)));
        for _ in 0..22 {
            let u = turned(&mut m, 1);
            held = four.from_unit(u);
            t += 1.0;
            m.observe_learned(t, |_| Some((four, held)));
        }
        assert_eq!(held, 1.0);
    }

    /// A key held before arming still ends; a learned key plays nothing;
    /// a port set to "Plays tracks" learns nothing.
    #[test]
    fn learn_takes_only_what_it_uses() {
        let mut m = MusicIn::new();
        let s = model();
        m.sync(&s, 0.0);
        run(&mut m, DIN, &[0x90, 48, 100]);
        m.arm(cutoff());
        let (keys, _) = run(&mut m, DIN, &[0x80, 48, 0]);
        assert_eq!(keys, [Key::Up { note: 48 }]);
        let (keys, _) = run(&mut m, DIN, &[0x90, 50, 100, 0x80, 50, 0]);
        assert!(keys.is_empty());
        assert!(matches!(m.learning().next(), Some(Learning::Took { .. })));
        let (keys, _) = run(&mut m, DIN, &[0x90, 50, 100]);
        assert!(keys.is_empty());
        s.lock()
            .unwrap()
            .setup
            .set_port(mpfree_midi::setup::DIN_NAME, |p| {
                p.input = PortInput::PlaysTracks
            });
        s.lock().unwrap().gen += 1;
        m.sync(&s, 0.1);
        m.arm(cutoff());
        let (keys, _) = run(&mut m, DIN, &[0xB0, 20, 64]);
        assert!(keys.is_empty());
        assert_eq!(m.armed(), Some(cutoff()), "nothing took it");
        // The mod wheel plays the track instead.
        let (keys, _) = run(&mut m, DIN, &[0xB0, 1, 127]);
        assert_eq!(
            keys,
            [Key::Control {
                ctl: Ctl::ModWheel,
                value: 1.0
            }]
        );
        assert_eq!(m.armed(), Some(cutoff()));
    }
}
