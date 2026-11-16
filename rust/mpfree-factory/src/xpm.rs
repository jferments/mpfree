//! Akai drum programs (.xpm): which sample each pad plays (a port of
//! offline-factory-kit.py's reading).

use crate::kitfile::PAD_NOTES;
use crate::xml;
use std::collections::BTreeMap;

/// A sample as a program names it: (sample name, sample file).
pub type SampleRef = (String, String);

/// A drum program: its name, its instruments' first active layers by
/// instrument number, and its pad-to-note table.
pub struct Program {
    pub name: String,
    pub instruments: BTreeMap<i64, SampleRef>,
    pub pad_notes: BTreeMap<i64, i64>,
}

/// The drum program in an .xpm file, or None if it holds none.
pub fn program(text: &str) -> Option<Program> {
    let root = xml::parse(text).ok()?;
    let prog = root
        .iter("Program")
        .into_iter()
        .find(|p| p.attr("type").unwrap_or("").eq_ignore_ascii_case("drum"))?;
    let name = prog
        .find_text("ProgramName")
        .filter(|t| !t.is_empty())
        .or_else(|| prog.find_text("Name"))
        .unwrap_or("")
        .trim()
        .to_string();
    let mut instruments = BTreeMap::new();
    for inst in prog.iter("Instrument") {
        let Some(number) = inst
            .attr("number")
            .and_then(|n| n.trim().parse::<i64>().ok())
        else {
            continue;
        };
        for layer in inst.iter("Layer") {
            let active = layer
                .find_text("Active")
                .filter(|t| !t.is_empty())
                .unwrap_or("True");
            if active.trim().eq_ignore_ascii_case("false") {
                continue;
            }
            let sample = layer
                .find_text("SampleName")
                .unwrap_or("")
                .trim()
                .to_string();
            let file = layer
                .find_text("SampleFile")
                .unwrap_or("")
                .trim()
                .to_string();
            if !sample.is_empty() || !file.is_empty() {
                instruments.insert(number, (sample, file));
                break;
            }
        }
    }
    let mut pad_notes = BTreeMap::new();
    for pn in prog.iter("PadNote") {
        let (Some(n), Some(note)) = (
            pn.attr("number").and_then(|n| n.trim().parse::<i64>().ok()),
            pn.find_text("Note")
                .and_then(|t| t.trim().parse::<i64>().ok()),
        ) else {
            continue;
        };
        pad_notes.insert(n, note);
    }
    Some(Program {
        name,
        instruments,
        pad_notes,
    })
}

/// What kind of program an .xpm holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Drum,
    Keygroup,
    /// Anything else (a clip, plugin or MIDI program): rarely any samples.
    Other,
}

/// Every sample a program plays: its kind, its name, and each active
/// layer's sample once, in the order the file lists them.
pub struct Samples {
    pub kind: Kind,
    pub name: String,
    pub samples: Vec<SampleRef>,
}

/// The first program in an .xpm file, of any kind, with every sample its
/// active layers name; None if the file holds no program.
pub fn samples(text: &str) -> Option<Samples> {
    let root = xml::parse(text).ok()?;
    let prog = root.iter("Program").into_iter().next()?;
    let ty = prog.attr("type").unwrap_or("");
    let kind = if ty.eq_ignore_ascii_case("drum") {
        Kind::Drum
    } else if ty.eq_ignore_ascii_case("keygroup") {
        Kind::Keygroup
    } else {
        Kind::Other
    };
    let name = prog
        .find_text("ProgramName")
        .filter(|t| !t.is_empty())
        .or_else(|| prog.find_text("Name"))
        .unwrap_or("")
        .trim()
        .to_string();
    let mut samples: Vec<SampleRef> = Vec::new();
    for inst in prog.iter("Instrument") {
        for layer in inst.iter("Layer") {
            let active = layer
                .find_text("Active")
                .filter(|t| !t.is_empty())
                .unwrap_or("True");
            if active.trim().eq_ignore_ascii_case("false") {
                continue;
            }
            let r = (
                layer
                    .find_text("SampleName")
                    .unwrap_or("")
                    .trim()
                    .to_string(),
                layer
                    .find_text("SampleFile")
                    .unwrap_or("")
                    .trim()
                    .to_string(),
            );
            if (!r.0.is_empty() || !r.1.is_empty()) && !samples.contains(&r) {
                samples.push(r);
            }
        }
    }
    Some(Samples {
        kind,
        name,
        samples,
    })
}

