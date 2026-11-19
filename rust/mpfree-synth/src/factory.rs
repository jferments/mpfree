//! The factory presets, as preset files built into the program: one file
//! an engine in `rust/mpfree-synth/presets/` (`keys.presets` and so on),
//! each preset a block in the format users' presets have
//! ([`mpfree_node::preset`]), one after the other:
//!
//! ```text
//! mpfree-preset = 2
//! engine = mpfree.keys
//! name = Soft Tines
//!
//! # Tone
//! tone.hardness = 30
//! ```
//!
//! A block lists what differs from the engine's defaults (what the
//! engine's `load_preset` starts from), so a file reads as the sound's
//! recipe and a review sees each change. Lines the engine doesn't read (a
//! sound's effects and macros, `mpfree-instruments`) ride along in
//! [`Preset::text`] for whatever hosts it. The files are written,
//! reviewed and checked like any other file; the engines' tests read every
//! preset ([`read`] reports a block that doesn't read, and the tests fail
//! on it), and the lab measures them. Every block is in the preset format
//! this build writes, so a factory sound never needs converting: a block
//! in an older one is reported too.

use mpfree_node::preset::Preset as Text;
use mpfree_node::{NodeInfo, ParamInfo};

use crate::params::Preset;

/// The first line of every block.
const START: &str = mpfree_node::preset::FORMAT;

/// Every preset in `text` (an engine's file) for the engine `info` with
/// `params`, in the file's order, and a line for each block that doesn't
/// read (left out). Allocates once per engine and keeps the presets for
/// the program's life, as the engines hand out `&'static` presets.
pub fn read(
    text: &'static str,
    info: &NodeInfo,
    params: &[ParamInfo],
) -> (Vec<Preset>, Vec<String>) {
    let mut presets = Vec::new();
    let mut errors = Vec::new();
    for (n, block) in blocks(text).into_iter().enumerate() {
        match Text::parse(block, info, params) {
            Ok(t) if t.format < mpfree_node::preset::VERSION => errors.push(format!(
                "{} preset {}: written in preset format {}, not {}",
                info.name,
                n + 1,
                t.format,
                mpfree_node::preset::VERSION
            )),
            Ok(t) => presets.push(Preset {
                name: Box::leak(t.name.into_boxed_str()),
                values: Box::leak(t.values.into_boxed_slice()),
                text: block,
            }),
            Err(e) => errors.push(format!("{} preset {}: {e}", info.name, n + 1)),
        }
    }
    (presets, errors)
}

/// The blocks of a file: each from a format line to the next.
fn blocks(text: &str) -> Vec<&str> {
    let mut starts: Vec<usize> = Vec::new();
    let mut at = 0;
    for line in text.split_inclusive('\n') {
        let key = line.split('=').next().unwrap_or("").trim();
        if key == START {
            starts.push(at);
        }
        at += line.len();
    }
    starts
        .iter()
        .enumerate()
        .map(|(i, &s)| &text[s..starts.get(i + 1).copied().unwrap_or(text.len())])
        .collect()
}

/// The presets of an engine's file, read once: `presets!(KEYS, "keys")`
/// in an engine's `presets()`.
macro_rules! presets {
    ($file:literal, $info:expr, $params:expr) => {{
        static READ: std::sync::OnceLock<Vec<$crate::params::Preset>> = std::sync::OnceLock::new();
        READ.get_or_init(|| {
            $crate::factory::read(
                include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/presets/", $file)),
                $info,
                $params,
            )
            .0
        })
        .as_slice()
    }};
}
pub(crate) use presets;

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_node::Node;

    /// Every engine's file reads whole: every block a preset, Init first,
    /// no two presets with one name, every value one the engine has.
    #[test]
    fn every_factory_file_reads_whole() {
        let r = 48_000;
        let engines: Vec<(Box<dyn Node>, &'static str)> = vec![
            (
                Box::new(crate::Poly::new(r)),
                include_str!("../presets/poly.presets"),
            ),
            (
                Box::new(crate::Drums::new(r)),
                include_str!("../presets/drums.presets"),
            ),
            (
                Box::new(crate::Pluck::new(r)),
                include_str!("../presets/pluck.presets"),
            ),
            (
                Box::new(crate::Wavetable::new(r)),
                include_str!("../presets/wavetable.presets"),
            ),
            (
                Box::new(crate::Keys::new(r)),
                include_str!("../presets/keys.presets"),
            ),
            (
                Box::new(crate::Drawbar::new(r)),
                include_str!("../presets/drawbar.presets"),
            ),
            (
                Box::new(crate::Squelch::new(r)),
                include_str!("../presets/squelch.presets"),
            ),
            (
                Box::new(crate::Analog::new(r)),
                include_str!("../presets/analog.presets"),
            ),
            (
                Box::new(crate::Strings::new(r)),
                include_str!("../presets/strings.presets"),
            ),
            (
                Box::new(crate::Bass::new(r)),
                include_str!("../presets/bass.presets"),
            ),
            (
                Box::new(crate::Fm::new(r)),
                include_str!("../presets/fm.presets"),
            ),
        ];
        for (node, text) in engines {
            let name = node.info().name;
            let (presets, errors) = read(text, node.info(), node.params());
            assert!(errors.is_empty(), "{errors:?}");
            assert_eq!(presets.len(), blocks(text).len(), "{name}");
            assert!(presets[0].name.starts_with("Init"), "{name}: Init first");
            for (i, p) in presets.iter().enumerate() {
                assert!(
                    !presets[..i].iter().any(|q| q.name == p.name),
                    "{name}: two presets called {}",
                    p.name
                );
                for &(id, _) in p.values {
                    assert!(
                        node.params().iter().any(|q| q.id == id && !q.readonly),
                        "{name} {}: id {id}",
                        p.name
                    );
                }
            }
        }
    }

    #[test]
    fn a_block_that_does_not_read_is_left_out_and_said() {
        let r = 48_000;
        let k = crate::Keys::new(r);
        let text: &'static str = "mpfree-preset = 2\nengine = mpfree.keys\nname = One\n\n\
             mpfree-preset = 2\nengine = mpfree.poly\nname = Two\n\n\
             mpfree-preset = 2\nengine = mpfree.keys\nname = Three\ntone.bark = 10\n\n\
             mpfree-preset = 1\nengine = mpfree.keys\nname = Four\n";
        let (presets, errors) = read(text, k.info(), k.params());
        let names: Vec<&str> = presets.iter().map(|p| p.name).collect();
        assert_eq!(names, ["One", "Three"]);
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert!(errors[0].contains("preset 2"), "{errors:?}");
        assert!(
            errors[1].contains("preset 4") && errors[1].contains("format 1"),
            "a factory sound is never converted: {errors:?}"
        );
        assert!(presets[1]
            .text
            .starts_with("mpfree-preset = 2\nengine = mpfree.keys\nname = Three"));
        assert_eq!(presets[1].values, &[(crate::keys_params::BARK, 10.0)]);
    }
}
