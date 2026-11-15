//! MPC program files (`.xpm`): XML holding one `<Program type="...">`.
//! This reads keygroup programs: each `<Instrument>` is a keygroup with
//! its note range (`LowNote`, `HighNote`) and up to a few `<Layer>`s, each
//! a sample with its velocities (`VelStart`, `VelEnd`), root note
//! (`RootNote`), tuning, level and pan.
//!
//! The layout comes from community notes and program files, not from
//! Akai, so what a field means is inferred: the raw values are kept and
//! [`root_offset`] checks the root notes against the sample names.
//!
//! Checked on a factory keygroup program (MPC 2.9, File_Version 2.1):
//! `LowNote`/`HighNote` and `VelStart`/`VelEnd` are as read here;
//! `RootNote` is the MIDI note + 1; `Volume` is a linear gain (0.707946 is
//! -3 dB) and `Pan` 0.5 is the centre, on the program, each keygroup and
//! each layer; `VelocitySensitivity`, the tuning and the envelope sit on
//! each keygroup (`<Instrument>`), the envelope as 0 to 1 for the MPC's 0
//! to 127 knob positions (`VolumeRelease` 0.377953 is 48), so its times
//! are still unknown.

use crate::xml::{self, Element};

/// One sample of a keygroup, as the file has it.
#[derive(Clone, Debug, PartialEq)]
pub struct XpmLayer {
    pub number: u32,
    /// The sample's name (the file name without `.wav`, as the MPC keeps it).
    pub sample: String,
    /// A path to the file, when the program gives one (often empty).
    pub file: String,
    pub vel_lo: u8,
    pub vel_hi: u8,
    /// `RootNote` as written: 0 when the program leaves it to the sample.
    pub root_raw: i32,
    pub coarse: i32,
    pub fine: i32,
    /// `Pitch`, read as semitones.
    pub pitch: f32,
    /// `Volume`, a linear gain.
    pub volume: f32,
    /// `Pan`, 0 left, 0.5 centre, 1 right.
    pub pan: f32,
    pub start: u64,
    pub end: u64,
    pub looped: bool,
    pub loop_start: u64,
    pub loop_end: u64,
    pub key_track: bool,
}

/// One keygroup: its notes and its sampled layers.
#[derive(Clone, Debug, PartialEq)]
pub struct Keygroup {
    pub number: u32,
    pub lo: u8,
    pub hi: u8,
    pub layers: Vec<XpmLayer>,
    /// Envelope and play-mode fields as written (`VolumeRelease` and the like).
    pub raw: Vec<(String, String)>,
    /// The keygroup's own `Volume`, a linear gain on its layers.
    pub volume: f32,
    /// The keygroup's own `TuneCoarse` (semitones) and `TuneFine` (cents).
    pub coarse: i32,
    pub fine: i32,
    /// The keygroup's `VelocitySensitivity`, when written: 0 to 1.
    pub vel_to_level: Option<f32>,
    /// The keygroup's filter as written when `FilterType` isn't 0 (off):
    /// the type, then `Cutoff`, `Resonance` and `VelocityToFilter`.
    pub filter: Vec<(String, String)>,
}

/// A keygroup program: its name and the keygroups that have a sample.
#[derive(Clone, Debug, PartialEq)]
pub struct KeygroupProgram {
    pub name: String,
    /// `KeygroupNumKeygroups`, when written.
    pub declared: Option<u32>,
    pub keygroups: Vec<Keygroup>,
    /// `VelocitySensitivity` or the like on the program, when written: 0
    /// to 1. Programs from the MPC write it on each keygroup instead
    /// ([`Keygroup::vel_to_level`]).
    pub vel_to_level: Option<f32>,
    /// The program's switched-on effects (`InsertN` with `InsertEnableN`), by name.
    pub inserts: Vec<String>,
}

/// The type a program file says it is (`Keygroup`, `Drum`, ...), if it is one.
pub fn program_type(bytes: &[u8]) -> Result<String, String> {
    let root = xml::parse(bytes)?;
    program(&root)
        .and_then(|p| p.attr("type"))
        .map(str::to_string)
        .ok_or_else(|| "no <Program type=...> in the file".to_string())
}

