//! The XY pad's hand on a synth's, a pad's and an arp's own settings (the
//! XY controller plan, phase 2: "a synth's settings (locks.rs) and a pad's
//! sound (the sampler's pad locks) gain the same hand layer").
//!
//! A layout of yours plays the mod engine's settings straight through
//! `ModMix` (`xy_play.rs`). A synth's settings (`Owner::Inst`: the synth
//! on the pads, a track's own instrument), a kit pad's own settings
//! (`Owner::Pad`) and a track's arp's (`Owner::Arp`: rate, gate, swing,
//! octaves and the Modifier's push) live outside the mixer, so the pad's
//! moves on them wait here, one entry a setting, and the sound loop hands
//! each period's to their owners before they play ([`OwnHands::apply`],
//! `main.rs`):
//!
//! - **While held** the pad's value plays and the setting's own value is
//!   left alone: `locks::Under::press` for a synth's, the sampler's
//!   `Event::PadHand` for a pad's, `mpfree_arp::hand` for an arp's. It
//!   holds over a step's lock while a track records; otherwise the lock
//!   plays over it, as for the mixer.
//! - **Go back** glides from where the pad left it to what is underneath
//!   as it is by then (a lock, or the setting's own value), over the
//!   layout's glide, then lets go.
//! - **Stay** makes the pad's value the setting's own, as a turn of its
//!   knob would (the Synth tab's turn; a pad's own setting on its kit, as
//!   a learned control sets it; an arp's as a learned control sets it),
//!   for one UNDO step with the rest of the lift. The hand stays on until
//!   the setting's own value has arrived (a pad's comes back from the
//!   screen with the kit's next view), so nothing jumps, and never longer
//!   than [`SETTLE_S`].
//! - **Letting go always goes through:** a let-go is a state on the
//!   setting's own entry, not a message, so it never needs room in a
//!   queue, and the table has room for a layout's settings twice over (a
//!   new layout's while the last one's glide back).
//!
//! An Add setting can't push a synth's, a pad's or an arp's own setting
//! yet (they don't go through the mod engine): Setup greys Add on them
//! (`xy_song::NO_ADD`). Nothing here allocates.

use mpfree_play::mpfree_param::{Owner, ParamAddr};

/// The most settings held at once: a layout's, and the last one's still
/// gliding back.
pub const MAX: usize = 2 * mpfree_xy::SLOTS;

/// The longest the hand stays on after Stay, waiting for the setting's own
/// value to arrive, in seconds.
pub const SETTLE_S: f32 = 1.0;

/// Whether `addr` is a synth's, a pad's or an arp's own setting: one the
/// pad's hand reaches here, not through the mod engine. Named by its track
/// (`Owner::Inst`, `Owner::Pad`, `Owner::Arp`) or, since sampler step 16,
/// by its instrument (`Owner::Sound`, `Owner::KitPad`).
pub fn is_own(addr: ParamAddr) -> bool {
    matches!(
        addr.owner,
        Owner::Inst(_) | Owner::Pad(..) | Owner::Sound(_) | Owner::KitPad(..) | Owner::Arp(_)
    )
}

/// Where a synth's, a pad's and an arp's own settings live: the sound
/// loop's side.
pub trait OwnSide {
    /// The pad's hand holds `addr` at `value` (`Some`), over the locks
    /// when `over`, or lets go of it (`None`). False when nothing holds
    /// it: no such setting, a pad with no sound, a full table.
    fn hold(&mut self, addr: ParamAddr, value: Option<f32>, over: bool) -> bool;
    /// What `addr` goes back to if the pad lets go now (its lock, else
    /// its own value), and its own value. None: it isn't there.
    fn beneath(&self, addr: ParamAddr) -> Option<(f32, f32)>;
    /// Stay: `value` becomes `addr`'s own, as a turn of its knob. The
    /// value it became (a stepped setting's step: an arp's rate), or None
    /// when it can't (it isn't there).
    fn keep(&mut self, addr: ParamAddr, value: f32) -> Option<f32>;
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum State {
    /// The pad holds it; `fresh`: a value its owner hasn't had yet.
    Held { fresh: bool },
    /// Go back: from `from` to what's underneath, `left` of `all` frames
    /// still to go.
    Back { from: f32, left: u32, all: u32 },
    /// Let go at once.
    Free,
    /// Stay: its value becomes the setting's own.
    Keep,
    /// Stay sent: the hand stays on until the own value has arrived, for
    /// `left` frames at most.
    Settle { left: u32 },
}

#[derive(Clone, Copy, Debug)]
struct Own {
    addr: ParamAddr,
    value: f32,
    over: bool,
    state: State,
}

/// The pad's hand on a synth's and a pad's own settings. See the module
/// docs.
#[derive(Clone, Debug)]
pub struct OwnHands {
    e: [Option<Own>; MAX],
    settle: u32,
    /// Moves with no room in the table (they held nothing).
    pub full: u64,
    /// Moves their owner couldn't hold (no such setting now).
    pub unreached: u64,
}

impl OwnHands {
    /// Nothing held, for a sound at `rate` frames a second.
    pub fn new(rate: u32) -> OwnHands {
        OwnHands {
            e: [None; MAX],
            settle: ((rate as f32 * SETTLE_S) as u32).max(1),
            full: 0,
            unreached: 0,
        }
    }

