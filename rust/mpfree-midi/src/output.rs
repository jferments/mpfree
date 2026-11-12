//! MIDI out: tracks to ports and channels, and the bytes of one period.
//!
//! - [`OutputRouter`] sends each MIDI track's notes to its port and
//!   channel ([`OutRoute`]) and remembers what is sounding, so that a
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

/// One message at a frame offset in the period.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timed {
    pub offset: u32,
    bytes: [u8; 3],
    len: u8,
}

impl Timed {
    pub fn msg(offset: u32, m: Msg) -> Timed {
        let (bytes, len) = m.encode();
        Timed {
            offset,
            bytes,
            len: len as u8,
        }
    }

    /// One real-time byte (F8 clock, FA start, FB continue, FC stop).
    pub fn realtime(offset: u32, byte: u8) -> Timed {
        Timed {
            offset,
            bytes: [byte, 0, 0],
            len: 1,
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
        }
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes[..usize::from(self.len)]
    }

    pub fn is_realtime(&self) -> bool {
        self.len == 1 && self.bytes[0] >= 0xF8
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
        let key = |t: &Timed| (t.offset, !t.is_realtime());
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
    /// Note-ons not yet ended at the track's current route, per track and
    /// note (a count, as a track may play the same note twice). A route
    /// change ends them all first, so the route is all the rest it needs.
    sounding: Box<[u8]>,
    pub dropped: u64,
}

impl Default for OutputRouter {
    fn default() -> Self {
        Self::new()
    }
}

fn idx(track: usize, note: u8) -> usize {
    track * NOTES + usize::from(note & 0x7F)
}

impl OutputRouter {
    pub fn new() -> OutputRouter {
        OutputRouter {
            routes: [None; MAX_TRACKS],
            sounding: vec![0u8; MAX_TRACKS * NOTES].into_boxed_slice(),
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
        let t = usize::from(track);
        let Some(r) = self.routes.get(t).copied().flatten() else {
            return;
        };
        let Some(q) = queues.get_mut(usize::from(r.port.0)) else {
            self.dropped += 1;
            return;
        };
        let m = msg.on_channel(r.channel);
        match m {
            Msg::NoteOn { note, .. } => {
                let c = &mut self.sounding[idx(t, note)];
                *c = c.saturating_add(1);
            }
            Msg::NoteOff { note, .. } => {
                let c = &mut self.sounding[idx(t, note)];
                if *c == 0 {
                    return; // never started here (route changed meanwhile)
                }
                *c -= 1;
            }
            _ => {}
        }
        q.push(Timed::msg(offset, m));
    }

    fn track_off(&mut self, t: usize, offset: u32, queues: &mut [OutQueue]) {
        let Some(r) = self.routes[t] else { return };
        for note in 0..NOTES as u8 {
            let i = idx(t, note);
            let n = std::mem::take(&mut self.sounding[i]);
            if let Some(q) = queues.get_mut(usize::from(r.port.0)) {
                for _ in 0..n {
                    q.push(Timed::msg(
                        offset,
                        Msg::NoteOff {
                            ch: r.channel,
                            note,
                            vel: 64,
                        },
                    ));
                }
            }
        }
    }

    /// Ends every sounding note of every track (stop, panic), at `offset`.
    /// Only notes that are on are sent, not 2048 blind note-offs, so the
    /// DIN jack is not busy for 2 s.
    pub fn all_off(&mut self, offset: u32, queues: &mut [OutQueue]) {
        for t in 0..MAX_TRACKS {
            self.track_off(t, offset, queues);
        }
    }

    /// Notes on now, over every track.
    pub fn sounding(&self) -> usize {
        self.sounding.iter().map(|&c| usize::from(c)).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn queues() -> Vec<OutQueue> {
        (0..MAX_PORTS).map(|_| OutQueue::new()).collect()
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
        assert!(q[1].is_empty());
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
