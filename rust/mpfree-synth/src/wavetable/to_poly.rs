//! The Wavetable synth's sounds as Poly sounds. Since wave 2 of the synth
//! library Poly plays the same tables (its Table wave), so a Wavetable
//! sound, factory or a player's, becomes a Poly sound with both
//! oscillators on Table and every setting carried over.
//!
//! What changes on the way:
//!
//! - Ids: most settings have the same id in both (the filter, both
//!   envelopes, the voice, the output); the oscillators, the mix, unison
//!   and four of the LFO's sends move ([`SAME`]).
//! - The envelopes: the Wavetable synth's still have the old meaning of
//!   their times and velocity (Poly's before 19 Nov), so they go through
//!   Poly's own conversion from preset format 1 ([`crate::poly::upgrade`]).
//! - Pressure's targets are listed in another order; each slot keeps its
//!   target by name.
//! - The sub is a sine on the Wavetable synth: Poly's **Sub wave** is set
//!   to Sine.
//! - Poly's voices are 1.9 dB quieter at the same settings (a voice gain of
//!   0.16 against 0.2), so the output level goes up by that much. Above
//!   +4.1 dB the output can't go up all the way (Poly's level stops at
//!   +6 dB): [`Converted::short_db`] says by how much.
//!
//! Everything else Poly has (LFO 2, the routes, Age, the supersaw) stays at
//! its default, so route 1 gives the sound the mod wheel's vibrato, as on
//! every Poly sound.

use mpfree_node::preset::{id_of, key_of, Preset, VERSION};
use mpfree_node::{pressure as np, Node};

use super::params as wp;
use crate::params as pp;
use crate::pressure::{POLY_TARGETS, WT_TARGETS};
use crate::{Poly, Wavetable};

/// Wavetable id and Poly id of each setting the two share in meaning.
pub const SAME: &[(u16, u16)] = &[
    // Osc A, Osc B.
    (wp::TABLE, pp::OSC1_TABLE),
    (wp::POSITION, pp::OSC1_POS),
    (wp::WARP_MODE, pp::OSC1_WARP_MODE),
    (wp::WARP, pp::OSC1_WARP),
    (wp::OCT, pp::OSC1_OCT),
    (wp::SEMI, pp::OSC1_SEMI),
    (wp::FINE, pp::OSC1_FINE),
    (wp::OSC_LEVEL, pp::OSC1_LEVEL),
    (wp::OSC_B + wp::TABLE, pp::OSC2_TABLE),
    (wp::OSC_B + wp::POSITION, pp::OSC2_POS),
    (wp::OSC_B + wp::WARP_MODE, pp::OSC2_WARP_MODE),
    (wp::OSC_B + wp::WARP, pp::OSC2_WARP),
    (wp::OSC_B + wp::OCT, pp::OSC2_OCT),
    (wp::OSC_B + wp::SEMI, pp::OSC2_SEMI),
    (wp::OSC_B + wp::FINE, pp::OSC2_FINE),
    (wp::OSC_B + wp::OSC_LEVEL, pp::OSC2_LEVEL),
    // Mix and unison.
    (wp::SUB_LEVEL, pp::SUB_LEVEL),
    (wp::NOISE_LEVEL, pp::NOISE_LEVEL),
    (wp::UNISON, pp::UNISON),
    (wp::UNISON_DETUNE, pp::UNISON_DETUNE),
    (wp::UNISON_SPREAD, pp::UNISON_SPREAD),
    // The filter, with the mod envelope's amount.
    (wp::FILTER_MODE, pp::FILTER_MODE),
    (wp::CUTOFF, pp::CUTOFF),
    (wp::RESONANCE, pp::RESONANCE),
    (wp::DRIVE, pp::DRIVE),
    (wp::MENV_CUTOFF, pp::FENV_AMOUNT),
    (wp::KEY_TRACK, pp::KEY_TRACK),
    (wp::VEL_CUTOFF, pp::VEL_CUTOFF),
    (wp::DIST, pp::DIST),
    // The mod envelope is Poly's filter envelope.
    (wp::MENV_A, pp::FENV_A),
    (wp::MENV_D, pp::FENV_D),
    (wp::MENV_S, pp::FENV_S),
    (wp::MENV_R, pp::FENV_R),
    (wp::MENV_POS_A, pp::FENV_POS1),
    (wp::MENV_POS_B, pp::FENV_POS2),
    (wp::VEL_POS, pp::VEL_POS),
    (wp::MENV_WARP, pp::FENV_WARP),
    (wp::AMP_A, pp::AMP_A),
    (wp::AMP_D, pp::AMP_D),
    (wp::AMP_S, pp::AMP_S),
    (wp::AMP_R, pp::AMP_R),
    (wp::VEL_AMP, pp::VEL_AMP),
    (wp::LFO_SHAPE, pp::LFO_SHAPE),
    (wp::LFO_RATE, pp::LFO_RATE),
    (wp::LFO_SYNC, pp::LFO_SYNC),
    (wp::LFO_PITCH, pp::LFO_PITCH),
    (wp::LFO_CUTOFF, pp::LFO_CUTOFF),
    (wp::LFO_POS_A, pp::LFO_POS1),
    (wp::LFO_POS_B, pp::LFO_POS2),
    (wp::LFO_RETRIG, pp::LFO_RETRIG),
    (wp::LFO_AMP, pp::LFO_AMP),
    (wp::VOICE_MODE, pp::VOICE_MODE),
    (wp::POLYPHONY, pp::POLYPHONY),
    (wp::GLIDE, pp::GLIDE),
    (wp::BEND_RANGE, pp::BEND_RANGE),
    (wp::PRESS_VIB_RATE, pp::PRESS_VIB_RATE),
    (wp::LEVEL, pp::LEVEL),
    (wp::PAN, pp::PAN),
];

