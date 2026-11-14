//! A keygroup program made into an MPFree kit: each keygroup becomes a
//! pad with [`Keys`] (its notes and root) and its samples as velocity
//! layers, the pads then playing notes, as [`Project::put_instrument`]
//! places an auto-sampled instrument.
//!
//! What can't be matched exactly is approximated and said in the notes:
//! a keygroup whose layers have different roots gets one root and each
//! other layer tuned to make up the difference; a layer that doesn't
//! follow the keys on the MPC follows them here; more than
//! [`MAX_LAYERS`] layers keep the first ones.

use std::collections::HashMap;

use mpfree_sampler::keymap::note_in_name;
use mpfree_sampler::model::{
    Keys, Layer, LayerSelect, Loop, LoopMode, Pad, PlayMode, Project, Region, Source, SourceId,
    KEYGROUP_POLYPHONY, MAX_LAYERS, PADS,
};

use crate::xpm::{root_offset, KeygroupProgram, XpmLayer};

/// Choices the program doesn't make.
#[derive(Clone, Debug)]
pub struct Options {
    /// The pads' release after a key is let go. The program's own
    /// envelope values are reported but not read (their scale isn't known).
    pub release_ms: f32,
}

impl Default for Options {
    fn default() -> Options {
        Options { release_ms: 400.0 }
    }
}

/// The kit and what happened on the way.
#[derive(Debug)]
pub struct Imported {
    pub project: Project,
    /// One line each: how the program was read, what was approximated or left out.
    pub notes: Vec<String>,
    /// Pads made and layers placed.
    pub pads: usize,
    pub layers: usize,
}

/// The note's name with middle C (60) as C4.
pub fn note_text(n: u8) -> String {
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    format!("{}{}", NAMES[usize::from(n % 12)], i32::from(n / 12) - 1)
}