/// Two readings of which sample each of pads 1 to 16 plays: by pad number,
/// and by the pad's note.
pub fn mappings(p: &Program) -> ([Option<SampleRef>; 16], [Option<SampleRef>; 16]) {
    let base = match p.instruments.keys().next() {
        Some(0) => 0,
        _ => 1,
    };
    let by_pad = std::array::from_fn(|i| p.instruments.get(&(i as i64 + base)).cloned());
    let by_note = std::array::from_fn(|i| {
        let note = p
            .pad_notes
            .get(&(i as i64 + 1))
            .copied()
            .unwrap_or(PAD_NOTES[i] as i64);
        p.instruments.get(&note).cloned()
    });
    (by_pad, by_note)
}

fn dirname(path: &str) -> &str {
    match path.rfind('/') {
        Some(0) => "/",
        Some(i) => &path[..i],
        None => "",
    }
}

fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// The content path of a program's sample, or None. `files` holds every
/// file's path; `by_name` maps lower-case file names to their paths.
pub fn resolve(
    r: &SampleRef,
    xpm_dir: &str,
    files: &BTreeMap<String, u64>,
    by_name: &BTreeMap<String, Vec<String>>,
) -> Option<String> {
    let (sample, file) = r;
    let dir = xpm_dir.trim_end_matches('/');
    let mut cands: Vec<String> = Vec::new();
    let f = file.replace('\\', "/");
    if !f.is_empty() {
        cands.push(f.clone());
        cands.push(format!("{dir}/{}", f.trim_start_matches(['.', '/'])));
        cands.push(format!("/{}", f.trim_start_matches('/')));
        for prefix in ["/media/acvs-content", "/media/az01-internal"] {
            if let Some(rest) = f.strip_prefix(prefix) {
                cands.push(rest.to_string());
            }
        }
    }
    if !sample.is_empty() {
        for ext in [".wav", ".WAV"] {
            cands.push(format!("{dir}/{sample}{ext}"));
        }
    }
    if let Some(c) = cands.into_iter().find(|c| files.contains_key(c)) {
        return Some(c);
    }
    let mut keys = Vec::new();
    if !f.is_empty() {
        keys.push(basename(&f).to_lowercase());
    }
    if !sample.is_empty() {
        keys.push(format!("{sample}.wav").to_lowercase());
    }
    for k in keys {
        if let Some(hits) = by_name.get(&k) {
            if let Some(h) = hits.iter().find(|h| dirname(h) == xpm_dir) {
                return Some(h.clone());
            }
            let mut sorted = hits.clone();
            sorted.sort();
            if let Some(h) = sorted.into_iter().next() {
                return Some(h);
            }
        }
    }
    None
}

