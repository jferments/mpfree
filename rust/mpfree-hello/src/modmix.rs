//! The mod engine in the pad player: `mpfree-mod`'s [`ModEngine`] between
//! the clips' parameter locks and the mixer (its strips' own controls and
//! their inserts). Macros, the matrix and the Q-Link bindings come through
//! here too when the chain editor and the macros panel open in the app.
//!
//! - A step's lock (`Event::Lock` from the player) holds a mixer parameter
//!   until its `Unlock`, which puts back the value underneath, on the frame
//!   the player gives.
//! - A parameter becomes a target on its first lock, with the value the
//!   mixer has then, so the engine costs next to nothing until a clip locks
//!   something. From then on the host's own turns of that control (the
//!   mixer panel, the Play page) go through the engine as `Set`s
//!   ([`ModMix::set`]), so what comes back after a lock is the last turn.
//! - Strips and inserts are routed. A lock on a pad's sound or an
//!   instrument (`Owner::Pad`, `Owner::Inst`) is counted
//!   ([`ModMix::unrouted`]) and dropped until those engines take parameter
//!   events. An insert is found by its slot id, so a lock follows the effect
//!   when it moves.
//!
//! Nothing here allocates after [`ModMix::new`].

use mpfree_fx::mpfree_node::{Ctx, ParamInfo, Timed};
use mpfree_fx::{mixer_param_applies as param_applies, strip_params, Mixer, StripId};
use mpfree_mod::{ModEngine, ModEvent, ModTimed, DEFAULT_TICK, FOLLOWERS};
use mpfree_play::mpfree_param::{Owner, ParamAddr, Strip};

/// Events one period can carry (locks, unlocks, the host's turns).
const EVENTS: usize = 256;

pub struct ModMix {
    engine: ModEngine,
    events: Vec<ModTimed>,
    unrouted: u64,
    dropped: u64,
}

/// The mixer's id for a strip.
pub fn strip_id(s: Strip) -> StripId {
    match s {
        Strip::Input(i) => StripId::Input(i.into()),
        Strip::Pad(i) => StripId::Pad(i.into()),
        Strip::Track(i) => StripId::Track(i.into()),
        Strip::Bus(i) => StripId::Bus(i.into()),
        Strip::Return(i) => StripId::Return(i.into()),
        Strip::Master => StripId::Master,
    }
}

/// The parameter address's name for a mixer strip (`None` past 255).
pub fn strip_of(id: StripId) -> Option<Strip> {
    let n = |i: usize| u8::try_from(i).ok();
    Some(match id {
        StripId::Input(i) => Strip::Input(n(i)?),
        StripId::Pad(i) => Strip::Pad(n(i)?),
        StripId::Track(i) => Strip::Track(n(i)?),
        StripId::Bus(i) => Strip::Bus(n(i)?),
        StripId::Return(i) => Strip::Return(n(i)?),
        StripId::Master => Strip::Master,
    })
}

/// A mixer parameter's description and its value now, if `addr` names one
/// the mixer has.
fn describe(mixer: &Mixer, addr: ParamAddr) -> Option<(ParamInfo, f32)> {
    match addr.owner {
        Owner::Strip(s) => {
            let sid = strip_id(s);
            if !param_applies(sid, addr.param) || !mixer.strips().any(|x| x == sid) {
                return None;
            }
            let info = strip_params().iter().find(|p| p.id == addr.param)?;
            Some((*info, mixer.param(sid, addr.param)))
        }
        Owner::Insert(s, uid) => {
            let c = mixer.chain(strip_id(s))?;
            let n = c.node(c.slot_of(uid)?)?;
            let info = n.params().iter().find(|p| p.id == addr.param)?;
            Some((*info, n.param(addr.param)))
        }
        _ => None,
    }
}

