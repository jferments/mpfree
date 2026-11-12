//! A sound preset as plain text: the engine, a name, tags and every
//! setting, one `key = value` line each, so a person can read and edit it
//! and a later MPFree still loads it.
//!
//! ```text
//! mpfree-preset = 1
//! engine = mpfree.poly
//! name = Dust Pad
//! tags = pad, warm
//!
//! # Osc 1
//! osc-1.wave = Saw
//! osc-1.octave = 0
//! # Filter
//! filter.cutoff = 1840
//! ...
//! qset = 31 32 34 33 Tone
//! ```
//!
//! - **Settings by name.** A setting's key is its group and name
//!   (`filter.cutoff`), so a preset reads like the Synth tab. Where two
//!   settings of one node would share a key (a plugin's, say), both use
//!   their ids instead (`id.31`). Choices are written by name (`Saw`) and
//!   switches as `on` or `off`; a number works for either.
//! - **Every value exactly.** Numbers are written as the shortest text
//!   that reads back to the same `f32`, so a preset loads to the bit.
//! - **Old and new presets load.** A setting the file doesn't name (one
//!   added after it was saved) takes its default; a value out of range is
//!   clamped; a key this build doesn't know is kept as it is in
//!   [`Preset::extra`] and written back on the next save, as are other
//!   owners' lines such as the Q-Link sets (`qset`). A file from a newer
//!   format, or for another engine, is refused with a reason.
//! - Blank lines and lines starting with `#` are skipped.

use std::fmt;

use mpfree_node::{Curve, Node, NodeInfo, ParamInfo};

/// The first line's key.
pub const FORMAT: &str = "mpfree-preset";
/// The format this build writes; it reads this and older ones.
pub const VERSION: u32 = 1;
/// A preset file's extension.
pub const EXT: &str = "preset";
/// The largest preset text read.
pub const MAX_TEXT: usize = 256 * 1024;
/// The longest name, in characters.
pub const MAX_NAME: usize = 40;
/// The most tags a preset carries.
pub const MAX_TAGS: usize = 8;
/// The longest tag, in characters.
pub const MAX_TAG: usize = 20;

/// A sound: which engine, its name and tags, and its settings.
#[derive(Clone, Debug, PartialEq)]
pub struct Preset {
    /// The node's id, such as `mpfree.poly`.
    pub engine: String,
    pub name: String,
    pub tags: Vec<String>,
    /// Settings by parameter id, in the node's order. Meters are left out.
    pub values: Vec<(u16, f32)>,
    /// Lines this format doesn't read itself, in file order: other owners'
    /// (`qset`) and settings this build doesn't have.
    pub extra: Vec<(String, String)>,
}

/// Why a text is not a preset this build can load.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// Its first line is not `mpfree-preset = <version>`.
    NotAPreset,
    /// Saved by a newer MPFree, in a format this build can't read.
    Newer(u32),
    /// It names no engine.
    NoEngine,
    /// It is for another engine.
    WrongEngine { file: String, node: String },
    /// Over [`MAX_TEXT`] bytes.
    TooBig,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NotAPreset => write!(f, "not an MPFree preset"),
            Error::Newer(v) => write!(
                f,
                "saved by a newer MPFree (preset format {v}; this one reads up to {VERSION})"
            ),
            Error::NoEngine => write!(f, "the preset names no engine"),
            Error::WrongEngine { file, node } => {
                write!(f, "a preset for {file}, not for {node}")
            }
            Error::TooBig => write!(f, "too big for a preset ({MAX_TEXT} bytes at most)"),
        }
    }
}

impl std::error::Error for Error {}

impl Preset {
    /// The node's sound now, with a name and tags.
    pub fn capture(node: &dyn Node, name: &str, tags: &[&str]) -> Preset {
        Preset {
            engine: node.info().id.to_string(),
            name: clean_name(name),
            tags: clean_tags(tags.iter().copied()),
            values: node
                .params()
                .iter()
                .filter(|p| !p.readonly)
                .map(|p| (p.id, node.param(p.id)))
                .collect(),
            extra: Vec::new(),
        }
    }

