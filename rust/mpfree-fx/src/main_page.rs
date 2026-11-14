//! Main pages: for an effect with more than eight controls, the eight a
//! player reaches for, on one page in front of its groups (Jay, 14 Nov
//! 2026: layout B). Its first four are "Main's four": the Q-Links turn
//! them on every page of the effect, so the hands keep the same knobs
//! while the eyes look through the groups. An effect with stages it
//! switches (the Channel strip's gate, compressor and EQ) shows them on
//! its Main page too, each with its switch and the way to its page.
//!
//! An effect with no Main page (eight controls or fewer, or a hosted
//! plugin) shows its groups only; the chain editor splits a group over
//! eight into pages of eight.
//!
//! Controls are named as their effect names them, or "Group/Name" where a
//! name repeats (the EQ's bands); [`find`] reads both.

use mpfree_node::ParamInfo;

/// An effect's Main page.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MainPage {
    /// The engine's node id.
    pub id: &'static str,
    /// Up to eight controls, in the order they are laid out (four a row).
    pub knobs: &'static [&'static str],
    /// Main's four, the Q-Links on every page: four of `knobs`.
    pub qlinks: [&'static str; 4],
    /// The stages it switches, shown on the Main page.
    pub stages: &'static [Stage],
}

/// A stage of an effect: a switch and the group that holds its controls.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stage {
    /// "Gate".
    pub label: &'static str,
    /// The control that switches it on and off (a toggle).
    pub switch: &'static str,
    /// Its group of controls.
    pub group: &'static str,
}

const fn main(
    id: &'static str,
    knobs: &'static [&'static str],
    qlinks: [&'static str; 4],
) -> MainPage {
    MainPage {
        id,
        knobs,
        qlinks,
        stages: &[],
    }
}

/// Every Main page: every built-in effect with more than eight controls.
pub static MAIN_PAGES: &[MainPage] = &[
    main(
        "mpfree.fx.compressor",
        &[
            "Threshold",
            "Ratio",
            "Attack",
            "Release",
            "Knee",
            "Makeup",
            "Mix",
            "SC filter",
        ],
        ["Threshold", "Ratio", "Makeup", "Mix"],
    ),
    main(
        "mpfree.fx.ducker",
        &[
            "Source",
            "Rate",
            "Threshold",
            "Depth",
            "Attack",
            "Release",
            "Shape",
            "Mix",
        ],
        ["Depth", "Release", "Shape", "Mix"],
    ),
    main(
        "mpfree.fx.multiband",
        &[
            "Low thresh",
            "Mid thresh",
            "High thresh",
            "Mix",
            "Low gain",
            "Mid gain",
            "High gain",
            "Output",
        ],
        ["Low thresh", "Mid thresh", "High thresh", "Mix"],
    ),
    MainPage {
        id: "mpfree.fx.channel_strip",
        knobs: &[
            "Gate thresh",
            "Comp thresh",
            "Ratio",
            "Makeup",
            "Low cut",
            "Mid gain",
            "High gain",
            "Output",
        ],
        qlinks: ["Comp thresh", "Makeup", "Low cut", "High gain"],
        stages: &[
            Stage {
                label: "Gate",
                switch: "Gate",
                group: "Gate",
            },
            Stage {
                label: "Comp",
                switch: "Comp",
                group: "Compressor",
            },
            Stage {
                label: "EQ",
                switch: "EQ",
                group: "EQ",
            },
        ],
    },
    main(
        "mpfree.fx.eq",
        &[
            "Band 1/Gain",
            "Band 2/Gain",
            "Band 3/Gain",
            "Band 4/Gain",
            "Band 1/Freq",
            "Band 2/Freq",
            "Band 3/Freq",
            "Band 4/Freq",
        ],
        ["Band 1/Gain", "Band 2/Gain", "Band 3/Gain", "Band 4/Gain"],
    ),
    main(
        "mpfree.fx.autowah",
        &[
            "Center",
            "Depth",
            "Resonance",
            "Mix",
            "Source",
            "Sensitivity",
            "Rate",
            "Filter",
        ],
        ["Center", "Depth", "Resonance", "Mix"],
    ),
    main(
        "mpfree.fx.lofi",
        &[
            "Amount",
            "Saturation",
            "High cut",
            "Mix",
            "Wow",
            "Flutter",
            "Crackle",
            "Noise",
        ],
        ["Amount", "Saturation", "High cut", "Mix"],
    ),
    main(
        "mpfree.fx.tape_delay",
        &[
            "Time",
            "Feedback",
            "Tone",
            "Mix",
            "Sync",
            "Heads",
            "Wow",
            "Saturation",
        ],
        ["Time", "Feedback", "Tone", "Mix"],
    ),
    main(
        "mpfree.fx.multitap",
        &[
            "Feedback",
            "Tone",
            "Low cut",
            "Mix",
            "Tap 1 level",
            "Tap 2 level",
            "Tap 3 level",
            "Tap 4 level",
        ],
        ["Feedback", "Tone", "Low cut", "Mix"],
    ),
    main(
        "mpfree.fx.cloud",
        &[
            "Decay",
            "Size",
            "Pre-delay",
            "Mix",
            "Diffusion",
            "Damping",
            "Low cut",
            "High cut",
        ],
        ["Decay", "Size", "Pre-delay", "Mix"],
    ),
    main(
        "mpfree.fx.room",
        &[
            "Decay",
            "Size",
            "Pre-delay",
            "Mix",
            "Damping",
            "Early",
            "Gate",
            "Low cut",
        ],
        ["Decay", "Size", "Pre-delay", "Mix"],
    ),
    main(
        "mpfree.fx.plate",
        &[
            "Decay",
            "Size",
            "Pre-delay",
            "Mix",
            "Damping",
            "Tone",
            "Mod",
            "Low cut",
        ],
        ["Decay", "Size", "Pre-delay", "Mix"],
    ),
    main(
        "mpfree.fx.granulator",
        &[
            "Size", "Density", "Pitch", "Mix", "Detune", "Jitter", "Feedback", "Freeze",
        ],
        ["Size", "Density", "Pitch", "Mix"],
    ),
    main(
        "mpfree.fx.harmonizer",
        &[
            "Level 1", "Level 2", "Glide", "Dry", "Key", "Scale", "Voice 1", "Voice 2",
        ],
        ["Level 1", "Level 2", "Glide", "Dry"],
    ),
    main(
        "mpfree.fx.talkbox",
        &[
            "Vowel",
            "Formant",
            "Env depth",
            "Mix",
            "Resonance",
            "Sensitivity",
            "LFO rate",
            "LFO depth",
        ],
        ["Vowel", "Formant", "Env depth", "Mix"],
    ),
    main(
        "mpfree.fx.perform",
        &[
            "DJ filter",
            "Echo",
            "Echo time",
            "Mix",
            "Punch in on",
            "Fade",
        ],
        ["DJ filter", "Echo", "Echo time", "Mix"],
    ),
    main(
        "mpfree.fx.width",
        &[
            "Width",
            "Low width",
            "High width",
            "Delay",
            "Mid width",
            "Low split",
            "High split",
            "Output",
        ],
        ["Width", "Low width", "High width", "Delay"],
    ),
];