impl ModMix {
    /// Made before the sound starts (it allocates).
    pub fn new(rate: u32) -> ModMix {
        ModMix {
            engine: ModEngine::new(rate, DEFAULT_TICK),
            events: Vec::with_capacity(EVENTS),
            unrouted: 0,
            dropped: 0,
        }
    }

    #[cfg(test)]
    pub fn engine(&self) -> &ModEngine {
        &self.engine
    }

    /// Locks and values the mixer has no place for (a pad's sound, an
    /// instrument, a removed insert).
    #[cfg(test)]
    pub fn unrouted(&self) -> u64 {
        self.unrouted
    }

    /// The mod engine's part of the status line.
    pub fn describe(&self) -> String {
        format!(
            "mod: {} targets, {} values sent, {} with no place in the mixer, {} events dropped",
            self.engine.targets().count(),
            self.engine.stats().sent,
            self.unrouted,
            self.dropped
        )
    }

    /// A step's lock (`value`) or its end (`None`) at `offset` frames into
    /// the period. A mixer parameter becomes a target on its first lock.
    pub fn lock(&mut self, mixer: &Mixer, offset: u32, addr: ParamAddr, value: Option<f32>) {
        let ev = match value {
            Some(value) => {
                if !self.engine.is_target(addr) {
                    if let Some((info, now)) = describe(mixer, addr) {
                        // Full (512 targets): the lock passes through as a
                        // plain value, as for a parameter it doesn't know.
                        let _ = self.engine.add_target(addr, info, now);
                    }
                }
                ModEvent::Lock { addr, value }
            }
            None => ModEvent::Unlock { addr },
        };
        self.queue(offset, ev);
    }

    /// A turn of a mixer control by the host. Returns false when the engine
    /// doesn't hold it: then the host sets the mixer itself.
    pub fn set(&mut self, addr: ParamAddr, value: f32) -> bool {
        if !self.engine.is_target(addr) {
            return false;
        }
        self.queue(0, ModEvent::Set { addr, value });
        true
    }

    fn queue(&mut self, offset: u32, ev: ModEvent) {
        if self.events.len() == self.events.capacity() {
            self.dropped += 1;
            return;
        }
        let at = self.events.partition_point(|e| e.offset <= offset);
        self.events.insert(at, ModTimed::new(offset, ev));
    }

    /// Runs one period of `len` frames before the mixer's: what changed
    /// goes into the mixer's queues on its frame.
    pub fn process(&mut self, ctx: &Ctx, len: usize, mixer: &mut Mixer) {
        let unrouted = &mut self.unrouted;
        self.engine.process(
            ctx,
            len,
            &self.events,
            &[None; FOLLOWERS],
            &mut |addr, t| route(mixer, addr, t, unrouted),
        );
        self.events.clear();
    }
}

