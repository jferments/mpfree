//! A drum program made into an MPFree kit: each MPC pad with a sample
//! becomes a pad with its layers, level, pan, tuning, mute group, mute
//! targets (chokes) and simultaneous play (links).
//!
//! **Where the pads go.** By default ([`Placement::Notes`]) each sound
//! goes on the pad that plays the MPC pad's note here (pad 1 is note 36,
//! [`mpfree_sampler::node::DEFAULT_BASE_NOTE`]), so a MIDI file exported
//! from the MPC plays the right sounds. [`Placement::Pads`] keeps the
//! MPC's pad order instead (A01 on pad 1), which looks the same on the
//! pads but makes exported MIDI play other sounds. A pad whose note can't
//! be placed (below 36, none in the map, or taken) goes on a free pad
//! and the report says it won't answer its note.
//!
//! What can't be matched exactly is approximated and said in the notes,
//! as for keygroups ([`crate::kit`]): the envelope, filter and effects
//! are reported, not carried over; a pad that plays while held releases
//! over the release given; layers left out leave their velocities to the
//! others; mute groups past 16 get a free number.

use std::collections::HashMap;

use mpfree_sampler::model::{
    Layer, LayerSelect, Loop, LoopMode, Pad, PlayMode, Project, Region, Source, SourceId,
    MAX_LAYERS, PADS,
};
use mpfree_sampler::node::DEFAULT_BASE_NOTE;

use crate::kit::{cover, note_text, Options};
use crate::xpm::{knob_text, DrumPad, DrumProgram, XpmLayer};

/// Where an MPC pad's sound goes in the kit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Placement {
    /// On the pad that plays the MPC pad's note, so exported MIDI plays it.
    #[default]
    Notes,
    /// On the same pad as on the MPC (A01 on pad 1).
    Pads,
}

/// One pad of the kit and where it came from.
#[derive(Clone, Debug, PartialEq)]
pub struct Placed {
    /// The kit's pad, 0 to 127.
    pub index: usize,
    /// The MPC's pad, 1 to 128.
    pub mpc_pad: u32,
    pub note: Option<u8>,
    /// Whether the pad plays the MPC pad's note here.
    pub answers_note: bool,
}

/// The kit and what happened on the way.
#[derive(Debug)]
pub struct DrumImported {
    pub project: Project,
    pub notes: Vec<String>,
    pub placed: Vec<Placed>,
    pub layers: usize,
}

/// The MPC's name for pad `n` (1 to 128): A01 to H16.
pub fn mpc_pad_name(n: u32) -> String {
    let i = n.saturating_sub(1);
    let bank = char::from(b'A' + (i / 16).min(7) as u8);
    format!("{bank}{:02}", i % 16 + 1)
}

/// The kit's pad for note `n` when it plays the note, at the default base.
fn pad_of_note(n: u8) -> Option<usize> {
    n.checked_sub(DEFAULT_BASE_NOTE)
        .map(usize::from)
        .filter(|&i| i < PADS)
}

