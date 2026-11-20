//! Room in a host's queue of events for one node, kept so that no ending
//! is ever lost (step sequencer refactor A2b; the time engine's own table
//! is `mpfree_play::room`, refactor A2, and this follows its rule).
//!
//! A host queues each period's events for a node in a fixed list (the
//! shared synth one per engine, a track host one per slot) and hands them
//! over once a period. A [`NodeRoom`] says which go in, so the list never
//! runs out of room for an ending:
//!
//! - **Note-ons**: up to [`Limits::starts`] a period, and only while the
//!   notes this queue owes and the note-offs it passed this period are
//!   fewer than [`Limits::ends`]. A host sends a note one note-off however
//!   many note-ons it had (its last holder's), so a note is owed once. A
//!   refused note-on is owed nothing: a host that counts a note's holders
//!   may count it all the same, so its counts stay paired, and the last
//!   holder's note-off is left out if nothing here owes the note.
//! - **Note-offs**: one for an owed note always fits, since owed notes and
//!   passed note-offs together never pass [`Limits::ends`]. One for a note
//!   this queue doesn't owe (its note-on was refused here) is left out and
//!   counted ([`NodeRoom::unowed`]); nothing sounds for it.
//! - **Pedals.** The sustain and sostenuto pedals hold notes, so a pedal
//!   going down is a start and coming up its ending, owed like a note.
//! - **Values**: pressure, bend, expression, the other controllers, a
//!   parameter's value: up to [`Limits::values`] a period. Past that the
//!   moment is too busy: refused and counted ([`NodeRoom::refused`]).
//! - **Unlocks**: a parameter put back to the value under its locks, when
//!   the last lock on it ends. A host with at most `unlocks / 2`
//!   parameters held at once asks before it holds a new one
//!   ([`NodeRoom::hold`]), which grants `unlocks / 2` a period, so a period
//!   ends at most what was held as it began and what it held since.
//! - **All-offs**: up to [`Limits::all_offs`] a period; each ends every
//!   note and pedal owed. One past that with nothing owed (a burst of
//!   panics) ends nothing, so it is refused, not lost.
//!
//! So a list of [`Limits::room`] places never runs out of room for an
//! ending, and [`NodeRoom::lost`] counts one that found none anyway: a
//! bug, which must stay 0.
//!
//! Fixed size, no allocation, no panics.

use crate::{Ctl, NodeEvent};

/// How many of each kind of event one period of a queue takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub starts: usize,
    pub values: usize,
    /// Owed notes and pedals and note-offs passed this period, together.
    pub ends: usize,
    pub unlocks: usize,
    pub all_offs: usize,
}

impl Limits {
    /// Places a queue needs for everything its room lets in.
    pub const fn room(&self) -> usize {
        self.starts + self.values + self.ends + self.unlocks + self.all_offs
    }
}

/// The two pedals that hold notes, owed after the 128 notes (bits 128
/// and 129 of the owed set).
const SUSTAIN: usize = 128;
const SOSTENUTO: usize = 129;

/// Which events of a period go in a queue for one node; see the module
/// notes.
#[derive(Clone, Copy, Debug)]
pub struct NodeRoom {
    limits: Limits,
    owed: [u64; 3],
    n_owed: usize,
    starts: usize,
    values: usize,
    ended: usize,
    unlocks: usize,
    holds: usize,
    all_offs: usize,
    /// Starts, values and new holds refused for room, so far.
    pub refused: u64,
    /// Note-offs left out because this queue didn't owe their note.
    pub unowed: u64,
    /// Endings with no room: a bug (should stay 0).
    pub lost: u64,
}

impl NodeRoom {
    pub const fn new(limits: Limits) -> NodeRoom {
        NodeRoom {
            limits,
            owed: [0; 3],
            n_owed: 0,
            starts: 0,
            values: 0,
            ended: 0,
            unlocks: 0,
            holds: 0,
            all_offs: 0,
            refused: 0,
            unowed: 0,
            lost: 0,
        }
    }

    /// The limits it keeps.
    pub fn limits(&self) -> Limits {
        self.limits
    }

