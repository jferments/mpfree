//! The always-on log of moves, for capturing and recording them.

use mpfree_param::ParamAddr;
use mpfree_transport::time::Tick;
use mpfree_transport::Block;

use crate::{Kind, Map};

/// Moves kept in the log: minutes of busy knob work (a value about every
/// 5 ms on two knobs is 24 000 a minute).
pub const MAX_LOG: usize = 65_536;
/// Inputs one period can bring.
pub const MAX_INPUT: usize = 256;
/// Gestures that can be under way at once.
pub const MAX_ACTIVE: usize = 32;
/// A stillness of this many bars (no gesture under way) ends a phrase.
pub const GAP_BARS: i64 = 1;

/// A move, for [`MoveLog::input`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Input {
    /// A parameter being moved is at `value` (a knob turned, the XY pad, a
    /// pad's Hold turned on). The first value starts a gesture.
    Move {
        addr: ParamAddr,
        value: f32,
        map: Map,
    },
    /// The gesture on `addr` ended: the finger lifted, the knob let go,
    /// the XY pad back at rest, a pad released.
    End { addr: ParamAddr },
}

/// A move in the log.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LogEntry {
    /// Counts every entry ever logged.
    pub seq: u64,
    /// The song tick it was made on.
    pub song: f64,
    /// A clock in ticks that never goes back: song ticks, carried on
    /// across a jump (a song loop, a locate).
    pub at: f64,
    pub addr: ParamAddr,
    pub map: Map,
    /// Knob position, 0 to 1 (unused by an end).
    pub u: f32,
    /// [`Kind::Ramp`] (or [`Kind::Step`] on a switch) for a value,
    /// [`Kind::End`] for a gesture's end.
    pub kind: Kind,
}

const NO_ENTRY: LogEntry = LogEntry {
    seq: 0,
    song: 0.0,
    at: 0.0,
    addr: ParamAddr::macro_value(0),
    map: Map {
        min: 0.0,
        max: 1.0,
        log: false,
        steps: crate::Steps::None,
    },
    u: 0.0,
    kind: Kind::End,
};

/// One parameter's gesture from the log, for [`crate::Moves::put`].
#[derive(Clone, Debug, PartialEq)]
pub struct Gesture {
    pub addr: ParamAddr,
    pub map: Map,
    /// The song tick of its first point.
    pub song: f64,
    /// Its points: the clock ([`LogEntry::at`]), knob position and kind.
    /// It ends with an end unless it is still under way.
    pub points: Vec<(f64, f32, Kind)>,
}

impl Gesture {
    /// Its points in the ticks of a clip `len` ticks long whose tick 0 is
    /// on song tick `origin`: the first point where the song had it in the
    /// clip, the rest after it as they came (they may run past `len`; see
    /// [`crate::Moves::put`]).
    pub fn in_clip(&self, origin: f64, len: u32) -> Vec<(f64, f32, Kind)> {
        let lenf = f64::from(len.max(1));
        let start = (self.song - origin).rem_euclid(lenf);
        let at0 = self.points.first().map_or(0.0, |p| p.0);
        self.points
            .iter()
            .map(|&(at, u, k)| (start + (at - at0), u, k))
            .collect()
    }
}

/// Every move made while the transport rolls, in a fixed ring. See the
/// crate docs.
///
/// On the audio thread: [`MoveLog::input`] with each move and its offset
/// in the period, then [`MoveLog::process`] with the transport's block.
/// Neither allocates. Moves made while the transport is stopped are not
/// kept (they have no place in the song); a stop ends every gesture.
pub struct MoveLog {
    ring: Vec<LogEntry>,
    pushed: u64,
    input: [(u32, Input); MAX_INPUT],
    n_input: usize,
    /// Gestures under way and their last knob position.
    active: [(ParamAddr, f32, Map); MAX_ACTIVE],
    n_active: usize,
    /// The clock at the end of the last rolling span, and the song tick
    /// there.
    clock: f64,
    /// The clock minus the song tick, while rolling.
    shift: f64,
    song_end: Tick,
    rolling: bool,
    /// Ticks in a bar of the current meter.
    bar: i64,
    /// The clock when the last gesture ended.
    quiet_since: f64,
    /// The first entry of the current phrase.
    head: Option<u64>,
    dropped: u64,
}

