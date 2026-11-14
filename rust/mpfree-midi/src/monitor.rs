//! The MIDI monitor's model: every message in and out of the external
//! ports, as values, for the I/O view's Monitor section.
//!
//! Two halves, so the side that reads and writes the ports never waits for
//! the screen:
//!
//! - [`Feed`] lives where the bytes are. The bytes of each port in
//!   ([`Dir::In`]) and out ([`Dir::Out`]) become [`Seen`] values in a fixed
//!   queue: no allocation and no lock, so it can run in the audio loop.
//!   Clock, active sensing and time code come many times a second, so they
//!   are counted, not listed: one line when each starts arriving and one
//!   when it stops, and the clock's tempo.
//! - [`Monitor`] is what the screen shows: the newest [`KEEP`] messages,
//!   when each port's channels last had something (the activity lights),
//!   the tempo of each clock arriving or leaving, and the ports' names. It
//!   sits in the I/O view's model; [`Feed::drain_into`] moves the queue
//!   over while the app holds that model's lock.
//!
//! Times are seconds on the app's clock, the one the I/O view's tick gets.

use mpfree_controller::midi::{Message, Parser};

use crate::msg::{Channel, Msg};
use crate::port::{PortId, PortInfo, PortKind, DIN, MAX_PORTS};

/// How many messages the monitor keeps (the oldest go first).
pub const KEEP: usize = 1000;
/// How many messages a [`Feed`] holds between two drains. More are
/// counted in [`Monitor::lost`].
pub const QUEUE: usize = 512;
/// How long a channel's light stays lit after a message, in seconds.
pub const LIGHT_S: f64 = 0.25;
/// Clock, active sensing and time code have stopped arriving after this
/// long without a byte, in seconds. (Clock at 20 BPM ticks every 0.125 s;
/// active sensing comes at least every 0.3 s.)
pub const GAP_S: f64 = 1.0;

/// Which way a message went through its port.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Dir {
    In,
    Out,
}

/// What comes many times a second, so it is counted, not listed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Steady {
    /// MIDI clock (F8), 24 a beat.
    Clock,
    /// Active sensing (FE).
    Sensing,
    /// MIDI time code quarter frames (F1).
    TimeCode,
}

impl Steady {
    const ALL: [Steady; 3] = [Steady::Clock, Steady::Sensing, Steady::TimeCode];

    /// Its status byte.
    pub fn status(self) -> u8 {
        match self {
            Steady::Clock => 0xF8,
            Steady::Sensing => 0xFE,
            Steady::TimeCode => 0xF1,
        }
    }
}

/// One message, as the monitor lists it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum What {
    /// A channel message, with its bytes as they came: a note-on at
    /// velocity 0 keeps its bytes here while `msg` reads it as a note-off.
    /// `len` is 2 or 3 (a running-status message gets its status back).
    Channel {
        msg: Msg,
        bytes: [u8; 3],
        len: u8,
    },
    Start,
    Continue,
    Stop,
    /// System reset (FF).
    Reset,
    /// Song position, in sixteenths from the start.
    SongPosition(u16),
    SongSelect(u8),
    TuneRequest,
    /// System exclusive: its whole length (F0 and F7 included) and the
    /// maker's id (one byte, or three when the first is 0).
    SysEx {
        len: u32,
        maker: [u8; 3],
    },
    /// Clock, active sensing or time code started arriving (or leaving).
    Arriving(Steady),
    /// It stopped, after this many.
    Stopped(Steady, u32),
    /// A status byte MIDI 1.0 leaves undefined (F4, F5, F9, FD).
    Undefined(u8),
}

/// The monitor's filter groups, as bits of [`Filter::kinds`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// Note on and off.
    Notes,
    /// Control changes (channel mode messages included).
    Controls,
    /// Program changes.
    Programs,
    /// Pitch bend and pressure (channel and poly).
    Bends,
    /// Clock, start, stop, song position and select, time code, active
    /// sensing.
    Clock,
    /// System exclusive, tune request and undefined bytes.
    System,
}

impl Kind {
    pub const ALL: [Kind; 6] = [
        Kind::Notes,
        Kind::Controls,
        Kind::Programs,
        Kind::Bends,
        Kind::Clock,
        Kind::System,
    ];

