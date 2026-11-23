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
//!   instrument (`Owner::Pad`, `Owner::Inst`) goes to the pads first
//!   (`time::Pads::lock_at`: the sampler engine, the synth on the pads, a
//!   track's own instrument; see `locks.rs`); one none of them takes
//!   reaches here and is counted ([`ModMix::unrouted`]) and dropped. An
//!   insert is found by its slot id, so a lock follows the effect when it
//!   moves.
//!
//! - The chain editor's and the macros panel's set-up changes (a macro's
//!   targets, matrix rows, scenes) come in as [`Change`]s
//!   ([`ModMix::change`]); a parameter a change names becomes a target
//!   then. The set-up saves as text with the macros' names
//!   ([`ModMix::save`], [`ModMix::load`]). See `modpanels.rs`.
//!
//! - A track's MIDI effect settings (`Owner::NoteFx`) are targets too: a
//!   macro, an LFO or a step's lock moves them. The engine knows them from
//!   the tracks' effects as the sound loop has them
//!   ([`ModMix::set_note_fx`]), and their values go to the time engine
//!   ([`ModMix::take_note_values`], `TimeHost::modulate_note_fx`), which
//!   puts them on the track's chain from the next period.
//!
//! - A track's controllers, pitch bend and pressure out to its outside
//!   gear (`Owner::Midi`, the CC lanes of sequencer S6) are targets too,
//!   held quietly ([`ModEngine::add_quiet_target`]): nothing goes to the
//!   gear until a lock, a lane, a macro, a row, the morph or a hand moves
//!   one, since MPFree can't know where the gear's knob is. Each starts at
//!   General MIDI 2's value (`mpfree_param::midi::default`), which a lock's
//!   end goes back to until a hand or a saved project sets another. Their
//!   values go to MIDI out in the same period, on their frames
//!   ([`ModMix::take_midi_values`], `MidiOut::control`). A value set by
//!   hand is saved with the set-up (`base.t3.midi.74 = 90`).
//!
//! Locks, turns and [`ModMix::process`] don't allocate after
//! [`ModMix::new`]; set-up changes and saving run on the same loop between
//! periods, as the mixer panel's edits do, and may.

use crate::hand::{Hand, Hands, Move};
use mpfree_fx::mpfree_node::{Ctx, NodeEvent, ParamInfo, Timed};
use mpfree_fx::{mixer_param_applies as param_applies, strip_params, Mixer, StripId};
use mpfree_mod::{
    Change, LetGo, ModEngine, ModError, ModEvent, ModTimed, Source, DEFAULT_TICK, FOLLOWERS, MACROS,
};
use mpfree_notefx::TrackFx;
use mpfree_perform::morph::Loaded;
use mpfree_perform::{save_scene, SceneMorph};
use mpfree_play::mpfree_param::{Owner, ParamAddr, Strip};
use mpfree_seq::{Project, MAX_TRACKS};

/// Events one period can carry (locks, unlocks, the host's turns).
const EVENTS: usize = 256;

/// MIDI effect values one period can send on.
const NOTE_VALUES: usize = 256;

/// Values for the tracks' MIDI out controls one period can send on.
const MIDI_VALUES: usize = 512;

