//! A pad as files: for other software, and for MPFree to read back whole.
//!
//! [`pad`] gives, for one pad:
//!
//! - **Its sounds**, each a WAV file of the whole sound (so the pad's
//!   region, loop and slices still point at the same frames), named after
//!   the pad: `Break 92.wav`, and `Break 92 layer 2.wav` for a second layer
//!   with a sound of its own. The chopped sound's file also carries its
//!   loop and root note in a `smpl` chunk and its slice points as named
//!   `cue ` points ([`crate::wav::bytes_marked`]), which most samplers and
//!   editors read.
//! - **A sidecar**, `Break 92.mpfree`: the pad as text, in the format a
//!   saved project keeps its kits in ([`crate::save`]), with the slices as
//!   a chop set. [`read_sidecar`] gives it back: loading `Break 92.wav`
//!   with its sidecar beside it brings back the pad as it was, layers,
//!   regions, loops, warp, envelopes, filter and slices.
//! - **Its slices**, when it has more than one: `Break 92 01.wav`,
//!   `Break 92 02.wav` and so on, each the frames of one slice of the
//!   chopped sound as they are in it (not reversed, faded or tuned), to
//!   use anywhere.
//!
//! Pads it stops (chokes) are left out: they are other pads of the kit,
//! not this one. Nothing here touches the disk: the caller writes the
//! files, together in one folder, as the sidecar names its sounds by file
//! name.

use std::collections::HashMap;

use crate::model::*;
use crate::save;
use crate::wav::{self, Marks};

/// The sidecar's extension.
pub const SIDECAR: &str = "mpfree";

/// A pad's files ([`pad`]).
#[derive(Clone, Debug, PartialEq)]
pub struct Export {
    /// The pad's name, made safe for a file name: what the files are
    /// named after, and a name for the folder they go in.
    pub name: String,
    /// Each file's name and its bytes: the sounds, the sidecar, then the
    /// slices.
    pub files: Vec<(String, Vec<u8>)>,
    /// How many of the files are slices.
    pub slices: usize,
}

/// What [`read_sidecar`] gives back: a pad, the sounds it plays and the
/// chop sets on them, ready for [`Project::put_pad_with`].
#[derive(Clone, Debug)]
pub struct Opened {
    pub pad: Pad,
    pub sources: Vec<Source>,
    pub chops: Vec<ChopSet>,
    /// What was left out of it, and why, one line each.
    pub notes: Vec<String>,
}

/// `name` made safe for a file name on any system: no path separators or
/// characters Windows refuses, no control characters, no leading or
/// trailing dots and spaces, at most 80 characters; "Pad" if nothing is
/// left.
pub fn file_safe(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_control() || "/\\:*?\"<>|".contains(c) {
                ' '
            } else {
                c
            }
        })
        .take(80)
        .collect();
    let s = s.trim_matches(|c: char| c == '.' || c.is_whitespace());
    if s.is_empty() {
        "Pad".to_string()
    } else {
        s.to_string()
    }
}