fn program(root: &Element) -> Option<&Element> {
    root.descendants().into_iter().find(|e| e.name == "Program")
}

fn text<'a>(e: &'a Element, tag: &str) -> &'a str {
    e.child_text(tag).unwrap_or("")
}

fn num(e: &Element, tag: &str, default: f64) -> f64 {
    text(e, tag)
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite())
        .unwrap_or(default)
}

fn flag(e: &Element, tag: &str, default: bool) -> bool {
    match text(e, tag).to_ascii_lowercase().as_str() {
        "" => default,
        v => v == "true" || v == "1",
    }
}

fn note(v: f64) -> u8 {
    v.round().clamp(0.0, 127.0) as u8
}

fn frames(v: f64) -> u64 {
    if v > 0.0 {
        v.round().min(u64::MAX as f64) as u64
    } else {
        0
    }
}

/// Reads a keygroup program. A program of another type is an error that
/// names its type.
pub fn read_keygroup(bytes: &[u8]) -> Result<KeygroupProgram, String> {
    let root = xml::parse(bytes)?;
    let prog = program(&root).ok_or("no <Program> in the file")?;
    let kind = prog.attr("type").unwrap_or("");
    if !kind.eq_ignore_ascii_case("keygroup") {
        return Err(format!("a {kind:?} program, not a keygroup program"));
    }
    let name = [text(prog, "ProgramName"), text(prog, "Name")]
        .into_iter()
        .find(|s| !s.is_empty())
        .unwrap_or("")
        .to_string();
    let declared = text(prog, "KeygroupNumKeygroups").parse().ok();
    let vel_to_level = sensitivity(prog);
    let inserts = (1..=8)
        .filter(|n| flag(prog, &format!("InsertEnable{n}"), false))
        .filter_map(|n| prog.child(&format!("Insert{n}")))
        .map(|i| text(i, "Name").trim().to_string())
        .filter(|n| !n.is_empty())
        .collect();
    let mut keygroups = Vec::new();
    for inst in prog
        .descendants()
        .into_iter()
        .filter(|e| e.name == "Instrument")
    {
        let Some(number) = inst.attr("number").and_then(|n| n.parse().ok()) else {
            continue;
        };
        let mut layers = Vec::new();
        for l in inst.descendants().into_iter().filter(|e| e.name == "Layer") {
            if !flag(l, "Active", true) {
                continue;
            }
            let (sample, file) = (text(l, "SampleName"), text(l, "SampleFile"));
            if sample.is_empty() && file.is_empty() {
                continue;
            }
            let (a, b) = (note(num(l, "VelStart", 0.0)), note(num(l, "VelEnd", 127.0)));
            layers.push(XpmLayer {
                number: l.attr("number").and_then(|n| n.parse().ok()).unwrap_or(0),
                sample: sample.to_string(),
                file: file.to_string(),
                vel_lo: a.min(b),
                vel_hi: a.max(b),
                root_raw: num(l, "RootNote", 0.0).round().clamp(0.0, 128.0) as i32,
                coarse: num(l, "TuneCoarse", 0.0).round().clamp(-48.0, 48.0) as i32,
                fine: num(l, "TuneFine", 0.0).round().clamp(-100.0, 100.0) as i32,
                pitch: num(l, "Pitch", 0.0).clamp(-48.0, 48.0) as f32,
                volume: num(l, "Volume", 1.0).clamp(0.0, 16.0) as f32,
                pan: num(l, "Pan", 0.5).clamp(0.0, 1.0) as f32,
                start: frames(num(l, "SampleStart", 0.0)),
                end: frames(num(l, "SampleEnd", 0.0)),
                looped: flag(l, "Loop", false),
                loop_start: frames(num(l, "LoopStart", 0.0)),
                loop_end: frames(num(l, "LoopEnd", 0.0)),
                key_track: flag(l, "KeyTrack", true),
            });
        }
        if layers.is_empty() {
            continue;
        }
        let (a, b) = (
            note(num(inst, "LowNote", 0.0)),
            note(num(inst, "HighNote", 127.0)),
        );
        let raw = inst
            .children
            .iter()
            .filter(|c| {
                c.children.is_empty()
                    && (c.name.starts_with("Volume") && c.name != "Volume"
                        || matches!(c.name.as_str(), "ZonePlay" | "TriggerMode" | "Polyphony"))
            })
            .map(|c| (c.name.clone(), c.text.clone()))
            .collect();
        let filter = match num(inst, "FilterType", 0.0).round() as i32 {
            0 => Vec::new(),
            t => std::iter::once(("FilterType".to_string(), t.to_string()))
                .chain(
                    ["Cutoff", "Resonance", "VelocityToFilter"]
                        .into_iter()
                        .filter_map(|f| inst.child_text(f).map(|v| (f.to_string(), v.to_string()))),
                )
                .collect(),
        };
        keygroups.push(Keygroup {
            number,
            lo: a.min(b),
            hi: a.max(b),
            layers,
            raw,
            volume: num(inst, "Volume", 1.0).clamp(0.0, 16.0) as f32,
            coarse: num(inst, "TuneCoarse", 0.0).round().clamp(-48.0, 48.0) as i32,
            fine: num(inst, "TuneFine", 0.0).round().clamp(-100.0, 100.0) as i32,
            vel_to_level: sensitivity(inst),
            filter,
        });
    }
    Ok(KeygroupProgram {
        name,
        declared,
        keygroups,
        vel_to_level,
        inserts,
    })
}