/// Makes the kit. `sound` gives a layer's sound (the same [`Source`] for
/// a sample two layers share) or why it can't.
pub fn drum_kit(
    prog: &DrumProgram,
    sound: &mut dyn FnMut(&XpmLayer) -> Result<Source, String>,
    opts: &Options,
    placement: Placement,
) -> Result<DrumImported, String> {
    if prog.pads.is_empty() {
        return Err("the program has no pad with a sample".into());
    }
    let mut notes = Vec::new();
    notes.push(format!(
        "<Instrument number> read as counting from {} (pad A01 is number {})",
        prog.numbered_from, prog.numbered_from
    ));
    if !prog.has_note_map {
        notes.push(
            "no PadNoteMap: bank A takes the MPC's default notes, other banks have none".into(),
        );
    }
    notes.extend(reported(prog, opts));

    // Where each pad goes.
    let mut taken = [false; PADS];
    let mut spots: Vec<Option<usize>> = vec![None; prog.pads.len()];
    let want = |p: &DrumPad| match placement {
        Placement::Notes => p.note.and_then(pad_of_note),
        Placement::Pads => Some(p.pad as usize - 1).filter(|&i| i < PADS),
    };
    for (k, p) in prog.pads.iter().enumerate() {
        if let Some(i) = want(p).filter(|&i| !taken[i]) {
            taken[i] = true;
            spots[k] = Some(i);
        }
    }
    for (k, p) in prog.pads.iter().enumerate() {
        if spots[k].is_some() {
            continue;
        }
        let Some(i) = (0..PADS).find(|&i| !taken[i]) else {
            return Err("more pads than the kit holds".into());
        };
        taken[i] = true;
        spots[k] = Some(i);
        let why = match (placement, p.note) {
            (Placement::Pads, _) => "its pad is taken".to_string(),
            (_, None) => "the program gives it no note".to_string(),
            (_, Some(n)) if n < DEFAULT_BASE_NOTE => format!(
                "its note {} is below the kit's lowest, {}",
                note_text(n),
                note_text(DEFAULT_BASE_NOTE)
            ),
            (_, Some(n)) => format!("another pad already plays its note {}", note_text(n)),
        };
        notes.push(format!(
            "MPC pad {} goes on pad {}: {why}; MIDI from the MPC won't play it",
            mpc_pad_name(p.pad),
            i + 1
        ));
    }
    // A note's pad in the kit, for mute targets and simultaneous play.
    let mut by_note: HashMap<u8, usize> = HashMap::new();
    for (k, p) in prog.pads.iter().enumerate() {
        if let (Some(n), Some(i)) = (p.note, spots[k]) {
            by_note.entry(n).or_insert(i);
        }
    }

    // Mute groups: the MPC's number when it fits 1 to 16, else a free one.
    let mut groups: HashMap<u32, u8> = HashMap::new();
    let mut used: Vec<u8> = prog
        .pads
        .iter()
        .filter_map(|p| {
            u8::try_from(p.mute_group)
                .ok()
                .filter(|g| (1..=16).contains(g))
        })
        .collect();
    used.sort_unstable();
    used.dedup();
    let mut dropped_groups = Vec::new();
    for p in &prog.pads {
        let g = p.mute_group;
        if g == 0 || groups.contains_key(&g) {
            continue;
        }
        if let Some(n) = u8::try_from(g).ok().filter(|g| (1..=16).contains(g)) {
            groups.insert(g, n);
        } else if let Some(n) = (1..=16u8).find(|n| !used.contains(n)) {
            used.push(n);
            groups.insert(g, n);
            notes.push(format!("mute group {g} is mute group {n} here"));
        } else {
            dropped_groups.push(g);
        }
    }
    if !dropped_groups.is_empty() {
        notes.push(format!(
            "mute groups {dropped_groups:?} left out: a kit has 16"
        ));
    }

    // Levels are relative to the most common pad volume and the most
    // common layer volume, so the usual pad is at 0 dB.
    let common = |vals: Vec<f32>| {
        let mut counts: HashMap<u32, usize> = HashMap::new();
        for v in vals.into_iter().filter(|v| *v > 0.0) {
            *counts.entry(v.to_bits()).or_default() += 1;
        }
        counts
            .into_iter()
            .max_by_key(|&(bits, n)| (n, std::cmp::Reverse(bits)))
            .map_or(1.0, |(bits, _)| f32::from_bits(bits))
    };
    let pad_ref = common(prog.pads.iter().map(|p| p.volume).collect());
    let layer_ref = common(
        prog.pads
            .iter()
            .flat_map(|p| p.layers.iter().map(|l| l.volume))
            .collect(),
    );
    let db = |v: f32, r: f32| 20.0 * (v / r).log10();

    let mut project = Project::new();
    let mut have: Vec<SourceId> = Vec::new();
    let mut placed = Vec::new();
    let mut layer_count = 0;
    let mut selects: HashMap<i32, usize> = HashMap::new();
    for (k, p) in prog.pads.iter().enumerate() {
        let name = mpc_pad_name(p.pad);
        let mut list: Vec<&XpmLayer> = p.layers.iter().collect();
        list.sort_by_key(|l| (l.vel_lo, l.vel_hi, l.number));
        if list.len() > MAX_LAYERS {
            notes.push(format!(
                "pad {name}: {} layers; the first {MAX_LAYERS} by velocity kept",
                list.len()
            ));
            list.truncate(MAX_LAYERS);
        }
        let span = (
            list.iter().map(|l| l.vel_lo).min().unwrap_or(0),
            list.iter().map(|l| l.vel_hi).max().unwrap_or(127),
        );
        let gate = p.one_shot == Some(false);
        let mut layers = Vec::new();
        let mut labels = Vec::new();
        let mut sources = Vec::new();
        let mut left_out = false;
        for l in list {
            let label = if l.sample.is_empty() {
                l.file.clone()
            } else {
                l.sample.clone()
            };
            if l.volume <= 0.0 {
                notes.push(format!("pad {name}: {label} has volume 0; left out"));
                continue;
            }
            let s = match sound(l) {
                Ok(s) if s.frames() > 0 => s,
                Ok(_) => {
                    notes.push(format!("pad {name}: {label} holds no audio; left out"));
                    left_out = true;
                    continue;
                }
                Err(e) => {
                    notes.push(format!("pad {name}: {label}: {e}; left out"));
                    left_out = true;
                    continue;
                }
            };
            let mut region = Region::whole(&s);
            let frames = region.end;
            if l.start < l.end && l.end <= frames {
                (region.start, region.end) = (l.start, l.end);
            } else if l.start > 0 && l.start < frames {
                region.start = l.start;
            }
            region.reverse = l.reverse;
            region.mode = if gate {
                PlayMode::Gate
            } else {
                PlayMode::OneShot
            };
            if l.looped
                && region.start <= l.loop_start
                && l.loop_start < l.loop_end
                && l.loop_end <= region.end
            {
                region.sustain_loop = Some(Loop {
                    start: l.loop_start,
                    end: l.loop_end,
                    crossfade: 0,
                    mode: LoopMode::Forward,
                });
            }
            let mut layer = Layer::new(region);
            layer.vel_lo = l.vel_lo;
            layer.vel_hi = l.vel_hi;
            layer.level_db = db(l.volume, layer_ref);
            layer.pan = (l.pan - 0.5) * 2.0;
            layer.tune_cents = l.coarse * 100 + l.fine + (l.pitch * 100.0).round() as i32;
            labels.push((label, s.frames() as f64 / f64::from(s.rate().max(1))));
            if !have.contains(&s.id()) {
                have.push(s.id());
                sources.push(s);
            }
            layers.push(layer);
        }
        if left_out {
            notes.extend(cover(&format!("pad {name}"), &mut layers, &labels, span));
        }
        let Some(first) = layers.first().cloned() else {
            notes.push(format!("pad {name}: no sample could be used; left out"));
            continue;
        };
        let index = spots[k].unwrap_or(0);
        let mut pad = Pad::new(&labels[0].0, first);
        layer_count += layers.len();
        pad.layers = layers;
        pad.select = match p.zone_play {
            Some(0) => LayerSelect::Cycle,
            Some(2) => LayerSelect::Random,
            _ => LayerSelect::All,
        };
        if let Some(z) = p.zone_play {
            *selects.entry(z).or_default() += 1;
        }
        pad.polyphony = if p.mono {
            1
        } else {
            p.polyphony
                .map_or(pad.polyphony, |n| n.clamp(1, u32::from(u8::MAX)) as u8)
        };
        pad.mute_group = groups.get(&p.mute_group).copied();
        pad.chokes = p
            .mute_targets
            .iter()
            .filter_map(|n| by_note.get(n).copied())
            .filter(|&i| i != index)
            .collect();
        pad.links = p
            .simult_targets
            .iter()
            .filter_map(|n| by_note.get(n).copied())
            .filter(|&i| i != index)
            .collect();
        let lost =
            p.mute_targets.len() + p.simult_targets.len() - pad.chokes.len() - pad.links.len();
        if lost > 0 {
            notes.push(format!(
                "pad {name}: {lost} mute or simultaneous-play targets name notes no pad here plays; left out"
            ));
        }
        pad.level_db = db(p.volume, pad_ref);
        pad.pan = (p.pan - 0.5) * 2.0;
        pad.tune_cents = p.coarse * 100 + p.fine;
        pad.vel_to_level = p.vel_to_level.unwrap_or(1.0);
        if gate {
            pad.amp.release_ms = opts.release_ms;
        }
        project
            .put_pad_with(index, sources, pad, Vec::new())
            .map_err(|e| format!("the sampler refused pad {name}: {e:?}"))?;
        placed.push(Placed {
            index,
            mpc_pad: p.pad,
            note: p.note,
            answers_note: p.note.and_then(pad_of_note) == Some(index),
        });
    }
    if placed.is_empty() {
        return Err("no pad had a sample that could be used".into());
    }
    if !selects.is_empty() {
        let mut s: Vec<_> = selects.into_iter().collect();
        s.sort_unstable();
        let list: Vec<String> = s
            .iter()
            .map(|(z, n)| format!("{z} on {n} pad{}", if *n == 1 { "" } else { "s" }))
            .collect();
        notes.push(format!(
            "layer choice (ZonePlay) {}; read as 0 cycle, 2 random, others by velocity (inferred, not checked on the MPC)",
            list.join(", ")
        ));
    }
    project.clear_history();
    Ok(DrumImported {
        project,
        notes,
        placed,
        layers: layer_count,
    })
}