    /// The queue was handed over: a new period counts from 0. What is
    /// owed stays owed.
    pub fn period(&mut self) {
        self.starts = 0;
        self.values = 0;
        self.ended = 0;
        self.unlocks = 0;
        self.holds = 0;
        self.all_offs = 0;
    }

    /// Nothing is owed any more: the host ended every note another way (it
    /// reset the instrument, swapped it out, or let it rest). The period's
    /// counts stay as they are.
    pub fn clear(&mut self) {
        self.owed = [0; 3];
        self.n_owed = 0;
    }

    /// Notes and pedals started here and not ended.
    pub fn owed(&self) -> usize {
        self.n_owed
    }

    /// Whether note `note` (0 to 127) is owed its note-off here.
    pub fn owes(&self, note: u8) -> bool {
        self.is_owed(usize::from(note & 0x7F))
    }

    fn is_owed(&self, k: usize) -> bool {
        self.owed[k / 64] & (1 << (k % 64)) != 0
    }

    fn set_owed(&mut self, k: usize, on: bool) {
        if on {
            self.owed[k / 64] |= 1 << (k % 64);
        } else {
            self.owed[k / 64] &= !(1 << (k % 64));
        }
    }

    /// Whether `ev` goes in the queue now, counting it if it does. A
    /// parameter's value is a value here: an unlock asks
    /// [`NodeRoom::unlock`] instead.
    pub fn admit(&mut self, ev: &NodeEvent) -> bool {
        match *ev {
            NodeEvent::NoteOn { note, velocity } if velocity > 0 => {
                self.start(usize::from(note & 0x7F))
            }
            NodeEvent::NoteOn { note, .. } | NodeEvent::NoteOff { note } => {
                self.end(usize::from(note & 0x7F))
            }
            NodeEvent::Control { ctl, value } => {
                let pedal = match ctl {
                    Ctl::Sustain => SUSTAIN,
                    Ctl::Sostenuto => SOSTENUTO,
                    _ => return self.value(),
                };
                match (value >= 0.5, self.is_owed(pedal)) {
                    (true, false) => self.start(pedal),
                    (false, true) => self.end(pedal),
                    // Down further, or up with nothing held: a value.
                    _ => self.value(),
                }
            }
            NodeEvent::AllOff => {
                if self.all_offs < self.limits.all_offs {
                    self.all_offs += 1;
                    self.owed = [0; 3];
                    self.n_owed = 0;
                    true
                } else if self.n_owed == 0 {
                    self.refused += 1;
                    false
                } else {
                    self.lost += 1;
                    false
                }
            }
            NodeEvent::Pressure { .. }
            | NodeEvent::Bend { .. }
            | NodeEvent::Expression { .. }
            | NodeEvent::Param { .. } => self.value(),
        }
    }

    fn start(&mut self, k: usize) -> bool {
        let owed = self.is_owed(k);
        // A note owed already is owed once: its next note-off ends it.
        let more = usize::from(!owed);
        if self.starts < self.limits.starts && self.n_owed + self.ended + more <= self.limits.ends {
            self.starts += 1;
            if !owed {
                self.set_owed(k, true);
                self.n_owed += 1;
            }
            true
        } else {
            self.refused += 1;
            false
        }
    }

    fn end(&mut self, k: usize) -> bool {
        if !self.is_owed(k) {
            self.unowed += 1;
            return false;
        }
        // Owed notes and passed note-offs never pass the limit together:
        // this one moves from the first to the second.
        self.set_owed(k, false);
        self.n_owed -= 1;
        self.ended += 1;
        true
    }

    fn value(&mut self) -> bool {
        if self.values < self.limits.values {
            self.values += 1;
            true
        } else {
            self.refused += 1;
            false
        }
    }

    /// Whether a parameter may be held under a lock it wasn't held under
    /// (asked before its first lock goes in), counting it if so.
    pub fn hold(&mut self) -> bool {
        if self.holds < self.limits.unlocks / 2 {
            self.holds += 1;
            true
        } else {
            self.refused += 1;
            false
        }
    }