/// `VelocitySensitivity` (or `VelocityToAmpAmount`) on `e` itself, 0 to 1.
fn sensitivity(e: &Element) -> Option<f32> {
    ["VelocitySensitivity", "VelocityToAmpAmount"]
        .into_iter()
        .find_map(|t| e.child_text(t).and_then(|v| v.parse::<f32>().ok()))
        .filter(|v| v.is_finite())
        .map(|v| v.clamp(0.0, 1.0))
}

/// A field's value as the MPC shows it: a fraction from 0 to 1 written
/// with a decimal point is a 0 to 127 knob position, so 0.377953 reads
/// "48 of 127"; anything else as written.
pub fn knob_text(v: &str) -> String {
    match v.trim().parse::<f64>() {
        Ok(x) if v.contains('.') && (0.0..=1.0).contains(&x) => {
            format!("{} of 127", (x * 127.0).round() as i32)
        }
        _ => v.trim().to_string(),
    }
}

/// How `RootNote` relates to the MIDI note, and why: the MIDI note is
/// `RootNote - offset`. It compares `RootNote` with the note in each
/// sample's name by pitch class only (libraries number octaves from C3 or
/// from C4 for middle C) and takes 0, +1 or -1, whichever most layers
/// agree on; 0 when no name has a note.
pub fn root_offset(p: &KeygroupProgram) -> (i32, String) {
    let mut votes = [0usize; 12];
    let mut named = 0;
    for l in p.keygroups.iter().flat_map(|k| &k.layers) {
        let n = mpfree_sampler::keymap::note_in_name(if l.sample.is_empty() {
            &l.file
        } else {
            &l.sample
        });
        if let (Some(n), true) = (n, l.root_raw > 0) {
            votes[(l.root_raw - i32::from(n)).rem_euclid(12) as usize] += 1;
            named += 1;
        }
    }
    let best = [(0usize, 0i32), (1, 1), (11, -1)]
        .into_iter()
        .filter(|&(d, _)| votes[d] > 0)
        .max_by_key(|&(d, off)| (votes[d], off == 0, off > 0));
    match best {
        None => (
            0,
            format!("RootNote taken as the MIDI note (no sample name with a note to check it against; {named} named)"),
        ),
        Some((d, off)) => {
            let what = match off {
                0 => "the MIDI note".to_string(),
                o => format!("the MIDI note {} 1", if o > 0 { '+' } else { '-' }),
            };
            (
                off,
                format!("RootNote reads as {what}: {} of {named} layers whose sample name has a note agree", votes[d]),
            )
        }
    }
}
