//! A keygroup program made into an MPFree kit: each keygroup becomes a
//! pad with [`Keys`] (its notes and root) and its samples as velocity
//! layers, the pads then playing notes, as [`Project::put_instrument`]
//! places an auto-sampled instrument.
//!
//! What can't be matched exactly is approximated and said in the notes:
//! a keygroup whose layers have different roots gets one root and each
//! other layer tuned to make up the difference; a layer that doesn't
//! follow the keys on the MPC follows them here; more than
//! [`MAX_LAYERS`] layers keep the first ones; when a layer is left out
//! (no file, no audio), the layer below it, or failing that above it,
//! takes its velocities, so no hit falls silent. The envelope, the
//! keygroups' filters and the program's effects are reported, not
//! carried over.

use std::collections::HashMap;

use mpfree_sampler::keymap::note_in_name;
use mpfree_sampler::model::{
    Keys, Layer, LayerSelect, Loop, LoopMode, Pad, PlayMode, Project, Region, Source, SourceId,
    KEYGROUP_POLYPHONY, MAX_LAYERS, PADS,
};

use crate::xpm::{knob_text, root_offset, KeygroupProgram, XpmLayer};

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
    // Levels are relative to the most common layer volume (times its
    // keygroup's), so the usual one is 0 dB.
    let mut counts: HashMap<u32, usize> = HashMap::new();
    for k in &prog.keygroups {
        for l in &k.layers {
            let v = l.volume * k.volume;
            if v > 0.0 {
                *counts.entry(v.to_bits()).or_default() += 1;
            }
        }
    }
    let reference = counts
        .iter()
        .max_by_key(|&(bits, n)| (*n, std::cmp::Reverse(*bits)))
        .map_or(1.0, |(bits, _)| f32::from_bits(*bits));
    if prog
        .keygroups
        .iter()
        .all(|k| k.vel_to_level.or(prog.vel_to_level).is_none())
    {
        notes.push("no velocity sensitivity in the program: level follows velocity fully".into());
    }
    notes.extend(reported(prog, opts));

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
        let mut labels = Vec::new();
        let span = (
            list.iter().map(|l| l.vel_lo).min().unwrap_or(0),
            list.iter().map(|l| l.vel_hi).max().unwrap_or(127),
        );
        let mut left_out = false;
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
                    left_out = true;
                    continue;
                }
            };
            if s.frames() == 0 {
                notes.push(format!(
                    "keygroup {}: {label} holds no audio; left out",
                    kg.number
                ));
                left_out = true;
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
            layer.level_db = 20.0 * (l.volume * kg.volume / reference).log10();
            layer.pan = (l.pan - 0.5) * 2.0;
            layer.tune_cents = (kg.coarse + l.coarse) * 100
                + kg.fine
                + l.fine
                + (l.pitch * 100.0).round() as i32
                + (i32::from(pad_root) - i32::from(root)) * 100;
            labels.push((
                label.clone(),
                s.frames() as f64 / f64::from(s.rate().max(1)),
            ));
            if !have.contains(&s.id()) {
                have.push(s.id());
                sources.push(s);
            }
            layers.push(layer);
        }
        if left_out {
            notes.extend(cover(kg.number, &mut layers, &labels, span));
        }
        let longest = labels.iter().map(|l| l.1).fold(0.0, f64::max);
        for (label, secs) in &labels {
            if *secs < longest * 0.5 {
                notes.push(format!(
                    "keygroup {}: {label} is {secs:.1} s and the longest layer {longest:.1} s; it may be cut short",
                    kg.number
                ));
            }
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
        pad.vel_to_level = kg.vel_to_level.or(prog.vel_to_level).unwrap_or(1.0);
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

/// Gives the velocities of left-out layers to the ones kept: each kept
/// layer reaches up to the next one, the softest down to the keygroup's
/// softest and the loudest up to its loudest. `layers` are sorted by
/// velocity; `labels` name them. One note for each layer that grew.
fn cover(kg: u32, layers: &mut [Layer], labels: &[(String, f64)], span: (u8, u8)) -> Vec<String> {
    let mut notes = Vec::new();
    let n = layers.len();
    for i in 0..n {
        let was = (layers[i].vel_lo, layers[i].vel_hi);
        if i == 0 {
            layers[i].vel_lo = layers[i].vel_lo.min(span.0);
        }
        layers[i].vel_hi = if i + 1 < n {
            layers[i + 1].vel_lo.saturating_sub(1).max(layers[i].vel_hi)
        } else {
            layers[i].vel_hi.max(span.1)
        };
        let now = (layers[i].vel_lo, layers[i].vel_hi);
        if now != was {
            notes.push(format!(
                "keygroup {kg}: {} now plays velocities {}-{} (was {}-{}), in place of the layers left out",
                labels[i].0, now.0, now.1, was.0, was.1
            ));
        }
    }
    notes
}

/// What the program sets that the kit doesn't carry over: the envelope
/// and play mode (as 0 to 127 knob positions), the keygroups' filters and
/// the program's effects. Fields the keygroups share are said once.
fn reported(prog: &KeygroupProgram, opts: &Options) -> Vec<String> {
    let mut out = Vec::new();
    let line = |fields: &[(String, String)]| {
        fields
            .iter()
            .map(|(k, v)| format!("{k} {}", knob_text(v)))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut seen: Vec<&Vec<(String, String)>> = Vec::new();
    for k in &prog.keygroups {
        if !k.raw.is_empty() && !seen.contains(&&k.raw) {
            seen.push(&k.raw);
        }
    }
    if let Some(first) = seen.first() {
        out.push(format!(
            "envelope and play mode as written{}: {}; not read (their times aren't known), the release here is {} ms",
            if seen.len() > 1 {
                format!(" (keygroup {}; {} sets differ)", prog.keygroups[0].number, seen.len())
            } else {
                " in every keygroup".to_string()
            },
            line(first),
            opts.release_ms
        ));
    }
    let filters: Vec<_> = prog
        .keygroups
        .iter()
        .filter(|k| !k.filter.is_empty())
        .collect();
    if let Some(k) = filters.first() {
        out.push(format!(
            "{} of {} keygroups have a filter ({}{}); not carried over, so the kit may sound brighter",
            filters.len(),
            prog.keygroups.len(),
            line(&k.filter),
            if filters.iter().any(|o| o.filter != k.filter) {
                format!(" in keygroup {}, others differ", k.number)
            } else {
                String::new()
            }
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