    pub fn bit(self) -> u8 {
        1 << self as u8
    }
}

impl What {
    pub fn kind(&self) -> Kind {
        match self {
            What::Channel { msg, .. } => match msg {
                Msg::NoteOn { .. } | Msg::NoteOff { .. } => Kind::Notes,
                Msg::Cc { .. } => Kind::Controls,
                Msg::Program { .. } => Kind::Programs,
                Msg::PolyPressure { .. } | Msg::ChanPressure { .. } | Msg::PitchBend { .. } => {
                    Kind::Bends
                }
            },
            What::Start
            | What::Continue
            | What::Stop
            | What::Reset
            | What::SongPosition(_)
            | What::SongSelect(_)
            | What::Arriving(_)
            | What::Stopped(..) => Kind::Clock,
            What::TuneRequest | What::SysEx { .. } | What::Undefined(_) => Kind::System,
        }
    }

    /// The channel of a channel message.
    pub fn channel(&self) -> Option<Channel> {
        match self {
            What::Channel { msg, .. } => Some(msg.channel()),
            _ => None,
        }
    }

    /// The bytes it began with, as many as fit in three (a SysEx's F0 and
    /// maker; the status byte of the rest), and how many that is.
    pub fn bytes(&self) -> ([u8; 3], usize) {
        match *self {
            What::Channel { bytes, len, .. } => (bytes, usize::from(len)),
            What::Start => ([0xFA, 0, 0], 1),
            What::Continue => ([0xFB, 0, 0], 1),
            What::Stop => ([0xFC, 0, 0], 1),
            What::Reset => ([0xFF, 0, 0], 1),
            What::SongPosition(s) => ([0xF2, (s & 0x7F) as u8, (s >> 7) as u8 & 0x7F], 3),
            What::SongSelect(s) => ([0xF3, s & 0x7F, 0], 2),
            What::TuneRequest => ([0xF6, 0, 0], 1),
            What::SysEx { maker, .. } => {
                if maker[0] == 0 {
                    ([0xF0, 0, maker[1]], 3)
                } else {
                    ([0xF0, maker[0], 0], 2)
                }
            }
            What::Arriving(s) | What::Stopped(s, _) => ([s.status(), 0, 0], 1),
            What::Undefined(b) => ([b, 0, 0], 1),
        }
    }
}

/// A message the monitor saw: when, on which port, which way, and what.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Seen {
    pub t: f64,
    pub port: PortId,
    pub dir: Dir,
    pub what: What,
}

const EMPTY: Seen = Seen {
    t: 0.0,
    port: DIN,
    dir: Dir::In,
    what: What::Start,
};

/// Each port both ways: in first, then out.
const STREAMS: usize = 2 * MAX_PORTS;

fn stream(port: PortId, dir: Dir) -> Option<usize> {
    let p = usize::from(port.0);
    (p < MAX_PORTS).then_some(match dir {
        Dir::In => p,
        Dir::Out => MAX_PORTS + p,
    })
}

#[derive(Clone, Copy, Debug, Default)]
struct SteadyState {
    running: bool,
    count: u32,
    last: f64,
}

/// Measures a clock's tempo over windows of at least a beat and half a
/// second, smoothed.
#[derive(Clone, Copy, Debug, Default)]
struct Tempo {
    from: f64,
    ticks: u32,
    bpm: f32,
}

impl Tempo {
    fn start(&mut self, t: f64) {
        *self = Tempo {
            from: t,
            ticks: 0,
            bpm: 0.0,
        };
    }

    fn tick(&mut self, t: f64) {
        self.ticks += 1;
        let span = t - self.from;
        if self.ticks >= 24 && span >= 0.5 {
            // 24 ticks a beat.
            let now = (2.5 * f64::from(self.ticks) / span) as f32;
            self.bpm = if self.bpm > 0.0 {
                self.bpm + 0.5 * (now - self.bpm)
            } else {
                now
            };
            self.from = t;
            self.ticks = 0;
        }
    }
}

