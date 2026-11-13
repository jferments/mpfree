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
//! - The chain editor's and the macros panel's set-up changes (a macro's
//!   targets, matrix rows, scenes) come in as [`Change`]s
//!   ([`ModMix::change`]); a parameter a change names becomes a target
//!   then. The set-up saves as text with the macros' names
//!   ([`ModMix::save`], [`ModMix::load`]). See `modpanels.rs`.
//!
//! Locks, turns and [`ModMix::process`] don't allocate after
//! [`ModMix::new`]; set-up changes and saving run on the same loop between
//! periods, as the mixer panel's edits do, and may.

use mpfree_fx::mpfree_node::{Ctx, ParamInfo, Timed};
use mpfree_fx::{mixer_param_applies as param_applies, strip_params, Mixer, StripId};
use mpfree_mod::{
    Change, ModEngine, ModError, ModEvent, ModTimed, DEFAULT_TICK, FOLLOWERS, MACROS,
};
use mpfree_perform::morph::Loaded;
use mpfree_perform::{save_scene, SceneMorph};
use mpfree_play::mpfree_param::{Owner, ParamAddr, Strip};
use mpfree_seq::Project;

/// Events one period can carry (locks, unlocks, the host's turns).
const EVENTS: usize = 256;

pub struct ModMix {
    engine: ModEngine,
    events: Vec<ModTimed>,
    unrouted: u64,
    dropped: u64,
    /// The macros' names on their cards ("Wash", "Grit"), saved with the
    /// set-up.
    names: Vec<String>,
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
pub fn describe(mixer: &Mixer, addr: ParamAddr) -> Option<(ParamInfo, f32)> {
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
            names: (1..=MACROS).map(|m| format!("Macro {m}")).collect(),
        }
    }

    pub fn engine(&self) -> &ModEngine {
        &self.engine
    }

    pub fn names(&self) -> &[String] {
        &self.names
    }

    /// Renames macro `m` (a blank name puts back "Macro n").
    // Used once the panels are on the app's screens (after round 9).
    #[allow(dead_code)]
    pub fn rename(&mut self, m: usize, name: &str) {
        if let Some(n) = self.names.get_mut(m) {
            let name = name.trim();
            *n = if name.is_empty() {
                format!("Macro {}", m + 1)
            } else {
                name.to_string()
            };
        }
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
                // Not held (the engine full, or no such parameter): the
                // lock passes through as a plain value, as for a parameter
                // the engine doesn't know.
                self.hold(mixer, addr);
                ModEvent::Lock { addr, value }
            }
            None => ModEvent::Unlock { addr },
        };
        self.queue(offset, ev);
    }

    /// A move lane's value (an FX move played from a clip): it holds the
    /// parameter as a lock does, but a hand on the control beats it
    /// (`ModEvent::Lane`). The lane's end comes as a lock's `None`.
    pub fn lane(&mut self, mixer: &Mixer, offset: u32, addr: ParamAddr, value: f32) {
        self.hold(mixer, addr);
        self.queue(offset, ModEvent::Lane { addr, value });
    }

    /// Makes a mixer parameter a target, with the value the mixer has now,
    /// if it isn't one. False when the engine can't hold it (full, or not
    /// a parameter the mixer has). Macros and the morph always are.
    pub fn hold(&mut self, mixer: &Mixer, addr: ParamAddr) -> bool {
        if self.engine.is_target(addr) {
            return true;
        }
        describe(mixer, addr)
            .is_some_and(|(info, now)| self.engine.add_target(addr, info, now).is_ok())
    }

    /// A hand on a control (a knob on the chain editor, a macro, the
    /// morph), by `ModEvent::hand`'s rule: a `Set`, or a `Touch` that beats
    /// the step locks while `recording`. False when the engine doesn't
    /// hold `addr`: then the host sets it itself (nothing can lock or
    /// modulate a parameter the engine doesn't hold).
    pub fn hand(&mut self, addr: ParamAddr, value: f32, recording: bool) -> bool {
        if !self.engine.is_target(addr) {
            return false;
        }
        self.queue(0, ModEvent::hand(addr, value, false, recording));
        true
    }

    /// The hand let go of `addr` (nothing happens after a plain `Set`).
    pub fn release(&mut self, addr: ParamAddr) {
        if self.engine.is_target(addr) {
            self.queue(0, ModEvent::Untouch { addr });
        }
    }

    /// A set-up change from the chain editor or the macros panel. A
    /// parameter it maps or modulates becomes a target first.
    pub fn change(&mut self, mixer: &Mixer, c: Change) -> Result<(), ModError> {
        match c {
            Change::Map { map, .. } if !self.hold(mixer, map.target) => {
                return Err(ModError::Unknown(map.target))
            }
            Change::AddRow(row) if !self.hold(mixer, row.target) => {
                return Err(ModError::Unknown(row.target))
            }
            _ => {}
        }
        self.engine.change(c)
    }

    /// An insert left its strip: the engine stops holding its parameters
    /// (and their macro maps and rows go with them).
    pub fn forget(&mut self, owner: Owner) {
        let gone: Vec<ParamAddr> = self.engine.targets().filter(|a| a.owner == owner).collect();
        for a in gone {
            self.engine.remove_target(a);
        }
    }

    /// The set-up as `key = value` lines: [`ModEngine::save`]'s, then the
    /// macros' names (`name.0 = Wash`) for any not called "Macro n".
    // Used once the panels are on the app's screens (after round 9).
    #[allow(dead_code)]
    pub fn save(&self) -> String {
        use std::fmt::Write as _;
        let mut out = self.engine.save();
        for (m, n) in self.names.iter().enumerate() {
            if *n != format!("Macro {}", m + 1) {
                let _ = writeln!(out, "name.{m} = {n}");
            }
        }
        out
    }

    /// Takes back what [`ModMix::save`] wrote, once the mixer has its
    /// inserts (a line about a parameter it doesn't have is skipped).
    /// Returns how many lines were skipped.
    // Used once the panels are on the app's screens (after round 9).
    #[allow(dead_code)]
    pub fn load(&mut self, mixer: &Mixer, text: &str) -> usize {
        let mut rest = String::new();
        let mut names: Vec<String> = (1..=MACROS).map(|m| format!("Macro {m}")).collect();
        for line in text.lines() {
            let named = line.trim().strip_prefix("name.").and_then(|l| {
                let (m, n) = l.split_once('=')?;
                Some((m.trim().parse::<usize>().ok()?, n.trim()))
            });
            match named {
                Some((m, n)) if m < MACROS && !n.is_empty() => names[m] = n.to_string(),
                _ => {
                    rest.push_str(line);
                    rest.push('\n');
                }
            }
        }
        self.names = names;
        self.engine.load(&rest, &|a| describe(mixer, a))
    }

    /// The scene morph (`mpfree_perform::SceneMorph`): put rows `a` and
    /// `b` of `project` on the engine's scenes A and B. Every mixer
    /// parameter either row stores becomes a target first, with the value
    /// the mixer has now; a pad's sound or an instrument's is counted
    /// unknown (as a lock on one is unrouted). While morphing, turns made
    /// so far go back into the rows on A and B first, and the sound from
    /// before the morph stays the one Snap back goes to.
    pub fn morph_load(
        &mut self,
        mixer: &Mixer,
        morph: &mut SceneMorph,
        project: &mut Project,
        a: usize,
        b: usize,
    ) -> Result<Loaded, ModError> {
        morph.store(&self.engine, project);
        for row in [a, b] {
            if let Some(s) = project.scene(row) {
                for &(addr, _) in &s.values {
                    if !matches!(
                        addr.owner,
                        Owner::Macro(_) | Owner::Morph(_) | Owner::Mod(_)
                    ) {
                        self.hold(mixer, addr);
                    }
                }
            }
        }
        morph.load(&mut self.engine, project, a, b)
    }

    /// The morph's crossfader, 0 (A) to 1 (B), from the next period.
    pub fn morph_to(&mut self, pos: f32) {
        self.queue(
            0,
            ModEvent::Set {
                addr: ParamAddr::morph_pos(0),
                value: pos.clamp(0.0, 1.0),
            },
        );
    }

    /// Let the morph go: back to the sound from before it (`keep` false),
    /// or staying where it has the parameters. Turns made while morphing
    /// are written back into the rows first. Returns the values sent.
    pub fn morph_end(
        &mut self,
        morph: &mut SceneMorph,
        project: &mut Project,
        keep: bool,
    ) -> usize {
        morph.store(&self.engine, project);
        let values = if keep {
            morph.keep(&mut self.engine)
        } else {
            morph.snap_back(&mut self.engine)
        };
        for &(addr, value) in &values {
            self.queue(0, ModEvent::Set { addr, value });
        }
        values.len()
    }

    /// Row `row` stores every parameter's own value now and the macros'
    /// positions. How many it holds, or None (past the last row, too many).
    pub fn save_scene(&self, project: &mut Project, row: usize) -> Option<usize> {
        save_scene(&self.engine, project, row)
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
            h.period(&ctx, (&z, &z), (&z, &z), (&z, &z), (&z, &z), &mut l, &mut r);
        }
    }

    fn vol(t: usize) -> ParamAddr {
        ParamAddr::new(Owner::Strip(Strip::Track(t as u8)), mx::VOLUME)
    }

    #[test]
    fn the_scene_morph_moves_the_mixer_between_two_rows_and_snaps_back() {
        let mut h = MixHost::new(RATE, 0.0, 6.0);
        let level = |h: &MixHost, t: usize| h.mixer.param(StripId::Track(t), mx::VOLUME);
        let (loop_was, drums_was) = (level(&h, LOOP), level(&h, DRUMS));
        let mut p = Project::new();
        p.scene_mut(0).unwrap().set_value(vol(LOOP), -20.0).unwrap();
        p.scene_mut(2).unwrap().set_value(vol(LOOP), -4.0).unwrap();
        // Only row 3 holds the drums: the morph moves them from where they are.
        p.scene_mut(2)
            .unwrap()
            .set_value(vol(DRUMS), -30.0)
            .unwrap();
        let mut m = SceneMorph::default();
        let l = h.modu.morph_load(&h.mixer, &mut m, &mut p, 0, 2).unwrap();
        assert_eq!((l.morphing, l.unknown), (2, 0));
        h.modu.morph_to(0.0);
        run(&mut h, 40, 0);
        assert!((level(&h, LOOP) + 20.0).abs() < 0.05, "{}", level(&h, LOOP));
        assert!((level(&h, DRUMS) - drums_was).abs() < 0.05);
        h.modu.morph_to(1.0);
        run(&mut h, 40, 40);
        assert!((level(&h, LOOP) + 4.0).abs() < 0.05, "{}", level(&h, LOOP));
        assert!(
            (level(&h, DRUMS) + 30.0).abs() < 0.05,
            "{}",
            level(&h, DRUMS)
        );
        h.modu.morph_to(0.5);
        run(&mut h, 40, 80);
        let mid = level(&h, LOOP);
        assert!(mid > -20.0 && mid < -4.0, "{mid}");
        // Snap back: the sound from before the morph.
        assert_eq!(h.modu.morph_end(&mut m, &mut p, false), 3);
        run(&mut h, 40, 120);
        assert!(
            (level(&h, LOOP) - loop_was).abs() < 0.05,
            "{}",
            level(&h, LOOP)
        );
        assert!((level(&h, DRUMS) - drums_was).abs() < 0.05);
        // Keep: where the morph had them stays.
        h.modu.morph_load(&h.mixer, &mut m, &mut p, 0, 2).unwrap();
        h.modu.morph_to(1.0);
        run(&mut h, 40, 160);
        h.modu.morph_end(&mut m, &mut p, true);
        run(&mut h, 40, 200);
        assert!((level(&h, LOOP) + 4.0).abs() < 0.05, "{}", level(&h, LOOP));
        // A row stores the mixer as it is now.
        assert!(h.modu.save_scene(&mut p, 5).is_some_and(|n| n >= 2));
        let v = p.scene(5).unwrap().value(vol(LOOP)).unwrap();
        assert!((v + 4.0).abs() < 0.05, "{v}");
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
