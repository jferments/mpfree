//! Let-gos that find the queue to the sound loop full wait here, never
//! dropped (the XY controller plan, phase 1: "a held setting can't stay
//! held").
//!
//! The screen hands its actions to the sound loop through a queue of 256.
//! When the sound loop is busy (a project opening, a long edit) the queue
//! can fill, and an action that doesn't fit used to be dropped. A press
//! or a hit lost that way is a sound that didn't play, but a lost let-go
//! is a setting or a note that stays held: the XY pad's effect left open,
//! a throw left on, the drums left dropped, a knob still in the hand.
//!
//! So an action that holds a control is sorted by what it does to it
//! ([`hold_of`]):
//!
//! - a **let-go** that can't go waits here, oldest first, and goes first
//!   on a later frame;
//! - a **move** (a knob turned, the XY pad moved) waits too, the newest
//!   taking the place of an older one waiting for the same control, so the
//!   control ends where the finger left it;
//! - a **press** that can't go is dropped as before (a hit late after a
//!   stall would be wrong), unless a let-go of the same control waits: then
//!   it waits behind it, so a control's actions never overtake each other.
//!
//! The list lives on the screen's thread, so it can grow as it needs; it
//! empties as soon as the sound loop takes actions again. What waited is
//! counted and said when the list empties. The XY pad has a second guard
//! beside this: `xy_slot.rs`.

use std::collections::VecDeque;

use crate::gui::FromUi;
use crate::mixing::{PlayFx, ThrowFx};
use mpfree_chain_ui::{ChainAction, MacroAction};
use mpfree_play::mpfree_param::ParamAddr;

/// A control on the screen an action holds, moves or lets go of.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Control {
    Pad(usize),
    Audition(usize),
    SampleHit {
        kit: usize,
        pad: usize,
    },
    SampleTake(usize),
    SampleKey {
        kit: usize,
        note: u8,
    },
    /// A knob on a setting: the chain editor's, a Perform view Q-Link's.
    Knob(ParamAddr),
    /// A macro's knob on the macros panel.
    Macro(usize),
    Morph,
    Throw(ThrowFx),
    /// A Perform pad holding a macro (`PlayFx::Macro`).
    PerformMacro(u8),
    Drop,
    Xy,
    Overlay,
    Recall,
    Outline,
    /// A finger resting on a control with a value (`FromUi::Held`).
    Resting,
}

/// What an action does to its control.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hold {
    Press,
    Move,
    LetGo,
}

/// The control `a` holds, moves or lets go of, if any.
pub fn hold_of(a: &FromUi) -> Option<(Control, Hold)> {
    let press = |down: bool| if down { Hold::Press } else { Hold::LetGo };
    let state = |on: bool| if on { Hold::Move } else { Hold::LetGo };
    Some(match a {
        FromUi::Pad { pad, down } => (Control::Pad(*pad), press(*down)),
        FromUi::Audition { pad, down } => (Control::Audition(*pad), press(*down)),
        FromUi::SampleHit { kit, pad, down, .. } => (
            Control::SampleHit {
                kit: *kit,
                pad: *pad,
            },
            press(*down),
        ),
        FromUi::SampleTake { kit, down } => (Control::SampleTake(*kit), press(*down)),
        FromUi::SampleKey {
            kit, note, down, ..
        } => (
            Control::SampleKey {
                kit: *kit,
                note: *note,
            },
            press(*down),
        ),
        FromUi::ThrowKnob { addr, .. } => (Control::Knob(*addr), Hold::Move),
        FromUi::ThrowKnobLift(addr) => (Control::Knob(*addr), Hold::LetGo),
        FromUi::Chain(ChainAction::Set { addr, .. }) => (Control::Knob(*addr), Hold::Move),
        FromUi::Chain(ChainAction::Release(addr)) => (Control::Knob(*addr), Hold::LetGo),
        FromUi::Macros(MacroAction::Macro { m, .. }) => (Control::Macro(*m), Hold::Move),
        FromUi::Macros(MacroAction::Release(m)) => (Control::Macro(*m), Hold::LetGo),
        FromUi::Macros(MacroAction::Morph(_)) => (Control::Morph, Hold::Move),
        FromUi::PlayFx(PlayFx::Throw { fx, down, .. }) => (Control::Throw(*fx), press(*down)),
        FromUi::PlayFx(PlayFx::Macro { card, value }) => {
            (Control::PerformMacro(*card), press(value.is_some()))
        }
        FromUi::PlayFx(PlayFx::Drop(pads)) => (Control::Drop, press(*pads != 0)),
        // The pad's glide home after a lift is let-gos too: still up.
        FromUi::PlayFx(PlayFx::Xy { down, .. }) => (Control::Xy, state(*down)),
        FromUi::Overlay(on) => (Control::Overlay, state(*on)),
        FromUi::Recall(on) => (Control::Recall, state(*on)),
        FromUi::Outline(on) => (Control::Outline, state(*on)),
        FromUi::Held(on) => (Control::Resting, state(*on)),
        _ => return None,
    })
}