    /// Sets every setting of `node` from the preset: its value, or its
    /// default when the preset doesn't have it, clamped to its range.
    /// Sounding notes carry on, as with a factory preset.
    pub fn apply(&self, node: &mut dyn Node) -> Result<(), Error> {
        let info = node.info();
        if self.engine != info.id {
            return Err(Error::WrongEngine {
                file: self.engine.clone(),
                node: info.id.to_string(),
            });
        }
        for p in node.params() {
            if p.readonly {
                continue;
            }
            let v = self
                .values
                .iter()
                .rev()
                .find(|(id, _)| *id == p.id)
                .map_or(p.default, |&(_, v)| v);
            node.set_param(p.id, p.clamp(v));
        }
        Ok(())
    }

    /// The value of setting `id`, if the preset has it.
    pub fn value(&self, id: u16) -> Option<f32> {
        self.values
            .iter()
            .rev()
            .find(|(i, _)| *i == id)
            .map(|&(_, v)| v)
    }

    /// The preset as text, with `params` the engine's parameters.
    pub fn to_text(&self, params: &[ParamInfo]) -> String {
        let keys = Keys::new(params);
        let mut s = String::new();
        s.push_str(&format!("{FORMAT} = {VERSION}\n"));
        s.push_str(&format!("engine = {}\n", one_line(&self.engine)));
        s.push_str(&format!("name = {}\n", clean_name(&self.name)));
        let tags = clean_tags(self.tags.iter().map(String::as_str));
        if !tags.is_empty() {
            s.push_str(&format!("tags = {}\n", tags.join(", ")));
        }
        let mut group = None;
        for (i, p) in params.iter().enumerate() {
            if p.readonly {
                continue;
            }
            let Some(v) = self.value(p.id) else {
                continue;
            };
            if group != Some(p.group) {
                group = Some(p.group);
                s.push('\n');
                if !p.group.is_empty() {
                    s.push_str(&format!("# {}\n", one_line(p.group)));
                }
            }
            s.push_str(&format!("{} = {}\n", keys.key(i), value_text(p, v)));
        }
        if !self.extra.is_empty() {
            s.push('\n');
        }
        for (k, v) in &self.extra {
            s.push_str(&format!("{} = {}\n", one_line(k), one_line(v)));
        }
        s
    }

    /// Reads a preset for the node described by `info` and `params`.
    pub fn parse(text: &str, info: &NodeInfo, params: &[ParamInfo]) -> Result<Preset, Error> {
        if text.len() > MAX_TEXT {
            return Err(Error::TooBig);
        }
        let mut lines = text.lines().filter_map(split_line);
        match lines.next() {
            Some((FORMAT, v)) => match v.parse::<u32>() {
                Ok(v) if v > VERSION => return Err(Error::Newer(v)),
                Ok(_) => {}
                Err(_) => return Err(Error::NotAPreset),
            },
            _ => return Err(Error::NotAPreset),
        }
        let keys = Keys::new(params);
        let mut engine = None;
        let mut preset = Preset {
            engine: String::new(),
            name: String::new(),
            tags: Vec::new(),
            values: Vec::new(),
            extra: Vec::new(),
        };
        for (key, value) in lines {
            match key {
                "engine" => engine = Some(value.to_string()),
                "name" => preset.name = clean_name(value),
                "tags" => preset.tags = clean_tags(value.split(',')),
                _ => match keys.find(key) {
                    Some(i) if !params[i].readonly => {
                        let p = &params[i];
                        if let Some(v) = read_value(p, value) {
                            preset.values.retain(|(id, _)| *id != p.id);
                            preset.values.push((p.id, v));
                        }
                    }
                    Some(_) => {}
                    None => preset.extra.push((key.to_string(), value.to_string())),
                },
            }
        }
        let engine = engine.ok_or(Error::NoEngine)?;
        if engine != info.id {
            return Err(Error::WrongEngine {
                file: engine,
                node: info.id.to_string(),
            });
        }
        preset.engine = engine;
        // In the node's order, as `capture` makes them.
        preset
            .values
            .sort_by_key(|(id, _)| params.iter().position(|p| p.id == *id));
        if preset.name.is_empty() {
            preset.name = "Untitled".to_string();
        }
        Ok(preset)
    }
}

