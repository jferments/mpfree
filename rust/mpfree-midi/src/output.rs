//! MIDI out: tracks to ports and channels, and the bytes of one period.
//!
//! - [`OutputRouter`] sends each MIDI track's notes to its port and
//!   channel ([`OutRoute`]), and what thru passes on from an input port
//!   ([`OutputRouter::thru`]), and remembers what is sounding, so that a
//!   changed route, a stop or a panic ends exactly the notes that are on
//!   ([`OutputRouter::set_route`], [`OutputRouter::all_off`]).
//! - [`OutQueue`] holds one port's messages for one period, in frame
//!   order, with real-time bytes (clock) first among messages at the same
//!   frame, and writes them out as bytes, with running status if asked.
//! - [`DinPacer`] works out when each message really leaves the DIN jack:
//!   MIDI 1.0 runs at 31250 bit/s, 10 bits a byte, so a note takes about
//!   0.96 ms and a 16-note chord about 15 ms. It reports how late each
//!   message and each clock pulse will be, which is what the timing tests
//!   and the Dev page show.
//!
//! Nothing allocates after `new`.

use crate::msg::{Channel, Msg};
use crate::port::{PortId, MAX_PORTS};

/// Where one MIDI track plays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutRoute {
    pub port: PortId,
    pub channel: Channel,
}

/// MIDI tracks (the same as `mpfree_seq::MAX_TRACKS`).
pub const MAX_TRACKS: usize = 16;
/// Messages one port can take in one period.
pub const QUEUE: usize = 256;

/// One message at a frame offset in the period, and the track that sent
/// it (none for the clock).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timed {
    pub offset: u32,
    bytes: [u8; 3],
    len: u8,
    /// The sending track, or [`NO_TRACK`] or [`PAGE`], or [`THRU`] with
    /// the input port it came in on.
    track: u8,
}

/// [`Timed`]'s track when no track sent it.
const NO_TRACK: u8 = 0xFF;
/// [`Timed`]'s track when a port's page sent it (its test note or All
/// notes off).
const PAGE: u8 = 0xFE;
/// [`Timed`]'s track when the Sample tab's auto-sample of outside gear
/// sent it.
const SAMPLER: u8 = 0xFD;
/// [`Timed`]'s track when thru passed it on: this bit, with the input
/// port's number below it (tracks are below [`MAX_TRACKS`]).
const THRU: u8 = 0x80;
/// The input thru's bookkeeping names for controller mode's notes (the
/// MPC's own pads), so [`Timed`]'s track reads `THRU | CONTROLLER_FROM`.
const CONTROLLER_FROM: u8 = 0x7C;
/// [`Timed`]'s track when controller mode sent it.
const CONTROLLER: u8 = THRU | CONTROLLER_FROM;
const _: () = assert!(MAX_TRACKS <= THRU as usize && MAX_PORTS < CONTROLLER_FROM as usize);

impl Timed {
    pub fn msg(offset: u32, m: Msg) -> Timed {
        let (bytes, len) = m.encode();
        Timed {
            offset,
            bytes,
            len: len as u8,
            track: NO_TRACK,
        }
    }

    /// The same message, sent by `track` (for the MIDI monitor).
    pub fn by(self, track: u8) -> Timed {
        Timed { track, ..self }
    }

    /// The same message, sent from a port's page (for the MIDI monitor).
    pub fn from_page(self) -> Timed {
        Timed {
            track: PAGE,
            ..self
        }
    }

    /// The same message, sent by the Sample tab's auto-sample of outside
    /// gear (for the MIDI monitor).
    pub fn from_sampler(self) -> Timed {
        Timed {
            track: SAMPLER,
            ..self
        }
    }

    /// The same message, passed on by thru from the input `port` (for
    /// the MIDI monitor).
    pub fn thru(self, port: PortId) -> Timed {
        Timed {
            track: THRU | (port.0 & 0x7F),
            ..self
        }
    }

    /// The track that sent it, if one did.
    pub fn track(&self) -> Option<u8> {
        (self.track < THRU).then_some(self.track)
    }

    /// The input port thru passed it on from, if it did.
    pub fn thru_from(&self) -> Option<PortId> {
        (self.track >= THRU && ![NO_TRACK, PAGE, SAMPLER, CONTROLLER].contains(&self.track))
            .then_some(PortId(self.track & 0x7F))
    }