/// The actions waiting for room in the queue to the sound loop.
#[derive(Debug, Default)]
pub struct Waiting {
    list: VecDeque<FromUi>,
    /// Actions that waited since the list last emptied, for the line said
    /// then.
    stall: u64,
    /// Actions that waited, all told.
    pub waited: u64,
    /// The most that waited at once.
    pub most: usize,
}

impl Waiting {
    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    /// `a` on its way to the queue: kept here, behind it, when an action
    /// for the same control waits, so it can't overtake it (None); else
    /// handed back to send.
    pub fn behind(&mut self, a: FromUi) -> Option<FromUi> {
        let Some((c, h)) = hold_of(&a) else {
            return Some(a);
        };
        if !self
            .list
            .iter()
            .any(|w| matches!(hold_of(w), Some((wc, _)) if wc == c))
        {
            return Some(a);
        }
        self.keep(a, c, h);
        None
    }

    /// `a` found the queue full: kept if it lets go of a control or moves
    /// one (None), handed back if it is a press or holds nothing.
    pub fn full(&mut self, a: FromUi) -> Option<FromUi> {
        match hold_of(&a) {
            Some((c, h @ (Hold::LetGo | Hold::Move))) => {
                self.keep(a, c, h);
                None
            }
            _ => Some(a),
        }
    }

    fn keep(&mut self, a: FromUi, c: Control, h: Hold) {
        // The newest move or let-go takes the place of the last one waiting
        // for the same control, when that one did the same.
        let last = self
            .list
            .iter_mut()
            .rev()
            .find(|w| matches!(hold_of(w), Some((wc, _)) if wc == c));
        match last {
            Some(w) if h != Hold::Press && hold_of(w).is_some_and(|(_, wh)| wh == h) => *w = a,
            _ => {
                self.list.push_back(a);
                self.stall += 1;
                self.waited += 1;
                self.most = self.most.max(self.list.len());
            }
        }
    }

