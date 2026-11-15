//! UNDO's history on the mixer: a strip control's turn (a fader, a pan, a
//! send) or a dot dragged on the Mix tab's EQ look, which moves a band's
//! frequency and gain together, so one drag is one entry. The audio loop
//! (main.rs) and the smoke tests' session keep one each and go through
//! these, so both record and undo alike.

use std::time::Instant;

use mpfree_fx::ids::eq;
use mpfree_fx::StripId;
use mpfree_mixer_ui::MixAction;
use mpfree_play::mpfree_param::{Owner, ParamAddr};

use crate::mixing::{self, MixHost};
use crate::undo::{Turn, Turns};

/// No second value.
const NONE: u16 = u16::MAX;

/// What a mixer turn moved.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MixKey {
    /// A strip control (an id from `mpfree_fx::ids::mixer`).
    Strip(StripId, u16),
    /// An insert's parameters moved together: the insert by its slot id
    /// (it keeps it when it moves along the chain), then up to two
    /// parameter ids (an EQ band: frequency and gain).
    Insert {
        strip: StripId,
        uid: u16,
        ids: [u16; 2],
    },
}

pub type MixTurns = Turns<MixKey, [f32; 2]>;
pub type MixTurn = Turn<MixKey, [f32; 2]>;

/// The parameters an EQ parameter is moved with on the EQ look: a band's
/// frequency and gain, a cut's frequency alone.
fn moved_with(id: u16) -> [u16; 2] {
    match eq::band_of(id) {
        Some((b, 1 | 2)) => [eq::freq(b), eq::gain(b)],
        _ => [id, NONE],
    }
}

fn insert_param(mix: &MixHost, strip: StripId, uid: u16, id: u16) -> f32 {
    mix.mixer
        .chain(strip)
        .and_then(|c| c.node(c.slot_of(uid)?))
        .map_or(f32::NAN, |n| n.param(id))
}

/// A turn an action is about to make: its key, the value before, and the
/// value it sets (a NaN part: not moved by it). Read before the desk
/// takes the action.
pub fn before(a: &MixAction, mix: &MixHost) -> Option<(MixKey, [f32; 2], [f32; 2])> {
    match *a {
        MixAction::Set { strip, id, value } => Some((
            MixKey::Strip(strip, id),
            [mix.value(strip, id), 0.0],
            [value, 0.0],
        )),
        MixAction::SetInsert {
            strip,
            slot,
            id,
            value,
        } => {
            let uid = mix.mixer.chain(strip)?.uid(slot)?;
            let ids = moved_with(id);
            let from = ids.map(|i| {
                if i == NONE {
                    0.0
                } else {
                    insert_param(mix, strip, uid, i)
                }
            });
            let to = ids.map(|i| {
                if i == id {
                    value
                } else if i == NONE {
                    0.0
                } else {
                    f32::NAN
                }
            });
            Some((MixKey::Insert { strip, uid, ids }, from, to))
        }
        _ => None,
    }
}

/// Records a turn read by [`before`], once the desk has taken the action.
/// The master's Volume is the Level: `level_db` is where it ended up.
pub fn record(
    turns: &mut MixTurns,
    (key, from, to): (MixKey, [f32; 2], [f32; 2]),
    level_db: f32,
    now: Instant,
) {
    let to = match key {
        MixKey::Strip(StripId::Master, mpfree_fx::ids::mixer::VOLUME) => [level_db, 0.0],
        _ => to,
    };
    turns.turned(key, from, to, now);
}

/// Sets a turn back (UNDO) or again (REDO). Returns the line to say, and
/// the Level when the turn was the master's Volume.
pub fn apply(mix: &mut MixHost, t: &MixTurn, redo: bool) -> (String, Option<f32>) {
    let v = if redo { t.to } else { t.from };
    let line = said(&t.key, redo);
    match t.key {
        MixKey::Strip(strip, id) => {
            let level = mix.action(&MixAction::Set {
                strip,
                id,
                value: v[0],
            });
            (line, level)
        }
        MixKey::Insert { strip, uid, ids } => {
            if let Some(s) = crate::modmix::strip_of(strip) {
                for (k, id) in ids.into_iter().enumerate() {
                    if id != NONE && !v[k].is_nan() {
                        let addr = ParamAddr::new(Owner::Insert(s, uid), id);
                        crate::modpanels::set_param_now(mix, addr, v[k]);
                    }
                }
            }
            (line, None)
        }
    }
}

/// What UNDO (or REDO) of a turn says: "Undid Drums Volume", "Redid Bass
/// EQ Band 2".
pub fn said(key: &MixKey, redo: bool) -> String {
    let done = if redo { "Redid" } else { "Undid" };
    match *key {
        MixKey::Strip(strip, id) => format!("{done} {}", strip_name(strip, id)),
        MixKey::Insert { strip, ids, .. } => format!("{done} {}", insert_name(strip, ids)),
    }
}

/// "Drums Volume".
fn strip_name(strip: StripId, id: u16) -> String {
    let name = mpfree_fx::strip_params()
        .iter()
        .find(|p| p.id == id)
        .map_or("a control", |p| p.name);
    format!("{} {name}", mixing::name(strip))
}

/// "Bass EQ Band 2", "Bass EQ Low cut".
fn insert_name(strip: StripId, ids: [u16; 2]) -> String {
    let what = match ids[0] {
        eq::LOW_CUT => "Low cut".to_string(),
        eq::HIGH_CUT => "High cut".to_string(),
        id => match eq::band_of(id) {
            Some((b, _)) => format!("Band {}", b + 1),
            None => "a control".to_string(),
        },
    };
    format!("{} EQ {what}", mixing::name(strip))
}

/// A turn as the undo history lists it: "Drums Volume to -6.0 dB", "Bass
/// EQ Band 2 to 320 Hz, -3.5 dB".
pub fn row(t: &MixTurn) -> String {
    match t.key {
        MixKey::Strip(strip, id) => match mpfree_fx::strip_params().iter().find(|p| p.id == id) {
            Some(p) => format!(
                "{} {} to {}",
                mixing::name(strip),
                p.name,
                mpfree_synth_ui::format::value_line(p, t.to[0])
            ),
            None => format!("{} control", mixing::name(strip)),
        },
        MixKey::Insert { strip, ids, .. } => {
            let hz = mpfree_mixer_ui::eq_look::hz_text(t.to[0]);
            if ids[1] == NONE {
                format!("{} to {hz}", insert_name(strip, ids))
            } else {
                format!("{} to {hz}, {:+.1} dB", insert_name(strip, ids), t.to[1])
            }
        }
    }
}