#[derive(Clone, Debug)]
struct Stream {
    parser: Parser,
    /// Bytes of the SysEx under way, F0 included; 0 when none is.
    sysex: u32,
    steady: [SteadyState; 3],
    tempo: Tempo,
}

impl Stream {
    fn new() -> Stream {
        Stream {
            parser: Parser::new(),
            sysex: 0,
            steady: [SteadyState::default(); 3],
            tempo: Tempo::default(),
        }
    }
}

/// The ports' bytes into [`Seen`] values: where the bytes are (see the
/// module docs). One per thread that reads or writes ports; each drains
/// into the same [`Monitor`].
pub struct Feed {
    streams: Box<[Stream]>,
    queue: Box<[Seen]>,
    queued: usize,
    lost: u64,
}

impl Default for Feed {
    fn default() -> Self {
        Self::new()
    }
}

impl Feed {
    pub fn new() -> Feed {
        Feed {
            streams: (0..STREAMS).map(|_| Stream::new()).collect(),
            queue: vec![EMPTY; QUEUE].into_boxed_slice(),
            queued: 0,
            lost: 0,
        }
    }

    fn push(&mut self, s: Seen) {
        if self.queued < QUEUE {
            self.queue[self.queued] = s;
            self.queued += 1;
        } else {
            self.lost += 1;
        }
    }

    /// Bytes that came in on (`Dir::In`) or went out of (`Dir::Out`)
    /// `port` at `t` seconds. A port past [`MAX_PORTS`] is ignored.
    pub fn bytes(&mut self, port: PortId, dir: Dir, t: f64, data: &[u8]) {
        let Some(s) = stream(port, dir) else {
            return;
        };
        for &b in data {
            self.byte(s, port, dir, t, b);
        }
    }

    fn byte(&mut self, s: usize, port: PortId, dir: Dir, t: f64, b: u8) {
        let st = &mut self.streams[s];
        // SysEx length, counted here (the parser keeps only the start).
        match b {
            0xF0 => st.sysex = 1,
            0xF7 if st.sysex > 0 => st.sysex += 1,
            0x00..=0x7F if st.sysex > 0 => st.sysex += 1,
            0xF8..=0xFF => {}
            _ => st.sysex = 0,
        }
        let len = st.sysex;
        let what = match st.parser.feed(b) {
            None => None,
            Some(Message::Channel {
                status,
                data1,
                data2,
            }) => Msg::from_bytes(status, data1, data2).map(|msg| {
                let len = if matches!(status & 0xF0, 0xC0 | 0xD0) {
                    2
                } else {
                    3
                };
                What::Channel {
                    msg,
                    bytes: [status, data1, data2],
                    len,
                }
            }),
            Some(Message::SysEx { bytes, .. }) => {
                let maker = match bytes {
                    [_, 0, a, b, ..] => [0, *a, *b],
                    [_, m, ..] if *m != 0xF7 => [*m, 0, 0],
                    _ => [0x7F, 0, 0],
                };
                st.sysex = 0;
                Some(What::SysEx { len, maker })
            }
            Some(Message::RealTime(r)) => match r {
                0xF8 => {
                    st.tempo.tick(t);
                    self.steady(s, t, Steady::Clock)
                }
                0xFE => self.steady(s, t, Steady::Sensing),
                0xFA => Some(What::Start),
                0xFB => Some(What::Continue),
                0xFC => Some(What::Stop),
                0xFF => Some(What::Reset),
                other => Some(What::Undefined(other)),
            },
            Some(Message::Common {
                status,
                data1,
                data2,
            }) => match status {
                0xF1 => self.steady(s, t, Steady::TimeCode),
                0xF2 => Some(What::SongPosition(
                    u16::from(data1) | (u16::from(data2) << 7),
                )),
                0xF3 => Some(What::SongSelect(data1)),
                0xF6 => Some(What::TuneRequest),
                other => Some(What::Undefined(other)),
            },
        };
        if let Some(what) = what {
            self.push(Seen { t, port, dir, what });
        }
    }