    /// Sends what waits, oldest first, with `send` (which hands an action
    /// back when the queue is still full), stopping at the first that
    /// doesn't fit. Returns a line to say when the list has just emptied
    /// after waiting.
    pub fn send(&mut self, mut send: impl FnMut(FromUi) -> Result<(), FromUi>) -> Option<String> {
        while let Some(a) = self.list.pop_front() {
            if let Err(a) = send(a) {
                self.list.push_front(a);
                return None;
            }
        }
        if self.stall == 0 {
            return None;
        }
        let n = std::mem::take(&mut self.stall);
        Some(format!(
            "screen: {n} let-gos and moves waited for room on their way to the sound and were sent ({} so far)",
            self.waited
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn xy(x: f32, down: bool) -> FromUi {
        FromUi::PlayFx(PlayFx::Xy { x, y: 0.5, down })
    }

    fn pad(pad: usize, down: bool) -> FromUi {
        FromUi::Pad { pad, down }
    }

    /// A queue that takes `room` actions, then is full.
    fn queue(room: usize) -> (Vec<FromUi>, usize) {
        (Vec::new(), room)
    }

    fn take(q: &mut (Vec<FromUi>, usize)) -> impl FnMut(FromUi) -> Result<(), FromUi> + '_ {
        move |a| {
            if q.0.len() < q.1 {
                q.0.push(a);
                Ok(())
            } else {
                Err(a)
            }
        }
    }

    #[test]
    fn a_let_go_waits_and_a_press_is_dropped() {
        let mut w = Waiting::default();
        assert!(w.full(pad(3, false)).is_none(), "a let-go waits");
        assert_eq!(w.full(pad(4, true)), Some(pad(4, true)), "a press doesn't");
        assert_eq!(
            w.full(FromUi::TapTempo),
            Some(FromUi::TapTempo),
            "nor does what holds nothing"
        );
        let mut q = queue(0);
        assert_eq!(w.send(take(&mut q)), None, "still full");
        assert!(!w.is_empty());
        let mut q = queue(8);
        let said = w.send(take(&mut q)).expect("said once it empties");
        assert!(said.starts_with("screen: 1 let-gos"), "{said}");
        assert_eq!(q.0, [pad(3, false)]);
        assert_eq!(w.send(take(&mut q)), None, "said once");
    }

    #[test]
    fn a_controls_actions_never_overtake_its_waiting_let_go() {
        let mut w = Waiting::default();
        assert!(w.full(pad(3, false)).is_none());
        // Pad 3 hit again: it goes behind its let-go, not before it.
        assert!(w.behind(pad(3, true)).is_none());
        // Another pad goes straight on.
        assert_eq!(w.behind(pad(5, true)), Some(pad(5, true)));
        let mut q = queue(1);
        assert_eq!(w.send(take(&mut q)), None, "room for one");
        assert_eq!(q.0, [pad(3, false)]);
        let mut q = queue(8);
        assert!(w.send(take(&mut q)).is_some());
        assert_eq!(q.0, [pad(3, true)], "the press after its let-go");
    }

    #[test]
    fn the_newest_move_takes_the_place_of_one_waiting() {
        let mut w = Waiting::default();
        // The XY pad moved, lifted and glided home while the queue was
        // full: what lands is the press's last place, the lift, and only
        // the last place of the glide.
        assert!(w.full(xy(0.1, true)).is_none());
        assert!(w.full(xy(0.2, true)).is_none());
        assert!(w.full(xy(0.3, false)).is_none());
        assert!(w.behind(xy(0.4, false)).is_none());
        assert!(w.behind(xy(0.5, false)).is_none());
        assert_eq!((w.waited, w.most), (2, 2));
        let mut q = queue(8);
        w.send(take(&mut q));
        assert_eq!(q.0, [xy(0.2, true), xy(0.5, false)]);
    }

    #[test]
    fn what_holds_and_lets_go_is_sorted() {
        let addr = ParamAddr::macro_value(2);
        let cases = [
            (FromUi::ThrowKnob { addr, value: 0.5 }, Hold::Move),
            (FromUi::ThrowKnobLift(addr), Hold::LetGo),
            (FromUi::Chain(ChainAction::Release(addr)), Hold::LetGo),
            (FromUi::Macros(MacroAction::Release(1)), Hold::LetGo),
            (
                FromUi::PlayFx(PlayFx::Macro {
                    card: 1,
                    value: None,
                }),
                Hold::LetGo,
            ),
            (FromUi::PlayFx(PlayFx::Drop(0)), Hold::LetGo),
            (FromUi::PlayFx(PlayFx::Drop(5)), Hold::Press),
            (FromUi::Overlay(false), Hold::LetGo),
            (FromUi::Held(true), Hold::Move),
        ];
        for (a, h) in cases {
            assert_eq!(hold_of(&a).map(|(_, h)| h), Some(h), "{a:?}");
        }
    }
}