/// The engine a preset text is for, without reading the rest (to pick the
/// node before [`Preset::parse`]). `None` if it isn't a preset.
pub fn engine_of(text: &str) -> Option<&str> {
    let mut lines = text.lines().filter_map(split_line);
    lines.next().filter(|(k, _)| *k == FORMAT)?;
    lines.find(|(k, _)| *k == "engine").map(|(_, v)| v)
}

/// The text line's key and value, trimmed; `None` for a blank line, a
/// comment or a line without `=`.
fn split_line(line: &str) -> Option<(&str, &str)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let (k, v) = line.split_once('=')?;
    Some((k.trim(), v.trim()))
}

/// Each parameter's key in a node: its group and name, or its id where
/// that would be shared or empty.
struct Keys {
    keys: Vec<String>,
}

impl Keys {
    fn new(params: &[ParamInfo]) -> Keys {
        let mut keys: Vec<String> = params.iter().map(named_key).collect();
        let shared: Vec<bool> = keys
            .iter()
            .map(|k| k.is_empty() || keys.iter().filter(|o| *o == k).count() > 1)
            .collect();
        for (i, k) in keys.iter_mut().enumerate() {
            if shared[i] {
                *k = format!("id.{}", params[i].id);
            }
        }
        Keys { keys }
    }

    fn key(&self, i: usize) -> &str {
        &self.keys[i]
    }

    /// The parameter a key names (ignoring case).
    fn find(&self, key: &str) -> Option<usize> {
        let key = key.to_ascii_lowercase();
        self.keys.iter().position(|k| *k == key)
    }
}

/// `group.name` in lower case with `-` between words (`filter-env.attack`);
/// `param.name` for a parameter with no group; empty if the name has no
/// letters or digits.
fn named_key(p: &ParamInfo) -> String {
    let name = slug(p.name);
    if name.is_empty() {
        return String::new();
    }
    let group = slug(p.group);
    if group.is_empty() {
        format!("param.{name}")
    } else {
        format!("{group}.{name}")
    }
}

fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

/// A value as text: a choice by name, a switch as on or off, a number as
/// the shortest text that reads back to it.
fn value_text(p: &ParamInfo, v: f32) -> String {
    match p.curve {
        Curve::Choice(names) => {
            let i = v.round();
            match names.get(i as usize) {
                Some(n) if i >= 0.0 && i == v && usable_choice(names, n) => n.to_string(),
                _ => format!("{v}"),
            }
        }
        Curve::Toggle if v == 0.0 => "off".to_string(),
        Curve::Toggle if v == 1.0 => "on".to_string(),
        _ => format!("{v}"),
    }
}

/// A choice name that reads back to its own index: one line, and no
/// other choice with the same name.
fn usable_choice(names: &[&str], n: &str) -> bool {
    let t = one_line(n);
    !t.is_empty() && t == *n && names.iter().filter(|m| m.eq_ignore_ascii_case(n)).count() == 1
}

/// A value from text, or `None` if it isn't one (the setting then takes
/// its default).
fn read_value(p: &ParamInfo, text: &str) -> Option<f32> {
    match p.curve {
        Curve::Choice(names) => {
            if let Some(i) = names
                .iter()
                .position(|n| usable_choice(names, n) && n.eq_ignore_ascii_case(text))
            {
                return Some(i as f32);
            }
        }
        Curve::Toggle => match text.to_ascii_lowercase().as_str() {
            "on" | "true" | "yes" => return Some(1.0),
            "off" | "false" | "no" => return Some(0.0),
            _ => {}
        },
        _ => {}
    }
    text.parse::<f32>().ok().filter(|v| v.is_finite())
}

/// Text on one line: control characters become spaces, trimmed.
fn one_line(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .trim()
        .to_string()
}

fn clean_name(s: &str) -> String {
    one_line(s)
        .chars()
        .take(MAX_NAME)
        .collect::<String>()
        .trim_end()
        .to_string()
}