/// Makes the kit. `sound` gives a layer's sound (the same [`Source`], id
/// and all, for a sample two layers share) or why it can't.
pub fn keygroup_kit(
    prog: &KeygroupProgram,
    sound: &mut dyn FnMut(&XpmLayer) -> Result<Source, String>,
    opts: &Options,
) -> Result<Imported, String> {
    if prog.keygroups.is_empty() {
        return Err("the program has no keygroup with a sample".into());
    }
    if prog.keygroups.len() > PADS {
        return Err(format!(
            "{} keygroups; a kit holds {PADS}",
            prog.keygroups.len()
        ));
    }
    let mut notes = Vec::new();
    let (offset, why) = root_offset(prog);
    notes.push(why);
    if let Some(d) = prog
        .declared
        .filter(|&d| d as usize != prog.keygroups.len())
    {
        notes.push(format!(
            "the program says {d} keygroups; {} have a sample",
            prog.keygroups.len()
        ));
    }
    // Levels are relative to the most common layer volume, so the usual one is 0 dB.
    let mut counts: HashMap<u32, usize> = HashMap::new();
    for l in prog.keygroups.iter().flat_map(|k| &k.layers) {
        if l.volume > 0.0 {
            *counts.entry(l.volume.to_bits()).or_default() += 1;
        }
    }
    let reference = counts
        .iter()
        .max_by_key(|&(bits, n)| (*n, std::cmp::Reverse(*bits)))
        .map_or(1.0, |(bits, _)| f32::from_bits(*bits));
    let vel_to_level = prog.vel_to_level.unwrap_or(1.0);
    if prog.vel_to_level.is_none() {
        notes.push("no velocity sensitivity in the program: level follows velocity fully".into());
    }

    let mut keygroups: Vec<_> = prog.keygroups.iter().collect();
    keygroups.sort_by_key(|k| (k.lo, k.hi, k.number));
    let mut sources: Vec<Source> = Vec::new();
    let mut have: Vec<SourceId> = Vec::new();
    let mut pads = Vec::new();
    let mut placed = 0;
    let mut unkeyed = 0;
    for kg in keygroups {
        let mut list: Vec<&XpmLayer> = kg.layers.iter().collect();
        list.sort_by_key(|l| (l.vel_lo, l.vel_hi, l.number));
        if list.len() > MAX_LAYERS {
            notes.push(format!(
                "keygroup {}: {} layers; the first {MAX_LAYERS} by velocity kept",
                kg.number,
                list.len()
            ));
            list.truncate(MAX_LAYERS);
        }
        let mut pad_root = None;
        let mut layers = Vec::new();
        for l in list {
            let label = if l.sample.is_empty() {
                &l.file
            } else {
                &l.sample
            };
            if l.volume <= 0.0 {
                notes.push(format!(
                    "keygroup {}: {label} has volume 0; left out",
                    kg.number
                ));
                continue;
            }
            let s = match sound(l) {
                Ok(s) => s,
                Err(e) => {
                    notes.push(format!("keygroup {}: {label}: {e}; left out", kg.number));
                    continue;
                }
            };
            if s.frames() == 0 {
                notes.push(format!(
                    "keygroup {}: {label} holds no audio; left out",
                    kg.number
                ));
                continue;
            }
            let root = if l.root_raw > 0 {
                (l.root_raw - offset).clamp(0, 127) as u8
            } else if let Some(n) = note_in_name(label) {
                notes.push(format!(
                    "keygroup {}: {label} has no RootNote; its name gives {}",
                    kg.number,
                    note_text(n)
                ));
                n
            } else {
                let mid = ((u16::from(kg.lo) + u16::from(kg.hi)) / 2) as u8;
                notes.push(format!(
                    "keygroup {}: {label} has no root note; the middle of the keygroup, {}, is used",
                    kg.number,
                    note_text(mid)
                ));
                mid
            };
            let pad_root = *pad_root.get_or_insert(root);
            if !l.key_track {
                unkeyed += 1;
            }
            let mut region = Region::whole(&s);
            let frames = region.end;
            if l.start < l.end && l.end <= frames {
                (region.start, region.end) = (l.start, l.end);
            } else if l.start > 0 && l.start < frames {
                region.start = l.start;
            }
            region.mode = PlayMode::Gate;
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
            layer.level_db = 20.0 * (l.volume / reference).log10();
            layer.pan = (l.pan - 0.5) * 2.0;
            layer.tune_cents = l.coarse * 100
                + l.fine
                + (l.pitch * 100.0).round() as i32
                + (i32::from(pad_root) - i32::from(root)) * 100;
            if !have.contains(&s.id()) {
                have.push(s.id());
                sources.push(s);
            }
            layers.push(layer);
        }
        let Some(root) = pad_root else {
            notes.push(format!(
                "keygroup {} ({} to {}): no sample could be used; left out",
                kg.number,
                note_text(kg.lo),
                note_text(kg.hi)
            ));
            continue;
        };
        placed += layers.len();
        let name = if prog.name.is_empty() {
            note_text(root)
        } else {
            format!("{} {}", prog.name, note_text(root))
        };
        let mut pad = Pad::new(&name, layers[0].clone());
        pad.layers = layers;
        pad.select = LayerSelect::All;
        pad.polyphony = KEYGROUP_POLYPHONY;
        pad.keys = Some(Keys {
            lo: kg.lo,
            hi: kg.hi,
            root,
        });
        pad.amp.release_ms = opts.release_ms;
        pad.vel_to_level = vel_to_level;
        pads.push(pad);
    }
    if unkeyed > 0 {
        notes.push(format!(
            "{unkeyed} layers don't follow the keys on the MPC (KeyTrack off); here they do"
        ));
    }
    if pads.is_empty() {
        return Err("no keygroup had a sample that could be used".into());
    }
    let lowest = pads
        .iter()
        .filter_map(|p| p.keys.map(|k| k.root))
        .min()
        .unwrap_or(60);
    let from = (lowest - lowest % 12).min(127 - 15);
    let n = pads.len();
    let mut project = Project::new();
    project
        .put_instrument(0, sources, pads, from)
        .map_err(|e| format!("the sampler refused the kit: {e:?}"))?;
    project.clear_history();
    Ok(Imported {
        project,
        notes,
        pads: n,
        layers: placed,
    })
}