pub struct ModMix {
    engine: ModEngine,
    events: Vec<ModTimed>,
    unrouted: u64,
    dropped: u64,
    /// Let-gos that found the period's events full and were kept on their
    /// target for the next period ([`ModEngine::let_go_later`]).
    late: u64,
    /// Values that found the period's events full and took the place of an
    /// earlier value for the same setting.
    merged: u64,
    /// Who holds which setting by hand (`hand.rs`).
    hands: Hands,
    /// The tracks' MIDI effects, for their settings' descriptions.
    note_fx: Box<[TrackFx; MAX_TRACKS]>,
    /// This period's values on MIDI effect settings, for the time engine.
    note_values: Vec<(ParamAddr, f32)>,
    /// This period's values on the tracks' MIDI out controls, with their
    /// frames, for MIDI out.
    midi_values: Vec<(u32, ParamAddr, f32)>,
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
            late: 0,
            merged: 0,
            hands: Hands::new(rate),
            note_fx: Box::new([TrackFx::default(); MAX_TRACKS]),
            note_values: Vec::with_capacity(NOTE_VALUES),
            midi_values: Vec::with_capacity(MIDI_VALUES),
            names: (1..=MACROS).map(|m| format!("Macro {m}")).collect(),
        }
    }

    /// The tracks' MIDI effects, as the sound loop has them (with their
    /// ids): what the engine knows a MIDI effect setting by. Copies, never
    /// allocates.
    pub fn set_note_fx(&mut self, fx: &[TrackFx; MAX_TRACKS]) {
        *self.note_fx = *fx;
    }

    /// A parameter's description and value now: a mixer's, or a track's
    /// MIDI effect setting.
    pub fn param(&self, mixer: &Mixer, addr: ParamAddr) -> Option<(ParamInfo, f32)> {
        match addr.owner {
            Owner::NoteFx(..) => note_param(&self.note_fx, addr),
            Owner::Midi(_) => midi_param(addr),
            _ => describe(mixer, addr),
        }
    }

    /// A MIDI effect setting's name on the macro cards: "Chord inversion
    /// (Track 3)".
    pub fn note_label(&self, addr: ParamAddr) -> Option<String> {
        let Owner::NoteFx(t, id) = addr.owner else {
            return None;
        };
        let fx = self.note_fx.get(usize::from(t))?;
        let fx = fx.slots[fx.place_of(id)?]?.fx;
        let setting = fx.settings().get(usize::from(addr.param))?.name;
        let name = fx.name();
        Some(if setting.starts_with(name) {
            format!("{setting} (Track {})", u16::from(t) + 1)
        } else {
            format!(
                "{name} {} (Track {})",
                setting.to_lowercase(),
                u16::from(t) + 1
            )
        })
    }

    /// This period's values on MIDI effect settings (`Owner::NoteFx`):
    /// for `TimeHost::modulate_note_fx`, then cleared.
    pub fn take_note_values(&mut self) -> std::vec::Drain<'_, (ParamAddr, f32)> {
        self.note_values.drain(..)
    }

    /// This period's values on the tracks' MIDI out controls
    /// (`Owner::Midi`), each with its frame in the period: for
    /// `MidiOut::control`, then cleared.
    pub fn take_midi_values(&mut self) -> std::vec::Drain<'_, (u32, ParamAddr, f32)> {
        self.midi_values.drain(..)
    }

    pub fn engine(&self) -> &ModEngine {
        &self.engine
    }

    /// The mod matrix rows aimed at `addr`: each one's place in the
    /// matrix (for a depth change or removal), source and depth.
    pub fn rows_on(&self, addr: ParamAddr) -> Vec<(usize, Source, f32)> {
        self.engine
            .rows()
            .enumerate()
            .filter(|(_, r)| r.target == addr)
            .map(|(i, r)| (i, r.source, r.depth))
            .collect()
    }

    /// The tracks' MIDI out controls (`Owner::Midi`) a matrix row moves
    /// (an LFO on a gear's CC lane), each once, in matrix order.
    pub fn midi_moved(&self) -> Vec<ParamAddr> {
        let mut v = Vec::new();
        self.midi_moved_into(&mut v);
        v
    }

    /// [`ModMix::midi_moved`] into `v`, cleared first: no allocation once
    /// it has the room (the audio loop checks a few times a second).
    pub fn midi_moved_into(&self, v: &mut Vec<ParamAddr>) {
        v.clear();
        for r in self.engine.rows() {
            if matches!(r.target.owner, Owner::Midi(_)) && !v.contains(&r.target) {
                v.push(r.target);
            }
        }
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
            "mod: {} targets, {} values sent, {} with no place in the mixer, {} events dropped, \
             {} let-gos kept aside, {} values merged; hands: {} held, {} taken by a newer \
             gesture, {} moves ignored, {} MIDI gestures ended, {} with no room",
            self.engine.targets().count(),
            self.engine.stats().sent,
            self.unrouted,
            self.dropped,
            self.late,
            self.merged,
            self.hands.holding(),
            self.hands.taken,
            self.hands.ignored,
            self.hands.ended,
            self.hands.full
        )
    }

    /// Let-gos kept for the next period, and values merged, with the
    /// period's events full (the tests').
    #[cfg(test)]
    pub fn waited(&self) -> (u64, u64, u64) {
        (self.late, self.merged, self.dropped)
    }

    /// Lets go of every hand, lock and lane at once: a project is opening.
    pub fn let_go_all(&mut self) {
        self.hands.clear();
        self.events.retain(|e| {
            !matches!(
                e.ev,
                ModEvent::Touch { .. } | ModEvent::Lock { .. } | ModEvent::Lane { .. }
            )
        });
        self.engine.let_go_all();
    }

    /// A step's lock (`value`) or its end (`None`) at `offset` frames into
    /// the period. A mixer parameter becomes a target on its first lock.
    pub fn lock(&mut self, mixer: &Mixer, offset: u32, addr: ParamAddr, value: Option<f32>) {
        let ev = match value {
            Some(value) => {
                // Not held (the engine full, or no such parameter): the
                // lock passes through as a plain value, as for a parameter
                // the engine doesn't know.
                if !self.hold_or_skip(mixer, addr) {
                    return;
                }
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
        if self.hold_or_skip(mixer, addr) {
            self.queue(offset, ModEvent::Lane { addr, value });
        }
    }

    /// Holds `addr` for a lock or a lane. False only for a MIDI effect
    /// setting or a MIDI out control the engine can't hold (its effect
    /// isn't on the track, a channel mode message, or the engine is full):
    /// its lock is dropped, since a plain value would hold it with nothing
    /// to let go of it. Anything else passes through as a plain value when
    /// not held.
    fn hold_or_skip(&mut self, mixer: &Mixer, addr: ParamAddr) -> bool {
        self.hold(mixer, addr) || !matches!(addr.owner, Owner::NoteFx(..) | Owner::Midi(_))
    }

    /// Makes a mixer parameter a target, with the value the mixer has now,
    /// if it isn't one. False when the engine can't hold it (full, or not
    /// a parameter the mixer has). Macros and the morph always are.
    pub fn hold(&mut self, mixer: &Mixer, addr: ParamAddr) -> bool {
        if self.engine.is_target(addr) {
            return true;
        }
        let Some((info, now)) = self.param(mixer, addr) else {
            return false;
        };
        match addr.owner {
            // Outside gear: nothing is sent until something moves it.
            Owner::Midi(_) => self.engine.add_quiet_target(addr, info, now).is_ok(),
            _ => self.engine.add_target(addr, info, now).is_ok(),
        }
    }

    /// `who`'s hand on a control (a knob on the chain editor, a macro, the
    /// morph, a learned MIDI control), by `ModEvent::hand`'s rule: a `Set`,
    /// or a `Touch` that beats the step locks while `recording`. A touch
    /// has an owner (`hand.rs`): a new gesture takes the setting, and a
    /// hand that lost it is ignored. False when the engine doesn't hold
    /// `addr`: then the host sets it itself (nothing can lock or modulate
    /// a parameter the engine doesn't hold).
    pub fn hand(&mut self, who: Hand, addr: ParamAddr, value: f32, recording: bool) -> bool {
        // A learned knob on outside gear: only the engine sends it.
        if matches!(addr.owner, Owner::Midi(_)) {
            self.hold_midi(addr);
        }
        if !self.engine.is_target(addr) {
            return false;
        }
        let ev = match ModEvent::hand(addr, value, false, recording) {
            ev @ ModEvent::Touch { .. } => match self.hands.moved(who, addr, value) {
                Move::Took | Move::Again => ev,
                Move::Same | Move::Lost => return true,
                // No room to keep who holds it: a plain value, which
                // never needs letting go.
                Move::Full => ModEvent::Set { addr, value },
            },
            ev => ev,
        };
        self.queue(0, ev);
        true
    }

    /// `who`'s hand let go of `addr` (nothing happens after a plain
    /// `Set`). Only the hand that holds it lets it go.
    pub fn release(&mut self, who: Hand, addr: ParamAddr) {
        if self.engine.is_target(addr) && self.hands.release(who, addr) {
            self.queue(0, ModEvent::Untouch { addr });
        }
    }

    /// The XY pad's hand moves `addr` to `value` (a setting a layout of
    /// yours plays by Set, `xy_play.rs`): a `Touch`, always, so the
    /// setting's own value is left alone and the pad lets go back to it.
    /// It beats a move lane always and a step's lock while `recording`
    /// (`ModEvent::hand`'s rule). The setting becomes a target first.
    /// False when the engine can't hold it.
    pub fn pad_touch(
        &mut self,
        mixer: &Mixer,
        addr: ParamAddr,
        value: f32,
        recording: bool,
    ) -> bool {
        if !self.hold(mixer, addr) {
            return false;
        }
        match self.hands.moved(Hand::Pad, addr, value) {
            Move::Same | Move::Lost => {}
            // With no room to keep who holds it, the touch still holds it:
            // the pad's let-go of a setting nobody holds goes through.
            Move::Took | Move::Again | Move::Full => self.queue(
                0,
                ModEvent::Touch {
                    addr,
                    value,
                    over_locks: recording,
                },
            ),
        }
        true
    }

    /// The XY pad lets go of `addr` over `frames` (Go back:
    /// `ModEvent::LetGo`), if it still holds it.
    pub fn pad_let_go(&mut self, addr: ParamAddr, frames: u32) {
        if self.engine.is_target(addr) && self.hands.release(Hand::Pad, addr) {
            self.queue(0, ModEvent::LetGo { addr, frames });
        }
    }

    /// Stay: the XY pad's value becomes `addr`'s own, as a turn of its
    /// knob, and the pad lets go of it. The own value it had before, for
    /// UNDO; None when the pad no longer held it (a knob took it).
    pub fn pad_keep(&mut self, addr: ParamAddr, value: f32) -> Option<f32> {
        if !self.engine.is_target(addr) || !self.hands.release(Hand::Pad, addr) {
            return None;
        }
        let before = self.engine.base(addr);
        self.queue(0, ModEvent::Set { addr, value });
        self.queue(0, ModEvent::Untouch { addr });
        before
    }

    /// The XY pad's push on host source `n` (`mpfree_mod::XY_SOURCES`),
    /// -1 to 1: what its Add settings' rows add.
    pub fn pad_push(&mut self, n: u8, value: f32) {
        self.queue(0, ModEvent::Source { n, value });
    }

    /// The push on host source `n` as the engine has it now.
    pub fn pushed(&self, n: u8) -> f32 {
        self.engine.source(Source::Host(n))
    }

    /// An Add setting of the XY pad's: a matrix row from host source `n`
    /// to `addr` at `depth` (the setting becomes a target first).
    pub fn pad_row(
        &mut self,
        mixer: &Mixer,
        n: u8,
        addr: ParamAddr,
        depth: f32,
    ) -> Result<(), ModError> {
        self.change(
            mixer,
            Change::AddRow(mpfree_mod::Row {
                source: Source::Host(n),
                via: None,
                target: addr,
                depth,
            }),
        )
    }

    /// Takes the XY pad's row from host source `n` to `addr` out, if it is
    /// there (the chain editor may have taken it out already).
    pub fn pad_unrow(&mut self, n: u8, addr: ParamAddr) {
        let row = self
            .engine
            .rows()
            .position(|r| r.source == Source::Host(n) && r.target == addr);
        if let Some(i) = row {
            let _ = self.engine.change(Change::RemoveRow(i as u8));
        }
    }

    /// Who holds what by hand (`hand.rs`; the tests').
    #[cfg(test)]
    pub fn hands(&self) -> &Hands {
        &self.hands
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
        self.hands.forget(owner);
    }

    /// The set-up as `key = value` lines: [`ModEngine::save`]'s, then the
    /// macros' names (`name.0 = Wash`) for any not called "Macro n", then
    /// each MIDI out control whose own value isn't its default
    /// (`base.t3.midi.74 = 90`).
    // Saved and opened with the project (project.rs).
    pub fn save(&self) -> String {
        use std::fmt::Write as _;
        let mut out = self.engine.save();
        for (m, n) in self.names.iter().enumerate() {
            if *n != format!("Macro {}", m + 1) {
                let _ = writeln!(out, "name.{m} = {n}");
            }
        }
        for a in self.engine.targets() {
            let Owner::Midi(_) = a.owner else { continue };
            let (Some(base), Some(d)) = (
                self.engine.base(a),
                mpfree_play::mpfree_param::midi::default(a.param),
            ) else {
                continue;
            };
            if base != f32::from(d) {
                let _ = writeln!(out, "base.{a} = {base}");
            }
        }
        out
    }

    /// Takes back what [`ModMix::save`] wrote, once the mixer has its
    /// inserts (a line about a parameter it doesn't have is skipped).
    /// Returns how many lines were skipped.
    // Saved and opened with the project (project.rs).
    pub fn load(&mut self, mixer: &Mixer, text: &str) -> usize {
        let mut rest = String::new();
        let mut names: Vec<String> = (1..=MACROS).map(|m| format!("Macro {m}")).collect();
        let mut bases = Vec::new();
        let mut skipped = 0;
        for line in text.lines() {
            let named = line.trim().strip_prefix("name.").and_then(|l| {
                let (m, n) = l.split_once('=')?;
                Some((m.trim().parse::<usize>().ok()?, n.trim()))
            });
            if let Some(b) = line.trim().strip_prefix("base.") {
                let base = b.split_once('=').and_then(|(a, v)| {
                    let a: ParamAddr = a.trim().parse().ok()?;
                    let v: f32 = v.trim().parse().ok()?;
                    (matches!(a.owner, Owner::Midi(_)) && v.is_finite()).then_some((a, v))
                });
                match base {
                    Some(b) => bases.push(b),
                    None => skipped += 1,
                }
                continue;
            }
            match named {
                Some((m, n)) if m < MACROS && !n.is_empty() => names[m] = n.to_string(),
                _ => {
                    rest.push_str(line);
                    rest.push('\n');
                }
            }
        }
        self.names = names;
        let note_fx = &*self.note_fx;
        skipped += self.engine.load(&rest, &|a| match a.owner {
            Owner::NoteFx(..) => note_param(note_fx, a),
            Owner::Midi(_) => midi_param(a),
            _ => describe(mixer, a),
        });
        // A MIDI out control's own value: held, and set, so it goes to the
        // gear once its track plays there (MidiOut keeps it until then).
        for (a, v) in bases {
            if self.hold_midi(a) {
                self.queue(0, ModEvent::Set { addr: a, value: v });
            } else {
                skipped += 1;
            }
        }
        skipped
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
    /// doesn't hold it: then the host sets the mixer itself. A track's MIDI
    /// out control (a learned knob on outside gear) is held first: only the
    /// engine sends it.
    pub fn set(&mut self, addr: ParamAddr, value: f32) -> bool {
        if matches!(addr.owner, Owner::Midi(_)) {
            self.hold_midi(addr);
        }
        if !self.engine.is_target(addr) {
            return false;
        }
        self.queue(0, ModEvent::Set { addr, value });
        true
    }

    /// Holds a MIDI out control (it needs no mixer).
    fn hold_midi(&mut self, addr: ParamAddr) -> bool {
        if self.engine.is_target(addr) {
            return true;
        }
        midi_param(addr)
            .is_some_and(|(info, now)| self.engine.add_quiet_target(addr, info, now).is_ok())
    }

    fn queue(&mut self, offset: u32, ev: ModEvent) {
        if self.events.len() == self.events.capacity() {
            self.full(ev);
            return;
        }
        let at = self.events.partition_point(|e| e.offset <= offset);
        self.events.insert(at, ModTimed::new(offset, ev));
    }

    /// `ev` found the period's events full. A let-go never needs a slot:
    /// its target keeps it and the engine does it first next period, so a
    /// setting never stays held. A value takes the place of the last one
    /// queued for the same setting, when that is the same kind (the newest
    /// value is the one that matters). Anything else is dropped, counted.
    fn full(&mut self, ev: ModEvent) {
        let late = match ev {
            ModEvent::Untouch { addr } | ModEvent::LetGo { addr, .. } => Some((addr, LetGo::Touch)),
            ModEvent::Unlock { addr } => Some((addr, LetGo::Lock)),
            _ => None,
        };
        if let Some((addr, what)) = late {
            // Not a target: nothing holds it, so nothing to let go of.
            if self.engine.let_go_later(addr, what) {
                self.late += 1;
            }
            return;
        }
        if let ModEvent::Source { n, .. } = ev {
            // The XY pad's push: the newest value is the one that matters
            // (and the pad sends it again until it lands).
            let same = self
                .events
                .iter_mut()
                .rev()
                .find(|e| matches!(e.ev, ModEvent::Source { n: m, .. } if m == n));
            match same {
                Some(e) => {
                    e.ev = ev;
                    self.merged += 1;
                }
                None => self.dropped += 1,
            }
            return;
        }
        let Some(addr) = value_addr(&ev) else {
            self.dropped += 1;
            return;
        };
        let same = self
            .events
            .iter_mut()
            .rev()
            .find(|e| value_addr(&e.ev) == Some(addr) || late_addr(&e.ev) == Some(addr));
        match same {
            Some(e) if std::mem::discriminant(&e.ev) == std::mem::discriminant(&ev) => {
                e.ev = ev;
                self.merged += 1;
            }
            _ => self.dropped += 1,
        }
    }

    /// Runs one period of `len` frames before the mixer's: what changed
    /// goes into the mixer's queues on its frame.
    pub fn process(&mut self, ctx: &Ctx, len: usize, mixer: &mut Mixer) {
        // A learned MIDI control's gesture ends HAND_S after its last move:
        // it lets go first this period (a let-go never needs a slot).
        self.hands.tick(len as u32);
        while let Some(addr) = self.hands.take_ended() {
            self.queue(0, ModEvent::Untouch { addr });
        }
        let unrouted = &mut self.unrouted;
        let notes = &mut self.note_values;
        let midi = &mut self.midi_values;
        self.engine.process(
            ctx,
            len,
            &self.events,
            &[None; FOLLOWERS],
            &mut |addr, t| match (addr.owner, t.ev) {
                // A MIDI effect setting: the time engine's, next period.
                (Owner::NoteFx(..), NodeEvent::Param { value, .. }) => {
                    if notes.len() < notes.capacity() {
                        notes.push((addr, value));
                    } else {
                        *unrouted += 1;
                    }
                }
                // Outside gear: MIDI out's, on its frame of this period.
                (Owner::Midi(_), NodeEvent::Param { value, .. }) => {
                    if midi.len() < midi.capacity() {
                        midi.push((t.offset, addr, value));
                    } else {
                        *unrouted += 1;
                    }
                }
                _ => route(mixer, addr, t, unrouted),
            },
        );
        self.events.clear();
    }
}

/// The setting a value event sets (a turn, a hand, a lock, a lane).
fn value_addr(ev: &ModEvent) -> Option<ParamAddr> {
    match *ev {
        ModEvent::Set { addr, .. }
        | ModEvent::Touch { addr, .. }
        | ModEvent::Lock { addr, .. }
        | ModEvent::Lane { addr, .. } => Some(addr),
        _ => None,
    }
}

/// The setting a let-go lets go of.
fn late_addr(ev: &ModEvent) -> Option<ParamAddr> {
    match *ev {
        ModEvent::Untouch { addr } | ModEvent::Unlock { addr } | ModEvent::LetGo { addr, .. } => {
            Some(addr)
        }
        _ => None,
    }
}

/// A MIDI effect setting's description and value now, from `fx`.
fn note_param(fx: &[TrackFx; MAX_TRACKS], addr: ParamAddr) -> Option<(ParamInfo, f32)> {
    let Owner::NoteFx(t, id) = addr.owner else {
        return None;
    };
    let fx = fx.get(usize::from(t))?;
    let slot = fx.slots[fx.place_of(id)?]?;
    let info = mpfree_chain_ui::notes::info(&slot.fx, usize::from(addr.param))?;
    Some((info, info.default))
}

/// A track's MIDI out control (`Owner::Midi`): its range and its value
/// before anything sets it (`mpfree_param::midi`), in whole steps.
pub fn midi_param(addr: ParamAddr) -> Option<(ParamInfo, f32)> {
    use mpfree_fx::mpfree_node::{Curve, Unit};
    use mpfree_play::mpfree_param::midi::{self, Midi};
    let Owner::Midi(t) = addr.owner else {
        return None;
    };
    if usize::from(t) >= MAX_TRACKS {
        return None;
    }
    let (min, max) = midi::range(addr.param)?;
    let default = f32::from(midi::default(addr.param)?);
    let name = match midi::of(addr.param)? {
        Midi::Cc(_) => "Controller",
        Midi::Bend => "Pitch bend",
        Midi::Pressure => "Pressure",
    };
    let info = ParamInfo {
        id: addr.param,
        name,
        group: "MIDI out",
        min: f32::from(min),
        max: f32::from(max),
        default,
        unit: Unit::None,
        curve: Curve::Stepped,
        readonly: false,
    };
    Some((info, default))
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

    /// A track's MIDI effect setting: a step's lock and a macro move it,
    /// and its values go to the time engine, not the mixer. The engine
    /// knows it only once it has the track's effects; it saves and loads
    /// by the effect's id, and forgets an effect that left its track.
    #[test]
    fn a_midi_effect_setting_is_locked_and_moved_by_a_macro() {
        use mpfree_mod::{MacroMap, MapCurve};
        use mpfree_notefx::Fx;
        let mut h = MixHost::new(RATE, 0.0, 6.0);
        let mut fx = Box::new([TrackFx::default(); MAX_TRACKS]);
        fx[2].add(Fx::Chance { percent: 50 });
        let id = fx[2].ids[0];
        let chance = ParamAddr::new(Owner::NoteFx(2, id), 0);
        let values = |h: &mut MixHost| h.modu.take_note_values().collect::<Vec<_>>();
        // Not known yet: a lock is dropped, and nothing is sent.
        h.lock(0, chance, Some(80.0));
        run(&mut h, 4, 0);
        assert!(!h.modu.engine().is_target(chance));
        assert!(values(&mut h).is_empty());
        h.modu.set_note_fx(&fx);
        h.lock(0, chance, Some(80.0));
        run(&mut h, 4, 4);
        assert_eq!(values(&mut h), [(chance, 80.0)]);
        h.lock(0, chance, None);
        run(&mut h, 4, 8);
        assert_eq!(values(&mut h), [(chance, 50.0)], "back to its own");
        // A macro from 20 % to 100 %, turned half way.
        let map = MacroMap {
            target: chance,
            from: 0.2,
            to: 1.0,
            curve: MapCurve::Linear,
        };
        assert!(h.modu.change(&h.mixer, Change::Map { m: 1, map }).is_ok());
        h.modu.set(ParamAddr::macro_value(1), 0.5);
        run(&mut h, 4, 12);
        assert_eq!(values(&mut h).last(), Some(&(chance, 60.0)));
        assert_eq!(h.modu.unrouted(), 0);
        // Saved by the effect's id, and read back.
        let text = h.modu.save();
        assert!(text.contains(&format!("t2.nfx{id}.0")), "{text}");
        let mut fresh = MixHost::new(RATE, 0.0, 6.0);
        assert_eq!(fresh.modu.load(&fresh.mixer, &text), 1, "unknown before");
        fresh.modu.set_note_fx(&fx);
        assert_eq!(fresh.modu.load(&fresh.mixer, &text), 0);
        assert!(fresh.modu.engine().is_target(chance));
        // The effect leaves its track: its settings go.
        h.modu.forget(Owner::NoteFx(2, id));
        assert!(!h.modu.engine().is_target(chance));
    }

    /// LFO to CC: an LFO row on a track's CC lane holds it quietly and
    /// sweeps it round its value, every tick; the lane's rows and the
    /// controls something moves are listed; the row taken off, the CC
    /// goes back to its value.
    #[test]
    fn an_lfo_row_sweeps_a_gear_cc_and_is_listed() {
        use mpfree_mod::Row;
        let mut h = MixHost::new(RATE, 0.0, 6.0);
        let cc = ParamAddr::new(Owner::Midi(1), 74);
        let row = Row {
            source: Source::Lfo(0),
            via: None,
            target: cc,
            depth: 0.25,
        };
        assert!(h.modu.change(&h.mixer, Change::AddRow(row)).is_ok());
        assert_eq!(h.modu.rows_on(cc), [(0, Source::Lfo(0), 0.25)]);
        assert_eq!(h.modu.midi_moved(), [cc]);
        assert!(h.modu.rows_on(ParamAddr::new(Owner::Midi(1), 1)).is_empty());
        let mut got = Vec::new();
        for p in 0..400 {
            run(&mut h, 1, p);
            got.extend(h.modu.take_midi_values().map(|(_, a, v)| (a, v)));
        }
        assert!(got.iter().all(|(a, _)| *a == cc));
        let (lo, hi) = got.iter().fold((f32::MAX, f32::MIN), |(lo, hi), (_, v)| {
            (lo.min(*v), hi.max(*v))
        });
        // Round General MIDI's 64, a quarter of the travel each way.
        assert!(lo < 50.0 && hi > 78.0, "{lo} to {hi}");
        assert!(lo >= 30.0 && hi <= 98.0, "{lo} to {hi}");
        assert!(h.modu.change(&h.mixer, Change::RemoveRow(0)).is_ok());
        assert!(h.modu.midi_moved().is_empty());
        run(&mut h, 4, 400);
        let last = h.modu.take_midi_values().next_back().map(|(_, _, v)| v);
        assert_eq!(last, Some(64.0), "back to its value");
    }

    /// A track's CC out (S6's CC lanes): held quietly, so nothing goes to
    /// the gear until a lock moves it, on the lock's frame; its end goes
    /// back to General MIDI's value; a hand's value is saved and read back
    /// (and sent then); a macro moves it through its range.
    #[test]
    fn a_tracks_cc_out_is_held_quietly_locked_on_its_frame_and_saved() {
        use mpfree_mod::{MacroMap, MapCurve};
        let mut h = MixHost::new(RATE, 0.0, 6.0);
        let cc = ParamAddr::new(Owner::Midi(2), 74);
        let values = |h: &mut MixHost| h.modu.take_midi_values().collect::<Vec<_>>();
        h.lock(37, cc, Some(20.0));
        run(&mut h, 4, 0);
        assert!(h.modu.engine().is_target(cc));
        assert_eq!(values(&mut h), [(37, cc, 20.0)], "no default first");
        h.lock(5, cc, None);
        run(&mut h, 4, 4);
        assert_eq!(values(&mut h), [(5, cc, 64.0)], "General MIDI's 64");
        // A channel mode message isn't a control.
        let mode = ParamAddr::new(Owner::Midi(2), 123);
        h.lock(0, mode, Some(0.0));
        run(&mut h, 4, 8);
        assert!(!h.modu.engine().is_target(mode));
        assert!(values(&mut h).is_empty());
        // A hand (a learned knob): held if it wasn't, and sent.
        let bend = ParamAddr::new(Owner::Midi(0), mpfree_play::mpfree_param::midi::BEND);
        assert!(h.modu.hand(Hand::Screen, bend, -4000.0, false));
        assert!(h.modu.set(cc, 90.0));
        run(&mut h, 4, 12);
        assert_eq!(values(&mut h), [(0, bend, -4000.0), (0, cc, 90.0)]);
        // Saved where it isn't its default, and sent again once read back.
        let text = h.modu.save();
        assert!(text.contains("base.t2.midi.74 = 90"), "{text}");
        assert!(text.contains("base.t0.midi.128 = -4000"), "{text}");
        let mut fresh = MixHost::new(RATE, 0.0, 6.0);
        assert_eq!(fresh.modu.load(&fresh.mixer, &text), 0);
        run(&mut fresh, 4, 0);
        let mut got = values(&mut fresh);
        got.sort_by_key(|v| v.1);
        assert_eq!(got, [(0, bend, -4000.0), (0, cc, 90.0)]);
        // A macro across its range: 0 to 127 in whole steps.
        let map = MacroMap {
            target: cc,
            from: 0.0,
            to: 1.0,
            curve: MapCurve::Linear,
        };
        assert!(h.modu.change(&h.mixer, Change::Map { m: 0, map }).is_ok());
        h.modu.set(ParamAddr::macro_value(0), 0.25);
        run(&mut h, 4, 16);
        assert_eq!(values(&mut h).last(), Some(&(0, cc, 32.0)));
        assert_eq!(h.modu.unrouted(), 0);
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
    fn a_let_go_lands_even_when_the_periods_events_are_full() {
        let mut h = MixHost::new(RATE, 0.0, 6.0);
        let level = |h: &MixHost, t: usize| h.mixer.param(StripId::Track(t), mx::VOLUME);
        let (loop_was, drums_was) = (level(&h, LOOP), level(&h, DRUMS));
        // A step's lock on the Loop track's fader, and a hand on the
        // Drums' while recording.
        h.lock(0, vol(LOOP), Some(-18.0));
        assert!(h.modu.hold(&h.mixer, vol(DRUMS)));
        assert!(h.modu.hand(Hand::Screen, vol(DRUMS), -12.0, true));
        run(&mut h, 4, 0);
        assert!((level(&h, LOOP) + 18.0).abs() < 0.01);
        assert!((level(&h, DRUMS) + 12.0).abs() < 0.01);
        // A busy period fills the events (the morph's crossfader, moved
        // EVENTS times)...
        for i in 0..EVENTS {
            h.modu.morph_to(i as f32 / EVENTS as f32);
        }
        // ...then the lock ends and the hand lets go: neither is lost.
        h.lock(0, vol(LOOP), None);
        h.modu.release(Hand::Screen, vol(DRUMS));
        // One more move of the crossfader takes the last one's place.
        h.modu.morph_to(0.0);
        // A source's value has no setting to merge into: dropped.
        h.modu.queue(0, ModEvent::Trigger);
        assert_eq!(h.modu.waited(), (2, 1, 1));
        run(&mut h, 40, 4);
        assert!(!h.modu.engine().is_locked(vol(LOOP)));
        assert!(!h.modu.engine().is_touched(vol(DRUMS)));
        assert!((level(&h, LOOP) - loop_was).abs() < 0.01, "the lock let go");
        assert!(
            (level(&h, DRUMS) - drums_was).abs() < 0.01,
            "the hand let go"
        );
        assert_eq!(h.modu.engine().late(), 0);
        assert!(h
            .modu
            .describe()
            .contains("2 let-gos kept aside, 1 values merged"));
    }

    #[test]
    fn opening_a_project_lets_go_of_every_hand_and_lock() {
        let mut h = MixHost::new(RATE, 0.0, 6.0);
        let level = |h: &MixHost, t: usize| h.mixer.param(StripId::Track(t), mx::VOLUME);
        let (loop_was, drums_was) = (level(&h, LOOP), level(&h, DRUMS));
        h.lock(0, vol(LOOP), Some(-18.0));
        assert!(h.modu.hold(&h.mixer, vol(DRUMS)));
        assert!(h.modu.hand(Hand::Screen, vol(DRUMS), -12.0, true));
        run(&mut h, 4, 0);
        // A hand queued in the same period goes too.
        assert!(h.modu.hand(Hand::Screen, vol(DRUMS), -20.0, true));
        h.modu.let_go_all();
        run(&mut h, 40, 4);
        assert!(!h.modu.engine().is_locked(vol(LOOP)));
        assert!(!h.modu.engine().is_touched(vol(DRUMS)));
        assert!((level(&h, LOOP) - loop_was).abs() < 0.01);
        assert!((level(&h, DRUMS) - drums_was).abs() < 0.01);
    }

    /// The periods in `s` seconds, rounded up.
    fn periods(s: f32) -> u64 {
        (s * RATE as f32 / PERIOD as f32).ceil() as u64
    }

    #[test]
    fn a_learned_control_turned_while_recording_lets_go_half_a_second_after_its_last_move() {
        let mut h = MixHost::new(RATE, 0.0, 6.0);
        let level = |h: &MixHost| h.mixer.param(StripId::Track(DRUMS), mx::VOLUME);
        let was = level(&h);
        // A step's lock on the Drums' fader; a learned knob turns it while
        // recording, over the lock.
        h.lock(0, vol(DRUMS), Some(-18.0));
        assert!(h.modu.hand(Hand::Midi, vol(DRUMS), -12.0, true));
        run(&mut h, 4, 0);
        assert!((level(&h) + 12.0).abs() < 0.01);
        // Turned again a little later: the hold runs from the last move.
        run(&mut h, periods(0.3), 4);
        assert!(h.modu.hand(Hand::Midi, vol(DRUMS), -10.0, true));
        let mut at = 4 + periods(0.3);
        // The same value again isn't a move.
        run(&mut h, periods(0.3), at);
        at += periods(0.3);
        assert!(h.modu.hand(Hand::Midi, vol(DRUMS), -10.0, true));
        run(&mut h, periods(0.15), at);
        at += periods(0.15);
        assert!(h.modu.engine().is_touched(vol(DRUMS)), "still held");
        assert!((level(&h) + 10.0).abs() < 0.01);
        // Half a second after the last move it lets go, back to the lock.
        run(&mut h, periods(0.1), at);
        at += periods(0.1);
        assert!(!h.modu.engine().is_touched(vol(DRUMS)));
        assert!((level(&h) + 18.0).abs() < 0.01, "{}", level(&h));
        assert_eq!(h.modu.hands().ended, 1);
        // The lock ends: the fader's own value, untouched by the knob.
        h.lock(0, vol(DRUMS), None);
        run(&mut h, 40, at);
        assert!((level(&h) - was).abs() < 0.01);
        assert!(h.modu.describe().contains("1 MIDI gestures ended"));
    }

    #[test]
    fn only_the_hand_that_holds_a_setting_lets_it_go() {
        let mut h = MixHost::new(RATE, 0.0, 6.0);
        let level = |h: &MixHost| h.mixer.param(StripId::Track(DRUMS), mx::VOLUME);
        assert!(h.modu.hold(&h.mixer, vol(DRUMS)));
        let was = level(&h);
        // A finger on the chain editor's knob while recording...
        assert!(h.modu.hand(Hand::Screen, vol(DRUMS), -12.0, true));
        run(&mut h, 4, 0);
        // ...then a learned knob takes it: the newest gesture wins.
        assert!(h.modu.hand(Hand::Midi, vol(DRUMS), -20.0, true));
        run(&mut h, 4, 4);
        assert!((level(&h) + 20.0).abs() < 0.01);
        // The finger moves on: ignored, the knob holds it.
        assert!(h.modu.hand(Hand::Screen, vol(DRUMS), -6.0, true));
        run(&mut h, 4, 8);
        assert!((level(&h) + 20.0).abs() < 0.01, "{}", level(&h));
        // The finger lifts: not its to let go.
        h.modu.release(Hand::Screen, vol(DRUMS));
        run(&mut h, 4, 12);
        assert!(h.modu.engine().is_touched(vol(DRUMS)));
        assert_eq!((h.modu.hands().taken, h.modu.hands().ignored), (1, 1));
        // The knob's gesture ends: let go.
        run(&mut h, periods(0.5), 16);
        assert!(!h.modu.engine().is_touched(vol(DRUMS)));
        assert!((level(&h) - was).abs() < 0.01);
        // A new touch from the screen holds it again, and lets it go.
        assert!(h.modu.hand(Hand::Screen, vol(DRUMS), -3.0, true));
        run(&mut h, 4, 16 + periods(0.5));
        assert!((level(&h) + 3.0).abs() < 0.01);
        h.modu.release(Hand::Screen, vol(DRUMS));
        run(&mut h, 4, 20 + periods(0.5));
        assert!(!h.modu.engine().is_touched(vol(DRUMS)));
        assert_eq!(h.modu.hands().holding(), 0);
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