/// The Main page of engine `id`, if it has one.
pub fn main_page(id: &str) -> Option<&'static MainPage> {
    MAIN_PAGES.iter().find(|m| m.id == id)
}

/// The control `name` names among `params` ("Decay", or "Band 2/Gain"
/// where a name repeats; a name with a slash of its own, the Multiband's
/// "Low/mid", is found as it is).
pub fn find<'p>(params: &'p [ParamInfo], name: &str) -> Option<&'p ParamInfo> {
    params.iter().find(|p| p.name == name).or_else(|| {
        let (group, n) = name.split_once('/')?;
        params.iter().find(|p| p.group == group && p.name == n)
    })
}

/// The ids of a Main page's knobs among `params`, in its order.
pub fn knob_ids(m: &MainPage, params: &[ParamInfo]) -> Vec<u16> {
    m.knobs
        .iter()
        .filter_map(|k| find(params, k).map(|p| p.id))
        .collect()
}

/// The ids of Main's four among `params`.
pub fn qlink_ids(m: &MainPage, params: &[ParamInfo]) -> [Option<u16>; 4] {
    m.qlinks.map(|k| find(params, k).map(|p| p.id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{self, EFFECTS};
    use mpfree_node::Curve;

    fn settable(id: &str) -> usize {
        let n = registry::make(id).expect(id);
        n.params().iter().filter(|p| !p.readonly).count()
    }

    /// Every built-in effect with more than eight controls has a Main
    /// page, and only those.
    #[test]
    fn every_long_effect_has_a_main_page() {
        for e in EFFECTS {
            let long = settable(e.id) > 8;
            assert_eq!(main_page(e.id).is_some(), long, "{}", e.name);
        }
        for m in MAIN_PAGES {
            assert!(registry::entry(m.id).is_some(), "{}", m.id);
        }
    }

    /// A Main page names up to eight settable controls of its effect, each
    /// once and none of a family mode's fixed ones; Main's four are among
    /// them; a stage's switch is a toggle and its group is the effect's.
    #[test]
    fn main_pages_name_real_controls() {
        for m in MAIN_PAGES {
            let n = registry::make(m.id).unwrap();
            let params = n.params();
            assert!(!m.knobs.is_empty() && m.knobs.len() <= 8, "{}", m.id);
            let ids = knob_ids(m, params);
            assert_eq!(ids.len(), m.knobs.len(), "{}: {:?}", m.id, m.knobs);
            for (i, id) in ids.iter().enumerate() {
                assert!(!ids[..i].contains(id), "{} twice in {}", m.knobs[i], m.id);
                let p = params.iter().find(|p| p.id == *id).unwrap();
                assert!(!p.readonly, "{} {}", m.id, p.name);
            }
            let fixed: Vec<u16> = crate::families::family(m.id)
                .into_iter()
                .flat_map(|f| f.modes.iter())
                .filter(|md| md.id == m.id)
                .flat_map(|md| md.fixed.iter().map(|&(p, _)| p))
                .collect();
            assert!(!ids.iter().any(|id| fixed.contains(id)), "{}", m.id);
            for q in qlink_ids(m, params) {
                assert!(
                    q.is_some_and(|q| ids.contains(&q)),
                    "{} {:?}",
                    m.id,
                    m.qlinks
                );
            }
            for s in m.stages {
                let sw = find(params, s.switch).expect(s.switch);
                assert_eq!(sw.curve, Curve::Toggle, "{} {}", m.id, s.label);
                assert!(params.iter().any(|p| p.group == s.group), "{}", s.group);
            }
        }
    }

    #[test]
    fn a_repeated_name_is_found_by_its_group() {
        let n = registry::make("mpfree.fx.eq").unwrap();
        let a = find(n.params(), "Band 1/Gain").unwrap();
        let b = find(n.params(), "Band 2/Gain").unwrap();
        assert_ne!(a.id, b.id);
        assert_eq!((a.group, a.name), ("Band 1", "Gain"));
        assert!(find(n.params(), "Band 9/Gain").is_none());
    }
}