/// What the program sets that the kit doesn't carry over: the envelope
/// (as 0 to 127 knob positions), filters and the program's effects.
fn reported(prog: &DrumProgram, opts: &Options) -> Vec<String> {
    let mut out = Vec::new();
    let line = |fields: &[(String, String)]| {
        fields
            .iter()
            .map(|(k, v)| format!("{k} {}", knob_text(v)))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut seen: Vec<&Vec<(String, String)>> = Vec::new();
    for p in &prog.pads {
        if !p.raw.is_empty() && !seen.contains(&&p.raw) {
            seen.push(&p.raw);
        }
    }
    if let Some(first) = seen.first() {
        out.push(format!(
            "envelope as written{}: {}; not read (its times aren't known); pads that play while held release over {} ms",
            if seen.len() > 1 {
                format!(" (pad {}; {} sets differ)", mpc_pad_name(prog.pads[0].pad), seen.len())
            } else {
                " on every pad".to_string()
            },
            line(first),
            opts.release_ms
        ));
    }
    let filters: Vec<_> = prog.pads.iter().filter(|p| !p.filter.is_empty()).collect();
    if let Some(p) = filters.first() {
        out.push(format!(
            "{} of {} pads have a filter ({} on pad {}); not carried over, so the kit may sound brighter",
            filters.len(),
            prog.pads.len(),
            line(&p.filter),
            mpc_pad_name(p.pad)
        ));
    }
    if !prog.inserts.is_empty() {
        out.push(format!(
            "the program's effects ({}) are not carried over",
            prog.inserts.join(", ")
        ));
    }
    out
}
