//! Switching an insert between its family's modes (`mpfree_fx::families`)
//! from the chain editor's Mode chooser (`ChainAction::SetMode`).
//!
//! - Two modes of one engine (Reverb's Hall and Cloud, Drive's Soft and
//!   Fuzz): the mode's fixed values are set the way a knob turn is.
//! - Across engines (Room to Plate, Digital to Tape): the new engine is
//!   built off the audio thread like an added effect, set up with the
//!   controls the two share (same name, unit and kind) or, when this insert
//!   was in that engine before, with what it had then, and swapped in
//!   place with the insert's slot id ([`MixHost::replace_insert`]), so the
//!   editor keeps it selected and it stays bypassed if it was. Its macro
//!   maps and matrix rows go to the new engine's matching controls once it
//!   is in; the ones with no match are dropped. If the new engine never
//!   comes (no maker, the chain had no room for its latency), they go back
//!   on the old one.
//!
//! Left behind by a switch across engines: step locks and move lanes on
//! the insert's controls (the engine forgets them, as for a removed
//! insert), the morph scenes' values for them, and a Q-Link learned on one
//! of them, which stays on the old control's id.
//!
//! The values an insert had in each engine are kept for the session, not
//! saved with the project, and dropped when a project is opened (its
//! inserts may reuse slot ids).

use mpfree_fx::families::{self, Mode, Switch};
use mpfree_fx::StripId;
use mpfree_mod::{Change, MacroMap, Row, MACROS};
use mpfree_node::ParamInfo;
use mpfree_play::mpfree_param::{Owner, ParamAddr, Strip};

use crate::mixing::MixHost;
use crate::modmix::strip_id;

/// The values kept per insert and engine, and the switches on their way.
#[derive(Default)]
pub struct Modes {
    kept: Vec<Kept>,
    waiting: Vec<Waiting>,
}

/// What an insert had in an engine it left.
struct Kept {
    strip: StripId,
    uid: u16,
    engine: &'static str,
    values: Vec<(u16, f32)>,
}

/// A switch across engines not in yet, with the macro maps and matrix
/// rows that were on the insert.
struct Waiting {
    strip: Strip,
    uid: u16,
    old: &'static str,
    new: &'static str,
    from: &'static [ParamInfo],
    /// The new mode's fixed values: never a map's or a row's target.
    fixed: &'static [(u16, f32)],
    maps: Vec<(u8, MacroMap)>,
    rows: Vec<Row>,
}

/// Which way a switch went.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Switched {
    /// Already in that mode: nothing to do.
    Already,
    /// The mode's fixed values were set on the same engine.
    Same,
    /// The new engine is on its way; it lands in a few periods.
    Across,
}

impl Modes {
    /// Whether a switch across engines is on its way for this insert.
    fn switching(&self, strip: Strip, uid: u16) -> bool {
        self.waiting
            .iter()
            .any(|w| w.strip == strip && w.uid == uid)
    }

    /// The values `uid` on `strip` had in `engine`, if it left it.
    fn kept(&self, strip: StripId, uid: u16, engine: &str) -> Option<Vec<(u16, f32)>> {
        self.kept
            .iter()
            .find(|k| k.strip == strip && k.uid == uid && k.engine == engine)
            .map(|k| k.values.clone())
    }

    fn keep(&mut self, strip: StripId, uid: u16, engine: &'static str, values: Vec<(u16, f32)>) {
        match self
            .kept
            .iter_mut()
            .find(|k| k.strip == strip && k.uid == uid && k.engine == engine)
        {
            Some(k) => k.values = values,
            None => self.kept.push(Kept {
                strip,
                uid,
                engine,
                values,
            }),
        }
    }
}