/// Pad `pad` of `project` as files (see the module docs), with `cuts`, the
/// slice starts in its layer `layer`'s sound (inside its region, as the
/// Chop tab has them; none for an unchopped pad).
pub fn pad(project: &Project, pad: usize, layer: usize, cuts: &[u64]) -> Result<Export, String> {
    let p = project.pad(pad).ok_or("the pad has no sound")?;
    let chopped = p.layers.get(layer).ok_or("the pad has no such layer")?;
    let region = chopped.region.clone();
    let main = project
        .source(region.source)
        .ok_or("the pad's sound is missing")?;
    let name = file_safe(&p.name);
    // The sound of each layer once, the chopped one first.
    let mut names: HashMap<SourceId, String> = HashMap::new();
    let mut sounds: Vec<&Source> = Vec::new();
    names.insert(main.id(), format!("{name}.wav"));
    sounds.push(main);
    for (j, l) in p.layers.iter().enumerate() {
        if names.contains_key(&l.region.source) {
            continue;
        }
        let s = project
            .source(l.region.source)
            .ok_or("one of the pad's sounds is missing")?;
        names.insert(s.id(), format!("{name} layer {}.wav", j + 1));
        sounds.push(s);
    }
    let mut cuts: Vec<u64> = cuts
        .iter()
        .copied()
        .filter(|&c| c > region.start && c < region.end)
        .collect();
    cuts.sort_unstable();
    cuts.dedup();
    // The sidecar: a kit of this one pad (on pad 1) and its chop.
    let mut kit = Project::new();
    for s in &sounds {
        kit.add_source((*s).clone());
    }
    if !cuts.is_empty() {
        let c = ChopSet::from_markers(&p.name, main, ChopMode::Manual, cuts.iter().copied())
            .map_err(|e| format!("the slices: {e}"))?;
        kit.add_chop_set(c)
            .map_err(|e| format!("the slices: {e}"))?;
    }
    let mut own = p.clone();
    own.chokes.clear();
    kit.set_pad(0, Some(own), None)
        .map_err(|e| format!("the pad: {e}"))?;
    let file_of = |s: &Source| {
        names
            .get(&s.id())
            .cloned()
            .unwrap_or_else(|| wav::file_name(s))
    };
    let sidecar = save::to_text(&[(&p.name, &kit)], 0, &file_of);
    // The files.
    let mut files = Vec::with_capacity(sounds.len() + 1 + cuts.len() + 1);
    for s in &sounds {
        let marks = if s.id() == main.id() {
            Marks {
                root: p.keys.map(|k| k.root),
                loop_: region
                    .sustain_loop
                    .map(|l| (l.start, l.end, l.mode == LoopMode::PingPong)),
                cues: cuts
                    .iter()
                    .enumerate()
                    .map(|(i, &c)| (c, format!("Slice {}", i + 2)))
                    .collect(),
            }
        } else {
            Marks::default()
        };
        files.push((file_of(s), wav::bytes_marked(s, &marks)));
    }
    files.push((format!("{name}.{SIDECAR}"), sidecar.into_bytes()));
    let mut slices = 0;
    if !cuts.is_empty() {
        let mut bounds = Vec::with_capacity(cuts.len() + 2);
        bounds.push(region.start);
        bounds.extend_from_slice(&cuts);
        bounds.push(region.end);
        let ch = usize::from(main.channels());
        let data = main.data();
        for (i, w) in bounds.windows(2).enumerate() {
            let (a, b) = (w[0] as usize * ch, (w[1] as usize * ch).min(data.len()));
            let slice = Source::new(
                &format!("{} {:02}", p.name, i + 1),
                main.rate(),
                main.channels(),
                data[a..b].to_vec(),
            )
            .map_err(|e| format!("slice {}: {e}", i + 1))?;
            files.push((format!("{name} {:02}.wav", i + 1), wav::bytes(&slice)));
            slices += 1;
        }
    }
    Ok(Export {
        name,
        files,
        slices,
    })
}

/// The pad a sidecar [`pad`] wrote describes. `read` gives a sound from its
/// file name (the sounds beside the sidecar). Fails when the text isn't a
/// sidecar or its pad can't be had; a chop set that can't is left out
/// with a note.
pub fn read_sidecar(
    text: &str,
    read: &mut dyn FnMut(&str) -> Result<Source, String>,
) -> Result<Opened, String> {
    let o = save::from_text(text, read)?;
    let (name, kit) = o.kits.into_iter().next().ok_or("no pad in it")?;
    let Some(pad) = (0..PADS).find_map(|i| kit.pad(i)).cloned() else {
        let why = o
            .notes
            .first()
            .cloned()
            .unwrap_or_else(|| "no pad in it".into());
        return Err(why);
    };
    let _ = name;
    Ok(Opened {
        pad,
        sources: kit.sources().to_vec(),
        chops: kit.chop_sets.clone(),
        notes: o.notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_safe_for_any_disk() {
        assert_eq!(file_safe("Break 92"), "Break 92");
        assert_eq!(file_safe("a/b\\c:d*e?f\"g<h>i|j"), "a b c d e f g h i j");
        assert_eq!(file_safe("  ..Kick.  "), "Kick");
        assert_eq!(file_safe("\u{7}"), "Pad");
        assert_eq!(file_safe(&"x".repeat(200)).len(), 80);
    }
}
