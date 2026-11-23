//! Who holds a setting by hand (the XY controller plan, phase 1: "who may
//! let a setting go").
//!
//! A hand on a setting the mod engine holds (a knob on the chain editor or
//! the Perform view, a macro, a Q-Link, a learned MIDI control) holds it
//! while recording, over the step locks, until it lets go
//! (`ModEvent::hand`, `ModEvent::Touch`). Before this table any let-go let
//! go of it, whoever's it was, and a learned MIDI control, which has no
//! let-go of its own, held it until the project closed. Now each hold has
//! an owner:
//!
//! - **A new gesture takes the setting**: the newest hand wins, never
//!   "whichever moved last".
//! - **A hand that lost it is ignored** until it lets go and starts again
//!   (a learned control: until it has been still for [`HAND_S`]).
//! - **Only the hand that holds it lets it go.**
//! - **A gesture with no end of its own** (a learned MIDI control) ends
//!   [`HAND_S`] after its last move, the time a synth's knob is held for
//!   (`locks.rs`). The same value again isn't a move.
//!
//! Only holds are kept: a hand that just sets a value (not recording, no
//! lane) holds nothing, so nothing waits for its let-go. Fixed table,
//! nothing allocated after [`Hands::new`]; it lives in `ModMix` on the
//! sound loop.

pub use crate::locks::HAND_S;
use mpfree_play::mpfree_param::{Owner, ParamAddr};

/// The most settings held by hand at once.
pub const MAX: usize = 64;

/// Where a hand's gesture comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hand {
    /// The screen and the MPC's own controls: a finger on a knob, a Q-Link
    /// (its touch ends the gesture; one turned from MIDI Follow ends
    /// [`HAND_S`] after its last turn, `qlink_touch.rs`).
    Screen,
    /// A learned MIDI control: its gesture ends [`HAND_S`] after its last
    /// move.
    Midi,
}

impl Hand {
    /// Whether a gesture from this hand ends by itself, with no let-go.
    fn times_out(self) -> bool {
        self == Hand::Midi
    }
}

/// What a hand's move does to the setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Move {
    /// A new gesture: it holds the setting now (another hand may have
    /// lost it).
    Took,
    /// The holder moved it again.
    Again,
    /// The holder sent the same value again: not a move.
    Same,
    /// This hand lost the setting to a newer gesture: ignored.
    Lost,
    /// No room to keep the hold: the move should hold nothing.
    Full,
}

#[derive(Clone, Copy, Debug)]
struct Hold {
    addr: ParamAddr,
    hand: Hand,
    /// A newer gesture took the setting.
    lost: bool,
    /// The last value sent.
    value: f32,
    /// Frames until a gesture with no end of its own ends (0: never).
    left: u32,
    /// It ran out: [`Hands::take_ended`] lets go of it.
    ended: bool,
}

/// The settings held by hand, each with its owner.
#[derive(Clone, Debug)]
pub struct Hands {
    h: [Option<Hold>; MAX],
    /// [`HAND_S`] in frames.
    timeout: u32,
    /// Settings a newer gesture took from another hand.
    pub taken: u64,
    /// Moves ignored: their hand had lost the setting.
    pub ignored: u64,
    /// Gestures with no end of their own that ended after [`HAND_S`].
    pub ended: u64,
    /// Holds with no room in the table (they held nothing).
    pub full: u64,
}

impl Hands {
    /// Empty, for a sound at `rate` frames a second.
    pub fn new(rate: u32) -> Hands {
        Hands {
            h: [None; MAX],
            timeout: ((rate as f32 * HAND_S) as u32).max(1),
            taken: 0,
            ignored: 0,
            ended: 0,
            full: 0,
        }
    }

    fn find(&self, hand: Hand, addr: ParamAddr) -> Option<usize> {
        self.h
            .iter()
            .position(|h| h.is_some_and(|h| h.hand == hand && h.addr == addr))
    }

    /// `hand` moved `addr` to `value` in a way that holds it (a `Touch`).
    pub fn moved(&mut self, hand: Hand, addr: ParamAddr, value: f32) -> Move {
        let timeout = if hand.times_out() { self.timeout } else { 0 };
        if let Some(i) = self.find(hand, addr) {
            let h = self.h[i].as_mut().expect("found");
            if h.lost {
                // Still the same gesture: one with no end of its own runs
                // on from this move, so it can't take the setting back
                // until it has been still for HAND_S.
                h.left = timeout;
                self.ignored += 1;
                return Move::Lost;
            }
            if hand.times_out() && h.value == value {
                return Move::Same;
            }
            h.value = value;
            h.left = timeout;
            return Move::Again;
        }
        let Some(free) = self.h.iter().position(Option::is_none) else {
            self.full += 1;
            return Move::Full;
        };
        for h in self.h.iter_mut().flatten() {
            if h.addr == addr && !h.lost {
                h.lost = true;
                self.taken += 1;
            }
        }
        self.h[free] = Some(Hold {
            addr,
            hand,
            lost: false,
            value,
            left: timeout,
            ended: false,
        });
        Move::Took
    }

    /// `hand` let go of `addr`: true when the setting should be let go of
    /// (this hand held it, or no hand did).
    pub fn release(&mut self, hand: Hand, addr: ParamAddr) -> bool {
        match self.find(hand, addr) {
            Some(i) => !self.h[i].take().expect("found").lost,
            None => !self.held(addr),
        }
    }

    /// Whether a hand holds `addr`.
    pub fn held(&self, addr: ParamAddr) -> bool {
        self.h.iter().flatten().any(|h| h.addr == addr && !h.lost)
    }