    /// Controller mode sent it (the MPC's pads and Q-Links).
    pub fn is_from_controller(&self) -> bool {
        self.track == CONTROLLER
    }

    /// A port's page sent it.
    pub fn is_from_page(&self) -> bool {
        self.track == PAGE
    }

    /// The Sample tab's auto-sample of outside gear sent it.
    pub fn is_from_sampler(&self) -> bool {
        self.track == SAMPLER
    }

    /// One real-time byte (F8 clock, FA start, FB continue, FC stop).
    pub fn realtime(offset: u32, byte: u8) -> Timed {
        Timed {
            offset,
            bytes: [byte, 0, 0],
            len: 1,
            track: NO_TRACK,
        }
    }

    /// Up to three raw bytes (Song Position from `mpfree_midi_clock`).
    pub fn raw(offset: u32, b: &[u8]) -> Timed {
        let mut bytes = [0; 3];
        let len = b.len().min(3);
        bytes[..len].copy_from_slice(&b[..len]);
        Timed {
            offset,
            bytes,
            len: len as u8,
            track: NO_TRACK,
        }
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes[..usize::from(self.len)]
    }

    pub fn is_realtime(&self) -> bool {
        self.len == 1 && self.bytes[0] >= 0xF8
    }

    /// MPFree's own clock: a pulse, Start, Continue, Stop or Song Position
    /// no track, page or thru sent.
    pub fn is_clock(&self) -> bool {
        self.track == NO_TRACK && matches!(self.bytes[0], 0xF2 | 0xF8 | 0xFA | 0xFB | 0xFC)
    }
}

/// One port's messages for one period.
#[derive(Clone, Debug)]
pub struct OutQueue {
    buf: [Timed; QUEUE],
    len: usize,
    /// Messages that did not fit, since it was made.
    pub dropped: u64,
}

impl Default for OutQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl OutQueue {
    pub fn new() -> OutQueue {
        OutQueue {
            buf: [Timed::realtime(0, 0xF8); QUEUE],
            len: 0,
            dropped: 0,
        }
    }

    pub fn clear(&mut self) {
        self.len = 0;
    }

    pub fn push(&mut self, t: Timed) {
        if self.len < QUEUE {
            self.buf[self.len] = t;
            self.len += 1;
        } else {
            self.dropped += 1;
        }
    }

    /// In frame order; real-time bytes first among equal frames; otherwise
    /// the order they were pushed (a stable insertion sort, no allocation).
    pub fn sort(&mut self) {
        self.sort_by(|t| (t.offset, !t.is_realtime()));
    }

    /// In frame order only, in the order they were pushed among equal
    /// frames. For a caller that pushes the clock first, where the order
    /// matters: Stop, Song Position, Continue, then the pulse (Continue is
    /// a real-time byte and Song Position isn't, so [`OutQueue::sort`]
    /// would swap them).
    pub fn sort_by_frame(&mut self) {
        self.sort_by(|t| (t.offset, false));
    }

    /// In the order they are due: each at its frame plus `shift(t)` frames
    /// (a port's clock nudged later or earlier), in the order they are now
    /// among equal ones. After [`OutQueue::sort_by_frame`].
    pub fn sort_by_due(&mut self, shift: impl Fn(&Timed) -> i64) {
        let key = |t: &Timed| i64::from(t.offset) + shift(t);
        for i in 1..self.len {
            let mut j = i;
            while j > 0 && key(&self.buf[j - 1]) > key(&self.buf[j]) {
                self.buf.swap(j - 1, j);
                j -= 1;
            }
        }
    }

    fn sort_by(&mut self, key: impl Fn(&Timed) -> (u32, bool)) {
        for i in 1..self.len {
            let mut j = i;
            while j > 0 && key(&self.buf[j - 1]) > key(&self.buf[j]) {
                self.buf.swap(j - 1, j);
                j -= 1;
            }
        }
    }

