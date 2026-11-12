//! MIDI in: keyboards and other devices into tracks, and the clock source
//! into the time engine.
//!
//! The app reads each external port (the DIN jack, a USB keyboard) and
//! gives every byte to [`InputRouter::feed`] with the port it came from.
//! The MPC's own pads and knobs never come through here: they have their
//! own mapper in the app.
//!
//! Two modes, as on Akai's MPC:
//!
//! - [`InputMode::Selected`] (the default): whatever plays on any port,
//!   any channel, goes to the selected track. Plug in a keyboard and it
//!   plays what is on screen.
//! - [`InputMode::PerTrack`]: each track listens to the port and channel
//!   set in its [`Listen`]; several tracks may listen to the same one.
//!
//! A note-off always goes to the tracks its note-on went to, even if the
//! selection or a track's [`Listen`] changed while the key was held, so a
//! note is never left hanging.
//!
//! Real-time and Song Position bytes from the clock source
//! ([`InputRouter::set_clock_source`]) go out as [`In::ClockByte`], for
//! `mpfree_midi_clock::ClockIn`. Clock from any other port is ignored.
//!
//! Nothing allocates after [`InputRouter::new`].

use mpfree_controller::midi::{Message, Parser};

use crate::msg::{Channel, Msg};
use crate::port::{PortId, MAX_PORTS};

/// Tracks the router knows (the same as `mpfree_seq::MAX_TRACKS`).
pub const MAX_TRACKS: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PortFilter {
    Any,
    One(PortId),
}