/// How much quieter a Poly voice is than a Wavetable voice at the same
/// settings: 20 log10(0.2 / 0.16).
pub const GAIN_DB: f32 = 1.938_2;

/// Poly's id for a Wavetable id, where the setting carries over: the
/// settings in [`SAME`] and pressure's.
pub fn poly_id(id: u16) -> Option<u16> {
    let press = wp::PRESS_BASE..wp::PRESS_BASE + np::FIELDS as u16;
    if press.contains(&id) {
        return Some(pp::PRESS_BASE + (id - wp::PRESS_BASE));
    }
    SAME.iter().find(|&&(w, _)| w == id).map(|&(_, p)| p)
}

/// Poly's pressure target for a Wavetable one, by name (Off if Poly had
/// none of that name).
pub fn poly_target(t: f32) -> f32 {
    let name = WT_TARGETS
        .get(t.max(0.0) as usize)
        .copied()
        .unwrap_or("Off");
    POLY_TARGETS.iter().position(|&n| n == name).unwrap_or(0) as f32
}

/// A Wavetable setting's value as Poly's: Poly's id for it and the value
/// in Poly's meaning (the envelopes' times and velocity converted, a
/// pressure target by name, the output level up by [`GAIN_DB`], past
/// Poly's top where it is loud). None where Poly has no place for it.
pub fn poly_value(id: u16, v: f32) -> Option<(u16, f32)> {
    let p = poly_id(id)?;
    if (wp::PRESS_BASE..wp::PRESS_BASE + np::FIELDS as u16).contains(&id) {
        let k = id - wp::PRESS_BASE;
        let target = (0..np::SLOTS).any(|n| np::field::target(n) == k);
        return Some((p, if target { poly_target(v) } else { v }));
    }
    // (+ 0.0: a velocity of none converts to -0 dB.)
    let v = crate::poly::upgrade(1, p, v) + 0.0;
    Some((p, if p == pp::LEVEL { v + GAIN_DB } else { v }))
}

/// A Wavetable sound as Poly's settings.
#[derive(Clone, Debug, PartialEq)]
pub struct Converted {
    /// Every one of Poly's settings, by id, in Poly's order.
    pub values: Vec<(u16, f32)>,
    /// How far under the Wavetable sound's level the output stays, in dB,
    /// where Poly's output level could not go up all the way (0 if it
    /// could).
    pub short_db: f32,
}

