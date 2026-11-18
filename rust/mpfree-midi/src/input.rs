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
//! In either mode, **routes** ([`InputRouter::set_routes`], the I/O view's
//! "Tracks by channel") send a port and channel to one of the app's tracks
//! whichever track is selected, as [`In::Routed`]. **Thru**
//! ([`InputRouter::set_thru`], the Routing section's squares from a port to
//! an output) sends a port and channel's messages straight on to an
//! output, as [`In::Thru`]. A message a route or a thru takes goes only to
//! them, so a keyboard's channel given to a track or an output never also
//! plays the selected one (no doubled notes).
//!
//! A note-off always goes to the tracks and outputs its note-on went to,
//! even if the selection, a route or a track's [`Listen`] changed while the
//! key was held, so a note is never left hanging. All notes off, All sound off and the mode
//! changes (CC 120, 123 to 127) let go of every key held on their port
//! and channel.
//!
//! Each port can ignore some kinds coming in ([`IgnoreSet`], the port's
//! "Ignore coming in"): an ignored message is still [`In::Seen`], then
//! [`In::Ignored`] instead of being routed; ignored clock bytes never reach
//! the clock source.
//!
//! Real-time and Song Position bytes from the clock source
//! ([`InputRouter::set_clock_source`]) go out as [`In::ClockByte`], for
//! `mpfree_midi_clock::ClockIn`. Clock from any other port is ignored.
//!
//! Each route, each thru and each port's playing of tracks has its own
//! [`Blocks`] (board 5): Filter, Channel, Split, Transpose and Velocity,
//! in order. A note a route's Split leaves out is free for another route
//! or the armed track; what a Filter drops ends there. A note-off goes
//! through the blocks its note-on went through: a route or thru whose
//! blocks would end a held key on another note or channel lets it go
//! first, and a track keeps the note it was sent.
//!
//! Nothing allocates after [`InputRouter::new`].

use mpfree_controller::midi::{Message, Parser};

use crate::blocks::{Blocks, Through};
use crate::msg::{Channel, Msg};
use crate::port::{PortId, MAX_PORTS};
use crate::setup::{ends_notes, IgnoreSet, Ignored};

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

/// The most routes [`InputRouter::set_routes`] keeps.
pub const MAX_ROUTES: usize = 32;

/// A port and channel that play one track, whichever track is selected:
/// the I/O view's "Tracks by channel".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Route {
    pub port: PortFilter,
    pub channel: ChanFilter,
    /// Any of the app's tracks, from 0 (not only the router's
    /// [`MAX_TRACKS`]).
    pub track: u8,
    /// What it does to the messages it takes, in order.
    pub blocks: Blocks,
}

impl Route {
    pub fn takes(&self, port: PortId, ch: Channel) -> bool {
        self.port.matches(port) && self.channel.matches(ch)
    }

    /// Whether a key held through `self` ends the same way through
    /// `other` (the same source and track, blocks that differ in velocity
    /// at most).
    fn plays_like(&self, other: &Route) -> bool {
        self.port == other.port
            && self.channel == other.channel
            && self.track == other.track
            && self.blocks.same_notes(&other.blocks)
    }
}

/// The most thru routes [`InputRouter::set_thru`] keeps.
pub const MAX_THRU: usize = 16;

/// A port and channel whose messages go straight on to an output port:
/// the Routing section's square from a port to a MIDI out port. Channel
/// messages only (notes, pressure, bend, controls, program change); clock
/// and SysEx never go thru.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Thru {
    pub port: PortFilter,
    pub channel: ChanFilter,
    /// The output, by the same port numbers as the inputs.
    pub to: PortId,
    /// What it does to the messages it takes, in order.
    pub blocks: Blocks,
}

impl Thru {
    pub fn takes(&self, port: PortId, ch: Channel) -> bool {
        self.port.matches(port) && self.channel.matches(ch)
    }

    fn plays_like(&self, other: &Thru) -> bool {
        self.port == other.port
            && self.channel == other.channel
            && self.to == other.to
            && self.blocks.same_notes(&other.blocks)
    }
}

