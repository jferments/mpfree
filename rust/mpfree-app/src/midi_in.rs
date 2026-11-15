//! MIDI from outside the MPC: the 5-pin MIDI In jack and USB-MIDI keyboards
//! on the USB-A socket. Read only: nothing here ever sends a byte.
//!
//! - **MIDI In** is the controller's cable 2 ("MIDI Port"), seen in the
//!   owner's read-only captures of 2026-11-12
//!   (docs/dev/hardware/controller-protocol.md). The app already reads that
//!   cable with the controller's other two; [`MusicIn::bytes`] takes what
//!   came in on it. Its bytes are never read as the MPC's own pads, knobs or
//!   buttons: a keyboard on channel 1 would otherwise press buttons, and one
//!   on channel 10 would hit pads.
//! - **USB keyboards** are ALSA cards of their own. [`usb_midi_cards`] finds
//!   them in `/proc/asound/cards`, the app opens each one's first input for
//!   reading, and [`MusicIn::plugged`] / [`MusicIn::unplugged`] keep the
//!   port table (a keyboard plugged back in gets its old port number).
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

use mpfree_midi::input::{In, InputRouter, PortFilter};
use mpfree_midi::learn::{Input, LearnMap, Saved, Write};
use mpfree_midi::msg::Msg;
pub use mpfree_midi::port::DIN;
use mpfree_midi::port::{PortId, Ports, MAX_PORTS};
use mpfree_midi::setup::{PortInput, TempoSource};
use mpfree_midi_clock::ClockIn;
use mpfree_midi_ui::{PortLive, Shared};
use mpfree_node::ParamInfo;
use mpfree_play::mpfree_param::ParamAddr;

use crate::time::Follow;

/// The clock's frames are microseconds of the app's clock.
const CLOCK_RATE: u32 = 1_000_000;

/// What a keyboard did, for the armed track.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Down {
        note: u8,
        velocity: u8,
    },
    Up {
        note: u8,
    },
    /// Poly aftertouch on a held key.
    Pressure {
        note: u8,
        value: u8,
    },
}