/// Poly's settings for the Wavetable sound whose every setting `get`
/// reads (a node's, or a preset's with its defaults filled in).
pub fn to_poly(get: impl Fn(u16) -> f32) -> Converted {
    let mut poly = Poly::new(48_000);
    for p in poly.params().iter().filter(|p| !p.readonly) {
        poly.set_param(p.id, p.default);
    }
    poly.set_param(pp::OSC1_WAVE, f32::from(pp::WAVE_TABLE));
    poly.set_param(pp::OSC2_WAVE, f32::from(pp::WAVE_TABLE));
    poly.set_param(pp::SUB_WAVE, 1.0);
    let press = (0..np::FIELDS as u16).map(|k| wp::PRESS_BASE + k);
    for w in SAME.iter().map(|&(w, _)| w).chain(press) {
        if let Some((p, v)) = poly_value(w, get(w)) {
            poly.set_param(p, v);
        }
    }
    let want = get(wp::LEVEL) + GAIN_DB;
    let short_db = (want - poly.param(pp::LEVEL)).max(0.0);
    Converted {
        values: poly
            .params()
            .iter()
            .filter(|p| !p.readonly)
            .map(|p| (p.id, poly.param(p.id)))
            .collect(),
        short_db,
    }
}

/// A Wavetable node's sound as Poly's settings.
pub fn node_to_poly(node: &Wavetable) -> Converted {
    to_poly(|id| node.param(id))
}

/// A saved Wavetable preset (a file, or a track's sound) as a Poly preset:
/// the same name and tags, Poly's settings, and the Q-Link sets' ids moved
/// to Poly's (a knob Poly has no place for is left blank). A sound's own
/// settings (its effects' and macros', whose ids are never an engine's)
/// stay as they were, and each macro's map moves to Poly's setting with
/// its range in Poly's meaning (a map onto a setting Poly has no place for
/// goes). Lines other owners wrote stay as they were. A preset for another
/// engine comes back as it is.
pub fn preset_to_poly(p: &Preset) -> Preset {
    if p.engine != "mpfree.wavetable" {
        return p.clone();
    }
    let mut wt = Wavetable::new(48_000);
    let mut from = p.clone();
    from.upgrade(&wt);
    for &(id, v) in &from.values {
        wt.set_param(id, v);
    }
    let engine = |id: u16| wt.params().iter().any(|q| q.id == id);
    let mut values = node_to_poly(&wt).values;
    values.extend(from.values.iter().filter(|&&(id, _)| !engine(id)));
    Preset {
        engine: "mpfree.poly".to_string(),
        name: p.name.clone(),
        tags: p.tags.clone(),
        values,
        extra: p
            .extra
            .iter()
            .filter_map(|(k, v)| match k.as_str() {
                "qset" => Some((k.clone(), qset_to_poly(v))),
                _ if k.starts_with("macro.") && k.ends_with(".map") => {
                    map_to_poly(v, p.format, &wt).map(|m| (k.clone(), m))
                }
                _ => Some((k.clone(), v.clone())),
            })
            .collect(),
        format: VERSION,
    }
}

/// A macro's map line ("osc-a.position 0 80 exp": the setting's key, the
/// range, the curve) saved in preset format `format` as Poly's; a map onto
/// an effect's setting stays as it is. None where Poly has no place for
/// the setting.
fn map_to_poly(line: &str, format: u32, wt: &Wavetable) -> Option<String> {
    let mut w = line.split_whitespace();
    let key = w.next()?;
    let Some(id) = id_of(wt.params(), key) else {
        return Some(line.to_string());
    };
    let (from, to) = (
        w.next()?.parse::<f32>().ok()?,
        w.next()?.parse::<f32>().ok()?,
    );
    // An older format's range in this build's meaning, then in Poly's.
    let poly = |v: f32| {
        let v = if format < VERSION {
            wt.upgrade(format, id, v)
        } else {
            v
        };
        poly_value(id, v)
    };
    let ((p, from), (_, to)) = (poly(from)?, poly(to)?);
    let mut out = format!("{} {from} {to}", key_of(pp::PARAMS, p)?);
    for rest in w {
        out.push(' ');
        out.push_str(rest);
    }
    Some(out)
}

/// A Q-Link set line ("31 32 34 33 Tone": four ids or `-`, then its name)
/// with Wavetable's ids as Poly's.
fn qset_to_poly(line: &str) -> String {
    let mut parts: Vec<String> = line.split(' ').map(str::to_string).collect();
    for part in parts.iter_mut().take(4) {
        if let Ok(id) = part.parse::<u16>() {
            *part = poly_id(id).map_or_else(|| "-".to_string(), |p| p.to_string());
        }
    }
    parts.join(" ")
}