    fn find(&self, addr: ParamAddr) -> Option<usize> {
        self.e
            .iter()
            .position(|e| e.is_some_and(|e| e.addr == addr))
    }

    /// The pad's hand moves `addr` to `value`, over the locks when `over`
    /// (a track records). False when there is no room to hold it.
    pub fn touch(&mut self, addr: ParamAddr, value: f32, over: bool) -> bool {
        let i = match self.find(addr) {
            Some(i) => i,
            None => match self.e.iter().position(Option::is_none) {
                Some(i) => i,
                None => {
                    self.full += 1;
                    return false;
                }
            },
        };
        self.e[i] = Some(Own {
            addr,
            value,
            over,
            state: State::Held { fresh: true },
        });
        true
    }

    /// The pad lets go of `addr` over `frames` (0: at once), if it holds
    /// it.
    pub fn let_go(&mut self, addr: ParamAddr, frames: u32) {
        if let Some(e) = self.find(addr).and_then(|i| self.e[i].as_mut()) {
            e.state = if frames == 0 {
                State::Free
            } else {
                State::Back {
                    from: e.value,
                    left: frames,
                    all: frames,
                }
            };
        }
    }

    /// Stay: the pad's `value` becomes `addr`'s own, if it holds it. The
    /// turn for UNDO comes from [`OwnHands::apply`], which knows the value
    /// before.
    pub fn keep(&mut self, addr: ParamAddr, value: f32) {
        if let Some(e) = self.find(addr).and_then(|i| self.e[i].as_mut()) {
            e.value = value;
            e.state = State::Keep;
        }
    }

    /// Whether the pad holds `addr` here.
    #[cfg(test)]
    pub fn holds(&self, addr: ParamAddr) -> bool {
        self.find(addr).is_some()
    }

    /// How many settings the pad holds here.
    pub fn holding(&self) -> usize {
        self.e.iter().flatten().count()
    }

    /// Once a period of `frames`, before the sounds play: each setting's
    /// new value, glide or let-go goes to its owner. `kept` hears each
    /// Stay: the setting, its own value before and after.
    pub fn apply(
        &mut self,
        frames: u32,
        side: &mut dyn OwnSide,
        mut kept: impl FnMut(ParamAddr, f32, f32),
    ) {
        for slot in self.e.iter_mut() {
            let Some(e) = slot.as_mut() else {
                continue;
            };
            let done = match e.state {
                State::Held { fresh: false } => false,
                State::Held { fresh: true } => {
                    e.state = State::Held { fresh: false };
                    if !side.hold(e.addr, Some(e.value), e.over) {
                        self.unreached += 1;
                        // Nothing holds it: nothing to let go of.
                        true
                    } else {
                        false
                    }
                }
                State::Back { from, left, all } => {
                    let left = left.saturating_sub(frames);
                    match side.beneath(e.addr) {
                        Some((to, _)) if left > 0 => {
                            let v = to + (from - to) * left as f32 / all.max(1) as f32;
                            e.value = v;
                            e.state = State::Back { from, left, all };
                            side.hold(e.addr, Some(v), e.over);
                            false
                        }
                        // There, or gone: let go.
                        _ => {
                            side.hold(e.addr, None, e.over);
                            true
                        }
                    }
                }
                State::Free => {
                    side.hold(e.addr, None, e.over);
                    true
                }
                State::Keep => {
                    let before = side.beneath(e.addr).map(|(_, own)| own);
                    side.hold(e.addr, Some(e.value), e.over);
                    let now = before.and_then(|b| Some((b, side.keep(e.addr, e.value)?)));
                    match now {
                        Some((before, v)) => {
                            e.value = v;
                            kept(e.addr, before, v);
                            e.state = State::Settle { left: self.settle };
                            Self::settled(side, e, 0)
                        }
                        _ => {
                            side.hold(e.addr, None, e.over);
                            true
                        }
                    }
                }
                State::Settle { .. } => Self::settled(side, e, frames),
            };
            if done {
                *slot = None;
            }
        }
    }