/// Puts insert `slot` of `strip` in its family's mode `mode`. `set` is how
/// the editor turns a knob (through the mod engine when it holds the
/// control). Err: the reason, for the hint line.
pub fn switch(
    h: &mut MixHost,
    strip: Strip,
    slot: usize,
    mode: usize,
    mut set: impl FnMut(&mut MixHost, ParamAddr, f32),
) -> Result<Switched, &'static str> {
    let sid = strip_id(strip);
    let c = h.mixer.chain(sid).ok_or("That effect has gone")?;
    let (uid, n) = c
        .uid(slot)
        .zip(c.node(slot))
        .ok_or("That effect has gone")?;
    let (family, now) = families::node_mode(n).ok_or("That effect has no modes")?;
    let to: &'static Mode = family.modes.get(mode).ok_or("That effect has no such mode")?;
    if now == Some(mode) {
        return Ok(Switched::Already);
    }
    if h.modes.switching(strip, uid) {
        return Err("The mixer is busy");
    }
    let owner = Owner::Insert(strip, uid);
    let old = n.info().id;
    if to.id == old {
        for &(p, v) in to.fixed {
            set(h, ParamAddr::new(owner, p), v);
        }
        return Ok(Switched::Same);
    }
    let from_fixed = now.map_or(&[][..], |i| family.modes[i].fixed);
    let mut sw = Switch::new(n, from_fixed, to);
    // A control the engine holds: its own value, not where a macro, a row
    // or a lock has it now.
    let e = h.modu.engine();
    for (p, v) in sw.values.iter_mut() {
        if let Some(b) = e.base(ParamAddr::new(owner, *p)) {
            *v = b;
        }
    }
    sw.remembered = h.modes.kept(sid, uid, to.id);
    let (from, values) = (sw.from, sw.values.clone());
    let maps: Vec<(u8, MacroMap)> = (0..MACROS)
        .flat_map(|m| e.macro_maps(m).map(move |map| (m as u8, map)))
        .filter(|(_, map)| map.target.owner == owner)
        .collect();
    let rows: Vec<Row> = e.rows().filter(|r| r.target.owner == owner).collect();
    h.replace_insert(sid, uid, to.id, sw)?;
    // The old engine's controls mean nothing to the new one: nothing may
    // drive them by id while it comes in.
    h.modu.forget(owner);
    let mixer = &h.mixer;
    h.modes
        .kept
        .retain(|k| mixer.chain(k.strip).is_some_and(|c| c.slot_of(k.uid).is_some()));
    h.modes.keep(sid, uid, old, values);
    h.modes.waiting.push(Waiting {
        strip,
        uid,
        old,
        new: to.id,
        from,
        fixed: to.fixed,
        maps,
        rows,
    });
    Ok(Switched::Across)
}

/// Once a period ([`MixHost::poll_built`]): the macro maps and matrix rows
/// of a switch whose new engine is in go to its matching controls (or
/// back on the old engine when it isn't coming). Nothing happens, and
/// nothing allocates, while no switch is on its way.
pub fn land(h: &mut MixHost) {
    if h.modes.waiting.is_empty() {
        return;
    }
    // Nothing built, waiting for room, or in the mixer's edits: a switch
    // still on the old engine now isn't coming.
    let quiet = h.inserts_building() == 0 && h.mixer.edits_pending() == 0;
    let mut i = 0;
    while i < h.modes.waiting.len() {
        let w = &h.modes.waiting[i];
        let now = h
            .mixer
            .chain(strip_id(w.strip))
            .and_then(|c| c.node(c.slot_of(w.uid)?))
            .map(|n| (n.info().id, n.params()));
        let to = match now {
            // Taken out meanwhile: its maps and rows went with it.
            None => None,
            Some((id, params)) if id == w.new => Some(Some(params)),
            Some((id, _)) if id == w.old && quiet => Some(None),
            Some(_) => {
                i += 1;
                continue;
            }
        };
        let w = h.modes.waiting.swap_remove(i);
        let Some(to) = to else {
            continue;
        };
        let owner = Owner::Insert(w.strip, w.uid);
        let target = |addr: ParamAddr| -> Option<ParamAddr> {
            let Some(to) = to else {
                return Some(addr);
            };
            let p = families::carried_id(w.from, addr.param, to)?;
            (!w.fixed.iter().any(|&(q, _)| q == p)).then(|| ParamAddr::new(owner, p))
        };
        for (m, map) in &w.maps {
            if let Some(t) = target(map.target) {
                let map = MacroMap { target: t, ..*map };
                let _ = h.modu.change(&h.mixer, Change::Map { m: *m, map });
            }
        }
        for row in &w.rows {
            if let Some(t) = target(row.target) {
                let row = Row { target: t, ..*row };
                let _ = h.modu.change(&h.mixer, Change::AddRow(row));
            }
        }
    }
}