impl Default for MoveLog {
    fn default() -> Self {
        Self::new()
    }
}

impl MoveLog {
    /// Allocates the ring (control thread).
    pub fn new() -> MoveLog {
        let none = (ParamAddr::macro_value(0), 0.0, NO_ENTRY.map);
        MoveLog {
            ring: vec![NO_ENTRY; MAX_LOG],
            pushed: 0,
            input: [(0, Input::End { addr: none.0 }); MAX_INPUT],
            n_input: 0,
            active: [none; MAX_ACTIVE],
            n_active: 0,
            clock: 0.0,
            shift: 0.0,
            song_end: 0,
            rolling: false,
            bar: 3840,
            quiet_since: f64::NEG_INFINITY,
            head: None,
            dropped: 0,
        }
    }

    /// A move for the coming [`MoveLog::process`], at `offset` in the
    /// period. They may come in any order; they are kept in offset order.
    pub fn input(&mut self, offset: u32, input: Input) {
        if self.n_input == MAX_INPUT {
            self.dropped += 1;
            return;
        }
        let mut i = self.n_input;
        while i > 0 && self.input[i - 1].0 > offset {
            self.input[i] = self.input[i - 1];
            i -= 1;
        }
        self.input[i] = (offset, input);
        self.n_input += 1;
    }

    /// Puts the period's moves on the song's ticks, by the transport's
    /// `block`.
    pub fn process(&mut self, block: &Block) {
        let mut next = 0;
        for span in block.spans() {
            let timed = span.state.rolling();
            let end = span.offset + span.len;
            if timed {
                let ticks = span.ticks();
                if !self.rolling {
                    // Started again: the clock carries on from where it was.
                    self.shift_to(span.tick_at(span.frame));
                } else if ticks.start != self.song_end {
                    // A jump: the clock carries on over it.
                    self.shift_to(span.tick_at(span.frame));
                }
                self.bar = span.meter.bar_ticks().max(1);
                self.song_end = ticks.end;
            } else if self.rolling {
                // Stopped: every gesture under way ends here.
                let at = self.clock;
                let song = self.song_end as f64;
                while self.n_active > 0 {
                    let (addr, u, map) = self.active[self.n_active - 1];
                    self.n_active -= 1;
                    self.push(song, at, addr, map, u, Kind::End);
                }
                self.quiet_since = at;
            }
            self.rolling = timed;
            while next < self.n_input && self.input[next].0 < end {
                let (offset, input) = self.input[next];
                next += 1;
                if !timed {
                    continue;
                }
                let frame = span.frame + u64::from(offset.saturating_sub(span.offset));
                let song = span.tick_at(frame);
                let at = song + self.shift;
                self.take(song, at.max(self.last_at()), input);
            }
            if timed {
                let song = span.tick_at(span.frame + u64::from(span.len));
                self.clock = song + self.shift;
            }
        }
        self.dropped += (self.n_input - next) as u64;
        self.n_input = 0;
    }

    fn last_at(&self) -> f64 {
        if self.pushed == 0 {
            f64::NEG_INFINITY
        } else {
            self.ring[((self.pushed - 1) % MAX_LOG as u64) as usize].at
        }
    }

    /// Sets the clock's shift so song tick `song` is the clock now.
    fn shift_to(&mut self, song: f64) {
        self.shift = self.clock - song;
    }