/// Tags trimmed, at most [`MAX_TAG`] characters, without commas, each
/// once (ignoring case), at most [`MAX_TAGS`].
fn clean_tags<'a>(tags: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in tags {
        let t: String = one_line(&t.replace(',', " "))
            .chars()
            .take(MAX_TAG)
            .collect::<String>()
            .trim_end()
            .to_string();
        if t.is_empty() || out.iter().any(|o| o.eq_ignore_ascii_case(&t)) {
            continue;
        }
        if out.len() == MAX_TAGS {
            break;
        }
        out.push(t);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_node::{Kind, Unit};

    static INFO: NodeInfo = NodeInfo {
        id: "test.plugin",
        name: "Plugin",
        kind: Kind::Instrument,
    };

    const fn knob(id: u16, name: &'static str, group: &'static str) -> ParamInfo {
        ParamInfo {
            id,
            name,
            group,
            min: 0.0,
            max: 10.0,
            default: 1.0,
            unit: Unit::None,
            curve: Curve::Linear,
            readonly: false,
        }
    }

    static WAVES: &[&str] = &["Saw", "Square", "8"];

    static PARAMS: &[ParamInfo] = &[
        knob(3, "Cutoff", "Filter"),
        knob(4, "Attack", "Filter env"),
        knob(5, "Attack", "Amp env"),
        // Two "Mix" knobs in one group: both fall back to ids.
        knob(6, "Mix", "FX"),
        knob(7, "Mix", "FX"),
        knob(8, "Gain", ""),
        knob(9, "→", "Odd"),
        ParamInfo {
            curve: Curve::Choice(WAVES),
            max: 2.0,
            default: 0.0,
            ..knob(10, "Wave", "Osc 1")
        },
        ParamInfo {
            curve: Curve::Toggle,
            max: 1.0,
            default: 0.0,
            ..knob(11, "Sync", "Osc 1")
        },
        ParamInfo {
            readonly: true,
            ..knob(12, "Voices", "Meters")
        },
    ];

    fn preset(values: &[(u16, f32)]) -> Preset {
        Preset {
            engine: INFO.id.to_string(),
            name: "Test".to_string(),
            tags: vec!["pad".to_string()],
            values: values.to_vec(),
            extra: Vec::new(),
        }
    }

    #[test]
    fn keys_read_like_the_synth_tab_and_fall_back_to_ids() {
        let k = Keys::new(PARAMS);
        let keys: Vec<&str> = (0..PARAMS.len()).map(|i| k.key(i)).collect();
        assert_eq!(
            keys,
            [
                "filter.cutoff",
                "filter-env.attack",
                "amp-env.attack",
                "id.6",
                "id.7",
                "param.gain",
                "id.9",
                "osc-1.wave",
                "osc-1.sync",
                "meters.voices",
            ]
        );
        assert_eq!(k.find("Filter.Cutoff"), Some(0));
        assert_eq!(k.find("id.7"), Some(4));
        assert_eq!(k.find("id.3"), None, "an id key only for shared names");
    }

    #[test]
    fn text_round_trips_every_value_exactly() {
        let p = preset(&[
            (3, 0.1),
            (4, 1.0 / 3.0),
            (5, 7.25),
            (6, 1e-7),
            (7, 9.999_999),
            (8, 0.0),
            (9, 2.5),
            (10, 2.0),
            (11, 1.0),
        ]);
        let text = p.to_text(PARAMS);
        assert!(
            text.starts_with("mpfree-preset = 1\nengine = test.plugin\nname = Test\ntags = pad\n")
        );
        assert!(
            text.contains("\n# Osc 1\nosc-1.wave = 8\nosc-1.sync = on\n"),
            "{text}"
        );
        assert!(!text.contains("voices"), "meters aren't saved");
        let back = Preset::parse(&text, &INFO, PARAMS).unwrap();
        assert_eq!(back, p);
        for (a, b) in back.values.iter().zip(&p.values) {
            assert_eq!(a.1.to_bits(), b.1.to_bits());
        }
    }

    #[test]
    fn choices_and_switches_read_by_name_or_number() {
        let text =
            "mpfree-preset = 1\nengine = test.plugin\nosc-1.wave = square\nosc-1.sync = TRUE\n";
        let p = Preset::parse(text, &INFO, PARAMS).unwrap();
        assert_eq!(p.value(10), Some(1.0));
        assert_eq!(p.value(11), Some(1.0));
        let text = "mpfree-preset = 1\nengine = test.plugin\nosc-1.wave = 0\nosc-1.sync = 0\n";
        let p = Preset::parse(text, &INFO, PARAMS).unwrap();
        assert_eq!((p.value(10), p.value(11)), (Some(0.0), Some(0.0)));
        // "8" is a choice's name, so it means that choice, not index 8.
        let text = "mpfree-preset = 1\nengine = test.plugin\nosc-1.wave = 8\n";
        assert_eq!(
            Preset::parse(text, &INFO, PARAMS).unwrap().value(10),
            Some(2.0)
        );
    }

    #[test]
    fn unknown_lines_are_kept_and_written_back() {
        let text = "# saved by hand\n\
                    mpfree-preset = 1\n\
                    engine = test.plugin\n\
                    name =   Wide  pad  \n\
                    tags = Pad, warm, pad, , a tag far longer than twenty\n\
                    filter.cutoff = 4\n\
                    filter.resonance = 0.7\n\
                    qset = 3 4 - - Tone\n\
                    this line has no equals\n\
                    filter.cutoff = 5\n\
                    meters.voices = 3\n\
                    amp-env.attack = loud\n";
        let p = Preset::parse(text, &INFO, PARAMS).unwrap();
        assert_eq!(p.name, "Wide  pad");
        assert_eq!(p.tags, ["Pad", "warm", "a tag far longer tha"]);
        assert_eq!(p.value(3), Some(5.0), "the last line wins");
        assert_eq!(p.value(12), None, "a meter is never read");
        assert_eq!(p.value(5), None, "not a number: the default");
        assert_eq!(
            p.extra,
            [
                ("filter.resonance".to_string(), "0.7".to_string()),
                ("qset".to_string(), "3 4 - - Tone".to_string()),
            ]
        );
        let again = Preset::parse(&p.to_text(PARAMS), &INFO, PARAMS).unwrap();
        assert_eq!(again, p);
    }

    #[test]
    fn what_is_not_a_preset_says_why() {
        let parse = |t: &str| Preset::parse(t, &INFO, PARAMS);
        assert_eq!(parse(""), Err(Error::NotAPreset));
        assert_eq!(parse("engine = test.plugin\n"), Err(Error::NotAPreset));
        assert_eq!(parse("mpfree-preset = one\n"), Err(Error::NotAPreset));
        assert_eq!(
            parse("mpfree-preset = 2\nengine = test.plugin\n"),
            Err(Error::Newer(2))
        );
        assert_eq!(parse("mpfree-preset = 1\nname = x\n"), Err(Error::NoEngine));
        assert_eq!(
            parse("mpfree-preset = 1\nengine = mpfree.poly\n"),
            Err(Error::WrongEngine {
                file: "mpfree.poly".to_string(),
                node: "test.plugin".to_string()
            })
        );
        let big = format!("mpfree-preset = 1\n{}", "# x\n".repeat(MAX_TEXT / 4));
        assert_eq!(parse(&big), Err(Error::TooBig));
        let ok = parse("mpfree-preset = 1\nengine = test.plugin\n").unwrap();
        assert_eq!(ok.name, "Untitled");
        assert!(ok.values.is_empty());
        assert_eq!(
            engine_of("mpfree-preset = 1\n# c\nengine = mpfree.fm\n"),
            Some("mpfree.fm")
        );
        assert_eq!(engine_of("engine = mpfree.fm\n"), None);
        assert!(Error::Newer(2).to_string().contains("newer MPFree"));
    }

    #[test]
    fn names_and_tags_stay_on_one_line() {
        let mut p = preset(&[]);
        p.name = "Bad\nname = evil\u{7}".to_string();
        p.tags = vec!["a\nb".to_string(), "c,d".to_string()];
        p.extra = vec![("qset".to_string(), "1 2\n3 4".to_string())];
        let text = p.to_text(PARAMS);
        let back = Preset::parse(&text, &INFO, PARAMS).unwrap();
        assert_eq!(back.name, "Bad name = evil");
        assert_eq!(back.tags, ["a b", "c d"]);
        assert_eq!(back.extra, [("qset".to_string(), "1 2 3 4".to_string())]);
        assert_eq!(text.lines().count(), 6, "{text}");
    }
}