    pub fn as_slice(&self) -> &[Timed] {
        &self.buf[..self.len]
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The bytes, in queue order, into `out`; returns how many. With
    /// `running_status` a channel message whose status byte equals the
    /// last one sent leaves it out (real-time bytes do not change it;
    /// anything else resets it). `last` carries the running status from
    /// one period to the next. Stops early if `out` is full.
    pub fn write_bytes(&self, out: &mut [u8], running_status: bool, last: &mut u8) -> usize {
        let mut n = 0;
        for t in self.as_slice() {
            let b = t.bytes();
            let skip = running_status && b[0] < 0xF0 && b[0] == *last;
            let part = if skip { &b[1..] } else { b };
            if n + part.len() > out.len() {
                break;
            }
            out[n..n + part.len()].copy_from_slice(part);
            n += part.len();
            if b[0] < 0xF0 {
                *last = b[0];
            } else if b[0] < 0xF8 {
                *last = 0;
            }
        }
        n
    }
}

/// When each message leaves a 31250 bit/s DIN jack.
#[derive(Clone, Debug)]
pub struct DinPacer {
    /// Frames per byte on the wire, as a fraction: rate * 10 / 31250.
    frames_per_byte: f64,
    /// When the wire is free again, in frames from the current period's
    /// start (can carry into the next period).
    busy_until: f64,
    last_status: u8,
    pub running_status: bool,
    pub stats: PaceStats,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PaceStats {
    pub messages: u64,
    /// The most any message left after its frame, in frames.
    pub max_late: f64,
    /// The most a clock pulse (F8) left after its frame, in frames.
    pub max_clock_late: f64,
}

/// The DIN bit rate.
pub const DIN_BAUD: f64 = 31250.0;

impl DinPacer {
    pub fn new(rate: u32) -> DinPacer {
        DinPacer {
            frames_per_byte: f64::from(rate) * 10.0 / DIN_BAUD,
            busy_until: 0.0,
            last_status: 0,
            running_status: true,
            stats: PaceStats::default(),
        }
    }

    pub fn frames_per_byte(&self) -> f64 {
        self.frames_per_byte
    }

    /// Paces one sorted period and calls `f(message, frame it starts
    /// leaving)` for each. Then moves on by `period` frames.
    pub fn pace(&mut self, q: &OutQueue, period: u32, mut f: impl FnMut(&Timed, f64)) {
        for t in q.as_slice() {
            let b = t.bytes();
            let skip = self.running_status && b[0] < 0xF0 && b[0] == self.last_status;
            let len = b.len() - usize::from(skip);
            if b[0] < 0xF0 {
                self.last_status = b[0];
            } else if b[0] < 0xF8 {
                self.last_status = 0;
            }
            let start = self.busy_until.max(f64::from(t.offset));
            let late = start - f64::from(t.offset);
            self.stats.messages += 1;
            self.stats.max_late = self.stats.max_late.max(late);
            if b[0] == 0xF8 {
                self.stats.max_clock_late = self.stats.max_clock_late.max(late);
            }
            self.busy_until = start + len as f64 * self.frames_per_byte;
            f(t, start);
        }
        self.busy_until = (self.busy_until - f64::from(period)).max(0.0);
    }

    /// Frames still being sent from earlier periods.
    pub fn backlog(&self) -> f64 {
        self.busy_until
    }
}

const NOTES: usize = 128;

/// MIDI tracks to ports, with what is sounding on each port and channel.
pub struct OutputRouter {
    routes: [Option<OutRoute>; MAX_TRACKS],
    /// Note-ons not yet ended at the track's current route, per track,
    /// channel and note (a count, as a track may play the same note
    /// twice). A route change ends them all first, so the route's port is
    /// all the rest it needs; the channel is kept, as a kit pad may play
    /// on a channel of its own ([`OutputRouter::send_on`]).
    sounding: Box<[u8]>,
    /// Notes on per track, so a panic skips the tracks with none.
    track_on: [u32; MAX_TRACKS],
    /// Thru's note-ons not yet ended, per output port, channel and note.
    thru_sounding: Box<[u8]>,
    /// The input port each of thru's notes came in on, for the monitor's
    /// line when a panic ends it.
    thru_src: Box<[u8]>,
    /// Thru's notes on per output port, so a panic skips the ports with
    /// none.
    thru_on: [u32; MAX_PORTS],
    pub dropped: u64,
}

impl Default for OutputRouter {
    fn default() -> Self {
        Self::new()
    }
}

const CHANNELS: usize = 16;

fn idx(track: usize, ch: Channel, note: u8) -> usize {
    (track * CHANNELS + usize::from(ch & 0x0F)) * NOTES + usize::from(note & 0x7F)
}

fn thru_idx(port: usize, ch: Channel, note: u8) -> usize {
    (port * CHANNELS + usize::from(ch & 0x0F)) * NOTES + usize::from(note & 0x7F)
}

impl OutputRouter {
    pub fn new() -> OutputRouter {
        OutputRouter {
            routes: [None; MAX_TRACKS],
            sounding: vec![0u8; MAX_TRACKS * CHANNELS * NOTES].into_boxed_slice(),
            track_on: [0; MAX_TRACKS],
            thru_sounding: vec![0u8; MAX_PORTS * CHANNELS * NOTES].into_boxed_slice(),
            thru_src: vec![0u8; MAX_PORTS * CHANNELS * NOTES].into_boxed_slice(),
            thru_on: [0; MAX_PORTS],
            dropped: 0,
        }
    }