/// A program's name family: the file name before "-Kit" (or "_Kit", " Kit"),
/// or its first word, lower case.
pub fn family(path: &str) -> String {
    let base = basename(path);
    let base = match base.rfind('.') {
        Some(i) if i > 0 => &base[..i],
        _ => base,
    };
    let lower = base.to_ascii_lowercase();
    let cut = ["-kit", "_kit", " kit"]
        .iter()
        .filter_map(|p| lower.find(p))
        .min();
    if let Some(i) = cut {
        if i > 0 {
            return base[..i].to_lowercase();
        }
    }
    base.split(['-', '_', ' '])
        .next()
        .unwrap_or("")
        .to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    const XPM: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<MPCVObject><Version><File_Version>2.1</File_Version></Version>
<Program type="Drum"><ProgramName>Trap Kit 1</ProgramName>
<Instruments>
<Instrument number="1"><Layers><Layer number="1"><Active>True</Active><SampleName>Kick 1</SampleName><SampleFile></SampleFile></Layer></Layers></Instrument>
<Instrument number="2"><Layers><Layer number="1"><Active>False</Active><SampleName>Off</SampleName></Layer><Layer number="2"><SampleName>Snare 1</SampleName><SampleFile>Samples\Snare 1.WAV</SampleFile></Layer></Layers></Instrument>
<Instrument number="37"><Layers><Layer><SampleName>Rim</SampleName></Layer></Layers></Instrument>
</Instruments>
<PadNoteMap><PadNote number="1"><Note>37</Note></PadNote></PadNoteMap>
</Program></MPCVObject>"#;

    #[test]
    fn reads_a_drum_program() {
        let p = program(XPM).unwrap();
        assert_eq!(p.name, "Trap Kit 1");
        assert_eq!(p.instruments.len(), 3);
        assert_eq!(p.instruments[&2].0, "Snare 1");
        let (by_pad, by_note) = mappings(&p);
        assert_eq!(by_pad[0].as_ref().unwrap().0, "Kick 1");
        assert_eq!(by_pad[1].as_ref().unwrap().0, "Snare 1");
        assert_eq!(by_note[0].as_ref().unwrap().0, "Rim");
        assert!(program("<MPCVObject><Program type=\"Keygroup\"/></MPCVObject>").is_none());
    }

    #[test]
    fn lists_every_sample_of_any_program() {
        let s = samples(XPM).unwrap();
        assert_eq!(s.kind, Kind::Drum);
        assert_eq!(s.name, "Trap Kit 1");
        let names: Vec<&str> = s.samples.iter().map(|r| r.0.as_str()).collect();
        assert_eq!(names, ["Kick 1", "Snare 1", "Rim"], "active layers only");
        let keys = r#"<MPCVObject><Program type="Keygroup"><ProgramName>Soft Keys</ProgramName>
<Instruments><Instrument number="1"><Layers><Layer number="1"><SampleName>Keys C3</SampleName></Layer>
<Layer number="2"><SampleName>Keys C3 Soft</SampleName></Layer></Layers></Instrument>
<Instrument number="2"><Layers><Layer number="1"><SampleName>Keys C3</SampleName></Layer></Layers></Instrument>
</Instruments></Program></MPCVObject>"#;
        let s = samples(keys).unwrap();
        assert_eq!(s.kind, Kind::Keygroup);
        let names: Vec<&str> = s.samples.iter().map(|r| r.0.as_str()).collect();
        assert_eq!(names, ["Keys C3", "Keys C3 Soft"], "each once, every layer");
        assert!(samples("<MPCVObject/>").is_none());
    }

    #[test]
    fn resolves_samples_like_the_python_tool() {
        let mut files = BTreeMap::new();
        for f in [
            "/Drums/Trap/Kick 1.wav",
            "/Drums/Trap/Samples/Snare 1.WAV",
            "/Other/rim.wav",
        ] {
            files.insert(f.to_string(), 100u64);
        }
        let mut by_name: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for f in files.keys() {
            by_name
                .entry(basename(f).to_lowercase())
                .or_default()
                .push(f.clone());
        }
        let r = |s: &str, f: &str| resolve(&(s.into(), f.into()), "/Drums/Trap", &files, &by_name);
        assert_eq!(r("Kick 1", "").as_deref(), Some("/Drums/Trap/Kick 1.wav"));
        assert_eq!(
            r("", "Samples\\Snare 1.WAV").as_deref(),
            Some("/Drums/Trap/Samples/Snare 1.WAV")
        );
        assert_eq!(r("Rim", "").as_deref(), Some("/Other/rim.wav"));
        assert_eq!(r("Nope", ""), None);
    }

    #[test]
    fn families() {
        assert_eq!(family("/a/Trap-Kit 2.xpm"), "trap");
        assert_eq!(family("/a/Boom Bap Kit.xpm"), "boom bap");
        assert_eq!(family("/a/House_Drums.xpm"), "house");
        assert_eq!(family("/a/Kit-Thing.xpm"), "kit");
    }
}