/// Whether a key held through `a` ends the same way through `b`.
fn same_way<T>(a: &Option<T>, b: &Option<T>, like: impl Fn(&T, &T) -> bool) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => like(a, b),
        (None, None) => true,
        _ => false,
    }
}

/// What [`InputRouter::feed`] hands back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum In {
    /// Play this on a track (notes, pressure, bend, CCs, program change).
    Track { track: u8, msg: Msg, frame: u64 },
    /// Play this on the track a route names ([`Route`]), whichever track
    /// is selected. `via` is the route's place in
    /// [`InputRouter::set_routes`]'s list.
    Routed {
        track: u8,
        msg: Msg,
        frame: u64,
        via: u8,
    },
    /// Send this, as it came in on `from` and through the thru's blocks,
    /// to the output port `to` ([`Thru`]). `via` is the thru's place in
    /// [`InputRouter::set_thru`]'s list.
    Thru {
        from: PortId,
        to: PortId,
        msg: Msg,
        frame: u64,
        via: u8,
    },
    /// Every channel message on every port, before routing: for MIDI
    /// Learn, the MIDI monitor and the port activity lights.
    Seen { port: PortId, msg: Msg, frame: u64 },
    /// A byte from the clock source, for `ClockIn::byte`.
    ClockByte { byte: u8, frame: u64 },
    /// A channel message the port ignores ([`InputRouter::set_ignore`]),
    /// after its [`In::Seen`]: nothing else gets it.
    Ignored { port: PortId, msg: Msg, frame: u64 },
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
    /// Messages and clock bytes a port's setting ignored.
    pub ignored: u64,
}

const NOTES: usize = 128;
const CHANNELS: usize = 16;
/// A held key's bit for route `r` in `held`, after the tracks' bits.
const ROUTE_BIT: usize = MAX_TRACKS;
/// A held key's bit for thru `r` in `held`, after the routes' bits.
const THRU_BIT: usize = ROUTE_BIT + MAX_ROUTES;
const _: () = assert!(THRU_BIT + MAX_THRU <= 64);

pub struct InputRouter {
    mode: InputMode,
    selected: Option<u8>,
    tracks: [Option<Listen>; MAX_TRACKS],
    routes: [Option<Route>; MAX_ROUTES],
    thru: [Option<Thru>; MAX_THRU],
    /// Each port's blocks on its way to the tracks (the armed track, or
    /// those listening to it), after the routes and thru.
    port_blocks: [Blocks; MAX_PORTS],
    clock_source: Option<PortId>,
    /// What each port ignores coming in.
    ignore: [IgnoreSet; MAX_PORTS],
    parsers: [Parser; MAX_PORTS],
    /// Song Position data bytes still to come, per port.
    spp_left: [u8; MAX_PORTS],
    /// For each port, channel and incoming note: the tracks (a bit per
    /// track), routes (a bit per route, from `ROUTE_BIT`) and thru (a bit
    /// each, from `THRU_BIT`) its note-on went to.
    held: Box<[u64]>,
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
            routes: [None; MAX_ROUTES],
            thru: [None; MAX_THRU],
            port_blocks: [Blocks::NONE; MAX_PORTS],
            clock_source: None,
            ignore: [IgnoreSet::NONE; MAX_PORTS],
            parsers: std::array::from_fn(|_| Parser::new()),
            spp_left: [0; MAX_PORTS],
            held: vec![0u64; MAX_PORTS * CHANNELS * NOTES].into_boxed_slice(),
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