    pub fn route(&self, track: u8) -> Option<OutRoute> {
        self.routes.get(usize::from(track)).copied().flatten()
    }

    /// A track's port and channel (None: it sends nothing). Notes it has
    /// on at the old route are ended there, at `offset`.
    pub fn set_route(
        &mut self,
        track: u8,
        route: Option<OutRoute>,
        offset: u32,
        queues: &mut [OutQueue],
    ) {
        let t = usize::from(track);
        if t >= MAX_TRACKS || self.routes[t] == route {
            return;
        }
        self.track_off(t, offset, queues);
        self.routes[t] = route.filter(|r| usize::from(r.port.0) < MAX_PORTS);
    }

    /// One message from a track (a note from a clip, the live pads, the
    /// arp), at `offset` in the period. Its channel is replaced by the
    /// route's. `queues` is indexed by port.
    pub fn send(&mut self, track: u8, msg: Msg, offset: u32, queues: &mut [OutQueue]) {
        self.send_on(track, msg, None, offset, queues);
    }

    /// [`OutputRouter::send`], on `channel` (0 to 15) rather than the
    /// route's when there is one: a kit pad the port's profile gives a
    /// channel of its own. A note ends on the channel it started on.
    pub fn send_on(
        &mut self,
        track: u8,
        msg: Msg,
        channel: Option<Channel>,
        offset: u32,
        queues: &mut [OutQueue],
    ) {
        let t = usize::from(track);
        let Some(r) = self.routes.get(t).copied().flatten() else {
            return;
        };
        let Some(q) = queues.get_mut(usize::from(r.port.0)) else {
            self.dropped += 1;
            return;
        };
        let ch = channel.map_or(r.channel, |c| c & 0x0F);
        let m = msg.on_channel(ch);
        match m {
            Msg::NoteOn { note, .. } => {
                let c = &mut self.sounding[idx(t, ch, note)];
                if *c < u8::MAX {
                    *c += 1;
                    self.track_on[t] += 1;
                }
            }
            Msg::NoteOff { note, .. } => {
                let c = &mut self.sounding[idx(t, ch, note)];
                if *c == 0 {
                    return; // never started here (route changed meanwhile)
                }
                *c -= 1;
                self.track_on[t] -= 1;
            }
            _ => {}
        }
        q.push(Timed::msg(offset, m).by(track));
    }

    /// A message thru passes on from the input port `from` to the output
    /// port `to`, as it came, at `offset`. Its notes are kept, so a panic
    /// or the port's All notes off ends them; a note-off for a
    /// note not on there (ended by a panic, or sent while MIDI out was
    /// off) goes nowhere.
    pub fn thru(
        &mut self,
        from: PortId,
        to: PortId,
        msg: Msg,
        offset: u32,
        queues: &mut [OutQueue],
    ) {
        let p = usize::from(to.0);
        let Some(q) = queues.get_mut(p).filter(|_| p < MAX_PORTS) else {
            self.dropped += 1;
            return;
        };
        match msg {
            Msg::NoteOn { ch, note, .. } => {
                let i = thru_idx(p, ch, note);
                self.thru_src[i] = from.0;
                let c = &mut self.thru_sounding[i];
                if *c < u8::MAX {
                    *c += 1;
                    self.thru_on[p] += 1;
                }
            }
            Msg::NoteOff { ch, note, .. } => {
                let c = &mut self.thru_sounding[thru_idx(p, ch, note)];
                if *c == 0 {
                    return;
                }
                *c -= 1;
                self.thru_on[p] -= 1;
            }
            _ => {}
        }
        q.push(Timed::msg(offset, msg).thru(from));
    }