    /// Counts one of `kind`; the line that it started arriving, the first
    /// time.
    fn steady(&mut self, s: usize, t: f64, kind: Steady) -> Option<What> {
        let st = &mut self.streams[s];
        let k = &mut st.steady[kind as usize];
        k.last = t;
        k.count = k.count.saturating_add(1);
        if k.running {
            return None;
        }
        k.running = true;
        k.count = 1;
        if kind == Steady::Clock {
            st.tempo.start(t);
        }
        Some(What::Arriving(kind))
    }

    /// Says which steady streams stopped by `t` (none for [`GAP_S`]).
    fn gaps(&mut self, t: f64) {
        for s in 0..STREAMS {
            let (port, dir) = if s < MAX_PORTS {
                (PortId(s as u8), Dir::In)
            } else {
                (PortId((s - MAX_PORTS) as u8), Dir::Out)
            };
            for kind in Steady::ALL {
                let k = &mut self.streams[s].steady[kind as usize];
                if k.running && t - k.last > GAP_S {
                    k.running = false;
                    let n = k.count;
                    self.push(Seen {
                        t,
                        port,
                        dir,
                        what: What::Stopped(kind, n),
                    });
                }
            }
        }
    }

    /// Moves everything seen so far into `m`, after saying which clocks,
    /// sensing and time code stopped by `t`. Call it while holding the
    /// I/O model's lock, as often as the screen is updated or more.
    pub fn drain_into(&mut self, m: &mut Monitor, t: f64) {
        self.gaps(t);
        for i in 0..self.queued {
            m.push(self.queue[i]);
        }
        self.queued = 0;
        m.lost += self.lost;
        self.lost = 0;
        for s in 0..STREAMS {
            let st = &self.streams[s];
            let running = st.steady[Steady::Clock as usize].running;
            m.bpm[s] = (running && st.tempo.bpm > 0.0).then_some(st.tempo.bpm);
        }
    }
}

/// What to list: a bit per [`Kind`], which ways, and one port or channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Filter {
    pub kinds: u8,
    /// Bit 0: in; bit 1: out.
    pub dirs: u8,
    pub port: Option<PortId>,
    /// Only this channel's messages (system messages have none).
    pub channel: Option<Channel>,
}

impl Default for Filter {
    fn default() -> Self {
        Filter::ALL
    }
}

impl Filter {
    /// Everything.
    pub const ALL: Filter = Filter {
        kinds: 0b11_1111,
        dirs: 0b11,
        port: None,
        channel: None,
    };

    pub fn dir_bit(d: Dir) -> u8 {
        match d {
            Dir::In => 1,
            Dir::Out => 2,
        }
    }

    pub fn shows_kind(&self, k: Kind) -> bool {
        self.kinds & k.bit() != 0
    }

    pub fn shows_dir(&self, d: Dir) -> bool {
        self.dirs & Filter::dir_bit(d) != 0
    }

    pub fn matches(&self, s: &Seen) -> bool {
        self.shows_kind(s.what.kind())
            && self.shows_dir(s.dir)
            && self.port.is_none_or(|p| p == s.port)
            && self.channel.is_none_or(|c| s.what.channel() == Some(c))
    }
}

/// What the Monitor section shows (see the module docs).
#[derive(Clone, Debug, PartialEq)]
pub struct Monitor {
    ring: Vec<Seen>,
    /// Where the next one goes.
    head: usize,
    total: u64,
    /// The newest one's number when the list was last cleared.
    cleared: u64,
    /// When each port's channels last had a message, each way.
    last: [[f64; 16]; STREAMS],
    /// When each port last had a message with no channel, each way.
    sys: [f64; STREAMS],
    bpm: [Option<f32>; STREAMS],
    ports: Vec<PortInfo>,
    /// Messages a [`Feed`] had no room for between two drains.
    pub lost: u64,
}

impl Default for Monitor {
    fn default() -> Self {
        Self::new()
    }
}

impl Monitor {
    /// Empty, with the 5-pin jacks as the only port.
    pub fn new() -> Monitor {
        Monitor {
            ring: Vec::with_capacity(KEEP),
            head: 0,
            total: 0,
            cleared: 0,
            last: [[f64::NEG_INFINITY; 16]; STREAMS],
            sys: [f64::NEG_INFINITY; STREAMS],
            bpm: [None; STREAMS],
            ports: crate::port::Ports::new().all().to_vec(),
            lost: 0,
        }
    }