    /// The routes, in order; past [`MAX_ROUTES`] they are left out (the
    /// count left out is returned). A key held through a route that
    /// changes or goes is let go first, on the track it played (a
    /// note-off to `out`), so no note hangs; a change of velocity alone
    /// keeps it.
    pub fn set_routes(
        &mut self,
        routes: impl IntoIterator<Item = Route>,
        frame: u64,
        mut out: impl FnMut(In),
    ) -> usize {
        let mut next = [None; MAX_ROUTES];
        let mut left_out = 0;
        for (i, r) in routes.into_iter().enumerate() {
            match next.get_mut(i) {
                Some(slot) => *slot = Some(r),
                None => left_out += 1,
            }
        }
        for r in 0..MAX_ROUTES {
            if !same_way(&self.routes[r], &next[r], Route::plays_like) {
                self.let_go_route(r, frame, &mut out);
            }
        }
        self.routes = next;
        left_out
    }

    pub fn routes(&self) -> impl Iterator<Item = &Route> {
        self.routes.iter().flatten()
    }

    /// Note-offs for every key held through route `r`, on its track.
    fn let_go_route(&mut self, r: usize, frame: u64, out: &mut impl FnMut(In)) {
        let Some(route) = self.routes[r] else { return };
        let bit = 1u64 << (ROUTE_BIT + r);
        for (hi, mask) in self.held.iter_mut().enumerate() {
            if *mask & bit == 0 {
                continue;
            }
            *mask &= !bit;
            let ch = ((hi / NOTES) % CHANNELS) as Channel;
            let note = (hi % NOTES) as u8;
            if let Some(msg) = route.blocks.apply(Msg::NoteOff { ch, note, vel: 64 }) {
                out(In::Routed {
                    track: route.track,
                    msg,
                    frame,
                    via: r as u8,
                });
            }
        }
    }

    /// Where thru sends each port and channel's messages, in order; past
    /// [`MAX_THRU`] they are left out (the count left out is returned). A
    /// key held through a thru that changes or goes is let go first, at
    /// the output it went to (a note-off to `out`), so no note hangs.
    pub fn set_thru(
        &mut self,
        thru: impl IntoIterator<Item = Thru>,
        frame: u64,
        mut out: impl FnMut(In),
    ) -> usize {
        let mut next = [None; MAX_THRU];
        let mut left_out = 0;
        for (i, r) in thru.into_iter().enumerate() {
            match next.get_mut(i) {
                Some(slot) => *slot = Some(r),
                None => left_out += 1,
            }
        }
        for r in 0..MAX_THRU {
            if !same_way(&self.thru[r], &next[r], Thru::plays_like) {
                self.let_go_thru(r, frame, &mut out);
            }
        }
        self.thru = next;
        left_out
    }

    pub fn thru(&self) -> impl Iterator<Item = &Thru> {
        self.thru.iter().flatten()
    }

    /// Note-offs for every key held through thru `r`, at its output.
    fn let_go_thru(&mut self, r: usize, frame: u64, out: &mut impl FnMut(In)) {
        let Some(thru) = self.thru[r] else { return };
        let bit = 1u64 << (THRU_BIT + r);
        for (hi, mask) in self.held.iter_mut().enumerate() {
            if *mask & bit == 0 {
                continue;
            }
            *mask &= !bit;
            let ch = ((hi / NOTES) % CHANNELS) as Channel;
            let note = (hi % NOTES) as u8;
            if let Some(msg) = thru.blocks.apply(Msg::NoteOff { ch, note, vel: 64 }) {
                out(In::Thru {
                    from: PortId((hi / (NOTES * CHANNELS)) as u8),
                    to: thru.to,
                    msg,
                    frame,
                    via: r as u8,
                });
            }
        }
    }

    /// `port`'s blocks on its way to the tracks (the I/O view's Routing,
    /// its Armed track square). A key held on the port ends on the note
    /// it started whatever the new blocks say: the router keeps the note
    /// each track was sent.
    pub fn set_port_blocks(&mut self, port: PortId, blocks: Blocks) {
        if let Some(slot) = self.port_blocks.get_mut(usize::from(port.0)) {
            *slot = blocks;
        }
    }

    pub fn port_blocks(&self, port: PortId) -> Blocks {
        self.port_blocks
            .get(usize::from(port.0))
            .copied()
            .unwrap_or_default()
    }