    /// `frames` more have played: gestures with no end of their own run
    /// down.
    pub fn tick(&mut self, frames: u32) {
        for h in self.h.iter_mut().flatten() {
            if h.left > 0 {
                h.left = h.left.saturating_sub(frames);
                h.ended = h.left == 0;
            }
        }
    }

    /// The next gesture [`Hands::tick`] found ended, taken off the table:
    /// the setting to let go of (one a newer gesture took is dropped
    /// quietly).
    pub fn take_ended(&mut self) -> Option<ParamAddr> {
        while let Some(i) = self.h.iter().position(|h| h.is_some_and(|h| h.ended)) {
            let h = self.h[i].take().expect("found");
            if !h.lost {
                self.ended += 1;
                return Some(h.addr);
            }
        }
        None
    }

    /// Every hold dropped (a project opened: the engine lets go of them all).
    pub fn clear(&mut self) {
        self.h = [None; MAX];
    }

    /// The holds on `owner`'s settings dropped (an effect left its strip).
    pub fn forget(&mut self, owner: Owner) {
        for h in self.h.iter_mut() {
            if h.is_some_and(|h| h.addr.owner == owner) {
                *h = None;
            }
        }
    }

    /// Settings held now.
    pub fn holding(&self) -> usize {
        self.h.iter().flatten().filter(|h| !h.lost).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_play::mpfree_param::Strip;

    const RATE: u32 = 1000;

    fn knob(n: u16) -> ParamAddr {
        ParamAddr::new(Owner::Strip(Strip::Master), n)
    }

    #[test]
    fn a_newer_gesture_takes_the_setting_and_only_its_hand_lets_go() {
        let mut h = Hands::new(RATE);
        assert_eq!(h.moved(Hand::Screen, knob(1), 0.2), Move::Took);
        assert_eq!(h.moved(Hand::Screen, knob(1), 0.3), Move::Again);
        assert_eq!(h.moved(Hand::Midi, knob(1), 0.5), Move::Took);
        assert_eq!(h.taken, 1);
        // The screen lost it: its moves are ignored, its let-go isn't one.
        assert_eq!(h.moved(Hand::Screen, knob(1), 0.4), Move::Lost);
        assert!(!h.release(Hand::Screen, knob(1)));
        assert!(h.held(knob(1)));
        // Once it has let go, a new touch takes it back.
        assert_eq!(h.moved(Hand::Screen, knob(1), 0.6), Move::Took);
        assert!(!h.release(Hand::Midi, knob(1)), "the knob lost it");
        assert!(h.release(Hand::Screen, knob(1)));
        assert!(!h.held(knob(1)));
        // A let-go nobody holds goes through (a set and its let-go at once).
        assert!(h.release(Hand::Screen, knob(2)));
        assert_eq!((h.ignored, h.holding()), (1, 0));
    }

    #[test]
    fn a_gesture_with_no_end_ends_half_a_second_after_its_last_move() {
        let mut h = Hands::new(RATE);
        let half = (RATE as f32 * HAND_S) as u32;
        assert_eq!(h.moved(Hand::Midi, knob(1), 0.2), Move::Took);
        assert_eq!(h.moved(Hand::Screen, knob(2), 0.2), Move::Took);
        h.tick(half - 10);
        assert_eq!(h.moved(Hand::Midi, knob(1), 0.3), Move::Again);
        h.tick(half - 10);
        // The same value again doesn't keep it.
        assert_eq!(h.moved(Hand::Midi, knob(1), 0.3), Move::Same);
        assert_eq!(h.take_ended(), None);
        h.tick(10);
        assert_eq!(h.take_ended(), Some(knob(1)));
        assert_eq!(h.take_ended(), None);
        // A finger never runs out.
        h.tick(10 * half);
        assert_eq!(h.take_ended(), None);
        assert!(h.held(knob(2)));
        assert_eq!(h.ended, 1);
        // A knob that lost its setting is ignored while it keeps turning,
        // and runs out quietly once it is still.
        assert_eq!(h.moved(Hand::Midi, knob(3), 0.9), Move::Took);
        assert_eq!(h.moved(Hand::Screen, knob(3), 0.1), Move::Took);
        h.tick(half - 10);
        assert_eq!(h.moved(Hand::Midi, knob(3), 0.8), Move::Lost);
        h.tick(half - 10);
        assert_eq!(h.moved(Hand::Midi, knob(3), 0.7), Move::Lost);
        h.tick(half);
        assert_eq!(h.take_ended(), None);
        assert!(h.held(knob(3)));
        assert_eq!((h.ended, h.holding()), (1, 2));
        // Still for half a second, it is a new gesture: it takes it.
        assert_eq!(h.moved(Hand::Midi, knob(3), 0.6), Move::Took);
    }

    #[test]
    fn a_full_table_holds_nothing_and_says_so() {
        let mut h = Hands::new(RATE);
        for n in 0..MAX as u16 {
            assert_eq!(h.moved(Hand::Screen, knob(n), 0.5), Move::Took);
        }
        assert_eq!(h.moved(Hand::Midi, knob(999), 0.5), Move::Full);
        assert_eq!(h.full, 1);
        h.clear();
        assert_eq!(h.holding(), 0);
        assert_eq!(h.moved(Hand::Midi, knob(999), 0.5), Move::Took);
        h.forget(Owner::Strip(Strip::Master));
        assert_eq!(h.holding(), 0);
    }
}