fn route(mixer: &mut Mixer, addr: ParamAddr, t: Timed, unrouted: &mut u64) {
    let ok = match addr.owner {
        Owner::Strip(s) => mixer.queue(strip_id(s), t),
        Owner::Insert(s, uid) => {
            let sid = strip_id(s);
            match mixer.chain(sid).and_then(|c| c.slot_of(uid)) {
                Some(slot) => mixer.queue_insert(sid, slot, t),
                None => false,
            }
        }
        // The macros' and the morph's own values (for their Q-Link cards).
        Owner::Macro(_) | Owner::Morph(_) | Owner::Mod(_) => true,
        _ => false,
    };
    if !ok {
        *unrouted += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mixing::{MixHost, DRUMS, LOOP};
    use crate::PERIOD;
    use mpfree_fx::ids::{dj_filter, mixer as mx};
    use mpfree_fx::Edit;

    const RATE: u32 = 44_100;

    fn run(h: &mut MixHost, periods: u64, from: u64) {
        let z = [0f32; PERIOD];
        let (mut l, mut r) = ([0f32; PERIOD], [0f32; PERIOD]);
        for i in from..from + periods {
            let ctx = Ctx::new(RATE, i * PERIOD as u64);
            h.period(&ctx, (&z, &z), (&z, &z), (&z, &z), &mut l, &mut r);
        }
    }

    fn vol(t: usize) -> ParamAddr {
        ParamAddr::new(Owner::Strip(Strip::Track(t as u8)), mx::VOLUME)
    }

    #[test]
    fn a_step_lock_holds_a_strip_control_and_its_end_puts_back_the_last_turn() {
        let mut h = MixHost::new(RATE, 0.0, 6.0);
        let before = h.mixer.param(StripId::Track(LOOP), mx::VOLUME);
        h.lock(10, vol(LOOP), Some(-18.0));
        run(&mut h, 40, 0);
        assert!((h.mixer.param(StripId::Track(LOOP), mx::VOLUME) + 18.0).abs() < 0.01);
        assert!(h.modu.engine().is_target(vol(LOOP)));
        // A turn on the mixer panel while the lock holds: heard after it.
        h.action(&mpfree_mixer_ui::MixAction::Set {
            strip: StripId::Track(LOOP),
            id: mx::VOLUME,
            value: -6.0,
        });
        run(&mut h, 40, 40);
        assert!((h.mixer.param(StripId::Track(LOOP), mx::VOLUME) + 18.0).abs() < 0.01);
        h.lock(0, vol(LOOP), None);
        run(&mut h, 40, 80);
        let after = h.mixer.param(StripId::Track(LOOP), mx::VOLUME);
        assert!((after + 6.0).abs() < 0.01, "{before} -> {after}");
        assert_eq!(h.modu.unrouted(), 0);
    }

    #[test]
    fn a_lock_on_an_insert_follows_the_effect_by_its_slot_id() {
        let mut h = MixHost::new(RATE, 0.0, 6.0);
        // The Play page's inserts go in once their strips duck (~10 ms).
        run(&mut h, 20, 0);
        assert_eq!(h.mixer.edits_pending(), 0);
        let s = StripId::Track(DRUMS);
        let uid = h.mixer.chain(s).unwrap().uid(0).unwrap();
        let addr = ParamAddr::new(
            Owner::Insert(strip_of(s).unwrap(), uid),
            dj_filter::POSITION,
        );
        let pos = |h: &MixHost| {
            let c = h.mixer.chain(s).unwrap();
            c.node(c.slot_of(uid).unwrap())
                .unwrap()
                .param(dj_filter::POSITION)
        };
        let rest = pos(&h);
        h.lock(0, addr, Some(-0.7));
        run(&mut h, 4, 20);
        assert!((pos(&h) + 0.7).abs() < 1e-4, "{}", pos(&h));
        // Put another effect before it: the lock's end still finds it.
        let n = Box::new(mpfree_fx::Drive::new());
        let insert = mpfree_fx::Insert::new(n, RATE, PERIOD);
        assert!(h
            .mixer
            .edit(Edit::Insert {
                strip: s,
                at: 0,
                insert,
            })
            .is_ok());
        run(&mut h, 20, 24);
        assert_eq!(h.mixer.chain(s).unwrap().slot_of(uid), Some(1));
        h.lock(0, addr, None);
        run(&mut h, 4, 44);
        assert!((pos(&h) - rest).abs() < 1e-4, "{} vs {rest}", pos(&h));
        assert_eq!(h.modu.unrouted(), 0);
    }

    #[test]
    fn locks_the_mixer_has_no_place_for_are_counted_not_applied() {
        let mut h = MixHost::new(RATE, 0.0, 6.0);
        let pad = ParamAddr::new(Owner::Pad(0, 3), 1);
        h.lock(0, pad, Some(0.5));
        run(&mut h, 2, 0);
        assert!(!h.modu.engine().is_target(pad));
        assert_eq!(h.modu.unrouted(), 1);
        // A turn of a control no lock has touched goes straight to the mixer.
        let mut m = ModMix::new(RATE);
        assert!(!m.set(vol(DRUMS), -3.0));
    }
}
