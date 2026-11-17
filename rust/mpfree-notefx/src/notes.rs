//! Notes with their times, and a fixed list of them kept in time order.

/// The most events a list holds: a stage's notes waiting to leave, or one
/// period's notes between two stages (32 KB, made once on the heap). More
/// are dropped and counted. A four-note chord echoed three times is 28
/// events, so a list holds a busy part's worth of keys while their echoes
/// wait.
pub const MAX_EVENTS: usize = 2048;

/// A note starting or ending.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    On { note: u8, velocity: u8 },
    Off { note: u8 },
}

impl Kind {
    pub fn note(self) -> u8 {
        match self {
            Kind::On { note, .. } | Kind::Off { note } => note,
        }
    }
}

/// A note on or off at frame `at`, counted from when the sound started
/// (the period's [`crate::Period::start`] is on the same count).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ev {
    pub at: u64,
    pub kind: Kind,
}

impl Ev {
    pub const fn on(at: u64, note: u8, velocity: u8) -> Ev {
        Ev {
            at,
            kind: Kind::On { note, velocity },
        }
    }

    pub const fn off(at: u64, note: u8) -> Ev {
        Ev {
            at,
            kind: Kind::Off { note },
        }
    }
}

const NONE: Ev = Ev::off(0, 0);

/// Events in time order, made once and never grown. Events at the same
/// frame keep the order they were put in, so a note's off put in after its
/// on never comes before it.
pub struct Notes {
    buf: Box<[Ev]>,
    len: usize,
    /// Events that didn't fit, since it was made.
    pub dropped: u64,
}

impl Default for Notes {
    fn default() -> Self {
        Self::new()
    }
}

impl Notes {
    /// Allocates (32 KB); nothing after it does.
    pub fn new() -> Notes {
        Notes {
            buf: vec![NONE; MAX_EVENTS].into_boxed_slice(),
            len: 0,
            dropped: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Room for `n` more.
    pub fn room(&self, n: usize) -> bool {
        self.len + n <= MAX_EVENTS
    }

    pub fn as_slice(&self) -> &[Ev] {
        &self.buf[..self.len]
    }

    pub fn clear(&mut self) {
        self.len = 0;
    }

    /// Keeps the first `n` events.
    pub fn truncate(&mut self, n: usize) {
        self.len = self.len.min(n);
    }

    /// Puts `ev` in after every event at or before its frame. False, and
    /// counted, when the list is full.
    pub fn push(&mut self, ev: Ev) -> bool {
        if self.len == MAX_EVENTS {
            self.dropped += 1;
            return false;
        }
        let mut i = self.len;
        while i > 0 && self.buf[i - 1].at > ev.at {
            self.buf[i] = self.buf[i - 1];
            i -= 1;
        }
        self.buf[i] = ev;
        self.len += 1;
        true
    }

    /// Moves the events before frame `end` to `out`, in order.
    pub fn drain_before(&mut self, end: u64, out: &mut Notes) {
        let n = self.buf[..self.len]
            .iter()
            .take_while(|e| e.at < end)
            .count();
        for e in &self.buf[..n] {
            out.push(*e);
        }
        self.buf.copy_within(n..self.len, 0);
        self.len -= n;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_stay_in_time_order_and_a_frames_events_keep_theirs() {
        let mut n = Notes::new();
        n.push(Ev::on(10, 60, 100));
        n.push(Ev::off(5, 62));
        n.push(Ev::off(10, 60));
        n.push(Ev::on(10, 64, 90));
        assert_eq!(
            n.as_slice(),
            [
                Ev::off(5, 62),
                Ev::on(10, 60, 100),
                Ev::off(10, 60),
                Ev::on(10, 64, 90)
            ]
        );
        let mut out = Notes::new();
        n.drain_before(10, &mut out);
        assert_eq!(out.as_slice(), [Ev::off(5, 62)]);
        assert_eq!(n.len(), 3);
        n.drain_before(11, &mut out);
        assert_eq!(out.len(), 4);
        assert!(n.is_empty());
    }

    #[test]
    fn a_full_list_counts_what_it_drops() {
        let mut n = Notes::new();
        for i in 0..MAX_EVENTS {
            assert!(n.push(Ev::on(i as u64, 60, 1)));
        }
        assert!(!n.room(1));
        assert!(!n.push(Ev::on(0, 60, 1)));
        assert_eq!(n.dropped, 1);
    }
}