/// What MIDI Learn did, for the app.
#[derive(Clone, Debug, PartialEq)]
pub enum Learning {
    /// A control took the armed parameter: what it is, in words.
    Took { target: ParamAddr, control: String },
    /// A learned control moved: set this.
    Set(Write),
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
    /// Periods whose I/O view update was skipped because the screen held
    /// the model.
    busy: u64,
    /// The project's learned controls, on this run's port numbers.
    learn: LearnMap,
    /// What Learn did since the app last asked.
    learning: Vec<Learning>,
    /// Controls learned but not yet in the project (the screen held it).
    unsaved: Vec<Saved>,
    /// Switches of mode whose learned controls follow their parameters,
    /// not yet in the project (the screen held it).
    moved: Vec<crate::fxmodes::Landed>,
    /// Learned controls the project couldn't keep (past its limit).
    unkept: u64,
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
            busy: 0,
            learn: LearnMap::new(),
            learning: Vec::new(),
            unsaved: Vec::new(),
            moved: Vec::new(),
            unkept: 0,
            learn_sets: Vec::with_capacity(mpfree_midi::setup::MAX_LEARNED),
            next_look: 0.0,
            follow: None,
            leader: None,
            leader_bytes: [0; LEADER_BYTES],
            leader_len: 0,
            leader_sp: 0,
            leader_dropped: 0,
        }
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
    /// 5-pin jack, a USB device's own name), while the project's tempo is
    /// from a port and that port is plugged in.
    pub fn leader_name(&self) -> Option<&str> {
        let id = self.leader?;
        let p = self.ports.all().iter().find(|p| p.id == id)?;
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
    /// `target`.
    pub fn arm(&mut self, target: ParamAddr) {
        self.learn.arm(target);
    }

    pub fn disarm(&mut self) {
        self.learn.disarm();
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
            for l in self.unsaved.drain(..) {
                if !m.setup.learned(l) {
                    self.unkept += 1;
                }
            }
            m.gen += 1;
        }
        if !self.moved.is_empty() {
            for landed in self.moved.drain(..) {
                m.setup.learn.retain_mut(|s| follow(s, &landed));
            }
            // The learn map is built again from the list.
            m.gen += 1;
        }
        if self.applied != Some(m.gen) {
            for p in self.ports.all() {
                if let Some(i) = self.input.get_mut(usize::from(p.id.0)) {
                    *i = m.setup.port(&p.name).input;
                }
            }
            let learned = std::mem::take(&mut m.setup.learn);
            self.build_learn(&learned);
            m.setup.learn = learned;
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
            // The app follows a clock now (M4).
            m.live.built.clock_in = true;
            self.applied = Some(m.gen);
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
                    has_in: p.has_in,
                    has_out: p.has_out,
                    last_in: None,
                })
                .collect();
            self.ports_changed = false;
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

    /// A USB keyboard was opened for reading: its port number, or None when
    /// every port number is taken.
    pub fn plugged(&mut self, name: &str) -> Option<PortId> {
        self.ports_changed = true;
        // Its input setting is the project's, by name.
        self.applied = None;
        self.ports.plugged(name, true, false)
    }

    /// A USB keyboard went away: every key it held is let go.
    pub fn unplugged(&mut self, port: PortId, keys: &mut Vec<Key>) {
        self.router.port_gone(port, 0, |e| key_of(e, keys));
        self.ports.unplugged(port);
        self.ports_changed = true;
        if let Some(c) = self.clock.get_mut(usize::from(port.0)) {
            *c = ClockIn::new(CLOCK_RATE);
        }
        if let Some(s) = self.seen.get_mut(usize::from(port.0)) {
            s.clock_running = false;
            s.sysex = None;
        }
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
        if self.leader == Some(port) {
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
        for &b in data {
            self.clock[p].byte(frame, b);
            let s = &mut self.seen[p];
            match b {
                0xF8 => {
                    s.clock_ticks += 1;
                    s.last_tick = t;
                    if !s.clock_running {
                        s.clock_running = true;
                        let follows = if self.leader == Some(port) {
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
            let mut learned = None;
            let (learn, learning, sets) =
                (&mut self.learn, &mut self.learning, &mut self.learn_sets);
            self.router.feed(port, b, 0, |e| match e {
                In::Seen { msg, .. } => seen = Some(msg),
                other => {
                    got = true;
                    // What Learn takes doesn't play.
                    if let In::Track { msg, .. } = &other {
                        if learns && learn.takes(port, msg) {
                            learn.input(port, msg, |i| match i {
                                Input::Learned(i) => learned = Some(i),
                                Input::Write(w) => {
                                    note_set(sets, w.target, w.value, t);
                                    learning.push(Learning::Set(w))
                                }
                            });
                            return;
                        }
                    }
                    // "Learn only" plays nothing, but a note-off still
                    // ends a note this port started before.
                    if !(learn_only && starts_or_presses(&other)) {
                        key_of(other, keys)
                    }
                }
            });
            if let Some(m) = learned.and_then(|i| self.learn.get(i).copied()) {
                let name = self.ports.get(port).map(|p| p.name.clone());
                let control =
                    mpfree_midi_ui::control_text(&m.control.source, m.control.channel, &label);
                log(format!(
                    "+{t:.3} s {label}: MIDI Learn: {control} takes {}",
                    m.target
                ));
                self.unsaved.push(Saved::of(&m, name));
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
                    "+{t:.3} s {label}: {} = {}",
                    hex(&bytes[..n]),
                    describe(&msg)
                ));
            }
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

/// A note-on or pressure: what a "Learn only" port doesn't play.
fn starts_or_presses(e: &In) -> bool {
    matches!(
        e,
        In::Track {
            msg: Msg::NoteOn { .. } | Msg::PolyPressure { .. },
            ..
        }
    )
}

/// The router's notes as keys; CCs, bends and program changes wait for
/// MIDI Learn and the track settings.
fn key_of(e: In, keys: &mut Vec<Key>) {
    let In::Track { msg, .. } = e else {
        return;
    };
    match msg {
        Msg::NoteOn { note, vel, .. } => keys.push(Key::Down {
            note,
            velocity: vel,
        }),
        Msg::NoteOff { note, .. } => keys.push(Key::Up { note }),
        Msg::PolyPressure { note, value, .. } => keys.push(Key::Pressure { note, value }),
        _ => {}
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
        assert!(!v.live.ports[1].has_out, "USB ports are only read for now");
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
        m.unplugged(usb, &mut gone);
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
        let (keys, _) = run(&mut m, DIN, &[0xB0, 1, 64]);
        assert!(keys.is_empty());
        assert_eq!(m.armed(), Some(cutoff()), "nothing took it");
    }
}