    /// Which port's clock the time engine follows; None: none.
    pub fn set_clock_source(&mut self, port: Option<PortId>) {
        self.clock_source = port;
    }

    pub fn clock_source(&self) -> Option<PortId> {
        self.clock_source
    }

    /// What `port` ignores coming in (the rig's "Ignore coming in").
    pub fn set_ignore(&mut self, port: PortId, set: IgnoreSet) {
        if let Some(slot) = self.ignore.get_mut(usize::from(port.0)) {
            *slot = set;
        }
    }

    pub fn ignores(&self, port: PortId) -> IgnoreSet {
        self.ignore
            .get(usize::from(port.0))
            .copied()
            .unwrap_or_default()
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
            if self.ignore[p].has(Ignored::Clock) {
                if byte >= 0xF8 || byte == 0xF2 {
                    self.stats.ignored += 1;
                }
            } else if is_clock_source {
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
                if self.ignore[p].msg(&m) {
                    self.stats.ignored += 1;
                    out(In::Ignored {
                        port,
                        msg: m,
                        frame,
                    });
                    return;
                }
                if let Msg::Cc { ch, cc, .. } = m {
                    if ends_notes(cc) {
                        self.release_channel(port, ch, frame, &mut out);
                    }
                }
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
        let p = usize::from(port.0);
        match m {
            Msg::NoteOn { note, .. } => {
                let hi = held_index(port, ch, note);
                // A second note-on for a held key: end the first one.
                if self.held[hi] != 0 {
                    self.release(port, ch, note, 64, frame, out);
                }
                // Each route and thru that takes it, unless its Split
                // leaves it out.
                let mut mask = 0u64;
                for (r, thru) in self.thru.iter().enumerate() {
                    let Some(x) = thru.filter(|x| x.takes(port, ch)) else {
                        continue;
                    };
                    match x.blocks.run(m) {
                        Through::Free => continue,
                        Through::Dropped => {}
                        Through::Out(msg) => out(In::Thru {
                            from: port,
                            to: x.to,
                            msg,
                            frame,
                            via: r as u8,
                        }),
                    }
                    mask |= 1 << (THRU_BIT + r);
                }
                for (r, route) in self.routes.iter().enumerate() {
                    let Some(x) = route.filter(|x| x.takes(port, ch)) else {
                        continue;
                    };
                    match x.blocks.run(m) {
                        Through::Free => continue,
                        Through::Dropped => {}
                        Through::Out(msg) => out(In::Routed {
                            track: x.track,
                            msg,
                            frame,
                            via: r as u8,
                        }),
                    }
                    mask |= 1 << (ROUTE_BIT + r);
                }
                let routed = mask != 0;
                let shaped = match self.port_blocks[p].run(m) {
                    Through::Out(msg) if !routed => Some(msg),
                    _ => None,
                };
                if let Some(pm) = shaped {
                    for t in 0..MAX_TRACKS {
                        if !self.takes(t, port, ch) {
                            continue;
                        }
                        if let Some(tm) = pm.transposed(self.transpose(t)) {
                            mask |= 1 << t;
                            self.sent_note[hi * MAX_TRACKS + t] = tm.note().unwrap_or(note);
                            out(In::Track {
                                track: t as u8,
                                msg: tm,
                                frame,
                            });
                        }
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
                self.each_thru(mask, |r, x| {
                    if let Some(msg) = x.blocks.apply(m) {
                        out(In::Thru {
                            from: port,
                            to: x.to,
                            msg,
                            frame,
                            via: r,
                        })
                    }
                });
                self.each_route(mask, |r, x| {
                    if let Some(msg) = x.blocks.apply(m) {
                        out(In::Routed {
                            track: x.track,
                            msg,
                            frame,
                            via: r,
                        })
                    }
                });
                if let Some(pm) = self.port_blocks[p].apply(m) {
                    for t in 0..MAX_TRACKS {
                        if mask & (1 << t) != 0 {
                            let n = self.sent_note[hi * MAX_TRACKS + t];
                            out(In::Track {
                                track: t as u8,
                                msg: Msg::PolyPressure {
                                    ch: pm.channel(),
                                    note: n,
                                    value,
                                },
                                frame,
                            });
                        }
                    }
                }
            }
            _ => {
                let mut any = false;
                for (r, thru) in self.thru.iter().enumerate() {
                    let Some(x) = thru.filter(|x| x.takes(port, ch)) else {
                        continue;
                    };
                    match x.blocks.run(m) {
                        Through::Free => continue,
                        Through::Dropped => {}
                        Through::Out(msg) => out(In::Thru {
                            from: port,
                            to: x.to,
                            msg,
                            frame,
                            via: r as u8,
                        }),
                    }
                    any = true;
                }
                for (r, route) in self.routes.iter().enumerate() {
                    let Some(x) = route.filter(|x| x.takes(port, ch)) else {
                        continue;
                    };
                    match x.blocks.run(m) {
                        Through::Free => continue,
                        Through::Dropped => {}
                        Through::Out(msg) => out(In::Routed {
                            track: x.track,
                            msg,
                            frame,
                            via: r as u8,
                        }),
                    }
                    any = true;
                }
                if !any {
                    match self.port_blocks[p].run(m) {
                        Through::Out(pm) => {
                            for t in 0..MAX_TRACKS {
                                if self.takes(t, port, ch) {
                                    any = true;
                                    out(In::Track {
                                        track: t as u8,
                                        msg: pm,
                                        frame,
                                    });
                                }
                            }
                        }
                        Through::Dropped => any = true,
                        Through::Free => {}
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
        let off = Msg::NoteOff { ch, note, vel };
        self.each_thru(mask, |r, x| {
            if let Some(msg) = x.blocks.apply(off) {
                out(In::Thru {
                    from: port,
                    to: x.to,
                    msg,
                    frame,
                    via: r,
                })
            }
        });
        self.each_route(mask, |r, x| {
            if let Some(msg) = x.blocks.apply(off) {
                out(In::Routed {
                    track: x.track,
                    msg,
                    frame,
                    via: r,
                })
            }
        });
        self.end_tracks(hi, mask, port, ch, vel, frame, out);
    }

    /// Note-offs on the tracks whose bits are in `mask`, for the key at
    /// `hi`, through the port's blocks as its note-on went.
    #[allow(clippy::too_many_arguments)]
    fn end_tracks(
        &self,
        hi: usize,
        mask: u64,
        port: PortId,
        ch: Channel,
        vel: u8,
        frame: u64,
        out: &mut impl FnMut(In),
    ) {
        if mask & ((1u64 << MAX_TRACKS) - 1) == 0 {
            return;
        }
        let note = (hi % NOTES) as u8;
        let off = Msg::NoteOff { ch, note, vel };
        let ch = self
            .port_blocks(port)
            .apply(off)
            .map_or(ch, |m| m.channel());
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

    /// Each route whose bit is in `mask`, with its place.
    fn each_route(&self, mask: u64, mut f: impl FnMut(u8, &Route)) {
        for (r, route) in self.routes.iter().enumerate() {
            if mask & (1 << (ROUTE_BIT + r)) != 0 {
                if let Some(route) = route {
                    f(r as u8, route);
                }
            }
        }
    }

    /// Each thru whose bit is in `mask`, with its place.
    fn each_thru(&self, mask: u64, mut f: impl FnMut(u8, &Thru)) {
        for (r, thru) in self.thru.iter().enumerate() {
            if mask & (1 << (THRU_BIT + r)) != 0 {
                if let Some(thru) = thru {
                    f(r as u8, thru);
                }
            }
        }
    }

    /// Note-offs for every key held on `port`'s channel `ch` (All notes
    /// off came on it).
    fn release_channel(&mut self, port: PortId, ch: Channel, frame: u64, out: &mut impl FnMut(In)) {
        for note in 0..NOTES as u8 {
            if self.held[held_index(port, ch, note)] != 0 {
                self.release(port, ch, note, 64, frame, out);
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