    /// A message controller mode sends to the output port `to` (the MPC's
    /// pads and Q-Links), at `offset`. Its notes are kept as thru's are,
    /// so a panic or the port's All notes off ends them.
    pub fn controller(&mut self, to: PortId, msg: Msg, offset: u32, queues: &mut [OutQueue]) {
        self.thru(PortId(CONTROLLER_FROM), to, msg, offset, queues);
    }

    /// Ends thru's notes on port `p`, at `offset`.
    fn thru_off(&mut self, p: usize, offset: u32, queues: &mut [OutQueue]) {
        let Some(q) = queues.get_mut(p).filter(|_| p < MAX_PORTS) else {
            return;
        };
        if std::mem::take(&mut self.thru_on[p]) == 0 {
            return;
        }
        for ch in 0..CHANNELS as Channel {
            for note in 0..NOTES as u8 {
                let i = thru_idx(p, ch, note);
                let n = std::mem::take(&mut self.thru_sounding[i]);
                for _ in 0..n {
                    let off = Msg::NoteOff { ch, note, vel: 64 };
                    q.push(Timed::msg(offset, off).thru(PortId(self.thru_src[i])));
                }
            }
        }
    }

    fn track_off(&mut self, t: usize, offset: u32, queues: &mut [OutQueue]) {
        let Some(r) = self.routes[t] else { return };
        if std::mem::take(&mut self.track_on[t]) == 0 {
            return;
        }
        for ch in 0..CHANNELS as Channel {
            for note in 0..NOTES as u8 {
                let n = std::mem::take(&mut self.sounding[idx(t, ch, note)]);
                if let Some(q) = queues.get_mut(usize::from(r.port.0)) {
                    for _ in 0..n {
                        let off = Msg::NoteOff { ch, note, vel: 64 };
                        q.push(Timed::msg(offset, off).by(t as u8));
                    }
                }
            }
        }
    }

    /// Ends the sounding notes of every track that plays on `port`, and
    /// thru's there, at `offset`: the port went away, or its page's All
    /// notes off.
    pub fn port_off(&mut self, port: PortId, offset: u32, queues: &mut [OutQueue]) {
        for t in 0..MAX_TRACKS {
            if self.routes[t].is_some_and(|r| r.port == port) {
                self.track_off(t, offset, queues);
            }
        }
        self.thru_off(usize::from(port.0), offset, queues);
    }

    /// Ends every sounding note of every track (stop, panic), at `offset`.
    /// Only notes that are on are sent, not 2048 blind note-offs, so the
    /// DIN jack is not busy for 2 s.
    pub fn all_off(&mut self, offset: u32, queues: &mut [OutQueue]) {
        for t in 0..MAX_TRACKS {
            self.track_off(t, offset, queues);
        }
        for p in 0..MAX_PORTS {
            self.thru_off(p, offset, queues);
        }
    }