    /// Stay's hand lets go once the own value has arrived, or after
    /// [`SETTLE_S`] (`took`: frames since the last look).
    fn settled(side: &mut dyn OwnSide, e: &mut Own, took: u32) -> bool {
        let State::Settle { left } = e.state else {
            return false;
        };
        let there = side
            .beneath(e.addr)
            .is_none_or(|(_, own)| (own - e.value).abs() <= 1e-4 * (1.0 + e.value.abs()));
        let left = left.saturating_sub(took);
        if there || left == 0 {
            side.hold(e.addr, None, e.over);
            return true;
        }
        e.state = State::Settle { left };
        false
    }

    /// The status line's words.
    pub fn describe(&self) -> String {
        format!(
            "XY on a synth's, a pad's or an arp's own settings: {} held, {} unreached, {} with no room",
            self.holding(),
            self.unreached,
            self.full
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 1000;

    /// A synth's settings as `locks::Under` holds them: each with its own
    /// value, a lock maybe, and the pad's hand.
    #[derive(Default)]
    struct Side {
        own: Vec<(ParamAddr, f32)>,
        lock: Vec<(ParamAddr, f32)>,
        hand: Vec<(ParamAddr, f32)>,
        /// Stay's own values arrive this many looks late (a pad's: the
        /// screen sends the kit's next view).
        late: u32,
        waiting: Vec<(ParamAddr, f32, u32)>,
        held: Vec<(ParamAddr, Option<f32>)>,
    }

    impl Side {
        fn get(v: &[(ParamAddr, f32)], a: ParamAddr) -> Option<f32> {
            v.iter().find(|(x, _)| *x == a).map(|(_, v)| *v)
        }
        fn put(v: &mut Vec<(ParamAddr, f32)>, a: ParamAddr, x: Option<f32>) {
            v.retain(|(y, _)| *y != a);
            if let Some(x) = x {
                v.push((a, x));
            }
        }
        /// What plays now.
        fn plays(&self, a: ParamAddr) -> Option<f32> {
            Self::get(&self.hand, a)
                .or(Self::get(&self.lock, a))
                .or(Self::get(&self.own, a))
        }
    }

    impl OwnSide for Side {
        fn hold(&mut self, addr: ParamAddr, value: Option<f32>, _over: bool) -> bool {
            if Self::get(&self.own, addr).is_none() {
                return false;
            }
            self.held.push((addr, value));
            Self::put(&mut self.hand, addr, value);
            true
        }
        fn beneath(&self, addr: ParamAddr) -> Option<(f32, f32)> {
            // A late Stay arrives on a look.
            let own = Self::get(&self.own, addr)?;
            Some((Self::get(&self.lock, addr).unwrap_or(own), own))
        }
        fn keep(&mut self, addr: ParamAddr, value: f32) -> Option<f32> {
            if self.late == 0 {
                Self::put(&mut self.own, addr, Some(value));
            } else {
                self.waiting.push((addr, value, self.late));
            }
            Some(value)
        }
    }

    impl Side {
        /// A period passes: late own values come nearer.
        fn period(&mut self) {
            let mut arrived = Vec::new();
            for w in self.waiting.iter_mut() {
                w.2 -= 1;
                if w.2 == 0 {
                    arrived.push((w.0, w.1));
                }
            }
            self.waiting.retain(|w| w.2 > 0);
            for (a, v) in arrived {
                Self::put(&mut self.own, a, Some(v));
            }
        }
    }

    fn cutoff() -> ParamAddr {
        ParamAddr::new(Owner::Inst(1), 3)
    }

    fn side() -> Side {
        Side {
            own: vec![(cutoff(), 0.2)],
            ..Side::default()
        }
    }

    #[test]
    fn only_a_synths_a_pads_and_an_arps_own_settings_are_here() {
        assert!(is_own(cutoff()));
        assert!(is_own(ParamAddr::new(Owner::Pad(0, 4), 1)));
        assert!(is_own(ParamAddr::new(Owner::Arp(2), 1)));
        assert!(!is_own(ParamAddr::new(
            Owner::Strip(mpfree_play::mpfree_param::Strip::Master),
            0
        )));
    }

    /// The pad's value plays and the own value is left alone; Go back
    /// glides to what is underneath as it is by then (a lock that came
    /// meanwhile), then lets go.
    #[test]
    fn go_back_glides_to_what_is_underneath_by_then() {
        let (mut h, mut s) = (OwnHands::new(RATE), side());
        assert!(h.touch(cutoff(), 0.9, false));
        h.apply(10, &mut s, |_, _, _| panic!("no Stay"));
        assert_eq!(s.plays(cutoff()), Some(0.9));
        h.apply(10, &mut s, |_, _, _| {});
        assert_eq!(s.held.len(), 1, "a value goes once");
        // A lock comes while the pad holds it (recording: under the pad).
        s.lock.push((cutoff(), 0.5));
        h.let_go(cutoff(), 100);
        let mut seen = Vec::new();
        for _ in 0..10 {
            h.apply(10, &mut s, |_, _, _| {});
            seen.push(s.plays(cutoff()).unwrap());
        }
        assert!(seen[0] < 0.9 && seen[0] > 0.85, "{seen:?}");
        assert!(seen.windows(2).all(|w| w[1] <= w[0]), "{seen:?}");
        assert_eq!(seen[9], 0.5, "the lock plays: let go");
        assert!(!h.holds(cutoff()));
        assert_eq!(Side::get(&s.own, cutoff()), Some(0.2), "left alone");
    }

    /// Stay: the pad's value becomes the own value, one turn for UNDO with
    /// the own value before; the hand stays on until it has arrived.
    #[test]
    fn stay_keeps_the_value_and_lets_go_once_it_has_arrived() {
        let (mut h, mut s) = (OwnHands::new(RATE), side());
        s.late = 3;
        h.touch(cutoff(), 0.9, false);
        h.apply(10, &mut s, |_, _, _| {});
        h.keep(cutoff(), 0.8);
        let mut kept = Vec::new();
        h.apply(10, &mut s, |a, b, c| kept.push((a, b, c)));
        assert_eq!(kept, [(cutoff(), 0.2, 0.8)]);
        for _ in 0..2 {
            s.period();
            h.apply(10, &mut s, |_, _, _| panic!("once"));
            assert!(h.holds(cutoff()), "waits for the own value");
        }
        s.period();
        h.apply(10, &mut s, |_, _, _| {});
        assert!(!h.holds(cutoff()));
        assert_eq!(s.plays(cutoff()), Some(0.8), "nothing jumped");
        // One that never arrives lets go after SETTLE_S.
        s.late = 10_000;
        h.touch(cutoff(), 0.3, false);
        h.keep(cutoff(), 0.3);
        let mut periods = 0;
        while h.holds(cutoff()) {
            h.apply(10, &mut s, |_, _, _| {});
            periods += 1;
            assert!(periods < 200, "never stays held");
        }
        assert!(periods >= (RATE as f32 * SETTLE_S) as u32 / 10);
    }

    /// A setting that isn't there holds nothing, and a full table says so;
    /// a let-go at once goes through on the next period.
    #[test]
    fn nothing_reached_holds_nothing_and_a_let_go_always_goes() {
        let (mut h, mut s) = (OwnHands::new(RATE), side());
        let gone = ParamAddr::new(Owner::Inst(5), 3);
        h.touch(gone, 0.5, false);
        h.apply(10, &mut s, |_, _, _| {});
        assert_eq!(h.unreached, 1);
        assert!(!h.holds(gone));
        for p in 0..MAX as u16 {
            assert!(h.touch(ParamAddr::new(Owner::Inst(0), p), 0.5, false));
        }
        assert!(!h.touch(cutoff(), 0.5, false));
        assert_eq!(h.full, 1);
        h.touch(ParamAddr::new(Owner::Inst(0), 0), 0.5, false);
        h.let_go(ParamAddr::new(Owner::Inst(0), 0), 0);
        h.apply(10, &mut s, |_, _, _| {});
        assert_eq!(h.holding(), 0, "every one let go or unreached");
    }
}