    /// The ports as the app's table has them (names, which are plugged in,
    /// which have an input and an output). Not for the audio loop: it
    /// copies the names.
    pub fn set_ports(&mut self, ports: &[PortInfo]) {
        if self.ports != ports {
            self.ports = ports.to_vec();
        }
    }

    pub fn ports(&self) -> &[PortInfo] {
        &self.ports
    }

    /// The ports to show: the 5-pin jacks always, and every other one
    /// plugged in.
    pub fn shown_ports(&self) -> impl Iterator<Item = &PortInfo> {
        self.ports
            .iter()
            .filter(|p| p.kind == PortKind::Din || p.connected)
    }

    /// A port's name, if the table has it.
    pub fn port_name(&self, port: PortId) -> Option<&str> {
        self.ports
            .iter()
            .find(|p| p.id == port)
            .map(|p| p.name.as_str())
    }

    /// Adds one, dropping the oldest past [`KEEP`], and notes it for the
    /// lights.
    pub fn push(&mut self, s: Seen) {
        if let Some(i) = stream(s.port, s.dir) {
            match s.what.channel() {
                Some(c) => self.last[i][usize::from(c & 0x0F)] = s.t,
                None => self.sys[i] = s.t,
            }
        }
        if self.ring.len() < KEEP {
            self.ring.push(s);
        } else {
            self.ring[self.head] = s;
        }
        self.head = (self.head + 1) % KEEP;
        self.total += 1;
    }

    /// Empties the list (the lights and tempos stay).
    pub fn clear(&mut self) {
        self.ring.clear();
        self.head = 0;
        self.cleared = self.total;
    }

    /// How many have come since the start (cleared or not): the newest
    /// one's number plus one.
    pub fn total(&self) -> u64 {
        self.total
    }

    /// How many the list holds.
    pub fn len(&self) -> usize {
        self.ring.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ring.is_empty()
    }