    /// Notes on now, over every track and thru.
    pub fn sounding(&self) -> usize {
        self.sounding
            .iter()
            .chain(self.thru_sounding.iter())
            .map(|&c| usize::from(c))
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn queues() -> Vec<OutQueue> {
        (0..MAX_PORTS).map(|_| OutQueue::new()).collect()
    }

    #[test]
    fn sort_by_frame_keeps_the_order_pushed() {
        let mut q = OutQueue::new();
        q.push(Timed::msg(
            20,
            Msg::NoteOn {
                ch: 0,
                note: 60,
                vel: 100,
            },
        ));
        q.push(Timed::raw(10, &[0xF2, 16, 0]));
        q.push(Timed::realtime(10, 0xFB));
        q.push(Timed::realtime(10, 0xF8));
        q.sort_by_frame();
        let firsts: Vec<u8> = q.as_slice().iter().map(|t| t.bytes()[0]).collect();
        assert_eq!(firsts, [0xF2, 0xFB, 0xF8, 0x90]);
        // The clock nudged 15 frames later: after the note, in its order.
        let mut later = q.clone();
        later.sort_by_due(|t| if t.is_clock() { 15 } else { 0 });
        let firsts: Vec<u8> = later.as_slice().iter().map(|t| t.bytes()[0]).collect();
        assert_eq!(firsts, [0x90, 0xF2, 0xFB, 0xF8]);
        // A track's or thru's F8 isn't MPFree's clock.
        assert!(!Timed::realtime(0, 0xF8).by(1).is_clock());
        assert!(!Timed::realtime(0, 0xF8).thru(PortId(2)).is_clock());
        assert!(Timed::realtime(0, 0xFC).is_clock());
        assert!(!Timed::realtime(0, 0xFE).is_clock());
    }

    #[test]
    fn route_change_ends_the_old_notes_there() {
        let mut r = OutputRouter::new();
        let mut q = queues();
        r.set_route(
            2,
            Some(OutRoute {
                port: PortId(0),
                channel: 0,
            }),
            0,
            &mut q,
        );
        r.send(
            2,
            Msg::NoteOn {
                ch: 9,
                note: 60,
                vel: 100,
            },
            5,
            &mut q,
        );
        r.set_route(
            2,
            Some(OutRoute {
                port: PortId(1),
                channel: 3,
            }),
            10,
            &mut q,
        );
        // The key comes up after the change: nothing more to send for it.
        r.send(
            2,
            Msg::NoteOff {
                ch: 0,
                note: 60,
                vel: 0,
            },
            20,
            &mut q,
        );
        let b: Vec<Vec<u8>> = q[0].as_slice().iter().map(|t| t.bytes().to_vec()).collect();
        assert_eq!(b, vec![vec![0x90, 60, 100], vec![0x80, 60, 64]]);
        // Both say which track sent them (the MIDI monitor's "sent by").
        assert!(q[0].as_slice().iter().all(|t| t.track() == Some(2)));
        assert_eq!(Timed::realtime(0, 0xF8).track(), None);
        assert!(q[1].is_empty());
        assert_eq!(r.sounding(), 0);
    }

    /// A kit pad on a channel of its own ends there, by its note-off, a
    /// panic or a route change, however many share its note.
    #[test]
    fn a_note_on_a_channel_of_its_own_ends_there() {
        let mut r = OutputRouter::new();
        let mut q = queues();
        let route = |ch| {
            Some(OutRoute {
                port: PortId(0),
                channel: ch,
            })
        };
        r.set_route(0, route(1), 0, &mut q);
        let on = Msg::NoteOn {
            ch: 0,
            note: 36,
            vel: 100,
        };
        let off = Msg::NoteOff {
            ch: 0,
            note: 36,
            vel: 64,
        };
        r.send_on(0, on, Some(9), 0, &mut q);
        r.send(0, on, 0, &mut q);
        r.send_on(0, off, Some(9), 1, &mut q);
        // A note-off on a channel it never started on goes nowhere.
        r.send_on(0, off, Some(4), 1, &mut q);
        r.send_on(0, on, Some(9), 2, &mut q);
        let bytes = |q: &[OutQueue]| -> Vec<Vec<u8>> {
            q[0].as_slice().iter().map(|t| t.bytes().to_vec()).collect()
        };
        assert_eq!(
            bytes(&q),
            vec![
                vec![0x99, 36, 100],
                vec![0x91, 36, 100],
                vec![0x89, 36, 64],
                vec![0x99, 36, 100],
            ]
        );
        assert_eq!(r.sounding(), 2);
        q[0].clear();
        // A new channel for the track ends both where they sound.
        r.set_route(0, route(2), 3, &mut q);
        let mut ended = bytes(&q);
        ended.sort();
        assert_eq!(ended, vec![vec![0x81, 36, 64], vec![0x89, 36, 64]]);
        assert_eq!(r.sounding(), 0);
        q[0].clear();
        r.send_on(0, on, Some(15), 4, &mut q);
        q[0].clear();
        r.all_off(5, &mut q);
        assert_eq!(bytes(&q), vec![vec![0x8F, 36, 64]]);
        assert_eq!(r.sounding(), 0);
    }

    #[test]
    fn all_off_sends_only_what_is_on() {
        let mut r = OutputRouter::new();
        let mut q = queues();
        r.set_route(
            0,
            Some(OutRoute {
                port: PortId(0),
                channel: 1,
            }),
            0,
            &mut q,
        );
        r.set_route(
            1,
            Some(OutRoute {
                port: PortId(0),
                channel: 2,
            }),
            0,
            &mut q,
        );
        r.send(
            0,
            Msg::NoteOn {
                ch: 0,
                note: 40,
                vel: 1,
            },
            0,
            &mut q,
        );
        r.send(
            0,
            Msg::NoteOn {
                ch: 0,
                note: 40,
                vel: 1,
            },
            1,
            &mut q,
        );
        r.send(
            1,
            Msg::NoteOn {
                ch: 0,
                note: 41,
                vel: 1,
            },
            2,
            &mut q,
        );
        q[0].clear();
        r.all_off(7, &mut q);
        let b: Vec<Vec<u8>> = q[0].as_slice().iter().map(|t| t.bytes().to_vec()).collect();
        assert_eq!(
            b,
            vec![vec![0x81, 40, 64], vec![0x81, 40, 64], vec![0x82, 41, 64]]
        );
        assert_eq!(r.sounding(), 0);
    }

    #[test]
    fn clock_goes_first_and_running_status_saves_bytes() {
        let mut q = OutQueue::new();
        q.push(Timed::msg(
            10,
            Msg::NoteOn {
                ch: 0,
                note: 60,
                vel: 90,
            },
        ));
        q.push(Timed::msg(
            10,
            Msg::NoteOn {
                ch: 0,
                note: 64,
                vel: 90,
            },
        ));
        q.push(Timed::realtime(10, 0xF8));
        q.push(Timed::msg(
            3,
            Msg::Cc {
                ch: 0,
                cc: 1,
                value: 2,
            },
        ));
        q.sort();
        let offs: Vec<u32> = q.as_slice().iter().map(|t| t.offset).collect();
        assert_eq!(offs, vec![3, 10, 10, 10]);
        assert!(q.as_slice()[1].is_realtime());
        let mut out = [0u8; 16];
        let mut last = 0;
        let n = q.write_bytes(&mut out, true, &mut last);
        assert_eq!(&out[..n], &[0xB0, 1, 2, 0xF8, 0x90, 60, 90, 64, 90]);
        assert_eq!(last, 0x90);
        let mut last = 0;
        assert_eq!(q.write_bytes(&mut out, false, &mut last), 10);
    }

    /// A port's notes end alone: another port's go on.
    #[test]
    fn port_off_ends_only_that_ports_notes() {
        let mut r = OutputRouter::new();
        let mut qs = [OutQueue::new(), OutQueue::new()];
        let on = |note| Msg::NoteOn {
            ch: 0,
            note,
            vel: 100,
        };
        let usb = PortId(1);
        let din = crate::port::DIN;
        r.set_route(
            0,
            Some(OutRoute {
                port: din,
                channel: 0,
            }),
            0,
            &mut qs,
        );
        r.set_route(
            1,
            Some(OutRoute {
                port: usb,
                channel: 3,
            }),
            0,
            &mut qs,
        );
        r.send(0, on(60), 0, &mut qs);
        r.send(1, on(62), 0, &mut qs);
        r.send(1, on(64), 0, &mut qs);
        for q in qs.iter_mut() {
            q.clear();
        }
        r.port_off(usb, 7, &mut qs);
        assert!(qs[0].is_empty());
        let offs: Vec<&[u8]> = qs[1].as_slice().iter().map(Timed::bytes).collect();
        assert_eq!(offs, [&[0x83, 62, 64][..], &[0x83, 64, 64][..]]);
        assert!(qs[1].as_slice().iter().all(|t| t.offset == 7));
        assert_eq!(r.sounding(), 1, "the jacks' note is still on");
    }

    #[test]
    fn din_chord_takes_about_a_millisecond_a_note() {
        let rate = 44_100;
        let mut p = DinPacer::new(rate);
        p.running_status = false;
        let mut q = OutQueue::new();
        for n in 0..16 {
            q.push(Timed::msg(
                0,
                Msg::NoteOn {
                    ch: 0,
                    note: 48 + n,
                    vel: 100,
                },
            ));
        }
        q.push(Timed::realtime(64, 0xF8));
        q.sort();
        let mut starts = Vec::new();
        p.pace(&q, 128, |_, s| starts.push(s));
        let fpb = f64::from(rate) * 10.0 / 31250.0; // 14.1 frames a byte
        assert!((starts[15] - 15.0 * 3.0 * fpb).abs() < 1e-9);
        // The clock pulse at 64 waits for the chord: 16 notes x 3 bytes.
        let chord_end = 16.0 * 3.0 * fpb;
        assert!((p.stats.max_clock_late - (chord_end - 64.0)).abs() < 1e-9);
        // About 15.4 ms: most of it carries into the next periods.
        assert!((chord_end + fpb - 128.0 - p.backlog()).abs() < 1e-9);
    }
}