    /// Whether a parameter put back to the value under its locks goes in
    /// (always, while the host keeps to [`NodeRoom::hold`]).
    pub fn unlock(&mut self) -> bool {
        if self.unlocks < self.limits.unlocks {
            self.unlocks += 1;
            true
        } else {
            self.lost += 1;
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const L: Limits = Limits {
        starts: 4,
        values: 2,
        ends: 6,
        unlocks: 4,
        all_offs: 1,
    };

    fn on(note: u8) -> NodeEvent {
        NodeEvent::NoteOn { note, velocity: 90 }
    }

    fn off(note: u8) -> NodeEvent {
        NodeEvent::NoteOff { note }
    }

    #[test]
    fn a_busy_period_refuses_starts_and_takes_every_owed_ending() {
        let mut r = NodeRoom::new(L);
        // Four starts a period, then refused.
        let taken: Vec<bool> = (0..6).map(|n| r.admit(&on(n))).collect();
        assert_eq!(taken, [true, true, true, true, false, false]);
        assert_eq!((r.owed(), r.refused), (4, 2));
        // Values have their own room: they never take an ending's place.
        assert!(r.admit(&NodeEvent::Pressure { note: 0, value: 9 }));
        assert!(r.admit(&NodeEvent::Bend { value: 0.1 }));
        assert!(!r.admit(&NodeEvent::Bend { value: 0.2 }));
        // Every owed note ends; a refused one's note-off is left out.
        let ended: Vec<bool> = (0..6).map(|n| r.admit(&off(n))).collect();
        assert_eq!(ended, [true, true, true, true, false, false]);
        assert_eq!((r.owed(), r.unowed, r.lost), (0, 2, 0));
        // With starts to spare, owed notes and passed note-offs still stay
        // within six: four ended this period, so only two more start
        // before the period is handed over.
        let mut r = NodeRoom::new(Limits { starts: 8, ..L });
        assert!((0..4).all(|n| r.admit(&on(n))));
        assert!((0..4).all(|n| r.admit(&off(n))));
        assert!(r.admit(&on(9)) && r.admit(&on(10)));
        assert!(!r.admit(&on(11)), "past the ends");
        assert!(r.admit(&off(9)) && r.admit(&off(10)));
        assert_eq!(r.lost, 0);
    }

    #[test]
    fn a_note_owed_twice_is_owed_once_and_a_pedal_is_owed_like_a_note() {
        let mut r = NodeRoom::new(L);
        assert!(r.admit(&on(60)) && r.admit(&on(60)));
        assert_eq!(r.owed(), 1);
        assert!(r.admit(&off(60)));
        assert!(!r.admit(&off(60)), "its one note-off went");
        let pedal = |value| NodeEvent::Control {
            ctl: Ctl::Sustain,
            value,
        };
        assert!(r.admit(&pedal(1.0)));
        assert_eq!(r.owed(), 1);
        assert!(r.admit(&pedal(0.8)), "further down: a value");
        assert!(r.admit(&pedal(0.0)));
        assert_eq!(r.owed(), 0);
        // The mod wheel is a value; the values are spent now.
        let wheel = NodeEvent::Control {
            ctl: Ctl::ModWheel,
            value: 0.3,
        };
        assert!(r.admit(&wheel));
        assert!(!r.admit(&wheel));
    }

    #[test]
    fn an_all_off_ends_everything_owed_and_unlocks_fit_every_hold() {
        let mut r = NodeRoom::new(L);
        assert!(r.admit(&on(1)) && r.admit(&on(2)));
        assert!(r.admit(&NodeEvent::AllOff));
        assert_eq!(r.owed(), 0);
        assert!(!r.admit(&off(1)), "the all-off ended it");
        // A second all-off with nothing owed ends nothing: refused.
        assert!(!r.admit(&NodeEvent::AllOff));
        assert_eq!((r.refused, r.lost), (1, 0));
        // With a note owed again, it has no room: a bug.
        assert!(r.admit(&on(3)));
        assert!(!r.admit(&NodeEvent::AllOff));
        assert_eq!(r.lost, 1, "an all-off with no room: a bug");
        // Two new holds a period, so the four unlocks always fit.
        let mut r = NodeRoom::new(L);
        assert!(r.hold() && r.hold());
        assert!(!r.hold());
        assert!((0..4).all(|_| r.unlock()));
        assert_eq!(r.lost, 0);
        assert!(!r.unlock());
        assert_eq!(r.lost, 1);
    }
}