    fn take(&mut self, song: f64, at: f64, input: Input) {
        match input {
            Input::Move { addr, value, map } => {
                let u = map.unit(value);
                let kind = if map.stepped() {
                    Kind::Step
                } else {
                    Kind::Ramp
                };
                match self.active[..self.n_active]
                    .iter()
                    .position(|a| a.0 == addr)
                {
                    Some(i) => {
                        if self.active[i].1 == u {
                            return;
                        }
                        self.active[i].1 = u;
                    }
                    None => {
                        if self.n_active == MAX_ACTIVE {
                            self.dropped += 1;
                            return;
                        }
                        let gap = (self.bar * GAP_BARS) as f64;
                        if self.n_active == 0
                            && (self.head.is_none() || at - self.quiet_since >= gap)
                        {
                            self.head = Some(self.pushed);
                        }
                        self.active[self.n_active] = (addr, u, map);
                        self.n_active += 1;
                    }
                }
                self.push(song, at, addr, map, u, kind);
            }
            Input::End { addr } => {
                let Some(i) = self.active[..self.n_active]
                    .iter()
                    .position(|a| a.0 == addr)
                else {
                    return;
                };
                let (_, u, map) = self.active[i];
                self.active[i] = self.active[self.n_active - 1];
                self.n_active -= 1;
                self.push(song, at, addr, map, u, Kind::End);
                if self.n_active == 0 {
                    self.quiet_since = at;
                }
            }
        }
    }

    fn push(&mut self, song: f64, at: f64, addr: ParamAddr, map: Map, u: f32, kind: Kind) {
        let seq = self.pushed;
        self.ring[(seq % MAX_LOG as u64) as usize] = LogEntry {
            seq,
            song,
            at,
            addr,
            map,
            u,
            kind,
        };
        self.pushed += 1;
    }

    /// Entries ever logged; the next one gets this as its `seq`.
    pub fn seq(&self) -> u64 {
        self.pushed
    }

    /// The entries from `seq` on that the ring still holds, oldest first
    /// (for a recorder writing moves into a clip as they come). No
    /// allocation.
    pub fn since(&self, seq: u64) -> impl Iterator<Item = &LogEntry> + '_ {
        let from = seq.max(self.pushed.saturating_sub(MAX_LOG as u64));
        (from..self.pushed).map(move |s| &self.ring[(s % MAX_LOG as u64) as usize])
    }

    /// Whether a gesture is under way now.
    pub fn moving(&self) -> bool {
        self.n_active > 0
    }

    /// Inputs lost (a full input table, a full gesture table). Stays 0 in
    /// normal use.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// The current phrase's span on the song's ticks (first and last
    /// moves), or `None` when there is none: what CAPTURE would keep.
    pub fn phrase_span(&self) -> Option<(f64, f64)> {
        let head = self.head?;
        let first = self.since(head).next()?;
        let last = &self.ring[((self.pushed - 1) % MAX_LOG as u64) as usize];
        Some((first.song, first.song + (last.at - first.at)))
    }

    /// The moves since the last stillness of [`GAP_BARS`] bars, as one
    /// gesture per parameter move, in the order they began. Allocates:
    /// never on the audio thread.
    pub fn phrase(&self) -> Vec<Gesture> {
        let Some(head) = self.head else {
            return Vec::new();
        };
        let mut out: Vec<Gesture> = Vec::new();
        // The gesture each parameter has open in `out`.
        let mut open: Vec<(ParamAddr, usize)> = Vec::new();
        for e in self.since(head) {
            let k = match open.iter().position(|o| o.0 == e.addr) {
                Some(i) => open[i].1,
                None => {
                    if e.kind == Kind::End {
                        continue;
                    }
                    out.push(Gesture {
                        addr: e.addr,
                        map: e.map,
                        song: e.song,
                        points: Vec::new(),
                    });
                    open.push((e.addr, out.len() - 1));
                    out.len() - 1
                }
            };
            out[k].points.push((e.at, e.u, e.kind));
            if e.kind == Kind::End {
                open.retain(|o| o.0 != e.addr);
            }
        }
        out
    }

    /// Forgets the current phrase (it was kept, or thrown away), so CAPTURE
    /// does nothing until something new is moved.
    pub fn forget(&mut self) {
        if self.n_active == 0 {
            self.head = None;
        } else {
            self.head = Some(self.pushed);
        }
    }
}