impl PortFilter {
    pub fn matches(self, p: PortId) -> bool {
        match self {
            PortFilter::Any => true,
            PortFilter::One(q) => q == p,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChanFilter {
    Omni,
    One(Channel),
}

impl ChanFilter {
    pub fn matches(self, ch: Channel) -> bool {
        match self {
            ChanFilter::Omni => true,
            ChanFilter::One(c) => c == ch,
        }
    }
}

/// What one track listens to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Listen {
    pub port: PortFilter,
    pub channel: ChanFilter,
    /// Semitones added to every note. Notes pushed out of 0..=127 are
    /// dropped.
    pub transpose: i8,
}

impl Default for Listen {
    fn default() -> Self {
        Listen {
            port: PortFilter::Any,
            channel: ChanFilter::Omni,
            transpose: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InputMode {
    #[default]
    Selected,
    PerTrack,
}

/// What [`InputRouter::feed`] hands back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum In {
    /// Play this on a track (notes, pressure, bend, CCs, program change).
    Track { track: u8, msg: Msg, frame: u64 },
    /// Every channel message on every port, before routing: for MIDI
    /// Learn, the MIDI monitor and the port activity lights.
    Seen { port: PortId, msg: Msg, frame: u64 },
    /// A byte from the clock source, for `ClockIn::byte`.
    ClockByte { byte: u8, frame: u64 },
}

/// Counters for the Dev page.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InStats {
    pub messages: u64,
    /// Channel messages no track took.
    pub unrouted: u64,
    /// Clock bytes from a port that is not the clock source.
    pub foreign_clock: u64,
    /// Bytes for a port number past [`MAX_PORTS`].
    pub bad_port: u64,
}

const NOTES: usize = 128;
const CHANNELS: usize = 16;

pub struct InputRouter {
    mode: InputMode,
    selected: Option<u8>,
    tracks: [Option<Listen>; MAX_TRACKS],
    clock_source: Option<PortId>,
    parsers: [Parser; MAX_PORTS],
    /// Song Position data bytes still to come, per port.
    spp_left: [u8; MAX_PORTS],
    /// For each port, channel and incoming note: the tracks (a bit per
    /// track) its note-on went to.
    held: Box<[u16]>,
    /// The note each track was sent for a held key (after transpose).
    sent_note: Box<[u8]>,
    pub stats: InStats,
}

impl Default for InputRouter {
    fn default() -> Self {
        Self::new()
    }
}

fn held_index(port: PortId, ch: Channel, note: u8) -> usize {
    (usize::from(port.0) * CHANNELS + usize::from(ch & 0x0F)) * NOTES + usize::from(note & 0x7F)
}

impl InputRouter {
    pub fn new() -> InputRouter {
        InputRouter {
            mode: InputMode::Selected,
            selected: None,
            tracks: [None; MAX_TRACKS],
            clock_source: None,
            parsers: std::array::from_fn(|_| Parser::new()),
            spp_left: [0; MAX_PORTS],
            held: vec![0u16; MAX_PORTS * CHANNELS * NOTES].into_boxed_slice(),
            sent_note: vec![0u8; MAX_PORTS * CHANNELS * NOTES * MAX_TRACKS].into_boxed_slice(),
            stats: InStats::default(),
        }
    }

    pub fn mode(&self) -> InputMode {
        self.mode
    }

    pub fn set_mode(&mut self, mode: InputMode) {
        self.mode = mode;
    }

    /// The track on screen, which plays in [`InputMode::Selected`].
    pub fn set_selected(&mut self, track: Option<u8>) {
        self.selected = track.filter(|&t| usize::from(t) < MAX_TRACKS);
    }

    /// A track's input in [`InputMode::PerTrack`]; None: it listens to
    /// nothing. In [`InputMode::Selected`] its transpose still applies.
    pub fn set_listen(&mut self, track: u8, listen: Option<Listen>) {
        if let Some(slot) = self.tracks.get_mut(usize::from(track)) {
            *slot = listen;
        }
    }

    pub fn listen(&self, track: u8) -> Option<Listen> {
        self.tracks.get(usize::from(track)).copied().flatten()
    }

    /// Which port's clock the time engine follows; None: none.
    pub fn set_clock_source(&mut self, port: Option<PortId>) {
        self.clock_source = port;
    }

    pub fn clock_source(&self) -> Option<PortId> {
        self.clock_source
    }

    /// One byte from `port`, which arrived at `frame`.
    pub fn feed(&mut self, port: PortId, byte: u8, frame: u64, mut out: impl FnMut(In)) {
        let p = usize::from(port.0);
        if p >= MAX_PORTS {
            self.stats.bad_port += 1;
            return;
        }
        let is_clock_source = self.clock_source == Some(port);
        // Song Position (F2 and its two data bytes) and real-time bytes
        // are the clock's; ClockIn skips everything else itself.
        let clock_byte = if byte >= 0xF8 {
            true
        } else if byte == 0xF2 {
            self.spp_left[p] = 2;
            true
        } else if byte >= 0x80 {
            self.spp_left[p] = 0;
            false
        } else if self.spp_left[p] > 0 {
            self.spp_left[p] -= 1;
            true
        } else {
            false
        };
        if clock_byte {
            if is_clock_source {
                out(In::ClockByte { byte, frame });
            } else if byte >= 0xF8 || byte == 0xF2 {
                self.stats.foreign_clock += 1;
            }
        }
        let msg = self.parsers[p].feed(byte);
        if let Some(Message::Channel {
            status,
            data1,
            data2,
        }) = msg
        {
            if let Some(m) = Msg::from_bytes(status, data1, data2) {
                self.stats.messages += 1;
                out(In::Seen {
                    port,
                    msg: m,
                    frame,
                });
                self.route(port, m, frame, &mut out);
            }
        }
    }

    /// Every byte of `bytes` from `port`, all at `frame`.
    pub fn feed_all(&mut self, port: PortId, bytes: &[u8], frame: u64, mut out: impl FnMut(In)) {
        for &b in bytes {
            self.feed(port, b, frame, &mut out);
        }
    }

    fn takes(&self, track: usize, port: PortId, ch: Channel) -> bool {
        match self.mode {
            InputMode::Selected => self.selected == Some(track as u8),
            InputMode::PerTrack => {
                self.tracks[track].is_some_and(|l| l.port.matches(port) && l.channel.matches(ch))
            }
        }
    }

    fn transpose(&self, track: usize) -> i8 {
        self.tracks[track].map_or(0, |l| l.transpose)
    }

    fn route(&mut self, port: PortId, m: Msg, frame: u64, out: &mut impl FnMut(In)) {
        let ch = m.channel();
        match m {
            Msg::NoteOn { note, .. } => {
                let hi = held_index(port, ch, note);
                // A second note-on for a held key: end the first one.
                if self.held[hi] != 0 {
                    self.release(port, ch, note, 64, frame, out);
                }
                let mut mask = 0u16;
                for t in 0..MAX_TRACKS {
                    if !self.takes(t, port, ch) {
                        continue;
                    }
                    if let Some(tm) = m.transposed(self.transpose(t)) {
                        mask |= 1 << t;
                        self.sent_note[hi * MAX_TRACKS + t] = tm.note().unwrap_or(note);
                        out(In::Track {
                            track: t as u8,
                            msg: tm,
                            frame,
                        });
                    }
                }
                self.held[hi] = mask;
                if mask == 0 {
                    self.stats.unrouted += 1;
                }
            }
            Msg::NoteOff { note, vel, .. } => {
                if self.held[held_index(port, ch, note)] == 0 {
                    self.stats.unrouted += 1;
                } else {
                    self.release(port, ch, note, vel, frame, out);
                }
            }
            Msg::PolyPressure { note, value, .. } => {
                let hi = held_index(port, ch, note);
                let mask = self.held[hi];
                for t in 0..MAX_TRACKS {
                    if mask & (1 << t) != 0 {
                        let n = self.sent_note[hi * MAX_TRACKS + t];
                        out(In::Track {
                            track: t as u8,
                            msg: Msg::PolyPressure { ch, note: n, value },
                            frame,
                        });
                    }
                }
            }
            _ => {
                let mut any = false;
                for t in 0..MAX_TRACKS {
                    if self.takes(t, port, ch) {
                        any = true;
                        out(In::Track {
                            track: t as u8,
                            msg: m,
                            frame,
                        });
                    }
                }
                if !any {
                    self.stats.unrouted += 1;
                }
            }
        }
    }

    fn release(
        &mut self,
        port: PortId,
        ch: Channel,
        note: u8,
        vel: u8,
        frame: u64,
        out: &mut impl FnMut(In),
    ) {
        let hi = held_index(port, ch, note);
        let mask = std::mem::take(&mut self.held[hi]);
        for t in 0..MAX_TRACKS {
            if mask & (1 << t) != 0 {
                let n = self.sent_note[hi * MAX_TRACKS + t];
                out(In::Track {
                    track: t as u8,
                    msg: Msg::NoteOff { ch, note: n, vel },
                    frame,
                });
            }
        }
    }

    /// Note-offs for every held key (a port went away, or panic).
    pub fn release_all(&mut self, frame: u64, mut out: impl FnMut(In)) {
        for p in 0..MAX_PORTS {
            for ch in 0..CHANNELS as u8 {
                for note in 0..NOTES as u8 {
                    if self.held[held_index(PortId(p as u8), ch, note)] != 0 {
                        self.release(PortId(p as u8), ch, note, 64, frame, &mut out);
                    }
                }
            }
        }
    }

    /// Note-offs for every key held on `port` (it was unplugged), and a
    /// fresh parser for it.
    pub fn port_gone(&mut self, port: PortId, frame: u64, mut out: impl FnMut(In)) {
        if usize::from(port.0) >= MAX_PORTS {
            return;
        }
        for ch in 0..CHANNELS as u8 {
            for note in 0..NOTES as u8 {
                if self.held[held_index(port, ch, note)] != 0 {
                    self.release(port, ch, note, 64, frame, &mut out);
                }
            }
        }
        self.parsers[usize::from(port.0)] = Parser::new();
        self.spp_left[usize::from(port.0)] = 0;
    }

    /// Keys held now, over every port.
    pub fn held(&self) -> usize {
        self.held.iter().filter(|&&m| m != 0).count()
    }
}