    /// The list, newest first, each with its number (counted from the
    /// first since the start, so a number keeps naming the same message
    /// as more come).
    pub fn newest(&self) -> impl Iterator<Item = (u64, &Seen)> + '_ {
        let n = self.ring.len();
        (0..n).map(move |k| {
            let i = (self.head + KEEP - 1 - k) % KEEP;
            let i = if n < KEEP { n - 1 - k } else { i };
            (self.total - 1 - k as u64, &self.ring[i])
        })
    }

    /// When `port`'s `channel` last had a message going `dir`.
    pub fn last(&self, port: PortId, dir: Dir, channel: Channel) -> Option<f64> {
        let t = self.last[stream(port, dir)?][usize::from(channel & 0x0F)];
        t.is_finite().then_some(t)
    }

    /// Whether `port`'s `channel` light is lit at `now`.
    pub fn lit(&self, port: PortId, dir: Dir, channel: Channel, now: f64) -> bool {
        self.last(port, dir, channel)
            .is_some_and(|t| now - t < LIGHT_S)
    }

    /// Whether a message with no channel went through `port` lately.
    pub fn sys_lit(&self, port: PortId, dir: Dir, now: f64) -> bool {
        stream(port, dir).is_some_and(|i| now - self.sys[i] < LIGHT_S)
    }

    /// The tempo of the clock arriving on (or leaving) `port`, once a
    /// beat and half a second of it have come.
    pub fn bpm(&self, port: PortId, dir: Dir) -> Option<f32> {
        self.bpm[stream(port, dir)?]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed_and_drain(bytes: &[(f64, PortId, Dir, &[u8])], at: f64) -> Monitor {
        let mut f = Feed::new();
        let mut m = Monitor::new();
        for &(t, p, d, b) in bytes {
            f.bytes(p, d, t, b);
        }
        f.drain_into(&mut m, at);
        m
    }

    fn whats(m: &Monitor) -> Vec<What> {
        let mut v: Vec<What> = m.newest().map(|(_, s)| s.what).collect();
        v.reverse();
        v
    }

    #[test]
    fn channel_messages_keep_their_bytes_and_running_status() {
        let usb = PortId(1);
        let m = feed_and_drain(
            &[
                (1.0, usb, Dir::In, &[0x92, 60, 100, 60, 0]),
                (1.1, usb, Dir::In, &[0xC9, 5]),
                (1.2, DIN, Dir::Out, &[0xB0, 74, 127]),
            ],
            1.3,
        );
        assert_eq!(
            whats(&m),
            [
                What::Channel {
                    msg: Msg::NoteOn {
                        ch: 2,
                        note: 60,
                        vel: 100
                    },
                    bytes: [0x92, 60, 100],
                    len: 3
                },
                // Running status, velocity 0: a note-off, its bytes as sent.
                What::Channel {
                    msg: Msg::NoteOff {
                        ch: 2,
                        note: 60,
                        vel: 64
                    },
                    bytes: [0x92, 60, 0],
                    len: 3
                },
                What::Channel {
                    msg: Msg::Program { ch: 9, program: 5 },
                    bytes: [0xC9, 5, 0],
                    len: 2
                },
                What::Channel {
                    msg: Msg::Cc {
                        ch: 0,
                        cc: 74,
                        value: 127
                    },
                    bytes: [0xB0, 74, 127],
                    len: 3
                },
            ]
        );
        let newest: Vec<(u64, PortId, Dir)> = m.newest().map(|(n, s)| (n, s.port, s.dir)).collect();
        assert_eq!(newest[0], (3, DIN, Dir::Out));
        assert_eq!(newest[3], (0, usb, Dir::In));
        // The lights: channel 3 in on the USB port, channel 1 out on DIN.
        assert!(m.lit(usb, Dir::In, 2, 1.2));
        assert!(!m.lit(usb, Dir::In, 2, 1.4), "out after a quarter second");
        assert!(m.lit(DIN, Dir::Out, 0, 1.3));
        assert!(!m.lit(DIN, Dir::In, 0, 1.3));
        assert_eq!(m.last(usb, Dir::In, 9), Some(1.1));
    }

    #[test]
    fn clock_is_counted_not_listed_with_its_tempo() {
        let mut f = Feed::new();
        let mut m = Monitor::new();
        // Two seconds of clock at 125 BPM (a tick every 20 ms), read in
        // bursts of four as USB delivers them, with a start in the middle.
        for i in 0..100 {
            if i % 4 == 0 {
                f.bytes(DIN, Dir::In, i as f64 * 0.02 + 0.06, &[0xF8; 4]);
            }
            if i == 48 {
                f.bytes(DIN, Dir::In, 0.96, &[0xFA]);
            }
        }
        f.drain_into(&mut m, 2.0);
        assert_eq!(whats(&m), [What::Arriving(Steady::Clock), What::Start]);
        let bpm = m.bpm(DIN, Dir::In).unwrap();
        assert!((bpm - 125.0).abs() < 1.0, "{bpm}");
        assert_eq!(m.bpm(DIN, Dir::Out), None);
        // A second without: stopped, with the count.
        f.drain_into(&mut m, 3.1);
        assert_eq!(whats(&m).last(), Some(&What::Stopped(Steady::Clock, 100)));
        assert_eq!(m.bpm(DIN, Dir::In), None);
        // Sensing and time code alike.
        f.bytes(DIN, Dir::In, 4.0, &[0xFE, 0xF1, 0x12, 0xFE]);
        f.drain_into(&mut m, 4.0);
        let w = whats(&m);
        assert_eq!(
            w[w.len() - 2..],
            [
                What::Arriving(Steady::Sensing),
                What::Arriving(Steady::TimeCode)
            ]
        );
    }

    #[test]
    fn sysex_is_one_line_with_its_length_and_maker() {
        let mut long = vec![0xF0, 0x47, 0x00, 0x26];
        long.extend(std::iter::repeat_n(0x11, 200));
        long.push(0xF7);
        let m = feed_and_drain(
            &[
                (1.0, DIN, Dir::In, &long),
                (
                    1.1,
                    DIN,
                    Dir::In,
                    &[0xF0, 0x00, 0x20, 0x29, 0x01, 0xF8, 0xF7],
                ),
                (
                    1.2,
                    DIN,
                    Dir::In,
                    &[0xF2, 0x10, 0x01, 0xF3, 0x03, 0xF6, 0xF9],
                ),
            ],
            1.3,
        );
        assert_eq!(
            whats(&m),
            [
                What::SysEx {
                    len: 205,
                    maker: [0x47, 0, 0]
                },
                // A clock byte inside a SysEx doesn't count towards it.
                What::Arriving(Steady::Clock),
                What::SysEx {
                    len: 6,
                    maker: [0, 0x20, 0x29]
                },
                What::SongPosition(0x90),
                What::SongSelect(3),
                What::TuneRequest,
                What::Undefined(0xF9),
            ]
        );
        assert_eq!(What::SongPosition(0x90).bytes(), ([0xF2, 0x10, 0x01], 3));
        assert!(m.sys_lit(DIN, Dir::In, 1.3));
    }

    #[test]
    fn the_list_keeps_the_newest_and_numbers_stay_put() {
        let mut m = Monitor::new();
        for i in 0..KEEP + 10 {
            m.push(Seen {
                t: i as f64,
                port: DIN,
                dir: Dir::In,
                what: What::SongSelect((i % 128) as u8),
            });
        }
        assert_eq!(m.len(), KEEP);
        assert_eq!(m.total(), (KEEP + 10) as u64);
        let v: Vec<(u64, f64)> = m.newest().map(|(n, s)| (n, s.t)).collect();
        assert_eq!(v[0], ((KEEP + 9) as u64, (KEEP + 9) as f64));
        assert_eq!(v[KEEP - 1], (10, 10.0));
        m.clear();
        assert!(m.is_empty());
        assert_eq!(m.total(), (KEEP + 10) as u64);
        // The lights stay.
        assert!(m.sys_lit(DIN, Dir::In, (KEEP + 9) as f64));
    }

    #[test]
    fn a_full_queue_counts_what_it_drops() {
        let mut f = Feed::new();
        let mut m = Monitor::new();
        for i in 0..QUEUE + 5 {
            f.bytes(DIN, Dir::In, i as f64 * 1e-3, &[0x90, 60, 1]);
        }
        f.drain_into(&mut m, 1.0);
        assert_eq!(m.len(), QUEUE);
        assert_eq!(m.lost, 5);
    }

    #[test]
    fn the_filter_picks_kinds_ways_ports_and_channels() {
        let note = Seen {
            t: 0.0,
            port: PortId(2),
            dir: Dir::In,
            what: What::Channel {
                msg: Msg::NoteOn {
                    ch: 9,
                    note: 36,
                    vel: 90,
                },
                bytes: [0x99, 36, 90],
                len: 3,
            },
        };
        let start = Seen {
            what: What::Start,
            ..note
        };
        let all = Filter::ALL;
        assert!(all.matches(&note) && all.matches(&start));
        let no_notes = Filter {
            kinds: all.kinds & !Kind::Notes.bit(),
            ..all
        };
        assert!(!no_notes.matches(&note) && no_notes.matches(&start));
        let out_only = Filter { dirs: 2, ..all };
        assert!(!out_only.matches(&note));
        let ch10 = Filter {
            port: Some(PortId(2)),
            channel: Some(9),
            ..all
        };
        assert!(ch10.matches(&note));
        assert!(!ch10.matches(&start), "no channel: not on channel 10");
        let other_port = Filter {
            port: Some(PortId(1)),
            ..all
        };
        assert!(!other_port.matches(&note));
    }

    #[test]
    fn ports_past_the_table_are_ignored() {
        let mut f = Feed::new();
        let mut m = Monitor::new();
        f.bytes(PortId(MAX_PORTS as u8), Dir::In, 0.0, &[0x90, 60, 1]);
        f.drain_into(&mut m, 0.0);
        assert!(m.is_empty());
        assert!(!m.lit(PortId(200), Dir::In, 0, 0.0));
        assert_eq!(m.port_name(DIN), Some("MIDI"));
        assert_eq!(m.shown_ports().count(), 1, "the 5-pin jacks always");
    }
}
